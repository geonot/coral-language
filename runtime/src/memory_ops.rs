use libc::{free, malloc};
use std::ffi::c_void;
use std::ptr;

#[unsafe(no_mangle)]
pub extern "C" fn coral_malloc(size: usize) -> usize {
    if size == 0 {
        return 0;
    }
    unsafe {
        let ptr = malloc(size);
        if ptr.is_null() { 0 } else { ptr as usize }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_free(ptr: usize) {
    if ptr != 0 {
        unsafe {
            free(ptr as *mut c_void);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_memcpy(dst: usize, src: usize, len: usize) -> usize {
    if dst == 0 || src == 0 || len == 0 {
        return dst;
    }
    unsafe {
        ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, len);
    }
    dst
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_memset(dst: usize, value: u8, len: usize) -> usize {
    if dst == 0 || len == 0 {
        return dst;
    }
    unsafe {
        ptr::write_bytes(dst as *mut u8, value, len);
    }
    dst
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_ptr_add(ptr: usize, offset: usize) -> usize {
    ptr.wrapping_add(offset)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u8(ptr: usize) -> u8 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u8) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u16(ptr: usize) -> u16 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u16) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u32(ptr: usize) -> u32 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u32) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u64(ptr: usize) -> u64 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u64) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_load_f64(ptr: usize) -> f64 {
    if ptr == 0 {
        return 0.0;
    }
    unsafe { *(ptr as *const f64) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u8(ptr: usize, value: u8) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u8) = value;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u16(ptr: usize, value: u16) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u16) = value;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u32(ptr: usize, value: u32) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u32) = value;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u64(ptr: usize, value: u64) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u64) = value;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_store_f64(ptr: usize, value: f64) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut f64) = value;
        }
    }
}
