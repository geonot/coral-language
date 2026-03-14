#!/usr/bin/env python3
"""Coral Benchmark Runner

Compiles and runs each benchmark, extracting elapsed_ms from output.
Usage: python3 benchmarks/run_benchmarks.py [--release] [--runs N] [--jit]
"""

import subprocess
import sys
import os
import json
import statistics
from pathlib import Path

BENCHMARKS_DIR = Path(__file__).parent
PROJECT_ROOT = BENCHMARKS_DIR.parent

BENCHMARKS = [
    ("fibonacci",         "fibonacci.coral",         "Recursive calls, arithmetic"),
    ("tight_loop",        "tight_loop.coral",        "Numeric loop, NaN-boxing"),
    ("list_ops",          "list_ops.coral",           "List alloc, map/filter/reduce"),
    ("string_ops",        "string_ops.coral",         "String concat, split, replace"),
    ("matrix_mul",        "matrix_mul.coral",         "Nested loops, array indexing"),
    ("map_ops",           "map_ops.coral",            "Map insert, lookup, keys"),
    ("closures",          "closures.coral",           "Lambda calls, HOF pipeline"),
    ("pattern_matching",  "pattern_matching.coral",   "If/elif, match, ternary"),
    ("for_iteration",     "for_iteration.coral",      "For-in loops, nested iteration"),
    ("recursion",         "recursion.coral",          "Ackermann, tree sum, GCD"),
    ("store_ops",         "store_ops.coral",          "Store creation, methods, fields"),
    ("math_compute",      "math_compute.coral",       "Float, trig, sqrt"),
]

def compile_benchmark(coral_file: Path, release: bool = False) -> Path:
    """Compile a .coral file to a native binary."""
    out = coral_file.with_suffix("")
    cmd = ["cargo", "run"]
    if release:
        cmd.append("--release")
    cmd += ["-q", "--", str(coral_file), "--emit-binary", str(out)]
    result = subprocess.run(cmd, cwd=PROJECT_ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        stderr = result.stderr.strip()
        # Filter out Rust compiler warnings in stderr
        real_errors = [l for l in stderr.split('\n') if l.startswith("Error:")]
        if real_errors:
            print(f"COMPILE FAIL: {real_errors[0]}")
            return None
    if not out.exists():
        print("COMPILE FAIL: no binary produced")
        return None
    return out

def run_benchmark_jit(coral_file: Path, release: bool = False) -> dict:
    """Run a benchmark via JIT (no binary compilation)."""
    cmd = ["cargo", "run"]
    if release:
        cmd.append("--release")
    cmd += ["-q", "--", "--jit", str(coral_file)]
    result = subprocess.run(cmd, cwd=PROJECT_ROOT, capture_output=True, text=True, timeout=120)
    return parse_output(result.stdout)

def run_benchmark(exe: Path) -> dict:
    """Run the compiled benchmark and extract results."""
    result = subprocess.run([str(exe)], capture_output=True, text=True, timeout=120)
    return parse_output(result.stdout)

def parse_output(stdout: str) -> dict:
    """Parse benchmark output extracting elapsed_ms and other values."""
    output = stdout.strip()
    lines = output.split("\n")
    elapsed_ms = None
    info = {}
    for line in lines:
        if line.startswith("elapsed_ms: "):
            try:
                elapsed_ms = float(line.split(": ", 1)[1])
            except ValueError:
                pass
        elif " = " in line:
            key, _, val = line.partition(" = ")
            info[key.strip()] = val.strip()
    return {"elapsed_ms": elapsed_ms, "output": info, "raw": output}

def main():
    release = "--release" in sys.argv
    jit_mode = "--jit" in sys.argv
    runs = 1
    for i, arg in enumerate(sys.argv):
        if arg == "--runs" and i + 1 < len(sys.argv):
            runs = int(sys.argv[i + 1])

    mode = "release" if release else "debug"
    exec_mode = "JIT" if jit_mode else "binary"

    print(f"=== Coral Benchmark Suite ({mode} build, {exec_mode} execution, {runs} run{'s' if runs > 1 else ''}) ===\n")

    results = {}
    for name, filename, description in BENCHMARKS:
        coral_file = BENCHMARKS_DIR / filename
        if not coral_file.exists():
            print(f"  [{name}] SKIP: {filename} not found")
            continue

        timings = []
        exe = None

        if not jit_mode:
            print(f"  [{name:18s}] Compiling...", end=" ", flush=True)
            exe = compile_benchmark(coral_file, release)
            if exe is None:
                continue

        print(f"Running ({runs}x)...", end=" ", flush=True)
        try:
            for r in range(runs):
                if jit_mode:
                    result = run_benchmark_jit(coral_file, release)
                else:
                    result = run_benchmark(exe)
                if result["elapsed_ms"] is not None:
                    timings.append(result["elapsed_ms"])

            if timings:
                median = statistics.median(timings)
                if runs > 1:
                    mn = min(timings)
                    mx = max(timings)
                    print(f"median {median:.1f} ms  (min {mn:.1f}, max {mx:.1f})")
                else:
                    print(f"{median:.1f} ms")
                results[name] = {
                    "elapsed_ms": median,
                    "min_ms": min(timings),
                    "max_ms": max(timings),
                    "runs": timings,
                    "description": description,
                    "output": result.get("output", {}),
                }
            else:
                print("(no timing)")
                results[name] = {"elapsed_ms": None, "description": description}
        except subprocess.TimeoutExpired:
            print("TIMEOUT (>120s)")
        except Exception as e:
            print(f"ERROR: {e}")
        finally:
            if exe and exe.exists():
                exe.unlink()

    # Summary table
    print(f"\n{'='*72}")
    print(f"{'Benchmark':<20} {'Description':<32} {'Time (ms)':>10}")
    print(f"{'-'*72}")
    total = 0.0
    for name, _, description in BENCHMARKS:
        if name in results:
            elapsed = results[name].get("elapsed_ms")
            if elapsed is not None:
                total += elapsed
                print(f"{name:<20} {description:<32} {elapsed:>10.1f}")
            else:
                print(f"{name:<20} {description:<32} {'N/A':>10}")
    print(f"{'-'*72}")
    print(f"{'TOTAL':<20} {'':<32} {total:>10.1f}")
    print(f"{'='*72}")

    # Save results to JSON
    out_file = BENCHMARKS_DIR / f"results_{mode}_{exec_mode}.json"
    with open(out_file, "w") as f:
        json.dump({"mode": mode, "execution": exec_mode, "runs": runs, "benchmarks": results}, f, indent=2)
    print(f"\nResults saved to {out_file}")

if __name__ == "__main__":
    main()
