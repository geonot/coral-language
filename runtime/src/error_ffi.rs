use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_error(code: u32, name_ptr: *const u8, name_len: usize) -> ValueHandle {
    let name_handle = coral_make_string(name_ptr, name_len);

    let metadata = Box::new(ErrorMetadata {
        code,
        _reserved: 0,
        name: name_handle,
        origin_span: 0,
    });

    record_heap_bytes(std::mem::size_of::<ErrorMetadata>());
    alloc_value(Value::error(Box::into_raw(metadata)))
}

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

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_absent() -> ValueHandle {
    alloc_value(Value::absent())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_is_err(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_err() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_is_absent(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_absent() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_is_ok(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_ok() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_error_name(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if let Some(metadata) = value_ref.error_metadata() {
        unsafe {
            coral_value_retain(metadata.name);
        }
        metadata.name
    } else {
        coral_make_unit()
    }
}

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

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_or(value: ValueHandle, default: ValueHandle) -> ValueHandle {
    if value.is_null() {
        unsafe {
            coral_value_retain(default);
        }
        return default;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_ok() {
        unsafe {
            coral_value_retain(value);
        }
        value
    } else {
        unsafe {
            coral_value_retain(default);
        }
        default
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_unwrap_or(value: ValueHandle, default: ValueHandle) -> ValueHandle {
    coral_value_or(value, default)
}
