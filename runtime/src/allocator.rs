use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::ptr;

const SIZE_CLASSES: [usize; 8] = [16, 32, 64, 128, 256, 512, 1024, 2048];
const MAX_FREE_PER_CLASS: usize = 256;

struct FreeList {
    slots: Vec<*mut u8>,
}

impl FreeList {
    fn new() -> Self {
        Self {
            slots: Vec::with_capacity(64),
        }
    }

    fn pop(&mut self) -> Option<*mut u8> {
        self.slots.pop()
    }

    fn push(&mut self, ptr: *mut u8) -> bool {
        if self.slots.len() < MAX_FREE_PER_CLASS {
            self.slots.push(ptr);
            true
        } else {
            false
        }
    }

    fn drain(&mut self) -> Vec<*mut u8> {
        std::mem::take(&mut self.slots)
    }
}

struct SizeClassAllocator {
    free_lists: [FreeList; 8],
    alloc_count: u64,
    reuse_count: u64,
}

impl SizeClassAllocator {
    fn new() -> Self {
        Self {
            free_lists: [
                FreeList::new(),
                FreeList::new(),
                FreeList::new(),
                FreeList::new(),
                FreeList::new(),
                FreeList::new(),
                FreeList::new(),
                FreeList::new(),
            ],
            alloc_count: 0,
            reuse_count: 0,
        }
    }

    fn size_class_index(size: usize) -> Option<usize> {
        SIZE_CLASSES.iter().position(|&s| s >= size)
    }

    fn allocate(&mut self, size: usize, align: usize) -> *mut u8 {
        if let Some(idx) = Self::size_class_index(size) {
            if let Some(ptr) = self.free_lists[idx].pop() {
                self.reuse_count += 1;
                return ptr;
            }
            self.alloc_count += 1;
            let class_size = SIZE_CLASSES[idx];
            let layout = Layout::from_size_align(class_size, align.max(8)).unwrap();
            unsafe { alloc::alloc(layout) }
        } else {
            self.alloc_count += 1;
            let layout = Layout::from_size_align(size, align.max(8)).unwrap();
            unsafe { alloc::alloc(layout) }
        }
    }

    fn deallocate(&mut self, ptr: *mut u8, size: usize, align: usize) {
        if let Some(idx) = Self::size_class_index(size) {
            if self.free_lists[idx].push(ptr) {
                return;
            }
        }
        let actual_size = Self::size_class_index(size)
            .map(|i| SIZE_CLASSES[i])
            .unwrap_or(size);
        let layout = Layout::from_size_align(actual_size, align.max(8)).unwrap();
        unsafe { alloc::dealloc(ptr, layout) };
    }

    fn purge(&mut self) {
        for (i, fl) in self.free_lists.iter_mut().enumerate() {
            let ptrs = fl.drain();
            for p in ptrs {
                let layout = Layout::from_size_align(SIZE_CLASSES[i], 8).unwrap();
                unsafe { alloc::dealloc(p, layout) };
            }
        }
    }

    fn stats(&self) -> AllocStats {
        let cached: usize = self.free_lists.iter().map(|fl| fl.slots.len()).sum();
        AllocStats {
            alloc_count: self.alloc_count,
            reuse_count: self.reuse_count,
            cached_blocks: cached as u64,
        }
    }
}

impl Drop for SizeClassAllocator {
    fn drop(&mut self) {
        self.purge();
    }
}

thread_local! {
    static THREAD_ALLOCATOR: RefCell<SizeClassAllocator> = RefCell::new(SizeClassAllocator::new());
}

pub fn pool_alloc(size: usize, align: usize) -> *mut u8 {
    THREAD_ALLOCATOR.with(|a| a.borrow_mut().allocate(size, align))
}

pub fn pool_dealloc(ptr: *mut u8, size: usize, align: usize) {
    THREAD_ALLOCATOR.with(|a| a.borrow_mut().deallocate(ptr, size, align));
}

pub fn pool_purge() {
    THREAD_ALLOCATOR.with(|a| a.borrow_mut().purge());
}

pub fn pool_stats() -> AllocStats {
    THREAD_ALLOCATOR.with(|a| a.borrow().stats())
}

#[derive(Debug, Clone)]
pub struct AllocStats {
    pub alloc_count: u64,
    pub reuse_count: u64,
    pub cached_blocks: u64,
}

pub struct ArenaAllocator {
    chunks: Vec<(*mut u8, Layout)>,
    current: *mut u8,
    offset: usize,
    capacity: usize,
}

const ARENA_CHUNK_SIZE: usize = 64 * 1024;

impl ArenaAllocator {
    pub fn new() -> Self {
        let layout = Layout::from_size_align(ARENA_CHUNK_SIZE, 16).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        Self {
            chunks: vec![(ptr, layout)],
            current: ptr,
            offset: 0,
            capacity: ARENA_CHUNK_SIZE,
        }
    }

    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        let aligned_offset = (self.offset + align - 1) & !(align - 1);
        if aligned_offset + size <= self.capacity {
            let ptr = unsafe { self.current.add(aligned_offset) };
            self.offset = aligned_offset + size;
            return ptr;
        }

        let chunk_size = ARENA_CHUNK_SIZE.max(size + align);
        let layout = Layout::from_size_align(chunk_size, 16).unwrap();
        let ptr = unsafe { alloc::alloc(layout) };
        self.chunks.push((ptr, layout));
        self.current = ptr;
        self.capacity = chunk_size;
        self.offset = size;
        ptr
    }

    pub fn reset(&mut self) {
        while self.chunks.len() > 1 {
            let (ptr, layout) = self.chunks.pop().unwrap();
            unsafe { alloc::dealloc(ptr, layout) };
        }
        self.current = self.chunks[0].0;
        self.capacity = self.chunks[0].1.size();
        self.offset = 0;
    }

    pub fn bytes_used(&self) -> usize {
        let prev_chunks: usize = if self.chunks.len() > 1 {
            self.chunks[..self.chunks.len() - 1]
                .iter()
                .map(|(_, l)| l.size())
                .sum()
        } else {
            0
        };
        prev_chunks + self.offset
    }
}

impl Drop for ArenaAllocator {
    fn drop(&mut self) {
        for (ptr, layout) in &self.chunks {
            unsafe { alloc::dealloc(*ptr, *layout) };
        }
    }
}

unsafe impl Send for ArenaAllocator {}

thread_local! {
    static FUNCTION_ARENA: RefCell<Option<ArenaAllocator>> = RefCell::new(None);
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_region_enter() {
    FUNCTION_ARENA.with(|a| {
        let mut slot = a.borrow_mut();
        if slot.is_none() {
            *slot = Some(ArenaAllocator::new());
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_region_exit() {
    FUNCTION_ARENA.with(|a| {
        if let Some(arena) = a.borrow_mut().as_mut() {
            arena.reset();
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_region_alloc(size: usize, align: usize) -> *mut u8 {
    FUNCTION_ARENA.with(|a| {
        let mut slot = a.borrow_mut();
        match slot.as_mut() {
            Some(arena) => arena.alloc(size, align),
            None => pool_alloc(size, align),
        }
    })
}

pub fn batch_alloc_list(count: usize, elem_size: usize) -> Vec<*mut u8> {
    let total = count * elem_size;
    let layout = Layout::from_size_align(total, 8).unwrap();
    let base = unsafe { alloc::alloc(layout) };
    if base.is_null() {
        return Vec::new();
    }
    (0..count)
        .map(|i| unsafe { base.add(i * elem_size) })
        .collect()
}

pub fn batch_dealloc_list(ptrs: &[*mut u8], count: usize, elem_size: usize) {
    if ptrs.is_empty() {
        return;
    }
    let total = count * elem_size;
    let layout = Layout::from_size_align(total, 8).unwrap();
    unsafe { alloc::dealloc(ptrs[0], layout) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_alloc_reuse() {
        let p1 = pool_alloc(64, 8);
        assert!(!p1.is_null());
        pool_dealloc(p1, 64, 8);
        let p2 = pool_alloc(64, 8);
        assert_eq!(p1, p2);
        pool_dealloc(p2, 64, 8);
    }

    #[test]
    fn pool_stats_tracking() {
        pool_purge();
        let _ = pool_alloc(32, 8);
        let stats = pool_stats();
        assert!(stats.alloc_count >= 1);
        pool_purge();
    }

    #[test]
    fn arena_basic() {
        let mut arena = ArenaAllocator::new();
        let p1 = arena.alloc(100, 8);
        let p2 = arena.alloc(200, 8);
        assert!(!p1.is_null());
        assert!(!p2.is_null());
        assert_ne!(p1, p2);
        assert!(arena.bytes_used() >= 300);
        arena.reset();
        assert_eq!(arena.bytes_used(), 0);
    }

    #[test]
    fn arena_large_allocation() {
        let mut arena = ArenaAllocator::new();
        let p = arena.alloc(128 * 1024, 16);
        assert!(!p.is_null());
    }

    #[test]
    fn batch_alloc_and_dealloc() {
        let ptrs = batch_alloc_list(10, 64);
        assert_eq!(ptrs.len(), 10);
        for (i, p) in ptrs.iter().enumerate() {
            assert!(!p.is_null());
            if i > 0 {
                assert_eq!(
                    (*p as usize) - (ptrs[i - 1] as usize),
                    64
                );
            }
        }
        batch_dealloc_list(&ptrs, 10, 64);
    }

    #[test]
    fn size_class_index_mapping() {
        assert_eq!(SizeClassAllocator::size_class_index(1), Some(0));
        assert_eq!(SizeClassAllocator::size_class_index(16), Some(0));
        assert_eq!(SizeClassAllocator::size_class_index(17), Some(1));
        assert_eq!(SizeClassAllocator::size_class_index(2048), Some(7));
        assert_eq!(SizeClassAllocator::size_class_index(2049), None);
    }

    #[test]
    fn region_enter_alloc_exit() {
        coral_region_enter();
        let p1 = coral_region_alloc(128, 8);
        assert!(!p1.is_null());
        let p2 = coral_region_alloc(256, 16);
        assert!(!p2.is_null());
        assert_ne!(p1, p2);
        coral_region_exit();
    }
}
