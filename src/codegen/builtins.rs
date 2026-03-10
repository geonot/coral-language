//! Builtin function dispatch for Coral runtime.
//!
//! Contains emit_builtin_call (the largest function in codegen),
//! plus member expression/call dispatch and IO calls.

use super::*;

impl<'ctx> CodeGenerator<'ctx> {
    pub(super) fn build_message_value(
        &mut self,
        name_value: IntValue<'ctx>,
        payload_value: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let entry_ptr_type = self.runtime.map_entry_type.ptr_type(AddressSpace::default());
        let array_type = self.runtime.map_entry_type.array_type(2);
        let mut temp_array = array_type.get_undef();

        let name_value = self.nb_to_ptr(name_value);
        let payload_value = self.nb_to_ptr(payload_value);
        let name_key = self.emit_string_literal("name");
        let name_key = self.nb_to_ptr(name_key);
        let data_key = self.emit_string_literal("data");
        let data_key = self.nb_to_ptr(data_key);

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


    pub(super) fn emit_member_expression(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        target: &Expression,
        property: &str,
        _span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        // For 'self' target (store instance), always use map lookup for field access
        if let Expression::Identifier(name, _) = target {
            if name == "self" {
                let target_value = self.emit_expression(ctx, target)?;
                let key_value = self.emit_string_literal(property);
                return Ok(self.call_bridged(self.runtime.map_get, &[target_value, key_value], "map_get_property"));
            }
        }
        let target_value = self.emit_expression(ctx, target)?;
        match property {
            "length" | "count" if !self.store_field_names.contains(property) => Ok(self.call_bridged(self.runtime.value_length, &[target_value], "value_length")),
            "length" | "count" => {
                // If any store defines a field with this name, use field_or_length
                // to dispatch at runtime: maps/stores → field lookup, else → length.
                let key_value = self.emit_string_literal(property);
                Ok(self.call_bridged(self.runtime.field_or_length, &[target_value, key_value], "field_or_length"))
            }
            "size" => Ok(self.call_bridged(self.runtime.map_length, &[target_value], "map_length")),
            "err" => {
                // x.err - returns true if x is an error value
                let target_ptr = self.nb_to_ptr(target_value);
                let is_err = self.builder
                    .build_call(self.runtime.is_err, &[target_ptr.into()], "is_err_check")
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
                Ok(self.call_bridged(self.runtime.map_get, &[target_value, key_value], "map_get_property"))
            }
        }
    }


    pub(super) fn emit_member_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        target: &Expression,
        property: &str,
        args: &[Expression],
        span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        if let Expression::Identifier(namespace, _) = target {
            if namespace == "io" {
                return self.emit_io_call(ctx, property, args, span);
            }
            // CC3.2: Resolve module-qualified function calls (e.g., math.sin())
            if let Some(exports) = self.module_exports.get(namespace.as_str()).cloned() {
                if exports.contains(&property.to_string()) {
                    // The function is exported by this module — call it by its unqualified name
                    if let Some(&function) = self.functions.get(property) {
                        let mut arg_values = Vec::new();
                        for arg in args {
                            let saved_tail = ctx.in_tail_position;
                            ctx.in_tail_position = false;
                            let value = self.emit_expression(ctx, arg)?;
                            ctx.in_tail_position = saved_tail;
                            arg_values.push(value);
                        }
                        let call_args: Vec<_> = arg_values.iter().map(|v| (*v).into()).collect();
                        let result = self.builder.build_call(function, &call_args, "modcall").unwrap();
                        return Ok(result.try_as_basic_value().left().unwrap().into_int_value());
                    }
                    // Might be a builtin function — emit as a regular call
                    let call_expr = Expression::Call {
                        callee: Box::new(Expression::Identifier(property.to_string(), span)),
                        args: args.to_vec(),
                        arg_names: vec![],
                        span,
                    };
                    return self.emit_expression(ctx, &call_expr);
                }
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
                Ok(self.call_bridged(self.runtime.value_equals, &[target_value, arg_value], "value_equals"))
            }
            // x.not_equals(y) - value inequality comparison
            "not_equals" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("not_equals expects exactly one argument", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                let arg_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_bridged(self.runtime.value_not_equals, &[target_value, arg_value], "value_not_equals"))
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
                Ok(self.call_bridged(self.runtime.value_iter, &[target_value], "value_iter"))
            }
            "keys" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("keys does not take arguments", span));
                }
                let map_value = self.emit_expression(ctx, target)?;
                Ok(self.call_bridged(self.runtime.map_keys, &[map_value], "map_keys"))
            }
            "map" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list.map expects a single function", span));
                }
                // C3.2: Inline lambda body directly into a map loop when the
                // argument is a lambda expression, avoiding closure allocation
                // and indirect calls through the runtime.
                if let Expression::Lambda { params, body, .. } = &args[0] {
                    if params.len() == 1 {
                        return self.emit_inline_map(ctx, target, &params[0].name, body, span);
                    }
                }
                let list_value = self.emit_expression(ctx, target)?;
                let func_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_bridged(self.runtime.list_map, &[list_value, func_value], "list_map"))
            }
            "filter" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list.filter expects a predicate", span));
                }
                // C3.2: Inline lambda body for filter as well
                if let Expression::Lambda { params, body, .. } = &args[0] {
                    if params.len() == 1 {
                        return self.emit_inline_filter(ctx, target, &params[0].name, body, span);
                    }
                }
                let list_value = self.emit_expression(ctx, target)?;
                let func_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_bridged(self.runtime.list_filter, &[list_value, func_value], "list_filter"))
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
                    (self.wrap_none(), self.emit_expression(ctx, &args[0])?)
                } else {
                    (self.emit_expression(ctx, &args[0])?, self.emit_expression(ctx, &args[1])?)
                };
                Ok(self.call_bridged(
                    self.runtime.list_reduce,
                    &[list_value, seed_arg, func_value],
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
                Ok(self.call_bridged(self.runtime.list_push, &[list_value, arg_value], "list_push"))
            }
            "pop" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new(
                        "list.pop does not take arguments",
                        span,
                    ));
                }
                let list_value = self.emit_expression(ctx, target)?;
                Ok(self.call_bridged(self.runtime.list_pop, &[list_value], "list_pop"))
            }
            "get" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        ".get() expects exactly one argument",
                        span,
                    ));
                }
                let target_value = self.emit_expression(ctx, target)?;
                let key_value = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_bridged(self.runtime.value_get, &[target_value, key_value], "value_get_method"))
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
                                let old_value = self.call_bridged(
                                    self.runtime.map_get,
                                    &[map_value, key_value],
                                    "get_old_ref",
                                );
                                // Retain new value
                                let new_ptr = self.nb_to_ptr(new_value);
                                self.call_runtime_void(self.runtime.value_retain, &[new_ptr.into()], "retain_new_ref");
                                // Set the field
                                let result = self.call_bridged(
                                    self.runtime.map_set,
                                    &[map_value, key_value, new_value],
                                    "map_set_method",
                                );
                                // Release old value
                                let old_ptr = self.nb_to_ptr(old_value);
                                self.call_runtime_void(self.runtime.value_release, &[old_ptr.into()], "release_old_ref");
                                return Ok(result);
                            }
                        }
                    }
                }
                
                Ok(self.call_bridged(self.runtime.map_set, &[map_value, key_value, new_value], "map_set_method"))
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
                Ok(self.call_bridged(self.runtime.list_get, &[list_value, index_value], "list_get"))
            }
            "length" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("length does not take arguments", span));
                }
                let target_value = self.emit_expression(ctx, target)?;
                Ok(self.call_bridged(self.runtime.value_length, &[target_value], "value_length"))
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
                    // Call the store method (returns i64 NaN-boxed value)
                    let result = self.builder.build_call(store_method, &call_args, "store_method_call")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();
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

    pub(super) fn emit_io_call(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        method: &str,
        args: &[Expression],
        span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match method {
            "read" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("io.read expects path", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_bridged(self.runtime.fs_read, &[path], "io_read"))
            }
            "write" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("io.write expects path and data", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                let data = self.emit_expression(ctx, &args[1])?;
                Ok(self.call_bridged(self.runtime.fs_write, &[path, data], "io_write"))
            }
            "exists" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("io.exists expects path", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(self.call_bridged(self.runtime.fs_exists, &[path], "io_exists"))
            }
            _ => Err(Diagnostic::new(
                format!("namespace `io` has no method `{method}`"),
                span,
            )),
        }
    }

    pub(super) fn emit_builtin_call(
        &mut self,
        name: &str,
        args: &[Expression],
        ctx: &mut FunctionContext<'ctx>,
        span: Span,
    ) -> Result<Option<IntValue<'ctx>>, Diagnostic> {
        match name {
            "log" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "log expects exactly one argument",
                        span,
                    ));
                }
                let value = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.log, &[value], "log_call")))
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
                Ok(Some(self.call_bridged(self.runtime.value_add, &[a, b], "concat_call")))
            }
            "fs_read" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "fs_read expects exactly one argument",
                        span,
                    ));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_read, &[path], "fs_read_call")))
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
                Ok(Some(self.call_bridged(self.runtime.fs_write, &[path, data], "fs_write_call")))
            }
            "fs_exists" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "fs_exists expects exactly one argument",
                        span,
                    ));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_exists, &[path], "fs_exists_call")))
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
                Ok(Some(self.call_bridged(func, &[lhs, rhs], "bit_call")))
            }
            "bit_not" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new(
                        "bit_not expects one argument",
                        span,
                    ));
                }
                let value = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.value_bitnot, &[value], "bit_not_call")))
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
                Ok(Some(self.call_bridged(self.runtime.actor_send, &[actor, message], "actor_send_builtin")))
            }
            "actor_self" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("actor_self expects no arguments", span));
                }
                Ok(Some(self.call_bridged(self.runtime.actor_self, &[], "actor_self_builtin")))
            }
            // String operations
            "string_slice" | "slice" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("string_slice expects string, start, end", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let start = self.emit_expression(ctx, &args[1])?;
                let end = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_bridged(self.runtime.string_slice, &[s, start, end], "string_slice_call")))
            }
            "string_char_at" | "char_at" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_char_at expects string, index", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let idx = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_char_at, &[s, idx], "string_char_at_call")))
            }
            "string_index_of" | "index_of" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_index_of expects haystack, needle", span));
                }
                let haystack = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_index_of, &[haystack, needle], "string_index_of_call")))
            }
            "string_split" | "split" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_split expects string, delimiter", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let delim = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_split, &[s, delim], "string_split_call")))
            }
            "string_to_chars" | "chars" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_to_chars expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_to_chars, &[s], "string_to_chars_call")))
            }
            "string_starts_with" | "starts_with" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_starts_with expects string, prefix", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let prefix = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_starts_with, &[s, prefix], "string_starts_with_call")))
            }
            "string_ends_with" | "ends_with" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_ends_with expects string, suffix", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let suffix = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_ends_with, &[s, suffix], "string_ends_with_call")))
            }
            "string_trim" | "trim" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_trim expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_trim, &[s], "string_trim_call")))
            }
            "string_to_upper" | "to_upper" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_to_upper expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_to_upper, &[s], "string_to_upper_call")))
            }
            "string_to_lower" | "to_lower" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_to_lower expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_to_lower, &[s], "string_to_lower_call")))
            }
            "string_replace" | "replace" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("string_replace expects string, old, new", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let old = self.emit_expression(ctx, &args[1])?;
                let new = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_bridged(self.runtime.string_replace, &[s, old, new], "string_replace_call")))
            }
            "string_contains" | "contains" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_contains expects haystack, needle", span));
                }
                let haystack = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_contains, &[haystack, needle], "string_contains_call")))
            }
            "string_parse_number" | "parse_number" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_parse_number expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_parse_number, &[s], "string_parse_number_call")))
            }
            "string_length" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_length expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.value_length, &[s], "string_length_call")))
            }
            "number_to_string" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("number_to_string expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.number_to_string, &[n], "number_to_string_call")))
            }
            "to_string" | "value_to_string" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("to_string expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.value_to_string, &[v], "value_to_string_call")))
            }
            // Math functions - unary
            "abs" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("abs expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_abs, &[n], "math_abs_call")))
            }
            "sqrt" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sqrt expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_sqrt, &[n], "math_sqrt_call")))
            }
            "floor" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("floor expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_floor, &[n], "math_floor_call")))
            }
            "ceil" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("ceil expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_ceil, &[n], "math_ceil_call")))
            }
            "round" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("round expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_round, &[n], "math_round_call")))
            }
            "sin" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sin expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_sin, &[n], "math_sin_call")))
            }
            "cos" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("cos expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_cos, &[n], "math_cos_call")))
            }
            "tan" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("tan expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_tan, &[n], "math_tan_call")))
            }
            "ln" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("ln expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_ln, &[n], "math_ln_call")))
            }
            "log10" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("log10 expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_log10, &[n], "math_log10_call")))
            }
            "exp" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("exp expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_exp, &[n], "math_exp_call")))
            }
            "asin" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("asin expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_asin, &[n], "math_asin_call")))
            }
            "acos" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("acos expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_acos, &[n], "math_acos_call")))
            }
            "atan" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("atan expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_atan, &[n], "math_atan_call")))
            }
            "sinh" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sinh expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_sinh, &[n], "math_sinh_call")))
            }
            "cosh" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("cosh expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_cosh, &[n], "math_cosh_call")))
            }
            "tanh" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("tanh expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_tanh, &[n], "math_tanh_call")))
            }
            "trunc" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("trunc expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_trunc, &[n], "math_trunc_call")))
            }
            "sign" | "signum" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sign expects one argument", span));
                }
                let n = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.math_sign, &[n], "math_sign_call")))
            }
            // Math functions - binary
            "pow" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("pow expects two arguments (base, exponent)", span));
                }
                let base = self.emit_expression(ctx, &args[0])?;
                let exp = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.math_pow, &[base, exp], "math_pow_call")))
            }
            "min" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("min expects two arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.math_min, &[a, b], "math_min_call")))
            }
            "max" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("max expects two arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.math_max, &[a, b], "math_max_call")))
            }
            "atan2" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("atan2 expects two arguments (y, x)", span));
                }
                let y = self.emit_expression(ctx, &args[0])?;
                let x = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.math_atan2, &[y, x], "math_atan2_call")))
            }
            // Process/environment
            "process_args" | "args" => {
                Ok(Some(self.call_bridged(self.runtime.process_args, &[], "process_args_call")))
            }
            "process_exit" | "exit" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("exit expects one argument (exit code)", span));
                }
                let code = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.process_exit, &[code], "process_exit_call")))
            }
            "env_get" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("env_get expects one argument", span));
                }
                let name_val = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.env_get, &[name_val], "env_get_call")))
            }
            "env_set" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("env_set expects two arguments (name, value)", span));
                }
                let name_val = self.emit_expression(ctx, &args[0])?;
                let val = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.env_set, &[name_val, val], "env_set_call")))
            }
            // File I/O extensions
            "fs_append" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("fs_append expects path and data", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                let data = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.fs_append, &[path, data], "fs_append_call")))
            }
            "fs_read_dir" | "read_dir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_read_dir expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_read_dir, &[path], "fs_read_dir_call")))
            }
            "fs_mkdir" | "mkdir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_mkdir expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_mkdir, &[path], "fs_mkdir_call")))
            }
            "fs_delete" | "delete" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_delete expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_delete, &[path], "fs_delete_call")))
            }
            "fs_is_dir" | "is_dir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_is_dir expects one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_is_dir, &[path], "fs_is_dir_call")))
            }
            "stdin_read_line" | "read_line" => {
                Ok(Some(self.call_bridged(self.runtime.stdin_read_line, &[], "stdin_read_line_call")))
            }
            // L2.4: std.io enhancements
            "stderr_write" | "eprint" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("stderr_write expects exactly one argument", span));
                }
                let msg = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.stderr_write, &[msg], "stderr_write_call")))
            }
            "fs_size" | "file_size" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_size expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_size, &[path], "fs_size_call")))
            }
            "fs_rename" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("fs_rename expects two arguments (old, new)", span));
                }
                let old = self.emit_expression(ctx, &args[0])?;
                let new = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.fs_rename, &[old, new], "fs_rename_call")))
            }
            "fs_copy" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("fs_copy expects two arguments (src, dst)", span));
                }
                let src = self.emit_expression(ctx, &args[0])?;
                let dst = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.fs_copy, &[src, dst], "fs_copy_call")))
            }
            "fs_mkdirs" | "make_dirs" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("fs_mkdirs expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.fs_mkdirs, &[path], "fs_mkdirs_call")))
            }
            "fs_temp_dir" | "temp_dir" => {
                Ok(Some(self.call_bridged(self.runtime.fs_temp_dir, &[], "fs_temp_dir_call")))
            }
            // L2.5: std.process enhancements
            "process_exec" | "exec" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("process_exec expects two arguments (cmd, args_list)", span));
                }
                let cmd = self.emit_expression(ctx, &args[0])?;
                let args_val = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.process_exec, &[cmd, args_val], "process_exec_call")))
            }
            "process_cwd" | "cwd" => {
                Ok(Some(self.call_bridged(self.runtime.process_cwd, &[], "process_cwd_call")))
            }
            "process_chdir" | "chdir" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("process_chdir expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.process_chdir, &[path], "process_chdir_call")))
            }
            "process_pid" => {
                Ok(Some(self.call_bridged(self.runtime.process_pid, &[], "process_pid_call")))
            }
            "process_hostname" | "hostname" => {
                Ok(Some(self.call_bridged(self.runtime.process_hostname, &[], "process_hostname_call")))
            }
            // L4.2: std.path operations
            "path_normalize" | "normalize" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("path_normalize expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.path_normalize, &[path], "path_normalize_call")))
            }
            "path_resolve" | "resolve" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("path_resolve expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.path_resolve, &[path], "path_resolve_call")))
            }
            "path_is_absolute" | "is_absolute" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("path_is_absolute expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.path_is_absolute, &[path], "path_is_absolute_call")))
            }
            "path_parent" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("path_parent expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.path_parent, &[path], "path_parent_call")))
            }
            "path_stem" | "stem" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("path_stem expects exactly one argument", span));
                }
                let path = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.path_stem, &[path], "path_stem_call")))
            }
            // List extensions
            "list_contains" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_contains expects list and value", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.list_contains, &[list, needle], "list_contains_call")))
            }
            "list_index_of" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_index_of expects list and value", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let needle = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.list_index_of, &[list, needle], "list_index_of_call")))
            }
            "list_reverse" | "reverse" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list_reverse expects one argument", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.list_reverse, &[list], "list_reverse_call")))
            }
            "list_slice" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("list_slice expects list, start, end", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let start = self.emit_expression(ctx, &args[1])?;
                let end = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_bridged(self.runtime.list_slice, &[list, start, end], "list_slice_call")))
            }
            "list_sort" | "sort" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("list_sort expects one argument", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.list_sort, &[list], "list_sort_call")))
            }
            "list_join" | "join" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_join expects list and separator", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let sep = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.list_join, &[list, sep], "list_join_call")))
            }
            "list_concat" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("list_concat expects two lists", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.list_concat, &[a, b], "list_concat_call")))
            }
            // Map extensions
            "map_keys" | "keys" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("map_keys expects one argument", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.map_keys, &[map], "map_keys_call")))
            }
            "map_remove" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("map_remove expects map and key", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                let key = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.map_remove, &[map, key], "map_remove_call")))
            }
            "map_values" | "values" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("map_values expects one argument", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.map_values, &[map], "map_values_call")))
            }
            "map_entries" | "entries" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("map_entries expects one argument", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.map_entries, &[map], "map_entries_call")))
            }
            "map_has_key" | "has_key" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("map_has_key expects map and key", span));
                }
                let map = self.emit_expression(ctx, &args[0])?;
                let key = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.map_has_key, &[map, key], "map_has_key_call")))
            }
            "map_merge" | "merge" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("map_merge expects two maps", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.map_merge, &[a, b], "map_merge_call")))
            }
            // Bytes extensions
            "bytes_get" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("bytes_get expects bytes and index", span));
                }
                let b = self.emit_expression(ctx, &args[0])?;
                let idx = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_get, &[b, idx], "bytes_get_call")))
            }
            "bytes_from_string" | "to_bytes" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("bytes_from_string expects one argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_from_string, &[s], "bytes_from_string_call")))
            }
            "bytes_to_string" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("bytes_to_string expects one argument", span));
                }
                let b = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_to_string, &[b], "bytes_to_string_call")))
            }
            "bytes_slice" => {
                if args.len() != 3 {
                    return Err(Diagnostic::new("bytes_slice expects bytes, start, end", span));
                }
                let b = self.emit_expression(ctx, &args[0])?;
                let start = self.emit_expression(ctx, &args[1])?;
                let end = self.emit_expression(ctx, &args[2])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_slice_val, &[b, start, end], "bytes_slice_call")))
            }
            // Type reflection
            "type_of" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("type_of expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.type_of, &[v], "type_of_call")))
            }
            // Character operations
            "ord" | "string_ord" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("ord expects one string argument", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_ord, &[s], "ord_call")))
            }
            "chr" | "string_chr" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("chr expects one number argument", span));
                }
                let code = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_chr, &[code], "chr_call")))
            }
            "string_compare" | "strcmp" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_compare expects two string arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_compare, &[a, b], "strcmp_call")))
            }
            // Error checking builtins
            "is_err" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("is_err expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                let v_ptr = self.nb_to_ptr(v);
                let is_err = self.builder
                    .build_call(self.runtime.is_err, &[v_ptr.into()], "is_err_check")
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
                let v_ptr = self.nb_to_ptr(v);
                let is_ok = self.builder
                    .build_call(self.runtime.is_ok, &[v_ptr.into()], "is_ok_check")
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
                let v_ptr = self.nb_to_ptr(v);
                let is_absent = self.builder
                    .build_call(self.runtime.is_absent, &[v_ptr.into()], "is_absent_check")
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
                Ok(Some(self.call_bridged(self.runtime.error_name, &[v], "error_name_call")))
            }
            "error_code" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("error_code expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                let v_ptr = self.nb_to_ptr(v);
                let code_i32 = self.builder
                    .build_call(self.runtime.error_code, &[v_ptr.into()], "error_code_call")
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
            // JSON operations (SL-8)
            "json_parse" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("json_parse expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.json_parse, &[v], "json_parse_call")))
            }
            "json_serialize" | "json_stringify" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("json_serialize expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.json_serialize, &[v], "json_serialize_call")))
            }
            "json_serialize_pretty" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("json_serialize_pretty expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.json_serialize_pretty, &[v], "json_pretty_call")))
            }
            // Random operations (L2.1)
            "random" => {
                Ok(Some(self.call_bridged(self.runtime.random, &[], "random_call")))
            }
            "random_int" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("random_int expects two arguments (min, max)", span));
                }
                let min_v = self.emit_expression(ctx, &args[0])?;
                let max_v = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.random_int, &[min_v, max_v], "random_int_call")))
            }
            "random_seed" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("random_seed expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.random_seed, &[v], "random_seed_call")))
            }
            // Sleep (L2.3)
            "time_sleep" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_sleep expects one argument (milliseconds)", span));
                }
                let ms = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.sleep, &[ms], "sleep_call")))
            }
            // Time operations (SL-9)
            "time_now" => {
                Ok(Some(self.call_bridged(self.runtime.time_now, &[], "time_now_call")))
            }
            "time_timestamp" => {
                Ok(Some(self.call_bridged(self.runtime.time_timestamp, &[], "time_ts_call")))
            }
            "time_format_iso" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_format_iso expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_format_iso, &[v], "time_fmt_call")))
            }
            "time_year" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_year expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_year, &[v], "time_year_call")))
            }
            "time_month" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_month expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_month, &[v], "time_month_call")))
            }
            "time_day" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_day expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_day, &[v], "time_day_call")))
            }
            "time_hour" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_hour expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_hour, &[v], "time_hour_call")))
            }
            "time_minute" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_minute expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_minute, &[v], "time_min_call")))
            }
            "time_second" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("time_second expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.time_second, &[v], "time_sec_call")))
            }
            // String lines
            "string_lines" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_lines expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_lines, &[v], "str_lines_call")))
            }
            // Sort
            "sort_natural" | "list_sort_natural" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sort_natural expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.list_sort_natural, &[v], "sort_nat_call")))
            }
            // Bytes extensions
            "bytes_from_hex" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("bytes_from_hex expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_from_hex, &[v], "bytes_hex_call")))
            }
            "bytes_contains" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("bytes_contains expects two arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_contains, &[a, b], "bytes_contains_call")))
            }
            "bytes_find" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("bytes_find expects two arguments", span));
                }
                let a = self.emit_expression(ctx, &args[0])?;
                let b = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.bytes_find, &[a, b], "bytes_find_call")))
            }
            // Encoding
            "base64_encode" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("base64_encode expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.base64_encode, &[v], "b64_enc_call")))
            }
            "base64_decode" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("base64_decode expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.base64_decode, &[v], "b64_dec_call")))
            }
            "hex_encode" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("hex_encode expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.hex_encode, &[v], "hex_enc_call")))
            }
            "hex_decode" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("hex_decode expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.hex_decode, &[v], "hex_dec_call")))
            }
            // TCP networking
            "tcp_listen" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("tcp_listen expects two arguments (host, port)", span));
                }
                let h = self.emit_expression(ctx, &args[0])?;
                let p = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.tcp_listen, &[h, p], "tcp_listen_call")))
            }
            "tcp_accept" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("tcp_accept expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.tcp_accept, &[v], "tcp_accept_call")))
            }
            "tcp_connect" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("tcp_connect expects two arguments (host, port)", span));
                }
                let h = self.emit_expression(ctx, &args[0])?;
                let p = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.tcp_connect, &[h, p], "tcp_connect_call")))
            }
            "tcp_read" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("tcp_read expects two arguments (conn, n)", span));
                }
                let c = self.emit_expression(ctx, &args[0])?;
                let n = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.tcp_read, &[c, n], "tcp_read_call")))
            }
            "tcp_write" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("tcp_write expects two arguments (conn, data)", span));
                }
                let c = self.emit_expression(ctx, &args[0])?;
                let d = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.tcp_write, &[c, d], "tcp_write_call")))
            }
            "tcp_close" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("tcp_close expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.tcp_close, &[v], "tcp_close_call")))
            }
            // Actor monitoring (AC-2)
            "monitor" | "actor_monitor" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("monitor expects two arguments (watcher, watched)", span));
                }
                let w = self.emit_expression(ctx, &args[0])?;
                let t = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.actor_monitor, &[w, t], "monitor_call")))
            }
            "demonitor" | "actor_demonitor" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("demonitor expects two arguments (watcher, watched)", span));
                }
                let w = self.emit_expression(ctx, &args[0])?;
                let t = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.actor_demonitor, &[w, t], "demonitor_call")))
            }
            // Graceful stop (AC-4)
            "graceful_stop" | "actor_graceful_stop" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("graceful_stop expects one argument", span));
                }
                let v = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.actor_graceful_stop, &[v], "graceful_stop_call")))
            }
            // StringBuilder / optimized string ops (L1.1)
            "sb_new" => {
                if !args.is_empty() {
                    return Err(Diagnostic::new("sb_new expects no arguments", span));
                }
                Ok(Some(self.call_bridged(self.runtime.sb_new, &[], "sb_new_call")))
            }
            "sb_push" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("sb_push expects 2 arguments (builder, string)", span));
                }
                let sb = self.emit_expression(ctx, &args[0])?;
                let s = self.emit_expression(ctx, &args[1])?;
                let sb_ptr = self.nb_to_ptr(sb);
                let s_ptr = self.nb_to_ptr(s);
                self.call_runtime_void(self.runtime.sb_push, &[sb_ptr.into(), s_ptr.into()], "sb_push_call");
                // Return unit
                let unit = self.call_nb(self.runtime.nb_make_unit, &[], "sb_push_unit");
                Ok(Some(unit))
            }
            "sb_finish" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sb_finish expects 1 argument (builder)", span));
                }
                let sb = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.sb_finish, &[sb], "sb_finish_call")))
            }
            "sb_len" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("sb_len expects 1 argument (builder)", span));
                }
                let sb = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.sb_len, &[sb], "sb_len_call")))
            }
            "string_join_list" | "join_list" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_join_list expects 2 arguments (list, separator)", span));
                }
                let list = self.emit_expression(ctx, &args[0])?;
                let sep = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_join_list, &[list, sep], "join_list_call")))
            }
            "string_repeat" | "repeat_string" => {
                if args.len() != 2 {
                    return Err(Diagnostic::new("string_repeat expects 2 arguments (string, count)", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                let n = self.emit_expression(ctx, &args[1])?;
                Ok(Some(self.call_bridged(self.runtime.string_repeat, &[s, n], "string_repeat_call")))
            }
            "string_reverse" | "reverse_string" => {
                if args.len() != 1 {
                    return Err(Diagnostic::new("string_reverse expects 1 argument (string)", span));
                }
                let s = self.emit_expression(ctx, &args[0])?;
                Ok(Some(self.call_bridged(self.runtime.string_reverse, &[s], "string_reverse_call")))
            }
            // Range helper (Phase D)
            "range" => {
                match args.len() {
                    1 => {
                        let zero_f64 = self.f64_type.const_float(0.0);
                        let zero = self.wrap_number(zero_f64);
                        let n = self.emit_expression(ctx, &args[0])?;
                        Ok(Some(self.call_bridged(self.runtime.list_range, &[zero, n], "range_call")))
                    }
                    2 => {
                        let start = self.emit_expression(ctx, &args[0])?;
                        let end = self.emit_expression(ctx, &args[1])?;
                        Ok(Some(self.call_bridged(self.runtime.list_range, &[start, end], "range_call")))
                    }
                    _ => Err(Diagnostic::new("range expects 1 or 2 arguments", span)),
                }
            }
            _ => {
                // Check if it's a store/actor constructor (not arbitrary make_* functions)
                if self.store_constructors.contains(name) {
                    let ctor_fn = self.functions[name];
                    let call = self.builder.build_call(ctor_fn, &[], "actor_ctor").unwrap();
                    let handle = call.try_as_basic_value().left()
                        .ok_or_else(|| Diagnostic::new("actor constructor produced no value", span))?
                        .into_int_value();
                    Ok(Some(handle))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// C3.2: Emit an inline map loop, directly embedding the lambda body
    /// instead of creating a closure and calling the runtime `coral_list_map`.
    fn emit_inline_map(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        target: &Expression,
        param_name: &str,
        body: &Block,
        _span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let function = ctx.function;
        let list_value = self.emit_expression(ctx, target)?;

        // Get list length as f64
        let len_nb = self.call_bridged(self.runtime.list_length, &[list_value], "map_len");
        let len_f64 = self.value_to_number(len_nb);

        // Create empty output list: coral_make_list(null, 0)
        let null_ptr = self.runtime.value_ptr_type
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        let zero_usize = self.usize_type.const_int(0, false);
        let out_list_ptr = self.call_runtime_ptr(
            self.runtime.make_list,
            &[null_ptr.into(), zero_usize.into()],
            "map_out_list",
        );
        let out_list_nb = self.ptr_to_nb(out_list_ptr);

        // Alloca for output list (mutated by push)
        let out_alloca = self.builder.build_alloca(self.runtime.value_i64_type, "map_out_alloca").unwrap();
        self.builder.build_store(out_alloca, out_list_nb).unwrap();

        // Counter alloca
        let counter_alloca = self.builder.build_alloca(self.f64_type, "map_counter").unwrap();
        self.builder.build_store(counter_alloca, self.f64_type.const_float(0.0)).unwrap();

        let loop_header = self.context.append_basic_block(function, "map_cond");
        let loop_body = self.context.append_basic_block(function, "map_body");
        let loop_exit = self.context.append_basic_block(function, "map_exit");

        self.builder.build_unconditional_branch(loop_header).unwrap();

        // Header: check counter < length
        self.builder.position_at_end(loop_header);
        let current = self.builder.build_load(self.f64_type, counter_alloca, "map_i")
            .unwrap().into_float_value();
        let is_done = self.builder.build_float_compare(
            inkwell::FloatPredicate::OGE, current, len_f64, "map_done",
        ).unwrap();
        self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

        // Body: get element, emit lambda body, push result
        self.builder.position_at_end(loop_body);
        ctx.cse_cache.clear();
        let idx_nb = self.wrap_number(current);
        let elem_nb = self.call_bridged(self.runtime.list_get, &[list_value, idx_nb], "map_elem");
        self.store_variable(ctx, param_name, elem_nb);

        let result = self.emit_block(ctx, body)?;

        // Push result to output list
        let cur_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "cur_out")
            .unwrap().into_int_value();
        let new_out = self.call_bridged(self.runtime.list_push, &[cur_out, result], "map_push");
        self.builder.build_store(out_alloca, new_out).unwrap();

        // Increment counter
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            let cur_f64 = self.builder.build_load(self.f64_type, counter_alloca, "map_cur_upd")
                .unwrap().into_float_value();
            let next = self.builder.build_float_add(
                cur_f64, self.f64_type.const_float(1.0), "map_next",
            ).unwrap();
            self.builder.build_store(counter_alloca, next).unwrap();
            self.builder.build_unconditional_branch(loop_header).unwrap();
        }

        // Return the output list
        self.builder.position_at_end(loop_exit);
        let final_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "map_result")
            .unwrap().into_int_value();
        Ok(final_out)
    }

    /// C3.2: Emit an inline filter loop, directly embedding the predicate body
    /// instead of creating a closure and calling the runtime `coral_list_filter`.
    fn emit_inline_filter(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        target: &Expression,
        param_name: &str,
        body: &Block,
        _span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let function = ctx.function;
        let list_value = self.emit_expression(ctx, target)?;

        // Get list length as f64
        let len_nb = self.call_bridged(self.runtime.list_length, &[list_value], "filter_len");
        let len_f64 = self.value_to_number(len_nb);

        // Create empty output list
        let null_ptr = self.runtime.value_ptr_type
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        let zero_usize = self.usize_type.const_int(0, false);
        let out_list_ptr = self.call_runtime_ptr(
            self.runtime.make_list,
            &[null_ptr.into(), zero_usize.into()],
            "filter_out_list",
        );
        let out_list_nb = self.ptr_to_nb(out_list_ptr);

        let out_alloca = self.builder.build_alloca(self.runtime.value_i64_type, "filter_out_alloca").unwrap();
        self.builder.build_store(out_alloca, out_list_nb).unwrap();

        let counter_alloca = self.builder.build_alloca(self.f64_type, "filter_counter").unwrap();
        self.builder.build_store(counter_alloca, self.f64_type.const_float(0.0)).unwrap();

        // Alloca to hold current element across block boundaries
        let elem_alloca = self.builder.build_alloca(self.runtime.value_i64_type, "filter_elem_alloca").unwrap();

        let loop_header = self.context.append_basic_block(function, "filter_cond");
        let loop_body = self.context.append_basic_block(function, "filter_body");
        let loop_push = self.context.append_basic_block(function, "filter_push");
        let loop_skip = self.context.append_basic_block(function, "filter_skip");
        let loop_exit = self.context.append_basic_block(function, "filter_exit");

        self.builder.build_unconditional_branch(loop_header).unwrap();

        // Header: check counter < length
        self.builder.position_at_end(loop_header);
        let current = self.builder.build_load(self.f64_type, counter_alloca, "filter_i")
            .unwrap().into_float_value();
        let is_done = self.builder.build_float_compare(
            inkwell::FloatPredicate::OGE, current, len_f64, "filter_done",
        ).unwrap();
        self.builder.build_conditional_branch(is_done, loop_exit, loop_body).unwrap();

        // Body: get element, eval predicate
        self.builder.position_at_end(loop_body);
        ctx.cse_cache.clear();
        let idx_nb = self.wrap_number(current);
        let elem_nb = self.call_bridged(self.runtime.list_get, &[list_value, idx_nb], "filter_elem");
        self.builder.build_store(elem_alloca, elem_nb).unwrap();
        self.store_variable(ctx, param_name, elem_nb);

        let predicate_result = self.emit_block(ctx, body)?;

        // Check truthiness: nb_is_truthy returns i8
        let is_truthy = self.call_nb(self.runtime.nb_is_truthy, &[predicate_result.into()], "filter_truthy");
        let truthy_bool = self.builder.build_int_compare(
            inkwell::IntPredicate::NE,
            is_truthy,
            self.i8_type.const_int(0, false),
            "filter_bool",
        ).unwrap();
        self.builder.build_conditional_branch(truthy_bool, loop_push, loop_skip).unwrap();

        // Push: add element to output list (reload from alloca for SSA safety)
        self.builder.position_at_end(loop_push);
        let saved_elem = self.builder.build_load(self.runtime.value_i64_type, elem_alloca, "saved_elem")
            .unwrap().into_int_value();
        let cur_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "cur_out_f")
            .unwrap().into_int_value();
        let new_out = self.call_bridged(self.runtime.list_push, &[cur_out, saved_elem], "filter_push");
        self.builder.build_store(out_alloca, new_out).unwrap();
        self.builder.build_unconditional_branch(loop_skip).unwrap();

        // Skip / continue: increment counter
        self.builder.position_at_end(loop_skip);
        let next = self.builder.build_float_add(
            current, self.f64_type.const_float(1.0), "filter_next",
        ).unwrap();
        self.builder.build_store(counter_alloca, next).unwrap();
        self.builder.build_unconditional_branch(loop_header).unwrap();

        // Return the output list
        self.builder.position_at_end(loop_exit);
        let final_out = self.builder.build_load(self.runtime.value_i64_type, out_alloca, "filter_result")
            .unwrap().into_int_value();
        Ok(final_out)
    }
}
