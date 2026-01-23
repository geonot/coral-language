# Coral Alpha Implementation Tasks

_Created: 2026-01-06_
_Status: Active Development_

## Overview

This file tracks detailed implementation tasks toward Coral Alpha release.
Target: ~10 weeks of development (405 hours estimated)

**Current Phase**: Phase 4 - Value-Error Model & ADT Completion

---

## Phase 4: Value-Error Model & ADT Completion (Weeks 1-4)

### 4.1 Value-Error Model - Runtime Foundation

#### 4.1.1 Add error flags to Value struct
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 4h
- **Details**:
  - Added `FLAG_ERR = 0b0001_0000` constant
  - Added `FLAG_ABSENT = 0b0010_0000` constant
  - Added `is_err()`, `is_absent()`, `is_ok()` helper methods to Value

#### 4.1.2 Add error metadata struct
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 2h
- **Details**:
  - Created `ErrorMetadata` struct with code (u32), name (ValueHandle), origin_span (u64)
  - Error values store pointer to metadata in payload

#### 4.1.3 Implement `coral_make_error` runtime function
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 2h
- **Details**:
  - `coral_make_error(code, name_ptr, name_len)`
  - `coral_make_error_with_span(code, name_ptr, name_len, span)`
  - `coral_make_absent()`

#### 4.1.4 Implement error checking functions
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 2h
- **Details**:
  - `coral_is_err(v) -> u8`
  - `coral_is_absent(v) -> u8`
  - `coral_is_ok(v) -> u8`
  - `coral_error_name(v) -> ValueHandle`
  - `coral_error_code(v) -> u32`

#### 4.1.5 Update binary operations for error propagation
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 8h
- **Details**:
  - Updated `coral_value_add` with error propagation
  - Updated `coral_value_equals` with error propagation
  - Updated all bitwise ops with `propagate_binary_error` helper
  - Updated shift operations with error propagation

#### 4.1.6 Add short-circuit behavior to binary ops
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 4h
- **Details**:
  - Left operand error returns immediately
  - Added `propagate_binary_error` helper function
  - Added `propagate_unary_error` helper function

#### 4.1.7 Implement error handling methods
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete
- **Effort**: 2h
- **Details**:
  - `coral_value_or(v, default) -> ValueHandle`
  - `coral_unwrap_or(v, default) -> ValueHandle`

### 4.1 Value-Error Model - Parser & Syntax

#### Lexer: `#` comment support
- **File**: `src/lexer.rs`
- **Status**: ✅ Complete
- **Effort**: 0.5h
- **Details**:
  - Added `#` single-line comment support to lexer
  - Comments skip until newline
  - Fixes parsing of example files and std library

#### 4.1.8 Parse `err Name` error value syntax
- **File**: `src/parser.rs`
- **Status**: ✅ Already Working
- **Effort**: 2h
- **Details**:
  - `KeywordErr` token exists
  - `ErrorValue` AST node exists
  - Parser handles `err Foo`, `err Foo:Bar:Baz` patterns
  - Parse `err Foo`, `err Foo:Bar`, `err Foo:Bar:Baz`
  - Ensure proper error messages

#### 4.1.9 Parse `! return err` propagation syntax
- **File**: `src/parser.rs`, `src/ast.rs`
- **Status**: ✅ Complete
- **Effort**: 2h
- **Details**:
  - Added `ErrorPropagate` expression type to AST
  - Parses `expr ! return err` pattern with lookahead to avoid ternary conflict
  - Added `check_ahead` helper for 2-token lookahead
  - Doesn't conflict with ternary `cond ? then ! else` syntax

#### 4.1.10 Parse error definitions (`err Hierarchy`)
- **File**: `src/parser.rs`, `src/ast.rs`
- **Status**: ⬜ Not Started
- **Effort**: 4h
- **Details**:
  - Add `ErrorDefinition` item type
  - Parse hierarchical error definitions with code/message bindings
  - Store in semantic context

### 4.1 Value-Error Model - Codegen

#### 4.1.11 Codegen for error value creation
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete
- **Effort**: 4h
- **Details**:
  - Replaced string-based error representation
  - Calls `coral_make_error` runtime function
  - Passes error code (0) and name to runtime
  - Added runtime bindings for all error functions

#### 4.1.12 Codegen for error propagation
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete
- **Effort**: 4h
- **Details**:
  - Generates `coral_is_err` call to check if value is error
  - Emits conditional branch: early return if error, continue if success
  - Integrates with value-based error model

#### 4.1.13 Top-level unhandled error diagnostics
- **File**: `src/semantic.rs`
- **Status**: ⬜ Not Started
- **Effort**: 2h
- **Details**:
  - Warn when error values are silently ignored
  - Track "may-error" state in type inference
  - Emit diagnostic at top-level if error not handled

#### 4.1.14 Test suite: error handling (20+ tests)
- **File**: `tests/error_handling.rs`
- **Status**: ✅ Complete (14 tests)
- **Effort**: 4h
- **Details**:
  - Error creation and propagation ✓
  - Error checking methods (runtime) ✓
  - `! return err` syntax tests (7 new tests) ✓
  - Ternary/propagation conflict test ✓
  - Binary operation error propagation ✓
  - Hierarchical error names ✓

---

### 4.2 ADT Completion

#### 4.2.1 ADT construction codegen
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete (discovered existing)
- **Effort**: 4h
- **Details**:
  - `emit_tagged_constructor()` generates `Some(value)`, custom variants
  - `emit_enum_constructor_nullary()` generates `None` (0-field variants)
  - Calls runtime `coral_make_tagged` with tag name and fields array

#### 4.2.2 ADT variant tag storage and checking
- **File**: `runtime/src/lib.rs`
- **Status**: ✅ Complete (discovered existing)
- **Effort**: 4h
- **Details**:
  - `TaggedValue` struct with tag_name, tag_name_len, field_count, fields
  - `coral_tagged_get_tag(v)` returns tag name as string
  - `coral_tagged_is_tag(v, name, len)` for tag comparison
  - `coral_tagged_get_field(v, idx)` for field extraction
  - `coral_tagged_field_count(v)` for field count

#### 4.2.3 Pattern matching extraction for ADT variants
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete (discovered existing)
- **Effort**: 6h
- **Details**:
  - `emit_match_condition()` generates `coral_tagged_is_tag()` calls
  - `emit_match()` extracts fields via `coral_tagged_get_field()` 
  - Binds field values to pattern variable names in scope

#### 4.2.4 Exhaustiveness checking for match expressions
- **File**: `src/semantic.rs`
- **Status**: ✅ Complete (discovered existing)
- **Effort**: 8h
- **Details**:
  - `check_single_match_exhaustiveness()` tracks all variants
  - `build_constructor_map()` maps constructors to enum types
  - Reports missing variants with helpful diagnostic
  - Handles wildcard/identifier catch-all patterns

#### 4.2.5 Nested pattern matching
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete
- **Effort**: 4h
- **Details**:
  - Supports arbitrarily nested patterns like `Some(Some(x))`, `Ok(Some(x))`
  - Recursive `emit_match_condition` for nested tag checks
  - `bind_pattern_variables` helper for recursive variable binding
  - 13 tests in `tests/nested_patterns.rs`

#### 4.2.6 Test suite: ADT edge cases (15+ tests)
- **File**: `tests/adt.rs`
- **Status**: ✅ Complete (18 tests)
- **Effort**: 4h
- **Details**:
  - ADT construction (4 tests)
  - Pattern matching (5 tests)  
  - Exhaustiveness checking (5 tests)
  - Edge cases (4 tests)

---

## Phase 5: Language Features & Standard Library (Weeks 5-6)

### 5.1 Pipeline Operator (~)

#### 5.1.1 Add `~` token to lexer
- **Status**: ✅ Complete (Tilde token exists)

#### 5.1.2 Parse pipeline expressions
- **File**: `src/parser.rs`
- **Status**: ✅ Complete (Pipeline in AST)

#### 5.1.3-5.1.4 Desugar pipeline to function calls
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete (desugaring implemented)

#### 5.1.5 Handle `$` placeholder in pipeline context
- **File**: `src/codegen/mod.rs`
- **Status**: ✅ Complete
- **Effort**: 2h
- **Details**:
  - Added `contains_placeholder()` helper to detect $ in expressions
  - Added `replace_placeholder_with()` to substitute piped value
  - Pipeline detects $ in args and replaces instead of prepending
  - Supports `a ~ f($, extra)` becoming `f(a, extra)`

#### 5.1.6 Test suite: pipeline operator (10+ tests)
- **File**: `tests/pipeline.rs`
- **Status**: ✅ Complete (11 tests)
- **Effort**: 2h
- **Details**:
  - Basic pipeline (3 tests)
  - $ placeholder positioning (6 tests)
  - Edge cases (2 tests)

---

### 5.2 Trait/Mixin System

#### 5.2.1 Parse `trait` definitions
- **File**: `src/parser.rs`
- **Status**: ⬜ Not Started
- **Effort**: 4h
- **Details**:
  - `KeywordTrait` token exists
  - Parse trait name, method signatures, default impls

#### 5.2.2 Parse `with Trait` in type/store definitions
- **File**: `src/parser.rs`
- **Status**: ⬜ Not Started
- **Effort**: 2h
- **Details**:
  - `KeywordWith` token exists
  - Parse `store Foo with Trait1, Trait2`

#### 5.2.3 AST nodes for traits
- **File**: `src/ast.rs`
- **Status**: ⬜ Not Started
- **Effort**: 2h
- **Details**:
  - `TraitDefinition` item type
  - `TraitMethod` with optional body (default impl)
  - `with_traits` field on StoreDefinition/TypeDefinition

#### 5.2.4 Semantic: trait method resolution
- **File**: `src/semantic.rs`
- **Status**: ⬜ Not Started
- **Effort**: 6h

#### 5.2.5 Semantic: check required methods implemented
- **File**: `src/semantic.rs`
- **Status**: ⬜ Not Started
- **Effort**: 4h

#### 5.2.6 Semantic: default method inheritance
- **File**: `src/semantic.rs`
- **Status**: ⬜ Not Started
- **Effort**: 4h

#### 5.2.7 Codegen: trait method dispatch
- **File**: `src/codegen/mod.rs`
- **Status**: ⬜ Not Started
- **Effort**: 4h

#### 5.2.8 Trait composition
- **Status**: ⬜ Not Started
- **Effort**: 2h

#### 5.2.9 Test suite: traits (15+ tests)
- **File**: `tests/traits.rs` (new)
- **Status**: ⬜ Not Started
- **Effort**: 4h

---

### 5.3 Standard Library Core

#### 5.3.1 `std.collections.list` - full implementation
- **File**: `std/collections/list.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 4h
- **Details**: map, filter, reduce, find, any, all, take, drop, sort, reverse

#### 5.3.2 `std.collections.map` - full implementation
- **File**: `std/collections/map.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 4h
- **Details**: keys, values, entries, has, delete, merge

#### 5.3.3 `std.collections.set` - implementation
- **File**: `std/collections/set.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 4h

#### 5.3.4 `std.string` - string manipulation
- **File**: `std/string.coral`
- **Status**: ✅ Complete (Basic)
- **Effort**: 2h (remaining: advanced features)
- **Details**:
  - Case conversion: upper, lower
  - Trimming: strip
  - Searching: find, has, begins_with, finishes_with
  - Transformation: sub, divide, part, at, to_list
  - Conversion: parse_int, from_int
  - Utility: len, empty, join_two

#### 5.3.5 `std.math` - math functions
- **File**: `std/math.coral`, `runtime/src/lib.rs`, `src/codegen/mod.rs`
- **Status**: ✅ Complete
- **Effort**: 4h
- **Details**:
  - Constants: pi, half_pi, tau, e
  - Angle conversion: deg_to_rad, rad_to_deg
  - Utility: average, is_positive, is_negative, is_zero, clamp, lerp
  - Geometry: squared, cubed, reciprocal, distance, hypot, magnitude
  - **Runtime intrinsics** (24 functions):
    - Unary: abs, sqrt, floor, ceil, round, trunc, sign
    - Trig: sin, cos, tan, asin, acos, atan
    - Hyperbolic: sinh, cosh, tanh
    - Exponential: exp, ln, log10
    - Binary: pow, min, max, atan2

#### 5.3.6 `std.io.file` - complete file I/O
- **File**: `std/io.coral`
- **Status**: ⚠️ Basic exists
- **Effort**: 4h

#### 5.3.7 `std.io.path` - path manipulation
- **File**: `std/io/path.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 2h

#### 5.3.8 `std.json` - JSON parse/serialize
- **File**: `std/json.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 8h

#### 5.3.9 `std.time` - time/date utilities
- **File**: `std/time.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 4h

#### 5.3.10 `std.error` - error utilities
- **File**: `std/error.coral` (new)
- **Status**: ⬜ Not Started
- **Effort**: 2h

#### 5.3.11 Documentation for all modules
- **Status**: ⬜ Not Started
- **Effort**: 4h

---

## Phase 6: Actor System Completion (Weeks 7-8)

### 6.1 Named Actor Registry

#### 6.1.1-6.1.7 Named actor implementation
- **Status**: ✅ Complete
- **Effort**: 18h total
- **Details**:
  - Added `named_registry: Arc<Mutex<HashMap<String, ActorHandle>>>` to ActorSystem
  - Runtime methods: register, lookup, unregister, spawn_named, send_named, is_name_taken, list_named
  - FFI functions: coral_actor_spawn_named, coral_actor_lookup, coral_actor_register, coral_actor_unregister, coral_actor_send_named, coral_actor_list_named
  - Helper: value_to_rust_string for extracting Rust strings from Value
  - Codegen bindings in src/codegen/runtime.rs
  - Coral wrappers in std/runtime/actor.coral
  - 3 tests in tests/named_actors.rs

### 6.2 Actor Supervision

#### 6.2.1-6.2.6 Supervision tree implementation
- **Status**: ⬜ Not Started
- **Effort**: 24h total

### 6.3 Actor Timers & Scheduling

#### 6.3.1-6.3.5 Timer implementation
- **Status**: ✅ Complete
- **Effort**: 14h total
- **Details**:
  - Added `TimerWheel` with priority queue (BinaryHeap) for scheduling
  - `TimerId` and `TimerToken` for timer identification and cancellation
  - `TimerEntry` with fire_at, target, message, repeat_interval, cancelled
  - Timer worker thread polls heap and fires due timers
  - Runtime methods: send_after, schedule_repeat, cancel_timer, pending_timers
  - FFI functions: coral_timer_send_after, coral_timer_schedule_repeat, coral_timer_cancel, coral_timer_pending_count
  - Codegen bindings in src/codegen/runtime.rs
  - Coral wrappers in std/runtime/actor.coral
  - Added `#` comment support to lexer
  - 6 tests in tests/timers.rs

### 6.4 Typed Message Contracts

#### 6.4.1-6.4.6 Typed messages implementation
- **Status**: ⬜ Not Started
- **Effort**: 20h total

---

## Phase 7: Technical Debt Resolution (Week 9)

### 7.1 Critical Fixes (P0)

- [ ] 7.1.1 Generic type instantiation (`List[T]`, `Map[K,V]`) - 8h
- [ ] 7.1.2 Type parameter tracking in TypeEnv - 4h
- [ ] 7.1.3 List/map element type checking - 4h
- [ ] 7.1.4 Weak reference implementation - 6h
- [ ] 7.1.5 Cycle detection for reference counting - 8h
- [ ] 7.1.6 Document cycle-safe patterns - 2h
- [ ] 7.1.7 Audit store method return types - 2h
- [ ] 7.1.8 Fix Value* return consistency - 2h

### 7.2 High Priority Fixes (P1)

- [ ] 7.2.1 Module caching with content hashing - 4h
- [ ] 7.2.2 Proper namespace scoping for modules - 6h
- [ ] 7.2.3 Circular import detection - 2h
- [ ] 7.2.4 Intern message names to numeric IDs - 4h
- [ ] 7.2.5 Compile-time dispatch table for actors - 4h
- [ ] 7.2.6 Audit refcount operations for ordering - 4h
- [ ] 7.2.7 Use Acquire/Release for cross-thread sharing - 2h

### 7.3 Code Quality (P2)

- [ ] 7.3.1 Split `runtime/src/lib.rs` into modules - 4h
- [ ] 7.3.2-7.3.5 Extract value/list/map/string modules - 8h
- [ ] 7.3.6 Split `src/codegen/mod.rs` - 4h
- [ ] 7.3.7 Audit and remove unwrap() calls - 2h
- [ ] 7.3.8 Add module-level documentation - 4h

---

## Phase 8: Testing & Quality (Week 9-10)

### 8.1 Test Coverage Expansion
- [ ] Type error tests (30 more)
- [ ] Actor tests (15 more)
- [ ] Store tests (12 more)
- [ ] Runtime stress tests (10)
- [ ] Memory leak tests (10)
- [ ] Concurrent tests (10)
- [ ] Error handling tests (20)

### 8.2 Fuzzing & Security
- [ ] Lexer/Parser/Runtime fuzzer setup
- [ ] Audit unsafe blocks
- [ ] Document safety invariants

---

## Phase 9: Documentation & Polish (Week 10)

### 9.1 User Documentation
- [ ] Getting Started guide
- [ ] Language reference
- [ ] Standard library docs
- [ ] Error handling guide
- [ ] Actor programming guide
- [ ] Example programs (5+)
- [ ] Known limitations document

### 9.2 CI/CD
- [ ] CI with fmt/clippy checks
- [ ] CI with AddressSanitizer
- [ ] Release build automation

---

## Quick Start: Next Actions

### Immediate Priorities (Phase 4/5 Completion)
1. ~~**4.2.5**: Nested pattern matching (`Some(Some(x))`) - ~4h~~ ✅ DONE
2. ~~**4.1.9-10**: Error propagation syntax (`! return err`) - ~6h~~ ✅ DONE
3. ~~**6.1.x**: Named actor registry - ~18h~~ ✅ DONE
4. ~~**6.3.x**: Actor timers and scheduling - ~14h~~ ✅ DONE

### High-Value Next (Phase 5/6)
5. **5.3.1-3**: Collection modules (list, map, set) - ~12h
6. **6.2.x**: Actor supervision - ~24h
7. **6.4.x**: Typed message contracts - ~20h

### Current Test Coverage
- **Total tests**: 214
- **ADT tests**: 18 (tests/adt.rs)
- **Pipeline tests**: 11 (tests/pipeline.rs)  
- **Error handling tests**: 14 (tests/error_handling.rs)
- **Math tests**: 31 (tests/math.rs)
- **Nested pattern tests**: 13 (tests/nested_patterns.rs)
- **Named actor tests**: 3 (tests/named_actors.rs)
- **Timer tests**: 6 (tests/timers.rs)

---

## Progress Tracking

| Phase | Tasks | Completed | Progress |
|-------|-------|-----------|----------|
| 4.1 | 14 | 10 | 71% |
| 4.2 | 6 | 5 | 83% |
| 5.1 | 6 | 6 | 100% |
| 5.2 | 9 | 0 | 0% |
| 5.3 | 11 | 2 | 18% |
| 6.x | 25 | 12 | 48% |
| 7.x | 23 | 0 | 0% |
| 8.x | 13 | 0 | 0% |
| 9.x | 11 | 0 | 0% |
| **Total** | **118** | **35** | **30%** |

