# Runtime Module System & Actor Runtime Plan

## Goals
- Pluggable runtime modules: allow extending/overriding runtime services (allocators, IO, schedulers, metrics) without recompiling the compiler.
- Feedback loop: runtime emits structured usage/telemetry for future compiler runs to tune allocation and inlining decisions.
- Actor-first concurrency: `main` runs as an actor/supervisor; actors are green-threaded, spawn and supervise children, with failure isolation.

## Runtime Module System
- Module registry: each module exposes a descriptor `{name, version, init, shutdown, capabilities}`. The runtime holds a thread-safe registry and dispatch table.
- Capabilities: alloc (custom allocators/arenas), io (file/net), time, metrics, scheduler hooks, tracing/logging sinks.
- Loading: built-in modules compiled in; dynamic modules loadable via shared objects flagged by env/CLI (`CORAL_RUNTIME_MODULES=path1,path2`).
- Configuration: runtime reads a module manifest at startup; compiler can embed defaults based on target.
- Dispatch: subsystems call through a thin vtable; if no module provided, fall back to builtin implementation.
- Safety: capability-based—modules only receive pointers to their domains; no global mutable state sharing.
- Hot-swap (optional, later): allow swapping a module at safe points (no outstanding handles) for long-running services.

## Feedback Loop (Usage Metrics)
- Runtime already tracks stack/heap usage and copy/COW events (new `UsageKind`).
- Extend metrics sink to periodically write snapshots (JSON/line-delimited) to a rotating file (`$TMP/coral_usage*.log`).
- Compiler ingest: on next compile, read latest snapshot and adjust heuristics:
  - Prefer stack/arena for symbols marked immutable/non-escaping with low spill rate.
  - Switch to heap/COW for types showing frequent mutations or COW breaks.
  - Inline hotspots identified by high call counts.
- Privacy: snapshots are local-only; provide an opt-out env flag.

## Actor Runtime (Initial Cut)
- Main as actor: program entry spawns `actor main` supervised by root; process exits when root shuts down.
- Green threading: actor mailbox + cooperative scheduler (MPSC queues). Use work-stealing for multi-core.
- Supervision tree: actors can spawn children; parent monitors exits; restart strategies (one-for-one / one-for-all) configurable.
- Messaging: immutable payloads; leverage immutable inference to pass by share/no-copy. Larger payloads use COW + refcount.
- Mailboxes: bounded queues with backpressure signals; overflow policy: drop oldest / reject sender / block with timeout.
- Timers: scheduler-managed timers; timer events delivered as messages.
- Failure semantics: panics/exits propagate to supervisor via `Exit(Reason)` message; supervisors decide restart/stop/escalate.
- Cancellation: cooperative; actor checks cancellation flag set by supervisor.
- Isolation: no shared mutable state; cross-actor comms via messages only; async IO integrated via runtime module hooks.

## Near-Term Implementation Steps
1) Wire SemanticModel hints into lowering/codegen to choose alloc path (stack/arena vs heap) and enable COW for read-mostly collections.
2) Add runtime usage snapshot writer (periodic) and compiler-side reader to feed heuristics.
3) Add module registry scaffold with capability enums and dispatch table; keep builtin defaults.
4) Introduce actor primitives in runtime: mailbox struct, scheduler loop, spawn/supervise APIs; make `main` an actor entry.
5) Expose diagnostics: report per-symbol alloc decisions and spill/fallback counts during compile.

## Safety/Testing
- Provide a `RUNTIME_MODULE=debug` module that logs all allocations/messages for tracing.
- Add stress tests for supervisor restart strategies and mailbox backpressure.
- Fuzz message passing with random failures to validate isolation.
