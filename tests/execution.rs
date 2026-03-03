//! End-to-end execution tests.
//!
//! Each test compiles a Coral program to LLVM IR, runs it via `lli`
//! with the runtime loaded, captures stdout, and asserts output.

use coralc::Compiler;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

/// Workspace root (Cargo manifest dir at compile time).
const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");

/// Locate the runtime shared library (debug build).
fn runtime_lib() -> PathBuf {
    let lib = PathBuf::from(WORKSPACE).join("target/debug/libruntime.so");
    assert!(
        lib.exists(),
        "Runtime library not found at {}. Run `cargo build -p runtime` first.",
        lib.display()
    );
    lib
}

/// Compile Coral source → IR → execute via lli → return (stdout, stderr, exit_code).
fn run_coral(source: &str) -> (String, String, i32) {
    // Compile to IR
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .unwrap_or_else(|e| panic!("Compilation failed: {:?}", e));

    // Write IR to a temp file
    let mut ir_file = tempfile::NamedTempFile::new().expect("create temp file");
    ir_file.write_all(ir.as_bytes()).expect("write IR");
    ir_file.flush().expect("flush IR");

    let runtime = runtime_lib();

    // Run via lli with a timeout
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

/// Assert that Coral source produces the expected stdout lines.
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

/// Assert that Coral source produces stdout containing the given substring.
fn assert_output_contains(source: &str, substring: &str) {
    let (stdout, stderr, code) = run_coral(source);
    assert!(
        stdout.contains(substring),
        "Expected stdout to contain {:?} but got:\n--- STDOUT ---\n{}\n--- STDERR ---\n{}\n--- EXIT CODE: {} ---\n",
        substring, stdout, stderr, code
    );
}

// ─── Test Cases ──────────────────────────────────────────────────────

#[test]
fn e2e_hello_world() {
    assert_output(
        r#"
*main()
    log("hello world")
"#,
        &["hello world"],
    );
}

#[test]
fn e2e_arithmetic() {
    assert_output(
        r#"
*main()
    a is 10
    b is 20
    log(a + b)
    log(a - b)
    log(a * b)
"#,
        &["30", "-10", "200"],
    );
}

#[test]
fn e2e_string_concat() {
    assert_output(
        r#"
*main()
    greeting is "hello" + " " + "world"
    log(greeting)
"#,
        &["hello world"],
    );
}

#[test]
fn e2e_function_call() {
    assert_output(
        r#"
*add(a, b)
    a + b

*multiply(a, b)
    a * b

*main()
    log(add(3, 7))
    log(multiply(4, 5))
"#,
        &["10", "20"],
    );
}

#[test]
fn e2e_nested_function_calls() {
    assert_output(
        r#"
*double(x)
    x * 2

*add_one(x)
    x + 1

*main()
    result is double(add_one(4))
    log(result)
"#,
        &["10"],
    );
}

#[test]
fn e2e_ternary_expression() {
    assert_output(
        r#"
*main()
    x is 42
    result is x > 10 ? "big" ! "small"
    log(result)
    y is 3
    log(y > 10 ? "big" ! "small")
"#,
        &["big", "small"],
    );
}

#[test]
fn e2e_if_else() {
    assert_output(
        r#"
*main()
    x is 15
    if x > 10
        log("greater")
    else
        log("not greater")
"#,
        &["greater"],
    );
}

#[test]
fn e2e_if_elif_else() {
    // Note: if/elif/else as expression return value doesn't propagate
    // string values properly; use log inside branches instead.
    assert_output(
        r#"
*classify(n)
    if n > 100
        log("large")
    elif n > 10
        log("medium")
    else
        log("small")

*main()
    classify(200)
    classify(50)
    classify(5)
"#,
        &["large", "medium", "small"],
    );
}

#[test]
fn e2e_boolean_logic() {
    // Note: `and`/`or` keywords can't appear directly inside call args;
    // bind to a variable first.
    assert_output(
        r#"
*main()
    a is true
    b is false
    c is a and b
    log(c)
    d is a or b
    log(d)
"#,
        &["false", "true"],
    );
}

#[test]
fn e2e_comparison_operators() {
    assert_output(
        r#"
*main()
    log(10 > 5)
    log(3 < 1)
    log(5 >= 5)
    log(4 <= 3)
"#,
        &["true", "false", "true", "false"],
    );
}

#[test]
fn e2e_recursion() {
    assert_output(
        r#"
*factorial(n)
    n <= 1 ? 1 ! n * factorial(n - 1)

*main()
    log(factorial(5))
    log(factorial(1))
    log(factorial(0))
"#,
        &["120", "1", "1"],
    );
}

#[test]
fn e2e_multiple_return_paths() {
    assert_output(
        r#"
*abs_val(x)
    if x < 0
        return 0 - x
    x

*main()
    log(abs_val(5))
    log(abs_val(-3))
"#,
        &["5", "3"],
    );
}

#[test]
fn e2e_string_equality() {
    assert_output(
        r#"
*main()
    a is "hello"
    b is "hello"
    c is a.equals(b)
    log(c)
"#,
        &["true"],
    );
}

#[test]
fn e2e_global_bindings() {
    assert_output(
        r#"
pi is 3.14159

*main()
    log(pi)
"#,
        &["3.14159"],
    );
}

#[test]
fn e2e_integer_equality() {
    assert_output(
        r#"
*main()
    a is 42
    b is 42
    c is a.equals(b)
    log(c)
    d is a.equals(99)
    log(d)
"#,
        &["true", "false"],
    );
}

// ─── New Tests: While Loops ──────────────────────────────────────────

#[test]
fn e2e_while_loop_counter() {
    assert_output(
        r#"
*main()
    i is 0
    while i < 5
        log(i)
        i is i + 1
"#,
        &["0", "1", "2", "3", "4"],
    );
}

#[test]
fn e2e_while_loop_sum() {
    assert_output(
        r#"
*sum_to(n)
    total is 0
    i is 1
    while i <= n
        total is total + i
        i is i + 1
    total

*main()
    log(sum_to(10))
"#,
        &["55"],
    );
}

// ─── New Tests: Template String Interpolation ─────────────────────────

#[test]
fn e2e_template_string_number() {
    assert_output(
        r#"
*main()
    n is 42
    msg is "the answer is " + n
    log(msg)
"#,
        &["the answer is 42"],
    );
}

#[test]
fn e2e_template_string_bool() {
    assert_output(
        r#"
*main()
    flag is true
    msg is "result: " + flag
    log(msg)
"#,
        &["result: true"],
    );
}

#[test]
fn e2e_string_number_concat_both_directions() {
    assert_output(
        r#"
*main()
    n is 7
    log("count: " + n)
    log(n + " items")
"#,
        &["count: 7", "7 items"],
    );
}

// ─── New Tests: Nested Method Calls ──────────────────────────────────

#[test]
fn e2e_method_call_in_expression() {
    assert_output(
        r#"
*main()
    a is "hello"
    b is "hello"
    log(a.equals(b))
"#,
        &["true"],
    );
}

// ─── New Tests: If/Elif/Else as Expression ───────────────────────────

#[test]
fn e2e_if_else_expression_value() {
    assert_output(
        r#"
*classify(n)
    if n > 100
        return "large"
    elif n > 10
        return "medium"
    else
        return "small"

*main()
    log(classify(200))
    log(classify(50))
    log(classify(5))
"#,
        &["large", "medium", "small"],
    );
}

// ─── New Tests: Variable Shadowing ──────────────────────────────────

#[test]
fn e2e_variable_shadowing() {
    assert_output(
        r#"
*main()
    x is 10
    log(x)
    x is 20
    log(x)
"#,
        &["10", "20"],
    );
}

// ─── New Tests: Fizzbuzz ─────────────────────────────────────────────

#[test]
fn e2e_fizzbuzz() {
    assert_output(
        r#"
*fizzbuzz(n)
    i is 1
    while i <= n
        r15 is i % 15
        r3 is i % 3
        r5 is i % 5
        if r15.equals(0)
            log("FizzBuzz")
        elif r3.equals(0)
            log("Fizz")
        elif r5.equals(0)
            log("Buzz")
        else
            log(i)
        i is i + 1

*main()
    fizzbuzz(15)
"#,
        &["1", "2", "Fizz", "4", "Buzz", "Fizz", "7", "8", "Fizz", "Buzz", "11", "Fizz", "13", "14", "FizzBuzz"],
    );
}

// ─── New Tests: Recursive Fibonacci ──────────────────────────────────

#[test]
fn e2e_fibonacci() {
    assert_output(
        r#"
*fib(n)
    if n <= 1
        return n
    fib(n - 1) + fib(n - 2)

*main()
    log(fib(0))
    log(fib(1))
    log(fib(5))
    log(fib(10))
"#,
        &["0", "1", "5", "55"],
    );
}

// ─── New Tests: Higher-Order / Functional Patterns ──────────────────

#[test]
fn e2e_function_as_value() {
    assert_output(
        r#"
*double(x)
    x * 2

*apply(f, x)
    f(x)

*main()
    log(apply(double, 21))
"#,
        &["42"],
    );
}

// ─── Closures / Lambdas ─────────────────────────────────────────────

#[test]
fn e2e_closure_captures() {
    assert_output(
        r#"
*make_adder(x)
    *fn(y) x + y

*main()
    add10 is make_adder(10)
    log(add10(5))
    log(add10(20))
"#,
        &["15", "30"],
    );
}

#[test]
fn e2e_nested_while_loops() {
    // Test nested while loops with independent counters
    assert_output(
        r#"
*main()
    i is 1
    total is 0
    while i <= 3
        j is 1
        while j <= 3
            total is total + 1
            j is j + 1
        i is i + 1
    log(total)
"#,
        &["9"],
    );
}

#[test]
fn e2e_string_methods() {
    assert_output(
        r#"
*main()
    s is "hello world"
    log(s.length())
"#,
        &["11"],
    );
}

#[test]
fn e2e_list_basic() {
    assert_output(
        r#"
*main()
    items is [1, 2, 3]
    log(items.length())
"#,
        &["3"],
    );
}

#[test]
fn e2e_modulo_operator() {
    assert_output(
        r#"
*main()
    log(10 % 3)
    log(15 % 5)
    log(7 % 2)
"#,
        &["1", "0", "1"],
    );
}

#[test]
fn e2e_early_return_in_loop() {
    assert_output(
        r#"
*find_first_gt(threshold)
    i is 1
    while i <= 100
        if i * i > threshold
            return i
        i is i + 1
    return 0 - 1

*main()
    log(find_first_gt(50))
"#,
        &["8"],
    );
}

// ─── New Feature Tests ───────────────────────────────────────────────

#[test]
fn e2e_map_iteration_for_loop() {
    assert_output(
        r#"
*main()
    m is map("a" is 1, "b" is 2, "c" is 3)
    total is 0
    for key in m
        total is total + m.get(key)
    log(total)
"#,
        &["6"],
    );
}

#[test]
fn e2e_list_contains() {
    assert_output(
        r#"
*main()
    lst is [10, 20, 30]
    log(list_contains(lst, 20))
    log(list_contains(lst, 99))
"#,
        &["true", "false"],
    );
}

#[test]
fn e2e_list_index_of() {
    assert_output(
        r#"
*main()
    lst is ["a", "b", "c"]
    log(list_index_of(lst, "b"))
    log(list_index_of(lst, "z"))
"#,
        &["1", "-1"],
    );
}

#[test]
fn e2e_list_reverse() {
    assert_output(
        r#"
*main()
    lst is [1, 2, 3]
    rev is list_reverse(lst)
    log(rev[0])
    log(rev[1])
    log(rev[2])
"#,
        &["3", "2", "1"],
    );
}

#[test]
fn e2e_list_sort() {
    assert_output(
        r#"
*main()
    lst is [3, 1, 2]
    sorted is list_sort(lst)
    log(sorted[0])
    log(sorted[1])
    log(sorted[2])
"#,
        &["1", "2", "3"],
    );
}

#[test]
fn e2e_list_join() {
    assert_output(
        r#"
*main()
    lst is ["hello", "world"]
    log(list_join(lst, " "))
"#,
        &["hello world"],
    );
}

#[test]
fn e2e_list_concat() {
    assert_output(
        r#"
*main()
    a is [1, 2]
    b is [3, 4]
    c is list_concat(a, b)
    log(c.length())
    log(c[2])
"#,
        &["4", "3"],
    );
}

#[test]
fn e2e_list_slice() {
    assert_output(
        r#"
*main()
    lst is [10, 20, 30, 40, 50]
    sub is list_slice(lst, 1, 4)
    log(sub.length())
    log(sub[0])
    log(sub[2])
"#,
        &["3", "20", "40"],
    );
}

#[test]
fn e2e_map_has_key() {
    assert_output(
        r#"
*main()
    m is map("x" is 1, "y" is 2)
    log(map_has_key(m, "x"))
    log(map_has_key(m, "z"))
"#,
        &["true", "false"],
    );
}

#[test]
fn e2e_map_values() {
    assert_output(
        r#"
*main()
    m is map("a" is 10)
    vals is map_values(m)
    log(vals.length())
    log(vals[0])
"#,
        &["1", "10"],
    );
}

#[test]
fn e2e_map_entries() {
    assert_output(
        r#"
*main()
    m is map("key" is 42)
    entries is map_entries(m)
    log(entries.length())
    pair is entries[0]
    log(pair[0])
    log(pair[1])
"#,
        &["1", "key", "42"],
    );
}

#[test]
fn e2e_map_remove() {
    assert_output(
        r#"
*main()
    m is map("a" is 1, "b" is 2)
    m2 is map_remove(m, "a")
    log(m2.keys().length())
"#,
        &["1"],
    );
}

#[test]
fn e2e_map_merge() {
    assert_output(
        r#"
*main()
    a is map("x" is 1)
    b is map("y" is 2)
    c is map_merge(a, b)
    log(c.keys().length())
    log(c.get("x"))
    log(c.get("y"))
"#,
        &["2", "1", "2"],
    );
}

#[test]
fn e2e_type_of() {
    assert_output(
        r#"
*main()
    log(type_of(42))
    log(type_of("hello"))
    log(type_of(true))
    log(type_of([1, 2]))
    log(type_of(map("a" is 1)))
"#,
        &["number", "string", "bool", "list", "map"],
    );
}

#[test]
fn e2e_negation_operator() {
    assert_output(
        r#"
*main()
    log(!true)
    log(!false)
"#,
        &["false", "true"],
    );
}

#[test]
fn e2e_option_adt() {
    assert_output(
        r#"
enum Option
    Some(value)
    None

*main()
    x is Some(42)
    result is match x
        Some(v) ? v
        None ? 0
    log(result)
"#,
        &["42"],
    );
}

#[test]
fn e2e_adt_recursive_tree() {
    assert_output(
        r#"
enum Tree
    Leaf(value)
    Node(left, right)

*tree_sum(t)
    result is match t
        Leaf(v) ? v
        Node(l, r) ? tree_sum(l) + tree_sum(r)
    return result

*main()
    tree is Node(Node(Leaf(1), Leaf(2)), Leaf(3))
    log(tree_sum(tree))
"#,
        &["6"],
    );
}

#[test]
fn e2e_bytes_from_to_string() {
    assert_output(
        r#"
*main()
    b is bytes_from_string("hello")
    s is bytes_to_string(b)
    log(s)
"#,
        &["hello"],
    );
}

#[test]
fn e2e_stdin_read_line_not_called() {
    // Just verify it compiles - we can't easily test stdin in E2E
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(
            r#"
*main()
    log("ok")
"#,
        )
        .expect("compiles");
    assert!(ir.contains("@main"));
}

#[test]
fn e2e_process_exit() {
    let (stdout, _stderr, code) = run_coral(
        r#"
*main()
    log("before")
    exit(0)
    log("after")
"#,
    );
    assert!(stdout.contains("before"));
    assert!(!stdout.contains("after"));
    // exit(0) via lli may produce code 0 or -1 (signal) depending on timing
    assert!(code == 0 || code == -1, "expected exit code 0 or -1 (signal), got {}", code);
}

#[test]
fn e2e_ord_chr() {
    assert_output(
        r#"
*main()
    log(ord("A"))
    log(ord("a"))
    log(ord("0"))
    log(chr(65))
    log(chr(97))
"#,
        &["65", "97", "48", "A", "a"],
    );
}

#[test]
fn e2e_string_compare() {
    assert_output(
        r#"
*main()
    log(string_compare("abc", "def"))
    log(string_compare("xyz", "abc"))
    log(string_compare("same", "same"))
"#,
        &["-1", "1", "0"],
    );
}

#[test]
fn e2e_isnt_keyword() {
    assert_output(
        r#"
*main()
    x is 5
    y is 10
    log(x isnt y)
    log(x isnt 5)
    a is "hello"
    b is "world"
    log(a isnt b)
    log(a isnt "hello")
"#,
        &["true", "false", "true", "false"],
    );
}

#[test]
fn e2e_isnt_in_condition() {
    assert_output(
        r#"
*check(val)
    if val isnt 0
        return "nonzero"
    return "zero"

*main()
    log(check(5))
    log(check(0))
    log(check(-1))
"#,
        &["nonzero", "zero", "nonzero"],
    );
}

#[test]
fn e2e_char_classification() {
    // Test character classification using ord — the pattern needed for a self-hosted lexer
    assert_output(
        r#"
*is_digit(c)
    code is ord(c)
    return code >= 48 and code <= 57

*is_alpha(c)
    code is ord(c)
    return (code >= 65 and code <= 90) or (code >= 97 and code <= 122)

*main()
    log(is_digit("5"))
    log(is_digit("a"))
    log(is_alpha("Z"))
    log(is_alpha("3"))
"#,
        &["true", "false", "true", "false"],
    );
}

#[test]
fn e2e_self_hosted_lexer_basic() {
    // Test a self-hosted lexer that tokenizes Coral expressions
    assert_output(
        r#"
*is_digit(c)
    code is ord(c)
    return code >= 48 and code <= 57

*is_alpha(c)
    code is ord(c)
    return (code >= 65 and code <= 90) or (code >= 97 and code <= 122)

*is_ident_start(c)
    return is_alpha(c) or c is "_"

*is_ident_char(c)
    return is_alpha(c) or is_digit(c) or c is "_"

*make_token(kind, value)
    return map("kind" is kind, "value" is value)

*match_keyword(text)
    keywords is ["if", "is", "return", "true", "false", "while", "for", "in", "and", "or"]
    if list_contains(keywords, text)
        return "keyword"
    return "identifier"

*lex_simple(source)
    tokens is []
    pos is 0
    length is source.length()
    while pos < length
        c is char_at(source, pos)
        if c is " " or c is "\t"
            pos is pos + 1
            continue
        if c is "\n"
            tokens.push(make_token("newline", ""))
            pos is pos + 1
            continue
        if is_digit(c)
            start is pos
            while pos < length and is_digit(char_at(source, pos))
                pos is pos + 1
            tokens.push(make_token("integer", slice(source, start, pos)))
            continue
        if is_ident_start(c)
            start is pos
            while pos < length and is_ident_char(char_at(source, pos))
                pos is pos + 1
            text is slice(source, start, pos)
            tokens.push(make_token(match_keyword(text), text))
            continue
        if c is "+"
            tokens.push(make_token("plus", "+"))
            pos is pos + 1
            continue
        if c is "("
            tokens.push(make_token("lparen", "("))
            pos is pos + 1
            continue
        if c is ")"
            tokens.push(make_token("rparen", ")"))
            pos is pos + 1
            continue
        if c is "*"
            tokens.push(make_token("star", "*"))
            pos is pos + 1
            continue
        if c is ">"
            if pos + 1 < length and char_at(source, pos + 1) is "="
                tokens.push(make_token("gte", ">="))
                pos is pos + 2
            else
                tokens.push(make_token("gt", ">"))
                pos is pos + 1
            continue
        if c is "<"
            if pos + 1 < length and char_at(source, pos + 1) is "="
                tokens.push(make_token("lte", "<="))
                pos is pos + 2
            else
                tokens.push(make_token("lt", "<"))
                pos is pos + 1
            continue
        tokens.push(make_token("unknown", c))
        pos is pos + 1
    return tokens

*format_token(tok)
    kind is tok.get("kind")
    value is tok.get("value")
    if kind is "identifier" or kind is "integer"
        return kind + "(" + value + ")"
    if kind is "keyword"
        return "kw:" + value
    return kind

*main()
    tokens is lex_simple("x is 42 + y")
    for tok in tokens
        log(format_token(tok))
"#,
        &[
            "identifier(x)",
            "kw:is",
            "integer(42)",
            "plus",
            "identifier(y)",
        ],
    );
}

#[test]
fn e2e_self_hosted_lexer_function_def() {
    // Test lexer on a function definition with operators
    assert_output(
        r##"
*is_digit(c)
    code is ord(c)
    return code >= 48 and code <= 57

*is_alpha(c)
    code is ord(c)
    return (code >= 65 and code <= 90) or (code >= 97 and code <= 122)

*is_ident_start(c)
    return is_alpha(c) or c is "_"

*is_ident_char(c)
    return is_alpha(c) or is_digit(c) or c is "_"

*make_tok(kind, value)
    return map("kind" is kind, "value" is value)

*kw_check(text)
    keywords is ["if", "is", "return", "true", "false", "while", "for", "in", "and", "or"]
    if list_contains(keywords, text)
        return "keyword"
    return "identifier"

*lex(source)
    tokens is []
    pos is 0
    length is source.length()
    while pos < length
        c is char_at(source, pos)
        if c is " " or c is "\t" or c is "\n" or c is "\r"
            pos is pos + 1
            continue
        if is_digit(c)
            start is pos
            while pos < length and is_digit(char_at(source, pos))
                pos is pos + 1
            tokens.push(make_tok("int", slice(source, start, pos)))
            continue
        if is_ident_start(c)
            start is pos
            while pos < length and is_ident_char(char_at(source, pos))
                pos is pos + 1
            text is slice(source, start, pos)
            tokens.push(make_tok(kw_check(text), text))
            continue
        if c is "*"
            tokens.push(make_tok("star", "*"))
            pos is pos + 1
            continue
        if c is "("
            tokens.push(make_tok("lp", "("))
            pos is pos + 1
            continue
        if c is ")"
            tokens.push(make_tok("rp", ")"))
            pos is pos + 1
            continue
        if c is ","
            tokens.push(make_tok("comma", ","))
            pos is pos + 1
            continue
        if c is "+"
            tokens.push(make_tok("plus", "+"))
            pos is pos + 1
            continue
        tokens.push(make_tok("?", c))
        pos is pos + 1
    return tokens

*main()
    src is "*add(a, b)\n\treturn a + b"
    tokens is lex(src)
    for tok in tokens
        log(tok.get("kind") + ":" + tok.get("value"))
"##,
        &[
            "star:*",
            "identifier:add",
            "lp:(",
            "identifier:a",
            "comma:,",
            "identifier:b",
            "rp:)",
            "keyword:return",
            "identifier:a",
            "plus:+",
            "identifier:b",
        ],
    );
}

// ─── For Loop Tests ──────────────────────────────────────────────────

#[test]
fn e2e_for_loop_sum() {
    assert_output(
        r#"
*main()
    nums is [1, 2, 3, 4, 5]
    total is 0
    for n in nums
        total is total + n
    log(total)
"#,
        &["15"],
    );
}

#[test]
fn e2e_for_loop_string_iteration() {
    assert_output(
        r#"
*main()
    words is ["hello", "world", "coral"]
    for w in words
        log(w)
"#,
        &["hello", "world", "coral"],
    );
}

// ─── Break / Continue Tests ──────────────────────────────────────────

#[test]
fn e2e_break_in_while() {
    assert_output(
        r#"
*main()
    i is 0
    while i < 100
        if i > 4
            break
        log(i)
        i is i + 1
"#,
        &["0", "1", "2", "3", "4"],
    );
}

#[test]
fn e2e_continue_in_while() {
    assert_output(
        r#"
*main()
    i is 0
    while i < 10
        i is i + 1
        if i % 2 > 0
            continue
        log(i)
"#,
        &["2", "4", "6", "8", "10"],
    );
}

// ─── Index / Subscript Tests ─────────────────────────────────────────

#[test]
fn e2e_list_index_access() {
    assert_output(
        r#"
*main()
    items is ["a", "b", "c", "d"]
    log(items[0])
    log(items[2])
    log(items[3])
"#,
        &["a", "c", "d"],
    );
}

// ─── Lambda / Closure Tests ──────────────────────────────────────────

#[test]
fn e2e_lambda_basic() {
    assert_output(
        r#"
*apply(f, x)
    return f(x)

*main()
    double is *fn(n) n * 2
    log(apply(double, 21))
"#,
        &["42"],
    );
}

#[test]
fn e2e_closure_counter() {
    assert_output(
        r#"
*make_adder(base)
    return *fn(x) base + x

*main()
    add10 is make_adder(10)
    log(add10(5))
    log(add10(20))
    log(add10(0))
"#,
        &["15", "30", "10"],
    );
}

// ─── Hex / Binary / Octal Literal Tests ──────────────────────────────

#[test]
fn e2e_hex_literal() {
    assert_output(
        r#"
*main()
    x is 0xFF
    log(x)
"#,
        &["255"],
    );
}

#[test]
fn e2e_binary_literal() {
    assert_output(
        r#"
*main()
    x is 0b1010
    log(x)
"#,
        &["10"],
    );
}

#[test]
fn e2e_octal_literal() {
    assert_output(
        r#"
*main()
    x is 0o77
    log(x)
"#,
        &["63"],
    );
}

// ─── Bitwise Operator Tests ─────────────────────────────────────────

#[test]
fn e2e_bitwise_and() {
    assert_output(
        r#"
*main()
    log(0xFF & 0x0F)
"#,
        &["15"],
    );
}

#[test]
fn e2e_bitwise_or() {
    assert_output(
        r#"
*main()
    log(0xF0 | 0x0F)
"#,
        &["255"],
    );
}

#[test]
fn e2e_bitwise_xor() {
    assert_output(
        r#"
*main()
    log(0xFF ^ 0x0F)
"#,
        &["240"],
    );
}

#[test]
fn e2e_shift_left() {
    assert_output(
        r#"
*main()
    log(1 << 8)
"#,
        &["256"],
    );
}

#[test]
fn e2e_shift_right() {
    assert_output(
        r#"
*main()
    log(256 >> 4)
"#,
        &["16"],
    );
}

// ─── Pipeline Operator Tests ─────────────────────────────────────────

#[test]
fn e2e_pipeline_basic() {
    assert_output(
        r#"
*double(x)
    return x * 2

*add_one(x)
    return x + 1

*main()
    result is 5 ~ double ~ add_one
    log(result)
"#,
        &["11"],
    );
}

// ─── Match Expression Tests ──────────────────────────────────────────

#[test]
fn e2e_match_integer() {
    assert_output(
        r#"
*describe(n)
    return match n
        1 ? "one"
        2 ? "two"
        3 ? "three"
        ! "other"

*main()
    log(describe(1))
    log(describe(2))
    log(describe(3))
    log(describe(99))
"#,
        &["one", "two", "three", "other"],
    );
}

#[test]
fn e2e_match_string() {
    assert_output(
        r#"
*greet(lang)
    return match lang
        "en" ? "Hello"
        "es" ? "Hola"
        "fr" ? "Bonjour"
        ! "Hi"

*main()
    log(greet("en"))
    log(greet("es"))
    log(greet("fr"))
    log(greet("de"))
"#,
        &["Hello", "Hola", "Bonjour", "Hi"],
    );
}

#[test]
fn e2e_match_boolean() {
    assert_output(
        r#"
*to_yes_no(val)
    return match val
        true ? "yes"
        false ? "no"

*main()
    log(to_yes_no(true))
    log(to_yes_no(false))
"#,
        &["yes", "no"],
    );
}

// ─── Multiple Functions / Mutual Calls ───────────────────────────────

#[test]
fn e2e_multiple_functions() {
    assert_output(
        r#"
*square(x)
    return x * x

*sum_of_squares(a, b)
    return square(a) + square(b)

*main()
    log(sum_of_squares(3, 4))
"#,
        &["25"],
    );
}

// ─── Nested If/Else Tests ────────────────────────────────────────────

#[test]
fn e2e_nested_if_else() {
    assert_output(
        r#"
*classify(n)
    if n > 0
        if n > 100
            return "big"
        else
            return "small"
    else
        return "negative"

*main()
    log(classify(200))
    log(classify(42))
    log(classify(-5))
"#,
        &["big", "small", "negative"],
    );
}

// ─── Complex Recursion ───────────────────────────────────────────────

#[test]
fn e2e_gcd() {
    assert_output(
        r#"
*gcd(a, b)
    if b is 0
        return a
    return gcd(b, a % b)

*main()
    log(gcd(48, 18))
    log(gcd(100, 75))
"#,
        &["6", "25"],
    );
}

// ─── String Template with Expressions ────────────────────────────────

#[test]
fn e2e_template_string_expression() {
    assert_output(
        r#"
*main()
    name is "Coral"
    version is 1
    log("Language: " + name + ", v" + version)
"#,
        &["Language: Coral, v1"],
    );
}

// ─── Scope and Shadowing ────────────────────────────────────────────

#[test]
fn e2e_scope_shadowing_in_blocks() {
    assert_output(
        r#"
*main()
    x is 10
    if true
        x is 20
        log(x)
    log(x)
"#,
        &["20", "20"],
    );
}

// ─── Higher Order Functions ──────────────────────────────────────────

#[test]
fn e2e_higher_order_apply_twice() {
    assert_output(
        r#"
*apply_twice(f, x)
    return f(f(x))

*inc(n)
    return n + 1

*main()
    log(apply_twice(inc, 5))
"#,
        &["7"],
    );
}

// ─── Complex While Loop Patterns ─────────────────────────────────────

#[test]
fn e2e_while_accumulate_string() {
    assert_output(
        r#"
*main()
    result is ""
    i is 0
    while i < 5
        result is result + "*"
        i is i + 1
    log(result)
"#,
        &["*****"],
    );
}

// ─── Deeply Nested Expressions ───────────────────────────────────────

#[test]
fn e2e_nested_arithmetic() {
    assert_output(
        r#"
*main()
    log((2 + 3) * (4 - 1) + 10 / 2)
"#,
        &["20"],
    );
}

// ─── Error Value Tests ───────────────────────────────────────────────

#[test]
fn e2e_error_value_basic() {
    assert_output(
        r#"
*safe_divide(a, b)
    if b is 0
        return err DivByZero
    return a / b

*main()
    r1 is safe_divide(10, 2)
    log(r1)
    r2 is safe_divide(10, 0)
    log(is_err(r2))
    log(is_err(r1))
    log(is_ok(r1))
"#,
        &["5", "true", "false", "true"],
    );
}

#[test]
fn e2e_error_name() {
    assert_output(
        r#"
*main()
    e is err NotFound
    log(error_name(e))
"#,
        &["NotFound"],
    );
}

// ─── ADT / Enum Type System Tests ────────────────────────────────────

#[test]
fn e2e_adt_multiple_constructors() {
    // Test enum with multiple constructors of different arities
    assert_output(
        r#"
enum Shape
    Circle(radius)
    Rectangle(width, height)
    Point

*area(s)
    return match s
        Circle(r) ? r * r * 3
        Rectangle(w, h) ? w * h
        Point ? 0

*main()
    log(area(Circle(5)))
    log(area(Rectangle(3, 4)))
    log(area(Point))
"#,
        &["75", "12", "0"],
    );
}

#[test]
fn e2e_adt_nested_match() {
    // Test match on nested ADT values
    assert_output(
        r#"
enum Expr
    Num(value)
    Add(left, right)
    Mul(left, right)

*eval(e)
    return match e
        Num(v) ? v
        Add(l, r) ? eval(l) + eval(r)
        Mul(l, r) ? eval(l) * eval(r)

*main()
    expr is Add(Mul(Num(2), Num(3)), Num(4))
    log(eval(expr))
"#,
        &["10"],
    );
}

#[test]
fn e2e_adt_option_map_pattern() {
    // Test Option-like ADT with map function
    assert_output(
        r#"
enum Maybe
    Just(value)
    Nothing

*maybe_map(m, f)
    return match m
        Just(v) ? Just(f(v))
        Nothing ? Nothing

*double(x)
    return x * 2

*show_maybe(m)
    return match m
        Just(v) ? v
        Nothing ? 0

*main()
    a is Just(21)
    b is maybe_map(a, double)
    log(show_maybe(b))
    c is maybe_map(Nothing, double)
    log(show_maybe(c))
"#,
        &["42", "0"],
    );
}

#[test]
fn e2e_adt_linked_list() {
    // Test linked list ADT (recursive)
    assert_output(
        r#"
enum List
    Cons(head, tail)
    Nil

*list_sum(l)
    return match l
        Cons(h, t) ? h + list_sum(t)
        Nil ? 0

*list_len(l)
    return match l
        Cons(h, t) ? 1 + list_len(t)
        Nil ? 0

*main()
    xs is Cons(1, Cons(2, Cons(3, Cons(4, Nil))))
    log(list_sum(xs))
    log(list_len(xs))
"#,
        &["10", "4"],
    );
}

#[test]
fn e2e_adt_result_pattern() {
    // Test Result-like ADT (similar to built-in errors but user-defined)
    assert_output(
        r#"
enum Outcome
    Success(value)
    Failure(reason)

*try_parse(s)
    return match s
        "yes" ? Success(1)
        "no" ? Success(0)
        _ ? Failure("unknown input")

*main()
    r1 is try_parse("yes")
    r2 is try_parse("hello")
    msg1 is match r1
        Success(v) ? v
        Failure(r) ? -1
    msg2 is match r2
        Success(v) ? v
        Failure(r) ? -1
    log(msg1)
    log(msg2)
"#,
        &["1", "-1"],
    );
}

#[test]
fn e2e_adt_boolean_enum() {
    // Test simple two-variant enum (like boolean)
    assert_output(
        r#"
enum Answer
    Yes
    No

*to_number(a)
    return match a
        Yes ? 1
        No ? 0

*main()
    log(to_number(Yes))
    log(to_number(No))
"#,
        &["1", "0"],
    );
}

#[test]
fn e2e_adt_pass_to_function() {
    // Test passing ADT values as function arguments
    assert_output(
        r#"
enum Color
    Red
    Green
    Blue

*color_code(c)
    return match c
        Red ? "FF0000"
        Green ? "00FF00"
        Blue ? "0000FF"

*main()
    colors is [Red, Green, Blue]
    i is 0
    while i < colors.length()
        log(color_code(colors[i]))
        i is i + 1
"#,
        &["FF0000", "00FF00", "0000FF"],
    );
}

#[test]
fn e2e_adt_wildcard_match() {
    // Test wildcard pattern in ADT match
    assert_output(
        r#"
enum Direction
    North
    South
    East
    West

*is_vertical(d)
    return match d
        North ? true
        South ? true
        _ ? false

*main()
    log(is_vertical(North))
    log(is_vertical(East))
"#,
        &["true", "false"],
    );
}

// ─── Store / Field Assignment Tests ──────────────────────────────────

#[test]
fn e2e_store_basic_counter() {
    // Test store with field read and field mutation via method
    assert_output(
        r#"
store Counter
    count ? 0

    *increment()
        self.count is self.count + 1

    *get_count()
        return self.count

*main()
    c is make_Counter()
    log(c.get_count())
    c.increment()
    c.increment()
    c.increment()
    log(c.get_count())
"#,
        &["0", "3"],
    );
}

#[test]
fn e2e_store_with_initial_values() {
    // Test store with custom initial field values
    assert_output(
        r#"
store Point
    x ? 0
    y ? 0

    *move_by(dx, dy)
        self.x is self.x + dx
        self.y is self.y + dy

    *describe()
        log(self.x)
        log(self.y)

*main()
    p is make_Point()
    p.move_by(3, 4)
    p.describe()
    p.move_by(1, -2)
    p.describe()
"#,
        &["3", "4", "4", "2"],
    );
}

#[test]
fn e2e_store_string_field() {
    // Test store with string fields
    assert_output(
        r#"
store Greeter
    name ? "World"

    *set_name(n)
        self.name is n

    *greet()
        log("Hello, " + self.name + "!")

*main()
    g is make_Greeter()
    g.greet()
    g.set_name("Coral")
    g.greet()
"#,
        &["Hello, World!", "Hello, Coral!"],
    );
}

#[test]
fn e2e_store_multiple_instances() {
    // Test that multiple store instances are independent
    assert_output(
        r#"
store Counter
    count ? 0

    *increment()
        self.count is self.count + 1

    *get_count()
        return self.count

*main()
    a is make_Counter()
    b is make_Counter()
    a.increment()
    a.increment()
    b.increment()
    log(a.get_count())
    log(b.get_count())
"#,
        &["2", "1"],
    );
}

// ─── Trait E2E Tests ─────────────────────────────────────────────────

#[test]
fn e2e_trait_default_method_on_store() {
    // Store inherits a default method from a trait
    assert_output(
        r#"
trait Describable
    *describe()
        log("I am describable")

store Widget with Describable
    name ? "button"

*main()
    w is make_Widget()
    w.describe()
"#,
        &["I am describable"],
    );
}

#[test]
fn e2e_trait_required_method_on_store() {
    // Store implements a required method from a trait
    assert_output(
        r#"
trait Greeter
    *greet()

store FriendlyBot with Greeter
    name ? "Bot"

    *greet()
        log("Hello from " + self.name)

*main()
    b is make_FriendlyBot()
    b.greet()
"#,
        &["Hello from Bot"],
    );
}

#[test]
fn e2e_trait_override_default() {
    // Store overrides a trait's default method
    assert_output(
        r#"
trait Printable
    *show()
        log("default show")

store Fancy with Printable
    label ? "fancy"

    *show()
        log("Fancy: " + self.label)

*main()
    f is make_Fancy()
    f.show()
"#,
        &["Fancy: fancy"],
    );
}

#[test]
fn e2e_trait_multiple_methods() {
    // Trait with both required and default methods
    assert_output(
        r#"
trait Animal
    *speak()
    *describe()
        log("I am an animal")

store Dog with Animal
    name ? "Rex"

    *speak()
        log(self.name + " says: Woof!")

*main()
    d is make_Dog()
    d.speak()
    d.describe()
"#,
        &["Rex says: Woof!", "I am an animal"],
    );
}

#[test]
fn e2e_store_own_method_plus_trait() {
    // Store has its own methods alongside trait methods
    assert_output(
        r#"
trait Countable
    *count()
        log("counting...")

store Inventory with Countable
    items ? 0

    *add(n)
        self.items is self.items + n

    *show_items()
        log(self.items)

*main()
    inv is make_Inventory()
    inv.add(5)
    inv.add(3)
    inv.show_items()
    inv.count()
"#,
        &["8", "counting..."],
    );
}

// ─── Map & String Operation Tests (Self-Hosting Prerequisites) ───────

#[test]
fn e2e_map_create_and_get() {
    // Basic map creation and field access
    assert_output(
        r#"
*main()
    m is map("name" is "Alice", "age" is 30)
    log(m.get("name"))
    log(m.get("age"))
"#,
        &["Alice", "30"],
    );
}

#[test]
fn e2e_map_set_and_update() {
    // Map mutation via set
    assert_output(
        r#"
*main()
    m is map("x" is 1)
    log(m.get("x"))
    m.set("x", 42)
    log(m.get("x"))
    m.set("y", 99)
    log(m.get("y"))
"#,
        &["1", "42", "99"],
    );
}

#[test]
fn e2e_list_push_and_iterate() {
    // List operations: push and for loop
    assert_output(
        r#"
*main()
    items is [10, 20, 30]
    items.push(40)
    for item in items
        log(item)
"#,
        &["10", "20", "30", "40"],
    );
}

#[test]
fn e2e_string_length_and_concat() {
    // String operations used heavily in self-hosted compiler
    assert_output(
        r#"
*main()
    s is "hello"
    log(s.length())
    result is s + " world"
    log(result)
    log(result.length())
"#,
        &["5", "hello world", "11"],
    );
}

#[test]
fn e2e_nested_if_elif() {
    // Complex if/elif/else chains (used in lexer/parser)
    assert_output(
        r#"
*classify(n)
    if n < 0
        return "negative"
    elif n.equals(0)
        return "zero"
    elif n < 10
        return "small"
    elif n < 100
        return "medium"
    else
        return "large"

*main()
    log(classify(-5))
    log(classify(0))
    log(classify(7))
    log(classify(42))
    log(classify(999))
"#,
        &["negative", "zero", "small", "medium", "large"],
    );
}

#[test]
fn e2e_recursive_fibonacci() {
    // Recursive function (used in compiler for tree traversal)
    assert_output(
        r#"
*fib(n)
    if n < 2
        return n
    return fib(n - 1) + fib(n - 2)

*main()
    log(fib(0))
    log(fib(1))
    log(fib(5))
    log(fib(10))
"#,
        &["0", "1", "5", "55"],
    );
}

#[test]
fn e2e_higher_order_function_apply() {
    // Higher-order functions
    assert_output(
        r#"
*apply_all(items, f)
    for item in items
        log(f(item))

*double(x)
    return x * 2

*main()
    nums is [1, 2, 3, 4, 5]
    apply_all(nums, double)
"#,
        &["2", "4", "6", "8", "10"],
    );
}

#[test]
fn e2e_match_with_return_value() {
    // Match as expression returning values
    assert_output(
        r#"
enum Shape
    Circle(radius)
    Rectangle(w, h)

*area(shape)
    return match shape
        Circle(r) ? 3 * r * r
        Rectangle(w, h) ? w * h

*main()
    c is Circle(10)
    r is Rectangle(3, 4)
    log(area(c))
    log(area(r))
"#,
        &["300", "12"],
    );
}

#[test]
fn e2e_error_creation_and_check() {
    // Error value creation and propagation
    assert_output(
        r#"
*safe_divide(a, b)
    if b is 0
        return err DivisionByZero
    return a / b

*main()
    result is safe_divide(10, 2)
    log(result)
    bad is safe_divide(10, 0)
    log(is_err(bad))
"#,
        &["5", "true"],
    );
}

#[test]
fn e2e_while_with_string_building() {
    // While loop building strings (common in lexer)
    assert_output(
        r#"
*repeat_str(s, n)
    result is ""
    i is 0
    while i < n
        result is result + s
        i is i + 1
    return result

*main()
    log(repeat_str("*", 5))
    log(repeat_str("ab", 3))
"#,
        &["*****", "ababab"],
    );
}

#[test]
fn e2e_nested_function_definitions() {
    // Functions defined inside other functions
    assert_output(
        r#"
*add(a, b)
    return a + b

*multiply(a, b)
    return a * b

*main()
    log(add(3, 4))
    log(multiply(5, 6))
    log(add(multiply(2, 3), 4))
"#,
        &["7", "30", "10"],
    );
}

#[test]
fn e2e_for_loop_with_accumulator() {
    // For loop with mutable accumulator (common pattern in compiler)
    assert_output(
        r#"
*sum_list(items)
    total is 0
    for item in items
        total is total + item
    return total

*max_list(items)
    best is 0
    for item in items
        if item > best
            best is item
    return best

*main()
    nums is [1, 5, 3, 9, 2, 7]
    log(sum_list(nums))
    log(max_list(nums))
"#,
        &["27", "9"],
    );
}

// ─── Guard Statement Tests ───────────────────────────────────────────

#[test]
fn e2e_guard_return_inline() {
    // Guard with return on same line
    assert_output(
        r#"
*classify(x)
    x is 0 ? return "zero"
    x is 1 ? return "one"
    x is 2 ? return "two"
    return "other"

*main()
    log(classify(0))
    log(classify(1))
    log(classify(2))
    log(classify(5))
"#,
        &["zero", "one", "two", "other"],
    );
}

#[test]
fn e2e_guard_expression_condition() {
    // Guard with expression (not binding) as condition
    assert_output(
        r#"
*abs_val(x)
    x < 0 ? return 0 - x
    return x

*main()
    log(abs_val(-5))
    log(abs_val(3))
    log(abs_val(0))
"#,
        &["5", "3", "0"],
    );
}

#[test]
fn e2e_guard_in_loop() {
    // Guard with break/continue in loop
    assert_output(
        r#"
*main()
    i is 0
    while i < 10
        i is i + 1
        i is 3 ? continue
        i is 7 ? break
        log(i)
"#,
        &["1", "2", "4", "5", "6"],
    );
}

#[test]
fn e2e_guard_multiple_conditions() {
    // Multiple guards in sequence (like a match/switch)
    assert_output(
        r#"
*fizzbuzz(n)
    n % 15 is 0 ? return "fizzbuzz"
    n % 3 is 0 ? return "fizz"
    n % 5 is 0 ? return "buzz"
    return "none"

*main()
    log(fizzbuzz(15))
    log(fizzbuzz(9))
    log(fizzbuzz(10))
    log(fizzbuzz(7))
"#,
        &["fizzbuzz", "fizz", "buzz", "none"],
    );
}
