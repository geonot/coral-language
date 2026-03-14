use crate::list_ops::{coral_list_get_index, coral_list_len};
use crate::*;
use std::io::BufRead;
use std::io::Write;

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
        unsafe {
            coral_value_release(*h);
        }
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
    if name.is_null() {
        return coral_make_unit();
    }
    let v = unsafe { &*name };
    if v.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let key = value_to_rust_string(v);
    match env::var(&key) {
        Ok(val) => coral_make_string(val.as_ptr(), val.len()),
        Err(_) => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_env_set(name: ValueHandle, val: ValueHandle) -> ValueHandle {
    if name.is_null() || val.is_null() {
        return coral_make_unit();
    }
    let nv = unsafe { &*name };
    let vv = unsafe { &*val };
    if nv.tag != ValueTag::String as u8 || vv.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let key = value_to_rust_string(nv);
    let value = value_to_rust_string(vv);
    unsafe {
        env::set_var(&key, &value);
    }
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_append(path: ValueHandle, data: ValueHandle) -> ValueHandle {
    if path.is_null() || data.is_null() {
        return coral_make_unit();
    }
    let pv = unsafe { &*path };
    let dv = unsafe { &*data };
    let Some(p) = value_to_path(pv) else {
        return coral_make_unit();
    };
    let content = if dv.tag == ValueTag::String as u8 {
        value_to_rust_string(dv).into_bytes()
    } else if dv.tag == ValueTag::Bytes as u8 {
        string_to_bytes(dv)
    } else {
        return coral_make_unit();
    };
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&p)
    {
        Ok(mut f) => match f.write_all(&content) {
            Ok(_) => coral_make_bool(1),
            Err(_) => coral_make_bool(0),
        },
        Err(_) => coral_make_bool(0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_read_dir(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_list(ptr::null(), 0);
    }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else {
        return coral_make_list(ptr::null(), 0);
    };
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
        unsafe {
            coral_value_release(*h);
        }
    }
    list
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_mkdir(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else {
        return coral_make_bool(0);
    };
    match fs::create_dir_all(&p) {
        Ok(_) => coral_make_bool(1),
        Err(_) => coral_make_bool(0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_delete(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else {
        return coral_make_bool(0);
    };
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
    if path.is_null() {
        return coral_make_bool(0);
    }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else {
        return coral_make_bool(0);
    };
    coral_make_bool(if p.is_dir() { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_stdin_read_line() -> ValueHandle {
    let mut buf = String::new();
    match std::io::stdin().read_line(&mut buf) {
        Ok(_) => {
            if buf.ends_with('\n') {
                buf.pop();
            }
            if buf.ends_with('\r') {
                buf.pop();
            }
            coral_make_string(buf.as_ptr(), buf.len())
        }
        Err(_) => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_stderr_write(msg: ValueHandle) -> ValueHandle {
    if msg.is_null() {
        return coral_make_unit();
    }
    let mv = unsafe { &*msg };
    let s = value_to_rust_string(mv);
    let _ = std::io::stderr().write_all(s.as_bytes());
    let _ = std::io::stderr().flush();
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_size(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_number(0.0);
    }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else {
        return coral_make_number(-1.0);
    };
    match std::fs::metadata(&p) {
        Ok(meta) => coral_make_number(meta.len() as f64),
        Err(_) => coral_make_number(-1.0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_rename(old: ValueHandle, new: ValueHandle) -> ValueHandle {
    if old.is_null() || new.is_null() {
        return coral_make_bool(0);
    }
    let ov = unsafe { &*old };
    let nv = unsafe { &*new };
    let Some(old_path) = value_to_path(ov) else {
        return coral_make_bool(0);
    };
    let Some(new_path) = value_to_path(nv) else {
        return coral_make_bool(0);
    };
    coral_make_bool(if std::fs::rename(old_path, new_path).is_ok() {
        1
    } else {
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_copy(src: ValueHandle, dst: ValueHandle) -> ValueHandle {
    if src.is_null() || dst.is_null() {
        return coral_make_bool(0);
    }
    let sv = unsafe { &*src };
    let dv = unsafe { &*dst };
    let Some(src_path) = value_to_path(sv) else {
        return coral_make_bool(0);
    };
    let Some(dst_path) = value_to_path(dv) else {
        return coral_make_bool(0);
    };
    coral_make_bool(if std::fs::copy(src_path, dst_path).is_ok() {
        1
    } else {
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_mkdirs(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let pv = unsafe { &*path };
    let Some(p) = value_to_path(pv) else {
        return coral_make_bool(0);
    };
    coral_make_bool(if std::fs::create_dir_all(p).is_ok() {
        1
    } else {
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_temp_dir() -> ValueHandle {
    let tmp = std::env::temp_dir();
    let s = tmp.to_string_lossy();
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_process_exec(cmd: ValueHandle, args: ValueHandle) -> ValueHandle {
    if cmd.is_null() {
        return coral_make_unit();
    }
    let cv = unsafe { &*cmd };
    let cmd_str = value_to_rust_string(cv);

    let mut command = std::process::Command::new(&cmd_str);

    if !args.is_null() {
        let av = unsafe { &*args };
        if av.tag == ValueTag::List as u8 {
            let len = coral_list_len(args);
            for i in 0..len {
                let item = coral_list_get_index(args, i);
                if !item.is_null() {
                    let iv = unsafe { &*item };
                    let arg_str = value_to_rust_string(iv);
                    command.arg(&arg_str);
                }
            }
        }
    }

    match command.output() {
        Ok(output) => {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1) as f64;

            let key_stdout = coral_make_string("stdout".as_ptr(), 6);
            let val_stdout = coral_make_string(stdout_str.as_ptr(), stdout_str.len());
            let key_stderr = coral_make_string("stderr".as_ptr(), 6);
            let val_stderr = coral_make_string(stderr_str.as_ptr(), stderr_str.len());
            let key_exit = coral_make_string("exit_code".as_ptr(), 9);
            let val_exit = coral_make_number(exit_code);

            let entries = [
                MapEntry {
                    key: key_stdout,
                    value: val_stdout,
                },
                MapEntry {
                    key: key_stderr,
                    value: val_stderr,
                },
                MapEntry {
                    key: key_exit,
                    value: val_exit,
                },
            ];
            coral_make_map(entries.as_ptr(), entries.len())
        }
        Err(_) => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_process_cwd() -> ValueHandle {
    match std::env::current_dir() {
        Ok(p) => {
            let s = p.to_string_lossy();
            coral_make_string(s.as_ptr(), s.len())
        }
        Err(_) => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_process_chdir(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let pv = unsafe { &*path };
    let s = value_to_rust_string(pv);
    match std::env::set_current_dir(&s) {
        Ok(()) => coral_make_bool(1),
        Err(_) => coral_make_bool(0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_process_pid() -> ValueHandle {
    coral_make_number(std::process::id() as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_path_normalize(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_unit();
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_unit();
    };
    use std::path::{Component, PathBuf};
    let mut result = PathBuf::new();
    for component in pb.components() {
        match component {
            Component::ParentDir => {
                if !result.pop() {
                    result.push("..");
                }
            }
            Component::CurDir => {}
            other => result.push(other),
        }
    }
    let s = result.to_string_lossy();
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_path_resolve(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_unit();
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_unit();
    };
    match std::fs::canonicalize(&pb) {
        Ok(p) => {
            let s = p.to_string_lossy();
            coral_make_string(s.as_ptr(), s.len())
        }
        Err(_) => {
            let s = pb.to_string_lossy();
            coral_make_string(s.as_ptr(), s.len())
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_path_is_absolute(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_bool(0);
    };
    if pb.is_absolute() {
        coral_make_bool(1)
    } else {
        coral_make_bool(0)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_path_parent(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_unit();
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_unit();
    };
    match pb.parent() {
        Some(parent) => {
            let s = parent.to_string_lossy();
            coral_make_string(s.as_ptr(), s.len())
        }
        None => {
            let empty = "";
            coral_make_string(empty.as_ptr(), empty.len())
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_path_stem(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_unit();
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_unit();
    };
    match pb.file_stem() {
        Some(stem) => {
            let s = stem.to_string_lossy();
            coral_make_string(s.as_ptr(), s.len())
        }
        None => {
            let empty = "";
            coral_make_string(empty.as_ptr(), empty.len())
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_process_hostname() -> ValueHandle {
    match std::fs::read_to_string("/etc/hostname") {
        Ok(name) => {
            let trimmed = name.trim();
            coral_make_string(trimmed.as_ptr(), trimmed.len())
        }
        Err(_) => {
            let fallback = "unknown";
            coral_make_string(fallback.as_ptr(), fallback.len())
        }
    }
}
