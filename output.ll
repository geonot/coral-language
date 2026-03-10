; ModuleID = 'coral_module'
source_filename = "coral_module"

@coral_global_true_value = global i64 0
@coral_global_false_value = global i64 0
@__coral_globals_initialized = global i1 false
@str_0 = private unnamed_addr constant [14 x i8] c"Hello, Coral!\00", align 1
@str_1 = private unnamed_addr constant [6 x i8] c"World\00", align 1
@str_2 = private unnamed_addr constant [8 x i8] c"Hello, \00", align 1
@str_3 = private unnamed_addr constant [2 x i8] c"!\00", align 1
@str_4 = private unnamed_addr constant [15 x i8] c"The answer is \00", align 1
@str_5 = private unnamed_addr constant [21 x i8] c"Pi is approximately \00", align 1
@str_6 = private unnamed_addr constant [8 x i8] c"Ready: \00", align 1
@str_7 = private unnamed_addr constant [6 x i8] c"apple\00", align 1
@str_8 = private unnamed_addr constant [7 x i8] c"banana\00", align 1
@str_9 = private unnamed_addr constant [7 x i8] c"cherry\00", align 1
@str_10 = private unnamed_addr constant [9 x i8] c"Fruits: \00", align 1
@str_11 = private unnamed_addr constant [14 x i8] c"First fruit: \00", align 1
@str_12 = private unnamed_addr constant [5 x i8] c"host\00", align 1
@str_13 = private unnamed_addr constant [10 x i8] c"localhost\00", align 1
@str_14 = private unnamed_addr constant [5 x i8] c"port\00", align 1
@str_15 = private unnamed_addr constant [6 x i8] c"debug\00", align 1
@str_16 = private unnamed_addr constant [7 x i8] c"Host: \00", align 1
@str_17 = private unnamed_addr constant [4 x i8] c"big\00", align 1
@str_18 = private unnamed_addr constant [6 x i8] c"small\00", align 1
@str_19 = private unnamed_addr constant [11 x i8] c"Answer is \00", align 1
@str_20 = private unnamed_addr constant [17 x i8] c"Program complete\00", align 1

declare ptr @coral_nb_to_handle(i64)

declare i64 @coral_nb_from_handle(ptr)

declare i64 @coral_nb_make_number(double)

declare i64 @coral_nb_make_bool(i8)

declare i64 @coral_nb_make_unit()

declare i64 @coral_nb_make_none()

declare i64 @coral_nb_make_string(ptr, i64)

declare double @coral_nb_as_number(i64)

declare i8 @coral_nb_as_bool(i64)

declare i8 @coral_nb_tag(i64)

declare i8 @coral_nb_is_truthy(i64)

declare i8 @coral_nb_is_err(i64)

declare i8 @coral_nb_is_absent(i64)

declare void @coral_nb_retain(i64)

declare void @coral_nb_release(i64)

declare i64 @coral_nb_add(i64, i64)

declare i64 @coral_nb_sub(i64, i64)

declare i64 @coral_nb_mul(i64, i64)

declare i64 @coral_nb_div(i64, i64)

declare i64 @coral_nb_rem(i64, i64)

declare i64 @coral_nb_neg(i64)

declare i64 @coral_nb_equals(i64, i64)

declare i64 @coral_nb_not_equals(i64, i64)

declare i64 @coral_nb_less_than(i64, i64)

declare i64 @coral_nb_greater_than(i64, i64)

declare i64 @coral_nb_less_equal(i64, i64)

declare i64 @coral_nb_greater_equal(i64, i64)

declare void @coral_nb_print(i64)

declare void @coral_nb_println(i64)

declare ptr @coral_make_number(double)

declare ptr @coral_make_bool(i8)

declare ptr @coral_make_string(ptr, i64)

declare ptr @coral_make_bytes(ptr, i64)

declare ptr @coral_make_unit()

declare ptr @coral_make_list(ptr, i64)

declare ptr @coral_make_list_hinted(ptr, i64, i8)

declare ptr @coral_make_map(ptr, i64)

declare ptr @coral_make_map_hinted(ptr, i64, i8)

declare ptr @coral_list_push(ptr, ptr)

declare ptr @coral_list_get(ptr, ptr)

declare ptr @coral_list_pop(ptr)

declare ptr @coral_list_iter(ptr)

declare ptr @coral_list_iter_next(ptr)

declare ptr @coral_list_map(ptr, ptr)

declare ptr @coral_list_filter(ptr, ptr)

declare ptr @coral_list_reduce(ptr, ptr, ptr)

declare ptr @coral_map_get(ptr, ptr)

declare ptr @coral_map_set(ptr, ptr, ptr)

declare ptr @coral_map_length(ptr)

declare ptr @coral_map_keys(ptr)

declare ptr @coral_map_iter(ptr)

declare ptr @coral_map_iter_next(ptr)

declare ptr @coral_value_length(ptr)

declare ptr @coral_value_get(ptr, ptr)

declare ptr @coral_field_or_length(ptr, ptr)

declare ptr @coral_value_iter(ptr)

declare double @coral_value_as_number(ptr)

declare i8 @coral_value_as_bool(ptr)

declare ptr @coral_value_add(ptr, ptr)

declare ptr @coral_value_equals(ptr, ptr)

declare ptr @coral_value_not_equals(ptr, ptr)

declare i64 @coral_value_hash(ptr)

declare ptr @coral_value_bitand(ptr, ptr)

declare ptr @coral_value_bitor(ptr, ptr)

declare ptr @coral_value_bitxor(ptr, ptr)

declare ptr @coral_value_bitnot(ptr)

declare ptr @coral_value_shift_left(ptr, ptr)

declare ptr @coral_value_shift_right(ptr, ptr)

declare ptr @coral_log(ptr)

declare ptr @coral_fs_read(ptr)

declare ptr @coral_fs_write(ptr, ptr)

declare ptr @coral_fs_exists(ptr)

declare ptr @coral_make_closure(ptr, ptr, ptr)

declare ptr @coral_closure_invoke(ptr, ptr, i64)

declare void @coral_value_retain(ptr)

declare void @coral_value_release(ptr)

declare ptr @coral_heap_alloc(i64)

declare void @coral_heap_free(ptr)

declare ptr @coral_actor_spawn(ptr)

declare ptr @coral_actor_send(ptr, ptr)

declare ptr @coral_actor_stop(ptr)

declare ptr @coral_actor_self()

declare ptr @coral_actor_spawn_named(ptr, ptr)

declare ptr @coral_actor_lookup(ptr)

declare ptr @coral_actor_register(ptr)

declare ptr @coral_actor_unregister(ptr)

declare ptr @coral_actor_send_named(ptr, ptr)

declare ptr @coral_actor_list_named()

declare ptr @coral_timer_send_after(ptr, ptr, ptr)

declare ptr @coral_timer_schedule_repeat(ptr, ptr, ptr)

declare ptr @coral_timer_cancel(ptr)

declare ptr @coral_timer_pending_count()

declare ptr @coral_main_wait()

declare ptr @coral_main_done_signal()

declare ptr @coral_make_tagged(ptr, i64, ptr, i64)

declare ptr @coral_tagged_is_tag(ptr, ptr, i64)

declare ptr @coral_tagged_get_field(ptr, i64)

declare ptr @coral_string_slice(ptr, ptr, ptr)

declare ptr @coral_string_char_at(ptr, ptr)

declare ptr @coral_string_index_of(ptr, ptr)

declare ptr @coral_string_split(ptr, ptr)

declare ptr @coral_string_to_chars(ptr)

declare ptr @coral_string_starts_with(ptr, ptr)

declare ptr @coral_string_ends_with(ptr, ptr)

declare ptr @coral_string_trim(ptr)

declare ptr @coral_string_to_upper(ptr)

declare ptr @coral_string_to_lower(ptr)

declare ptr @coral_string_replace(ptr, ptr, ptr)

declare ptr @coral_string_contains(ptr, ptr)

declare ptr @coral_string_parse_number(ptr)

declare ptr @coral_number_to_string(ptr)

declare ptr @coral_make_error(i32, ptr, i64)

declare ptr @coral_make_absent()

declare i8 @coral_is_err(ptr)

declare i8 @coral_is_absent(ptr)

declare i8 @coral_is_ok(ptr)

declare ptr @coral_error_name(ptr)

declare i32 @coral_error_code(ptr)

declare ptr @coral_value_or(ptr, ptr)

declare ptr @coral_math_abs(ptr)

declare ptr @coral_math_sqrt(ptr)

declare ptr @coral_math_floor(ptr)

declare ptr @coral_math_ceil(ptr)

declare ptr @coral_math_round(ptr)

declare ptr @coral_math_sin(ptr)

declare ptr @coral_math_cos(ptr)

declare ptr @coral_math_tan(ptr)

declare ptr @coral_math_ln(ptr)

declare ptr @coral_math_log10(ptr)

declare ptr @coral_math_exp(ptr)

declare ptr @coral_math_asin(ptr)

declare ptr @coral_math_acos(ptr)

declare ptr @coral_math_atan(ptr)

declare ptr @coral_math_sinh(ptr)

declare ptr @coral_math_cosh(ptr)

declare ptr @coral_math_tanh(ptr)

declare ptr @coral_math_trunc(ptr)

declare ptr @coral_math_sign(ptr)

declare ptr @coral_math_pow(ptr, ptr)

declare ptr @coral_math_min(ptr, ptr)

declare ptr @coral_math_max(ptr, ptr)

declare ptr @coral_math_atan2(ptr, ptr)

declare ptr @coral_value_iter_next(ptr)

declare ptr @coral_process_args()

declare ptr @coral_process_exit(ptr)

declare ptr @coral_env_get(ptr)

declare ptr @coral_env_set(ptr, ptr)

declare ptr @coral_fs_append(ptr, ptr)

declare ptr @coral_fs_read_dir(ptr)

declare ptr @coral_fs_mkdir(ptr)

declare ptr @coral_fs_delete(ptr)

declare ptr @coral_fs_is_dir(ptr)

declare ptr @coral_stdin_read_line()

declare ptr @coral_list_contains(ptr, ptr)

declare ptr @coral_list_index_of(ptr, ptr)

declare ptr @coral_list_reverse(ptr)

declare ptr @coral_list_slice(ptr, ptr, ptr)

declare ptr @coral_list_sort(ptr)

declare ptr @coral_list_join(ptr, ptr)

declare ptr @coral_list_concat(ptr, ptr)

declare ptr @coral_range(ptr, ptr)

declare ptr @coral_sb_new()

declare void @coral_sb_push(ptr, ptr)

declare ptr @coral_sb_finish(ptr)

declare ptr @coral_sb_len(ptr)

declare ptr @coral_string_join_list(ptr, ptr)

declare ptr @coral_string_repeat(ptr, ptr)

declare ptr @coral_string_reverse(ptr)

declare ptr @coral_value_to_string(ptr)

declare ptr @coral_map_remove(ptr, ptr)

declare ptr @coral_map_values(ptr)

declare ptr @coral_map_entries(ptr)

declare ptr @coral_map_has_key(ptr, ptr)

declare ptr @coral_map_merge(ptr, ptr)

declare ptr @coral_bytes_get(ptr, ptr)

declare ptr @coral_bytes_from_string(ptr)

declare ptr @coral_bytes_to_string(ptr)

declare ptr @coral_bytes_slice_val(ptr, ptr, ptr)

declare ptr @coral_type_of(ptr)

declare ptr @coral_string_ord(ptr)

declare ptr @coral_string_chr(ptr)

declare ptr @coral_string_compare(ptr, ptr)

declare ptr @coral_store_open(ptr, i64, ptr, i64, ptr, i64)

declare ptr @coral_store_close(ptr)

declare ptr @coral_store_save_all()

declare ptr @coral_store_create(ptr, ptr)

declare ptr @coral_store_get_by_index(ptr, ptr)

declare ptr @coral_store_get_by_uuid(ptr, ptr, i64)

declare ptr @coral_store_update(ptr, ptr, ptr)

declare ptr @coral_store_soft_delete(ptr, ptr)

declare ptr @coral_store_stats(ptr)

declare ptr @coral_store_count(ptr)

declare ptr @coral_store_persist(ptr)

declare ptr @coral_store_checkpoint(ptr)

declare ptr @coral_store_all_indices(ptr)

declare ptr @coral_json_parse(ptr)

declare ptr @coral_json_serialize(ptr)

declare ptr @coral_json_serialize_pretty(ptr)

declare ptr @coral_time_now()

declare ptr @coral_time_timestamp()

declare ptr @coral_time_format_iso(ptr)

declare ptr @coral_time_year(ptr)

declare ptr @coral_time_month(ptr)

declare ptr @coral_time_day(ptr)

declare ptr @coral_time_hour(ptr)

declare ptr @coral_time_minute(ptr)

declare ptr @coral_time_second(ptr)

declare ptr @coral_string_lines(ptr)

declare ptr @coral_list_sort_natural(ptr)

declare ptr @coral_bytes_from_hex(ptr)

declare ptr @coral_bytes_contains(ptr, ptr)

declare ptr @coral_bytes_find(ptr, ptr)

declare ptr @coral_base64_encode(ptr)

declare ptr @coral_base64_decode(ptr)

declare ptr @coral_hex_encode(ptr)

declare ptr @coral_hex_decode(ptr)

declare ptr @coral_tcp_listen(ptr, ptr)

declare ptr @coral_tcp_accept(ptr)

declare ptr @coral_tcp_connect(ptr, ptr)

declare ptr @coral_tcp_read(ptr, ptr)

declare ptr @coral_tcp_write(ptr, ptr)

declare ptr @coral_tcp_close(ptr)

declare ptr @coral_actor_monitor(ptr, ptr)

declare ptr @coral_actor_demonitor(ptr, ptr)

declare ptr @coral_actor_graceful_stop(ptr)

define i64 @__user_main() {
entry:
  %status_slot = alloca i64, align 8
  %config_slot = alloca i64, align 8
  %fruits_slot = alloca i64, align 8
  %ready_slot = alloca i64, align 8
  %pi_slot = alloca i64, align 8
  %answer_slot = alloca i64, align 8
  %greeting_slot = alloca i64, align 8
  %name_slot = alloca i64, align 8
  call void @__coral_init_globals()
  %nb_str = call i64 @coral_nb_make_string(ptr @str_0, i64 13)
  %nb_to_ptr = call ptr @coral_nb_to_handle(i64 %nb_str)
  %log_call = call ptr @coral_log(ptr %nb_to_ptr)
  %ptr_to_nb = call i64 @coral_nb_from_handle(ptr %log_call)
  %nb_str1 = call i64 @coral_nb_make_string(ptr @str_1, i64 5)
  store i64 %nb_str1, ptr %name_slot, align 4
  %nb_str2 = call i64 @coral_nb_make_string(ptr @str_2, i64 7)
  %load_name = load i64, ptr %name_slot, align 4
  %nb_add = call i64 @coral_nb_add(i64 %nb_str2, i64 %load_name)
  %nb_str3 = call i64 @coral_nb_make_string(ptr @str_3, i64 1)
  %nb_add4 = call i64 @coral_nb_add(i64 %nb_add, i64 %nb_str3)
  store i64 %nb_add4, ptr %greeting_slot, align 4
  %load_greeting = load i64, ptr %greeting_slot, align 4
  %nb_to_ptr5 = call ptr @coral_nb_to_handle(i64 %load_greeting)
  %log_call6 = call ptr @coral_log(ptr %nb_to_ptr5)
  %ptr_to_nb7 = call i64 @coral_nb_from_handle(ptr %log_call6)
  %nb_num = call i64 @coral_nb_make_number(double 4.200000e+01)
  store i64 %nb_num, ptr %answer_slot, align 4
  %nb_num8 = call i64 @coral_nb_make_number(double 3.141590e+00)
  store i64 %nb_num8, ptr %pi_slot, align 4
  %nb_str9 = call i64 @coral_nb_make_string(ptr @str_4, i64 14)
  %load_answer = load i64, ptr %answer_slot, align 4
  %nb_add10 = call i64 @coral_nb_add(i64 %nb_str9, i64 %load_answer)
  %nb_to_ptr11 = call ptr @coral_nb_to_handle(i64 %nb_add10)
  %log_call12 = call ptr @coral_log(ptr %nb_to_ptr11)
  %ptr_to_nb13 = call i64 @coral_nb_from_handle(ptr %log_call12)
  %nb_str14 = call i64 @coral_nb_make_string(ptr @str_5, i64 20)
  %load_pi = load i64, ptr %pi_slot, align 4
  %nb_add15 = call i64 @coral_nb_add(i64 %nb_str14, i64 %load_pi)
  %nb_to_ptr16 = call ptr @coral_nb_to_handle(i64 %nb_add15)
  %log_call17 = call ptr @coral_log(ptr %nb_to_ptr16)
  %ptr_to_nb18 = call i64 @coral_nb_from_handle(ptr %log_call17)
  %nb_bool = call i64 @coral_nb_make_bool(i8 1)
  store i64 %nb_bool, ptr %ready_slot, align 4
  %nb_str19 = call i64 @coral_nb_make_string(ptr @str_6, i64 7)
  %load_ready = load i64, ptr %ready_slot, align 4
  %nb_add20 = call i64 @coral_nb_add(i64 %nb_str19, i64 %load_ready)
  %nb_to_ptr21 = call ptr @coral_nb_to_handle(i64 %nb_add20)
  %log_call22 = call ptr @coral_log(ptr %nb_to_ptr21)
  %ptr_to_nb23 = call i64 @coral_nb_from_handle(ptr %log_call22)
  %nb_str24 = call i64 @coral_nb_make_string(ptr @str_7, i64 5)
  %nb_str25 = call i64 @coral_nb_make_string(ptr @str_8, i64 6)
  %nb_str26 = call i64 @coral_nb_make_string(ptr @str_9, i64 6)
  %nb_to_ptr27 = call ptr @coral_nb_to_handle(i64 %nb_str24)
  %nb_to_ptr28 = call ptr @coral_nb_to_handle(i64 %nb_str25)
  %nb_to_ptr29 = call ptr @coral_nb_to_handle(i64 %nb_str26)
  %list_init = insertvalue [3 x ptr] undef, ptr %nb_to_ptr27, 0
  %list_init30 = insertvalue [3 x ptr] %list_init, ptr %nb_to_ptr28, 1
  %list_init31 = insertvalue [3 x ptr] %list_init30, ptr %nb_to_ptr29, 2
  %list_literal = alloca [3 x ptr], align 8
  store [3 x ptr] %list_init31, ptr %list_literal, align 8
  %make_list_hinted = call ptr @coral_make_list_hinted(ptr %list_literal, i64 3, i8 4)
  %ptr_to_nb32 = call i64 @coral_nb_from_handle(ptr %make_list_hinted)
  store i64 %ptr_to_nb32, ptr %fruits_slot, align 4
  %nb_str33 = call i64 @coral_nb_make_string(ptr @str_10, i64 8)
  %load_fruits = load i64, ptr %fruits_slot, align 4
  %nb_add34 = call i64 @coral_nb_add(i64 %nb_str33, i64 %load_fruits)
  %nb_to_ptr35 = call ptr @coral_nb_to_handle(i64 %nb_add34)
  %log_call36 = call ptr @coral_log(ptr %nb_to_ptr35)
  %ptr_to_nb37 = call i64 @coral_nb_from_handle(ptr %log_call36)
  %nb_str38 = call i64 @coral_nb_make_string(ptr @str_11, i64 13)
  %load_fruits39 = load i64, ptr %fruits_slot, align 4
  %nb_num40 = call i64 @coral_nb_make_number(double 0.000000e+00)
  %nb_to_ptr41 = call ptr @coral_nb_to_handle(i64 %load_fruits39)
  %nb_to_ptr42 = call ptr @coral_nb_to_handle(i64 %nb_num40)
  %subscript = call ptr @coral_list_get(ptr %nb_to_ptr41, ptr %nb_to_ptr42)
  %ptr_to_nb43 = call i64 @coral_nb_from_handle(ptr %subscript)
  %nb_add44 = call i64 @coral_nb_add(i64 %nb_str38, i64 %ptr_to_nb43)
  %nb_to_ptr45 = call ptr @coral_nb_to_handle(i64 %nb_add44)
  %log_call46 = call ptr @coral_log(ptr %nb_to_ptr45)
  %ptr_to_nb47 = call i64 @coral_nb_from_handle(ptr %log_call46)
  %nb_str48 = call i64 @coral_nb_make_string(ptr @str_12, i64 4)
  %nb_str49 = call i64 @coral_nb_make_string(ptr @str_13, i64 9)
  %nb_str50 = call i64 @coral_nb_make_string(ptr @str_14, i64 4)
  %nb_num51 = call i64 @coral_nb_make_number(double 8.080000e+03)
  %nb_str52 = call i64 @coral_nb_make_string(ptr @str_15, i64 5)
  %nb_bool53 = call i64 @coral_nb_make_bool(i8 1)
  %nb_to_ptr54 = call ptr @coral_nb_to_handle(i64 %nb_str48)
  %nb_to_ptr55 = call ptr @coral_nb_to_handle(i64 %nb_str49)
  %map_key = insertvalue { ptr, ptr } undef, ptr %nb_to_ptr54, 0
  %map_value = insertvalue { ptr, ptr } %map_key, ptr %nb_to_ptr55, 1
  %map_entry = insertvalue [3 x { ptr, ptr }] undef, { ptr, ptr } %map_value, 0
  %nb_to_ptr56 = call ptr @coral_nb_to_handle(i64 %nb_str50)
  %nb_to_ptr57 = call ptr @coral_nb_to_handle(i64 %nb_num51)
  %map_key58 = insertvalue { ptr, ptr } undef, ptr %nb_to_ptr56, 0
  %map_value59 = insertvalue { ptr, ptr } %map_key58, ptr %nb_to_ptr57, 1
  %map_entry60 = insertvalue [3 x { ptr, ptr }] %map_entry, { ptr, ptr } %map_value59, 1
  %nb_to_ptr61 = call ptr @coral_nb_to_handle(i64 %nb_str52)
  %nb_to_ptr62 = call ptr @coral_nb_to_handle(i64 %nb_bool53)
  %map_key63 = insertvalue { ptr, ptr } undef, ptr %nb_to_ptr61, 0
  %map_value64 = insertvalue { ptr, ptr } %map_key63, ptr %nb_to_ptr62, 1
  %map_entry65 = insertvalue [3 x { ptr, ptr }] %map_entry60, { ptr, ptr } %map_value64, 2
  %map_literal = alloca [3 x { ptr, ptr }], align 8
  store [3 x { ptr, ptr }] %map_entry65, ptr %map_literal, align 8
  %make_map_hinted = call ptr @coral_make_map_hinted(ptr %map_literal, i64 3, i8 4)
  %ptr_to_nb66 = call i64 @coral_nb_from_handle(ptr %make_map_hinted)
  store i64 %ptr_to_nb66, ptr %config_slot, align 4
  %nb_str67 = call i64 @coral_nb_make_string(ptr @str_16, i64 6)
  %load_config = load i64, ptr %config_slot, align 4
  %nb_str68 = call i64 @coral_nb_make_string(ptr @str_12, i64 4)
  %nb_to_ptr69 = call ptr @coral_nb_to_handle(i64 %load_config)
  %nb_to_ptr70 = call ptr @coral_nb_to_handle(i64 %nb_str68)
  %value_get_method = call ptr @coral_value_get(ptr %nb_to_ptr69, ptr %nb_to_ptr70)
  %ptr_to_nb71 = call i64 @coral_nb_from_handle(ptr %value_get_method)
  %nb_add72 = call i64 @coral_nb_add(i64 %nb_str67, i64 %ptr_to_nb71)
  %nb_to_ptr73 = call ptr @coral_nb_to_handle(i64 %nb_add72)
  %log_call74 = call ptr @coral_log(ptr %nb_to_ptr73)
  %ptr_to_nb75 = call i64 @coral_nb_from_handle(ptr %log_call74)
  %load_answer76 = load i64, ptr %answer_slot, align 4
  %nb_num77 = call i64 @coral_nb_make_number(double 4.000000e+01)
  %nb_as_number = call double @coral_nb_as_number(i64 %load_answer76)
  %nb_as_number78 = call double @coral_nb_as_number(i64 %nb_num77)
  %gt = fcmp ogt double %nb_as_number, %nb_as_number78
  %bool_byte = zext i1 %gt to i8
  %nb_bool79 = call i64 @coral_nb_make_bool(i8 %bool_byte)
  %nb_is_truthy = call i8 @coral_nb_is_truthy(i64 %nb_bool79)
  %bool_from_byte = trunc i8 %nb_is_truthy to i1
  br i1 %bool_from_byte, label %then, label %else

then:                                             ; preds = %entry
  %nb_str80 = call i64 @coral_nb_make_string(ptr @str_17, i64 3)
  br label %cont

else:                                             ; preds = %entry
  %nb_str81 = call i64 @coral_nb_make_string(ptr @str_18, i64 5)
  br label %cont

cont:                                             ; preds = %else, %then
  %ternary_phi = phi i64 [ %nb_str80, %then ], [ %nb_str81, %else ]
  store i64 %ternary_phi, ptr %status_slot, align 4
  %nb_str82 = call i64 @coral_nb_make_string(ptr @str_19, i64 10)
  %load_status = load i64, ptr %status_slot, align 4
  %nb_add83 = call i64 @coral_nb_add(i64 %nb_str82, i64 %load_status)
  %nb_to_ptr84 = call ptr @coral_nb_to_handle(i64 %nb_add83)
  %log_call85 = call ptr @coral_log(ptr %nb_to_ptr84)
  %ptr_to_nb86 = call i64 @coral_nb_from_handle(ptr %log_call85)
  %nb_str87 = call i64 @coral_nb_make_string(ptr @str_20, i64 16)
  ret i64 %nb_str87
}

define void @__coral_init_globals() {
entry:
  %globals_flag = load i1, ptr @__coral_globals_initialized, align 1
  %globals_ready = icmp eq i1 %globals_flag, true
  br i1 %globals_ready, label %done, label %body

body:                                             ; preds = %entry
  store i1 true, ptr @__coral_globals_initialized, align 1
  %nb_bool = call i64 @coral_nb_make_bool(i8 1)
  store i64 %nb_bool, ptr @coral_global_true_value, align 4
  %nb_bool1 = call i64 @coral_nb_make_bool(i8 0)
  store i64 %nb_bool1, ptr @coral_global_false_value, align 4
  br label %done

done:                                             ; preds = %body, %entry
  ret void
}

define i32 @main() {
entry:
  call void @__coral_init_globals()
  call void @__coral_init_globals()
  %main_handler_closure = call ptr @coral_make_closure(ptr @__coral_main_handler, ptr null, ptr null)
  %main_actor = call ptr @coral_actor_spawn(ptr %main_handler_closure)
  %nb_unit = call i64 @coral_nb_make_unit()
  %nb_to_ptr = call ptr @coral_nb_to_handle(i64 %nb_unit)
  %send_unit = call ptr @coral_actor_send(ptr %main_actor, ptr %nb_to_ptr)
  %wait_main = call ptr @coral_main_wait()
  ret i32 0
}

define void @__coral_main_handler(ptr %0, ptr %1) {
entry:
  %call_user_main = call i64 @__user_main()
  %main_done = call ptr @coral_main_done_signal()
  ret void
}
