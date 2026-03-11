//! Epoch-based weak reference implementation for Coral runtime (M3.5).
//!
//! Instead of maintaining a global `Mutex<HashMap>` registry, each `Value` now
//! carries a 16-bit *epoch* counter.  A `WeakRef` stores `(ptr, epoch_snapshot)`.
//! Validity is checked by comparing the stored epoch against the current value
//! of `(*ptr).epoch` — a single memory load + compare, with **no locking**.
//!
//! Values that have (or ever had) a weak reference get `FLAG_HAS_WEAK_REFS` set
//! on their `flags` field.  The runtime guarantees such values are always pooled
//! rather than freed, so the epoch memory remains accessible for stale WeakRefs.

use std::ptr;
use std::sync::atomic::Ordering;

use crate::{ValueHandle, FLAG_HAS_WEAK_REFS};

/// A weak reference handle.
///
/// The weak reference stores a raw pointer and the epoch of the target at
/// creation time.  Upgrading succeeds only when the target has not been
/// deallocated (i.e. its epoch has not been incremented).
#[repr(C)]
pub struct WeakRef {
    /// Raw pointer to the target `Value`.
    target: ValueHandle,
    /// Epoch snapshot taken when the weak reference was created.
    epoch: u16,
}

impl WeakRef {
    /// Create a new weak reference to the given value.
    /// Returns `None` if the value is null.
    pub fn new(target: ValueHandle) -> Option<Self> {
        if target.is_null() {
            return None;
        }

        unsafe {
            // Mark the value so it will never be freed (always pooled).
            (*target).flags |= FLAG_HAS_WEAK_REFS;
            let epoch = (*target).epoch;
            Some(Self { target, epoch })
        }
    }

    /// Try to upgrade this weak reference to a strong reference.
    /// Returns `None` if the target has been deallocated.
    pub fn upgrade(&self) -> Option<ValueHandle> {
        if self.target.is_null() {
            return None;
        }

        // Safety: FLAG_HAS_WEAK_REFS guarantees the memory is still accessible.
        let current_epoch = unsafe { (*self.target).epoch };
        if current_epoch != self.epoch {
            return None;
        }

        // Still alive — create a strong reference via retain.
        unsafe {
            crate::coral_value_retain(self.target);
        }
        Some(self.target)
    }

    /// Check if the target is still alive without upgrading.
    pub fn is_alive(&self) -> bool {
        if self.target.is_null() {
            return false;
        }
        // Safety: FLAG_HAS_WEAK_REFS guarantees the memory is still accessible.
        let current_epoch = unsafe { (*self.target).epoch };
        current_epoch == self.epoch
    }

    /// Get the stored epoch snapshot.
    pub fn epoch(&self) -> u16 {
        self.epoch
    }
}

impl Clone for WeakRef {
    fn clone(&self) -> Self {
        Self {
            target: self.target,
            epoch: self.epoch,
        }
    }
}

// Drop is a no-op — no registry to update.
impl Drop for WeakRef {
    fn drop(&mut self) {
        // Nothing to do. The epoch-based scheme requires no bookkeeping on drop.
    }
}

/// Notify the weak reference system that a value is being deallocated.
/// Bumps the epoch counter so that all existing weak references become stale.
///
/// Called from the release path **before** the value's payload is torn down.
pub fn notify_value_deallocated(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        // Wrapping increment — u16 overflow is fine (see module doc).
        (*handle).epoch = (*handle).epoch.wrapping_add(1);
    }
}

/// Get the number of weak references to a value.
///
/// With the epoch-based scheme there is no central registry to query, so this
/// always returns 0.  Kept for API compatibility.
pub fn weak_ref_count(_handle: ValueHandle) -> u64 {
    0
}

// ── FFI exports ──────────────────────────────────────────────────────────────

/// Create a weak reference to a value.
/// Returns a pointer to the `WeakRef`, or null if creation failed.
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

    #[test]
    fn test_weak_ref_invalidated_after_deallocation() {
        let value = coral_make_number(99.0);
        let weak = WeakRef::new(value).unwrap();
        assert!(weak.is_alive());

        // Simulate deallocation — bumps epoch
        notify_value_deallocated(value);

        assert!(!weak.is_alive(), "weak ref must be stale after epoch bump");
        assert!(weak.upgrade().is_none(), "upgrade must fail after deallocation");

        // Memory is still pooled (FLAG_HAS_WEAK_REFS), so we can safely
        // release the value.
        unsafe { coral_value_release(value); }
    }

    #[test]
    fn test_weak_ref_clone_both_invalidated() {
        let value = coral_make_number(7.0);
        let original = WeakRef::new(value).unwrap();
        let cloned = original.clone();

        assert!(original.is_alive());
        assert!(cloned.is_alive());

        // Bump epoch — invalidates both
        notify_value_deallocated(value);

        assert!(!original.is_alive());
        assert!(!cloned.is_alive());

        drop(original);
        drop(cloned);
        unsafe { coral_value_release(value); }
    }

    #[test]
    fn test_weak_ref_epoch_no_lock_required() {
        // Epoch-based: creating, checking, and dropping weak refs requires no
        // mutex and no HashMap.  This test just exercises the path to confirm
        // no deadlock or panic happens.
        let value = coral_make_number(3.14);
        let w1 = WeakRef::new(value).unwrap();
        let w2 = w1.clone();
        let w3 = w2.clone();

        assert!(w1.is_alive());
        assert!(w2.is_alive());
        assert!(w3.is_alive());

        notify_value_deallocated(value);

        assert!(!w1.is_alive());
        assert!(!w2.is_alive());
        assert!(!w3.is_alive());

        drop(w1);
        drop(w2);
        drop(w3);
        unsafe { coral_value_release(value); }
    }

    #[test]
    fn test_flag_has_weak_refs_set() {
        let value = coral_make_number(1.0);
        // Before creating a weak ref, flag should be 0
        let flags_before = unsafe { (*value).flags };
        assert_eq!(flags_before & FLAG_HAS_WEAK_REFS, 0);

        let _weak = WeakRef::new(value).unwrap();

        let flags_after = unsafe { (*value).flags };
        assert_ne!(flags_after & FLAG_HAS_WEAK_REFS, 0,
            "FLAG_HAS_WEAK_REFS must be set after creating a weak ref");

        unsafe { coral_value_release(value); }
    }
}
