# Coral Examples

Example programs demonstrating Coral's features.

## Examples

| File | Description | Status |
|------|-------------|--------|
| `hello.coral` | Variables, lists, maps, ternaries, template strings | **Runs** |
| `calculator.coral` | Arithmetic, match expressions, conditional logic | **Runs** |
| `traits_demo.coral` | Trait definitions, default methods, implementations | **Runs** |
| `data_pipeline.coral` | Store construction, iteration, data processing | **Compiles** (display issues) |
| `fizzbuzz.coral` | Classic FizzBuzz with tuple pattern matching | **Parse error** (tuple patterns unsupported) |
| `chat_server.coral` | Multi-user chat with actors and stores | **Lex error** (indentation) |
| `http_server.coral` | Simple HTTP server using actors | **Lex error** (indentation) |

## Running Examples

```bash
# Build the compiler and runtime first
cargo build
cargo build -p runtime --release

# Run via JIT
./target/debug/coralc --jit examples/hello.coral

# Compile to native binary
./target/debug/coralc examples/hello.coral --emit-binary ./hello
./hello

# Emit LLVM IR only
./target/debug/coralc examples/hello.coral --emit-ir hello.ll
```
