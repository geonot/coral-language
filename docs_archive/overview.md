# Coral Overview

_Last updated: 2025-12-04_

Coral is a "post-modern" systems language that borrows Python's readability while targeting LLVM for performance. The `coralc` compiler in this repo currently lexes, parses, performs light semantic checks, and lowers Coral programs to LLVM IR backed by a tagged `Value` runtime.

## Language Snapshot
- **Surface style:** Significant indentation, expression-oriented blocks, suffix method calls, taxonomy literals (`!!Name:Child`), and placeholder shorthand (`prices.map($ * 1.15)`).
- **Data literals:** Numbers, strings (single/double quotes with interpolation), lists, and `map(...)` literals.
- **Structural types:** `type` (product types), `store` (persistent structs), and `store actor` (message handlers). Parsing exists; lowering is pending.
- **Control flow:** Ternaries (`cond ? a ! b`), `match` expressions, logical `and`/`or`, lambda literals, and function calls (`*name(args)` for definitions).

## Compiler Architecture
1. **Frontend**
   - `lexer`: Emits indentation-aware tokens (`INDENT`, `DEDENT`) plus Coral keywords and literals.
   - `parser`: Recursive-descent with layout recovery for missing `INDENT/DEDENT`, producing AST nodes in `ast.rs`.
   - `ast`: Immutable trees with `Span`s for diagnostics.
2. **Middle-end**
   - `lower`: Rewrites placeholder arguments into synthesized lambdas so later stages never see `$` nodes.
   - `semantic`: Validates duplicate functions/bindings/parameters and scopes block-local names. Store/type metadata is recorded but not yet lowered.
3. **Backend**
   - `codegen`: Uses Inkwell to emit LLVM IR. Expressions lower to tagged `Value` handles, calling runtime intrinsics for constructors and helpers.
   - `compiler`: Orchestrates the pipeline and exposes `compile_to_ir` used by CLI/tests.
4. **Runtime (`runtime/`)**
   - Tagged union (`Value`) covering `Number`, `Bool`, `String`, `List`, `Map`, `Store`, `Actor`, `Unit`.
   - Inline storage for small strings/bools, heap objects (`StringObject`, `ListObject`, `MapObject`) with refcounts.
   - Exported helpers: `coral_make_*`, `coral_list_push/length/get`, `coral_map_get/set`, `coral_value_add/equals`, heap alloc/free, and telemetry intrinsics for runtime metrics dumps.

## Current Capabilities
- Strings (including template literals), taxonomy values, lists, maps, logical ops, and ternaries lower end-to-end via runtime helpers.
- Parser emits actionable diagnostics for missing indentation via layout recovery hooks.
- Parser fixture matrix (`tests/fixtures/parser`) exercises both valid programs and curated failure cases (missing indent, inconsistent dedent, mixed tabs/spaces) to guard against layout regressions, and JSON AST snapshots (`tests/parser_snapshots.rs`) ensure valid programs keep a stable tree shape.
- Lexer enforces indentation alignment *and* homogeneous whitespace usage, rejecting any outdent that does not match a previously observed indent width or that mixes tabs and spaces, so malformed layout never reaches the parser.
- Semantic layer catches duplicate globals/locals/parameters, duplicate store/type fields, and rejects default parameters that reference later parameters.
- Runtime exposes `list.push/length/get/pop`, `map.get/set/size`, and string/list/map constructors, so common collection workflows can execute end-to-end.
- Runtime can emit allocator telemetry (`--collect-metrics` or `CORAL_RUNTIME_METRICS=...`) so future compilations can reuse live allocation data for arena sizing.
- Smoke tests exercise runtime helpers (`coral_value_add/equals`, list/map literals, member calls) plus the comprehensive fixture in `tests/fixtures/programs/full_language_no_store.coral` that compiles nearly all non-store/actor constructs from `syntax.coral`.

## Module System Snapshot

- `use module_name` directives are expanded by a pre-lexer `ModuleLoader`, enabling hierarchical imports without changing the AST.
- The CLI wires the loader with the bundled `std/` directory so programs can `use std.prelude`, `use std.math`, etc.
- Tests or tools can call `ModuleLoader::with_default_std()` (or inject custom search paths) to mirror CLI behavior.

## Known Gaps
1. **Stores/actors lowering:** Parsed but not emitted; requires runtime representation and codegen for fields/methods/actors.
2. **Closures + higher-order helpers:** Placeholder lowering exists, but there is no closure ABI or runtime capture structs, so `map`/`reduce` are still surface-only.
3. **Match patterns:** Only integer patterns lower; string/bool/list patterns need runtime support.
4. **Diagnostics depth:** Parser still reports only the first error per file; semantics lacks undefined-name detection, unused-binding checks, and richer actor/store validation.
5. **Perf/safety:** No Criterion benchmarks or leak/analyzer runs for the runtime; reference counting is unverified under stress and maps remain O(n).

Refer to `README.md` for quickstart commands and to `docs/roadmap.md` for the living delivery plan.
