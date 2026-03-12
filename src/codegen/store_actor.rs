//! Store and Actor constructor/method codegen for Coral.
//!
//! Contains store constructor building, store method body compilation,
//! actor constructor building, and actor handler invoke/release.

use super::*;

impl<'ctx> CodeGenerator<'ctx> {
    pub(super) fn build_actor_constructor(&mut self, store: &crate::ast::StoreDefinition) -> Result<(), Diagnostic> {
        let constructor_name = format!("make_{}", store.name);
        let ctor_fn = *self.functions.get(&constructor_name).unwrap();
        let entry = self.context.append_basic_block(ctor_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();

        // 1. Create state Map with field defaults
        let null_entries = self.runtime.map_entry_type
            .ptr_type(AddressSpace::default())
            .const_null();
        let zero_len = self.usize_type.const_zero();
        let state_map = self.call_runtime_ptr(self.runtime.make_map, &[null_entries.into(), zero_len.into()], "actor_state");
        // Emit a dummy context for field evaluation
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: ctor_fn,
            loop_stack: Vec::new(),
            di_scope: None,
            fn_name: String::new(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
            lambda_out_param: None,
        };
        for field in &store.fields {
            let key = self.emit_string_literal(&field.name);
            let value = if let Some(default) = &field.default {
                self.emit_expression(&mut ctx, default)?
            } else {
                // Default to 0 for fields without default
                self.wrap_number(self.f64_type.const_float(0.0))
            };
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[state_map.into(), key.into(), value.into()],
                "set_field",
            );
        }

        // 2. Build the handler invoke function (follows closure signature)
        // Signature: fn(env: *mut c_void, args: *const ValueHandle, len: usize, out: *mut ValueHandle)
        let handler_fn_name = format!("__{}_handler_invoke", store.name);
        let handler_invoke_fn = self.module.add_function(
            &handler_fn_name,
            self.runtime.closure_invoke_type,
            None,
        );
        self.build_actor_handler_invoke(handler_invoke_fn, store)?;

        // 3. Build the handler release function
        let release_fn_name = format!("__{}_handler_release", store.name);
        let release_fn = self.module.add_function(
            &release_fn_name,
            self.runtime.closure_release_type,
            None,
        );
        self.build_actor_handler_release(release_fn);

        // 4. Back to constructor: create closure with state as env and spawn actor
        self.builder.position_at_end(entry);
        let handler_closure = self.call_runtime_ptr(
            self.runtime.make_closure,
            &[
                handler_invoke_fn.as_global_value().as_pointer_value().into(),
                release_fn.as_global_value().as_pointer_value().into(),
                state_map.into(), // state Map as closure environment
                self.usize_type.const_zero().into(), // actor env is not NaN-boxed captures
            ],
            "handler_closure",
        );

        let actor_handle = self.call_runtime_ptr(
            self.runtime.actor_spawn,
            &[handler_closure.into()],
            "actor",
        );

        self.builder.build_return(Some(&actor_handle)).unwrap();
        Ok(())
    }

    /// Build handler invoke function for actor message dispatch.
    /// env = actor state Map, args[0] = self (actor handle), args[1] = message
    pub(super) fn build_actor_handler_invoke(
        &mut self,
        invoke_fn: FunctionValue<'ctx>,
        store: &crate::ast::StoreDefinition,
    ) -> Result<(), Diagnostic> {
        let saved_block = self.builder.get_insert_block();
        let handler_entry = self.context.append_basic_block(invoke_fn, "entry");
        self.builder.position_at_end(handler_entry);

        // env = state Map (ValuePtr cast from *mut c_void)
        let env_param = invoke_fn.get_nth_param(0).unwrap().into_pointer_value();
        let args_param = invoke_fn.get_nth_param(1).unwrap().into_pointer_value();
        // Cast env to ValuePtr for state access
        let state = self.builder.build_pointer_cast(
            env_param,
            self.runtime.value_ptr_type,
            "actor_state",
        ).unwrap();
        
        // args[1] = message
        let msg_index = self.usize_type.const_int(1, false);
        let msg_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.runtime.value_ptr_type,
                args_param,
                &[msg_index],
                "msg_ptr",
            ).unwrap()
        };
        let msg_param = self.builder.build_load(
            self.runtime.value_ptr_type,
            msg_ptr,
            "msg",
        ).unwrap().into_pointer_value();

        // Extract message name/data and dispatch to @message methods
        let name_key = self.emit_string_literal("name");
        let name_key_ptr = self.nb_to_ptr(name_key);
        let data_key = self.emit_string_literal("data");
        let data_key_ptr = self.nb_to_ptr(data_key);
        let name_field = self.call_runtime_ptr(
            self.runtime.map_get,
            &[msg_param.into(), name_key_ptr.into()],
            "msg_name",
        );
        let data_field = self.call_runtime_ptr(
            self.runtime.map_get,
            &[msg_param.into(), data_key_ptr.into()],
            "msg_data",
        );

        // Collect message handlers
        let handlers: Vec<_> = store.methods.iter()
            .filter(|m| m.kind == crate::ast::FunctionKind::ActorMessage)
            .collect();

        if handlers.is_empty() {
            // No handlers, just return
            self.builder.build_return(None).unwrap();
            if let Some(block) = saved_block {
                self.builder.position_at_end(block);
            }
            return Ok(());
        }

        // R2.3: Fast dispatch via coral_msg_dispatch + LLVM switch.
        // Build dispatch table: arrays of string pointers and lengths.
        let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
        let handler_count = handlers.len();

        // Create compile-time string constant arrays for the dispatch table
        let mut ptr_values = Vec::with_capacity(handler_count);
        let mut len_values = Vec::with_capacity(handler_count);
        for method in &handlers {
            let str_global = self.get_or_create_string_constant(&method.name);
            let str_ptr = self.builder.build_pointer_cast(
                str_global.as_pointer_value(), i8_ptr_type, "str_ptr"
            ).unwrap();
            ptr_values.push(str_ptr);
            len_values.push(self.usize_type.const_int(method.name.len() as u64, false));
        }

        // Allocate arrays on the stack
        let ptrs_array = self.builder.build_array_alloca(
            i8_ptr_type,
            self.usize_type.const_int(handler_count as u64, false),
            "dispatch_ptrs",
        ).unwrap();
        let lens_array = self.builder.build_array_alloca(
            self.usize_type,
            self.usize_type.const_int(handler_count as u64, false),
            "dispatch_lens",
        ).unwrap();

        // Fill the arrays
        for (i, (ptr_val, len_val)) in ptr_values.iter().zip(len_values.iter()).enumerate() {
            let idx = self.usize_type.const_int(i as u64, false);
            let ptr_slot = unsafe {
                self.builder.build_in_bounds_gep(i8_ptr_type, ptrs_array, &[idx], "ptr_slot").unwrap()
            };
            self.builder.build_store(ptr_slot, *ptr_val).unwrap();
            let len_slot = unsafe {
                self.builder.build_in_bounds_gep(self.usize_type, lens_array, &[idx], "len_slot").unwrap()
            };
            self.builder.build_store(len_slot, *len_val).unwrap();
        }

        let count_val = self.usize_type.const_int(handler_count as u64, false);

        // Call coral_msg_dispatch → returns handler index (i64) or -1
        let dispatch_result = self.builder.build_call(
            self.runtime.msg_dispatch,
            &[name_field.into(), ptrs_array.into(), lens_array.into(), count_val.into()],
            "dispatch_idx",
        ).unwrap().try_as_basic_value().left().unwrap().into_int_value();

        // Build LLVM switch over the dispatch result
        let done_bb = self.context.append_basic_block(invoke_fn, "msg_done");
        let default_bb = self.context.append_basic_block(invoke_fn, "msg_default");

        let mut cases: Vec<(inkwell::values::IntValue, inkwell::basic_block::BasicBlock)> = Vec::new();
        for (i, method) in handlers.iter().enumerate() {
            let case_bb = self.context.append_basic_block(invoke_fn, &format!("msg_{}", method.name));
            cases.push((self.context.i64_type().const_int(i as u64, false), case_bb));

            self.builder.position_at_end(case_bb);
            let mangled = format!("{}_{}", store.name, method.name);
            if let Some(target_fn) = self.functions.get(&mangled).copied() {
                let mut args: Vec<BasicMetadataValueEnum> = vec![state.into()];
                if !method.params.is_empty() {
                    args.push(data_field.into());
                }
                let _ = self.builder.build_call(target_fn, &args, "call_msg_fn");
            }
            self.builder.build_unconditional_branch(done_bb).unwrap();
        }

        // Default case: no match, just branch to done
        self.builder.position_at_end(default_bb);
        self.builder.build_unconditional_branch(done_bb).unwrap();

        // Emit the switch from the entry block
        self.builder.position_at_end(handler_entry);
        self.builder.build_switch(dispatch_result, default_bb, &cases).unwrap();

        self.builder.position_at_end(done_bb);
        self.builder.build_return(None).unwrap();

        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(())
    }

    /// Release function for actor handler closure - releases state Map
    pub(super) fn build_actor_handler_release(&mut self, release_fn: FunctionValue<'ctx>) {
        let saved_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(release_fn, "entry");
        self.builder.position_at_end(entry);
        
        let env_param = release_fn.get_first_param().unwrap().into_pointer_value();
        let is_null = self.builder.build_is_null(env_param, "env_is_null").unwrap();
        
        let exit_block = self.context.append_basic_block(release_fn, "release_exit");
        let body_block = self.context.append_basic_block(release_fn, "release_body");
        self.builder.build_conditional_branch(is_null, exit_block, body_block).unwrap();
        
        self.builder.position_at_end(body_block);
        // Release the state Map
        let state = self.builder.build_pointer_cast(
            env_param,
            self.runtime.value_ptr_type,
            "actor_state",
        ).unwrap();
        self.builder.build_call(
            self.runtime.value_release,
            &[state.into()],
            "",
        ).unwrap();
        self.builder.build_unconditional_branch(exit_block).unwrap();
        
        self.builder.position_at_end(exit_block);
        self.builder.build_return(None).unwrap();
        
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
    }

    /// Build constructor for non-actor stores.
    /// Creates a Map with all fields initialized to defaults, returns the Map.
    /// For persistent stores, also calls coral_store_open + coral_store_create.
    pub(super) fn build_store_constructor(&mut self, store: &crate::ast::StoreDefinition) -> Result<(), Diagnostic> {
        let constructor_name = format!("make_{}", store.name);
        let ctor_fn = *self.functions.get(&constructor_name).unwrap();
        let entry = self.context.append_basic_block(ctor_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();

        // Create store as a Map with fields
        let null_entries = self.runtime.map_entry_type
            .ptr_type(AddressSpace::default())
            .const_null();
        let zero_len = self.usize_type.const_zero();
        let store_map = self.call_runtime_ptr(
            self.runtime.make_map,
            &[null_entries.into(), zero_len.into()],
            "store_data",
        );
        
        // Create a dummy context for field evaluation
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: ctor_fn,
            loop_stack: Vec::new(),
            di_scope: None,
            fn_name: String::new(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
            lambda_out_param: None,
        };
        
        // Set __type__ field so we know what store type this is for method dispatch
        let type_key = self.emit_string_literal("__type__");
        let type_key_ptr = self.nb_to_ptr(type_key);
        let type_value = self.emit_string_literal(&store.name);
        let type_value_ptr = self.nb_to_ptr(type_value);
        self.call_runtime_ptr(
            self.runtime.map_set,
            &[store_map.into(), type_key_ptr.into(), type_value_ptr.into()],
            "set_type",
        );
        
        // Initialize each field
        for field in &store.fields {
            let key = self.emit_string_literal(&field.name);
            let key_ptr = self.nb_to_ptr(key);
            let value = if let Some(default) = &field.default {
                let val = self.emit_expression(&mut ctx, default)?;
                // For reference fields, retain the initial value
                if field.is_reference {
                    self.call_nb_void(self.runtime.nb_retain, &[val.into()]);
                }
                val
            } else if field.is_reference {
                // Reference fields default to unit (null reference)
                let unit_ptr = self.call_runtime_ptr(self.runtime.make_unit, &[], "null_ref");
                self.ptr_to_nb(unit_ptr)
            } else {
                // Value fields default to 0
                self.wrap_number(self.f64_type.const_float(0.0))
            };
            // Convert NaN-boxed value to Value* for map_set
            let value_ptr = self.nb_to_ptr(value);
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[store_map.into(), key_ptr.into(), value_ptr.into()],
                "set_field",
            );
        }

        if store.is_persistent {
            // For persistent stores:
            // 1. Open the store engine via coral_store_open(type_ptr, type_len, name_ptr, name_len, path_ptr, path_len)
            // 2. Create a persistent object via coral_store_create(handle, fields_map)
            // 3. Stash the handle in __store_handle__ for later operations
            // 4. Return the enriched object map from coral_store_create

            let i8_ptr_type = self.i8_type.ptr_type(AddressSpace::default());
            
            // Store type name (e.g., "Counter")
            let type_global = self.get_or_create_string_constant(&store.name);
            let type_ptr = self.builder.build_pointer_cast(
                type_global.as_pointer_value(), i8_ptr_type, "type_ptr"
            ).unwrap();
            let type_len = self.usize_type.const_int(store.name.len() as u64, false);

            // Store name = "default" (can be parameterized later)
            let name_global = self.get_or_create_string_constant("default");
            let name_ptr = self.builder.build_pointer_cast(
                name_global.as_pointer_value(), i8_ptr_type, "name_ptr"
            ).unwrap();
            let name_len = self.usize_type.const_int(7, false); // "default".len()

            // Data path = ".coral_data" (default; can be parameterized later)
            let path_global = self.get_or_create_string_constant(".coral_data");
            let path_ptr = self.builder.build_pointer_cast(
                path_global.as_pointer_value(), i8_ptr_type, "path_ptr"
            ).unwrap();
            let path_len = self.usize_type.const_int(11, false); // ".coral_data".len()

            let store_handle = self.call_runtime_ptr(
                self.runtime.store_open,
                &[
                    type_ptr.into(), type_len.into(),
                    name_ptr.into(), name_len.into(),
                    path_ptr.into(), path_len.into(),
                ],
                "store_handle",
            );

            // Stash the handle in the map so methods can use it for persistence
            let handle_key = self.emit_string_literal("__store_handle__");
            let handle_key_ptr = self.nb_to_ptr(handle_key);
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[store_map.into(), handle_key_ptr.into(), store_handle.into()],
                "set_handle",
            );

            // Create the persistent object: coral_store_create(handle, fields_map)
            // The fields_map already has all user fields populated.
            let created = self.call_runtime_ptr(
                self.runtime.store_create,
                &[store_handle.into(), store_map.into()],
                "store_created",
            );

            // Copy the __type__ and __store_handle__ into the created map for method dispatch
            let type_key2 = self.emit_string_literal("__type__");
            let type_key2_ptr = self.nb_to_ptr(type_key2);
            let type_val2 = self.emit_string_literal(&store.name);
            let type_val2_ptr = self.nb_to_ptr(type_val2);
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[created.into(), type_key2_ptr.into(), type_val2_ptr.into()],
                "set_type_created",
            );
            let handle_key2 = self.emit_string_literal("__store_handle__");
            let handle_key2_ptr = self.nb_to_ptr(handle_key2);
            self.call_runtime_ptr(
                self.runtime.map_set,
                &[created.into(), handle_key2_ptr.into(), store_handle.into()],
                "set_handle_created",
            );

            let created_nb = self.ptr_to_nb(created);
            self.builder.build_return(Some(&created_nb)).unwrap();
        } else {
            let store_nb = self.ptr_to_nb(store_map);
            self.builder.build_return(Some(&store_nb)).unwrap();
        }
        Ok(())
    }

    /// Build body for a store method.
    /// Store methods have self (store Map) as hidden first parameter.
    pub(super) fn build_store_method_body(
        &mut self,
        function: &Function,
        llvm_fn: FunctionValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let entry = self.context.append_basic_block(llvm_fn, "entry");
        self.builder.position_at_end(entry);
        self.ensure_globals_initialized();
        let mut ctx = FunctionContext {
            variables: HashMap::new(),
            variable_allocas: HashMap::new(),
            function: llvm_fn,
            loop_stack: Vec::new(),
            di_scope: None,
            fn_name: String::new(),
            in_tail_position: false,
            cse_cache: HashMap::new(),
            lambda_out_param: None,
        };

        // First param is the store (Map), inject as `self`
        let store_val = llvm_fn.get_nth_param(0).unwrap().into_int_value();
        self.store_variable(&mut ctx, "self", store_val);

        // Remaining params are NaN-boxed i64 values
        for (i, param_ast) in function.params.iter().enumerate() {
            let param = llvm_fn.get_nth_param((i + 1) as u32).unwrap();
            let value = param.into_int_value();
            self.store_variable(&mut ctx, &param_ast.name, value);
        }

        let block_value = self.emit_block(&mut ctx, &function.body)?;
        // Return the value directly as ptr (CoralValue*) instead of converting to f64
        self.builder.build_return(Some(&block_value)).unwrap();
        Ok(())
    }
}