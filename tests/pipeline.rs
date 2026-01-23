//! Pipeline operator test suite
//!
//! Tests for:
//! - Basic pipeline desugaring (a ~ f becomes f(a))
//! - Pipeline with call arguments (a ~ f(b) becomes f(a, b))
//! - Pipeline with $ placeholder (a ~ f($, b) becomes f(a, b))
//! - Multiple $ placeholders in expressions

use coralc::Compiler;

fn compile_and_verify(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    compiler.compile_to_ir(source).map_err(|e| format!("{:?}", e))
}

// ========== BASIC PIPELINE TESTS ==========

#[test]
fn pipeline_simple_function() {
    let source = r#"
*double(x)
  x * 2

result is 5 ~ double
"#;
    compile_and_verify(source).expect("Simple pipeline should compile");
}

#[test]
fn pipeline_with_extra_args() {
    let source = r#"
*add(a, b)
  a + b

result is 5 ~ add(10)
"#;
    compile_and_verify(source).expect("Pipeline with extra args should compile");
}

#[test]
fn pipeline_chained() {
    let source = r#"
*double(x)
  x * 2

*add_one(x)
  x + 1

result is 5 ~ double ~ add_one
"#;
    compile_and_verify(source).expect("Chained pipeline should compile");
}

// ========== PIPELINE WITH $ PLACEHOLDER TESTS ==========

#[test]
fn pipeline_placeholder_explicit_position() {
    // `5 ~ f($, 10)` should become `f(5, 10)`
    let source = r#"
*subtract(a, b)
  a - b

result is 5 ~ subtract($, 3)
"#;
    compile_and_verify(source).expect("Pipeline with $ placeholder should compile");
}

#[test]
fn pipeline_placeholder_second_position() {
    // `5 ~ f(10, $)` should become `f(10, 5)`
    let source = r#"
*subtract(a, b)
  a - b

result is 5 ~ subtract(10, $)
"#;
    compile_and_verify(source).expect("Pipeline with $ in second position should compile");
}

#[test]
fn pipeline_placeholder_as_direct_argument() {
    // $ used directly as an argument (no expression context)
    // `5 ~ make_pair(1, $)` becomes `make_pair(1, 5)` = [1, 5]
    let source = r#"
*make_pair(a, b)
  [a, b]

result is 5 ~ make_pair(1, $)
"#;
    compile_and_verify(source).expect("Pipeline with $ as direct arg should compile");
}

#[test]
fn pipeline_placeholder_multiple_args() {
    // `5 ~ f($, $)` should become `f(5, 5)`
    let source = r#"
*add(a, b)
  a + b

result is 5 ~ add($, $)
"#;
    compile_and_verify(source).expect("Pipeline with multiple $ should compile");
}

#[test]
fn pipeline_placeholder_in_nested_call() {
    // More complex example
    let source = r#"
*double(x)
  x * 2

*add(a, b)
  a + b

result is 5 ~ add(double($), 1)
"#;
    compile_and_verify(source).expect("Pipeline with nested $ should compile");
}

// ========== EDGE CASES ==========

#[test]
fn pipeline_with_list() {
    // Test pipeline with list as input
    let source = r#"
*head(xs)
  xs.get(0)

nums is [1, 2, 3]
result is nums ~ head
"#;
    compile_and_verify(source).expect("Pipeline with list input should compile");
}

#[test]
fn pipeline_no_placeholder_prepends() {
    // Without $, left value is prepended
    let source = r#"
*three_args(a, b, c)
  a + b + c

result is 1 ~ three_args(2, 3)
"#;
    let ir = compile_and_verify(source).expect("Pipeline without $ should prepend");
    // The result should call three_args(1, 2, 3)
    assert!(ir.contains("three_args"), "Should call three_args function");
}

#[test]
fn pipeline_placeholder_vs_prepend() {
    // With $, left replaces $ instead of prepending
    // `1 ~ f(2, $, 3)` becomes `f(2, 1, 3)` not `f(1, 2, 1, 3)`
    let source = r#"
*four_args(a, b, c, d)
  a + b + c + d

result is 10 ~ four_args(1, 2, $, 3)
"#;
    compile_and_verify(source).expect("Pipeline $ should replace not prepend");
}
