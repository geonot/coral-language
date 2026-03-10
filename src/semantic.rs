use crate::ast::{
    Binding,
    Block,
    ErrorDefinition,
    ExtensionDefinition,
    Field,
    Function,
    FunctionKind,
    Item,
    MatchExpression,
    Parameter,
    Program,
    Statement,
    Expression,
    TraitDefinition,
};
use crate::diagnostics::{Diagnostic, WarningCategory};
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
    /// Maps (TypeName, FieldName) → field index for store/type field tracking
    pub field_types: HashMap<(String, String), usize>,
    /// Maps store/type names to list of field names for member access validation
    pub store_field_names: HashMap<String, Vec<String>>,
    /// CC3.2: Maps short module name (e.g., "math") to list of exported function names
    pub module_exports: HashMap<String, Vec<String>>,
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
    // CC3.2: Build module_exports map from parsed modules
    let mut module_exports: HashMap<String, Vec<String>> = HashMap::new();
    for module in &program.modules {
        // Use the short name (last segment) as the lookup key
        let short_name = module.name.rsplit('.').next().unwrap_or(&module.name).to_string();
        module_exports.insert(short_name, module.exports.clone());
        // Also register the full name for qualified access like `std.math.sin()`
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
    
    // Track constructor→enum_name to detect collisions (S3)
    let mut constructor_owners: HashMap<String, String> = HashMap::new();
    // Track (TypeName, FieldName) → field index for member access type inference (TS-4)
    let mut field_types: HashMap<(String, String), usize> = HashMap::new();
    let mut store_field_names: HashMap<String, Vec<String>> = HashMap::new();

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
                    // T2.1: Register generic type parameters in TypeEnv
                    let has_type_params = !r#type.type_params.is_empty();
                    if has_type_params {
                        types.register_generic_type(
                            r#type.name.clone(),
                            r#type.param_names(),
                        );
                        // T2.4: Register trait bounds for each type parameter
                        for tp in &r#type.type_params {
                            if !tp.bounds.is_empty() {
                                types.register_type_param_bounds(
                                    &r#type.name,
                                    &tp.name,
                                    tp.bounds.clone(),
                                );
                            }
                        }
                    }
                    
                    // Register the enum type itself as an ADT
                    types.insert(r#type.name.clone(), TypeId::Adt(r#type.name.clone(), vec![]));
                    
                    // Register each variant constructor
                    for variant in &r#type.variants {
                        let ctor_name = variant.name.clone();
                        let adt_type = TypeId::Adt(r#type.name.clone(), vec![]);
                        
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
                        
                        // T2.2: For generic enums, register constructors for let-polymorphism
                        // instead of a fixed monomorphic type. Each call site will get fresh vars.
                        if has_type_params {
                            types.register_generic_constructor(
                                ctor_name.clone(),
                                r#type.name.clone(),
                                r#type.type_params.iter().map(|tp| tp.name.clone()).collect(),
                                variant.fields.len(),
                            );
                        }
                        
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
                
                // Track type field names for member access type inference (TS-4)
                {
                    let names: Vec<String> = r#type.fields.iter().map(|f| f.name.clone()).collect();
                    for (i, field) in r#type.fields.iter().enumerate() {
                        field_types.insert((r#type.name.clone(), field.name.clone()), i);
                    }
                    store_field_names.insert(r#type.name.clone(), names);
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
                    // Non-actor store: register constructor make_StoreName() -> Store(name)
                    let ctor_name = format!("make_{}", store.name);
                    types.insert(
                        ctor_name,
                        TypeId::Func(vec![], Box::new(TypeId::Store(store.name.clone()))),
                    );
                }
                // Track store field names for member access type inference (TS-4)
                {
                    let names: Vec<String> = store.fields.iter().map(|f| f.name.clone()).collect();
                    for (i, field) in store.fields.iter().enumerate() {
                        field_types.insert((store.name.clone(), field.name.clone()), i);
                    }
                    store_field_names.insert(store.name.clone(), names);
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
            Item::Extension(ext) => {
                // S4.5: Merge extension methods into the target type/store.
                // Extension methods have lower priority — only add if not already defined.
                let target = &ext.target_type;
                let mut merged = false;

                // Try to merge into a store definition
                for store in stores.iter_mut() {
                    if store.name == *target {
                        for method in &ext.methods {
                            let already_exists = store.methods.iter().any(|m| m.name == method.name);
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

                // Try to merge into a type definition
                if !merged {
                    for type_def in type_defs.iter_mut() {
                        if type_def.name == *target {
                            for method in &ext.methods {
                                let already_exists = type_def.methods.iter().any(|m| m.name == method.name);
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

                // For built-in types (String, List, Map, Int, Float, Bool),
                // create a synthetic store entry so codegen registers the methods.
                if !merged {
                    let builtin_types = ["String", "List", "Map", "Int", "Float", "Bool", "Number", "Bytes"];
                    if builtin_types.contains(&target.as_str()) {
                        let methods: Vec<Function> = ext.methods.iter().map(|m| {
                            let mut func = m.clone();
                            func.kind = FunctionKind::Method;
                            func
                        }).collect();
                        let synthetic = crate::ast::StoreDefinition {
                            name: target.clone(),
                            with_traits: vec![],
                            fields: vec![],
                            methods,
                            is_actor: false,
                            is_persistent: false,
                            span: ext.span,
                        };
                        stores.push(synthetic);
                        merged = true;
                    }
                }

                // If target type not found, silently skip (warning deferred)
                if !merged {
                    // Store for later warning (after warnings vec is initialized)
                    // For now, silently ignore — the type may be from a module
                }
            }
        }
    }
    let mut constraints = ConstraintSet::default();
    let mut graph = TypeGraph::default();
    // T4.4: Collect branch type pairs for If/elif/else to check consistency post-solving
    let mut branch_type_hints: Vec<(Vec<TypeId>, Span)> = Vec::new();
    collect_program_constraints(&globals, &functions, &mut constraints, &mut types, &mut graph, &mut branch_type_hints);
    if let Err(errors) = crate::types::solve_constraints(&constraints, &mut graph) {
        // T4.1 + T4.2: Emit one diagnostic per type error with provenance.
        let first = &errors[0];
        let mut msg = format!("type inference failed: {}", first.message);
        // T4.2: Append provenance info if available
        if let Some(ref origin) = first.expected_origin {
            msg.push_str(&format!("\n  {} inferred from: {}",
                first.expected.as_ref().map(|t| crate::types::format_type(t)).unwrap_or_default(),
                origin.description));
        }
        if let Some(ref origin) = first.found_origin {
            msg.push_str(&format!("\n  {} required by: {}",
                first.found.as_ref().map(|t| crate::types::format_type(t)).unwrap_or_default(),
                origin.description));
        }
        let mut primary = Diagnostic::new(msg, first.span);
        // Attach remaining errors as related diagnostics
        for error in errors.iter().skip(1) {
            let mut related_msg = error.message.clone();
            if let Some(ref origin) = error.expected_origin {
                related_msg.push_str(&format!("\n  {} inferred from: {}",
                    error.expected.as_ref().map(|t| crate::types::format_type(t)).unwrap_or_default(),
                    origin.description));
            }
            if let Some(ref origin) = error.found_origin {
                related_msg.push_str(&format!("\n  {} required by: {}",
                    error.found.as_ref().map(|t| crate::types::format_type(t)).unwrap_or_default(),
                    origin.description));
            }
            primary.related.push(Diagnostic::new(
                related_msg,
                error.span,
            ));
        }
        return Err(primary);
    }
    // Resolve types after solving for easier diagnostics downstream.
    let mut resolved = TypeEnv::default();
    for (name, ty) in types.iter_all() {
        let mut g = graph.clone();
        let r = crate::types::resolve(ty.clone(), &mut g);
        resolved.insert(name, r);
    }

    let (usage, mutability, allocation) = infer_mutability_and_usage(&globals, &functions);

    // Collect warnings for unhandled error values
    let mut warnings = Vec::new();

    // T1.1/T1.5: Warn on remaining Unknown types after solving.
    // Skip internal names (prefixed with $, $$, or containing ::) and builtins.
    for (name, ty) in resolved.iter_all() {
        if ty.contains_unknown()
            && !name.starts_with('$')
            && !name.contains("::")
            && !is_builtin_name(&name)
        {
            warnings.push(Diagnostic::categorized_warning(
                format!("type of `{}` could not be fully inferred (contains Unknown)", name),
                Span::new(0, 0),
                WarningCategory::General,
            ));
        }
    }

    // Check match exhaustiveness after type definitions are collected (TS-9: warnings, not errors)
    check_all_match_exhaustiveness(&globals, &functions, &type_defs, &mut warnings);
    check_unhandled_errors(&globals, &functions, &mut warnings);
    // T3.5: Dead code detection — warn on statements after return/break/continue
    check_dead_code(&functions, &mut warnings);
    // T3.2: Definite assignment analysis — warn on variables that may be uninitialized
    check_definite_assignment(&functions, &mut warnings);
    // T4.4: Check branch type consistency for if/elif/else
    check_branch_type_consistency(&branch_type_hints, &mut graph, &mut warnings);

    // CC5.2/S6: Check member access validity on known store/type fields
    check_member_access_validity(&globals, &functions, &store_field_names, &mut warnings);

    // T3.3: Nullability tracking — warn on functions that may return none on some paths
    check_nullability_returns(&functions, &mut warnings);

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
        field_types,
        store_field_names,
        module_exports,
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
    // Create a return-type TypeVar. Each `return expr` in the body will constrain
    // this to equal the expression's type, so the function's return type is correctly
    // inferred even when the body ends with a `return` statement (which has no
    // trailing block expression).
    let return_ty = TypeId::TypeVar(graph.fresh());
    let body_ty = collect_block_constraints(&function.body, constraints, types, graph, Some(&return_ty), branch_type_hints);
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
            crate::ast::Statement::ForRange { body, .. } => {
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
    branch_type_hints: &mut Vec<(Vec<TypeId>, Span)>,
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
            crate::ast::Statement::If { condition, body, elif_branches, else_body, span } => {
                let _ = collect_constraints_expr(condition, constraints, types, graph);
                let body_ty = collect_block_constraints(body, constraints, types, graph, return_ty, branch_type_hints);
                // T4.4: Collect branch types for consistency check
                if else_body.is_some() {
                    let mut branch_tys = vec![body_ty];
                    for (cond, blk) in elif_branches {
                        let _ = collect_constraints_expr(cond, constraints, types, graph);
                        let blk_ty = collect_block_constraints(blk, constraints, types, graph, return_ty, branch_type_hints);
                        branch_tys.push(blk_ty);
                    }
                    if let Some(else_blk) = else_body {
                        let else_ty = collect_block_constraints(else_blk, constraints, types, graph, return_ty, branch_type_hints);
                        branch_tys.push(else_ty);
                    }
                    branch_type_hints.push((branch_tys, *span));
                } else {
                    for (cond, blk) in elif_branches {
                        let _ = collect_constraints_expr(cond, constraints, types, graph);
                        let _ = collect_block_constraints(blk, constraints, types, graph, return_ty, branch_type_hints);
                    }
                }
            }
            crate::ast::Statement::While { condition, body, .. } => {
                let _ = collect_constraints_expr(condition, constraints, types, graph);
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty, branch_type_hints);
            }
            crate::ast::Statement::For { variable, iterable, body, span } => {
                let iterable_ty = collect_constraints_expr(iterable, constraints, types, graph);
                let elem_ty = TypeId::TypeVar(graph.fresh());
                constraints.push(ConstraintKind::IterableAt(iterable_ty, elem_ty.clone(), *span));
                types.insert(variable.clone(), elem_ty);
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty, branch_type_hints);
            }
            crate::ast::Statement::ForKV { key_var, value_var, iterable, body, span } => {
                let iterable_ty = collect_constraints_expr(iterable, constraints, types, graph);
                let elem_ty = TypeId::TypeVar(graph.fresh());
                constraints.push(ConstraintKind::IterableAt(iterable_ty, elem_ty.clone(), *span));
                types.insert(key_var.clone(), elem_ty.clone());
                types.insert(value_var.clone(), elem_ty);
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty, branch_type_hints);
            }
            crate::ast::Statement::ForRange { variable, start, end, step, body, .. } => {
                let _ = collect_constraints_expr(start, constraints, types, graph);
                let _ = collect_constraints_expr(end, constraints, types, graph);
                if let Some(s) = step {
                    let _ = collect_constraints_expr(s, constraints, types, graph);
                }
                types.insert(variable.clone(), TypeId::Primitive(Primitive::Float));
                let _ = collect_block_constraints(body, constraints, types, graph, return_ty, branch_type_hints);
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
        Expression::None(_) => TypeId::Primitive(Primitive::None),  // none is absent (distinct from Unit)
        Expression::InlineAsm { .. } => TypeId::Unknown,
        Expression::PtrLoad { .. } => TypeId::Unknown,
        Expression::Unsafe { .. } => TypeId::Unknown,
        Expression::Spread(inner, _) => {
            let inner_ty = collect_constraints_expr(inner, constraints, types, graph);
            inner_ty
        }
        Expression::Identifier(name, span) => {
            // T2.2: Let-polymorphism for generic constructors.
            // Each use of a generic constructor (e.g., None, Some) gets fresh type vars.
            if let Some((enum_name, type_params, field_count)) = types.get_generic_constructor(name).cloned() {
                // Create fresh type variables for each type parameter
                let fresh_args: Vec<TypeId> = type_params.iter()
                    .map(|_| TypeId::TypeVar(graph.fresh()))
                    .collect();
                // T2.4: Emit HasTrait constraints for bounded type parameters
                for (param_name, fresh_ty) in type_params.iter().zip(fresh_args.iter()) {
                    if let Some(bounds) = types.get_type_param_bounds(&enum_name, param_name) {
                        for bound in bounds.clone() {
                            constraints.push(ConstraintKind::HasTrait(fresh_ty.clone(), bound, *span));
                        }
                    }
                }
                let adt_ty = TypeId::Adt(enum_name.clone(), fresh_args.clone());
                if field_count == 0 {
                    // Nullary constructor: return ADT directly
                    adt_ty
                } else {
                    // N-ary constructor: return Func(fresh_vars...) -> ADT[fresh_vars...]
                    let param_types: Vec<TypeId> = (0..field_count)
                        .map(|_| TypeId::TypeVar(graph.fresh()))
                        .collect();
                    TypeId::Func(param_types, Box::new(adt_ty))
                }
            } else {
                types
                    .get(name)
                    .cloned()
                    .unwrap_or(TypeId::Unknown)
            }
        }
        Expression::Placeholder(id, _) => TypeId::Placeholder(*id),
        Expression::TaxonomyPath { .. } => TypeId::Primitive(Primitive::String),
        Expression::List(items, span) => {
            let elem_ty = TypeId::TypeVar(graph.fresh());
            for item in items {
                let ty = collect_constraints_expr(item, constraints, types, graph);
                if matches!(item, Expression::Spread(..)) {
                    // S2.6: Spread element has list type — constrain it to [elem_ty]
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
        Expression::Call { callee, args, span, .. } => {
            // Special-case: if callee is a Member expression with a known method name,
            // bypass the CallableAt constraint (which would fail because e.g. "length" 
            // returns Int, not a callable type). Instead, directly return the method's
            // result type. This fixes `log(s.length())` and similar nested method calls.
            // S4.4: Return precise types for chainable methods to enable method chaining.
            if let Expression::Member { target, property, .. } = callee.as_ref() {
                let target_ty = collect_constraints_expr(target, constraints, types, graph);
                // Collect arg constraints regardless
                for arg in args {
                    collect_constraints_expr(arg, constraints, types, graph);
                }
                match property.as_str() {
                    // Int-returning methods
                    "length" | "count" | "size" | "index_of" => return TypeId::Primitive(Primitive::Int),
                    // Bool-returning methods
                    "err" | "equals" | "not_equals" | "contains" | "any" | "all"
                    | "starts_with" | "ends_with" | "is_empty" => return TypeId::Primitive(Primitive::Bool),
                    // String-returning methods (chainable on strings)
                    "trim" | "lower" | "upper" | "strip" | "lstrip" | "rstrip"
                    | "replace" | "pad_left" | "pad_right" | "reverse" | "repeat"
                    | "to_string" | "join" | "slice" | "substr" | "char_at"
                    | "concat" => return TypeId::Primitive(Primitive::String),
                    // List-returning methods (chainable on lists)
                    "split" | "map" | "filter" | "sort" | "keys" | "values"
                    | "find_all" | "chars" | "lines" | "bytes" => {
                        return TypeId::List(Box::new(TypeId::Unknown));
                    }
                    // Methods that return same type as target (preserve chain type)
                    "push" | "pop" | "append" | "remove" | "insert" | "clear" => {
                        return target_ty;
                    }
                    // Unknown-returning (unresolvable without more context)
                    "get" | "set" | "at" | "reduce" | "find" | "not" | "iter"
                    | "or" | "unwrap_or" | "first" | "last" => {
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
        Expression::Slice { target, start, end, .. } => {
            let target_ty = collect_constraints_expr(target, constraints, types, graph);
            let _start_ty = collect_constraints_expr(start, constraints, types, graph);
            let _end_ty = collect_constraints_expr(end, constraints, types, graph);
            // Slice returns the same collection type
            target_ty
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
                // S4.4: Include all known method names to prevent false map-constraint fallthrough.
                "push" | "pop" | "get" | "set" | "append" | "remove" | "insert" 
                | "contains" | "keys" | "values" | "clear" | "join"
                | "map" | "filter" | "reduce" | "find" | "any" | "all" | "sort"
                | "equals" | "not_equals" | "not" | "iter"
                | "split" | "trim" | "lower" | "upper" | "strip" | "lstrip" | "rstrip"
                | "replace" | "pad_left" | "pad_right" | "reverse" | "repeat"
                | "starts_with" | "ends_with" | "index_of" | "is_empty"
                | "to_string" | "concat" | "chars" | "lines" | "bytes"
                | "slice" | "substr" | "char_at" | "find_all"
                | "or" | "unwrap_or" | "first" | "last" | "at" => {
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
                            TypeId::Primitive(Primitive::Any)
                        };
                        constraints.push(ConstraintKind::EqualAt(
                            scrutinee_ty.clone(),
                            TypeId::List(Box::new(elem_ty.clone())),
                            match_span,
                        ));
                        // S3.4: Recurse into list sub-patterns for bindings
                        for pat in patterns {
                            collect_pattern_bindings(pat, &elem_ty, types);
                        }
                    }
                    crate::ast::MatchPattern::Identifier(name) => {
                        types.insert(name.clone(), scrutinee_ty.clone());
                    }
                    crate::ast::MatchPattern::Constructor { name, fields, span: _ } => {
                        // T2.2: Let-polymorphism for generic constructors in match patterns.
                        // If the constructor belongs to a generic enum, instantiate fresh type vars
                        // and unify the fields with those vars.
                        if let Some((enum_name, type_params, _field_count)) = types.get_generic_constructor(name).cloned() {
                            let fresh_args: Vec<TypeId> = type_params.iter()
                                .map(|_| TypeId::TypeVar(graph.fresh()))
                                .collect();
                            // T2.4: Emit HasTrait constraints for bounded type parameters
                            for (param_name, fresh_ty) in type_params.iter().zip(fresh_args.iter()) {
                                if let Some(bounds) = types.get_type_param_bounds(&enum_name, param_name) {
                                    for bound in bounds.clone() {
                                        constraints.push(ConstraintKind::HasTrait(fresh_ty.clone(), bound, match_span));
                                    }
                                }
                            }
                            let adt_ty = TypeId::Adt(enum_name.clone(), fresh_args.clone());
                            constraints.push(ConstraintKind::EqualAt(
                                scrutinee_ty.clone(),
                                adt_ty,
                                match_span,
                            ));
                            // Bind pattern fields to fresh type vars
                            for pat in fields {
                                let field_ty = TypeId::TypeVar(graph.fresh());
                                collect_pattern_bindings(pat, &field_ty, types);
                            }
                        } else {
                            // Non-generic constructor: original behavior
                            // T3.1: Extract constructor parameter types for field narrowing.
                            let ctor_param_types: Option<Vec<TypeId>> = types.get(name).and_then(|ctor_ty| {
                                match ctor_ty {
                                    TypeId::Func(param_types, _) => Some(param_types.clone()),
                                    _ => None,
                                }
                            });

                            if let Some(ctor_ty) = types.get(name) {
                                let adt_ty = match ctor_ty {
                                    TypeId::Adt(adt_name, args) => TypeId::Adt(adt_name.clone(), args.clone()),
                                    TypeId::Func(_, ret) => (**ret).clone(),
                                    _ => TypeId::Primitive(Primitive::Any),
                                };
                                constraints.push(ConstraintKind::EqualAt(
                                    scrutinee_ty.clone(),
                                    adt_ty,
                                    match_span,
                                ));
                            }
                            // T3.1: Bind field variables to constructor parameter types
                            // (narrowed types) instead of Any when the constructor signature is known.
                            if let Some(ref param_types) = ctor_param_types {
                                for (i, pat) in fields.iter().enumerate() {
                                    let field_ty = param_types.get(i)
                                        .cloned()
                                        .unwrap_or(TypeId::Primitive(Primitive::Any));
                                    collect_pattern_bindings(pat, &field_ty, types);
                                }
                            } else {
                                for pat in fields {
                                    collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
                                }
                            }
                        }
                    }
                    crate::ast::MatchPattern::Wildcard(_) => {
                        // Wildcard matches anything, no type constraints
                    }
                    crate::ast::MatchPattern::Range { .. } => {
                        // S3.5: Range pattern constrains scrutinee to numeric
                        constraints.push(ConstraintKind::NumericAt(scrutinee_ty.clone(), match_span));
                    }
                    crate::ast::MatchPattern::Rest(name, _) => {
                        // S3.4: Rest pattern binds remaining list elements
                        types.insert(name.clone(), TypeId::List(Box::new(TypeId::Primitive(Primitive::Any))));
                    }
                    crate::ast::MatchPattern::Or(alternatives) => {
                        // Or-pattern: collect constraints from each alternative
                        for alt in alternatives {
                            match alt {
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
                                crate::ast::MatchPattern::Identifier(name) => {
                                    types.insert(name.clone(), scrutinee_ty.clone());
                                }
                                crate::ast::MatchPattern::Constructor { name, fields, .. } => {
                                    if let Some((enum_name, type_params, _field_count)) = types.get_generic_constructor(name).cloned() {
                                        let fresh_args: Vec<TypeId> = type_params.iter()
                                            .map(|_| TypeId::TypeVar(graph.fresh()))
                                            .collect();
                                        // T2.4: Emit HasTrait constraints for bounded type parameters
                                        for (param_name, fresh_ty) in type_params.iter().zip(fresh_args.iter()) {
                                            if let Some(bounds) = types.get_type_param_bounds(&enum_name, param_name) {
                                                for bound in bounds.clone() {
                                                    constraints.push(ConstraintKind::HasTrait(fresh_ty.clone(), bound, match_span));
                                                }
                                            }
                                        }
                                        let adt_ty = TypeId::Adt(enum_name.clone(), fresh_args.clone());
                                        constraints.push(ConstraintKind::EqualAt(scrutinee_ty.clone(), adt_ty, match_span));
                                    }
                                    // T3.1: Narrow or-pattern constructor fields too
                                    let ctor_param_types: Option<Vec<TypeId>> = types.get(name).and_then(|ctor_ty| {
                                        match ctor_ty {
                                            TypeId::Func(param_types, _) => Some(param_types.clone()),
                                            _ => None,
                                        }
                                    });
                                    if let Some(ref param_types) = ctor_param_types {
                                        for (i, pat) in fields.iter().enumerate() {
                                            let field_ty = param_types.get(i)
                                                .cloned()
                                                .unwrap_or(TypeId::Primitive(Primitive::Any));
                                            collect_pattern_bindings(pat, &field_ty, types);
                                        }
                                    } else {
                                        for pat in fields {
                                            collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
                                        }
                                    }
                                }
                                crate::ast::MatchPattern::Range { .. } => {
                                    // S3.5: Range in or-pattern constrains scrutinee to numeric
                                    constraints.push(ConstraintKind::NumericAt(scrutinee_ty.clone(), match_span));
                                }
                                crate::ast::MatchPattern::Rest(name, _) => {
                                    types.insert(name.clone(), TypeId::List(Box::new(TypeId::Primitive(Primitive::Any))));
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
                // S3.2: Collect constraints from guard expression
                if let Some(guard) = &arm.guard {
                    collect_constraints_expr(guard, constraints, types, graph);
                }
                let arm_ty = collect_block_constraints(&arm.body, constraints, types, graph, None, &mut Vec::new());
                arm_tys.push(arm_ty);
            }
            if let Some(default) = &match_expr.default {
                arm_tys.push(collect_block_constraints(default, constraints, types, graph, None, &mut Vec::new()));
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
            let body_ty = collect_block_constraints(body, &mut nested_constraints, &mut shadow, &mut nested_graph, None, &mut Vec::new());
            constraints.constraints.extend(nested_constraints.constraints);
            TypeId::Func(param_tys, Box::new(body_ty))
        }
        Expression::Pipeline { left, right, span } => {
            // Pipeline `a ~ f(args)` desugars to `f(a, args)`
            // Handle the right side based on its form:
            match right.as_ref() {
                Expression::Call { callee, args, span: call_span, .. } => {
                    // a ~ f(args) desugars to f(a, args)
                    // Build a desugared call with left prepended to args
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
                    // a ~ f desugars to f(a)
                    let desugared = Expression::Call {
                        callee: right.clone(),
                        args: vec![*left.clone()],
                        arg_names: vec![],
                        span: *span,
                    };
                    collect_constraints_expr(&desugared, constraints, types, graph)
                }
                _ => {
                    // CC5.2/S8: For any other expression on the right, treat as callable
                    // with left as argument — emit proper callable constraint
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
            }
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
        Expression::ListComprehension { body, var, iterable, condition, span } => {
            let iter_ty = collect_constraints_expr(iterable, constraints, types, graph);
            let elem_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::EqualAt(
                TypeId::List(Box::new(elem_ty.clone())),
                iter_ty,
                *span,
            ));
            types.insert(var.clone(), elem_ty);
            if let Some(cond) = condition {
                collect_constraints_expr(cond, constraints, types, graph);
            }
            let body_ty = collect_constraints_expr(body, constraints, types, graph);
            TypeId::List(Box::new(body_ty))
        }
        Expression::MapComprehension { key, value, var, iterable, condition, span } => {
            let iter_ty = collect_constraints_expr(iterable, constraints, types, graph);
            let elem_ty = TypeId::TypeVar(graph.fresh());
            constraints.push(ConstraintKind::EqualAt(
                TypeId::List(Box::new(elem_ty.clone())),
                iter_ty,
                *span,
            ));
            types.insert(var.clone(), elem_ty);
            if let Some(cond) = condition {
                collect_constraints_expr(cond, constraints, types, graph);
            }
            let key_ty = collect_constraints_expr(key, constraints, types, graph);
            let _val_ty = collect_constraints_expr(value, constraints, types, graph);
            TypeId::Map(Box::new(key_ty), Box::new(TypeId::Primitive(Primitive::Any)))
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
        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                collect_pattern_bindings(alt, ty, types);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            // S3.4: Recurse into list sub-patterns
            for pat in patterns {
                collect_pattern_bindings(pat, &TypeId::Primitive(Primitive::Any), types);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            // S3.4: Rest captures remaining elements as a list
            types.insert(name.clone(), TypeId::List(Box::new(TypeId::Primitive(Primitive::Any))));
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
        // T2.3: Handle user-defined generic types like Option[Int], Result[T, E], etc.
        name => {
            if !ann.type_args.is_empty() {
                let type_args: Vec<TypeId> = ann.type_args.iter()
                    .map(|a| type_from_annotation(a))
                    .collect();
                TypeId::Adt(name.to_string(), type_args)
            } else {
                // Could be a non-generic ADT or truly unknown
                TypeId::Adt(name.to_string(), vec![])
            }
        },
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
            Statement::ForKV { key_var, value_var, iterable, body, span } => {
                check_expression(iterable, scopes, known_names)?;
                scopes.push();
                scopes.declare(key_var.clone(), *span);
                scopes.declare(value_var.clone(), *span);
                check_block(body, scopes, known_names)?;
                scopes.pop();
            }
            Statement::ForRange { variable, start, end, step, body, span } => {
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
            Statement::PatternBinding { pattern, value, span } => {
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

/// Declare variable names introduced by a destructuring pattern into the scope stack.
fn declare_pattern_scope_names(pattern: &crate::ast::MatchPattern, scopes: &mut ScopeStack, span: Span) {
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
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
    }
}

fn check_expression(expr: &Expression, scopes: &mut ScopeStack, known_functions: &HashSet<String>) -> Result<(), Diagnostic> {
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
        Expression::Slice { target, start, end, .. } => {
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
        Expression::ListComprehension { body, var, iterable, condition, .. } => {
            check_expression(iterable, scopes, known_functions)?;
            scopes.push();
            scopes.declare(var.clone(), iterable.span());
            check_expression(body, scopes, known_functions)?;
            if let Some(cond) = condition {
                check_expression(cond, scopes, known_functions)?;
            }
            scopes.pop();
        }
        Expression::MapComprehension { key, value, var, iterable, condition, .. } => {
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
        "string_length" |
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
        "map_remove" | "map_values" | "map_keys" | "map_entries" | "entries" |
        "map_has_key" | "has_key" | "map_merge" | "merge" |
        // Type introspection
        "type_of" |
        // Error handling builtins
        "is_err" | "is_ok" | "is_absent" | "error_name" | "error_code" |
        // Character operations
        "ord" | "string_ord" | "chr" | "string_chr" | "string_compare" | "strcmp" |
        // Actor operations (runtime FFI)
        "actor_spawn" | "actor_send" | "actor_stop" | "actor_self" |
        "actor_monitor" | "monitor" | "actor_demonitor" | "demonitor" |
        "actor_graceful_stop" | "graceful_stop" |
        // JSON operations
        "json_parse" | "json_serialize" | "json_stringify" | "json_serialize_pretty" |
        // Time operations
        "time_now" | "time_timestamp" | "time_format_iso" |
        "time_year" | "time_month" | "time_day" |
        "time_hour" | "time_minute" | "time_second" |
        // Sleep (L2.3)
        "time_sleep" |
        // Random operations (L2.1)
        "random" | "random_int" | "random_seed" |
        // String extended
        "string_lines" |
        // Sort operations
        "sort_natural" | "list_sort_natural" |
        // Bytes extended
        "bytes_from_hex" | "bytes_contains" | "bytes_find" |
        // Encoding operations
        "base64_encode" | "base64_decode" |
        "hex_encode" | "hex_decode" |
        // TCP networking
        "tcp_listen" | "tcp_accept" | "tcp_connect" |
        "tcp_read" | "tcp_write" | "tcp_close" |
        // Memory operations (runtime FFI)
        "value_retain" | "value_release" | "heap_alloc" | "heap_free" |
        // Range helper
        "range" |
        // StringBuilder / optimized string ops (L1.1)
        "sb_new" | "sb_push" | "sb_finish" | "sb_len" |
        "string_join_list" | "join_list" |
        "string_repeat" | "repeat_string" |
        "string_reverse" | "reverse_string" |
        "value_to_string" |
        // L2.4: std.io enhancements
        "stderr_write" | "eprint" |
        "fs_size" | "file_size" |
        "fs_rename" | "fs_copy" |
        "fs_mkdirs" | "make_dirs" |
        "fs_temp_dir" | "temp_dir" |
        // L2.5: std.process enhancements
        "process_exec" | "exec" |
        "process_cwd" | "cwd" |
        "process_chdir" | "chdir" |
        "process_pid" |
        "process_hostname" | "hostname" |
        // L4.2: std.path operations
        "path_normalize" | "normalize" |
        "path_resolve" | "resolve" |
        "path_is_absolute" | "is_absolute" |
        "path_parent" |
        "path_stem" | "stem" |
        // Regex operations (L2.2)
        "regex_match" | "regex_find" | "regex_find_all" |
        "regex_replace" | "regex_split"
    )
}

fn check_match_expression(expr: &MatchExpression, scopes: &mut ScopeStack, known_names: &HashSet<String>) -> Result<(), Diagnostic> {
    check_expression(&expr.value, scopes, known_names)?;
    for arm in &expr.arms {
        // Match arms can introduce bindings via patterns
        scopes.push();
        declare_pattern_bindings(&arm.pattern, scopes, arm.body.span);
        // S3.2: Check guard expression in the arm's scope (pattern vars available)
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
        crate::ast::MatchPattern::Or(alternatives) => {
            for alt in alternatives {
                declare_pattern_bindings(alt, scopes, span);
            }
        }
        crate::ast::MatchPattern::List(patterns) => {
            // S3.4: Recurse into list sub-patterns
            for field_pattern in patterns {
                declare_pattern_bindings(field_pattern, scopes, span);
            }
        }
        crate::ast::MatchPattern::Rest(name, _) => {
            scopes.declare(name.clone(), span);
        }
        crate::ast::MatchPattern::Integer(_)
        | crate::ast::MatchPattern::Bool(_)
        | crate::ast::MatchPattern::String(_)
        | crate::ast::MatchPattern::Wildcard(_)
        | crate::ast::MatchPattern::Range { .. } => {}
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
    warnings: &mut Vec<Diagnostic>,
) {
    let ctor_map = build_constructor_map(type_defs);
    // Also build a map from enum name → list of variant names (for nested checking)
    let enum_variants: HashMap<String, Vec<String>> = type_defs.iter()
        .filter(|td| !td.variants.is_empty())
        .map(|td| (td.name.clone(), td.variants.iter().map(|v| v.name.clone()).collect()))
        .collect();
    
    // Check globals
    for binding in globals {
        check_expr_match_exhaustiveness(&binding.value, &ctor_map, &enum_variants, warnings);
    }
    
    // Check functions
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
            Statement::If { condition, body, elif_branches, else_body, .. } => {
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
            Statement::While { condition, body, .. } => {
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
            Statement::ForRange { start, end, step, body, .. } => {
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
        Expression::Slice { target, start, end, .. } => {
            check_expr_match_exhaustiveness(target, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(start, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(end, ctor_map, enum_variants, warnings);
        }
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            check_expr_match_exhaustiveness(condition, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(then_branch, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(else_branch, ctor_map, enum_variants, warnings);
        }
        Expression::Match(match_expr) => {
            // Recursively check the matched value and arm bodies
            check_expr_match_exhaustiveness(&match_expr.value, ctor_map, enum_variants, warnings);
            for arm in &match_expr.arms {
                check_block_match_exhaustiveness(&arm.body, ctor_map, enum_variants, warnings);
            }
            if let Some(default) = &match_expr.default {
                check_block_match_exhaustiveness(default, ctor_map, enum_variants, warnings);
            }
            
            // Now check exhaustiveness (emits warnings)
            check_single_match_exhaustiveness(match_expr, ctor_map, enum_variants, warnings);
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
        Expression::ErrorValue { .. } => {
            // Error values don't contain sub-expressions
        }
        Expression::ErrorPropagate { expr, .. } => {
            check_expr_match_exhaustiveness(expr, ctor_map, enum_variants, warnings);
        }
        Expression::ListComprehension { body, iterable, condition, .. } => {
            check_expr_match_exhaustiveness(iterable, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(body, ctor_map, enum_variants, warnings);
            if let Some(cond) = condition {
                check_expr_match_exhaustiveness(cond, ctor_map, enum_variants, warnings);
            }
        }
        Expression::MapComprehension { key, value, iterable, condition, .. } => {
            check_expr_match_exhaustiveness(iterable, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(key, ctor_map, enum_variants, warnings);
            check_expr_match_exhaustiveness(value, ctor_map, enum_variants, warnings);
            if let Some(cond) = condition {
                check_expr_match_exhaustiveness(cond, ctor_map, enum_variants, warnings);
            }
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
}

/// Recursively collect constructor names, wildcards, and identifiers from a pattern
/// for exhaustiveness checking.  Handles Or-patterns by recursing into alternatives.
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
                collect_pattern_ctors_for_exhaustiveness(alt, matched_ctors, has_wildcard, has_identifier_catch_all);
            }
        }
        // Literals don't help with enum exhaustiveness
        _ => {}
    }
}

/// Check a single match expression for exhaustiveness (TS-9: warnings + nested ADT checking)
fn check_single_match_exhaustiveness(
    match_expr: &MatchExpression,
    ctor_map: &HashMap<String, (String, Vec<String>)>,
    enum_variants: &HashMap<String, Vec<String>>,
    warnings: &mut Vec<Diagnostic>,
) {
    // If there's a default block, the match is exhaustive
    if match_expr.default.is_some() {
        return;
    }
    
    // Collect all matched constructors and check for wildcards
    let mut matched_ctors: HashSet<String> = HashSet::new();
    let mut has_wildcard = false;
    let mut has_identifier_catch_all = false;
    
    for arm in &match_expr.arms {
        collect_pattern_ctors_for_exhaustiveness(&arm.pattern, &mut matched_ctors, &mut has_wildcard, &mut has_identifier_catch_all);
    }
    
    // If there's a wildcard or identifier catch-all, match is exhaustive
    if has_wildcard || has_identifier_catch_all {
        return;
    }
    
    // If no constructor patterns, we can't determine exhaustiveness 
    // (matching on literals or unknown types)
    if matched_ctors.is_empty() {
        return;
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
                    return;
                }
            }
        }
        
        // Check which variants are missing at the top level
        let missing: Vec<&String> = all_variants.iter()
            .filter(|v| !matched_ctors.contains(*v))
            .collect();
        
        if !missing.is_empty() {
            let missing_list = missing.iter()
                .map(|s| format!("`{}`", s))
                .collect::<Vec<_>>()
                .join(", ");
            
            warnings.push(
                Diagnostic::new(
                    format!("non-exhaustive match: missing pattern(s) for {}", missing_list),
                    match_expr.span,
                ).with_help(
                    "add arm(s) for the missing variant(s) or add a default arm".to_string()
                )
            );
            return; // Don't check nested if top-level is already non-exhaustive
        }
        
        // TS-9: Check nested pattern exhaustiveness for each constructor
        // Group arms by their top-level constructor, then check sub-patterns
        check_nested_exhaustiveness(&match_expr.arms, ctor_map, enum_variants, match_expr.span, warnings);
    }
}

/// TS-9: Check nested pattern exhaustiveness within each constructor group.
/// For example, in `match x` with arms `Some(Some(v)) ? ...` and `Some(None) ? ...` and `None ? ...`,
/// the `Some` arms have sub-patterns on an inner `Option` type; we check those are exhaustive.
fn check_nested_exhaustiveness(
    arms: &[crate::ast::MatchArm],
    ctor_map: &HashMap<String, (String, Vec<String>)>,
    enum_variants: &HashMap<String, Vec<String>>,
    span: Span,
    warnings: &mut Vec<Diagnostic>,
) {
    // Group arms by top-level constructor name
    let mut groups: HashMap<String, Vec<&[crate::ast::MatchPattern]>> = HashMap::new();
    
    for arm in arms {
        if let crate::ast::MatchPattern::Constructor { name, fields, .. } = &arm.pattern {
            groups.entry(name.clone()).or_default().push(fields);
        }
    }
    
    // For each constructor group, check sub-pattern exhaustiveness at each field position
    for (ctor_name, field_groups) in &groups {
        if field_groups.is_empty() {
            continue;
        }
        
        // Determine how many fields this constructor has
        let max_fields = field_groups.iter().map(|f| f.len()).max().unwrap_or(0);
        
        for field_idx in 0..max_fields {
            // Collect the sub-patterns at this field position
            let mut sub_ctors: HashSet<String> = HashSet::new();
            let mut has_catch_all = false;
            
            for fields in field_groups {
                if field_idx < fields.len() {
                    match &fields[field_idx] {
                        crate::ast::MatchPattern::Constructor { name, .. } => {
                            sub_ctors.insert(name.clone());
                        }
                        crate::ast::MatchPattern::Identifier(_) | crate::ast::MatchPattern::Wildcard(_) => {
                            has_catch_all = true;
                        }
                        _ => {
                            // Literal patterns don't contribute to constructor exhaustiveness
                        }
                    }
                } else {
                    // If this arm has fewer fields, it implicitly catches all
                    has_catch_all = true;
                }
            }
            
            // If there's a catch-all, this position is exhaustive
            if has_catch_all || sub_ctors.is_empty() {
                continue;
            }
            
            // All sub-patterns are constructors — check if they cover all variants of their enum
            let first_sub = sub_ctors.iter().next().unwrap();
            if let Some((sub_enum_name, sub_all_variants)) = ctor_map.get(first_sub) {
                let sub_missing: Vec<&String> = sub_all_variants.iter()
                    .filter(|v| !sub_ctors.contains(*v))
                    .collect();
                
                if !sub_missing.is_empty() {
                    let missing_list = sub_missing.iter()
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
            Statement::ForKV { key_var, value_var, iterable, body, .. } => {
                visit_expression(iterable, tracker);
                tracker.touch(key_var);
                tracker.touch(value_var);
                visit_block(body, tracker, mark_returns_as_escape);
            }
            Statement::ForRange { start, end, step, body, variable, .. } => {
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
            mark_escapes(value, tracker);
        }
    }
}

/// Touch variable names introduced by a destructuring pattern in the usage tracker.
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
        Expression::Slice { target, start, end, .. } => {
            visit_expression(target, tracker);
            visit_expression(start, tracker);
            visit_expression(end, tracker);
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
        Expression::ListComprehension { body, var, iterable, condition, .. } => {
            visit_expression(iterable, tracker);
            tracker.touch(var);
            visit_expression(body, tracker);
            if let Some(cond) = condition {
                visit_expression(cond, tracker);
            }
        }
        Expression::MapComprehension { key, value, var, iterable, condition, .. } => {
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
        Expression::Slice { target, start, end, .. } => {
            mark_escapes(target, tracker);
            mark_escapes(start, tracker);
            mark_escapes(end, tracker);
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
        Expression::ListComprehension { body, iterable, condition, .. } => {
            mark_escapes(iterable, tracker);
            mark_escapes(body, tracker);
            if let Some(cond) = condition {
                mark_escapes(cond, tracker);
            }
        }
        Expression::MapComprehension { key, value, iterable, condition, .. } => {
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
        Expression::Spread(inner, _) => find_forbidden_identifier(inner, forbidden),
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
        Expression::Slice { target, start, end, .. } => {
            find_forbidden_identifier(target, forbidden)
                .or_else(|| find_forbidden_identifier(start, forbidden))
                .or_else(|| find_forbidden_identifier(end, forbidden))
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
        Expression::ListComprehension { body, iterable, condition, .. } => {
            find_forbidden_identifier(iterable, forbidden)
                .or_else(|| find_forbidden_identifier(body, forbidden))
                .or_else(|| condition.as_ref().and_then(|c| find_forbidden_identifier(c, forbidden)))
        }
        Expression::MapComprehension { key, value, iterable, condition, .. } => {
            find_forbidden_identifier(iterable, forbidden)
                .or_else(|| find_forbidden_identifier(key, forbidden))
                .or_else(|| find_forbidden_identifier(value, forbidden))
                .or_else(|| condition.as_ref().and_then(|c| find_forbidden_identifier(c, forbidden)))
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
        Statement::ForKV { iterable, body, .. } => {
            find_forbidden_identifier(iterable, forbidden)
                .or_else(|| find_in_block(body, forbidden))
        }
        Statement::ForRange { start, end, step, body, .. } => {
            find_forbidden_identifier(start, forbidden)
                .or_else(|| find_forbidden_identifier(end, forbidden))
                .or_else(|| step.as_ref().and_then(|s| find_forbidden_identifier(s, forbidden)))
                .or_else(|| find_in_block(body, forbidden))
        }
        Statement::Break(_) | Statement::Continue(_) => None,
        Statement::FieldAssign { target, value, .. } => {
            find_forbidden_identifier(target, forbidden)
                .or_else(|| find_forbidden_identifier(value, forbidden))
        }
        Statement::PatternBinding { value, .. } => {
            find_forbidden_identifier(value, forbidden)
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
            Statement::ForKV { iterable, body, .. } => {
                check_expr_nested_blocks(iterable, warnings);
                check_block_for_unhandled_errors(body, warnings);
            }
            Statement::ForRange { start, end, step, body, .. } => {
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
        Expression::Slice { target, start, end, .. } => {
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
        Expression::ListComprehension { body, iterable, condition, .. } => {
            check_expr_nested_blocks(iterable, warnings);
            check_expr_nested_blocks(body, warnings);
            if let Some(cond) = condition {
                check_expr_nested_blocks(cond, warnings);
            }
        }
        Expression::MapComprehension { key, value, iterable, condition, .. } => {
            check_expr_nested_blocks(iterable, warnings);
            check_expr_nested_blocks(key, warnings);
            check_expr_nested_blocks(value, warnings);
            if let Some(cond) = condition {
                check_expr_nested_blocks(cond, warnings);
            }
        }
    }
}

// ── T3.5: Dead code detection ──────────────────────────────────────────────

/// Get the span of a statement (for warning locations).
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

/// T4.4: Check that if/elif/else branches return consistent types.
/// Emits warnings (not errors) when branch types differ.
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
        // Resolve all branch types
        let resolved: Vec<TypeId> = branch_tys
            .iter()
            .map(|ty| resolve(ty.clone(), graph))
            .collect();

        // Check if all resolved types are the same (ignoring Unknown/Any/None)
        let first = &resolved[0];
        let mut mismatch = false;
        for ty in &resolved[1..] {
            if !types_compatible_for_branch(first, ty) {
                mismatch = true;
                break;
            }
        }

        if mismatch {
            let type_names: Vec<String> = resolved
                .iter()
                .map(|t| format!("{:?}", t))
                .collect();
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

/// Check if two types are compatible for branch consistency.
/// Permissive: Unknown, Any, None are compatible with anything.
/// TypeVars are compatible with anything (not yet resolved).
fn types_compatible_for_branch(a: &TypeId, b: &TypeId) -> bool {
    use crate::types::core::Primitive;
    match (a, b) {
        // Wildcard types are always compatible
        (TypeId::Unknown, _) | (_, TypeId::Unknown) => true,
        (TypeId::Primitive(Primitive::Any), _) | (_, TypeId::Primitive(Primitive::Any)) => true,
        (TypeId::Primitive(Primitive::None), _) | (_, TypeId::Primitive(Primitive::None)) => true,
        (TypeId::TypeVar(_), _) | (_, TypeId::TypeVar(_)) => true,
        // Int and Float are compatible (numeric promotion)
        (TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::Float)) => true,
        (TypeId::Primitive(Primitive::Float), TypeId::Primitive(Primitive::Int)) => true,
        // Same type
        _ => a == b,
    }
}

// ====================== T3.3: Nullability Tracking ======================

/// Check functions whose return paths include both `none` and a non-none type.
/// Emits a warning when a function may implicitly return `none` alongside a
/// concrete type, indicating potential nullability issues.
fn check_nullability_returns(
    functions: &[Function],
    warnings: &mut Vec<Diagnostic>,
) {
    for function in functions {
        // Skip main — it's entry point, no meaningful return
        if function.name == "main" {
            continue;
        }
        let mut has_none_return = false;
        let mut has_value_return = false;
        let mut none_span: Option<Span> = None;

        // Check trailing block value
        if let Some(ref val) = function.body.value {
            if expr_is_none(val) {
                has_none_return = true;
                none_span = Some(val.span());
            } else {
                has_value_return = true;
            }
        }

        // Walk body for return statements
        collect_return_nullability(
            &function.body,
            &mut has_none_return,
            &mut has_value_return,
            &mut none_span,
        );

        // If function has no explicit returns and no trailing value, it returns
        // unit implicitly — that's fine, not a nullability issue.
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

/// Walk a block's statements recursively to find return statements and
/// classify them as none-returning or value-returning.
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
            crate::ast::Statement::If { body, elif_branches, else_body, .. } => {
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

/// Check if an expression is the `none` literal.
fn expr_is_none(expr: &Expression) -> bool {
    matches!(expr, Expression::None(_))
}

// ====================== T3.2: Definite Assignment Analysis ======================

/// Check function bodies for variables that may be used before being
/// definitely assigned on all execution paths.
fn check_definite_assignment(
    functions: &[Function],
    warnings: &mut Vec<Diagnostic>,
) {
    for function in functions {
        let mut definitely_assigned: HashSet<String> = HashSet::new();
        // Parameters are always assigned
        for param in &function.params {
            definitely_assigned.insert(param.name.clone());
        }
        da_check_block(&function.body, &mut definitely_assigned, warnings);
    }
}

/// Walk a block collecting definite assignments and checking uses.
fn da_check_block(
    block: &Block,
    assigned: &mut HashSet<String>,
    warnings: &mut Vec<Diagnostic>,
) {
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
            // Check the RHS for uses first
            da_check_expression_uses(&binding.value, assigned, warnings);
            // Then mark the LHS as assigned
            assigned.insert(binding.name.clone());
        }
        Statement::Expression(expr) => {
            da_check_expression_uses(expr, assigned, warnings);
        }
        Statement::Return(expr, _) => {
            da_check_expression_uses(expr, assigned, warnings);
        }
        Statement::If { condition, body, elif_branches, else_body, .. } => {
            da_check_expression_uses(condition, assigned, warnings);
            // Compute names assigned in each branch
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

                // Only if all branches (including else) exist:
                // intersect to find names assigned on ALL paths
                if !all_branches_assigned.is_empty() {
                    let intersection: HashSet<String> = all_branches_assigned[0]
                        .iter()
                        .filter(|name| all_branches_assigned.iter().all(|s| s.contains(*name)))
                        .cloned()
                        .collect();
                    *assigned = intersection;
                }
            }
            // Without else: no new names are definitely assigned
        }
        Statement::For { variable, iterable, body, .. } => {
            da_check_expression_uses(iterable, assigned, warnings);
            let mut loop_assigned = assigned.clone();
            loop_assigned.insert(variable.clone());
            da_check_block(body, &mut loop_assigned, warnings);
            // After for: loop variable is not definitely assigned (loop may not execute)
        }
        Statement::ForRange { variable, start, end, step, body, .. } => {
            da_check_expression_uses(start, assigned, warnings);
            da_check_expression_uses(end, assigned, warnings);
            if let Some(s) = step {
                da_check_expression_uses(s, assigned, warnings);
            }
            let mut loop_assigned = assigned.clone();
            loop_assigned.insert(variable.clone());
            da_check_block(body, &mut loop_assigned, warnings);
        }
        Statement::ForKV { key_var, value_var, iterable, body, .. } => {
            da_check_expression_uses(iterable, assigned, warnings);
            let mut loop_assigned = assigned.clone();
            loop_assigned.insert(key_var.clone());
            loop_assigned.insert(value_var.clone());
            da_check_block(body, &mut loop_assigned, warnings);
        }
        Statement::While { condition, body, .. } => {
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

/// Check an expression for uses of potentially-uninitialized variables.
fn da_check_expression_uses(
    expr: &Expression,
    assigned: &HashSet<String>,
    warnings: &mut Vec<Diagnostic>,
) {
    match expr {
        Expression::Identifier(name, span) => {
            // Only warn for simple lowercase names (not builtins, not type constructors)
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
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
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
        Expression::Lambda { .. } => {
            // Lambdas capture their own scope — skip deep analysis
        }
        _ => {}
    }
}

/// Check all function bodies for dead code after unconditional terminators.
fn check_dead_code(
    functions: &[Function],
    warnings: &mut Vec<Diagnostic>,
) {
    for function in functions {
        check_block_for_dead_code(&function.body, warnings);
    }
}

/// Walk a block and warn on statements that follow an unconditional Return, Break, or Continue.
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
            // Only warn once per block — stop after first unreachable statement
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

        // Recurse into nested blocks
        check_statement_nested_blocks_for_dead_code(statement, warnings);
    }
}

/// Recurse into nested blocks within a statement to detect dead code.
fn check_statement_nested_blocks_for_dead_code(
    statement: &Statement,
    warnings: &mut Vec<Diagnostic>,
) {
    match statement {
        Statement::If { body, elif_branches, else_body, .. } => {
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

/// Check expressions that contain nested blocks (match, lambda) for dead code.
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

/// CC5.2/S6: Check member accesses on store/type instances for validity.
/// If a member access targets a known store constructor and the property
/// is not a known field, emit a warning.
fn check_member_access_validity(
    globals: &[Binding],
    functions: &[Function],
    store_field_names: &HashMap<String, Vec<String>>,
    warnings: &mut Vec<Diagnostic>,
) {
    // Known method names that apply to all values — don't warn on these
    static UNIVERSAL_MEMBERS: &[&str] = &[
        "length", "count", "size", "err", "push", "pop", "get", "set",
        "append", "remove", "insert", "contains", "keys", "values", "clear",
        "join", "map", "filter", "reduce", "find", "any", "all", "sort",
        "equals", "not_equals", "not", "iter", "to_string", "type",
        "trim", "split", "starts_with", "ends_with", "replace", "to_upper",
        "to_lower", "chars", "bytes", "slice", "reverse", "flat_map",
        "enumerate", "zip", "take", "skip", "head", "tail", "last",
        "is_empty", "has_key", "entries",
    ];

    // Track variable name → store type name
    let mut var_types: HashMap<String, String> = HashMap::new();

    for binding in globals {
        if let Some(store_name) = extract_store_type(&binding.value, store_field_names) {
            var_types.insert(binding.name.clone(), store_name);
        }
        check_member_access_in_expr(&binding.value, store_field_names, UNIVERSAL_MEMBERS, &var_types, warnings);
    }
    for func in functions {
        let mut local_var_types = var_types.clone();
        for stmt in &func.body.statements {
            collect_store_bindings(stmt, store_field_names, &mut local_var_types);
            check_member_access_in_statement(stmt, store_field_names, UNIVERSAL_MEMBERS, &local_var_types, warnings);
        }
        if let Some(ref val_expr) = func.body.value {
            check_member_access_in_expr(val_expr, store_field_names, UNIVERSAL_MEMBERS, &local_var_types, warnings);
        }
    }
}

/// If an expression is a call to `make_StoreName()`, return the store name.
fn extract_store_type(expr: &Expression, store_field_names: &HashMap<String, Vec<String>>) -> Option<String> {
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

/// Scan a statement for bindings to store constructors.
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
            check_member_access_in_expr(&binding.value, store_field_names, universal, var_types, warnings);
        }
        Statement::If { condition, body, elif_branches, else_body, .. } => {
            check_member_access_in_expr(condition, store_field_names, universal, var_types, warnings);
            for s in &body.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
            if let Some(ref v) = body.value { check_member_access_in_expr(v, store_field_names, universal, var_types, warnings); }
            for (cond, block) in elif_branches {
                check_member_access_in_expr(cond, store_field_names, universal, var_types, warnings);
                for s in &block.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
                if let Some(ref v) = block.value { check_member_access_in_expr(v, store_field_names, universal, var_types, warnings); }
            }
            if let Some(block) = else_body {
                for s in &block.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
                if let Some(ref v) = block.value { check_member_access_in_expr(v, store_field_names, universal, var_types, warnings); }
            }
        }
        Statement::Return(expr, _) => {
            check_member_access_in_expr(expr, store_field_names, universal, var_types, warnings);
        }
        Statement::While { condition, body, .. } => {
            check_member_access_in_expr(condition, store_field_names, universal, var_types, warnings);
            for s in &body.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
        }
        Statement::For { iterable, body, .. } | Statement::ForKV { iterable, body, .. } => {
            check_member_access_in_expr(iterable, store_field_names, universal, var_types, warnings);
            for s in &body.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
        }
        Statement::ForRange { start, end, step, body, .. } => {
            check_member_access_in_expr(start, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(end, store_field_names, universal, var_types, warnings);
            if let Some(s) = step { check_member_access_in_expr(s, store_field_names, universal, var_types, warnings); }
            for s in &body.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
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
        Expression::Member { target, property, span } => {
            // Check the target for nested member accesses first
            check_member_access_in_expr(target, store_field_names, universal, var_types, warnings);

            // Determine the store type name from the target expression
            let store_type = match target.as_ref() {
                // Direct call: make_Point().z
                Expression::Call { callee, .. } => {
                    if let Expression::Identifier(name, _) = callee.as_ref() {
                        name.strip_prefix("make_")
                            .and_then(|t| store_field_names.get(t).map(|_| t.to_string()))
                    } else {
                        None
                    }
                }
                // Variable: p.z where p was bound to make_Point()
                Expression::Identifier(var_name, _) => {
                    var_types.get(var_name.as_str()).cloned()
                }
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
            for arg in args { check_member_access_in_expr(arg, store_field_names, universal, var_types, warnings); }
        }
        Expression::Binary { left, right, .. } => {
            check_member_access_in_expr(left, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(right, store_field_names, universal, var_types, warnings);
        }
        Expression::Unary { expr, .. } => {
            check_member_access_in_expr(expr, store_field_names, universal, var_types, warnings);
        }
        Expression::Ternary { condition, then_branch, else_branch, .. } => {
            check_member_access_in_expr(condition, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(then_branch, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(else_branch, store_field_names, universal, var_types, warnings);
        }
        Expression::Pipeline { left, right, .. } => {
            check_member_access_in_expr(left, store_field_names, universal, var_types, warnings);
            check_member_access_in_expr(right, store_field_names, universal, var_types, warnings);
        }
        Expression::List(items, _) => {
            for item in items { check_member_access_in_expr(item, store_field_names, universal, var_types, warnings); }
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
            for s in &body.statements { check_member_access_in_statement(s, store_field_names, universal, var_types, warnings); }
            if let Some(ref v) = body.value { check_member_access_in_expr(v, store_field_names, universal, var_types, warnings); }
        }
        _ => {}
    }
}
