# Coral libc Independence & System Calls Specification

_Created: 2026-01-06_

## Executive Summary

This document outlines how Coral can achieve full independence from libc by implementing system calls directly, using assembly shims where necessary. This enables:
- Smaller binaries (no libc linkage)
- Full control over memory allocation
- Embedded/WASM portability
- Understanding of low-level operations for educational purposes

---

## 1. Architecture Overview

### 1.1 Layers

```
┌─────────────────────────────────────────────────────────────┐
│                    User Coral Code                          │
│              (std.io, std.net, std.collections)             │
├─────────────────────────────────────────────────────────────┤
│                 Coral Standard Library                      │
│                 (Pure Coral Implementations)                │
├─────────────────────────────────────────────────────────────┤
│                   Runtime Primitives                        │
│              (coral_*, memory management)                   │
├─────────────────────────────────────────────────────────────┤
│                  System Call Layer                          │
│              (sys_read, sys_write, sys_mmap)                │
├─────────────────────────────────────────────────────────────┤
│                   Assembly Shims                            │
│                 (Architecture Specific)                     │
├─────────────────────────────────────────────────────────────┤
│                      Linux Kernel                           │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 Design Goals

1. **Minimal Dependencies**: No libc, no dynamic linking
2. **Portability**: Abstract syscall interface for multiple platforms
3. **Safety**: Wrap unsafe operations in safe Coral interfaces
4. **Performance**: Direct syscalls avoid libc overhead

---

## 2. System Call Interface

### 2.1 Linux x86_64 Syscall ABI

```
Syscall Number: RAX
Arguments: RDI, RSI, RDX, R10, R8, R9
Return Value: RAX
Clobbered: RCX, R11
```

### 2.2 Core Syscalls Needed

| Syscall | Number | Purpose |
|---------|--------|---------|
| `read` | 0 | Read from file descriptor |
| `write` | 1 | Write to file descriptor |
| `open` | 2 | Open file |
| `close` | 3 | Close file descriptor |
| `mmap` | 9 | Map memory |
| `munmap` | 11 | Unmap memory |
| `brk` | 12 | Set program break (heap) |
| `exit` | 60 | Exit process |
| `socket` | 41 | Create socket |
| `connect` | 42 | Connect socket |
| `accept` | 43 | Accept connection |
| `bind` | 49 | Bind socket |
| `listen` | 50 | Listen on socket |
| `sendto` | 44 | Send data |
| `recvfrom` | 45 | Receive data |

### 2.3 Assembly Shim Template (x86_64 Linux)

```asm
# syscall_linux_x86_64.s

.global coral_syscall0
.global coral_syscall1
.global coral_syscall2
.global coral_syscall3
.global coral_syscall4
.global coral_syscall5
.global coral_syscall6

# No arguments: rdi = syscall number
coral_syscall0:
    mov %rdi, %rax
    syscall
    ret

# 1 argument: rdi = syscall number, rsi = arg1
coral_syscall1:
    mov %rdi, %rax
    mov %rsi, %rdi
    syscall
    ret

# 2 arguments
coral_syscall2:
    mov %rdi, %rax
    mov %rsi, %rdi
    mov %rdx, %rsi
    syscall
    ret

# 3 arguments
coral_syscall3:
    mov %rdi, %rax
    mov %rsi, %rdi
    mov %rdx, %rsi
    mov %rcx, %rdx
    syscall
    ret

# 4 arguments
coral_syscall4:
    mov %rdi, %rax
    mov %rsi, %rdi
    mov %rdx, %rsi
    mov %rcx, %rdx
    mov %r8, %r10
    syscall
    ret

# 5 arguments
coral_syscall5:
    mov %rdi, %rax
    mov %rsi, %rdi
    mov %rdx, %rsi
    mov %rcx, %rdx
    mov %r8, %r10
    mov %r9, %r8
    syscall
    ret

# 6 arguments
coral_syscall6:
    mov %rdi, %rax
    mov %rsi, %rdi
    mov %rdx, %rsi
    mov %rcx, %rdx
    mov %r8, %r10
    mov %r9, %r8
    mov 8(%rsp), %r9
    syscall
    ret
```

---

## 3. Memory Allocation Without libc

### 3.1 The Story: Runtime Memory Management

When a Coral program starts, it has no heap. The runtime must bootstrap memory allocation:

```
1. Program entry point (_start)
   ↓
2. Initialize data segments (static data)
   ↓
3. Create initial heap via brk() or mmap()
   ↓
4. Initialize allocator data structures
   ↓
5. Runtime can now allocate CoralValues
   ↓
6. Call user's main() function
```

### 3.2 Allocator Implementation

```coral
# std/runtime/allocator.coral
# This would be implemented in Coral + inline assembly

# Global allocator state (in static memory)
heap_start is 0
heap_current is 0
heap_end is 0

# Initialize heap via mmap
*init_allocator()
    # Request 1MB initial heap
    size is 1048576
    
    # mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)
    ptr is sys_mmap(0, size, 3, 34, -1, 0)
    
    ptr < 0 ? ! err AllocationFailed
    
    heap_start is ptr
    heap_current is ptr
    heap_end is ptr + size
    true

# Simple bump allocator
*allocate(size)
    aligned_size is (size + 7) & ~7  # 8-byte align
    
    heap_current + aligned_size > heap_end ?
        grow_heap(aligned_size) ! return err OutOfMemory
    
    ptr is heap_current
    heap_current is heap_current + aligned_size
    ptr

# Grow heap when needed
*grow_heap(min_size)
    grow_amount is max(min_size * 2, 1048576)
    
    new_region is sys_mmap(0, grow_amount, 3, 34, -1, 0)
    new_region < 0 ? ! err OutOfMemory
    
    # For simplicity, track as separate region
    # Full implementation would use free lists
    heap_start is new_region
    heap_current is new_region
    heap_end is new_region + grow_amount
    true
```

### 3.3 Runtime Value Allocation

```rust
// runtime/src/alloc.rs - Rust side using our syscall shims

extern "C" {
    fn coral_syscall6(nr: i64, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64;
}

const SYS_MMAP: i64 = 9;
const PROT_READ_WRITE: i64 = 0x3;
const MAP_PRIVATE_ANONYMOUS: i64 = 0x22;

pub unsafe fn sys_mmap(size: usize) -> *mut u8 {
    coral_syscall6(
        SYS_MMAP,
        0,                        // addr (NULL = kernel chooses)
        size as i64,
        PROT_READ_WRITE,
        MAP_PRIVATE_ANONYMOUS,
        -1,                       // fd
        0                         // offset
    ) as *mut u8
}

pub struct CoralAllocator {
    current: *mut u8,
    end: *mut u8,
    regions: Vec<(*mut u8, usize)>,
}

impl CoralAllocator {
    pub fn new() -> Self {
        let size = 1024 * 1024; // 1MB initial
        let ptr = unsafe { sys_mmap(size) };
        CoralAllocator {
            current: ptr,
            end: unsafe { ptr.add(size) },
            regions: vec![(ptr, size)],
        }
    }
    
    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        let aligned = (size + 7) & !7;
        if self.current.wrapping_add(aligned) > self.end {
            self.grow(aligned);
        }
        let ptr = self.current;
        self.current = self.current.wrapping_add(aligned);
        ptr
    }
}
```

---

## 4. File I/O Without libc

### 4.1 Low-Level File Operations

```coral
# std/sys/file.coral

SYS_READ is 0
SYS_WRITE is 1
SYS_OPEN is 2
SYS_CLOSE is 3

O_RDONLY is 0
O_WRONLY is 1
O_RDWR is 2
O_CREAT is 64
O_TRUNC is 512

*sys_open(path, flags, mode)
    syscall3(SYS_OPEN, path.as_ptr(), flags, mode)

*sys_read(fd, buffer, count)
    syscall3(SYS_READ, fd, buffer.as_ptr(), count)

*sys_write(fd, buffer, count)
    syscall3(SYS_WRITE, fd, buffer.as_ptr(), count)

*sys_close(fd)
    syscall1(SYS_CLOSE, fd)
```

### 4.2 High-Level File API

```coral
# std/io/file.coral
use std.sys.file

store File
    fd
    path
    mode
    
    *open(path, mode)
        flags is match mode
            'r' ? O_RDONLY
            'w' ? O_WRONLY | O_CREAT | O_TRUNC
            'rw' ? O_RDWR | O_CREAT
                ! err InvalidMode
        
        fd is sys_open(path, flags, 0o644)
        fd < 0 ? ! err IO:OpenFailed
        
        File(fd is fd, path is path, mode is mode)
    
    *read(size)
        buffer is bytes.allocate(size)
        count is sys_read(fd, buffer, size)
        count < 0 ? ! err IO:ReadFailed
        buffer.slice(0, count)
    
    *read_all()
        chunks is []
        loop
            chunk is read(4096)
            chunk.is_err ? ! return err
            chunk.length == 0 ? break
            chunks.push(chunk)
        bytes.concat(chunks)
    
    *write(data)
        written is sys_write(fd, data, data.length)
        written < 0 ? ! err IO:WriteFailed
        written
    
    *close()
        result is sys_close(fd)
        result < 0 ? ! err IO:CloseFailed
        true

# Convenience functions
*read_file(path)
    f is File.open(path, 'r') ! return err
    content is f.read_all()
    f.close()
    content

*write_file(path, data)
    f is File.open(path, 'w') ! return err
    f.write(data)
    f.close()
    true
```

---

## 5. Networking Without libc

### 5.1 Socket System Calls

```coral
# std/sys/socket.coral

SYS_SOCKET is 41
SYS_CONNECT is 42
SYS_ACCEPT is 43
SYS_SENDTO is 44
SYS_RECVFROM is 45
SYS_BIND is 49
SYS_LISTEN is 50

AF_INET is 2
SOCK_STREAM is 1
SOCK_DGRAM is 2

*sys_socket(domain, type, protocol)
    syscall3(SYS_SOCKET, domain, type, protocol)

*sys_bind(fd, addr, addrlen)
    syscall3(SYS_BIND, fd, addr, addrlen)

*sys_listen(fd, backlog)
    syscall2(SYS_LISTEN, fd, backlog)

*sys_accept(fd, addr, addrlen)
    syscall3(SYS_ACCEPT, fd, addr, addrlen)

*sys_connect(fd, addr, addrlen)
    syscall3(SYS_CONNECT, fd, addr, addrlen)

*sys_send(fd, buffer, len, flags)
    syscall4(SYS_SENDTO, fd, buffer, len, flags)

*sys_recv(fd, buffer, len, flags)
    syscall4(SYS_RECVFROM, fd, buffer, len, flags)
```

### 5.2 High-Level TCP Client

```coral
# std/net/tcp.coral
use std.sys.socket

store TcpConnection
    fd
    remote_addr
    
    *connect(host, port)
        fd is sys_socket(AF_INET, SOCK_STREAM, 0)
        fd < 0 ? ! err Net:SocketFailed
        
        addr is sockaddr_in(AF_INET, port, resolve_host(host))
        result is sys_connect(fd, addr, 16)
        result < 0 ? 
            sys_close(fd)
            ! err Net:ConnectFailed
        
        TcpConnection(fd is fd, remote_addr is '{host}:{port}')
    
    *send(data)
        sent is sys_send(fd, data, data.length, 0)
        sent < 0 ? ! err Net:SendFailed
        sent
    
    *recv(max_size)
        buffer is bytes.allocate(max_size)
        received is sys_recv(fd, buffer, max_size, 0)
        received < 0 ? ! err Net:RecvFailed
        received == 0 ? ! err Net:ConnectionClosed
        buffer.slice(0, received)
    
    *close()
        sys_close(fd)

# Example usage:
*fetch_http(host, path)
    conn is TcpConnection.connect(host, 80) ! return err
    
    request is 'GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n'
    conn.send(request.as_bytes()) ! return err
    
    response is bytes.new()
    loop
        chunk is conn.recv(4096)
        chunk.is_err and chunk.err == Net:ConnectionClosed ? break
        chunk.is_err ? ! return chunk
        response.append(chunk)
    
    conn.close()
    response.as_string()
```

### 5.3 High-Level TCP Server

```coral
# std/net/server.coral
use std.sys.socket

store TcpServer
    fd
    addr
    
    *bind(host, port)
        fd is sys_socket(AF_INET, SOCK_STREAM, 0)
        fd < 0 ? ! err Net:SocketFailed
        
        addr is sockaddr_in(AF_INET, port, resolve_host(host))
        result is sys_bind(fd, addr, 16)
        result < 0 ?
            sys_close(fd)
            ! err Net:BindFailed
        
        TcpServer(fd is fd, addr is '{host}:{port}')
    
    *listen(backlog)
        result is sys_listen(fd, backlog)
        result < 0 ? ! err Net:ListenFailed
        true
    
    *accept()
        client_addr is bytes.allocate(16)
        client_fd is sys_accept(fd, client_addr, 16)
        client_fd < 0 ? ! err Net:AcceptFailed
        TcpConnection(fd is client_fd, remote_addr is parse_sockaddr(client_addr))
    
    *close()
        sys_close(fd)

# Example echo server:
*run_echo_server(port)
    server is TcpServer.bind('0.0.0.0', port) ! return err
    server.listen(10) ! return err
    
    log('Echo server listening on port {port}')
    
    loop
        client is server.accept() ! continue  # Skip failed accepts
        
        # Handle client (in real code, spawn actor)
        loop
            data is client.recv(1024)
            data.is_err ? break
            client.send(data) ! break
        
        client.close()
```

---

## 6. Program Entry Point

### 6.1 Custom _start (No libc)

```asm
# start_linux_x86_64.s

.global _start
.extern coral_runtime_init
.extern __user_main
.extern coral_runtime_shutdown

_start:
    # Clear frame pointer for clean backtraces
    xor %rbp, %rbp
    
    # Get argc, argv from stack
    pop %rdi              # argc
    mov %rsp, %rsi        # argv
    
    # Align stack to 16 bytes
    and $-16, %rsp
    
    # Initialize Coral runtime (allocator, etc.)
    call coral_runtime_init
    
    # Call user's main function
    call __user_main
    
    # Shutdown runtime
    call coral_runtime_shutdown
    
    # Exit with return code
    mov %rax, %rdi
    mov $60, %rax         # SYS_exit
    syscall
```

### 6.2 Runtime Initialization

```rust
// runtime/src/init.rs

#[no_mangle]
pub extern "C" fn coral_runtime_init() {
    // Initialize allocator
    ALLOCATOR.lock().init();
    
    // Initialize actor runtime
    ACTOR_RUNTIME.lock().init();
    
    // Initialize string interning
    STRING_INTERNER.lock().init();
}

#[no_mangle]
pub extern "C" fn coral_runtime_shutdown() {
    // Cleanup actors
    ACTOR_RUNTIME.lock().shutdown();
    
    // Report any leaked memory (debug mode)
    #[cfg(debug_assertions)]
    ALLOCATOR.lock().report_leaks();
}
```

---

## 7. Task Breakdown

### 7.1 Phase 1: Assembly Shims (Week 1)

| Task | Description | Effort |
|------|-------------|--------|
| 1.1 | Create `syscall_linux_x86_64.s` with syscall wrappers | 2h |
| 1.2 | Create `start_linux_x86_64.s` entry point | 2h |
| 1.3 | Update build system to link assembly | 2h |
| 1.4 | Add ARM64 Linux syscall shims | 4h |
| 1.5 | Test minimal "hello world" without libc | 2h |

### 7.2 Phase 2: Memory Allocator (Week 2)

| Task | Description | Effort |
|------|-------------|--------|
| 2.1 | Implement mmap-based allocator in Rust | 4h |
| 2.2 | Add free list for memory reuse | 4h |
| 2.3 | Integrate with CoralValue allocation | 4h |
| 2.4 | Add memory tracking/debugging | 2h |
| 2.5 | Benchmark vs libc malloc | 2h |

### 7.3 Phase 3: File I/O (Week 3)

| Task | Description | Effort |
|------|-------------|--------|
| 3.1 | Low-level `std/sys/file.coral` | 2h |
| 3.2 | High-level `std/io/file.coral` | 4h |
| 3.3 | Directory operations | 4h |
| 3.4 | Path manipulation | 2h |
| 3.5 | File descriptor table management | 2h |

### 7.4 Phase 4: Networking (Week 4)

| Task | Description | Effort |
|------|-------------|--------|
| 4.1 | Low-level `std/sys/socket.coral` | 4h |
| 4.2 | TCP client implementation | 4h |
| 4.3 | TCP server implementation | 4h |
| 4.4 | UDP support | 2h |
| 4.5 | DNS resolution (simple) | 4h |

### 7.5 Phase 5: Platform Abstraction (Week 5)

| Task | Description | Effort |
|------|-------------|--------|
| 5.1 | Create platform abstraction layer | 4h |
| 5.2 | macOS syscall shims | 4h |
| 5.3 | Windows API shims (future) | 8h |
| 5.4 | WASI shims for WebAssembly | 4h |

---

## 8. Benefits and Tradeoffs

### 8.1 Benefits

1. **Smaller Binaries**: No libc = 100KB+ smaller
2. **Full Control**: Understand every byte of allocation
3. **Portability**: Same Coral code, different syscall backends
4. **Security**: No libc vulnerabilities
5. **Education**: Learn systems programming through Coral

### 8.2 Tradeoffs

1. **More Work**: Must implement everything libc provides
2. **Platform Specific**: Need shims per OS/architecture
3. **Less Tested**: libc is battle-tested over decades
4. **Missing Features**: No localization, timezone, etc. initially

### 8.3 Hybrid Approach

For alpha, use libc through Rust's std. Gradually replace with direct syscalls:

```
Alpha:    Rust std (uses libc)
Beta:     Optional no-libc mode
1.0:      Both modes fully supported
```

---

## 9. Example: Complete Story

### 9.1 A Simple HTTP Server in Pure Coral

```coral
# examples/http_server.coral
use std.net.server
use std.net.http

*handle_request(request)
    match request.path
        '/' ? http.response(200, '<h1>Hello from Coral!</h1>')
        '/api/time' ? http.response(200, '{time.now()}')
            ! http.response(404, 'Not Found')

*main()
    server is HttpServer.bind('0.0.0.0', 8080) ! 
        log('Failed to bind: {err}')
        exit(1)
    
    log('Server running at http://localhost:8080')
    
    loop
        request is server.accept_request() ! continue
        response is handle_request(request)
        server.send_response(request.connection, response)
```

This server:
- Uses direct syscalls for socket operations
- Allocates memory via mmap (no malloc)
- Runs without libc linked
- Produces a ~200KB static binary

---

## Appendix A: Syscall Reference

### Linux x86_64 Common Syscalls

| Name | Number | Args | Description |
|------|--------|------|-------------|
| read | 0 | fd, buf, count | Read bytes |
| write | 1 | fd, buf, count | Write bytes |
| open | 2 | path, flags, mode | Open file |
| close | 3 | fd | Close fd |
| stat | 4 | path, statbuf | Get file info |
| fstat | 5 | fd, statbuf | Get fd info |
| poll | 7 | fds, nfds, timeout | Poll fds |
| mmap | 9 | addr, len, prot, flags, fd, off | Map memory |
| munmap | 11 | addr, len | Unmap memory |
| brk | 12 | addr | Set heap break |
| socket | 41 | domain, type, proto | Create socket |
| connect | 42 | fd, addr, len | Connect |
| accept | 43 | fd, addr, len | Accept |
| sendto | 44 | fd, buf, len, flags, addr, len | Send |
| recvfrom | 45 | fd, buf, len, flags, addr, len | Receive |
| bind | 49 | fd, addr, len | Bind socket |
| listen | 50 | fd, backlog | Listen |
| clone | 56 | flags, stack, ptid, ctid, tls | Create thread |
| exit | 60 | code | Exit process |
| getpid | 39 | | Get process ID |
| getuid | 102 | | Get user ID |
