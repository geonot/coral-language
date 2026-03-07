# Coral — Self-Hosted Compiler Status

**Last updated:** March 2026

## Overview

The self-hosted compiler has **all 7 modules implemented** and compiling to LLVM IR through the Rust-based `coralc` compiler, both individually and as a combined pipeline. Phase C is complete — code is written and passes compilation. **However, the self-hosted compiler has never been executed as a standalone binary, and several cross-module data-format bugs have been identified.**

| File | Lines | Status | Description |
|------|-------|--------|-------------|
| `lexer.coral` | 528 | **Compiles to LLVM IR** | Indent-aware tokenizer |
| `parser.coral` | 1,800 | **Compiles to LLVM IR** | Recursive descent parser, 25+ AST node types |
| `lower.coral` | 665 | **Compiles to LLVM IR** | AST desugaring (pipelines, guards, defaults) |
| `module_loader.coral` | 284 | **Compiles to LLVM IR** | `use` directive resolution and file merging |
| `semantic.coral` | 1,677 | **Compiles to LLVM IR** | Type inference, constraint solving, scope analysis |
| `codegen.coral` | 2,109 | **Compiles to LLVM IR** | LLVM IR text emission |
| `compiler.coral` | 280 | **Compiles to LLVM IR** | Pipeline orchestrator (lex → parse → lower → analyze → fold → generate) |
| **Total** | **7,343** | | |

**Self-hosted code / Rust reference code:** 7,343 / 16,143 lines (45% by line count; Coral is terser due to no type annotations).

**Overall self-hosted compiler completeness: ~75-80%** — all phases structurally implemented but never executed end-to-end. Known cross-module bugs and missing edge cases remain.

**All 18 self-hosting tests pass** (`tests/self_hosting.rs`).

---

## Phase History

| Phase | Scope | Status |
|-------|-------|--------|
| Phase A | Lexer + Parser | **Complete** |
| Phase B | Lower + Module Loader | **Complete** |
| Phase C | Semantic + Codegen + Compiler | **Complete** (code written, not execution-verified) |
| Phase D | Bootstrap (compile self with self) | Not started |

---

## Phase C Critical Evaluation

### Strengths
- All 7 modules contain genuine, functional logic — no stubs or placeholder code
- Compiler pipeline (lex → parse → lower → module load → semantic → codegen) is architecturally complete
- Union-find type inference in `semantic.coral` is a faithful port of the algorithm
- LLVM IR text emitter in `codegen.coral` handles 130+ runtime function declarations
- Constant folding pass in `compiler.coral` handles int/float/bool/string folding
- Closure capture analysis with `find_captures`/`collect_free_vars` implemented
- 18 tests verify each module loads and compiles to IR through the Rust compiler

### Known Bugs (Must Fix Before Bootstrap)

| ID | Module | Severity | Description |
|----|--------|----------|-------------|
| SH-1 | semantic ↔ parser | **High** | `elif_branches` format mismatch: parser emits `[map("condition"→..., "body"→...)]` but semantic accesses `eb[0]`, `eb[1]` (array indexing into a map). Will crash on any if/elif. |
| SH-2 | codegen ↔ parser | **High** | Actor message dispatch: codegen checks `method.get("is_message")` but parser stores `func_kind: "message"`. Actor message handlers will never dispatch. |
| SH-3 | codegen ↔ parser | **Medium** | Error value field name: codegen reads `expr.get("name")` but parser stores `"path"` (list of segments). Error values will be miscompiled. |
| SH-4 | codegen | **Medium** | `range(start, end)` returns empty list instead of populated range. Any range-based iteration produces no output. |
| SH-5 | parser | **Low** | Template string interpolation simplified — `{expr}` treated as identifier reference, not recursively parsed. Limits interpolation to simple variables. |
| SH-6 | codegen | **Low** | No `unsafe`/`asm`/`ptr` support — blocks low-level code compilation. |

### Test Quality Assessment

| Category | Count | Description |
|----------|-------|-------------|
| Meaningful (compile-to-IR) | 8 | Full Rust pipeline on self-hosted sources, verify non-trivial IR |
| Content/existence checks | 9 | Verify source contains expected function names |
| Debug utility | 1 | Dumps expanded sources (not a test) |
| **Execution tests** | **0** | **No test runs the self-hosted compiler as a binary** |

### Honest Completeness Assessment

The "85-90%" figure previously cited measures *code written*. A more accurate assessment factoring in execution readiness:

- **Code coverage:** ~85% — all phases implemented, most constructs handled
- **Execution readiness:** ~0% — never run as a binary, cross-module bugs undetected
- **Effective completeness:** ~60-65% — significant bug-fixing and execution testing needed before bootstrap

---

## Module Details

### Lexer (`lexer.coral`) — 528 lines

- Indent-aware tokenization with INDENT/DEDENT/NEWLINE layout tokens
- Number lexing (decimal, hex, binary, octal, float, underscore separators)
- String lexing with escape sequences
- 30+ keywords recognized
- All operators (single/multi-char)
- Placeholder and comment support

### Parser (`parser.coral`) — 1,800 lines

- Full expression precedence chain (pipeline → ternary → or → and → comparison → arithmetic → unary → postfix → primary)
- 25+ AST node constructors (expressions, statements, items)
- Store/type/trait/actor definitions
- Match expressions with patterns
- Lambda expressions, list/map literals
- `use` import parsing

### Lower (`lower.coral`) — 665 lines

- Pipeline desugaring (`a ~ f` → `f(a)`)
- Guard statement lowering to if/throw
- Default parameter injection
- Expression/statement recursive traversal

### Module Loader (`module_loader.coral`) — 284 lines

- `use` directive discovery and resolution
- Standard library path resolution
- File content inlining (textual inclusion)
- Circular import detection

### Semantic (`semantic.coral`) — 1,677 lines

- Type graph (union-find) for type variables
- Constraint generation from expressions, statements, and function signatures
- Constraint solving (equality, numeric, boolean, callable, iterable)
- Scope stack with lexical scoping
- Function registry for cross-function type checking
- Store field type tracking and method validation
- Trait validation and exhaustiveness checking
- ADT variant resolution

### Codegen (`codegen.coral`) — 2,109 lines

- LLVM IR text emission targeting the Coral runtime
- Function definitions with parameter handling
- Expression codegen: arithmetic, comparison, logic, string interpolation
- Control flow: if/elif/else, while, for, match, guard, break/continue
- Store/actor construction and field access
- List and map literal emission
- Closure capture analysis and lambda emission
- Pipeline codegen (desugar to call)
- Global variable initialization
- String constant deduplication

### Compiler (`compiler.coral`) — 280 lines

- Pipeline: `lex → parse → lower → analyze → fold_constants → generate`
- Constant folding optimization pass
- Error propagation across phases
- File-based compilation entry point

---

## Regression Tests

18 self-hosting tests in `tests/self_hosting.rs`:

- 7 **load tests** — verify each module's source loads without parse errors
- 7 **compile tests** — verify each module compiles to LLVM IR
- 1 **combined compile test** — verify `compiler.coral` (which `use`s all others) compiles as a unit
- 1 **parser coverage test** — verify parser handles all AST node types
- 2 **stdlib tests** — verify standard library modules compile

---

## What's Needed to Complete Self-Hosting

See `PHASE_D_GUIDE.md` for the full execution plan.

### Phase D: Bootstrap (~80-120 hours)

**Track 1: Bug Fixes (est. 15-25h)**
1. Fix SH-1 through SH-4 (cross-module data format bugs)
2. Add execution smoke tests for each fix

**Track 2: Execution Verification (est. 25-35h)**
1. Compile self-hosted compiler to native binary via `lli`/`llc+clang` with `libcoral_runtime`
2. Run self-hosted compiler on trivial programs, then progressively complex ones
3. Compare LLVM IR output between self-hosted and Rust compilers

**Track 3: Self-Compile (est. 20-30h)**
1. Self-hosted compiler compiles itself
2. Second-generation output matches first-generation
3. Full bootstrap chain verified

**Track 4: Edge Cases & Hardening (est. 20-30h)**
1. Template string interpolation (full expression parsing)
2. Advanced match patterns
3. Error recovery improvements
4. Performance comparison (target: within 5x of Rust compiler)

### Known Gaps

| Feature | Impact | Blocking Bootstrap? | Notes |
|---------|--------|---------------------|-------|
| Cross-module data format bugs (SH-1..SH-4) | **Critical** | **Yes** | Will cause runtime crashes |
| Template string interpolation | Medium | Partial | Self-hosted lexer/parser use template strings |
| Tab/space mixing detection | Low | No | Not enforced |
| Multi-line strings | Low | No | Not commonly used |
| Advanced match patterns (nested, tuple) | Low | No | Basic patterns work |
| Error recovery | Low | No | Single-error model (stops at first error) |
| `unsafe`/`asm`/`ptr` codegen | Low | No | Not used in compiler itself |
