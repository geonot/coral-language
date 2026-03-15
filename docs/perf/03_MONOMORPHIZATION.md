# Performance Gap #3: Monomorphization

**Gap vs Native:** 1.5–3x on generic/polymorphic code  
**Fix Difficulty:** Very Hard  
**Impact Breadth:** Generic functions, polymorphic containers, any code using higher-order functions  
**Affected Benchmarks:** closures (67ms), list_ops (417ms), recursion (69ms), pattern_matching (6134ms)

---

## Problem Statement

All Coral functions have a single compiled body operating on NaN-boxed `i64` values. A function like `*max(a, b)` is compiled once for all types — it cannot use `maxsd` for floats or `cmp` for integers, requiring runtime type dispatch. Generic containers (`List`, `Map`) store boxed values with no `Vec<i32>` equivalent. No opportunity for type-specific LLVM optimization exists because the optimizer sees only `i64` operations.

In Rust, `fn max<T: Ord>(a: T, b: T)` generates specialized machine code for each concrete type T. In Coral, all types flow through the same code path, paying boxing/dispatching overhead on every operation.

Monomorphization requires: type parameter tracking through codegen, template instantiation (one LLVM function per type combination), and code size management to avoid exponential explosion.

---

## Action Items (Ordered)

### 1. Profile-guided hot function identification
- **What:** Instrument the compiler to identify which functions are called with consistent type profiles. Functions always called with `(Int, Int) → Int` are monomorphization candidates. Functions called with varying types stay polymorphic
- **Where:** `src/semantic.rs` — call-site type analysis; new `MonomorphInfo` struct
- **Complexity:** Medium

### 2. Implement call-site type collection
- **What:** During semantic analysis, for each call expression, record the resolved types of all arguments. Build a `call_type_profiles: HashMap<FunctionName, Vec<Vec<TypeId>>>` mapping each function to all observed call signatures
- **Where:** `src/semantic.rs`
- **Complexity:** Medium

### 3. Identify monomorphization candidates
- **What:** A function is a monomorphization candidate if: (a) all call sites agree on a single type signature, OR (b) there are ≤4 distinct type signatures (to limit code bloat). Exclude recursive functions from initial implementation
- **Where:** `src/semantic.rs` — post-inference analysis
- **Complexity:** Low

### 4. Generate specialized function clones
- **What:** For each monomorphization candidate, generate a specialized LLVM function with concrete typed parameters. E.g., `*add(a, b)` called only with `(Int, Int)` generates `coral_add_Int_Int(i64, i64) → i64` using native `add i64` instructions
- **Where:** `src/codegen/mod.rs` — new `emit_specialized_function` method
- **Complexity:** Hard

### 5. Rewrite call sites to use specialized variants
- **What:** At each call site where the argument types match a specialized variant, emit a direct call to the specialized function instead of the polymorphic one. Insert box/unbox coercions only at boundaries between specialized and polymorphic code
- **Where:** `src/codegen/mod.rs` — call expression emission
- **Complexity:** Medium

### 6. Dead polymorphic function elimination
- **What:** After rewriting all call sites, if the original polymorphic function has no remaining callers, eliminate it. This reduces binary size and enables further LLVM optimizations (no opaque function bodies blocking inlining)
- **Where:** `src/codegen/mod.rs` — post-emission cleanup pass
- **Complexity:** Low

### 7. Specialize higher-order function calls
- **What:** For HOF patterns like `list.map(func)` where `func` is a known function with known types, inline the function body into the map loop and specialize the element type. This turns `map(box→unbox→apply→box)` into `map(apply_native)`
- **Where:** `src/codegen/mod.rs` — builtin method emission, closure inlining
- **Complexity:** Very Hard

### 8. Container type specialization
- **What:** When a `List` is used exclusively with `Int` elements (all `push` calls pass `Int`, all `get` results used as `Int`), generate a specialized `List_Int` that stores contiguous `i64` values instead of NaN-boxed values. This enables SIMD and eliminates per-element boxing
- **Where:** `src/codegen/mod.rs`, `runtime/src/list_ops.rs` (specialized list FFI)
- **Complexity:** Very Hard

### 9. Mirror in self-hosted compiler
- **What:** Implement type-profile collection in `self_hosted/semantic.coral` and function cloning in `self_hosted/codegen.coral`
- **Where:** `self_hosted/semantic.coral`, `self_hosted/codegen.coral`
- **Complexity:** Hard

---

## Implementation Plan

### Phase A: Type Profile Collection (Items 1–3)

In `src/semantic.rs`, after type inference completes:

```rust
pub struct MonomorphInfo {
    pub candidates: HashMap<String, Vec<MonomorphVariant>>,
}

pub struct MonomorphVariant {
    pub param_types: Vec<TypeId>,
    pub return_type: TypeId,
    pub call_count: usize,
}

fn collect_type_profiles(model: &SemanticModel, ast: &[Statement]) -> MonomorphInfo {
    let mut profiles: HashMap<String, HashMap<Vec<TypeId>, usize>> = HashMap::new();
    
    // Walk all call expressions
    for call in ast.all_calls() {
        let fn_name = call.callee_name();
        let arg_types: Vec<TypeId> = call.args.iter()
            .map(|arg| model.resolve_expr_type(arg))
            .collect();
        *profiles.entry(fn_name).or_default()
            .entry(arg_types).or_default() += 1;
    }
    
    // Filter: ≤4 distinct signatures, non-recursive
    let candidates = profiles.into_iter()
        .filter(|(name, sigs)| sigs.len() <= 4 && !is_recursive(name, ast))
        .map(|(name, sigs)| {
            let variants = sigs.into_iter()
                .map(|(types, count)| MonomorphVariant {
                    param_types: types,
                    return_type: infer_return_for(name, &types, model),
                    call_count: count,
                })
                .collect();
            (name, variants)
        })
        .collect();
    
    MonomorphInfo { candidates }
}
```

### Phase B: Specialized Function Generation (Items 4–5)

In `src/codegen/mod.rs`:

```rust
fn emit_specialized_function(
    &mut self,
    fn_name: &str,
    fn_def: &FunctionDef,
    variant: &MonomorphVariant,
) -> FunctionValue<'ctx> {
    let mangled = format!("{}_{}", fn_name, 
        variant.param_types.iter().map(|t| t.short_name()).collect::<Vec<_>>().join("_"));
    
    // Build LLVM function type with concrete types
    let param_types: Vec<BasicMetadataTypeEnum> = variant.param_types.iter()
        .map(|t| match t {
            TypeId::Primitive(Primitive::Int) => self.usize_type.into(),
            TypeId::Primitive(Primitive::Float) => self.f64_type.into(),
            TypeId::Primitive(Primitive::Bool) => self.bool_type.into(),
            _ => self.runtime.value_i64_type.into(), // boxed fallback
        })
        .collect();
    
    let ret_type = match &variant.return_type {
        TypeId::Primitive(Primitive::Int) => self.usize_type.into(),
        TypeId::Primitive(Primitive::Float) => self.f64_type.into(),
        _ => self.runtime.value_i64_type.into(),
    };
    
    let fn_type = ret_type.fn_type(&param_types, false);
    let function = self.module.add_function(&mangled, fn_type, None);
    
    // Emit body with unboxed context (reuse Phase A type specialization from Gap #1)
    // ...
    
    function
}
```

At call sites:
```rust
fn emit_call_expression(&mut self, ctx: &mut FunctionContext, call: &CallExpr) -> ... {
    if let Some(variants) = self.monomorph_info.candidates.get(call.name) {
        let arg_types: Vec<TypeId> = call.args.iter()
            .map(|a| self.resolve_expr_type(a))
            .collect();
        
        if let Some(variant) = variants.iter().find(|v| v.param_types == arg_types) {
            let specialized_fn = self.specialized_functions[&(call.name, arg_types)];
            // Emit direct call with native-typed arguments
            return self.emit_specialized_call(ctx, specialized_fn, call, variant);
        }
    }
    // Fall back to polymorphic call
    self.emit_polymorphic_call(ctx, call)
}
```

### Phase C: Higher-Order Specialization (Items 7–8)

For `list.map(func)` where `func` is a known function:
1. Resolve `func` to its definition
2. If `func` operates on `Int → Int`, generate an inlined loop:
   ```llvm
   for each element in list (as raw i64):
     unbox element → native i64
     apply func body inline → native i64 result
     box result → i64
     store in result list
   ```
3. Better yet, with container specialization, the elements are already native, eliminating the box/unbox.

For `List_Int` specialization:
- Create a parallel set of runtime functions: `coral_list_int_new`, `coral_list_int_push(list, i64)`, `coral_list_int_get(list, i64) → i64`
- These store contiguous `i64` values, enabling cache-friendly access
- Codegen detects monomorphic list usage and routes to specialized functions

### Phase D: Self-Hosted Mirror (Item 9)

In `self_hosted/semantic.coral`, collect call-site types as string tags (e.g., `"Int,Int→Int"`). In `self_hosted/codegen.coral`, generate mangled function names and specialized IR when all call sites agree on types.

---

## Implementation Prompt

```
Implement monomorphization for the Coral compiler to generate type-specialized function clones.

CONTEXT:
- All Coral functions compile to a single body operating on NaN-boxed i64 values
- The semantic pass resolves types via TypeId enum (Primitive::Int, Primitive::Float, etc.)
- Goal: when a function is always called with the same concrete types, generate a specialized clone using native types

CHANGES REQUIRED:

1. src/semantic.rs — Type profile collection:
   - After inference, walk all CallExpression nodes
   - For each call, record resolved types of all arguments
   - Build call_type_profiles: HashMap<String, Vec<(Vec<TypeId>, usize)>>
   - Mark functions with ≤4 distinct type profiles and non-recursive as monomorphization candidates
   - Store in MonomorphInfo on SemanticModel

2. src/codegen/mod.rs — Specialized function generation:
   - For each candidate, generate a specialized LLVM function with native-typed params
   - Mangle name: fn_name + "_" + type1 + "_" + type2 (e.g., "max_Int_Int")  
   - Use i64 for Int params, double for Float params
   - Emit native instructions in body (add i64 for Int+Int, fadd for Float+Float)
   - Store mapping: (fn_name, type_signature) → FunctionValue

3. src/codegen/mod.rs — Call site rewriting:
   - At each call, check if specialized variant exists for the call's arg types
   - If yes: emit direct call to specialized function with native args
   - Insert box→unbox coercions only at polymorphic boundaries
   - If no: fall back to current polymorphic call

4. src/codegen/mod.rs — Dead code elimination:
   - After all call sites rewritten, check if original polymorphic function has callers
   - If no callers remain, remove from module

5. self_hosted/semantic.coral — Collect type profiles as string-based tags
6. self_hosted/codegen.coral — Generate specialized function IR for matching profiles

TEST: cargo test. Run benchmarks focusing on closures, recursion.
Expected: recursion should improve 1.5-2x (from 69ms toward 40ms).
```
