use crate::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_now() -> ValueHandle {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    coral_make_number(ms)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_timestamp() -> ValueHandle {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as f64)
        .unwrap_or(0.0);
    coral_make_number(secs)
}

fn timestamp_to_components(ts: f64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ts as i64;
    let second = ((secs % 60) + 60) as u32 % 60;
    let total_mins = secs.div_euclid(60);
    let minute = (total_mins.rem_euclid(60)) as u32;
    let total_hours = total_mins.div_euclid(60);
    let hour = (total_hours.rem_euclid(24)) as u32;
    let mut days = total_hours.div_euclid(24);

    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, hour, minute, second)
}

fn get_ts_arg(value: ValueHandle) -> f64 {
    if value.is_null() {
        return 0.0;
    }
    let v = unsafe { &*value };
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::Number) => unsafe { v.payload.number },
        _ => 0.0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_year(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (year, _, _, _, _, _) = timestamp_to_components(ts);
    coral_make_number(year as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_month(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (_, month, _, _, _, _) = timestamp_to_components(ts);
    coral_make_number(month as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_day(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (_, _, day, _, _, _) = timestamp_to_components(ts);
    coral_make_number(day as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_hour(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (_, _, _, hour, _, _) = timestamp_to_components(ts);
    coral_make_number(hour as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_minute(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (_, _, _, _, minute, _) = timestamp_to_components(ts);
    coral_make_number(minute as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_second(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (_, _, _, _, _, second) = timestamp_to_components(ts);
    coral_make_number(second as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_time_format_iso(value: ValueHandle) -> ValueHandle {
    let ts = get_ts_arg(value);
    let (year, month, day, hour, minute, second) = timestamp_to_components(ts);
    let iso = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    );
    coral_make_string_from_rust(&iso)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sleep(ms_value: ValueHandle) -> ValueHandle {
    let ms = get_ts_arg(ms_value) as u64;
    std::thread::sleep(std::time::Duration::from_millis(ms));
    coral_make_unit()
}
