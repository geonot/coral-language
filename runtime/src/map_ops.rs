use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_iter(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    let snapshot = map_iter_snapshot(map_obj);
    let boxed = Box::new(snapshot);
    alloc_value(Value::from_heap_with_flags(
        ValueTag::Map,
        FLAG_MAP_ITER,
        Box::into_raw(boxed) as *mut c_void,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_iter(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let v = unsafe { &*value };
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::List) => coral_list_iter(value),
        Ok(ValueTag::Map) => coral_map_iter(value),
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_iter_next(iter: ValueHandle) -> ValueHandle {
    if iter.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &mut *iter };
    if value.tag != ValueTag::Map as u8 || (value.flags & FLAG_MAP_ITER) == 0 {
        return coral_make_unit();
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let iter_obj = unsafe { &mut *(ptr as *mut MapIter) };
    while iter_obj.index < iter_obj.buckets.len() {
        let idx = iter_obj.index;
        iter_obj.index += 1;
        let bucket = &iter_obj.buckets[idx];
        if bucket.state == MapBucketState::Occupied
            && !bucket.key.is_null()
            && !bucket.value.is_null()
        {
            unsafe {
                coral_value_retain(bucket.key);
            }
            return bucket.key;
        }
    }
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_get(map: ValueHandle, key: ValueHandle) -> ValueHandle {
    if map.is_null() || key.is_null() {
        return coral_make_absent();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_absent();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_absent();
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    if let Some(bucket) = map_get_entry(map_obj, key) {
        if bucket.value.is_null() {
            return coral_make_absent();
        }
        unsafe {
            coral_value_retain(bucket.value);
        }
        bucket.value
    } else {
        coral_make_absent()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_keys(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    let mut keys: Vec<ValueHandle> = Vec::with_capacity(map_obj.len);
    for bucket in &map_obj.buckets {
        if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() {
            unsafe {
                coral_value_retain(bucket.key);
            }
            keys.push(bucket.key);
        }
    }
    let handle = coral_make_list(keys.as_ptr(), keys.len());

    unsafe {
        for key in keys {
            coral_value_release(key);
        }
    }
    handle
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_set(
    map: ValueHandle,
    key: ValueHandle,
    value: ValueHandle,
) -> ValueHandle {
    if map.is_null() || key.is_null() || value.is_null() {
        return coral_make_unit();
    }
    if is_frozen(map) {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &mut *(ptr as *mut MapObject) };
    let replaced = map_insert(map_obj, key, value);
    if let Some(old) = replaced {
        unsafe {
            coral_value_release(old);
            coral_value_retain(map);
        }
        return map;
    }
    unsafe {
        coral_value_retain(map);
    }
    map
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_length(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_number(0.0);
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_number(0.0);
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_number(0.0);
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    coral_make_number(map_obj.len as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_remove(map: ValueHandle, key: ValueHandle) -> ValueHandle {
    if map.is_null() || key.is_null() {
        return map;
    }
    let mv = unsafe { &mut *map };
    if mv.tag != ValueTag::Map as u8 {
        return map;
    }
    let ptr = mv.heap_ptr();
    if ptr.is_null() {
        return map;
    }
    let map_obj = unsafe { &mut *(ptr as *mut MapObject) };
    let hash = hash_value(key);
    let capacity = map_obj.buckets.len();
    if capacity == 0 {
        return map;
    }
    let mut idx = (hash as usize) & (capacity - 1);
    for _ in 0..capacity {
        let bucket = &mut map_obj.buckets[idx];
        match bucket.state {
            MapBucketState::Empty => return map,
            MapBucketState::Occupied => {
                if bucket.hash == hash && values_equal_handles(bucket.key, key) {
                    if !bucket.key.is_null() {
                        unsafe {
                            coral_value_release(bucket.key);
                        }
                    }
                    if !bucket.value.is_null() {
                        unsafe {
                            coral_value_release(bucket.value);
                        }
                    }
                    bucket.state = MapBucketState::Tombstone;
                    bucket.key = ptr::null_mut();
                    bucket.value = ptr::null_mut();
                    map_obj.len -= 1;
                    map_obj.tombstones += 1;
                    return map;
                }
            }
            MapBucketState::Tombstone => {}
        }
        idx = (idx + 1) & (capacity - 1);
    }
    map
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_values(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_list(ptr::null(), 0);
    }
    let mv = unsafe { &*map };
    let Some(mo) = map_from_value(mv) else {
        return coral_make_list(ptr::null(), 0);
    };
    let mut values: Vec<ValueHandle> = Vec::with_capacity(mo.len);
    for bucket in &mo.buckets {
        if bucket.state == MapBucketState::Occupied && !bucket.value.is_null() {
            unsafe {
                coral_value_retain(bucket.value);
            }
            values.push(bucket.value);
        }
    }
    let result = coral_make_list(values.as_ptr(), values.len());
    for &h in &values {
        unsafe {
            coral_value_release(h);
        }
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_entries(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_list(ptr::null(), 0);
    }
    let mv = unsafe { &*map };
    let Some(mo) = map_from_value(mv) else {
        return coral_make_list(ptr::null(), 0);
    };
    let mut entries: Vec<ValueHandle> = Vec::with_capacity(mo.len);
    for bucket in &mo.buckets {
        if bucket.state == MapBucketState::Occupied
            && !bucket.key.is_null()
            && !bucket.value.is_null()
        {
            unsafe {
                coral_value_retain(bucket.key);
                coral_value_retain(bucket.value);
            }
            let pair = [bucket.key, bucket.value];
            let entry = coral_make_list(pair.as_ptr(), 2);
            unsafe {
                coral_value_release(bucket.key);
                coral_value_release(bucket.value);
            }
            entries.push(entry);
        }
    }
    let result = coral_make_list(entries.as_ptr(), entries.len());
    for &h in &entries {
        unsafe {
            coral_value_release(h);
        }
    }
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_has_key(map: ValueHandle, key: ValueHandle) -> ValueHandle {
    if map.is_null() || key.is_null() {
        return coral_make_bool(0);
    }
    let result = coral_map_get(map, key);
    let has = !result.is_null() && unsafe { &*result }.tag != ValueTag::Unit as u8;
    if !result.is_null() {
        unsafe {
            coral_value_release(result);
        }
    }
    coral_make_bool(if has { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_merge(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    let mut result = if a.is_null() {
        coral_make_map(ptr::null(), 0)
    } else {
        let av = unsafe { &*a };
        if let Some(mo) = map_from_value(av) {
            let mut new_map = coral_make_map(ptr::null(), 0);
            for bucket in &mo.buckets {
                if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() {
                    new_map = coral_map_set(new_map, bucket.key, bucket.value);
                }
            }
            new_map
        } else {
            coral_make_map(ptr::null(), 0)
        }
    };
    if !b.is_null() {
        let bv = unsafe { &*b };
        if let Some(mo) = map_from_value(bv) {
            for bucket in &mo.buckets {
                if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() {
                    result = coral_map_set(result, bucket.key, bucket.value);
                }
            }
        }
    }
    result
}
