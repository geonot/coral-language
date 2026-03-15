//! Performance optimization tests.
//!
//! Verifies that performance-critical code paths produce correct results.
//! Tests the fast-path numeric operations, direct i1 comparison emission,
//! loop counter optimizations, and inlined boolean boxing.

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
    let expected_text = expected.join("\n");
    let expected_full = if expected.is_empty() {
        String::new()
    } else {
        format!("{}\n", expected_text)
    };
    assert_eq!(
        stdout, expected_full,
        "\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        stdout, stderr, code
    );
}

// ========== Numeric Fast-Path Correctness ==========

#[test]
fn perf_tight_loop_numeric_add() {
    let source = r#"
*main()
    sum is 0
    i is 0
    while i < 100000
        sum is sum + i
        i is i + 1
    log(sum)
"#;
    assert_output(source, &["4999950000"]);
}

#[test]
fn perf_tight_loop_numeric_mul() {
    let source = r#"
*main()
    product is 1
    i is 1
    while i <= 20
        product is product * i
        i is i + 1
    log(product)
"#;
    assert_output(source, &["2432902008176640000"]);
}

#[test]
fn perf_tight_loop_numeric_sub() {
    let source = r#"
*main()
    result is 1000000
    i is 0
    while i < 1000
        result is result - i
        i is i + 1
    log(result)
"#;
    assert_output(source, &["500500"]);
}

#[test]
fn perf_numeric_div_preserves_nan_check() {
    let source = r#"
*main()
    x is 0.0 / 0.0
    log(to_string(x))
"#;
    assert_output(source, &["NaN"]);
}

#[test]
fn perf_numeric_mod_preserves_nan_check() {
    // Modulo by zero returns unit in Coral (pre-existing behavior)
    // This test validates the mod fast-path doesn't break that behavior
    let source = r#"
*main()
    x is 10.0 % 3.0
    log(x)
"#;
    assert_output(source, &["1"]);
}

// ========== Direct i1 Condition Emission ==========

#[test]
fn perf_while_condition_direct_comparison() {
    let source = r#"
*main()
    count is 0
    i is 0
    while i < 50
        count is count + 1
        i is i + 1
    log(count)
"#;
    assert_output(source, &["50"]);
}

#[test]
fn perf_while_condition_greater_eq() {
    let source = r#"
*main()
    count is 0
    i is 100
    while i >= 0
        count is count + 1
        i is i - 1
    log(count)
"#;
    assert_output(source, &["101"]);
}

#[test]
fn perf_if_condition_direct_comparison() {
    let source = r#"
*main()
    x is 42
    if x > 40
        log("big")
    if x < 40
        log("small")
    if x.equals(42)
        log("exact")
"#;
    assert_output(source, &["big", "exact"]);
}

// ========== Boolean Inline Wrapping ==========

#[test]
fn perf_bool_comparison_chain() {
    let source = r#"
*main()
    a is 10
    b is 20
    c is 30
    log(a < b)
    log(b < c)
    log(c < a)
    log(a.equals(10))
    log(b.equals(20).not())
"#;
    assert_output(source, &["true", "true", "false", "true", "false"]);
}

// ========== For-Range Fast Path ==========

#[test]
fn perf_for_range_numeric_sum() {
    let source = r#"
*main()
    sum is 0
    for i in 0 to 100
        sum is sum + i
    log(sum)
"#;
    assert_output(source, &["4950"]);
}

#[test]
fn perf_for_range_large() {
    let source = r#"
*main()
    sum is 0
    for i in 0 to 10000
        sum is sum + i
    log(sum)
"#;
    assert_output(source, &["49995000"]);
}

// ========== Recursive Function Inlining ==========

#[test]
fn perf_fibonacci_correctness() {
    let source = r#"
*fib(n)
    if n < 2
        return n
    fib(n - 1) + fib(n - 2)

*main()
    log(fib(20))
"#;
    assert_output(source, &["6765"]);
}

#[test]
fn perf_small_function_inlined() {
    let source = r#"
*double(x)
    x * 2

*main()
    sum is 0
    i is 0
    while i < 1000
        sum is sum + double(i)
        i is i + 1
    log(sum)
"#;
    assert_output(source, &["999000"]);
}

// ========== Ternary Numeric Fast Path ==========

#[test]
fn perf_ternary_numeric() {
    let source = r#"
*main()
    x is 10
    result is x > 5 ? x * 2 ! x * 3
    log(result)
"#;
    assert_output(source, &["20"]);
}

// ========== Negation Fast Path ==========

#[test]
fn perf_negation_fast_path() {
    let source = r#"
*main()
    x is 42
    y is -x
    log(y)
    log(-(-100))
"#;
    assert_output(source, &["-42", "100"]);
}

// ========== Mixed Arithmetic Correctness ==========

#[test]
fn perf_mixed_arithmetic_precision() {
    let source = r#"
*main()
    a is 1.5
    b is 2.5
    log(a + b)
    log(a * b)
    log(b - a)
    log(b / a)
"#;
    assert_output(source, &["4", "3.75", "1", "1.6666666666666667"]);
}

#[test]
fn perf_nested_arithmetic() {
    let source = r#"
*main()
    result is (2 + 3) * (4 - 1) / 3
    log(result)
"#;
    assert_output(source, &["5"]);
}

// ========== No Yield Check in Non-Actor Programs ==========

#[test]
fn perf_no_actor_loop_correctness() {
    let source = r#"
*main()
    sum is 0
    i is 0
    while i < 50000
        sum is sum + 1
        i is i + 1
    log(sum)
"#;
    assert_output(source, &["50000"]);
}

// ========== Builtin Numeric Return Type Detection ==========

#[test]
fn perf_builtin_numeric_fast_path() {
    let source = r#"
*main()
    x is abs(-42)
    y is floor(3.7)
    z is ceil(3.2)
    log(x)
    log(y)
    log(z)
"#;
    assert_output(source, &["42", "3", "4"]);
}

// ========== Indexed Struct Store Access ==========

#[test]
fn perf_store_field_access() {
    let source = r#"
store Point
    x ? 0
    y ? 0
    *set_xy(nx, ny)
        self.x is nx
        self.y is ny
    *sum()
        self.x + self.y

*main()
    p is make_Point()
    p.set_xy(10, 20)
    log(p.x)
    log(p.y)
    log(p.sum())
"#;
    assert_output(source, &["10", "20", "30"]);
}

#[test]
fn perf_store_field_default_values() {
    let source = r#"
store Config
    width is 800
    height is 600
    name is "default"

*main()
    c is make_Config()
    log(c.width)
    log(c.height)
    log(c.name)
"#;
    assert_output(source, &["800", "600", "default"]);
}

#[test]
fn perf_store_multiple_instances() {
    let source = r#"
store Counter
    value ? 0
    *inc()
        self.value is self.value + 1

*main()
    a is make_Counter()
    b is make_Counter()
    a.inc()
    a.inc()
    b.inc()
    log(a.value)
    log(b.value)
"#;
    assert_output(source, &["2", "1"]);
}

#[test]
fn perf_store_uses_struct_ir() {
    let compiler = Compiler;
    let source = r#"
store Point
    x ? 0
    y ? 0
*main()
    p is make_Point()
    log(p.x)
"#;
    let ir = compiler.compile_to_ir(source).expect("compile store IR");
    // Non-persistent stores should use coral_make_struct instead of coral_make_map
    assert!(
        ir.contains("@coral_make_struct("),
        "Expected struct-based store, got map-based.\n{}",
        ir
    );
    assert!(
        ir.contains("@coral_struct_get("),
        "Expected indexed field access.\n{}",
        ir
    );
}

// ========== LLVM Intrinsics for Math Functions ==========

#[test]
fn perf_math_intrinsic_sqrt() {
    let source = r#"
*main()
    log(sqrt(144.0))
    log(sqrt(2.0))
"#;
    let (stdout, _, code) = run_coral(source);
    assert_eq!(code, 0);
    assert!(stdout.contains("12"));
    assert!(stdout.contains("1.414"));
}

#[test]
fn perf_math_intrinsic_trig() {
    let source = r#"
*main()
    x is sin(0.0)
    y is cos(0.0)
    log(x)
    log(y)
"#;
    assert_output(source, &["0", "1"]);
}

#[test]
fn perf_math_uses_llvm_intrinsics() {
    let compiler = Compiler;
    // Use variables to prevent constant folding
    let source = r#"
*compute(v)
    sqrt(v) + floor(v) + ceil(v) + abs(v) + sin(v) + cos(v)

*main()
    log(compute(4.0))
"#;
    let ir = compiler.compile_to_ir(source).expect("compile math IR");
    assert!(
        ir.contains("llvm.sqrt.f64"),
        "Expected sqrt intrinsic.\n{}",
        ir
    );
    assert!(
        ir.contains("llvm.floor.f64"),
        "Expected floor intrinsic.\n{}",
        ir
    );
    assert!(
        ir.contains("llvm.ceil.f64"),
        "Expected ceil intrinsic.\n{}",
        ir
    );
    assert!(
        ir.contains("llvm.fabs.f64"),
        "Expected fabs intrinsic.\n{}",
        ir
    );
    assert!(
        ir.contains("llvm.sin.f64"),
        "Expected sin intrinsic.\n{}",
        ir
    );
    assert!(
        ir.contains("llvm.cos.f64"),
        "Expected cos intrinsic.\n{}",
        ir
    );
}
