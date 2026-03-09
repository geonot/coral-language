//! Tests for Coral's value-error model.
//!
//! These tests verify that error values propagate correctly through
//! operations and that error checking/handling works as expected.

use coralc::Compiler;
use coralc::semantic;
use coralc::parser;
use coralc::lexer;

#[test]
fn error_value_creation() {
    // Test that we can create an error value
    let code = r#"
*main()
    x is err NotFound
    log(x)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Error value creation should compile");
    // Should call coral_make_error
    assert!(ir.contains("@coral_make_error"), "Should call coral_make_error");
}

#[test]
fn error_propagation_through_addition() {
    // Test that errors propagate through arithmetic
    let code = r#"
*main()
    x is err DivByZero
    y is x + 5
    log(y)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Error propagation should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value");
    assert!(ir.contains("@coral_value_add"), "Should use value_add for addition");
}

#[test]
fn error_propagation_through_string_concat() {
    // Test that errors propagate through string operations
    let code = r#"
*main()
    x is err InvalidInput
    y is x + ' suffix'
    log(y)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("String error propagation should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value");
}

#[test]
fn error_propagation_through_comparison() {
    // Test that errors propagate through comparisons
    let code = r#"
*main()
    x is err NotFound
    y is x.equals(5)
    log(y)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Comparison error propagation should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value");
    assert!(ir.contains("@coral_value_equals"), "Should use value_equals for comparison");
}

#[test]
fn error_propagation_through_bitwise() {
    // Test that errors propagate through bitwise operations
    let code = r#"
*main()
    x is err Overflow
    y is x & 255
    log(y)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Bitwise error propagation should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value");
    assert!(ir.contains("@coral_value_bitand"), "Should use value_bitand for bitwise AND");
}

#[test]
fn hierarchical_error_name() {
    // Test hierarchical error names
    let code = r#"
*main()
    x is err Database:Connection:Timeout
    log(x)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Hierarchical error names should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value");
    // The error name should be stored as a global constant
    assert!(ir.contains("Database:Connection:Timeout") || ir.contains("Database_Connection_Timeout"),
        "Should contain hierarchical error name");
}

#[test]
fn multiple_error_propagation() {
    // Test that first error is propagated (compile-time check only)
    let code = r#"
*main()
    a is err First
    b is err Second
    c is a + b
    log(c)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Multiple error propagation should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error values");
}

// ========== ERROR PROPAGATION SYNTAX TESTS ==========

#[test]
fn error_propagation_syntax_parses() {
    // Test that `! return err` syntax parses correctly
    let code = r#"
*might_fail()
    err NotFound

*use_result()
    x is might_fail() ! return err
    x + 1
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Error propagation syntax should parse");
    assert!(ir.contains("@coral_nb_is_err"), "Should call coral_nb_is_err to check for errors");
}

#[test]
fn error_propagation_returns_early_on_error() {
    // Test that error propagation generates early return logic
    let code = r#"
*might_fail()
    err Database:NotFound

*process()
    result is might_fail() ! return err
    result + 10
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Error propagation should compile");
    // Should have error checking and conditional return
    assert!(ir.contains("@coral_nb_is_err"), "Should check if value is error");
    // Should have conditional branch for error case
    assert!(ir.contains("br i1"), "Should have conditional branch for error handling");
}

#[test]
fn error_propagation_continues_on_success() {
    // Test that non-error values continue normally
    let code = r#"
*succeed()
    42

*use_success()
    x is succeed() ! return err
    x * 2
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Success case should compile");
    assert!(ir.contains("@coral_nb_is_err"), "Should still check for errors");
}

#[test]
fn error_propagation_chained() {
    // Test chaining multiple error propagation
    let code = r#"
*step1()
    42

*step2(x)
    x + 10

*pipeline()
    a is step1() ! return err
    b is step2(a) ! return err
    b * 2
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Chained propagation should compile");
    // Should have multiple coral_is_err calls
    let is_err_count = ir.matches("@coral_nb_is_err").count();
    assert!(is_err_count >= 2, "Should have multiple error checks, found {}", is_err_count);
}

#[test]
fn error_propagation_in_expression() {
    // Test error propagation as part of larger expression
    let code = r#"
*get_value()
    100

*compute()
    x is get_value() ! return err
    y is x + 50
    y
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Expression propagation should compile");
    assert!(ir.contains("@coral_nb_is_err"), "Should check for errors");
}

#[test]
fn error_propagation_does_not_conflict_with_ternary() {
    // Test that `!` in ternary expressions still works
    let code = r#"
*main()
    x is 5
    result is x > 0 ? x ! 0
    log(result)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Ternary should still work");
    // Should NOT have error propagation calls, just ternary
    // Note: declaration of coral_is_err may exist, but no calls should be made
    assert!(!ir.contains("call i8 @coral_nb_is_err"), "Ternary should not trigger error propagation calls");
}

#[test]
fn error_propagation_with_function_call() {
    // Test error propagation with direct function call
    let code = r#"
*fetch_data(id)
    id > 0 ? id ! err InvalidId

*process_data()
    data is fetch_data(42) ! return err
    data * 2
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Function call propagation should compile");
    assert!(ir.contains("@coral_nb_is_err"), "Should check function result for errors");
}

// ========== .err PROPERTY TESTS ==========

#[test]
fn err_property_on_error_value() {
    // Test that .err property access compiles and checks for errors
    let code = r#"
*main()
    x is err NotFound
    is_error is x.err
    log(is_error)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect(".err property should compile");
    assert!(ir.contains("@coral_is_err"), "Should call coral_is_err for .err property");
}

#[test]
fn err_property_on_normal_value() {
    // Test that .err property works on non-error values
    let code = r#"
*main()
    x is 42
    x.err ? log("error") ! log("ok")
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect(".err property on normal value should compile");
    assert!(ir.contains("@coral_is_err"), "Should call coral_is_err for .err property");
}

// ========== GUARD CLAUSE SYNTAX TESTS ==========

#[test]
fn guard_clause_basic() {
    // Test the guard clause syntax: cond ! err Name
    let code = r#"
*validate(x)
    x > 0 ! err InvalidInput
    x
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Guard clause should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value for guard clause");
}

#[test]
fn guard_clause_hierarchical() {
    // Test guard clause with hierarchical error name
    let code = r#"
*check_bounds(x, min, max)
    x >= min ! err Validation:OutOfBounds:TooLow
    x <= max ! err Validation:OutOfBounds:TooHigh
    x
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Hierarchical guard clause should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error values for guard clauses");
}

// ========== ERROR DEFINITION TESTS ==========

#[test]
fn parse_simple_error_definition() {
    // Test basic error definition without body
    let code = r#"
err NotFound

*main()
    log("hello")
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Simple error definition should compile");
    assert!(ir.contains("main"), "Should have main function");
}

#[test]
fn parse_error_definition_with_children() {
    // Test nested error definitions
    let code = r#"
err Database
    err Connection
        err Timeout
        err Refused
    err Query
        err Syntax

*main()
    log("hello")
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Nested error definition should compile");
    assert!(ir.contains("main"), "Should have main function");
}

#[test]
fn parse_error_definition_with_code_and_message() {
    // Test error definition with code and message attributes
    let code = r#"
err NotFound
    code is 404
    message is 'Resource not found'

*main()
    log("hello")
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Error definition with attributes should compile");
    assert!(ir.contains("main"), "Should have main function");
}

#[test]
fn parse_full_error_hierarchy() {
    // Test comprehensive error hierarchy like in the spec
    let code = r#"
err Database
    err Connection
        err Timeout
            code is 5001
            message is 'Connection timed out'
        err Refused
            code is 5002
            message is 'Connection refused'
    err Query
        err Syntax
            code is 4001
            message is 'Invalid SQL syntax'

*main()
    e is err Database:Connection:Timeout
    log(e)
"#;
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(code).expect("Full error hierarchy should compile");
    assert!(ir.contains("@coral_make_error"), "Should create error value");
}

// ========== UNHANDLED ERROR WARNING TESTS ==========

#[test]
fn warning_for_standalone_error_value() {
    // Test that a standalone error value expression produces a warning
    let code = r#"
*main()
    err NotFound
    log("done")
"#;
    let tokens = lexer::lex(code).expect("Should lex");
    let p = parser::Parser::new(tokens, code.len());
    let ast = p.parse().expect("Should parse");
    let model = semantic::analyze(ast).expect("Should analyze");
    
    // Should have a warning about the unhandled error value
    assert!(!model.warnings.is_empty(), "Should have warning for standalone error value");
    assert!(model.warnings[0].message.contains("NotFound"), 
        "Warning should mention the error name");
}

#[test]
fn no_warning_for_returned_error() {
    // Test that returning an error doesn't produce a warning
    let code = r#"
*validate(x)
    x > 0 ? x ! err InvalidInput
"#;
    let tokens = lexer::lex(code).expect("Should lex");
    let p = parser::Parser::new(tokens, code.len());
    let ast = p.parse().expect("Should parse");
    let model = semantic::analyze(ast).expect("Should analyze");
    
    // Should not have warnings - the error is returned via ternary
    assert!(model.warnings.is_empty(), "Should not warn when error is returned: {:?}", model.warnings);
}

#[test]
fn no_warning_for_bound_error() {
    // Test that binding an error to a variable doesn't produce a warning
    // (the programmer may handle it later)
    let code = r#"
*main()
    x is err NotFound
    x ? log("has error") ! log("ok")
"#;
    let tokens = lexer::lex(code).expect("Should lex");
    let p = parser::Parser::new(tokens, code.len());
    let ast = p.parse().expect("Should parse");
    let model = semantic::analyze(ast).expect("Should analyze");
    
    // Should not have warnings - the error is bound and then checked
    assert!(model.warnings.is_empty(), "Should not warn when error is bound and used: {:?}", model.warnings);
}
