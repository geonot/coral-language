# Actor & Runtime Implementation Status

## Current State (Dec 2025)

### Runtime (Rust)
- **M:N actor system**: work-stealing scheduler, parent tracking, frozen messages (no deep clone).
- **FFI surface**: `coral_actor_spawn/send/stop/self` for LLVM IR.
- **Value system**: tagged union with refcounting; `ValueTag::Actor` wraps `ActorHandle`.
- **Frozen bit**: prevents mutation after send; list/map mutators check `FLAG_FROZEN`.

### Parser/AST
- ✅ Parses `actor Foo` and `store actor Bar` with `@message` handlers.
- ✅ AST: `StoreDefinition.is_actor`, `FunctionKind::ActorMessage`.

### Lowering/Semantic
- ⚠️  Placeholder lowering passes through stores/actors unchanged.
- ⚠️  Semantic validates field uniqueness but doesn't analyze actor message handlers or lifetime.

### Codegen
- ✅ Actor intrinsics declared (`coral_actor_spawn/send/stop/self`).
- ❌ No lowering for actor definitions → runtime calls.
- ❌ Stores (including `store actor`) are not emitted at all.

## Missing for Full Actor Support

### Immediate (alpha-ready)
1. **Actor constructor lowering**: treat `actor Foo` as a factory function:
   - Build a closure capturing actor fields.
   - Call `coral_actor_spawn(closure)` → returns actor handle.
   - Each `@msg` method → closure handler for message dispatch.
2. **Message dispatch**: handler closure receives `(self, message)`, matches on message envelope `{code, data, type}`.
3. **Codegen for actor calls**: `actor_instance.method(args)` → `coral_actor_send(actor_instance, envelope)`.

### Store Foundation (needed for `store actor`)
- Heap layout for store fields (similar to closures: struct with fields).
- Constructor functions that allocate store objects.
- Field access via offset/GEP.
- Reference fields (`&product`) → hold another Value handle.

### Later Enhancements
- Supervision strategies (restart policies).
- Distributed actors (serialization, network transport).
- Hot code reload.

## Path to Coral-Native Runtime

### Why Rust Today?
- Coral lacks raw pointers, inline asm, FFI, unsafe ops.
- Rust runtime = productivity + safety; C ABI exports for LLVM.
- Static/dynamic link keeps single-binary deployment simple.

### Bootstrapping Coral Runtime
1. **Add FFI surface to Coral**:
   - `extern "C"` function declarations.
   - Raw pointer type (`*T`).
   - Unsafe blocks for direct memory manipulation.
2. **Minimal C shim**:
   - Syscall wrappers (read/write/mmap/futex).
   - Atomic ops (CAS, fence).
   - Panic handler.
3. **Port runtime modules incrementally**:
   - Start with pure collections (list/map) in Coral.
   - Then actor scheduler (queues, work-stealing).
   - Keep libc/syscalls in C shim until Coral inline asm lands.
4. **Self-hosting compiler**:
   - Parser/lexer/lowering in Coral.
   - LLVM bindings via FFI or generate IR as text.

### Stack vs Heap (Linux Performance)

**Stack**:
- Per-thread, contiguous, guard-page protected.
- Allocation = pointer bump (sub rsp); extremely fast (~1 cycle).
- Great cache locality; no syscalls for normal growth (kernel expands on page fault).
- Limited size (8MB default ulimit); stack overflow = SIGSEGV.

**Heap**:
- General-purpose allocator (malloc/jemalloc); more overhead (metadata, locking).
- Fragmentation over time; larger allocations may trigger mmap/brk syscalls.
- Unlimited (subject to VM/swap); slower than stack but necessary for dynamic lifetime.

**Shared/Global Stack Ideas**:
- ❌ Sharing parent stack with callee breaks isolation in async/multithreaded contexts; not safe.
- ❌ Single global stack complicates reentrancy/recursion; non-standard ABI.
- ✅ **Segmented stacks** (goroutine model): each actor gets small stacklet; grow by chaining segments on heap. Bounds-checked; good for cooperative schedulers.
- ✅ **Arena/region allocators**: stack-like but on heap; bump-allocate, bulk-free on scope exit. Great for request handlers.

**Recommendation**: Per-actor stacklets + arena allocators for actor-local temporaries; frozen messages avoid cross-actor heap contention.

## Rust Runtime Dependency Audit

### What's in Rust?
- All `coral_*` intrinsics (~55 symbols): make_number/string/list/map, list_push/pop, map_get/set, actor_spawn/send/stop, fs_read/write, closure_invoke, value_retain/release, etc.
- Actor scheduler: M:N work queue, worker threads, message channels.
- Value refcounting, pool, heap allocator wrappers.

### How It's Linked
- Rust crate compiled as `cdylib` (shared lib) + `rlib` (static).
- Compiler emits LLVM IR with `declare` stubs for `coral_*`.
- CLI links runtime:
  - JIT: preload shared lib via `lli -load`.
  - AOT: static link with `clang`.

### Moving Off Rust
- Replace one intrinsic at a time: rewrite in Coral, export via C shim.
- Priority order: collections → actor scheduler → I/O → allocator.
- Keep thin C layer for syscalls/atomics until Coral gets inline asm.

## Next Steps
1. Implement minimal actor constructor/message dispatch lowering (treat as opaque for now; skip store fields).
2. Add codegen test for actor spawn/send.
3. Document FFI design for Coral-native runtime.
4. Plan store layout (similar to closures: heap struct with fields).
