# Coral Codebase — LLM Session Onboarding Guide

**Purpose:** Get a new LLM agent session productive in the Coral codebase within minutes. Read this document first before starting any work.

**Last updated:** March 8, 2026

---

## Table of Contents

- [What Is Coral](#what-is-coral)
- [Design Principles](#design-principles)
- [Project Layout](#project-layout)
- [Compiler Pipeline (Rust)](#compiler-pipeline-rust)
- [Self-Hosted Compiler (Coral)](#self-hosted-compiler-coral)
- [Runtime Library](#runtime-library)
- [NaN-Boxing Architecture](#nan-boxing-architecture)
- [Standard Library](#standard-library)
- [Test Infrastructure](#test-infrastructure)
- [Key Patterns and Gotchas](#key-patterns-and-gotchas)
- [Roadmap and Progress](#roadmap-and-progress)
- [Agent Workflow Guidelines](#agent-workflow-guidelines)

---

## What Is Coral

Coral is a programming language that reads like Python, compiles to native code via LLVM, and has built-in actors for concurrency. Key facts:

- **Indentation-scoped** — no braces or semicolons
- **`is` for binding** — no `=` or `==` tokens exist
- **Functions declared with `*`** — `*foo(x, y)` declares a function
- **Implicit return** — last expression in a block is the return value
- **Pure type inference** — zero type annotations in the syntax; constraint-based solver handles everything
- **Single numeric type** — `f64` at runtime; AST distinguishes `Int`/`Float` only for constant folding
- **Actors** — `actor` keyword, `@handler` syntax, M:N scheduler, supervision trees
- **Persistent stores** — `store` keyword, WAL-backed durable state
- **Algebraic data types** — `type`/`enum` keyword with `match` exhaustiveness checking
- **Traits** — `trait` keyword, `with` for implementation, default methods
- **Errors as values** — hierarchical `err Name:Sub:Detail`, propagation with `! return err`
- **Self-hosted** — the compiler is written in Coral itself and bootstraps (gen2 == gen3 byte-identical)

```coral
*main()
    name is 'World'
    log('Hello, {name}!')

    items is [1, 2, 3]
    config is map('host': 'localhost', 'port': 8080)

    for item in items
        log('{item}')

    result is 42 > 10 ? 'big' ! 'small'
```

---

## Design Principles

These are non-negotiable in the language design. Never introduce changes that violate them:

1. **`is` for binding, `.equals()` for comparison.** No `=`, `==`, or `!=` tokens.
2. **No type annotations.** Types are inferred, not declared. They don't exist in the syntax.
3. **Indentation is structure.** The lexer emits `INDENT`/`DEDENT`/`NEWLINE` tokens.
4. **Errors are data.** No exceptions, no `try`/`catch`, no stack unwinding.
5. **One numeric type.** `f64` everywhere at runtime. No `i32`/`u64`/`f32` in user code.
6. **Dual implementation.** Language features must work in BOTH the Rust reference compiler AND the self-hosted Coral compiler.
7. **Conversational syntax.** Code should read like natural language where possible.

---

## Project Layout

```
coral/                          # Root workspace
├── src/                        # Rust reference compiler (~16,500 lines)
│   ├── lexer.rs                # Indent-aware tokenizer (~870 lines)
│   ├── parser.rs               # Recursive-descent parser (~2,300 lines)
│   ├── ast.rs                  # AST node definitions (~420 lines)
│   ├── semantic.rs             # Semantic analysis + scope checking (~2,580 lines)
│   ├── lower.rs                # Placeholder ($) desugaring, IR prep (~680 lines)
│   ├── compiler.rs             # Pipeline orchestration + constant folding (~280 lines)
│   ├── main.rs                 # CLI: coralc binary (~330 lines)
│   ├── module_loader.rs        # `use` directive expansion (~520 lines)
│   ├── diagnostics.rs          # Error types and spans (~170 lines)
│   ├── span.rs                 # Byte-offset span type (~40 lines)
│   ├── lib.rs                  # Public API re-exports
│   ├── types/                  # Type system (~1,500 lines)
│   │   ├── core.rs             # TypeId enum, AllocationStrategy
│   │   ├── env.rs              # Type environment (scoped bindings)
│   │   ├── solver.rs           # Constraint-based type solver
│   │   └── mod.rs              # Re-exports
│   └── codegen/                # LLVM IR generation via Inkwell (~6,900 lines)
│       ├── mod.rs              # Core codegen: expressions, statements, control flow (~2,200 lines)
│       ├── runtime.rs          # RuntimeBindings: all FFI function declarations (~1,880 lines)
│       ├── builtins.rs         # Builtin function dispatch (~1,420 lines)
│       ├── closures.rs         # Lambda/closure codegen (~730 lines)
│       ├── match_adt.rs        # Pattern matching codegen (~260 lines)
│       └── store_actor.rs      # Store/actor codegen (~420 lines)
│
├── runtime/                    # Runtime library (Rust, ~11,000+ lines in src/)
│   ├── Cargo.toml              # Builds libruntime.so (cdylib)
│   └── src/
│       ├── lib.rs              # Value type, tag system, core FFI
│       ├── nanbox.rs           # NanBoxedValue type + encoding helpers
│       ├── nanbox_ffi.rs       # NaN-boxed FFI: constructors, arithmetic, comparisons
│       ├── rc_ops.rs           # Reference counting (retain/release)
│       ├── cycle_detector.rs   # Mark-gray/scan/collect-white cycle detection
│       ├── list_ops.rs         # List FFI (push, pop, get, set, slice, map, filter, etc.)
│       ├── map_ops.rs          # Map FFI (get, set, delete, keys, values, entries)
│       ├── string_ops.rs       # String FFI (concat, split, replace, StringBuilder)
│       ├── arithmetic.rs       # Arithmetic FFI (add, sub, mul, div, mod, pow)
│       ├── math_ops.rs         # Math FFI (sin, cos, sqrt, floor, etc.)
│       ├── io_ops.rs           # I/O FFI (read_file, write_file, stdin)
│       ├── json_ops.rs         # JSON parse/stringify
│       ├── actor.rs            # Actor system (spawn, send, scheduler)
│       ├── actor_ops.rs        # Actor FFI bindings
│       ├── closure_ops.rs      # Closure FFI (invoke, release)
│       ├── error_ffi.rs        # Error value FFI
│       ├── store/              # Persistent store engine (WAL, binary format)
│       ├── bytes_ops.rs        # Bytes type FFI
│       ├── encoding_ops.rs     # Base64, hex encoding
│       ├── time_ops.rs         # Time FFI (now, sleep)
│       ├── tagged_ops.rs       # Tagged/ADT value helpers
│       ├── memory_ops.rs       # Memory utilities
│       ├── metrics.rs          # Runtime telemetry
│       └── ...
│
├── self_hosted/                # Self-hosted compiler (Coral, ~7,860 lines)
│   ├── compiler.coral          # Pipeline orchestration (~320 lines)
│   ├── lexer.coral             # Tokenizer (~540 lines)
│   ├── parser.coral            # Parser (~1,880 lines)
│   ├── semantic.coral          # Semantic analysis (~1,720 lines)
│   ├── lower.coral             # Placeholder desugaring (~670 lines)
│   ├── codegen.coral           # LLVM IR text emitter (~2,430 lines)
│   ├── module_loader.coral     # Module resolution (~280 lines)
│   └── debug_tokens.coral      # Token debug printer (~24 lines)
│
├── std/                        # Standard library (Coral, ~1,700 lines, 20 modules)
│   ├── prelude.coral           # Common imports
│   ├── string.coral            # String operations (str_split, str_replace, etc.)
│   ├── list.coral              # List utilities
│   ├── map.coral               # Map utilities
│   ├── math.coral              # Math constants and functions
│   ├── io.coral                # File I/O
│   ├── json.coral              # JSON helpers
│   ├── testing.coral           # Test assertions (assert_eq, assert_true)
│   ├── option.coral            # Option type (Some/None) + unwrap
│   ├── result.coral            # Result type (Ok/Err) + unwrap
│   ├── fmt.coral               # Formatting
│   ├── sort.coral              # Sorting algorithms
│   ├── net.coral               # Networking (TCP)
│   ├── time.coral              # Time utilities
│   ├── encoding.coral          # Base64, hex
│   ├── char.coral              # Character utilities
│   ├── bytes.coral             # Bytes type
│   ├── set.coral               # Set type
│   ├── bit.coral               # Bitwise operations
│   └── process.coral           # Process/environment
│
├── tests/                      # Integration tests (Rust, ~14,000 lines, 816 tests)
│   ├── execution.rs            # E2E: compile → lli → assert stdout (~2,550 lines)
│   ├── codegen_extended.rs     # IR compilation tests
│   ├── semantic.rs             # Semantic pass tests
│   ├── parser_*.rs             # Parser tests (multiple files)
│   ├── lexer_*.rs              # Lexer tests
│   ├── self_hosting.rs         # Bootstrap tests
│   ├── fixtures/               # Test fixture files (.coral programs)
│   └── snapshots/              # Insta snapshot files
│
├── benchmarks/                 # Performance benchmarks
│   ├── fibonacci.coral         # Recursive fib(30)
│   ├── tight_loop.coral        # Sum 0..10M
│   ├── list_ops.coral          # 100K list operations
│   ├── string_ops.coral        # 10K string operations
│   ├── matrix_mul.coral        # 3×3 matrix multiply ×50K
│   └── run_benchmarks.py       # Benchmark runner script
│
├── examples/                   # Example programs
├── docs/                       # Documentation
│   ├── LANGUAGE_EVOLUTION_ROADMAP.md   # Master 6-pillar roadmap (AUTHORITATIVE)
│   ├── EVOLUTION_PROGRESS.md           # Progress tracker
│   ├── LLM_ONBOARDING.md              # This file
│   └── ...
├── tools/                      # Agent helper scripts (codemap, xref)
├── tree-sitter-coral/          # Tree-sitter grammar
└── vscode-coral/               # VS Code extension
```

---

## Compiler Pipeline (Rust)

The reference compiler lives in `src/`. The pipeline is:

```
Source → ModuleLoader → Lexer → Parser → Lowering → Semantic → Constant Folding → Codegen → LLVM IR
```

### 1. Module Loader (`src/module_loader.rs`)

Resolves `use std.X` directives by text-substituting `std/X.coral` content into the source before lexing. This is a simple text-based expansion, not AST-level imports.

### 2. Lexer (`src/lexer.rs`)

Indent-aware tokenizer. Tracks indentation level and emits synthetic `INDENT`, `DEDENT`, and `NEWLINE` tokens. This is how Coral avoids braces — the lexer converts whitespace structure into explicit tokens the parser can consume.

Key tokens: `Is` (binding), `Star` (function prefix), `Question` / `Bang` (ternary/error), `Tilde` (pipeline), `Colon` (map entries).

### 3. Parser (`src/parser.rs`)

Recursive-descent parser producing an AST (`src/ast.rs`). Key AST nodes:

**Statements:** `Binding` (`x is expr`), `If`/`While`/`For`/`ForKV`/`ForRange`, `Return`, `FieldAssign` (`self.field is value`), `Break`/`Continue`

**Expressions:** `Identifier`, `Integer`/`Float`/`Bool`/`String`, `Call`, `Member`, `Binary`/`Unary`, `Ternary`, `Lambda`, `List`/`Map`, `Match`, `Pipeline`, `ErrorValue`/`ErrorPropagate`, `Index`, `InlineAsm`, `PtrLoad`, `Unsafe`

**Items (top-level):** `Function`, `Type`/`Store`/`Trait`/`Taxonomy`/`ErrorDefinition`, `ExternFunction`, `Binding`, `Expression`

### 4. Lowering (`src/lower.rs`)

Transforms the AST before semantic analysis:
- **Placeholder desugaring:** `$ * 2` in pipeline/HOF contexts becomes an auto-generated lambda `($$0) -> $$0 * 2`
- Walks all expression and statement forms to find and replace `Placeholder` nodes

### 5. Semantic Analysis (`src/semantic.rs`)

Produces a `SemanticModel` containing:
- `functions`: Vec of analyzed functions with resolved scopes
- `globals`: Vec of global bindings
- `type_definitions`, `store_definitions`, `trait_definitions`, `taxonomy_nodes`, `actor_definitions`
- `warnings`: Vec of diagnostic messages

Key responsibilities:
- Scope checking (variable resolution, forward references for functions)
- Builtin name recognition (`is_builtin_name()` — if you add a new builtin, register it here)
- Warning generation (unused variables, shadowing)
- Exhaustive match arm validation for AST Statement/Expression variants

**CRITICAL:** When adding a new AST node (Statement or Expression variant), you must add it to **every** `match` in `semantic.rs`. There are typically 7+ match sites for Statement variants. The Rust compiler will catch missing arms, but be thorough.

### 6. Constant Folding (`src/compiler.rs`)

Post-semantic pass that folds literal expressions: `1 + 2` → `3`, `true and false` → `false`, `'a' + 'b'` → `'ab'`. The `fold_expressions()` method walks the semantic model and rewrites constant sub-expressions.

### 7. Codegen (`src/codegen/`)

LLVM IR generation via the [Inkwell](https://github.com/TheDan64/inkwell) crate (LLVM 16.0 bindings). This is the largest and most complex part of the compiler.

#### `codegen/mod.rs` (~2,200 lines) — Core

The `CodeGenerator` struct holds:
- `context: &'ctx Context` — LLVM context
- `module: Module<'ctx>` — LLVM module being built
- `builder: Builder<'ctx>` — instruction builder
- `runtime: RuntimeBindings<'ctx>` — all FFI function references
- `variables: HashMap<String, IntValue<'ctx>>` — variable storage (NaN-boxed `i64`)
- `string_globals: HashMap<String, GlobalValue<'ctx>>` — interned string constants

Key methods:
- `compile(&self, model: &SemanticModel) -> Result<Module>` — entry point
- `emit_expression(expr) -> Result<IntValue>` — all expressions return `i64` (NaN-boxed)
- `emit_block(block) -> Result<IntValue>` — block return value
- `emit_statement(stmt) -> Result<Option<IntValue>>` — may or may not produce a value
- `emit_string_literal(s) -> IntValue` — interned global string → NaN-boxed value
- `nb_to_ptr(i64) -> PointerValue` — bridge: NaN-boxed → old-API pointer
- `ptr_to_nb(ptr) -> IntValue` — bridge: old-API pointer → NaN-boxed

**All Coral values are `i64` in LLVM IR** (NaN-boxed representation). When calling old-API runtime functions that still use `%CoralValue*`, values are bridged through `nb_to_ptr` / `ptr_to_nb`.

#### `codegen/runtime.rs` (~1,880 lines) — FFI Declarations

`RuntimeBindings` struct declares every runtime function the generated IR can call. The `declare()` method registers all functions in the LLVM module. This includes:

- **NaN-box constructors:** `nb_make_number`, `nb_make_bool`, `nb_make_unit`, `nb_make_none`, `nb_make_string`
- **NaN-box extractors:** `nb_as_number`, `nb_as_bool`, `nb_tag`, `nb_is_truthy`, `nb_is_err`
- **NaN-box lifecycle:** `nb_retain`, `nb_release`
- **NaN-box arithmetic:** `nb_add`, `nb_sub`, `nb_mul`, `nb_div`, `nb_rem`, `nb_neg`
- **NaN-box comparison:** `nb_equals`, `nb_not_equals`, `nb_less_than`, etc.
- **Bridge:** `nb_to_handle` (i64 → ptr), `nb_from_handle` (ptr → i64)
- **Old API (bridged):** `coral_make_string`, `coral_list_*`, `coral_map_*`, `coral_value_*`, etc.
- **StringBuilder:** `sb_new`, `sb_push`, `sb_finish`, `sb_len`
- **Optimized string ops:** `string_join_list`, `string_repeat`, `string_reverse`

**Pattern for adding a new FFI function:**
1. Add `pub field: FunctionValue<'ctx>` to `RuntimeBindings` struct
2. Add `module.add_function(...)` call in `declare()` with correct signature
3. Set the field in the `Self { ... }` block
4. Add a builtin match arm in `builtins.rs`
5. Register the name in `semantic.rs` → `is_builtin_name()`

#### `codegen/builtins.rs` (~1,420 lines) — Builtin Dispatch

The massive `emit_builtin_call()` method dispatches recognized function names to runtime calls. This handles: `log`, `print`, `length`, `push`, `pop`, `get`, `set`, `map`, `filter`, `reduce`, `keys`, `values`, `entries`, `to_string`, `number_to_string`, `string_join_list`, `string_repeat`, `string_reverse`, arithmetic helpers, type checks, and more.

Also contains `emit_member_call()` for method dispatch on objects (`.length()`, `.push()`, `.get()`, `.equals()`, etc.) and `emit_io_call()` for `read_file`/`write_file`/`append_file`.

#### `codegen/closures.rs` (~730 lines) — Lambdas

Handles closure compilation:
- Environment capture (free variable analysis)
- Closure struct creation (env + function pointer)
- Lambda invocation through function pointers
- Function-as-closure wrapping (passing named functions where closures are expected)
- Enum/ADT constructors as closures

#### `codegen/match_adt.rs` (~260 lines) — Pattern Matching

Emits code for `match` expressions against ADT variants. Handles:
- Tag extraction and comparison
- Field binding in patterns
- Wildcard patterns
- Nested patterns (recursive)
- Literal patterns

#### `codegen/store_actor.rs` (~420 lines) — Stores and Actors

Emits constructors and methods for `store` and `actor` definitions:
- Store: map-backed field storage, constructor with defaults, method compilation
- Actor: spawn, message handler dispatch, named actor registry

### CLI (`src/main.rs`)

The `coralc` binary supports:
```bash
coralc program.coral                    # Print IR to stdout
coralc program.coral --emit-ir out.ll   # Write IR to file
coralc --jit program.coral              # JIT via lli + libruntime.so
coralc program.coral --emit-binary ./a  # Native binary via llc + clang
```

The `--emit-binary` path: IR → temp file → `llc -filetype=obj` → `clang` (links with `-lruntime -lm`).
The `--jit` path: IR → temp file → `lli -load libruntime.so`.

---

## Self-Hosted Compiler (Coral)

The self-hosted compiler in `self_hosted/` mirrors the Rust compiler's pipeline exactly, but is written entirely in Coral. It bootstraps: compiling itself produces byte-identical output across generations.

### Architecture Differences from Rust Compiler

1. **Text-based IR emission** — The self-hosted codegen emits LLVM IR as text strings (`.ll` format), NOT via Inkwell bindings. It builds IR by string concatenation using an `IRBuilder` map.

2. **Still uses `%CoralValue*`** — The self-hosted codegen has NOT been transitioned to NaN-boxing (`i64`). It still emits the old pointer-based calling convention. This is intentional — it's a separate migration task.

3. **Data structures are maps** — AST nodes are `map(...)` dictionaries, not Rust enums. A function node looks like `map("kind": "function", "name": "foo", "params": [...], "body": [...])`. Checking node type: `stmt.get("kind")`.

4. **No type system** — The self-hosted semantic pass does scope checking but no type inference. Types are effectively all `Any`.

5. **Module system** — Same text-based `use` expansion as the Rust compiler.

### Self-Hosted File Mapping

| Rust File | Self-Hosted File | Notes |
|-----------|-----------------|-------|
| `src/compiler.rs` | `self_hosted/compiler.coral` | Pipeline orchestration |
| `src/lexer.rs` | `self_hosted/lexer.coral` | Token types as string constants |
| `src/parser.rs` | `self_hosted/parser.coral` | Recursive descent, maps as AST |
| `src/semantic.rs` | `self_hosted/semantic.coral` | Scope checking only |
| `src/lower.rs` | `self_hosted/lower.coral` | Placeholder desugaring |
| `src/codegen/` | `self_hosted/codegen.coral` | Text-based IR emitter |
| `src/module_loader.rs` | `self_hosted/module_loader.coral` | `use` expansion |

### When You Modify the Rust Compiler

If you add a new AST node or language feature to the Rust compiler, you should also add it to the self-hosted compiler. The key files to update:

1. **`parser.coral`** — Add parsing for the new syntax
2. **`codegen.coral`** — Add IR emission (text-based `%CoralValue*` style)
3. **`semantic.coral`** — Add to scope checking if it introduces variables
4. **`lower.coral`** — Add if it requires placeholder desugaring

---

## Runtime Library

The runtime (`runtime/src/`) is a Rust shared library (`libruntime.so` / `libruntime.dylib`) exposing 220+ FFI functions with C calling convention (`extern "C"`).

### Value Representation

**Old system (still used by self-hosted compiler):**
- `Value` struct: 40 bytes, heap-allocated, refcounted with `AtomicU64`
- Tags: `Number(f64)`, `StringVal`, `Bool`, `List(Vec<*mut Value>)`, `Map`, `Unit`, `None`, `Closure`, `Tagged`, `Bytes`, `Store`, `Actor`
- Every value, including `42` or `true`, is heap-allocated

**NaN-boxing (used by Rust compiler's codegen):**
- All values are `i64` in LLVM IR
- IEEE 754 doubles pass through directly (any `u64` where bits 63..51 ≠ `0x7FF8`)
- Quiet NaN payloads encode: Bool, Unit, None, heap pointers, error markers
- Heap-allocated containers (`String`, `List`, `Map`, etc.) are encoded as heap pointers in the NaN-box
- Primitives (`Number`, `Bool`, `Unit`, `None`) are **zero allocation, zero refcount**
- Bridge functions `coral_nb_to_handle` / `coral_nb_from_handle` convert between old `*mut Value` and new `i64`

### Key FFI Naming Conventions

- `coral_nb_*` — NaN-boxed API (new, used by Rust codegen)
- `coral_make_*` — Old constructors (still used via bridge)
- `coral_value_*` — Old value operations (still used via bridge)
- `coral_list_*` — List operations
- `coral_map_*` — Map operations
- `coral_string_*` / `coral_sb_*` — String operations / StringBuilder
- `coral_actor_*` — Actor operations
- `coral_store_*` — Store operations

### Adding a New Runtime Function

1. Implement in the appropriate `runtime/src/*.rs` file with `#[no_mangle] pub extern "C" fn`
2. Declare in `src/codegen/runtime.rs` (struct field + `module.add_function` + Self block)
3. Add dispatch in `src/codegen/builtins.rs`
4. Add to `is_builtin_name()` in `src/semantic.rs`
5. Build runtime: `cargo build -p runtime` (debug) or `cargo build -p runtime --release`

---

## NaN-Boxing Architecture

This is the most important architectural detail in the codebase. The Rust compiler's codegen uses NaN-boxed `i64` values everywhere. The transition happened in sessions 12-15 and touched nearly every codegen file.

### How NaN-Boxing Works in Codegen

1. **All variables** are `IntValue<'ctx>` (i64), stored in `HashMap<String, IntValue>`
2. **All function signatures** use `i64` params and `i64` returns
3. **Hot-path operations** use NaN-box FFI directly: `nb_add`, `nb_equals`, `nb_make_number`, etc.
4. **Cold-path operations** (list/map/string ops that still use old API) use the bridge pattern:
   ```
   value_i64 → nb_to_ptr → call old_api(ptr) → nb_from_handle → result_i64
   ```
5. **PHI nodes** are `i64` typed
6. **Global variables** are `i64` typed

### The Bridge Pattern

Many runtime functions still use `*mut Value` (pointer) arguments. To call them from NaN-boxed codegen:

```rust
// In codegen:
let ptr = self.nb_to_ptr(nb_value);           // i64 → %CoralValue*
let result_ptr = call_old_api(ptr);           // Returns %CoralValue*
let result_nb = self.ptr_to_nb(result_ptr);   // %CoralValue* → i64
```

This is the `call_bridged` pattern in `builtins.rs`. Over time, more runtime functions should get native `i64` APIs to eliminate bridge overhead.

---

## Standard Library

The stdlib lives in `std/` (Coral source files). Modules are loaded via `use std.X`. The module loader does text substitution — the entire module content is spliced into the source before lexing.

Important notes:
- `std/string.coral` — Recently standardized naming: `str_starts_with`, `str_ends_with`, `str_replace`, `str_split`, `str_slice`, `str_contains`, `str_trim`. Old names (`begins_with`, `sub`, `divide`, `part`) are deprecated aliases.
- `std/option.coral` / `std/result.coral` — `unwrap` calls `exit(1)` on failure (not just log).
- `std/testing.coral` — `assert_eq` uses polymorphic `to_string()` for display.
- `std/list.coral` — Has `pop`, `sublist`, standard list operations.

---

## Test Infrastructure

### Test Categories

Tests live in `tests/*.rs`. Current baseline: **816 tests passing, 0 failures**.

| File | Type | Description |
|------|------|-------------|
| `execution.rs` | E2E | Compile → lli → assert stdout. The most important tests. |
| `codegen_extended.rs` | IR | Compile to IR, assert IR properties |
| `core_spec.rs` | IR | Core example compilation checks |
| `semantic.rs` / `semantic_extended.rs` | Unit | Semantic pass correctness |
| `parser_*.rs` | Unit | Parser output verification |
| `lexer_*.rs` | Unit | Tokenizer tests |
| `self_hosting.rs` | E2E | Bootstrap verification |
| `stdlib.rs` | E2E | Standard library tests |
| `traits.rs` | E2E | Trait system tests |
| `stores.rs` | E2E | Persistent store tests |
| `named_actors.rs` / `timers.rs` | E2E | Actor system tests |

### Running Tests

```bash
# Full suite
cargo test

# Specific test file
cargo test --test execution

# Specific test
cargo test --test execution e2e_for_kv_map_iteration

# Build runtime first (required for E2E tests)
cargo build -p runtime

# E2E tests require lli (LLVM interpreter) to be installed
```

### E2E Test Pattern

```rust
fn run_coral(source: &str) -> (String, String, i32) {
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).unwrap();
    // Write IR to temp file → run via lli -load libruntime.so → capture stdout
}

#[test]
fn e2e_my_feature() {
    let (stdout, _stderr, exit_code) = run_coral(r#"
*main()
    log('hello')
"#);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("hello"));
}
```

### IR Compilation Test Pattern

```rust
#[test]
fn my_feature_compiles() {
    let compiler = Compiler;
    let ir = compiler.compile_to_ir("*main()\n    log('hi')").unwrap();
    assert!(ir.contains("@coral_nb_println"));
}
```

---

## Key Patterns and Gotchas

### Adding a New Statement Type

This is the most common structural change. Here's the full checklist:

1. **`src/ast.rs`** — Add variant to `Statement` enum
2. **`src/parser.rs`** — Parse the new syntax, emit the AST node
3. **`src/semantic.rs`** — Add to ALL match arms (7+ sites). Search for `Statement::For` to find them all.
4. **`src/lower.rs`** — Add to `lower_statement()` and `replace_block_placeholders()`
5. **`src/compiler.rs`** — Add to `fold_block()` in constant folding
6. **`src/codegen/mod.rs`** — Add to `emit_statement()`
7. **`src/codegen/closures.rs`** — Add to capture collection (`collect_captures`)
8. **`tests/parser_snapshots.rs`** — Add snapshot test if parser output is important
9. **`tests/execution.rs`** — Add E2E test
10. **`self_hosted/parser.coral`** — Parse in self-hosted compiler
11. **`self_hosted/codegen.coral`** — Emit IR in self-hosted compiler
12. **`self_hosted/semantic.coral`** — Add to scope checking

### Adding a New Builtin Function

1. Implement FFI in `runtime/src/*.rs` (`#[no_mangle] pub extern "C" fn`)
2. Declare in `src/codegen/runtime.rs` (struct field + declaration)
3. Dispatch in `src/codegen/builtins.rs` (match arm in `emit_builtin_call`)
4. Register in `src/semantic.rs` → `is_builtin_name()` list
5. Build runtime: `cargo build -p runtime`
6. Test: `cargo test`

### Common Mistakes

- **Forgetting `is_builtin_name`** — New builtins that aren't registered in `semantic.rs` will trigger "undefined variable" errors at compile time.
- **Forgetting match arms in `semantic.rs`** — The Rust compiler will tell you, but there are multiple matches to update.
- **Not bridging values** — When calling old-API functions from NaN-boxed codegen, you must use `nb_to_ptr` / `ptr_to_nb`. Calling an old-API function with a raw `i64` will segfault.
- **Not linking `-lm`** — The native binary link step needs `-lm` for math functions.
- **Build order** — Runtime must be built before running E2E tests: `cargo build -p runtime`.

### Debugging Tips

- **View generated IR:** `cargo run -- program.coral` prints LLVM IR to stdout. Search for your function name.
- **Environmental flags:**
  - `CORAL_INLINE_ASM=allow-noop` — Enable inline asm (noop mode)
  - `CORAL_RUNTIME_METRICS=1` — Enable runtime telemetry
- **Failing E2E test:** Check stderr output — lli often gives useful error messages about type mismatches.
- **Snapshot tests:** Use `INSTA_UPDATE=1 cargo test` to update snapshots after intentional changes.

---

## Roadmap and Progress

### Key Documents

- **`docs/LANGUAGE_EVOLUTION_ROADMAP.md`** — Master 6-pillar roadmap with all planned work
- **`docs/EVOLUTION_PROGRESS.md`** — Progress tracker with session log and task status
- **`docs/ALPHA_ROADMAP.md`** — Alpha release plan

### Completed Work Streams

| Stream | Status | Summary |
|--------|--------|---------|
| **M1 — NaN-Boxing** | ✅ Complete (8/8 tasks) | Full transition from `%CoralValue*` to `i64`. 5-10x numeric speedup. |
| **S1 — Core Syntax** | ✅ Complete (3/5, 2 skipped) | Map colon syntax, range loops, unary negation |
| **L1 — Stdlib Foundation** | ✅ Complete (6/6) | StringBuilder, unwrap/panic, assert_eq, naming, list.pop, map iteration |

### Next Priority Areas (from roadmap)

1. **T1 — Seal Type Escape Hatches** — Make type inference reliable
2. **C1 — Enhanced Constant Folding** — Comptime optimization
3. **S2 — Collection & Data Expression** — Pipeline operator, comprehensions
4. **CC2 — Error Reporting** — Source-mapped errors, multi-error reporting

---

## Agent Workflow Guidelines

This section describes how the project owner wants LLM agents to work on this codebase.

### Operating Philosophy

**Be self-directed and autonomous.** Take on a set of related tasks, plan the approach, execute each change, test, and remediate problems — all without asking for confirmation or guidance at each step. The user provides the goal; the agent plans and executes.

### Workflow for Each Task

1. **Understand** — Read the relevant code. Search for existing patterns. Understand the full scope of what needs to change.
2. **Plan** — Think through the changeset before writing code. Identify all files that need modification. Consider edge cases.
3. **Execute** — Make the changes. Follow existing code style. Keep changes minimal and focused.
4. **Test** — Run `cargo test` after each meaningful change. Check for compilation errors first, then test failures.
5. **Remediate** — If tests fail, diagnose and fix. Don't move on to the next task until the current one passes.
6. **Record** — After completing a set of tasks, update `docs/EVOLUTION_PROGRESS.md` when asked.

### Task Batching

Work on **related tasks together** in a single session. For example, if implementing a new language feature:
- Parse it, add semantic checking, add codegen, add to self-hosted compiler, write tests — all in one flow.
- Don't implement just the parser and hand back; complete the feature end-to-end.

### Code Style

- **Rust code:** Follow existing patterns in the file. Use `unwrap()` sparingly — prefer `?` or explicit error handling.
- **Coral code:** Follow the language's own idioms — `is` for binding, no type annotations, indentation-scoped.
- **Comments:** Only where behavior is non-obvious. Don't over-comment.
- **Test names:** Descriptive, prefixed by category: `e2e_feature_name`, `parse_feature_name`, `semantic_feature_name`.

### Agent Navigation Tools

The `tools/` directory contains scripts that help LLM agents navigate the codebase efficiently:

- **`python tools/codemap.py src/`** — Generate a structural map of all source files with function/struct/enum definitions, line numbers, parameters, return types, doc comments, and caller/callee relationships.
- **`python tools/xref.py . --include "*.rs"`** — Build a cross-reference report showing most-referenced symbols, caller graphs, and potentially unused functions.
- **`python tools/codemap.py . --compact`** — Compact mode for a quick overview (no docs/calls).

Run these at the start of a session to orient yourself quickly. See `tools/README.md` for full usage.

### Before Starting Work

1. Read this document
2. Check `docs/EVOLUTION_PROGRESS.md` for current status
3. Check `docs/LANGUAGE_EVOLUTION_ROADMAP.md` for the task details
4. Run `cargo test` to establish baseline (expect 816 pass, 0 failures)
5. Ensure runtime is built: `cargo build -p runtime`
6. Optionally generate a code map: `python tools/codemap.py src/ --compact`

### Decision Making

- If a task has an obvious best approach, take it. Don't ask for confirmation.
- If there are genuine design tradeoffs (e.g., two incompatible syntax options), present the options with a recommendation and ask.
- If a task is blocked by a prerequisite, note it and move to the next task.
- If tests break due to your change, fix them before proceeding.

### Progress Updates

When asked to update progress:
- Update the test baseline count in `EVOLUTION_PROGRESS.md`
- Mark completed tasks in the appropriate section
- Add a session log entry with detailed notes on what was done
- Be specific about what files were changed and why
