use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SymbolId(pub u64);

impl SymbolId {
    pub const INVALID: SymbolId = SymbolId(0);

    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

pub struct SymbolTable {
    string_to_id: RwLock<HashMap<String, SymbolId>>,

    id_to_string: RwLock<HashMap<SymbolId, String>>,

    next_id: AtomicU64,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            string_to_id: RwLock::new(HashMap::new()),
            id_to_string: RwLock::new(HashMap::new()),

            next_id: AtomicU64::new(1),
        }
    }

    pub fn intern(&self, s: &str) -> SymbolId {
        {
            let table = self.string_to_id.read().unwrap();
            if let Some(&id) = table.get(s) {
                return id;
            }
        }

        let mut string_to_id = self.string_to_id.write().unwrap();
        let mut id_to_string = self.id_to_string.write().unwrap();

        if let Some(&id) = string_to_id.get(s) {
            return id;
        }

        let id = SymbolId(self.next_id.fetch_add(1, Ordering::Relaxed));
        string_to_id.insert(s.to_string(), id);
        id_to_string.insert(id, s.to_string());

        id
    }

    pub fn resolve(&self, id: SymbolId) -> Option<String> {
        if !id.is_valid() {
            return None;
        }
        let table = self.id_to_string.read().unwrap();
        table.get(&id).cloned()
    }

    pub fn lookup(&self, s: &str) -> Option<SymbolId> {
        let table = self.string_to_id.read().unwrap();
        table.get(s).copied()
    }

    pub fn len(&self) -> usize {
        self.string_to_id.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static SYMBOL_TABLE: OnceLock<SymbolTable> = OnceLock::new();

fn get_symbol_table() -> &'static SymbolTable {
    SYMBOL_TABLE.get_or_init(SymbolTable::new)
}

pub fn global_symbols() -> &'static SymbolTable {
    get_symbol_table()
}

pub fn intern(s: &str) -> SymbolId {
    get_symbol_table().intern(s)
}

pub fn resolve(id: SymbolId) -> Option<String> {
    get_symbol_table().resolve(id)
}

use crate::{
    Value, ValueHandle, ValueTag, coral_make_bool, coral_make_number, coral_make_string,
    value_to_rust_string,
};

#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_intern(string_val: ValueHandle) -> ValueHandle {
    if string_val.is_null() {
        return coral_make_number(SymbolId::INVALID.0 as f64);
    }

    let value = unsafe { &*string_val };
    if value.tag != ValueTag::String as u8 {
        return coral_make_number(SymbolId::INVALID.0 as f64);
    }

    let s = value_to_rust_string(value);
    if s.is_empty() {
        return coral_make_number(SymbolId::INVALID.0 as f64);
    }

    let id = intern(&s);
    coral_make_number(id.0 as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_lookup(string_val: ValueHandle) -> ValueHandle {
    if string_val.is_null() {
        return coral_make_number(0.0);
    }

    let value = unsafe { &*string_val };
    if value.tag != ValueTag::String as u8 {
        return coral_make_number(0.0);
    }

    let s = value_to_rust_string(value);

    match get_symbol_table().lookup(&s) {
        Some(id) => coral_make_number(id.0 as f64),
        None => coral_make_number(0.0),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_resolve(id_val: ValueHandle) -> ValueHandle {
    if id_val.is_null() {
        return std::ptr::null_mut();
    }

    let value = unsafe { &*id_val };
    if value.tag != ValueTag::Number as u8 {
        return std::ptr::null_mut();
    }

    let id = SymbolId(unsafe { value.payload.number as u64 });

    match resolve(id) {
        Some(s) => coral_make_string(s.as_ptr(), s.len()),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_equals(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    let id_a = if a.is_null() {
        0
    } else {
        let value = unsafe { &*a };
        if value.tag == ValueTag::Number as u8 {
            unsafe { value.payload.number as u64 }
        } else {
            0
        }
    };

    let id_b = if b.is_null() {
        0
    } else {
        let value = unsafe { &*b };
        if value.tag == ValueTag::Number as u8 {
            unsafe { value.payload.number as u64 }
        } else {
            0
        }
    };

    coral_make_bool(if id_a == id_b { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_count() -> usize {
    get_symbol_table().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_and_resolve() {
        let id = intern("hello");
        assert!(id.is_valid());

        let resolved = resolve(id);
        assert_eq!(resolved, Some("hello".to_string()));
    }

    #[test]
    fn test_intern_same_string_twice() {
        let id1 = intern("world");
        let id2 = intern("world");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_different_strings_different_ids() {
        let id1 = intern("foo");
        let id2 = intern("bar");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_lookup_existing() {
        intern("lookup_test");
        let id = get_symbol_table().lookup("lookup_test");
        assert!(id.is_some());
    }

    #[test]
    fn test_invalid_symbol() {
        assert!(!SymbolId::INVALID.is_valid());
        let resolved = resolve(SymbolId::INVALID);
        assert_eq!(resolved, None);
    }
}
