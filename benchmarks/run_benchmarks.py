#!/usr/bin/env python3
"""Coral Benchmark Runner

Compiles and runs each benchmark, extracting elapsed_ms from output.
Usage: python3 benchmarks/run_benchmarks.py [--release]
"""

import subprocess
import sys
import os
import json
from pathlib import Path

BENCHMARKS_DIR = Path(__file__).parent
PROJECT_ROOT = BENCHMARKS_DIR.parent

BENCHMARKS = [
    ("fibonacci",   "fibonacci.coral"),
    ("tight_loop",  "tight_loop.coral"),
    ("list_ops",    "list_ops.coral"),
    ("string_ops",  "string_ops.coral"),
    ("matrix_mul",  "matrix_mul.coral"),
]

def compile_benchmark(coral_file: Path, release: bool = False) -> Path:
    """Compile a .coral file and return the path to the executable."""
    out = coral_file.with_suffix("")
    cmd = ["cargo", "run"]
    if release:
        cmd.append("--release")
    cmd += ["--", str(coral_file), "--emit-binary", str(out)]
    result = subprocess.run(cmd, cwd=PROJECT_ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"  COMPILE FAIL: {result.stderr.strip()}")
        return None
    return out

def run_benchmark(exe: Path) -> dict:
    """Run the compiled benchmark and extract results."""
    result = subprocess.run([str(exe)], capture_output=True, text=True, timeout=120)
    output = result.stdout.strip()
    lines = output.split("\n")
    
    elapsed_ms = None
    info = {}
    for line in lines:
        if line.startswith("elapsed_ms: "):
            elapsed_ms = float(line.split(": ", 1)[1])
        else:
            info[line.split(" = ")[0].strip() if " = " in line else line] = line.split(" = ")[1].strip() if " = " in line else ""
    
    return {"elapsed_ms": elapsed_ms, "output": info, "raw": output}

def main():
    release = "--release" in sys.argv
    mode = "release" if release else "debug"
    
    print(f"=== Coral Benchmark Suite ({mode} mode) ===\n")
    
    results = {}
    for name, filename in BENCHMARKS:
        coral_file = BENCHMARKS_DIR / filename
        if not coral_file.exists():
            print(f"  [{name}] SKIP: {filename} not found")
            continue
        
        print(f"  [{name}] Compiling...", end=" ", flush=True)
        exe = compile_benchmark(coral_file, release)
        if exe is None:
            continue
        
        print("Running...", end=" ", flush=True)
        try:
            result = run_benchmark(exe)
            elapsed = result["elapsed_ms"]
            if elapsed is not None:
                print(f"{elapsed:.1f} ms")
            else:
                print("(no timing)")
                print(f"    Output: {result['raw']}")
            results[name] = result
        except subprocess.TimeoutExpired:
            print("TIMEOUT (>120s)")
        except Exception as e:
            print(f"ERROR: {e}")
        finally:
            # Clean up executable
            if exe and exe.exists():
                exe.unlink()
    
    print(f"\n=== Summary ===")
    print(f"{'Benchmark':<15} {'Time (ms)':>12}")
    print("-" * 30)
    for name, result in results.items():
        elapsed = result.get("elapsed_ms")
        if elapsed is not None:
            print(f"{name:<15} {elapsed:>12.1f}")
        else:
            print(f"{name:<15} {'N/A':>12}")
    
    # Save results to JSON
    out_file = BENCHMARKS_DIR / f"results_{mode}.json"
    with open(out_file, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nResults saved to {out_file}")

if __name__ == "__main__":
    main()
