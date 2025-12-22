//! Type system scaffolding: type ids, variables, and constraint forms (not yet wired).
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Primitive {
    Int,
    Float,
    Bool,
    String,
    Bytes,
    Unit,
    Any,
    Actor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Immutable,
    EffectivelyImmutable,
    Mutable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AllocationStrategy {
    Stack,
    Arena,
    Heap,
    SharedCow,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeId {
    Primitive(Primitive),
    List(Box<TypeId>),
    Map(Box<TypeId>, Box<TypeId>),
    Func(Vec<TypeId>, Box<TypeId>),
    Placeholder(u32),
    TypeVar(TypeVarId),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeVarId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintKind {
    Equal(TypeId, TypeId),
    Numeric(TypeId),
    Boolean(TypeId),
    Iterable(TypeId, TypeId), // iterable -> element type
    Callable(TypeId, Vec<TypeId>, TypeId),
}

#[derive(Debug, Default, Clone)]
pub struct ConstraintSet {
    pub constraints: Vec<ConstraintKind>,
}

impl ConstraintSet {
    pub fn push(&mut self, c: ConstraintKind) {
        self.constraints.push(c);
    }
}

#[derive(Debug, Default, Clone)]
pub struct TypeGraph {
    pub next_var: u32,
    pub parents: HashMap<TypeVarId, TypeVarId>,
    pub repr: HashMap<TypeVarId, TypeId>,
}

impl TypeGraph {
    pub fn fresh(&mut self) -> TypeVarId {
        let id = TypeVarId(self.next_var);
        self.next_var += 1;
        id
    }

    pub fn find(&mut self, var: TypeVarId) -> TypeVarId {
        let parent = *self.parents.get(&var).unwrap_or(&var);
        if parent == var {
            return var;
        }
        let root = self.find(parent);
        self.parents.insert(var, root);
        root
    }

    pub fn unify(&mut self, a: TypeVarId, b: TypeVarId) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parents.insert(ra, rb);
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct TypeEnv {
    pub symbols: HashMap<String, TypeId>,
    pub undefined: HashSet<String>,
}

impl TypeEnv {
    pub fn insert(&mut self, name: impl Into<String>, ty: TypeId) {
        self.symbols.insert(name.into(), ty);
    }
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
}

#[derive(Debug, Clone, Default)]
pub struct UsageMetrics {
    pub symbols: HashMap<String, SymbolUsage>,
}

pub fn format_type(ty: &TypeId) -> String {
    match ty {
        TypeId::Primitive(p) => match p {
            Primitive::Int => "Int".into(),
            Primitive::Float => "Float".into(),
            Primitive::Bool => "Bool".into(),
            Primitive::String => "String".into(),
            Primitive::Bytes => "Bytes".into(),
            Primitive::Unit => "Unit".into(),
            Primitive::Any => "Any".into(),
            Primitive::Actor => "Actor".into(),
        },
        TypeId::List(elem) => format!("[{}]", format_type(elem)),
        TypeId::Map(k, v) => format!("{{{}:{}}}", format_type(k), format_type(v)),
        TypeId::Func(args, ret) => {
            let args_s: Vec<String> = args.iter().map(format_type).collect();
            format!("fn({})->{}", args_s.join(","), format_type(ret))
        }
        TypeId::Placeholder(id) => format!("<_{}>", id),
        TypeId::TypeVar(id) => format!("t{}", id.0),
        TypeId::Unknown => "_".into(),
    }
}

pub fn solve_constraints(
    constraints: &ConstraintSet,
    graph: &mut TypeGraph,
) -> Result<(), String> {
    for c in &constraints.constraints {
        match c {
            ConstraintKind::Equal(a, b) => unify(a.clone(), b.clone(), graph)?,
            ConstraintKind::Numeric(ty) => enforce_numeric(ty.clone(), graph)?,
            ConstraintKind::Boolean(ty) => enforce_boolean(ty.clone(), graph)?,
            ConstraintKind::Iterable(container, elem) => {
                unify(container.clone(), TypeId::List(Box::new(elem.clone())), graph)?;
            }
            ConstraintKind::Callable(func, args, ret) => {
                unify(
                    func.clone(),
                    TypeId::Func(args.clone(), Box::new(ret.clone())),
                    graph,
                )?;
            }
        }
    }
    Ok(())
}

fn enforce_numeric(ty: TypeId, graph: &mut TypeGraph) -> Result<(), String> {
    match resolve(ty.clone(), graph) {
        TypeId::Primitive(Primitive::Int) | TypeId::Primitive(Primitive::Float) => Ok(()),
        TypeId::TypeVar(var) => {
            graph.repr.insert(var, TypeId::Primitive(Primitive::Float));
            Ok(())
        }
        other => Err(format!("expected numeric type, found {}", format_type(&other))),
    }
}

fn enforce_boolean(ty: TypeId, graph: &mut TypeGraph) -> Result<(), String> {
    match resolve(ty.clone(), graph) {
        TypeId::Primitive(Primitive::Bool) => Ok(()),
        TypeId::TypeVar(var) => {
            graph.repr.insert(var, TypeId::Primitive(Primitive::Bool));
            Ok(())
        }
        other => Err(format!("expected boolean type, found {}", format_type(&other))),
    }
}

fn unify(a: TypeId, b: TypeId, graph: &mut TypeGraph) -> Result<(), String> {
    let ra = resolve(a, graph);
    let rb = resolve(b, graph);
    match (ra, rb) {
        (x, y) if x == y => Ok(()),
        (TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::Float))
        | (TypeId::Primitive(Primitive::Float), TypeId::Primitive(Primitive::Int)) => Ok(()),
        (TypeId::TypeVar(v), ty) | (ty, TypeId::TypeVar(v)) => bind_var(v, ty, graph),
        (TypeId::List(ae), TypeId::List(be)) => unify(*ae, *be, graph),
        (TypeId::Map(ak, av), TypeId::Map(bk, bv)) => {
            unify(*ak, *bk, graph)?;
            unify(*av, *bv, graph)
        }
        (TypeId::Func(a_args, a_ret), TypeId::Func(b_args, b_ret)) => {
            if a_args.len() == b_args.len() {
                for (aa, ba) in a_args.into_iter().zip(b_args.into_iter()) {
                    unify(aa, ba, graph)?;
                }
                unify(*a_ret, *b_ret, graph)
            } else {
                Ok(())
            }
        }
        (TypeId::Unknown, _ty) | (_ty, TypeId::Unknown) => Ok(()),
        (a, b) => {
            // Fallback: permit mismatched primitives to keep inference permissive for now.
            let _ = (a, b);
            Ok(())
        }
    }
}

fn bind_var(var: TypeVarId, ty: TypeId, graph: &mut TypeGraph) -> Result<(), String> {
    if occurs(var, &ty, graph) {
        return Err("occurs check failed".into());
    }
    let root = graph.find(var);
    graph.repr.insert(root, ty);
    Ok(())
}

fn occurs(var: TypeVarId, ty: &TypeId, graph: &mut TypeGraph) -> bool {
    match resolve(ty.clone(), graph) {
        TypeId::TypeVar(v) => graph.find(v) == graph.find(var),
        TypeId::List(elem) => occurs(var, &elem, graph),
        TypeId::Map(k, v) => occurs(var, &k, graph) || occurs(var, &v, graph),
        TypeId::Func(args, ret) => args.iter().any(|a| occurs(var, a, graph)) || occurs(var, &ret, graph),
        _ => false,
    }
}

pub fn resolve(ty: TypeId, graph: &mut TypeGraph) -> TypeId {
    match ty {
        TypeId::TypeVar(v) => {
            let root = graph.find(v);
            if let Some(t) = graph.repr.get(&root).cloned() {
                resolve(t, graph)
            } else {
                TypeId::TypeVar(root)
            }
        }
        TypeId::List(elem) => TypeId::List(Box::new(resolve(*elem, graph))),
        TypeId::Map(k, v) => TypeId::Map(Box::new(resolve(*k, graph)), Box::new(resolve(*v, graph))),
        TypeId::Func(args, ret) => {
            let args_r: Vec<TypeId> = args.into_iter().map(|a| resolve(a, graph)).collect();
            TypeId::Func(args_r, Box::new(resolve(*ret, graph)))
        }
        other => other,
    }
}
