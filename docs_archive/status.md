# Coral Compiler Status — 2025-12-04

## Executive Summary
The `coralc` toolchain now lexes, parses, and lowers nearly every construct in `syntax.coral` other than stores/actors and higher-order list APIs. Since November we added template-string tokenization + lowering, taxonomy-path parsing, placeholder ⇒ lambda desugaring, and a comprehensive program fixture that exercises strings, lists, maps, match expressions, taxonomy literals, and ternaries end-to-end through LLVM. The remaining large gaps are closure invocation (needed for `map`/`reduce`), store/actor lowering, and richer semantic analysis/type inference. This document highlights per-component strengths, gaps, and the work left to reach production-grade quality.

## Language Surface vs Implementation
| Feature (from `syntax.coral`) | Status | Notes |
| --- | --- | --- |
| Numeric/bool literals, bindings, arithmetic | ✅ | Fully lexed, parsed, and lowered via runtime helpers.
| String interpolation (`'Hello, {name}'`) | ✅ | Template strings lex into fragments; parser lowers them into concatenations.
| Lists and literal syntax | ✅ | Parser + runtime support; codegen emits runtime handles.
| `map(...)` literals | ✅ | Parser + codegen exist; runtime uses linear search.
| Higher-order list ops (`map`, `reduce`) | ⚠️ | Placeholder syntax now rewrites into lambdas, but runtime lacks closure ABI so `map`/`reduce` remain stubs.
| `type` definitions with methods | ⚠️ | Parsed & semantically checked, but no codegen/lowering.
| `store` / `store actor` | ⚠️ | Parsed; no runtime structs, actor isolation, or message dispatch. `@` functions parsed but runtime ignores actor semantics.
| Namespaces via `!!` taxonomy | ✅ | Parser handles nested `!!` blocks and `!!A:B:C` expressions; currently lowered to string literals.
| Placeholder shorthand (`prices.map($ * 1.15)`) | ✅ | Lexer emits `$` tokens, parser builds AST nodes, new lowering pass wraps arguments in synthesized lambdas.
| Function defaults referencing later params | ✅ | Semantic check prevents invalid references.
| Pattern matching | ⚠️ | Parser + codegen support limited integer patterns; string/bool/list patterns not lowered.
| Method chaining (`object.method()`) | ✅ | Parser + codegen emit runtime member helpers.
| `math.sqrt`, `io.append`, `http.post` | ☐ | Standard library not implemented; calls would resolve at runtime only if manually injected.

## Component Reviews
### Lexer (`src/lexer.rs`)
**Positives**
- Tracks indentation depth with `INDENT/DEDENT` tokens, enforces alignment plus homogeneous tab/space usage, and now recognizes `$` placeholders, template strings, and `!!` taxonomy prefixes.
- Emits final `Newline` and `Eof` tokens for parser simplicity.
- Provides escape handling inside both traditional strings and template fragments.

**Issues & Gaps**
- Still aborts on the first invalid rune; richer diagnostic spans/notes are pending.
- No heredoc or triple-quoted literal support yet.

**Recommendations**
1. Emit line/column caret diagnostics and continue lexing after recoverable errors.
2. Consider lexing heredoc blocks or raw strings for multiline documentation use cases.

### Parser (`src/parser.rs`)
**Positives**
- Handles layout via `consume_indent_with_recovery` + `layout_depth` stack to emit actionable diagnostics.
- Supports functions, bindings, types/stores, lambdas, placeholder expressions, taxonomy nodes, template literals, map/list literals, matches, ternary expressions, and member calls.
- Fixture suite now includes substring expectations plus JSON AST snapshots to prevent regressions.

**Issues & Gaps**
- Only reports the first error; additional errors are dropped after `pending_error` is set.
- Actor message send syntax and store instantiation sugar still need AST coverage.
- `parse_field` treats `?` as both default marker and ternary operator, which can yield confusing spans.

**Recommendations**
1. Accumulate diagnostics instead of bailing on the first error to improve IDE feedback.
2. Extend grammar for actor message sends and store initialization sugar from `syntax.coral`.
3. Add AST-level sugar nodes for store literals so future lowering phases can reason about ownership/mutability.

### Semantics (`src/semantic.rs`)
**Positives**
- Detects duplicate bindings/functions/parameters and invalid default argument references.
- Tracks lexical scopes inside blocks/match arms.
- Works on the lowered AST (post placeholder → lambda), so downstream stages never see raw placeholder nodes.

**Issues & Gaps**
- No type checking or validation that identifiers resolve to known bindings (undefined names still pass through).
- Stores/types aren’t validated for constructor availability, reference cycles, or actor-specific constraints (@ handlers, `&` references).
- No enforcement of return type consistency or immutability (e.g., reassigning parameters in method bodies).

**Recommendations**
1. Introduce symbol resolution to flag undefined variables and to annotate AST nodes with resolved bindings.
2. Add analyses for stores/actors: ensure required fields are provided, references are safe, and actor messages obey restrictions (no shared mutable state).
3. Produce warnings for unused bindings/parameters and unreachable expressions to help users catch bugs early.

### Code Generation (`src/codegen.rs`)
**Positives**
- Uses Inkwell to emit LLVM IR, deferring most operations to runtime helpers (`coral_make_*`, list/map helpers, etc.).
- Supports logical operators, ternaries, match lowering, list/map literals, member dispatch, template strings (via lowered concatenations), and taxonomy literals (as strings).

**Issues & Gaps**
- No lowering for user-defined types/stores/actors: methods are ignored outside `*main`-style free functions.
- Arithmetic is limited to numeric `+`; subtraction/multiplication/division degrade to runtime addition or are unsupported.
- No optimization passes (e.g., constant folding, dead code removal), leaving IR verbose and slower than necessary.

**Recommendations**
1. Map basic arithmetic operators (`-`, `*`, `/`, `%`) to runtime helpers or inline LLVM instructions when operands are numeric.
2. Add a lowering pass for `type`/`store` definitions (struct layouts, constructors, method dispatch tables).
3. Run LLVM optimization pipelines (`PassManager`) in `compile_to_ir` when targeting performance builds.

### Runtime (`runtime/src/lib.rs`)
**Positives**
- Tagged-value representation with ref-counting for heap data.
- Provides helpers for list push/get/length/pop and map get/set/length.
- Includes unit tests for value helpers, lists/maps, and equality.

**Issues & Gaps**
- Map implementation uses linear search (`Vec<MapEntry>`), so lookups are O(n); no hashing or structural equality for composite keys.
- No garbage collection beyond naive ref-counting (risking leaks on cycles) and closures/capture structs are still unimplemented.
- No actor runtime, concurrency model, or IO primitives despite syntax referencing `store actor` and `io/http` modules.

**Recommendations**
1. Replace linear maps with hashed buckets, and expose iterators for list/map to back `map/reduce` semantics.
2. Introduce arena/RC instrumentation to detect leaks and add stress/perf tests (Criterion).
3. Sketch actor runtime (mailboxes, thread pools, effect system) to honor the language goals.

### Tooling & Tests
- `cargo test` covers lexer layout, parser fixtures/snapshots, semantic checks, runtime helpers, and smoke-level codegen tests including the new `tests/fixtures/programs/full_language_no_store.coral` scenario.
- Missing continuous integration tasks, fuzzing, property-based tests, or performance benchmarks.
- No formatting/lint configuration (rustfmt/clippy) enforced.

**Recommendations**
1. Add `cargo fmt --check`/`cargo clippy` to CI plus GitHub Actions.
2. Build fixture-driven semantic suites (positive/negative) and runtime benchmarks.
3. Execute compiled LLVM IR for fixtures (e.g., via `lli`) once runtime side effects are available, not just IR inspections.

## Recent Improvements (This Change)
- Added placeholder → lambda lowering pass (`src/lower.rs`), ensuring call arguments with `$`/`$1` rewrite into synthesized lambdas before semantics/codegen.
- Implemented template-string/token fragment parsing and lowering, so `'Hello, {name}'` compiles into string concatenations.
- Added taxonomy path parsing (`!!Diagnostics:Connection:Timeout`) and ensured they lower into string literals for now.
- Created `tests/fixtures/programs/full_language_no_store.coral` plus a smoke test that compiles it, covering strings, lists, maps, match expressions, taxonomy literals, templated text, and ternaries end-to-end.
- Regenerated documentation (README + overview + this status) to match the new capabilities and highlight the remaining blockers (closures, stores/actors, inference, RC auditing).

## Recommendations to Reach Production Quality
1. **Language Coverage:** Implement the remaining surface constructs (string interpolation, lambdas/placeholders, `!!` taxonomies, actor semantics, store lowering, iterable helpers) so real Coral programs from `syntax.coral` compile and run unchanged.
2. **Semantic Depth:** Add full symbol resolution, type checking, effect/ownership rules, and cross-module linking; report multiple diagnostics per pass.
3. **Runtime & Backend:** Replace linear data structures with high-performance implementations, add actor/mailbox runtime, and emit optimized LLVM IR (including native binaries via `llc`/`clang`).
4. **Tooling:** Establish CI, formatting/linting, fuzz/property tests, and benchmarking harnesses; ship documentation for standard library/runtime APIs.
5. **Developer Experience:** Provide better error messages (spans with context), IDE integration (language server), and package management to improve usability.

This status should be regenerated after each milestone to keep leadership informed of progress toward a production-grade Coral compiler.
