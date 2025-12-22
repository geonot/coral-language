# Coral Roadmap · November 2025

This roadmap replaces the previous multi-file plan. It tracks the single source of truth for goals, milestones, and concrete tasks. Status markers: ✅ (done), 🚧 (in progress), ☐ (not started).

## Snapshot
- **Runtime:** Tagged `Value` runtime handles strings, lists, maps, equality, and arithmetic via FFI helpers.
- **Backend:** LLVM lowering covers literals, member helpers, logical operators, and match expressions.
- **Frontend:** Layout-aware parser recovery exists; semantic scope checks catch duplicate bindings/params.
- **Gaps:** Stores/actors untouched, maps lack hashing, parser only reports the first error per file, and no perf/leak harnesses exist.

## Goal 1 – Harden Frontend Diagnostics (NOW)
1. 🚧 **B1.2 Layout Recovery Polish**
   - Add explicit diagnostics for extra dedents/indents and snapshot them in tests.
   - Emit secondary notes pointing to matching block starts.
   - ✅ Lexer now rejects misaligned dedents *and* mixed tab/space indentation before they reach the parser; fixtures/assertions cover these diagnostics.
2. ✅ **B2.2 Default Parameter Rules**
   - Semantic analyzer now rejects defaults referencing later parameters (see `tests/semantic.rs`), unblocking deeper store/type validation work.
3. ✅ **B3 Parser Fixture Matrix**
   - Added `tests/fixtures/parser/{valid,invalid}` corpus plus `parser_fixtures.rs` tests that ensure valid programs parse cleanly and curated failures surface the expected diagnostics.
   - JSON AST snapshots (`tests/parser_snapshots.rs`) now guard valid fixtures against structural regressions; keep expanding the corpus as new syntax lands.

## Goal 2 – Runtime Depth & Semantic Safety (NEXT)
1. ☐ **A3.1 Higher-order List Helpers**
   - Implement `coral_list_map`, `coral_list_reduce`, and iterator helpers with corresponding codegen paths.
2. ☐ **A3.2 Hash-backed Maps**
   - Choose hashing strategy (SipHash/FNV), define bucket layout, and wire equality semantics for composite keys.
3. ☐ **B2.3 Store/Type Validation**
   - Ensure store/type field defaults are evaluated safely; block reference cycles and invalid references.
4. ☐ **B3.2 Semantic Regression Suite**
   - Add positive/negative semantic fixtures (duplicate stores, invalid defaults, actor misuse) with snapshot diagnostics.

5. 🚧 **A3.3 Telemetry-driven Allocation Feedback**
   - Instrument the runtime with per-type allocation counters, pool hit/miss ratios, and stack arena usage.
   - Dump metrics to JSON via `CORAL_RUNTIME_METRICS` or an explicit CLI flag so future compilations can fold live data into lowering heuristics.
6. ☐ **B2.4 Actor-ready Persistence Hooks**
   - Define storage schemas for persistent actors/stores and use the allocator metrics to size mailboxes and snapshot buffers ahead of time.

## Goal 3 – Performance & Production Readiness (LATER)
1. ☐ **A4.1 Benchmarks**
   - Criterion harnesses for string concat, list push, map lookup, and equality to establish baselines.
2. ☐ **A4.2 Memory Safety Review**
   - Run `asan`/`valgrind` against generated IR that hammers the runtime; document findings in `docs/overview.md`.
3. ☐ **Deployment Tooling**
   - Add CI tasks running smoke tests, parser fixtures, semantic suites, and property tests separately.
   - Expose `cargo coralc --emit-bin` path (llc/clang) to produce native binaries once runtime coverage is broader.

4. ☐ **A4.3 Profile-guided Recompilation**
   - Consume the JSON telemetry generated at runtime and feed it into compiler passes (list/map literal sizing, arena selection, actor mailbox budgets).
   - Ship a minimal heuristic that uses historical sizes to choose between stack arenas, pooled heap values, or persistent pages per module.

## Timeline Guide
- **NOW:** Goal 1 – unblock day-to-day ergonomics and diagnostics.
- **NEXT:** Goal 2 – expand runtime semantics once diagnostics are trustworthy.
- **LATER:** Goal 3 – harden for long-running programs and prepare for stores/actors + performance work.

This file supersedes the previous `docs/plan.md`, `docs/value-runtime-audit.md`, and `ROADMAP.md`. Update it whenever tasks complete or priorities shift.
