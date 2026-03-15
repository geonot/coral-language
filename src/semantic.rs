use crate::ast::{
    BinaryOp, Binding, Block, ErrorDefinition, Expression, Field, Function, FunctionKind, Item,
    MatchExpression, MatchPattern, Parameter, Program, Statement, TraitDefinition, UnaryOp,
};
use crate::diagnostics::{Diagnostic, WarningCategory};
use crate::span::Span;
use crate::types::{
    AllocationHints, AllocationStrategy, ConstraintKind, ConstraintSet, Mutability, MutabilityEnv,
    Primitive, SymbolUsage, TraitRegistry, TypeEnv, TypeGraph, TypeId, UsageMetrics,
};
use std::collections::{HashMap, HashSet};

/// Escape analysis results for a function's local variables.
#[derive(Debug, Clone, Default)]
pub struct EscapeInfo {
    /// Set of (fn_name, var_name) for locals that do NOT escape the function.
    pub non_escaping: HashSet<(String, String)>,
    /// Subset of non_escaping that are eligible for stack allocation.
    pub stack_eligible: HashSet<(String, String)>,
}

/// Information about functions eligible for monomorphization.
#[derive(Debug, Clone, Default)]
pub struct MonomorphInfo {
    /// Maps function name → list of specialized type variants
    pub candidates: HashMap<String, Vec<MonomorphVariant>>,
}

/// A specific type-specialization variant of a function.
#[derive(Debug, Clone)]
pub struct MonomorphVariant {
    pub param_types: Vec<TypeId>,
    pub return_type: TypeId,
    pub call_count: usize,
}

#[derive(Debug, Clone)]
pub struct SemanticModel {
    pub globals: Vec<Binding>,
    pub functions: Vec<Function>,
    pub extern_functions: Vec<crate::ast::ExternFunction>,
    pub stores: Vec<crate::ast::StoreDefinition>,
    pub type_defs: Vec<crate::ast::TypeDefinition>,
    pub trait_defs: Vec<TraitDefinition>,
    pub error_defs: Vec<ErrorDefinition>,
    pub constraints: ConstraintSet,
    pub types: TypeEnv,
    pub mutability: MutabilityEnv,
    pub allocation: AllocationHints,
    pub usage: UsageMetrics,
    pub warnings: Vec<Diagnostic>,

    pub field_types: HashMap<(String, String), usize>,

    pub store_field_names: HashMap<String, Vec<String>>,

    pub module_exports: HashMap<String, Vec<String>>,

    pub actor_message_types: HashMap<String, String>,

    pub actor_handler_names: HashMap<String, Vec<String>>,

    pub trait_registry: TraitRegistry,

    pub monomorphizations: HashMap<String, Vec<Vec<TypeId>>>,

    pub unboxed_number_lists: HashSet<String>,

    /// Maps (fn_name, var_name) → element TypeId for lists with uniform element types
    pub typed_lists: HashMap<(String, String), TypeId>,

    pub store_field_indices: HashMap<(String, String), u32>,

    pub specialized_stores: HashSet<String>,

    /// Maps (fn_name, var_name) → resolved TypeId for local bindings
    pub resolved_locals: HashMap<(String, String), TypeId>,

    /// Maps (fn_name, param_index) → resolved TypeId for function parameters
    pub resolved_params: HashMap<(String, usize), TypeId>,

    /// Maps fn_name → resolved return TypeId for functions
    pub resolved_returns: HashMap<String, TypeId>,

    /// Escape analysis: which locals don't escape their function
    pub escape_info: EscapeInfo,

    /// Monomorphization candidates: functions with consistent type profiles
    pub monomorph_info: MonomorphInfo,
}

fn register_error_definitions(
    def: &ErrorDefinition,
    prefix: &str,
    known_names: &mut HashSet<String>,
) {
    let full_name = if prefix.is_empty() {
        def.name.clone()
    } else {
        format!("{}:{}", prefix, def.name)
    };

    known_names.insert(full_name.clone());

    for child in &def.children {
        register_error_definitions(child, &full_name, known_names);
    }
}

pub fn analyze(program: Program) -> Result<SemanticModel, Diagnostic> {
    let mut module_exports: HashMap<String, Vec<String>> = HashMap::new();
    for module in &program.modules {
        let short_name = module
            .name
            .rsplit('.')
            .next()
            .unwrap_or(&module.name)
            .to_string();
        module_exports.insert(short_name, module.exports.clone());

        if module.name.contains('.') {
            module_exports.insert(module.name.clone(), module.exports.clone());
        }
    }

    let mut globals = Vec::new();
    let mut functions = Vec::new();
    let mut extern_functions = Vec::new();
    let mut stores = Vec::new();
    let mut type_defs = Vec::new();
    let mut trait_defs = Vec::new();
    let mut error_defs = Vec::new();
    let mut seen_functions = HashSet::new();
    let mut types = TypeEnv::default();
    let mut global_scope = ScopeStack::new();
    global_scope.push();

    let mut constructor_owners: HashMap<String, String> = HashMap::new();

    let mut field_types: HashMap<(String, String), usize> = HashMap::new();
    let mut store_field_names: HashMap<String, Vec<String>> = HashMap::new();

    let mut actor_message_types: HashMap<String, String> = HashMap::new();
    let mut actor_handler_names: HashMap<String, Vec<String>> = HashMap::new();

    let mut known_names: HashSet<String> = HashSet::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                known_names.insert(function.name.clone());
            }
            Item::ExternFunction(extern_fn) => {
                known_names.insert(extern_fn.name.clone());
            }
            Item::Store(store) => {
                known_names.insert(format!("make_{}", store.name));

                known_names.insert(store.name.clone());
            }
            Item::Binding(binding) => {
                known_names.insert(binding.name.clone());
            }
            Item::Type(r#type) => {
                known_names.insert(r#type.name.clone());

                for variant in &r#type.variants {
                    known_names.insert(variant.name.clone());
                }
            }
            Item::TraitDefinition(trait_def) => {
                known_names.insert(trait_def.name.clone());
            }
            _ => {}
        }
    }

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
                check_function(&function, &known_names)?;
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

                if r#type.name == "Message" {
                    for field in &r#type.fields {
                        if field.name == "data" {
                            types.insert(
                                "Message.data".to_string(),
                                TypeId::Primitive(Primitive::Any),
                            );
                        }
                    }
                }

                if !r#type.variants.is_empty() {
                    let has_type_params = !r#type.type_params.is_empty();
                    if has_type_params {
                        types.register_generic_type(r#type.name.clone(), r#type.param_names());

                        for tp in &r#type.type_params {
                            if !tp.bounds.is_empty() {
                                types.register_type_param_bounds(
                                    &r#type.name,
                                    &tp.name,
                                    tp.bounds.clone(),
                                );
                            }
                            if tp.is_const {
                                types.register_const_param(&r#type.name, &tp.name);
                            }
                        }
                    }

                    types.insert(
                        r#type.name.clone(),
                        TypeId::Adt(r#type.name.clone(), vec![]),
                    );

                    for variant in &r#type.variants {
                        let ctor_name = variant.name.clone();
                        let adt_type = TypeId::Adt(r#type.name.clone(), vec![]);

                        if let Some(other_enum) = constructor_owners.get(&ctor_name) {
                            if other_enum != &r#type.name {
                                return Err(Diagnostic::new(
                                    format!(
                                        "constructor `{}` is already defined in enum `{}`, cannot reuse in enum `{}`",
                                        ctor_name, other_enum, r#type.name
                                    ),
                                    r#type.span,
                                ));
                            }
                        }
                        constructor_owners.insert(ctor_name.clone(), r#type.name.clone());

                        if has_type_params {
                            types.register_generic_constructor(
                                ctor_name.clone(),
                                r#type.name.clone(),
                                r#type
                                    .type_params
                                    .iter()
                                    .map(|tp| tp.name.clone())
                                    .collect(),
                                variant.fields.len(),
                            );
                        }

                        if variant.fields.is_empty() {
                            types.insert(ctor_name.clone(), adt_type);
                        } else {
                            let param_types: Vec<TypeId> = variant
                                .fields
                                .iter()
                                .map(|_| TypeId::Primitive(Primitive::Any))
                                .collect();
                            let return_type = Box::new(adt_type);

                            types.insert(ctor_name.clone(), TypeId::Func(param_types, return_type));
                        }

                        known_names.insert(ctor_name);
                    }
                }

                {
                    let names: Vec<String> = r#type.fields.iter().map(|f| f.name.clone()).collect();
                    for (i, field) in r#type.fields.iter().enumerate() {
                        field_types.insert((r#type.name.clone(), field.name.clone()), i);
                    }
                    store_field_names.insert(r#type.name.clone(), names);
                }

                for method in &r#type.methods {
                    check_method_with_fields(method, &r#type.fields, &known_names)?;
                }

                type_defs.push(r#type);
            }
            Item::Store(store) => {
                let kind = if store.is_actor { "actor" } else { "store" };
                check_field_uniqueness(kind, &store.name, &store.fields)?;
                if store.is_actor {
                    types.insert(store.name.clone(), TypeId::Primitive(Primitive::Actor));

                    for method in &store.methods {
                        if method.kind == crate::ast::FunctionKind::ActorMessage
                            && method.params.len() > 1
                        {
                            return Err(Diagnostic::new(
                                format!(
                                    "actor message handler `@{}` has {} parameters, but handlers can have at most 1 (message payload)",
                                    method.name,
                                    method.params.len()
                                ),
                                method.span,
                            ));
                        }
                    }

                    let handlers: Vec<String> = store
                        .methods
                        .iter()
                        .filter(|m| m.kind == crate::ast::FunctionKind::ActorMessage)
                        .map(|m| m.name.clone())
                        .collect();
                    actor_handler_names.insert(store.name.clone(), handlers);

                    if let Some(ref msg_type) = store.message_type {
                        actor_message_types.insert(store.name.clone(), msg_type.clone());
                    }
                } else {
                    let ctor_name = format!("make_{}", store.name);
                    types.insert(
                        ctor_name,
                        TypeId::Func(vec![], Box::new(TypeId::Store(store.name.clone()))),
                    );
                }

                {
                    let names: Vec<String> = store.fields.iter().map(|f| f.name.clone()).collect();
                    for (i, field) in store.fields.iter().enumerate() {
                        field_types.insert((store.name.clone(), field.name.clone()), i);
                    }
                    store_field_names.insert(store.name.clone(), names);
                }

                for method in &store.methods {
                    check_method_with_fields(method, &store.fields, &known_names)?;
                }
                stores.push(store);
            }
            Item::Taxonomy(_) => {}
            Item::ErrorDefinition(error_def) => {
                register_error_definitions(&error_def, "", &mut known_names);

                error_defs.push(error_def);
            }
            Item::TraitDefinition(trait_def) => {
                known_names.insert(trait_def.name.clone());

                for method in &trait_def.methods {
                    let param_types: Vec<TypeId> = method
                        .params
                        .iter()
                        .map(|_| TypeId::Primitive(Primitive::Any))
                        .collect();
                    types.insert(
                        format!("{}::{}", trait_def.name, method.name),
                        TypeId::Func(param_types, Box::new(TypeId::Primitive(Primitive::Any))),
                    );
                }

                trait_defs.push(trait_def);
            }
            Item::Extension(ext) => {
                let target = &ext.target_type;
                let mut merged = false;

                for store in stores.iter_mut() {
                    if store.name == *target {
                        for method in &ext.methods {
                            let already_exists =
                                store.methods.iter().any(|m| m.name == method.name);
                            if !already_exists {
                                let mut m = method.clone();
                                m.kind = FunctionKind::Method;
                                store.methods.push(m);
                            }
                        }
                        merged = true;
                        break;
                    }
                }

                if !merged {
                    for type_def in type_defs.iter_mut() {
                        if type_def.name == *target {
                            for method in &ext.methods {
                                let already_exists =
                                    type_def.methods.iter().any(|m| m.name == method.name);
                                if !already_exists {
                                    let mut m = method.clone();
                                    m.kind = FunctionKind::Method;
                                    type_def.methods.push(m);
                                }
                            }
                            merged = true;
                            break;
                        }
                    }
                }

                if !merged {
                    let builtin_types = [
                        "String", "List", "Map", "Int", "Float", "Bool", "Number", "Bytes",
                    ];
                    if builtin_types.contains(&target.as_str()) {
                        let methods: Vec<Function> = ext
                            .methods
                            .iter()
                            .map(|m| {
                                let mut func = m.clone();
                                func.kind = FunctionKind::Method;
                                func
                            })
                            .collect();
                        let synthetic = crate::ast::StoreDefinition {
                            name: target.clone(),
                            with_traits: vec![],
                            fields: vec![],
                            methods,
                            is_actor: false,
                            is_persistent: false,
                            message_type: None,
                            span: ext.span,
                        };
                        stores.push(synthetic);
                        merged = true;
                    }
                }

                if !merged {}
            }
        }
    }
    let mut constraints = ConstraintSet::default();
    let mut graph = TypeGraph::default();

    let mut branch_type_hints: Vec<(Vec<TypeId>, Span)> = Vec::new();
    collect_program_constraints(
        &globals,
        &functions,
        &mut constraints,
        &mut types,
        &mut graph,
        &mut branch_type_hints,
    );

    let mut trait_registry = TraitRegistry::new();
    for td in &type_defs {
        for trait_name in &td.with_traits {
            trait_registry.register_impl(&td.name, trait_name);
        }
    }
    for sd in &stores {
        for trait_name in &sd.with_traits {
            trait_registry.register_impl(&sd.name, trait_name);
        }
    }
    for trd in &trait_defs {
        trait_registry.register_super_traits(&trd.name, trd.required_traits.clone());
    }

    if let Err(errors) = crate::types::solve_constraints(&constraints, &mut graph, &trait_registry)
    {
        let first = &errors[0];
        let mut msg = format!("type inference failed: {}", first.message);

        if let Some(ref origin) = first.expected_origin {
            msg.push_str(&format!(
                "\n  {} inferred from: {}",
                first
                    .expected
                    .as_ref()
                    .map(|t| crate::types::format_type(t))
                    .unwrap_or_default(),
                origin.description
            ));
        }
        if let Some(ref origin) = first.found_origin {
            msg.push_str(&format!(
                "\n  {} required by: {}",
                first
                    .found
                    .as_ref()
                    .map(|t| crate::types::format_type(t))
                    .unwrap_or_default(),
                origin.description
            ));
        }
        let mut primary = Diagnostic::new(msg, first.span);

        for error in errors.iter().skip(1) {
            let mut related_msg = error.message.clone();
            if let Some(ref origin) = error.expected_origin {
                related_msg.push_str(&format!(
                    "\n  {} inferred from: {}",
                    error
                        .expected
                        .as_ref()
                        .map(|t| crate::types::format_type(t))
                        .unwrap_or_default(),
                    origin.description
                ));
            }
            if let Some(ref origin) = error.found_origin {
                related_msg.push_str(&format!(
                    "\n  {} required by: {}",
                    error
                        .found
                        .as_ref()
                        .map(|t| crate::types::format_type(t))
                        .unwrap_or_default(),
                    origin.description
                ));
            }
            primary
                .related
                .push(Diagnostic::new(related_msg, error.span));
        }
        return Err(primary);
    }

    let mut resolved = TypeEnv::default();
    for (name, ty) in types.iter_all() {
        let mut g = graph.clone();
        let r = crate::types::resolve(ty.clone(), &mut g);
        resolved.insert(name, r);
    }

    let (usage, mutability, allocation) = infer_mutability_and_usage(&globals, &functions);

    let mut warnings = Vec::new();

    for (name, ty) in resolved.iter_all() {
        if ty.contains_unknown()
            && !name.starts_with('$')
            && !name.contains("::")
            && !is_builtin_name(&name)
        {
            warnings.push(Diagnostic::categorized_warning(
                format!(
                    "type of `{}` could not be fully inferred (contains Unknown)",
                    name
                ),
                Span::new(0, 0),
                WarningCategory::General,
            ));
        }
    }

    check_all_match_exhaustiveness(&globals, &functions, &type_defs, &mut warnings);
    check_unhandled_errors(&globals, &functions, &mut warnings);

    check_error_type_exhaustiveness(&functions, &mut warnings);

    check_dead_code(&functions, &mut warnings);

    check_definite_assignment(&functions, &mut warnings);

    check_branch_type_consistency(&branch_type_hints, &mut graph, &mut warnings);

    check_member_access_validity(&globals, &functions, &store_field_names, &mut warnings);

    check_nullability_returns(&functions, &mut warnings);

    inject_trait_default_methods(&mut type_defs, &mut stores, &trait_defs);

    validate_trait_implementations(&type_defs, &stores, &trait_defs, &mut warnings)?;

    validate_typed_actor_sends(
        &functions,
        &globals,
        &actor_handler_names,
        &actor_message_types,
        &mut warnings,
    );

    let mut monomorphizations: HashMap<String, Vec<Vec<TypeId>>> = HashMap::new();
    for (_name, ty) in resolved.iter_all() {
        collect_monomorphizations(&ty, &mut monomorphizations);
    }

    let mut unboxed_number_lists = HashSet::new();
    let mut typed_lists: HashMap<(String, String), TypeId> = HashMap::new();
    for (name, ty) in resolved.iter_all() {
        if let TypeId::List(elem) = &ty {
            if matches!(
                elem.as_ref(),
                TypeId::Primitive(crate::types::core::Primitive::Int)
                    | TypeId::Primitive(crate::types::core::Primitive::Float)
            ) {
                unboxed_number_lists.insert(name.to_string());
            }
        }
    }

    // Collect typed list info per function
    for func in &functions {
        for (name, ty) in resolved.iter_all() {
            if let TypeId::List(elem) = &ty {
                match elem.as_ref() {
                    TypeId::Primitive(crate::types::core::Primitive::Int)
                    | TypeId::Primitive(crate::types::core::Primitive::Float) => {
                        typed_lists.insert(
                            (func.name.clone(), name.clone()),
                            *elem.clone(),
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    // Also detect typed lists from resolved_locals
    let (resolved_locals, resolved_params, resolved_returns) =
        collect_resolved_variable_types(&functions, &resolved);
    for ((fn_name, var_name), ty) in &resolved_locals {
        if let TypeId::List(elem) = ty {
            match elem.as_ref() {
                TypeId::Primitive(crate::types::core::Primitive::Int)
                | TypeId::Primitive(crate::types::core::Primitive::Float) => {
                    typed_lists.insert(
                        (fn_name.clone(), var_name.clone()),
                        *elem.clone(),
                    );
                }
                _ => {}
            }
        }
    }

    let mut store_field_indices = HashMap::new();
    let mut specialized_stores = HashSet::new();
    for (store_name, fields) in &store_field_names {
        specialized_stores.insert(store_name.clone());
        for (idx, field_name) in fields.iter().enumerate() {
            store_field_indices.insert((store_name.clone(), field_name.clone()), idx as u32);
        }
    }

    let escape_info = analyze_escape_info(&functions);

    let monomorph_info = collect_monomorph_info(&functions, &globals, &resolved);

    Ok(SemanticModel {
        globals,
        functions,
        extern_functions,
        stores,
        type_defs,
        trait_defs,
        error_defs,
        constraints,
        types: resolved,
        mutability,
        allocation,
        usage,
        warnings,
        field_types,
        store_field_names,
        module_exports,
        actor_message_types,
        actor_handler_names,
        trait_registry,
        monomorphizations,
        unboxed_number_lists,
        typed_lists,
        store_field_indices,
        specialized_stores,
        resolved_locals,
        resolved_params,
        resolved_returns,
        escape_info,
        monomorph_info,
    })
}

/// Analyze which local bindings escape each function.
/// A local escapes if it is: returned, captured by a closure, passed to a function call,
/// stored in a container field assignment, or used in a spread expression.
fn analyze_escape_info(functions: &[Function]) -> EscapeInfo {
    let mut info = EscapeInfo::default();
    for func in functions {
        // Collect all local binding names
        let locals = collect_local_bindings(&func.body);
        // Collect names that escape
        let mut escaping = HashSet::new();
        collect_escaping_names(&func.body, &mut escaping);
        for local in &locals {
            if !escaping.contains(local) {
                info.non_escaping
                    .insert((func.name.clone(), local.clone()));
                // Stack eligible: non-escaping locals that are bound to heap-creating expressions
                // (the codegen will check if the value is actually a heap type)
                info.stack_eligible
                    .insert((func.name.clone(), local.clone()));
            }
        }
    }
    info
}

/// Collect all local binding names in a block.
fn collect_local_bindings(block: &Block) -> Vec<String> {
    let mut names = Vec::new();
    for stmt in &block.statements {
        if let Statement::Binding(b) = stmt {
            names.push(b.name.clone());
        }
    }
    names
}

/// Walk a function body and collect all variable names that "escape":
/// - Returned from the function (in Return statements or block tail value)
/// - Captured by a lambda/closure
/// - Passed as an argument to a function call
/// - Stored in a field assignment
/// - Used in a spread expression
fn collect_escaping_names(block: &Block, escaping: &mut HashSet<String>) {
    for stmt in &block.statements {
        collect_escaping_from_stmt(stmt, escaping);
    }
    // The block's tail value is the return value
    if let Some(val) = &block.value {
        collect_identifiers_from_expr(val, escaping);
    }
}

fn collect_escaping_from_stmt(stmt: &Statement, escaping: &mut HashSet<String>) {
    match stmt {
        Statement::Return(expr, _) => {
            collect_identifiers_from_expr(expr, escaping);
        }
        Statement::Binding(b) => {
            // The binding value itself doesn't make the var escape,
            // but if the value is a call/lambda we need to check if the bound
            // variable is used in escaping contexts (handled by other stmt walks).
            // However, if the initializer passes other locals to function calls or
            // captures them in lambdas, those locals escape.
            collect_escaping_from_expr_rhs(&b.value, escaping);
        }
        Statement::Expression(expr) => {
            collect_escaping_from_expr_rhs(expr, escaping);
        }
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => {
            collect_escaping_from_expr_rhs(condition, escaping);
            collect_escaping_names(body, escaping);
            for (cond, blk) in elif_branches {
                collect_escaping_from_expr_rhs(cond, escaping);
                collect_escaping_names(blk, escaping);
            }
            if let Some(eb) = else_body {
                collect_escaping_names(eb, escaping);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_escaping_from_expr_rhs(condition, escaping);
            collect_escaping_names(body, escaping);
        }
        Statement::For {
            iterable, body, ..
        } => {
            collect_escaping_from_expr_rhs(iterable, escaping);
            collect_escaping_names(body, escaping);
        }
        Statement::ForKV {
            iterable, body, ..
        } => {
            collect_escaping_from_expr_rhs(iterable, escaping);
            collect_escaping_names(body, escaping);
        }
        Statement::ForRange {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_escaping_from_expr_rhs(start, escaping);
            collect_escaping_from_expr_rhs(end, escaping);
            if let Some(s) = step {
                collect_escaping_from_expr_rhs(s, escaping);
            }
            collect_escaping_names(body, escaping);
        }
        Statement::FieldAssign { target, value, .. } => {
            // Both target and value identifiers escape (stored into a container)
            collect_identifiers_from_expr(target, escaping);
            collect_identifiers_from_expr(value, escaping);
        }
        Statement::PatternBinding { value, .. } => {
            collect_escaping_from_expr_rhs(value, escaping);
        }
        Statement::Break(_) | Statement::Continue(_) => {}
    }
}

/// From an expression used as an rvalue, collect names that escape through:
/// - Being passed as a call argument
/// - Being captured by a lambda
/// - Being used in a spread
fn collect_escaping_from_expr_rhs(expr: &Expression, escaping: &mut HashSet<String>) {
    match expr {
        Expression::Call { args, callee, .. } => {
            // All arguments to function calls are considered escaping
            for arg in args {
                collect_identifiers_from_expr(arg, escaping);
            }
            collect_escaping_from_expr_rhs(callee, escaping);
        }
        Expression::Lambda { body, .. } => {
            // All identifiers referenced in the lambda body are captures and escape
            collect_all_identifiers_in_block(body, escaping);
        }
        Expression::Pipeline { left, right, .. } => {
            // Left is passed to right (a function call)
            collect_identifiers_from_expr(left, escaping);
            collect_escaping_from_expr_rhs(right, escaping);
        }
        Expression::Spread(inner, _) => {
            collect_identifiers_from_expr(inner, escaping);
        }
        // List/Map literals: elements don't escape unless the container itself escapes
        // (which is handled at the statement level)
        Expression::List(elems, _) => {
            for e in elems {
                collect_escaping_from_expr_rhs(e, escaping);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                collect_escaping_from_expr_rhs(k, escaping);
                collect_escaping_from_expr_rhs(v, escaping);
            }
        }
        Expression::Binary { left, right, .. } => {
            collect_escaping_from_expr_rhs(left, escaping);
            collect_escaping_from_expr_rhs(right, escaping);
        }
        Expression::Unary { expr, .. } => {
            collect_escaping_from_expr_rhs(expr, escaping);
        }
        Expression::Member { target, .. } => {
            collect_escaping_from_expr_rhs(target, escaping);
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_escaping_from_expr_rhs(condition, escaping);
            collect_escaping_from_expr_rhs(then_branch, escaping);
            collect_escaping_from_expr_rhs(else_branch, escaping);
        }
        Expression::Match(m) => {
            collect_escaping_from_expr_rhs(&m.value, escaping);
            for arm in &m.arms {
                collect_escaping_names(&arm.body, escaping);
                if let Some(g) = &arm.guard {
                    collect_escaping_from_expr_rhs(g, escaping);
                }
            }
            if let Some(default_block) = &m.default {
                collect_escaping_names(default_block, escaping);
            }
        }
        Expression::ListComprehension {
            body,
            iterable,
            condition,
            ..
        } => {
            collect_escaping_from_expr_rhs(body, escaping);
            collect_escaping_from_expr_rhs(iterable, escaping);
            if let Some(c) = condition {
                collect_escaping_from_expr_rhs(c, escaping);
            }
        }
        Expression::MapComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => {
            collect_escaping_from_expr_rhs(key, escaping);
            collect_escaping_from_expr_rhs(value, escaping);
            collect_escaping_from_expr_rhs(iterable, escaping);
            if let Some(c) = condition {
                collect_escaping_from_expr_rhs(c, escaping);
            }
        }
        Expression::Throw { value, .. } => {
            collect_identifiers_from_expr(value, escaping);
        }
        Expression::ErrorPropagate { expr, .. } => {
            collect_escaping_from_expr_rhs(expr, escaping);
        }
        // Leaf expressions: no sub-expressions to recurse into
        _ => {}
    }
}

/// Collect all direct Identifier names from an expression (shallow — one level).
fn collect_identifiers_from_expr(expr: &Expression, names: &mut HashSet<String>) {
    match expr {
        Expression::Identifier(name, _) => {
            names.insert(name.clone());
        }
        Expression::Binary { left, right, .. } => {
            collect_identifiers_from_expr(left, names);
            collect_identifiers_from_expr(right, names);
        }
        Expression::Unary { expr, .. } => {
            collect_identifiers_from_expr(expr, names);
        }
        Expression::Member { target, .. } => {
            collect_identifiers_from_expr(target, names);
        }
        Expression::Call { callee, args, .. } => {
            collect_identifiers_from_expr(callee, names);
            for a in args {
                collect_identifiers_from_expr(a, names);
            }
        }
        Expression::List(elems, _) => {
            for e in elems {
                collect_identifiers_from_expr(e, names);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                collect_identifiers_from_expr(k, names);
                collect_identifiers_from_expr(v, names);
            }
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_identifiers_from_expr(condition, names);
            collect_identifiers_from_expr(then_branch, names);
            collect_identifiers_from_expr(else_branch, names);
        }
        Expression::Pipeline { left, right, .. } => {
            collect_identifiers_from_expr(left, names);
            collect_identifiers_from_expr(right, names);
        }
        Expression::Spread(inner, _) => {
            collect_identifiers_from_expr(inner, names);
        }
        _ => {}
    }
}

/// Collect all identifiers in a block recursively (for lambda capture analysis).
fn collect_all_identifiers_in_block(block: &Block, names: &mut HashSet<String>) {
    for stmt in &block.statements {
        collect_all_identifiers_in_stmt(stmt, names);
    }
    if let Some(val) = &block.value {
        collect_identifiers_from_expr(val, names);
    }
}

fn collect_all_identifiers_in_stmt(stmt: &Statement, names: &mut HashSet<String>) {
    match stmt {
        Statement::Binding(b) => {
            collect_identifiers_from_expr(&b.value, names);
        }
        Statement::Expression(expr) | Statement::Return(expr, _) => {
            collect_identifiers_from_expr(expr, names);
        }
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => {
            collect_identifiers_from_expr(condition, names);
            collect_all_identifiers_in_block(body, names);
            for (cond, blk) in elif_branches {
                collect_identifiers_from_expr(cond, names);
                collect_all_identifiers_in_block(blk, names);
            }
            if let Some(eb) = else_body {
                collect_all_identifiers_in_block(eb, names);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_identifiers_from_expr(condition, names);
            collect_all_identifiers_in_block(body, names);
        }
        Statement::For {
            iterable, body, ..
        }
        | Statement::ForKV {
            iterable, body, ..
        } => {
            collect_identifiers_from_expr(iterable, names);
            collect_all_identifiers_in_block(body, names);
        }
        Statement::ForRange {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_identifiers_from_expr(start, names);
            collect_identifiers_from_expr(end, names);
            if let Some(s) = step {
                collect_identifiers_from_expr(s, names);
            }
            collect_all_identifiers_in_block(body, names);
        }
        Statement::FieldAssign {
            target, value, ..
        } => {
            collect_identifiers_from_expr(target, names);
            collect_identifiers_from_expr(value, names);
        }
        Statement::PatternBinding { value, .. } => {
            collect_identifiers_from_expr(value, names);
        }
        Statement::Break(_) | Statement::Continue(_) => {}
    }
}

fn collect_monomorphizations(ty: &TypeId, table: &mut HashMap<String, Vec<Vec<TypeId>>>) {
    match ty {
        TypeId::Adt(name, args) if !args.is_empty() && args.iter().all(|a| a.is_concrete()) => {
            let entry = table.entry(name.clone()).or_default();
            if !entry.iter().any(|existing| existing == args) {
                entry.push(args.clone());
            }

            for arg in args {
                collect_monomorphizations(arg, table);
            }
        }
        TypeId::List(elem) => collect_monomorphizations(elem, table),
        TypeId::Map(k, v) => {
            collect_monomorphizations(k, table);
            collect_monomorphizations(v, table);
        }
        TypeId::Func(params, ret) => {
            for p in params {
                collect_monomorphizations(p, table);
            }
            collect_monomorphizations(ret, table);
        }
        TypeId::Adt(_, args) => {
            for arg in args {
                collect_monomorphizations(arg, table);
            }
        }
        _ => {}
    }
}

fn inject_trait_default_methods(
    type_defs: &mut [crate::ast::TypeDefinition],
    stores: &mut [crate::ast::StoreDefinition],
    trait_defs: &[TraitDefinition],
) {
    let trait_map: HashMap<&str, &TraitDefinition> =
        trait_defs.iter().map(|t| (t.name.as_str(), t)).collect();

    for type_def in type_defs.iter_mut() {
        for trait_name in &type_def.with_traits {
            if let Some(trait_def) = trait_map.get(trait_name.as_str()) {
                for method in &trait_def.methods {
                    if let Some(ref body) = method.body {
                        let already_has = type_def.methods.iter().any(|m| m.name == method.name);
                        if !already_has {
                            type_def.methods.push(crate::ast::Function {
                                name: method.name.clone(),
                                params: method.params.clone(),
                                body: body.clone(),
                                kind: crate::ast::FunctionKind::Method,
                                span: method.span,
                            });
                        }
                    }
                }
            }
        }
    }

    for store in stores.iter_mut() {
        for trait_name in &store.with_traits {
            if let Some(trait_def) = trait_map.get(trait_name.as_str()) {
                for method in &trait_def.methods {
                    if let Some(ref body) = method.body {
                        let already_has = store.methods.iter().any(|m| m.name == method.name);
                        if !already_has {
                            store.methods.push(crate::ast::Function {
                                name: method.name.clone(),
                                params: method.params.clone(),
                                body: body.clone(),
                                kind: crate::ast::FunctionKind::Method,
                                span: method.span,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn validate_trait_implementations(
    type_defs: &[crate::ast::TypeDefinition],
    stores: &[crate::ast::StoreDefinition],
    trait_defs: &[TraitDefinition],
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    let trait_map: HashMap<&str, &TraitDefinition> =
        trait_defs.iter().map(|t| (t.name.as_str(), t)).collect();

    for type_def in type_defs {
        validate_type_traits(type_def, &trait_map, warnings)?;
    }

    for store in stores {
        validate_store_traits(store, &trait_map, warnings)?;
    }

    for trait_def in trait_defs {
        for required in &trait_def.required_traits {
            if !trait_map.contains_key(required.as_str()) {
                return Err(Diagnostic::new(
                    format!(
                        "trait `{}` requires unknown trait `{}`",
                        trait_def.name, required
                    ),
                    trait_def.span,
                ));
            }
        }
    }

    Ok(())
}

fn validate_type_traits(
    type_def: &crate::ast::TypeDefinition,
    trait_map: &HashMap<&str, &TraitDefinition>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    for trait_name in &type_def.with_traits {
        let Some(trait_def) = trait_map.get(trait_name.as_str()) else {
            return Err(Diagnostic::new(
                format!(
                    "type `{}` implements unknown trait `{}`",
                    type_def.name, trait_name
                ),
                type_def.span,
            ));
        };

        for required in &trait_def.required_traits {
            if !type_def.with_traits.contains(required) {
                return Err(Diagnostic::new(
                    format!(
                        "type `{}` implements `{}` which requires `{}`, but `{}` is not implemented",
                        type_def.name, trait_name, required, required
                    ),
                    type_def.span,
                ));
            }
        }

        let type_method_names: HashSet<&str> =
            type_def.methods.iter().map(|m| m.name.as_str()).collect();

        for method in &trait_def.methods {
            if method.body.is_none() && !type_method_names.contains(method.name.as_str()) {
                return Err(Diagnostic::new(
                    format!(
                        "type `{}` does not implement required method `{}` from trait `{}`",
                        type_def.name, method.name, trait_name
                    ),
                    type_def.span,
                ));
            }

            if method.body.is_some() && type_method_names.contains(method.name.as_str()) {
                warnings.push(Diagnostic::new(
                    format!(
                        "type `{}` overrides default implementation of `{}` from trait `{}`",
                        type_def.name, method.name, trait_name
                    ),
                    type_def.span,
                ));
            }
        }
    }

    Ok(())
}

fn validate_store_traits(
    store: &crate::ast::StoreDefinition,
    trait_map: &HashMap<&str, &TraitDefinition>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    let kind = if store.is_actor { "actor" } else { "store" };

    for trait_name in &store.with_traits {
        let Some(trait_def) = trait_map.get(trait_name.as_str()) else {
            return Err(Diagnostic::new(
                format!(
                    "{} `{}` implements unknown trait `{}`",
                    kind, store.name, trait_name
                ),
                store.span,
            ));
        };

        for required in &trait_def.required_traits {
            if !store.with_traits.contains(required) {
                return Err(Diagnostic::new(
                    format!(
                        "{} `{}` implements `{}` which requires `{}`, but `{}` is not implemented",
                        kind, store.name, trait_name, required, required
                    ),
                    store.span,
                ));
            }
        }

        let store_method_names: HashSet<&str> =
            store.methods.iter().map(|m| m.name.as_str()).collect();

        for method in &trait_def.methods {
            if method.body.is_none() && !store_method_names.contains(method.name.as_str()) {
                return Err(Diagnostic::new(
                    format!(
                        "{} `{}` does not implement required method `{}` from trait `{}`",
                        kind, store.name, method.name, trait_name
                    ),
                    store.span,
                ));
            }

            if method.body.is_some() && store_method_names.contains(method.name.as_str()) {
                warnings.push(Diagnostic::new(
                    format!(
                        "{} `{}` overrides default implementation of `{}` from trait `{}`",
                        kind, store.name, method.name, trait_name
                    ),
                    store.span,
                ));
            }
        }
    }

    Ok(())
}

fn collect_program_constraints(
    globals: &[Binding],
    functions: &[Function],
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
    branch_type_hints: &mut Vec<(Vec<TypeId>, Span)>,
) {
    for binding in globals {
        let ty = collect_constraints_expr(&binding.value, constraints, types, graph);
        if let Some(name) = Some(binding.name.clone()) {
            types.insert(name, ty);
        }
    }
    for function in functions {
        collect_function_constraints(function, constraints, types, graph, branch_type_hints);
    }
}

fn collect_function_constraints(
    function: &Function,
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
    branch_type_hints: &mut Vec<(Vec<TypeId>, Span)>,
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

    let return_ty = TypeId::TypeVar(graph.fresh());
    let body_ty = collect_block_constraints(
        &function.body,
        constraints,
        types,
        graph,
        Some(&return_ty),
        branch_type_hints,
    );

    let fn_return = if function.body.value.is_some() {
        // If there are also return statements, unify body value type with return type
        if has_return_statements(&function.body) {
            constraints.push(ConstraintKind::Equal(body_ty.clone(), return_ty));
        }
        body_ty
    } else if has_return_statements(&function.body) {
        return_ty
    } else {
        body_ty
    };
    let fn_ty = TypeId::Func(params_tys, Box::new(fn_return));
    types.insert(function.name.clone(), fn_ty);
}

fn has_return_statements(block: &Block) -> bool {
    for stmt in &block.statements {
        match stmt {
            crate::ast::Statement::Return(_, _) => return true,
            crate::ast::Statement::If {
                body,
                elif_branches,
                else_body,
                ..
            } => {
                if has_return_statements(body) {
                    return true;
                }
                for (_, blk) in elif_branches {
                    if has_return_statements(blk) {
                        return true;
                    }
                }
                if let Some(eb) = else_body {
                    if has_return_statements(eb) {
                        return true;
                    }
                }
            }
            crate::ast::Statement::While { body, .. } => {
                if has_return_statements(body) {
                    return true;
                }
            }
            crate::ast::Statement::For { body, .. } => {
                if has_return_statements(body) {
                    return true;
                }
            }
            crate::ast::Statement::ForRange { body, .. } => {
                if has_return_statements(body) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn collect_block_constraints(
    block: &Block,
    constraints: &mut ConstraintSet,
    types: &mut TypeEnv,
    graph: &mut TypeGraph,
    return_ty: Option<&TypeId>,
    branch_type_hints: &mut Vec<(Vec<TypeId>, Span)>,
) -> TypeId {
    for statement in &block.statements {
        match statement {
            crate::ast::Statement::Binding(binding) => {
                let rhs_ty = collect_constraints_expr(&binding.value, constraints, types, graph);
                if let Some(ann) = &binding.type_annotation {
                    let ann_ty = type_from_annotation(ann);
                    constraints.push(ConstraintKind::EqualAt(
                        rhs_ty.clone(),
                        ann_ty.clone(),
                        binding.span,
                    ));
                    types.insert(binding.name.clone(), ann_ty);
                } else {
                    types.insert(binding.name.clone(), rhs_ty.clone());
                }
            }
            crate::ast::Statement::Expression(expr) => {
                let _ = collect_constraints_expr(expr, constraints, types, graph);
            }
            crate::ast::Statement::Return(expr, span) => {
                let ret_expr_ty = collect_constraints_expr(expr, constraints, types, graph);

                if let Some(ret_ty) = return_ty {
                    constraints.push(ConstraintKind::EqualAt(ret_expr_ty, ret_ty.clone(), *span));
                }
            }
            crate::ast::Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                span,
            } => {
                let _ = collect_constraints_expr(condition, constraints, types, graph);
                let body_ty = collect_block_constraints(
                    body,
                    constraints,
                    types,
                    graph,
                    return_ty,
                    branch_type_hints,
                );

                if else_body.is_some() {
                    let mut branch_tys = vec![body_ty];
                    for (cond, blk) in elif_branches {
                        let _ = collect_constraints_expr(cond, constraints, types, graph);
                        let blk_ty = collect_block_constraints(
                            blk,
                            constraints,
                            types,
                            graph,
                            return_ty,
                            branch_type_hints,
                        );
                        branch_tys.push(blk_ty);
                    }
                    if let Some(else_blk) = else_body {
                        let else_ty = collect_block_constraints(
                            else_blk,
                            constraints,
                            types,
                            graph,
                            return_ty,
                            branch_type_hints,
                        );
                        branch_tys.push(else_ty);
                    }
                    branch_type_hints.push((branch_tys, *span));
                } else {
                    for (cond, blk) in elif_branches {
                        let _ = collect_constraints_expr(cond, constraints, types, graph);
                        let _ = collect_block_constraints(
                            blk,
                            constraints,
                            types,
                            graph,
                            return_ty,
                            branch_type_hints,
                        );
                    }
                }
            }
            crate::ast::Statement::While {
                condition, body, ..
            } => {
                let _ = collect_constraints_expr(condition, constraints, types, graph);
                let _ = collect_block_constraints(
                    body,
                    constraints,
                    types,
                    graph,
                    return_ty,
                    branch_type_hints,
                );
            }
            crate::ast::Statement::For {
                variable,
                iterable,
                body,
                span,
            } => {
                let iterable_ty = collect_constraints_expr(iterable, constraints, types, graph);
                let elem_ty = TypeId::TypeVar(graph.fresh());
                constraints.push(ConstraintKind::IterableAt(
                    iterable_ty,
                    elem_ty.clone(),
                    *span,
                ));
                types.insert(variable.clone(), elem_ty);
                let _ = collect_block_constraints(
                    body,
                    constraints,
                    types,
                    graph,
                    return_ty,
                    branch_type_hints,
                );
            }
            crate::ast::Statement::ForKV {
                key_var,
                value_var,
                iterable,
                body,
                span,
            } => {
                let iterable_ty = collect_constraints_expr(iterable, constraints, types, graph);
                let elem_ty = TypeId::TypeVar(graph.fresh());
                constraints.push(ConstraintKind::IterableAt(
                    iterable_ty,
                    elem_ty.clone(),
                    *span,
                ));
                types.insert(key_var.clone(), elem_ty.clone());
                types.insert(value_var.clone(), elem_ty);
                let _ = collect_block_constraints(
                    body,
                    constraints,
                    types,
                    graph,
                    return_ty,
                    branch_type_hints,
                );
            }
            crate::ast::Statement::ForRange {
                variable,
                start,
                end,
                step,
                body,
                ..
            } => {
                let _ = collect_constraints_expr(start, constraints, types, graph);
                let _ = collect_constraints_expr(end, constraints, types, graph);
                if let Some(s) = step {
                    let _ = collect_constraints_expr(s, constraints, types, graph);
                }
                types.insert(variable.clone(), TypeId::Primitive(Primitive::Float));
                let _ = collect_block_constraints(
                    body,
                    constraints,
                    types,
                    graph,
                    return_ty,
                    branch_type_hints,
                );
            }
            crate::ast::Statement::Break(_) | crate::ast::Statement::Continue(_) => {}
            crate::ast::Statement::FieldAssign { target, value, .. } => {
                let _ = collect_constraints_expr(target, constraints, types, graph);
                let _ = collect_constraints_expr(value, constraints, types, graph);
            }
            crate::ast::Statement::PatternBinding { pattern, value, .. } => {
                let rhs_ty = collect_constraints_expr(value, constraints, types, graph);
                collect_pattern_bindings(pattern, &rhs_ty, types);
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
        Expression::None(_) => TypeId::Primitive(Primitive::None),
        Expression::InlineAsm { .. } => TypeId::Unknown,
        Expression::PtrLoad { .. } => TypeId::Unknown,
        Expression::Unsafe { .. } => TypeId::Unknown,
        Expression::Spread(inner, _) => {
            let inner_ty = collect_constraints_expr(inner, constraints, types, graph);
            inner_ty
        }
        Expression::Identifier(name, span) => {
            if let Some((enum_name, type_params, field_count)) =
                types.get_generic_constructor(name).cloned()
            {
                let fresh_args: Vec<TypeId> = type_params
                    .iter()
                    .map(|_| TypeId::TypeVar(graph.fresh()))
                    .collect();

                for (param_name, fresh_ty) in type_params.iter().zip(fresh_args.iter()) {
                    if let Some(bounds) = types.get_type_param_bounds(&enum_name, param_name) {
                        for bound in bounds.clone() {
                            constraints.push(ConstraintKind::HasTrait(
                                fresh_ty.clone(),
                                bound,
                                *span,
                            ));
                        }
                    }
                }
                let adt_ty = TypeId::Adt(enum_name.clone(), fresh_args.clone());
                if field_count == 0 {
                    adt_ty
                } else {
                    let param_types: Vec<TypeId> = (0..field_count)
                        .map(|_| TypeId::TypeVar(graph.fresh()))
                        .collect();
                    TypeId::Func(param_types, Box::new(adt_ty))
                }
            } else {
                types.get(name).cloned().unwrap_or(TypeId::Unknown)
            }
        }
        Expression::Placeholder(id, _) => TypeId::Placeholder(*id),
        Expression::TaxonomyPath { .. } => TypeId::Primitive(Primitive::String),
        Expression::List(items, span) => {
            let elem_ty = TypeId::TypeVar(graph.fresh());
            for item in items {
                let ty = collect_constraints_expr(item, constraints, types, graph);
                if matches!(item, Expression::Spread(..)) {
                    constraints.push(ConstraintKind::EqualAt(
                        TypeId::List(Box::new(elem_ty.clone())),
                        ty,
                        *span,
                    ));
                } else {
                    constraints.push(ConstraintKind::EqualAt(elem_ty.clone(), ty, *span));
                }
            }
            TypeId::List(Box::new(elem_ty))
        }
        Expression::Map(entries, span) => {
            let key_ty = TypeId::TypeVar(graph.fresh());
            for (k, v) in entries {
                let kt = collect_constraints_expr(k, constraints, types, graph);
                let _vt = collect_constraints_expr(v, constraints, types, graph);
                constraints.push(ConstraintKind::EqualAt(key_ty.clone(), kt, *span));
            }

            TypeId::Map(
                Box::new(key_ty),
                Box::new(TypeId::Primitive(Primitive::Any)),
            )
        }
        Expression::Binary {
            op,
            left,
            right,
            span,
        } => {
            let l = collect_constraints_expr(left, constraints, types, graph);
            let r = collect_constraints_expr(right, constraints, types, graph);
            match op {
                crate::ast::BinaryOp::Add => match (&l, &r) {
                    (TypeId::Primitive(Primitive::String), _)
                    | (_, TypeId::Primitive(Primitive::String)) => {
                        TypeId::Primitive(Primitive::String)
                    }
                    _ => {
                        constraints.push(ConstraintKind::EqualAt(l.clone(), r.clone(), *span));
                        l
                    }
                },
                crate::ast::BinaryOp::Sub
                | crate::ast::BinaryOp::Mul
                | crate::ast::BinaryOp::Div
                | crate::ast::BinaryOp::Mod
                | crate::ast::BinaryOp::BitAnd
                | crate::ast::BinaryOp::BitOr
                | crate::ast::BinaryOp::BitXor
                | crate::ast::BinaryOp::ShiftLeft
                | crate::ast::BinaryOp::ShiftRight => {
                    constraints.push(ConstraintKind::NumericAt(l.clone(), *span));
                    constraints.push(ConstraintKind::NumericAt(r.clone(), *span));
                    constraints.push(ConstraintKind::EqualAt(l.clone(), r.clone(), *span));
                    l
                }
                crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or => {
                    constraints.push(ConstraintKind::BooleanAt(l.clone(), *span));
                    constraints.push(ConstraintKind::BooleanAt(r.clone(), *span));
                    TypeId::Primitive(Primitive::Bool)
                }
                crate::ast::BinaryOp::Equals | crate::ast::BinaryOp::NotEquals => {
                    constraints.push(ConstraintKind::EqualAt(l.clone(), r.clone(), *span));
                    TypeId::Primitive(Primitive::Bool)
                }
                crate::ast::BinaryOp::Greater
                | crate::ast::BinaryOp::GreaterEq
                | crate::ast::BinaryOp::Less
                | crate::ast::BinaryOp::LessEq => {
                    constraints.push(ConstraintKind::NumericAt(l.clone(), *span));
                    constraints.push(ConstraintKind::NumericAt(r.clone(), *span));
                    TypeId::Primitive(Primitive::Bool)
                }
            }
        }
        Expression::Unary { op, expr, span } => {
            let inner = collect_constraints_expr(expr, constraints, types, graph);
            match op {
                crate::ast::UnaryOp::Neg => {
                    constraints.push(ConstraintKind::NumericAt(inner.clone(), *span));
                    inner
                }
                crate::ast::UnaryOp::Not => {
                    constraints.push(ConstraintKind::BooleanAt(inner.clone(), *span));
                    TypeId::Primitive(Primitive::Bool)
                }
                crate::ast::UnaryOp::BitNot => {
                    constraints.push(ConstraintKind::NumericAt(inner.clone(), *span));
                    inner
                }
            }
        }
        Expression::Call {
            callee, args, span, ..
        } => {
            if let Expression::Member {
                target, property, ..
            } = callee.as_ref()
            {
                let target_ty = collect_constraints_expr(target, constraints, types, graph);

                for arg in args {
                    collect_constraints_expr(arg, constraints, types, graph);
                }
                match property.as_str() {
                    "length" | "count" | "size" | "index_of" => {
                        return TypeId::Primitive(Primitive::Int);
                    }

                    "err" | "equals" | "not_equals" | "contains" | "any" | "all"
                    | "starts_with" | "ends_with" | "is_empty" => {
                        return TypeId::Primitive(Primitive::Bool);
                    }

                    "trim" | "lower" | "upper" | "strip" | "lstrip" | "rstrip" | "replace"
                    | "pad_left" | "pad_right" | "reverse" | "repeat" | "to_string" | "join"
                    | "slice" | "substr" | "char_at" | "concat" => {
                        return TypeId::Primitive(Primitive::String);
                    }

                    "split" | "map" | "filter" | "sort" | "keys" | "values" | "find_all"
                    | "chars" | "lines" | "bytes" => {
                        return TypeId::List(Box::new(TypeId::Unknown));
                    }

                    "push" | "pop" | "append" | "remove" | "insert" | "clear" => {
                        return target_ty;
                    }

                    "get" | "set" | "at" | "reduce" | "find" | "not" | "iter" | "or"
                    | "unwrap_or" | "first" | "last" => {
                        return TypeId::Unknown;
                    }
                    _ => {}
                }
            }
            let callee_ty = collect_constraints_expr(callee, constraints, types, graph);
            let mut arg_tys = Vec::new();
            for arg in args {
                arg_tys.push(collect_constraints_expr(arg, constraints, types, graph));
            }
            let result_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::CallableAt(
                callee_ty.clone(),
                arg_tys.clone(),
                result_ty.clone(),
                *span,
            ));
            result_ty
        }
        Expression::Index {
            target,
            index,
            span: _,
        } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            let _index_ty = collect_constraints_expr(index, constraints, types, graph);

            match &target_ty {
                TypeId::List(elem) => *elem.clone(),
                TypeId::Map(_, val) => *val.clone(),
                _ => TypeId::Unknown,
            }
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            let _start_ty = collect_constraints_expr(start, constraints, types, graph);
            let _end_ty = collect_constraints_expr(end, constraints, types, graph);

            target_ty
        }
        Expression::Member {
            target,
            property,
            span,
        } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            match property.as_str() {
                "length" | "count" | "size" => TypeId::Primitive(Primitive::Int),

                "err" => TypeId::Primitive(Primitive::Bool),

                "push" | "pop" | "get" | "set" | "append" | "remove" | "insert" | "contains"
                | "keys" | "values" | "clear" | "join" | "map" | "filter" | "reduce" | "find"
                | "any" | "all" | "sort" | "equals" | "not_equals" | "not" | "iter" | "split"
                | "trim" | "lower" | "upper" | "strip" | "lstrip" | "rstrip" | "replace"
                | "pad_left" | "pad_right" | "reverse" | "repeat" | "starts_with" | "ends_with"
                | "index_of" | "is_empty" | "to_string" | "concat" | "chars" | "lines"
                | "bytes" | "slice" | "substr" | "char_at" | "find_all" | "or" | "unwrap_or"
                | "first" | "last" | "at" => TypeId::Unknown,

                _ => match &target_ty {
                    TypeId::Primitive(Primitive::Any) | TypeId::Unknown | TypeId::TypeVar(_) => {
                        TypeId::Unknown
                    }
                    _ => {
                        let val_ty = TypeId::TypeVar(graph.fresh());
                        let map_ty = TypeId::Map(
                            Box::new(TypeId::Primitive(Primitive::String)),
                            Box::new(val_ty.clone()),
                        );
                        constraints.push(ConstraintKind::EqualAt(target_ty, map_ty, *span));
                        val_ty
                    }
                },
            }
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let cond_ty = collect_constraints_expr(condition, constraints, types, graph);
            let then_ty = collect_constraints_expr(then_branch, constraints, types, graph);
            let else_ty = collect_constraints_expr(else_branch, constraints, types, graph);
            constraints.push(ConstraintKind::BooleanAt(cond_ty, *span));
            constraints.push(ConstraintKind::EqualAt(
                then_ty.clone(),
                else_ty.clone(),
                *span,
            ));
            then_ty
        }
        Expression::Match(match_expr) => {
            let match_span = match_expr.span;
            let scrutinee_ty =
                collect_constraints_expr(&match_expr.value, constraints, types, graph);
            let mut arm_tys = Vec::new();
            for arm in &match_expr.arms {
                match &arm.pattern {
                    crate::ast::MatchPattern::Integer(_) => {
                        constraints
                            .push(ConstraintKind::NumericAt(scrutinee_ty.clone(), match_span));
                    }
                    crate::ast::MatchPattern::Bool(_) => {
                        constraints.push(ConstraintKind::EqualAt(
                            scrutinee_ty.clone(),
                            TypeId::Primitive(Primitive::Bool),
                            match_span,
                        ));
                    }
                    crate::ast::MatchPattern::String(_) => {
                        constraints.push(ConstraintKind::EqualAt(
                            scrutinee_ty.clone(),
                            TypeId::Primitive(Primitive::String),
                            match_span,
                        ));
                    }
                    crate::ast::MatchPattern::List(patterns) => {
                        let elem_ty = if patterns.is_empty() {
                            TypeId::TypeVar(graph.fresh())
                        } else {
                            TypeId::Primitive(Primitive::Any)
                        };
                        constraints.push(ConstraintKind::EqualAt(
                            scrutinee_ty.clone(),
                            TypeId::List(Box::new(elem_ty.clone())),
                            match_span,
                        ));

                        for pat in patterns {
                            collect_pattern_bindings(pat, &elem_ty, types);
                        }
                    }
                    crate::ast::MatchPattern::Identifier(name) => {
                        types.insert(name.clone(), scrutinee_ty.clone());
                    }
                    crate::ast::MatchPattern::Constructor {
                        name,
                        fields,
                        span: _,
                    } => {
                        if let Some((enum_name, type_params, _field_count)) =
                            types.get_generic_constructor(name).cloned()
                        {
                            let fresh_args: Vec<TypeId> = type_params
                                .iter()
                                .map(|_| TypeId::TypeVar(graph.fresh()))
                                .collect();

                            for (param_name, fresh_ty) in type_params.iter().zip(fresh_args.iter())
                            {
                                if let Some(bounds) =
                                    types.get_type_param_bounds(&enum_name, param_name)
                                {
                                    for bound in bounds.clone() {
                                        constraints.push(ConstraintKind::HasTrait(
                                            fresh_ty.clone(),
                                            bound,
                                            match_span,
                                        ));
                                    }
                                }
                            }
                            let adt_ty = TypeId::Adt(enum_name.clone(), fresh_args.clone());
                            constraints.push(ConstraintKind::EqualAt(
                                scrutinee_ty.clone(),
                                adt_ty,
                                match_span,
                            ));

                            for pat in fields {
                                let field_ty = TypeId::TypeVar(graph.fresh());
                                collect_pattern_bindings(pat, &field_ty, types);
                            }
                        } else {
                            let ctor_param_types: Option<Vec<TypeId>> =
                                types.get(name).and_then(|ctor_ty| match ctor_ty {
                                    TypeId::Func(param_types, _) => Some(param_types.clone()),
                                    _ => None,
                                });

                            if let Some(ctor_ty) = types.get(name) {
                                let adt_ty = match ctor_ty {
                                    TypeId::Adt(adt_name, args) => {
                                        TypeId::Adt(adt_name.clone(), args.clone())
                                    }
                                    TypeId::Func(_, ret) => (**ret).clone(),
                                    _ => TypeId::Primitive(Primitive::Any),
                                };
                                constraints.push(ConstraintKind::EqualAt(
                                    scrutinee_ty.clone(),
                                    adt_ty,
                                    match_span,
                                ));
                            }

                            if let Some(ref param_types) = ctor_param_types {
                                for (i, pat) in fields.iter().enumerate() {
                                    let field_ty = param_types
                                        .get(i)
                                        .cloned()
                                        .unwrap_or(TypeId::Primitive(Primitive::Any));
                                    collect_pattern_bindings(pat, &field_ty, types);
                                }
                            } else {
                                for pat in fields {
                                    collect_pattern_bindings(
                                        pat,
                                        &TypeId::Primitive(Primitive::Any),
                                        types,
                                    );
                                }
                            }
                        }
                    }
                    crate::ast::MatchPattern::Wildcard(_) => {}
                    crate::ast::MatchPattern::Range { .. } => {
                        constraints
                            .push(ConstraintKind::NumericAt(scrutinee_ty.clone(), match_span));
                    }
                    crate::ast::MatchPattern::RangeBinding { name, .. } => {
                        constraints
                            .push(ConstraintKind::NumericAt(scrutinee_ty.clone(), match_span));
                        types.insert(name.clone(), scrutinee_ty.clone());
                    }
                    crate::ast::MatchPattern::Rest(name, _) => {
                        types.insert(
                            name.clone(),
                            TypeId::List(Box::new(TypeId::Primitive(Primitive::Any))),
                        );
                    }
                    crate::ast::MatchPattern::Or(alternatives) => {
                        for alt in alternatives {
                            match alt {
                                crate::ast::MatchPattern::Integer(_) => {
                                    constraints.push(ConstraintKind::NumericAt(
                                        scrutinee_ty.clone(),
                                        match_span,
                                    ));
                                }
                                crate::ast::MatchPattern::Bool(_) => {
                                    constraints.push(ConstraintKind::EqualAt(
                                        scrutinee_ty.clone(),
                                        TypeId::Primitive(Primitive::Bool),
                                        match_span,
                                    ));
                                }
                                crate::ast::MatchPattern::String(_) => {
                                    constraints.push(ConstraintKind::EqualAt(
                                        scrutinee_ty.clone(),
                                        TypeId::Primitive(Primitive::String),
                                        match_span,
                                    ));
                                }
                                crate::ast::MatchPattern::Identifier(name) => {
                                    types.insert(name.clone(), scrutinee_ty.clone());
                                }
                                crate::ast::MatchPattern::Constructor { name, fields, .. } => {
                                    if let Some((enum_name, type_params, _field_count)) =
                                        types.get_generic_constructor(name).cloned()
                                    {
                                        let fresh_args: Vec<TypeId> = type_params
                                            .iter()
                                            .map(|_| TypeId::TypeVar(graph.fresh()))
                                            .collect();

                                        for (param_name, fresh_ty) in
                                            type_params.iter().zip(fresh_args.iter())
                                        {
                                            if let Some(bounds) =
                                                types.get_type_param_bounds(&enum_name, param_name)
                                            {
                                                for bound in bounds.clone() {
                                                    constraints.push(ConstraintKind::HasTrait(
                                                        fresh_ty.clone(),
                                                        bound,
                                                        match_span,
                                                    ));
                                                }
                                            }
                                        }
                                        let adt_ty =
                                            TypeId::Adt(enum_name.clone(), fresh_args.clone());
                                        constraints.push(ConstraintKind::EqualAt(
                                            scrutinee_ty.clone(),
                                            adt_ty,
                                            match_span,
                                        ));
                                    }

                                    let ctor_param_types: Option<Vec<TypeId>> =
                                        types.get(name).and_then(|ctor_ty| match ctor_ty {
                                            TypeId::Func(param_types, _) => {
                                                Some(param_types.clone())
                                            }
                                            _ => None,
                                        });
                                    if let Some(ref param_types) = ctor_param_types {
                                        for (i, pat) in fields.iter().enumerate() {
                                            let field_ty = param_types
                                                .get(i)
                                                .cloned()
                                                .unwrap_or(TypeId::Primitive(Primitive::Any));
                                            collect_pattern_bindings(pat, &field_ty, types);
                                        }
                                    } else {
                                        for pat in fields {
                                            collect_pattern_bindings(
                                                pat,
                                                &TypeId::Primitive(Primitive::Any),
                                                types,
                                            );
                                        }
                                    }
                                }
                                crate::ast::MatchPattern::Range { .. } => {
                                    constraints.push(ConstraintKind::NumericAt(
                                        scrutinee_ty.clone(),
                                        match_span,
                                    ));
                                }
                                crate::ast::MatchPattern::Rest(name, _) => {
                                    types.insert(
                                        name.clone(),
                                        TypeId::List(Box::new(TypeId::Primitive(Primitive::Any))),
                                    );
                                }
                                crate::ast::MatchPattern::List(patterns) => {
                                    let elem_ty = TypeId::Primitive(Primitive::Any);
                                    constraints.push(ConstraintKind::EqualAt(
                                        scrutinee_ty.clone(),
                                        TypeId::List(Box::new(elem_ty.clone())),
                                        match_span,
                                    ));
                                    for pat in patterns {
                                        collect_pattern_bindings(pat, &elem_ty, types);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }

                if let Some(guard) = &arm.guard {
                    collect_constraints_expr(guard, constraints, types, graph);
                }
                let arm_ty = collect_block_constraints(
                    &arm.body,
                    constraints,
                    types,
                    graph,
                    None,
                    &mut Vec::new(),
                );
                arm_tys.push(arm_ty);
            }
            if let Some(default) = &match_expr.default {
                arm_tys.push(collect_block_constraints(
                    default,
                    constraints,
                    types,
                    graph,
                    None,
                    &mut Vec::new(),
                ));
            }
            arm_tys
                .into_iter()
                .reduce(|a, b| {
                    constraints.push(ConstraintKind::EqualAt(a.clone(), b.clone(), match_span));
                    a
                })
                .unwrap_or(TypeId::Primitive(Primitive::Unit))
        }
        Expression::Throw { value, .. } => {
            collect_constraints_expr(value, constraints, types, graph)
        }
        Expression::Lambda { params, body, .. } => {
            let mut param_tys = Vec::new();
            let mut shadow = types.clone();
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
            let body_ty = collect_block_constraints(
                body,
                &mut nested_constraints,
                &mut shadow,
                &mut nested_graph,
                None,
                &mut Vec::new(),
            );
            constraints
                .constraints
                .extend(nested_constraints.constraints);
            TypeId::Func(param_tys, Box::new(body_ty))
        }
        Expression::Pipeline { left, right, span } => match right.as_ref() {
            Expression::Call {
                callee,
                args,
                span: call_span,
                ..
            } => {
                let mut full_args = vec![*left.clone()];
                full_args.extend(args.clone());
                let desugared = Expression::Call {
                    callee: callee.clone(),
                    args: full_args,
                    arg_names: vec![],
                    span: *call_span,
                };
                collect_constraints_expr(&desugared, constraints, types, graph)
            }
            Expression::Identifier(_name, _id_span) => {
                let desugared = Expression::Call {
                    callee: right.clone(),
                    args: vec![*left.clone()],
                    arg_names: vec![],
                    span: *span,
                };
                collect_constraints_expr(&desugared, constraints, types, graph)
            }
            _ => {
                let left_ty = collect_constraints_expr(left, constraints, types, graph);
                let right_ty = collect_constraints_expr(right, constraints, types, graph);
                let result_ty = TypeId::TypeVar(graph.fresh());
                constraints.push(ConstraintKind::CallableAt(
                    right_ty,
                    vec![left_ty],
                    result_ty.clone(),
                    *span,
                ));
                result_ty
            }
        },
        Expression::ErrorValue { path, .. } => TypeId::Error(path.clone()),
        Expression::ErrorPropagate { expr, .. } => {
            collect_constraints_expr(expr, constraints, types, graph)
        }
        Expression::ListComprehension {
            body,
            var,
            iterable,
            condition,
            span,
        } => {
            let iter_ty = collect_constraints_expr(iterable, constraints, types, graph);
            let elem_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::EqualAt(
                TypeId::List(Box::new(elem_ty.clone())),
                iter_ty,
                *span,
            ));
            types.push_scope();
            types.insert(var.clone(), elem_ty);
            if let Some(cond) = condition {
                collect_constraints_expr(cond, constraints, types, graph);
            }
            let body_ty = collect_constraints_expr(body, constraints, types, graph);
            types.pop_scope();
            TypeId::List(Box::new(body_ty))
        }
        Expression::MapComprehension {
            key,
            value,
            var,
            iterable,
            condition,
            span,
        } => {
            let iter_ty = collect_constraints_expr(iterable, constraints, types, graph);
            let elem_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::EqualAt(
                TypeId::List(Box::new(elem_ty.clone())),
                iter_ty,
                *span,
            ));
            types.push_scope();
            types.insert(var.clone(), elem_ty);
            if let Some(cond) = condition {
                collect_constraints_expr(cond, constraints, types, graph);
            }
            let key_ty = collect_constraints_expr(key, constraints, types, graph);
            let _val_ty = collect_constraints_expr(value, constraints, types, graph);
            types.pop_scope();
            TypeId::Map(
                Box::new(key_ty),
                Box::new(TypeId::Primitive(Primitive::Any)),
            )
        }
    }
}

fn collect_pattern_bindings(pattern: &crate::ast::MatchPattern, ty: &TypeId, types: &mut TypeEnv) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            types.insert(name.clone(), ty.clone());
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            for pat in fields {
                collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
            }
        }

        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                collect_pattern_bindings(alt, ty, types);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            for pat in patterns {
                collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            types.insert(
                name.clone(),
                TypeId::List(Box::new(TypeId::Primitive(Primitive::Any))),
            );
        }
        crate::ast::MatchPattern::RangeBinding { name, .. } => {
            types.insert(name.clone(), ty.clone());
        }
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
    }
}

fn type_from_annotation(ann: &crate::ast::TypeAnnotation) -> TypeId {
    if ann.segments.is_empty() {
        return TypeId::Unknown;
    }

    let base_type = match ann.segments[0].as_str() {
        "Int" => TypeId::Primitive(Primitive::Int),
        "Float" => TypeId::Primitive(Primitive::Float),
        "Bool" => TypeId::Primitive(Primitive::Bool),
        "String" => TypeId::Primitive(Primitive::String),
        "Bytes" => TypeId::Primitive(Primitive::Bytes),
        "Unit" => TypeId::Primitive(Primitive::Unit),
        "Any" => TypeId::Primitive(Primitive::Any),
        "Actor" => TypeId::Primitive(Primitive::Actor),

        "usize" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" | "i8" | "i16" | "i32" | "i64"
        | "isize" => TypeId::Primitive(Primitive::Any),

        "List" => {
            if ann.type_args.len() == 1 {
                let elem_type = type_from_annotation(&ann.type_args[0]);
                TypeId::List(Box::new(elem_type))
            } else {
                TypeId::List(Box::new(TypeId::Primitive(Primitive::Any)))
            }
        }
        "Map" => {
            if ann.type_args.len() == 2 {
                let key_type = type_from_annotation(&ann.type_args[0]);
                let value_type = type_from_annotation(&ann.type_args[1]);
                TypeId::Map(Box::new(key_type), Box::new(value_type))
            } else {
                TypeId::Map(
                    Box::new(TypeId::Primitive(Primitive::Any)),
                    Box::new(TypeId::Primitive(Primitive::Any)),
                )
            }
        }

        name => {
            if !ann.type_args.is_empty() {
                let type_args: Vec<TypeId> = ann
                    .type_args
                    .iter()
                    .map(|a| type_from_annotation(a))
                    .collect();
                TypeId::Adt(name.to_string(), type_args)
            } else {
                TypeId::Adt(name.to_string(), vec![])
            }
        }
    };

    base_type
}

fn check_function(function: &Function, known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    check_method_with_fields(function, &[], known_names)
}

fn check_method_with_fields(
    function: &Function,
    fields: &[crate::ast::Field],
    known_names: &HashSet<String>,
) -> Result<(), Diagnostic> {
    validate_parameter_defaults(&function.params)?;
    let mut scopes = ScopeStack::new();
    scopes.push();

    if !fields.is_empty() {
        scopes.declare("self".to_string(), function.span);
    }

    for field in fields {
        scopes.declare(field.name.clone(), field.span);
    }
    for param in &function.params {
        if let Some(previous) = scopes.lookup(&param.name) {
            return Err(duplicate_symbol(
                "parameter",
                &param.name,
                param.span,
                previous,
            ));
        }
        scopes.declare(param.name.clone(), param.span);
    }
    check_block(&function.body, &mut scopes, known_names)
}

fn check_block(
    block: &Block,
    scopes: &mut ScopeStack,
    known_names: &HashSet<String>,
) -> Result<(), Diagnostic> {
    scopes.push();
    for statement in &block.statements {
        match statement {
            Statement::Binding(binding) => {
                scopes.declare(binding.name.clone(), binding.span);
                check_expression(&binding.value, scopes, known_names)?;
            }
            Statement::Expression(expr) => check_expression(expr, scopes, known_names)?,
            Statement::Return(expr, _) => check_expression(expr, scopes, known_names)?,
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
                check_expression(condition, scopes, known_names)?;
                check_block(body, scopes, known_names)?;
                for (elif_cond, elif_body) in elif_branches {
                    check_expression(elif_cond, scopes, known_names)?;
                    check_block(elif_body, scopes, known_names)?;
                }
                if let Some(else_body) = else_body {
                    check_block(else_body, scopes, known_names)?;
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                check_expression(condition, scopes, known_names)?;
                check_block(body, scopes, known_names)?;
            }
            Statement::For {
                variable,
                iterable,
                body,
                span,
            } => {
                check_expression(iterable, scopes, known_names)?;
                scopes.push();
                scopes.declare(variable.clone(), *span);
                check_block(body, scopes, known_names)?;
                scopes.pop();
            }
            Statement::ForKV {
                key_var,
                value_var,
                iterable,
                body,
                span,
            } => {
                check_expression(iterable, scopes, known_names)?;
                scopes.push();
                scopes.declare(key_var.clone(), *span);
                scopes.declare(value_var.clone(), *span);
                check_block(body, scopes, known_names)?;
                scopes.pop();
            }
            Statement::ForRange {
                variable,
                start,
                end,
                step,
                body,
                span,
            } => {
                check_expression(start, scopes, known_names)?;
                check_expression(end, scopes, known_names)?;
                if let Some(s) = step {
                    check_expression(s, scopes, known_names)?;
                }
                scopes.push();
                scopes.declare(variable.clone(), *span);
                check_block(body, scopes, known_names)?;
                scopes.pop();
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { target, value, .. } => {
                check_expression(target, scopes, known_names)?;
                check_expression(value, scopes, known_names)?;
            }
            Statement::PatternBinding {
                pattern,
                value,
                span,
            } => {
                check_expression(value, scopes, known_names)?;
                declare_pattern_scope_names(pattern, scopes, *span);
            }
        }
    }
    if let Some(value) = &block.value {
        check_expression(value, scopes, known_names)?;
    }
    scopes.pop();
    Ok(())
}

fn declare_pattern_scope_names(
    pattern: &crate::ast::MatchPattern,
    scopes: &mut ScopeStack,
    span: Span,
) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            for pat in fields {
                declare_pattern_scope_names(pat, scopes, span);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            for pat in patterns {
                declare_pattern_scope_names(pat, scopes, span);
            }
        }
        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                declare_pattern_scope_names(alt, scopes, span);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::RangeBinding { name, .. } => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
    }
}

fn check_expression(
    expr: &Expression,
    scopes: &mut ScopeStack,
    known_functions: &HashSet<String>,
) -> Result<(), Diagnostic> {
    match expr {
        Expression::Binary { left, right, .. } => {
            check_expression(left, scopes, known_functions)?;
            check_expression(right, scopes, known_functions)?;
        }
        Expression::Unary { expr, .. } => check_expression(expr, scopes, known_functions)?,
        Expression::Spread(inner, _) => check_expression(inner, scopes, known_functions)?,
        Expression::List(items, _) => {
            for item in items {
                check_expression(item, scopes, known_functions)?;
            }
        }
        Expression::Map(entries, _) => {
            for (key, value) in entries {
                check_expression(key, scopes, known_functions)?;
                check_expression(value, scopes, known_functions)?;
            }
        }
        Expression::Call { callee, args, .. } => {
            check_expression(callee, scopes, known_functions)?;
            for arg in args {
                check_expression(arg, scopes, known_functions)?;
            }
        }
        Expression::Member { target, .. } => check_expression(target, scopes, known_functions)?,
        Expression::Index { target, index, .. } => {
            check_expression(target, scopes, known_functions)?;
            check_expression(index, scopes, known_functions)?;
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            check_expression(target, scopes, known_functions)?;
            check_expression(start, scopes, known_functions)?;
            check_expression(end, scopes, known_functions)?;
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            check_expression(condition, scopes, known_functions)?;
            check_expression(then_branch, scopes, known_functions)?;
            check_expression(else_branch, scopes, known_functions)?;
        }
        Expression::Pipeline { left, right, .. } => {
            check_expression(left, scopes, known_functions)?;
            check_expression(right, scopes, known_functions)?;
        }
        Expression::ErrorValue { .. } => {}
        Expression::ErrorPropagate { expr, .. } => {
            check_expression(expr, scopes, known_functions)?;
        }
        Expression::Match(match_expr) => {
            check_match_expression(match_expr, scopes, known_functions)?
        }
        Expression::Throw { value, .. } => check_expression(value, scopes, known_functions)?,
        Expression::Lambda { params, body, .. } => {
            check_lambda(params, body, scopes, known_functions)?
        }
        Expression::Identifier(name, span) => {
            if scopes.lookup(name).is_none()
                && !known_functions.contains(name)
                && !is_builtin_name(name)
            {
                return Err(Diagnostic::new(format!("undefined name `{}`", name), *span));
            }
        }
        Expression::ListComprehension {
            body,
            var,
            iterable,
            condition,
            ..
        } => {
            check_expression(iterable, scopes, known_functions)?;
            scopes.push();
            scopes.declare(var.clone(), iterable.span());
            check_expression(body, scopes, known_functions)?;
            if let Some(cond) = condition {
                check_expression(cond, scopes, known_functions)?;
            }
            scopes.pop();
        }
        Expression::MapComprehension {
            key,
            value,
            var,
            iterable,
            condition,
            ..
        } => {
            check_expression(iterable, scopes, known_functions)?;
            scopes.push();
            scopes.declare(var.clone(), iterable.span());
            check_expression(key, scopes, known_functions)?;
            check_expression(value, scopes, known_functions)?;
            if let Some(cond) = condition {
                check_expression(cond, scopes, known_functions)?;
            }
            scopes.pop();
        }
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
        | Expression::Unit
        | Expression::None(_) => {}
    }
    Ok(())
}

/// All built-in names recognized by the Coral runtime.
const BUILTIN_NAMES: &[&str] = &[
    "log",
    "io",
    "self",
    "true",
    "false",
    "bit_and",
    "bit_or",
    "bit_xor",
    "bit_not",
    "bit_shl",
    "bit_shr",
    "length",
    "push",
    "pop",
    "get",
    "set",
    "keys",
    "values",
    "abs",
    "sqrt",
    "floor",
    "ceil",
    "round",
    "sin",
    "cos",
    "tan",
    "ln",
    "log10",
    "exp",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "trunc",
    "sign",
    "signum",
    "deg_to_rad",
    "rad_to_deg",
    "min",
    "max",
    "pow",
    "is_number",
    "is_string",
    "is_bool",
    "is_list",
    "is_map",
    "concat",
    "split",
    "join",
    "trim",
    "to_string",
    "string_slice",
    "slice",
    "string_char_at",
    "char_at",
    "string_index_of",
    "index_of",
    "string_split",
    "string_to_chars",
    "chars",
    "string_starts_with",
    "starts_with",
    "string_ends_with",
    "ends_with",
    "string_trim",
    "string_to_upper",
    "to_upper",
    "string_to_lower",
    "to_lower",
    "string_replace",
    "replace",
    "string_contains",
    "contains",
    "string_parse_number",
    "parse_number",
    "number_to_string",
    "string_length",
    "bytes_length",
    "bytes_get",
    "bytes_set",
    "bytes_from_string",
    "to_bytes",
    "bytes_to_string",
    "bytes_slice",
    "fs_read",
    "fs_write",
    "fs_exists",
    "fs_append",
    "fs_read_dir",
    "read_dir",
    "fs_mkdir",
    "mkdir",
    "fs_delete",
    "delete",
    "fs_is_dir",
    "is_dir",
    "process_args",
    "args",
    "process_exit",
    "exit",
    "env_get",
    "env_set",
    "stdin_read_line",
    "read_line",
    "list_contains",
    "list_index_of",
    "list_reverse",
    "list_slice",
    "list_sort",
    "list_join",
    "list_concat",
    "map_remove",
    "map_values",
    "map_keys",
    "map_entries",
    "entries",
    "map_has_key",
    "has_key",
    "map_merge",
    "merge",
    "type_of",
    "is_err",
    "is_ok",
    "is_absent",
    "error_name",
    "error_code",
    "ord",
    "string_ord",
    "chr",
    "string_chr",
    "string_compare",
    "strcmp",
    "actor_spawn",
    "actor_send",
    "actor_stop",
    "actor_self",
    "actor_monitor",
    "monitor",
    "actor_demonitor",
    "demonitor",
    "actor_graceful_stop",
    "graceful_stop",
    "json_parse",
    "json_serialize",
    "json_stringify",
    "json_serialize_pretty",
    "time_now",
    "time_timestamp",
    "time_format_iso",
    "time_year",
    "time_month",
    "time_day",
    "time_hour",
    "time_minute",
    "time_second",
    "time_sleep",
    "random",
    "random_int",
    "random_seed",
    "string_lines",
    "sort_natural",
    "list_sort_natural",
    "bytes_from_hex",
    "bytes_contains",
    "bytes_find",
    "base64_encode",
    "base64_decode",
    "hex_encode",
    "hex_decode",
    "tcp_listen",
    "tcp_accept",
    "tcp_connect",
    "tcp_read",
    "tcp_write",
    "tcp_close",
    "http_get",
    "http_post",
    "http_request",
    "value_retain",
    "value_release",
    "heap_alloc",
    "heap_free",
    "range",
    "sb_new",
    "sb_push",
    "sb_finish",
    "sb_len",
    "string_join_list",
    "join_list",
    "string_repeat",
    "repeat_string",
    "string_reverse",
    "reverse_string",
    "value_to_string",
    "stderr_write",
    "eprint",
    "fs_size",
    "file_size",
    "fs_rename",
    "fs_copy",
    "fs_mkdirs",
    "make_dirs",
    "fs_temp_dir",
    "temp_dir",
    "process_exec",
    "exec",
    "process_cwd",
    "cwd",
    "process_chdir",
    "chdir",
    "process_pid",
    "process_hostname",
    "hostname",
    "path_normalize",
    "normalize",
    "path_resolve",
    "resolve",
    "path_is_absolute",
    "is_absolute",
    "path_parent",
    "path_stem",
    "stem",
    "regex_match",
    "regex_find",
    "regex_find_all",
    "regex_replace",
    "regex_split",
    "inspect",
    "debug_inspect",
    "time_ns",
    "debug_time_ns",
];

fn is_builtin_name(name: &str) -> bool {
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let set = SET.get_or_init(|| BUILTIN_NAMES.iter().copied().collect());
    set.contains(name)
}

fn check_match_expression(
    expr: &MatchExpression,
    scopes: &mut ScopeStack,
    known_names: &HashSet<String>,
) -> Result<(), Diagnostic> {
    check_expression(&expr.value, scopes, known_names)?;
    for arm in &expr.arms {
        scopes.push();
        declare_pattern_bindings(&arm.pattern, scopes, arm.body.span);

        if let Some(guard) = &arm.guard {
            check_expression(guard, scopes, known_names)?;
        }
        check_block(&arm.body, scopes, known_names)?;
        scopes.pop();
    }
    if let Some(default) = &expr.default {
        check_block(default, scopes, known_names)?;
    }
    Ok(())
}

fn declare_pattern_bindings(
    pattern: &crate::ast::MatchPattern,
    scopes: &mut ScopeStack,
    span: crate::span::Span,
) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            for field_pattern in fields {
                declare_pattern_bindings(field_pattern, scopes, span);
            }
        }

        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                declare_pattern_bindings(alt, scopes, span);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            for field_pattern in patterns {
                declare_pattern_bindings(field_pattern, scopes, span);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::RangeBinding { name, .. } => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
    }
}

fn build_constructor_map(
    type_defs: &[crate::ast::TypeDefinition],
) -> HashMap<String, (String, Vec<String>)> {
    let mut map = HashMap::new();
    for typedef in type_defs {
        if !typedef.variants.is_empty() {
            let variant_names: Vec<String> =
                typedef.variants.iter().map(|v| v.name.clone()).collect();
            for variant in &typedef.variants {
                map.insert(
                    variant.name.clone(),
                    (typedef.name.clone(), variant_names.clone()),
                );
            }
        }
    }
    map
}

fn check_all_match_exhaustiveness(
    globals: &[Binding],
    functions: &[Function],
    type_defs: &[crate::ast::TypeDefinition],
    warnings: &mut Vec<Diagnostic>,
) {
    let ctor_map = build_constructor_map(type_defs);

    let enum_variants: HashMap<String, Vec<String>> = type_defs
        .iter()
        .filter(|td| !td.variants.is_empty())
        .map(|td| {
            (
                td.name.clone(),
                td.variants.iter().map(|v| v.name.clone()).collect(),
            )
        })
        .collect();

    for binding in globals {
        check_expr_match_exhaustiveness(&binding.value, &ctor_map, &enum_variants, warnings);
    }

    for function in functions {
        check_block_match_exhaustiveness(&function.body, &ctor_map, &enum_variants, warnings);
    }
}

fn check_block_match_exhaustiveness(
    block: &Block,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
    enum_variants: &HashMap<String, Vec<String>>,
    warnings: &mut Vec<Diagnostic>,
) {
    for statement in &block.statements {
        match statement {
            Statement::Binding(binding) => {
                check_expr_match_exhaustiveness(&binding.value, ctor_map, enum_variants, warnings);
            }
            Statement::Expression(expr) => {
                check_expr_match_exhaustiveness(expr, ctor_map, enum_variants, warnings);
            }
            Statement::Return(expr, _) => {
                check_expr_match_exhaustiveness(expr, ctor_map, enum_variants, warnings);
            }
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
                check_expr_match_exhaustiveness(condition, ctor_map, enum_variants, warnings);
                check_block_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
                for (cond, blk) in elif_branches {
                    check_expr_match_exhaustiveness(cond, ctor_map, enum_variants, warnings);
                    check_block_match_exhaustiveness(blk, ctor_map, enum_variants, warnings);
                }
                if let Some(else_blk) = else_body {
                    check_block_match_exhaustiveness(else_blk, ctor_map, enum_variants, warnings);
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                check_expr_match_exhaustiveness(condition, ctor_map, enum_variants, warnings);
                check_block_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
            }
            Statement::For { iterable, body, .. } => {
                check_expr_match_exhaustiveness(iterable, ctor_map, enum_variants, warnings);
                check_block_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
            }
            Statement::ForKV { iterable, body, .. } => {
                check_expr_match_exhaustiveness(iterable, ctor_map, enum_variants, warnings);
                check_block_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
            }
            Statement::ForRange {
                start,
                end,
                step,
                body,
                ..
            } => {
                check_expr_match_exhaustiveness(start, ctor_map, enum_variants, warnings);
                check_expr_match_exhaustiveness(end, ctor_map, enum_variants, warnings);
                if let Some(s) = step {
                    check_expr_match_exhaustiveness(s, ctor_map, enum_variants, warnings);
                }
                check_block_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { value, .. } => {
                check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
            }
            Statement::PatternBinding { value, .. } => {
                check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
            }
        }
    }
    if let Some(value) = &block.value {
        check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
    }
}

fn check_expr_match_exhaustiveness(
    expr: &Expression,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
    enum_variants: &HashMap<String, Vec<String>>,
    warnings: &mut Vec<Diagnostic>,
) {
    match expr {
        Expression::Binary { left, right, .. } => {
            check_expr_match_exhaustiveness(left, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(right, ctor_map, enum_variants, warnings);
        }
        Expression::Unary { expr, .. } => {
            check_expr_match_exhaustiveness(expr, ctor_map, enum_variants, warnings);
        }
        Expression::Spread(inner, _) => {
            check_expr_match_exhaustiveness(inner, ctor_map, enum_variants, warnings);
        }
        Expression::List(items, _) => {
            for item in items {
                check_expr_match_exhaustiveness(item, ctor_map, enum_variants, warnings);
            }
        }
        Expression::Map(entries, _) => {
            for (key, value) in entries {
                check_expr_match_exhaustiveness(key, ctor_map, enum_variants, warnings);
                check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
            }
        }
        Expression::Call { callee, args, .. } => {
            check_expr_match_exhaustiveness(callee, ctor_map, enum_variants, warnings);
            for arg in args {
                check_expr_match_exhaustiveness(arg, ctor_map, enum_variants, warnings);
            }
        }
        Expression::Member { target, .. } => {
            check_expr_match_exhaustiveness(target, ctor_map, enum_variants, warnings);
        }
        Expression::Index { target, index, .. } => {
            check_expr_match_exhaustiveness(target, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(index, ctor_map, enum_variants, warnings);
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            check_expr_match_exhaustiveness(target, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(start, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(end, ctor_map, enum_variants, warnings);
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            check_expr_match_exhaustiveness(condition, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(then_branch, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(else_branch, ctor_map, enum_variants, warnings);
        }
        Expression::Match(match_expr) => {
            check_expr_match_exhaustiveness(&match_expr.value, ctor_map, enum_variants, warnings);
            for arm in &match_expr.arms {
                check_block_match_exhaustiveness(&arm.body, ctor_map, enum_variants, warnings);
            }
            if let Some(default) = &match_expr.default {
                check_block_match_exhaustiveness(default, ctor_map, enum_variants, warnings);
            }

            check_single_match_exhaustiveness(match_expr, ctor_map, enum_variants, warnings);
            check_overlapping_patterns(match_expr, warnings);
        }
        Expression::Throw { value, .. } => {
            check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
        }
        Expression::Lambda { body, .. } => {
            check_block_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
        }
        Expression::Pipeline { left, right, .. } => {
            check_expr_match_exhaustiveness(left, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(right, ctor_map, enum_variants, warnings);
        }
        Expression::ErrorValue { .. } => {}
        Expression::ErrorPropagate { expr, .. } => {
            check_expr_match_exhaustiveness(expr, ctor_map, enum_variants, warnings);
        }
        Expression::ListComprehension {
            body,
            iterable,
            condition,
            ..
        } => {
            check_expr_match_exhaustiveness(iterable, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
            if let Some(cond) = condition {
                check_expr_match_exhaustiveness(cond, ctor_map, enum_variants, warnings);
            }
        }
        Expression::MapComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => {
            check_expr_match_exhaustiveness(iterable, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(key, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
            if let Some(cond) = condition {
                check_expr_match_exhaustiveness(cond, ctor_map, enum_variants, warnings);
            }
        }

        Expression::Identifier(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::Float(_, _)
        | Expression::Integer(_, _)
        | Expression::TaxonomyPath { .. }
        | Expression::Placeholder(_, _)
        | Expression::InlineAsm { .. }
        | Expression::PtrLoad { .. }
        | Expression::Unsafe { .. }
        | Expression::Unit
        | Expression::None(_) => {}
    }
}

fn collect_pattern_ctors_for_exhaustiveness(
    pattern: &crate::ast::MatchPattern,
    matched_ctors: &mut HashSet<String>,
    has_wildcard: &mut bool,
    has_identifier_catch_all: &mut bool,
) {
    match pattern {
        crate::ast::MatchPattern::Constructor { name, .. } => {
            matched_ctors.insert(name.clone());
        }
        crate::ast::MatchPattern::Wildcard(_) => {
            *has_wildcard = true;
        }
        crate::ast::MatchPattern::Identifier(_) => {
            *has_identifier_catch_all = true;
        }
        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                collect_pattern_ctors_for_exhaustiveness(
                    alt,
                    matched_ctors,
                    has_wildcard,
                    has_identifier_catch_all,
                );
            }
        }

        _ => {}
    }
}

fn pattern_key(pattern: &crate::ast::MatchPattern) -> Option<String> {
    match pattern {
        crate::ast::MatchPattern::Integer(n) => Some(format!("int:{}", n)),
        crate::ast::MatchPattern::Bool(b) => Some(format!("bool:{}", b)),
        crate::ast::MatchPattern::String(s) => Some(format!("str:{}", s)),
        crate::ast::MatchPattern::Constructor { name, fields, .. } if fields.is_empty() => {
            Some(format!("ctor:{}", name))
        }
        _ => None,
    }
}

fn check_overlapping_patterns(match_expr: &MatchExpression, warnings: &mut Vec<Diagnostic>) {
    let mut seen: HashSet<String> = HashSet::new();
    for (i, arm) in match_expr.arms.iter().enumerate() {
        let keys = collect_pattern_keys(&arm.pattern);
        for key in keys {
            if !seen.insert(key.clone()) {
                warnings.push(
                    Diagnostic::warning(
                        format!("unreachable pattern in match arm {}: pattern already matched by a previous arm", i + 1),
                        match_expr.span,
                    )
                    .with_help("Remove the duplicate arm or use a guard to distinguish it."),
                );
                break;
            }
        }
    }
}

fn collect_pattern_keys(pattern: &crate::ast::MatchPattern) -> Vec<String> {
    match pattern {
        crate::ast::MatchPattern::Or(alternatives) => {
            alternatives.iter().filter_map(|p| pattern_key(p)).collect()
        }
        _ => pattern_key(pattern).into_iter().collect(),
    }
}

fn check_single_match_exhaustiveness(
    match_expr: &MatchExpression,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
    enum_variants: &HashMap<String, Vec<String>>,
    warnings: &mut Vec<Diagnostic>,
) {
    if match_expr.default.is_some() {
        return;
    }

    let mut matched_ctors: HashSet<String> = HashSet::new();
    let mut has_wildcard = false;
    let mut has_identifier_catch_all = false;

    for arm in &match_expr.arms {
        if arm.guard.is_some() {
            continue;
        }
        collect_pattern_ctors_for_exhaustiveness(
            &arm.pattern,
            &mut matched_ctors,
            &mut has_wildcard,
            &mut has_identifier_catch_all,
        );
    }

    if has_wildcard || has_identifier_catch_all {
        return;
    }

    if matched_ctors.is_empty() {
        return;
    }

    let first_ctor = matched_ctors
        .iter()
        .next()
        .expect("matched_ctors must be non-empty after is_empty check");
    if let Some((enum_name, all_variants)) = ctor_map.get(first_ctor) {
        for ctor in &matched_ctors {
            if let Some((other_enum, _)) = ctor_map.get(ctor) {
                if other_enum != enum_name {
                    return;
                }
            }
        }

        let missing: Vec<&String> = all_variants
            .iter()
            .filter(|v| !matched_ctors.contains(*v))
            .collect();

        if !missing.is_empty() {
            let missing_list = missing
                .iter()
                .map(|s| format!("`{}`", s))
                .collect::<Vec<_>>()
                .join(", ");

            warnings.push(
                Diagnostic::new(
                    format!(
                        "non-exhaustive match: missing pattern(s) for {}",
                        missing_list
                    ),
                    match_expr.span,
                )
                .with_help(
                    "add arm(s) for the missing variant(s) or add a default arm".to_string(),
                ),
            );
            return;
        }

        check_nested_exhaustiveness(
            &match_expr.arms,
            ctor_map,
            enum_variants,
            match_expr.span,
            warnings,
        );
    }
}

fn check_nested_exhaustiveness(
    arms: &[crate::ast::MatchArm],
    ctor_map: &HashMap<String, (String, Vec<String>)>,
    _enum_variants: &HashMap<String, Vec<String>>,
    span: Span,
    warnings: &mut Vec<Diagnostic>,
) {
    let mut groups: HashMap<String, Vec<&[crate::ast::MatchPattern]>> = HashMap::new();

    for arm in arms {
        if let crate::ast::MatchPattern::Constructor { name, fields, .. } = &arm.pattern {
            groups.entry(name.clone()).or_default().push(fields);
        }
    }

    for (ctor_name, field_groups) in &groups {
        if field_groups.is_empty() {
            continue;
        }

        let max_fields = field_groups.iter().map(|f| f.len()).max().unwrap_or(0);

        for field_idx in 0..max_fields {
            let mut sub_ctors: HashSet<String> = HashSet::new();
            let mut has_catch_all = false;

            for fields in field_groups {
                if field_idx < fields.len() {
                    match &fields[field_idx] {
                        crate::ast::MatchPattern::Constructor { name, .. } => {
                            sub_ctors.insert(name.clone());
                        }
                        crate::ast::MatchPattern::Identifier(_)
                        | crate::ast::MatchPattern::Wildcard(_) => {
                            has_catch_all = true;
                        }
                        _ => {}
                    }
                } else {
                    has_catch_all = true;
                }
            }

            if has_catch_all || sub_ctors.is_empty() {
                continue;
            }

            let first_sub = sub_ctors.iter().next().unwrap();
            if let Some((sub_enum_name, sub_all_variants)) = ctor_map.get(first_sub) {
                let sub_missing: Vec<&String> = sub_all_variants
                    .iter()
                    .filter(|v| !sub_ctors.contains(*v))
                    .collect();

                if !sub_missing.is_empty() {
                    let missing_list = sub_missing
                        .iter()
                        .map(|s| format!("`{}`", s))
                        .collect::<Vec<_>>()
                        .join(", ");

                    warnings.push(
                        Diagnostic::new(
                            format!(
                                "non-exhaustive match: within `{}`, nested pattern(s) for {} of `{}` are missing",
                                ctor_name, missing_list, sub_enum_name
                            ),
                            span,
                        ).with_help(
                            format!("add arm(s) for `{}({})` or use a catch-all pattern", ctor_name,
                                sub_missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                        )
                    );
                }
            }
        }
    }
}

fn check_field_uniqueness(
    owner_kind: &str,
    owner_name: &str,
    fields: &[Field],
) -> Result<(), Diagnostic> {
    let mut seen = HashMap::new();
    for field in fields {
        if let Some(previous) = seen.insert(field.name.clone(), field.span) {
            return Err(Diagnostic::new(
                format!("duplicate field `{}.{}`", owner_name, field.name),
                field.span,
            )
            .with_help(format!(
                "previous {} field defined at {}",
                owner_kind, previous
            )));
        }
    }
    Ok(())
}

fn check_lambda(
    params: &[Parameter],
    body: &Block,
    scopes: &mut ScopeStack,
    known_names: &HashSet<String>,
) -> Result<(), Diagnostic> {
    scopes.push();
    for param in params {
        if let Some(previous) = scopes.lookup(&param.name) {
            return Err(duplicate_symbol(
                "parameter",
                &param.name,
                param.span,
                previous,
            ));
        }
        scopes.declare(param.name.clone(), param.span);
        if let Some(default) = &param.default {
            check_expression(default, scopes, known_names)?;
        }
    }
    check_block(body, scopes, known_names)?;
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

        let strategy = if usage.closure_captures > 0 {
            AllocationStrategy::Heap
        } else if usage.mutations > 0 {
            AllocationStrategy::Heap
        } else if usage.returned > 0 && usage.escapes == usage.returned {
            AllocationStrategy::SharedCow
        } else if usage.escapes > 0 {
            AllocationStrategy::SharedCow
        } else {
            AllocationStrategy::Stack
        };

        mut_env.insert(name.clone(), mutability);
        alloc.insert(name.clone(), strategy);
    }

    (
        UsageMetrics {
            symbols: tracker.usage,
        },
        mut_env,
        alloc,
    )
}

#[derive(Default)]
struct UsageTracker {
    usage: HashMap<String, SymbolUsage>,
}

impl UsageTracker {
    fn touch(&mut self, name: &str) {
        self.usage.entry(name.to_string()).or_default();
    }

    fn read(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_default();
        entry.reads += 1;
    }

    fn mutate(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_default();
        entry.mutations += 1;
    }

    fn escape(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_default();
        entry.escapes += 1;
    }

    fn call(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_default();
        entry.calls += 1;
    }

    fn capture(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_default();
        entry.closure_captures += 1;

        entry.escapes += 1;
    }

    fn mark_returned(&mut self, name: &str) {
        let entry = self.usage.entry(name.to_string()).or_default();
        entry.returned += 1;
        entry.escapes += 1;
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
                    mark_returns(expr, tracker);
                }
            }
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
                visit_expression(condition, tracker);
                visit_block(body, tracker, mark_returns_as_escape);
                for (cond, blk) in elif_branches {
                    visit_expression(cond, tracker);
                    visit_block(blk, tracker, mark_returns_as_escape);
                }
                if let Some(else_blk) = else_body {
                    visit_block(else_blk, tracker, mark_returns_as_escape);
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                visit_expression(condition, tracker);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::For {
                iterable,
                body,
                variable,
                ..
            } => {
                visit_expression(iterable, tracker);
                tracker.touch(variable);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::ForKV {
                key_var,
                value_var,
                iterable,
                body,
                ..
            } => {
                visit_expression(iterable, tracker);
                tracker.touch(key_var);
                tracker.touch(value_var);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::ForRange {
                start,
                end,
                step,
                body,
                variable,
                ..
            } => {
                visit_expression(start, tracker);
                visit_expression(end, tracker);
                if let Some(s) = step {
                    visit_expression(s, tracker);
                }
                tracker.touch(variable);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { target, value, .. } => {
                visit_expression(target, tracker);
                visit_expression(value, tracker);
            }
            Statement::PatternBinding { pattern, value, .. } => {
                visit_expression(value, tracker);
                touch_pattern_names(pattern, tracker);
            }
        }
    }
    if let Some(value) = &block.value {
        visit_expression(value, tracker);
        if mark_returns_as_escape {
            mark_returns(value, tracker);
        }
    }
}

fn touch_pattern_names(pattern: &crate::ast::MatchPattern, tracker: &mut UsageTracker) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            tracker.touch(name);
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            for pat in fields {
                touch_pattern_names(pat, tracker);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            for pat in patterns {
                touch_pattern_names(pat, tracker);
            }
        }
        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                touch_pattern_names(alt, tracker);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            tracker.touch(name);
        }
        crate::ast::MatchPattern::RangeBinding { name, .. } => {
            tracker.touch(name);
        }
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
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
        | Expression::None(_)
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
        Expression::Spread(inner, _) => visit_expression(inner, tracker),
        Expression::Member { target, .. } => visit_expression(target, tracker),
        Expression::Index { target, index, .. } => {
            visit_expression(target, tracker);
            visit_expression(index, tracker);
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            visit_expression(target, tracker);
            visit_expression(start, tracker);
            visit_expression(end, tracker);
        }
        Expression::Call { callee, args, .. } => {
            visit_expression(callee, tracker);
            if let Expression::Identifier(name, _) = callee.as_ref() {
                tracker.call(name);
            }
            if let Expression::Member {
                target, property, ..
            } = callee.as_ref()
            {
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
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            visit_expression(condition, tracker);
            visit_expression(then_branch, tracker);
            visit_expression(else_branch, tracker);
        }
        Expression::Match(match_expr) => {
            visit_expression(&match_expr.value, tracker);
            for arm in &match_expr.arms {
                if let Some(guard) = &arm.guard {
                    visit_expression(guard, tracker);
                }
                visit_block(&arm.body, tracker, false);
            }
            if let Some(default) = &match_expr.default {
                visit_block(default, tracker, false);
            }
        }
        Expression::Throw { value, .. } => visit_expression(value, tracker),
        Expression::Lambda { params, body, .. } => {
            let param_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
            for p in params {
                tracker.touch(&p.name);
            }

            let mut lambda_tracker = UsageTracker::default();
            visit_block(body, &mut lambda_tracker, false);
            for name in lambda_tracker.usage.keys() {
                if !param_names.contains(name) {
                    tracker.capture(name);
                }
            }

            visit_block(body, tracker, false);
        }
        Expression::Pipeline { left, right, .. } => {
            visit_expression(left, tracker);
            visit_expression(right, tracker);
        }
        Expression::ErrorValue { .. } => {}
        Expression::ErrorPropagate { expr, .. } => visit_expression(expr, tracker),
        Expression::InlineAsm { .. } | Expression::PtrLoad { .. } | Expression::Unsafe { .. } => {}
        Expression::ListComprehension {
            body,
            var,
            iterable,
            condition,
            ..
        } => {
            visit_expression(iterable, tracker);
            tracker.touch(var);
            visit_expression(body, tracker);
            if let Some(cond) = condition {
                visit_expression(cond, tracker);
            }
        }
        Expression::MapComprehension {
            key,
            value,
            var,
            iterable,
            condition,
            ..
        } => {
            visit_expression(iterable, tracker);
            tracker.touch(var);
            visit_expression(key, tracker);
            visit_expression(value, tracker);
            if let Some(cond) = condition {
                visit_expression(cond, tracker);
            }
        }
    }
}

fn mark_returns(expr: &Expression, tracker: &mut UsageTracker) {
    match expr {
        Expression::Identifier(name, _) => tracker.mark_returned(name),

        Expression::List(items, _) => {
            for item in items {
                mark_returns(item, tracker);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                mark_returns(k, tracker);
                mark_returns(v, tracker);
            }
        }
        Expression::Ternary {
            condition: _,
            then_branch,
            else_branch,
            ..
        } => {
            mark_returns(then_branch, tracker);
            mark_returns(else_branch, tracker);
        }
        _ => {
            mark_escapes(expr, tracker);
        }
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
        Expression::Spread(inner, _) => mark_escapes(inner, tracker),
        Expression::Call { callee, args, .. } => {
            mark_escapes(callee, tracker);
            for arg in args {
                mark_escapes(arg, tracker);
            }
        }
        Expression::Member { target, .. } => mark_escapes(target, tracker),
        Expression::Index { target, index, .. } => {
            mark_escapes(target, tracker);
            mark_escapes(index, tracker);
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            mark_escapes(target, tracker);
            mark_escapes(start, tracker);
            mark_escapes(end, tracker);
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
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
        Expression::Pipeline { left, right, .. } => {
            mark_escapes(left, tracker);
            mark_escapes(right, tracker);
        }
        Expression::ErrorValue { .. } => {}
        Expression::ErrorPropagate { expr, .. } => mark_escapes(expr, tracker),
        Expression::ListComprehension {
            body,
            iterable,
            condition,
            ..
        } => {
            mark_escapes(iterable, tracker);
            mark_escapes(body, tracker);
            if let Some(cond) = condition {
                mark_escapes(cond, tracker);
            }
        }
        Expression::MapComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => {
            mark_escapes(iterable, tracker);
            mark_escapes(key, tracker);
            mark_escapes(value, tracker);
            if let Some(cond) = condition {
                mark_escapes(cond, tracker);
            }
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Unit
        | Expression::None(_)
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
    "push", "pop", "append", "insert", "remove", "clear", "update", "set", "add_item",
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
        Expression::Binary { left, right, .. } => find_forbidden_identifier(left, forbidden)
            .or_else(|| find_forbidden_identifier(right, forbidden)),
        Expression::Unary { expr, .. } => find_forbidden_identifier(expr, forbidden),
        Expression::Spread(inner, _) => find_forbidden_identifier(inner, forbidden),
        Expression::List(items, _) => items
            .iter()
            .find_map(|item| find_forbidden_identifier(item, forbidden)),
        Expression::Map(entries, _) => entries.iter().find_map(|(key, value)| {
            find_forbidden_identifier(key, forbidden)
                .or_else(|| find_forbidden_identifier(value, forbidden))
        }),
        Expression::Call { callee, args, .. } => find_forbidden_identifier(callee, forbidden)
            .or_else(|| {
                args.iter()
                    .find_map(|arg| find_forbidden_identifier(arg, forbidden))
            }),
        Expression::Member { target, .. } => find_forbidden_identifier(target, forbidden),
        Expression::Index { target, index, .. } => find_forbidden_identifier(target, forbidden)
            .or_else(|| find_forbidden_identifier(index, forbidden)),
        Expression::Slice {
            target, start, end, ..
        } => find_forbidden_identifier(target, forbidden)
            .or_else(|| find_forbidden_identifier(start, forbidden))
            .or_else(|| find_forbidden_identifier(end, forbidden)),
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
        Expression::Pipeline { left, right, .. } => find_forbidden_identifier(left, forbidden)
            .or_else(|| find_forbidden_identifier(right, forbidden)),
        Expression::ErrorValue { .. } => None,
        Expression::ErrorPropagate { expr, .. } => find_forbidden_identifier(expr, forbidden),
        Expression::ListComprehension {
            body,
            iterable,
            condition,
            ..
        } => find_forbidden_identifier(iterable, forbidden)
            .or_else(|| find_forbidden_identifier(body, forbidden))
            .or_else(|| {
                condition
                    .as_ref()
                    .and_then(|c| find_forbidden_identifier(c, forbidden))
            }),
        Expression::MapComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => find_forbidden_identifier(iterable, forbidden)
            .or_else(|| find_forbidden_identifier(key, forbidden))
            .or_else(|| find_forbidden_identifier(value, forbidden))
            .or_else(|| {
                condition
                    .as_ref()
                    .and_then(|c| find_forbidden_identifier(c, forbidden))
            }),
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
        | Expression::Unit
        | Expression::None(_) => None,
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
        .or_else(|| {
            match_expr
                .default
                .as_ref()
                .and_then(|block| find_in_block(block, forbidden))
        })
}

fn find_in_block(block: &Block, forbidden: &HashMap<String, Span>) -> Option<(String, Span, Span)> {
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
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => find_forbidden_identifier(condition, forbidden)
            .or_else(|| find_in_block(body, forbidden))
            .or_else(|| {
                elif_branches.iter().find_map(|(cond, blk)| {
                    find_forbidden_identifier(cond, forbidden)
                        .or_else(|| find_in_block(blk, forbidden))
                })
            })
            .or_else(|| {
                else_body
                    .as_ref()
                    .and_then(|blk| find_in_block(blk, forbidden))
            }),
        Statement::While {
            condition, body, ..
        } => find_forbidden_identifier(condition, forbidden)
            .or_else(|| find_in_block(body, forbidden)),
        Statement::For { iterable, body, .. } => find_forbidden_identifier(iterable, forbidden)
            .or_else(|| find_in_block(body, forbidden)),
        Statement::ForKV { iterable, body, .. } => find_forbidden_identifier(iterable, forbidden)
            .or_else(|| find_in_block(body, forbidden)),
        Statement::ForRange {
            start,
            end,
            step,
            body,
            ..
        } => find_forbidden_identifier(start, forbidden)
            .or_else(|| find_forbidden_identifier(end, forbidden))
            .or_else(|| {
                step.as_ref()
                    .and_then(|s| find_forbidden_identifier(s, forbidden))
            })
            .or_else(|| find_in_block(body, forbidden)),
        Statement::Break(_) | Statement::Continue(_) => None,
        Statement::FieldAssign { target, value, .. } => {
            find_forbidden_identifier(target, forbidden)
                .or_else(|| find_forbidden_identifier(value, forbidden))
        }
        Statement::PatternBinding { value, .. } => find_forbidden_identifier(value, forbidden),
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

fn check_unhandled_errors(
    globals: &[Binding],
    functions: &[Function],
    warnings: &mut Vec<Diagnostic>,
) {
    for binding in globals {
        if let Some(warning) = check_expr_may_produce_unhandled_error(&binding.value) {
            warnings.push(warning);
        }
    }

    for function in functions {
        check_block_for_unhandled_errors(&function.body, warnings);
    }
}

fn check_expr_may_produce_unhandled_error(expr: &Expression) -> Option<Diagnostic> {
    match expr {
        Expression::ErrorValue { span, path } => Some(
            Diagnostic::new(
                format!(
                    "error value `err {}` is created but may not be handled",
                    path.join(":")
                ),
                *span,
            )
            .with_help("consider returning this error or handling it with a conditional"),
        ),
        _ => None,
    }
}

fn check_block_for_unhandled_errors(block: &Block, warnings: &mut Vec<Diagnostic>) {
    for statement in &block.statements {
        match statement {
            Statement::Expression(expr) => {
                if let Some(warning) = check_expr_may_produce_unhandled_error(expr) {
                    warnings.push(warning);
                }

                check_expr_nested_blocks(expr, warnings);
            }
            Statement::Binding(binding) => {
                check_expr_nested_blocks(&binding.value, warnings);
            }
            Statement::Return(expr, _) => {
                check_expr_nested_blocks(expr, warnings);
            }
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
                check_expr_nested_blocks(condition, warnings);
                check_block_for_unhandled_errors(body, warnings);
                for (cond, blk) in elif_branches {
                    check_expr_nested_blocks(cond, warnings);
                    check_block_for_unhandled_errors(blk, warnings);
                }
                if let Some(else_blk) = else_body {
                    check_block_for_unhandled_errors(else_blk, warnings);
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                check_expr_nested_blocks(condition, warnings);
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::For { iterable, body, .. } => {
                check_expr_nested_blocks(iterable, warnings);
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::ForKV { iterable, body, .. } => {
                check_expr_nested_blocks(iterable, warnings);
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::ForRange {
                start,
                end,
                step,
                body,
                ..
            } => {
                check_expr_nested_blocks(start, warnings);
                check_expr_nested_blocks(end, warnings);
                if let Some(s) = step {
                    check_expr_nested_blocks(s, warnings);
                }
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { value, .. } => {
                check_expr_nested_blocks(value, warnings);
            }
            Statement::PatternBinding { value, .. } => {
                check_expr_nested_blocks(value, warnings);
            }
        }
    }

    if let Some(value) = &block.value {
        check_expr_nested_blocks(value, warnings);
    }
}

fn check_expr_nested_blocks(expr: &Expression, warnings: &mut Vec<Diagnostic>) {
    match expr {
        Expression::Lambda { body, .. } => {
            check_block_for_unhandled_errors(body, warnings);
        }
        Expression::Match(m) => {
            for arm in &m.arms {
                check_block_for_unhandled_errors(&arm.body, warnings);
            }
            if let Some(default) = &m.default {
                check_block_for_unhandled_errors(default, warnings);
            }
        }
        Expression::Ternary {
            then_branch,
            else_branch,
            ..
        } => {
            check_expr_nested_blocks(then_branch, warnings);
            check_expr_nested_blocks(else_branch, warnings);
        }
        Expression::Binary { left, right, .. } => {
            check_expr_nested_blocks(left, warnings);
            check_expr_nested_blocks(right, warnings);
        }
        Expression::Unary { expr: inner, .. } => {
            check_expr_nested_blocks(inner, warnings);
        }
        Expression::Spread(inner, _) => {
            check_expr_nested_blocks(inner, warnings);
        }
        Expression::Call { callee, args, .. } => {
            check_expr_nested_blocks(callee, warnings);
            for arg in args {
                check_expr_nested_blocks(arg, warnings);
            }
        }
        Expression::Pipeline { left, right, .. } => {
            check_expr_nested_blocks(left, warnings);
            check_expr_nested_blocks(right, warnings);
        }
        Expression::ErrorPropagate { expr: inner, .. } => {
            check_expr_nested_blocks(inner, warnings);
        }
        Expression::Member { target, .. } => {
            check_expr_nested_blocks(target, warnings);
        }
        Expression::Index { target, index, .. } => {
            check_expr_nested_blocks(target, warnings);
            check_expr_nested_blocks(index, warnings);
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            check_expr_nested_blocks(target, warnings);
            check_expr_nested_blocks(start, warnings);
            check_expr_nested_blocks(end, warnings);
        }
        Expression::List(items, _) => {
            for item in items {
                check_expr_nested_blocks(item, warnings);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                check_expr_nested_blocks(k, warnings);
                check_expr_nested_blocks(v, warnings);
            }
        }
        Expression::Unsafe { block, .. } => {
            check_block_for_unhandled_errors(block, warnings);
        }

        Expression::Unit
        | Expression::None(_)
        | Expression::Identifier(_, _)
        | Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Placeholder(_, _)
        | Expression::TaxonomyPath { .. }
        | Expression::ErrorValue { .. }
        | Expression::Throw { .. }
        | Expression::InlineAsm { .. }
        | Expression::PtrLoad { .. } => {}
        Expression::ListComprehension {
            body,
            iterable,
            condition,
            ..
        } => {
            check_expr_nested_blocks(iterable, warnings);
            check_expr_nested_blocks(body, warnings);
            if let Some(cond) = condition {
                check_expr_nested_blocks(cond, warnings);
            }
        }
        Expression::MapComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => {
            check_expr_nested_blocks(iterable, warnings);
            check_expr_nested_blocks(key, warnings);
            check_expr_nested_blocks(value, warnings);
            if let Some(cond) = condition {
                check_expr_nested_blocks(cond, warnings);
            }
        }
    }
}

fn collect_error_types_from_block(block: &Block, error_types: &mut Vec<(Vec<String>, Span)>) {
    for statement in &block.statements {
        match statement {
            Statement::Return(expr, _) => {
                collect_error_types_from_expr(expr, error_types);
            }
            Statement::Expression(expr) => {
                collect_error_types_from_expr_nested(expr, error_types);
            }
            Statement::Binding(binding) => {
                collect_error_types_from_expr_nested(&binding.value, error_types);
            }
            Statement::If {
                body,
                elif_branches,
                else_body,
                ..
            } => {
                collect_error_types_from_block(body, error_types);
                for (_, blk) in elif_branches {
                    collect_error_types_from_block(blk, error_types);
                }
                if let Some(else_blk) = else_body {
                    collect_error_types_from_block(else_blk, error_types);
                }
            }
            Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::ForKV { body, .. } => {
                collect_error_types_from_block(body, error_types);
            }
            Statement::ForRange { body, .. } => {
                collect_error_types_from_block(body, error_types);
            }
            _ => {}
        }
    }
}

fn collect_error_types_from_expr(expr: &Expression, error_types: &mut Vec<(Vec<String>, Span)>) {
    match expr {
        Expression::ErrorValue { path, span } => {
            if !path.is_empty() {
                error_types.push((path.clone(), *span));
            }
        }
        Expression::Ternary {
            then_branch,
            else_branch,
            ..
        } => {
            collect_error_types_from_expr(then_branch, error_types);
            collect_error_types_from_expr(else_branch, error_types);
        }
        _ => {}
    }
}

fn collect_error_types_from_expr_nested(
    expr: &Expression,
    error_types: &mut Vec<(Vec<String>, Span)>,
) {
    match expr {
        Expression::Lambda { body, .. } => {
            collect_error_types_from_block(body, error_types);
        }
        Expression::Match(m) => {
            for arm in &m.arms {
                collect_error_types_from_block(&arm.body, error_types);
            }
            if let Some(default) = &m.default {
                collect_error_types_from_block(default, error_types);
            }
        }
        _ => {}
    }
}

fn collect_handled_error_types_from_block(block: &Block, handled: &mut Vec<Vec<String>>) {
    for statement in &block.statements {
        match statement {
            Statement::Expression(expr) => {
                collect_handled_error_types_from_expr(expr, handled);
            }
            Statement::Binding(binding) => {
                collect_handled_error_types_from_expr(&binding.value, handled);
            }
            Statement::If {
                condition,
                body,
                elif_branches,
                else_body,
                ..
            } => {
                collect_handled_error_types_from_expr(condition, handled);
                collect_handled_error_types_from_block(body, handled);
                for (cond, blk) in elif_branches {
                    collect_handled_error_types_from_expr(cond, handled);
                    collect_handled_error_types_from_block(blk, handled);
                }
                if let Some(else_blk) = else_body {
                    collect_handled_error_types_from_block(else_blk, handled);
                }
            }
            Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::ForKV { body, .. } => {
                collect_handled_error_types_from_block(body, handled);
            }
            Statement::ForRange { body, .. } => {
                collect_handled_error_types_from_block(body, handled);
            }
            Statement::Return(expr, _) => {
                collect_handled_error_types_from_expr(expr, handled);
            }
            _ => {}
        }
    }
}

fn collect_handled_error_types_from_expr(expr: &Expression, handled: &mut Vec<Vec<String>>) {
    match expr {
        Expression::Match(m) => {
            for arm in &m.arms {
                match &arm.pattern {
                    MatchPattern::Constructor { name, .. } => {
                        if name == "Err" || name.starts_with("Error") {
                            handled.push(vec![name.clone()]);
                        }
                    }
                    MatchPattern::Identifier(name) => {
                        handled.push(vec![name.clone()]);
                    }
                    MatchPattern::Wildcard(_) => {
                        handled.push(vec!["*".to_string()]);
                    }
                    _ => {}
                }
                collect_handled_error_types_from_block(&arm.body, handled);
            }
            if m.default.is_some() {
                handled.push(vec!["*".to_string()]);
            }
            if let Some(default) = &m.default {
                collect_handled_error_types_from_block(default, handled);
            }
        }
        Expression::Lambda { body, .. } => {
            collect_handled_error_types_from_block(body, handled);
        }
        Expression::ErrorPropagate { .. } => {}
        _ => {}
    }
}

fn check_error_type_exhaustiveness(functions: &[Function], warnings: &mut Vec<Diagnostic>) {
    for function in functions {
        let mut produced_errors: Vec<(Vec<String>, Span)> = Vec::new();
        collect_error_types_from_block(&function.body, &mut produced_errors);

        if produced_errors.is_empty() {
            continue;
        }

        let mut seen = std::collections::HashSet::new();
        produced_errors.retain(|(path, _)| seen.insert(path.clone()));

        let mut handled_errors: Vec<Vec<String>> = Vec::new();
        collect_handled_error_types_from_block(&function.body, &mut handled_errors);

        for (error_path, span) in &produced_errors {
            let is_handled = handled_errors
                .iter()
                .any(|handled| error_path == handled || error_path.starts_with(handled.as_slice()));

            if !is_handled {
                let in_return = function.body.statements.iter().any(|s| {
                    if let Statement::Return(expr, _) = s {
                        contains_error_path(expr, error_path)
                    } else {
                        false
                    }
                });
                if !in_return {
                    warnings.push(
                        Diagnostic::new(
                            format!(
                                "error type `err {}` may not be exhaustively handled",
                                error_path.join(":")
                            ),
                            *span,
                        )
                        .with_help("consider matching on error types or propagating with `!`"),
                    );
                }
            }
        }
    }
}

fn contains_error_path(expr: &Expression, target: &[String]) -> bool {
    match expr {
        Expression::ErrorValue { path, .. } => path == target,
        Expression::Ternary {
            then_branch,
            else_branch,
            ..
        } => contains_error_path(then_branch, target) || contains_error_path(else_branch, target),
        _ => false,
    }
}

fn statement_span(stmt: &Statement) -> Span {
    match stmt {
        Statement::Binding(b) => b.value.span(),
        Statement::Expression(e) => e.span(),
        Statement::Return(_, span) => *span,
        Statement::If { span, .. } => *span,
        Statement::While { span, .. } => *span,
        Statement::For { span, .. } => *span,
        Statement::ForKV { span, .. } => *span,
        Statement::ForRange { span, .. } => *span,
        Statement::FieldAssign { span, .. } => *span,
        Statement::Break(span) => *span,
        Statement::Continue(span) => *span,
        Statement::PatternBinding { span, .. } => *span,
    }
}

fn check_branch_type_consistency(
    branch_type_hints: &[(Vec<TypeId>, Span)],
    graph: &mut TypeGraph,
    warnings: &mut Vec<Diagnostic>,
) {
    use crate::types::resolve;
    for (branch_tys, span) in branch_type_hints {
        if branch_tys.len() < 2 {
            continue;
        }

        let resolved: Vec<TypeId> = branch_tys
            .iter()
            .map(|ty| resolve(ty.clone(), graph))
            .collect();

        let first = &resolved[0];
        let mut mismatch = false;
        for ty in &resolved[1..] {
            if !types_compatible_for_branch(first, ty) {
                mismatch = true;
                break;
            }
        }

        if mismatch {
            let type_names: Vec<String> = resolved.iter().map(|t| format!("{:?}", t)).collect();
            warnings.push(
                Diagnostic::categorized_warning(
                    format!(
                        "if/else branches return different types: {}",
                        type_names.join(" vs ")
                    ),
                    *span,
                    WarningCategory::TypeMismatchBranch,
                )
                .with_help("all branches should return the same type for consistency"),
            );
        }
    }
}

fn types_compatible_for_branch(a: &TypeId, b: &TypeId) -> bool {
    use crate::types::core::Primitive;
    match (a, b) {
        (TypeId::Unknown, _) | (_, TypeId::Unknown) => true,
        (TypeId::Primitive(Primitive::Any), _) | (_, TypeId::Primitive(Primitive::Any)) => true,
        (TypeId::Primitive(Primitive::None), _) | (_, TypeId::Primitive(Primitive::None)) => true,
        (TypeId::TypeVar(_), _) | (_, TypeId::TypeVar(_)) => true,

        (TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::Float)) => true,
        (TypeId::Primitive(Primitive::Float), TypeId::Primitive(Primitive::Int)) => true,

        _ => a == b,
    }
}

fn check_nullability_returns(functions: &[Function], warnings: &mut Vec<Diagnostic>) {
    for function in functions {
        if function.name == "main" {
            continue;
        }
        let mut has_none_return = false;
        let mut has_value_return = false;
        let mut none_span: Option<Span> = None;

        if let Some(ref val) = function.body.value {
            if expr_is_none(val) {
                has_none_return = true;
                none_span = Some(val.span());
            } else {
                has_value_return = true;
            }
        }

        collect_return_nullability(
            &function.body,
            &mut has_none_return,
            &mut has_value_return,
            &mut none_span,
        );

        if has_none_return && has_value_return {
            let span = none_span.unwrap_or(function.span);
            warnings.push(
                Diagnostic::categorized_warning(
                    format!(
                        "function '{}' may return 'none' on some paths",
                        function.name
                    ),
                    span,
                    WarningCategory::Nullability,
                )
                .with_help("consider returning an explicit default value or using an Option type"),
            );
        }
    }
}

fn collect_return_nullability(
    block: &Block,
    has_none: &mut bool,
    has_value: &mut bool,
    none_span: &mut Option<Span>,
) {
    for stmt in &block.statements {
        match stmt {
            crate::ast::Statement::Return(expr, span) => {
                if expr_is_none(expr) {
                    *has_none = true;
                    if none_span.is_none() {
                        *none_span = Some(*span);
                    }
                } else {
                    *has_value = true;
                }
            }
            crate::ast::Statement::If {
                body,
                elif_branches,
                else_body,
                ..
            } => {
                collect_return_nullability(body, has_none, has_value, none_span);
                for (_, blk) in elif_branches {
                    collect_return_nullability(blk, has_none, has_value, none_span);
                }
                if let Some(eb) = else_body {
                    collect_return_nullability(eb, has_none, has_value, none_span);
                }
            }
            crate::ast::Statement::While { body, .. } => {
                collect_return_nullability(body, has_none, has_value, none_span);
            }
            crate::ast::Statement::For { body, .. } => {
                collect_return_nullability(body, has_none, has_value, none_span);
            }
            crate::ast::Statement::ForRange { body, .. } => {
                collect_return_nullability(body, has_none, has_value, none_span);
            }
            _ => {}
        }
    }
}

fn expr_is_none(expr: &Expression) -> bool {
    matches!(expr, Expression::None(_))
}

fn check_definite_assignment(functions: &[Function], warnings: &mut Vec<Diagnostic>) {
    for function in functions {
        let mut definitely_assigned: HashSet<String> = HashSet::new();

        for param in &function.params {
            definitely_assigned.insert(param.name.clone());
        }
        da_check_block(&function.body, &mut definitely_assigned, warnings);
    }
}

fn da_check_block(block: &Block, assigned: &mut HashSet<String>, warnings: &mut Vec<Diagnostic>) {
    for stmt in &block.statements {
        da_check_statement(stmt, assigned, warnings);
    }
    if let Some(ref val) = block.value {
        da_check_expression_uses(val, assigned, warnings);
    }
}

fn da_check_statement(
    stmt: &Statement,
    assigned: &mut HashSet<String>,
    warnings: &mut Vec<Diagnostic>,
) {
    match stmt {
        Statement::Binding(binding) => {
            da_check_expression_uses(&binding.value, assigned, warnings);

            assigned.insert(binding.name.clone());
        }
        Statement::Expression(expr) => {
            da_check_expression_uses(expr, assigned, warnings);
        }
        Statement::Return(expr, _) => {
            da_check_expression_uses(expr, assigned, warnings);
        }
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => {
            da_check_expression_uses(condition, assigned, warnings);

            let mut then_assigned = assigned.clone();
            da_check_block(body, &mut then_assigned, warnings);

            let mut all_branches_assigned = Vec::new();
            all_branches_assigned.push(then_assigned);

            for (elif_cond, elif_body) in elif_branches {
                da_check_expression_uses(elif_cond, assigned, warnings);
                let mut branch_assigned = assigned.clone();
                da_check_block(elif_body, &mut branch_assigned, warnings);
                all_branches_assigned.push(branch_assigned);
            }

            if let Some(else_body) = else_body {
                let mut else_assigned = assigned.clone();
                da_check_block(else_body, &mut else_assigned, warnings);
                all_branches_assigned.push(else_assigned);

                if !all_branches_assigned.is_empty() {
                    let intersection: HashSet<String> = all_branches_assigned[0]
                        .iter()
                        .filter(|name| all_branches_assigned.iter().all(|s| s.contains(*name)))
                        .cloned()
                        .collect();
                    *assigned = intersection;
                }
            }
        }
        Statement::For {
            variable,
            iterable,
            body,
            ..
        } => {
            da_check_expression_uses(iterable, assigned, warnings);
            let mut loop_assigned = assigned.clone();
            loop_assigned.insert(variable.clone());
            da_check_block(body, &mut loop_assigned, warnings);
        }
        Statement::ForRange {
            variable,
            start,
            end,
            step,
            body,
            ..
        } => {
            da_check_expression_uses(start, assigned, warnings);
            da_check_expression_uses(end, assigned, warnings);
            if let Some(s) = step {
                da_check_expression_uses(s, assigned, warnings);
            }
            let mut loop_assigned = assigned.clone();
            loop_assigned.insert(variable.clone());
            da_check_block(body, &mut loop_assigned, warnings);
        }
        Statement::ForKV {
            key_var,
            value_var,
            iterable,
            body,
            ..
        } => {
            da_check_expression_uses(iterable, assigned, warnings);
            let mut loop_assigned = assigned.clone();
            loop_assigned.insert(key_var.clone());
            loop_assigned.insert(value_var.clone());
            da_check_block(body, &mut loop_assigned, warnings);
        }
        Statement::While {
            condition, body, ..
        } => {
            da_check_expression_uses(condition, assigned, warnings);
            let mut loop_assigned = assigned.clone();
            da_check_block(body, &mut loop_assigned, warnings);
        }
        Statement::FieldAssign { target, value, .. } => {
            da_check_expression_uses(target, assigned, warnings);
            da_check_expression_uses(value, assigned, warnings);
        }
        Statement::PatternBinding { value, .. } => {
            da_check_expression_uses(value, assigned, warnings);
        }
        Statement::Break(_) | Statement::Continue(_) => {}
    }
}

fn da_check_expression_uses(
    expr: &Expression,
    assigned: &HashSet<String>,
    warnings: &mut Vec<Diagnostic>,
) {
    match expr {
        Expression::Identifier(name, span) => {
            if !assigned.contains(name)
                && !is_builtin_name(name)
                && !name.is_empty()
                && name.chars().next().map_or(false, |c| c.is_lowercase())
            {
                warnings.push(
                    Diagnostic::categorized_warning(
                        format!("variable `{}` may not be initialized on all paths", name),
                        *span,
                        WarningCategory::UnusedVariable,
                    )
                    .with_help("ensure the variable is assigned before use, or add an else branch"),
                );
            }
        }
        Expression::Binary { left, right, .. } => {
            da_check_expression_uses(left, assigned, warnings);
            da_check_expression_uses(right, assigned, warnings);
        }
        Expression::Unary { expr: inner, .. } => {
            da_check_expression_uses(inner, assigned, warnings);
        }
        Expression::Call { callee, args, .. } => {
            da_check_expression_uses(callee, assigned, warnings);
            for arg in args {
                da_check_expression_uses(arg, assigned, warnings);
            }
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            da_check_expression_uses(condition, assigned, warnings);
            da_check_expression_uses(then_branch, assigned, warnings);
            da_check_expression_uses(else_branch, assigned, warnings);
        }
        Expression::Member { target, .. } => {
            da_check_expression_uses(target, assigned, warnings);
        }
        Expression::List(items, _) => {
            for item in items {
                da_check_expression_uses(item, assigned, warnings);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                da_check_expression_uses(k, assigned, warnings);
                da_check_expression_uses(v, assigned, warnings);
            }
        }
        Expression::Lambda { .. } => {}
        _ => {}
    }
}

fn check_dead_code(functions: &[Function], warnings: &mut Vec<Diagnostic>) {
    for function in functions {
        check_block_for_dead_code(&function.body, warnings);
    }
}

fn check_block_for_dead_code(block: &Block, warnings: &mut Vec<Diagnostic>) {
    let mut terminated = false;
    let mut terminator_kind: &str = "";

    for statement in &block.statements {
        if terminated {
            warnings.push(
                Diagnostic::categorized_warning(
                    format!("unreachable code after {}", terminator_kind),
                    statement_span(statement),
                    WarningCategory::UnreachableCode,
                )
                .with_help("consider removing the unreachable statements"),
            );

            break;
        }

        match statement {
            Statement::Return(_, _) => {
                terminated = true;
                terminator_kind = "return";
            }
            Statement::Break(_) => {
                terminated = true;
                terminator_kind = "break";
            }
            Statement::Continue(_) => {
                terminated = true;
                terminator_kind = "continue";
            }
            _ => {}
        }

        check_statement_nested_blocks_for_dead_code(statement, warnings);
    }
}

fn check_statement_nested_blocks_for_dead_code(
    statement: &Statement,
    warnings: &mut Vec<Diagnostic>,
) {
    match statement {
        Statement::If {
            body,
            elif_branches,
            else_body,
            ..
        } => {
            check_block_for_dead_code(body, warnings);
            for (_, blk) in elif_branches {
                check_block_for_dead_code(blk, warnings);
            }
            if let Some(else_blk) = else_body {
                check_block_for_dead_code(else_blk, warnings);
            }
        }
        Statement::While { body, .. }
        | Statement::For { body, .. }
        | Statement::ForKV { body, .. }
        | Statement::ForRange { body, .. } => {
            check_block_for_dead_code(body, warnings);
        }
        Statement::Expression(expr) => {
            check_expr_for_dead_code(expr, warnings);
        }
        Statement::Binding(binding) => {
            check_expr_for_dead_code(&binding.value, warnings);
        }
        _ => {}
    }
}

fn check_expr_for_dead_code(expr: &Expression, warnings: &mut Vec<Diagnostic>) {
    match expr {
        Expression::Match(match_expr) => {
            for arm in &match_expr.arms {
                check_block_for_dead_code(&arm.body, warnings);
            }
        }
        Expression::Lambda { body, .. } => {
            check_block_for_dead_code(body, warnings);
        }
        _ => {}
    }
}

fn check_member_access_validity(
    globals: &[Binding],
    functions: &[Function],
    store_field_names: &HashMap<String, Vec<String>>,
    warnings: &mut Vec<Diagnostic>,
) {
    static UNIVERSAL_MEMBERS: &[&str] = &[
        "length",
        "count",
        "size",
        "err",
        "push",
        "pop",
        "get",
        "set",
        "append",
        "remove",
        "insert",
        "contains",
        "keys",
        "values",
        "clear",
        "join",
        "map",
        "filter",
        "reduce",
        "find",
        "any",
        "all",
        "sort",
        "equals",
        "not_equals",
        "not",
        "iter",
        "to_string",
        "type",
        "trim",
        "split",
        "starts_with",
        "ends_with",
        "replace",
        "to_upper",
        "to_lower",
        "chars",
        "bytes",
        "slice",
        "reverse",
        "flat_map",
        "enumerate",
        "zip",
        "take",
        "skip",
        "head",
        "tail",
        "last",
        "is_empty",
        "has_key",
        "entries",
    ];

    let mut var_types: HashMap<String, String> = HashMap::new();

    for binding in globals {
        if let Some(store_name) = extract_store_type(&binding.value, store_field_names) {
            var_types.insert(binding.name.clone(), store_name);
        }
        check_member_access_in_expr(
            &binding.value,
            store_field_names,
            UNIVERSAL_MEMBERS,
            &var_types,
            warnings,
        );
    }
    for func in functions {
        let mut local_var_types = var_types.clone();
        for stmt in &func.body.statements {
            collect_store_bindings(stmt, store_field_names, &mut local_var_types);
            check_member_access_in_statement(
                stmt,
                store_field_names,
                UNIVERSAL_MEMBERS,
                &local_var_types,
                warnings,
            );
        }
        if let Some(ref val_expr) = func.body.value {
            check_member_access_in_expr(
                val_expr,
                store_field_names,
                UNIVERSAL_MEMBERS,
                &local_var_types,
                warnings,
            );
        }
    }
}

fn extract_store_type(
    expr: &Expression,
    store_field_names: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if let Expression::Call { callee, .. } = expr {
        if let Expression::Identifier(name, _) = callee.as_ref() {
            let type_name = name.strip_prefix("make_")?;
            if store_field_names.contains_key(type_name) {
                return Some(type_name.to_string());
            }
        }
    }
    None
}

fn collect_store_bindings(
    stmt: &Statement,
    store_field_names: &HashMap<String, Vec<String>>,
    var_types: &mut HashMap<String, String>,
) {
    if let Statement::Binding(binding) = stmt {
        if let Some(store_name) = extract_store_type(&binding.value, store_field_names) {
            var_types.insert(binding.name.clone(), store_name);
        }
    }
}

fn check_member_access_in_statement(
    stmt: &Statement,
    store_field_names: &HashMap<String, Vec<String>>,
    universal: &[&str],
    var_types: &HashMap<String, String>,
    warnings: &mut Vec<Diagnostic>,
) {
    match stmt {
        Statement::Expression(expr) => {
            check_member_access_in_expr(expr, store_field_names, universal, var_types, warnings);
        }
        Statement::Binding(binding) => {
            check_member_access_in_expr(
                &binding.value,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
        }
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => {
            check_member_access_in_expr(
                condition,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
            for s in &body.statements {
                check_member_access_in_statement(
                    s,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
            }
            if let Some(ref v) = body.value {
                check_member_access_in_expr(v, store_field_names, universal, var_types, warnings);
            }
            for (cond, block) in elif_branches {
                check_member_access_in_expr(
                    cond,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
                for s in &block.statements {
                    check_member_access_in_statement(
                        s,
                        store_field_names,
                        universal,
                        var_types,
                        warnings,
                    );
                }
                if let Some(ref v) = block.value {
                    check_member_access_in_expr(
                        v,
                        store_field_names,
                        universal,
                        var_types,
                        warnings,
                    );
                }
            }
            if let Some(block) = else_body {
                for s in &block.statements {
                    check_member_access_in_statement(
                        s,
                        store_field_names,
                        universal,
                        var_types,
                        warnings,
                    );
                }
                if let Some(ref v) = block.value {
                    check_member_access_in_expr(
                        v,
                        store_field_names,
                        universal,
                        var_types,
                        warnings,
                    );
                }
            }
        }
        Statement::Return(expr, _) => {
            check_member_access_in_expr(expr, store_field_names, universal, var_types, warnings);
        }
        Statement::While {
            condition, body, ..
        } => {
            check_member_access_in_expr(
                condition,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
            for s in &body.statements {
                check_member_access_in_statement(
                    s,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
            }
        }
        Statement::For { iterable, body, .. } | Statement::ForKV { iterable, body, .. } => {
            check_member_access_in_expr(
                iterable,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
            for s in &body.statements {
                check_member_access_in_statement(
                    s,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
            }
        }
        Statement::ForRange {
            start,
            end,
            step,
            body,
            ..
        } => {
            check_member_access_in_expr(start, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(end, store_field_names, universal, var_types, warnings);
            if let Some(s) = step {
                check_member_access_in_expr(s, store_field_names, universal, var_types, warnings);
            }
            for s in &body.statements {
                check_member_access_in_statement(
                    s,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
            }
        }
        Statement::FieldAssign { target, value, .. } => {
            check_member_access_in_expr(target, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(value, store_field_names, universal, var_types, warnings);
        }
        Statement::PatternBinding { value, .. } => {
            check_member_access_in_expr(value, store_field_names, universal, var_types, warnings);
        }
        Statement::Break(_) | Statement::Continue(_) => {}
    }
}

fn check_member_access_in_expr(
    expr: &Expression,
    store_field_names: &HashMap<String, Vec<String>>,
    universal: &[&str],
    var_types: &HashMap<String, String>,
    warnings: &mut Vec<Diagnostic>,
) {
    match expr {
        Expression::Member {
            target,
            property,
            span,
        } => {
            check_member_access_in_expr(target, store_field_names, universal, var_types, warnings);

            let store_type = match target.as_ref() {
                Expression::Call { callee, .. } => {
                    if let Expression::Identifier(name, _) = callee.as_ref() {
                        name.strip_prefix("make_")
                            .and_then(|t| store_field_names.get(t).map(|_| t.to_string()))
                    } else {
                        None
                    }
                }

                Expression::Identifier(var_name, _) => var_types.get(var_name.as_str()).cloned(),
                _ => None,
            };

            if let Some(type_name) = store_type {
                if let Some(fields) = store_field_names.get(&type_name) {
                    if !fields.contains(property) && !universal.contains(&property.as_str()) {
                        warnings.push(Diagnostic::categorized_warning(
                            format!(
                                "type `{}` has no field `{}`; known fields: {}",
                                type_name,
                                property,
                                fields.join(", ")
                            ),
                            *span,
                            WarningCategory::General,
                        ));
                    }
                }
            }
        }
        Expression::Call { callee, args, .. } => {
            check_member_access_in_expr(callee, store_field_names, universal, var_types, warnings);
            for arg in args {
                check_member_access_in_expr(arg, store_field_names, universal, var_types, warnings);
            }
        }
        Expression::Binary { left, right, .. } => {
            check_member_access_in_expr(left, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(right, store_field_names, universal, var_types, warnings);
        }
        Expression::Unary { expr, .. } => {
            check_member_access_in_expr(expr, store_field_names, universal, var_types, warnings);
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            check_member_access_in_expr(
                condition,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
            check_member_access_in_expr(
                then_branch,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
            check_member_access_in_expr(
                else_branch,
                store_field_names,
                universal,
                var_types,
                warnings,
            );
        }
        Expression::Pipeline { left, right, .. } => {
            check_member_access_in_expr(left, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(right, store_field_names, universal, var_types, warnings);
        }
        Expression::List(items, _) => {
            for item in items {
                check_member_access_in_expr(
                    item,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                check_member_access_in_expr(k, store_field_names, universal, var_types, warnings);
                check_member_access_in_expr(v, store_field_names, universal, var_types, warnings);
            }
        }
        Expression::Index { target, index, .. } => {
            check_member_access_in_expr(target, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(index, store_field_names, universal, var_types, warnings);
        }
        Expression::Lambda { body, .. } => {
            for s in &body.statements {
                check_member_access_in_statement(
                    s,
                    store_field_names,
                    universal,
                    var_types,
                    warnings,
                );
            }
            if let Some(ref v) = body.value {
                check_member_access_in_expr(v, store_field_names, universal, var_types, warnings);
            }
        }
        _ => {}
    }
}

fn validate_typed_actor_sends(
    functions: &[Function],
    globals: &[crate::ast::Binding],
    actor_handler_names: &HashMap<String, Vec<String>>,
    actor_message_types: &HashMap<String, String>,
    warnings: &mut Vec<Diagnostic>,
) {
    if actor_message_types.is_empty() {
        return;
    }

    let mut var_types: HashMap<String, String> = HashMap::new();
    for global in globals {
        if let Some(atype) = trace_actor_type_from_expr(&global.value) {
            var_types.insert(global.name.clone(), atype);
        }
        walk_typed_send_expr(
            &global.value,
            actor_handler_names,
            actor_message_types,
            &var_types,
            warnings,
        );
    }
    for func in functions {
        let mut local_vars = var_types.clone();
        walk_typed_send_block(
            &func.body,
            actor_handler_names,
            actor_message_types,
            &mut local_vars,
            warnings,
        );
    }
}

fn walk_typed_send_block(
    block: &crate::ast::Block,
    h: &HashMap<String, Vec<String>>,
    m: &HashMap<String, String>,
    vars: &mut HashMap<String, String>,
    w: &mut Vec<Diagnostic>,
) {
    for stmt in &block.statements {
        match stmt {
            crate::ast::Statement::Expression(e) => walk_typed_send_expr(e, h, m, vars, w),
            crate::ast::Statement::Binding(b) => {
                if let Some(atype) = trace_actor_type_from_expr(&b.value) {
                    vars.insert(b.name.clone(), atype);
                }
                walk_typed_send_expr(&b.value, h, m, vars, w);
            }
            crate::ast::Statement::If {
                condition,
                body,
                else_body,
                ..
            } => {
                walk_typed_send_expr(condition, h, m, vars, w);
                walk_typed_send_block(body, h, m, vars, w);
                if let Some(eb) = else_body {
                    walk_typed_send_block(eb, h, m, vars, w);
                }
            }
            crate::ast::Statement::While {
                condition, body, ..
            } => {
                walk_typed_send_expr(condition, h, m, vars, w);
                walk_typed_send_block(body, h, m, vars, w);
            }
            crate::ast::Statement::For { iterable, body, .. } => {
                walk_typed_send_expr(iterable, h, m, vars, w);
                walk_typed_send_block(body, h, m, vars, w);
            }
            _ => {}
        }
    }
    if let Some(ref v) = block.value {
        walk_typed_send_expr(v, h, m, vars, w);
    }
}

fn walk_typed_send_expr(
    expr: &Expression,
    h: &HashMap<String, Vec<String>>,
    m: &HashMap<String, String>,
    vars: &HashMap<String, String>,
    w: &mut Vec<Diagnostic>,
) {
    match expr {
        Expression::Call {
            callee, args, span, ..
        } => {
            if let Expression::Identifier(name, _) = callee.as_ref() {
                if name == "actor_send" && args.len() >= 2 {
                    if let Expression::String(handler, _) = &args[1] {
                        let actor_name = trace_actor_type_from_expr(&args[0]).or_else(|| {
                            if let Expression::Identifier(var, _) = &args[0] {
                                vars.get(var).cloned()
                            } else {
                                None
                            }
                        });
                        if let Some(actor_name) = actor_name {
                            if m.contains_key(&actor_name) {
                                if let Some(known) = h.get(&actor_name) {
                                    if !known.contains(handler) {
                                        w.push(Diagnostic::new(
                                            format!(
                                                "actor `{}` has @messages annotation but no handler `@{}`. Known handlers: {}",
                                                actor_name, handler,
                                                if known.is_empty() { "(none)".to_string() } else { known.join(", ") }
                                            ),
                                            *span,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            walk_typed_send_expr(callee, h, m, vars, w);
            for arg in args {
                walk_typed_send_expr(arg, h, m, vars, w);
            }
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            walk_typed_send_expr(condition, h, m, vars, w);
            walk_typed_send_expr(then_branch, h, m, vars, w);
            walk_typed_send_expr(else_branch, h, m, vars, w);
        }
        Expression::Lambda { body, .. } => {
            let mut inner_vars = vars.clone();
            walk_typed_send_block(body, h, m, &mut inner_vars, w);
        }
        _ => {}
    }
}

fn trace_actor_type_from_expr(expr: &Expression) -> Option<String> {
    if let Expression::Call { callee, .. } = expr {
        if let Expression::Identifier(name, _) = callee.as_ref() {
            if name.starts_with("make_") {
                return Some(name[5..].to_string());
            }
        }
    }
    None
}

/// Walk all functions to collect per-variable and per-parameter resolved types.
/// Returns (resolved_locals, resolved_params).
fn collect_resolved_variable_types(
    functions: &[Function],
    resolved: &TypeEnv,
) -> (
    HashMap<(String, String), TypeId>,
    HashMap<(String, usize), TypeId>,
    HashMap<String, TypeId>,
) {
    let mut resolved_locals: HashMap<(String, String), TypeId> = HashMap::new();
    let mut resolved_params: HashMap<(String, usize), TypeId> = HashMap::new();
    let mut resolved_returns: HashMap<String, TypeId> = HashMap::new();

    for function in functions {
        // Collect parameter types
        for (idx, param) in function.params.iter().enumerate() {
            if let Some(ty) = resolved.get(&param.name) {
                if is_specializable_type(ty) {
                    resolved_params.insert((function.name.clone(), idx), ty.clone());
                    // Also store by name in resolved_locals for easier lookup in codegen
                    resolved_locals.insert((function.name.clone(), param.name.clone()), ty.clone());
                }
            }
        }
        // Collect function return type from resolved Func type
        if let Some(TypeId::Func(_, ret_ty)) = resolved.get(&function.name) {
            if is_specializable_type(ret_ty) {
                resolved_returns.insert(function.name.clone(), *ret_ty.clone());
            }
        }
        // Walk body to collect local binding types
        collect_block_variable_types(&function.name, &function.body, resolved, &mut resolved_locals);
    }

    (resolved_locals, resolved_params, resolved_returns)
}

fn is_specializable_type(ty: &TypeId) -> bool {
    matches!(
        ty,
        TypeId::Primitive(Primitive::Int)
            | TypeId::Primitive(Primitive::Float)
            | TypeId::Primitive(Primitive::Bool)
            | TypeId::Store(_)
            | TypeId::List(_)
    )
}

fn collect_block_variable_types(
    fn_name: &str,
    block: &Block,
    resolved: &TypeEnv,
    locals: &mut HashMap<(String, String), TypeId>,
) {
    for stmt in &block.statements {
        match stmt {
            Statement::Binding(binding) => {
                if let Some(ty) = resolved.get(&binding.name) {
                    if is_specializable_type(ty) {
                        locals.insert(
                            (fn_name.to_string(), binding.name.clone()),
                            ty.clone(),
                        );
                    }
                }
            }
            Statement::If {
                body,
                elif_branches,
                else_body,
                ..
            } => {
                collect_block_variable_types(fn_name, body, resolved, locals);
                for (_, blk) in elif_branches {
                    collect_block_variable_types(fn_name, blk, resolved, locals);
                }
                if let Some(eb) = else_body {
                    collect_block_variable_types(fn_name, eb, resolved, locals);
                }
            }
            Statement::While { body, .. } => {
                collect_block_variable_types(fn_name, body, resolved, locals);
            }
            Statement::For {
                variable, body, ..
            } => {
                if let Some(ty) = resolved.get(variable) {
                    if is_specializable_type(ty) {
                        locals.insert((fn_name.to_string(), variable.clone()), ty.clone());
                    }
                }
                collect_block_variable_types(fn_name, body, resolved, locals);
            }
            Statement::ForRange {
                variable, body, ..
            } => {
                // ForRange loop variables are always Int
                locals.insert(
                    (fn_name.to_string(), variable.clone()),
                    TypeId::Primitive(Primitive::Int),
                );
                collect_block_variable_types(fn_name, body, resolved, locals);
            }
            Statement::ForKV {
                key_var,
                value_var,
                body,
                ..
            } => {
                if let Some(ty) = resolved.get(key_var) {
                    if is_specializable_type(ty) {
                        locals.insert((fn_name.to_string(), key_var.clone()), ty.clone());
                    }
                }
                if let Some(ty) = resolved.get(value_var) {
                    if is_specializable_type(ty) {
                        locals.insert((fn_name.to_string(), value_var.clone()), ty.clone());
                    }
                }
                collect_block_variable_types(fn_name, body, resolved, locals);
            }
            _ => {}
        }
    }
}

// ─── Monomorphization: Call-Site Type Profile Collection ──────────────────────

/// Infer the static type of an expression from the resolved TypeEnv.
fn infer_expr_type(expr: &Expression, resolved: &TypeEnv) -> TypeId {
    match expr {
        Expression::Integer(_, _) => TypeId::Primitive(Primitive::Int),
        Expression::Float(_, _) => TypeId::Primitive(Primitive::Float),
        Expression::Bool(_, _) => TypeId::Primitive(Primitive::Bool),
        Expression::String(_, _) => TypeId::Primitive(Primitive::String),
        Expression::Bytes(_, _) => TypeId::Primitive(Primitive::Bytes),
        Expression::Identifier(name, _) => {
            resolved.get(name).cloned().unwrap_or(TypeId::Unknown)
        }
        Expression::Call { callee, .. } => {
            if let Expression::Identifier(name, _) = callee.as_ref() {
                if let Some(TypeId::Func(_, ret)) = resolved.get(name) {
                    return *ret.clone();
                }
            }
            TypeId::Unknown
        }
        Expression::Binary { op, left, right, .. } => {
            let lt = infer_expr_type(left, resolved);
            let rt = infer_expr_type(right, resolved);
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                | BinaryOp::Div | BinaryOp::Mod => {
                    if lt == TypeId::Primitive(Primitive::Int)
                        && rt == TypeId::Primitive(Primitive::Int)
                    {
                        TypeId::Primitive(Primitive::Int)
                    } else if lt.is_numeric() && rt.is_numeric() {
                        TypeId::Primitive(Primitive::Float)
                    } else {
                        TypeId::Unknown
                    }
                }
                BinaryOp::Equals | BinaryOp::NotEquals | BinaryOp::Less
                | BinaryOp::LessEq | BinaryOp::Greater | BinaryOp::GreaterEq
                | BinaryOp::And | BinaryOp::Or => TypeId::Primitive(Primitive::Bool),
                _ => TypeId::Unknown,
            }
        }
        Expression::Unary { op, expr: inner, .. } => match op {
            UnaryOp::Neg => infer_expr_type(inner, resolved),
            UnaryOp::Not => TypeId::Primitive(Primitive::Bool),
            _ => TypeId::Unknown,
        },
        Expression::Ternary {
            then_branch,
            else_branch,
            ..
        } => {
            let t = infer_expr_type(then_branch, resolved);
            let e = infer_expr_type(else_branch, resolved);
            if t == e { t } else { TypeId::Unknown }
        }
        _ => TypeId::Unknown,
    }
}

/// Walk an expression tree and collect type profiles for all call sites.
fn collect_call_profiles_expr(
    expr: &Expression,
    resolved: &TypeEnv,
    profiles: &mut HashMap<String, HashMap<Vec<TypeId>, usize>>,
) {
    match expr {
        Expression::Call {
            callee, args, ..
        } => {
            // Collect profiles for the callee target
            if let Expression::Identifier(name, _) = callee.as_ref() {
                let arg_types: Vec<TypeId> =
                    args.iter().map(|arg| infer_expr_type(arg, resolved)).collect();
                if arg_types.iter().all(|t| is_specializable_type(t)) && !arg_types.is_empty() {
                    *profiles
                        .entry(name.clone())
                        .or_default()
                        .entry(arg_types)
                        .or_default() += 1;
                }
            }
            // Recurse into callee and args
            collect_call_profiles_expr(callee, resolved, profiles);
            for arg in args {
                collect_call_profiles_expr(arg, resolved, profiles);
            }
        }
        Expression::Binary { left, right, .. } => {
            collect_call_profiles_expr(left, resolved, profiles);
            collect_call_profiles_expr(right, resolved, profiles);
        }
        Expression::Unary { expr: inner, .. } => {
            collect_call_profiles_expr(inner, resolved, profiles);
        }
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_call_profiles_expr(condition, resolved, profiles);
            collect_call_profiles_expr(then_branch, resolved, profiles);
            collect_call_profiles_expr(else_branch, resolved, profiles);
        }
        Expression::List(items, _) => {
            for item in items {
                collect_call_profiles_expr(item, resolved, profiles);
            }
        }
        Expression::Map(entries, _) => {
            for (k, v) in entries {
                collect_call_profiles_expr(k, resolved, profiles);
                collect_call_profiles_expr(v, resolved, profiles);
            }
        }
        Expression::Index { target, index, .. } => {
            collect_call_profiles_expr(target, resolved, profiles);
            collect_call_profiles_expr(index, resolved, profiles);
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            collect_call_profiles_expr(target, resolved, profiles);
            collect_call_profiles_expr(start, resolved, profiles);
            collect_call_profiles_expr(end, resolved, profiles);
        }
        Expression::Member { target, .. } => {
            collect_call_profiles_expr(target, resolved, profiles);
        }
        Expression::Lambda { body, .. } => {
            collect_call_profiles_block(body, resolved, profiles);
        }
        Expression::Match(m) => {
            collect_call_profiles_expr(&m.value, resolved, profiles);
            for arm in &m.arms {
                if let Some(guard) = &arm.guard {
                    collect_call_profiles_expr(guard, resolved, profiles);
                }
                collect_call_profiles_block(&arm.body, resolved, profiles);
            }
            if let Some(ref def) = m.default {
                collect_call_profiles_block(def, resolved, profiles);
            }
        }
        Expression::Pipeline { left, right, .. } => {
            collect_call_profiles_expr(left, resolved, profiles);
            collect_call_profiles_expr(right, resolved, profiles);
        }
        Expression::ErrorPropagate { expr: inner, .. }
        | Expression::Throw { value: inner, .. } => {
            collect_call_profiles_expr(inner, resolved, profiles);
        }
        Expression::Unsafe { block, .. } => {
            collect_call_profiles_block(block, resolved, profiles);
        }
        _ => {}
    }
}

/// Walk a block and collect call-site type profiles.
fn collect_call_profiles_block(
    block: &Block,
    resolved: &TypeEnv,
    profiles: &mut HashMap<String, HashMap<Vec<TypeId>, usize>>,
) {
    for stmt in &block.statements {
        collect_call_profiles_stmt(stmt, resolved, profiles);
    }
    if let Some(val) = &block.value {
        collect_call_profiles_expr(val, resolved, profiles);
    }
}

/// Walk a statement and collect call-site type profiles.
fn collect_call_profiles_stmt(
    stmt: &Statement,
    resolved: &TypeEnv,
    profiles: &mut HashMap<String, HashMap<Vec<TypeId>, usize>>,
) {
    match stmt {
        Statement::Binding(b) => {
            collect_call_profiles_expr(&b.value, resolved, profiles);
        }
        Statement::Expression(expr) => {
            collect_call_profiles_expr(expr, resolved, profiles);
        }
        Statement::Return(expr, _) => {
            collect_call_profiles_expr(expr, resolved, profiles);
        }
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => {
            collect_call_profiles_expr(condition, resolved, profiles);
            collect_call_profiles_block(body, resolved, profiles);
            for (cond, blk) in elif_branches {
                collect_call_profiles_expr(cond, resolved, profiles);
                collect_call_profiles_block(blk, resolved, profiles);
            }
            if let Some(eb) = else_body {
                collect_call_profiles_block(eb, resolved, profiles);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_call_profiles_expr(condition, resolved, profiles);
            collect_call_profiles_block(body, resolved, profiles);
        }
        Statement::For {
            iterable, body, ..
        } => {
            collect_call_profiles_expr(iterable, resolved, profiles);
            collect_call_profiles_block(body, resolved, profiles);
        }
        Statement::ForRange {
            start, end, step, body, ..
        } => {
            collect_call_profiles_expr(start, resolved, profiles);
            collect_call_profiles_expr(end, resolved, profiles);
            if let Some(s) = step {
                collect_call_profiles_expr(s, resolved, profiles);
            }
            collect_call_profiles_block(body, resolved, profiles);
        }
        Statement::ForKV {
            iterable, body, ..
        } => {
            collect_call_profiles_expr(iterable, resolved, profiles);
            collect_call_profiles_block(body, resolved, profiles);
        }
        Statement::FieldAssign { target, value, .. } => {
            collect_call_profiles_expr(target, resolved, profiles);
            collect_call_profiles_expr(value, resolved, profiles);
        }
        Statement::PatternBinding { value, .. } => {
            collect_call_profiles_expr(value, resolved, profiles);
        }
        _ => {}
    }
}

/// Check whether a function body calls itself recursively.
fn is_recursive_function(body: &Block, name: &str) -> bool {
    for stmt in &body.statements {
        if stmt_calls_name(stmt, name) {
            return true;
        }
    }
    if let Some(val) = &body.value {
        if expr_calls_name(val, name) {
            return true;
        }
    }
    false
}

fn stmt_calls_name(stmt: &Statement, name: &str) -> bool {
    match stmt {
        Statement::Binding(b) => expr_calls_name(&b.value, name),
        Statement::Expression(expr) => expr_calls_name(expr, name),
        Statement::Return(expr, _) => expr_calls_name(expr, name),
        Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            ..
        } => {
            expr_calls_name(condition, name)
                || block_calls_name(body, name)
                || elif_branches
                    .iter()
                    .any(|(c, b)| expr_calls_name(c, name) || block_calls_name(b, name))
                || else_body.as_ref().map_or(false, |b| block_calls_name(b, name))
        }
        Statement::While {
            condition, body, ..
        } => expr_calls_name(condition, name) || block_calls_name(body, name),
        Statement::For {
            iterable, body, ..
        } => expr_calls_name(iterable, name) || block_calls_name(body, name),
        Statement::ForRange {
            start,
            end,
            step,
            body,
            ..
        } => {
            expr_calls_name(start, name)
                || expr_calls_name(end, name)
                || step.as_ref().map_or(false, |s| expr_calls_name(s, name))
                || block_calls_name(body, name)
        }
        Statement::ForKV {
            iterable, body, ..
        } => expr_calls_name(iterable, name) || block_calls_name(body, name),
        Statement::FieldAssign { target, value, .. } => {
            expr_calls_name(target, name) || expr_calls_name(value, name)
        }
        Statement::PatternBinding { value, .. } => expr_calls_name(value, name),
        _ => false,
    }
}

fn block_calls_name(block: &Block, name: &str) -> bool {
    for stmt in &block.statements {
        if stmt_calls_name(stmt, name) {
            return true;
        }
    }
    block.value.as_ref().map_or(false, |v| expr_calls_name(v, name))
}

fn expr_calls_name(expr: &Expression, name: &str) -> bool {
    match expr {
        Expression::Call { callee, args, .. } => {
            if let Expression::Identifier(ref id, _) = **callee {
                if id == name {
                    return true;
                }
            }
            expr_calls_name(callee, name) || args.iter().any(|a| expr_calls_name(a, name))
        }
        Expression::Binary { left, right, .. } => {
            expr_calls_name(left, name) || expr_calls_name(right, name)
        }
        Expression::Unary { expr: inner, .. } => expr_calls_name(inner, name),
        Expression::Ternary {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            expr_calls_name(condition, name)
                || expr_calls_name(then_branch, name)
                || expr_calls_name(else_branch, name)
        }
        Expression::Lambda { body, .. } => block_calls_name(body, name),
        Expression::Member { target, .. } => expr_calls_name(target, name),
        Expression::Index { target, index, .. } => {
            expr_calls_name(target, name) || expr_calls_name(index, name)
        }
        Expression::Slice {
            target, start, end, ..
        } => {
            expr_calls_name(target, name)
                || expr_calls_name(start, name)
                || expr_calls_name(end, name)
        }
        Expression::List(items, _) => items.iter().any(|i| expr_calls_name(i, name)),
        Expression::Map(entries, _) => entries
            .iter()
            .any(|(k, v)| expr_calls_name(k, name) || expr_calls_name(v, name)),
        Expression::Pipeline { left, right, .. } => {
            expr_calls_name(left, name) || expr_calls_name(right, name)
        }
        Expression::Match(m) => {
            expr_calls_name(&m.value, name)
                || m.arms.iter().any(|arm| {
                    arm.guard.as_ref().map_or(false, |g| expr_calls_name(g, name))
                        || block_calls_name(&arm.body, name)
                })
                || m.default.as_ref().map_or(false, |d| block_calls_name(d, name))
        }
        Expression::ErrorPropagate { expr: inner, .. }
        | Expression::Throw { value: inner, .. } => expr_calls_name(inner, name),
        Expression::Unsafe { block, .. } => block_calls_name(block, name),
        _ => false,
    }
}

/// Collect monomorphization info: identify functions with consistent call-site type profiles.
fn collect_monomorph_info(
    functions: &[Function],
    globals: &[Binding],
    resolved: &TypeEnv,
) -> MonomorphInfo {
    let mut profiles: HashMap<String, HashMap<Vec<TypeId>, usize>> = HashMap::new();

    // Walk all expressions across functions and globals
    for func in functions {
        collect_call_profiles_block(&func.body, resolved, &mut profiles);
    }
    for global in globals {
        collect_call_profiles_expr(&global.value, resolved, &mut profiles);
    }

    // Determine which functions are recursive
    let mut recursive: HashSet<String> = HashSet::new();
    for func in functions {
        if is_recursive_function(&func.body, &func.name) {
            recursive.insert(func.name.clone());
        }
    }

    // Set of user-defined function names
    let fn_names: HashSet<String> = functions.iter().map(|f| f.name.clone()).collect();

    // Filter candidates: user-defined, non-recursive, ≤4 distinct profiles
    let candidates = profiles
        .into_iter()
        .filter(|(name, sigs)| {
            sigs.len() <= 4 && !recursive.contains(name) && fn_names.contains(name)
        })
        .map(|(name, sigs)| {
            let return_type = resolved
                .get(&name)
                .and_then(|ty| {
                    if let TypeId::Func(_, ret) = ty {
                        Some(*ret.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or(TypeId::Unknown);
            let variants = sigs
                .into_iter()
                .map(|(types, count)| MonomorphVariant {
                    param_types: types,
                    return_type: return_type.clone(),
                    call_count: count,
                })
                .collect();
            (name, variants)
        })
        .collect();

    MonomorphInfo { candidates }
}
