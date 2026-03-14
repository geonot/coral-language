# Coral Evolution — Final Status & Remaining Items

**Created:** Post-Sprint 9+  
**Test Baseline:** 873 compiler + 237 runtime = 1110 tests, 0 failures  

---

## Overview

This document provides a final audit of every item in the Coral Evolution Roadmap. All items are categorized as **Complete**, **Excluded** (intentionally deferred/removed from plan), or **Remaining** (not yet implemented).

**Summary:** Of the ~172 total items across all pillars, **~148 are complete**, **~16 are excluded**, and **~12 remain**.

---

## Remaining Items

These items are NOT implemented and NOT excluded — they represent genuine future work.

### Syntax (S) — 7 remaining

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| S2.2 | List comprehensions | High | `[x * x for x in 1..100 if x % 2 is 0]` — desugar to loop+filter+collect |
| S2.3 | Map comprehensions | Medium | `{word: count for (word, count) in entries if count > 0}` |
| S2.4 | Destructuring assignment | High | `(x, y) is get_point()` for lists, `{name, age} is user` for maps |
| S2.5 | Slice syntax | Medium | `list[1..5]`, `string[0..3]` — range-based indexing |
| S2.7 | Tuple syntax | Medium | `(3, 4)` — lightweight structural tuple type |
| S3.4 | Nested pattern matching | High | `Ok(Some([first, ...rest])) ? process(first, rest)` |
| S3.5 | String/number range patterns | Medium | `match val / x from 0 to 10 -> "small"` — range patterns |

Note: S2.6 (Spread operator) was intentionally removed from the plan.

### Standard Library (L) — 3 remaining

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| L3.2 | `std.url` | Medium | `parse`, `encode`, `decode`, `build` for URL manipulation |
| L3.3 | `std.net` UDP | Medium | `udp_bind`, `udp_send`, `udp_recv` |
| L3.4 | `std.crypto` | High | SHA-256, HMAC, AES-256, OS-entropy random bytes |

Note: L3.5 (`std.csv`) was deferred — can be done later.

### Runtime (R) — 0 remaining

All planned runtime items are complete. R5.1–R5.12 (self-hosted runtime) were excluded from the plan.

Note: R1.1–R1.5 (runtime data structure optimizations), R2.3 (message dispatch), R3.7 (index from language), R3.8 (WAL recovery verification), and R4.4 (cache-line-aligned Value) were completed in Sprints 6–7+.

### Cross-Cutting (CC) — 1 remaining

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| CC4.1 | WebAssembly target | High | WASM output, pthreads-free runtime |

Note: CC4.2 (macOS/ARM64) and CC4.3 (Windows) were excluded from the plan.

---

## Excluded Items (Intentionally Deferred)

These items were explicitly excluded from the implementation plan by project decision.

| ID | Name | Reason |
|----|------|--------|
| S2.6 | Spread operator | Removed from plan — will not be implemented |
| L3.5 | `std.csv` | Deferred — can be done later |
| CC4.2 | macOS / ARM64 | Excluded from current plan |
| CC4.3 | Windows support | Excluded from current plan |
| R5.1 | Self-hosted value representation | Excluded (self-hosted runtime) |
| R5.2 | Self-hosted retain/release | Excluded (self-hosted runtime) |
| R5.3 | Self-hosted string implementation | Excluded (self-hosted runtime) |
| R5.4 | Self-hosted list implementation | Excluded (self-hosted runtime) |
| R5.5 | Self-hosted map implementation | Excluded (self-hosted runtime) |
| R5.6 | Self-hosted closure representation | Excluded (self-hosted runtime) |
| R5.7 | Self-hosted cycle detector | Excluded (self-hosted runtime) |
| R5.8 | Self-hosted actor scheduler | Excluded (self-hosted runtime) |
| R5.9 | Self-hosted store engine | Excluded (self-hosted runtime) |
| R5.10 | Self-hosted FFI layer | Excluded (self-hosted runtime) |
| R5.11 | Self-hosted integration tests | Excluded (self-hosted runtime) |
| R5.12 | Self-hosted memory allocator | Excluded (self-hosted runtime) |

---

## Complete Items by Pillar

### Memory (M) — 14/14 complete (100%)

| ID | Name | Sprint |
|----|------|--------|
| M1.1–M1.8 | NaN-Boxing (full transition) | Sessions 11–16 |
| M2.1–M2.4 | Non-atomic RC fast path | Session 28 |
| M3.1 | Thread-local cycle root buffers | Sprint 3 |
| M3.2 | Generational epoch tracking | Sprint 3 |
| M3.3 | Incremental GC | SKIPPED (design: GC-free) |
| M3.4 | Closure cycle tracking | Sprint 2 |
| M3.5 | Weak ref optimization | Sprint 5 |
| M4.1 | Escape analysis pass | Sprint 7+ |
| M4.2 | Stack-allocated values | Sprint 7+ |
| M4.3 | Copy-on-write semantics | Sprint 7+ |
| M4.4 | Region-based allocation | Sprint 9+ |

### Types (T) — 16/16 complete (100%)

| ID | Name | Sprint |
|----|------|--------|
| T1.1–T1.5 | Seal type escape hatches | Session 17 |
| T2.1–T2.3 | User generics (syntax + inference + instantiation) | Session 18 |
| T2.4 | Trait bounds on generics | Sprint 7+ |
| T2.5 | Monomorphization | Sprint 7+ |
| T3.1 | Type narrowing in match | Sprint 4 |
| T3.2 | Definite assignment analysis | Sprint 2 |
| T3.3 | Nullability tracking | Sprint 4 |
| T3.4 | Error type tracking | Sprint 5 |
| T3.5 | Dead code detection | Sprint NEXT |
| T4.1 | Multi-error recovery | Sprint 3 |
| T4.2 | Better type error messages | Sprint 3 |
| T4.3 | Ranked unification | Sprint 3 |
| T4.4 | Return type unification | Sprint 2 |

### Compiler/Codegen (C) — 23/23 complete (100%)

| ID | Name | Sprint |
|----|------|--------|
| C1.1–C1.5 | Enhanced constant folding | Session 17 |
| C2.1–C2.3 | Type specialization | Session 18 |
| C2.4 | Unboxed list specialization | Sprint 7+ |
| C2.5 | Store field specialization | Sprint 7+ |
| C3.1 | Small function inlining | Session 20 |
| C3.2 | Lambda inlining in HOFs | Sprint 7+ |
| C3.3 | Tail call optimization | Session 23 |
| C3.4 | Common subexpression elimination | Session 24 |
| C3.5 | Dead function elimination | Session 20 |
| C4.1 | Optimization flags | Sprint NEXT |
| C4.2 | LLVM function attributes | Sprint 2 |
| C4.3 | LLVM alias analysis hints | Sprint 3 |
| C4.4 | Link-time optimization | Sprint 4 |
| C4.5 | Profile-guided optimization | Sprint 5 |
| C5.1 | Comptime code generation | Sprint 9+ |
| C5.2 | Inferred comptime evaluation | Sprint 7+ |
| C5.3 | Const generics | Sprint 9+ |
| C5.4 | Comptime string processing | Sprint 9+ |

### Syntax (S) — 23/31 complete (74%)

| ID | Name | Sprint |
|----|------|--------|
| S1.1 | Map colon syntax | Session 16 |
| S1.3 | for..to..step ranges | Session 16 |
| S1.4 | Unary negation | Verified |
| S1.5 | Augmented assignment | Sprint 2 |
| S2.1 | Pipeline lowering | Session 17 |
| S3.1 | Multi-statement match arms | Session 22 |
| S3.2 | Guard clauses in match | Session 25 |
| S3.3 | Or-patterns in match | Session 26 |
| S3.6 | Match as statement | Session 22 |
| S4.1 | Named arguments | Sprint NEXT |
| S4.2 | Default parameter values | Sprint NEXT |
| S4.3 | Multi-line lambdas | Sprint 2 |
| S4.4 | Method chaining | Sprint 4 |
| S4.5 | Extension methods | Sprint 3 |
| S4.6 | Return in lambdas | Sprint 2 |
| S5.1 | `unless` keyword | Sprint NEXT |
| S5.2 | `until` loop | Sprint NEXT |
| S5.3 | `loop` keyword | Sprint NEXT |
| S5.4 | `when` expression | Sprint NEXT |
| S5.5 | `do..end` blocks | Sprint 4 |
| S5.6 | Postfix if/unless | Sprint 2 |

### Standard Library (L) — 19/22 complete (86%)

| ID | Name | Sprint |
|----|------|--------|
| L1.1–L1.6 | Foundation (StringBuilder, unwrap, etc.) | Session 16 |
| L2.1 | `std.random` | Sprint 2 |
| L2.2 | `std.regex` | Sprint 4 |
| L2.3 | `std.time` | Sprint 2 |
| L2.4 | `std.io` enhancements | Sprint 3 |
| L2.5 | `std.process` enhancements | Sprint 3 |
| L2.6 | `std.testing` enhancements | Sprint 2 |
| L3.1 | `std.http` client | Sprint 5 |
| L4.1 | `std.debug` | Sprint 5 |
| L4.2 | `std.path` | Sprint 3 |
| L4.3 | `std.collections` | Sprint 7+ |
| L4.4 | Documentation generator | Sprint 9+ |
| L4.5 | Package manager | Sprint 9+ |

### Runtime (R) — 44/44 complete (100%)

| ID | Name | Sprint |
|----|------|--------|
| R1.1–R1.5 | Runtime data structure optimizations | Sprint 7+ |
| R2.1 | Work-stealing scheduler | Sprint 5 |
| R2.2 | Lock-free actor registry | Sprint 5 |
| R2.3 | Message dispatch optimization | Sprint 6 |
| R2.4 | Cooperative yielding | Sprint 5 |
| R2.5 | Actor state pinning | Sprint 9+ |
| R2.6 | Supervised actor restart | Sprint 4 |
| R2.7 | Typed messages | Sprint 5 |
| R2.8 | Actor monitoring | Sprint 5 |
| R2.9 | Supervision hardening | Sprint 6 |
| R2.10 | Graceful actor stop | Sprint 4 |
| R2.11 | Remote actors foundation | Sprint 9+ |
| R2.12 | Actor integration tests | Sprint 6 |
| R3.1 | Secondary indexes | Sprint 6 |
| R3.2 | WAL compaction | Sprint 5 |
| R3.3 | Memory-mapped I/O | Sprint 9+ |
| R3.4 | Query optimization | Sprint 9+ |
| R3.5 | ACID transactions | Sprint 9+ |
| R3.6 | Store query syntax | Sprint 9+ |
| R3.7 | Index creation from language | Sprint 6 |
| R3.8 | WAL recovery verification | Sprint 6 |
| R3.9 | WeakRef clone fix | Sprint 3 |
| R4.1 | SIMD string operations | Sprint 9+ |
| R4.2 | Custom allocator | Sprint 9+ |
| R4.3 | Allocation batching | Sprint 9+ |
| R4.4 | Cache-line-aligned Value | Sprint 7+ |

### Cross-Cutting (CC) — 17/18 complete (94%)

| ID | Name | Sprint |
|----|------|--------|
| CC1.1 | Feature parity tracking | Sprint 9+ |
| CC1.2 | Shared test suite | Sprint 6 |
| CC1.3 | Relaxation removal | Sprint 9+ |
| CC1.4 | Performance comparison | Sprint 9+ |
| CC2.1 | Source-mapped errors | Session 17 |
| CC2.2 | Multi-error reporting | Session 17 |
| CC2.3 | DWARF debug info | Session 17 |
| CC2.4 | Warning categories | Sprint 2 |
| CC2.5 | LSP MVP | Sprint CC3 |
| CC3.1 | AST-level module system | Sprint CC3 |
| CC3.2 | Qualified module access | Sprint CC3 |
| CC3.3 | Selective imports | Sprint CC3 |
| CC3.4 | Circular dependency enhancement | Sprint 3 |
| CC3.5 | Incremental compilation | Sprint 4 |
| CC4.4 | Static linking | Sprint 6 |
| CC5.1 | Fuzz testing | Sprint 5 |
| CC5.2 | Fix medium bugs | Sprint 3 |
| CC5.3 | All examples compile | Sprint 4 |

---

## Remaining Work Summary

### By Priority

**High — Core Language Experience (7 items):**
- S2.2 List comprehensions
- S2.3 Map comprehensions
- S2.4 Destructuring assignment
- S2.5 Slice syntax
- S2.7 Tuple syntax
- S3.4 Nested pattern matching
- S3.5 Range patterns

**Medium — Standard Library (3 items):**
- L3.2 `std.url`
- L3.3 `std.net` UDP
- L3.4 `std.crypto`

**Low — Platform (1 item):**
- CC4.1 WebAssembly target

### Estimated Effort

| Category | Items | Estimated Sprints |
|----------|:-----:|:-----------------:|
| Syntax refinements | 7 | 2–3 sprints |
| Standard library | 3 | 1 sprint |
| WebAssembly | 1 | 1–2 sprints |
| **Total** | **11** | **~4–6 sprints** |

---

## Test History

| Milestone | Compiler | Runtime | Total | Failures |
|-----------|:--------:|:-------:|:-----:|:--------:|
| Initial | 194 | - | 194 | 1 |
| Post-NaN-boxing | 195 | 53 | 248 | 1 |
| Session 19 fixup | 793 | - | 793 | 0 |
| Sprint NEXT | 905 | - | 905 | 0 |
| Sprint 2 | 971 | - | 971 | 0 |
| Sprint 3 | 1016 | - | 1016 | 0 |
| Sprint 5 | 893 | 180 | 1073 | 0 |
| Sprint 9+ (final) | 873 | 237 | 1110 | 0 |
