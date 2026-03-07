# Coral Language — Comprehensive Codebase Review

**Date:** February 16, 2026  
**Scope:** Complete codebase review — lexer, parser, AST, semantic analysis, type system, MIR, codegen, runtime, actor system, persistence, standard library, tests, documentation  
**Methodology:** Line-by-line source analysis, cross-reference against specs and docs, build verification  

---

## Executive Summary

Coral is an experimental language combining Python-like ergonomics with systems-level performance via LLVM. The Rust-based compiler (~15,000 LOC) and C-compatible runtime (~12,000 LOC) implement a tagged-value system with reference counting, an actor framework, and a persistent store. **248 of 249 tests pass.**

**Overall Assessment: Early Alpha (30% feature-complete)**

| Dimension | Rating | Summary |
|-----------|--------|---------|
| Language Design | ★★★★☆ | Novel error-as-attribute model, clean syntax, well-thought-out pipeline/placeholder system |
| Lexer | ★★★★☆ | Production-quality indent tracking; missing hex literals, has float-dot ambiguity |
| Parser | ★★★☆☆ | Solid recursive-descent; missing loops/if-else/return, single-error model |
| AST | ★★★★☆ | Clean structure; some dead fields (PersistenceMode, STORE_DEFAULT_FIELDS) |
| Semantic Analysis | ★★☆☆☆ | Constraint-based HM inference exists but is "best effort"; many paths fall to Any |
| Type System | ★★☆☆☆ | No ADT types, no tuples; Int/Float silently unify; generics never instantiated |
| MIR | ★☆☆☆☆ | Vestigial — cannot represent 80%+ of the language; only used for trivial const-eval |
| Codegen (LLVM) | ★★★☆☆ | Impressive breadth (stores, actors, closures, ADTs); 4 critical type-confusion bugs |
| Runtime | ★★★☆☆ | Well-designed value system; deadlock in cycle detector, data races, store FFI broken |
| Actor System | ★★★☆☆ | M:N scheduler with supervision; race conditions, no shutdown, single work queue |
| Persistent Store | ★★★☆☆ | Impressive architecture (WAL, binary, JSONL); stub FFI, tag mismatch bugs |
| Standard Library | ★★☆☆☆ | Good wrappers where runtime exists; many files aspirational or broken |
| Test Suite | ★★★☆☆ | 249 tests with decent breadth; zero runtime-execution tests, weak negative testing |
| Documentation | ★★★★☆ | Exceptional spec documents; honest about status |

---

## 1. Lexer Analysis — `src/lexer.rs` (734 lines)

### Architecture
Single-pass hand-written lexer with Python-style indent/dedent generation. 50+ token kinds including layout tokens, keywords, operators, and literals. Template strings use single quotes with `{expr}` interpolation.

### Strengths
- Indentation tracking is well-implemented with mixed-tab/space rejection
- Template string lexing with nested expression support
- Comprehensive operator coverage

### Critical Issues

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| L1 | **HIGH** | 652-661 | `=` and `==` produce identical `TokenKind::Equals` — assignment and equality are indistinguishable at the token level |
| L2 | **HIGH** | 218-223 | Float literal `42.` consumes the dot, stealing it from member access (`42.method()` breaks) |
| L3 | **MEDIUM** | 209-232 | No hex (`0x`), octal (`0o`), binary (`0b`) literals — essential for a systems language |
| L4 | **MEDIUM** | 323 | Unknown string escapes silently accepted (`\q` → `q`) — masks typos |
| L5 | **MEDIUM** | 274 | Bytes literal escapes truncate non-ASCII characters silently |
| L6 | **LOW** | 88-700 | 612-line monolithic `lex()` function — should be decomposed |
| L7 | **LOW** | 113 | Tab width hardcoded to 4 spaces (not configurable) |
| L8 | **LOW** | — | No `!=` (NotEqual) operator token; no `->` (Arrow) token |
| L9 | **LOW** | — | No multi-line string support (triple-quote or heredoc) |
| L10 | **LOW** | — | No underscore separators in numeric literals (`1_000_000`) |

---

## 2. Parser Analysis — `src/parser.rs` (2002 lines)

### Architecture
Recursive-descent with Pratt-style precedence for binary operators. Layout-sensitive with `layout_depth` tracking for indent/dedent pairing.

### Strengths
- Correct operator precedence hierarchy (13 levels)
- Clean pipeline operator integration
- Placeholder-to-lambda desugaring infrastructure

### Critical Issues

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| P1 | **HIGH** | — | **No `for`/`while`/`loop` constructs** — the language has no iteration |
| P2 | **HIGH** | — | **No `if`/`elif`/`else` blocks** — only ternary `cond ? then ! else` expressions |
| P3 | **HIGH** | — | **`return` keyword is lexed but never parsed** — dead feature |
| P4 | **HIGH** | 770-807 | `self.field is value` desugared to `self.set("field", value)` — fragile hack depending on `set` method existence |
| P5 | **MEDIUM** | 52-74 | `synchronize()` error recovery is dead code — no panic-mode recovery exists |
| P6 | **MEDIUM** | 1885 | Only first parse error is preserved; subsequent errors silently dropped |
| P7 | **MEDIUM** | 1946 | `peek_kind()` clones String/Vec payloads on every call — allocation-heavy |
| P8 | **MEDIUM** | — | No index/subscript syntax (`a[0]`) — must use `.get(0)` method |
| P9 | **MEDIUM** | 1694-1700 | Constructor vs binding in patterns based on capitalization heuristic — undocumented |
| P10 | **MEDIUM** | 599-601 | Trait method return types parsed but silently discarded |
| P11 | **LOW** | 282-284 | `parse_error_definition` doesn't track `layout_depth` for indent/dedent |
| P12 | **LOW** | 1861-1878 | `peek_is_binding` heuristic fails for type annotations with brackets |

---

## 3. AST Analysis — `src/ast.rs` (384 lines)

### Strengths
- Clean 22-variant Expression enum, 9-variant Item enum
- Good span tracking on all nodes
- Proper ADT representation (TypeDefinition, TypeVariant, VariantField)

### Issues

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| A1 | **MEDIUM** | 230 | `PersistenceMode` enum is dead code — never used by `StoreDefinition` |
| A2 | **MEDIUM** | 235-241 | `STORE_DEFAULT_FIELDS` declared but never injected into store definitions |
| A3 | **MEDIUM** | 341 | `MatchPattern::List` uses `Vec<Expression>` not `Vec<MatchPattern>` — breaks nested list destructuring |
| A4 | **LOW** | 162 | Multi-segment type paths (`Foo.Bar`) parsed but only `segments[0]` ever used |
| A5 | **LOW** | 278 | `Expression::Unit` returns `Span::default()` — zero-span at byte 0 is ambiguous |
| A6 | **LOW** | — | No `NotEquals` BinaryOp variant |
| A7 | **LOW** | — | No `Statement::For`/`Statement::While`/`Statement::If` variants |

---

## 4. Semantic Analysis — `src/semantic.rs` (1908 lines)

### Architecture
Two-pass analysis: (1) name collection + scope checking, (2) HM constraint generation + solving. Also performs mutability inference, exhaustiveness checking, and trait validation.

### Critical Issues

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| S1 | **HIGH** | 88 | First pass misses Type items — forward references to type names produce false "undefined name" errors |
| S2 | **HIGH** | 143-170 | All variant constructors typed as `Primitive::Any` — ADT type safety is effectively dynamic |
| S3 | **HIGH** | 160 | Variant constructor names are global with no namespace — two types with same variant name silently collide |
| S4 | **HIGH** | 126 | Store/type method bodies are **never scope-checked** — undefined names in methods go undetected |
| S5 | **MEDIUM** | 553 | `Expression::None` maps to `Primitive::Unit` — conflates absence and void |
| S6 | **MEDIUM** | 639 | Member access on concrete types falls back to Map constraint — incorrect for stores/types |
| S7 | **MEDIUM** | 700-706 | Constructor patterns don't constrain scrutinee type; nested bindings get `Unknown` |
| S8 | **MEDIUM** | 775-779 | Pipeline type inference discards left type — doesn't verify argument compatibility |
| S9 | **MEDIUM** | 867 | Inner bindings can't shadow outer bindings — unusual for lexical scoping |
| S10 | **LOW** | 1001-1035 | `is_builtin_name` is a massive hardcoded list; falls out of sync with runtime |
| S11 | **LOW** | 753-757 | Lambda constraint collection clones entire TypeEnv — O(n) per lambda |

---

## 5. Type System — `src/types/` (1444 lines total)

### Architecture
- **core.rs** (207 lines): `Primitive` (8 variants), `TypeId` (7 variants), `TypeVarId`
- **env.rs** (543 lines): Scoped type environment with mutable bindings, function registry
- **solver.rs** (676 lines): HM unification with constraint sets and union-find

### Critical Issues

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| T1 | **HIGH** | solver 450-452 | Int and Float **silently unify** — never catches int/float mismatches |
| T2 | **HIGH** | env 180-190 | Generic instantiation is hardcoded: `Option→List`, `Set→List`, `Result→Any` |
| T3 | **HIGH** | — | No ADT/sum types in the type system despite codegen supporting tagged values |
| T4 | **MEDIUM** | env dual-track | `TypeEnv` has both `scopes` (correct) and `symbols` (flat legacy map) — inconsistent |
| T5 | **MEDIUM** | env 172 | `instantiate_generic` leaks type param bindings into environment |
| T6 | **MEDIUM** | solver 463 | `solve_callable` allows fewer args for defaults but never checks default types |
| T7 | **LOW** | — | No tuple types, no set types, no proper result/option types |
| T8 | **LOW** | solver 266 | Constraint sorting clones the entire vector |

---

## 6. MIR — `src/mir*.rs` (428 lines total)

### Assessment: Vestigial

The MIR is **not on the main compilation path**. It's a side channel used only for constant-evaluating zero-arity function calls in global bindings. The real codegen walks the AST directly.

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| M1 | **HIGH** | mir_lower 133-134 | All comparison operators (`>`, `>=`, `<`, `<=`) silently mapped to `BinOp::Eq` |
| M2 | **HIGH** | mir_lower 63 | `match_operand_to_literal(Local(_))` returns Unit — computed values lost |
| M3 | **HIGH** | mir_lower 156 | Catch-all returns Unit for all unhandled expression types |
| M4 | **HIGH** | — | MIR only has 6 instruction types — cannot represent ~80% of the language |
| M5 | **MEDIUM** | mir_interpreter 60-66 | Function calls ignore argument values — always called with zero args |
| M6 | **MEDIUM** | — | No SSA, no phi nodes, no control flow, no type information |

---

## 7. LLVM Codegen — `src/codegen/` (4759 lines total)

### Architecture
- **mod.rs** (3830 lines): Core IR generation — all expression/statement forms, builtins, stores, actors, closures, ADTs, match, pipeline, error handling, inline asm
- **runtime.rs** (929 lines): 120+ `coral_*` FFI function declarations

### Strengths
- Impressive feature breadth (handles stores, actors, closures, ADTs, errors, pipelines, match, inline asm, unsafe blocks)
- Correct closure capture mechanism with upvalue boxing
- Working ADT tagged value construction and pattern matching

### Critical Bugs

| ID | Severity | Line | Issue |
|----|----------|------|-------|
| C1 | **CRITICAL** | mod 396 | `Statement::Return` converts value to f64 then returns as pointer — **type confusion, will segfault** |
| C2 | **CRITICAL** | mod 3598 | Actor handler dispatch passes f64 where `Value*` expected — **pointer corruption** |
| C3 | **CRITICAL** | mod 3608-3631 | Hash-based actor dispatch uses compile-time `DefaultHasher` vs different runtime hash — **switch cases never match** (>4 handlers) |
| C4 | **CRITICAL** | mod 3417 | `make_map` called with 0 args but declared with 2 — **argument mismatch → UB** |
| C5 | **HIGH** | runtime 407 | `value_hash` returns `i64` but codegen treats return as pointer — type mismatch |
| C6 | **MEDIUM** | mod 510-512 | `Expression::Throw` is completely unimplemented |
| C7 | **MEDIUM** | mod 2200-2203 | Return statements in lambdas are rejected |
| C8 | **MEDIUM** | — | No `for`/`while` loop codegen (no AST variants exist either) |
| C9 | **MEDIUM** | mod 1396-1402 | Five `unreachable!()` calls that will panic-abort the compiler |
| C10 | **LOW** | — | 500+ lines of duplicated boilerplate in `emit_builtin_call` |

---

## 8. Runtime — `runtime/src/` (~12,000 lines)

### Architecture

| Module | Lines | Purpose |
|--------|-------|---------|
| lib.rs | 4844 | Monolithic core: Value system, collections, FFI surface |
| actor.rs | 1059 | M:N actor scheduler, supervision, timers, named registry |
| store/ | ~4000 | Persistent store (engine, WAL, binary, JSONL, index, config, UUID7, FFI) |
| cycle_detector.rs | 550 | Bacon & Rajan cycle collection |
| weak_ref.rs | ~300 | Weak reference registry |
| symbol.rs | ~300 | String interning |
| memory_ops.rs | ~160 | Raw C-callable memory operations |
| Orphaned files | ~1600 | value.rs, memory.rs, collections/ — **never compiled** |

### Value System
32-byte tagged values with atomic refcount, inline string optimization (≤14 bytes), heap objects for collections/closures/actors. Value pool recycling with 8192-entry capacity.

### Critical Bugs

| ID | Severity | Location | Issue |
|----|----------|----------|-------|
| R1 | **CRITICAL** | cycle_detector.rs | `collect_white` calls `coral_value_release` while holding detector mutex → `possible_root` re-locks → **deadlock** |
| R2 | **CRITICAL** | symbol.rs | `coral_symbol_intern` misreads string memory layout (assumes `(len, data)` but heap strings are `Vec(ptr, len, cap)`) → **crash** |
| R3 | **CRITICAL** | store/ffi.rs | TAG constants off by one (`TAG_NUMBER=1` but `ValueTag::Number=0`) → **every value type misidentified** |
| R4 | **HIGH** | store/ffi.rs | `handle_to_stored_value` is a stub → store create/update **ignores all user data** |
| R5 | **HIGH** | lib.rs | `retain_events`/`release_events` are plain `u32` accessed from multiple threads → **data race (UB)** |
| R6 | **HIGH** | actor.rs | `spawn_named` spawns actor before checking name → actor leaks if name taken |
| R7 | **HIGH** | store/uuid7.rs | `LAST_TIMESTAMP` + `COUNTER` non-atomic update → UUID collision |
| R8 | **MEDIUM** | cycle_detector.rs | `scan()` accesses raw pointers to potentially freed values |
| R9 | **MEDIUM** | lib.rs | `drop_heap_value` is recursive → stack overflow for deeply nested structures |
| R10 | **MEDIUM** | actor.rs | Timer thread and scheduler workers never exit → thread leak on shutdown |
| R11 | **MEDIUM** | actor.rs | Single `Mutex<Receiver>` work queue → serialized task dequeue |
| R12 | **LOW** | lib.rs | MapObject linear probing with no resize strategy → O(n) lookup |
| R13 | **LOW** | lib.rs | SeqCst atomic ordering everywhere — Acquire/Release suffices |

### Orphaned Code
`value.rs` (607 lines), `memory.rs` (~480 lines), and `collections/` (~1100 lines) exist but are never compiled. Remnants of incomplete refactoring.

---

## 9. Persistent Store — `runtime/src/store/` (~4000 lines)

### Architecture
Dual binary + JSONL storage with WAL, hierarchical TOML config, primary index, UUID7.

### Strengths
- Well-structured into focused modules
- Custom minimal TOML parser with hierarchical config overrides
- CRC32 integrity checking in binary format
- UUID7 per RFC 9562

### Issues

| ID | Severity | Issue |
|----|----------|-------|
| ST1 | **CRITICAL** | Store FFI TAG mismatch (R3) — every value type misidentified |
| ST2 | **HIGH** | `handle_to_stored_value` stub — **stores cannot actually store data** |
| ST3 | **HIGH** | `coral_store_get_by_uuid` linear scan instead of index |
| ST4 | **MEDIUM** | `with_engine` opens new engine per call instead of handle lookup |
| ST5 | **MEDIUM** | Compression (LZ4/Zstd/Snappy) configured but never applied |
| ST6 | **MEDIUM** | Bloom filters, field indexes, cache eviction, backup — all configured but unimplemented |
| ST7 | **LOW** | Binary writer opens/closes file per write |

---

## 10. Standard Library — `std/`

### Compilation Status

| Status | Files |
|--------|-------|
| **Compiles** | prelude, math, string, bit, bytes, runtime/actor, runtime/memory, runtime/value |
| **Mostly compiles** | io, list |
| **Partially broken** | result (uses `for` loops), map/set (use `.equals(none).not()`) |
| **Stubs** | net (explicit stubs returning errors) |

### Critical Gaps
- Many runtime functions have no std wrapper (trig, abs, floor, ceil, round, error handling, tagged values, iterators, store FFI, weak refs, cycle detector, metrics)
- No `time`, `json`, `encoding` modules despite being in the stdlib spec
- `map.coral` and `set.coral` depend on unimplemented `.equals()` and `.not()` methods

---

## 11. Examples

| File | Compilable | Key Issues |
|------|-----------|------------|
| hello.coral | ✅ | None |
| calculator.coral | ✅ | None |
| data_pipeline.coral | ✅ | Manual unrolling (no loops) |
| traits_demo.coral | ✅ | Uses ADTs, not actual traits |
| fizzbuzz.coral | ❌ | Uses `while`, `$` implicit params |
| chat_server.coral | ❌ | Uses `loop`, `for`, networking, time |
| http_server.coral | ❌ | Uses `while`, arrow lambdas, JSON, networking |

---

## 12. Test Suite — 249 tests (248 pass, 1 fail)

### Coverage by Area

| Feature Area | Tests | Rating |
|-------------|-------|--------|
| ADT/Pattern Matching | 44 | ★★★★☆ |
| Error Handling | 25 | ★★★★☆ |
| Math Intrinsics | 31 | ★★★★☆ |
| Semantic Analysis | 29 | ★★★☆☆ |
| Traits | 19 | ★★★☆☆ |
| Modules | 15 | ★★★☆☆ |
| Pipeline Operator | 11 | ★★★☆☆ |
| Parser | 32 | ★★☆☆☆ |
| Lexer | 5 | ★★☆☆☆ |
| Codegen (direct) | 0 | ☆☆☆☆☆ |
| **Runtime Execution** | **0** | **☆☆☆☆☆** |
| MIR | 2 | ★☆☆☆☆ |
| CLI | 0 | ☆☆☆☆☆ |

### Critical Gap
**Zero tests execute compiled Coral code.** All 249 tests verify IR string output only. The compiler's LLVM output is never linked against the runtime and actually run. All critical codegen bugs (C1-C4) are undetected by the test suite.

---

## 13. Span & Diagnostics

| ID | Severity | Issue |
|----|----------|-------|
| D1 | **HIGH** | Span has no file ID — blocks multi-file diagnostics |
| D2 | **MEDIUM** | No severity levels (warning/info/error distinction) |
| D3 | **MEDIUM** | No error codes for programmatic handling |
| D4 | **LOW** | `CompileError::with_context` is dead code |
| D5 | **LOW** | `saturating_sub` missing on usize subtraction |

---

## 14. Module System — `src/module_loader.rs` (534 lines)

| ID | Severity | Issue |
|----|----------|-------|
| ML1 | **HIGH** | Text-based export extraction — comments/strings produce false matches |
| ML2 | **HIGH** | No proper namespacing — all names global after inclusion |
| ML3 | **MEDIUM** | Dependency hash uses XOR (self-cancelling) |
| ML4 | **MEDIUM** | Cache validates mtime but ignores computed content hash |

---

## 15. Cross-Cutting Architectural Issues

### 15.1 No Visitor Pattern
~10 independent full-tree traversals with near-identical match arms. A visitor trait would eliminate thousands of lines of duplication.

### 15.2 Type System Disconnected from Codegen
Types have `Int` vs `Float` but codegen treats everything as `f64`. Type inference results are not used to specialize code generation.

### 15.3 Boxed-Everything Runtime
Every value is 32-byte heap-allocated `CoralValue*`. All arithmetic: unbox → compute → re-box. No escape analysis, no stack allocation, no integer specialization.

### 15.4 Single-File Monoliths
Three files exceed 1000 LOC: runtime lib.rs (4844), codegen mod.rs (3830), semantic.rs (1908).

### 15.5 Orphaned/Dead Code (~3800 LOC)
- Runtime: value.rs, memory.rs, collections/ (never compiled)
- Runtime: map_hash.rs, module_registry.rs (dead scaffolds)
- AST: PersistenceMode, STORE_DEFAULT_FIELDS
- Parser: synchronize()
- Diagnostics: CompileError::with_context

### 15.6 MIR Is a Dead End
The MIR cannot represent most of the language and is only used for trivial constant evaluation. Real codegen walks the AST directly. This forecloses optimization opportunities.

---

## 16. Complete Bug Summary

### Critical (7) — Will Crash / Produce Wrong Results
| # | Component | Description |
|---|-----------|-------------|
| C1 | Codegen | Return converts f64 as pointer — segfault |
| C2 | Codegen | Actor handler dispatch f64/Value* confusion |
| C3 | Codegen | Hash-based dispatch never matches |
| C4 | Codegen | make_map 0-arg to 2-arg function |
| R1 | Runtime | Cycle detector deadlock |
| R2 | Runtime | Symbol interning memory layout crash |
| R3 | Runtime | Store FFI off-by-one tags |

### High (15) — Incorrect Behavior / Missing Critical Features
| # | Component | Description |
|---|-----------|-------------|
| P1 | Parser | No for/while/loop |
| P2 | Parser | No if/elif/else blocks |
| P3 | Parser | return keyword dead |
| P4 | Parser | self.field hack |
| S1 | Semantic | Forward ref failures for types |
| S2 | Semantic | Constructors typed as Any |
| S3 | Semantic | Constructor name collisions |
| S4 | Semantic | Methods never scope-checked |
| T1 | Types | Int/Float silently unify |
| T2 | Types | Generic instantiation faked |
| T3 | Types | No ADT types in type system |
| R4 | Store | handle_to_stored_value stub |
| R5 | Runtime | Data race on event counters |
| R6 | Actor | spawn_named race condition |
| C5 | Codegen | value_hash type mismatch |

### Medium (20+) — Incomplete / Degraded Functionality
### Low (15+) — Code Quality / Minor

**Total: ~57 identified issues across the codebase**

---

## 17. Metrics

| Metric | Value |
|--------|-------|
| Compiler LOC (Rust) | ~15,000 |
| Runtime LOC (Rust) | ~12,000 |
| Standard Library (Coral) | ~800 LOC |
| Tests | 249 (248 pass) |
| Documentation | ~10,000 LOC |
| Orphaned code | ~3,800 LOC |
| Critical bugs | 7 |
| Total issues | ~57 |
| Feature completeness | ~30% |
| Self-hosting readiness | ~25% |
