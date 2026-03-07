//! Pattern matching and ADT dispatch for Coral codegen.
//!
//! Contains match expression compilation, condition checking,
//! and pattern variable binding.

use super::*;

impl<'ctx> CodeGenerator<'ctx> {
    pub(super) fn emit_match(
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

    pub(super) fn emit_match_condition(
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
    pub(super) fn bind_pattern_variables(
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

}