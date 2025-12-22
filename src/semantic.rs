use crate::ast::{
    Binding,
    Block,
    Field,
    Function,
    Item,
    MatchExpression,
    Parameter,
    Program,
    Statement,
    Expression,
};
use crate::diagnostics::Diagnostic;
use crate::types::{
    AllocationHints,
    AllocationStrategy,
    ConstraintKind,
    ConstraintSet,
    Mutability,
    MutabilityEnv,
    Primitive,
    SymbolUsage,
    TypeEnv,
    TypeId,
    TypeGraph,
    UsageMetrics,
};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct SemanticModel {
    pub globals: Vec<Binding>,
    pub functions: Vec<Function>,
    pub extern_functions: Vec<crate::ast::ExternFunction>,
    pub stores: Vec<crate::ast::StoreDefinition>,
    pub constraints: ConstraintSet,
    pub types: TypeEnv,
    pub mutability: MutabilityEnv,
    pub allocation: AllocationHints,
    pub usage: UsageMetrics,
}

pub fn analyze(program: Program) -> Result<SemanticModel, Diagnostic> {
    let mut globals = Vec::new();
    let mut functions = Vec::new();
    let mut extern_functions = Vec::new();
    let mut stores = Vec::new();
    let mut seen_functions = HashSet::new();
    let mut types = TypeEnv::default();
    let mut global_scope = ScopeStack::new();
    global_scope.push();

    for item in program.items {
        match item {
            Item::Binding(binding) => {
                if let Some(previous) = global_scope.lookup(&binding.name) {
                    return Err(duplicate_symbol(
                        "binding",
                        &binding.name,
                        binding.span,
                        previous,
                    ));
                }
                global_scope.declare(binding.name.clone(), binding.span);
                globals.push(binding);
            }
            Item::ExternFunction(extern_fn) => {
                extern_functions.push(extern_fn);
            }
            Item::Function(function) => {
                if !seen_functions.insert(function.name.clone()) {
                    return Err(Diagnostic::new(
                        format!("duplicate function `{}`", function.name),
                        function.span,
                    ));
                }
                check_function(&function)?;
                functions.push(function);
            }
            Item::Expression(expr) => {
                let span = expr.span();
                globals.push(Binding {
                    name: format!("__expr{}", globals.len()),
                    type_annotation: None,
                    value: expr,
                    span,
                });
            }
            Item::Type(r#type) => {
                check_field_uniqueness("type", &r#type.name, &r#type.fields)?;
                // Special-case Message type: force data field to Any
                if r#type.name == "Message" {
                    for field in &r#type.fields {
                        if field.name == "data" {
                            types.insert(
                                "Message.data",
                                TypeId::Primitive(Primitive::Any),
                            );
                        }
                    }
                }
            }
            Item::Store(store) => {
                let kind = if store.is_actor { "actor" } else { "store" };
                check_field_uniqueness(kind, &store.name, &store.fields)?;
                if store.is_actor {
                    types.insert(store.name.clone(), TypeId::Primitive(Primitive::Actor));
                }
                stores.push(store);
            }
            Item::Taxonomy(_) => {}
        }
    }
    let mut constraints = ConstraintSet::default();
    let mut graph = TypeGraph::default();
    collect_program_constraints(&globals, &functions, &mut constraints, &mut types, &mut graph);
    if let Err(msg) = crate::types::solve_constraints(&constraints, &mut graph) {
        return Err(Diagnostic::new(format!("type inference failed: {msg}"), program.span));
    }
    // Resolve types after solving for easier diagnostics downstream.
    let mut resolved = TypeEnv::default();
    for (name, ty) in types.symbols.iter() {
        let mut g = graph.clone();
        let r = crate::types::resolve(ty.clone(), &mut g);
        resolved.insert(name.clone(), r);
    }

    let (usage, mutability, allocation) = infer_mutability_and_usage(&globals, &functions);

    Ok(SemanticModel {
        globals,
        functions,
        extern_functions,
        stores,
        constraints,
        types: resolved,
        mutability,
        allocation,
        usage,
    })
}

fn collect_program_constraints(
    globals: &[Binding],
    functions: &[Function],
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
) {
    for binding in globals {
        let ty = collect_constraints_expr(&binding.value, constraints, types, graph);
        if let Some(name) = Some(binding.name.clone()) {
            types.insert(name, ty);
        }
    }
    for function in functions {
        collect_function_constraints(function, constraints, types, graph);
    }
}

fn collect_function_constraints(
    function: &Function,
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
) {
    let mut params_tys = Vec::new();
    for param in &function.params {
        let ty = match &param.type_annotation {
            Some(ann) => type_from_annotation(ann),
            None => TypeId::TypeVar(graph.fresh()),
        };
        types.insert(param.name.clone(), ty.clone());
        params_tys.push(ty);
    }
    let body_ty = collect_block_constraints(&function.body, constraints, types, graph);
    let fn_ty = TypeId::Func(params_tys, Box::new(body_ty));
    types.insert(function.name.clone(), fn_ty);
}

fn collect_block_constraints(
    block: &Block,
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
) -> TypeId {
    for statement in &block.statements {
        match statement {
            crate::ast::Statement::Binding(binding) => {
                let rhs_ty = collect_constraints_expr(&binding.value, constraints, types, graph);
                if let Some(ann) = &binding.type_annotation {
                    let ann_ty = type_from_annotation(ann);
                    constraints.push(ConstraintKind::Equal(rhs_ty.clone(), ann_ty.clone()));
                    types.insert(binding.name.clone(), ann_ty);
                } else {
                    types.insert(binding.name.clone(), rhs_ty.clone());
                }
            }
            crate::ast::Statement::Expression(expr) => {
                let _ = collect_constraints_expr(expr, constraints, types, graph);
            }
            crate::ast::Statement::Return(expr, _) => {
                let _ = collect_constraints_expr(expr, constraints, types, graph);
            }
        }
    }
    if let Some(value) = &block.value {
        collect_constraints_expr(value, constraints, types, graph)
    } else {
        TypeId::Primitive(Primitive::Unit)
    }
}

fn collect_constraints_expr(
    expr: &Expression,
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
) -> TypeId {
    match expr {
        Expression::Integer(_, _) => TypeId::Primitive(Primitive::Int),
        Expression::Float(_, _) => TypeId::Primitive(Primitive::Float),
        Expression::Bool(_, _) => TypeId::Primitive(Primitive::Bool),
        Expression::String(_, _) => TypeId::Primitive(Primitive::String),
        Expression::Bytes(_, _) => TypeId::Primitive(Primitive::Bytes),
        Expression::Unit => TypeId::Primitive(Primitive::Unit),
        Expression::InlineAsm { .. } => TypeId::Unknown,
        Expression::PtrLoad { .. } => TypeId::Unknown,
        Expression::Unsafe { .. } => TypeId::Unknown,
        Expression::Identifier(name, _) => types
            .symbols
            .get(name)
            .cloned()
            .unwrap_or(TypeId::Unknown),
        Expression::Placeholder(id, _) => TypeId::Placeholder(*id),
        Expression::TaxonomyPath { .. } => TypeId::Primitive(Primitive::String),
        Expression::List(items, _) => {
            let elem_ty = if items.is_empty() {
                TypeId::TypeVar(graph.fresh())
            } else {
                let first = collect_constraints_expr(&items[0], constraints, types, graph);
                for item in &items[1..] {
                    let ty = collect_constraints_expr(item, constraints, types, graph);
                    constraints.push(ConstraintKind::Equal(first.clone(), ty));
                }
                first
            };
            TypeId::List(Box::new(elem_ty))
        }
        Expression::Map(entries, _) => {
            let key_ty = TypeId::TypeVar(graph.fresh());
            let val_ty = TypeId::TypeVar(graph.fresh());
            for (k, v) in entries {
                let kt = collect_constraints_expr(k, constraints, types, graph);
                let vt = collect_constraints_expr(v, constraints, types, graph);
                constraints.push(ConstraintKind::Equal(key_ty.clone(), kt));
                constraints.push(ConstraintKind::Equal(val_ty.clone(), vt));
            }
            TypeId::Map(Box::new(key_ty), Box::new(val_ty))
        }
        Expression::Binary { op, left, right, .. } => {
            let l = collect_constraints_expr(left, constraints, types, graph);
            let r = collect_constraints_expr(right, constraints, types, graph);
            match op {
                crate::ast::BinaryOp::Add => {
                    constraints.push(ConstraintKind::Equal(l.clone(), r.clone()));
                    l
                }
                crate::ast::BinaryOp::Sub
                | crate::ast::BinaryOp::Mul
                | crate::ast::BinaryOp::Div
                | crate::ast::BinaryOp::Mod
                | crate::ast::BinaryOp::BitAnd
                | crate::ast::BinaryOp::BitOr
                | crate::ast::BinaryOp::BitXor
                | crate::ast::BinaryOp::ShiftLeft
                | crate::ast::BinaryOp::ShiftRight => {
                    constraints.push(ConstraintKind::Numeric(l.clone()));
                    constraints.push(ConstraintKind::Numeric(r.clone()));
                    constraints.push(ConstraintKind::Equal(l.clone(), r.clone()));
                    l
                }
                crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or => {
                    constraints.push(ConstraintKind::Boolean(l.clone()));
                    constraints.push(ConstraintKind::Boolean(r.clone()));
                    TypeId::Primitive(Primitive::Bool)
                }
                crate::ast::BinaryOp::Equals | crate::ast::BinaryOp::NotEquals => {
                    constraints.push(ConstraintKind::Equal(l.clone(), r.clone()));
                    TypeId::Primitive(Primitive::Bool)
                }
                crate::ast::BinaryOp::Greater
                | crate::ast::BinaryOp::GreaterEq
                | crate::ast::BinaryOp::Less
                | crate::ast::BinaryOp::LessEq => {
                    constraints.push(ConstraintKind::Numeric(l.clone()));
                    constraints.push(ConstraintKind::Numeric(r.clone()));
                    TypeId::Primitive(Primitive::Bool)
                }
            }
        }
        Expression::Unary { op, expr, .. } => {
            let inner = collect_constraints_expr(expr, constraints, types, graph);
            match op {
                crate::ast::UnaryOp::Neg => {
                    constraints.push(ConstraintKind::Numeric(inner.clone()));
                    inner
                }
                crate::ast::UnaryOp::Not => {
                    constraints.push(ConstraintKind::Boolean(inner.clone()));
                    TypeId::Primitive(Primitive::Bool)
                }
                crate::ast::UnaryOp::BitNot => {
                    constraints.push(ConstraintKind::Numeric(inner.clone()));
                    inner
                }
            }
        }
        Expression::Call { callee, args, .. } => {
            let callee_ty = collect_constraints_expr(callee, constraints, types, graph);
            let mut arg_tys = Vec::new();
            for arg in args {
                arg_tys.push(collect_constraints_expr(arg, constraints, types, graph));
            }
            let result_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::Callable(callee_ty.clone(), arg_tys.clone(), result_ty.clone()));
            result_ty
        }
        Expression::Member { target, property, .. } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            match property.as_str() {
                "length" | "count" | "size" => TypeId::Primitive(Primitive::Int),
                _ => {
                    // Treat as map lookup with string key by default.
                    let val_ty = TypeId::TypeVar(graph.fresh());
                    let map_ty = TypeId::Map(Box::new(TypeId::Primitive(Primitive::String)), Box::new(val_ty.clone()));
                    constraints.push(ConstraintKind::Equal(target_ty, map_ty));
                    val_ty
                }
            }
        }
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            let cond_ty = collect_constraints_expr(condition, constraints, types, graph);
            let then_ty = collect_constraints_expr(then_branch, constraints, types, graph);
            let else_ty = collect_constraints_expr(else_branch, constraints, types, graph);
            constraints.push(ConstraintKind::Boolean(cond_ty));
            constraints.push(ConstraintKind::Equal(then_ty.clone(), else_ty.clone()));
            then_ty
        }
        Expression::Match(match_expr) => {
            let scrutinee_ty = collect_constraints_expr(&match_expr.value, constraints, types, graph);
            let mut arm_tys = Vec::new();
            for arm in &match_expr.arms {
                match &arm.pattern {
                    crate::ast::MatchPattern::Integer(_) => {
                        constraints.push(ConstraintKind::Numeric(scrutinee_ty.clone()));
                    }
                    crate::ast::MatchPattern::Bool(_) => {
                        constraints.push(ConstraintKind::Equal(
                            scrutinee_ty.clone(),
                            TypeId::Primitive(Primitive::Bool),
                        ));
                    }
                    crate::ast::MatchPattern::String(_) => {
                        constraints.push(ConstraintKind::Equal(scrutinee_ty.clone(), TypeId::Primitive(Primitive::String)));
                    }
                    crate::ast::MatchPattern::List(items) => {
                        let elem_ty = if items.is_empty() {
                            TypeId::TypeVar(graph.fresh())
                        } else {
                            let first = collect_constraints_expr(&items[0], constraints, types, graph);
                            for item in &items[1..] {
                                let t = collect_constraints_expr(item, constraints, types, graph);
                                constraints.push(ConstraintKind::Equal(first.clone(), t));
                            }
                            first
                        };
                        constraints.push(ConstraintKind::Equal(
                            scrutinee_ty.clone(),
                            TypeId::List(Box::new(elem_ty)),
                        ));
                    }
                    crate::ast::MatchPattern::Identifier(name) => {
                        types.insert(name.clone(), scrutinee_ty.clone());
                    }
                }
                let arm_ty = collect_block_constraints(&arm.body, constraints, types, graph);
                arm_tys.push(arm_ty);
            }
            if let Some(default) = &match_expr.default {
                arm_tys.push(collect_block_constraints(default, constraints, types, graph));
            }
            arm_tys
                .into_iter()
                .reduce(|a, b| {
                    constraints.push(ConstraintKind::Equal(a.clone(), b.clone()));
                    a
                })
                .unwrap_or(TypeId::Primitive(Primitive::Unit))
        }
        Expression::Throw { value, .. } => collect_constraints_expr(value, constraints, types, graph),
        Expression::Lambda { params, body, .. } => {
            let mut param_tys = Vec::new();
            let mut shadow = TypeEnv { symbols: types.symbols.clone(), undefined: Default::default() };
            for param in params {
                let ty = match &param.type_annotation {
                    Some(ann) => type_from_annotation(ann),
                    None => TypeId::TypeVar(graph.fresh()),
                };
                shadow.insert(param.name.clone(), ty.clone());
                param_tys.push(ty);
            }
            let mut nested_graph = graph.clone();
            let mut nested_constraints = ConstraintSet::default();
            let body_ty = collect_block_constraints(body, &mut nested_constraints, &mut shadow, &mut nested_graph);
            constraints.constraints.extend(nested_constraints.constraints);
            TypeId::Func(param_tys, Box::new(body_ty))
        }
    }    
}

fn type_from_annotation(ann: &crate::ast::TypeAnnotation) -> TypeId {
    if ann.segments.is_empty() {
        return TypeId::Unknown;
    }
    match ann.segments[0].as_str() {
        "Int" => TypeId::Primitive(Primitive::Int),
        "Float" => TypeId::Primitive(Primitive::Float),
        "Bool" => TypeId::Primitive(Primitive::Bool),
        "String" => TypeId::Primitive(Primitive::String),
        "Bytes" => TypeId::Primitive(Primitive::Bytes),
        "Unit" => TypeId::Primitive(Primitive::Unit),
        "Any" => TypeId::Primitive(Primitive::Any),
        "Actor" => TypeId::Primitive(Primitive::Actor),
        other => TypeId::Placeholder(other.len() as u32),
    }
}

fn check_function(function: &Function) -> Result<(), Diagnostic> {
    validate_parameter_defaults(&function.params)?;
    let mut scopes = ScopeStack::new();
    scopes.push();
    for param in &function.params {
        if let Some(previous) = scopes.lookup(&param.name) {
            return Err(duplicate_symbol("parameter", &param.name, param.span, previous));
        }
        scopes.declare(param.name.clone(), param.span);
    }
    check_block(&function.body, &mut scopes)
}

fn check_block(block: &Block, scopes: &mut ScopeStack) -> Result<(), Diagnostic> {
    scopes.push();
    for statement in &block.statements {
        match statement {
            Statement::Binding(binding) => {
                if let Some(previous) = scopes.lookup(&binding.name) {
                    return Err(duplicate_symbol("binding", &binding.name, binding.span, previous));
                }
                scopes.declare(binding.name.clone(), binding.span);
                check_expression(&binding.value, scopes)?;
            }
            Statement::Expression(expr) => check_expression(expr, scopes)?,
            Statement::Return(expr, _) => check_expression(expr, scopes)?,
        }
    }
    if let Some(value) = &block.value {
        check_expression(value, scopes)?;
    }
    scopes.pop();
    Ok(())
}

fn check_expression(expr: &Expression, scopes: &mut ScopeStack) -> Result<(), Diagnostic> {
    match expr {
        Expression::Binary { left, right, .. } => {
            check_expression(left, scopes)?;
            check_expression(right, scopes)?;
        }
        Expression::Unary { expr, .. } => check_expression(expr, scopes)?,
        Expression::List(items, _) => {
            for item in items {
                check_expression(item, scopes)?;
            }
        }
        Expression::Map(entries, _) => {
            for (key, value) in entries {
                check_expression(key, scopes)?;
                check_expression(value, scopes)?;
            }
        }
        Expression::Call { callee, args, .. } => {
            check_expression(callee, scopes)?;
            for arg in args {
                check_expression(arg, scopes)?;
            }
        }
        Expression::Member { target, .. } => check_expression(target, scopes)?,
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            check_expression(condition, scopes)?;
            check_expression(then_branch, scopes)?;
            check_expression(else_branch, scopes)?;
        }
        Expression::Match(match_expr) => check_match_expression(match_expr, scopes)?,
    Expression::Throw { value, .. } => check_expression(value, scopes)?,
    Expression::Lambda { params, body, .. } => check_lambda(params, body, scopes)?,
    Expression::String(_, _)
    | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::Float(_, _)
        | Expression::Integer(_, _)
        | Expression::TaxonomyPath { .. }
        | Expression::Placeholder(_, _)
        | Expression::Identifier(_, _)
        | Expression::InlineAsm { .. }
        | Expression::PtrLoad { .. }
        | Expression::Unsafe { .. }
        | Expression::Unit => {}
    }
    Ok(())
}fn check_match_expression(expr: &MatchExpression, scopes: &mut ScopeStack) -> Result<(), Diagnostic> {
    check_expression(&expr.value, scopes)?;
    for arm in &expr.arms {
        check_block(&arm.body, scopes)?;
    }
    if let Some(default) = &expr.default {
        check_block(default, scopes)?;
    }
    Ok(())
}

fn check_field_uniqueness(owner_kind: &str, owner_name: &str, fields: &[Field]) -> Result<(), Diagnostic> {
    let mut seen = HashMap::new();
    for field in fields {
        if let Some(previous) = seen.insert(field.name.clone(), field.span) {
            return Err(
                Diagnostic::new(
                    format!("duplicate field `{}.{}`", owner_name, field.name),
                    field.span,
                )
                .with_help(format!(
                    "previous {} field defined at {}",
                    owner_kind, previous
                )),
            );
        }
    }
    Ok(())
}

fn check_lambda(params: &[Parameter], body: &Block, scopes: &mut ScopeStack) -> Result<(), Diagnostic> {
    scopes.push();
    for param in params {
        if let Some(previous) = scopes.lookup(&param.name) {
            return Err(duplicate_symbol("parameter", &param.name, param.span, previous));
        }
        scopes.declare(param.name.clone(), param.span);
        if let Some(default) = &param.default {
            check_expression(default, scopes)?;
        }
    }
    check_block(body, scopes)?;
    scopes.pop();
    Ok(())
}

fn duplicate_symbol(kind: &str, name: &str, span: Span, previous: Span) -> Diagnostic {
    Diagnostic::new(format!("duplicate {} `{}`", kind, name), span)
        .with_help(format!("previous definition at {}", previous))
}

fn infer_mutability_and_usage(
    globals: &[Binding],
    functions: &[Function],
) -> (UsageMetrics, MutabilityEnv, AllocationHints) {
    let mut tracker = UsageTracker::default();

    for binding in globals {
        tracker.touch(&binding.name);
        visit_expression(&binding.value, &mut tracker);
    }

    for function in functions {
        for param in &function.params {
            tracker.touch(&param.name);
        }
        visit_block(&function.body, &mut tracker, true);
    }

    let mut mut_env = MutabilityEnv::default();
    let mut alloc = AllocationHints::default();
    for (name, usage) in tracker.usage.iter() {
        let mutability = if usage.mutations == 0 {
            if usage.escapes == 0 {
                Mutability::Immutable
            } else {
                Mutability::EffectivelyImmutable
            }
        } else {
            Mutability::Mutable
        };

        let strategy = match mutability {
            Mutability::Immutable if usage.escapes == 0 => AllocationStrategy::Stack,
            Mutability::Immutable | Mutability::EffectivelyImmutable => AllocationStrategy::SharedCow,
            Mutability::Mutable => AllocationStrategy::Heap,
            Mutability::Unknown => AllocationStrategy::Unknown,
        };

        mut_env.insert(name.clone(), mutability);
        alloc.insert(name.clone(), strategy);
    }

    (UsageMetrics { symbols: tracker.usage }, mut_env, alloc)
}

#[derive(Default)]
struct UsageTracker {
    usage: HashMap<String, SymbolUsage>,
}

impl UsageTracker {
    fn touch(&mut self, name: &str) {
        self.usage.entry(name.to_string()).or_insert_with(SymbolUsage::default);
    }

    fn read(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_insert_with(SymbolUsage::default);
        entry.reads += 1;
    }

    fn mutate(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_insert_with(SymbolUsage::default);
        entry.mutations += 1;
    }

    fn escape(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_insert_with(SymbolUsage::default);
        entry.escapes += 1;
    }

    fn call(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_insert_with(SymbolUsage::default);
        entry.calls += 1;
    }
}

fn visit_block(block: &Block, tracker: &mut UsageTracker, mark_returns_as_escape: bool) {
    for stmt in &block.statements {
        match stmt {
            Statement::Binding(binding) => {
                tracker.touch(&binding.name);
                visit_expression(&binding.value, tracker);
            }
            Statement::Expression(expr) => visit_expression(expr, tracker),
            Statement::Return(expr, _) => {
                visit_expression(expr, tracker);
                if mark_returns_as_escape {
                    mark_escapes(expr, tracker);
                }
            }
        }
    }
    if let Some(value) = &block.value {
        visit_expression(value, tracker);
        if mark_returns_as_escape {
            mark_escapes(value, tracker);
        }
    }
}

fn visit_expression(expr: &Expression, tracker: &mut UsageTracker) {
    match expr {
        Expression::Identifier(name, _) => tracker.read(name),
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Unit
        | Expression::Placeholder(_, _)
        | Expression::TaxonomyPath { .. } => {}
        Expression::List(items, _) => {
            for item in items {
                visit_expression(item, tracker);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                visit_expression(k, tracker);
                visit_expression(v, tracker);
            }
        }
        Expression::Binary { left, right, .. } => {
            visit_expression(left, tracker);
            visit_expression(right, tracker);
        }
        Expression::Unary { expr, .. } => visit_expression(expr, tracker),
        Expression::Member { target, .. } => visit_expression(target, tracker),
        Expression::Call { callee, args, .. } => {
            visit_expression(callee, tracker);
            if let Expression::Identifier(name, _) = callee.as_ref() {
                tracker.call(name);
            }
            if let Expression::Member { target, property, .. } = callee.as_ref() {
                visit_expression(target, tracker);
                if MUTATING_METHODS.contains(&property.as_str()) {
                    if let Some(id) = identifier_name(target) {
                        tracker.mutate(id);
                    }
                }
            }
            for arg in args {
                visit_expression(arg, tracker);
                mark_escapes(arg, tracker);
            }
        }
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            visit_expression(condition, tracker);
            visit_expression(then_branch, tracker);
            visit_expression(else_branch, tracker);
        }
        Expression::Match(match_expr) => {
            visit_expression(&match_expr.value, tracker);
            for arm in &match_expr.arms {
                visit_block(&arm.body, tracker, false);
            }
            if let Some(default) = &match_expr.default {
                visit_block(default, tracker, false);
            }
        }
        Expression::Throw { value, .. } => visit_expression(value, tracker),
        Expression::Lambda { params, body, .. } => {
            for p in params {
                tracker.touch(&p.name);
            }
            visit_block(body, tracker, false);
        }
        Expression::InlineAsm { .. } | Expression::PtrLoad { .. } | Expression::Unsafe { .. } => {}
    }
}

fn mark_escapes(expr: &Expression, tracker: &mut UsageTracker) {
    match expr {
        Expression::Identifier(name, _) => tracker.escape(name),
        Expression::List(items, _) => {
            for item in items {
                mark_escapes(item, tracker);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                mark_escapes(k, tracker);
                mark_escapes(v, tracker);
            }
        }
        Expression::Binary { left, right, .. } => {
            mark_escapes(left, tracker);
            mark_escapes(right, tracker);
        }
        Expression::Unary { expr, .. } => mark_escapes(expr, tracker),
        Expression::Call { callee, args, .. } => {
            mark_escapes(callee, tracker);
            for arg in args {
                mark_escapes(arg, tracker);
            }
        }
        Expression::Member { target, .. } => mark_escapes(target, tracker),
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            mark_escapes(condition, tracker);
            mark_escapes(then_branch, tracker);
            mark_escapes(else_branch, tracker);
        }
        Expression::Match(match_expr) => {
            mark_escapes(&match_expr.value, tracker);
            for arm in &match_expr.arms {
                for stmt in &arm.body.statements {
                    if let Statement::Expression(e) = stmt {
                        mark_escapes(e, tracker);
                    }
                }
                if let Some(value) = &arm.body.value {
                    mark_escapes(value, tracker);
                }
            }
            if let Some(default) = &match_expr.default {
                for stmt in &default.statements {
                    if let Statement::Expression(e) = stmt {
                        mark_escapes(e, tracker);
                    }
                }
                if let Some(value) = &default.value {
                    mark_escapes(value, tracker);
                }
            }
        }
        Expression::Throw { value, .. } => mark_escapes(value, tracker),
        Expression::Lambda { body, .. } => {
            visit_block(body, tracker, false);
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Unit
        | Expression::Placeholder(_, _)
        | Expression::InlineAsm { .. }
        | Expression::PtrLoad { .. }
        | Expression::Unsafe { .. }
        | Expression::TaxonomyPath { .. } => {}
    }
}

fn identifier_name(expr: &Expression) -> Option<&str> {
    if let Expression::Identifier(name, _) = expr {
        Some(name.as_str())
    } else {
        None
    }
}

const MUTATING_METHODS: [&str; 9] = [
    "push",
    "pop",
    "append",
    "insert",
    "remove",
    "clear",
    "update",
    "set",
    "add_item",
];

fn validate_parameter_defaults(params: &[Parameter]) -> Result<(), Diagnostic> {
    for (index, param) in params.iter().enumerate() {
        if let Some(default_expr) = &param.default {
            let mut forbidden = HashMap::new();
            for later in &params[index + 1..] {
                forbidden.insert(later.name.clone(), later.span);
            }
            if let Some((name, ident_span, later_span)) =
                find_forbidden_identifier(default_expr, &forbidden)
            {
                return Err(
                    Diagnostic::new(
                        format!(
                            "default for parameter `{}` references later parameter `{}`",
                            param.name, name
                        ),
                        ident_span,
                    )
                    .with_help(format!(
                        "Parameter `{}` is declared later at {}. Reorder parameters or remove the reference.",
                        name, later_span
                    )),
                );
            }
        }
    }
    Ok(())
}

fn find_forbidden_identifier(
    expr: &Expression,
    forbidden: &HashMap<String, Span>,
) -> Option<(String, Span, Span)> {
    match expr {
        Expression::Identifier(name, span) => forbidden
            .get(name)
            .copied()
            .map(|later_span| (name.clone(), *span, later_span)),
        Expression::Binary { left, right, .. } => {
            find_forbidden_identifier(left, forbidden)
                .or_else(|| find_forbidden_identifier(right, forbidden))
        }
        Expression::Unary { expr, .. } => find_forbidden_identifier(expr, forbidden),
        Expression::List(items, _) => items
            .iter()
            .find_map(|item| find_forbidden_identifier(item, forbidden)),
        Expression::Map(entries, _) => entries.iter().find_map(|(key, value)| {
            find_forbidden_identifier(key, forbidden)
                .or_else(|| find_forbidden_identifier(value, forbidden))
        }),
        Expression::Call { callee, args, .. } => find_forbidden_identifier(callee, forbidden)
            .or_else(|| args.iter().find_map(|arg| find_forbidden_identifier(arg, forbidden))),
        Expression::Member { target, .. } => find_forbidden_identifier(target, forbidden),
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => find_forbidden_identifier(condition, forbidden)
            .or_else(|| find_forbidden_identifier(then_branch, forbidden))
            .or_else(|| find_forbidden_identifier(else_branch, forbidden)),
        Expression::Match(match_expr) => find_in_match(match_expr, forbidden),
        Expression::Throw { value, .. } => find_forbidden_identifier(value, forbidden),
    Expression::Lambda { body, .. } => find_in_block(body, forbidden),
    Expression::String(_, _)
    | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::Float(_, _)
        | Expression::Integer(_, _)
        | Expression::TaxonomyPath { .. }
        | Expression::Placeholder(_, _)
        | Expression::InlineAsm { .. }
        | Expression::PtrLoad { .. }
        | Expression::Unsafe { .. }
        | Expression::Unit => None,
    }
}

fn find_in_match(
    match_expr: &MatchExpression,
    forbidden: &HashMap<String, Span>,
) -> Option<(String, Span, Span)> {
    find_forbidden_identifier(&match_expr.value, forbidden)
        .or_else(|| {
            match_expr
                .arms
                .iter()
                .find_map(|arm| find_in_block(&arm.body, forbidden))
        })
        .or_else(|| match_expr.default.as_ref().and_then(|block| find_in_block(block, forbidden)))
}

fn find_in_block(
    block: &Block,
    forbidden: &HashMap<String, Span>,
) -> Option<(String, Span, Span)> {
    for statement in &block.statements {
        if let Some(hit) = find_in_statement(statement, forbidden) {
            return Some(hit);
        }
    }
    block
        .value
        .as_ref()
        .and_then(|value| find_forbidden_identifier(value, forbidden))
}

fn find_in_statement(
    statement: &Statement,
    forbidden: &HashMap<String, Span>,
) -> Option<(String, Span, Span)> {
    match statement {
        Statement::Binding(binding) => find_forbidden_identifier(&binding.value, forbidden),
        Statement::Expression(expr) => find_forbidden_identifier(expr, forbidden),
        Statement::Return(expr, _) => find_forbidden_identifier(expr, forbidden),
    }
}

struct ScopeStack {
    frames: Vec<HashMap<String, Span>>,
}

impl ScopeStack {
    fn new() -> Self {
        Self { frames: Vec::new() }
    }

    fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.frames.pop();
    }

    fn lookup(&self, name: &str) -> Option<Span> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(name).copied())
    }

    fn declare(&mut self, name: String, span: Span) {
        if let Some(frame) = self.frames.last_mut() {
            frame.insert(name, span);
        }
    }
}

pub fn synthetic_span() -> Span {
    Span::new(0, 0)
}
