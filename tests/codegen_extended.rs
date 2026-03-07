//! Extended codegen/execution tests for Phase B.
//!
//! Covers: stores, pattern matching, lambdas, closures, error values,
//! pipeline operations, and advanced language features.

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

fn compile_ok(source: &str) -> String {
    let compiler = Compiler;
    compiler.compile_to_ir(source).expect("Should compile")
}

fn compile_err(source: &str) -> String {
    let compiler = Compiler;
    match compiler.compile_to_ir(source) {
        Err(e) => format!("{:?}", e),
        Ok(_) => panic!("Expected compilation to fail, but it succeeded"),
    }
}

// ─── Store Tests ─────────────────────────────────────────────────────

#[test]
fn store_basic_construction_and_field_access() {
    assert_output(r#"
store Point
    x ? 10
    y ? 20

*main()
    p is make_Point()
    log(p.x)
    log(p.y)
"#, &["10", "20"]);
}

#[test]
fn store_with_method() {
    assert_output(r#"
store Counter
    count ? 0

    *increment()
        self.count is self.count + 1

    *get_count()
        self.count

*main()
    c is make_Counter()
    c.increment()
    c.increment()
    c.increment()
    log(c.get_count())
"#, &["3"]);
}

#[test]
fn store_method_returns_value() {
    assert_output(r#"
store Rect
    w ? 5
    h ? 3

    *area()
        return self.w * self.h

*main()
    r is make_Rect()
    log(r.area())
"#, &["15"]);
}

#[test]
fn store_multiple_instances() {
    assert_output(r#"
store Pair
    first ? 0
    second ? 0

    *set_vals(a, b)
        self.first is a
        self.second is b

*main()
    a is make_Pair()
    a.set_vals(1, 2)
    b is make_Pair()
    b.set_vals(10, 20)
    log(a.first + b.first)
    log(a.second + b.second)
"#, &["11", "22"]);
}

// ─── Lambda / Higher-order Function Tests ────────────────────────────

#[test]
fn lambda_basic() {
    assert_output(r#"
*apply(f, x)
    f(x)

*main()
    result is apply(*fn(n) n * 2, 21)
    log(result)
"#, &["42"]);
}

#[test]
fn lambda_in_list_map() {
    assert_output(r#"
*main()
    lst is [1, 2, 3]
    result is lst.map(*fn(x) x * 10)
    log(result.get(0))
    log(result.get(1))
    log(result.get(2))
"#, &["10", "20", "30"]);
}

#[test]
fn lambda_in_list_filter() {
    assert_output(r#"
*main()
    lst is [1, 2, 3, 4, 5, 6]
    big is lst.filter(*fn(x) x > 3)
    log(big.length)
    log(big.get(0))
"#, &["3", "4"]);
}

#[test]
fn lambda_in_list_reduce() {
    assert_output(r#"
*main()
    lst is [1, 2, 3, 4]
    total is lst.reduce(0, *fn(acc, x) acc + x)
    log(total)
"#, &["10"]);
}

#[test]
fn lambda_reduce_product() {
    assert_output(r#"
*main()
    lst is [2, 3, 4]
    product is lst.reduce(1, *fn(acc, x) acc * x)
    log(product)
"#, &["24"]);
}

// ─── Error Value Tests ───────────────────────────────────────────────

#[test]
fn error_value_is_err_check() {
    assert_output(r#"
*main()
    e is err NotFound
    log(is_err(e))
    log(is_ok(e))
"#, &["true", "false"]);
}

#[test]
fn error_value_name_extraction() {
    assert_output(r#"
*main()
    e is err Timeout
    log(error_name(e))
"#, &["Timeout"]);
}

#[test]
fn error_hierarchical_name() {
    assert_output(r#"
*main()
    e is err Database:Connection
    log(error_name(e))
"#, &["Database:Connection"]);
}

#[test]
fn normal_value_is_ok() {
    assert_output(r#"
*main()
    x is 42
    log(is_ok(x))
    log(is_err(x))
"#, &["true", "false"]);
}

// ─── Ternary Tests ──────────────────────────────────────────────────

#[test]
fn ternary_true_branch() {
    assert_output(r#"
*main()
    x is 10
    result is x > 5 ? "big" ! "small"
    log(result)
"#, &["big"]);
}

#[test]
fn ternary_false_branch() {
    assert_output(r#"
*main()
    x is 3
    result is x > 5 ? "big" ! "small"
    log(result)
"#, &["small"]);
}

#[test]
fn ternary_nested() {
    assert_output(r#"
*main()
    x is 50
    label is x > 100 ? "huge" ! x > 10 ? "medium" ! "small"
    log(label)
"#, &["medium"]);
}

// ─── Pipeline Tests (E2E) ───────────────────────────────────────────

#[test]
fn pipeline_simple_function_chain() {
    assert_output(r#"
*double(x)
    x * 2

*add_one(x)
    x + 1

*main()
    result is 5 ~ double ~ add_one
    log(result)
"#, &["11"]);
}

#[test]
fn pipeline_three_stages() {
    assert_output(r#"
*negate(x)
    0 - x

*double(x)
    x * 2

*add_ten(x)
    x + 10

*main()
    result is 3 ~ double ~ add_ten ~ negate
    log(result)
"#, &["-16"]);
}

// ─── While Loop Tests ───────────────────────────────────────────────

#[test]
fn while_loop_countdown() {
    assert_output(r#"
*main()
    i is 5
    while i > 0
        log(i)
        i is i - 1
"#, &["5", "4", "3", "2", "1"]);
}

#[test]
fn while_loop_accumulate() {
    assert_output(r#"
*main()
    sum is 0
    i is 1
    while i < 6
        sum is sum + i
        i is i + 1
    log(sum)
"#, &["15"]);
}

// ─── For Loop Tests ─────────────────────────────────────────────────

#[test]
fn for_loop_over_list() {
    assert_output(r#"
*main()
    items is [10, 20, 30]
    for item in items
        log(item)
"#, &["10", "20", "30"]);
}

#[test]
fn for_loop_accumulate() {
    assert_output(r#"
*main()
    total is 0
    for x in [1, 2, 3, 4]
        total is total + x
    log(total)
"#, &["10"]);
}

// ─── Nested Function Calls ──────────────────────────────────────────

#[test]
fn nested_function_calls() {
    assert_output(r#"
*square(x)
    x * x

*double(x)
    x * 2

*main()
    result is square(double(3))
    log(result)
"#, &["36"]);
}

#[test]
fn function_with_multiple_args() {
    assert_output(r#"
*add3(a, b, c)
    a + b + c

*main()
    log(add3(10, 20, 30))
"#, &["60"]);
}

// ─── String Concatenation Tests ─────────────────────────────────────

#[test]
fn string_concat_operator() {
    assert_output(r#"
*main()
    greeting is concat("hello", " world")
    log(greeting)
"#, &["hello world"]);
}

#[test]
fn string_length() {
    assert_output(r#"
*main()
    s is "hello"
    log(s.length)
"#, &["5"]);
}

// ─── List Operations E2E ────────────────────────────────────────────

#[test]
fn list_push_and_length() {
    assert_output(r#"
*main()
    lst is [1, 2, 3]
    lst.push(4)
    log(lst.length)
    log(lst.get(3))
"#, &["4", "4"]);
}

#[test]
fn list_nested_access() {
    assert_output(r#"
*main()
    lst is [100, 200, 300, 400, 500]
    log(lst.get(0))
    log(lst.get(4))
    log(lst.length)
"#, &["100", "500", "5"]);
}

#[test]
fn list_empty_creation() {
    assert_output(r#"
*main()
    lst is []
    log(lst.length)
    lst.push(42)
    log(lst.length)
    log(lst.get(0))
"#, &["0", "1", "42"]);
}

// ─── Map Operations E2E ─────────────────────────────────────────────

#[test]
fn map_basic_creation() {
    assert_output(r#"
*main()
    m is map("name" is "coral", "version" is "1.0")
    log(m.length)
"#, &["2"]);
}

#[test]
fn map_get_and_has() {
    assert_output(r#"
*main()
    m is map("x" is 10, "y" is 20)
    log(has_key(m, "x"))
    log(has_key(m, "z"))
"#, &["true", "false"]);
}

// ─── Recursion Tests ────────────────────────────────────────────────

#[test]
fn recursive_factorial() {
    assert_output(r#"
*factorial(n)
    n < 2 ? 1 ! n * factorial(n - 1)

*main()
    log(factorial(5))
    log(factorial(0))
    log(factorial(1))
"#, &["120", "1", "1"]);
}

#[test]
fn recursive_fibonacci() {
    assert_output(r#"
*fib(n)
    n < 2 ? n ! fib(n - 1) + fib(n - 2)

*main()
    log(fib(0))
    log(fib(1))
    log(fib(5))
    log(fib(10))
"#, &["0", "1", "5", "55"]);
}

// ─── Match Expression Tests ─────────────────────────────────────────

#[test]
fn match_on_number() {
    assert_output(r#"
*describe(n)
    return match n
        1 ? "one"
        2 ? "two"
        3 ? "three"
        ! "other"

*main()
    log(describe(1))
    log(describe(2))
    log(describe(5))
"#, &["one", "two", "other"]);
}

#[test]
fn match_on_bool() {
    assert_output(r#"
*to_label(val)
    return match val
        true ? "yes"
        false ? "no"

*main()
    log(to_label(true))
    log(to_label(false))
"#, &["yes", "no"]);
}

// ─── If/Else Tests ──────────────────────────────────────────────────

#[test]
fn if_else_basic() {
    assert_output(r#"
*classify(x)
    if x > 0
        log("positive")
    else
        log("non-positive")

*main()
    classify(5)
    classify(-3)
    classify(0)
"#, &["positive", "non-positive", "non-positive"]);
}

#[test]
fn if_elif_else() {
    assert_output(r#"
*grade(score)
    if score > 90
        log("A")
    elif score > 80
        log("B")
    elif score > 70
        log("C")
    else
        log("F")

*main()
    grade(95)
    grade(85)
    grade(75)
    grade(50)
"#, &["A", "B", "C", "F"]);
}

// ─── Scope Tests ────────────────────────────────────────────────────

#[test]
fn variable_shadowing_in_function() {
    assert_output(r#"
*main()
    x is 10
    log(x)
    x is 20
    log(x)
"#, &["10", "20"]);
}

#[test]
fn function_scope_isolation() {
    assert_output(r#"
*get_value()
    42

*main()
    result is get_value()
    log(result)
"#, &["42"]);
}

// ─── Bitwise Operations E2E ─────────────────────────────────────────

#[test]
fn bitwise_and_or_xor() {
    assert_output(r#"
*main()
    log(bit_and(12, 10))
    log(bit_or(12, 10))
    log(bit_xor(12, 10))
"#, &["8", "14", "6"]);
}

#[test]
fn bitwise_shift() {
    assert_output(r#"
*main()
    log(bit_shl(1, 4))
    log(bit_shr(32, 2))
"#, &["16", "8"]);
}

// ─── Type Introspection ─────────────────────────────────────────────

#[test]
fn type_of_all_types() {
    assert_output(r#"
*main()
    log(type_of(42))
    log(type_of("hello"))
    log(type_of(true))
    log(type_of([1]))
"#, &["number", "string", "bool", "list"]);
}

// ─── Math E2E ───────────────────────────────────────────────────────

#[test]
fn math_abs_floor_ceil() {
    assert_output(r#"
*main()
    log(abs(-7))
    log(floor(3.9))
    log(ceil(3.1))
"#, &["7", "3", "4"]);
}

#[test]
fn math_min_max() {
    assert_output(r#"
*main()
    log(min(5, 3))
    log(max(5, 3))
    log(min(-1, 1))
"#, &["3", "5", "-1"]);
}

#[test]
fn math_pow_sqrt() {
    assert_output(r#"
*main()
    log(pow(2, 8))
    log(sqrt(144))
"#, &["256", "12"]);
}

// ─── Character Operations ───────────────────────────────────────────

#[test]
fn char_operations_e2e() {
    assert_output(r#"
*main()
    log(ord("Z"))
    log(chr(97))
"#, &["90", "a"]);
}

// ─── Bytes Operations ───────────────────────────────────────────────

#[test]
fn bytes_roundtrip() {
    assert_output(r#"
*main()
    b is bytes_from_string("coral")
    s is bytes_to_string(b)
    log(s)
"#, &["coral"]);
}
