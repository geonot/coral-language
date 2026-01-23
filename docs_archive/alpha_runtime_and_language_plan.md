# Coral Alpha Runtime & Language Plan

_Last updated: 2025-12-10_

This consolidates runtime, actor, memory, metrics, higher-order, type-system, and error-model plans. It supersedes `actor_runtime_status.md`, `runtime_module_actor_plan.md`, `runtime_memory_plan.md`, `runtime_metrics.md`, `runtime/closures.md`, `higher_order_plan.md`, `libc_io_plan.md`, `performance_plan.md`, and `value_error_model.md`.

## Actors & Concurrency
- **Runtime state:** M:N scheduler, frozen messages, parent tracking, `coral_actor_spawn/send/stop/self` FFI. Main runs as an actor supervisor.
- **Dispatch (current):** Message map `{name,data}`; handler compares `name` to `@` methods and invokes mangled functions; payload passed as `Any` when handler has one param.
- **Near-term (alpha):**
  - Struct layout for actor state and fields; `self` access in handlers.
  - Dispatch table keyed by interned strings/ids (avoid per-message string comparisons).
  - Backpressure (bounded mailboxes), failure messages, supervision policies, timers, cancellation.
  - Typed message contracts + arity checks; better ergonomics (e.g., `actor ! msg(payload)`).

## Runtime Modules & IO
- Pluggable modules for alloc/io/time/metrics/scheduler via capability registry and optional dynamic loads (`CORAL_RUNTIME_MODULES=...`).
- Std IO surface (`std.io` → `coral_fs_read/write/exists`); roadmap: directory helpers, streaming APIs, virtual FS hooks, permissions.

## Memory & Performance
- Refcounted tagged `Value`; value pool + telemetry counters (retains/releases/live/slots/stack pages).
- Plans: 64-bit (atomic for actors) refcounts, release queues, cycle detection, arenas for stack-like temps, telemetry-driven sizing for lists/maps/actors.
- Performance track: typed MIR for unboxed hot paths, arena-backed temporaries, PGO hooks from metrics JSON, Criterion benchmarks (string concat, list/map ops, dispatch).

## Metrics & Feedback Loop
- `CORAL_RUNTIME_METRICS` or `--collect-metrics` dumps JSON (`retains`, `releases`, `live_values`, pool hits/misses, heap bytes, string bytes, list/map slots, stack pages/bytes`).
- Compiler should consume snapshots to pre-size literals, arenas, actor mailboxes, and persistence pages; track drift for regressions.

## Closures, Lambdas, and HOFs
- Closure ABI: `CoralClosure { invoke(void* env, ValueHandle* args, usize len, ValueHandle* out), release, env }` with runtime helpers `coral_make_closure` and `coral_closure_invoke`.
- Placeholder sugar (`$`, `$1`) desugars to lambdas with synthesized params; captures retained in env struct; release drops captures.
- Next: finish closure codegen + runtime invoke; implement `list.map/filter/reduce` using closures; retain/release batching (`retain_many/release_many`).

## Type System
- HM-style constraint solver with union-find; primitive types (Int/Float/Bool/String/Bytes/Unit/Actor/Any), generics (List[T], Map[K,V]), function types.
- Planned surface: annotations on bindings/params/returns, literal suffixes (`42i64`, `3.14f32`, `b""`), pointer/bitvector types gated by `unsafe`.
- Phases: primitive inference → collections → closures/HOFs → traits/effects (IO/Actor/Comptime) later.

## Error & Absence Model
- Proposed flags on `Value` header: `ERR`, `ABSENT` with payload (`code`, `message`, `origin_span`).
- Helpers: `is_ok/is_err/unwrap/or/expect`; IO/runtime setters propagate flags; optional `?` desugar to `unwrap`.

## Open Research / Later
- Sum types/union syntax, pipelines/chaining operator, effect handlers.
- Inline asm and explicit `unsafe` blocks for low-level interop once runtime is stable.
