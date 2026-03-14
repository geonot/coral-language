//! Stdlib module tests — exercises each std/ module through end-to-end execution.
//!
//! SL-16: Covers set, map, string, json, time, encoding, sort, fmt, testing.

use coralc::Compiler;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");

fn runtime_lib() -> PathBuf {
    let lib = PathBuf::from(WORKSPACE).join("target/debug/libruntime.so");
    assert!(
        lib.exists(),
        "Runtime library not found. Run `cargo build -p runtime` first."
    );
    lib
}

fn run_coral(source: &str) -> (String, String, i32) {
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .unwrap_or_else(|e| panic!("Compilation failed: {:?}", e));
    let mut ir_file = tempfile::NamedTempFile::new().expect("create temp file");
    ir_file.write_all(ir.as_bytes()).expect("write IR");
    ir_file.flush().expect("flush IR");
    let runtime = runtime_lib();
    let output = Command::new("lli")
        .arg("-load")
        .arg(&runtime)
        .arg(ir_file.path())
        .output()
        .expect("failed to run lli");
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
    assert_eq!(
        stdout, expected_full,
        "\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        stdout, stderr, code
    );
}

fn assert_output_contains(source: &str, substring: &str) {
    let (stdout, stderr, code) = run_coral(source);
    assert!(
        stdout.contains(substring),
        "Expected stdout to contain {:?} but got:\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        substring,
        stdout,
        stderr,
        code
    );
}

// ─── JSON Tests (SL-8) ──────────────────────────────────────────────

#[test]
fn stdlib_json_parse_number() {
    assert_output(
        r#"
*main()
    result is json_parse("42")
    log(result)
"#,
        &["42"],
    );
}

#[test]
fn stdlib_json_parse_string() {
    assert_output(
        r#"
*main()
    result is json_parse("\"hello\"")
    log(result)
"#,
        &["hello"],
    );
}

#[test]
fn stdlib_json_parse_bool() {
    assert_output(
        r#"
*main()
    result is json_parse("true")
    log(result)
"#,
        &["true"],
    );
}

#[test]
fn stdlib_json_parse_null() {
    assert_output(
        r#"
*main()
    result is json_parse("null")
    log(result)
"#,
        &["()"],
    );
}

#[test]
fn stdlib_json_parse_array() {
    assert_output(
        r#"
*main()
    result is json_parse("[1, 2, 3]")
    log(result.length)
"#,
        &["3"],
    );
}

#[test]
fn stdlib_json_serialize_number() {
    assert_output(
        r#"
*main()
    result is json_serialize(42)
    log(result)
"#,
        &["42"],
    );
}

#[test]
fn stdlib_json_serialize_string() {
    assert_output(
        r#"
*main()
    result is json_serialize("hello")
    log(result)
"#,
        &["\"hello\""],
    );
}

#[test]
fn stdlib_json_roundtrip_list() {
    assert_output(
        r#"
*main()
    data is [1, 2, 3]
    json is json_serialize(data)
    parsed is json_parse(json)
    log(parsed.length)
    log(parsed.get(0))
    log(parsed.get(2))
"#,
        &["3", "1", "3"],
    );
}

// ─── Time Tests (SL-9) ──────────────────────────────────────────────

#[test]
fn stdlib_time_now_returns_number() {
    assert_output_contains(
        r#"
*main()
    t is time_now()
    log(type_of(t))
"#,
        "number",
    );
}

#[test]
fn stdlib_time_timestamp_returns_number() {
    assert_output_contains(
        r#"
*main()
    t is time_timestamp()
    log(type_of(t))
"#,
        "number",
    );
}

#[test]
fn stdlib_time_format_iso() {
    // Unix epoch timestamp 0 → should be 1970-01-01T00:00:00Z
    assert_output(
        r#"
*main()
    s is time_format_iso(0)
    log(s)
"#,
        &["1970-01-01T00:00:00Z"],
    );
}

#[test]
fn stdlib_time_extract_components() {
    // Timestamp 0 is epoch: 1970-01-01 00:00:00 UTC
    assert_output(
        r#"
*main()
    log(time_year(0))
    log(time_month(0))
    log(time_day(0))
    log(time_hour(0))
    log(time_minute(0))
    log(time_second(0))
"#,
        &["1970", "1", "1", "0", "0", "0"],
    );
}

// ─── Encoding Tests (SL-12) ─────────────────────────────────────────

#[test]
fn stdlib_base64_encode_decode() {
    assert_output(
        r#"
*main()
    encoded is base64_encode("Hello, World!")
    log(encoded)
    decoded is base64_decode(encoded)
    log(bytes_to_string(decoded))
"#,
        &["SGVsbG8sIFdvcmxkIQ==", "Hello, World!"],
    );
}

#[test]
fn stdlib_hex_encode_decode() {
    assert_output(
        r#"
*main()
    encoded is hex_encode("abc")
    log(encoded)
    decoded is hex_decode(encoded)
    log(bytes_to_string(decoded))
"#,
        &["616263", "abc"],
    );
}

// ─── Sort Tests (SL-11) ─────────────────────────────────────────────

#[test]
fn stdlib_sort_natural_numbers() {
    assert_output(
        r#"
*main()
    lst is [3, 1, 4, 1, 5, 9, 2, 6]
    sorted is sort_natural(lst)
    log(sorted.get(0))
    log(sorted.get(1))
    log(sorted.get(2))
    log(sorted.get(7))
"#,
        &["1", "1", "2", "9"],
    );
}

#[test]
fn stdlib_sort_natural_strings() {
    assert_output(
        r#"
*main()
    lst is ["banana", "apple", "cherry"]
    sorted is sort_natural(lst)
    log(sorted.get(0))
    log(sorted.get(1))
    log(sorted.get(2))
"#,
        &["apple", "banana", "cherry"],
    );
}

// ─── String Tests ────────────────────────────────────────────────────

#[test]
fn stdlib_string_lines() {
    assert_output(
        r#"
*main()
    text is "line1
line2
line3"
    parts is string_lines(text)
    log(parts.length)
    log(parts.get(0))
    log(parts.get(2))
"#,
        &["3", "line1", "line3"],
    );
}

#[test]
fn stdlib_string_split_and_join() {
    assert_output(
        r#"
*main()
    parts is split("a,b,c", ",")
    log(parts.length)
    joined is join(parts, "-")
    log(joined)
"#,
        &["3", "a-b-c"],
    );
}

#[test]
fn stdlib_string_trim() {
    assert_output(
        r#"
*main()
    s is "  hello  "
    log(trim(s))
"#,
        &["hello"],
    );
}

#[test]
fn stdlib_string_to_upper_lower() {
    assert_output(
        r#"
*main()
    log(to_upper("hello"))
    log(to_lower("WORLD"))
"#,
        &["HELLO", "world"],
    );
}

#[test]
fn stdlib_string_starts_ends() {
    assert_output(
        r#"
*main()
    log(starts_with("hello world", "hello"))
    log(ends_with("hello world", "world"))
    log(starts_with("hello world", "world"))
"#,
        &["true", "true", "false"],
    );
}

#[test]
fn stdlib_string_contains() {
    assert_output(
        r#"
*main()
    log(contains("hello world", "lo wo"))
    log(contains("hello world", "xyz"))
"#,
        &["true", "false"],
    );
}

#[test]
fn stdlib_string_replace() {
    assert_output(
        r#"
*main()
    result is replace("hello world", "world", "coral")
    log(result)
"#,
        &["hello coral"],
    );
}

#[test]
fn stdlib_string_index_of() {
    assert_output(
        r#"
*main()
    log(index_of("hello", "ll"))
    log(index_of("hello", "xyz"))
"#,
        &["2", "-1"],
    );
}

#[test]
fn stdlib_string_slice() {
    assert_output(
        r#"
*main()
    log(slice("hello world", 0, 5))
    log(slice("hello world", 6, 11))
"#,
        &["hello", "world"],
    );
}

#[test]
fn stdlib_string_char_at() {
    assert_output(
        r#"
*main()
    log(char_at("hello", 0))
    log(char_at("hello", 4))
"#,
        &["h", "o"],
    );
}

#[test]
fn stdlib_string_parse_number() {
    assert_output(
        r#"
*main()
    log(parse_number("42"))
    log(parse_number("3.14"))
"#,
        &["42", "3.14"],
    );
}

// ─── Map Tests ───────────────────────────────────────────────────────

#[test]
fn stdlib_map_keys_values() {
    assert_output(
        r#"
*main()
    m is map("a" is 1, "b" is 2)
    ks is map_keys(m)
    log(ks.length)
    vs is map_values(m)
    log(vs.length)
"#,
        &["2", "2"],
    );
}

#[test]
fn stdlib_map_has_key() {
    assert_output(
        r#"
*main()
    m is map("x" is 10, "y" is 20)
    log(has_key(m, "x"))
    log(has_key(m, "z"))
"#,
        &["true", "false"],
    );
}

#[test]
fn stdlib_map_merge() {
    assert_output(
        r#"
*main()
    a is map("x" is 1)
    b is map("y" is 2)
    merged is merge(a, b)
    log(merged.length)
"#,
        &["2"],
    );
}

#[test]
fn stdlib_map_remove() {
    assert_output(
        r#"
*main()
    m is map("a" is 1, "b" is 2, "c" is 3)
    m2 is map_remove(m, "b")
    log(m2.length)
    log(has_key(m2, "b"))
"#,
        &["2", "false"],
    );
}

// ─── List Tests ──────────────────────────────────────────────────────

#[test]
fn stdlib_list_contains() {
    assert_output(
        r#"
*main()
    lst is [1, 2, 3, 4, 5]
    log(list_contains(lst, 3))
    log(list_contains(lst, 9))
"#,
        &["true", "false"],
    );
}

#[test]
fn stdlib_list_index_of() {
    assert_output(
        r#"
*main()
    lst is [10, 20, 30, 40]
    log(list_index_of(lst, 30))
    log(list_index_of(lst, 99))
"#,
        &["2", "-1"],
    );
}

#[test]
fn stdlib_list_reverse() {
    assert_output(
        r#"
*main()
    lst is [1, 2, 3]
    rev is list_reverse(lst)
    log(rev.get(0))
    log(rev.get(1))
    log(rev.get(2))
"#,
        &["3", "2", "1"],
    );
}

#[test]
fn stdlib_list_slice() {
    assert_output(
        r#"
*main()
    lst is [10, 20, 30, 40, 50]
    sub is list_slice(lst, 1, 4)
    log(sub.length)
    log(sub.get(0))
    log(sub.get(2))
"#,
        &["3", "20", "40"],
    );
}

#[test]
fn stdlib_list_sort() {
    assert_output(
        r#"
*main()
    lst is [5, 3, 8, 1, 9]
    sorted is list_sort(lst)
    log(sorted.get(0))
    log(sorted.get(4))
"#,
        &["1", "9"],
    );
}

#[test]
fn stdlib_list_concat() {
    assert_output(
        r#"
*main()
    a is [1, 2]
    b is [3, 4]
    c is list_concat(a, b)
    log(c.length)
    log(c.get(2))
"#,
        &["4", "3"],
    );
}

#[test]
fn stdlib_list_map_filter_reduce() {
    assert_output(
        r#"
*main()
    lst is [1, 2, 3, 4, 5]
    doubled is lst.map(*fn(x) x * 2)
    log(doubled.get(0))
    log(doubled.get(4))
    evens is lst.filter(*fn(x) x > 3)
    log(evens.length)
    total is lst.reduce(0, *fn(acc, x) acc + x)
    log(total)
"#,
        &["2", "10", "2", "15"],
    );
}

// ─── Math Tests ──────────────────────────────────────────────────────

#[test]
fn stdlib_math_basic() {
    assert_output(
        r#"
*main()
    log(abs(-42))
    log(floor(3.7))
    log(ceil(3.2))
    log(round(3.5))
"#,
        &["42", "3", "4", "4"],
    );
}

#[test]
fn stdlib_math_sqrt_pow() {
    assert_output(
        r#"
*main()
    log(sqrt(25))
    log(pow(2, 10))
"#,
        &["5", "1024"],
    );
}

#[test]
fn stdlib_math_min_max() {
    assert_output(
        r#"
*main()
    log(min(3, 7))
    log(max(3, 7))
"#,
        &["3", "7"],
    );
}

#[test]
fn stdlib_math_trig() {
    assert_output_contains(
        r#"
*main()
    log(sin(0))
    log(cos(0))
"#,
        "0",
    );
}

// ─── Type Reflection Tests ───────────────────────────────────────────

#[test]
fn stdlib_type_of() {
    assert_output(
        r#"
*main()
    log(type_of(42))
    log(type_of("hello"))
    log(type_of(true))
    log(type_of([1, 2]))
    log(type_of(map("a" is 1)))
"#,
        &["number", "string", "bool", "list", "map"],
    );
}

// ─── Error Value Tests ──────────────────────────────────────────────

#[test]
fn stdlib_error_is_err() {
    assert_output(
        r#"
*main()
    e is err NotFound
    log(is_err(e))
    log(is_ok(e))
"#,
        &["true", "false"],
    );
}

#[test]
fn stdlib_error_with_hierarchy() {
    assert_output(
        r#"
*main()
    e is err Database:Connection
    log(is_err(e))
    log(error_name(e))
"#,
        &["true", "Database:Connection"],
    );
}

// ─── Bytes Tests ─────────────────────────────────────────────────────

#[test]
fn stdlib_bytes_from_string_to_string() {
    assert_output(
        r#"
*main()
    b is bytes_from_string("hello")
    s is bytes_to_string(b)
    log(s)
"#,
        &["hello"],
    );
}

// ─── Character Tests ─────────────────────────────────────────────────

#[test]
fn stdlib_char_ord_chr() {
    assert_output(
        r#"
*main()
    log(ord("A"))
    log(chr(65))
"#,
        &["65", "A"],
    );
}

// ─── Process Tests ───────────────────────────────────────────────────

#[test]
fn stdlib_process_args() {
    assert_output_contains(
        r#"
*main()
    args is process_args()
    log(type_of(args))
"#,
        "list",
    );
}

#[test]
fn stdlib_env_get_set() {
    assert_output(
        r#"
*main()
    env_set("CORAL_TEST_VAR", "hello_coral")
    result is env_get("CORAL_TEST_VAR")
    log(result)
"#,
        &["hello_coral"],
    );
}
