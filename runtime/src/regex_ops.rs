//! Regex operations FFI functions for the Coral runtime (L2.2).

use crate::*;
use regex::Regex;

/// `coral_regex_match(pattern, text)` → Bool (true if the entire text matches the pattern)
#[unsafe(no_mangle)]
pub extern "C" fn coral_regex_match(pattern: ValueHandle, text: ValueHandle) -> ValueHandle {
    if pattern.is_null() || text.is_null() {
        return coral_make_bool(0);
    }
    let pat_str = value_to_rust_string(unsafe { &*pattern });
    let txt_str = value_to_rust_string(unsafe { &*text });
    match Regex::new(&pat_str) {
        Ok(re) => {
            // Full match: anchor the pattern
            let full = format!("^(?:{})$", pat_str);
            match Regex::new(&full) {
                Ok(re_full) => coral_make_bool(if re_full.is_match(&txt_str) { 1 } else { 0 }),
                Err(_) => coral_make_bool(if re.is_match(&txt_str) { 1 } else { 0 }),
            }
        }
        Err(_) => coral_make_bool(0),
    }
}

/// `coral_regex_find(pattern, text)` → String (first match) or unit
#[unsafe(no_mangle)]
pub extern "C" fn coral_regex_find(pattern: ValueHandle, text: ValueHandle) -> ValueHandle {
    if pattern.is_null() || text.is_null() {
        return coral_make_unit();
    }
    let pat_str = value_to_rust_string(unsafe { &*pattern });
    let txt_str = value_to_rust_string(unsafe { &*text });
    match Regex::new(&pat_str) {
        Ok(re) => {
            if let Some(m) = re.find(&txt_str) {
                let bytes = m.as_str().as_bytes();
                let handle = alloc_string(bytes);
                alloc_value(Value::from_heap(ValueTag::String, handle))
            } else {
                coral_make_unit()
            }
        }
        Err(_) => coral_make_unit(),
    }
}

/// `coral_regex_find_all(pattern, text)` → List of matched strings
#[unsafe(no_mangle)]
pub extern "C" fn coral_regex_find_all(pattern: ValueHandle, text: ValueHandle) -> ValueHandle {
    if pattern.is_null() || text.is_null() {
        return coral_make_list(std::ptr::null_mut(), 0);
    }
    let pat_str = value_to_rust_string(unsafe { &*pattern });
    let txt_str = value_to_rust_string(unsafe { &*text });
    match Regex::new(&pat_str) {
        Ok(re) => {
            let matches: Vec<ValueHandle> = re.find_iter(&txt_str)
                .map(|m| {
                    let bytes = m.as_str().as_bytes();
                    let handle = alloc_string(bytes);
                    alloc_value(Value::from_heap(ValueTag::String, handle))
                })
                .collect();
            if matches.is_empty() {
                coral_make_list(std::ptr::null_mut(), 0)
            } else {
                coral_make_list(matches.as_ptr() as *mut ValueHandle, matches.len())
            }
        }
        Err(_) => coral_make_list(std::ptr::null_mut(), 0),
    }
}

/// `coral_regex_replace(pattern, replacement, text)` → String with all matches replaced
#[unsafe(no_mangle)]
pub extern "C" fn coral_regex_replace(pattern: ValueHandle, replacement: ValueHandle, text: ValueHandle) -> ValueHandle {
    if pattern.is_null() || replacement.is_null() || text.is_null() {
        return coral_make_unit();
    }
    let pat_str = value_to_rust_string(unsafe { &*pattern });
    let rep_str = value_to_rust_string(unsafe { &*replacement });
    let txt_str = value_to_rust_string(unsafe { &*text });
    match Regex::new(&pat_str) {
        Ok(re) => {
            let result = re.replace_all(&txt_str, rep_str.as_str());
            let bytes = result.as_bytes();
            let handle = alloc_string(bytes);
            alloc_value(Value::from_heap(ValueTag::String, handle))
        }
        Err(_) => {
            // Return original text on bad pattern
            let bytes = txt_str.as_bytes();
            let handle = alloc_string(bytes);
            alloc_value(Value::from_heap(ValueTag::String, handle))
        }
    }
}

/// `coral_regex_split(pattern, text)` → List of strings
#[unsafe(no_mangle)]
pub extern "C" fn coral_regex_split(pattern: ValueHandle, text: ValueHandle) -> ValueHandle {
    if pattern.is_null() || text.is_null() {
        return coral_make_list(std::ptr::null_mut(), 0);
    }
    let pat_str = value_to_rust_string(unsafe { &*pattern });
    let txt_str = value_to_rust_string(unsafe { &*text });
    match Regex::new(&pat_str) {
        Ok(re) => {
            let parts: Vec<ValueHandle> = re.split(&txt_str)
                .map(|s| {
                    let bytes = s.as_bytes();
                    let handle = alloc_string(bytes);
                    alloc_value(Value::from_heap(ValueTag::String, handle))
                })
                .collect();
            if parts.is_empty() {
                coral_make_list(std::ptr::null_mut(), 0)
            } else {
                coral_make_list(parts.as_ptr() as *mut ValueHandle, parts.len())
            }
        }
        Err(_) => coral_make_list(std::ptr::null_mut(), 0),
    }
}

/// Helper: extract a Rust String from a Value (duplicated here for self-containment)
fn value_to_rust_string(v: &Value) -> String {
    if v.tag == ValueTag::String as u8 {
        crate::string_to_bytes(v).iter().map(|&b| b as char).collect()
    } else {
        String::new()
    }
}
