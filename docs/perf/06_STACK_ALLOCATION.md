# Performance Gap #6: Stack Allocation for Value Types

**Gap vs Native:** 1.5–3x on code with many short-lived temporaries  
**Fix Difficulty:** Hard  
**Impact Breadth:** String building, list transformations, closure creation, any function creating temporaries  
**Affected Benchmarks:** string_ops (30ms), list_ops (417ms), closures (67ms), store_ops (6290ms), pattern_matching (6134ms)

---

## Problem Statement

Every composite value (string, list, map, store instance, closure) is heap-allocated via `Box::new()` in the runtime, even if it's a local variable that never escapes the function. Heap allocation costs ~50–100ns per allocation (`malloc` + bookkeeping). Deallocation costs ~20–50ns (RC decrement + conditional `free`). A function creating a temporary list, transforming it, and returning a scalar pays full allocation cost for a value that could live on the stack.

In Rust, values are stack-allocated by default; heap allocation is explicit (`Box::new`, `Vec::new`). The compiler's escape analysis and borrow checker ensure stack references are safe. In Zig, arena allocators provide zero-overhead allocation patterns.

Coral already has `FLAG_STACK (0b1000_0000)` in the value header flags, which causes retain/release to skip refcounting. But this flag is never set by codegen — all values go through heap allocation.

---

## Action Items (Ordered)

### 1. Implement function-level escape analysis
- **What:** For each function, determine which local heap allocations (string concatenations, list literals, map literals, closures, store instances) never escape the function scope. A value escapes if it is: returned, stored in a mutable variable visible outside the function, passed to a function that captures it, stored in a container that escapes
- **Where:** `src/semantic.rs` — new escape analysis pass (shared with Gap #2)
- **Complexity:** Hard

### 2. Emit alloca for non-escaping strings
- **What:** When a string is created within a function and does not escape, allocate the string header + data buffer on the stack via `alloca`. Set `FLAG_STACK` in the header so retain/release are no-ops. String data for known-size strings (literals, fixed concatenations) can be fully stack-allocated
- **Where:** `src/codegen/mod.rs`, `src/codegen/builtins.rs` — string construction
- **Complexity:** Medium

### 3. Emit alloca for non-escaping lists
- **What:** When a list with a known max size is created and does not escape, allocate the list header + element buffer on the stack. For lists created as `[]` and then pushed to N times (where N is statically determinable), allocate `N * 8` bytes on the stack
- **Where:** `src/codegen/mod.rs`, `src/codegen/builtins.rs` — list construction
- **Complexity:** Medium

### 4. Emit alloca for non-escaping closures
- **What:** Closures capture environment variables into a heap-allocated struct. When the closure does not escape (used only for local higher-order calls like `list.map(closure)` where the list operation is inline), allocate the closure environment on the stack
- **Where:** `src/codegen/closures.rs`
- **Complexity:** Hard

### 5. Emit alloca for non-escaping store instances
- **What:** Store instances created for temporary computation within a function can be stack-allocated if they don't escape. The store struct (field vector / LLVM struct if specialized per Gap #4) lives on the stack with FLAG_STACK set
- **Where:** `src/codegen/store_actor.rs`
- **Complexity:** Medium

### 6. Implement stack frame size budgeting
- **What:** Prevent stack overflow from excessive stack allocation. Set a per-function stack budget (e.g., 64KB). Track cumulative stack allocation size during codegen. When budget is exceeded, fall back to heap allocation for remaining values. Emit stack probe for functions exceeding 4KB
- **Where:** `src/codegen/mod.rs` — new `StackBudget` tracker in `FunctionContext`
- **Complexity:** Medium

### 7. Implement stack-allocated small string optimization
- **What:** For strings ≤ 23 bytes (fits in 3 registers), store the string data inline in the NaN-box payload area or in a small stack buffer. No heap allocation needed. This covers most identifier strings, short messages, and number-to-string conversions
- **Where:** `runtime/src/lib.rs` (small string representation), `src/codegen/mod.rs` (inline emission)
- **Complexity:** Medium

### 8. Implement stack-slot reuse
- **What:** When multiple non-overlapping-lifetime stack allocations exist in the same function, reuse the same stack slot. E.g., if a temporary list is created in an if-branch and another in the else-branch, they can share the same alloca. LLVM's mem2reg pass handles some of this, but explicit slot merging helps
- **Where:** `src/codegen/mod.rs`
- **Complexity:** Medium

### 9. Add stack allocation for known-size map literals
- **What:** `map("x" is 1, "y" is 2)` with fixed keys can be represented as a stack-allocated struct: `{ key0: "x", val0: 1, key1: "y", val1: 2, len: 2 }`. Lookup by known key → direct field access at compile time
- **Where:** `src/codegen/mod.rs` — map literal emission
- **Complexity:** Hard

### 10. Mirror in self-hosted compiler
- **What:** Self-hosted codegen emits alloca instructions for non-escaping values and sets FLAG_STACK
- **Where:** `self_hosted/codegen.coral`
- **Complexity:** Medium

---

## Implementation Plan

### Phase A: Escape Analysis Foundation (Item 1)

This shares infrastructure with Gap #2 (Escape Analysis and RC Optimization). The escape analysis pass produces:

```rust
pub struct EscapeInfo {
    /// Variables whose values never escape function scope
    pub non_escaping: HashSet<(String, String)>,  // (fn_name, var_name)
    /// Variables eligible for stack allocation (non-escaping + known size)
    pub stack_eligible: HashSet<(String, String)>,
    /// Estimated stack size needed per variable
    pub stack_sizes: HashMap<(String, String), usize>,
}
```

Escape rules:
1. **Returns:** If variable is returned or part of return value → escapes
2. **Captured:** If variable is captured by a closure that escapes → escapes
3. **Stored:** If variable is stored in a container field that escapes → escapes
4. **Passed to unknown:** If variable is passed to a function whose body is not visible (FFI, indirect call) → conservatively escapes
5. **Aliased:** If variable is assigned to another variable that escapes → escapes

### Phase B: Stack-Allocated Strings (Items 2, 7)

For non-escaping string concatenation:
```rust
fn emit_stack_string(&mut self, ctx: &mut FunctionContext, parts: &[StringPart]) -> IntValue<'ctx> {
    // Calculate total size at compile time (if all parts are literals/known-size)
    let total_size = parts.iter().map(|p| p.known_size()).sum::<Option<usize>>();
    
    if let Some(size) = total_size {
        if size <= ctx.stack_budget.remaining() {
            // Allocate header + data on stack
            let header_alloca = self.builder.build_alloca(self.string_struct_type, "sstr")?;
            let data_alloca = self.builder.build_array_alloca(
                self.i8_type, self.usize_type.const_int(size as u64, false), "sdata")?;
            
            // Initialize header with FLAG_STACK
            self.init_stack_string_header(header_alloca, size, data_alloca)?;
            
            // Copy string parts into data buffer
            let mut offset = 0;
            for part in parts {
                self.emit_memcpy_part(data_alloca, offset, part)?;
                offset += part.size();
            }
            
            ctx.stack_budget.consume(size + STRING_HEADER_SIZE);
            return self.emit_ptr_to_nb(header_alloca);
        }
    }
    
    // Fall back to heap allocation
    self.emit_heap_string(parts)
}
```

For small string optimization (≤23 bytes):
```rust
fn emit_small_string_inline(&mut self, data: &[u8]) -> IntValue<'ctx> {
    if data.len() <= 7 {
        // Encode directly in NaN-box payload (48 bits = 6 bytes + length byte)
        let mut bits: u64 = SMALL_STRING_TAG;
        bits |= (data.len() as u64) << 40;
        for (i, &byte) in data.iter().enumerate() {
            bits |= (byte as u64) << (i * 8);
        }
        self.usize_type.const_int(bits, false)
    } else {
        // Stack allocate for 8-23 byte strings
        self.emit_stack_string_fixed(data)
    }
}
```

### Phase C: Stack-Allocated Lists and Closures (Items 3–4)

For non-escaping lists with known max size:
```rust
fn emit_stack_list(&mut self, ctx: &mut FunctionContext, max_elements: usize) -> IntValue<'ctx> {
    let data_size = max_elements * 8; // 8 bytes per NaN-boxed element
    let total_size = LIST_HEADER_SIZE + data_size;
    
    if total_size <= ctx.stack_budget.remaining() {
        let header = self.builder.build_alloca(self.list_struct_type, "slist")?;
        let data = self.builder.build_array_alloca(
            self.usize_type, self.usize_type.const_int(max_elements as u64, false), "sdata")?;
        
        // Init: length=0, capacity=max_elements, data=data_ptr, flags=FLAG_STACK
        self.init_stack_list_header(header, 0, max_elements, data)?;
        
        ctx.stack_budget.consume(total_size);
        return self.emit_ptr_to_nb(header);
    }
    
    self.emit_heap_list(max_elements)
}
```

For non-escaping closures:
```rust
fn emit_stack_closure_env(&mut self, ctx: &mut FunctionContext, captures: &[String]) -> PointerValue<'ctx> {
    let env_size = captures.len() * 8 + CLOSURE_HEADER_SIZE;
    
    if env_size <= ctx.stack_budget.remaining() {
        // Alloca for closure environment struct
        let env = self.builder.build_alloca(
            self.context.struct_type(
                &vec![self.usize_type.into(); captures.len() + 1], // header + captured values
                false
            ),
            "senv"
        )?;
        
        // Set FLAG_STACK to skip refcounting
        self.set_flag_stack(env)?;
        
        ctx.stack_budget.consume(env_size);
        return env;
    }
    
    self.emit_heap_closure_env(captures)
}
```

### Phase D: Stack Budget and Slot Reuse (Items 6, 8)

```rust
pub struct StackBudget {
    max_bytes: usize,    // Default: 65536 (64KB)
    used_bytes: usize,
    slots: Vec<StackSlot>,
}

pub struct StackSlot {
    alloca: PointerValue<'ctx>,
    size: usize,
    live_range: (BasicBlock, BasicBlock),
    in_use: bool,
}

impl StackBudget {
    pub fn remaining(&self) -> usize {
        self.max_bytes.saturating_sub(self.used_bytes)
    }
    
    pub fn consume(&mut self, bytes: usize) {
        self.used_bytes += bytes;
    }
    
    pub fn try_reuse(&mut self, size: usize, current_block: BasicBlock) -> Option<PointerValue<'ctx>> {
        // Find a slot that's: same size or larger, no longer in use
        for slot in &mut self.slots {
            if !slot.in_use && slot.size >= size {
                slot.in_use = true;
                return Some(slot.alloca);
            }
        }
        None
    }
}
```

### Phase E: Store and Map Stack Allocation (Items 5, 9)

For non-escaping stores:
```rust
fn emit_stack_store(&mut self, ctx: &mut FunctionContext, store_name: &str, field_count: usize) -> IntValue<'ctx> {
    let store_size = STORE_HEADER_SIZE + field_count * 8;
    
    if store_size <= ctx.stack_budget.remaining() {
        let store = self.builder.build_alloca(
            self.store_struct_types.get(store_name).unwrap_or(&self.generic_store_type),
            "sstore"
        )?;
        self.set_flag_stack(store)?;
        ctx.stack_budget.consume(store_size);
        return self.emit_ptr_to_nb(store);
    }
    
    self.emit_heap_store(store_name, field_count)
}
```

### Phase F: Self-Hosted Mirror (Item 10)

In `self_hosted/codegen.coral`, when emitting allocation instructions, check if the variable is in the stack-eligible set and emit `alloca` IR text instead of runtime constructor calls.

---

## Implementation Prompt

```
Implement stack allocation for non-escaping value types in the Coral compiler.

CONTEXT:
- All composite values (string, list, map, closure, store) are heap-allocated via runtime FFI
- FLAG_STACK (0b1000_0000) exists in ValueHeader flags but is never set by codegen
- Heap alloc costs ~50-100ns, stack alloc is essentially free
- Escape analysis from Gap #2 provides non_escaping and stack_eligible sets

CHANGES REQUIRED:

1. src/codegen/mod.rs — Stack budget tracker:
   - Add StackBudget to FunctionContext with 64KB default limit
   - Track cumulative allocation, prevent overflow
   - Add slot reuse for non-overlapping lifetimes

2. src/codegen/mod.rs — Stack string allocation:
   - For non-escaping string concats with known size: alloca for header + data
   - Set FLAG_STACK in header, memcpy parts into stack buffer
   - For strings ≤7 bytes: inline in NaN-box payload (small string opt)
   - Fall back to heap when budget exceeded

3. src/codegen/builtins.rs — Stack list allocation:
   - For non-escaping lists with known max size: alloca for header + data array
   - Init with length=0, capacity=max, FLAG_STACK
   - list.push becomes: store at data[length], increment length (inline, no FFI)

4. src/codegen/closures.rs — Stack closure environment:
   - For non-escaping closures: alloca for env struct
   - Set FLAG_STACK, store captured values directly
   - Skip retain for captured values (they outlive the closure on the stack)

5. src/codegen/store_actor.rs — Stack store instances:
   - For non-escaping stores: alloca for store struct
   - Set FLAG_STACK, init fields directly

6. self_hosted/codegen.coral — Emit alloca for non-escaping values

TEST: cargo test. Run benchmarks focusing on closures, string_ops.
Expected: closures 67ms → 35ms. string_ops 30ms → 15ms.
```
