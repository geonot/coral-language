use super::core::{Primitive, TypeId, TypeVarId, format_type};
use crate::span::Span;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ConstraintOrigin {
    pub description: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
    pub expected: Option<TypeId>,
    pub found: Option<TypeId>,

    pub expected_origin: Option<ConstraintOrigin>,

    pub found_origin: Option<ConstraintOrigin>,
}

impl TypeError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            expected: None,
            found: None,
            expected_origin: None,
            found_origin: None,
        }
    }

    pub fn with_types(mut self, expected: TypeId, found: TypeId) -> Self {
        self.expected = Some(expected);
        self.found = Some(found);
        self
    }

    pub fn with_origins(
        mut self,
        expected_origin: Option<ConstraintOrigin>,
        found_origin: Option<ConstraintOrigin>,
    ) -> Self {
        self.expected_origin = expected_origin;
        self.found_origin = found_origin;
        self
    }

    pub fn mismatch(expected: &TypeId, found: &TypeId, span: Span) -> Self {
        Self {
            message: format!(
                "type mismatch: expected `{}`, found `{}`",
                format_type(expected),
                format_type(found)
            ),
            span,
            expected: Some(expected.clone()),
            found: Some(found.clone()),
            expected_origin: None,
            found_origin: None,
        }
    }

    pub fn not_numeric(ty: &TypeId, span: Span) -> Self {
        Self {
            message: format!(
                "expected numeric type (Int or Float), found `{}`",
                format_type(ty)
            ),
            span,
            expected: Some(TypeId::Primitive(Primitive::Float)),
            found: Some(ty.clone()),
            expected_origin: None,
            found_origin: None,
        }
    }

    pub fn not_boolean(ty: &TypeId, span: Span) -> Self {
        Self {
            message: format!("expected Bool, found `{}`", format_type(ty)),
            span,
            expected: Some(TypeId::Primitive(Primitive::Bool)),
            found: Some(ty.clone()),
            expected_origin: None,
            found_origin: None,
        }
    }

    pub fn not_callable(ty: &TypeId, span: Span) -> Self {
        Self {
            message: format!("`{}` is not callable", format_type(ty)),
            span,
            expected: None,
            found: Some(ty.clone()),
            expected_origin: None,
            found_origin: None,
        }
    }

    pub fn arity_mismatch(expected: usize, found: usize, span: Span) -> Self {
        Self {
            message: format!(
                "function expects {} argument{}, but {} {} provided",
                expected,
                if expected == 1 { "" } else { "s" },
                found,
                if found == 1 { "was" } else { "were" }
            ),
            span,
            expected: None,
            found: None,
            expected_origin: None,
            found_origin: None,
        }
    }

    pub fn undefined_name(name: &str, span: Span) -> Self {
        Self {
            message: format!("undefined name `{}`", name),
            span,
            expected: None,
            found: None,
            expected_origin: None,
            found_origin: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintKind {
    EqualAt(TypeId, TypeId, Span),

    Equal(TypeId, TypeId),

    NumericAt(TypeId, Span),

    Numeric(TypeId),

    BooleanAt(TypeId, Span),

    Boolean(TypeId),

    IterableAt(TypeId, TypeId, Span),

    Iterable(TypeId, TypeId),

    CallableAt(TypeId, Vec<TypeId>, TypeId, Span),

    Callable(TypeId, Vec<TypeId>, TypeId),

    HasTrait(TypeId, String, Span),
}

#[derive(Debug, Default, Clone)]
pub struct TraitRegistry {
    pub implementations: HashMap<String, Vec<String>>,

    pub super_traits: HashMap<String, Vec<String>>,
}

impl TraitRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_impl(&mut self, type_name: impl Into<String>, trait_name: impl Into<String>) {
        self.implementations
            .entry(type_name.into())
            .or_default()
            .push(trait_name.into());
    }

    pub fn register_super_traits(&mut self, trait_name: impl Into<String>, supers: Vec<String>) {
        self.super_traits.insert(trait_name.into(), supers);
    }

    pub fn type_implements(&self, type_name: &str, trait_name: &str) -> bool {
        if let Some(impls) = self.implementations.get(type_name) {
            impls.iter().any(|t| t == trait_name)
        } else {
            false
        }
    }

    pub fn check_trait(&self, ty: &TypeId, trait_name: &str, span: Span) -> Result<(), TypeError> {
        match ty {
            TypeId::TypeVar(_) | TypeId::Primitive(Primitive::Any) | TypeId::Unknown => Ok(()),

            TypeId::Adt(name, _) | TypeId::Store(name) => {
                if self.type_implements(name, trait_name) {
                    Ok(())
                } else {
                    Err(TypeError::new(
                        format!(
                            "type `{}` does not implement trait `{}`",
                            format_type(ty),
                            trait_name
                        ),
                        span,
                    ))
                }
            }

            TypeId::Primitive(p) => {
                if primitive_implements_trait(p, trait_name) {
                    Ok(())
                } else {
                    Err(TypeError::new(
                        format!(
                            "type `{}` does not implement trait `{}`",
                            format_type(ty),
                            trait_name
                        ),
                        span,
                    ))
                }
            }

            _ => Ok(()),
        }
    }
}

fn primitive_implements_trait(prim: &Primitive, trait_name: &str) -> bool {
    match trait_name {
        "Comparable" => matches!(
            prim,
            Primitive::Int | Primitive::Float | Primitive::String | Primitive::Bool
        ),
        "Printable" | "Display" => true,
        "Hashable" => matches!(
            prim,
            Primitive::Int
                | Primitive::Float
                | Primitive::String
                | Primitive::Bool
                | Primitive::Unit
                | Primitive::None
        ),
        "Numeric" => matches!(prim, Primitive::Int | Primitive::Float),
        "Iterable" => false,
        _ => false,
    }
}

#[derive(Debug, Default, Clone)]
pub struct ConstraintSet {
    pub constraints: Vec<ConstraintKind>,
}

impl ConstraintSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, c: ConstraintKind) {
        self.constraints.push(c);
    }

    pub fn extend(&mut self, other: ConstraintSet) {
        self.constraints.extend(other.constraints);
    }

    pub fn len(&self) -> usize {
        self.constraints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }
}

#[derive(Debug, Default, Clone)]
pub struct TypeGraph {
    next_var: u32,
    parents: HashMap<TypeVarId, TypeVarId>,
    repr: HashMap<TypeVarId, TypeId>,

    ranks: HashMap<TypeVarId, u32>,

    binding_origins: HashMap<TypeVarId, ConstraintOrigin>,
}

impl TypeGraph {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn union(&mut self, a: TypeVarId, b: TypeVarId) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            let rank_a = *self.ranks.get(&ra).unwrap_or(&0);
            let rank_b = *self.ranks.get(&rb).unwrap_or(&0);
            let has_repr_a = self.repr.contains_key(&ra);
            let has_repr_b = self.repr.contains_key(&rb);

            if rank_a > rank_b {
                self.parents.insert(rb, ra);
                if !has_repr_a && has_repr_b {
                    if let Some(ty) = self.repr.remove(&rb) {
                        self.repr.insert(ra, ty);
                    }
                }
            } else if rank_b > rank_a {
                self.parents.insert(ra, rb);
                if has_repr_a && !has_repr_b {
                    if let Some(ty) = self.repr.remove(&ra) {
                        self.repr.insert(rb, ty);
                    }
                }
            } else {
                if has_repr_a && !has_repr_b {
                    self.parents.insert(rb, ra);
                    self.ranks.insert(ra, rank_a + 1);
                } else {
                    self.parents.insert(ra, rb);
                    self.ranks.insert(rb, rank_b + 1);
                    if has_repr_a && !has_repr_b {
                        if let Some(ty) = self.repr.remove(&ra) {
                            self.repr.insert(rb, ty);
                        }
                    }
                }
            }
        }
    }

    pub fn bind(&mut self, var: TypeVarId, ty: TypeId) {
        let root = self.find(var);
        self.repr.insert(root, ty);
    }

    pub fn bind_with_origin(&mut self, var: TypeVarId, ty: TypeId, origin: ConstraintOrigin) {
        let root = self.find(var);
        self.repr.insert(root, ty);
        self.binding_origins.entry(root).or_insert(origin);
    }

    pub fn get_binding(&mut self, var: TypeVarId) -> Option<TypeId> {
        let root = self.find(var);
        self.repr.get(&root).cloned()
    }

    pub fn get_binding_origin(&mut self, var: TypeVarId) -> Option<ConstraintOrigin> {
        let root = self.find(var);
        self.binding_origins.get(&root).cloned()
    }
}

pub fn solve_constraints(
    constraints: &ConstraintSet,
    graph: &mut TypeGraph,
    trait_registry: &TraitRegistry,
) -> Result<(), Vec<TypeError>> {
    let mut errors = Vec::new();
    let dummy = Span::new(0, 0);

    let mut sorted_constraints = constraints.constraints.clone();
    sorted_constraints.sort_by_key(|c| match c {
        ConstraintKind::Equal(_, _) | ConstraintKind::EqualAt(_, _, _) => 0,
        ConstraintKind::Numeric(_)
        | ConstraintKind::NumericAt(_, _)
        | ConstraintKind::Boolean(_)
        | ConstraintKind::BooleanAt(_, _) => 1,
        ConstraintKind::Iterable(_, _) | ConstraintKind::IterableAt(_, _, _) => 2,
        ConstraintKind::Callable(_, _, _) | ConstraintKind::CallableAt(_, _, _, _) => 3,
        ConstraintKind::HasTrait(_, _, _) => 4,
    });

    for c in &sorted_constraints {
        let result = match c {
            ConstraintKind::EqualAt(a, b, span) => unify(a.clone(), b.clone(), graph, *span),
            ConstraintKind::NumericAt(ty, span) => enforce_numeric(ty.clone(), graph, *span),
            ConstraintKind::BooleanAt(ty, span) => enforce_boolean(ty.clone(), graph, *span),
            ConstraintKind::IterableAt(container, elem, span) => {
                let resolved_container = resolve(container.clone(), graph);
                match &resolved_container {
                    TypeId::Map(key, _val) => unify(elem.clone(), *key.clone(), graph, *span),

                    TypeId::Primitive(Primitive::Any) | TypeId::Unknown => {
                        unify(elem.clone(), resolved_container.clone(), graph, *span)
                    }

                    _ => unify(
                        container.clone(),
                        TypeId::List(Box::new(elem.clone())),
                        graph,
                        *span,
                    ),
                }
            }
            ConstraintKind::CallableAt(func, args, ret, span) => {
                solve_callable(func, args, ret, graph, *span)
            }

            ConstraintKind::Equal(a, b) => unify(a.clone(), b.clone(), graph, dummy),
            ConstraintKind::Numeric(ty) => enforce_numeric(ty.clone(), graph, dummy),
            ConstraintKind::Boolean(ty) => enforce_boolean(ty.clone(), graph, dummy),
            ConstraintKind::Iterable(container, elem) => {
                let resolved_container = resolve(container.clone(), graph);
                match &resolved_container {
                    TypeId::Map(key, _val) => unify(elem.clone(), *key.clone(), graph, dummy),
                    TypeId::Primitive(Primitive::Any) | TypeId::Unknown => {
                        unify(elem.clone(), resolved_container.clone(), graph, dummy)
                    }
                    _ => unify(
                        container.clone(),
                        TypeId::List(Box::new(elem.clone())),
                        graph,
                        dummy,
                    ),
                }
            }
            ConstraintKind::Callable(func, args, ret) => {
                solve_callable(func, args, ret, graph, dummy)
            }

            ConstraintKind::HasTrait(ty, trait_name, span) => {
                let resolved = resolve(ty.clone(), graph);
                trait_registry.check_trait(&resolved, trait_name, *span)
            }
        };

        if let Err(e) = result {
            errors.push(e);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        errors.sort_by_key(|e| (e.span.start, e.span.end));
        Err(errors)
    }
}

fn solve_callable(
    func: &TypeId,
    args: &[TypeId],
    ret: &TypeId,
    graph: &mut TypeGraph,
    span: Span,
) -> Result<(), TypeError> {
    let func_resolved = resolve(func.clone(), graph);
    match func_resolved {
        TypeId::Func(expected_args, expected_ret) => {
            if args.len() > expected_args.len() {
                Err(TypeError::arity_mismatch(
                    expected_args.len(),
                    args.len(),
                    span,
                ))
            } else {
                let mut inner_errors = Vec::new();

                for (expected, actual) in expected_args.iter().zip(args.iter()) {
                    if let Err(e) = unify(expected.clone(), actual.clone(), graph, span) {
                        inner_errors.push(e);
                    }
                }
                if let Err(e) = unify(*expected_ret, ret.clone(), graph, span) {
                    inner_errors.push(e);
                }
                if inner_errors.is_empty() {
                    Ok(())
                } else {
                    Err(inner_errors.remove(0))
                }
            }
        }
        TypeId::TypeVar(var) => {
            let fn_type = TypeId::Func(args.to_vec(), Box::new(ret.clone()));
            graph.bind(var, fn_type);
            Ok(())
        }
        TypeId::Unknown | TypeId::Primitive(Primitive::Any) => Ok(()),
        other => Err(TypeError::not_callable(&other, span)),
    }
}

fn enforce_numeric(ty: TypeId, graph: &mut TypeGraph, span: Span) -> Result<(), TypeError> {
    let resolved = resolve(ty.clone(), graph);
    match resolved {
        TypeId::Primitive(Primitive::Int) | TypeId::Primitive(Primitive::Float) => Ok(()),
        TypeId::Primitive(Primitive::Any) | TypeId::Unknown => Ok(()),

        TypeId::Error(_) => Ok(()),
        TypeId::TypeVar(var) => {
            graph.bind(var, TypeId::Primitive(Primitive::Float));
            Ok(())
        }
        other => Err(TypeError::not_numeric(&other, span)),
    }
}

fn enforce_boolean(ty: TypeId, graph: &mut TypeGraph, span: Span) -> Result<(), TypeError> {
    let resolved = resolve(ty.clone(), graph);
    match resolved {
        TypeId::Primitive(Primitive::Bool) => Ok(()),
        TypeId::Primitive(Primitive::Any) | TypeId::Unknown => Ok(()),

        TypeId::Error(_) => Ok(()),
        TypeId::TypeVar(var) => {
            graph.bind(var, TypeId::Primitive(Primitive::Bool));
            Ok(())
        }
        other => Err(TypeError::not_boolean(&other, span)),
    }
}

fn unify(a: TypeId, b: TypeId, graph: &mut TypeGraph, span: Span) -> Result<(), TypeError> {
    let ra = resolve(a, graph);
    let rb = resolve(b, graph);

    match (&ra, &rb) {
        (x, y) if x == y => Ok(()),

        (TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::Float))
        | (TypeId::Primitive(Primitive::Float), TypeId::Primitive(Primitive::Int)) => Ok(()),

        (TypeId::Primitive(Primitive::Any), _) | (_, TypeId::Primitive(Primitive::Any)) => Ok(()),

        (TypeId::Primitive(Primitive::None), TypeId::Primitive(Primitive::Unit))
        | (TypeId::Primitive(Primitive::Unit), TypeId::Primitive(Primitive::None)) => Ok(()),

        (TypeId::Primitive(Primitive::None), _) | (_, TypeId::Primitive(Primitive::None)) => Ok(()),

        (TypeId::Unknown, _) | (_, TypeId::Unknown) => Ok(()),

        (TypeId::TypeVar(v), ty) | (ty, TypeId::TypeVar(v)) => {
            if occurs(*v, ty, graph) {
                Err(TypeError::new("infinite type (occurs check failed)", span))
            } else {
                graph.bind(*v, ty.clone());
                Ok(())
            }
        }

        (TypeId::Adt(a_name, a_args), TypeId::Adt(b_name, b_args)) => {
            if a_name == b_name {
                if a_args.is_empty() && b_args.is_empty() {
                    Ok(())
                } else if a_args.len() != b_args.len() {
                    Err(TypeError::mismatch(&ra, &rb, span))
                } else {
                    let mut field_errors = Vec::new();
                    for (aa, ba) in a_args.iter().zip(b_args.iter()) {
                        if let Err(e) = unify(aa.clone(), ba.clone(), graph, span) {
                            field_errors.push(e);
                        }
                    }
                    if field_errors.is_empty() {
                        Ok(())
                    } else {
                        Err(field_errors.remove(0))
                    }
                }
            } else {
                Err(TypeError::mismatch(&ra, &rb, span))
            }
        }

        (TypeId::Store(a_name), TypeId::Store(b_name)) => {
            if a_name == b_name {
                Ok(())
            } else {
                Err(TypeError::mismatch(&ra, &rb, span))
            }
        }

        (TypeId::Error(_), TypeId::Error(_)) => Ok(()),

        (TypeId::Error(_), _) | (_, TypeId::Error(_)) => Ok(()),

        (TypeId::List(ae), TypeId::List(be)) => unify(*ae.clone(), *be.clone(), graph, span),

        (TypeId::Map(ak, av), TypeId::Map(bk, bv)) => {
            let mut map_errors = Vec::new();
            if let Err(e) = unify(*ak.clone(), *bk.clone(), graph, span) {
                map_errors.push(e);
            }
            if let Err(e) = unify(*av.clone(), *bv.clone(), graph, span) {
                map_errors.push(e);
            }
            if map_errors.is_empty() {
                Ok(())
            } else {
                Err(map_errors.remove(0))
            }
        }

        (TypeId::Func(a_args, a_ret), TypeId::Func(b_args, b_ret)) => {
            if a_args.len() != b_args.len() {
                return Err(TypeError::arity_mismatch(a_args.len(), b_args.len(), span));
            }

            let mut fn_errors = Vec::new();
            for (aa, ba) in a_args.iter().zip(b_args.iter()) {
                if let Err(e) = unify(aa.clone(), ba.clone(), graph, span) {
                    fn_errors.push(e);
                }
            }
            if let Err(e) = unify(*a_ret.clone(), *b_ret.clone(), graph, span) {
                fn_errors.push(e);
            }
            if fn_errors.is_empty() {
                Ok(())
            } else {
                Err(fn_errors.remove(0))
            }
        }

        (TypeId::Primitive(a), TypeId::Primitive(b)) => Err(TypeError::mismatch(
            &TypeId::Primitive(a.clone()),
            &TypeId::Primitive(b.clone()),
            span,
        )),

        _ => Err(TypeError::mismatch(&ra, &rb, span)),
    }
}

fn occurs(var: TypeVarId, ty: &TypeId, graph: &mut TypeGraph) -> bool {
    let resolved = resolve(ty.clone(), graph);
    match resolved {
        TypeId::TypeVar(v) => graph.find(v) == graph.find(var),
        TypeId::List(elem) => occurs(var, &elem, graph),
        TypeId::Map(k, v) => occurs(var, &k, graph) || occurs(var, &v, graph),
        TypeId::Func(args, ret) => {
            args.iter().any(|a| occurs(var, a, graph)) || occurs(var, &ret, graph)
        }
        TypeId::Adt(_, args) => args.iter().any(|a| occurs(var, a, graph)),
        _ => false,
    }
}

pub fn resolve(ty: TypeId, graph: &mut TypeGraph) -> TypeId {
    resolve_inner(ty, graph, 0)
}

fn resolve_inner(ty: TypeId, graph: &mut TypeGraph, depth: usize) -> TypeId {
    if depth > 100 {
        return TypeId::Primitive(Primitive::Any);
    }
    match ty {
        TypeId::TypeVar(v) => {
            let root = graph.find(v);
            if let Some(t) = graph.get_binding(root) {
                resolve_inner(t, graph, depth + 1)
            } else {
                TypeId::TypeVar(root)
            }
        }
        TypeId::List(elem) => TypeId::List(Box::new(resolve_inner(*elem, graph, depth + 1))),
        TypeId::Map(k, v) => TypeId::Map(
            Box::new(resolve_inner(*k, graph, depth + 1)),
            Box::new(resolve_inner(*v, graph, depth + 1)),
        ),
        TypeId::Func(args, ret) => {
            let args_r: Vec<TypeId> = args
                .into_iter()
                .map(|a| resolve_inner(a, graph, depth + 1))
                .collect();
            TypeId::Func(args_r, Box::new(resolve_inner(*ret, graph, depth + 1)))
        }
        TypeId::Adt(name, args) => {
            let args_r: Vec<TypeId> = args
                .into_iter()
                .map(|a| resolve_inner(a, graph, depth + 1))
                .collect();
            TypeId::Adt(name, args_r)
        }

        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 0)
    }

    fn empty_registry() -> TraitRegistry {
        TraitRegistry::new()
    }

    #[test]
    fn unify_same_types() {
        let mut graph = TypeGraph::new();
        let result = unify(
            TypeId::Primitive(Primitive::Int),
            TypeId::Primitive(Primitive::Int),
            &mut graph,
            span(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn unify_int_float() {
        let mut graph = TypeGraph::new();
        let result = unify(
            TypeId::Primitive(Primitive::Int),
            TypeId::Primitive(Primitive::Float),
            &mut graph,
            span(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn unify_type_var() {
        let mut graph = TypeGraph::new();
        let var = graph.fresh();
        let result = unify(
            TypeId::TypeVar(var),
            TypeId::Primitive(Primitive::Int),
            &mut graph,
            span(),
        );
        assert!(result.is_ok());
        assert_eq!(
            resolve(TypeId::TypeVar(var), &mut graph),
            TypeId::Primitive(Primitive::Int)
        );
    }

    #[test]
    fn unify_mismatch() {
        let mut graph = TypeGraph::new();
        let result = unify(
            TypeId::List(Box::new(TypeId::Primitive(Primitive::Int))),
            TypeId::Primitive(Primitive::String),
            &mut graph,
            span(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn unify_primitives_strict() {
        let mut graph = TypeGraph::new();
        let result = unify(
            TypeId::Primitive(Primitive::Bool),
            TypeId::Primitive(Primitive::String),
            &mut graph,
            span(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("type mismatch"));
    }

    #[test]
    fn solve_numeric_constraint() {
        let mut graph = TypeGraph::new();
        let var = graph.fresh();
        let mut constraints = ConstraintSet::new();
        constraints.push(ConstraintKind::NumericAt(TypeId::TypeVar(var), span()));

        let result = solve_constraints(&constraints, &mut graph, &empty_registry());
        assert!(result.is_ok());
        assert_eq!(
            resolve(TypeId::TypeVar(var), &mut graph),
            TypeId::Primitive(Primitive::Float)
        );
    }

    #[test]
    fn solve_boolean_constraint_error() {
        let mut graph = TypeGraph::new();
        let mut constraints = ConstraintSet::new();
        constraints.push(ConstraintKind::BooleanAt(
            TypeId::Primitive(Primitive::Int),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &empty_registry());
        assert!(result.is_err());
    }

    #[test]
    fn solve_callable_constraint() {
        let mut graph = TypeGraph::new();
        let mut constraints = ConstraintSet::new();

        let fn_type = TypeId::Func(
            vec![TypeId::Primitive(Primitive::Int)],
            Box::new(TypeId::Primitive(Primitive::Bool)),
        );
        let ret_var = graph.fresh();

        constraints.push(ConstraintKind::CallableAt(
            fn_type,
            vec![TypeId::Primitive(Primitive::Int)],
            TypeId::TypeVar(ret_var),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &empty_registry());
        assert!(result.is_ok());
        assert_eq!(
            resolve(TypeId::TypeVar(ret_var), &mut graph),
            TypeId::Primitive(Primitive::Bool)
        );
    }

    #[test]
    fn solve_arity_mismatch() {
        let mut graph = TypeGraph::new();
        let mut constraints = ConstraintSet::new();

        let fn_type = TypeId::Func(
            vec![TypeId::Primitive(Primitive::Int)],
            Box::new(TypeId::Primitive(Primitive::Int)),
        );

        constraints.push(ConstraintKind::CallableAt(
            fn_type,
            vec![
                TypeId::Primitive(Primitive::Int),
                TypeId::Primitive(Primitive::Int),
            ],
            TypeId::Primitive(Primitive::Int),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &empty_registry());
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors[0].message.contains("expects 1 argument"));
    }

    #[test]
    fn solve_legacy_equal_constraint() {
        let mut graph = TypeGraph::new();
        let var = graph.fresh();
        let mut constraints = ConstraintSet::new();
        constraints.push(ConstraintKind::Equal(
            TypeId::TypeVar(var),
            TypeId::Primitive(Primitive::Int),
        ));

        let result = solve_constraints(&constraints, &mut graph, &empty_registry());
        assert!(result.is_ok());
        assert_eq!(
            resolve(TypeId::TypeVar(var), &mut graph),
            TypeId::Primitive(Primitive::Int)
        );
    }

    #[test]
    fn solve_has_trait_with_registered_impl() {
        let mut graph = TypeGraph::new();
        let mut constraints = ConstraintSet::new();
        let mut registry = TraitRegistry::new();
        registry.register_impl("MyType", "Comparable");

        constraints.push(ConstraintKind::HasTrait(
            TypeId::Adt("MyType".into(), vec![]),
            "Comparable".into(),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &registry);
        assert!(result.is_ok());
    }

    #[test]
    fn solve_has_trait_missing_impl() {
        let mut graph = TypeGraph::new();
        let mut constraints = ConstraintSet::new();
        let registry = TraitRegistry::new();

        constraints.push(ConstraintKind::HasTrait(
            TypeId::Adt("MyType".into(), vec![]),
            "Comparable".into(),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &registry);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors[0].message.contains("does not implement trait"));
    }

    #[test]
    fn solve_has_trait_primitive_comparable() {
        let mut graph = TypeGraph::new();
        let mut constraints = ConstraintSet::new();
        let registry = TraitRegistry::new();

        constraints.push(ConstraintKind::HasTrait(
            TypeId::Primitive(Primitive::Int),
            "Comparable".into(),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &registry);
        assert!(result.is_ok());
    }

    #[test]
    fn solve_has_trait_unresolved_permissive() {
        let mut graph = TypeGraph::new();
        let var = graph.fresh();
        let mut constraints = ConstraintSet::new();
        let registry = TraitRegistry::new();

        constraints.push(ConstraintKind::HasTrait(
            TypeId::TypeVar(var),
            "Comparable".into(),
            span(),
        ));

        let result = solve_constraints(&constraints, &mut graph, &registry);
        assert!(result.is_ok());
    }
}
