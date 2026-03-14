use crate::*;

#[inline]
fn handle_to_f64(handle: ValueHandle) -> Option<f64> {
    if handle.is_null() {
        return None;
    }
    let value = unsafe { &*handle };
    if value.tag == ValueTag::Number as u8 {
        Some(unsafe { value.payload.number })
    } else {
        None
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_abs(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.abs()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sqrt(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.sqrt()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_floor(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.floor()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_ceil(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.ceil()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_round(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.round()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sin(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.sin()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_cos(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.cos()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_tan(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.tan()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_pow(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    match (handle_to_f64(a), handle_to_f64(b)) {
        (Some(base), Some(exp)) => coral_make_number(base.powf(exp)),
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_min(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    match (handle_to_f64(a), handle_to_f64(b)) {
        (Some(x), Some(y)) => coral_make_number(x.min(y)),
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_max(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    match (handle_to_f64(a), handle_to_f64(b)) {
        (Some(x), Some(y)) => coral_make_number(x.max(y)),
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_ln(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.ln()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_log10(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.log10()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_exp(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.exp()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_asin(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.asin()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_acos(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.acos()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_atan(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.atan()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_atan2(y: ValueHandle, x: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(y, x) {
        return err;
    }
    match (handle_to_f64(y), handle_to_f64(x)) {
        (Some(y_val), Some(x_val)) => coral_make_number(y_val.atan2(x_val)),
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sinh(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.sinh()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_cosh(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.cosh()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_tanh(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.tanh()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_trunc(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.trunc()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sign(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.signum()),
        None => coral_make_unit(),
    }
}
