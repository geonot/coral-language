//! Lambda, closure, and enum constructor codegen for Coral.
//!
//! Contains lambda emission, closure environment building,
//! capture analysis, and enum/ADT constructor generation.

use super::*;

impl<'ctx> CodeGenerator<'ctx> {
    pub(super) fn emit_lambda(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        params: &[Parameter],
        body: &Block,
        span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
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
                .map(|_| self.runtime.value_i64_type.into())
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

        // C3.2: Mark small lambda invoke functions as alwaysinline so LLVM can
        // inline them at call sites, especially inside map/filter/reduce loops.
        {
            let stmt_count = body.statements.len()
                + if body.value.is_some() { 1 } else { 0 };
            if stmt_count <= 5 {
                let kind_id = Attribute::get_named_enum_kind_id("alwaysinline");
                let attr = self.context.create_enum_attribute(kind_id, 0);
                invoke_fn.add_attribute(AttributeLoc::Function, attr);
            }
        }

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
        let capture_count_val = self.usize_type.const_int(capture_names.len() as u64, false);
        let args = &[invoke_ptr.into(), release_ptr.into(), env_ptr.into(), capture_count_val.into()];
        let closure_ptr = self.call_runtime_ptr(self.runtime.make_closure, args, "make_closure");
        Ok(self.ptr_to_nb(closure_ptr))
    }

    /// Wrap a named function in a closure so it can be used as a first-class value.
    /// Generates a thunk function matching the closure invoke signature that
    /// extracts args from the args array and delegates to the original function.
    pub(super) fn emit_function_as_closure(
        &mut self,
        _ctx: &FunctionContext<'ctx>,
        name: &str,
        target_fn: FunctionValue<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
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
            // Retain before converting: nb_from_handle releases immediates,
            // but args are borrowed from the runtime
            self.call_runtime_void(self.runtime.value_retain, &[arg_val.into()], "retain_borrowed_arg");
            // Convert Value* to NaN-boxed i64 for the target function
            let arg_nb = self.ptr_to_nb(arg_val);
            call_args.push(arg_nb.into());
        }

        // Call the original function (returns NaN-boxed i64)
        let result = self
            .builder
            .build_call(target_fn, &call_args, "thunk_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        // Convert NaN-boxed i64 back to Value* for closure protocol
        let result_ptr = self.nb_to_ptr(result);
        // Store result and return
        self.builder.build_store(out_param, result_ptr).unwrap();
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
        let zero_captures = self.usize_type.const_zero();
        let closure_args = &[invoke_ptr.into(), null_release.into(), null_env.into(), zero_captures.into()];
        let closure_ptr = self.call_runtime_ptr(self.runtime.make_closure, closure_args, "fn_as_closure");
        Ok(self.ptr_to_nb(closure_ptr))
    }

    /// Emit code to construct an enum (ADT) variant value.
    pub(super) fn emit_enum_constructor(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        variant_name: &str,
        args: &[Expression],
    ) -> Result<IntValue<'ctx>, Diagnostic> {
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
                let value_ptr = self.nb_to_ptr(*value);
                temp_array = self
                    .builder
                    .build_insert_value(temp_array, value_ptr, idx as u32, "field")
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
        let tagged_ptr = self.call_runtime_ptr(
            self.runtime.make_tagged,
            &[tag_name_ptr.into(), tag_name_len.into(), fields_ptr.into(), field_count_val.into()],
            "make_tagged",
        );
        Ok(self.ptr_to_nb(tagged_ptr))
    }
    
    /// Emit a nullary enum constructor (no fields, e.g., None)
    pub(super) fn emit_enum_constructor_nullary(
        &mut self,
        variant_name: &str,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
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
        
        let tagged_ptr = self.call_runtime_ptr(
            self.runtime.make_tagged,
            &[tag_name_ptr.into(), tag_name_len.into(), null_ptr.into(), zero.into()],
            "make_tagged_nullary",
        );
        Ok(self.ptr_to_nb(tagged_ptr))
    }

    pub(super) fn emit_closure_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        closure: IntValue<'ctx>,
        args: &[Expression],
    ) -> Result<IntValue<'ctx>, Diagnostic> {
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
                let value_ptr = self.nb_to_ptr(*value);
                temp_array = self
                    .builder
                    .build_insert_value(temp_array, value_ptr, idx as u32, "closure_arg")
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
        let closure_ptr = self.nb_to_ptr(closure);
        let args = &[closure_ptr.into(), args_ptr.into(), len_value.into()];
        let result_ptr = self.call_runtime_ptr(self.runtime.closure_invoke, args, "closure_invoke");
        Ok(self.ptr_to_nb(result_ptr))
    }
    pub(super) fn determine_lambda_captures(
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

    pub(super) fn collect_captures_block(
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
                Statement::ForKV { key_var, value_var, iterable, body, .. } => {
                    self.collect_captures_expr(iterable, available, &mut locals, captures, seen);
                    locals.insert(key_var.clone());
                    locals.insert(value_var.clone());
                    self.collect_captures_block(body, available, &mut locals, captures, seen);
                }
                Statement::ForRange { variable, start, end, step, body, .. } => {
                    self.collect_captures_expr(start, available, &mut locals, captures, seen);
                    self.collect_captures_expr(end, available, &mut locals, captures, seen);
                    if let Some(s) = step {
                        self.collect_captures_expr(s, available, &mut locals, captures, seen);
                    }
                    locals.insert(variable.clone());
                    self.collect_captures_block(body, available, &mut locals, captures, seen);
                }
                Statement::Break(_) | Statement::Continue(_) => {}
                Statement::FieldAssign { target, value, .. } => {
                    self.collect_captures_expr(target, available, &mut locals, captures, seen);
                    self.collect_captures_expr(value, available, &mut locals, captures, seen);
                }
                Statement::PatternBinding { pattern, value, .. } => {
                    self.collect_captures_expr(value, available, &mut locals, captures, seen);
                    collect_pattern_locals(pattern, &mut locals);
                }
            }
        }
        if let Some(value) = &block.value {
            self.collect_captures_expr(value, available, &mut locals, captures, seen);
        }
    }

    pub(super) fn collect_captures_expr(
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
            Expression::Spread(inner, _) =>
                self.collect_captures_expr(inner, available, locals, captures, seen),
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
            Expression::Slice { target, start, end, .. } => {
                self.collect_captures_expr(target, available, locals, captures, seen);
                self.collect_captures_expr(start, available, locals, captures, seen);
                self.collect_captures_expr(end, available, locals, captures, seen);
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
                    // S3.2: Guard expression may capture variables
                    if let Some(guard) = &arm.guard {
                        self.collect_captures_expr(guard, available, locals, captures, seen);
                    }
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
            Expression::ListComprehension { body, iterable, condition, .. } => {
                self.collect_captures_expr(iterable, available, locals, captures, seen);
                self.collect_captures_expr(body, available, locals, captures, seen);
                if let Some(cond) = condition {
                    self.collect_captures_expr(cond, available, locals, captures, seen);
                }
            }
            Expression::MapComprehension { key, value, iterable, condition, .. } => {
                self.collect_captures_expr(iterable, available, locals, captures, seen);
                self.collect_captures_expr(key, available, locals, captures, seen);
                self.collect_captures_expr(value, available, locals, captures, seen);
                if let Some(cond) = condition {
                    self.collect_captures_expr(cond, available, locals, captures, seen);
                }
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

    pub(super) fn build_lambda_invoke(
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
            di_scope: None,
            fn_name: String::new(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
            lambda_out_param: Some(out_param),
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
                            self.runtime.value_i64_type,
                            field_ptr,
                            &format!("capture_load_{}", idx),
                        )
                        .unwrap()
                        .into_int_value();
                    self.store_variable(&mut lambda_ctx, name, value);
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
            let arg_ptr_val = self
                .builder
                .build_load(
                    self.runtime.value_ptr_type,
                    arg_ptr,
                    &format!("lambda_arg_{}", idx),
                )
                .unwrap()
                .into_pointer_value();
            // Retain before converting: nb_from_handle releases immediates,
            // but args are borrowed from the runtime (reduce, closure_invoke, etc.)
            self.call_runtime_void(self.runtime.value_retain, &[arg_ptr_val.into()], "retain_borrowed_arg");
            // Convert Value* to NaN-boxed i64
            let arg_value = self.ptr_to_nb(arg_ptr_val);
            self.store_variable(&mut lambda_ctx, &param.name, arg_value);
        }

    let result = self.emit_block(&mut lambda_ctx, body)?;
        let result_ptr = self.nb_to_ptr(result);
        self.builder.build_store(out_param, result_ptr).unwrap();
        self.builder.build_return(None).unwrap();

        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
    }

    pub(super) fn build_lambda_release(
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
                    self.runtime.value_i64_type,
                    field_ptr,
                    &format!("release_capture_{}", idx),
                )
                .unwrap()
                .into_int_value();
            self.call_nb_void(self.runtime.nb_release, &[value.into()]);
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

    pub(super) fn build_closure_env(
        &mut self,
        env_struct: Option<StructType<'ctx>>,
        capture_values: &[IntValue<'ctx>],
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
                self.call_nb_void(self.runtime.nb_retain, &[(*value).into()]);
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
}

/// Collect variable names introduced by a destructuring pattern into a locals set.
fn collect_pattern_locals(pattern: &crate::ast::MatchPattern, locals: &mut HashSet<String>) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            locals.insert(name.clone());
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            for pat in fields {
                collect_pattern_locals(pat, locals);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            for pat in patterns {
                collect_pattern_locals(pat, locals);
            }
        }
        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                collect_pattern_locals(alt, locals);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            locals.insert(name.clone());
        }
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
    }
}