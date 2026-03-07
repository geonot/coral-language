//! Error value FFI functions for the Coral runtime.

use crate::*;


/// Create an error value with the given code and name.
/// 
/// # Arguments
/// * `code` - Numeric error code (0 for anonymous errors)
/// * `name_ptr` - Pointer to UTF-8 error name string (e.g., "NotFound")
/// * `name_len` - Length of the name string in bytes
/// 
/// # Returns
/// A new error value with the ERR flag set
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_error(code: u32, name_ptr: *const u8, name_len: usize) -> ValueHandle {
    // Create the error name as a string value
    let name_handle = coral_make_string(name_ptr, name_len);
    
    // Allocate error metadata
    let metadata = Box::new(ErrorMetadata {
        code,
        _reserved: 0,
        name: name_handle,
        origin_span: 0,
    });
    
    record_heap_bytes(std::mem::size_of::<ErrorMetadata>());
    alloc_value(Value::error(Box::into_raw(metadata)))
}


/// Create an error value with the given code, name, and origin span.
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_error_with_span(
    code: u32,
    name_ptr: *const u8,
    name_len: usize,
    origin_span: u64,
) -> ValueHandle {
    let name_handle = coral_make_string(name_ptr, name_len);
    
    let metadata = Box::new(ErrorMetadata {
        code,
        _reserved: 0,
        name: name_handle,
        origin_span,
    });
    
    record_heap_bytes(std::mem::size_of::<ErrorMetadata>());
    alloc_value(Value::error(Box::into_raw(metadata)))
}


/// Create an absent/None value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_absent() -> ValueHandle {
    alloc_value(Value::absent())
}


/// Check if a value is an error.
/// Returns 1 if the value has the ERR flag set, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn coral_is_err(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_err() { 1 } else { 0 }
}


/// Check if a value is absent/None.
/// Returns 1 if the value has the ABSENT flag set, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn coral_is_absent(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_absent() { 1 } else { 0 }
}


/// Check if a value is ok (neither error nor absent).
/// Returns 1 if the value is ok, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn coral_is_ok(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_ok() { 1 } else { 0 }
}


/// Get the error name from an error value.
/// Returns the error name string, or unit if not an error.
#[unsafe(no_mangle)]
pub extern "C" fn coral_error_name(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if let Some(metadata) = value_ref.error_metadata() {
        unsafe { coral_value_retain(metadata.name); }
        metadata.name
    } else {
        coral_make_unit()
    }
}


/// Get the error code from an error value.
/// Returns the error code, or 0 if not an error.
#[unsafe(no_mangle)]
pub extern "C" fn coral_error_code(value: ValueHandle) -> u32 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if let Some(metadata) = value_ref.error_metadata() {
        metadata.code
    } else {
        0
    }
}


/// Return the value if ok, or the default if error/absent.
/// This retains the returned value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_value_or(value: ValueHandle, default: ValueHandle) -> ValueHandle {
    if value.is_null() {
        unsafe { coral_value_retain(default); }
        return default;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_ok() {
        unsafe { coral_value_retain(value); }
        value
    } else {
        unsafe { coral_value_retain(default); }
        default
    }
}


/// Unwrap the value or return the default.
/// Same as coral_value_or but named for familiarity.
#[unsafe(no_mangle)]
pub extern "C" fn coral_unwrap_or(value: ValueHandle, default: ValueHandle) -> ValueHandle {
    coral_value_or(value, default)
}

