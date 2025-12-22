# Runtime Metrics & Feedback Loop

_Last updated: 2025-12-06_

## Overview
The Coral runtime now exposes a first-class telemetry channel so that allocation strategies can be tuned using real execution data. At runtime every allocation path reports per-type counters (bytes, slots, retain/release churn, stack pages, pool hits/misses). When the environment variable `CORAL_RUNTIME_METRICS` is set, the runtime writes a JSON snapshot of these counters at process exit.

The compiler CLI (`coralc`) wires this up via the `--collect-metrics <FILE>` flag when running with `--jit`. This allows iterative workflows:

1. Compile + run workload with `coralc --jit --collect-metrics metrics.json program.coral`.
2. Inspect `metrics.json` (or feed it into analysis tools) to learn how many values, list slots, map entries, and bytes were touched.
3. Feed the insights back into compiler passes (arena sizing, literal lowering, actor persistence budgets) for the next build.

## JSON Payload
Example payload:

```json
{
  "timestamp_ns": 1765000000123456789,
  "retains": 128,
  "releases": 128,
  "live_values": 4,
  "value_pool_hits": 512,
  "value_pool_misses": 16,
  "heap_bytes": 65536,
  "string_bytes": 8192,
  "list_slots": 96,
  "map_slots": 12,
  "stack_pages": 4,
  "stack_bytes": 16384
}
```

Field descriptions:

- `timestamp_ns`: monotonic-ish timestamp when the snapshot was produced.
- `retains`, `releases`, `live_values`: ARC health indicators.
- `value_pool_hits`/`misses`: effectiveness of the recycled `Value` arena.
- `heap_bytes`: approximate heap pressure caused by runtime objects (values + heap-backed payloads).
- `string_bytes`, `list_slots`, `map_slots`: per-type sizing information.
- `stack_pages`, `stack_bytes`: scratch arena usage, useful for deciding default `@stack_pages` annotations.

## Integrating With Compiler Passes
- **Arena Pre-sizing:** Use `list_slots` and `map_slots` to adjust literal lowering so collections start with realistic capacities.
- **Actor Mailboxes:** Feed `value_pool_hits`/`misses` into actor store planning to decide when to dedicate arenas vs shared heaps.
- **Persistence Pages:** Combine `heap_bytes` and `stack_bytes` to choose snapshot chunk sizes for persistent stores.
- **Regression detection:** Track metrics over time; significant drifts highlight leaks or regressions before they hit production workloads.

## Manual Snapshotting
In addition to the env var, Coral exposes two runtime intrinsics:

- `coral_runtime_metrics(out_ptr)` – populates a `CoralRuntimeMetrics` struct.
- `coral_runtime_metrics_dump(path_ptr, len)` – immediately writes the JSON snapshot to disk.

These APIs enable programmatic sampling from Coral code (e.g., via future `std.metrics` helpers) or external harnesses without waiting for process exit.
