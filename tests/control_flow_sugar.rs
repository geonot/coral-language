//! Tests for control flow sugar keywords:
//! - S5.1: `unless` (negated if)
//! - S5.2: `until` (negated while)
//! - S5.3: `loop` (infinite loop with break)
//! - S5.4: `when` (multi-branch conditional)

use coralc::ast::{Expression, Item, Statement, UnaryOp};
use coralc::lexer;
use coralc::parser::Parser;
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
