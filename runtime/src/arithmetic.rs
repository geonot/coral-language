//! Value arithmetic operations for the Coral runtime.

use crate::*;


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_add(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_unit();
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    
    // Error propagation: if either operand is an error, return that error
    if va.is_err() {
        unsafe { coral_value_retain(a); }
        return a;
    }
    if vb.is_err() {
        unsafe { coral_value_retain(b); }
        return b;
    }
    // Absent propagation
    if va.is_absent() || vb.is_absent() {
        return coral_make_absent();
    }
    
    if va.tag == ValueTag::Number as u8 && vb.tag == ValueTag::Number as u8 {
        let result = unsafe { va.payload.number } + unsafe { vb.payload.number };
        coral_make_number(result)
    } else if va.tag == ValueTag::String as u8 && vb.tag == ValueTag::String as u8 {
        coral_string_concat(a, b)
    } else if va.tag == ValueTag::String as u8 {
        // String + non-string: coerce right operand to string, then concat.
        // This enables template string interpolation like "count: {n}".
        let right_str = value_to_display_string(vb);
        let left_bytes = string_to_bytes(va);
        let mut result = Vec::with_capacity(left_bytes.len() + right_str.len());
        result.extend_from_slice(&left_bytes);
        result.extend_from_slice(right_str.as_bytes());
        coral_make_string(result.as_ptr(), result.len())
    } else if vb.tag == ValueTag::String as u8 {
        // non-string + String: coerce left operand to string, then concat.
        let left_str = value_to_display_string(va);
        let right_bytes = string_to_bytes(vb);
        let mut result = Vec::with_capacity(left_str.len() + right_bytes.len());
        result.extend_from_slice(left_str.as_bytes());
        result.extend_from_slice(&right_bytes);
        coral_make_string(result.as_ptr(), result.len())
    } else if va.tag == ValueTag::Bytes as u8 && vb.tag == ValueTag::Bytes as u8 {
        coral_bytes_concat(a, b)
    } else {
        coral_make_unit()
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_equals(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    // Error propagation for equality checks
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return a;
        }
    }
    if !b.is_null() {
        let vb = unsafe { &*b };
        if vb.is_err() {
            unsafe { coral_value_retain(b); }
            return b;
        }
    }
    
    let result = values_equal_handles(a, b);
    coral_make_bool(if result { 1 } else { 0 })
}


/// Not-equals: returns a Bool value that is true when a != b.
#[unsafe(no_mangle)]
pub extern "C" fn coral_value_not_equals(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    // Error propagation
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return a;
        }
    }
    if !b.is_null() {
        let vb = unsafe { &*b };
        if vb.is_err() {
            unsafe { coral_value_retain(b); }
            return b;
        }
    }
    let result = values_equal_handles(a, b);
    coral_make_bool(if result { 0 } else { 1 })
}


/// Helper to propagate errors in binary operations.
/// Returns Some(error_handle) if either operand is an error/absent, None otherwise.
#[inline]
pub(crate) fn propagate_binary_error(a: ValueHandle, b: ValueHandle) -> Option<ValueHandle> {
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return Some(a);
        }
        if va.is_absent() {
            return Some(coral_make_absent());
        }
    }
    if !b.is_null() {
        let vb = unsafe { &*b };
        if vb.is_err() {
            unsafe { coral_value_retain(b); }
            return Some(b);
        }
        if vb.is_absent() {
            return Some(coral_make_absent());
        }
    }
    None
}


/// Helper to propagate errors in unary operations.
#[inline]
pub(crate) fn propagate_unary_error(a: ValueHandle) -> Option<ValueHandle> {
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return Some(a);
        }
        if va.is_absent() {
            return Some(coral_make_absent());
        }
    }
    None
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitand(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let result = handle_to_i64(a) & handle_to_i64(b);
    coral_make_number(result as f64)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitor(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let result = handle_to_i64(a) | handle_to_i64(b);
    coral_make_number(result as f64)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitxor(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let result = handle_to_i64(a) ^ handle_to_i64(b);
    coral_make_number(result as f64)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitnot(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    let result = !handle_to_i64(value);
    coral_make_number(result as f64)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_shift_left(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let lhs = handle_to_i64(a);
    let rhs = (handle_to_i64(b) & 63) as u32;
    coral_make_number(lhs.wrapping_shl(rhs) as f64)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_shift_right(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let lhs = handle_to_i64(a);
    let rhs = (handle_to_i64(b) & 63) as u32;
    coral_make_number((lhs >> rhs) as f64)
}

