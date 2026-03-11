# Coral Evolution — Remaining Roadmap

**Created:** March 10, 2026  
**Status:** Post-Sprint 5 — consolidates all remaining work from `LANGUAGE_EVOLUTION_ROADMAP.md`  
**Baseline:** 893 compiler + 180 runtime = 1073 tests, 0 failures  

---

## Summary

Sprints 1–5 completed **~100 items** across all 6 pillars. This document consolidates the **63 remaining items** into prioritized work streams. Items are grouped by theme and ordered by strategic value.

### Completion Status by Pillar

| Pillar | Done | Remaining | % Complete |
|--------|:----:|:---------:|:----------:|
| Syntax (S) | 23 | 8 | 74% |
| Types (T) | 14 | 2 | 88% |
| Compiler/Codegen (C) | 16 | 7 | 70% |
| Standard Library (L) | 14 | 7 | 67% |
| Memory (M) | 10 | 4 | 71% |
| Runtime (R) | 17 | 27 | 39% |
| Cross-Cutting (CC) | 10 | 8 | 56% |
| **Total** | **~104** | **63** | **62%** |

---

## Stream A: Core Language Maturity (Priority: High)

Items that improve daily developer experience and round out the language.

### Syntax Refinements

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| S2.2 | List comprehensions | High | `[x * x for x in 1..100 if x % 2 is 0]` — desugar to loop+filter+collect |
| S2.3 | Map comprehensions | Medium | `{word: count for (word, count) in entries if count > 0}` |
| S2.4 | Destructuring assignment | High | `(x, y) is get_point()` for lists, `{name, age} is user` for maps |
| S2.5 | Slice syntax | Medium | `list[1..5]`, `string[0..3]` — range-based indexing |
| S2.6 | Spread operator | Medium | `[...list1, ...list2]`, `{...map1, key: val}` |
| S2.7 | Tuple syntax | Medium | `(3, 4)` — lightweight structural tuple type |
| S3.4 | Nested pattern matching | High | `Ok(Some([first, ...rest])) ? process(first, rest)` |
| S3.5 | String/number range patterns | Medium | `match code / 200 to 299 ? "success"` |

### Type System

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| T2.4 | Trait bounds on generics | High | `type SortedList[T with Comparable]` — constrained polymorphism |
| T2.5 | Monomorphization | Very High | Generate specialized LLVM functions per concrete type argument |

### Standard Library

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| L3.2 | `std.url` | Medium | `parse`, `encode`, `decode`, `build` for URL manipulation |
| L3.3 | `std.net` UDP | Medium | `udp_bind`, `udp_send`, `udp_recv` |
| L3.4 | `std.crypto` | High | SHA-256, HMAC, AES-256, OS-entropy random bytes |
| L3.5 | `std.csv` | Medium | Parse/stringify CSV with headers, quoting, custom delimiters |
| L4.3 | `std.collections` | High | `Deque`, `PriorityQueue`, `OrderedMap`, `DefaultMap` |
| L4.4 | Documentation generator | High | Extract `##` doc comments → HTML/Markdown |
| L4.5 | Package manager | High | `coral.toml` manifest, dependency resolution, registry |

---

## Stream B: Compiler Intelligence (Priority: Medium-High)

Items that improve generated code quality and enable advanced metaprogramming.

### Codegen Specialization

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| C2.4 | Unboxed list specialization | Very High | `List[Number]` → contiguous `f64` array |
| C2.5 | Store field specialization | Very High | Known-field stores → struct layout, direct offset loads |
| C3.2 | Lambda inlining in HOFs | Very High | Inline lambda body into `map`/`filter` loop bodies |

### Compile-Time Features

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| C5.1 | Comptime code generation | Very High | `comptime` blocks producing AST/IR at compile time |
| C5.2 | Comptime assertions | Medium | `comptime_assert(size_of(Point) <= 64)` |
| C5.3 | Const generics | Very High | `type FixedArray[T, N]` with compile-time constant params |
| C5.4 | Comptime string processing | High | Regex compilation, format validation at compile time |

---

## Stream C: Memory & Performance (Priority: Medium)

Items that push Coral toward zero-overhead abstraction.

### Escape Analysis & Stack Allocation

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| M4.1 | Escape analysis pass | Very High | Mark non-escaping values as `StackEligible` |
| M4.2 | Stack-allocated values | Very High | LLVM `alloca` for eligible values, skip retain/release |
| M4.3 | Copy-on-write semantics | High | COW for shared values — share until mutation, then copy |
| M4.4 | Region-based allocation | High | Per-function arena, free-all on return |

### Runtime Data Structures

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| R1.1 | Branch-free tag dispatch | Medium | Jump tables or function pointer arrays for tag matching |
| R1.2 | Inline string threshold | Medium | Profile and potentially increase from 14 to 22 bytes |
| R1.3 | Small-list optimization | High | Lists ≤8 elements stored inline |
| R1.4 | Robin Hood hashing | Medium | Better cache behavior for map operations |
| R1.5 | Comparison fast paths | Medium | Bitwise number compare, `memcmp` for inline strings |
| R4.1 | SIMD string operations | High | AVX2/NEON for search, compare, case conversion |
| R4.2 | Custom allocator | Very High | Size-class pools, thread-local free lists |
| R4.3 | Allocation batching | High | `[1,2,3,4,5]` as single arena allocation |
| R4.4 | Cache-line-aligned Value | Medium | Fit Value to 64-byte cache line |

---

## Stream D: Actor System Completion (Priority: Medium)

The actor system is functional (scheduler, registry, monitoring, typed messages, restart, stop, yielding). These items harden it for production and distribution.

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| R2.3 | Message dispatch optimization | High | Integer tags or function pointers instead of string matching |
| R2.5 | Actor state pinning | High | Pin to worker thread, migrate only on steal |
| R2.9 | Supervision hardening | High | Restart budgets, time windows, escalation chains |
| R2.11 | Remote actors (foundation) | Very High | TCP transport, serialization, location-transparent proxy |
| R2.12 | Actor integration tests | High | Multi-level supervision, monitoring, typed messages E2E |

---

## Stream E: Store Engine Advancement (Priority: Medium)

WAL compaction is done. These items add indexing, transactions, and language-level query syntax.

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| R3.1 | Secondary indexes | High | B+ tree for ordered fields, hash index for equality |
| R3.3 | Memory-mapped I/O | High | `mmap` for binary store, OS page caching |
| R3.4 | Query optimization | High | Simple query planner (seq scan vs index lookup) |
| R3.5 | ACID transactions | Very High | MVCC for multi-operation commit/rollback |
| R3.6 | Store query syntax | High | Language-level `filter`/`find`/`aggregate` compilable to plans |
| R3.7 | Index creation from language | Medium | `store.index("field")` exposed to Coral code |
| R3.8 | WAL recovery verification | Medium | Automated crash → recover → verify tests |

---

## Stream F: Self-Hosted Runtime (Priority: Low — Long-Term)

Replace the Rust runtime (`libruntime.so`) with a Coral-native runtime for full bootstrap.

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| R5.1 | Value representation | Very High | NaN-boxed values in Coral matching Rust layout |
| R5.2 | Retain/release | High | Atomic refcounting in Coral |
| R5.3 | String implementation | High | SSO ≤15 bytes + heap allocation |
| R5.4 | List implementation | High | Dynamic array with push/pop/get/set/iteration |
| R5.5 | Map implementation | High | Open-addressing hash table |
| R5.6 | Closure representation | High | Captured environment struct + invoke |
| R5.7 | Cycle detector | High | Bacon's synchronous cycle collection |
| R5.8 | Actor scheduler | Very High | M:N scheduling, mailboxes, work queues |
| R5.9 | Store engine | Very High | WAL, binary/JSON storage, B+ tree indexes |
| R5.10 | FFI layer | High | C function declarations, syscall wrappers |
| R5.11 | Integration tests | High | Verify parity with Rust runtime |
| R5.12 | Memory allocator | Very High | Custom allocator via `mmap`/`brk` |

---

## Stream G: Cross-Cutting & Platform (Priority: Medium)

### Dual Compiler Parity

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| CC1.1 | Feature parity tracking | Ongoing | Rust vs Coral compiler feature matrix |
| CC1.2 | Shared test suite | High | Same programs → identical output from both compilers |
| CC1.3 | Self-hosted relaxation removal | Medium | Tighten scope-checking and boolean constraint relaxations |
| CC1.4 | Performance comparison | Medium | Compilation speed, binary size, runtime perf benchmarks |

### Compilation Targets

| ID | Name | Complexity | Description |
|----|------|:----------:|-------------|
| CC4.1 | WebAssembly target | High | WASM output, pthreads-free runtime |
| CC4.2 | macOS / ARM64 | Medium | Apple Silicon + cross-compilation CI |
| CC4.3 | Windows support | Medium | Platform-agnostic runtime, MSVC/MinGW |
| CC4.4 | Static linking | Medium | Fully static binaries embedding runtime |

---

## Suggested Sprint Sequencing

### Sprint 6 — Language Polish & Networking
Focus: Round out syntax and stdlib for day-to-day usability.
- S2.7 Tuple syntax
- S2.4 Destructuring assignment
- S2.5 Slice syntax
- S2.6 Spread operator
- L3.2 `std.url`
- L3.5 `std.csv`
- R3.8 WAL recovery verification
- CC4.4 Static linking
- R2.3 Message dispatch optimization
- S3.5 String/number range patterns

### Sprint 7 — Comprehensions, Crypto & Store Indexing
- S2.2 List comprehensions
- S2.3 Map comprehensions
- S3.4 Nested pattern matching
- L3.3 UDP support
- L3.4 `std.crypto`
- R3.1 Secondary indexes
- R3.7 Index from language level
- R2.9 Supervision hardening
- R2.12 Actor integration tests
- CC1.2 Shared test suite

### Sprint 8 — Type Intelligence & Specialization
- T2.4 Trait bounds on generics
- T2.5 Monomorphization
- C2.4 Unboxed list specialization
- C2.5 Store field specialization
- C3.2 Lambda inlining in HOFs
- C5.2 Comptime assertions
- L4.3 `std.collections`

### Sprint 9 — Performance Engineering
- M4.1 Escape analysis
- M4.2 Stack-allocated values
- M4.3 Copy-on-write
- R1.1–R1.5 Runtime data structure optimizations
- R4.4 Cache-line-aligned Value
- CC4.1 WASM target

### Sprint 10+ — Ecosystem & Bootstrap
- L4.4 Documentation generator
- L4.5 Package manager
- C5.1/C5.3/C5.4 Comptime features
- R2.5/R2.11 Actor pinning & remote actors
- R3.3–R3.6 Store engine advanced (mmap, queries, ACID)
- R4.1–R4.3 SIMD, custom allocator, batching
- M4.4 Region allocation
- CC4.2/CC4.3 macOS/Windows
- CC1.1/CC1.3/CC1.4 Compiler parity
- R5.1–R5.12 Self-hosted runtime (multi-sprint)

---

## Architecture Notes for Future Work

### Key Constraints
- **NaN-boxing**: All values are `i64`. Heap pointers encoded in lower 48 bits. Immediates are tag+payload. This is settled and should not change.
- **No GC**: Coral uses deterministic reference counting + cycle detection. This is a deliberate design choice for real-time suitability.
- **LLVM via Inkwell**: Codegen targets LLVM 16 via the inkwell crate. Major LLVM version upgrades may affect pass manager APIs.
- **Runtime is cdylib**: `libruntime.so` is dynamically linked. Static linking (CC4.4) would embed it.
- **Self-hosted compiler parity**: Changes to the Rust compiler should be reflected in `self_hosted/*.coral`. Feature matrix tracking (CC1.1) is essential.

### Test Infrastructure
- Compiler integration tests: `ModuleSource { name, path, source, import_directives, imports, exports }`
- E2E tests: Compile to LLVM IR → run via `lli -load target/debug/libruntime.so`
- Runtime unit tests: Standard Rust `#[test]` in runtime crate
- Fuzz testing: `cargo fuzz run fuzz_lexer/fuzz_parser` via libfuzzer

### Crate Dependencies Added Through Sprint 5
- `crossbeam-deque 0.8` — work-stealing deques
- `crossbeam-utils 0.8` — concurrency utilities  
- `dashmap 6` — concurrent hashmap
- `ureq 2` — synchronous HTTP client
- `regex 1` — regular expressions
