use crate::{Value, ValueHandle, ValueTag, alloc_value, nanbox::NanBoxedValue};
use std::sync::atomic::Ordering;

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_number(value: f64) -> u64 {
    NanBoxedValue::from_number(value).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_bool(value: u8) -> u64 {
    NanBoxedValue::from_bool(value != 0).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_unit() -> u64 {
    NanBoxedValue::unit().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_none() -> u64 {
    NanBoxedValue::none().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_string(ptr: *const u8, len: usize) -> u64 {
    let handle = crate::coral_make_string(ptr, len);
    NanBoxedValue::from_heap_ptr(handle).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_list(items: *const u64, len: usize) -> u64 {
    let mut handles: Vec<ValueHandle> = Vec::with_capacity(len);
    if len > 0 && !items.is_null() {
        let slice = unsafe { std::slice::from_raw_parts(items, len) };
        for &bits in slice {
            handles.push(nb_to_handle(bits));
        }
    }
    let handle = crate::coral_make_list(handles.as_ptr(), handles.len());
    NanBoxedValue::from_heap_ptr(handle).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_make_map(entries: *const crate::MapEntry, len: usize) -> u64 {
    let handle = crate::coral_make_map(entries, len);
    NanBoxedValue::from_heap_ptr(handle).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_as_number(value: u64) -> f64 {
    NanBoxedValue::from_bits(value).as_number()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_as_bool(value: u64) -> u8 {
    if NanBoxedValue::from_bits(value).as_bool() {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_tag(value: u64) -> u8 {
    let v = NanBoxedValue::from_bits(value);
    if v.is_number() {
        ValueTag::Number as u8
    } else if v.is_bool() {
        ValueTag::Bool as u8
    } else if v.is_unit() {
        ValueTag::Unit as u8
    } else if v.is_none() {
        ValueTag::Unit as u8
    } else if v.is_heap_ptr() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            unsafe { (*ptr).tag }
        } else {
            ValueTag::Unit as u8
        }
    } else {
        ValueTag::Unit as u8
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_is_truthy(value: u64) -> u8 {
    if NanBoxedValue::from_bits(value).is_truthy() {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_is_err(value: u64) -> u8 {
    let v = NanBoxedValue::from_bits(value);
    if v.is_error() {
        return 1;
    }

    if v.is_heap_ptr() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            let val = unsafe { &*ptr };
            if val.is_err() {
                return 1;
            }
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_is_absent(value: u64) -> u8 {
    let v = NanBoxedValue::from_bits(value);
    if v.is_none() {
        return 1;
    }

    if v.is_heap_ptr() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            let val = unsafe { &*ptr };
            if val.flags & crate::FLAG_ABSENT != 0 {
                return 1;
            }
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_nb_retain(value: u64) {
    let v = NanBoxedValue::from_bits(value);
    if v.is_heap_ptr() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            crate::coral_value_retain(ptr);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_nb_release(value: u64) {
    let v = NanBoxedValue::from_bits(value);
    if v.is_heap_ptr() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            crate::coral_value_release(ptr);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_from_handle(handle: ValueHandle) -> u64 {
    if handle.is_null() {
        return NanBoxedValue::none().to_bits();
    }
    let value = unsafe { &*handle };
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::Number) => {
            let n = unsafe { value.payload.number };

            let result = NanBoxedValue::from_number(n).to_bits();
            unsafe {
                crate::coral_value_release(handle);
            }
            result
        }
        Ok(ValueTag::Bool) => {
            let b = unsafe { value.payload.inline[0] & 1 } != 0;
            let result = NanBoxedValue::from_bool(b).to_bits();
            unsafe {
                crate::coral_value_release(handle);
            }
            result
        }
        Ok(ValueTag::Unit) => {
            if value.flags & crate::FLAG_ERR != 0 {
                NanBoxedValue::from_heap_ptr(handle).to_bits()
            } else {
                let is_absent = value.flags & crate::FLAG_ABSENT != 0;
                let result = if is_absent {
                    NanBoxedValue::none().to_bits()
                } else {
                    NanBoxedValue::unit().to_bits()
                };
                unsafe {
                    crate::coral_value_release(handle);
                }
                result
            }
        }
        _ => {
            if value.is_err() {
                NanBoxedValue::from_heap_ptr(handle).to_bits()
            } else {
                NanBoxedValue::from_heap_ptr(handle).to_bits()
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_to_handle(value: u64) -> ValueHandle {
    nb_to_handle(value)
}

pub(crate) fn nb_to_handle(value: u64) -> ValueHandle {
    let v = NanBoxedValue::from_bits(value);
    if v.is_number() {
        alloc_value(Value::number(v.as_number()))
    } else if v.is_bool() {
        alloc_value(Value::boolean(v.as_bool()))
    } else if v.is_unit() {
        alloc_value(Value::unit())
    } else if v.is_none() {
        alloc_value(Value::absent())
    } else if v.is_heap_ptr() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            unsafe {
                crate::coral_value_retain(ptr);
            }
        }
        ptr
    } else if v.is_error() {
        let ptr = v.as_heap_ptr();
        if !ptr.is_null() {
            unsafe {
                crate::coral_value_retain(ptr);
            }
        }
        ptr
    } else {
        alloc_value(Value::unit())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_add(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(result) = va.fast_add(vb) {
        return result.to_bits();
    }

    let ha = nb_to_handle(a);
    let hb = nb_to_handle(b);
    let result = crate::arithmetic::coral_value_add(ha, hb);
    unsafe {
        let nb_result = coral_nb_from_handle(result);
        nb_result
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_sub(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(result) = va.fast_sub(vb) {
        return result.to_bits();
    }

    NanBoxedValue::unit().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_mul(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(result) = va.fast_mul(vb) {
        return result.to_bits();
    }
    NanBoxedValue::unit().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_div(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(result) = va.fast_div(vb) {
        return result.to_bits();
    }
    NanBoxedValue::unit().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_rem(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(result) = va.fast_rem(vb) {
        return result.to_bits();
    }
    NanBoxedValue::unit().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_neg(a: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    if va.is_number() {
        return NanBoxedValue::from_number(-va.as_number()).to_bits();
    }
    NanBoxedValue::unit().to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_equals(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(eq) = va.fast_equals(vb) {
        return NanBoxedValue::from_bool(eq).to_bits();
    }

    let ha = nb_to_handle(a);
    let hb = nb_to_handle(b);
    let result = crate::arithmetic::coral_value_equals(ha, hb);
    unsafe { coral_nb_from_handle(result) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_not_equals(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(eq) = va.fast_equals(vb) {
        return NanBoxedValue::from_bool(!eq).to_bits();
    }
    let ha = nb_to_handle(a);
    let hb = nb_to_handle(b);
    let result = crate::arithmetic::coral_value_not_equals(ha, hb);
    unsafe { coral_nb_from_handle(result) }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_less_than(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(lt) = va.fast_less_than(vb) {
        return NanBoxedValue::from_bool(lt).to_bits();
    }

    NanBoxedValue::from_bool(false).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_greater_than(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if let Some(gt) = va.fast_greater_than(vb) {
        return NanBoxedValue::from_bool(gt).to_bits();
    }
    NanBoxedValue::from_bool(false).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_less_equal(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if va.is_number() && vb.is_number() {
        return NanBoxedValue::from_bool(va.as_number() <= vb.as_number()).to_bits();
    }
    NanBoxedValue::from_bool(false).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_greater_equal(a: u64, b: u64) -> u64 {
    let va = NanBoxedValue::from_bits(a);
    let vb = NanBoxedValue::from_bits(b);
    if va.is_number() && vb.is_number() {
        return NanBoxedValue::from_bool(va.as_number() >= vb.as_number()).to_bits();
    }
    NanBoxedValue::from_bool(false).to_bits()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_print(value: u64) {
    let v = NanBoxedValue::from_bits(value);
    if v.is_number() {
        let n = v.as_number();
        if n == (n as i64) as f64 && n.abs() < 1e15 && !n.is_nan() {
            print!("{}", n as i64);
        } else {
            print!("{}", n);
        }
        return;
    }
    if v.is_bool() {
        print!("{}", if v.as_bool() { "true" } else { "false" });
        return;
    }
    if v.is_unit() {
        print!("unit");
        return;
    }
    if v.is_none() {
        print!("none");
        return;
    }

    let handle = nb_to_handle(value);
    crate::coral_log(handle);
    unsafe {
        crate::coral_value_release(handle);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_println(value: u64) {
    coral_nb_print(value);
    println!();
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_value_length(value: u64) -> u64 {
    let v = NanBoxedValue::from_bits(value);
    if v.is_immediate() {
        return NanBoxedValue::from_number(0.0).to_bits();
    }
    let handle = nb_to_handle(value);
    let result = crate::coral_value_length(handle);
    let nb = unsafe { coral_nb_from_handle(result) };
    unsafe {
        crate::coral_value_release(handle);
    }
    nb
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_nb_value_get(collection: u64, key: u64) -> u64 {
    let hc = nb_to_handle(collection);
    let hk = nb_to_handle(key);
    let result = crate::coral_value_get(hc, hk);
    let nb = unsafe { coral_nb_from_handle(result) };
    unsafe {
        crate::coral_value_release(hc);
        crate::coral_value_release(hk);
    }
    nb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nanbox::NanBoxedValue;

    #[test]
    fn nb_make_number_roundtrip() {
        let bits = coral_nb_make_number(42.0);
        let v = NanBoxedValue::from_bits(bits);
        assert!(v.is_number());
        assert_eq!(v.as_number(), 42.0);
        assert_eq!(coral_nb_as_number(bits), 42.0);
    }

    #[test]
    fn nb_make_bool_roundtrip() {
        let t = coral_nb_make_bool(1);
        let f = coral_nb_make_bool(0);
        assert_eq!(coral_nb_as_bool(t), 1);
        assert_eq!(coral_nb_as_bool(f), 0);
    }

    #[test]
    fn nb_make_unit_is_unit() {
        let u = coral_nb_make_unit();
        let v = NanBoxedValue::from_bits(u);
        assert!(v.is_unit());
    }

    #[test]
    fn nb_make_none_is_absent() {
        let n = coral_nb_make_none();
        assert_eq!(coral_nb_is_absent(n), 1);
    }

    #[test]
    fn nb_is_truthy_numbers() {
        assert_eq!(coral_nb_is_truthy(coral_nb_make_number(1.0)), 1);
        assert_eq!(coral_nb_is_truthy(coral_nb_make_number(0.0)), 0);
        assert_eq!(coral_nb_is_truthy(coral_nb_make_number(-1.0)), 1);
    }

    #[test]
    fn nb_is_truthy_bools() {
        assert_eq!(coral_nb_is_truthy(coral_nb_make_bool(1)), 1);
        assert_eq!(coral_nb_is_truthy(coral_nb_make_bool(0)), 0);
    }

    #[test]
    fn nb_is_truthy_unit_none() {
        assert_eq!(coral_nb_is_truthy(coral_nb_make_unit()), 0);
        assert_eq!(coral_nb_is_truthy(coral_nb_make_none()), 0);
    }

    #[test]
    fn nb_add_fast_path() {
        let a = coral_nb_make_number(10.0);
        let b = coral_nb_make_number(20.0);
        let result = coral_nb_add(a, b);
        assert_eq!(coral_nb_as_number(result), 30.0);
    }

    #[test]
    fn nb_sub_fast_path() {
        let a = coral_nb_make_number(50.0);
        let b = coral_nb_make_number(8.0);
        let result = coral_nb_sub(a, b);
        assert_eq!(coral_nb_as_number(result), 42.0);
    }

    #[test]
    fn nb_mul_fast_path() {
        let a = coral_nb_make_number(6.0);
        let b = coral_nb_make_number(7.0);
        let result = coral_nb_mul(a, b);
        assert_eq!(coral_nb_as_number(result), 42.0);
    }

    #[test]
    fn nb_div_fast_path() {
        let a = coral_nb_make_number(84.0);
        let b = coral_nb_make_number(2.0);
        let result = coral_nb_div(a, b);
        assert_eq!(coral_nb_as_number(result), 42.0);
    }

    #[test]
    fn nb_equals_fast_path() {
        let a = coral_nb_make_number(42.0);
        let b = coral_nb_make_number(42.0);
        let c = coral_nb_make_number(99.0);
        let eq = coral_nb_equals(a, b);
        let ne = coral_nb_equals(a, c);
        assert_eq!(coral_nb_as_bool(eq), 1);
        assert_eq!(coral_nb_as_bool(ne), 0);
    }

    #[test]
    fn nb_less_than_fast_path() {
        let a = coral_nb_make_number(3.0);
        let b = coral_nb_make_number(5.0);
        let lt = coral_nb_less_than(a, b);
        assert_eq!(coral_nb_as_bool(lt), 1);
        let gt = coral_nb_less_than(b, a);
        assert_eq!(coral_nb_as_bool(gt), 0);
    }

    #[test]
    fn nb_retain_release_immediate_is_noop() {
        let n = coral_nb_make_number(42.0);
        let b = coral_nb_make_bool(1);
        let u = coral_nb_make_unit();
        unsafe {
            coral_nb_retain(n);
            coral_nb_release(n);
            coral_nb_retain(b);
            coral_nb_release(b);
            coral_nb_retain(u);
            coral_nb_release(u);
        }
    }

    #[test]
    fn nb_tag_immediates() {
        assert_eq!(
            coral_nb_tag(coral_nb_make_number(1.0)),
            ValueTag::Number as u8
        );
        assert_eq!(coral_nb_tag(coral_nb_make_bool(1)), ValueTag::Bool as u8);
        assert_eq!(coral_nb_tag(coral_nb_make_unit()), ValueTag::Unit as u8);
    }
}
