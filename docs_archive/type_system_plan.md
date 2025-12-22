# Type System & Inference Plan

_Last updated: 2025-12-04_

## 1. Goals
- Provide a sound, predictable type system for Coral while maintaining its hackable feel.
- Cover primitive types (Int, Float, Bool, String, Bytes), composite types (List[T], Map[K, V], Structs), and low-level constructs (Pointer[T], BitVector[N]).
- Support gradual inference: unannotated code still runs, but diagnostics guide users toward explicit typing.

## 2. Architecture Overview
1. **Type graph + constraint solver**
   - Use a Hindley–Milner–inspired graph with union-find for type variables.
   - Constraints gathered during semantic analysis; solver produces principal types where possible.
2. **Annotation surface**
   - Allow annotations on bindings, parameters, and function return values: `value is 0 : Int` or `*add(a: Int, b: Int) : Int`.
   - Type literals for bytes (`0xFFu8`), arrays (`bytes[16]`), pointer types (`ptr[Int]`).
3. **Literal typing**
   - Distinguish `Int` vs `Float` based on literal form; allow suffixes (`42i64`, `3.14f32`).
   - Strings default to `String`; `b"..."` to `Bytes`.
4. **Bitwise & low-level ops**
   - Add AST + runtime support for `& | ^ ~ << >>` with overloads for Int/BitVector/Bytes.
   - Provide pointer arithmetic in a `unsafe` block (future).

## 3. Inference Phases
1. **Primitive inference (NOW)**
   - Build symbol table with types for globals + functions.
   - Constraint rules for arithmetic (`+` demands numeric), logic (`and` demands Bool), comparisons.
2. **Collection generics (NEXT)**
   - Parameterize lists/maps: `List[T]`, `Map[K, V]`.
   - Unify element types during literal creation and when calling methods.
3. **Higher-order / closures (NEXT)**
   - Represent function types as `(ParamTypes) -> ReturnType`.
   - Inference for placeholder lambdas by synthesizing function type variables.
4. **Traits / interfaces (LATER)**
   - Introduce structural typing for records and ability to satisfy interfaces implicitly.
5. **Effect typing & comptime (LATER)**
   - Tag functions with effects (`IO`, `Actor`, `Comptime`) and enforce at call sites.

## 4. Diagnostics & UX
- Emit multi-span diagnostics showing both the inferred type and the conflicting usage.
- Suggest annotations when inference fails ("annotate `value` as `Int`")
- Provide `:type expr` REPL command (future) using the solver result.

## 5. Implementation Tasks
1. AST updates: allow optional `TypeExpr` on bindings/params, parse primitive types and pointer/bytes syntax.
2. Type definitions: create `TypeId` enum with variants for primitives, generics, function types, placeholders.
3. Constraint builder: traverse AST collecting constraints, storing them in `TypeConstraintSet`.
4. Solver: union-find structure with occurs-check, ability to generalize at bindings.
5. Runtime alignment: ensure runtime APIs know the primitive layout (e.g., separate constructors for `Bytes`, pointer wrappers).
6. Bitwise runtime support: add `coral_value_bitand` etc., backed by specialized numeric paths.

## 6. Research & inspiration
- **ML/HM** for baseline inference.
- **Rust Chalk / Trait solving** for future trait system.
- **Swift's request-based type checker** to borrow incremental checking ideas.
- **Julia multiple dispatch** as a north star for dynamic specializations (possible future add-on).
