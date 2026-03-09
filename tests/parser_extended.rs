//! Extended parser tests for Phase B — edge cases, error recovery, and complex syntax.

use coralc::ast::{Expression, Item, BinaryOp};
use coralc::lexer;
use coralc::parser::Parser;

fn parse_ok(source: &str) -> coralc::ast::Program {
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    parser.parse().expect("parsing failed")
}

fn parse_err(source: &str) -> String {
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    match parser.parse() {
        Err(d) => d.message,
        Ok(_) => panic!("Expected parse error but succeeded"),
    }
}

fn first_binding_expr(source: &str) -> Expression {
    let program = parse_ok(source);
    match program.items.into_iter().next().unwrap() {
        Item::Binding(b) => b.value,
        other => panic!("expected binding, got {:?}", other),
    }
}

// ─── Complex Expression Parsing ─────────────────────────────────────

#[test]
fn parse_nested_ternary() {
    // Ternary in the else-branch works
    let expr = first_binding_expr("x is a > 0 ? \"positive\" ! a > -10 ? \"small neg\" ! \"big neg\"\n");
    match expr {
        Expression::Ternary { .. } => {}
        _ => panic!("expected ternary expression"),
    }
}

#[test]
fn parse_pipeline_expression() {
    let expr = first_binding_expr("x is 5 ~ double ~ add_one\n");
    match expr {
        Expression::Pipeline { .. } => {}
        _ => panic!("expected pipeline expression, got {:?}", expr),
    }
}

#[test]
fn parse_list_literal_empty() {
    let expr = first_binding_expr("x is []\n");
    match expr {
        Expression::List(items, _) => assert_eq!(items.len(), 0),
        _ => panic!("expected list"),
    }
}

#[test]
fn parse_list_literal_multiple() {
    let expr = first_binding_expr("x is [1, 2, 3, 4, 5]\n");
    match expr {
        Expression::List(items, _) => assert_eq!(items.len(), 5),
        _ => panic!("expected list"),
    }
}

#[test]
fn parse_map_literal() {
    let program = parse_ok("x is map(\"a\" is 1, \"b\" is 2)\n");
    assert_eq!(program.items.len(), 1);
}

#[test]
fn parse_lambda_expression() {
    let expr = first_binding_expr("f is *fn(x) x * 2\n");
    match expr {
        Expression::Lambda { .. } => {}
        _ => panic!("expected lambda expression, got {:?}", expr),
    }
}

#[test]
fn parse_lambda_multi_param() {
    let expr = first_binding_expr("f is *fn(a, b) a + b\n");
    match expr {
        Expression::Lambda { params, .. } => assert_eq!(params.len(), 2),
        _ => panic!("expected lambda, got {:?}", expr),
    }
}

#[test]
fn parse_member_access() {
    let expr = first_binding_expr("x is obj.field\n");
    match expr {
        Expression::Member { property, .. } => assert_eq!(property, "field"),
        _ => panic!("expected member expression"),
    }
}

#[test]
fn parse_method_call() {
    let expr = first_binding_expr("x is lst.get(0)\n");
    match expr {
        Expression::Call { .. } => {}
        _ => panic!("expected call expression, got {:?}", expr),
    }
}

#[test]
fn parse_chained_method_calls() {
    let program = parse_ok("x is lst.map(*fn(x) x).filter(*fn(x) x > 0)\n");
    assert_eq!(program.items.len(), 1);
}

// ─── Function Parsing ───────────────────────────────────────────────

#[test]
fn parse_function_no_params() {
    let program = parse_ok("*greet()\n    log(\"hello\")\n");
    match &program.items[0] {
        Item::Function(f) => {
            assert_eq!(f.name, "greet");
            assert_eq!(f.params.len(), 0);
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn parse_function_three_params() {
    let program = parse_ok("*add3(a, b, c)\n    a + b + c\n");
    match &program.items[0] {
        Item::Function(f) => {
            assert_eq!(f.params.len(), 3);
            assert_eq!(f.params[0].name, "a");
            assert_eq!(f.params[2].name, "c");
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn parse_function_with_if() {
    let program = parse_ok("*check(x)\n    if x > 0\n        \"positive\"\n    else\n        \"non-positive\"\n");
    match &program.items[0] {
        Item::Function(f) => {
            assert_eq!(f.name, "check");
        }
        _ => panic!("expected function"),
    }
}

// ─── Store Parsing ──────────────────────────────────────────────────

#[test]
fn parse_store_definition() {
    let program = parse_ok("store Point\n    x ? 0\n    y ? 0\n");
    match &program.items[0] {
        Item::Store(s) => {
            assert_eq!(s.name, "Point");
            assert_eq!(s.fields.len(), 2);
        }
        _ => panic!("expected store"),
    }
}

#[test]
fn parse_store_with_method() {
    let program = parse_ok("store Counter\n    count ? 0\n\n    *inc()\n        self.count is self.count + 1\n");
    match &program.items[0] {
        Item::Store(s) => {
            assert_eq!(s.name, "Counter");
            assert!(s.methods.len() >= 1);
        }
        _ => panic!("expected store"),
    }
}

// ─── Error Definition Parsing ───────────────────────────────────────

#[test]
fn parse_error_definition() {
    let program = parse_ok("err NotFound\n    code is 404\n    message is \"not found\"\n");
    match &program.items[0] {
        Item::ErrorDefinition(e) => {
            assert_eq!(e.name, "NotFound");
        }
        _ => panic!("expected error definition, got {:?}", program.items[0]),
    }
}

// ─── Match Expression Parsing ───────────────────────────────────────

#[test]
fn parse_match_with_wildcards() {
    let program = parse_ok("*test(x)\n    return match x\n        1 ? \"one\"\n        2 ? \"two\"\n        ! \"other\"\n");
    match &program.items[0] {
        Item::Function(f) => {
            assert_eq!(f.name, "test");
        }
        _ => panic!("expected function"),
    }
}

// ─── Trait Parsing ──────────────────────────────────────────────────

#[test]
fn parse_trait_with_multiple_methods() {
    let program = parse_ok("trait Serializable\n    *to_json()\n    *from_json(data)\n");
    match &program.items[0] {
        Item::TraitDefinition(t) => {
            assert_eq!(t.name, "Serializable");
            assert_eq!(t.methods.len(), 2);
        }
        _ => panic!("expected trait"),
    }
}

// ─── Type Definition Parsing ────────────────────────────────────────

#[test]
fn parse_type_definition() {
    let program = parse_ok("enum Color\n    Red\n    Green\n    Blue\n");
    match &program.items[0] {
        Item::Type(t) => {
            assert_eq!(t.name, "Color");
            assert_eq!(t.variants.len(), 3);
        }
        _ => panic!("expected type definition"),
    }
}

// ─── Operator Precedence ────────────────────────────────────────────

#[test]
fn parse_mul_before_add() {
    let expr = first_binding_expr("x is 1 + 2 * 3\n");
    match expr {
        Expression::Binary { op: BinaryOp::Add, right, .. } => {
            match *right {
                Expression::Binary { op: BinaryOp::Mul, .. } => {}
                _ => panic!("expected multiplication on right side"),
            }
        }
        _ => panic!("expected addition at top level"),
    }
}

#[test]
fn parse_comparison_in_binding() {
    let expr = first_binding_expr("result is x > 5\n");
    match expr {
        Expression::Binary { op: BinaryOp::Greater, .. } => {}
        _ => panic!("expected comparison"),
    }
}

#[test]
fn parse_logical_and_or() {
    let expr = first_binding_expr("result is a and b or c\n");
    // `or` should be at the top (lower precedence)
    match expr {
        Expression::Binary { op: BinaryOp::Or, .. } => {}
        _ => panic!("expected `or` at top level, got {:?}", expr),
    }
}

// ─── Comments ───────────────────────────────────────────────────────

#[test]
fn parse_with_comments() {
    let program = parse_ok("# this is a comment\nx is 42\n");
    assert_eq!(program.items.len(), 1);
}

#[test]
fn parse_inline_comment() {
    let program = parse_ok("x is 42 # inline comment\n");
    assert_eq!(program.items.len(), 1);
}

// ─── Whitespace Edge Cases ──────────────────────────────────────────

#[test]
fn parse_multiple_blank_lines() {
    let program = parse_ok("x is 1\n\n\ny is 2\n");
    assert_eq!(program.items.len(), 2);
}

#[test]
fn parse_trailing_newlines() {
    let program = parse_ok("x is 42\n\n\n");
    assert_eq!(program.items.len(), 1);
}

// ─── T2.1: Generic Type Parameter Syntax ─────────────────────────────

#[test]
fn parse_enum_with_type_params() {
    let program = parse_ok("enum Option[T]\n    Some(value)\n    None\n");
    match &program.items[0] {
        Item::Type(t) => {
            assert_eq!(t.name, "Option");
            assert_eq!(t.type_params, vec!["T"]);
            assert_eq!(t.variants.len(), 2);
            assert_eq!(t.variants[0].name, "Some");
            assert_eq!(t.variants[1].name, "None");
        }
        _ => panic!("expected type definition"),
    }
}

#[test]
fn parse_enum_with_multiple_type_params() {
    let program = parse_ok("enum Result[T, E]\n    Ok(value)\n    Err(error)\n");
    match &program.items[0] {
        Item::Type(t) => {
            assert_eq!(t.name, "Result");
            assert_eq!(t.type_params, vec!["T", "E"]);
            assert_eq!(t.variants.len(), 2);
        }
        _ => panic!("expected type definition"),
    }
}

#[test]
fn parse_enum_no_type_params() {
    // Enums without type params should still work
    let program = parse_ok("enum Color\n    Red\n    Green\n    Blue\n");
    match &program.items[0] {
        Item::Type(t) => {
            assert_eq!(t.name, "Color");
            assert!(t.type_params.is_empty());
            assert_eq!(t.variants.len(), 3);
        }
        _ => panic!("expected type definition"),
    }
}

#[test]
fn parse_type_annotation_with_type_args() {
    // Type annotation (in a binding): x: Option[Int]
    let program = parse_ok("*foo(x: Option[Int])\n    x\n");
    match &program.items[0] {
        Item::Function(f) => {
            let ann = f.params[0].type_annotation.as_ref().expect("expected annotation");
            assert_eq!(ann.segments, vec!["Option"]);
            assert_eq!(ann.type_args.len(), 1);
            assert_eq!(ann.type_args[0].segments, vec!["Int"]);
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn parse_nested_type_annotation_args() {
    // Map[String, List[Int]]
    let program = parse_ok("*foo(x: Map[String, List[Int]])\n    x\n");
    match &program.items[0] {
        Item::Function(f) => {
            let ann = f.params[0].type_annotation.as_ref().expect("expected annotation");
            assert_eq!(ann.segments, vec!["Map"]);
            assert_eq!(ann.type_args.len(), 2);
            assert_eq!(ann.type_args[0].segments, vec!["String"]);
            assert_eq!(ann.type_args[1].segments, vec!["List"]);
            assert_eq!(ann.type_args[1].type_args.len(), 1);
            assert_eq!(ann.type_args[1].type_args[0].segments, vec!["Int"]);
        }
        _ => panic!("expected function"),
    }
}
