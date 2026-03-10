# Coral Standard Library Specification

_Created: 2026-01-06_
_Revised: 2026-01-07 - Coral-native syntax, no type annotations_

## 1. Overview

This document specifies the complete standard library for Coral. The standard library provides foundational modules, functions, and values that every Coral program can rely on.

### 1.1 Design Principles

1. **Batteries Included**: Common tasks shouldn't require external libraries
2. **Consistent API**: Similar patterns across all modules
3. **Performance**: No hidden costs, predictable performance
4. **Safety**: Default to safe operations, explicit unsafe when needed
5. **Coral-Native**: APIs feel natural in Coral, not ported from other languages
6. **No Type Annotations**: Coral uses dynamic typing with runtime error values

### 1.2 Module Organization

```
std/
├── prelude.coral       # Auto-imported basics
├── collections/
│   ├── list.coral      # List operations
│   ├── map.coral       # Map operations
│   ├── set.coral       # Set operations
│   └── queue.coral     # Queue/Deque
├── text/
│   ├── string.coral    # String operations
│   ├── regex.coral     # Regular expressions
│   └── format.coral    # String formatting
├── io/
│   ├── file.coral      # File operations
│   ├── path.coral      # Path manipulation
│   ├── console.coral   # Console I/O
│   └── stream.coral    # Streaming I/O
├── math/
│   ├── basic.coral     # Basic math functions
│   ├── trig.coral      # Trigonometry
│   └── random.coral    # Random number generation
├── time/
│   ├── datetime.coral  # Date and time
│   ├── duration.coral  # Time durations
│   └── timer.coral     # Timing utilities
├── crypto/
│   ├── hash.coral      # Hashing functions
│   └── random.coral    # Cryptographic random
├── net/
│   ├── http.coral      # HTTP client/server
│   ├── tcp.coral       # TCP sockets
│   ├── udp.coral       # UDP sockets
│   └── url.coral       # URL parsing
├── json/
│   └── mod.coral       # JSON parse/serialize
├── encoding/
│   ├── base64.coral    # Base64 encoding
│   ├── hex.coral       # Hex encoding
│   └── utf8.coral      # UTF-8 utilities
├── runtime/
│   ├── actor.coral     # Actor primitives
│   ├── memory.coral    # Memory operations
│   └── value.coral     # Value introspection
└── testing/
    └── assert.coral    # Testing assertions
```

---

## 2. Core Concepts

### 2.1 Values and Errors

Coral uses a unified value-error model. Any value can be a normal value or an error value:

```coral
# Normal values
x is 42
name is 'Alice'
items is [1, 2, 3]

# Error values
bad is err NotFound
result is err Validation:InvalidInput

# Check if a value is an error
result.err ? handle_error() ! process(result)
```

### 2.2 Guard Clauses

Use `cond ! err Name` to return an error if condition is false:

```coral
*validate_age(age)
    age >= 0 ! err Validation:NegativeAge
    age <= 150 ! err Validation:UnrealisticAge
    age

*safe_divide(a, b)
    b != 0 ! err Math:DivByZero
    a / b
```

### 2.3 Error Propagation

Use `expr ! return err` to propagate errors up the call stack:

```coral
*process_data(id)
    data is fetch(id) ! return err
    validated is validate(data) ! return err
    transform(validated)
```

### 2.4 Optional Values

Use `none` for absent/missing values:

```coral
*find_user(id)
    user is db.lookup(id)
    user.err ? none ! user

*get_setting(key, default)
    value is config.get(key)
    value.err ? default ! value
```

---

## 3. Prelude (Auto-imported)

These functions are available in all Coral programs without explicit import:

```coral
# std/prelude.coral

*log_line(value)
    log(value)
    value

*log_with_prefix(prefix, value)
    message is prefix + value
    log(message)
    message

*identity(input)
    input

*tap(value)
    log_line(value)
    value

*if_value(condition, value)
    condition ? value ! none

# Constants
true_value is true
false_value is false
```

### Proposed Additions

```coral
# Type checking
*is_number(value)
    # Runtime introspection
    
*is_string(value)
    # Runtime introspection

*is_list(value)
    # Runtime introspection

*is_map(value)
    # Runtime introspection

# Comparison
*min(a, b)
    a < b ? a ! b

*max(a, b)
    a > b ? a ! b

*clamp(value, low, high)
    min(max(value, low), high)

# Control flow
*unless(condition, value)
    not condition ? value ! none

*repeat(n, fn)
    i is 0
    loop i < n
        fn(i)
        i is i + 1
```

---

## 4. Collections

### 4.1 `std.collections.list`

```coral
use std/collections/list

# List creation
*empty_list()
    []

*list_from_range(start, end)
    result is []
    i is start
    loop i < end
        result.push(i)
        i is i + 1
    result

*list_repeat(value, n)
    result is []
    i is 0
    loop i < n
        result.push(value)
        i is i + 1
    result

# Basic operations (many exist as methods)
*length(list)
    list.length

*is_empty(list)
    list.length == 0

*first(list)
    is_empty(list) ? none ! list.get(0)

*last(list)
    is_empty(list) ? none ! list.get(list.length - 1)

*get_or(list, index, default)
    item is list.get(index)
    item.err ? default ! item

# Transformations
*reversed(list)
    result is []
    i is list.length - 1
    loop i >= 0
        result.push(list.get(i))
        i is i - 1
    result

*sorted(list)
    list.sort()

*sorted_by(list, key_fn)
    # Sort using key function for comparison
    list.sort(key_fn)

*unique(list)
    seen is {}
    result is []
    for item in list
        not seen.contains(item) ? 
            seen.set(item, true)
            result.push(item)
        ! none
    result

# Searching
*index_of(list, value)
    i is 0
    for item in list
        item == value ? i ! none
        i is i + 1
    none

*contains(list, value)
    found is index_of(list, value)
    not found.err

*count(list, pred)
    n is 0
    for item in list
        pred(item) ? n is n + 1 ! none
    n

# Aggregation
*sum(list)
    total is 0
    for item in list
        total is total + item
    total

*product(list)
    result is 1
    for item in list
        result is result * item
    result

# Partitioning
*take(list, n)
    result is []
    i is 0
    loop i < n and i < list.length
        result.push(list.get(i))
        i is i + 1
    result

*drop(list, n)
    result is []
    i is n
    loop i < list.length
        result.push(list.get(i))
        i is i + 1
    result

*split_at(list, index)
    [take(list, index), drop(list, index)]

# Zipping
*zip(list1, list2)
    result is []
    len is min(list1.length, list2.length)
    i is 0
    loop i < len
        result.push([list1.get(i), list2.get(i)])
        i is i + 1
    result

*zip_with(list1, list2, fn)
    result is []
    len is min(list1.length, list2.length)
    i is 0
    loop i < len
        result.push(fn(list1.get(i), list2.get(i)))
        i is i + 1
    result

*enumerate(list)
    result is []
    i is 0
    for item in list
        result.push([i, item])
        i is i + 1
    result

# Flattening
*flatten(list_of_lists)
    result is []
    for sublist in list_of_lists
        for item in sublist
            result.push(item)
    result

*flat_map(list, fn)
    flatten(list.map(fn))
```

### 4.2 `std.collections.map`

```coral
use std/collections/map

# Map creation
*empty_map()
    {}

*map_from_pairs(pairs)
    result is {}
    for pair in pairs
        result.set(pair.get(0), pair.get(1))
    result

# Basic operations
*has_key(map, key)
    value is map.get(key)
    not value.err

*get_or(map, key, default)
    value is map.get(key)
    value.err ? default ! value

*keys(map)
    map.keys()

*values(map)
    result is []
    for key in map.keys()
        result.push(map.get(key))
    result

*entries(map)
    result is []
    for key in map.keys()
        result.push([key, map.get(key)])
    result

# Transformations
*map_values(map, fn)
    result is {}
    for key in map.keys()
        result.set(key, fn(map.get(key)))
    result

*filter_map(map, pred)
    result is {}
    for key in map.keys()
        value is map.get(key)
        pred(key, value) ? result.set(key, value) ! none
    result

# Merging
*merge(map1, map2)
    result is {}
    for key in map1.keys()
        result.set(key, map1.get(key))
    for key in map2.keys()
        result.set(key, map2.get(key))
    result

*merge_with(map1, map2, combine_fn)
    result is {}
    for key in map1.keys()
        result.set(key, map1.get(key))
    for key in map2.keys()
        has_key(result, key) ?
            result.set(key, combine_fn(result.get(key), map2.get(key)))
        ! result.set(key, map2.get(key))
    result
```

### 4.3 `std.collections.set`

```coral
use std/collections/set

# Set implementation using map
*empty_set()
    { __set: true }

*set_from_list(list)
    s is empty_set()
    for item in list
        set_add(s, item)
    s

*set_add(set, value)
    set.set(value, true)
    set

*set_remove(set, value)
    set.remove(value)
    set

*set_contains(set, value)
    has_key(set, value)

*set_size(set)
    set.keys().length - 1  # Subtract __set marker

# Set operations
*set_union(set1, set2)
    result is empty_set()
    for key in set1.keys()
        key != '__set' ? set_add(result, key) ! none
    for key in set2.keys()
        key != '__set' ? set_add(result, key) ! none
    result

*set_intersection(set1, set2)
    result is empty_set()
    for key in set1.keys()
        key != '__set' and set_contains(set2, key) ?
            set_add(result, key)
        ! none
    result

*set_difference(set1, set2)
    result is empty_set()
    for key in set1.keys()
        key != '__set' and not set_contains(set2, key) ?
            set_add(result, key)
        ! none
    result
```

---

## 5. Text Processing

### 5.1 `std.text.string`

```coral
use std/text/string

# String info
*str_length(s)
    s.length

*is_blank(s)
    s.trim().length == 0

# Case conversion
*to_upper(s)
    s.upper()

*to_lower(s)
    s.lower()

*capitalize(s)
    s.length == 0 ? s ! s.get(0).upper() + s.slice(1)

# Trimming
*trim(s)
    s.trim()

*trim_start(s)
    s.trim_start()

*trim_end(s)
    s.trim_end()

# Splitting and joining
*split(s, delimiter)
    s.split(delimiter)

*lines(s)
    s.split('\n')

*words(s)
    s.split(' ').filter(|w| w.length > 0)

*join(list, separator)
    list.join(separator)

# Searching
*contains_str(s, substr)
    s.contains(substr)

*starts_with(s, prefix)
    s.starts_with(prefix)

*ends_with(s, suffix)
    s.ends_with(suffix)

*index_of_str(s, substr)
    s.index_of(substr)

# Replacement
*replace(s, old, new)
    s.replace(old, new)

*replace_all(s, old, new)
    s.replace_all(old, new)

# Slicing
*substring(s, start, end)
    s.slice(start, end)

*char_at(s, index)
    index >= 0 and index < s.length ? s.get(index) ! err String:IndexOutOfBounds

# Padding
*pad_start(s, length, char)
    s.length >= length ? s ! repeat_str(char, length - s.length) + s

*pad_end(s, length, char)
    s.length >= length ? s ! s + repeat_str(char, length - s.length)

*repeat_str(s, n)
    result is ''
    i is 0
    loop i < n
        result is result + s
        i is i + 1
    result

# Parsing
*parse_int(s)
    # Returns number or error
    s.parse_int()

*parse_float(s)
    # Returns number or error
    s.parse_float()
```

---

## 6. Math

### 6.1 `std.math.basic`

```coral
use std/math/basic

# Constants
PI is 3.141592653589793
E is 2.718281828459045
TAU is 6.283185307179586

# Basic functions
*abs(x)
    x < 0 ? -x ! x

*sign(x)
    x < 0 ? -1 ! (x > 0 ? 1 ! 0)

*floor(x)
    x.floor()

*ceil(x)
    x.ceil()

*round(x)
    x.round()

*trunc(x)
    x.trunc()

# Powers and roots
*pow(base, exp)
    base.pow(exp)

*sqrt(x)
    x < 0 ! err Math:NegativeSqrt
    x.sqrt()

*cbrt(x)
    x.cbrt()

# Logarithms
*log(x)
    x <= 0 ! err Math:InvalidLog
    x.ln()

*log10(x)
    x <= 0 ! err Math:InvalidLog
    x.log10()

*log2(x)
    x <= 0 ! err Math:InvalidLog
    x.log2()

*exp(x)
    E.pow(x)

# Number theory
*gcd(a, b)
    b == 0 ? abs(a) ! gcd(b, a % b)

*lcm(a, b)
    a == 0 or b == 0 ? 0 ! abs(a * b) / gcd(a, b)

*is_even(n)
    n % 2 == 0

*is_odd(n)
    n % 2 != 0

# Floating point
*is_nan(x)
    x != x

*is_infinite(x)
    x == x + 1

*is_finite(x)
    not is_nan(x) and not is_infinite(x)

# Clamping and mapping
*clamp(x, low, high)
    min(max(x, low), high)

*lerp(a, b, t)
    a + (b - a) * t

*map_range(x, in_low, in_high, out_low, out_high)
    t is (x - in_low) / (in_high - in_low)
    lerp(out_low, out_high, t)
```

### 6.2 `std.math.trig`

```coral
use std/math/trig

# Trigonometric functions
*sin(x)
    x.sin()

*cos(x)
    x.cos()

*tan(x)
    x.tan()

*asin(x)
    (x < -1 or x > 1) ! err Math:DomainError
    x.asin()

*acos(x)
    (x < -1 or x > 1) ! err Math:DomainError
    x.acos()

*atan(x)
    x.atan()

*atan2(y, x)
    y.atan2(x)

# Hyperbolic functions
*sinh(x)
    x.sinh()

*cosh(x)
    x.cosh()

*tanh(x)
    x.tanh()

# Angle conversion
*degrees(radians)
    radians * 180 / PI

*radians(degrees)
    degrees * PI / 180
```

### 6.3 `std.math.random`

```coral
use std/math/random

# Random number generation
*random()
    # Returns random float in [0, 1)
    random_float()

*random_int(min, max)
    min > max ! err Random:InvalidRange
    floor(random() * (max - min + 1)) + min

*random_float_range(min, max)
    min > max ! err Random:InvalidRange
    random() * (max - min) + min

*random_bool()
    random() < 0.5

*random_choice(list)
    is_empty(list) ! err Random:EmptyList
    list.get(random_int(0, list.length - 1))

*shuffle(list)
    result is list.clone()
    i is result.length - 1
    loop i > 0
        j is random_int(0, i)
        temp is result.get(i)
        result.set(i, result.get(j))
        result.set(j, temp)
        i is i - 1
    result

*sample(list, n)
    n > list.length ! err Random:SampleTooLarge
    shuffled is shuffle(list)
    take(shuffled, n)
```

---

## 7. I/O

### 7.1 `std.io.file`

```coral
use std/io/file

# File reading
*read_file(path)
    # Returns string content or error
    io.read_file(path)

*read_lines(path)
    content is read_file(path) ! return err
    lines(content)

*read_bytes(path)
    # Returns bytes or error
    io.read_bytes(path)

# File writing
*write_file(path, content)
    # Returns unit or error
    io.write_file(path, content)

*append_file(path, content)
    io.append_file(path, content)

*write_bytes(path, data)
    io.write_bytes(path, data)

# File info
*file_exists(path)
    io.exists(path)

*file_size(path)
    not file_exists(path) ! err File:NotFound
    io.file_size(path)

*is_file(path)
    io.is_file(path)

*is_directory(path)
    io.is_directory(path)

# File operations
*delete_file(path)
    not file_exists(path) ! err File:NotFound
    io.delete(path)

*rename_file(old_path, new_path)
    not file_exists(old_path) ! err File:NotFound
    io.rename(old_path, new_path)

*copy_file(src, dest)
    not file_exists(src) ! err File:NotFound
    io.copy(src, dest)

# Directory operations
*list_dir(path)
    not is_directory(path) ! err File:NotDirectory
    io.list_dir(path)

*make_dir(path)
    io.make_dir(path)

*make_dirs(path)
    io.make_dirs(path)
```

### 7.2 `std.io.console`

```coral
use std/io/console

# Output
*print(message)
    io.print(message)

*println(message)
    io.println(message)

*eprint(message)
    io.eprint(message)

*eprintln(message)
    io.eprintln(message)

# Input
*input()
    io.read_line()

*input_with_prompt(prompt)
    print(prompt)
    input()

# Formatting
*printf(format, args...)
    formatted is format_string(format, args)
    print(formatted)
```

---

## 8. Time

### 8.1 `std.time.datetime`

```coral
use std/time/datetime

# Current time
*now()
    time.now()

*now_utc()
    time.now_utc()

*timestamp()
    time.timestamp()

# Date/time components
*year(dt)
    dt.year

*month(dt)
    dt.month

*day(dt)
    dt.day

*hour(dt)
    dt.hour

*minute(dt)
    dt.minute

*second(dt)
    dt.second

*weekday(dt)
    dt.weekday

# Formatting
*format_datetime(dt, pattern)
    dt.format(pattern)

*format_iso(dt)
    format_datetime(dt, '%Y-%m-%dT%H:%M:%S')

# Parsing
*parse_datetime(s, pattern)
    time.parse(s, pattern)

*parse_iso(s)
    parse_datetime(s, '%Y-%m-%dT%H:%M:%S')
```

### 8.2 `std.time.duration`

```coral
use std/time/duration

# Duration creation
*seconds(n)
    { value: n, unit: 'seconds' }

*minutes(n)
    { value: n * 60, unit: 'seconds' }

*hours(n)
    { value: n * 3600, unit: 'seconds' }

*days(n)
    { value: n * 86400, unit: 'seconds' }

*milliseconds(n)
    { value: n / 1000, unit: 'seconds' }

# Duration operations
*duration_add(d1, d2)
    { value: d1.value + d2.value, unit: 'seconds' }

*duration_sub(d1, d2)
    { value: d1.value - d2.value, unit: 'seconds' }

*duration_mul(d, factor)
    { value: d.value * factor, unit: 'seconds' }

# Conversion
*to_seconds(d)
    d.value

*to_minutes(d)
    d.value / 60

*to_hours(d)
    d.value / 3600

*to_days(d)
    d.value / 86400

*to_milliseconds(d)
    d.value * 1000
```

---

## 9. JSON

### 9.1 `std.json`

```coral
use std/json

# Parsing
*parse_json(s)
    # Returns parsed value or error
    json.parse(s)

*parse_json_file(path)
    content is read_file(path) ! return err
    parse_json(content)

# Serialization
*to_json(value)
    json.stringify(value)

*to_json_pretty(value)
    json.stringify(value, 2)

# JSON path access
*json_get(obj, path)
    # Path like 'users.0.name'
    parts is path.split('.')
    current is obj
    for part in parts
        current is current.get(part)
        current.err ? none ! none
    current

*json_set(obj, path, value)
    # Immutable set, returns new object
    json.set(obj, path, value)
```

---

## 10. Encoding

### 10.1 `std.encoding.base64`

```coral
use std/encoding/base64

*base64_encode(data)
    encoding.base64_encode(data)

*base64_decode(s)
    encoding.base64_decode(s)

*base64_encode_url(data)
    # URL-safe Base64
    encoding.base64_encode_url(data)

*base64_decode_url(s)
    encoding.base64_decode_url(s)
```

### 10.2 `std.encoding.hex`

```coral
use std/encoding/hex

*hex_encode(data)
    encoding.hex_encode(data)

*hex_decode(s)
    encoding.hex_decode(s)

*to_hex_string(n)
    n.to_hex()

*from_hex_string(s)
    parse_int_base(s, 16)
```

---

## 11. Crypto

### 11.1 `std.crypto.hash`

```coral
use std/crypto/hash

*md5(data)
    crypto.md5(data)

*sha1(data)
    crypto.sha1(data)

*sha256(data)
    crypto.sha256(data)

*sha512(data)
    crypto.sha512(data)

*hmac_sha256(key, data)
    crypto.hmac_sha256(key, data)
```

### 11.2 `std.crypto.random`

```coral
use std/crypto/random

*secure_random_bytes(n)
    crypto.random_bytes(n)

*secure_random_hex(n)
    bytes is secure_random_bytes(n)
    hex_encode(bytes)

*uuid_v4()
    crypto.uuid_v4()
```

---

## 12. Runtime

### 12.1 `std.runtime.actor`

```coral
use std/runtime/actor

# Actor creation
extern fn coral_spawn(closure) 
extern fn coral_self()
extern fn coral_send(actor, message)
extern fn coral_receive(handler)

# Named actor registry
extern fn coral_register_actor(name, actor)
extern fn coral_lookup_actor(name)
extern fn coral_unregister_actor(name)

# Timers
extern fn coral_timer_send_after(delay_ms, actor, message)
extern fn coral_timer_schedule_repeat(interval_ms, actor, message)
extern fn coral_timer_cancel(timer_id)
extern fn coral_timer_pending_count()

# Wrappers
*spawn(fn)
    coral_spawn(fn)

*self()
    coral_self()

*send(actor, message)
    coral_send(actor, message)

*receive(handler)
    coral_receive(handler)

*register(name, actor)
    coral_register_actor(name, actor)

*lookup(name)
    result is coral_lookup_actor(name)
    result.err ? none ! result

*unregister(name)
    coral_unregister_actor(name)

*send_after(delay_ms, actor, message)
    coral_timer_send_after(delay_ms, actor, message)

*schedule_repeat(interval_ms, actor, message)
    coral_timer_schedule_repeat(interval_ms, actor, message)

*cancel_timer(timer_id)
    coral_timer_cancel(timer_id)

*pending_timers()
    coral_timer_pending_count()
```

### 12.2 `std.runtime.memory`

```coral
use std/runtime/memory

# Low-level memory (unsafe)
extern fn coral_malloc(size)
extern fn coral_free(ptr)
extern fn coral_realloc(ptr, new_size)

*allocate(size)
    size <= 0 ! err Memory:InvalidSize
    coral_malloc(size)

*deallocate(ptr)
    coral_free(ptr)

*reallocate(ptr, new_size)
    new_size <= 0 ! err Memory:InvalidSize
    coral_realloc(ptr, new_size)
```

### 12.3 `std.runtime.value`

```coral
use std/runtime/value

# Value introspection
extern fn coral_type_of(value)
extern fn coral_is_err(value)

*type_of(value)
    coral_type_of(value)

*is_error(value)
    value.err

*error_name(value)
    not value.err ! err Value:NotAnError
    value.error_name

*clone(value)
    value.clone()

*equals(a, b)
    a == b

*hash_code(value)
    value.hash()
```

---

## 13. Testing

### 13.1 `std.testing.assert`

```coral
use std/testing/assert

*assert(condition, message)
    not condition ! err Test:AssertionFailed
    none

*assert_eq(actual, expected)
    actual != expected ! err Test:NotEqual
    none

*assert_ne(actual, expected)
    actual == expected ! err Test:ShouldNotEqual
    none

*assert_true(value)
    assert_eq(value, true)

*assert_false(value)
    assert_eq(value, false)

*assert_error(value)
    not value.err ! err Test:ExpectedError
    none

*assert_not_error(value)
    value.err ! err Test:UnexpectedError
    none

*fail(message)
    err Test:Fail
```

---

## 14. Implementation Notes

### 14.1 Error Handling Philosophy

Coral uses error values rather than exceptions. Every operation that can fail returns either a normal value or an error value. Use `.err` to check:

```coral
result is may_fail()
result.err ?
    log('Operation failed')
    handle_error(result)
! process_success(result)
```

### 14.2 Guard Clause Pattern

Use `cond ! err Name` for precondition checking:

```coral
*process_order(order)
    order != none ! err Order:Missing
    order.items.length > 0 ! err Order:Empty
    order.customer != none ! err Order:NoCustomer
    
    # Proceed with processing
    calculate_total(order)
```

### 14.3 Error Propagation Pattern

Use `expr ! return err` to bubble up errors:

```coral
*complex_operation()
    step1 is do_first_thing() ! return err
    step2 is do_second_thing(step1) ! return err
    step3 is do_third_thing(step2) ! return err
    finalize(step3)
```

### 14.4 None vs Error

- Use `none` for expected absence (optional values)
- Use `err Name` for exceptional conditions

```coral
# none for optional
*find_user(id)
    user is db.get(id)
    user.err ? none ! user

# err for failure
*get_required_user(id)
    user is db.get(id)
    user.err ! err User:NotFound
    user
```
