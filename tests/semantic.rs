use coralc::ast::{
    Binding,
    Block,
    Expression,
    Field,
    Function,
    FunctionKind,
    Item,
    Parameter,
    Program,
    Statement,
    StoreDefinition,
    TypeDefinition,
};
use coralc::semantic;
use coralc::span::Span;
use coralc::types::{Primitive, TypeId};

fn span() -> Span {
    Span::new(0, 0)
}

fn int_literal(value: i64) -> Expression {
    Expression::Integer(value, span())
}

fn ident(name: &str) -> Expression {
    Expression::Identifier(name.to_string(), span())
}

#[test]
fn rejects_duplicate_global_bindings() {
    let program = Program::new(
        vec![
            Item::Binding(Binding {
                name: "value".into(),
                type_annotation: None,
                value: int_literal(1),
                span: span(),
            }),
            Item::Binding(Binding {
                name: "value".into(),
                type_annotation: None,
                value: int_literal(2),
                span: span(),
            }),
        ],
        span(),
    );
    let error = semantic::analyze(program).expect_err("expected duplicate binding error");
    assert_eq!(error.message, "duplicate binding `value`");
    assert_eq!(error.help.as_deref(), Some("previous definition at 0..0"));
}

#[test]
fn rejects_duplicate_bindings_in_function_scope() {
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![
                Statement::Binding(Binding {
                    name: "value".into(),
                    type_annotation: None,
                    value: int_literal(1),
                    span: span(),
                }),
                Statement::Binding(Binding {
                    name: "value".into(),
                    type_annotation: None,
                    value: int_literal(2),
                    span: span(),
                }),
            ],
            value: None,
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    let error = semantic::analyze(program).expect_err("expected duplicate binding error");
    assert_eq!(error.message, "duplicate binding `value`");
}

#[test]
fn rejects_duplicate_parameters() {
    let function = Function {
        name: "main".into(),
        params: vec![
            Parameter {
                name: "value".into(),
                type_annotation: None,
                default: None,
                span: span(),
            },
            Parameter {
                name: "value".into(),
                type_annotation: None,
                default: None,
                span: span(),
            },
        ],
        body: Block {
            statements: vec![],
            value: None,
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    let error = semantic::analyze(program).expect_err("expected duplicate parameter error");
    assert_eq!(error.message, "duplicate parameter `value`");
}

#[test]
fn rejects_duplicate_store_fields() {
    let store = StoreDefinition {
        name: "Ledger".into(),
        fields: vec![
            Field {
                name: "balance".into(),
                is_reference: false,
                default: Some(int_literal(0)),
                span: span(),
            },
            Field {
                name: "balance".into(),
                is_reference: false,
                default: Some(int_literal(1)),
                span: span(),
            },
        ],
        methods: vec![],
        is_actor: false,
        span: span(),
    };
    let program = Program::new(vec![Item::Store(store)], span());
    let error = semantic::analyze(program).expect_err("expected duplicate field error");
    assert!(error.message.contains("duplicate field"));
}

#[test]
fn rejects_default_referencing_later_parameter() {
    let function = Function {
        name: "main".into(),
        params: vec![
            Parameter {
                name: "a".into(),
                type_annotation: None,
                default: Some(ident("b")),
                span: span(),
            },
            Parameter {
                name: "b".into(),
                type_annotation: None,
                default: None,
                span: span(),
            },
        ],
        body: Block {
            statements: vec![],
            value: None,
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    let error = semantic::analyze(program).expect_err("expected default order error");
    assert_eq!(
        error.message,
        "default for parameter `a` references later parameter `b`"
    );
}

#[test]
fn allows_default_referencing_earlier_parameter() {
    let function = Function {
        name: "main".into(),
        params: vec![
            Parameter {
                name: "a".into(),
                type_annotation: None,
                default: None,
                span: span(),
            },
            Parameter {
                name: "b".into(),
                type_annotation: None,
                default: Some(ident("a")),
                span: span(),
            },
        ],
        body: Block {
            statements: vec![],
            value: None,
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    semantic::analyze(program).expect("valid default reference to earlier parameter");
}

#[test]
fn assigns_any_to_message_data_and_actor_primitive() {
    let message_type = TypeDefinition {
        name: "Message".into(),
        fields: vec![Field {
            name: "data".into(),
            is_reference: false,
            default: None,
            span: span(),
        }],
        methods: vec![],
        span: span(),
    };
    let actor_store = StoreDefinition {
        name: "Worker".into(),
        fields: vec![],
        methods: vec![],
        is_actor: true,
        span: span(),
    };
    let program = Program::new(vec![Item::Type(message_type), Item::Store(actor_store)], span());

    let model = semantic::analyze(program).expect("semantic analysis should succeed");

    assert_eq!(
        model.types.symbols.get("Message.data"),
        Some(&TypeId::Primitive(Primitive::Any)),
        "Message.data should be forced to Any",
    );
    assert_eq!(
        model.types.symbols.get("Worker"),
        Some(&TypeId::Primitive(Primitive::Actor)),
        "actor stores should register Actor primitive type",
    );
}
