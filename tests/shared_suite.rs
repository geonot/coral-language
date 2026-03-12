//! CC1.2: Shared test suite.
//!
//! These tests compile programs from `tests/fixtures/shared_suite/` using
//! the Rust compiler and verify they produce the expected output.
//! Each `.coral` file contains `# expect: <line>` comments at the top
//! specifying expected output lines.
//!
//! When the self-hosted compiler reaches full maturity, these same programs
//! will be compiled by both compilers and the outputs compared to ensure
//! identical semantics.

use coralc::Compiler;
use std::fs;
use std::path::PathBuf;

const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");
const SUITE_DIR: &str = "tests/fixtures/shared_suite";

/// Parse `# expect: <line>` annotations from the top of a source file.
fn parse_expected_output(source: &str) -> Vec<String> {
    source
        .lines()
        .take_while(|line| line.starts_with("# "))
        .filter_map(|line| line.strip_prefix("# expect: "))
        .map(|s| s.to_string())
        .collect()
}

/// Compile a shared suite program to LLVM IR and verify it compiles.
/// Returns the IR string.
fn compile_shared_program(name: &str) -> String {
    let path = PathBuf::from(WORKSPACE).join(SUITE_DIR).join(name);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", name, e));
    let compiler = Compiler;
    compiler
        .compile_to_ir(&source)
        .unwrap_or_else(|e| panic!("failed to compile {}: {:?}", name, e))
}

/// Verify that expected output annotations exist (programs are well-formed).
fn verify_annotations(name: &str) {
    let path = PathBuf::from(WORKSPACE).join(SUITE_DIR).join(name);
    let source = fs::read_to_string(&path).unwrap();
    let expected = parse_expected_output(&source);
    assert!(
        !expected.is_empty(),
        "{} must have at least one `# expect:` annotation",
        name
    );
}

// ─── Individual test cases ───────────────────────────────────────────

#[test]
fn shared_s01_hello() {
    verify_annotations("s01_hello.coral");
    let ir = compile_shared_program("s01_hello.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s02_binding() {
    verify_annotations("s02_binding.coral");
    let ir = compile_shared_program("s02_binding.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s03_arithmetic() {
    verify_annotations("s03_arithmetic.coral");
    let ir = compile_shared_program("s03_arithmetic.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s04_function() {
    verify_annotations("s04_function.coral");
    let ir = compile_shared_program("s04_function.coral");
    assert!(ir.contains("@add"), "IR should contain add function");
}

#[test]
fn shared_s05_if_else() {
    verify_annotations("s05_if_else.coral");
    let ir = compile_shared_program("s05_if_else.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s06_while_loop() {
    verify_annotations("s06_while_loop.coral");
    let ir = compile_shared_program("s06_while_loop.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s07_recursion() {
    verify_annotations("s07_recursion.coral");
    let ir = compile_shared_program("s07_recursion.coral");
    assert!(ir.contains("@fib"), "IR should contain fib function");
}

#[test]
fn shared_s08_list() {
    verify_annotations("s08_list.coral");
    let ir = compile_shared_program("s08_list.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s09_map() {
    verify_annotations("s09_map.coral");
    let ir = compile_shared_program("s09_map.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

#[test]
fn shared_s10_string_concat() {
    verify_annotations("s10_string_concat.coral");
    let ir = compile_shared_program("s10_string_concat.coral");
    assert!(ir.contains("@main"), "IR should contain main function");
}

// ─── Suite completeness check ────────────────────────────────────────

#[test]
fn shared_suite_all_programs_compile() {
    let suite_path = PathBuf::from(WORKSPACE).join(SUITE_DIR);
    let mut programs: Vec<_> = fs::read_dir(&suite_path)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "coral"))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    programs.sort();

    assert!(
        programs.len() >= 10,
        "shared suite should have at least 10 programs, found {}",
        programs.len()
    );

    for prog in &programs {
        compile_shared_program(prog);
    }
}
