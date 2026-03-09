//! Runtime metrics and stack frame operations.

use crate::*;


#[unsafe(no_mangle)]
pub extern "C" fn coral_heap_alloc(size: usize) -> *mut c_void {
    unsafe { malloc(size) }
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_heap_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe { free(ptr); }
    }
}


fn align_up(value: usize, align: usize) -> usize {
    let align = align.max(1);
    (value + align - 1) & !(align - 1)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_stack_frame_enter(pages: usize) {
    ensure_runtime_initialized();
    let page_count = pages.max(1);
    let size = page_count * PAGE_SIZE;
    STACK_PAGES_COMMITTED.fetch_add(page_count as u64, Ordering::Relaxed);
    STACK_BYTES_REQUESTED.fetch_add(size as u64, Ordering::Relaxed);
    record_heap_bytes(std::mem::size_of::<StackFrame>() + size);
    record_usage(UsageKind::StackAllocSuccess, size as u64);
    STACK_FRAMES.with(|frames| {
        frames
            .borrow_mut()
            .push(StackFrame { buffer: vec![0u8; size], cursor: 0 });
    });
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_stack_frame_leave() {
    STACK_FRAMES.with(|frames| {
        frames.borrow_mut().pop();
    });
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_stack_alloc(size: usize, align: usize) -> *mut c_void {
    if size == 0 {
        return ptr::null_mut();
    }
    STACK_FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        if let Some(frame) = frames.last_mut() {
            let cursor = align_up(frame.cursor, align.max(1));
            if cursor + size > frame.buffer.len() {
                record_usage(UsageKind::StackAllocFailure, size as u64);
                return ptr::null_mut();
            }
            let ptr = unsafe { frame.buffer.as_mut_ptr().add(cursor) };
            frame.cursor = cursor + size;
            record_usage(UsageKind::StackAllocSuccess, size as u64);
            ptr as *mut c_void
        } else {
            ptr::null_mut()
        }
    })
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_retain_count() -> u64 {
    RETAIN_COUNT.load(Ordering::Relaxed)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_release_count() -> u64 {
    RELEASE_COUNT.load(Ordering::Relaxed)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_live_values() -> u64 {
    LIVE_VALUE_COUNT.load(Ordering::Relaxed)
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_stats(out: *mut CoralRuntimeStats) {
    if out.is_null() {
        return;
    }
    let stats = CoralRuntimeStats {
        retains: RETAIN_COUNT.load(Ordering::Relaxed),
        releases: RELEASE_COUNT.load(Ordering::Relaxed),
        live_values: LIVE_VALUE_COUNT.load(Ordering::Relaxed),
    };
    unsafe {
        *out = stats;
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_metrics(out: *mut CoralRuntimeMetrics) {
    if out.is_null() {
        return;
    }
    unsafe {
        *out = snapshot_runtime_metrics();
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_value_metrics(value: ValueHandle, out: *mut CoralHandleMetrics) {
    if value.is_null() || out.is_null() {
        return;
    }
    let value_ref = unsafe { &*value };
    unsafe {
        *out = CoralHandleMetrics {
            refcount: value_ref.refcount.load(Ordering::Relaxed),
            #[cfg(feature = "metrics")]
            retains: value_ref.retain_events.load(Ordering::Relaxed) as u64,
            #[cfg(not(feature = "metrics"))]
            retains: 0,
            #[cfg(feature = "metrics")]
            releases: value_ref.release_events.load(Ordering::Relaxed) as u64,
            #[cfg(not(feature = "metrics"))]
            releases: 0,
        };
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_metrics_dump(path: *const u8, len: usize) {
    if path.is_null() || len == 0 {
        return;
    }
    let bytes = read_bytes(path, len);
    if bytes.is_empty() {
        return;
    }
    if let Ok(text) = String::from_utf8(bytes) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            dump_metrics_to_path(Path::new(trimmed));
        }
    }
}


pub(crate) fn snapshot_runtime_metrics() -> CoralRuntimeMetrics {
    CoralRuntimeMetrics {
        retains: RETAIN_COUNT.load(Ordering::Relaxed),
        retain_saturated: RETAIN_SATURATED.load(Ordering::Relaxed),
        releases: RELEASE_COUNT.load(Ordering::Relaxed),
        release_underflow: RELEASE_UNDERFLOW.load(Ordering::Relaxed),
        live_values: LIVE_VALUE_COUNT.load(Ordering::Relaxed),
        value_pool_hits: VALUE_POOL_HITS.load(Ordering::Relaxed),
        value_pool_misses: VALUE_POOL_MISSES.load(Ordering::Relaxed),
        heap_bytes: HEAP_BYTES_ALLOCATED.load(Ordering::Relaxed),
        string_bytes: STRING_BYTES_ALLOCATED.load(Ordering::Relaxed),
        list_slots: LIST_SLOTS_ALLOCATED.load(Ordering::Relaxed),
        map_slots: MAP_SLOTS_ALLOCATED.load(Ordering::Relaxed),
        stack_pages: STACK_PAGES_COMMITTED.load(Ordering::Relaxed),
        stack_bytes: STACK_BYTES_REQUESTED.load(Ordering::Relaxed),
        timestamp_ns: metrics_timestamp_ns(),
    }
}


fn metrics_timestamp_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}


pub(crate) fn metrics_json(metrics: &CoralRuntimeMetrics) -> String {
    format!(
        "{{\n  \"timestamp_ns\": {},\n  \"retains\": {},\n  \"retain_saturated\": {},\n  \"releases\": {},\n  \"release_underflow\": {},\n  \"live_values\": {},\n  \"value_pool_hits\": {},\n  \"value_pool_misses\": {},\n  \"heap_bytes\": {},\n  \"string_bytes\": {},\n  \"list_slots\": {},\n  \"map_slots\": {},\n  \"stack_pages\": {},\n  \"stack_bytes\": {}\n}}\n",
        metrics.timestamp_ns,
        metrics.retains,
        metrics.retain_saturated,
        metrics.releases,
        metrics.release_underflow,
        metrics.live_values,
        metrics.value_pool_hits,
        metrics.value_pool_misses,
        metrics.heap_bytes,
        metrics.string_bytes,
        metrics.list_slots,
        metrics.map_slots,
        metrics.stack_pages,
        metrics.stack_bytes
    )
}

