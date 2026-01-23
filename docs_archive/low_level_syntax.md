# Low-Level Surface Syntax (extern / ptr / unsafe / asm)

## Extern functions
- Syntax: `extern fn name(param: type, ...) : ret` (return type optional).
- Required param annotations; supported types: `f64`, `bool`, `u8`, `u16`, `u32`, `u64`, `usize`, `ptr` (treated as `usize`).
- Return types support `f64`, `bool`, and the integer forms above; omit to get `void`.
- Codegen lowers arguments by converting Coral `Number` values to the annotated widths; integer returns are re-lifted as `Number` (bool as `Bool`).

## Pointer load
- Expression: `@addr_expr`.
- Semantics: evaluates `addr_expr` to a number, casts to `usize` as an address, loads an `f64` from that address, and produces a `Number`.
- Errors during bitcast/load surface as diagnostics.

## Unsafe blocks
- Syntax: `unsafe` followed by an indented block.
- Currently transparent in codegen (no extra checks); intended for wrapping raw pointer/FFI operations.

## Inline asm
- Syntax: `asm("template", constraint: expr, ...)` (inputs optional).
- Modes (via `CORAL_INLINE_ASM` env):
	- default: error
	- `allow-noop`: accept and drop (evaluates inputs for side effects)
	- `emit`: lower to LLVM inline asm with side effects (ATT dialect)
- Inputs are lowered as `f64` numbers to the asm function type; constraints are passed through verbatim.

## Memory FFI shims
- `std/runtime/memory.coral` wraps runtime intrinsics like `coral_malloc`, `coral_free`, `coral_memcpy`, `coral_memset`, `coral_ptr_add`, and load/store helpers for widths 8/16/32/64-bit and `f64`.
- All pointers are passed as `usize`; callers should use `ptr_add` and load/store helpers rather than manual arithmetic when possible.
