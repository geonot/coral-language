# Coral Alpha Overview

_Last updated: 2025-12-10_

This document consolidates the high-level intent, current state, and surface language snapshot for the Coral compiler/runtime. It replaces `about.md`, `status.md`, and `docs/overview.md`.

## Vision
- Readable, indentation-driven systems language with Python-like ergonomics and LLVM-backed performance.
- Deterministic, GC-free memory via refcounted `Value` plus arenas; actors as the core concurrency primitive; stores as persistent data containers.
- First-class tooling: module loader, std library, telemetry-fed optimczsizations, and a path to self-hosting.

## Current Status
- **Frontend:** Layout-aware lexer/parser; placeholder → lambda lowering; semantics catch duplicate bindings/params/default-order and record actors/stores.
- **Backend:** LLVM IR emission for literals, lists/maps, template strings, taxonomy literals, logical/match lowering; main runs as an actor supervisor and dispatches to `__user_main`. **Stores:** constructors (`make_StoreName()`), field access (`instance.field`), methods with `self` parameter and assignment syntax (`self.field is value`), reference fields (`&field`) with retain/release semantics.
- **Runtime:** Tagged `Value` with refcounts, list/map helpers, string/bytes support, actor runtime (M:N scheduler, frozen messages, parent tracking), runtime metrics dump.
- **Stdlib:** Minimal `std.prelude`, `std.io`, `std.bit`; module loader resolves bundled `std/` paths.

## What Works End-to-End
- Expressions (arithmetic via runtime helpers), lists/maps (constructors, get/set/push/pop/length), logical ops, ternaries, match (numeric patterns), taxonomy literals, template strings.
- Module loading for `std` and sibling `.coral` files; IR + JIT via `--jit` and optional `--emit-binary` path.
- Actor entrypoint: `main` becomes an actor; `actor_send`/`actor_self` builtins and message dispatch by name for `@` handlers (payload as `Any`).
- **Stores:** Define data structures with `store Name`, create instances with `make_Name()`, access fields with `.field` syntax, define methods with `*method_name(params)`, use reference fields with `&field` syntax for automatic memory management. Methods take `self` as hidden first parameter and support field assignment via `self.field is value`.

## Major Gaps (Alpha Blockers)
- Actor state layout and field access; typed message contracts; backpressure, supervision policies, timers.
- Closure ABI for higher-order helpers; list/map HOFs (`map/filter/reduce`) not executable yet.
- Type inference and undefined-name diagnostics; effect typing for IO/actor/comptime.
- Hash-backed maps, perf/RC stress tests, and CI coverage; richer error model beyond tagged `Value` flags.
- Store limitations: only `self.field is value` in methods (general instance assignment requires explicit `.set()`), no type checking for reference fields, no circular reference detection.

## Language Snapshot (Core Subset)
- Top-level: bindings, `*fn` functions, `type`, `store` (with constructors, methods, and reference fields), `actor` (parsed), `use module.path`.
- Expressions: numbers, bools, strings (with interpolation), bytes, lists, maps, taxonomy `!!A:B`, calls, member access, match, ternary, lambdas/placeholder sugar, logical and bitwise helpers (via runtime `std.bit`).
- Runtime value tags: Number, Bool, String, Bytes, List, Map, Store, Actor, Unit, Closure.

## Quick Usage
- Build: `cargo build`
- Emit IR: `cargo run -- file.coral --emit-ir out.ll`
- JIT: `cargo run -- file.coral --jit [--collect-metrics metrics.json]`
- Std modules: use `ModuleLoader::with_default_std()` in tests/tools.

See `docs/alpha_runtime_and_language_plan.md` for deeper runtime/type plans and `docs/alpha_roadmap.md` for delivery priorities.
