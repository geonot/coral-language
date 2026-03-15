# Fundamental Design Issues Limiting Native Performance

These are architectural decisions in Coral's design that create a **permanent
performance ceiling** below Rust/Zig/C level. They are documented here for
transparency and to guide long-term language evolution.

### 3.1 Universal NaN-Boxing (Most Impactful)

**What**: Every value in Coral is a 64-bit float-encoded NaN-boxed value. Integers,
booleans, pointers, and floats all share the same `i64` representation.

**Cost**:
- Integer arithmetic requires `bitcast i64→f64`, compute in f64, `bitcast f64→i64`.
  Native integer add is 1 cycle; NaN-boxed "integer" add is ~4 cycles.
- No native integer types means no LLVM integer optimizations (strength reduction,
  constant folding of integer math, loop induction variable widening).
- Array/list indexing pays f64→i64 truncation on every access.
- Branch conditions go through float comparison instead of integer flags.
- SIMD vectorization is largely blocked — LLVM can't vectorize mixed i64/f64 ops.

**What Rust/Zig do differently**: Static types known at compile time. Integer
variables compile to native `i32`/`i64` with zero overhead. Floats compile to
`f32`/`f64`. No boxing penalty.

**Mitigation applied**: `wrap_number_unchecked` eliminates NaN canonicalization
for arithmetic results known to be finite. `try_emit_condition_i1` bypasses
boxing for boolean conditions. These help but don't eliminate the fundamental
`i64↔f64` bitcast cost.

**What would fix it**: Type-specialized codegen. The type solver already infers
types — feeding resolved types into codegen would allow emitting native `i64`
for integer-typed variables and `f64` for floats, with NaN-boxing only at
polymorphic boundaries. This is a **major architecture change** requiring:
- Type annotations flowing from semantic analysis to codegen
- Dual code paths in codegen (boxed vs unboxed)
- Box/unbox coercion at function call boundaries
- Unboxed local variable slots

**Estimated performance gain**: 2x-5x on integer-heavy code (tight loops, indexing,
counting).

---

### 3.2 Reference Counting Without Escape Analysis

**What**: Every heap object (string, list, map, closure, store instance) is
reference-counted. Every variable assignment, function argument pass, and
return increments the RC. Every scope exit decrements it.

**Cost**:
- RC increment/decrement is an atomic operation (for thread safety). Atomic ops
  are 5-20x slower than regular memory writes.
- Short-lived temporaries that never escape the current scope still pay full
  RC cost. E.g., `"hello" + " " + "world"` creates 2 intermediate strings,
  each with RC alloc/increment/decrement/free.
- Cache line contention when multiple threads access the same object.
- No cycle collection by default — leaked cycles are possible.

**What Rust does differently**: Ownership + borrowing eliminates RC for most code.
Values are moved (zero-cost) or borrowed (compile-time checked reference).
`Rc`/`Arc` only used when explicitly requested.

**What Zig does differently**: Manual memory management with arena allocators.
Zero overhead for allocation patterns.

**Mitigation applied**: None in this sprint.

**What would fix it**:
1. **Escape analysis**: Analyze which allocations never escape the current function.
   Stack-allocate those instead of heap-allocating with RC. Covers ~60-80% of
   temporary strings and closures.
2. **Move semantics**: When a variable is used exactly once after creation, move
   it instead of RC-increment + RC-decrement. Zero-cost transfer.
3. **Region-based allocation**: Group short-lived allocations into a bump allocator
   that frees them all at once (arena pattern).

**Estimated performance gain**: 2x-4x on allocation-heavy code (string manipulation,
list transformation, closure-heavy functional patterns).

---

### 3.3 No Monomorphization

**What**: All Coral functions have a single compiled body that operates on
NaN-boxed `i64` values. Generic functions are not specialized per type argument.

**Cost**:
- A function `*max(a, b)` compiled once for all types. Cannot use `maxsd` for
  floats or `cmp` for integers — must go through runtime type dispatch.
- No opportunity for type-specific LLVM optimization. The optimizer sees only
  `i64` operations and cannot deduce that the values are integers.
- Generic containers (List, Map) store boxed values. No `Vec<i32>` equivalent
  with contiguous integer storage.

**What Rust does differently**: Generics are monomorphized — `fn max<T: Ord>(a: T, b: T)`
generates specialized machine code for each concrete type. `max::<i32>` uses
native integer comparison. `max::<f64>` uses float comparison.

**Mitigation applied**: None — this is a fundamental architecture decision.

**What would fix it**: Full monomorphization requires:
- Type parameter tracking through codegen
- Template instantiation (one LLVM function per type combination)
- Exponential code size management (similar to Rust's approach)
- This is a **very large** architecture change.

**Estimated performance gain**: 1.5x-3x on generic/polymorphic code.

---

### 3.4 No Unboxed Aggregate Types (Arrays, Tuples)

**What**: Coral lists are runtime heap objects (`Vec<ValueHandle>`) where each
element is a NaN-boxed `i64`. There are no unboxed arrays, tuples, or structs
at the language level.

**Cost**:
- A list of 1000 integers stores 1000 × 8 bytes of NaN-boxed values plus a
  heap allocation header. In Rust, `Vec<i32>` stores 1000 × 4 bytes contiguous.
- Every element access goes through a runtime FFI call (`coral_list_get`),
  extracts a pointer, dereferences, and unboxes. Native array access is a
  single memory load.
- No SIMD vectorization possible — elements are not contiguous typed values.
- Cache performance degraded — boxed values interleave type tags with data.

**What Rust/C do differently**: Typed arrays, tuples, and structs with known layout.
Compiler can emit `getelementptr` for O(1) field access with no function call.

**Mitigation applied**: Store fields now use indexed struct access (O(1) vector
lookup), but the values within the struct are still NaN-boxed.

**What would fix it**:
1. Typed array syntax: `[Int]` compiles to contiguous `i64` storage with direct
   `getelementptr` access.
2. Tuple types: `(Int, String)` compiles to a struct with known field offsets.
3. This requires type information flowing to codegen (same prerequisite as §3.1).

**Estimated performance gain**: 3x-10x on array-heavy code (matrix operations,
numerical computation).

---

### 3.5 Runtime FFI Call Overhead

**What**: Many operations that could be inlined (list get/set, string length,
type checks) go through FFI calls to the Rust runtime (`extern "C"` functions).

**Cost**:
- Each FFI call has calling convention overhead: argument register setup, call
  instruction, return. ~5ns per call even for trivial operations.
- FFI calls are opaque to LLVM — the optimizer cannot see through them, blocking
  constant propagation, dead code elimination, and loop invariant code motion.
- Hot inner loops doing `list.get(i)` pay ~5ns per iteration for the call alone.

**What Rust does differently**: Operations are compiled inline. `vec[i]` compiles
to a bounds check + memory load — no function call.

**Mitigation applied**: Math functions converted to LLVM intrinsics (83x improvement).
Store field access converted to direct struct indexing.

**What would fix it**: Inline the most common runtime operations into LLVM IR:
1. `list_get` / `list_set` → bounds check + `getelementptr` + load/store
2. `string_len` → load length field from string header
3. `type_of` → extract NaN-box tag bits with bitwise ops
4. `list_len` → load length field from list header

This is **moderate effort** — requires knowing the runtime object layout in codegen
and emitting the appropriate LLVM IR directly.

**Estimated performance gain**: 2x-5x on list/string-heavy code.

---

### 3.6 No Stack Allocation for Value Types

**What**: Every composite value (string, list, map, store instance) is heap-
allocated via `Box::new()` in the runtime, even if it's a local variable that
never escapes the function.

**Cost**:
- Heap allocation: `malloc` + bookkeeping = ~50-100ns per allocation.
- Deallocation: RC decrement + conditional `free` = ~20-50ns.
- A function creating a temporary list, transforming it, and returning a scalar
  pays full allocation cost for a value that could live on the stack.

**What Rust does differently**: Values are stack-allocated by default. Heap
allocation is explicit (`Box::new`, `Vec::new`). The compiler's escape analysis
and borrow checker ensure stack references are safe.

**Mitigation applied**: None.

**What would fix it**: Escape analysis + stack allocation for non-escaping values.
Requires lifetime analysis in the semantic pass — a significant addition to
the type system.

**Estimated performance gain**: 1.5x-3x on code with many short-lived temporaries.

---

### 3.7 String Representation and Comparison

**What**: Strings in Coral are Rust `String` objects (heap-allocated UTF-8 byte
buffers). String comparison is byte-by-byte. Pattern matching on strings uses
runtime string comparison.

**Cost**:
- Match arms comparing against string literals do O(n) byte comparison per arm.
  A match with 10 string arms costs 10 × O(n) comparisons in the worst case.
- String creation requires heap allocation even for string literals.
- No string deduplication or interning — the same literal string can exist in
  memory multiple times.

**What would fix it**:
1. **String interning**: Deduplicate string literals at compile time. Compare
   interned strings by pointer equality (O(1) instead of O(n)).
2. **Perfect hash dispatch**: For match expressions on strings, generate a
   perfect hash table at compile time for O(1) dispatch.
3. **Small string optimization**: Strings ≤23 bytes stored inline in the NaN-box
   payload without heap allocation.

**Estimated performance gain**: 2x-5x on string-match-heavy code (the pattern_matching
benchmark would benefit enormously).

---

### Summary: Performance Ceiling Analysis

| Issue | Gap vs Native | Fix Difficulty | Impact Breadth |
|-------|---------------|---------------|----------------|
| NaN-boxing overhead | 3-5x | **Very Hard** | All code |
| RC without escape analysis | 2-4x | Hard | Allocation-heavy code |
| No monomorphization | 1.5-3x | **Very Hard** | Generic/polymorphic code |
| No unboxed aggregates | 3-10x | Hard | Array/matrix code |
| FFI call overhead | 2-5x | **Medium** | List/string-heavy code |
| No stack allocation | 1.5-3x | Hard | Temporary-heavy code |
| String comparison | 2-5x | Medium | String matching code |

**Realistic performance ceiling without addressing these**: Coral is currently
~5-50x slower than equivalent Rust code across workloads. The optimizations
applied in this sprint reduced the gap from ~50-200x to ~5-50x.

**To reach within 2x of Rust**: Type-specialized codegen (§3.1) + escape analysis
(§3.2) + inline FFI (§3.5) would be required.

**To reach Rust parity**: All seven issues must be addressed. This effectively
means building a fully typed, ownership-aware compilation pipeline.

