//! I/O operations for the Coral runtime.

use crate::*;
use std::io::BufRead;


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_read(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_unit();
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_unit();
    };
    match fs::read(&pb) {
        Ok(bytes) => coral_make_bytes(bytes.as_ptr(), bytes.len()),
        Err(_) => coral_make_unit(),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_exists(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_bool(0);
    };
    let exists = pb.exists();
    coral_make_bool(if exists { 1 } else { 0 })
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_write(path: ValueHandle, data: ValueHandle) -> ValueHandle {
    if path.is_null() || data.is_null() {
        return coral_make_bool(0);
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_bool(0);
    };
    let bytes = {
        let data_ref = unsafe { &*data };
        string_to_bytes(data_ref)
    };
    let result = fs::write(&pb, bytes);
    coral_make_bool(if result.is_ok() { 1 } else { 0 })
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_process_args() -> ValueHandle {
    let args: Vec<String> = env::args().collect();
    let mut handles: Vec<ValueHandle> = Vec::with_capacity(args.len());
    for a in &args {
        handles.push(coral_make_string(a.as_ptr(), a.len()));
    }
    let list = coral_make_list(handles.as_ptr(), handles.len());
    for h in &handles {
        unsafe { coral_value_release(*h); }
    }
    list
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_process_exit(code: ValueHandle) -> ValueHandle {
    let exit_code = if code.is_null() {
        0
    } else {
        let v = unsafe { &*code };
        if v.tag == ValueTag::Number as u8 {
            (unsafe { v.payload.number }) as i32
        } else {
            1
        }
    };
    std::process::exit(exit_code);
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_env_get(name: ValueHandle) -> ValueHandle {
    if name.is_null() { return coral_make_unit(); }
    let v = unsafe { &*name };
    if v.tag != ValueTag::String as u8 { return coral_make_unit(); }
    let key = value_to_rust_string(v);
    match env::var(&key) {
        Ok(val) => coral_make_string(val.as_ptr(), val.len()),
        Err(_) => coral_make_unit(),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_env_set(name: ValueHandle, val: ValueHandle) -> ValueHandle {
    if name.is_null() || val.is_null() { return coral_make_unit(); }
    let nv = unsafe { &*name };
    let vv = unsafe { &*val };
    if nv.tag != ValueTag::String as u8 || vv.tag != ValueTag::String as u8 { return coral_make_unit(); }
    let key = value_to_rust_string(nv);
    let value = value_to_rust_string(vv);
    unsafe { env::set_var(&key, &value); }
    coral_make_unit()
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_append(path: ValueHandle, data: ValueHandle) -> ValueHandle {
    if path.is_null() || data.is_null() { return coral_make_unit(); }
    let pv = unsafe { &*path };
    let dv = unsafe { &*data };
    let Some(p) = value_to_path(pv) else { return coral_make_unit(); };
    let content = if dv.tag == ValueTag::String as u8 {
        value_to_rust_string(dv).into_bytes()
    } else if dv.tag == ValueTag::Bytes as u8 {
        string_to_bytes(dv)
    } else {
        return coral_make_unit();
    };
    use std::io::Write;
    match std::fs::OpenOptions::new().append(true).create(true).open(&p) {
        Ok(mut f) => {
            match f.write_all(&content) {
                Ok(_) => coral_make_bool(1),
                Err(_) => coral_make_bool(0),
            }
        }
        Err(_) => coral_make_bool(0),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_read_dir(path: ValueHandle) -> ValueHandle {
    if path.is_null() { return coral_make_list(ptr::null(), 0); }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else { return coral_make_list(ptr::null(), 0); };
    let entries: Vec<ValueHandle> = match fs::read_dir(&p) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                coral_make_string(name.as_ptr(), name.len())
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    let list = coral_make_list(entries.as_ptr(), entries.len());
    for h in &entries {
        unsafe { coral_value_release(*h); }
    }
    list
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_mkdir(path: ValueHandle) -> ValueHandle {
    if path.is_null() { return coral_make_bool(0); }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else { return coral_make_bool(0); };
    match fs::create_dir_all(&p) {
        Ok(_) => coral_make_bool(1),
        Err(_) => coral_make_bool(0),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_delete(path: ValueHandle) -> ValueHandle {
    if path.is_null() { return coral_make_bool(0); }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else { return coral_make_bool(0); };
    let result = if p.is_dir() {
        fs::remove_dir_all(&p)
    } else {
        fs::remove_file(&p)
    };
    match result {
        Ok(_) => coral_make_bool(1),
        Err(_) => coral_make_bool(0),
    }
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_is_dir(path: ValueHandle) -> ValueHandle {
    if path.is_null() { return coral_make_bool(0); }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else { return coral_make_bool(0); };
    coral_make_bool(if p.is_dir() { 1 } else { 0 })
}


#[unsafe(no_mangle)]
pub extern "C" fn coral_stdin_read_line() -> ValueHandle {
    let mut buf = String::new();
    match std::io::stdin().read_line(&mut buf) {
        Ok(_) => {
            // Trim trailing newline
            if buf.ends_with('\n') { buf.pop(); }
            if buf.ends_with('\r') { buf.pop(); }
            coral_make_string(buf.as_ptr(), buf.len())
        }
        Err(_) => coral_make_unit(),
    }
}

