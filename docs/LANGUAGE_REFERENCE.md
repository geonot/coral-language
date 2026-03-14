# Coral Language Reference

## Lexical Elements

### Keywords
`if` `elif` `else` `while` `for` `in` `loop` `break` `continue` `return`
`match` `when` `unless` `until` `is` `isnt` `and` `or` `true` `false`
`none` `type` `enum` `trait` `with` `store` `actor` `persist` `err`
`use` `extern` `fn` `do` `end` `unsafe` `asm` `map`

### Operators (by precedence, lowest to highest)
| Precedence | Operators | Description |
|------------|-----------|-------------|
| 1 | `~` | Pipeline |
| 2 | `? !` | Ternary, error propagation |
| 3 | `or` | Logical OR |
| 4 | `and` | Logical AND |
| 5 | `< > <= >=` | Comparison |
| 6 | `is isnt` | Equality |
| 7 | `\|` | Bitwise OR |
| 8 | `^` | Bitwise XOR |
| 9 | `&` | Bitwise AND |
| 10 | `<< >>` | Shift |
| 11 | `+ -` | Addition |
| 12 | `* / %` | Multiplication |
| 13 | `- !` (unary) | Negation, logical NOT |
| 14 | `()` | Call |
| 15 | `.` `[]` | Member access, index |

### Literals
- **Integer**: `42`, `0xFF`, `0b1010`, `0o77`, `1_000_000`
- **Float**: `3.14`, `1_000.5`
- **String**: `'hello'`, `"hello"` (double-quote for raw)
- **Template**: `'Hello ${name}!'` or `'Hello {name}!'`
- **Boolean**: `true`, `false`
- **None**: `none`
- **Unit**: `()`
- **List**: `[1, 2, 3]`
- **Map**: `map('key' is value, 'k2' is v2)`
- **Bytes**: `b"raw bytes"`

## Declarations

### Functions
```coral
*name(param1, param2)
    body

# With type annotations (optional, never required)
*name(param1: Type, param2: Type)
    body

# With default parameters
*name(param ? default_value)
    body
```

### Types
```coral
type Name
    field1
    field2 ? default_value
    &mutable_field           # Mutable field

    *method(self_field_access)
        body

# With trait implementation
type Name with Trait1, Trait2
    fields...
```

### Enums (ADTs)
```coral
enum Name
    Variant1
    Variant2(field1, field2)
```

### Traits
```coral
trait Name
    *method_signature(params)

    # Can include default implementations
    *default_method(params)
        body
```

### Errors
```coral
err Name                    # Simple error
err Name                    # Error hierarchy
    err ChildError1
    err ChildError2

# Error values
! err Name                  # Throw error
value ! return err          # Propagate error
value.is_err                # Check for error
```

### Stores (Stateful objects)
```coral
store Name
    field1
    field2 ? default

    *method()
        body
```

### Actors (Concurrent entities)
```coral
actor Name
    state_field ? initial

    @message_handler(params)
        body

    *private_method()
        body
```

### Extern Functions (FFI)
```coral
extern fn c_function_name(param: Type): ReturnType
```

## Statements

### Binding
```coral
x is 42                     # Immutable binding
x is x + 1                  # Rebinding (shadowing)
obj.field is value           # Field assignment
```

### Control Flow
```coral
# If/elif/else
if condition
    body
elif condition
    body
else
    body

# Match
match value
    pattern1 ? result1
    pattern2 ? result2
    _ ? default_result

# Match with blocks
match value
    pattern ?
        statement1
        statement2
    _ ? default

# When (condition chains)
when
    condition1 ? result1
    condition2 ? result2
    _ ? default

# Loops
while condition
    body

for item in iterable
    body

for i in start..end
    body

loop
    body
    if done
        break

# Loop control
break
continue
```

### Imports
```coral
use std.io
use std.math
use mymodule
```

## Expressions

### Ternary
```coral
condition ? true_value ! false_value
```

### Pipeline
```coral
value ~ transform1() ~ transform2()
```

### Lambda
```coral
*fn(x) x * 2               # Expression lambda
*fn(x, y)                   # Block lambda
    x + y

# do..end trailing lambda
list.each() do
    log($)
end
```

### Error Propagation
```coral
result ! return err          # Propagate if error
condition ? ! err Name       # Guard: throw if falsy
```

### Spread
```coral
combined is [...list1, ...list2]
```

### List Comprehension
```coral
[x * 2 for x in items if x > 0]
```

## Match Patterns

```coral
42                          # Integer literal
'hello'                     # String literal
true / false                # Boolean
none                        # None
Variant(x, y)               # Constructor destructure
[a, b, c]                   # List destructure
[head, ...rest]             # List with rest
(a, b)                      # Tuple destructure
_                           # Wildcard
x                           # Binding (captures value)
A or B                      # Or-pattern
x when condition            # Guard
1..10                       # Range pattern
```

## Module System

Files are modules. The filename is the module name:
```
project/
  coral.toml
  src/
    main.coral          # Entry point (*main function)
    utils.coral         # use utils
    math/
      vectors.coral     # use math.vectors
```

## Concurrency Model

Actors communicate via message passing:
```coral
actor Counter
    count ? 0

    @increment(n)
        count is count + n
        count

c is Counter()
result is c.increment(5)    # Sends message, gets response
```

Actors run on an M:N scheduler with bounded mailboxes and automatic backpressure.

## Memory Model

- NaN-boxed values: all values fit in 64 bits
- Reference-counted heap objects with cycle detection
- No garbage collector pauses
- Deterministic destruction
