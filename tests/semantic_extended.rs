//! Extended semantic analysis tests for Phase B.
//!
//! Tests scope checking, builtin recognition, error detection,
//! and the new builtins added in Phase B.

use coralc::Compiler;

/// Helper: attempts to compile — returns Ok(ir) or Err(message).
fn try_compile(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    compiler
        .compile_to_ir(source)
        .map_err(|e| format!("{:?}", e))
}

fn compiles_ok(source: &str) {
    try_compile(source).unwrap_or_else(|e| panic!("Expected compilation to succeed: {}", e));
}

fn compile_fails(source: &str) -> String {
    try_compile(source).expect_err("Expected compilation to fail")
}

// ─── Builtin Recognition ────────────────────────────────────────────

#[test]
fn semantic_recognizes_json_parse() {
    compiles_ok("*main()\n    x is json_parse(\"42\")\n    log(x)\n");
}

#[test]
fn semantic_recognizes_json_serialize() {
    compiles_ok("*main()\n    x is json_serialize(42)\n    log(x)\n");
}

#[test]
fn semantic_recognizes_time_now() {
    compiles_ok("*main()\n    t is time_now()\n    log(t)\n");
}

#[test]
fn semantic_recognizes_time_timestamp() {
    compiles_ok("*main()\n    t is time_timestamp()\n    log(t)\n");
}

#[test]
fn semantic_recognizes_time_format_iso() {
    compiles_ok("*main()\n    s is time_format_iso(0)\n    log(s)\n");
}

#[test]
fn semantic_recognizes_time_components() {
    compiles_ok(
        "*main()\n    log(time_year(0))\n    log(time_month(0))\n    log(time_day(0))\n    log(time_hour(0))\n    log(time_minute(0))\n    log(time_second(0))\n",
    );
}

#[test]
fn semantic_recognizes_base64() {
    compiles_ok(
        "*main()\n    x is base64_encode(\"hello\")\n    y is base64_decode(x)\n    log(y)\n",
    );
}

#[test]
fn semantic_recognizes_hex() {
    compiles_ok("*main()\n    x is hex_encode(\"ab\")\n    y is hex_decode(x)\n    log(y)\n");
}

#[test]
fn semantic_recognizes_list_ops() {
    compiles_ok(
        "*main()\n    lst is [1, 2, 3]\n    log(list_contains(lst, 1))\n    log(list_index_of(lst, 2))\n    log(list_reverse(lst))\n    log(list_sort(lst))\n",
    );
}

#[test]
fn semantic_recognizes_map_ops() {
    compiles_ok(
        "*main()\n    m is map(\"a\" is 1)\n    log(map_keys(m))\n    log(map_values(m))\n    log(has_key(m, \"a\"))\n",
    );
}

#[test]
fn semantic_recognizes_string_ops() {
    compiles_ok(
        "*main()\n    log(trim(\"  hi  \"))\n    log(to_upper(\"hi\"))\n    log(to_lower(\"HI\"))\n    log(starts_with(\"abc\", \"a\"))\n    log(ends_with(\"abc\", \"c\"))\n",
    );
}

#[test]
fn semantic_recognizes_type_of() {
    compiles_ok("*main()\n    log(type_of(42))\n");
}

#[test]
fn semantic_recognizes_error_builtins() {
    compiles_ok(
        "*main()\n    e is err NotFound\n    log(is_err(e))\n    log(is_ok(e))\n    log(error_name(e))\n",
    );
}

#[test]
fn semantic_recognizes_bytes_ops() {
    compiles_ok(
        "*main()\n    b is bytes_from_string(\"hi\")\n    s is bytes_to_string(b)\n    log(s)\n",
    );
}

#[test]
fn semantic_recognizes_char_ops() {
    compiles_ok("*main()\n    log(ord(\"A\"))\n    log(chr(65))\n");
}

#[test]
fn semantic_recognizes_process_ops() {
    compiles_ok("*main()\n    args is process_args()\n    log(args)\n");
}

#[test]
fn semantic_recognizes_env_ops() {
    compiles_ok("*main()\n    env_set(\"X\", \"Y\")\n    v is env_get(\"X\")\n    log(v)\n");
}

#[test]
fn semantic_recognizes_sort_natural() {
    compiles_ok("*main()\n    lst is [3, 1, 2]\n    s is sort_natural(lst)\n    log(s)\n");
}

#[test]
fn semantic_recognizes_string_lines() {
    compiles_ok("*main()\n    lines is string_lines(\"a\\nb\")\n    log(lines)\n");
}

// ─── Undefined Name Detection ───────────────────────────────────────

#[test]
fn semantic_rejects_undefined_variable() {
    let err = compile_fails("*main()\n    log(undefined_thing)\n");
    assert!(
        err.contains("undefined") || err.contains("unknown") || err.contains("Semantic"),
        "Expected undefined name error, got: {}",
        err
    );
}

#[test]
fn semantic_rejects_undefined_function_call() {
    let err = compile_fails("*main()\n    result is nonexistent_func(42)\n    log(result)\n");
    assert!(
        err.contains("undefined") || err.contains("unknown") || err.contains("Semantic"),
        "Expected undefined name error, got: {}",
        err
    );
}

// ─── Store Compilation ──────────────────────────────────────────────

#[test]
fn semantic_accepts_store_definition() {
    compiles_ok(
        "store Point\n    x ? 0\n    y ? 0\n\n*main()\n    p is make_Point()\n    log(p.x)\n",
    );
}

#[test]
fn semantic_accepts_store_with_method() {
    compiles_ok(
        "store Counter\n    count ? 0\n\n    *inc()\n        self.count is self.count + 1\n\n*main()\n    c is make_Counter()\n    c.inc()\n    log(c.count)\n",
    );
}

// ─── Trait Compilation ──────────────────────────────────────────────

#[test]
fn semantic_accepts_trait_definition() {
    compiles_ok("trait Printable\n    *to_string(self)\n\n*main()\n    log(\"ok\")\n");
}

// ─── Type/Enum Compilation ──────────────────────────────────────────

#[test]
fn semantic_accepts_type_definition() {
    compiles_ok("enum Color\n    Red\n    Green\n    Blue\n\n*main()\n    c is Red\n    log(c)\n");
}

// ─── Error Definition Compilation ───────────────────────────────────

#[test]
fn semantic_accepts_error_definition() {
    compiles_ok(
        "err NotFound\n    code is 404\n    message is \"not found\"\n\n*main()\n    e is err NotFound\n    log(is_err(e))\n",
    );
}

// ─── Complex Programs ───────────────────────────────────────────────

#[test]
fn semantic_accepts_fibonacci_program() {
    compiles_ok("*fib(n)\n    n < 2 ? n ! fib(n - 1) + fib(n - 2)\n\n*main()\n    log(fib(10))\n");
}

#[test]
fn semantic_accepts_store_with_trait() {
    compiles_ok(
        "trait Describable\n    *describe()\n\nstore Dog with Describable\n    name ? \"Rex\"\n\n    *describe()\n        self.name\n\n*main()\n    d is make_Dog()\n    log(d.describe())\n",
    );
}

#[test]
fn semantic_accepts_match_expression() {
    compiles_ok(
        "*classify(n)\n    return match n\n        1 ? \"one\"\n        2 ? \"two\"\n        ! \"other\"\n\n*main()\n    log(classify(1))\n",
    );
}

#[test]
fn semantic_accepts_for_loop() {
    compiles_ok("*main()\n    for x in [1, 2, 3]\n        log(x)\n");
}

#[test]
fn semantic_accepts_while_loop() {
    compiles_ok("*main()\n    i is 0\n    while i < 5\n        log(i)\n        i is i + 1\n");
}

#[test]
fn semantic_accepts_pipeline() {
    compiles_ok("*double(x)\n    x * 2\n\n*main()\n    result is 5 ~ double\n    log(result)\n");
}

#[test]
fn semantic_accepts_ternary() {
    compiles_ok("*main()\n    x is 10\n    r is x > 5 ? \"big\" ! \"small\"\n    log(r)\n");
}

#[test]
fn semantic_accepts_lambda() {
    compiles_ok("*apply(f, x)\n    f(x)\n\n*main()\n    log(apply(*fn(n) n * 2, 5))\n");
}

#[test]
fn semantic_accepts_bitwise_ops() {
    compiles_ok(
        "*main()\n    log(bit_and(3, 5))\n    log(bit_or(3, 5))\n    log(bit_xor(3, 5))\n    log(bit_shl(1, 3))\n    log(bit_shr(8, 1))\n",
    );
}
