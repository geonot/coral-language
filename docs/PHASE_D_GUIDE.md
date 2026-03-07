# Phase D Implementation Guide — Coral Self-Hosted Bootstrap

**Purpose:** Self-contained guide for executing Phase D: fix self-hosted compiler bugs, achieve first execution, bootstrap the compiler.  
**Scope:** ~45 tasks across 4 tracks: Bug Fixes, Execution Verification, Bootstrap, Hardening.  
**Prerequisite:** Phase C is complete — all 7 self-hosted modules compile to LLVM IR (7,343 lines), 755+ tests pass, 0 failures.  
**Estimated effort:** 80-120 hours.

---

## Table of Contents

1. [Language Design Constraints](#1-language-design-constraints)
2. [Architecture Overview](#2-architecture-overview)
3. [File Map](#3-file-map)
4. [Build & Test Commands](#4-build--test-commands)
5. [Phase C Evaluation Summary](#5-phase-c-evaluation-summary)
6. [Track 1: Cross-Module Bug Fixes](#6-track-1-cross-module-bug-fixes)
7. [Track 2: First Execution](#7-track-2-first-execution)
8. [Track 3: Bootstrap](#8-track-3-bootstrap)
9. [Track 4: Hardening & Edge Cases](#9-track-4-hardening--edge-cases)
10. [Execution Order](#10-execution-order)
11. [Verification Checklist](#11-verification-checklist)

---

## 1. Language Design Constraints

These are non-negotiable. Every change must respect all 10 rules:

1. **Pure type inference** — no type annotations in user code; all types inferred via constraint solving
2. **`is` for binding** — no `=` or `==`; `is` is the binding operator (`x is 5`)
3. **Method-based equality** — `.equals()` / `.not_equals()` instead of `==` / `!=`
4. **Single `Number(f64)` at runtime** — one numeric type; Int distinction is compile-time only
5. **Value-error model** — every value carries error/absence metadata via flags (bit 0 = ERR, bit 1 = ABSENT); no exceptions
6. **Indentation-based syntax** — Python-style blocks via INDENT/DEDENT tokens
7. **`*` marks functions** — `*foo(x)` defines a function
8. **`?`/`!` for ternary** — `condition ? then ! else`
9. **`~` for pipeline** — `value ~ fn1 ~ fn2` desugars to `fn2(fn1(value))`
10. **Actors are the concurrency primitive** — no shared mutable state; message passing only

---

## 2. Architecture Overview

### Rust Compiler Pipeline (builds the self-hosted compiler)

```
self_hosted/*.coral
  → Rust Lexer (src/lexer.rs)         → Vec<Token>
  → Rust Parser (src/parser.rs)       → AST (Program)
  → Rust Semantic (src/semantic.rs)   → SemanticModel
  → Rust Lower (src/lower.rs)         → Lowered AST
  → Rust Codegen (src/codegen/)       → LLVM IR
  → lli -load libruntime.so           → Execution (self-hosted compiler running)
```

### Self-Hosted Compiler Pipeline (what the self-hosted compiler itself does)

```
input.coral
  → Coral Lexer (lexer.coral)          → Token list (maps)
  → Coral Parser (parser.coral)        → AST (nested maps/lists)
  → Coral Lower (lower.coral)          → Desugared AST
  → Coral Module Loader (module_loader.coral) → Merged source
  → Coral Semantic (semantic.coral)    → Type-checked model
  → Coral Codegen (codegen.coral)      → LLVM IR text (.ll)
  → llc + clang (external)            → Binary
```

### Bootstrap Chain

```
Phase D goal:

  coralc (Rust) → compiles → self-hosted.ll → lli → runs → compiles hello.coral → hello.ll ✓
                                                         → compiles self → self-v2.ll
                                                         → diff self.ll self-v2.ll → identical ✓
```

---

## 3. File Map

### Self-Hosted Compiler (`self_hosted/`)

| File | Lines | Key Functions | Known Bugs |
|------|-------|---------------|------------|
| `lexer.coral` | 528 | `lex()`, `make_lexer()`, `measure_indent()` | None |
| `parser.coral` | 1,800 | `parse_program()`, `parse_expression()`, `parse_function()` | SH-5 (template interpolation) |
| `lower.coral` | 665 | `lower_program()`, `lower_expression()`, `replace_placeholders()` | None |
| `module_loader.coral` | 284 | `load_module()`, `resolve_path()`, `extract_exports()` | ML1 (text-based exports) |
| `semantic.coral` | 1,677 | `analyze()`, `check_expression()`, `solve_constraints()` | SH-1 (elif_branches format) |
| `codegen.coral` | 2,109 | `generate()`, `emit_expression()`, `emit_function()` | SH-2, SH-3, SH-4 |
| `compiler.coral` | 280 | `compile()`, `compile_file()`, `fold_constants()` | None |

### Rust Reference Compiler (`src/`)

| File | Lines | Purpose |
|------|-------|---------|
| `src/lexer.rs` | 869 | Reference lexer implementation |
| `src/parser.rs` | 2,243 | Reference parser |
| `src/semantic.rs` | 2,480 | Reference semantic analysis |
| `src/lower.rs` | 651 | Reference lowering |
| `src/module_loader.rs` | 521 | Reference module loader |
| `src/codegen/` | 6,654 | Reference LLVM codegen (6 files) |
| `src/compiler.rs` | 263 | Reference pipeline orchestrator |
| `src/types/` | 1,483 | Type system (core, env, solver) |
| **Total** | **16,143** | |

### Runtime (`runtime/src/`)

Not modified by Phase D. The self-hosted compiler links against the existing Rust runtime (`libruntime.so`).

---

## 4. Build & Test Commands

```bash
# Build the Rust compiler
cargo build --release

# Run all tests
cargo test

# Run self-hosting tests only
cargo test --test self_hosting

# Compile a self-hosted module to LLVM IR (via Rust compiler)
cargo run -- self_hosted/compiler.coral -o /tmp/self_hosted_compiler.ll

# Execute the self-hosted compiler via lli (once bugs are fixed)
lli -load target/release/libruntime.so /tmp/self_hosted_compiler.ll < input_program.coral

# Compare LLVM IR output
diff <(cargo run -- test.coral -o -) <(lli -load target/release/libruntime.so /tmp/self_hosted_compiler.ll test.coral)
```

---

## 5. Phase C Evaluation Summary

### Strengths
- All 7 modules contain genuine, detailed implementations — zero stubs
- Union-find type inference (`semantic.coral`) faithfully ports the algorithm
- LLVM IR text emitter (`codegen.coral`) handles 130+ runtime function declarations
- Closure capture analysis with environment heap allocation
- Constant folding for int/float/bool/string/ternary
- 7,343 lines of Coral code covering the full compiler pipeline

### Critical Findings
- **Never executed as a binary** — all 18 tests only verify the Rust compiler can process the self-hosted source, not that the self-hosted compiler can process anything
- **4 cross-module bugs** that will cause runtime at first execution
- **Estimated effective completeness: 60-65%** when factoring in execution readiness

---

## 6. Track 1: Cross-Module Bug Fixes

These MUST be fixed before any execution attempt. Each bug will cause immediate crashes or incorrect output.

### SH-1: elif_branches Format Mismatch (Critical)

**Location:** `semantic.coral` lines 567, 807, 879, 1594 AND `codegen.coral` lines 898, 903

**Problem:** Parser emits elif_branches as a list of maps:
```coral
# parser.coral line 611
elif_branches.push(map("condition" is elif_cond, "body" is elif_body))
```

But semantic checker and codegen both index into each branch as an array:
```coral
# semantic.coral line 571-574
for eb in elifs
    result is check_expression(eb[0], ss, known_names)  # WRONG: eb is a map, not a list
    ...
    result is check_block(eb[1], ss, known_names)        # WRONG

# codegen.coral lines 898, 903
elif_cond is emit_expression(b, elifs[idx][0])   # WRONG
emit_block(b, elifs[idx][1])                       # WRONG
```

**Fix:** Change all sites in `semantic.coral` (4 locations) and `codegen.coral` (2 locations) to use map access:
```coral
# semantic.coral fix:
for eb in elifs
    result is check_expression(eb.get("condition"), ss, known_names)
    ...
    result is check_block(eb.get("body"), ss, known_names)

# codegen.coral fix:
elif_cond is emit_expression(b, elifs[idx].get("condition"))
emit_block(b, elifs[idx].get("body"))
```

**Verify:** Write a test that compiles a program with `if/elif/else` through the self-hosted compiler.

---

### SH-2: Actor Message Dispatch Field Mismatch (High)

**Location:** `codegen.coral` line 683

**Problem:** Codegen checks for actor message handlers using:
```coral
if method.get("is_message") is true
```

But the parser stores the function kind as:
```coral
# parser.coral line 119 (make_function_item)
return map("kind" is "function", ... "func_kind" is func_kind, ...)
# parser.coral line 1488 (parse_actor_message)
return make_function_item(name, params, body, "message", ...)
```

So `method.get("func_kind")` would be `"message"`, but `method.get("is_message")` returns `none`.

**Fix:** Change `codegen.coral` line 683:
```coral
# Before:
if method.get("is_message") is true
# After:
if method.get("func_kind") is "message"
```

**Verify:** Write a test with an actor that receives a message.

---

### SH-3: Error Value Field Name Mismatch (Medium)

**Location:** `codegen.coral` line 2081

**Problem:** Codegen reads:
```coral
name is expr.get("name")
```

But parser stores error values with:
```coral
# parser.coral line 79
return map("kind" is "error_value", "path" is path, "span" is span)
```

Where `path` is a list of path segments (e.g., `["IO", "NotFound"]`).

**Fix:** Change `emit_error_value` in `codegen.coral`:
```coral
*emit_error_value(b, expr)
    path is expr.get("path")
    if path isnt none
        name is path.join(".")
    if name is none
        name is "Error"
    # ... rest unchanged
```

**Verify:** Write a test with error taxonomy usage (`error IO.NotFound`).

---

### SH-4: range() Returns Empty List (Medium)

**Location:** `codegen.coral` lines 1421-1439

**Problem:** The `range(start, end)` two-argument case creates an empty list:
```coral
r is fresh_reg(b)
emit_indent(b, '{r} = call %CoralValue* @coral_make_list(%CoralValue** null, i64 0)')
return r
```

**Fix:** Call the runtime's `coral_range` function (if it exists) or build the list via a loop in IR. Check if `coral_range` is declared in the runtime:

```bash
grep -r "coral_range" runtime/src/ src/codegen/
```

If a runtime helper exists, use it. If not, emit a loop:
```coral
*emit_range_call(b, args)
    if args.length() is 1
        zero is fresh_reg(b)
        emit_indent(b, '{zero} = call %CoralValue* @coral_make_number(double 0.0)')
        n is emit_expression(b, args[0])
        r is fresh_reg(b)
        emit_indent(b, '{r} = call %CoralValue* @coral_range(%CoralValue* {zero}, %CoralValue* {n})')
        return r
    start is emit_expression(b, args[0])
    end is emit_expression(b, args[1])
    r is fresh_reg(b)
    emit_indent(b, '{r} = call %CoralValue* @coral_range(%CoralValue* {start}, %CoralValue* {end})')
    return r
```

If `coral_range` doesn't exist in the runtime, add it or emit an inline IR loop.

**Verify:** Write a test: `for i in range(5)` → should print 0,1,2,3,4.

---

## 7. Track 2: First Execution

Goal: Get the self-hosted compiler to successfully compile and run a trivial program.

### D-2.1: Compile Self-Hosted Compiler to Binary (est. 4h)

1. Use the Rust compiler to compile `self_hosted/compiler.coral` to LLVM IR
2. Use `lli -load libruntime.so` to load and execute the self-hosted compiler
3. Pass a trivial program as input (e.g., `log("hello")`)
4. Capture output `.ll` file

**Expected failure:** This will likely crash due to missing runtime functions or data format issues. Debug systematically.

**Command sequence:**
```bash
# Step 1: Generate LLVM IR for the self-hosted compiler
cargo run -- self_hosted/compiler.coral -o target/tmp/self_hosted.ll

# Step 2: Attempt execution  
echo 'log("hello from self-hosted")' | lli -load target/release/libruntime.so target/tmp/self_hosted.ll

# Step 3: If step 2 produces .ll output, try compiling it
lli -load target/release/libruntime.so target/tmp/hello.ll
```

### D-2.2: Progressive Test Suite (est. 15h)

Build a suite of increasingly complex test programs to run through the self-hosted compiler. For each, compare output with the Rust compiler.

| Level | Program | Tests |
|-------|---------|-------|
| L0 | Empty program | Produces valid `.ll` with main function |
| L1 | `log("hello")` | Single function call, string constant |
| L2 | `x is 5; log(x)` | Variable binding + reference |
| L3 | `log(2 + 3)` | Arithmetic expression |
| L4 | `*add(a, b) { return a + b }; log(add(2, 3))` | Function definition + call |
| L5 | `if true { log("yes") } else { log("no") }` | Control flow |
| L6 | `for i in range(5) { log(i) }` | Loop + range |
| L7 | `items is [1, 2, 3]; for x in items { log(x) }` | List literal + iteration |
| L8 | `type Color { Red; Green; Blue }; c is Red; match c { Red -> log("red") }` | ADT + pattern matching |
| L9 | `*fib(n) { n < 2 ? n ! fib(n-1) + fib(n-2) }; log(fib(10))` | Recursion + ternary |
| L10 | Full `fizzbuzz.coral` example | Real-world program |

### D-2.3: IR Diff Testing Framework (est. 6h)

Create a test harness that:
1. Compiles a program with the Rust compiler → `expected.ll`
2. Compiles the same program with the self-hosted compiler → `actual.ll`
3. Normalizes both (strip comments, normalize register names)
4. Diffs and reports discrepancies

This becomes the primary verification mechanism for correctness.

### D-2.4: Add Execution Tests to CI (est. 4h)

Add to `tests/self_hosting.rs`:
- Tests that actually RUN the self-hosted compiler (not just compile it)
- At minimum: L0-L5 programs from D-2.2
- Assert on stdout output matching expected

---

## 8. Track 3: Bootstrap

Goal: Self-hosted compiler compiles itself, producing identical output.

### D-3.1: Self-Compile Attempt (est. 10h)

1. Compile `self_hosted/compiler.coral` → `gen1.ll` (Rust compiler)
2. Run `gen1.ll` on `self_hosted/compiler.coral` → `gen2.ll` (self-hosted compiler, generation 1)
3. Run `gen2.ll` on `self_hosted/compiler.coral` → `gen3.ll` (self-hosted compiler, generation 2)
4. Verify: `gen2.ll` ≈ `gen3.ll` (identical after normalization)

**Expected issues:**
- Self-hosted compiler uses features it can't yet compile (template strings with expressions, complex closures)
- Missing builtins or runtime functions
- String handling edge cases (escapes, interpolation)

### D-3.2: Self-Compile Feature Gap Analysis (est. 8h)

For each self-hosted module, identify Coral features it uses and verify the self-hosted compiler handles each:

| Feature | Used In | Self-Hosted Handles? |
|---------|---------|---------------------|
| Map literals | All modules | ✅ Yes |
| List literals | All modules | ✅ Yes |
| Template strings (simple) | codegen.coral | ⚠️ Partial (SH-5) |
| Closures | lower, semantic | ✅ Yes |
| For..in loops | All modules | ✅ Yes |
| Match expressions | parser, semantic | ✅ Yes |
| `isnt none` checks | All modules | Needs verification |
| Chained method calls | Various | Needs verification |
| Multi-argument functions | All modules | ✅ Yes |
| `use` imports | compiler.coral | ✅ Yes |
| Error values | semantic | ⚠️ SH-3 bug |
| Guard statements | Various | ✅ Yes |

### D-3.3: Bootstrap Verification (est. 5h)

Once gen2 ≈ gen3:
1. Run gen2 on all 7 example programs → verify output matches Rust compiler
2. Run gen2 on all stdlib modules → verify they compile
3. Record and report any discrepancies

### D-3.4: SC-10 Performance Comparison (est. 5h)

Benchmark:
- Compile time for `fizzbuzz.coral` (Rust compiler vs self-hosted)
- Compile time for `self_hosted/compiler.coral` (Rust vs self-hosted)
- Memory usage
- Target: self-hosted within 5x of Rust compiler

---

## 9. Track 4: Hardening & Edge Cases

### D-4.1: Template String Interpolation (est. 8h)

**Problem:** Self-hosted parser treats `{expr}` in template strings as simple identifier references instead of recursively parsing arbitrary expressions.

**Impact:** High — the codegen module (`codegen.coral`) makes heavy use of template strings like:
```coral
emit_indent(b, '{r} = call %CoralValue* @coral_make_number(double {val})')
```

These contain simple identifiers, so they work. But expressions like `'{a + b}'` would fail.

**Fix:** Modify `parser.coral` to re-lex and re-parse interpolation content, or verify that only simple identifier interpolations are used across the self-hosted codebase (which appears to be the case).

**Pragmatic approach:** Audit all template strings in `self_hosted/*.coral`. If all are simple identifier references, document this as a known limitation and defer full expression interpolation.

### D-4.2: Advanced Match Patterns (est. 5h)

Test and fix:
- Nested constructor patterns: `Some(Some(x))`
- List patterns in match: `[head, ...tail]`
- Negative integer patterns: `match x { -1 -> ... }`
- String patterns in match

### D-4.3: Error Recovery Improvements (est. 5h)

The self-hosted compiler currently uses single-error model. For better developer experience:
- Add `synchronize_to_item()` recovery in parser (already present)
- Collect multiple errors across all phases
- Report all errors at end instead of stopping at first

### D-4.4: Missing Construct Support (est. 8h)

Features the Rust compiler handles but the self-hosted compiler may not:
- `unsafe` blocks
- `asm` inline assembly
- `ptr` operations
- Bytes literals (`b"..."`)
- Multi-line strings (if added)

Verify each by attempting to compile programs using these features.

### D-4.5: Module Loader Upgrade (est. 8h)

Replace text-based export extraction (ML1) with AST-based approach:
1. Parse each imported module
2. Extract exported names from AST (functions, types, stores, traits)
3. Enable proper namespacing (ML2)

This improves correctness and enables `use std.io { read_file, write_file }` selective imports.

---

## 10. Execution Order

### Week 1: Bug Fixes (Track 1)

```
Day 1-2: SH-1 (elif_branches) + SH-2 (actor dispatch) + tests
Day 3:   SH-3 (error value) + SH-4 (range) + tests
Day 4-5: Rebuild, run expanded test suite, verify all 755+ tests still pass
```

**Gate:** All 4 bugs fixed, regression tests pass.

### Week 2: First Execution (Track 2, D-2.1 through D-2.2)

```
Day 1:   D-2.1 — Attempt first execution of self-hosted compiler via lli
Day 2-3: Debug crashes, fix additional runtime issues discovered
Day 4-5: D-2.2 — Progressive test levels L0-L5
```

**Gate:** Self-hosted compiler successfully compiles L0-L5 programs.

### Week 3: Full Execution + Diff Testing (Track 2, D-2.2 through D-2.4)

```
Day 1-2: Complete L6-L10 progressive tests
Day 3:   D-2.3 — Build IR diff testing framework
Day 4-5: D-2.4 — Add execution tests to CI
```

**Gate:** Self-hosted compiler passes L0-L10, diff testing framework operational.

### Week 4: Bootstrap (Track 3)

```
Day 1-2: D-3.1 — Self-compile attempt (gen1 → gen2 → gen3)
Day 3:   D-3.2 — Feature gap analysis and fixes
Day 4-5: D-3.3 — Bootstrap verification
```

**Gate:** gen2.ll ≈ gen3.ll (bootstrap achieved).

### Week 5: Hardening (Track 4)

```
Day 1-2: D-4.1 — Template string audit/fix
Day 3:   D-4.2 — Advanced match patterns
Day 4:   D-4.4 — Missing construct support
Day 5:   D-3.4 — Performance comparison, D-4.5 — Module loader upgrade (start)
```

**Gate:** SC-9 and SC-10 from roadmap fully complete.

---

## 11. Verification Checklist

### Track 1 Complete
- [ ] SH-1 fixed: if/elif/else works through self-hosted semantic analysis
- [ ] SH-2 fixed: actor message dispatch uses `func_kind` field
- [ ] SH-3 fixed: error values use `path` field
- [ ] SH-4 fixed: `range(start, end)` produces correct list
- [ ] All 755+ existing tests still pass
- [ ] New regression tests added for each fix

### Track 2 Complete
- [ ] Self-hosted compiler executes via `lli` without crashes
- [ ] L0 (empty program) produces valid `.ll`
- [ ] L1-L5 (basic programs) produce correct output
- [ ] L6-L10 (complex programs) produce correct output
- [ ] IR diff framework operational
- [ ] At least 10 execution tests in CI

### Track 3 Complete
- [ ] gen1 → gen2 → gen3 chain succeeds
- [ ] gen2.ll ≈ gen3.ll (bootstrap verified)
- [ ] Self-hosted compiler compiles all 7 examples correctly
- [ ] Self-hosted compiler compiles all stdlib modules
- [ ] Performance within 5x of Rust compiler

### Track 4 Complete
- [ ] Template string audit complete — all interpolations verified
- [ ] Advanced match patterns tested
- [ ] Error recovery collects multiple errors
- [ ] Missing constructs documented or implemented

### Phase D Complete (SC-9 + SC-10)
- [ ] Bootstrap chain verified (gen2 ≈ gen3)
- [ ] Performance benchmarked and within target
- [ ] 20+ new self-hosting tests in CI
- [ ] SELF_HOSTING_STATUS.md updated with execution-verified status
- [ ] ALPHA_ROADMAP.md Phase D marked complete

---

## Appendix A: Key Runtime Functions

The self-hosted codegen emits calls to these runtime functions. All must be available in `libruntime.so`:

**Value constructors:** `coral_make_number`, `coral_make_bool`, `coral_make_string`, `coral_make_unit`, `coral_make_none`, `coral_make_list`, `coral_make_map`, `coral_make_tagged`, `coral_make_error`

**Value accessors:** `coral_value_as_f64`, `coral_value_as_bool`, `coral_value_as_string_ptr`, `coral_value_as_string_len`, `coral_value_tag`

**Operations:** `coral_value_add`, `coral_value_equals`, `coral_value_not_equals`, `coral_value_less`, `coral_value_greater`, `coral_value_length`

**Collections:** `coral_list_push`, `coral_list_get_index`, `coral_list_len`, `coral_map_set`, `coral_map_get`, `coral_map_keys`

**Control:** `coral_value_iter`, `coral_value_iter_next`, `coral_value_retain`, `coral_value_release`

**I/O:** `coral_log`, `coral_print`, `fs_read`, `fs_write`, `fs_exists`

**Closures:** `coral_make_closure`, `coral_closure_invoke`

**Actors:** `coral_actor_spawn`, `coral_actor_send`, `coral_actor_self`

Verify all are declared in `src/codegen/runtime.rs` and exported from `runtime/src/lib.rs`.

---

## Appendix B: Self-Hosted AST Format

The self-hosted compiler represents AST nodes as maps. Key patterns:

```coral
# Expression node
map("kind" is "integer", "value" is 42, "span" is span)
map("kind" is "binary", "op" is "+", "left" is expr, "right" is expr, "span" is span)
map("kind" is "call", "callee" is expr, "args" is [expr, ...], "span" is span)

# Statement node
map("kind" is "binding", "name" is "x", "value" is expr, "span" is span)
map("kind" is "if", "condition" is expr, "body" is stmts, "elif_branches" is [...], "else_body" is stmts)

# Item node
map("kind" is "function", "name" is "foo", "params" is [...], "body" is stmts, "func_kind" is "free"|"method"|"message")

# Token (from lexer)
map("kind" is "integer", "value" is "42", "start" is pos, "end" is pos)
```

This is the data contract between modules. Bugs SH-1 through SH-3 were all caused by one module writing a field name/format that another module reads differently. When adding new features, verify the contract at both the producer and consumer sites.

---

## Appendix C: Remaining Roadmap Bugs (Not Self-Hosting Specific)

These bugs affect the Rust compiler and may need fixing to support full self-hosted testing:

| ID | Severity | Description | Impact on Phase D |
|----|----------|-------------|-------------------|
| T2 | High | Generic instantiation faked (Option→List) | Low — self-hosted uses dynamic typing |
| P6 | Medium | Single-error model | Low — affects DX, not correctness |
| S6 | Medium | Member access falls back to Map constraint | Low — maps work for self-hosted AST |
| S8 | Medium | Pipeline type inference discards left type | Low — pipelines used sparingly |
| R11 | Medium | Single work queue contention | None — no actors in compiler |
| ML1 | Medium | Text-based export extraction | Medium — affects module loader accuracy |
| ML2 | Medium | No proper namespacing | Medium — may cause name collisions |
