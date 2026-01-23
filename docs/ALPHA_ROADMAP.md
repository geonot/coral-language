# Coral Alpha Roadmap

_Created: 2026-01-06_

## Executive Summary

This document provides a fresh assessment of Coral's current state and defines the path to alpha release. After completing phases 1-3 of the original action plan, Coral has a solid foundation with 100+ tests passing, working type inference, actor state access, and functioning HOF (higher-order function) runtime.

---

## 1. Current State (January 2026)

### 1.1 What's Working

| Component | Status | Notes |
|-----------|--------|-------|
| **Lexer** | ✅ Complete | Indent-aware, tab/space handling, mixed-indent rejection |
| **Parser** | ✅ Complete | All syntax forms, ADT variants, match patterns |
| **Type System** | ✅ Functional | HM inference, union-find solver, constraint propagation |
| **Semantic Analysis** | ✅ Functional | Undefined name detection, arity checking, scope analysis |
| **MIR** | ⚠️ Basic | Simple IR, const evaluation, needs optimization passes |
| **LLVM Codegen** | ✅ Working | All expressions, functions, closures, stores, actors |
| **Runtime** | ✅ Working | Tagged values, refcounting, lists, maps, strings, bytes |
| **Actor System** | ⚠️ MVP | M:N scheduler, bounded mailboxes, backpressure, no named actors |
| **Stores** | ⚠️ Basic | Fields, methods, reference fields, no persistence |
| **Standard Library** | ⚠️ Minimal | prelude, io, bit, math, runtime modules |

### 1.2 Test Coverage

- **Total Tests**: 253 passing, 0 failing
- **Unit Tests**: 24 (type system, MIR, module loader)
- **Integration Tests**: 229 (parser, semantic, codegen, modules, traits, error handling, ADT)

### 1.3 Code Metrics

| File | Lines | Status |
|------|-------|--------|
| `src/codegen/mod.rs` | 3264 | ⚠️ Large, but modularized |
| `runtime/src/lib.rs` | 3804 | ⚠️ Large, needs splitting |
| `src/parser.rs` | ~1900 | Acceptable |
| `src/semantic.rs` | ~1500 | Acceptable |

---

## 2. Alpha Definition

### 2.1 Alpha Requirements

An alpha release must be:
1. **Honest**: All documented features work correctly
2. **Useful**: Non-trivial programs can be written
3. **Safe**: No silent memory corruption or crashes
4. **Documented**: Clear guides for what works and what doesn't

### 2.2 Alpha Feature Set

#### Must Have (P0)
- [x] Basic types (numbers, bools, strings, bytes, lists, maps)
- [x] Functions with parameters and closures
- [x] Type inference with error messages
- [x] Match expressions
- [x] Module imports
- [x] Actor spawn/send with state
- [x] Store field access (in-memory, no persistence)
- [x] ADT (sum type) construction and matching
- [x] Complete standard library core (list, map, set, string, math, io)

#### Should Have (P1)
- [x] Named actors
- [ ] Actor supervision (restart policies)
- [ ] Typed message contracts
- [x] Pipeline operator (`~`)
- [x] Comprehensive error messages with suggestions
- [x] Trait/mixin system
- [x] Value-error model with propagation

#### Nice to Have (P2)
- [ ] Remote actors (networking)
- [ ] Store persistence (basic)
- [ ] REPL

---

## 3. Technical Debt & Issues

### 3.1 Critical Issues

| Issue | Impact | Effort | Priority |
|-------|--------|--------|----------|
| No cycle detection in RC | Memory leaks | High | P1 |
| Generic types declared but not instantiated | Type safety gaps | Medium | P0 |
| Module system is string expansion | No caching, no scoping | Medium | P1 |
| `codegen/mod.rs` too large | Maintainability | Medium | P2 |
| `runtime/lib.rs` too large | Maintainability | Medium | P2 |

### 3.2 Technical Debt Inventory

#### Type System
- [ ] `List[T]`, `Map[K,V]` generic instantiation not implemented
- [ ] Type annotations parsed but not enforced at boundaries
- [ ] No effect typing (IO, Actor, Comptime)
- [ ] No trait/interface system

#### Memory Management
- [ ] Reference counting has no cycle detection
- [ ] 64-bit counters exist but aren't atomic in all paths
- [ ] No arenas for stack-like temporaries
- [ ] Copy-on-write not implemented

#### Actors
- [ ] No named actor registry
- [ ] No remote actors / networking
- [ ] String-based dispatch (could be optimized to interned IDs)
- [ ] No typed message contracts
- [ ] No supervision policies (restart, escalate, stop)
- [ ] No actor timers or cancellation tokens

#### Stores
- [ ] No persistence mechanism
- [ ] Only `self.field is value` in methods works
- [ ] No type checking for reference fields
- [ ] No circular reference detection
- [ ] No weak references

#### Compiler
- [ ] MIR is trivial - no real optimization passes
- [ ] No inlining
- [ ] No escape analysis
- [ ] No dead code elimination
- [ ] No incremental compilation

---

## 4. Alpha Milestones

### Phase 4: ADT Completion (Week 1-2)

**Goal**: Sum types work end-to-end

#### Tasks
- [ ] 4.1 ADT construction codegen (`Some(value)`, `None`)
- [ ] 4.2 Pattern matching on ADT variants
- [ ] 4.3 Exhaustiveness checking for match
- [ ] 4.4 `Option` and `Result` in standard library
- [ ] 4.5 Test coverage for ADT edge cases

**Definition of Done**: Can define, construct, and match on custom sum types.

### Phase 5: Standard Library Core (Week 2-3)

**Goal**: Useful standard library for alpha users

#### Tasks
- [ ] 5.1 `std.collections` - List/Map utilities
- [ ] 5.2 `std.string` - String manipulation
- [ ] 5.3 `std.option` - Option type and helpers
- [ ] 5.4 `std.result` - Result type and error handling
- [ ] 5.5 `std.io` - File I/O completion
- [ ] 5.6 `std.json` - JSON parsing/serialization
- [ ] 5.7 Documentation for all modules

### Phase 6: Actor System Completion (Week 3-4)

**Goal**: Production-ready actor model

#### Tasks
- [ ] 6.1 Named actor registry
- [ ] 6.2 Actor supervision policies
- [ ] 6.3 Actor timers
- [ ] 6.4 Typed message contracts (compile-time)
- [ ] 6.5 Better error propagation

### Phase 7: Polish & Documentation (Week 4-5)

**Goal**: Release-ready quality

#### Tasks
- [ ] 7.1 Getting Started guide
- [ ] 7.2 Language reference
- [ ] 7.3 Standard library docs
- [ ] 7.4 Example programs (5+)
- [ ] 7.5 Known limitations document
- [ ] 7.6 CI/CD with fmt/clippy/ASAN

---

## 5. Post-Alpha Roadmap

### Beta Priorities
1. **Performance**: Typed MIR paths, escape analysis, inlining
2. **Persistence**: Store serialization and recovery
3. **Networking**: Remote actors
4. **Tooling**: REPL, language server, formatter

### Self-Hosting Requirements
1. Algebraic data types (for AST) ✅ Parsing done
2. Exhaustive pattern matching ⚠️ In progress
3. String manipulation
4. File I/O ✅ Basic working
5. Error recovery in parser

---

## 6. Success Metrics

| Metric | Alpha Target | Current |
|--------|--------------|---------|
| Tests Passing | 100% | 100% |
| Type Error Coverage | 30+ test cases | ~20 |
| Actor Tests | 15+ | ~5 |
| Store Tests | 10+ | ~3 |
| Example Programs | 5+ | 2 |
| Documentation Pages | 15+ | ~8 |
| Clippy Warnings | 0 | Unknown |

---

## 7. Timeline

| Week | Focus | Deliverables |
|------|-------|--------------|
| 1 | ADT Completion | Working sum types, pattern matching |
| 2 | Standard Library | Core modules documented |
| 3 | Actor System | Named actors, supervision |
| 4 | Polish | Documentation, examples |
| 5 | Release Prep | CI/CD, binaries, announcement |

**Target Alpha Release**: February 2026
