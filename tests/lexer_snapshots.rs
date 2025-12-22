use coralc::lexer;
use insta::assert_snapshot;

const MIXED_INDENT: &str = include_str!("fixtures/lexer/indent_mixed.coral");
const BLANK_LINES: &str = include_str!("fixtures/lexer/blank_lines.coral");

fn snapshot_tokens(source: &str) -> String {
    let tokens = lexer::lex(source).expect("lexing failed");
    tokens
        .iter()
        .map(|token| format!("{:?}@{}..{}", token.kind, token.span.start, token.span.end))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn snapshot_indent_mixed_tokens() {
    match lexer::lex(MIXED_INDENT) {
        Ok(_) => panic!("mixed indent fixture should now fail to lex"),
        Err(diag) => {
            assert_snapshot!(
                "indent_mixed_tokens",
                format!("{} @ {}..{}", diag.message, diag.span.start, diag.span.end)
            );
        }
    }
}

#[test]
fn snapshot_blank_line_tokens() {
    assert_snapshot!("blank_line_tokens", snapshot_tokens(BLANK_LINES));
}
