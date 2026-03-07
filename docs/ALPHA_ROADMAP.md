# Coral — Alpha Roadmap

**Authority:** This is the singular, authoritative roadmap for reaching Coral's alpha release.  
**Last updated:** March 2026

---

## Alpha Definition

Coral alpha means:

1. **Complete standard library** — all modules specified in `STANDARD_LIBRARY_SPEC.md` are functional
2. **Self-hosted compiler** — Coral compiler written in Coral, producing identical output to the Rust compiler
3. **Self-hosted runtime** — Coral runtime written in Coral with equivalent functionality to the Rust runtime
4. **Native performance** — LLVM backend with constant folding, dead-code elimination, escape analysis, numeric unboxing
5. **Functional actors** — typed messages, supervision trees, monitoring, timers, clean shutdown
6. **Persistent stores** — full CRUD, WAL, indexing, ACID transactions, store queries from language level
7. **Language design adherence** — pure type inference, `is` binding, method-based equality, value-error model, pipeline syntax

---

## Current State (March 2026)

### What Works
- **Compiler pipeline:** Lexer → Parser → Semantic → Lower → LLVM Codegen — all stages operational
- **LLVM backend:** Compiles to native binaries via Inkwell (LLVM 16)
- **Core language:** Functions, closures, ADTs, pattern matching, if/elif/else, while, for..in, return, break/continue, ternary, pipeline, lambda, match expressions, guard statements, template strings, index/subscript syntax
- **Type system:** Constraint-based inference, ADT types with type parameters, method resolution, scope checking, forward references, None/Unit distinction, unified scope-based TypeEnv, None unifies with any type (nullable semantics), exhaustiveness checking with warnings + nested patterns
- **Runtime (Rust):** RC with cycle detection, actor M:N scheduler, bounded mailboxes, persistent store engine, WAL, 220+ FFI functions; split into 12+ submodules (string_ops, list_ops, map_ops, math_ops, io_ops, bytes_ops, tagged_ops, actor_ops, json_ops, time_ops, encoding_ops, sort_ops)
- **Standard library:** 100% populated — all 20 modules (1,700+ lines): math, io, process, string, list, map, set, result, option, fmt, char, net, time, json, encoding, sort, bytes, bit, testing, prelude
- **Self-hosted compiler:** All 7 modules (7,343 lines) compile to LLVM IR — lexer, parser, lower, module_loader, semantic, codegen, compiler. **Not yet execution-verified.**
- **Tests:** 755+ tests (all passing), 26 store E2E tests, 100 coverage expansion tests, 50 Phase B tests, 18 self-hosting tests
- **Codegen:** Split into mod.rs + builtins.rs + closures.rs + match_adt.rs + store_actor.rs + runtime.rs (no file >2,100 lines)
- **Store persistence codegen:** `coral_store_open`/`coral_store_create`/`coral_store_save_all` wired in LLVM codegen
- **Examples:** 5 of 7 examples compile (calculator, data_pipeline, fizzbuzz, hello, traits_demo)

### What's Broken / Missing
- **0 failing tests** — all 755+ tests pass
- **Self-hosted compiler bugs:** SH-1 (elif format mismatch), SH-2 (actor message dispatch), SH-3 (error value field name), SH-4 (range returns empty list)
- **Medium bugs:** P6 (single-error), S6 (member access fallback), S8 (pipeline type inference), R11 (single work queue), ML1 (text-based exports), ML2 (no namespacing)

### Phase A Completed
- **TS-1** ✅ Generic instantiation — `Adt(String, Vec<TypeId>)` carries type arguments
- **TS-2** ✅ Dual-track TypeEnv removed — scopes is single source of truth
- **TS-3** ✅ None/Unit distinction — `Primitive::None` added, `make_absent()` emitted for None
- **TS-7** ✅ peek_kind() returns `&TokenKind` (zero-clone)
- **TS-8** ✅ Keywords in call arguments — already working
- **TS-10** ✅ Wildcard match pattern codegen — fixed
- **PS-1** ✅ Store persistence codegen — open/create/save_all wired
- **PS-3** ✅ Store field display — `.get()` dispatches by tag, `length`/`count` field conflict resolved
- **SL-1/2/3** ✅ math/io/process FFI fully wired with expanded stdlib modules
- **IQ-4** ✅ Fixed examples — fizzbuzz, chat_server/http_server indent cleanup

### Phase B Completed
- **TS-9** ✅ Exhaustiveness checking — changed from hard errors to warnings, recursive nested patterns
- **IQ-2** ✅ Split `codegen/mod.rs` — 5,047 → 2,009 + 1,245 (builtins.rs) + 406 (store_actor.rs) + 1,313 (runtime.rs)
- **IQ-3** ✅ Split `runtime/src/lib.rs` — 5,873 → 2,552 + 8 submodules (string/list/map/math/io/bytes/tagged/actor)
- **IQ-5** ✅ Test coverage — 740+ tests (target was 500+), including 100 coverage expansion + 50 Phase B tests
- **SL-4** ✅ `set.coral` — intersection, difference, subset operations (108 lines)
- **SL-5** ✅ `map.coral` — map_keys, flat_map, group_by, iteration support (160 lines)
- **SL-6** ✅ `string.coral` — chars, lines, string operations (112 lines)
- **SL-7** ✅ `bytes.coral` — from_hex, contains, find (31 lines)
- **SL-8** ✅ `json.coral` + runtime — json_parse, json_serialize, json_serialize_pretty (25 lines + json_ops.rs)
- **SL-9** ✅ `time.coral` + runtime — time_now, time_components, time_format_iso, time_elapsed (59 lines + time_ops.rs)
- **SL-10** ✅ `fmt.coral` — string formatting utilities (114 lines)
- **SL-11** ✅ `sort.coral` — natural sort, comparison-based sorting (44 lines + sort_ops.rs)
- **SL-12** ✅ `encoding.coral` + runtime — base64/hex encode/decode (20 lines + encoding_ops.rs)
- **SL-13** ✅ `net.coral` — TCP listen/accept/connect/read/write/close (83 lines)
- **SL-14** ✅ Error propagation — AST + parser + codegen already complete
- **SL-15** ✅ `testing.coral` — assert_eq, assert_true, assert_false, test runner integration (66 lines)
- **SL-16** ✅ Stdlib test suite — 49 tests in tests/stdlib.rs exercising module functions
- **SC-1** ✅ Self-hosted lexer — template string interpolation, error recovery, 529 lines, compiles to LLVM IR
- **SC-2** ✅ Self-hosted parser — template strings, list patterns, error recovery, 1,800 lines, compiles to LLVM IR
- **SC-4** ✅ Front-end verification — 7 tests: lexer/parser load, compile, coverage, all stdlib modules compile
- **PS-8** ✅ Store E2E tests — 26 tests: CRUD lifecycle, multi-instance, methods, traits, list fields, function passing
- **Type system** ✅ None unifies with any type — nullable/option-like semantics for dynamic language
- **Builtins** ✅ `string_length`, `parse_number` recognized + codegen wired

---

## Work Streams

The roadmap is organized into six parallel-capable work streams. Dependencies are noted where they exist.

### Stream 1: Type System & Semantics

Fix the type system to actually enforce correctness. All type checking works via inference — no annotations.

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| TS-1 | **Proper generic instantiation (T2)** — substitute type params in `instantiate_generic`; specialize `List[T]`, `Map[K,V]`, `Option[T]`, `Result[T,E]` during constraint solving | Critical | 25h | — |
| TS-2 | **Remove dual-track TypeEnv (T4)** — unify `scopes` and `symbols` into single mechanism | High | 5h | — |
| TS-3 | **Distinguish None from Unit (S5)** — add `Primitive::None`/`Absent`, stop conflating them | High | 4h | — |
| TS-4 | **Fix member access type inference (S6)** — generate Store/Type-specific member constraints before Map fallback | High | 10h | — |
| TS-5 | **Fix pipeline type inference (S8)** — thread left-hand type through pipeline desugaring | Medium | 5h | — |
| TS-6 | **Multi-error recovery (P6)** — parser continues after errors, reports multiple diagnostics | Medium | 15h | — |
| TS-7 | **Fix peek_kind() clones (P7)** — return `&TokenKind` or use discriminant comparison | Low | 3h | — |
| TS-8 | **Fix keywords in call arguments** — `log(a and b)` should parse correctly | Medium | 5h | — |
| TS-9 | **Exhaustiveness checking for nested ADTs** ✅ — warnings + recursive nested patterns | Medium | 5h | TS-1 |
| TS-10 | **Fix wildcard match pattern codegen** — `_` arm in ADT match produces no output (1 failing test) | High | 4h | — |

**Subtotal: ~81 hours**

---

### Stream 2: Standard Library Completion

Complete all modules per `STANDARD_LIBRARY_SPEC.md`. Currently ~55% complete.

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| SL-1 | **Wire up `math.coral` trig/exp functions** — `sqrt`, `pow`, `log`, `sin`, `cos`, `tan`, `exp` via libm FFI | High | 4h | — |
| SL-2 | **Wire up `io.coral` file operations** — ensure `read_file`, `write_file`, `append_file`, `file_exists`, `delete_file`, `list_dir`, `create_dir` all work via runtime FFI | High | 8h | — |
| SL-3 | **Wire up `process.coral` functions** — `args`, `env_get`, `env_set`, `exit`, `exec`, `cwd` | High | 6h | — |
| SL-4 | **Complete `set.coral`** ✅ — intersection, difference, symmetric_difference, is_subset, is_superset | Medium | 4h | — |
| SL-5 | **Complete `map.coral`** ✅ — map_keys, flat_map, group_by, map iteration | Medium | 4h | — |
| SL-6 | **Add `string.coral` iteration** ✅ — chars(s) and lines(s) | Medium | 3h | — |
| SL-7 | **Add `bytes.coral` operations** ✅ — from_hex, contains, find | Low | 3h | — |
| SL-8 | **Create `std/json.coral`** ✅ — JSON parse/serialize with runtime FFI | High | 10h | — |
| SL-9 | **Create `std/time.coral`** ✅ — time operations with runtime FFI | Medium | 6h | — |
| SL-10 | **Create `std/fmt.coral`** ✅ — string formatting utilities | Medium | 5h | — |
| SL-11 | **Create `std/sort.coral`** ✅ — natural sort + comparison-based sorting | Low | 4h | — |
| SL-12 | **Create `std/encoding.coral`** ✅ — base64, hex encode/decode | Medium | 6h | — |
| SL-13 | **Complete `net.coral`** ✅ — TCP client/server via runtime FFI | High | 20h | — |
| SL-14 | **Error propagation operator (`?`)** ✅ — AST + parser + codegen complete | High | 15h | TS-1 |
| SL-15 | **Add `std/testing.coral`** ✅ — assertion functions, test runner integration | Medium | 5h | — |
| SL-16 | **Stdlib test suite** ✅ — 49 tests in tests/stdlib.rs | Medium | 12h | SL-1..SL-15 |

**Subtotal: ~115 hours**

---

### Stream 3: Actor System Completion

Complete the actor framework per `ACTOR_SYSTEM_COMPLETION.md`. Current state: spawn, send, M:N scheduler, bounded mailboxes, backpressure, @handler syntax, named actors, timers, clean shutdown all working.

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| AC-1 | **Typed messages** — `@messages(MessageType)` annotation + compile-time type checking at `send()` call sites | High | 10h | TS-1 |
| AC-2 | **Actor monitoring** — `monitor(actor)` / `demonitor(actor)` + `ActorDown` message delivery | High | 8h | — |
| AC-3 | **Supervision hardening** — restart budget enforcement, time windows, escalation chains | High | 10h | — |
| AC-4 | **Graceful actor stop** — flush mailbox before termination | Medium | 4h | — |
| AC-5 | **Work-stealing scheduler (R11)** — replace single work queue with per-worker channels or crossbeam-deque | Medium | 8h | — |
| AC-6 | **Remote actors (foundation)** — TCP transport, CACT protocol serialization, remote proxy, location-transparent lookup | Low | 20h | AC-1, AC-2, AC-3 |
| AC-7 | **Actor integration tests** — supervision trees, monitoring, typed messages, multi-level restart scenarios | High | 6h | AC-1..AC-4 |

**Subtotal: ~66 hours**

---

### Stream 4: Persistent Stores

Complete store system per `PERSISTENT_STORE_SPEC.md`. Current state: runtime WAL + storage engine works, store fields wired in create/update, `get_by_uuid` indexed.

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| PS-1 | **Store persistence codegen** — complete LLVM codegen for store save/load/query operations | Critical | 15h | — |
| PS-2 | **Store query syntax** — add language-level syntax for store queries (filter, find, etc.) | High | 12h | PS-1 |
| PS-3 | **Fix store field display** — `data_pipeline.coral` shows `()` for store field values | High | 4h | — |
| PS-4 | **Store indexing from language level** — expose B+ tree index creation and query to Coral code | Medium | 8h | PS-1 |
| PS-5 | **ACID transactions** — MVCC with isolation levels, transaction syntax in language | Medium | 15h | PS-1 |
| PS-6 | **WAL recovery verification** — write data → simulate crash → recover → verify integrity | Medium | 4h | PS-1 |
| PS-7 | **Fix WeakRef clone semantics** — shares registry IDs (use-after-free risk) | Medium | 5h | — |
| PS-8 | **Store E2E tests** ✅ — 26 tests: CRUD lifecycle, traits, function passing | High | 8h | PS-1, PS-2 |

**Subtotal: ~71 hours**

---

### Stream 5: Self-Hosted Compiler

Coral compiler written in Coral. **Phase C complete** — all 7 modules (7,343 lines) compile to LLVM IR. 18 regression tests. **Not yet execution-verified** — cross-module bugs SH-1..SH-4 must be fixed before bootstrap.

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| SC-1 | **Complete self-hosted lexer** ✅ — 528 lines, compiles to LLVM IR | High | 8h | — |
| SC-2 | **Complete self-hosted parser** ✅ — 1,800 lines, compiles to LLVM IR | High | 15h | SC-1 |
| SC-3 | **Module loader in Coral** ✅ — `use` resolution, file merging, circular import detection, 284 lines | High | 12h | SL-2 |
| SC-4 | **Front-end verification** ✅ — 18 tests: load, compile, coverage, stdlib modules | High | 8h | SC-1, SC-2 |
| SC-5 | **Semantic analysis in Coral** ✅ — scope tracking, constraint generation, type inference, ADT resolution, 1,677 lines | Critical | 60h | SC-2, TS-1 |
| SC-6 | **Lowering pass in Coral** ✅ — desugar pipelines, placeholders, guard statements, 665 lines | Medium | 15h | SC-5 |
| SC-7 | **LLVM IR text emission** ✅ — emit `.ll` text strings from Coral, 2,109 lines | Critical | 60h | SC-5, SC-6 |
| SC-8 | **Compiler integration** ✅ — full pipeline: lex → parse → lower → analyze → fold → generate, 280 lines | High | 15h | SC-7 |
| SC-9 | **Bootstrap test** — self-hosted compiler compiles itself; output matches Rust compiler | Critical | 20h | SC-8 |
| SC-10 | **Performance comparison** — self-hosted vs Rust compiler speed (target: within 5x) | Medium | 5h | SC-9 |

**Subtotal: ~218 hours**

---

### Stream 6: Self-Hosted Runtime

Rewrite the Coral runtime in Coral per `SELF_HOSTED_RUNTIME_SPEC.md`. Currently entirely in Rust (~23,000 lines).

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| SR-1 | **Value representation in Coral** — tagged 32-byte values with inline/heap layout | Critical | 20h | SC-7 |
| SR-2 | **Retain/release in Coral** — refcounting via atomic operations (inline asm or FFI) | Critical | 15h | SR-1 |
| SR-3 | **String implementation** — SSO at ≤15 bytes, heap allocation, all string operations | High | 20h | SR-1 |
| SR-4 | **List implementation** — dynamic array, push/pop/get/set, iteration | High | 15h | SR-1 |
| SR-5 | **Map implementation** — open-addressing hash table, get/set/delete | High | 20h | SR-1 |
| SR-6 | **Closure representation** — captured environment, invoke mechanism | High | 10h | SR-1 |
| SR-7 | **Cycle detector in Coral** — Bacon's synchronous cycle collection algorithm | Medium | 15h | SR-2 |
| SR-8 | **Actor scheduler in Coral** — M:N scheduling, mailboxes, work queues | High | 25h | SR-1, SR-6 |
| SR-9 | **Store engine in Coral** — WAL, binary/JSON storage, B+ tree indexes | Medium | 30h | SR-5 |
| SR-10 | **FFI layer** — C function declarations, syscall wrappers, atomics | Critical | 15h | SR-1 |
| SR-11 | **Runtime integration tests** — verify Coral runtime matches Rust runtime behavior | High | 15h | SR-1..SR-10 |
| SR-12 | **Memory allocator (optional)** — custom allocator via `mmap`/`brk` for libc independence | Low | 20h | SR-10 |

**Subtotal: ~220 hours**

---

## Optimization Tasks (Native Performance)

These can be worked on in parallel once the compiler pipeline is stable.

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| OP-1 | **Escape analysis** — identify values that can be stack-allocated | High | 15h | TS-1 |
| OP-2 | **Numeric unboxing** — keep f64/i64 in registers for pure-arithmetic paths | High | 15h | TS-1 |
| OP-3 | **Dead code elimination** — remove unreachable code paths | Medium | 8h | — |
| OP-4 | **Basic function inlining** — inline small functions at call sites | Medium | 10h | — |
| OP-5 | **LLVM optimization pass wiring** — expose `-O1`/`-O2`/`-Os` flags | Medium | 5h | — |
| OP-6 | **Performance benchmarks** — establish baseline and track improvements | High | 5h | — |

**Subtotal: ~58 hours**

---

## Infrastructure & Code Quality

| ID | Task | Priority | Est. | Depends On |
|----|------|----------|------|------------|
| IQ-1 | **AST-level module system (ML1, ML2)** — replace text-based `use` expansion with proper AST imports, namespacing, selective imports | High | 20h | — |
| IQ-2 | **Split `codegen/mod.rs`** ✅ (~5,047 → 2,009 + 1,245 + 406 + 1,313 lines) | Medium | 10h | — |
| IQ-3 | **Split `runtime/src/lib.rs`** ✅ (~5,873 → 2,552 + 8 submodules) | Medium | 10h | — |
| IQ-4 | **Fix all examples** — `fizzbuzz.coral`, `chat_server.coral`, `http_server.coral`, `data_pipeline.coral` | Medium | 8h | TS-10, PS-3 |
| IQ-5 | **Expand test coverage** ✅ — 740+ tests (target was 500+) | Medium | 15h | — |
| IQ-6 | **Fuzzing** — at least lexer/parser fuzz testing | Low | 8h | — |
| IQ-7 | **Update documentation** — review pass across all docs | Low | 5h | — |

**Subtotal: ~76 hours**

---

## Dependency Graph

```
Stream 1 (Type System)
  │
  ├──→ Stream 2 (Stdlib) ──→ Stream 5 (Self-Hosted Compiler) ──→ Stream 6 (Self-Hosted Runtime)
  │         │
  │         └──→ Stream 3 (Actors)
  │
  └──→ Stream 4 (Stores)
  │
  └──→ Optimization Tasks

Infrastructure (IQ-*) can proceed in parallel with everything.
```

**Critical path:** TS-1 (generics) → SC-5 (semantic in Coral) → SC-7 (LLVM IR emission) → SC-9 (bootstrap) → SR-* (runtime)

---

## Priority Ordering

### Phase A — Foundation (do first)
1. **TS-1** Generic instantiation — everything else depends on a working type system
2. **TS-10** Fix wildcard match pattern codegen — 1 failing test
3. **TS-2** Remove dual-track TypeEnv
4. **TS-3** None/Unit distinction
5. **PS-1** Store persistence codegen
6. **SL-1, SL-2, SL-3** Wire up math/io/process FFI

### Phase B — Completeness (parallel tracks)
- **Stdlib:** SL-4 through SL-16
- **Actors:** AC-1 through AC-5
- **Stores:** PS-2 through PS-8
- **Type system:** TS-4 through TS-9
- **Infrastructure:** IQ-1 through IQ-5
- **Self-hosted compiler:** SC-1 through SC-8 ✅

### Phase C — Self-Hosting ✅ (code complete, not execution-verified)
- **SC-3** ✅ Module loader in Coral
- **SC-5** ✅ Semantic analysis in Coral
- **SC-6** ✅ Lowering in Coral
- **SC-7** ✅ LLVM IR text emission
- **SC-8** ✅ Compiler integration
- **SC-9** Bootstrap test — **NOT STARTED** (blocked on SH-1..SH-4 bug fixes)
- **Optimization:** OP-1 through OP-6

### Phase D — Bootstrap & Runtime
- **SH-1..SH-4** Fix cross-module data format bugs in self-hosted compiler
- **SC-9** Bootstrap test — self-hosted compiles itself, output matches Rust compiler
- **SC-10** Performance comparison — self-hosted vs Rust compiler speed
- **SR-1 through SR-12** Self-hosted runtime
- **IQ-6, IQ-7** Fuzzing, documentation
- **AC-6** Remote actors

---

## Estimated Totals

| Stream | Hours |
|--------|-------|
| Type System & Semantics | 81 |
| Standard Library | 115 |
| Actor System | 66 |
| Persistent Stores | 71 |
| Self-Hosted Compiler | 218 |
| Self-Hosted Runtime | 220 |
| Optimization | 58 |
| Infrastructure & Quality | 76 |
| **Total** | **~905 hours** |

---

## Remaining Known Bugs

| ID | Severity | Description | Stream |
|----|----------|-------------|--------|
| P6 | Medium | Single-error model (subsequent errors dropped) | TS-6 |
| S6 | Medium | Member access falls back to Map constraint | TS-4 |
| S8 | Medium | Pipeline type inference discards left type | TS-5 |
| R11 | Medium | Single work queue contention | AC-5 |
| ML1 | Medium | Text-based export extraction | IQ-1 |
| ML2 | Medium | No proper namespacing | IQ-1 |

### Resolved Bugs (Complete History)

All 7 critical bugs resolved. All high-severity bugs resolved. 18+ medium bugs resolved. See `REMEDIATION_TRACKER.md` for the full session-by-session history of bug fixes and changes.

---

## Language Design Constraints

These are non-negotiable design decisions that all work must respect:

1. **Pure type inference** — no type annotations anywhere in user code; all types inferred via constraint solving
2. **`is` for binding** — no `=` or `==` operators; `is` is the binding operator
3. **Method-based equality** — `.equals()` / `.not_equals()` instead of `==` / `!=`
4. **Single `Number(f64)` at runtime** — runtime uses one numeric type; integer distinction is compile-time only
5. **Value-error model** — every value can carry error/absence metadata; no exceptions, no Result wrappers at runtime level
6. **Indentation-based syntax** — Python-style blocks via INDENT/DEDENT tokens
7. **`*` marks functions** — function definitions start with `*`
8. **`?`/`!` for ternary** — `condition ? then_branch ! else_branch`
9. **`~` for pipeline** — `value ~ fn1 ~ fn2` desugars to `fn2(fn1(value))`
10. **Actors are the concurrency primitive** — no shared mutable state between actors; message passing only

---

## Reference Specifications

| Document | Purpose |
|----------|---------|
| [STANDARD_LIBRARY_SPEC.md](STANDARD_LIBRARY_SPEC.md) | Complete stdlib API specification |
| [PERSISTENT_STORE_SPEC.md](PERSISTENT_STORE_SPEC.md) | Store system: WAL, indexing, ACID, queries |
| [ACTOR_SYSTEM_COMPLETION.md](ACTOR_SYSTEM_COMPLETION.md) | Actor typed messages, supervision, remote |
| [SELF_HOSTED_COMPILER_SPEC.md](SELF_HOSTED_COMPILER_SPEC.md) | Self-hosting architecture and bootstrap plan |
| [SELF_HOSTED_RUNTIME_SPEC.md](SELF_HOSTED_RUNTIME_SPEC.md) | Runtime reimplementation in Coral |
| [VALUE_ERROR_MODEL.md](VALUE_ERROR_MODEL.md) | Error handling design: flags, propagation, syntax |
| [COMPILATION_TARGETS.md](COMPILATION_TARGETS.md) | Target architectures and WASM |
| [CYCLE_SAFE_PATTERNS.md](CYCLE_SAFE_PATTERNS.md) | Memory safety patterns for RC runtime |
| [LIBC_INDEPENDENCE.md](LIBC_INDEPENDENCE.md) | Syscall-direct runtime (post-alpha goal) |
| [SELF_HOSTING_STATUS.md](SELF_HOSTING_STATUS.md) | Current self-hosted compiler progress |
| [STDLIB_STATUS.md](STDLIB_STATUS.md) | Current stdlib module completion status |
