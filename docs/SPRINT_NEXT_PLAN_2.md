# Sprint Plan 2 — Phase Gamma: Ergonomics, Stdlib & Compiler Quality

> **Baseline**: 920 tests passing, 0 failures. Phase Beta complete.
> **Sprint 1 recap**: S5.1-S5.4, S4.1-S4.2, C4.1, T3.5, CC3.1-CC3.3, CC2.5 — all done.
> **Focus**: Syntax ergonomics, stdlib expansion, compiler quality, and runtime fixes.

---

## Quick-Start for New LLM Sessions

Before working on any task below, do ALL of these steps:

1. **Read reference docs** (in this order):
   - `docs/SPRINT_NEXT_PLAN_2.md` — this file (you're here)
   - `AGENTS.md` — build commands, key file locations, critical syntax patterns, helper tools
   - `docs/LLM_ONBOARDING.md` — compiler pipeline, NaN-boxing architecture, test infrastructure, common gotchas

2. **Run the codemap tool** to understand the codebase layout:
   ```
   ./tools/coral-dev codemap compact
   ```

3. **Familiarize yourself with `coral-dev`** — the multi-command helper script:
   ```
   ./tools/coral-dev help
   ```
   Especially useful:
   - `./tools/coral-dev test summary` — see test counts at a glance
   - `./tools/coral-dev test one <name>` — run a single test
   - `./tools/coral-dev test grep <pattern>` — run tests matching a pattern
   - `./tools/coral-dev check` — quick build check
   - `./tools/coral-dev find text <pattern>` — search the codebase
   - `./tools/coral-dev find sym <name>` — find a symbol definition
   - `./tools/coral-dev extract <file> <symbol> [-c N]` — extract a function with context
   - `./tools/coral-dev checklist new-syntax --enrich` — get a checklist for adding new syntax
   - `./tools/coral-dev scaffold e2e <name>` — scaffold an E2E test

4. **Run the full test suite** to confirm baseline:
   ```
   cargo test 2>&1 | tail -5
   ```
   Expected: 920 passed, 0 failed.

5. **Check the roadmap** for overall project direction:
   - `docs/LANGUAGE_EVOLUTION_ROADMAP.md` — authoritative feature roadmap
   - `docs/EVOLUTION_PROGRESS.md` — what's been completed

### Critical Coral Syntax Reminders

- **Binding**: `x is 5` (NEVER `=` or `==`)
- **Function decl**: `*name(params)` (asterisk prefix)
- **Ternary/if**: `condition ? true_branch ! false_branch`
- **Match arms**: use `?` NOT `->` — e.g. `/ Some(x) ? x`
- **Pipeline**: `expr ~ fn(args)`
- **Named args**: `func(name: value)`
- **Default params**: `*func(x, port ? 5432)`
- **No type annotations in user code** — inference only (design principle)
- **Indentation-based scoping** — no braces

---

## Tier 1 — Syntax Ergonomics (Low Complexity, High Impact)

Quick wins that make Coral feel polished. All are pure desugaring or simple additions.

---

### S5.6: Postfix `if`/`unless`

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S5.6
**Complexity**: Low
**Impact**: Natural English-like conditional statements

**Syntax**:
```
log("warning") if debug_mode
return if x > limit
exit(1) unless valid
```

**What exists today**:
- `unless` keyword already in lexer and parser (S5.1 — desugars to `if not`)
- No postfix conditional syntax of any kind

**Implementation plan**:
1. **Parser** (`src/parser.rs`): After parsing a statement (binding, expression-statement, return, etc.), check for trailing `KeywordIf` or `KeywordUnless`. If found, parse the condition and wrap the statement in an `If` node.
2. **No new AST node needed** — the statement becomes the body of an `If`. For `unless`, wrap condition in `UnaryOp(Not, condition)` (same as S5.1).
3. **Semantic/Codegen**: Nothing — it's already an `If` by the time it reaches these phases.

**Key files**: `src/parser.rs`
**Tests**: Parser tests + E2E execution tests. ~4 tests.

---

### S1.5: Augmented Assignment Operators

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S1.5
**Complexity**: Medium
**Impact**: Eliminates `x is x + 1` boilerplate — major ergonomic win for loops and accumulators

**Syntax**:
```
counter += 1
total -= discount
scale *= 2.0
ratio /= count
```

**What exists today**:
- No `+=`, `-=`, `*=`, `/=` tokens in the lexer
- `Statement::Binding` handles `x is expr` rebinding
- The lowering pass (`src/lower.rs`) is the ideal place for desugaring

**Implementation plan**:
1. **Lexer** (`src/lexer.rs`): Add tokens `PlusEquals`, `MinusEquals`, `StarEquals`, `SlashEquals`. Each is a two-character token (`+=`, `-=`, `*=`, `/=`).
2. **Parser** (`src/parser.rs`): When parsing a statement that starts with an identifier followed by one of the augmented-assign tokens, parse as a new `Statement::AugmentedAssign { name, op, value, span }` (or desugar directly in the parser to `Statement::Binding { name, value: BinaryOp(name, op, value) }`).
3. **Preferred approach**: Desugar in the **parser** to `Binding` with `value = BinaryOp(Identifier(name), op, rhs)`. This avoids any changes to lower/semantic/codegen.
4. **FieldAssign variant**: Also support `self.field += 1` by desugaring to `self.field is self.field + 1`.

**Key files**: `src/lexer.rs`, `src/parser.rs`
**Tests**: Parser tests + E2E execution tests. ~6 tests.

---

### T4.4: Return Type Unification Across Branches

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T4.4
**Complexity**: Medium
**Impact**: Catches type mismatches between if/elif/else branches

**What exists today**:
- Match arms already unify their types in the type solver
- `if/elif/else` branches do NOT unify — each branch can return a different type without warning
- The constraint-based solver in `src/types/solver.rs` handles unification

**Implementation plan**:
1. **Semantic** (`src/semantic.rs`): When processing `Expression::If` with both `then` and `else` blocks, generate a unification constraint: `type(then_result) == type(else_result)`. For conditional chains (`elif`), all branches unify.
2. **Type solver**: The existing unification machinery handles this — just need to emit the constraints.
3. **Warning (not error)**: Initially emit a warning when unification fails, not a hard error. This avoids breaking existing code.

**Key files**: `src/semantic.rs`, `src/types/solver.rs`
**Tests**: Semantic warning tests. ~4 tests.

---

## Tier 2 — Standard Library Expansion (Medium Complexity)

These have zero compiler dependencies — purely runtime FFI + Coral wrappers.

---

### L2.1: `std.random` Module

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L2.1
**Complexity**: Medium
**Impact**: Unblocks many example programs and practical applications

**What exists today**: No random functionality at all. No `std/random.coral` file.

**Implementation plan**:
1. **Runtime** (`runtime/src/lib.rs`): Add FFI functions using Rust's built-in random (or a fast PRNG like xoshiro256**):
   - `coral_random() -> f64` — returns 0.0..1.0
   - `coral_random_int(min: i64, max: i64) -> i64` — returns min..=max (NaN-boxed)
   - `coral_random_seed(seed: i64)` — set PRNG seed for reproducibility
2. **Codegen builtins** (`src/codegen/builtins.rs`): Register the FFI functions as known builtins: `random()`, `random_int()`, `random_seed()`.
3. **Std module** (`std/random.coral`): Coral wrappers:
   - `*random()` — 0.0 to 1.0 float
   - `*random_int(min, max)` — integer in range
   - `*random_choice(list)` — random element from list
   - `*shuffle(list)` — Fisher-Yates shuffle, returns new list
   - `*seed(s)` — set seed for reproducibility

**Key files**: `runtime/src/lib.rs`, `src/codegen/builtins.rs`, `std/random.coral`
**Tests**: Runtime unit tests + E2E execution tests. ~6 tests.

---

### L2.3: `std.time` Enhancements (Sleep FFI + Duration)

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L2.3
**Complexity**: Medium
**Impact**: Replace busy-wait `sleep` with proper FFI; enables real-world timing

**What exists today**:
- `std/time.coral` has `now()`, `timestamp()`, `format_iso()`, date/time extraction, `measure()`
- `sleep(ms)` exists but uses a **busy-wait loop** (recursive self-call checking `now()`)
- Runtime has `time_now`, `time_timestamp`, `time_format_iso`, `time_year/month/day/hour/minute/second` FFI

**Implementation plan**:
1. **Runtime** (`runtime/src/lib.rs`): Add `coral_sleep(ms: i64)` using `std::thread::sleep(Duration::from_millis(ms))`. Return unit.
2. **Codegen builtins** (`src/codegen/builtins.rs`): Register `coral_sleep` as builtin.
3. **Std module** (`std/time.coral`): Replace the busy-wait `sleep` with a call to the FFI `coral_sleep()`. Add:
   - `*elapsed(start)` — shorthand for `now() - start`
   - `*duration(amount, unit)` — convert to milliseconds: `duration(5, "seconds")` → `5000`

**Key files**: `runtime/src/lib.rs`, `src/codegen/builtins.rs`, `std/time.coral`
**Tests**: Runtime unit test for sleep + E2E test. ~4 tests.

---

### L2.6: `std.testing` Enhancements

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L2.6
**Complexity**: Medium
**Impact**: Better test infrastructure for the language itself and user programs

**What exists today**:
- `std/testing.coral` (67 lines): `assert()`, `assert_eq()`, `assert_ne()`, `assert_truthy()`, `assert_err()`, `assert_ok()`, `run_test()`
- No test suite runner, no `assert_close` for floats, no `assert_contains`

**Implementation plan**:
1. **Pure Coral additions** to `std/testing.coral`:
   - `*assert_close(actual, expected, tolerance, label)` — for float comparison within epsilon
   - `*assert_contains(collection, item, label)` — checks if list contains item
   - `*assert_starts_with(text, prefix, label)` — string prefix check
   - `*suite(name, tests)` — takes a list of `[name, fn]` pairs, runs all, reports summary
   - `*before_each(setup_fn, tests)` — wraps each test with setup
2. **No runtime changes needed** — all implementable in pure Coral.

**Key files**: `std/testing.coral`
**Tests**: E2E test exercising the new assertions. ~4 tests.

---

## Tier 3 — Compiler Quality (Medium Complexity)

Leverage existing infrastructure to improve code generation and diagnostics.

---

### C4.2: LLVM Function Attributes

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` C4.2
**Complexity**: Medium
**Impact**: Enables LLVM's optimizer to work much harder on our IR

**What exists today**:
- Purity analysis from C1.3 classifies functions as Pure/ReadOnly/Effectful
- Small function inlining (C3.1) already uses `alwaysinline` attribute
- No `nounwind`, `readnone`, `readonly`, or `willreturn` attributes emitted

**Implementation plan**:
1. **Codegen** (`src/codegen/mod.rs`): After building each LLVM function, check the semantic model's purity classification:
   - Pure functions → `readnone`, `nounwind`, `willreturn`
   - ReadOnly functions → `readonly`, `nounwind`
   - All non-panicking functions → `nounwind`
2. **Runtime FFI declarations**: Mark known-pure runtime functions (e.g., `coral_value_add`, `coral_make_number`) with appropriate attributes at declaration time.
3. Use inkwell's `add_attribute()` API on `FunctionValue`.

**Key files**: `src/codegen/mod.rs`, `src/semantic.rs` (purity data)
**Tests**: IR verification tests — check emitted IR contains expected attributes. ~4 tests.

---

### CC2.4: Warning Categories

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` CC2.4
**Complexity**: Medium
**Impact**: Lets users suppress specific warnings; reduces noise

**What exists today**:
- Warnings are collected as `Vec<String>` during semantic analysis
- Diagnostic struct has a `severity: Severity` field (Error, Warning)
- Dead code detection (T3.5) emits "dead_code" warnings
- No warning categorization or suppression mechanism

**Implementation plan**:
1. **Diagnostics** (`src/diagnostics.rs`): Add a `WarningCategory` enum: `UnusedVariable`, `DeadCode`, `ShadowedBinding`, `TypeMismatchBranch`, `UnreachableCode`.
2. **Diagnostic struct**: Add `category: Option<WarningCategory>` field.
3. **CLI** (`src/main.rs`): Add `--warn` / `--allow` flags: `--allow dead_code` suppresses dead-code warnings.
4. **Filtering**: In the main pipeline, filter out suppressed categories before printing.

**Key files**: `src/diagnostics.rs`, `src/semantic.rs`, `src/main.rs`
**Tests**: CLI tests + semantic tests confirming correct categorization. ~4 tests.

---

### T3.2: Definite Assignment Analysis

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T3.2
**Complexity**: Medium
**Impact**: Catches use of potentially uninitialized variables — common bug class

**What exists today**:
- Scope checking in semantic analysis tracks variable declarations
- No flow-sensitive analysis for whether a variable is assigned on all paths

**Implementation plan**:
1. **Semantic** (`src/semantic.rs`): Add a pass (or extend the existing scope walk) that tracks definite-assignment state per variable. After `if` without `else`, variables bound inside the `if` are NOT definitely assigned. After `if/else` where both branches bind the same name, it IS.
2. **Warning**: Emit warning when a variable is used that might not be assigned on all paths leading to the use.
3. **Scope**: Start with function-level analysis (not interprocedural).

**Key files**: `src/semantic.rs`
**Tests**: Semantic warning tests. ~5 tests.

---

## Tier 4 — Expressiveness & Runtime Fixes (Medium-High Complexity)

Fill gaps that real-world programs hit.

---

### S4.3: Multi-Line Lambda Syntax

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S4.3
**Complexity**: Medium
**Impact**: Lambdas currently limited to single expressions; can't have local bindings or multi-step logic

**Syntax**:
```
callback is (x) ->
  validated is validate(x)
  transform(validated)
```

**What exists today**:
- Lambda syntax `(params) -> expr` parses single-expression bodies
- Parser's lambda handling is in `parse_lambda()` or similar
- Closures codegen in `src/codegen/closures.rs` handles single-expression bodies

**Implementation plan**:
1. **Parser** (`src/parser.rs`): After consuming `->`, check if the next token is a newline+indent. If so, parse a full block (reuse `parse_block()`). Otherwise parse a single expression (current behavior).
2. **AST**: Lambda body is already `Block`; just populate it with multiple statements.
3. **Codegen** (`src/codegen/closures.rs`): Ensure multi-statement lambda bodies are emitted correctly. The last statement's expression value is the lambda's return.
4. **Semantic**: Walk lambda body statements normally.

**Key files**: `src/parser.rs`, `src/codegen/closures.rs`
**Tests**: Parser tests + E2E execution tests. ~5 tests.

---

### S4.6: Return Expressions in Lambdas

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S4.6
**Complexity**: Medium
**Impact**: Currently explicitly blocked; real programs need early return from lambdas

**What exists today**:
- `src/codegen/closures.rs` explicitly rejects `return` inside lambdas
- Semantically, `return` in a lambda should return from the lambda, not the enclosing function

**Implementation plan**:
1. **Codegen** (`src/codegen/closures.rs`): Remove the restriction. When emitting a `Return` inside a lambda body, emit a `ret` instruction for the lambda's own function (not the parent).
2. **Parser/Semantic**: Ensure `return` inside lambdas is parsed and validated. It returns the lambda's value.
3. **Depends on S4.3**: Multi-line lambdas make `return` much more useful.

**Key files**: `src/codegen/closures.rs`, `src/parser.rs`
**Tests**: E2E tests. ~3 tests.

---

### M3.4: Closure Cycle Tracking

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` M3.4
**Complexity**: Medium
**Impact**: Fixes silent memory leaks when closures capture references that form cycles

**What exists today**:
- The cycle detector in `runtime/src/lib.rs` tracks containers (lists, maps, stores)
- Closures are NOT tracked as containers even though they can capture and hold references
- `is_container()` check excludes closures

**Implementation plan**:
1. **Runtime** (`runtime/src/lib.rs`): Add closures to `is_container()` check.
2. **Implement `get_children()` for closures**: Return the captured environment values so the cycle detector can trace through them.
3. **Test**: Create a closure↔value cycle and verify it's collected.

**Key files**: `runtime/src/lib.rs`
**Tests**: Runtime unit test for closure cycle collection. ~2 tests.

---

### CC5.3: Fix Example Programs

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` CC5.3
**Complexity**: Medium
**Impact**: All 7 examples compiling is a completeness milestone

**What exists today**:
- `examples/hello.coral` ✅
- `examples/fizzbuzz.coral` ✅
- `examples/calculator.coral` ✅
- `examples/traits_demo.coral` ✅
- `examples/data_pipeline.coral` ✅
- `examples/chat_server.coral` ❌ (likely indent/dedent or feature issues)
- `examples/http_server.coral` ❌ (likely indent/dedent or feature issues)

**Implementation plan**:
1. **Diagnose**: Attempt to compile each failing example with `cargo run -- --emit-ir /dev/null examples/<file>.coral` and capture the error.
2. **Fix**: Adjust the example code (not the compiler) to use valid Coral syntax. If a missing feature is needed, either implement it or simplify the example.
3. **Goal**: All 7 examples compile without errors.

**Key files**: `examples/*.coral`
**Tests**: Add an integration test that compiles all examples. ~1 test.

---

## Implementation Order (Recommended)

| Order | Item | Est. Effort | Cumulative Tests |
|-------|------|-------------|-----------------|
| 1 | S5.6 (postfix `if`/`unless`) | ~45 min | ~924 |
| 2 | S1.5 (augmented assignment `+=` `-=` `*=` `/=`) | ~60 min | ~930 |
| 3 | T4.4 (branch type unification) | ~90 min | ~934 |
| 4 | L2.1 (`std.random`) | ~90 min | ~940 |
| 5 | L2.3 (`std.time` sleep FFI + duration) | ~90 min | ~944 |
| 6 | L2.6 (`std.testing` enhancements) | ~2 hours | ~948 |
| 7 | C4.2 (LLVM function attributes) | ~2 hours | ~952 |
| 8 | CC2.4 (warning categories) | ~2 hours | ~956 |
| 9 | T3.2 (definite assignment analysis) | ~2-3 hours | ~961 |
| 10 | S4.3 (multi-line lambdas) | ~3 hours | ~966 |
| 11 | S4.6 (return in lambdas) | ~2 hours | ~969 |
| 12 | M3.4 (closure cycle tracking) | ~90 min | ~971 |
| 13 | CC5.3 (fix example programs) | ~2 hours | ~973 |

---

## Workflow Reminders

- **After each feature**: Run `cargo test 2>&1 | tail -5` to confirm no regressions
- **Update AGENTS.md baseline** after each commit (test count)
- **Update `docs/EVOLUTION_PROGRESS.md`** to mark items complete
- **Commit frequently** — one commit per feature with descriptive message
- **Use coral-dev helpers** to scaffold tests and verify:
  ```
  ./tools/coral-dev scaffold e2e <test_name>
  ./tools/coral-dev test one <test_name>
  ./tools/coral-dev checklist new-syntax --enrich
  ```
- **For new keywords/tokens** (S5.6, S1.5): Also update `self_hosted/lexer.coral`, `tree-sitter-coral/grammar.js`, `vscode-coral/` if relevant
- **For runtime FFI** (L2.1, L2.3, M3.4): Also run `cargo test -p runtime` to test the runtime crate independently
- **For stdlib changes** (L2.1, L2.3, L2.6): Test by writing a `.coral` file that uses the new functions and running with `--jit`
