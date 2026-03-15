use super::*;

impl<'ctx> CodeGenerator<'ctx> {
    fn pattern_has_bindings(pattern: &MatchPattern) -> bool {
        match pattern {
            MatchPattern::Identifier(_) => true,
            MatchPattern::Rest(..) => true,
            MatchPattern::Constructor { fields, .. } => {
                fields.iter().any(Self::pattern_has_bindings)
            }
            MatchPattern::Or(alts) => alts.iter().any(Self::pattern_has_bindings),
            MatchPattern::List(pats) => pats.iter().any(Self::pattern_has_bindings),
            MatchPattern::Range { .. } => false,
            MatchPattern::RangeBinding { .. } => true,
            _ => false,
        }
    }

    pub(super) fn emit_match(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        match_expr: &MatchExpression,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let match_value = self.emit_expression(ctx, match_expr.value.as_ref())?;
        let function = ctx.function;
        let cont_bb = self.context.append_basic_block(function, "match_cont");
        let mut incoming: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();

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
            let condition =
                self.emit_match_condition(ctx, match_value, &arm.pattern, match_expr.span)?;
            self.builder
                .build_conditional_branch(condition, arm_block, next_block)
                .unwrap();

            self.builder.position_at_end(arm_block);

            ctx.cse_cache.clear();

            self.bind_pattern_variables(ctx, match_value, &arm.pattern);

            if let Some(guard) = &arm.guard {
                let guard_val = self.emit_expression(ctx, guard)?;
                let guard_bool = self.value_to_bool(guard_val);
                let guard_pass_bb = self
                    .context
                    .append_basic_block(function, &format!("guard_pass_{index}"));
                self.builder
                    .build_conditional_branch(guard_bool, guard_pass_bb, next_block)
                    .unwrap();
                self.builder.position_at_end(guard_pass_bb);
            }

            let result = self.emit_block(ctx, &arm.body)?;

            let arm_end_bb = self.builder.get_insert_block().unwrap();
            if arm_end_bb.get_terminator().is_none() {
                self.builder.build_unconditional_branch(cont_bb).unwrap();
                incoming.push((result, arm_end_bb));
            }

            current_block = next_block;
        }

        let default_block = self.context.append_basic_block(function, "match_default");
        self.builder.position_at_end(current_block);
        self.builder
            .build_unconditional_branch(default_block)
            .unwrap();

        self.builder.position_at_end(default_block);

        ctx.cse_cache.clear();
        let default_value = if let Some(default_block_ast) = &match_expr.default {
            self.emit_block(ctx, default_block_ast.as_ref())?
        } else {
            self.runtime.value_i64_type.const_zero()
        };
        let default_end_bb = self.builder.get_insert_block().unwrap();
        if default_end_bb.get_terminator().is_none() {
            self.builder.build_unconditional_branch(cont_bb).unwrap();
            incoming.push((default_value, default_end_bb));
        }

        self.builder.position_at_end(cont_bb);
        if incoming.is_empty() {
            Ok(self.runtime.value_i64_type.const_zero())
        } else {
            let phi = self
                .builder
                .build_phi(self.runtime.value_i64_type, "match_phi")
                .unwrap();
            for (value, block) in incoming {
                phi.add_incoming(&[(&value as &dyn BasicValue<'ctx>, block)]);
            }
            Ok(phi.as_basic_value().into_int_value())
        }
    }

    pub(super) fn emit_match_condition(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        match_value: IntValue<'ctx>,
        pattern: &MatchPattern,
        _span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match pattern {
            MatchPattern::Integer(value) => {
                let literal = self.wrap_number_unchecked(self.f64_type.const_float(*value as f64));
                let eq = self.call_nb(
                    self.runtime.nb_equals,
                    &[match_value.into(), literal.into()],
                    "match_eq_num",
                );
                Ok(self.value_to_bool(eq))
            }
            MatchPattern::Bool(value) => {
                let literal = self.wrap_bool(self.boolean_to_int(*value));
                let eq = self.call_nb(
                    self.runtime.nb_equals,
                    &[match_value.into(), literal.into()],
                    "match_eq_bool",
                );
                Ok(self.value_to_bool(eq))
            }
            MatchPattern::String(text) => {
                let literal = self.emit_string_literal(text);
                let eq = self.call_nb(
                    self.runtime.nb_equals,
                    &[match_value.into(), literal.into()],
                    "match_eq_str",
                );
                Ok(self.value_to_bool(eq))
            }
            MatchPattern::List(patterns) => {
                let has_rest = patterns.iter().any(|p| matches!(p, MatchPattern::Rest(..)));
                let fixed_count = patterns
                    .iter()
                    .filter(|p| !matches!(p, MatchPattern::Rest(..)))
                    .count();

                let len_nb =
                    self.call_bridged(self.runtime.list_length, &[match_value], "list_len");
                let expected_len = self.wrap_number_unchecked(self.f64_type.const_float(fixed_count as f64));

                let len_ok = if has_rest {
                    let ge = self.call_nb(
                        self.runtime.nb_greater_equal,
                        &[len_nb.into(), expected_len.into()],
                        "len_ge",
                    );
                    self.value_to_bool(ge)
                } else {
                    let eq = self.call_nb(
                        self.runtime.nb_equals,
                        &[len_nb.into(), expected_len.into()],
                        "len_eq",
                    );
                    self.value_to_bool(eq)
                };

                let has_nontrivial = patterns.iter().any(|p| {
                    !matches!(
                        p,
                        MatchPattern::Identifier(_)
                            | MatchPattern::Wildcard(_)
                            | MatchPattern::Rest(..)
                    )
                });
                if !has_nontrivial {
                    return Ok(len_ok);
                }

                let function = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let check_bb = self.context.append_basic_block(function, "list_check");
                let fail_bb = self.context.append_basic_block(function, "list_fail");
                let cont_bb = self.context.append_basic_block(function, "list_cont");

                self.builder
                    .build_conditional_branch(len_ok, check_bb, fail_bb)
                    .unwrap();

                self.builder.position_at_end(check_bb);
                let mut all_match = self.bool_type.const_int(1, false);

                for (idx, pat) in patterns.iter().enumerate() {
                    match pat {
                        MatchPattern::Identifier(_)
                        | MatchPattern::Wildcard(_)
                        | MatchPattern::Rest(..) => continue,
                        _ => {}
                    }
                    let idx_nb = self.wrap_number(self.f64_type.const_float(idx as f64));
                    let elem = self.call_bridged(
                        self.runtime.list_get,
                        &[match_value, idx_nb],
                        &format!("list_elem_{}", idx),
                    );
                    let elem_match = self.emit_match_condition(ctx, elem, pat, _span)?;
                    all_match = self
                        .builder
                        .build_and(all_match, elem_match, "and_elem")
                        .unwrap();
                }

                self.builder.build_unconditional_branch(cont_bb).unwrap();
                let check_end = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(fail_bb);
                self.builder.build_unconditional_branch(cont_bb).unwrap();

                self.builder.position_at_end(cont_bb);
                let phi = self
                    .builder
                    .build_phi(self.bool_type, "list_result")
                    .unwrap();
                phi.add_incoming(&[
                    (&all_match, check_end),
                    (&self.bool_type.const_int(0, false), fail_bb),
                ]);

                Ok(phi.as_basic_value().into_int_value())
            }
            MatchPattern::Identifier(_) => Ok(self.bool_type.const_int(1, false)),
            MatchPattern::Wildcard(_) => Ok(self.bool_type.const_int(1, false)),
            MatchPattern::Rest(..) => Ok(self.bool_type.const_int(1, false)),
            MatchPattern::Range { start, end, .. } => {
                let start_val = self.wrap_number(self.f64_type.const_float(*start as f64));
                let end_val = self.wrap_number(self.f64_type.const_float(*end as f64));

                let ge_result = self.call_nb(
                    self.runtime.nb_greater_equal,
                    &[match_value.into(), start_val.into()],
                    "range_ge",
                );
                let ge_bool = self.value_to_bool(ge_result);

                let le_result = self.call_nb(
                    self.runtime.nb_less_equal,
                    &[match_value.into(), end_val.into()],
                    "range_le",
                );
                let le_bool = self.value_to_bool(le_result);

                let in_range = self
                    .builder
                    .build_and(ge_bool, le_bool, "in_range")
                    .unwrap();
                Ok(in_range)
            }
            MatchPattern::RangeBinding {
                name, start, end, ..
            } => {
                let start_val = self.wrap_number(self.f64_type.const_float(*start as f64));
                let end_val = self.wrap_number(self.f64_type.const_float(*end as f64));

                let ge_result = self.call_nb(
                    self.runtime.nb_greater_equal,
                    &[match_value.into(), start_val.into()],
                    "rb_ge",
                );
                let ge_bool = self.value_to_bool(ge_result);

                let le_result = self.call_nb(
                    self.runtime.nb_less_equal,
                    &[match_value.into(), end_val.into()],
                    "rb_le",
                );
                let le_bool = self.value_to_bool(le_result);

                self.store_variable(ctx, name, match_value);

                let in_range = self
                    .builder
                    .build_and(ge_bool, le_bool, "rb_in_range")
                    .unwrap();
                Ok(in_range)
            }
            MatchPattern::Or(alternatives) => {
                let mut result = self.bool_type.const_int(0, false);
                for alt in alternatives {
                    let alt_cond = self.emit_match_condition(ctx, match_value, alt, _span)?;
                    result = self.builder.build_or(result, alt_cond, "or_pat").unwrap();
                }
                Ok(result)
            }
            MatchPattern::Constructor { name, fields, span } => {
                let match_ptr = self.nb_to_ptr(match_value);
                let tag_name_bytes = name.as_bytes();
                let tag_name_global = self.get_or_create_string_constant(name);
                let tag_name_ptr = self
                    .builder
                    .build_pointer_cast(
                        tag_name_global.as_pointer_value(),
                        self.i8_type.ptr_type(AddressSpace::default()),
                        "tag_name_ptr",
                    )
                    .unwrap();
                let tag_name_len = self
                    .usize_type
                    .const_int(tag_name_bytes.len() as u64, false);

                let is_tag_result = self.call_runtime_ptr(
                    self.runtime.tagged_is_tag,
                    &[match_ptr.into(), tag_name_ptr.into(), tag_name_len.into()],
                    "is_tag",
                );
                let tag_nb = self.ptr_to_nb(is_tag_result);
                let tag_matches = self.value_to_bool(tag_nb);

                if fields.is_empty() {
                    return Ok(tag_matches);
                }

                let function = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let nested_check_bb = self.context.append_basic_block(function, "nested_check");
                let nested_fail_bb = self.context.append_basic_block(function, "nested_fail");
                let nested_cont_bb = self.context.append_basic_block(function, "nested_cont");

                self.builder
                    .build_conditional_branch(tag_matches, nested_check_bb, nested_fail_bb)
                    .unwrap();

                self.builder.position_at_end(nested_check_bb);
                let mut all_fields_match = self.bool_type.const_int(1, false);

                for (idx, field_pattern) in fields.iter().enumerate() {
                    match field_pattern {
                        MatchPattern::Identifier(_) | MatchPattern::Wildcard(_) => continue,
                        _ => {}
                    }

                    let idx_val = self.usize_type.const_int(idx as u64, false);
                    let field_ptr = self.call_runtime_ptr(
                        self.runtime.tagged_get_field,
                        &[match_ptr.into(), idx_val.into()],
                        &format!("nested_field_{}", idx),
                    );
                    let field_value = self.ptr_to_nb(field_ptr);

                    let field_matches =
                        self.emit_match_condition(ctx, field_value, field_pattern, *span)?;

                    all_fields_match = self
                        .builder
                        .build_and(all_fields_match, field_matches, "and_fields")
                        .unwrap();
                }

                self.builder
                    .build_unconditional_branch(nested_cont_bb)
                    .unwrap();
                let nested_check_end = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(nested_fail_bb);
                self.builder
                    .build_unconditional_branch(nested_cont_bb)
                    .unwrap();

                self.builder.position_at_end(nested_cont_bb);
                let phi = self
                    .builder
                    .build_phi(self.bool_type, "nested_result")
                    .unwrap();
                phi.add_incoming(&[
                    (&all_fields_match, nested_check_end),
                    (&self.bool_type.const_int(0, false), nested_fail_bb),
                ]);

                Ok(phi.as_basic_value().into_int_value())
            }
        }
    }

    pub(super) fn bind_pattern_variables(
        &mut self,
        ctx: &mut FunctionContext<'ctx>,
        value: IntValue<'ctx>,
        pattern: &MatchPattern,
    ) {
        match pattern {
            MatchPattern::Identifier(name) => {
                self.call_nb_void(self.runtime.nb_retain, &[value.into()]);
                self.store_variable(ctx, name, value);
            }
            MatchPattern::Constructor { fields, .. } => {
                let value_ptr = self.nb_to_ptr(value);
                for (idx, field_pattern) in fields.iter().enumerate() {
                    let idx_val = self.usize_type.const_int(idx as u64, false);
                    let field_ptr = self.call_runtime_ptr(
                        self.runtime.tagged_get_field,
                        &[value_ptr.into(), idx_val.into()],
                        &format!("get_field_{}", idx),
                    );
                    let field_value = self.ptr_to_nb(field_ptr);

                    self.bind_pattern_variables(ctx, field_value, field_pattern);
                }
            }
            MatchPattern::Wildcard(_) => {}
            MatchPattern::Or(alternatives) => {
                let has_bindings = alternatives.iter().any(|a| Self::pattern_has_bindings(a));
                if !has_bindings {
                    return;
                }

                let function = ctx.function;
                let cont_bb = self.context.append_basic_block(function, "or_bind_done");

                for (i, alt) in alternatives.iter().enumerate() {
                    let cond = self
                        .emit_match_condition(ctx, value, alt, Span::new(0, 0))
                        .unwrap_or_else(|_| self.bool_type.const_int(0, false));

                    let bind_bb = self
                        .context
                        .append_basic_block(function, &format!("or_bind_{}", i));
                    let next_bb = if i + 1 < alternatives.len() {
                        self.context
                            .append_basic_block(function, &format!("or_try_{}", i + 1))
                    } else {
                        cont_bb
                    };

                    self.builder
                        .build_conditional_branch(cond, bind_bb, next_bb)
                        .unwrap();

                    self.builder.position_at_end(bind_bb);
                    self.bind_pattern_variables(ctx, value, alt);
                    self.builder.build_unconditional_branch(cont_bb).unwrap();

                    if i + 1 < alternatives.len() {
                        self.builder.position_at_end(next_bb);
                    }
                }

                self.builder.position_at_end(cont_bb);
            }
            MatchPattern::List(patterns) => {
                let has_bindings = patterns.iter().any(|p| Self::pattern_has_bindings(p));
                if !has_bindings {
                    return;
                }
                for (idx, pat) in patterns.iter().enumerate() {
                    match pat {
                        MatchPattern::Rest(name, _) => {
                            let start_nb = self.wrap_number(self.f64_type.const_float(idx as f64));
                            let len_nb =
                                self.call_bridged(self.runtime.list_length, &[value], "rest_len");
                            let rest_val = self.call_bridged(
                                self.runtime.list_slice,
                                &[value, start_nb, len_nb],
                                "rest_slice",
                            );
                            self.call_nb_void(self.runtime.nb_retain, &[rest_val.into()]);
                            self.store_variable(ctx, name, rest_val);
                        }
                        _ => {
                            if !Self::pattern_has_bindings(pat)
                                && !matches!(pat, MatchPattern::Identifier(_))
                            {
                                continue;
                            }
                            let idx_nb = self.wrap_number(self.f64_type.const_float(idx as f64));
                            let elem = self.call_bridged(
                                self.runtime.list_get,
                                &[value, idx_nb],
                                &format!("bind_elem_{}", idx),
                            );
                            self.bind_pattern_variables(ctx, elem, pat);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
