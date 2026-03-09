# Coral — Quick Reference for LLM Agents
# This file provides fast context. Read this INSTEAD of browsing multiple docs.

## Build & Test Commands
- Full test:         cargo test 2>&1 | tail -5
- Build runtime:     cargo build -p runtime
- Build compiler:    cargo build
- Release build:     cargo build --release
- Single test:       cargo test test_name_here -- --nocapture
- Run .coral file:   cargo run -- --jit examples/hello.coral
- Emit IR:           cargo run -- --emit-ir output.ll examples/hello.coral

## Test Baseline (update after each session)
- Tests: 816 pass, 0 failures
- Runtime: cargo test -p runtime

## Key File Locations
- Lexer:          src/lexer.rs
- Parser:         src/parser.rs (~2,300 lines — most edits happen here)
- AST:            src/ast.rs
- Semantic:       src/semantic.rs
- Codegen:        src/codegen/mod.rs (~3,100 lines — second most edited)
- Codegen/Builtin:src/codegen/builtins.rs
- Type System:    src/types/solver.rs, src/types/core.rs, src/types/env.rs
- Runtime core:   runtime/src/lib.rs
- Runtime NaN:    runtime/src/nanbox.rs, runtime/src/nanbox_ffi.rs
- Pipeline:       src/compiler.rs
- CLI:            src/main.rs

## Critical Patterns
- Binding: `is` keyword, NEVER = or ==
- Function decl: *name(params)
- Ternary: condition ? true_branch ! false_branch
- Pipeline: expr ~ fn(args)
- Error: err Name:Sub — propagated with ! return err
- Match: match expr / Variant(x) -> body / _ -> default
- Guards: match expr / x when condition -> body
- Or-patterns: match expr / A or B -> body

## Architecture Notes
- NaN-boxing: all values are i64, tagged via NaN payload bits
- LLVM via Inkwell (LLVM 16) — codegen emits LLVM IR
- Runtime is cdylib (libruntime.so) — linked at compile time
- Self-hosted compiler in self_hosted/ must stay in sync with src/

## Helper Script (./tools/coral-dev)
- Test summary:   ./tools/coral-dev test summary
- Test failures:  ./tools/coral-dev test failures
- Single test:    ./tools/coral-dev test one <test_name>
- Test by pattern: ./tools/coral-dev test grep <pattern>
- E2E tests:     ./tools/coral-dev test e2e
- Run .coral:    ./tools/coral-dev run examples/hello.coral
- Compile only:  ./tools/coral-dev compile examples/hello.coral
- Check build:   ./tools/coral-dev check
- Code map:      ./tools/coral-dev codemap compact
- Cross-ref:     ./tools/coral-dev xref rust
- Find symbol:   ./tools/coral-dev find sym <name>
- Find callers:  ./tools/coral-dev find callers <name>
- Find text:     ./tools/coral-dev find text <pattern>
- Goto symbol:   ./tools/coral-dev goto <file> <symbol>
- Extract fn:    ./tools/coral-dev extract <file> <symbol> [-c N]
- File index:    ./tools/coral-dev context <file> [--brief]
- List symbols:  ./tools/coral-dev symbols <file> [--kind fn|enum|...]
- Checklist:     ./tools/coral-dev checklist new-syntax [--enrich]
- Scaffold test: ./tools/coral-dev scaffold e2e <name>
- Parse errors:  ./tools/coral-dev parse-errors build [--format json]
- Project status: ./tools/coral-dev status
- Update baseline: ./tools/coral-dev baseline update
- Onboard:       ./tools/coral-dev onboard quick
- All commands:  ./tools/coral-dev help

## Planning
- Roadmap:    docs/LANGUAGE_EVOLUTION_ROADMAP.md (AUTHORITATIVE)
- Progress:   docs/EVOLUTION_PROGRESS.md
- Onboarding: docs/LLM_ONBOARDING.md
