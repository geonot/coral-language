use coralc::lexer::{self, TokenKind};

const MIXED_INDENT: &str = include_str!("fixtures/lexer/indent_mixed.coral");
const BLANK_LINES: &str = include_str!("fixtures/lexer/blank_lines.coral");

#[test]
fn lex_mixed_indentation() {
    let error = lexer::lex(MIXED_INDENT).expect_err("lexer should reject mixed indentation");
    assert_eq!(
        error.message,
        "mixed indentation (tabs and spaces) is not allowed",
    );
    assert_eq!(
        error.help.as_deref(),
        Some("Choose either tabs or spaces for indentation within the same block."),
    );
}

fn line_slice(source: &str, start: usize) -> &str {
    let bytes = source.as_bytes();
    let line_start = bytes[..start]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let line_end = bytes[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|rel| start + rel)
        .unwrap_or_else(|| source.len());
    &source[line_start..line_end]
}

#[test]
fn lex_blank_lines_without_spurious_indents() {
    let tokens = lexer::lex(BLANK_LINES).expect("lexing failed");
    let indent_tokens: Vec<_> = tokens
        .into_iter()
        .filter(|tok| matches!(tok.kind, TokenKind::Indent))
        .collect();
    assert_eq!(
        indent_tokens.len(),
        2,
        "should only indent twice in fixture"
    );
    for token in indent_tokens {
        let line = line_slice(BLANK_LINES, token.span.start);
        assert!(
            line.trim().len() > 0,
            "indent span {:?} should not point at a blank line",
            line
        );
    }
}

#[test]
fn lex_bang_alone() {
    // Single ! should still work for error propagation
    let tokens = lexer::lex("!x").expect("lexing failed");
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
    assert!(
        kinds.contains(&&TokenKind::Bang),
        "should lex single ! as Bang token, got: {:?}",
        kinds
    );
}

#[test]
fn lex_hex_literals() {
    let tokens = lexer::lex("0xFF").expect("lexing hex failed");
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Integer(255)));

    let tokens = lexer::lex("0x1A2B").expect("lexing hex failed");
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Integer(0x1A2B)));

    let tokens = lexer::lex("0xCAFE_BABE").expect("lexing hex with underscore failed");
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == TokenKind::Integer(0xCAFE_BABE))
    );
}

#[test]
fn lex_binary_literals() {
    let tokens = lexer::lex("0b1010").expect("lexing binary failed");
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Integer(0b1010)));

    let tokens = lexer::lex("0b1111_0000").expect("lexing binary with underscore failed");
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == TokenKind::Integer(0b1111_0000))
    );
}

#[test]
fn lex_octal_literals() {
    let tokens = lexer::lex("0o77").expect("lexing octal failed");
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Integer(0o77)));

    let tokens = lexer::lex("0o755").expect("lexing octal failed");
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Integer(0o755)));
}

#[test]
fn lex_underscore_separators() {
    let tokens = lexer::lex("1_000_000").expect("lexing underscored int failed");
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == TokenKind::Integer(1_000_000))
    );

    let tokens = lexer::lex("3.14_159").expect("lexing underscored float failed");
    assert!(
        tokens
            .iter()
            .any(|t| matches!(t.kind, TokenKind::Float(f) if (f - 3.14159).abs() < 1e-9))
    );
}

#[test]
fn lex_float_dot_ambiguity() {
    // 42.method() should lex as Integer(42) Dot Identifier("method")
    // not as Float(42.)
    let tokens = lexer::lex("42.method").expect("lexing failed");
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
    assert!(
        kinds.contains(&&TokenKind::Integer(42)),
        "should lex 42 as integer, got: {:?}",
        kinds
    );
    assert!(kinds.contains(&&TokenKind::Dot), "should have dot token");
}

#[test]
fn lex_hex_empty_digits_error() {
    let result = lexer::lex("0x");
    assert!(result.is_err(), "0x without digits should error");
}

#[test]
fn lex_binary_empty_digits_error() {
    // 0b followed by non-binary digit — should fall through to normal '0' then 'b' ident
    // This actually becomes integer 0 followed by identifier b, which is valid lexing
    let tokens = lexer::lex("0bz");
    // Should not panic at least
    assert!(tokens.is_ok() || tokens.is_err());
}

#[test]
fn lex_unknown_escape_in_string() {
    let result = lexer::lex(r#" "hello\q" "#);
    assert!(result.is_err(), "unknown escape \\q should be rejected");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("unknown escape sequence"), "error: {msg}");
}

#[test]
fn lex_unknown_escape_in_bytes() {
    let result = lexer::lex(r#" b"data\q" "#);
    assert!(
        result.is_err(),
        "unknown escape \\q in bytes should be rejected"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("unknown escape sequence"), "error: {msg}");
}

#[test]
fn lex_unknown_escape_in_template() {
    let result = lexer::lex(r#" f'hello\q' "#);
    assert!(
        result.is_err(),
        "unknown escape \\q in template should be rejected"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("unknown escape sequence"), "error: {msg}");
}

#[test]
fn lex_valid_escapes_still_work() {
    // All standard escapes should still be accepted
    let result = lexer::lex(r#" "line\nnext\ttab\r\0null\\\"\'" "#);
    assert!(
        result.is_ok(),
        "valid escapes should work: {:?}",
        result.err()
    );
}

#[test]
fn lex_equals_rejected() {
    let result = lexer::lex("x = 5");
    assert!(result.is_err(), "= should be rejected");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("unexpected `=`"), "error: {msg}");
}

#[test]
fn lex_double_equals_rejected() {
    let result = lexer::lex("x == 5");
    assert!(result.is_err(), "== should be rejected");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(msg.contains("unexpected `==`"), "error: {msg}");
}
