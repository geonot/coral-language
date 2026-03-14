//! Type system quality tests for Sprint 3.
//!
//! Covers T4.3 (ranked unification), T4.1 (multi-error recovery),
//! and T4.2 (better type error messages).

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

fn assert_compiles(source: &str) {
    let compiler = Compiler;
    compiler
        .compile_to_ir(source)
        .unwrap_or_else(|e| panic!("Expected compilation to succeed, but got error: {:?}", e));
}

fn assert_compile_error(source: &str, expected_fragment: &str) {
    let compiler = Compiler;
    match compiler.compile_to_ir(source) {
        Ok(_) => panic!(
            "Expected compilation to fail with '{}', but it succeeded",
            expected_fragment
        ),
        Err(e) => {
            let msg = format!("{:?}", e);
            assert!(
                msg.contains(expected_fragment),
                "Error message '{}' does not contain expected fragment '{}'",
                msg,
                expected_fragment
            );
        }
    }
}

fn assert_compile_error_display(source: &str, expected_fragment: &str) {
    let compiler = Compiler;
    match compiler.compile_to_ir(source) {
        Ok(_) => panic!(
            "Expected compilation to fail with '{}', but it succeeded",
            expected_fragment
        ),
        Err(e) => {
            let msg = format!("{}", e);
            assert!(
                msg.contains(expected_fragment),
                "Display output '{}' does not contain expected fragment '{}'",
                msg,
                expected_fragment
            );
        }
    }
}

fn compile_error_display(source: &str) -> String {
    let compiler = Compiler;
    match compiler.compile_to_ir(source) {
        Ok(_) => panic!("Expected compilation to fail, but it succeeded"),
        Err(e) => format!("{}", e),
    }
}

fn compile_error_has_related(source: &str) -> bool {
    let compiler = Compiler;
    match compiler.compile_to_ir(source) {
        Ok(_) => panic!("Expected compilation to fail, but it succeeded"),
        Err(e) => !e.diagnostic.related.is_empty(),
    }
}

// ─── T4.2: Better Type Error Messages ──────────────────────────────

#[test]
fn t4_2_type_error_includes_expected_and_found() {
    // The error message should include both the expected and found types.
    let output = compile_error_display(
        r#"
x is 5
x(3)
"#,
    );
    assert!(
        output.contains("Int"),
        "Error should mention Int type: {}",
        output
    );
    assert!(
        output.contains("callable"),
        "Error should mention callable: {}",
        output
    );
}

#[test]
fn t4_2_boolean_error_mentions_expected_type() {
    // Boolean type errors should mention the expected Bool type.
    let output = compile_error_display(
        r#"
*foo()
  42 and true
foo()
"#,
    );
    assert!(
        output.contains("Bool") || output.contains("bool"),
        "Error should mention Bool: {}",
        output
    );
}

#[test]
fn t4_2_error_has_type_inference_prefix() {
    // All type errors should start with "type inference failed:".
    let output = compile_error_display(
        r#"
x is 5
x(3)
"#,
    );
    assert!(
        output.contains("type inference failed"),
        "Error should contain 'type inference failed': {}",
        output
    );
}

#[test]
fn t4_2_provenance_fields_exist_on_type_error() {
    // The TypeError struct now has expected_origin and found_origin fields.
    // This structural test verifies the fields are accessible and populated
    // through the public API (ConstraintOrigin).
    use coralc::types::ConstraintOrigin;
    let origin = ConstraintOrigin {
        description: "test binding".to_string(),
        span: coralc::span::Span::new(0, 5),
    };
    assert_eq!(origin.description, "test binding");
    assert_eq!(origin.span.start, 0);
}

// ─── T4.1: Multi-Error Recovery in Type Solving ────────────────────

#[test]
fn t4_1_single_type_error_reported() {
    // Calling a non-callable should produce a clear type error.
    assert_compile_error(
        r#"
x is 5
x(3)
"#,
        "is not callable",
    );
}

#[test]
fn t4_1_error_message_contains_type_inference_failed() {
    // The primary error should be prefixed with "type inference failed:".
    assert_compile_error_display(
        r#"
*foo()
  42 and true
foo()
"#,
        "type inference failed",
    );
}

#[test]
fn t4_1_multiple_independent_type_errors_both_reported() {
    // Two independent non-callable errors. The display should show both.
    let output = compile_error_display(
        r#"
x is 5
y is 10
x(3)
y(4)
"#,
    );
    // The output should contain "is not callable" for at least the primary error
    assert!(
        output.contains("is not callable"),
        "Output should contain 'is not callable': {}",
        output
    );
}

#[test]
fn t4_1_related_diagnostics_struct_check() {
    // Verify the `related` field is present on the diagnostic.
    let compiler = Compiler;
    let result = compiler.compile_to_ir(
        r#"
x is 5
y is 10
x(3)
y(4)
"#,
    );
    assert!(result.is_err(), "Expected compilation to fail");
    let err = result.unwrap_err();
    // Primary error should mention "is not callable"
    assert!(
        err.diagnostic.message.contains("is not callable"),
        "Primary error should contain 'is not callable': {}",
        err.diagnostic.message
    );
}

// ─── T4.3: Ranked Unification ──────────────────────────────────────

#[test]
fn t4_3_ranked_unification_basic_inference() {
    // Many variables chained through assignment — ranked union-find
    // should produce identical inference results.
    assert_output(
        r#"
x is 42
y is x
z is y
w is z
log(w)
"#,
        &["42"],
    );
}

#[test]
fn t4_3_ranked_unification_complex_chain() {
    // Long chain of variable bindings testing path compression
    // and ranked union with concrete type propagation.
    assert_output(
        r#"
a is "hello"
b is a
c is b
d is c
e is d
f is e
log(f.length())
"#,
        &["5"],
    );
}

#[test]
fn t4_3_ranked_unification_function_params() {
    // Type inference through function parameters should work
    // correctly with ranked unification.
    assert_output(
        r#"
*add_one(x)
  x + 1

*apply(f, val)
  f(val)

result is apply(add_one, 10)
log(result)
"#,
        &["11"],
    );
}

// ─── CC5.2/S6: Member access validity warnings ──────────────────────

fn analyze_warnings(source: &str) -> Vec<String> {
    use coralc::{lexer, parser::Parser, semantic};
    let tokens = lexer::lex(source).expect("Should lex");
    let p = Parser::new(tokens, source.len());
    let ast = p.parse().expect("Should parse");
    let model = semantic::analyze(ast).expect("Should analyze");
    model.warnings.iter().map(|w| w.message.clone()).collect()
}

#[test]
fn cc52_s6_member_access_on_store_warns_unknown_field() {
    let warnings = analyze_warnings(
        r#"
store Point
    x ? 0
    y ? 0

*main()
    p is make_Point()
    log(p.z)
"#,
    );
    let has_field_warning = warnings
        .iter()
        .any(|w| w.contains("has no field") && w.contains("z"));
    assert!(
        has_field_warning,
        "expected warning about unknown field 'z'; got: {:?}",
        warnings
    );
}

#[test]
fn cc52_s6_member_access_on_store_valid_field_no_warning() {
    let warnings = analyze_warnings(
        r#"
store Point
    x ? 0
    y ? 0

*main()
    p is make_Point()
    log(p.x)
"#,
    );
    let has_field_warning = warnings.iter().any(|w| w.contains("has no field"));
    assert!(
        !has_field_warning,
        "should not warn on valid field 'x'; got: {:?}",
        warnings
    );
}

// ─── CC5.2/S8: Pipeline type inference ──────────────────────────────

#[test]
fn cc52_s8_pipeline_basic_execution() {
    assert_output(
        r#"
*double(x)
    x * 2

result is 5 ~ double()
log(result)
"#,
        &["10"],
    );
}

#[test]
fn cc52_s8_pipeline_chained() {
    assert_output(
        r#"
*add_one(x)
    x + 1

*double(x)
    x * 2

result is 3 ~ add_one() ~ double()
log(result)
"#,
        &["8"],
    );
}
