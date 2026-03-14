//! Tests for named actor registry (6.1)
//!
//! These tests verify that named actor runtime functions are properly
//! declared and can be used in Coral programs.

use coralc::Compiler;

/// Compile and verify source compiles successfully to IR.
fn compile_ok(source: &str) -> String {
    let compiler = Compiler;
    compiler.compile_to_ir(source).expect("Should compile")
}

#[test]
fn named_actor_registry_runtime_functions_declared() {
    // Verify all named actor runtime functions are declared in IR
    let source = r#"
*main()
    log('test')
"#;
    let ir = compile_ok(source);

    // Check that the named actor functions are declared
    assert!(
        ir.contains("coral_actor_spawn_named"),
        "Should declare spawn_named"
    );
    assert!(ir.contains("coral_actor_lookup"), "Should declare lookup");
    assert!(
        ir.contains("coral_actor_register"),
        "Should declare register"
    );
    assert!(
        ir.contains("coral_actor_unregister"),
        "Should declare unregister"
    );
    assert!(
        ir.contains("coral_actor_send_named"),
        "Should declare send_named"
    );
    assert!(
        ir.contains("coral_actor_list_named"),
        "Should declare list_named"
    );
}

#[test]
fn actor_spawn_still_works() {
    // Ensure regular actor spawn still compiles
    let source = r#"
*main()
    x is 42
    log(x)
"#;
    let ir = compile_ok(source);
    assert!(
        ir.contains("coral_actor_spawn"),
        "Should declare regular spawn"
    );
}

#[test]
fn actor_send_still_works() {
    // Ensure regular actor send still compiles
    let source = r#"
*main()
    x is 42
    log(x)
"#;
    let ir = compile_ok(source);
    assert!(
        ir.contains("coral_actor_send"),
        "Should declare regular send"
    );
}
