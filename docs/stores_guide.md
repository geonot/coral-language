# Coral Stores Guide

_Last updated: 2025-01-XX_

## Overview

Stores are Coral's mechanism for defining structured data types with fields and methods. They provide:
- Named fields with default values
- Reference fields for managed object references
- Methods with automatic `self` parameter
- Map-based implementation with type identification

## Basic Store Definition

```coral
store Person
    name is "Unknown"
    age is 0
```

This creates:
- A constructor function `make_Person()` that returns a Map with:
  - `__type__` field set to "Person"
  - `name` field defaulting to "Unknown"
  - `age` field defaulting to 0

## Creating Instances

```coral
p is make_Person()
log(p.name)  // "Unknown"
log(p.age)   // 0
```

## Field Access

Access fields using dot notation:

```coral
name is p.name
age is p.age
```

Field access compiles to `coral_map_get(instance, "field_name")`.

## Store Methods

Define methods using `*method_name(params)`:

```coral
store Counter
    count is 0
    
    *increment()
        self.count is self.count + 1
    
    *set_count(n)
        self.count is n
    
    *get_count()
        self.count
```

Methods:
- Take `self` as hidden first parameter (the store instance)
- Can use `self.field is value` syntax for assignment
- Support accessing fields via `self.field`

Using methods:

```coral
c is make_Counter()
c.increment()
c.set_count(10)
log(c.get_count())  // 10
```

## Reference Fields

Reference fields manage object references with automatic memory management:

```coral
store Node
    value is 0
    &next    // Reference field (defaults to null/unit)
    
    *set_next(n)
        self.next is n
```

Reference fields (`&field`):
- Default to unit (null) instead of 0
- Automatically retain new values when assigned
- Automatically release old values when replaced
- Use for storing references to other stores or objects

Example usage:

```coral
n1 is make_Node()
n2 is make_Node()
n2.set_value(10)
n1.set_next(n2)
log(n1.next.value)  // 10
```

## Implementation Details

### Constructor
Generated LLVM function `make_StoreName()`:
- Creates empty map with `coral_make_map(null, 0)`
- Sets `__type__` field to store name
- Initializes each field with default value
- Retains default values for reference fields with defaults
- Returns the map pointer

### Method Signature
Store methods compile to:
```llvm
define double @StoreName_methodName(ptr %self, ptr %param1, ...) {
```

All parameters are `CoralValue*` pointers (not `f64`), allowing stores and other objects to be passed without corruption.

### Field Assignment in Methods
`self.field is value` compiles to:
- For value fields: `coral_map_set(self, "field", value)`
- For reference fields:
  1. Get old value: `old = coral_map_get(self, "field")`
  2. Retain new value: `coral_value_retain(value)`
  3. Set field: `coral_map_set(self, "field", value)`
  4. Release old value: `coral_value_release(old)`

## Known Limitations (Alpha)

1. **Assignment Syntax:** Only `self.field is value` in methods works. General `instance.field is value` outside methods requires explicit `.set()` call.

2. **No Type Checking:** Reference fields can be assigned any value (no compile-time type safety).

3. **Method Returns:** All methods return `f64` (even though parameters are `ptr`).

4. **No Circular Detection:** No detection or prevention of circular references.

5. **No Weak References:** All references are strong; no weak reference support.

6. **Default Value Retention:** Reference field defaults aren't retained during initialization (only explicit assignments trigger retain).

## Patterns

### Linked List

```coral
store Node
    value is 0
    &next
    
    *append(n)
        current is self
        // Traverse to end (would need iteration support)
        current.next is n

head is make_Node()
n2 is make_Node()
head.append(n2)
```

### Binary Tree

```coral
store TreeNode
    value is 0
    &left
    &right
    
    *insert_left(n)
        self.left is n
    
    *insert_right(n)
        self.right is n

root is make_TreeNode()
left is make_TreeNode()
right is make_TreeNode()
root.insert_left(left)
root.insert_right(right)
```

## Best Practices

1. **Use reference fields for object references:** Always use `&field` for fields that will hold other stores or managed objects.

2. **Methods for mutations:** Use methods to modify fields, especially reference fields, to ensure proper retain/release.

3. **Default values:** Provide sensible defaults for value fields; reference fields default to null automatically.

4. **Type field:** The `__type__` field enables future runtime type checking and dispatch.

## See Also

- `docs/alpha_overview.md` - Current language status
- `docs/known_limitations.md` - Complete list of alpha limitations
- `tests/fixtures/programs/store_*.coral` - Example programs
