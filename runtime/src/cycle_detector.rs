//! Cycle detection for reference-counted values in Coral runtime.
//!
//! This module implements cycle detection using a mark-and-sweep approach
//! for container types (Lists, Maps, and Stores) that can form reference cycles.
//!
//! The algorithm is based on Bacon & Rajan's "Concurrent Cycle Collection" 
//! with simplifications for single-threaded collection phases.

use std::collections::{HashSet, HashMap, VecDeque};
use std::sync::{Mutex, atomic::Ordering};

use crate::{ValueHandle, ValueTag, Value};

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
    /// Number of times this value has been decremented since last collection
    buffered: bool,
}

impl Default for CycleInfo {
    fn default() -> Self {
        Self {
            color: Color::Black,
            buffered: false,
        }
    }
}

/// Global cycle detector state.
struct CycleDetector {
    /// Metadata for tracked values
    info: HashMap<usize, CycleInfo>,
    /// Possible cycle roots (values with decremented refcount)
    roots: HashSet<usize>,
    /// Whether cycle collection is currently in progress
    collecting: bool,
    /// Statistics
    cycles_detected: u64,
    values_collected: u64,
}

impl Default for CycleDetector {
    fn default() -> Self {
        Self {
            info: HashMap::new(),
            roots: HashSet::new(),
            collecting: false,
            cycles_detected: 0,
            values_collected: 0,
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
        Ok(ValueTag::List) | Ok(ValueTag::Map) | Ok(ValueTag::Store) | Ok(ValueTag::Tagged)
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
        _ => {}
    }
    
    children
}

/// Called when a container value's refcount is decremented.
/// This marks the value as a potential cycle root.
pub fn possible_root(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let value = unsafe { &*handle };
    if !is_container(value) {
        return;
    }
    
    let addr = handle as usize;
    
    if let Ok(mut det) = detector().lock() {
        let info = det.info.entry(addr).or_default();
        
        if info.color != Color::Purple {
            info.color = Color::Purple;
            if !info.buffered {
                info.buffered = true;
                det.roots.insert(addr);
            }
        }
    }
}

/// Run a cycle collection phase.
/// This should be called periodically or when memory pressure is high.
pub fn collect_cycles() {
    // Mark candidates
    mark_roots();
    
    // Scan for actual garbage
    scan_roots();
    
    // Collect garbage cycles
    collect_roots();
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
        
        let value = unsafe { &*handle };
        let refcount = value.refcount.load(Ordering::Relaxed);
        
        let mut det = detector().lock().unwrap();
        
        // Get the current color and buffered status
        let (current_color, is_buffered) = {
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
        
        // Note: We need to drop the lock before getting children to avoid deadlock
        // This is a simplified version - production code would need more care
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
    
    let color = {
        let det = detector().lock().unwrap();
        det.info.get(&addr).map(|i| i.color).unwrap_or(Color::Black)
    };
    
    if color == Color::Gray {
        let value = unsafe { &*handle };
        let refcount = value.refcount.load(Ordering::Relaxed);
        
        if refcount > 0 {
            // This value is still reachable from outside
            scan_black(handle);
        } else {
            // This value is only referenced by the cycle
            let mut det = detector().lock().unwrap();
            if let Some(info) = det.info.get_mut(&addr) {
                info.color = Color::White;
            }
            
            let children = get_children(handle);
            for child in children {
                scan(child);
            }
        }
    }
}

/// Mark a subgraph as reachable (black)
fn scan_black(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let addr = handle as usize;
    
    let mut det = detector().lock().unwrap();
    let info = det.info.entry(addr).or_default();
    
    if info.color != Color::Black {
        info.color = Color::Black;
        
        let children = get_children(handle);
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
        let det = detector().lock().unwrap();
        det.info.get(&addr).map(|i| i.color == Color::White).unwrap_or(false)
    };
    
    if is_white {
        {
            let mut det = detector().lock().unwrap();
            if let Some(info) = det.info.get_mut(&addr) {
                info.color = Color::Black;
            }
        }
        
        let children = get_children(handle);
        for child in children {
            collect_white(child);
        }
        
        // Actually free the value
        // Note: In production, we'd need to be more careful about the order
        // of releases to avoid double-frees
        unsafe {
            crate::coral_value_release(handle);
        }
        
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
    let mut det = detector().lock().unwrap();
    det.info.clear();
    det.roots.clear();
    det.collecting = false;
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
        
        let det = detector().lock().unwrap();
        assert!(det.roots.is_empty());
        
        unsafe { crate::coral_value_release(num); }
    }

    #[test]
    fn test_list_marked_as_possible_root() {
        reset_cycle_detector();
        let num = coral_make_number(42.0);
        let list = coral_make_list(&num as *const _, 1);
        
        possible_root(list);
        
        {
            let det = detector().lock().unwrap();
            assert!(det.roots.contains(&(list as usize)));
        }
        
        unsafe {
            crate::coral_value_release(list);
            crate::coral_value_release(num);
        }
    }
}
