# Sprint Plan — Post-Beta Gamma Batch

> **Baseline**: 865 tests passing, 0 failures. Commit `5a6b532`.
> **Phase Beta**: ~90% complete. Remaining Beta items (T2.5, C2.4, C2.5) deferred due to "Very High" complexity.

---

## Quick-Start for New LLM Sessions

Before working on any task below, do ALL of these steps:

1. **Read reference docs** (in this order):
   - `docs/SPRINT_NEXT_PLAN.md` — this file (you're here)
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
   Expected: 865 passed, 0 failed.

5. **Check the roadmap** for overall project direction:
   - `docs/LANGUAGE_EVOLUTION_ROADMAP.md` — authoritative feature roadmap
   - `docs/EVOLUTION_PROGRESS.md` — what's been completed

### Critical Coral Syntax Reminders

- **Binding**: `x is 5` (NEVER `=` or `==`)
- **Function decl**: `*name(params)` (asterisk prefix)
- **Ternary/if**: `condition ? true_branch ! false_branch`
- **Match arms**: use `?` NOT `->` — e.g. `/ Some(x) ? x`
- **Pipeline**: `expr ~ fn(args)`
- **No type annotations in user code** — inference only (design principle)
- **Indentation-based scoping** — no braces

---

## Tier 1 — Immediate High-Impact (Low-Medium Complexity)

These items provide maximum user-visible improvement for minimal implementation effort.

---

### C4.1: Optimization Level Flags

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 187
**Complexity**: Medium
**Impact**: Users can trade compile speed for runtime performance

**What exists today**:
- `src/main.rs` uses `clap::Parser` with `Args` struct (line 20)
- CLI flags: `--emit-ir`, `--emit-binary`, `--jit`, `--runtime-lib`, `--lli`, `--llc`, `--clang`, `--collect-metrics`
- No `-O` flags at all
- `--emit-binary` shells out to `llc` and `clang` — the opt level must be passed to those tools
- JIT execution uses `lli` — opt level applies there too

**Implementation plan**:
1. **Add CLI flag** in `src/main.rs` `Args` struct:
   ```rust
   #[arg(short = 'O', value_name = "LEVEL", default_value = "0")]
   opt_level: u8,  // 0, 1, 2, 3
   ```
2. **Pass to `llc`**: Add `-O{level}` to the `llc` command invocation (~line 130-160 in main.rs)
3. **Pass to `clang`**: Add `-O{level}` to the `clang` link step
4. **Pass to `lli`**: Add `-O{level}` flag for JIT mode
5. **Default behavior**: `--jit` defaults to `-O0` (fast iteration), `--emit-binary` defaults to `-O2`

**Key files**: `src/main.rs`
**Tests**: Add E2E tests in `tests/pipeline.rs` or a new `tests/optimization.rs` confirming flag is accepted and binaries run correctly.

---

### S4.2: Default Parameter Values

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 248
**Complexity**: Medium
**Impact**: Eliminates boilerplate for functions with optional arguments

**What exists today**:
- **Parser**: `parse_parameters_impl()` at `src/parser.rs` ~line 792 already parses `*foo(x, port ? 5432)` and stores `Parameter.default: Option<Expression>`
- **AST**: `Parameter` struct in `src/ast.rs` line 127 has `pub default: Option<Expression>`
- **Semantic**: Unknown — needs checking whether defaults are validated
- **Codegen**: Does NOT handle defaults. `src/codegen/mod.rs` ~line 1180 just zips LLVM params with AST params and stores them. No conditional logic for missing args.
- **Closures** (`src/codegen/closures.rs`): Same pattern — iterates `params` without default handling

**Implementation plan**:
1. **Semantic check**: Ensure default expressions are type-compatible with the parameter. Default params must come after non-default params (parser may already enforce this — verify).
2. **Codegen — call site**: When emitting a function call, if fewer arguments are provided than the function expects, fill remaining args by emitting the default expressions. This requires:
   - Knowing the callee's parameter list at each call site
   - Emitting the default expression's codegen inline at the call site
3. **Alternative approach** (simpler): Generate a prologue in the function body that checks if an argument is a sentinel value (e.g., NaN-boxed None) and replaces it with the default. Call sites pass `None` for omitted args.
4. **Closures**: Ensure lambda default params work too

**Key files**: `src/codegen/mod.rs` (function body builder, call emission), `src/codegen/closures.rs`, `src/semantic.rs`
**Tests**: Parser tests already exist. Add codegen/execution tests confirming defaults are applied when args are omitted.

---

### S5.4: `when` Expression

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 261
**Complexity**: Medium
**Impact**: Clean multi-branch conditionals without needing a match target

**Syntax**:
```
result is when
  x > 100 ? "high"
  x > 50  ? "medium"
  _       ? "low"
```

**What exists today**: Nothing — `when` is not a keyword, not in the lexer, parser, AST, or codegen.

**Implementation plan**:
1. **Lexer** (`src/lexer.rs`): Add `KeywordWhen` token to the keyword map (~line 80-120 where other keywords are registered)
2. **AST** (`src/ast.rs`): Add `When` variant to `Expression`:
   ```
   When { arms: Vec<(Expression, Block)>, default: Option<Box<Block>>, span: Span }
   ```
   Each arm is (condition_expr, body_block). The `_` arm is the default.
3. **Parser** (`src/parser.rs`): Add `parse_when_expression()`. Pattern:
   - Consume `KeywordWhen`
   - Expect indented block
   - Each line: `condition ? body` (reuse existing conditional/ternary parsing)
   - `_ ? body` for the default arm
4. **Semantic** (`src/semantic.rs`): Walk arms, check condition is bool, infer body types
5. **Codegen** (`src/codegen/mod.rs`): Desugar to chained if/elif/else. Emit conditional branches for each arm in sequence; emit default block as the final else.

**Key files**: `src/lexer.rs`, `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`, `src/codegen/mod.rs`
**Tests**: Parser tests + E2E execution tests

---

### S5.1: `unless` Keyword

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 258
**Complexity**: Low
**Impact**: Cleaner negative conditionals

**Syntax**: `unless ready ? return` (equivalent to `not ready ? return`)

**Implementation plan**:
1. **Lexer**: Add `KeywordUnless` token
2. **Parser**: When `KeywordUnless` is encountered, parse as `If` with the condition wrapped in `Expression::UnaryOp(Not, condition)`. Pure desugaring — no new AST node needed.
3. **Semantic/Codegen**: Nothing — it's already an `If` by the time it reaches these phases.

**Key files**: `src/lexer.rs`, `src/parser.rs`
**Tests**: Parser tests, E2E execution tests

---

### S5.2: `until` Loop

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 259
**Complexity**: Low
**Impact**: Cleaner negative-condition loops

**Syntax**: `until done ? ...body...` (equivalent to `while not done`)

**Implementation plan**:
1. **Lexer**: Add `KeywordUntil` token
2. **Parser**: When `KeywordUntil` is encountered, parse as `While` with condition wrapped in `Expression::UnaryOp(Not, condition)`. Pure desugaring.
3. **Semantic/Codegen**: Nothing — it's already a `While`.

**Key files**: `src/lexer.rs`, `src/parser.rs`
**Tests**: Parser tests, E2E execution tests

---

### S5.3: `loop` Keyword for Infinite Loops

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 260
**Complexity**: Low
**Impact**: Communicates infinite loop intent clearly

**Syntax**:
```
loop
  ...body...
  ready ? break
```

**Implementation plan**:
1. **Lexer**: Add `KeywordLoop` token
2. **Parser**: When `KeywordLoop` is encountered, parse body block, emit `While` with condition `Expression::BoolLiteral(true)`. Pure desugaring.
3. **Semantic/Codegen**: Nothing — it's already a `While(true, body)`.

**Key files**: `src/lexer.rs`, `src/parser.rs`
**Tests**: Parser tests, E2E execution tests

---

## Tier 2 — Foundation Building (Medium-High Complexity)

These items strengthen the compiler's correctness and enable future features.

---

### CC3.1-CC3.3: Module System Overhaul

**Roadmap refs**: `LANGUAGE_EVOLUTION_ROADMAP.md` lines 423-425
**Complexity**: Very High (CC3.1), High (CC3.2), Medium (CC3.3)
**Impact**: Proper namespacing eliminates name collisions; selective imports keep scope clean

**What exists today**:
- `src/module_loader.rs` (~500 lines): Text-based `use` expansion. `load()` reads entry file, extracts `use <module>` directives, recursively loads dependencies, concatenates all source text into a single string before parsing.
- `ModuleInfo` struct tracks `path`, `source`, `dependencies`, `imports`
- Circular import detection works via a stack-based DFS
- No namespacing — all symbols from imported modules are injected into global scope
- No selective imports — `use std.math` imports everything from math.coral

**Implementation plan (phased)**:

**CC3.1 — AST-level module system**:
1. Change `ModuleLoader` to parse each module into its own AST
2. Add `Module` wrapper to AST: `Module { name: String, items: Vec<TopLevel>, exports: Vec<String> }`
3. Semantic analysis processes modules in dependency order, building per-module symbol tables
4. Codegen receives a `Vec<Module>` instead of a single flat AST

**CC3.2 — Namespacing**:
1. After `use std.io`, symbols accessed as `io.read()` not `read()`
2. Add `QualifiedName` AST node for dotted access on module names
3. Resolve qualified names in semantic analysis by looking up the module's symbol table
4. Codegen maps qualified names to the correct LLVM function

**CC3.3 — Selective imports**:
1. Parse `use std.math.{sin, cos, pi}` syntax
2. Only the named symbols are imported into the current module's scope
3. Unselected symbols still accessible via qualified name (`math.tan()`)

**Key files**: `src/module_loader.rs`, `src/parser.rs`, `src/ast.rs`, `src/semantic.rs`, `src/codegen/mod.rs`
**Tests**: Module-level integration tests in `tests/modules.rs`

> **Note**: This is the highest-complexity item in the plan. Consider doing CC3.1 first as standalone, then CC3.2+CC3.3 together.

---

### L1.5: `list.pop()` Runtime FFI

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 360
**Complexity**: Low
**Impact**: O(1) pop replaces O(n) copy patterns used 5+ times in self-hosted compiler

**What exists today**:
- `runtime/src/lib.rs` has `coral_list_*` FFI functions (push, get, set, len, etc.)
- No `coral_list_pop` function

**Implementation plan**:
1. **Runtime** (`runtime/src/lib.rs`): Add `#[no_mangle] pub extern "C" fn coral_list_pop(list: i64) -> i64` that removes and returns the last element. Return NaN-boxed None if empty.
2. **Codegen builtins** (`src/codegen/builtins.rs`): Register `coral_list_pop` as a known builtin, wire `.pop()` method calls to it.
3. **Runtime tests**: Add tests in `runtime/src/lib.rs` mod tests

**Key files**: `runtime/src/lib.rs`, `src/codegen/builtins.rs`
**Tests**: Runtime unit tests + E2E coral test

---

### L1.2: Fix `unwrap` to Panic

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 357
**Complexity**: Low
**Impact**: Correctness fix — unwrap on None/Err must crash, not just log

**What exists today**:
- `std/option.coral` and `std/result.coral` define `unwrap` — currently they log but don't exit
- Runtime has `coral_exit` or similar (needs verification)

**Implementation plan**:
1. **Check** what `unwrap` currently does in `std/option.coral` and `std/result.coral`
2. **Fix** to call a runtime panic function or `exit(1)` on failure
3. If no panic/exit runtime function exists, add `coral_panic(msg)` to `runtime/src/lib.rs`

**Key files**: `std/option.coral`, `std/result.coral`, `runtime/src/lib.rs`
**Tests**: E2E test that confirms unwrap on None exits with non-zero code

---

### T3.5: Dead Code Detection

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 134
**Complexity**: Low
**Impact**: Warns on statements after return/break/err — catches common bugs

**What exists today**:
- Semantic analysis in `src/semantic.rs` (~3,056 lines) walks statements but doesn't track reachability
- Warning infrastructure exists (CC2.2 warnings are already collected)

**Implementation plan**:
1. **Semantic** (`src/semantic.rs`): In the block-walking function, track when a `Return`, `Break`, or unconditional `Err` return is encountered. Any statements after such a terminator emit a warning.
2. **Warning emission**: Use the existing warning collection mechanism — push to the `warnings: Vec<String>` that semantic already maintains.
3. **No codegen change** — dead code is still compiled (it's a warning, not an error).

**Key files**: `src/semantic.rs`
**Tests**: Semantic tests confirming warnings are emitted for unreachable code

---

## Tier 3 — Adoption & Ergonomics (High Complexity)

These items are critical for real-world usage but require significant design work.

---

### CC2.5: LSP Protocol Implementation

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 417
**Complexity**: Very High
**Impact**: Editor integration — the gateway to adoption

**What exists today**:
- `vscode-coral/` directory with a TextMate grammar for syntax highlighting
- No LSP server
- The compiler exposes `Compiler::compile_to_ir_with_warnings()` as its main API

**Implementation plan**:
1. **New crate**: Create `coral-lsp/` as a Cargo workspace member
2. **Dependencies**: Use the `tower-lsp` or `lsp-server` crate
3. **Diagnostics**: On file save/change, run the compiler pipeline up to semantic analysis, collect errors/warnings, publish as LSP diagnostics
4. **Go-to-definition**: Requires building a symbol table with source locations (semantic phase already tracks spans)
5. **Hover types**: Requires type information from the solver (expose `TypeId → human-readable` mapping)
6. **Auto-complete**: Symbol enumeration from the current scope

> **Note**: Start with diagnostics-only (step 3) as an MVP. The other features can follow incrementally.

**Key files**: New `coral-lsp/` crate, `src/lib.rs` (compiler API), `vscode-coral/` (client extension)

---

### S4.1: Named Arguments

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` line 247
**Complexity**: High
**Impact**: Dramatically improves readability for multi-parameter functions

**Syntax**: `connect(host: "db.local", port: 5432, timeout: 30)`

**What exists today**:
- Function calls parse positional arguments only
- No `name:` syntax in argument lists

**Implementation plan**:
1. **AST**: Add `NamedArg { name: String, value: Expression }` variant or a `name: Option<String>` field on call arguments
2. **Parser**: In argument list parsing, detect `identifier : expression` pattern. Must distinguish from ternary `?` — the `:` is unambiguous.
   - Actually, Coral uses `?` not `:` for ternary. The colon is available for named args.
3. **Semantic**: At each call site, resolve named args to positional indices using the callee's parameter list. Error if name doesn't match any parameter. Error if mixed named/positional is ambiguous.
4. **Codegen**: By the time codegen runs, named args have been reordered to positional — no codegen changes needed if semantic does the rewriting.

**Key files**: `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`
**Tests**: Parser tests, semantic error tests, E2E execution tests

---

## Implementation Order (Recommended)

| Order | Item | Est. Effort | Cumulative Tests |
|-------|------|-------------|-----------------|
| 1 | S5.1 (`unless`) | ~30 min | ~868 |
| 2 | S5.2 (`until`) | ~30 min | ~871 |
| 3 | S5.3 (`loop`) | ~30 min | ~874 |
| 4 | S5.4 (`when`) | ~90 min | ~878 |
| 5 | C4.1 (optimization flags) | ~60 min | ~881 |
| 6 | S4.2 (default params) | ~90 min | ~885 |
| 7 | L1.5 (`list.pop()`) | ~45 min | ~888 |
| 8 | L1.2 (fix `unwrap`) | ~30 min | ~890 |
| 9 | T3.5 (dead code detection) | ~60 min | ~894 |
| 10 | CC3.1 (AST-level modules) | ~4 hours | ~900+ |
| 11 | CC3.2-CC3.3 (namespacing + selective imports) | ~3 hours | ~910+ |
| 12 | S4.1 (named arguments) | ~2 hours | ~916+ |
| 13 | CC2.5 (LSP MVP — diagnostics only) | ~4 hours | ~920+ |

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
- **For new keywords** (S5.1-S5.4): Run `./tools/coral-dev checklist new-syntax --enrich` to get the full checklist of files to touch
- **For runtime FFI** (L1.5): Also run `cargo test -p runtime` to test the runtime crate independently
