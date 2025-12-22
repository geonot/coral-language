# Coral Runtime Bootstrap (Coral-in-Coral)

_Last updated: 2025-12-11_

## Status (2025-12-11)
- ✅ AST/Parser: `extern fn`, `unsafe` blocks, `asm(...)` expressions, `@` pointer load added and parsing correctly.
- ✅ Lexer: keywords `extern`, `unsafe`, `asm`, `ptr` added.
- ✅ Runtime FFI: 15 new low-level memory shims exposed (`coral_malloc/free/memcpy/memset/ptr_add/load*/store*`).
- ✅ Std modules: `std/runtime/memory.coral` wraps FFI intrinsics; `std/runtime/value.coral` sketches tagged value layout.
- ✅ Codegen: extern calls, pointer loads, and inline asm now lower to LLVM IR (inline asm guarded by `CORAL_INLINE_ASM=emit`, ATT dialect, side-effecting).
- ✅ Tests: parser + extern/asm smoke tests passing; module loader exercises std + runtime memory.
- 🔲 Module loader integration: ensure `std/runtime/*` loads before default runtime binding.

## Goals
- Replace the Rust runtime with a Coral-written runtime, staged and testable.
- Preserve working JIT/binary output by shimming Coral runtime calls onto the existing Rust runtime while Coral code matures.
- Enable low-level control: pointers, manual layout, inline LLVM asm, and C shims for syscalls and platform interop.

## Constraints
- Keep the current compiler pipeline usable during the transition (no hard breaks to tests/examples).
- Stage features behind guarded surface syntax (`extern`, `asm`, `ptr`, `unsafe`) so we can land them incrementally.
- Maintain determinism: explicit retains/releases (or arenas) and no hidden GC.

## Phased Plan
1) **FFI bridge + intrinsics (scaffold)** — COMPLETE (core forms + lowering landed; inline asm emits under env flag)
   - Surface forms: `extern fn name(args) -> type`, `asm("...", inputs -> outputs)`, `ptr` alias to `usize`, and `@expr` load semantics.
   - Intrinsics bound to Rust runtime: `coral_malloc`, `coral_free`, `coral_memcpy`, `coral_memset`, `coral_ptr_add`, loads/stores, etc.
   - Codegen: extern calls lower to LLVM `declare` + call; `asm` lowers to LLVM inline asm (side-effect, ATT); `@x` lowers to pointer load of f64.

2) **Coral runtime core in Coral (stage 1)** — IN PROGRESS
   - `runtime.memory` wrappers landed (alloc/free/copy/set/ptr_add/load*/store* via intrinsics).
   - `runtime.value` now exposes FFI constructors/accessors/retain/release; next: header layout helpers + Coral-side allocations.
   - `runtime.actor` wrappers added (spawn/send/stop/self, closure make/invoke); next: Coral mailbox helpers atop Rust scheduler.
   - TODO: `runtime.list` / `runtime.map` address-indexed collections with Rust helpers underneath.

3) **Coral actors + scheduling (stage 2)**
   - Re-express actor spawn/send/recv in Coral atop the Rust scheduler, then progressively reimplement scheduler primitives (mailboxes, threads/workers) in Coral once asm/syscalls are stable.
   - Define message dispatch tables in Coral (interned ids) instead of Rust string compare.

4) **Native resource and syscall layer (stage 3)**
   - Provide `runtime.sys` for raw syscalls via `asm`/`syscall` instruction numbers; fall back to libc shims where needed.
   - Expose CPU/atomic primitives (fence, cas, atomic add/sub/xchg) via inline asm.

5) **Rust runtime retirement (stage 4)**
   - Flip bindings so Coral runtime implementations no longer call into Rust; keep Rust only as a host for LLVM and minimal bootstrapping.

## Required Compiler Work (near-term)
- **AST/Parser**: add nodes for `extern fn`, `asm` expression, pointer type literal, `unsafe` blocks (for unchecked loads/stores/asm).
- **Semantic**: track `extern` decls and type-check calls; mark unsafe contexts.
- **MIR/Codegen**:
  - Lower `extern` calls to LLVM `declare` (C ABI) and emit calls.
  - Lower `asm` to LLVM inline asm with side-effect flags; support input/output constraints minimal set.
  - Add `ptr`/`usize` lowering, pointer arithmetic, load/store intrinsics.
  - Keep current tagged `Value` path for existing code; allow new code paths for raw pointers behind feature flags.
- **Std module loader**: allow `std.runtime.*` modules written in Coral to load before falling back to Rust runtime symbols.

## Initial Coral Modules to Author
- `std/runtime/memory.coral`: alloc/free, memcpy/memset, ptr_add, load/store sized ops, align.
- `std/runtime/value.coral`: tagged header layout, constructors for number/bool/string/bytes/unit, retain/release delegating to Rust for now.
- `std/runtime/list.coral`: address-indexed list with a side index of element addresses; push/get/len; optional contiguous buffer fallback.
- `std/runtime/map.coral`: key/value address table + hash helpers (reuse `std.bit` ops for hashing initially).
- `std/runtime/actor.coral`: spawn/send/recv wrappers calling into Rust actor FFI until scheduler is Coral-native.

## Incremental Landing Plan (PR-sized chunks)
1) Parser + AST for `extern`/`asm`/`ptr`/`unsafe`; codegen stubs emitting `unimplemented` to keep builds green.
2) Wire LLVM inline asm emission; add a few smoke tests for `asm("nop")` and a `syscall` wrapper prototype.
3) Add `extern` binding for `coral_malloc/free/memcpy/memset` (declared in runtime crate) and expose them as intrinsics.
4) Add `std/runtime/memory.coral` wrapping those intrinsics; add tests that allocate/copy memory and round-trip bytes.
5) Implement `runtime.value` in Coral with retain/release delegating to Rust; add number/bool/string constructors.
6) Implement address-indexed `list`/`map` in Coral; gate behind feature flag; keep Rust-backed versions as default until stable.
7) Re-express actor spawn/send/recv in Coral while still calling Rust scheduler; gradually inline mailbox logic.
8) Remove Rust runtime dependencies once metrics and perf are within tolerance.

## Testing Hooks
- Add new `tests/runtime_coral_*.rs` to drive Coral modules through the compiler + JIT path.
- Add property tests for retain/release balance, list/map semantics, and pointer arithmetic (alignment/overflow guards).
- Include sanitizer runs (`asan`, `miri`) on the Rust shim side while Coral code is calling into it.

## Open Questions
- Exact syntax for pointer literals (`ptr` alias vs `*T` style); recommend starting with `ptr` as `usize` and `@` for load.
- Whether to allow implicit retain/release in Coral runtime or keep them explicit for predictability.
- Target set for inline asm constraints (x86_64 first? arm64 later?).

## Pointer + ASM Surface (proposed)
- `ptr` aliases `usize`; `@expr` loads a value of the static type of `expr` (or cast: `@expr as T`).
- `ptr_add(p, bytes)` returns a `ptr`; `ptr_diff(a, b)` returns `usize`.
- `load8/16/32/64/float64(ptr)` and matching `store*` for unboxed memory ops; only legal in `unsafe`.
- `unsafe { ... }` encloses unchecked memory/asm/syscall blocks.
- `extern fn coral_memcpy(dst: ptr, src: ptr, len: usize) -> ptr` style declarations for binding to Rust shims.
- `asm("syscall", inputs -> outputs)` expression, with a minimal constraint set (`r`, `m`, `i`, `~{memory}`) and `volatile` flag by default.

## Sample Coral Snippet (address-indexed list)
```coral
use std.runtime.memory as mem

type addr_list
   addresses ? []

   *push(elem_addr)
      addresses.push(elem_addr)

   *at(i)
      addresses.get(i)

   *len()
      addresses.length()

*clone_into_heap(src_ptr, byte_len)
   dst is mem.alloc(byte_len)
   mem.copy(dst, src_ptr, byte_len)
   dst

*example()
   p0 is mem.alloc(8)
   mem.store64(p0, 42)
   xs is addr_list()
   xs.push(p0)
   first_ptr is xs.at(0)
   mem.load64(first_ptr)
```

The snippet assumes `std.runtime.memory` exposes `alloc`, `copy`, `load64`, and `store64` backed by intrinsics declared via `extern`.

## Next Actions (immediate)
- Add IR assertions for inline asm (done) and extend to syscall-style templates with inputs/constraints.
- Flesh out `std/runtime/value.coral` with header layout helpers and retain/release stubs calling Rust.
- Add Coral-side pointer helpers for slice/struct layout (align, sizeof) and tests that round-trip through runtime FFI.
