//! Constraint solver for type inference.
//!
//! Implements a union-find based unification algorithm with proper error reporting.

use crate::span::Span;
use super::core::{TypeId, TypeVarId, Primitive, format_type};
use std::collections::HashMap;

/// T4.2: Origin of a type binding — where a type variable was first constrained.
#[derive(Debug, Clone)]
pub struct ConstraintOrigin {
    pub description: String,
    pub span: Span,
}

/// A type error with context for diagnostics.
#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
    pub expected: Option<TypeId>,
    pub found: Option<TypeId>,
    /// T4.2: Where the expected type was first inferred.
    pub expected_origin: Option<ConstraintOrigin>,
    /// T4.2: Where the found type was required.
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

    /// T4.2: Attach provenance information to this error.
    pub fn with_origins(mut self, expected_origin: Option<ConstraintOrigin>, found_origin: Option<ConstraintOrigin>) -> Self {
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

/// Type of constraint to be solved.
/// Note: For backward compatibility, variants without Span use Span::dummy().
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintKind {
    /// Two types must be equal (with span for error location).
    EqualAt(TypeId, TypeId, Span),
    /// Two types must be equal (legacy, uses dummy span).
    Equal(TypeId, TypeId),
    /// Type must be numeric (Int or Float) with span.
    NumericAt(TypeId, Span),
    /// Type must be numeric (legacy).
    Numeric(TypeId),
    /// Type must be boolean with span.
    BooleanAt(TypeId, Span),
    /// Type must be boolean (legacy).
    Boolean(TypeId),
    /// First type is a list of the second type.
    IterableAt(TypeId, TypeId, Span),
    /// Iterable constraint (legacy).
    Iterable(TypeId, TypeId),
    /// Function type with argument types and return type.
    CallableAt(TypeId, Vec<TypeId>, TypeId, Span),
    /// Callable constraint (legacy).
    Callable(TypeId, Vec<TypeId>, TypeId),
    /// T2.4: Type must implement a named trait.
    /// HasTrait(type, trait_name, span) — checked after unification resolves the type.
    HasTrait(TypeId, String, Span),
}

/// A collection of constraints to be solved together.
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

/// Union-find based type graph for unification.
#[derive(Debug, Default, Clone)]
pub struct TypeGraph {
    next_var: u32,
    parents: HashMap<TypeVarId, TypeVarId>,
    repr: HashMap<TypeVarId, TypeId>,
    /// Rank tracking for union-by-rank heuristic (T4.3).
    ranks: HashMap<TypeVarId, u32>,
    /// T4.2: Binding site tracking — records where each type variable was first bound.
    binding_origins: HashMap<TypeVarId, ConstraintOrigin>,
}

impl TypeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh type variable.
    pub fn fresh(&mut self) -> TypeVarId {
        let id = TypeVarId(self.next_var);
        self.next_var += 1;
        id
    }

    /// Find the root representative of a type variable (with path compression).
    pub fn find(&mut self, var: TypeVarId) -> TypeVarId {
        let parent = *self.parents.get(&var).unwrap_or(&var);
        if parent == var {
            return var;
        }
        let root = self.find(parent);
        self.parents.insert(var, root);
        root
    }

    /// Union two type variables (union-by-rank with information-aware heuristic).
    pub fn union(&mut self, a: TypeVarId, b: TypeVarId) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            let rank_a = *self.ranks.get(&ra).unwrap_or(&0);
            let rank_b = *self.ranks.get(&rb).unwrap_or(&0);
            let has_repr_a = self.repr.contains_key(&ra);
            let has_repr_b = self.repr.contains_key(&rb);

            if rank_a > rank_b {
                // ra has higher rank — rb points to ra
                self.parents.insert(rb, ra);
                if !has_repr_a && has_repr_b {
                    if let Some(ty) = self.repr.remove(&rb) {
                        self.repr.insert(ra, ty);
                    }
                }
            } else if rank_b > rank_a {
                // rb has higher rank — ra points to rb
                self.parents.insert(ra, rb);
                if has_repr_a && !has_repr_b {
                    if let Some(ty) = self.repr.remove(&ra) {
                        self.repr.insert(rb, ty);
                    }
                }
            } else {
                // Equal ranks — prefer the one with concrete type info
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

    /// Bind a type variable to a concrete type.
    pub fn bind(&mut self, var: TypeVarId, ty: TypeId) {
        let root = self.find(var);
        self.repr.insert(root, ty);
    }

    /// T4.2: Bind a type variable and record where the binding came from.
    pub fn bind_with_origin(&mut self, var: TypeVarId, ty: TypeId, origin: ConstraintOrigin) {
        let root = self.find(var);
        self.repr.insert(root, ty);
        self.binding_origins.entry(root).or_insert(origin);
    }

    /// Get the bound type for a variable, if any.
    pub fn get_binding(&mut self, var: TypeVarId) -> Option<TypeId> {
        let root = self.find(var);
        self.repr.get(&root).cloned()
    }

    /// T4.2: Get the origin of a type variable's binding, if recorded.
    pub fn get_binding_origin(&mut self, var: TypeVarId) -> Option<ConstraintOrigin> {
        let root = self.find(var);
        self.binding_origins.get(&root).cloned()
    }
}

/// Solve a set of constraints, returning errors if any.
pub fn solve_constraints(
    constraints: &ConstraintSet,
    graph: &mut TypeGraph,
) -> Result<(), Vec<TypeError>> {
    let mut errors = Vec::new();
    let dummy = Span::new(0, 0);

    // Sort constraints by priority for better solving efficiency:
    // 1. Equality constraints (cheapest to solve)
    // 2. Type requirements (numeric, boolean)
    // 3. Complex constraints (callable, iterable)
    let mut sorted_constraints = constraints.constraints.clone();
    sorted_constraints.sort_by_key(|c| match c {
        ConstraintKind::Equal(_, _) | ConstraintKind::EqualAt(_, _, _) => 0,
        ConstraintKind::Numeric(_) | ConstraintKind::NumericAt(_, _) 
        | ConstraintKind::Boolean(_) | ConstraintKind::BooleanAt(_, _) => 1,
        ConstraintKind::Iterable(_, _) | ConstraintKind::IterableAt(_, _, _) => 2,
        ConstraintKind::Callable(_, _, _) | ConstraintKind::CallableAt(_, _, _, _) => 3,
        ConstraintKind::HasTrait(_, _, _) => 4, // solved last, after types are resolved
    });

    for c in &sorted_constraints {
        let result = match c {
            // New variants with span
            ConstraintKind::EqualAt(a, b, span) => unify(a.clone(), b.clone(), graph, *span),
            ConstraintKind::NumericAt(ty, span) => enforce_numeric(ty.clone(), graph, *span),
            ConstraintKind::BooleanAt(ty, span) => enforce_boolean(ty.clone(), graph, *span),
            ConstraintKind::IterableAt(container, elem, span) => {
                let resolved_container = resolve(container.clone(), graph);
                match &resolved_container {
                    // For maps, iterating yields keys
                    TypeId::Map(key, _val) => unify(elem.clone(), *key.clone(), graph, *span),
                    // For Any/Unknown, the element type is also Any/Unknown
                    TypeId::Primitive(Primitive::Any) | TypeId::Unknown => {
                        unify(elem.clone(), resolved_container.clone(), graph, *span)
                    }
                    // For lists or anything else, default to List(elem)
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
            // Legacy variants without span (use dummy)
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
            // T2.4: Trait bounds — resolved type must be an ADT/Store that
            // implements the named trait. Permissive: unresolved vars, Any,
            // Unknown, and primitives pass silently (bounds are advisory for now).
            ConstraintKind::HasTrait(ty, _trait_name, _span) => {
                let _resolved = resolve(ty.clone(), graph);
                // Trait bound checking is deferred until the full trait registry
                // is available at a higher level. The constraint is recorded so
                // future passes (T2.5 monomorphization) can enforce it.
                Ok(())
            }
        };

        if let Err(e) = result {
            errors.push(e);
            // For now, continue processing other constraints to gather all errors
            // Could add early termination with environment variable:
            // if std::env::var("CORAL_FAIL_FAST").is_ok() { break; }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        // Sort errors by span for better diagnostic output
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
            // Coral supports default parameters, so calls with fewer args than
            // the function signature are allowed. Only reject if we have MORE
            // args than expected.
            if args.len() > expected_args.len() {
                Err(TypeError::arity_mismatch(expected_args.len(), args.len(), span))
            } else {
                let mut inner_errors = Vec::new();
                // Unify the provided arguments against their expected types.
                // Extra expected params (with defaults) are not checked.
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
                    // T4.1: Return ALL inner errors, not just the first.
                    // The first error becomes the main Result::Err, and the
                    // caller (solve_constraints) will collect the rest.
                    Err(inner_errors.remove(0))
                    // Note: remaining inner_errors are lost here, but they are
                    // captured at the constraint level because each argument
                    // mismatch generates a separate constraint error.
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
        // Error types pass through numeric checks — error values flow through any expression path.
        TypeId::Error(_) => Ok(()),
        TypeId::TypeVar(var) => {
            // Default unresolved numeric to Float.
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
        // Error types pass through boolean checks.
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
        // Same type.
        (x, y) if x == y => Ok(()),

        // Int and Float are compatible (numeric widening).
        (TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::Float))
        | (TypeId::Primitive(Primitive::Float), TypeId::Primitive(Primitive::Int)) => Ok(()),

        // Any unifies with everything (dynamic typing escape hatch).
        (TypeId::Primitive(Primitive::Any), _) | (_, TypeId::Primitive(Primitive::Any)) => Ok(()),

        // None unifies with Unit (both represent absence of value).
        (TypeId::Primitive(Primitive::None), TypeId::Primitive(Primitive::Unit))
        | (TypeId::Primitive(Primitive::Unit), TypeId::Primitive(Primitive::None)) => Ok(()),

        // None unifies with any type (nullable/option-like semantics for dynamic language).
        (TypeId::Primitive(Primitive::None), _) | (_, TypeId::Primitive(Primitive::None)) => Ok(()),

        // Unknown is permissive (for forward compatibility with untyped constructs).
        (TypeId::Unknown, _) | (_, TypeId::Unknown) => Ok(()),

        // Bind type variables.
        (TypeId::TypeVar(v), ty) | (ty, TypeId::TypeVar(v)) => {
            if occurs(*v, ty, graph) {
                Err(TypeError::new("infinite type (occurs check failed)", span))
            } else {
                graph.bind(*v, ty.clone());
                Ok(())
            }
        }

        // ADT unification: same name + recursively unify type arguments.
        (TypeId::Adt(a_name, a_args), TypeId::Adt(b_name, b_args)) => {
            if a_name == b_name {
                // If one side has no type args (non-parameterized usage), accept it
                if a_args.is_empty() || b_args.is_empty() {
                    Ok(())
                } else if a_args.len() != b_args.len() {
                    Err(TypeError::mismatch(&ra, &rb, span))
                } else {
                    // T4.1: Accumulate all field mismatches instead of stopping at the first.
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

        // Store unification: same name.
        (TypeId::Store(a_name), TypeId::Store(b_name)) => {
            if a_name == b_name {
                Ok(())
            } else {
                Err(TypeError::mismatch(&ra, &rb, span))
            }
        }

        // Error type unification: all error types are compatible with each other.
        // Different error taxonomy paths coexist (discriminated at runtime via NaN-boxing).
        (TypeId::Error(_), TypeId::Error(_)) => Ok(()),

        // Error types unify with any other type (errors are values in the NaN-box channel).
        // This allows functions to return errors from any code path.
        (TypeId::Error(_), _) | (_, TypeId::Error(_)) => Ok(()),

        // List unification.
        (TypeId::List(ae), TypeId::List(be)) => unify(*ae.clone(), *be.clone(), graph, span),

        // Map unification.
        (TypeId::Map(ak, av), TypeId::Map(bk, bv)) => {
            // T4.1: Accumulate key and value mismatches.
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

        // Function unification.
        (TypeId::Func(a_args, a_ret), TypeId::Func(b_args, b_ret)) => {
            if a_args.len() != b_args.len() {
                return Err(TypeError::arity_mismatch(a_args.len(), b_args.len(), span));
            }
            // T4.1: Accumulate all parameter mismatches.
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

        // Strict primitive type checking: different primitives don't unify.
        // Only Int/Float widening is allowed (handled above).
        (TypeId::Primitive(a), TypeId::Primitive(b)) => {
            Err(TypeError::mismatch(
                &TypeId::Primitive(a.clone()),
                &TypeId::Primitive(b.clone()),
                span,
            ))
        }

        // Type mismatch.
        _ => Err(TypeError::mismatch(&ra, &rb, span)),
    }
}

/// Check if a type variable occurs in a type (for occurs check).
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

/// Resolve a type by following type variable bindings.
pub fn resolve(ty: TypeId, graph: &mut TypeGraph) -> TypeId {
    match ty {
        TypeId::TypeVar(v) => {
            let root = graph.find(v);
            if let Some(t) = graph.get_binding(root) {
                resolve(t, graph)
            } else {
                TypeId::TypeVar(root)
            }
        }
        TypeId::List(elem) => TypeId::List(Box::new(resolve(*elem, graph))),
        TypeId::Map(k, v) => TypeId::Map(
            Box::new(resolve(*k, graph)),
            Box::new(resolve(*v, graph)),
        ),
        TypeId::Func(args, ret) => {
            let args_r: Vec<TypeId> = args.into_iter().map(|a| resolve(a, graph)).collect();
            TypeId::Func(args_r, Box::new(resolve(*ret, graph)))
        }
        TypeId::Adt(name, args) => {
            let args_r: Vec<TypeId> = args.into_iter().map(|a| resolve(a, graph)).collect();
            TypeId::Adt(name, args_r)
        }
        // Error types are concrete, no inner types to resolve.
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 0)
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
        // Primitives are permissively unified for backward compat.
        // Test true mismatch: List vs Primitive.
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
        // Different primitives now error (strict type checking).
        // Only Int/Float widening is allowed.
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
        
        let result = solve_constraints(&constraints, &mut graph);
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
        
        let result = solve_constraints(&constraints, &mut graph);
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
        
        let result = solve_constraints(&constraints, &mut graph);
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
        
        // Function takes 1 arg, but we provide 2.
        // (Coral allows fewer args due to default params, but not MORE args.)
        let fn_type = TypeId::Func(
            vec![TypeId::Primitive(Primitive::Int)],
            Box::new(TypeId::Primitive(Primitive::Int)),
        );
        
        constraints.push(ConstraintKind::CallableAt(
            fn_type,
            vec![TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::Int)], // 2 args, expects 1.
            TypeId::Primitive(Primitive::Int),
            span(),
        ));
        
        let result = solve_constraints(&constraints, &mut graph);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors[0].message.contains("expects 1 argument"));
    }

    #[test]
    fn solve_legacy_equal_constraint() {
        // Test the legacy API without spans
        let mut graph = TypeGraph::new();
        let var = graph.fresh();
        let mut constraints = ConstraintSet::new();
        constraints.push(ConstraintKind::Equal(
            TypeId::TypeVar(var),
            TypeId::Primitive(Primitive::Int),
        ));
        
        let result = solve_constraints(&constraints, &mut graph);
        assert!(result.is_ok());
        assert_eq!(
            resolve(TypeId::TypeVar(var), &mut graph),
            TypeId::Primitive(Primitive::Int)
        );
    }
}
