//! Symbol interning for efficient message dispatch.
//! 
//! This module provides a global symbol table that maps strings to unique
//! integer IDs. This enables O(1) message dispatch instead of O(n) string
//! comparison in actor message handlers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{RwLock, OnceLock};

/// A unique identifier for an interned symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SymbolId(pub u64);

impl SymbolId {
    /// The invalid/uninitialized symbol ID.
    pub const INVALID: SymbolId = SymbolId(0);
    
    /// Check if this is a valid symbol ID.
    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// Global symbol table for string interning.
pub struct SymbolTable {
    /// Maps strings to their symbol IDs.
    string_to_id: RwLock<HashMap<String, SymbolId>>,
    /// Maps symbol IDs back to strings (for debugging).
    id_to_string: RwLock<HashMap<SymbolId, String>>,
    /// Next available symbol ID.
    next_id: AtomicU64,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    /// Create a new empty symbol table.
    pub fn new() -> Self {
        Self {
            string_to_id: RwLock::new(HashMap::new()),
            id_to_string: RwLock::new(HashMap::new()),
            // Start from 1 since 0 is INVALID
            next_id: AtomicU64::new(1),
        }
    }
    
    /// Intern a string and return its symbol ID.
    /// 
    /// If the string is already interned, returns the existing ID.
    /// Thread-safe: uses a read-write lock for efficiency.
    pub fn intern(&self, s: &str) -> SymbolId {
        // Fast path: check if already interned (read lock)
        {
            let table = self.string_to_id.read().unwrap();
            if let Some(&id) = table.get(s) {
                return id;
            }
        }
        
        // Slow path: acquire write lock and intern
        let mut string_to_id = self.string_to_id.write().unwrap();
        let mut id_to_string = self.id_to_string.write().unwrap();
        
        // Double-check after acquiring write lock
        if let Some(&id) = string_to_id.get(s) {
            return id;
        }
        
        // Create new symbol
        let id = SymbolId(self.next_id.fetch_add(1, Ordering::Relaxed));
        string_to_id.insert(s.to_string(), id);
        id_to_string.insert(id, s.to_string());
        
        id
    }
    
    /// Get the string associated with a symbol ID.
    /// Returns None if the ID is invalid or not found.
    pub fn resolve(&self, id: SymbolId) -> Option<String> {
        if !id.is_valid() {
            return None;
        }
        let table = self.id_to_string.read().unwrap();
        table.get(&id).cloned()
    }
    
    /// Get the symbol ID for a string without interning.
    /// Returns None if the string has not been interned.
    pub fn lookup(&self, s: &str) -> Option<SymbolId> {
        let table = self.string_to_id.read().unwrap();
        table.get(s).copied()
    }
    
    /// Get the number of interned symbols.
    pub fn len(&self) -> usize {
        self.string_to_id.read().unwrap().len()
    }
    
    /// Check if the symbol table is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Global symbol table instance using OnceLock
static SYMBOL_TABLE: OnceLock<SymbolTable> = OnceLock::new();

fn get_symbol_table() -> &'static SymbolTable {
    SYMBOL_TABLE.get_or_init(SymbolTable::new)
}

/// Get the global symbol table.
pub fn global_symbols() -> &'static SymbolTable {
    get_symbol_table()
}

/// Convenience function to intern a string in the global table.
pub fn intern(s: &str) -> SymbolId {
    get_symbol_table().intern(s)
}

/// Convenience function to resolve a symbol ID from the global table.
pub fn resolve(id: SymbolId) -> Option<String> {
    get_symbol_table().resolve(id)
}

// ========== FFI Exports ==========

use crate::{Value, ValueHandle, ValueTag, coral_make_string, coral_make_number, coral_make_bool, value_to_rust_string};

/// Intern a string Value and return its symbol ID as a number Value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_intern(string_val: ValueHandle) -> ValueHandle {
    if string_val.is_null() {
        return coral_make_number(SymbolId::INVALID.0 as f64);
    }
    
    let value = unsafe { &*string_val };
    if value.tag != ValueTag::String as u8 {
        return coral_make_number(SymbolId::INVALID.0 as f64);
    }
    
    // Use the canonical string extraction that handles both inline and heap strings
    let s = value_to_rust_string(value);
    if s.is_empty() {
        return coral_make_number(SymbolId::INVALID.0 as f64);
    }
    
    let id = intern(&s);
    coral_make_number(id.0 as f64)
}

/// Get the symbol ID for a string without interning.
/// Returns 0 if not found.
#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_lookup(string_val: ValueHandle) -> ValueHandle {
    if string_val.is_null() {
        return coral_make_number(0.0);
    }
    
    let value = unsafe { &*string_val };
    if value.tag != ValueTag::String as u8 {
        return coral_make_number(0.0);
    }
    
    // Use the canonical string extraction that handles both inline and heap strings
    let s = value_to_rust_string(value);
    
    match get_symbol_table().lookup(&s) {
        Some(id) => coral_make_number(id.0 as f64),
        None => coral_make_number(0.0),
    }
}

/// Resolve a symbol ID back to a string Value.
/// Returns null if the ID is invalid.
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

/// Check if two symbol IDs are equal.
/// Takes symbol IDs as number Values.
#[unsafe(no_mangle)]
pub extern "C" fn coral_symbol_equals(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    let id_a = if a.is_null() { 0 } else {
        let value = unsafe { &*a };
        if value.tag == ValueTag::Number as u8 {
            unsafe { value.payload.number as u64 }
        } else {
            0
        }
    };
    
    let id_b = if b.is_null() { 0 } else {
        let value = unsafe { &*b };
        if value.tag == ValueTag::Number as u8 {
            unsafe { value.payload.number as u64 }
        } else {
            0
        }
    };
    
    coral_make_bool(if id_a == id_b { 1 } else { 0 })
}

/// Get the number of interned symbols.
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
