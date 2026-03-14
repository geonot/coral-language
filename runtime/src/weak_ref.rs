use std::ptr;
use std::sync::atomic::Ordering;

use crate::{FLAG_HAS_WEAK_REFS, ValueHandle};

#[repr(C)]
pub struct WeakRef {
    target: ValueHandle,

    epoch: u16,
}

impl WeakRef {
    pub fn new(target: ValueHandle) -> Option<Self> {
        if target.is_null() {
            return None;
        }

        unsafe {
            (*target).flags |= FLAG_HAS_WEAK_REFS;
            let epoch = (*target).epoch;
            Some(Self { target, epoch })
        }
    }

    pub fn upgrade(&self) -> Option<ValueHandle> {
        if self.target.is_null() {
            return None;
        }

        let current_epoch = unsafe { (*self.target).epoch };
        if current_epoch != self.epoch {
            return None;
        }

        unsafe {
            crate::coral_value_retain(self.target);
        }
        Some(self.target)
    }

    pub fn is_alive(&self) -> bool {
        if self.target.is_null() {
            return false;
        }

        let current_epoch = unsafe { (*self.target).epoch };
        current_epoch == self.epoch
    }

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

impl Drop for WeakRef {
    fn drop(&mut self) {}
}

pub fn notify_value_deallocated(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        (*handle).epoch = (*handle).epoch.wrapping_add(1);
    }
}

pub fn weak_ref_count(_handle: ValueHandle) -> u64 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_weak_ref(target: ValueHandle) -> *mut WeakRef {
    match WeakRef::new(target) {
        Some(weak) => Box::into_raw(Box::new(weak)),
        None => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_upgrade(weak: *mut WeakRef) -> ValueHandle {
    if weak.is_null() {
        return ptr::null_mut();
    }
    let weak_ref = unsafe { &*weak };
    weak_ref.upgrade().unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_is_alive(weak: *mut WeakRef) -> u8 {
    if weak.is_null() {
        return 0;
    }
    let weak_ref = unsafe { &*weak };
    if weak_ref.is_alive() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_weak_ref_release(weak: *mut WeakRef) {
    if weak.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(weak));
    }
}

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

        unsafe {
            coral_value_release(value);
        }
    }

    #[test]
    fn test_weak_ref_upgrade_alive() {
        let value = coral_make_number(42.0);
        let weak = WeakRef::new(value).unwrap();

        let upgraded = weak.upgrade();
        assert!(upgraded.is_some());

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

        notify_value_deallocated(value);

        assert!(!weak.is_alive(), "weak ref must be stale after epoch bump");
        assert!(
            weak.upgrade().is_none(),
            "upgrade must fail after deallocation"
        );

        unsafe {
            coral_value_release(value);
        }
    }

    #[test]
    fn test_weak_ref_clone_both_invalidated() {
        let value = coral_make_number(7.0);
        let original = WeakRef::new(value).unwrap();
        let cloned = original.clone();

        assert!(original.is_alive());
        assert!(cloned.is_alive());

        notify_value_deallocated(value);

        assert!(!original.is_alive());
        assert!(!cloned.is_alive());

        drop(original);
        drop(cloned);
        unsafe {
            coral_value_release(value);
        }
    }

    #[test]
    fn test_weak_ref_epoch_no_lock_required() {
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
        unsafe {
            coral_value_release(value);
        }
    }

    #[test]
    fn test_flag_has_weak_refs_set() {
        let value = coral_make_number(1.0);

        let flags_before = unsafe { (*value).flags };
        assert_eq!(flags_before & FLAG_HAS_WEAK_REFS, 0);

        let _weak = WeakRef::new(value).unwrap();

        let flags_after = unsafe { (*value).flags };
        assert_ne!(
            flags_after & FLAG_HAS_WEAK_REFS,
            0,
            "FLAG_HAS_WEAK_REFS must be set after creating a weak ref"
        );

        unsafe {
            coral_value_release(value);
        }
    }
}
