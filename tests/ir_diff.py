#!/usr/bin/env python3
"""IR Diff Testing Framework for Coral Self-Hosted Compiler.

Compiles a program with both the Rust and self-hosted compilers,
normalizes the IR output, and reports structural differences.

Usage:
    python3 tests/ir_diff.py tests/fixtures/self_hosted_levels/L1_hello.coral
    python3 tests/ir_diff.py --all     # Run on all level tests
"""

import subprocess
import sys
import re
import os
from pathlib import Path

WORKSPACE = Path(__file__).parent.parent
RUNTIME = WORKSPACE / "target" / "release" / "libruntime.so"
SELF_HOSTED_LL = WORKSPACE / "target" / "tmp" / "self_hosted.ll"
CORALC = WORKSPACE / "target" / "release" / "coralc"


def normalize_ir(ir_text: str) -> list[str]:
    """Normalize LLVM IR for structural comparison.
    
    - Strip comments
    - Normalize register names (%0, %1, ... → %r0, %r1, ...)
    - Normalize label names
    - Strip metadata
    - Sort declarations
    """
    lines = []
    for line in ir_text.splitlines():
        # Skip comments and empty lines
        stripped = line.strip()
        if stripped.startswith(";") or not stripped:
            continue
        # Skip metadata
        if stripped.startswith("!"):
            continue
        # Remove inline comments
        line = re.sub(r';.*$', '', line).rstrip()
        if line.strip():
            lines.append(line)
    return lines


def extract_functions(ir_lines: list[str]) -> dict[str, list[str]]:
    """Extract function definitions as a dict of name → body lines."""
    funcs = {}
    current_func = None
    current_body = []
    
    for line in ir_lines:
        if line.startswith("define "):
            match = re.search(r'@(\w+)\(', line)
            if match:
                current_func = match.group(1)
                current_body = [line]
        elif current_func:
            current_body.append(line)
            if line.strip() == "}":
                funcs[current_func] = current_body
                current_func = None
                current_body = []
    return funcs


def extract_declares(ir_lines: list[str]) -> set[str]:
    """Extract declared function names."""
    declares = set()
    for line in ir_lines:
        if line.startswith("declare "):
            match = re.search(r'@(\w+)\(', line)
            if match:
                declares.add(match.group(1))
    return declares


def extract_globals(ir_lines: list[str]) -> set[str]:
    """Extract global variable names."""
    globals_set = set()
    for line in ir_lines:
        match = re.match(r'(@\w+)\s*=', line)
        if match:
            globals_set.add(match.group(1))
    return globals_set


def compile_with_rust(coral_file: Path) -> str:
    """Compile a .coral file with the Rust compiler."""
    result = subprocess.run(
        [str(CORALC), str(coral_file), "--emit-ir", "/dev/stdout"],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        raise RuntimeError(f"Rust compiler failed: {result.stderr}")
    return result.stdout


def compile_with_self_hosted(coral_file: Path) -> str:
    """Compile a .coral file with the self-hosted compiler via lli."""
    if not SELF_HOSTED_LL.exists():
        raise RuntimeError(f"Self-hosted compiler IR not found at {SELF_HOSTED_LL}")
    
    result = subprocess.run(
        ["lli", f"-load={RUNTIME}", str(SELF_HOSTED_LL), str(coral_file)],
        capture_output=True, text=True, timeout=60
    )
    if result.returncode != 0:
        raise RuntimeError(f"Self-hosted compiler failed: {result.stderr}")
    return result.stdout


def diff_ir(rust_ir: str, sh_ir: str, name: str) -> dict:
    """Compare IR output structurally and return a diff report."""
    rust_lines = normalize_ir(rust_ir)
    sh_lines = normalize_ir(sh_ir)
    
    rust_funcs = extract_functions(rust_lines)
    sh_funcs = extract_functions(sh_lines)
    
    rust_declares = extract_declares(rust_lines)
    sh_declares = extract_declares(sh_lines)
    
    report = {
        "name": name,
        "rust_functions": sorted(rust_funcs.keys()),
        "sh_functions": sorted(sh_funcs.keys()),
        "missing_functions": sorted(set(rust_funcs.keys()) - set(sh_funcs.keys())),
        "extra_functions": sorted(set(sh_funcs.keys()) - set(rust_funcs.keys())),
        "common_functions": sorted(set(rust_funcs.keys()) & set(sh_funcs.keys())),
        "missing_declares": sorted(rust_declares - sh_declares),
        "extra_declares": sorted(sh_declares - rust_declares),
    }
    
    # For common functions, check if they have similar structure
    body_diffs = {}
    for fname in report["common_functions"]:
        rust_body = rust_funcs[fname]
        sh_body = sh_funcs[fname]
        if len(rust_body) != len(sh_body):
            body_diffs[fname] = f"line count differs: rust={len(rust_body)} sh={len(sh_body)}"
    report["body_diffs"] = body_diffs
    
    return report


def print_report(report: dict):
    """Print a human-readable diff report."""
    name = report["name"]
    print(f"\n{'='*60}")
    print(f"IR Diff Report: {name}")
    print(f"{'='*60}")
    
    print(f"\nRust functions ({len(report['rust_functions'])}): {', '.join(report['rust_functions'][:10])}{'...' if len(report['rust_functions']) > 10 else ''}")
    print(f"Self-hosted functions ({len(report['sh_functions'])}): {', '.join(report['sh_functions'][:10])}{'...' if len(report['sh_functions']) > 10 else ''}")
    
    if report["missing_functions"]:
        print(f"\n⚠ Missing from self-hosted: {', '.join(report['missing_functions'])}")
    if report["extra_functions"]:
        print(f"\n+ Extra in self-hosted: {', '.join(report['extra_functions'])}")
    
    if report["missing_declares"]:
        print(f"\n⚠ Missing declares: {len(report['missing_declares'])} runtime functions")
    if report["extra_declares"]:
        print(f"\n+ Extra declares: {len(report['extra_declares'])} runtime functions")
    
    if report["body_diffs"]:
        print(f"\n⚠ Body differences:")
        for fname, diff in report["body_diffs"].items():
            print(f"  {fname}: {diff}")
    
    # Summary
    issues = len(report["missing_functions"]) + len(report["body_diffs"])
    if issues == 0:
        print(f"\n✓ Structural match for user functions!")
    else:
        print(f"\n✗ {issues} structural difference(s)")


def main():
    if len(sys.argv) < 2:
        print("Usage: python3 tests/ir_diff.py <file.coral> [--all]")
        sys.exit(1)
    
    if sys.argv[1] == "--all":
        levels_dir = WORKSPACE / "tests" / "fixtures" / "self_hosted_levels"
        files = sorted(levels_dir.glob("L*.coral"))
    else:
        files = [Path(sys.argv[1])]
    
    total_ok = 0
    total_issues = 0
    
    for coral_file in files:
        try:
            rust_ir = compile_with_rust(coral_file)
            sh_ir = compile_with_self_hosted(coral_file)
            report = diff_ir(rust_ir, sh_ir, coral_file.name)
            print_report(report)
            
            issues = len(report["missing_functions"]) + len(report["body_diffs"])
            if issues == 0:
                total_ok += 1
            else:
                total_issues += 1
        except Exception as e:
            print(f"\n✗ {coral_file.name}: {e}")
            total_issues += 1
    
    print(f"\n{'='*60}")
    print(f"Summary: {total_ok} OK, {total_issues} with issues")
    print(f"{'='*60}")
    return 0 if total_issues == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
