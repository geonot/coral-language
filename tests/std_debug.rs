//! L4.1: std.debug module tests
//!
//! Tests for debug_inspect() and time_ns() debug utilities.

use coralc::Compiler;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");

fn runtime_lib() -> PathBuf {
    let lib = PathBuf::from(WORKSPACE).join("target/debug/libruntime.so");
    assert!(
        lib.exists(),
        "Runtime library not found at {}. Run `cargo build -p runtime` first.",
        lib.display()
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

fn assert_output_contains(source: &str, substring: &str) {
    let (stdout, stderr, code) = run_coral(source);
    assert!(
        stdout.contains(substring),
        "Expected stdout to contain {:?} but got:\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        substring, stdout, stderr, code
    );
}

#[test]
fn l41_inspect_number() {
    assert_output(
        r#"
*main()
    x is 42
    log(debug_inspect(x))
"#,
        &["Number(42)"],
    );
}

#[test]
fn l41_inspect_string() {
    assert_output(
        r#"
*main()
    s is "hello"
    log(debug_inspect(s))
"#,
        &["String(hello)[len=5]"],
    );
}

#[test]
fn l41_inspect_list() {
    assert_output(
        r#"
*main()
    xs is [1, 2, 3]
    log(debug_inspect(xs))
"#,
        &["List[3 items]"],
    );
}

#[test]
fn l41_time_ns_monotonic() {
    // time_ns() should return a number (we just verify it compiles and runs)
    assert_output(
        r#"
*main()
    t1 is time_ns()
    t2 is time_ns()
    log(t2 >= t1)
"#,
        &["true"],
    );
}
