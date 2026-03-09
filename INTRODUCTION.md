# Introducing Coral

**A language that reads like Python, runs like C, and scales like Erlang.**

---

Coral is a new programming language built on a simple conviction: you shouldn't have to choose between code that's pleasant to write and code that's fast to run. It combines indentation-based syntax with LLVM-compiled native performance, a built-in actor model for concurrency, and full type inference — so you never write a type annotation, but the compiler catches your mistakes anyway.

```coral
*main()
    name is 'World'
    log('Hello, {name}!')

    fruits is ['apple', 'banana', 'cherry']
    log('First: {fruits[0]}')

    status is 42 > 40 ? 'big' ! 'small'
    log('Answer is {status}')
```

No braces. No semicolons. No `=` vs `==` confusion — Coral uses `is` for binding and methods like `.equals()` for comparison, eliminating an entire class of bugs by design.

---

## What Makes Coral Different

### Everything is inferred

Coral has a constraint-based type system inspired by Hindley-Milner inference. You write code that looks dynamically typed, but the compiler statically verifies it before a single instruction runs. There are no type annotations anywhere in the syntax — not because they're optional, but because they don't exist.

```coral
*factorial(n)
    n <= 1 ? 1 ! n * factorial(n - 1)

*process_users(users)
    users
        ~ filter($.age >= 18)
        ~ map($.name)
        ~ sort()
        ~ take(10)
```

The compiler infers that `factorial` takes and returns a number, that `users` is a list, that `$` refers to each element — all without you saying so.

### Actors are built in, not bolted on

Concurrency in Coral isn't a library — it's part of the language grammar. Actors are declared with the `actor` keyword, message handlers with `@`, and that's it. No locks, no mutexes, no colored functions.

```coral
actor counter
    value ? 0

    @increment(amount)
        value is value + amount
        value

    @reset()
        value is 0

c is counter()
c.increment(5)
c.increment(3)
```

Actors run on an M:N scheduler, communicate through bounded mailboxes with automatic backpressure, and support supervision trees for fault recovery. The model comes from Erlang, but the syntax comes from working with it every day and wanting it to be simpler.

### Errors are values, not exceptions

Coral has no `try`/`catch`. No stack unwinding. Every value can carry an error state as an intrinsic attribute — you don't wrap things in `Result<T, E>` and unwrap them; you check when you want to and propagate otherwise.

```coral
*connect(host)
    host.is_empty() ? ! err Connection:Refused
    socket is open_socket(host) ! return err
    handshake(socket) ! return err
    socket

result is connect('db.local')
result.is_err ?
    log('Failed: {result.err}')
    use_fallback()
  ! process(result)
```

Errors are defined hierarchically (`err Database:Connection:Timeout`), propagated explicitly with `! return err`, and can never be silently ignored.

### Persistent stores

Coral has a `store` keyword for data that survives process restarts. Stores look like types, but the runtime backs them with a write-ahead log and dual-format persistence engine. You get durable state without an ORM, without a database driver, without leaving the language.

```coral
store product
    sku
    name
    price ? 0.0
    stock ? 0

    *in_stock()
        stock.greater_than(0)

p is product('SKU-42', 'Widget', 19.99, 100)
```

### Pattern matching and algebraic data types

Define variants. Match exhaustively. The compiler checks that you handle every case.

```coral
enum Shape
    Circle(radius)
    Rectangle(width, height)
    Triangle(base, height)

*area(shape)
    match shape
        Circle(r) ? 3.14159 * r * r
        Rectangle(w, h) ? w * h
        Triangle(b, h) ? 0.5 * b * h
```

### Traits with defaults

```coral
trait Printable
    *to_string()

    *print()
        log(to_string())

type Point
    with Printable
    x ? 0
    y ? 0

    *to_string()
        'Point({x}, {y})'

p is Point(3, 4)
p.print()
```

---

## A Real Language

Coral isn't a paper design or a weekend prototype. The compiler is written in both Rust (~16,000 lines) and Coral itself (~7,700 lines), and **the self-hosted compiler bootstraps** — it compiles itself, and the output compiles itself again to produce byte-identical results. This is the standard proof that a language works: not for toy programs, but for a complex, multi-module codebase with recursive data structures, cross-module dependencies, and real code generation.

The numbers, as of March 2026:

| | |
|---|---|
| **Tests** | 745+, all passing |
| **Self-hosting tests** | 30, including 7 end-to-end execution tests |
| **Standard library** | 20 modules, ~1,900 lines of Coral |
| **Runtime** | ~25,000 lines of Rust — tagged values, refcounting with cycle detection, actor scheduler, WAL-backed persistence |
| **Compiler** | Lexer → Parser → Semantic → Lower → LLVM Codegen, full pipeline |
| **Bootstrap** | gen2 == gen3, byte-for-byte identical |

The self-hosted compiler is 2.1x more concise than the Rust reference compiler. Not because it's incomplete — because Coral doesn't need braces, semicolons, type annotations, or lifetime markers.

---

## How It Works

Coral compiles to native code through LLVM. The pipeline:

```
Source → Lexer → Parser → Semantic Analysis → Lowering → LLVM IR → Native Binary
```

The lexer is indent-aware — it emits `INDENT`, `DEDENT`, and `NEWLINE` tokens so the parser doesn't need braces. The semantic pass runs constraint-based type inference over the entire program. The codegen phase emits LLVM IR text, which LLVM's toolchain optimizes and compiles to machine code.

You can run programs three ways:

```bash
# JIT — fast iteration
coralc --jit program.coral

# Native binary — ship it
coralc program.coral --emit-binary ./program
./program

# Inspect the IR
coralc program.coral --emit-ir out.ll
```

The runtime is a shared library providing 220+ FFI functions: tagged value operations, reference counting, list/map/string manipulation, actor scheduling, store persistence, JSON parsing, networking, and more.

---

## The Syntax in 60 Seconds

```coral
# Variables
name is 'Coral'
count is 42
ready is true

# Functions start with *
*greet(person)
    'Hello, {person}!'

# Ternary: condition ? then ! else
size is count > 100 ? 'big' ! 'small'

# Lists and maps
items is [1, 2, 3]
config is map('host' is 'localhost', 'port' is 8080)

# Loops
for item in items
    log('{item}')

while count > 0
    count is count - 1

# Pipeline operator
result is [1, 2, 3, 4, 5]
    ~ map($ * 2)
    ~ filter($ > 4)
    ~ sum()

# Algebraic data types
enum Option
    Some(value)
    None

# Pattern matching
match response
    Some(v) ? process(v)
    None ? log('nothing')

# Actors
actor logger
    @write(level, text)
        log('[{level}] {text}')

# Stores (persistent objects)
store user
    username
    email
    created_at ? now()

# Traits
trait Serializable
    *serialize()

# Error hierarchies
err Network
    err Timeout
    err Refused
```

---

## Design Principles

**`is` for binding.** The `=` and `==` tokens don't exist in Coral. This isn't arbitrary — it eliminates the most common class of bugs in C-family languages and makes code read more naturally.

**Pure type inference.** Types are for the compiler to figure out, not for you to write. The constraint solver handles generics, ADTs, closures, and method dispatch without a single annotation.

**One numeric type.** At runtime, numbers are `f64`. The AST distinguishes `Int` and `Float` for constant folding, but there's no `i32` vs `u64` vs `f32` to juggle. For the programs Coral targets, this is the right tradeoff.

**Errors are data.** No hidden control flow. Every error originates at a known point, propagates explicitly, and can be inspected or handled at any level.

**Concurrency is structural.** Actors aren't threads with extra steps — they're a language-level construct with their own declaration syntax, message protocol, and scheduling model.

---

## Current Status

Coral is in **pre-alpha**. The core language works. The self-hosted compiler bootstraps. The standard library covers math, I/O, strings, collections, JSON, networking, time, encoding, sorting, and testing. Five of seven example programs compile and run as native binaries.

What's still ahead: completing the self-hosted runtime (replacing the Rust runtime with Coral), performance optimization passes, a language server for editor integration, and growing the standard library toward production use.

Coral is open source. If you're interested in a language that values clarity without sacrificing power, we'd welcome you.

```bash
git clone https://github.com/coral-lang/coral
cd coral
cargo build --release
./target/release/coralc --jit examples/hello.coral
```

---

*Coral: Write less. Do more. Sleep better.*
