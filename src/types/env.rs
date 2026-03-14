use super::core::{Primitive, TypeId};
use std::collections::{HashMap, HashSet};

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

#[derive(Debug, Clone)]
pub struct TypeEnv {
    scopes: Vec<Scope>,

    pub undefined: HashSet<String>,

    type_params: HashMap<String, TypeId>,

    type_param_stack: Vec<HashMap<String, TypeId>>,

    generic_types: HashMap<String, Vec<String>>,

    generic_type_bounds: HashMap<(String, String), Vec<String>>,

    generic_constructors: HashMap<String, (String, Vec<String>, usize)>,

    const_type_params: HashSet<(String, String)>,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self {
            scopes: vec![Scope::new()],
            undefined: HashSet::new(),
            type_params: HashMap::new(),
            type_param_stack: Vec::new(),
            generic_types: HashMap::new(),
            generic_type_bounds: HashMap::new(),
            generic_constructors: HashMap::new(),
            const_type_params: HashSet::new(),
        }
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        let mut env = Self {
            scopes: vec![Scope::new()],
            undefined: HashSet::new(),
            type_params: HashMap::new(),
            type_param_stack: Vec::new(),
            generic_types: HashMap::new(),
            generic_type_bounds: HashMap::new(),
            generic_constructors: HashMap::new(),
            const_type_params: HashSet::new(),
        };

        env.register_generic_type("List", vec!["T"]);
        env.register_generic_type("Map", vec!["K", "V"]);
        env.register_generic_type("Set", vec!["T"]);
        env.register_generic_type("Option", vec!["T"]);
        env.register_generic_type("Result", vec!["T", "E"]);
        env
    }

    pub fn register_generic_type(&mut self, name: impl Into<String>, params: Vec<&str>) {
        self.generic_types
            .insert(name.into(), params.into_iter().map(String::from).collect());
    }

    pub fn register_const_param(&mut self, type_name: &str, param_name: &str) {
        self.const_type_params
            .insert((type_name.to_string(), param_name.to_string()));
    }

    pub fn is_const_param(&self, type_name: &str, param_name: &str) -> bool {
        self.const_type_params
            .contains(&(type_name.to_string(), param_name.to_string()))
    }

    pub fn register_type_param_bounds(
        &mut self,
        type_name: &str,
        param_name: &str,
        bounds: Vec<String>,
    ) {
        if !bounds.is_empty() {
            self.generic_type_bounds
                .insert((type_name.to_string(), param_name.to_string()), bounds);
        }
    }

    pub fn get_type_param_bounds(&self, type_name: &str, param_name: &str) -> Option<&Vec<String>> {
        self.generic_type_bounds
            .get(&(type_name.to_string(), param_name.to_string()))
    }

    pub fn get_generic_params(&self, type_name: &str) -> Option<&Vec<String>> {
        self.generic_types.get(type_name)
    }

    pub fn is_generic_type(&self, type_name: &str) -> bool {
        self.generic_types.contains_key(type_name)
    }

    pub fn register_generic_constructor(
        &mut self,
        ctor_name: impl Into<String>,
        enum_name: String,
        type_params: Vec<String>,
        field_count: usize,
    ) {
        self.generic_constructors
            .insert(ctor_name.into(), (enum_name, type_params, field_count));
    }

    pub fn get_generic_constructor(
        &self,
        ctor_name: &str,
    ) -> Option<&(String, Vec<String>, usize)> {
        self.generic_constructors.get(ctor_name)
    }

    pub fn push_type_params(&mut self) {
        self.type_param_stack.push(self.type_params.clone());
        self.type_params.clear();
    }

    pub fn pop_type_params(&mut self) {
        if let Some(params) = self.type_param_stack.pop() {
            self.type_params = params;
        }
    }

    pub fn bind_type_param(&mut self, param: impl Into<String>, ty: TypeId) {
        self.type_params.insert(param.into(), ty);
    }

    pub fn get_type_param(&self, param: &str) -> Option<&TypeId> {
        self.type_params.get(param)
    }

    pub fn resolve_type(&self, ty: &TypeId) -> TypeId {
        match ty {
            TypeId::TypeVar(var) => TypeId::TypeVar(*var),
            TypeId::List(elem) => TypeId::List(Box::new(self.resolve_type(elem))),
            TypeId::Map(k, v) => TypeId::Map(
                Box::new(self.resolve_type(k)),
                Box::new(self.resolve_type(v)),
            ),
            TypeId::Func(params, ret) => TypeId::Func(
                params.iter().map(|p| self.resolve_type(p)).collect(),
                Box::new(self.resolve_type(ret)),
            ),
            TypeId::Adt(name, args) => TypeId::Adt(
                name.clone(),
                args.iter().map(|a| self.resolve_type(a)).collect(),
            ),
            other => other.clone(),
        }
    }

    pub fn instantiate_generic(
        &mut self,
        type_name: &str,
        type_args: Vec<TypeId>,
    ) -> Option<TypeId> {
        let params = self.generic_types.get(type_name)?.clone();

        if type_args.len() != params.len() {
            return None;
        }

        match type_name {
            "List" | "Set" => Some(TypeId::List(Box::new(type_args.into_iter().next()?))),
            "Map" => {
                let mut args = type_args.into_iter();
                let k = args.next()?;
                let v = args.next()?;
                Some(TypeId::Map(Box::new(k), Box::new(v)))
            }
            "Option" => Some(TypeId::Adt("Option".to_string(), type_args)),
            "Result" => Some(TypeId::Adt("Result".to_string(), type_args)),

            _ => Some(TypeId::Adt(type_name.to_string(), type_args)),
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    pub fn pop_scope(&mut self) -> Option<Scope> {
        if self.scopes.len() > 1 {
            self.scopes.pop()
        } else {
            None
        }
    }

    pub fn insert(&mut self, name: String, ty: TypeId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, Binding::immutable(ty));
        }
    }

    pub fn insert_binding(&mut self, name: String, binding: Binding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, binding);
        }
    }

    pub fn define(&mut self, name: String, ty: TypeId) {
        self.insert(name, ty);
    }

    pub fn define_mut(&mut self, name: String, ty: TypeId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, Binding::mutable(ty));
        }
    }

    pub fn get_binding(&self, name: &str) -> Option<&Binding> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(binding);
            }
        }
        None
    }

    pub fn get(&self, name: &str) -> Option<&TypeId> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(&binding.ty);
            }
        }
        None
    }

    pub fn get_type(&self, name: &str) -> Option<TypeId> {
        self.get(name).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains(name))
    }

    pub fn is_mutable(&self, name: &str) -> bool {
        self.get_binding(name).map(|b| b.mutable).unwrap_or(false)
    }

    pub fn defined_in_current_scope(&self, name: &str) -> bool {
        self.scopes
            .last()
            .map(|s| s.contains(name))
            .unwrap_or(false)
    }

    pub fn depth(&self) -> usize {
        self.scopes.len().saturating_sub(1)
    }

    pub fn iter_all(&self) -> Vec<(String, TypeId)> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for scope in self.scopes.iter().rev() {
            for (name, binding) in &scope.bindings {
                if seen.insert(name.clone()) {
                    result.push((name.clone(), binding.ty.clone()));
                }
            }
        }
        result
    }
}

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

    pub fn register_builtins(&mut self) {
        self.register(FunctionSig::new(
            "print".to_string(),
            vec![("value".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::Unit),
        ));

        self.register(FunctionSig::new(
            "println".to_string(),
            vec![("value".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::Unit),
        ));

        self.register(FunctionSig::new(
            "len".to_string(),
            vec![("collection".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::Int),
        ));

        self.register(FunctionSig::new(
            "str".to_string(),
            vec![("value".to_string(), TypeId::Primitive(Primitive::Any))],
            TypeId::Primitive(Primitive::String),
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Immutable,
    EffectivelyImmutable,
    Mutable,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct MutabilityEnv {
    pub symbols: HashMap<String, Mutability>,
}

impl MutabilityEnv {
    pub fn insert(&mut self, name: impl Into<String>, m: Mutability) {
        self.symbols.insert(name.into(), m);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocationStrategy {
    Stack,
    Arena,
    Heap,
    SharedCow,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct AllocationHints {
    pub symbols: HashMap<String, AllocationStrategy>,
}

impl AllocationHints {
    pub fn insert(&mut self, name: impl Into<String>, hint: AllocationStrategy) {
        self.symbols.insert(name.into(), hint);
    }
}

#[derive(Debug, Clone, Default)]
pub struct SymbolUsage {
    pub reads: u64,
    pub mutations: u64,
    pub escapes: u64,
    pub calls: u64,

    pub closure_captures: u64,

    pub returned: u64,
}

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

        assert!(!env.contains("inner"));
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

        assert_eq!(env.get_type("x"), Some(TypeId::Primitive(Primitive::Bool)));

        env.pop_scope();

        let outer_binding = env.get_binding("x");
        assert!(outer_binding.is_some());
        assert_eq!(outer_binding.unwrap().ty, TypeId::Primitive(Primitive::Int));
        assert_eq!(env.get_type("x"), Some(TypeId::Primitive(Primitive::Int)));
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
