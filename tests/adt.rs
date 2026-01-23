//! ADT (Algebraic Data Types) test suite
//!
//! Tests for:
//! - ADT construction (Some, None, custom variants)
//! - Pattern matching on ADTs
//! - Field extraction from matched variants
//! - Exhaustiveness checking

use coralc::Compiler;

fn compile_and_verify(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    compiler.compile_to_ir(source).map_err(|e| format!("{:?}", e))
}

fn expect_compile_error(source: &str, expected_substring: &str) {
    let compiler = Compiler;
    let result = compiler.compile_to_ir(source);

    match result {
        Ok(_) => panic!("Expected compile error containing '{}', but compilation succeeded", expected_substring),
        Err(e) => {
            let msg = format!("{:?}", e);
            assert!(
                msg.contains(expected_substring),
                "Expected error containing '{}', got: {}",
                expected_substring,
                msg
            );
        }
    }
}

// ========== ADT CONSTRUCTION TESTS ==========

#[test]
fn adt_construct_variant_with_field() {
    let source = r#"
enum Option
  Some(value)
  None

x is Some(42)
"#;
    compile_and_verify(source).expect("Should compile ADT with field");
}

#[test]
fn adt_construct_nullary_variant() {
    let source = r#"
enum Option
  Some(value)
  None

x is None
"#;
    compile_and_verify(source).expect("Should compile nullary ADT variant");
}

#[test]
fn adt_construct_multiple_fields() {
    let source = r#"
enum Result
  Ok(value)
  Error(code, message)

x is Error(404, "not found")
"#;
    compile_and_verify(source).expect("Should compile ADT with multiple fields");
}

#[test]
fn adt_custom_three_variants() {
    let source = r#"
enum Color
  Red
  Green
  Blue

c is Red
"#;
    compile_and_verify(source).expect("Should compile custom enum with 3 variants");
}

// ========== PATTERN MATCHING TESTS ==========

#[test]
fn match_adt_all_variants() {
    let source = r#"
enum Option
  Some(value)
  None

*describe(opt)
  match opt
    Some(v) ? v
    None ? 0

x is Some(42)
result is describe(x)
"#;
    compile_and_verify(source).expect("Should compile match on all ADT variants");
}

#[test]
fn match_adt_with_wildcard() {
    let source = r#"
enum Option
  Some(value)
  None

*is_some(opt)
  match opt
    Some(_) ? true
    None ? false

result is is_some(Some(1))
"#;
    compile_and_verify(source).expect("Should compile match with wildcard pattern");
}

#[test]
fn match_adt_with_default() {
    let source = r#"
enum Color
  Red
  Green
  Blue

*is_red(c)
  match c
    Red ? true
    _ ? false

result is is_red(Blue)
"#;
    compile_and_verify(source).expect("Should compile match with default arm");
}

#[test]
fn match_extract_field_value() {
    let source = r#"
enum Wrapper
  Value(inner)

*unwrap(w)
  match w
    Value(x) ? x

v is Value(100)
result is unwrap(v)
"#;
    compile_and_verify(source).expect("Should extract field value in pattern match");
}

#[test]
fn match_multiple_fields() {
    let source = r#"
enum Pair
  Pair(first, second)

*get_first(p)
  match p
    Pair(a, _) ? a

p is Pair(1, 2)
result is get_first(p)
"#;
    compile_and_verify(source).expect("Should extract multiple fields in pattern match");
}

// ========== EXHAUSTIVENESS CHECKING TESTS ==========

#[test]
fn exhaustiveness_error_missing_variant() {
    let source = r#"
enum Option
  Some(value)
  None

*test(opt)
  match opt
    Some(v) ? v

result is test(None)
"#;
    expect_compile_error(source, "non-exhaustive");
}

#[test]
fn exhaustiveness_error_multiple_missing() {
    let source = r#"
enum Color
  Red
  Green
  Blue

*name(c)
  match c
    Red ? "red"

result is name(Blue)
"#;
    expect_compile_error(source, "non-exhaustive");
}

#[test]
fn exhaustiveness_ok_with_wildcard() {
    let source = r#"
enum Color
  Red
  Green
  Blue

*is_red(c)
  match c
    Red ? true
    _ ? false

result is is_red(Green)
"#;
    compile_and_verify(source).expect("Wildcard should satisfy exhaustiveness");
}

#[test]
fn exhaustiveness_ok_with_identifier_catchall() {
    let source = r#"
enum Option
  Some(value)
  None

*safe_unwrap(opt)
  match opt
    Some(v) ? v
    other ? 0

result is safe_unwrap(None)
"#;
    compile_and_verify(source).expect("Identifier catchall should satisfy exhaustiveness");
}

#[test]
fn exhaustiveness_ok_all_variants_covered() {
    let source = r#"
enum Bool3
  Yes
  No
  Maybe

*to_num(b)
  match b
    Yes ? 1
    No ? 0
    Maybe ? -1

result is to_num(Maybe)
"#;
    compile_and_verify(source).expect("All variants covered should satisfy exhaustiveness");
}

// ========== EDGE CASE TESTS ==========

#[test]
fn adt_string_field() {
    let source = r#"
enum Message
  Text(content)
  Empty

m is Text("hello")
"#;
    compile_and_verify(source).expect("ADT with string field should compile");
}

#[test]
fn adt_nested_as_field() {
    let source = r#"
enum Option
  Some(value)
  None

enum Container
  Box(contents)

inner is Some(42)
outer is Box(inner)
"#;
    compile_and_verify(source).expect("Nested ADT as field should compile");
}

#[test]
fn match_after_function_call() {
    let source = r#"
enum Option
  Some(value)
  None

*make_some(x)
  Some(x)

opt is make_some(5)
result is match opt
  Some(v) ? v
  None ? 0
"#;
    compile_and_verify(source).expect("Match on function result should compile");
}

#[test]
fn multiple_match_expressions() {
    let source = r#"
enum Option
  Some(value)
  None

*process(a, b)
  x is match a
    Some(v) ? v
    None ? 0
  y is match b
    Some(v) ? v
    None ? 0
  x + y

result is process(Some(10), Some(20))
"#;
    compile_and_verify(source).expect("Multiple match expressions should compile");
}
