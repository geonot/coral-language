#!/usr/bin/env python3
"""Remove non-doc comments from Rust source files."""

import re
import glob

files = []
for pattern in ['src/**/*.rs', 'runtime/src/**/*.rs']:
    files.extend(glob.glob(pattern, recursive=True))

total_removed = 0
for filepath in sorted(set(files)):
    with open(filepath, 'r') as f:
        lines = f.readlines()

    new_lines = []
    removed = 0
    for line in lines:
        stripped = line.strip()
        # Skip whole-line comments (but keep doc comments /// and //!)
        if re.match(r'^\s*//(?!/|!)', line):
            removed += 1
            continue
        # Remove trailing comments from code lines
        if '//' in line and not stripped.startswith('//'):
            in_string = False
            string_char = None
            i = 0
            comment_start = -1
            while i < len(line):
                c = line[i]
                if in_string:
                    if c == '\\' and i + 1 < len(line):
                        i += 2
                        continue
                    if c == string_char:
                        in_string = False
                elif c in ('"', "'"):
                    in_string = True
                    string_char = c
                elif c == '/' and i + 1 < len(line) and line[i + 1] == '/':
                    if i + 2 < len(line) and line[i + 2] in ('/', '!'):
                        break
                    comment_start = i
                    break
                i += 1

            if comment_start >= 0:
                new_line = line[:comment_start].rstrip() + '\n'
                if new_line.strip():
                    new_lines.append(new_line)
                    removed += 1
                    continue
                else:
                    removed += 1
                    continue

        new_lines.append(line)

    if removed > 0:
        with open(filepath, 'w') as f:
            f.writelines(new_lines)
        print(f'{filepath}: removed {removed} comment lines')
        total_removed += removed

print(f'Total: removed {total_removed} comment lines from {len(files)} files')
