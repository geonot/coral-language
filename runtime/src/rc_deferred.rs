//! Deferred release queue placeholder for future RC batching.
use std::collections::VecDeque;
use std::ptr::NonNull;

pub struct ReleaseQueue {
    queue: VecDeque<NonNull<core::ffi::c_void>>,
    limit: usize,
}

impl ReleaseQueue {
    pub fn with_limit(limit: usize) -> Self {
        Self { queue: VecDeque::with_capacity(limit.min(4096)), limit }
    }

    pub fn push(&mut self, ptr: NonNull<core::ffi::c_void>) {
        if self.queue.len() < self.limit {
            self.queue.push_back(ptr);
        }
    }

    pub fn drain<F: FnMut(NonNull<core::ffi::c_void>)>(&mut self, mut drop_fn: F) {
        while let Some(ptr) = self.queue.pop_front() {
            drop_fn(ptr);
        }
    }
}
