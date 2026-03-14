# Getting Started with Coral

## Prerequisites

- **Rust toolchain** (1.70+): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **LLVM 16**: For JIT execution and native binary compilation
  - Ubuntu/Debian: `sudo apt install llvm-16`
  - macOS: `brew install llvm@16`

## Building from Source

```bash
git clone <repo-url> coral
cd coral
make build          # Build compiler + runtime (debug)
make install        # Install to ~/.cargo/bin
```

Or without Make:
```bash
cargo build
cargo build -p runtime
```

## Your First Program

Create `hello.coral`:
```coral
*main()
    log('Hello, Coral!')
```

Run it:
```bash
coralc --jit hello.coral
```

## Language Basics

### Variables (Bindings)

Coral uses `is` for binding — there is no `=` operator:
```coral
name is 'Alice'
age is 30
pi is 3.14159
active is true
```

### Functions

Functions are declared with `*` and use indentation for scope:
```coral
*greet(name)
    log('Hello, ${name}!')

*add(a, b)
    a + b

*main()
    greet('World')
    result is add(2, 3)
    log('2 + 3 = ${result}')
```

### Conditionals

```coral
*classify(n)
    if n > 100
        'large'
    elif n > 10
        'medium'
    else
        'small'
```

Ternary: `condition ? true_value ! false_value`
```coral
label is age >= 18 ? 'adult' ! 'minor'
```

### Loops

```coral
# For loop over a collection
for item in [1, 2, 3]
    log(item)

# While loop
i is 0
while i < 10
    log(i)
    i is i + 1

# Infinite loop with break
loop
    if done
        break
```

### Lists and Maps

```coral
# Lists
fruits is ['apple', 'banana', 'cherry']
first is fruits[0]
count is fruits.length

# Maps
config is map('host' is 'localhost', 'port' is 8080)
host is config.get('host')
```

### Template Strings

Single quotes with `${}` or `{}` interpolation:
```coral
name is 'Coral'
log('Hello, ${name}!')
log('2 + 2 = ${2 + 2}')
```

### Match Expressions

```coral
match value
    1 ? 'one'
    2 ? 'two'
    _ ? 'other'
```

With enum variants:
```coral
enum Shape
    Circle(radius)
    Rect(width, height)

match shape
    Circle(r) ? 3.14 * r * r
    Rect(w, h) ? w * h
```

### Error Handling

Errors are values, not exceptions:
```coral
err NotFound
err InvalidInput

*find_user(id)
    id < 0 ? ! err InvalidInput
    # ... lookup logic
    ! err NotFound

result is find_user(42)
result.is_err ?
    log('Error: ${result.err}')
```

Propagate errors with `! return err`:
```coral
*process()
    user is find_user(1) ! return err
    log('Found: ${user}')
```

### Pipelines

Chain operations with `~`:
```coral
result is [3, 1, 4, 1, 5]
    ~ sort()
    ~ take(3)
    ~ map(*fn(x) x * 2)
```

### Type Definitions

```coral
type Point
    x
    y

p is Point(1, 2)
log('x=${p.x}, y=${p.y}')
```

### Traits

```coral
trait Printable
    *to_string()

type Circle with Printable
    radius

    *to_string()
        'Circle(r=${radius})'
```

### Lambdas

```coral
double is *fn(x) x * 2
items is [1, 2, 3] ~ map(double)

# Or inline with do..end blocks:
[1, 2, 3].each() do
    log($)
end
```

## Running Tests

Create a test file with `*test_` prefixed functions:
```coral
*test_addition()
    result is 2 + 2
    log('2 + 2 = ${result}')

*test_string()
    name is 'Coral'
    log('Hello ${name}')
```

Run:
```bash
coralc --test my_tests.coral
coralc --test --test-filter addition my_tests.coral    # Filter by name
```

## Compiling to Binary

```bash
# Emit LLVM IR
coralc --emit-ir output.ll hello.coral

# Compile to native binary
coralc --emit-binary hello hello.coral

# With optimizations
coralc --emit-binary hello -O2 hello.coral

# With link-time optimization
coralc --emit-binary hello -O2 --lto hello.coral
```

## Project Structure

Initialize a new project:
```bash
coralc --init my_project
```

This creates:
```
my_project/
  coral.toml      # Project manifest
  src/
    main.coral    # Entry point
```

## Standard Library

Import modules with `use`:
```coral
use std.math
use std.string
use std.io
```

Available modules: `math`, `string`, `io`, `json`, `collections`, `fmt`, `random`, `sort`, `path`, `process`, `crypto`, `encoding`, `debug`, `testing`.

## Further Reading

- [INTRODUCTION.md](INTRODUCTION.md) — Language philosophy and design
- [docs/LANGUAGE_EVOLUTION_ROADMAP.md](docs/LANGUAGE_EVOLUTION_ROADMAP.md) — Feature roadmap
- [docs/syntax.coral](docs/syntax.coral) — Syntax reference
- [std/overview.md](std/overview.md) — Standard library overview
