# Coral Core Spec (Minimal subset for bootstrapping)

This document defines the Coral "Core" — the minimal language subset we will stabilize and use to bootstrap further compiler work and eventually self-hosting.

Design goals
- Small, orthogonal syntax that maps cleanly to MIR.
- Deterministic semantics with minimal runtime dependencies.
- Expressive enough to implement compiler plumbing (parser, AST transforms, simple codegen) in Coral itself.

Core syntax
- Top-level forms allowed in Core v1:
	- Bindings (`name is expr`)
	- Function definitions (`*name(param, other ? default)`)
	- `use module.path` imports (resolved to `.coral` siblings or `std/` tree)
	- Simple `type` declarations (fields only; methods deferred)
- Expressions (all of these must lower cleanly to MIR):
	- Literals: integers, floats, bools, strings, taxonomy paths, `()`
	- Collections: list literals `[expr, ...]`, map literals `map('key' is expr, ...)`
	- Calls & members: `callee(arg)`, `value.method`
	- Logic & arithmetic: `+ - * / and or not`
	- Conditionals: ternary `cond ? a ! b`, `match value` with literal patterns
	- Lambdas using placeholder sugar (future work) or explicit `*(x, y) ...`

Types
- Core initially uses a dynamic `Value` tagged runtime type: Number, Bool, String, List, Map, Unit.
- Optional annotations `: Number` may be added later to help the type inferencer; not required for bootstrapping.

Standard library
- Small std modules implemented in Coral: `std.prelude`, `std.math`, `std.collections` (thin wrappers over `coral_*` runtime shims).
- Core utilities must rely exclusively on these std modules to avoid reimplementing runtime helpers.

Semantics
- Evaluation is left-to-right for calls and binary operators.
- Short-circuiting for `and`/`or`.
- `match` patterns are exact by value for integers/strings; pattern bindings introduce block-local variables.

Bootstrap subset (must be expressible in Coral Core)
- Parser: tokenization and simple parse combinators (can be expressed in Coral with limited recursion depth).
- AST: simple algebraic data structures (records) to represent nodes.
- MIR builder: a small set of helpers to emit MIR nodes.
- Interpreter: an evaluator for MIR to verify behavior.

Testing & verification
- Real Coral samples now live under `tests/fixtures/programs/core_examples.coral` and are compiled in `tests/core_spec.rs`.
- Each new Core feature must add at least one Coral-only utility (formatter, doc emitter, MIR printer) to dogfood the subset.
- Keep a fixed regression suite in `tests/bootstrap/` to ensure the subset remains stable.

Reference sample

```coral
use std.prelude
use std.math

*main()
	angle is deg_to_rad(90)
	log_line('core-subset demo: ' + angle)
```

This file (`tests/fixtures/programs/core_examples.coral`) compiles today and should remain valid as the canonical Core smoke test.

Roadmap
1. Define exact function/lambda syntax and scoping rules.
2. Stabilize the subset with examples (see `tests/fixtures/programs/core_examples.coral`) and write Coral implementations of small tools (AST dumper, pretty-printer).
3. Port parser (or parts of it) to Coral and iterate until stable.
