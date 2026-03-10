use coralc::compiler::Compiler;

/// Helper: compile source and return warnings
fn warnings_for(source: &str) -> Vec<String> {
    let compiler = Compiler;
    match compiler.compile_to_ir_with_warnings(source) {
        Ok((_ir, warnings)) => warnings,
        Err(e) => panic!("compilation failed: {}", e.diagnostic.message),
    }
}

/// Helper: check that source produces a warning containing the given substring
fn assert_warning(source: &str, expected_substring: &str) {
    let warnings = warnings_for(source);
    assert!(
        warnings.iter().any(|w| w.contains(expected_substring)),
        "expected a warning containing {:?}, got: {:?}",
        expected_substring,
        warnings
    );
}

/// Helper: check that source produces NO warning containing the given substring
fn assert_no_warning(source: &str, unexpected_substring: &str) {
    let warnings = warnings_for(source);
    assert!(
        !warnings.iter().any(|w| w.contains(unexpected_substring)),
        "did NOT expect a warning containing {:?}, but got: {:?}",
        unexpected_substring,
        warnings
    );
}

// ── Return dead code ──────────────────────────────────────────────

#[test]
fn dead_code_after_return() {
    let source = r#"
*main()
    return 1
    x is 2
    x
"#;
    assert_warning(source, "unreachable code after return");
}

#[test]
fn no_dead_code_without_return() {
    let source = r#"
*main()
    x is 1
    y is 2
    x + y
"#;
    assert_no_warning(source, "unreachable");
}

// ── Break dead code ──────────────────────────────────────────────

#[test]
fn dead_code_after_break() {
    let source = r#"
*main()
    i is 0
    while i < 10
        break
        i is i + 1
    i
"#;
    assert_warning(source, "unreachable code after break");
}

// ── Continue dead code ──────────────────────────────────────────

#[test]
fn dead_code_after_continue() {
    let source = r#"
*main()
    i is 0
    while i < 10
        continue
        i is i + 1
    i
"#;
    assert_warning(source, "unreachable code after continue");
}

// ── Nested blocks ────────────────────────────────────────────────

#[test]
fn dead_code_in_if_body() {
    let source = r#"
*main()
    if true
        return 1
        x is 2
    0
"#;
    assert_warning(source, "unreachable code after return");
}

#[test]
fn dead_code_in_else_body() {
    let source = r#"
*main()
    if false
        1
    else
        return 2
        x is 3
    0
"#;
    assert_warning(source, "unreachable code after return");
}

// ── No false positives ──────────────────────────────────────────

#[test]
fn conditional_return_not_dead() {
    // Return inside if branch doesn't make the code after the if unreachable
    let source = r#"
*main()
    if true
        return 1
    2
"#;
    assert_no_warning(source, "unreachable");
}

#[test]
fn return_at_end_not_dead() {
    // Return as the last statement is fine
    let source = r#"
*main()
    x is 1
    return x
"#;
    assert_no_warning(source, "unreachable");
}

#[test]
fn break_at_end_of_loop_not_dead() {
    let source = r#"
*main()
    i is 0
    while true
        i is i + 1
        break
    i
"#;
    assert_no_warning(source, "unreachable");
}
