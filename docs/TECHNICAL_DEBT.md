# Coral Technical Debt & Issue Tracker

_Created: 2026-01-06_

This document provides a rigorous, critical evaluation of outstanding technical debt, shortfalls, issues, gaps, and problems in the Coral codebase.

---

## 1. Critical Issues (Must Fix for Alpha)

### 1.1 Generic Types Not Instantiated

**Location**: `src/types/`
**Severity**: 🔴 Critical
**Impact**: Type safety holes, potential runtime crashes

**Description**: 
Generic type declarations (`List[T]`, `Map[K,V]`) are parsed and stored but never actually instantiated or checked. The type system treats all collections as `Any`, providing no compile-time safety.

**Evidence**:
```rust
// src/types/core.rs - GenericType variant exists but isn't used in unification
pub enum TypeId {
    GenericType(String, Vec<TypeId>), // Never properly instantiated
    ...
}
```

**Tasks**:
- [ ] Implement generic instantiation in `solver.rs`
- [ ] Add type parameter tracking in `TypeEnv`
- [ ] Test list/map element type checking
- [ ] Add type error for mismatched element types

---

### 1.2 Reference Counting Cycle Leaks

**Location**: `runtime/src/lib.rs`
**Severity**: 🔴 Critical
**Impact**: Memory leaks in cyclic data structures

**Description**:
The runtime uses reference counting without any cycle detection. Any cyclic data structure (linked lists with back-pointers, graphs, etc.) will leak memory.

**Evidence**:
```rust
// No cycle detection in coral_value_release
pub extern "C" fn coral_value_release(handle: ValueHandle) {
    // Simply decrements refcount - no cycle breaking
}
```

**Tasks**:
- [ ] Implement weak references for cycle-prone structures
- [ ] Add cycle collector (Bacon-Rajan or simpler)
- [ ] Document cycle-safe patterns for users
- [ ] Consider arena allocation for short-lived cycles

---

### 1.3 Module System is Naive String Expansion

**Location**: `src/module_loader.rs`
**Severity**: 🟠 High
**Impact**: No incremental compilation, namespace collisions, duplicate work

**Description**:
The `use` directive simply expands module contents inline. There's no:
- Module caching
- Fingerprinting for incremental builds
- Proper scoping (everything flattens to global)
- Dependency cycle detection

**Evidence**:
```rust
// module_loader.rs - just string replacement
pub fn expand_uses(source: &str, ...) -> Result<String, ...> {
    // Naively replaces `use x.y` with file contents
}
```

**Tasks**:
- [ ] Implement module cache with content hashing
- [ ] Add proper namespace scoping
- [ ] Detect and report circular imports
- [ ] Track module dependencies for incremental compilation

---

## 2. High Priority Issues

### 2.1 Actors Use String-Based Dispatch

**Location**: `runtime/src/actor.rs`, `src/codegen/mod.rs`
**Severity**: 🟠 High  
**Impact**: Performance overhead, no compile-time handler validation

**Description**:
Actor message dispatch compares message names as strings at runtime. This is functional but inefficient and prevents compile-time verification of handler existence.

**Tasks**:
- [ ] Intern message names to numeric IDs
- [ ] Generate dispatch table at compile time
- [ ] Add compile-time check that message handlers exist

---

### 2.2 No Typed Message Contracts

**Location**: `runtime/src/actor.rs`
**Severity**: 🟠 High
**Impact**: Runtime type errors instead of compile-time

**Description**:
All actor messages pass payload as `Any`. There's no way to specify or enforce message types.

**Tasks**:
- [ ] Design message type syntax
- [ ] Implement compile-time message type checking
- [ ] Generate typed envelope structures

---

### 2.3 Store Methods Return Incorrect Type

**Location**: `src/codegen/mod.rs`
**Severity**: 🟠 High
**Impact**: Potential value corruption

**Description**:
Store methods are declared to return `f64` even though they actually return `Value*` pointers. This mismatch can cause issues.

**Evidence**:
```rust
// Comment in codegen says f64 but implementation uses ptr
// "Return ptr (CoralValue*) instead of f64 to avoid corruption"
```

**Tasks**:
- [ ] Audit all store method signatures
- [ ] Ensure consistent Value* return types
- [ ] Add tests for store method return values

---

## 3. Medium Priority Issues

### 3.1 Large File Size - `runtime/src/lib.rs`

**Location**: `runtime/src/lib.rs`
**Severity**: 🟡 Medium
**Impact**: Maintainability, compile times

**Description**:
At 3804 lines, this file is too large. It contains value handling, list operations, map operations, string operations, and more.

**Tasks**:
- [ ] Extract `value.rs` - Value type and basic operations
- [ ] Extract `list.rs` - List operations
- [ ] Extract `map.rs` - Map operations  
- [ ] Extract `string.rs` - String operations
- [ ] Extract `bytes.rs` - Bytes operations

---

### 3.2 Large File Size - `src/codegen/mod.rs`

**Location**: `src/codegen/mod.rs`
**Severity**: 🟡 Medium
**Impact**: Maintainability

**Description**:
At 3264 lines, codegen is large but has better organization than runtime. Already has `runtime.rs` extracted.

**Tasks**:
- [ ] Extract `expression.rs` - Expression emission
- [ ] Extract `statement.rs` - Statement emission
- [ ] Extract `function.rs` - Function/method emission
- [ ] Extract `store.rs` - Store/actor constructor emission

---

### 3.3 MIR Has No Optimization Passes

**Location**: `src/mir.rs`, `src/mir_lower.rs`
**Severity**: 🟡 Medium
**Impact**: Suboptimal generated code

**Description**:
MIR exists but is essentially a trivial transformation. No optimizations are performed.

**Missing Optimizations**:
- [ ] Constant folding (partially done in `compiler.rs`)
- [ ] Dead code elimination
- [ ] Common subexpression elimination
- [ ] Inlining
- [ ] Escape analysis

---

### 3.4 No Atomic Refcount for Actor Sharing

**Location**: `runtime/src/lib.rs`
**Severity**: 🟡 Medium
**Impact**: Data races when actors share values

**Description**:
While `AtomicU64` is used for refcounts, not all paths use atomic operations correctly when values cross actor boundaries.

**Evidence**:
```rust
// Value struct uses AtomicU64 but some operations use Relaxed ordering
pub refcount: AtomicU64,
```

**Tasks**:
- [ ] Audit all refcount operations for correct ordering
- [ ] Use Acquire/Release for cross-thread sharing
- [ ] Add stress tests for concurrent refcount operations

---

## 4. Low Priority Issues

### 4.1 Inconsistent Error Handling

**Location**: Various
**Severity**: 🟢 Low
**Impact**: Potential panics, poor error messages

**Description**:
Some code paths use `unwrap()` where proper error handling should occur.

**Tasks**:
- [ ] Audit for unwrap() in library code
- [ ] Replace with proper Result propagation
- [ ] Add context to errors

---

### 4.2 Missing Documentation

**Location**: Various
**Severity**: 🟢 Low
**Impact**: Onboarding difficulty

**Tasks**:
- [ ] Add module-level docs to all src/ files
- [ ] Document public APIs in runtime
- [ ] Add architecture overview document

---

### 4.3 No Benchmarks

**Location**: N/A
**Severity**: 🟢 Low
**Impact**: No performance regression detection

**Tasks**:
- [ ] Add Criterion benchmarks for runtime operations
- [ ] Add benchmarks for compiler phases
- [ ] Set up CI performance tracking

---

## 5. Architectural Gaps

### 5.1 No Effect System

**Impact**: Cannot distinguish pure functions from IO/Actor operations

**Description**:
There's no way to track or enforce effects. Functions that do IO or spawn actors look the same as pure functions.

**Tasks**:
- [ ] Design effect syntax (`fn foo() : IO[String]`)
- [ ] Implement effect inference
- [ ] Add effect checking

---

### 5.2 No Error/Absence Model

**Impact**: No standard way to represent errors or missing values

**Description**:
The planned `Value` flags (ERR, ABSENT) aren't implemented. There's no standard error handling pattern.

**Tasks**:
- [ ] Implement Result/Option types
- [ ] Add error propagation operator (`?`)
- [ ] Design error value representation

---

### 5.3 No Trait/Interface System

**Impact**: No polymorphism beyond `Any`

**Description**:
There's no way to define or implement interfaces/traits.

**Tasks**:
- [ ] Design trait syntax
- [ ] Implement trait bounds
- [ ] Add trait method dispatch

---

## 6. Testing Gaps

### 6.1 Missing Test Categories

| Category | Current | Target |
|----------|---------|--------|
| Type error tests | ~20 | 50+ |
| Actor tests | ~5 | 20+ |
| Store tests | ~3 | 15+ |
| Runtime stress tests | 1 | 10+ |
| Memory leak tests | 0 | 10+ |
| Concurrent tests | 0 | 10+ |

### 6.2 No Fuzzing

**Tasks**:
- [ ] Add lexer fuzzer
- [ ] Add parser fuzzer
- [ ] Add runtime fuzzer

---

## 7. Security Concerns

### 7.1 Inline Assembly

**Location**: `src/codegen/mod.rs`
**Status**: Gated behind `CORAL_INLINE_ASM` env var

**Risk**: Arbitrary code execution if enabled

**Tasks**:
- [ ] Document security implications
- [ ] Consider removing or further restricting

### 7.2 Memory Safety

**Location**: `runtime/src/lib.rs`
**Status**: Uses `unsafe` for FFI

**Risk**: Potential memory corruption from misuse

**Tasks**:
- [ ] Audit all unsafe blocks
- [ ] Add safety comments
- [ ] Consider safer abstractions

---

## 8. Priority Matrix

| Issue | Severity | Effort | Priority Score |
|-------|----------|--------|----------------|
| Generic instantiation | Critical | Medium | **P0** |
| Cycle detection | Critical | High | **P0** |
| Module caching | High | Medium | **P1** |
| String dispatch | High | Low | **P1** |
| Typed messages | High | Medium | **P1** |
| Runtime split | Medium | Medium | **P2** |
| Codegen split | Medium | Medium | **P2** |
| MIR optimizations | Medium | High | **P2** |
| Effect system | Low | High | **P3** |
| Trait system | Low | High | **P3** |

---

## 9. Action Items Summary

### This Week
1. [ ] Fix generic type instantiation
2. [ ] Add cycle detection (at least weak refs)
3. [ ] Audit store method return types

### This Month
1. [ ] Implement module caching
2. [ ] Add named actor registry
3. [ ] Split runtime/lib.rs
4. [ ] Add 20+ new tests

### This Quarter
1. [ ] Full MIR optimization pipeline
2. [ ] Effect system design
3. [ ] Trait system design
4. [ ] Performance benchmarks
