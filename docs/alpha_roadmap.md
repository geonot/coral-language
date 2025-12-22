# Coral Alpha Roadmap

_Last updated: 2025-12-10_

This replaces `docs/roadmap.md`, `feature_completion_plan.md`, and `compiler_gap_analysis.md`. It aligns delivery priorities for the alpha release.

## NOW (Stabilize + Actor MVP)
- Actor dispatch table + typed envelopes; actor state layout and `self` field access.
- Closure ABI + list/map HOF runtime (`map/filter/reduce`); placeholder desugar already lands.
- Hash-backed maps; pre-size collections from literal length; retain/release instrumentation with 64-bit counters.
- Semantic depth: undefined-name diagnostics; store/actor validation (fields/defaults, @ handler arity) and basic type constraints for bitwise ops.
- CI sanity: `cargo fmt/clippy`, IR smoke + actor send test via `--jit`, metrics snapshot check.

## NEXT (Performance + Safety)
- Backpressure, mailbox limits, and supervision policies; failure propagation tests.
- Typed MIR fast-paths for numeric ops; arena-backed temporaries with copy-on-escape; `--emit-bin` path via `llc/clang` smoke.
- Runtime telemetry ingestion in compiler (arena sizing, map/list capacity, actor mailbox budgets).
- Memory safety drills: ASAN/Miri stress harness for refcounts, list/map, actor send; leak/regression dashboards.
- Std IO expansion: directory helpers, basic VFS hooks, streaming readers/writers.

## LATER (Language Depth + Tooling)
- Effect typing (IO/Actor/Comptime) and gradual annotations; trait/interface exploration.
- Sum types and richer pattern matching; pipeline/chaining operator decision.
- Error/absence flags on `Value` plus std helpers; optional `?` desugar.
- Package/formatter/language-server prototypes; module cache/fingerprints for incremental builds; `coralc run/build/test` UX.
- PGO: consume metrics JSON to tune arena sizes and literal lowering; benchmarks (Criterion) and perf budgets.

## Definitions of Done for Alpha
- Actor programs run end-to-end with bounded mailboxes, dispatch tables, failure propagation, and typed envelopes.
- Closure/HOFs executing with retained captures; list/map HOFs validated by runtime tests.
- Hash-backed map + typed MIR numeric path for core arithmetic; collection pre-sizing from literals and telemetry.
- Semantic checks for undefined names, store/actor validation, and basic type constraints.
- CI running fmt/clippy + smoke suites (parser fixtures, semantic negatives, runtime, actor send) and at least one ASAN/Miri stress job.

## Key References
- Runtime/type/actor plans: `docs/alpha_runtime_and_language_plan.md`.
- Overview/status snapshot: `docs/alpha_overview.md`.
