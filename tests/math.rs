//! Tests for math intrinsic functions.

use coralc::Compiler;

fn compile_and_verify(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    compiler
        .compile_to_ir(source)
        .map_err(|e| format!("{:?}", e))
}

// ==================== Unary Math Functions ====================

#[test]
fn test_abs() {
    let source = r#"
result is abs(-5)
"#;
    let ir = compile_and_verify(source).expect("abs should compile");
    assert!(
        ir.contains("coral_math_abs"),
        "IR should call coral_math_abs"
    );
}

#[test]
fn test_sqrt() {
    let source = r#"
result is sqrt(16)
"#;
    let ir = compile_and_verify(source).expect("sqrt should compile");
    assert!(
        ir.contains("coral_math_sqrt"),
        "IR should call coral_math_sqrt"
    );
}

#[test]
fn test_floor() {
    let source = r#"
result is floor(3.7)
"#;
    let ir = compile_and_verify(source).expect("floor should compile");
    assert!(
        ir.contains("coral_math_floor"),
        "IR should call coral_math_floor"
    );
}

#[test]
fn test_ceil() {
    let source = r#"
result is ceil(3.2)
"#;
    let ir = compile_and_verify(source).expect("ceil should compile");
    assert!(
        ir.contains("coral_math_ceil"),
        "IR should call coral_math_ceil"
    );
}

#[test]
fn test_round() {
    let source = r#"
result is round(3.5)
"#;
    let ir = compile_and_verify(source).expect("round should compile");
    assert!(
        ir.contains("coral_math_round"),
        "IR should call coral_math_round"
    );
}

#[test]
fn test_trunc() {
    let source = r#"
result is trunc(3.9)
"#;
    let ir = compile_and_verify(source).expect("trunc should compile");
    assert!(
        ir.contains("coral_math_trunc"),
        "IR should call coral_math_trunc"
    );
}

#[test]
fn test_sign() {
    let source = r#"
result is sign(42)
"#;
    let ir = compile_and_verify(source).expect("sign should compile");
    assert!(
        ir.contains("coral_math_sign"),
        "IR should call coral_math_sign"
    );
}

#[test]
fn test_signum_alias() {
    let source = r#"
result is signum(-10)
"#;
    let ir = compile_and_verify(source).expect("signum alias should compile");
    assert!(
        ir.contains("coral_math_sign"),
        "IR should call coral_math_sign"
    );
}

// ==================== Trigonometric Functions ====================

#[test]
fn test_sin() {
    let source = r#"
result is sin(0)
"#;
    let ir = compile_and_verify(source).expect("sin should compile");
    assert!(
        ir.contains("coral_math_sin"),
        "IR should call coral_math_sin"
    );
}

#[test]
fn test_cos() {
    let source = r#"
result is cos(0)
"#;
    let ir = compile_and_verify(source).expect("cos should compile");
    assert!(
        ir.contains("coral_math_cos"),
        "IR should call coral_math_cos"
    );
}

#[test]
fn test_tan() {
    let source = r#"
result is tan(0)
"#;
    let ir = compile_and_verify(source).expect("tan should compile");
    assert!(
        ir.contains("coral_math_tan"),
        "IR should call coral_math_tan"
    );
}

// ==================== Inverse Trigonometric Functions ====================

#[test]
fn test_asin() {
    let source = r#"
result is asin(0.5)
"#;
    let ir = compile_and_verify(source).expect("asin should compile");
    assert!(
        ir.contains("coral_math_asin"),
        "IR should call coral_math_asin"
    );
}

#[test]
fn test_acos() {
    let source = r#"
result is acos(0.5)
"#;
    let ir = compile_and_verify(source).expect("acos should compile");
    assert!(
        ir.contains("coral_math_acos"),
        "IR should call coral_math_acos"
    );
}

#[test]
fn test_atan() {
    let source = r#"
result is atan(1)
"#;
    let ir = compile_and_verify(source).expect("atan should compile");
    assert!(
        ir.contains("coral_math_atan"),
        "IR should call coral_math_atan"
    );
}

#[test]
fn test_atan2() {
    let source = r#"
result is atan2(1, 1)
"#;
    let ir = compile_and_verify(source).expect("atan2 should compile");
    assert!(
        ir.contains("coral_math_atan2"),
        "IR should call coral_math_atan2"
    );
}

// ==================== Hyperbolic Functions ====================

#[test]
fn test_sinh() {
    let source = r#"
result is sinh(0)
"#;
    let ir = compile_and_verify(source).expect("sinh should compile");
    assert!(
        ir.contains("coral_math_sinh"),
        "IR should call coral_math_sinh"
    );
}

#[test]
fn test_cosh() {
    let source = r#"
result is cosh(0)
"#;
    let ir = compile_and_verify(source).expect("cosh should compile");
    assert!(
        ir.contains("coral_math_cosh"),
        "IR should call coral_math_cosh"
    );
}

#[test]
fn test_tanh() {
    let source = r#"
result is tanh(0)
"#;
    let ir = compile_and_verify(source).expect("tanh should compile");
    assert!(
        ir.contains("coral_math_tanh"),
        "IR should call coral_math_tanh"
    );
}

// ==================== Exponential and Logarithm Functions ====================

#[test]
fn test_exp() {
    let source = r#"
result is exp(1)
"#;
    let ir = compile_and_verify(source).expect("exp should compile");
    assert!(
        ir.contains("coral_math_exp"),
        "IR should call coral_math_exp"
    );
}

#[test]
fn test_ln() {
    let source = r#"
result is ln(10)
"#;
    let ir = compile_and_verify(source).expect("ln should compile");
    assert!(ir.contains("coral_math_ln"), "IR should call coral_math_ln");
}

#[test]
fn test_log10() {
    let source = r#"
result is log10(100)
"#;
    let ir = compile_and_verify(source).expect("log10 should compile");
    assert!(
        ir.contains("coral_math_log10"),
        "IR should call coral_math_log10"
    );
}

// ==================== Binary Math Functions ====================

#[test]
fn test_pow() {
    let source = r#"
result is pow(2, 3)
"#;
    let ir = compile_and_verify(source).expect("pow should compile");
    assert!(
        ir.contains("coral_math_pow"),
        "IR should call coral_math_pow"
    );
}

#[test]
fn test_min() {
    let source = r#"
result is min(5, 3)
"#;
    let ir = compile_and_verify(source).expect("min should compile");
    assert!(
        ir.contains("coral_math_min"),
        "IR should call coral_math_min"
    );
}

#[test]
fn test_max() {
    let source = r#"
result is max(5, 3)
"#;
    let ir = compile_and_verify(source).expect("max should compile");
    assert!(
        ir.contains("coral_math_max"),
        "IR should call coral_math_max"
    );
}

// ==================== Chained Math Operations ====================

#[test]
fn test_chained_math() {
    let source = r#"
result is abs(floor(sqrt(17)))
"#;
    let ir = compile_and_verify(source).expect("chained math should compile");
    assert!(
        ir.contains("coral_math_sqrt"),
        "IR should call coral_math_sqrt"
    );
    assert!(
        ir.contains("coral_math_floor"),
        "IR should call coral_math_floor"
    );
    assert!(
        ir.contains("coral_math_abs"),
        "IR should call coral_math_abs"
    );
}

#[test]
fn test_math_in_expression() {
    let source = r#"
result is sqrt(16) + pow(2, 3)
"#;
    let ir = compile_and_verify(source).expect("math in expression should compile");
    assert!(
        ir.contains("coral_math_sqrt"),
        "IR should call coral_math_sqrt"
    );
    assert!(
        ir.contains("coral_math_pow"),
        "IR should call coral_math_pow"
    );
}

#[test]
fn test_math_in_function() {
    let source = r#"
*hypotenuse(a, b)
    sqrt(pow(a, 2) + pow(b, 2))

result is hypotenuse(3, 4)
"#;
    let ir = compile_and_verify(source).expect("math in function should compile");
    assert!(
        ir.contains("coral_math_sqrt"),
        "IR should call coral_math_sqrt"
    );
    assert!(
        ir.contains("coral_math_pow"),
        "IR should call coral_math_pow"
    );
}

#[test]
fn test_math_with_pipeline() {
    let source = r#"
result is 16 ~ sqrt ~ floor
"#;
    let ir = compile_and_verify(source).expect("math with pipeline should compile");
    assert!(
        ir.contains("coral_math_sqrt"),
        "IR should call coral_math_sqrt"
    );
    assert!(
        ir.contains("coral_math_floor"),
        "IR should call coral_math_floor"
    );
}

// ==================== Arity Error Tests ====================

#[test]
fn test_abs_wrong_arity() {
    let source = r#"
result is abs(1, 2)
"#;
    let result = compile_and_verify(source);
    assert!(result.is_err(), "abs with two args should error");
}

#[test]
fn test_pow_wrong_arity() {
    let source = r#"
result is pow(2)
"#;
    let result = compile_and_verify(source);
    assert!(result.is_err(), "pow with one arg should error");
}

#[test]
fn test_min_wrong_arity() {
    let source = r#"
result is min(1, 2, 3)
"#;
    let result = compile_and_verify(source);
    assert!(result.is_err(), "min with three args should error");
}
