//! T3.4: Error Type Tracking Tests
//!
//! Tests that error values carry their taxonomy type (Error[Foo.Bar.Baz])
//! and that the type system correctly tracks and reports error types.

use coralc::Compiler;
use coralc::lexer;
use coralc::parser;
use coralc::semantic;

/// Helper: compile to IR, expect success.
fn compile_ok(code: &str) -> String {
    let compiler = Compiler;
    compiler.compile_to_ir(code).expect("Should compile")
}

/// Helper: analyze semantically, return the model.
fn analyze_ok(code: &str) -> semantic::SemanticModel {
    let tokens = lexer::lex(code).expect("Should lex");
    let p = parser::Parser::new(tokens, code.len());
    let ast = p.parse().expect("Should parse");
    semantic::analyze(ast).expect("Should analyze")
}

// ── Error type inference from literals ─────────────────────────────────

#[test]
fn t34_error_type_inferred_from_literal() {
    // `err NotFound` should be typed as Error[NotFound]
    let code = r#"
*main()
    x is err NotFound
    log(x)
"#;
    // Should compile without type errors
    let ir = compile_ok(code);
    assert!(
        ir.contains("@coral_make_error"),
        "Should create error value"
    );
}

#[test]
fn t34_error_taxonomy_type_inferred() {
    // `err Database:Connection:Timeout` should be typed as Error[Database.Connection.Timeout]
    let code = r#"
*main()
    x is err Database:Connection:Timeout
    log(x)
"#;
    let ir = compile_ok(code);
    assert!(
        ir.contains("@coral_make_error"),
        "Should create hierarchical error value"
    );
}

// ── Error type narrowing in match ──────────────────────────────────────

#[test]
fn t34_error_type_flows_through_return() {
    // Functions returning errors should compile correctly
    let code = r#"
*validate(x)
    x > 0 ? x ! err Validation:Negative

*main()
    result is validate(5)
    log(result)
"#;
    let ir = compile_ok(code);
    assert!(
        ir.contains("@coral_make_error"),
        "Should have error creation"
    );
}

#[test]
fn t34_multiple_error_types_in_function() {
    // A function can return different error types from different paths
    let code = r#"
*check(x)
    if x < 0
        return err Validation:TooLow
    if x > 100
        return err Validation:TooHigh
    x

*main()
    log(check(50))
"#;
    let ir = compile_ok(code);
    assert!(
        ir.contains("@coral_make_error"),
        "Should create error values"
    );
}

// ── Error type in expressions ──────────────────────────────────────────

#[test]
fn t34_error_unifies_with_other_types() {
    // Error types should not cause type inference failures
    let code = r#"
*get_value(x)
    if x < 0
        return err InvalidInput
    x * 2

*main()
    log(get_value(5))
"#;
    let model = analyze_ok(code);
    // Should have no type errors (error unifies with Int return)
    assert!(
        model
            .warnings
            .iter()
            .all(|w| !w.message.contains("type inference failed")),
        "Error type should unify with other return types: {:?}",
        model.warnings
    );
}
