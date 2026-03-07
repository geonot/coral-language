# Coral — Standard Library Status

**Last updated:** March 2026  
**Total:** ~1,150 lines across 17 modules (14 in `std/`, 3 in `std/runtime/`)

## Assessment Summary

| Rating | Count | Modules |
|--------|-------|---------|
| Good (>70% complete) | 6 | option, result, char, list, math, string |
| Partial (30-70%) | 7 | prelude, map, bit, bytes, io, set, process |
| Stub (<30%) | 1 | net |
| Runtime-Facing | 3 | runtime/actor, runtime/memory, runtime/value |

**Overall stdlib completeness: ~55%** — core data structure modules are solid, I/O and system modules are mostly stubs.

---

## Per-Module Assessment

### Core Data Types

#### `option.coral` — 74 lines, 12 functions — **Good**
Pure Coral ADT implementation of `Option` (Some/None). Well-designed with:
- `some(v)`, `none()` constructors
- `is_some()`, `is_none()` predicates
- `unwrap()`, `unwrap_or(default)`, `map(f)`, `flat_map(f)`, `filter(pred)`
- `and_then(f)`, `or_else(f)`

**Missing:** `zip()`, `inspect()`, documentation examples

#### `result.coral` — 121 lines, 18 functions — **Good**
Pure Coral ADT (`enum Result` with `Ok`/`Err` variants). Comprehensive:
- `ok(v)`, `err(e)` constructors
- `is_ok()`, `is_err()` predicates
- `unwrap()`, `unwrap_or(default)`, `unwrap_err()`
- `map(f)`, `map_err(f)`, `flat_map(f)`, `and_then(f)`, `or_else(f)`
- `try_map(f, handler)`, `recover(f)`, `to_option()`

**Missing:** Error propagation operator (`?`) integration (language-level feature)

#### `list.coral` — 144 lines, 28 functions — **Good**
Most complete stdlib module. Functional operations on lists:
- `map(l,f)`, `filter(l,f)`, `reduce(l,f)`, `fold(l,init,f)`
- `for_each(l,f)`, `find(l,f)`, `any(l,f)`, `every(l,f)`
- `flatten(l)`, `flat_map(l,f)`, `zip(a,b)`, `enumerate(l)`
- `take(l,n)`, `drop(l,n)`, `slice(l,start,end)`
- `reverse(l)`, `contains(l,item)`, `index_of(l,item)`, `count(l,f)`
- `min(l)`, `max(l)`, `sum(l)`, `sort(l)`, `unique(l)`
- `group_by(l,f)`, `partition(l,f)`, `join(l,sep)`, `chunk(l,n)`

**Missing:** `zip_with()`, `scan()`, `windows()`, `intersperse()`

#### `map.coral` — 84 lines, 15 functions — **Partial**
Wrapper functions around runtime map operations:
- `create()`, `from_list(pairs)`, `get(m,k)`, `set(m,k,v)`
- `has(m,k)`, `remove(m,k)`, `size(m)`
- `keys(m)`, `values(m)`, `entries(m)`
- `merge(a,b)`, `map_values(m,f)`, `filter_entries(m,f)`, `for_each(m,f)`, `to_list(m)`

**Missing:** `map_keys()`, `flat_map()`, `group_by()`, map iteration in `for..in` loops (needs language support)

#### `set.coral` — 33 lines, 7 functions — **Partial**
Implemented using maps (keys with `true` values):
- `create()`, `add(s,v)`, `has(s,v)`, `remove(s,v)`, `size(s)`, `to_list(s)`, `union(a,b)`

**Missing:** `intersection()`, `difference()`, `symmetric_difference()`, `is_subset()`, `is_superset()`

### String/Character Processing

#### `string.coral` — 69 lines, 17 functions — **Good**
String operations wrapping runtime builtins:
- `upper(s)`, `lower(s)`, `trim(s)`, `split(s,delim)`, `join(list,delim)`
- `starts_with(s,prefix)`, `ends_with(s,suffix)`, `contains(s,sub)`
- `replace(s,old,new)`, `repeat(s,n)`, `pad_left(s,len,char)`, `pad_right(s,len,char)`
- `char_at(s,i)`, `index_of(s,sub)`, `reverse(s)`
- `is_empty(s)`, `substring(s,start,end)`

**Missing:** `chars()` (iterate characters), `lines()`, regex support (future)

#### `char.coral` — 104 lines, 19 functions — **Good**
Character classification using `ord()`/`chr()` builtins:
- `is_alpha(c)`, `is_digit(c)`, `is_alnum(c)`, `is_upper(c)`, `is_lower(c)`
- `is_whitespace(c)`, `is_hex_digit(c)`, `is_ascii(c)`, `is_printable(c)`
- `is_ident_start(c)`, `is_ident_char(c)`
- `to_upper(c)`, `to_lower(c)`, `to_digit(c)`, `from_digit(n)`
- `is_vowel(c)`, `is_consonant(c)`, `is_punctuation(c)`, `is_control(c)`

**Complete** for current needs. Used by the self-hosted lexer.

### Numeric/Binary

#### `math.coral` — 47 lines, 14 functions — **Good**
Mathematical constants and functions:
- Constants: `pi`, `half_pi`, `tau`, `e`
- Functions: `abs(x)`, `max(a,b)`, `min(a,b)`, `clamp(x,lo,hi)`
- `floor(x)`, `ceil(x)`, `round(x)`, `sign(x)`, `lerp(a,b,t)`
- Predicates: `is_positive(x)`, `is_negative(x)`, `is_zero(x)`, `is_between(x,lo,hi)`, `approx_equal(a,b,eps)`

**Missing:** `sqrt()`, `pow()`, `log()`, `sin()`/`cos()`/`tan()`, `exp()` — these need runtime FFI to libm

#### `bit.coral` — 29 lines, 8 functions — **Partial**
Bitwise operations using runtime builtins:
- `mask(v,flag)`, `set(v,flag)`, `clear(v,flag)`, `toggle(v,flag)`, `test(v,flag)`
- `shift_left(v,n)`, `shift_right(v,n)`, `count_ones(v)`

**Reasonable** for current needs. Could add `count_zeros()`, `leading_zeros()`, `trailing_zeros()`.

#### `bytes.coral` — 30 lines, 7 functions — **Partial**
Byte array operations:
- `concat(a,b)`, `from_string(s)`, `to_string(b)`, `length(b)`, `slice(b,start,end)`
- `at(b,i)`, `to_hex(b)`

**Missing:** `from_hex()`, `contains()`, `find()`, byte-level iteration

### I/O and System

#### `io.coral` — 98 lines, 18 functions — **Partial (Stubs)**
File I/O operations — most are **stub implementations** that call builtins which may not all be wired:
- `read_file(path)`, `write_file(path,content)`, `append_file(path,content)`
- `file_exists(path)`, `delete_file(path)`, `list_dir(path)`
- `create_dir(path)`, `read_lines(path)`, `write_lines(path,lines)`

Higher-level: `with_file(path,mode,fn)`, `copy_file(src,dst)`, `file_size(path)`, `file_extension(path)`

**Status:** Functions exist but underlying runtime FFI for file operations is incomplete. Only `read_file` and `write_file` may work via runtime builtins.

#### `process.coral` — 44 lines, 9 functions — **Partial (Stubs)**
Process/system operations:
- `args()`, `env_get(key)`, `env_set(key,val)`, `exit(code)`
- `exec(cmd,args)`, `cwd()`, `pid()`, `hostname()`, `os_name()`

**Status:** Most are stub implementations. `exit()` likely works via runtime. Others need FFI wiring.

#### `net.coral` — 12 lines, 2 functions — **Stub**
Networking stubs only:
- `tcp_listen(host, port)` — returns placeholder string
- `tcp_connect(host, port)` — returns placeholder string

**Not functional.** Needs complete implementation with runtime FFI for socket operations.

### Prelude

#### `prelude.coral` — 63 lines, 12 functions — **Partial**
Utility functions automatically available:
- `log_line(value)`, `identity(x)`, `constant(x)`, `compose(f,g)`
- `pipe(value, fns)`, `tap(value, f)`, `times(n, f)`, `unless(cond, f)`
- `first(list)`, `last(list)`, `is_nil(value)`, `default(value, fallback)`

**Reasonable** as a prelude. Could add `not()`, `flip()`, `curry()`.

### Runtime-Facing Modules (`std/runtime/`)

#### `runtime/actor.coral` — 85 lines
Actor system wrappers. Declares extern functions for spawn, send, named actors, timers.

#### `runtime/memory.coral` — 63 lines
Memory management wrappers. Declares extern functions for retain/release, weak refs, cycle detection.

#### `runtime/value.coral` — 53 lines
Value creation wrappers. Declares extern functions for making numbers, strings, bools, lists, maps.

**Status:** These are FFI declaration modules for the Rust runtime. They work but are thin wrappers.

---

## Path to Complete Standard Library

### Priority 1: Fix What's Broken (~20 hours)
1. Wire up missing runtime FFI for `io.coral` file operations
2. Wire up `process.coral` functions (args, env, exit)
3. Add `sqrt()`, `pow()`, `log()`, trig functions to `math.coral` via libm FFI

### Priority 2: Fill Gaps (~30 hours)
1. Set operations: intersection, difference, subset checks
2. Map iteration support (requires language-level `for key in map`)
3. String `chars()` and `lines()` iteration
4. Bytes `from_hex()`, `find()`

### Priority 3: New Modules (~40 hours)
1. `std/json.coral` — JSON parsing/serialization
2. `std/time.coral` — Date/time operations
3. `std/fmt.coral` — String formatting utilities
4. `std/sort.coral` — Sorting algorithms (if not using runtime sort)
5. `std/regex.coral` — Regular expression support (long-term, needs runtime FFI)

### Priority 4: Network Stack (~60+ hours)
1. TCP client/server via runtime FFI
2. HTTP client (minimal)
3. UDP support

**Total estimated effort to reasonably complete stdlib: ~150 hours**

---

## Testing

Currently no dedicated stdlib tests exist. The stdlib modules are exercised indirectly through:
- E2E execution tests that `use std.prelude`
- Self-hosting regression tests that `use std.char`
- Parser fixture tests that include `use` directives

**Recommended:** Add a `tests/stdlib.rs` test file that exercises each stdlib module's functions directly.
