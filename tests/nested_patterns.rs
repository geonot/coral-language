//! Tests for nested pattern matching (4.2.5)
//!
//! These tests verify that match expressions can handle deeply nested
//! patterns like `Some(Some(x))`, `Ok(Some(x))`, and constructor patterns
//! with multiple levels of nesting.

use coralc::Compiler;

/// Compile and verify source compiles successfully to IR.
fn compile_ok(source: &str) -> String {
    let compiler = Compiler;
    compiler.compile_to_ir(source).expect("Should compile")
}

#[test]
fn nested_option_some_some() {
    let source = r#"
enum Option
  None
  Some(value)

*unwrap_nested(opt)
  match opt
    Some(Some(x)) ? x
    Some(None) ? -1
    None ? -2

nested is Some(Some(42))
result is unwrap_nested(nested)
"#;
    let ir = compile_ok(source);
    // Should generate nested tag checks
    assert!(
        ir.contains("coral_tagged_is_tag"),
        "Should check tags for nested patterns"
    );
}

#[test]
fn nested_option_some_none() {
    let source = r#"
enum Option
  None
  Some(value)

*unwrap_nested(opt)
  match opt
    Some(Some(x)) ? x
    Some(None) ? -1
    None ? -2

nested is Some(None)
result is unwrap_nested(nested)
"#;
    compile_ok(source);
}

#[test]
fn nested_option_outer_none() {
    let source = r#"
enum Option
  None
  Some(value)

*unwrap_nested(opt)
  match opt
    Some(Some(x)) ? x
    Some(None) ? -1
    None ? -2

nested is None
result is unwrap_nested(nested)
"#;
    compile_ok(source);
}

#[test]
fn nested_result_ok_some() {
    let source = r#"
enum Option
  None
  Some(value)

enum Result
  Ok(value)
  Err(error)

*extract(r)
  match r
    Ok(Some(x)) ? x
    Ok(None) ? -1
    Err(_) ? -999

result is Ok(Some(100))
out is extract(result)
"#;
    compile_ok(source);
}

#[test]
fn nested_result_ok_none() {
    let source = r#"
enum Option
  None
  Some(value)

enum Result
  Ok(value)
  Err(error)

*extract(r)
  match r
    Ok(Some(x)) ? x
    Ok(None) ? -1
    Err(_) ? -999

result is Ok(None)
out is extract(result)
"#;
    compile_ok(source);
}

#[test]
fn nested_result_err_with_wildcard() {
    let source = r#"
enum Option
  None
  Some(value)

enum Result
  Ok(value)
  Err(error)

*extract(r)
  match r
    Ok(Some(x)) ? x
    Ok(None) ? -1
    Err(_) ? -999

result is Err("something failed")
out is extract(result)
"#;
    compile_ok(source);
}

#[test]
fn triple_nested_option() {
    let source = r#"
enum Option
  None
  Some(value)

*deep_unwrap(opt)
  match opt
    Some(Some(Some(x))) ? x
    Some(Some(None)) ? 1
    Some(None) ? 2
    None ? 3

deep is Some(Some(Some(77)))
result is deep_unwrap(deep)
"#;
    compile_ok(source);
}

#[test]
fn nested_pattern_with_multiple_bindings() {
    let source = r#"
enum Pair
  Pair(first, second)

enum Option
  None
  Some(value)

*extract_both(opt)
  match opt
    Some(Pair(a, b)) ? a + b
    None ? 0

wrapped is Some(Pair(10, 20))
result is extract_both(wrapped)
"#;
    compile_ok(source);
}

#[test]
fn nested_pattern_with_literals() {
    let source = r#"
enum Option
  None
  Some(value)

*check_specific(opt)
  match opt
    Some(42) ? 1
    Some(x) ? x
    None ? 0

a is Some(42)
result is check_specific(a)
"#;
    compile_ok(source);
}

#[test]
fn nested_constructor_in_constructor_multiple_fields() {
    let source = r#"
enum Option
  None
  Some(value)

enum Triple
  Triple(a, b, c)

*extract_middle(t)
  match t
    Triple(Some(x), y, Some(z)) ? x + y + z
    Triple(Some(x), y, None) ? x + y
    Triple(None, y, Some(z)) ? y + z
    Triple(None, y, None) ? y

t1 is Triple(Some(1), 10, Some(100))
result is extract_middle(t1)
"#;
    compile_ok(source);
}

#[test]
fn nested_pattern_shadowing() {
    // Inner binding 'x' should shadow outer 'x'
    let source = r#"
enum Option
  None
  Some(value)

*check(opt)
  x is 1000
  match opt
    Some(x) ? x
    None ? x

result is check(Some(5))
"#;
    compile_ok(source);
}

#[test]
fn deeply_nested_four_levels() {
    let source = r#"
enum Option
  None
  Some(value)

*deep(opt)
  match opt
    Some(Some(Some(Some(x)))) ? x
    _ ? -1

val is Some(Some(Some(Some(999))))
result is deep(val)
"#;
    compile_ok(source);
}

#[test]
fn nested_with_wildcard_in_middle() {
    let source = r#"
enum Option
  None
  Some(value)

enum Pair
  Pair(first, second)

*check(p)
  match p
    Pair(Some(x), _) ? x
    Pair(None, Some(y)) ? y
    Pair(None, None) ? 0

a is Pair(Some(10), Some(20))
result is check(a)
"#;
    compile_ok(source);
}
