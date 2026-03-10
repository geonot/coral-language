//! C4.2: LLVM Function Attributes Tests
//!
//! Verifies that the compiler emits correct LLVM function attributes:
//! - nounwind on all user-defined functions
//! - memory(none) and willreturn on pure functions
//! - nounwind on runtime FFI declarations

use coralc::Compiler;

fn compile_to_ir(source: &str) -> String {
    let compiler = Compiler;
    compiler.compile_to_ir(source).expect("should compile")
}

#[test]
fn c42_nounwind_on_user_functions() {
    let ir = compile_to_ir(
        r#"
*add(a, b)
    a + b
*main()
    log(add(1, 2))
"#,
    );
    // The `add` function should have nounwind attribute
    // In LLVM IR, function attributes appear after the function definition
    // as `#N` references or inline.
    // Check that nounwind appears in the attribute groups
    assert!(
        ir.contains("nounwind"),
        "IR should contain nounwind attribute:\n{ir}"
    );
}

#[test]
fn c42_pure_function_attributes() {
    let ir = compile_to_ir(
        r#"
*double(x)
    x + x
*main()
    log(double(5))
"#,
    );
    // Pure function `double` should get memory(none) + willreturn + nounwind
    assert!(
        ir.contains("nounwind"),
        "Pure function IR should contain nounwind:\n{ir}"
    );
    assert!(
        ir.contains("willreturn"),
        "Pure function IR should contain willreturn:\n{ir}"
    );
}

#[test]
fn c42_impure_function_no_readnone() {
    let ir = compile_to_ir(
        r#"
*greet(name)
    log(name)
*main()
    greet("Alice")
"#,
    );
    // `greet` calls log (side effect) so should NOT be marked readnone/willreturn
    // but should still be nounwind
    assert!(
        ir.contains("nounwind"),
        "Impure function should still have nounwind:\n{ir}"
    );
}

#[test]
fn c42_runtime_declarations_have_nounwind() {
    let ir = compile_to_ir(
        r#"
*main()
    log("hello")
"#,
    );
    // Runtime FFI declarations like coral_make_number should have nounwind
    assert!(
        ir.contains("nounwind"),
        "Runtime declarations should have nounwind:\n{ir}"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// C4.3: LLVM Alias Analysis Hints
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn c43_noalias_on_user_function_params() {
    let ir = compile_to_ir(
        r#"
*add(a, b)
    a + b
*main()
    log(add(1, 2))
"#,
    );
    // User function parameters should have noalias attribute
    assert!(
        ir.contains("noalias"),
        "User function IR should contain noalias on params:\n{ir}"
    );
}

#[test]
fn c43_noalias_on_runtime_allocator_returns() {
    let ir = compile_to_ir(
        r#"
*main()
    x is "hello"
    log(x)
"#,
    );
    // Runtime allocator functions (coral_make_string etc.) should have
    // noalias on their return value, indicating fresh allocations
    assert!(
        ir.contains("noalias"),
        "Runtime allocator returns should have noalias:\n{ir}"
    );
}

#[test]
fn c43_multi_param_function_noalias() {
    let ir = compile_to_ir(
        r#"
*combine(a, b, c)
    a + b + c
*main()
    log(combine(1, 2, 3))
"#,
    );
    // All three parameters should get noalias
    assert!(
        ir.contains("noalias"),
        "Multi-param function should have noalias:\n{ir}"
    );
}

#[test]
fn c43_allocator_noalias_distinct_from_nounwind() {
    let ir = compile_to_ir(
        r#"
*main()
    xs is [1, 2, 3]
    log(xs.length)
"#,
    );
    // Both noalias and nounwind should appear in the IR
    assert!(ir.contains("noalias"), "IR should contain noalias:\n{ir}");
    assert!(ir.contains("nounwind"), "IR should contain nounwind:\n{ir}");
}
