# Sprint Plan 4 — Phase Gamma/Delta: Type Safety, Method Dispatch & Actor Foundation

> **Baseline**: 1016 tests passing, 0 failures. Sprint 3 complete.
> **Sprint 3 recap**: T4.1-T4.3, L2.4, L2.5, L4.2, C4.3, M3.1, M3.2, CC3.4, R3.9, CC5.2, S4.5 — all 13 done.
> **Focus**: Flow-sensitive typing, type-aware method dispatch (KI-1 fix), actor system hardening, incremental GC, regex stdlib, examples compilation.

---

## Quick-Start for New LLM Sessions

Before working on any task below, do ALL of these steps:

1. **Read reference docs** (in this order):
   - `docs/SPRINT_NEXT_PLAN_4.md` — this file (you're here)
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
   - `./tools/coral-dev scaffold e2e <test_name>` — scaffold an E2E test

4. **Run the full test suite** to confirm baseline:
   ```
   cargo test 2>&1 | tail -5
   ```
   Expected: 1016 passed, 0 failed.

5. **Check the roadmap** for overall project direction:
   - `docs/LANGUAGE_EVOLUTION_ROADMAP.md` — authoritative feature roadmap
   - `docs/EVOLUTION_PROGRESS.md` — what's been completed

### Critical Coral Syntax Reminders

- **Binding**: `x is 5` (NEVER `=` or `==`)
- **Function decl**: `*name(params)` (asterisk prefix)
- **Ternary/if**: `condition ? true_branch ! false_branch`
- **Match arms**: use `->` for the arrow — e.g., `/ Some(x) -> x`
- **Guards**: `/ x when x > 0 -> positive`
- **Or-patterns**: `/ A or B -> body`
- **Pipeline**: `expr ~ fn(args)`
- **Named args**: `func(name: value)`
- **Default params**: `*func(x, port ? 5432)`
- **Augmented assign**: `x += 1`, `x -= 1`, `x *= 2`, `x /= 2`
- **Postfix conditionals**: `log("debug") if verbose`, `return unless valid`
- **Extension methods**: `extend TypeName` + indented `*method()` blocks
- **No type annotations in user code** — inference only (design principle)
- **Indentation-based scoping** — no braces

---

## Tier 1 — Type-Aware Method Dispatch (High Impact, Medium Complexity)

Fix the built-in method name shadowing issue discovered in Sprint 3. This is a correctness bug that affects real programs.

---

### KI-1: Type-Aware Method Dispatch in `emit_member_call()`

**Roadmap ref**: Known Issue KI-1 in `LANGUAGE_EVOLUTION_ROADMAP.md`
**Complexity**: Medium-High
**Impact**: Fixes method name collisions between user-defined store methods and built-in methods (`set`, `get`, `push`, `pop`, `map`, `filter`, `reduce`, `length`, `keys`, `equals`, etc.)

**The Bug**:
In `src/codegen/builtins.rs`, `emit_member_call()` uses a hardcoded `match property` block that dispatches ALL method calls by name — regardless of the target's type. If a user defines `store Counter` with a method `*get()`, calling `c.get()` dispatches to the built-in `list/map.get(index)` which expects 1 argument, and fails with "map.get expects exactly one argument."

The ~20 shadowed names: `equals`, `not_equals`, `not`, `iter`, `keys`, `map`, `filter`, `reduce`, `push`, `pop`, `get`, `set`, `at`, `length`, `read`, `write`, `exists`, `log`, `concat`, `size`, `err`.

**What exists today**:
- `store_methods: HashMap<String, (String, usize)>` maps method name → (store_name, param_count)
- Built-in methods dispatch first (hardcoded `match`), `store_methods` is only checked in the `_ =>` fallthrough
- `resolved_types: HashMap<String, TypeId>` tracks type-inferred variable types (added in Sprint session 18 for numeric specialization)
- Semantic analysis already produces `TypeId::Store(name)` for store instances (T1.2, Sprint session 17)

**Implementation plan**:
1. **Identify target type** (`src/codegen/builtins.rs`): At the top of `emit_member_call()`, before the `match property` block, check if the target expression is an identifier with a resolved store type. Use `resolved_types` to look up the variable's type.
2. **Store-type priority**: If the target has `TypeId::Store(name)` and `store_methods` contains `(name, property)`, dispatch to the store method FIRST — skip the built-in match entirely.
3. **Fallback to built-ins**: If the target is not a known store instance (or the store doesn't have that method), proceed to the existing `match property` built-in dispatch as before.
4. **Type annotation propagation**: Ensure `resolved_types` is populated for variables bound via `make_StoreName()` calls, `for`-loop iteration variables, function parameters, and match bindings. The existing infrastructure from C2.1 should cover most cases, but verify coverage.
5. **Extension method integration**: This fix naturally benefits extension methods too — extension methods are already in `store_methods` and the store-type check will prefer them over built-ins.

**Key files**: `src/codegen/builtins.rs`, `src/codegen/mod.rs`
**Tests**: ~5 tests:
- Store method `get()` on a store variable doesn't clash with built-in `list.get()`
- Store method `set()` with 1 arg works (built-in `map.set()` expects 2)
- Extension method `length()` on custom store overrides built-in `.length()`
- Built-in `.length()` still works on string/list variables
- Method dispatch respects store type across assignment chains

---

## Tier 2 — Flow-Sensitive Typing (High Complexity, High Impact)

These tasks build on the improved type solver from Sprint 3 (T4.1-T4.3). They transform the type system from "permissive but useless" to "catches real bugs."

---

### T3.1: Type Narrowing in Conditionals

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T3.1
**Complexity**: High
**Impact**: After `if x.is_err`, the else branch knows `x` is the success type. After `match x / Some(v) ->`, `v` carries the inner type.

**What exists today**:
- `TypeId` has variants: `Store(String)`, `Adt(String, Vec<TypeId>)`, `Float`, `Int`, `Bool`, `String`, `List`, `Map`, `Any`, `Unknown`
- Match pattern codegen binds variables but without type information — bindings are just `IntValue` in the `FunctionContext.variables` map
- The `resolved_types` HashMap tracks variable→TypeId but is not modified by control flow
- `collect_constraints_expr()` in `src/semantic.rs` generates type constraints but doesn't account for control flow narrowing

**Implementation plan**:
1. **Narrowing context** (`src/semantic.rs`): Add a `type_narrowings: Vec<HashMap<String, TypeId>>` stack to the constraint collection pass. Push a new scope at conditional boundaries.
2. **Pattern narrowing**: In `match` arms, when a pattern like `Some(v)` binds `v`, record `v: T` where `T` is the inner type of the matched variant. This requires the solver to know the ADT structure.
3. **Conditional narrowing**: For `if x.is_err` / `if x.is_none`, narrow `x` to the success/some type in the else branch. Requires recognizing these as type-test expressions.
4. **Propagate to codegen**: Feed narrowed types into `resolved_types` so codegen can use them for specialization. Scoped narrowings expire at the end of their block.
5. **Scope**: Focus on `match` arms (highest value, clearest semantics). Defer `if`-based narrowing of error/none to a subsequent sprint if the match narrowing proves complex.

**Key files**: `src/semantic.rs`, `src/types/solver.rs`, `src/types/core.rs`
**Tests**: ~5 tests:
- Match arm binds variable with narrowed type (e.g., `Some(v)` → v is inner type)
- Nested pattern narrowing (`Ok(Some(v))` → v is doubly unwrapped)
- Or-pattern with consistent bindings preserves narrowed types
- Guard condition doesn't incorrectly narrow
- Narrowed type doesn't leak out of match arm scope

---

### T3.3: Nullability Tracking

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T3.3
**Complexity**: High (scoped to diagnostics only — no mandatory Option everywhere)
**Impact**: Functions returning `none` on some paths get warned about implicit nullability

**What exists today**:
- `None` has TypeId `TypeId::Unknown` in many contexts (it unifies with everything)
- `Option` is a stdlib type alias mapped to `List` (not a proper ADT in the type system)
- No tracking of which functions might return `none`
- No `Option[T]` type-level distinction

**Implementation plan**:
1. **`None` type identity** (`src/types/core.rs`): Add `TypeId::None` variant (or `TypeId::Option(Box<TypeId>)`) so `None` doesn't silently unify with everything.
2. **Return path analysis** (`src/semantic.rs`): After solving, scan all function return types. If a function has both `none` and `T` return paths, the effective return type is `Option[T]`. Emit a warning: "function 'lookup' may return none — consider returning Option[T]".
3. **Gradual enforcement**: Initially emit warnings only (not errors). This lets existing code continue working while surfacing nullability bugs.
4. **Scope**: Focus on function return types (highest signal). Defer variable-level nullability tracking to a later sprint.

**Key files**: `src/types/core.rs`, `src/types/solver.rs`, `src/semantic.rs`
**Tests**: ~4 tests:
- Function with `none` return path emits nullability warning
- Function that only returns `T` (no none) — no warning
- Match on Option-like type with Some/None arms — no false positive
- Explicitly returning `none` as documented behavior — warning is informational

**Depends on**: T3.1 (type narrowing provides the match-arm infrastructure)

---

## Tier 3 — Runtime: Incremental GC & Actor Foundation (Medium-High Complexity)

Build on the generational infrastructure from Sprint 3 and start the large actor system block.

---

### M3.3: Incremental Cycle Collection

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` M3.3
**Complexity**: High
**Impact**: Eliminates stop-the-world GC pauses by interleaving collection work with allocation

**What exists today**:
- Sprint 3 added thread-local root buffers (M3.1) and generational partitioning (M3.2)
- Collection is still synchronous: `collect_cycles()` runs all three phases (mark → scan → collect) in one call
- Collection triggers every 1000 container releases via `CYCLE_COLLECTION_COUNTER`
- The `CycleDetector` holds all roots in `young_roots` and `old_roots` HashSets

**Implementation plan**:
1. **Phased state machine** (`runtime/src/cycle_detector.rs`): Replace the monolithic `collect_cycles()` with a state machine: `Idle → Marking(progress) → Scanning(progress) → Collecting → Idle`. Each state processes a bounded number of roots (e.g., 16 per step).
2. **Incremental trigger**: On every `possible_root()` or `release()` call, advance the state machine by one step instead of doing a full collection at the threshold.
3. **Write barrier**: When the GC is in Marking or Scanning state and a value is mutated (list push, map set), mark the mutated container as needing re-scan. This prevents missed cycles from concurrent mutation.
4. **Concurrent safety**: Since Coral actors run on multiple threads, the incremental GC must be safe with the thread-local buffer merging from M3.1. Each thread progresses independently on its local roots, with global phases synchronized via atomic state flags.
5. **Fallback full GC**: Retain the synchronous `collect_cycles()` as an explicit `gc.collect()` builtin for programs that want deterministic collection.

**Key files**: `runtime/src/cycle_detector.rs`
**Tests**: ~4 tests:
- Incremental collection finds and frees a simple cycle
- Write barrier during marking doesn't miss mutated containers
- Incremental GC processes across multiple `possible_root()` calls
- Full GC still works as a fallback

---

### R2.6: Complete Supervision Restart

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` R2.6
**Complexity**: Medium
**Impact**: Supervision can actually restart failed actors — currently restart decisions are made but not executed

**What exists today**:
- `spawn_supervised_child` in the actor runtime uses `FnOnce` for the actor factory
- Supervision tree detects failure and decides to restart, but the factory can only be called once
- The restart path logs "restarting actor" but cannot re-create the actor

**Implementation plan**:
1. **`Arc<dyn Fn>` factory** (`runtime/src/lib.rs` or `runtime/src/actor.rs`): Change `spawn_supervised_child` from `FnOnce` to `Arc<dyn Fn() -> ActorState + Send + Sync>`. The factory is now callable multiple times.
2. **Restart implementation**: On actor failure, the supervisor calls the factory again to create a fresh actor state, re-registers the actor with the same name/address, and resumes message processing.
3. **Restart budget**: Add a `max_restarts: u32` and `restart_window: Duration` to supervisor configuration. If restarts exceed the budget within the window, escalate to the parent supervisor (or terminate).
4. **Mailbox preservation**: On restart, the actor's mailbox is preserved — unprocessed messages are delivered to the new actor instance. This matches Erlang semantics.

**Key files**: `runtime/src/lib.rs` (actor runtime)
**Tests**: ~4 tests:
- Supervised actor restarts after panic/failure
- Restart budget enforcement (max_restarts exceeded → escalation)
- Restarted actor receives pending messages
- Non-supervised actor failure doesn't trigger restart

---

### R2.10: Graceful Actor Stop

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` R2.10
**Complexity**: Medium
**Impact**: Actors cleanly shut down by processing remaining messages before termination

**What exists today**:
- Actor termination is abrupt — mailbox contents are discarded
- No `stop()` message or graceful shutdown protocol
- Active actors are tracked in a global registry

**Implementation plan**:
1. **Stop signal**: Add a special `Stop` message variant (internal, not user-visible) that signals the actor to finish processing its current message and terminate.
2. **Drain mode**: When an actor receives `Stop`, it enters drain mode: process all messages currently in the mailbox, then terminate. New messages arriving after `Stop` are rejected (sender gets an error).
3. **`actor_stop(ref)` FFI**: Expose a runtime function that sends the `Stop` signal.
4. **Coral wrapper**: Add `*stop(actor)` function in the actor support layer.
5. **Termination callback**: Optionally allow actors to define a `*on_stop()` handler for cleanup.

**Key files**: `runtime/src/lib.rs` (actor runtime)
**Tests**: ~3 tests:
- Stopped actor processes remaining mailbox before termination
- Messages sent after stop are rejected
- Stopped actor is removed from registry

---

## Tier 4 — Standard Library: Regex & Examples (Medium Complexity)

Fill a critical stdlib gap and validate all example programs compile.

---

### L2.2: `std.regex` Module

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L2.2
**Complexity**: Medium-High
**Impact**: Pattern matching on strings — enables parsing, validation, text processing. High-value gap.

**What exists today**:
- No regex support at all — string manipulation is limited to `split()`, `starts_with()`, `contains()`, etc.
- The `runtime` crate already depends on system libraries; adding `regex` crate is straightforward
- No `str_find_all`, `str_replace_regex`, or `str_match` builtins

**Implementation plan**:
1. **Runtime FFI** (`runtime/src/string_ops.rs` or new `runtime/src/regex_ops.rs`):
   - `coral_regex_match(pattern: ValueHandle, text: ValueHandle) -> ValueHandle` — returns Bool (full match)
   - `coral_regex_find(pattern: ValueHandle, text: ValueHandle) -> ValueHandle` — returns first match as String (or none)
   - `coral_regex_find_all(pattern: ValueHandle, text: ValueHandle) -> ValueHandle` — returns List of matched strings
   - `coral_regex_replace(pattern: ValueHandle, replacement: ValueHandle, text: ValueHandle) -> ValueHandle` — returns String
   - `coral_regex_split(pattern: ValueHandle, text: ValueHandle) -> ValueHandle` — returns List of strings
   - Use Rust's `regex` crate under the hood. Compile patterns lazily (cache compiled regex for repeated use).
2. **Cargo dependency**: Add `regex = "1"` to `runtime/Cargo.toml`.
3. **Codegen builtins** (`src/codegen/builtins.rs`): Register each as a builtin.
4. **Runtime bindings** (`src/codegen/runtime.rs`): Declare LLVM signatures.
5. **Semantic** (`src/semantic.rs`): Add to `is_builtin_name()`.
6. **Std module** (`std/regex.coral`):
   ```coral
   *matches(pattern, text)
       return regex_match(pattern, text)
   *find(pattern, text)
       return regex_find(pattern, text)
   *find_all(pattern, text)
       return regex_find_all(pattern, text)
   *replace(pattern, replacement, text)
       return regex_replace(pattern, replacement, text)
   *split(pattern, text)
       return regex_split(pattern, text)
   ```

**Key files**: `runtime/Cargo.toml`, `runtime/src/regex_ops.rs` (new), `src/codegen/builtins.rs`, `src/codegen/runtime.rs`, `src/semantic.rs`, `std/regex.coral`
**Tests**: ~5 tests:
- `regex_match` returns true/false for pattern matching
- `regex_find` returns first match
- `regex_find_all` returns all matches as a list
- `regex_replace` substitutes matches
- `regex_split` splits on pattern

---

### CC5.3: All Examples Compile and Run

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` CC5.3
**Complexity**: Medium (diagnosis-heavy — fixes may be trivial or may reveal deeper issues)
**Impact**: Demonstrates language completeness. Validates stdlib coverage. Great for documentation.

**What exists today**:
- 7 example programs in `examples/`:
  - `hello.coral` — likely works (basic)
  - `fizzbuzz.coral` — likely works (basic control flow)
  - `calculator.coral` — may need stdin improvements
  - `data_pipeline.coral` — needs pipeline `~` + functional methods
  - `traits_demo.coral` — needs trait/store methods
  - `chat_server.coral` — needs actor system + networking
  - `http_server.coral` — needs TCP + HTTP parsing
- Some examples may use syntax that has evolved since they were written
- Examples that depend on unimplemented stdlib features (HTTP, chat) may need to be simplified or deferred

**Implementation plan**:
1. **Audit**: Try compiling each example with `cargo run -- --jit examples/NAME.coral`. Record the error for each.
2. **Fix simple issues**: Syntax mismatches (old `?` match syntax vs new `->`, missing `make_` constructor prefix, etc.) are quick fixes.
3. **Fix stdlib gaps**: If an example needs a missing builtin, add it if small, or simplify the example if the feature is out-of-scope.
4. **Defer complex examples**: `chat_server.coral` and `http_server.coral` require networking that isn't implemented yet. Mark them as `# Requires: std.http` with a note.
5. **Add compilation tests**: For each working example, add a test that compiles it successfully.

**Key files**: `examples/*.coral`, various compiler/stdlib files for fixes
**Tests**: ~5 tests (one per compilable example)
**Scope**: Target 5/7 examples compiling (defer chat_server and http_server until L3.1 HTTP is done)

---

## Tier 5 — Method Chaining & S5.5 (Medium Complexity)

Quality-of-life syntax improvements that leverage Sprint 3's infrastructure.

---

### S4.4: Method Chaining Fluency

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S4.4
**Complexity**: Medium
**Impact**: `string.trim().lower().split(" ").map($.capitalize()).join(" ")` works as a single fluent expression

**What exists today**:
- Method calls are dispatched through `emit_member_call()` in `src/codegen/builtins.rs`
- Chained calls parse correctly (left-to-right member access)
- Type information is lost between chain links — each method returns `i64` (NaN-boxed), so the next call doesn't know the target type
- Built-in methods like `.split()` return a List, but the type system doesn't track this through the chain

**Implementation plan**:
1. **Chain type tracking** (`src/codegen/builtins.rs`): After each built-in method call, record the return type in a local mapping. E.g., after `x.split(" ")`, the return value has type `List`. When the next `.map(f)` is called on it, the dispatch knows the target is a List.
2. **Return type registry**: Build a static mapping of `(input_type, method_name) → return_type` for built-in methods. E.g., `(String, "split") → List`, `(List, "map") → List`, `(List, "join") → String`.
3. **Scope**: Focus on the built-in methods that are commonly chained: `split`, `join`, `map`, `filter`, `trim`, `lower`, `upper`, `push`, `pop`, `length`. Don't attempt general return-type inference.
4. **Store method chains**: Store methods already return NaN-boxed values and dispatch via `store_methods` — chaining works as long as each method returns `self` or a store instance. No changes needed for basic store chains; type tracking for store-to-different-type chains is deferred.

**Key files**: `src/codegen/builtins.rs`, `src/codegen/mod.rs`
**Tests**: ~4 tests:
- `"hello world".split(" ").length()` returns 2
- `list.map($ * 2).filter($ > 3).length()` chains functional methods
- `string.trim().lower()` chains string methods
- Store method chain `b.append("x").result()` works

---

### S5.5: `do..end` Block Syntax

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S5.5
**Complexity**: Medium
**Impact**: Alternative block delimiters for DSL-style syntax and multi-line arguments

**Syntax**:
```coral
describe "math" do
    it "adds numbers" do
        assert_eq(1 + 1, 2)
    end
end
```

**What exists today**:
- All blocks use indentation-based scoping (INDENT/DEDENT tokens from the lexer)
- No `do`/`end` keywords
- Multi-line block arguments to functions are awkward with indentation only

**Implementation plan**:
1. **Lexer** (`src/lexer.rs`): Add `KeywordDo` and `KeywordEnd` tokens.
2. **Parser** (`src/parser.rs`): After parsing a function call's arguments, check for `do` keyword. If present, parse a block until `end` token. The block becomes the last argument (as a lambda).
3. **Desugaring**: `func(args) do ... end` desugars to `func(args, *() -> ...)` — the do-block becomes a zero-argument lambda.
4. **Scope**: Initial implementation supports `do..end` as an alternative to indentation for block arguments only (not for function bodies or control flow). This keeps the change contained.
5. **Self-hosted**: Update `self_hosted/lexer.coral` with new keywords.

**Key files**: `src/lexer.rs`, `src/parser.rs`, `src/lower.rs`
**Tests**: ~4 tests:
- `func(arg) do ... end` parses as func(arg, lambda)
- Nested `do..end` blocks work
- `do..end` block can contain multiple statements
- Mismatch of `do` without `end` produces clear error

---

## Tier 6 — Compilation Infrastructure (Medium Complexity)

Improve compilation speed and link-time optimization.

---

### CC3.5: Incremental Compilation

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` CC3.5
**Complexity**: High (scoped to module-level caching)
**Impact**: Only recompile changed modules — dramatic speedup for large projects

**What exists today**:
- `ModuleLoader` resolves and loads all modules on every compilation
- Each module is independently parsed (CC3.1), but semantic analysis and codegen process the merged `Program`
- No caching of parsed ASTs or compiled LLVM IR between runs

**Implementation plan**:
1. **Module fingerprinting**: Hash each module's source content. Store alongside the compiled artifact.
2. **Artifact cache**: After compiling a module to LLVM IR (or its semantic analysis result), write it to a cache directory (`.coral-cache/`). Key by source hash.
3. **Cache hit**: On recompilation, check if the module's source hash matches the cached artifact. If so, load the cached result instead of re-parsing/re-analyzing.
4. **Invalidation**: When a module changes, invalidate its cache AND the caches of all modules that import it (transitive invalidation). Use the dependency graph from CC3.1.
5. **Scope**: Cache at the parsed-AST level first (cheapest to implement, eliminates the parsing cost). IR-level caching (eliminates codegen) is a follow-up.

**Key files**: `src/module_loader.rs`, `src/compiler.rs`
**Tests**: ~3 tests:
- Recompilation with unchanged modules is faster (timing test)
- Changed module invalidates dependents
- Cache miss falls back to full compilation

---

### C4.4: Link-Time Optimization (LTO)

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` C4.4
**Complexity**: Medium
**Impact**: LLVM can inline runtime functions into Coral code, eliminating call overhead for hot paths

**What exists today**:
- Runtime library (`libruntime.so`) is compiled separately and linked at load time
- Runtime functions like `coral_nb_add`, `coral_nb_retain`, `coral_nb_release` are called via external function declarations
- LLVM cannot optimize across the Coral IR ↔ runtime boundary

**Implementation plan**:
1. **Emit runtime as LLVM bitcode**: Compile `runtime/src/` to LLVM bitcode (`.bc`) using `rustc --emit=llvm-bc` or build a separate bitcode compilation step.
2. **Link bitcode at compile time**: In `src/compiler.rs`, when compiling in `-O2` or higher, link the runtime bitcode into the Coral module using LLVM's `link_in_module`.
3. **LTO pass**: Run LLVM's optimization passes on the combined module. This allows inlining `coral_nb_add` (which is a small function) directly into Coral code.
4. **Scope**: Focus on the most called runtime functions: `coral_nb_add`, `coral_nb_sub`, `coral_nb_mul`, `coral_nb_equals`, `coral_nb_retain`, `coral_nb_release`. Full runtime LTO is a future optimization.

**Key files**: `src/compiler.rs`, `src/main.rs`, `runtime/Cargo.toml`
**Tests**: ~3 tests:
- LTO build produces a working binary
- LTO build is smaller than non-LTO
- Performance benchmark shows improvement for numeric loops

---

## Implementation Order (Recommended)

| Order | Item | Est. Effort | Est. Tests | Cumulative |
|-------|------|-------------|-----------|------------|
| 1 | KI-1 (type-aware method dispatch) | ~2 hours | ~5 | ~1021 |
| 2 | CC5.3 (examples compile) | ~2 hours | ~5 | ~1026 |
| 3 | L2.2 (`std.regex`) | ~2 hours | ~5 | ~1031 |
| 4 | T3.1 (type narrowing in conditionals) | ~3 hours | ~5 | ~1036 |
| 5 | T3.3 (nullability tracking) | ~2 hours | ~4 | ~1040 |
| 6 | S4.4 (method chaining fluency) | ~2 hours | ~4 | ~1044 |
| 7 | M3.3 (incremental cycle collection) | ~3 hours | ~4 | ~1048 |
| 8 | R2.6 (supervision restart) | ~2 hours | ~4 | ~1052 |
| 9 | R2.10 (graceful actor stop) | ~2 hours | ~3 | ~1055 |
| 10 | S5.5 (`do..end` blocks) | ~2 hours | ~4 | ~1059 |
| 11 | CC3.5 (incremental compilation) | ~3 hours | ~3 | ~1062 |
| 12 | C4.4 (LTO) | ~2 hours | ~3 | ~1065 |

**Target**: 1016 → ~1065 tests (~49 new tests)

---

## Rationale & Dependencies

### Why this ordering?

1. **KI-1 first** — it's a correctness bug discovered in Sprint 3. Fixing it unblocks natural method naming patterns and removes a footgun for every store/extension method definition. Quick win, high impact.
2. **CC5.3 early** — auditing examples reveals real-world compilation issues, serves as integration test for the entire compiler, and generates confidence in language completeness.
3. **L2.2 (regex)** — highest-value stdlib gap. Unblocks string processing, input validation, and parsing use cases. Required for many real programs.
4. **T3.1 → T3.3** — type narrowing provides the infrastructure for nullability tracking. Both rely on the improved solver from Sprint 3 (T4.1-T4.3). Together they transform the type system from "catches some bugs" to "catches most bugs."
5. **S4.4** (method chaining) — builds on KI-1 fix and extension methods. Provides the fluency promised by the language's design.
6. **M3.3** — builds directly on M3.1 (thread-local buffers) and M3.2 (generational partitioning) from Sprint 3. Completes the GC overhaul.
7. **R2.6, R2.10** — two manageable actor system tasks that provide concrete functionality. Full actor system overhaul (R2.1-R2.12) is too large for one sprint; these two provide the most user-visible value.
8. **S5.5, CC3.5, C4.4** — nice-to-have infrastructure items that round out the sprint. Any of these can be deferred if higher-priority items take longer than expected.

### What this unblocks for Sprint 5

- **T3.4 (Error type tracking)**: Builds on T3.1/T3.3 type narrowing infrastructure
- **R2.1 (Work-stealing scheduler)**: Foundation from R2.6/R2.10 actor improvements
- **R2.7 (Typed messages)**: Builds on T3.1 type narrowing for message type checking
- **L3.1 (std.http)**: Regex + examples provide the usage patterns
- **C4.5 (PGO)**: LTO infrastructure provides the build pipeline
- **CC4.1 (WASM target)**: Incremental compilation reduces iteration time

### What is deferred

- **R2.1-R2.5, R2.7-R2.9, R2.11-R2.12** — Full actor system overhaul is 12 tasks. Sprint 4 takes the most impactful two (R2.6, R2.10). The remaining 10 need a dedicated actor sprint.
- **M3.5 (Weak ref optimization)** — Lower priority than M3.3.
- **M4.x (Escape analysis)** — Very High complexity, needs dedicated sprint.
- **C5.x (Advanced comptime)** — Very High complexity, needs dedicated sprint.
- **R3.1-R3.8 (Store engine)** — Large block, deferred to a dedicated store sprint.
- **CC4.x (Compilation targets)** — WASM, macOS, Windows deferred until core is more stable.
- **T2.5 (Monomorphization)** — Deferred since Beta (Very High complexity).
- **C2.4-C2.5 (Unboxed lists, store field specialization)** — Deferred since Beta.

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
- **For new keywords/tokens** (S5.5): Also update `self_hosted/lexer.coral`, `tree-sitter-coral/grammar.js`, `vscode-coral/` if relevant
- **For runtime FFI** (L2.2, M3.3, R2.6, R2.10): Also run `cargo test -p runtime` to test the runtime crate independently
- **For stdlib changes** (L2.2): Test by writing a `.coral` file that uses the new functions and running with `--jit`
- **For type system changes** (T3.1, T3.3): Run `./tools/coral-dev test grep type` to verify all type-related tests still pass
- **For codegen changes** (KI-1, S4.4, C4.4): Run `./tools/coral-dev test grep codegen` plus `./tools/coral-dev test grep extend` to verify method dispatch
- **KI-1 method name list** (for reference): `equals`, `not_equals`, `not`, `iter`, `keys`, `map`, `filter`, `reduce`, `push`, `pop`, `get`, `set`, `at`, `length`, `read`, `write`, `exists`, `log`, `concat`, `size`, `err`
