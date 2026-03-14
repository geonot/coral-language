use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_length(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_number(0.0);
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Bytes as u8 && value_ref.tag != ValueTag::String as u8 {
        return coral_make_number(0.0);
    }
    coral_make_number(string_to_bytes(value_ref).len() as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_slice(value: ValueHandle, start: usize, len: usize) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Bytes as u8 && value_ref.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let data = string_to_bytes(value_ref);
    if start >= data.len() {
        return coral_make_bytes(ptr::null(), 0);
    }
    let end = (start + len).min(data.len());
    let slice = &data[start..end];
    let handle = alloc_bytes_obj(slice);
    alloc_value(Value::from_heap(ValueTag::Bytes, handle))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_concat(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_unit();
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    if va.tag == ValueTag::Bytes as u8 && vb.tag == ValueTag::Bytes as u8 {
        let mut bytes = string_to_bytes(va);
        bytes.extend(string_to_bytes(vb));
        let handle = alloc_bytes_obj(&bytes);
        return alloc_value(Value::from_heap(ValueTag::Bytes, handle));
    }
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_get(value: ValueHandle, index: ValueHandle) -> ValueHandle {
    if value.is_null() || index.is_null() {
        return coral_make_number(0.0);
    }
    let v = unsafe { &*value };
    let iv = unsafe { &*index };
    if v.tag != ValueTag::Bytes as u8 {
        return coral_make_number(0.0);
    }
    if iv.tag != ValueTag::Number as u8 {
        return coral_make_number(0.0);
    }
    let idx = unsafe { iv.payload.number } as usize;
    let bytes = string_to_bytes(v);
    if idx < bytes.len() {
        coral_make_number(bytes[idx] as f64)
    } else {
        coral_make_number(0.0)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_from_string(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_bytes(ptr::null(), 0);
    }
    let v = unsafe { &*s };
    if v.tag != ValueTag::String as u8 {
        return coral_make_bytes(ptr::null(), 0);
    }
    let bytes = string_to_bytes(v);
    coral_make_bytes(bytes.as_ptr(), bytes.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_to_string(b: ValueHandle) -> ValueHandle {
    if b.is_null() {
        return coral_make_string(ptr::null(), 0);
    }
    let v = unsafe { &*b };
    if v.tag != ValueTag::Bytes as u8 {
        return coral_make_string(ptr::null(), 0);
    }
    let bytes = string_to_bytes(v);

    let s = String::from_utf8_lossy(&bytes);
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_slice_val(
    value: ValueHandle,
    start: ValueHandle,
    end: ValueHandle,
) -> ValueHandle {
    if value.is_null() {
        return coral_make_bytes(ptr::null(), 0);
    }
    let v = unsafe { &*value };
    if v.tag != ValueTag::Bytes as u8 {
        return coral_make_bytes(ptr::null(), 0);
    }
    let bytes = string_to_bytes(v);
    let len = bytes.len();
    let s = if start.is_null() {
        0
    } else {
        let sv = unsafe { &*start };
        if sv.tag == ValueTag::Number as u8 {
            unsafe { sv.payload.number }.max(0.0) as usize
        } else {
            0
        }
    };
    let e = if end.is_null() {
        len
    } else {
        let ev = unsafe { &*end };
        if ev.tag == ValueTag::Number as u8 {
            (unsafe { ev.payload.number } as usize).min(len)
        } else {
            len
        }
    };
    if s >= e || s >= len {
        return coral_make_bytes(ptr::null(), 0);
    }
    coral_make_bytes(bytes[s..e.min(len)].as_ptr(), e.min(len) - s)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_from_hex(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_bytes(ptr::null(), 0);
    }
    let v = unsafe { &*value };
    let s = value_to_rust_string(v);
    let mut result = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return coral_make_absent();
    }
    let mut i = 0;
    while i < bytes.len() {
        let hi = match hex_val(bytes[i]) {
            Some(v) => v,
            None => return coral_make_absent(),
        };
        let lo = match hex_val(bytes[i + 1]) {
            Some(v) => v,
            None => return coral_make_absent(),
        };
        result.push((hi << 4) | lo);
        i += 2;
    }
    coral_bytes_from_vec(result)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_contains(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if haystack.is_null() || needle.is_null() {
        return coral_make_bool(0);
    }
    let hv = unsafe { &*haystack };
    let nv = unsafe { &*needle };
    let h = string_to_bytes(hv);
    let n = string_to_bytes(nv);
    if n.is_empty() {
        return coral_make_bool(1);
    }
    for window in h.windows(n.len()) {
        if window == n.as_slice() {
            return coral_make_bool(1);
        }
    }
    coral_make_bool(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_find(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if haystack.is_null() || needle.is_null() {
        return coral_make_number(-1.0);
    }
    let hv = unsafe { &*haystack };
    let nv = unsafe { &*needle };
    let h = string_to_bytes(hv);
    let n = string_to_bytes(nv);
    if n.is_empty() {
        return coral_make_number(0.0);
    }
    for (i, window) in h.windows(n.len()).enumerate() {
        if window == n.as_slice() {
            return coral_make_number(i as f64);
        }
    }
    coral_make_number(-1.0)
}
