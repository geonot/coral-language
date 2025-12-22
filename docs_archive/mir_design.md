# MIR Design (Coral Intermediate Representation)

This document describes a minimal Medium-level IR (MIR) intended as a stable, higher-level target between the Coral AST and LLVM IR.

Goals
- Small, simple, explicit control flow (basic blocks + jumps).
- Typed-ish operands with `Value` shape that maps cleanly to the runtime's tagged `Value`.
- Easy to interpret for quick validation and to drive a MIR-to-LLVM lowering pass.

Core concepts

- Module: collection of functions.
- Function: named entry with typed params, local temporaries, and a list of BasicBlocks.
- BasicBlock: flat list of instructions ending with a terminator (Jump, Cond, Return).
- Instr: operations that mutate locals or perform side-effects (calls, memory ops, arithmetic).
- Operand: either a Local(String) or an immediate Constant.

Value model
- At MIR level `Value` is an opaque runtime value. For interpreter/testing we use a Rust enum with Number/Bool/String/List/Map/Unit.
- When lowering to LLVM, MIR ops that manipulate values must call runtime intrinsics (e.g., `coral_make_number`, `coral_value_add`, `coral_log`).

Primary instructions (prototype set)
- Const { dst, value } — write an immediate constant to `dst`.
- BinOp { dst, op, lhs, rhs } — arithmetic/logical op; delegates to runtime helpers when operands are non-numeric or for tagged ops.
- Call { dst, func, args } — call a function in the module or an external intrinsic by name; returns Value.
- Ret { value } — return from function; value optional.
- Jump { target } — unconditional branch.
- Cond { cond, then, else } — conditional branch on truthiness.
- AllocList { dst, len } — allocate a list (runtime helper) and place in dst.
- ListPush { list, value } — push value into list (runtime helper).
- MapMake/MapSet/MapGet — map helpers.
- GetField/SetField — for store/type lowering.

Calling conventions
- Functions accept/return `Value` (tagged); specific lowering may change this (e.g., specialized numeric ABI), but MIR keeps `Value` uniform.
- Intrinsics are identified by reserved names (e.g., `log`, `coral_make_number`) to map to runtime functions.

Example MIR (pseudocode)

function main() {
  const a = 1
  const b = 2
  binop c = add a b
  call _ = log(c)
  ret c
}

Lowering strategy
- Lower AST to MIR with explicit temporaries for expressions and named basic blocks for control flow.
- MIR→LLVM should be a relatively direct mapping: operations that require runtime helpers become `call` instructions to `coral_*` FFI functions, and values are represented as LLVM `ptr` to the runtime `Value` objects.

Interpreter role
- A MIR interpreter lets us validate semantics quickly and iterate on lowering design.
- The interpreter is implemented in Rust (prototype) and uses a simple Rust `Value` enum rather than the runtime: this decouples early validation from the runtime and allows fast iteration.

Next steps
- Implement MIR lowering pass (AST→MIR).
- Implement MIR-to-LLVM lowering that emits calls into the runtime.
- Expand the MIR instruction set as needed (closures, allocations, store/actor ops).
