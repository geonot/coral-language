# Performance Gap #4: Unboxed Aggregate Types (Arrays, Tuples, Structs)

**Gap vs Native:** 3–10x on array/matrix code  
**Fix Difficulty:** Hard  
**Impact Breadth:** Array-heavy code, numerical computation, data processing  
**Affected Benchmarks:** matrix_mul (4197ms), list_ops (417ms), for_iteration (305ms), store_ops (6290ms)

---

## Problem Statement

Coral lists are runtime heap objects (`Vec<ValueHandle>`) where each element is a NaN-boxed `i64`. There are no unboxed arrays, tuples, or structs at the language level. A list of 1000 integers stores 1000 × 8 bytes of NaN-boxed values plus a heap allocation header. In Rust, `Vec<i32>` stores 1000 × 4 bytes contiguous. Every element access goes through a runtime FFI call (`coral_list_get`), extracts a pointer, dereferences, and unboxes. Native array access is a single `getelementptr` + `load`. No SIMD vectorization is possible because elements are not contiguous typed values. Cache performance is degraded — boxed values interleave type tags with data.

Store fields use indexed struct access (O(1) vector lookup via `store_field_indices`), but the values within the struct are still NaN-boxed. Accessing a store field that is known to be an integer still pays the NaN-box decode cost.

---

## Action Items (Ordered)

### 1. Implement typed list detection in semantic analysis
- **What:** When a list variable has all `push` calls passing the same resolved type (e.g., all `Int`), and all `get` results are used in numeric context, mark it as a typed list: `TypedList(TypeId::Primitive(Primitive::Int))`
- **Where:** `src/semantic.rs` — add `typed_lists: HashMap<(String, String), TypeId>` to `SemanticModel`
- **Complexity:** Medium

### 2. Create runtime typed list operations
- **What:** Add specialized runtime functions for `List<i64>` and `List<f64>`: `coral_typed_list_i64_new`, `coral_typed_list_i64_push(list, i64)`, `coral_typed_list_i64_get(list, usize) → i64`, `coral_typed_list_i64_set(list, usize, i64)`, `coral_typed_list_i64_len(list) → usize`
- **Where:** `runtime/src/list_ops.rs` — new typed list implementation backed by `Vec<i64>` / `Vec<f64>`
- **Complexity:** Medium

### 3. Emit direct getelementptr for typed list access
- **What:** When accessing a typed list, instead of calling `coral_list_get` (FFI), emit inline LLVM IR: bounds check → `getelementptr i64, ptr %data, i64 %index` → `load i64`. This is a single instruction sequence instead of a function call
- **Where:** `src/codegen/mod.rs` — list method emission (`builtins.rs`)
- **Complexity:** Medium

### 4. Implement store field type specialization
- **What:** When all fields of a store have known static types, generate a C-like struct layout in LLVM IR with concrete field types. `store Point { x ? 0, y ? 0 }` → `{ i64, i64 }` instead of `Vec<NaN-boxed i64>`. Field access becomes `getelementptr { i64, i64 }, ptr %store, 0, field_index` → single load instruction
- **Where:** `src/codegen/mod.rs`, `src/codegen/store_actor.rs`
- **Complexity:** Hard

### 5. Implement tuple type support
- **What:** Add tuple syntax (or use existing list-like patterns) where fixed-size, typed tuples compile to LLVM struct types. `(1, "hello", true)` → `{ i64, ptr, i1 }`. Element access by index → `extractvalue`
- **Where:** `src/parser.rs` (if syntax needed), `src/codegen/mod.rs`
- **Complexity:** Hard

### 6. Enable SIMD vectorization for typed lists
- **What:** With contiguous `i64` or `f64` storage, LLVM's auto-vectorizer can process 4 elements at once (AVX2) or 8 (AVX-512). Ensure loop structure is vectorization-friendly: no early exits, predictable access pattern, alignment
- **Where:** `src/codegen/mod.rs` — loop emission with alignment hints
- **Complexity:** Medium (once items 1–3 are done)

### 7. Inline list iteration for typed lists
- **What:** For `for item in typed_list`, instead of calling the iterator FFI, emit a direct index loop: `for i = 0..len { item = getelementptr + load }`. This eliminates iterator allocation and FFI overhead per element
- **Where:** `src/codegen/mod.rs` — for-loop emission
- **Complexity:** Medium

### 8. Implement matrix/2D array specialization
- **What:** Detect `List<List<Int>>` patterns (list of lists with uniform inner type) and flatten to a contiguous 2D array: `[rows × cols × sizeof(i64)]`. Access `matrix[i][j]` → single `getelementptr` with `i * cols + j`. Eliminates double indirection
- **Where:** `src/semantic.rs` (nested list detection), `src/codegen/mod.rs`
- **Complexity:** Hard

### 9. Mirror in self-hosted compiler
- **What:** Emit typed list IR in `self_hosted/codegen.coral` when using specialized list functions. Emit struct-based store layouts
- **Where:** `self_hosted/codegen.coral`
- **Complexity:** Medium

---

## Implementation Plan

### Phase A: Typed List Detection (Item 1)

In `src/semantic.rs`, after type inference:

```rust
fn detect_typed_lists(model: &mut SemanticModel, ast: &[Statement]) {
    // For each function, find all list variables
    for func in ast.functions() {
        for local in func.locals_of_type(TypeId::List(_)) {
            let element_type = analyze_list_element_type(local, func);
            if let Some(concrete_type) = element_type {
                model.typed_lists.insert(
                    (func.name.clone(), local.name.clone()),
                    concrete_type,
                );
            }
        }
    }
}

fn analyze_list_element_type(var: &str, func: &Function) -> Option<TypeId> {
    let mut types = HashSet::new();
    for usage in func.usages_of(var) {
        match usage {
            Usage::MethodCall("push", [arg]) => types.insert(resolve_type(arg)),
            Usage::MethodCall("set", [_, arg]) => types.insert(resolve_type(arg)),
            _ => {}
        }
    }
    if types.len() == 1 { types.into_iter().next() } else { None }
}
```

### Phase B: Runtime Typed Lists (Items 2–3)

In `runtime/src/list_ops.rs`:

```rust
#[repr(C)]
pub struct TypedListI64 {
    header: ValueHeader,
    data: Vec<i64>,
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_typed_list_i64_new(capacity: usize) -> *mut TypedListI64 {
    Box::into_raw(Box::new(TypedListI64 {
        header: ValueHeader::new(),
        data: Vec::with_capacity(capacity),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_typed_list_i64_push(list: *mut TypedListI64, value: i64) {
    (*list).data.push(value);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_typed_list_i64_get(list: *const TypedListI64, index: usize) -> i64 {
    (*list).data[index]
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_typed_list_i64_len(list: *const TypedListI64) -> usize {
    (*list).data.len()
}
```

In codegen, for typed lists, emit inline LLVM IR instead of FFI calls:
```llvm
; list.get(i) for typed List<Int>
%data_ptr = getelementptr %TypedListI64, ptr %list, 0, 1, 0  ; data field, pointer
%elem_ptr = getelementptr i64, ptr %data_ptr, i64 %index
%value = load i64, ptr %elem_ptr
; No unboxing needed — value is native i64
```

### Phase C: Store Struct Layout (Item 4)

In `src/codegen/store_actor.rs`, when all store fields have known types:

```rust
fn emit_specialized_store_type(&mut self, store_name: &str, fields: &[(String, TypeId)]) {
    let field_types: Vec<BasicTypeEnum> = fields.iter()
        .map(|(_, ty)| match ty {
            TypeId::Primitive(Primitive::Int) => self.usize_type.into(),
            TypeId::Primitive(Primitive::Float) => self.f64_type.into(),
            TypeId::Primitive(Primitive::Bool) => self.bool_type.into(),
            _ => self.runtime.value_i64_type.into(), // NaN-boxed fallback
        })
        .collect();
    
    let struct_type = self.context.struct_type(&field_types, false);
    self.store_struct_types.insert(store_name.to_string(), struct_type);
}
```

Field access:
```llvm
; store.x where x is field 0 of type Int
%field_ptr = getelementptr %Point, ptr %store, 0, 0
%value = load i64, ptr %field_ptr
; Single instruction — no hash lookup, no NaN-box decode
```

### Phase D: Typed Iteration and SIMD (Items 6–7)

For `for item in typed_list`:
```llvm
entry:
  %len = call i64 @coral_typed_list_i64_len(ptr %list)
  %data = getelementptr %TypedListI64, ptr %list, 0, 1, 0
  br label %loop

loop:
  %i = phi i64 [ 0, %entry ], [ %next_i, %body ]
  %cmp = icmp ult i64 %i, %len
  br i1 %cmp, label %body, label %exit

body:
  %elem_ptr = getelementptr i64, ptr %data, i64 %i
  %item = load i64, ptr %elem_ptr, align 8
  ; Use %item directly as native i64
  %next_i = add i64 %i, 1
  br label %loop
```

With `align 8` and a simple loop body, LLVM's LoopVectorize pass will auto-vectorize this to process 4 elements per iteration (AVX2) or 8 (AVX-512).

### Phase E: Matrix Flattening (Item 8)

Detect `List<List<Int>>` where all inner lists have the same length:
```rust
// In semantic analysis:
if is_list_of_typed_lists(var, TypeId::Primitive(Primitive::Int)) {
    if inner_lists_same_length(var, func) {
        model.flat_matrices.insert((fn_name, var_name), (rows, cols, TypeId::Primitive(Primitive::Int)));
    }
}
```

In codegen, allocate flat buffer and index with `row * cols + col`:
```llvm
%flat = alloca [rows * cols x i64]
; matrix[i][j]:
%offset = mul i64 %i, %cols
%index = add i64 %offset, %j  
%ptr = getelementptr [N x i64], ptr %flat, 0, %index
%val = load i64, ptr %ptr
```

This transforms matrix_mul from O(n³) FFI calls to O(n³) memory loads — massive speedup.

### Phase F: Self-Hosted Mirror (Item 9)

In `self_hosted/codegen.coral`, when emitting list operations, check if the list is typed and emit the specialized function calls instead of generic list FFI calls.

---

## Implementation Prompt

```
Implement unboxed aggregate types (typed lists, specialized stores) in the Coral compiler and runtime.

CONTEXT:
- Coral lists are Vec<ValueHandle> where each element is NaN-boxed i64
- List access goes through FFI: coral_list_get(list, index) → i64
- Store fields use indexed vector (O(1)) but values are NaN-boxed
- matrix_mul benchmark is 4197ms — should be ~50ms with native array access
- SemanticModel already has store_field_indices and typed inference

CHANGES REQUIRED:

1. src/semantic.rs — Typed list detection:
   - After inference, analyze each list variable's push/set calls
   - If all elements are same type, mark as typed_lists: HashMap<(String,String), TypeId>
   - Also detect List<List<T>> for matrix flattening

2. runtime/src/list_ops.rs — Typed list runtime:
   - Add TypedListI64 struct: { header: ValueHeader, data: Vec<i64> }
   - Add coral_typed_list_i64_new, _push, _get, _set, _len functions
   - Same for TypedListF64

3. src/codegen/mod.rs and src/codegen/builtins.rs — Inline list access:
   - For typed lists, emit getelementptr + load instead of FFI call
   - For typed list iteration, emit direct index loop
   - Add alignment hints for SIMD vectorization

4. src/codegen/store_actor.rs — Specialized store layout:
   - When all fields have known types, generate LLVM struct type
   - Field access → getelementptr instead of vector index + unbox
   - Constructor → alloca struct + store fields

5. src/codegen/mod.rs — For-loop specialization:
   - For `for item in typed_list`, emit counted loop with native element load
   - No iterator allocation, no FFI per element

6. self_hosted/codegen.coral — Emit typed list IR for detected cases

TEST: cargo test. Run matrix_mul benchmark specifically.
Expected: matrix_mul should drop from 4197ms toward 100-200ms.
```
