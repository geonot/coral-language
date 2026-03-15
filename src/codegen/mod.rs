mod builtins;
mod closures;
mod match_adt;
mod runtime;
mod store_actor;

use runtime::RuntimeBindings;

use crate::ast::{
    BinaryOp, Binding, Block, Expression, Function, FunctionKind, MatchExpression, MatchPattern,
    Parameter, Statement, TypeAnnotation, UnaryOp,
};
use crate::diagnostics::Diagnostic;
use crate::semantic::{EscapeInfo, MonomorphInfo, MonomorphVariant, SemanticModel};
use crate::span::{LineIndex, Span};
use crate::types::{AllocationStrategy, Primitive, TypeId};
use inkwell::InlineAsmDialect;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DIScope, DISubroutineType,
    DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::Module;
use inkwell::types::{
    BasicMetadataTypeEnum, BasicTypeEnum, FloatType, FunctionType, IntType, StructType,
};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, FunctionValue, GlobalValue,
    IntValue, PointerValue,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};
use std::collections::{HashMap, HashSet};

/// Tracks the native representation of an unboxed variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnboxedKind {
    /// Native i64 integer — no NaN-boxing
    NativeInt,
    /// Native f64 (stored as raw f64 bits in alloca double) — no NaN-boxing tags
    NativeFloat,
    /// Native i1 boolean
    NativeBool,
    /// Standard NaN-boxed i64 (default)
    Boxed,
}

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
    /// Cache of NaN-boxed string literal values (global i64 variables computed once at init)
    string_nb_cache: HashMap<String, GlobalValue<'ctx>>,
    bytes_pool: HashMap<Vec<u8>, GlobalValue<'ctx>>,
    global_variables: HashMap<String, GlobalValue<'ctx>>,
    globals_initialized_flag: Option<GlobalValue<'ctx>>,
    global_init_fn: Option<FunctionValue<'ctx>>,
    lambda_counter: usize,
    allocation_hints: HashMap<String, AllocationStrategy>,
    extern_sigs: HashMap<String, ExternSignature<'ctx>>,
    inline_asm_mode: InlineAsmMode,

    store_methods: HashMap<String, (String, usize)>,

    reference_fields: HashSet<(String, String)>,

    enum_constructors: HashMap<String, (String, usize)>,

    store_constructors: HashSet<String>,

    store_field_names: HashSet<String>,

    /// Maps (store_name, field_name) -> field_index for indexed struct access
    store_field_indices: HashMap<(String, String), usize>,
    /// Maps store_name -> field_count for struct allocation
    store_field_counts: HashMap<String, usize>,

    persistent_stores: HashSet<String>,

    has_persistent_stores: bool,

    uses_actors: bool,

    /// When compiling a store method, the name of the current store
    current_store_name: Option<String>,

    /// Name of the current function being compiled (for resolved_locals lookups)
    current_fn_name: Option<String>,

    debug_ctx: Option<DebugContext<'ctx>>,

    resolved_types: HashMap<String, TypeId>,

    /// Per-function resolved types: (fn_name, var_name) → TypeId
    resolved_locals: HashMap<(String, String), TypeId>,

    /// Per-function resolved param types: (fn_name, param_index) → TypeId
    resolved_params: HashMap<(String, usize), TypeId>,

    /// Resolved return types: fn_name → TypeId
    resolved_returns: HashMap<String, TypeId>,

    fn_param_defaults: HashMap<String, Vec<Parameter>>,

    module_exports: HashMap<String, Vec<String>>,

    /// Escape analysis info from semantic pass
    escape_info: EscapeInfo,

    /// Monomorphization candidates from semantic pass
    monomorph_info: MonomorphInfo,

    /// Maps (fn_name, var_name) → element TypeId for homogeneous-type lists
    typed_lists: HashMap<(String, String), TypeId>,

    /// Maps (fn_name, param_types) → specialized LLVM function
    specialized_functions: HashMap<(String, Vec<TypeId>), FunctionValue<'ctx>>,
}

struct DebugContext<'ctx> {
    builder: DebugInfoBuilder<'ctx>,
    compile_unit: DICompileUnit<'ctx>,
    file: DIFile<'ctx>,
    line_index: LineIndex,

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
            string_nb_cache: HashMap::new(),
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
            store_field_indices: HashMap::new(),
            store_field_counts: HashMap::new(),
            persistent_stores: HashSet::new(),
            has_persistent_stores: false,
            uses_actors: false,
            current_store_name: None,
            current_fn_name: None,
            debug_ctx: None,
            resolved_types: HashMap::new(),
            resolved_locals: HashMap::new(),
            resolved_params: HashMap::new(),
            resolved_returns: HashMap::new(),
            fn_param_defaults: HashMap::new(),
            module_exports: HashMap::new(),
            escape_info: EscapeInfo::default(),
            monomorph_info: MonomorphInfo::default(),
            typed_lists: HashMap::new(),
            specialized_functions: HashMap::new(),
        }
    }

    pub fn with_inline_asm_mode(mut self, mode: InlineAsmMode) -> Self {
        self.inline_asm_mode = mode;
        self
    }

    pub fn with_debug_info(mut self, filename: &str, source: &str) -> Self {
        let debug_metadata_version = self.context.i32_type().const_int(3, false);
        self.module.add_basic_value_flag(
            "Debug Info Version",
            inkwell::module::FlagBehavior::Warning,
            debug_metadata_version,
        );
        let (dibuilder, compile_unit) = self.module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::C,
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
            "",
            "",
        );
        let file = compile_unit.get_file();
        let line_index = LineIndex::new(source);

        let fn_di_type = dibuilder.create_subroutine_type(file, None, &[], DIFlags::PUBLIC);
        self.debug_ctx = Some(DebugContext {
            builder: dibuilder,
            compile_unit,
            file,
            line_index,
            fn_di_type,
        });
        self
    }

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
        let triple = inkwell::targets::TargetMachine::get_default_triple();
        self.module.set_triple(&triple);

        self.apply_runtime_attributes();
        self.allocation_hints = model.allocation.symbols.clone();
        self.uses_actors = !model.actor_handler_names.is_empty();

        self.module_exports = model.module_exports.clone();

        for (name, ty) in model.types.iter_all() {
            self.resolved_types.insert(name, ty);
        }

        self.resolved_locals = model.resolved_locals.clone();
        self.resolved_params = model.resolved_params.clone();
        self.resolved_returns = model.resolved_returns.clone();
        self.escape_info = model.escape_info.clone();
        self.monomorph_info = model.monomorph_info.clone();
        self.typed_lists = model.typed_lists.clone();

        let reachable = Self::compute_reachable_functions(model);

        self.declare_global_bindings(&model.globals);
        self.extern_sigs.clear();

        for extern_fn in &model.extern_functions {
            let mut param_types = Vec::new();
            for param in &extern_fn.params {
                let ann = param.type_annotation.as_ref().ok_or_else(|| {
                    Diagnostic::new("extern parameters require a type", param.span)
                })?;
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

        for function in &model.functions {
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

            self.fn_param_defaults
                .insert(function.name.clone(), function.params.clone());
        }

        for store in &model.stores {
            for (idx, field) in store.fields.iter().enumerate() {
                self.store_field_names.insert(field.name.clone());
                if field.is_reference {
                    self.reference_fields
                        .insert((store.name.clone(), field.name.clone()));
                }
                if !store.is_persistent {
                    self.store_field_indices
                        .insert((store.name.clone(), field.name.clone()), idx + 1);
                }
            }
            if !store.is_persistent {
                self.store_field_counts
                    .insert(store.name.clone(), store.fields.len() + 1);
            }

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
                        self.store_methods.insert(
                            method.name.clone(),
                            (store.name.clone(), method.params.len()),
                        );
                    }
                }
            }
        }

        for type_def in &model.type_defs {
            for variant in &type_def.variants {
                self.enum_constructors.insert(
                    variant.name.clone(),
                    (type_def.name.clone(), variant.fields.len()),
                );
            }

            for method in &type_def.methods {
                if method.kind == FunctionKind::Method {
                    let mangled = format!("{}_{}", type_def.name, method.name);
                    let mut param_types: Vec<BasicMetadataTypeEnum> =
                        vec![self.runtime.value_i64_type.into()];
                    for _ in 0..method.params.len() {
                        param_types.push(self.runtime.value_i64_type.into());
                    }
                    let fn_type = self.runtime.value_i64_type.fn_type(&param_types, false);
                    let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                    self.functions.insert(mangled.clone(), llvm_fn);
                    self.store_methods.insert(
                        method.name.clone(),
                        (type_def.name.clone(), method.params.len()),
                    );
                }
            }
        }

        self.build_global_initializer(&model.globals)?;

        for function in &model.functions {
            if !reachable.contains(&function.name) {
                continue;
            }
            if let Some(llvm_fn) = self.functions.get(&function.name) {
                self.build_function_body(function, *llvm_fn)?;
            }
        }

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
                for method in &store.methods {
                    if method.kind == FunctionKind::Method {
                        let mangled = format!("{}_{}", store.name, method.name);
                        if let Some(llvm_fn) = self.functions.get(&mangled) {
                            self.current_store_name = Some(store.name.clone());
                            self.build_store_method_body(method, *llvm_fn)?;
                            self.current_store_name = None;
                        }
                    }
                }
            }
        }

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

        for store in &model.stores {
            if store.is_actor {
                self.build_actor_constructor(store)?;
            } else {
                self.build_store_constructor(store)?;
            }
        }

        // ── Monomorphization: declare and build specialized function clones ──
        self.declare_specialized_functions(model)?;
        self.build_specialized_function_bodies(model)?;

        let main_fn =
            self.module
                .add_function("main", self.context.i32_type().fn_type(&[], false), None);
        let main_entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(main_entry);
        self.ensure_globals_initialized();
        if let Some(init_fn) = self.global_init_fn {
            self.builder
                .build_call(init_fn, &[], "init_globals")
                .unwrap();
        }

        if let Some(user_main) = self.functions.get("main") {
            let handler_ty = self.context.void_type().fn_type(
                &[
                    self.runtime.value_ptr_type.into(),
                    self.runtime.value_ptr_type.into(),
                ],
                false,
            );
            let handler_fn = self
                .module
                .add_function("__coral_main_handler", handler_ty, None);
            let h_entry = self.context.append_basic_block(handler_fn, "entry");
            self.builder.position_at_end(h_entry);
            let _ = self.builder.build_call(*user_main, &[], "call_user_main");

            if self.has_persistent_stores {
                let _ = self
                    .builder
                    .build_call(self.runtime.store_save_all, &[], "save_stores");
            }

            let _ = self
                .builder
                .build_call(self.runtime.main_done_signal, &[], "main_done");
            self.builder.build_return(None).unwrap();

            self.builder.position_at_end(main_entry);

            let handler_closure = self.call_runtime_ptr(
                self.runtime.make_closure,
                &[
                    handler_fn.as_global_value().as_pointer_value().into(),
                    self.runtime.value_ptr_type.const_null().into(),
                    self.runtime.value_ptr_type.const_null().into(),
                    self.usize_type.const_zero().into(),
                ],
                "main_handler_closure",
            );
            let actor = self.call_runtime_ptr(
                self.runtime.actor_spawn,
                &[handler_closure.into()],
                "main_actor",
            );

            let unit = self.wrap_unit();
            let unit_ptr = self.nb_to_ptr(unit);
            let _ = self.call_runtime_ptr(
                self.runtime.actor_send,
                &[actor.into(), unit_ptr.into()],
                "send_unit",
            );

            let _ = self.call_runtime_ptr(self.runtime.main_wait, &[], "wait_main");
        }
        self.builder
            .build_return(Some(&self.context.i32_type().const_int(0, false)))
            .unwrap();

        if let Some(dbg) = &self.debug_ctx {
            dbg.builder.finalize();
        }

        // Dead code elimination: mark monomorphized originals as private so
        // LLVM's GlobalDCE pass removes them when they have no remaining uses.
        for fn_name in self.monomorph_info.candidates.keys() {
            if let Some(&llvm_fn) = self.functions.get(fn_name) {
                llvm_fn.set_linkage(inkwell::module::Linkage::Private);
            }
        }

        Ok(self.module)
    }

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
            Statement::While {
                condition, body, ..
            } => Self::expr_calls_self(condition, name) || Self::body_calls_self(body, name),
            Statement::For { iterable, body, .. } => {
                Self::expr_calls_self(iterable, name) || Self::body_calls_self(body, name)
            }
            Statement::ForKV { iterable, body, .. } => {
                Self::expr_calls_self(iterable, name) || Self::body_calls_self(body, name)
            }
            Statement::ForRange {
                start,
                end,
                step,
                body,
                ..
            } => {
                Self::expr_calls_self(start, name)
                    || Self::expr_calls_self(end, name)
                    || step
                        .as_ref()
                        .map_or(false, |s| Self::expr_calls_self(s, name))
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
            Expression::Slice {
                target, start, end, ..
            } => {
                Self::expr_calls_self(target, name)
                    || Self::expr_calls_self(start, name)
                    || Self::expr_calls_self(end, name)
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

    fn expr_cache_key(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(..)
            | Expression::Integer(..)
            | Expression::Float(..)
            | Expression::Bool(..)
            | Expression::String(..)
            | Expression::Bytes(..) => None,

            Expression::Binary {
                op, left, right, ..
            } => {
                let lk = Self::expr_cache_key_inner(left)?;
                let rk = Self::expr_cache_key_inner(right)?;
                Some(format!("({:?} {} {})", op, lk, rk))
            }

            Expression::Unary {
                op, expr: inner, ..
            } => {
                let ik = Self::expr_cache_key_inner(inner)?;
                Some(format!("({:?} {})", op, ik))
            }

            Expression::Member { .. } | Expression::Index { .. } | Expression::Slice { .. } => None,

            Expression::Call { callee, args, .. } => match callee.as_ref() {
                Expression::Identifier(name, _) if Self::is_pure_function(name) => {
                    let mut key = format!("{}(", name);
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            key.push(',');
                        }
                        key.push_str(&Self::expr_cache_key_inner(arg)?);
                    }
                    key.push(')');
                    Some(key)
                }
                _ => None,
            },

            _ => None,
        }
    }

    fn expr_cache_key_inner(expr: &Expression) -> Option<String> {
        match expr {
            Expression::Identifier(name, _) => Some(format!("v:{}", name)),
            Expression::Integer(n, _) => Some(format!("i:{}", n)),
            Expression::Float(f, _) => Some(format!("f:{}", f)),
            Expression::Bool(b, _) => Some(format!("b:{}", b)),
            Expression::String(s, _) => Some(format!("s:{}", s)),

            other => Self::expr_cache_key(other),
        }
    }

    fn is_pure_function(name: &str) -> bool {
        matches!(
            name,
            "len"
                | "length"
                | "abs"
                | "sqrt"
                | "min"
                | "max"
                | "floor"
                | "ceil"
                | "round"
                | "to_string"
                | "to_number"
                | "type_of"
                | "is_number"
                | "is_string"
                | "is_bool"
                | "is_list"
                | "is_map"
        )
    }

    fn is_function_pure(body: &Block) -> bool {
        for stmt in &body.statements {
            if !Self::is_statement_pure(stmt) {
                return false;
            }
        }
        if let Some(ref val) = body.value {
            Self::is_expression_pure(val)
        } else {
            true
        }
    }

    fn is_statement_pure(stmt: &Statement) -> bool {
        match stmt {
            Statement::Binding(binding) => Self::is_expression_pure(&binding.value),
            Statement::Expression(expr) => Self::is_expression_pure(expr),
            Statement::If {
                condition,
                body,
                else_body,
                ..
            } => {
                Self::is_expression_pure(condition)
                    && Self::is_block_pure(body)
                    && else_body
                        .as_ref()
                        .map_or(true, |eb| Self::is_block_pure(eb))
            }
            Statement::Return(expr, _) => Self::is_expression_pure(expr),
            _ => false,
        }
    }

    fn is_block_pure(block: &Block) -> bool {
        for stmt in &block.statements {
            if !Self::is_statement_pure(stmt) {
                return false;
            }
        }
        block
            .value
            .as_ref()
            .map_or(true, |v| Self::is_expression_pure(v))
    }

    fn is_expression_pure(expr: &Expression) -> bool {
        match expr {
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bool(_, _)
            | Expression::Unit
            | Expression::None(_)
            | Expression::Identifier(_, _) => true,
            Expression::Binary { left, right, .. } => {
                Self::is_expression_pure(left) && Self::is_expression_pure(right)
            }
            Expression::Unary { expr: inner, .. } => Self::is_expression_pure(inner),
            Expression::Call { callee, args, .. } => {
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    if Self::is_pure_function(name) {
                        return args.iter().all(|a| Self::is_expression_pure(a));
                    }
                }
                false
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                Self::is_expression_pure(condition)
                    && Self::is_expression_pure(then_branch)
                    && Self::is_expression_pure(else_branch)
            }
            _ => false,
        }
    }

    fn apply_function_attributes(&self, llvm_fn: FunctionValue<'ctx>, is_pure: bool) {
        let nounwind_id = Attribute::get_named_enum_kind_id("nounwind");
        let nounwind = self.context.create_enum_attribute(nounwind_id, 0);
        llvm_fn.add_attribute(AttributeLoc::Function, nounwind);

        let noalias_id = Attribute::get_named_enum_kind_id("noalias");
        let noalias = self.context.create_enum_attribute(noalias_id, 0);
        let param_count = llvm_fn.count_params();
        for i in 0..param_count {
            if llvm_fn
                .get_nth_param(i)
                .map_or(false, |p| p.is_pointer_value())
            {
                llvm_fn.add_attribute(AttributeLoc::Param(i), noalias);
            }
        }

        if is_pure {
            let readnone_id = Attribute::get_named_enum_kind_id("memory");

            let readnone = self.context.create_enum_attribute(readnone_id, 0);
            llvm_fn.add_attribute(AttributeLoc::Function, readnone);

            let willreturn_id = Attribute::get_named_enum_kind_id("willreturn");
            let willreturn = self.context.create_enum_attribute(willreturn_id, 0);
            llvm_fn.add_attribute(AttributeLoc::Function, willreturn);
        }
    }

    fn apply_runtime_attributes(&self) {
        let nounwind_id = Attribute::get_named_enum_kind_id("nounwind");
        let nounwind = self.context.create_enum_attribute(nounwind_id, 0);

        let nounwind_fns = [
            self.runtime.make_number,
            self.runtime.make_string,
            self.runtime.make_bool,
            self.runtime.make_unit,
            self.runtime.make_bytes,
            self.runtime.make_list,
            self.runtime.make_map,
            self.runtime.value_add,
            self.runtime.value_equals,
            self.runtime.value_not_equals,
            self.runtime.value_length,
            self.runtime.type_of,
            self.runtime.list_push,
            self.runtime.list_get,
            self.runtime.list_length,
            self.runtime.list_pop,
            self.runtime.map_get,
            self.runtime.map_set,
            self.runtime.map_length,
            self.runtime.map_keys,
            self.runtime.list_get_fast,
            self.runtime.list_get_nb,
            self.runtime.list_get_raw_f64,
            self.runtime.list_get_sublist_ptr,
            self.runtime.list_items_raw,
            self.runtime.list_set_fast,
            self.runtime.list_len,
            self.runtime.string_len,
        ];
        for f in &nounwind_fns {
            f.add_attribute(AttributeLoc::Function, nounwind);
        }

        // Mark read-only functions with memory(read) attribute (no side effects)
        // This allows LLVM LICM to hoist them out of loops when arguments are invariant
        // Encoding: memory attr uses 2 bits per location
        // (ArgMem=loc0, InaccessibleMem=loc1, Other=loc2, ErrnoMem=loc3)
        // Ref=1 at all locations = 1 | (1<<2) | (1<<4) | (1<<6) = 85
        let memory_read_id = Attribute::get_named_enum_kind_id("memory");
        if memory_read_id > 0 {
            let memory_read = self.context.create_enum_attribute(memory_read_id, 85);
            let readonly_fns = [
                self.runtime.list_get_raw_f64,
                self.runtime.list_get_sublist_ptr,
                self.runtime.list_get_nb,
                self.runtime.list_len,
                self.runtime.string_len,
            ];
            for f in &readonly_fns {
                f.add_attribute(AttributeLoc::Function, memory_read);
            }
        }

        let noalias_id = Attribute::get_named_enum_kind_id("noalias");
        let noalias = self.context.create_enum_attribute(noalias_id, 0);
        let allocator_fns = [
            self.runtime.make_string,
            self.runtime.make_list,
            self.runtime.make_map,
            self.runtime.make_bytes,
            self.runtime.make_number,
            self.runtime.make_bool,
            self.runtime.make_unit,
        ];
        for f in &allocator_fns {
            f.add_attribute(AttributeLoc::Return, noalias);
        }
    }

    fn compute_reachable_functions(model: &SemanticModel) -> HashSet<String> {
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

        let mut reachable: HashSet<String> = HashSet::new();
        let mut worklist: Vec<String> = vec!["main".to_string()];

        for global in &model.globals {
            Self::collect_expr_refs(&global.value, &all_names, &method_name_map, &mut worklist);
        }

        while let Some(name) = worklist.pop() {
            if reachable.contains(&name) {
                continue;
            }
            reachable.insert(name.clone());

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
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
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
            Statement::While {
                condition, body, ..
            } => {
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
            Statement::ForRange {
                start,
                end,
                step,
                body,
                ..
            } => {
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

    fn collect_expr_refs(
        expr: &Expression,
        all_names: &HashSet<String>,
        method_map: &HashMap<String, Vec<String>>,
        worklist: &mut Vec<String>,
    ) {
        match expr {
            Expression::Identifier(name, _) => {
                if all_names.contains(name) {
                    worklist.push(name.clone());
                }
            }
            Expression::Call { callee, args, .. } => {
                if let Expression::Member {
                    target, property, ..
                } = callee.as_ref()
                {
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
            Expression::Member {
                target, property, ..
            } => {
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
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
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
            Expression::Slice {
                target, start, end, ..
            } => {
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
            Expression::ListComprehension {
                body,
                iterable,
                condition,
                ..
            } => {
                Self::collect_expr_refs(iterable, all_names, method_map, worklist);
                Self::collect_expr_refs(body, all_names, method_map, worklist);
                if let Some(cond) = condition {
                    Self::collect_expr_refs(cond, all_names, method_map, worklist);
                }
            }
            Expression::MapComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                Self::collect_expr_refs(iterable, all_names, method_map, worklist);
                Self::collect_expr_refs(key, all_names, method_map, worklist);
                Self::collect_expr_refs(value, all_names, method_map, worklist);
                if let Some(cond) = condition {
                    Self::collect_expr_refs(cond, all_names, method_map, worklist);
                }
            }

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
            MatchPattern::Integer(_)
            | MatchPattern::Bool(_)
            | MatchPattern::String(_)
            | MatchPattern::Wildcard(_)
            | MatchPattern::Range { .. }
            | MatchPattern::RangeBinding { .. }
            | MatchPattern::Rest(..) => {}
        }
    }

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
                ));
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
                    format!(
                        "extern return type not supported: `{}`",
                        self.format_type_enum(*other)
                    ),
                    Span::default(),
                ));
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

    // ── Monomorphization ─────────────────────────────────────────────────────

    /// Generate the mangled name for a specialized function variant.
    fn mangle_specialized_name(fn_name: &str, param_types: &[TypeId]) -> String {
        let suffix: Vec<&str> = param_types
            .iter()
            .map(|t| match t {
                TypeId::Primitive(Primitive::Int) => "Int",
                TypeId::Primitive(Primitive::Float) => "Float",
                TypeId::Primitive(Primitive::Bool) => "Bool",
                _ => "Box",
            })
            .collect();
        format!("{}_{}", fn_name, suffix.join("_"))
    }

    /// Declare LLVM functions for all monomorphization candidates with native-typed params.
    fn declare_specialized_functions(
        &mut self,
        model: &SemanticModel,
    ) -> Result<(), Diagnostic> {
        let candidates: Vec<(String, Vec<MonomorphVariant>)> = self
            .monomorph_info
            .candidates
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        for (fn_name, variants) in &candidates {
            // Skip if the function doesn't exist in the model
            let function = match model.functions.iter().find(|f| &f.name == fn_name) {
                Some(f) => f,
                None => continue,
            };

            for variant in variants {
                // Skip variants where arg count doesn't match function params
                if variant.param_types.len() != function.params.len() {
                    continue;
                }
                let mangled = Self::mangle_specialized_name(fn_name, &variant.param_types);

                // Build native param types
                let param_types: Vec<BasicMetadataTypeEnum> = variant
                    .param_types
                    .iter()
                    .map(|t| match t {
                        TypeId::Primitive(Primitive::Int) => self.usize_type.into(),
                        TypeId::Primitive(Primitive::Float) => self.f64_type.into(),
                        TypeId::Primitive(Primitive::Bool) => self.bool_type.into(),
                        _ => self.runtime.value_i64_type.into(),
                    })
                    .collect();

                // Return type
                let fn_type = match &variant.return_type {
                    TypeId::Primitive(Primitive::Int) => {
                        self.usize_type.fn_type(&param_types, false)
                    }
                    TypeId::Primitive(Primitive::Float) => {
                        self.f64_type.fn_type(&param_types, false)
                    }
                    TypeId::Primitive(Primitive::Bool) => {
                        self.bool_type.fn_type(&param_types, false)
                    }
                    _ => self.runtime.value_i64_type.fn_type(&param_types, false),
                };

                let llvm_fn = self.module.add_function(&mangled, fn_type, None);

                // Mark for inlining since these are specialized hot paths
                let kind_id = Attribute::get_named_enum_kind_id("inlinehint");
                let attr = self.context.create_enum_attribute(kind_id, 0);
                llvm_fn.add_attribute(AttributeLoc::Function, attr);

                self.specialized_functions.insert(
                    (fn_name.clone(), variant.param_types.clone()),
                    llvm_fn,
                );
            }
        }
        Ok(())
    }

    /// Build function bodies for all specialized monomorphization variants.
    fn build_specialized_function_bodies(
        &mut self,
        model: &SemanticModel,
    ) -> Result<(), Diagnostic> {
        let candidates: Vec<(String, Vec<MonomorphVariant>)> = self
            .monomorph_info
            .candidates
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        for (fn_name, variants) in &candidates {
            let function = match model.functions.iter().find(|f| &f.name == fn_name) {
                Some(f) => f.clone(),
                None => continue,
            };

            for variant in variants {
                // Skip variants where arg count doesn't match function params
                if variant.param_types.len() != function.params.len() {
                    continue;
                }
                let key = (fn_name.clone(), variant.param_types.clone());
                if let Some(&llvm_fn) = self.specialized_functions.get(&key) {
                    self.build_specialized_function_body(&function, llvm_fn, variant)?;
                }
            }
        }
        Ok(())
    }

    /// Build the body of a specialized (monomorphized) function variant.
    /// Parameters arrive as native types (i64 for Int, double for Float) — no unboxing needed.
    /// The body is emitted through the standard path; native arithmetic kicks in since params
    /// are marked as NativeInt/NativeFloat. The return value is converted to native type.
    fn build_specialized_function_body(
        &mut self,
        function: &Function,
        llvm_fn: FunctionValue<'ctx>,
        variant: &MonomorphVariant,
    ) -> Result<(), Diagnostic> {
        let entry = self.context.append_basic_block(llvm_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();

        let mut non_escaping = HashSet::new();
        for key in self.escape_info.non_escaping.iter() {
            if key.0 == function.name {
                non_escaping.insert(key.1.clone());
            }
        }

        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: llvm_fn,
            loop_stack: Vec::new(),
            di_scope: None,
            fn_name: function.name.clone(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
            lambda_out_param: None,
            unboxed_vars: HashMap::new(),
            non_escaping_locals: non_escaping,
            specialized_return_type: Some(variant.return_type.clone()),
        };
        self.current_fn_name = Some(function.name.clone());

        // Store params directly as native — NO unboxing needed
        for (i, param_ast) in function.params.iter().enumerate() {
            let param = llvm_fn.get_nth_param(i as u32).unwrap();
            match &variant.param_types[i] {
                TypeId::Primitive(Primitive::Int) => {
                    let native_int = param.into_int_value();
                    self.store_variable_native_int(&mut ctx, &param_ast.name, native_int);
                }
                TypeId::Primitive(Primitive::Float) => {
                    let native_float = param.into_float_value();
                    self.store_variable_native_float(&mut ctx, &param_ast.name, native_float);
                }
                _ => {
                    let value_nb = param.into_int_value();
                    self.store_variable(&mut ctx, &param_ast.name, value_nb);
                }
            }
        }

        ctx.in_tail_position = true;
        let block_value = self.emit_block(&mut ctx, &function.body)?;
        ctx.in_tail_position = false;

        // Convert the NaN-boxed return value to the specialized return type
        match &variant.return_type {
            TypeId::Primitive(Primitive::Int) => {
                let native_result = self.unbox_to_native_int(block_value);
                self.builder.build_return(Some(&native_result)).unwrap();
            }
            TypeId::Primitive(Primitive::Float) => {
                let native_result = self.value_to_number_fast(block_value);
                self.builder.build_return(Some(&native_result)).unwrap();
            }
            _ => {
                self.builder.build_return(Some(&block_value)).unwrap();
            }
        }

        Ok(())
    }

    /// Determine the static type of a call argument for monomorphization dispatch.
    fn infer_call_arg_type(&self, ctx: &FunctionContext<'ctx>, expr: &Expression) -> TypeId {
        match expr {
            Expression::Integer(_, _) => TypeId::Primitive(Primitive::Int),
            Expression::Float(_, _) => TypeId::Primitive(Primitive::Float),
            Expression::Bool(_, _) => TypeId::Primitive(Primitive::Bool),
            Expression::Identifier(name, _) => {
                if self.var_is_native_int(ctx, name) {
                    return TypeId::Primitive(Primitive::Int);
                }
                if self.var_is_native_float(ctx, name) {
                    return TypeId::Primitive(Primitive::Float);
                }
                // Check resolved types
                if let Some(ty) = self.resolved_locals.get(&(ctx.fn_name.clone(), name.clone())) {
                    return ty.clone();
                }
                if let Some(ty) = self.resolved_types.get(name.as_str()) {
                    return ty.clone();
                }
                TypeId::Unknown
            }
            Expression::Call { callee, .. } => {
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    if let Some(ty) = self.resolved_returns.get(name.as_str()) {
                        return ty.clone();
                    }
                }
                TypeId::Unknown
            }
            Expression::Binary {
                op, left, right, ..
            } => {
                if matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Mod
                ) {
                    if self.expr_is_int(ctx, left) && self.expr_is_int(ctx, right) {
                        return TypeId::Primitive(Primitive::Int);
                    }
                }
                TypeId::Unknown
            }
            Expression::Unary {
                op: UnaryOp::Neg,
                expr: inner,
                ..
            } => {
                if self.expr_is_int(ctx, inner) {
                    TypeId::Primitive(Primitive::Int)
                } else {
                    TypeId::Unknown
                }
            }
            _ => TypeId::Unknown,
        }
    }

    /// Emit a call to a specialized (monomorphized) function variant.
    /// Arguments are emitted as native types, and the native return is converted to NaN-boxed.
    fn emit_specialized_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        spec_fn: FunctionValue<'ctx>,
        variant: &MonomorphVariant,
        args: &[Expression],
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let mut arg_values: Vec<BasicMetadataValueEnum> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let saved_tail = ctx.in_tail_position;
            ctx.in_tail_position = false;
            match &variant.param_types[i] {
                TypeId::Primitive(Primitive::Int) => {
                    let val = self.emit_expression_as_native_int(ctx, arg)?;
                    arg_values.push(val.into());
                }
                TypeId::Primitive(Primitive::Float) => {
                    let boxed = self.emit_expression(ctx, arg)?;
                    let native = self.value_to_number_fast(boxed);
                    arg_values.push(native.into());
                }
                _ => {
                    let val = self.emit_expression(ctx, arg)?;
                    arg_values.push(val.into());
                }
            }
            ctx.in_tail_position = saved_tail;
        }

        let call = self
            .builder
            .build_call(spec_fn, &arg_values, "mono_call")
            .unwrap();

        let result = call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| Diagnostic::new("specialized call produced no value", Span::new(0, 0)))?;

        // Convert native return to NaN-boxed i64
        match &variant.return_type {
            TypeId::Primitive(Primitive::Int) => Ok(self.box_native_int(result.into_int_value())),
            TypeId::Primitive(Primitive::Float) => {
                Ok(self.wrap_number_unchecked(result.into_float_value()))
            }
            _ => Ok(result.into_int_value()),
        }
    }

    fn build_function_body(
        &mut self,
        function: &Function,
        llvm_fn: FunctionValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        if function.name != "main" {
            let stmt_count =
                function.body.statements.len() + if function.body.value.is_some() { 1 } else { 0 };
            if stmt_count <= 8 && !Self::body_calls_self(&function.body, &function.name) {
                let kind_id = Attribute::get_named_enum_kind_id("alwaysinline");
                let attr = self.context.create_enum_attribute(kind_id, 0);
                llvm_fn.add_attribute(AttributeLoc::Function, attr);
            } else if stmt_count <= 20 && !Self::body_calls_self(&function.body, &function.name) {
                let kind_id = Attribute::get_named_enum_kind_id("inlinehint");
                let attr = self.context.create_enum_attribute(kind_id, 0);
                llvm_fn.add_attribute(AttributeLoc::Function, attr);
            }
        }

        let is_pure = Self::is_function_pure(&function.body);
        self.apply_function_attributes(llvm_fn, is_pure);

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
        let mut non_escaping = HashSet::new();
        for key in self.escape_info.non_escaping.iter() {
            if key.0 == function.name {
                non_escaping.insert(key.1.clone());
            }
        }

        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: llvm_fn,
            loop_stack: Vec::new(),
            di_scope,
            fn_name: function.name.clone(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
            lambda_out_param: None,
            unboxed_vars: HashMap::new(),
            non_escaping_locals: non_escaping,
            specialized_return_type: None,
        };
        self.current_fn_name = Some(function.name.clone());

        for (i, (param, param_ast)) in llvm_fn.get_param_iter().zip(function.params.iter()).enumerate() {
            let value_nb = param.into_int_value();
            // Check if this parameter has a known specialized type
            if let Some(ty) = self.resolved_params.get(&(function.name.clone(), i)) {
                match ty {
                    TypeId::Primitive(Primitive::Int) => {
                        let native_int = self.unbox_to_native_int(value_nb);
                        self.store_variable_native_int(&mut ctx, &param_ast.name, native_int);
                        continue;
                    }
                    TypeId::Primitive(Primitive::Float) => {
                        let native_float = self.value_to_number_fast(value_nb);
                        self.store_variable_native_float(&mut ctx, &param_ast.name, native_float);
                        continue;
                    }
                    _ => {}
                }
            }
            self.store_variable(&mut ctx, &param_ast.name, value_nb);
        }

        ctx.in_tail_position = true;
        let block_value = self.emit_block(&mut ctx, &function.body)?;
        ctx.in_tail_position = false;

        self.builder.build_return(Some(&block_value)).unwrap();
        Ok(())
    }

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
            lambda_out_param: None,
            unboxed_vars: HashMap::new(),
            non_escaping_locals: HashSet::new(),
            specialized_return_type: None,
        };
        self.current_fn_name = Some(function.name.clone());

        let state_val = llvm_fn.get_nth_param(0).unwrap().into_int_value();
        self.store_variable(&mut ctx, "self", state_val);

        for (i, param_ast) in function.params.iter().enumerate() {
            let param = llvm_fn.get_nth_param((i + 1) as u32).unwrap();
            let value_nb = param.into_int_value();
            self.store_variable(&mut ctx, &param_ast.name, value_nb);
        }

        let block_value = self.emit_block(&mut ctx, &function.body)?;

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
                    // Check if this binding should use native (unboxed) representation
                    // BEFORE calling emit_expression, to avoid box/unbox round-trips
                    let fn_name = ctx.fn_name.clone();
                    let var_name = binding.name.clone();
                    let mut handled_native = false;
                    if let Some(ty) = self.resolved_locals.get(&(fn_name, var_name.clone())) {
                        match ty {
                            TypeId::Primitive(Primitive::Int) => {
                                if self.expr_is_int(ctx, &binding.value) {
                                    // Fast path: evaluate directly as native i64
                                    let native = self.emit_expression_as_native_int(ctx, &binding.value)?;
                                    self.store_variable_native_int(ctx, &binding.name, native);
                                } else {
                                    // Slow path: evaluate as boxed, then unbox
                                    let value = self.emit_expression(ctx, &binding.value)?;
                                    let native = self.unbox_to_native_int(value);
                                    self.store_variable_native_int(ctx, &binding.name, native);
                                }
                                let var_key = format!("v:{}", binding.name);
                                ctx.cse_cache.retain(|k, _| !k.contains(&var_key));
                                handled_native = true;
                            }
                            TypeId::Primitive(Primitive::Float) => {
                                let value = self.emit_expression(ctx, &binding.value)?;
                                let native = self.value_to_number_fast(value);
                                self.store_variable_native_float(ctx, &binding.name, native);
                                let var_key = format!("v:{}", binding.name);
                                ctx.cse_cache.retain(|k, _| !k.contains(&var_key));
                                handled_native = true;
                            }
                            _ => {}
                        }
                    }
                    if handled_native {
                        continue;
                    }

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

                    let var_key = format!("v:{}", binding.name);
                    ctx.cse_cache.retain(|k, _| !k.contains(&var_key));
                }
                Statement::Expression(expr) => {
                    let _ = self.emit_expression(ctx, expr)?;
                }
                Statement::Return(expr, _) => {
                    ctx.in_tail_position = true;
                    let value = self.emit_expression(ctx, expr)?;
                    ctx.in_tail_position = false;

                    if let Some(out_ptr) = ctx.lambda_out_param {
                        let result_ptr = self.nb_to_ptr(value);
                        self.builder.build_store(out_ptr, result_ptr).unwrap();
                        self.builder.build_return(None).unwrap();
                    } else if let Some(ref ret_ty) = ctx.specialized_return_type {
                        match ret_ty {
                            TypeId::Primitive(Primitive::Int) => {
                                let native = self.unbox_to_native_int(value);
                                self.builder.build_return(Some(&native)).unwrap();
                            }
                            TypeId::Primitive(Primitive::Float) => {
                                let native = self.value_to_number_fast(value);
                                self.builder.build_return(Some(&native)).unwrap();
                            }
                            _ => {
                                self.builder.build_return(Some(&value)).unwrap();
                            }
                        }
                    } else {
                        self.builder.build_return(Some(&value)).unwrap();
                    }

                    return Ok(self.runtime.value_i64_type.const_zero());
                }
                Statement::If {
                    condition,
                    body,
                    elif_branches,
                    else_body,
                    ..
                } => {
                    let function = ctx.function;
                    let cond_bool =
                        if let Some(i1_val) = self.try_emit_condition_i1(ctx, condition)? {
                            i1_val
                        } else {
                            let cond_is_bool = self.expr_is_bool(condition);
                            let cond_value = self.emit_expression(ctx, condition)?;
                            if cond_is_bool {
                                self.value_to_bool_fast(cond_value)
                            } else {
                                self.value_to_bool(cond_value)
                            }
                        };

                    let then_bb = self.context.append_basic_block(function, "if_then");
                    let merge_bb = self.context.append_basic_block(function, "if_merge");

                    let mut phi_incoming: Vec<(
                        IntValue<'ctx>,
                        inkwell::basic_block::BasicBlock<'ctx>,
                    )> = Vec::new();

                    let first_else_bb = if elif_branches.is_empty() && else_body.is_none() {
                        merge_bb
                    } else {
                        self.context.append_basic_block(function, "if_else")
                    };
                    self.builder
                        .build_conditional_branch(cond_bool, then_bb, first_else_bb)
                        .unwrap();

                    self.builder.position_at_end(then_bb);
                    ctx.cse_cache.clear();
                    let then_value = self.emit_block(ctx, body)?;
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        let then_end_bb = self.builder.get_insert_block().unwrap();
                        phi_incoming.push((then_value, then_end_bb));
                        self.builder.build_unconditional_branch(merge_bb).unwrap();
                    }

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

                            let elif_then_bb = self
                                .context
                                .append_basic_block(function, &format!("elif_then_{i}"));
                            let next_else_bb = if i + 1 < elif_branches.len() || else_body.is_some()
                            {
                                self.context
                                    .append_basic_block(function, &format!("elif_else_{i}"))
                            } else {
                                merge_bb
                            };
                            self.builder
                                .build_conditional_branch(
                                    elif_cond_bool,
                                    elif_then_bb,
                                    next_else_bb,
                                )
                                .unwrap();

                            self.builder.position_at_end(elif_then_bb);
                            ctx.cse_cache.clear();
                            let elif_value = self.emit_block(ctx, elif_body)?;
                            if self
                                .builder
                                .get_insert_block()
                                .unwrap()
                                .get_terminator()
                                .is_none()
                            {
                                let elif_end_bb = self.builder.get_insert_block().unwrap();
                                phi_incoming.push((elif_value, elif_end_bb));
                                self.builder.build_unconditional_branch(merge_bb).unwrap();
                            }
                            current_else_bb = next_else_bb;
                        }
                        if let Some(else_block) = else_body {
                            self.builder.position_at_end(current_else_bb);
                            ctx.cse_cache.clear();
                            let else_value = self.emit_block(ctx, else_block)?;
                            if self
                                .builder
                                .get_insert_block()
                                .unwrap()
                                .get_terminator()
                                .is_none()
                            {
                                let else_end_bb = self.builder.get_insert_block().unwrap();
                                phi_incoming.push((else_value, else_end_bb));
                                self.builder.build_unconditional_branch(merge_bb).unwrap();
                            }
                        }
                    }

                    if else_body.is_none()
                        && (elif_branches.is_empty() || elif_branches.last().is_some())
                    {
                    }

                    self.builder.position_at_end(merge_bb);
                    ctx.cse_cache.clear();

                    if !phi_incoming.is_empty() && else_body.is_some() {
                        let phi = self
                            .builder
                            .build_phi(self.runtime.value_i64_type, "if_phi")
                            .unwrap();
                        for (val, bb) in &phi_incoming {
                            phi.add_incoming(&[(val as &dyn BasicValue<'ctx>, *bb)]);
                        }

                        let if_result = phi.as_basic_value().into_int_value();
                        self.store_variable(ctx, "__if_result", if_result);
                    }
                }
                Statement::While {
                    condition, body, ..
                } => {
                    let function = ctx.function;
                    let loop_header = self.context.append_basic_block(function, "while_cond");
                    let loop_body = self.context.append_basic_block(function, "while_body");
                    let loop_exit = self.context.append_basic_block(function, "while_exit");

                    self.builder
                        .build_unconditional_branch(loop_header)
                        .unwrap();

                    self.builder.position_at_end(loop_header);
                    let cond_bool =
                        if let Some(i1_val) = self.try_emit_condition_i1(ctx, condition)? {
                            i1_val
                        } else {
                            let while_cond_is_bool = self.expr_is_bool(condition);
                            let cond_value = self.emit_expression(ctx, condition)?;
                            if while_cond_is_bool {
                                self.value_to_bool_fast(cond_value)
                            } else {
                                self.value_to_bool(cond_value)
                            }
                        };
                    self.builder
                        .build_conditional_branch(cond_bool, loop_body, loop_exit)
                        .unwrap();

                    self.builder.position_at_end(loop_body);

                    if self.uses_actors {
                        self.builder
                            .build_call(self.runtime.actor_yield_check, &[], "yield_chk")
                            .unwrap();
                    }
                    ctx.cse_cache.clear();
                    ctx.loop_stack.push((loop_header, loop_exit));
                    self.emit_block(ctx, body)?;
                    ctx.loop_stack.pop();
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }

                    self.builder.position_at_end(loop_exit);
                }
                Statement::For {
                    variable,
                    iterable,
                    body,
                    ..
                } => {
                    let function = ctx.function;
                    let iter_value = self.emit_expression(ctx, iterable)?;

                    // Determine if iterable is a known list at compile time
                    let known_list = self.expr_is_known_list(ctx, iterable);

                    if known_list {
                        // ── Fast path: direct counted loop over list items ──
                        let list_ptr = self.nb_extract_heap_ptr(iter_value);

                        // Get length as usize
                        let len = self
                            .builder
                            .build_call(
                                self.runtime.list_len,
                                &[list_ptr.into()],
                                "for_list_len",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();

                        let zero = self.usize_type.const_int(0, false);
                        let one = self.usize_type.const_int(1, false);

                        // Allocate counter
                        let counter = self.builder.build_alloca(self.usize_type, "for_idx").unwrap();
                        self.builder.build_store(counter, zero).unwrap();

                        let loop_header = self.context.append_basic_block(function, "for_list_cond");
                        let loop_body = self.context.append_basic_block(function, "for_list_body");
                        let loop_exit = self.context.append_basic_block(function, "for_list_exit");

                        self.builder.build_unconditional_branch(loop_header).unwrap();

                        // Loop header: check i < len
                        self.builder.position_at_end(loop_header);
                        let idx = self
                            .builder
                            .build_load(self.usize_type, counter, "for_i")
                            .unwrap()
                            .into_int_value();
                        let cmp = self
                            .builder
                            .build_int_compare(IntPredicate::ULT, idx, len, "for_list_done")
                            .unwrap();
                        self.builder
                            .build_conditional_branch(cmp, loop_body, loop_exit)
                            .unwrap();

                        // Loop body: get element, run body, increment counter
                        self.builder.position_at_end(loop_body);

                        if self.uses_actors {
                            self.builder
                                .build_call(self.runtime.actor_yield_check, &[], "yield_chk")
                                .unwrap();
                        }
                        ctx.cse_cache.clear();

                        let elem_nb = self.call_nb(
                            self.runtime.list_get_nb,
                            &[list_ptr.into(), idx.into()],
                            "for_list_elem",
                        );
                        self.store_variable(ctx, variable, elem_nb);

                        ctx.loop_stack.push((loop_header, loop_exit));
                        self.emit_block(ctx, body)?;
                        ctx.loop_stack.pop();

                        // Increment counter and branch back
                        if self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_terminator()
                            .is_none()
                        {
                            let next_idx = self
                                .builder
                                .build_int_add(
                                    self.builder
                                        .build_load(self.usize_type, counter, "for_i_cur")
                                        .unwrap()
                                        .into_int_value(),
                                    one,
                                    "for_i_next",
                                )
                                .unwrap();
                            self.builder.build_store(counter, next_idx).unwrap();
                            self.builder.build_unconditional_branch(loop_header).unwrap();
                        }

                        self.builder.position_at_end(loop_exit);
                        // nb_extract_heap_ptr does not retain, so no release needed
                    } else {
                        // ── Fallback: runtime iterator-based loop (original path) ──
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

                        self.builder.position_at_end(loop_header);
                        let elem_ptr = self.call_runtime_ptr(
                            self.runtime.value_iter_next,
                            &[iter.into()],
                            "for_next",
                        );
                        let tag_ptr = self
                            .builder
                            .build_pointer_cast(
                                elem_ptr,
                                self.i8_type.ptr_type(AddressSpace::default()),
                                "tag_ptr",
                            )
                            .unwrap();
                        let tag_val = self
                            .builder
                            .build_load(self.i8_type, tag_ptr, "tag_val")
                            .unwrap()
                            .into_int_value();
                        let unit_tag = self.i8_type.const_int(7, false);
                        let is_done = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, tag_val, unit_tag, "for_done")
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_done, loop_exit, loop_body)
                            .unwrap();

                        self.builder.position_at_end(loop_body);
                        if self.uses_actors {
                            self.builder
                                .build_call(self.runtime.actor_yield_check, &[], "yield_chk")
                                .unwrap();
                        }
                        ctx.cse_cache.clear();
                        let elem_nb = self.ptr_to_nb(elem_ptr);
                        self.store_variable(ctx, variable, elem_nb);

                        ctx.loop_stack.push((loop_header, loop_exit));
                        self.emit_block(ctx, body)?;
                        ctx.loop_stack.pop();

                        if self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_terminator()
                            .is_none()
                        {
                            self.builder.build_unconditional_branch(loop_header).unwrap();
                        }

                        self.builder.position_at_end(loop_exit);
                        self.call_runtime_void(
                            self.runtime.value_release,
                            &[iter.into()],
                            "release_iter",
                        );
                    }
                }
                Statement::ForKV {
                    key_var,
                    value_var,
                    iterable,
                    body,
                    ..
                } => {
                    let function = ctx.function;
                    let iter_value = self.emit_expression(ctx, iterable)?;

                    let entries_list =
                        self.call_bridged(self.runtime.map_entries, &[iter_value], "forkv_entries");

                    let entries_ptr = self.nb_to_ptr(entries_list);
                    let iter = self.call_runtime_ptr(
                        self.runtime.value_iter,
                        &[entries_ptr.into()],
                        "forkv_iter",
                    );

                    let loop_header = self.context.append_basic_block(function, "forkv_cond");
                    let loop_body = self.context.append_basic_block(function, "forkv_body");
                    let loop_exit = self.context.append_basic_block(function, "forkv_exit");

                    self.builder
                        .build_unconditional_branch(loop_header)
                        .unwrap();

                    self.builder.position_at_end(loop_header);
                    let elem_ptr = self.call_runtime_ptr(
                        self.runtime.value_iter_next,
                        &[iter.into()],
                        "forkv_next",
                    );
                    let tag_ptr = self
                        .builder
                        .build_pointer_cast(
                            elem_ptr,
                            self.i8_type.ptr_type(AddressSpace::default()),
                            "tag_ptr",
                        )
                        .unwrap();
                    let tag_val = self
                        .builder
                        .build_load(self.i8_type, tag_ptr, "tag_val")
                        .unwrap()
                        .into_int_value();
                    let unit_tag = self.i8_type.const_int(7, false);
                    let is_done = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, tag_val, unit_tag, "forkv_done")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(is_done, loop_exit, loop_body)
                        .unwrap();

                    self.builder.position_at_end(loop_body);

                    if self.uses_actors {
                        self.builder
                            .build_call(self.runtime.actor_yield_check, &[], "yield_chk")
                            .unwrap();
                    }
                    ctx.cse_cache.clear();
                    let pair_nb = self.ptr_to_nb(elem_ptr);
                    let index_zero = self.wrap_number_unchecked(self.f64_type.const_float(0.0));
                    let index_one = self.wrap_number_unchecked(self.f64_type.const_float(1.0));
                    let key_nb = self.call_bridged(
                        self.runtime.list_get,
                        &[pair_nb, index_zero],
                        "forkv_key",
                    );
                    let val_nb = self.call_bridged(
                        self.runtime.list_get,
                        &[pair_nb, index_one],
                        "forkv_val",
                    );
                    self.store_variable(ctx, key_var, key_nb);
                    self.store_variable(ctx, value_var, val_nb);
                    ctx.loop_stack.push((loop_header, loop_exit));
                    self.emit_block(ctx, body)?;
                    ctx.loop_stack.pop();
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }

                    self.builder.position_at_end(loop_exit);
                    self.call_runtime_void(
                        self.runtime.value_release,
                        &[iter.into()],
                        "release_iter",
                    );
                }
                Statement::ForRange {
                    variable,
                    start,
                    end,
                    step,
                    body,
                    ..
                } => {
                    let function = ctx.function;
                    let start_is_int = self.expr_is_int(ctx, start);
                    let end_is_int = self.expr_is_int(ctx, end);
                    let step_is_int = step
                        .as_ref()
                        .map(|s| self.expr_is_int(ctx, s))
                        .unwrap_or(true);
                    let use_native_int = start_is_int && end_is_int && step_is_int;
                    let start_is_num = use_native_int || self.expr_is_numeric(start);
                    let end_is_num = use_native_int || self.expr_is_numeric(end);
                    let fast_path = start_is_num && end_is_num;

                    if use_native_int {
                        // === Native i64 loop path ===
                        let start_i64 = self.emit_expression_as_native_int(ctx, start)?;
                        let end_i64 = self.emit_expression_as_native_int(ctx, end)?;
                        let step_i64 = if let Some(step_expr) = step {
                            self.emit_expression_as_native_int(ctx, step_expr)?
                        } else {
                            self.runtime.value_i64_type.const_int(1, false)
                        };

                        let counter_alloca = self
                            .builder
                            .build_alloca(self.runtime.value_i64_type, "for_irange_counter")
                            .unwrap();
                        self.builder
                            .build_store(counter_alloca, start_i64)
                            .unwrap();

                        let loop_header =
                            self.context.append_basic_block(function, "for_irange_cond");
                        let loop_body =
                            self.context.append_basic_block(function, "for_irange_body");
                        let loop_exit =
                            self.context.append_basic_block(function, "for_irange_exit");

                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();

                        self.builder.position_at_end(loop_header);
                        let current = self
                            .builder
                            .build_load(
                                self.runtime.value_i64_type,
                                counter_alloca,
                                "icur",
                            )
                            .unwrap()
                            .into_int_value();
                        let is_done = self
                            .builder
                            .build_int_compare(
                                IntPredicate::SGE,
                                current,
                                end_i64,
                                "for_irange_done",
                            )
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_done, loop_exit, loop_body)
                            .unwrap();

                        self.builder.position_at_end(loop_body);

                        if self.uses_actors {
                            self.builder
                                .build_call(self.runtime.actor_yield_check, &[], "yield_chk")
                                .unwrap();
                        }
                        ctx.cse_cache.clear();

                        // Store loop variable as native int
                        self.store_variable_native_int(ctx, variable, current);
                        ctx.loop_stack.push((loop_header, loop_exit));
                        self.emit_block(ctx, body)?;
                        ctx.loop_stack.pop();

                        if self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_terminator()
                            .is_none()
                        {
                            let updated_current = self
                                .builder
                                .build_load(
                                    self.runtime.value_i64_type,
                                    counter_alloca,
                                    "icur_upd",
                                )
                                .unwrap()
                                .into_int_value();
                            let next = self
                                .builder
                                .build_int_add(updated_current, step_i64, "inext_counter")
                                .unwrap();
                            self.builder.build_store(counter_alloca, next).unwrap();
                            self.builder
                                .build_unconditional_branch(loop_header)
                                .unwrap();
                        }

                        self.builder.position_at_end(loop_exit);
                    } else {
                        // === Original f64 loop path ===
                        let start_val = self.emit_expression(ctx, start)?;
                        let end_val = self.emit_expression(ctx, end)?;
                        let step_f64 = if let Some(step_expr) = step {
                            let step_nb = self.emit_expression(ctx, step_expr)?;
                            if self.expr_is_numeric(step_expr) {
                                self.value_to_number_fast(step_nb)
                            } else {
                                self.value_to_number(step_nb)
                            }
                        } else {
                            self.f64_type.const_float(1.0)
                        };

                        let start_f64 = if fast_path {
                            self.value_to_number_fast(start_val)
                        } else {
                            self.value_to_number(start_val)
                        };
                        let end_f64 = if fast_path {
                            self.value_to_number_fast(end_val)
                        } else {
                            self.value_to_number(end_val)
                        };

                        let counter_alloca = self
                            .builder
                            .build_alloca(self.f64_type, "for_range_counter")
                            .unwrap();
                        self.builder.build_store(counter_alloca, start_f64).unwrap();

                        let loop_header =
                            self.context.append_basic_block(function, "for_range_cond");
                        let loop_body =
                            self.context.append_basic_block(function, "for_range_body");
                        let loop_exit =
                            self.context.append_basic_block(function, "for_range_exit");

                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();

                        self.builder.position_at_end(loop_header);
                        let current = self
                            .builder
                            .build_load(self.f64_type, counter_alloca, "cur")
                            .unwrap()
                            .into_float_value();
                        let is_done = self
                            .builder
                            .build_float_compare(
                                inkwell::FloatPredicate::OGE,
                                current,
                                end_f64,
                                "for_range_done",
                            )
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_done, loop_exit, loop_body)
                            .unwrap();

                        self.builder.position_at_end(loop_body);

                        if self.uses_actors {
                            self.builder
                                .build_call(self.runtime.actor_yield_check, &[], "yield_chk")
                                .unwrap();
                        }
                        ctx.cse_cache.clear();
                        let counter_nb = self.wrap_number_unchecked(current);
                        self.store_variable(ctx, variable, counter_nb);
                        ctx.loop_stack.push((loop_header, loop_exit));
                        self.emit_block(ctx, body)?;
                        ctx.loop_stack.pop();

                        if self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_terminator()
                            .is_none()
                        {
                            let updated_current = self
                                .builder
                                .build_load(self.f64_type, counter_alloca, "cur_upd")
                                .unwrap()
                                .into_float_value();
                            let next = self
                                .builder
                                .build_float_add(
                                    updated_current,
                                    step_f64,
                                    "next_counter",
                                )
                                .unwrap();
                            self.builder.build_store(counter_alloca, next).unwrap();
                            self.builder
                                .build_unconditional_branch(loop_header)
                                .unwrap();
                        }

                        self.builder.position_at_end(loop_exit);
                    }
                }
                Statement::FieldAssign {
                    target,
                    field,
                    value,
                    ..
                } => {
                    let target_value = self.emit_expression(ctx, &target)?;
                    let new_value = self.emit_expression(ctx, &value)?;

                    let mut used_struct_set = false;
                    if let Expression::Identifier(name, _) = &target {
                        let store_name_opt = if name == "self" {
                            self.current_store_name.clone()
                        } else if let Some(crate::types::core::TypeId::Store(sn)) =
                            self.resolved_types.get(name.as_str())
                        {
                            Some(sn.clone())
                        } else if let Some(crate::types::core::TypeId::Store(sn)) =
                            self.resolved_locals.get(&(ctx.fn_name.clone(), name.clone()))
                        {
                            Some(sn.clone())
                        } else {
                            self.current_store_name.clone()
                        };
                        if let Some(store_name) = store_name_opt {
                            if let Some(&idx) =
                                self.store_field_indices.get(&(store_name, field.clone()))
                            {
                                self.inline_struct_set_nb(target_value, idx as u64, new_value);
                                used_struct_set = true;
                            }
                        }
                    }

                    if !used_struct_set {
                        let key_value = self.emit_string_literal(&field);
                        let target_ptr = self.nb_to_ptr(target_value);
                        let key_ptr = self.nb_to_ptr(key_value);
                        let new_ptr = self.nb_to_ptr(new_value);

                        if let Expression::Identifier(name, _) = &target {
                            if name == "self" {
                                let is_ref = self
                                    .reference_fields
                                    .iter()
                                    .any(|(_, f)| f == field.as_str());
                                if is_ref {
                                    let old_value = self.call_runtime_ptr(
                                        self.runtime.map_get,
                                        &[target_ptr.into(), key_ptr.into()],
                                        "old_field_value",
                                    );
                                    self.call_runtime_void(
                                        self.runtime.value_release,
                                        &[old_value.into()],
                                        "release_old",
                                    );
                                    self.call_runtime_void(
                                        self.runtime.value_retain,
                                        &[new_ptr.into()],
                                        "retain_new",
                                    );
                                }
                            }
                        }

                        self.call_runtime_ptr(
                            self.runtime.map_set,
                            &[target_ptr.into(), key_ptr.into(), new_ptr.into()],
                            "map_set_field",
                        );
                    }

                    if let Expression::Identifier(name, _) = &target {
                        let var_key = format!("v:{}", name);
                        ctx.cse_cache.retain(|k, _| !k.contains(&var_key));
                    } else {
                        ctx.cse_cache.clear();
                    }
                }
                Statement::Break(_) => {
                    if let Some(&(_, loop_exit)) = ctx.loop_stack.last() {
                        self.builder.build_unconditional_branch(loop_exit).unwrap();
                    }

                    let function = ctx.function;
                    let unreachable_bb = self.context.append_basic_block(function, "after_break");
                    self.builder.position_at_end(unreachable_bb);
                }
                Statement::Continue(_) => {
                    if let Some(&(loop_header, _)) = ctx.loop_stack.last() {
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }
                    let function = ctx.function;
                    let unreachable_bb =
                        self.context.append_basic_block(function, "after_continue");
                    self.builder.position_at_end(unreachable_bb);
                }

                Statement::PatternBinding { pattern, value, .. } => {
                    let val = self.emit_expression(ctx, value)?;
                    self.bind_pattern_variables(ctx, val, pattern);
                }
            }
        }

        if let Some(expr) = &block.value {
            self.emit_expression(ctx, expr.as_ref())
        } else {
            // 0.0 as f64 bits = 0 as i64, which is the NaN-boxed representation of 0.0
            Ok(self.runtime.value_i64_type.const_zero())
        }
    }

    fn resolve_named_args(
        &self,
        fn_name: &str,
        args: &[Expression],
        arg_names: &[Option<String>],
        span: Span,
    ) -> Result<Vec<Expression>, Diagnostic> {
        let param_defs = self.fn_param_defaults.get(fn_name).ok_or_else(|| {
            Diagnostic::new(
                format!(
                    "cannot use named arguments for unknown function `{}`",
                    fn_name
                ),
                span,
            )
        })?;

        let positional_count = arg_names.iter().take_while(|n| n.is_none()).count();
        let named_start = positional_count;

        let mut result: Vec<Option<Expression>> = vec![None; param_defs.len()];

        for i in 0..positional_count {
            if i >= param_defs.len() {
                return Err(Diagnostic::new(
                    format!(
                        "too many arguments for `{}`: expected {}, got at least {}",
                        fn_name,
                        param_defs.len(),
                        positional_count
                    ),
                    span,
                ));
            }
            result[i] = Some(args[i].clone());
        }

        for i in named_start..args.len() {
            let name = arg_names[i].as_ref().unwrap();
            let param_idx = param_defs
                .iter()
                .position(|p| &p.name == name)
                .ok_or_else(|| {
                    Diagnostic::new(
                        format!("unknown parameter `{}` in call to `{}`", name, fn_name),
                        span,
                    )
                })?;
            if result[param_idx].is_some() {
                return Err(Diagnostic::new(
                    format!(
                        "duplicate argument for parameter `{}` in call to `{}`",
                        name, fn_name
                    ),
                    span,
                ));
            }
            result[param_idx] = Some(args[i].clone());
        }

        let mut resolved = Vec::new();
        for (idx, slot) in result.iter().enumerate() {
            match slot {
                Some(expr) => resolved.push(expr.clone()),
                None => {
                    if let Some(ref default_expr) = param_defs[idx].default {
                        resolved.push(default_expr.clone());
                    } else {
                        return Err(Diagnostic::new(
                            format!(
                                "missing argument `{}` with no default in call to `{}`",
                                param_defs[idx].name, fn_name
                            ),
                            span,
                        ));
                    }
                }
            }
        }

        Ok(resolved)
    }

    fn emit_expression(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        if let Some(scope) = ctx.di_scope {
            self.set_debug_location(expr.span(), scope);
        }

        if let Some(key) = Self::expr_cache_key(expr) {
            if let Some(&cached) = ctx.cse_cache.get(&key) {
                return Ok(cached);
            }

            let result = self.emit_expression_inner(ctx, expr)?;
            ctx.cse_cache.insert(key, result);
            return Ok(result);
        }
        self.emit_expression_inner(ctx, expr)
    }

    fn emit_expression_inner(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match expr {
            Expression::Integer(value, _) => {
                let f = *value as f64;
                let bits = f.to_bits();
                Ok(if (bits & 0xFFF8_0000_0000_0000) == 0x7FF8_0000_0000_0000 {
                    self.runtime
                        .value_i64_type
                        .const_int(0x7FFB_8000_0000_0000, false)
                } else {
                    self.runtime.value_i64_type.const_int(bits, false)
                })
            }
            Expression::Float(value, _) => {
                let bits = value.to_bits();
                Ok(if (bits & 0xFFF8_0000_0000_0000) == 0x7FF8_0000_0000_0000 {
                    self.runtime
                        .value_i64_type
                        .const_int(0x7FFB_8000_0000_0000, false)
                } else {
                    self.runtime.value_i64_type.const_int(bits, false)
                })
            }
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
            Expression::Lambda { params, body, span } => self.emit_lambda(ctx, params, body, *span),
            Expression::Placeholder(_, span) => Err(Diagnostic::new(
                "placeholder expressions require higher-order lowering, which is not implemented yet",
                *span,
            )),
            Expression::Binary {
                op, left, right, ..
            } => match op {
                BinaryOp::And | BinaryOp::Or => self.emit_logical_binary(ctx, *op, left, right),
                _ => {
                    let saved_tail = ctx.in_tail_position;
                    ctx.in_tail_position = false;

                    let both_int = self.expr_is_int(ctx, left) && self.expr_is_int(ctx, right);
                    let both_numeric = both_int || (self.expr_is_numeric(left) && self.expr_is_numeric(right));

                    if both_int && matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Equals | BinaryOp::NotEquals | BinaryOp::Less | BinaryOp::LessEq | BinaryOp::Greater | BinaryOp::GreaterEq) {
                        let lhs = self.emit_expression_as_native_int(ctx, left)?;
                        let rhs = self.emit_expression_as_native_int(ctx, right)?;
                        ctx.in_tail_position = saved_tail;
                        return self.emit_native_int_binary(*op, lhs, rhs);
                    }

                    let lhs = self.emit_expression(ctx, left)?;
                    let rhs = self.emit_expression(ctx, right)?;
                    ctx.in_tail_position = saved_tail;
                    self.emit_numeric_binary(*op, lhs, rhs, both_numeric)
                }
            },
            Expression::Unary { op, expr, .. } => {
                let saved_tail = ctx.in_tail_position;
                ctx.in_tail_position = false;
                let is_bool = self.expr_is_bool(expr);
                let value = self.emit_expression(ctx, expr)?;
                ctx.in_tail_position = saved_tail;
                match op {
                    UnaryOp::Neg => {
                        if self.expr_is_numeric(expr) {
                            let as_number = self.value_to_number_fast(value);
                            let neg = self.builder.build_float_neg(as_number, "neg_fast").unwrap();
                            Ok(self.wrap_number_unchecked(neg))
                        } else {
                            let as_number = self.value_to_number(value);
                            let neg = self.builder.build_float_neg(as_number, "neg").unwrap();
                            Ok(self.wrap_number(neg))
                        }
                    }
                    UnaryOp::Not => {
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
            Expression::Call {
                callee,
                args,
                arg_names,
                ..
            } => {
                let args = if !arg_names.is_empty() {
                    if let Expression::Identifier(name, _) = callee.as_ref() {
                        self.resolve_named_args(name, args, arg_names, expr.span())?
                    } else {
                        return Err(Diagnostic::new(
                            "named arguments are only supported for named function calls",
                            expr.span(),
                        ));
                    }
                } else {
                    args.clone()
                };
                let args = &args;

                if let Expression::Member {
                    target,
                    property,
                    span,
                } = callee.as_ref()
                {
                    let result = self.emit_member_call(ctx, target, property, args, *span);
                    ctx.cse_cache.clear();
                    return result;
                }
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    if let Some(result) = self.emit_builtin_call(name, args, ctx, expr.span())? {
                        return Ok(result);
                    }

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
                            let ret_val = call.try_as_basic_value().left().ok_or_else(|| {
                                Diagnostic::new("extern call produced no value", expr.span())
                            })?;
                            return self.wrap_extern_return(ret_val, *ret_ty, expr.span());
                        } else {
                            return Ok(self.wrap_unit());
                        }
                    }

                    // ── Monomorphization: try specialized variant first ──
                    if self.monomorph_info.candidates.contains_key(name.as_str()) {
                        let arg_types: Vec<TypeId> = args
                            .iter()
                            .map(|arg| self.infer_call_arg_type(ctx, arg))
                            .collect();
                        let key = (name.to_string(), arg_types.clone());
                        if let Some(&spec_fn) = self.specialized_functions.get(&key) {
                            if let Some(variants) = self.monomorph_info.candidates.get(name.as_str())
                            {
                                if let Some(variant) =
                                    variants.iter().find(|v| v.param_types == arg_types)
                                {
                                    let variant = variant.clone();
                                    return self
                                        .emit_specialized_call(ctx, spec_fn, &variant, args);
                                }
                            }
                        }
                    }

                    if let Some(&function) = self.functions.get(name) {
                        let mut arg_values = Vec::new();
                        for arg in args {
                            let saved_tail = ctx.in_tail_position;
                            ctx.in_tail_position = false;
                            let value = self.emit_expression(ctx, arg)?;
                            ctx.in_tail_position = saved_tail;
                            arg_values.push(value);
                        }

                        if let Some(param_defs) = self.fn_param_defaults.get(name).cloned() {
                            let provided = arg_values.len();
                            let expected = param_defs.len();
                            if provided < expected {
                                let mut temp_bindings: Vec<String> = Vec::new();
                                for (idx, pdef) in param_defs.iter().enumerate().take(provided) {
                                    if !ctx.variables.contains_key(&pdef.name) {
                                        ctx.variables.insert(pdef.name.clone(), arg_values[idx]);
                                        temp_bindings.push(pdef.name.clone());
                                    }
                                }
                                for i in provided..expected {
                                    if let Some(default_expr) = &param_defs[i].default {
                                        let saved_tail = ctx.in_tail_position;
                                        ctx.in_tail_position = false;
                                        let value = self.emit_expression(ctx, default_expr)?;
                                        ctx.in_tail_position = saved_tail;

                                        if !ctx.variables.contains_key(&param_defs[i].name) {
                                            ctx.variables.insert(param_defs[i].name.clone(), value);
                                            temp_bindings.push(param_defs[i].name.clone());
                                        }
                                        arg_values.push(value);
                                    } else {
                                        for tb in &temp_bindings {
                                            ctx.variables.remove(tb);
                                        }
                                        return Err(Diagnostic::new(
                                            format!(
                                                "missing argument `{}` with no default",
                                                param_defs[i].name
                                            ),
                                            expr.span(),
                                        ));
                                    }
                                }

                                for tb in &temp_bindings {
                                    ctx.variables.remove(tb);
                                }
                            }
                        }
                        let metadata_args: Vec<BasicMetadataValueEnum> =
                            arg_values.iter().map(|v| (*v).into()).collect();
                        let call = self
                            .builder
                            .build_call(function, &metadata_args, "call")
                            .unwrap();

                        if ctx.in_tail_position && name == &ctx.fn_name {
                            call.set_tail_call(true);
                        }

                        let value = call
                            .try_as_basic_value()
                            .left()
                            .ok_or_else(|| Diagnostic::new("call produced no value", expr.span()))?
                            .into_int_value();
                        Ok(value)
                    } else if let Some((enum_name, expected_field_count)) =
                        self.enum_constructors.get(name).cloned()
                    {
                        if args.len() != expected_field_count {
                            return Err(Diagnostic::new(
                                format!(
                                    "enum constructor `{}::{}` expects {} argument(s), found {}",
                                    enum_name,
                                    name,
                                    expected_field_count,
                                    args.len()
                                ),
                                expr.span(),
                            ));
                        }
                        self.emit_enum_constructor(ctx, name, args)
                    } else if ctx.variables.contains_key(name)
                        || ctx.variable_allocas.contains_key(name)
                    {
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
            Expression::Member {
                target,
                property,
                span,
            } => self.emit_member_expression(ctx, target, property, *span),
            Expression::Index {
                target,
                index,
                span: _,
            } => {
                let target_val = self.emit_expression(ctx, target)?;
                let index_val = self.emit_expression(ctx, index)?;

                let target_ptr = self.nb_to_ptr(target_val);
                let index_ptr = self.nb_to_ptr(index_val);
                let result = self.call_runtime_ptr(
                    self.runtime.list_get,
                    &[target_ptr.into(), index_ptr.into()],
                    "subscript",
                );
                Ok(self.ptr_to_nb(result))
            }
            Expression::Slice {
                target, start, end, ..
            } => {
                let target_val = self.emit_expression(ctx, target)?;
                let start_val = self.emit_expression(ctx, start)?;
                let end_val = self.emit_expression(ctx, end)?;
                let result = self.call_bridged(
                    self.runtime.list_slice,
                    &[target_val, start_val, end_val],
                    "slice",
                );
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
            Expression::None(_) => Ok(self.wrap_none()),
            Expression::InlineAsm {
                template,
                inputs,
                span,
                ..
            } => {
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
                let addr_val = self.emit_expression(ctx, address)?;
                let addr_num = self.value_to_number(addr_val);
                let addr_int = self
                    .builder
                    .build_bitcast(addr_num, self.usize_type, "addr_usize")
                    .map_err(|e| Diagnostic::new(format!("ptr load bitcast failed: {e}"), *span))?;
                let addr_ptr = self
                    .builder
                    .build_int_to_ptr(
                        addr_int.into_int_value(),
                        self.f64_type.ptr_type(AddressSpace::default()),
                        "addr_ptr",
                    )
                    .map_err(|e| {
                        Diagnostic::new(format!("ptr load int_to_ptr failed: {e}"), *span)
                    })?;
                let loaded = self
                    .builder
                    .build_load(self.f64_type, addr_ptr, "ptr_load")
                    .map_err(|e| Diagnostic::new(format!("ptr load failed: {e}"), *span))?
                    .into_float_value();
                Ok(self.wrap_number(loaded))
            }
            Expression::Unsafe { block, .. } => self.emit_block(ctx, block),
            Expression::Pipeline { left, right, span } => match right.as_ref() {
                Expression::Call {
                    callee,
                    args,
                    span: call_span,
                    ..
                } => {
                    let has_placeholder = args.iter().any(|arg| self.contains_placeholder(arg));

                    let new_args = if has_placeholder {
                        args.iter()
                            .map(|arg| self.replace_placeholder_with(arg, left.as_ref()))
                            .collect()
                    } else {
                        let mut new_args = vec![left.as_ref().clone()];
                        new_args.extend(args.iter().cloned());
                        new_args
                    };

                    let desugared = Expression::Call {
                        callee: callee.clone(),
                        args: new_args,
                        arg_names: vec![],
                        span: *call_span,
                    };
                    self.emit_expression(ctx, &desugared)
                }
                Expression::Identifier(name, id_span) => {
                    let desugared = Expression::Call {
                        callee: Box::new(Expression::Identifier(name.clone(), *id_span)),
                        args: vec![left.as_ref().clone()],
                        arg_names: vec![],
                        span: *span,
                    };
                    self.emit_expression(ctx, &desugared)
                }
                _ => Err(Diagnostic::new(
                    "pipeline right-hand side must be a function call or identifier",
                    *span,
                )),
            },
            Expression::ErrorValue { path, span: _ } => {
                let error_name = path.join(":");
                let name_bytes = error_name.as_bytes();

                let name_array = self.context.const_string(name_bytes, false);
                let name_global = self.module.add_global(
                    name_array.get_type(),
                    Some(AddressSpace::default()),
                    &format!("err_name_{}", error_name.replace(':', "_")),
                );
                name_global.set_linkage(inkwell::module::Linkage::Private);
                name_global.set_initializer(&name_array);
                name_global.set_constant(true);

                let name_ptr = self
                    .builder
                    .build_pointer_cast(
                        name_global.as_pointer_value(),
                        self.i8_type.ptr_type(AddressSpace::default()),
                        "err_name_ptr",
                    )
                    .unwrap();

                let error_code = self.context.i32_type().const_int(0, false);
                let name_len = self.usize_type.const_int(name_bytes.len() as u64, false);

                let err_ptr = self.call_runtime_ptr(
                    self.runtime.make_error,
                    &[error_code.into(), name_ptr.into(), name_len.into()],
                    "make_error",
                );
                Ok(self.ptr_to_nb(err_ptr))
            }
            Expression::Spread(inner, _) => self.emit_expression(ctx, inner),
            Expression::ListComprehension {
                body,
                var,
                iterable,
                condition,
                span: _,
            } => {
                let function = ctx.function;
                let list_value = self.emit_expression(ctx, iterable)?;

                let len_nb = self.call_bridged(self.runtime.list_length, &[list_value], "lc_len");
                let len_f64 = self.value_to_number(len_nb);

                let null_ptr = self
                    .runtime
                    .value_ptr_type
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null();
                let zero_usize = self.usize_type.const_int(0, false);
                let out_list_ptr = self.call_runtime_ptr(
                    self.runtime.make_list,
                    &[null_ptr.into(), zero_usize.into()],
                    "lc_out_list",
                );
                let out_list_nb = self.ptr_to_nb(out_list_ptr);

                let out_alloca = self
                    .builder
                    .build_alloca(self.runtime.value_i64_type, "lc_out_alloca")
                    .unwrap();
                self.builder.build_store(out_alloca, out_list_nb).unwrap();

                let counter_alloca = self
                    .builder
                    .build_alloca(self.f64_type, "lc_counter")
                    .unwrap();
                self.builder
                    .build_store(counter_alloca, self.f64_type.const_float(0.0))
                    .unwrap();

                let loop_header = self.context.append_basic_block(function, "lc_cond");
                let loop_body = self.context.append_basic_block(function, "lc_body");
                let loop_exit = self.context.append_basic_block(function, "lc_exit");

                self.builder
                    .build_unconditional_branch(loop_header)
                    .unwrap();

                self.builder.position_at_end(loop_header);
                let current = self
                    .builder
                    .build_load(self.f64_type, counter_alloca, "lc_i")
                    .unwrap()
                    .into_float_value();
                let is_done = self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OGE, current, len_f64, "lc_done")
                    .unwrap();
                self.builder
                    .build_conditional_branch(is_done, loop_exit, loop_body)
                    .unwrap();

                self.builder.position_at_end(loop_body);
                ctx.cse_cache.clear();
                let idx_nb = self.wrap_number(current);
                let elem_nb =
                    self.call_bridged(self.runtime.list_get, &[list_value, idx_nb], "lc_elem");
                self.store_variable(ctx, var, elem_nb);

                if let Some(cond) = condition {
                    let cond_val = self.emit_expression(ctx, cond)?;
                    let is_truthy =
                        self.call_nb(self.runtime.nb_is_truthy, &[cond_val.into()], "lc_truthy");
                    let truthy_bool = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            is_truthy,
                            self.i8_type.const_int(0, false),
                            "lc_cond_bool",
                        )
                        .unwrap();
                    let lc_push = self.context.append_basic_block(function, "lc_push");
                    let lc_skip = self.context.append_basic_block(function, "lc_skip");
                    self.builder
                        .build_conditional_branch(truthy_bool, lc_push, lc_skip)
                        .unwrap();

                    self.builder.position_at_end(lc_push);
                    let body_val = self.emit_expression(ctx, body)?;
                    let cur_out = self
                        .builder
                        .build_load(self.runtime.value_i64_type, out_alloca, "lc_cur_out")
                        .unwrap()
                        .into_int_value();
                    let new_out = self.call_bridged(
                        self.runtime.list_push,
                        &[cur_out, body_val],
                        "lc_push_res",
                    );
                    self.builder.build_store(out_alloca, new_out).unwrap();

                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        let cur_f64 = self
                            .builder
                            .build_load(self.f64_type, counter_alloca, "lc_cur_upd")
                            .unwrap()
                            .into_float_value();
                        let next = self
                            .builder
                            .build_float_add(cur_f64, self.f64_type.const_float(1.0), "lc_next")
                            .unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }

                    self.builder.position_at_end(lc_skip);
                    let cur_f64 = self
                        .builder
                        .build_load(self.f64_type, counter_alloca, "lc_skip_upd")
                        .unwrap()
                        .into_float_value();
                    let next = self
                        .builder
                        .build_float_add(cur_f64, self.f64_type.const_float(1.0), "lc_skip_next")
                        .unwrap();
                    self.builder.build_store(counter_alloca, next).unwrap();
                    self.builder
                        .build_unconditional_branch(loop_header)
                        .unwrap();
                } else {
                    let body_val = self.emit_expression(ctx, body)?;
                    let cur_out = self
                        .builder
                        .build_load(self.runtime.value_i64_type, out_alloca, "lc_cur_out")
                        .unwrap()
                        .into_int_value();
                    let new_out = self.call_bridged(
                        self.runtime.list_push,
                        &[cur_out, body_val],
                        "lc_push_res",
                    );
                    self.builder.build_store(out_alloca, new_out).unwrap();

                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        let cur_f64 = self
                            .builder
                            .build_load(self.f64_type, counter_alloca, "lc_cur_upd")
                            .unwrap()
                            .into_float_value();
                        let next = self
                            .builder
                            .build_float_add(cur_f64, self.f64_type.const_float(1.0), "lc_next")
                            .unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }
                }

                self.builder.position_at_end(loop_exit);
                let final_out = self
                    .builder
                    .build_load(self.runtime.value_i64_type, out_alloca, "lc_result")
                    .unwrap()
                    .into_int_value();
                Ok(final_out)
            }
            Expression::MapComprehension {
                key,
                value,
                var,
                iterable,
                condition,
                span: _,
            } => {
                let function = ctx.function;
                let list_value = self.emit_expression(ctx, iterable)?;

                let len_nb = self.call_bridged(self.runtime.list_length, &[list_value], "mc_len");
                let len_f64 = self.value_to_number(len_nb);

                let entry_ptr_type = self
                    .runtime
                    .map_entry_type
                    .ptr_type(inkwell::AddressSpace::default());
                let null_entries = entry_ptr_type.const_null();
                let zero_usize = self.usize_type.const_int(0, false);
                let out_map_ptr = self.call_runtime_ptr(
                    self.runtime.make_map,
                    &[null_entries.into(), zero_usize.into()],
                    "mc_out_map",
                );
                let out_map_nb = self.ptr_to_nb(out_map_ptr);

                let out_alloca = self
                    .builder
                    .build_alloca(self.runtime.value_i64_type, "mc_out_alloca")
                    .unwrap();
                self.builder.build_store(out_alloca, out_map_nb).unwrap();
                let counter_alloca = self
                    .builder
                    .build_alloca(self.f64_type, "mc_counter")
                    .unwrap();
                self.builder
                    .build_store(counter_alloca, self.f64_type.const_float(0.0))
                    .unwrap();

                let loop_header = self.context.append_basic_block(function, "mc_cond");
                let loop_body = self.context.append_basic_block(function, "mc_body");
                let loop_exit = self.context.append_basic_block(function, "mc_exit");

                self.builder
                    .build_unconditional_branch(loop_header)
                    .unwrap();

                self.builder.position_at_end(loop_header);
                let current = self
                    .builder
                    .build_load(self.f64_type, counter_alloca, "mc_i")
                    .unwrap()
                    .into_float_value();
                let is_done = self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OGE, current, len_f64, "mc_done")
                    .unwrap();
                self.builder
                    .build_conditional_branch(is_done, loop_exit, loop_body)
                    .unwrap();

                self.builder.position_at_end(loop_body);
                ctx.cse_cache.clear();
                let idx_nb = self.wrap_number(current);
                let elem_nb =
                    self.call_bridged(self.runtime.list_get, &[list_value, idx_nb], "mc_elem");
                self.store_variable(ctx, var, elem_nb);

                if let Some(cond) = condition {
                    let cond_val = self.emit_expression(ctx, cond)?;
                    let is_truthy =
                        self.call_nb(self.runtime.nb_is_truthy, &[cond_val.into()], "mc_truthy");
                    let truthy_bool = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            is_truthy,
                            self.i8_type.const_int(0, false),
                            "mc_cond_bool",
                        )
                        .unwrap();
                    let mc_set = self.context.append_basic_block(function, "mc_set");
                    let mc_skip = self.context.append_basic_block(function, "mc_skip");
                    self.builder
                        .build_conditional_branch(truthy_bool, mc_set, mc_skip)
                        .unwrap();

                    self.builder.position_at_end(mc_set);
                    let key_val = self.emit_expression(ctx, key)?;
                    let val_val = self.emit_expression(ctx, value)?;
                    let cur_map = self
                        .builder
                        .build_load(self.runtime.value_i64_type, out_alloca, "mc_cur_map")
                        .unwrap()
                        .into_int_value();
                    let new_map = self.call_bridged(
                        self.runtime.map_set,
                        &[cur_map, key_val, val_val],
                        "mc_set_res",
                    );
                    self.builder.build_store(out_alloca, new_map).unwrap();
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        let cur_f64 = self
                            .builder
                            .build_load(self.f64_type, counter_alloca, "mc_i_upd")
                            .unwrap()
                            .into_float_value();
                        let next = self
                            .builder
                            .build_float_add(cur_f64, self.f64_type.const_float(1.0), "mc_next")
                            .unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }

                    self.builder.position_at_end(mc_skip);
                    let cur_f64 = self
                        .builder
                        .build_load(self.f64_type, counter_alloca, "mc_skip_upd")
                        .unwrap()
                        .into_float_value();
                    let next = self
                        .builder
                        .build_float_add(cur_f64, self.f64_type.const_float(1.0), "mc_skip_next")
                        .unwrap();
                    self.builder.build_store(counter_alloca, next).unwrap();
                    self.builder
                        .build_unconditional_branch(loop_header)
                        .unwrap();
                } else {
                    let key_val = self.emit_expression(ctx, key)?;
                    let val_val = self.emit_expression(ctx, value)?;
                    let cur_map = self
                        .builder
                        .build_load(self.runtime.value_i64_type, out_alloca, "mc_cur_map")
                        .unwrap()
                        .into_int_value();
                    let new_map = self.call_bridged(
                        self.runtime.map_set,
                        &[cur_map, key_val, val_val],
                        "mc_set_res",
                    );
                    self.builder.build_store(out_alloca, new_map).unwrap();
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        let cur_f64 = self
                            .builder
                            .build_load(self.f64_type, counter_alloca, "mc_i_upd")
                            .unwrap()
                            .into_float_value();
                        let next = self
                            .builder
                            .build_float_add(cur_f64, self.f64_type.const_float(1.0), "mc_next")
                            .unwrap();
                        self.builder.build_store(counter_alloca, next).unwrap();
                        self.builder
                            .build_unconditional_branch(loop_header)
                            .unwrap();
                    }
                }

                self.builder.position_at_end(loop_exit);
                let final_map = self
                    .builder
                    .build_load(self.runtime.value_i64_type, out_alloca, "mc_result")
                    .unwrap()
                    .into_int_value();
                Ok(final_map)
            }
            Expression::ErrorPropagate { expr, span: _ } => {
                let value = self.emit_expression(ctx, expr)?;

                let is_err = self
                    .builder
                    .build_call(self.runtime.nb_is_err, &[value.into()], "is_err_check")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let is_err_bool = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        is_err,
                        self.i8_type.const_zero(),
                        "is_err_bool",
                    )
                    .unwrap();

                let current_fn = ctx.function;
                let err_return_bb = self.context.append_basic_block(current_fn, "err_return");
                let continue_bb = self.context.append_basic_block(current_fn, "err_continue");

                self.builder
                    .build_conditional_branch(is_err_bool, err_return_bb, continue_bb)
                    .unwrap();

                self.builder.position_at_end(err_return_bb);
                self.builder.build_return(Some(&value)).unwrap();

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
            .build_pointer_cast(alloca, handles_ptr_type, "list_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(ptrs.len() as u64, false);
        let args = &[ptr.into(), len_value.into()];
        let list_ptr = self.call_list_with_hint(args, hint);
        Ok(list_ptr)
    }

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

        let null_ptr = handles_ptr_type.const_null();
        let len_zero = self.usize_type.const_zero();
        let mut list = self.call_list_with_hint(&[null_ptr.into(), len_zero.into()], hint);
        for element in elements {
            if let Expression::Spread(inner, _) = element {
                let spread_val = self.emit_expression(ctx, inner)?;
                list = self.call_bridged(
                    self.runtime.list_concat,
                    &[list, spread_val],
                    "spread_concat",
                );
            } else {
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
        let alloca = self
            .builder
            .build_alloca(array_type, "map_literal")
            .unwrap();
        self.builder.build_store(alloca, temp_array).unwrap();
        let ptr = self
            .builder
            .build_pointer_cast(alloca, entry_ptr_type, "map_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(evaluated.len() as u64, false);
        let args = &[ptr.into(), len_value.into()];
        Ok(self.call_map_with_hint(args, hint))
    }

    /// Check if an expression is known to produce a List value at compile time.
    /// Handles identifiers with known List types and chained .get() on lists of lists.
    fn expr_is_known_list(&self, ctx: &FunctionContext<'ctx>, expr: &Expression) -> bool {
        match expr {
            Expression::Identifier(name, _) => {
                // Direct variable with known list type
                let key = (ctx.fn_name.clone(), name.clone());
                self.resolved_locals
                    .get(&key)
                    .map_or(false, |ty| matches!(ty, TypeId::List(_)))
                    || self.typed_lists.contains_key(&key)
                    || self.resolved_types
                        .get(name.as_str())
                        .map_or(false, |ty| matches!(ty, TypeId::List(_)))
            }
            // Chained .get() on a known list — result is a list if element type is List
            Expression::Call {
                callee, args, ..
            } if args.len() == 1 => {
                if let Expression::Member { target, property, .. } = callee.as_ref() {
                    if property == "get" {
                        // Check if target is a known list whose elements are also lists
                        if let Expression::Identifier(name, _) = target.as_ref() {
                            let key = (ctx.fn_name.clone(), name.clone());
                            if let Some(TypeId::List(elem)) = self.resolved_locals.get(&key) {
                                return matches!(elem.as_ref(), TypeId::List(_));
                            }
                            if let Some(TypeId::List(elem)) = self.resolved_types.get(name.as_str()) {
                                return matches!(elem.as_ref(), TypeId::List(_));
                            }
                        }
                    }
                }
                false
            }
            Expression::List(_, _) => true,
            _ => false,
        }
    }

    /// Detect a chained .get().get() pattern where the inner .get() returns a sublist.
    /// Returns Some((inner_target, inner_args)) if the target is a .get() call on a
    /// known list-of-lists, meaning the intermediate result is a sublist pointer.
    fn detect_chained_list_get<'a>(
        &self,
        ctx: &FunctionContext<'ctx>,
        target: &'a Expression,
    ) -> Option<(&'a Expression, &'a [Expression])> {
        // target must be Call { callee: Member { target: inner, property: "get" }, args }
        if let Expression::Call { callee, args, .. } = target {
            if args.len() == 1 {
                if let Expression::Member { target: inner, property, .. } = callee.as_ref() {
                    if property == "get" && self.expr_is_known_list(ctx, inner) {
                        // Check if the known list's element type is also a List
                        if let Expression::Identifier(name, _) = inner.as_ref() {
                            let key = (ctx.fn_name.clone(), name.clone());
                            if let Some(TypeId::List(elem)) = self.resolved_locals.get(&key) {
                                if matches!(elem.as_ref(), TypeId::List(_)) {
                                    return Some((inner, args.as_slice()));
                                }
                            }
                            if let Some(TypeId::List(elem)) = self.resolved_types.get(name.as_str()) {
                                if matches!(elem.as_ref(), TypeId::List(_)) {
                                    return Some((inner, args.as_slice()));
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn expr_is_numeric(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Float(_, _) | Expression::Integer(_, _) => true,
            Expression::Identifier(name, _) => {
                if matches!(
                    self.resolved_types.get(name.as_str()),
                    Some(TypeId::Primitive(Primitive::Float))
                        | Some(TypeId::Primitive(Primitive::Int))
                ) {
                    return true;
                }
                // Check resolved_locals for local variables in the current function
                if let Some(ref fn_name) = self.current_fn_name {
                    if matches!(
                        self.resolved_locals.get(&(fn_name.clone(), name.clone())),
                        Some(TypeId::Primitive(Primitive::Float))
                            | Some(TypeId::Primitive(Primitive::Int))
                    ) {
                        return true;
                    }
                }
                false
            }
            Expression::Unary {
                op: UnaryOp::Neg,
                expr: inner,
                ..
            } => self.expr_is_numeric(inner),
            Expression::Binary {
                op, left, right, ..
            } => {
                // Sub/Mul/Div/Mod always produce numbers via inline float arithmetic
                // Add is numeric only when both operands are known numeric (since Add also handles string concat)
                matches!(op, BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod)
                    || (matches!(op, BinaryOp::Add)
                        && self.expr_is_numeric(left)
                        && self.expr_is_numeric(right))
            }
            Expression::Ternary {
                then_branch,
                else_branch,
                ..
            } => self.expr_is_numeric(then_branch) && self.expr_is_numeric(else_branch),
            // Member access on a known store where the field has a numeric default
            Expression::Member {
                target, property, ..
            } => {
                if let Expression::Identifier(name, _) = target.as_ref() {
                    // Check if a store field from current store context
                    let store_name_opt = self
                        .resolved_types
                        .get(name.as_str())
                        .and_then(|ty| {
                            if let TypeId::Store(s) = ty { Some(s.clone()) } else { None }
                        })
                        .or_else(|| {
                            if name == "self" { self.current_store_name.clone() } else { None }
                        })
                        .or_else(|| self.current_store_name.clone());
                    if let Some(ref store_name) = store_name_opt {
                        self.store_field_indices
                            .contains_key(&(store_name.clone(), property.clone()))
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Expression::Call { callee, .. } => {
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    matches!(
                        name.as_str(),
                        "abs"
                            | "floor"
                            | "ceil"
                            | "round"
                            | "sqrt"
                            | "sin"
                            | "cos"
                            | "tan"
                            | "asin"
                            | "acos"
                            | "atan"
                            | "atan2"
                            | "exp"
                            | "ln"
                            | "log2"
                            | "log10"
                            | "pow"
                            | "min"
                            | "max"
                            | "length"
                            | "size"
                            | "to_number"
                            | "time_now"
                            | "random"
                    )
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn expr_is_bool(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Bool(_, _) => true,
            Expression::Identifier(name, _) => {
                matches!(
                    self.resolved_types.get(name.as_str()),
                    Some(TypeId::Primitive(Primitive::Bool))
                )
            }
            Expression::Binary { op, .. } => matches!(
                op,
                BinaryOp::Equals
                    | BinaryOp::NotEquals
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEq
                    | BinaryOp::Less
                    | BinaryOp::LessEq
                    | BinaryOp::And
                    | BinaryOp::Or
            ),
            Expression::Unary {
                op: UnaryOp::Not, ..
            } => true,
            _ => false,
        }
    }

    /// Try to emit a condition expression directly as an i1 boolean,
    /// avoiding box-then-unbox round-trip for comparison operators in loops.
    /// Returns None if the expression can't be directly evaluated to i1.
    fn try_emit_condition_i1(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<Option<IntValue<'ctx>>, Diagnostic> {
        match expr {
            // Native integer comparison path
            Expression::Binary {
                op, left, right, ..
            } if matches!(
                op,
                BinaryOp::Less
                    | BinaryOp::LessEq
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEq
                    | BinaryOp::Equals
                    | BinaryOp::NotEquals
            ) && self.expr_is_int(ctx, left)
                && self.expr_is_int(ctx, right) =>
            {
                let saved_tail = ctx.in_tail_position;
                ctx.in_tail_position = false;
                let lhs = self.emit_expression_as_native_int(ctx, left)?;
                let rhs = self.emit_expression_as_native_int(ctx, right)?;
                ctx.in_tail_position = saved_tail;
                let pred = match op {
                    BinaryOp::Less => IntPredicate::SLT,
                    BinaryOp::LessEq => IntPredicate::SLE,
                    BinaryOp::Greater => IntPredicate::SGT,
                    BinaryOp::GreaterEq => IntPredicate::SGE,
                    BinaryOp::Equals => IntPredicate::EQ,
                    BinaryOp::NotEquals => IntPredicate::NE,
                    _ => unreachable!(),
                };
                let cmp = self
                    .builder
                    .build_int_compare(pred, lhs, rhs, "icmp_i1")
                    .unwrap();
                Ok(Some(cmp))
            }
            // Numeric (float) comparison path
            Expression::Binary {
                op, left, right, ..
            } if matches!(
                op,
                BinaryOp::Less
                    | BinaryOp::LessEq
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEq
                    | BinaryOp::Equals
                    | BinaryOp::NotEquals
            ) && self.expr_is_numeric(left)
                && self.expr_is_numeric(right) =>
            {
                let saved_tail = ctx.in_tail_position;
                ctx.in_tail_position = false;
                let lhs = self.emit_expression(ctx, left)?;
                let rhs = self.emit_expression(ctx, right)?;
                ctx.in_tail_position = saved_tail;
                let lhs_num = self.value_to_number_fast(lhs);
                let rhs_num = self.value_to_number_fast(rhs);
                let pred = match op {
                    BinaryOp::Less => FloatPredicate::OLT,
                    BinaryOp::LessEq => FloatPredicate::OLE,
                    BinaryOp::Greater => FloatPredicate::OGT,
                    BinaryOp::GreaterEq => FloatPredicate::OGE,
                    BinaryOp::Equals => FloatPredicate::OEQ,
                    BinaryOp::NotEquals => FloatPredicate::ONE,
                    _ => unreachable!(),
                };
                let cmp = self
                    .builder
                    .build_float_compare(pred, lhs_num, rhs_num, "cmp_i1")
                    .unwrap();
                Ok(Some(cmp))
            }
            Expression::Bool(val, _) => {
                let i1_val = self.context.bool_type().const_int(*val as u64, false);
                Ok(Some(i1_val))
            }
            Expression::Unary {
                op: UnaryOp::Not,
                expr: inner,
                ..
            } => {
                if let Some(inner_i1) = self.try_emit_condition_i1(ctx, inner)? {
                    Ok(Some(self.builder.build_not(inner_i1, "not_i1").unwrap()))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn emit_numeric_binary(
        &mut self,
        op: BinaryOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        both_numeric: bool,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        use BinaryOp::*;

        if both_numeric {
            let lhs_num = self.value_to_number_fast(lhs);
            let rhs_num = self.value_to_number_fast(rhs);
            return Ok(match op {
                Add => self.wrap_number_unchecked(
                    self.builder
                        .build_float_add(lhs_num, rhs_num, "add_fast")
                        .unwrap(),
                ),
                Sub => self.wrap_number_unchecked(
                    self.builder
                        .build_float_sub(lhs_num, rhs_num, "sub_fast")
                        .unwrap(),
                ),
                Mul => self.wrap_number_unchecked(
                    self.builder
                        .build_float_mul(lhs_num, rhs_num, "mul_fast")
                        .unwrap(),
                ),
                Div => self.wrap_number_fast(
                    self.builder
                        .build_float_div(lhs_num, rhs_num, "div_fast")
                        .unwrap(),
                ),
                Mod => self.wrap_number_fast(
                    self.builder
                        .build_float_rem(lhs_num, rhs_num, "rem_fast")
                        .unwrap(),
                ),
                Equals => self.wrap_bool(
                    self.builder
                        .build_float_compare(FloatPredicate::OEQ, lhs_num, rhs_num, "eq_fast")
                        .unwrap(),
                ),
                NotEquals => self.wrap_bool(
                    self.builder
                        .build_float_compare(FloatPredicate::ONE, lhs_num, rhs_num, "ne_fast")
                        .unwrap(),
                ),
                Greater => self.wrap_bool(
                    self.builder
                        .build_float_compare(FloatPredicate::OGT, lhs_num, rhs_num, "gt_fast")
                        .unwrap(),
                ),
                GreaterEq => self.wrap_bool(
                    self.builder
                        .build_float_compare(FloatPredicate::OGE, lhs_num, rhs_num, "ge_fast")
                        .unwrap(),
                ),
                Less => self.wrap_bool(
                    self.builder
                        .build_float_compare(FloatPredicate::OLT, lhs_num, rhs_num, "lt_fast")
                        .unwrap(),
                ),
                LessEq => self.wrap_bool(
                    self.builder
                        .build_float_compare(FloatPredicate::OLE, lhs_num, rhs_num, "le_fast")
                        .unwrap(),
                ),
                _ => {
                    return self.emit_numeric_binary_general(op, lhs, rhs);
                }
            });
        }

        if matches!(op, Add) {
            return Ok(self.call_nb(self.runtime.nb_add, &[lhs.into(), rhs.into()], "nb_add"));
        }
        if matches!(op, Equals) {
            return Ok(self.call_nb(
                self.runtime.nb_equals,
                &[lhs.into(), rhs.into()],
                "nb_equals",
            ));
        }
        if matches!(op, NotEquals) {
            return Ok(self.call_nb(
                self.runtime.nb_not_equals,
                &[lhs.into(), rhs.into()],
                "nb_not_equals",
            ));
        }

        self.emit_numeric_binary_general(op, lhs, rhs)
    }

    fn emit_numeric_binary_general(
        &mut self,
        op: BinaryOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        use BinaryOp::*;

        if matches!(op, BitAnd) {
            return Ok(self.call_bridged(self.runtime.value_bitand, &[lhs, rhs], "bitand"));
        }
        if matches!(op, BitOr) {
            return Ok(self.call_bridged(self.runtime.value_bitor, &[lhs, rhs], "bitor"));
        }
        if matches!(op, BitXor) {
            return Ok(self.call_bridged(self.runtime.value_bitxor, &[lhs, rhs], "bitxor"));
        }
        if matches!(op, ShiftLeft) {
            return Ok(self.call_bridged(self.runtime.value_shift_left, &[lhs, rhs], "shift_left"));
        }
        if matches!(op, ShiftRight) {
            return Ok(self.call_bridged(
                self.runtime.value_shift_right,
                &[lhs, rhs],
                "shift_right",
            ));
        }
        let lhs_num = self.value_to_number_fast(lhs);
        let rhs_num = self.value_to_number_fast(rhs);
        Ok(match op {
            Add | Equals | NotEquals => unreachable!(),
            Sub => self.wrap_number_unchecked(
                self.builder
                    .build_float_sub(lhs_num, rhs_num, "sub")
                    .unwrap(),
            ),
            Mul => self.wrap_number_unchecked(
                self.builder
                    .build_float_mul(lhs_num, rhs_num, "mul")
                    .unwrap(),
            ),
            Div => self.wrap_number_fast(
                self.builder
                    .build_float_div(lhs_num, rhs_num, "div")
                    .unwrap(),
            ),
            Mod => self.wrap_number_fast(
                self.builder
                    .build_float_rem(lhs_num, rhs_num, "rem")
                    .unwrap(),
            ),
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
            And | Or => unreachable!(),
        })
    }

    /// Emit a binary operation on two native i64 integers, returning a NaN-boxed result.
    fn emit_native_int_binary(
        &mut self,
        op: BinaryOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        use BinaryOp::*;
        Ok(match op {
            Add => {
                let result = self.builder.build_int_add(lhs, rhs, "iadd").unwrap();
                self.box_native_int(result)
            }
            Sub => {
                let result = self.builder.build_int_sub(lhs, rhs, "isub").unwrap();
                self.box_native_int(result)
            }
            Mul => {
                let result = self.builder.build_int_mul(lhs, rhs, "imul").unwrap();
                self.box_native_int(result)
            }
            Div => {
                let result = self
                    .builder
                    .build_int_signed_div(lhs, rhs, "idiv")
                    .unwrap();
                self.box_native_int(result)
            }
            Mod => {
                let result = self
                    .builder
                    .build_int_signed_rem(lhs, rhs, "imod")
                    .unwrap();
                self.box_native_int(result)
            }
            Equals => self.wrap_bool(
                self.builder
                    .build_int_compare(IntPredicate::EQ, lhs, rhs, "ieq")
                    .unwrap(),
            ),
            NotEquals => self.wrap_bool(
                self.builder
                    .build_int_compare(IntPredicate::NE, lhs, rhs, "ine")
                    .unwrap(),
            ),
            Less => self.wrap_bool(
                self.builder
                    .build_int_compare(IntPredicate::SLT, lhs, rhs, "ilt")
                    .unwrap(),
            ),
            LessEq => self.wrap_bool(
                self.builder
                    .build_int_compare(IntPredicate::SLE, lhs, rhs, "ile")
                    .unwrap(),
            ),
            Greater => self.wrap_bool(
                self.builder
                    .build_int_compare(IntPredicate::SGT, lhs, rhs, "igt")
                    .unwrap(),
            ),
            GreaterEq => self.wrap_bool(
                self.builder
                    .build_int_compare(IntPredicate::SGE, lhs, rhs, "ige")
                    .unwrap(),
            ),
            _ => {
                // Fall back to boxed path for unsupported ops
                let lhs_boxed = self.box_native_int(lhs);
                let rhs_boxed = self.box_native_int(rhs);
                return self.emit_numeric_binary(op, lhs_boxed, rhs_boxed, true);
            }
        })
    }

    /// Emit an expression and return the result as a native i64 integer.
    /// Only valid when expr_is_int() returns true for this expression.
    fn emit_expression_as_native_int(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match expr {
            Expression::Integer(val, _) => {
                Ok(self.runtime.value_i64_type.const_int(*val as u64, true))
            }
            Expression::Identifier(name, _) => {
                if let Some(native) = self.load_variable_as_native_int(ctx, name) {
                    Ok(native)
                } else {
                    // Variable is boxed; unbox it
                    let boxed = self.load_variable(ctx, name)?;
                    Ok(self.unbox_to_native_int(boxed))
                }
            }
            Expression::Binary {
                op, left, right, ..
            } if matches!(
                op,
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
            ) && self.expr_is_int(ctx, left)
                && self.expr_is_int(ctx, right) =>
            {
                let lhs = self.emit_expression_as_native_int(ctx, left)?;
                let rhs = self.emit_expression_as_native_int(ctx, right)?;
                Ok(match op {
                    BinaryOp::Add => self.builder.build_int_add(lhs, rhs, "iadd").unwrap(),
                    BinaryOp::Sub => self.builder.build_int_sub(lhs, rhs, "isub").unwrap(),
                    BinaryOp::Mul => self.builder.build_int_mul(lhs, rhs, "imul").unwrap(),
                    BinaryOp::Div => self.builder.build_int_signed_div(lhs, rhs, "idiv").unwrap(),
                    BinaryOp::Mod => self.builder.build_int_signed_rem(lhs, rhs, "imod").unwrap(),
                    _ => unreachable!(),
                })
            }
            Expression::Unary {
                op: UnaryOp::Neg,
                expr: inner,
                ..
            } if self.expr_is_int(ctx, inner) => {
                let val = self.emit_expression_as_native_int(ctx, inner)?;
                Ok(self.builder.build_int_neg(val, "ineg").unwrap())
            }
            _ => {
                // Fallback: emit as boxed, then unbox
                let boxed = self.emit_expression(ctx, expr)?;
                Ok(self.unbox_to_native_int(boxed))
            }
        }
    }

    fn emit_logical_binary(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let saved_tail = ctx.in_tail_position;
        ctx.in_tail_position = false;

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
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(rhs_bb);
        let right_value = self.emit_expression(ctx, right)?;
        let right_bool = if right_is_bool {
            self.value_to_bool_fast(right_value)
        } else {
            self.value_to_bool(right_value)
        };

        let rhs_end_bb = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        let phi = self.builder.build_phi(self.bool_type, "logic_phi").unwrap();
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
        ctx.cse_cache.clear();
        let then_value = self.emit_expression(ctx, then_branch)?;
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        let then_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(else_bb);
        ctx.cse_cache.clear();
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
        // Handle unboxed variables: load native value and box it
        match ctx.unboxed_vars.get(name) {
            Some(UnboxedKind::NativeInt) => {
                if let Some(alloca) = ctx.variable_allocas.get(name) {
                    let native_val = self
                        .builder
                        .build_load(
                            self.runtime.value_i64_type,
                            *alloca,
                            &format!("load_int_{name}"),
                        )
                        .unwrap()
                        .into_int_value();
                    return Ok(self.box_native_int(native_val));
                }
            }
            Some(UnboxedKind::NativeFloat) => {
                if let Some(alloca) = ctx.variable_allocas.get(name) {
                    let native_val = self
                        .builder
                        .build_load(self.f64_type, *alloca, &format!("load_float_{name}"))
                        .unwrap()
                        .into_float_value();
                    return Ok(self.wrap_number_unchecked(native_val));
                }
            }
            _ => {}
        }

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

        if let Some((_, field_count)) = self.enum_constructors.get(name).cloned() {
            if field_count == 0 {
                return self.emit_enum_constructor_nullary(name);
            }
        }

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
        if let Some(alloca) = ctx.variable_allocas.get(name) {
            self.builder.build_store(*alloca, value).unwrap();
            return;
        }

        let entry_bb = ctx.function.get_first_basic_block().unwrap();
        let current_bb = self.builder.get_insert_block().unwrap();

        if let Some(first_instr) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_bb);
        }
        let alloca = self
            .builder
            .build_alloca(self.runtime.value_i64_type, &format!("{name}_slot"))
            .unwrap();

        self.builder.position_at_end(current_bb);
        self.builder.build_store(alloca, value).unwrap();
        ctx.variable_allocas.insert(name.to_string(), alloca);
    }

    /// Store a native i64 integer (not NaN-boxed) into a variable.
    fn store_variable_native_int(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        name: &str,
        value: IntValue<'ctx>,
    ) {
        if let Some(alloca) = ctx.variable_allocas.get(name) {
            self.builder.build_store(*alloca, value).unwrap();
            return;
        }

        let entry_bb = ctx.function.get_first_basic_block().unwrap();
        let current_bb = self.builder.get_insert_block().unwrap();

        if let Some(first_instr) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_bb);
        }
        let alloca = self
            .builder
            .build_alloca(self.runtime.value_i64_type, &format!("{name}_int_slot"))
            .unwrap();

        self.builder.position_at_end(current_bb);
        self.builder.build_store(alloca, value).unwrap();
        ctx.variable_allocas.insert(name.to_string(), alloca);
        ctx.unboxed_vars
            .insert(name.to_string(), UnboxedKind::NativeInt);
    }

    /// Store a native f64 float (not NaN-boxed) into a variable.
    fn store_variable_native_float(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        name: &str,
        value: FloatValue<'ctx>,
    ) {
        if let Some(alloca) = ctx.variable_allocas.get(name) {
            self.builder.build_store(*alloca, value).unwrap();
            return;
        }

        let entry_bb = ctx.function.get_first_basic_block().unwrap();
        let current_bb = self.builder.get_insert_block().unwrap();

        if let Some(first_instr) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_bb);
        }
        let alloca = self
            .builder
            .build_alloca(self.f64_type, &format!("{name}_float_slot"))
            .unwrap();

        self.builder.position_at_end(current_bb);
        self.builder.build_store(alloca, value).unwrap();
        ctx.variable_allocas.insert(name.to_string(), alloca);
        ctx.unboxed_vars
            .insert(name.to_string(), UnboxedKind::NativeFloat);
    }

    /// Load a variable, returning a NaN-boxed value. Handles unboxed vars by boxing them.
    fn load_variable_boxed(
        &mut self,
        ctx: &FunctionContext<'ctx>,
        name: &str,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match ctx.unboxed_vars.get(name) {
            Some(UnboxedKind::NativeInt) => {
                if let Some(alloca) = ctx.variable_allocas.get(name) {
                    let native_val = self
                        .builder
                        .build_load(
                            self.runtime.value_i64_type,
                            *alloca,
                            &format!("load_int_{name}"),
                        )
                        .unwrap()
                        .into_int_value();
                    Ok(self.box_native_int(native_val))
                } else {
                    self.load_variable(ctx, name)
                }
            }
            Some(UnboxedKind::NativeFloat) => {
                if let Some(alloca) = ctx.variable_allocas.get(name) {
                    let native_val = self
                        .builder
                        .build_load(self.f64_type, *alloca, &format!("load_float_{name}"))
                        .unwrap()
                        .into_float_value();
                    Ok(self.wrap_number_unchecked(native_val))
                } else {
                    self.load_variable(ctx, name)
                }
            }
            _ => self.load_variable(ctx, name),
        }
    }

    /// Load a native i64 value from an unboxed int variable, or unbox from NaN-boxed.
    fn load_variable_as_native_int(
        &mut self,
        ctx: &FunctionContext<'ctx>,
        name: &str,
    ) -> Option<IntValue<'ctx>> {
        match ctx.unboxed_vars.get(name) {
            Some(UnboxedKind::NativeInt) => {
                if let Some(alloca) = ctx.variable_allocas.get(name) {
                    Some(
                        self.builder
                            .build_load(
                                self.runtime.value_i64_type,
                                *alloca,
                                &format!("load_int_{name}"),
                            )
                            .unwrap()
                            .into_int_value(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Load a native f64 value from an unboxed float variable, or return None.
    fn load_variable_as_native_float(
        &mut self,
        ctx: &FunctionContext<'ctx>,
        name: &str,
    ) -> Option<FloatValue<'ctx>> {
        match ctx.unboxed_vars.get(name) {
            Some(UnboxedKind::NativeFloat) => {
                if let Some(alloca) = ctx.variable_allocas.get(name) {
                    Some(
                        self.builder
                            .build_load(self.f64_type, *alloca, &format!("load_float_{name}"))
                            .unwrap()
                            .into_float_value(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Convert a NaN-boxed integer value to native i64 (extract from f64 representation).
    fn unbox_to_native_int(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        // NaN-boxed numbers store the value as f64 bits in i64.
        // To get native int: bitcast i64 → f64, then fptosi f64 → i64
        let f64_val = self
            .builder
            .build_bitcast(value, self.f64_type, "unbox_f64")
            .unwrap()
            .into_float_value();
        self.builder
            .build_float_to_signed_int(f64_val, self.runtime.value_i64_type, "unbox_int")
            .unwrap()
    }

    /// Convert a native i64 integer to NaN-boxed representation.
    fn box_native_int(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        // Convert native i64 → f64 → NaN-box bits
        let f64_val = self
            .builder
            .build_signed_int_to_float(value, self.f64_type, "box_f64")
            .unwrap();
        self.wrap_number_unchecked(f64_val)
    }

    /// Check if a variable is an unboxed native int in the current function context.
    fn var_is_native_int(&self, ctx: &FunctionContext<'ctx>, name: &str) -> bool {
        ctx.unboxed_vars.get(name) == Some(&UnboxedKind::NativeInt)
    }

    /// Check if a variable is an unboxed native float in the current function context.
    fn var_is_native_float(&self, ctx: &FunctionContext<'ctx>, name: &str) -> bool {
        ctx.unboxed_vars.get(name) == Some(&UnboxedKind::NativeFloat)
    }

    /// Check if an expression is known to be statically typed as Int (for native i64 paths).
    fn expr_is_int(&self, ctx: &FunctionContext<'ctx>, expr: &Expression) -> bool {
        match expr {
            Expression::Integer(_, _) => true,
            Expression::Identifier(name, _) => self.var_is_native_int(ctx, name),
            Expression::Binary {
                op, left, right, ..
            } => {
                matches!(
                    op,
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
                ) && self.expr_is_int(ctx, left)
                    && self.expr_is_int(ctx, right)
            }
            Expression::Unary {
                op: UnaryOp::Neg,
                expr: inner,
                ..
            } => self.expr_is_int(ctx, inner),
            Expression::Call { callee, .. } => {
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    self.resolved_returns.get(name.as_str())
                        == Some(&TypeId::Primitive(Primitive::Int))
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Check if an expression is known to be statically typed as Float (for native f64 paths).
    fn expr_is_float(&self, ctx: &FunctionContext<'ctx>, expr: &Expression) -> bool {
        match expr {
            Expression::Float(_, _) => true,
            Expression::Identifier(name, _) => self.var_is_native_float(ctx, name),
            _ => false,
        }
    }

    fn wrap_number(&mut self, value: FloatValue<'ctx>) -> IntValue<'ctx> {
        self.call_nb(self.runtime.nb_make_number, &[value.into()], "nb_num")
    }

    /// Inline NaN-boxing for numbers: bitcast f64 → i64 with NaN canonicalization.
    /// Avoids the overhead of a runtime function call when we know we're working with numbers.
    fn wrap_number_fast(&mut self, value: FloatValue<'ctx>) -> IntValue<'ctx> {
        let bits = self
            .builder
            .build_bitcast(value, self.runtime.value_i64_type, "nb_bits")
            .unwrap()
            .into_int_value();
        let qnan_mask = self
            .runtime
            .value_i64_type
            .const_int(0xFFF8_0000_0000_0000, false);
        let qnan_prefix = self
            .runtime
            .value_i64_type
            .const_int(0x7FF8_0000_0000_0000, false);
        let masked = self
            .builder
            .build_and(bits, qnan_mask, "nan_check")
            .unwrap();
        let is_nan = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, masked, qnan_prefix, "is_nan")
            .unwrap();
        let canonical_nan = self
            .runtime
            .value_i64_type
            .const_int(0x7FFB_8000_0000_0000, false);
        self.builder
            .build_select(is_nan, canonical_nan, bits, "nb_num_fast")
            .unwrap()
            .into_int_value()
    }

    /// Inline NaN-boxing without NaN canonicalization.
    /// Use ONLY when the result is guaranteed non-NaN (e.g., integer add/sub/mul,
    /// or operations on finite inputs that cannot produce NaN).
    fn wrap_number_unchecked(&mut self, value: FloatValue<'ctx>) -> IntValue<'ctx> {
        self.builder
            .build_bitcast(value, self.runtime.value_i64_type, "nb_bits_uc")
            .unwrap()
            .into_int_value()
    }

    /// Inline unboxing for known-numeric values: direct bitcast i64 → f64.
    /// Only safe when the value is guaranteed to be a number (not a tagged value).
    fn value_to_number_fast(&mut self, value: IntValue<'ctx>) -> FloatValue<'ctx> {
        self.builder
            .build_bitcast(value, self.f64_type, "nb_f64_fast")
            .unwrap()
            .into_float_value()
    }

    fn wrap_bool(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        let false_bits = self
            .runtime
            .value_i64_type
            .const_int(0x7FF9_0000_0000_0000, false);
        let true_bits = self
            .runtime
            .value_i64_type
            .const_int(0x7FF9_0000_0000_0001, false);
        let val_i1 = self
            .builder
            .build_int_truncate(value, self.context.bool_type(), "bool_trunc")
            .unwrap();
        self.builder
            .build_select(val_i1, true_bits, false_bits, "nb_bool_fast")
            .unwrap()
            .into_int_value()
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

    fn build_bytes_from_global(&mut self, global: GlobalValue<'ctx>, len: usize) -> IntValue<'ctx> {
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        let data_ptr = self
            .builder
            .build_pointer_cast(global.as_pointer_value(), i8_ptr_type, "bytes_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(len as u64, false);

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

    fn wrap_none(&mut self) -> IntValue<'ctx> {
        self.call_nb(self.runtime.nb_make_none, &[], "nb_none")
    }

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
        // Get or create a global i64 cache for this string literal's NaN-boxed value.
        // String NaN-boxed values always have QNAN bits set (never 0), so 0 = uninitialized.
        let cache_global = if let Some(gv) = self.string_nb_cache.get(literal) {
            *gv
        } else {
            let name = format!("nb_str_cache_{}", self.string_nb_cache.len());
            let gv = self
                .module
                .add_global(self.runtime.value_i64_type, None, &name);
            gv.set_initializer(&self.runtime.value_i64_type.const_zero());
            self.string_nb_cache.insert(literal.to_string(), gv);
            gv
        };

        let cached = self
            .builder
            .build_load(
                self.runtime.value_i64_type,
                cache_global.as_pointer_value(),
                "cached_str",
            )
            .unwrap()
            .into_int_value();

        let is_init = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                cached,
                self.runtime.value_i64_type.const_zero(),
                "str_cached",
            )
            .unwrap();

        let current_bb = self.builder.get_insert_block().unwrap();
        let func = current_bb.get_parent().unwrap();
        let init_bb = self.context.append_basic_block(func, "str_init");
        let use_bb = self.context.append_basic_block(func, "str_use");

        self.builder
            .build_conditional_branch(is_init, use_bb, init_bb)
            .unwrap();

        // Init block: create the string and cache the NaN-boxed value
        self.builder.position_at_end(init_bb);
        let global = self.get_or_create_string_constant(literal);
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        let cast_ptr = self
            .builder
            .build_pointer_cast(global.as_pointer_value(), i8_ptr_type, "str_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(literal.len() as u64, false);
        let args = &[cast_ptr.into(), len_value.into()];
        let new_str = self.call_nb(self.runtime.nb_make_string, args, "nb_str");
        self.builder
            .build_store(cache_global.as_pointer_value(), new_str)
            .unwrap();
        self.builder
            .build_unconditional_branch(use_bb)
            .unwrap();

        // Use block: phi selects cached or newly created
        self.builder.position_at_end(use_bb);
        let phi = self
            .builder
            .build_phi(self.runtime.value_i64_type, "str_val")
            .unwrap();
        phi.add_incoming(&[(&cached, current_bb), (&new_str, init_bb)]);
        phi.as_basic_value().into_int_value()
    }

    fn call_nb(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) -> IntValue<'ctx> {
        self.builder
            .build_call(function, args, name)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value()
    }

    fn call_nb_void(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) {
        self.builder.build_call(function, args, "").unwrap();
    }

    fn nb_to_ptr(&mut self, value: IntValue<'ctx>) -> PointerValue<'ctx> {
        self.call_runtime_ptr(self.runtime.nb_to_handle, &[value.into()], "nb_to_ptr")
    }

    fn ptr_to_nb(&mut self, ptr: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.call_nb(self.runtime.nb_from_handle, &[ptr.into()], "ptr_to_nb")
    }

    /// Inline check: is this NaN-boxed value a heap pointer?
    /// Returns an i1 boolean.  Pure bit-ops, zero allocations.
    fn nb_is_heap_ptr(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        let i64_ty = self.runtime.value_i64_type;
        let qnan_mask = i64_ty.const_int(0xFFF8_0000_0000_0000, false);
        let qnan_prefix = i64_ty.const_int(0x7FF8_0000_0000_0000, false);
        let tag_mask = i64_ty.const_int(0x0007_0000_0000_0000, false);
        let zero = i64_ty.const_int(0, false);

        // (value & QNAN_MASK) == QNAN_PREFIX
        let masked = self.builder.build_and(value, qnan_mask, "hp_qnan").unwrap();
        let is_tagged = self.builder.build_int_compare(
            IntPredicate::EQ, masked, qnan_prefix, "hp_is_tagged",
        ).unwrap();

        // (value & TAG_MASK) == 0  (TAG_HEAP = 0)
        let tag_bits = self.builder.build_and(value, tag_mask, "hp_tag").unwrap();
        let is_heap_tag = self.builder.build_int_compare(
            IntPredicate::EQ, tag_bits, zero, "hp_is_heap_tag",
        ).unwrap();

        // value != QNAN_PREFIX  (bare prefix is not a valid heap ptr)
        let not_bare = self.builder.build_int_compare(
            IntPredicate::NE, value, qnan_prefix, "hp_not_bare",
        ).unwrap();

        let c1 = self.builder.build_and(is_tagged, is_heap_tag, "hp_c1").unwrap();
        self.builder.build_and(c1, not_bare, "is_heap_ptr").unwrap()
    }

    /// Extract the raw pointer from a NaN-boxed heap value (no allocation).
    /// Caller must ensure the value IS a heap pointer (use nb_is_heap_ptr first).
    fn nb_extract_heap_ptr(&mut self, value: IntValue<'ctx>) -> PointerValue<'ctx> {
        let payload_mask = self.runtime.value_i64_type.const_int(0x0000_FFFF_FFFF_FFFF, false);
        let addr = self.builder.build_and(value, payload_mask, "heap_addr").unwrap();
        self.builder
            .build_int_to_ptr(addr, self.i8_type.ptr_type(AddressSpace::default()), "heap_ptr")
            .unwrap()
    }

    /// Inline list element access: given a Value* (list), load the items data pointer.
    /// Layout: Value.payload.ptr (offset 16) → ListObject → items.ptr (offset 8)
    fn inline_list_items_ptr(&mut self, list_ptr: PointerValue<'ctx>) -> PointerValue<'ctx> {
        let ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        // Load payload.ptr at offset 16 from Value → ListObject*
        let payload_ptr_addr = unsafe {
            self.builder.build_gep(self.i8_type, list_ptr,
                &[self.usize_type.const_int(16, false)], "payload_ptr_addr").unwrap()
        };
        let list_obj = self.builder.build_load(ptr_type, payload_ptr_addr, "list_obj").unwrap().into_pointer_value();
        // Load items.ptr at offset 8 from ListObject → *mut ValueHandle
        let items_ptr_addr = unsafe {
            self.builder.build_gep(self.i8_type, list_obj,
                &[self.usize_type.const_int(8, false)], "items_ptr_addr").unwrap()
        };
        self.builder.build_load(ptr_type, items_ptr_addr, "items_data").unwrap().into_pointer_value()
    }

    /// Inline list element access: given items data ptr and index, load the element as *mut Value.
    fn inline_list_get_ptr(&mut self, items_data: PointerValue<'ctx>, index: IntValue<'ctx>) -> PointerValue<'ctx> {
        let ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        // items_data[index] — each element is 8 bytes (pointer)
        let byte_offset = self.builder.build_int_mul(index, self.usize_type.const_int(8, false), "elem_byte_off").unwrap();
        let elem_addr = unsafe {
            self.builder.build_gep(self.i8_type, items_data,
                &[byte_offset], "elem_addr").unwrap()
        };
        self.builder.build_load(ptr_type, elem_addr, "elem_ptr").unwrap().into_pointer_value()
    }

    /// Inline: given a Value* known to contain a number, load payload.number as f64.
    fn inline_load_number(&mut self, value_ptr: PointerValue<'ctx>) -> FloatValue<'ctx> {
        // payload.number is at offset 16 from Value*
        let payload_addr = unsafe {
            self.builder.build_gep(self.i8_type, value_ptr,
                &[self.usize_type.const_int(16, false)], "num_payload_addr").unwrap()
        };
        self.builder.build_load(self.f64_type, payload_addr, "raw_num").unwrap().into_float_value()
    }

    /// Inline: given a Value* known to contain a number, store a new f64 to payload.number.
    /// This overwrites the number in-place without allocating a new Value.
    fn inline_store_number(&mut self, value_ptr: PointerValue<'ctx>, number: FloatValue<'ctx>) {
        let payload_addr = unsafe {
            self.builder.build_gep(self.i8_type, value_ptr,
                &[self.usize_type.const_int(16, false)], "num_payload_wr_addr").unwrap()
        };
        self.builder.build_store(payload_addr, number).unwrap();
    }

    /// Inline struct field GET: extracts heap ptr, chases through StructObject fields,
    /// and loads the number directly. Falls back to FFI for non-numeric fields.
    fn inline_struct_get_nb(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        store_nb: IntValue<'ctx>,
        field_idx: u64,
    ) -> IntValue<'ctx> {
        let heap_ptr = self.nb_extract_heap_ptr(store_nb);
        let fields_data = self.inline_list_items_ptr(heap_ptr);
        let idx_val = self.usize_type.const_int(field_idx, false);
        let field_handle = self.inline_list_get_ptr(fields_data, idx_val);

        // Check tag byte — only inline for Number (tag = 0)
        let tag = self.nb_heap_tag(field_handle);
        let is_number = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, tag,
            self.i8_type.const_int(0, false), "is_num_field",
        ).unwrap();

        let current_fn = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let fast_bb = self.context.append_basic_block(current_fn, "struct_get_fast");
        let slow_bb = self.context.append_basic_block(current_fn, "struct_get_slow");
        let merge_bb = self.context.append_basic_block(current_fn, "struct_get_merge");

        self.builder.build_conditional_branch(is_number, fast_bb, slow_bb).unwrap();

        // Fast path: load number directly
        self.builder.position_at_end(fast_bb);
        let num = self.inline_load_number(field_handle);
        let fast_result = self.builder.build_bitcast(num, self.runtime.value_i64_type, "fast_nb").unwrap().into_int_value();
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        // Slow path: FFI call
        self.builder.position_at_end(slow_bb);
        let slow_result = self.call_nb(
            self.runtime.nb_struct_get,
            &[store_nb.into(), idx_val.into()],
            "slow_struct_get",
        );
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        // Merge
        self.builder.position_at_end(merge_bb);
        let phi = self.builder.build_phi(self.runtime.value_i64_type, "struct_field_val").unwrap();
        phi.add_incoming(&[(&fast_result, fast_bb), (&slow_result, slow_bb)]);
        phi.as_basic_value().into_int_value()
    }

    /// Inline struct field SET for numeric values: writes f64 directly to the
    /// existing field Value's payload, avoiding allocation/deallocation.
    /// Falls back to FFI for non-numeric current field values.
    fn inline_struct_set_nb(
        &mut self,
        store_nb: IntValue<'ctx>,
        field_idx: u64,
        value_nb: IntValue<'ctx>,
    ) {
        let heap_ptr = self.nb_extract_heap_ptr(store_nb);
        let fields_data = self.inline_list_items_ptr(heap_ptr);
        let idx_val = self.usize_type.const_int(field_idx, false);
        let field_handle = self.inline_list_get_ptr(fields_data, idx_val);

        // Check if the existing field is a Number (tag=0) — safe to overwrite in-place
        let tag = self.nb_heap_tag(field_handle);
        let is_number = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, tag,
            self.i8_type.const_int(0, false), "is_num_field_set",
        ).unwrap();

        let current_fn = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let fast_bb = self.context.append_basic_block(current_fn, "struct_set_fast");
        let slow_bb = self.context.append_basic_block(current_fn, "struct_set_slow");
        let merge_bb = self.context.append_basic_block(current_fn, "struct_set_merge");

        self.builder.build_conditional_branch(is_number, fast_bb, slow_bb).unwrap();

        // Fast path: write number directly to existing Value payload
        self.builder.position_at_end(fast_bb);
        let new_f64 = self.builder.build_bitcast(value_nb, self.f64_type, "new_f64").unwrap().into_float_value();
        self.inline_store_number(field_handle, new_f64);
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        // Slow path: FFI call
        self.builder.position_at_end(slow_bb);
        self.builder.build_call(
            self.runtime.nb_struct_set,
            &[store_nb.into(), idx_val.into(), value_nb.into()],
            "slow_struct_set",
        ).unwrap();
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        self.builder.position_at_end(merge_bb);
    }

    /// Read the ValueTag byte (offset 0) from a Value struct pointer.
    fn nb_heap_tag(&mut self, ptr: PointerValue<'ctx>) -> IntValue<'ctx> {
        self.builder.build_load(self.i8_type, ptr, "vtag").unwrap().into_int_value()
    }

    fn call_bridged(
        &mut self,
        function: FunctionValue<'ctx>,
        nb_args: &[IntValue<'ctx>],
        name: &str,
    ) -> IntValue<'ctx> {
        let ptr_args: Vec<BasicMetadataValueEnum<'ctx>> =
            nb_args.iter().map(|v| self.nb_to_ptr(*v).into()).collect();
        let ptr_result = self.call_runtime_ptr(function, &ptr_args, name);
        self.ptr_to_nb(ptr_result)
    }

    fn call_runtime_ptr(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
        name: &str,
    ) -> PointerValue<'ctx> {
        self.builder
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
        self.builder
            .build_call(self.runtime.nb_as_number, &[value.into()], "nb_as_number")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_float_value()
    }

    /// Emit an LLVM intrinsic call for a unary f64→f64 math function.
    /// Unboxes the argument, calls the intrinsic, and reboxes the result.
    /// `can_produce_nan`: whether the operation might produce NaN (e.g., sqrt of negative).
    fn emit_math_intrinsic_unary(
        &mut self,
        arg: IntValue<'ctx>,
        intrinsic_name: &str,
        call_name: &str,
        can_produce_nan: bool,
    ) -> IntValue<'ctx> {
        use inkwell::intrinsics::Intrinsic;

        let f64_type: BasicTypeEnum = self.f64_type.into();
        let intrinsic = Intrinsic::find(intrinsic_name).expect("LLVM intrinsic not found");
        let decl = intrinsic
            .get_declaration(&self.module, &[f64_type])
            .expect("failed to get intrinsic declaration");

        let num = self.value_to_number_fast(arg);
        let result = self
            .builder
            .build_call(decl, &[num.into()], call_name)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_float_value();

        if can_produce_nan {
            self.wrap_number_fast(result)
        } else {
            self.wrap_number_unchecked(result)
        }
    }

    /// Emit an LLVM intrinsic call for a binary (f64, f64)→f64 math function.
    fn emit_math_intrinsic_binary(
        &mut self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        intrinsic_name: &str,
        call_name: &str,
        can_produce_nan: bool,
    ) -> IntValue<'ctx> {
        use inkwell::intrinsics::Intrinsic;

        let f64_type: BasicTypeEnum = self.f64_type.into();
        let intrinsic = Intrinsic::find(intrinsic_name).expect("LLVM intrinsic not found");
        let decl = intrinsic
            .get_declaration(&self.module, &[f64_type])
            .expect("failed to get intrinsic declaration");

        let lhs_num = self.value_to_number_fast(lhs);
        let rhs_num = self.value_to_number_fast(rhs);
        let result = self
            .builder
            .build_call(decl, &[lhs_num.into(), rhs_num.into()], call_name)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_float_value();

        if can_produce_nan {
            self.wrap_number_fast(result)
        } else {
            self.wrap_number_unchecked(result)
        }
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
            true,
            false,
            Some(InlineAsmDialect::ATT),
            false,
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
                format!(
                    "extern argument type not supported: `{}`",
                    self.format_type_enum(target)
                ),
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
                        .map_err(|e| {
                            Diagnostic::new(format!("int->float cast failed: {e}"), span)
                        })?;
                    Ok(self.wrap_number(as_float))
                }
            }
            _ => Err(Diagnostic::new(
                format!(
                    "extern return type not supported: `{}`",
                    self.format_type_enum(ret_ty)
                ),
                span,
            )),
        }
    }

    fn contains_placeholder(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Placeholder(_, _) => true,
            Expression::Binary { left, right, .. } => {
                self.contains_placeholder(left) || self.contains_placeholder(right)
            }
            Expression::Unary { expr, .. } => self.contains_placeholder(expr),
            Expression::Call { callee, args, .. } => {
                self.contains_placeholder(callee)
                    || args.iter().any(|a| self.contains_placeholder(a))
            }
            Expression::List(items, _) => items.iter().any(|i| self.contains_placeholder(i)),
            Expression::Map(entries, _) => entries
                .iter()
                .any(|(k, v)| self.contains_placeholder(k) || self.contains_placeholder(v)),
            Expression::Member { target, .. } => self.contains_placeholder(target),
            Expression::Index { target, index, .. } => {
                self.contains_placeholder(target) || self.contains_placeholder(index)
            }
            Expression::Slice {
                target, start, end, ..
            } => {
                self.contains_placeholder(target)
                    || self.contains_placeholder(start)
                    || self.contains_placeholder(end)
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.contains_placeholder(condition)
                    || self.contains_placeholder(then_branch)
                    || self.contains_placeholder(else_branch)
            }
            Expression::Lambda { body, .. } => {
                body.statements.iter().any(|s| match s {
                    Statement::Binding(b) => self.contains_placeholder(&b.value),
                    Statement::Expression(e) => self.contains_placeholder(e),
                    Statement::Return(e, _) => self.contains_placeholder(e),
                    Statement::If {
                        condition,
                        body,
                        elif_branches,
                        else_body,
                        ..
                    } => {
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
                    Statement::While {
                        condition, body, ..
                    } => {
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
                    Statement::ForRange {
                        start,
                        end,
                        step,
                        body,
                        ..
                    } => {
                        self.contains_placeholder(start)
                            || self.contains_placeholder(end)
                            || step
                                .as_ref()
                                .map_or(false, |s| self.contains_placeholder(s))
                            || body.statements.iter().any(|s2| match s2 {
                                Statement::Expression(e) => self.contains_placeholder(e),
                                _ => false,
                            })
                    }
                    Statement::Break(_) | Statement::Continue(_) => false,
                    Statement::FieldAssign { value, .. } => self.contains_placeholder(value),
                    Statement::PatternBinding { value, .. } => self.contains_placeholder(value),
                }) || body
                    .value
                    .as_ref()
                    .map_or(false, |v| self.contains_placeholder(v))
            }
            _ => false,
        }
    }

    fn replace_placeholder_with(&self, expr: &Expression, replacement: &Expression) -> Expression {
        match expr {
            Expression::Placeholder(_, _) => replacement.clone(),
            Expression::Binary {
                op,
                left,
                right,
                span,
            } => Expression::Binary {
                op: *op,
                left: Box::new(self.replace_placeholder_with(left, replacement)),
                right: Box::new(self.replace_placeholder_with(right, replacement)),
                span: *span,
            },
            Expression::Unary {
                op,
                expr: inner,
                span,
            } => Expression::Unary {
                op: *op,
                expr: Box::new(self.replace_placeholder_with(inner, replacement)),
                span: *span,
            },
            Expression::Call {
                callee, args, span, ..
            } => Expression::Call {
                callee: Box::new(self.replace_placeholder_with(callee, replacement)),
                args: args
                    .iter()
                    .map(|a| self.replace_placeholder_with(a, replacement))
                    .collect(),
                arg_names: vec![],
                span: *span,
            },
            Expression::List(items, span) => Expression::List(
                items
                    .iter()
                    .map(|i| self.replace_placeholder_with(i, replacement))
                    .collect(),
                *span,
            ),
            Expression::Map(entries, span) => Expression::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        (
                            self.replace_placeholder_with(k, replacement),
                            self.replace_placeholder_with(v, replacement),
                        )
                    })
                    .collect(),
                *span,
            ),
            Expression::Member {
                target,
                property,
                span,
            } => Expression::Member {
                target: Box::new(self.replace_placeholder_with(target, replacement)),
                property: property.clone(),
                span: *span,
            },
            Expression::Index {
                target,
                index,
                span,
            } => Expression::Index {
                target: Box::new(self.replace_placeholder_with(target, replacement)),
                index: Box::new(self.replace_placeholder_with(index, replacement)),
                span: *span,
            },
            Expression::Slice {
                target,
                start,
                end,
                span,
            } => Expression::Slice {
                target: Box::new(self.replace_placeholder_with(target, replacement)),
                start: Box::new(self.replace_placeholder_with(start, replacement)),
                end: Box::new(self.replace_placeholder_with(end, replacement)),
                span: *span,
            },
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                span,
            } => Expression::Ternary {
                condition: Box::new(self.replace_placeholder_with(condition, replacement)),
                then_branch: Box::new(self.replace_placeholder_with(then_branch, replacement)),
                else_branch: Box::new(self.replace_placeholder_with(else_branch, replacement)),
                span: *span,
            },

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
            self.builder
                .build_call(init_fn, &[], "init_globals")
                .unwrap();
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
        let init_fn = self.module.add_function(
            "__coral_init_globals",
            self.context.void_type().fn_type(&[], false),
            None,
        );
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
            lambda_out_param: None,
            unboxed_vars: HashMap::new(),
            non_escaping_locals: HashSet::new(),
            specialized_return_type: None,
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

    variable_allocas: HashMap<String, PointerValue<'ctx>>,
    function: FunctionValue<'ctx>,

    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,

    di_scope: Option<DIScope<'ctx>>,

    fn_name: String,

    in_tail_position: bool,

    cse_cache: HashMap<String, IntValue<'ctx>>,

    lambda_out_param: Option<PointerValue<'ctx>>,

    /// Tracks which local variables are stored in native (unboxed) representation
    unboxed_vars: HashMap<String, UnboxedKind>,

    /// Tracks non-escaping local variable names (retain/release can be elided)
    non_escaping_locals: HashSet<String>,

    /// If set, the function is a monomorphized specialized variant.
    /// Return statements must convert the NaN-boxed value to this native type.
    specialized_return_type: Option<TypeId>,
}
