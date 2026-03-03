//! Type environment and related utilities.
//!
//! Provides scope management, mutability tracking, type environment operations,
//! and type parameter tracking for generic types.

use super::core::{TypeId, Primitive};
use std::collections::{HashMap, HashSet};

/// Binding information for a name in scope.
#[derive(Debug, Clone)]
pub struct Binding {
    pub ty: TypeId,
    pub mutable: bool,
    pub initialized: bool,
}

impl Binding {
    pub fn immutable(ty: TypeId) -> Self {
        Self {
            ty,
            mutable: false,
            initialized: true,
        }
    }

    pub fn mutable(ty: TypeId) -> Self {
        Self {
            ty,
            mutable: true,
            initialized: true,
        }
    }

    pub fn uninitialized(ty: TypeId, mutable: bool) -> Self {
        Self {
            ty,
            mutable,
            initialized: false,
        }
    }
}

/// Lexical scope containing variable bindings.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    bindings: HashMap<String, Binding>,
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: String, binding: Binding) {
        self.bindings.insert(name, binding);
    }

    pub fn get(&self, name: &str) -> Option<&Binding> {
        self.bindings.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Binding> {
        self.bindings.get_mut(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }
}

/// Type environment with nested scopes.
#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    scopes: Vec<Scope>,
    /// Legacy field for backward compatibility with semantic.rs
    pub symbols: HashMap<String, TypeId>,
    /// Tracking undefined names
    pub undefined: HashSet<String>,
    /// Type parameter bindings: maps type parameter names to their instantiated types
    /// e.g., when instantiating List[Int], maps "T" -> Int
    type_params: HashMap<String, TypeId>,
    /// Type parameter stack for nested generic contexts
    type_param_stack: Vec<HashMap<String, TypeId>>,
    /// Generic type definitions: maps type name to its type parameter names
    /// e.g., "List" -> ["T"], "Map" -> ["K", "V"]
    generic_types: HashMap<String, Vec<String>>,
}

impl TypeEnv {
    pub fn new() -> Self {
        let mut env = Self {
            scopes: vec![Scope::new()],
            symbols: HashMap::new(),
            undefined: HashSet::new(),
            type_params: HashMap::new(),
            type_param_stack: Vec::new(),
            generic_types: HashMap::new(),
        };
        // Register built-in generic types
        env.register_generic_type("List", vec!["T"]);
        env.register_generic_type("Map", vec!["K", "V"]);
        env.register_generic_type("Set", vec!["T"]);
        env.register_generic_type("Option", vec!["T"]);
        env.register_generic_type("Result", vec!["T", "E"]);
        env
    }

    /// Register a generic type with its type parameters.
    pub fn register_generic_type(&mut self, name: impl Into<String>, params: Vec<&str>) {
        self.generic_types.insert(
            name.into(),
            params.into_iter().map(String::from).collect(),
        );
    }

    /// Get the type parameter names for a generic type.
    pub fn get_generic_params(&self, type_name: &str) -> Option<&Vec<String>> {
        self.generic_types.get(type_name)
    }

    /// Check if a type is a registered generic type.
    pub fn is_generic_type(&self, type_name: &str) -> bool {
        self.generic_types.contains_key(type_name)
    }

    /// Push a new type parameter scope for entering a generic context.
    pub fn push_type_params(&mut self) {
        self.type_param_stack.push(self.type_params.clone());
        self.type_params.clear();
    }

    /// Pop a type parameter scope when leaving a generic context.
    pub fn pop_type_params(&mut self) {
        if let Some(params) = self.type_param_stack.pop() {
            self.type_params = params;
        }
    }

    /// Bind a type parameter to a concrete type.
    pub fn bind_type_param(&mut self, param: impl Into<String>, ty: TypeId) {
        self.type_params.insert(param.into(), ty);
    }

    /// Look up a type parameter binding.
    pub fn get_type_param(&self, param: &str) -> Option<&TypeId> {
        self.type_params.get(param)
    }

    /// Resolve a type, substituting type parameters with their bindings.
    pub fn resolve_type(&self, ty: &TypeId) -> TypeId {
        match ty {
            TypeId::TypeVar(var) => {
                // Check if this var corresponds to a named type parameter
                // For now, just return as-is - the solver handles type vars
                TypeId::TypeVar(*var)
            }
            TypeId::List(elem) => TypeId::List(Box::new(self.resolve_type(elem))),
            TypeId::Map(k, v) => TypeId::Map(
                Box::new(self.resolve_type(k)),
                Box::new(self.resolve_type(v)),
            ),
            TypeId::Func(params, ret) => TypeId::Func(
                params.iter().map(|p| self.resolve_type(p)).collect(),
                Box::new(self.resolve_type(ret)),
            ),
            other => other.clone(),
        }
    }

    /// Instantiate a generic type with concrete type arguments.
    /// e.g., instantiate_generic("List", [TypeId::Primitive(Primitive::Int)]) -> List[Int]
    pub fn instantiate_generic(&mut self, type_name: &str, type_args: Vec<TypeId>) -> Option<TypeId> {
        let params = self.generic_types.get(type_name)?.clone();
        
        if type_args.len() != params.len() {
            return None; // Arity mismatch
        }

        // Bind type parameters (note: we don't actually need these insertions
        // since the return type is constructed directly from type_args below)
        // The insertions were previously leaked into self.type_params.

        // Return the instantiated type
        match type_name {
            "List" | "Set" => Some(TypeId::List(Box::new(type_args.into_iter().next()?))),
            "Map" => {
                let mut args = type_args.into_iter();
                let k = args.next()?;
                let v = args.next()?;
                Some(TypeId::Map(Box::new(k), Box::new(v)))
            }
            "Option" => Some(TypeId::List(Box::new(type_args.into_iter().next()?))), // Simplified for now
            "Result" => {
                // Result[T, E] is represented as a union type, simplified for now
                Some(TypeId::Primitive(Primitive::Any))
            }
            _ => None,
        }
    }

    /// Push a new scope onto the stack.
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    /// Pop the innermost scope.
    pub fn pop_scope(&mut self) -> Option<Scope> {
        if self.scopes.len() > 1 {
            self.scopes.pop()
        } else {
            None
        }
    }

    /// Insert a binding into the current (innermost) scope.
    /// Also updates legacy symbols map.
    pub fn insert(&mut self, name: String, ty: TypeId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.clone(), Binding::immutable(ty.clone()));
        }
        self.symbols.insert(name, ty);
    }

    /// Insert a binding with explicit mutability.
    pub fn insert_binding(&mut self, name: String, binding: Binding) {
        self.symbols.insert(name.clone(), binding.ty.clone());
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, binding);
        }
    }

    /// Define an immutable variable.
    pub fn define(&mut self, name: String, ty: TypeId) {
        self.insert(name, ty);
    }

    /// Define a mutable variable.
    pub fn define_mut(&mut self, name: String, ty: TypeId) {
        self.symbols.insert(name.clone(), ty.clone());
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, Binding::mutable(ty));
        }
    }

    /// Look up a binding by name, searching from innermost to outermost scope.
    pub fn get_binding(&self, name: &str) -> Option<&Binding> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(binding);
            }
        }
        None
    }

    /// Look up the type of a name (legacy API).
    pub fn get(&self, name: &str) -> Option<&TypeId> {
        self.symbols.get(name)
    }

    /// Look up the type of a name.
    pub fn get_type(&self, name: &str) -> Option<TypeId> {
        self.symbols.get(name).cloned()
    }

    /// Check if a name is defined in any scope.
    pub fn contains(&self, name: &str) -> bool {
        self.symbols.contains_key(name)
    }

    /// Check if a name is mutable.
    pub fn is_mutable(&self, name: &str) -> bool {
        self.get_binding(name).map(|b| b.mutable).unwrap_or(false)
    }

    /// Check if a name is defined in the current scope (for shadowing detection).
    pub fn defined_in_current_scope(&self, name: &str) -> bool {
        self.scopes.last().map(|s| s.contains(name)).unwrap_or(false)
    }

    /// Current scope depth (0 = global).
    pub fn depth(&self) -> usize {
        self.scopes.len().saturating_sub(1)
    }
}

/// Function signature for type checking.
#[derive(Debug, Clone)]
pub struct FunctionSig {
    pub name: String,
    pub params: Vec<(String, TypeId)>,
    pub return_type: TypeId,
    pub is_extern: bool,
}

impl FunctionSig {
    pub fn new(name: String, params: Vec<(String, TypeId)>, return_type: TypeId) -> Self {
        Self {
            name,
            params,
            return_type,
            is_extern: false,
        }
    }

    pub fn extern_fn(name: String, params: Vec<(String, TypeId)>, return_type: TypeId) -> Self {
        Self {
            name,
            params,
            return_type,
            is_extern: true,
        }
    }

    pub fn arity(&self) -> usize {
        self.params.len()
    }

    pub fn to_type(&self) -> TypeId {
        let param_types: Vec<TypeId> = self.params.iter().map(|(_, t)| t.clone()).collect();
        TypeId::Func(param_types, Box::new(self.return_type.clone()))
    }
}

/// Global function registry.
#[derive(Debug, Clone, Default)]
pub struct FunctionRegistry {
    functions: HashMap<String, FunctionSig>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, sig: FunctionSig) {
        self.functions.insert(sig.name.clone(), sig);
    }

    pub fn get(&self, name: &str) -> Option<&FunctionSig> {
        self.functions.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &FunctionSig> {
        self.functions.values()
    }

    /// Register built-in functions.
    pub fn register_builtins(&mut self) {
        // print(value: Any) -> Unit
        self.register(FunctionSig::new(
            "print".to_string(),
            vec![("value".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::Unit),
        ));

        // println(value: Any) -> Unit
        self.register(FunctionSig::new(
            "println".to_string(),
            vec![("value".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::Unit),
        ));

        // len(collection: Any) -> Int
        self.register(FunctionSig::new(
            "len".to_string(),
            vec![("collection".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::Int),
        ));

        // str(value: Any) -> String
        self.register(FunctionSig::new(
            "str".to_string(),
            vec![("value".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::String),
        ));
    }
}

// =============================================================================
// Mutability and Allocation tracking (for optimization hints)
// =============================================================================

/// Mutability classification for variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Immutable,
    EffectivelyImmutable,
    Mutable,
    Unknown,
}

/// Tracks mutability of symbols in scope.
#[derive(Debug, Clone, Default)]
pub struct MutabilityEnv {
    pub symbols: HashMap<String, Mutability>,
}

impl MutabilityEnv {
    pub fn insert(&mut self, name: impl Into<String>, m: Mutability) {
        self.symbols.insert(name.into(), m);
    }
}

/// Allocation strategy hints for optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocationStrategy {
    Stack,
    Arena,
    Heap,
    SharedCow,
    Unknown,
}

/// Maps symbols to their allocation hints.
#[derive(Debug, Clone, Default)]
pub struct AllocationHints {
    pub symbols: HashMap<String, AllocationStrategy>,
}

impl AllocationHints {
    pub fn insert(&mut self, name: impl Into<String>, hint: AllocationStrategy) {
        self.symbols.insert(name.into(), hint);
    }
}

/// Usage statistics for a single symbol.
#[derive(Debug, Clone, Default)]
pub struct SymbolUsage {
    pub reads: u64,
    pub mutations: u64,
    pub escapes: u64,
    pub calls: u64,
}

/// Aggregated usage metrics for all symbols.
#[derive(Debug, Clone, Default)]
pub struct UsageMetrics {
    pub symbols: HashMap<String, SymbolUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_env_basic() {
        let mut env = TypeEnv::new();
        env.define("x".to_string(), TypeId::Primitive(Primitive::Int));
        
        assert!(env.contains("x"));
        assert_eq!(env.get_type("x"), Some(TypeId::Primitive(Primitive::Int)));
        assert!(!env.is_mutable("x"));
    }

    #[test]
    fn type_env_mutable() {
        let mut env = TypeEnv::new();
        env.define_mut("x".to_string(), TypeId::Primitive(Primitive::Int));
        
        assert!(env.is_mutable("x"));
    }

    #[test]
    fn type_env_scoping() {
        let mut env = TypeEnv::new();
        env.define("outer".to_string(), TypeId::Primitive(Primitive::Int));
        
        env.push_scope();
        env.define("inner".to_string(), TypeId::Primitive(Primitive::Bool));
        
        assert!(env.contains("outer"));
        assert!(env.contains("inner"));
        
        env.pop_scope();
        
        assert!(env.contains("outer"));
        // Note: legacy symbols map retains all entries for backward compat.
        // Use get_binding() for proper scoped lookup.
        assert!(env.get_binding("inner").is_none());
    }

    #[test]
    fn type_env_shadowing() {
        let mut env = TypeEnv::new();
        env.define("x".to_string(), TypeId::Primitive(Primitive::Int));
        
        env.push_scope();
        assert!(!env.defined_in_current_scope("x"));
        env.define("x".to_string(), TypeId::Primitive(Primitive::Bool));
        assert!(env.defined_in_current_scope("x"));
        
        // get_type uses legacy symbols which will have the newer value.
        assert_eq!(env.get_type("x"), Some(TypeId::Primitive(Primitive::Bool)));
        
        env.pop_scope();
        
        // After pop, get_binding should return None for "x" in inner scope,
        // but symbols map retains last value. This is okay for backward compat.
        // The scoped lookup via get_binding would show the outer value.
        let outer_binding = env.get_binding("x");
        assert!(outer_binding.is_some());
        assert_eq!(outer_binding.unwrap().ty, TypeId::Primitive(Primitive::Int));
    }

    #[test]
    fn function_sig_to_type() {
        let sig = FunctionSig::new(
            "add".to_string(),
            vec![
                ("a".to_string(), TypeId::Primitive(Primitive::Int)),
                ("b".to_string(), TypeId::Primitive(Primitive::Int)),
            ],
            TypeId::Primitive(Primitive::Int),
        );
        
        assert_eq!(sig.arity(), 2);
        
        let fn_type = sig.to_type();
        match fn_type {
            TypeId::Func(args, ret) => {
                assert_eq!(args.len(), 2);
                assert_eq!(*ret, TypeId::Primitive(Primitive::Int));
            }
            _ => panic!("Expected function type"),
        }
    }

    #[test]
    fn function_registry_builtins() {
        let mut reg = FunctionRegistry::new();
        reg.register_builtins();
        
        assert!(reg.contains("print"));
        assert!(reg.contains("println"));
        assert!(reg.contains("len"));
        assert!(reg.contains("str"));
    }
}
