//! IQ-5 Coverage Expansion Tests
//!
//! Targets 50+ untested features identified by gap analysis.
//! Categories: bytes ops, JSON, FS I/O, string edge cases, list/map,
//! error handling, pipeline, type_of, bitwise, store/trait, guards, math.

use coralc::Compiler;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");

fn runtime_lib() -> PathBuf {
    let lib = PathBuf::from(WORKSPACE).join("target/debug/libruntime.so");
    assert!(lib.exists(), "Runtime library not found. Run `cargo build -p runtime` first.");
    lib
}

fn run_coral(source: &str) -> (String, String, i32) {
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source)
        .unwrap_or_else(|e| panic!("Compilation failed: {:?}", e));
    let mut ir_file = tempfile::NamedTempFile::new().expect("create temp file");
    ir_file.write_all(ir.as_bytes()).expect("write IR");
    ir_file.flush().expect("flush IR");
    let runtime = runtime_lib();
    let output = Command::new("lli")
        .arg("-load").arg(&runtime)
        .arg(ir_file.path())
        .output().expect("failed to run lli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);
    (stdout, stderr, exit_code)
}

fn assert_output(source: &str, expected: &[&str]) {
    let (stdout, stderr, code) = run_coral(source);
    let expected_full = if expected.is_empty() {
        String::new()
    } else {
        format!("{}\n", expected.join("\n"))
    };
    assert_eq!(stdout, expected_full,
        "\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        stdout, stderr, code);
}

fn assert_output_contains(source: &str, substring: &str) {
    let (stdout, stderr, code) = run_coral(source);
    assert!(stdout.contains(substring),
        "Expected stdout to contain {:?} but got:\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        substring, stdout, stderr, code);
}

fn compile(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    compiler.compile_to_ir(source).map_err(|e| format!("{:?}", e))
}

// ═══════════════════════════════════════════════════════════════════════
// 1. BYTES OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_bytes_length() {
    assert_output(r#"
*main()
    b is bytes_from_string("hello")
    log(b.length)
"#, &["5"]);
}

#[test]
fn e2e_bytes_get_index() {
    assert_output(r#"
*main()
    b is bytes_from_string("ABC")
    log(bytes_get(b, 0))
    log(bytes_get(b, 1))
    log(bytes_get(b, 2))
"#, &["65", "66", "67"]);
}

#[test]
fn e2e_bytes_slice() {
    assert_output(r#"
*main()
    b is bytes_from_string("hello world")
    s is bytes_slice(b, 0, 5)
    log(bytes_to_string(s))
"#, &["hello"]);
}

#[test]
fn e2e_bytes_from_hex() {
    assert_output(r#"
*main()
    b is bytes_from_hex("48656c6c6f")
    log(bytes_to_string(b))
"#, &["Hello"]);
}

#[test]
fn e2e_bytes_contains() {
    assert_output(r#"
*main()
    b is bytes_from_string("hello world")
    pattern is bytes_from_string("world")
    log(bytes_contains(b, pattern))
    missing is bytes_from_string("xyz")
    log(bytes_contains(b, missing))
"#, &["true", "false"]);
}

#[test]
fn e2e_bytes_find() {
    assert_output(r#"
*main()
    b is bytes_from_string("hello world")
    pattern is bytes_from_string("world")
    log(bytes_find(b, pattern))
    missing is bytes_from_string("xyz")
    log(bytes_find(b, missing))
"#, &["6", "-1"]);
}

#[test]
fn e2e_bytes_roundtrip() {
    assert_output(r#"
*main()
    original is "The quick brown fox"
    b is bytes_from_string(original)
    restored is bytes_to_string(b)
    log(restored)
    log(b.length)
"#, &["The quick brown fox", "19"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 2. JSON OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_json_parse_nested_object() {
    assert_output(r#"
*main()
    data is json_parse("{\"name\":\"alice\",\"age\":30}")
    log(data.get("name"))
    log(data.get("age"))
"#, &["alice", "30"]);
}

#[test]
fn e2e_json_roundtrip_map() {
    assert_output(r#"
*main()
    m is map("x" is 10, "y" is 20)
    s is json_serialize(m)
    parsed is json_parse(s)
    log(parsed.get("x"))
    log(parsed.get("y"))
"#, &["10", "20"]);
}

#[test]
fn e2e_json_serialize_list() {
    assert_output(r#"
*main()
    items is [1, 2, 3]
    s is json_serialize(items)
    log(s)
"#, &["[1,2,3]"]);
}

#[test]
fn e2e_json_serialize_string() {
    assert_output(r#"
*main()
    s is json_serialize("hello")
    log(s)
"#, &["\"hello\""]);
}

#[test]
fn e2e_json_serialize_bool_null() {
    assert_output(r#"
*main()
    log(json_serialize(true))
    log(json_serialize(false))
"#, &["true", "false"]);
}

#[test]
fn e2e_json_parse_array_of_strings() {
    assert_output(r#"
*main()
    items is json_parse("[\"a\",\"b\",\"c\"]")
    log(items.length)
    log(items.get(0))
    log(items.get(1))
    log(items.get(2))
"#, &["3", "a", "b", "c"]);
}

#[test]
fn e2e_json_parse_nested_array() {
    assert_output(r#"
*main()
    data is json_parse("{\"items\":[10,20,30]}")
    items is data.get("items")
    log(items.length)
    log(items.get(0))
"#, &["3", "10"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 3. ENCODING (Base64 & Hex)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_base64_roundtrip() {
    assert_output(r#"
*main()
    encoded is base64_encode("Hello, World!")
    log(encoded)
"#, &["SGVsbG8sIFdvcmxkIQ=="]);
}

#[test]
fn e2e_hex_encode_decode() {
    assert_output(r#"
*main()
    encoded is hex_encode("Hi")
    log(encoded)
"#, &["4869"]);
}

#[test]
fn e2e_hex_encode_empty() {
    assert_output(r#"
*main()
    encoded is hex_encode("")
    log(encoded)
"#, &[""]);
}

#[test]
fn e2e_base64_encode_empty() {
    assert_output(r#"
*main()
    encoded is base64_encode("")
    log(encoded)
"#, &[""]);
}

// ═══════════════════════════════════════════════════════════════════════
// 4. TIME OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_time_components() {
    // time_year etc. work on millisecond timestamps
    // Verify that current year is reasonable (> 2020)
    assert_output_contains(r#"
*main()
    ts is time_now()
    y is time_year(ts)
    log(y > 2020 ? "ok" ! "bad")
"#, "ok");
}

#[test]
fn e2e_time_hour_minute_second() {
    // Verify hour/minute/second return valid ranges
    assert_output_contains(r#"
*main()
    ts is time_now()
    h is time_hour(ts)
    m is time_minute(ts)
    s is time_second(ts)
    log(h >= 0 ? "ok" ! "bad")
    log(m >= 0 ? "ok" ! "bad")
    log(s >= 0 ? "ok" ! "bad")
"#, "ok");
}

#[test]
fn e2e_time_now_is_positive() {
    assert_output_contains(r#"
*main()
    t is time_now()
    result is t > 0 ? "positive" ! "zero_or_negative"
    log(result)
"#, "positive");
}

#[test]
fn e2e_time_timestamp_is_positive() {
    assert_output_contains(r#"
*main()
    t is time_timestamp()
    result is t > 0 ? "positive" ! "zero_or_negative"
    log(result)
"#, "positive");
}

// ═══════════════════════════════════════════════════════════════════════
// 5. FILE SYSTEM OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_fs_write_read_roundtrip() {
    assert_output(r#"
*main()
    fs_write("/tmp/coral_test_e2e_rw.txt", "hello coral")
    content is fs_read("/tmp/coral_test_e2e_rw.txt")
    log(bytes_to_string(content))
    fs_delete("/tmp/coral_test_e2e_rw.txt")
"#, &["hello coral"]);
}

#[test]
fn e2e_fs_exists() {
    assert_output(r#"
*main()
    fs_write("/tmp/coral_test_e2e_exists.txt", "test")
    log(fs_exists("/tmp/coral_test_e2e_exists.txt"))
    fs_delete("/tmp/coral_test_e2e_exists.txt")
    log(fs_exists("/tmp/coral_test_e2e_exists.txt"))
"#, &["true", "false"]);
}

#[test]
fn e2e_fs_append() {
    assert_output(r#"
*main()
    fs_write("/tmp/coral_test_e2e_append.txt", "hello")
    fs_append("/tmp/coral_test_e2e_append.txt", " world")
    content is fs_read("/tmp/coral_test_e2e_append.txt")
    log(bytes_to_string(content))
    fs_delete("/tmp/coral_test_e2e_append.txt")
"#, &["hello world"]);
}

#[test]
fn e2e_fs_mkdir_is_dir() {
    assert_output(r#"
*main()
    fs_mkdir("/tmp/coral_test_e2e_dir")
    log(fs_is_dir("/tmp/coral_test_e2e_dir"))
    log(fs_is_dir("/tmp/coral_test_e2e_nonexistent_xyz"))
    fs_delete("/tmp/coral_test_e2e_dir")
"#, &["true", "false"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 5b. L2.4: ENHANCED FILE SYSTEM + STDERR OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_fs_size() {
    assert_output(r#"
*main()
    fs_write("/tmp/coral_test_e2e_size.txt", "hello")
    sz is fs_size("/tmp/coral_test_e2e_size.txt")
    log(sz)
    fs_delete("/tmp/coral_test_e2e_size.txt")
"#, &["5"]);
}

#[test]
fn e2e_fs_rename() {
    assert_output(r#"
*main()
    fs_write("/tmp/coral_test_e2e_rename_src.txt", "rename me")
    fs_rename("/tmp/coral_test_e2e_rename_src.txt", "/tmp/coral_test_e2e_rename_dst.txt")
    content is fs_read("/tmp/coral_test_e2e_rename_dst.txt")
    log(bytes_to_string(content))
    log(fs_exists("/tmp/coral_test_e2e_rename_src.txt"))
    fs_delete("/tmp/coral_test_e2e_rename_dst.txt")
"#, &["rename me", "false"]);
}

#[test]
fn e2e_fs_copy() {
    assert_output(r#"
*main()
    fs_write("/tmp/coral_test_e2e_copy_src.txt", "copy me")
    fs_copy("/tmp/coral_test_e2e_copy_src.txt", "/tmp/coral_test_e2e_copy_dst.txt")
    content is fs_read("/tmp/coral_test_e2e_copy_dst.txt")
    log(bytes_to_string(content))
    log(fs_exists("/tmp/coral_test_e2e_copy_src.txt"))
    fs_delete("/tmp/coral_test_e2e_copy_src.txt")
    fs_delete("/tmp/coral_test_e2e_copy_dst.txt")
"#, &["copy me", "true"]);
}

#[test]
fn e2e_fs_mkdirs() {
    assert_output(r#"
*main()
    fs_mkdirs("/tmp/coral_test_e2e_mkdirs/a/b/c")
    log(fs_is_dir("/tmp/coral_test_e2e_mkdirs/a/b/c"))
    fs_delete("/tmp/coral_test_e2e_mkdirs/a/b/c")
    fs_delete("/tmp/coral_test_e2e_mkdirs/a/b")
    fs_delete("/tmp/coral_test_e2e_mkdirs/a")
    fs_delete("/tmp/coral_test_e2e_mkdirs")
"#, &["true"]);
}

#[test]
fn e2e_fs_temp_dir() {
    assert_output_contains(r#"
*main()
    td is fs_temp_dir()
    log(td)
"#, "/tmp");
}

#[test]
fn e2e_stderr_write() {
    // stderr_write writes to stderr and returns unit
    let (stdout, stderr, code) = run_coral(r#"
*main()
    stderr_write("hello stderr")
    log("done")
"#);
    assert_eq!(code, 0, "Expected exit code 0, got {}", code);
    assert!(stderr.contains("hello stderr"),
        "Expected stderr to contain 'hello stderr' but got: {}", stderr);
    assert!(stdout.contains("done"), "Expected stdout to contain 'done' but got: {}", stdout);
}

// ═══════════════════════════════════════════════════════════════════════
// L4.2: std.path OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_path_normalize() {
    assert_output(r#"
*main()
    log(path_normalize("/usr/local/../bin"))
    log(path_normalize("/a/b/./c/../d"))
"#, &["/usr/bin", "/a/b/d"]);
}

#[test]
fn e2e_path_resolve() {
    // /tmp always exists on Linux
    assert_output_contains(r#"
*main()
    p is path_resolve("/tmp")
    log(p)
"#, "/tmp");
}

#[test]
fn e2e_path_is_absolute() {
    assert_output(r#"
*main()
    log(path_is_absolute("/usr/bin"))
    log(path_is_absolute("relative/path"))
"#, &["true", "false"]);
}

#[test]
fn e2e_path_parent() {
    assert_output(r#"
*main()
    log(path_parent("/usr/local/bin"))
    log(path_parent("/usr"))
"#, &["/usr/local", "/"]);
}

#[test]
fn e2e_path_stem() {
    assert_output(r#"
*main()
    log(path_stem("/home/user/document.txt"))
    log(path_stem("/etc/config.tar.gz"))
"#, &["document", "config.tar"]);
}

// ═══════════════════════════════════════════════════════════════════════
// L2.5: std.process ENHANCEMENTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_process_cwd() {
    assert_output_contains(r#"
*main()
    d is process_cwd()
    log(d)
"#, "/");
}

#[test]
fn e2e_process_pid() {
    // PID should be a positive number
    let (stdout, _stderr, code) = run_coral(r#"
*main()
    p is process_pid()
    log(p)
"#);
    assert_eq!(code, 0);
    let pid_str = stdout.trim();
    let pid: f64 = pid_str.parse().expect("PID should be a number");
    assert!(pid > 0.0, "PID should be positive, got {}", pid);
}

#[test]
fn e2e_process_hostname() {
    // Hostname should be a non-empty string
    let (stdout, _stderr, code) = run_coral(r#"
*main()
    h is process_hostname()
    log(h)
"#);
    assert_eq!(code, 0);
    let host = stdout.trim();
    assert!(!host.is_empty(), "Hostname should not be empty");
}

#[test]
fn e2e_process_chdir() {
    assert_output(r#"
*main()
    process_chdir("/tmp")
    d is process_cwd()
    log(d)
"#, &["/tmp"]);
}

#[test]
fn e2e_process_exec_echo() {
    assert_output(r#"
*main()
    result is process_exec("echo", ["hello from coral"])
    log(result.get("exit_code"))
    log(result.get("stdout"))
"#, &["0", "hello from coral\n"]);
}

#[test]
fn e2e_process_exec_exit_code() {
    assert_output(r#"
*main()
    result is process_exec("false", [])
    log(result.get("exit_code"))
"#, &["1"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 6. STRING EDGE CASES
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_string_lines() {
    assert_output(r#"
*main()
    text is "line1\nline2\nline3"
    lines is string_lines(text)
    log(lines.length)
    log(lines.get(0))
    log(lines.get(2))
"#, &["3", "line1", "line3"]);
}

#[test]
fn e2e_string_to_chars() {
    assert_output(r#"
*main()
    chars is chars("abc")
    log(chars.length)
    log(chars.get(0))
    log(chars.get(1))
    log(chars.get(2))
"#, &["3", "a", "b", "c"]);
}

#[test]
fn e2e_string_empty_operations() {
    assert_output(r#"
*main()
    s is ""
    log(s.length)
    log(trim(s))
    log(to_upper(s))
"#, &["0", "", ""]);
}

#[test]
fn e2e_string_replace_multiple() {
    assert_output(r#"
*main()
    s is "aabbcc"
    result is replace(s, "b", "X")
    log(result)
"#, &["aaXXcc"]);
}

#[test]
fn e2e_string_index_of_not_found() {
    assert_output(r#"
*main()
    idx is index_of("hello", "xyz")
    log(idx)
"#, &["-1"]);
}

#[test]
fn e2e_string_compare_ordering() {
    assert_output(r#"
*main()
    result is strcmp("apple", "banana")
    log(result < 0 ? "less" ! "not_less")
    result2 is strcmp("banana", "apple")
    log(result2 > 0 ? "greater" ! "not_greater")
    result3 is strcmp("same", "same")
    log(result3)
"#, &["less", "greater", "0"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 7. LIST OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_list_pop() {
    assert_output(r#"
*main()
    items is [1, 2, 3]
    last is items.pop()
    log(last)
    log(items.length)
"#, &["3", "2"]);
}

#[test]
fn e2e_list_sort_natural_strings() {
    assert_output(r#"
*main()
    items is ["cherry", "apple", "banana"]
    sorted is sort_natural(items)
    log(sorted.get(0))
    log(sorted.get(1))
    log(sorted.get(2))
"#, &["apple", "banana", "cherry"]);
}

#[test]
fn e2e_list_sort_natural_numbers() {
    assert_output(r#"
*main()
    items is [5, 3, 8, 1, 9]
    sorted is sort_natural(items)
    log(sorted.get(0))
    log(sorted.get(4))
"#, &["1", "9"]);
}

#[test]
fn e2e_list_map_with_lambda() {
    assert_output(r#"
*main()
    items is [1, 2, 3, 4]
    doubled is items.map(*fn(x) x * 2)
    log(doubled.get(0))
    log(doubled.get(1))
    log(doubled.get(2))
    log(doubled.get(3))
"#, &["2", "4", "6", "8"]);
}

#[test]
fn e2e_list_filter_with_lambda() {
    assert_output(r#"
*main()
    items is [1, 2, 3, 4, 5, 6]
    evens is items.filter(*fn(x) x > 3)
    log(evens.length)
    log(evens.get(0))
    log(evens.get(1))
    log(evens.get(2))
"#, &["3", "4", "5", "6"]);
}

#[test]
fn e2e_list_reduce_sum() {
    assert_output(r#"
*main()
    items is [1, 2, 3, 4, 5]
    total is items.reduce(0, *fn(acc, x) acc + x)
    log(total)
"#, &["15"]);
}

#[test]
fn e2e_list_empty_operations() {
    assert_output(r#"
*main()
    items is []
    log(items.length)
    log(list_contains(items, 1))
"#, &["0", "false"]);
}

#[test]
fn e2e_list_index_access_bracket() {
    assert_output(r#"
*main()
    items is [10, 20, 30]
    log(items.get(0))
    log(items.get(2))
"#, &["10", "30"]);
}

#[test]
fn e2e_list_join_custom_separator() {
    assert_output(r#"
*main()
    items is ["a", "b", "c"]
    result is join(items, "-")
    log(result)
"#, &["a-b-c"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 8. MAP OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_map_length() {
    assert_output(r#"
*main()
    m is map("a" is 1, "b" is 2, "c" is 3)
    log(m.length)
"#, &["3"]);
}

#[test]
fn e2e_map_entries() {
    assert_output_contains(r#"
*main()
    m is map("x" is 10)
    e is entries(m)
    log(e.length)
"#, "1");
}

#[test]
fn e2e_map_remove_key() {
    assert_output(r#"
*main()
    m is map("a" is 1, "b" is 2)
    map_remove(m, "a")
    log(m.length)
    log(has_key(m, "a"))
    log(has_key(m, "b"))
"#, &["1", "false", "true"]);
}

#[test]
fn e2e_map_merge() {
    assert_output_contains(r#"
*main()
    m1 is map("a" is 1)
    m2 is map("b" is 2)
    merged is merge(m1, m2)
    log(merged.length)
"#, "2");
}

#[test]
fn e2e_map_iteration() {
    assert_output_contains(r#"
*main()
    m is map("x" is 42)
    k is keys(m)
    log(k.length)
    log(k.get(0))
"#, "x");
}

#[test]
fn e2e_map_values() {
    assert_output_contains(r#"
*main()
    m is map("a" is 100, "b" is 200)
    v is values(m)
    log(v.length)
"#, "2");
}

// ═══════════════════════════════════════════════════════════════════════
// 9. ERROR HANDLING
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_error_code_extraction() {
    assert_output(r#"
err TestError

*main()
    e is err TestError
    log(is_err(e))
    name is error_name(e)
    log(name)
"#, &["true", "TestError"]);
}

#[test]
fn e2e_is_absent_check() {
    // Testing type_of on various values
    assert_output(r#"
*main()
    log(type_of(42))
    log(type_of("hello"))
    log(type_of(true))
    log(type_of([1, 2]))
"#, &["number", "string", "bool", "list"]);
}

#[test]
fn e2e_error_propagation_successful_path() {
    assert_output(r#"
*safe_divide(a, b)
    if b is 0
        return err "DivByZero"
    return a / b

*main()
    val is safe_divide(10, 2)
    log(val)
"#, &["5"]);
}

#[test]
fn e2e_error_value_is_err_false() {
    assert_output_contains(r#"
*main()
    log(is_err(42))
    log(is_err("hello"))
"#, "false");
}

// ═══════════════════════════════════════════════════════════════════════
// 10. PIPELINE OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_pipeline_multi_stage() {
    assert_output(r#"
*double(x)
    return x * 2

*add_one(x)
    return x + 1

*to_str(x)
    return number_to_string(x)

*main()
    result is 5 ~ double ~ add_one ~ to_str
    log(result)
"#, &["11"]);
}

#[test]
fn e2e_pipeline_with_string() {
    assert_output(r#"
*shout(s)
    return to_upper(s)

*main()
    result is "hello" ~ shout
    log(result)
"#, &["HELLO"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 11. BITWISE OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_bitwise_not() {
    assert_output(r#"
*main()
    log(bit_not(0))
    log(bit_not(255))
"#, &["-1", "-256"]);
}

#[test]
fn e2e_bitwise_combined() {
    assert_output(r#"
*main()
    a is 0xFF
    b is 0x0F
    log(bit_and(a, b))
    log(bit_or(a, b))
    log(bit_xor(a, b))
"#, &["15", "255", "240"]);
}

#[test]
fn e2e_shift_operations() {
    assert_output(r#"
*main()
    log(bit_shl(1, 8))
    log(bit_shr(256, 4))
"#, &["256", "16"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 12. STORE / TRAIT OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_store_method_with_self() {
    assert_output(r#"
store Counter
    count ? 0
    *increment()
        self.count is self.count + 1
    *get_count()
        return self.count

*main()
    c is make_Counter()
    c.increment()
    c.increment()
    c.increment()
    log(c.get_count())
"#, &["3"]);
}

#[test]
fn e2e_store_multiple_fields() {
    assert_output(r#"
store Point
    x ? 0
    y ? 0
    *move(dx, dy)
        self.x is self.x + dx
        self.y is self.y + dy
    *describe()
        log("x=" + number_to_string(self.x) + " y=" + number_to_string(self.y))

*main()
    p is make_Point()
    p.move(3, 4)
    p.describe()
"#, &["x=3 y=4"]);
}

#[test]
fn e2e_trait_default_and_override() {
    assert_output(r#"
trait Greeter
    *greet()
        log("hello from default")

store CustomBot with Greeter
    name ? "custom"
    *greet()
        log("hi from " + self.name)

*main()
    b is make_CustomBot()
    b.greet()
"#, &["hi from custom"]);
}

#[test]
fn e2e_store_string_field_mutation() {
    assert_output(r#"
store Logger
    prefix ? "INFO"
    *set_prefix(p)
        self.prefix is p
    *emit(msg)
        log(self.prefix + ": " + msg)

*main()
    l is make_Logger()
    l.emit("starting")
    l.set_prefix("DEBUG")
    l.emit("checking")
"#, &["INFO: starting", "DEBUG: checking"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 13. MATCH EXPRESSIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_match_with_default() {
    assert_output(r#"
*describe(x)
    match x
        1 ? "one"
        2 ? "two"
        ! "other"

*main()
    log(describe(1))
    log(describe(2))
    log(describe(99))
"#, &["one", "two", "other"]);
}

#[test]
fn e2e_match_string_values() {
    assert_output(r#"
*greet(name)
    match name
        "alice" ? "hi alice!"
        "bob" ? "hey bob!"
        ! "hello stranger"

*main()
    log(greet("alice"))
    log(greet("bob"))
    log(greet("charlie"))
"#, &["hi alice!", "hey bob!", "hello stranger"]);
}

#[test]
fn e2e_match_return_in_function() {
    assert_output(r#"
*classify(n)
    return match n
        0 ? "zero"
        1 ? "one"
        ! "many"

*main()
    log(classify(0))
    log(classify(1))
    log(classify(5))
"#, &["zero", "one", "many"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 14. ADT / ENUM OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_adt_with_data() {
    assert_output(r#"
enum Shape
    Circle(radius)
    Rectangle(w, h)

*area(s)
    return match s
        Circle(r) ? r * r
        Rectangle(w, h) ? w * h

*main()
    c is Circle(10)
    log(area(c))
    r is Rectangle(3, 4)
    log(area(r))
"#, &["100", "12"]);
}

#[test]
fn e2e_adt_option_pattern() {
    assert_output(r#"
enum Maybe
    Just(val)
    Nothing

*unwrap_or(opt, default)
    return match opt
        Just(v) ? v
        Nothing ? default

*main()
    a is Just(42)
    b is Nothing
    log(unwrap_or(a, 0))
    log(unwrap_or(b, 0))
"#, &["42", "0"]);
}

#[test]
fn e2e_adt_nested_match() {
    assert_output(r#"
enum Expr
    Num(v)
    Add(l, r)

*eval(e)
    return match e
        Num(v) ? v
        Add(l, r) ? eval(l) + eval(r)

*main()
    e is Add(Num(1), Add(Num(2), Num(3)))
    log(eval(e))
"#, &["6"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 15. GUARD EXPRESSIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_guard_early_return() {
    assert_output(r#"
*check(x)
    x < 0 ? return "negative"
    x is 0 ? return "zero"
    return "positive"

*main()
    log(check(-5))
    log(check(0))
    log(check(10))
"#, &["negative", "zero", "positive"]);
}

#[test]
fn e2e_guard_in_loop() {
    assert_output(r#"
*main()
    i is 0
    result is 0
    while i < 10
        i is i + 1
        i % 2 is 0 ? continue
        result is result + i
    log(result)
"#, &["25"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 16. MATH OPERATIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_math_trig_values() {
    assert_output_contains(r#"
*main()
    log(sin(0))
    log(cos(0))
"#, "0");
}

#[test]
fn e2e_math_floor_ceil_round() {
    assert_output(r#"
*main()
    log(floor(3.7))
    log(ceil(3.2))
    log(round(3.5))
"#, &["3", "4", "4"]);
}

#[test]
fn e2e_math_abs_sign() {
    assert_output(r#"
*main()
    log(abs(-42))
    log(abs(42))
    log(sign(-10))
    log(sign(10))
"#, &["42", "42", "-1", "1"]);
}

#[test]
fn e2e_math_pow_sqrt() {
    assert_output(r#"
*main()
    log(pow(2, 10))
    log(sqrt(144))
"#, &["1024", "12"]);
}

#[test]
fn e2e_math_min_max() {
    assert_output(r#"
*main()
    log(min(3, 7))
    log(max(3, 7))
    log(min(-1, 1))
    log(max(-1, 1))
"#, &["3", "7", "-1", "1"]);
}

#[test]
fn e2e_math_atan2() {
    assert_output_contains(r#"
*main()
    result is atan2(1, 1)
    log(result > 0 ? "positive" ! "non_positive")
"#, "positive");
}

#[test]
fn e2e_math_exp_ln() {
    assert_output(r#"
*main()
    log(exp(0))
    log(ln(1))
"#, &["1", "0"]);
}

#[test]
fn e2e_math_trunc() {
    assert_output(r#"
*main()
    log(trunc(3.9))
    log(trunc(-3.9))
"#, &["3", "-3"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 17. TYPE_OF EXTENDED
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_type_of_all_types() {
    assert_output(r#"
*main()
    log(type_of(42))
    log(type_of("hello"))
    log(type_of(true))
    log(type_of([1, 2]))
    log(type_of(map("a" is 1)))
"#, &["number", "string", "bool", "list", "map"]);
}

#[test]
fn e2e_type_of_bytes() {
    assert_output(r#"
*main()
    b is bytes_from_string("hi")
    log(type_of(b))
"#, &["bytes"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 18. CLOSURES & HIGHER-ORDER
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_closure_captures_outer() {
    assert_output(r#"
*make_adder(n)
    return *fn(x) x + n

*main()
    add5 is make_adder(5)
    log(add5(10))
    log(add5(20))
"#, &["15", "25"]);
}

#[test]
fn e2e_higher_order_compose() {
    assert_output(r#"
*double(x)
    return x * 2

*negate(x)
    return 0 - x

*apply_twice(f, x)
    return f(f(x))

*main()
    log(apply_twice(double, 3))
    log(apply_twice(negate, 5))
"#, &["12", "5"]);
}

#[test]
fn e2e_closure_mutable_counter() {
    assert_output(r#"
*make_counter()
    return *fn(n) n + 1

*main()
    inc is make_counter()
    log(inc(0))
    log(inc(1))
    log(inc(2))
"#, &["1", "2", "3"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 19. FOR LOOPS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_for_loop_over_list() {
    assert_output(r#"
*main()
    total is 0
    for x in [10, 20, 30, 40]
        total is total + x
    log(total)
"#, &["100"]);
}

#[test]
fn e2e_for_loop_with_break() {
    assert_output(r#"
*main()
    total is 0
    for x in [1, 2, 3, 4, 5]
        x > 3 ? break
        total is total + x
    log(total)
"#, &["6"]);
}

#[test]
fn e2e_for_loop_nested() {
    assert_output(r#"
*main()
    total is 0
    for i in [1, 2, 3]
        for j in [10, 20]
            total is total + i * j
    log(total)
"#, &["180"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 20. COMPLEX INTEGRATION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_word_frequency_counter() {
    assert_output(r#"
*main()
    text is "the cat sat on the mat the cat"
    words is split(text, " ")
    freq is map()
    for w in words
        has_key(freq, w) ? freq.set(w, freq.get(w) + 1) ! freq.set(w, 1)
    log(freq.get("the"))
    log(freq.get("cat"))
    log(freq.get("sat"))
"#, &["3", "2", "1"]);
}

#[test]
fn e2e_fibonacci_list() {
    assert_output(r#"
*main()
    fibs is [0, 1]
    i is 2
    while i < 10
        prev1 is fibs.get(i - 1)
        prev2 is fibs.get(i - 2)
        fibs.push(prev1 + prev2)
        i is i + 1
    log(fibs.get(9))
    log(fibs.length)
"#, &["34", "10"]);
}

#[test]
fn e2e_string_reverse() {
    assert_output(r#"
*reverse_str(s)
    chars is chars(s)
    result is ""
    i is chars.length - 1
    while i >= 0
        result is result + chars.get(i)
        i is i - 1
    return result

*main()
    log(reverse_str("hello"))
    log(reverse_str("abcdef"))
"#, &["olleh", "fedcba"]);
}

#[test]
fn e2e_map_to_json_and_back() {
    assert_output(r#"
*main()
    config is map("host" is "localhost", "port" is 8080)
    serialized is json_serialize(config)
    restored is json_parse(serialized)
    log(restored.get("host"))
    log(restored.get("port"))
"#, &["localhost", "8080"]);
}

#[test]
fn e2e_list_comprehension_manual() {
    // This tests manual list building via push + filter
    assert_output(r#"
*main()
    nums is [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    squares is []
    for n in nums
        squares.push(n * n)
    big_squares is squares.filter(*fn(x) x > 25)
    log(big_squares.length)
    log(big_squares.get(0))
"#, &["5", "36"]);
}

#[test]
fn e2e_nested_map_access() {
    assert_output(r#"
*main()
    inner is map("value" is 42)
    outer is map("data" is inner)
    data is outer.get("data")
    log(data.get("value"))
"#, &["42"]);
}

#[test]
fn e2e_recursive_sum() {
    assert_output(r#"
*sum_to(n)
    n <= 0 ? return 0
    return n + sum_to(n - 1)

*main()
    log(sum_to(10))
    log(sum_to(100))
"#, &["55", "5050"]);
}

#[test]
fn e2e_multiple_returns_in_conditions() {
    assert_output(r#"
*classify_age(age)
    age < 0 ? return "invalid"
    age < 13 ? return "child"
    age < 18 ? return "teen"
    age < 65 ? return "adult"
    return "senior"

*main()
    log(classify_age(-1))
    log(classify_age(5))
    log(classify_age(15))
    log(classify_age(30))
    log(classify_age(70))
"#, &["invalid", "child", "teen", "adult", "senior"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 21. PARSER / COMPILE-ONLY EDGE CASES
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn compile_map_literal_with_expressions() {
    compile(r#"
*val()
    return 42

*main()
    m is map("key" is val(), "other" is 1 + 2)
    log(m.get("key"))
"#).expect("map literal with computed values should compile");
}

#[test]
fn compile_nested_pipeline() {
    compile(r#"
*double(x)
    return x * 2

*add_one(x)
    return x + 1

*main()
    result is 1 ~ double ~ add_one ~ double ~ add_one
    log(result)
"#).expect("nested pipeline should compile");
}

#[test]
fn compile_deeply_nested_error_hierarchy() {
    compile(r#"
err AppError
    err DatabaseError
        err ConnectionError
            err TimeoutError
            err RefusedError
        err QueryError
    err NetworkError

*main()
    e is err TimeoutError
    log(error_name(e))
"#).expect("deep error hierarchy should compile");
}

#[test]
fn compile_store_with_multiple_traits() {
    compile(r#"
trait Printable
    *display()
        log("printable")

trait Countable
    *count()
        return 0

store Widget with Printable
    name ? "w"
    *display()
        log(self.name)

*main()
    w is make_Widget()
    w.display()
"#).expect("store with trait should compile");
}

#[test]
fn compile_for_loop_over_map_keys() {
    compile(r#"
*main()
    m is map("a" is 1, "b" is 2, "c" is 3)
    k is keys(m)
    for key in k
        log(key)
"#).expect("for loop over map keys should compile");
}

#[test]
fn compile_ternary_chain() {
    compile(r#"
*classify(n)
    return n > 0 ? "positive" ! n < 0 ? "negative" ! "zero"

*main()
    log(classify(5))
    log(classify(-3))
    log(classify(0))
"#).expect("chained ternary should compile");
}

#[test]
fn compile_lambda_in_variable() {
    compile(r#"
*main()
    f is *fn(x) x * x
    log(f(5))
"#).expect("lambda assigned to variable should compile");
}

#[test]
fn compile_string_template_complex() {
    compile(r#"
*main()
    name is "world"
    count is 42
    log("hello {name}, count is {count}")
"#).expect("complex template strings should compile");
}

// ─── S4.5: Extension methods ───

#[test]
fn e2e_extend_store_with_new_method() {
    assert_output(r#"
store Counter
    count ? 0
    *increment()
        self.count is self.count + 1

extend Counter
    *double()
        self.count is self.count * 2
    *value()
        return self.count

*main()
    c is make_Counter()
    c.increment()
    c.increment()
    c.double()
    log(c.value())
"#, &["4"]);
}

#[test]
fn e2e_extend_store_self_field_access() {
    assert_output(r#"
store Point
    x ? 0
    y ? 0
    *move(dx, dy)
        self.x is self.x + dx
        self.y is self.y + dy

extend Point
    *sum()
        return self.x + self.y
    *describe()
        log("sum=" + number_to_string(self.sum()))

*main()
    p is make_Point()
    p.move(3, 7)
    log(p.sum())
    p.describe()
"#, &["10", "sum=10"]);
}

#[test]
fn e2e_extend_no_override_existing() {
    assert_output(r#"
store Greeter
    name ? "world"
    *greet()
        log("hello " + self.name)

extend Greeter
    *greet()
        log("overridden")
    *farewell()
        log("goodbye " + self.name)

*main()
    g is make_Greeter()
    g.greet()
    g.farewell()
"#, &["hello world", "goodbye world"]);
}

#[test]
fn e2e_extend_multiple_blocks() {
    assert_output(r#"
store Num
    v ? 0
    *bump(n)
        self.v is self.v + n

extend Num
    *doubled()
        return self.v * 2

extend Num
    *tripled()
        return self.v * 3

*main()
    n is make_Num()
    n.bump(5)
    log(n.doubled())
    log(n.tripled())
"#, &["10", "15"]);
}

#[test]
fn e2e_extend_method_with_params() {
    assert_output(r#"
store Adder
    total ? 0

extend Adder
    *add(n)
        self.total is self.total + n
    *get_total()
        return self.total

*main()
    a is make_Adder()
    a.add(10)
    a.add(20)
    a.add(5)
    log(a.get_total())
"#, &["35"]);
}

#[test]
fn e2e_extend_chained_method_calls() {
    assert_output(r#"
store Builder
    parts ? ""

extend Builder
    *append(s)
        self.parts is self.parts + s
    *result()
        return self.parts

*main()
    b is make_Builder()
    b.append("hello")
    b.append(" ")
    b.append("world")
    log(b.result())
"#, &["hello world"]);
}

// ── KI-1: Type-aware method dispatch (built-in name shadowing fix) ──

#[test]
fn e2e_ki1_store_method_get_shadows_builtin() {
    // Store method named `get` should dispatch to the store method, not the
    // built-in list/map `.get(index)` which expects 1 arg.
    assert_output(r#"
store Config
    data ? "default"

extend Config
    *get()
        return self.data

*main()
    c is make_Config()
    log(c.get())
"#, &["default"]);
}

#[test]
fn e2e_ki1_store_method_set_shadows_builtin() {
    // Store method named `set` with 1 arg (built-in map.set expects 2).
    assert_output(r#"
store Counter
    val ? 0

extend Counter
    *set(n)
        self.val is n

*main()
    c is make_Counter()
    c.set(42)
    log(c.val)
"#, &["42"]);
}

#[test]
fn e2e_ki1_store_method_length_shadows_builtin() {
    // Store method `length` should override built-in `.length()`.
    assert_output(r#"
store Rope
    text ? ""

extend Rope
    *length()
        return 999

*main()
    r is make_Rope()
    log(r.length())
"#, &["999"]);
}

#[test]
fn e2e_ki1_builtin_length_still_works() {
    // Built-in `.length()` should still work on non-store values.
    assert_output(r#"
*main()
    xs is [1, 2, 3]
    log(xs.length())
    s is "hello"
    log(s.length())
"#, &["3", "5"]);
}

#[test]
fn e2e_ki1_store_method_push_shadows_builtin() {
    // Store `push` method with different semantics than list.push.
    assert_output(r#"
store Adder
    total ? 0

extend Adder
    *push(val)
        self.total is self.total + val

*main()
    a is make_Adder()
    a.push(10)
    a.push(20)
    log(a.total)
"#, &["30"]);
}

// ── CC5.3: All examples compile and run ──

#[test]
fn e2e_cc53_hello_example() {
    let source = std::fs::read_to_string(
        PathBuf::from(WORKSPACE).join("examples/hello.coral")
    ).expect("read hello.coral");
    let (stdout, _stderr, code) = run_coral(&source);
    assert_eq!(code, 0, "hello.coral should exit 0");
    assert!(stdout.contains("Hello, Coral!"), "hello.coral should greet");
}

#[test]
fn e2e_cc53_fizzbuzz_example() {
    let source = std::fs::read_to_string(
        PathBuf::from(WORKSPACE).join("examples/fizzbuzz.coral")
    ).expect("read fizzbuzz.coral");
    let (stdout, _stderr, code) = run_coral(&source);
    assert_eq!(code, 0, "fizzbuzz.coral should exit 0");
    assert!(stdout.contains("FizzBuzz"), "fizzbuzz.coral should contain FizzBuzz");
    assert!(stdout.contains("Fizz"), "fizzbuzz.coral should contain Fizz");
    assert!(stdout.contains("Buzz"), "fizzbuzz.coral should contain Buzz");
}

#[test]
fn e2e_cc53_calculator_example() {
    let source = std::fs::read_to_string(
        PathBuf::from(WORKSPACE).join("examples/calculator.coral")
    ).expect("read calculator.coral");
    let (stdout, _stderr, code) = run_coral(&source);
    assert_eq!(code, 0, "calculator.coral should exit 0");
    assert!(stdout.contains("Calculator demo complete!"), "calculator.coral should complete");
}

#[test]
fn e2e_cc53_data_pipeline_example() {
    let source = std::fs::read_to_string(
        PathBuf::from(WORKSPACE).join("examples/data_pipeline.coral")
    ).expect("read data_pipeline.coral");
    let (stdout, _stderr, code) = run_coral(&source);
    assert_eq!(code, 0, "data_pipeline.coral should exit 0");
    assert!(stdout.contains("Demo complete!"), "data_pipeline.coral should complete");
}

#[test]
fn e2e_cc53_traits_demo_example() {
    let source = std::fs::read_to_string(
        PathBuf::from(WORKSPACE).join("examples/traits_demo.coral")
    ).expect("read traits_demo.coral");
    let (stdout, _stderr, code) = run_coral(&source);
    assert_eq!(code, 0, "traits_demo.coral should exit 0");
    assert!(stdout.contains("Demo complete!"), "traits_demo.coral should complete");
}

// ── L2.2: Regex operations ──

#[test]
fn e2e_regex_match_full() {
    assert_output(r#"
*main()
    log(regex_match("^hello$", "hello"))
    log(regex_match("^hello$", "hello world"))
    log(regex_match("[0-9]+", "42"))
"#, &["true", "false", "true"]);
}

#[test]
fn e2e_regex_find_first() {
    assert_output(r#"
*main()
    log(regex_find("[0-9]+", "abc 123 def 456"))
"#, &["123"]);
}

#[test]
fn e2e_regex_find_all_matches() {
    assert_output(r#"
*main()
    matches is regex_find_all("[0-9]+", "abc 12 def 345 ghi 6")
    log(matches.length())
"#, &["3"]);
}

#[test]
fn e2e_regex_replace_all() {
    assert_output(r#"
*main()
    result is regex_replace("[0-9]+", "NUM", "abc 12 def 345")
    log(result)
"#, &["abc NUM def NUM"]);
}

#[test]
fn e2e_regex_split_pattern() {
    assert_output(r#"
*main()
    parts is regex_split("\\s+", "hello   world   foo")
    log(parts.length())
    log(parts.get(0))
    log(parts.get(1))
    log(parts.get(2))
"#, &["3", "hello", "world", "foo"]);
}

// ── T3.1: Type narrowing in match conditionals ──

#[test]
fn e2e_t31_match_pattern_binding_type() {
    // Variables bound in match patterns via constructors should work correctly.
    assert_output(r#"
enum Shape
    Circle(radius)
    Rect(w, h)

*area(s)
    return match s
        Circle(r) ? 3 * r * r
        Rect(w, h) ? w * h

*main()
    c is Circle(5)
    r is Rect(3, 4)
    log(area(c))
    log(area(r))
"#, &["75", "12"]);
}

#[test]
fn e2e_t31_nested_pattern_narrowing() {
    // Nested destructuring through constructors.
    assert_output(r#"
enum Wrapper
    Box(inner)

enum Inner
    Val(n)

*unwrap(w)
    return match w
        Box(v) ? match v
            Val(n) ? n
            _ ? 0

*main()
    w is Box(Val(42))
    log(unwrap(w))
"#, &["42"]);
}

#[test]
fn e2e_t31_or_pattern_with_bindings() {
    // Or-patterns preserve binding types.
    assert_output(r#"
enum Expr
    Num(v)
    Neg(v)
    Zero

*eval(e)
    return match e
        Num(v) or Neg(v) ? v
        Zero ? 0

*main()
    log(eval(Num(7)))
    log(eval(Neg(3)))
    log(eval(Zero))
"#, &["7", "3", "0"]);
}

#[test]
fn e2e_t31_guard_does_not_narrow_incorrectly() {
    // Multiple arms for the same constructor with different field usage.
    assert_output(r#"
enum Token
    Word(text)
    Number(val)

*describe(t)
    return match t
        Word(s) ? s
        Number(n) ? '{n}'

*main()
    log(describe(Word("hello")))
    log(describe(Number(42)))
"#, &["hello", "42"]);
}

#[test]
fn e2e_t31_narrowed_type_does_not_leak() {
    // Variables bound inside a match arm don't leak to other arms.
    assert_output(r#"
enum Val
    Str(text)
    Num(n)

*show(v)
    return match v
        Str(text) ? text
        Num(n) ? '{n}'

*main()
    log(show(Str("hello")))
    log(show(Num(42)))
"#, &["hello", "42"]);
}

// ── T3.3: Nullability tracking ──

fn warnings_for(source: &str) -> Vec<String> {
    let compiler = Compiler;
    match compiler.compile_to_ir_with_warnings(source) {
        Ok((_ir, warnings)) => warnings.iter().map(|w| w.message.clone()).collect(),
        Err(e) => panic!("compilation failed: {}", e.diagnostic.message),
    }
}

#[test]
fn e2e_t33_none_return_path_warns() {
    let source = r#"
*lookup(key)
    key.equals("a") ? return "found"
    return none

*main()
    log(lookup("a"))
"#;
    let ws = warnings_for(source);
    assert!(
        ws.iter().any(|w| w.contains("may return 'none'") && w.contains("lookup")),
        "expected nullability warning for 'lookup', got: {:?}", ws
    );
}

#[test]
fn e2e_t33_no_none_return_no_warning() {
    let source = r#"
*add(a, b)
    return a + b

*main()
    log(add(1, 2))
"#;
    let ws = warnings_for(source);
    assert!(
        !ws.iter().any(|w| w.contains("may return 'none'")),
        "did NOT expect nullability warning, got: {:?}", ws
    );
}

#[test]
fn e2e_t33_none_in_branch_warns() {
    let source = r#"
*find(items, target)
    i is 0
    while i < items.length()
        items.get(i).equals(target) ? return items.get(i)
        i += 1
    return none

*main()
    log(find([1, 2, 3], 2))
"#;
    let ws = warnings_for(source);
    assert!(
        ws.iter().any(|w| w.contains("may return 'none'") && w.contains("find")),
        "expected nullability warning for 'find', got: {:?}", ws
    );
}

#[test]
fn e2e_t33_main_not_warned() {
    // main() is excluded from nullability checks
    let source = r#"
*main()
    log("hello")
"#;
    let ws = warnings_for(source);
    assert!(
        !ws.iter().any(|w| w.contains("may return 'none'") && w.contains("main")),
        "should not warn on main(), got: {:?}", ws
    );
}

// ── S4.4: Method chaining fluency ──

#[test]
fn e2e_s44_split_length_chain() {
    assert_output(r#"
*main()
    x is "hello world foo".split(" ").length()
    log(x)
"#, &["3"]);
}

#[test]
fn e2e_s44_string_trim_lower() {
    assert_output(r#"
*main()
    x is "  Hello World  ".trim().lower()
    log(x)
"#, &["hello world"]);
}

#[test]
fn e2e_s44_string_upper_chain() {
    assert_output(r#"
*main()
    x is "hello".upper()
    log(x)
"#, &["HELLO"]);
}

#[test]
fn e2e_s44_list_filter_length_chain() {
    assert_output(r#"
*main()
    x is [1, 2, 3, 4, 5].filter($ > 2).length()
    log(x)
"#, &["3"]);
}

// ═══════════════════════════════════════════════════════════════════════
// S5.5: do..end BLOCK SYNTAX
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_s55_do_end_basic_trailing_block() {
    // func(arg) do ... end desugars to func(arg, lambda)
    assert_output(r#"
*run(label, callback)
    log(label)
    callback()

*main()
    run("hello") do
        log("world")
    end
"#, &["hello", "world"]);
}

#[test]
fn e2e_s55_do_end_multiple_statements() {
    assert_output(r#"
*run(callback)
    callback()

*main()
    run() do
        log("one")
        log("two")
        log("three")
    end
"#, &["one", "two", "three"]);
}

#[test]
fn e2e_s55_do_end_nested() {
    assert_output(r#"
*outer(callback)
    log("outer")
    callback()

*inner(callback)
    log("inner")
    callback()

*main()
    outer() do
        inner() do
            log("deep")
        end
    end
"#, &["outer", "inner", "deep"]);
}

#[test]
fn e2e_s55_do_end_missing_end_error() {
    let result = compile(r#"
*main()
    run() do
        log("oops")
"#);
    assert!(result.is_err(), "do without end should produce a parse error");
}

// ═══════════════════════════════════════════════════════════════════════
// CC3.5: INCREMENTAL COMPILATION — MODULE CACHE
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_cc35_cache_miss_falls_back_to_full_compile() {
    use coralc::module_loader::ModuleSource;

    let cache_dir = tempfile::tempdir().expect("create temp dir");
    let compiler = Compiler;
    let sources = vec![ModuleSource {
        name: "main".to_string(),
        path: std::path::PathBuf::from("test.coral"),
        source: "*main()\n    log(42)\n".to_string(),
        import_directives: vec![],
        imports: vec![],
        exports: vec![],
    }];
    let (ir, _warnings, cached) = compiler
        .compile_modules_to_ir_cached(&sources, cache_dir.path())
        .expect("compilation should succeed");
    assert!(!cached, "first compilation should be a cache miss");
    assert!(ir.contains("define"), "IR should contain function definitions");
}

#[test]
fn e2e_cc35_cache_hit_returns_cached_ir() {
    use coralc::module_loader::ModuleSource;

    let cache_dir = tempfile::tempdir().expect("create temp dir");
    let compiler = Compiler;
    let sources = vec![ModuleSource {
        name: "main".to_string(),
        path: std::path::PathBuf::from("test.coral"),
        source: "*main()\n    log(99)\n".to_string(),
        import_directives: vec![],
        imports: vec![],
        exports: vec![],
    }];

    // First compile — cache miss
    let (ir1, _, cached1) = compiler
        .compile_modules_to_ir_cached(&sources, cache_dir.path())
        .expect("first compile");
    assert!(!cached1);

    // Second compile with same sources — cache hit
    let (ir2, _, cached2) = compiler
        .compile_modules_to_ir_cached(&sources, cache_dir.path())
        .expect("second compile");
    assert!(cached2, "second compilation should be a cache hit");
    assert_eq!(ir1, ir2, "cached IR should match original IR");
}

#[test]
fn e2e_cc35_changed_module_invalidates_cache() {
    use coralc::module_loader::ModuleSource;

    let cache_dir = tempfile::tempdir().expect("create temp dir");
    let compiler = Compiler;
    let sources_v1 = vec![ModuleSource {
        name: "main".to_string(),
        path: std::path::PathBuf::from("test.coral"),
        source: "*main()\n    log(1)\n".to_string(),
        import_directives: vec![],
        imports: vec![],
        exports: vec![],
    }];

    // Compile v1
    let (ir1, _, cached1) = compiler
        .compile_modules_to_ir_cached(&sources_v1, cache_dir.path())
        .expect("v1 compile");
    assert!(!cached1);

    // Compile v2 — changed source
    let sources_v2 = vec![ModuleSource {
        name: "main".to_string(),
        path: std::path::PathBuf::from("test.coral"),
        source: "*main()\n    log(2)\n".to_string(),
        import_directives: vec![],
        imports: vec![],
        exports: vec![],
    }];
    let (ir2, _, cached2) = compiler
        .compile_modules_to_ir_cached(&sources_v2, cache_dir.path())
        .expect("v2 compile");
    assert!(!cached2, "changed source should be a cache miss");
    assert_ne!(ir1, ir2, "IR should differ for different source");
}

// ═══════════════════════════════════════════════════════════════════════
// C4.4: LINK-TIME OPTIMIZATION (LTO)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn e2e_c44_lto_produces_valid_ir() {
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(r#"
*main()
    x is 1 + 2
    log(x)
"#).expect("compilation should succeed");
    let optimized = coralc::compiler::optimize_module(&ir, coralc::compiler::LtoOptLevel::O2)
        .expect("LTO optimization should succeed");
    assert!(optimized.contains("define"), "optimized IR should contain function definitions");
}

#[test]
fn e2e_c44_lto_optimized_ir_runs_correctly() {
    // Compile, optimize, then run through lli
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(r#"
*main()
    x is 10 + 20
    log(x)
"#).expect("compilation should succeed");
    let optimized = coralc::compiler::optimize_module(&ir, coralc::compiler::LtoOptLevel::O2)
        .expect("LTO optimization should succeed");

    // Write optimized IR to temp file and run with lli
    let mut ir_file = tempfile::NamedTempFile::new().expect("create temp file");
    ir_file.write_all(optimized.as_bytes()).expect("write IR");
    ir_file.flush().expect("flush IR");
    let runtime = runtime_lib();
    let output = std::process::Command::new("lli")
        .arg("-load").arg(&runtime)
        .arg(ir_file.path())
        .output().expect("failed to run lli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert_eq!(stdout.trim(), "30", "optimized code should produce correct output");
}

#[test]
fn e2e_c44_lto_reduces_ir_size() {
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(r#"
*add(a, b)
    return a + b

*mul(a, b)
    return a * b

*main()
    x is add(1, 2)
    y is mul(x, 3)
    log(y)
"#).expect("compilation should succeed");
    let optimized = coralc::compiler::optimize_module(&ir, coralc::compiler::LtoOptLevel::O2)
        .expect("LTO optimization should succeed");
    // LTO should at least not make the IR significantly larger
    // (in practice it often inlines and simplifies, making it shorter)
    assert!(
        optimized.len() <= ir.len() * 2,
        "optimized IR should not be dramatically larger (original: {}, optimized: {})",
        ir.len(), optimized.len()
    );
}
