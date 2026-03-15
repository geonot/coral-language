use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_json_parse(input: ValueHandle) -> ValueHandle {
    if input.is_null() {
        return coral_make_absent();
    }
    let s = value_to_rust_string(unsafe { &*input });
    let trimmed = s.trim();
    json_parse_value(trimmed).unwrap_or_else(|| coral_make_absent())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_json_serialize(value: ValueHandle) -> ValueHandle {
    let json = value_to_json(value);
    coral_make_string_from_rust(&json)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_json_serialize_pretty(value: ValueHandle) -> ValueHandle {
    let json = value_to_json_pretty(value, 0);
    coral_make_string_from_rust(&json)
}

fn json_parse_value(s: &str) -> Option<ValueHandle> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.as_bytes()[0] {
        b'"' => json_parse_string(s),
        b'[' => json_parse_array(s),
        b'{' => json_parse_object(s),
        b't' if s.starts_with("true") => Some(coral_make_bool(1)),
        b'f' if s.starts_with("false") => Some(coral_make_bool(0)),
        b'n' if s.starts_with("null") => Some(coral_make_unit()),
        _ => json_parse_number(s),
    }
}

fn json_parse_number(s: &str) -> Option<ValueHandle> {
    s.trim().parse::<f64>().ok().map(|n| coral_make_number(n))
}

fn json_parse_string(s: &str) -> Option<ValueHandle> {
    let inner = extract_json_string(s)?;
    Some(coral_make_string_from_rust(&inner))
}

fn extract_json_string(s: &str) -> Option<String> {
    if !s.starts_with('"') {
        return None;
    }
    let bytes = s.as_bytes();
    let mut result = String::new();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            return Some(result);
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'"' => result.push('"'),
                b'\\' => result.push('\\'),
                b'/' => result.push('/'),
                b'n' => result.push('\n'),
                b'r' => result.push('\r'),
                b't' => result.push('\t'),
                b'b' => result.push('\u{0008}'),
                b'f' => result.push('\u{000C}'),
                b'u' if i + 4 < bytes.len() => {
                    let hex = &s[i + 1..i + 5];
                    if let Ok(cp) = u32::from_str_radix(hex, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            result.push(c);
                        }
                    }
                    i += 4;
                }
                _ => {
                    result.push('\\');
                    result.push(bytes[i] as char);
                }
            }
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }
    None
}

fn skip_json_value(s: &str, start: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = start;

    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return i;
    }
    match bytes[i] {
        b'"' => {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    return i + 1;
                }
                i += 1;
            }
            i
        }
        b'[' | b'{' => {
            let (open, close) = if bytes[i] == b'[' {
                (b'[', b']')
            } else {
                (b'{', b'}')
            };
            let mut depth = 1;
            i += 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'"' {
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\\' {
                            i += 2;
                            continue;
                        }
                        if bytes[i] == b'"' {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    continue;
                }
                if bytes[i] == open {
                    depth += 1;
                }
                if bytes[i] == close {
                    depth -= 1;
                }
                i += 1;
            }
            i
        }
        _ => {
            while i < bytes.len()
                && !matches!(bytes[i], b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t')
            {
                i += 1;
            }
            i
        }
    }
}

fn json_parse_array(s: &str) -> Option<ValueHandle> {
    let bytes = s.as_bytes();
    let mut items: Vec<ValueHandle> = Vec::new();
    let mut i = 1;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b']' {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }

        let end = skip_json_value(s, i);
        let elem_str = &s[i..end];
        if let Some(val) = json_parse_value(elem_str.trim()) {
            items.push(val);
        }
        i = end;
    }
    Some(coral_make_list(items.as_ptr(), items.len()))
}

fn json_parse_object(s: &str) -> Option<ValueHandle> {
    let bytes = s.as_bytes();
    let mut entries: Vec<MapEntry> = Vec::new();
    let mut i = 1;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'}' {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }

        if bytes[i] != b'"' {
            break;
        }
        let key_str = extract_json_string(&s[i..])?;
        let key_end = skip_json_value(s, i);
        i = key_end;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b':' {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let val_end = skip_json_value(s, i);
        let val_str = &s[i..val_end];
        let key_handle = coral_make_string_from_rust(&key_str);
        let val_handle = json_parse_value(val_str.trim()).unwrap_or_else(|| coral_make_unit());
        entries.push(MapEntry {
            key: key_handle,
            value: val_handle,
        });
        i = val_end;
    }
    Some(coral_make_map(entries.as_ptr(), entries.len()))
}

fn value_to_json(value: ValueHandle) -> String {
    if value.is_null() {
        return "null".to_string();
    }
    let v = unsafe { &*value };
    if v.is_err() || v.is_absent() {
        return "null".to_string();
    }
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::Number) => {
            let n = unsafe { v.payload.number };
            if n == n.trunc() && n.abs() < 1e15 {
                format!("{}", n as i64)
            } else {
                format!("{}", n)
            }
        }
        Ok(ValueTag::Bool) => {
            if unsafe { v.payload.number } != 0.0 {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Ok(ValueTag::String) => {
            let s = value_to_rust_string(v);
            json_escape_string(&s)
        }
        Ok(ValueTag::Unit) => "null".to_string(),
        Ok(ValueTag::List) => {
            let handle = unsafe { v.payload.ptr } as *mut ListObject;
            let list = unsafe { &*handle };
            let mut parts = Vec::new();
            for &elem in &list.items {
                parts.push(value_to_json(elem));
            }
            format!("[{}]", parts.join(","))
        }
        Ok(ValueTag::Map) => {
            let handle = unsafe { v.payload.ptr } as *mut MapObject;
            let map = unsafe { &*handle };
            let mut parts = Vec::new();
            for bucket in &map.buckets {
                if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() {
                    let key_v = unsafe { &*bucket.key };
                    let key_s = value_to_rust_string(key_v);
                    parts.push(format!(
                        "{}:{}",
                        json_escape_string(&key_s),
                        value_to_json(bucket.value)
                    ));
                }
            }
            format!("{{{}}}", parts.join(","))
        }
        _ => "null".to_string(),
    }
}

fn value_to_json_pretty(value: ValueHandle, indent: usize) -> String {
    if value.is_null() {
        return "null".to_string();
    }
    let v = unsafe { &*value };
    if v.is_err() || v.is_absent() {
        return "null".to_string();
    }
    let pad = "  ".repeat(indent);
    let inner_pad = "  ".repeat(indent + 1);
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::List) => {
            let handle = unsafe { v.payload.ptr } as *mut ListObject;
            let list = unsafe { &*handle };
            if list.items.is_empty() {
                return "[]".to_string();
            }
            let mut parts = Vec::new();
            for &elem in &list.items {
                parts.push(format!(
                    "{}{}",
                    inner_pad,
                    value_to_json_pretty(elem, indent + 1)
                ));
            }
            format!("[\n{}\n{}]", parts.join(",\n"), pad)
        }
        Ok(ValueTag::Map) => {
            let handle = unsafe { v.payload.ptr } as *mut MapObject;
            let map = unsafe { &*handle };
            if map.len == 0 {
                return "{}".to_string();
            }
            let mut parts = Vec::new();
            for bucket in &map.buckets {
                if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() {
                    let key_v = unsafe { &*bucket.key };
                    let key_s = value_to_rust_string(key_v);
                    parts.push(format!(
                        "{}{}: {}",
                        inner_pad,
                        json_escape_string(&key_s),
                        value_to_json_pretty(bucket.value, indent + 1)
                    ));
                }
            }
            format!("{{\n{}\n{}}}", parts.join(",\n"), pad)
        }
        _ => value_to_json(value),
    }
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if c < '\u{0020}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
