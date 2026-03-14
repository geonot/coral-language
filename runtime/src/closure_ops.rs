use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_closure(
    invoke: ClosureInvokeFn,
    release: ClosureReleaseFn,
    env: *mut c_void,
    capture_count: usize,
) -> ValueHandle {
    if invoke.is_none() {
        return coral_make_unit();
    }
    let object = Box::new(ClosureObject {
        invoke,
        release,
        env,
        capture_count,
    });
    alloc_value(Value::from_heap(
        ValueTag::Closure,
        Box::into_raw(object) as *mut c_void,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_closure_invoke(
    closure: ValueHandle,
    args: *const ValueHandle,
    len: usize,
) -> ValueHandle {
    if closure.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &*closure };
    if value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let object = unsafe { &*(ptr as *const ClosureObject) };
    let invoke = match object.invoke {
        Some(func) => func,
        None => return coral_make_unit(),
    };
    let mut out: ValueHandle = ptr::null_mut();
    unsafe {
        invoke(object.env, args, len, &mut out);
    }
    if out.is_null() {
        coral_make_unit()
    } else {
        out
    }
}
