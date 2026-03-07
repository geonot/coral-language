# Coral Bootstraps: The Compiler Can Compile Itself

**March 7, 2026**

---

Today we hit the most significant milestone in Coral's history: **the self-hosted compiler successfully bootstraps**. The Coral compiler, written in Coral, can compile itself — and when the output compiles itself again, the result is byte-for-byte identical.

This is the moment a language goes from "interesting experiment" to "real."

## What Happened

We ran the full three-generation bootstrap chain:

| Stage | What | Result |
|-------|------|--------|
| **gen1** | Rust compiler compiles `self_hosted/*.coral` → LLVM IR | 52,037 lines of IR |
| **gen2** | gen1 binary compiles `self_hosted/*.coral` → LLVM IR | 55,235 lines of IR |
| **gen3** | gen2 binary compiles `self_hosted/*.coral` → LLVM IR | 55,235 lines of IR |

**gen2 and gen3 are identical.** Zero diff. Byte-for-byte. This is the gold standard for compiler self-hosting — it proves the self-hosted compiler is a fixed point. It faithfully reproduces itself.

The gen1→gen2 difference (52K vs 55K lines) is expected: the Rust compiler and the Coral compiler make slightly different codegen choices (register naming, instruction ordering, constant layout). What matters is that once the self-hosted compiler is in the loop, it's perfectly stable.

All 30 self-hosting tests pass, including 7 end-to-end tests that compile Coral programs through the self-hosted compiler and execute the results.

## Why This Matters

Self-hosting is the rite of passage for a programming language. It answers a brutal question: **is this language capable of building complex, real software?** A compiler is one of the most demanding programs to write — it needs string processing, tree traversal, pattern matching, code generation, error handling, and file I/O, all working together correctly.

When a language can build its own compiler, and that compiler can build itself, it proves:

1. **The language works.** Not just for toy programs — for a 7,690-line codebase spanning 7 modules with complex control flow, recursive data structures, and cross-module dependencies.

2. **The compiler is correct.** The fixed-point property (gen2 == gen3) means the compiler isn't introducing subtle bugs during compilation. If it were miscompiling anything, the output would drift with each generation.

3. **The runtime is solid.** The self-hosted compiler runs on Coral's own runtime — the reference-counted value system, string operations, list/map manipulation, file I/O — all exercised under real workload. No crashes, no leaks, no corruption.

4. **The language is expressive enough.** Writing a compiler without type annotations, using `is` for binding, `*` for function declarations, `~` for pipelines, and `match` for pattern dispatch — the language design holds up under serious pressure.

## The Journey Here

Getting to this point required building up from nothing through four distinct phases:

**Phase A** — Built the self-hosted lexer and parser. Coral's indent-aware tokenizer and recursive-descent parser, reimplemented in Coral itself. 2,380 lines handling 30+ keywords, full expression precedence, 25+ AST node types.

**Phase B** — Added the lowering pass and module loader. Pipeline desugaring (`a ~ f` → `f(a)`), guard statement lowering, default parameter injection, `use` directive resolution, circular import detection. 949 lines.

**Phase C** — The heavy lift: semantic analysis and code generation. A union-find type inference engine, constraint generation and solving, scope analysis, closure capture analysis, and a full LLVM IR text emitter handling 130+ runtime function declarations. 4,361 lines. At the end of Phase C, all 7 modules compiled to IR but had never been *executed*.

**Phase D** — The bootstrap phase. This is where we fixed every cross-module data-format bug, achieved first execution, progressively tested increasingly complex programs, and finally ran the full self-compile chain. The gap between "compiles to IR" and "actually runs correctly" was substantial — dozens of issues from format mismatches to missing edge cases in codegen. Each one had to be hunted down while executing the compiler on itself.

## By the Numbers

| Metric | Value |
|--------|-------|
| Self-hosted compiler | 7,690 lines of Coral |
| Modules | 7 (lexer, parser, lower, module_loader, semantic, codegen, compiler) |
| Rust reference compiler | ~16,000 lines |
| Compression ratio | 2.1x (Coral is terser — no type annotations, no braces, no semicolons) |
| Bootstrap gen2 IR | 55,235 lines |
| gen2 vs gen3 diff | **0 lines** |
| Self-hosting tests | 30/30 passing |
| Total project tests | 745+ (0 failures) |
| Runtime FFI surface | 220+ functions |
| Standard library | 20 modules, 1,700+ lines |

## What the Compiler Looks Like

This is a real language doing real work. Here's a taste of what the self-hosted codegen looks like — emitting LLVM IR from Coral:

```coral
*emit_function(ctx, func)
    name is func.get('name')
    params is func.get('params')
    body is func.get('body')

    param_types is []
    for p in params
        param_types.push('ptr')

    sig is 'define ptr @{name}({param_types ~ join(", ")})'
    ctx.emit(sig)
    ctx.emit('{')

    ctx.emit('entry:')
    for stmt in body
        emit_statement(ctx, stmt)

    ctx.emit('}')
```

No type annotations. No braces for blocks. Pipeline operator for string joining. Template strings for IR emission. It reads like pseudocode but compiles to native code via LLVM.

## What's Next

The bootstrap milestone completes **Stream 5** (Self-Hosted Compiler) of our alpha roadmap. Here's what remains:

- **Self-hosted runtime** — Rewriting the Coral runtime (currently 23,000 lines of Rust) in Coral itself. This is the final piece for full self-hosting: the value system, reference counting, cycle detection, actor scheduler, and persistent store engine, all in Coral.

- **Performance tuning** — The self-hosted compiler currently runs slower than the Rust version (expected — it's an unoptimized first generation). Escape analysis, numeric unboxing, and LLVM optimization pass wiring will close the gap.

- **Actor system hardening** — Typed messages, actor monitoring, supervision budget enforcement, and graceful shutdown.

- **Persistent stores** — Query syntax from the language level, ACID transactions, and index management.

But today, we celebrate. A language that can build itself is a language that can build anything.

---

*Coral is an open-source programming language combining Python-like ergonomics with native performance, built-in actors, persistent stores, and automatic memory management. Learn more at the [project repository](https://github.com/example/coral).*
