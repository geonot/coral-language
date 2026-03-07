#!/usr/bin/env python3
"""Simulate the Rust module loader to find error locations in expanded source."""
import os, sys

STD_DIR = "std"

def load_std(name):
    path = os.path.join(STD_DIR, f"{name}.coral")
    with open(path) as f:
        return f.read()

def load_recursive(filename, included=None):
    """Simulate the Rust module loader's load_recursive."""
    if included is None:
        included = set()
    
    canonical = os.path.abspath(filename)
    if canonical in included:
        return ""
    
    with open(filename) as f:
        source = f.read()
    
    expanded = ""
    for line in source.split('\n'):
        stripped = line.strip()
        if stripped.startswith("use "):
            module_name = stripped[4:].strip()
            # Resolve: std.X -> std/X.coral
            module_path = module_name.replace(".", "/") + ".coral"
            if os.path.exists(module_path):
                mod_canonical = os.path.abspath(module_path)
                if mod_canonical not in included:
                    module_source = load_recursive(module_path, included)
                    if module_source:
                        expanded += module_source
                        expanded += "\n"
            continue
        expanded += line
        expanded += "\n"
    
    included.add(canonical)
    return expanded

def expand_source(filename):
    """Full load like the Rust module loader: prelude + file."""
    included = set()
    
    # Load prelude first
    prelude_path = os.path.join(STD_DIR, "prelude.coral")
    prelude_source = load_recursive(prelude_path, included)
    
    result = prelude_source
    result += "\n"
    
    # Load the actual file
    user_source = load_recursive(filename, included)
    result += user_source
    
    return result

files_offsets = [
    ('self_hosted/lower.coral', 5439, 5444),
    ('self_hosted/module_loader.coral', 8625, 8636),
    ('self_hosted/semantic.coral', 8798, 8800),
    ('self_hosted/codegen.coral', 7705, 7709),
    ('self_hosted/compiler.coral', 5043, 5047),
]

for fname, start, end in files_offsets:
    expanded = expand_source(fname)
    print(f"=== {fname} ===")
    print(f"  Expanded size: {len(expanded)} bytes")
    
    if start >= len(expanded):
        print(f"  ERROR: offset {start} beyond expanded size {len(expanded)}")
        # Try to dump last 20 lines
        all_lines = expanded.split('\n')
        for i in range(max(0, len(all_lines)-10), len(all_lines)):
            print(f"    {i+1:4d}: {repr(all_lines[i])}")
        print()
        continue
    
    token = expanded[start:end]
    line_num = expanded[:start].count('\n') + 1
    
    line_start = expanded.rfind('\n', 0, start) + 1
    line_end = expanded.find('\n', start)
    if line_end == -1: line_end = len(expanded)
    error_line = expanded[line_start:line_end]
    
    all_lines = expanded.split('\n')
    ctx_start = max(0, line_num - 8)
    ctx_end = min(len(all_lines), line_num + 5)
    
    print(f"  Error at byte {start}-{end}, line {line_num}")
    print(f"  Token at span: {repr(token)}")
    print(f"  Error line: {repr(error_line)}")
    print(f"  Context:")
    for i in range(ctx_start, ctx_end):
        marker = " >>>" if i == line_num - 1 else "    "
        print(f"  {marker} {i+1:4d}: {repr(all_lines[i])}")
    print()
