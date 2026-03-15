# Performance Gap #5: Runtime FFI Call Overhead Elimination

**Gap vs Native:** 2–5x on list/string-heavy code  
**Fix Difficulty:** Medium  
**Impact Breadth:** List operations, string operations, type checks, any code calling runtime functions  
**Affected Benchmarks:** list_ops (417ms), string_ops (30ms), map_ops (59ms), math_compute (1266ms), for_iteration (305ms), pattern_matching (6134ms)

---

## Problem Statement

Many operations that could be inlined go through FFI calls to the Rust runtime (`extern "C"` functions). Each FFI call has calling convention overhead: argument register setup, call instruction, stack frame, return (~5ns per call). FFI calls are opaque to LLVM — the optimizer cannot see through them, blocking constant propagation, dead code elimination, loop invariant code motion, and auto-vectorization.

Hot inner loops doing `list.get(i)` pay ~5ns per iteration for the FFI call alone. `string.length()` is an FFI call to read a single struct field. `type_of(value)` is an FFI call to extract 3 bits from a NaN-boxed value. The runtime has ~220 FFI functions when most hot-path operations could be 1–3 LLVM instructions each.

Math functions have already been converted to LLVM intrinsics (83x improvement). Store field access has been converted to direct struct indexing. The remaining FFI hot spots are: list get/set/len, map get/set, string length/charAt, type checking, comparison operations, and iterator stepping.

---

## Action Items (Ordered)

### 1. Inline list_get as bounds check + getelementptr + load
- **What:** Replace `coral_list_get(list_ptr, index)` FFI call with inline LLVM IR: extract length field, compare index, branch to bounds-check failure or direct memory load. The list data layout is `{ header, length: i64, capacity: i64, data: ptr }` — document this in codegen and keep in sync with runtime
- **Where:** `src/codegen/builtins.rs` — list.get handling
- **Complexity:** Medium

### 2. Inline list_set as bounds check + getelementptr + store
- **What:** Same as list_get but with store instead of load. Also inline the retain of the new value and release of the old value (or skip if types are known non-heap from type info)
- **Where:** `src/codegen/builtins.rs`
- **Complexity:** Medium

### 3. Inline list_len as direct field load
- **What:** `list.length()` → load length field directly: `%len_ptr = getelementptr %List, ptr %list, 0, 1; %len = load i64, ptr %len_ptr`. Single instruction, no function call
- **Where:** `src/codegen/builtins.rs`
- **Complexity:** Low

### 4. Inline string_len as direct field load
- **What:** `string.length()` → load the length field from the string struct header. String layout is `{ header, length: i64, data: ptr }`. One memory load instead of FFI call
- **Where:** `src/codegen/builtins.rs`
- **Complexity:** Low

### 5. Inline type_of as bitwise tag extraction
- **What:** `type_of(value)` extracts the NaN-box tag using bitwise operations: `(bits >> 48) & 0x7` when the QNaN prefix is set, or "number" when it's not. This is 3–4 instructions instead of an FFI call
- **Where:** `src/codegen/builtins.rs` or `src/codegen/mod.rs`
- **Complexity:** Low

### 6. Inline is_number/is_string/is_list type checks
- **What:** These are just tag comparisons. `is_number(v)` → `(v & QNAN_MASK) != QNAN_PREFIX`. `is_string(v)` → has heap tag and string type marker. Emit inline bitwise ops
- **Where:** `src/codegen/mod.rs`
- **Complexity:** Low

### 7. Inline map_get with hash computation
- **What:** For string-keyed maps, inline the hash computation (use the same algorithm as runtime — FxHash or SipHash) and the lookup probe sequence. This allows LLVM to hoist invariant hash computations out of loops
- **Where:** `src/codegen/builtins.rs`
- **Complexity:** Hard

### 8. Inline iterator stepping for list/map iteration
- **What:** For `for item in list`, instead of calling `coral_list_iter_new` + `coral_list_iter_next`, emit a counted loop directly: `i = 0; while i < len { item = data[i]; i++ }`. Eliminates iterator allocation and per-step FFI
- **Where:** `src/codegen/mod.rs` — for-in-list emission
- **Complexity:** Medium

### 9. Document and freeze runtime data layouts
- **What:** Create a shared header file / constants file that defines the exact memory layout of List, Map, String, Store, and other runtime types. Both the Rust runtime and the LLVM codegen must agree on field offsets. Use `#[repr(C)]` on all runtime structs and document offsets as constants in codegen
- **Where:** `runtime/src/lib.rs` (repr(C) annotations), `src/codegen/runtime.rs` (offset constants)
- **Complexity:** Medium

### 10. Inline comparison operations (equals, not_equals, less_than)
- **What:** For known-type comparisons (both numeric), already done via `emit_numeric_binary`. Extend to string comparison: inline memcmp-based comparison for strings of known small length, or pointer equality check for interned strings (see Gap #7)
- **Where:** `src/codegen/mod.rs`
- **Complexity:** Medium

### 11. Mirror in self-hosted compiler
- **What:** In `self_hosted/codegen.coral`, emit inline IR for list_get, list_len, string_len, type checks instead of emitting FFI call instructions. Define the runtime struct offsets as constants
- **Where:** `self_hosted/codegen.coral`
- **Complexity:** Medium

---

## Implementation Plan

### Phase A: Freeze Runtime Layouts (Item 9)

First, ensure all hot-path runtime types use `#[repr(C)]` and document their layout:

In `runtime/src/lib.rs`:
```rust
/// Coral List layout (frozen — codegen depends on these offsets)
/// Offset 0: ValueHeader (refcount: u64, flags: u32, pad: u32) = 16 bytes
/// Offset 16: length (u64) = 8 bytes
/// Offset 24: capacity (u64) = 8 bytes
/// Offset 32: data pointer (*mut i64) = 8 bytes
#[repr(C)]
pub struct CoralList {
    pub header: ValueHeader,
    pub length: u64,
    pub capacity: u64,
    pub data: *mut i64, // NaN-boxed values
}
```

In `src/codegen/runtime.rs`, define matching constants:
```rust
pub const LIST_LENGTH_OFFSET: u32 = 2;  // field index in LLVM struct
pub const LIST_DATA_OFFSET: u32 = 4;
pub const STRING_LENGTH_OFFSET: u32 = 2;
pub const STRING_DATA_OFFSET: u32 = 3;
```

### Phase B: Inline List Operations (Items 1–3, 8)

In `src/codegen/builtins.rs`, when emitting `list.get(index)`:

```rust
fn emit_inline_list_get(&mut self, list: IntValue<'ctx>, index: IntValue<'ctx>) -> IntValue<'ctx> {
    // Extract pointer from NaN-box
    let list_ptr = self.emit_nb_to_ptr(list);
    
    // Load length
    let len_ptr = self.builder.build_struct_gep(
        self.list_struct_type, list_ptr, LIST_LENGTH_OFFSET, "len_ptr")?;
    let len = self.builder.build_load(self.usize_type, len_ptr, "len")?;
    
    // Bounds check
    let idx_i64 = self.value_to_number_fast(index);
    let idx = self.builder.build_float_to_unsigned_int(idx_i64, self.usize_type, "idx")?;
    let in_bounds = self.builder.build_int_compare(IntPredicate::ULT, idx, len, "bounds")?;
    
    // Branch: in-bounds → load, out-of-bounds → return None/error
    let then_bb = self.context.append_basic_block(ctx.function, "get.ok");
    let else_bb = self.context.append_basic_block(ctx.function, "get.oob");
    let merge_bb = self.context.append_basic_block(ctx.function, "get.merge");
    self.builder.build_conditional_branch(in_bounds, then_bb, else_bb)?;
    
    // In bounds: getelementptr + load
    self.builder.position_at_end(then_bb);
    let data_ptr = self.builder.build_struct_gep(
        self.list_struct_type, list_ptr, LIST_DATA_OFFSET, "data")?;
    let data = self.builder.build_load(self.ptr_type, data_ptr, "data_ptr")?;
    let elem_ptr = unsafe {
        self.builder.build_gep(self.usize_type, data, &[idx], "elem")?
    };
    let value = self.builder.build_load(self.usize_type, elem_ptr, "elem_val")?;
    self.builder.build_unconditional_branch(merge_bb)?;
    
    // Out of bounds: return None
    self.builder.position_at_end(else_bb);
    let none_val = self.none_value();
    self.builder.build_unconditional_branch(merge_bb)?;
    
    // Merge
    self.builder.position_at_end(merge_bb);
    let phi = self.builder.build_phi(self.usize_type, "get_result")?;
    phi.add_incoming(&[(&value, then_bb), (&none_val, else_bb)]);
    phi.as_basic_value().into_int_value()
}
```

For `list.length()`:
```rust
fn emit_inline_list_len(&mut self, list: IntValue<'ctx>) -> IntValue<'ctx> {
    let list_ptr = self.emit_nb_to_ptr(list);
    let len_ptr = self.builder.build_struct_gep(
        self.list_struct_type, list_ptr, LIST_LENGTH_OFFSET, "len_ptr")?;
    let len = self.builder.build_load(self.usize_type, len_ptr, "len")?;
    self.wrap_number_unchecked(
        self.builder.build_unsigned_int_to_float(len, self.f64_type, "len_f64")?
    )
}
```

For `for item in list` — direct counted loop:
```rust
fn emit_inline_list_iteration(&mut self, ctx: &mut FunctionContext, list: IntValue<'ctx>, body: ...) {
    let list_ptr = self.emit_nb_to_ptr(list);
    let len = self.load_list_length(list_ptr);
    let data = self.load_list_data(list_ptr);
    
    // Emit: for (i = 0; i < len; i++) { item = data[i]; body }
    let loop_bb = self.context.append_basic_block(ctx.function, "iter");
    let body_bb = self.context.append_basic_block(ctx.function, "iter.body");
    let exit_bb = self.context.append_basic_block(ctx.function, "iter.exit");
    
    self.builder.build_unconditional_branch(loop_bb)?;
    self.builder.position_at_end(loop_bb);
    
    let i = self.builder.build_phi(self.usize_type, "i")?;
    i.add_incoming(&[(&self.usize_type.const_zero(), /* entry block */)]);
    let i_val = i.as_basic_value().into_int_value();
    
    let cmp = self.builder.build_int_compare(IntPredicate::ULT, i_val, len, "cmp")?;
    self.builder.build_conditional_branch(cmp, body_bb, exit_bb)?;
    
    self.builder.position_at_end(body_bb);
    let elem_ptr = unsafe { self.builder.build_gep(self.usize_type, data, &[i_val], "ep")? };
    let item = self.builder.build_load(self.usize_type, elem_ptr, "item")?;
    // ... emit body with item ...
    let next_i = self.builder.build_int_add(i_val, self.usize_type.const_int(1, false), "next")?;
    i.add_incoming(&[(&next_i, body_bb)]);
    self.builder.build_unconditional_branch(loop_bb)?;
}
```

### Phase C: Inline String and Type Operations (Items 4–6)

String length:
```rust
fn emit_inline_string_len(&mut self, str_val: IntValue<'ctx>) -> IntValue<'ctx> {
    let str_ptr = self.emit_nb_to_ptr(str_val);
    let len_ptr = self.builder.build_struct_gep(
        self.string_struct_type, str_ptr, STRING_LENGTH_OFFSET, "slen_ptr")?;
    let len = self.builder.build_load(self.usize_type, len_ptr, "slen")?;
    self.wrap_number_unchecked(
        self.builder.build_unsigned_int_to_float(len, self.f64_type, "slen_f64")?
    )
}
```

Type tag extraction:
```rust
fn emit_inline_type_tag(&mut self, value: IntValue<'ctx>) -> IntValue<'ctx> {
    let qnan_mask = self.usize_type.const_int(0xFFF8_0000_0000_0000, false);
    let qnan_prefix = self.usize_type.const_int(0x7FF8_0000_0000_0000, false);
    
    let masked = self.builder.build_and(value, qnan_mask, "masked")?;
    let is_number = self.builder.build_int_compare(
        IntPredicate::NE, masked, qnan_prefix, "is_num")?;
    // If is_number: return 0 (number tag)
    // Else: extract tag bits (bits >> 48) & 0x7
    let shifted = self.builder.build_right_shift(value, 
        self.usize_type.const_int(48, false), false, "shifted")?;
    let tag = self.builder.build_and(shifted, 
        self.usize_type.const_int(0x7, false), "tag")?;
    self.builder.build_select(is_number, self.usize_type.const_zero(), tag, "type_tag")?
}
```

### Phase D: Inline Map Operations (Item 7)

For simple string-keyed maps with known key (constant string), inline the hash:
```rust
fn emit_inline_map_get_const_key(&mut self, map: IntValue<'ctx>, key: &str) -> IntValue<'ctx> {
    // Precompute hash at compile time
    let hash = fxhash(key.as_bytes());
    let hash_val = self.usize_type.const_int(hash, false);
    
    // Call a simplified lookup: coral_map_get_by_hash(map, hash, key_ptr, key_len)
    // This skips hash computation, only does probe sequence
    let result = self.builder.build_call(
        self.runtime.map_get_by_hash, 
        &[map.into(), hash_val.into(), key_ptr.into(), key_len.into()],
        "map_val")?;
    result.try_as_basic_value().left().unwrap().into_int_value()
}
```

### Phase E: Self-Hosted Mirror (Item 11)

In `self_hosted/codegen.coral`, define constants for struct offsets and emit getelementptr/load sequences as LLVM IR text strings instead of call instructions for the hot-path operations.

---

## Implementation Prompt

```
Implement inline FFI operation replacement in the Coral compiler to eliminate runtime function call overhead for hot-path operations.

CONTEXT:
- Coral's codegen emits FFI calls (extern "C") for ~220 runtime operations
- Each FFI call costs ~5ns overhead and blocks LLVM optimizations
- Math functions already converted to LLVM intrinsics (83x speedup achieved)
- Hot paths: list.get/set/length, string.length, type_of, for-in iteration
- Runtime types use #[repr(C)] layouts defined in runtime/src/lib.rs

CHANGES REQUIRED:

1. src/codegen/runtime.rs — Define layout constants:
   - LIST_HEADER_SIZE, LIST_LENGTH_OFFSET, LIST_DATA_OFFSET
   - STRING_HEADER_SIZE, STRING_LENGTH_OFFSET, STRING_DATA_OFFSET
   - Add LLVM struct type definitions matching runtime #[repr(C)] layouts
   - Verify offsets match with static_assert in runtime

2. src/codegen/builtins.rs — Inline list operations:
   - list.get(i): bounds check + getelementptr + load (replace coral_list_get)
   - list.set(i, v): bounds check + getelementptr + store + retain/release (replace coral_list_set)
   - list.length(): struct field load (replace coral_list_length)
   - list.push(v): length check + potential realloc + store (or fallback to FFI for realloc case)

3. src/codegen/builtins.rs — Inline string operations:
   - string.length(): struct field load (replace coral_string_length) 

4. src/codegen/mod.rs — Inline type operations:
   - type_of(v): bitwise tag extraction (3 instructions)
   - is_number(v): QNaN mask check (2 instructions)

5. src/codegen/mod.rs — Inline for-in-list iteration:
   - Replace: coral_list_iter_new + coral_list_iter_next loop
   - With: load length + data ptr, counted i = 0..len loop with getelementptr + load

6. runtime/src/lib.rs — Add static assertions:
   - Verify #[repr(C)] field offsets match codegen constants
   - Add compile-time size/offset checks

7. self_hosted/codegen.coral — Emit inline IR instead of call for hot-path ops

TEST: cargo test. Run all benchmarks.
Expected: list_ops 417ms → 200ms. for_iteration 305ms → 150ms. pattern_matching 6134ms → 3000ms.
```
