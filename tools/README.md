# Agent Tools

Helper scripts that improve LLM agent efficiency when working on this codebase.

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
