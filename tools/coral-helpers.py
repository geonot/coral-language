#!/usr/bin/env python3
"""
coral-helpers.py — Advanced code navigation helpers for LLM agents.

Reuses codemap.py's parsers for accurate symbol extraction rather than
reimplementing regex patterns. Provides:

  - goto:         Find line range for a symbol (for targeted read_file calls)
  - extract:      Print a function/symbol body with exact line numbers
  - context:      Structural index of a file (functions, impls, types with lines)
  - symbols:      Quick symbol listing for a file, with optional kind filter
  - checklist:    Edit checklist for common tasks (new-syntax, new-builtin, etc.)
  - scaffold:     Generate test boilerplate for different test categories
  - parse-errors: Parse cargo build/test output into structured error list

Usage:
    python tools/coral-helpers.py <command> [args...]
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ROOT / "tools"

# Import codemap.py's parsers and data structures
sys.path.insert(0, str(TOOLS))
from codemap import (
    PARSERS,
    Symbol,
    FileMap,
    detect_language,
    format_symbol,
)


# ──────────────────────────────────────────────────────────────────────
# Shared: parse a file into a FileMap using codemap's parsers
# ──────────────────────────────────────────────────────────────────────

def resolve_path(filepath: str) -> Path:
    """Resolve a filepath relative to ROOT, with fuzzy fallback."""
    path = ROOT / filepath
    if path.exists():
        return path
    # Try rglob for partial matches (skip target/)
    candidates = [c for c in ROOT.rglob(filepath) if "target" not in str(c)]
    if candidates:
        return candidates[0]
    # Try just the basename
    basename = Path(filepath).name
    candidates = [c for c in ROOT.rglob(basename) if "target" not in str(c)]
    if len(candidates) == 1:
        return candidates[0]
    return path  # let caller handle the error


def parse_file(filepath: str) -> FileMap:
    """Parse a file using codemap's language-specific parser."""
    path = resolve_path(filepath)
    if not path.exists():
        print(f"File not found: {filepath}", file=sys.stderr)
        sys.exit(1)

    lang = detect_language(str(path))
    if not lang or lang not in PARSERS:
        print(f"Unsupported language for {path.suffix}", file=sys.stderr)
        sys.exit(1)

    content = path.read_text(encoding="utf-8", errors="replace")
    rel = str(path.relative_to(ROOT))
    return PARSERS[lang].parse(rel, content)


def find_symbols(fm: FileMap, name: str) -> list:
    """Find all symbols matching `name` in a FileMap (searches children too)."""
    results = []
    for sym in fm.symbols:
        if sym.name == name:
            results.append(sym)
        # Also search inside impl blocks, traits, classes
        for child in sym.children:
            if child.name == name:
                child.parent = sym.name
                results.append(child)
    return results


# ──────────────────────────────────────────────────────────────────────
# goto — Find exact line range for a symbol
# ──────────────────────────────────────────────────────────────────────

def cmd_goto(args):
    """Find symbol location: goto <file> <symbol_name>"""
    fm = parse_file(args.file)
    matches = find_symbols(fm, args.symbol)

    if not matches:
        # Fuzzy: try substring match
        for sym in fm.symbols:
            if args.symbol in sym.name:
                matches.append(sym)
            for child in sym.children:
                if args.symbol in child.name:
                    child.parent = sym.name
                    matches.append(child)

    if not matches:
        print(f"Symbol '{args.symbol}' not found in {args.file}", file=sys.stderr)
        # Show available symbols as hints
        all_names = []
        for sym in fm.symbols:
            all_names.append(sym.name)
            for c in sym.children:
                all_names.append(f"  {sym.name}::{c.name}")
        if all_names:
            print(f"Available symbols ({len(all_names)}):", file=sys.stderr)
            for n in all_names[:30]:
                print(f"  {n}", file=sys.stderr)
            if len(all_names) > 30:
                print(f"  ... and {len(all_names) - 30} more", file=sys.stderr)
        sys.exit(1)

    for sym in matches:
        end = sym.end_line or sym.line
        span = f"L{sym.line}-L{end}" if end > sym.line else f"L{sym.line}"
        size = end - sym.line + 1
        parent_tag = f"  in {sym.parent}" if sym.parent else ""
        vis_tag = f" [{sym.visibility}]" if sym.visibility else ""

        sig = sym.name
        if sym.params is not None:
            p = sym.params[:80] + "..." if len(sym.params) > 80 else sym.params
            sig += f"({p})"
        if sym.return_type:
            sig += f" -> {sym.return_type}"

        print(f"{fm.path}  {span}  ({size} lines)  [{sym.display_kind}]{vis_tag}{parent_tag}")
        print(f"  {sig}")
        if sym.doc:
            print(f"  /// {sym.doc[:120]}")


# ──────────────────────────────────────────────────────────────────────
# extract — Print a function/symbol body with exact lines
# ──────────────────────────────────────────────────────────────────────

def cmd_extract(args):
    """Extract a function body: extract <file> <symbol_name> [--context N]"""
    fm = parse_file(args.file)
    matches = find_symbols(fm, args.symbol)

    if not matches:
        print(f"Symbol '{args.symbol}' not found in {args.file}", file=sys.stderr)
        print(f"Tip: use 'goto' to search by substring or list symbols.", file=sys.stderr)
        sys.exit(1)

    # If --which is specified, pick that match
    sym = matches[0]
    if hasattr(args, "which") and args.which is not None and args.which < len(matches):
        sym = matches[args.which]

    ctx = args.context if hasattr(args, "context") and args.context else 0
    path = resolve_path(args.file)
    all_lines = path.read_text(encoding="utf-8", errors="replace").split("\n")

    start_line = sym.line
    end_line = sym.end_line or sym.line

    disp_start = max(1, start_line - ctx)
    disp_end = min(len(all_lines), end_line + ctx)

    parent_tag = f" in {sym.parent}" if sym.parent else ""
    print(f"# {fm.path}  L{start_line}-L{end_line}  [{sym.display_kind}] {sym.name}{parent_tag}")
    if len(matches) > 1:
        print(f"# {len(matches)} matches found, showing first (use --which N to pick)")
    print(f"# Showing L{disp_start}-L{disp_end}")
    print()
    for i in range(disp_start - 1, disp_end):
        marker = " " if i + 1 < start_line or i + 1 > end_line else "|"
        print(f"{i+1:>5} {marker} {all_lines[i]}")


# ──────────────────────────────────────────────────────────────────────
# context — Structural index of a file (powered by codemap)
# ──────────────────────────────────────────────────────────────────────

def cmd_context(args):
    """Show structural index for a file using codemap's parser."""
    fm = parse_file(args.file)

    print(f"# {fm.path}  ({fm.line_count} lines, {fm.language})")
    if fm.description:
        print(f"# {fm.description[:200]}")
    print()

    if fm.imports:
        print(f"## Imports ({len(fm.imports)})")
        for imp in fm.imports[:20]:
            print(f"  {imp}")
        if len(fm.imports) > 20:
            print(f"  ... +{len(fm.imports) - 20} more")
        print()

    if args.brief:
        # Compact: just kind, name, line range
        for sym in fm.symbols:
            end = sym.end_line or sym.line
            span = f"L{sym.line:>4}-L{end:<4}" if end > sym.line else f"L{sym.line:>4}      "
            vis = f" [{sym.visibility}]" if sym.visibility else ""
            print(f"  {span}  {sym.display_kind:6s} {sym.name}{vis}")
            for child in sym.children:
                cend = child.end_line or child.line
                cspan = f"L{child.line:>4}-L{cend:<4}" if cend > child.line else f"L{child.line:>4}      "
                cvis = f" [{child.visibility}]" if child.visibility else ""
                print(f"    {cspan}  {child.display_kind:6s} {child.name}{cvis}")
    else:
        # Full: use codemap's markdown format_symbol
        for sym in fm.symbols:
            for line in format_symbol(sym, indent=0, file_path=fm.path):
                print(line)


# ──────────────────────────────────────────────────────────────────────
# symbols — Quick symbol listing
# ──────────────────────────────────────────────────────────────────────

def cmd_symbols(args):
    """List all symbols in a file, optionally filtered by kind."""
    fm = parse_file(args.file)
    kind_filter = args.kind if hasattr(args, "kind") and args.kind else None

    print(f"# {fm.path}  ({fm.line_count} lines, {fm.language})")
    print()

    count = 0
    for sym in fm.symbols:
        if kind_filter and sym.kind != kind_filter:
            # Still check children
            for child in sym.children:
                if child.kind == kind_filter:
                    cend = child.end_line or child.line
                    cspan = f"L{child.line}-L{cend}" if cend > child.line else f"L{child.line}"
                    cvis = f" [{child.visibility}]" if child.visibility else ""
                    print(f"    {cspan:14s}  {child.display_kind:6s} {child.name}{cvis}  in {sym.name}")
                    count += 1
            continue
        end = sym.end_line or sym.line
        span = f"L{sym.line}-L{end}" if end > sym.line else f"L{sym.line}"
        vis = f" [{sym.visibility}]" if sym.visibility else ""
        print(f"  {span:14s}  {sym.display_kind:6s} {sym.name}{vis}")
        count += 1

        if not kind_filter:
            for child in sym.children:
                cend = child.end_line or child.line
                cspan = f"L{child.line}-L{cend}" if cend > child.line else f"L{child.line}"
                cvis = f" [{child.visibility}]" if child.visibility else ""
                print(f"    {cspan:14s}  {child.display_kind:6s} {child.name}{cvis}")
                count += 1

    print(f"\n  {count} symbols total")


# ──────────────────────────────────────────────────────────────────────
# checklist — Edit checklist for common tasks
# ──────────────────────────────────────────────────────────────────────

CHECKLISTS = {
    "new-syntax": {
        "title": "Add New Syntax / Language Feature",
        "steps": [
            ("src/lexer.rs",         "Add new token(s) to Token enum and lexer rules"),
            ("src/parser.rs",        "Add parsing logic (new parse_* method or extend existing)"),
            ("src/ast.rs",           "Add AST node variant(s) to Expression/Statement/Item enums"),
            ("src/semantic.rs",      "Add semantic checking (type validation, scope rules)"),
            ("src/lower.rs",         "Add lowering/desugaring if needed"),
            ("src/codegen/mod.rs",   "Add LLVM IR generation for the new construct"),
            ("self_hosted/lexer.coral",    "Mirror token changes in self-hosted lexer"),
            ("self_hosted/parser.coral",   "Mirror parsing in self-hosted parser"),
            ("self_hosted/semantic.coral",  "Mirror semantic checks in self-hosted"),
            ("self_hosted/codegen.coral",   "Mirror codegen in self-hosted"),
            ("tests/",              "Add tests: parser, semantic, codegen, e2e execution"),
            ("docs/syntax.coral",   "Update syntax reference if applicable"),
        ],
    },
    "new-builtin": {
        "title": "Add New Builtin Function",
        "steps": [
            ("runtime/src/lib.rs",          "Add FFI function implementation"),
            ("src/codegen/runtime.rs",      "Declare the FFI function binding"),
            ("src/codegen/builtins.rs",     "Add dispatch case in compile_builtin_call()"),
            ("src/semantic.rs",             "Register builtin in known functions (if needed)"),
            ("self_hosted/codegen.coral",   "Mirror builtin dispatch in self-hosted"),
            ("tests/",                      "Add e2e test calling the new builtin"),
        ],
    },
    "new-runtime-op": {
        "title": "Add New Runtime Operation",
        "steps": [
            ("runtime/src/<module>.rs",     "Implement the operation as #[no_mangle] extern fn"),
            ("runtime/src/lib.rs",          "Re-export if needed"),
            ("src/codegen/runtime.rs",      "Declare the FFI binding in RuntimeBindings"),
            ("src/codegen/mod.rs",          "Call the operation from codegen"),
            ("self_hosted/codegen.coral",   "Mirror in self-hosted codegen"),
            ("tests/",                      "Add tests"),
        ],
    },
    "new-type": {
        "title": "Add New Type / ADT Variant",
        "steps": [
            ("src/ast.rs",           "Add TypeDefinition or extend existing"),
            ("src/parser.rs",        "Parse the type definition syntax"),
            ("src/semantic.rs",      "Add type checking and validation"),
            ("src/types/core.rs",    "Add TypeId variant if needed"),
            ("src/types/solver.rs",  "Update type solver constraints"),
            ("src/codegen/mod.rs",   "Add codegen for construction and access"),
            ("runtime/src/nanbox.rs","Add NaN-box tag if new heap type"),
            ("runtime/src/lib.rs",   "Add runtime support functions"),
            ("tests/",              "Add tests across all phases"),
        ],
    },
    "new-optimization": {
        "title": "Add Compiler Optimization",
        "steps": [
            ("src/compiler.rs",      "Add optimization pass to pipeline (or inline in codegen)"),
            ("src/codegen/mod.rs",   "Implement optimization in codegen if IR-level"),
            ("tests/",              "Add tests verifying optimization fires + correctness"),
        ],
    },
    "new-test": {
        "title": "Add New Tests",
        "steps": [
            ("tests/execution.rs",         "E2E tests (compile -> lli -> assert stdout)"),
            ("tests/codegen_extended.rs",  "IR compilation tests (compile_to_ir -> verify)"),
            ("tests/semantic.rs",          "Semantic analysis tests (check errors/warnings)"),
            ("tests/parser_extended.rs",   "Parser tests (parse -> assert AST)"),
            ("tests/",                     "Choose the right file for the test category"),
        ],
    },
    "fix-bug": {
        "title": "Fix a Bug",
        "steps": [
            ("(identify)",          "Write a minimal failing test that reproduces the bug"),
            ("(diagnose)",          "Use coral-dev find/grep to locate the root cause"),
            ("(fix)",               "Apply the minimal fix"),
            ("(verify)",            "Run the failing test: coral-dev test one <name>"),
            ("(regression)",        "Run full suite: coral-dev test summary"),
        ],
    },
}


def cmd_checklist(args):
    """Show an edit checklist: checklist <task-type>"""
    task = args.task
    if task == "list" or task is None:
        print("Available checklists:")
        for key, val in CHECKLISTS.items():
            print(f"  {key:20s} -- {val['title']}")
        return

    if task not in CHECKLISTS:
        print(f"Unknown checklist: {task}", file=sys.stderr)
        print(f"Available: {', '.join(CHECKLISTS.keys())}", file=sys.stderr)
        sys.exit(1)

    cl = CHECKLISTS[task]
    print(f"# {cl['title']}")
    print()

    # For each step, if the file exists, use codemap to show what's already
    # there (symbol count, key functions) for quick orientation
    for i, (filepath, desc) in enumerate(cl["steps"], 1):
        print(f"  {i:>2}. [ ] {filepath}")
        print(f"       {desc}")

        # Enrich with structural info from codemap when --enrich is set
        if args.enrich:
            real_path = ROOT / filepath
            if real_path.exists() and real_path.is_file():
                lang = detect_language(str(real_path))
                if lang and lang in PARSERS:
                    try:
                        content = real_path.read_text(encoding="utf-8", errors="replace")
                        fm = PARSERS[lang].parse(filepath, content)
                        n_syms = len(fm.symbols)
                        n_lines = fm.line_count
                        top_syms = [s.name for s in fm.symbols[:8]]
                        sym_list = ", ".join(top_syms)
                        if len(fm.symbols) > 8:
                            sym_list += f" (+{len(fm.symbols) - 8})"
                        print(f"       [{n_lines} lines, {n_syms} top-level symbols: {sym_list}]")
                    except Exception:
                        pass

    print()
    print("Run `coral-dev test summary` after each change to catch regressions.")


# ──────────────────────────────────────────────────────────────────────
# scaffold — Generate test boilerplate
# ──────────────────────────────────────────────────────────────────────

SCAFFOLDS = {
    "e2e": '''\
#[test]
fn e2e_{name}() {{
    assert_output(
        r#"
*main()
    log('TODO')
"#,
        "TODO\\n",
    );
}}
''',
    "codegen": '''\
#[test]
fn codegen_{name}() {{
    let src = r#"
*main()
    log('TODO')
"#;
    let ir = compile_to_ir(src).expect("should compile");
    assert!(ir.contains("TODO"), "Expected TODO in IR");
}}
''',
    "semantic": '''\
#[test]
fn semantic_{name}() {{
    let src = r#"
*main()
    x is 42
"#;
    let model = compile_to_model(src);
    assert!(model.is_ok(), "Should pass semantic analysis");
}}
''',
    "semantic-reject": '''\
#[test]
fn semantic_rejects_{name}() {{
    let src = r#"
*main()
    x is undefined_var
"#;
    let result = compile_to_model(src);
    assert!(result.is_err(), "Should reject");
    let err = result.unwrap_err();
    assert!(err.message.contains("TODO"), "Expected specific error: {{err:?}}");
}}
''',
    "parser": '''\
#[test]
fn parse_{name}() {{
    let src = r#"
*main()
    x is 42
"#;
    let ast = parse(src);
    assert!(ast.is_ok(), "Should parse: {{:?}}", ast.err());
}}
''',
}


def cmd_scaffold(args):
    """Generate test boilerplate: scaffold <type> <name>"""
    kind = args.kind
    name = args.name

    if kind == "list" or kind is None:
        print("Available scaffolds:")
        for key in SCAFFOLDS:
            print(f"  {key}")
        return

    if kind not in SCAFFOLDS:
        print(f"Unknown scaffold type: {kind}", file=sys.stderr)
        print(f"Available: {', '.join(SCAFFOLDS.keys())}", file=sys.stderr)
        sys.exit(1)

    if not name:
        print("Usage: coral-helpers.py scaffold <type> <test_name>", file=sys.stderr)
        sys.exit(1)

    print(SCAFFOLDS[kind].format(name=name))


# ──────────────────────────────────────────────────────────────────────
# parse-errors — Parse cargo output into structured error list
# ──────────────────────────────────────────────────────────────────────

def cmd_parse_errors(args):
    """Run cargo build/test and extract structured errors."""
    cmd = args.cargo_cmd or "build"
    if cmd == "build":
        result = subprocess.run(
            ["cargo", "build"], capture_output=True, text=True, cwd=ROOT
        )
        output = result.stderr + result.stdout
    elif cmd == "test":
        result = subprocess.run(
            ["cargo", "test"], capture_output=True, text=True, cwd=ROOT
        )
        output = result.stderr + result.stdout
    else:
        print(f"Unknown cargo command: {cmd}", file=sys.stderr)
        sys.exit(1)

    # Parse compilation errors
    error_re = re.compile(r'^error(?:\[E\d+\])?: (.+)')
    location_re = re.compile(r'^\s+--> (.+?):(\d+):(\d+)')
    # Parse test failures
    test_fail_re = re.compile(r'^test (.+) \.\.\. FAILED$')

    errors = []
    current_error = None

    for line in output.split("\n"):
        m = error_re.match(line)
        if m:
            current_error = {"message": m.group(1), "file": None, "line": None, "col": None}
            continue

        m = location_re.match(line)
        if m and current_error:
            current_error["file"] = m.group(1)
            current_error["line"] = int(m.group(2))
            current_error["col"] = int(m.group(3))
            errors.append(current_error)
            current_error = None
            continue

        m = test_fail_re.match(line)
        if m:
            errors.append({"message": f"Test FAILED: {m.group(1)}", "file": "tests/", "line": None, "col": None})

    if not errors:
        if result.returncode == 0:
            print("No errors found.")
        else:
            # Couldn't parse, show raw output tail
            print("Could not parse errors. Raw output tail:")
            print("\n".join(output.strip().split("\n")[-20:]))
        return

    if args.format == "json":
        print(json.dumps(errors, indent=2))
    else:
        print(f"Found {len(errors)} error(s):\n")
        for e in errors:
            loc = ""
            if e["file"]:
                loc = f"{e['file']}:{e['line'] or '?'}"
                if e["col"]:
                    loc += f":{e['col']}"
            print(f"  {loc:40s}  {e['message']}")


# ──────────────────────────────────────────────────────────────────────
# Main dispatch
# ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Advanced code navigation helpers for LLM agents (powered by codemap.py).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""\
Commands:
  goto <file> <symbol>         Find symbol location with line range
  extract <file> <symbol>      Extract function/symbol body to stdout
  context <file>               Show structural index for a file
  symbols <file>               List all symbols in a file
  checklist [task-type]        Edit checklist for common task types
  scaffold <type> <name>       Generate test boilerplate
  parse-errors [build|test]    Parse cargo errors into structured list

Examples:
  %(prog)s goto src/parser.rs parse_match_arm
  %(prog)s extract src/codegen/mod.rs compile_call --context 5
  %(prog)s context src/semantic.rs --brief
  %(prog)s symbols src/ast.rs --kind enum
  %(prog)s checklist new-syntax --enrich
  %(prog)s scaffold e2e my_feature
  %(prog)s parse-errors test --format json
""",
    )
    sub = parser.add_subparsers(dest="command")

    # goto
    p = sub.add_parser("goto", help="Find symbol location with line range")
    p.add_argument("file", help="File path (relative to project root)")
    p.add_argument("symbol", help="Symbol name to find")

    # extract
    p = sub.add_parser("extract", help="Extract a function/symbol body")
    p.add_argument("file", help="File path")
    p.add_argument("symbol", help="Symbol name")
    p.add_argument("--context", "-c", type=int, default=3, help="Lines of context (default: 3)")
    p.add_argument("--which", "-w", type=int, default=None, help="Which match (0-indexed)")

    # context
    p = sub.add_parser("context", help="Show structural index for a file")
    p.add_argument("file", help="File path")
    p.add_argument("--brief", "-b", action="store_true", help="Compact output")

    # symbols
    p = sub.add_parser("symbols", help="List all symbols in a file")
    p.add_argument("file", help="File path")
    p.add_argument("--kind", "-k", help="Filter by kind (fn, method, struct, enum, trait, impl)")

    # checklist
    p = sub.add_parser("checklist", help="Edit checklist for a task type")
    p.add_argument("task", nargs="?", default=None, help="Task type (or 'list')")
    p.add_argument("--enrich", "-e", action="store_true", help="Show structural info for each file")

    # scaffold
    p = sub.add_parser("scaffold", help="Generate test boilerplate")
    p.add_argument("kind", nargs="?", default=None, help="Test type (or 'list')")
    p.add_argument("name", nargs="?", default=None, help="Test name")

    # parse-errors
    p = sub.add_parser("parse-errors", help="Parse cargo errors into structured list")
    p.add_argument("cargo_cmd", nargs="?", default="build", help="build or test")
    p.add_argument("--format", choices=["text", "json"], default="text")

    args = parser.parse_args()

    commands = {
        "goto": cmd_goto,
        "extract": cmd_extract,
        "context": cmd_context,
        "symbols": cmd_symbols,
        "checklist": cmd_checklist,
        "scaffold": cmd_scaffold,
        "parse-errors": cmd_parse_errors,
    }

    if args.command in commands:
        commands[args.command](args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
