# Coral Value-Error Model Specification

_Created: 2026-01-06_

## Executive Summary

Coral takes a fundamentally different approach to error handling and optional values. Rather than wrapping values in `Result<T, E>` or `Option<T>` containers that require explicit unwrapping, **every value in Coral can carry error/absence metadata as an intrinsic attribute**. This design eliminates boilerplate while maintaining safety.

---

## 1. Core Philosophy

### 1.1 Design Principles

1. **No Containers**: Values are not wrapped in Result/Option - they ARE results/options
2. **Transparent Propagation**: Errors flow through operations automatically unless explicitly handled
3. **No Unwrapping**: Direct use of values - no `.unwrap()`, `.?`, or pattern matching required for the common case
4. **Complete Type Inference**: The error/absence state is inferred, not declared
5. **Explicit When Needed**: Clean syntax for error handling when you want to handle it

### 1.2 Mental Model

```coral
# In other languages:
result = do_something()    # Returns Result<Value, Error>
value = result.unwrap()    # Must unwrap to get value
use(value)

# In Coral:
value = do_something()     # Returns Value (which MAY be an error)
use(value)                 # Works directly - errors propagate automatically
```

---

## 2. Value Representation

### 2.1 Runtime Structure

Every Coral value has a header with flag bits:

```
┌─────────────────────────────────────────────────────────────┐
│                    CoralValue Header                         │
├──────────────┬───────────┬───────────┬──────────────────────┤
│  refcount    │  tag      │  flags    │  payload_ptr         │
│  (8 bytes)   │  (1 byte) │  (1 byte) │  (8 bytes)           │
└──────────────┴───────────┴───────────┴──────────────────────┘

Tag: Number, String, List, Map, Bool, Unit, Bytes, Actor, Store

Flags byte:
  bit 0: ERR     - Value represents an error state
  bit 1: ABSENT  - Value is logically None/missing
  bit 2-7: Reserved for future use
```

### 2.2 Error Metadata (Optional)

When ERR flag is set, an additional metadata structure is accessible:

```
┌─────────────────────────────────────────────────────────────┐
│                    Error Metadata                            │
├──────────────────┬──────────────────┬───────────────────────┤
│  error_code      │  error_name      │  origin_span          │
│  (4 bytes)       │  (ValueHandle)   │  (SpanId)             │
└──────────────────┴──────────────────┴───────────────────────┘
```

---

## 3. Syntax Design

### 3.1 Error Definitions

Errors are defined hierarchically using the `err` keyword:

```coral
err Database
    err Connection
        err Timeout
            code is 5001
            message is 'DB Connection Timed Out'
        err Refused
            code is 5002
            message is 'Connection Refused'
    err Query
        err Syntax
            code is 4001
            message is 'Invalid SQL'
```

### 3.2 Returning Errors

Functions return errors using `err` as a value:

```coral
*connect_db(host)
    host.is_empty() ? ! err Connection:Refused
    ping(host) > 1000 ? ! err Connection:Timeout
    true  # success case
```

### 3.3 Conditional Expression with Error

The ternary `?` `!` syntax handles conditions and errors:

```coral
# Basic conditional
x = condition ? true_value ! false_value

# Error return on failure
*do_something(p)
    p.is_active() ? p.process() ! err NotActive
```

### 3.4 Error Propagation

Errors propagate automatically, but can be explicitly handled:

```coral
*some_function(bar)
    # If do_something returns an error, return it immediately
    foo = do_something(bar) ! return err
    
    # Continue with foo (which is guaranteed non-error here)
    process(foo)
```

The `! return err` is a propagation directive that says "if the preceding expression is an error, return that error from this function."

### 3.5 Error Checking and Handling

```coral
*main()
    x = some_function(input())
    
    # Nested error handling
    x ? go_ahead(x) ?
        ! ga_error_handler()      # Handle go_ahead errors
      ! sf_error_handler(x)       # Handle some_function errors
```

### 3.6 Naked Error in Match

```coral
*go_ahead(x)
    return match something_else(x)
        101 ? 'ok'
        102 ? 'no'
            ! err  # naked err returns generic error
```

---

## 4. Semantic Rules

### 4.1 Error Propagation

When an operation involves an error value, the error propagates:

```coral
x = err NotFound       # x is an error
y = x + 5              # y is ALSO an error (same error as x)
z = y.to_string()      # z is ALSO an error
```

### 4.2 Short-Circuit Evaluation

Operations short-circuit on errors:

```coral
# If get_user returns error, validate_user is never called
user = get_user(id)
valid = validate_user(user)  # Skipped if user is error
```

### 4.3 Value Methods

Every value has these intrinsic methods:

```coral
value.is_ok       # true if not error and not absent
value.is_err      # true if ERR flag set
value.is_absent   # true if ABSENT flag set
value.err         # access error metadata (or unit if not error)
value.or(default) # return value if ok, else default
```

### 4.4 Explicit Error Checks

```coral
*safe_divide(a, b)
    b == 0 ? ! err DivisionByZero
    a / b

*calculate()
    result = safe_divide(10, x)
    
    # Explicit check
    result.is_err ? 
        log('Division failed: {result.err}')
        0
      ! result * 2
```

---

## 5. Type System Integration

### 5.1 No Generic Result Types Needed

The type system doesn't need `Result[T, E]` because:
- Every type can carry error state
- Error metadata is orthogonal to value type
- Type inference tracks "may-error" vs "definitely-ok"

### 5.2 Inferred Error States

```coral
*always_works(x)
    x + 1           # Never errors, compiler knows this

*might_fail(x)
    x > 0 ? x ! err Negative  # May error, compiler tracks this

*uses_both()
    a = always_works(5)       # Type: Number (ok)
    b = might_fail(input())   # Type: Number (may-error)
```

### 5.3 Compile-Time Checks

The compiler warns when:
- Error values are silently ignored at top level
- Propagation patterns don't handle all error paths

---

## 6. Runtime Behavior

### 6.1 Error Propagation in Operations

```rust
// Runtime pseudo-code for binary operation
fn coral_add(left: Value, right: Value) -> Value {
    if left.is_err() { return left; }
    if right.is_err() { return right; }
    // Normal addition
    Value::number(left.as_number() + right.as_number())
}
```

### 6.2 Error Creation

```rust
// Creating an error value at runtime
fn coral_make_error(name: &str, code: u32, origin: SpanId) -> Value {
    let mut value = Value::unit();
    value.flags |= FLAG_ERR;
    value.error_meta = Some(ErrorMeta { name, code, origin });
    value
}
```

### 6.3 Top-Level Error Handling

Unhandled errors at the program's top level trigger a runtime diagnostic:

```
Error: Connection:Timeout
  Code: 5001
  Message: DB Connection Timed Out
  Origin: main.coral:45:12
```

---

## 7. Comparison with Other Languages

### 7.1 vs Rust Result/Option

```rust
// Rust
fn get_user(id: i32) -> Result<User, Error> { ... }
let user = get_user(5)?;  // Must use ? to propagate
```

```coral
# Coral
*get_user(id)
    ...

user = get_user(5)        # Errors propagate automatically
user = get_user(5) ! return err  # Explicit propagation
```

### 7.2 vs Go Error Handling

```go
// Go
user, err := getUser(5)
if err != nil {
    return nil, err
}
```

```coral
# Coral
user = get_user(5) ! return err
# No need for explicit nil check - it's one expression
```

### 7.3 vs Exceptions

Unlike exceptions:
- Errors are values, not control flow
- Errors don't unwind the stack
- Errors are visible in the value system
- No try/catch blocks needed

---

## 8. Implementation Plan

### 8.1 Phase 1: Runtime Changes

1. **Extend ValueHeader**
   - Add flags byte to header structure
   - Add optional error_meta pointer
   - Update all value allocation paths

2. **Error Propagation**
   - Modify all runtime operations to check ERR flag
   - Implement short-circuit behavior
   - Add `coral_make_error` runtime function

### 8.2 Phase 2: Parser/AST

1. **Error Syntax**
   - Parse `err Name` as error value expression
   - Parse `! return err` propagation syntax
   - Parse error definitions (`err Hierarchy`)

2. **AST Nodes**
   - Add `ErrorValue` expression variant
   - Add `ErrorPropagate` statement variant
   - Add `ErrorDef` top-level item

### 8.3 Phase 3: Codegen

1. **Error Value Emission**
   - Generate calls to `coral_make_error`
   - Track error hierarchies in string pool
   
2. **Propagation Codegen**
   - Generate error checks at propagation points
   - Emit early returns for error propagation

### 8.4 Phase 4: Type System

1. **Error Tracking**
   - Add "may-error" type state to inference
   - Track error paths through functions
   
2. **Warnings**
   - Warn on unhandled errors at top level
   - Suggest propagation for unhandled cases

---

## 9. Standard Library Support

### 9.1 `std.error` Module

```coral
# Error inspection
*error_code(value)
    value.is_err ? value.err.code ! 0

*error_name(value)
    value.is_err ? value.err.name ! ''

*error_message(value)
    value.is_err ? value.err.message ! ''

# Error utilities
*require(condition, error)
    condition ? true ! error

*ensure_ok(value)
    value.is_err ? throw value.err ! value
```

### 9.2 Common Error Definitions

```coral
err Core
    err NotFound
    err InvalidArgument
    err OutOfBounds
    err TypeMismatch
    err DivisionByZero

err IO
    err FileNotFound
    err PermissionDenied
    err ReadFailed
    err WriteFailed
    err EndOfFile

err Parse
    err InvalidFormat
    err UnexpectedToken
    err UnexpectedEOF
```

---

## 10. Future Extensions

### 10.1 Typed Error Contracts (Optional)

For critical paths, allow error type hints:

```coral
*connect(host) ! Connection
    # Compiler ensures only Connection errors returned
```

### 10.2 Error Recovery

```coral
*with_retry(times, fn)
    result = fn()
    result.is_err and times > 0 ?
        with_retry(times - 1, fn)
      ! result
```

### 10.3 Error Context Chains

```coral
*process_file(path)
    content = read(path) ! return err.with_context('processing {path}')
    parse(content)
```

---

## Appendix: Grammar Updates

```ebnf
error_def     = 'err' IDENTIFIER error_body? ;
error_body    = NEWLINE INDENT error_field* DEDENT ;
error_field   = IDENTIFIER 'is' expression NEWLINE ;

error_expr    = 'err' error_path ;
error_path    = IDENTIFIER (':' IDENTIFIER)* ;

propagation   = expression '!' 'return' 'err' ;
```
