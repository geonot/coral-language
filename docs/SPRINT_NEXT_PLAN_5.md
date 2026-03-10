# Sprint 5 Plan: Actor System Hardening, Error Types, HTTP, Optimization

**Created:** March 10, 2026  
**Baseline:** 310 compiler tests (1 pre-existing fail), 162 runtime tests (1 pre-existing fail)  
**Focus:** Make actor system production-grade, add error type tracking, enable real networking, improve optimization pipeline  

---

## Sprint Theme

Sprint 4 delivered type-aware dispatch, regex, type narrowing, nullability, actor restart/stop, do..end blocks, incremental compilation, and LTO. Sprint 5 builds on this by:

1. **Completing the actor system** — work-stealing scheduler, lock-free registry, typed messages, monitoring, cooperative yielding
2. **Advancing type safety** — error type tracking for exhaustive error handling
3. **Enabling real networking** — HTTP client/server to unblock chat_server.coral and http_server.coral
4. **Extending optimization** — profile-guided optimization building on LTO
5. **Improving reliability** — fuzz testing for lexer and parser

---

## Items (12)

### 1. R2.1 — Work-Stealing Scheduler (High)

**Goal:** Replace the single `mpsc::channel` work queue with per-worker deques and work stealing via `crossbeam-deque`. Eliminates central contention when actors are distributed across workers.

**Implementation Plan:**
- Add `crossbeam-deque` dependency to `runtime/Cargo.toml`
- Replace `WorkerPool`/`ActorSystem` work distribution with per-worker `Worker<T>` deques
- Each worker thread owns a local deque; idle workers steal from a random non-empty peer
- New `WorkStealingScheduler` struct in `runtime/src/actor.rs`:
  - `workers: Vec<Worker<ActorTask>>` — local deques
  - `stealers: Vec<Stealer<ActorTask>>` — handles for cross-worker stealing
  - `push(task)` → round-robin to workers, or push to least-loaded
  - Workers loop: pop local → steal from random peer → park thread
- Keep existing `mpsc` for external actor spawning (feeds tasks into scheduler)
- Preserve backward compat: actor API unchanged

**Files:** `runtime/Cargo.toml`, `runtime/src/actor.rs`, `runtime/src/actor_ops.rs`  
**Tests:** Work-stealing across N workers, fairness under load, actor message ordering preserved, starvation-free test  
**Estimate:** 4 tests

---

### 2. R2.2 — Lock-Free Actor Registry (Medium)

**Goal:** Replace `Mutex<HashMap>` for the named actor registry with a concurrent hash map. Named actor lookup becomes wait-free on the read path.

**Implementation Plan:**
- Add `dashmap` dependency to `runtime/Cargo.toml`
- Replace `named_actors: Mutex<HashMap<String, ActorRef>>` with `DashMap<String, ActorRef>`
- Update `register_named_actor()`, `lookup_named_actor()`, `unregister_named_actor()`
- Read path (lookup) becomes lock-free; write path uses dashmap's fine-grained sharding
- Update `coral_actor_register`, `coral_actor_lookup` FFI functions

**Files:** `runtime/Cargo.toml`, `runtime/src/actor.rs`, `runtime/src/actor_ops.rs`  
**Tests:** Concurrent register/lookup, unregister+lookup race, many-named-actor throughput  
**Estimate:** 3 tests

---

### 3. R2.7 — Typed Messages (High)

**Goal:** `@messages(MessageType)` annotation with compile-time type checking at `send()` call sites. Ensures message type safety.

**Implementation Plan:**
- **Parser:** Recognize `@messages(TypeName)` attribute on actor definitions
  - Add `message_type: Option<String>` field to actor AST node
  - Parse type name after `@messages(` ... `)`
- **Semantic:** When `send(actor, msg)` is called and actor has a known message type, verify `msg` is compatible
  - Store actor message type in `TypeEnv` as `ActorType(name, message_type_id)`
  - Emit warning/error on type mismatch at send() call sites
- **Codegen:** No runtime change needed — checking is at compile time
  - Optionally: emit debug assertion in debug builds

**Files:** `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`, `src/codegen/store_actor.rs`  
**Tests:** Typed send passes, mismatched type send error, actor without annotation accepts any, nested types  
**Estimate:** 4 tests

---

### 4. R2.8 — Actor Monitoring (High)

**Goal:** `monitor(actor)` / `demonitor(actor)` + `ActorDown` message delivery. Enables reactive failure handling without full supervision trees.

**Implementation Plan:**
- **Runtime:** Add `monitors: Vec<ActorRef>` to actor state
  - `coral_actor_monitor(watcher, target)` FFI — adds watcher to target's monitors list
  - `coral_actor_demonitor(watcher, target)` FFI — removes watcher
  - On actor termination (normal or crash), iterate monitors and send `ActorDown { actor, reason }` message
  - `ActorDown` as a special message type (or tagged union value)
- **Codegen:** Add `monitor`/`demonitor` as built-in functions, emit FFI calls
- **Semantic:** Register `monitor`/`demonitor` as built-in names

**Files:** `runtime/src/actor.rs`, `runtime/src/actor_ops.rs`, `src/codegen/builtins.rs`, `src/codegen/runtime.rs`, `src/semantic.rs`  
**Tests:** Monitor receives ActorDown on crash, demonitor stops notifications, multiple monitors, monitoring non-existent actor  
**Estimate:** 4 tests

---

### 5. T3.4 — Error Type Tracking (High)

**Goal:** Error values carry their taxonomy type: `err Database:Connection:Timeout` should have type `Error[Database.Connection.Timeout]`. Enables exhaustive error handling verification.

**Implementation Plan:**
- **Types:** Add `TypeId::Error(Vec<String>)` variant to `src/types/core.rs`
  - Segments from `err Foo:Bar:Baz` → `Error(["Foo", "Bar", "Baz"])`
  - Unification: two Error types unify if segments match; `Error(any)` unifies with `Error(specific)`
- **Semantic:** In `collect_constraints_expr`, when encountering an `err` expression:
  - Extract taxonomy segments from the error literal
  - Assign `TypeId::Error(segments)` instead of the current approach
  - At catch/match sites for errors, track which error types are handled
- **Diagnostic:** Warn when a function can produce error types not handled by callers
  - `check_error_exhaustiveness()` pass after constraint solving
- **Codegen:** No change needed — error representation is unchanged at runtime

**Files:** `src/types/core.rs`, `src/types/solver.rs`, `src/semantic.rs`  
**Tests:** Error type inference from literal, error type narrowing in match, exhaustive error handling warning, nested error taxonomy  
**Estimate:** 5 tests

---

### 6. R2.4 — Cooperative Yielding (Medium)

**Goal:** Insert yield points at loop back-edges in actor message handlers. Prevents a single CPU-bound actor from starving others.

**Implementation Plan:**
- **Runtime:** Add `yield_counter: u32` and `YIELD_THRESHOLD: u32 = 1000` to actor context
  - `coral_actor_yield_check()` FFI: increment counter, if > threshold, yield thread via `thread::yield_now()` and reset
- **Codegen:** At loop back-edges (`While`, `For`, `Loop`) inside actor handler functions:
  - Detect if current function is an actor handler (via `FunctionContext` flag)
  - Insert `coral_actor_yield_check()` call at the start of each loop iteration
- **Semantic:** Mark actor handlers in function metadata for codegen to detect

**Files:** `runtime/src/actor.rs`, `runtime/src/actor_ops.rs`, `src/codegen/mod.rs`, `src/codegen/runtime.rs`  
**Tests:** CPU-bound actor yields to peers, yield threshold configurable, non-actor loops unaffected  
**Estimate:** 3 tests

---

### 7. C4.5 — Profile-Guided Optimization (Medium)

**Goal:** Support `--emit-profile` to generate instrumented binaries and `--use-profile` to apply profile data. Hot paths get aggressive optimization; cold paths optimized for size.

**Implementation Plan:**
- **CLI:** Add `--emit-profile` and `--use-profile <path>` flags to `src/main.rs`
- **Codegen:** 
  - `--emit-profile`: Add LLVM IR instrumentation pass via `Module::run_passes("pgo-instr-gen")` from pass manager
  - Generate instrumented binary that writes `default.profraw` on exit
  - `--use-profile`: Load profile data via `Module::run_passes("pgo-instr-use")` with `PGOOptions`
  - Apply profile weights to branch probabilities
- **Workflow:** `coral --emit-profile prog.coral && ./prog && coral --use-profile default.profdata prog.coral`

**Files:** `src/main.rs`, `src/codegen/mod.rs`  
**Tests:** Instrumented binary runs and produces profile data, profile-guided compilation succeeds, PGO binary produces correct output  
**Estimate:** 3 tests

---

### 8. L3.1 — std.http (Client Foundation) (High)

**Goal:** HTTP/1.1 client with `get(url)`, `post(url, body)`, `request(method, url, headers, body)`. Server deferred to Sprint 6.

**Implementation Plan:**
- **Runtime:** New `runtime/src/http_ops.rs` module
  - Use `ureq` crate (sync HTTP client, no async runtime needed, small dependency tree)
  - `coral_http_get(url: i64) -> i64` — returns response as map `{status, headers, body}`
  - `coral_http_post(url: i64, body: i64) -> i64` — POST with body
  - `coral_http_request(method: i64, url: i64, headers: i64, body: i64) -> i64` — generic request
  - Response format: `map("status": 200, "body": "...", "headers": map(...))`
  - Errors return `err Http:Connection:Failed` or `err Http:Status:404` etc.
- **Codegen:** Add `http_get`, `http_post`, `http_request` builtins in `builtins.rs`
- **Stdlib:** Update `std/net.coral` with high-level wrappers
- **Examples:** Unblock `http_server.coral` (client part)

**Files:** `runtime/Cargo.toml`, `runtime/src/http_ops.rs`, `runtime/src/lib.rs`, `src/codegen/builtins.rs`, `src/codegen/runtime.rs`, `src/semantic.rs`, `std/net.coral`  
**Tests:** GET request to httpbin, POST with body, request with headers, error on invalid URL, response parsing  
**Estimate:** 5 tests

---

### 9. CC5.1 — Fuzz Testing (Medium)

**Goal:** Fuzz the lexer and parser with cargo-fuzz / libfuzzer to find crash and hang bugs.

**Implementation Plan:**
- Add `fuzz/` directory with `Cargo.toml` for cargo-fuzz
- **fuzz_lexer:** Feed random bytes to `Lexer::new().tokenize()` — must not panic, always returns Ok or Err
- **fuzz_parser:** Feed random token streams to `Parser::new().parse()` — must not panic
- **fuzz_semantic:** Feed random valid-ish ASTs to semantic analysis — must not panic
- Add CI-compatible seed corpus from existing test files
- Document how to run: `cargo fuzz run fuzz_lexer -- -max_len=1024`

**Files:** `fuzz/Cargo.toml`, `fuzz/fuzz_targets/fuzz_lexer.rs`, `fuzz/fuzz_targets/fuzz_parser.rs`  
**Tests:** Infrastructure setup (no counted tests — fuzzing runs continuously)  
**Estimate:** 0 counted tests (infrastructure)

---

### 10. L4.1 — std.debug Module (Medium)

**Goal:** `inspect(value)`, `trace(label, value)`, `time_it(label, fn)` for developer debugging.

**Implementation Plan:**
- **Runtime:** New FFI functions in `runtime/src/lib.rs` or dedicated `debug_ops.rs`:
  - `coral_debug_inspect(value: i64) -> i64` — returns string with type info + pretty-printed value (e.g. `"Number(42.0)"`, `"String(hello)[len=5]"`, `"List[3 items]"`)
  - `coral_debug_type_of(value: i64) -> i64` — returns type name as string (`"Number"`, `"String"`, `"List"`, `"Map"`, `"Bool"`, `"Unit"`, `"None"`, `"Store(Point)"`)
  - `coral_debug_time_ns() -> i64` — returns monotonic nanosecond timestamp as number
- **Codegen:** Add `inspect`, `type_of`, `time_ns` builtins
- **Stdlib:** `std/debug.coral` with higher-level wrappers:
  - `*inspect(value)` — prints type+value, returns value (passthrough for chaining)
  - `*trace(label, value)` — prints `[label] type: value`, returns value
  - `*time_it(label, f)` — calls f, prints elapsed time, returns result

**Files:** `runtime/src/lib.rs` (or `debug_ops.rs`), `src/codegen/builtins.rs`, `src/codegen/runtime.rs`, `src/semantic.rs`, `std/debug.coral`  
**Tests:** inspect returns correct type strings, type_of covers all types, time_it measures elapsed, trace passthrough  
**Estimate:** 4 tests

---

### 11. R3.2 — WAL Compaction (Medium)

**Goal:** Periodic compaction of the write-ahead log: merge committed entries, remove stale versions, reclaim disk space.

**Implementation Plan:**
- **Runtime:** Add `compact_wal()` method to store engine in appropriate runtime module
  - Read all WAL entries, keep only latest version of each key
  - Write compacted entries to new WAL file
  - Atomic swap: rename new WAL over old
  - Track WAL size; auto-trigger compaction when size > threshold (e.g., 10x data size)
- **FFI:** `coral_store_compact()` — exposed for manual compaction from Coral code
- **Auto-compaction:** Optional background compaction after N write operations

**Files:** `runtime/src/lib.rs` (store engine section)  
**Tests:** Compaction reduces WAL size, data integrity after compaction, concurrent read during compaction  
**Estimate:** 3 tests

---

### 12. M3.5 — Weak Reference Optimization (Medium)

**Goal:** Profile and optimize weak ref overhead. Consider epoch-based reclamation for validity checks instead of global registry lookups.

**Implementation Plan:**
- **Runtime:** Profile current `WeakRef` access cost (registry mutex + HashMap lookup per deref)
- Replace global `Mutex<HashMap<usize, bool>>` weak registry with epoch-based scheme:
  - Each epoch is a generation counter on the target value
  - `WeakRef` stores `(ptr, epoch)` — validity check is `(*ptr).epoch == stored_epoch`
  - No mutex, no hashmap — single memory load + compare
  - Add `epoch: u32` field to `Value` struct (fits in existing alignment padding)
  - On value deallocation, increment epoch (invalidates all weak refs)
- **FFI:** Update `coral_weak_ref_new`, `coral_weak_ref_deref`, `coral_weak_ref_is_valid`

**Files:** `runtime/src/lib.rs` (weak ref section)  
**Tests:** Epoch-based validity check, deallocation invalidates weak refs, concurrent weak ref access, performance comparison  
**Estimate:** 4 tests

---

## Summary Table

| # | ID | Task | Pillar | Complexity | Tests |
|---|------|------|--------|------------|-------|
| 1 | R2.1 | Work-stealing scheduler | Runtime | High | 4 |
| 2 | R2.2 | Lock-free actor registry | Runtime | Medium | 3 |
| 3 | R2.7 | Typed messages | Runtime+Types | High | 4 |
| 4 | R2.8 | Actor monitoring | Runtime | High | 4 |
| 5 | T3.4 | Error type tracking | Types | High | 5 |
| 6 | R2.4 | Cooperative yielding | Runtime | Medium | 3 |
| 7 | C4.5 | Profile-guided optimization | Compiler | Medium | 3 |
| 8 | L3.1 | std.http (client) | Stdlib | High | 5 |
| 9 | CC5.1 | Fuzz testing | Quality | Medium | 0 |
| 10 | L4.1 | std.debug module | Stdlib | Medium | 4 |
| 11 | R3.2 | WAL compaction | Runtime | Medium | 3 |
| 12 | M3.5 | Weak ref optimization | Memory | Medium | 4 |

**Total estimated new tests:** ~42  
**Pillar breakdown:** Runtime 5, Types 1, Compiler 1, Stdlib 2, Quality 1, Memory 1  
**Dependencies:** R2.1 should precede R2.4 (yielding needs scheduler). T3.4 is independent. L3.1 is independent. CC5.1 is independent.

---

## Implementation Order

**Phase A (Independent — can be parallelized):**
1. T3.4 — Error type tracking (compiler only, no runtime changes)
2. CC5.1 — Fuzz testing infrastructure
3. L4.1 — std.debug module
4. M3.5 — Weak ref optimization

**Phase B (Actor system — sequential):**
5. R2.1 — Work-stealing scheduler (foundation)
6. R2.2 — Lock-free actor registry (builds on scheduler)
7. R2.4 — Cooperative yielding (needs scheduler)
8. R2.8 — Actor monitoring (needs registry)

**Phase C (Type safety + networking):**
9. R2.7 — Typed messages (needs actor system stable)
10. L3.1 — std.http client

**Phase D (Optimization):**
11. C4.5 — PGO
12. R3.2 — WAL compaction

---

## Post-Sprint 5 Outlook

After Sprint 5, the major remaining roadmap items will be:
- **R2.3** Message dispatch optimization (integer tags)
- **R2.5** Actor state pinning
- **R2.9** Supervision hardening
- **R2.11** Remote actors (foundation)
- **R3.1, R3.3-R3.8** Store engine performance (indexes, mmap, ACID, queries)
- **T2.5** Monomorphization
- **C2.4-C2.5** Unboxed lists, store field specialization
- **C5.1-C5.4** Advanced comptime features
- **M4.1-M4.4** Escape analysis & stack allocation
- **CC4.1-CC4.4** WASM, macOS, Windows, static linking
- **L3.2-L3.5** URL, UDP, crypto, CSV
- **L4.3-L4.5** Collections, docs, packages
- **R5.1-R5.12** Self-hosted runtime
- **S2.7** Tuple syntax (remaining S2 item)
- **L3.1** std.http server (Sprint 5 does client only)
