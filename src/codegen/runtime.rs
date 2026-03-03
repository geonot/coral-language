//! Runtime function bindings for LLVM codegen.
//!
//! This module declares all external runtime functions that the generated
//! LLVM IR can call into the Coral runtime library.

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{FunctionType, PointerType, StructType};
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

/// Bindings to Coral runtime functions declared in the LLVM module.
#[allow(dead_code)]
pub struct RuntimeBindings<'ctx> {
    pub value_ptr_type: PointerType<'ctx>,
    pub make_number: FunctionValue<'ctx>,
    pub make_bool: FunctionValue<'ctx>,
    pub make_string: FunctionValue<'ctx>,
    pub make_bytes: FunctionValue<'ctx>,
    pub make_unit: FunctionValue<'ctx>,
    pub make_list: FunctionValue<'ctx>,
    pub make_list_hinted: FunctionValue<'ctx>,
    pub make_map: FunctionValue<'ctx>,
    pub make_map_hinted: FunctionValue<'ctx>,
    pub value_as_number: FunctionValue<'ctx>,
    pub value_as_bool: FunctionValue<'ctx>,
    pub value_add: FunctionValue<'ctx>,
    pub value_equals: FunctionValue<'ctx>,
    pub value_not_equals: FunctionValue<'ctx>,
    pub value_hash: FunctionValue<'ctx>,
    pub value_bitand: FunctionValue<'ctx>,
    pub value_bitor: FunctionValue<'ctx>,
    pub value_bitxor: FunctionValue<'ctx>,
    pub value_bitnot: FunctionValue<'ctx>,
    pub value_shift_left: FunctionValue<'ctx>,
    pub value_shift_right: FunctionValue<'ctx>,
    pub value_iter: FunctionValue<'ctx>,
    pub list_push: FunctionValue<'ctx>,
    pub list_get: FunctionValue<'ctx>,
    pub list_pop: FunctionValue<'ctx>,
    pub list_iter: FunctionValue<'ctx>,
    pub list_iter_next: FunctionValue<'ctx>,
    pub list_map: FunctionValue<'ctx>,
    pub list_filter: FunctionValue<'ctx>,
    pub list_reduce: FunctionValue<'ctx>,
    pub map_get: FunctionValue<'ctx>,
    pub map_set: FunctionValue<'ctx>,
    pub map_length: FunctionValue<'ctx>,
    pub map_keys: FunctionValue<'ctx>,
    pub map_iter: FunctionValue<'ctx>,
    pub map_iter_next: FunctionValue<'ctx>,
    pub value_length: FunctionValue<'ctx>,
    pub map_entry_type: StructType<'ctx>,
    pub make_closure: FunctionValue<'ctx>,
    pub closure_invoke: FunctionValue<'ctx>,
    pub log: FunctionValue<'ctx>,
    pub fs_read: FunctionValue<'ctx>,
    pub fs_write: FunctionValue<'ctx>,
    pub fs_exists: FunctionValue<'ctx>,
    pub value_retain: FunctionValue<'ctx>,
    pub value_release: FunctionValue<'ctx>,
    pub heap_alloc: FunctionValue<'ctx>,
    pub heap_free: FunctionValue<'ctx>,
    pub actor_spawn: FunctionValue<'ctx>,
    pub actor_send: FunctionValue<'ctx>,
    pub actor_stop: FunctionValue<'ctx>,
    pub actor_self: FunctionValue<'ctx>,
    // Named actor registry
    pub actor_spawn_named: FunctionValue<'ctx>,
    pub actor_lookup: FunctionValue<'ctx>,
    pub actor_register: FunctionValue<'ctx>,
    pub actor_unregister: FunctionValue<'ctx>,
    pub actor_send_named: FunctionValue<'ctx>,
    pub actor_list_named: FunctionValue<'ctx>,
    // Timer operations
    pub timer_send_after: FunctionValue<'ctx>,
    pub timer_schedule_repeat: FunctionValue<'ctx>,
    pub timer_cancel: FunctionValue<'ctx>,
    pub timer_pending_count: FunctionValue<'ctx>,
    pub closure_invoke_type: FunctionType<'ctx>,
    pub closure_release_type: FunctionType<'ctx>,
    // Tagged value (ADT) operations
    pub make_tagged: FunctionValue<'ctx>,
    pub tagged_is_tag: FunctionValue<'ctx>,
    pub tagged_get_field: FunctionValue<'ctx>,
    // String operations
    pub string_slice: FunctionValue<'ctx>,
    pub string_char_at: FunctionValue<'ctx>,
    pub string_index_of: FunctionValue<'ctx>,
    pub string_split: FunctionValue<'ctx>,
    pub string_to_chars: FunctionValue<'ctx>,
    pub string_starts_with: FunctionValue<'ctx>,
    pub string_ends_with: FunctionValue<'ctx>,
    pub string_trim: FunctionValue<'ctx>,
    pub string_to_upper: FunctionValue<'ctx>,
    pub string_to_lower: FunctionValue<'ctx>,
    pub string_replace: FunctionValue<'ctx>,
    pub string_contains: FunctionValue<'ctx>,
    pub string_parse_number: FunctionValue<'ctx>,
    pub number_to_string: FunctionValue<'ctx>,
    // Error value operations
    pub make_error: FunctionValue<'ctx>,
    pub make_absent: FunctionValue<'ctx>,
    pub is_err: FunctionValue<'ctx>,
    pub is_absent: FunctionValue<'ctx>,
    pub is_ok: FunctionValue<'ctx>,
    pub error_name: FunctionValue<'ctx>,
    pub error_code: FunctionValue<'ctx>,
    pub value_or: FunctionValue<'ctx>,
    // Math operations
    pub math_abs: FunctionValue<'ctx>,
    pub math_sqrt: FunctionValue<'ctx>,
    pub math_floor: FunctionValue<'ctx>,
    pub math_ceil: FunctionValue<'ctx>,
    pub math_round: FunctionValue<'ctx>,
    pub math_sin: FunctionValue<'ctx>,
    pub math_cos: FunctionValue<'ctx>,
    pub math_tan: FunctionValue<'ctx>,
    pub math_pow: FunctionValue<'ctx>,
    pub math_min: FunctionValue<'ctx>,
    pub math_max: FunctionValue<'ctx>,
    pub math_ln: FunctionValue<'ctx>,
    pub math_log10: FunctionValue<'ctx>,
    pub math_exp: FunctionValue<'ctx>,
    pub math_asin: FunctionValue<'ctx>,
    pub math_acos: FunctionValue<'ctx>,
    pub math_atan: FunctionValue<'ctx>,
    pub math_atan2: FunctionValue<'ctx>,
    pub math_sinh: FunctionValue<'ctx>,
    pub math_cosh: FunctionValue<'ctx>,
    pub math_tanh: FunctionValue<'ctx>,
    pub math_trunc: FunctionValue<'ctx>,
    pub math_sign: FunctionValue<'ctx>,
    // Universal iterator
    pub value_iter_next: FunctionValue<'ctx>,
    // Process/environment
    pub process_args: FunctionValue<'ctx>,
    pub process_exit: FunctionValue<'ctx>,
    pub env_get: FunctionValue<'ctx>,
    pub env_set: FunctionValue<'ctx>,
    // File I/O extensions
    pub fs_append: FunctionValue<'ctx>,
    pub fs_read_dir: FunctionValue<'ctx>,
    pub fs_mkdir: FunctionValue<'ctx>,
    pub fs_delete: FunctionValue<'ctx>,
    pub fs_is_dir: FunctionValue<'ctx>,
    // stdin
    pub stdin_read_line: FunctionValue<'ctx>,
    // List extensions
    pub list_contains: FunctionValue<'ctx>,
    pub list_index_of: FunctionValue<'ctx>,
    pub list_reverse: FunctionValue<'ctx>,
    pub list_slice: FunctionValue<'ctx>,
    pub list_sort: FunctionValue<'ctx>,
    pub list_join: FunctionValue<'ctx>,
    pub list_concat: FunctionValue<'ctx>,
    // Map extensions
    pub map_remove: FunctionValue<'ctx>,
    pub map_values: FunctionValue<'ctx>,
    pub map_entries: FunctionValue<'ctx>,
    pub map_has_key: FunctionValue<'ctx>,
    pub map_merge: FunctionValue<'ctx>,
    // Bytes extensions
    pub bytes_get: FunctionValue<'ctx>,
    pub bytes_from_string: FunctionValue<'ctx>,
    pub bytes_to_string: FunctionValue<'ctx>,
    pub bytes_slice_val: FunctionValue<'ctx>,
    // Type reflection
    pub type_of: FunctionValue<'ctx>,
    // Character operations
    pub string_ord: FunctionValue<'ctx>,
    pub string_chr: FunctionValue<'ctx>,
    pub string_compare: FunctionValue<'ctx>,
}

impl<'ctx> RuntimeBindings<'ctx> {
    /// Declare all runtime functions in the given LLVM module.
    pub fn declare(context: &'ctx Context, module: &Module<'ctx>) -> Self {
        let i8_type = context.i8_type();
        let i16_type = context.i16_type();
        let i32_type = context.i32_type();
        let i64_type = context.i64_type();
        let f64_type = context.f64_type();
        let usize_type = context.i64_type();
        
        // Value struct layout matching runtime
        let payload = i8_type.array_type(16);
        let value_type = context.struct_type(
            &[
                i8_type.into(),   // tag
                i8_type.into(),   // flags
                i16_type.into(),  // reserved
                i64_type.into(),  // rc
                i32_type.into(),  // extra1
                i32_type.into(),  // extra2
                payload.into(),   // payload
            ],
            false,
        );
        let value_ptr_type = value_type.ptr_type(AddressSpace::default());
        let value_ptr_ptr_type = value_ptr_type.ptr_type(AddressSpace::default());
        let i8_ptr = i8_type.ptr_type(AddressSpace::default());
        
        // Map entry type: (key, value) pair
        let map_entry_type = context.struct_type(
            &[value_ptr_type.into(), value_ptr_type.into()],
            false,
        );
        let map_entry_ptr_type = map_entry_type.ptr_type(AddressSpace::default());
        
        // Closure function types
        let closure_invoke_type = context.void_type().fn_type(
            &[
                i8_ptr.into(),           // env pointer
                value_ptr_ptr_type.into(), // args array
                usize_type.into(),       // arg count
                value_ptr_ptr_type.into(), // out pointer
            ],
            false,
        );
        let closure_release_type = context.void_type().fn_type(&[i8_ptr.into()], false);

        // Value constructors
        let make_number = module.add_function(
            "coral_make_number",
            value_ptr_type.fn_type(&[f64_type.into()], false),
            None,
        );
        let make_bool = module.add_function(
            "coral_make_bool",
            value_ptr_type.fn_type(&[i8_type.into()], false),
            None,
        );
        let make_string = module.add_function(
            "coral_make_string",
            value_ptr_type.fn_type(&[i8_ptr.into(), usize_type.into()], false),
            None,
        );
        let make_bytes = module.add_function(
            "coral_make_bytes",
            value_ptr_type.fn_type(&[i8_ptr.into(), usize_type.into()], false),
            None,
        );
        let make_unit = module.add_function(
            "coral_make_unit",
            value_ptr_type.fn_type(&[], false),
            None,
        );
        let make_list = module.add_function(
            "coral_make_list",
            value_ptr_type.fn_type(&[value_ptr_ptr_type.into(), usize_type.into()], false),
            None,
        );
        let make_list_hinted = module.add_function(
            "coral_make_list_hinted",
            value_ptr_type.fn_type(
                &[value_ptr_ptr_type.into(), usize_type.into(), i8_type.into()],
                false,
            ),
            None,
        );
        let make_map = module.add_function(
            "coral_make_map",
            value_ptr_type.fn_type(&[map_entry_ptr_type.into(), usize_type.into()], false),
            None,
        );
        let make_map_hinted = module.add_function(
            "coral_make_map_hinted",
            value_ptr_type.fn_type(
                &[map_entry_ptr_type.into(), usize_type.into(), i8_type.into()],
                false,
            ),
            None,
        );

        // List operations
        let list_push = module.add_function(
            "coral_list_push",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_get = module.add_function(
            "coral_list_get",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_pop = module.add_function(
            "coral_list_pop",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_iter = module.add_function(
            "coral_list_iter",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_iter_next = module.add_function(
            "coral_list_iter_next",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_map = module.add_function(
            "coral_list_map",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_filter = module.add_function(
            "coral_list_filter",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_reduce = module.add_function(
            "coral_list_reduce",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );

        // Map operations
        let map_get = module.add_function(
            "coral_map_get",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let map_set = module.add_function(
            "coral_map_set",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let map_length = module.add_function(
            "coral_map_length",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_keys = module.add_function(
            "coral_map_keys",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_iter = module.add_function(
            "coral_map_iter",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_iter_next = module.add_function(
            "coral_map_iter_next",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // Value operations
        let value_length = module.add_function(
            "coral_value_length",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_iter = module.add_function(
            "coral_value_iter",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_as_number = module.add_function(
            "coral_value_as_number",
            f64_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_as_bool = module.add_function(
            "coral_value_as_bool",
            i8_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_add = module.add_function(
            "coral_value_add",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_equals = module.add_function(
            "coral_value_equals",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_not_equals = module.add_function(
            "coral_value_not_equals",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_hash = module.add_function(
            "coral_value_hash",
            i64_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // Bitwise operations
        let value_bitand = module.add_function(
            "coral_value_bitand",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_bitor = module.add_function(
            "coral_value_bitor",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_bitxor = module.add_function(
            "coral_value_bitxor",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_bitnot = module.add_function(
            "coral_value_bitnot",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_shift_left = module.add_function(
            "coral_value_shift_left",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let value_shift_right = module.add_function(
            "coral_value_shift_right",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        // I/O and logging
        let log = module.add_function(
            "coral_log",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_read = module.add_function(
            "coral_fs_read",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_write = module.add_function(
            "coral_fs_write",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let fs_exists = module.add_function(
            "coral_fs_exists",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // Closure operations
        let make_closure = module.add_function(
            "coral_make_closure",
            value_ptr_type.fn_type(
                &[
                    closure_invoke_type.ptr_type(AddressSpace::default()).into(),
                    closure_release_type.ptr_type(AddressSpace::default()).into(),
                    i8_ptr.into(),
                ],
                false,
            ),
            None,
        );
        let closure_invoke = module.add_function(
            "coral_closure_invoke",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_ptr_type.into(), usize_type.into()],
                false,
            ),
            None,
        );

        // Memory management
        let value_retain = module.add_function(
            "coral_value_retain",
            context.void_type().fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_release = module.add_function(
            "coral_value_release",
            context.void_type().fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let heap_alloc = module.add_function(
            "coral_heap_alloc",
            i8_ptr.fn_type(&[usize_type.into()], false),
            None,
        );
        let heap_free = module.add_function(
            "coral_heap_free",
            context.void_type().fn_type(&[i8_ptr.into()], false),
            None,
        );

        // Actor operations
        let actor_spawn = module.add_function(
            "coral_actor_spawn",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_send = module.add_function(
            "coral_actor_send",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let actor_stop = module.add_function(
            "coral_actor_stop",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_self = module.add_function(
            "coral_actor_self",
            value_ptr_type.fn_type(&[], false),
            None,
        );

        // Named actor registry operations
        let actor_spawn_named = module.add_function(
            "coral_actor_spawn_named",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let actor_lookup = module.add_function(
            "coral_actor_lookup",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_register = module.add_function(
            "coral_actor_register",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_unregister = module.add_function(
            "coral_actor_unregister",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let actor_send_named = module.add_function(
            "coral_actor_send_named",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let actor_list_named = module.add_function(
            "coral_actor_list_named",
            value_ptr_type.fn_type(&[], false),
            None,
        );

        // Timer operations
        let timer_send_after = module.add_function(
            "coral_timer_send_after",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let timer_schedule_repeat = module.add_function(
            "coral_timer_schedule_repeat",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let timer_cancel = module.add_function(
            "coral_timer_cancel",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let timer_pending_count = module.add_function(
            "coral_timer_pending_count",
            value_ptr_type.fn_type(&[], false),
            None,
        );

        // Tagged value (ADT) operations
        let make_tagged = module.add_function(
            "coral_make_tagged",
            value_ptr_type.fn_type(
                &[i8_ptr.into(), usize_type.into(), value_ptr_ptr_type.into(), usize_type.into()],
                false,
            ),
            None,
        );
        let tagged_is_tag = module.add_function(
            "coral_tagged_is_tag",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), i8_ptr.into(), usize_type.into()],
                false,
            ),
            None,
        );
        let tagged_get_field = module.add_function(
            "coral_tagged_get_field",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), usize_type.into()],
                false,
            ),
            None,
        );

        // String operations
        let string_slice = module.add_function(
            "coral_string_slice",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_char_at = module.add_function(
            "coral_string_char_at",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_index_of = module.add_function(
            "coral_string_index_of",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_split = module.add_function(
            "coral_string_split",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_to_chars = module.add_function(
            "coral_string_to_chars",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let string_starts_with = module.add_function(
            "coral_string_starts_with",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_ends_with = module.add_function(
            "coral_string_ends_with",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_trim = module.add_function(
            "coral_string_trim",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let string_to_upper = module.add_function(
            "coral_string_to_upper",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let string_to_lower = module.add_function(
            "coral_string_to_lower",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let string_replace = module.add_function(
            "coral_string_replace",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_contains = module.add_function(
            "coral_string_contains",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let string_parse_number = module.add_function(
            "coral_string_parse_number",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let number_to_string = module.add_function(
            "coral_number_to_string",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // Error value operations
        let make_error = module.add_function(
            "coral_make_error",
            value_ptr_type.fn_type(&[i32_type.into(), i8_ptr.into(), usize_type.into()], false),
            None,
        );
        let make_absent = module.add_function(
            "coral_make_absent",
            value_ptr_type.fn_type(&[], false),
            None,
        );
        let is_err = module.add_function(
            "coral_is_err",
            i8_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let is_absent = module.add_function(
            "coral_is_absent",
            i8_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let is_ok = module.add_function(
            "coral_is_ok",
            i8_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let error_name = module.add_function(
            "coral_error_name",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let error_code = module.add_function(
            "coral_error_code",
            i32_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let value_or = module.add_function(
            "coral_value_or",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        // Math operations - unary functions (Value -> Value)
        let math_abs = module.add_function(
            "coral_math_abs",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_sqrt = module.add_function(
            "coral_math_sqrt",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_floor = module.add_function(
            "coral_math_floor",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_ceil = module.add_function(
            "coral_math_ceil",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_round = module.add_function(
            "coral_math_round",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_sin = module.add_function(
            "coral_math_sin",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_cos = module.add_function(
            "coral_math_cos",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_tan = module.add_function(
            "coral_math_tan",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_ln = module.add_function(
            "coral_math_ln",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_log10 = module.add_function(
            "coral_math_log10",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_exp = module.add_function(
            "coral_math_exp",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_asin = module.add_function(
            "coral_math_asin",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_acos = module.add_function(
            "coral_math_acos",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_atan = module.add_function(
            "coral_math_atan",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_sinh = module.add_function(
            "coral_math_sinh",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_cosh = module.add_function(
            "coral_math_cosh",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_tanh = module.add_function(
            "coral_math_tanh",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_trunc = module.add_function(
            "coral_math_trunc",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let math_sign = module.add_function(
            "coral_math_sign",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        // Math operations - binary functions (Value, Value -> Value)
        let math_pow = module.add_function(
            "coral_math_pow",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let math_min = module.add_function(
            "coral_math_min",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let math_max = module.add_function(
            "coral_math_max",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let math_atan2 = module.add_function(
            "coral_math_atan2",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        // Universal iterator next
        let value_iter_next = module.add_function(
            "coral_value_iter_next",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // Process/environment
        let process_args = module.add_function(
            "coral_process_args",
            value_ptr_type.fn_type(&[], false),
            None,
        );
        let process_exit = module.add_function(
            "coral_process_exit",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let env_get = module.add_function(
            "coral_env_get",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let env_set = module.add_function(
            "coral_env_set",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        // File I/O extensions
        let fs_append = module.add_function(
            "coral_fs_append",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let fs_read_dir = module.add_function(
            "coral_fs_read_dir",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_mkdir = module.add_function(
            "coral_fs_mkdir",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_delete = module.add_function(
            "coral_fs_delete",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let fs_is_dir = module.add_function(
            "coral_fs_is_dir",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // stdin
        let stdin_read_line = module.add_function(
            "coral_stdin_read_line",
            value_ptr_type.fn_type(&[], false),
            None,
        );

        // List extensions
        let list_contains = module.add_function(
            "coral_list_contains",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_index_of = module.add_function(
            "coral_list_index_of",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_reverse = module.add_function(
            "coral_list_reverse",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_slice = module.add_function(
            "coral_list_slice",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );
        let list_sort = module.add_function(
            "coral_list_sort",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let list_join = module.add_function(
            "coral_list_join",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let list_concat = module.add_function(
            "coral_list_concat",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        // Map extensions
        let map_remove = module.add_function(
            "coral_map_remove",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let map_values = module.add_function(
            "coral_map_values",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_entries = module.add_function(
            "coral_map_entries",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let map_has_key = module.add_function(
            "coral_map_has_key",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let map_merge = module.add_function(
            "coral_map_merge",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        // Bytes extensions
        let bytes_get = module.add_function(
            "coral_bytes_get",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );
        let bytes_from_string = module.add_function(
            "coral_bytes_from_string",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let bytes_to_string = module.add_function(
            "coral_bytes_to_string",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let bytes_slice_val = module.add_function(
            "coral_bytes_slice_val",
            value_ptr_type.fn_type(
                &[value_ptr_type.into(), value_ptr_type.into(), value_ptr_type.into()],
                false,
            ),
            None,
        );

        // Type reflection
        let type_of = module.add_function(
            "coral_type_of",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );

        // Character operations
        let string_ord = module.add_function(
            "coral_string_ord",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let string_chr = module.add_function(
            "coral_string_chr",
            value_ptr_type.fn_type(&[value_ptr_type.into()], false),
            None,
        );
        let string_compare = module.add_function(
            "coral_string_compare",
            value_ptr_type.fn_type(&[value_ptr_type.into(), value_ptr_type.into()], false),
            None,
        );

        Self {
            value_ptr_type,
            make_number,
            make_bool,
            make_string,
            make_bytes,
            make_unit,
            make_list,
            make_list_hinted,
            value_as_number,
            value_as_bool,
            value_add,
            value_equals,
            value_not_equals,
            value_hash,
            value_bitand,
            value_bitor,
            value_bitxor,
            value_bitnot,
            value_shift_left,
            value_shift_right,
            value_iter,
            list_push,
            list_get,
            list_pop,
            list_iter,
            list_iter_next,
            list_map,
            list_filter,
            list_reduce,
            map_get,
            map_set,
            map_length,
            map_keys,
            map_iter,
            map_iter_next,
            value_length,
            make_map,
            make_map_hinted,
            map_entry_type,
            make_closure,
            closure_invoke,
            log,
            fs_read,
            fs_write,
            fs_exists,
            value_retain,
            value_release,
            heap_alloc,
            heap_free,
            actor_spawn,
            actor_send,
            actor_stop,
            actor_self,
            actor_spawn_named,
            actor_lookup,
            actor_register,
            actor_unregister,
            actor_send_named,
            actor_list_named,
            timer_send_after,
            timer_schedule_repeat,
            timer_cancel,
            timer_pending_count,
            closure_invoke_type,
            closure_release_type,
            make_tagged,
            tagged_is_tag,
            tagged_get_field,
            string_slice,
            string_char_at,
            string_index_of,
            string_split,
            string_to_chars,
            string_starts_with,
            string_ends_with,
            string_trim,
            string_to_upper,
            string_to_lower,
            string_replace,
            string_contains,
            string_parse_number,
            number_to_string,
            make_error,
            make_absent,
            is_err,
            is_absent,
            is_ok,
            error_name,
            error_code,
            value_or,
            math_abs,
            math_sqrt,
            math_floor,
            math_ceil,
            math_round,
            math_sin,
            math_cos,
            math_tan,
            math_pow,
            math_min,
            math_max,
            math_ln,
            math_log10,
            math_exp,
            math_asin,
            math_acos,
            math_atan,
            math_atan2,
            math_sinh,
            math_cosh,
            math_tanh,
            math_trunc,
            math_sign,
            value_iter_next,
            process_args,
            process_exit,
            env_get,
            env_set,
            fs_append,
            fs_read_dir,
            fs_mkdir,
            fs_delete,
            fs_is_dir,
            stdin_read_line,
            list_contains,
            list_index_of,
            list_reverse,
            list_slice,
            list_sort,
            list_join,
            list_concat,
            map_remove,
            map_values,
            map_entries,
            map_has_key,
            map_merge,
            bytes_get,
            bytes_from_string,
            bytes_to_string,
            bytes_slice_val,
            type_of,
            string_ord,
            string_chr,
            string_compare,
        }
    }
}
