# Coral Production Readiness: Comprehensive Issues & Improvements

> Generated from systematic review of the full codebase (82,600 lines across
> compiler, runtime, tests, self-hosted compiler, and standard library).
>
> **Test baseline**: 1,118 compiler + 238 runtime + 38 self-hosting = 1,394 tests, 0 failures
> **Benchmark suite**: 12 benchmarks covering arithmetic, loops, lists, strings, maps, closures,
> pattern matching, for-in, recursion, stores, and math operations

---

## Table of Contents

1. [Critical Issues (P0)](#1-critical-issues-p0)
2. [Major Issues (P1)](#2-major-issues-p1)
3. [Moderate Issues (P2)](#3-moderate-issues-p2)
4. [Minor Issues & Enhancements (P3)](#4-minor-issues--enhancements-p3)
5. [Missing Features for Production](#5-missing-features-for-production)
6. [Performance Observations](#6-performance-observations)
7. [Tooling & Ecosystem Gaps](#7-tooling--ecosystem-gaps)

---

## 1. Critical Issues (P0)

These MUST be fixed before any production use.

### P0-1: Captured Closure Segfault in Binary Mode (Codegen/Runtime)

**Reproducer**: A captured closure called 500K+ times in a loop causes a segfault
in `--emit-binary` mode:

```coral
*main()
    mult is 3
    f is *fn(x) x * mult
    sum is 0
    i is 0
    while i < 500000
        sum is sum + f(i % 100)
        i is i + 1
    log(sum)
```

**Impact**: Silent crash. Any non-trivial program using closures with captures is
at risk of data corruption or crash.

**Root cause**: Likely reference counting issue in closure environment — the captured
variable's refcount is not correctly managed after many invocations, leading to
use-after-free.

**Files**: `src/codegen/closures.rs`, `runtime/src/rc_ops.rs`

---

### P0-2: Nested Match Expression Generates Invalid LLVM IR (Codegen)

**Reproducer**: Nested `match` expressions produce LLVM basic blocks that the
LLVM verifier rejects:

```
llc: error: expected instruction opcode
match_arm_011:    ; preds = %match_default
```

**Impact**: Programs using nested match cannot compile to binaries. The IR label
naming/branching creates invalid predecessor chains.

**Files**: `src/codegen/mod.rs` (match emission), `src/codegen/match_adt.rs`

---

### P0-3: Path Traversal in Module Resolution (Security)

**Location**: `src/module_loader.rs:622`

Module names like `../../etc/passwd` are not validated before being resolved
relative to the current file. A malicious import can read arbitrary files.

**Fix**: Canonicalize resolved path and verify it falls within the project root
or standard library paths.

---

### P0-4: Path Traversal in Package Dependencies (Security)

**Location**: `src/package.rs:43-44`

`DepSpec.path` accepts arbitrary relative paths without validation. A malicious
`coral.toml` can reference files outside the project.

**Fix**: Validate that resolved dependency paths don't escape the project root.

---

### P0-5: Race Condition in Multi-threaded Reference Release (Runtime)

**Location**: `runtime/src/rc_ops.rs:95-155`

Two threads can both observe `refcount == 1` and both attempt deallocation.
The compare-exchange prevents double-increment but not double-decrement on
final release.

**Impact**: Use-after-free in concurrent programs using actors.

**Fix**: Use generation counters or ensure deallocation is protected by
a more robust CAS pattern that prevents both threads from entering the
deallocation path.

---

### P0-6: Debug eprintln in Production Parser Code

**Location**: `src/parser.rs:2470-2479`

`eprintln!("=== TOKEN DUMP ===")` fires every time an unexpected token is
encountered. This spams stderr in production programs.

**Fix**: Remove or gate behind a `--debug-tokens` flag.

---

### P0-7: noalias Attribute on Non-Pointer Parameters [FIXED THIS SESSION]

**Location**: `src/codegen/mod.rs:786-791`

LLVM's `noalias` attribute was applied to ALL function parameters, including
`i64` NaN-boxed values. `noalias` is only valid on pointer types.

**Impact**: ALL programs with user-defined functions failed LLVM verification
when compiled to binary (`--emit-binary`).

**Status**: FIXED — now only applies `noalias` to pointer-typed parameters.

---

## 2. Major Issues (P1)

Should be fixed before production use.

### P1-1: Map Iterator Snapshot Not Deep-Copied (Runtime)

**Location**: `runtime/src/map_ops.rs:19-26`

Map iteration creates a shallow snapshot of the bucket array. If the map is
structurally modified (rehash, insert causing resize) during iteration, bucket
pointers become stale.

**Impact**: Use-after-free during map iteration with concurrent modification.

---

### P1-2: LSP Server Missing All Core IDE Features

**Location**: `coral-lsp/src/main.rs`

The LSP currently provides diagnostics only. Missing:
- Go to definition
- Hover/type information
- Autocomplete
- Find references
- Rename refactoring
- Code actions/quick fixes
- Code formatting
- Workspace symbol search

**Impact**: The language is unusable in modern editors without these features.

---

### P1-3: LSP Full Recompilation Per Keystroke

**Location**: `coral-lsp/src/main.rs:68-74`

Uses `TextDocumentSyncKind::FULL` — every keystroke triggers full module
compilation. No incremental analysis. Scales O(n²) with project size.

---

### P1-4: Type Solver Unsoundness — ADT Unification Ignores Type Arguments

**Location**: `src/types/solver.rs:483-503`

`Adt(Name, [])` unifies with `Adt(Name, [Int])` without error. Generic types
lose their type parameters during unification.

**Impact**: `Option` and `Option[Int]` are treated as the same type.

---

### P1-5: Error Types Unify With Anything

**Location**: `src/types/solver.rs:506-513`

`TypeId::Error` unifies with any type, silently suppressing real type mismatches.

---

### P1-6: Callable Constraint Allows Too Few Arguments

**Location**: `src/types/solver.rs:395-407`

Only checks `args.len() > expected`, not `args.len() < expected`. A function
`fn(A, B, C)` can be called with just `fn(A)`.

---

### P1-7: Module Export Extraction Uses Regex, Not AST

**Location**: `src/module_loader.rs:118-150`

Export detection uses string pattern matching (`starts_with('*')`) instead of
parsing. Misses multi-line signatures, type-annotated functions, and generics.

---

### P1-8: Package TOML Parser Is Hand-Written and Fragile

**Location**: `src/package.rs:30-85`

Custom regex parsing instead of using the `toml` crate. Breaks on comments,
multi-line strings, and TOML 1.0 features.

---

### P1-9: Git Dependencies Accepted But Not Implemented

**Location**: `src/package.rs:14, 74-75`

`DepSpec.git` field is parsed from the manifest but `resolve_dependencies()`
never clones. Users think git dependencies work but they silently do nothing.

---

### P1-10: Persistent Store Has No fsync (Data Loss Risk)

**Location**: `runtime/src/store/mmap.rs:162-183`

`persist()` writes data but never calls `sync_all()`. Power loss during write
can result in data corruption.

---

### P1-11: Integer Division by Zero Not Checked (Codegen)

Division operations emit LLVM `sdiv`/`udiv` without checking for zero divisor.
Results in SIGFPE on x86.

---

### P1-12: Pervasive .unwrap() in LLVM IR Building (~100+ instances)

**Location**: `src/codegen/mod.rs`, `src/codegen/builtins.rs`

LLVM builder operations use `.unwrap()` throughout. Any operational failure
panics the compiler instead of producing a diagnostic.

---

### P1-13: Circular Type Bindings Cause Infinite Loop

**Location**: `src/types/solver.rs:564-590`

No cycle detection in type variable resolution. `t1 = List[t2]; t2 = List[t1]`
causes an infinite loop.

---

### P1-14: Match Exhaustiveness Ignores Guard Expressions

**Location**: `src/semantic.rs:1900-1950`

Guards can fail, leaving unhandled cases. Exhaustiveness checking doesn't
account for fallible guard conditions.

---

### P1-15: NaN-Boxing Pointer Assertion Only in Debug

**Location**: `runtime/src/nanbox.rs:65-69`

`from_heap_ptr()` only checks that pointers fit in 48 bits via `debug_assert!`.
In release mode, addresses exceeding 48 bits silently corrupt the value.

---

### P1-16: RefCell-Based Release Queue Can Deadlock on Panic

**Location**: `runtime/src/rc_ops.rs:126-140`

Thread-local `RefCell<Option<VecDeque<...>>>` permanently breaks if a panic
occurs while the borrow is held. Subsequent accesses panic unconditionally.

---

## 3. Moderate Issues (P2)

Should be addressed for reliability and correctness.

### P2-1: Placeholder Index Collision ($0 vs bare $)

**Location**: `src/lexer.rs:856-866`

Bare `$` produces `Placeholder(0)`, colliding with `$0`. Can't distinguish
between the two at parse time.

---

### P2-2: Match Guard Syntax Mismatch — `if` vs `when`

**Location**: `src/parser.rs:1813`

Parser expects `if` for match guards, but AGENTS.md and language spec say `when`.

---

### P2-3: Template String Nesting Not Depth-Limited

**Location**: `src/lexer.rs:740-750`

No maximum depth for template interpolation nesting. A deeply nested template
can cause stack exhaustion (DoS vector).

---

### P2-4: Error Name Allows Empty Path

**Location**: `src/parser.rs:2334-2349`

`err` with no name produces `ErrorValue { path: vec![] }`, which is likely
invalid at codegen time.

---

### P2-5: No Unicode Escape Sequences in Strings

**Location**: `src/lexer.rs:636-640`

Only `\n`, `\r`, `\t`, `\0`, `\\`, `\'`, `\"` supported. No `\uHHHH` or `\u{HHHHHH}`.

---

### P2-6: Module Cache Doesn't Track File Deletion

**Location**: `src/module_loader.rs:95-110`

If a dependency file is deleted, the cache remains valid. Stale cache used
silently for deleted modules.

---

### P2-7: Comprehension Variable Scope Leaks

**Location**: `src/semantic.rs:2124-2140`

`[x | x in list]; print(x)` — variable `x` remains accessible outside the
comprehension scope.

---

### P2-8: Map Type Inference Allows Heterogeneous Values

**Location**: `src/semantic.rs:1547`

Map value type set to `Any` instead of unifying. Allows `map("a" is 1, "b" is "hello")`
without type error.

---

### P2-9: If-else Definite Assignment Tracking Incomplete

**Location**: `src/semantic.rs:2840-2890`

Variables assigned only in the if-branch (not else) become "unknown" instead
of "unassigned". `if cond; y is 5; end; print(y)` doesn't warn.

---

### P2-10: Pattern Binding Types All Become Any

**Location**: `src/semantic.rs:1236-1250`

Constructor pattern bindings assigned `Any` instead of inferring from the
constructor's type parameters.

---

### P2-11: Actor Mailbox Queue Unbounded

**Location**: `runtime/src/actor.rs` (DEFAULT_MAILBOX_CAPACITY = 1024)

No backpressure mechanism. A fast sender can overwhelm a slow receiver,
leading to OOM.

---

### P2-12: CSE Cache Invalidation Incomplete

**Location**: `src/codegen/mod.rs:1630`

CSE cache only cleared on field assignment, not on all side effects (e.g.,
list push, map set, actor send).

---

### P2-13: PGO Flags Not Validated as Mutually Exclusive

**Location**: `src/main.rs:50-51`

`--pgo-gen` and `--pgo-use` accepted together without error.

---

### P2-14: Optimization Level Not Validated

**Location**: `src/main.rs:45`

`-O LEVEL` accepts 0-255 but only 0-3 are meaningful LLVM levels.

---

### P2-15: do..end Block Accepted in Invalid Positions

**Location**: `src/parser.rs:1107-1116`

`5 do ... end` and `match x do ... end` parse successfully.

---

### P2-16: `from` Used as Contextual Keyword in Patterns

**Location**: `src/parser.rs:2024-2061`

`from` matched as an identifier, not a keyword. A variable named `from`
will be misinterpreted in pattern matching context.

---

### P2-17: No Duplicate Type Parameter Detection

**Location**: `src/parser.rs:435-450`

`type Foo[T, T]` parses successfully without error.

---

### P2-18: 400+ Line Hardcoded Builtin Names List

**Location**: `src/semantic.rs:2247-2500`

Builtin function names manually listed. Misses dynamically added names from
extensions.

---

### P2-19: Trait Bounds Not Enforced on Type Parameters

**Location**: `src/semantic.rs:130-170`

Generic `T : Comparable` can be instantiated with `Function` type.

---

### P2-20: Actor Timer Cancellation Race

**Location**: `runtime/src/actor.rs:380-410`

Cancelled timers can fire one more time due to TOCTOU race between checking
the cancelled flag and scheduling the next repeat.

---

## 4. Minor Issues & Enhancements (P3)

### P3-1: TokenKind Clone Overhead
Parser frequently clones `TokenKind` (which contains `String`). Should use
references for comparisons.

### P3-2: No Overflow-Specific Error for Numeric Literals
`src/lexer.rs:301-309` — "invalid hex literal" for all errors including overflow.

### P3-3: Exhaustiveness Checking is O(n³)
`src/semantic.rs:1980-2060` — Triple nested loops over constructors/fields.

### P3-4: Type Environment Uses Linear Scope Traversal
`src/types/env.rs:360-380` — Every identifier lookup traverses all scopes.

### P3-5: Diagnostic Severity Levels Insufficient
Only Error/Warning/Info. No Hint/Suggestion for IDE integration.

### P3-6: Doc Generator Uses Line Patterns Instead of AST
`src/doc_gen.rs:30-70` — Fails on multi-line function signatures.

### P3-7: Lockfile Not Read for Reproducibility
`src/package.rs:150-160` — Lockfile generated but never consumed.

### P3-8: Prelude Always Included, Not Optional
`src/module_loader.rs:179-195` — No way to opt out for minimal builds.

### P3-9: String UTF-8 Not Validated at FFI Boundaries
External strings accepted without validation.

### P3-10: Overlapping Match Patterns Not Detected
`match x / 1 ? ... / 1 ? ...` — unreachable arm not warned.

### P3-11: Missing Indentation in dedent error messages
`src/parser.rs:1233-1241` — EOF dedent errors report wrong span.

### P3-12: Unused Build Warnings (~10 items)
Unused imports, variables, constants, and functions in `src/compiler.rs`,
`src/semantic.rs`, `src/doc_gen.rs`, `src/main.rs`.

---

## 5. Missing Features for Production

### Language Features

| Feature | Status | Priority |
|---------|--------|----------|
| Async/await | Not implemented | HIGH |
| Generics (full inference) | Partial — type args often become Any | HIGH |
| Interface/Protocol types | Traits exist but no polymorphic dispatch | HIGH |
| Pattern matching on strings | Only ADT and integer literals | MEDIUM |
| Destructuring assignment | Only in match arms | MEDIUM |
| Default parameter values | Not supported | MEDIUM |
| Named arguments | Not supported | MEDIUM |
| Variadic functions | Not supported | LOW |
| Operator overloading | Not supported | LOW |

### Runtime Features

| Feature | Status | Priority |
|---------|--------|----------|
| Garbage collection (beyond RC) | None — cycle collector only | HIGH |
| Stack overflow detection | None | HIGH |
| Signal handling | None | MEDIUM |
| Profiling / tracing hooks | Metrics flag exists but incomplete | MEDIUM |
| Debugging support (DWARF) | None — no debug info in binaries | HIGH |
| Runtime type reflection | `type_of()` only — no introspection | MEDIUM |

### Tooling

| Feature | Status | Priority |
|---------|--------|----------|
| Formatter (coral fmt) | Not implemented | HIGH |
| Linter (coral lint) | Not implemented | MEDIUM |
| Package registry | Not implemented | HIGH |
| REPL | Not implemented | MEDIUM |
| Test framework | Basic — no assertions, setup/teardown | HIGH |
| Debugger integration | None | HIGH |
| Profiler | None | MEDIUM |
| Cross-compilation | Limited — host target only | MEDIUM |

### Standard Library

| Module | Status | Gaps |
|--------|--------|------|
| std/io | Spec only | File I/O not wired to runtime |
| std/net | Spec only | TCP/UDP not complete |
| std/crypto | Spec only | Not implemented |
| std/time | Partial | `time_now()` works, formatting limited |
| std/json | Partial | Parse/serialize work |
| std/collections | Spec only | Set, deque not implemented |
| std/testing | Spec only | No test runner integration |
| std/process | Spec only | No subprocess support |
| std/path | Spec only | Not implemented |
| std/url | Spec only | Not implemented |

---

## 6. Performance Observations

### Benchmark Results (Release Binary, 3 runs, median)

| Benchmark | Time (ms) | Notes |
|-----------|-----------|-------|
| fibonacci(30) | 249 | Recursive — ~200μs per call |
| tight_loop (10M) | 1,217 | ~122ns per NaN-boxed add |
| list_ops (100K) | 593 | map/filter/reduce pipeline |
| string_ops (10K) | 33 | Concat/split/replace |
| matrix_mul (50K×3×3) | 4,945 | Heavy nested indexing |
| map_ops (10K) | 82 | HashMap insert/lookup |
| closures (500K calls) | 251 | Lambda + HOF pipeline |
| pattern_matching (500K) | 6,766 | If/elif + match + ternary |
| for_iteration (50K) | 443 | For-in loops |
| recursion (Ackermann+) | 633 | Deep recursion |
| store_ops (100K) | 7,153 | Store create/method/field |
| math_compute (1M) | 1,508 | Float + trig + sqrt |
| **TOTAL** | **23,873** | |

### Performance Concerns

1. **NaN-boxing overhead**: ~122ns per integer add in tight loop is ~100x slower
   than native. The boxing/unboxing on every operation dominates.

2. **Store operations very slow**: 7s for 300K store operations. Each `make_Store()`
   allocates a heap object + map for fields. Consider struct-of-arrays or
   fixed-layout optimization.

3. **Pattern matching slow**: 6.7s for 500K classifications. String comparison in
   match arms is expensive. Consider string interning or tag-based dispatch.

4. **Matrix multiply**: 5s for 50K 3×3 multiplies. Nested `.get()` calls go through
   the NaN-boxing layer each time. Consider unboxed array optimization.

5. **No JIT tiering**: Binary compilation is one-shot. No adaptive optimization
   for hot loops.

---

## 7. Tooling & Ecosystem Gaps

### Build System
- No `coral build` command — must use `cargo run -- file.coral`
- No `coral test` command — no test runner
- No `coral run` command — must specify `--jit` explicitly
- No `coral init` project scaffolding (flag exists but is broken)

### Editor Support
- VS Code extension exists (`vscode-coral/`) but LSP is minimal
- Tree-sitter grammar exists (`tree-sitter-coral/`) but not validated
- No syntax highlighting for other editors (vim, neovim, emacs)

### Documentation
- No language specification document
- No tutorial or getting-started guide
- Standard library docs are spec files, not rendered documentation
- No API reference generation pipeline

### Testing Infrastructure
- No built-in assertion functions (`assert`, `assert_eq`)
- No test discovery or runner
- No test fixtures, setup/teardown
- No code coverage tool
- Fuzz testing exists (`fuzz/`) but not integrated into CI

### Distribution
- No pre-built binaries
- No package download mechanism
- No cross-compilation support
- No homebrew/apt/cargo-install formula

---

## Summary

| Category | Critical | Major | Moderate | Minor |
|----------|----------|-------|----------|-------|
| **Codegen** | 2 | 3 | 2 | 2 |
| **Runtime** | 2 | 4 | 3 | 1 |
| **Parser/Lexer** | 1 | 0 | 8 | 3 |
| **Semantic/Types** | 0 | 5 | 7 | 3 |
| **Security** | 2 | 0 | 0 | 0 |
| **Tooling/LSP** | 0 | 2 | 2 | 2 |
| **Package/Modules** | 0 | 2 | 2 | 2 |
| **TOTAL** | **7** | **16** | **24** | **13** |

### Recommended Fix Order

1. **P0-1**: Captured closure segfault (blocks real applications)
2. **P0-2**: Nested match IR generation (blocks language features)
3. **P0-3, P0-4**: Path traversal vulnerabilities (security)
4. **P0-5**: RC race condition (blocks actor system)
5. **P0-6**: Debug eprintln removal (trivial)
6. **P1-1 through P1-5**: Type system soundness
7. **P1-2, P1-3**: LSP improvements (developer experience)
8. **P1-11**: Division by zero checking
9. Remaining P1 items
10. P2 items by domain
