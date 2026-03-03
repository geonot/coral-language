//! FFI functions for persistent store access from Coral code
//!
//! These functions expose the store engine to Coral programs via the runtime.
//! All functions use the `coral_store_*` naming convention.
//!
//! Store handles are returned as numbers encoding the store key.

use std::collections::HashMap;
use std::ffi::c_void;
use std::slice;
use std::sync::RwLock;

use super::config::StoreConfig;
use super::binary::StoredValue;
use super::{open_store_engine, save_all_engines, close_engine, SharedStoreEngine};

// Type alias for ValueHandle from parent crate
type ValueHandle = *mut c_void;

// These are declared extern to access runtime value creation functions
// They're defined in lib.rs and we call them via FFI-style linkage
unsafe extern "C" {
    fn coral_make_unit() -> ValueHandle;
    fn coral_make_number(value: f64) -> ValueHandle;
    fn coral_make_bool(value: u8) -> ValueHandle;
    fn coral_make_string(ptr: *const u8, len: usize) -> ValueHandle;
    fn coral_make_bytes(ptr: *const u8, len: usize) -> ValueHandle;
    fn coral_make_list(items: *const ValueHandle, len: usize) -> ValueHandle;
    fn coral_make_map(entries: *const MapEntry, len: usize) -> ValueHandle;
    fn coral_make_error(code: u32, name_ptr: *const u8, name_len: usize) -> ValueHandle;
    fn coral_make_absent() -> ValueHandle;
    fn coral_value_retain(value: ValueHandle);
    fn coral_value_release(value: ValueHandle);
    fn coral_value_tag(value: ValueHandle) -> u8;
    fn coral_value_as_number(value: ValueHandle) -> f64;
    fn coral_value_as_bool(value: ValueHandle) -> u8;
    fn coral_list_len(list: ValueHandle) -> usize;
    fn coral_list_get_index(list: ValueHandle, index: usize) -> ValueHandle;
    fn coral_map_get(map: ValueHandle, key: ValueHandle) -> ValueHandle;
    fn coral_map_keys(map: ValueHandle) -> ValueHandle;
}

// Tag values matching ValueTag in lib.rs
// ValueTag: Number=0, Bool=1, String=2, List=3, Map=4, Store=5, Actor=6, Unit=7, Closure=8, Bytes=9
const TAG_NUMBER: u8 = 0;
const TAG_BOOL: u8 = 1;
const TAG_STRING: u8 = 2;
const TAG_LIST: u8 = 3;
const TAG_MAP: u8 = 4;
const TAG_UNIT: u8 = 7;
const TAG_BYTES: u8 = 9;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MapEntry {
    pub key: ValueHandle,
    pub value: ValueHandle,
}

/// Store handle info (store_type, store_name, and data_path)
#[derive(Clone)]
pub struct StoreHandleInfo {
    pub store_type: String,
    pub store_name: String,
    pub data_path: String,
}

// Global handle registry
static STORE_HANDLES: RwLock<Option<HashMap<u64, StoreHandleInfo>>> = RwLock::new(None);
static NEXT_HANDLE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn register_handle(store_type: String, store_name: String, data_path: String) -> u64 {
    let id = NEXT_HANDLE_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut guard = STORE_HANDLES.write().unwrap();
    let handles = guard.get_or_insert_with(HashMap::new);
    handles.insert(id, StoreHandleInfo { store_type, store_name, data_path });
    id
}

fn get_handle_info(handle_id: u64) -> Option<StoreHandleInfo> {
    let guard = STORE_HANDLES.read().unwrap();
    guard.as_ref()?.get(&handle_id).cloned()
}

fn remove_handle(handle_id: u64) {
    let mut guard = STORE_HANDLES.write().unwrap();
    if let Some(handles) = guard.as_mut() {
        handles.remove(&handle_id);
    }
}

// ============================================================================
// Value Conversion Functions
// ============================================================================

/// Convert a StoredValue to a Coral ValueHandle
fn stored_value_to_handle(value: &StoredValue) -> ValueHandle {
    unsafe {
        match value {
            StoredValue::Unit => coral_make_unit(),
            StoredValue::None => coral_make_absent(),
            StoredValue::Bool(b) => coral_make_bool(if *b { 1 } else { 0 }),
            StoredValue::Int(i) => coral_make_number(*i as f64),
            StoredValue::Float(f) => coral_make_number(*f),
            StoredValue::String(s) => coral_make_string(s.as_ptr(), s.len()),
            StoredValue::Bytes(b) => coral_make_bytes(b.as_ptr(), b.len()),
            StoredValue::List(items) => {
                let handles: Vec<ValueHandle> = items.iter()
                    .map(|v| stored_value_to_handle(v))
                    .collect();
                let result = coral_make_list(handles.as_ptr(), handles.len());
                // Release our temporary handles (list retains them)
                for h in handles {
                    coral_value_release(h);
                }
                result
            }
            StoredValue::Map(pairs) => {
                let entries: Vec<MapEntry> = pairs.iter()
                    .map(|(k, v)| {
                        let key = coral_make_string(k.as_ptr(), k.len());
                        let value = stored_value_to_handle(v);
                        MapEntry { key, value }
                    })
                    .collect();
                let result = coral_make_map(entries.as_ptr(), entries.len());
                // Release our temporary handles (map retains them)
                for e in entries {
                    coral_value_release(e.key);
                    coral_value_release(e.value);
                }
                result
            }
        }
    }
}

/// Convert a Coral ValueHandle to a StoredValue
fn handle_to_stored_value(value: ValueHandle) -> Option<StoredValue> {
    if value.is_null() {
        return Some(StoredValue::Unit);
    }
    let tag = unsafe { coral_value_tag(value) };
    match tag {
        TAG_UNIT => Some(StoredValue::Unit),
        TAG_NUMBER => {
            let n = unsafe { coral_value_as_number(value) };
            // Use Int if the value is a whole number that fits in i64, else Float
            if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                Some(StoredValue::Int(n as i64))
            } else {
                Some(StoredValue::Float(n))
            }
        }
        TAG_BOOL => {
            let b = unsafe { coral_value_as_bool(value) };
            Some(StoredValue::Bool(b != 0))
        }
        TAG_STRING => {
            // Cast to the actual Value type to access string data
            let val = unsafe { &*(value as *const crate::Value) };
            let s = crate::value_to_rust_string(val);
            Some(StoredValue::String(s))
        }
        TAG_BYTES => {
            let val = unsafe { &*(value as *const crate::Value) };
            let bytes = crate::string_to_bytes(val);
            Some(StoredValue::Bytes(bytes))
        }
        TAG_LIST => {
            let len = unsafe { coral_list_len(value) };
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                let elem = unsafe { coral_list_get_index(value, i) };
                if let Some(sv) = handle_to_stored_value(elem) {
                    items.push(sv);
                } else {
                    items.push(StoredValue::Unit);
                }
            }
            Some(StoredValue::List(items))
        }
        TAG_MAP => {
            let keys_list = unsafe { coral_map_keys(value) };
            let keys_len = unsafe { coral_list_len(keys_list) };
            let mut pairs = Vec::with_capacity(keys_len);
            for i in 0..keys_len {
                let key_handle = unsafe { coral_list_get_index(keys_list, i) };
                let key_val = unsafe { &*(key_handle as *const crate::Value) };
                let key_str = crate::value_to_rust_string(key_val);
                let val_handle = unsafe { coral_map_get(value, key_handle) };
                let stored_val = handle_to_stored_value(val_handle)
                    .unwrap_or(StoredValue::Unit);
                pairs.push((key_str, stored_val));
            }
            unsafe { coral_value_release(keys_list); }
            Some(StoredValue::Map(pairs))
        }
        _ => {
            // Unsupported tags (Store, Actor, Closure, Tagged) → store as Unit
            Some(StoredValue::Unit)
        }
    }
}

/// Extract field pairs from a Map ValueHandle.
/// If the handle is not a map or is null, returns an empty vec.
fn extract_field_pairs(fields: ValueHandle) -> Vec<(String, StoredValue)> {
    if fields.is_null() {
        return Vec::new();
    }
    let tag = unsafe { coral_value_tag(fields) };
    if tag != TAG_MAP {
        return Vec::new();
    }
    // Use handle_to_stored_value to convert the map, then extract pairs
    match handle_to_stored_value(fields) {
        Some(StoredValue::Map(pairs)) => pairs,
        _ => Vec::new(),
    }
}

/// Create a map with system attributes from stored object
fn create_object_map(
    index: u64,
    uuid: &str,
    version: u32,
    created_at: i64,
    updated_at: i64,
    deleted_at: i64,
    fields: &[(String, StoredValue)],
) -> ValueHandle {
    unsafe {
        let mut entries = Vec::with_capacity(fields.len() + 6);
        
        // Add system attributes
        let index_key = coral_make_string(b"_index".as_ptr(), 6);
        let index_val = coral_make_number(index as f64);
        entries.push(MapEntry { key: index_key, value: index_val });
        
        let uuid_key = coral_make_string(b"_uuid".as_ptr(), 5);
        let uuid_val = coral_make_string(uuid.as_ptr(), uuid.len());
        entries.push(MapEntry { key: uuid_key, value: uuid_val });
        
        let version_key = coral_make_string(b"_version".as_ptr(), 8);
        let version_val = coral_make_number(version as f64);
        entries.push(MapEntry { key: version_key, value: version_val });
        
        let created_key = coral_make_string(b"_created_at".as_ptr(), 11);
        let created_val = coral_make_number(created_at as f64);
        entries.push(MapEntry { key: created_key, value: created_val });
        
        let updated_key = coral_make_string(b"_updated_at".as_ptr(), 11);
        let updated_val = coral_make_number(updated_at as f64);
        entries.push(MapEntry { key: updated_key, value: updated_val });
        
        if deleted_at >= 0 {
            let deleted_key = coral_make_string(b"_deleted_at".as_ptr(), 11);
            let deleted_val = coral_make_number(deleted_at as f64);
            entries.push(MapEntry { key: deleted_key, value: deleted_val });
        }
        
        // Add user fields
        for (k, v) in fields {
            let key = coral_make_string(k.as_ptr(), k.len());
            let value = stored_value_to_handle(v);
            entries.push(MapEntry { key, value });
        }
        
        let result = coral_make_map(entries.as_ptr(), entries.len());
        
        // Release temporary handles
        for e in entries {
            coral_value_release(e.key);
            coral_value_release(e.value);
        }
        
        result
    }
}

// ============================================================================
// Store FFI Functions
// ============================================================================

/// Open or create a store.
/// Returns a handle ID (as number) or error.
///
/// # Arguments
/// * `store_type_ptr` - Pointer to store type name
/// * `store_type_len` - Length of store type name
/// * `store_name_ptr` - Pointer to store name (or null for "default")
/// * `store_name_len` - Length of store name
/// * `data_path_ptr` - Pointer to data path (or null for ".coral_data")
/// * `data_path_len` - Length of data path
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_open(
    store_type_ptr: *const u8,
    store_type_len: usize,
    store_name_ptr: *const u8,
    store_name_len: usize,
    data_path_ptr: *const u8,
    data_path_len: usize,
) -> ValueHandle {
    // Parse store_type (required)
    let store_type = if store_type_ptr.is_null() || store_type_len == 0 {
        return unsafe { coral_make_error(1, b"InvalidStoreType".as_ptr(), 16) };
    } else {
        let bytes = unsafe { slice::from_raw_parts(store_type_ptr, store_type_len) };
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return unsafe { coral_make_error(2, b"InvalidUtf8".as_ptr(), 11) },
        }
    };
    
    // Parse store_name (optional, default = "default")
    let store_name = if store_name_ptr.is_null() || store_name_len == 0 {
        "default".to_string()
    } else {
        let bytes = unsafe { slice::from_raw_parts(store_name_ptr, store_name_len) };
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return unsafe { coral_make_error(2, b"InvalidUtf8".as_ptr(), 11) },
        }
    };
    
    // Parse data_path (optional, default = ".coral_data")
    let data_path = if data_path_ptr.is_null() || data_path_len == 0 {
        ".coral_data".to_string()
    } else {
        let bytes = unsafe { slice::from_raw_parts(data_path_ptr, data_path_len) };
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return unsafe { coral_make_error(2, b"InvalidUtf8".as_ptr(), 11) },
        }
    };
    
    // Create config
    let config = StoreConfig::minimal(&store_type, &data_path);
    
    // Open store engine
    match open_store_engine(&store_type, &store_name, config) {
        Ok(_) => {
            let handle_id = register_handle(store_type, store_name, data_path);
            unsafe { coral_make_number(handle_id as f64) }
        }
        Err(e) => {
            let msg = format!("StoreOpenFailed:{}", e);
            unsafe { coral_make_error(3, msg.as_ptr(), msg.len()) }
        }
    }
}

/// Close a store handle.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_close(handle: ValueHandle) -> ValueHandle {
    let handle_id = unsafe { coral_value_as_number(handle) as u64 };
    
    if let Some(info) = get_handle_info(handle_id) {
        close_engine(&info.store_type, &info.store_name);
        remove_handle(handle_id);
        unsafe { coral_make_bool(1) }
    } else {
        unsafe { coral_make_error(4, b"InvalidHandle".as_ptr(), 13) }
    }
}

/// Save all stores to disk.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_save_all() -> ValueHandle {
    match save_all_engines() {
        Ok(_) => unsafe { coral_make_bool(1) },
        Err(e) => {
            let msg = format!("SaveFailed:{}", e);
            unsafe { coral_make_error(5, msg.as_ptr(), msg.len()) }
        }
    }
}

/// Helper to get the engine for a handle
fn with_engine<F, T>(handle: ValueHandle, f: F) -> Result<T, ValueHandle>
where
    F: FnOnce(&SharedStoreEngine) -> Result<T, String>,
{
    let handle_id = unsafe { coral_value_as_number(handle) as u64 };
    
    let info = match get_handle_info(handle_id) {
        Some(i) => i,
        None => return Err(unsafe { coral_make_error(4, b"InvalidHandle".as_ptr(), 13) }),
    };
    
    let config = StoreConfig::minimal(&info.store_type, &info.data_path);
    let engine = match open_store_engine(&info.store_type, &info.store_name, config) {
        Ok(e) => e,
        Err(e) => {
            let msg = format!("EngineError:{}", e);
            return Err(unsafe { coral_make_error(3, msg.as_ptr(), msg.len()) });
        }
    };
    
    f(&engine).map_err(|e| {
        let msg = e.as_bytes();
        unsafe { coral_make_error(6, msg.as_ptr(), msg.len()) }
    })
}

/// Create a new object in the store.
/// Returns the created object (map with system attributes) or error.
///
/// # Arguments
/// * `handle` - Store handle
/// * `fields` - Map of field name -> value pairs
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_create(handle: ValueHandle, fields: ValueHandle) -> ValueHandle {
    // Convert fields map to stored value pairs
    let field_pairs = extract_field_pairs(fields);
    
    match with_engine(handle, |engine| {
        engine.create(field_pairs.clone())
            .map_err(|e| e.to_string())
    }) {
        Ok(index) => {
            // Get the created object to return it
            match with_engine(handle, |engine| {
                engine.get(index)
                    .map_err(|e| e.to_string())
                    .and_then(|opt| opt.ok_or_else(|| "Object not found".to_string()))
            }) {
                Ok(obj) => {
                    create_object_map(
                        obj.index,
                        &obj.uuid.to_string(),
                        obj.version,
                        obj.created_at,
                        obj.updated_at,
                        obj.deleted_at,
                        &obj.fields,
                    )
                }
                Err(e) => e,
            }
        }
        Err(e) => e,
    }
}

/// Get an object by its index.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_get_by_index(handle: ValueHandle, index: ValueHandle) -> ValueHandle {
    let idx = unsafe { coral_value_as_number(index) as u64 };
    
    match with_engine(handle, |engine| {
        engine.get(idx)
            .map_err(|e| e.to_string())
    }) {
        Ok(Some(obj)) => {
            create_object_map(
                obj.index,
                &obj.uuid.to_string(),
                obj.version,
                obj.created_at,
                obj.updated_at,
                obj.deleted_at,
                &obj.fields,
            )
        }
        Ok(None) => unsafe { coral_make_absent() },
        Err(e) => e,
    }
}

/// Get an object by its UUID.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_get_by_uuid(
    handle: ValueHandle,
    uuid_ptr: *const u8,
    uuid_len: usize,
) -> ValueHandle {
    // Parse UUID string
    let uuid_str = if uuid_ptr.is_null() || uuid_len == 0 {
        return unsafe { coral_make_error(8, b"InvalidUuid".as_ptr(), 11) };
    } else {
        let bytes = unsafe { slice::from_raw_parts(uuid_ptr, uuid_len) };
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return unsafe { coral_make_error(2, b"InvalidUtf8".as_ptr(), 11) },
        }
    };
    
    // Parse UUID
    let uuid = match super::uuid7::Uuid7::parse(&uuid_str) {
        Ok(u) => u,
        Err(_) => return unsafe { coral_make_error(8, b"InvalidUuid".as_ptr(), 11) },
    };
    
    // Use indexed lookup via with_engine + get_by_uuid
    match with_engine(handle, |engine| {
        engine.get_by_uuid(&uuid)
            .map_err(|e| e.to_string())
    }) {
        Ok(Some(obj)) => {
            create_object_map(
                obj.index,
                &obj.uuid.to_string(),
                obj.version,
                obj.created_at,
                obj.updated_at,
                obj.deleted_at,
                &obj.fields,
            )
        }
        Ok(None) => unsafe { coral_make_absent() },
        Err(e) => e,
    }
}

/// Update an object by index.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_update(
    handle: ValueHandle,
    index: ValueHandle,
    fields: ValueHandle,
) -> ValueHandle {
    let idx = unsafe { coral_value_as_number(index) as u64 };
    
    // Convert fields map to stored value pairs
    let field_pairs = extract_field_pairs(fields);
    
    match with_engine(handle, |engine| {
        engine.update(idx, field_pairs.clone())
            .map_err(|e| e.to_string())
    }) {
        Ok(()) => {
            // Return updated object
            match with_engine(handle, |engine| {
                engine.get(idx)
                    .map_err(|e| e.to_string())
                    .and_then(|opt| opt.ok_or_else(|| "Object not found".to_string()))
            }) {
                Ok(obj) => {
                    create_object_map(
                        obj.index,
                        &obj.uuid.to_string(),
                        obj.version,
                        obj.created_at,
                        obj.updated_at,
                        obj.deleted_at,
                        &obj.fields,
                    )
                }
                Err(e) => e,
            }
        }
        Err(e) => e,
    }
}

/// Soft-delete an object by index (sets _deleted_at timestamp).
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_soft_delete(handle: ValueHandle, index: ValueHandle) -> ValueHandle {
    let idx = unsafe { coral_value_as_number(index) as u64 };
    
    match with_engine(handle, |engine| {
        engine.delete(idx)
            .map_err(|e| e.to_string())
    }) {
        Ok(()) => unsafe { coral_make_bool(1) },
        Err(e) => e,
    }
}

/// Get store statistics.
/// Returns a map with: total_objects, cached_objects, dirty_objects, binary_size, jsonl_size
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_stats(handle: ValueHandle) -> ValueHandle {
    match with_engine(handle, |engine| {
        Ok(engine.stats())
    }) {
        Ok(stats) => unsafe {
            let mut entries = Vec::with_capacity(5);
            
            let total_key = coral_make_string(b"total_objects".as_ptr(), 13);
            let total_val = coral_make_number(stats.total_objects as f64);
            entries.push(MapEntry { key: total_key, value: total_val });
            
            let cached_key = coral_make_string(b"cached_objects".as_ptr(), 14);
            let cached_val = coral_make_number(stats.cached_objects as f64);
            entries.push(MapEntry { key: cached_key, value: cached_val });
            
            let dirty_key = coral_make_string(b"dirty_objects".as_ptr(), 13);
            let dirty_val = coral_make_number(stats.dirty_objects as f64);
            entries.push(MapEntry { key: dirty_key, value: dirty_val });
            
            let bin_key = coral_make_string(b"binary_size".as_ptr(), 11);
            let bin_val = coral_make_number(stats.binary_size as f64);
            entries.push(MapEntry { key: bin_key, value: bin_val });
            
            let json_key = coral_make_string(b"jsonl_size".as_ptr(), 10);
            let json_val = coral_make_number(stats.jsonl_size as f64);
            entries.push(MapEntry { key: json_key, value: json_val });
            
            let result = coral_make_map(entries.as_ptr(), entries.len());
            
            for e in entries {
                coral_value_release(e.key);
                coral_value_release(e.value);
            }
            
            result
        },
        Err(e) => e,
    }
}

/// Get count of non-deleted objects in the store.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_count(handle: ValueHandle) -> ValueHandle {
    match with_engine(handle, |engine| {
        Ok(engine.count())
    }) {
        Ok(count) => unsafe { coral_make_number(count as f64) },
        Err(e) => e,
    }
}

/// Persist all dirty objects to disk.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_persist(handle: ValueHandle) -> ValueHandle {
    match with_engine(handle, |engine| {
        engine.save()
            .map_err(|e| e.to_string())
    }) {
        Ok(()) => unsafe { coral_make_bool(1) },
        Err(e) => e,
    }
}

/// Create a checkpoint (save + truncate WAL).
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_checkpoint(handle: ValueHandle) -> ValueHandle {
    match with_engine(handle, |engine| {
        engine.checkpoint()
            .map_err(|e| e.to_string())
    }) {
        Ok(()) => unsafe { coral_make_bool(1) },
        Err(e) => e,
    }
}

/// Get all object indices in the store.
#[unsafe(no_mangle)]
pub extern "C" fn coral_store_all_indices(handle: ValueHandle) -> ValueHandle {
    match with_engine(handle, |engine| {
        Ok(engine.all())
    }) {
        Ok(indices) => unsafe {
            let handles: Vec<ValueHandle> = indices.iter()
                .map(|idx| coral_make_number(*idx as f64))
                .collect();
            
            let result = coral_make_list(handles.as_ptr(), handles.len());
            
            for h in handles {
                coral_value_release(h);
            }
            
            result
        },
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_stored_value_conversions() {
        // Test StoredValue creation
        let unit = StoredValue::Unit;
        let bool_val = StoredValue::Bool(true);
        let int_val = StoredValue::Int(42);
        let float_val = StoredValue::Float(3.14);
        let string_val = StoredValue::String("hello".to_string());
        let bytes_val = StoredValue::Bytes(vec![1, 2, 3]);
        let list_val = StoredValue::List(vec![
            StoredValue::Int(1),
            StoredValue::Int(2),
        ]);
        let map_val = StoredValue::Map(vec![
            ("key".to_string(), StoredValue::String("value".to_string())),
        ]);
        
        assert_eq!(unit, StoredValue::Unit);
        assert_eq!(bool_val, StoredValue::Bool(true));
        assert_eq!(int_val, StoredValue::Int(42));
        assert_eq!(float_val, StoredValue::Float(3.14));
        assert_eq!(string_val, StoredValue::String("hello".to_string()));
        assert_eq!(bytes_val, StoredValue::Bytes(vec![1, 2, 3]));
        
        // List and map checks
        if let StoredValue::List(items) = list_val {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
        
        if let StoredValue::Map(pairs) = map_val {
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].0, "key");
        } else {
            panic!("expected map");
        }
    }
    
    #[test]
    fn test_handle_registry() {
        let id1 = register_handle("TestType".to_string(), "default".to_string(), ".coral_data".to_string());
        let id2 = register_handle("TestType".to_string(), "other".to_string(), "/tmp/test".to_string());
        
        assert_ne!(id1, id2);
        
        let info1 = get_handle_info(id1).unwrap();
        assert_eq!(info1.store_type, "TestType");
        assert_eq!(info1.store_name, "default");
        assert_eq!(info1.data_path, ".coral_data");
        
        let info2 = get_handle_info(id2).unwrap();
        assert_eq!(info2.store_type, "TestType");
        assert_eq!(info2.store_name, "other");
        assert_eq!(info2.data_path, "/tmp/test");
        
        remove_handle(id1);
        assert!(get_handle_info(id1).is_none());
        assert!(get_handle_info(id2).is_some());
    }
}
