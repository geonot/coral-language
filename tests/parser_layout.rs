use coralc::ast::Item;
use coralc::lexer::{self, Token, TokenKind};
use coralc::parser::Parser;
use coralc::span::Span;

#[test]
fn parses_store_with_blank_lines_and_indents() {
    let source = "store Ledger\n    balance is 0\n\n    reserve is 1\n\n    nested is 2\n\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Store(store) => {
            assert_eq!(store.fields.len(), 3);
            assert_eq!(store.fields[0].name, "balance");
            assert_eq!(store.fields[1].name, "reserve");
            assert_eq!(store.fields[2].name, "nested");
        }
        other => panic!("expected store definition, got {:?}", other),
    }
}

#[test]
fn ignores_leading_indent_before_toplevel_binding() {
    let source = "    \nvalue is 1\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Binding(binding) => {
            assert_eq!(binding.name, "value");
        }
        other => panic!("expected binding, got {:?}", other),
    }
}

#[test]
fn reports_missing_block_indent_with_help() {
    let source = "*main()\nvalue is 1\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let error = parser.parse().expect_err("parser should emit diagnostics");
    assert_eq!(error.message, "expected indentation for block");
    assert_eq!(
        error.help.as_deref(),
        Some("Indent block contents with spaces or a tab")
    );
    let newline = source.find('\n').unwrap() + 1;
    assert_eq!(error.span.start, newline);
}

#[test]
fn reports_missing_block_dedent() {
    let source = "*main()\n    value is 1";
    let mut tokens = lexer::lex(source).expect("lexing failed");
    if let Some(index) = tokens
        .iter()
        .rposition(|token| matches!(token.kind, TokenKind::Dedent))
    {
        tokens.remove(index);
    }
    let parser = Parser::new(tokens, source.len());
    let error = parser.parse().expect_err("parser should emit diagnostics");
    assert_eq!(error.message, "missing dedent to close block");
    assert_eq!(
        error.help.as_deref(),
        Some("Add a matching dedent/outdent to close this block."),
    );
}

#[test]
fn reports_unexpected_dedent_when_tokens_underflow() {
    let source = "value is 1\n";
    let mut tokens = lexer::lex(source).expect("lexing failed");
    tokens.insert(0, Token::new(TokenKind::Dedent, Span::new(0, 0)));
    let parser = Parser::new(tokens, source.len());
    let error = parser.parse().expect_err("parser should emit diagnostics");
    assert_eq!(error.message, "unexpected dedent");
    assert_eq!(
        error.help.as_deref(),
        Some("Remove the extra outdent or ensure there's a matching indented block."),
    );
}

#[test]
fn reports_unexpected_dedent_inside_block() {
    let source = "*main()\n    value is 1\n  value is 2\n";
    let error = lexer::lex(source).expect_err("lexer should reject inconsistent dedent");
    assert_eq!(
        error.message,
        "indentation must align with a previous indent level",
    );
    assert_eq!(
        error.help.as_deref(),
        Some("Use consistent indentation widths for all nested blocks."),
    );
}

#[test]
fn rejects_mixed_tabs_and_spaces_in_indent() {
    let source = "*main()\n\tvalue is 1\n    value is 2\n";
    let error = lexer::lex(source).expect_err("lexer should reject mixed indentation");
    assert_eq!(
        error.message,
        "mixed indentation (tabs and spaces) is not allowed",
    );
    assert_eq!(
        error.help.as_deref(),
        Some("Choose either tabs or spaces for indentation within the same block."),
    );
}

#[test]
fn parses_persist_store_syntax() {
    let source = "persist store Account\n    id is 0\n    balance is 0.0\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Store(store) => {
            assert_eq!(store.name, "Account");
            assert!(store.is_persistent, "should be marked as persistent");
            assert!(!store.is_actor, "should not be an actor");
            assert_eq!(store.fields.len(), 2);
            assert_eq!(store.fields[0].name, "id");
            assert_eq!(store.fields[1].name, "balance");
        }
        other => panic!("expected store definition, got {:?}", other),
    }
}

#[test]
fn parses_regular_store_as_non_persistent() {
    let source = "store Ephemeral\n    data is 0\n";
    let tokens = lexer::lex(source).expect("lexing failed");
    let parser = Parser::new(tokens, source.len());
    let program = parser.parse().expect("parsing failed");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Store(store) => {
            assert_eq!(store.name, "Ephemeral");
            assert!(
                !store.is_persistent,
                "regular store should not be persistent"
            );
            assert!(!store.is_actor);
        }
        other => panic!("expected store definition, got {:?}", other),
    }
}
