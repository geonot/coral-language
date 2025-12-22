# Universal Value-as-Result Model

Goal: every Coral value can behave like `Result`/`Option` without adding explicit sum types. Errors propagate through standard operations by carrying metadata on the `Value` header.

## Representation
- Extend `ValueTag` with two flag bits: `ERR` (value represents an error) and `ABSENT` (value is logically `None`).
- Store optional payloads:
  - `error_code: u32`
  - `error_message: ValueHandle` (string)
  - `origin_span: SpanId` for diagnostics.
- Regular values simply have both flags unset.

## API surface
- Every value gains methods (available via member syntax) implemented in runtime:
  - `value.is_ok`, `value.is_err`, `value.unwrap(default)`, `value.or(other)`.
  - `value.expect(message)` triggers runtime diagnostic if `ERR` flag is set.
- Built-ins automatically propagate flags: arithmetic on an `ERR` value yields the same error without recomputing.

## Construction
- `raise(err)` helper wraps any value with the `ERR` flag.
- Parser/lowerer desugars `?` operator (future) into `value.unwrap()` semantics.
- IO/runtime functions mark failures accordingly (e.g., map lookup returning missing entry sets `ABSENT`).

## Advantages
- No need for explicit `Result[T, E]` or `Option[T]` in surface syntax; values remain dynamic.
- Works alongside future sum types if we want stronger typing.
- Allows composable error pipelines without `unsafe` or verbose pattern matching.

## Implementation plan
1. Modify `ValueHeader` to include flag bits and optional metadata pointer.
2. Update runtime helpers to copy/merge flag bits during operations.
3. Expose std helpers (`std.result`) that manipulate flags purely in Coral.
4. Extend diagnostics printer to surface origin spans when errors bubble to top-level.
