//! Low-level memory operations for unsafe/external code.
//!
//! This module provides C-callable functions for direct memory manipulation,
//! used by extern declarations and inline assembly.

use libc::{free, malloc};
use std::ffi::c_void;
use std::ptr;

/// Allocate `size` bytes and return pointer (usize); returns 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn coral_malloc(size: usize) -> usize {
    if size == 0 {
        return 0;
    }
    unsafe {
        let ptr = malloc(size);
        if ptr.is_null() {
            0
        } else {
            ptr as usize
        }
    }
}

/// Free memory at `ptr` (usize).
#[unsafe(no_mangle)]
pub extern "C" fn coral_free(ptr: usize) {
    if ptr != 0 {
        unsafe {
            free(ptr as *mut c_void);
        }
    }
}

/// Copy `len` bytes from `src` to `dst`; returns `dst`.
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

/// Set `len` bytes at `dst` to `value`; returns `dst`.
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

/// Add `offset` bytes to pointer; returns new pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_ptr_add(ptr: usize, offset: usize) -> usize {
    ptr.wrapping_add(offset)
}

/// Load u8 from pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u8(ptr: usize) -> u8 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u8) }
}

/// Load u16 from pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u16(ptr: usize) -> u16 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u16) }
}

/// Load u32 from pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u32(ptr: usize) -> u32 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u32) }
}

/// Load u64 from pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_load_u64(ptr: usize) -> u64 {
    if ptr == 0 {
        return 0;
    }
    unsafe { *(ptr as *const u64) }
}

/// Load f64 from pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_load_f64(ptr: usize) -> f64 {
    if ptr == 0 {
        return 0.0;
    }
    unsafe { *(ptr as *const f64) }
}

/// Store u8 to pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u8(ptr: usize, value: u8) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u8) = value;
        }
    }
}

/// Store u16 to pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u16(ptr: usize, value: u16) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u16) = value;
        }
    }
}

/// Store u32 to pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u32(ptr: usize, value: u32) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u32) = value;
        }
    }
}

/// Store u64 to pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_u64(ptr: usize, value: u64) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut u64) = value;
        }
    }
}

/// Store f64 to pointer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_f64(ptr: usize, value: f64) {
    if ptr != 0 {
        unsafe {
            *(ptr as *mut f64) = value;
        }
    }
}
