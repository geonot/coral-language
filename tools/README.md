# Agent Tools

Helper scripts that improve LLM agent efficiency when working on this codebase.

## coral-dev — All-in-One Helper Script

The primary tool for agents and humans. Wraps build, test, run, search,
navigation, and documentation workflows into short subcommands.

```bash
./tools/coral-dev help              # Show all commands
./tools/coral-dev test summary      # Run tests, one-line pass/fail
./tools/coral-dev test failures     # Show only failing tests
./tools/coral-dev test one my_test  # Run single test with output
./tools/coral-dev test grep pattern # Run tests matching a pattern
./tools/coral-dev test e2e          # End-to-end tests only
./tools/coral-dev run file.coral    # Compile + run via JIT
./tools/coral-dev compile file.coral # Compile to LLVM IR only
./tools/coral-dev check             # Quick build error check
./tools/coral-dev codemap compact   # Structural code map
./tools/coral-dev xref rust         # Cross-reference report
./tools/coral-dev find sym name     # Find symbol definition
./tools/coral-dev find callers fn   # Who calls this function?
./tools/coral-dev find text pattern # Full-text search
./tools/coral-dev status            # Git + test + line count
./tools/coral-dev baseline update   # Update test count in docs
./tools/coral-dev onboard quick     # Fast agent context loading
```

For shell function aliases, source the env file:

```bash
source tools/coral-dev.env
ct summary    # ct = coral test
cr file.coral # cr = coral run
cm compact    # cm = codemap
co quick      # co = onboard
```

## codemap.py — Structural Code Map

Generates a Markdown document mapping every source file's structure: classes,
functions, methods, traits, types, with line numbers, parameters, return types,
doc comments, attributes, and caller/callee relationships.

```bash
# Map entire project
python tools/codemap.py . --output CODEMAP.md

# Map only Rust source files
python tools/codemap.py src/ --include "*.rs"

# Compact mode (no docs/calls — smaller output)
python tools/codemap.py . --compact

# Exclude test files
python tools/codemap.py . --exclude "tests/*" "target/*"
```

**Supported languages:** Rust, Python, TypeScript/JavaScript, Coral, Go, C/C++, Java, Ruby

## xref.py — Cross-Reference / Caller Graph

Builds a caller/callee relationship graph across the codebase. Identifies:
- Most-referenced symbols (hot paths)
- Caller → callee relationships
- Potentially unused functions

```bash
# Full cross-reference
python tools/xref.py . --output XREF.md

# JSON format for programmatic use
python tools/xref.py . --format json

# Only Rust files
python tools/xref.py . --include "*.rs"
```

## Typical Agent Workflow

1. At session start, generate a codemap: `python tools/codemap.py src/ --compact`
2. Use it to locate relevant files and functions before making changes
3. Run xref to understand call relationships: `python tools/xref.py src/ --include "*.rs"`
4. After making changes, verify nothing broke: `cargo test 2>&1 | tail -5`
