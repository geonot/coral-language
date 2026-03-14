# Reference Compiler Issues & Shortcomings

Issues, limitations, bugs, workarounds, partial implementations, and other
shortcomings discovered during the self-hosted compiler parity review.

## Critical Bugs (Fixed)

### 1. CSE Cache Leak Across Basic Blocks
**File:** `src/codegen/mod.rs` (line ~1574)
**Status:** ✅ FIXED

The Common Subexpression Elimination cache was not cleared after emitting
if-statement branches. When a then-block and the fall-through code both
contained the same sub-expression (e.g. string interpolation `'{base}{name}'`
and `'{base}/{name}'` both start with `"" + base`), the CSE cache would
return the LLVM `IntValue` from the then-block while emitting code in the
merge block. This violates SSA domination: the then-block does not dominate
the merge block (they are siblings).

**Trigger:** Any function with string interpolation in an if-then branch and
similar interpolation after it. Specifically hit in `path_join` in
`self_hosted/module_loader.coral`.

**Symptom:** `lli` error: "Instruction does not dominate all uses!"

**Fix:** Added `ctx.cse_cache.clear()` after `self.builder.position_at_end(merge_bb)`.

---

## Known Test Failures

### 2. map_iterator_is_snapshot_after_mutation
**File:** `runtime/src/lib.rs` (test)
**Status:** ⚠️ UNFIXED (pre-existing)

The `map_iterator_is_snapshot_after_mutation` runtime test has been failing.
Map iterators are not snapshotting correctly after mutation.

---

## Panic-Based Error Handling

Several code paths use `panic!()` where they should return diagnostics or
`Result` types:

### 3. Compile-Time Assertion Panics
**File:** `src/compiler.rs` (lines 853, 871)

`assert_static` and regex validation failures at compile time trigger
`panic!()` instead of producing proper `Diagnostic` errors.

### 4. Store FFI Type Assumption Panics
**Files:** `runtime/src/store/ffi.rs` (lines 789, 796), `runtime/src/store/engine.rs` (line 1154)

FFI functions `panic!("expected list")`, `panic!("expected map")`, and
`panic!("Expected Int value")` when encountering unexpected value types.
These should return error values or propagate errors.

### 5. Actor Supervisor Type Mismatch Panics
**File:** `runtime/src/actor.rs` (lines 1940, 1947)

Supervisor restart decision handling panics on unexpected types instead of
using error propagation.

---

## Error Handling Risks

### 6. Excessive unwrap() in Module Loader
**File:** `src/module_loader.rs` (lines 124-538)

Over 10 `.unwrap()` / `.unwrap_or_else()` calls on `fs::canonicalize()` and
other I/O operations. Path normalization failures silently fall back to
non-canonical paths.

### 7. Null Pointer Returns for Error Conditions
**Files:** `runtime/src/list_ops.rs`, `runtime/src/bytes_ops.rs`,
`runtime/src/regex_ops.rs`, `runtime/src/io_ops.rs`

Many runtime functions return `coral_make_list(ptr::null(), 0)` or similar
on error conditions. Null pointer results are indistinguishable from valid
empty collections, providing no error propagation.

---

## Memory Safety Concerns

### 8. Reference Counting Debug-Only Assertions
**File:** `runtime/src/rc_ops.rs` (lines 92-173)

Reference count underflow checks use `debug_assert!()` — these are stripped
in release builds, allowing silent underflows that could lead to
use-after-free or double-free.

### 9. Unsafe Remote Value Deserialization
**File:** `runtime/src/remote.rs` (lines 54-56)

Reading memory length prefix with `*(ptr.sub(8) as *const u64)` assumes a
specific memory layout. No bounds checking or validation of the pointer.

### 10. SIMD Code Architecture-Specific
**File:** `runtime/src/simd_string.rs`

AVX2 SIMD string operations are x86-64 specific with hardcoded 32-byte chunk
sizes. Scalar fallbacks exist for non-x86 architectures, but the SIMD paths
have no runtime feature detection guards at the instruction level (only at
the function dispatch level).

---

## Partial Implementations

### 11. CSE Cache Scope Too Broad
**File:** `src/codegen/mod.rs`

While the specific if-branch leak was fixed, the CSE cache approach remains
fragile. The cache is cleared at various points (loop headers, branch entries)
but the pattern is ad-hoc. Any new control flow construct added to the codegen
must remember to clear the CSE cache, or risk the same domination bug. A more
robust approach would be to scope the cache per basic block or use LLVM's own
GVN pass.

### 12. Store Subsystem Type Validation
**Files:** `runtime/src/store/engine.rs`, `runtime/src/store/ffi.rs`,
`runtime/src/store/query.rs`

The persistent store subsystem lacks schema enforcement. Value types can change
without validation, query filtering panics on type mismatches, and object
indices are not bounds-checked in all code paths.

---

## Code Quality Notes

### 13. Parser/Codegen Size
- `src/parser.rs`: ~3,100 lines — large monolithic file
- `src/codegen/mod.rs`: ~3,700 lines — large monolithic file
- `src/semantic.rs`: ~4,000 lines — largest file in the compiler

These files would benefit from modular decomposition, though this is a
style/maintainability concern rather than a correctness issue.

### 14. Lexer unwrap() Density
**File:** `src/lexer.rs`

Over 30 `.unwrap()` calls on character extraction during tokenization. While
most are safe (the lexer validates bounds before extracting), they make it
harder to distinguish genuinely risky unwraps from safe ones.

---

## Summary

| Severity | Count | Status |
|----------|-------|--------|
| Critical bugs | 1 | ✅ Fixed (CSE cache) |
| Known test failures | 1 | ⚠️ Pre-existing |
| Panic error handling | 3 | ❌ Not fixed |
| Error handling risks | 2 | ❌ Not fixed |
| Memory safety concerns | 3 | ❌ Not fixed |
| Partial implementations | 2 | ❌ Not fixed |
| Code quality notes | 2 | Informational |
