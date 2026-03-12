# Coral — Quick Reference for LLM Agents
# This file provides fast context. Read this INSTEAD of browsing multiple docs.

## MANDATORY: Use coral-dev Before Writing Code

**DO NOT skip these steps.** The `./tools/coral-dev` script exists specifically
for code navigation and project understanding. Use it INSTEAD of manually
grepping or reading files blind.

### Session Start (do these FIRST, every session)

```bash
# 1. Understand the codebase structure (run this BEFORE reading any file)
./tools/coral-dev codemap compact

# 2. Check project health and test baseline
./tools/coral-dev check

# 3. If you need to understand a specific file's layout:
./tools/coral-dev context src/parser.rs --brief
```

### Before Modifying Any File

```bash
# Find the exact symbol you need to change (gives line numbers):
./tools/coral-dev goto src/codegen/builtins.rs emit_member_call

# Extract the full function body with context:
./tools/coral-dev extract src/codegen/builtins.rs emit_member_call -c 5

# List all symbols in a file by kind:
./tools/coral-dev symbols src/ast.rs --kind enum
```

### When Searching for Code

```bash
# Find a symbol definition across the project:
./tools/coral-dev find sym compile_modules_to_ir

# Find all callers of a function:
./tools/coral-dev find callers emit_expression

# Text search across the project:
./tools/coral-dev find text "GracefulStop"
```

### When Adding New Syntax or Features

```bash
# Get a full checklist of ALL files that need updating:
./tools/coral-dev checklist new-syntax --enrich

# Scaffold test boilerplate:
./tools/coral-dev scaffold e2e my_feature_name
```

### After Making Changes

```bash
# Quick build check (faster than full test):
./tools/coral-dev check

# Run a single test:
./tools/coral-dev test one my_test_name

# Run tests matching a pattern:
./tools/coral-dev test grep "regex"

# Run E2E tests only:
./tools/coral-dev test e2e

# Run runtime tests only:
./tools/coral-dev test runtime
```

---

## Build & Test Commands
- Full test:         cargo test 2>&1 | tail -5
- Build runtime:     cargo build -p runtime
- Build compiler:    cargo build
- Release build:     cargo build --release
- Single test:       cargo test test_name_here -- --nocapture
- Run .coral file:   cargo run -- --jit examples/hello.coral
- Emit IR:           cargo run -- --emit-ir output.ll examples/hello.coral

## Test Baseline (update after each session)
- Compiler tests: ~879 pass, 0 failures
- Runtime tests: 212 pass, 1 pre-existing fail (map_iterator_is_snapshot_after_mutation)
- Total: 1091 pass across all crates
- Runtime: cargo test -p runtime

## Key File Locations
- Lexer:          src/lexer.rs
- Parser:         src/parser.rs (~3,100 lines — most edits happen here)
- AST:            src/ast.rs
- Semantic:       src/semantic.rs (~4,000 lines)
- Codegen:        src/codegen/mod.rs (~3,700 lines — second most edited)
- Codegen/Builtin:src/codegen/builtins.rs (~1,900 lines)
- Codegen/Runtime:src/codegen/runtime.rs (~2,100 lines — runtime bindings)
- Type System:    src/types/solver.rs, src/types/core.rs, src/types/env.rs
- Runtime core:   runtime/src/lib.rs
- Runtime NaN:    runtime/src/nanbox.rs, runtime/src/nanbox_ffi.rs
- Runtime Actors: runtime/src/actor.rs, runtime/src/actor_ops.rs
- Runtime Regex:  runtime/src/regex_ops.rs
- Pipeline:       src/compiler.rs
- CLI:            src/main.rs
- Module Loader:  src/module_loader.rs
- LSP Server:     coral-lsp/src/main.rs

## Critical Patterns
- Binding: `is` keyword, NEVER = or ==
- Function decl: *name(params)
- Ternary: condition ? true_branch ! false_branch
- Pipeline: expr ~ fn(args)
- Error: err Name:Sub — propagated with ! return err
- Match: match expr / Variant(x) -> body / _ -> default
- Guards: match expr / x when condition -> body
- Or-patterns: match expr / A or B -> body
- do..end blocks: `func() do ... end` (trailing lambda)

## Architecture Notes
- NaN-boxing: all values are i64, tagged via NaN payload bits
- LLVM via Inkwell (LLVM 16) — codegen emits LLVM IR
- Runtime is cdylib (libruntime.so) — linked at compile time
- Self-hosted compiler in self_hosted/ must stay in sync with src/
- Incremental compilation cache in .coral-cache/ (CC3.5)
- LTO via --lto flag (C4.4)

## coral-dev Tool Reference

All commands are run from the project root as `./tools/coral-dev <command>`.

### Navigation & Code Understanding
| Command | Purpose |
|---------|---------|
| `codemap compact` | Structural overview of all source files with line counts and symbols |
| `codemap full` | Detailed codemap with all symbol signatures |
| `context <file> [--brief]` | Structural index of a single file |
| `symbols <file> [--kind K]` | List symbols in a file (kinds: fn, enum, struct, impl, etc.) |
| `goto <file> <symbol>` | Find exact line range for a symbol |
| `extract <file> <symbol> [-c N]` | Extract function body with N lines of context |
| `find sym <name>` | Find symbol definition across the project |
| `find callers <name>` | Find all callers of a function |
| `find text <pattern>` | Grep-like search across the project |
| `find test <pattern>` | Find tests matching a pattern |
| `xref rust` | Cross-reference report of all Rust symbols |

### Build & Test
| Command | Purpose |
|---------|---------|
| `check` | Quick build check — are there compile errors? |
| `test one <name>` | Run a single test with full output |
| `test grep <pattern>` | Run tests matching a pattern |
| `test e2e` | Run E2E tests only |
| `test runtime` | Run runtime tests only |
| `test failures` | Show only failing tests |
| `test count` | Show test count without running |

### Scaffolding & Checklists
| Command | Purpose |
|---------|---------|
| `checklist new-syntax [--enrich]` | Checklist for adding new syntax (lexer, parser, AST, etc.) |
| `checklist new-builtin [--enrich]` | Checklist for adding a new builtin function |
| `checklist new-type` | Checklist for adding a new type system feature |
| `checklist fix-bug` | Checklist for fixing a bug |
| `scaffold e2e <name>` | Generate E2E test boilerplate |
| `scaffold codegen <name>` | Generate codegen test boilerplate |
| `scaffold parser <name>` | Generate parser test boilerplate |

### Project Management
| Command | Purpose |
|---------|---------|
| `status` | Full project health check (git, lines, tests) |
| `baseline show` | Show current test baseline |
| `baseline update` | Update test baseline in docs |
| `diff staged` | Show staged changes |
| `diff all` | Show all uncommitted changes |
| `onboard quick` | Print agent onboarding context (this file content) |

## Planning
- Roadmap:    docs/LANGUAGE_EVOLUTION_ROADMAP.md (AUTHORITATIVE)
- Progress:   docs/EVOLUTION_PROGRESS.md
- Onboarding: docs/LLM_ONBOARDING.md
