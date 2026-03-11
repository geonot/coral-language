//! R2.4: Cooperative yielding tests.
//!
//! Verifies that yield checks are emitted at loop back-edges and that
//! programs with tight loops still complete correctly.

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
    let ir = compiler
        .compile_to_ir(source)
        .unwrap_or_else(|e| panic!("Compilation failed: {:?}", e));
    let mut ir_file = tempfile::NamedTempFile::new().expect("temp file");
    ir_file.write_all(ir.as_bytes()).expect("write IR");
    ir_file.flush().expect("flush");
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

#[test]
fn r24_while_loop_with_yield_completes() {
    // A tight while loop should still complete (yield check doesn't break logic).
    assert_output(
        r#"
*main()
    i is 0
    while i < 2000
        i is i + 1
    log(i)
"#,
        &["2000"],
    );
}

#[test]
fn r24_for_range_with_yield_completes() {
    // A for-range loop should still produce correct results.
    assert_output(
        r#"
*main()
    total is 0
    for i in 0 to 100
        total is total + i
    log(total)
"#,
        &["4950"],
    );
}

#[test]
fn r24_for_in_with_yield_completes() {
    // A for-in loop should still iterate correctly.
    assert_output(
        r#"
*main()
    xs is [10, 20, 30]
    total is 0
    for x in xs
        total is total + x
    log(total)
"#,
        &["60"],
    );
}
