# Coral Language — Remediation Plan

**Date:** February 16, 2026  
**Companion to:** COMPREHENSIVE_REVIEW_REPORT.md  
**Goal:** Resolve all outstanding issues, complete the language to alpha, and lay the foundation for self-hosted compiler, Coral-based runtime, actor framework, and built-in persistence.

---

## Language Design Decisions (Binding Constraints)

> These decisions were established during remediation and override any conflicting tasks below.

1. **Pure Type Inference** — No type annotations anywhere in Coral syntax. No `->` return type syntax. No parameter type annotations. The `TypeAnnotation` AST node exists internally but is always `None` in parsed code. All type checking works via inference and constraint solving.

2. **Method-Based Equality** — No `==` or `!=` operators. Equality is expressed via method calls:
   - `var.equals(otherVar)` — returns boolean
   - `var.not_equals(otherVar)` or `not var.equals(otherVar)` — returns boolean
   - Runtime already provides `coral_value_equals()`. Need to add `coral_value_not_equals()` and wire both as built-in methods.

3. **`is` for Binding** — Variable binding uses `is` keyword, not `=`. The `=` token may be needed only for store field assignment; audit required (task 6.8).

---

## Plan Overview

**Total estimated effort: ~640 hours across 24 weeks (6 months)**  
**Organized into 8 phases, 5 tracks running partially in parallel**

### Tracks
- **T1 — Correctness:** Fix critical/high bugs to stabilize what exists
- **T2 — Language Completeness:** Add missing language features (loops, if/else, return, etc.)
- **T3 — Runtime Hardening:** Fix concurrency bugs, complete store FFI, improve performance
- **T4 — Self-Hosting Foundation:** Build features needed for compiler self-compilation
- **T5 — Polish & Testing:** End-to-end tests, documentation, code quality

### Phase Summary

| Phase | Weeks | Focus | Hours |
|-------|-------|-------|-------|
| 1 | 1-3 | **Critical Bug Fixes** — Stop the bleeding | 60h |
| 2 | 4-6 | **Core Language Gaps** — Loops, if/else, return | 80h |
| 3 | 7-9 | **Runtime Hardening** — Store FFI, concurrency, shutdown | 80h |
| 4 | 10-12 | **Type System & Semantics** — Real ADT types, generics, namespacing | 80h |
| 5 | 13-15 | **Actor System Completion** — Typed messages, networking, monitoring | 60h |
| 6 | 16-18 | **MIR Rewrite & Optimization** — Single codegen path | 80h |
| 7 | 19-21 | **Self-Hosting Preparation** — Bootstrap subset, std completeness | 100h |
| 8 | 22-24 | **Self-Hosted Compiler & Runtime** — Begin Coral-in-Coral | 100h |

---

## Phase 1: Critical Bug Fixes (Weeks 1-3, 60h)

**Goal:** Fix all 7 critical bugs and the most impactful high-severity issues. Establish end-to-end execution tests.

### Week 1: Codegen Crash Fixes (20h)

| Task | Bug ID | Description | Hours |
|------|--------|-------------|-------|
| 1.1 | C1 | Fix `Statement::Return` — return Value* directly, not f64-cast-to-pointer | 3h |
| 1.2 | C2 | Fix actor handler dispatch — pass Value* data, not f64 conversion | 4h |
| 1.3 | C3 | Fix hash-based actor dispatch — use same hash function as runtime, or always use sequential dispatch | 4h |
| 1.4 | C4 | Fix `make_map` zero-arg call — pass empty entries pointer and count=0 | 2h |
| 1.5 | C5 | Fix `value_hash` return type in runtime.rs declaration — change to i64 and use `into_int_value()` | 2h |
| 1.6 | — | Create first end-to-end execution test: compile Coral program → link with runtime → execute → verify output | 5h |

**Deliverable:** All compiled Coral programs that use return/actors/maps no longer crash.

### Week 2: Runtime Critical Fixes (20h)

| Task | Bug ID | Description | Hours |
|------|--------|-------------|-------|
| 2.1 | R1 | Fix cycle detector deadlock — collect values to free in buffer, release after dropping mutex | 4h |
| 2.2 | R2 | Fix symbol interning — correctly extract string data from Value (handle inline vs heap) | 3h |
| 2.3 | R3 | Fix store FFI TAG constants — align with lib.rs ValueTag (Number=0, Bool=1, String=2) | 2h |
| 2.4 | R5 | Make `retain_events`/`release_events` AtomicU32, or guard with `#[cfg(debug_assertions)]` | 2h |
| 2.5 | R4 | Implement `handle_to_stored_value` — convert runtime Value to StoredValue by reading tag+payload | 6h |
| 2.6 | R7 | Fix UUID7 atomicity — use compare-and-swap loop for timestamp+counter update | 3h |

**Deliverable:** Stores can actually persist data. No deadlocks. No data races.

### Week 3: Test Infrastructure (20h)

| Task | Description | Hours |
|------|-------------|-------|
| 3.1 | Create `tests/execution/` directory with end-to-end test harness: compile → link → run → assert stdout | 8h |
| 3.2 | Write 15 execution tests: hello world, arithmetic, strings, lists, maps, closures, match, ADTs, error values, pipeline, actors (spawn+send), stores (create+get), nested calls, recursion, globals | 8h |
| 3.3 | Fix the 1 failing test (parser_fixtures unexpected_dedent.expect message) | 1h |
| 3.4 | Delete orphaned runtime files (value.rs, memory.rs, collections/, map_hash.rs) or integrate them | 3h |

**Deliverable:** 264+ tests, all passing. First runtime-execution tests in the suite. Orphaned code resolved.

---

## Phase 2: Core Language Gaps (Weeks 4-6, 80h)

**Goal:** Implement the missing language constructs that block any non-trivial program.

### Week 4: Loop Constructs (28h)

| Task | Description | Hours |
|------|-------------|-------|
| 4.1 | Add `while`, `for`, `break`, `continue` keywords to lexer | 2h |
| 4.2 | Add `Statement::While(condition, body, span)` and `Statement::For(binding, iterable, body, span)` to AST | 2h |
| 4.3 | Implement `parse_while_statement()` — `while condition` + indented body block | 4h |
| 4.4 | Implement `parse_for_statement()` — `for name in expr` + indented body block | 4h |
| 4.5 | Add `Break`/`Continue` statement variants and parsing | 2h |
| 4.6 | Semantic analysis for loops — scope checking, type constraints for iterable | 4h |
| 4.7 | LLVM codegen for while — create loop header/body/exit basic blocks, emit condition branch | 5h |
| 4.8 | LLVM codegen for for — emit iterator creation, loop with iter_next, null-check exit | 5h |

**Deliverable:** `while` and `for` loops compile and execute. fizzbuzz.coral is viable.

### Week 5: If/Else & Return (26h)

| Task | Description | Hours |
|------|-------------|-------|
| 5.1 | Add `if`, `elif`, `else` keywords to lexer (if not already present) | 1h |
| 5.2 | Add `Statement::If(condition, then_block, elif_clauses, else_block, span)` to AST | 2h |
| 5.3 | Implement `parse_if_statement()` — handles if/elif chains and optional else | 6h |
| 5.4 | Semantic analysis for if/else — scope checking, branch type compatibility | 3h |
| 5.5 | LLVM codegen for if/else — basic block chain with phi node for value-producing if-expression | 6h |
| 5.6 | Implement `parse_return_statement()` — connect the already-lexed `return` keyword | 2h |
| 5.7 | Fix `Statement::Return` codegen — return Value* directly (verify Phase 1 fix) | 2h |
| 5.8 | Support return in lambdas (currently rejected) | 4h |

**Deliverable:** Block-level conditionals and return statements work. Ternary is no longer the only conditional mechanism.

### Week 6: Essential Operators & Syntax (21h)

> **Design Decisions Applied:**
> - ~~6.1 (`!=` operator)~~ — **REMOVED.** Equality is via `.equals()` / `.not_equals()` method calls, not operators.
> - ~~6.2 (`->` arrow token)~~ — **REMOVED.** No type annotations in syntax; Coral uses pure type inference throughout.
> - 6.8 updated — `==` is no longer needed as an operator; only `=` for store field assignment needs audit.

| Task | Description | Hours |
|------|-------------|-------|
| ~~6.1~~ | ~~Add `!=` (NotEqual) token~~ | ~~3h~~ | **REMOVED — equality via `.equals()`/`.not_equals()` methods** |
| ~~6.2~~ | ~~Add `->` (Arrow) token for return type annotations~~ | ~~2h~~ | **REMOVED — pure type inference, no annotations** |
| 6.3 | Implement index/subscript syntax `expr[index]` in parser and codegen | 6h |
| 6.4 | Add hex (`0xFF`), binary (`0b1010`), octal (`0o77`) numeric literals to lexer | 4h |
| 6.5 | Add underscore separators in numbers (`1_000_000`) | 2h |
| 6.6 | Fix float-dot ambiguity — lookahead: if char after `.` is a digit, it's a float; otherwise consume number only | 3h |
| 6.7 | Reject unknown string escape sequences (error on `\q`) | 2h |
| 6.8 | Audit `=`/`==` tokens — `==` is no longer needed (equality via `.equals()` method). Determine if `=` is needed for store field assignment or can be fully replaced by `is`. Remove `==` token production from lexer. | 4h |

**Deliverable:** Essential operators work. Numeric literal support suitable for systems programming.

---

## Phase 3: Runtime Hardening (Weeks 7-9, 80h)

**Goal:** Make the runtime production-quality for single-machine workloads.

### Week 7: Store FFI Completion (28h)

| Task | Description | Hours |
|------|-------------|-------|
| 7.1 | Implement full `handle_to_stored_value` — read Value tag, extract payload, recursively convert nested lists/maps | 8h |
| 7.2 | Implement `stored_value_to_handle` — reverse conversion, creating runtime Values from store records | 6h |
| 7.3 | Fix `coral_store_get_by_uuid` — use index's uuid_to_index HashMap for O(1) lookup | 2h |
| 7.4 | Fix `with_engine` — maintain handle→engine direct mapping instead of string lookup + config reconstruction | 4h |
| 7.5 | Store end-to-end test: create → get → update → delete → query through Coral code | 6h |
| 7.6 | WAL recovery test: write data → simulate crash → recover → verify data integrity | 2h |

**Deliverable:** Stores can persist and retrieve real data end-to-end.

### Week 8: Concurrency Fixes (26h)

| Task | Description | Hours |
|------|-------------|-------|
| 8.1 | Replace single work queue with per-worker channels or work-stealing deque (crossbeam-deque) | 8h |
| 8.2 | Fix `spawn_named` — check+register name atomically BEFORE spawning actor | 3h |
| 8.3 | Add runtime shutdown mechanism — poison pill for workers, timer thread signal, store flush | 6h |
| 8.4 | Replace global value pool Mutex with thread-local pools + periodic global drain | 5h |
| 8.5 | Change refcount ordering from SeqCst to Acquire (retain) / AcqRel (release) | 2h |
| 8.6 | Make `drop_heap_value` iterative (explicit worklist) to prevent stack overflow | 2h |

**Deliverable:** Actor system is production-grade for local workloads. Clean shutdown. No contention bottlenecks.

### Week 9: Memory Management Hardening (26h)

| Task | Description | Hours |
|------|-------------|-------|
| 9.1 | Fix cycle detector reentrance — collect-then-release pattern with deferred free list | 4h |
| 9.2 | Fix `scan()` — validate value liveness before accessing children (use weak pointers or epoch tracking) | 6h |
| 9.3 | Add ErrorMetadata free path in `drop_heap_value` — currently leaks the Box'd metadata | 3h |
| 9.4 | MapObject proper resize — implement load factor threshold (0.75) with power-of-2 growth | 5h |
| 9.5 | Add memory leak detection test: create cyclic structures, trigger collection, verify all freed | 4h |
| 9.6 | Add 48-hour stress test: continuous alloc/free/cycle-create/actor-spawn with memory monitoring | 4h |

**Deliverable:** No memory leaks. No crashes under sustained load. Cycle collection is correct and deadlock-free.

---

## Phase 4: Type System & Semantics (Weeks 10-12, 80h)

**Goal:** Make the type system actually enforce correctness.

> **DESIGN CONSTRAINT:** Coral uses **pure type inference** throughout — NO type annotations, NO return type signatures, NO `->` syntax.
> All type checking in this phase must work via inference, constraint solving, and flow analysis.
> The existing `TypeAnnotation` AST node is internal-only and always `None` in parsed code.
> Equality checking uses `.equals()` / `.not_equals()` method calls, not operators.

### Week 10: ADT Types in Type System (28h)

| Task | Description | Hours |
|------|-------------|-------|
| 10.1 | Add `TypeId::Adt(name, type_args)` variant to type system | 3h |
| 10.2 | Register type definitions with proper type IDs during semantic first pass (fix S1) | 4h |
| 10.3 | Namespace variant constructors under their parent type (fix S3) — `Option.Some`, `Option.None` resolves correctly | 6h |
| 10.4 | Type-check constructor calls — verify field count and infer types from variant definition (no annotations) | 4h |
| 10.5 | Constrain match scrutinee type from constructor patterns (fix S7) | 4h |
| 10.6 | Implement exhaustiveness checking for ADTs with >1 level of nesting | 4h |
| 10.7 | Add `MatchPattern::List` to use `Vec<MatchPattern>` instead of `Vec<Expression>` (fix A3) | 3h |

**Deliverable:** ADTs are properly typed. Pattern matching is type-safe. Constructor namespaces prevent collisions.

### Week 11: Generic Type Instantiation (28h)

| Task | Description | Hours |
|------|-------------|-------|
| 11.1 | Implement proper type parameter substitution in `instantiate_generic` | 6h |
| 11.2 | Specialize `List[T]` → `List[Int]`, `List[String]`, etc. during constraint solving | 6h |
| 11.3 | Specialize `Map[K,V]` similarly | 4h |
| 11.4 | Implement proper `Option[T]` and `Result[T,E]` as ADT types (not hardcoded to List/Any) | 6h |
| 11.5 | Remove dual-track `symbols` map from TypeEnv — use only scoped bindings (fix T4) | 4h |
| 11.6 | Fix `instantiate_generic` type param leaking (fix T5) | 2h |

**Deliverable:** Generics work. `List[Int]` catches `push(string)` errors at compile time.

### Week 12: Semantic Analysis Completion (24h)

| Task | Description | Hours |
|------|-------------|-------|
| 12.1 | Scope-check store/type method bodies (fix S4) | 4h |
| 12.2 | Distinguish `None` from `Unit` — add `Primitive::None`/`Absent` (fix S5) | 3h |
| 12.3 | Fix member access type inference for stores/types (fix S6) | 4h |
| 12.4 | Fix pipeline type inference — verify left type is compatible with first param (fix S8) | 3h |
| 12.5 | Allow shadowing in inner scopes (fix S9) — or document prohibition | 2h |
| 12.6 | Replace hardcoded `is_builtin_name` with registry derived from extern declarations (fix S10) | 4h |
| 12.7 | Add severity levels to diagnostics (warning/info/error) (fix D2) | 2h |
| 12.8 | Add file ID to Span for multi-file diagnostics (fix D1) | 2h |

**Deliverable:** Semantic analysis catches real errors in methods, stores, and pipelines. Multi-file diagnostics work.

---

## Phase 5: Actor System Completion (Weeks 13-15, 60h)

**Goal:** Complete the actor framework per the ACTOR_SYSTEM_COMPLETION spec.

### Week 13: Typed Messages & Monitoring (20h)

| Task | Description | Hours |
|------|-------------|-------|
| 13.1 | Implement `@messages(MessageType)` annotation in parser | 4h |
| 13.2 | Compile-time type checking at `send()` call sites — verify message type matches handler | 6h |
| 13.3 | Implement `monitor(actor)` / `demonitor(actor)` runtime functions | 4h |
| 13.4 | Implement `ActorDown` message delivery when monitored actor dies | 4h |
| 13.5 | Add tests for typed messages and monitoring | 2h |

### Week 14: Supervision Hardening (20h)

| Task | Description | Hours |
|------|-------------|-------|
| 14.1 | Implement full supervision strategy execution (restart budget enforcement, time window) | 6h |
| 14.2 | Implement escalation chain — child failure propagates to parent if budget exhausted | 4h |
| 14.3 | Implement graceful actor stop — flush mailbox before termination | 4h |
| 14.4 | Add supervision tree integration tests — multi-level parent/child with restart scenarios | 4h |
| 14.5 | Document supervision API and strategies | 2h |

### Week 15: Remote Actors (Foundation) (20h)

| Task | Description | Hours |
|------|-------------|-------|
| 15.1 | Implement TCP transport layer with connection pooling | 8h |
| 15.2 | Implement "CACT" protocol serialization — header/target/payload framing | 4h |
| 15.3 | Implement remote actor proxy — `send()` serializes and transmits to remote node | 4h |
| 15.4 | Location-transparent actor lookup — registry checks local then remote | 2h |
| 15.5 | Remote actor integration tests | 2h |

**Deliverable:** Full actor framework — typed messages, supervision, monitoring, basic networking.

---

## Phase 6: MIR Rewrite & Optimization (Weeks 16-18, 80h)

**Goal:** Replace the vestigial MIR with a proper IR that all codegen routes through.

### Week 16: MIR Data Structures (28h)

| Task | Description | Hours |
|------|-------------|-------|
| 16.1 | Design new MIR with SSA, phi nodes, types on all values | 8h |
| 16.2 | Add all missing instruction types: comparisons, bitwise, unary, member access, construct, match, call (direct/indirect/method), error handling, pipeline | 10h |
| 16.3 | Add control flow: if/else, while, for, pattern match, break/continue | 6h |
| 16.4 | Add closure representation, actor message send, store operations | 4h |

### Week 17: AST → MIR Lowering (28h)

| Task | Description | Hours |
|------|-------------|-------|
| 17.1 | Implement complete AST → MIR lowering for all expression types | 12h |
| 17.2 | Implement statement lowering (bindings, assignments, returns, loops, if/else) | 8h |
| 17.3 | Implement item lowering (functions, stores, actors, types, error defs) | 4h |
| 17.4 | Verify MIR→LLVM produces identical output as current AST→LLVM for all 249 tests | 4h |

### Week 18: MIR Optimizations (24h)

| Task | Description | Hours |
|------|-------------|-------|
| 18.1 | Implement constant folding pass on MIR (replaces current AST-level folder) | 4h |
| 18.2 | Implement dead code elimination | 4h |
| 18.3 | Implement basic inlining for small functions | 6h |
| 18.4 | Implement escape analysis — identify values that can be stack-allocated | 6h |
| 18.5 | Implement numeric unboxing — keep f64/i64 in registers for pure-arithmetic paths | 4h |

**Deliverable:** Single codegen path through well-typed MIR. Initial optimization passes. Measurable performance improvement.

---

## Phase 7: Self-Hosting Preparation (Weeks 19-21, 100h)

**Goal:** Complete all prerequisites for writing the compiler in Coral.

### Week 19: Standard Library Completion (35h)

| Task | Description | Hours |
|------|-------------|-------|
| 19.1 | Expose all runtime trig/log/math functions in std/math.coral | 2h |
| 19.2 | Expose all error handling functions (make_error, is_err, is_absent, error_name, etc.) as std/error.coral | 4h |
| 19.3 | Implement std/json.coral with parse/serialize (using runtime string operations) | 8h |
| 19.4 | Implement std/time.coral wrapping libc time functions | 4h |
| 19.5 | Fix std/map.coral — replace `.equals(none).not()` with proper `.has()` using `is_absent` check | 2h |
| 19.6 | Fix std/set.coral — same `.has()` fix, proper empty map construction | 2h |
| 19.7 | Fix std/result.coral — replace `for` loops with `.map()`/`.reduce()` or implement `for` (done in Phase 2) | 3h |
| 19.8 | Expose store FFI as std/store.coral | 4h |
| 19.9 | Create std/encoding.coral (base64, hex, utf8) | 4h |
| 19.10 | Add all missing runtime function wrappers to appropriate std modules | 2h |

### Week 20: Self-Hosting Language Features (35h)

| Task | Description | Hours |
|------|-------------|-------|
| 20.1 | Implement proper `import`/`use` with namespaces — `use std.io` → access as `io.read()` | 10h |
| 20.2 | Implement trait/mixin system — `trait Printable` with `method` declarations and `with` syntax | 12h |
| 20.3 | Implement process spawning (`extern fn` wrapping `fork`/`exec`/`waitpid`) for build orchestration | 4h |
| 20.4 | Implement environment variable access (`extern fn` wrapping `getenv`/`setenv`) | 2h |
| 20.5 | Implement comprehensive string manipulation (regex matching via POSIX regex or simple patterns) | 5h |
| 20.6 | File I/O completion — append, delete, mkdir, list_dir, file metadata | 2h |

### Week 21: Compiler Bootstrap Subset Definition (30h)

| Task | Description | Hours |
|------|-------------|-------|
| 21.1 | Define "Coral-0" — the exact subset of Coral sufficient to write the lexer (types, match, strings, lists, ternary, functions, error handling) | 4h |
| 21.2 | Write the Coral lexer in Coral-0 — port lexer.rs to lexer.coral | 12h |
| 21.3 | Verify Coral lexer produces identical tokens as Rust lexer for all test fixtures | 6h |
| 21.4 | Document all self-hosting prerequisites and their completion status | 4h |
| 21.5 | Create bootstrap test: Coral lexer → compiled → runs on syntax.coral → matches Rust lexer output | 4h |

**Deliverable:** Complete standard library. All self-hosting prerequisites met. Coral lexer written in Coral and verified.

---

## Phase 8: Self-Hosted Compiler & Runtime (Weeks 22-24, 100h)

**Goal:** Begin the self-hosting journey — compiler and runtime written in Coral.

### Week 22: Parser in Coral (35h)

| Task | Description | Hours |
|------|-------------|-------|
| 22.1 | Define AST data structures as Coral ADTs (`enum Expr`, `enum Stmt`, `enum Item`, etc.) | 6h |
| 22.2 | Port parser.rs → parser.coral — recursive-descent with indentation tracking | 16h |
| 22.3 | Verify Coral parser produces identical AST as Rust parser for all test fixtures | 8h |
| 22.4 | Performance comparison: Coral parser vs Rust parser | 2h |
| 22.5 | Create combined bootstrap test: Coral lexer → Coral parser → AST verification | 3h |

### Week 23: Semantic Analysis in Coral (35h)

| Task | Description | Hours |
|------|-------------|-------|
| 23.1 | Port type system (core.rs, env.rs, solver.rs) → types.coral | 12h |
| 23.2 | Port semantic.rs → semantic.coral | 14h |
| 23.3 | Port lower.rs → lower.coral | 4h |
| 23.4 | Verify identical semantic output | 5h |

### Week 24: Codegen & Integration (30h)

| Task | Description | Hours |
|------|-------------|-------|
| 24.1 | Begin MIR codegen port (target: emit LLVM IR as text strings, not via inkwell) | 12h |
| 24.2 | Text-based LLVM IR emission for core expression types | 10h |
| 24.3 | Integration test: Coral compiler compiles hello.coral to identical LLVM IR as Rust compiler | 4h |
| 24.4 | Runtime bootstrap planning: identify which runtime modules can be rewritten first | 4h |

**Deliverable:** Coral compiler front-end (lexer + parser + semantic) self-hosted. Codegen partially ported.

---

## Dependency Graph

```
Phase 1 (Critical Fixes) ─┬─→ Phase 2 (Language Gaps) ─────┬─→ Phase 4 (Type System) ──→ Phase 6 (MIR Rewrite)
                           │                                 │                                      │
                           └─→ Phase 3 (Runtime Hardening) ──┘                                      │
                                                                                                     ↓
                           Phase 5 (Actor Completion) ──────────→ Phase 7 (Self-Host Prep) ──→ Phase 8 (Self-Hosted)
```

- **Phase 1** has no dependencies — start immediately
- **Phase 2** depends on Phase 1 (codegen fixes needed before adding features)
- **Phase 3** depends on Phase 1 (runtime fixes from week 2)
- **Phase 4** depends on Phase 2 (new AST variants need type checking)
- **Phase 5** depends on Phase 3 (runtime concurrency fixes)
- **Phase 6** depends on Phase 4 (MIR needs types) and Phase 2 (MIR needs all statement types)
- **Phase 7** depends on Phase 2 + Phase 4 (complete language features + types)
- **Phase 8** depends on Phase 7 (all prerequisites)

---

## Success Criteria

### Alpha Release (end of Phase 4, Week 12)
- [x] All 7 critical bugs fixed
- [ ] All 15 high-severity bugs fixed (6 remaining: P4, S2, S3, T1, T2, T3)
- [ ] `for`/`while` loops, `if`/`else` blocks, `return` statements work
- [ ] 350+ tests, all passing
- [ ] 20+ end-to-end execution tests
- [ ] ADTs properly typed with namespace isolation
- [ ] Generics instantiated correctly
- [ ] Stores can persist and retrieve real data
- [ ] All examples that don't require networking compile and run
- [ ] Zero memory leaks in 4-hour stress test

### Beta Release (end of Phase 6, Week 18)
- [ ] Full actor framework (typed messages, supervision, monitoring)
- [ ] Proper MIR as single codegen path
- [ ] Basic optimization passes (constant folding, dead code, inlining)
- [ ] Measurable performance improvement from MIR optimizations
- [ ] 500+ tests
- [ ] Standard library >80% complete vs spec

### Self-Hosting Milestone (end of Phase 8, Week 24)
- [ ] Coral compiler front-end (lexer, parser, semantic) self-hosted
- [ ] Self-hosted front-end produces identical output as Rust front-end
- [ ] Compilation within 5x of Rust compiler speed
- [ ] All self-hosting language prerequisites documented and verified
- [ ] Coral lexer in Coral is the reference lexer going forward

---

## Risk Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| MIR rewrite takes longer than estimated | High | Phase 6 delays | Start with MIR that only handles new features; gradually migrate old paths |
| Self-hosted parser too complex for current type system | Medium | Phase 8 delays | Define smallest viable subset; use `Any` types where inference is insufficient |
| Actor networking introduces security issues | Medium | Low | Ship local-only first; networking is optional |
| LLVM 16 becomes unsupported | Low | High | Pin to known-good LLVM version; inkwell abstraction helps migration |
| Performance regression from boxed-everything model | High | Medium | Phase 6 escape analysis and unboxing are explicitly scheduled |

---

## Appendix A: Complete Bug Fix Checklist

### Critical (7) — ALL RESOLVED ✅
- [x] C1: Statement::Return f64/pointer confusion
- [x] C2: Actor handler dispatch f64/Value* confusion
- [x] C3: Hash-based actor dispatch hash function mismatch
- [x] C4: make_map zero-arg to two-arg function
- [x] R1: Cycle detector deadlock (collect_white/possible_root)
- [x] R2: Symbol interning memory layout crash
- [x] R3: Store FFI TAG constants off by one

### High (15) — 6 REMAINING
- [x] P1: No for/while/loop constructs
- [x] P2: No if/elif/else blocks
- [x] P3: return keyword lexed but never parsed
- [ ] P4: self.field desugaring hack
- [x] S1: Forward reference failures for type items
- [ ] S2: Variant constructors typed as Any
- [ ] S3: Constructor name collisions (no namespace)
- [x] S4: Store/type method bodies never scope-checked
- [ ] T1: Int/Float silently unify
- [ ] T2: Generic instantiation faked (Option→List, Result→Any)
- [ ] T3: No ADT types in type system
- [x] R4: Store handle_to_stored_value stub
- [x] R5: Non-atomic retain/release event counters
- [x] R6: spawn_named race condition
- [x] C5: value_hash return type mismatch (consumer removed)

### Medium (20+)
- [ ] L3: No hex/octal/binary numeric literals
- [ ] L4: Unknown escape sequences silently accepted
- [ ] P5: synchronize() dead code
- [ ] P6: Single-error model (subsequent errors dropped)
- [ ] P7: peek_kind() clones String/Vec payloads
- [ ] P8: No index/subscript syntax
- [ ] P10: ~~Trait method return types discarded~~ → **Redesigned:** Remove `-> Type` parsing from `parse_trait_method()` — no return type annotations per design decision
- [ ] A1: PersistenceMode dead code
- [ ] A3: MatchPattern::List uses Vec<Expression>
- [ ] S5: None/Unit conflation
- [ ] S6: Member access falls back to Map constraint
- [ ] S8: Pipeline type inference discards left type
- [ ] T4: Dual-track TypeEnv (scopes vs symbols)
- [ ] T5: instantiate_generic leaks type params
- [ ] R8: scan() accesses potentially freed values
- [ ] R9: drop_heap_value recursive stack overflow
- [ ] R10: Worker/timer threads never exit
- [ ] R11: Single work queue contention
- [ ] D1: Span has no file ID
- [ ] D2: No diagnostic severity levels
- [ ] ML1: Text-based export extraction
- [ ] ML2: No proper namespacing

---

## Appendix B: File Decomposition Plan

### runtime/src/lib.rs (4844 lines → 6 files)
| New File | Content | Est. Lines |
|----------|---------|------------|
| value.rs | Value, ValueTag, Payload, alloc_value, drop_heap_value, retain, release | 800 |
| string.rs | StringObject, inline strings, all coral_string_* FFI | 600 |
| list.rs | ListObject, all coral_list_* FFI | 400 |
| map.rs | MapObject, MapBucket, all coral_map_* FFI | 500 |
| closure.rs | ClosureObject, coral_make_closure, coral_closure_invoke | 200 |
| lib.rs | Re-exports, remaining FFI (make_number, make_bool, etc.), tests | 800 |

### src/codegen/mod.rs (3830 lines → 5 files)
| New File | Content | Est. Lines |
|----------|---------|------------|
| expression.rs | emit_expression, emit_builtin_call | 1200 |
| statement.rs | emit_block, emit_statement, emit_binding | 400 |
| store_actor.rs | Store/actor construction, methods, handlers | 600 |
| match_adt.rs | Pattern matching, ADT construction, exhaustiveness | 400 |
| mod.rs | CodeGenerator struct, compile(), function signatures | 800 |

### src/semantic.rs (1908 lines → 4 files)
| New File | Content | Est. Lines |
|----------|---------|------------|
| scope.rs | ScopeStack, scope checking, duplicate detection | 400 |
| constraints.rs | Constraint generation, expression/statement traversal | 600 |
| mutability.rs | Mutability inference, usage tracking | 300 |
| semantic.rs | analyze() orchestrator, exhaustiveness, trait validation | 600 |

---

## Appendix C: Self-Hosting Prerequisites Checklist

| Prerequisite | Status | Needed For |
|-------------|--------|------------|
| Sum types (enum/ADT) | ✅ Implemented | Token, Expr, Stmt, Item, Type representation |
| Exhaustive pattern matching | ✅ Implemented | Token dispatch, AST traversal |
| String manipulation | ✅ Implemented | Source text processing, error messages |
| File I/O | ⚠️ Basic | Reading source files, writing output |
| Maps | ✅ Implemented | Symbol tables, scope maps |
| Lists | ✅ Implemented | Token streams, AST node children |
| Closures | ✅ Implemented | Higher-order functions, visitors |
| Error handling | ✅ Implemented | Parse errors, type errors |
| For/while loops | ✅ Implemented | Token iteration, tree traversal |
| If/else blocks | ✅ Implemented | Conditional logic throughout |
| Return statements | ✅ Implemented | Early returns in parse functions |
| Index/subscript access | ❌ **Missing** | Array access patterns |
| Namespaced imports | ❌ **Missing** | Module organization |
| Trait system | ❌ **Missing** | Visitor pattern, interface abstraction |
| Process spawning | ❌ **Missing** | Invoking LLVM tools |
| Environment variables | ❌ **Missing** | Build configuration |
| Hex numeric literals | ❌ **Missing** | Low-level constants |
| String regex/patterns | ❌ **Missing** | Lexer patterns (alternative: char-by-char) |
| Proper generics | ❌ **Missing** | Type-safe collections |
| Multi-line strings | ❌ **Missing** | LLVM IR template strings |
