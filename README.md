# Coral Programming Language

An experimental programming language combining Python-like ergonomics with C/Rust-level performance, featuring built-in actors, persistent stores, and automatic memory management.

**Status**: Pre-Alpha (102 tests passing, core features working)

## Documentation

| Document | Description |
|----------|-------------|
| [docs/ALPHA_ROADMAP.md](docs/ALPHA_ROADMAP.md) | Current state and path to alpha release |
| [docs/TECHNICAL_DEBT.md](docs/TECHNICAL_DEBT.md) | Known issues and technical debt |
| [docs/STANDARD_LIBRARY_SPEC.md](docs/STANDARD_LIBRARY_SPEC.md) | Standard library specification |
| [docs/ACTOR_SYSTEM_COMPLETION.md](docs/ACTOR_SYSTEM_COMPLETION.md) | Actor system completion plan |
| [docs/PERSISTENT_STORE_SPEC.md](docs/PERSISTENT_STORE_SPEC.md) | Persistent storage specification |
| [docs/SELF_HOSTED_COMPILER_SPEC.md](docs/SELF_HOSTED_COMPILER_SPEC.md) | Self-hosting compiler spec |
| [docs/SELF_HOSTED_RUNTIME_SPEC.md](docs/SELF_HOSTED_RUNTIME_SPEC.md) | Self-hosting runtime spec |

## Implementation

The compiler is implemented in Rust with LLVM backend via [Inkwell](https://github.com/InkwellLang/Inkwell).

## Features

- **Indent-aware lexer** that understands Coral keywords (`type`, `store`, `actor`, `match`, `is`, etc.), literals (ints, floats, booleans, single/double quoted strings, template strings), placeholder tokens (`$`, `$1`, …), taxonomy prefixes (`!!`), and layout tokens (`INDENT`, `DEDENT`, `NEWLINE`). It rejects misaligned dedents *and* mixed tab/space indentation so layout errors are caught before parsing.
- **Recursive-descent parser** with support for:
  - Top-level bindings (`name is expr`), functions (`*name(args)`), types/stores, taxonomy trees, and match expressions.
  - Expression grammar with arithmetic, comparisons, ternaries (`cond ? a ! b`), member access, template-string interpolation, taxonomy literals (`!!A:B:C`), lambda literals, placeholders, and list/map literals.
- **Parser fixture harness** (`tests/fixtures/parser`) that feeds real Coral files—both valid and failing—through the lexer/parser, backed by substring diagnostics for invalid cases and JSON AST snapshots (`tests/parser_snapshots.rs`) for valid ones.
- **Lowering + semantic model**: a dedicated placeholder-to-lambda pass rewrites `$`/`$1` expressions into synthesized lambdas so later stages never see shorthand placeholders. Semantics validates duplicate functions/bindings/parameters, enforces parameter default ordering rules, and prepares IR-friendly collections of globals/functions.
- **LLVM code generation** that lowers strings, template literals, taxonomy values, lists, maps, logical operators, and numeric code to LLVM IR by calling into the tagged `Value` runtime. Functions still return the last expression in their block, similar to Coral's implicit return style.
- **Runtime crate (`runtime/`)** exposing refcounted tagged values plus helpers such as `coral_make_string`, `coral_make_list`, `coral_map_get/set`, `coral_list_push/length/get/pop`, `coral_map_length`, and arithmetic/equality shims.
- **CLI (`coralc`)** that reads a `.coral` file and prints or writes the generated LLVM IR.

## Usage

```bash
# Build
cargo build

# Emit LLVM IR to stdout
cargo run -- path/to/source.coral

# Emit LLVM IR to a file
cargo run -- path/to/source.coral --emit-ir out.ll

# Run the smoke tests
cargo test

# Compile a program that uses std modules
cargo run -- examples/hello.coral --jit

# Profile a workload and collect runtime metrics for feedback
cargo run -- examples/hello.coral --jit --collect-metrics metrics.json

# Runtime stress under sanitizers (nightly)
cargo asan-run
cargo asan-test
cargo miri-test
```

### Running the generated LLVM IR

`coralc` now provides optional post-processing hooks so you can stay in one command-line flow, along with manual steps if you prefer to drive the LLVM tools yourself.

#### Built-in pipeline integrations

Add `--jit` to run the compiled IR through `lli`, or `--emit-binary <FILE>` to produce a native executable via `llc` + `clang`. Both options automatically:

- Invoke `cargo build -p runtime --release` (if necessary) so the shared runtime library is available.
- Default to `target/release/libruntime.{so|dylib|dll}`, with overrides through `--runtime-lib PATH`.
- Allow tool overrides: `--lli`, `--llc`, and `--clang` accept explicit executable paths.

Examples:

```bash
# Print IR and immediately run it via lli, preloading the runtime
cargo run -- program.coral --jit

# Emit IR, link a native binary, and skip printing IR
cargo run -- program.coral --emit-ir /tmp/program.ll --emit-binary ./program

# Run with lli and emit allocator telemetry
cargo run -- program.coral --jit --collect-metrics /tmp/coral-metrics.json
```

#### Manual workflows (if you want full control)

1. **JIT with `lli` and the runtime shared library**
  1. Build the runtime crate as a shared library: `cargo build -p runtime --release` (produces `target/release/libruntime.so` on Linux, `.dylib` on macOS, `.dll` on Windows).
  2. Emit IR to a file: `cargo run -- program.coral --emit-ir /tmp/program.ll`.
  3. Run the IR through LLVM's interpreter while preloading the runtime: `lli -load target/release/libruntime.so /tmp/program.ll`.

2. **Native binary via `llc` + `clang`**
  1. Build the runtime crate as above so the linker can find `libruntime`.
  2. Lower the IR to an object file: `llc -filetype=obj /tmp/program.ll -o /tmp/program.o`.
  3. Link it into an executable, pointing the linker at the runtime artifacts: `clang /tmp/program.o -L target/release -l runtime -o /tmp/program` (add `-Wl,-rpath,$PWD/target/release` or set `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH` so the binary can locate the shared library at runtime).
  4. Execute `/tmp/program` to see the `log(...)` output and final return value.

### Modules and the standard library

Coral supports lightweight imports via `use module_name`, which is resolved to `module_name.coral` relative to the caller
and the bundled `std/` directory. Module expansion happens before parsing, so you can keep writing plain Coral without a
new AST node. The CLI automatically injects the standard paths; tools and tests can mirror this behavior with
`ModuleLoader::with_default_std()`.

```coral
use std.prelude
use math.helpers

*main()
    log_line('hello from modules')
    2 * pi
```

Add `.coral` files under `std/` to grow the standard library. Nested module names like `std.math.stats` map to
`std/math/stats.coral`.

> **Note:** All non-store/actor constructs from `syntax.coral` now parse and lower to LLVM IR, including template strings, taxonomy literals, and placeholder-driven lambdas. Stores/actors, higher-order list ops (`map`, `reduce`), and closures still parse but do not emit runnable LLVM yet; keep those constructs at the surface until Milestones A3.2/B2/B3 land.

## Runtime Telemetry

- Set `CORAL_RUNTIME_METRICS=/absolute/path.json` before running generated programs (or pass `--collect-metrics` while using `--jit`) to collect allocator stats such as retain/release counts, value-pool hit rates, heap bytes, list/map slot counts, and stack arena usage.
- The runtime writes the JSON snapshot at process exit and also exposes `coral_runtime_metrics`/`coral_runtime_metrics_dump` for programmatic sampling. See `docs/runtime_metrics.md` for field descriptions and integration ideas.

## Project Layout

- `about.md`, `syntax.coral` – Source inspiration describing Coral's philosophy and syntax.
- `src/lexer.rs` – Hand-written lexer with indentation tracking.
- `src/parser.rs` – Recursive-descent parser that builds the typed AST in `src/ast.rs`.
- `src/semantic.rs` – Light semantic analysis and IR preparation.
- `src/codegen.rs` – LLVM IR emission via Inkwell.
- `src/compiler.rs` – Pipeline glue code.
- `src/main.rs` – Command-line interface.
- `docs/overview.md` – Single-source architecture + runtime overview with current gaps.
- `docs/roadmap.md` – Living delivery plan with NOW/NEXT/LATER goals.
- `runtime/` – Companion crate exposing the tagged `Value` type and FFI hooks (`coral_make_*`, `coral_string_concat`, etc.) used by future codegen stages.

## Next Steps

- Finish parser layout recovery + diagnostics (Plan B1.2) and grow the negative fixture matrix (`tests/parser/*.coral`).
- Expand semantic checks with symbol resolution + undefined-name diagnostics, and bootstrap ordered contexts for minimal type inference.
- Add property/benchmark coverage for the runtime (`coral_value_equals`, list/map builders) plus leak/perf audits.
- Implement closure/placeholder runtime (`coral_make_closure`, invoke shims) so higher-order list APIs (`map`, `reduce`) can execute; then lower stores/actors to LLVM structs/methods and design the actor runtime.
- Allow emitting native binaries by chaining `llc`/`clang`.
