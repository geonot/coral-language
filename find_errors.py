#!/usr/bin/env python3
# Simulate the module loader's expansion to find the actual error locations
import os

def load_std(name):
    path = f"std/{name}.coral"
    with open(path) as f:
        return f.read()

def expand_source(filename):
    with open(filename) as f:
        lines = f.readlines()
    
    uses = []
    source_lines = []
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("use std."):
            mod_name = stripped[len("use std."):]
            uses.append(mod_name)
        else:
            source_lines.append(line)
    
    expanded = ""
    for mod_name in uses:
        content = load_std(mod_name)
        expanded += content
        if not content.endswith("\n"):
            expanded += "\n"
    expanded += "".join(source_lines)
    return expanded

files_offsets = [
    ('self_hosted/lower.coral', 5439, 5444),
    ('self_hosted/module_loader.coral', 8625, 8636),
    ('self_hosted/semantic.coral', 8798, 8800),
    ('self_hosted/codegen.coral', 7705, 7709),
    ('self_hosted/compiler.coral', 5043, 5047),
]

for fname, start, end in files_offsets:
    expanded = expand_source(fname)
    if start >= len(expanded):
        print(f"=== {fname}: offset {start} beyond expanded size {len(expanded)} ===")
        continue
    
    token = expanded[start:end]
    line_num = expanded[:start].count('\n') + 1
    
    line_start = expanded.rfind('\n', 0, start) + 1
    line_end = expanded.find('\n', start)
    if line_end == -1: line_end = len(expanded)
    error_line = expanded[line_start:line_end]
    
    all_lines = expanded.split('\n')
    ctx_start = max(0, line_num - 6)
    ctx_end = min(len(all_lines), line_num + 5)
    
    print(f"=== {fname} ===")
    print(f"  Expanded size: {len(expanded)} bytes")
    print(f"  Error at byte {start}-{end}, line {line_num}")
    print(f"  Token at span: {repr(token)}")
    print(f"  Context:")
    for i in range(ctx_start, ctx_end):
        marker = " >>>" if i == line_num - 1 else "    "
        print(f"  {marker} {i+1:4d}: {repr(all_lines[i])}")
    print()
