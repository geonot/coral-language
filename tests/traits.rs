//! Tests for Coral's trait/mixin system.
//!
//! Traits provide reusable behavior that can be composed into types and stores.
//! Coral uses `*` prefix for function/method definitions.

use coralc::ast::*;
use coralc::lexer;
use coralc::parser;
use coralc::semantic;

#[test]
fn parse_simple_trait_definition() {
    let source = "trait Printable\n    *to_string()\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse simple trait");
    assert_eq!(program.items.len(), 1);
    
    match &program.items[0] {
        Item::TraitDefinition(trait_def) => {
            assert_eq!(trait_def.name, "Printable");
            assert_eq!(trait_def.required_traits.len(), 0);
            assert_eq!(trait_def.methods.len(), 1);
            assert_eq!(trait_def.methods[0].name, "to_string");
            assert!(trait_def.methods[0].body.is_none(), "method should be abstract");
        }
        _ => panic!("expected trait definition"),
    }
}

#[test]
fn parse_trait_with_default_implementation() {
    let source = "trait Logger\n    *log(message: String)\n        print(message)\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse trait with default");
    
    match &program.items[0] {
        Item::TraitDefinition(trait_def) => {
            assert_eq!(trait_def.name, "Logger");
            assert_eq!(trait_def.methods.len(), 1);
            assert_eq!(trait_def.methods[0].name, "log");
            assert!(trait_def.methods[0].body.is_some(), "method should have default impl");
        }
        _ => panic!("expected trait definition"),
    }
}

#[test]
fn parse_trait_with_dependencies() {
    let source = "trait Serializable with Printable\n    *serialize()\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse trait with dependencies");
    
    match &program.items[0] {
        Item::TraitDefinition(trait_def) => {
            assert_eq!(trait_def.name, "Serializable");
            assert_eq!(trait_def.required_traits, vec!["Printable"]);
            assert_eq!(trait_def.methods.len(), 1);
        }
        _ => panic!("expected trait definition"),
    }
}

#[test]
fn parse_trait_with_multiple_dependencies() {
    let source = "trait Advanced with Printable, Comparable, Serializable\n    *process()\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse trait with multiple deps");
    
    match &program.items[0] {
        Item::TraitDefinition(trait_def) => {
            assert_eq!(trait_def.name, "Advanced");
            assert_eq!(
                trait_def.required_traits,
                vec!["Printable", "Comparable", "Serializable"]
            );
        }
        _ => panic!("expected trait definition"),
    }
}

#[test]
fn parse_type_with_trait() {
    // Use Coral syntax: fields without type annotations, just identifiers
    let source = "type User with Printable\n    name\n    age\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse type with trait");
    
    match &program.items[0] {
        Item::Type(type_def) => {
            assert_eq!(type_def.name, "User");
            assert_eq!(type_def.with_traits, vec!["Printable"]);
            assert_eq!(type_def.fields.len(), 2);
        }
        _ => panic!("expected type definition"),
    }
}

#[test]
fn parse_store_with_trait() {
    let source = "store Counter with Resettable\n    count ? 0\n    *increment()\n        self.count is self.count + 1\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse store with trait");
    
    match &program.items[0] {
        Item::Store(store_def) => {
            assert_eq!(store_def.name, "Counter");
            assert_eq!(store_def.with_traits, vec!["Resettable"]);
        }
        _ => panic!("expected store definition"),
    }
}

#[test]
fn parse_type_with_multiple_traits() {
    let source = "type Document with Printable, Serializable, Comparable\n    title\n    content\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse type with multiple traits");
    
    match &program.items[0] {
        Item::Type(type_def) => {
            assert_eq!(type_def.name, "Document");
            assert_eq!(
                type_def.with_traits,
                vec!["Printable", "Serializable", "Comparable"]
            );
        }
        _ => panic!("expected type definition"),
    }
}

#[test]
fn parse_trait_with_multiple_methods() {
    let source = "trait Comparable\n    *compare(other)\n    *equals(other)\n        self.compare(other) is 0\n    *less_than(other)\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse trait with multiple methods");
    
    match &program.items[0] {
        Item::TraitDefinition(trait_def) => {
            assert_eq!(trait_def.name, "Comparable");
            assert_eq!(trait_def.methods.len(), 3);
            assert_eq!(trait_def.methods[0].name, "compare");
            assert!(trait_def.methods[0].body.is_none());
            assert_eq!(trait_def.methods[1].name, "equals");
            assert!(trait_def.methods[1].body.is_some());
            assert_eq!(trait_def.methods[2].name, "less_than");
            assert!(trait_def.methods[2].body.is_none());
        }
        _ => panic!("expected trait definition"),
    }
}

#[test]
fn parse_empty_trait() {
    let source = "trait Marker\n";
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse empty trait");
    
    match &program.items[0] {
        Item::TraitDefinition(trait_def) => {
            assert_eq!(trait_def.name, "Marker");
            assert!(trait_def.methods.is_empty());
            assert!(trait_def.required_traits.is_empty());
        }
        _ => panic!("expected trait definition"),
    }
}

// ============================================================
// Semantic validation tests
// ============================================================

fn analyze(source: &str) -> Result<coralc::semantic::SemanticModel, coralc::diagnostics::Diagnostic> {
    let tokens = lexer::lex(source).expect("should lex");
    let parser = parser::Parser::new(tokens, source.len());
    let program = parser.parse().expect("should parse");
    semantic::analyze(program)
}

#[test]
fn semantic_unknown_trait_error() {
    let source = "type User with UnknownTrait\n    name\n";
    let result = analyze(source);
    assert!(result.is_err(), "should fail for unknown trait");
    let err = result.unwrap_err();
    assert!(err.message.contains("unknown trait"), "error: {}", err.message);
}

#[test]
fn semantic_store_unknown_trait_error() {
    let source = "store Counter with NonExistent\n    count ? 0\n";
    let result = analyze(source);
    assert!(result.is_err(), "should fail for unknown trait");
    let err = result.unwrap_err();
    assert!(err.message.contains("unknown trait"), "error: {}", err.message);
}

#[test]
fn semantic_missing_required_method() {
    let source = r#"trait Printable
    *to_string()

type User with Printable
    name
"#;
    let result = analyze(source);
    assert!(result.is_err(), "should fail for missing required method");
    let err = result.unwrap_err();
    assert!(err.message.contains("does not implement required method"), "error: {}", err.message);
    assert!(err.message.contains("to_string"), "error should mention method name: {}", err.message);
}

#[test]
fn semantic_trait_with_default_passes() {
    let source = r#"trait Logger
    *log(msg)
        print(msg)

type App with Logger
    name
"#;
    let result = analyze(source);
    assert!(result.is_ok(), "should pass when trait has default impl: {:?}", result);
}

#[test]
fn semantic_required_method_implemented() {
    let source = r#"trait Printable
    *to_string()

type User with Printable
    name
    
    *to_string()
        name
"#;
    let result = analyze(source);
    assert!(result.is_ok(), "should pass when required method is implemented: {:?}", result);
}

#[test]
fn semantic_missing_trait_dependency() {
    let source = r#"trait Base
    *init()

trait Derived with Base
    *process()

type Worker with Derived
    id
"#;
    let result = analyze(source);
    assert!(result.is_err(), "should fail when trait dependency is missing");
    let err = result.unwrap_err();
    assert!(err.message.contains("requires"), "error: {}", err.message);
    assert!(err.message.contains("Base"), "error should mention required trait: {}", err.message);
}

#[test]
fn semantic_trait_dependency_satisfied() {
    let source = r#"trait Base
    *init()
        42

trait Derived with Base
    *process()
        init()

type Worker with Base, Derived
    id
"#;
    let result = analyze(source);
    assert!(result.is_ok(), "should pass when trait dependencies are satisfied: {:?}", result);
}

#[test]
fn semantic_marker_trait_passes() {
    let source = r#"trait Marker

type Token with Marker
    value
"#;
    let result = analyze(source);
    assert!(result.is_ok(), "marker trait (no methods) should pass: {:?}", result);
}

#[test]
fn semantic_override_warning() {
    let source = r#"trait Defaulted
    *action()
        42

type Custom with Defaulted
    val
    
    *action()
        99
"#;
    let result = analyze(source);
    assert!(result.is_ok(), "overriding default should be valid: {:?}", result);
    let model = result.unwrap();
    assert!(!model.warnings.is_empty(), "should have warning for override");
    assert!(model.warnings[0].message.contains("overrides"), "warning: {}", model.warnings[0].message);
}

#[test]
fn semantic_invalid_trait_dependency() {
    let source = r#"trait Broken with NonExistent
    *method()
"#;
    let result = analyze(source);
    assert!(result.is_err(), "should fail when trait depends on unknown trait");
    let err = result.unwrap_err();
    assert!(err.message.contains("unknown trait"), "error: {}", err.message);
}
