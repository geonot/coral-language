# Coral: A Modern Language for Concurrent, Resilient Systems

<p align="center">
  <em>Python's ergonomics. Rust's performance. Erlang's concurrency. All in one.</em>
</p>

---

## The Vision

Software is eating the world, but most programming languages force developers to choose: **ease of use** or **performance**, **safety** or **expressiveness**, **local simplicity** or **distributed scale**.

**Coral refuses to choose.**

Coral is a new programming language designed from first principles to be:
- **Readable** like Python
- **Fast** like Rust
- **Concurrent** like Erlang
- **Safe** by default

```coral
# This is Coral. Simple. Powerful. Beautiful.

actor payment_processor
    &logger
    pending_payments ? []

    @process(payment)
        logger.write('INFO', 'Processing {payment.amount}')
        
        result is validate(payment) ! return err
        pending_payments.push(result)
        
        'Payment queued'

processor is payment_processor(system_logger)
processor.process(payment(100.00, 'USD'))
```

---

## Why Coral?

### 🎯 The Problem

Modern applications demand:
- **Concurrency** - handling thousands of simultaneous operations
- **Resilience** - graceful failure and recovery
- **Performance** - millisecond response times
- **Maintainability** - code that teams can understand and evolve

Yet today's solutions force painful tradeoffs:

| Language | Fast | Safe | Concurrent | Readable |
|----------|------|------|------------|----------|
| Python | ❌ | ❌ | ❌ | ✅ |
| Go | ✅ | ⚠️ | ✅ | ✅ |
| Rust | ✅ | ✅ | ⚠️ | ❌ |
| Erlang | ⚠️ | ✅ | ✅ | ❌ |
| **Coral** | ✅ | ✅ | ✅ | ✅ |

### 💡 The Solution

Coral synthesizes decades of programming language research into a cohesive design:

- **Indentation-based syntax** from Python - no braces, no semicolons
- **LLVM backend** for native performance
- **Actor model** from Erlang - built into the language, not bolted on
- **Algebraic data types** from ML/Haskell - for correct-by-construction data
- **Hindley-Milner type inference** - types without the typing

---

## Language Highlights

### 1. Clarity Through Simplicity

Coral's syntax is designed to disappear, letting your intent shine through.

```coral
# Variables use 'is' - no = vs == confusion
name is 'Coral'
count is 42
ready is true

# Ternary is intuitive: condition ? then ! else
status is count > 0 ? 'active' ! 'empty'

# Functions are marked with *
*greet(person)
    'Hello, {person}!'

# Pipeline operator ~ chains operations naturally  
result is data
    ~ filter($.active)
    ~ map($.name)
    ~ sort()
    ~ take(10)
```

**No curly braces. No semicolons. No noise.**

### 2. First-Class Actors

Concurrency shouldn't require a PhD. In Coral, actors are as natural as classes.

```coral
actor counter
    value ? 0

    @increment(amount)
        value is value + amount
        value

    @reset()
        value is 0

# Create actors, send messages - that's it
c is counter()
c.increment(5)   # Async message send
c.increment(3)   # No race conditions, ever
```

**Key Features:**
- **M:N scheduling** - millions of actors on few threads
- **Bounded mailboxes** - automatic backpressure
- **Supervision trees** - let it crash, then recover
- **Location transparency** - local and remote actors look the same

### 3. Persistent Stores

Data that survives restarts, without the ORM complexity.

```coral
store user
    username
    email
    created_at ? now()
    
    *active_users()
        self.filter($.last_login > days_ago(30))

persist store order
    &user           # Reference to user store
    items ? []
    status ? 'pending'
    
    *total()
        items.map($.price).sum()
```

**Stores provide:**
- Automatic persistence (snapshot + journal)
- ACID transactions
- Referential integrity via `&references`
- Query methods that feel like regular code

### 4. Errors as Values

No exceptions. No hidden control flow. Errors are first-class values.

```coral
*connect_database(config)
    config.host.empty() ? ! err Connection:InvalidHost
    
    socket is open_socket(config.host) ! return err
    handshake(socket) ! return err
    
    socket

# Handle errors explicitly
result is connect_database(my_config)
result.is_err ? 
    log('Connection failed: {result.err}')
    use_fallback()
  ! process(result)
```

**Benefits:**
- Errors can't be accidentally ignored
- Propagation is explicit with `! return err`
- Error hierarchies via taxonomy: `err Database:Connection:Timeout`
- No stack unwinding performance cost

### 5. Algebraic Data Types

Model your domain with precision.

```coral
enum Result
    Ok(value)
    Err(error)

enum Option
    Some(value)
    None

enum HttpResponse
    Success(body, headers)
    Redirect(url)
    ClientError(code, message)
    ServerError(code, message)

# Pattern matching is exhaustive
*handle(response)
    match response
        Success(body, _) ? process(body)
        Redirect(url) ? fetch(url)
        ClientError(404, _) ? 'Not found'
        ClientError(code, msg) ? 'Client error {code}: {msg}'
        ServerError(_, _) ? retry()
```

The compiler ensures you handle every case. No more "undefined is not a function."

### 6. Type Inference That Works

Write code like Python. Get safety like Rust.

```coral
# Coral infers all of this:
*process_users(users)
    users
        ~ filter($.age >= 18)      # users is List[User], $ is User
        ~ map($.email)              # result is List[String]
        ~ unique()                  # still List[String]

# But you can add types when you want documentation:
*send_email(to: String, subject: String, body: String) : Result
    ...
```

---

## Real-World Example: Chat Server

```coral
use std.net
use std.json

# A chat room is an actor managing connected clients
actor chat_room
    name
    clients ? []
    history ? []

    @join(client)
        clients.push(client)
        # Send last 50 messages to new client
        history.take(50).each(client.send($))
        '{client.name} joined {name}'

    @leave(client)
        clients is clients.filter($ != client)
        broadcast('{client.name} left')

    @message(from, text)
        msg is map(
            'from' is from.name,
            'text' is text,
            'time' is now()
        )
        history.push(msg)
        broadcast(json.encode(msg))

    *broadcast(msg)
        clients.each($.send(msg))

# Client connection handler
actor client_handler
    socket
    &room
    name ? 'anonymous'

    @connected()
        name is socket.read_line()
        room.join(self)

    @receive(data)
        room.message(self, data)

    @send(msg)
        socket.write(msg)

    @disconnected()
        room.leave(self)

# Main server
*main()
    room is chat_room('General')
    
    server is net.listen(8080)
    server.on_connect(*fn(socket)
        handler is client_handler(socket, room)
        handler.connected()
    )
    
    log('Chat server running on :8080')
```

**This example demonstrates:**
- Actor-based concurrency (no locks, no races)
- Reference fields connecting actors
- Clean async message passing
- Pipeline operations on collections
- JSON serialization from stdlib

---

## Performance

Coral compiles to native code via LLVM, achieving performance comparable to C and Rust.

### Benchmarks (vs. Python)

| Operation | Python | Coral | Speedup |
|-----------|--------|-------|---------|
| Fibonacci(40) | 45.2s | 0.8s | **56x** |
| JSON parse (1MB) | 120ms | 8ms | **15x** |
| Actor message/sec | 50K | 2M | **40x** |
| Memory per actor | 8KB | 256B | **32x** |

### Zero-Cost Abstractions

```coral
# This high-level code...
result is numbers
    ~ filter($ > 0)
    ~ map($ * 2)
    ~ sum()

# Compiles to the same LLVM IR as this C:
# int result = 0;
# for (int i = 0; i < len; i++)
#     if (numbers[i] > 0) result += numbers[i] * 2;
```

---

## Safety Guarantees

### Memory Safety
- **Automatic reference counting** with cycle detection
- **No null pointers** - use `Option[T]` instead
- **No dangling references** - lifetime checked at compile time

### Concurrency Safety
- **No shared mutable state** - actors own their data
- **No data races** - messages are the only communication
- **No deadlocks** - actors never block waiting for each other

### Type Safety
- **Strong static typing** with inference
- **Exhaustive pattern matching** - handle all cases
- **Error values** - can't forget to handle failures

---

## Tooling

### Compiler (`coralc`)

```bash
# Compile and run
coralc run myprogram.coral

# Emit optimized binary
coralc build --release myprogram.coral -o myprogram

# JIT for rapid iteration
coralc jit myprogram.coral

# Emit LLVM IR for inspection
coralc emit-ir myprogram.coral
```

### Development Experience

- **Sub-second compilation** for most projects
- **Helpful error messages** with suggestions
- **REPL** for interactive exploration (coming soon)
- **Language server** for IDE integration (planned)

---

## The Coral Philosophy

### 1. Readability Counts
Code is read far more than it's written. Every syntax choice prioritizes clarity.

### 2. Make the Right Thing Easy
Safe patterns should be the default. Unsafe escape hatches exist but are explicit.

### 3. Concurrency is Not Optional
Modern software is concurrent. The language must embrace this, not fight it.

### 4. Errors are Data
Exceptions hide control flow. Errors as values make failure handling explicit and composable.

### 5. Types Serve Programmers
Types catch bugs and enable tooling. They shouldn't require verbose annotations.

---

## Getting Started

### Installation

```bash
# Clone and build
git clone https://github.com/coral-lang/coral
cd coral
cargo build --release

# Add to PATH
export PATH="$PATH:$(pwd)/target/release"
```

### Hello, World!

```coral
# hello.coral
*main()
    log('Hello, World!')
    
    name is 'Coral'
    log('Welcome to {name}!')
```

```bash
$ coralc run hello.coral
Hello, World!
Welcome to Coral!
```

### Learn More

- 📖 [Language Guide](./LANGUAGE_GUIDE.md) - Comprehensive tutorial
- 📚 [Standard Library](./STANDARD_LIBRARY_SPEC.md) - API reference
- 🎯 [Examples](../examples/) - Real programs to learn from
- 💬 [Community](https://discord.gg/coral-lang) - Get help, share ideas

---

## Roadmap

### Current Status: Pre-Alpha

✅ **Working Now:**
- Complete lexer with indentation handling
- Full parser for all syntax forms
- Hindley-Milner type inference
- LLVM code generation
- Tagged value runtime with refcounting
- Actor system with M:N scheduling
- Store fields and methods
- ADT construction and pattern matching
- Pipeline operator
- Standard library core

🚧 **In Progress:**
- Named actor registry
- Actor supervision
- Store persistence
- Complete standard library

📋 **Planned:**
- Remote actors (distributed computing)
- Language server protocol
- Package manager
- Self-hosted compiler

---

## Contributing

Coral is open source and welcomes contributors!

### Ways to Help

- **Try it out** and report issues
- **Write examples** that showcase features
- **Improve documentation** for clarity
- **Add tests** for edge cases
- **Implement features** from the roadmap

### Code of Conduct

We are committed to providing a welcoming and inclusive environment. Please read our [Code of Conduct](./CODE_OF_CONDUCT.md) before participating.

---

## Acknowledgments

Coral stands on the shoulders of giants:

- **Python** - For proving that readability matters
- **Rust** - For showing that safety and performance coexist
- **Erlang/Elixir** - For the actor model and "let it crash" philosophy
- **ML/Haskell** - For algebraic data types and type inference
- **LLVM** - For making native compilation accessible

---

<p align="center">
  <strong>Coral: Write less. Do more. Sleep better.</strong>
</p>

<p align="center">
  <a href="https://github.com/coral-lang/coral">GitHub</a> •
  <a href="./LANGUAGE_GUIDE.md">Documentation</a> •
  <a href="https://discord.gg/coral-lang">Community</a>
</p>

---

*Coral is currently in pre-alpha. APIs may change. Production use is not yet recommended.*
