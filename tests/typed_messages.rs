//! R2.7: Typed messages tests.
//!
//! Verifies that `@messages(TypeName)` annotation on actors enables
//! compile-time validation of `actor_send()` handler names.

use coralc::lexer;
use coralc::parser::Parser;
use coralc::semantic;

fn compile_ok(source: &str) -> semantic::SemanticModel {
    let tokens = lexer::lex(source).expect("lex should succeed");
    let source_len = source.len();
    let parser = Parser::new(tokens, source_len);
    let program = parser.parse().expect("parse should succeed");
    semantic::analyze(program).expect("semantic analysis should succeed")
}

fn compile_warnings(source: &str) -> Vec<String> {
    let model = compile_ok(source);
    model.warnings.iter().map(|w| w.message.clone()).collect()
}

#[test]
fn r27_typed_send_to_valid_handler_no_warning() {
    let source = r#"
actor Counter @messages(CounterMsg)
    count is 0

    @increment()
        count is count + 1

    @reset()
        count is 0

*main()
    c is make_Counter()
    actor_send(c, 'increment', 0)
    actor_send(c, 'reset', 0)
    log('done')
"#;
    let warnings = compile_warnings(source);
    let typed_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("@messages"))
        .collect();
    assert!(
        typed_warnings.is_empty(),
        "valid handler names should not produce warnings, got: {:?}",
        typed_warnings
    );
}

#[test]
fn r27_typed_send_to_invalid_handler_warns() {
    let source = r#"
actor Counter @messages(CounterMsg)
    count is 0

    @increment()
        count is count + 1

*main()
    c is make_Counter()
    actor_send(c, 'nonexistent', 0)
    log('done')
"#;
    let warnings = compile_warnings(source);
    let typed_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("@messages") && w.contains("nonexistent"))
        .collect();
    assert!(
        !typed_warnings.is_empty(),
        "sending to nonexistent handler should produce a warning. All warnings: {:?}",
        warnings
    );
    // Verify it mentions the actor name and known handlers
    let w = &typed_warnings[0];
    assert!(w.contains("Counter"), "warning should mention actor name: {}", w);
    assert!(w.contains("increment"), "warning should list known handlers: {}", w);
}

#[test]
fn r27_actor_without_annotation_accepts_any() {
    let source = r#"
actor Worker
    state is 0

    @handle(msg)
        state is msg

*main()
    w is make_Worker()
    actor_send(w, 'anything_goes', 42)
    log('ok')
"#;
    let warnings = compile_warnings(source);
    let typed_warnings: Vec<_> = warnings.iter()
        .filter(|w| w.contains("@messages"))
        .collect();
    assert!(
        typed_warnings.is_empty(),
        "actor without @messages annotation should not produce typed send warnings, got: {:?}",
        typed_warnings
    );
}

#[test]
fn r27_annotation_parsed_and_stored() {
    let source = r#"
actor Validator @messages(ValidatorMsg)
    status is 'ok'

    @validate(data)
        log(data)

    @clear()
        status is 'ok'

*main()
    log('test')
"#;
    let model = compile_ok(source);
    assert!(
        model.actor_message_types.contains_key("Validator"),
        "actor_message_types should contain 'Validator'"
    );
    assert_eq!(
        model.actor_message_types.get("Validator").map(|s| s.as_str()),
        Some("ValidatorMsg"),
        "message type should be 'ValidatorMsg'"
    );
    let handlers = model.actor_handler_names.get("Validator").expect("should have handler names");
    assert!(handlers.contains(&"validate".to_string()), "should contain 'validate'");
    assert!(handlers.contains(&"clear".to_string()), "should contain 'clear'");
}
