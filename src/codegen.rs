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
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FloatType, FunctionType, IntType, PointerType, StructType};
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
        for function in &model.functions {
            let llvm_name = if function.name == "main" {
                "__user_main"
            } else {
                &function.name
            };
            let fn_type = self.f64_type.fn_type(
                &vec![self.f64_type.into(); function.params.len()],
                false,
            );
            let llvm_fn = self.module.add_function(llvm_name, fn_type, None);
            self.functions.insert(function.name.clone(), llvm_fn);
        }
        // Handle actor send and self
        for store in &model.stores {
            if store.is_actor {
                let constructor_name = format!("make_{}", store.name);
                let ctor_type = self.runtime.value_ptr_type.fn_type(&[], false);
                let ctor_fn = self.module.add_function(&constructor_name, ctor_type, None);
                self.functions.insert(constructor_name, ctor_fn);
                // Declare message handler functions for each @method
                for method in &store.methods {
                    if method.kind == FunctionKind::ActorMessage {
                        let mangled = format!("{}_{}", store.name, method.name);
                        let fn_type = self.f64_type.fn_type(
                            &vec![self.f64_type.into(); method.params.len()],
                            false,
                        );
                        let llvm_fn = self.module.add_function(&mangled, fn_type, None);
                        self.functions.insert(mangled, llvm_fn);
                    }
                }
            }
        }
        self.build_global_initializer(&model.globals)?;
        
        for function in &model.functions {
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
                            self.build_function_body(method, *llvm_fn)?;
                        }
                    }
                }
            }
        }
        
        // Generate actor constructor bodies
        for store in &model.stores {
            if store.is_actor {
                self.build_actor_constructor(store)?;
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
            function: llvm_fn,
        };

        for (param, param_ast) in llvm_fn
            .get_param_iter()
            .zip(function.params.iter())
        {
            let number_ptr = self.wrap_number(param.into_float_value());
            ctx.variables.insert(param_ast.name.clone(), number_ptr);
        }

        let block_value = self.emit_block(&mut ctx, &function.body)?;
        let return_value = self.value_to_number(block_value);
        self.builder.build_return(Some(&return_value)).unwrap();
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
                    let numeric = self.value_to_number(value);
                    self.builder.build_return(Some(&numeric)).unwrap();
                    return Ok(self.wrap_number(self.f64_type.const_float(0.0)));
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
                        let mut arg_values = Vec::new();
                        for arg in args {
                            let value = self.emit_expression(ctx, arg)?;
                            arg_values.push(self.value_to_number(value));
                        }
                        let metadata_args: Vec<BasicMetadataValueEnum> =
                            arg_values.iter().map(|v| (*v).into()).collect();
                        let call = self
                            .builder
                            .build_call(function, &metadata_args, "call")
                            .unwrap();
                        let value = call
                            .try_as_basic_value()
                            .left()
                            .ok_or_else(|| Diagnostic::new("call produced no value", expr.span()))?
                            .into_float_value();
                        Ok(self.wrap_number(value))
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
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.emit_ternary(ctx, condition, then_branch, else_branch),
            Expression::Match(match_expr) => self.emit_match(ctx, match_expr),
            Expression::Unit => Ok(self.wrap_unit()),
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
                let seed_meta: BasicMetadataValueEnum<'ctx> = if args.len() == 1 {
                    seed_arg.into()
                } else {
                    seed_arg.into()
                };
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
            _ => Err(Diagnostic::new(
                format!("method `{property}` not supported yet"),
                span,
            )),
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
            let equals_ptr = self.call_runtime_ptr(self.runtime.value_equals, args, "value_equals");
            let equals_bool = self.value_to_bool(equals_ptr);
            let inverted = self.builder.build_not(equals_bool, "neq").unwrap();
            return Ok(self.wrap_bool(inverted));
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
            (&right_bool as &dyn BasicValue<'ctx>, rhs_bb),
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
            if let MatchPattern::Identifier(name) = &arm.pattern {
                self.call_runtime_void(self.runtime.value_retain, &[match_value.into()], "retain_match_binding");
                ctx.variables.insert(name.clone(), match_value);
            }
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
        _ctx: &mut FunctionContext<'ctx>,
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
            MatchPattern::List(items) => {
                let list_lit = self.emit_list_literal(_ctx, items)?;
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
        }
    }

    fn load_variable(
        &mut self,
        ctx: &FunctionContext<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
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
        ctx.variables.insert(name.to_string(), value);
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

    fn emit_string_literal(&mut self, literal: &str) -> PointerValue<'ctx> {
        let global = if let Some(global) = self.string_pool.get(literal) {
            *global
        } else {
            let name = format!("str_{}", self.string_pool.len());
            let gv = self
                .builder
                .build_global_string_ptr(literal, &name)
                .unwrap();
            self.string_pool.insert(literal.to_string(), gv);
            gv
        };
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
            if let Some(value) = ctx.variables.get(name) {
                capture_values.push(*value);
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
            _ => {
                // Check if it's an actor constructor
                if name.starts_with("make_") && self.functions.contains_key(name) {
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
        let available: HashSet<String> = ctx.variables.keys().cloned().collect();
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
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::Bool(_, _)
            | Expression::String(_, _)
            | Expression::TaxonomyPath { .. }
            | Expression::Placeholder(_, _)
            | Expression::InlineAsm { .. }
            | Expression::PtrLoad { .. }
            | Expression::Unsafe { .. }
            | Expression::Unit => {}
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
            function: invoke_fn,
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
                    lambda_ctx.variables.insert(name.clone(), value);
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
            lambda_ctx.variables.insert(param.name.clone(), arg_value);
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
            function: init_fn,
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

        // Build message handler closure
        // For now, we'll create a dummy closure that ignores messages
        // Real implementation: match on message fields and call @message methods
        let handler_fn_name = format!("__{}_handler", store.name);
        let handler_fn_type = self.context.void_type().fn_type(
            &[
                self.runtime.value_ptr_type.into(), // self (actor handle)
                self.runtime.value_ptr_type.into(), // message
            ],
            false,
        );
        let handler_fn = self.module.add_function(&handler_fn_name, handler_fn_type, None);
        let handler_entry = self.context.append_basic_block(handler_fn, "entry");
        self.builder.position_at_end(handler_entry);
        // Extract message name/data and dispatch to @message methods by string match
        let name_key = self.emit_string_literal("name");
        let data_key = self.emit_string_literal("data");
        let msg_param = handler_fn.get_nth_param(1).unwrap().into_pointer_value();
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

        let mut current_bb = handler_entry;
        let done_bb = self.context.append_basic_block(handler_fn, "msg_done");
        for method in &store.methods {
            if method.kind != crate::ast::FunctionKind::ActorMessage {
                continue;
            }
            let match_bb = self.context.append_basic_block(handler_fn, &format!("msg_{}_match", method.name));
            let next_bb = self.context.append_basic_block(handler_fn, &format!("msg_{}_next", method.name));

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
                let mut args = Vec::new();
                if method.params.len() == 1 {
                    // Pass payload as single argument when method expects one parameter
                    args.push(self.value_to_number(data_field));
                }
                let meta_args: Vec<BasicMetadataValueEnum> = args.iter().map(|v| (*v).into()).collect();
                let _ = self.builder.build_call(target_fn, &meta_args, "call_msg_fn");
            }
            self.builder.build_unconditional_branch(done_bb).unwrap();
            current_bb = next_bb;
        }

        self.builder.position_at_end(current_bb);
        self.builder.build_unconditional_branch(done_bb).unwrap();
        self.builder.position_at_end(done_bb);
        self.builder.build_return(None).unwrap();

        // Back to constructor: wrap handler as closure and spawn
        self.builder.position_at_end(entry);
        let null_env = self.runtime.value_ptr_type.const_null();
        let handler_closure = self.call_runtime_ptr(
            self.runtime.make_closure,
            &[
                handler_fn.as_global_value().as_pointer_value().into(),
                null_env.into(),
                self.runtime.value_ptr_type.const_null().into(), // release_fn = null (no captured env)
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

    fn boolean_to_int(&self, value: bool) -> IntValue<'ctx> {
        self.bool_type.const_int(if value { 1 } else { 0 }, false)
    }
}

struct FunctionContext<'ctx> {
    variables: HashMap<String, PointerValue<'ctx>>,
    function: FunctionValue<'ctx>,
}

#[allow(dead_code)]
struct RuntimeBindings<'ctx> {
    value_ptr_type: PointerType<'ctx>,
    make_number: FunctionValue<'ctx>,
    make_bool: FunctionValue<'ctx>,
    make_string: FunctionValue<'ctx>,
    make_bytes: FunctionValue<'ctx>,
    make_unit: FunctionValue<'ctx>,
    make_list: FunctionValue<'ctx>,
    make_list_hinted: FunctionValue<'ctx>,
    make_map: FunctionValue<'ctx>,
    make_map_hinted: FunctionValue<'ctx>,
    value_as_number: FunctionValue<'ctx>,
    value_as_bool: FunctionValue<'ctx>,
    value_add: FunctionValue<'ctx>,
    value_equals: FunctionValue<'ctx>,
    value_hash: FunctionValue<'ctx>,
    value_bitand: FunctionValue<'ctx>,
    value_bitor: FunctionValue<'ctx>,
    value_bitxor: FunctionValue<'ctx>,
    value_bitnot: FunctionValue<'ctx>,
    value_shift_left: FunctionValue<'ctx>,
    value_shift_right: FunctionValue<'ctx>,
    value_iter: FunctionValue<'ctx>,
    list_push: FunctionValue<'ctx>,
    list_get: FunctionValue<'ctx>,
    list_pop: FunctionValue<'ctx>,
    list_iter: FunctionValue<'ctx>,
    list_iter_next: FunctionValue<'ctx>,
    list_map: FunctionValue<'ctx>,
    list_filter: FunctionValue<'ctx>,
    list_reduce: FunctionValue<'ctx>,
    map_get: FunctionValue<'ctx>,
    map_set: FunctionValue<'ctx>,
    map_length: FunctionValue<'ctx>,
    map_keys: FunctionValue<'ctx>,
    map_iter: FunctionValue<'ctx>,
    map_iter_next: FunctionValue<'ctx>,
    value_length: FunctionValue<'ctx>,
    map_entry_type: StructType<'ctx>,
    make_closure: FunctionValue<'ctx>,
    closure_invoke: FunctionValue<'ctx>,
    log: FunctionValue<'ctx>,
    fs_read: FunctionValue<'ctx>,
    fs_write: FunctionValue<'ctx>,
    fs_exists: FunctionValue<'ctx>,
    value_retain: FunctionValue<'ctx>,
    value_release: FunctionValue<'ctx>,
    heap_alloc: FunctionValue<'ctx>,
    heap_free: FunctionValue<'ctx>,
    actor_spawn: FunctionValue<'ctx>,
    actor_send: FunctionValue<'ctx>,
    actor_stop: FunctionValue<'ctx>,
    actor_self: FunctionValue<'ctx>,
    closure_invoke_type: FunctionType<'ctx>,
    closure_release_type: FunctionType<'ctx>,
}

impl<'ctx> RuntimeBindings<'ctx> {
    fn declare(context: &'ctx Context, module: &Module<'ctx>) -> Self {
        let i8_type = context.i8_type();
        let i16_type = context.i16_type();
        let i32_type = context.i32_type();
        let payload = i8_type.array_type(16);
        let i64_type = context.i64_type();
        let value_type = context.struct_type(
            &[
                i8_type.into(),
                i8_type.into(),
                i16_type.into(),
                i64_type.into(),
                i32_type.into(),
                i32_type.into(),
                payload.into(),
            ],
            false,
        );
        let value_ptr_type = value_type.ptr_type(AddressSpace::default());
        let f64_type = context.f64_type();
        let usize_type = context.i64_type();
        let i8_ptr = i8_type.ptr_type(AddressSpace::default());
        let map_entry_type = context.struct_type(
            &[value_ptr_type.into(), value_ptr_type.into()],
            false,
        );
        let map_entry_ptr_type = map_entry_type.ptr_type(AddressSpace::default());
        let value_ptr_ptr_type = value_ptr_type.ptr_type(AddressSpace::default());
        let closure_invoke_type = context.void_type().fn_type(
            &[
                i8_ptr.into(),
                value_ptr_ptr_type.into(),
                usize_type.into(),
                value_ptr_ptr_type.into(),
            ],
            false,
        );
        let closure_release_type = context
            .void_type()
            .fn_type(&[i8_ptr.into()], false);

        let make_number = module.add_function(
            "coral_make_number",
            value_ptr_type.fn_type(&[f64_type.into()], false),
            None,
        );
        let make_bool = module.add_function(
            "coral_make_bool",
            value_ptr_type.fn_type(&[i8_type.into()], false),
            None,
        );
        let make_string = module.add_function(
            "coral_make_string",
            value_ptr_type.fn_type(&[i8_ptr.into(), usize_type.into()], false),
            None,
        );
        let make_bytes = module.add_function(
            "coral_make_bytes",
            value_ptr_type.fn_type(&[i8_ptr.into(), usize_type.into()], false),
            None,
        );
        let make_unit = module.add_function(
            "coral_make_unit",
            value_ptr_type.fn_type(&[], false),
            None,
        );
        let make_list = module.add_function(
            "coral_make_list",
            value_ptr_type.fn_type(
                &[value_ptr_ptr_type.into(), usize_type.into()],
                false,
            ),
            None,
        );
        let make_list_hinted = module.add_function(
            "coral_make_list_hinted",
            value_ptr_type.fn_type(
                &[value_ptr_ptr_type.into(), usize_type.into(), i8_type.into()],
                false,
            ),
            None,
        );
        let make_map = module.add_function(
            "coral_make_map",
            value_ptr_type.fn_type(&[map_entry_ptr_type.into(), usize_type.into()], false),
            None,
        );
        let make_map_hinted = module.add_function(
            "coral_make_map_hinted",
            value_ptr_type.fn_type(
                &[map_entry_ptr_type.into(), usize_type.into(), i8_type.into()],
                false,
            ),
            None,
        );

        let list_push = module.add_function(
            "coral_list_push",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_get = module.add_function(
            "coral_list_get",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_pop = module.add_function(
            "coral_list_pop",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_iter = module.add_function(
            "coral_list_iter",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_iter_next = module.add_function(
            "coral_list_iter_next",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_map = module.add_function(
            "coral_list_map",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_filter = module.add_function(
            "coral_list_filter",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_reduce = module.add_function(
            "coral_list_reduce",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let map_get = module.add_function(
            "coral_map_get",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let map_set = module.add_function(
            "coral_map_set",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let map_length = module.add_function(
            "coral_map_length",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_keys = module.add_function(
            "coral_map_keys",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_iter = module.add_function(
            "coral_map_iter",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_iter_next = module.add_function(
            "coral_map_iter_next",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_length = module.add_function(
            "coral_value_length",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_iter = module.add_function(
            "coral_value_iter",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_as_number = module.add_function(
            "coral_value_as_number",
            f64_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_as_bool = module.add_function(
            "coral_value_as_bool",
            i8_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_add = module.add_function(
            "coral_value_add",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_equals = module.add_function(
            "coral_value_equals",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_hash = module.add_function(
            "coral_value_hash",
            i64_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_bitand = module.add_function(
            "coral_value_bitand",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_bitor = module.add_function(
            "coral_value_bitor",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_bitxor = module.add_function(
            "coral_value_bitxor",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_bitnot = module.add_function(
            "coral_value_bitnot",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_shift_left = module.add_function(
            "coral_value_shift_left",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_shift_right = module.add_function(
            "coral_value_shift_right",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let log = module.add_function(
            "coral_log",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_read = module.add_function(
            "coral_fs_read",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_write = module.add_function(
            "coral_fs_write",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let fs_exists = module.add_function(
            "coral_fs_exists",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let make_closure = module.add_function(
            "coral_make_closure",
            value_ptr_type.fn_type(
                &[
                    closure_invoke_type
                        .ptr_type(AddressSpace::default())
                        .into(),
                    closure_release_type
                        .ptr_type(AddressSpace::default())
                        .into(),
                    i8_ptr.into(),
                ],
                false,
            ),
            None,
        );
        let closure_invoke = module.add_function(
            "coral_closure_invoke",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_ptr_type.into(), usize_type.into()],
                false,
            ),
            None,
        );
        let value_retain = module.add_function(
            "coral_value_retain",
            context.void_type().fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_release = module.add_function(
            "coral_value_release",
            context.void_type().fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let heap_alloc = module.add_function(
            "coral_heap_alloc",
            i8_ptr.fn_type(&[usize_type.into()], false),
            None,
        );
        let heap_free = module.add_function(
            "coral_heap_free",
            context.void_type().fn_type(&[i8_ptr.into()], false),
            None,
        );
        let actor_spawn = module.add_function(
            "coral_actor_spawn",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_send = module.add_function(
            "coral_actor_send",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let actor_stop = module.add_function(
            "coral_actor_stop",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_self = module.add_function(
            "coral_actor_self",
            value_ptr_type.fn_type(&[], false),
            None,
        );

        Self {
            value_ptr_type,
            make_number,
            make_bool,
            make_string,
            make_bytes,
            make_unit,
            make_list,
            make_list_hinted,
            value_as_number,
            value_as_bool,
            value_add,
            value_equals,
            value_hash,
            value_bitand,
            value_bitor,
            value_bitxor,
            value_bitnot,
            value_shift_left,
            value_shift_right,
            value_iter,
            list_push,
            list_get,
            list_pop,
            list_iter,
            list_iter_next,
            list_map,
            list_filter,
            list_reduce,
            map_get,
            map_set,
            map_length,
            map_keys,
            map_iter,
            map_iter_next,
            value_length,
            make_map,
            make_map_hinted,
            map_entry_type,
            make_closure,
            closure_invoke,
            log,
            fs_read,
            fs_write,
            fs_exists,
            value_retain,
            value_release,
            heap_alloc,
            heap_free,
            actor_spawn,
            actor_send,
            actor_stop,
            actor_self,
            closure_invoke_type,
            closure_release_type,
        }
    }
}