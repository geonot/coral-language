# Coral Language Evolution Brainstorm

## Production polish checklist
- **Bitwise operations**: add `& | ^ ~ << >>` plus `bitcount`, `rotate_left/right` helpers in `std.bit`.
- **Bytes & pointer literals**: `b"hello"`, `ptr(addr)`, `addr-of` operator for interop with runtime buffers.
- **Unsafe blocks**: explicit `unsafe { ... }` region for pointer arithmetic and foreign calls.
- **Inline assembly hooks**: consider `asm` blocks targeting LLVM for low-level hackers.
- **Deterministic build pipeline**: `coralc build --cache` with reproducible IR and embedded runtime metadata.

## Potential new language features
1. **Pipelines / method chaining**
   - Proposed symbols/words (no implementation yet):
     - `value ~> fn()` (tilde arrow keeps single token)
     - `value @ fn()` (reads "value at fn")
     - `value then fn()` (one-word keyword alternative)
   - Decision pending feedback; lowering will desugar to nested calls once symbol picked.
2. **Pattern destructuring for lists/maps**
   - `let [head, ..tail] is numbers` style matches.
3. **Tagged unions / sum types**
   - `union Result<T, E> { Ok(T), Err(E) }` with exhaustive matching.
4. **Compile-time reflection**
   - Access AST/MIR metadata during comptime to build DSLs or auto-generate bindings.
5. **Effect handlers**
   - Structured concurrency primitives to manage async actors.
6. **Metaprogramming via Coral macros**
   - Quote/unquote syntax (`quote { ... }`) to manipulate Coral AST from Coral.

## Alignment with Coral paradigm
- Keep syntax indentation-driven and string-friendly.
- Favor expression-oriented constructs (match, pipelines) over statements.
- Provide zero-cost abstractions: compile-time evaluation ensures optimized runtime.
- Maintain hackable surface: inline assembly, unsafe, pointer types for low-level control, but gated by explicit opt-in constructs.

## Next steps
1. Implement bitwise ops + bytes type in runtime and expose them via std.
2. Prototype pipeline operator lowering (desugars to nested calls) and add tests.
3. Design union type syntax + MIR lowering.
4. Extend comptime interpreter to allow reflection/splicing experiments.
