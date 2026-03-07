#!/usr/bin/env python3
"""Binary search for first parse error line in expanded Coral source."""
import subprocess, sys, os

def try_compile(source_text):
    """Try to compile the given source text using the Coral compiler test."""
    # Write to temp file
    tmp_path = "target/tmp/test_partial.coral"
    with open(tmp_path, 'w') as f:
        f.write(source_text)
    
    # Try to compile using Rust
    result = subprocess.run(
        ["cargo", "test", "--test", "self_hosting", "dump_expanded_sources", "--", "--nocapture"],
        capture_output=True, text=True, timeout=60
    )
    # We can't easily test partial source this way...
    # Instead, let's use the compiler directly
    return True

def try_compile_direct(source_text):
    """Try to parse the source using the Coral compiler."""
    tmp_path = "target/tmp/test_partial.coral"
    with open(tmp_path, 'w') as f:
        f.write(source_text)
    
    # Use rustc inline test - actually let me just check the tokenization
    result = subprocess.run(
        ["cargo", "run", "--", "--check", tmp_path],
        capture_output=True, text=True, timeout=30
    )
    return result.returncode == 0, result.stderr + result.stdout

# For compiler.coral, use the expanded source
fname = "target/tmp/self_hosted_compiler_expanded.txt"
with open(fname) as f:
    text = f.read()

lines = text.split('\n')
total = len(lines)

# Binary search: find the first N such that lines[0:N] causes an error
lo, hi = 1, total

# First, verify the full source fails
print(f"Total lines: {total}")
print(f"Testing compilation of partial sources...")

# Try progressively larger chunks
for n in [50, 100, 150, 200, 210, 220, 225, 230, 232, 233, 234, 235, 236]:
    partial = '\n'.join(lines[:n])
    tmp_path = "target/tmp/test_partial.coral"
    with open(tmp_path, 'w') as f:
        f.write(partial)
    print(f"  Lines 1-{n}: {len(partial)} bytes")
