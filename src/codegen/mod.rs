//! LLVM code generation for Coral programs.
//!
//! This module transforms semantic models into LLVM IR using inkwell bindings.

mod runtime;
mod builtins;
mod match_adt;
mod store_actor;
mod closures;

use runtime::RuntimeBindings;

use crate::ast::{
    BinaryOp,
    Binding,
    Block,
    Expression,
    Function,
    FunctionKind,
    MatchExpression,
    MatchPattern,
    Parameter,
    Statement,
    TypeAnnotation,
    UnaryOp,
};
use crate::diagnostics::Diagnostic;
use crate::semantic::SemanticModel;
use crate::span::{LineIndex, Span};
use crate::types::{AllocationStrategy, TypeId, Primitive};
use inkwell::InlineAsmDialect;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DWARFEmissionKind, DWARFSourceLanguage, DIFlags, DIFlagsConstants, DIScope,
    DISubroutineType, DebugInfoBuilder, DICompileUnit, DIFile,
};
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FloatType, FunctionType, IntType, StructType};
use inkwell::values::{
    BasicMetadataValueEnum,
    BasicValue,
    BasicValueEnum,
    FloatValue,
    FunctionValue,
    GlobalValue,
    IntValue,
    PointerValue,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};
use std::collections::{HashMap, HashSet};

pub struct CodeGenerator<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    f64_type: FloatType<'ctx>,
    i8_type: IntType<'ctx>,
    bool_type: IntType<'ctx>,
    usize_type: IntType<'ctx>,
    runtime: RuntimeBindings<'ctx>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    string_pool: HashMap<String, GlobalValue<'ctx>>,
    bytes_pool: HashMap<Vec<u8>, GlobalValue<'ctx>>,
    global_variables: HashMap<String, GlobalValue<'ctx>>,
    globals_initialized_flag: Option<GlobalValue<'ctx>>,
    global_init_fn: Option<FunctionValue<'ctx>>,
    lambda_counter: usize,
    allocation_hints: HashMap<String, AllocationStrategy>,
    extern_sigs: HashMap<String, ExternSignature<'ctx>>,
    inline_asm_mode: InlineAsmMode,
    /// Maps store method name to (store_name, param_count) for dynamic dispatch
    store_methods: HashMap<String, (String, usize)>,
    /// Maps (store_name, field_name) to is_reference for reference field tracking
    reference_fields: HashSet<(String, String)>,
    /// Maps enum constructor name to (enum_name, field_count) for ADT construction
    enum_constructors: HashMap<String, (String, usize)>,
    /// Set of store constructor function names (e.g., "make_Counter")
    store_constructors: HashSet<String>,
    /// Set of all known store field names (for disambiguation in member access)
    store_field_names: HashSet<String>,
    /// Set of persistent store type names (for persistence hooks)
    persistent_stores: HashSet<String>,
    /// Tracks whether any persistent store was opened (to emit save_all at exit)
    has_persistent_stores: bool,
    /// CC2.3: Optional DWARF debug info context.
    debug_ctx: Option<DebugContext<'ctx>>,
    /// C2.1/C2.2: Resolved variable types from semantic analysis for type specialization.
    resolved_types: HashMap<String, TypeId>,
}

/// CC2.3: Holds state for DWARF debug-info emission.
struct DebugContext<'ctx> {
    builder: DebugInfoBuilder<'ctx>,
    compile_unit: DICompileUnit<'ctx>,
    file: DIFile<'ctx>,
    line_index: LineIndex,
    /// A generic subroutine type used for all Coral functions (all i64 → i64).
    fn_di_type: DISubroutineType<'ctx>,
}

#[derive(Clone)]
struct ExternSignature<'ctx> {
    function: FunctionValue<'ctx>,
    param_types: Vec<BasicTypeEnum<'ctx>>,
    ret_type: Option<BasicTypeEnum<'ctx>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineAsmMode {
    Deny,
    Noop,
    Emit,
}

impl<'ctx> CodeGenerator<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        let f64_type = context.f64_type();
        let i8_type = context.i8_type();
        let bool_type = context.bool_type();
        let usize_type = context.i64_type();
        let runtime = RuntimeBindings::declare(context, &module);
        Self {
            context,
            module,
            builder,
            f64_type,
            i8_type,
            bool_type,
            usize_type,
            runtime,
            functions: HashMap::new(),
            string_pool: HashMap::new(),
            bytes_pool: HashMap::new(),
            global_variables: HashMap::new(),
            globals_initialized_flag: None,
            global_init_fn: None,
            lambda_counter: 0,
            allocation_hints: HashMap::new(),
            extern_sigs: HashMap::new(),
            inline_asm_mode: InlineAsmMode::Deny,
            store_methods: HashMap::new(),
            reference_fields: HashSet::new(),
            enum_constructors: HashMap::new(),
            store_constructors: HashSet::new(),
            store_field_names: HashSet::new(),
            persistent_stores: HashSet::new(),
            has_persistent_stores: false,
            debug_ctx: None,
            resolved_types: HashMap::new(),
        }
    }

    pub fn with_inline_asm_mode(mut self, mode: InlineAsmMode) -> Self {
        self.inline_asm_mode = mode;
        self
    }

    /// CC2.3: Enable DWARF debug info emission, supplying filename and source text.
    pub fn with_debug_info(mut self, filename: &str, source: &str) -> Self {
        let debug_metadata_version = self.context.i32_type().const_int(3, false);
        self.module.add_basic_value_flag(
            "Debug Info Version",
            inkwell::module::FlagBehavior::Warning,
            debug_metadata_version,
        );
        let (dibuilder, compile_unit) = self.module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::C, // closest available; Coral has no DWARF lang code
            filename,
            ".",
            "coralc",
            false,
            "",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "", // sysroot (llvm16)
            "", // sdk      (llvm16)
        );
        let file = compile_unit.get_file();
        let line_index = LineIndex::new(source);
        // Generic subroutine type for Coral functions (returns void-placeholder, no typed params)
        let fn_di_type = dibuilder.create_subroutine_type(
            file,
            None,
            &[],
            DIFlags::PUBLIC,
        );
        self.debug_ctx = Some(DebugContext {
            builder: dibuilder,
            compile_unit,
            file,
            line_index,
            fn_di_type,
        });
        self
    }

    /// CC2.3: Set the LLVM builder's current debug location from a Coral span.
    fn set_debug_location(&self, span: Span, scope: DIScope<'ctx>) {
        if let Some(dbg) = &self.debug_ctx {
            let (line, col) = dbg.line_index.line_col(span.start);
            let loc = dbg.builder.create_debug_location(
                self.context,
                line as u32,
                col as u32,
                scope,
                None,
            );
            self.builder.set_current_debug_location(loc);
        }
    }

    pub fn compile(mut self, model: &SemanticModel) -> Result<Module<'ctx>, Diagnostic> {
        self.allocation_hints = model.allocation.symbols.clone();
        // C2.1: Populate resolved types from semantic analysis for type specialization.
        for (name, ty) in model.types.iter_all() {
            self.resolved_types.insert(name, ty);
        }

        // C3.5: Compute reachable functions (dead function elimination).
        let reachable = Self::compute_reachable_functions(model);

        self.declare_global_bindings(&model.globals);
        self.extern_sigs.clear();

        // Declare extern functions
        for extern_fn in &model.extern_functions {
            let mut param_types = Vec::new();
            for param in &extern_fn.params {
                let ann = param
                    .type_annotation
                    .as_ref()
                    .ok_or_else(|| Diagnostic::new("extern parameters require a type", param.span))?;
                param_types.push(self.map_extern_type(ann)?);
            }
            let ret_type = if let Some(ret_ann) = &extern_fn.return_type {
                Some(self.map_extern_type(ret_ann)?)
            } else {
                None
            };
            let fn_type = self.extern_function_type(ret_type.as_ref(), &param_types)?;
            let llvm_fn = self.module.add_function(&extern_fn.name, fn_type, None);
            self.extern_sigs.insert(
                extern_fn.name.clone(),
                ExternSignature {
                    function: llvm_fn,
                    param_types,
                    ret_type,
                },
            );
            self.functions.insert(extern_fn.name.clone(), llvm_fn);
        }
        
        // Declare user functions
        // All Coral functions use Value* (pointer to tagged value) for params and returns.
        // This ensures non-numeric values (strings, lists, etc.) are passed correctly.
        for function in &model.functions {
            // C3.5: Skip unreachable functions
            if !reachable.contains(&function.name) {
                continue;
            }
            let llvm_name = if function.name == "main" {
                "__user_main"
            } else {
                &function.name
            };
            let fn_type = self.runtime.value_i64_type.fn_type(
                &vec![self.runtime.value_i64_type.into(); function.params.len()],
                false,
            );
            let llvm_fn = self.module.add_function(llvm_name, fn_type, None);
            self.functions.insert(function.name.clone(), llvm_fn);
        }
        // Handle stores and actors
        for store in &model.stores {
            // Track reference fields for this store
            for field in &store.fields {
                self.store_field_names.insert(field.name.clone());
                if field.is_reference {
                    self.reference_fields.insert((store.name.clone(), field.name.clone()));
                }
            }
            
            // All stores get a constructor that returns a Map with fields
            let constructor_name = format!("make_{}", store.name);
            let ctor_type = self.runtime.value_i64_type.fn_type(&[], false);
            let ctor_fn = self.module.add_function(&constructor_name, ctor_type, None);
            self.functions.insert(constructor_name.clone(), ctor_fn);
            self.store_constructors.insert(constructor_name);
            
            if store.is_persistent {
                self.persistent_stores.insert(store.name.clone());
                self.has_persistent_stores = true;
            }
            
            if store.is_actor {
                // Declare message handler functions for each @method
                // Actor methods take state as hidden first param (i64), plus user params (i64)
                for method in &store.methods {
                    if method.kind == FunctionKind::ActorMessage {
                        let mangled = format!("{}_{}", store.name, method.name);
                        let mut param_types: Vec<BasicMetadataTypeEnum> = 
                            vec![self.runtime.value_i64_type.into()];
                        for _ in 0..method.params.len() {
                            param_types.push(self.runtime.value_i64_type.into());
                        }
                        let fn_type = self.runtime.value_i64_type.fn_type(&param_types, false);
                        let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                        self.functions.insert(mangled, llvm_fn);
                    }
                }
            } else {
                // Non-actor store methods: take self (store Map) as first param
                for method in &store.methods {
                    if method.kind == FunctionKind::Method {
                        let mangled = format!("{}_{}", store.name, method.name);
                        let mut param_types: Vec<BasicMetadataTypeEnum> = 
                            vec![self.runtime.value_i64_type.into()];
                        for _ in 0..method.params.len() {
                            param_types.push(self.runtime.value_i64_type.into());
                        }
                        let fn_type = self.runtime.value_i64_type.fn_type(&param_types, false);
                        let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                        self.functions.insert(mangled.clone(), llvm_fn);
                        self.store_methods.insert(method.name.clone(), (store.name.clone(), method.params.len()));
                    }
                }
            }
        }
        
        // Register enum constructors from type definitions
        for type_def in &model.type_defs {
            for variant in &type_def.variants {
                // Track constructor: (enum_name, field_count)
                self.enum_constructors.insert(
                    variant.name.clone(), 
                    (type_def.name.clone(), variant.fields.len())
                );
            }
            // Declare type methods (mirrors store method pattern)
            for method in &type_def.methods {
                if method.kind == FunctionKind::Method {
                    let mangled = format!("{}_{}", type_def.name, method.name);
                    let mut param_types: Vec<BasicMetadataTypeEnum> = 
                        vec![self.runtime.value_i64_type.into()]; // self
                    for _ in 0..method.params.len() {
                        param_types.push(self.runtime.value_i64_type.into());
                    }
                    let fn_type = self.runtime.value_i64_type.fn_type(&param_types, false);
                    let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                    self.functions.insert(mangled.clone(), llvm_fn);
                    self.store_methods.insert(method.name.clone(), (type_def.name.clone(), method.params.len()));
                }
            }
        }
        
        self.build_global_initializer(&model.globals)?;
        
        for function in &model.functions {
            // C3.5: Skip unreachable functions
            if !reachable.contains(&function.name) {
                continue;
            }
            if let Some(llvm_fn) = self.functions.get(&function.name) {
                self.build_function_body(function, *llvm_fn)?;
            }
        }
        // Build store/actor method bodies
        for store in &model.stores {
            if store.is_actor {
                for method in &store.methods {
                    if method.kind == FunctionKind::ActorMessage {
                        let mangled = format!("{}_{}", store.name, method.name);
                        if let Some(llvm_fn) = self.functions.get(&mangled) {
                            self.build_actor_method_body(method, *llvm_fn)?;
                        }
                    }
                }
            } else {
                // Non-actor store methods
                for method in &store.methods {
                    if method.kind == FunctionKind::Method {
                        let mangled = format!("{}_{}", store.name, method.name);
                        if let Some(llvm_fn) = self.functions.get(&mangled) {
                            self.build_store_method_body(method, *llvm_fn)?;
                        }
                    }
                }
            }
        }
        // Build type method bodies (same mechanism as store methods)
        for type_def in &model.type_defs {
            for method in &type_def.methods {
                if method.kind == FunctionKind::Method {
                    let mangled = format!("{}_{}", type_def.name, method.name);
                    if let Some(llvm_fn) = self.functions.get(&mangled) {
                        self.build_store_method_body(method, *llvm_fn)?;
                    }
                }
            }
        }
        
        // Generate store/actor constructor bodies
        for store in &model.stores {
            if store.is_actor {
                self.build_actor_constructor(store)?;
            } else {
                self.build_store_constructor(store)?;
            }
        }
        
        // Emit a minimal main that initializes globals
        let main_fn = self
            .module
            .add_function("main", self.context.i32_type().fn_type(&[], false), None);
        let main_entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(main_entry);
        self.ensure_globals_initialized();
        if let Some(init_fn) = self.global_init_fn {
            self.builder.build_call(init_fn, &[], "init_globals").unwrap();
        }
        // Build a handler closure that calls __user_main once per message
        if let Some(user_main) = self.functions.get("main") {
            // Create trampoline function handler(self, msg)
            let handler_ty = self.context.void_type().fn_type(
                &[self.runtime.value_ptr_type.into(), self.runtime.value_ptr_type.into()],
                false,
            );
            let handler_fn = self.module.add_function("__coral_main_handler", handler_ty, None);
            let h_entry = self.context.append_basic_block(handler_fn, "entry");
            self.builder.position_at_end(h_entry);
            let _ = self.builder.build_call(*user_main, &[], "call_user_main");
            // Flush all persistent stores before exiting
            if self.has_persistent_stores {
                let _ = self.builder.build_call(self.runtime.store_save_all, &[], "save_stores");
            }
            // Signal that main handler is done
            let _ = self.builder.build_call(self.runtime.main_done_signal, &[], "main_done");
            self.builder.build_return(None).unwrap();

            // Return builder to main entry before constructing closure/actor
            self.builder.position_at_end(main_entry);

            // Make closure with null env/release
            let handler_closure = self.call_runtime_ptr(
                self.runtime.make_closure,
                &[
                    handler_fn.as_global_value().as_pointer_value().into(),
                    self.runtime.value_ptr_type.const_null().into(),
                    self.runtime.value_ptr_type.const_null().into(),
                ],
                "main_handler_closure",
            );
            let actor = self.call_runtime_ptr(
                self.runtime.actor_spawn,
                &[handler_closure.into()],
                "main_actor",
            );
            // Send unit to trigger handler
            let unit = self.wrap_unit();
            let unit_ptr = self.nb_to_ptr(unit);
            let _ = self.call_runtime_ptr(self.runtime.actor_send, &[actor.into(), unit_ptr.into()], "send_unit");
            // Wait for main actor to complete
            let _ = self.call_runtime_ptr(self.runtime.main_wait, &[], "wait_main");
        }
        self.builder.build_return(Some(&self.context.i32_type().const_int(0, false))).unwrap();

        // CC2.3: Finalize DWARF debug info before returning the module.
        if let Some(dbg) = &self.debug_ctx {
            dbg.builder.finalize();
        }

        Ok(self.module)
    }

    // ====================== C3.5: Dead Function Elimination ======================

    /// Check whether a function body contains a direct call to itself (recursion).
    /// Used by C3.1 inlining to avoid marking recursive functions as `alwaysinline`.
    fn body_calls_self(block: &Block, fn_name: &str) -> bool {
        for stmt in &block.statements {
            if Self::stmt_calls_self(stmt, fn_name) {
                return true;
            }
        }
        if let Some(ref expr) = block.value {
            if Self::expr_calls_self(expr, fn_name) {
                return true;
            }
        }
        false
    }

    fn stmt_calls_self(stmt: &Statement, name: &str) -> bool {
        match stmt {
            Statement::Binding(b) => Self::expr_calls_self(&b.value, name),
            Statement::Expression(e) => Self::expr_calls_self(e, name),
            Statement::Return(e, _) => Self::expr_calls_self(e, name),
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
                if Self::expr_calls_self(condition, name) || Self::body_calls_self(body, name) {
                    return true;
                }
                for (cond, blk) in elif_branches {
                    if Self::expr_calls_self(cond, name) || Self::body_calls_self(blk, name) {
                        return true;
                    }
                }
                if let Some(blk) = else_body {
                    if Self::body_calls_self(blk, name) {
                        return true;
                    }
                }
                false
            }
            Statement::While { condition, body, .. } => {
                Self::expr_calls_self(condition, name) || Self::body_calls_self(body, name)
            }
            Statement::For { iterable, body, .. } => {
                Self::expr_calls_self(iterable, name) || Self::body_calls_self(body, name)
            }
            Statement::ForKV { iterable, body, .. } => {
                Self::expr_calls_self(iterable, name) || Self::body_calls_self(body, name)
            }
            Statement::ForRange {
                start, end, step, body, ..
            } => {
                Self::expr_calls_self(start, name)
                    || Self::expr_calls_self(end, name)
                    || step.as_ref().map_or(false, |s| Self::expr_calls_self(s, name))
                    || Self::body_calls_self(body, name)
            }
            Statement::FieldAssign { target, value, .. } => {
                Self::expr_calls_self(target, name) || Self::expr_calls_self(value, name)
            }
            Statement::Break(_) | Statement::Continue(_) => false,
            Statement::PatternBinding { value, .. } => Self::expr_calls_self(value, name),
        }
    }

    fn expr_calls_self(expr: &Expression, name: &str) -> bool {
        match expr {
            Expression::Call { callee, args, .. } => {
                if let Expression::Identifier(ref id, ..) = **callee {
                    if id == name {
                        return true;
                    }
                }
                if Self::expr_calls_self(callee, name) {
                    return true;
                }
                args.iter().any(|a| Self::expr_calls_self(a, name))
            }
            Expression::Binary { left, right, .. } => {
                Self::expr_calls_self(left, name) || Self::expr_calls_self(right, name)
            }
            Expression::Unary { expr: operand, .. } => Self::expr_calls_self(operand, name),
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                Self::expr_calls_self(condition, name)
                    || Self::expr_calls_self(then_branch, name)
                    || Self::expr_calls_self(else_branch, name)
            }
            Expression::Lambda { body, .. } => Self::body_calls_self(body, name),
            Expression::Member { target, .. } => Self::expr_calls_self(target, name),
            Expression::Index { target, index, .. } => {
                Self::expr_calls_self(target, name) || Self::expr_calls_self(index, name)
            }
            Expression::Slice { target, start, end, .. } => {
                Self::expr_calls_self(target, name) || Self::expr_calls_self(start, name) || Self::expr_calls_self(end, name)
            }
            Expression::List(items, _) => items.iter().any(|i| Self::expr_calls_self(i, name)),
            Expression::Map(entries, _) => entries
                .iter()
                .any(|(k, v)| Self::expr_calls_self(k, name) || Self::expr_calls_self(v, name)),
            Expression::Pipeline { left, right, .. } => {
                Self::expr_calls_self(left, name) || Self::expr_calls_self(right, name)
            }
            Expression::Match(m) => {
                if Self::expr_calls_self(&m.value, name) {
                    return true;
                }
                for arm in &m.arms {
                    if let Some(guard) = &arm.guard {
                        if Self::expr_calls_self(guard, name) {
                            return true;
                        }
                    }
                    if Self::body_calls_self(&arm.body, name) {
                        return true;
                    }
                }
                if let Some(ref def) = m.default {
                    if Self::body_calls_self(def, name) {
                        return true;
                    }
                }
                false
            }
            Expression::ErrorPropagate { expr: inner, .. }
            | Expression::Throw { value: inner, .. } => Self::expr_calls_self(inner, name),
            Expression::Unsafe { block, .. } => Self::body_calls_self(block, name),
            _ => false,
        }
    }

    /// C3.4: Compute a cache key for a pure (side-effect-free) expression.
    /// Returns Some(key) if the expression is cacheable, None if it has side effects
    /// or isn't worth caching (e.g., simple variable reads or literals).
    /// Conservative: only caches pure computations (binary/unary on values).
    /// Does NOT cache member/index access (may read from mutable stores/maps).
    fn expr_cache_key(expr: &Expression) -> Option<String> {
        match expr {
            // Variable reads and literals are already cheap — not worth caching
            Expression::Identifier(..) | Expression::Integer(..) | Expression::Float(..)
            | Expression::Bool(..) | Expression::String(..) | Expression::Bytes(..) => None,
            // Binary on pure sub-expressions
            Expression::Binary { op, left, right, .. } => {
                let lk = Self::expr_cache_key_inner(left)?;
                let rk = Self::expr_cache_key_inner(right)?;
                Some(format!("({:?} {} {})", op, lk, rk))
            }
            // Unary on pure sub-expression
            Expression::Unary { op, expr: inner, .. } => {
                let ik = Self::expr_cache_key_inner(inner)?;
                Some(format!("({:?} {})", op, ik))
            }
            // Member and Index accesses are NOT cached — they may read from mutable
            // stores/maps and caching them could return stale values.
            Expression::Member { .. } | Expression::Index { .. } | Expression::Slice { .. } => None,
            // Only cache calls to known-pure global functions (not methods on objects)
            Expression::Call { callee, args, .. } => {
                match callee.as_ref() {
                    Expression::Identifier(name, _) if Self::is_pure_function(name) => {
                        let mut key = format!("{}(", name);
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 { key.push(','); }
                            key.push_str(&Self::expr_cache_key_inner(arg)?);
                        }
                        key.push(')');
                        Some(key)
                    }
                    _ => None,
                }
            }
            // Everything else (if/match/lambda/etc.) — not cached
            _ => None,
        }
    }

    /// Inner helper: produces a key string for any pure sub-expression,
    /// including simple variables and literals.
    fn expr_cache_key_inner(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(name, _) => Some(format!("v:{}", name)),
            Expression::Integer(n, _) => Some(format!("i:{}", n)),
            Expression::Float(f, _) => Some(format!("f:{}", f)),
            Expression::Bool(b, _) => Some(format!("b:{}", b)),
            Expression::String(s, _) => Some(format!("s:{}", s)),
            // For compound expressions, delegate to the outer key computation
            other => Self::expr_cache_key(other),
        }
    }

    /// Check if a function name refers to a known-pure builtin (no side effects).
    fn is_pure_function(name: &str) -> bool {
        matches!(name, "len" | "length" | "abs" | "sqrt" | "min" | "max"
            | "floor" | "ceil" | "round" | "to_string" | "to_number"
            | "type_of" | "is_number" | "is_string" | "is_bool" | "is_list" | "is_map")
    }

    /// Compute the set of function names transitively reachable from `main` and
    /// global initializers.  Functions not in this set are dead code and won't be
    /// emitted to LLVM IR, reducing binary size and compilation time.
    fn compute_reachable_functions(model: &SemanticModel) -> HashSet<String> {
        // If there's no main function, skip DCE — all functions are reachable.
        // This handles library/fragment compilation and tests without main.
        let has_main = model.functions.iter().any(|f| f.name == "main");
        if !has_main {
            let mut all: HashSet<String> = HashSet::new();
            for f in &model.functions {
                all.insert(f.name.clone());
            }
            for store in &model.stores {
                all.insert(format!("make_{}", store.name));
                for m in &store.methods {
                    all.insert(format!("{}_{}", store.name, m.name));
                }
            }
            for td in &model.type_defs {
                for v in &td.variants {
                    all.insert(v.name.clone());
                }
                for m in &td.methods {
                    all.insert(format!("{}_{}", td.name, m.name));
                }
            }
            return all;
        }

        // Build a map from function name → body for lookup
        let mut fn_bodies: HashMap<String, &Block> = HashMap::new();
        for f in &model.functions {
            fn_bodies.insert(f.name.clone(), &f.body);
        }
        for store in &model.stores {
            for method in &store.methods {
                let mangled = format!("{}_{}", store.name, method.name);
                fn_bodies.insert(mangled, &method.body);
            }
        }
        for type_def in &model.type_defs {
            for method in &type_def.methods {
                let mangled = format!("{}_{}", type_def.name, method.name);
                fn_bodies.insert(mangled, &method.body);
            }
        }

        // Collect all mangled method names keyed by their base method name,
        // so method calls can resolve to the right mangled forms.
        let mut method_name_map: HashMap<String, Vec<String>> = HashMap::new();
        for store in &model.stores {
            for method in &store.methods {
                let mangled = format!("{}_{}", store.name, method.name);
                method_name_map
                    .entry(method.name.clone())
                    .or_default()
                    .push(mangled);
            }
        }
        for type_def in &model.type_defs {
            for method in &type_def.methods {
                let mangled = format!("{}_{}", type_def.name, method.name);
                method_name_map
                    .entry(method.name.clone())
                    .or_default()
                    .push(mangled);
            }
        }

        // Collect all enum constructor names and store constructor names
        let mut all_names: HashSet<String> = HashSet::new();
        for f in &model.functions {
            all_names.insert(f.name.clone());
        }
        for store in &model.stores {
            all_names.insert(format!("make_{}", store.name));
            for method in &store.methods {
                all_names.insert(format!("{}_{}", store.name, method.name));
            }
        }
        for type_def in &model.type_defs {
            for variant in &type_def.variants {
                all_names.insert(variant.name.clone());
            }
            for method in &type_def.methods {
                all_names.insert(format!("{}_{}", type_def.name, method.name));
            }
        }

        // Worklist algorithm: start from "main" + globals, expand transitively
        let mut reachable: HashSet<String> = HashSet::new();
        let mut worklist: Vec<String> = vec!["main".to_string()];

        // Global initializers can reference any function
        for global in &model.globals {
            Self::collect_expr_refs(&global.value, &all_names, &method_name_map, &mut worklist);
        }

        while let Some(name) = worklist.pop() {
            if reachable.contains(&name) {
                continue;
            }
            reachable.insert(name.clone());

            // If this is a store constructor (make_X), also mark all methods of
            // that store as reachable since they may be called via method dispatch.
            if let Some(store_name) = name.strip_prefix("make_") {
                for store in &model.stores {
                    if store.name == store_name {
                        for method in &store.methods {
                            let mangled = format!("{}_{}", store.name, method.name);
                            worklist.push(mangled);
                        }
                    }
                }
                for type_def in &model.type_defs {
                    if type_def.name == store_name {
                        for method in &type_def.methods {
                            let mangled = format!("{}_{}", type_def.name, method.name);
                            worklist.push(mangled);
                        }
                    }
                }
            }

            if let Some(body) = fn_bodies.get(&name) {
                Self::collect_block_refs(body, &all_names, &method_name_map, &mut worklist);
            }
        }

        reachable
    }

    /// Collect function references from a block (statements + optional trailing expression).
    fn collect_block_refs(
        block: &Block,
        all_names: &HashSet<String>,
        method_map: &HashMap<String, Vec<String>>,
        worklist: &mut Vec<String>,
    ) {
        for stmt in &block.statements {
            Self::collect_stmt_refs(stmt, all_names, method_map, worklist);
        }
        if let Some(value) = &block.value {
            Self::collect_expr_refs(value, all_names, method_map, worklist);
        }
    }

    /// Collect function references from a statement.
    fn collect_stmt_refs(
        stmt: &Statement,
        all_names: &HashSet<String>,
        method_map: &HashMap<String, Vec<String>>,
        worklist: &mut Vec<String>,
    ) {
        match stmt {
            Statement::Binding(b) => {
                Self::collect_expr_refs(&b.value, all_names, method_map, worklist);
            }
            Statement::Expression(e) | Statement::Return(e, _) => {
                Self::collect_expr_refs(e, all_names, method_map, worklist);
            }
            Statement::If { condition, body, elif_branches, else_body, .. } => {
                Self::collect_expr_refs(condition, all_names, method_map, worklist);
                Self::collect_block_refs(body, all_names, method_map, worklist);
                for (cond, blk) in elif_branches {
                    Self::collect_expr_refs(cond, all_names, method_map, worklist);
                    Self::collect_block_refs(blk, all_names, method_map, worklist);
                }
                if let Some(eb) = else_body {
                    Self::collect_block_refs(eb, all_names, method_map, worklist);
                }
            }
            Statement::While { condition, body, .. } => {
                Self::collect_expr_refs(condition, all_names, method_map, worklist);
                Self::collect_block_refs(body, all_names, method_map, worklist);
            }
            Statement::For { iterable, body, .. } => {
                Self::collect_expr_refs(iterable, all_names, method_map, worklist);
                Self::collect_block_refs(body, all_names, method_map, worklist);
            }
            Statement::ForKV { iterable, body, .. } => {
                Self::collect_expr_refs(iterable, all_names, method_map, worklist);
                Self::collect_block_refs(body, all_names, method_map, worklist);
            }
            Statement::ForRange { start, end, step, body, .. } => {
                Self::collect_expr_refs(start, all_names, method_map, worklist);
                Self::collect_expr_refs(end, all_names, method_map, worklist);
                if let Some(s) = step {
                    Self::collect_expr_refs(s, all_names, method_map, worklist);
                }
                Self::collect_block_refs(body, all_names, method_map, worklist);
            }
            Statement::FieldAssign { target, value, .. } => {
                Self::collect_expr_refs(target, all_names, method_map, worklist);
                Self::collect_expr_refs(value, all_names, method_map, worklist);
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::PatternBinding { value, .. } => {
                Self::collect_expr_refs(value, all_names, method_map, worklist);
            }
        }
    }

    /// Collect function references from an expression.
    fn collect_expr_refs(
        expr: &Expression,
        all_names: &HashSet<String>,
        method_map: &HashMap<String, Vec<String>>,
        worklist: &mut Vec<String>,
    ) {
        match expr {
            Expression::Identifier(name, _) => {
                // A bare identifier may be a function reference (e.g., passed as an argument).
                if all_names.contains(name) {
                    worklist.push(name.clone());
                }
            }
            Expression::Call { callee, args, .. } => {
                // Check for method call pattern: obj.method(args)
                if let Expression::Member { target, property, .. } = callee.as_ref() {
                    // Mark all mangled methods that match this property name as reachable
                    if let Some(candidates) = method_map.get(property) {
                        for mangled in candidates {
                            worklist.push(mangled.clone());
                        }
                    }
                    Self::collect_expr_refs(target, all_names, method_map, worklist);
                } else {
                    Self::collect_expr_refs(callee, all_names, method_map, worklist);
                }
                for arg in args {
                    Self::collect_expr_refs(arg, all_names, method_map, worklist);
                }
            }
            Expression::Member { target, property, .. } => {
                // Property access alone (no call) — could be used as a method reference
                if let Some(candidates) = method_map.get(property) {
                    for mangled in candidates {
                        worklist.push(mangled.clone());
                    }
                }
                Self::collect_expr_refs(target, all_names, method_map, worklist);
            }
            Expression::Binary { left, right, .. } => {
                Self::collect_expr_refs(left, all_names, method_map, worklist);
                Self::collect_expr_refs(right, all_names, method_map, worklist);
            }
            Expression::Unary { expr: e, .. } => {
                Self::collect_expr_refs(e, all_names, method_map, worklist);
            }
            Expression::Spread(inner, _) => {
                Self::collect_expr_refs(inner, all_names, method_map, worklist);
            }
            Expression::Ternary { condition, then_branch, else_branch, .. } => {
                Self::collect_expr_refs(condition, all_names, method_map, worklist);
                Self::collect_expr_refs(then_branch, all_names, method_map, worklist);
                Self::collect_expr_refs(else_branch, all_names, method_map, worklist);
            }
            Expression::Pipeline { left, right, .. } => {
                Self::collect_expr_refs(left, all_names, method_map, worklist);
                Self::collect_expr_refs(right, all_names, method_map, worklist);
            }
            Expression::List(elems, _) => {
                for e in elems {
                    Self::collect_expr_refs(e, all_names, method_map, worklist);
                }
            }
            Expression::Map(pairs, _) => {
                for (k, v) in pairs {
                    Self::collect_expr_refs(k, all_names, method_map, worklist);
                    Self::collect_expr_refs(v, all_names, method_map, worklist);
                }
            }
            Expression::Lambda { body, .. } => {
                Self::collect_block_refs(body, all_names, method_map, worklist);
            }
            Expression::Match(m) => {
                Self::collect_expr_refs(&m.value, all_names, method_map, worklist);
                for arm in &m.arms {
                    Self::collect_match_pattern_refs(&arm.pattern, all_names, worklist);
                    if let Some(guard) = &arm.guard {
                        Self::collect_expr_refs(guard, all_names, method_map, worklist);
                    }
                    Self::collect_block_refs(&arm.body, all_names, method_map, worklist);
                }
                if let Some(default) = &m.default {
                    Self::collect_block_refs(default, all_names, method_map, worklist);
                }
            }
            Expression::ErrorPropagate { expr: e, .. } => {
                Self::collect_expr_refs(e, all_names, method_map, worklist);
            }
            Expression::Index { target, index, .. } => {
                Self::collect_expr_refs(target, all_names, method_map, worklist);
                Self::collect_expr_refs(index, all_names, method_map, worklist);
            }
            Expression::Slice { target, start, end, .. } => {
                Self::collect_expr_refs(target, all_names, method_map, worklist);
                Self::collect_expr_refs(start, all_names, method_map, worklist);
                Self::collect_expr_refs(end, all_names, method_map, worklist);
            }
            Expression::Throw { value, .. } => {
                Self::collect_expr_refs(value, all_names, method_map, worklist);
            }
            Expression::Unsafe { block, .. } => {
                Self::collect_block_refs(block, all_names, method_map, worklist);
            }
            Expression::InlineAsm { inputs, .. } => {
                for (_, e) in inputs {
                    Self::collect_expr_refs(e, all_names, method_map, worklist);
                }
            }
            Expression::PtrLoad { address, .. } => {
                Self::collect_expr_refs(address, all_names, method_map, worklist);
            }
            Expression::ListComprehension { body, iterable, condition, .. } => {
                Self::collect_expr_refs(iterable, all_names, method_map, worklist);
                Self::collect_expr_refs(body, all_names, method_map, worklist);
                if let Some(cond) = condition {
                    Self::collect_expr_refs(cond, all_names, method_map, worklist);
                }
            }
            Expression::MapComprehension { key, value, iterable, condition, .. } => {
                Self::collect_expr_refs(iterable, all_names, method_map, worklist);
                Self::collect_expr_refs(key, all_names, method_map, worklist);
                Self::collect_expr_refs(value, all_names, method_map, worklist);
                if let Some(cond) = condition {
                    Self::collect_expr_refs(cond, all_names, method_map, worklist);
                }
            }
            // Leaf expressions: no function references
            Expression::Unit
            | Expression::None(_)
            | Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::Bool(_, _)
            | Expression::String(_, _)
            | Expression::Bytes(_, _)
            | Expression::Placeholder(_, _)
            | Expression::TaxonomyPath { .. }
            | Expression::ErrorValue { .. } => {}
        }
    }

    /// Collect constructor names referenced in match patterns (e.g., `Some(v)`, `None`).
    fn collect_match_pattern_refs(
        pattern: &MatchPattern,
        all_names: &HashSet<String>,
        worklist: &mut Vec<String>,
    ) {
        match pattern {
            MatchPattern::Constructor { name, fields, .. } => {
                if all_names.contains(name) {
                    worklist.push(name.clone());
                }
                for f in fields {
                    Self::collect_match_pattern_refs(f, all_names, worklist);
                }
            }
            MatchPattern::Identifier(name) => {
                if all_names.contains(name) {
                    worklist.push(name.clone());
                }
            }
            MatchPattern::List(pats) => {
                for p in pats {
                    Self::collect_match_pattern_refs(p, all_names, worklist);
                }
            }
            MatchPattern::Or(alternatives) => {
                for alt in alternatives {
                    Self::collect_match_pattern_refs(alt, all_names, worklist);
                }
            }
            MatchPattern::Integer(_) | MatchPattern::Bool(_) | MatchPattern::String(_)
            | MatchPattern::Wildcard(_) | MatchPattern::Range { .. } | MatchPattern::Rest(..) => {}
        }
    }

    // ====================== End C3.5 ======================

    fn map_extern_type(&self, ann: &TypeAnnotation) -> Result<BasicTypeEnum<'ctx>, Diagnostic> {
        let name = ann
            .segments
            .last()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let ty = match name.as_str() {
            "f64" => self.f64_type.into(),
            "bool" => self.bool_type.into(),
            "u8" => self.i8_type.into(),
            "u16" => self.context.i16_type().into(),
            "u32" => self.context.i32_type().into(),
            "u64" | "usize" | "ptr" => self.usize_type.into(),
            _ => {
                return Err(Diagnostic::new(
                    format!("unsupported extern type `{}`", name),
                    ann.span,
                ))
            }
        };
        Ok(ty)
    }

    fn extern_function_type(
        &self,
        ret: Option<&BasicTypeEnum<'ctx>>,
        params: &[BasicTypeEnum<'ctx>],
    ) -> Result<FunctionType<'ctx>, Diagnostic> {
        let param_meta: Vec<BasicMetadataTypeEnum> = params.iter().map(|t| (*t).into()).collect();
        let fn_type = match ret {
            Some(BasicTypeEnum::IntType(t)) => t.fn_type(&param_meta, false),
            Some(BasicTypeEnum::FloatType(t)) => t.fn_type(&param_meta, false),
            Some(other) => {
                return Err(Diagnostic::new(
                    format!("extern return type not supported: `{}`", self.format_type_enum(*other)),
                    Span::default(),
                ))
            }
            None => self.context.void_type().fn_type(&param_meta, false),
        };
        Ok(fn_type)
    }

    fn format_type_enum(&self, ty: BasicTypeEnum<'ctx>) -> String {
        match ty {
            BasicTypeEnum::ArrayType(_) => "array".into(),
            BasicTypeEnum::FloatType(_) => "float".into(),
            BasicTypeEnum::IntType(_) => "int".into(),
            BasicTypeEnum::PointerType(_) => "ptr".into(),
            BasicTypeEnum::StructType(_) => "struct".into(),
            BasicTypeEnum::VectorType(_) => "vector".into(),
        }
    }

    fn build_function_body(
        &mut self,
        function: &Function,
        llvm_fn: FunctionValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        // C3.1: Small function inlining — annotate small, non-recursive functions
        // with LLVM's alwaysinline attribute so they get inlined at call sites.
        if function.name != "main" {
            let stmt_count = function.body.statements.len()
                + if function.body.value.is_some() { 1 } else { 0 };
            if stmt_count <= 5 && !Self::body_calls_self(&function.body, &function.name) {
                let kind_id = Attribute::get_named_enum_kind_id("alwaysinline");
                let attr = self.context.create_enum_attribute(kind_id, 0);
                llvm_fn.add_attribute(AttributeLoc::Function, attr);
            }
        }

        // CC2.3: Attach DISubprogram debug metadata to the function.
        let di_scope: Option<DIScope<'ctx>> = if let Some(dbg) = &self.debug_ctx {
            let (line, _) = dbg.line_index.line_col(function.body.span.start);
            let subprogram = dbg.builder.create_function(
                dbg.compile_unit.as_debug_info_scope(),
                &function.name,
                None,
                dbg.file,
                line as u32,
                dbg.fn_di_type,
                true,
                true,
                line as u32,
                DIFlags::PUBLIC,
                false,
            );
            llvm_fn.set_subprogram(subprogram);
            Some(subprogram.as_debug_info_scope())
        } else {
            None
        };

        let entry = self.context.append_basic_block(llvm_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: llvm_fn,
            loop_stack: Vec::new(),
            di_scope,
            fn_name: function.name.clone(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
        };

        // Parameters are NaN-boxed i64 values
        for (param, param_ast) in llvm_fn
            .get_param_iter()
            .zip(function.params.iter())
        {
            let value_nb = param.into_int_value();
            self.store_variable(&mut ctx, &param_ast.name, value_nb);
        }

        // C3.3: The function body's tail expression is in tail position
        ctx.in_tail_position = true;
        let block_value = self.emit_block(&mut ctx, &function.body)?;
        ctx.in_tail_position = false;
        // Return Value* pointer directly, not as f64
        self.builder.build_return(Some(&block_value)).unwrap();
        Ok(())
    }

    /// Build body for an actor @message method.
    /// Actor methods have a hidden first parameter (state Map) accessible as `self`.
    fn build_actor_method_body(
        &mut self,
        function: &Function,
        llvm_fn: FunctionValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let entry = self.context.append_basic_block(llvm_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: llvm_fn,
            loop_stack: Vec::new(),
            di_scope: None,
            fn_name: function.name.clone(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
        };

        // First param is the state (NaN-boxed i64), inject as `self`
        let state_val = llvm_fn.get_nth_param(0).unwrap().into_int_value();
        self.store_variable(&mut ctx, "self", state_val);

        // Remaining params are user params (starting at index 1) - NaN-boxed i64
        for (i, param_ast) in function.params.iter().enumerate() {
            let param = llvm_fn.get_nth_param((i + 1) as u32).unwrap();
            let value_nb = param.into_int_value();
            self.store_variable(&mut ctx, &param_ast.name, value_nb);
        }

        let block_value = self.emit_block(&mut ctx, &function.body)?;
        // Return Value* pointer directly
        self.builder.build_return(Some(&block_value)).unwrap();
        Ok(())
    }

    fn emit_block(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        block: &Block,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        for stmt in &block.statements {
            match stmt {
                Statement::Binding(binding) => {
                    let hint_byte = self
                        .allocation_hints
                        .get(&binding.name)
                        .copied()
                        .map(|h| self.alloc_hint_byte(h));
                    let value = match &binding.value {
                        Expression::List(elements, _) => {
                            self.emit_list_literal_hinted(ctx, elements, hint_byte)?
                        }
                        Expression::Map(entries, _) => {
                            self.emit_map_literal_hinted(ctx, entries, hint_byte)?
                        }
                        _ => self.emit_expression(ctx, &binding.value)?,
                    };
                    self.store_variable(ctx, &binding.name, value);
                    // C3.4: Invalidate CSE cache entries involving this variable
                    let var_key = format!("v:{}", binding.name);
                    ctx.cse_cache.retain(|k, _| !k.contains(&var_key));
                }
                Statement::Expression(expr) => {
                    let _ = self.emit_expression(ctx, expr)?;
                }
                Statement::Return(expr, _) => {
                    // C3.3: Return expression is in tail position
                    ctx.in_tail_position = true;
                    let value = self.emit_expression(ctx, expr)?;
                    ctx.in_tail_position = false;
                    self.builder.build_return(Some(&value)).unwrap();
                    // Return a zero sentinel without emitting any LLVM instruction.
                    // const_zero() is a compile-time constant, so no instruction is added
                    // after the `ret` terminator.
                    return Ok(self.runtime.value_i64_type.const_zero());
                }
                Statement::If { condition, body, elif_branches, else_body, .. } => {
                    let function = ctx.function;
                    let cond_is_bool = self.expr_is_bool(condition);
                    let cond_value = self.emit_expression(ctx, condition)?;
                    let cond_bool = if cond_is_bool {
                        self.value_to_bool_fast(cond_value)
                    } else {
                        self.value_to_bool(cond_value)
                    };

                    let then_bb = self.context.append_basic_block(function, "if_then");
                    let merge_bb = self.context.append_basic_block(function, "if_merge");

                    // Track (value, source_block) pairs for PHI node
                    let mut phi_incoming: Vec<(IntValue<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();

                    // Determine initial else target
                    let first_else_bb = if elif_branches.is_empty() && else_body.is_none() {
                        merge_bb
                    } else {
                        self.context.append_basic_block(function, "if_else")
                    };
                    self.builder.build_conditional_branch(cond_bool, then_bb, first_else_bb).unwrap();

                    // Emit then body
                    self.builder.position_at_end(then_bb);
                    ctx.cse_cache.clear(); // C3.4: Clear at control flow boundary
                    let then_value = self.emit_block(ctx, body)?;
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        let then_end_bb = self.builder.get_insert_block().unwrap();
                        phi_incoming.push((then_value, then_end_bb));
                        self.builder.build_unconditional_branch(merge_bb).unwrap();
                    }

                    // Emit elif/else chain
                    if !elif_branches.is_empty() || else_body.is_some() {
                        let mut current_else_bb = first_else_bb;
                        for (i, (elif_cond, elif_body)) in elif_branches.iter().enumerate() {
                            self.builder.position_at_end(current_else_bb);
                            let elif_is_bool = self.expr_is_bool(elif_cond);
                            let elif_cond_val = self.emit_expression(ctx, elif_cond)?;
                            let elif_cond_bool = if elif_is_bool {
                                self.value_to_bool_fast(elif_cond_val)
                            } else {
                                self.value_to_bool(elif_cond_val)
                            };

                            let elif_then_bb = self.context.append_basic_block(function, &format!("elif_then_{i}"));
                            let next_else_bb = if i + 1 < elif_branches.len() || else_body.is_some() {
                                self.context.append_basic_block(function, &format!("elif_else_{i}"))
                            } else {
                                merge_bb
                            };
                            self.builder.build_conditional_branch(elif_cond_bool, elif_then_bb, next_else_bb).unwrap();

                            self.builder.position_at_end(elif_then_bb);
                            ctx.cse_cache.clear(); // C3.4
                            let elif_value = self.emit_block(ctx, elif_body)?;
                            if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                                let elif_end_bb = self.builder.get_insert_block().unwrap();
                                phi_incoming.push((elif_value, elif_end_bb));
                                self.builder.build_unconditional_branch(merge_bb).unwrap();
                            }
                            current_else_bb = next_else_bb;
                        }
                        if let Some(else_block) = else_body {
                            self.builder.position_at_end(current_else_bb);
                            ctx.cse_cache.clear(); // C3.4
                            let else_value = self.emit_block(ctx, else_block)?;
                            if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                                let else_end_bb = self.builder.get_insert_block().unwrap();
                                phi_incoming.push((else_value, else_end_bb));
                                self.builder.build_unconditional_branch(merge_bb).unwrap();
                            }
                        }
                    }

                    // If no else body, the implicit fall-through produces unit
                    if else_body.is_none() && (elif_branches.is_empty() || elif_branches.last().is_some()) {
                        // The merge_bb is the fall-through from the last condition check
                        // when there's no else block. We need to add a unit value for that path.
                        // Actually, we need a dedicated block for this since merge_bb is the target.
                        // The fall-through already branches to merge_bb via the conditional branch.
                        // We handle this by checking if merge_bb has predecessors without phi entries.
                    }

                    self.builder.position_at_end(merge_bb);

                    // Build PHI node if we have incoming values from branches
                    if !phi_incoming.is_empty() && else_body.is_some() {
                        let phi = self
                            .builder
                            .build_phi(self.runtime.value_i64_type, "if_phi")
                            .unwrap();
                        for (val, bb) in &phi_incoming {
                            phi.add_incoming(&[(val as &dyn BasicValue<'ctx>, *bb)]);
                        }
                        // Store the if-expression result as __if_result for potential use
                        let if_result = phi.as_basic_value().into_int_value();
                        self.store_variable(ctx, "__if_result", if_result);
                    }
                }
                Statement::While { condition, body, .. } => {
                    let function = ctx.function;
                    let loop_header = self.context.append_basic_block(function, "while_cond");
                    let loop_body = self.context.append_basic_block(function, "while_body");
                    let loop_exit = self.context.append_basic_block(function, "while_exit");

                    self.builder.build_unconditional_branch(loop_header).unwrap();

                    // Condition check
                    self.builder.position_at_end(loop_header);
                    let while_cond_is_bool = self.expr_is_bool(condition);
                    let cond_value = self.emit_expression(ctx, condition)?;
                    let cond_bool = if while_cond_is_bool {
                        self.value_to_bool_fast(cond_value)
                    } else {
                        self.value_to_bool(cond_value)
                    };
                    self.builder.build_conditional_branch(cond_bool, loop_body, loop_exit).unwrap();

                    // Body
                    self.builder.position_at_end(loop_body);
                    ctx.cse_cache.clear(); // C3.4
                    ctx.loop_stack.push((loop_header, loop_exit));
                    self.emit_block(ctx, body)?;
                    ctx.loop_stack.pop();
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }

                    self.builder.position_at_end(loop_exit);
                }
                Statement::For { variable, iterable, body, .. } => {
                    let function = ctx.function;
                    let iter_value = self.emit_expression(ctx, iterable)?;
                    // Create iterator from the iterable (works for lists and maps)
                    let iter_ptr = self.nb_to_ptr(iter_value);
                    let iter = self.call_runtime_ptr(
                        self.runtime.value_iter,
                        &[iter_ptr.into()],
                        "for_iter",
                    );

                    let loop_header = self.context.append_basic_block(function, "for_cond");
                    let loop_body = self.context.append_basic_block(function, "for_body");
                    let loop_exit = self.context.append_basic_block(function, "for_exit");

                    self.builder.build_unconditional_branch(loop_header).unwrap();

                    // Get next element and check if iteration is done (Unit tag == 7)
                    self.builder.position_at_end(loop_header);
                    let elem_ptr = self.call_runtime_ptr(
                        self.runtime.value_iter_next,
                        &[iter.into()],
                        "for_next",
                    );
                    // Read the tag byte at offset 0 of the Value struct
                    let tag_ptr = self.builder.build_pointer_cast(
                        elem_ptr,
                        self.i8_type.ptr_type(AddressSpace::default()),
                        "tag_ptr",
                    ).unwrap();
                    let tag_val = self.builder.build_load(self.i8_type, tag_ptr, "tag_val")
                        .unwrap().into_int_value();
                    let unit_tag = self.i8_type.const_int(7, false); // Unit = 7
                    let is_done = self.builder.build_int_compare(
                        IntPredicate::EQ, tag_val, unit_tag, "for_done",
                    ).unwrap();
                    self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

                    // Body: bind loop variable (convert element pointer to NaN-boxed)
                    self.builder.position_at_end(loop_body);
                    ctx.cse_cache.clear(); // C3.4
                    let elem_nb = self.ptr_to_nb(elem_ptr);
                    self.store_variable(ctx, variable, elem_nb);
                    ctx.loop_stack.push((loop_header, loop_exit));
                    self.emit_block(ctx, body)?;
                    ctx.loop_stack.pop();
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }

                    // Release the iterator after the loop
                    self.builder.position_at_end(loop_exit);
                    self.call_runtime_void(self.runtime.value_release, &[iter.into()], "release_iter");
                }
                Statement::ForKV { key_var, value_var, iterable, body, .. } => {
                    let function = ctx.function;
                    let iter_value = self.emit_expression(ctx, iterable)?;
                    // Get map entries as a list of [key, value] pairs
                    let entries_list = self.call_bridged(self.runtime.map_entries, &[iter_value], "forkv_entries");
                    // Create iterator over the entries list
                    let entries_ptr = self.nb_to_ptr(entries_list);
                    let iter = self.call_runtime_ptr(
                        self.runtime.value_iter,
                        &[entries_ptr.into()],
                        "forkv_iter",
                    );

                    let loop_header = self.context.append_basic_block(function, "forkv_cond");
                    let loop_body = self.context.append_basic_block(function, "forkv_body");
                    let loop_exit = self.context.append_basic_block(function, "forkv_exit");

                    self.builder.build_unconditional_branch(loop_header).unwrap();

                    // Get next entry pair and check if iteration is done (Unit tag == 7)
                    self.builder.position_at_end(loop_header);
                    let elem_ptr = self.call_runtime_ptr(
                        self.runtime.value_iter_next,
                        &[iter.into()],
                        "forkv_next",
                    );
                    let tag_ptr = self.builder.build_pointer_cast(
                        elem_ptr,
                        self.i8_type.ptr_type(AddressSpace::default()),
                        "tag_ptr",
                    ).unwrap();
                    let tag_val = self.builder.build_load(self.i8_type, tag_ptr, "tag_val")
                        .unwrap().into_int_value();
                    let unit_tag = self.i8_type.const_int(7, false);
                    let is_done = self.builder.build_int_compare(
                        IntPredicate::EQ, tag_val, unit_tag, "forkv_done",
                    ).unwrap();
                    self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

                    // Body: extract key and value from the [key, value] pair
                    self.builder.position_at_end(loop_body);
                    ctx.cse_cache.clear(); // C3.4
                    let pair_nb = self.ptr_to_nb(elem_ptr);
                    let index_zero = self.wrap_number(self.f64_type.const_float(0.0));
                    let index_one = self.wrap_number(self.f64_type.const_float(1.0));
                    let key_nb = self.call_bridged(self.runtime.list_get, &[pair_nb, index_zero], "forkv_key");
                    let val_nb = self.call_bridged(self.runtime.list_get, &[pair_nb, index_one], "forkv_val");
                    self.store_variable(ctx, key_var, key_nb);
                    self.store_variable(ctx, value_var, val_nb);
                    ctx.loop_stack.push((loop_header, loop_exit));
                    self.emit_block(ctx, body)?;
                    ctx.loop_stack.pop();
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }

                    // Release the iterator after the loop
                    self.builder.position_at_end(loop_exit);
                    self.call_runtime_void(self.runtime.value_release, &[iter.into()], "release_iter");
                }
                Statement::ForRange { variable, start, end, step, body, .. } => {
                    // Efficient counted loop: for i in start to end [step s]
                    // All arithmetic done as f64, NaN-boxed only for body access
                    let function = ctx.function;
                    let start_val = self.emit_expression(ctx, start)?;
                    let end_val = self.emit_expression(ctx, end)?;
                    let step_f64 = if let Some(step_expr) = step {
                        let step_nb = self.emit_expression(ctx, step_expr)?;
                        self.value_to_number(step_nb)
                    } else {
                        // Default step: 1.0
                        self.f64_type.const_float(1.0)
                    };

                    // Extract f64 values for direct arithmetic
                    let start_f64 = self.value_to_number(start_val);
                    let end_f64 = self.value_to_number(end_val);

                    // Allocate loop counter as f64
                    let counter_alloca = self.builder.build_alloca(self.f64_type, "for_range_counter").unwrap();
                    self.builder.build_store(counter_alloca, start_f64).unwrap();

                    let loop_header = self.context.append_basic_block(function, "for_range_cond");
                    let loop_body = self.context.append_basic_block(function, "for_range_body");
                    let loop_exit = self.context.append_basic_block(function, "for_range_exit");

                    self.builder.build_unconditional_branch(loop_header).unwrap();

                    // Check: counter < end (exclusive upper bound, like Python range)
                    self.builder.position_at_end(loop_header);
                    let current = self.builder.build_load(self.f64_type, counter_alloca, "cur")
                        .unwrap().into_float_value();
                    let is_done = self.builder.build_float_compare(
                        inkwell::FloatPredicate::OGE, current, end_f64, "for_range_done",
                    ).unwrap();
                    self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

                    // Body: wrap counter as NaN-boxed number, bind to variable
                    self.builder.position_at_end(loop_body);
                    ctx.cse_cache.clear(); // C3.4
                    let counter_nb = self.wrap_number(current);
                    self.store_variable(ctx, variable, counter_nb);
                    ctx.loop_stack.push((loop_header, loop_exit));
                    self.emit_block(ctx, body)?;
                    ctx.loop_stack.pop();

                    // Increment counter: counter += step
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        let updated_current = self.builder.build_load(self.f64_type, counter_alloca, "cur_upd")
                            .unwrap().into_float_value();
                        let next = self.builder.build_float_add(updated_current, step_f64, "next_counter").unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }

                    self.builder.position_at_end(loop_exit);
                }
                Statement::FieldAssign { target, field, value, .. } => {
                    // self.field is value → coral_map_set(self, "field", value)
                    let target_value = self.emit_expression(ctx, &target)?;
                    let key_value = self.emit_string_literal(&field);
                    let new_value = self.emit_expression(ctx, &value)?;
                    
                    // Bridge to pointer-based API
                    let target_ptr = self.nb_to_ptr(target_value);
                    let key_ptr = self.nb_to_ptr(key_value);
                    let new_ptr = self.nb_to_ptr(new_value);
                    
                    // Handle reference field retain/release for proper refcounting
                    if let Expression::Identifier(name, _) = &target {
                        if name == "self" {
                            let is_ref = self.reference_fields.iter().any(|(_, f)| f == field.as_str());
                            if is_ref {
                                // Release old value before setting new one
                                let old_value = self.call_runtime_ptr(
                                    self.runtime.map_get,
                                    &[target_ptr.into(), key_ptr.into()],
                                    "old_field_value",
                                );
                                self.call_runtime_void(self.runtime.value_release, &[old_value.into()], "release_old");
                                self.call_runtime_void(self.runtime.value_retain, &[new_ptr.into()], "retain_new");
                            }
                        }
                    }
                    
                    self.call_runtime_ptr(
                        self.runtime.map_set,
                        &[target_ptr.into(), key_ptr.into(), new_ptr.into()],
                        "map_set_field",
                    );
                    // C3.4: Field mutation invalidates all CSE entries involving the target
                    if let Expression::Identifier(name, _) = &target {
                        let var_key = format!("v:{}", name);
                        ctx.cse_cache.retain(|k, _| !k.contains(&var_key));
                    } else {
                        // Conservative: clear entire cache on non-trivial target mutation
                        ctx.cse_cache.clear();
                    }
                }
                Statement::Break(_) => {
                    if let Some(&(_, loop_exit)) = ctx.loop_stack.last() {
                        self.builder.build_unconditional_branch(loop_exit).unwrap();
                    }
                    // After break, no more code in this block is reachable
                    let function = ctx.function;
                    let unreachable_bb = self.context.append_basic_block(function, "after_break");
                    self.builder.position_at_end(unreachable_bb);
                }
                Statement::Continue(_) => {
                    if let Some(&(loop_header, _)) = ctx.loop_stack.last() {
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }
                    let function = ctx.function;
                    let unreachable_bb = self.context.append_basic_block(function, "after_continue");
                    self.builder.position_at_end(unreachable_bb);
                }
                // S2.4: Destructuring pattern binding
                Statement::PatternBinding { pattern, value, .. } => {
                    let val = self.emit_expression(ctx, value)?;
                    self.bind_pattern_variables(ctx, val, pattern);
                }
            }
        }

        if let Some(expr) = &block.value {
            // C3.3: Block tail value inherits the current tail position
            // (already set by caller if this is a function body return)
            self.emit_expression(ctx, expr.as_ref())
        } else {
            Ok(self.wrap_number(self.f64_type.const_float(0.0)))
        }
    }

    fn emit_expression(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        // CC2.3: Set debug location from the expression's span.
        if let Some(scope) = ctx.di_scope {
            self.set_debug_location(expr.span(), scope);
        }
        // C3.4: Common subexpression elimination — check cache for pure expressions.
        if let Some(key) = Self::expr_cache_key(expr) {
            if let Some(&cached) = ctx.cse_cache.get(&key) {
                return Ok(cached);
            }
            // Fall through to compute, then cache the result below.
            let result = self.emit_expression_inner(ctx, expr)?;
            ctx.cse_cache.insert(key, result);
            return Ok(result);
        }
        self.emit_expression_inner(ctx, expr)
    }

    /// Inner expression emit — called by emit_expression after CSE check.
    fn emit_expression_inner(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match expr {
            Expression::Integer(value, _) => Ok(self.wrap_number(self.f64_type.const_float(*value as f64))),
            Expression::Float(value, _) => Ok(self.wrap_number(self.f64_type.const_float(*value))),
            Expression::Bool(value, _) => Ok(self.wrap_bool(self.boolean_to_int(*value))),
            Expression::String(value, _) => Ok(self.emit_string_literal(value)),
            Expression::Bytes(value, _) => Ok(self.emit_bytes_literal(value)),
            Expression::List(elements, _) => self.emit_list_literal(ctx, elements),
            Expression::Map(entries, _) => self.emit_map_literal(ctx, entries),
            Expression::Identifier(name, _) => self.load_variable(ctx, name),
            Expression::TaxonomyPath { segments, .. } => {
                let mut text = String::from("!!");
                text.push_str(&segments.join(":"));
                Ok(self.emit_string_literal(&text))
            }
            Expression::Throw { span, .. } => Err(Diagnostic::new(
                "throw expressions are not lowered yet",
                *span,
            )),
            Expression::Lambda { params, body, span } =>
                self.emit_lambda(ctx, params, body, *span),
            Expression::Placeholder(_, span) => Err(Diagnostic::new(
                "placeholder expressions require higher-order lowering, which is not implemented yet",
                *span,
            )),
            Expression::Binary { op, left, right, .. } => match op {
                BinaryOp::And | BinaryOp::Or =>
                    self.emit_logical_binary(ctx, *op, left, right),
                _ => {
                    // C3.3: Operands of a binary op are NOT in tail position
                    let saved_tail = ctx.in_tail_position;
                    ctx.in_tail_position = false;
                    // C2.1/C2.2: Check if both operands have known numeric/bool types
                    // for specialization (avoids runtime FFI for Add, Equals, NotEquals).
                    let both_numeric = self.expr_is_numeric(left) && self.expr_is_numeric(right);
                    let lhs = self.emit_expression(ctx, left)?;
                    let rhs = self.emit_expression(ctx, right)?;
                    ctx.in_tail_position = saved_tail;
                    self.emit_numeric_binary(*op, lhs, rhs, both_numeric)
                }
            },
            Expression::Unary { op, expr, .. } => {
                // C3.3: Operand of unary expression is not in tail position
                let saved_tail = ctx.in_tail_position;
                ctx.in_tail_position = false;
                let is_bool = self.expr_is_bool(expr);
                let value = self.emit_expression(ctx, expr)?;
                ctx.in_tail_position = saved_tail;
                match op {
                    UnaryOp::Neg => {
                        let as_number = self.value_to_number(value);
                        let neg = self.builder.build_float_neg(as_number, "neg").unwrap();
                        Ok(self.wrap_number(neg))
                    }
                    UnaryOp::Not => {
                        // C2.2: Use fast bool extraction when operand is known-boolean.
                        let predicate = if is_bool {
                            self.value_to_bool_fast(value)
                        } else {
                            self.value_to_bool(value)
                        };
                        let inverted = self.builder.build_not(predicate, "not").unwrap();
                        Ok(self.wrap_bool(inverted))
                    }
                    UnaryOp::BitNot => {
                        let ptr = self.nb_to_ptr(value);
                        let result = self.call_runtime_ptr(
                            self.runtime.value_bitnot,
                            &[ptr.into()],
                            "bitnot",
                        );
                        Ok(self.ptr_to_nb(result))
                    }
                }
            }
            Expression::Call { callee, args, .. } => {
                if let Expression::Member { target, property, span } = callee.as_ref() {
                    return self.emit_member_call(ctx, target, property, args, *span);
                }
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    // Check builtins first (includes actor constructors)
                    if let Some(result) = self.emit_builtin_call(name, args, ctx, expr.span())? {
                        return Ok(result);
                    }
                    // Extern functions with typed lowering
                    if let Some(sig) = self.extern_sigs.get(name).cloned() {
                        if sig.param_types.len() != args.len() {
                            return Err(Diagnostic::new(
                                format!(
                                    "extern call arity mismatch: expected {}, found {}",
                                    sig.param_types.len(),
                                    args.len()
                                ),
                                expr.span(),
                            ));
                        }
                        let mut lowered_args = Vec::new();
                        for (arg_expr, ty) in args.iter().zip(sig.param_types.iter()) {
                            let value = self.emit_expression(ctx, arg_expr)?;
                            lowered_args.push(self.cast_extern_arg(value, *ty, arg_expr.span())?);
                        }
                        let call = self
                            .builder
                            .build_call(sig.function, &lowered_args, "extern_call")
                            .unwrap();
                        if let Some(ret_ty) = &sig.ret_type {
                            let ret_val = call
                                .try_as_basic_value()
                                .left()
                                .ok_or_else(|| Diagnostic::new("extern call produced no value", expr.span()))?;
                            return self.wrap_extern_return(ret_val, *ret_ty, expr.span());
                        } else {
                            return Ok(self.wrap_unit());
                        }
                    }
                    // Then check user functions
                    if let Some(&function) = self.functions.get(name) {
                        // Pass Value* pointers directly, not as f64
                        let mut arg_values = Vec::new();
                        for arg in args {
                            // C3.3: Arguments are never in tail position themselves
                            let saved_tail = ctx.in_tail_position;
                            ctx.in_tail_position = false;
                            let value = self.emit_expression(ctx, arg)?;
                            ctx.in_tail_position = saved_tail;
                            arg_values.push(value);
                        }
                        let metadata_args: Vec<BasicMetadataValueEnum> =
                            arg_values.iter().map(|v| (*v).into()).collect();
                        let call = self
                            .builder
                            .build_call(function, &metadata_args, "call")
                            .unwrap();
                        // C3.3: Mark direct self-recursive calls in tail position
                        if ctx.in_tail_position && name == &ctx.fn_name {
                            call.set_tail_call(true);
                        }
                        // Return is NaN-boxed i64
                        let value = call
                            .try_as_basic_value()
                            .left()
                            .ok_or_else(|| Diagnostic::new("call produced no value", expr.span()))?
                            .into_int_value();
                        Ok(value)
                    } else if let Some((enum_name, expected_field_count)) = self.enum_constructors.get(name).cloned() {
                        // Enum constructor call - create tagged value
                        if args.len() != expected_field_count {
                            return Err(Diagnostic::new(
                                format!(
                                    "enum constructor `{}::{}` expects {} argument(s), found {}",
                                    enum_name, name, expected_field_count, args.len()
                                ),
                                expr.span(),
                            ));
                        }
                        self.emit_enum_constructor(ctx, name, args)
                    } else if ctx.variables.contains_key(name) || ctx.variable_allocas.contains_key(name) {
                        // Local variable - might be a closure stored in a binding.
                        let callee_value = self.emit_expression(ctx, callee)?;
                        self.emit_closure_call(ctx, callee_value, args)
                    } else {
                        Err(Diagnostic::new(
                            format!("unknown function `{}`", name),
                            callee.span(),
                        ))
                    }
                } else {
                    let callee_value = self.emit_expression(ctx, callee)?;
                    self.emit_closure_call(ctx, callee_value, args)
                }
            }
            Expression::Member { target, property, span } =>
                self.emit_member_expression(ctx, target, property, *span),
            Expression::Index { target, index, span: _ } => {
                let target_val = self.emit_expression(ctx, target)?;
                let index_val = self.emit_expression(ctx, index)?;
                // Bridge via nb_to_ptr for old API, convert result back
                let target_ptr = self.nb_to_ptr(target_val);
                let index_ptr = self.nb_to_ptr(index_val);
                let result = self.call_runtime_ptr(
                    self.runtime.list_get,
                    &[target_ptr.into(), index_ptr.into()],
                    "subscript",
                );
                Ok(self.ptr_to_nb(result))
            }
            Expression::Slice { target, start, end, .. } => {
                let target_val = self.emit_expression(ctx, target)?;
                let start_val = self.emit_expression(ctx, start)?;
                let end_val = self.emit_expression(ctx, end)?;
                let result = self.call_bridged(self.runtime.list_slice, &[target_val, start_val, end_val], "slice");
                Ok(result)
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.emit_ternary(ctx, condition, then_branch, else_branch),
            Expression::Match(match_expr) => self.emit_match(ctx, match_expr),
            Expression::Unit => Ok(self.wrap_unit()),
            Expression::None(_) => {
                Ok(self.wrap_none())
            }
            Expression::InlineAsm { template, inputs, span, .. } => {
                let mut arg_vals: Vec<BasicMetadataValueEnum> = Vec::with_capacity(inputs.len());
                let mut constraint_parts: Vec<&str> = Vec::with_capacity(inputs.len());
                for (constraint, expr) in inputs {
                    let val = self.emit_expression(ctx, expr)?;
                    arg_vals.push(self.value_to_number(val).into());
                    constraint_parts.push(constraint.as_str());
                }
                let constraint_str = constraint_parts.join(",");
                match self.inline_asm_mode {
                    InlineAsmMode::Deny => Err(Diagnostic::new(
                        format!("inline asm not supported in codegen yet: `{template}`"),
                        *span,
                    )),
                    InlineAsmMode::Noop => Ok(self.wrap_unit()),
                    InlineAsmMode::Emit => {
                        self.emit_inline_asm(template, &constraint_str, &arg_vals, *span)?;
                        Ok(self.wrap_unit())
                    }
                }
            }
            Expression::PtrLoad { address, span } => {
                // Evaluate address expression as number, cast to pointer, load f64, wrap as number.
                let addr_val = self.emit_expression(ctx, address)?;
                let addr_num = self.value_to_number(addr_val);
                let addr_int = self
                    .builder
                    .build_bitcast(addr_num, self.usize_type, "addr_usize")
                    .map_err(|e| Diagnostic::new(format!("ptr load bitcast failed: {e}"), *span))?;
                let addr_ptr = self
                    .builder
                    .build_int_to_ptr(
                        addr_int
                            .into_int_value(),
                        self.f64_type.ptr_type(AddressSpace::default()),
                        "addr_ptr",
                    )
                    .map_err(|e| Diagnostic::new(format!("ptr load int_to_ptr failed: {e}"), *span))?;
                let loaded = self
                    .builder
                    .build_load(self.f64_type, addr_ptr, "ptr_load")
                    .map_err(|e| Diagnostic::new(format!("ptr load failed: {e}"), *span))?
                    .into_float_value();
                Ok(self.wrap_number(loaded))
            }
            Expression::Unsafe { block, .. } => {
                // Unsafe is transparent to codegen for now.
                self.emit_block(ctx, block)
            }
            Expression::Pipeline { left, right, span } => {
                // Desugar pipeline: `a ~ f(args)` becomes `f(a, args)`
                // With explicit $ placeholder: `a ~ f($, extra)` becomes `f(a, extra)`
                match right.as_ref() {
                    Expression::Call { callee, args, span: call_span } => {
                        // Check if any argument is a placeholder (or contains one)
                        let has_placeholder = args.iter().any(|arg| self.contains_placeholder(arg));
                        
                        let new_args = if has_placeholder {
                            // Replace $ placeholders with the piped value
                            args.iter()
                                .map(|arg| self.replace_placeholder_with(arg, left.as_ref()))
                                .collect()
                        } else {
                            // No placeholder - prepend left as first argument
                            let mut new_args = vec![left.as_ref().clone()];
                            new_args.extend(args.iter().cloned());
                            new_args
                        };
                        
                        let desugared = Expression::Call {
                            callee: callee.clone(),
                            args: new_args,
                            span: *call_span,
                        };
                        self.emit_expression(ctx, &desugared)
                    }
                    Expression::Identifier(name, id_span) => {
                        // `a ~ f` becomes `f(a)`
                        let desugared = Expression::Call {
                            callee: Box::new(Expression::Identifier(name.clone(), *id_span)),
                            args: vec![left.as_ref().clone()],
                            span: *span,
                        };
                        self.emit_expression(ctx, &desugared)
                    }
                    _ => Err(Diagnostic::new(
                        "pipeline right-hand side must be a function call or identifier",
                        *span,
                    ))
                }
            }
            Expression::ErrorValue { path, span: _ } => {
                // Create an error value with the given path
                let error_name = path.join(":");
                let name_bytes = error_name.as_bytes();
                
                // Create a global constant for the error name string
                let name_array = self.context.const_string(name_bytes, false);
                let name_global = self.module.add_global(
                    name_array.get_type(),
                    Some(AddressSpace::default()),
                    &format!("err_name_{}", error_name.replace(':', "_")),
                );
                name_global.set_linkage(inkwell::module::Linkage::Private);
                name_global.set_initializer(&name_array);
                name_global.set_constant(true);
                
                // Get pointer to name string
                let name_ptr = self.builder.build_pointer_cast(
                    name_global.as_pointer_value(),
                    self.i8_type.ptr_type(AddressSpace::default()),
                    "err_name_ptr",
                ).unwrap();
                
                // Error code: could be derived from the error definition, for now use 0
                let error_code = self.context.i32_type().const_int(0, false);
                let name_len = self.usize_type.const_int(name_bytes.len() as u64, false);
                
                // Call coral_make_error(code, name_ptr, name_len)
                let err_ptr = self.call_runtime_ptr(
                    self.runtime.make_error,
                    &[error_code.into(), name_ptr.into(), name_len.into()],
                    "make_error",
                );
                Ok(self.ptr_to_nb(err_ptr))
            }
            Expression::Spread(inner, _) => {
                // Spread outside list literal context: just emit the inner expression
                self.emit_expression(ctx, inner)
            }
            Expression::ListComprehension { body, var, iterable, condition, span: _ } => {
                let function = ctx.function;
                let list_value = self.emit_expression(ctx, iterable)?;

                // Get list length as f64
                let len_nb = self.call_bridged(self.runtime.list_length, &[list_value], "lc_len");
                let len_f64 = self.value_to_number(len_nb);

                // Create empty output list: coral_make_list(null, 0)
                let null_ptr = self.runtime.value_ptr_type
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null();
                let zero_usize = self.usize_type.const_int(0, false);
                let out_list_ptr = self.call_runtime_ptr(
                    self.runtime.make_list,
                    &[null_ptr.into(), zero_usize.into()],
                    "lc_out_list",
                );
                let out_list_nb = self.ptr_to_nb(out_list_ptr);

                // Alloca for output list (mutated by push)
                let out_alloca = self.builder.build_alloca(self.runtime.value_i64_type, "lc_out_alloca").unwrap();
                self.builder.build_store(out_alloca, out_list_nb).unwrap();

                // Counter alloca
                let counter_alloca = self.builder.build_alloca(self.f64_type, "lc_counter").unwrap();
                self.builder.build_store(counter_alloca, self.f64_type.const_float(0.0)).unwrap();

                let loop_header = self.context.append_basic_block(function, "lc_cond");
                let loop_body = self.context.append_basic_block(function, "lc_body");
                let loop_exit = self.context.append_basic_block(function, "lc_exit");

                self.builder.build_unconditional_branch(loop_header).unwrap();

                // Header: check counter < length
                self.builder.position_at_end(loop_header);
                let current = self.builder.build_load(self.f64_type, counter_alloca, "lc_i")
                    .unwrap().into_float_value();
                let is_done = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OGE, current, len_f64, "lc_done",
                ).unwrap();
                self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

                // Body: get element, bind var, optionally check condition, emit body, push
                self.builder.position_at_end(loop_body);
                ctx.cse_cache.clear();
                let idx_nb = self.wrap_number(current);
                let elem_nb = self.call_bridged(self.runtime.list_get, &[list_value, idx_nb], "lc_elem");
                self.store_variable(ctx, var, elem_nb);

                if let Some(cond) = condition {
                    let cond_val = self.emit_expression(ctx, cond)?;
                    let is_truthy = self.call_nb(self.runtime.nb_is_truthy, &[cond_val.into()], "lc_truthy");
                    let truthy_bool = self.builder.build_int_compare(
                        inkwell::IntPredicate::NE,
                        is_truthy,
                        self.i8_type.const_int(0, false),
                        "lc_cond_bool",
                    ).unwrap();
                    let lc_push = self.context.append_basic_block(function, "lc_push");
                    let lc_skip = self.context.append_basic_block(function, "lc_skip");
                    self.builder.build_conditional_branch(truthy_bool, lc_push, lc_skip).unwrap();

                    // Push block
                    self.builder.position_at_end(lc_push);
                    let body_val = self.emit_expression(ctx, body)?;
                    let cur_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "lc_cur_out")
                        .unwrap().into_int_value();
                    let new_out = self.call_bridged(self.runtime.list_push, &[cur_out, body_val], "lc_push_res");
                    self.builder.build_store(out_alloca, new_out).unwrap();
                    // Increment and loop back
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "lc_cur_upd")
                            .unwrap().into_float_value();
                        let next = self.builder.build_float_add(
                            cur_f64, self.f64_type.const_float(1.0), "lc_next",
                        ).unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }

                    // Skip block: just increment counter
                    self.builder.position_at_end(lc_skip);
                    let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "lc_skip_upd")
                        .unwrap().into_float_value();
                    let next = self.builder.build_float_add(
                        cur_f64, self.f64_type.const_float(1.0), "lc_skip_next",
                    ).unwrap();
                    self.builder.build_store(counter_alloca, next).unwrap();
                    self.builder.build_unconditional_branch(loop_header).unwrap();
                } else {
                    // No condition — unconditionally push
                    let body_val = self.emit_expression(ctx, body)?;
                    let cur_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "lc_cur_out")
                        .unwrap().into_int_value();
                    let new_out = self.call_bridged(self.runtime.list_push, &[cur_out, body_val], "lc_push_res");
                    self.builder.build_store(out_alloca, new_out).unwrap();
                    // Increment counter
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "lc_cur_upd")
                            .unwrap().into_float_value();
                        let next = self.builder.build_float_add(
                            cur_f64, self.f64_type.const_float(1.0), "lc_next",
                        ).unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }
                }

                // Return the output list
                self.builder.position_at_end(loop_exit);
                let final_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "lc_result")
                    .unwrap().into_int_value();
                Ok(final_out)
            }
            Expression::MapComprehension { key, value, var, iterable, condition, span: _ } => {
                let function = ctx.function;
                let list_value = self.emit_expression(ctx, iterable)?;

                // Get list length
                let len_nb = self.call_bridged(self.runtime.list_length, &[list_value], "mc_len");
                let len_f64 = self.value_to_number(len_nb);

                // Create empty output map
                let entry_ptr_type = self.runtime.map_entry_type
                    .ptr_type(inkwell::AddressSpace::default());
                let null_entries = entry_ptr_type.const_null();
                let zero_usize = self.usize_type.const_int(0, false);
                let out_map_ptr = self.call_runtime_ptr(
                    self.runtime.make_map,
                    &[null_entries.into(), zero_usize.into()],
                    "mc_out_map",
                );
                let out_map_nb = self.ptr_to_nb(out_map_ptr);

                // Allocas
                let out_alloca = self.builder.build_alloca(self.runtime.value_i64_type, "mc_out_alloca").unwrap();
                self.builder.build_store(out_alloca, out_map_nb).unwrap();
                let counter_alloca = self.builder.build_alloca(self.f64_type, "mc_counter").unwrap();
                self.builder.build_store(counter_alloca, self.f64_type.const_float(0.0)).unwrap();

                let loop_header = self.context.append_basic_block(function, "mc_cond");
                let loop_body = self.context.append_basic_block(function, "mc_body");
                let loop_exit = self.context.append_basic_block(function, "mc_exit");

                self.builder.build_unconditional_branch(loop_header).unwrap();

                // Header: counter < length
                self.builder.position_at_end(loop_header);
                let current = self.builder.build_load(self.f64_type, counter_alloca, "mc_i")
                    .unwrap().into_float_value();
                let is_done = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OGE, current, len_f64, "mc_done",
                ).unwrap();
                self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

                // Body
                self.builder.position_at_end(loop_body);
                ctx.cse_cache.clear();
                let idx_nb = self.wrap_number(current);
                let elem_nb = self.call_bridged(self.runtime.list_get, &[list_value, idx_nb], "mc_elem");
                self.store_variable(ctx, var, elem_nb);

                if let Some(cond) = condition {
                    let cond_val = self.emit_expression(ctx, cond)?;
                    let is_truthy = self.call_nb(self.runtime.nb_is_truthy, &[cond_val.into()], "mc_truthy");
                    let truthy_bool = self.builder.build_int_compare(
                        inkwell::IntPredicate::NE,
                        is_truthy,
                        self.i8_type.const_int(0, false),
                        "mc_cond_bool",
                    ).unwrap();
                    let mc_set = self.context.append_basic_block(function, "mc_set");
                    let mc_skip = self.context.append_basic_block(function, "mc_skip");
                    self.builder.build_conditional_branch(truthy_bool, mc_set, mc_skip).unwrap();

                    self.builder.position_at_end(mc_set);
                    let key_val = self.emit_expression(ctx, key)?;
                    let val_val = self.emit_expression(ctx, value)?;
                    let cur_map = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "mc_cur_map")
                        .unwrap().into_int_value();
                    let new_map = self.call_bridged(self.runtime.map_set, &[cur_map, key_val, val_val], "mc_set_res");
                    self.builder.build_store(out_alloca, new_map).unwrap();
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "mc_i_upd")
                            .unwrap().into_float_value();
                        let next = self.builder.build_float_add(cur_f64, self.f64_type.const_float(1.0), "mc_next").unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }

                    self.builder.position_at_end(mc_skip);
                    let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "mc_skip_upd")
                        .unwrap().into_float_value();
                    let next = self.builder.build_float_add(cur_f64, self.f64_type.const_float(1.0), "mc_skip_next").unwrap();
                    self.builder.build_store(counter_alloca, next).unwrap();
                    self.builder.build_unconditional_branch(loop_header).unwrap();
                } else {
                    let key_val = self.emit_expression(ctx, key)?;
                    let val_val = self.emit_expression(ctx, value)?;
                    let cur_map = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "mc_cur_map")
                        .unwrap().into_int_value();
                    let new_map = self.call_bridged(self.runtime.map_set, &[cur_map, key_val, val_val], "mc_set_res");
                    self.builder.build_store(out_alloca, new_map).unwrap();
                    if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                        let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "mc_i_upd")
                            .unwrap().into_float_value();
                        let next = self.builder.build_float_add(cur_f64, self.f64_type.const_float(1.0), "mc_next").unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder.build_unconditional_branch(loop_header).unwrap();
                    }
                }

                self.builder.position_at_end(loop_exit);
                let final_map = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "mc_result")
                    .unwrap().into_int_value();
                Ok(final_map)
            }
            Expression::ErrorPropagate { expr, span: _ } => {
                // Error propagation: `expr ! return err`
                // 1. Evaluate the expression
                // 2. Check if it's an error
                // 3. If error, return it from the current function
                // 4. Otherwise, continue with the value
                
                let value = self.emit_expression(ctx, expr)?;
                
                // Call coral_nb_is_err to check if value is an error (returns i8)
                let is_err = self.builder
                    .build_call(self.runtime.nb_is_err, &[value.into()], "is_err_check")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                
                let is_err_bool = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    is_err,
                    self.i8_type.const_zero(),
                    "is_err_bool",
                ).unwrap();
                
                // Create basic blocks for the branch
                let current_fn = ctx.function;
                let err_return_bb = self.context.append_basic_block(current_fn, "err_return");
                let continue_bb = self.context.append_basic_block(current_fn, "err_continue");
                
                self.builder.build_conditional_branch(is_err_bool, err_return_bb, continue_bb).unwrap();
                
                // Error return block: return the error value
                self.builder.position_at_end(err_return_bb);
                self.builder.build_return(Some(&value)).unwrap();
                
                // Continue block: value is not an error, use it
                self.builder.position_at_end(continue_bb);
                
                Ok(value)
            }
        }
    }

    fn emit_list_literal(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        elements: &[Expression],
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        self.emit_list_literal_hinted(ctx, elements, None)
    }

    fn emit_list_literal_hinted(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        elements: &[Expression],
        hint: Option<i8>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let has_spread = elements.iter().any(|e| matches!(e, Expression::Spread(..)));
        if has_spread {
            return self.emit_list_literal_with_spread(ctx, elements, hint);
        }
        let mut values = Vec::new();
        for element in elements {
            values.push(self.emit_expression(ctx, element)?);
        }
        let handles_ptr_type = self
            .runtime
            .value_ptr_type
            .ptr_type(AddressSpace::default());
        if values.is_empty() {
            let null_ptr = handles_ptr_type.const_null();
            let len_value = self.usize_type.const_zero();
            let args = &[null_ptr.into(), len_value.into()];
            return Ok(self.call_list_with_hint(args, hint));
        }
        // Convert NaN-boxed i64 values to pointers for old API
        let ptrs: Vec<PointerValue<'ctx>> = values.iter().map(|v| self.nb_to_ptr(*v)).collect();
        let element_type = self.runtime.value_ptr_type;
        let array_type = element_type.array_type(ptrs.len() as u32);
        let mut temp_array = array_type.get_undef();
        for (index, value) in ptrs.iter().enumerate() {
            temp_array = self
                .builder
                .build_insert_value(temp_array, *value, index as u32, "list_init")
                .unwrap()
                .into_array_value();
        }
        let alloca = self
            .builder
            .build_alloca(array_type, "list_literal")
            .unwrap();
        self.builder.build_store(alloca, temp_array).unwrap();
        let ptr = self
            .builder
            .build_pointer_cast(
                alloca,
                handles_ptr_type,
                "list_ptr",
            )
            .unwrap();
        let len_value = self.usize_type.const_int(ptrs.len() as u64, false);
        let args = &[ptr.into(), len_value.into()];
        let list_ptr = self.call_list_with_hint(args, hint);
        Ok(list_ptr)
    }

    /// Emit a list literal that contains at least one spread element.
    /// Uses incremental push/concat instead of a stack array.
    fn emit_list_literal_with_spread(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        elements: &[Expression],
        hint: Option<i8>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let handles_ptr_type = self
            .runtime
            .value_ptr_type
            .ptr_type(AddressSpace::default());
        // Start with an empty list
        let null_ptr = handles_ptr_type.const_null();
        let len_zero = self.usize_type.const_zero();
        let mut list = self.call_list_with_hint(&[null_ptr.into(), len_zero.into()], hint);
        for element in elements {
            if let Expression::Spread(inner, _) = element {
                // Emit the spread operand (should be a list) and concat
                let spread_val = self.emit_expression(ctx, inner)?;
                list = self.call_bridged(self.runtime.list_concat, &[list, spread_val], "spread_concat");
            } else {
                // Normal element: push onto the list
                let val = self.emit_expression(ctx, element)?;
                list = self.call_bridged(self.runtime.list_push, &[list, val], "spread_push");
            }
        }
        Ok(list)
    }

    fn emit_map_literal(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        entries: &[(Expression, Expression)],
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        self.emit_map_literal_hinted(ctx, entries, None)
    }

    fn emit_map_literal_hinted(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        entries: &[(Expression, Expression)],
        hint: Option<i8>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let mut evaluated = Vec::with_capacity(entries.len());
        for (key_expr, value_expr) in entries {
            let key = self.emit_expression(ctx, key_expr)?;
            let value = self.emit_expression(ctx, value_expr)?;
            evaluated.push((key, value));
        }
        let entry_ptr_type = self
            .runtime
            .map_entry_type
            .ptr_type(AddressSpace::default());
        if evaluated.is_empty() {
            let null_ptr = entry_ptr_type.const_null();
            let len_value = self.usize_type.const_zero();
            let args = &[null_ptr.into(), len_value.into()];
            return Ok(self.call_map_with_hint(args, hint));
        }
        let array_type = self
            .runtime
            .map_entry_type
            .array_type(evaluated.len() as u32);
        let mut temp_array = array_type.get_undef();
        for (index, (key, value)) in evaluated.iter().enumerate() {
            let key_ptr = self.nb_to_ptr(*key);
            let value_ptr = self.nb_to_ptr(*value);
            let mut entry_value = self.runtime.map_entry_type.get_undef();
            entry_value = self
                .builder
                .build_insert_value(entry_value, key_ptr, 0, "map_key")
                .unwrap()
                .into_struct_value();
            entry_value = self
                .builder
                .build_insert_value(entry_value, value_ptr, 1, "map_value")
                .unwrap()
                .into_struct_value();
            temp_array = self
                .builder
                .build_insert_value(temp_array, entry_value, index as u32, "map_entry")
                .unwrap()
                .into_array_value();
        }
        let alloca = self.builder.build_alloca(array_type, "map_literal").unwrap();
        self.builder.build_store(alloca, temp_array).unwrap();
        let ptr = self
            .builder
            .build_pointer_cast(alloca, entry_ptr_type, "map_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(evaluated.len() as u64, false);
        let args = &[ptr.into(), len_value.into()];
        Ok(self.call_map_with_hint(args, hint))
    }

    /// C2.1: Infer whether an expression is statically known to be numeric.
    /// Returns true for number literals and variables resolved to Float/Int.
    fn expr_is_numeric(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Float(_, _) => true,
            Expression::Identifier(name, _) => {
                matches!(
                    self.resolved_types.get(name.as_str()),
                    Some(TypeId::Primitive(Primitive::Float)) | Some(TypeId::Primitive(Primitive::Int))
                )
            }
            Expression::Unary { op: UnaryOp::Neg, .. } => true, // negation always yields a number
            _ => false,
        }
    }

    /// C2.2: Infer whether an expression is statically known to be boolean.
    fn expr_is_bool(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Bool(_, _) => true,
            Expression::Identifier(name, _) => {
                matches!(
                    self.resolved_types.get(name.as_str()),
                    Some(TypeId::Primitive(Primitive::Bool))
                )
            }
            _ => false,
        }
    }

    /// Build a Message map { name: <name_value>, data: <payload_value> } from already-evaluated values.
    fn emit_numeric_binary(
        &mut self,
        op: BinaryOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        both_numeric: bool,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        use BinaryOp::*;
        if matches!(op, Add) {
            if both_numeric {
                // C2.1: Specialize Add for known-numeric operands — bypass runtime polymorphic add.
                let lhs_num = self.value_to_number(lhs);
                let rhs_num = self.value_to_number(rhs);
                return Ok(self.wrap_number(self.builder.build_float_add(lhs_num, rhs_num, "add_spec").unwrap()));
            }
            return Ok(self.call_nb(self.runtime.nb_add, &[lhs.into(), rhs.into()], "nb_add"));
        }
        if matches!(op, BinaryOp::Equals) {
            if both_numeric {
                // C2.1: Specialize Equals for known-numeric operands.
                let lhs_num = self.value_to_number(lhs);
                let rhs_num = self.value_to_number(rhs);
                return Ok(self.wrap_bool(
                    self.builder.build_float_compare(FloatPredicate::OEQ, lhs_num, rhs_num, "eq_spec").unwrap(),
                ));
            }
            return Ok(self.call_nb(self.runtime.nb_equals, &[lhs.into(), rhs.into()], "nb_equals"));
        }
        if matches!(op, BinaryOp::NotEquals) {
            if both_numeric {
                // C2.1: Specialize NotEquals for known-numeric operands.
                let lhs_num = self.value_to_number(lhs);
                let rhs_num = self.value_to_number(rhs);
                return Ok(self.wrap_bool(
                    self.builder.build_float_compare(FloatPredicate::ONE, lhs_num, rhs_num, "ne_spec").unwrap(),
                ));
            }
            return Ok(self.call_nb(self.runtime.nb_not_equals, &[lhs.into(), rhs.into()], "nb_not_equals"));
        }

        if matches!(op, BinaryOp::BitAnd) {
            return Ok(self.call_bridged(self.runtime.value_bitand, &[lhs, rhs], "bitand"));
        }
        if matches!(op, BinaryOp::BitOr) {
            return Ok(self.call_bridged(self.runtime.value_bitor, &[lhs, rhs], "bitor"));
        }
        if matches!(op, BinaryOp::BitXor) {
            return Ok(self.call_bridged(self.runtime.value_bitxor, &[lhs, rhs], "bitxor"));
        }
        if matches!(op, BinaryOp::ShiftLeft) {
            return Ok(self.call_bridged(self.runtime.value_shift_left, &[lhs, rhs], "shift_left"));
        }
        if matches!(op, BinaryOp::ShiftRight) {
            return Ok(self.call_bridged(self.runtime.value_shift_right, &[lhs, rhs], "shift_right"));
        }
        let lhs_num = self.value_to_number(lhs);
        let rhs_num = self.value_to_number(rhs);
        Ok(match op {
            Add => unreachable!(),
            Sub => self.wrap_number(self.builder.build_float_sub(lhs_num, rhs_num, "sub").unwrap()),
            Mul => self.wrap_number(self.builder.build_float_mul(lhs_num, rhs_num, "mul").unwrap()),
            Div => self.wrap_number(self.builder.build_float_div(lhs_num, rhs_num, "div").unwrap()),
            Mod => self.wrap_number(self.builder.build_float_rem(lhs_num, rhs_num, "rem").unwrap()),
            BitAnd | BitOr | BitXor | ShiftLeft | ShiftRight => unreachable!(),
            Greater => self.wrap_bool(
                self.builder
                    .build_float_compare(FloatPredicate::OGT, lhs_num, rhs_num, "gt")
                    .unwrap(),
            ),
            GreaterEq => self.wrap_bool(
                self.builder
                    .build_float_compare(FloatPredicate::OGE, lhs_num, rhs_num, "ge")
                    .unwrap(),
            ),
            Less => self.wrap_bool(
                self.builder
                    .build_float_compare(FloatPredicate::OLT, lhs_num, rhs_num, "lt")
                    .unwrap(),
            ),
            LessEq => self.wrap_bool(
                self.builder
                    .build_float_compare(FloatPredicate::OLE, lhs_num, rhs_num, "le")
                    .unwrap(),
            ),
            Equals | NotEquals | And | Or => unreachable!(),
        })
    }

    fn emit_logical_binary(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        // C3.3: Operands of logical ops are not in tail position
        let saved_tail = ctx.in_tail_position;
        ctx.in_tail_position = false;
        // C2.2: Use fast bool extraction when operand is known-boolean.
        let left_is_bool = self.expr_is_bool(left);
        let right_is_bool = self.expr_is_bool(right);
        let left_value = self.emit_expression(ctx, left)?;
        let left_bool = if left_is_bool {
            self.value_to_bool_fast(left_value)
        } else {
            self.value_to_bool(left_value)
        };
        let function = ctx.function;
        let rhs_bb = self.context.append_basic_block(
            function,
            match op {
                BinaryOp::And => "and_rhs",
                BinaryOp::Or => "or_rhs",
                _ => "logic_rhs",
            },
        );
        let short_bb = self.context.append_basic_block(
            function,
            match op {
                BinaryOp::And => "and_short",
                BinaryOp::Or => "or_short",
                _ => "logic_short",
            },
        );
        let cont_bb = self.context.append_basic_block(function, "logic_cont");

        match op {
            BinaryOp::And => {
                self.builder
                    .build_conditional_branch(left_bool, rhs_bb, short_bb)
                    .unwrap();
            }
            BinaryOp::Or => {
                self.builder
                    .build_conditional_branch(left_bool, short_bb, rhs_bb)
                    .unwrap();
            }
            _ => unreachable!(),
        }

        self.builder.position_at_end(short_bb);
        let short_value = match op {
            BinaryOp::And => self.boolean_to_int(false),
            BinaryOp::Or => self.boolean_to_int(true),
            _ => unreachable!(),
        };
        self.builder
            .build_unconditional_branch(cont_bb)
            .unwrap();

        self.builder.position_at_end(rhs_bb);
        let right_value = self.emit_expression(ctx, right)?;
        let right_bool = if right_is_bool {
            self.value_to_bool_fast(right_value)
        } else {
            self.value_to_bool(right_value)
        };
        // Capture the actual current block — nested and/or may have created sub-blocks
        let rhs_end_bb = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(cont_bb)
            .unwrap();

        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(self.bool_type, "logic_phi")
            .unwrap();
        phi.add_incoming(&[
            (&short_value as &dyn BasicValue<'ctx>, short_bb),
            (&right_bool as &dyn BasicValue<'ctx>, rhs_end_bb),
        ]);
        let bool_value = phi.as_basic_value().into_int_value();
        ctx.in_tail_position = saved_tail;
        Ok(self.wrap_bool(bool_value))
    }

    fn emit_ternary(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        condition: &Expression,
        then_branch: &Expression,
        else_branch: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let cond_is_bool = self.expr_is_bool(condition);
        let cond_value = self.emit_expression(ctx, condition)?;
        let cond_bool = if cond_is_bool {
            self.value_to_bool_fast(cond_value)
        } else {
            self.value_to_bool(cond_value)
        };
        let function = ctx.function;
        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let cont_bb = self.context.append_basic_block(function, "cont");

        self.builder
            .build_conditional_branch(cond_bool, then_bb, else_bb)
            .unwrap();

        self.builder.position_at_end(then_bb);
        ctx.cse_cache.clear(); // C3.4
        let then_value = self.emit_expression(ctx, then_branch)?;
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        let then_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(else_bb);
        ctx.cse_cache.clear(); // C3.4
        let else_value = self.emit_expression(ctx, else_branch)?;
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        let else_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(self.runtime.value_i64_type, "ternary_phi")
            .unwrap();
        let incoming = [
            (&then_value as &dyn BasicValue<'ctx>, then_end),
            (&else_value as &dyn BasicValue<'ctx>, else_end),
        ];
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value().into_int_value())
    }

    fn load_variable(
        &mut self,
        ctx: &FunctionContext<'ctx>,
        name: &str,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        // Check alloca-based variables first (these can be mutated in loops)
        if let Some(alloca) = ctx.variable_allocas.get(name) {
            let loaded = self
                .builder
                .build_load(
                    self.runtime.value_i64_type,
                    *alloca,
                    &format!("load_{name}"),
                )
                .unwrap()
                .into_int_value();
            return Ok(loaded);
        }
        if let Some(val) = ctx.variables.get(name) {
            return Ok(*val);
        }
        if let Some(global) = self.global_variables.get(name) {
            let loaded = self
                .builder
                .build_load(
                    self.runtime.value_i64_type,
                    global.as_pointer_value(),
                    &format!("load_global_{name}"),
                )
                .unwrap()
                .into_int_value();
            self.call_nb_void(self.runtime.nb_retain, &[loaded.into()]);
            return Ok(loaded);
        }
        // Check if this is a nullary enum constructor (e.g., None)
        if let Some((_, field_count)) = self.enum_constructors.get(name).cloned() {
            if field_count == 0 {
                // Emit a nullary constructor call
                return self.emit_enum_constructor_nullary(name);
            }
        }
        // Check if this is a named function used as a value (function reference).
        // Wrap it in a closure so it can be passed around and invoked via coral_closure_invoke.
        if let Some(target_fn) = self.functions.get(name).copied() {
            return self.emit_function_as_closure(ctx, name, target_fn);
        }
        Err(Diagnostic::new(
            format!("unknown variable `{name}`"),
            Span::new(0, 0),
        ))
    }

    fn store_variable(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        name: &str,
        value: IntValue<'ctx>,
    ) {
        // If there's already an alloca for this variable, store to it (mutation/rebinding)
        if let Some(alloca) = ctx.variable_allocas.get(name) {
            self.builder.build_store(*alloca, value).unwrap();
            return;
        }
        // Create an alloca for the variable in the function's entry block.
        // This ensures proper SSA behavior for variables that may be rebound in loops.
        let entry_bb = ctx.function.get_first_basic_block().unwrap();
        let current_bb = self.builder.get_insert_block().unwrap();
        
        // Position at the start of the entry block for the alloca
        if let Some(first_instr) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_bb);
        }
        let alloca = self
            .builder
            .build_alloca(self.runtime.value_i64_type, &format!("{name}_slot"))
            .unwrap();
        
        // Restore position and store the value
        self.builder.position_at_end(current_bb);
        self.builder.build_store(alloca, value).unwrap();
        ctx.variable_allocas.insert(name.to_string(), alloca);
    }

    fn wrap_number(&mut self, value: FloatValue<'ctx>) -> IntValue<'ctx> {
        self.call_nb(self.runtime.nb_make_number, &[value.into()], "nb_num")
    }

    fn wrap_bool(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        let bool_byte = self
            .builder
            .build_int_z_extend(value, self.i8_type, "bool_byte")
            .unwrap();
        self.call_nb(self.runtime.nb_make_bool, &[bool_byte.into()], "nb_bool")
    }

    fn emit_bytes_literal(&mut self, literal: &[u8]) -> IntValue<'ctx> {
        if let Some(global) = self.bytes_pool.get(literal) {
            return self.build_bytes_from_global(*global, literal.len());
        }
        let array_len = literal.len().max(1) as u32;
        let ty = self.i8_type.array_type(array_len);
        let name = format!("bytes_{}", self.bytes_pool.len());
        let global = self.module.add_global(ty, None, &name);
        let values: Vec<_> = if literal.is_empty() {
            vec![self.i8_type.const_zero()]
        } else {
            literal
                .iter()
                .map(|byte| self.i8_type.const_int(*byte as u64, false))
                .collect()
        };
        let const_array = self.i8_type.const_array(&values);
        global.set_initializer(&const_array);
        global.set_constant(true);
        self.bytes_pool.insert(literal.to_vec(), global);
        self.build_bytes_from_global(global, literal.len())
    }

    fn build_bytes_from_global(
        &mut self,
        global: GlobalValue<'ctx>,
        len: usize,
    ) -> IntValue<'ctx> {
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        let data_ptr = self
            .builder
            .build_pointer_cast(global.as_pointer_value(), i8_ptr_type, "bytes_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(len as u64, false);
        // bytes still heap-allocate; bridge through old API
        let ptr = self.call_runtime_ptr(
            self.runtime.make_bytes,
            &[data_ptr.into(), len_value.into()],
            "make_bytes",
        );
        self.ptr_to_nb(ptr)
    }

    fn wrap_unit(&mut self) -> IntValue<'ctx> {
        self.call_nb(self.runtime.nb_make_unit, &[], "nb_unit")
    }

    /// Wrap an absent/none value
    fn wrap_none(&mut self) -> IntValue<'ctx> {
        self.call_nb(self.runtime.nb_make_none, &[], "nb_none")
    }

    /// Get or create a raw string constant (global) for use in runtime calls.
    /// Returns the GlobalValue which can be cast to i8* for runtime functions.
    fn get_or_create_string_constant(&mut self, literal: &str) -> GlobalValue<'ctx> {
        if let Some(global) = self.string_pool.get(literal) {
            *global
        } else {
            let name = format!("str_{}", self.string_pool.len());
            let gv = self
                .builder
                .build_global_string_ptr(literal, &name)
                .unwrap();
            self.string_pool.insert(literal.to_string(), gv);
            gv
        }
    }

    fn emit_string_literal(&mut self, literal: &str) -> IntValue<'ctx> {
        let global = self.get_or_create_string_constant(literal);
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        let cast_ptr = self
            .builder
            .build_pointer_cast(global.as_pointer_value(), i8_ptr_type, "str_ptr")
            .unwrap();
        let len_value = self
            .usize_type
            .const_int(literal.len() as u64, false);
        let args = &[cast_ptr.into(), len_value.into()];
        self.call_nb(self.runtime.nb_make_string, args, "nb_str")
    }

    /// Call a coral_nb_* function that returns i64 (NaN-boxed value).
    fn call_nb(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) -> IntValue<'ctx> {
        self
            .builder
            .build_call(function, args, name)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value()
    }

    /// Call a coral_nb_* void function (e.g., retain, release, print).
    fn call_nb_void(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) {
        self.builder.build_call(function, args, "").unwrap();
    }

    /// Convert a NaN-boxed i64 to a legacy pointer via coral_nb_to_handle.
    /// Used when calling old-API functions that still take %CoralValue*.
    fn nb_to_ptr(&mut self, value: IntValue<'ctx>) -> PointerValue<'ctx> {
        self.call_runtime_ptr(
            self.runtime.nb_to_handle,
            &[value.into()],
            "nb_to_ptr",
        )
    }

    /// Convert a legacy pointer to a NaN-boxed i64 via coral_nb_from_handle.
    /// Used when receiving results from old-API functions.
    fn ptr_to_nb(&mut self, ptr: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.call_nb(
            self.runtime.nb_from_handle,
            &[ptr.into()],
            "ptr_to_nb",
        )
    }

    /// Call a legacy pointer-based runtime function, bridging NaN-boxed i64 args.
    /// Converts all IntValue args to pointers, calls the function, converts result back.
    fn call_bridged(
        &mut self,
        function: FunctionValue<'ctx>,
        nb_args: &[IntValue<'ctx>],
        name: &str,
    ) -> IntValue<'ctx> {
        let ptr_args: Vec<BasicMetadataValueEnum<'ctx>> = nb_args
            .iter()
            .map(|v| self.nb_to_ptr(*v).into())
            .collect();
        let ptr_result = self.call_runtime_ptr(function, &ptr_args, name);
        self.ptr_to_nb(ptr_result)
    }

    /// Call a legacy function with mixed args (some NaN-boxed, some raw).
    /// The caller must construct the args with proper bridging.
    fn call_runtime_ptr(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) -> PointerValue<'ctx> {
        self
            .builder
            .build_call(function, args, name)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value()
    }

    fn alloc_hint_byte(&self, hint: AllocationStrategy) -> i8 {
        match hint {
            AllocationStrategy::Stack => 1,
            AllocationStrategy::Arena => 2,
            AllocationStrategy::Heap => 3,
            AllocationStrategy::SharedCow => 4,
            AllocationStrategy::Unknown => 0,
        }
    }

    fn call_list_with_hint(
        &mut self,
        args: &[BasicMetadataValueEnum<'ctx>],
        hint: Option<i8>,
    ) -> IntValue<'ctx> {
        let ptr = if let Some(h) = hint {
            let hint_val = self.i8_type.const_int(h as u64, false);
            let mut extended = Vec::with_capacity(args.len() + 1);
            extended.extend_from_slice(args);
            extended.push(hint_val.into());
            self.call_runtime_ptr(self.runtime.make_list_hinted, &extended, "make_list_hinted")
        } else {
            self.call_runtime_ptr(self.runtime.make_list, args, "make_list")
        };
        self.ptr_to_nb(ptr)
    }

    fn call_map_with_hint(
        &mut self,
        args: &[BasicMetadataValueEnum<'ctx>],
        hint: Option<i8>,
    ) -> IntValue<'ctx> {
        let ptr = if let Some(h) = hint {
            let hint_val = self.i8_type.const_int(h as u64, false);
            let mut extended = Vec::with_capacity(args.len() + 1);
            extended.extend_from_slice(args);
            extended.push(hint_val.into());
            self.call_runtime_ptr(self.runtime.make_map_hinted, &extended, "make_map_hinted")
        } else {
            self.call_runtime_ptr(self.runtime.make_map, args, "make_map")
        };
        self.ptr_to_nb(ptr)
    }

    fn value_to_number(&mut self, value: IntValue<'ctx>) -> FloatValue<'ctx> {
        self
            .builder
            .build_call(self.runtime.nb_as_number, &[value.into()], "nb_as_number")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_float_value()
    }

    fn emit_inline_asm(
        &mut self,
        template: &str,
        constraints: &str,
        args: &[BasicMetadataValueEnum<'ctx>],
        span: Span,
    ) -> Result<(), Diagnostic> {
        let fn_type = self
            .context
            .void_type()
            .fn_type(&vec![self.f64_type.into(); args.len()], false);
        let asm = self.context.create_inline_asm(
            fn_type,
            template.to_string(),
            constraints.to_string(),
            true,  // side effects
            false, // align stack
            Some(InlineAsmDialect::ATT),
            false, // can_throw
        );
        let _ = self
            .builder
            .build_indirect_call(fn_type, asm, args, "inline_asm")
            .map_err(|e| Diagnostic::new(format!("inline asm emission failed: {e}"), span))?;
        Ok(())
    }

    fn cast_extern_arg(
        &mut self,
        value: IntValue<'ctx>,
        target: BasicTypeEnum<'ctx>,
        span: Span,
    ) -> Result<BasicMetadataValueEnum<'ctx>, Diagnostic> {
        match target {
            BasicTypeEnum::FloatType(ft) => {
                let num = self.value_to_number(value);
                let cast = if ft == self.f64_type {
                    num
                } else {
                    self.builder
                        .build_float_cast(num, ft, "extern_arg_float")
                        .map_err(|e| Diagnostic::new(format!("float cast failed: {e}"), span))?
                };
                Ok(cast.into())
            }
            BasicTypeEnum::IntType(it) => {
                let num = self.value_to_number(value);
                let int = self
                    .builder
                    .build_float_to_unsigned_int(num, it, "extern_arg_int")
                    .map_err(|e| Diagnostic::new(format!("float->int cast failed: {e}"), span))?;
                Ok(int.into())
            }
            _ => Err(Diagnostic::new(
                format!("extern argument type not supported: `{}`", self.format_type_enum(target)),
                span,
            )),
        }
    }

    fn wrap_extern_return(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ret_ty: BasicTypeEnum<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match ret_ty {
            BasicTypeEnum::FloatType(ft) => {
                let fv = value.into_float_value();
                let cast = if ft == self.f64_type {
                    fv
                } else {
                    self.builder
                        .build_float_cast(fv, self.f64_type, "extern_ret_float")
                        .map_err(|e| Diagnostic::new(format!("float cast failed: {e}"), span))?
                };
                Ok(self.wrap_number(cast))
            }
            BasicTypeEnum::IntType(it) => {
                let iv = value.into_int_value();
                if it.get_bit_width() == 1 {
                    Ok(self.wrap_bool(iv))
                } else {
                    let as_float = self
                        .builder
                        .build_unsigned_int_to_float(iv, self.f64_type, "extern_ret_int")
                        .map_err(|e| Diagnostic::new(format!("int->float cast failed: {e}"), span))?;
                    Ok(self.wrap_number(as_float))
                }
            }
            _ => Err(Diagnostic::new(
                format!("extern return type not supported: `{}`", self.format_type_enum(ret_ty)),
                span,
            )),
        }
    }

    /// Check if an expression contains a $ placeholder (for pipeline desugaring)
    fn contains_placeholder(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Placeholder(_, _) => true,
            Expression::Binary { left, right, .. } => {
                self.contains_placeholder(left) || self.contains_placeholder(right)
            }
            Expression::Unary { expr, .. } => self.contains_placeholder(expr),
            Expression::Call { callee, args, .. } => {
                self.contains_placeholder(callee) || args.iter().any(|a| self.contains_placeholder(a))
            }
            Expression::List(items, _) => items.iter().any(|i| self.contains_placeholder(i)),
            Expression::Map(entries, _) => entries.iter().any(|(k, v)| {
                self.contains_placeholder(k) || self.contains_placeholder(v)
            }),
            Expression::Member { target, .. } => self.contains_placeholder(target),
            Expression::Index { target, index, .. } => {
                self.contains_placeholder(target) || self.contains_placeholder(index)
            }
            Expression::Slice { target, start, end, .. } => {
                self.contains_placeholder(target) || self.contains_placeholder(start) || self.contains_placeholder(end)
            }
            Expression::Ternary { condition, then_branch, else_branch, .. } => {
                self.contains_placeholder(condition)
                    || self.contains_placeholder(then_branch)
                    || self.contains_placeholder(else_branch)
            }
            Expression::Lambda { body, .. } => {
                body.statements.iter().any(|s| match s {
                    Statement::Binding(b) => self.contains_placeholder(&b.value),
                    Statement::Expression(e) => self.contains_placeholder(e),
                    Statement::Return(e, _) => self.contains_placeholder(e),
                    Statement::If { condition, body, elif_branches, else_body, .. } => {
                        self.contains_placeholder(condition)
                            || body.statements.iter().any(|s2| match s2 {
                                Statement::Expression(e) => self.contains_placeholder(e),
                                _ => false,
                            })
                            || elif_branches.iter().any(|(cond, blk)| {
                                self.contains_placeholder(cond)
                                    || blk.statements.iter().any(|s2| match s2 {
                                        Statement::Expression(e) => self.contains_placeholder(e),
                                        _ => false,
                                    })
                            })
                            || else_body.as_ref().map_or(false, |blk| {
                                blk.statements.iter().any(|s2| match s2 {
                                    Statement::Expression(e) => self.contains_placeholder(e),
                                    _ => false,
                                })
                            })
                    }
                    Statement::While { condition, body, .. } => {
                        self.contains_placeholder(condition)
                            || body.statements.iter().any(|s2| match s2 {
                                Statement::Expression(e) => self.contains_placeholder(e),
                                _ => false,
                            })
                    }
                    Statement::For { iterable, body, .. } => {
                        self.contains_placeholder(iterable)
                            || body.statements.iter().any(|s2| match s2 {
                                Statement::Expression(e) => self.contains_placeholder(e),
                                _ => false,
                            })
                    }
                    Statement::ForKV { iterable, body, .. } => {
                        self.contains_placeholder(iterable)
                            || body.statements.iter().any(|s2| match s2 {
                                Statement::Expression(e) => self.contains_placeholder(e),
                                _ => false,
                            })
                    }
                    Statement::ForRange { start, end, step, body, .. } => {
                        self.contains_placeholder(start)
                            || self.contains_placeholder(end)
                            || step.as_ref().map_or(false, |s| self.contains_placeholder(s))
                            || body.statements.iter().any(|s2| match s2 {
                                Statement::Expression(e) => self.contains_placeholder(e),
                                _ => false,
                            })
                    }
                    Statement::Break(_) | Statement::Continue(_) => false,
                    Statement::FieldAssign { value, .. } => self.contains_placeholder(value),
                    Statement::PatternBinding { value, .. } => self.contains_placeholder(value),
                }) || body.value.as_ref().map_or(false, |v| self.contains_placeholder(v))
            }
            _ => false,
        }
    }

    /// Replace $ placeholders in an expression with a replacement expression
    fn replace_placeholder_with(&self, expr: &Expression, replacement: &Expression) -> Expression {
        match expr {
            Expression::Placeholder(_, _) => replacement.clone(),
            Expression::Binary { op, left, right, span } => Expression::Binary {
                op: *op,
                left: Box::new(self.replace_placeholder_with(left, replacement)),
                right: Box::new(self.replace_placeholder_with(right, replacement)),
                span: *span,
            },
            Expression::Unary { op, expr: inner, span } => Expression::Unary {
                op: *op,
                expr: Box::new(self.replace_placeholder_with(inner, replacement)),
                span: *span,
            },
            Expression::Call { callee, args, span } => Expression::Call {
                callee: Box::new(self.replace_placeholder_with(callee, replacement)),
                args: args.iter().map(|a| self.replace_placeholder_with(a, replacement)).collect(),
                span: *span,
            },
            Expression::List(items, span) => Expression::List(
                items.iter().map(|i| self.replace_placeholder_with(i, replacement)).collect(),
                *span,
            ),
            Expression::Map(entries, span) => Expression::Map(
                entries.iter().map(|(k, v)| {
                    (self.replace_placeholder_with(k, replacement), self.replace_placeholder_with(v, replacement))
                }).collect(),
                *span,
            ),
            Expression::Member { target, property, span } => Expression::Member {
                target: Box::new(self.replace_placeholder_with(target, replacement)),
                property: property.clone(),
                span: *span,
            },
            Expression::Index { target, index, span } => Expression::Index {
                target: Box::new(self.replace_placeholder_with(target, replacement)),
                index: Box::new(self.replace_placeholder_with(index, replacement)),
                span: *span,
            },
            Expression::Slice { target, start, end, span } => Expression::Slice {
                target: Box::new(self.replace_placeholder_with(target, replacement)),
                start: Box::new(self.replace_placeholder_with(start, replacement)),
                end: Box::new(self.replace_placeholder_with(end, replacement)),
                span: *span,
            },
            Expression::Ternary { condition, then_branch, else_branch, span } => Expression::Ternary {
                condition: Box::new(self.replace_placeholder_with(condition, replacement)),
                then_branch: Box::new(self.replace_placeholder_with(then_branch, replacement)),
                else_branch: Box::new(self.replace_placeholder_with(else_branch, replacement)),
                span: *span,
            },
            // For expressions that don't contain placeholders or shouldn't be traversed, return as-is
            other => other.clone(),
        }
    }

    fn value_to_bool(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        let byte = self
            .builder
            .build_call(self.runtime.nb_is_truthy, &[value.into()], "nb_is_truthy")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();
        self.builder
            .build_int_truncate(byte, self.bool_type, "bool_from_byte")
            .unwrap()
    }

    /// C2.2: Fast boolean extraction when value is statically known to be a NaN-boxed Bool.
    /// Extracts bit 0 of the NaN-boxed representation directly, avoiding the runtime `is_truthy` call.
    fn value_to_bool_fast(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        let one = self.runtime.value_i64_type.const_int(1, false);
        let masked = self.builder.build_and(value, one, "bool_extract").unwrap();
        self.builder
            .build_int_truncate(masked, self.bool_type, "bool_fast")
            .unwrap()
    }

    fn call_runtime_void(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) {
        self.builder.build_call(function, args, name).unwrap();
    }

    fn ensure_globals_initialized(&mut self) {
        if let Some(init_fn) = self.global_init_fn {
            self.builder.build_call(init_fn, &[], "init_globals").unwrap();
        }
    }

    fn declare_global_bindings(&mut self, globals: &[Binding]) {
        if globals.is_empty() {
            return;
        }
        for binding in globals {
            let global = self.module.add_global(
                self.runtime.value_i64_type,
                None,
                &format!("coral_global_{}", binding.name),
            );
            global.set_initializer(&self.runtime.value_i64_type.const_zero());
            self.global_variables.insert(binding.name.clone(), global);
        }
        let flag = self
            .module
            .add_global(self.bool_type, None, "__coral_globals_initialized");
        flag.set_initializer(&self.bool_type.const_zero());
        self.globals_initialized_flag = Some(flag);
    }

    fn build_global_initializer(&mut self, globals: &[Binding]) -> Result<(), Diagnostic> {
        if globals.is_empty() {
            return Ok(());
        }
        let init_fn = self
            .module
            .add_function("__coral_init_globals", self.context.void_type().fn_type(&[], false), None);
        self.global_init_fn = Some(init_fn);
        let entry = self.context.append_basic_block(init_fn, "entry");
        let body = self.context.append_basic_block(init_fn, "body");
        let done = self.context.append_basic_block(init_fn, "done");

        self.builder.position_at_end(entry);
        if let Some(flag) = self.globals_initialized_flag {
            let current = self
                .builder
                .build_load(self.bool_type, flag.as_pointer_value(), "globals_flag")
                .unwrap()
                .into_int_value();
            let cmp = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    current,
                    self.bool_type.const_int(1, false),
                    "globals_ready",
                )
                .unwrap();
            self.builder
                .build_conditional_branch(cmp, done, body)
                .unwrap();
        } else {
            self.builder.build_unconditional_branch(body).unwrap();
        }

        self.builder.position_at_end(body);
        if let Some(flag) = self.globals_initialized_flag {
            self.builder
                .build_store(flag.as_pointer_value(), self.bool_type.const_int(1, false))
                .unwrap();
        }

        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: init_fn,
            loop_stack: Vec::new(),
            di_scope: None,
            fn_name: String::from("__coral_init_globals"),
            in_tail_position: false,
            cse_cache: HashMap::new(),
        };

        for binding in globals {
            let value = self.emit_expression(&mut ctx, &binding.value)?;
            if let Some(global) = self.global_variables.get(&binding.name) {
                self.builder
                    .build_store(global.as_pointer_value(), value)
                    .unwrap();
            }
        }

        self.builder.build_unconditional_branch(done).unwrap();
        self.builder.position_at_end(done);
        self.builder.build_return(None).unwrap();
        Ok(())
    }

    fn boolean_to_int(&self, value: bool) -> IntValue<'ctx> {
        self.bool_type.const_int(if value { 1 } else { 0 }, false)
    }
}

struct FunctionContext<'ctx> {
    variables: HashMap<String, IntValue<'ctx>>,
    /// Stack-allocated slots for variables (alloca i64) that support mutation/rebinding in loops.
    variable_allocas: HashMap<String, PointerValue<'ctx>>,
    function: FunctionValue<'ctx>,
    /// Stack of (loop_header_bb, loop_exit_bb) for break/continue support
    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
    /// CC2.3: Debug info scope for this function (if debug info is enabled).
    di_scope: Option<DIScope<'ctx>>,
    /// C3.3: The Coral-level function name (used for tail call detection).
    fn_name: String,
    /// C3.3: When true, the expression being emitted is in tail position.
    /// Self-recursive calls in this position are marked as tail calls.
    in_tail_position: bool,
    /// C3.4: Common subexpression elimination cache.
    /// Maps a normalized expression key to the previously emitted LLVM value.
    /// Cleared on mutation (variable assignment, store field write, etc.).
    cse_cache: HashMap<String, IntValue<'ctx>>,
}
