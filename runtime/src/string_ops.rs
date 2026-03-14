use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_concat(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_unit();
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    if va.tag == ValueTag::String as u8 && vb.tag == ValueTag::String as u8 {
        let mut bytes = string_to_bytes(va);
        bytes.extend(string_to_bytes(vb));
        let handle = alloc_string(&bytes);
        alloc_value(Value::from_heap(ValueTag::String, handle))
    } else {
        coral_make_unit()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_slice(
    s: ValueHandle,
    start: ValueHandle,
    end: ValueHandle,
) -> ValueHandle {
    if s.is_null() || start.is_null() || end.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let start_idx = unsafe { (*start).payload.number } as usize;
    let end_idx = (unsafe { (*end).payload.number } as usize).min(bytes.len());
    if start_idx >= bytes.len() || start_idx >= end_idx {
        return coral_make_string(std::ptr::null(), 0);
    }
    let slice = &bytes[start_idx..end_idx];
    coral_make_string(slice.as_ptr(), slice.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_char_at(s: ValueHandle, index: ValueHandle) -> ValueHandle {
    if s.is_null() || index.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let idx = unsafe { (*index).payload.number } as usize;
    if idx >= bytes.len() {
        return coral_make_unit();
    }
    let byte = bytes[idx];
    coral_make_string(&byte as *const u8, 1)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_index_of(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if haystack.is_null() || needle.is_null() {
        return coral_make_number(-1.0);
    }
    let vh = unsafe { &*haystack };
    let vn = unsafe { &*needle };
    if (vh.tag != ValueTag::String as u8 && vh.tag != ValueTag::Bytes as u8)
        || (vn.tag != ValueTag::String as u8 && vn.tag != ValueTag::Bytes as u8)
    {
        return coral_make_number(-1.0);
    }
    let haystack_bytes = string_to_bytes(vh);
    let needle_bytes = string_to_bytes(vn);
    if needle_bytes.is_empty() {
        return coral_make_number(0.0);
    }
    for i in 0..=haystack_bytes.len().saturating_sub(needle_bytes.len()) {
        if haystack_bytes[i..].starts_with(&needle_bytes) {
            return coral_make_number(i as f64);
        }
    }
    coral_make_number(-1.0)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_split(s: ValueHandle, delimiter: ValueHandle) -> ValueHandle {
    if s.is_null() || delimiter.is_null() {
        return coral_make_list(std::ptr::null(), 0);
    }
    let vs = unsafe { &*s };
    let vd = unsafe { &*delimiter };
    if (vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8)
        || (vd.tag != ValueTag::String as u8 && vd.tag != ValueTag::Bytes as u8)
    {
        return coral_make_list(std::ptr::null(), 0);
    }
    let s_bytes = string_to_bytes(vs);
    let d_bytes = string_to_bytes(vd);

    let mut parts: Vec<ValueHandle> = Vec::new();

    if d_bytes.is_empty() {
        for byte in &s_bytes {
            let part = coral_make_string(byte as *const u8, 1);
            parts.push(part);
        }
    } else {
        let mut start = 0;
        let s_str = String::from_utf8_lossy(&s_bytes);
        let d_str = String::from_utf8_lossy(&d_bytes);

        for (i, _) in s_str.match_indices(&*d_str) {
            if i > start {
                let part_bytes = &s_bytes[start..i];
                let part = coral_make_string(part_bytes.as_ptr(), part_bytes.len());
                parts.push(part);
            } else if i == start {
                let part = coral_make_string(std::ptr::null(), 0);
                parts.push(part);
            }
            start = i + d_bytes.len();
        }

        if start <= s_bytes.len() {
            let part_bytes = &s_bytes[start..];
            let part = coral_make_string(part_bytes.as_ptr(), part_bytes.len());
            parts.push(part);
        }
    }

    coral_make_list(parts.as_ptr(), parts.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_to_chars(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_list(std::ptr::null(), 0);
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_list(std::ptr::null(), 0);
    }
    let bytes = string_to_bytes(vs);
    let mut chars: Vec<ValueHandle> = Vec::with_capacity(bytes.len());
    for byte in &bytes {
        let char_str = coral_make_string(byte as *const u8, 1);
        chars.push(char_str);
    }
    coral_make_list(chars.as_ptr(), chars.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_starts_with(s: ValueHandle, prefix: ValueHandle) -> ValueHandle {
    if s.is_null() || prefix.is_null() {
        return coral_make_bool(0);
    }
    let vs = unsafe { &*s };
    let vp = unsafe { &*prefix };
    if (vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8)
        || (vp.tag != ValueTag::String as u8 && vp.tag != ValueTag::Bytes as u8)
    {
        return coral_make_bool(0);
    }
    let s_bytes = string_to_bytes(vs);
    let p_bytes = string_to_bytes(vp);
    coral_make_bool(if s_bytes.starts_with(&p_bytes) { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_ends_with(s: ValueHandle, suffix: ValueHandle) -> ValueHandle {
    if s.is_null() || suffix.is_null() {
        return coral_make_bool(0);
    }
    let vs = unsafe { &*s };
    let vx = unsafe { &*suffix };
    if (vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8)
        || (vx.tag != ValueTag::String as u8 && vx.tag != ValueTag::Bytes as u8)
    {
        return coral_make_bool(0);
    }
    let s_bytes = string_to_bytes(vs);
    let x_bytes = string_to_bytes(vx);
    coral_make_bool(if s_bytes.ends_with(&x_bytes) { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_trim(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    let trimmed = s_str.trim();
    coral_make_string(trimmed.as_ptr(), trimmed.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_to_upper(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    let upper = s_str.to_uppercase();
    coral_make_string(upper.as_ptr(), upper.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_to_lower(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    let lower = s_str.to_lowercase();
    coral_make_string(lower.as_ptr(), lower.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_replace(
    s: ValueHandle,
    old: ValueHandle,
    new: ValueHandle,
) -> ValueHandle {
    if s.is_null() || old.is_null() || new.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    let vo = unsafe { &*old };
    let vn = unsafe { &*new };
    if vs.tag != ValueTag::String as u8
        || vo.tag != ValueTag::String as u8
        || vn.tag != ValueTag::String as u8
    {
        return coral_make_unit();
    }
    let s_bytes = string_to_bytes(vs);
    let o_bytes = string_to_bytes(vo);
    let n_bytes = string_to_bytes(vn);

    let s_str = String::from_utf8_lossy(&s_bytes);
    let o_str = String::from_utf8_lossy(&o_bytes);
    let n_str = String::from_utf8_lossy(&n_bytes);

    let result = s_str.replace(&*o_str, &*n_str);
    coral_make_string(result.as_ptr(), result.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_contains(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if haystack.is_null() || needle.is_null() {
        return coral_make_bool(0);
    }
    let vh = unsafe { &*haystack };
    let vn = unsafe { &*needle };
    if (vh.tag != ValueTag::String as u8 && vh.tag != ValueTag::Bytes as u8)
        || (vn.tag != ValueTag::String as u8 && vn.tag != ValueTag::Bytes as u8)
    {
        return coral_make_bool(0);
    }
    let h_bytes = string_to_bytes(vh);
    let n_bytes = string_to_bytes(vn);

    if n_bytes.is_empty() {
        return coral_make_bool(1);
    }

    let h_str = String::from_utf8_lossy(&h_bytes);
    let n_str = String::from_utf8_lossy(&n_bytes);

    coral_make_bool(if h_str.contains(&*n_str) { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_parse_number(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 && vs.tag != ValueTag::Bytes as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    match s_str.trim().parse::<f64>() {
        Ok(n) => coral_make_number(n),
        Err(_) => coral_make_unit(),
    }
}

pub(crate) fn value_to_display_string(v: &Value) -> String {
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::Number) => {
            let n = unsafe { v.payload.number };

            if n.fract() == 0.0 && n.is_finite() && n.abs() < (i64::MAX as f64) {
                format!("{}", n as i64)
            } else {
                format!("{n}")
            }
        }
        Ok(ValueTag::Bool) => {
            let byte = unsafe { v.payload.inline[0] } & 1;
            if byte != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Ok(ValueTag::String) => value_to_rust_string(v),
        Ok(ValueTag::Unit) => "()".to_string(),
        Ok(ValueTag::List) => "[list]".to_string(),
        Ok(ValueTag::Map) => "{map}".to_string(),
        Ok(ValueTag::Tagged) => {
            let ptr = v.heap_ptr();
            if !ptr.is_null() {
                let tagged = unsafe { &*(ptr as *const TaggedValue) };
                let tag_name = unsafe {
                    let slice = slice::from_raw_parts(tagged.tag_name, tagged.tag_name_len);
                    String::from_utf8_lossy(slice).to_string()
                };
                if tagged.field_count == 0 {
                    tag_name
                } else {
                    format!("{tag_name}(...)")
                }
            } else {
                "Tagged(?)".to_string()
            }
        }
        _ => "(?)".to_string(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_to_string(v: ValueHandle) -> ValueHandle {
    if v.is_null() {
        let s = "()";
        return coral_make_string(s.as_ptr(), s.len());
    }
    let val = unsafe { &*v };

    if val.tag == ValueTag::String as u8 {
        unsafe {
            coral_value_retain(v);
        }
        return v;
    }
    let s = value_to_display_string(val);
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_number_to_string(n: ValueHandle) -> ValueHandle {
    if n.is_null() {
        return coral_make_unit();
    }
    let vn = unsafe { &*n };
    if vn.tag != ValueTag::Number as u8 {
        return coral_make_unit();
    }
    let num = unsafe { vn.payload.number };
    let s = num.to_string();
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_from_number(n: ValueHandle) -> ValueHandle {
    coral_number_to_string(n)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_ord(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_number(-1.0);
    }
    let v = unsafe { &*s };
    if v.tag != ValueTag::String as u8 && v.tag != ValueTag::Bytes as u8 {
        return coral_make_number(-1.0);
    }
    let rust_str = value_to_rust_string(v);
    match rust_str.chars().next() {
        Some(c) => coral_make_number(c as u32 as f64),
        None => coral_make_number(-1.0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_chr(code: ValueHandle) -> ValueHandle {
    if code.is_null() {
        return coral_make_string(std::ptr::null(), 0);
    }
    let v = unsafe { &*code };
    if v.tag != ValueTag::Number as u8 {
        return coral_make_string(std::ptr::null(), 0);
    }
    let n = unsafe { v.payload.number } as u32;
    match char::from_u32(n) {
        Some(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            coral_make_string(s.as_ptr(), s.len())
        }
        None => coral_make_string(std::ptr::null(), 0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_compare(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_number(0.0);
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    if va.tag != ValueTag::String as u8 || vb.tag != ValueTag::String as u8 {
        return coral_make_number(0.0);
    }
    let sa = value_to_rust_string(va);
    let sb = value_to_rust_string(vb);
    let result = match sa.cmp(&sb) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    };
    coral_make_number(result)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_lines(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_list(std::ptr::null(), 0);
    }
    let s = value_to_rust_string(unsafe { &*value });
    let lines: Vec<ValueHandle> = s
        .lines()
        .map(|line| coral_make_string_from_rust(line))
        .collect();
    coral_make_list(lines.as_ptr(), lines.len())
}

struct StringBuilderObject {
    buf: Vec<u8>,
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sb_new() -> ValueHandle {
    let sb = Box::new(StringBuilderObject {
        buf: Vec::with_capacity(256),
    });
    let handle = Box::into_raw(sb) as *mut std::ffi::c_void;
    alloc_value(Value::from_heap(ValueTag::Bytes, handle))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sb_push(sb: ValueHandle, s: ValueHandle) {
    if sb.is_null() || s.is_null() {
        return;
    }
    let sb_val = unsafe { &*sb };
    let s_val = unsafe { &*s };
    if sb_val.tag != ValueTag::Bytes as u8 {
        return;
    }
    let builder = unsafe { &mut *(sb_val.payload.ptr as *mut StringBuilderObject) };
    let bytes = string_to_bytes(s_val);
    builder.buf.extend_from_slice(&bytes);
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sb_push_bytes(sb: ValueHandle, data: *const u8, len: usize) {
    if sb.is_null() || data.is_null() {
        return;
    }
    let sb_val = unsafe { &*sb };
    if sb_val.tag != ValueTag::Bytes as u8 {
        return;
    }
    let builder = unsafe { &mut *(sb_val.payload.ptr as *mut StringBuilderObject) };
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    builder.buf.extend_from_slice(slice);
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sb_finish(sb: ValueHandle) -> ValueHandle {
    if sb.is_null() {
        return coral_make_string(std::ptr::null(), 0);
    }
    let sb_val = unsafe { &*sb };
    if sb_val.tag != ValueTag::Bytes as u8 {
        return coral_make_string(std::ptr::null(), 0);
    }
    let builder = unsafe { &mut *(sb_val.payload.ptr as *mut StringBuilderObject) };
    let result = coral_make_string(builder.buf.as_ptr(), builder.buf.len());
    builder.buf.clear();
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sb_len(sb: ValueHandle) -> ValueHandle {
    if sb.is_null() {
        return coral_make_number(0.0);
    }
    let sb_val = unsafe { &*sb };
    if sb_val.tag != ValueTag::Bytes as u8 {
        return coral_make_number(0.0);
    }
    let builder = unsafe { &*(sb_val.payload.ptr as *const StringBuilderObject) };
    coral_make_number(builder.buf.len() as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_join_list(list: ValueHandle, sep: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_string(std::ptr::null(), 0);
    }
    let list_val = unsafe { &*list };
    if list_val.tag != ValueTag::List as u8 {
        return coral_make_string(std::ptr::null(), 0);
    }
    let items = unsafe { &*(list_val.payload.ptr as *const ListObject) };
    if items.items.is_empty() {
        return coral_make_string(std::ptr::null(), 0);
    }

    let sep_bytes = if !sep.is_null() {
        string_to_bytes(unsafe { &*sep })
    } else {
        Vec::new()
    };

    let mut total_len = 0usize;
    let mut parts: Vec<Vec<u8>> = Vec::with_capacity(items.items.len());
    for elem in &items.items {
        if elem.is_null() {
            parts.push(Vec::new());
            continue;
        }
        let val = unsafe { &**elem };
        let bytes = if val.tag == ValueTag::String as u8 {
            string_to_bytes(val)
        } else {
            value_to_rust_string(val).into_bytes()
        };
        total_len += bytes.len();
        parts.push(bytes);
    }
    total_len += sep_bytes.len() * parts.len().saturating_sub(1);

    let mut buf = Vec::with_capacity(total_len);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            buf.extend_from_slice(&sep_bytes);
        }
        buf.extend_from_slice(part);
    }

    coral_make_string(buf.as_ptr(), buf.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_repeat(s: ValueHandle, count: ValueHandle) -> ValueHandle {
    if s.is_null() || count.is_null() {
        return coral_make_string(std::ptr::null(), 0);
    }
    let s_val = unsafe { &*s };
    let n = unsafe { (*count).payload.number } as usize;
    if n == 0 {
        return coral_make_string(std::ptr::null(), 0);
    }

    let bytes = string_to_bytes(s_val);
    if bytes.is_empty() {
        return coral_make_string(std::ptr::null(), 0);
    }

    let total = bytes.len() * n;
    let mut buf = Vec::with_capacity(total);
    for _ in 0..n {
        buf.extend_from_slice(&bytes);
    }
    coral_make_string(buf.as_ptr(), buf.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_reverse(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_string(std::ptr::null(), 0);
    }
    let s_val = unsafe { &*s };
    let s_str = value_to_rust_string(s_val);

    let reversed: String = s_str.chars().rev().collect();
    coral_make_string(reversed.as_ptr(), reversed.len())
}
