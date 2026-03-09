# Coral — Quick Reference for LLM Agents
# This file provides fast context. Read this INSTEAD of browsing multiple docs.

## Build & Test Commands
- Full test:         cargo test 2>&1 | tail -5
- Build runtime:     cargo build -p runtime
- Build compiler:    cargo build
- Release build:     cargo build --release
- Single test:       cargo test test_name_here -- --nocapture
- Run .coral file:   cargo run -- compile examples/hello.coral && lli output.ll

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

## Navigation Tools
- Code map:   python tools/codemap.py src/ --compact
- Xref:       python tools/xref.py . --include "*.rs"
- Full map:   python tools/codemap.py . --output CODEMAP.md

## Planning
- Roadmap:    docs/LANGUAGE_EVOLUTION_ROADMAP.md (AUTHORITATIVE)
- Progress:   docs/EVOLUTION_PROGRESS.md
- Onboarding: docs/LLM_ONBOARDING.md
