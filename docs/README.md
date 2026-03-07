# Coral Documentation

_Updated: 2026-03-02_

## Documentation Index

### Status & Planning

| Document | Description |
|----------|-------------|
| [ALPHA_ROADMAP.md](ALPHA_ROADMAP.md) | **Authoritative roadmap** — all alpha goals, work streams, task tracking, known bugs |
| [SELF_HOSTING_STATUS.md](SELF_HOSTING_STATUS.md) | Self-hosted compiler progress and completion plan |
| [STDLIB_STATUS.md](STDLIB_STATUS.md) | Per-module standard library assessment and gaps |
| [REVIEW_REPORT_FEB2026.md](REVIEW_REPORT_FEB2026.md) | Comprehensive codebase review (historical reference) |

### Language Reference

| Document | Description |
|----------|-------------|
| [syntax.coral](syntax.coral) | Canonical syntax reference — all language constructs |
| [VALUE_ERROR_MODEL.md](VALUE_ERROR_MODEL.md) | Value and error model design spec |
| [CYCLE_SAFE_PATTERNS.md](CYCLE_SAFE_PATTERNS.md) | Guide to cycle-safe programming patterns |
| [CORAL_LANGUAGE_SHOWCASE.md](CORAL_LANGUAGE_SHOWCASE.md) | Language feature showcase |
| [stores_guide.md](stores_guide.md) | Working guide for Coral stores |

### Specification Documents (Forward-Looking)

| Document | Description |
|----------|-------------|
| [SELF_HOSTED_COMPILER_SPEC.md](SELF_HOSTED_COMPILER_SPEC.md) | Requirements for Coral-written compiler |
| [SELF_HOSTED_RUNTIME_SPEC.md](SELF_HOSTED_RUNTIME_SPEC.md) | Requirements for Coral-written runtime |
| [PERSISTENT_STORE_SPEC.md](PERSISTENT_STORE_SPEC.md) | Persistent object storage design spec |
| [ACTOR_SYSTEM_COMPLETION.md](ACTOR_SYSTEM_COMPLETION.md) | Actor model completion plan |
| [STANDARD_LIBRARY_SPEC.md](STANDARD_LIBRARY_SPEC.md) | Target standard library specification |
| [COMPILATION_TARGETS.md](COMPILATION_TARGETS.md) | Compilation targets spec (WASM, cross-compilation — future) |
| [LIBC_INDEPENDENCE.md](LIBC_INDEPENDENCE.md) | libc independence design spec (future) |

## Quick Start

1. **New to Coral?** Start with the [project README](../README.md) for overview and usage
2. **Where are we headed?** See [ALPHA_ROADMAP.md](ALPHA_ROADMAP.md) — the single source of truth for all alpha goals
3. **Self-hosting progress?** See [SELF_HOSTING_STATUS.md](SELF_HOSTING_STATUS.md)
4. **Standard library?** See [STDLIB_STATUS.md](STDLIB_STATUS.md)
5. **Bug fix history?** See [../REMEDIATION_TRACKER.md](../REMEDIATION_TRACKER.md) in the project root

## Document Status

| Document | Status | Last Updated |
|----------|--------|--------------|
| ALPHA_ROADMAP | ✅ **Authoritative** | 2026-03-02 |
| SELF_HOSTING_STATUS | ✅ Current | 2026-03-02 |
| STDLIB_STATUS | ✅ Current | 2026-03-02 |
| REVIEW_REPORT_FEB2026 | 📄 Historical | 2026-02-16 |
| syntax.coral | ✅ Current | 2026-01 |
| VALUE_ERROR_MODEL | ✅ Current | 2026-01-06 |
| CYCLE_SAFE_PATTERNS | ⚠️ Code examples need update | 2026-01 |
| CORAL_LANGUAGE_SHOWCASE | ⚠️ Code samples need update | 2026-01 |
| stores_guide | ⚠️ Needs rewrite | 2025-01 |
| SELF_HOSTED_COMPILER_SPEC | ⚠️ References MIR (removed) | 2026-01-06 |
| SELF_HOSTED_RUNTIME_SPEC | ✅ Current (spec) | 2026-01-06 |
| PERSISTENT_STORE_SPEC | 🔮 Future spec | 2026-01-06 |
| ACTOR_SYSTEM_COMPLETION | ⚠️ Partially done | 2026-01-06 |
| STANDARD_LIBRARY_SPEC | 🔮 Target spec | 2026-01-06 |
| COMPILATION_TARGETS | 🔮 Future spec | 2026-01-06 |
| LIBC_INDEPENDENCE | 🔮 Future spec | 2026-01-06 |

## Documentation Policy

All authoritative documentation is maintained in this folder. The project root contains only [README.md](../README.md) (project overview) and [REMEDIATION_TRACKER.md](../REMEDIATION_TRACKER.md) (live bug fix tracker). All other documentation lives here.
