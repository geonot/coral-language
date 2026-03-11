//! List operations FFI functions for the Coral runtime.

use crate::*;

/// M4.3: Copy-on-write — if the list is marked COW, clone the backing store
/// before mutation. Clears the COW flag after copying.
fn cow_ensure_unique(list: ValueHandle) {
    if list.is_null() { return; }
    let value = unsafe { &mut *list };
    if value.flags & FLAG_COW == 0 { return; }
    // Copy the list's backing store
    if value.tag == ValueTag::List as u8 {
        let old_ptr = value.heap_ptr();
        if !old_ptr.is_null() {
            let old_list = unsafe { &*(old_ptr as *const ListObject) };
            let new_items: Vec<ValueHandle> = old_list.items.iter().copied().collect();
            for item in &new_items {
                unsafe { coral_value_retain(*item); }
            }
            let new_list = Box::new(ListObject { items: new_items });
            let new_ptr = Box::into_raw(new_list) as *mut std::ffi::c_void;
            value.payload.ptr = new_ptr;
        }
    }
    // Clear COW flag — this value now owns its backing store
    value.flags &= !FLAG_COW;
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_iter(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut items: Vec<ValueHandle> = Vec::with_capacity(list_obj.items.len());
    for &h in &list_obj.items {
        if !h.is_null() {
            unsafe { coral_value_retain(h); }
        }
        items.push(h);
    }
    let iter = Box::new(ListIter { items, index: 0 });
    alloc_value(Value::from_heap_with_flags(
        ValueTag::List,
        FLAG_LIST_ITER,
        Box::into_raw(iter) as *mut c_void,
    ))
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_iter_next(iter: ValueHandle) -> ValueHandle {
    if iter.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &mut *iter };
    if value.tag != ValueTag::List as u8 || (value.flags & FLAG_LIST_ITER) == 0 {
        return coral_make_unit();
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let iter_obj = unsafe { &mut *(ptr as *mut ListIter) };
    if iter_obj.index >= iter_obj.items.len() {
        return coral_make_unit();
    }
    let handle = iter_obj.items[iter_obj.index];
    iter_obj.index += 1;
    if handle.is_null() {
        return coral_make_unit();
    }
    unsafe { coral_value_retain(handle); }
    handle
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_map(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut results: Vec<ValueHandle> = Vec::with_capacity(list_obj.items.len());
    for &item in &list_obj.items {
        let args = [item];
        let mapped = coral_closure_invoke(func, args.as_ptr(), args.len());
        results.push(mapped);
    }
    let out = coral_make_list(results.as_ptr(), results.len());
    unsafe {
        for h in results {
            coral_value_release(h);
        }
    }
    out
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_filter(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut kept: Vec<ValueHandle> = Vec::new();
    for &item in &list_obj.items {
        let args = [item];
        let predicate = coral_closure_invoke(func, args.as_ptr(), args.len());
        let truthy = coral_value_as_bool(predicate) != 0;
        unsafe { coral_value_release(predicate); }
        if truthy && !item.is_null() {
            unsafe { coral_value_retain(item); }
            kept.push(item);
        }
    }
    let out = coral_make_list(kept.as_ptr(), kept.len());
    unsafe {
        for h in kept {
            coral_value_release(h);
        }
    }
    out
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_reduce(list: ValueHandle, seed: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut iter = list_obj.items.iter();
    let mut acc = if !seed.is_null() {
        unsafe { coral_value_retain(seed); }
        seed
    } else {
        match iter.next() {
            Some(first) if !first.is_null() => {
                unsafe { coral_value_retain(*first); }
                *first
            }
            _ => return coral_make_unit(),
        }
    };
    for &item in iter {
        let args = [acc, item];
        let next = coral_closure_invoke(func, args.as_ptr(), args.len());
        unsafe { coral_value_release(acc); }
        acc = next;
    }
    acc
}

/// C3.2: list.find(predicate) — return first element matching predicate, or none.
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_find(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_absent();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_absent();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_absent();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_absent();
    };
    for &item in &list_obj.items {
        let args = [item];
        let result = coral_closure_invoke(func, args.as_ptr(), args.len());
        let truthy = coral_value_as_bool(result) != 0;
        unsafe { coral_value_release(result); }
        if truthy {
            if !item.is_null() {
                unsafe { coral_value_retain(item); }
            }
            return item;
        }
    }
    coral_make_absent()
}

/// C3.2: list.any(predicate) — return true if any element matches predicate.
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_any(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_bool(0);
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_bool(0);
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_bool(0);
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_bool(0);
    };
    for &item in &list_obj.items {
        let args = [item];
        let result = coral_closure_invoke(func, args.as_ptr(), args.len());
        let truthy = coral_value_as_bool(result) != 0;
        unsafe { coral_value_release(result); }
        if truthy {
            return coral_make_bool(1);
        }
    }
    coral_make_bool(0)
}

/// C3.2: list.all(predicate) — return true if all elements match predicate.
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_all(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_bool(1);
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_bool(1);
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_bool(1);
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_bool(1);
    };
    for &item in &list_obj.items {
        let args = [item];
        let result = coral_closure_invoke(func, args.as_ptr(), args.len());
        let truthy = coral_value_as_bool(result) != 0;
        unsafe { coral_value_release(result); }
        if truthy == false {
            return coral_make_bool(0);
        }
    }
    coral_make_bool(1)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_push(list: ValueHandle, value: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    if is_frozen(list) {
        return coral_make_unit();
    }
    // M4.3: Copy-on-write — clone backing store if shared
    cow_ensure_unique(list);
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let ptr = list_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let list_obj = unsafe { &mut *(ptr as *mut ListObject) };
    if !value.is_null() {
        unsafe {
            coral_value_retain(value);
        }
        list_obj.items.push(value);
    }
    unsafe {
        coral_value_retain(list);
    }
    list
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_length(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_number(0.0);
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_number(0.0);
    }
    let len = list_from_value(list_value)
        .map(|obj| obj.items.len())
        .unwrap_or(0);
    coral_make_number(len as f64)
}


/// Raw list length as usize (used by store FFI internally).
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_len(list: ValueHandle) -> usize {
    if list.is_null() {
        return 0;
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return 0;
    }
    list_from_value(list_value)
        .map(|obj| obj.items.len())
        .unwrap_or(0)
}


/// Index-based list get (usize index, used by store FFI internally).
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_get_index(list: ValueHandle, index: usize) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let list_obj = match list_from_value(list_value) {
        Some(obj) => obj,
        None => return coral_make_unit(),
    };
    let handle = match list_obj.items.get(index) {
        Some(handle) => *handle,
        None => return coral_make_unit(),
    };
    if handle.is_null() {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handle);
    }
    handle
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_get(list: ValueHandle, index: ValueHandle) -> ValueHandle {
    if list.is_null() || index.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let index_value = unsafe { &*index };
    if index_value.tag != ValueTag::Number as u8 {
        return coral_make_unit();
    }
    let raw_index = unsafe { index_value.payload.number };
    if !raw_index.is_finite() {
        return coral_make_unit();
    }
    let idx = raw_index as isize;
    if raw_index.fract().abs() > f64::EPSILON || idx < 0 {
        return coral_make_unit();
    }
    let list_obj = match list_from_value(list_value) {
        Some(obj) => obj,
        None => return coral_make_unit(),
    };
    let handle = match list_obj.items.get(idx as usize) {
        Some(handle) => *handle,
        None => return coral_make_unit(),
    };
    if handle.is_null() {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handle);
    }
    handle
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_pop(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let ptr = list_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let list_obj = unsafe { &mut *(ptr as *mut ListObject) };
    match list_obj.items.pop() {
        Some(handle) if !handle.is_null() => unsafe {
            // Transfer ownership: retain for the caller, release the list's hold.
            coral_value_retain(handle);
            coral_value_release(handle);
            handle
        },
        _ => coral_make_unit(),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_iter_next(iter: ValueHandle) -> ValueHandle {
    if iter.is_null() {
        return coral_make_unit();
    }
    let v = unsafe { &*iter };
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::List) if (v.flags & FLAG_LIST_ITER) != 0 => coral_list_iter_next(iter),
        Ok(ValueTag::Map) if (v.flags & FLAG_MAP_ITER) != 0 => coral_map_iter_next(iter),
        _ => coral_make_unit(),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_contains(list: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if list.is_null() { return coral_make_bool(0); }
    let v = unsafe { &*list };
    let Some(lo) = list_from_value(v) else { return coral_make_bool(0); };
    for &item in &lo.items {
        if values_equal_handles(item, needle) {
            return coral_make_bool(1);
        }
    }
    coral_make_bool(0)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_index_of(list: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if list.is_null() { return coral_make_number(-1.0); }
    let v = unsafe { &*list };
    let Some(lo) = list_from_value(v) else { return coral_make_number(-1.0); };
    for (i, &item) in lo.items.iter().enumerate() {
        if values_equal_handles(item, needle) {
            return coral_make_number(i as f64);
        }
    }
    coral_make_number(-1.0)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_reverse(list: ValueHandle) -> ValueHandle {
    if list.is_null() { return coral_make_list(ptr::null(), 0); }
    let v = unsafe { &*list };
    let Some(lo) = list_from_value(v) else { return coral_make_list(ptr::null(), 0); };
    let items: Vec<ValueHandle> = lo.items.iter().rev().copied().collect();
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_retain(h); } }
    }
    let result = coral_make_list(items.as_ptr(), items.len());
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_release(h); } }
    }
    result
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_slice(list: ValueHandle, start: ValueHandle, end: ValueHandle) -> ValueHandle {
    if list.is_null() { return coral_make_list(ptr::null(), 0); }
    let v = unsafe { &*list };
    let Some(lo) = list_from_value(v) else { return coral_make_list(ptr::null(), 0); };
    let len = lo.items.len();
    let s = if start.is_null() { 0 } else {
        let sv = unsafe { &*start };
        if sv.tag == ValueTag::Number as u8 { unsafe { sv.payload.number }.max(0.0) as usize } else { 0 }
    };
    let e = if end.is_null() { len } else {
        let ev = unsafe { &*end };
        if ev.tag == ValueTag::Number as u8 { (unsafe { ev.payload.number } as usize).min(len) } else { len }
    };
    if s >= e || s >= len {
        return coral_make_list(ptr::null(), 0);
    }
    let slice = &lo.items[s..e.min(len)];
    for &h in slice {
        if !h.is_null() { unsafe { coral_value_retain(h); } }
    }
    let result = coral_make_list(slice.as_ptr(), slice.len());
    for &h in slice {
        if !h.is_null() { unsafe { coral_value_release(h); } }
    }
    result
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_sort(list: ValueHandle) -> ValueHandle {
    if list.is_null() { return coral_make_list(ptr::null(), 0); }
    let v = unsafe { &*list };
    let Some(lo) = list_from_value(v) else { return coral_make_list(ptr::null(), 0); };
    let mut items: Vec<ValueHandle> = lo.items.clone();
    // Sort by numeric value, with non-numbers sorted to end
    items.sort_by(|a, b| {
        let va = if a.is_null() { f64::INFINITY } else {
            let av = unsafe { &**a };
            if av.tag == ValueTag::Number as u8 { unsafe { av.payload.number } } else { f64::INFINITY }
        };
        let vb = if b.is_null() { f64::INFINITY } else {
            let bv = unsafe { &**b };
            if bv.tag == ValueTag::Number as u8 { unsafe { bv.payload.number } }
            else if bv.tag == ValueTag::String as u8 {
                // If both are strings, compare lexicographically
                f64::INFINITY
            }
            else { f64::INFINITY }
        };
        // If both are strings, compare as strings
        let a_is_str = !a.is_null() && unsafe { &**a }.tag == ValueTag::String as u8;
        let b_is_str = !b.is_null() && unsafe { &**b }.tag == ValueTag::String as u8;
        if a_is_str && b_is_str {
            let sa = value_to_rust_string(unsafe { &**a });
            let sb = value_to_rust_string(unsafe { &**b });
            return sa.cmp(&sb);
        }
        va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
    });
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_retain(h); } }
    }
    let result = coral_make_list(items.as_ptr(), items.len());
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_release(h); } }
    }
    result
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_join(list: ValueHandle, sep: ValueHandle) -> ValueHandle {
    if list.is_null() { return coral_make_string(ptr::null(), 0); }
    let v = unsafe { &*list };
    let Some(lo) = list_from_value(v) else { return coral_make_string(ptr::null(), 0); };
    let separator = if sep.is_null() { String::new() } else {
        let sv = unsafe { &*sep };
        if sv.tag == ValueTag::String as u8 { value_to_rust_string(sv) } else { String::new() }
    };
    let mut parts: Vec<String> = Vec::with_capacity(lo.items.len());
    for &item in &lo.items {
        if item.is_null() {
            parts.push("none".to_string());
        } else {
            let iv = unsafe { &*item };
            parts.push(value_to_display_string(iv));
        }
    }
    let joined = parts.join(&separator);
    coral_make_string(joined.as_ptr(), joined.len())
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_list_concat(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    let mut items: Vec<ValueHandle> = Vec::new();
    if !a.is_null() {
        let av = unsafe { &*a };
        if let Some(la) = list_from_value(av) {
            for &h in &la.items {
                if !h.is_null() { unsafe { coral_value_retain(h); } }
                items.push(h);
            }
        }
    }
    if !b.is_null() {
        let bv = unsafe { &*b };
        if let Some(lb) = list_from_value(bv) {
            for &h in &lb.items {
                if !h.is_null() { unsafe { coral_value_retain(h); } }
                items.push(h);
            }
        }
    }
    let result = coral_make_list(items.as_ptr(), items.len());
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_release(h); } }
    }
    result
}

/// Sort a list using natural ordering (numbers numerically, strings lexicographically).
/// Returns a new sorted list.
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_sort_natural(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_list(std::ptr::null(), 0);
    }
    let v = unsafe { &*list };
    let lo = match list_from_value(v) {
        Some(lo) => lo,
        None => return coral_make_list(std::ptr::null(), 0),
    };
    let mut items: Vec<ValueHandle> = lo.items.clone();
    items.sort_by(|a, b| compare_values(*a, *b));
    // Retain all items for the new list
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_retain(h); } }
    }
    let result = coral_make_list(items.as_ptr(), items.len());
    // Release our retained copies
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_release(h); } }
    }
    result
}

/// Compare two values for natural ordering.
fn compare_values(a: ValueHandle, b: ValueHandle) -> std::cmp::Ordering {
    if a.is_null() && b.is_null() { return std::cmp::Ordering::Equal; }
    if a.is_null() { return std::cmp::Ordering::Less; }
    if b.is_null() { return std::cmp::Ordering::Greater; }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    // Both numbers?
    if va.tag == ValueTag::Number as u8 && vb.tag == ValueTag::Number as u8 {
        let na = unsafe { va.payload.number };
        let nb = unsafe { vb.payload.number };
        return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
    }
    // Both strings?
    if va.tag == ValueTag::String as u8 && vb.tag == ValueTag::String as u8 {
        let sa = value_to_rust_string(va);
        let sb = value_to_rust_string(vb);
        return sa.cmp(&sb);
    }
    // Mixed: compare as strings
    let sa = value_to_rust_string(va);
    let sb = value_to_rust_string(vb);
    sa.cmp(&sb)
}

/// M4.3: Mark a value as copy-on-write (shared backing store).
/// The next mutating operation (push, pop, set) will clone before writing.
#[unsafe(no_mangle)]
pub extern "C" fn coral_value_mark_cow(handle: ValueHandle) {
    if !handle.is_null() {
        unsafe { (*handle).flags |= FLAG_COW; }
    }
}

/// M4.3: Create a COW copy of a list — both the original and the copy share
/// the same backing store until one of them is mutated.
#[unsafe(no_mangle)]
pub extern "C" fn coral_list_cow_copy(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return list;
    }
    let value = unsafe { &*list };
    if value.tag != ValueTag::List as u8 {
        return list;
    }
    // Mark the original as COW
    unsafe { (*list).flags |= FLAG_COW; }
    // Create a new Value pointing to the same ListObject
    let new_val = Value {
        tag: ValueTag::List as u8,
        flags: FLAG_COW,
        epoch: 0,
        owner_thread: current_thread_id(),
        refcount: AtomicU64::new(1),
        #[cfg(feature = "metrics")]
        retain_events: AtomicU32::new(0),
        #[cfg(feature = "metrics")]
        release_events: AtomicU32::new(0),
        payload: Payload { ptr: value.heap_ptr() },
    };
    // Retain all items (they're now referenced by two list objects)
    if let Some(list_obj) = list_from_value(value) {
        for item in &list_obj.items {
            unsafe { coral_value_retain(*item); }
        }
    }
    alloc_value(new_val)
}

/// Build a list of numbers from `start` (inclusive) to `end` (exclusive).
/// Both arguments are CoralValue Numbers.
#[unsafe(no_mangle)]
pub extern "C" fn coral_range(start: ValueHandle, end: ValueHandle) -> ValueHandle {
    let s = if start.is_null() { 0i64 } else {
        let sv = unsafe { &*start };
        if sv.tag == ValueTag::Number as u8 { (unsafe { sv.payload.number }) as i64 } else { 0 }
    };
    let e = if end.is_null() { 0i64 } else {
        let ev = unsafe { &*end };
        if ev.tag == ValueTag::Number as u8 { (unsafe { ev.payload.number }) as i64 } else { 0 }
    };
    if s >= e {
        return coral_make_list(ptr::null(), 0);
    }
    let count = (e - s) as usize;
    let mut items: Vec<ValueHandle> = Vec::with_capacity(count);
    for i in s..e {
        items.push(coral_make_number(i as f64));
    }
    let result = coral_make_list(items.as_ptr(), items.len());
    for &h in &items {
        if !h.is_null() { unsafe { coral_value_release(h); } }
    }
    result
}