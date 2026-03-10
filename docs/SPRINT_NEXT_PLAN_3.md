# Sprint Plan 3 — Phase Gamma: Type Quality, Runtime GC & Stdlib Expansion

> **Baseline**: 971 tests passing, 0 failures. Sprint 2 complete.
> **Sprint 2 recap**: S5.6, S1.5, T4.4, L2.1, L2.3, L2.6, C4.2, CC2.4, T3.2, S4.3, S4.6, M3.4 — all done (CC5.3 skipped).
> **Focus**: Type system error quality, generational cycle detection, stdlib I/O & process expansion, extension methods.

---

## Quick-Start for New LLM Sessions

Before working on any task below, do ALL of these steps:

1. **Read reference docs** (in this order):
   - `docs/SPRINT_NEXT_PLAN_3.md` — this file (you're here)
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
   Expected: 971 passed, 0 failed.

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
- **Augmented assign**: `x += 1`, `x -= 1`, `x *= 2`, `x /= 2`
- **Postfix conditionals**: `log("debug") if verbose`, `return unless valid`
- **No type annotations in user code** — inference only (design principle)
- **Indentation-based scoping** — no braces

---

## Tier 1 — Type System Quality (Medium Complexity, High Impact)

Improve the type solver's error reporting and heuristics. These lay the foundation for flow-sensitive typing (T3.x) in future sprints.

---

### T4.1: Multi-Error Recovery in Type Solving

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T4.1
**Complexity**: Medium
**Impact**: Reports ALL type errors in a compilation, not just the first one

**What exists today**:
- The solver's `solve_constraints()` in `src/types/solver.rs` already iterates all constraints and collects `Vec<TypeError>` — it does NOT stop at the first error
- However, `unify()` (line ~383) returns `Result<(), TypeError>` and uses `?` to short-circuit on the first inner mismatch (e.g., within ADT fields, function param lists)
- `solve_callable()` (line ~310) collects inner errors but discards all but the first: `Err(inner_errors.remove(0))`
- The semantic layer in `src/semantic.rs` (line ~356) joins all error messages into a single string and uses only the first span: `let first_span = errors.first().map(|e| e.span)` — multi-error information is **lost**

**Implementation plan**:
1. **Solver** (`src/types/solver.rs`): Modify `unify()` to accumulate errors instead of early-returning with `?`. For compound types (ADT fields, function params), collect all field mismatches instead of stopping at the first.
2. **Solver** (`src/types/solver.rs`): Fix `solve_callable()` to return all inner errors, not just the first.
3. **Semantic** (`src/semantic.rs`): Instead of collapsing `Vec<TypeError>` into a single `Diagnostic`, emit one `Diagnostic` per `TypeError` — each with its own span and message.
4. **Compiler pipeline**: Ensure the `Vec<Diagnostic>` propagates through `src/compiler.rs` and `src/main.rs` so all errors are displayed.

**Key files**: `src/types/solver.rs`, `src/semantic.rs`, `src/compiler.rs`
**Tests**: Semantic tests with programs containing multiple type errors — verify all are reported. ~4 tests.

---

### T4.2: Better Type Error Messages

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T4.2
**Complexity**: High (scoped to constraint chain context)
**Impact**: When inference fails, show WHY — "x was inferred as Number from line 5, but String is required from the + on line 8"

**What exists today**:
- `TypeError` struct has `message`, `span`, `expected`, `found` fields
- Error messages are generic: "Type mismatch: expected Number, found String"
- No information about the chain of constraints that led to the conflict
- The constraint-based solver knows which expression generated each constraint but doesn't thread that context to errors

**Implementation plan**:
1. **Constraint provenance** (`src/types/solver.rs`): Add an `origin: Option<ConstraintOrigin>` field to `TypeError`. Define `ConstraintOrigin { description: String, span: Span }` to record where a type was first inferred.
2. **Solver tracking**: When `unify()` fails, include both the "expected" origin (where the type was first bound) and the "found" origin (where the conflicting constraint came from).
3. **Enhanced message format**: Instead of `"Type mismatch: expected Number, found String"`, emit:
   ```
   Type mismatch: expected Number, found String
     Number inferred from: line 5, column 3 (x is 42)
     String required by:   line 8, column 7 (x + "hello")
   ```
4. **Type variable history**: Track the first binding site for each type variable in the `TypeGraph.repr` map.
5. **Scope**: Focus on the most common case — binary operations and function calls where argument types conflict. Don't attempt full path reconstruction for complex generic inference.

**Key files**: `src/types/solver.rs`, `src/types/core.rs`, `src/semantic.rs`
**Tests**: Tests that verify error messages include provenance information. ~4 tests.
**Depends on**: T4.1 (multi-error reporting provides the diagnostic pipeline)

---

### T4.3: Ranked Unification

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` T4.3
**Complexity**: Medium
**Impact**: Better root selection in union-find produces more informative error messages when type conflicts surface

**What exists today**:
- The union-find in `TypeGraph` (`src/types/solver.rs`, line ~191) always makes `ra` point to `rb` — no heuristic for choosing the root
- `TypeVarId(pub u32)` is a bare integer with no metadata
- Path compression exists in `find()`
- No rank, weight, or information-level tracking on type variables

**Implementation plan**:
1. **Add rank tracking** (`src/types/solver.rs`): Add `ranks: HashMap<TypeVarId, u32>` to `TypeGraph`. Initialize each new type variable with rank 0.
2. **Union-by-rank**: In `union()`, compare ranks. Attach the lower-rank tree to the higher-rank root. On equal ranks, choose arbitrarily and increment the winner's rank.
3. **Information-aware heuristic**: When both type variables have the same rank, prefer the one that has a concrete `repr` binding (i.e., the one with more type information). This ensures error messages reference the variable that carries actual type data.
4. **Backward compatible**: This is a pure optimization — no semantic changes. Type inference results should be identical; only internal representation changes.

**Key files**: `src/types/solver.rs`
**Tests**: Verify type inference produces identical results (regression tests). Add a test with many type variables to verify no performance degradation. ~3 tests.

---

## Tier 2 — Standard Library Expansion (Medium Complexity)

Fill critical gaps in I/O, process management, and path manipulation. Each requires runtime FFI + Coral wrappers.

---

### L2.4: `std.io` Enhancements

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L2.4
**Complexity**: Medium
**Impact**: Binary I/O, stderr, file metadata — essential for real programs

**What exists today**:
- Runtime FFI in `runtime/src/io_ops.rs`: `coral_fs_read`, `coral_fs_write`, `coral_fs_exists`, `coral_fs_append`, `coral_fs_read_dir`, `coral_fs_mkdir`, `coral_fs_delete`, `coral_fs_is_dir`, `coral_stdin_read_line`
- `std/io.coral` (108 lines): wrappers plus pure path functions (`extension`, `filename`, `dirname`, `path_join`)
- No stderr, no file_size, no binary read with offset, no rename/copy

**Implementation plan**:
1. **Runtime FFI** (`runtime/src/io_ops.rs`): Add new functions:
   - `coral_stderr_write(msg: ValueHandle)` — write to stderr
   - `coral_fs_size(path: ValueHandle) -> ValueHandle` — return file size in bytes as Number
   - `coral_fs_rename(old: ValueHandle, new: ValueHandle) -> ValueHandle` — rename/move file
   - `coral_fs_copy(src: ValueHandle, dst: ValueHandle) -> ValueHandle` — copy file
   - `coral_fs_mkdirs(path: ValueHandle) -> ValueHandle` — recursive mkdir (like `mkdir -p`)
   - `coral_fs_temp_dir() -> ValueHandle` — return temp directory path
2. **Codegen builtins** (`src/codegen/builtins.rs`): Register each as a builtin.
3. **Runtime bindings** (`src/codegen/runtime.rs`): Declare LLVM signatures.
4. **Semantic** (`src/semantic.rs`): Add to `is_builtin_name()`.
5. **Std module** (`std/io.coral`): Add wrappers:
   - `*eprint(msg)`, `*eprintln(msg)` — stderr output
   - `*file_size(path)` — file size in bytes
   - `*rename(old_path, new_path)`, `*copy(src, dst)` — file management
   - `*make_dirs(path)` — recursive directory creation
   - `*temp_dir()` — get temp directory

**Key files**: `runtime/src/io_ops.rs`, `src/codegen/builtins.rs`, `src/codegen/runtime.rs`, `src/semantic.rs`, `std/io.coral`
**Tests**: Runtime tests + E2E execution tests. ~6 tests.

---

### L2.5: `std.process` Enhancements

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L2.5
**Complexity**: Medium
**Impact**: Shell command execution, working directory, PID — enables build tools and scripts

**What exists today**:
- Runtime FFI in `runtime/src/io_ops.rs`: `coral_process_args`, `coral_process_exit`, `coral_env_get`, `coral_env_set`
- `std/process.coral` (52 lines): `argv()`, `argc()`, `arg(i)`, `exit_with()`, `exit_ok()`, `exit_fail()`, `env()`, `env_or()`, `set_env()`
- No command execution, no cwd, no pid

**Implementation plan**:
1. **Runtime FFI** (`runtime/src/io_ops.rs`): Add new functions:
   - `coral_process_exec(cmd: ValueHandle, args: ValueHandle) -> ValueHandle` — run command, return map with `{stdout, stderr, exit_code}`. Uses `std::process::Command`.
   - `coral_process_cwd() -> ValueHandle` — return current working directory as string
   - `coral_process_chdir(path: ValueHandle) -> ValueHandle` — change working directory
   - `coral_process_pid() -> ValueHandle` — return current process ID as Number
   - `coral_process_hostname() -> ValueHandle` — return hostname
2. **Codegen builtins** (`src/codegen/builtins.rs`): Register each.
3. **Runtime bindings** (`src/codegen/runtime.rs`): Declare LLVM signatures.
4. **Semantic** (`src/semantic.rs`): Add to `is_builtin_name()`.
5. **Std module** (`std/process.coral`): Add wrappers:
   - `*exec(cmd, args)` — run command, return `{stdout, stderr, exit_code}` map
   - `*cwd()` — current working directory
   - `*chdir(path)` — change directory
   - `*pid()` — process ID
   - `*hostname()` — machine hostname

**Key files**: `runtime/src/io_ops.rs`, `src/codegen/builtins.rs`, `src/codegen/runtime.rs`, `src/semantic.rs`, `std/process.coral`
**Tests**: Runtime unit tests + E2E execution tests. ~6 tests.

---

### L4.2: `std.path` Module

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` L4.2
**Complexity**: Medium
**Impact**: Dedicated path manipulation — replaces fragile string surgery patterns

**What exists today**:
- `std/io.coral` has `extension(path)`, `filename(path)`, `dirname(path)`, `path_join(a, b)` — all implemented in pure Coral with string operations
- No `normalize`, `resolve`, `relative_to`, `is_absolute`, `parent`, `stem`
- These pure-Coral implementations are correct but don't handle edge cases (trailing slashes, `..` components, Windows paths)

**Implementation plan**:
1. **Runtime FFI** (`runtime/src/io_ops.rs`): Add path operations backed by Rust's `std::path::Path`:
   - `coral_path_normalize(path: ValueHandle) -> ValueHandle` — resolve `.` and `..` lexically
   - `coral_path_resolve(path: ValueHandle) -> ValueHandle` — absolute path via `std::fs::canonicalize`
   - `coral_path_is_absolute(path: ValueHandle) -> ValueHandle` — returns boolean
   - `coral_path_parent(path: ValueHandle) -> ValueHandle` — parent directory
   - `coral_path_stem(path: ValueHandle) -> ValueHandle` — filename without extension
2. **Codegen builtins**: Register each.
3. **New std module** (`std/path.coral`): Comprehensive module:
   - `*join(parts...)` — join path components (variadic via list)
   - `*parent(path)` — parent directory
   - `*filename(path)` — file name component
   - `*stem(path)` — filename without extension
   - `*extension(path)` — file extension
   - `*normalize(path)` — resolve `.` and `..`
   - `*resolve(path)` — absolute canonical path
   - `*is_absolute(path)` — boolean check
   - `*relative_to(path, base)` — relative path computation
   - `*components(path)` — split into list of components

**Key files**: `runtime/src/io_ops.rs`, `src/codegen/builtins.rs`, `std/path.coral`
**Tests**: Runtime tests + E2E tests. ~5 tests.

---

## Tier 3 — Runtime GC Improvement (Medium Complexity)

Upgrade the cycle detector from global-lock + scan-everything to thread-local buffering with generational tracking. These two tasks together form the foundation for M3.3 (incremental collection) in a future sprint.

---

### M3.1: Thread-Local Cycle Root Buffers

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` M3.1
**Complexity**: Medium
**Impact**: Eliminates global mutex contention on every `possible_root()` call

**What exists today**:
- `CycleDetector` in `runtime/src/cycle_detector.rs` is fully global and mutex-protected:
  ```rust
  static CYCLE_DETECTOR: OnceLock<Mutex<CycleDetector>> = OnceLock::new();
  ```
- `possible_root()` (line ~166) takes the global lock to insert into `roots: HashSet<usize>`
- `collect_cycles()` (line ~195) takes the lock, then runs mark → scan → collect phases
- The counter `CYCLE_COLLECTION_COUNTER` triggers collection every 1000 releases

**Implementation plan**:
1. **Thread-local buffer**: Add `thread_local! { static LOCAL_ROOTS: RefCell<Vec<usize>> = ... }` in `cycle_detector.rs`.
2. **Fast-path `possible_root()`**: Push into the thread-local buffer instead of locking the global mutex. This is the hot path — called on every decrement-to-non-zero for containers.
3. **Buffer flush threshold**: When the local buffer reaches a threshold (e.g., 64 entries), flush to the global `CycleDetector.roots`.
4. **Collection trigger**: Before `collect_cycles()` runs, flush ALL thread-local buffers. Use a `flush_all_thread_local_roots()` barrier. Since Coral actors run on a thread pool, the simplest approach is: at collection time, take the global lock and let each thread's next `possible_root()` call detect the "collection pending" flag and flush.
5. **Simpler alternative**: Use `crossbeam-utils::CachePadded<AtomicBool>` as a "collection pending" flag. Threads check it periodically (every N operations) and flush their local buffers when set.
6. **Thread exit cleanup**: Register a thread-local destructor that flushes remaining roots on thread exit.

**Key files**: `runtime/src/cycle_detector.rs`
**Tests**: Multi-threaded test that creates cycles on different threads and verifies all are collected. Runtime-crate test. ~3 tests.

---

### M3.2: Generational Hypothesis / Epoch Tracking

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` M3.2
**Complexity**: Medium
**Impact**: Most cycles are short-lived — scan young roots frequently, old roots rarely

**What exists today**:
- Bacon & Rajan synchronous cycle collection with colors: Black (in use), Purple (possible root), Gray (potential garbage), White (confirmed garbage)
- Three phases: `mark_roots()` → `scan_roots()` → `collect_roots()`
- **No age/generation tracking** — every collection scans ALL roots

**Implementation plan**:
1. **Add epoch to CycleInfo** (`runtime/src/cycle_detector.rs`): Add `birth_epoch: u64` field alongside existing `color` and `buffered` fields.
2. **Global epoch counter**: Add `current_epoch: u64` to `CycleDetector`, incremented at each collection.
3. **Tag roots on insertion**: When `possible_root()` adds a new root, stamp it with `current_epoch`.
4. **Two-generation partition**: Split `roots` into `young_roots: HashSet<usize>` and `old_roots: HashSet<usize>`. Young roots are those added since the last collection.
5. **Promotion policy**: Roots that survive one young-generation collection get moved to `old_roots`. Only scan `old_roots` every K collections (e.g., K=5).
6. **Young-only collection**: A young-gen collection only marks/scans/collects `young_roots`. If a young root references an old root's children, just skip (don't trace into old generation).
7. **Full collection**: Every K-th collection scans both young and old roots.
8. **Counters**: Track `young_collections` and `full_collections` for profiling.

**Key files**: `runtime/src/cycle_detector.rs`
**Tests**: Runtime tests verifying young roots are collected without touching old roots, and that promoted roots are eventually collected. ~3 tests.

---

## Tier 4 — Compiler Optimization (Medium-High Complexity)

Leverage LLVM more effectively with metadata hints.

---

### C4.3: LLVM Alias Analysis Hints

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` C4.3
**Complexity**: Medium-High (scoped to `noalias` on function parameters)
**Impact**: Enables LLVM to reorder and optimize memory operations

**What exists today**:
- Sprint 2 added `apply_function_attributes()` in `src/codegen/mod.rs` (line ~771) — applies `nounwind`, `memory(none)`, `willreturn` based on purity
- Uses `Attribute::get_named_enum_kind_id()` and `context.create_enum_attribute()` inkwell APIs
- `AttributeLoc::Param(idx)` is the inkwell API for per-parameter attributes
- **No `noalias` attributes emitted anywhere**

**Scope for this sprint**: Focus on `noalias` for function parameters. Full TBAA metadata deferred to a future sprint (requires deep inkwell FFI work).

**Implementation plan**:
1. **`noalias` on function parameters** (`src/codegen/mod.rs`): In `apply_function_attributes()`, mark all function parameters as `noalias`. In Coral's NaN-boxed model, function arguments are `i64` values that don't alias by construction (they're passed by value). This is always safe for non-pointer `i64` params.
2. **`noalias` on return values of allocation functions**: Mark runtime allocator functions (`coral_make_string`, `coral_make_list`, `coral_make_map`, `coral_make_closure`) with `noalias` on their return. Freshly allocated objects don't alias any existing pointer.
3. **`nonnull` on heap pointers**: When the runtime returns a heap pointer (list, map, store), it's never null. Mark with `nonnull` where applicable.
4. **`dereferenceable` hints**: For known-size runtime objects, add `dereferenceable(N)` to help LLVM optimize loads.
5. **Apply to runtime declarations**: In `src/codegen/runtime.rs`, when declaring external runtime functions, add the appropriate attributes.

**Key files**: `src/codegen/mod.rs`, `src/codegen/runtime.rs`
**Tests**: IR verification tests checking emitted attributes. ~4 tests.

---

## Tier 5 — Syntax Feature (High Complexity)

A flagship language feature that enables rich library patterns.

---

### S4.5: Extension Methods

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` S4.5
**Complexity**: High
**Impact**: Add methods to existing types without modifying their definition — enables library augmentation of built-in types

**Syntax**:
```
extend String
  *word_count()
    self.split(" ").length()

  *reverse_words()
    self.split(" ") ~ reverse() ~ join(" ")

name is "hello world"
log(name.word_count())       -- 2
log(name.reverse_words())    -- "world hello"
```

**What exists today**:
- Method calls dispatch through `emit_member_call()` in `src/codegen/builtins.rs` (line ~123)
- Built-in methods are a hardcoded `match property` block: `equals`, `map`, `filter`, `length`, etc.
- User-defined type methods use `store_methods: HashMap<String, (String, usize)>` mapping method name → (owner_type, param_count)
- Methods compile as `TypeName_methodName(self, ...)` via name mangling
- `with TraitName` syntax exists for trait implementation on stores/types
- **No `extend` keyword, no way to add methods to existing types**

**Implementation plan**:
1. **Lexer** (`src/lexer.rs`): Add `KeywordExtend` token for the `extend` keyword. (Check if it's already reserved.)
2. **AST** (`src/ast.rs`): Add `Item::Extension { type_name: String, methods: Vec<Function>, span: Span }` variant.
3. **Parser** (`src/parser.rs`): Parse `extend TypeName` followed by an indented block of method definitions (reuse `parse_method()` or equivalent). Methods have implicit `self` parameter.
4. **Semantic** (`src/semantic.rs`): Register extension methods in a new `extension_methods: HashMap<(String, String), FunctionDef>` map (keyed by `(type_name, method_name)`). Validate no conflicts with existing methods.
5. **Codegen** (`src/codegen/mod.rs`): Generate extension methods as `TypeName_methodName(self, ...)` — same name mangling as regular methods. Add them to `store_methods` during the function declaration pass so `emit_member_call()` finds them.
6. **Method resolution order**: Extension methods have lower priority than built-in methods and original type methods. If a built-in type already has `.length()`, an extension can't override it.
7. **Supported types**: Allow extending `String`, `List`, `Map`, `Number`, and user-defined stores/types.
8. **Self-hosted compiler**: Update `self_hosted/parser.coral` to recognize `extend`.
9. **Tree-sitter / VS Code**: Update grammar and syntax highlighting.

**Key files**: `src/lexer.rs`, `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`, `src/codegen/mod.rs`, `src/codegen/builtins.rs`
**Tests**: Parser tests + E2E execution tests for extending both built-in and user types. ~6 tests.

---

## Tier 6 — Cross-Cutting Quality (Medium Complexity)

Bug fixes and infrastructure hardening.

---

### CC3.4: Circular Dependency Enhancement

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` CC3.4
**Complexity**: Medium (enhancement of existing detection)
**Impact**: Better developer experience when circular imports are detected

**What exists today**:
- `src/module_loader.rs` has cycle detection in two places: `load_recursive()` (line ~476) and `collect_modules()` (line ~353)
- Both use a `stack: Vec<PathBuf>` to track the current import chain
- Error: `"circular import detected: a -> b -> a\nHint: Consider restructuring..."` — clear but minimal
- Test at line ~661 validates detection works

**Implementation plan**:
1. **Full dependency graph**: After `collect_modules()` builds the module list, construct an explicit directed graph of imports. Store as `HashMap<PathBuf, Vec<PathBuf>>`.
2. **Cycle visualization**: When a cycle is detected, show the full cycle with module paths and the specific `use` lines that create the dependency: `a.coral:3 uses b → b.coral:7 uses a`.
3. **Suggestion engine**: Analyze the cycle and suggest which `use` to remove or restructure. If module A only uses one function from B and B uses many from A, suggest moving that function.
4. **Multi-cycle detection**: Find ALL cycles in the graph (not just the first one hit during DFS). Report them all.
5. **Scope**: Keep this as hard errors (not warnings). True circular dependency resolution (forward declarations, lazy imports) is deferred.

**Key files**: `src/module_loader.rs`
**Tests**: Tests with various cycle patterns (direct A↔B, triangle A→B→C→A, diamond). ~4 tests.

---

### R3.9: WeakRef Clone Semantics Fix

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` R3.9
**Complexity**: Medium
**Impact**: Fixes use-after-free risk in WeakRef clones — actual safety bug

**What exists today**:
- WeakRef clones share the same registry ID as the original
- If the original WeakRef is freed, the registry entry is removed
- The clone still holds the (now-invalid) registry ID → use-after-free on access

**Implementation plan**:
1. **Investigate**: Examine the WeakRef implementation in `runtime/src/lib.rs` and/or dedicated weak_ref module. Identify how registry IDs are assigned and freed.
2. **Fix**: Each WeakRef clone must register its own independent entry in the weak reference registry. Alternatively, use reference counting on registry entries so they're only freed when ALL WeakRefs (original + clones) are dropped.
3. **Registry entry refcounting**: Add `ref_count: usize` to registry entries. Clone increments it. Drop decrements it. Entry is only removed when ref_count reaches 0.
4. **Test**: Create a WeakRef, clone it, drop the original, verify the clone still works correctly.

**Key files**: `runtime/src/lib.rs` (or `runtime/src/weak_ref.rs` if exists)
**Tests**: Runtime unit tests. ~3 tests.

---

### CC5.2: Fix Remaining Medium Bugs

**Roadmap ref**: `LANGUAGE_EVOLUTION_ROADMAP.md` CC5.2
**Complexity**: Medium
**Impact**: Fixes quality-of-life issues that affect real programs

**What exists today (known medium bugs)**:
- **P6**: Single-error model — the pipeline sometimes stops at one error even when more could be reported (overlaps with T4.1)
- **S6**: Member access fallback — accessing a non-existent member on a typed value falls through to `Unknown` instead of producing a clear error
- **S8**: Pipeline type inference — `expr ~ fn()` pipeline doesn't always infer types correctly

**Implementation plan**:
1. **S6 — Member access error** (`src/semantic.rs`): When a member access is attempted on a value with a known type that doesn't have that member, emit a diagnostic: "Type X has no member 'y'" instead of silently returning `Unknown`.
2. **S8 — Pipeline type inference** (`src/types/solver.rs`): The pipeline `x ~ f()` desugars to `f(x)`. Verify that the type constraint for `f`'s first parameter unifies with `x`'s type. Add specific constraint generation for pipeline expressions in the semantic pass.
3. **Other bugs**: Use `./tools/coral-dev test failures` and manual testing to identify and fix additional issues.

**Key files**: `src/semantic.rs`, `src/types/solver.rs`
**Tests**: Regression tests for each bug fixed. ~4 tests.

---

## Implementation Order (Recommended)

| Order | Item | Est. Effort | Cumulative Tests |
|-------|------|-------------|-----------------|
| 1 | T4.3 (ranked unification) | ~45 min | ~974 |
| 2 | T4.1 (multi-error recovery in solver) | ~90 min | ~978 |
| 3 | T4.2 (better type error messages) | ~2 hours | ~982 |
| 4 | L2.4 (`std.io` enhancements) | ~2 hours | ~988 |
| 5 | L2.5 (`std.process` enhancements) | ~2 hours | ~994 |
| 6 | L4.2 (`std.path` module) | ~90 min | ~999 |
| 7 | C4.3 (LLVM alias analysis hints) | ~2 hours | ~1003 |
| 8 | M3.1 (thread-local cycle root buffers) | ~2 hours | ~1006 |
| 9 | M3.2 (generational hypothesis) | ~2 hours | ~1009 |
| 10 | CC3.4 (circular dependency enhancement) | ~90 min | ~1013 |
| 11 | R3.9 (WeakRef clone fix) | ~90 min | ~1016 |
| 12 | CC5.2 (fix medium bugs) | ~2 hours | ~1020 |
| 13 | S4.5 (extension methods) | ~3 hours | ~1026 |

**Target**: 971 → ~1026 tests (~55 new tests)

---

## Rationale & Dependencies

### Why this ordering?

1. **T4.3 → T4.1 → T4.2** chain naturally: ranked unification improves the solver, multi-error recovery expands what it reports, better messages make reports useful. Each builds on the previous.
2. **L2.4 → L2.5 → L4.2** share the same FFI pattern (runtime function → codegen builtin → std wrapper). Once the pattern is flowing for `io`, `process` and `path` follow quickly.
3. **C4.3** is standalone — add LLVM attributes, verify with IR tests.
4. **M3.1 → M3.2** are tightly coupled: thread-local buffers change how roots are stored, generational tracking partitions them by age. Both modify `cycle_detector.rs`.
5. **CC3.4, R3.9, CC5.2** are independent fixes that can slot in at any point.
6. **S4.5** (extension methods) is last because it's the most complex end-to-end feature and benefits from a stable codebase.

### What this unblocks for Sprint 4

- **T3.1 (type narrowing)**: Requires the improved type solver from T4.1-T4.3
- **M3.3 (incremental collection)**: Requires the generational infrastructure from M3.1-M3.2
- **T3.3 (nullability tracking)**: Benefits from multi-error reporting
- **L3.x (networking)**: Benefits from the I/O and process infrastructure
- **S4.4 (method chaining fluency)**: Benefits from extension methods establishing the method dispatch infrastructure
- **CC5.3 (example programs)**: More stdlib coverage makes examples compilable

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
- **For new keywords/tokens** (S4.5): Also update `self_hosted/lexer.coral`, `tree-sitter-coral/grammar.js`, `vscode-coral/` if relevant
- **For runtime FFI** (L2.4, L2.5, L4.2, M3.1, M3.2, R3.9): Also run `cargo test -p runtime` to test the runtime crate independently
- **For stdlib changes** (L2.4, L2.5, L4.2): Test by writing a `.coral` file that uses the new functions and running with `--jit`
- **For type system changes** (T4.1, T4.2, T4.3): Run `./tools/coral-dev test grep type` to verify all type-related tests still pass
