# Cycle-Safe Programming Patterns in Coral

This document describes best practices for avoiding memory leaks caused by reference cycles in Coral programs.

## Background

Coral uses reference counting for memory management. While simple and deterministic, reference counting cannot automatically free cycles of objects that reference each other. This document explains how to write cycle-safe code.

## The Problem: Reference Cycles

A reference cycle occurs when objects reference each other directly or indirectly, preventing their reference counts from ever reaching zero:

```coral
# BAD: Creates a reference cycle
store Node
    value: Int
    next: Node?  # Can create cycles

*create_cycle()
    node1 is make_Node()
    node2 is make_Node()
    node1.next is node2
    node2.next is node1  # Cycle! Neither can be freed
```

## Solution 1: Use Weak References

Weak references don't contribute to the reference count, breaking potential cycles:

```coral
store Node
    value: Int
    next: weak Node?  # Weak reference - doesn't prevent deallocation

*safe_link(parent: Node, child: Node)
    parent.next is weak child  # Create weak reference
    # When parent is released, child can still be freed if no other refs
```

### When to Use Weak References

Use weak references for:
- Back-references (child -> parent)
- Caches
- Observer patterns
- Graph edges that shouldn't "own" their targets

### Upgrading Weak References

Before using a weak reference, upgrade it to a strong reference:

```coral
*use_weak(node: Node)
    # Try to get strong reference
    strong is node.next.upgrade()
    if strong.is_some()
        # Safe to use - we have a strong reference
        process(strong.unwrap())
    else
        # Target was deallocated
        handle_missing()
```

## Solution 2: Ownership Hierarchies

Design your data structures with clear ownership:

```coral
# GOOD: Clear ownership hierarchy - parent owns children
store TreeNode
    value: Int
    children: [TreeNode]      # Strong refs - parent owns children
    parent: weak TreeNode?    # Weak ref - child doesn't own parent

*add_child(parent: TreeNode, child: TreeNode)
    parent.children.append(child)
    child.parent is weak parent
```

## Solution 3: Explicit Cycle Breaking

For complex structures, break cycles explicitly when done:

```coral
store Graph
    nodes: [GraphNode]
    
*clear()
    # Break all internal references before dropping
    for node in self.nodes
        node.edges.clear()
    self.nodes.clear()
```

## Solution 4: Actor-Based Design

Actors naturally avoid cycles because messages are copied, not shared:

```coral
# GOOD: Actors communicate via messages, no shared references
actor NodeActor
    value: Int
    
    @process(data)
        # Process data - no shared references to worry about
        self.value is data.value
```

## Automatic Cycle Collection

Coral includes a cycle collector for cases where cycles are unavoidable:

```coral
# Trigger cycle collection manually when needed
runtime.collect_cycles()

# Or let the runtime collect periodically (automatic)
```

The cycle collector runs automatically when:
- Memory pressure is detected
- After a certain number of allocations
- When explicitly triggered

## Best Practices Summary

1. **Prefer trees over graphs** - Tree structures have natural ownership
2. **Use weak references for back-pointers** - Break parent-child cycles
3. **Clear containers before dropping** - Explicitly break internal cycles
4. **Use actors for concurrent patterns** - Message-passing avoids sharing
5. **Run cycle collection** - When using unavoidable cycles

## Performance Considerations

- Weak reference lookup has overhead (~50ns)
- Cycle collection is O(n) in potential cycle roots
- Breaking cycles manually is faster than collection

## Debugging Cycles

Use runtime introspection to detect leaks:

```coral
# Check for potential leaks
count is runtime.cycle_roots_count()
if count > threshold
    runtime.collect_cycles()
    log("Collected cycles, detected: ", runtime.cycles_detected())
```

## Common Patterns

### Observer Pattern (Weak Back-Reference)

```coral
store Observable
    observers: [weak Observer]
    
*notify()
    for obs in self.observers
        strong is obs.upgrade()
        if strong.is_some()
            strong.unwrap().update()
        # Dead observers are automatically skipped
```

### Cache Pattern (Weak Values)

```coral
store Cache
    entries: {String: weak CacheEntry}
    
*get(key: String) -> CacheEntry?
    entry is self.entries.get(key)
    if entry.is_some()
        entry.upgrade()  # Returns None if entry was evicted
    else
        None
```

### Parent-Child Pattern

```coral
store Parent
    children: [Child]
    
store Child
    parent: weak Parent
    
*make_family(parent: Parent, child: Child)
    parent.children.append(child)
    child.parent is weak parent
```

## Conclusion

While reference cycles can cause memory leaks, following these patterns ensures efficient memory management:

1. Design with ownership in mind
2. Use weak references for non-owning references  
3. Let the cycle collector handle edge cases
4. Use actors when appropriate

These patterns cover the vast majority of real-world scenarios while keeping memory management simple and predictable.
