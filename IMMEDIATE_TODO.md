# Coral: Immediate Action TODO List

_Created: January 8, 2026_
_Target: Conference/Paper Readiness_

---

## 🔴 CRITICAL (P0) - Fix This Week

### 1. Implement Loop Constructs
**Why**: `fizzbuzz.coral` example uses `while` but it's not implemented
**Effort**: 8 hours
**Files**: 
- `src/lexer.rs` - Add `KeywordWhile`, `KeywordFor`, `KeywordBreak`, `KeywordContinue`
- `src/ast.rs` - Add `While`, `For`, `Break`, `Continue` statement variants
- `src/parser.rs` - Add parsing for loops
- `src/codegen/mod.rs` - Add LLVM emission for loops

### 2. Fix Standard Library Runtime Functions
**Why**: `std/io.coral` calls `fs_read`/`fs_write` which don't exist
**Effort**: 4 hours
**Files**:
- `runtime/src/lib.rs` - Add `coral_fs_read`, `coral_fs_write`, `coral_fs_exists` FFI functions
- `src/codegen/runtime.rs` - Declare runtime bindings

### 3. Fix Float Parsing Edge Case
**Why**: `123.` (trailing dot, no digits) is incorrectly accepted
**Effort**: 1 hour
**File**: `src/lexer.rs` around line 240
```rust
// After the loop, verify that if has_dot, there are digits after it
if has_dot && slice.ends_with('.') {
    return Err(Diagnostic::new("invalid float literal: expected digits after decimal point", Span::new(start, pos)));
}
```

### 4. Add Circular Import Detection
**Why**: Circular imports cause infinite loop in `expand_uses`
**Effort**: 2 hours
**File**: `src/module_loader.rs`
```rust
// Add visited set to expand_uses signature and check
fn expand_uses_inner(source: &str, ..., visited: &mut HashSet<PathBuf>) -> Result<String, ...>
```

### 5. Implement Generic Type Instantiation
**Why**: `List[Int]` compiles but doesn't enforce element types
**Effort**: 16 hours
**Files**:
- `src/types/solver.rs` - Add generic unification
- `src/types/env.rs` - Track type parameter bindings
- `src/semantic.rs` - Generate constraints for generic uses

---

## 🟠 HIGH PRIORITY (P1) - Fix in 2 Weeks

### 6. Add End-to-End Execution Tests
**Why**: Current tests verify IR, not runtime behavior
**Effort**: 8 hours
**Files**: `tests/execution.rs` (new)
```rust
#[test]
fn executes_hello_world() {
    let output = compile_and_run("examples/hello.coral");
    assert!(output.contains("Hello"));
}
```

### 7. Fix Atomic Ordering for Cross-Actor Values
**Why**: Using `Relaxed` ordering can cause data races
**Effort**: 4 hours
**File**: `runtime/src/lib.rs`
- Change refcount operations to use `Acquire`/`Release` when crossing thread boundaries

### 8. Trigger Cycle Collection Automatically
**Why**: Manual-only triggering leads to memory leaks
**Effort**: 4 hours
**File**: `runtime/src/cycle_detector.rs`
- Add allocation counter
- Trigger collection after N allocations or memory threshold

### 9. Enforce Type Annotations
**Why**: `x: Int is "hello"` compiles without error
**Effort**: 8 hours
**File**: `src/semantic.rs`
```rust
// In collect_constraints_expr for bindings:
if let Some(ann) = &binding.type_annotation {
    let ann_ty = type_from_annotation(ann);
    constraints.push(ConstraintKind::EqualAt(rhs_ty.clone(), ann_ty, binding.span));
}
```

### 10. Add `if/elif/else` Statement Support
**Why**: Only ternary expressions available, no statement form
**Effort**: 4 hours
**Files**:
- `src/lexer.rs` - Add `KeywordIf`, `KeywordElif`, `KeywordElse`
- `src/ast.rs` - Add `IfStatement` variant
- `src/parser.rs` - Parse if statements
- `src/codegen/mod.rs` - Emit conditionals

### 11. Add `return` Statement
**Why**: No way to return early from functions
**Effort**: 2 hours
**Files**:
- `src/parser.rs` - Parse `return expr`
- `src/codegen/mod.rs` - Emit `br` to return block

### 12. Resolve All Clippy Warnings
**Why**: Code quality for paper review
**Effort**: 2 hours
**Commands**:
```bash
cargo clippy --fix --lib -p coralc
# Then manually fix remaining issues
```

---

## 🟡 MEDIUM PRIORITY (P2) - Fix Before Beta

### 13. Split `runtime/src/lib.rs` (4824 lines)
**Effort**: 8 hours
**New files**:
- `runtime/src/value.rs` - Value struct and basic ops
- `runtime/src/list.rs` - List operations
- `runtime/src/map.rs` - Map operations
- `runtime/src/string.rs` - String operations
- `runtime/src/bytes.rs` - Bytes operations
- `runtime/src/error.rs` - Error handling

### 14. Split `src/codegen/mod.rs` (3831 lines)
**Effort**: 8 hours
**New files**:
- `src/codegen/expression.rs` - Expression emission
- `src/codegen/statement.rs` - Statement emission
- `src/codegen/function.rs` - Function/method bodies
- `src/codegen/store.rs` - Store/actor construction

### 15. Implement or Remove MIR
**Effort**: 16-40 hours depending on choice
**Options**:
- **Remove**: Delete `src/mir.rs`, `src/mir_lower.rs`, update imports
- **Implement**: Add SSA form, phi nodes, optimization passes

### 16. Add Fuzzing Infrastructure
**Effort**: 8 hours
**Files**: `fuzz/` directory with:
- `fuzz_lexer.rs`
- `fuzz_parser.rs`
- `fuzz_runtime.rs`

### 17. Add Benchmark Suite
**Effort**: 8 hours
**Files**: `benches/` directory with Criterion benchmarks
- Lexer throughput
- Parser throughput
- Runtime operations (list, map, string)

### 18. Intern Actor Message Names
**Why**: Currently string-compared on every dispatch
**Effort**: 4 hours
**File**: `runtime/src/actor.rs`
- Use `runtime/src/symbol.rs` SymbolId for message names
- Generate dispatch table at compile time

---

## 🟢 LOW PRIORITY (P3) - Nice to Have

### 19. Add Unicode Identifier Support
**File**: `src/lexer.rs`
- Use `unicode-xid` crate for `is_xid_start`/`is_xid_continue`

### 20. Add Number Literal Separators
**Example**: `1_000_000` for readability
**File**: `src/lexer.rs`

### 21. Add Multi-line Comments
**Syntax**: `/* ... */`
**File**: `src/lexer.rs`

### 22. Add Doc Comments
**Syntax**: `## This is a doc comment`
**Files**: `src/lexer.rs`, `src/ast.rs`

### 23. Pattern Matching Extensions
- Guard clauses: `Some(x) when x > 0 ? ...`
- Range patterns: `1..10 ? ...`
- OR patterns: `Some(x) | None ? ...`

### 24. Effect System Design
**Effort**: 40+ hours
- Design syntax: `*foo() : IO[String]`
- Implement effect inference
- Add effect checking

---

## Quick Wins (< 1 hour each)

1. ✅ Remove unused import `TypeVarId` in `src/types/env.rs`
2. ✅ Remove dead field `dependency_hash` in `src/module_loader.rs`
3. ✅ Change `map_or` to `is_some_and` in `src/codegen/mod.rs:2123`
4. ✅ Make `contains_placeholder` a free function (doesn't need `&self`)
5. ✅ Make `replace_placeholder_with` a free function (doesn't need `&self`)
6. ✅ Remove `assert!(true)` in tests
7. ✅ Fix length comparison to zero (use `.is_empty()`)

---

## Testing Checklist Before Conference

- [ ] All 214+ existing tests pass
- [ ] New loop construct tests pass
- [ ] End-to-end execution tests pass
- [ ] No Clippy warnings
- [ ] Demo programs run without error
- [ ] `cargo doc` generates without errors
- [ ] README examples work

---

## Documentation TODO

- [ ] Write "Known Limitations" document
- [ ] Update README with accurate feature status
- [ ] Add inline comments to complex functions
- [ ] Write "Getting Started" guide
- [ ] Document all std/ module functions
- [ ] Create architecture diagram

---

## Conference Preparation Checklist

- [ ] Prepare 3-5 working demo programs
- [ ] Write speaker notes explaining design decisions
- [ ] Prepare slides showing:
  - Language philosophy
  - Syntax innovations
  - Actor/store model
  - Error handling approach
  - Current limitations (honestly!)
- [ ] Have backup plan if live demo fails
- [ ] Practice explaining type inference approach
- [ ] Know related work to compare against
