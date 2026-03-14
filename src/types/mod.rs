pub mod core;
pub mod env;
mod solver;

pub use core::{Primitive, TypeId, TypeVarId, format_type};
pub use env::{
    AllocationHints, AllocationStrategy, Binding, FunctionRegistry, FunctionSig, Mutability,
    MutabilityEnv, Scope, SymbolUsage, TypeEnv, UsageMetrics,
};
pub use solver::{
    ConstraintKind, ConstraintOrigin, ConstraintSet, TraitRegistry, TypeError, TypeGraph, resolve,
    solve_constraints,
};
