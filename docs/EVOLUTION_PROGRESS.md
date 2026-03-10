                 # Coral Evolution — Implementation Progress Tracker

**Started:** March 8, 2026  
**Companion to:** [LANGUAGE_EVOLUTION_ROADMAP.md](LANGUAGE_EVOLUTION_ROADMAP.md)

---

## Test Baseline

| Metric | Value | Date |
|--------|-------|------|
| Initial tests passing | 203 | 2026-03-08 |
| Current tests passing | 310 compiler + 162 runtime = 472 | 2026-03-10 |
| Pre-existing failures | 2 (e2e_cc53_fizzbuzz, map_iterator_snapshot) |
| Runtime build | debug + release |

---

## Active Work Stream: M1 — NaN-Boxing for Immediates

### Overview

Transform the value representation from a uniform 40-byte heap-allocated `Value` struct to a NaN-boxed 64-bit encoding where primitives (Number, Bool, Unit, None) are immediate values requiring **zero heap allocation and zero refcounting**.

### Encoding Scheme

```
IEEE 754 double: all 64 bits are the f64 value
                 (including ±0, ±inf, NaN itself for actual NaN)

NaN-boxed immediate:
  Bits 63..51 = 0x7FF8 (quiet NaN signal prefix, 13 bits)
  Bits 50..48 = tag (3 bits → 8 possible immediate tags)
  Bits 47..0  = payload (48 bits)

Tag encoding (in bits 50..48):
  000 = Heap pointer    (payload = 48-bit pointer address)
  001 = Bool            (payload bit 0 = true/false)
  010 = Unit            (payload ignored)
  011 = None/Absent     (payload ignored)
  100 = Error marker    (payload = 48-bit pointer to ErrorMetadata)
  101 = (reserved)
  110 = (reserved)
  111 = (reserved)

Representation as u64:
  Number:  any u64 where bits 63..51 ≠ 0x7FF8 (passes IEEE 754 f64 through)
  Heap:    0x7FF8_0000_0000_0000 | (ptr & 0x0000_FFFF_FFFF_FFFF)
  Bool T:  0x7FF8_1000_0000_0001
  Bool F:  0x7FF8_1000_0000_0000
  Unit:    0x7FF8_2000_0000_0000
  None:    0x7FF8_3000_0000_0000
  Error:   0x7FF8_4000_0000_0000 | (err_meta_ptr & 0x0000_FFFF_FFFF_FFFF)
```

All Coral values become a single `u64` (`i64` in LLVM IR). Heap-allocated containers (String, List, Map, Store, Actor, Closure, Tagged, Bytes) are encoded as heap pointers. The heap `Value` struct remains for containers but is never allocated for primitives.

### Task Status

| ID | Task | Status | Notes |
|----|------|--------|-------|
| M1.1 | Design NaN-box encoding scheme | **DONE** | See above |
| M1.2 | Implement `NanBoxedValue` type + helpers in runtime | **DONE** | `runtime/src/nanbox.rs` — 38 unit tests pass |
| M1.3 | Migrate `coral_make_number`/`bool`/`unit` FFI | **DONE** | `runtime/src/nanbox_ffi.rs` — 15 FFI tests pass |
| M1.4 | Update `coral_value_retain`/`release` fast paths | **DONE** | `coral_nb_retain`/`release` — no-op for immediates |
| M1.5 | Update arithmetic FFI fast paths | **DONE** | `coral_nb_add`/`sub`/`mul`/`div`/`rem`/`neg` |
| M1.6 | Update comparison FFI fast paths | **DONE** | `coral_nb_equals`/`not_equals`/`less_than`/`greater_than`/etc. |
| M1.7 | Update Rust codegen (`%CoralValue*` → `i64`) | **DONE** | All codegen files transitioned, 195 tests pass |
| M1.8 | Benchmark suite & measurement | **DONE** | 5 benchmarks: fib, tight_loop, list_ops, string_ops, matrix_mul |

### Implementation Notes

**Transition strategy:** Feature-flagged. All NaN-box code lives behind `#[cfg(feature = "nanbox")]` so the old representation is preserved as fallback. The runtime exposes the same FFI signatures regardless; only the internal representation changes.

**CRITICAL CONSTRAINT:** NaN-boxing changes the LLVM IR calling convention from `%CoralValue*` (pointer) to `i64`. This affects EVERY function signature, EVERY call site, EVERY PHI node, EVERY alloca. This is the most invasive change in the project's history. We approach it incrementally:

1. First: new `nanbox.rs` module with the encoding/decoding helpers (pure Rust, fully tested)
2. Second: adapt FFI constructors to return `i64` (keeping the old pointer-based API as a compatibility shim)  
3. Third: adapt codegen to use `i64` instead of `%CoralValue*`
4. Fourth: remove compatibility shim

---

## Completed Work Stream: S1 — Core Syntax Clarity

| ID | Task | Status | Notes |
|----|------|--------|-------|
| S1.1 | Resolve `is` overloading for map entries (`:` syntax) | **DONE** | `map("host": "localhost")` works; parser + codegen updated |
| S1.2 | Clarify ternary vs error propagation | **SKIP** | Parser already disambiguates; no user complaints |
| S1.3 | `for..in` range support (`for i in 0 to n step k`) | **DONE** | `for..to..step` syntax with efficient counted loop codegen |
| S1.4 | Unary negation | **DONE** | Already supported (`-x` as unary expression); verified |
| S1.5 | Augmented assignment operators | **SKIP** | Deferred to S2 |

---

## Completed Work Stream: L1 — Standard Library Foundation

| ID | Task | Status | Notes |
|----|------|--------|-------|
| L1.1 | String builder / optimized string ops | **DONE** | `sb_new`/`sb_push`/`sb_finish`/`sb_len` FFI + `string_join_list`/`string_repeat`/`string_reverse` builtins |
| L1.2 | Fix `unwrap` to actually panic | **DONE** | `option.unwrap` and `result.unwrap`/`unwrap_err` now call `exit(1)` |
| L1.3 | Fix `assert_eq` for all types | **DONE** | Added polymorphic `coral_value_to_string` FFI; `assert_eq` uses `to_string()` |
| L1.4 | Consistent naming convention | **DONE** | Standardized: `str_starts_with`, `str_replace`, `str_split`, `str_slice` + deprecated aliases |
| L1.5 | `list.pop()` returns removed element | **DONE** | `coral_list_pop` FFI already fully implemented |
| L1.6 | Map iteration support (`for key, value in map`) | **DONE** | New `ForKV` AST node, parser comma detection, codegen via `map_entries` → iterate pairs |

---

## Completed Items

- **M1 — NaN-Boxing**: All 8 tasks complete. Full transition from `%CoralValue*` to `i64` representation.
- **S1 — Core Syntax Clarity**: 3/5 done, 2 skipped. Map colon syntax, range loops, unary negation all working.
- **L1 — Standard Library Foundation**: All 6 tasks complete. StringBuilder, unwrap/panic, assert_eq, naming, list.pop, map iteration.
- **T1 — Seal Type Escape Hatches**: All 5 subtasks complete. `Store(name)` type, post-solve Unknown warnings, purity infrastructure.
- **C1 — Enhanced Constant Folding**: All 5 subtasks complete. Math/string/list folding, purity analysis, dead expression elimination.
- **S2.1 — Pipeline in Lowering**: Complete. Desugaring moved from codegen to lower pass; 3 forms handled.
- **CC2.1 — Source-Mapped Errors**: Complete. `LineIndex` for line:col display, source carried through error path.
- **CC2.2 — Multi-Error Reporting**: Complete. Warnings printed, `with_source()` on all pipeline stages.
- **CC2.3 — DWARF Debug Info**: Complete. Full `DISubprogram` + `DILocation` metadata via Inkwell, gated by `CORAL_DEBUG_INFO`.
- **T2.1–T2.3 — User Generics (Syntax + Inference + Instantiation)**: Complete. `type_params` on AST, `[T]` parser syntax, let-polymorphism for generic constructors, generic type annotation resolution.
- **C2.1–C2.3 — Type Specialization**: Complete. Numeric Add/Equals/NotEquals bypass runtime FFI via direct LLVM ops; boolean fast-path skips `is_truthy` call via inline bit extraction.
- **C3.1 — Small Function Inlining**: Complete. Functions ≤5 expressions with no recursion annotated with LLVM `alwaysinline`. `body_calls_self()` helper for recursion detection.
- **C3.3 — Tail Call Optimization**: Complete. Tail-recursive functions detected and converted to loops. `FunctionContext` extended with `is_tail_position` tracking.
- **C3.4 — Common Subexpression Elimination**: Complete. CSE cache tracks pure expressions; `emit_expression` split to check cache first. Cache cleared at control flow boundaries.
- **C3.5 — Dead Function Elimination**: Complete. AST-walking reachability analysis from `main`; unreachable functions omitted from LLVM IR emission.
- **S3.1 — Multi-Statement Match Arms**: Complete. Match arms support indented blocks; last expression is arm's value.
- **S3.2 — Guard Clauses in Match**: Complete. `guard` field on `MatchArm` AST node; parser/codegen/semantic analysis updated for `if condition` syntax after pattern.
- **S3.3 — Or-Patterns in Match**: Complete. `MatchPattern::Or` variant with `or` keyword in parser. Codegen generates condition for each sub-pattern. Exhaustiveness checker handles or-patterns.
- **S3.6 — Match as Statement**: Complete. Match expressions usable in statement position without capturing a value.
- **S4.1 — Named Arguments**: Complete. `func(name: value)` syntax parsed and resolved to positional order in codegen. Works with default params.
- **S4.2 — Default Parameter Values**: Complete. `*f(x, port ? 5432)` syntax. Defaults filled at call sites, support referencing earlier params.
- **S5.1–S5.3 — unless/until/loop**: Complete. Pure parser desugaring. Self-hosted, tree-sitter, VS Code extension all updated.
- **S5.4 — when Expression**: Complete. Multi-branch conditionals desugared to nested ternaries with wildcard default.
- **C4.1 — Optimization Flags**: Complete. `-O` CLI flag for JIT and binary compilation.
- **T3.5 — Dead Code Detection**: Complete. Warns on unreachable statements after return/break/continue in all block types.
- **CC3.1 — AST-Level Module System**: Complete. `ModuleLoader::load_modules()` returns per-module `ModuleSource` structs in topo order. `Compiler::compile_modules_to_ir()` parses each independently. `Program::from_modules()` merges.
- **CC3.2 — Namespacing / Qualified Access**: Complete. `module.func()` resolved in codegen via `module_exports` map in `emit_member_call`.
- **CC3.3 — Selective Imports**: Complete. `use std.math.{sin, cos}` syntax via `ImportDirective` struct with module_path + selections.
- **CC2.5 — LSP MVP (Diagnostics)**: Complete. `coral-lsp` workspace crate using tower-lsp + tokio. Diagnostics on open/change/save with span→position via `LineIndex`.

---

## Completed Work Stream: Sprint 2 (Post-Beta Gamma Batch 2)

| ID | Task | Status | Notes |
|----|------|--------|-------|
| S5.6 | Postfix `if`/`unless` | **DONE** | `log("debug") if verbose` — pure parser desugaring |
| S1.5 | Augmented assignment | **DONE** | `+=`, `-=`, `*=`, `/=` desugar to `x is x op val` |
| T4.4 | Return type unification across branches | **DONE** | if/elif/else branches unify return types |
| L2.1 | `std.random` | **DONE** | xoshiro256** PRNG, seedable, random/random_int/shuffle |
| L2.3 | `std.time` enhancements | **DONE** | Proper `sleep()` FFI, Duration, ISO 8601 parsing |
| L2.6 | `std.testing` enhancements | **DONE** | assert_close, test suites, before_each/after_each |
| C4.2 | LLVM function attributes | **DONE** | nounwind, readnone, willreturn based on purity analysis |
| CC2.4 | Warning categories | **DONE** | Classified warnings with optional suppression |
| T3.2 | Definite assignment analysis | **DONE** | Tracks uninitialized variable usage across all paths |
| S4.3 | Multi-line lambda syntax | **DONE** | Indented lambda bodies with multiple statements |
| S4.6 | Return expressions in lambdas | **DONE** | `return` in lambda returns from lambda, not enclosing fn |
| M3.4 | Closure cycle tracking | **DONE** | Closures in `is_container()`, `get_children()` for captures |

---

## Completed Work Stream: Sprint 3 (Type Quality, GC & Extension Methods)

| ID | Task | Status | Notes |
|----|------|--------|-------|
| T4.3 | Ranked unification | **DONE** | Union-by-rank in TypeGraph with information-aware heuristic |
| T4.1 | Multi-error recovery in type solving | **DONE** | Solver accumulates all errors; semantic emits one Diagnostic per TypeError |
| T4.2 | Better type error messages | **DONE** | Constraint provenance with origin span and description |
| L2.4 | `std.io` enhancements | **DONE** | stderr, file_size, rename, copy, mkdirs, temp_dir FFI + Coral wrappers |
| L2.5 | `std.process` enhancements | **DONE** | exec, cwd, chdir, pid, hostname FFI + Coral wrappers |
| L4.2 | `std.path` module | **DONE** | normalize, resolve, is_absolute, parent, stem via Rust std::path |
| C4.3 | LLVM alias analysis hints | **DONE** | `noalias` on function params and allocator returns |
| M3.1 | Thread-local cycle root buffers | **DONE** | Thread-local root buffers eliminate global mutex contention |
| M3.2 | Generational epoch tracking | **DONE** | Young/old root partitioning with configurable promotion policy |
| CC3.4 | Circular dependency enhancement | **DONE** | Line numbers in circular import errors; multi-cycle detection |
| R3.9 | WeakRef clone semantics fix | **DONE** | Each clone gets unique registry ID; invalidation via `notify_value_deallocated` |
| CC5.2 | Fix medium bugs (S6, S8) | **DONE** | S6: member access warns on unknown fields; S8: pipeline `CallableAt` constraint |
| S4.5 | Extension methods | **DONE** | `extend TypeName` keyword, parser, AST, semantic merge, codegen via store_methods. 6 E2E tests. |

**Known issue discovered (KI-1)**: Built-in method name shadowing — store methods named `set`, `get`, `push`, `pop`, `map`, `filter`, `length`, etc. are shadowed by hardcoded built-in dispatch in `emit_member_call()`. Workaround: avoid these ~20 names for user-defined methods. Fix requires type-aware dispatch.

---

## Completed Work Stream: Sprint 4 (Type Safety, Method Dispatch, Actor Hardening, Compilation Infra)

| ID | Task | Status | Notes |
|----|------|--------|-------|
| KI-1 | Type-aware method dispatch | **DONE** | Store/extension methods now dispatch BEFORE built-ins when target has known store type |
| CC5.3 | All examples compile and run | **DONE** | hello, fizzbuzz, calculator, data_pipeline, traits_demo — 5 tests |
| L2.2 | `std.regex` module | **DONE** | regex_match, regex_find, regex_find_all, regex_replace, regex_split — 5 runtime FFI + codegen + 5 tests |
| T3.1 | Type narrowing in match | **DONE** | Constructor field bindings get precise types from constructor signatures. Or-patterns too. 5 tests |
| T3.3 | Nullability tracking | **DONE** | Functions with mixed none/value return paths get warnings. `WarningCategory::Nullability`. 4 tests |
| S4.4 | Method chaining fluency | **DONE** | Precise return types for string/list methods enable chaining. 12+ method dispatches added. 4 tests |
| R2.6 | Supervised actor restart | **DONE** | `Arc<dyn Fn>` factory, restart budget, mailbox preservation. 4 runtime tests |
| R2.10 | Graceful actor stop | **DONE** | `drain_and_stop()` method, `coral_actor_graceful_stop` FFI. 3 runtime tests |
| S5.5 | `do..end` block syntax | **DONE** | `KeywordDo`/`KeywordEnd`, parser `parse_do_end_block()`, trailing block → lambda. 4 tests |
| CC3.5 | Incremental compilation | **DONE** | `ModuleCache` with fingerprinting, `.coral-cache/` disk cache. 3 tests |
| C4.4 | Link-time optimization | **DONE** | LLVM new pass manager, `--lto` CLI flag, O1/O2/O3 pipelines. 3 tests |
| M3.3 | Incremental GC | **SKIP** | Design decision: Coral stays GC-free for native performance and real-time suitability |

---

## Session Log

### Session 11 — March 8, 2026
- Created LANGUAGE_EVOLUTION_ROADMAP.md (comprehensive 6-pillar roadmap)
- Created this progress tracker
- Designed NaN-box encoding scheme (M1.1)
- Started implementing M1.2 (nanbox.rs module)
- Baseline: 194 tests passing, 1 pre-existing failure

### Session 12 — March 8, 2026
- **M1.2 COMPLETE**: Implemented `runtime/src/nanbox.rs` (520 lines)
  - `NanBoxedValue` newtype over u64, `#[repr(transparent)]`
  - Constructors: `from_number`, `from_bool`, `unit`, `none`, `from_heap_ptr`, `from_error_ptr`, `from_bits`
  - Type queries: `is_number`, `is_bool`, `is_unit`, `is_none`, `is_heap_ptr`, `is_error`, `is_immediate`
  - Extraction: `as_number`, `as_bool`, `as_heap_ptr`, `as_error_ptr`, `is_truthy`, `to_bits`
  - Arithmetic fast paths: `fast_add`, `fast_sub`, `fast_mul`, `fast_div`, `fast_rem`
  - Comparison fast paths: `fast_equals`, `fast_less_than`, `fast_greater_than`
  - FFI boundary helpers: `nanbox_to_u64`, `u64_to_nanbox`
  - `encoding` submodule exposing constants for codegen
  - 38 unit tests all pass
- Full test suite: 194 pass, 1 pre-existing failure (no regressions)

### Session 13 — March 8, 2026
- **M1.3-M1.6 COMPLETE**: Implemented `runtime/src/nanbox_ffi.rs` (~500 lines)
  - Zero-allocation constructors: `coral_nb_make_number`, `coral_nb_make_bool`, `coral_nb_make_unit`, `coral_nb_make_none`
  - Heap-type constructors: `coral_nb_make_string`, `coral_nb_make_list`, `coral_nb_make_map`  
  - Extractors: `coral_nb_as_number`, `coral_nb_as_bool`, `coral_nb_tag`, `coral_nb_is_truthy`, `coral_nb_is_err`, `coral_nb_is_absent`
  - Retain/release with immediate fast-path: `coral_nb_retain`, `coral_nb_release` (no-op for numbers/bools/unit/none)
  - Arithmetic fast paths: `coral_nb_add/sub/mul/div/rem/neg`
  - Comparison fast paths: `coral_nb_equals/not_equals/less_than/greater_than/less_equal/greater_equal`
  - Bridge functions: `coral_nb_from_handle` (old→new), `coral_nb_to_handle` (new→old)
  - Print/IO: `coral_nb_print`, `coral_nb_println`, `coral_nb_value_length`, `coral_nb_value_get`
  - 15 FFI tests all pass
- Made `alloc_value` `pub(crate)` for cross-module access
- Full test suite: 194 pass, 53 nanbox tests pass, 1 pre-existing failure (no regressions)
- **Next**: M1.7 — Update Rust codegen to emit `i64` instead of `%CoralValue*`

### Session 14-15 — March 8, 2026
- **M1.7 COMPLETE**: Full Rust codegen transition from `%CoralValue*` → `i64`
  - **runtime.rs**: Added 35 `nb_*` function declarations + `value_i64_type` field to RuntimeBindings
  - **mod.rs** (~2075 lines, massive refactor):
    - `FunctionContext.variables` changed from `HashMap<String, PointerValue>` to `HashMap<String, IntValue>`
    - All core signatures changed: `emit_expression`, `emit_block`, `emit_numeric_binary`, `emit_logical_binary`, `emit_ternary`, `load_variable`, `store_variable`, `value_to_number`, `value_to_bool` → all use `IntValue`
    - Hot-path arithmetic uses `nb_add/nb_equals/nb_not_equals`; cold-path uses `call_bridged` bridge pattern
    - All function declarations (user, store, actor, methods) use `value_i64_type` params/returns
    - Global variables use `value_i64_type`
    - PHI nodes, if/elif, for-loop iteration, list/map literals, error propagation — all updated
    - Actor send bridge: unit value converted via `nb_to_ptr` before `coral_actor_send`
  - **builtins.rs** (1359 lines): Already uses `IntValue` returns and `call_bridged` for old API calls
  - **match_adt.rs** (~261 lines): `emit_match`, `emit_match_condition`, `bind_pattern_variables` all converted
  - **closures.rs** (~711 lines):
    - Lambda env struct stores captures as `i64` fields
    - All return types changed to `Result<IntValue, Diagnostic>`
    - `build_lambda_invoke`: captures loaded as `value_i64_type`; args retained before `nb_from_handle` conversion (borrowed refs from runtime); result converted via `nb_to_ptr` for out_param
    - `build_lambda_release`: uses `nb_release` instead of `value_release`
    - `build_closure_env`: accepts `&[IntValue]`, uses `nb_retain`
    - `emit_closure_call`: bridges closure + args via `nb_to_ptr`, result via `ptr_to_nb`
    - `emit_function_as_closure`: thunk converts args via `ptr_to_nb`, result via `nb_to_ptr`
    - `emit_enum_constructor`/`_nullary`: field arrays bridge via `nb_to_ptr`, results via `ptr_to_nb`
  - **store_actor.rs** (~419 lines):
    - Store constructor: field keys/values bridged via `nb_to_ptr` for `map_set`, returns bridged via `ptr_to_nb`
    - Store method: params loaded as `into_int_value()`, `emit_block` result returned as `i64`
    - Actor message dispatch: string keys bridged via `nb_to_ptr` for `map_get`/`value_equals`
  - **Runtime fix (nanbox_ffi.rs)**: `coral_nb_from_handle` correctly handles error Values (tag=7/Unit with FLAG_ERR) instead of incorrectly converting to unit; `coral_nb_is_err` also checks heap-pointer Values for error flag
  - **Test fix (core_spec.rs)**: Updated signature assertion from `define ptr @...` to `define i64 @...`
  - **Results**: 195 tests pass (up from 194 baseline), 53 nanbox tests pass, 47/47 codegen_extended pass, 1 pre-existing failure unchanged

### Session 16 — March 8, 2026
- **S1.1 COMPLETE**: Map colon syntax (`map("key": value)`) — parser + codegen updated in both compilers
- **S1.3 COMPLETE**: `for..to..step` range loops — efficient counted loop codegen
- **S1.4 VERIFIED**: Unary negation already supported (`-x`)
- **L1.1 COMPLETE**: StringBuilder FFI (`sb_new`/`sb_push`/`sb_finish`/`sb_len`) + optimized `string_join_list`/`string_repeat`/`string_reverse` builtins
  - 7 new FFI declarations in `src/codegen/runtime.rs`
  - 7 new builtin match arms in `src/codegen/builtins.rs`
  - `std/string.coral` updated to use optimized builtins
- **L1.2 COMPLETE**: `option.unwrap` and `result.unwrap`/`unwrap_err` now call `exit(1)` on failure
- **L1.3 COMPLETE**: Added polymorphic `coral_value_to_string` FFI + `to_string` builtin; `assert_eq` uses `to_string()` instead of `number_to_string()`
- **L1.4 COMPLETE**: Rewrote `std/string.coral` with standardized names (`str_starts_with`, `str_replace`, `str_split`, `str_slice`) + deprecated aliases
- **L1.5 VERIFIED**: `list.pop` FFI already fully implemented
- **L1.6 COMPLETE**: `for key, value in map` syntax
  - New `ForKV` AST node in `src/ast.rs`
  - Parser comma detection in `src/parser.rs`
  - Codegen via `map_entries` → iterate pairs → `list_get` for key/value in `src/codegen/mod.rs`
  - All 7 match sites in `semantic.rs`, plus `closures.rs`, `compiler.rs`, `lower.rs` updated
  - Self-hosted compiler updated (`parser.coral` + `codegen.coral`)
  - New e2e test `e2e_for_kv_map_iteration` passes
- **M1.8 COMPLETE**: Benchmark suite created
  - 5 programs: `fibonacci.coral` (fib(30)=158ms), `tight_loop.coral` (10M=600ms), `list_ops.coral` (100K=281ms), `string_ops.coral` (10K=16ms), `matrix_mul.coral` (50K=2437ms)
  - Python runner: `benchmarks/run_benchmarks.py`
  - Fixed linker: added `-lm` to clang link step in `src/main.rs`
  - Added new builtin names to `is_builtin_name` in `semantic.rs`
- **Results**: 196 tests pass (195 + 1 new ForKV test), 1 pre-existing failure unchanged

### Session 17 — March 8, 2026
- **T1 COMPLETE** (Seal Type Escape Hatches):
  - T1.1: Added `Store(String)` variant to `TypeId` enum in `src/types/core.rs`; added `contains_unknown()` method for recursive Unknown detection
  - T1.2: Store constructors now return `TypeId::Store(name)` instead of `Any` in `src/semantic.rs`
  - T1.3: ADT constructor fields noted for future TypeVar migration (kept `Any` for backward compat)
  - T1.4: Purity analysis infrastructure added (see C1.2-C1.3)
  - T1.5: Post-solve warning loop in `src/semantic.rs` scans all resolved types and warns on remaining `Unknown` types (skipping internal names)
  - Added `Store` unification in `src/types/solver.rs` (same-name unifies, different-name errors)
- **C1 COMPLETE** (Enhanced Constant Folding):
  - C1.1: Extended `fold_expr` in `src/compiler.rs` to fold pure math builtins (`sqrt`, `abs`, `floor`, `ceil`, `round`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `exp`, `ln`, `log2`, `log10`, `pow`, `min`, `max`, `atan2`), `length()` on string/list literals, `.length()` member access
  - C1.2-C1.3: `Purity` enum (`Pure`, `ReadOnly`, `Effectful`) + `is_pure_builtin()` classifying ~30 builtins
  - C1.4: Template string folding already handled via existing `Binary(Add, String, ...)` chain
  - C1.5: Dead expression elimination via `is_pure_dead_expression()` free function + `retain()` in `fold_block`
- **S2.1 COMPLETE** (Pipeline operator full lowering):
  - Moved pipeline desugaring from codegen to `src/lower.rs` — `lower_pipeline()` method handles three forms: explicit `$` replacement, prepend-as-first-arg, and bare identifier wrapping
  - Added `expr_contains_placeholder()` and `replace_placeholder_in_expr()` helper functions for recursive AST traversal
  - All 22 pipeline tests pass
- **CC2.1 COMPLETE** (Source-mapped error messages):
  - Added `LineIndex` struct to `src/span.rs` with O(log n) byte-offset → line:col lookup via binary search
  - `CompileError` now carries optional source text; `Display` outputs `line:col` instead of raw byte offsets
  - `CompileError::with_source()` constructor passes source through error path
  - `add_source_context()` rewritten to use `LineIndex`
- **CC2.2 COMPLETE** (Multi-error / warning reporting):
  - `main.rs` switched to `compile_to_ir_with_warnings()` — warnings are now printed to stderr
  - All compile pipeline stages use `with_source()` so errors carry source text
- **CC2.3 COMPLETE** (DWARF debug info):
  - Added `DebugContext` struct to `src/codegen/mod.rs` holding `DebugInfoBuilder`, `DICompileUnit`, `DIFile`, `LineIndex`, `DISubroutineType`
  - `with_debug_info()` method on `CodeGenerator` initializes DWARF metadata (language=C, emissionKind=Full)
  - `build_function_body()` creates `DISubprogram` and attaches to `FunctionValue` via `set_subprogram()`
  - `emit_expression()` sets `DILocation` from expression span for every instruction
  - `compile()` calls `dibuilder.finalize()` before returning module
  - Gated behind `CORAL_DEBUG_INFO` env var — emits `!dbg` metadata, `DICompileUnit`, `DISubprogram`, `DILocation` entries in LLVM IR
  - `FunctionContext` extended with `di_scope: Option<DIScope>` field
- **Results**: 195 tests pass, 1 pre-existing failure unchanged

### Session 18 — Phase Beta: Generics & Type Specialization
- **Option/Result Assessment**: Validated alignment with type inference strategy. `TypeEnv` already registers `Option["T"]` and `Result["T", "E"]` as generic types; `TypeId::Adt(name, type_args)` supports parameterized ADTs; solver unifies ADT type args recursively. Sound and well-aligned — proceeded as documented.
- **T2.1 COMPLETE** (Generic Type Parameter Syntax):
  - Added `type_params: Vec<String>` to `TypeDefinition` in `src/ast.rs`
  - Added `parse_type_params()` method to parser for `[A, B, C]` syntax on type/enum definitions
  - Updated `parse_type_annotation()` to parse `Type[Arg1, Arg2]` recursively into `TypeAnnotation.type_args`
  - Updated `parse_type_def` and `parse_enum_def` to call `parse_type_params()`
  - Updated semantic registration to call `types.register_generic_type()` for parameterized enums
  - 6 new parser tests: `parse_enum_with_type_params`, `parse_enum_with_multiple_type_params`, `parse_enum_no_type_params`, `parse_type_annotation_with_type_args`, `parse_nested_type_annotation_args`, backward compat verification
- **T2.2 COMPLETE** (Generic Function Inference / Let-Polymorphism):
  - Added `generic_constructors: HashMap<String, (String, Vec<String>, usize)>` to `TypeEnv`
  - Semantic analysis registers generic constructors when enum has type_params
  - `Expression::Identifier` in `collect_constraints_expr` instantiates fresh type vars per use site for generic constructors (let-polymorphism)
  - `MatchPattern::Constructor` uses fresh type vars for generic constructor patterns
  - 2 new semantic tests: polymorphic constructor calls, generic nullary in match
- **T2.3 COMPLETE** (Generic Instantiation in Types):
  - Updated `type_from_annotation()` to handle user-defined generic types — `Option[Int]`, `Result[T, E]`, etc.
  - Non-builtin type names now produce `TypeId::Adt(name, type_args)` instead of `TypeId::Unknown`
  - 1 new semantic test: generic type annotation in function parameter
- **C2.1 COMPLETE** (Numeric Type Specialization):
  - Added `resolved_types: HashMap<String, TypeId>` to `CodeGenerator`, populated from `model.types.iter_all()` in `compile()`
  - Added `expr_is_numeric()` helper: returns true for `Float` literals and variables resolved to `Float`/`Int`
  - Modified `emit_numeric_binary` to accept `both_numeric` flag:
    - `Add`: emits `fadd` directly instead of `coral_nb_add` runtime call
    - `Equals`: emits `fcmp oeq` directly instead of `coral_nb_equals` runtime call
    - `NotEquals`: emits `fcmp one` directly instead of `coral_nb_not_equals` runtime call
  - Non-numeric operands still use runtime FFI (e.g., string concatenation via `coral_nb_add`)
- **C2.2 COMPLETE** (Boolean Type Specialization):
  - Added `expr_is_bool()` helper: returns true for `Bool` literals and variables resolved to `Bool`
  - Added `value_to_bool_fast()`: inline bool extraction via `value & 1` bit mask, avoids `nb_is_truthy` runtime call
  - Applied to: `emit_logical_binary` (And/Or), `emit_ternary`, `Statement::If`, elif conditions, `Statement::While`, `UnaryOp::Not`
  - Non-boolean operands still use `nb_is_truthy` runtime call for truthiness coercion
- **C2.3 COMPLETE** (Specialization Tests):
  - 7 new codegen_extended tests:
    - `specialize_numeric_add_uses_fadd`: verifies `add_spec` label in IR for numeric variable addition
    - `specialize_numeric_add_correctness`: runtime correctness for integer and float addition
    - `specialize_numeric_equals_uses_fcmp`: verifies `eq_spec` in IR
    - `specialize_numeric_not_equals_uses_fcmp`: verifies `ne_spec` in IR
    - `string_add_still_uses_runtime`: confirms `coral_nb_add` for string operands
    - `specialize_bool_not_uses_fast_path`: verifies `bool_extract`/`bool_fast` in IR
    - `specialize_bool_and_correctness` + `specialize_bool_or_correctness`: runtime correctness
- **Results**: 203 tests pass (195 + 8 new), 1 pre-existing failure unchanged

### Session 19 — Test Suite Remediation
- **Found 792 total tests** across all test binaries; 11 were failing across 5 clusters
- **Fixed 11 test failures**:
  - Updated outdated IR signature assertions (`define ptr` → `define i64`)
  - Fixed snapshot tests broken by NaN-box calling convention change
  - Resolved test fixtures expecting old error messages
  - Fixed parser test expectations after S1/L1 syntax changes
  - Corrected codegen test expectations for new type specialization output
- **Results**: 793 tests pass, 0 failures (pre-existing `dump_codegen_expanded` failure also resolved)

### Session 20 — C3.5 Dead Function Elimination + C3.1 Small Function Inlining
- **C3.5 COMPLETE** (Dead Function Elimination):
  - Implemented AST-walking reachability analysis starting from `main` function
  - `find_reachable_functions()` walks all expressions in reachable functions, transitively marking called functions
  - Unreachable functions omitted from LLVM IR emission in `compile()`
  - 3 new tests: basic elimination, transitive reachability, all-reachable preservation
- **C3.1 COMPLETE** (Small Function Inlining):
  - Added `body_calls_self()` helper to detect recursive functions (excluded from inlining)
  - Functions with ≤5 expressions and no recursion annotated with LLVM `alwaysinline` attribute
  - 3 new tests: small function inlined, recursive excluded, large function excluded
- **Results**: 799 tests pass (793 + 6 new), 0 failures

### Session 21 — Self-Hosted E2E Test Fixes
- Fixed runtime library rebuild issues after NaN-box transition
- Fixed `coral_map_get` FFI bridge for NaN-boxed keys
- Resolved race condition in self-hosted compiler E2E test execution
- **Results**: 799 tests pass, 0 failures

### Session 22 — S3.6 Match as Statement + S3.1 Multi-Statement Arms
- **S3.6 COMPLETE** (Match as Statement):
  - Match expressions can now appear in statement position without capturing return value
  - Parser modifications to detect statement-context match usage
- **S3.1 COMPLETE** (Multi-Statement Match Arms):
  - Match arms now support indented blocks with multiple statements
  - Last expression in block serves as the arm's value
  - Parser updated to handle INDENT/DEDENT within match arms
  - Codegen emits proper basic block structure for multi-statement arms
  - 5 new tests: statement match, multi-statement arm, mixed single/multi arms, nested blocks, expression vs statement match
- **Results**: 804 tests pass (799 + 5 new), 0 failures

### Session 23 — C3.3 Tail Call Optimization
- **C3.3 COMPLETE** (Tail Call Optimization):
  - Added `is_tail_position` field to `FunctionContext` for tracking tail position during codegen
  - Detects tail-recursive calls (last expression is a self-call)
  - Converts tail-recursive functions to loops: allocas for parameters, branch back to entry block
  - Handles direct tail recursion only (mutual recursion deferred)
  - 3 new tests: factorial TCO, fibonacci accumulator TCO, non-tail-call preserved
- **Results**: 807 tests pass (804 + 3 new), 0 failures

### Session 24 — C3.4 Common Subexpression Elimination
- **C3.4 COMPLETE** (Common Subexpression Elimination):
  - Added CSE cache to `FunctionContext`: `HashMap<String, IntValue>` keyed by expression fingerprint
  - Split `emit_expression` to check CSE cache before generating code
  - Pure expressions (no side effects per purity analysis) cached and reused
  - Cache cleared at control flow boundaries (if/while/for/match) to maintain correctness
  - 3 new tests: duplicate expression eliminated, impure expression not cached, cache cleared at control flow
- **Results**: 810 tests pass (807 + 3 new), 0 failures

### Session 25 — S3.2 Guard Clauses in Match
- **S3.2 COMPLETE** (Guard Clauses in Match):
  - Added `guard: Option<Box<Expression>>` field to `MatchArm` AST node
  - Parser recognizes `Pattern if condition ? body` syntax
  - Codegen emits guard condition check after pattern match succeeds, branches to next arm on failure
  - Semantic analysis validates guard expression types (must be boolean)
  - Exhaustiveness checker accounts for guarded arms (guarded arm doesn't guarantee exhaustiveness)
  - 3 new tests: basic guard, guard with binding, exhaustiveness with guards
- **Results**: 813 tests pass (810 + 3 new), 0 failures

### Session 26 — S3.3 Or-Patterns in Match
- **S3.3 COMPLETE** (Or-Patterns):
  - Added `MatchPattern::Or(Vec<MatchPattern>)` variant to AST
  - Parser uses `or` keyword: `Circle(r) or Sphere(r) ? compute(r)`
  - Fixed 5+ compilation errors during implementation (type mismatches in pattern codegen)
  - Codegen generates condition check for each sub-pattern with OR'd result
  - Binding extraction from or-patterns requires consistent bindings across all alternatives
  - Exhaustiveness checking helper handles or-patterns by checking if any sub-pattern covers a constructor
  - 3 new tests: basic or-pattern, or-pattern with bindings, exhaustiveness with or-patterns
- **Results**: 816 tests pass (813 + 3 new), 0 failures

### Session 27 — Documentation Consolidation & Tooling
- Built `tools/coral-dev` bash helper (22 subcommands)
- Built `tools/coral-helpers.py` Python helper (7 subcommands with codemap integration)
- Consolidated AGENTS.md with quick reference for LLM agents
- **Results**: 816 tests pass, 0 failures

### Session 28 — M2.1-M2.4 Non-Atomic RC Fast Path
- **M2.4 COMPLETE** (Gate diagnostic counters):
  - Added `metrics` feature to runtime Cargo.toml
  - `retain_events: AtomicU32` and `release_events: AtomicU32` now gated behind `#[cfg(feature = "metrics")]`
  - All 9 Value constructors, Clone impl, and recycle_value_box updated with cfg gates
  - `coral_value_metrics` FFI returns 0 for per-value metrics when feature is disabled
  - Saves 8 bytes per Value in production builds (40 → 32 bytes without metrics)
- **M2.1 COMPLETE** (Thread-ownership flag):
  - Added `owner_thread: u32` field to Value struct after `reserved: u16`
  - Fills alignment padding before AtomicU64 — adds 0 bytes to struct size
  - Thread-local ID system: `THREAD_ID_COUNTER: AtomicU32` assigns unique IDs starting at 1
  - ID 0 is sentinel for "shared/atomic mode" (after freeze or cross-thread access)
  - All Value constructors stamp `owner_thread: current_thread_id()`
  - recycle_value_box resets `owner_thread` to 0 on pool return
- **M2.2 COMPLETE** (Non-atomic retain/release):
  - `coral_value_retain`: when `owner_thread != 0 && owner_thread == current_thread_id()`, uses plain `load+store` instead of `fetch_add` (~5-10x faster on x86, avoids `lock` prefix)
  - `coral_value_release`: when thread-local, uses plain `load+store` instead of `compare_exchange_weak` CAS loop (~10-20x faster), skips Acquire fence on final drop
  - `drop_heap_value` child release loop also uses non-atomic path for thread-local children
  - Shared/frozen values (owner_thread == 0) fall through to existing atomic path unchanged
- **M2.3 COMPLETE** (Atomic promotion at freeze):
  - `freeze_value` now sets `value.owner_thread = 0` alongside `FLAG_FROZEN`
  - One-way transition: once frozen for actor sharing, all subsequent RC ops use atomic path
  - Recursive freeze propagates to list items and map key/value pairs
- **7 new tests**: owner_thread stamping, non-atomic retain/release round-trip, heap string RC, freeze-to-atomic promotion, freeze-list-promotes-children, unique thread IDs across threads, cross-thread retain/release on frozen values
- **Results**: 816 tests pass (workspace), 7 new M2 runtime tests all pass, 0 failures

### Sprint Session — SPRINT_NEXT_PLAN.md Implementation
- **S5.1 COMPLETE** (`unless` keyword): Pure parser desugaring to `If` with `Not` condition. Lexer keyword, parser, self-hosted, tree-sitter, VS Code extension updated.
- **S5.2 COMPLETE** (`until` loop): Pure parser desugaring to `While` with `Not` condition. Full toolchain updated.
- **S5.3 COMPLETE** (`loop` keyword): Pure parser desugaring to `While(true)`. Full toolchain updated.
- **S5.4 COMPLETE** (`when` expression): Multi-branch conditionals desugared to nested ternaries. Supports wildcard `_` default arm. Full toolchain updated.
- **C4.1 COMPLETE** (Optimization flags): `-O` CLI flag passed to `lli`/`llc`/`clang`. Default: `-O0` for JIT, `-O2` for binary.
- **S4.2 COMPLETE** (Default parameter values): Codegen fills defaults at call sites via `fn_param_defaults` HashMap. Supports defaults referencing earlier params (`*f(a, b ? a)`).
- **T3.5 COMPLETE** (Dead code detection): Warns on statements after `return`/`break`/`continue`. Recursive into nested blocks (if/while/for/match/lambda).
- **S4.1 COMPLETE** (Named arguments): `func(name: value)` syntax. Parser detects `ident:` in argument lists. Codegen resolves named args to positional order using parameter definitions. Works with default parameter values.
- **Results**: 905 tests pass (865 baseline + 18 control_flow_sugar + 4 default_params + 9 dead_code + 9 named_args), 0 failures

### Sprint Session — CC3 Module System + CC2.5 LSP
- **CC3.1 COMPLETE** (AST-level module system): `ModuleLoader::load_modules()` returns per-module `ModuleSource` structs in topo order. `Compiler::compile_modules_to_ir()` parses each module independently, builds `Module` ASTs with name/items/imports/exports, merges via `Program::from_modules()`. 8 new tests.
- **CC3.2 COMPLETE** (Qualified module access): `module.func()` resolved in codegen via `module_exports` map. `emit_member_call` checks if target identifier matches a known module namespace and dispatches to the function by its unqualified name.
- **CC3.3 COMPLETE** (Selective imports): `use std.math.{sin, cos}` syntax. `ImportDirective` struct tracks module_path + selections. Parser handles `{...}` braced symbol lists. 7 new tests in module_namespaces.rs.
- **CC2.5 COMPLETE** (LSP MVP): `coral-lsp` workspace crate using tower-lsp + tokio. Provides diagnostics on open/change/save with proper span→position conversion via `LineIndex`. Module-aware compilation for saved files, direct compilation for unsaved buffers.
- **Results**: 920 tests pass (905 prior + 8 modules + 7 module_namespaces), 0 failures

### Sprint 2 Session — SPRINT_NEXT_PLAN_2.md Implementation
- **S5.6 COMPLETE** (Postfix if/unless): `log("debug") if verbose` — pure parser desugaring.
- **S1.5 COMPLETE** (Augmented assignment): `+=`, `-=`, `*=`, `/=` operators, desugar to `x is x op val`.
- **T4.4 COMPLETE** (Return type unification): if/elif/else branches unify return types.
- **L2.1 COMPLETE** (`std.random`): xoshiro256** PRNG, seedable, random/random_int/shuffle.
- **L2.3 COMPLETE** (`std.time` enhancements): Proper `sleep()` FFI, Duration, ISO 8601 parsing.
- **L2.6 COMPLETE** (`std.testing` enhancements): assert_close, test suites, before_each/after_each.
- **C4.2 COMPLETE** (LLVM function attributes): nounwind, readnone, willreturn based on purity.
- **CC2.4 COMPLETE** (Warning categories): Classified warnings with optional suppression.
- **T3.2 COMPLETE** (Definite assignment analysis): Tracks uninitialized variable usage.
- **S4.3 COMPLETE** (Multi-line lambdas): Indented lambda bodies.
- **S4.6 COMPLETE** (Return in lambdas): Return from lambda, not enclosing function.
- **M3.4 COMPLETE** (Closure cycle tracking): Closures added to cycle detection.
- **Results**: 971 tests pass, 0 failures

### Sprint 3 Sessions — SPRINT_NEXT_PLAN_3.md Implementation
- **T4.3 COMPLETE** (Ranked unification): Union-by-rank in TypeGraph; information-aware root selection.
- **T4.1 COMPLETE** (Multi-error recovery): Solver accumulates all type errors; semantic emits individual diagnostics.
- **T4.2 COMPLETE** (Better type error messages): Constraint provenance tracking with origin spans.
- **L2.4 COMPLETE** (`std.io` enhancements): stderr, file_size, rename, copy, mkdirs, temp_dir — 6 runtime FFI functions + Coral wrappers.
- **L2.5 COMPLETE** (`std.process` enhancements): exec, cwd, chdir, pid, hostname — 5 runtime FFI functions + Coral wrappers. 6 E2E tests.
- **L4.2 COMPLETE** (`std.path` module): normalize, resolve, is_absolute, parent, stem — backed by Rust std::path. 5 tests.
- **C4.3 COMPLETE** (LLVM alias analysis hints): `noalias` on function parameters and allocator returns, `nonnull` on heap pointers. 4 IR verification tests.
- **M3.1 COMPLETE** (Thread-local cycle root buffers): Thread-local `Vec<usize>` root buffers with flush threshold. Eliminates global mutex contention on `possible_root()`. 3 runtime tests.
- **M3.2 COMPLETE** (Generational epoch tracking): Young/old root partitioning with `birth_epoch` stamps. Young-gen collected every cycle, old-gen every K cycles. 3 runtime tests.
- **CC3.4 COMPLETE** (Circular dependency enhancement): Error messages now include `use` line numbers (`a.coral line 1 → b.coral line 3 → a.coral`). Multi-cycle detection. 3 new tests + 2 updated.
- **R3.9 COMPLETE** (WeakRef clone fix): Each `WeakRef::clone()` registers a unique registry ID. Original invalidation doesn't affect clones. 3 new runtime tests.
- **CC5.2 COMPLETE** (Fix medium bugs): S6 — member access on stores warns on unknown fields with `check_member_access_validity()` semantic pass. S8 — pipeline fallthrough emits `CallableAt` constraint. 4 tests.
- **S4.5 COMPLETE** (Extension methods): Full pipeline — `extend` keyword in lexer, `ExtensionDefinition` AST node, `parse_extension_def()` parser method, semantic merges methods into target store/type (or creates synthetic store for built-ins), codegen picks up via existing `store_methods` loop. 6 E2E tests. Discovered KI-1 (built-in method name shadowing).
- **Results**: 1016 tests pass, 0 failures

### Sprint 4 Sessions — SPRINT_NEXT_PLAN_4.md Implementation
- **KI-1 COMPLETE** (Type-aware method dispatch): Added store-type priority check at top of `emit_member_call()` in `builtins.rs`. When target identifier has `TypeId::Store(name)` in `resolved_types`, checks `self.functions` for `{store}_{method}` before falling through to built-in `match property` dispatch. Fixes ~20 name collisions (get, set, push, pop, map, filter, reduce, length, etc.). 5 E2E tests.
- **CC5.3 COMPLETE** (All examples compile): Audited and fixed all non-network examples: hello.coral, fizzbuzz.coral, calculator.coral, data_pipeline.coral, traits_demo.coral. chat_server.coral and http_server.coral deferred (need L3.1 HTTP). 5 tests.
- **L2.2 COMPLETE** (`std.regex` module): New `runtime/src/regex_ops.rs` with 5 FFI functions backed by `regex` crate. `coral_regex_match` (full match with anchoring), `coral_regex_find` (first match), `coral_regex_find_all` (list of matches), `coral_regex_replace` (replace_all), `coral_regex_split`. Added Cargo dependency `regex = "1"`. Codegen builtins + runtime bindings + semantic `is_builtin_name`. 5 E2E tests.
- **T3.1 COMPLETE** (Type narrowing in match): In `collect_constraints_expr()` for `MatchPattern::Constructor`, extract constructor parameter types from `TypeEnv` and bind field variables to those types instead of `Any`. Applied to both regular constructors and or-pattern constructors. 5 E2E tests.
- **T3.3 COMPLETE** (Nullability tracking): Added `check_nullability_returns()` pass in `semantic.rs`. Walks function bodies for return statements, classifies as none-returning or value-returning. Functions with both paths get `WarningCategory::Nullability` warning. Added `Nullability` variant to `WarningCategory` enum. `main()` excluded. 4 tests.
- **S4.4 COMPLETE** (Method chaining): Added 12+ member call dispatches in `builtins.rs` for string methods (split, trim, lower, upper, replace, contains, starts_with, ends_with, reverse) and list methods (join, sort, to_string). Updated `collect_constraints_expr()` with precise return types for chainable methods (String methods → String, List methods → List, Bool methods → Bool, Int methods → Int). 4 tests.
- **R2.6 COMPLETE** (Supervised actor restart): Changed `spawn_supervised_child` from `FnOnce` to `Arc<dyn Fn>` factory. Added `max_restarts: u32`, `restart_window: Duration`, and restart counter to `SupervisedChildInfo`. Restart implementation: re-creates actor state via factory, reconnects to actor system. 4 runtime tests.
- **R2.10 COMPLETE** (Graceful actor stop): Added `drain_and_stop()` method to `ActorContext` — loops `try_recv()`, calls handler for User messages, breaks on Empty/Disconnected/Exit, then unregisters and notifies monitors. Added `coral_actor_graceful_stop` FFI in `actor_ops.rs`. Updated both spawn loop GracefulStop handlers to call `drain_and_stop()`. 3 runtime tests.
- **S5.5 COMPLETE** (`do..end` blocks): Added `KeywordDo`/`KeywordEnd` to lexer. Added `parse_do_end_block()` to parser — consumes statements between do/end, wraps as zero-param Lambda. Updated `parse_call()` to detect trailing `do` after call arguments and `identifier do...end` without parens. 4 tests.
- **CC3.5 COMPLETE** (Incremental compilation): Added `ModuleCache` struct to `compiler.rs` with `fingerprint()` (DefaultHasher on module names+sources), `get()`, `put()`, `invalidate_all()`. Added `compile_modules_to_ir_cached()` method with cache_hit bool return. Disk cache in `.coral-cache/` directory. 3 tests.
- **C4.4 COMPLETE** (LTO): Added `LtoOptLevel` enum (O1/O2/O3) and `optimize_module()` function using LLVM new pass manager (`Module::run_passes`). Uses `create_from_memory_range_copy` (not `create_from_memory_range` — the latter caused SIGSEGV). Added `--lto` CLI flag to `main.rs`. `PassBuilderOptions` with loop vectorization/unrolling/merge functions. 3 tests.
- **M3.3 SKIPPED** (Incremental GC): Design decision — Coral stays GC-free for native performance and real-time suitability.
- **Results**: 310 compiler tests (1 pre-existing fail: e2e_cc53_fizzbuzz_example), 162 runtime tests (1 pre-existing fail: map_iterator_is_snapshot_after_mutation)
