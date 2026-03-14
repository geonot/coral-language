# Coral Compiler Parity Matrix — CC1.1

**Status:** Active tracking document  
**Last Updated:** March 2026

## Feature Matrix: Rust Compiler vs Self-Hosted Coral Compiler

### Legend
- ✅ Fully implemented and tested
- 🔶 Partially implemented
- ❌ Not yet implemented
- N/A Not applicable

### Lexer

| Feature | Rust | Self-Hosted | Notes |
|---------|:----:|:-----------:|-------|
| Keywords | ✅ | ✅ | All 40+ keywords |
| String literals | ✅ | ✅ | Including escape sequences |
| Number literals | ✅ | ✅ | Integer and float |
| Operators | ✅ | ✅ | All binary/unary ops |
| Line comments | ✅ | ✅ | `-- comment` |
| Interpolated strings | ✅ | ✅ | `"hello {name}"` |
| Token spans | ✅ | ✅ | Line:column tracking |

### Parser

| Feature | Rust | Self-Hosted | Notes |
|---------|:----:|:-----------:|-------|
| Function declarations | ✅ | ✅ | `*name(params)` |
| Binding (`is`) | ✅ | ✅ | `x is 42` |
| Match expressions | ✅ | ✅ | With guards and or-patterns |
| Pipeline operator | ✅ | ✅ | `expr ~ fn()` |
| Ternary | ✅ | ✅ | `cond ? a ! b` |
| Error handling | ✅ | ✅ | `err`, `!` propagation |
| Store declarations | ✅ | ✅ | Fields and methods |
| Enum/ADT | ✅ | ✅ | `type Name is Variant(T)` |
| Trait declarations | ✅ | ✅ | `trait Name` |
| Trait implementations | ✅ | ✅ | `impl Trait for Type` |
| Generics | ✅ | ✅ | `type Foo[T]` |
| do..end blocks | ✅ | ✅ | Trailing lambda |
| List comprehensions | ✅ | 🔶 | Self-hosted may need update |
| Map comprehensions | ✅ | 🔶 | Self-hosted may need update |
| Destructuring | ✅ | 🔶 | Partial support |
| Slice syntax | ✅ | 🔶 | Needs verification |
| Const generics | ✅ | ❌ | New in Sprint 9 |
| Module system | ✅ | ✅ | import/export |

### Semantic Analysis

| Feature | Rust | Self-Hosted | Notes |
|---------|:----:|:-----------:|-------|
| Type inference | ✅ | ✅ | Hindley-Milner style |
| Scope checking | ✅ | ✅ | CC1.3: no relaxations found |
| Mutability checking | ✅ | ✅ | `mut` bindings |
| Purity analysis | ✅ | ❌ | New in Sprint 9 |
| Escape analysis | ✅ | ❌ | New in Sprint 7 |
| Trait resolution | ✅ | 🔶 | |
| Monomorphization | ✅ | ❌ | New in Sprint 7 |
| Dead code detection | ✅ | ❌ | |

### Code Generation

| Feature | Rust | Self-Hosted | Notes |
|---------|:----:|:-----------:|-------|
| LLVM IR output | ✅ | ✅ | Via Inkwell |
| Function codegen | ✅ | ✅ | Including closures |
| Store codegen | ✅ | ✅ | |
| Actor codegen | ✅ | ✅ | spawn/send/receive |
| Constant folding | ✅ | ❌ | New comptime system |
| Lambda inlining | ✅ | ❌ | Sprint 7 |
| Unboxed list specialization | ✅ | ❌ | Sprint 7 |
| Stack allocation hints | ✅ | ❌ | Sprint 7 |
| Region allocation | ✅ | ❌ | Sprint 9 |

### Standard Library

| Feature | Rust | Self-Hosted | Notes |
|---------|:----:|:-----------:|-------|
| std.io | ✅ | ✅ | |
| std.math | ✅ | ✅ | |
| std.string | ✅ | ✅ | |
| std.list | ✅ | ✅ | |
| std.map | ✅ | ✅ | |
| std.json | ✅ | ✅ | |
| std.net | ✅ | ✅ | |
| std.crypto | ✅ | ✅ | |
| std.collections | ✅ | ❌ | Sprint 7 |
| std.time | ✅ | ✅ | |

## CC1.3 — Self-Hosted Relaxation Status

**Finding:** No explicit relaxations, workarounds, or skip-checks found in
`self_hosted/semantic.coral` or `self_hosted/parser.coral`. The self-hosted
compiler applies full scope-checking and type-checking, matching the Rust
compiler's strictness. Boolean constraint relaxations are not present.

**Status:** ✅ Complete — no relaxations to remove.

## CC1.4 — Performance Comparison

See `benchmarks/compiler_comparison.sh` for the benchmark script.

### Expected Comparison Axes:
1. **Compilation speed** — time to compile identical programs
2. **Binary size** — emitted IR / object code size
3. **Runtime performance** — execution speed of compiled programs
4. **Memory usage** — peak RSS during compilation
