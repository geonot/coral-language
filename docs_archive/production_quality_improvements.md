# Coral Production Quality Improvements

_Created: 2026-01-06_
_Updated: 2026-01-06_
_Status: **RESOLVED**_

## Executive Summary

This document tracked non-production-quality issues in the Coral codebase following Phase 1-3 implementation. **All critical and high-priority issues have been fixed.**

---

## ✅ Completed Fixes

### 1. Function Parameters/Returns Fixed (CRITICAL)

**Location:** `src/codegen/mod.rs`

**Problem:** All user function parameters and returns were declared as `f64` instead of `Value*`, causing data corruption for non-numeric values.

**Fix Applied:** Changed function declarations to use `value_ptr_type` for all parameters and returns. Call sites now pass `PointerValue` directly instead of converting to f64.

**Verification:** Tests in `extern_and_inline.rs`, `modules.rs`, `smoke.rs` updated and passing.

---

### 2. Type System Strict Mode Enabled (HIGH)

**Location:** `src/types/solver.rs`

**Problem:** Type system allowed all primitive types to unify, e.g., `Bool` with `String`, hiding type errors until runtime.

**Fix Applied:** Changed permissive primitive unification to strict type checking:
- Only `Int`/`Float` widening is allowed
- String + anything polymorphism handled via special case in semantic analysis
- All other primitive mismatches now produce compile-time errors

**Verification:** Tests updated to expect strict behavior. All 120 tests passing.

---

### 3. Heterogeneous Map Support (MEDIUM)

**Location:** `src/semantic.rs`

**Problem:** Map literals required all values to have the same type, but Coral maps are heterogeneous at runtime.

**Fix Applied:** Maps now allow different value types in constraint collection. Key type still enforced as homogeneous for lookup semantics. Map value type is `Any`.

**Verification:** `compiles_full_language_fixture` test now passes with heterogeneous maps.

---

### 4. Constraint Spans for Better Errors (MEDIUM)

**Location:** `src/semantic.rs`

**Problem:** Type errors showed `0..0` span instead of actual source location.

**Fix Applied:** All constraint kinds now use span-aware variants:
- `ConstraintKind::Numeric` → `ConstraintKind::NumericAt(ty, span)`
- `ConstraintKind::Equal` → `ConstraintKind::EqualAt(a, b, span)`
- `ConstraintKind::Boolean` → `ConstraintKind::BooleanAt(ty, span)`
- `ConstraintKind::Callable` → `ConstraintKind::CallableAt(..., span)`

**Verification:** Error messages now show exact source locations.

---

### 5. Critical unwrap() Calls Improved (LOW)

**Location:** `src/parser.rs`, `src/semantic.rs`

**Problem:** Some `unwrap()` calls could panic on edge cases without helpful messages.

**Fix Applied:** Changed critical `unwrap()` to `expect()` with descriptive messages explaining the invariant being relied upon.

---

### 6. Compile-Time Constant Folding (LOW)

**Location:** `src/compiler.rs`

**Problem:** Expressions like `1 + 2` were computed at runtime.

**Fix Applied:** Added `fold_expressions()` optimization pass that:
- Folds arithmetic on integer/float literals (`1 + 2` → `3`)
- Folds boolean operations (`true and false` → `false`)
- Folds string concatenation at compile time (`"a" + "b"` → `"ab"`)
- Eliminates constant-condition ternaries (`true ? x ! y` → `x`)

**Verification:** LLVM IR now shows folded constants instead of runtime operations.

---

## Deferred Items

These items are not critical for Alpha release but may be addressed later:

### Code Organization
- Large files (`codegen/mod.rs`, `semantic.rs`, `parser.rs`) could be split into smaller modules
- Runtime library could be organized into separate files per feature

### Further Optimizations
- Dead code elimination after return statements
- MIR optimization passes
- Generic type instantiation/monomorphization

### Enhanced Type System
- Full enforcement of type annotations
- More detailed error messages with suggestions

---

## Testing Summary

All 120 tests pass after these changes:
- 24 unit tests
- 2 core spec tests
- 4 extern/inline tests
- 2 lexer layout tests
- 2 lexer snapshot tests
- 10 module tests
- 2 parser fixture tests
- 7 parser layout tests
- 15 parser logic tests
- 4 parser low-level tests
- 1 parser snapshot test
- 1 sanity test
- 28 semantic tests
- 16 smoke tests

---

_Document completed: All priority fixes implemented._
