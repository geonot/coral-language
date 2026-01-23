# Coral Self-Hosting Runtime Specification

_Created: 2026-01-06_

## 1. Overview

This document specifies the requirements and architecture for rewriting the Coral runtime in Coral itself. The self-hosted runtime will replace the current Rust implementation while maintaining binary compatibility with existing compiled programs.

### 1.1 Goals

1. **Dogfooding**: Prove Coral is suitable for systems programming
2. **Portability**: Easier to port runtime to new platforms
3. **Transparency**: Runtime code is readable Coral, not foreign Rust
4. **Performance**: Opportunity for Coral-specific optimizations

### 1.2 Non-Goals (Initial Version)

- Matching Rust runtime performance initially
- Supporting all platforms (Linux x86_64 first)
- Exotic memory allocators

---

## 2. Prerequisites

### 2.1 Required Language Features

| Feature | Status | Blocking |
|---------|--------|----------|
| Low-level memory access | ✅ `std.runtime.memory` | Core runtime |
| Inline assembly | ✅ Behind flag | Atomic ops, syscalls |
| Extern function declarations | ✅ Working | libc FFI |
| Pointer arithmetic | ✅ `coral_ptr_add` | Data structures |
| Raw byte arrays | ✅ Working | Memory buffers |
| Bitwise operations | ✅ `std.bit` | Tag manipulation |
| Unsafe blocks | ✅ Syntax exists | Low-level code |

### 2.2 Required External Dependencies

```
libc functions:
  - malloc / free / realloc
  - memcpy / memset / memmove
  - pthread_* (for actor threads)
  - write / read / open / close (syscalls)
```

---

## 3. Architecture

### 3.1 Runtime Components

```
┌─────────────────────────────────────────────────────────────────┐
│                         Coral Runtime                            │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │   Values    │  │   Memory    │  │   Actors    │             │
│  │  (tagging,  │  │  (alloc,    │  │  (spawn,    │             │
│  │   refcount) │  │   pools)    │  │   mailbox)  │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │    Lists    │  │    Maps     │  │   Strings   │             │
│  │  (dynamic   │  │  (hash      │  │  (inline,   │             │
│  │   arrays)   │  │   tables)   │  │   heap)     │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │  Closures   │  │     I/O     │  │   Metrics   │             │
│  │  (env,      │  │  (files,    │  │  (counters, │             │
│  │   invoke)   │  │   console)  │  │   tracing)  │             │
│  └─────────────┘  └─────────────┘  └─────────────┘             │
├─────────────────────────────────────────────────────────────────┤
│                    Platform Abstraction Layer                    │
├─────────────────────────────────────────────────────────────────┤
│                         libc / syscalls                          │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 Module Structure

```
coral-runtime/
├── src/
│   ├── lib.coral            # Main exports
│   ├── value/
│   │   ├── mod.coral        # Value type and core ops
│   │   ├── tag.coral        # Tag constants and manipulation
│   │   └── refcount.coral   # Reference counting
│   ├── memory/
│   │   ├── mod.coral        # Memory management
│   │   ├── pool.coral       # Value pools
│   │   └── arena.coral      # Arena allocator
│   ├── collections/
│   │   ├── list.coral       # Dynamic array implementation
│   │   ├── map.coral        # Hash map implementation
│   │   └── string.coral     # String implementation
│   ├── actor/
│   │   ├── mod.coral        # Actor system
│   │   ├── mailbox.coral    # Message queues
│   │   ├── scheduler.coral  # M:N scheduler
│   │   └── worker.coral     # Worker threads
│   ├── io/
│   │   ├── mod.coral        # I/O abstraction
│   │   ├── file.coral       # File operations
│   │   └── console.coral    # Console I/O
│   └── platform/
│       ├── linux.coral      # Linux-specific
│       └── macos.coral      # macOS-specific
└── tests/
    └── ...
```

---

## 4. Data Structures

### 4.1 Value Representation

```coral
// 32 bytes per Value (cache-line friendly)
store Value
    tag: u8           // Type discriminant
    flags: u8         // Inline, frozen, etc.
    reserved: u16     // Alignment padding
    refcount: u64     // Reference count (atomic)
    payload: Payload  // 16 bytes of data

type Payload
    | Number(f64)
    | Bool(u8)
    | Ptr(usize)
    | Inline([u8; 16])

// Tag values
TAG_NUMBER   is 0
TAG_BOOL     is 1
TAG_STRING   is 2
TAG_LIST     is 3
TAG_MAP      is 4
TAG_STORE    is 5
TAG_ACTOR    is 6
TAG_UNIT     is 7
TAG_CLOSURE  is 8
TAG_BYTES    is 9
TAG_TAGGED   is 10

// Flag values
FLAG_INLINE   is 0b0001  // String stored inline
FLAG_FROZEN   is 0b0010  // Immutable (for actor messages)
FLAG_LIST_ITER is 0b0100 // List iteration in progress
FLAG_MAP_ITER  is 0b1000 // Map iteration in progress
```

### 4.2 List Implementation

```coral
store ListData
    capacity: usize
    length: usize
    items: usize      // Pointer to Value* array

*make_list(hint: u8)
    initial_cap is hint > 0 ? hint * 2 ! 8
    data is alloc(size_of_list_data())
    items is alloc(initial_cap * ptr_size())
    data.capacity is initial_cap
    data.length is 0
    data.items is items
    wrap_list(data)

*list_push(list, value)
    data is unwrap_list(list)
    if data.length >= data.capacity
        grow_list(data)
    store_ptr(data.items + data.length * ptr_size(), value)
    retain(value)
    data.length is data.length + 1

*list_get(list, index)
    data is unwrap_list(list)
    if index < 0 or index >= data.length
        return unit()
    load_ptr(data.items + index * ptr_size())
```

### 4.3 Map Implementation (Hash Table)

```coral
store MapData
    capacity: usize
    length: usize
    entries: usize    // Pointer to MapEntry array

store MapEntry
    hash: u64
    key: usize        // Value*
    value: usize      // Value*

// Open addressing with linear probing
*map_get(map, key)
    data is unwrap_map(map)
    h is hash_value(key)
    idx is h % data.capacity
    
    for i in 0..data.capacity
        entry is entry_at(data, (idx + i) % data.capacity)
        if entry.key == null
            return unit()  // Not found
        if entry.hash == h and value_eq(entry.key, key)
            return entry.value
    
    unit()  // Not found

*map_set(map, key, value)
    data is unwrap_map(map)
    if should_grow(data)
        grow_map(data)
    
    h is hash_value(key)
    idx is h % data.capacity
    
    for i in 0..data.capacity
        entry is entry_at(data, (idx + i) % data.capacity)
        if entry.key == null or value_eq(entry.key, key)
            if entry.key == null
                data.length is data.length + 1
            else
                release(entry.value)
            entry.hash is h
            entry.key is key
            entry.value is value
            retain(key)
            retain(value)
            return
```

### 4.4 String Implementation

```coral
// Small string optimization: strings ≤ 15 bytes stored inline
INLINE_STRING_MAX is 15

*make_string(bytes, len)
    if len <= INLINE_STRING_MAX
        make_inline_string(bytes, len)
    else
        make_heap_string(bytes, len)

*make_inline_string(bytes, len)
    v is alloc_value()
    v.tag is TAG_STRING
    v.flags is FLAG_INLINE
    // Store length in first byte, data in remaining 15
    v.payload.inline[0] is len
    memcpy(addr_of(v.payload.inline) + 1, bytes, len)
    v

*make_heap_string(bytes, len)
    data is alloc(len + 8)  // 8 bytes for length prefix
    store_u64(data, len)
    memcpy(data + 8, bytes, len)
    wrap_string_ptr(data)
```

---

## 5. Actor System

### 5.1 Actor Data Structures

```coral
store ActorData
    id: u64
    mailbox: usize      // Mailbox*
    parent_id: u64
    state: usize        // Value* (state map)
    handler: usize      // Closure*

store Mailbox
    capacity: usize
    read_pos: usize     // Atomic
    write_pos: usize    // Atomic
    messages: usize     // Ring buffer of Message*

type Message
    | Exit
    | User(payload: Value)
    | Failure(reason: String)
```

### 5.2 M:N Scheduler

```coral
store Scheduler
    workers: List[Worker]
    queue: WorkQueue
    next_id: u64        // Atomic

store Worker
    thread: usize       // pthread handle
    running: Bool       // Atomic

store WorkQueue
    tasks: List[Task]
    mutex: usize        // pthread_mutex_t
    cond: usize         // pthread_cond_t

*spawn_actor(parent, handler)
    id is atomic_inc(scheduler.next_id)
    mailbox is make_mailbox(DEFAULT_CAPACITY)
    actor is make_actor_data(id, mailbox, parent, handler)
    
    task is *() ->
        run_actor(actor)
    
    enqueue_task(scheduler.queue, task)
    make_actor_handle(id, mailbox)

*run_actor(actor)
    loop
        msg is mailbox_recv(actor.mailbox)
        match msg
            | Exit -> break
            | User(payload) ->
                invoke_handler(actor.handler, actor.state, payload)
            | Failure(reason) ->
                propagate_failure(actor.parent_id, reason)
```

### 5.3 Backpressure

```coral
*mailbox_send(mailbox, msg)
    // Try non-blocking first
    if mailbox_try_send(mailbox, msg)
        return Ok
    
    // Mailbox full - backpressure
    if BACKPRESSURE_POLICY == "drop"
        return Err("mailbox full, message dropped")
    else if BACKPRESSURE_POLICY == "block"
        // Wait for space (with timeout)
        mailbox_send_blocking(mailbox, msg, TIMEOUT_MS)
    else
        // Default: return error
        Err("mailbox full")
```

---

## 6. Memory Management

### 6.1 Reference Counting

```coral
*retain(value)
    if value == null
        return
    // Atomic increment
    atomic_add(addr_of(value.refcount), 1)

*release(value)
    if value == null
        return
    // Atomic decrement
    prev is atomic_sub(addr_of(value.refcount), 1)
    if prev == 1
        // Last reference - free
        free_value(value)

*free_value(value)
    match value.tag
        | TAG_STRING if not (value.flags & FLAG_INLINE) ->
            free(value.payload.ptr)
        | TAG_LIST ->
            free_list_data(value)
        | TAG_MAP ->
            free_map_data(value)
        | TAG_CLOSURE ->
            free_closure(value)
        | _ -> ()
    
    return_to_pool(value)
```

### 6.2 Value Pool

```coral
store ValuePool
    values: List[Value]
    limit: usize

POOL_LIMIT is 8192

*pool_get()
    if pool.values.length > 0
        record_metric(POOL_HIT)
        pool.values.pop()
    else
        record_metric(POOL_MISS)
        alloc_value()

*pool_return(value)
    if pool.values.length < POOL_LIMIT
        // Reset value
        value.refcount is 1
        value.flags is 0
        pool.values.push(value)
    else
        free(value)
```

### 6.3 Cycle Detection (Future)

```coral
// Bacon-Rajan concurrent cycle collector
// Tracks potential roots for cycle detection

store CycleCollector
    potential_roots: Set[Value]
    collecting: Bool    // Atomic

*mark_potential_root(value)
    // Called when refcount decremented but not to zero
    // and value contains references
    if could_be_cyclic(value)
        collector.potential_roots.add(value)

*collect_cycles()
    // Mark-sweep over potential roots
    // See Bacon-Rajan paper for algorithm
    ...
```

---

## 7. FFI Layer

### 7.1 C Function Declarations

```coral
// Memory
extern fn malloc(size: usize) : usize
extern fn free(ptr: usize)
extern fn realloc(ptr: usize, size: usize) : usize
extern fn memcpy(dst: usize, src: usize, len: usize) : usize
extern fn memset(dst: usize, value: u8, len: usize) : usize

// Threads
extern fn pthread_create(thread: usize, attr: usize, fn: usize, arg: usize) : u32
extern fn pthread_join(thread: usize, result: usize) : u32
extern fn pthread_mutex_init(mutex: usize, attr: usize) : u32
extern fn pthread_mutex_lock(mutex: usize) : u32
extern fn pthread_mutex_unlock(mutex: usize) : u32

// Atomics (via inline asm or compiler intrinsics)
*atomic_load(addr: usize) : u64
    // Platform-specific implementation
    unsafe
        asm "mov (%rdi), %rax"

*atomic_store(addr: usize, value: u64)
    unsafe
        asm "mov %rsi, (%rdi)"

*atomic_add(addr: usize, delta: u64) : u64
    unsafe
        asm "lock xadd %rsi, (%rdi)"
```

---

## 8. Implementation Plan

### Phase 1: Foundation (Weeks 1-3)

**Goal**: Basic memory and value operations

#### Tasks
- [ ] 1.1 Port value representation
- [ ] 1.2 Implement alloc/free wrappers
- [ ] 1.3 Implement tag manipulation
- [ ] 1.4 Implement retain/release
- [ ] 1.5 Test with simple programs

### Phase 2: Collections (Weeks 4-6)

**Goal**: List and Map working

#### Tasks
- [ ] 2.1 Implement list creation/access
- [ ] 2.2 Implement list mutation (push/pop)
- [ ] 2.3 Implement hash function
- [ ] 2.4 Implement map creation/access
- [ ] 2.5 Implement map mutation
- [ ] 2.6 Port collection tests

### Phase 3: Strings (Weeks 7-8)

**Goal**: String operations working

#### Tasks
- [ ] 3.1 Implement inline strings
- [ ] 3.2 Implement heap strings
- [ ] 3.3 Implement string concatenation
- [ ] 3.4 Implement string comparison
- [ ] 3.5 Port string tests

### Phase 4: Closures (Week 9)

**Goal**: Closures working

#### Tasks
- [ ] 4.1 Implement closure structure
- [ ] 4.2 Implement closure invocation
- [ ] 4.3 Implement capture retain/release
- [ ] 4.4 Port closure tests

### Phase 5: Actor Foundation (Weeks 10-12)

**Goal**: Basic actor spawn/send

#### Tasks
- [ ] 5.1 Implement mailbox
- [ ] 5.2 Implement actor data structure
- [ ] 5.3 Implement spawn
- [ ] 5.4 Implement send
- [ ] 5.5 Implement worker threads
- [ ] 5.6 Port actor tests

### Phase 6: Actor Features (Weeks 13-14)

**Goal**: Full actor system

#### Tasks
- [ ] 6.1 Implement backpressure
- [ ] 6.2 Implement failure propagation
- [ ] 6.3 Implement actor stop
- [ ] 6.4 Add metrics collection

### Phase 7: I/O (Week 15)

**Goal**: File and console I/O

#### Tasks
- [ ] 7.1 Implement file read/write
- [ ] 7.2 Implement console output (log)
- [ ] 7.3 Port I/O tests

### Phase 8: Integration (Weeks 16-17)

**Goal**: Full runtime working

#### Tasks
- [ ] 8.1 Integration with compiler
- [ ] 8.2 Full test suite passing
- [ ] 8.3 Performance benchmarks
- [ ] 8.4 Documentation

---

## 9. Testing Strategy

### 9.1 Compatibility Testing

```coral
// Every FFI function must produce identical results
*test_list_compat()
    rust_list is rust_runtime.make_list(0)
    coral_list is coral_runtime.make_list(0)
    
    for i in 0..1000
        rust_runtime.list_push(rust_list, i)
        coral_runtime.list_push(coral_list, i)
    
    for i in 0..1000
        assert_eq(
            rust_runtime.list_get(rust_list, i),
            coral_runtime.list_get(coral_list, i)
        )
```

### 9.2 Stress Testing

```coral
*test_refcount_stress()
    // Create and release many values rapidly
    for _ in 0..1000000
        v is make_number(42)
        retain(v)
        retain(v)
        release(v)
        release(v)
        release(v)
    
    assert_eq(live_value_count(), 0)
```

### 9.3 Concurrency Testing

```coral
*test_actor_stress()
    // Spawn many actors sending messages
    actors is []
    for i in 0..100
        a is spawn(*() ->
            count is 0
            loop
                recv()
                count is count + 1
                if count >= 1000
                    break
        )
        actors.push(a)
    
    for _ in 0..1000
        for a in actors
            send(a, unit())
    
    // Wait for completion
    for a in actors
        send(a, Exit)
```

---

## 10. Success Criteria

1. **Compatibility**: All existing programs run unchanged
2. **Correctness**: Full test suite passes
3. **Performance**: Within 2x of Rust runtime
4. **Stability**: No crashes in stress tests
5. **Memory**: No leaks in simple programs

---

## 11. Timeline Summary

| Phase | Duration | Milestone |
|-------|----------|-----------|
| 1. Foundation | 3 weeks | Values working |
| 2. Collections | 3 weeks | List/Map working |
| 3. Strings | 2 weeks | Strings working |
| 4. Closures | 1 week | Closures working |
| 5. Actor Foundation | 3 weeks | Basic actors |
| 6. Actor Features | 2 weeks | Full actors |
| 7. I/O | 1 week | I/O working |
| 8. Integration | 2 weeks | Full runtime |

**Total Estimated Time**: 17 weeks (~4 months)

---

## 12. Future Enhancements

After initial self-hosting:

1. **Cycle Collector**: Implement Bacon-Rajan or similar
2. **Arena Allocator**: For temporary values
3. **Value Compression**: Smaller value representation
4. **SIMD Operations**: Vectorized list/string ops
5. **Alternative Allocators**: jemalloc, mimalloc integration
