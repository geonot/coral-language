//! LLVM code generation for Coral programs.
//!
//! This module transforms semantic models into LLVM IR using inkwell bindings.

mod runtime;

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
use crate::span::Span;
use crate::types::AllocationStrategy;
use inkwell::InlineAsmDialect;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
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
        }
    }

    pub fn with_inline_asm_mode(mut self, mode: InlineAsmMode) -> Self {
        self.inline_asm_mode = mode;
        self
    }
    pub fn compile(mut self, model: &SemanticModel) -> Result<Module<'ctx>, Diagnostic> {
        self.allocation_hints = model.allocation.symbols.clone();
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
            let llvm_name = if function.name == "main" {
                "__user_main"
            } else {
                &function.name
            };
            let fn_type = self.runtime.value_ptr_type.fn_type(
                &vec![self.runtime.value_ptr_type.into(); function.params.len()],
                false,
            );
            let llvm_fn = self.module.add_function(llvm_name, fn_type, None);
            self.functions.insert(function.name.clone(), llvm_fn);
        }
        // Handle stores and actors
        for store in &model.stores {
            // Track reference fields for this store
            for field in &store.fields {
                if field.is_reference {
                    self.reference_fields.insert((store.name.clone(), field.name.clone()));
                }
            }
            
            // All stores get a constructor that returns a Map with fields
            let constructor_name = format!("make_{}", store.name);
            let ctor_type = self.runtime.value_ptr_type.fn_type(&[], false);
            let ctor_fn = self.module.add_function(&constructor_name, ctor_type, None);
            self.functions.insert(constructor_name.clone(), ctor_fn);
            self.store_constructors.insert(constructor_name);
            
            if store.is_actor {
                // Declare message handler functions for each @method
                // Actor methods take state (ValuePtr) as hidden first param, plus user params (ValuePtr)
                for method in &store.methods {
                    if method.kind == FunctionKind::ActorMessage {
                        let mangled = format!("{}_{}", store.name, method.name);
                        // Hidden first param: state pointer (ValuePtr), user params also ValuePtr
                        let mut param_types: Vec<BasicMetadataTypeEnum> = 
                            vec![self.runtime.value_ptr_type.into()];
                        for _ in 0..method.params.len() {
                            param_types.push(self.runtime.value_ptr_type.into());
                        }
                        // Return ValuePtr
                        let fn_type = self.runtime.value_ptr_type.fn_type(&param_types, false);
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
                            vec![self.runtime.value_ptr_type.into()];
                        // For alpha, all method params are CoralValue* pointers (not f64)
                        // This allows passing stores, lists, and other values without corruption
                        for _ in 0..method.params.len() {
                            param_types.push(self.runtime.value_ptr_type.into());
                        }
                        // Return ptr (CoralValue*) instead of f64 to avoid corruption
                        let fn_type = self.runtime.value_ptr_type.fn_type(&param_types, false);
                        let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                        self.functions.insert(mangled.clone(), llvm_fn);
                        // Track store methods for dynamic dispatch
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
                        vec![self.runtime.value_ptr_type.into()]; // self
                    for _ in 0..method.params.len() {
                        param_types.push(self.runtime.value_ptr_type.into());
                    }
                    let fn_type = self.runtime.value_ptr_type.fn_type(&param_types, false);
                    let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                    self.functions.insert(mangled.clone(), llvm_fn);
                    self.store_methods.insert(method.name.clone(), (type_def.name.clone(), method.params.len()));
                }
            }
        }
        
        self.build_global_initializer(&model.globals)?;
        
        for function in &model.functions {
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
            let _ = self.call_runtime_ptr(self.runtime.actor_send, &[actor.into(), unit.into()], "send_unit");
        }
        self.builder.build_return(Some(&self.context.i32_type().const_int(0, false))).unwrap();

        Ok(self.module)
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
        let entry = self.context.append_basic_block(llvm_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: llvm_fn,
            loop_stack: Vec::new(),
        };

        // Parameters are Value* pointers - use them directly without wrapping
        for (param, param_ast) in llvm_fn
            .get_param_iter()
            .zip(function.params.iter())
        {
            // Parameter is already a Value* pointer
            let value_ptr = param.into_pointer_value();
            self.store_variable(&mut ctx, &param_ast.name, value_ptr);
        }

        let block_value = self.emit_block(&mut ctx, &function.body)?;
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
        };

        // First param is the state pointer (ValuePtr), inject as `self`
        let state_ptr = llvm_fn.get_nth_param(0).unwrap().into_pointer_value();
        // Store state directly as `self` - it's already a ValuePtr to the state Map
        self.store_variable(&mut ctx, "self", state_ptr);

        // Remaining params are user params (starting at index 1) - now Value* pointers
        for (i, param_ast) in function.params.iter().enumerate() {
            let param = llvm_fn.get_nth_param((i + 1) as u32).unwrap();
            // Parameter is already a Value* pointer
            let value_ptr = param.into_pointer_value();
            self.store_variable(&mut ctx, &param_ast.name, value_ptr);
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
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
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
                }
                Statement::Expression(expr) => {
                    let _ = self.emit_expression(ctx, expr)?;
                }
                Statement::Return(expr, _) => {
                    let value = self.emit_expression(ctx, expr)?;
                    // Return Value* pointer directly — functions return Value*, not f64
                    self.builder.build_return(Some(&value)).unwrap();
                    // Return a null sentinel without emitting any LLVM instruction.
                    // const_null() is a compile-time constant, so no instruction is added
                    // after the `ret` terminator. This means get_terminator() correctly
                    // identifies this block as terminated, and PHI/branch logic skips it.
                    return Ok(self.runtime.value_ptr_type.const_null());
                }
                Statement::If { condition, body, elif_branches, else_body, .. } => {
                    let function = ctx.function;
                    let cond_value = self.emit_expression(ctx, condition)?;
                    let cond_bool = self.value_to_bool(cond_value);

                    let then_bb = self.context.append_basic_block(function, "if_then");
                    let merge_bb = self.context.append_basic_block(function, "if_merge");

                    // Track (value, source_block) pairs for PHI node
                    let mut phi_incoming: Vec<(PointerValue<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();

                    // Determine initial else target
                    let first_else_bb = if elif_branches.is_empty() && else_body.is_none() {
                        merge_bb
                    } else {
                        self.context.append_basic_block(function, "if_else")
                    };
                    self.builder.build_conditional_branch(cond_bool, then_bb, first_else_bb).unwrap();

                    // Emit then body
                    self.builder.position_at_end(then_bb);
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
                            let elif_cond_val = self.emit_expression(ctx, elif_cond)?;
                            let elif_cond_bool = self.value_to_bool(elif_cond_val);

                            let elif_then_bb = self.context.append_basic_block(function, &format!("elif_then_{i}"));
                            let next_else_bb = if i + 1 < elif_branches.len() || else_body.is_some() {
                                self.context.append_basic_block(function, &format!("elif_else_{i}"))
                            } else {
                                merge_bb
                            };
                            self.builder.build_conditional_branch(elif_cond_bool, elif_then_bb, next_else_bb).unwrap();

                            self.builder.position_at_end(elif_then_bb);
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
                            .build_phi(self.runtime.value_ptr_type, "if_phi")
                            .unwrap();
                        for (val, bb) in &phi_incoming {
                            phi.add_incoming(&[(val as &dyn BasicValue<'ctx>, *bb)]);
                        }
                        // Store the if-expression result as __if_result for potential use
                        let if_result = phi.as_basic_value().into_pointer_value();
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
                    let cond_value = self.emit_expression(ctx, condition)?;
                    let cond_bool = self.value_to_bool(cond_value);
                    self.builder.build_conditional_branch(cond_bool, loop_body, loop_exit).unwrap();

                    // Body
                    self.builder.position_at_end(loop_body);
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
                    let iter = self.call_runtime_ptr(
                        self.runtime.value_iter,
                        &[iter_value.into()],
                        "for_iter",
                    );

                    let loop_header = self.context.append_basic_block(function, "for_cond");
                    let loop_body = self.context.append_basic_block(function, "for_body");
                    let loop_exit = self.context.append_basic_block(function, "for_exit");

                    self.builder.build_unconditional_branch(loop_header).unwrap();

                    // Get next element and check if iteration is done (Unit tag == 7)
                    self.builder.position_at_end(loop_header);
                    let elem = self.call_runtime_ptr(
                        self.runtime.value_iter_next,
                        &[iter.into()],
                        "for_next",
                    );
                    // Read the tag byte at offset 0 of the Value struct
                    let tag_ptr = self.builder.build_pointer_cast(
                        elem,
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

                    // Body: bind loop variable
                    self.builder.position_at_end(loop_body);
                    self.store_variable(ctx, variable, elem);
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
                Statement::FieldAssign { target, field, value, .. } => {
                    // self.field is value → coral_map_set(self, "field", value)
                    let target_value = self.emit_expression(ctx, &target)?;
                    let key_value = self.emit_string_literal(&field);
                    let new_value = self.emit_expression(ctx, &value)?;
                    
                    // Handle reference field retain/release for proper refcounting
                    if let Expression::Identifier(name, _) = &target {
                        if name == "self" {
                            let is_ref = self.reference_fields.iter().any(|(_, f)| f == field.as_str());
                            if is_ref {
                                // Release old value before setting new one
                                let old_value = self.call_runtime_ptr(
                                    self.runtime.map_get,
                                    &[target_value.into(), key_value.into()],
                                    "old_field_value",
                                );
                                self.call_runtime_void(self.runtime.value_release, &[old_value.into()], "release_old");
                                self.call_runtime_void(self.runtime.value_retain, &[new_value.into()], "retain_new");
                            }
                        }
                    }
                    
                    self.call_runtime_ptr(
                        self.runtime.map_set,
                        &[target_value.into(), key_value.into(), new_value.into()],
                        "map_set_field",
                    );
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
            }
        }

        if let Some(expr) = &block.value {
            self.emit_expression(ctx, expr.as_ref())
        } else {
            Ok(self.wrap_number(self.f64_type.const_float(0.0)))
        }
    }

    fn emit_expression(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        expr: &Expression,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
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
                    let lhs = self.emit_expression(ctx, left)?;
                    let rhs = self.emit_expression(ctx, right)?;
                    self.emit_numeric_binary(*op, lhs, rhs)
                }
            },
            Expression::Unary { op, expr, .. } => {
                let value = self.emit_expression(ctx, expr)?;
                match op {
                    UnaryOp::Neg => {
                        let as_number = self.value_to_number(value);
                        let neg = self.builder.build_float_neg(as_number, "neg").unwrap();
                        Ok(self.wrap_number(neg))
                    }
                    UnaryOp::Not => {
                        let predicate = self.value_to_bool(value);
                        let inverted = self.builder.build_not(predicate, "not").unwrap();
                        Ok(self.wrap_bool(inverted))
                    }
                    UnaryOp::BitNot => Ok(self.call_runtime_ptr(
                        self.runtime.value_bitnot,
                        &[value.into()],
                        "bitnot",
                    )),
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
                            let value = self.emit_expression(ctx, arg)?;
                            arg_values.push(value);
                        }
                        let metadata_args: Vec<BasicMetadataValueEnum> =
                            arg_values.iter().map(|v| (*v).into()).collect();
                        let call = self
                            .builder
                            .build_call(function, &metadata_args, "call")
                            .unwrap();
                        // Return is Value* pointer
                        let value = call
                            .try_as_basic_value()
                            .left()
                            .ok_or_else(|| Diagnostic::new("call produced no value", expr.span()))?
                            .into_pointer_value();
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
                // For alpha: desugar `x[i]` to coral_list_get (handles numeric indices
                // on lists). Map subscript `m[key]` also works since coral_list_get
                // will return unit for non-list targets — a proper coral_subscript
                // dispatcher can be added later.
                Ok(self.call_runtime_ptr(
                    self.runtime.list_get,
                    &[target_val.into(), index_val.into()],
                    "subscript",
                ))
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.emit_ternary(ctx, condition, then_branch, else_branch),
            Expression::Match(match_expr) => self.emit_match(ctx, match_expr),
            Expression::Unit => Ok(self.wrap_unit()),
            Expression::None(_) => Ok(self.wrap_unit()),
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
                Ok(self.call_runtime_ptr(
                    self.runtime.make_error,
                    &[error_code.into(), name_ptr.into(), name_len.into()],
                    "make_error",
                ))
            }
            Expression::ErrorPropagate { expr, span: _ } => {
                // Error propagation: `expr ! return err`
                // 1. Evaluate the expression
                // 2. Check if it's an error
                // 3. If error, return it from the current function
                // 4. Otherwise, continue with the value
                
                let value = self.emit_expression(ctx, expr)?;
                
                // Call coral_is_err to check if value is an error (returns i8)
                let is_err = self.builder
                    .build_call(self.runtime.is_err, &[value.into()], "is_err_check")
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
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        self.emit_list_literal_hinted(ctx, elements, None)
    }

    fn emit_list_literal_hinted(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        elements: &[Expression],
        hint: Option<i8>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
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
        let element_type = self.runtime.value_ptr_type;
        let array_type = element_type.array_type(values.len() as u32);
        let mut temp_array = array_type.get_undef();
        for (index, value) in values.iter().enumerate() {
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
        let len_value = self.usize_type.const_int(values.len() as u64, false);
        let args = &[ptr.into(), len_value.into()];
        let list_ptr = self.call_list_with_hint(args, hint);
        Ok(list_ptr)
    }

    fn emit_map_literal(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        entries: &[(Expression, Expression)],
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        self.emit_map_literal_hinted(ctx, entries, None)
    }

    fn emit_map_literal_hinted(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        entries: &[(Expression, Expression)],
        hint: Option<i8>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
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
            let mut entry_value = self.runtime.map_entry_type.get_undef();
            entry_value = self
                .builder
                .build_insert_value(entry_value, *key, 0, "map_key")
                .unwrap()
                .into_struct_value();
            entry_value = self
                .builder
                .build_insert_value(entry_value, *value, 1, "map_value")
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

    /// Build a Message map { name: <name_value>, data: <payload_value> } from already-evaluated values.
    fn build_message_value(
        &mut self,
        name_value: PointerValue<'ctx>,
        payload_value: PointerValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let entry_ptr_type = self.runtime.map_entry_type.ptr_type(AddressSpace::default());
        let array_type = self.runtime.map_entry_type.array_type(2);
        let mut temp_array = array_type.get_undef();

        let name_key = self.emit_string_literal("name");
        let data_key = self.emit_string_literal("data");

        let mut name_entry = self.runtime.map_entry_type.get_undef();
        name_entry = self
            .builder
            .build_insert_value(name_entry, name_key, 0, "msg_key")
            .unwrap()
            .into_struct_value();
        name_entry = self
            .builder
            .build_insert_value(name_entry, name_value, 1, "msg_value")
            .unwrap()
            .into_struct_value();

        let mut data_entry = self.runtime.map_entry_type.get_undef();
        data_entry = self
            .builder
            .build_insert_value(data_entry, data_key, 0, "msg_key")
            .unwrap()
            .into_struct_value();
        data_entry = self
            .builder
            .build_insert_value(data_entry, payload_value, 1, "msg_value")
            .unwrap()
            .into_struct_value();

        temp_array = self
            .builder
            .build_insert_value(temp_array, name_entry, 0, "msg_entry_name")
            .unwrap()
            .into_array_value();
        temp_array = self
            .builder
            .build_insert_value(temp_array, data_entry, 1, "msg_entry_data")
            .unwrap()
            .into_array_value();

        let alloca = self.builder.build_alloca(array_type, "message_literal").unwrap();
        self.builder.build_store(alloca, temp_array).unwrap();
        let ptr = self
            .builder
            .build_pointer_cast(alloca, entry_ptr_type, "message_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(2, false);
        let args = &[ptr.into(), len_value.into()];
        self.call_map_with_hint(args, None)
    }

    fn emit_member_expression(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        target: &Expression,
        property: &str,
        _span: Span,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        // For 'self' target (store instance), always use map lookup for field access
        if let Expression::Identifier(name, _) = target {
            if name == "self" {
                let target_value = self.emit_expression(ctx, target)?;
                let key_value = self.emit_string_literal(property);
                return Ok(self.call_runtime_ptr(
                    self.runtime.map_get,
                    &[target_value.into(), key_value.into()],
                    "map_get_property",
                ));
            }
        }
        let target_value = self.emit_expression(ctx, target)?;
        match property {
            "length" | "count" => Ok(self.call_runtime_ptr(
                self.runtime.value_length,
                &[target_value.into()],
                "value_length",
            )),
            "size" => Ok(self.call_runtime_ptr(
                self.runtime.map_length,
                &[target_value.into()],
                "map_length",
            )),
            "err" => {
                // x.err - returns true if x is an error value
                let is_err = self.builder
                    .build_call(self.runtime.is_err, &[target_value.into()], "is_err_check")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let is_err_bool = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    is_err,
                    self.context.i8_type().const_zero(),
                    "is_err_bool",
                ).unwrap();
                Ok(self.wrap_bool(is_err_bool))
            }
            _ => {
                let key_value = self.emit_string_literal(property);
                Ok(self.call_runtime_ptr(
                    self.runtime.map_get,
                    &[target_value.into(), key_value.into()],
                    "map_get_property",
                ))
            }
        }
    }

    fn emit_member_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        target: &Expression,
        property: &str,
        args: &[Expression],
        span: Span,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        if let Expression::Identifier(namespace, _) = target {
            if namespace == "io" {
                return self.emit_io_call(ctx, property, args, span);
            }
        }
        match property {
            // x.equals(y) - value equality comparison
            "equals" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("equals expects exactly one argument", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                let arg_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.value_equals,
                    &[target_value.into(), arg_value.into()],
                    "value_equals",
                ))
            }
            // x.not_equals(y) - value inequality comparison
            "not_equals" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("not_equals expects exactly one argument", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                let arg_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.value_not_equals,
                    &[target_value.into(), arg_value.into()],
                    "value_not_equals",
                ))
            }
            // x.not() - boolean negation
            "not" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("not does not take arguments", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                let bool_val = self.value_to_bool(target_value);
                let inverted = self.builder.build_not(bool_val, "not").unwrap();
                Ok(self.wrap_bool(inverted))
            }
            "iter" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("iter does not take arguments", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                Ok(self.call_runtime_ptr(
                    self.runtime.value_iter,
                    &[target_value.into()],
                    "value_iter",
                ))
            }
            "keys" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("keys does not take arguments", span));
                }
                let map_value = self.emit_expression(ctx, target)?;
                Ok(self.call_runtime_ptr(
                    self.runtime.map_keys,
                    &[map_value.into()],
                    "map_keys",
                ))
            }
            "map" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list.map expects a single function", span));
                }
                let list_value = self.emit_expression(ctx, target)?;
                let func_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.list_map,
                    &[list_value.into(), func_value.into()],
                    "list_map",
                ))
            }
            "filter" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list.filter expects a predicate", span));
                }
                let list_value = self.emit_expression(ctx, target)?;
                let func_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.list_filter,
                    &[list_value.into(), func_value.into()],
                    "list_filter",
                ))
            }
            "reduce" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(Diagnostic::new(
                        "list.reduce expects a function and optional seed",
                        span,
                    ));
                }
                let list_value = self.emit_expression(ctx, target)?;
                let (seed_arg, func_value) = if args.len() == 1 {
                    (self.runtime.value_ptr_type.const_null(), self.emit_expression(ctx, &args[0])?)
                } else {
                    (self.emit_expression(ctx, &args[0])?, self.emit_expression(ctx, &args[1])?)
                };
                let seed_meta: BasicMetadataValueEnum<'ctx> = seed_arg.into();
                Ok(self.call_runtime_ptr(
                    self.runtime.list_reduce,
                    &[list_value.into(), seed_meta, func_value.into()],
                    "list_reduce",
                ))
            }
            "push" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "list.push expects exactly one argument",
                        span,
                    ));
                }
                let list_value = self.emit_expression(ctx, target)?;
                let arg_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.list_push,
                    &[list_value.into(), arg_value.into()],
                    "list_push",
                ))
            }
            "pop" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new(
                        "list.pop does not take arguments",
                        span,
                    ));
                }
                let list_value = self.emit_expression(ctx, target)?;
                Ok(self.call_runtime_ptr(
                    self.runtime.list_pop,
                    &[list_value.into()],
                    "list_pop",
                ))
            }
            "get" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "map.get expects exactly one argument",
                        span,
                    ));
                }
                let map_value = self.emit_expression(ctx, target)?;
                let key_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.map_get,
                    &[map_value.into(), key_value.into()],
                    "map_get_method",
                ))
            }
            "set" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new(
                        "map.set expects exactly two arguments",
                        span,
                    ));
                }
                let map_value = self.emit_expression(ctx, target)?;
                let key_value = self.emit_expression(ctx, &args[0])?;
                let new_value = self.emit_expression(ctx, &args[1])?;
                
                // For self.field = value on stores with reference fields, handle retain/release
                if let Expression::Identifier(name, _) = target {
                    if name == "self" {
                        // Extract field name from key if it's a string literal
                        if let Expression::String(field_name, _) = &args[0] {
                            // Check all stores to see if any have this as a reference field
                            // Since we don't track the current store context, check if field is a reference in ANY store
                            let is_ref = self.reference_fields.iter().any(|(_, f)| f == field_name);
                            
                            if is_ref {
                                // Get old value before setting
                                let old_value = self.call_runtime_ptr(
                                    self.runtime.map_get,
                                    &[map_value.into(), key_value.into()],
                                    "get_old_ref",
                                );
                                // Retain new value
                                self.call_runtime_void(self.runtime.value_retain, &[new_value.into()], "retain_new_ref");
                                // Set the field
                                let result = self.call_runtime_ptr(
                                    self.runtime.map_set,
                                    &[map_value.into(), key_value.into(), new_value.into()],
                                    "map_set_method",
                                );
                                // Release old value
                                self.call_runtime_void(self.runtime.value_release, &[old_value.into()], "release_old_ref");
                                return Ok(result);
                            }
                        }
                    }
                }
                
                Ok(self.call_runtime_ptr(
                    self.runtime.map_set,
                    &[map_value.into(), key_value.into(), new_value.into()],
                    "map_set_method",
                ))
            }
            "at" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "list.at expects exactly one argument",
                        span,
                    ));
                }
                let list_value = self.emit_expression(ctx, target)?;
                let index_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.list_get,
                    &[list_value.into(), index_value.into()],
                    "list_get",
                ))
            }
            "length" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("length does not take arguments", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                Ok(self.call_runtime_ptr(
                    self.runtime.value_length,
                    &[target_value.into()],
                    "value_length",
                ))
            }
            _ => {
                // Check if this is a store method call
                if let Some((store_name, param_count)) = self.store_methods.get(property).cloned() {
                    // Verify argument count
                    if args.len() != param_count {
                        return Err(Diagnostic::new(
                            format!("method `{}` expects {} argument(s), but {} were provided", 
                                    property, param_count, args.len()),
                            span,
                        ));
                    }
                    // Emit target (the store instance)
                    let target_value = self.emit_expression(ctx, target)?;
                    // Build arguments: self (target) + user args as CoralValue* pointers
                    let mut call_args: Vec<BasicMetadataValueEnum> = vec![target_value.into()];
                    for arg in args {
                        let arg_val = self.emit_expression(ctx, arg)?;
                        // Pass as pointer (CoralValue*), not as number
                        call_args.push(arg_val.into());
                    }
                    // Build the mangled function name and look up function
                    let mangled = format!("{}_{}", store_name, property);
                    let store_method = *self.functions.get(&mangled)
                        .ok_or_else(|| Diagnostic::new(
                            format!("internal error: store method {} not found", mangled),
                            span,
                        ))?;
                    // Call the store method (returns ptr, not f64)
                    let result = self.builder.build_call(store_method, &call_args, "store_method_call")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();
                    // Return the ptr directly (CoralValue*)
                    Ok(result)
                } else {
                    Err(Diagnostic::new(
                        format!("method `{property}` not supported yet"),
                        span,
                    ))
                }
            }
        }
    }

    fn emit_io_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        method: &str,
        args: &[Expression],
        span: Span,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        match method {
            "read" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("io.read expects path", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(self.runtime.fs_read, &[path.into()], "io_read"))
            }
            "write" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("io.write expects path and data", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                let data = self.emit_expression(ctx, &args[1])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.fs_write,
                    &[path.into(), data.into()],
                    "io_write",
                ))
            }
            "exists" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("io.exists expects path", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_runtime_ptr(
                    self.runtime.fs_exists,
                    &[path.into()],
                    "io_exists",
                ))
            }
            _ => Err(Diagnostic::new(
                format!("namespace `io` has no method `{method}`"),
                span,
            )),
        }
    }

    fn emit_numeric_binary(
        &mut self,
        op: BinaryOp,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        use BinaryOp::*;
        if matches!(op, Add) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_add, args, "value_add"));
        }
        if matches!(op, BinaryOp::Equals) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_equals, args, "value_equals"));
        }
        if matches!(op, BinaryOp::NotEquals) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_not_equals, args, "value_not_equals"));
        }

        if matches!(op, BinaryOp::BitAnd) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_bitand, args, "bitand"));
        }
        if matches!(op, BinaryOp::BitOr) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_bitor, args, "bitor"));
        }
        if matches!(op, BinaryOp::BitXor) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_bitxor, args, "bitxor"));
        }
        if matches!(op, BinaryOp::ShiftLeft) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_shift_left, args, "shift_left"));
        }
        if matches!(op, BinaryOp::ShiftRight) {
            let args = &[lhs.into(), rhs.into()];
            return Ok(self.call_runtime_ptr(self.runtime.value_shift_right, args, "shift_right"));
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
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let left_value = self.emit_expression(ctx, left)?;
        let left_bool = self.value_to_bool(left_value);
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
        let right_bool = self.value_to_bool(right_value);
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
        Ok(self.wrap_bool(bool_value))
    }

    fn emit_ternary(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        condition: &Expression,
        then_branch: &Expression,
        else_branch: &Expression,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let cond_value = self.emit_expression(ctx, condition)?;
        let cond_bool = self.value_to_bool(cond_value);
        let function = ctx.function;
        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let cont_bb = self.context.append_basic_block(function, "cont");

        self.builder
            .build_conditional_branch(cond_bool, then_bb, else_bb)
            .unwrap();

        self.builder.position_at_end(then_bb);
        let then_value = self.emit_expression(ctx, then_branch)?;
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        let then_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(else_bb);
        let else_value = self.emit_expression(ctx, else_branch)?;
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        let else_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(self.runtime.value_ptr_type, "ternary_phi")
            .unwrap();
        let incoming = [
            (&then_value as &dyn BasicValue<'ctx>, then_end),
            (&else_value as &dyn BasicValue<'ctx>, else_end),
        ];
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    fn emit_match(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        match_expr: &MatchExpression,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let match_value = self.emit_expression(ctx, match_expr.value.as_ref())?;
        let function = ctx.function;
        let cont_bb = self.context.append_basic_block(function, "match_cont");
        let mut incoming: Vec<(PointerValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();

        let mut current_block = self
            .builder
            .get_insert_block()
            .expect("builder must be positioned before match");

        for (index, arm) in match_expr.arms.iter().enumerate() {
            let arm_block = self
                .context
                .append_basic_block(function, &format!("match_arm_{index}"));
            let next_block = self
                .context
                .append_basic_block(function, &format!("match_next_{index}"));

            self.builder.position_at_end(current_block);
            let condition = self.emit_match_condition(ctx, match_value, &arm.pattern, match_expr.span)?;
            self.builder
                .build_conditional_branch(condition, arm_block, next_block)
                .unwrap();

            self.builder.position_at_end(arm_block);
            // Bind pattern variables (including nested patterns)
            self.bind_pattern_variables(ctx, match_value, &arm.pattern);
            let result = self.emit_block(ctx, &arm.body)?;
            if arm_block.get_terminator().is_none() {
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .unwrap();
                incoming.push((result, arm_block));
            }

            current_block = next_block;
        }

        let default_block = self
            .context
            .append_basic_block(function, "match_default");
        self.builder.position_at_end(current_block);
        self.builder
            .build_unconditional_branch(default_block)
            .unwrap();

        self.builder.position_at_end(default_block);
        let default_value = if let Some(default_block_ast) = &match_expr.default {
            self.emit_block(ctx, default_block_ast.as_ref())?
        } else {
            self.wrap_number(self.f64_type.const_float(0.0))
        };
        if default_block.get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(cont_bb)
                .unwrap();
            incoming.push((default_value, default_block));
        }

        self.builder.position_at_end(cont_bb);
        if incoming.is_empty() {
            Ok(self.wrap_number(self.f64_type.const_float(0.0)))
        } else {
            let phi = self
                .builder
                .build_phi(self.runtime.value_ptr_type, "match_phi")
                .unwrap();
            for (value, block) in incoming {
                phi.add_incoming(&[(&value as &dyn BasicValue<'ctx>, block)]);
            }
            Ok(phi.as_basic_value().into_pointer_value())
        }
    }

    fn emit_match_condition(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        match_value: PointerValue<'ctx>,
        pattern: &MatchPattern,
        _span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match pattern {
            MatchPattern::Integer(value) => {
                let literal = self.wrap_number(self.f64_type.const_float(*value as f64));
                let eq = self.call_runtime_ptr(
                    self.runtime.value_equals,
                    &[match_value.into(), literal.into()],
                    "match_eq_num",
                );
                let as_bool = self.value_to_bool(eq);
                self.call_runtime_void(self.runtime.value_release, &[eq.into()], "match_eq_num_drop");
                self.call_runtime_void(self.runtime.value_release, &[literal.into()], "match_lit_drop");
                Ok(as_bool)
            }
            MatchPattern::Bool(value) => {
                let literal = self.wrap_bool(self.boolean_to_int(*value));
                let eq = self.call_runtime_ptr(
                    self.runtime.value_equals,
                    &[match_value.into(), literal.into()],
                    "match_eq_bool",
                );
                let as_bool = self.value_to_bool(eq);
                self.call_runtime_void(self.runtime.value_release, &[eq.into()], "match_eq_bool_drop");
                self.call_runtime_void(self.runtime.value_release, &[literal.into()], "match_bool_drop");
                Ok(as_bool)
            }
            MatchPattern::String(text) => {
                let literal = self.emit_string_literal(text);
                let eq = self.call_runtime_ptr(
                    self.runtime.value_equals,
                    &[match_value.into(), literal.into()],
                    "match_eq_str",
                );
                let as_bool = self.value_to_bool(eq);
                self.call_runtime_void(self.runtime.value_release, &[eq.into()], "match_eq_str_drop");
                self.call_runtime_void(self.runtime.value_release, &[literal.into()], "match_str_drop");
                Ok(as_bool)
            }
            MatchPattern::List(patterns) => {
                // Convert patterns to expressions for equality comparison
                let items: Vec<Expression> = patterns.iter().map(|p| match p {
                    MatchPattern::Integer(n) => Expression::Integer(*n, Span::new(0, 0)),
                    MatchPattern::Bool(b) => Expression::Bool(*b, Span::new(0, 0)),
                    MatchPattern::String(s) => Expression::String(s.clone(), Span::new(0, 0)),
                    MatchPattern::Identifier(name) => Expression::Identifier(name.clone(), Span::new(0, 0)),
                    _ => Expression::Identifier("_".to_string(), Span::new(0, 0)),
                }).collect();
                let list_lit = self.emit_list_literal(ctx, &items)?;
                let eq = self.call_runtime_ptr(
                    self.runtime.value_equals,
                    &[match_value.into(), list_lit.into()],
                    "match_eq_list",
                );
                let as_bool = self.value_to_bool(eq);
                self.call_runtime_void(self.runtime.value_release, &[eq.into()], "match_eq_list_drop");
                self.call_runtime_void(self.runtime.value_release, &[list_lit.into()], "match_list_drop");
                Ok(as_bool)
            }
            MatchPattern::Identifier(_) => Ok(self.bool_type.const_int(1, false)),
            MatchPattern::Wildcard(_) => Ok(self.bool_type.const_int(1, false)),
            MatchPattern::Constructor { name, fields, span } => {
                // For ADT constructor patterns, check if the tagged value's tag matches
                let tag_name_bytes = name.as_bytes();
                let tag_name_global = self.get_or_create_string_constant(name);
                let tag_name_ptr = self.builder
                    .build_pointer_cast(
                        tag_name_global.as_pointer_value(),
                        self.i8_type.ptr_type(AddressSpace::default()),
                        "tag_name_ptr",
                    )
                    .unwrap();
                let tag_name_len = self.usize_type.const_int(tag_name_bytes.len() as u64, false);
                
                // Call coral_tagged_is_tag(value, tag_name, tag_name_len)
                let is_tag_result = self.call_runtime_ptr(
                    self.runtime.tagged_is_tag,
                    &[match_value.into(), tag_name_ptr.into(), tag_name_len.into()],
                    "is_tag",
                );
                let tag_matches = self.value_to_bool(is_tag_result);
                self.call_runtime_void(self.runtime.value_release, &[is_tag_result.into()], "is_tag_drop");
                
                // If there are no nested field patterns, we're done
                if fields.is_empty() {
                    return Ok(tag_matches);
                }
                
                // For nested patterns, we need to check each field recursively
                // First, get the current function and create blocks for the nested check
                let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
                let nested_check_bb = self.context.append_basic_block(function, "nested_check");
                let nested_fail_bb = self.context.append_basic_block(function, "nested_fail");
                let nested_cont_bb = self.context.append_basic_block(function, "nested_cont");
                
                // Branch: if tag matches, check nested patterns; else fail
                self.builder.build_conditional_branch(tag_matches, nested_check_bb, nested_fail_bb).unwrap();
                
                // Nested check block: recursively check each field pattern
                self.builder.position_at_end(nested_check_bb);
                let mut all_fields_match = self.bool_type.const_int(1, false);
                
                for (idx, field_pattern) in fields.iter().enumerate() {
                    // Skip identifier and wildcard patterns - they always match
                    match field_pattern {
                        MatchPattern::Identifier(_) | MatchPattern::Wildcard(_) => continue,
                        _ => {}
                    }
                    
                    // Extract the field value
                    let idx_val = self.usize_type.const_int(idx as u64, false);
                    let field_value = self.call_runtime_ptr(
                        self.runtime.tagged_get_field,
                        &[match_value.into(), idx_val.into()],
                        &format!("nested_field_{}", idx),
                    );
                    
                    // Recursively check the nested pattern
                    let field_matches = self.emit_match_condition(ctx, field_value, field_pattern, *span)?;
                    
                    // Combine with previous results
                    all_fields_match = self.builder.build_and(all_fields_match, field_matches, "and_fields").unwrap();
                }
                
                self.builder.build_unconditional_branch(nested_cont_bb).unwrap();
                let nested_check_end = self.builder.get_insert_block().unwrap();
                
                // Nested fail block
                self.builder.position_at_end(nested_fail_bb);
                self.builder.build_unconditional_branch(nested_cont_bb).unwrap();
                
                // Continue block with phi
                self.builder.position_at_end(nested_cont_bb);
                let phi = self.builder.build_phi(self.bool_type, "nested_result").unwrap();
                phi.add_incoming(&[
                    (&all_fields_match, nested_check_end),
                    (&self.bool_type.const_int(0, false), nested_fail_bb),
                ]);
                
                Ok(phi.as_basic_value().into_int_value())
            }
        }
    }

    /// Recursively bind pattern variables, extracting fields from nested constructor patterns
    fn bind_pattern_variables(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        value: PointerValue<'ctx>,
        pattern: &MatchPattern,
    ) {
        match pattern {
            MatchPattern::Identifier(name) => {
                self.call_runtime_void(self.runtime.value_retain, &[value.into()], "retain_match_binding");
                self.store_variable(ctx, name, value);
            }
            MatchPattern::Constructor { fields, .. } => {
                // Extract field values and recursively bind them
                for (idx, field_pattern) in fields.iter().enumerate() {
                    let idx_val = self.usize_type.const_int(idx as u64, false);
                    let field_value = self.call_runtime_ptr(
                        self.runtime.tagged_get_field,
                        &[value.into(), idx_val.into()],
                        &format!("get_field_{}", idx),
                    );
                    // Recursively bind nested patterns
                    self.bind_pattern_variables(ctx, field_value, field_pattern);
                }
            }
            MatchPattern::Wildcard(_) => {
                // Wildcard patterns don't bind anything
            }
            _ => {
                // Other patterns (Integer, Bool, String, List) don't create bindings
            }
        }
    }

    fn load_variable(
        &mut self,
        ctx: &FunctionContext<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        // Check alloca-based variables first (these can be mutated in loops)
        if let Some(alloca) = ctx.variable_allocas.get(name) {
            let loaded = self
                .builder
                .build_load(
                    self.runtime.value_ptr_type,
                    *alloca,
                    &format!("load_{name}"),
                )
                .unwrap()
                .into_pointer_value();
            return Ok(loaded);
        }
        if let Some(ptr) = ctx.variables.get(name) {
            return Ok(*ptr);
        }
        if let Some(global) = self.global_variables.get(name) {
            let loaded = self
                .builder
                .build_load(
                    self.runtime.value_ptr_type,
                    global.as_pointer_value(),
                    &format!("load_global_{name}"),
                )
                .unwrap()
                .into_pointer_value();
            self.call_runtime_void(self.runtime.value_retain, &[loaded.into()], "retain_global");
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
        value: PointerValue<'ctx>,
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
            .build_alloca(self.runtime.value_ptr_type, &format!("{name}_ptr"))
            .unwrap();
        
        // Restore position and store the value
        self.builder.position_at_end(current_bb);
        self.builder.build_store(alloca, value).unwrap();
        ctx.variable_allocas.insert(name.to_string(), alloca);
    }

    fn wrap_number(&mut self, value: FloatValue<'ctx>) -> PointerValue<'ctx> {
        self.call_runtime_ptr(self.runtime.make_number, &[value.into()], "make_number")
    }

    fn wrap_bool(&mut self, value: IntValue<'ctx>) -> PointerValue<'ctx> {
        let bool_byte = self
            .builder
            .build_int_z_extend(value, self.i8_type, "bool_byte")
            .unwrap();
        self.call_runtime_ptr(self.runtime.make_bool, &[bool_byte.into()], "make_bool")
    }

    fn emit_bytes_literal(&mut self, literal: &[u8]) -> PointerValue<'ctx> {
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
    ) -> PointerValue<'ctx> {
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        let data_ptr = self
            .builder
            .build_pointer_cast(global.as_pointer_value(), i8_ptr_type, "bytes_ptr")
            .unwrap();
        let len_value = self.usize_type.const_int(len as u64, false);
        self.call_runtime_ptr(
            self.runtime.make_bytes,
            &[data_ptr.into(), len_value.into()],
            "make_bytes",
        )
    }

    fn wrap_unit(&mut self) -> PointerValue<'ctx> {
        let call = self
            .builder
            .build_call(self.runtime.make_unit, &[], "make_unit")
            .unwrap();
        call.try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value()
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

    fn emit_string_literal(&mut self, literal: &str) -> PointerValue<'ctx> {
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
        self.call_runtime_ptr(self.runtime.make_string, args, "make_string")
    }

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
    ) -> PointerValue<'ctx> {
        if let Some(h) = hint {
            let hint_val = self.i8_type.const_int(h as u64, false);
            let mut extended = Vec::with_capacity(args.len() + 1);
            extended.extend_from_slice(args);
            extended.push(hint_val.into());
            self.call_runtime_ptr(self.runtime.make_list_hinted, &extended, "make_list_hinted")
        } else {
            self.call_runtime_ptr(self.runtime.make_list, args, "make_list")
        }
    }

    fn call_map_with_hint(
        &mut self,
        args: &[BasicMetadataValueEnum<'ctx>],
        hint: Option<i8>,
    ) -> PointerValue<'ctx> {
        if let Some(h) = hint {
            let hint_val = self.i8_type.const_int(h as u64, false);
            let mut extended = Vec::with_capacity(args.len() + 1);
            extended.extend_from_slice(args);
            extended.push(hint_val.into());
            self.call_runtime_ptr(self.runtime.make_map_hinted, &extended, "make_map_hinted")
        } else {
            self.call_runtime_ptr(self.runtime.make_map, args, "make_map")
        }
    }

    fn value_to_number(&mut self, value: PointerValue<'ctx>) -> FloatValue<'ctx> {
        self
            .builder
            .build_call(self.runtime.value_as_number, &[value.into()], "value_as_number")
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
        value: PointerValue<'ctx>,
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
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
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
                    Statement::Break(_) | Statement::Continue(_) => false,
                    Statement::FieldAssign { value, .. } => self.contains_placeholder(value),
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

    fn value_to_bool(&mut self, value: PointerValue<'ctx>) -> IntValue<'ctx> {
        let byte = self
            .builder
            .build_call(self.runtime.value_as_bool, &[value.into()], "value_as_bool")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();
        self.builder
            .build_int_truncate(byte, self.bool_type, "bool_from_byte")
            .unwrap()
    }

    fn emit_lambda(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        params: &[Parameter],
        body: &Block,
        span: Span,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        if body
            .statements
            .iter()
            .any(|stmt| matches!(stmt, Statement::Return(_, _)))
        {
            return Err(Diagnostic::new(
                "return statements inside lambdas are not supported yet",
                span,
            ));
        }

        let capture_names = self.determine_lambda_captures(params, body, ctx);
        let mut capture_values = Vec::new();
        for name in &capture_names {
            if let Ok(value) = self.load_variable(ctx, name) {
                capture_values.push(value);
            }
        }

        let env_struct = if capture_names.is_empty() {
            None
        } else {
            let field_types: Vec<_> = capture_names
                .iter()
                .map(|_| self.runtime.value_ptr_type.into())
                .collect();
            Some(self.context.struct_type(&field_types, false))
        };

        let lambda_id = self.lambda_counter;
        self.lambda_counter += 1;

        let invoke_fn = self.module.add_function(
            &format!("lambda_invoke_{}", lambda_id),
            self.runtime.closure_invoke_type,
            None,
        );
        self.build_lambda_invoke(
            invoke_fn,
            env_struct,
            &capture_names,
            params,
            body,
        )?;

        let release_fn = if let Some(struct_type) = env_struct {
            let release = self.module.add_function(
                &format!("lambda_release_{}", lambda_id),
                self.runtime.closure_release_type,
                None,
            );
            self.build_lambda_release(release, struct_type, capture_names.len());
            Some(release)
        } else {
            None
        };

    let env_ptr = self.build_closure_env(env_struct, &capture_values, span)?;
        let invoke_ptr = invoke_fn.as_global_value().as_pointer_value();
        let release_ptr_type = self
            .runtime
            .closure_release_type
            .ptr_type(AddressSpace::default());
        let release_ptr = release_fn
            .map(|f| f.as_global_value().as_pointer_value())
            .unwrap_or_else(|| release_ptr_type.const_null());
        let args = &[invoke_ptr.into(), release_ptr.into(), env_ptr.into()];
        Ok(self.call_runtime_ptr(self.runtime.make_closure, args, "make_closure"))
    }

    /// Wrap a named function in a closure so it can be used as a first-class value.
    /// Generates a thunk function matching the closure invoke signature that
    /// extracts args from the args array and delegates to the original function.
    fn emit_function_as_closure(
        &mut self,
        _ctx: &FunctionContext<'ctx>,
        name: &str,
        target_fn: FunctionValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let param_count = target_fn.count_params();
        let saved_block = self.builder.get_insert_block();

        // Generate a thunk: void thunk(i8* env, Value** args, i64 nargs, Value** out)
        let thunk_name = format!("__fn_thunk_{name}");
        let thunk_fn = self.module.add_function(
            &thunk_name,
            self.runtime.closure_invoke_type,
            None,
        );
        let entry = self.context.append_basic_block(thunk_fn, "entry");
        self.builder.position_at_end(entry);

        let args_param = thunk_fn.get_nth_param(1).unwrap().into_pointer_value();
        let out_param = thunk_fn.get_nth_param(3).unwrap().into_pointer_value();

        // Extract each argument from the args array
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
        for i in 0..param_count {
            let index = self.usize_type.const_int(i as u64, false);
            let arg_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.runtime.value_ptr_type,
                        args_param,
                        &[index],
                        &format!("arg_ptr_{i}"),
                    )
                    .unwrap()
            };
            let arg_val = self
                .builder
                .build_load(
                    self.runtime.value_ptr_type,
                    arg_ptr,
                    &format!("arg_{i}"),
                )
                .unwrap()
                .into_pointer_value();
            call_args.push(arg_val.into());
        }

        // Call the original function
        let result = self
            .builder
            .build_call(target_fn, &call_args, "thunk_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store result and return
        self.builder.build_store(out_param, result).unwrap();
        self.builder.build_return(None).unwrap();

        // Restore builder position
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        // Create the closure with nil env and nil release
        let invoke_ptr = thunk_fn.as_global_value().as_pointer_value();
        let release_ptr_type = self
            .runtime
            .closure_release_type
            .ptr_type(inkwell::AddressSpace::default());
        let null_release = release_ptr_type.const_null();
        let null_env = self.runtime.value_ptr_type.const_null();
        let closure_args = &[invoke_ptr.into(), null_release.into(), null_env.into()];
        Ok(self.call_runtime_ptr(self.runtime.make_closure, closure_args, "fn_as_closure"))
    }

    /// Emit code to construct an enum (ADT) variant value.
    fn emit_enum_constructor(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        variant_name: &str,
        args: &[Expression],
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        // Create tag name as a global string constant
        let tag_name_bytes = variant_name.as_bytes();
        let tag_name_global = self.get_or_create_string_constant(variant_name);
        let tag_name_ptr = self.builder
            .build_pointer_cast(
                tag_name_global.as_pointer_value(),
                self.i8_type.ptr_type(AddressSpace::default()),
                "tag_name_ptr",
            )
            .unwrap();
        let tag_name_len = self.usize_type.const_int(tag_name_bytes.len() as u64, false);
        
        // Build array of field values
        let field_count = args.len();
        let (fields_ptr, field_count_val) = if field_count == 0 {
            let null_ptr = self.runtime.value_ptr_type
                .ptr_type(AddressSpace::default())
                .const_null();
            (null_ptr, self.usize_type.const_zero())
        } else {
            let mut field_values = Vec::with_capacity(field_count);
            for arg in args {
                let value = self.emit_expression(ctx, arg)?;
                field_values.push(value);
            }
            
            let array_type = self.runtime.value_ptr_type.array_type(field_count as u32);
            let mut temp_array = array_type.get_undef();
            for (idx, value) in field_values.iter().enumerate() {
                temp_array = self
                    .builder
                    .build_insert_value(temp_array, *value, idx as u32, "field")
                    .unwrap()
                    .into_array_value();
            }
            
            let alloca = self.builder.build_alloca(array_type, "tagged_fields").unwrap();
            self.builder.build_store(alloca, temp_array).unwrap();
            
            let ptr = self.builder
                .build_pointer_cast(
                    alloca,
                    self.runtime.value_ptr_type.ptr_type(AddressSpace::default()),
                    "fields_ptr",
                )
                .unwrap();
            (ptr, self.usize_type.const_int(field_count as u64, false))
        };
        
        // Call coral_make_tagged(tag_name, tag_name_len, fields, field_count)
        Ok(self.call_runtime_ptr(
            self.runtime.make_tagged,
            &[tag_name_ptr.into(), tag_name_len.into(), fields_ptr.into(), field_count_val.into()],
            "make_tagged",
        ))
    }
    
    /// Emit a nullary enum constructor (no fields, e.g., None)
    fn emit_enum_constructor_nullary(
        &mut self,
        variant_name: &str,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let tag_name_bytes = variant_name.as_bytes();
        let tag_name_global = self.get_or_create_string_constant(variant_name);
        let tag_name_ptr = self.builder
            .build_pointer_cast(
                tag_name_global.as_pointer_value(),
                self.i8_type.ptr_type(AddressSpace::default()),
                "tag_name_ptr",
            )
            .unwrap();
        let tag_name_len = self.usize_type.const_int(tag_name_bytes.len() as u64, false);
        
        let null_ptr = self.runtime.value_ptr_type
            .ptr_type(AddressSpace::default())
            .const_null();
        let zero = self.usize_type.const_zero();
        
        Ok(self.call_runtime_ptr(
            self.runtime.make_tagged,
            &[tag_name_ptr.into(), tag_name_len.into(), null_ptr.into(), zero.into()],
            "make_tagged_nullary",
        ))
    }

    fn emit_closure_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        closure: PointerValue<'ctx>,
        args: &[Expression],
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let mut arg_values = Vec::with_capacity(args.len());
        for arg in args {
            arg_values.push(self.emit_expression(ctx, arg)?);
        }
        let arg_ptr_type = self
            .runtime
            .value_ptr_type
            .ptr_type(AddressSpace::default());
        let (args_ptr, len_value) = if arg_values.is_empty() {
            (arg_ptr_type.const_null(), self.usize_type.const_zero())
        } else {
            let array_type = self
                .runtime
                .value_ptr_type
                .array_type(arg_values.len() as u32);
            let mut temp_array = array_type.get_undef();
            for (idx, value) in arg_values.iter().enumerate() {
                temp_array = self
                    .builder
                    .build_insert_value(temp_array, *value, idx as u32, "closure_arg")
                    .unwrap()
                    .into_array_value();
            }
            let alloca = self
                .builder
                .build_alloca(array_type, "closure_args")
                .unwrap();
            self.builder.build_store(alloca, temp_array).unwrap();
            let ptr = self
                .builder
                .build_pointer_cast(alloca, arg_ptr_type, "closure_args_ptr")
                .unwrap();
            let len = self
                .usize_type
                .const_int(arg_values.len() as u64, false);
            (ptr, len)
        };
        let args = &[closure.into(), args_ptr.into(), len_value.into()];
        Ok(self.call_runtime_ptr(self.runtime.closure_invoke, args, "closure_invoke"))
    }

    fn emit_builtin_call(
        &mut self,
        name: &str,
        args: &[Expression],
        ctx: &mut FunctionContext<'ctx>,
        span: Span,
    ) -> Result<Option<PointerValue<'ctx>>, Diagnostic> {
        match name {
            "log" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "log expects exactly one argument",
                        span,
                    ));
                }
                let value = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.log,
                    &[value.into()],
                    "log_call",
                )))
            }
            "concat" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new(
                        "concat expects exactly two arguments",
                        span,
                    ));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.value_add,
                    &[a.into(), b.into()],
                    "concat_call",
                )))
            }
            "fs_read" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "fs_read expects exactly one argument",
                        span,
                    ));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_read,
                    &[path.into()],
                    "fs_read_call",
                )))
            }
            "fs_write" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new(
                        "fs_write expects path and data",
                        span,
                    ));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                let data = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_write,
                    &[path.into(), data.into()],
                    "fs_write_call",
                )))
            }
            "fs_exists" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "fs_exists expects exactly one argument",
                        span,
                    ));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_exists,
                    &[path.into()],
                    "fs_exists_call",
                )))
            }
            "bit_and" | "bit_or" | "bit_xor" | "bit_shl" | "bit_shr" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new(
                        "bitwise helpers expect two arguments",
                        span,
                    ));
                }
                let lhs = self.emit_expression(ctx, &args[0])?;
                let rhs = self.emit_expression(ctx, &args[1])?;
                let func = match name {
                    "bit_and" => self.runtime.value_bitand,
                    "bit_or" => self.runtime.value_bitor,
                    "bit_xor" => self.runtime.value_bitxor,
                    "bit_shl" => self.runtime.value_shift_left,
                    _ => self.runtime.value_shift_right,
                };
                Ok(Some(self.call_runtime_ptr(func, &[lhs.into(), rhs.into()], "bit_call")))
            }
            "bit_not" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "bit_not expects one argument",
                        span,
                    ));
                }
                let value = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.value_bitnot,
                    &[value.into()],
                    "bit_not_call",
                )))
            }
            "actor_send" => {
                if args.len() != 2 && args.len() != 3 {
                    return Err(Diagnostic::new("actor_send expects actor, name, optional payload", span));
                }
                let actor = self.emit_expression(ctx, &args[0])?;
                let name = self.emit_expression(ctx, &args[1])?;
                let payload = if args.len() == 3 {
                    self.emit_expression(ctx, &args[2])?
                } else {
                    self.wrap_unit()
                };
                let message = self.build_message_value(name, payload);
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.actor_send,
                    &[actor.into(), message.into()],
                    "actor_send_builtin",
                )))
            }
            "actor_self" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("actor_self expects no arguments", span));
                }
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.actor_self,
                    &[],
                    "actor_self_builtin",
                )))
            }
            // String operations
            "string_slice" | "slice" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("string_slice expects string, start, end", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let start = self.emit_expression(ctx, &args[1])?;
                let end = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_slice,
                    &[s.into(), start.into(), end.into()],
                    "string_slice_call",
                )))
            }
            "string_char_at" | "char_at" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_char_at expects string, index", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let idx = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_char_at,
                    &[s.into(), idx.into()],
                    "string_char_at_call",
                )))
            }
            "string_index_of" | "index_of" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_index_of expects haystack, needle", span));
                }
                let haystack = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_index_of,
                    &[haystack.into(), needle.into()],
                    "string_index_of_call",
                )))
            }
            "string_split" | "split" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_split expects string, delimiter", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let delim = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_split,
                    &[s.into(), delim.into()],
                    "string_split_call",
                )))
            }
            "string_to_chars" | "chars" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_to_chars expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_to_chars,
                    &[s.into()],
                    "string_to_chars_call",
                )))
            }
            "string_starts_with" | "starts_with" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_starts_with expects string, prefix", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let prefix = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_starts_with,
                    &[s.into(), prefix.into()],
                    "string_starts_with_call",
                )))
            }
            "string_ends_with" | "ends_with" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_ends_with expects string, suffix", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let suffix = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_ends_with,
                    &[s.into(), suffix.into()],
                    "string_ends_with_call",
                )))
            }
            "string_trim" | "trim" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_trim expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_trim,
                    &[s.into()],
                    "string_trim_call",
                )))
            }
            "string_to_upper" | "to_upper" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_to_upper expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_to_upper,
                    &[s.into()],
                    "string_to_upper_call",
                )))
            }
            "string_to_lower" | "to_lower" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_to_lower expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_to_lower,
                    &[s.into()],
                    "string_to_lower_call",
                )))
            }
            "string_replace" | "replace" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("string_replace expects string, old, new", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let old = self.emit_expression(ctx, &args[1])?;
                let new = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_replace,
                    &[s.into(), old.into(), new.into()],
                    "string_replace_call",
                )))
            }
            "string_contains" | "contains" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_contains expects haystack, needle", span));
                }
                let haystack = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_contains,
                    &[haystack.into(), needle.into()],
                    "string_contains_call",
                )))
            }
            "string_parse_number" | "parse_number" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_parse_number expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_parse_number,
                    &[s.into()],
                    "string_parse_number_call",
                )))
            }
            "number_to_string" | "to_string" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("number_to_string expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.number_to_string,
                    &[n.into()],
                    "number_to_string_call",
                )))
            }
            // Math functions - unary
            "abs" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("abs expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_abs,
                    &[n.into()],
                    "math_abs_call",
                )))
            }
            "sqrt" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sqrt expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_sqrt,
                    &[n.into()],
                    "math_sqrt_call",
                )))
            }
            "floor" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("floor expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_floor,
                    &[n.into()],
                    "math_floor_call",
                )))
            }
            "ceil" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("ceil expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_ceil,
                    &[n.into()],
                    "math_ceil_call",
                )))
            }
            "round" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("round expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_round,
                    &[n.into()],
                    "math_round_call",
                )))
            }
            "sin" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sin expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_sin,
                    &[n.into()],
                    "math_sin_call",
                )))
            }
            "cos" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("cos expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_cos,
                    &[n.into()],
                    "math_cos_call",
                )))
            }
            "tan" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("tan expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_tan,
                    &[n.into()],
                    "math_tan_call",
                )))
            }
            "ln" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("ln expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_ln,
                    &[n.into()],
                    "math_ln_call",
                )))
            }
            "log10" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("log10 expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_log10,
                    &[n.into()],
                    "math_log10_call",
                )))
            }
            "exp" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("exp expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_exp,
                    &[n.into()],
                    "math_exp_call",
                )))
            }
            "asin" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("asin expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_asin,
                    &[n.into()],
                    "math_asin_call",
                )))
            }
            "acos" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("acos expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_acos,
                    &[n.into()],
                    "math_acos_call",
                )))
            }
            "atan" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("atan expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_atan,
                    &[n.into()],
                    "math_atan_call",
                )))
            }
            "sinh" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sinh expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_sinh,
                    &[n.into()],
                    "math_sinh_call",
                )))
            }
            "cosh" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("cosh expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_cosh,
                    &[n.into()],
                    "math_cosh_call",
                )))
            }
            "tanh" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("tanh expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_tanh,
                    &[n.into()],
                    "math_tanh_call",
                )))
            }
            "trunc" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("trunc expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_trunc,
                    &[n.into()],
                    "math_trunc_call",
                )))
            }
            "sign" | "signum" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sign expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_sign,
                    &[n.into()],
                    "math_sign_call",
                )))
            }
            // Math functions - binary
            "pow" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("pow expects two arguments (base, exponent)", span));
                }
                let base = self.emit_expression(ctx, &args[0])?;
                let exp = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_pow,
                    &[base.into(), exp.into()],
                    "math_pow_call",
                )))
            }
            "min" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("min expects two arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_min,
                    &[a.into(), b.into()],
                    "math_min_call",
                )))
            }
            "max" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("max expects two arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_max,
                    &[a.into(), b.into()],
                    "math_max_call",
                )))
            }
            "atan2" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("atan2 expects two arguments (y, x)", span));
                }
                let y = self.emit_expression(ctx, &args[0])?;
                let x = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.math_atan2,
                    &[y.into(), x.into()],
                    "math_atan2_call",
                )))
            }
            // Process/environment
            "process_args" | "args" => {
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.process_args,
                    &[],
                    "process_args_call",
                )))
            }
            "process_exit" | "exit" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("exit expects one argument (exit code)", span));
                }
                let code = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.process_exit,
                    &[code.into()],
                    "process_exit_call",
                )))
            }
            "env_get" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("env_get expects one argument", span));
                }
                let name_val = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.env_get,
                    &[name_val.into()],
                    "env_get_call",
                )))
            }
            "env_set" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("env_set expects two arguments (name, value)", span));
                }
                let name_val = self.emit_expression(ctx, &args[0])?;
                let val = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.env_set,
                    &[name_val.into(), val.into()],
                    "env_set_call",
                )))
            }
            // File I/O extensions
            "fs_append" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("fs_append expects path and data", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                let data = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_append,
                    &[path.into(), data.into()],
                    "fs_append_call",
                )))
            }
            "fs_read_dir" | "read_dir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_read_dir expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_read_dir,
                    &[path.into()],
                    "fs_read_dir_call",
                )))
            }
            "fs_mkdir" | "mkdir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_mkdir expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_mkdir,
                    &[path.into()],
                    "fs_mkdir_call",
                )))
            }
            "fs_delete" | "delete" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_delete expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_delete,
                    &[path.into()],
                    "fs_delete_call",
                )))
            }
            "fs_is_dir" | "is_dir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_is_dir expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.fs_is_dir,
                    &[path.into()],
                    "fs_is_dir_call",
                )))
            }
            "stdin_read_line" | "read_line" => {
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.stdin_read_line,
                    &[],
                    "stdin_read_line_call",
                )))
            }
            // List extensions
            "list_contains" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_contains expects list and value", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_contains,
                    &[list.into(), needle.into()],
                    "list_contains_call",
                )))
            }
            "list_index_of" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_index_of expects list and value", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_index_of,
                    &[list.into(), needle.into()],
                    "list_index_of_call",
                )))
            }
            "list_reverse" | "reverse" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list_reverse expects one argument", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_reverse,
                    &[list.into()],
                    "list_reverse_call",
                )))
            }
            "list_slice" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("list_slice expects list, start, end", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let start = self.emit_expression(ctx, &args[1])?;
                let end = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_slice,
                    &[list.into(), start.into(), end.into()],
                    "list_slice_call",
                )))
            }
            "list_sort" | "sort" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list_sort expects one argument", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_sort,
                    &[list.into()],
                    "list_sort_call",
                )))
            }
            "list_join" | "join" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_join expects list and separator", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let sep = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_join,
                    &[list.into(), sep.into()],
                    "list_join_call",
                )))
            }
            "list_concat" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_concat expects two lists", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.list_concat,
                    &[a.into(), b.into()],
                    "list_concat_call",
                )))
            }
            // Map extensions
            "map_remove" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("map_remove expects map and key", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                let key = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.map_remove,
                    &[map.into(), key.into()],
                    "map_remove_call",
                )))
            }
            "map_values" | "values" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("map_values expects one argument", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.map_values,
                    &[map.into()],
                    "map_values_call",
                )))
            }
            "map_entries" | "entries" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("map_entries expects one argument", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.map_entries,
                    &[map.into()],
                    "map_entries_call",
                )))
            }
            "map_has_key" | "has_key" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("map_has_key expects map and key", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                let key = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.map_has_key,
                    &[map.into(), key.into()],
                    "map_has_key_call",
                )))
            }
            "map_merge" | "merge" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("map_merge expects two maps", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.map_merge,
                    &[a.into(), b.into()],
                    "map_merge_call",
                )))
            }
            // Bytes extensions
            "bytes_get" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("bytes_get expects bytes and index", span));
                }
                let b = self.emit_expression(ctx, &args[0])?;
                let idx = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.bytes_get,
                    &[b.into(), idx.into()],
                    "bytes_get_call",
                )))
            }
            "bytes_from_string" | "to_bytes" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("bytes_from_string expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.bytes_from_string,
                    &[s.into()],
                    "bytes_from_string_call",
                )))
            }
            "bytes_to_string" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("bytes_to_string expects one argument", span));
                }
                let b = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.bytes_to_string,
                    &[b.into()],
                    "bytes_to_string_call",
                )))
            }
            "bytes_slice" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("bytes_slice expects bytes, start, end", span));
                }
                let b = self.emit_expression(ctx, &args[0])?;
                let start = self.emit_expression(ctx, &args[1])?;
                let end = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.bytes_slice_val,
                    &[b.into(), start.into(), end.into()],
                    "bytes_slice_call",
                )))
            }
            // Type reflection
            "type_of" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("type_of expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.type_of,
                    &[v.into()],
                    "type_of_call",
                )))
            }
            // Character operations
            "ord" | "string_ord" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("ord expects one string argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_ord,
                    &[s.into()],
                    "ord_call",
                )))
            }
            "chr" | "string_chr" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("chr expects one number argument", span));
                }
                let code = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_chr,
                    &[code.into()],
                    "chr_call",
                )))
            }
            "string_compare" | "strcmp" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_compare expects two string arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.string_compare,
                    &[a.into(), b.into()],
                    "strcmp_call",
                )))
            }
            // Error checking builtins
            "is_err" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("is_err expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                let is_err = self.builder
                    .build_call(self.runtime.is_err, &[v.into()], "is_err_check")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let is_err_bool = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    is_err,
                    self.context.i8_type().const_zero(),
                    "is_err_bool",
                ).unwrap();
                Ok(Some(self.wrap_bool(is_err_bool)))
            }
            "is_ok" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("is_ok expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                let is_ok = self.builder
                    .build_call(self.runtime.is_ok, &[v.into()], "is_ok_check")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let is_ok_bool = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    is_ok,
                    self.context.i8_type().const_zero(),
                    "is_ok_bool",
                ).unwrap();
                Ok(Some(self.wrap_bool(is_ok_bool)))
            }
            "is_absent" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("is_absent expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                let is_absent = self.builder
                    .build_call(self.runtime.is_absent, &[v.into()], "is_absent_check")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let is_absent_bool = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE,
                    is_absent,
                    self.context.i8_type().const_zero(),
                    "is_absent_bool",
                ).unwrap();
                Ok(Some(self.wrap_bool(is_absent_bool)))
            }
            "error_name" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("error_name expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_runtime_ptr(
                    self.runtime.error_name,
                    &[v.into()],
                    "error_name_call",
                )))
            }
            "error_code" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("error_code expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                let code_i32 = self.builder
                    .build_call(self.runtime.error_code, &[v.into()], "error_code_call")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                // Convert i32 to f64 for wrap_number
                let code_f64 = self.builder.build_signed_int_to_float(
                    code_i32,
                    self.context.f64_type(),
                    "code_f64",
                ).unwrap();
                Ok(Some(self.wrap_number(code_f64)))
            }
            _ => {
                // Check if it's a store/actor constructor (not arbitrary make_* functions)
                if self.store_constructors.contains(name) {
                    let ctor_fn = self.functions[name];
                    let call = self.builder.build_call(ctor_fn, &[], "actor_ctor").unwrap();
                    let handle = call.try_as_basic_value().left()
                        .ok_or_else(|| Diagnostic::new("actor constructor produced no value", span))?
                        .into_pointer_value();
                    Ok(Some(handle))
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn determine_lambda_captures(
        &self,
        params: &[Parameter],
        body: &Block,
        ctx: &FunctionContext<'ctx>,
    ) -> Vec<String> {
        let mut available: HashSet<String> = ctx.variables.keys().cloned().collect();
        available.extend(ctx.variable_allocas.keys().cloned());
        let mut locals: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut captures = Vec::new();
        let mut seen = HashSet::new();
        self.collect_captures_block(body, &available, &mut locals, &mut captures, &mut seen);
        captures
    }

    fn collect_captures_block(
        &self,
        block: &Block,
        available: &HashSet<String>,
        parent_locals: &mut HashSet<String>,
        captures: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        let mut locals = parent_locals.clone();
        for stmt in &block.statements {
            match stmt {
                Statement::Binding(binding) => {
                    self.collect_captures_expr(
                        &binding.value,
                        available,
                        &mut locals,
                        captures,
                        seen,
                    );
                    locals.insert(binding.name.clone());
                }
                Statement::Expression(expr) => self.collect_captures_expr(
                    expr,
                    available,
                    &mut locals,
                    captures,
                    seen,
                ),
                Statement::Return(expr, _) => self.collect_captures_expr(
                    expr,
                    available,
                    &mut locals,
                    captures,
                    seen,
                ),
                Statement::If { condition, body, elif_branches, else_body, .. } => {
                    self.collect_captures_expr(condition, available, &mut locals, captures, seen);
                    self.collect_captures_block(body, available, &mut locals, captures, seen);
                    for (cond, blk) in elif_branches {
                        self.collect_captures_expr(cond, available, &mut locals, captures, seen);
                        self.collect_captures_block(blk, available, &mut locals, captures, seen);
                    }
                    if let Some(else_blk) = else_body {
                        self.collect_captures_block(else_blk, available, &mut locals, captures, seen);
                    }
                }
                Statement::While { condition, body, .. } => {
                    self.collect_captures_expr(condition, available, &mut locals, captures, seen);
                    self.collect_captures_block(body, available, &mut locals, captures, seen);
                }
                Statement::For { variable, iterable, body, .. } => {
                    self.collect_captures_expr(iterable, available, &mut locals, captures, seen);
                    locals.insert(variable.clone());
                    self.collect_captures_block(body, available, &mut locals, captures, seen);
                }
                Statement::Break(_) | Statement::Continue(_) => {}
                Statement::FieldAssign { target, value, .. } => {
                    self.collect_captures_expr(target, available, &mut locals, captures, seen);
                    self.collect_captures_expr(value, available, &mut locals, captures, seen);
                }
            }
        }
        if let Some(value) = &block.value {
            self.collect_captures_expr(value, available, &mut locals, captures, seen);
        }
    }

    fn collect_captures_expr(
        &self,
        expr: &Expression,
        available: &HashSet<String>,
        locals: &mut HashSet<String>,
        captures: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        match expr {
            Expression::Identifier(name, _) => {
                if !locals.contains(name) && available.contains(name) && seen.insert(name.clone()) {
                    captures.push(name.clone());
                }
            }
            Expression::Binary { left, right, .. } => {
                self.collect_captures_expr(left, available, locals, captures, seen);
                self.collect_captures_expr(right, available, locals, captures, seen);
            }
            Expression::Unary { expr, .. } =>
                self.collect_captures_expr(expr, available, locals, captures, seen),
            Expression::List(items, _) => {
                for item in items {
                    self.collect_captures_expr(item, available, locals, captures, seen);
                }
            }
            Expression::Bytes(_, _) => {}
            Expression::Map(entries, _) => {
                for (key, value) in entries {
                    self.collect_captures_expr(key, available, locals, captures, seen);
                    self.collect_captures_expr(value, available, locals, captures, seen);
                }
            }
            Expression::Call { callee, args, .. } => {
                self.collect_captures_expr(callee, available, locals, captures, seen);
                for arg in args {
                    self.collect_captures_expr(arg, available, locals, captures, seen);
                }
            }
            Expression::Member { target, .. } =>
                self.collect_captures_expr(target, available, locals, captures, seen),
            Expression::Index { target, index, .. } => {
                self.collect_captures_expr(target, available, locals, captures, seen);
                self.collect_captures_expr(index, available, locals, captures, seen);
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_captures_expr(condition, available, locals, captures, seen);
                self.collect_captures_expr(then_branch, available, locals, captures, seen);
                self.collect_captures_expr(else_branch, available, locals, captures, seen);
            }
            Expression::Match(match_expr) => {
                self.collect_captures_expr(&match_expr.value, available, locals, captures, seen);
                for arm in &match_expr.arms {
                    self.collect_captures_block(&arm.body, available, locals, captures, seen);
                }
                if let Some(default) = &match_expr.default {
                    self.collect_captures_block(default, available, locals, captures, seen);
                }
            }
            Expression::Lambda { params, body, .. } => {
                let mut nested = locals.clone();
                for param in params {
                    nested.insert(param.name.clone());
                }
                self.collect_captures_block(body, available, &mut nested, captures, seen);
            }
            Expression::Throw { value, .. } =>
                self.collect_captures_expr(value, available, locals, captures, seen),
            Expression::Pipeline { left, right, .. } => {
                self.collect_captures_expr(left, available, locals, captures, seen);
                self.collect_captures_expr(right, available, locals, captures, seen);
            }
            Expression::ErrorValue { .. } => {
                // Error values don't capture any variables
            }
            Expression::ErrorPropagate { expr, .. } => {
                self.collect_captures_expr(expr, available, locals, captures, seen);
            }
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::Bool(_, _)
            | Expression::String(_, _)
            | Expression::TaxonomyPath { .. }
            | Expression::Placeholder(_, _)
            | Expression::InlineAsm { .. }
            | Expression::PtrLoad { .. }
            | Expression::Unsafe { .. }
            | Expression::Unit
            | Expression::None(_) => {}
        }
    }

    fn build_lambda_invoke(
        &mut self,
        invoke_fn: FunctionValue<'ctx>,
        env_struct: Option<StructType<'ctx>>,
        capture_names: &[String],
        params: &[Parameter],
        body: &Block,
    ) -> Result<(), Diagnostic> {
        let saved_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(invoke_fn, "entry");
        self.builder.position_at_end(entry);

        let env_param = invoke_fn.get_nth_param(0).unwrap().into_pointer_value();
        let args_param = invoke_fn.get_nth_param(1).unwrap().into_pointer_value();
        let out_param = invoke_fn.get_nth_param(3).unwrap().into_pointer_value();

        let mut lambda_ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: invoke_fn,
            loop_stack: Vec::new(),
        };

        if let Some(struct_type) = env_struct {
            if !capture_names.is_empty() {
                let typed_env = self
                    .builder
                    .build_pointer_cast(
                        env_param,
                        struct_type.ptr_type(AddressSpace::default()),
                        "closure_env",
                    )
                    .unwrap();
                for (idx, name) in capture_names.iter().enumerate() {
                    let field_ptr = self
                        .builder
                        .build_struct_gep(
                            struct_type,
                            typed_env,
                            idx as u32,
                            &format!("capture_gep_{}", idx),
                        )
                        .unwrap();
                    let value = self
                        .builder
                        .build_load(
                            self.runtime.value_ptr_type,
                            field_ptr,
                            &format!("capture_load_{}", idx),
                        )
                        .unwrap()
                        .into_pointer_value();
                    lambda_ctx.variable_allocas.insert(name.clone(), {
                        let alloca = self.builder.build_alloca(self.runtime.value_ptr_type, &format!("{name}_ptr")).unwrap();
                        self.builder.build_store(alloca, value).unwrap();
                        alloca
                    });
                }
            }
        }

        for (idx, param) in params.iter().enumerate() {
            let index = self.usize_type.const_int(idx as u64, false);
            let arg_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.runtime.value_ptr_type,
                        args_param,
                        &[index],
                        &format!("lambda_arg_ptr_{}", idx),
                    )
                    .unwrap()
            };
            let arg_value = self
                .builder
                .build_load(
                    self.runtime.value_ptr_type,
                    arg_ptr,
                    &format!("lambda_arg_{}", idx),
                )
                .unwrap()
                .into_pointer_value();
            self.store_variable(&mut lambda_ctx, &param.name, arg_value);
        }

    let result = self.emit_block(&mut lambda_ctx, body)?;
        self.builder.build_store(out_param, result).unwrap();
        self.builder.build_return(None).unwrap();

        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
    }

    fn build_lambda_release(
        &mut self,
        release_fn: FunctionValue<'ctx>,
        env_struct: StructType<'ctx>,
        capture_count: usize,
    ) {
        let saved_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(release_fn, "entry");
        self.builder.position_at_end(entry);
        let env_param = release_fn.get_first_param().unwrap().into_pointer_value();
        let is_null = self
            .builder
            .build_is_null(env_param, "env_is_null")
            .unwrap();
        let exit_block = self.context.append_basic_block(release_fn, "release_exit");
        let body_block = self.context.append_basic_block(release_fn, "release_body");
        self.builder
            .build_conditional_branch(is_null, exit_block, body_block)
            .unwrap();

        self.builder.position_at_end(body_block);
        let typed_env = self
            .builder
            .build_pointer_cast(
                env_param,
                env_struct.ptr_type(AddressSpace::default()),
                "release_env",
            )
            .unwrap();
        for idx in 0..capture_count {
            let field_ptr = self
                .builder
                .build_struct_gep(
                    env_struct,
                    typed_env,
                    idx as u32,
                    &format!("release_capture_ptr_{}", idx),
                )
                .unwrap();
            let value = self
                .builder
                .build_load(
                    self.runtime.value_ptr_type,
                    field_ptr,
                    &format!("release_capture_{}", idx),
                )
                .unwrap()
                .into_pointer_value();
            self.call_runtime_void(self.runtime.value_release, &[value.into()], "release_capture");
        }
        let raw_ptr = self
            .builder
            .build_pointer_cast(
                typed_env,
                self.i8_type.ptr_type(AddressSpace::default()),
                "release_env_raw",
            )
            .unwrap();
        self.call_runtime_void(self.runtime.heap_free, &[raw_ptr.into()], "closure_env_free");
        self
            .builder
            .build_unconditional_branch(exit_block)
            .unwrap();

        self.builder.position_at_end(exit_block);
        self.builder.build_return(None).unwrap();

        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
    }

    fn build_closure_env(
        &mut self,
        env_struct: Option<StructType<'ctx>>,
        capture_values: &[PointerValue<'ctx>],
        span: Span,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        if let Some(struct_type) = env_struct {
            let size = struct_type
                .size_of()
                .ok_or_else(|| Diagnostic::new("failed to compute closure env size", span))?;
            let raw_ptr = self
                .builder
                .build_call(self.runtime.heap_alloc, &[size.into()], "closure_env_alloc")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();
            let typed_ptr = self
                .builder
                .build_pointer_cast(
                    raw_ptr,
                    struct_type.ptr_type(AddressSpace::default()),
                    "closure_env_typed",
                )
                .unwrap();
            for (idx, value) in capture_values.iter().enumerate() {
                self.call_runtime_void(self.runtime.value_retain, &[(*value).into()], "retain_capture");
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        struct_type,
                        typed_ptr,
                        idx as u32,
                        &format!("env_store_capture_ptr_{}", idx),
                    )
                    .unwrap();
                self.builder.build_store(field_ptr, *value).unwrap();
            }
            Ok(
                self.builder
                    .build_pointer_cast(typed_ptr, i8_ptr_type, "closure_env_raw")
                    .unwrap(),
            )
        } else {
            Ok(i8_ptr_type.const_null())
        }
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
                self.runtime.value_ptr_type,
                None,
                &format!("coral_global_{}", binding.name),
            );
            global.set_initializer(&self.runtime.value_ptr_type.const_null());
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

    fn build_actor_constructor(&mut self, store: &crate::ast::StoreDefinition) -> Result<(), Diagnostic> {
        let constructor_name = format!("make_{}", store.name);
        let ctor_fn = *self.functions.get(&constructor_name).unwrap();
        let entry = self.context.append_basic_block(ctor_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();

        // 1. Create state Map with field defaults
        let null_entries = self.runtime.map_entry_type
            .ptr_type(AddressSpace::default())
            .const_null();
        let zero_len = self.usize_type.const_zero();
        let state_map = self.call_runtime_ptr(self.runtime.make_map, &[null_entries.into(), zero_len.into()], "actor_state");
        // Emit a dummy context for field evaluation
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: ctor_fn,
            loop_stack: Vec::new(),
        };
        for field in &store.fields {
            let key = self.emit_string_literal(&field.name);
            let value = if let Some(default) = &field.default {
                self.emit_expression(&mut ctx, default)?
            } else {
                // Default to 0 for fields without default
                self.wrap_number(self.f64_type.const_float(0.0))
            };
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[state_map.into(), key.into(), value.into()],
                "set_field",
            );
        }

        // 2. Build the handler invoke function (follows closure signature)
        // Signature: fn(env: *mut c_void, args: *const ValueHandle, len: usize, out: *mut ValueHandle)
        let handler_fn_name = format!("__{}_handler_invoke", store.name);
        let handler_invoke_fn = self.module.add_function(
            &handler_fn_name,
            self.runtime.closure_invoke_type,
            None,
        );
        self.build_actor_handler_invoke(handler_invoke_fn, store)?;

        // 3. Build the handler release function
        let release_fn_name = format!("__{}_handler_release", store.name);
        let release_fn = self.module.add_function(
            &release_fn_name,
            self.runtime.closure_release_type,
            None,
        );
        self.build_actor_handler_release(release_fn);

        // 4. Back to constructor: create closure with state as env and spawn actor
        self.builder.position_at_end(entry);
        let handler_closure = self.call_runtime_ptr(
            self.runtime.make_closure,
            &[
                handler_invoke_fn.as_global_value().as_pointer_value().into(),
                release_fn.as_global_value().as_pointer_value().into(),
                state_map.into(), // state Map as closure environment
            ],
            "handler_closure",
        );

        let actor_handle = self.call_runtime_ptr(
            self.runtime.actor_spawn,
            &[handler_closure.into()],
            "actor",
        );

        self.builder.build_return(Some(&actor_handle)).unwrap();
        Ok(())
    }

    /// Build handler invoke function for actor message dispatch.
    /// env = actor state Map, args[0] = self (actor handle), args[1] = message
    fn build_actor_handler_invoke(
        &mut self,
        invoke_fn: FunctionValue<'ctx>,
        store: &crate::ast::StoreDefinition,
    ) -> Result<(), Diagnostic> {
        let saved_block = self.builder.get_insert_block();
        let handler_entry = self.context.append_basic_block(invoke_fn, "entry");
        self.builder.position_at_end(handler_entry);

        // env = state Map (ValuePtr cast from *mut c_void)
        let env_param = invoke_fn.get_nth_param(0).unwrap().into_pointer_value();
        let args_param = invoke_fn.get_nth_param(1).unwrap().into_pointer_value();
        // Cast env to ValuePtr for state access
        let state = self.builder.build_pointer_cast(
            env_param,
            self.runtime.value_ptr_type,
            "actor_state",
        ).unwrap();
        
        // args[1] = message
        let msg_index = self.usize_type.const_int(1, false);
        let msg_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.runtime.value_ptr_type,
                args_param,
                &[msg_index],
                "msg_ptr",
            ).unwrap()
        };
        let msg_param = self.builder.build_load(
            self.runtime.value_ptr_type,
            msg_ptr,
            "msg",
        ).unwrap().into_pointer_value();

        // Extract message name/data and dispatch to @message methods
        // Use hash-based dispatch table for efficient message routing
        let name_key = self.emit_string_literal("name");
        let data_key = self.emit_string_literal("data");
        let name_field = self.call_runtime_ptr(
            self.runtime.map_get,
            &[msg_param.into(), name_key.into()],
            "msg_name",
        );
        let data_field = self.call_runtime_ptr(
            self.runtime.map_get,
            &[msg_param.into(), data_key.into()],
            "msg_data",
        );

        // Collect message handlers
        let handlers: Vec<_> = store.methods.iter()
            .filter(|m| m.kind == crate::ast::FunctionKind::ActorMessage)
            .collect();

        if handlers.is_empty() {
            // No handlers, just return
            self.builder.build_return(None).unwrap();
            if let Some(block) = saved_block {
                self.builder.position_at_end(block);
            }
            return Ok(());
        }

        // Sequential dispatch: compare message name against each handler.
        // (Hash-based dispatch removed due to compile-time/runtime hash mismatch
        //  and value_hash return-type incompatibility with call_runtime_ptr.)
        let done_bb = self.context.append_basic_block(invoke_fn, "msg_done");

        let mut current_bb = handler_entry;
        for method in &handlers {
            let match_bb = self.context.append_basic_block(invoke_fn, &format!("msg_{}_match", method.name));
            let next_bb = self.context.append_basic_block(invoke_fn, &format!("msg_{}_next", method.name));

            self.builder.position_at_end(current_bb);
            let method_name = self.emit_string_literal(&method.name);
            let eq = self.call_runtime_ptr(
                self.runtime.value_equals,
                &[name_field.into(), method_name.into()],
                "msg_name_eq",
            );
            let is_match = self.value_to_bool(eq);
            self.builder
                .build_conditional_branch(is_match, match_bb, next_bb)
                .unwrap();

            self.builder.position_at_end(match_bb);
            let mangled = format!("{}_{}", store.name, method.name);
            if let Some(target_fn) = self.functions.get(&mangled).copied() {
                let mut args: Vec<BasicMetadataValueEnum> = vec![state.into()];
                if !method.params.is_empty() {
                    // Pass data_field as Value* pointer directly — handlers expect Value*
                    args.push(data_field.into());
                }
                let _ = self.builder.build_call(target_fn, &args, "call_msg_fn");
            }
            self.builder.build_unconditional_branch(done_bb).unwrap();
            current_bb = next_bb;
        }
        self.builder.position_at_end(current_bb);
        self.builder.build_unconditional_branch(done_bb).unwrap();

        self.builder.position_at_end(done_bb);
        self.builder.build_return(None).unwrap();

        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
    }

    /// Release function for actor handler closure - releases state Map
    fn build_actor_handler_release(&mut self, release_fn: FunctionValue<'ctx>) {
        let saved_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(release_fn, "entry");
        self.builder.position_at_end(entry);
        
        let env_param = release_fn.get_first_param().unwrap().into_pointer_value();
        let is_null = self.builder.build_is_null(env_param, "env_is_null").unwrap();
        
        let exit_block = self.context.append_basic_block(release_fn, "release_exit");
        let body_block = self.context.append_basic_block(release_fn, "release_body");
        self.builder.build_conditional_branch(is_null, exit_block, body_block).unwrap();
        
        self.builder.position_at_end(body_block);
        // Release the state Map
        let state = self.builder.build_pointer_cast(
            env_param,
            self.runtime.value_ptr_type,
            "actor_state",
        ).unwrap();
        self.builder.build_call(
            self.runtime.value_release,
            &[state.into()],
            "",
        ).unwrap();
        self.builder.build_unconditional_branch(exit_block).unwrap();
        
        self.builder.position_at_end(exit_block);
        self.builder.build_return(None).unwrap();
        
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
    }

    /// Build constructor for non-actor stores.
    /// Creates a Map with all fields initialized to defaults, returns the Map.
    fn build_store_constructor(&mut self, store: &crate::ast::StoreDefinition) -> Result<(), Diagnostic> {
        let constructor_name = format!("make_{}", store.name);
        let ctor_fn = *self.functions.get(&constructor_name).unwrap();
        let entry = self.context.append_basic_block(ctor_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();

        // Create store as a Map with fields
        let null_entries = self.runtime.map_entry_type
            .ptr_type(AddressSpace::default())
            .const_null();
        let zero_len = self.usize_type.const_zero();
        let store_map = self.call_runtime_ptr(
            self.runtime.make_map,
            &[null_entries.into(), zero_len.into()],
            "store_data",
        );
        
        // Create a dummy context for field evaluation
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: ctor_fn,
            loop_stack: Vec::new(),
        };
        
        // Set __type__ field so we know what store type this is for method dispatch
        let type_key = self.emit_string_literal("__type__");
        let type_value = self.emit_string_literal(&store.name);
        self.call_runtime_ptr(
            self.runtime.map_set,
            &[store_map.into(), type_key.into(), type_value.into()],
            "set_type",
        );
        
        // Initialize each field
        for field in &store.fields {
            let key = self.emit_string_literal(&field.name);
            let value = if let Some(default) = &field.default {
                let val = self.emit_expression(&mut ctx, default)?;
                // For reference fields, retain the initial value
                if field.is_reference {
                    self.call_runtime_void(self.runtime.value_retain, &[val.into()], "retain_ref_field");
                }
                val
            } else if field.is_reference {
                // Reference fields default to unit (null reference)
                self.call_runtime_ptr(self.runtime.make_unit, &[], "null_ref")
            } else {
                // Value fields default to 0
                self.wrap_number(self.f64_type.const_float(0.0))
            };
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[store_map.into(), key.into(), value.into()],
                "set_field",
            );
        }

        self.builder.build_return(Some(&store_map)).unwrap();
        Ok(())
    }

    /// Build body for a store method.
    /// Store methods have self (store Map) as hidden first parameter.
    fn build_store_method_body(
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
        };

        // First param is the store (Map), inject as `self`
        let store_ptr = llvm_fn.get_nth_param(0).unwrap().into_pointer_value();
        self.store_variable(&mut ctx, "self", store_ptr);

        // Remaining params are CoralValue* pointers (not f64)
        for (i, param_ast) in function.params.iter().enumerate() {
            let param = llvm_fn.get_nth_param((i + 1) as u32).unwrap();
            let value_ptr = param.into_pointer_value();
            self.store_variable(&mut ctx, &param_ast.name, value_ptr);
        }

        let block_value = self.emit_block(&mut ctx, &function.body)?;
        // Return the value directly as ptr (CoralValue*) instead of converting to f64
        self.builder.build_return(Some(&block_value)).unwrap();
        Ok(())
    }

    fn boolean_to_int(&self, value: bool) -> IntValue<'ctx> {
        self.bool_type.const_int(if value { 1 } else { 0 }, false)
    }
}

struct FunctionContext<'ctx> {
    variables: HashMap<String, PointerValue<'ctx>>,
    /// Stack-allocated slots for variables (alloca Value**) that support mutation/rebinding in loops.
    variable_allocas: HashMap<String, PointerValue<'ctx>>,
    function: FunctionValue<'ctx>,
    /// Stack of (loop_header_bb, loop_exit_bb) for break/continue support
    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
}
