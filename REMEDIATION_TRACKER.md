# Coral Language — Remediation Tracker

**Created:** February 17, 2026
**Updated:** Session 9 — Trait codegen, dead code cleanup, MatchPattern fix, 12 new E2E tests
**Companion to:** REMEDIATION_PLAN.md, COMPREHENSIVE_REVIEW_REPORT.md

---

## Language Design Decisions (Binding Constraints)

> These decisions override any earlier task descriptions.

1. **Pure Type Inference** — No type annotations, no `->` return types, no parameter type annotations anywhere in syntax. All typing via inference.
2. **Method-Based Equality** — No `==` or `!=` operators. Use `var.equals(otherVar)` / `var.not_equals(otherVar)` / `not var.equals(otherVar)`. Runtime `coral_value_equals()` and `coral_value_not_equals()` exist; built-in methods `.equals()` / `.not_equals()` wired in codegen.
3. **`is` for Binding** — `=` and `==` tokens removed from lexer (produce helpful errors). `is` used for binding at statement level and equality comparison in expression context. Use `.equals()` for bare-identifier equality at statement level.

---

## Status Summary

| Metric | Before | Current | Target (Alpha) |
|--------|--------|---------|----------------|
| Critical bugs | 7 | **0** | 0 |
| High-severity bugs | 15 | **1 remaining** | 0 |
| Compiler tests | 248 pass / 1 fail | **406 pass / 0 fail** | 350+ pass |
| Runtime tests | 76 pass | **84 pass** | 100+ pass |
| E2E execution tests | 15 pass | **121 pass** | 50+ pass |
| Feature completeness | ~30% | **~80%** | ~60% |

> **Note:** Session 9 completed trait codegen (5.2.7), guard-statement syntax (`cond ? stmt`), fixed 7 medium-severity bugs (P5, A1, P10, A3, D1, D2, T5), removed ~116 lines of dead code, added 21 new E2E tests + 3 self-hosting regression tests. Self-hosted lexer.coral now compiles to LLVM IR. Total: 406 tests, 121 E2E, 0 failures, 0 warnings. Only 1 high-severity bug remains (T2).

---

## Phase 1: Critical Bug Fixes — COMPLETE

### Week 1: Codegen Crash Fixes — DONE

| Task | Bug ID | Description | Status | Notes |
|------|--------|-------------|--------|-------|
| 1.1 | C1 | `Statement::Return` — return Value* directly, not f64-cast | **DONE** | codegen/mod.rs ~L475 |
| 1.2 | C2 | Actor handler dispatch — pass Value* data, not f64 | **DONE** | Both sequential and hash paths |
| 1.3 | C3 | Hash-based actor dispatch — compile-time vs runtime hash mismatch | **DONE** | Removed entire hash path; always sequential dispatch |
| 1.4 | C4 | `make_map` zero-arg call in actor constructor | **DONE** | Now passes null_entries + zero_len |
| 1.5 | C5 | `value_hash` return type / broken hash dispatch consumer | **DONE** | Consumer removed with C3; declaration left intact |
| 1.6 | — | End-to-end execution test harness | **NOT STARTED** | Deferred to Phase 2 Week 6 or Phase 3 |

### Week 2: Runtime Critical Fixes — DONE

| Task | Bug ID | Description | Status | Notes |
|------|--------|-------------|--------|-------|
| 2.1 | R1 | Cycle detector use-after-free / deadlock | **DONE** | Added `drop_heap_value_for_gc()` + `dealloc_value_box()`; also fixed deadlock in `scan_black`/`scan` (lock held across recursion) |
| 2.2 | R2 | Symbol interning misreads string memory layout | **DONE** | Uses `value_to_rust_string()` now (handles inline + heap) |
| 2.3 | R3 | Store FFI TAG constants off-by-one | **DONE** | Aligned to Number=0, Bool=1, String=2, ..., Unit=7, Bytes=9 |
| 2.4 | R5 | `retain_events`/`release_events` data race | **DONE** | Changed to `AtomicU32`, all sites updated |
| 2.5 | R4 | `handle_to_stored_value` stub | **DONE** | Full implementation with tag dispatch for all value types |
| 2.6 | R7 | UUID7 atomicity | **NOT STARTED** | Lower priority; deferred |

### Week 3: Test Infrastructure — NOT STARTED

| Task | Description | Status | Notes |
|------|-------------|--------|-------|
| 3.1 | E2E test harness (compile → link → run → assert) | **DONE** | `tests/execution.rs` — compile_to_ir → lli -load libruntime.so → assert stdout |
| 3.2 | 15 execution tests | **DONE** | hello world, arithmetic, string concat, function calls, nested calls, ternary, if/else, if/elif/else, boolean logic, comparisons, recursion, multiple return paths, string equality, integer equality, globals |
| 3.3 | Fix `parser_invalid_fixtures` test | **DONE** | Added explicit Indent/Dedent arms in parse_item(); updated .expect files |
| 3.4 | Delete/integrate orphaned runtime files | **DONE** | Removed value.rs, memory.rs, store_old.rs, collections/ from runtime/src/ |

---

## Phase 1 — Additional Fixes (High-Severity)

These high-severity fixes from Phase 2-4 were completed early during Phase 1 work:

| Bug ID | Category | Description | Status | Notes |
|--------|----------|-------------|--------|-------|
| R6 | Actor | `spawn_named` TOCTOU race condition | **DONE** | Registry lock held for entire check-and-register |
| S1 | Semantic | Forward reference failures for types/traits/stores | **DONE** | First pass now collects Type, TraitDefinition, store names |
| S4 | Semantic | Store/type method bodies never scope-checked | **DONE** | Second pass calls `check_function()` for each method |
| P1 | Parser | No for/while/loop constructs | **DONE** | Full pipeline: lexer → parser → semantic → codegen |
| P2 | Parser | No if/elif/else blocks | **DONE** | Full pipeline: lexer → parser → semantic → codegen |
| P3 | Parser | `return` keyword lexed but never parsed | **DONE** | Added `KeywordReturn` dispatch in `parse_statement()` |

---

## Remaining High-Severity Bugs (1)

| Bug ID | Category | Description | Phase | Complexity |
|--------|----------|-------------|-------|------------|
| T2 | Types | Generic instantiation faked (Option→List) | Phase 4 | High — needs proper substitution |

> **T1 reclassified as by-design**: Int/Float silently unifying is correct behavior — the runtime has a single `Number(f64)` representation, MIR uses `Number(f64)`, and codegen emits identical `f64` code for both. The distinction exists only in the AST (`Integer(i64)` vs `Float(f64)`) for const-folding precision. If a real integer runtime type is added later, this decision should be revisited.

### Session 8 Fixes

| Bug ID | Category | Description | Status | Notes |
|--------|----------|-------------|--------|-------|
| T3 | Types | No ADT types in type system | **DONE** | Added `TypeId::Adt(String)` variant; updated `is_concrete()`, `format_type()`, `unify()`, `occurs()` |
| S2 | Semantic | Variant constructors typed as `Any` | **DONE** | Constructors now return `TypeId::Adt(enum_name)`; match patterns constrain scrutinee to ADT type |
| S3 | Semantic | Constructor name collisions (no namespace) | **DONE** | Added `constructor_owners` HashMap with collision detection |
| P4 | Parser | `self.field is value` desugaring hack | **DONE** | Added `Statement::FieldAssign` AST node; updated parser, codegen, semantic, lower, mir_lower, compiler |
| C5* | Codegen | `value_hash` return type mismatch | **RESOLVED** | Consumer removed; declaration harmless |

> *C5 was resolved in Session 5 — the broken hash-based dispatch that consumed `value_hash` was removed entirely (C3 fix).

**Adjusted remaining: 1 high-severity bug (T2). Medium-severity: P5, A1, P10, A3 resolved in Session 9.**

---

## Completed Changes — File Impact Summary

### Compiler (`src/`)

| File | Changes |
|------|---------|
| [src/codegen/mod.rs](src/codegen/mod.rs) | C1: Return fix. C2: Actor dispatch fix. C3+C5: Removed hash dispatch. C4: Actor map init. P1+P2: Full codegen for If/While/For/Break/Continue. Added `loop_stack` to FunctionContext. P4: `Statement::FieldAssign` codegen with `coral_map_set`. Session 9: Type method declaration + body compilation, MatchPattern::List handler. |
| [src/lexer.rs](src/lexer.rs) | P1+P2: Added keywords: `if`, `elif`, `else`, `while`, `for`, `in`, `break`, `continue` |
| [src/ast.rs](src/ast.rs) | P1+P2: Added Statement variants: If, While, For, Break, Continue. P4: Added `Statement::FieldAssign`. Session 9: Removed `PersistenceMode`/`STORE_DEFAULT_FIELDS`, `MatchPattern::List` → `Vec<MatchPattern>`. |
| [src/parser.rs](src/parser.rs) | P1+P2+P3: `parse_if_statement()`, `parse_while_statement()`, `parse_for_statement()`, return dispatch. P4: `parse_self_field_assignment()` emits FieldAssign. Session 9: Removed `synchronize()`, removed `-> Type` parsing, list pattern parsing. |
| [src/semantic.rs](src/semantic.rs) | S1: Forward refs in first pass. S4: Method body scope-checking. P1+P2: New statement handling. S2: ADT-typed constructors. S3: Constructor collision detection. T3: Match pattern ADT constraints. P4: FieldAssign in 6 match blocks. Session 9: `inject_trait_default_methods()`, removed `lookup_current_frame()`, MatchPattern::List constraints. |
| [src/types/core.rs](src/types/core.rs) | T3: Added `TypeId::Adt(String)` variant, updated `is_concrete()` and `format_type()` |
| [src/types/solver.rs](src/types/solver.rs) | T3: ADT unification rules, `occurs()` check for Adt |
| [src/compiler.rs](src/compiler.rs) | P1+P2: New statement handling in `fold_block`. P4: FieldAssign in `fold_block` |
| [src/lower.rs](src/lower.rs) | P1+P2: New statement handling in lowering passes. P4: FieldAssign in lowering |
| [src/mir_lower.rs](src/mir_lower.rs) | P1+P2: Skip new statements (MIR doesn't support them yet). P4: FieldAssign skip |
| [tests/parser_snapshots.rs](tests/parser_snapshots.rs) | P1+P2: Snapshot support for new statement types |

### Runtime (`runtime/src/`)

| File | Changes |
|------|---------|
| [runtime/src/lib.rs](runtime/src/lib.rs) | R1: Added `drop_heap_value_for_gc()` + `dealloc_value_box()`. R5: AtomicU32 for retain/release events. Made `string_to_bytes` and `value_to_rust_string` pub(crate). New: `coral_list_len`/`coral_list_get_index` FFI wrappers. Iterative `drop_heap_collect_children` + worklist-based `drop_heap_value`. Thread-local `LOCAL_VALUE_POOL` with overflow to global. CAS-based `coral_value_release` (eliminates TOCTOU). **Flag collision fix**: `is_err()`/`is_absent()`/`is_ok()` now guard against inline string length bits overlapping FLAG_ERR/FLAG_ABSENT. ErrorMetadata GC path leak fix. MAP_SLOTS_ALLOCATED rehash tracking. 6 leak detection tests. |
| [runtime/src/cycle_detector.rs](runtime/src/cycle_detector.rs) | R1: `collect_white` uses GC-safe deallocation. Fixed deadlock in `scan_black`/`scan` (lock released before recursion). Added `collecting` guard, auto-collection toggle, force-collection FFI. **R8 fix**: `notify_value_freed()` removes handles before freeing. `mark_roots`/`scan`/`collect_white` verify handles under lock before dereferencing. `get_children` handles Store arm. |
| [runtime/src/symbol.rs](runtime/src/symbol.rs) | R2: Uses `value_to_rust_string()` for correct string reading |
| [runtime/src/store/ffi.rs](runtime/src/store/ffi.rs) | R3: TAG constants corrected. R4: Full `handle_to_stored_value()` implementation. New: `StoreHandleInfo.data_path` field, `extract_field_pairs()` helper, fields wired in create/update, `coral_store_get_by_uuid` rewritten with indexed lookup, extern decls use `coral_list_len`/`coral_list_get_index`. |
| [runtime/src/store/engine.rs](runtime/src/store/engine.rs) | New: `SharedStoreEngine::get_by_uuid()` method. New tests: `test_shared_engine_get_by_uuid`, `test_engine_full_persistence_cycle`. |
| [runtime/src/actor.rs](runtime/src/actor.rs) | R6: Atomic check-and-register in `spawn_named`. New: `TimerWheel` shutdown (AtomicBool + JoinHandle). `WorkQueue` shutdown + worker handle storage. `Scheduler::shutdown()`. `ActorSystem::shutdown()` (timer → scheduler → save_all_engines). |

---

## Next Phase: Phase 2 Remaining + Phase 3 Planning

### Phase 2 Remaining: Essential Operators & Syntax — **COMPLETE**

Weeks 4-5 (loops, if/else, return) and Week 6 (operators, syntax) are **all complete**.

### Phase 3: Runtime Hardening (Est. 80h)

| Week | Focus | Key Tasks |
|------|-------|-----------|
| **Week 7** | Store FFI Completion | **DONE** — data_path wiring, fields in create/update, get_by_uuid indexed, SharedStoreEngine::get_by_uuid, coral_list_len/coral_list_get_index FFI wrappers, 2 new engine tests |
| **Week 8** | Concurrency Fixes | **DONE** — Thread shutdown (8.3): AtomicBool + JoinHandle for TimerWheel/WorkQueue/Scheduler/ActorSystem. Thread-local pools (8.4): LOCAL_VALUE_POOL thread_local, removed O(n) duplicate scan. Refcount CAS (8.5): compare_exchange_weak loop eliminates TOCTOU. Iterative drop (8.6): worklist-based drop_heap_collect_children + iterative release loop. Work-stealing (8.1): deferred (needs crossbeam). |
| **Week 9** | Memory Hardening | **DONE** — R8: scan() safety (notify_value_freed, lock-guarded dereferences, Store arm in get_children). Flag collision fix (is_err/is_absent/is_ok guard inline strings). ErrorMetadata GC path leak fix. MAP_SLOTS_ALLOCATED rehash delta. 6 leak detection tests (numbers, heap strings, inline strings, list+children, nested list, map). |

### Phase 3 Dependencies
- Phase 2 is complete — Phase 3 can start immediately
- Store E2E tests (7.5) can use the new index/subscript syntax (6.3) for ergonomic testing
- Concurrency fixes (Week 8) are independent and can start anytime

### Recommended Next Session Priority

1. ~~End-to-end test harness (3.1 + 3.2)~~ — **DONE** (15 tests, all passing)
2. ~~Phase 3 Week 7: Store FFI Completion~~ — **DONE** (fields, get_by_uuid, data_path, FFI wrappers)
3. ~~Phase 3 Week 8: Concurrency Fixes~~ — **DONE** (shutdown, thread-local pools, CAS refcount, iterative drop)
4. **Phase 3 Week 9: Memory Hardening** — scan() safety, ErrorMetadata free path, MapObject resize, leak detection

### Known Limitations Discovered During E2E Testing
- ~~**While loop variable rebinding**: `i is i + 1` fails with "duplicate binding"~~ — **FIXED** (alloca-based variable storage + rebinding allowed)
- ~~**If/elif/else as expression return**: Returns 0 instead of string values from branches~~ — **FIXED** (PHI nodes for branch values + `return` dead-code block fix)
- ~~**Template string interpolation**: Compiles but produces `()` at runtime~~ — **FIXED** (runtime auto-coercion in `coral_value_add`)
- ~~**`.length()` nested in call**: `log(s.length())` fails with type inference error~~ — **FIXED** (special-cased Member callee in Call constraint + `.length()` method dispatch in codegen)
- **Keywords in call args**: `log(a and b)` fails to parse; works when bound to variable first

### Session 9: Trait Codegen, Dead Code Cleanup, Self-Hosting E2E Tests

| Fix | Description | Files Changed |
|-----|-------------|---------------|
| **Trait codegen (5.2.7)** | Inject default trait method bodies into type/store method lists at semantic level. Added type method declaration + compilation in codegen (was completely missing). | `src/semantic.rs`, `src/codegen/mod.rs` |
| **Guard-statement syntax** | `cond ? return/break/stmt` desugars to `if cond { stmt }`. Handles inline and block body. Binding-as-condition reinterprets `name is val ?` as equality check. | `src/parser.rs` |
| **Dead code: P5** | Removed `synchronize()` (~30 lines) | `src/parser.rs` |
| **Dead code: A1** | Removed `PersistenceMode` enum + `STORE_DEFAULT_FIELDS` (~16 lines) | `src/ast.rs` |
| **Dead code: P10** | Removed `-> Type` return annotation parsing from trait methods (~5 lines) | `src/parser.rs` |
| **Dead code: misc** | Removed `lookup_current_frame()`, `dependency_hash`, `constraint_statistics()`/`ConstraintStats` (~65 lines) | `src/semantic.rs`, `src/module_loader.rs`, `src/types/solver.rs` |
| **A3: MatchPattern::List** | Changed from `Vec<Expression>` to `Vec<MatchPattern>` with proper pattern parsing and codegen. | `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`, `src/codegen/mod.rs`, `tests/parser_snapshots.rs` |
| **D1: Span file ID** | Added `file_id: u32` field (default 0) to `Span`. Added `Span::with_file()`. Enables multi-file diagnostics. | `src/span.rs` |
| **D2: Diagnostic severity** | Added `Severity` enum (Error/Warning/Info) and `severity` field to `Diagnostic`. Added `Diagnostic::warning()` constructor. | `src/diagnostics.rs` |
| **T5: Generic leak** | Removed leaked `type_params` insertions in `instantiate_generic()` — the bindings were dead writes with side-effect contamination. | `src/types/env.rs` |
| **5 trait E2E tests** | Default methods, required methods, override default, multiple methods, store own + trait. | `tests/execution.rs` |
| **12 self-hosting E2E tests** | Map create/get/set, list push+iterate, string ops, nested if/elif, recursion, higher-order functions, match expressions, error values, while+string building, nested functions, for+accumulator. | `tests/execution.rs` |
| **4 guard E2E tests** | Guard with return, expression condition, loop break/continue, multiple guards (fizzbuzz-style). | `tests/execution.rs` |
| **3 self-hosting regression tests** | Lexer module loading, lexer IR compilation, std.char verification. | `tests/self_hosting.rs` |

### Session 8: ADT Type System & Store Field Assignment

| Fix | Description | Files Changed |
|-----|-------------|---------------|
| **TypeId::Adt(String)** | Added ADT variant to type system. `is_concrete()` returns true, `format_type()` displays name. ADT-ADT unification requires matching names. | `src/types/core.rs`, `src/types/solver.rs` |
| **ADT-typed constructors** | Nullary constructors typed as `Adt(name)`, constructors with fields as `Func(params, Adt(name))`. Match patterns constrain scrutinee to ADT type. | `src/semantic.rs` |
| **Constructor collision detection** | `constructor_owners` HashMap tracks constructor→enum mapping. Two enums with same constructor name produces error. | `src/semantic.rs` |
| **Statement::FieldAssign** | Replaced synthetic `self.set("field", value)` call with proper AST node. Updated across 6 source files + test file. Codegen emits direct `coral_map_set`. | `src/ast.rs`, `src/parser.rs`, `src/codegen/mod.rs`, `src/semantic.rs`, `src/lower.rs`, `src/mir_lower.rs`, `src/compiler.rs`, `tests/parser_snapshots.rs` |
| **8 ADT E2E tests** | Shape (Circle/Rectangle/Point), Expr (recursive eval), Maybe (map), List (Cons/Nil sum/length), Outcome, Answer, Color (list loop), Direction (wildcard). | `tests/execution.rs` |
| **4 Store E2E tests** | Basic counter (field mutation), Point (move_by/describe), Greeter (string field), multiple instances (independence). | `tests/execution.rs` |

### Session 7: Codegen & Feature Improvements

| Fix | Description | Files Changed |
|-----|-------------|---------------|
| **Alloca-based variables** | Variables use `alloca`+`load`/`store` instead of direct SSA pointers. Enables mutation in while loops. | `src/codegen/mod.rs` |
| **Variable rebinding** | Removed duplicate binding check in semantic analysis. `x is 1; x is 2` now allowed (like Rust's `let` shadowing). | `src/semantic.rs` |
| **Return statement fix** | `Statement::Return` no longer emits `wrap_number(0.0)` after `ret` terminator. Returns `const_null()` — keeps `ret` as last instruction so `get_terminator()` works. | `src/codegen/mod.rs` |
| **PHI nodes for if/elif/else** | Branches that fall through (no return) collect values for PHI node at merge block. Branches with `return` are correctly skipped. | `src/codegen/mod.rs` |
| **Template string coercion** | `coral_value_add` auto-coerces Number/Bool/Unit to string when other operand is String. Added `value_to_display_string()` helper. | `runtime/src/lib.rs` |
| **Nested method inference** | `log(s.length())` no longer fails — Call constraint with Member callee returns known method result types directly. | `src/semantic.rs` |
| **Function-as-value** | Named functions can be passed as arguments: `apply(double, 21)`. Creates closure thunk wrapping the function. | `src/codegen/mod.rs` |
| **`.length()` method** | Added to member call dispatch — calls `coral_value_length` for strings and lists. | `src/codegen/mod.rs` |
| **Store constructor fix** | `make_*` functions only treated as store constructors when explicitly registered (not any user function starting with `make_`). | `src/codegen/mod.rs` |
| **Float parsing edge case** | Trailing dot at EOF treated as integer + dot token, not incomplete float. | `src/lexer.rs` |
| **Dead code cleanup** | `#[allow(dead_code)]` on future-use infrastructure: `dependency_hash`, `synchronize`, `lookup_current_frame`, `constraint_statistics`. | Various |

---

## Appendix: Bug Checklist (Updated)

### Critical (7) — ALL RESOLVED
- [x] C1: Statement::Return f64/pointer confusion
- [x] C2: Actor handler dispatch f64/Value* confusion
- [x] C3: Hash-based actor dispatch hash function mismatch
- [x] C4: make_map zero-arg to two-arg function
- [x] R1: Cycle detector deadlock + use-after-free
- [x] R2: Symbol interning memory layout crash
- [x] R3: Store FFI TAG constants off by one

### High (15) — 1 REMAINING
- [x] P1: No for/while/loop constructs
- [x] P2: No if/elif/else blocks
- [x] P3: return keyword lexed but never parsed
- [x] P4: self.field desugaring hack
- [x] S1: Forward reference failures for types/traits/stores
- [x] S2: Variant constructors typed as Any
- [x] S3: Constructor name collisions (no namespace)
- [x] S4: Store/type method bodies never scope-checked
- [x] T1: Int/Float silently unify — **BY DESIGN** (runtime uses single Number(f64))
- [ ] T2: Generic instantiation faked (Option→List, Result→Any)
- [x] T3: No ADT types in type system
- [x] R4: Store handle_to_stored_value stub
- [x] R5: Non-atomic retain/release event counters
- [x] R6: spawn_named race condition
- [x] C5: value_hash return type mismatch (consumer removed)

### Medium (20+) — 5 RESOLVED
- [x] L1: `=`/`==` tokens removed from lexer (helpful errors, `is` and `.equals()` used instead)
- [x] L2: Float-dot ambiguity fixed (lookahead after `.`)
- [x] L3: Hex/octal/binary numeric literals added
- [x] L4: Unknown escape sequences now rejected with error
- [x] P5: synchronize() dead code — **REMOVED** (Session 9)
- [ ] P6: Single-error model (subsequent errors dropped)
- [ ] P7: peek_kind() clones String/Vec payloads
- [x] P8: Index/subscript syntax `expr[index]` implemented
- [x] P10: Trait method `-> Type` parsing removed — **DONE** (Session 9, no annotations per design)
- [x] A1: PersistenceMode dead code — **REMOVED** (Session 9)
- [x] A3: MatchPattern::List changed to Vec<MatchPattern> — **DONE** (Session 9)
- [ ] S5: None/Unit conflation
- [ ] S6: Member access falls back to Map constraint
- [ ] S8: Pipeline type inference discards left type
- [x] L10: Underscore separators in numbers added
- [ ] T4: Dual-track TypeEnv (scopes vs symbols)
- [x] T5: instantiate_generic leaks type params — **FIXED** (Session 9, removed dead writes)
- [x] R8: scan() accesses potentially freed values — fixed with notify_value_freed + lock-guarded checks
- [x] R9: drop_heap_value recursive stack overflow — iterative worklist
- [x] R10: Worker/timer threads never exit — ActorSystem::shutdown()
- [ ] R11: Single work queue contention
- [x] D1: Span has file ID — **DONE** (Session 9, `file_id: u32` field added)
- [x] D2: Diagnostic severity levels — **DONE** (Session 9, `Severity` enum added)
- [ ] ML1: Text-based export extraction
- [ ] ML2: No proper namespacing
