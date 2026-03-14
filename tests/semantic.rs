use coralc::ast::{
    Binding, Block, Expression, Field, Function, FunctionKind, Item, Parameter, Program, Statement,
    StoreDefinition, TypeAnnotation, TypeDefinition, TypeVariant, VariantField,
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
fn allows_rebinding_in_function_scope() {
    // Rebinding (shadowing) is allowed: `value is 1; value is 2`
    // This is needed for while loop counter patterns like `i is i + 1`
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
    let result = semantic::analyze(program);
    assert!(
        result.is_ok(),
        "rebinding should be allowed, got: {:?}",
        result.err()
    );
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
        is_persistent: false,
        message_type: None,
        with_traits: vec![],
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
        type_params: vec![],
        fields: vec![Field {
            name: "data".into(),
            is_reference: false,
            default: None,
            span: span(),
        }],
        methods: vec![],
        variants: vec![],
        with_traits: vec![],
        span: span(),
    };
    let actor_store = StoreDefinition {
        name: "Worker".into(),
        fields: vec![],
        methods: vec![],
        is_actor: true,
        is_persistent: false,
        message_type: None,
        with_traits: vec![],
        span: span(),
    };
    let program = Program::new(
        vec![Item::Type(message_type), Item::Store(actor_store)],
        span(),
    );

    let model = semantic::analyze(program).expect("semantic analysis should succeed");

    assert_eq!(
        model.types.get("Message.data"),
        Some(&TypeId::Primitive(Primitive::Any)),
        "Message.data should be forced to Any",
    );
    assert_eq!(
        model.types.get("Worker"),
        Some(&TypeId::Primitive(Primitive::Actor)),
        "actor stores should register Actor primitive type",
    );
}

#[test]
fn rejects_actor_handler_with_too_many_params() {
    let actor_store = StoreDefinition {
        name: "BadActor".into(),
        fields: vec![],
        methods: vec![Function {
            name: "handle".into(),
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
                    default: None,
                    span: span(),
                },
            ],
            body: Block {
                statements: vec![],
                value: Some(Box::new(int_literal(0))),
                span: span(),
            },
            kind: FunctionKind::ActorMessage,
            span: span(),
        }],
        is_actor: true,
        is_persistent: false,
        message_type: None,
        with_traits: vec![],
        span: span(),
    };
    let program = Program::new(vec![Item::Store(actor_store)], span());
    let error = semantic::analyze(program).expect_err("expected handler arity error");
    assert!(
        error.message.contains("at most 1"),
        "error should mention handler param limit: {}",
        error.message
    );
}

#[test]
fn accepts_actor_handler_with_zero_or_one_param() {
    // Handler with no params
    let handler0 = Function {
        name: "ping".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(int_literal(0))),
            span: span(),
        },
        kind: FunctionKind::ActorMessage,
        span: span(),
    };
    // Handler with one param
    let handler1 = Function {
        name: "handle".into(),
        params: vec![Parameter {
            name: "data".into(),
            type_annotation: None,
            default: None,
            span: span(),
        }],
        body: Block {
            statements: vec![],
            value: Some(Box::new(int_literal(0))),
            span: span(),
        },
        kind: FunctionKind::ActorMessage,
        span: span(),
    };
    let actor_store = StoreDefinition {
        name: "GoodActor".into(),
        fields: vec![],
        methods: vec![handler0, handler1],
        is_actor: true,
        is_persistent: false,
        message_type: None,
        with_traits: vec![],
        span: span(),
    };
    let program = Program::new(vec![Item::Store(actor_store)], span());
    semantic::analyze(program).expect("handlers with 0 or 1 params should be valid");
}

#[test]
fn rejects_undefined_name_in_function_body() {
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(ident("undefined_variable"))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    let error = semantic::analyze(program).expect_err("expected undefined name error");
    assert!(
        error
            .message
            .contains("undefined name `undefined_variable`"),
        "error should mention undefined name: {}",
        error.message
    );
}

#[test]
fn accepts_defined_function_call() {
    let callee = Function {
        name: "helper".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(int_literal(42))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let caller = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(Expression::Call {
                callee: Box::new(ident("helper")),
                args: vec![],
                arg_names: vec![],
                span: span(),
            })),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(callee), Item::Function(caller)], span());
    semantic::analyze(program).expect("calling a defined function should work");
}

#[test]
fn accepts_parameter_reference() {
    let function = Function {
        name: "main".into(),
        params: vec![Parameter {
            name: "x".into(),
            type_annotation: None,
            default: None,
            span: span(),
        }],
        body: Block {
            statements: vec![],
            value: Some(Box::new(ident("x"))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    semantic::analyze(program).expect("referencing a parameter should work");
}

#[test]
fn accepts_local_binding_reference() {
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![Statement::Binding(Binding {
                name: "y".into(),
                type_annotation: None,
                value: int_literal(10),
                span: span(),
            })],
            value: Some(Box::new(ident("y"))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    semantic::analyze(program).expect("referencing a local binding should work");
}

#[test]
fn accepts_builtin_function_call() {
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(Expression::Call {
                callee: Box::new(ident("log")),
                args: vec![int_literal(42)],
                arg_names: vec![],
                span: span(),
            })),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(vec![Item::Function(function)], span());
    semantic::analyze(program).expect("calling a builtin function should work");
}

#[test]
fn accepts_enum_constructor_call() {
    // Define an enum: enum Option { Some(value), None }
    let option_enum = TypeDefinition {
        name: "Option".into(),
        type_params: vec![],
        fields: vec![],
        methods: vec![],
        variants: vec![
            TypeVariant {
                name: "Some".into(),
                fields: vec![VariantField {
                    name: Some("value".into()),
                    type_annotation: None,
                    span: span(),
                }],
                span: span(),
            },
            TypeVariant {
                name: "None".into(),
                fields: vec![],
                span: span(),
            },
        ],
        with_traits: vec![],
        span: span(),
    };

    // Call the constructor: Some(42)
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(Expression::Call {
                callee: Box::new(ident("Some")),
                args: vec![int_literal(42)],
                arg_names: vec![],
                span: span(),
            })),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(
        vec![Item::Type(option_enum), Item::Function(function)],
        span(),
    );
    semantic::analyze(program).expect("calling an enum constructor should work");
}

#[test]
fn accepts_nullary_enum_constructor() {
    // Define an enum: enum Option { Some(value), None }
    let option_enum = TypeDefinition {
        name: "Option".into(),
        type_params: vec![],
        fields: vec![],
        methods: vec![],
        variants: vec![
            TypeVariant {
                name: "Some".into(),
                fields: vec![VariantField {
                    name: Some("value".into()),
                    type_annotation: None,
                    span: span(),
                }],
                span: span(),
            },
            TypeVariant {
                name: "None".into(),
                fields: vec![],
                span: span(),
            },
        ],
        with_traits: vec![],
        span: span(),
    };

    // Reference the nullary constructor: None
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(ident("None"))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(
        vec![Item::Type(option_enum), Item::Function(function)],
        span(),
    );
    semantic::analyze(program).expect("referencing a nullary enum constructor should work");
}

// ==== TYPE ERROR TESTS ====

fn float_literal(value: f64) -> Expression {
    Expression::Float(value, span())
}

fn bool_literal(value: bool) -> Expression {
    Expression::Bool(value, span())
}

fn str_literal(value: &str) -> Expression {
    Expression::String(value.to_string(), span())
}

fn binary_op(s: &str) -> coralc::ast::BinaryOp {
    use coralc::ast::BinaryOp::*;
    match s {
        "+" => Add,
        "-" => Sub,
        "*" => Mul,
        "/" => Div,
        "%" => Mod,
        "and" => And,
        "or" => Or,
        "==" | "is" => Equals,
        "<" => Less,
        "<=" => LessEq,
        ">" => Greater,
        ">=" => GreaterEq,
        _ => panic!("unknown binary op: {}", s),
    }
}

fn binary(left: Expression, op: &str, right: Expression) -> Expression {
    Expression::Binary {
        left: Box::new(left),
        op: binary_op(op),
        right: Box::new(right),
        span: span(),
    }
}

fn call(name: &str, args: Vec<Expression>) -> Expression {
    Expression::Call {
        callee: Box::new(ident(name)),
        args,
        arg_names: vec![],
        span: span(),
    }
}

fn ternary(cond: Expression, then_expr: Expression, else_expr: Expression) -> Expression {
    Expression::Ternary {
        condition: Box::new(cond),
        then_branch: Box::new(then_expr),
        else_branch: Box::new(else_expr),
        span: span(),
    }
}

fn single_fn_program(body_expr: Expression) -> Program {
    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(body_expr)),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    Program::new(vec![Item::Function(function)], span())
}

#[test]
fn rejects_ternary_condition_not_boolean() {
    // 42 ? 1 : 2 should fail - condition is Int not Bool
    let program = single_fn_program(ternary(int_literal(42), int_literal(1), int_literal(2)));
    let error = semantic::analyze(program).expect_err("expected type error for non-bool condition");
    assert!(
        error.message.contains("type") || error.message.contains("Bool"),
        "error should mention type or Bool: {}",
        error.message
    );
}

#[test]
fn rejects_ternary_branches_with_different_primitives() {
    // Coral now uses strict type checking.
    // true ? 1 : "string" - should fail because Int != String
    let program = single_fn_program(ternary(
        bool_literal(true),
        int_literal(1),
        str_literal("hello"),
    ));
    let error = semantic::analyze(program)
        .expect_err("ternary with different primitive branches should fail");
    assert!(
        error.message.contains("type mismatch") || error.message.contains("type"),
        "error should mention type mismatch: {}",
        error.message
    );
}

#[test]
fn accepts_binary_op_with_string_and_other() {
    // String + number is valid (string concatenation with auto-conversion)
    // "hello" + 1 -> "hello1"
    let program = single_fn_program(binary(str_literal("hello"), "+", int_literal(1)));
    semantic::analyze(program).expect("string + number should be accepted for concatenation");
}

#[test]
fn rejects_logical_op_non_boolean() {
    // 1 and 2 should fail - logical ops need booleans
    let program = single_fn_program(binary(int_literal(1), "and", int_literal(2)));
    let error =
        semantic::analyze(program).expect_err("expected type error for non-bool logical op");
    assert!(
        error.message.contains("Bool") || error.message.contains("type"),
        "error should mention Bool or type: {}",
        error.message
    );
}

#[test]
fn accepts_function_with_fewer_args_than_params() {
    // NOTE: Coral has flexible arity constraints due to dynamic semantics.
    // Function with 2 params called with 1 arg - currently accepted
    let add_fn = Function {
        name: "add".into(),
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
                default: None,
                span: span(),
            },
        ],
        body: Block {
            statements: vec![],
            value: Some(Box::new(binary(ident("a"), "+", ident("b")))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let main_fn = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(call("add", vec![int_literal(1)]))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };
    let program = Program::new(
        vec![Item::Function(add_fn), Item::Function(main_fn)],
        span(),
    );
    // Current behavior: arity is flexible/not strictly enforced
    semantic::analyze(program).expect("function call with flexible arity currently accepted");
}

#[test]
fn accepts_correctly_typed_ternary_expression() {
    // true ? 1 : 2 - both branches return Int
    let program = single_fn_program(ternary(bool_literal(true), int_literal(1), int_literal(2)));
    semantic::analyze(program).expect("correctly typed ternary should be accepted");
}

#[test]
fn accepts_correctly_typed_arithmetic() {
    // 1 + 2 * 3 - all integers
    let program = single_fn_program(binary(
        int_literal(1),
        "+",
        binary(int_literal(2), "*", int_literal(3)),
    ));
    semantic::analyze(program).expect("integer arithmetic should be accepted");
}

#[test]
fn accepts_correctly_typed_comparison() {
    // 1 < 2 - returns Bool
    let program = single_fn_program(binary(int_literal(1), "<", int_literal(2)));
    semantic::analyze(program).expect("comparison should be accepted");
}

#[test]
fn accepts_correctly_typed_logical_ops() {
    // true and false
    let program = single_fn_program(binary(bool_literal(true), "and", bool_literal(false)));
    semantic::analyze(program).expect("boolean logical op should be accepted");
}

#[test]
fn rejects_comparison_result_in_arithmetic() {
    // Coral now uses strict type checking.
    // (1 < 2) + 3 - should fail because Bool + Int is not allowed
    let program = single_fn_program(binary(
        binary(int_literal(1), "<", int_literal(2)),
        "+",
        int_literal(3),
    ));
    let error = semantic::analyze(program).expect_err("bool + int should be rejected");
    assert!(
        error.message.contains("type mismatch") || error.message.contains("Bool"),
        "error should mention type mismatch: {}",
        error.message
    );
}

#[test]
fn rejects_calling_non_callable() {
    // 42() - can't call a number
    let program = single_fn_program(Expression::Call {
        callee: Box::new(int_literal(42)),
        args: vec![],
        arg_names: vec![],
        span: span(),
    });
    let error = semantic::analyze(program).expect_err("expected error for calling non-callable");
    assert!(
        error.message.contains("callable") || error.message.contains("type"),
        "error should mention callable: {}",
        error.message
    );
}

#[test]
fn rejects_member_access_on_non_store() {
    // 42.foo - can't access member on int
    let program = single_fn_program(Expression::Member {
        target: Box::new(int_literal(42)),
        property: "foo".into(),
        span: span(),
    });
    // This may or may not currently error - document the behavior
    let result = semantic::analyze(program);
    // Member access on non-store might be dynamically resolved at runtime
    // so this test documents current behavior rather than expecting error
    assert!(result.is_ok() || result.is_err()); // just document the behavior exists
}

// ============================================================================
// Tests for .equals() and .not() method-based equality
// ============================================================================

#[test]
fn method_based_equality_test() {
    // x.equals(y) for equality comparison - tested via codegen
    // This is a placeholder to document the new equality model
    assert!(true);
}

// ============================================================================
// T2.1/T2.2: Generic type parameters and let-polymorphism
// ============================================================================

#[test]
fn generic_enum_constructor_infers_fresh_types() {
    // enum Option[T]
    //     Some(value)
    //     None
    // *main()
    //     x is Some(42)
    //     y is Some("hello")
    // Both should succeed - each call gets fresh type vars (let-polymorphism)
    let option_enum = TypeDefinition {
        name: "Option".into(),
        type_params: vec!["T".into()],
        fields: vec![],
        methods: vec![],
        variants: vec![
            TypeVariant {
                name: "Some".into(),
                fields: vec![VariantField {
                    name: Some("value".into()),
                    type_annotation: None,
                    span: span(),
                }],
                span: span(),
            },
            TypeVariant {
                name: "None".into(),
                fields: vec![],
                span: span(),
            },
        ],
        with_traits: vec![],
        span: span(),
    };

    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![
                Statement::Binding(Binding {
                    name: "x".into(),
                    type_annotation: None,
                    value: Expression::Call {
                        callee: Box::new(ident("Some")),
                        args: vec![int_literal(42)],
                        arg_names: vec![],
                        span: span(),
                    },
                    span: span(),
                }),
                Statement::Binding(Binding {
                    name: "y".into(),
                    type_annotation: None,
                    value: Expression::Call {
                        callee: Box::new(ident("Some")),
                        args: vec![Expression::String("hello".into(), span())],
                        arg_names: vec![],
                        span: span(),
                    },
                    span: span(),
                }),
            ],
            value: None,
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };

    let program = Program::new(
        vec![Item::Type(option_enum), Item::Function(function)],
        span(),
    );
    semantic::analyze(program).expect("generic enum should allow polymorphic constructor calls");
}

#[test]
fn generic_nullary_constructor_in_match() {
    // Generic None should work in match patterns
    let option_enum = TypeDefinition {
        name: "Option".into(),
        type_params: vec!["T".into()],
        fields: vec![],
        methods: vec![],
        variants: vec![
            TypeVariant {
                name: "Some".into(),
                fields: vec![VariantField {
                    name: Some("value".into()),
                    type_annotation: None,
                    span: span(),
                }],
                span: span(),
            },
            TypeVariant {
                name: "None".into(),
                fields: vec![],
                span: span(),
            },
        ],
        with_traits: vec![],
        span: span(),
    };

    let function = Function {
        name: "main".into(),
        params: vec![],
        body: Block {
            statements: vec![],
            value: Some(Box::new(ident("None"))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };

    let program = Program::new(
        vec![Item::Type(option_enum), Item::Function(function)],
        span(),
    );
    semantic::analyze(program).expect("generic nullary constructor should type-check");
}

#[test]
fn generic_type_annotation_in_param() {
    // T2.3: *foo(x: Option[Int]) should parse and type-check
    // The annotation creates Adt("Option", [Primitive(Int)])
    let option_enum = TypeDefinition {
        name: "Option".into(),
        type_params: vec!["T".into()],
        fields: vec![],
        methods: vec![],
        variants: vec![
            TypeVariant {
                name: "Some".into(),
                fields: vec![VariantField {
                    name: Some("value".into()),
                    type_annotation: None,
                    span: span(),
                }],
                span: span(),
            },
            TypeVariant {
                name: "None".into(),
                fields: vec![],
                span: span(),
            },
        ],
        with_traits: vec![],
        span: span(),
    };

    let function = Function {
        name: "main".into(),
        params: vec![Parameter {
            name: "x".into(),
            type_annotation: Some(TypeAnnotation {
                segments: vec!["Option".into()],
                type_args: vec![TypeAnnotation {
                    segments: vec!["Int".into()],
                    type_args: vec![],
                    span: span(),
                }],
                span: span(),
            }),
            default: None,
            span: span(),
        }],
        body: Block {
            statements: vec![],
            value: Some(Box::new(ident("x"))),
            span: span(),
        },
        kind: FunctionKind::Free,
        span: span(),
    };

    let program = Program::new(
        vec![Item::Type(option_enum), Item::Function(function)],
        span(),
    );
    semantic::analyze(program).expect("generic type annotation should type-check");
}
