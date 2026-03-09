#!/usr/bin/env python3
"""
xref.py — Build cross-reference / caller-callee graph for a codebase.

Complements codemap.py by focusing specifically on symbol relationships:
  - Which functions call which other functions
  - Reverse index: for each function, who calls it
  - Unused functions (no callers)

Usage:
    python tools/xref.py [directory] [options]
    python tools/xref.py src/ --output xref.md
    python tools/xref.py . --include "*.rs" --format json
"""

import argparse
import json
import os
import re
import sys
from collections import defaultdict
from pathlib import Path

# Reuse codemap's discovery and parsing
sys.path.insert(0, os.path.dirname(__file__))
from codemap import discover_files, detect_language, PARSERS


def build_xref(root: str, include_patterns=None, exclude_patterns=None, max_files=500):
    """Build a cross-reference map of function definitions and calls."""
    files = discover_files(root, include_patterns, exclude_patterns)
    if len(files) > max_files:
        files = files[:max_files]

    # Phase 1: Collect all definitions
    definitions = {}  # name -> [(file, line, kind)]
    all_calls = {}    # (file, name) -> [called_names]
    
    for rel_path in files:
        filepath = os.path.join(root, rel_path)
        lang = detect_language(rel_path)
        parser_obj = PARSERS.get(lang)
        if not parser_obj:
            continue

        try:
            with open(filepath, "r", encoding="utf-8", errors="replace") as f:
                content = f.read()
            fm = parser_obj.parse(rel_path, content)
        except Exception:
            continue

        def register_symbol(sym, file_path):
            key = sym.name
            if key not in definitions:
                definitions[key] = []
            definitions[key].append({
                "file": file_path,
                "line": sym.line,
                "kind": sym.kind,
            })
            if sym.calls:
                all_calls[(file_path, sym.name)] = sym.calls
            for child in sym.children:
                register_symbol(child, file_path)

        for sym in fm.symbols:
            register_symbol(sym, rel_path)

    # Phase 2: Build caller->callee and callee->caller graphs
    callers = defaultdict(list)   # callee_name -> [(caller_file, caller_name)]
    callees = defaultdict(list)   # (caller_file, caller_name) -> [callee_name]

    for (file, caller_name), called_names in all_calls.items():
        for callee_name in called_names:
            if callee_name in definitions:
                callers[callee_name].append({"file": file, "name": caller_name})
                callees[(file, caller_name)].append(callee_name)

    # Phase 3: Identify unreferenced definitions
    unreferenced = []
    # Skip common entry points and known patterns
    skip_names = {"main", "new", "default", "from", "into", "drop", "fmt", "clone",
                  "eq", "hash", "partial_cmp", "cmp", "display", "debug",
                  "__init__", "__str__", "__repr__", "__eq__", "__hash__",
                  "setup", "teardown", "test", "run"}

    for name, defs in definitions.items():
        if name.lower() in skip_names:
            continue
        if name.startswith("_") or name.startswith("test_"):
            continue
        if name not in callers:
            for d in defs:
                if d["kind"] in ("fn", "method"):
                    unreferenced.append({"name": name, **d})

    return {
        "definitions": definitions,
        "callers": dict(callers),
        "callees": {f"{k[0]}::{k[1]}": v for k, v in callees.items()},
        "unreferenced": unreferenced,
        "file_count": len(files),
    }


def format_markdown(xref_data: dict) -> str:
    """Format cross-reference data as Markdown."""
    lines = []
    lines.append("# Cross-Reference Report")
    lines.append("")
    lines.append(f"| Metric | Value |")
    lines.append(f"|--------|-------|")
    lines.append(f"| Files analyzed | {xref_data['file_count']} |")
    lines.append(f"| Unique symbols | {len(xref_data['definitions'])} |")
    lines.append(f"| Symbols with callers | {len(xref_data['callers'])} |")
    lines.append(f"| Potentially unused | {len(xref_data['unreferenced'])} |")
    lines.append("")

    # Most-called functions (hot paths)
    lines.append("## Most Referenced Symbols")
    lines.append("")
    sorted_callers = sorted(xref_data["callers"].items(), key=lambda x: len(x[1]), reverse=True)
    for name, caller_list in sorted_callers[:30]:
        defs = xref_data["definitions"].get(name, [])
        def_loc = ""
        if defs:
            d = defs[0]
            def_loc = f" — defined at [{d['file']}:{d['line']}]({d['file']}#L{d['line']})"
        caller_names = sorted(set(c["name"] for c in caller_list))
        lines.append(f"- **`{name}`** ({len(caller_list)} refs){def_loc}")
        lines.append(f"  Called by: {', '.join(f'`{c}`' for c in caller_names[:10])}")
        if len(caller_names) > 10:
            lines.append(f"  +{len(caller_names) - 10} more callers")
    lines.append("")

    # Unreferenced functions
    if xref_data["unreferenced"]:
        lines.append("## Potentially Unused Functions")
        lines.append("")
        lines.append("_Functions with no detected callers (may be entry points, trait impls, or test helpers)_")
        lines.append("")
        # Group by file
        by_file = defaultdict(list)
        for item in xref_data["unreferenced"]:
            by_file[item["file"]].append(item)
        for file, items in sorted(by_file.items()):
            lines.append(f"**{file}**")
            for item in sorted(items, key=lambda x: x["line"]):
                lines.append(f"- `{item['name']}` ({item['kind']}) L{item['line']}")
            lines.append("")

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="Build cross-reference report for a codebase.")
    parser.add_argument("directory", nargs="?", default=".", help="Root directory")
    parser.add_argument("--output", "-o", help="Output file")
    parser.add_argument("--include", nargs="*", help="Include glob patterns")
    parser.add_argument("--exclude", nargs="*", help="Exclude glob patterns")
    parser.add_argument("--format", choices=["markdown", "json"], default="markdown", help="Output format")
    parser.add_argument("--max-files", type=int, default=500, help="Max files")

    args = parser.parse_args()

    if not os.path.isdir(args.directory):
        print(f"Error: '{args.directory}' is not a directory", file=sys.stderr)
        sys.exit(1)

    xref_data = build_xref(args.directory, args.include, args.exclude, args.max_files)

    if args.format == "json":
        output = json.dumps(xref_data, indent=2)
    else:
        output = format_markdown(xref_data)

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(output)
        print(f"Cross-reference written to {args.output}", file=sys.stderr)
    else:
        print(output)


if __name__ == "__main__":
    main()
