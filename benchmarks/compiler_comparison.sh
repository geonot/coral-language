#!/usr/bin/env bash
# CC1.4 — Compiler Performance Comparison
# Compares Rust compiler vs Self-Hosted Coral compiler
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
RESULTS_DIR="$PROJECT_ROOT/benchmarks/results"
mkdir -p "$RESULTS_DIR"

BENCHMARK_FILES=(
    "examples/fizzbuzz.coral"
    "examples/hello.coral"
    "examples/calculator.coral"
    "benchmarks/fibonacci.coral"
)

echo "=== Coral Compiler Performance Comparison ==="
echo "Date: $(date -Iseconds)"
echo ""

echo "--- Building Rust compiler (release) ---"
time cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml" 2>/dev/null
RUST_COMPILER="$PROJECT_ROOT/target/release/coral"

if [ ! -f "$RUST_COMPILER" ]; then
    echo "Warning: Release binary not at expected path, trying debug"
    RUST_COMPILER="$PROJECT_ROOT/target/debug/coral"
fi

echo ""
echo "=== Compilation Speed ==="
echo ""
printf "%-30s %15s %15s\n" "File" "Rust (ms)" "IR Size (bytes)"
printf "%-30s %15s %15s\n" "----" "---------" "---------------"

for file in "${BENCHMARK_FILES[@]}"; do
    filepath="$PROJECT_ROOT/$file"
    if [ ! -f "$filepath" ]; then
        continue
    fi
    
    outfile="$RESULTS_DIR/$(basename "$file" .coral).ll"
    
    start_ns=$(date +%s%N)
    "$RUST_COMPILER" --emit-ir "$outfile" "$filepath" 2>/dev/null || true
    end_ns=$(date +%s%N)
    
    elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
    
    if [ -f "$outfile" ]; then
        ir_size=$(wc -c < "$outfile")
    else
        ir_size="N/A"
    fi
    
    printf "%-30s %15s %15s\n" "$file" "${elapsed_ms}" "$ir_size"
done

echo ""
echo "=== Memory Usage (peak RSS) ==="
echo ""

for file in "${BENCHMARK_FILES[@]}"; do
    filepath="$PROJECT_ROOT/$file"
    if [ ! -f "$filepath" ]; then
        continue
    fi
    
    outfile="$RESULTS_DIR/$(basename "$file" .coral)_mem.ll"
    
    if command -v /usr/bin/time &>/dev/null; then
        mem=$(/usr/bin/time -v "$RUST_COMPILER" --emit-ir "$outfile" "$filepath" 2>&1 | grep "Maximum resident" | awk '{print $NF}')
        printf "%-30s %10s KB\n" "$file" "$mem"
    else
        echo "(/usr/bin/time not available — skipping memory measurement)"
        break
    fi
done

echo ""
echo "=== Summary ==="
echo "Results saved to: $RESULTS_DIR"
echo "Done."
