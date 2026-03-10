use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::span::Span;
use std::collections::BTreeSet;

pub fn lower(program: Program) -> Result<Program, Diagnostic> {
    PlaceholderLowerer.lower_program(program)
}

#[derive(Default)]
struct PlaceholderLowerer;

const PLACEHOLDER_USAGE_MESSAGE: &str =
    "placeholder expressions are only valid inside function call arguments";

impl PlaceholderLowerer {
    fn lower_program(&mut self, program: Program) -> Result<Program, Diagnostic> {
        let mut items = Vec::with_capacity(program.items.len());
        for item in program.items {
            items.push(self.lower_item(item)?);
        }
        Ok(Program {
            items,
            span: program.span,
        })
    }

    fn lower_item(&mut self, item: Item) -> Result<Item, Diagnostic> {
        match item {
            Item::Binding(binding) => self.lower_binding(binding).map(Item::Binding),
            Item::Function(function) => self.lower_function(function).map(Item::Function),
            Item::Type(ty) => self.lower_type(ty).map(Item::Type),
            Item::Store(store) => self.lower_store(store).map(Item::Store),
            Item::Taxonomy(node) => self.lower_taxonomy(node).map(Item::Taxonomy),
            Item::ExternFunction(_) => Ok(item),
            Item::ErrorDefinition(_) => Ok(item),  // Error definitions are already in final form
            Item::TraitDefinition(_) => Ok(item),  // Trait definitions are already in final form
            Item::Expression(expr) => {
                let expr = self
                    .lower_expression(expr)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                Ok(Item::Expression(expr))
            }
        }
    }

    fn lower_binding(&mut self, mut binding: Binding) -> Result<Binding, Diagnostic> {
        binding.value = self
            .lower_expression(binding.value)?
            .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
        Ok(binding)
    }

    fn lower_function(&mut self, mut function: Function) -> Result<Function, Diagnostic> {
        self.lower_parameters(&mut function.params)?;
        function.body = self.lower_block(function.body)?;
        Ok(function)
    }

    fn lower_type(&mut self, mut ty: TypeDefinition) -> Result<TypeDefinition, Diagnostic> {
        ty.fields = ty
            .fields
            .into_iter()
            .map(|field| self.lower_field(field))
            .collect::<Result<_, _>>()?;
        ty.methods = ty
            .methods
            .into_iter()
            .map(|method| self.lower_function(method))
            .collect::<Result<_, _>>()?;
        Ok(ty)
    }

    fn lower_store(&mut self, mut store: StoreDefinition) -> Result<StoreDefinition, Diagnostic> {
        store.fields = store
            .fields
            .into_iter()
            .map(|field| self.lower_field(field))
            .collect::<Result<_, _>>()?;
        store.methods = store
            .methods
            .into_iter()
            .map(|method| self.lower_function(method))
            .collect::<Result<_, _>>()?;
        Ok(store)
    }

    fn lower_taxonomy(&mut self, mut node: TaxonomyNode) -> Result<TaxonomyNode, Diagnostic> {
        node.bindings = node
            .bindings
            .into_iter()
            .map(|binding| self.lower_binding(binding))
            .collect::<Result<_, _>>()?;
        node.children = node
            .children
            .into_iter()
            .map(|child| self.lower_taxonomy(child))
            .collect::<Result<_, _>>()?;
        Ok(node)
    }

    fn lower_field(&mut self, mut field: Field) -> Result<Field, Diagnostic> {
        if let Some(default) = field.default.take() {
            field.default = Some(
                self.lower_expression(default)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?,
            );
        }
        Ok(field)
    }

    fn lower_parameters(&mut self, params: &mut [Parameter]) -> Result<(), Diagnostic> {
        for param in params.iter_mut() {
            if let Some(default) = param.default.take() {
                param.default = Some(
                    self.lower_expression(default)?
                        .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?,
                );
            }
        }
        Ok(())
    }

    fn lower_block(&mut self, mut block: Block) -> Result<Block, Diagnostic> {
        let mut lowered_statements = Vec::with_capacity(block.statements.len());
        for statement in block.statements.into_iter() {
            lowered_statements.push(self.lower_statement(statement)?);
        }
        block.statements = lowered_statements;
        if let Some(value) = block.value.take() {
            block.value = Some(Box::new(
                self.lower_expression(*value)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?,
            ));
        }
        Ok(block)
    }

    fn lower_statement(&mut self, statement: Statement) -> Result<Statement, Diagnostic> {
        match statement {
            Statement::Binding(binding) => self.lower_binding(binding).map(Statement::Binding),
            Statement::Expression(expr) => {
                let expr = self
                    .lower_expression(expr)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                Ok(Statement::Expression(expr))
            }
            Statement::Return(expr, span) => {
                let value = self
                    .lower_expression(expr)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                Ok(Statement::Return(value, span))
            }
            Statement::If { condition, body, elif_branches, else_body, span } => {
                let condition = self
                    .lower_expression(condition)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let body = self.lower_block(body)?;
                let mut lowered_elifs = Vec::with_capacity(elif_branches.len());
                for (cond, blk) in elif_branches {
                    let cond = self
                        .lower_expression(cond)?
                        .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                    let blk = self.lower_block(blk)?;
                    lowered_elifs.push((cond, blk));
                }
                let else_body = match else_body {
                    Some(blk) => Some(self.lower_block(blk)?),
                    None => None,
                };
                Ok(Statement::If { condition, body, elif_branches: lowered_elifs, else_body, span })
            }
            Statement::While { condition, body, span } => {
                let condition = self
                    .lower_expression(condition)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let body = self.lower_block(body)?;
                Ok(Statement::While { condition, body, span })
            }
            Statement::For { variable, iterable, body, span } => {
                let iterable = self
                    .lower_expression(iterable)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let body = self.lower_block(body)?;
                Ok(Statement::For { variable, iterable, body, span })
            }
            Statement::ForKV { key_var, value_var, iterable, body, span } => {
                let iterable = self
                    .lower_expression(iterable)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let body = self.lower_block(body)?;
                Ok(Statement::ForKV { key_var, value_var, iterable, body, span })
            }
            Statement::ForRange { variable, start, end, step, body, span } => {
                let start = self
                    .lower_expression(start)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let end = self
                    .lower_expression(end)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let step = step.map(|s| self.lower_expression(s)
                    .and_then(|e| e.expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)))
                    .transpose()?;
                let body = self.lower_block(body)?;
                Ok(Statement::ForRange { variable, start, end, step, body, span })
            }
            Statement::Break(span) => Ok(Statement::Break(span)),
            Statement::Continue(span) => Ok(Statement::Continue(span)),
            Statement::FieldAssign { target, field, value, span } => {
                let target = self
                    .lower_expression(target)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                let value = self
                    .lower_expression(value)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                Ok(Statement::FieldAssign { target, field, value, span })
            }
            Statement::PatternBinding { pattern, value, span } => {
                let value = self
                    .lower_expression(value)?
                    .expect_no_placeholders(PLACEHOLDER_USAGE_MESSAGE)?;
                Ok(Statement::PatternBinding { pattern, value, span })
            }
        }
    }

    fn lower_expression(&mut self, expr: Expression) -> Result<ExprLowering, Diagnostic> {
        match expr {
            Expression::Placeholder(index, span) => Ok(ExprLowering::with_placeholder(
                Expression::Placeholder(index, span),
                PlaceholderInfo::new(index, span),
            )),
            Expression::Binary { op, left, right, span } => {
                let left = self.lower_expression(*left)?;
                let right = self.lower_expression(*right)?;
                Ok(ExprLowering::with_children(
                    Expression::Binary {
                        op,
                        left: Box::new(left.expr),
                        right: Box::new(right.expr),
                        span,
                    },
                    left.placeholder,
                    right.placeholder,
                ))
            }
            Expression::Unary { op, expr: inner, span } => {
                let lowered = self.lower_expression(*inner)?;
                Ok(ExprLowering::with_child(
                    Expression::Unary {
                        op,
                        expr: Box::new(lowered.expr),
                        span,
                    },
                    lowered.placeholder,
                ))
            }
            Expression::List(items, span) => {
                self.lower_collection(items, span, |exprs| Expression::List(exprs, span))
            }
            Expression::Map(entries, span) => {
                let mut lowered_entries = Vec::with_capacity(entries.len());
                let mut placeholder = None;
                for (key, value) in entries {
                    let lowered_key = self.lower_expression(key)?;
                    let lowered_value = self.lower_expression(value)?;
                    placeholder = PlaceholderInfo::merge_option(placeholder, lowered_key.placeholder);
                    placeholder = PlaceholderInfo::merge_option(placeholder, lowered_value.placeholder);
                    lowered_entries.push((lowered_key.expr, lowered_value.expr));
                }
                Ok(ExprLowering::new_with_placeholder(
                    Expression::Map(lowered_entries, span),
                    placeholder,
                ))
            }
            Expression::Throw { value, span } => {
                let lowered = self.lower_expression(*value)?;
                Ok(ExprLowering::with_child(
                    Expression::Throw {
                        value: Box::new(lowered.expr),
                        span,
                    },
                    lowered.placeholder,
                ))
            }
            Expression::Call { callee, args, arg_names, span, .. } => self.lower_call(*callee, args, arg_names, span),
            Expression::Member { target, property, span } => {
                let lowered = self.lower_expression(*target)?;
                Ok(ExprLowering::with_child(
                    Expression::Member {
                        target: Box::new(lowered.expr),
                        property,
                        span,
                    },
                    lowered.placeholder,
                ))
            }
            Expression::Index { target, index, span } => {
                let target_lowered = self.lower_expression(*target)?;
                let index_lowered = self.lower_expression(*index)?;
                let placeholder = PlaceholderInfo::merge_option(
                    target_lowered.placeholder,
                    index_lowered.placeholder,
                );
                Ok(ExprLowering::new_with_placeholder(
                    Expression::Index {
                        target: Box::new(target_lowered.expr),
                        index: Box::new(index_lowered.expr),
                        span,
                    },
                    placeholder,
                ))
            }
            Expression::Ternary { condition, then_branch, else_branch, span } => {
                let cond = self.lower_expression(*condition)?;
                let then_b = self.lower_expression(*then_branch)?;
                let else_b = self.lower_expression(*else_branch)?;
                let placeholder = PlaceholderInfo::merge_option(
                    PlaceholderInfo::merge_option(cond.placeholder, then_b.placeholder),
                    else_b.placeholder,
                );
                Ok(ExprLowering::new_with_placeholder(
                    Expression::Ternary {
                        condition: Box::new(cond.expr),
                        then_branch: Box::new(then_b.expr),
                        else_branch: Box::new(else_b.expr),
                        span,
                    },
                    placeholder,
                ))
            }
            Expression::Match(expr) => self.lower_match(*expr),
            Expression::Lambda { params, body, span } => {
                let body = self.lower_block(body)?;
                Ok(ExprLowering::new(Expression::Lambda { params, body, span }))
            }
            // S2.1: Pipeline desugaring — `a ~ f(args)` → `f(a, args)` (or with $ replacement)
            Expression::Pipeline { left, right, span } => {
                self.lower_pipeline(*left, *right, span)
            }
            // S2.2/S2.3: Comprehensions — lower sub-expressions
            Expression::ListComprehension { body, var, iterable, condition, span } => {
                let body = self.lower_expression(*body)?;
                let iterable = self.lower_expression(*iterable)?;
                let condition = match condition {
                    Some(c) => Some(Box::new(self.lower_expression(*c)?.expr)),
                    None => None,
                };
                Ok(ExprLowering::new(Expression::ListComprehension {
                    body: Box::new(body.expr),
                    var,
                    iterable: Box::new(iterable.expr),
                    condition,
                    span,
                }))
            }
            Expression::MapComprehension { key, value, var, iterable, condition, span } => {
                let key = self.lower_expression(*key)?;
                let value = self.lower_expression(*value)?;
                let iterable = self.lower_expression(*iterable)?;
                let condition = match condition {
                    Some(c) => Some(Box::new(self.lower_expression(*c)?.expr)),
                    None => None,
                };
                Ok(ExprLowering::new(Expression::MapComprehension {
                    key: Box::new(key.expr),
                    value: Box::new(value.expr),
                    var,
                    iterable: Box::new(iterable.expr),
                    condition,
                    span,
                }))
            }
            other => Ok(ExprLowering::new(other)),
        }
    }

    fn lower_collection<F>(&mut self, items: Vec<Expression>, _span: Span, make: F) -> Result<ExprLowering, Diagnostic>
    where
        F: Fn(Vec<Expression>) -> Expression,
    {
        let mut lowered_items = Vec::with_capacity(items.len());
        let mut placeholder = None;
        for item in items {
            let lowered = self.lower_expression(item)?;
            placeholder = PlaceholderInfo::merge_option(placeholder, lowered.placeholder);
            lowered_items.push(lowered.expr);
        }
        Ok(ExprLowering::new_with_placeholder(make(lowered_items), placeholder))
    }

    fn lower_call(
        &mut self,
        callee: Expression,
        args: Vec<Expression>,
        arg_names: Vec<Option<String>>,
        span: Span,
    ) -> Result<ExprLowering, Diagnostic> {
        let callee_lowered = self.lower_expression(callee)?;
        if let Some(info) = callee_lowered.placeholder {
            return Err(info.diagnostic("placeholder cannot appear in function position"));
        }
        let mut lowered_args = Vec::with_capacity(args.len());
        for arg in args {
            let lowered = self.lower_expression(arg)?;
            if let Some(info) = lowered.placeholder {
                lowered_args.push(self.wrap_placeholder_lambda(lowered.expr, info)?);
            } else {
                lowered_args.push(lowered.expr);
            }
        }
        Ok(ExprLowering::new(Expression::Call {
            callee: Box::new(callee_lowered.expr),
            args: lowered_args,
            arg_names,
            span,
        }))
    }

    fn lower_match(&mut self, match_expr: MatchExpression) -> Result<ExprLowering, Diagnostic> {
        let mut lowered = match_expr;
    let value = self.lower_expression(*lowered.value)?;
    lowered.value = Box::new(value.expr);
    let placeholder = value.placeholder;
        for arm in lowered.arms.iter_mut() {
            arm.body = self.lower_block(arm.body.clone())?;
            // S3.2: Lower guard expression if present
            if let Some(guard) = arm.guard.take() {
                let lowered_guard = self.lower_expression(*guard)?;
                arm.guard = Some(Box::new(lowered_guard.expr));
            }
        }
        if let Some(block) = lowered.default.take() {
            lowered.default = Some(Box::new(self.lower_block(*block)?));
        }
        Ok(ExprLowering::new_with_placeholder(
            Expression::Match(Box::new(lowered)),
            placeholder,
        ))
    }

    /// S2.1: Desugar pipeline `a ~ f(args)` into a plain call.
    ///
    /// Three forms:
    ///   1. `a ~ f(x, $, y)` → `f(x, a, y)`       (explicit $ placeholder)
    ///   2. `a ~ f(x, y)`    → `f(a, x, y)`        (prepend as first arg)
    ///   3. `a ~ f`          → `f(a)`               (bare identifier)
    fn lower_pipeline(
        &mut self,
        left: Expression,
        right: Expression,
        span: Span,
    ) -> Result<ExprLowering, Diagnostic> {
        // Lower the left-hand side first
        let left_lowered = self.lower_expression(left)?;
        let left_expr = left_lowered
            .expect_no_placeholders("placeholder ($) cannot appear on the left side of a pipeline")?;

        let desugared = match right {
            Expression::Call { callee, args, span: call_span, .. } => {
                let has_placeholder = args.iter().any(|a| expr_contains_placeholder(a));
                let new_args = if has_placeholder {
                    args.into_iter()
                        .map(|a| replace_placeholder_in_expr(a, &left_expr))
                        .collect()
                } else {
                    let mut new_args = vec![left_expr.clone()];
                    new_args.extend(args);
                    new_args
                };
                Expression::Call {
                    callee,
                    args: new_args,
                    arg_names: vec![],
                    span: call_span,
                }
            }
            Expression::Identifier(name, id_span) => Expression::Call {
                callee: Box::new(Expression::Identifier(name, id_span)),
                args: vec![left_expr],
                arg_names: vec![],
                span,
            },
            _ => {
                return Err(Diagnostic::new(
                    "pipeline right-hand side must be a function call or identifier",
                    span,
                ));
            }
        };

        // Recursively lower the desugared result (handles nested pipelines, placeholders in args, etc.)
        self.lower_expression(desugared)
    }

    fn wrap_placeholder_lambda(
        &mut self,
        body: Expression,
        info: PlaceholderInfo,
    ) -> Result<Expression, Diagnostic> {
        info.validate()?;
        let param_count = info.param_count();
        let mut params = Vec::with_capacity(param_count);
        let mut names = Vec::with_capacity(param_count);
        for idx in 0..param_count {
            let name = format!("_arg{}", idx);
            params.push(Parameter {
                name: name.clone(),
                type_annotation: None,
                default: None,
                span: info.span,
            });
            names.push(name);
        }
        let replaced = info.replace_placeholders(body, &names);
        let block = Block::from_expression(replaced);
        let span = block.span;
        Ok(Expression::Lambda { params, body: block, span })
    }
}

struct ExprLowering {
    expr: Expression,
    placeholder: Option<PlaceholderInfo>,
}

impl ExprLowering {
    fn new(expr: Expression) -> Self {
        Self {
            expr,
            placeholder: None,
        }
    }

    fn new_with_placeholder(expr: Expression, placeholder: Option<PlaceholderInfo>) -> Self {
        Self { expr, placeholder }
    }

    fn with_placeholder(expr: Expression, placeholder: PlaceholderInfo) -> Self {
        Self {
            expr,
            placeholder: Some(placeholder),
        }
    }

    fn with_child(expr: Expression, placeholder: Option<PlaceholderInfo>) -> Self {
        Self { expr, placeholder }
    }

    fn with_children(
        expr: Expression,
        left: Option<PlaceholderInfo>,
        right: Option<PlaceholderInfo>,
    ) -> Self {
        Self {
            expr,
            placeholder: PlaceholderInfo::merge_option(left, right),
        }
    }

    fn expect_no_placeholders(self, message: &str) -> Result<Expression, Diagnostic> {
        if let Some(info) = self.placeholder {
            Err(info.diagnostic(message))
        } else {
            Ok(self.expr)
        }
    }
}

#[derive(Clone)]
struct PlaceholderInfo {
    indexes: BTreeSet<u32>,
    occurrences: Vec<(u32, Span)>,
    span: Span,
}

impl PlaceholderInfo {
    fn new(index: u32, span: Span) -> Self {
        let normalized = Self::normalize_index(index);
        let mut indexes = BTreeSet::new();
        indexes.insert(normalized);
        Self {
            indexes,
            occurrences: vec![(normalized, span)],
            span,
        }
    }

    fn normalize_index(index: u32) -> u32 {
        if index == 0 { 1 } else { index }
    }

    fn merge_option(
        left: Option<PlaceholderInfo>,
        right: Option<PlaceholderInfo>,
    ) -> Option<PlaceholderInfo> {
        match (left, right) {
            (Some(mut a), Some(b)) => {
                for idx in b.indexes {
                    a.indexes.insert(idx);
                }
                a.occurrences.extend(b.occurrences);
                a.span = a.span.join(b.span);
                Some(a)
            }
            (a @ Some(_), None) | (None, a @ Some(_)) => a,
            (None, None) => None,
        }
    }

    fn param_count(&self) -> usize {
        self.indexes.iter().copied().max().unwrap_or(0) as usize
    }

    fn validate(&self) -> Result<(), Diagnostic> {
        let max = self.indexes.iter().copied().max().unwrap_or(0);
        if max == 0 {
            return Err(self.diagnostic("invalid placeholder usage"));
        }
        for expected in 1..=max {
            if !self.indexes.contains(&expected) {
                return Err(
                    self.diagnostic(format!(
                        "missing placeholder ${expected} (placeholders must be contiguous from $ or $1)"
                    )),
                );
            }
        }
        Ok(())
    }

    fn replace_placeholders(&self, expr: Expression, names: &[String]) -> Expression {
        match expr {
            Expression::Placeholder(index, span) => {
                let normalized = Self::normalize_index(index);
                let param_idx = (normalized - 1) as usize;
                let name = names
                    .get(param_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("_arg{}", param_idx));
                Expression::Identifier(name, span)
            }
            Expression::Binary { op, left, right, span } => Expression::Binary {
                op,
                left: Box::new(self.replace_placeholders(*left, names)),
                right: Box::new(self.replace_placeholders(*right, names)),
                span,
            },
            Expression::Unary { op, expr, span } => Expression::Unary {
                op,
                expr: Box::new(self.replace_placeholders(*expr, names)),
                span,
            },
            Expression::Throw { value, span } => Expression::Throw {
                value: Box::new(self.replace_placeholders(*value, names)),
                span,
            },
            Expression::List(items, span) => Expression::List(
                items
                    .into_iter()
                    .map(|item| self.replace_placeholders(item, names))
                    .collect(),
                span,
            ),
            Expression::Map(entries, span) => Expression::Map(
                entries
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            self.replace_placeholders(k, names),
                            self.replace_placeholders(v, names),
                        )
                    })
                    .collect(),
                span,
            ),
            Expression::Call { callee, args, span, .. } => Expression::Call {
                callee: Box::new(self.replace_placeholders(*callee, names)),
                args: args
                    .into_iter()
                    .map(|arg| self.replace_placeholders(arg, names))
                    .collect(),
                arg_names: vec![],
                span,
            },
            Expression::Member { target, property, span } => Expression::Member {
                target: Box::new(self.replace_placeholders(*target, names)),
                property,
                span,
            },
            Expression::Index { target, index, span } => Expression::Index {
                target: Box::new(self.replace_placeholders(*target, names)),
                index: Box::new(self.replace_placeholders(*index, names)),
                span,
            },
            Expression::Ternary { condition, then_branch, else_branch, span } => Expression::Ternary {
                condition: Box::new(self.replace_placeholders(*condition, names)),
                then_branch: Box::new(self.replace_placeholders(*then_branch, names)),
                else_branch: Box::new(self.replace_placeholders(*else_branch, names)),
                span,
            },
            Expression::Match(expr) => Expression::Match(Box::new(MatchExpression {
                value: Box::new(self.replace_placeholders(*expr.value, names)),
                arms: expr
                    .arms
                    .into_iter()
                    .map(|mut arm| {
                        // S3.2: Replace placeholders in guard expression
                        if let Some(guard) = arm.guard.take() {
                            arm.guard = Some(Box::new(self.replace_placeholders(*guard, names)));
                        }
                        arm.body = self.replace_block_placeholders(arm.body, names);
                        arm
                    })
                    .collect(),
                default: expr
                    .default
                    .map(|block| Box::new(self.replace_block_placeholders(*block, names))),
                span: expr.span,
            })),
            Expression::Lambda { params, body, span } => Expression::Lambda { params, body, span },
            other => other,
        }
    }

    fn replace_block_placeholders(&self, mut block: Block, names: &[String]) -> Block {
        block.statements = block
            .statements
            .into_iter()
            .map(|statement| match statement {
                Statement::Binding(mut binding) => {
                    binding.value = self.replace_placeholders(binding.value, names);
                    Statement::Binding(binding)
                }
                Statement::Expression(expr) => {
                    Statement::Expression(self.replace_placeholders(expr, names))
                }
                Statement::Return(expr, span) => Statement::Return(
                    self.replace_placeholders(expr, names),
                    span,
                ),
                Statement::If { condition, body, elif_branches, else_body, span } => {
                    let condition = self.replace_placeholders(condition, names);
                    let body = self.replace_block_placeholders(body, names);
                    let elif_branches = elif_branches.into_iter().map(|(cond, blk)| {
                        (self.replace_placeholders(cond, names), self.replace_block_placeholders(blk, names))
                    }).collect();
                    let else_body = else_body.map(|blk| self.replace_block_placeholders(blk, names));
                    Statement::If { condition, body, elif_branches, else_body, span }
                }
                Statement::While { condition, body, span } => {
                    let condition = self.replace_placeholders(condition, names);
                    let body = self.replace_block_placeholders(body, names);
                    Statement::While { condition, body, span }
                }
                Statement::For { variable, iterable, body, span } => {
                    let iterable = self.replace_placeholders(iterable, names);
                    let body = self.replace_block_placeholders(body, names);
                    Statement::For { variable, iterable, body, span }
                }
                Statement::ForKV { key_var, value_var, iterable, body, span } => {
                    let iterable = self.replace_placeholders(iterable, names);
                    let body = self.replace_block_placeholders(body, names);
                    Statement::ForKV { key_var, value_var, iterable, body, span }
                }
                Statement::ForRange { variable, start, end, step, body, span } => {
                    let start = self.replace_placeholders(start, names);
                    let end = self.replace_placeholders(end, names);
                    let step = step.map(|s| self.replace_placeholders(s, names));
                    let body = self.replace_block_placeholders(body, names);
                    Statement::ForRange { variable, start, end, step, body, span }
                }
                Statement::Break(span) => Statement::Break(span),
                Statement::Continue(span) => Statement::Continue(span),
                Statement::FieldAssign { target, field, value, span } => {
                    let target = self.replace_placeholders(target, names);
                    let value = self.replace_placeholders(value, names);
                    Statement::FieldAssign { target, field, value, span }
                }
                Statement::PatternBinding { pattern, value, span } => {
                    let value = self.replace_placeholders(value, names);
                    Statement::PatternBinding { pattern, value, span }
                }
            })
            .collect();
        if let Some(value) = block.value.take() {
            block.value = Some(Box::new(self.replace_placeholders(*value, names)));
        }
        block
    }

    fn diagnostic(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(message, self.span).with_help(
            "Placeholders like `$`, `$1`, `$2`, ... are only valid inside function call arguments and must start at $ or $1 without gaps.",
        )
    }
}

// ── Pipeline helpers (S2.1) ──────────────────────────────────────────

/// Returns `true` if `expr` (or any sub-expression) contains a `$` placeholder.
fn expr_contains_placeholder(expr: &Expression) -> bool {
    match expr {
        Expression::Placeholder(_, _) => true,
        Expression::Binary { left, right, .. } => {
            expr_contains_placeholder(left) || expr_contains_placeholder(right)
        }
        Expression::Unary { expr: inner, .. } => expr_contains_placeholder(inner),
        Expression::Call { callee, args, .. } => {
            expr_contains_placeholder(callee) || args.iter().any(expr_contains_placeholder)
        }
        Expression::Member { target, .. } => expr_contains_placeholder(target),
        Expression::Index { target, index, .. } => {
            expr_contains_placeholder(target) || expr_contains_placeholder(index)
        }
        Expression::List(items, _) => items.iter().any(expr_contains_placeholder),
        Expression::Map(entries, _) => entries
            .iter()
            .any(|(k, v)| expr_contains_placeholder(k) || expr_contains_placeholder(v)),
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            expr_contains_placeholder(condition)
                || expr_contains_placeholder(then_branch)
                || expr_contains_placeholder(else_branch)
        }
        Expression::Throw { value, .. } => expr_contains_placeholder(value),
        _ => false,
    }
}

/// Replaces every `$` placeholder in `expr` with a clone of `replacement`.
fn replace_placeholder_in_expr(expr: Expression, replacement: &Expression) -> Expression {
    match expr {
        Expression::Placeholder(_, _) => replacement.clone(),
        Expression::Binary { op, left, right, span } => Expression::Binary {
            op,
            left: Box::new(replace_placeholder_in_expr(*left, replacement)),
            right: Box::new(replace_placeholder_in_expr(*right, replacement)),
            span,
        },
        Expression::Unary { op, expr: inner, span } => Expression::Unary {
            op,
            expr: Box::new(replace_placeholder_in_expr(*inner, replacement)),
            span,
        },
        Expression::Call { callee, args, span, .. } => Expression::Call {
            callee: Box::new(replace_placeholder_in_expr(*callee, replacement)),
            args: args
                .into_iter()
                .map(|a| replace_placeholder_in_expr(a, replacement))
                .collect(),
            arg_names: vec![],
            span,
        },
        Expression::Member { target, property, span } => Expression::Member {
            target: Box::new(replace_placeholder_in_expr(*target, replacement)),
            property,
            span,
        },
        Expression::Index { target, index, span } => Expression::Index {
            target: Box::new(replace_placeholder_in_expr(*target, replacement)),
            index: Box::new(replace_placeholder_in_expr(*index, replacement)),
            span,
        },
        Expression::List(items, span) => Expression::List(
            items
                .into_iter()
                .map(|i| replace_placeholder_in_expr(i, replacement))
                .collect(),
            span,
        ),
        Expression::Map(entries, span) => Expression::Map(
            entries
                .into_iter()
                .map(|(k, v)| {
                    (
                        replace_placeholder_in_expr(k, replacement),
                        replace_placeholder_in_expr(v, replacement),
                    )
                })
                .collect(),
            span,
        ),
        Expression::Ternary { condition, then_branch, else_branch, span } => {
            Expression::Ternary {
                condition: Box::new(replace_placeholder_in_expr(*condition, replacement)),
                then_branch: Box::new(replace_placeholder_in_expr(*then_branch, replacement)),
                else_branch: Box::new(replace_placeholder_in_expr(*else_branch, replacement)),
                span,
            }
        }
        Expression::Throw { value, span } => Expression::Throw {
            value: Box::new(replace_placeholder_in_expr(*value, replacement)),
            span,
        },
        other => other,
    }
}