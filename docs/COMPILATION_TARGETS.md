# Coral Compilation Targets

_Created: 2026-01-06_

## Executive Summary

Coral compiles through LLVM IR, unlocking access to many compilation targets. This document covers supported targets, requirements, and implementation status.

---

## 1. Target Overview

### 1.1 Currently Supported

| Target | Status | Binary Format | Notes |
|--------|--------|---------------|-------|
| Linux x86_64 | ✅ Full | ELF | Primary development target |
| Linux aarch64 | ⚠️ Partial | ELF | Needs testing |
| macOS x86_64 | ⚠️ Partial | Mach-O | Needs syscall shims |
| macOS aarch64 | ⚠️ Partial | Mach-O | Apple Silicon |

### 1.2 Planned Targets

| Target | Status | Binary Format | Notes |
|--------|--------|---------------|-------|
| WASM32 | 🔜 Planned | WebAssembly | Browser/WASI support |
| Windows x86_64 | 🔜 Planned | PE/COFF | Win32 API integration |
| Linux RISC-V | 🔜 Planned | ELF | Emerging architecture |
| Bare Metal ARM | 🔮 Future | Raw | Embedded systems |

---

## 2. Native Targets (ELF/Mach-O/PE)

### 2.1 Linux x86_64 (Primary)

**Triple**: `x86_64-unknown-linux-gnu`

**Characteristics**:
- 64-bit pointers
- System V ABI calling convention
- ELF binary format
- Can link with libc or run standalone

**Build Command**:
```bash
coralc program.coral -o program
# or with explicit target
coralc program.coral --target x86_64-unknown-linux-gnu -o program
```

**Output Sizes** (approximate):
- Hello World with libc: ~15KB
- Hello World standalone: ~8KB
- With runtime: ~100KB
- Full std library: ~500KB

### 2.2 Linux aarch64 (ARM64)

**Triple**: `aarch64-unknown-linux-gnu`

**Requirements**:
- Cross-compilation toolchain or native ARM64 hardware
- ARM64 syscall numbers (different from x86_64)
- NEON SIMD instructions available

**Build Command**:
```bash
coralc program.coral --target aarch64-unknown-linux-gnu -o program
```

### 2.3 macOS x86_64 / aarch64

**Triples**: 
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

**Differences from Linux**:
- Mach-O binary format (not ELF)
- Different syscall numbers and conventions
- Universal binary support (fat binaries with both architectures)

**Build Command**:
```bash
# Intel Mac
coralc program.coral --target x86_64-apple-darwin -o program
# Apple Silicon
coralc program.coral --target aarch64-apple-darwin -o program
# Universal binary
coralc program.coral --universal -o program
```

### 2.4 Windows x86_64

**Triple**: `x86_64-pc-windows-msvc`

**Requirements**:
- Windows API instead of syscalls
- PE/COFF binary format
- Microsoft x64 calling convention
- SEH exception handling

**Implementation Notes**:
- No direct syscalls (use ntdll.dll or kernel32.dll)
- Different path separators
- UTF-16 for system calls

---

## 3. WebAssembly Targets

### 3.1 WASM32 Overview

WebAssembly enables Coral to run in:
- Web browsers
- WASI-compatible runtimes (Wasmtime, Wasmer, WasmEdge)
- Edge computing platforms (Cloudflare Workers, Fastly Compute)

**Triple**: `wasm32-unknown-unknown` or `wasm32-wasi`

### 3.2 Browser Target (wasm32-unknown-unknown)

**Characteristics**:
- No file system access
- No networking (except via JavaScript)
- Memory is a linear array
- Garbage collection via WASM GC proposal (future)

**Build Command**:
```bash
coralc program.coral --target wasm32-unknown-unknown -o program.wasm
```

**JavaScript Interop**:
```javascript
// Loading Coral WASM module
const response = await fetch('program.wasm');
const bytes = await response.arrayBuffer();
const module = await WebAssembly.instantiate(bytes, {
    coral_env: {
        log: (ptr, len) => console.log(decodeString(ptr, len)),
        // ... other imports
    }
});

// Call Coral function
module.instance.exports.main();
```

### 3.3 WASI Target (wasm32-wasi)

**Characteristics**:
- Standardized system interface
- File system access (sandboxed)
- Environment variables
- Command-line arguments
- Clock/random access

**Build Command**:
```bash
coralc program.coral --target wasm32-wasi -o program.wasm
```

**Running**:
```bash
# Using Wasmtime
wasmtime program.wasm

# Using Wasmer
wasmer run program.wasm

# With file system access
wasmtime --dir=. program.wasm
```

### 3.4 WASM Limitations

| Feature | Support | Notes |
|---------|---------|-------|
| Integers | ✅ Full | i32, i64 native |
| Floats | ✅ Full | f32, f64 native |
| Memory | ⚠️ Limited | Linear memory, no mmap |
| Threads | ⚠️ Proposal | SharedArrayBuffer required |
| GC | 🔜 Coming | WASM GC proposal |
| Exceptions | 🔜 Coming | WASM exceptions proposal |
| SIMD | ✅ Available | 128-bit SIMD |
| Tail Calls | 🔜 Coming | Tail call proposal |

### 3.5 WASM-Specific Runtime

```coral
# std/platform/wasm.coral

# Memory operations use WASM instructions
*memory_grow(pages)
    __wasm_memory_grow(0, pages)

*memory_size()
    __wasm_memory_size(0) * 65536  # pages to bytes

# No direct syscalls - use WASI or JS imports
*wasi_fd_write(fd, iovs, iovs_len, nwritten)
    __wasi_fd_write(fd, iovs, iovs_len, nwritten)
```

---

## 4. LLVM Backend Options

### 4.1 Optimization Levels

```bash
# No optimization (fast compile)
coralc program.coral -O0 -o program

# Basic optimization
coralc program.coral -O1 -o program

# Standard optimization (default)
coralc program.coral -O2 -o program

# Aggressive optimization
coralc program.coral -O3 -o program

# Size optimization
coralc program.coral -Os -o program

# Extreme size optimization
coralc program.coral -Oz -o program
```

### 4.2 Link-Time Optimization (LTO)

```bash
# Thin LTO (faster)
coralc program.coral --lto=thin -o program

# Full LTO (smaller, slower compile)
coralc program.coral --lto=full -o program
```

### 4.3 Debug Information

```bash
# Full debug info
coralc program.coral -g -o program

# Line tables only (smaller)
coralc program.coral -gline-tables-only -o program
```

---

## 5. Cross-Compilation

### 5.1 From Linux to Other Targets

```bash
# Linux to macOS (requires macOS SDK)
coralc program.coral --target x86_64-apple-darwin \
    --sysroot=/path/to/macos-sdk -o program

# Linux to Windows (requires Windows SDK or MinGW)
coralc program.coral --target x86_64-pc-windows-gnu -o program.exe

# Linux to WASM
coralc program.coral --target wasm32-wasi -o program.wasm
```

### 5.2 Required Tools

| Host | Target | Requirements |
|------|--------|--------------|
| Linux | macOS | osxcross, macOS SDK |
| Linux | Windows | mingw-w64 or MSVC SDK |
| Linux | WASM | wasm-ld (LLVM) |
| macOS | Linux | linux cross toolchain |
| Any | Any | LLVM with target enabled |

---

## 6. Binary Format Details

### 6.1 ELF (Linux)

```
ELF Header
├── Program Headers (segments)
│   ├── PT_LOAD (code)
│   ├── PT_LOAD (data)
│   └── PT_DYNAMIC (if dynamic)
├── Sections
│   ├── .text (code)
│   ├── .rodata (constants, strings)
│   ├── .data (initialized globals)
│   ├── .bss (uninitialized globals)
│   └── .coral_meta (Coral-specific metadata)
└── Symbol Table
```

### 6.2 WebAssembly

```
WASM Module
├── Type Section (function signatures)
├── Import Section (external functions)
├── Function Section (function indices)
├── Memory Section (linear memory)
├── Global Section (global variables)
├── Export Section (public API)
├── Code Section (function bodies)
├── Data Section (initialized data)
└── Custom Sections
    └── coral_debug (source maps, etc.)
```

---

## 7. Target-Specific Features

### 7.1 SIMD Support

```coral
# Using SIMD when available
*vector_add(a, b)
    # Compiler auto-vectorizes when beneficial
    result is []
    for i in 0..a.length
        result.push(a[i] + b[i])
    result

# Explicit SIMD (future)
*simd_dot_product(a, b)
    __simd_f32x4_dot(a.as_simd(), b.as_simd())
```

### 7.2 Platform Detection

```coral
# Runtime platform detection
*get_platform()
    __coral_platform()  # Returns 'linux', 'macos', 'windows', 'wasm'

*get_arch()
    __coral_arch()  # Returns 'x86_64', 'aarch64', 'wasm32'

# Compile-time feature detection (future)
@[cfg(target = 'wasm')]
*special_wasm_function()
    # Only compiled for WASM
```

---

## 8. Implementation Status

### 8.1 Current State

| Component | x86_64 Linux | aarch64 Linux | macOS | WASM |
|-----------|--------------|---------------|-------|------|
| Basic codegen | ✅ | ✅ | ✅ | ⚠️ |
| Runtime | ✅ | ✅ | ⚠️ | ❌ |
| Actors | ✅ | ✅ | ⚠️ | ❌ |
| File I/O | ✅ | ✅ | ⚠️ | ❌ |
| Networking | ⚠️ | ⚠️ | ❌ | ❌ |

### 8.2 WASM Roadmap

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Basic WASM codegen | 🔜 Planned |
| 2 | WASI file I/O | 🔜 Planned |
| 3 | JS interop layer | 🔜 Planned |
| 4 | WASM threads | 🔮 Future |
| 5 | WASM GC integration | 🔮 Future |

---

## 9. Task Breakdown for WASM Support

### Phase 1: Basic WASM Codegen (Week 1-2)

| Task | Description | Effort |
|------|-------------|--------|
| W1.1 | Add wasm32 target to compiler | 2h |
| W1.2 | Adjust calling conventions for WASM | 4h |
| W1.3 | Implement linear memory allocator | 4h |
| W1.4 | Handle WASM-specific integer types | 2h |
| W1.5 | Generate valid WASM binary | 4h |
| W1.6 | Test hello world in browser | 2h |

### Phase 2: WASI Integration (Week 2-3)

| Task | Description | Effort |
|------|-------------|--------|
| W2.1 | Import WASI functions | 2h |
| W2.2 | Implement file operations | 4h |
| W2.3 | Implement console I/O | 2h |
| W2.4 | Handle command-line args | 2h |
| W2.5 | Environment variables | 2h |
| W2.6 | Test in Wasmtime/Wasmer | 2h |

### Phase 3: JavaScript Interop (Week 3-4)

| Task | Description | Effort |
|------|-------------|--------|
| W3.1 | Design JS binding format | 4h |
| W3.2 | String passing (UTF-8/UTF-16) | 4h |
| W3.3 | Callback support | 4h |
| W3.4 | TypeScript type generation | 4h |
| W3.5 | Example web application | 4h |

---

## 10. Other LLVM-Enabled Targets (Future)

### 10.1 GPU Targets (NVPTX, AMDGPU)

LLVM can target NVIDIA and AMD GPUs. Future Coral could have:

```coral
# GPU kernel (future syntax)
@[kernel]
*vector_add_gpu(a, b, result)
    i is __thread_idx_x()
    result[i] is a[i] + b[i]
```

### 10.2 Embedded Targets

- ARM Cortex-M (bare metal)
- RISC-V embedded
- ESP32 (Xtensa)

### 10.3 Exotic Targets

- WebGPU shaders (WGSL)
- SPIR-V (Vulkan shaders)
- BPF (Linux eBPF programs)

---

## Appendix: Compiler Flags Reference

```bash
# Target selection
--target <triple>           # Set target triple
--cpu <name>               # Set target CPU
--features <+feat,-feat>   # Enable/disable CPU features

# Optimization
-O0, -O1, -O2, -O3, -Os, -Oz
--lto=thin|full            # Link-time optimization

# Output control
-o <file>                  # Output file
--emit=asm|llvm|obj|exe    # Output type
-c                         # Compile only, no link

# Debug
-g                         # Debug info
--sanitize=address         # AddressSanitizer
--sanitize=thread          # ThreadSanitizer

# Linking
--static                   # Static linking
--shared                   # Create shared library
-l<lib>                    # Link library
-L<path>                   # Library search path

# Platform-specific
--sysroot <path>          # System root for cross-compile
--wasm-opt                # Run wasm-opt on output
```
