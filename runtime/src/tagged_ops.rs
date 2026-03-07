//! Tagged value (ADT) operations for the Coral runtime.

use crate::*;


/// Get the tag name of a tagged value.
/// 
/// # Returns
/// A string ValueHandle containing the tag name, or Unit if not a tagged value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_get_tag(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_unit();
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    coral_make_string(tagged.tag_name, tagged.tag_name_len)
}


/// Check if a tagged value has a specific tag.
/// 
/// # Returns
/// A bool ValueHandle (true if tag matches, false otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_is_tag(
    value: ValueHandle,
    tag_name: *const u8,
    tag_name_len: usize,
) -> ValueHandle {
    if value.is_null() {
        return coral_make_bool(0);
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_bool(0);
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_bool(0);
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    
    if tagged.tag_name_len != tag_name_len {
        return coral_make_bool(0);
    }
    
    let stored = unsafe { slice::from_raw_parts(tagged.tag_name, tagged.tag_name_len) };
    let check = unsafe { slice::from_raw_parts(tag_name, tag_name_len) };
    
    coral_make_bool(if stored == check { 1 } else { 0 })
}


/// Get a field from a tagged value by index.
/// 
/// # Returns
/// The field ValueHandle, or Unit if out of bounds or not a tagged value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_get_field(value: ValueHandle, index: usize) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_unit();
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    
    if index >= tagged.field_count || tagged.fields.is_null() {
        return coral_make_unit();
    }
    
    let field = unsafe { *tagged.fields.add(index) };
    if !field.is_null() {
        unsafe { coral_value_retain(field) };
    }
    field
}


/// Get the number of fields in a tagged value.
/// 
/// # Returns
/// A number ValueHandle with the field count, or 0 if not a tagged value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_field_count(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_number(0.0);
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_number(0.0);
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_number(0.0);
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    coral_make_number(tagged.field_count as f64)
}

