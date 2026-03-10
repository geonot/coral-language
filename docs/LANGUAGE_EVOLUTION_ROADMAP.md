# Coral Language Evolution Roadmap

**Created:** March 8, 2026  
**Scope:** Post-Bootstrap → Production-Ready Language  
**Goal:** Transform Coral from a bootstrapped proof-of-concept into a language that delivers on its founding promise: *reads like Python, runs like C, scales like Erlang*.

---

## Preamble

Coral has reached an extraordinary milestone. The self-hosted compiler bootstraps — gen2 equals gen3 byte-for-byte. 920 tests pass with zero failures. The language compiles itself. NaN-boxing has been implemented, transforming the value representation from 40-byte heap-allocated structs to 64-bit immediate values. Type specialization generates native `fadd`/`fcmp` instructions for numeric hot paths. Pattern matching supports guards, or-patterns, and multi-statement arms. The compiler performs dead function elimination, common subexpression elimination, tail call optimization, and small function inlining. Named arguments, default parameters, `unless`/`until`/`loop`/`when` syntax sugar, dead code detection, AST-level modules with namespacing and selective imports, and an LSP server with real-time diagnostics have all been implemented.

Phase Alpha is **complete**. Phase Beta is **complete**. Early Phase Gamma/Delta items are in progress. This is the proof that Coral *works* — and increasingly, works *fast*.

But working is not enough. To achieve the audacious goal of becoming the language that Rust or Zig could have been — a language that combines low-level systems power with high-level expressiveness — Coral must evolve across six foundational pillars:

1. **Memory Management** — From universal boxing to intelligent value representation
2. **Type Intelligence** — From permissive inference to powerful, precise type reasoning
3. **Compiler Optimizations** — From runtime-delegated execution to compile-time brilliance
4. **Syntax & Expressiveness** — From functional-but-rough to conversational elegance
5. **Runtime Performance** — From safe-but-slow to competitive-with-C
6. **Standard Library & Ecosystem** — From populated stubs to production-grade tools

Each pillar contains a phased task list. Every task must be implemented in **both** the Rust reference compiler and the Coral self-hosted compiler unless noted otherwise. This dual-implementation discipline is not overhead — it is Coral's proof of capability.

---

## Table of Contents

- [Pillar 1: Memory Management Revolution](#pillar-1-memory-management-revolution)
- [Pillar 2: Type Intelligence & Inference](#pillar-2-type-intelligence--inference)
- [Pillar 3: Compiler Intelligence & Comptime Optimizations](#pillar-3-compiler-intelligence--comptime-optimizations)
- [Pillar 4: Syntax Refinement & Expressiveness](#pillar-4-syntax-refinement--expressiveness)
- [Pillar 5: Runtime Performance Engineering](#pillar-5-runtime-performance-engineering)
- [Pillar 6: Standard Library & Ecosystem](#pillar-6-standard-library--ecosystem)
- [Cross-Cutting Concerns](#cross-cutting-concerns)
- [Implementation Phases](#implementation-phases)
- [Success Metrics](#success-metrics)

---

## Pillar 1: Memory Management Revolution

**Current State:** Every value — even a simple integer `42` or boolean `true` — is heap-allocated as a 40-byte `Value` struct with an `AtomicU64` refcount. This means `x + 1` in a loop allocates, tag-checks, unboxes, adds, boxes, and deallocates on every iteration. The cycle detector holds a global `Mutex` checked on every container release. Coral currently has *Python-tier* memory characteristics hidden behind LLVM compilation.

**Target State:** Primitive values (numbers, booleans, unit, small integers) are immediate/unboxed — zero allocation, zero refcounting, zero GC pressure. Containers use non-atomic refcounting on the fast path with atomic promotion only at actor boundaries. The cycle collector is incremental and generational. Memory-sensitive code approaches C performance.

### Phase M1: NaN-Boxing for Immediates
*The single highest-leverage optimization in the entire language.*

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| M1.1 | **Design NaN-box encoding scheme** | Define 64-bit representation: IEEE 754 doubles pass through directly; quiet-NaN payloads encode Bool (1 bit), Unit (1 tag), None (1 tag), small integers (48-bit), and heap pointers (48-bit, tag in upper bits). Document the encoding table exhaustively. | Medium | Spec only |
| M1.2 | **Implement `CoralImmediate` type in runtime** | A `u64` wrapper with methods: `is_number()`, `is_bool()`, `is_heap_ptr()`, `as_f64()`, `as_bool()`, `as_ptr()`, `from_f64()`, `from_bool()`, `from_unit()`. All operations branchless via bit manipulation. | High | Rust first |
| M1.3 | **Migrate `coral_make_number` / `coral_make_bool` / `coral_make_unit`** | These FFI functions now return immediate `u64` values instead of heap-allocated `Value*`. The NaN-boxed representation means no allocation for ~60% of values in typical programs. | High | Rust |
| M1.4 | **Update `coral_value_retain` / `coral_value_release`** | Add fast-path: if the value is an immediate (NaN-box check), return immediately — no refcount work. Only heap pointers proceed to the atomic refcount path. | Medium | Rust |
| M1.5 | **Update all arithmetic FFI** | `coral_value_add`, `coral_value_sub`, `coral_value_mul`, etc. check both operands for NaN-boxed numbers and fast-path to direct `f64` arithmetic, returning a NaN-boxed result. No allocation for number→number operations. | High | Rust |
| M1.6 | **Update comparison FFI** | `coral_value_equals`, `coral_value_less_than`, etc. fast-path for immediate-vs-immediate comparison. Boolean results returned as NaN-boxed immediates. | Medium | Rust |
| M1.7 | **Update codegen for NaN-box calling convention** | LLVM IR changes from `%CoralValue*` (pointer) to `i64` for all value-typed arguments and returns. All `call` instructions, PHI nodes, allocas, and function signatures must be updated. This is a sweeping codegen change. | Very High | Both |
| M1.8 | **Benchmark: NaN-box speedup measurement** | Build a benchmark suite (fibonacci, matrix multiply, tight loops, string processing, list operations) and measure before/after. Target: **5-10x** improvement on numeric code, **2-3x** on mixed code. | Medium | Both |

**Expected Impact:** Eliminates ~60% of heap allocations. Numeric loops approach native speed. Boolean logic becomes zero-allocation. Function call overhead drops dramatically (passing `i64` vs `*mut Value`).

### Phase M2: Non-Atomic Reference Counting Fast Path

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| M2.1 | **Thread-ownership flag on heap values** | Add a `thread_id: u32` field (or use a reserved bit in the header) to track the owning thread. Values start as thread-local with non-atomic refcount operations. | Medium | Rust |
| M2.2 | **Non-atomic retain/release** | When `retain`/`release` is called from the owning thread, use plain `u64` increment/decrement instead of `AtomicU64` CAS. Single-threaded code sees ~3x refcounting speedup. | Medium | Rust |
| M2.3 | **Atomic promotion at actor boundary** | When a value is frozen for sending to an actor (the existing `FLAG_FROZEN` mechanism), promote its refcount to atomic mode. All subsequent retain/release use atomic ops. This is a one-way transition per value. | Medium | Rust |
| M2.4 | **Remove diagnostic refcount counters from release builds** | `retain_events` and `release_events` (8 bytes per value) should be `#[cfg(feature = "metrics")]` gated. Saves 8 bytes per value in production. | Low | Rust |

**Expected Impact:** 2-3x refcounting speedup for single-threaded code. 8 bytes saved per value in production builds. No semantic change — pure optimization.

### Phase M3: Incremental & Generational Cycle Detection

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| M3.1 | **Thread-local cycle root buffers** | Instead of a global `Mutex<HashMap>`, each thread maintains a local root buffer. Roots are merged into the global set only at collection time. Eliminates lock contention on every container release. | High | Rust |
| M3.2 | **Generational hypothesis** | Track object "age" (number of survived collections). Young objects (age 0) are collected frequently; old objects (age > threshold) are collected rarely. Most cycles involve young objects. | High | Rust |
| M3.3 | **Incremental collection** | Instead of stop-the-world collection every N releases, interleave collection work with allocation: mark a few roots per release call, spread scanning across time. Eliminates GC pauses. | Very High | Rust |
| M3.4 | **Closure cycle tracking** | Add closures to `is_container()` and implement `get_children()` for captured environments. Currently, closure↔value cycles are undetectable, causing silent memory leaks. | Medium | Rust |
| M3.5 | **Weak reference optimization** | Profile weak ref overhead (~50ns per access per CYCLE_SAFE_PATTERNS.md). Consider using epoch-based reclamation for weak ref validity checks instead of global registry lookups. | Medium | Rust |

**Expected Impact:** Eliminates cycle-detector mutex contention on the hot path. GC pauses become imperceptible. Closure cycles no longer leak.

### Phase M4: Escape Analysis & Stack Allocation

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| M4.1 | **Escape analysis pass in semantic analyzer** | Determine which values never escape their defining function: not stored in collections, not passed to other functions, not returned, not captured by closures. Mark these as `StackEligible`. | Very High | Both |
| M4.2 | **Stack-allocated values in codegen** | For `StackEligible` values, emit LLVM `alloca` instead of runtime allocation calls. Skip retain/release entirely — stack cleanup is automatic. | Very High | Both |
| M4.3 | **Copy-on-write for shared-but-rarely-mutated values** | The existing `AllocationStrategy::SharedCow` classification should trigger COW semantics: share the backing store until mutation, then copy. Useful for string slices and list views. | High | Rust first |
| M4.4 | **Region-based allocation for short-lived values** | For function-local values that don't escape, allocate from a per-function arena (bump allocator). Free the entire arena on function return. Eliminates per-value free overhead. | High | Rust first |

**Expected Impact:** Many intermediate values become zero-cost. List comprehension intermediates, temporary strings, and function-local state avoid heap allocation entirely.

---

## Pillar 2: Type Intelligence & Inference

**Current State:** The constraint-based type solver operates on a flat constraint set with two escape hatches: `Any` (explicitly untyped) and `Unknown` (inference gave up). Both unify with everything, silently accepting type errors. Store fields, ADT constructor fields, and method return types default to `Any/Unknown`. No flow-sensitive typing, no generics for user types, no type narrowing through conditionals. Member access on non-map types falls through to `Unknown`.

**Target State:** Full Hindley-Milner inference with let-polymorphism. User-defined generics. Flow-sensitive type narrowing. Store/type instances carry their field types. `Unknown` is an error, not an escape hatch. The type system catches real bugs while never requiring a single annotation.

### Phase T1: Seal the Escape Hatches

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| T1.1 | **Distinguish `Any` from `Unknown`** | `Any` means "explicitly dynamic" (e.g., heterogeneous list). `Unknown` means "inference failed." Currently both unify with everything. `Unknown` should produce a warning (and eventually an error) when it persists after solving. | Medium | Both |
| T1.2 | **Type store/type instances precisely** | Store constructors should return `TypeId::Store(name)` not `Any`. Field types should be inferred from defaults and usage. A `store Point` with `x ? 0`, `y ? 0` should know `x: Number, y: Number`. | High | Both |
| T1.3 | **Type ADT constructor fields** | Pattern matching `Circle(r) ? r * r` should infer `r: Number` if `Circle(radius)` was constructed with numbers. Currently all variant fields are `Any`. | High | Both |
| T1.4 | **Method return types from implementation** | `.length()` should return `Int`, `.push()` should return the updated collection type, `.map(f)` should return `List[ReturnType(f)]`. Currently these are hardcoded or `Unknown`. Build a method signature registry. | High | Both |
| T1.5 | **Remove `Unknown` default fallback** | After solving, any remaining `Unknown` types should trigger a diagnostic: "Cannot infer type of X — consider adding context." This is the key step that transforms the type system from optional to reliable. | Medium | Both |

### Phase T2: User-Defined Generics

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| T2.1 | **Generic type parameter syntax** | Enable `type Pair[A, B]` with `first` and `second` fields. No annotation on the fields themselves — types are inferred from usage. The `[A, B]` declares type variables that the solver binds through usage patterns. | High | Both |
| T2.2 | **Generic function inference** | A function `*identity(x)` is already inferred as `∀T. T → T` by the solver. Formalize this: all free type variables in a function signature are implicitly universally quantified. Add let-polymorphism so `id(42)` and `id("hello")` can coexist. | Very High | Both |
| T2.3 | **Generic instantiation in types** | `Option[Number]`, `List[String]`, `Map[String, List[Number]]`. Replace the current hack where `Option` is mapped to `List` with proper generic type application and substitution in the solver. | Very High | Both |
| T2.4 | **Trait bounds on generics** | `type SortedList[T with Comparable]` — constrain type variables to types implementing specific traits. This enables generic algorithms that require specific capabilities. | High | Both |
| T2.5 | **Monomorphization strategy** | At codegen time, instantiate generic types and functions for each concrete type argument used. `identity[Number]` and `identity[String]` become separate LLVM functions. This is crucial for unboxed performance. | Very High | Both |

### Phase T3: Flow-Sensitive Typing

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| T3.1 | **Type narrowing in conditionals** | After `if x.is_err`, the type of `x` in the else branch should be narrowed to the success type. After `match x / Some(v) ?`, `v` should carry the inner type. | High | Both |
| T3.2 | **Definite assignment analysis** | Track which variables are definitely assigned on all paths. Warn on use of potentially uninitialized variables. This catches a common class of bugs that the current scope-check misses. | Medium | Both |
| T3.3 | **Nullability tracking** | `None` currently unifies with everything, making all types implicitly nullable. Introduce `Option[T]` at the type level: a function returning `none` on some paths should have return type `Option[T]`, not `T`. | Very High | Both |
| T3.4 | **Error type tracking** | Error values should carry their error taxonomy type: `err Database:Connection:Timeout` should have type `Error[Database.Connection.Timeout]`. The compiler can then verify exhaustive error handling. | High | Both |
| T3.5 | **Dead code detection** | Statements after `return`, `break`, or unconditional `err` should produce warnings. Currently silently accepted. | Low | Both |

### Phase T4: Inference Quality of Life

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| T4.1 | **Multi-error recovery in type solving** | Currently the first type error halts solving. Collect all type errors and report them together, with context about what was being unified and why. | Medium | Both |
| T4.2 | **Better error messages** | When inference fails, show the chain of constraints that led to the conflict: "In `add(x, 'hello')`, `x` was inferred as Number from line 5, but String is required from the `+` on line 8." | High | Both |
| T4.3 | **Ranked unification** | When unifying two type variables, prefer the one with more information as the root. This produces more informative error messages when errors eventually surface. | Medium | Both |
| T4.4 | **Return type unification across branches** | `if`/`elif`/`else` branches should unify their types to determine a common return type. Currently only `match` arms are unified. | Medium | Both |

---

## Pillar 3: Compiler Intelligence & Comptime Optimizations

**Current State:** The only optimization is constant folding of literal arithmetic/boolean/string expressions. All value operations delegate to runtime FFI calls. No inlining, no dead code elimination, no loop optimization, no specialization. LLVM's optimizer is not invoked by default. The compiler essentially translates Coral to a series of C function calls.

**Target State:** The compiler performs deep analysis to identify values, expressions, and entire functions that can be evaluated or specialized at compile time. Hot loops are type-specialized with unboxed arithmetic. Dead code is eliminated. Small functions are inlined. The generated LLVM IR is optimized, annotated with metadata, and fed to LLVM with appropriate optimization flags. The result is machine code that competes with hand-written C for compute-heavy workloads.

### Phase C1: Enhanced Constant Folding & Comptime Evaluation

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| C1.1 | **Extend constant folding to all pure operations** | Fold string operations (`'hello'.length()` → `5`), list literals (`[1,2,3].length()` → `3`), math functions on constants (`sqrt(4.0)` → `2.0`), and chained operations (`2 * 3 + 1` → `7`). | Medium | Both |
| C1.2 | **Comptime function evaluation** | If a function's arguments are all compile-time constants and the function body is pure (no side effects, no I/O, no mutation), evaluate it at compile time and replace the call with the result. This is Zig's `comptime` concept. | Very High | Both |
| C1.3 | **Purity analysis** | Classify functions as Pure (no side effects), ReadOnly (reads but doesn't mutate), or Effectful (I/O, mutation, actor messaging). Pure functions are eligible for comptime evaluation, memoization, and reordering. | High | Both |
| C1.4 | **Compile-time string building** | Template strings with all-constant parts should be folded: `'Hello, {"World"}!'` → `'Hello, World!'`. Currently this generates runtime concatenation calls. | Medium | Both |
| C1.5 | **Dead expression elimination** | Expressions whose results are never used (no side effects) should be eliminated. `x is expensive_pure_fn(); log('done')` — if `x` is never read, the call can be removed. | Medium | Both |

### Phase C2: Type Specialization & Unboxing

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| C2.1 | **Numeric specialization in loops** | When a loop variable is inferred as `Number` and only used in arithmetic, generate unboxed `f64` LLVM code: `fadd`, `fsub`, `fmul`, `fdiv` directly. No boxing/unboxing per iteration. | Very High | Both |
| C2.2 | **Boolean specialization** | Boolean operations (`and`, `or`, `not`, comparisons) should use LLVM `i1` when both operands are known-boolean. No runtime tag checks. | High | Both |
| C2.3 | **Monomorphic call sites** | When a function is always called with the same types (e.g., `add(int, int)`), generate a specialized version that skips type checks. The generic version remains for polymorphic call sites. | Very High | Both |
| C2.4 | **Unboxed list specialization** | A `List[Number]` should use a contiguous `f64` array, not a `Vec<Value*>`. Element access is a direct memory load, not a tag-check-and-unbox. | Very High | Rust first |
| C2.5 | **Store field specialization** | When all instances of a store have fields with known types, replace the map-based representation with a struct layout. `point.x` becomes a direct offset load instead of a hash table lookup. | Very High | Both |

### Phase C3: Inlining & Function Optimization

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| C3.1 | **Small function inlining** | Functions with ≤5 expressions and no recursion should be inlined at call sites. Annotate with LLVM `alwaysinline` attribute. | High | Both |
| C3.2 | **Lambda inlining in higher-order functions** | `list ~ map($ * 2)` should inline the lambda body into the map loop body, eliminating the closure allocation and indirect call. | Very High | Both |
| C3.3 | **Tail call optimization** | Detect tail-recursive functions and convert to loops. `*factorial(n, acc)` with `factorial(n-1, n*acc)` as the last expression should compile to a loop, not a recursive call chain. | High | Both |
| C3.4 | **Common subexpression elimination** | `a.length() + a.length()` should evaluate `a.length()` once. Track pure expressions and reuse their results. | Medium | Both |
| C3.5 | **Dead function elimination** | Functions that are defined but never called (transitively from `main`) should not be emitted in the LLVM IR. Reduces binary size and compilation time. | Medium | Both |

### Phase C4: LLVM Optimization Integration

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| C4.1 | **Optimization level flags** | Add `-O0` (no optimization), `-O1` (basic), `-O2` (standard), `-O3` (aggressive) CLI flags. Map to `llc` optimization levels. Default `--jit` to `-O0` for fast iteration; default `--emit-binary` to `-O2`. | Medium | Both |
| C4.2 | **LLVM function attributes** | Emit `nounwind`, `readnone`, `readonly`, `argmemonly`, `willreturn` attributes on functions based on purity analysis. These enable LLVM's own optimization passes to work effectively. | Medium | Both |
| C4.3 | **LLVM alias analysis hints** | Emit `noalias` on function parameters that don't alias. Emit TBAA metadata for typed memory accesses. This enables LLVM to reorder and vectorize memory operations. | High | Both |
| C4.4 | **Link-time optimization (LTO)** | Support LTO between the compiled Coral IR and the runtime library. This allows LLVM to inline runtime functions (like `coral_value_add`) into Coral code, eliminating call overhead. | High | Rust first |
| C4.5 | **Profile-guided optimization (PGO)** | Support `--emit-profile` to generate instrumented binaries and `--use-profile` to apply profile data. Hot paths get aggressive optimization; cold paths are optimized for size. | Medium | Rust first |

### Phase C5: Advanced Comptime Features

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| C5.1 | **Compile-time code generation** | Allow `comptime` blocks that run at compile time and produce AST nodes or IR. Enables type-safe metaprogramming: macro-like code generation without the macro complexity. | Very High | Both |
| C5.2 | **Compile-time assertions** | `comptime_assert(size_of(Point) <= 64)` — assertions that are verified during compilation, not at runtime. Zero runtime cost for invariant checking. | Medium | Both |
| C5.3 | **Const generics** | Allow compile-time constant values as type parameters: `type FixedArray[T, N]` where `N` is a compile-time integer. Enables stack-allocated fixed-size arrays. | Very High | Both |
| C5.4 | **Compile-time string processing** | Regex compilation, format string validation, SQL query validation — all at compile time. Template strings with constant parts are validated and optimized before any code runs. | High | Both |

---

## Pillar 4: Syntax Refinement & Expressiveness

**Current State:** Coral's syntax is 80% beautiful and 20% awkward. The `is` keyword for binding, `*` for functions, and indentation scoping are genuine wins. But `.equals()` for comparison, `? !` for ternary/error contexts, missing `for..in` loops in examples, and `is` overloading across binding/comparison/map-entries create real confusion. The pipeline operator `~` is in the spec but not fully operational.

**Target State:** Every expression in Coral reads like natural language. The syntax has zero unnecessary ceremony. Common patterns (comparison, iteration, error handling, data transformation) are a single line. The language feels like writing pseudocode that compiles to optimal machine code. New users can read Coral programs without a tutorial.

### Phase S1: Core Syntax Clarity

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| S1.1 | **Resolve `is` overloading for map entries** | Currently `map('host' is 'localhost')` uses `is` for both binding and key-value association. Introduce `:` for map entries: `map('host': 'localhost', 'port': 8080)`. This eliminates the visual ambiguity while preserving the conversational style. Reserve `is` purely for variable binding and `match`-context equality. | Medium | Both |
| S1.2 | **Clarify ternary vs error propagation** | The `?` and `!` symbols serve double duty: `cond ? then ! else` (ternary) and `expr ! return err` (error propagation). Consider alternate ternary syntax: `cond then expr else expr` or `if cond: expr else: expr` as an inline form. Alternatively, make `!` unambiguous by context (the parser already disambiguates, but humans struggle). | Medium | Both |
| S1.3 | **`for..in` range support** | `for i in 1..20` should work. Implement range literal syntax (`start..end`, `start..=end`, `start..end..step`) with codegen that produces efficient counted loops. This is table-stakes for a modern language. | Medium | Both |
| S1.4 | **Unary negation** | Support `-x` as a unary expression. Currently the self-hosted code uses `0 - x` and `math` defines `neg_inf is -1.0 / 0.0`. Unary minus should be a first-class operator with proper precedence. | Low | Both |
| S1.5 | **Augmented assignment operators** | Support `x += 1`, `x -= 1`, `x *= 2`, `x /= 2`. These desugar to `x is x + 1` etc. in the lowering pass. Major ergonomic win for loops and accumulators. | Medium | Both |

### Phase S2: Collection & Data Expression

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| S2.1 | **Pipeline operator full implementation** | `data ~ map($ * 2) ~ filter($ > 4) ~ sum()` must work end-to-end in both compilers. Pipeline desugaring should happen in the lowering pass (not scattered across semantic/parser). Test with the data_pipeline example. | High | Both |
| S2.2 | **List comprehensions** | `squares is [x * x for x in 1..100 if x % 2 is 0]` — syntactic sugar for `range(1,100) ~ filter($ % 2 == 0) ~ map($ * $)`. Compiles to an efficient loop with pre-allocated output list. | High | Both |
| S2.3 | **Map comprehensions** | `counts is {word: count for (word, count) in entries if count > 0}` — similar to list comprehensions but producing maps. | Medium | Both |
| S2.4 | **Destructuring assignment** | `(x, y) is get_point()` for list destructuring. `{name, age} is user` for map destructuring. `Some(value) is lookup(key)` for ADT destructuring. Produces clear, concise variable extraction. | High | Both |
| S2.5 | **Slice syntax** | `list[1..5]` to extract a sublist. `string[0..3]` to extract a substring. Uses the range syntax from S1.3 applied to indexing. | Medium | Both |
| S2.6 | **Spread operator** | `combined is [...list1, ...list2, extra_item]` for list spreading. `merged is {...map1, ...map2, 'key': override}` for map spreading. | Medium | Both |
| S2.7 | **Tuple syntax** | `point is (3, 4)` as a lightweight, ordered, immutable collection. Tuples are structurally typed: `(Number, Number)`. Destructuring works naturally: `(x, y) is point`. | Medium | Both |

### Phase S3: Pattern Matching Enhancement

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| S3.1 | **Multi-statement match arms** | Currently match arms are single expressions. Allow indented blocks: `Some(v) ? \n  log(v) \n  process(v)`. The last expression is the arm's value. | Medium | Both |
| S3.2 | **Guard clauses in match** | `Some(v) if v > 0 ? handle_positive(v)` — conditional matching beyond structural pattern checks. | Medium | Both |
| S3.3 | **Or-patterns** | `Circle(r) | Sphere(r) ? compute_area(r)` — match multiple patterns with the same body. | Medium | Both |
| S3.4 | **Nested pattern matching** | `Some(Circle(r)) ? r` — currently partially supported. Ensure deep nesting works: `Ok(Some([first, ...rest])) ? process(first, rest)`. | High | Both |
| S3.5 | **String/number range patterns** | `match code / 200 to 299 ? 'success' / 400 to 499 ? 'client error' / 500 to 599 ? 'server error'`. Uses existing `to` keyword (consistent with `for..in..to` range loops). | Medium | Both |
| S3.6 | **`match` as statement** | Allow `match` to be used in statement position without capturing a value. Each arm executes side effects. Currently match is expression-only. | Low | Both |

### Phase S4: Function & Method Expressiveness

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| S4.1 | **Named arguments** | `connect(host: 'db.local', port: 5432, timeout: 30)` — especially for functions with many parameters. Desugars to positional at compile time. | High | Both |
| S4.2 | **Default parameter values** | `*connect(host, port ? 5432, timeout ? 30)` — already partially in the syntax spec. Ensure codegen handles optionality correctly. | Medium | Both |
| S4.3 | **Multi-line lambda syntax** | Allow lambdas with indented bodies: `callback is (x) ->\n  validate(x)\n  transform(x)`. Currently lambdas are single-expression. | Medium | Both |
| S4.4 | **Method chaining fluency** | Ensure `string.trim().lower().split(' ').map($.capitalize()).join(' ')` works as a single fluent expression. Each method returns the appropriate type for the next call. | High | Both |
| S4.5 | **Extension methods** | `extend String with *word_count() -> split(' ').length()` — add methods to existing types without modifying their definition. Enables library code to augment built-in types. | High | Both |
| S4.6 | **Return expressions in lambdas** | Currently explicitly unsupported (closures.rs line 22). Allow `return` in lambdas as a return from the lambda, not the enclosing function. | Medium | Both |

### Phase S5: Conversational Syntax Sugar

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| S5.1 | **`unless` keyword** | `unless ready ? return` — cleaner than `not ready ? return` for negative conditions. Desugars to `if not cond`. | Low | Both |
| S5.2 | **`until` loop** | `until done ? ...` — cleaner than `while not done`. | Low | Both |
| S5.3 | **`loop` keyword for infinite loops** | `loop / ...body... / break if condition` — replaces `while true`. Communicates intent clearly. | Low | Both |
| S5.4 | **`when` expression** | A multi-branch conditional without requiring a match target: `when / x > 100 ? 'high' / x > 50 ? 'medium' / _ ? 'low'`. Like Kotlin's `when` without an argument — sugar for if/elif/else. | Medium | Both |
| S5.5 | **`do..end` blocks for DSL-style syntax** | Optional `do`/`end` delimiters for blocks in addition to indentation. Useful for passing multi-line blocks as arguments, e.g., `describe 'tests' do ... end`. | Medium | Both |
| S5.6 | **Postfix `if`/`unless`** | `log('warning') if debug_mode` — statement executed conditionally, postfix style. The existing guard `debug_mode ? log('warning')` is similar, but postfix reads more naturally for simple statements. | Low | Both |

---

## Pillar 5: Runtime Performance Engineering

**Current State:** The runtime is ~25,000 lines of Rust providing 239 FFI functions. Every value is a 40-byte heap struct. Arithmetic, comparison, and collection operations all go through tagged-value dispatch. Actors use lock-based synchronization with a simple work queue. Persistent stores use WAL with no compaction. The runtime is safe and correct but not fast.

**Target State:** The runtime is a lean, highly optimized substrate. Hot paths (arithmetic, comparison, retain/release) are branch-free. Collections use cache-friendly layouts. The actor scheduler uses work-stealing. The store engine supports efficient queries. The runtime adds negligible overhead to compiled Coral code.

### Phase R1: Hot-Path Optimization

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| R1.1 | **Branch-free tag dispatch** | Replace `match value.tag { ... }` cascades with jump tables or computed gotos. The tag byte (0-10) maps directly to a function pointer array. | Medium | Rust |
| R1.2 | **Inline string threshold optimization** | Profile real-world string length distributions. The current 14-byte inline threshold may not be optimal. Consider 22 bytes (fit in a cache line with the header) if the Value struct can be reorganized. | Medium | Rust |
| R1.3 | **Small-list optimization** | Lists with ≤8 elements can be stored inline in the Value struct, similar to small-string optimization. Avoids a heap indirection for the very common case of short lists. | High | Rust |
| R1.4 | **Map optimization: Robin Hood hashing** | Replace linear probing with Robin Hood hashing for better cache behavior and lower worst-case probe counts. Or consider Swiss Table (like Rust's `hashbrown`). | Medium | Rust |
| R1.5 | **Comparison fast paths** | `coral_value_equals` with two NaN-boxed numbers is a single `f64` bitwise comparison. Two inline strings of same length are a single `memcmp`. Most comparisons should be 1-3 instructions. | Medium | Rust |

### Phase R2: Actor System Performance & Completion

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| R2.1 | **Work-stealing scheduler** | Replace the single `mpsc::channel` work queue with per-worker deques (crossbeam-deque) and work stealing. Eliminates central contention when actors are distributed across workers. | High | Rust |
| R2.2 | **Lock-free actor registry** | Replace `Mutex<HashMap>` for the named actor registry with a concurrent hash map (dashmap or custom). Named actor lookup should be wait-free on the read path. | Medium | Rust |
| R2.3 | **Message dispatch optimization** | Replace string-based message handler matching with integer tags or direct function pointers. A message `@increment(amount)` should dispatch by index, not by comparing `"increment"` as a string at runtime. | High | Both |
| R2.4 | **Cooperative yielding** | Instead of running actors to completion, insert yield points at loop back-edges and after every N messages. Prevents a single CPU-bound actor from starving others. | Medium | Rust |
| R2.5 | **Actor state pinning** | Pin actor state to the worker thread that runs it, reducing cache misses for repeated handler invocations. Migrate only when work-stealing requires it. | High | Rust |
| R2.6 | **Complete supervision restart** | Change `spawn_supervised_child` from `FnOnce` to `Arc<dyn Fn>` so that supervision can actually restart failed actors (not just decide to restart them). | Medium | Rust |
| R2.7 | **Typed messages** | `@messages(MessageType)` annotation + compile-time type checking at `send()` call sites. Ensures message type safety without runtime overhead. | High | Both |
| R2.8 | **Actor monitoring** | `monitor(actor)` / `demonitor(actor)` + `ActorDown` message delivery. Enables reactive failure handling without supervision trees. | High | Rust |
| R2.9 | **Supervision hardening** | Restart budget enforcement, time windows, escalation chains. Complete the supervision tree model for production use. | High | Rust |
| R2.10 | **Graceful actor stop** | Flush mailbox before termination. Ensure in-flight messages are processed or explicitly discarded. | Medium | Rust |
| R2.11 | **Remote actors (foundation)** | TCP transport, serialization protocol, remote proxy, location-transparent lookup. Foundation for distributed actor systems. | Very High | Rust |
| R2.12 | **Actor integration tests** | Supervision trees, monitoring, typed messages, multi-level restart scenarios, work-stealing verification. | High | Both |

### Phase R3: Store Engine Performance & Completion

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| R3.1 | **Secondary indexes** | Support indexing store fields for O(log n) lookup by value instead of O(n) linear scan. B+ tree implementation for ordered fields, hash index for equality lookups. | High | Rust |
| R3.2 | **WAL compaction** | Periodic compaction of the write-ahead log: merge committed entries, remove stale versions, reclaim disk space. Currently WAL grows monotonically. | Medium | Rust |
| R3.3 | **Memory-mapped I/O** | Use `mmap` for the binary store format, enabling OS-level page caching and lazy loading. Only pages actually accessed are read from disk. | High | Rust |
| R3.4 | **Query optimization** | Build a simple query planner that chooses between sequential scan and index lookup based on available indexes. Supports the store query syntax. | High | Rust |
| R3.5 | **ACID transactions** | Implement multi-operation transactions with commit/rollback semantics. Use MVCC for concurrent access. Essential for the store to be useful in real applications. | Very High | Rust |
| R3.6 | **Store query syntax** | Add language-level syntax for store queries (filter, find, aggregate). Compile to efficient query plans using available indexes. | High | Both |
| R3.7 | **Store indexing from language level** | Expose B+ tree index creation and query to Coral code. `store.index("field_name")` at definition time. | Medium | Both |
| R3.8 | **WAL recovery verification** | Write data → simulate crash → recover → verify integrity. Automated tests for crash recovery guarantees. | Medium | Rust |
| R3.9 | **WeakRef clone semantics fix** | Fix shared registry IDs in WeakRef clones (current use-after-free risk when original is freed). | Medium | Rust |

### Phase R4: Platform-Independent Optimizations

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| R4.1 | **SIMD string operations** | Use AVX2/NEON for string search, comparison, and case conversion. `string.find("pattern")` can be 4-8x faster with vectorized scanning. | High | Rust |
| R4.2 | **Custom allocator** | Replace the system allocator with a purpose-built allocator: size-class segregated pools for common Value sizes, thread-local free lists, and arena allocation for batch operations. | Very High | Rust |
| R4.3 | **Allocation batching** | For list/map construction, allocate all elements in a single allocation call (arena-style) and free them together. `[1, 2, 3, 4, 5]` should be one allocation, not five. | High | Rust |
| R4.4 | **Cache-line-aligned Value struct** | Reorganize the `Value` struct to fit in a single 64-byte cache line. Move cold fields (retain_events, release_events) out of the main struct. | Medium | Rust |

### Phase R5: Self-Hosted Runtime

*Rewrite the Coral runtime in Coral per `SELF_HOSTED_RUNTIME_SPEC.md`. Currently entirely in Rust (~25,000 lines). This is the second major self-hosting milestone after the compiler bootstrap.*

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| R5.1 | **Value representation in Coral** | Tagged NaN-boxed values with inline/heap layout, matching the Rust runtime's representation exactly. | Very High | Coral |
| R5.2 | **Retain/release in Coral** | Refcounting via atomic operations (inline asm or FFI to C atomics). Must match semantics of Rust `coral_nb_retain`/`coral_nb_release`. | High | Coral |
| R5.3 | **String implementation** | SSO at ≤15 bytes, heap allocation, all string operations. Must pass existing string tests. | High | Coral |
| R5.4 | **List implementation** | Dynamic array with push/pop/get/set/iteration. Must match Rust runtime semantics. | High | Coral |
| R5.5 | **Map implementation** | Open-addressing hash table with get/set/delete. Must match Rust runtime collision behavior. | High | Coral |
| R5.6 | **Closure representation** | Captured environment struct, invoke mechanism matching the Rust runtime's closure ABI. | High | Coral |
| R5.7 | **Cycle detector in Coral** | Bacon's synchronous cycle collection algorithm, matching the Rust implementation. | High | Coral |
| R5.8 | **Actor scheduler in Coral** | M:N scheduling, mailboxes, work queues. Must support the same actor primitives as the Rust runtime. | Very High | Coral |
| R5.9 | **Store engine in Coral** | WAL, binary/JSON storage, B+ tree indexes. Must pass existing store E2E tests. | Very High | Coral |
| R5.10 | **FFI layer** | C function declarations, syscall wrappers, atomics. The bridge between Coral runtime and OS. | High | Coral |
| R5.11 | **Runtime integration tests** | Verify Coral runtime matches Rust runtime behavior on all existing tests. | High | Coral |
| R5.12 | **Memory allocator (optional)** | Custom allocator via `mmap`/`brk` for libc independence (see `LIBC_INDEPENDENCE.md`). | Very High | Coral |

**Expected Impact:** Full self-hosting — both compiler and runtime written in Coral. Enables the language to evolve independently of Rust.

---

## Pillar 6: Standard Library & Ecosystem

**Current State:** 20 modules with ~1,900 lines covering basic functionality. Math is ~85% complete, most other modules are 40-70%. O(n²) string patterns are pervasive. Testing lacks suites and proper assertions. Networking is TCP-only stubs. No regex, no crypto, no random numbers, no HTTP, no child process spawning.

**Target State:** A comprehensive standard library that makes Coral self-sufficient for real applications. Every module is well-documented, tested, and performant. The library feels cohesive with consistent naming and patterns. External dependencies (regex, crypto, HTTP) are available within the language ecosystem.

### Phase L1: Foundation Fixes (Fix Broken Patterns)

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| L1.1 | **String builder / rope type** | Introduce `StringBuilder` or rope-based string concatenation to eliminate O(n²) concat patterns throughout the stdlib. `join`, `reverse_str`, format operations should use this. | High | Rust + Coral |
| L1.2 | **Fix `unwrap` to actually panic** | `Option.unwrap` and `Result.unwrap` currently only `log` — they should call `exit(1)` or `panic()` on failure. This is a correctness bug in the most critical error-handling functions. | Low | Coral |
| L1.3 | **Fix `assert_eq` for all types** | Currently uses `number_to_string`, failing for string/list/map comparisons. Should use `.equals()` for comparison and `to_string()` for display. | Low | Coral |
| L1.4 | **Consistent naming convention** | Standardize across all modules: `starts_with` not `begins_with`, `replace` not `sub`, `split` not `divide`, `slice` not `part`. Deprecate old names with helpful messages. | Medium | Coral |
| L1.5 | **`list.pop()` that returns removed element** | The O(n) copy pattern for removing the last element appears 5+ times in the self-hosted compiler. Add `coral_list_pop` FFI that removes and returns the last element in O(1). | Low | Rust + Coral |
| L1.6 | **Map iteration support** | Enable `for key in map.keys()`, `for value in map.values()`, `for (key, value) in map.entries()`. The self-hosted compiler's semantic pass is blocked by this absence. | Medium | Rust + Both |

### Phase L2: Core Module Completion

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| L2.1 | **`std.random`** | `random()` (0.0-1.0 float), `random_int(min, max)`, `random_choice(list)`, `shuffle(list)`, `random_bytes(n)`. Use xoshiro256** or similar fast PRNG. Seedable for reproducibility. | Medium | Rust + Coral |
| L2.2 | **`std.regex`** | `match(pattern, text)`, `find_all(pattern, text)`, `replace(pattern, replacement, text)`, `split(pattern, text)`. Compile patterns at comptime when possible. Use a Rust regex crate binding. | High | Rust + Coral |
| L2.3 | **`std.time` enhancements** | Duration type: `Duration(3, 'seconds')`. Arithmetic: `future is now() + Duration(1, 'hour')`. Proper `sleep()` via runtime FFI (not busy-wait). ISO 8601 parsing: `parse_iso('2026-03-08T12:00:00Z')`. | Medium | Rust + Coral |
| L2.4 | **`std.io` enhancements** | Binary I/O (`read_bytes`, `write_bytes`), `stderr`, `file_size`, `rename`, `copy`, `make_dirs` (recursive), `temp_file`, `temp_dir`. Buffered reader/writer for large files. | Medium | Rust + Coral |
| L2.5 | **`std.process` enhancements** | `exec(command, args)` returning stdout/stderr/exit_code, `spawn_process(command)` returning a handle, `cwd()`, `chdir()`, `pid()`. Essential for build tools and scripts. | Medium | Rust + Coral |
| L2.6 | **`std.testing` enhancements** | Test suites: `suite('name', [...tests])`. Setup/teardown: `before_each`, `after_each`. `assert_close(a, b, tolerance)` for floats. `assert_contains(collection, item)`. `assert_matches(value, pattern)`. `benchmark(name, fn, iterations)`. Pretty-printed failure diffs. | Medium | Coral |

### Phase L3: Networking & Data

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| L3.1 | **`std.http`** | HTTP/1.1 client: `get(url)`, `post(url, body)`, `request(method, url, headers, body)`. Response type with status, headers, body. Server: `serve(port, handler)` with routing. | Very High | Rust + Coral |
| L3.2 | **`std.url`** | `parse(url_string)` → `{scheme, host, port, path, query, fragment}`. `encode`, `decode`, `build(parts)`. | Medium | Coral |
| L3.3 | **`std.net` UDP support** | `udp_bind(port)`, `udp_send(socket, host, port, data)`, `udp_recv(socket)`. | Medium | Rust + Coral |
| L3.4 | **`std.crypto`** | MD5, SHA-256, SHA-512 hashes. HMAC. AES-256 encrypt/decrypt. Random bytes from OS entropy. Bind to a Rust crypto crate. | High | Rust + Coral |
| L3.5 | **`std.csv`** | `parse(text)` → list of records, `stringify(records)` → text. Header handling, quoted fields, custom delimiters. | Medium | Coral |

### Phase L4: Developer Experience

| ID | Task | Description | Complexity | Dual-Impl |
|----|------|-------------|------------|-----------|
| L4.1 | **`std.debug`** | `inspect(value)` for pretty-printed value display (with type info). `Breakpoint()` for debugger integration. `trace(label, value)` for print-debugging with context. `time_it(label, fn)` for quick benchmarking. | Medium | Coral |
| L4.2 | **`std.path`** | Dedicated path manipulation: `join`, `parent`, `filename`, `extension`, `normalize`, `resolve`, `relative_to`, `is_absolute`. Abstracts away string-based path surgery. | Medium | Coral |
| L4.3 | **`std.collections`** | `Deque` (double-ended queue), `PriorityQueue` (min/max heap), `OrderedMap` (insertion-ordered), `DefaultMap` (default factory). | High | Rust + Coral |
| L4.4 | **Documentation generator** | Parse Coral source files, extract doc comments (prefix with `##`), and generate HTML/Markdown documentation. Similar to `rustdoc` or `pydoc`. | High | Coral |
| L4.5 | **Package manager spec & registry design** | Define `coral.toml` package manifest format. Dependency resolution strategy. Central registry or Git-based dependencies. This is the foundation of an ecosystem. | High | Spec + Coral |

---

## Cross-Cutting Concerns

These improvements span multiple pillars and must be addressed holistically.

### CC1: Dual Compiler Parity

| ID | Task | Description | Priority |
|----|------|-------------|----------|
| CC1.1 | **Feature parity tracking** | Maintain a matrix of features implemented in Rust vs Coral compilers. Every new feature must have a tracked status for both. | Ongoing |
| CC1.2 | **Shared test suite** | The same test programs should be compiled by both compilers and produce identical LLVM IR (or semantically equivalent IR). Divergence is a bug. | High |
| CC1.3 | **Self-hosted compiler relaxation removal** | The self-hosted compiler relaxes scope checking (line 664) and boolean constraints (line 1391) "during bootstrap." These relaxations should be tightened as the type system improves. | Medium |
| CC1.4 | **Performance comparison benchmark** | Build a benchmark suite and regularly compare: compilation speed, output binary size, and runtime performance between the two compiler backends. Target: self-hosted within 5x of Rust compiler speed. | Medium |

### CC2: Error Reporting & Diagnostics

| ID | Task | Description | Priority |
|----|------|-------------|----------|
| CC2.1 | **Source-mapped error messages** | Convert byte-offset spans to line:column. Show the offending source line with a caret pointing to the error. This is the minimum for usable error messages. | High |
| CC2.2 | **Multi-error reporting** | Collect and display all errors in a compilation, not just the first one. The parser already has recovery infrastructure — wire it through the entire pipeline. | Medium |
| CC2.3 | **DWARF debug info in LLVM IR** | Emit debug metadata (`!dbg` annotations) so that `gdb`/`lldb` can show Coral source lines during debugging. Essential for production use. | High |
| CC2.4 | **Warning categories** | Classify warnings (unused variable, shadowed binding, unreachable code, missing match arm) with optional suppression: `#[allow(unused)]`. | Medium |
| CC2.5 | **LSP protocol implementation** | Language Server Protocol support for VS Code and other editors: real-time diagnostics, go-to-definition, hover types, auto-complete. This is the gateway to adoption. | Very High |

### CC3: Module System Evolution

| ID | Task | Description | Priority |
|----|------|-------------|----------|
| CC3.1 | **AST-level module system** | Replace text-based `use` expansion with proper AST-level imports. Modules define exported symbols; importers reference them by qualified name. No more text splicing. | Very High |
| CC3.2 | **Namespacing** | `use std.io` should make `io.read()` available, not inject `read` into the global scope. Qualified access prevents name collisions and improves readability. | High |
| CC3.3 | **Selective imports** | `use std.math.{sin, cos, pi}` — import specific symbols. Keeps the scope clean and makes dependencies explicit. | Medium |
| CC3.4 | **Circular dependency handling** | Proper dependency graph resolution with clear error messages for circular imports. Currently detected but not gracefully handled. | Medium |
| CC3.5 | **Incremental compilation** | Cache compiled module artifacts and only recompile changed modules. Dramatically reduces compilation time for large projects. | High |

### CC4: Compilation Targets

| ID | Task | Description | Priority |
|----|------|-------------|----------|
| CC4.1 | **WebAssembly (WASM) target** | Emit WASM instead of x86_64 LLVM IR. Requires a WASM-compatible runtime (no pthreads, limited I/O). Enables Coral in browsers and edge computing. | High |
| CC4.2 | **macOS / ARM64 CI** | Ensure the compiler and runtime work on macOS with Apple Silicon. Requires cross-compilation testing infrastructure. | Medium |
| CC4.3 | **Windows support** | Replace Unix-specific runtime calls (mmap, pthreads) with platform-agnostic abstractions. MSVC or MinGW toolchain support. | Medium |
| CC4.4 | **Static linking** | Support fully static binaries (no libruntime.so dependency). Embed the runtime into the output binary. Simplifies deployment. | Medium |

### CC5: Quality & Testing Infrastructure

| ID | Task | Description | Priority |
|----|------|-------------|----------|
| CC5.1 | **Fuzz testing** | At minimum, fuzz the lexer and parser with AFL/libfuzzer to find crash/hang bugs. Extend to semantic analysis and codegen. | Medium |
| CC5.2 | **Fix remaining medium bugs** | P6 (single-error model), S6 (member access fallback), S8 (pipeline type inference). These are quality-of-life issues that affect real programs. | Medium |
| CC5.3 | **All examples compile and run** | Ensure all 7 example programs (calculator, chat_server, data_pipeline, fizzbuzz, hello, http_server, traits_demo) compile and produce correct output. | High |

### Known Issues

| ID | Issue | Description | Severity | Discovered |
|----|-------|-------------|----------|------------|
| KI-1 | **Built-in method name shadowing** | In `emit_member_call()` (`src/codegen/builtins.rs`), built-in method names (`set`, `get`, `push`, `pop`, `map`, `filter`, `reduce`, `length`, `keys`, `equals`, `not`, `iter`, `at`, `read`, `write`, `exists`, `log`, `concat`, `size`, `err`, `not_equals`) are matched in a hardcoded `match property` block that dispatches **before** checking `store_methods`. This means a user-defined store method named `set` is shadowed by the built-in `map.set(key, value)` — the built-in captures the call and fails with an arity error ("map.set expects exactly two arguments"). **Workaround**: Avoid naming store/extension methods with any of the ~20 built-in method names. **Fix**: Method dispatch should be type-aware — check if the target is a known store instance and prefer store methods, falling back to built-ins only for untyped or built-in-typed values. Requires threading store type information from semantic analysis into codegen's `emit_member_call()`. | Medium | Sprint 3 (S4.5) |

---

## Implementation Phases

The work above represents 18-24 months of focused development. Here is the recommended sequencing, with phases designed for parallelism where possible.

### Phase Alpha — ✅ COMPLETE

**Goal:** Eliminate the biggest performance bottlenecks and syntax pain points.

All Phase Alpha tasks have been completed:
- **M1.1-M1.8** ✅ NaN-boxing — full transition from `%CoralValue*` to `i64`
- **S1.1-S1.5** ✅ Core syntax clarity — map colon syntax, range loops, unary negation
- **L1.1-L1.6** ✅ Foundation fixes — StringBuilder, unwrap/panic, assert_eq, naming, list.pop, map iteration
- **T1.1-T1.5** ✅ Seal escape hatches — Store types, Unknown warnings, purity infrastructure
- **C1.1-C1.5** ✅ Enhanced constant folding — math/string/list folding, purity analysis, dead expression elimination
- **S2.1** ✅ Pipeline operator — desugaring in lowering pass
- **CC2.1-CC2.3** ✅ Error reporting — source-mapped errors, multi-error reporting, DWARF debug info

### Phase Beta — ✅ COMPLETE

**Goal:** The type system catches real bugs. The compiler generates smart code. The syntax handles every common pattern elegantly.

| Priority | Tasks | Status |
|----------|-------|--------|
| T2.1-T2.3 (User generics — syntax, inference, instantiation) | ✅ Complete |
| T2.4 (Trait bounds on generics) | ✅ Complete |
| T2.5 (Monomorphization) | Deferred to Gamma |
| C2.1-C2.3 (Type specialization) | ✅ Complete |
| C2.4-C2.5 (Unboxed lists, store field specialization) | Deferred to Gamma |
| S3.1-S3.3 (Multi-statement arms, guards, or-patterns) | ✅ Complete |
| S3.4-S3.5 (Nested patterns, range patterns) | ✅ Complete |
| S3.6 (Match as statement) | ✅ Complete |
| C3.1, C3.3-C3.5 (Inlining, TCO, CSE, dead function elim) | ✅ Complete |
| C3.2 (Lambda inlining in higher-order functions) | ✅ Complete |
| S2.2-S2.7 (Collection expressions) | ✅ Complete |
| M2.1-M2.4 (Non-atomic RC fast path) | ✅ Complete |
| S4.1 (Named arguments) | ✅ Complete |
| S4.2 (Default parameter values) | ✅ Complete |
| S5.1-S5.3 (unless/until/loop) | ✅ Complete |
| S5.4 (when expression) | ✅ Complete |
| C4.1 (Optimization level flags) | ✅ Complete |
| T3.5 (Dead code detection) | ✅ Complete |
| CC3.1-CC3.3 (Module system, namespacing, selective imports) | ✅ Complete |
| CC2.5 (LSP MVP — diagnostics) | ✅ Complete |

### Phase Gamma (Months 9-12): Runtime & Ecosystem

**Goal:** The runtime is production-grade. The standard library handles real workloads. The actor system scales.

| Priority | Tasks | Rationale | Status |
|----------|-------|-----------|--------|
| **Month 9-10** | R2.1-R2.12 (Actor performance & completion) | Work-stealing, typed messages, monitoring, supervision | Not started |
| ~~Month 9-10~~ | ~~L2.1 (std.random), L2.3 (std.time), L2.6 (std.testing)~~ | ~~Random, time, testing~~ | ✅ Done (Sprint 2) |
| ~~Month 9-10~~ | ~~L2.4 (std.io), L2.5 (std.process)~~ | ~~I/O enhancements, process management~~ | ✅ Done (Sprint 3) |
| ~~Month 9-10~~ | ~~L2.2 (std.regex)~~ | ~~Regex support~~ | ✅ Done (Sprint 4) |
| **Month 10-11** | R3.1-R3.8 (Store performance & completion) | Indexes, compaction, transactions, query syntax | Not started |
| ~~Month 10-11~~ | ~~R3.9 (WeakRef clone fix)~~ | ~~Fix use-after-free in WeakRef clones~~ | ✅ Done (Sprint 3) |
| ~~Month 10-12~~ | ~~CC3.1-CC3.3 (Module system)~~ | ~~Proper namespacing, selective imports~~ | ✅ Done (Sprint 1) |
| ~~Month 10-12~~ | ~~CC3.4 (Circular dependency enhancement)~~ | ~~Better cycle error messages with line numbers~~ | ✅ Done (Sprint 3) |
| **Month 11-12** | L3.1-L3.4 (Networking & data) | HTTP client/server, crypto | Not started |
| ~~Month 11-12~~ | ~~CC2.5 (LSP)~~ | ~~Editor integration for adoption~~ | ✅ Done (Sprint 1) |

### Phase Delta (Months 13-18): Mastery & Polish

**Goal:** Coral is production-ready. Advanced optimizations make it competitive with Rust for many workloads. The ecosystem is self-sustaining.

| Priority | Tasks | Rationale |
|----------|-------|-----------|
| Priority | Tasks | Rationale | Status |
|----------|-------|-----------|--------|
| ~~Month 13-14~~ | ~~T3.1 (Type narrowing), T3.3 (Nullability)~~ | ~~Type narrowing in match, nullable return warnings~~ | ✅ Done (Sprint 4) |
| **Month 13-14** | T3.4 (Error type tracking) | Error taxonomy types, exhaustive handling | Not started |
| ~~Month 13-14~~ | ~~T3.2 (Definite assignment analysis)~~ | ~~Uninitialized variable detection~~ | ✅ Done (Sprint 2) |
| ~~Month 13-14~~ | ~~T3.5 (Dead code detection)~~ | ~~Dead code warnings~~ | ✅ Done (Sprint 1) |
| ~~Month 13-14~~ | ~~T4.1-T4.3 (Type solver quality)~~ | ~~Multi-error, better messages, ranked unification~~ | ✅ Done (Sprint 3) |
| ~~Month 13-14~~ | ~~T4.4 (Return type unification)~~ | ~~Unify if/elif/else branch types~~ | ✅ Done (Sprint 2) |
| ~~Month 13-14~~ | ~~M3.1-M3.2 (Thread-local buffers, generational GC)~~ | ~~Eliminate mutex contention, epoch tracking~~ | ✅ Done (Sprint 3) |
| ~~Month 13-14~~ | ~~M3.4 (Closure cycle tracking)~~ | ~~Detect closure↔value cycles~~ | ✅ Done (Sprint 2) |
| ~~Month 13-14~~ | ~~M3.3 (Incremental GC)~~ | ~~Skipped — GC-free design decision~~ | SKIP (Sprint 4) |
| **Month 13-14** | M3.5 (Weak ref optimization) | Epoch-based reclamation | Not started |
| ~~Month 14-15~~ | ~~C4.1 (Optimization flags)~~ | ~~-O flag for JIT/binary~~ | ✅ Done (Sprint 1) |
| ~~Month 14-15~~ | ~~C4.2 (LLVM function attributes)~~ | ~~nounwind, readnone, willreturn~~ | ✅ Done (Sprint 2) |
| ~~Month 14-15~~ | ~~C4.3 (LLVM alias analysis hints)~~ | ~~noalias on params and allocators~~ | ✅ Done (Sprint 3) |
| ~~Month 14-15~~ | ~~C4.4 (LTO)~~ | ~~Link-time optimization~~ | ✅ Done (Sprint 4) |
| **Month 14-15** | C4.5 (PGO) | Profile-guided optimization | Not started |
| **Month 14-16** | M4.1-M4.4 (Escape analysis) | Stack allocation, region-based memory | Not started |
| ~~Month 15-16~~ | ~~S4.1-S4.2 (Named args, defaults)~~ | ~~Named arguments, default params~~ | ✅ Done (Sprint 1) |
| ~~Month 15-16~~ | ~~S4.3 (Multi-line lambdas)~~ | ~~Indented lambda bodies~~ | ✅ Done (Sprint 2) |
| ~~Month 15-16~~ | ~~S4.5 (Extension methods)~~ | ~~extend TypeName with new methods~~ | ✅ Done (Sprint 3) |
| ~~Month 15-16~~ | ~~S4.6 (Return in lambdas)~~ | ~~Return from lambda, not enclosing fn~~ | ✅ Done (Sprint 2) |
| ~~Month 15-16~~ | ~~S4.4 (Method chaining fluency)~~ | ~~Precise return types for chainable methods~~ | ✅ Done (Sprint 4) |
| ~~Month 15-16~~ | ~~S5.1-S5.4 (Conversational sugar)~~ | ~~unless/until/loop/when~~ | ✅ Done (Sprint 1) |
| ~~Month 15-16~~ | ~~S5.6 (Postfix if/unless)~~ | ~~Statement-level conditionals~~ | ✅ Done (Sprint 2) |
| ~~Month 15-16~~ | ~~S1.5 (Augmented assignment)~~ | ~~+=, -=, *=, /= operators~~ | ✅ Done (Sprint 2) |
| ~~Month 15-16~~ | ~~CC2.4 (Warning categories)~~ | ~~Classified warnings with suppression~~ | ✅ Done (Sprint 2) |
| ~~Month 15-16~~ | ~~CC5.2 (Fix medium bugs S6/S8)~~ | ~~Member access fallback, pipeline inference~~ | ✅ Done (Sprint 3) |
| **Month 16-17** | C5.1-C5.4 (Advanced comptime) | Compile-time code generation | Not started |
| **Month 17-18** | CC4.1-CC4.4 (Compilation targets) | WASM, macOS, Windows, static linking | Not started |
| ~~Month 17-18~~ | ~~L4.2 (std.path)~~ | ~~Path manipulation module~~ | ✅ Done (Sprint 3) |
| **Month 17-18** | L4.1, L4.3-L4.5 (Developer experience) | Docs, collections, packages, debug tools | Not started |

### Phase Epsilon (Months 19-24): Self-Hosted Runtime & Full Independence

**Goal:** Coral runs on a runtime written entirely in Coral. The language is fully self-hosting and independent of Rust.

| Priority | Tasks | Rationale |
|----------|-------|-----------|
| **Month 19-20** | R5.1-R5.2 (Value repr + retain/release) | Foundation for Coral runtime |
| **Month 20-21** | R5.3-R5.6 (String, List, Map, Closure) | Core data types in Coral |
| **Month 21-22** | R5.7-R5.8 (Cycle detector, Actor scheduler) | Runtime infrastructure in Coral |
| **Month 22-23** | R5.9-R5.10 (Store engine, FFI layer) | Complete runtime feature set |
| **Month 23-24** | R5.11-R5.12 (Integration tests, allocator) | Verification and libc independence |
| **Ongoing** | CC5.1-CC5.3 (Fuzzing, bug fixes, examples) | Quality assurance throughout |

---

## Success Metrics

### Performance Benchmarks (vs. baseline March 2026)

| Benchmark | Current (est.) | Alpha Target | Beta Target | Delta Target |
|-----------|---------------|-------------|------------|-------------|
| Fibonacci(40) | ~150ms | ~20ms | ~8ms | ~3ms |
| String concat (1M chars) | ~2000ms | ~200ms | ~100ms | ~50ms |
| List map/filter (1M items) | ~800ms | ~200ms | ~80ms | ~30ms |
| Actor message throughput | ~100K/s | ~300K/s | ~1M/s | ~5M/s |
| Compiler self-compile | ~30s | ~15s | ~8s | ~3s |

### Quality Metrics

| Metric | Current | Alpha | Beta | Delta |
|--------|---------|-------|------|-------|
| Tests | 1016 | 1500+ | 3000+ | 5000+ |
| Examples that compile | 5/7 | 7/7 | 15+ | 30+ |
| Type errors caught (true positives) | ~30% | ~70% | ~90% | ~99% |
| Type errors missed (false negatives) | ~70% | ~30% | ~10% | ~1% |
| Compilation targets | 1 | 1 | 2 | 4 |
| Stdlib modules | 20 | 25 | 35 | 50+ |

### Expressiveness Metrics

| Pattern | Current Status | Target |
|---------|---------------|--------|
| `x + 1` (unboxed numeric) | Direct `fadd` via type specialization ✅ | Direct `fadd` instruction |
| `list ~ map($*2) ~ sum()` | Pipeline desugared in lowering pass ✅ | Zero-allocation pipeline |
| `for i in 1..100` | `for..to..step` counted loop ✅ | Counted loop, no allocation |
| `match x / Some(v) if v > 0 ? ...` | Guard clauses + or-patterns ✅ | Full guard + narrowing |
| `{name, age} is user` | ✅ Destructuring assignment (S2.4) | Destructuring with type inference |
| `server.handle(path: '/', method: 'GET')` | ✅ Named args implemented (S4.1) | Named arguments |
| `*connect(host, port ? 5432)` | ✅ Default params implemented (S4.2) | Default parameter values |
| `unless ready ? return` | ✅ Desugared to if-not (S5.1) | Conversational sugar |
| `use std.math.{sin, cos}` | ✅ Selective imports (CC3.3) | Module system |
| `math.sin(x)` | ✅ Qualified access (CC3.2) | Namespacing |

---

## Appendix A: Design Constraints (Non-Negotiable)

These principles from the original language design must be preserved through all changes:

1. **No type annotations in user code.** Period. All typing is via inference. If the type system can't figure it out, it's the compiler's problem to solve, not the user's problem to annotate.

2. **`is` for binding.** The `=` and `==` tokens remain invalid. `is` is the binding operator. `.equals()` and method-based comparison remain for explicit equality in expression contexts.

3. **Errors are values, not exceptions.** No try/catch, no stack unwinding. The intrinsic error-flag model is the primary error mechanism. The `Result`/`Option` ADTs exist in the stdlib as data types, not as the error mechanism.

4. **Actors are language-level, not library-level.** The `actor` keyword and `@handler` syntax remain first-class grammar elements.

5. **Indentation scoping.** No braces, ever. The lexer's indent/dedent tokens are the scoping mechanism.

6. **Single numeric type at runtime.** `Number(f64)` remains the unified numeric type. Integer-only operations (bitwise, indexing) use `f64` with runtime range checks where necessary. Future phases may add an `Int(i64)` type if benchmarks justify the complexity.

7. **The self-hosted compiler must keep pace.** Every feature added to the Rust reference compiler must be implementable in Coral and added to the self-hosted compiler. The bootstrap invariant (gen2 == gen3) must be maintained at all times.

---

## Appendix B: Dependency Graph

```
M1 (NaN-boxing) ──────────────────────────┐
  │                                         │
  ├→ C2 (Type specialization)              │
  │    └→ C4 (LLVM optimization)           │
  │                                         │
  ├→ R1 (Hot-path optimization)            │
  │    └→ R4 (Platform optimization)       │
  │                                         │
  └→ M4 (Escape analysis)                  │
       └→ C5 (Advanced comptime)           │
                                            │
T1 (Seal escape hatches) ─────────────────┤
  │                                         │
  ├→ T2 (User generics)                    │
  │    └→ T3 (Flow-sensitive typing)       │
  │    └→ C2.4 (Unboxed list specialization)│
  │                                         │
  └→ C1.2 (Comptime function evaluation)   │
                                            │
S1 (Core syntax) ─────────────────────────│
  │                                         │
  ├→ S2 (Collection expressions)           │
  │    └→ S3 (Pattern matching)            │
  │                                         │
  └→ S4 (Function expressiveness)          │
       └→ S5 (Conversational sugar)        │
                                            │
L1 (Foundation fixes) ────────────────────│
  │                                         │
  ├→ L2 (Core modules)                     │
  │    └→ L3 (Networking & data)           │
  │                                         │
  └→ CC3 (Module system)                   │
       └→ L4 (Developer experience)        │
                                            │
CC2 (Error reporting) ────────────────────│
  └→ CC2.5 (LSP)                           │
                                            │
M2 (Non-atomic RC) ───→ M3 (Generational GC)
                                            │
R2 (Actor performance) ───→ R3 (Store perf)│
                                            │
                           CC4 (Targets) ───┘
```

---

## Appendix C: Risk Register

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| NaN-boxing breaks the bootstrap | High | Critical | Implement in Rust first; keep the old calling convention behind a feature flag until both compilers are updated |
| Type system improvements invalidate existing programs | Medium | High | Phased rollout: new checks as warnings first, then errors after one release cycle |
| Performance targets not met with f64-only numerics | Medium | High | If benchmarks show f64 is the bottleneck, add `Int(i64)` as an internal optimization type (still no user-visible distinction) |
| WASM target incompatible with actor model | High | Medium | Define a single-threaded actor mode for WASM where actors are cooperative tasks, not OS threads |
| Self-hosted compiler falls behind reference | Medium | High | Mandatory dual-implementation for all Pillar 4 (syntax) changes; automated CI check that gen2==gen3 |
| Module system migration breaks self-hosted bootstrap | High | Critical | Keep text-based module loading as fallback; migrate modules one at a time with each maintaining backward compat |

---

*This roadmap is a living document. Each task has been sized based on the current codebase analysis as of March 2026. Priorities should be reassessed after each phase completion.*

*The vision is clear: Coral must be the language that makes systems programming joyful. Not just possible — joyful. Every task in this document serves that mission.*
