# Coral Compiler Gap Analysis

_Last updated: 2025-12-04_

This note documents the major deficiencies that block Coral from self-hosting or being production ready. Each section lists current status, blockers, and recommended next steps.

## 1. Frontend completeness

| Area | Current status | Gaps | Next steps |
| --- | --- | --- | --- |
| Lexer/layout | Handles indentation, dedent recovery, and tab/space enforcement. | No Unicode escapes, string interpolation only supports `${expr}` style baked into parser. | Extend lexer to emit `TEMPLATE_START/MID/END` tokens and add escape table. |
| Parser | Covers expressions, match, layout blocks, stores/types syntax. | Only reports first error per file, no recovery for many constructs, lambda syntax for placeholders is implicit. | Introduce Pratt parser for expressions with better diagnostics and implement lambda literal syntax in grammar. |
| Semantic | Detects duplicate names, default-parameter ordering issues. | No undefined-variable check, no store/type validation, no type inference. | Build symbol tables, implement flow-sensitive name resolution, add constraint-based typer. |

## 2. MIR and lowering

- **Status:** MIR structs/interpreter exist, but AST→MIR lowering only supports literals, binary ops, simple calls.
- **Gaps:** No representation for lists/maps/match, no closure or capture handling, no store/actor lowering, no interface to LLVM backend.
- **Plan:**
  1. Finish lowering coverage for all expression forms, produce SSA-like temps.
  2. Add MIR-to-LLVM pass that substitutes runtime intrinsics.
  3. Validate equivalence by comparing MIR interpreter output vs. current LLVM pipeline on fixture programs.

## 3. Runtime & memory model

- Tagged `Value` runtime with manual refcounts is functional but lacks cycle handling, hashing, or stress testing.
- Maps are O(n) scans. Closures don't yet capture environment lifetimes across threads.
- Need instrumentation (ASAN/Valgrind) and benchmark harnesses.
- **Action:** implement SipHash-based maps, dedicated closure objects, and region-based GC prototype for long-lived actors.

## 4. Module system & stdlib

- `use module` works by source splicing; there is no module graph, caching, or namespace isolation.
- Stdlib is just a few bindings/functions; no IO, collections, or math utilities.
- **Action:** implement module resolver that caches parsed modules, enforce symbol visibility rules, and grow `std` with Coral-written helpers to dogfood the language.

## 5. Tooling & pipeline gaps

- CLI always recompiles runtime, lacks incremental caching of modules, and cannot output packaged artifacts beyond raw binaries.
- No formatter, language server, or package manager.
- **Action:** add fingerprints for modules/runtime, wire `coralc run/build/test` subcommands, and spin up fmt/lint prototypes (possibly implemented in Coral Core).

## 6. Path to self-hosting

1. **Stabilize Coral Core**: freeze syntax subset, implement std helpers, and write multiple utilities purely in Coral.
2. **Implement Coral compiler stages in Coral**: start with lexer+parser, then MIR lowering.
3. **Bootstrapping**: compile the Coral-implemented compiler using Rust `coralc`, then use the resulting binary to rebuild itself. Requires reproducible builds and deterministic runtime.
4. **Ecosystem**: add package/module metadata and testing harness so Coral-written compiler has supporting tools.

Tracking these gaps explicitly will help prioritize engineering work for the next milestones.
