# Performance Gap #7: String Representation and Comparison Optimization

**Gap vs Native:** 2–5x on string-match-heavy code  
**Fix Difficulty:** Medium  
**Impact Breadth:** Pattern matching on strings, string equality checks, string-keyed maps  
**Affected Benchmarks:** pattern_matching (6134ms), string_ops (30ms), map_ops (59ms)

---

## Problem Statement

Strings in Coral are Rust `String` objects — heap-allocated UTF-8 byte buffers. String comparison is byte-by-byte O(n). Pattern matching on strings uses runtime string comparison: match arms with 10 string patterns cost 10 × O(n) comparisons in the worst case. String creation requires heap allocation even for string literals. No string deduplication or interning exists — the same literal string can exist in memory multiple times.

The pattern_matching benchmark is the second-slowest at 6134ms, dominated by string comparisons in match/if-elif chains. Map operations use string keys with repeated hashing and comparison.

---

## Action Items (Ordered)

### 1. Implement compile-time string interning for literals
- **What:** At compile time, build a global string table of all string literals in the program. Each unique literal gets a single global constant. References to the same literal share the same pointer. This enables pointer equality for literal-to-literal comparison (O(1) instead of O(n))
- **Where:** `src/codegen/mod.rs` — string literal emission (already has `string_nb_cache`; extend to share underlying data)
- **Complexity:** Low

### 2. Add runtime string interning pool
- **What:** For strings created at runtime that are used as map keys or match targets, intern them in a global hash set. Interned strings compare by pointer equality. Add `coral_string_intern(ptr, len) → interned_ptr` to runtime
- **Where:** `runtime/src/string_ops.rs` — new interning pool, `runtime/src/lib.rs` — global intern table
- **Complexity:** Medium

### 3. Implement pointer-equality fast path for string comparison
- **What:** Before doing byte-by-byte comparison, check if both string pointers are equal. If yes, strings are identical (O(1)). This benefits interned strings and cases where the same string object is compared against itself
- **Where:** `runtime/src/string_ops.rs` — `coral_string_equals`, or inline in codegen
- **Complexity:** Low

### 4. Implement length-first comparison optimization
- **What:** Before byte comparison, compare lengths. If lengths differ, strings are not equal (O(1)). This eliminates the majority of failed string comparisons without touching string data
- **Where:** `runtime/src/string_ops.rs` or inline in codegen
- **Complexity:** Low

### 5. Implement perfect hash dispatch for match expressions on strings
- **What:** For match expressions where all arms are string literal patterns, generate a minimal perfect hash function at compile time. Hash the match target once → direct index into jump table → O(1) dispatch instead of O(arms × string_length) sequential comparison
- **Where:** `src/codegen/mod.rs`, `src/codegen/match_adt.rs` — match expression emission
- **Complexity:** Hard

### 6. Implement length-based pre-filtering for match expressions
- **What:** Before comparing strings, group match arms by string length. First switch on `target.length()` (integer comparison), then only compare against arms of matching length. This reduces comparisons dramatically for varied-length patterns
- **Where:** `src/codegen/match_adt.rs`
- **Complexity:** Medium

### 7. Implement small string optimization (SSO)
- **What:** Strings ≤ 23 bytes stored inline in the string struct without heap allocation for the data buffer. The string header contains the data directly. For ≤ 7 bytes, encode entirely in the NaN-box payload (same as Gap #6 Item 7)
- **Where:** `runtime/src/lib.rs` — string value representation, `runtime/src/string_ops.rs`
- **Complexity:** Medium

### 8. Cache string hashes
- **What:** Compute and store the hash of a string the first time it's hashed (for map operations or interning). Subsequent hash requests return the cached value. Add a `cached_hash: u64` field to the string header (initially 0, computed on first use)
- **Where:** `runtime/src/lib.rs` — string value struct, `runtime/src/map_hash.rs`
- **Complexity:** Low

### 9. Implement SIMD string comparison
- **What:** For strings > 16 bytes, use SIMD instructions (SSE2/AVX2) for bulk comparison. Compare 16/32 bytes at a time instead of 1 byte. This gives ~4–8x speedup for long string comparisons
- **Where:** `runtime/src/string_ops.rs`
- **Complexity:** Medium

### 10. Implement first-byte/first-word discrimination for match
- **What:** Before full string comparison in match arms, check the first byte (or first 8 bytes loaded as u64). If first bytes differ, skip comparison. This is a lightweight pre-filter that catches most mismatches
- **Where:** `src/codegen/match_adt.rs`
- **Complexity:** Low

### 11. Mirror in self-hosted compiler
- **What:** Self-hosted codegen emits interned string references, length-first comparisons, and hash-based dispatch for match expressions
- **Where:** `self_hosted/codegen.coral`
- **Complexity:** Medium

---

## Implementation Plan

### Phase A: Compile-Time Interning and Fast Comparison (Items 1, 3, 4)

The codegen already has `string_nb_cache: HashMap<String, GlobalValue<'ctx>>` which deduplicates literal NaN-boxed values. Extend this to share the underlying string data:

```rust
fn emit_string_literal(&mut self, s: &str) -> IntValue<'ctx> {
    if let Some(&cached) = self.string_nb_cache.get(s) {
        // Return reference to existing interned global
        return self.builder.build_load(self.usize_type, cached, "str_cached")?.into_int_value();
    }
    
    // Create global string data
    let data_global = self.module.add_global(
        self.i8_type.array_type(s.len() as u32),
        Some(AddressSpace::default()),
        &format!("str_data_{}", self.string_pool.len()),
    );
    data_global.set_initializer(&self.context.const_string(s.as_bytes(), false));
    data_global.set_constant(true);  // Immutable — safe to compare by pointer
    
    // Create interned string header (global, never freed)
    let header_global = self.emit_interned_string_header(data_global, s.len());
    
    // NaN-box the pointer
    let nb = self.emit_ptr_to_nb(header_global);
    
    // Cache for future references to the same literal
    let global_ref = self.module.add_global(self.usize_type, None, &format!("str_nb_{}", self.string_pool.len()));
    global_ref.set_initializer(&nb);
    self.string_nb_cache.insert(s.to_string(), global_ref);
    
    nb
}
```

For comparison, add pointer-equality and length-first fast paths:

```rust
fn emit_string_equals(&mut self, a: IntValue<'ctx>, b: IntValue<'ctx>) -> IntValue<'ctx> {
    // Fast path 1: pointer equality (same interned string)
    let ptr_eq = self.builder.build_int_compare(IntPredicate::EQ, a, b, "ptr_eq")?;
    
    let slow_bb = self.context.append_basic_block(ctx.function, "cmp.slow");
    let true_bb = self.context.append_basic_block(ctx.function, "cmp.true");
    let merge_bb = self.context.append_basic_block(ctx.function, "cmp.merge");
    
    self.builder.build_conditional_branch(ptr_eq, true_bb, slow_bb)?;
    
    // Slow path: length check then byte comparison
    self.builder.position_at_end(slow_bb);
    let a_len = self.emit_inline_string_len_raw(a);
    let b_len = self.emit_inline_string_len_raw(b);
    let len_eq = self.builder.build_int_compare(IntPredicate::EQ, a_len, b_len, "len_eq")?;
    
    let bytes_bb = self.context.append_basic_block(ctx.function, "cmp.bytes");
    self.builder.build_conditional_branch(len_eq, bytes_bb, merge_bb)?;
    
    // Byte comparison only if lengths match
    self.builder.position_at_end(bytes_bb);
    let a_data = self.emit_string_data_ptr(a);
    let b_data = self.emit_string_data_ptr(b);
    let memcmp_result = self.builder.build_call(
        self.module.get_function("memcmp").unwrap_or_else(|| self.declare_memcmp()),
        &[a_data.into(), b_data.into(), a_len.into()],
        "memcmp"
    )?;
    let bytes_eq = self.builder.build_int_compare(
        IntPredicate::EQ, memcmp_result.try_as_basic_value().left().unwrap().into_int_value(),
        self.usize_type.const_zero(), "bytes_eq"
    )?;
    self.builder.build_conditional_branch(bytes_eq, true_bb, merge_bb)?;
    
    // Merge
    self.builder.position_at_end(true_bb);
    self.builder.build_unconditional_branch(merge_bb)?;
    
    self.builder.position_at_end(merge_bb);
    let phi = self.builder.build_phi(self.bool_type, "eq_result")?;
    phi.add_incoming(&[
        (&self.bool_type.const_int(1, false), true_bb),
        (&self.bool_type.const_zero(), slow_bb),
        (&self.bool_type.const_zero(), merge_bb),  // fallthrough from failed length check
    ]);
    self.wrap_bool(phi.as_basic_value().into_int_value())
}
```

### Phase B: Match Expression Optimization (Items 5, 6, 10)

For match expressions with string literal arms, generate a tiered dispatch:

**Tier 1: Length switch**
```llvm
%target_len = call i64 @inline_string_len(%target)
switch i64 %target_len, label %default [
  i64 4, label %len4
  i64 5, label %len5
  i64 7, label %len7
]
```

**Tier 2: First-word discrimination (within each length group)**
```llvm
len4:
  %first_word = load i32, ptr %target_data  ; Load first 4 bytes as i32
  switch i32 %first_word, label %default [
    i32 0x7A7A6966, label %check_fizz   ; "fizz" as little-endian i32
    i32 0x7A7A7562, label %check_buzz   ; "buzz" as little-endian i32
  ]
```

**Tier 3: Full comparison (only for collisions)**
```llvm
check_fizz:
  ; Already matched on length (4) and first word — string must be "fizz"
  br label %arm_fizz
```

For many patterns, Tiers 1+2 eliminate all byte-by-byte comparison entirely.

**Perfect hash dispatch** (for large match expressions with >8 string arms):
```rust
fn emit_perfect_hash_match(&mut self, target: IntValue<'ctx>, arms: &[(String, BasicBlock)]) {
    // Generate PHF (perfect hash function) at compile time
    let phf = phf_generator::generate_hash(&arms.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>());
    
    // Emit: hash target string → index → jump table
    let hash = self.emit_phf_hash(target, &phf);
    let table_size = phf.map.len();
    
    // Switch on hash result
    let mut cases: Vec<(IntValue, BasicBlock)> = Vec::new();
    for (i, (key, block)) in arms.iter().enumerate() {
        // Each case: verify key matches (hash collision guard), then branch to arm
        let verify_bb = self.context.append_basic_block(ctx.function, &format!("phf.{}", i));
        cases.push((self.usize_type.const_int(i as u64, false), verify_bb));
    }
    
    self.builder.build_switch(hash, default_bb, &cases)?;
}
```

### Phase C: Runtime String Interning and Hash Caching (Items 2, 8)

In `runtime/src/string_ops.rs`:
```rust
use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    static ref INTERN_POOL: RwLock<HashSet<Box<str>>> = RwLock::new(HashSet::new());
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_intern(ptr: *const u8, len: usize) -> ValueHandle {
    let s = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) };
    
    // Check if already interned
    let pool = INTERN_POOL.read().unwrap();
    if let Some(existing) = pool.get(s) {
        return existing.as_ptr() as ValueHandle;
    }
    drop(pool);
    
    // Intern new string
    let mut pool = INTERN_POOL.write().unwrap();
    let boxed: Box<str> = s.into();
    let ptr = boxed.as_ptr() as ValueHandle;
    pool.insert(boxed);
    ptr
}
```

For hash caching, extend the string value header:
```rust
#[repr(C)]
pub struct CoralString {
    header: ValueHeader,
    length: u64,
    cached_hash: AtomicU64,  // 0 = not computed yet
    data: *const u8,
}

impl CoralString {
    pub fn hash(&self) -> u64 {
        let cached = self.cached_hash.load(Ordering::Relaxed);
        if cached != 0 { return cached; }
        
        let h = fxhash::hash64(self.as_bytes());
        let h = if h == 0 { 1 } else { h };  // Reserve 0 for "not computed"
        self.cached_hash.store(h, Ordering::Relaxed);
        h
    }
}
```

### Phase D: Small String Optimization (Item 7)

In `runtime/src/lib.rs`:
```rust
#[repr(C)]
pub struct CoralString {
    header: ValueHeader,
    // Discriminated union:
    // If length <= 23: data stored inline starting at offset 24
    // If length > 23: heap pointer at offset 24
    length: u64,
    cached_hash: u64,
    data: StringData,
}

#[repr(C)]
union StringData {
    inline_data: [u8; 24],     // For SSO strings
    heap_ptr: *const u8,       // For long strings
}

impl CoralString {
    pub fn is_inline(&self) -> bool {
        self.length <= 23
    }
    
    pub fn as_bytes(&self) -> &[u8] {
        if self.is_inline() {
            &unsafe { self.data.inline_data }[..self.length as usize]
        } else {
            unsafe { std::slice::from_raw_parts(self.data.heap_ptr, self.length as usize) }
        }
    }
}
```

### Phase E: SIMD Comparison (Item 9)

For long string comparison, use platform intrinsics:
```rust
#[cfg(target_arch = "x86_64")]
unsafe fn simd_memcmp(a: *const u8, b: *const u8, len: usize) -> bool {
    use std::arch::x86_64::*;
    
    let mut offset = 0;
    // Compare 32 bytes at a time (AVX2)
    while offset + 32 <= len {
        let va = _mm256_loadu_si256(a.add(offset) as *const __m256i);
        let vb = _mm256_loadu_si256(b.add(offset) as *const __m256i);
        let cmp = _mm256_cmpeq_epi8(va, vb);
        if _mm256_movemask_epi8(cmp) != -1i32 as u32 {
            return false;
        }
        offset += 32;
    }
    
    // Handle remaining bytes
    a.add(offset).eq(b.add(offset), len - offset)
}
```

### Phase F: Self-Hosted Mirror (Item 11)

In `self_hosted/codegen.coral`, emit string comparison IR that includes pointer equality check and length check before calling memcmp. For match expressions, emit length-based switch dispatch.

---

## Implementation Prompt

```
Implement string representation and comparison optimizations in the Coral compiler and runtime.

CONTEXT:
- Strings are heap-allocated Rust String objects
- Comparison is byte-by-byte O(n) with no shortcuts
- pattern_matching benchmark is 6134ms — dominated by string comparison in match/elif
- codegen already has string_nb_cache for NaN-boxed literal deduplication
- No interning, no hash caching, no small string optimization exists

CHANGES REQUIRED:

1. src/codegen/mod.rs — Compile-time string interning:
   - String literals sharing the same value point to the same global constant
   - Mark as constant (no refcount needed)
   - Enable pointer equality for literal-to-literal comparison

2. src/codegen/mod.rs or builtins.rs — Inline string comparison:
   - Emit pointer equality check first (O(1) for interned strings)
   - Emit length comparison second (O(1) rejection for different-length strings)
   - Emit memcmp only when pointer != and length ==

3. src/codegen/match_adt.rs — Optimized string match dispatch:
   - Group arms by string length → switch on length first
   - Within each length group: compare first 8 bytes as u64 (word discrimination)
   - For >8 arms: generate perfect hash dispatch

4. runtime/src/string_ops.rs — Runtime string interning:
   - Add coral_string_intern(ptr, len) → interned handle
   - Thread-safe intern pool (RwLock<HashSet<Box<str>>>)
   
5. runtime/src/lib.rs — Hash caching:
   - Add cached_hash: AtomicU64 to string header
   - Compute on first use, return cached thereafter

6. runtime/src/lib.rs — Small string optimization:
   - Strings ≤23 bytes inline in struct (no separate heap allocation for data)
   - Check is_inline() flag for access

7. self_hosted/codegen.coral — Mirror interning references and length-first comparison

TEST: cargo test. Run pattern_matching benchmark specifically.
Expected: pattern_matching 6134ms → 1000-2000ms.
```
