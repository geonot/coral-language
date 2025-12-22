# Closure & Lambda Runtime Bridge

## Goals
- Represent Coral lambdas as first-class values that can be passed to runtime helpers (`list.map`, `reduce`, user-defined HOFs).
- Support captured variables without full GC yet—leverage ARC semantics already planned for the runtime.
- Keep ABI simple so the compiler can emit LLVM structs without bespoke per-lambda types.

## Closure Representation
```
struct CoralClosure {
    void (*invoke)(void* env, ValueHandle* args, size_t len, ValueHandle* out);
    void (*release)(void* env);
    void* env;
}
```
- `invoke` knows how to run the lambda with a Value array (to avoid per-arity trampolines initially). Later we can specialize for arities 1 and 2.
- `release` is optional; if null, env is plain pointer freed by runtime ARC.
- `env` holds captured values in a heap-allocated struct. Compiler emits one struct definition per lambda, but the runtime treats it opaquely.

## Compiler Responsibilities
1. Emit LLVM struct for captures (empty struct when no captures) plus a constructor function returning `CoralClosure`.
2. Generate an `invoke` shim with signature: `void invoke(void* env, ValueHandle* args, size_t len, ValueHandle* out)`.
   - Downcast env pointer to capture struct, load captures, then execute lambda body.
   - Writes result handle into `out` (retains before storing, so caller releases).
3. Provide a `release` shim that decrements capture refcounts then frees env.
4. At lambda literal site:
   - Allocate capture struct, retain captured handles, populate fields.
   - Call runtime helper `coral_make_closure(invoke_fn_ptr, release_fn_ptr, env_ptr)` returning a `ValueHandle` for use in expressions.

## Runtime Helpers (C ABI)
- `coral_make_closure(void* invoke, void* release, void* env) -> ValueHandle`
- `coral_closure_invoke(ValueHandle closure, ValueHandle* args, size_t len) -> ValueHandle`
- `coral_closure_release_env(void* env, void (*release)(void*))` used internally by Value drop.

## Placeholder Lowering Strategy
- Run a transform pass after parsing but before semantics.
- Track call sites expecting callables (currently `.map`, `.reduce`, `.filter`, method calls where the argument position is flagged as callable in metadata). For now, use syntactic cues: if placeholder appears inside argument expression, wrap that argument expression in synthesized lambda.
- Algorithm:
  1. Walk AST; whenever a placeholder is encountered, bubble up to nearest candidate expression boundary (argument, binding RHS) and record placeholder indices used.
  2. Replace placeholder references with identifier expressions for synthesized parameters `_arg0`, `_arg1`, ...
  3. Wrap expression in `Expression::Lambda { params: [_arg0, ...], body: Block::from_expression(original_expr) }`.
  4. Emit diagnostics if placeholder indices are non-contiguous or start at zero mismatch.

## Capture Semantics
- Lambdas capture by reference: captured handles are retained when closure is constructed; release shim drops them.
- Mutability: captured bindings remain immutable for now (matching existing semantics). Mutable captures would require copy-on-write semantics.

## Next Steps
1. Implement placeholder-lowering pass returning rewritten AST plus diagnostics.
2. Extend semantic model with `ClosureInfo` (list of captures, pointer to LLVM function).
3. Update codegen to emit closure structs, shims, and runtime calls using this ABI.
4. Flesh out runtime `coral_make_closure` / invoke APIs in `runtime/src/lib.rs`.
