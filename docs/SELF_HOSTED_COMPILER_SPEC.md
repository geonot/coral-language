# Coral Self-Hosting Compiler Specification

_Created: 2026-01-06_

## 1. Overview

This document specifies the requirements and architecture for rewriting the Coral compiler in Coral itself (self-hosting). The self-hosted compiler will replace the current Rust implementation while maintaining full compatibility.

### 1.1 Goals

1. **Prove the Language**: A self-hosted compiler demonstrates Coral is capable of complex, real-world programs
2. **Reduce Dependencies**: No longer require Rust toolchain for distribution
3. **Dogfooding**: Force improvements to language features by using them
4. **Community**: Lower barrier for contributors (only need to know Coral)

### 1.2 Non-Goals (Initial Version)

- Matching Rust compiler performance
- Full optimization suite
- IDE integration
- Incremental compilation

---

## 2. Prerequisites

Before self-hosting can begin, these language features must be complete:

### 2.1 Required Language Features

| Feature | Status | Blocking |
|---------|--------|----------|
| Sum types (ADT) | ⚠️ Parsing done | AST representation |
| Exhaustive pattern matching | ⚠️ Partial | AST/IR handling |
| String manipulation | ✅ Basic | Source code handling |
| File I/O | ✅ Basic | Reading source files |
| Maps with string keys | ✅ Working | Symbol tables |
| Lists | ✅ Working | AST children |
| Closures | ✅ Working | Visitor patterns |
| Error handling | ⚠️ No Result type | Compiler errors |
| Byte arrays | ✅ Working | Binary output |

### 2.2 Required Standard Library

```coral
// Must exist before self-hosting
std.io          // File read/write
std.string      // String operations
std.option      // Option type
std.result      // Result type
std.collections // List/Map utilities
std.bytes       // Byte manipulation
```

---

## 3. Architecture

### 3.1 Compiler Pipeline

```
Source Code (.coral)
        │
        ▼
┌───────────────────┐
│      Lexer        │  → Token stream
└───────────────────┘
        │
        ▼
┌───────────────────┐
│      Parser       │  → AST
└───────────────────┘
        │
        ▼
┌───────────────────┐
│   Lowering        │  → Desugared AST
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  Semantic Check   │  → Typed AST + Errors
└───────────────────┘
        │
        ▼
┌───────────────────┐
│   MIR Lowering    │  → MIR
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  LLVM Codegen     │  → LLVM IR (.ll)
└───────────────────┘
        │
        ▼
┌───────────────────┐
│    llc/clang      │  → Binary
└───────────────────┘
```

### 3.2 Module Structure

```
coral-compiler/
├── src/
│   ├── main.coral           # Entry point, CLI
│   ├── lexer/
│   │   ├── mod.coral        # Lexer module
│   │   ├── token.coral      # Token types
│   │   └── span.coral       # Source locations
│   ├── parser/
│   │   ├── mod.coral        # Parser module
│   │   ├── ast.coral        # AST node types
│   │   └── expr.coral       # Expression parsing
│   ├── semantic/
│   │   ├── mod.coral        # Semantic analysis
│   │   ├── types.coral      # Type representations
│   │   ├── solver.coral     # Type constraint solver
│   │   └── env.coral        # Type environments
│   ├── mir/
│   │   ├── mod.coral        # MIR types
│   │   └── lower.coral      # AST → MIR lowering
│   ├── codegen/
│   │   ├── mod.coral        # LLVM IR generation
│   │   ├── runtime.coral    # Runtime bindings
│   │   └── emit.coral       # IR emission helpers
│   └── diagnostics/
│       ├── mod.coral        # Error/warning types
│       └── render.coral     # Error formatting
└── tests/
    └── ...
```

---

## 4. Data Structures

### 4.1 Token Types

```coral
type Token
    | Identifier(name: String, span: Span)
    | Integer(value: Int, span: Span)
    | Float(value: Float, span: Span)
    | String(value: String, span: Span)
    | Keyword(kind: KeywordKind, span: Span)
    | Operator(kind: OpKind, span: Span)
    | Indent(span: Span)
    | Dedent(span: Span)
    | Newline(span: Span)
    | EOF(span: Span)

type KeywordKind
    | Is | If | Else | Match | For | While
    | Fn | Type | Store | Actor | Use | Return
    | True | False | And | Or | Not

type OpKind
    | Plus | Minus | Star | Slash | Percent
    | Eq | Ne | Lt | Le | Gt | Ge
    | Ampersand | Pipe | Caret | Tilde
    | Question | Bang | Dot | Comma | Colon
```

### 4.2 AST Types

```coral
type Expr
    | IntLit(value: Int, span: Span)
    | FloatLit(value: Float, span: Span)
    | StringLit(value: String, span: Span)
    | BoolLit(value: Bool, span: Span)
    | Ident(name: String, span: Span)
    | Binary(op: BinOp, left: Expr, right: Expr, span: Span)
    | Unary(op: UnOp, expr: Expr, span: Span)
    | Call(callee: Expr, args: List[Expr], span: Span)
    | Member(target: Expr, field: String, span: Span)
    | Lambda(params: List[Param], body: Block, span: Span)
    | Match(value: Expr, arms: List[MatchArm], span: Span)
    | Ternary(cond: Expr, then: Expr, else: Expr, span: Span)
    | List(items: List[Expr], span: Span)
    | Map(entries: List[MapEntry], span: Span)

type Stmt
    | Binding(name: String, value: Expr, span: Span)
    | ExprStmt(expr: Expr, span: Span)
    | Return(value: Expr, span: Span)

type Item
    | FnDef(name: String, params: List[Param], body: Block, span: Span)
    | TypeDef(name: String, variants: List[Variant], span: Span)
    | StoreDef(name: String, fields: List[Field], methods: List[FnDef], span: Span)
    | ActorDef(name: String, fields: List[Field], handlers: List[FnDef], span: Span)
    | Use(path: List[String], span: Span)
```

### 4.3 Type System Types

```coral
type Type
    | Primitive(kind: PrimKind)
    | Func(params: List[Type], ret: Type)
    | Generic(name: String, args: List[Type])
    | TypeVar(id: Int)
    | Any
    | Unit

type PrimKind
    | Int | Float | Bool | String | Bytes

type Constraint
    | Equal(a: Type, b: Type, span: Span)
    | Numeric(t: Type, span: Span)
    | Callable(t: Type, args: List[Type], ret: Type, span: Span)
```

---

## 5. Implementation Plan

### Phase 1: Bootstrap Preparation (Weeks 1-2)

**Goal**: Ensure all required language features work

#### Tasks
- [ ] 1.1 Complete ADT construction codegen
- [ ] 1.2 Implement exhaustive match checking
- [ ] 1.3 Add Result/Option to std library
- [ ] 1.4 Test recursive data structures (AST-like)
- [ ] 1.5 Verify file I/O works correctly

### Phase 2: Lexer in Coral (Weeks 3-4)

**Goal**: Lexer that produces same tokens as Rust version

#### Tasks
- [ ] 2.1 Port token types to Coral ADT
- [ ] 2.2 Implement character stream
- [ ] 2.3 Implement indent tracking
- [ ] 2.4 Port all lexer tests
- [ ] 2.5 Verify token-by-token equivalence with Rust lexer

### Phase 3: Parser in Coral (Weeks 5-7)

**Goal**: Parser that produces equivalent AST

#### Tasks
- [ ] 3.1 Port AST types to Coral ADT
- [ ] 3.2 Implement recursive descent parser
- [ ] 3.3 Handle operator precedence
- [ ] 3.4 Handle indent-based blocks
- [ ] 3.5 Port all parser tests
- [ ] 3.6 Compare AST output with Rust parser

### Phase 4: Semantic Analysis in Coral (Weeks 8-10)

**Goal**: Type checking and semantic passes

#### Tasks
- [ ] 4.1 Port type representations
- [ ] 4.2 Implement union-find solver
- [ ] 4.3 Implement constraint generation
- [ ] 4.4 Implement scope analysis
- [ ] 4.5 Port semantic tests

### Phase 5: MIR in Coral (Weeks 11-12)

**Goal**: Intermediate representation

#### Tasks
- [ ] 5.1 Design MIR types in Coral
- [ ] 5.2 Implement AST → MIR lowering
- [ ] 5.3 Port const evaluator
- [ ] 5.4 Add basic optimizations

### Phase 6: Codegen in Coral (Weeks 13-16)

**Goal**: LLVM IR text output

#### Tasks
- [ ] 6.1 Implement LLVM IR text emission
- [ ] 6.2 Port runtime bindings
- [ ] 6.3 Generate all expression forms
- [ ] 6.4 Generate function bodies
- [ ] 6.5 Generate store/actor constructors
- [ ] 6.6 Invoke llc/clang for binary output

### Phase 7: Integration & Testing (Weeks 17-18)

**Goal**: Full compiler working end-to-end

#### Tasks
- [ ] 7.1 CLI argument parsing
- [ ] 7.2 Error formatting and display
- [ ] 7.3 Run full test suite
- [ ] 7.4 Benchmark against Rust compiler
- [ ] 7.5 Document differences/improvements

### Phase 8: Self-Compilation (Week 19-20)

**Goal**: Compiler can compile itself

#### Tasks
- [ ] 8.1 Compile lexer with Coral compiler
- [ ] 8.2 Compile parser with Coral compiler
- [ ] 8.3 Compile full compiler with Coral compiler
- [ ] 8.4 Verify output equivalence
- [ ] 8.5 Document bootstrap process

---

## 6. Testing Strategy

### 6.1 Equivalence Testing

For each phase, test that the Coral implementation produces identical output to the Rust implementation:

```coral
*test_lexer_equivalence()
    sources is list_test_files("tests/fixtures/lexer/")
    for source in sources
        rust_tokens is rust_lexer.lex(source)
        coral_tokens is coral_lexer.lex(source)
        assert_eq(rust_tokens, coral_tokens)
```

### 6.2 Regression Testing

All existing tests must pass with the Coral compiler.

### 6.3 Fuzzing

Generate random valid/invalid programs and verify the compiler doesn't crash.

---

## 7. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Missing language features | High | Blocks progress | Prioritize prerequisites |
| Performance too slow | Medium | User experience | Profile and optimize hot paths |
| Memory leaks (cycles) | High | Compiler OOM | Implement cycle detection first |
| Subtle semantic differences | Medium | Wrong compilation | Extensive equivalence testing |
| LLVM IR incompatibility | Low | Won't link | Test early with llc |

---

## 8. Success Criteria

1. **Correctness**: Coral compiler produces identical LLVM IR to Rust compiler
2. **Self-Hosting**: Coral compiler can compile itself
3. **Performance**: Compilation time within 5x of Rust compiler
4. **Tests**: All existing tests pass
5. **Documentation**: Complete bootstrap guide

---

## 9. Timeline Summary

| Phase | Duration | Milestone |
|-------|----------|-----------|
| 1. Preparation | 2 weeks | Prerequisites complete |
| 2. Lexer | 2 weeks | Lexer in Coral |
| 3. Parser | 3 weeks | Parser in Coral |
| 4. Semantic | 3 weeks | Type checker in Coral |
| 5. MIR | 2 weeks | MIR in Coral |
| 6. Codegen | 4 weeks | Full compiler in Coral |
| 7. Integration | 2 weeks | Tests passing |
| 8. Bootstrap | 2 weeks | Self-hosting achieved |

**Total Estimated Time**: 20 weeks (~5 months)

---

## 10. Future Enhancements

After initial self-hosting:

1. **Incremental Compilation**: Track dependencies, only recompile changed modules
2. **Better Error Messages**: Rich diagnostics with suggestions
3. **Optimization Passes**: More MIR optimizations
4. **Language Server**: IDE support via LSP
5. **REPL**: Interactive Coral shell
6. **Alternative Backends**: WASM, native code generation without LLVM
