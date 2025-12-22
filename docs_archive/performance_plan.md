# Performance / Near-Native Track – Starter Notes

- **Typed MIR:** introduce parallel typed MIR to allow unboxed ints/floats/bytes/strings; monomorphize hot paths and fall back to tagged `Value` for dynamic code.
- **Stack arenas:** extend existing runtime stack arenas to host typed MIR temporaries; copy-on-escape into RC heap.
- **Emit-bin:** wire `--emit-bin` path via `llc`/`clang` once typed MIR covers core ops; add CI smoke.
- **PGO hooks:** consume runtime metrics JSON to size literals and arena pages; seed heuristic for list/map initial capacity.
- **Benchmarks:** add Criterion suites for string concat, list push/pop, map get/set, match dispatch; track regression budgets.
