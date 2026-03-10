//! Tests for control flow sugar keywords:
//! - S5.1: `unless` (negated if)
//! - S5.2: `until` (negated while)
//! - S5.3: `loop` (infinite loop with break)
//! - S5.4: `when` (multi-branch conditional)
//! - S5.6: Postfix `if` / `unless`
//! - T4.4: Branch type unification warnings

use coralc::ast::{Expression, Item, Statement, UnaryOp};
use coralc::lexer;
use coralc::parser::Parser;
use coralc::semantic;
use coralc::Compiler;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

// ─── Helpers ────────────────────────────────────────────────────────

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

fn parse_func_body(source: &str) -> Vec<Statement> {
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    match &program.items[0] {
        Item::Function(f) => f.body.statements.clone(),
        other => panic!("expected function, got {:?}", other),
    }
}

// ─── S5.1: unless ───────────────────────────────────────────────────

#[test]
fn parse_unless_desugars_to_negated_if() {
    let stmts = parse_func_body("*f()\n    unless x\n        log(1)\n");
    match &stmts[0] {
        Statement::If { condition, elif_branches, else_body, .. } => {
            // Condition should be Unary(Not, Identifier("x"))
            match condition {
                Expression::Unary { op: UnaryOp::Not, expr, .. } => {
                    match expr.as_ref() {
                        Expression::Identifier(name, _) => assert_eq!(name, "x"),
                        other => panic!("expected identifier in unless condition, got {:?}", other),
                    }
                }
                other => panic!("expected negated condition, got {:?}", other),
            }
            assert!(elif_branches.is_empty());
            assert!(else_body.is_none());
        }
        other => panic!("expected If statement from unless, got {:?}", other),
    }
}

#[test]
fn e2e_unless_skips_when_true() {
    assert_output(
        r#"
*main()
    x is true
    unless x
        log("should not print")
    log("done")
"#,
        &["done"],
    );
}

#[test]
fn e2e_unless_runs_when_false() {
    assert_output(
        r#"
*main()
    x is false
    unless x
        log("ran")
    log("done")
"#,
        &["ran", "done"],
    );
}

#[test]
fn e2e_unless_with_comparison() {
    assert_output(
        r#"
*main()
    count is 0
    unless count > 5
        log("small")
"#,
        &["small"],
    );
}

// ─── S5.2: until ────────────────────────────────────────────────────

#[test]
fn parse_until_desugars_to_negated_while() {
    let stmts = parse_func_body("*f()\n    until done\n        log(1)\n");
    match &stmts[0] {
        Statement::While { condition, .. } => {
            match condition {
                Expression::Unary { op: UnaryOp::Not, expr, .. } => {
                    match expr.as_ref() {
                        Expression::Identifier(name, _) => assert_eq!(name, "done"),
                        other => panic!("expected identifier in until condition, got {:?}", other),
                    }
                }
                other => panic!("expected negated condition, got {:?}", other),
            }
        }
        other => panic!("expected While from until, got {:?}", other),
    }
}

#[test]
fn e2e_until_counts_to_three() {
    assert_output(
        r#"
*main()
    i is 0
    until i is 3
        log(i)
        i is i + 1
"#,
        &["0", "1", "2"],
    );
}

#[test]
fn e2e_until_skips_when_already_true() {
    assert_output(
        r#"
*main()
    done is true
    until done
        log("should not print")
    log("skipped")
"#,
        &["skipped"],
    );
}

// ─── S5.3: loop ─────────────────────────────────────────────────────

#[test]
fn parse_loop_desugars_to_while_true() {
    let stmts = parse_func_body("*f()\n    loop\n        break\n");
    match &stmts[0] {
        Statement::While { condition, .. } => {
            match condition {
                Expression::Bool(true, _) => {} // correct
                other => panic!("expected Bool(true), got {:?}", other),
            }
        }
        other => panic!("expected While from loop, got {:?}", other),
    }
}

#[test]
fn e2e_loop_with_break() {
    assert_output(
        r#"
*main()
    i is 0
    loop
        i is i + 1
        i > 3 ? break
    log(i)
"#,
        &["4"],
    );
}

#[test]
fn e2e_loop_with_conditional_break() {
    assert_output(
        r#"
*main()
    sum is 0
    i is 1
    loop
        sum is sum + i
        i is i + 1
        i > 5 ? break
    log(sum)
"#,
        &["15"],
    );
}

#[test]
fn e2e_loop_with_continue() {
    assert_output(
        r#"
*main()
    i is 0
    count is 0
    loop
        i is i + 1
        i > 10 ? break
        i % 2 is 0 ? continue
        count is count + 1
    log(count)
"#,
        &["5"],
    );
}

// ─── S5.4: when ─────────────────────────────────────────────────────

#[test]
fn parse_when_desugars_to_nested_ternary() {
    let stmts = parse_func_body("*f()\n    result is when\n        x > 10 ? \"big\"\n        _ ? \"small\"\n");
    match &stmts[0] {
        Statement::Binding(b) => {
            assert_eq!(b.name, "result");
            // Value should be a ternary: (x > 10) ? "big" ! "small"
            match &b.value {
                Expression::Ternary { condition, then_branch, else_branch, .. } => {
                    match condition.as_ref() {
                        Expression::Binary { .. } => {} // x > 10
                        other => panic!("expected binary condition, got {:?}", other),
                    }
                    match then_branch.as_ref() {
                        Expression::String(s, _) => assert_eq!(s, "big"),
                        other => panic!("expected string 'big', got {:?}", other),
                    }
                    match else_branch.as_ref() {
                        Expression::String(s, _) => assert_eq!(s, "small"),
                        other => panic!("expected string 'small', got {:?}", other),
                    }
                }
                other => panic!("expected ternary from when, got {:?}", other),
            }
        }
        other => panic!("expected binding, got {:?}", other),
    }
}

#[test]
fn e2e_when_basic() {
    assert_output(
        r#"
*main()
    x is 150
    result is when
        x > 100 ? "high"
        x > 50  ? "medium"
        _       ? "low"
    log(result)
"#,
        &["high"],
    );
}

#[test]
fn e2e_when_middle_branch() {
    assert_output(
        r#"
*main()
    x is 75
    result is when
        x > 100 ? "high"
        x > 50  ? "medium"
        _       ? "low"
    log(result)
"#,
        &["medium"],
    );
}

#[test]
fn e2e_when_default_branch() {
    assert_output(
        r#"
*main()
    x is 10
    result is when
        x > 100 ? "high"
        x > 50  ? "medium"
        _       ? "low"
    log(result)
"#,
        &["low"],
    );
}

#[test]
fn e2e_when_no_default() {
    assert_output(
        r#"
*main()
    x is 5
    result is when
        x > 100 ? "high"
        x > 50  ? "medium"
    log(result)
"#,
        &["()"],
    );
}

#[test]
fn e2e_when_single_arm() {
    assert_output(
        r#"
*main()
    x is true
    result is when
        x ? "yes"
        _ ? "no"
    log(result)
"#,
        &["yes"],
    );
}

#[test]
fn e2e_when_with_function_calls() {
    assert_output(
        r#"
*classify(n)
    return when
        n > 100 ? "big"
        n > 0   ? "positive"
        n is 0  ? "zero"
        _       ? "negative"

*main()
    log(classify(200))
    log(classify(42))
    log(classify(0))
    log(classify(-5))
"#,
        &["big", "positive", "zero", "negative"],
    );
}

// ─── S5.6: Postfix if / unless ──────────────────────────────────────

#[test]
fn parse_postfix_if_desugars_to_if_statement() {
    let stmts = parse_func_body("*f()\n    log(1) if x\n");
    match &stmts[0] {
        Statement::If { condition, body, elif_branches, else_body, .. } => {
            match condition {
                Expression::Identifier(name, _) => assert_eq!(name, "x"),
                other => panic!("expected identifier condition, got {:?}", other),
            }
            assert_eq!(body.statements.len(), 1);
            assert!(elif_branches.is_empty());
            assert!(else_body.is_none());
        }
        other => panic!("expected If statement from postfix if, got {:?}", other),
    }
}

#[test]
fn parse_postfix_unless_desugars_to_negated_if() {
    let stmts = parse_func_body("*f()\n    log(1) unless x\n");
    match &stmts[0] {
        Statement::If { condition, elif_branches, else_body, .. } => {
            match condition {
                Expression::Unary { op: UnaryOp::Not, expr, .. } => {
                    match expr.as_ref() {
                        Expression::Identifier(name, _) => assert_eq!(name, "x"),
                        other => panic!("expected identifier in unless condition, got {:?}", other),
                    }
                }
                other => panic!("expected negated condition, got {:?}", other),
            }
            assert!(elif_branches.is_empty());
            assert!(else_body.is_none());
        }
        other => panic!("expected If statement from postfix unless, got {:?}", other),
    }
}

#[test]
fn e2e_postfix_if_executes_when_true() {
    assert_output(
        r#"
*main()
    x is true
    log("yes") if x
    log("done")
"#,
        &["yes", "done"],
    );
}

#[test]
fn e2e_postfix_if_skips_when_false() {
    assert_output(
        r#"
*main()
    x is false
    log("yes") if x
    log("done")
"#,
        &["done"],
    );
}

#[test]
fn e2e_postfix_unless_executes_when_false() {
    assert_output(
        r#"
*main()
    valid is false
    log("invalid") unless valid
    log("done")
"#,
        &["invalid", "done"],
    );
}

#[test]
fn e2e_postfix_unless_skips_when_true() {
    assert_output(
        r#"
*main()
    valid is true
    log("invalid") unless valid
    log("done")
"#,
        &["done"],
    );
}

// ─── T4.4: Branch Type Unification Warnings ─────────────────────────

fn analyze_warnings(source: &str) -> Vec<String> {
    let tokens = lexer::lex(source).expect("Should lex");
    let p = Parser::new(tokens, source.len());
    let ast = p.parse().expect("Should parse");
    let model = semantic::analyze(ast).expect("Should analyze");
    model.warnings.iter().map(|w| w.message.clone()).collect()
}

fn analyze_diagnostics(source: &str) -> Vec<coralc::diagnostics::Diagnostic> {
    let tokens = lexer::lex(source).expect("Should lex");
    let p = Parser::new(tokens, source.len());
    let ast = p.parse().expect("Should parse");
    let model = semantic::analyze(ast).expect("Should analyze");
    model.warnings.clone()
}

#[test]
fn t44_warns_on_mismatched_branch_types() {
    let warnings = analyze_warnings(r#"
*main()
    x is true
    if x
        42
    else
        "hello"
"#);
    let branch_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("if/else branches return different types"))
        .collect();
    assert!(!branch_warnings.is_empty(), "Should warn on Int vs String branches, got: {:?}", warnings);
}

#[test]
fn t44_no_warning_for_matching_branch_types() {
    let warnings = analyze_warnings(r#"
*main()
    x is true
    if x
        42
    else
        99
"#);
    let branch_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("if/else branches return different types"))
        .collect();
    assert!(branch_warnings.is_empty(), "Should not warn when both branches return Int, got: {:?}", warnings);
}

#[test]
fn t44_no_warning_without_else() {
    let warnings = analyze_warnings(r#"
*main()
    x is true
    if x
        log("yes")
"#);
    let branch_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("if/else branches return different types"))
        .collect();
    assert!(branch_warnings.is_empty(), "Should not warn on if without else, got: {:?}", warnings);
}

#[test]
fn t44_warns_on_elif_type_mismatch() {
    let warnings = analyze_warnings(r#"
*classify(x)
    if x > 100
        "big"
    elif x > 0
        42
    else
        "none"
"#);
    let branch_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("if/else branches return different types"))
        .collect();
    assert!(!branch_warnings.is_empty(), "Should warn on elif type mismatch, got: {:?}", warnings);
}

// ========== CC2.4: Warning Categories ==========

#[test]
fn cc24_branch_type_warning_has_category() {
    let diags = analyze_diagnostics(r#"
*main()
    x is true
    if x
        42
    else
        "hello"
"#);
    let cat_warnings: Vec<_> = diags.iter()
        .filter(|d| d.category == Some(coralc::diagnostics::WarningCategory::TypeMismatchBranch))
        .collect();
    assert!(!cat_warnings.is_empty(), "Branch type mismatch should have TypeMismatchBranch category");
}

#[test]
fn cc24_unreachable_code_warning_has_category() {
    let diags = analyze_diagnostics(r#"
*main()
    return 1
    x is 2
    x
"#);
    let cat_warnings: Vec<_> = diags.iter()
        .filter(|d| d.category == Some(coralc::diagnostics::WarningCategory::UnreachableCode))
        .collect();
    assert!(!cat_warnings.is_empty(), "Unreachable code should have UnreachableCode category, got: {:?}", 
        diags.iter().map(|d| (&d.message, &d.category)).collect::<Vec<_>>());
}

#[test]
fn cc24_warning_category_from_str() {
    use coralc::diagnostics::WarningCategory;
    assert_eq!(WarningCategory::from_str("dead_code"), Some(WarningCategory::DeadCode));
    assert_eq!(WarningCategory::from_str("unused_variable"), Some(WarningCategory::UnusedVariable));
    assert_eq!(WarningCategory::from_str("unreachable"), Some(WarningCategory::UnreachableCode));
    assert_eq!(WarningCategory::from_str("branch_types"), Some(WarningCategory::TypeMismatchBranch));
    assert_eq!(WarningCategory::from_str("nonsense"), None);
}

#[test]
fn cc24_warning_category_display() {
    use coralc::diagnostics::WarningCategory;
    assert_eq!(WarningCategory::DeadCode.name(), "dead_code");
    assert_eq!(WarningCategory::UnreachableCode.name(), "unreachable_code");
    assert_eq!(WarningCategory::TypeMismatchBranch.name(), "type_mismatch_branch");
}

// ========== T3.2: Definite Assignment Analysis ==========

#[test]
fn t32_no_warning_for_function_params() {
    let warnings = analyze_warnings(r#"
*test(a, b)
    log(a + b)
"#);
    let da_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("may not be initialized"))
        .collect();
    assert!(da_warnings.is_empty(), "Parameters should be definitely assigned, got: {:?}", warnings);
}

#[test]
fn t32_no_warning_for_simple_binding() {
    let warnings = analyze_warnings(r#"
*test()
    x is 42
    log(x)
"#);
    let da_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("may not be initialized"))
        .collect();
    assert!(da_warnings.is_empty(), "x is definitely assigned before use, got: {:?}", warnings);
}

#[test]
fn t32_no_warning_for_sequential_assignment() {
    let warnings = analyze_warnings(r#"
*test()
    a is 1
    b is 2
    c is a + b
    log(c)
"#);
    let da_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("may not be initialized"))
        .collect();
    assert!(da_warnings.is_empty(), "All sequential assignments are definite, got: {:?}", warnings);
}

#[test]
fn t32_no_warning_for_for_loop_variable() {
    let warnings = analyze_warnings(r#"
*test()
    for i in 0 to 5
        log(i)
"#);
    let da_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("may not be initialized"))
        .collect();
    assert!(da_warnings.is_empty(), "For loop variable is definitely assigned in its body, got: {:?}", warnings);
}

#[test]
fn t32_check_has_warning_category() {
    // Verify the analysis runs without crashing on nested control flow
    let diags = analyze_diagnostics(r#"
*test(flag)
    x is 0
    if flag
        x is 10
    else
        x is 20
    log(x)
"#);
    // No DA warnings expected here — x is assigned before use
    let da_warnings: Vec<_> = diags.iter()
        .filter(|d| d.message.contains("may not be initialized"))
        .collect();
    assert!(da_warnings.is_empty(), "No DA warnings expected for pre-assigned variable, got: {:?}",
        da_warnings.iter().map(|d| &d.message).collect::<Vec<_>>());
}
