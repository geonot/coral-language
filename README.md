# Coral Programming Language

A programming language combining Python-like ergonomics with C/Rust-level performance, featuring built-in actors, persistent stores, and automatic memory management via reference counting with cycle detection.

**Status**: Pre-Alpha — 406 tests passing (0 failures), 121 end-to-end execution tests, JIT and native binary compilation working.

## Quick Start

```bash
# Build the compiler and runtime
cargo build
cargo build -p runtime --release

# Run a program via JIT
./target/debug/coralc --jit examples/hello.coral

# Compile to native binary
./target/debug/coralc examples/hello.coral --emit-binary ./hello
./hello

# Run all tests
cargo test
```

## Language Overview

Coral uses indentation-based syntax with `is` for binding, `*` for function declarations, and implicit returns:

```coral
*main()
    name is 'World'
    log('Hello, {name}!')

    fruits is ['apple', 'banana', 'cherry']
    log('First: {fruits[0]}')

    config is map('host' is 'localhost', 'port' is 8080)
    log('Host: {config.get("host")}')

    status is 42 > 40 ? 'big' ! 'small'
    log('Answer is {status}')
```

### Key Language Features

| Feature | Status | Notes |
|---------|--------|-------|
| Functions (`*name(args)`) | Working | Implicit return, closures, higher-order |
| Variables (`name is expr`) | Working | Rebindable, alloca-based |
| Control flow (if/elif/else, while, for..in) | Working | Full codegen with PHI nodes |
| Pattern matching (`match`) | Working | ADTs, literals, wildcards, nested patterns |
| Algebraic data types (`type`) | Working | Variants with fields, exhaustiveness checking |
| Stores (mutable objects) | Working | Field access/mutation via `self.field` |
| Traits | Working | Default methods, required methods, store/type implementation |
| Actors | Working | Spawn, send, named actors, timers, supervision |
| Lists & Maps | Working | Literals, push/pop/get/set, map/filter/reduce |
| Template strings | Working | `'Value: {expr}'` with auto-coercion |
| Pipeline operator (`~`) | Partial | Parses, basic codegen |
| Persistent stores | Partial | Runtime WAL + dual-format storage; codegen incomplete |
| Module system (`use`) | Working | Text-based expansion from `std/` directory |
| Guard statements | Working | `cond ? stmt` as shorthand for `if cond { stmt }` |

### Design Principles

- **Pure type inference** — no type annotations anywhere in syntax
- **`is` for binding** — `=` and `==` are not valid tokens (helpful errors guide users)
- **Method-based equality** — `a.equals(b)` / `a.not_equals(b)` instead of `==`/`!=`
- **Single numeric type** — `Number(f64)` at runtime; `Int`/`Float` distinction only in AST for const-folding

## Architecture

### Compiler Pipeline (`src/`)

```
Source → Lexer → Parser → Semantic → Lower → Codegen → LLVM IR
              ↘ MIR (const-folding, limited)
```

| Component | File | Lines | Description |
|-----------|------|-------|-------------|
| Lexer | `src/lexer.rs` | ~900 | Indent-aware, layout tokens (INDENT/DEDENT/NEWLINE) |
| Parser | `src/parser.rs` | ~1,700 | Recursive-descent, all expression forms |
| AST | `src/ast.rs` | ~350 | Typed AST with all statement/expression variants |
| Semantic | `src/semantic.rs` | ~1,200 | Forward refs, scope checking, type inference |
| Type System | `src/types/` | ~1,500 | Constraint solver, unification, ADT types |
| Lowering | `src/lower.rs` | ~400 | Placeholder-to-lambda, IR preparation |
| Codegen | `src/codegen/` | ~5,900 | LLVM IR emission via Inkwell |
| MIR | `src/mir*.rs` | ~650 | Minimal const-evaluator (limited) |
| Module Loader | `src/module_loader.rs` | ~250 | `use` directive expansion |
| CLI | `src/main.rs` | ~330 | coralc binary with JIT/binary/IR emission |

### Runtime (`runtime/src/`)

~23,000 lines of Rust implementing:

- **Tagged value system** — refcounted `Value` with NaN-boxing-style inline storage
- **Reference counting** — CAS-based release, thread-local value pools, iterative drop
- **Cycle detector** — mark-gray/scan/collect-white with lock-guarded safety
- **175+ FFI functions** — `coral_make_*`, `coral_list_*`, `coral_map_*`, arithmetic, comparison, etc.
- **Actor system** — work queue, scheduler, named registry, timers, supervision
- **Persistent store** — WAL engine, JSONL + binary formats, field indexing
- **Runtime telemetry** — allocator stats via `CORAL_RUNTIME_METRICS` env var

### Standard Library (`std/`)

~950 lines of Coral across 16 modules. Most modules are thin wrappers around runtime builtins. See [docs/STDLIB_STATUS.md](docs/STDLIB_STATUS.md) for per-module assessment.

### Self-Hosted Compiler (`self_hosted/`)

~2,165 lines of Coral implementing a front-end (lexer + parser). See [docs/SELF_HOSTING_STATUS.md](docs/SELF_HOSTING_STATUS.md) for details.

## CLI Usage

```bash
# Emit LLVM IR to stdout
./target/debug/coralc program.coral

# Emit IR to file
./target/debug/coralc program.coral --emit-ir out.ll

# JIT execution via lli (auto-builds runtime if needed)
./target/debug/coralc --jit program.coral

# Native binary via llc + clang
./target/debug/coralc program.coral --emit-binary ./program

# Runtime telemetry
./target/debug/coralc --jit program.coral --collect-metrics metrics.json

# Override tool paths
./target/debug/coralc --jit program.coral --lli /usr/bin/lli-16 --runtime-lib ./libruntime.so
```

### Manual Workflows

1. **JIT**: `cargo build -p runtime --release` → `coralc program.coral --emit-ir /tmp/prog.ll` → `lli -load target/release/libruntime.so /tmp/prog.ll`
2. **Native**: `llc -filetype=obj /tmp/prog.ll -o /tmp/prog.o` → `clang /tmp/prog.o -L target/release -l runtime -Wl,-rpath,$PWD/target/release -o ./program`

## Modules

```coral
use std.prelude
use std.math

*main()
    log('2π = {2.0 * pi}')
```

- `use std.X` resolves to `std/X.coral`
- Text-based expansion before parsing

## Examples

| Example | Status | Notes |
|---------|--------|-------|
| `hello.coral` | Runs | Variables, lists, maps, ternaries, template strings |
| `calculator.coral` | Runs | Arithmetic, match, conditionals |
| `traits_demo.coral` | Runs | Trait definitions, implementations, default methods |
| `data_pipeline.coral` | Compiles | Store construction, iteration; some runtime display issues |
| `fizzbuzz.coral` | Parse error | Tuple pattern `(true, true)` not yet supported |
| `chat_server.coral` | Lex error | Indentation issue in source file |
| `http_server.coral` | Lex error | Indentation issue in source file |

## Documentation

All documentation is in [docs/](docs/). See [docs/README.md](docs/README.md) for the full index.

Key documents:
- [docs/ROADMAP_TO_ALPHA.md](docs/ROADMAP_TO_ALPHA.md) — Outstanding issues and path to alpha release
- [docs/SELF_HOSTING_STATUS.md](docs/SELF_HOSTING_STATUS.md) — Self-hosted compiler progress
- [docs/STDLIB_STATUS.md](docs/STDLIB_STATUS.md) — Standard library completeness
- [REMEDIATION_TRACKER.md](REMEDIATION_TRACKER.md) — Detailed bug fix history and current status

## Project Layout

```
coral/
├── src/                    # Compiler (Rust, ~15,500 lines)
│   ├── lexer.rs            # Indent-aware tokenizer
│   ├── parser.rs           # Recursive-descent parser
│   ├── ast.rs              # AST definitions
│   ├── semantic.rs         # Semantic analysis + type inference
│   ├── types/              # Type system (core, env, solver)
│   ├── codegen/            # LLVM IR generation (mod.rs, runtime.rs)
│   ├── lower.rs            # Placeholder desugaring
│   ├── mir*.rs             # MIR const-evaluator
│   ├── compiler.rs         # Pipeline orchestration
│   ├── main.rs             # CLI (coralc)
│   └── module_loader.rs    # use-directive resolution
├── runtime/                # Runtime library (Rust, ~23,000 lines)
│   └── src/                # Tagged values, RC, actors, stores
├── std/                    # Standard library (Coral, ~950 lines)
├── self_hosted/            # Self-hosted compiler (Coral, ~2,165 lines)
├── examples/               # Example programs
├── tests/                  # Integration tests (~7,500 lines)
├── docs/                   # Documentation
├── tree-sitter-coral/      # Tree-sitter grammar
└── vscode-coral/           # VS Code extension
```
