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
