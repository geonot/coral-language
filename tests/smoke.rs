use coralc::Compiler;

#[test]
fn compiles_basic_program() {
    let source = r"*main()
    total is 1
    total + 1
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    // Functions now return i64 (NaN-boxed) after M1 transition
    assert!(ir.contains("define i64 @__user_main"));
}

#[test]
fn lowers_match_expression() {
    let source = r"*main()
    value is 2
    match value
        1 ? 10
        2 ? 20
        ! 30
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(ir.contains("match_arm_0"));
    assert!(ir.contains("match_phi"));
}

#[test]
fn lowers_string_literal_binding() {
    let source = r"*main()
    greeting is 'hello world'
    0
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(
        ir.contains("@coral_make_string"),
        "IR should reference coral_make_string runtime hook"
    );
}

#[test]
fn lowers_logical_and_or() {
    let source = r"*main()
    a is true
    b is false
    (a and b) or true
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(
        ir.contains("@coral_value_as_bool"),
        "bool accessor should be declared"
    );
    assert!(
        ir.contains("logic_phi"),
        "logical operators should build phi nodes"
    );
}

#[test]
fn lowers_addition_via_runtime_helper() {
    let source = r"*main()
    greeting is 'foo'
    greeting + 'bar'
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(
        ir.contains("@coral_value_add"),
        "string addition should route through runtime helper"
    );
}

#[test]
fn lowers_equality_via_runtime_helper() {
    let source = r"*main()
    a is 'foo'
    b is 'bar'
    (a is b)
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(
        ir.contains("@coral_value_equals"),
        "equality should call runtime helper"
    );
}

#[test]
fn lowers_list_literal() {
    let source = r"*main()
    values is [1, 2, 3]
    0
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(
        ir.contains("@coral_make_list"),
        "list literals should call runtime list constructor"
    );
}

#[test]
fn lowers_list_push_and_length() {
    let source = r"*main()
    values is [1, 2]
    values.push(3)
    values.length
    values.get(0)
";
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect(
        "failed to compile source",
    );
    assert!(
        ir.contains("@coral_list_push"),
        "list push should call runtime helper"
    );
    assert!(
        ir.contains("@coral_value_length"),
        "length member should call coral_value_length runtime helper"
    );
    assert!(
        ir.contains("@coral_list_get"),
        "get member should call runtime helper"
    );
}

#[test]
fn lowers_map_literal_and_accessors() {
    let source = r"*main()
    config is map('foo' is 1, 'bar' is 2)
    config.foo
    config.set('foo', 3)
    config.get('bar')
    0
";
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile source");
    assert!(
        ir.contains("@coral_make_map"),
        "map literals should call runtime map constructor"
    );
    assert!(
        ir.contains("@coral_map_get"),
        "map accessors should call map_get runtime helper",
    );
    assert!(
        ir.contains("@coral_map_set"),
        "map.set should call map_set runtime helper",
    );
}

#[test]
fn lowers_list_pop_and_map_size() {
    let source = r"*main()
    values is [1, 2, 3]
    last is values.pop()
    config is map('foo' is 1)
    config.size
";
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile source");
    assert!(
        ir.contains("@coral_list_pop"),
        "list.pop should call runtime helper",
    );
    assert!(
        ir.contains("@coral_map_length"),
        "map.size should call map_length runtime helper",
    );
}

#[test]
fn lowers_log_builtin() {
    let source = r"*main()
    log('hello from coral')
    0
";
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile source");
    assert!(
        ir.contains("@coral_log"),
        "log builtin should call coral_log runtime helper",
    );
}

#[test]
fn compiles_closure_capturing_outer_scope() {
    let source = r"*main()
    x is 10
    f is *fn(y) x + y
    f(5)
";
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile closure with capture");
    // Should create a closure environment struct
    assert!(
        ir.contains("closure_env_alloc") || ir.contains("@coral_heap_alloc"),
        "closure should allocate environment for captured variables"
    );
    // Should store captured value in env
    assert!(
        ir.contains("env_store_capture") || ir.contains("capture"),
        "closure should store captured value `x` in environment"
    );
    // Should invoke closure
    assert!(
        ir.contains("@coral_closure_invoke"),
        "calling closure `f` should use coral_closure_invoke"
    );
}

#[test]
fn compiles_list_map_with_closure() {
    // Test list.map with inline lambda
    let source = r#"*main()
    numbers is [1, 2, 3]
    doubled is numbers.map(*fn(x) x + x)
    doubled.length
"#;
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile list.map with closure");
    assert!(
        ir.contains("@coral_list_map"),
        "list.map should call coral_list_map runtime helper"
    );
    assert!(
        ir.contains("@coral_make_closure") || ir.contains("lambda_invoke"),
        "list.map callback should be compiled as closure"
    );
}

#[test]
fn compiles_list_filter_with_closure() {
    let source = r#"*main()
    numbers is [1, 2, 3, 4, 5]
    evens is numbers.filter(*fn(x) x > 2)
    evens.length
"#;
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile list.filter with closure");
    assert!(
        ir.contains("@coral_list_filter"),
        "list.filter should call coral_list_filter runtime helper"
    );
}

#[test]
fn compiles_list_reduce_with_closure() {
    let source = r#"*main()
    numbers is [1, 2, 3, 4]
    total is numbers.reduce(0, *fn(acc, x) acc + x)
    total
"#;
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile list.reduce with closure");
    assert!(
        ir.contains("@coral_list_reduce"),
        "list.reduce should call coral_list_reduce runtime helper"
    );
}

#[test]
fn compiles_full_language_fixture() {
    let source = include_str!("fixtures/programs/full_language_no_store.coral");
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("failed to compile comprehensive program");
    assert!(ir.contains("@coral_make_list"), "lists should be emitted");
    assert!(ir.contains("@coral_list_push"), "list.push should call runtime helper");
    assert!(ir.contains("@coral_list_pop"), "list.pop should call runtime helper");
    assert!(ir.contains("@coral_make_map"), "map literals should call runtime constructor");
    assert!(ir.contains("@coral_map_get"), "map.get should call runtime helper");
    assert!(ir.contains("@coral_map_set"), "map.set should call runtime helper");
    assert!(ir.contains("match_arm_0"), "match expressions should lower to control flow");
}
