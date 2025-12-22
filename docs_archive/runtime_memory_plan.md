# Runtime Memory & Comptime Execution Roadmap

_Last updated: 2025-12-04_

## 1. Reference Counting & Memory Safety

### Current baseline
- Manual `u16` refcounts on every `Value`. No atomic ops, no saturation diagnostics, no cycle detection.
- Heap objects (`StringObject`, `ListObject`, `MapObject`, future closures) use `Vec` storage without allocator instrumentation.
- No visibility into how often values are recycled vs freshly allocated; allocator heuristics are blind to real workloads.

### Modern best practices to adopt
1. **64-bit refcounts with debug instrumentation**
   - Switch to 64-bit counters (like Swift/Objective-C) to reduce overflow risk.
   - Guard increments/decrements with debug asserts; expose `coral_runtime.rc_stats()` for tooling.
2. **Delayed release queue**
   - Keep per-thread release ring buffers to amortize decrements (inspired by Firefox's Gecko RC).
3. **Cycle detection via trial deletion**
   - Run Tarjan-style cycle detection on batches (like Python's cyclic GC) for actors/stores.
4. **Atomic RC for cross-thread actors**
   - Use `Relaxed` increments + release fences (`SeqCst` on drop) similar to Swift's ARC under concurrency.
5. **Borrowed views / arenas**
   - Introduce `BorrowedList`, `BorrowedBytes` wrappers that reference parent `Value` without bumping RC (akin to Rust slices) for zero-copy substring/list slicing.

### Implementation milestones
1. **Instrumentation phase (NOW)**
   - Replace `u16` with `u64`, add `debug_assert!(refcount > 0)` in release path, log underflow.
   - Add `cargo test --features runtime-asan` target to run ASan/Miri on runtime stress tests.
   - ✅ Install a lock-free value pool and telemetry counters (hits/misses, bytes allocated, list/map slot counts, stack pages) that can be dumped to JSON.
2. **Hybrid RC (NEXT)**
   - Introduce `ValueHeader { atomic_refcount, flags }` for Actor/Store values; keep non-atomic for purely local values.
   - Add `retain_many` / `release_many` APIs so collections can bump counts in bulk.
3. **Cycle GC (LATER)**
   - Maintain graph of `Map`/`List` references; periodically run trial deletion to detect cycles among RC-only nodes.
4. **Deferred free list & arenas**
   - Provide region allocators for short-lived list literals to reduce malloc pressure, using bump allocators reset per function.
5. **Profile-guided sizing (ONGOING)**
   - Feed live telemetry (JSON) back into compilation to choose arenas/pages ahead of time for actors, persistent stores, and large literals.

## 2. Comptime Execution Strategy

Goal: allow portions of Coral code to execute during compilation when inputs are constant, similar to Zig or Jai.

Phases:
1. **Const-eval MIR**
   - Extend MIR interpreter to run in compiler context with a budgeted fuel counter.
   - Mark functions as `comptime` or infer when all arguments are literals.
2. **Effect isolation**
   - Disallow runtime-only intrinsics (IO, actors) during comptime; expose safe subset (`log`, math, list/map constructors).
3. **Memoization cache**
   - Cache comptime evaluation results keyed by AST hash + inputs; reuse across incremental builds.
4. **Partial evaluation**
   - Allow MIR blocks to be partially reduced, emitting residual code for runtime; this enables constant folding of user-defined functions.

## 3. Memory Management Research References
- **Swift ARC + Borrowing**: Automatic Reference Counting with ownership modifiers (`weak`, `unowned`). Useful for actor handles.
- **Rust ownership/borrowing**: Inspire `BorrowedValue` views for zero-copy slices.
- **Perceus (Lean 4)**: Reference counting with usage analysis to eliminate increments (future optimization pass).
- **Hazard pointers / epoch-based reclamation**: Consider for lock-free actor mailboxes once concurrency lands.

## 4. Production Hardening Checklist
- [ ] Stress harness that constructs/destroys millions of Values; run under ASan + Valgrind.
- [ ] Benchmarks measuring RC churn (list push/pop, map set/get) with perf budget.
- [ ] Structured logging of allocation sizes for flamegraphs.
- [ ] Tunable growth factors for `ListObject` and `MapObject` Vecs to avoid quadratic reallocation.
- [ ] Continuous fuzzing for runtime APIs (via cargo-fuzz) to uncover RC leaks.

## 5. Stack-frame Page Arenas ("Fil-C" style)
- **Idea:** upon entering a Coral function, reserve one or more OS pages (configurable per module) and treat them as a bump allocator for temporaries. Exiting the function simply rewinds the bump pointer—no per-object frees.
- **Motivation:** cheap allocation for short-lived values without `unsafe` or explicit `free`, mirroring techniques from Fil-C, Wasm stack allocators, and Jai's scratch arena.
- **Plan:**
   1. Compiler annotates stack allocations with estimated peak usage (derived from MIR). If a block's footprint exceeds one page, request multiple pages (rounded up to OS granularity).
   2. Runtime maintains a per-thread `StackArena { base, cursor, limit }`. On entry, snapshot cursor; on exit, restore snapshot. All allocations within the frame use `cursor += size`.
   3. For values escaping the frame (returned or captured), materialize them into the RC heap before returning.
   4. Provide language-level settings: `@stack_pages(2)` on functions or module-level defaults.
- **Research references:** `mir2wasm` scratch stack, Unity's Baselib page allocators, and the Fil-C "allocate & forget" arena.

## 6. Static vs Dynamic Collection Sizing
- **Telemetry hooks:** Every allocator (value pool, string, bytes, list, map, stack arena) now reports into `coral_runtime_metrics`. Use `CORAL_RUNTIME_METRICS=/tmp/coral.json` while running `coralc --jit` (or set `--collect-metrics`) to persist the live counters for later analysis.
- **Arena selection heuristics:** Consume the dumped JSON in future compiler passes to decide whether a given block should use stack pages, pooled heap `Value`s, or persistent actor storage.
- **Static inference:** during compilation, flag list/map literals whose length is known and whose size never changes. Allocate them exactly once in the stack arena or a fixed heap chunk, skipping growth logic.
- **Dynamic detection:** for collections fed by user input or loops, enable exponential growth with tunable factors (1.5x/2x). Track `CollectionKind::{Static, Bounded, Dynamic}` in MIR metadata.
- **Runtime strategy:**
   - Static collections → allocate via stack arena, copy-on-escape.
   - Bounded collections (size known upper bound) → preallocate `upper_bound` slots and use RC heap.
   - Fully dynamic collections → start with page-sized buckets to reduce reallocation thrash; attach telemetry counters to adjust heuristics.
- **No `unsafe` policy:** even though we use low-level arenas, all pointer math stays inside runtime abstractions. Compiler inserts bounds checks when bumping cursors.

## 7. Deferred Release & Borrowed Views (Starter)
- Introduce per-thread release queues (`ReleaseQueue`) that batch value drops; drain on safepoints.
- Add `retain_many/release_many` to collections to amortize RC churn.
- Borrowed slices (`BorrowedList/Bytes`) avoid refcount bumps for views; copy-on-escape to RC heap.

## 7. Hash-backed Maps & List/Map HOFs (Next)

- **Map storage:** replace `Vec<MapEntry>` with open-addressing buckets (SipHash-1-3, 64-bit hash, robin-hood insertion). Store `hash`, `key`, `value`, `state` per bucket; tombstones for deletions; resize at 0.7 load factor.
- **Equality semantics:** reuse `values_equal_handles` but extend numeric path to typed integers once typed MIR lands; forbid NaN keys by normalizing to a stable hash; support bytes/string/list/map keys via structural hash.
- **APIs:** `coral_map_get/set/contains/len/iter_keys/iter_entries` and `coral_map_from_pairs` for bulk build. Expose iterator structs for codegen.
- **List/Map HOFs:** add runtime helpers `coral_list_map`, `coral_list_filter`, `coral_list_reduce` that accept a closure handle; codegen lowers `$` placeholders into closures capturing environment; closures retain/release env once per invocation batch via `retain_many/release_many` when available.
- **Perf guardrails:** pre-size maps from literal length in codegen; reuse telemetry (`map_slots`) to pick initial capacity; benchmark map insert/lookup and list HOFs under ASAN + perf.
