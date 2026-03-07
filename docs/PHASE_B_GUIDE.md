# Phase B Implementation Guide — Coral Compiler

**Purpose:** Self-contained guide for executing all Phase B tasks from a clean context.  
**Scope:** ~45 tasks across 6 parallel tracks: Stdlib, Actors, Stores, Type System, Infrastructure, Self-Hosted Front-End.  
**Prerequisite:** Phase A is complete — all 424 tests pass, 0 failures.

---

## Table of Contents

1. [Language Design Constraints](#1-language-design-constraints)
2. [Architecture Overview](#2-architecture-overview)
3. [File Map](#3-file-map)
4. [Key Data Structures](#4-key-data-structures)
5. [Build & Test Commands](#5-build--test-commands)
6. [Known Bugs to Fix](#6-known-bugs-to-fix)
7. [Track 1: Type System (TS-4, TS-5, TS-6, TS-9)](#7-track-1-type-system)
8. [Track 2: Standard Library (SL-4 through SL-16)](#8-track-2-standard-library)
9. [Track 3: Actor System (AC-1 through AC-5)](#9-track-3-actor-system)
10. [Track 4: Persistent Stores (PS-2 through PS-8)](#10-track-4-persistent-stores)
11. [Track 5: Infrastructure (IQ-1 through IQ-5)](#11-track-5-infrastructure)
12. [Track 6: Self-Hosted Front-End (SC-1 through SC-4)](#12-track-6-self-hosted-front-end)
13. [Execution Order](#13-execution-order)
14. [Verification Checklist](#14-verification-checklist)

---

## 1. Language Design Constraints

These are non-negotiable. Every change must respect all 10 rules:

1. **Pure type inference** — no type annotations in user code; all types inferred via constraint solving
2. **`is` for binding** — no `=` or `==`; `is` is the binding operator (`x is 5`)
3. **Method-based equality** — `.equals()` / `.not_equals()` instead of `==` / `!=`
4. **Single `Number(f64)` at runtime** — one numeric type; Int distinction is compile-time only
5. **Value-error model** — every value carries error/absence metadata via flags (bit 0 = ERR, bit 1 = ABSENT); no exceptions
6. **Indentation-based syntax** — Python-style blocks via INDENT/DEDENT tokens
7. **`*` marks functions** — `*foo(x)` defines a function
8. **`?`/`!` for ternary** — `condition ? then ! else`
9. **`~` for pipeline** — `value ~ fn1 ~ fn2` desugars to `fn2(fn1(value))`
10. **Actors are the concurrency primitive** — no shared mutable state; message passing only

---

## 2. Architecture Overview

### Compiler Pipeline

```
Source (.coral)
  → Lexer (src/lexer.rs)         → Vec<Token>
  → Parser (src/parser.rs)       → AST (Program)
  → Semantic (src/semantic.rs)   → SemanticModel (constraints, types, scopes)
  → Lower (src/lower.rs)         → Lowered AST (desugar $ placeholders)
  → Codegen (src/codegen/)       → LLVM IR via Inkwell (LLVM 16)
  → lli / llc+clang              → Execution / Binary
```

### Runtime

- Rust shared library (`libruntime.so`) loaded by `lli` or linked into binaries
- 213 FFI functions (`pub extern "C" fn coral_*`) across 6 files
- Reference counting with cycle detection (Bacon's algorithm)
- Actor M:N scheduler with bounded mailboxes (default 1024)
- Persistent store engine: WAL + binary/JSONL dual storage + B+ tree indexes

### E2E Test Flow

Tests compile Coral source → LLVM IR → run via `lli` with `libruntime.so` loaded. The test harness is in `tests/execution.rs` and related files. Each test asserts on stdout output.

---

## 3. File Map

### Compiler (`src/`)

| File | Lines | Purpose |
|------|-------|---------|
| `src/ast.rs` | 431 | AST node definitions (Item, Statement, Expression enums) |
| `src/lexer.rs` | ~900 | Indent-aware tokenizer |
| `src/parser.rs` | 2,151 | Recursive descent parser, 60 parse functions |
| `src/semantic.rs` | 2,303 | Semantic analysis: scope checking, constraint generation |
| `src/lower.rs` | ~200 | Lowering pass ($ placeholder desugaring) |
| `src/codegen/mod.rs` | 4,828 | LLVM IR codegen (largest file) |
| `src/codegen/runtime.rs` | 1,313 | FFI function declarations (RuntimeBindings struct) |
| `src/types/core.rs` | 210 | TypeId, Primitive enums |
| `src/types/env.rs` | 569 | TypeEnv (scope chain), FunctionRegistry |
| `src/types/solver.rs` | 658 | Constraint solver, unification, type graph |
| `src/module_loader.rs` | 522 | `use` directive expansion, module resolution |
| `src/diagnostics.rs` | ~100 | Diagnostic/Span types |
| `src/span.rs` | ~50 | Span definition |

### Runtime (`runtime/src/`)

| File | Lines | Purpose |
|------|-------|---------|
| `runtime/src/lib.rs` | 5,902 | Main runtime: 168 FFI functions, value ops |
| `runtime/src/actor.rs` | 1,095 | Actor system: spawn, send, supervision, timers |
| `runtime/src/store/ffi.rs` | 738 | Store FFI: 13 exported functions |
| `runtime/src/store/engine.rs` | ~600 | Storage engine |
| `runtime/src/store/wal.rs` | ~300 | Write-ahead log |
| `runtime/src/store/index.rs` | ~200 | B+ tree indexing |
| `runtime/src/cycle_detector.rs` | ~400 | Bacon's cycle collection |
| `runtime/src/weak_ref.rs` | 309 | Weak reference registry |
| `runtime/src/memory_ops.rs` | ~200 | Memory operations (15 FFI) |
| `runtime/src/symbol.rs` | ~100 | Symbol interning (5 FFI) |

### Standard Library (`std/`)

| File | Lines | Status | Key Missing |
|------|-------|--------|-------------|
| `std/prelude.coral` | 63 | Partial | `not()`, `flip()`, `curry()` |
| `std/math.coral` | 166 | Good | sqrt/pow/log/trig via libm (already wired in Phase A) |
| `std/io.coral` | 107 | Partial | Most FFI stubs; read/write_file may work |
| `std/process.coral` | 50 | Partial | Most need FFI wiring |
| `std/list.coral` | 144 | Good | `zip_with()`, `scan()`, `windows()` |
| `std/map.coral` | 84 | Partial | `map_keys()`, `flat_map()`, iteration |
| `std/set.coral` | 33 | Partial | intersection, difference, is_subset |
| `std/string.coral` | 69 | Good | `chars()`, `lines()` |
| `std/char.coral` | 104 | Complete | Used by self-hosted lexer |
| `std/option.coral` | 74 | Good | `zip()`, `inspect()` |
| `std/result.coral` | 121 | Good | `?` operator integration |
| `std/bytes.coral` | 30 | Partial | `from_hex()`, `contains()`, `find()` |
| `std/bit.coral` | 29 | Partial | `count_zeros()`, leading/trailing |
| `std/net.coral` | 12 | Stub | TCP not functional |
| `std/runtime/actor.coral` | 85 | Working | Thin FFI wrappers |
| `std/runtime/memory.coral` | 63 | Working | Thin FFI wrappers |
| `std/runtime/value.coral` | 53 | Working | Thin FFI wrappers |

### Self-Hosted (`self_hosted/`)

| File | Lines | Status |
|------|-------|--------|
| `self_hosted/lexer.coral` | 489 | ~92% complete, compiles to IR |
| `self_hosted/parser.coral` | 1,678 | ~82% complete, compiles to IR |

### Tests (`tests/`)

27 test crates, 424 total tests. Key files:

| File | Tests | Focus |
|------|-------|-------|
| `tests/execution.rs` | ~121 | E2E: compile → lli → check stdout |
| `tests/traits.rs` | 19 | Trait system |
| `tests/self_hosting.rs` | 3 | Lexer/parser compilation |
| `tests/parser_*.rs` | ~40 | Parser unit tests |
| `tests/semantic.rs` | ~30 | Semantic analysis |
| `tests/lexer_*.rs` | ~20 | Lexer tests |

---

## 4. Key Data Structures

### AST (src/ast.rs)

**Expression enum** (28 variants):
```
Unit, None(Span), Identifier(String, Span), Integer(i64, Span), Float(f64, Span),
Bool(bool, Span), String(String, Span), Bytes(Vec<u8>, Span), Placeholder(u32, Span),
TaxonomyPath, Throw, Lambda, List, Map, Binary, Unary, Call, Member, Ternary,
Pipeline, ErrorValue, ErrorPropagate, Match, InlineAsm, PtrLoad, Unsafe, Index
```

**Statement enum** (9 variants):
```
Binding, Expression, Return, If, While, For, FieldAssign, Break, Continue
```

**Item enum** (9 variants):
```
Binding, Function, ExternFunction, Type, Store, Taxonomy, ErrorDefinition,
TraitDefinition, Expression
```

**StoreDefinition**: `{ name, with_traits, fields, methods, is_actor, is_persistent, span }`

**FunctionKind**: `Free | Method | ActorMessage`

**MatchPattern** (7 variants):
```
Integer(i64), Bool(bool), Identifier(String), String(String),
List(Vec<MatchPattern>), Constructor { name, fields, span }, Wildcard(Span)
```

### Type System (src/types/)

**TypeId enum** (8 variants):
```
Primitive(Primitive), List(Box<TypeId>), Map(Box<TypeId>, Box<TypeId>),
Func(Vec<TypeId>, Box<TypeId>), Placeholder(u32), TypeVar(TypeVarId),
Adt(String, Vec<TypeId>), Unknown
```

**Primitive enum** (9 variants): `Int, Float, Bool, String, Bytes, Unit, None, Any, Actor`

**TypeEnv**: Scope-chain based; `scopes: Vec<Scope>`, generic types registered (`List["T"]`, `Map["K","V"]`, `Set["T"]`, `Option["T"]`, `Result["T","E"]`).

**TypeGraph** (solver.rs): Union-find with path compression. `fresh()` creates new TypeVar, `unify()` merges types, `resolve()` follows bindings.

**ConstraintKind** (10 variants): `Equal/EqualAt`, `Numeric/NumericAt`, `Boolean/BooleanAt`, `Iterable/IterableAt`, `Callable/CallableAt`.

### Codegen (src/codegen/mod.rs)

**CodeGenerator struct** key fields:
```rust
runtime: RuntimeBindings<'ctx>,           // all FFI function references
functions: HashMap<String, FunctionValue>,
string_pool: HashMap<String, GlobalValue>,
store_methods: HashMap<String, (String, usize)>,  // method_name → (store_name, param_count)
store_constructors: HashSet<String>,               // e.g. "make_Counter"
store_field_names: HashSet<String>,                // all known store field names
persistent_stores: HashSet<String>,                // persistent store names
enum_constructors: HashMap<String, (String, usize)>, // ctor → (enum_name, field_count)
```

### SemanticModel (src/semantic.rs)

```rust
pub struct SemanticModel {
    pub globals: Vec<Binding>,
    pub functions: Vec<Function>,
    pub extern_functions: Vec<ExternFunction>,
    pub stores: Vec<StoreDefinition>,
    pub type_defs: Vec<TypeDefinition>,
    pub trait_defs: Vec<TraitDefinition>,
    pub error_defs: Vec<ErrorDefinition>,
    pub constraints: ConstraintSet,
    pub types: TypeEnv,
    pub mutability: MutabilityEnv,
    pub allocation: AllocationHints,
    pub usage: UsageMetrics,
    pub warnings: Vec<Diagnostic>,
}
```

### Parser (src/parser.rs)

```rust
pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
    source_len: usize,
    pending_error: Option<Diagnostic>,
    layout_depth: usize,
}
type ParseResult<T> = Result<T, Diagnostic>;
```

Single-error model: returns `Err(Diagnostic)` on first parse failure. No recovery.

---

## 5. Build & Test Commands

```bash
# Build compiler
cargo build 2>&1 | tail -5

# Build runtime (required when adding FFI functions)
cd runtime && cargo build && cd ..

# Run all tests
cargo test 2>&1 | tail -20

# Run specific test crate
cargo test --test execution 2>&1 | tail -20
cargo test --test self_hosting 2>&1 | tail -10

# Compile a Coral file to IR
cargo run -- examples/hello.coral > /tmp/hello.ll 2>&1

# Run compiled IR
lli /tmp/hello.ll

# Check test count
cargo test 2>&1 | grep "test result"

# Rebuild runtime after changes (CRITICAL — lli loads libruntime.so)
cd runtime && cargo build && cd ..
```

**Important:** After adding any `pub extern "C" fn coral_*` function to the runtime, you must:
1. Add the function in `runtime/src/lib.rs` (or appropriate submodule)
2. Re-export it if in a submodule (check `runtime/src/lib.rs` top-level `pub use`)
3. Rebuild runtime: `cd runtime && cargo build && cd ..`
4. Declare it in `src/codegen/runtime.rs` RuntimeBindings struct
5. Wire it in `src/codegen/mod.rs` where needed

---

## 6. Known Bugs to Fix

These bugs are scoped within Phase B tasks. Fix them as part of the referenced task.

### P6 — Single-Error Parser (→ TS-6)

**Problem:** Parser returns `Result<T, Diagnostic>` — bails on first error. `pending_error` field exists but only stores one error.

**Location:** `src/parser.rs` L6-14, L45-48

**Fix approach:** Change to `Vec<Diagnostic>`, add synchronization points (skip tokens until next statement/item boundary after error), continue parsing. Return accumulated errors at the end.

### S6 — Member Access Type Inference (→ TS-4)

**Problem:** All member access (`.field`) generates a Map constraint. No Store-specific or ADT-specific member type checking.

**Location:** `src/semantic.rs` L1207 — only checks target expression, never validates property name.

**Fix approach:** When target type is known (Store, ADT), generate specific constraints for the field type. Fall back to Map constraint only for unknown targets.

### S8 — Pipeline Type Inference (→ TS-5)

**Problem:** Pipeline `a ~ f` collects constraints for both sides but never unifies left side's type with `f`'s first parameter.

**Location:** `src/semantic.rs` L1013-1020 — just returns right side's type, left side's type is discarded.

**Fix approach:** When right side is `Call` or `Identifier` (function), generate a `Callable` constraint that includes the left side's type as the first parameter. Or: desugar pipeline in semantic analysis (not just codegen) so type inference sees the actual call.

### R11 — Single Work Queue (→ AC-5)

**Problem:** All actor tasks go through one work queue, causing contention.

**Location:** `runtime/src/actor.rs` — single `SyncSender` per scheduler.

**Fix approach:** Per-worker channels with work-stealing (crossbeam-deque or similar).

### ML1/ML2 — Text Exports + No Namespacing (→ IQ-1)

**Problem:** `extract_exports()` in module_loader uses text pattern matching (`*name(` etc.). Modules are spliced as raw text — no namespacing, no selective imports.

**Location:** `src/module_loader.rs` L139-187 (extract_exports), L269+ (load_recursive).

**Fix approach:** Parse `use` directives at AST level. Support `use std.map { get_key, set_key }` for selective imports and `use std.map as m` for qualified access. Requires parser changes + module loader rewrite.

---

## 7. Track 1: Type System

### TS-4 — Fix Member Access Type Inference (S6 bug)

**Priority:** High | **Est:** 10h | **Depends:** None

**Goal:** When `.field` is accessed on a known Store or ADT type, generate specific typed constraints instead of falling back to generic Map access.

**Current behavior:**
- `src/semantic.rs` L1207: `Expression::Member { target, .. }` only recursively checks the target, never validates the property.
- `src/codegen/mod.rs` L1283-1350: `emit_member_expression` dispatches on field names like `"length"`, `"err"`, `"size"`, but defaults to `map_get` for everything else.

**Implementation:**
1. In `src/semantic.rs`, expand the `Expression::Member` handler:
   - Resolve the target's type
   - If it's `Adt(name, _)`, look up the type definition's fields and generate an `Equal` constraint between the member reference and the field's type
   - If it's a known store type, look up the store definition's fields
   - If type is unknown/TypeVar, fall back to current Map-based behavior
2. Add field type tracking to `SemanticModel` — a map from `(TypeName, FieldName) → TypeId`
3. Update codegen to use typed dispatch when the type is known

**Test:** Write tests in `tests/semantic.rs` that verify:
- Accessing a valid store field returns correct type
- Accessing a nonexistent field on a typed ADT produces a warning/error
- Map access still works for dynamic maps

---

### TS-5 — Fix Pipeline Type Inference (S8 bug)

**Priority:** Medium | **Est:** 5h | **Depends:** None

**Goal:** `x ~ f ~ g` should propagate types correctly: `x: T`, `f: T → U`, `g: U → V`, result: `V`.

**Current behavior:**
- `src/semantic.rs` L1013-1020: Collects constraints for left and right, returns right's type. Left type is discarded.
- `src/codegen/mod.rs` L983-1021: Properly desugars `a ~ f(args)` to `f(a, args)` — codegen is correct, semantic analysis is wrong.

**Implementation:**
1. In `src/semantic.rs`, handle `Expression::Pipeline { left, right, .. }`:
   - Collect constraints for `left`, get its type
   - If `right` is `Call { callee, args }`:
     - Create a desugared call with `left` prepended to args
     - Collect constraints as if it were a `Call` expression
   - If `right` is `Identifier(name)`:
     - Generate `Callable` constraint: `name: Func([left_type], result_var)`
     - Return `result_var` as the pipeline's type
   - If `right` is another `Pipeline`, recurse (already handled by left-to-right parsing)
2. Essentially: mirror the codegen desugaring but in the semantic pass

**Test:** Write E2E test:
```coral
*double(x)
    x * 2
*add_one(x)
    x + 1
result is 5 ~ double ~ add_one
println(result)
# Expected output: 11
```

---

### TS-6 — Multi-Error Recovery (P6 bug)

**Priority:** Medium | **Est:** 15h | **Depends:** None

**Goal:** Parser reports all syntax errors in a file, not just the first one.

**Current behavior:**
- `type ParseResult<T> = Result<T, Diagnostic>` — single error
- `pending_error: Option<Diagnostic>` — stores at most one
- `parse_expression()` at L924 uses `?` propagation — bails immediately

**Implementation:**
1. Add `errors: Vec<Diagnostic>` to Parser struct
2. Create a `synchronize()` method that skips tokens until a known synchronization point:
   - For items: skip until `Star | KeywordType | KeywordEnum | KeywordStore | KeywordActor | KeywordErr | KeywordTrait | KeywordExtern | Eof`
   - For statements: skip until `Newline | Indent | Dedent | Eof` + the item sync points
3. In `parse()` main loop, wrap item parsing in error recovery:
   ```rust
   match self.parse_item() {
       Ok(item) => items.push(item),
       Err(diag) => {
           self.errors.push(diag);
           self.synchronize_to_item();
       }
   }
   ```
4. Change return type: `parse()` returns `Result<Program, Vec<Diagnostic>>` or returns `(Program, Vec<Diagnostic>)` with partial AST
5. Update all callers in `src/compiler.rs` and test harnesses

**Test:** Create a Coral file with 3 syntax errors; verify all 3 are reported.

---

### TS-9 — Exhaustiveness Checking for Nested ADTs

**Priority:** Medium | **Est:** 5h | **Depends:** TS-1 (completed in Phase A)

**Goal:** Pattern matching on nested ADTs (e.g., `Option[List[Int]]`) checks all constructors at every nesting level.

**Current behavior:** Match exhaustiveness likely only checks top-level variants.

**Location:** Search `src/codegen/mod.rs` for exhaustiveness or match compilation logic. The match compilation is scattered across codegen — look for `build_match` or similar functions.

**Implementation:**
1. Build a pattern matrix from all match arms
2. For each constructor in the ADT, check if at least one arm covers it
3. Recursively check nested constructor patterns
4. If not exhaustive, emit a warning (not error, to stay permissive) listing uncovered patterns

**Test:** Write a match on `Option` with only `Some(_)` arm — should warn about missing `None`.

---

## 8. Track 2: Standard Library

### SL-4 — Complete `set.coral`

**Priority:** Medium | **Est:** 4h | **Depends:** None

**Goal:** Add `intersection`, `difference`, `symmetric_difference`, `is_subset`, `is_superset`.

**Context:** Sets are maps with `true` values. All operations use map primitives. Current functions: `empty_set()`, `add(s, elem)`, `has(s, elem)`, `size(s)`, `is_empty(s)`, `elements(s)`, `singleton(elem)`.

**Implementation guidance:**
```coral
*intersection(a, b)
    result is empty_set()
    for elem in elements(a)
        has(b, elem) ? result is add(result, elem) ! result
    result

*difference(a, b)
    result is empty_set()
    for elem in elements(a)
        has(b, elem) ? result ! result is add(result, elem)
    result

*symmetric_difference(a, b)
    union is merge_sets(a, b)          # needs merge helper
    common is intersection(a, b)
    difference(union, common)

*is_subset(a, b)
    for elem in elements(a)
        has(b, elem) ? true ! return false
    true

*is_superset(a, b)
    is_subset(b, a)
```

**Runtime FFI needed:** None — builds on existing `has_key`, `map_keys` via map operations.

**Test:** Add to `tests/execution.rs`.

---

### SL-5 — Complete `map.coral`

**Priority:** Medium | **Est:** 4h | **Depends:** None

**Goal:** Add `map_keys`, `flat_map`, `group_by`, map iteration.

**Context:** Current functions wrap builtins: `get_key`, `set_key`, `len`, `empty`, `keys_list`, `has`, `get_or`, `remove_key`, `values_list`, `entries_list`, `merge_maps`, `map_values_fn`, `filter_entries`, `count_entries`, `from_entries`.

**Runtime FFI available:** `coral_map_keys` (L2849), `coral_map_values` (L4331), `coral_map_entries`, `coral_map_merge`.

**Implementation:** `map_keys(m, f)` maps a function over keys (returns new map), `flat_map(m, f)` where `f` returns a map and results merge, `group_by(list, key_fn)` groups list elements by key function.

---

### SL-6 — Add `string.coral` Iteration

**Priority:** Medium | **Est:** 3h | **Depends:** None

**Goal:** `chars(s)` and `lines(s)` functions.

**Runtime FFI:** `coral_string_to_chars` EXISTS at `runtime/src/lib.rs` L3067. `coral_string_lines` does NOT exist — must be added.

**Implementation:**
1. `chars(s)` — call extern `coral_string_to_chars` (returns list of single-char strings)
2. `lines(s)` — two options:
   - (a) Add `coral_string_lines` to `runtime/src/lib.rs` that splits on `\n`
   - (b) Implement in pure Coral using `divide(s, "\n")` from existing `string.coral`
   - Option (b) is simpler if `divide` (split) works correctly

**Extern declaration needed** (if adding runtime function):
```rust
// runtime/src/lib.rs
#[no_mangle]
pub extern "C" fn coral_string_lines(s: ValueHandle) -> ValueHandle {
    // split on \n, return list of strings
}
```

Then declare in `src/codegen/runtime.rs` and wire in codegen.

---

### SL-7 — Add `bytes.coral` Operations

**Priority:** Low | **Est:** 3h | **Depends:** None

**Goal:** `from_hex`, `contains`, `find`.

**Implementation:** These need new runtime FFI functions in `runtime/src/lib.rs`:
- `coral_bytes_from_hex(s: ValueHandle) -> ValueHandle` — parse hex string to bytes
- `coral_bytes_contains(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle` — subsequence search
- `coral_bytes_find(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle` — find offset or -1

---

### SL-8 — Create `std/json.coral`

**Priority:** High | **Est:** 10h | **Depends:** None

**Goal:** JSON parse/serialize.

**Spec functions (from STANDARD_LIBRARY_SPEC):**
- `parse_json(s)` → Coral map/list/number/string/bool/none
- `to_json(value)` → JSON string
- `to_json_pretty(value)` → indented JSON string
- `json_get(obj, path)` → nested access via dot path
- `json_set(obj, path, value)` → set nested value

**Implementation approach:**
1. Add runtime FFI functions `coral_json_parse` and `coral_json_serialize` in `runtime/src/lib.rs` using `serde_json` (add to `runtime/Cargo.toml`)
2. JSON naturally maps to Coral's runtime values: `{}` → Map, `[]` → List, `"str"` → String, `123` → Number, `true`/`false` → Bool, `null` → None/Unit
3. `json_get` and `json_set` can be pure Coral using map/list access with path splitting

**Runtime addition:**
```rust
// runtime/src/lib.rs
#[no_mangle]
pub extern "C" fn coral_json_parse(s: ValueHandle) -> ValueHandle { ... }

#[no_mangle]
pub extern "C" fn coral_json_serialize(v: ValueHandle) -> ValueHandle { ... }

#[no_mangle]
pub extern "C" fn coral_json_serialize_pretty(v: ValueHandle) -> ValueHandle { ... }
```

Add `serde_json = "1"` to `runtime/Cargo.toml`.

---

### SL-9 — Create `std/time.coral`

**Priority:** Medium | **Est:** 6h | **Depends:** None

**Goal:** Date/time operations.

**Spec functions:** `now()`, `timestamp()`, `year/month/day/hour/minute/second(dt)`, `format_datetime(dt, pattern)`, `format_iso(dt)`, `seconds(n)`, `minutes(n)`, `hours(n)`, `days(n)`.

**Implementation:**
1. Add `coral_time_now` (returns Unix timestamp ms as Number), `coral_time_format` to runtime
2. Use libc `gettimeofday` or Rust's `std::time::SystemTime`
3. Duration can be represented as plain numbers (milliseconds)
4. Date components via `gmtime`/`localtime` from libc or Rust chrono
5. For minimal approach (no chrono dependency): store timestamps as Numbers, expose epoch-based functions

---

### SL-10 — Create `std/fmt.coral`

**Priority:** Medium | **Est:** 5h | **Depends:** None

**Goal:** String formatting utilities.

**Implementation:** Pure Coral is feasible for most formatting:
- `pad_start(s, len, char)`, `pad_end(s, len, char)` — using string ops
- `repeat_str(s, n)` — loop concatenation
- `format_number(n, decimals)` — manual formatting
- `join(list, separator)` — reduce with separator
- Can mostly build on existing `string.coral` functions

---

### SL-11 — Create `std/sort.coral`

**Priority:** Low | **Est:** 4h | **Depends:** None

**Goal:** Comparison-based sorting.

**Runtime check:** Look for `coral_list_sort` or similar. If it doesn't exist, implement merge sort in pure Coral or add a runtime FFI.

**Implementation options:**
- (a) Pure Coral merge sort using list slicing — works but slower
- (b) Add `coral_list_sort(list, compare_fn)` in runtime — faster but more complex FFI

---

### SL-12 — Create `std/encoding.coral`

**Priority:** Medium | **Est:** 6h | **Depends:** None

**Goal:** Base64 and hex encoding/decoding.

**Spec:** `base64_encode(data)`, `base64_decode(s)`, `hex_encode(data)`, `hex_decode(s)`.

**Implementation:**
1. Add runtime FFI functions — base64 and hex are byte-manipulation heavy, best done in Rust
2. Use `base64` crate in runtime (add to `runtime/Cargo.toml`)
3. Or implement in pure Coral using `bytes.coral` — possible but tedious

---

### SL-13 — Complete `net.coral` (TCP)

**Priority:** High | **Est:** 20h | **Depends:** None

**Goal:** TCP client/server via runtime FFI.

**Current state:** `net.coral` has only 2 stub functions returning error values.

**Implementation:**
1. Add runtime FFI functions using Rust's `std::net`:
   - `coral_tcp_listen(host, port)` → listener handle
   - `coral_tcp_accept(listener)` → connection handle
   - `coral_tcp_connect(host, port)` → connection handle
   - `coral_tcp_read(conn, n)` → bytes
   - `coral_tcp_write(conn, data)` → bytes written
   - `coral_tcp_close(conn)` → unit
2. Store TCP connections as opaque handles (actor-like references)
3. Wire in codegen as builtins or extern functions
4. This is the largest stdlib task — plan for incremental delivery

---

### SL-14 — Error Propagation Operator (`?`)

**Priority:** High | **Est:** 15h | **Depends:** TS-1 (completed)

**Goal:** `expr ! return err` and potentially `?` syntax for Result/Option propagation.

**Spec (from VALUE_ERROR_MODEL.md):**
- `foo = do_something(bar) ! return err` — if error, propagate immediately
- Automatic propagation: `x = err NotFound` → `y = x + 5` → `y` is also error
- Value intrinsics: `value.is_ok`, `value.is_err`, `value.is_absent`, `value.or(default)`

**AST support already exists:**
- `Expression::ErrorPropagate { expr, span }` — at `src/ast.rs`
- `Expression::ErrorValue { path, span }` — at `src/ast.rs`

**Implementation:**
1. Parser already handles `! return err` → `ErrorPropagate` (verify)
2. In codegen, `ErrorPropagate` should:
   - Emit the inner expression
   - Check `is_err` flag
   - If error: return the error value from the current function
   - If ok: continue with the value
3. Ensure builtin `is_err` check uses the value-error flag system
4. Wire `value.is_ok`, `value.is_err`, `value.or(default)` as member expressions in codegen

---

### SL-15 — Create `std/testing.coral`

**Priority:** Medium | **Est:** 5h | **Depends:** None

**Goal:** Assertion functions for Coral-level testing.

**Spec:** `assert(cond, msg)`, `assert_eq(actual, expected)`, `assert_ne(actual, expected)`, `assert_true(v)`, `assert_false(v)`, `assert_error(v)`, `fail(msg)`.

**Implementation:** Pure Coral using `println` for output and `process.exit(1)` for failure:
```coral
*assert_eq(actual, expected)
    actual.equals(expected) ? true ! panic("Assertion failed: expected " + str(expected) + ", got " + str(actual))
```

Note: Requires `panic` or some abort mechanism. Could use `process.exit(1)` with error message.

---

### SL-16 — Stdlib Test Suite

**Priority:** Medium | **Est:** 12h | **Depends:** SL-4 through SL-15

**Goal:** Create `tests/stdlib.rs` exercising each module.

**Implementation:** Follow the pattern in `tests/execution.rs`:
1. Create `tests/stdlib.rs`
2. For each stdlib module, write Coral source as string literals
3. Compile to IR, run with `lli`, assert stdout
4. Cover: set operations, map operations, string iteration, json parse/serialize, time functions, encoding, etc.

---

## 9. Track 3: Actor System

### AC-1 — Typed Messages

**Priority:** High | **Est:** 10h | **Depends:** TS-1 (completed)

**Goal:** `@messages(MessageType)` annotation enables compile-time type checking at `send()` sites.

**Spec (from ACTOR_SYSTEM_COMPLETION.md):**
- Syntax: `@messages(ChatMessage)` on actor definition
- Semantic: track message type in `SemanticModel`, check at `send()` call sites
- Implementation: `check_actor_send()` function in semantic analysis

**Context:**
- Actors are `StoreDefinition` with `is_actor: true` (`src/ast.rs` L171)
- Actor message handlers have `FunctionKind::ActorMessage` (`src/ast.rs` L91)
- Store/actor analysis at `src/semantic.rs` L228-253

**Implementation:**
1. **Parser:** Parse `@messages(TypeName)` annotation before actor definition (similar to `@handler`). Add to AST: new field `message_type: Option<String>` on `StoreDefinition`
2. **Semantic:** When analyzing actor definitions with `message_type`:
   - Look up the type definition
   - At every `send(actor, name, payload)` call site, verify `payload` matches the declared message type
   - Store actor message types in a map: `actor_name → TypeId`
3. **Codegen:** No changes needed — messages are already sent as values

**Test:** Create actor with `@messages(MyMsg)`, attempt `send` with wrong type → compile error.

---

### AC-2 — Actor Monitoring

**Priority:** High | **Est:** 8h | **Depends:** None

**Goal:** `monitor(actor)` / `demonitor(actor)` + `ActorDown` message on actor termination.

**Spec:**
- `monitor(worker)` — subscribe to death notifications
- `@down(msg)` handler — receives `ActorDown` message
- `demonitor(worker)` — unsubscribe

**Runtime implementation (`runtime/src/actor.rs`):**
1. Add `monitors: HashMap<ActorId, HashSet<ActorId>>` to `ActorSystem` (maps monitored → set of monitors)
2. `coral_actor_monitor(watcher, watched)` — register
3. `coral_actor_demonitor(watcher, watched)` — unregister
4. When an actor dies, iterate its monitors and send `ActorDown { actor_id, reason }` message
5. Wire in codegen: builtins `monitor` and `demonitor`

---

### AC-3 — Supervision Hardening

**Priority:** High | **Est:** 10h | **Depends:** None

**Goal:** Enforce restart budget, time windows, escalation chains.

**Current state (`runtime/src/actor.rs`):**
- `SupervisionStrategy` enum exists: `Restart, Stop, Escalate, Resume`
- `SupervisionConfig` exists: `max_restarts`, `restart_window_secs`
- `RestartTracker` exists: `restart_times`, `total_restarts`
- `SupervisedChild` exists: factory, config, tracker

**Implementation:**
1. Verify `RestartTracker` actually enforces the budget (check if restarts within the time window exceed `max_restarts`) — may need to implement the window check
2. Escalation: when a supervisor exhausts its restart budget, send `ChildFailure` to its parent
3. Add `@supervision(strategy: restart, max_restarts: 3, window: 60)` annotation parsing
4. Wire supervision config from AST through codegen to the runtime `spawn_supervised` call

---

### AC-4 — Graceful Actor Stop

**Priority:** Medium | **Est:** 4h | **Depends:** None

**Goal:** Flush mailbox before termination.

**Implementation (`runtime/src/actor.rs`):**
1. Add `Message::GracefulStop` variant to the `Message` enum
2. When received, drain remaining messages from the channel, process each, then terminate
3. Add `coral_actor_stop(actor_id)` FFI function
4. Wire as builtin `stop(actor)` in codegen

---

### AC-5 — Work-Stealing Scheduler (R11 bug)

**Priority:** Medium | **Est:** 8h | **Depends:** None

**Goal:** Replace single work queue with per-worker channels.

**Implementation (`runtime/src/actor.rs`):**
1. Add `crossbeam-deque` to `runtime/Cargo.toml`
2. Replace the single `SyncSender/Receiver` pair with per-worker `crossbeam_deque::Worker` queues
3. Each worker has a local deque; when empty, steal from other workers
4. Spawn falls back to round-robin assignment to worker queues
5. This is a runtime-only change — no compiler changes needed

---

## 10. Track 4: Persistent Stores

### PS-2 — Store Query Syntax

**Priority:** High | **Est:** 12h | **Depends:** PS-1 (completed)

**Goal:** Language-level query syntax for filtering/finding store records.

**Spec syntax (from PERSISTENT_STORE_SPEC.md):**
```coral
query Store
    where condition
    select fields
    order_by field asc
    limit n
```

**Implementation:**
1. **Lexer:** Add keywords `query`, `where`, `select`, `order_by`, `limit`, `offset`, `include_deleted`
2. **AST:** Add `Expression::Query { store, clauses, span }` with `QueryClause` enum
3. **Parser:** Parse query expression — `parse_query_expression()`
4. **Semantic:** Validate store name exists, field names exist, types match
5. **Runtime:** Add `coral_store_query(store, filter_fn, sort_field, sort_dir, limit, offset)` in `runtime/src/store/ffi.rs`
6. **Codegen:** Compile query expression into runtime call with filter closure

**Simpler alternative:** Start with function-based API:
```coral
results is store_query(MyStore, *filter(record) record.age > 18, "name", "asc", 10)
```

---

### PS-4 — Store Indexing from Language Level

**Priority:** Medium | **Est:** 8h | **Depends:** PS-1 (completed)

**Goal:** Expose B+ tree index creation and accelerated lookups.

**Implementation:**
1. `@index` annotation on store fields (parser support)
2. At store construction, call `coral_store_create_index(store, field_name)`
3. Queries automatically use indexes when filtering on indexed fields

---

### PS-5 — ACID Transactions

**Priority:** Medium | **Est:** 15h | **Depends:** PS-1 (completed)

**Goal:** Multi-operation atomic commits with isolation.

**Spec syntax:**
```coral
transaction
    user is MyStore.create(name: "Alice")
    MyStore.update(user, balance: 100)
    # Both succeed or both roll back
```

**Implementation:**
1. **Lexer/Parser:** Add `transaction` keyword, parse block
2. **Runtime:** Add `coral_transaction_begin`, `coral_transaction_commit`, `coral_transaction_rollback` FFI
3. **WAL integration:** WAL already has TXN_BEGIN/COMMIT/ROLLBACK entry types (defined in spec)
4. **Codegen:** Emit begin at block start, commit at end, rollback on error

---

### PS-6 — WAL Recovery Verification

**Priority:** Medium | **Est:** 4h | **Depends:** PS-1 (completed)

**Goal:** Write data → simulate crash → recover → verify integrity.

**Implementation:** Rust-level integration test in `runtime/`:
1. Open store, create records, write to WAL
2. Drop store without clean shutdown (simulating crash)
3. Re-open store, verify WAL replay produces correct data
4. Verify data integrity matches pre-crash state

---

### PS-7 — Fix WeakRef Clone Semantics

**Priority:** Medium | **Est:** 5h | **Depends:** None

**Goal:** WeakRef clones should have independent lifetimes.

**Location:** `runtime/src/weak_ref.rs` L52 — `WeakRef` struct. Current `Clone` may share registry IDs.

**Implementation:**
1. Audit `WeakRef::clone()` — ensure each clone increments `weak_count` in the registry
2. On drop, decrement `weak_count`; only remove entry when count reaches 0
3. Test: clone a WeakRef, drop the original, clone should still work until last clone dropped

---

### PS-8 — Store E2E Tests

**Priority:** High | **Est:** 8h | **Depends:** PS-2

**Goal:** Full CRUD lifecycle tests from Coral code.

**Implementation:** Add `tests/stores.rs`:
1. Create store, add records, read back, verify
2. Update records, verify changes
3. Delete records, verify deletion
4. For persistent stores: save, "restart" (new context), load, verify persistence
5. Query tests (after PS-2)

---

## 11. Track 5: Infrastructure

### IQ-1 — AST-Level Module System (ML1, ML2 bugs)

**Priority:** High | **Est:** 20h | **Depends:** None

**Goal:** Replace text-based `use` expansion with proper AST-level imports supporting namespacing and selective imports.

**Current behavior:**
- `src/module_loader.rs` L269+: `load_recursive` splices module text inline
- `src/module_loader.rs` L139-187: `extract_exports` uses regex-like text patterns

**Target syntax:**
```coral
use std.map                     # import all exports
use std.map { get_key, set_key } # selective import
use std.map as m                # qualified: m.get_key(...)
```

**Implementation:**
1. **Parser:** Parse `use` statement with optional `{ ... }` selective imports and `as name` qualifier. Add `Item::Use { module, imports, alias, span }` to AST.
2. **Module loader:** Instead of text splicing, return structured `ModuleExports`:
   - Parse each module into AST
   - Extract actual export names from AST (functions, types, stores)
   - Build a symbol table per module
3. **Semantic:** Resolve qualified names (`m.get_key`) via module symbol tables
4. **Scope:** Each imported module creates a namespace in TypeEnv
5. **Backward compat:** `use std.map` without qualifier still imports all names into current scope (star import)

**Incremental approach:**
- Step 1: Parse `use` as AST node (not text directive)
- Step 2: Build module export tables from parsed AST
- Step 3: Add selective imports `{ ... }`
- Step 4: Add qualified imports `as name`

---

### IQ-2 — Split `codegen/mod.rs`

**Priority:** Medium | **Est:** 10h | **Depends:** None

**Goal:** Break 4,828-line file into focused modules.

**Suggested split:**
| New File | Content |
|----------|---------|
| `src/codegen/mod.rs` | CodeGenerator struct, top-level dispatch, ~500 lines |
| `src/codegen/expression.rs` | `emit_expression`, `emit_member_expression`, `emit_member_call`, ~800 lines |
| `src/codegen/statement.rs` | `emit_statement`, `emit_if`, `emit_while`, `emit_for`, ~400 lines |
| `src/codegen/store_actor.rs` | `build_store_constructor`, `build_actor_constructor`, store/actor handler codegen, ~800 lines |
| `src/codegen/match_adt.rs` | Pattern matching, ADT constructor, match compilation, ~600 lines |
| `src/codegen/builtins.rs` | Built-in function dispatch (print, len, etc.), ~500 lines |
| `src/codegen/runtime.rs` | (already exists) RuntimeBindings, ~1,300 lines |

**Approach:** Move functions wholesale, keep `CodeGenerator<'ctx>` struct in `mod.rs`, use `impl CodeGenerator<'ctx>` blocks in each sub-module (Rust allows `impl` blocks in separate files within the same crate).

---

### IQ-3 — Split `runtime/src/lib.rs`

**Priority:** Medium | **Est:** 10h | **Depends:** None

**Goal:** Break 5,902-line file into focused modules.

**Suggested split:**
| New File | Content |
|----------|---------|
| `runtime/src/lib.rs` | Re-exports, initialization, ~200 lines |
| `runtime/src/value.rs` | Value representation, tag dispatch, retain/release, ~1,000 lines |
| `runtime/src/string_ops.rs` | All `coral_string_*` functions, ~800 lines |
| `runtime/src/list_ops.rs` | All `coral_list_*` functions, ~800 lines |
| `runtime/src/map_ops.rs` | All `coral_map_*` functions, ~600 lines |
| `runtime/src/closure.rs` | Closure creation, invocation, ~300 lines |
| `runtime/src/io_ops.rs` | File I/O, print, ~500 lines |
| `runtime/src/math_ops.rs` | Math FFI, ~300 lines |

---

### IQ-4 — Fix All Examples (already partially done)

**Status:** 5 of 7 compile. `chat_server.coral` and `http_server.coral` pass lexer but fail parser (need networking features from SL-13).

**Remaining work:**
- `chat_server.coral` requires TCP networking (SL-13)
- `http_server.coral` requires TCP networking (SL-13)
- These are blocked until net.coral is functional

---

### IQ-5 — Expand Test Coverage (target 500+)

**Priority:** Medium | **Est:** 15h | **Depends:** None

**Goal:** Add ~80+ tests to reach 500+ total.

**Focus areas:**
1. Parser negative cases — invalid syntax should produce meaningful errors
2. Semantic edge cases — scope shadowing, forward references, recursive types
3. Store E2E — full CRUD (overlaps with PS-8)
4. Actor E2E — spawn, send, receive, supervision (overlaps with AC-7 in Phase C)
5. Stdlib — exercise each module (overlaps with SL-16)

---

## 12. Track 6: Self-Hosted Front-End

### SC-1 — Complete Self-Hosted Lexer

**Priority:** High | **Est:** 8h | **Depends:** None

**Goal:** 92% → 100%. Fix: template string interpolation, tab/space detection, error recovery.

**Location:** `self_hosted/lexer.coral` (489 lines)

**Architecture:** All state in a map: `{ source, pos, len, tokens, indent_stack, line_start }`. Functions operate on and return the state map.

**Missing features:**
1. **Template string interpolation:** Parse `"hello {name}"` — need to emit `TemplateStart`, `TemplateExpr`, `TemplateEnd` tokens when encountering `{` inside a string
2. **Tab/space detection:** Detect mixed indentation and warn
3. **Error recovery:** On invalid character, emit an `Error` token and continue (don't crash)

**Test:** `tests/self_hosting.rs` already has 2 lexer tests. Add:
- Test that template strings tokenize correctly
- Test error recovery on invalid input

---

### SC-2 — Complete Self-Hosted Parser

**Priority:** High | **Est:** 15h | **Depends:** SC-1

**Goal:** 82% → 100%. Fix: template strings, tuple patterns, nested patterns, error recovery.

**Location:** `self_hosted/parser.coral` (1,678 lines)

**Architecture:** Recursive descent, AST nodes as maps with `"kind"` field. 40+ node constructors.

**Missing features:**
1. **Template string parsing:** Parse `TemplateStart/TemplateExpr/TemplateEnd` into an interpolated string node
2. **Tuple patterns:** `(a, b, c)` in match arms
3. **Nested match patterns:** `Some(Some(x))` — recursive pattern parsing
4. **Error recovery:** Skip to item boundary on error, continue parsing

**Test:** Add parser compilation test to `tests/self_hosting.rs`.

---

### SC-3 — Module Loader in Coral

**Priority:** High | **Est:** 12h | **Depends:** SL-2 (file I/O, completed in Phase A)

**Goal:** `use std.X` resolution in the self-hosted compiler.

**Implementation:**
1. Port `ModuleLoader` logic to Coral
2. Requires working `read_file()` (from io.coral)
3. Module resolution: convert `std.map` → `std/map.coral`, read file, splice content
4. Can be simpler than the Rust version initially — skip caching/dedup

---

### SC-4 — Front-End Verification

**Priority:** High | **Est:** 8h | **Depends:** SC-1, SC-2

**Goal:** Self-hosted lexer/parser produces identical output to Rust lexer/parser for all test fixtures.

**Implementation:**
1. For each test fixture in `tests/fixtures/`:
   - Run Rust lexer → capture token stream
   - Run self-hosted lexer → capture token stream
   - Compare
2. Same for parser → compare AST output
3. Add to `tests/self_hosting.rs` as automated comparison tests

---

## 13. Execution Order

### Recommended sequence (respecting dependencies):

**Wave 1 — Independent, High-Value** (do first, in parallel):
1. TS-4 (member access types) — unblocks better type checking everywhere
2. TS-5 (pipeline types) — common pattern, easy win
3. SL-4 (set.coral completion) — pure Coral, no dependencies
4. SL-5 (map.coral completion) — pure Coral, no dependencies
5. SL-6 (string iteration) — small runtime addition
6. SC-1 (complete self-hosted lexer) — independent track
7. PS-7 (WeakRef clone fix) — independent runtime fix

**Wave 2 — Medium Complexity** (after Wave 1):
8. TS-6 (multi-error parser) — significant refactor, do when stable
9. SL-8 (json.coral) — needs runtime FFI addition
10. SL-9 (time.coral) — needs runtime FFI addition
11. SL-10 (fmt.coral) — pure Coral
12. SL-15 (testing.coral) — pure Coral
13. AC-2 (actor monitoring) — runtime addition
14. AC-3 (supervision hardening) — runtime hardening
15. AC-4 (graceful stop) — small runtime change
16. IQ-2 (split codegen) — refactoring, no logic changes
17. IQ-3 (split runtime) — refactoring, no logic changes

**Wave 3 — Complex / Dependent**:
18. AC-1 (typed messages) — needs semantic analysis changes
19. AC-5 (work-stealing) — complex runtime change
20. PS-2 (store query syntax) — parser + semantic + codegen + runtime
21. PS-4 (store indexing) — runtime + codegen
22. PS-5 (ACID transactions) — complex runtime
23. PS-6 (WAL recovery tests) — runtime test
24. SL-13 (net.coral TCP) — large runtime addition
25. SL-14 (error propagation) — parser + codegen
26. IQ-1 (AST module system) — large refactor across loader + parser + semantic

**Wave 4 — Finalization**:
27. SC-2 (complete self-hosted parser) — depends on SC-1
28. SC-3 (module loader in Coral) — depends on working I/O
29. SC-4 (front-end verification) — depends on SC-1 + SC-2
30. SL-7, SL-11, SL-12 (bytes, sort, encoding) — lower priority
31. TS-9 (exhaustiveness) — nice to have
32. SL-16 (stdlib test suite) — depends on all stdlib work
33. PS-8 (store E2E tests) — depends on PS-2
34. IQ-5 (expand test coverage) — ongoing throughout

---

## 14. Verification Checklist

After completing Phase B, verify:

- [ ] `cargo build` succeeds with 0 errors
- [ ] `cd runtime && cargo build` succeeds with 0 errors
- [ ] `cargo test 2>&1 | grep "test result"` shows 500+ tests, 0 failures
- [ ] All 7 examples in `examples/` compile (including chat_server and http_server after SL-13)
- [ ] `std/set.coral` has intersection, difference, is_subset, is_superset
- [ ] `std/json.coral` exists and parse/serialize work
- [ ] `std/time.coral` exists with at least `now()`, `timestamp()`
- [ ] `std/fmt.coral` exists with basic formatting
- [ ] `std/testing.coral` exists with assert functions
- [ ] `std/encoding.coral` exists with base64/hex
- [ ] `std/sort.coral` exists
- [ ] Pipeline type inference works: `5 ~ double ~ add_one` infers correctly
- [ ] Member access on stores/ADTs infers field types (not Map fallback)
- [ ] Parser reports multiple errors per file
- [ ] Actor monitoring works: `monitor(worker)` delivers `ActorDown`
- [ ] Typed messages compile-time check: wrong type at `send()` → error
- [ ] Store queries work from Coral code
- [ ] Self-hosted lexer is 100% complete, all test fixtures match
- [ ] Self-hosted parser is 100% complete, all test fixtures match
- [ ] `tests/stdlib.rs` exists with comprehensive stdlib tests
- [ ] `tests/stores.rs` exists with store lifecycle tests
- [ ] `codegen/mod.rs` is split into ≤1,000 lines per module
- [ ] Module loader supports selective imports `use std.map { get_key }`
- [ ] `ALPHA_ROADMAP.md` updated with Phase B completion status

---

## Appendix A: Runtime FFI Functions to Add

Summary of new `pub extern "C" fn coral_*` functions needed:

| Function | File | Task |
|----------|------|------|
| `coral_string_lines` | `runtime/src/lib.rs` | SL-6 |
| `coral_bytes_from_hex` | `runtime/src/lib.rs` | SL-7 |
| `coral_bytes_contains` | `runtime/src/lib.rs` | SL-7 |
| `coral_bytes_find` | `runtime/src/lib.rs` | SL-7 |
| `coral_json_parse` | `runtime/src/lib.rs` | SL-8 |
| `coral_json_serialize` | `runtime/src/lib.rs` | SL-8 |
| `coral_json_serialize_pretty` | `runtime/src/lib.rs` | SL-8 |
| `coral_time_now` | `runtime/src/lib.rs` | SL-9 |
| `coral_time_format` | `runtime/src/lib.rs` | SL-9 |
| `coral_list_sort` | `runtime/src/lib.rs` | SL-11 |
| `coral_base64_encode` | `runtime/src/lib.rs` | SL-12 |
| `coral_base64_decode` | `runtime/src/lib.rs` | SL-12 |
| `coral_hex_encode` | `runtime/src/lib.rs` | SL-12 |
| `coral_hex_decode` | `runtime/src/lib.rs` | SL-12 |
| `coral_tcp_listen` | `runtime/src/lib.rs` | SL-13 |
| `coral_tcp_accept` | `runtime/src/lib.rs` | SL-13 |
| `coral_tcp_connect` | `runtime/src/lib.rs` | SL-13 |
| `coral_tcp_read` | `runtime/src/lib.rs` | SL-13 |
| `coral_tcp_write` | `runtime/src/lib.rs` | SL-13 |
| `coral_tcp_close` | `runtime/src/lib.rs` | SL-13 |
| `coral_actor_monitor` | `runtime/src/actor.rs` | AC-2 |
| `coral_actor_demonitor` | `runtime/src/actor.rs` | AC-2 |
| `coral_actor_stop` | `runtime/src/actor.rs` | AC-4 |
| `coral_store_query` | `runtime/src/store/ffi.rs` | PS-2 |
| `coral_store_create_index` | `runtime/src/store/ffi.rs` | PS-4 |
| `coral_transaction_begin` | `runtime/src/store/ffi.rs` | PS-5 |
| `coral_transaction_commit` | `runtime/src/store/ffi.rs` | PS-5 |
| `coral_transaction_rollback` | `runtime/src/store/ffi.rs` | PS-5 |

Each requires: (1) implement in runtime, (2) rebuild runtime, (3) declare in `src/codegen/runtime.rs`, (4) wire in `src/codegen/mod.rs`.

---

## Appendix B: Cargo Dependencies to Add

| Crate | Where | Purpose | Task |
|-------|-------|---------|------|
| `serde_json` | `runtime/Cargo.toml` | JSON parse/serialize | SL-8 |
| `base64` | `runtime/Cargo.toml` | Base64 encoding | SL-12 |
| `crossbeam-deque` | `runtime/Cargo.toml` | Work-stealing scheduler | AC-5 |

---

## Appendix C: Reference Specifications

Full specs are in `docs/`. Read these when working on the relevant track:

| Document | Read For |
|----------|----------|
| `docs/STANDARD_LIBRARY_SPEC.md` | Track 2 (Stdlib) — all function signatures |
| `docs/ACTOR_SYSTEM_COMPLETION.md` | Track 3 (Actors) — typed messages, supervision, monitoring |
| `docs/PERSISTENT_STORE_SPEC.md` | Track 4 (Stores) — query syntax, ACID, indexing, WAL format |
| `docs/VALUE_ERROR_MODEL.md` | SL-14 (error propagation) — flag system, `! return err` |
| `docs/SELF_HOSTING_STATUS.md` | Track 6 (Self-hosted) — current completion, blocking prerequisites |
| `docs/STDLIB_STATUS.md` | Track 2 (Stdlib) — per-module status |
| `docs/ALPHA_ROADMAP.md` | Master roadmap — task IDs, dependencies, priority ordering |
