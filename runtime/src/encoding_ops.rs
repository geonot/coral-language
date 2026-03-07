//! Encoding FFI functions for the Coral runtime: base64 and hex.

use crate::*;

// ============================================================================
// Base64
// ============================================================================

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[unsafe(no_mangle)]
pub extern "C" fn coral_base64_encode(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_string_from_rust("");
    }
    let input = value_to_rust_string(unsafe { &*value });
    let bytes = input.as_bytes();
    let encoded = base64_encode_bytes(bytes);
    coral_make_string_from_rust(&encoded)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_base64_decode(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_absent();
    }
    let input = value_to_rust_string(unsafe { &*value });
    match base64_decode_bytes(input.as_bytes()) {
        Some(decoded) => {
            // Return as Bytes value
            coral_bytes_from_vec(decoded)
        }
        None => coral_make_absent(),
    }
}

fn base64_encode_bytes(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 2 < data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i+1] as u32) << 8) | (data[i+2] as u32);
        result.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((n >> 6) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let remaining = data.len() - i;
    if remaining == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i+1] as u32) << 8);
        result.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((n >> 6) & 0x3F) as usize] as char);
        result.push('=');
    } else if remaining == 1 {
        let n = (data[i] as u32) << 16;
        result.push(BASE64_CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((n >> 12) & 0x3F) as usize] as char);
        result.push('=');
        result.push('=');
    }
    result
}

fn base64_decode_byte(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn base64_decode_bytes(data: &[u8]) -> Option<Vec<u8>> {
    // Filter whitespace
    let filtered: Vec<u8> = data.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    if filtered.len() % 4 != 0 {
        return None;
    }
    let mut result = Vec::with_capacity(filtered.len() / 4 * 3);
    let mut i = 0;
    while i < filtered.len() {
        let a = base64_decode_byte(filtered[i])?;
        let b = base64_decode_byte(filtered[i+1])?;
        result.push((a << 2) | (b >> 4));
        if filtered[i+2] != b'=' {
            let c = base64_decode_byte(filtered[i+2])?;
            result.push(((b & 0x0F) << 4) | (c >> 2));
            if filtered[i+3] != b'=' {
                let d = base64_decode_byte(filtered[i+3])?;
                result.push(((c & 0x03) << 6) | d);
            }
        }
        i += 4;
    }
    Some(result)
}

// ============================================================================
// Hex
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn coral_hex_encode(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_string_from_rust("");
    }
    let input = value_to_rust_string(unsafe { &*value });
    let mut hex = String::with_capacity(input.len() * 2);
    for b in input.as_bytes() {
        hex.push_str(&format!("{:02x}", b));
    }
    coral_make_string_from_rust(&hex)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_hex_decode(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_absent();
    }
    let input = value_to_rust_string(unsafe { &*value });
    match hex_decode_bytes(input.as_bytes()) {
        Some(decoded) => coral_bytes_from_vec(decoded),
        None => coral_make_absent(),
    }
}

fn hex_decode_bytes(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() % 2 != 0 {
        return None;
    }
    let mut result = Vec::with_capacity(data.len() / 2);
    let mut i = 0;
    while i < data.len() {
        let hi = hex_digit(data[i])?;
        let lo = hex_digit(data[i+1])?;
        result.push((hi << 4) | lo);
        i += 2;
    }
    Some(result)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
