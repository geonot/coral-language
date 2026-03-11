//! Type system for Coral: HM-style inference with proper error propagation.
//!
//! This module implements:
//! - Type representations (primitives, generics, functions)
//! - Constraint generation and solving with union-find
//! - Type error collection with spans for diagnostics

pub mod core;
mod solver;
pub mod env;

pub use core::{TypeId, TypeVarId, Primitive, format_type};
pub use solver::{ConstraintKind, ConstraintOrigin, ConstraintSet, TypeGraph, TypeError, TraitRegistry, solve_constraints, resolve};
pub use env::{
    Binding, Scope, TypeEnv, FunctionSig, FunctionRegistry,
    Mutability, MutabilityEnv, AllocationStrategy, AllocationHints,
    SymbolUsage, UsageMetrics,
};
