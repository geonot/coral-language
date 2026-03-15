# Performance Gap #1: Type-Specialized Code Generation (Universal NaN-Boxing Elimination)

**Gap vs Native:** 3–5x across all code  
**Fix Difficulty:** Very Hard  
**Impact Breadth:** All code paths  
**Affected Benchmarks:** All 12, especially tight_loop (90ms), fibonacci (37ms), math_compute (1266ms), matrix_mul (4197ms)

---

## Problem Statement

Every value in Coral is a 64-bit NaN-boxed `i64`. Integer arithmetic requires `bitcast i64→f64`, compute in `f64`, `bitcast f64→i64`. Native integer add is 1 cycle; NaN-boxed "integer" add is ~4 cycles. LLVM cannot apply integer optimizations (strength reduction, constant folding, loop induction variable widening) because it only sees `f64` operations. SIMD vectorization is blocked entirely. Branch conditions go through float comparison instead of integer flags.

The semantic analysis pass already resolves types via constraint-based inference (`SemanticModel.resolved_types: HashMap<String, TypeId>`) and the `TypeId` enum distinguishes `Primitive::Int`, `Primitive::Float`, `Primitive::Bool`, `Primitive::String`. The codegen already has a partial fast path (`emit_numeric_binary` with `both_numeric` flag, `try_emit_condition_i1`). The gap is that **local variables, function parameters, and return values** are always `i64` (NaN-boxed), never native `i64`/`f64`.

---

## Action Items (Ordered)

### 1. Extend SemanticModel to export per-variable type annotations
- **What:** Add `resolved_locals: HashMap<(FunctionName, VarName), TypeId>` and `resolved_params: HashMap<(FunctionName, usize), TypeId>` to `SemanticModel`
- **Where:** `src/semantic.rs`, `src/types/core.rs`
- **Why:** Codegen needs to know which variables are statically `Int`, `Float`, `Bool` to allocate them as native types
- **Complexity:** Medium

### 2. Add unboxed local variable slots in codegen
- **What:** When a local variable's resolved type is `Int` or `Float`, emit `alloca i64` or `alloca double` instead of `alloca i64` (NaN-boxed). Track which locals are unboxed in `FunctionContext`
- **Where:** `src/codegen/mod.rs` — variable declaration emission
- **Complexity:** Medium

### 3. Emit native integer operations for Int-typed expressions
- **What:** When both operands of `+`, `-`, `*`, `/`, `%` are `Int`, emit `add i64`, `sub i64`, `mul i64`, `sdiv i64`, `srem i64` instead of `fadd double`. When operands are `Float`, emit `fadd double`, `fsub double`, etc. directly without NaN-box wrapping
- **Where:** `src/codegen/mod.rs` — `emit_numeric_binary`
- **Complexity:** Medium

### 4. Emit native comparison instructions
- **What:** For `Int` comparisons, emit `icmp slt`, `icmp sle`, `icmp eq`, etc. For `Float`, continue using `fcmp`. Eliminate the `bitcast` round-trip for `try_emit_condition_i1`
- **Where:** `src/codegen/mod.rs` — `try_emit_condition_i1`, conditional branch emission
- **Complexity:** Low

### 5. Add box/unbox coercion at function call boundaries
- **What:** When calling a function with a known signature (all params typed), pass unboxed values directly. When calling a polymorphic function or storing into a polymorphic container, insert `box` instruction (native → NaN-boxed). When receiving from polymorphic context, insert `unbox` (NaN-boxed → native with tag check)
- **Where:** `src/codegen/mod.rs` — `emit_call_expression`, function prologue/epilogue
- **Complexity:** Hard

### 6. Specialize function signatures for monomorphic call sites
- **What:** When a function is only ever called with `(Int, Int) → Int`, generate the LLVM function with `i64, i64 → i64` signature instead of `i64, i64 → i64` (NaN-boxed). Track call-site type profiles during semantic analysis
- **Where:** `src/semantic.rs` (call-site analysis), `src/codegen/mod.rs` (function declaration)
- **Complexity:** Hard

### 7. Enable LLVM integer optimization passes
- **What:** With native `i64` operations in the IR, LLVM's standard passes (InstCombine, LoopStrengthReduction, IndVarSimplify, LICM) will activate automatically. Verify by inspecting optimized IR output with `opt -O2`
- **Where:** `src/codegen/mod.rs` — LLVM pass manager integration
- **Complexity:** Low (once items 1–6 are done)

### 8. For-loop induction variable optimization
- **What:** Range-based `for i in range(0, n)` should compile to a native `i64` induction variable with `add i64 %i, 1` increment and `icmp slt i64 %i, %n` termination, not NaN-boxed iterator stepping
- **Where:** `src/codegen/mod.rs` — for-loop emission
- **Complexity:** Medium

### 9. Mirror in self-hosted compiler
- **What:** Extend `self_hosted/semantic.coral` to propagate type annotations. Extend `self_hosted/codegen.coral` to emit native integer/float LLVM IR instructions when types are known
- **Where:** `self_hosted/semantic.coral`, `self_hosted/codegen.coral`
- **Complexity:** Hard (Coral lacks type system for tracking its own type annotations; must use string-based type tags)

---

## Implementation Plan

### Phase A: Type Annotation Plumbing (Items 1–2)

In `src/semantic.rs`, after the constraint solver runs, walk the resolved type map and for each function body, record the resolved type of each local binding and each parameter. Store these as `HashMap<(String, String), TypeId>` keyed by `(fn_name, var_name)`. In `src/codegen/mod.rs`, during function body emission, check `resolved_locals` before emitting `alloca`. If the type is `Primitive::Int`, emit `alloca i64` and mark the variable as `unboxed_int` in a new `FunctionContext.unboxed_vars: HashMap<String, UnboxedType>` enum. Similarly for `Float` → `alloca double`, `Bool` → `alloca i1`.

### Phase B: Arithmetic and Comparison (Items 3–4)

Refactor `emit_numeric_binary` to have three paths:
1. **Both Int:** `value_to_int_fast` → `add/sub/mul/sdiv/srem i64` → store directly (no wrap)
2. **Both Float:** `value_to_f64_fast` → `fadd/fsub/fmul/fdiv/frem double` → store directly (no wrap)
3. **Mixed/Unknown:** Current NaN-box path

For comparisons, add `try_emit_condition_i1_int` that uses `icmp` directly. Update `emit_if`, `emit_while`, `emit_for` to prefer the integer path.

### Phase C: Call Boundary Coercion (Items 5–6)

Define a `FunctionSignature` struct in codegen that knows which params are boxed vs unboxed. When emitting a call, if callee signature expects `i64` (native) but caller has `i64` (NaN-boxed), insert `bitcast_then_truncate`. If callee expects NaN-boxed but caller has native `i64`, insert `sext_then_bitcast`. Use a two-pass approach: first pass collects all function signatures; second pass emits bodies with coercion.

### Phase D: Loop and Pass Optimization (Items 7–8)

For `for i in range(start, end)`, detect the range pattern in codegen and emit:
```llvm
entry:
  %i = alloca i64
  store i64 %start, %i
  br label %loop
loop:
  %cur = load i64, %i
  %cmp = icmp slt i64 %cur, %end
  br i1 %cmp, label %body, label %exit
body:
  ; loop body with %cur as native i64
  %next = add i64 %cur, 1
  store i64 %next, %i
  br label %loop
```

Integrate LLVM's `PassManager` with `-O2` equivalent passes after module generation.

### Phase E: Self-Hosted Mirror (Item 9)

In `self_hosted/semantic.coral`, add a `var_types` map populated during type inference. In `self_hosted/codegen.coral`, check `var_types` before emitting arithmetic IR strings. Emit `add i64` vs `fadd double` based on resolved type. Add box/unbox helper functions `emit_box_int` and `emit_unbox_int` for boundary crossings.

---

## Implementation Prompt

```
Implement type-specialized code generation in the Coral compiler to eliminate NaN-boxing overhead for statically-typed variables.

CONTEXT:
- Coral uses NaN-boxed i64 for all values. The semantic pass already resolves types via TypeId (Primitive::Int, Primitive::Float, etc.)
- CodeGenerator in src/codegen/mod.rs already has partial specialization (emit_numeric_binary with both_numeric flag)
- Goal: variables known to be Int should use native i64; Float should use native f64; Bool should use native i1

CHANGES REQUIRED:

1. src/semantic.rs: After constraint solving, populate two new fields on SemanticModel:
   - resolved_locals: HashMap<(String, String), TypeId> mapping (fn_name, var_name) → resolved type
   - resolved_params: HashMap<(String, usize), TypeId> mapping (fn_name, param_index) → resolved type

2. src/codegen/mod.rs: Add to FunctionContext:
   - unboxed_vars: HashMap<String, UnboxedKind> where UnboxedKind is enum { NativeInt, NativeFloat, NativeBool, Boxed }
   
3. src/codegen/mod.rs: In variable declaration (emit_variable_decl or equivalent):
   - Lookup resolved type from semantic model
   - If Int: alloca i64, mark as NativeInt
   - If Float: alloca double, mark as NativeFloat  
   - If Bool: alloca i1, mark as NativeBool
   - Otherwise: alloca i64 (NaN-boxed), mark as Boxed

4. src/codegen/mod.rs: In emit_numeric_binary:
   - Add path for both-Int: emit add/sub/mul/sdiv/srem i64 directly
   - Add path for both-Float: emit fadd/fsub/fmul/fdiv/frem double directly
   - Keep existing NaN-box path as fallback

5. src/codegen/mod.rs: In function call emission:
   - When passing unboxed value to boxed parameter: insert boxing coercion
   - When receiving boxed value into unboxed variable: insert unboxing coercion
   - Boxing Int: build_bitcast(i64_to_f64) if within f64 range, else NaN-box encode
   - Unboxing to Int: extract from NaN-box, bitcast f64→i64

6. src/codegen/mod.rs: In for-loop emission for range patterns:
   - Detect range(start, end) calls
   - Emit native i64 induction variable with icmp slt + add i64

7. self_hosted/semantic.coral: Add var_types map, populate during type resolution
8. self_hosted/codegen.coral: Emit native i64/f64 ops when var_types indicates Int/Float

TEST: Run cargo test. Run benchmarks/run_benchmarks.py --release --runs 3.
Expected: tight_loop should drop from 90ms toward 20-30ms. fibonacci should drop from 37ms toward 10-15ms.
```
