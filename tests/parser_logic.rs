use coralc::ast::{Binding, Expression, Item, BinaryOp};
use coralc::lexer;
use coralc::parser::Parser;

fn parse_binding_item(source: &str) -> Binding {
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    assert_eq!(program.items.len(), 1);
    match program.items.into_iter().next().unwrap() {
        Item::Binding(binding) => binding,
        other => panic!("expected toplevel binding, found {:?}", other),
    }
}

fn parse_single_binding_expression(source: &str) -> Expression {
    parse_binding_item(source).value
}

#[test]
fn parses_binding_type_annotation() {
    let binding = parse_binding_item("answer: Number is 42\n");
    let annotation = binding
        .type_annotation
        .expect("missing annotation");
    assert_eq!(annotation.segments, vec!["Number".to_string()]);
    assert_eq!(annotation.type_args, vec![]);
}

#[test]
fn parses_function_parameter_annotation() {
    let source = "*compute(value: Number)\n    value\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    let func = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function item, got {:?}", other),
    };
    let annotation = func.params[0]
        .type_annotation
        .as_ref()
        .expect("missing parameter annotation");
    assert_eq!(annotation.segments, vec!["Number".to_string()]);
    assert_eq!(annotation.type_args, vec![]);
}

#[test]
fn logic_operator_precedence_or_vs_and() {
    let expr = parse_single_binding_expression("result is true or false and false\n");
    match expr {
        Expression::Binary { op: BinaryOp::Or, left, right, .. } => {
            assert!(matches!(*left, Expression::Bool(true, _)), "left side should be literal true");
            match *right {
                Expression::Binary { op: BinaryOp::And, .. } => {}
                other => panic!("expected right branch to be AND, got {:?}", other),
            }
        }
        other => panic!("expected top-level OR expression, got {:?}", other),
    }
}

#[test]
fn parses_list_literal_in_binding() {
    let expr = parse_single_binding_expression("items is [\n1,\n\n2,\n3\n]\n");
    match expr {
        Expression::List(elements, _) => assert_eq!(elements.len(), 3),
        other => panic!("expected list literal, got {:?}", other),
    }
}

#[test]
fn parses_member_call_expression() {
    let expr = parse_single_binding_expression("result is [1].push(2)\n");
    match expr {
        Expression::Call { callee, args, .. } => {
            assert_eq!(args.len(), 1);
            match *callee {
                Expression::Member { property, .. } => assert_eq!(property, "push"),
                other => panic!("expected member callee, got {:?}", other),
            }
        }
        other => panic!("expected call expression, got {:?}", other),
    }
}

#[test]
fn parses_member_call_with_argument() {
    let expr = parse_single_binding_expression("result is [1, 2].get(0)\n");
    match expr {
        Expression::Call { callee, args, .. } => {
            assert_eq!(args.len(), 1);
            match *callee {
                Expression::Member { property, .. } => assert_eq!(property, "get"),
                other => panic!("expected member callee, got {:?}", other),
            }
        }
        other => panic!("expected call expression, got {:?}", other),
    }
}

#[test]
fn parses_map_literal() {
    let expr = parse_single_binding_expression(
        "config is map('foo' is 1, 'bar' is 2)\n",
    );
    match expr {
        Expression::Map(entries, _) => {
            assert_eq!(entries.len(), 2);
        }
        other => panic!("expected map literal, got {:?}", other),
    }
}

#[test]
fn parses_map_property_access() {
    let expr = parse_single_binding_expression(
        "value is map('foo' is 1).foo\n",
    );
    match expr {
        Expression::Member { property, .. } => assert_eq!(property, "foo"),
        other => panic!("expected member access, got {:?}", other),
    }
}

#[test]
fn parses_map_get_method_call() {
    let expr = parse_single_binding_expression(
        "value is map('foo' is 1).get('foo')\n",
    );
    match expr {
        Expression::Call { callee, args, .. } => {
            assert_eq!(args.len(), 1);
            match *callee {
                Expression::Member { property, .. } => assert_eq!(property, "get"),
                other => panic!("expected member callee, got {:?}", other),
            }
        }
        other => panic!("expected call expression, got {:?}", other),
    }
}

#[test]
fn parses_map_set_method_call() {
    let expr = parse_single_binding_expression(
        "value is map('foo' is 1).set('foo', 2)\n",
    );
    match expr {
        Expression::Call { callee, args, .. } => {
            assert_eq!(args.len(), 2);
            match *callee {
                Expression::Member { property, .. } => assert_eq!(property, "set"),
                other => panic!("expected member callee, got {:?}", other),
            }
        }
        other => panic!("expected call expression, got {:?}", other),
    }
}

#[test]
fn parses_placeholder_argument_in_call() {
    let expr = parse_single_binding_expression("result is prices.map($ * 1.15)\n");
    let call = match expr {
        Expression::Call { args, .. } => {
            assert_eq!(args.len(), 1);
            args.into_iter().next().unwrap()
        }
        other => panic!("expected call expression, got {:?}", other),
    };
    match call {
        Expression::Binary { left, .. } => match *left {
            Expression::Placeholder(index, _) => assert_eq!(index, 0),
            other => panic!("expected placeholder on left side, got {:?}", other),
        },
        other => panic!("expected placeholder binary expression, got {:?}", other),
    }
}

#[test]
fn parses_interpolated_string() {
    let expr = parse_single_binding_expression("greeting is 'Hello, {name}!'\n");
    match expr {
        Expression::Binary { op: BinaryOp::Add, left, right, .. } => {
            match *right {
                Expression::String(ref suffix, _) => assert_eq!(suffix, "!"),
                other => panic!("expected suffix literal, got {:?}", other),
            }
            match *left {
                Expression::Binary { op: BinaryOp::Add, left: inner_left, right: inner_right, .. } => {
                    match *inner_left {
                        Expression::String(ref value, _) => assert_eq!(value, "Hello, "),
                        other => panic!("expected leading literal, got {:?}", other),
                    }
                    match *inner_right {
                        Expression::Identifier(ref name, _) => assert_eq!(name, "name"),
                        other => panic!("expected identifier, got {:?}", other),
                    }
                }
                other => panic!("expected inner concatenation, got {:?}", other),
            }
        }
        other => panic!("expected concatenation expression, got {:?}", other),
    }
}

#[test]
fn parses_taxonomy_definition() {
    let source = "!!Database\n    !!Connection\n        code is 5001\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Taxonomy(node) => {
            assert_eq!(node.name, "Database");
            assert_eq!(node.children.len(), 1);
            let child = &node.children[0];
            assert_eq!(child.name, "Connection");
            assert_eq!(child.bindings.len(), 1);
            assert_eq!(child.bindings[0].name, "code");
        }
        other => panic!("expected taxonomy item, got {:?}", other),
    }
}

#[test]
fn parses_throw_expression_with_taxonomy_path() {
    let expr = parse_single_binding_expression("result is ! !!Database:Connection:Timeout\n");
    match expr {
        Expression::Throw { value, .. } => match *value {
            Expression::TaxonomyPath { segments, .. } => {
                assert_eq!(segments, vec!["Database", "Connection", "Timeout"]);
            }
            other => panic!("expected taxonomy path, got {:?}", other),
        },
        other => panic!("expected throw expression, got {:?}", other),
    }
}

#[test]
fn parses_lambda_literal_with_inline_body() {
    let expr = parse_single_binding_expression("increment is *fn(value) value + 1\n");
    match expr {
        Expression::Lambda { params, body, .. } => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "value");
            assert!(body.statements.is_empty());
            assert!(body.value.is_some());
        }
        other => panic!("expected lambda expression, got {:?}", other),
    }
}
