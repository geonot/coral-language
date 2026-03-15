# Performance Gap #2: Escape Analysis and Reference Counting Optimization

**Gap vs Native:** 2–4x on allocation-heavy code  
**Fix Difficulty:** Hard  
**Impact Breadth:** String manipulation, list transformations, closure-heavy functional patterns  
**Affected Benchmarks:** list_ops (417ms), string_ops (30ms), closures (67ms), for_iteration (305ms), store_ops (6290ms)

---

## Problem Statement

Every heap object (string, list, map, closure, store instance) is reference-counted with atomic operations. Every variable assignment, function argument pass, and return increments the RC. Every scope exit decrements it. Atomic CAS operations are 5–20x slower than regular memory writes. Short-lived temporaries that never escape the current scope still pay full RC cost.

Currently, `coral_value_retain` does `fetch_add(1, Relaxed)` and `coral_value_release` does a CAS loop with `compare_exchange_weak` plus cycle detector notification on every container release. The cycle detector acquires a global mutex. Even a simple `"hello" + " " + "world"` creates 2 intermediate strings, each with heap alloc → RC increment → RC decrement → free.

Rust eliminates this with ownership + borrowing. Zig uses manual memory with arenas. Coral needs escape analysis to identify which allocations never escape the current function, move semantics for single-use values, and region-based allocation for short-lived groups.

---

## Action Items (Ordered)

### 1. Implement non-atomic fast path for thread-local values
- **What:** If a value's `thread_id` matches the current thread and RC > 1, use non-atomic increment/decrement. Only use atomic ops for values shared across threads (frozen/actors)
- **Where:** `runtime/src/rc_ops.rs` — `coral_value_retain`, `coral_value_release`
- **Why:** Eliminates atomic overhead for ~95% of retain/release calls in single-threaded code
- **Complexity:** Medium

### 2. Implement retain/release elision for temporaries
- **What:** In codegen, track the lifetime of each value. When a value is created and consumed within the same basic block with no intervening function calls that could capture it, skip the retain/release pair entirely
- **Where:** `src/codegen/mod.rs` — expression emission, scope management
- **Complexity:** Medium

### 3. Implement basic escape analysis in semantic pass
- **What:** For each function, analyze which local allocations (string concatenations, list literals, map literals, closures) escape the function via: return value, stored in a captured variable, passed to a function that captures it. Mark non-escaping allocations
- **Where:** `src/semantic.rs` — new `EscapeAnalysis` pass after type inference
- **Complexity:** Hard

### 4. Stack-allocate non-escaping values
- **What:** For allocations marked non-escaping by escape analysis, emit `alloca` on the stack instead of calling `coral_string_new` / `coral_list_new` / etc. Set the `FLAG_STACK` bit to skip refcounting
- **Where:** `src/codegen/mod.rs` — string/list/map construction emission
- **Complexity:** Hard

### 5. Implement move semantics for single-use values
- **What:** When a value is used exactly once after creation (assigned, then passed to a function or returned), transfer ownership without incrementing RC. The creator skips retain; the consumer skips release
- **Where:** `src/codegen/mod.rs` — variable load/store, function argument passing
- **Complexity:** Medium

### 6. Batch release at scope exit
- **What:** Instead of emitting individual `coral_value_release` calls for each variable at scope exit, collect all releasable variables and emit a single `coral_batch_release(ptrs, count)` call that processes them together, amortizing function call overhead
- **Where:** `runtime/src/rc_ops.rs` (new function), `src/codegen/mod.rs` (scope exit emission)
- **Complexity:** Low

### 7. Defer cycle detection to batch checkpoints
- **What:** Instead of calling `cycle_detector::possible_root` on every release, accumulate potential roots in a thread-local buffer and process them in batches (every N releases or at scope boundaries)
- **Where:** `runtime/src/rc_ops.rs`, `runtime/src/cycle_detector.rs`
- **Complexity:** Medium

### 8. Implement region/arena allocator for function-scoped allocations
- **What:** For functions with many short-lived allocations (string building, list transformations), allocate from a bump allocator that frees everything at function exit. No individual release calls needed
- **Where:** `runtime/src/allocator.rs` (extend existing), `src/codegen/mod.rs` (arena entry/exit)
- **Complexity:** Hard

### 9. Mirror in self-hosted compiler
- **What:** Self-hosted codegen should emit the same retain/release elision patterns and stack allocation markers
- **Where:** `self_hosted/codegen.coral`
- **Complexity:** Medium

---

## Implementation Plan

### Phase A: Non-Atomic Fast Path (Item 1)

In `runtime/src/rc_ops.rs`, modify `coral_value_retain`:
```rust
pub unsafe extern "C" fn coral_value_retain(value: ValueHandle) {
    if value.is_null() { return; }
    let v = unsafe { &*value };
    if (v.flags & FLAG_STACK) != 0 { return; }
    
    // Fast path: thread-local, non-frozen
    if (v.flags & FLAG_FROZEN) == 0 {
        // Non-atomic increment — safe because only this thread accesses it
        let rc = v.refcount.load(Ordering::Relaxed);
        v.refcount.store(rc + 1, Ordering::Relaxed);
        return;
    }
    
    // Slow path: atomic for shared/frozen values
    v.refcount.fetch_add(1, Ordering::Acquire);
}
```

Similarly for `coral_value_release` — use non-atomic decrement when the value is not frozen/shared. This alone should give 2–3x improvement on retain/release throughput.

### Phase B: Retain/Release Elision (Items 2, 5)

In codegen, add a `ValueLifetime` tracker to `FunctionContext`:
```rust
struct ValueLifetime {
    creation_block: BasicBlock,
    use_count: usize,
    escapes: bool,
}
```

During expression emission:
- When creating a heap value (string concat, list literal, etc.), record it in the lifetime tracker
- When the value is used (passed to function, stored, returned), increment use_count
- If use_count == 1 and creation_block == current_block and !escapes:
  - Skip the retain at creation
  - Skip the release at scope exit
  - Emit a "transfer" instead of "copy" at the use site

For move semantics: when a variable is loaded and the load is the last use (no further references), emit the load without retain, and don't release at scope exit. Mark the variable as "moved".

### Phase C: Escape Analysis (Items 3–4)

Add a new pass in `src/semantic.rs` after type inference:

```rust
pub struct EscapeInfo {
    pub non_escaping: HashSet<(String, String)>, // (fn_name, var_name)
    pub stack_eligible: HashSet<(String, String)>,
}

fn analyze_escapes(model: &SemanticModel, ast: &[Statement]) -> EscapeInfo {
    for function in ast.functions() {
        for local in function.locals() {
            let escapes = check_escape(local, function);
            // Escapes if: returned, stored in outer scope var, passed to 
            // capturing closure, stored in container that escapes
            if !escapes {
                info.non_escaping.insert((fn_name, local_name));
                if is_known_size(local.type) {
                    info.stack_eligible.insert((fn_name, local_name));
                }
            }
        }
    }
}
```

In codegen, when emitting a string/list/map construction for a stack-eligible variable:
- Allocate the object header + data on the stack via `alloca`
- Set `FLAG_STACK` in the header flags
- Skip all retain/release for this value
- At function exit, the stack frame cleanup handles deallocation automatically

### Phase D: Batch Operations (Items 6–7)

Add to runtime:
```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_batch_release(ptrs: *const ValueHandle, count: usize) {
    let slice = std::slice::from_raw_parts(ptrs, count);
    let mut roots = Vec::new();
    for &ptr in slice {
        if ptr.is_null() { continue; }
        let v = &*ptr;
        if (v.flags & FLAG_STACK) != 0 { continue; }
        let rc = v.refcount.load(Ordering::Relaxed);
        if rc <= 1 {
            deallocate(ptr);
        } else {
            v.refcount.store(rc - 1, Ordering::Relaxed);
            roots.push(ptr);
        }
    }
    if !roots.is_empty() {
        cycle_detector::batch_possible_roots(&roots);
    }
}
```

In codegen, at scope exit, collect all locals needing release into an array and emit one `coral_batch_release` call.

For cycle detection deferral: add a thread-local `DEFERRED_ROOTS: Vec<ValueHandle>` that accumulates roots. Flush at scope boundaries or every 256 releases.

### Phase E: Arena Allocator (Item 8)

Extend `runtime/src/allocator.rs`:
```rust
pub struct Arena {
    chunks: Vec<Vec<u8>>,
    current: *mut u8,
    remaining: usize,
}

impl Arena {
    pub fn alloc(&mut self, size: usize) -> *mut u8 { /* bump pointer */ }
    pub fn reset(&mut self) { /* reuse all chunks */ }
}

thread_local! {
    static FUNCTION_ARENA: RefCell<Arena> = RefCell::new(Arena::new(64 * 1024));
}
```

In codegen, for functions identified as "allocation-heavy" (>3 heap allocations visible), emit arena enter/exit:
```llvm
call void @coral_arena_enter()
; ... function body with arena-allocated temporaries ...
call void @coral_arena_exit()
```

### Phase F: Self-Hosted Mirror (Item 9)

In `self_hosted/codegen.coral`, add the same retain/release elision logic: track which values are single-use, skip emit of `coral_value_retain`/`coral_value_release` calls for temporaries consumed within the same expression.

---

## Implementation Prompt

```
Implement escape analysis and reference counting optimizations in the Coral compiler and runtime to reduce allocation overhead.

CONTEXT:
- Coral uses atomic reference counting for all heap values (strings, lists, maps, closures)
- runtime/src/rc_ops.rs: coral_value_retain uses fetch_add(1, Relaxed), coral_value_release uses CAS loop
- Every scope exit emits individual coral_value_release calls for each local
- Cycle detector (runtime/src/cycle_detector.rs) is called on every container release with global mutex
- FLAG_STACK (0b1000_0000) already exists but is not used by codegen

CHANGES REQUIRED:

1. runtime/src/rc_ops.rs — Non-atomic fast path:
   - In coral_value_retain: if !(flags & FLAG_FROZEN), use non-atomic load+store instead of fetch_add
   - In coral_value_release: if !(flags & FLAG_FROZEN), use non-atomic load+store instead of CAS loop
   - Add coral_batch_release(ptrs: *const ValueHandle, count: usize) that processes multiple releases
   
2. runtime/src/cycle_detector.rs — Deferred root collection:
   - Add thread-local DEFERRED_ROOTS buffer
   - possible_root() pushes to buffer instead of acquiring global mutex
   - Flush buffer every 256 entries or on explicit coral_cycle_flush() call

3. src/semantic.rs — Escape analysis:
   - Add EscapeInfo struct with non_escaping and stack_eligible sets
   - After type inference, analyze each function's locals for escape
   - A local escapes if: returned from function, captured by closure, stored in container that escapes, passed to unknown function
   - Attach EscapeInfo to SemanticModel

4. src/codegen/mod.rs — Retain/release elision:
   - Track value lifetimes in FunctionContext
   - When value created and consumed in same basic block with use_count==1: skip retain+release
   - At scope exit, use coral_batch_release instead of individual calls
   - For non-escaping values (from EscapeInfo), set FLAG_STACK and use alloca

5. self_hosted/codegen.coral — Mirror elision:
   - Skip retain/release emission for single-use temporaries

TEST: cargo test. Run benchmarks focusing on list_ops, closures, for_iteration.
Expected: list_ops should drop from 417ms toward 200ms. closures from 67ms toward 30ms.
```
