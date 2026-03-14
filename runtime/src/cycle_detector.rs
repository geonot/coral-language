use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, atomic::Ordering};

use crate::{Value, ValueHandle, ValueTag};

const LOCAL_BUFFER_THRESHOLD: usize = 64;

static COLLECTION_PENDING: AtomicBool = AtomicBool::new(false);

thread_local! {

    static LOCAL_ROOTS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(LOCAL_BUFFER_THRESHOLD));
}

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

                let value = unsafe { &*handle };
                if !is_container(value) {
                    continue;
                }

                let epoch = det.current_epoch;
                let info = det.info.entry(addr).or_default();
                if info.color != Color::Purple {
                    info.color = Color::Purple;
                    info.birth_epoch = epoch;
                    if !info.buffered {
                        info.buffered = true;
                    }
                }

                det.young_roots.insert(addr);
                det.roots.insert(addr);
            }
        }
    });
}

fn flush_all_thread_local_roots() {
    COLLECTION_PENDING.store(true, Ordering::Release);

    flush_local_roots();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Color {
    Black,

    Gray,

    White,

    Purple,
}

struct CycleInfo {
    color: Color,

    buffered: bool,

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

const FULL_COLLECTION_INTERVAL: u64 = 5;

struct CycleDetector {
    info: HashMap<usize, CycleInfo>,

    young_roots: HashSet<usize>,

    old_roots: HashSet<usize>,

    roots: HashSet<usize>,

    collecting: bool,

    cycles_detected: u64,
    values_collected: u64,

    current_epoch: u64,

    young_collections: u64,

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

fn is_container(value: &Value) -> bool {
    matches!(
        ValueTag::try_from(value.tag),
        Ok(ValueTag::List)
            | Ok(ValueTag::Map)
            | Ok(ValueTag::Store)
            | Ok(ValueTag::Tagged)
            | Ok(ValueTag::Closure)
    )
}

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

        Ok(ValueTag::Store) => {}
        Ok(ValueTag::Closure) => {
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

pub fn possible_root(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }

    let value = unsafe { &*handle };
    if !is_container(value) {
        return;
    }

    let addr = handle as usize;

    if COLLECTION_PENDING.load(Ordering::Acquire) {
        flush_local_roots();
        COLLECTION_PENDING.store(false, Ordering::Release);
    }

    LOCAL_ROOTS.with(|cell| {
        let mut local = cell.borrow_mut();
        local.push(addr);
        if local.len() >= LOCAL_BUFFER_THRESHOLD {
            drop(local);
            flush_local_roots();
        }
    });
}

pub fn notify_value_freed(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    let addr = handle as usize;
    let mut det = detector().lock().unwrap();
    det.roots.remove(&addr);

    det.young_roots.remove(&addr);
    det.old_roots.remove(&addr);
    det.info.remove(&addr);
}

pub fn collect_cycles() {
    flush_all_thread_local_roots();

    let is_full_collection;
    {
        let mut det = detector().lock().unwrap();
        if det.collecting {
            return;
        }
        det.collecting = true;
        det.current_epoch += 1;

        is_full_collection = (det.current_epoch % FULL_COLLECTION_INTERVAL) == 0;

        if is_full_collection {
            det.roots = det.young_roots.union(&det.old_roots).copied().collect();
            det.full_collections += 1;
        } else {
            det.roots = det.young_roots.clone();
            det.young_collections += 1;
        }
    }

    mark_roots();

    scan_roots();

    collect_roots();

    {
        let mut det = detector().lock().unwrap();

        let surviving_young: Vec<usize> = det
            .young_roots
            .iter()
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

        if !det.info.contains_key(&addr) {
            det.roots.remove(&addr);
            continue;
        }

        let value = unsafe { &*handle };
        let refcount = value.refcount.load(Ordering::Relaxed);

        let (current_color, _is_buffered) = {
            let info = det.info.entry(addr).or_default();
            (info.color, info.buffered)
        };

        if current_color == Color::Purple && refcount > 0 {
            mark_gray(handle, &mut det);
        } else {
            if let Some(info) = det.info.get_mut(&addr) {
                info.buffered = false;
            }
            det.roots.remove(&addr);
            if current_color == Color::Black && refcount == 0 {
                det.info.remove(&addr);
            }
        }
    }
}

fn mark_gray(handle: ValueHandle, det: &mut CycleDetector) {
    if handle.is_null() {
        return;
    }

    let addr = handle as usize;
    let info = det.info.entry(addr).or_default();

    if info.color != Color::Gray {
        info.color = Color::Gray;

        let children = get_children(handle);
        for child in children {
            mark_gray(child, det);
        }
    }
}

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

fn scan(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }

    let addr = handle as usize;

    let action = {
        let mut det = detector().lock().unwrap();
        let color = det.info.get(&addr).map(|i| i.color).unwrap_or(Color::Black);

        if color != Color::Gray {
            None
        } else {
            let value = unsafe { &*handle };
            let refcount = value.refcount.load(Ordering::Relaxed);

            if refcount > 0 {
                Some(true)
            } else {
                if let Some(info) = det.info.get_mut(&addr) {
                    info.color = Color::White;
                }
                Some(false)
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
    };

    if need_recurse {
        for child in children {
            scan_black(child);
        }
    }
}

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

    for addr in garbage {
        collect_white(addr as ValueHandle);
    }
}

fn collect_white(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }

    let addr = handle as usize;

    let is_white = {
        let mut det = detector().lock().unwrap();
        let white = det
            .info
            .get(&addr)
            .map(|i| i.color == Color::White)
            .unwrap_or(false);
        if white {
            if let Some(info) = det.info.get_mut(&addr) {
                info.color = Color::Black;
            }
        }
        white
    };

    if is_white {
        let children = get_children(handle);
        for child in children {
            collect_white(child);
        }

        crate::weak_ref::notify_value_deallocated(handle);

        unsafe {
            crate::drop_heap_value_for_gc(handle);
        }
        crate::dealloc_value_box(handle);

        let mut det = detector().lock().unwrap();
        det.info.remove(&addr);
    }
}

pub fn cycle_stats() -> (u64, u64) {
    let det = detector().lock().unwrap();
    (det.cycles_detected, det.values_collected)
}

pub fn reset_cycle_detector() {
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

#[unsafe(no_mangle)]
pub extern "C" fn coral_collect_cycles() {
    collect_cycles();
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_cycles_detected() -> u64 {
    let det = detector().lock().unwrap();
    det.cycles_detected
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_cycle_values_collected() -> u64 {
    let det = detector().lock().unwrap();
    det.values_collected
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_cycle_roots_count() -> u64 {
    let det = detector().lock().unwrap();
    det.roots.len() as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_force_cycle_collection() {
    collect_cycles();
}

static AUTO_CYCLE_COLLECTION: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

#[unsafe(no_mangle)]
pub extern "C" fn coral_set_auto_cycle_collection(enabled: u8) {
    AUTO_CYCLE_COLLECTION.store(enabled != 0, std::sync::atomic::Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_get_auto_cycle_collection() -> u8 {
    if AUTO_CYCLE_COLLECTION.load(std::sync::atomic::Ordering::Relaxed) {
        1
    } else {
        0
    }
}

pub fn auto_cycle_collection_enabled() -> bool {
    AUTO_CYCLE_COLLECTION.load(std::sync::atomic::Ordering::Relaxed)
}

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

        flush_local_roots();

        let det = detector().lock().unwrap();
        assert!(det.roots.is_empty());
        drop(det);

        unsafe {
            crate::coral_value_release(num);
        }
    }

    #[test]
    fn test_list_marked_as_possible_root() {
        reset_cycle_detector();
        let num = coral_make_number(42.0);
        let list = coral_make_list(&num as *const _, 1);

        possible_root(list);

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

        let root = coral_make_list(std::ptr::null(), 0);
        let child1 = coral_make_list(std::ptr::null(), 0);
        let child2 = coral_make_list(std::ptr::null(), 0);

        unsafe {
            let root_list = &mut *((*root).payload.ptr as *mut crate::ListObject);
            root_list.items.push(child1);
            root_list.items.push(child2);
            coral_value_retain(child1);
            coral_value_retain(child2);
        }

        possible_root(root);
        possible_root(child1);
        possible_root(child2);

        let initial_stats = cycle_stats();
        collect_cycles();
        let final_stats = cycle_stats();

        assert_eq!(
            final_stats.1, initial_stats.1,
            "Tree structure should not be collected as cycle"
        );

        unsafe {
            crate::coral_value_release(root);
            crate::coral_value_release(child1);
            crate::coral_value_release(child2);
        }
    }

    #[test]
    fn test_thread_local_buffering() {
        reset_cycle_detector();

        let lists: Vec<_> = (0..5)
            .map(|_| coral_make_list(std::ptr::null(), 0))
            .collect();
        for &list in &lists {
            possible_root(list);
        }

        {
            let det = detector().lock().unwrap();
            assert!(
                det.roots.is_empty(),
                "Roots should be buffered locally, not in global set"
            );
        }

        flush_local_roots();
        {
            let det = detector().lock().unwrap();
            assert_eq!(
                det.roots.len(),
                5,
                "All 5 roots should be in global set after flush"
            );
        }

        for list in lists {
            unsafe {
                crate::coral_value_release(list);
            }
        }
    }

    #[test]
    fn test_threshold_auto_flush() {
        reset_cycle_detector();

        let lists: Vec<_> = (0..LOCAL_BUFFER_THRESHOLD)
            .map(|_| coral_make_list(std::ptr::null(), 0))
            .collect();
        for &list in &lists {
            possible_root(list);
        }

        {
            let det = detector().lock().unwrap();
            assert_eq!(
                det.roots.len(),
                LOCAL_BUFFER_THRESHOLD,
                "Roots should auto-flush at threshold"
            );
        }

        for list in lists {
            unsafe {
                crate::coral_value_release(list);
            }
        }
    }

    #[test]
    fn test_collection_flushes_local_buffers() {
        reset_cycle_detector();

        let list = coral_make_list(std::ptr::null(), 0);
        possible_root(list);

        collect_cycles();

        LOCAL_ROOTS.with(|cell| {
            assert!(
                cell.borrow().is_empty(),
                "Local buffer should be empty after collection"
            );
        });

        unsafe {
            crate::coral_value_release(list);
        }
    }

    #[test]
    fn test_young_roots_tracked() {
        reset_cycle_detector();

        let list = coral_make_list(std::ptr::null(), 0);
        possible_root(list);
        flush_local_roots();

        {
            let det = detector().lock().unwrap();
            assert!(
                det.young_roots.contains(&(list as usize)),
                "New root should be in young generation"
            );
            assert!(
                !det.old_roots.contains(&(list as usize)),
                "New root should NOT be in old generation"
            );
        }

        unsafe {
            crate::coral_value_release(list);
        }
    }

    #[test]
    fn test_promotion_to_old() {
        reset_cycle_detector();

        let list = coral_make_list(std::ptr::null(), 0);
        unsafe {
            coral_value_retain(list);
        }
        possible_root(list);

        collect_cycles();

        {
            let det = detector().lock().unwrap();
            assert!(
                det.old_roots.contains(&(list as usize)),
                "Surviving root should be promoted to old generation"
            );
            assert!(
                det.young_roots.is_empty(),
                "Young roots should be cleared after collection"
            );
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

        for _ in 0..FULL_COLLECTION_INTERVAL {
            collect_cycles();
        }

        let (young, full) = generational_stats();
        assert_eq!(
            young,
            FULL_COLLECTION_INTERVAL - 1,
            "Should have {} young collections",
            FULL_COLLECTION_INTERVAL - 1
        );
        assert_eq!(full, 1, "Should have 1 full collection");
    }
}
