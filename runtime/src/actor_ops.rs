use crate::actor::{ActorHandle, ActorSystem};
use crate::*;

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_spawn(handler: ValueHandle) -> ValueHandle {
    if handler.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &*handler };
    if value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handler);
    }

    let handler_bits = handler as usize;
    let system = actor::global_system().clone();
    let parent = actor::current_actor();
    let handle = system.spawn(parent, move |ctx| {
        let handler = handler_bits as ValueHandle;
        let self_value = actor_to_value(ctx.handle(), ctx.system());
        loop {
            match ctx.recv() {
                Some(actor::Message::User(msg)) => {
                    let args = [self_value, msg];
                    let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                    unsafe {
                        coral_value_release(result);
                    }
                    unsafe {
                        coral_value_release(msg);
                    }
                }
                Some(actor::Message::Exit) | None => break,
                Some(actor::Message::Failure(reason)) => {
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle {
                                    id: parent,
                                    sender: entry.sender.clone(),
                                };
                                let _ = ctx
                                    .system()
                                    .send(&parent_handle, actor::Message::Failure(reason));
                            }
                        }
                    }
                    break;
                }
                Some(actor::Message::ChildFailure { child_id, reason }) => {
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle {
                                    id: parent,
                                    sender: entry.sender.clone(),
                                };
                                let _ = ctx.system().send(
                                    &parent_handle,
                                    actor::Message::ChildFailure { child_id, reason },
                                );
                            }
                        }
                    }
                }
                Some(actor::Message::GracefulStop) => {
                    ctx.drain_and_stop(|msg| {
                        let args = [self_value, msg];
                        let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                        unsafe {
                            coral_value_release(result);
                        }
                        unsafe {
                            coral_value_release(msg);
                        }
                    });
                    break;
                }
                Some(actor::Message::ActorDown { actor_id, reason }) => {
                    let down_msg = make_actor_down_value(actor_id, &reason);
                    let args = [self_value, down_msg];
                    let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                    unsafe {
                        coral_value_release(result);
                    }
                    unsafe {
                        coral_value_release(down_msg);
                    }
                }
            }
        }
        unsafe {
            coral_value_release(self_value);
        }
        unsafe {
            coral_value_release(handler);
        }
    });
    actor_to_value(handle, system)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_send(actor_value: ValueHandle, message: ValueHandle) -> ValueHandle {
    if actor_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(actor_obj) = actor_from_value(unsafe { &*actor_value }) else {
        return coral_make_bool(0);
    };
    freeze_value(message);
    unsafe {
        coral_value_retain(message);
    }
    let ok = actor_obj
        .system
        .send(&actor_obj.handle, actor::Message::User(message))
        .is_ok();
    if !ok {
        unsafe {
            coral_value_release(message);
        }
    }
    coral_make_bool(if ok { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_stop(actor_value: ValueHandle) -> ValueHandle {
    if actor_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(actor_obj) = actor_from_value(unsafe { &*actor_value }) else {
        return coral_make_bool(0);
    };
    let ok = actor_obj
        .system
        .send(&actor_obj.handle, actor::Message::Exit)
        .is_ok();
    coral_make_bool(if ok { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_graceful_stop(actor_value: ValueHandle) -> ValueHandle {
    if actor_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(actor_obj) = actor_from_value(unsafe { &*actor_value }) else {
        return coral_make_bool(0);
    };
    let ok = actor_obj
        .system
        .send(&actor_obj.handle, actor::Message::GracefulStop)
        .is_ok();
    coral_make_bool(if ok { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_self() -> ValueHandle {
    let Some(id) = actor::current_actor() else {
        return coral_make_unit();
    };
    let system = actor::global_system();
    let maybe_handle = system.registry.lock().ok().and_then(|reg| {
        reg.get(&id).map(|entry| ActorHandle {
            id,
            sender: entry.sender.clone(),
        })
    });
    if let Some(handle) = maybe_handle {
        actor_to_value(handle, system.clone())
    } else {
        coral_make_unit()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_spawn_named(
    name_value: ValueHandle,
    handler: ValueHandle,
) -> ValueHandle {
    if name_value.is_null() || handler.is_null() {
        return coral_make_unit();
    }

    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_unit();
        }
        value_to_rust_string(name_val)
    };

    let value = unsafe { &*handler };
    if value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handler);
    }

    let handler_bits = handler as usize;
    let system = actor::global_system().clone();
    let parent = actor::current_actor();

    let maybe_handle = system.spawn_named(&name, parent, move |ctx| {
        let handler = handler_bits as ValueHandle;
        let self_value = actor_to_value(ctx.handle(), ctx.system());
        loop {
            match ctx.recv() {
                Some(actor::Message::User(msg)) => {
                    let args = [self_value, msg];
                    let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                    unsafe {
                        coral_value_release(result);
                    }
                    unsafe {
                        coral_value_release(msg);
                    }
                }
                Some(actor::Message::Exit) | None => break,
                Some(actor::Message::Failure(reason)) => {
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle {
                                    id: parent,
                                    sender: entry.sender.clone(),
                                };
                                let _ = ctx
                                    .system()
                                    .send(&parent_handle, actor::Message::Failure(reason));
                            }
                        }
                    }
                    break;
                }
                Some(actor::Message::ChildFailure { child_id, reason }) => {
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle {
                                    id: parent,
                                    sender: entry.sender.clone(),
                                };
                                let _ = ctx.system().send(
                                    &parent_handle,
                                    actor::Message::ChildFailure { child_id, reason },
                                );
                            }
                        }
                    }
                }
                Some(actor::Message::GracefulStop) => {
                    ctx.drain_and_stop(|msg| {
                        let args = [self_value, msg];
                        let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                        unsafe {
                            coral_value_release(result);
                        }
                        unsafe {
                            coral_value_release(msg);
                        }
                    });
                    break;
                }
                Some(actor::Message::ActorDown { actor_id, reason }) => {
                    let down_msg = make_actor_down_value(actor_id, &reason);
                    let args = [self_value, down_msg];
                    let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                    unsafe {
                        coral_value_release(result);
                    }
                    unsafe {
                        coral_value_release(down_msg);
                    }
                }
            }
        }
        unsafe {
            coral_value_release(self_value);
        }
        unsafe {
            coral_value_release(handler);
        }
    });

    match maybe_handle {
        Some(handle) => actor_to_value(handle, system),
        None => {
            unsafe {
                coral_value_release(handler);
            }
            coral_make_unit()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_lookup(name_value: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_unit();
    }

    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_unit();
        }
        value_to_rust_string(name_val)
    };

    let system = actor::global_system();
    match system.lookup_named(&name) {
        Some(handle) => actor_to_value(handle, system.clone()),
        None => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_register(name_value: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_bool(0);
    }

    let Some(id) = actor::current_actor() else {
        return coral_make_bool(0);
    };

    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_bool(0);
        }
        value_to_rust_string(name_val)
    };

    let system = actor::global_system();

    let maybe_handle = system.registry.lock().ok().and_then(|reg| {
        reg.get(&id).map(|entry| ActorHandle {
            id,
            sender: entry.sender.clone(),
        })
    });

    if let Some(handle) = maybe_handle {
        let success = system.register_named(&name, handle);
        coral_make_bool(if success { 1 } else { 0 })
    } else {
        coral_make_bool(0)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_unregister(name_value: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_bool(0);
    }

    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_bool(0);
        }
        value_to_rust_string(name_val)
    };

    let system = actor::global_system();
    let success = system.unregister_named(&name);
    coral_make_bool(if success { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_send_named(
    name_value: ValueHandle,
    message: ValueHandle,
) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_bool(0);
    }

    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_bool(0);
        }
        value_to_rust_string(name_val)
    };

    let system = actor::global_system();

    if let Some(handle) = system.lookup_named(&name) {
        freeze_value(message);
        unsafe {
            coral_value_retain(message);
        }
        let ok = system.send(&handle, actor::Message::User(message)).is_ok();
        if !ok {
            unsafe {
                coral_value_release(message);
            }
        }
        coral_make_bool(if ok { 1 } else { 0 })
    } else {
        coral_make_bool(0)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_list_named() -> ValueHandle {
    let system = actor::global_system();
    let named = system.list_named();

    let mut names: Vec<ValueHandle> = Vec::with_capacity(named.len());
    for (name, _) in named {
        names.push(coral_make_string(name.as_ptr(), name.len()));
    }

    let handle = coral_make_list(names.as_ptr(), names.len());

    unsafe {
        for name in names {
            coral_value_release(name);
        }
    }
    handle
}

fn value_to_f64(value: ValueHandle) -> Option<f64> {
    if value.is_null() {
        return None;
    }
    let v = unsafe { &*value };
    if v.tag == ValueTag::Number as u8 {
        Some(unsafe { v.payload.number })
    } else {
        None
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_send_after(
    delay_ms_value: ValueHandle,
    actor_value: ValueHandle,
    message: ValueHandle,
) -> ValueHandle {
    use std::time::Duration;

    let delay_ms = match value_to_f64(delay_ms_value) {
        Some(d) if d >= 0.0 => d as u64,
        _ => return coral_make_number(0.0),
    };

    let actor_val = if actor_value.is_null() {
        return coral_make_number(0.0);
    } else {
        unsafe { &*actor_value }
    };

    if actor_val.tag != ValueTag::Actor as u8 {
        return coral_make_number(0.0);
    }

    let actor_ptr = actor_val.heap_ptr();
    if actor_ptr.is_null() {
        return coral_make_number(0.0);
    }

    let handle = unsafe { &*(actor_ptr as *const ActorHandle) };

    freeze_value(message);
    unsafe {
        coral_value_retain(message);
    }

    let system = actor::global_system();
    let token = system.send_after(Duration::from_millis(delay_ms), handle, message);

    coral_make_number(token.id().0 as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_schedule_repeat(
    interval_ms_value: ValueHandle,
    actor_value: ValueHandle,
    message: ValueHandle,
) -> ValueHandle {
    use std::time::Duration;

    let interval_ms = match value_to_f64(interval_ms_value) {
        Some(d) if d > 0.0 => d as u64,
        _ => return coral_make_number(0.0),
    };

    let actor_val = if actor_value.is_null() {
        return coral_make_number(0.0);
    } else {
        unsafe { &*actor_value }
    };

    if actor_val.tag != ValueTag::Actor as u8 {
        return coral_make_number(0.0);
    }

    let actor_ptr = actor_val.heap_ptr();
    if actor_ptr.is_null() {
        return coral_make_number(0.0);
    }

    let handle = unsafe { &*(actor_ptr as *const ActorHandle) };

    freeze_value(message);
    unsafe {
        coral_value_retain(message);
    }

    let system = actor::global_system();
    let token = system.schedule_repeat(Duration::from_millis(interval_ms), handle, message);

    coral_make_number(token.id().0 as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_cancel(timer_id_value: ValueHandle) -> ValueHandle {
    let timer_id = match value_to_f64(timer_id_value) {
        Some(id) if id > 0.0 => id as u64,
        _ => return coral_make_bool(0),
    };

    let system = actor::global_system();
    let cancelled = system.timer_wheel.cancel(actor::TimerId(timer_id));
    coral_make_bool(if cancelled { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_pending_count() -> ValueHandle {
    let system = actor::global_system();
    let count = system.pending_timers();
    coral_make_number(count as f64)
}

use std::sync::{Condvar, Mutex as StdMutex, OnceLock};

fn main_done() -> &'static (StdMutex<bool>, Condvar) {
    static MAIN_DONE: OnceLock<(StdMutex<bool>, Condvar)> = OnceLock::new();
    MAIN_DONE.get_or_init(|| (StdMutex::new(false), Condvar::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_main_done_signal() -> ValueHandle {
    let (lock, cvar) = main_done();
    let mut done = lock.lock().unwrap();
    *done = true;
    cvar.notify_all();
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_main_wait() -> ValueHandle {
    let (lock, cvar) = main_done();
    let mut done = lock.lock().unwrap();
    while !*done {
        done = cvar.wait(done).unwrap();
    }
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_monitor(
    watcher_value: ValueHandle,
    target_value: ValueHandle,
) -> ValueHandle {
    if watcher_value.is_null() || target_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(watcher_obj) = actor_from_value(unsafe { &*watcher_value }) else {
        return coral_make_bool(0);
    };
    let Some(target_obj) = actor_from_value(unsafe { &*target_value }) else {
        return coral_make_bool(0);
    };
    watcher_obj
        .system
        .monitor(watcher_obj.handle.id, target_obj.handle.id);
    coral_make_bool(1)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_demonitor(
    watcher_value: ValueHandle,
    target_value: ValueHandle,
) -> ValueHandle {
    if watcher_value.is_null() || target_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(watcher_obj) = actor_from_value(unsafe { &*watcher_value }) else {
        return coral_make_bool(0);
    };
    let Some(target_obj) = actor_from_value(unsafe { &*target_value }) else {
        return coral_make_bool(0);
    };
    watcher_obj
        .system
        .demonitor(watcher_obj.handle.id, target_obj.handle.id);
    coral_make_bool(1)
}

pub(crate) fn make_actor_down_value(actor_id: actor::ActorId, reason: &str) -> ValueHandle {
    let type_key = coral_make_string_from_rust("type");
    let type_val = coral_make_string_from_rust("actor_down");
    let id_key = coral_make_string_from_rust("actor_id");
    let id_val = coral_make_number(actor_id.0 as f64);
    let reason_key = coral_make_string_from_rust("reason");
    let reason_val = coral_make_string_from_rust(reason);
    let entries = [
        MapEntry {
            key: type_key,
            value: type_val,
        },
        MapEntry {
            key: id_key,
            value: id_val,
        },
        MapEntry {
            key: reason_key,
            value: reason_val,
        },
    ];
    coral_make_map(entries.as_ptr(), entries.len())
}

const YIELD_THRESHOLD: u32 = 1000;

thread_local! {
    static YIELD_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_yield_check() {
    YIELD_COUNTER.with(|c| {
        let val = c.get().wrapping_add(1);
        if val >= YIELD_THRESHOLD {
            c.set(0);
            std::thread::yield_now();
        } else {
            c.set(val);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_msg_dispatch(
    msg_name: crate::ValueHandle,
    table: *const *const u8,
    lengths: *const usize,
    table_count: usize,
) -> i64 {
    if msg_name.is_null() || table_count == 0 {
        return -1;
    }
    let val = unsafe { &*msg_name };
    let name_bytes = crate::string_to_bytes(val);

    for i in 0..table_count {
        let entry_ptr = unsafe { *table.add(i) };
        let entry_len = unsafe { *lengths.add(i) };
        if entry_len == name_bytes.len() {
            let entry_slice = unsafe { std::slice::from_raw_parts(entry_ptr, entry_len) };
            if entry_slice == name_bytes.as_slice() {
                return i as i64;
            }
        }
    }
    -1
}
