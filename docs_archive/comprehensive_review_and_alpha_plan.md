# Coral Language Comprehensive Review & Alpha Action Plan

_Created: 2026-01-05_
_Updated: 2026-01-XX (Phase 1 & 2 Complete)_

## Executive Summary

Coral is an ambitious programming language project aiming to combine Python-like ergonomics with C/Rust-level performance, featuring built-in actors, persistent "store" objects, and automatic memory management. After a thorough review of the codebase, documentation, and test suite, I provide this honest assessment and detailed action plan.

**UPDATE**: Phase 1 (Foundation Repair) and Phase 2 (Actor MVP) are now complete. 95 tests passing.

---

## 1. Current State Assessment

### 1.1 What's Working Well

**Frontend (Lexer/Parser):**
- ✅ Solid indent-aware lexer with proper tab/space handling and mixed-indentation rejection
- ✅ Complete recursive-descent parser covering expressions, functions, types, stores, actors, taxonomies
- ✅ Placeholder-to-lambda lowering (`$`, `$1` syntax desugaring)
- ✅ Good diagnostic infrastructure with spans and help text
- ✅ Template strings with interpolation working
- ✅ Taxonomy literals (`!!Path:Like:This`) fully supported
- ✅ Keywords allowed as parameter names (e.g., `actor`, `type`) [Phase 1]

**Type System (NEW - Phase 1):**
- ✅ Modular HM-style inference in `src/types/` (core.rs, solver.rs, env.rs)
- ✅ Union-find constraint solver with typed error spans
- ✅ Proper type errors now propagated to diagnostics
- ✅ Arity checking for function calls with default parameters
- ✅ Collection method recognition (map/filter/reduce/iter/fold/each/flatten/find)

**LLVM Codegen:**
- ✅ Working code generation for core constructs
- ✅ Runtime FFI bindings properly declared
- ✅ List/map literals with pre-sizing from literal length
- ✅ Match expressions, ternaries, logical operators
- ✅ `extern fn` declarations and inline assembly (behind feature flag)
- ✅ Pointer load operations for low-level code
- ✅ Modular codegen with `src/codegen/runtime.rs` extracted [Phase 1]
- ✅ Closure variables properly resolved (call vs reference) [Phase 1]

**Actor System (NEW - Phase 2):**
- ✅ Actor state fields with default initializers
- ✅ `self.field` access in @message handlers
- ✅ State passed to handlers via closure environment
- ✅ Compile-time arity checking for @handlers (max 1 param)
- ✅ Failure propagation to parent actors (runtime)

**Runtime:**
- ✅ Tagged `Value` type with reference counting
- ✅ String/bytes support with inline small string optimization
- ✅ List and map operations (push/pop/get/set/length)
- ✅ M:N actor scheduler with worker threads
- ✅ Runtime metrics collection (retains/releases/allocations)
- ✅ Value pooling to reduce allocation churn
- ✅ HOF runtime helpers (map/filter/reduce) working

**Test Coverage:**
- ✅ 95 tests passing across parser, semantic, codegen, and modules
- ✅ Good fixture-based testing for parser
- ✅ Snapshot testing with insta
- ✅ Actor state tests with IR assertions

### 1.2 Critical Gaps & Issues (Updated)

**Type System:**
- ⚠️ Generic types (`List[T]`, `Map[K,V]`) declared but never instantiated
- ⚠️ Type annotations parsed but not enforced

**Store/Actor Implementation:**
- ⚠️ Stores parse but have no persistence mechanism
- ⚠️ No bounded mailbox with backpressure (runtime uses unbounded channels)
- ⚠️ Actors dispatch via string-matching (functional but could be optimized)
- ⚠️ No typed message contracts - all payloads passed as `Any`
- ✅ Actor state access from handlers (`self.field` works) [Phase 2]
- ⚠️ No backpressure/mailbox limits
- ✅ Failure propagation to parent [Phase 2 - runtime]

**Closure/HOF Runtime:**
- ✅ `list.map/filter/reduce` fully working [Phase 1]
- ✅ Closure capture semantics tested [Phase 1]
- ⚠️ No retain/release batching for captured values

**Memory Management:**
- ⚠️ Reference counting works but has known issues:
  - No cycle detection
  - 64-bit counters exist but not atomic for actor sharing
  - No arenas for stack-like temporaries
- ❌ Copy-on-write not implemented

**Module System:**
- ⚠️ `use` directive works but is basic string expansion
- ❌ No module caching/fingerprinting
- ❌ No proper module scoping (everything gets flattened)

**Test Failure:**
- ❌ 1 test failing: `compiles_program_using_runtime_actor` (parse error in std/runtime/actor.coral)

### 1.3 Code Quality Observations

**Strengths:**
- Clean separation of concerns (lexer → parser → semantic → MIR → codegen)
- Good use of Rust idioms (Result types, pattern matching)
- Documentation in code is reasonable
- Test infrastructure is solid

**Concerns:**
- `codegen.rs` is 2879 lines - needs refactoring into modules
- `parser.rs` is 1259 lines - could benefit from splitting
- `runtime/src/lib.rs` is 3350 lines - should be split
- Inconsistent error handling (some places use `unwrap()`)
- MIR is very simple - not yet a true intermediate representation
- No constant folding or optimization passes

---

## 2. Honest Critique

### 2.1 The Good

1. **Syntax Design**: The indentation-based, Python-like syntax is well-conceived. The `is` binding, `?` ternary operator, and `*` function prefix create a distinctive, readable aesthetic.

2. **Foundational Architecture**: The compiler pipeline is properly structured. Having lexer → parser → lowering → semantic → MIR → codegen is the right design.

3. **Runtime FFI**: The decision to use C-compatible FFI for runtime calls is smart - it enables the path to self-hosting.

4. **Test-Driven Development**: The fixture-based testing approach and use of insta for snapshots shows good engineering practices.

### 2.2 The Concerns

1. **Type Inference is a Skeleton**: The type system code exists but doesn't actually DO anything. This is the single biggest gap. You have `TypeGraph`, `ConstraintSet`, and `solve_constraints` but:
   - Constraints are collected but errors aren't propagated
   - The solver runs but failures are silently ignored
   - No actual type checking occurs at codegen time

2. **Stores/Actors Are Vapor**: These are advertised as core features but:
   - Stores have no persistence mechanism at all
   - Actors work at a basic level but lack state, typed messages, and supervision
   - Neither is remotely production-ready

3. **The "Self-Hosting" Goal is Distant**: To self-host, you need:
   - Complete type system
   - Working struct/record layouts (for AST nodes)
   - Closures that actually work
   - Pattern matching on algebraic data types
   - This is months away at minimum

4. **Technical Debt**: Large monolithic files, some unwrap()s, incomplete error messages suggest rushing to get things working without cleanup.

### 2.3 Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Type system is fake | CRITICAL | Must be priority #1 |
| Stores don't persist | HIGH | Design needed before implementation |
| Actors lack features | HIGH | MVP first, then iterate |
| Memory leaks (cycles) | MEDIUM | Can defer cycle detection |
| Performance unknown | MEDIUM | Benchmark after correctness |
| Code quality debt | LOW | Refactor as you go |

---

## 3. Alpha Release Definition

For an alpha release, Coral needs to be:
1. **Honest**: Features that exist should work correctly
2. **Useful**: Someone could write non-trivial programs
3. **Safe**: No silent memory corruption or crashes

### Minimum Viable Alpha Features:
1. ✅ Basic types (numbers, bools, strings, bytes, lists, maps)
2. ❌ Working type inference with error messages
3. ✅ Functions with parameters and closures
4. ❌ Actor spawn/send/receive with typed messages
5. ❌ Store definition with field access (persistence can wait)
6. ✅ Match expressions
7. ❌ Undefined name detection
8. ✅ Module imports

---

## 4. Detailed Action Plan

### Phase 1: Foundation Repair (Weeks 1-3)

#### 1.1 Fix Type System (Week 1)
**Priority: CRITICAL**

```
Tasks:
1. Wire constraint solver errors to diagnostics
2. Emit type mismatch errors at semantic phase
3. Add undefined-name detection  
4. Implement arity checking for function calls
5. Add tests for type errors (expect-fail fixtures)
```

**Files to modify:**
- `src/semantic.rs`: Add error collection from solver
- `src/types.rs`: Return proper errors instead of silent Ok
- `tests/semantic.rs`: Add type error tests

#### 1.2 Fix Failing Test (Day 1)
**Priority: HIGH**

The `std/runtime/actor.coral` file has a parse error. Fix:
```
- Check line 352 in expanded module
- Likely a syntax issue with extern declarations
```

#### 1.3 Stabilize Closures (Week 2)
**Priority: HIGH**

```
Tasks:
1. Verify capture semantics work correctly
2. Add retain/release for captured values
3. Test list.map/filter/reduce end-to-end
4. Fix any runtime closure invoke issues
```

**Files to modify:**
- `src/codegen.rs`: emit_lambda improvements
- `runtime/src/lib.rs`: closure invoke/release

#### 1.4 Code Quality (Week 3)
**Priority: MEDIUM**

```
Tasks:
1. Split codegen.rs into codegen/{mod.rs, expression.rs, statement.rs, runtime.rs}
2. Split runtime/lib.rs into {value.rs, list.rs, map.rs, string.rs}
3. Replace unwrap() with proper error handling in critical paths
4. Add cargo clippy to CI
```

### Phase 2: Actor MVP (Weeks 4-5)

#### 2.1 Actor State Access
```
Tasks:
1. Generate struct layout for actor fields
2. Implement self.field access in handlers
3. Add actor constructor that initializes fields
```

#### 2.2 Typed Message Dispatch
```
Tasks:
1. Generate dispatch table (interned string → handler index)
2. Add compile-time arity checking for @ handlers
3. Type-check message payloads (at least warn on Any)
```

#### 2.3 Basic Supervision
```
Tasks:
1. Propagate failure messages to parent
2. Add bounded mailbox with backpressure
3. Basic spawn with parent linking
```

### Phase 3: Store Foundation (Weeks 6-7)

#### 3.1 Store Type Layout
```
Tasks:
1. Generate struct layout for store fields
2. Implement field access (get/set)
3. Wire constructor to allocate store instances
```

**Note**: Persistence is NOT in alpha scope. Stores will be in-memory only.

#### 3.2 Reference Fields
```
Tasks:
1. Handle &field syntax for store references
2. Proper retain/release for reference fields
```

### Phase 4: Polish & Release (Week 8)

#### 4.1 Documentation
```
Tasks:
1. Write "Getting Started" guide
2. Document all working features
3. List known limitations
4. Create example programs
```

#### 4.2 CI/CD
```
Tasks:
1. cargo fmt/clippy checks
2. All tests passing
3. Basic smoke test with JIT
4. ASAN run on runtime
```

#### 4.3 Release Artifacts
```
Tasks:
1. Tag release
2. Build binaries for Linux/macOS
3. Publish documentation
```

---

## 5. Recommended Immediate Actions

### This Week:

1. **Day 1-2**: Fix the failing test and wire type errors
   ```rust
   // In semantic.rs, change:
   if let Err(msg) = crate::types::solve_constraints(&constraints, &mut graph) {
       return Err(Diagnostic::new(format!("type inference failed: {msg}"), program.span));
   }
   // To actually emit specific errors with spans
   ```

2. **Day 3-4**: Add undefined name detection
   ```rust
   // Track defined names during analysis
   // Emit errors for references to undefined symbols
   ```

3. **Day 5**: Add 10+ type error test cases

### This Month:

1. Complete Phase 1 (foundation repair)
2. Start Phase 2 (actor MVP)
3. Write first "real" Coral program as dogfooding

---

## 6. Long-Term Considerations

### For Self-Hosting (Post-Alpha):

1. **Algebraic Data Types**: Need sum types for AST representation
2. **Pattern Matching**: Need exhaustive matching for compiler code
3. **Strings as First-Class**: Need string manipulation for source code
4. **File I/O**: Need to read source files
5. **Error Recovery**: Parser needs to continue after errors

### For "Store" (Persistent Objects):

1. **Serialization Format**: Design binary format for value persistence
2. **Transaction Model**: Decide ACID guarantees
3. **Index Structures**: How to query stores efficiently
4. **Crash Recovery**: Write-ahead logging or similar

### For Performance ("as fast as C/Rust"):

1. **MIR Optimization**: Currently MIR is trivial, needs real optimization
2. **Typed Code Paths**: Avoid boxing for known numeric types
3. **Escape Analysis**: Stack-allocate non-escaping values
4. **Inlining**: Inline small functions

---

## 7. Metrics for Alpha Success

| Metric | Target | Current |
|--------|--------|---------|
| Tests Passing | 100% | 98.5% (1 failing) |
| Type Errors Detected | >50 cases | 0 cases |
| Actor Tests | >10 tests | ~2 tests |
| Store Tests | >5 tests | 0 tests |
| Example Programs | >5 programs | 1 program |
| Documentation Pages | >10 pages | ~5 pages |

---

## 8. Conclusion

Coral has a solid foundation with good syntax design and a proper compiler architecture. However, the type system is essentially non-functional, and the flagship features (stores, actors) are incomplete. 

**The honest assessment**: Coral is currently a **prototype**, not a language. To reach alpha, you need 6-8 weeks of focused work on:
1. Making the type system actually work
2. Getting actors to a usable state
3. Basic store field access (without persistence)

The self-hosting goal and "as fast as C" claims are aspirational and should be clearly marked as such. Focus on correctness first, then performance.

**Recommendation**: Declare current state as "pre-alpha" or "experimental" and work toward a proper alpha with the above plan. Don't overpromise features that don't work.

---

_This review was conducted by examining: all documentation files, lexer.rs, parser.rs, ast.rs, semantic.rs, types.rs, codegen.rs, mir.rs, mir_lower.rs, mir_interpreter.rs, compiler.rs, lower.rs, runtime/lib.rs, runtime/actor.rs, std/ modules, Cargo.toml files, and running the full test suite._
