//! Weak reference implementation for Coral runtime.
//!
//! Weak references allow breaking reference cycles in the reference-counted
//! memory management system. A weak reference does not keep the referent alive,
//! but can be upgraded to a strong reference if the referent is still alive.

use std::sync::atomic::{AtomicU64, AtomicPtr, Ordering};
use std::ptr;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::ValueHandle;

/// Global registry of weak reference targets.
/// Maps object addresses to their weak reference metadata.
static WEAK_REF_REGISTRY: std::sync::OnceLock<Mutex<WeakRefRegistry>> = std::sync::OnceLock::new();

/// Metadata for weak reference tracking.
struct WeakRefRegistry {
    /// Maps object address to weak ref count
    targets: HashMap<usize, WeakTarget>,
    /// Next unique ID for weak references
    next_id: u64,
}

/// Information about a weakly-referenced target.
struct WeakTarget {
    /// Number of weak references to this target
    weak_count: u64,
    /// Whether the target is still alive
    alive: bool,
}

impl Default for WeakRefRegistry {
    fn default() -> Self {
        Self {
            targets: HashMap::new(),
            next_id: 1,
        }
    }
}

fn registry() -> &'static Mutex<WeakRefRegistry> {
    WEAK_REF_REGISTRY.get_or_init(|| Mutex::new(WeakRefRegistry::default()))
}

/// A weak reference handle.
/// 
/// This can be upgraded to a strong reference if the target is still alive.
/// The weak reference does not prevent the target from being deallocated.
#[repr(C)]
pub struct WeakRef {
    /// The address of the target value (stored as usize for FFI safety)
    target_addr: AtomicU64,
    /// Unique ID for this weak reference
    id: u64,
}

impl WeakRef {
    /// Create a new weak reference to the given value.
    /// Returns None if the value is null.
    pub fn new(target: ValueHandle) -> Option<Self> {
        if target.is_null() {
            return None;
        }
        
        let addr = target as usize;
        let mut reg = registry().lock().ok()?;
        
        // Update or create weak target entry
        let entry = reg.targets.entry(addr).or_insert(WeakTarget {
            weak_count: 0,
            alive: true,
        });
        entry.weak_count += 1;
        
        let id = reg.next_id;
        reg.next_id += 1;
        
        Some(Self {
            target_addr: AtomicU64::new(addr as u64),
            id,
        })
    }
    
    /// Try to upgrade this weak reference to a strong reference.
    /// Returns None if the target has been deallocated.
    pub fn upgrade(&self) -> Option<ValueHandle> {
        let addr = self.target_addr.load(Ordering::Acquire);
        if addr == 0 {
            return None;
        }
        
        let reg = registry().lock().ok()?;
        let target = reg.targets.get(&(addr as usize))?;
        
        if !target.alive {
            return None;
        }
        
        let handle = addr as ValueHandle;
        
        // Retain the value to create a strong reference
        unsafe {
            crate::coral_value_retain(handle);
        }
        
        Some(handle)
    }
    
    /// Check if the target is still alive without upgrading.
    pub fn is_alive(&self) -> bool {
        let addr = self.target_addr.load(Ordering::Acquire);
        if addr == 0 {
            return false;
        }
        
        if let Ok(reg) = registry().lock() {
            reg.targets.get(&(addr as usize))
                .map(|t| t.alive)
                .unwrap_or(false)
        } else {
            false
        }
    }
    
    /// Get the unique ID of this weak reference.
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Clone for WeakRef {
    fn clone(&self) -> Self {
        let addr = self.target_addr.load(Ordering::Acquire);
        
        // Each clone gets its own unique ID and increments weak_count
        let new_id = if addr != 0 {
            if let Ok(mut reg) = registry().lock() {
                if let Some(target) = reg.targets.get_mut(&(addr as usize)) {
                    target.weak_count += 1;
                }
                let id = reg.next_id;
                reg.next_id += 1;
                id
            } else {
                0
            }
        } else {
            0
        };
        
        Self {
            target_addr: AtomicU64::new(addr),
            id: new_id,
        }
    }
}

impl Drop for WeakRef {
    fn drop(&mut self) {
        let addr = self.target_addr.load(Ordering::Acquire);
        if addr == 0 {
            return;
        }
        
        if let Ok(mut reg) = registry().lock() {
            if let Some(target) = reg.targets.get_mut(&(addr as usize)) {
                target.weak_count = target.weak_count.saturating_sub(1);
                
                // Clean up entry if no more weak refs and target is dead
                if target.weak_count == 0 && !target.alive {
                    reg.targets.remove(&(addr as usize));
                }
            }
        }
    }
}

/// Notify the weak reference system that a value is being deallocated.
/// This should be called before actually freeing the memory.
pub fn notify_value_deallocated(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    
    let addr = handle as usize;
    
    if let Ok(mut reg) = registry().lock() {
        if let Some(target) = reg.targets.get_mut(&addr) {
            target.alive = false;
            
            // If no weak refs, remove the entry entirely
            if target.weak_count == 0 {
                reg.targets.remove(&addr);
            }
        }
    }
}

/// Get the number of weak references to a value.
pub fn weak_ref_count(handle: ValueHandle) -> u64 {
    if handle.is_null() {
        return 0;
    }
    
    let addr = handle as usize;
    
    if let Ok(reg) = registry().lock() {
        reg.targets.get(&addr).map(|t| t.weak_count).unwrap_or(0)
    } else {
        0
    }
}

// FFI exports for the Coral language

/// Create a weak reference to a value.
/// Returns a pointer to the WeakRef, or null if creation failed.
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_weak_ref(target: ValueHandle) -> *mut WeakRef {
    match WeakRef::new(target) {
        Some(weak) => Box::into_raw(Box::new(weak)),
        None => ptr::null_mut(),
    }
}

/// Try to upgrade a weak reference to a strong reference.
/// Returns the value handle if successful, or null if the target was deallocated.
#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_upgrade(weak: *mut WeakRef) -> ValueHandle {
    if weak.is_null() {
        return ptr::null_mut();
    }
    
    let weak_ref = unsafe { &*weak };
    weak_ref.upgrade().unwrap_or(ptr::null_mut())
}

/// Check if a weak reference's target is still alive.
/// Returns 1 if alive, 0 if dead.
#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_is_alive(weak: *mut WeakRef) -> u8 {
    if weak.is_null() {
        return 0;
    }
    
    let weak_ref = unsafe { &*weak };
    if weak_ref.is_alive() { 1 } else { 0 }
}

/// Release a weak reference.
#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_release(weak: *mut WeakRef) {
    if weak.is_null() {
        return;
    }
    
    unsafe {
        drop(Box::from_raw(weak));
    }
}

/// Clone a weak reference.
/// Returns a new weak reference pointing to the same target.
#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_clone(weak: *mut WeakRef) -> *mut WeakRef {
    if weak.is_null() {
        return ptr::null_mut();
    }
    
    let weak_ref = unsafe { &*weak };
    Box::into_raw(Box::new(weak_ref.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{coral_make_number, coral_value_release};
    
    #[test]
    fn test_weak_ref_creation() {
        let value = coral_make_number(42.0);
        let weak = WeakRef::new(value);
        
        assert!(weak.is_some());
        let weak = weak.unwrap();
        assert!(weak.is_alive());
        
        // Clean up
        unsafe { coral_value_release(value); }
    }
    
    #[test]
    fn test_weak_ref_upgrade_alive() {
        let value = coral_make_number(42.0);
        let weak = WeakRef::new(value).unwrap();
        
        let upgraded = weak.upgrade();
        assert!(upgraded.is_some());
        
        // Clean up both the original and upgraded references
        unsafe {
            if let Some(up) = upgraded {
                coral_value_release(up);
            }
            coral_value_release(value);
        }
    }
    
    #[test]
    fn test_weak_ref_null_input() {
        let weak = WeakRef::new(ptr::null_mut());
        assert!(weak.is_none());
    }
}
