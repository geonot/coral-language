# Coral Documentation

_Updated: 2026-03-09_

## Quick Start

1. **New to Coral?** Start with the [project README](../README.md) for overview and usage
2. **Where are we headed?** See [LANGUAGE_EVOLUTION_ROADMAP.md](LANGUAGE_EVOLUTION_ROADMAP.md) — the comprehensive roadmap across 6 pillars
3. **What's done?** See [EVOLUTION_PROGRESS.md](EVOLUTION_PROGRESS.md) — session-by-session implementation progress
4. **Self-hosting?** Bootstrap is complete (gen2 == gen3). See [SELF_HOSTED_COMPILER_SPEC.md](SELF_HOSTED_COMPILER_SPEC.md)
5. **Agent onboarding?** See [LLM_ONBOARDING.md](LLM_ONBOARDING.md) for the development workflow guide

## Status & Planning

| Document | Description | Status |
|----------|-------------|--------|
| [LANGUAGE_EVOLUTION_ROADMAP.md](LANGUAGE_EVOLUTION_ROADMAP.md) | **Authoritative roadmap** — 6 pillars, all tasks, implementation phases | ✅ Authoritative |
| [EVOLUTION_PROGRESS.md](EVOLUTION_PROGRESS.md) | Implementation progress — session log, task completion status | ✅ Current |
| [LLM_ONBOARDING.md](LLM_ONBOARDING.md) | Agent onboarding — project layout, design principles, workflow | ✅ Current |

## Language Reference

| Document | Description | Status |
|----------|-------------|--------|
| [syntax.coral](syntax.coral) | Canonical syntax reference — all constructs | ✅ Current |
| [VALUE_ERROR_MODEL.md](VALUE_ERROR_MODEL.md) | Value and error model design spec | ✅ Current |
| [CORAL_LANGUAGE_SHOWCASE.md](CORAL_LANGUAGE_SHOWCASE.md) | Language feature showcase | ⚠️ Some code samples aspirational |
| [CYCLE_SAFE_PATTERNS.md](CYCLE_SAFE_PATTERNS.md) | Guide to cycle-safe programming patterns | ⚠️ Code examples need update |

## Specification Documents (Forward-Looking)

| Document | Description | Status |
|----------|-------------|--------|
| [SELF_HOSTED_RUNTIME_SPEC.md](SELF_HOSTED_RUNTIME_SPEC.md) | Requirements for Coral-written runtime | 🔮 Future spec (Phase Epsilon) |
| [PERSISTENT_STORE_SPEC.md](PERSISTENT_STORE_SPEC.md) | Persistent object storage design spec | 🔮 Partially implemented |
| [ACTOR_SYSTEM_COMPLETION.md](ACTOR_SYSTEM_COMPLETION.md) | Actor model — remaining work (typed msgs, remote) | ⚠️ Core done, extensions in R2 |
| [STANDARD_LIBRARY_SPEC.md](STANDARD_LIBRARY_SPEC.md) | Target standard library specification | 🔮 Target spec |
| [COMPILATION_TARGETS.md](COMPILATION_TARGETS.md) | WASM, cross-compilation targets | 🔮 Future spec (CC4) |
| [LIBC_INDEPENDENCE.md](LIBC_INDEPENDENCE.md) | libc independence design | 🔮 Future spec |
| [SELF_HOSTED_COMPILER_SPEC.md](SELF_HOSTED_COMPILER_SPEC.md) | Self-hosting architecture (✅ completed) | ✅ Reference |
| [BLOG_BOOTSTRAP_MILESTONE.md](BLOG_BOOTSTRAP_MILESTONE.md) | Bootstrap milestone announcement | ✅ Historical |

## Documentation Policy

All authoritative documentation is maintained in this folder. The project root contains [README.md](../README.md) (project overview) and [INTRODUCTION.md](../INTRODUCTION.md) (language introduction). The single source of truth for all planning is [LANGUAGE_EVOLUTION_ROADMAP.md](LANGUAGE_EVOLUTION_ROADMAP.md).
