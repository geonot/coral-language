use crate::ast::{
    Binding,
    Block,
    ErrorDefinition,
    Field,
    Function,
    Item,
    MatchExpression,
    Parameter,
    Program,
    Statement,
    Expression,
    TraitDefinition,
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
    pub type_defs: Vec<crate::ast::TypeDefinition>,  // For ADT codegen
    pub trait_defs: Vec<TraitDefinition>,  // For trait method resolution
    pub error_defs: Vec<ErrorDefinition>,  // For error hierarchy
    pub constraints: ConstraintSet,
    pub types: TypeEnv,
    pub mutability: MutabilityEnv,
    pub allocation: AllocationHints,
    pub usage: UsageMetrics,
    pub warnings: Vec<Diagnostic>,  // Non-fatal warnings (e.g., unhandled errors)
}

/// Register error definitions recursively, building paths like "Database:Connection:Timeout"
fn register_error_definitions(def: &ErrorDefinition, prefix: &str, known_names: &mut HashSet<String>) {
    let full_name = if prefix.is_empty() {
        def.name.clone()
    } else {
        format!("{}:{}", prefix, def.name)
    };
    
    // Register the error name (like "Database" or "Database:Connection")
    known_names.insert(full_name.clone());
    
    // Recursively register child errors
    for child in &def.children {
        register_error_definitions(child, &full_name, known_names);
    }
}

pub fn analyze(program: Program) -> Result<SemanticModel, Diagnostic> {
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
    
    // Track constructor→enum_name to detect collisions (S3)
    let mut constructor_owners: HashMap<String, String> = HashMap::new();

    // First pass: collect all top-level names (functions, externs, store constructors, types)
    // This allows undefined name detection to know about forward references
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
                // Store constructors: make_StoreName
                known_names.insert(format!("make_{}", store.name));
                // Also register the store/actor name itself for type references
                known_names.insert(store.name.clone());
            }
            Item::Binding(binding) => {
                known_names.insert(binding.name.clone());
            }
            Item::Type(r#type) => {
                // Register the type name for forward references
                known_names.insert(r#type.name.clone());
                // Register variant constructor names
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
                // Special-case Message type: force data field to Any
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
                
                // Handle enum (sum type) variants - register constructors
                if !r#type.variants.is_empty() {
                    // Register the enum type itself as an ADT
                    types.insert(r#type.name.clone(), TypeId::Adt(r#type.name.clone()));
                    
                    // Register each variant constructor
                    for variant in &r#type.variants {
                        let ctor_name = variant.name.clone();
                        let adt_type = TypeId::Adt(r#type.name.clone());
                        
                        // Detect constructor name collisions (S3)
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
                        
                        if variant.fields.is_empty() {
                            // Nullary constructor - register as an ADT value (not a function)
                            // so that `None` can be used directly without calling it
                            types.insert(
                                ctor_name.clone(),
                                adt_type,
                            );
                        } else {
                            // Constructor with fields - register as a function returning the ADT
                            let param_types: Vec<TypeId> = variant.fields.iter()
                                .map(|_| TypeId::Primitive(Primitive::Any))
                                .collect();
                            let return_type = Box::new(adt_type);
                            
                            types.insert(
                                ctor_name.clone(),
                                TypeId::Func(param_types, return_type),
                            );
                        }
                        
                        // Also add to known_names so constructor calls are recognized
                        known_names.insert(ctor_name);
                    }
                }
                
                // Scope-check type method bodies
                for method in &r#type.methods {
                    check_method_with_fields(method, &r#type.fields, &known_names)?;
                }
                
                // Save type definition for codegen
                type_defs.push(r#type);
            }
            Item::Store(store) => {
                let kind = if store.is_actor { "actor" } else { "store" };
                check_field_uniqueness(kind, &store.name, &store.fields)?;
                if store.is_actor {
                    types.insert(store.name.clone(), TypeId::Primitive(Primitive::Actor));
                    // Validate actor message handlers
                    for method in &store.methods {
                        if method.kind == crate::ast::FunctionKind::ActorMessage && method.params.len() > 1 {
                            return Err(Diagnostic::new(
                                format!(
                                    "actor message handler `@{}` has {} parameters, but handlers can have at most 1 (message payload)",
                                    method.name, method.params.len()
                                ),
                                method.span,
                            ));
                        }
                    }
                } else {
                    // Non-actor store: register constructor make_StoreName() -> Any
                    let ctor_name = format!("make_{}", store.name);
                    types.insert(
                        ctor_name,
                        TypeId::Func(vec![], Box::new(TypeId::Primitive(Primitive::Any))),
                    );
                }
                // Scope-check store method bodies
                for method in &store.methods {
                    check_method_with_fields(method, &store.fields, &known_names)?;
                }
                stores.push(store);
            }
            Item::Taxonomy(_) => {}
            Item::ErrorDefinition(error_def) => {
                // Register the error hierarchy in the known names
                register_error_definitions(&error_def, "", &mut known_names);
                // Store the error definition for codegen
                error_defs.push(error_def);
            }
            Item::TraitDefinition(trait_def) => {
                // Register trait name
                known_names.insert(trait_def.name.clone());
                // Register trait methods in the type environment
                for method in &trait_def.methods {
                    // Trait methods are functions that can be called
                    let param_types: Vec<TypeId> = method.params.iter()
                        .map(|_| TypeId::Primitive(Primitive::Any))
                        .collect();
                    types.insert(
                        format!("{}::{}", trait_def.name, method.name),
                        TypeId::Func(param_types, Box::new(TypeId::Primitive(Primitive::Any))),
                    );
                }
                // Store trait definitions for method resolution and validation
                trait_defs.push(trait_def);
            }
        }
    }
    let mut constraints = ConstraintSet::default();
    let mut graph = TypeGraph::default();
    collect_program_constraints(&globals, &functions, &mut constraints, &mut types, &mut graph);
    if let Err(errors) = crate::types::solve_constraints(&constraints, &mut graph) {
        // Use the first error's span for precise location, list all messages.
        let first_span = errors.first().map(|e| e.span).unwrap_or(program.span);
        let msg = errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; ");
        return Err(Diagnostic::new(format!("type inference failed: {msg}"), first_span));
    }
    // Resolve types after solving for easier diagnostics downstream.
    let mut resolved = TypeEnv::default();
    for (name, ty) in types.symbols.iter() {
        let mut g = graph.clone();
        let r = crate::types::resolve(ty.clone(), &mut g);
        resolved.insert(name.clone(), r);
    }

    let (usage, mutability, allocation) = infer_mutability_and_usage(&globals, &functions);

    // Check match exhaustiveness after type definitions are collected
    check_all_match_exhaustiveness(&globals, &functions, &type_defs)?;

    // Collect warnings for unhandled error values
    let mut warnings = Vec::new();
    check_unhandled_errors(&globals, &functions, &mut warnings);

    // Inject default trait method bodies into types/stores that don't override them
    inject_trait_default_methods(&mut type_defs, &mut stores, &trait_defs);

    // Validate trait implementations
    validate_trait_implementations(&type_defs, &stores, &trait_defs, &mut warnings)?;

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
    })
}

/// Inject default trait method bodies into types/stores that don't override them.
/// This ensures codegen can compile trait default methods as regular methods
/// on the type/store, without needing any trait-specific codegen logic.
fn inject_trait_default_methods(
    type_defs: &mut [crate::ast::TypeDefinition],
    stores: &mut [crate::ast::StoreDefinition],
    trait_defs: &[TraitDefinition],
) {
    let trait_map: HashMap<&str, &TraitDefinition> = trait_defs
        .iter()
        .map(|t| (t.name.as_str(), t))
        .collect();

    // Inject into type definitions
    for type_def in type_defs.iter_mut() {
        for trait_name in &type_def.with_traits {
            if let Some(trait_def) = trait_map.get(trait_name.as_str()) {
                for method in &trait_def.methods {
                    if let Some(ref body) = method.body {
                        // Only inject if the type doesn't already have this method
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

    // Inject into store definitions
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

/// Validate trait implementations for types and stores
/// - Check that all declared traits exist
/// - Check that trait dependencies are satisfied  
/// - Check that required methods are implemented
fn validate_trait_implementations(
    type_defs: &[crate::ast::TypeDefinition],
    stores: &[crate::ast::StoreDefinition],
    trait_defs: &[TraitDefinition],
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    // Build a map of trait name -> trait definition
    let trait_map: HashMap<&str, &TraitDefinition> = trait_defs
        .iter()
        .map(|t| (t.name.as_str(), t))
        .collect();
    
    // Validate type implementations
    for type_def in type_defs {
        validate_type_traits(type_def, &trait_map, warnings)?;
    }
    
    // Validate store implementations
    for store in stores {
        validate_store_traits(store, &trait_map, warnings)?;
    }
    
    // Validate trait dependencies (within trait definitions themselves)
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

/// Validate trait implementations for a type definition
fn validate_type_traits(
    type_def: &crate::ast::TypeDefinition,
    trait_map: &HashMap<&str, &TraitDefinition>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    for trait_name in &type_def.with_traits {
        // Check trait exists
        let Some(trait_def) = trait_map.get(trait_name.as_str()) else {
            return Err(Diagnostic::new(
                format!(
                    "type `{}` implements unknown trait `{}`",
                    type_def.name, trait_name
                ),
                type_def.span,
            ));
        };
        
        // Check trait dependencies are satisfied
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
        
        // Check required methods are implemented
        let type_method_names: HashSet<&str> = type_def
            .methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        
        for method in &trait_def.methods {
            // Methods without a body are required
            if method.body.is_none() && !type_method_names.contains(method.name.as_str()) {
                return Err(Diagnostic::new(
                    format!(
                        "type `{}` does not implement required method `{}` from trait `{}`",
                        type_def.name, method.name, trait_name
                    ),
                    type_def.span,
                ));
            }
            
            // Warn if a method shadows a default implementation
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

/// Validate trait implementations for a store definition
fn validate_store_traits(
    store: &crate::ast::StoreDefinition,
    trait_map: &HashMap<&str, &TraitDefinition>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    let kind = if store.is_actor { "actor" } else { "store" };
    
    for trait_name in &store.with_traits {
        // Check trait exists
        let Some(trait_def) = trait_map.get(trait_name.as_str()) else {
            return Err(Diagnostic::new(
                format!(
                    "{} `{}` implements unknown trait `{}`",
                    kind, store.name, trait_name
                ),
                store.span,
            ));
        };
        
        // Check trait dependencies are satisfied
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
        
        // Check required methods are implemented
        let store_method_names: HashSet<&str> = store
            .methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        
        for method in &trait_def.methods {
            // Methods without a body are required
            if method.body.is_none() && !store_method_names.contains(method.name.as_str()) {
                return Err(Diagnostic::new(
                    format!(
                        "{} `{}` does not implement required method `{}` from trait `{}`",
                        kind, store.name, method.name, trait_name
                    ),
                    store.span,
                ));
            }
            
            // Warn if a method shadows a default implementation
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
    // Create a return-type TypeVar. Each `return expr` in the body will constrain
    // this to equal the expression's type, so the function's return type is correctly
    // inferred even when the body ends with a `return` statement (which has no
    // trailing block expression).
    let return_ty = TypeId::TypeVar(graph.fresh());
    let body_ty = collect_block_constraints(&function.body, constraints, types, graph, Some(&return_ty));
    // If the body has a trailing expression, use its type; otherwise use the
    // return-type TypeVar (which was constrained by any `return` statements).
    let fn_return = if function.body.value.is_some() {
        body_ty
    } else if has_return_statements(&function.body) {
        return_ty
    } else {
        body_ty // Unit
    };
    let fn_ty = TypeId::Func(params_tys, Box::new(fn_return));
    types.insert(function.name.clone(), fn_ty);
}

/// Check whether a block (or its nested blocks) contains any Return statements.
fn has_return_statements(block: &Block) -> bool {
    for stmt in &block.statements {
        match stmt {
            crate::ast::Statement::Return(_, _) => return true,
            crate::ast::Statement::If { body, elif_branches, else_body, .. } => {
                if has_return_statements(body) { return true; }
                for (_, blk) in elif_branches {
                    if has_return_statements(blk) { return true; }
                }
                if let Some(eb) = else_body {
                    if has_return_statements(eb) { return true; }
                }
            }
            crate::ast::Statement::While { body, .. } => {
                if has_return_statements(body) { return true; }
            }
            crate::ast::Statement::For { body, .. } => {
                if has_return_statements(body) { return true; }
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
) -> TypeId {
    for statement in &block.statements {
        match statement {
            crate::ast::Statement::Binding(binding) => {
                let rhs_ty = collect_constraints_expr(&binding.value, constraints, types, graph);
                if let Some(ann) = &binding.type_annotation {
                    let ann_ty = type_from_annotation(ann);
                    constraints.push(ConstraintKind::EqualAt(rhs_ty.clone(), ann_ty.clone(), binding.span));
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
                // Constrain the return expression type to match the function return type
                if let Some(ret_ty) = return_ty {
                    constraints.push(ConstraintKind::EqualAt(ret_expr_ty, ret_ty.clone(), *span));
                }
            }
            crate::ast::Statement::If { condition, body, elif_branches, else_body, .. } => {
                let _ = collect_constraints_expr(condition, constraints, types, graph);
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty);
                for (cond, blk) in elif_branches {
                    let _ = collect_constraints_expr(cond, constraints, types, graph);
                    let _ = collect_block_constraints(blk, constraints, types, graph, return_ty);
                }
                if let Some(else_blk) = else_body {
                    let _ = collect_block_constraints(else_blk, constraints, types, graph, return_ty);
                }
            }
            crate::ast::Statement::While { condition, body, .. } => {
                let _ = collect_constraints_expr(condition, constraints, types, graph);
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty);
            }
            crate::ast::Statement::For { iterable, body, .. } => {
                let _ = collect_constraints_expr(iterable, constraints, types, graph);
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty);
            }
            crate::ast::Statement::Break(_) | crate::ast::Statement::Continue(_) => {}
            crate::ast::Statement::FieldAssign { target, value, .. } => {
                let _ = collect_constraints_expr(target, constraints, types, graph);
                let _ = collect_constraints_expr(value, constraints, types, graph);
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
        Expression::None(_) => TypeId::Primitive(Primitive::Unit),  // none is unit/absent
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
        Expression::List(items, span) => {
            let elem_ty = if items.is_empty() {
                TypeId::TypeVar(graph.fresh())
            } else {
                let first = collect_constraints_expr(&items[0], constraints, types, graph);
                for item in &items[1..] {
                    let ty = collect_constraints_expr(item, constraints, types, graph);
                    constraints.push(ConstraintKind::EqualAt(first.clone(), ty, *span));
                }
                first
            };
            TypeId::List(Box::new(elem_ty))
        }
        Expression::Map(entries, span) => {
            // Maps in Coral are heterogeneous at runtime (all values are tagged Value*).
            // We still enforce homogeneous keys for lookup semantics, but values can differ.
            let key_ty = TypeId::TypeVar(graph.fresh());
            for (k, v) in entries {
                let kt = collect_constraints_expr(k, constraints, types, graph);
                // Collect constraints from value expressions but don't unify them
                let _vt = collect_constraints_expr(v, constraints, types, graph);
                constraints.push(ConstraintKind::EqualAt(key_ty.clone(), kt, *span));
            }
            // Value type is Any since maps are heterogeneous
            TypeId::Map(Box::new(key_ty), Box::new(TypeId::Primitive(Primitive::Any)))
        }
        Expression::Binary { op, left, right, span } => {
            let l = collect_constraints_expr(left, constraints, types, graph);
            let r = collect_constraints_expr(right, constraints, types, graph);
            match op {
                crate::ast::BinaryOp::Add => {
                    // Add is polymorphic:
                    // - String + anything = String (concatenation with auto-conversion)
                    // - Numeric + Numeric = Numeric (arithmetic)
                    // We check this at runtime via value_add, so don't constrain equal types.
                    // The result type depends on operands - if either is String, result is String.
                    match (&l, &r) {
                        (TypeId::Primitive(Primitive::String), _) | (_, TypeId::Primitive(Primitive::String)) => {
                            TypeId::Primitive(Primitive::String)
                        }
                        _ => {
                            // For non-string cases, require same types
                            constraints.push(ConstraintKind::EqualAt(l.clone(), r.clone(), *span));
                            l
                        }
                    }
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
                crate::ast::BinaryOp::Equals
                | crate::ast::BinaryOp::NotEquals => {
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
        Expression::Call { callee, args, span } => {
            // Special-case: if callee is a Member expression with a known method name,
            // bypass the CallableAt constraint (which would fail because e.g. "length" 
            // returns Int, not a callable type). Instead, directly return the method's
            // result type. This fixes `log(s.length())` and similar nested method calls.
            if let Expression::Member { target, property, .. } = callee.as_ref() {
                let _target_ty = collect_constraints_expr(target, constraints, types, graph);
                // Collect arg constraints regardless
                for arg in args {
                    collect_constraints_expr(arg, constraints, types, graph);
                }
                match property.as_str() {
                    "length" | "count" | "size" => return TypeId::Primitive(Primitive::Int),
                    "err" => return TypeId::Primitive(Primitive::Bool),
                    "equals" | "not_equals" | "contains" | "any" | "all" => return TypeId::Primitive(Primitive::Bool),
                    "push" | "pop" | "get" | "set" | "append" | "remove" | "insert" 
                    | "clear" | "join" | "map" | "filter" | "reduce" | "find" | "sort"
                    | "keys" | "values" | "not" | "iter" | "to_string" | "or" | "unwrap_or" => {
                        return TypeId::Unknown;
                    }
                    _ => {
                        // Not a known built-in method — fall through to normal Call handling
                    }
                }
            }
            let callee_ty = collect_constraints_expr(callee, constraints, types, graph);
            let mut arg_tys = Vec::new();
            for arg in args {
                arg_tys.push(collect_constraints_expr(arg, constraints, types, graph));
            }
            let result_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::CallableAt(callee_ty.clone(), arg_tys.clone(), result_ty.clone(), *span));
            result_ty
        }
        Expression::Index { target, index, span: _ } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            let _index_ty = collect_constraints_expr(index, constraints, types, graph);
            // Index returns element type for lists, value type for maps
            match &target_ty {
                TypeId::List(elem) => *elem.clone(),
                TypeId::Map(_, val) => *val.clone(),
                _ => TypeId::Unknown,
            }
        }
        Expression::Member { target, property, span } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            match property.as_str() {
                // Properties that return Int on any collection
                "length" | "count" | "size" => TypeId::Primitive(Primitive::Int),
                
                // .err property returns Bool - checks if value is an error
                "err" => TypeId::Primitive(Primitive::Bool),
                
                // Methods on collections - these are callable, not field accesses.
                // Return a function type that will be checked by Callable constraint
                // when used in a Call expression. For bare method access (unusual),
                // just return Unknown to avoid false unification.
                "push" | "pop" | "get" | "set" | "append" | "remove" | "insert" 
                | "contains" | "keys" | "values" | "clear" | "join"
                | "map" | "filter" | "reduce" | "find" | "any" | "all" | "sort"
                | "equals" | "not_equals" | "not" | "iter" => {
                    // Don't constrain target type - let the Call expression handle it
                    TypeId::Unknown
                }
                
                _ => {
                    // For Any/Unknown targets (e.g. store instances, unresolved types),
                    // don't constrain - the method/field access will be resolved at codegen time.
                    // Also for TypeVars that might resolve to Any.
                    // For other targets, treat as map lookup with string key.
                    match &target_ty {
                        TypeId::Primitive(Primitive::Any) | TypeId::Unknown | TypeId::TypeVar(_) => TypeId::Unknown,
                        _ => {
                            let val_ty = TypeId::TypeVar(graph.fresh());
                            let map_ty = TypeId::Map(Box::new(TypeId::Primitive(Primitive::String)), Box::new(val_ty.clone()));
                            constraints.push(ConstraintKind::EqualAt(target_ty, map_ty, *span));
                            val_ty
                        }
                    }
                }
            }
        }
        Expression::Ternary { condition, then_branch, else_branch, span } => {
            let cond_ty = collect_constraints_expr(condition, constraints, types, graph);
            let then_ty = collect_constraints_expr(then_branch, constraints, types, graph);
            let else_ty = collect_constraints_expr(else_branch, constraints, types, graph);
            constraints.push(ConstraintKind::BooleanAt(cond_ty, *span));
            constraints.push(ConstraintKind::EqualAt(then_ty.clone(), else_ty.clone(), *span));
            then_ty
        }
        Expression::Match(match_expr) => {
            let match_span = match_expr.span;
            let scrutinee_ty = collect_constraints_expr(&match_expr.value, constraints, types, graph);
            let mut arm_tys = Vec::new();
            for arm in &match_expr.arms {
                match &arm.pattern {
                    crate::ast::MatchPattern::Integer(_) => {
                        constraints.push(ConstraintKind::NumericAt(scrutinee_ty.clone(), match_span));
                    }
                    crate::ast::MatchPattern::Bool(_) => {
                        constraints.push(ConstraintKind::EqualAt(
                            scrutinee_ty.clone(),
                            TypeId::Primitive(Primitive::Bool),
                            match_span,
                        ));
                    }
                    crate::ast::MatchPattern::String(_) => {
                        constraints.push(ConstraintKind::EqualAt(scrutinee_ty.clone(), TypeId::Primitive(Primitive::String), match_span));
                    }
                    crate::ast::MatchPattern::List(patterns) => {
                        let elem_ty = if patterns.is_empty() {
                            TypeId::TypeVar(graph.fresh())
                        } else {
                            // All elements in a list pattern should have consistent types
                            // For now, use Any since patterns don't carry type info directly
                            TypeId::Primitive(Primitive::Any)
                        };
                        constraints.push(ConstraintKind::EqualAt(
                            scrutinee_ty.clone(),
                            TypeId::List(Box::new(elem_ty)),
                            match_span,
                        ));
                    }
                    crate::ast::MatchPattern::Identifier(name) => {
                        types.insert(name.clone(), scrutinee_ty.clone());
                    }
                    crate::ast::MatchPattern::Constructor { name, fields, span: _ } => {
                        // Look up the constructor to find its ADT type
                        if let Some(ctor_ty) = types.get(name) {
                            let adt_ty = match ctor_ty {
                                // Nullary constructor → type is directly the ADT
                                TypeId::Adt(adt_name) => TypeId::Adt(adt_name.clone()),
                                // Constructor with fields → return type of the function is the ADT
                                TypeId::Func(_, ret) => (**ret).clone(),
                                _ => TypeId::Primitive(Primitive::Any),
                            };
                            // Constrain the scrutinee to the ADT type
                            constraints.push(ConstraintKind::EqualAt(
                                scrutinee_ty.clone(),
                                adt_ty,
                                match_span,
                            ));
                        }
                        // Bind field variables (fields are dynamically typed for now)
                        for pat in fields {
                            collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
                        }
                    }
                    crate::ast::MatchPattern::Wildcard(_) => {
                        // Wildcard matches anything, no type constraints
                    }
                }
                let arm_ty = collect_block_constraints(&arm.body, constraints, types, graph, None);
                arm_tys.push(arm_ty);
            }
            if let Some(default) = &match_expr.default {
                arm_tys.push(collect_block_constraints(default, constraints, types, graph, None));
            }
            arm_tys
                .into_iter()
                .reduce(|a, b| {
                    constraints.push(ConstraintKind::EqualAt(a.clone(), b.clone(), match_span));
                    a
                })
                .unwrap_or(TypeId::Primitive(Primitive::Unit))
        }
        Expression::Throw { value, .. } => collect_constraints_expr(value, constraints, types, graph),
        Expression::Lambda { params, body, .. } => {
            let mut param_tys = Vec::new();
            let mut shadow = TypeEnv::default();
            shadow.symbols = types.symbols.clone();
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
            let body_ty = collect_block_constraints(body, &mut nested_constraints, &mut shadow, &mut nested_graph, None);
            constraints.constraints.extend(nested_constraints.constraints);
            TypeId::Func(param_tys, Box::new(body_ty))
        }
        Expression::Pipeline { left, right, .. } => {
            // Pipeline `a ~ f(args)` desugars to `f(a, args)`
            // Collect constraints for both sides
            let _left_ty = collect_constraints_expr(left, constraints, types, graph);
            let right_ty = collect_constraints_expr(right, constraints, types, graph);
            // The result type depends on what's on the right - usually a call expression
            // We'll handle actual desugaring in codegen, here we just propagate types
            right_ty
        }
        Expression::ErrorValue { .. } => {
            // Error values are a special type - for now treat as Any since
            // they can be returned from any function
            TypeId::Primitive(Primitive::Any)
        }
        Expression::ErrorPropagate { expr, .. } => {
            // The type of error propagation is the type of the inner expression
            // (when it's not an error). The propagation itself may return early.
            collect_constraints_expr(expr, constraints, types, graph)
        }
    }    
}

/// Recursively collect identifier bindings from a pattern.
fn collect_pattern_bindings(pattern: &crate::ast::MatchPattern, ty: &TypeId, types: &mut TypeEnv) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            types.insert(name.clone(), ty.clone());
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            // Constructor fields are dynamically typed at runtime (Any)
            for pat in fields {
                collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
            }
        }
        // Literal patterns don't introduce bindings
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::List(_) => {}
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
        // Low-level types used in extern declarations - treat as Any.
        // TODO: Add proper low-level type support.
        "usize" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64"
        | "i8" | "i16" | "i32" | "i64" | "isize" => TypeId::Primitive(Primitive::Any),
        // Handle generic types
        "List" => {
            if ann.type_args.len() == 1 {
                let elem_type = type_from_annotation(&ann.type_args[0]);
                TypeId::List(Box::new(elem_type))
            } else {
                // List with wrong number of type arguments - default to List[Any]
                TypeId::List(Box::new(TypeId::Primitive(Primitive::Any)))
            }
        },
        "Map" => {
            if ann.type_args.len() == 2 {
                let key_type = type_from_annotation(&ann.type_args[0]);
                let value_type = type_from_annotation(&ann.type_args[1]);
                TypeId::Map(Box::new(key_type), Box::new(value_type))
            } else {
                // Map with wrong number of type arguments - default to Map[Any, Any]
                TypeId::Map(
                    Box::new(TypeId::Primitive(Primitive::Any)), 
                    Box::new(TypeId::Primitive(Primitive::Any))
                )
            }
        },
        // Unknown type annotations are permissive.
        _ => TypeId::Unknown,
    };
    
    base_type
}

fn check_function(function: &Function, known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    check_method_with_fields(function, &[], known_names)
}

/// Like check_function, but also declares type/store fields in scope so method
/// bodies can reference them as bare identifiers (e.g. `name` instead of `self.name`).
fn check_method_with_fields(function: &Function, fields: &[crate::ast::Field], known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    validate_parameter_defaults(&function.params)?;
    let mut scopes = ScopeStack::new();
    scopes.push();
    // Declare 'self' in scope for methods
    if !fields.is_empty() {
        scopes.declare("self".to_string(), function.span);
    }
    // Declare fields so method bodies can reference them
    for field in fields {
        scopes.declare(field.name.clone(), field.span);
    }
    for param in &function.params {
        if let Some(previous) = scopes.lookup(&param.name) {
            return Err(duplicate_symbol("parameter", &param.name, param.span, previous));
        }
        scopes.declare(param.name.clone(), param.span);
    }
    check_block(&function.body, &mut scopes, known_names)
}

fn check_block(block: &Block, scopes: &mut ScopeStack, known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    scopes.push();
    for statement in &block.statements {
        match statement {
            Statement::Binding(binding) => {
                // Allow rebinding — `is` creates a new binding that shadows any previous one.
                // This enables patterns like `x is x + 1` in loops and `x is 10` then `x is 20`.
                // The alloca-based codegen properly handles rebinding at the LLVM level.
                scopes.declare(binding.name.clone(), binding.span);
                check_expression(&binding.value, scopes, known_names)?;
            }
            Statement::Expression(expr) => check_expression(expr, scopes, known_names)?,
            Statement::Return(expr, _) => check_expression(expr, scopes, known_names)?,
            Statement::If { condition, body, elif_branches, else_body, .. } => {
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
            Statement::While { condition, body, .. } => {
                check_expression(condition, scopes, known_names)?;
                check_block(body, scopes, known_names)?;
            }
            Statement::For { variable, iterable, body, span } => {
                check_expression(iterable, scopes, known_names)?;
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
        }
    }
    if let Some(value) = &block.value {
        check_expression(value, scopes, known_names)?;
    }
    scopes.pop();
    Ok(())
}

fn check_expression(expr: &Expression, scopes: &mut ScopeStack, known_functions: &HashSet<String>) -> Result<(), Diagnostic> {
    match expr {
        Expression::Binary { left, right, .. } => {
            check_expression(left, scopes, known_functions)?;
            check_expression(right, scopes, known_functions)?;
        }
        Expression::Unary { expr, .. } => check_expression(expr, scopes, known_functions)?,
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
        Expression::ErrorValue { .. } => {
            // Error values are always valid - the error path is checked at definition time
        }
        Expression::ErrorPropagate { expr, .. } => {
            // Check the inner expression
            check_expression(expr, scopes, known_functions)?;
        }
        Expression::Match(match_expr) => check_match_expression(match_expr, scopes, known_functions)?,
        Expression::Throw { value, .. } => check_expression(value, scopes, known_functions)?,
        Expression::Lambda { params, body, .. } => check_lambda(params, body, scopes, known_functions)?,
        Expression::Identifier(name, span) => {
            // Check if identifier is defined in scope, known functions, or is a builtin
            if scopes.lookup(name).is_none()
                && !known_functions.contains(name)
                && !is_builtin_name(name)
            {
                return Err(Diagnostic::new(
                    format!("undefined name `{}`", name),
                    *span,
                ));
            }
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

/// Check if a name is a builtin (log, io namespace, runtime functions, etc.)
fn is_builtin_name(name: &str) -> bool {
    matches!(name, 
        // Core builtins
        "log" | "io" | "self" | "true" | "false" |
        // Bit operations (intrinsics)
        "bit_and" | "bit_or" | "bit_xor" | "bit_not" | "bit_shl" | "bit_shr" |
        // Standard functions mapped to runtime
        "length" | "push" | "pop" | "get" | "set" | "keys" | "values" |
        // Math operations
        "abs" | "sqrt" | "floor" | "ceil" | "round" | "sin" | "cos" | "tan" |
        "ln" | "log10" | "exp" | "asin" | "acos" | "atan" | "atan2" |
        "sinh" | "cosh" | "tanh" | "trunc" | "sign" | "signum" |
        "deg_to_rad" | "rad_to_deg" | "min" | "max" | "pow" |
        // Type checks
        "is_number" | "is_string" | "is_bool" | "is_list" | "is_map" |
        // String operations  
        "concat" | "split" | "join" | "trim" | "to_string" |
        "string_slice" | "slice" | "string_char_at" | "char_at" |
        "string_index_of" | "index_of" | "string_split" |
        "string_to_chars" | "chars" | "string_starts_with" | "starts_with" |
        "string_ends_with" | "ends_with" | "string_trim" |
        "string_to_upper" | "to_upper" | "string_to_lower" | "to_lower" |
        "string_replace" | "replace" | "string_contains" | "contains" |
        "string_parse_number" | "parse_number" | "number_to_string" |
        // Bytes operations
        "bytes_length" | "bytes_get" | "bytes_set" |
        "bytes_from_string" | "to_bytes" | "bytes_to_string" | "bytes_slice" |
        // File I/O operations (runtime FFI)
        "fs_read" | "fs_write" | "fs_exists" |
        "fs_append" | "fs_read_dir" | "read_dir" |
        "fs_mkdir" | "mkdir" | "fs_delete" | "delete" | "fs_is_dir" | "is_dir" |
        // Process and environment
        "process_args" | "args" | "process_exit" | "exit" |
        "env_get" | "env_set" |
        // I/O
        "stdin_read_line" | "read_line" |
        // List operations
        "list_contains" | "list_index_of" | "list_reverse" | "list_slice" |
        "list_sort" | "list_join" | "list_concat" |
        // Map operations
        "map_remove" | "map_values" | "map_entries" | "entries" |
        "map_has_key" | "has_key" | "map_merge" | "merge" |
        // Type introspection
        "type_of" |
        // Error handling builtins
        "is_err" | "is_ok" | "is_absent" | "error_name" | "error_code" |
        // Character operations
        "ord" | "string_ord" | "chr" | "string_chr" | "string_compare" | "strcmp" |
        // Actor operations (runtime FFI)
        "actor_spawn" | "actor_send" | "actor_stop" | "actor_self" |
        // Memory operations (runtime FFI)
        "value_retain" | "value_release" | "heap_alloc" | "heap_free"
    )
}

fn check_match_expression(expr: &MatchExpression, scopes: &mut ScopeStack, known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    check_expression(&expr.value, scopes, known_names)?;
    for arm in &expr.arms {
        // Match arms can introduce bindings via patterns
        scopes.push();
        declare_pattern_bindings(&arm.pattern, scopes, arm.body.span);
        check_block(&arm.body, scopes, known_names)?;
        scopes.pop();
    }
    if let Some(default) = &expr.default {
        check_block(default, scopes, known_names)?;
    }
    Ok(())
}

/// Declare bindings from a pattern into the scope
fn declare_pattern_bindings(pattern: &crate::ast::MatchPattern, scopes: &mut ScopeStack, span: crate::span::Span) {
    match pattern {
        crate::ast::MatchPattern::Identifier(name) => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::Constructor { fields, .. } => {
            // Constructor patterns may have nested bindings in fields
            for field_pattern in fields {
                declare_pattern_bindings(field_pattern, scopes, span);
            }
        }
        // Other patterns don't introduce bindings
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::List(_) => {}
    }
}

/// Build a mapping from constructor name to (enum_name, all_variant_names)
fn build_constructor_map(type_defs: &[crate::ast::TypeDefinition]) -> HashMap<String, (String, Vec<String>)> {
    let mut map = HashMap::new();
    for typedef in type_defs {
        if !typedef.variants.is_empty() {
            let variant_names: Vec<String> = typedef.variants.iter()
                .map(|v| v.name.clone())
                .collect();
            for variant in &typedef.variants {
                map.insert(variant.name.clone(), (typedef.name.clone(), variant_names.clone()));
            }
        }
    }
    map
}

/// Check all match expressions in the program for exhaustiveness
fn check_all_match_exhaustiveness(
    globals: &[Binding],
    functions: &[Function],
    type_defs: &[crate::ast::TypeDefinition],
) -> Result<(), Diagnostic> {
    let ctor_map = build_constructor_map(type_defs);
    
    // Check globals
    for binding in globals {
        check_expr_match_exhaustiveness(&binding.value, &ctor_map)?;
    }
    
    // Check functions
    for function in functions {
        check_block_match_exhaustiveness(&function.body, &ctor_map)?;
    }
    
    Ok(())
}

fn check_block_match_exhaustiveness(
    block: &Block,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
) -> Result<(), Diagnostic> {
    for statement in &block.statements {
        match statement {
            Statement::Binding(binding) => {
                check_expr_match_exhaustiveness(&binding.value, ctor_map)?;
            }
            Statement::Expression(expr) => {
                check_expr_match_exhaustiveness(expr, ctor_map)?;
            }
            Statement::Return(expr, _) => {
                check_expr_match_exhaustiveness(expr, ctor_map)?;
            }
            Statement::If { condition, body, elif_branches, else_body, .. } => {
                check_expr_match_exhaustiveness(condition, ctor_map)?;
                check_block_match_exhaustiveness(body, ctor_map)?;
                for (cond, blk) in elif_branches {
                    check_expr_match_exhaustiveness(cond, ctor_map)?;
                    check_block_match_exhaustiveness(blk, ctor_map)?;
                }
                if let Some(else_blk) = else_body {
                    check_block_match_exhaustiveness(else_blk, ctor_map)?;
                }
            }
            Statement::While { condition, body, .. } => {
                check_expr_match_exhaustiveness(condition, ctor_map)?;
                check_block_match_exhaustiveness(body, ctor_map)?;
            }
            Statement::For { iterable, body, .. } => {
                check_expr_match_exhaustiveness(iterable, ctor_map)?;
                check_block_match_exhaustiveness(body, ctor_map)?;
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { value, .. } => {
                check_expr_match_exhaustiveness(value, ctor_map)?;
            }
        }
    }
    if let Some(value) = &block.value {
        check_expr_match_exhaustiveness(value, ctor_map)?;
    }
    Ok(())
}

fn check_expr_match_exhaustiveness(
    expr: &Expression,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
) -> Result<(), Diagnostic> {
    match expr {
        Expression::Binary { left, right, .. } => {
            check_expr_match_exhaustiveness(left, ctor_map)?;
            check_expr_match_exhaustiveness(right, ctor_map)?;
        }
        Expression::Unary { expr, .. } => {
            check_expr_match_exhaustiveness(expr, ctor_map)?;
        }
        Expression::List(items, _) => {
            for item in items {
                check_expr_match_exhaustiveness(item, ctor_map)?;
            }
        }
        Expression::Map(entries, _) => {
            for (key, value) in entries {
                check_expr_match_exhaustiveness(key, ctor_map)?;
                check_expr_match_exhaustiveness(value, ctor_map)?;
            }
        }
        Expression::Call { callee, args, .. } => {
            check_expr_match_exhaustiveness(callee, ctor_map)?;
            for arg in args {
                check_expr_match_exhaustiveness(arg, ctor_map)?;
            }
        }
        Expression::Member { target, .. } => {
            check_expr_match_exhaustiveness(target, ctor_map)?;
        }
        Expression::Index { target, index, .. } => {
            check_expr_match_exhaustiveness(target, ctor_map)?;
            check_expr_match_exhaustiveness(index, ctor_map)?;
        }
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            check_expr_match_exhaustiveness(condition, ctor_map)?;
            check_expr_match_exhaustiveness(then_branch, ctor_map)?;
            check_expr_match_exhaustiveness(else_branch, ctor_map)?;
        }
        Expression::Match(match_expr) => {
            // Recursively check the matched value and arm bodies
            check_expr_match_exhaustiveness(&match_expr.value, ctor_map)?;
            for arm in &match_expr.arms {
                check_block_match_exhaustiveness(&arm.body, ctor_map)?;
            }
            if let Some(default) = &match_expr.default {
                check_block_match_exhaustiveness(default, ctor_map)?;
            }
            
            // Now check exhaustiveness
            check_single_match_exhaustiveness(match_expr, ctor_map)?;
        }
        Expression::Throw { value, .. } => {
            check_expr_match_exhaustiveness(value, ctor_map)?;
        }
        Expression::Lambda { body, .. } => {
            check_block_match_exhaustiveness(body, ctor_map)?;
        }
        Expression::Pipeline { left, right, .. } => {
            check_expr_match_exhaustiveness(left, ctor_map)?;
            check_expr_match_exhaustiveness(right, ctor_map)?;
        }
        Expression::ErrorValue { .. } => {
            // Error values don't contain sub-expressions
        }
        Expression::ErrorPropagate { expr, .. } => {
            check_expr_match_exhaustiveness(expr, ctor_map)?;
        }
        // Other expressions don't contain sub-expressions or match
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
    Ok(())
}

/// Check a single match expression for exhaustiveness
fn check_single_match_exhaustiveness(
    match_expr: &MatchExpression,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
) -> Result<(), Diagnostic> {
    // If there's a default block, the match is exhaustive
    if match_expr.default.is_some() {
        return Ok(());
    }
    
    // Collect all matched constructors and check for wildcards
    let mut matched_ctors: HashSet<String> = HashSet::new();
    let mut has_wildcard = false;
    let mut has_identifier_catch_all = false;
    
    for arm in &match_expr.arms {
        match &arm.pattern {
            crate::ast::MatchPattern::Constructor { name, .. } => {
                matched_ctors.insert(name.clone());
            }
            crate::ast::MatchPattern::Wildcard(_) => {
                has_wildcard = true;
            }
            crate::ast::MatchPattern::Identifier(_) => {
                // An identifier pattern catches everything
                has_identifier_catch_all = true;
            }
            // Literals (integer, bool, string, list) don't help with enum exhaustiveness
            _ => {}
        }
    }
    
    // If there's a wildcard or identifier catch-all, match is exhaustive
    if has_wildcard || has_identifier_catch_all {
        return Ok(());
    }
    
    // If no constructor patterns, we can't determine exhaustiveness 
    // (matching on literals or unknown types)
    if matched_ctors.is_empty() {
        return Ok(());
    }
    
    // Find the enum these constructors belong to
    // All constructors should belong to the same enum for a well-typed match
    // Safe: we checked !matched_ctors.is_empty() above
    let first_ctor = matched_ctors.iter().next()
        .expect("matched_ctors must be non-empty after is_empty check");
    if let Some((enum_name, all_variants)) = ctor_map.get(first_ctor) {
        // Check that all matched constructors belong to the same enum
        for ctor in &matched_ctors {
            if let Some((other_enum, _)) = ctor_map.get(ctor) {
                if other_enum != enum_name {
                    // Mixed enum types - this is a type error, but we let type checker handle it
                    return Ok(());
                }
            }
        }
        
        // Check which variants are missing
        let missing: Vec<&String> = all_variants.iter()
            .filter(|v| !matched_ctors.contains(*v))
            .collect();
        
        if !missing.is_empty() {
            let missing_list = missing.iter()
                .map(|s| format!("`{}`", s))
                .collect::<Vec<_>>()
                .join(", ");
            
            return Err(Diagnostic::new(
                format!("non-exhaustive match: missing pattern(s) for {}", missing_list),
                match_expr.span,
            ).with_help(
                "add arm(s) for the missing variant(s) or add a `_ =>` default arm".to_string()
            ));
        }
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

fn check_lambda(params: &[Parameter], body: &Block, scopes: &mut ScopeStack, known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    scopes.push();
    for param in params {
        if let Some(previous) = scopes.lookup(&param.name) {
            return Err(duplicate_symbol("parameter", &param.name, param.span, previous));
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
            Statement::If { condition, body, elif_branches, else_body, .. } => {
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
            Statement::While { condition, body, .. } => {
                visit_expression(condition, tracker);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::For { iterable, body, variable, .. } => {
                visit_expression(iterable, tracker);
                tracker.touch(variable);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { target, value, .. } => {
                visit_expression(target, tracker);
                visit_expression(value, tracker);
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
        Expression::Member { target, .. } => visit_expression(target, tracker),
        Expression::Index { target, index, .. } => {
            visit_expression(target, tracker);
            visit_expression(index, tracker);
        }
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
        Expression::Pipeline { left, right, .. } => {
            visit_expression(left, tracker);
            visit_expression(right, tracker);
        }
        Expression::ErrorValue { .. } => {}
        Expression::ErrorPropagate { expr, .. } => visit_expression(expr, tracker),
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
        Expression::Index { target, index, .. } => {
            mark_escapes(target, tracker);
            mark_escapes(index, tracker);
        }
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
        Expression::Pipeline { left, right, .. } => {
            mark_escapes(left, tracker);
            mark_escapes(right, tracker);
        }
        Expression::ErrorValue { .. } => {}
        Expression::ErrorPropagate { expr, .. } => mark_escapes(expr, tracker),
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
        Expression::Index { target, index, .. } => {
            find_forbidden_identifier(target, forbidden)
                .or_else(|| find_forbidden_identifier(index, forbidden))
        }
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
        Expression::Pipeline { left, right, .. } => {
            find_forbidden_identifier(left, forbidden)
                .or_else(|| find_forbidden_identifier(right, forbidden))
        }
        Expression::ErrorValue { .. } => None,
        Expression::ErrorPropagate { expr, .. } => find_forbidden_identifier(expr, forbidden),
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
        Statement::If { condition, body, elif_branches, else_body, .. } => {
            find_forbidden_identifier(condition, forbidden)
                .or_else(|| find_in_block(body, forbidden))
                .or_else(|| elif_branches.iter().find_map(|(cond, blk)| {
                    find_forbidden_identifier(cond, forbidden)
                        .or_else(|| find_in_block(blk, forbidden))
                }))
                .or_else(|| else_body.as_ref().and_then(|blk| find_in_block(blk, forbidden)))
        }
        Statement::While { condition, body, .. } => {
            find_forbidden_identifier(condition, forbidden)
                .or_else(|| find_in_block(body, forbidden))
        }
        Statement::For { iterable, body, .. } => {
            find_forbidden_identifier(iterable, forbidden)
                .or_else(|| find_in_block(body, forbidden))
        }
        Statement::Break(_) | Statement::Continue(_) => None,
        Statement::FieldAssign { target, value, .. } => {
            find_forbidden_identifier(target, forbidden)
                .or_else(|| find_forbidden_identifier(value, forbidden))
        }
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

/// Check for unhandled error values at top-level (globals) and function bodies.
/// Emits warnings when error values are assigned to variables but never handled.
fn check_unhandled_errors(
    globals: &[Binding],
    functions: &[Function],
    warnings: &mut Vec<Diagnostic>,
) {
    // Check top-level globals - warn if an expression produces an error value and is not used
    for binding in globals {
        // Check if the binding's value is an error expression that might be ignored
        if let Some(warning) = check_expr_may_produce_unhandled_error(&binding.value) {
            warnings.push(warning);
        }
    }
    
    // Check function bodies for ignored error values
    for function in functions {
        check_block_for_unhandled_errors(&function.body, warnings);
    }
}

/// Check if an expression directly produces an error that's not handled.
fn check_expr_may_produce_unhandled_error(expr: &Expression) -> Option<Diagnostic> {
    match expr {
        // Direct error value that's not being used in a conditional or propagated
        Expression::ErrorValue { span, path } => {
            Some(Diagnostic::new(
                format!("error value `err {}` is created but may not be handled", path.join(":")),
                *span,
            ).with_help("consider returning this error or handling it with a conditional"))
        }
        _ => None,
    }
}

/// Check a block for statements that might silently ignore errors.
fn check_block_for_unhandled_errors(block: &Block, warnings: &mut Vec<Diagnostic>) {
    for statement in &block.statements {
        match statement {
            Statement::Expression(expr) => {
                // A standalone expression statement that's an error value is suspicious
                if let Some(warning) = check_expr_may_produce_unhandled_error(expr) {
                    warnings.push(warning);
                }
                // Also recurse into nested blocks (lambdas, match expressions, etc.)
                check_expr_nested_blocks(expr, warnings);
            }
            Statement::Binding(binding) => {
                check_expr_nested_blocks(&binding.value, warnings);
            }
            Statement::Return(expr, _) => {
                check_expr_nested_blocks(expr, warnings);
            }
            Statement::If { condition, body, elif_branches, else_body, .. } => {
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
            Statement::While { condition, body, .. } => {
                check_expr_nested_blocks(condition, warnings);
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::For { iterable, body, .. } => {
                check_expr_nested_blocks(iterable, warnings);
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::Break(_) | Statement::Continue(_) => {}
            Statement::FieldAssign { value, .. } => {
                check_expr_nested_blocks(value, warnings);
            }
        }
    }
    
    // Check the final value expression if any
    if let Some(value) = &block.value {
        check_expr_nested_blocks(value, warnings);
    }
}

/// Recursively check nested blocks in expressions for unhandled errors.
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
        Expression::Ternary { then_branch, else_branch, .. } => {
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
        // Leaf expressions that don't contain nested blocks
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
    }
}
