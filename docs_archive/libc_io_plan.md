# Libc & File I/O Integration Plan

_Last updated: 2025-12-04_

## Goals
- Provide safe, portable file-system helpers without exposing raw libc everywhere.
- Keep the Coral surface "batteries included" via `std.io` module built entirely in Coral.
- Allow future embedding targets to swap implementations (WASM sandbox, embedded RTOS, etc.).

## Current implementation
1. **Runtime exports**: `coral_fs_read`, `coral_fs_write`, `coral_fs_exists` . They operate on Coral `Value` handles to avoid manual pointer juggling.
2. **Standard library shim**: `std/io.coral` provides idiomatic Coral functions (`io.read`, `io.write`, `io.exists`) that wrap the builtins so user code never calls the raw runtime directly.
3. **Tests**: Runtime unit tests create temp files via the new APIs to ensure the bridge works and releases handles correctly.

## Next steps
- **Streaming APIs**: expose buffered readers/writers with coroutine-friendly adapters for future async runtime.
- **Directory helpers**: implement `io.list(dir)` returning Coral lists of strings, `io.mkdir(path)`, and `io.remove(path)`.
- **Virtual FS hook**: allow embedding hosts (e.g., game engines) to provide custom VFS callbacks so Coral code can target in-memory files.
- **Permission sandbox**: integrate allow-lists for accessible paths so CLI projects can declaratively limit file access during comptime execution.

## Relationship to libc
- Runtime stays in Rust and only calls libc via `std::fs`, which keeps portability and reduces manual `unsafe` usage.
- For niches where direct libc/FD operations are required (sockets, polling), introduce dedicated runtime modules (`coral_net_*`) with capability-based APIs rather than general `unsafe` escape hatches.
- Documented strategy lets us swap to platform-specific shims (WASI, Windows API) without rewriting the Coral stdlib surface.
