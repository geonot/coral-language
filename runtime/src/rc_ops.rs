//! Reference counting operations for the Coral runtime.

use crate::*;


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_retain_many(ptrs: *const ValueHandle, len: usize) {
    if ptrs.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { slice::from_raw_parts(ptrs, len) };
    for &p in slice {
        unsafe { coral_value_retain(p); }
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_release_many(ptrs: *const ValueHandle, len: usize) {
    if ptrs.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { slice::from_raw_parts(ptrs, len) };
    for &p in slice {
        unsafe { coral_value_release(p); }
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_tag(value: ValueHandle) -> u8 {
    if value.is_null() {
        return ValueTag::Unit as u8;
    }
    unsafe { (*value).tag }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_as_number(value: ValueHandle) -> f64 {
    if value.is_null() {
        return 0.0;
    }
    let value = unsafe { &*value };
    if value.tag == ValueTag::Number as u8 {
        unsafe { value.payload.number }
    } else {
        0.0
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_as_ptr(value: ValueHandle) -> *mut c_void {
    if value.is_null() {
        return ptr::null_mut();
    }
    let value = unsafe { &*value };
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::Number) | Ok(ValueTag::Bool) | Ok(ValueTag::Unit) => ptr::null_mut(),
        _ => value.heap_ptr(),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_as_bool(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value = unsafe { &*value };
    if value.tag == ValueTag::Bool as u8 {
        unsafe { value.payload.inline[0] & 1 }
    } else if value.tag == ValueTag::Number as u8 {
        let num = unsafe { value.payload.number };
        if num.abs() > f64::EPSILON {
            1
        } else {
            0
        }
    } else {
        0
    }
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_value_retain(value: ValueHandle) {
    if value.is_null() {
        return;
    }
    let value = unsafe { &*value };

    // Stack-allocated values are immortal — skip refcounting entirely
    if (value.flags & 0b1000_0000) != 0 {
        return;
    }

    // ── Non-atomic fast path (M2.2) ──────────────────────────────────────
    // When the value is still owned by the current thread, use plain
    // load+store instead of atomic fetch_add. On x86-64 this eliminates
    // the `lock` prefix (~5-10x faster). On ARM it avoids the ldxr/stxr
    // exclusive-monitor loop.
    let owner = value.owner_thread;
    if owner != 0 && owner == current_thread_id() {
        let rc = value.refcount.load(Ordering::Relaxed);
        debug_assert!(rc > 0, "retain on freed value");
        if rc == u64::MAX {
            RETAIN_SATURATED.fetch_add(1, Ordering::Relaxed);
            return;
        }
        value.refcount.store(rc + 1, Ordering::Relaxed);
        RETAIN_COUNT.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // ── Atomic path (shared/frozen values) ───────────────────────────────
    let rc = value.refcount.load(Ordering::Relaxed);
    debug_assert!(rc > 0, "retain on freed value");
    if rc == u64::MAX {
        RETAIN_SATURATED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    value.refcount.fetch_add(1, Ordering::Relaxed);
    RETAIN_COUNT.fetch_add(1, Ordering::Relaxed);
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_value_release(value: ValueHandle) {
    if value.is_null() {
        return;
    }
    let value_ref = unsafe { &*value };

    // Stack-allocated values are immortal — never freed
    if (value_ref.flags & 0b1000_0000) != 0 {
        return;
    }

    // ── Non-atomic fast path (M2.2) ──────────────────────────────────────
    // When the value is still owned by the current thread, skip the CAS
    // loop entirely. Plain load+store is sufficient since no other thread
    // can access this value.
    let owner = value_ref.owner_thread;
    if owner != 0 && owner == current_thread_id() {
        let rc = value_ref.refcount.load(Ordering::Relaxed);
        if rc == 0 {
            RELEASE_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
            debug_assert!(false, "release underflow on value tag {}", value_ref.tag);
            return;
        }
        RELEASE_COUNT.fetch_add(1, Ordering::Relaxed);
        value_ref.refcount.store(rc - 1, Ordering::Relaxed);
        if rc > 1 {
            cycle_detector::possible_root(value);
            if cycle_detector::auto_cycle_collection_enabled() {
                let count = CYCLE_COLLECTION_COUNTER.fetch_add(1, Ordering::Relaxed);
                if count % CYCLE_COLLECTION_THRESHOLD == 0 {
                    cycle_detector::collect_cycles();
                }
            }
            return;
        }
        // rc == 1: last reference — fall through to deallocation
        // No acquire fence needed: single-threaded ownership guarantees
        // all writes are visible to this thread.
        weak_ref::notify_value_deallocated(value);
        cycle_detector::notify_value_freed(value);
        let value_ref_mut = unsafe { &mut *value };
        RELEASE_QUEUE.with(|queue| {
            if let Ok(mut guard) = queue.try_borrow_mut() {
                if let Some(q) = &mut *guard {
                    if let Some(nn) = ptr::NonNull::new(value as *mut c_void) {
                        q.push(nn);
                        return;
                    }
                }
            }
            unsafe { drop_heap_value(value_ref_mut); }
            LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
            if !recycle_value_box(value) {
                unsafe { drop(Box::from_raw(value)); }
            }
        });
        return;
    }

    // ── Atomic path (shared/frozen values) ───────────────────────────────
    // Use compare_exchange loop to safely decrement the refcount,
    // avoiding the TOCTOU race between a Relaxed load and a subsequent
    // fetch_sub (which would wrap on underflow).
    loop {
        let rc = value_ref.refcount.load(Ordering::Relaxed);
        if rc == 0 {
            RELEASE_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
            debug_assert!(false, "release underflow on value tag {}", value_ref.tag);
            return;
        }
        // Release ordering: ensures all writes made while holding this
        // reference are visible to the thread that observes the final drop.
        match value_ref.refcount.compare_exchange_weak(
            rc, rc - 1, Ordering::Release, Ordering::Relaxed
        ) {
            Ok(prev) => {
                RELEASE_COUNT.fetch_add(1, Ordering::Relaxed);
                if prev > 1 {
                    cycle_detector::possible_root(value);
                    if cycle_detector::auto_cycle_collection_enabled() {
                        let count = CYCLE_COLLECTION_COUNTER.fetch_add(1, Ordering::Relaxed);
                        if count % CYCLE_COLLECTION_THRESHOLD == 0 {
                            cycle_detector::collect_cycles();
                        }
                    }
                    return;
                }
                // prev == 1: this was the last reference
                break;
            }
            Err(_) => continue, // CAS failed, retry
        }
    }
    // Acquire fence: ensures we see all writes from threads that released
    // their references before we free the value's memory.
    std::sync::atomic::fence(Ordering::Acquire);

    // Notify weak reference system before deallocation
    weak_ref::notify_value_deallocated(value);

    // Remove from cycle detector tracking before freeing (prevents use-after-free
    // in concurrent cycle collection scans)
    cycle_detector::notify_value_freed(value);

    let value_ref_mut = unsafe { &mut *value };
    RELEASE_QUEUE.with(|queue| {
        // Use try_borrow_mut to avoid panic on reentrant releases
        // (e.g., when drop_heap_value releases contained values)
        if let Ok(mut guard) = queue.try_borrow_mut() {
            if let Some(q) = &mut *guard {
                if let Some(nn) = ptr::NonNull::new(value as *mut c_void) {
                    q.push(nn);
                    return;
                }
            }
        }
        // Either no queue or reentrant call - free immediately
        unsafe { drop_heap_value(value_ref_mut); }
        LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
        if !recycle_value_box(value) {
            unsafe { drop(Box::from_raw(value)); }
        }
    });
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_release_queue_init(limit: usize) {
    RELEASE_QUEUE.with(|queue| {
        *queue.borrow_mut() = Some(rc_deferred::ReleaseQueue::with_limit(limit.max(1024)));
    });
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_release_queue_flush() {
    RELEASE_QUEUE.with(|queue| {
        if let Some(q) = &mut *queue.borrow_mut() {
            q.drain(|ptr| unsafe {
                let value = ptr.as_ptr() as ValueHandle;
                let value_ref = &mut *value;
                drop_heap_value(value_ref);
                LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
                if !recycle_value_box(value) {
                    drop(Box::from_raw(value));
                }
            });
        }
    });
}

