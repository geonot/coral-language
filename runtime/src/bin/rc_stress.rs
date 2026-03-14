use runtime::*;
use std::time::Instant;

fn churn_lists(iter: usize, inner: usize) {
    for i in 0..iter {
        let mut handles = Vec::with_capacity(inner);
        for j in 0..inner {
            let num = coral_make_number((i * j) as f64);
            handles.push(num);
        }
        let list = coral_make_list(handles.as_ptr(), handles.len());

        for _ in 0..3.min(inner) {
            let popped = coral_list_pop(list);
            unsafe { coral_value_release(popped) };
        }
        unsafe {
            coral_value_release(list);
            for h in handles {
                coral_value_release(h);
            }
        }
    }
}

fn churn_maps(iter: usize, inner: usize) {
    for i in 0..iter {
        let mut entries = Vec::with_capacity(inner);
        for j in 0..inner {
            let key = coral_make_number(j as f64);
            let val = coral_make_number((i * j) as f64);
            entries.push(runtime::MapEntry { key, value: val });
        }
        let map = coral_make_map(entries.as_ptr(), entries.len());

        if inner > 0 {
            let key0 = entries[0].key;
            let new_val = coral_make_number((i * inner) as f64);
            coral_map_set(map, key0, new_val);
        }
        unsafe {
            coral_value_release(map);
            for entry in entries {
                coral_value_release(entry.key);
                coral_value_release(entry.value);
            }
        }
    }
}

fn churn_strings(iter: usize, len: usize) {
    let base = "x".repeat(len);
    for i in 0..iter {
        let text = format!("{base}{i}");
        let handle = coral_make_string(text.as_ptr(), text.len());
        unsafe { coral_value_release(handle) };
    }
}

fn main() {
    runtime::coral_runtime_release_queue_init(4096);
    let start = Instant::now();
    churn_lists(500, 200);
    churn_maps(400, 150);
    churn_strings(2000, 64);
    runtime::coral_runtime_release_queue_flush();
    let elapsed = start.elapsed();
    println!("rc_stress complete in {:?}", elapsed);
}
