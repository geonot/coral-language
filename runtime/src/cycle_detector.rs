//! Cycle detection for reference-counted values in Coral runtime.
//!
//! This module implements cycle detection using a mark-and-sweep approach
//! for container types (Lists, Maps, and Stores) that can form reference cycles.
//!
//! The algorithm is based on Bacon & Rajan's "Concurrent Cycle Collection" 
//! with simplifications for single-threaded collection phases.

use std::collections::{HashSet, HashMap, VecDeque};
use std::sync::{Mutex, atomic::Ordering};
use std::sync::atomic::AtomicBool;
use std::cell::RefCell;

use crate::{ValueHandle, ValueTag, Value};

// ═══════════════════════════════════════════════════════════════════════
// M3.1: Thread-Local Cycle Root Buffers
// ═══════════════════════════════════════════════════════════════════════

/// Threshold: when local buffer reaches this many entries, flush to global.
const LOCAL_BUFFER_THRESHOLD: usize = 64;

/// Flag: set to true when a collection is pending and threads should flush.
static COLLECTION_PENDING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Thread-local buffer of possible cycle roots, avoiding global lock contention.
    static LOCAL_ROOTS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(LOCAL_BUFFER_THRESHOLD));
}

/// Flush the current thread's local root buffer into the global CycleDetector.
/// Must be called with the global lock NOT held (acquires it internally).
fn flush_local_roots() {
    LOCAL_ROOTS.with(|cell| {
        let mut local = cell.borrow_mut();
        if local.is_empty() {
            return;
        }
        if let Ok(mut det) = detector().lock() {
            for addr in local.drain(..) {
                let handle = addr as ValueHandle;
                if handle.is_null() {
                    continue;
                }
                // Check that the value is still a valid container
                let value = unsafe { &*handle };
                if !is_container(value) {
                    continue;
                }
                // M3.2: Read epoch before mutably borrowing info
                let epoch = det.current_epoch;
                let info = det.info.entry(addr).or_default();
                if info.color != Color::Purple {
                    info.color = Color::Purple;
                    info.birth_epoch = epoch;
                    if !info.buffered {
                        info.buffered = true;
                    }
                }
                // M3.2: New roots go to young generation
                det.young_roots.insert(addr);
                det.roots.insert(addr);
            }
        }
    });
}

/// Flush ALL thread-local buffers by signaling threads and flushing our own.
/// Called before collection to ensure all roots are visible to the collector.
fn flush_all_thread_local_roots() {
    // Signal other threads to flush on their next possible_root call
    COLLECTION_PENDING.store(true, Ordering::Release);
    // Flush our own buffer immediately
    flush_local_roots();
}

/// Color markers for cycle detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Color {
    /// In use or free (normal state)
    Black,
    /// Possible member of a garbage cycle
    Gray,
    /// Member of a garbage cycle (to be collected)
    White,
    /// Possible root of a garbage cycle
    Purple,
}

/// Metadata for cycle detection tracking.
struct CycleInfo {
    color: Color,
    /// Whether this root is buffered for collection
    buffered: bool,
    /// M3.2: Epoch when this root was first inserted
    birth_epoch: u64,
}

impl Default for CycleInfo {
    fn default() -> Self {
        Self {
            color: Color::Black,
            buffered: false,
            birth_epoch: 0,
        }
    }
}

/// M3.2: How often to run a full (old + young) collection.
/// Every FULL_COLLECTION_INTERVAL-th collection scans both generations.
const FULL_COLLECTION_INTERVAL: u64 = 5;

/// Global cycle detector state.
struct CycleDetector {
    /// Metadata for tracked values
    info: HashMap<usize, CycleInfo>,
    /// M3.2: Young generation roots (added since last collection)
    young_roots: HashSet<usize>,
    /// M3.2: Old generation roots (survived at least one young-gen collection)
    old_roots: HashSet<usize>,
    /// Unified roots view (for backward-compat with mark/scan/collect phases)
    roots: HashSet<usize>,
    /// Whether cycle collection is currently in progress
    collecting: bool,
    /// Statistics
    cycles_detected: u64,
    values_collected: u64,
    /// M3.2: Current epoch counter, incremented at each collection
    current_epoch: u64,
    /// M3.2: Number of young-generation collections performed
    young_collections: u64,
    /// M3.2: Number of full (both generations) collections performed
    full_collections: u64,
}

impl Default for CycleDetector {
    fn default() -> Self {
        Self {
            info: HashMap::new(),
            young_roots: HashSet::new(),
            old_roots: HashSet::new(),
            roots: HashSet::new(),
            collecting: false,
            cycles_detected: 0,
            values_collected: 0,
            current_epoch: 0,
            young_collections: 0,
            full_collections: 0,
        }
    }
}

static CYCLE_DETECTOR: std::sync::OnceLock<Mutex<CycleDetector>> = std::sync::OnceLock::new();

fn detector() -> &'static Mutex<CycleDetector> {
    CYCLE_DETECTOR.get_or_init(|| Mutex::new(CycleDetector::default()))
}

/// Check if a value can contain references (is a container type).
fn is_container(value: &Value) -> bool {
    matches!(
        ValueTag::try_from(value.tag),
        Ok(ValueTag::List) | Ok(ValueTag::Map) | Ok(ValueTag::Store) | Ok(ValueTag::Tagged) | Ok(ValueTag::Closure)
    )
}

/// Get all values referenced by a container.
fn get_children(handle: ValueHandle) -> Vec<ValueHandle> {
    if handle.is_null() {
        return Vec::new();
    }
    
    let value = unsafe { &*handle };
    let mut children = Vec::new();
    
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::List) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                let list = unsafe { &*(ptr as *const crate::ListObject) };
                children.extend(list.items.iter().copied().filter(|h| !h.is_null()));
            }
        }
        Ok(ValueTag::Map) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                let map = unsafe { &*(ptr as *const crate::MapObject) };
                for bucket in &map.buckets {
                    if bucket.state == crate::MapBucketState::Occupied {
                        if !bucket.key.is_null() {
                            children.push(bucket.key);
                        }
                        if !bucket.value.is_null() {
                            children.push(bucket.value);
                        }
                    }
                }
            }
        }
        Ok(ValueTag::Tagged) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                let tagged = unsafe { &*(ptr as *const crate::TaggedValue) };
                for i in 0..tagged.field_count {
                    if !tagged.fields.is_null() {
                        let field = unsafe { *tagged.fields.add(i) };
                        if !field.is_null() {
                            children.push(field);
                        }
                    }
                }
            }
        }
        // Store values are containers (tracked as roots) but their children
        // are managed by the persistent store engine, not by refcounting.
        // No child handles to traverse.
        Ok(ValueTag::Store) => {}
        Ok(ValueTag::Closure) => {
            // M3.4: Traverse captured values in the closure environment.
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                let closure = unsafe { &*(ptr as *const crate::ClosureObject) };
                if !closure.env.is_null() && closure.capture_count > 0 {
                    let env_ptr = closure.env as *const i64;
                    for i in 0..closure.capture_count {
                        let nb_val = unsafe { *env_ptr.add(i) } as u64;
                        let handle = crate::nanbox_ffi::nb_to_handle(nb_val);
                        if !handle.is_null() {
                            children.push(handle);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    
    children
}

/// Called when a container value's refcount is decremented.
/// This marks the value as a potential cycle root.
/// M3.1: Uses thread-local buffer to avoid global lock contention on the hot path.
pub fn possible_root(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let value = unsafe { &*handle };
    if !is_container(value) {
        return;
    }
    
    let addr = handle as usize;

    // If a collection is pending, flush our local buffer first
    if COLLECTION_PENDING.load(Ordering::Acquire) {
        flush_local_roots();
        COLLECTION_PENDING.store(false, Ordering::Release);
    }

    LOCAL_ROOTS.with(|cell| {
        let mut local = cell.borrow_mut();
        local.push(addr);
        if local.len() >= LOCAL_BUFFER_THRESHOLD {
            // Buffer full — flush to global under lock
            drop(local); // release borrow before flush_local_roots re-borrows
            flush_local_roots();
        }
    });
}

/// Called when a value is about to be freed (refcount reached 0).
/// Removes the value from cycle detection tracking to prevent use-after-free
/// during concurrent cycle collection scans.
pub fn notify_value_freed(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    let addr = handle as usize;
    let mut det = detector().lock().unwrap();
    det.roots.remove(&addr);
    // M3.2: Remove from both generations
    det.young_roots.remove(&addr);
    det.old_roots.remove(&addr);
    det.info.remove(&addr);
}

/// Run a cycle collection phase.
/// M3.2: Uses generational collection — young roots are collected frequently,
/// old roots only every FULL_COLLECTION_INTERVAL-th collection.
pub fn collect_cycles() {
    // M3.1: Flush all thread-local root buffers before collecting
    flush_all_thread_local_roots();

    // Check if already collecting to prevent recursion
    let is_full_collection;
    {
        let mut det = detector().lock().unwrap();
        if det.collecting {
            return;
        }
        det.collecting = true;
        det.current_epoch += 1;

        // M3.2: Decide if this is a full or young-only collection
        is_full_collection = (det.current_epoch % FULL_COLLECTION_INTERVAL) == 0;

        if is_full_collection {
            // Full collection: scan both generations
            det.roots = det.young_roots.union(&det.old_roots).copied().collect();
            det.full_collections += 1;
        } else {
            // Young-only collection: scan only young roots
            det.roots = det.young_roots.clone();
            det.young_collections += 1;
        }
    }

    // Mark candidates
    mark_roots();
    
    // Scan for actual garbage
    scan_roots();
    
    // Collect garbage cycles
    collect_roots();
    
    // M3.2: Promote surviving young roots to old generation
    {
        let mut det = detector().lock().unwrap();
        // Roots that survived this collection get promoted to old
        let surviving_young: Vec<usize> = det.young_roots.iter()
            .filter(|addr| det.info.contains_key(addr))
            .copied()
            .collect();
        for addr in surviving_young {
            det.old_roots.insert(addr);
        }
        det.young_roots.clear();
        det.collecting = false;
    }
}

/// Phase 1: Mark all potential roots
fn mark_roots() {
    let roots: Vec<usize> = {
        let det = detector().lock().unwrap();
        det.roots.iter().copied().collect()
    };
    
    for addr in roots {
        let handle = addr as ValueHandle;
        if handle.is_null() {
            continue;
        }
        
        let mut det = detector().lock().unwrap();
        
        // Verify the value is still tracked (may have been freed via
        // notify_value_freed since we collected the roots snapshot).
        if !det.info.contains_key(&addr) {
            det.roots.remove(&addr);
            continue;
        }
        
        // Safe to dereference: if the value were freed, notify_value_freed
        // would have removed it from det.info under this same lock.
        let value = unsafe { &*handle };
        let refcount = value.refcount.load(Ordering::Relaxed);
        
        // Get the current color and buffered status
        let (current_color, _is_buffered) = {
            let info = det.info.entry(addr).or_default();
            (info.color, info.buffered)
        };
        
        if current_color == Color::Purple && refcount > 0 {
            mark_gray(handle, &mut det);
        } else {
            // Update info
            if let Some(info) = det.info.get_mut(&addr) {
                info.buffered = false;
            }
            det.roots.remove(&addr);
            if current_color == Color::Black && refcount == 0 {
                // Already freed, clean up
                det.info.remove(&addr);
            }
        }
    }
}

/// Recursively mark a subgraph as gray (potential garbage)
fn mark_gray(handle: ValueHandle, det: &mut CycleDetector) {
    if handle.is_null() {
        return;
    }
    
    let addr = handle as usize;
    let info = det.info.entry(addr).or_default();
    
    if info.color != Color::Gray {
        info.color = Color::Gray;
        
        // Get children after marking to avoid infinite recursion
        let children = get_children(handle);
        for child in children {
            mark_gray(child, det);
        }
    }
}

/// Phase 2: Scan for actual cycles
fn scan_roots() {
    let roots: Vec<usize> = {
        let det = detector().lock().unwrap();
        det.roots.iter().copied().collect()
    };
    
    for addr in roots {
        let handle = addr as ValueHandle;
        scan(handle);
    }
}

/// Scan a value to determine if it's garbage
fn scan(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let addr = handle as usize;
    
    // Hold lock while checking color AND reading refcount to prevent
    // use-after-free (the value can't be freed while we hold the lock
    // because notify_value_freed also takes this lock).
    let action = {
        let mut det = detector().lock().unwrap();
        let color = det.info.get(&addr).map(|i| i.color).unwrap_or(Color::Black);
        
        if color != Color::Gray {
            None // Not gray, nothing to do
        } else {
            // Safe to dereference: value is still tracked
            let value = unsafe { &*handle };
            let refcount = value.refcount.load(Ordering::Relaxed);
            
            if refcount > 0 {
                Some(true) // scan_black
            } else {
                // Mark white and collect children under lock
                if let Some(info) = det.info.get_mut(&addr) {
                    info.color = Color::White;
                }
                Some(false) // scan children
            }
        }
    };
    
    match action {
        Some(true) => scan_black(handle),
        Some(false) => {
            let children = get_children(handle);
            for child in children {
                scan(child);
            }
        }
        None => {}
    }
}

/// Mark a subgraph as reachable (black)
fn scan_black(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let addr = handle as usize;
    
    let (need_recurse, children) = {
        let mut det = detector().lock().unwrap();
        let info = det.info.entry(addr).or_default();
        
        if info.color != Color::Black {
            info.color = Color::Black;
            (true, get_children(handle))
        } else {
            (false, Vec::new())
        }
    }; // lock released before recursion
    
    if need_recurse {
        for child in children {
            scan_black(child);
        }
    }
}

/// Phase 3: Collect garbage cycles
fn collect_roots() {
    let roots: Vec<usize> = {
        let det = detector().lock().unwrap();
        det.roots.iter().copied().collect()
    };
    
    let mut garbage = Vec::new();
    
    {
        let mut det = detector().lock().unwrap();
        
        for addr in roots {
            let info = det.info.get_mut(&addr);
            if let Some(info) = info {
                info.buffered = false;
                if info.color == Color::White {
                    garbage.push(addr);
                }
            }
        }
        
        det.roots.clear();
        
        if !garbage.is_empty() {
            det.cycles_detected += 1;
            det.values_collected += garbage.len() as u64;
        }
    }
    
    // Collect garbage
    for addr in garbage {
        collect_white(addr as ValueHandle);
    }
}

/// Recursively collect white (garbage) values
fn collect_white(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let addr = handle as usize;
    
    let is_white = {
        let mut det = detector().lock().unwrap();
        let white = det.info.get(&addr).map(|i| i.color == Color::White).unwrap_or(false);
        if white {
            // Mark black to prevent re-collection, then get children while tracked
            if let Some(info) = det.info.get_mut(&addr) {
                info.color = Color::Black;
            }
        }
        white
    };
    
    if is_white {
        // get_children is safe here: the value is still tracked (Black now),
        // and notify_value_freed hasn't been called yet for this handle.
        let children = get_children(handle);
        for child in children {
            collect_white(child);
        }
        
        // Notify weak reference system before deallocation
        crate::weak_ref::notify_value_deallocated(handle);
        
        // Deallocate the heap data WITHOUT releasing child handles.
        // Children in the cycle are handled by their own collect_white call;
        // non-cycle children had their refcounts adjusted during the scan phase.
        // Using coral_value_release here would re-release children that are
        // already freed, causing use-after-free.
        unsafe {
            crate::drop_heap_value_for_gc(handle);
        }
        crate::dealloc_value_box(handle);
        
        // Clean up tracking info
        let mut det = detector().lock().unwrap();
        det.info.remove(&addr);
    }
}

/// Get cycle detection statistics.
pub fn cycle_stats() -> (u64, u64) {
    let det = detector().lock().unwrap();
    (det.cycles_detected, det.values_collected)
}

/// Clear cycle detection state (for testing).
pub fn reset_cycle_detector() {
    // M3.1: Clear thread-local buffer too
    LOCAL_ROOTS.with(|cell| cell.borrow_mut().clear());
    COLLECTION_PENDING.store(false, Ordering::Release);
    let mut det = detector().lock().unwrap();
    det.info.clear();
    det.roots.clear();
    det.young_roots.clear();
    det.old_roots.clear();
    det.collecting = false;
    det.current_epoch = 0;
    det.young_collections = 0;
    det.full_collections = 0;
}

// FFI exports

/// Run a cycle collection.
#[unsafe(no_mangle)]
pub extern "C" fn coral_collect_cycles() {
    collect_cycles();
}

/// Get the number of cycles detected so far.
#[unsafe(no_mangle)]
pub extern "C" fn coral_cycles_detected() -> u64 {
    let det = detector().lock().unwrap();
    det.cycles_detected
}

/// Get the number of values collected by cycle detection.
#[unsafe(no_mangle)]
pub extern "C" fn coral_cycle_values_collected() -> u64 {
    let det = detector().lock().unwrap();
    det.values_collected
}

/// Get the number of potential cycle roots currently buffered.
#[unsafe(no_mangle)]
pub extern "C" fn coral_cycle_roots_count() -> u64 {
    let det = detector().lock().unwrap();
    det.roots.len() as u64
}

/// Force a cycle collection run (for testing/debugging).
#[unsafe(no_mangle)]
pub extern "C" fn coral_force_cycle_collection() {
    collect_cycles();
}

/// Enable/disable automatic cycle collection during value release.
static AUTO_CYCLE_COLLECTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[unsafe(no_mangle)]
pub extern "C" fn coral_set_auto_cycle_collection(enabled: u8) {
    AUTO_CYCLE_COLLECTION.store(enabled != 0, std::sync::atomic::Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_get_auto_cycle_collection() -> u8 {
    if AUTO_CYCLE_COLLECTION.load(std::sync::atomic::Ordering::Relaxed) { 1 } else { 0 }
}

/// Check if automatic cycle collection is enabled.
pub fn auto_cycle_collection_enabled() -> bool {
    AUTO_CYCLE_COLLECTION.load(std::sync::atomic::Ordering::Relaxed)
}

/// M3.2: Get generational collection statistics.
pub fn generational_stats() -> (u64, u64) {
    let det = detector().lock().unwrap();
    (det.young_collections, det.full_collections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{coral_make_list, coral_make_number, coral_value_retain};

    #[test]
    fn test_cycle_stats_initial() {
        reset_cycle_detector();
        let (detected, collected) = cycle_stats();
        assert_eq!(detected, 0);
        assert_eq!(collected, 0);
    }

    #[test]
    fn test_non_container_not_tracked() {
        reset_cycle_detector();
        let num = coral_make_number(42.0);
        possible_root(num);
        // M3.1: Flush local buffer, then check global
        flush_local_roots();
        
        let det = detector().lock().unwrap();
        assert!(det.roots.is_empty());
        drop(det);
        
        unsafe { crate::coral_value_release(num); }
    }

    #[test]
    fn test_list_marked_as_possible_root() {
        reset_cycle_detector();
        let num = coral_make_number(42.0);
        let list = coral_make_list(&num as *const _, 1);
        
        possible_root(list);
        // M3.1: Flush local buffer so roots appear in global set
        flush_local_roots();
        
        {
            let det = detector().lock().unwrap();
            assert!(det.roots.contains(&(list as usize)));
        }
        
        unsafe {
            crate::coral_value_release(list);
            crate::coral_value_release(num);
        }
    }

    #[test]
    fn test_no_false_positives() {
        reset_cycle_detector();
        
        // Create a tree structure (no cycles)
        let root = coral_make_list(std::ptr::null(), 0);
        let child1 = coral_make_list(std::ptr::null(), 0);
        let child2 = coral_make_list(std::ptr::null(), 0);
        
        unsafe {
            // Make root point to children
            let root_list = &mut *((*root).payload.ptr as *mut crate::ListObject);
            root_list.items.push(child1);
            root_list.items.push(child2);
            coral_value_retain(child1);
            coral_value_retain(child2);
        }
        
        // Mark as possible roots and collect
        // M3.1: collect_cycles() flushes local buffers automatically
        possible_root(root);
        possible_root(child1);
        possible_root(child2);
        
        let initial_stats = cycle_stats();
        collect_cycles();
        let final_stats = cycle_stats();
        
        // Should not collect anything from a tree structure
        assert_eq!(final_stats.1, initial_stats.1, "Tree structure should not be collected as cycle");
        
        // Clean up
        unsafe {
            crate::coral_value_release(root);
            crate::coral_value_release(child1);
            crate::coral_value_release(child2);
        }
    }

    // M3.1: Test thread-local buffering
    #[test]
    fn test_thread_local_buffering() {
        reset_cycle_detector();
        
        // Create several lists and add as possible roots
        let lists: Vec<_> = (0..5).map(|_| coral_make_list(std::ptr::null(), 0)).collect();
        for &list in &lists {
            possible_root(list);
        }
        
        // Before flushing, global roots should be empty (under threshold)
        {
            let det = detector().lock().unwrap();
            assert!(det.roots.is_empty(), "Roots should be buffered locally, not in global set");
        }
        
        // After flushing, all 5 should appear
        flush_local_roots();
        {
            let det = detector().lock().unwrap();
            assert_eq!(det.roots.len(), 5, "All 5 roots should be in global set after flush");
        }
        
        // Clean up
        for list in lists {
            unsafe { crate::coral_value_release(list); }
        }
    }

    #[test]
    fn test_threshold_auto_flush() {
        reset_cycle_detector();
        
        // Create LOCAL_BUFFER_THRESHOLD lists to trigger auto-flush
        let lists: Vec<_> = (0..LOCAL_BUFFER_THRESHOLD)
            .map(|_| coral_make_list(std::ptr::null(), 0))
            .collect();
        for &list in &lists {
            possible_root(list);
        }
        
        // Should have auto-flushed when threshold was reached
        {
            let det = detector().lock().unwrap();
            assert_eq!(det.roots.len(), LOCAL_BUFFER_THRESHOLD,
                "Roots should auto-flush at threshold");
        }
        
        // Clean up
        for list in lists {
            unsafe { crate::coral_value_release(list); }
        }
    }

    #[test]
    fn test_collection_flushes_local_buffers() {
        reset_cycle_detector();
        
        // Add a root but don't manually flush
        let list = coral_make_list(std::ptr::null(), 0);
        possible_root(list);
        
        // collect_cycles() should flush and process all roots
        collect_cycles();
        
        // After collection, the root should have been processed
        // The local buffer should be empty
        LOCAL_ROOTS.with(|cell| {
            assert!(cell.borrow().is_empty(), "Local buffer should be empty after collection");
        });
        
        unsafe { crate::coral_value_release(list); }
    }

    // M3.2: Generational hypothesis / epoch tracking tests

    #[test]
    fn test_young_roots_tracked() {
        reset_cycle_detector();
        
        let list = coral_make_list(std::ptr::null(), 0);
        possible_root(list);
        flush_local_roots();
        
        {
            let det = detector().lock().unwrap();
            assert!(det.young_roots.contains(&(list as usize)),
                "New root should be in young generation");
            assert!(!det.old_roots.contains(&(list as usize)),
                "New root should NOT be in old generation");
        }
        
        unsafe { crate::coral_value_release(list); }
    }

    #[test]
    fn test_promotion_to_old() {
        reset_cycle_detector();
        
        // Create a list that will survive collection (has refcount > 0)
        let list = coral_make_list(std::ptr::null(), 0);
        unsafe { coral_value_retain(list); } // extra retain so it survives
        possible_root(list);
        
        // Run a young collection — root survives and gets promoted
        collect_cycles();
        
        {
            let det = detector().lock().unwrap();
            assert!(det.old_roots.contains(&(list as usize)),
                "Surviving root should be promoted to old generation");
            assert!(det.young_roots.is_empty(),
                "Young roots should be cleared after collection");
        }
        
        unsafe {
            crate::coral_value_release(list);
            crate::coral_value_release(list);
        }
    }

    #[test]
    fn test_generational_stats() {
        reset_cycle_detector();
        
        let (young, full) = generational_stats();
        assert_eq!(young, 0);
        assert_eq!(full, 0);
        
        // Run collections — first 4 are young, 5th is full
        for _ in 0..FULL_COLLECTION_INTERVAL {
            collect_cycles();
        }
        
        let (young, full) = generational_stats();
        assert_eq!(young, FULL_COLLECTION_INTERVAL - 1,
            "Should have {} young collections", FULL_COLLECTION_INTERVAL - 1);
        assert_eq!(full, 1,
            "Should have 1 full collection");
    }
}
