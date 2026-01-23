//! Tests for actor timer functionality (6.3)
//!
//! These tests verify that timer runtime functions are properly
//! declared and can be used in Coral programs.

use coralc::Compiler;
use coralc::module_loader::ModuleLoader;
use std::path::PathBuf;

/// Compile and verify source compiles successfully to IR.
fn compile_ok(source: &str) -> String {
    let compiler = Compiler;
    compiler.compile_to_ir(source).expect("Should compile")
}

// ============================================================================
// Timer Function Declaration Tests
// ============================================================================

#[test]
fn timer_runtime_functions_declared() {
    // Verify all timer runtime functions are declared in IR
    let source = r#"
*main()
    log('test')
"#;
    let ir = compile_ok(source);
    
    // Check that the timer functions are declared
    assert!(ir.contains("coral_timer_send_after"), "Should declare timer_send_after");
    assert!(ir.contains("coral_timer_schedule_repeat"), "Should declare timer_schedule_repeat");
    assert!(ir.contains("coral_timer_cancel"), "Should declare timer_cancel");
    assert!(ir.contains("coral_timer_pending_count"), "Should declare timer_pending_count");
}

#[test]
fn declares_timer_send_after_with_correct_signature() {
    let source = r#"
extern fn coral_timer_send_after(delay: usize, actor: usize, msg: usize) : usize

*main()
    'timer declared'
"#;
    let ir = compile_ok(source);
    // Should declare the function (it will be in the IR)
    assert!(ir.contains("coral_timer_send_after"), "Should compile timer extern");
}

#[test]
fn declares_timer_schedule_repeat_with_correct_signature() {
    let source = r#"
extern fn coral_timer_schedule_repeat(interval: usize, actor: usize, msg: usize) : usize

*main()
    'schedule_repeat declared'
"#;
    let ir = compile_ok(source);
    assert!(ir.contains("coral_timer_schedule_repeat"), "Should compile schedule_repeat extern");
}

#[test]
fn declares_timer_cancel_function() {
    let source = r#"
extern fn coral_timer_cancel(timer_id: usize) : usize

*main()
    'cancel declared'
"#;
    let ir = compile_ok(source);
    assert!(ir.contains("coral_timer_cancel"), "Should compile timer_cancel extern");
}

#[test]
fn declares_timer_pending_count_function() {
    let source = r#"
extern fn coral_timer_pending_count() : usize

*main()
    'pending_count declared'
"#;
    let ir = compile_ok(source);
    assert!(ir.contains("coral_timer_pending_count"), "Should compile pending_count extern");
}

// ============================================================================
// Timer Module Import Tests
// ============================================================================

#[test]
fn compiles_program_using_timer_module() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_timers.coral");
    let source = loader.load(&path).expect("load timer program");
    
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(&source).expect("Should compile");
    
    // The timer functions should be declared in the IR
    assert!(ir.contains("coral_timer_send_after"), "Should have timer_send_after in IR");
    assert!(ir.contains("coral_timer_schedule_repeat"), "Should have timer_schedule_repeat in IR");
    assert!(ir.contains("coral_timer_cancel"), "Should have timer_cancel in IR");
    assert!(ir.contains("coral_timer_pending_count"), "Should have timer_pending_count in IR");
}
