use crate::diagnostics::Diagnostic;
use crate::span::Span;
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    TemplateString(Vec<TemplateFragment>),
    KeywordType,
    KeywordStore,
    KeywordPersist,
    KeywordActor,
    KeywordMatch,
    KeywordFn,
    KeywordAnd,
    KeywordOr,
    KeywordTrue,
    KeywordFalse,
    KeywordIs,
    KeywordIsnt,
    KeywordExtern,
    KeywordUnsafe,
    KeywordAsm,
    KeywordPtr,
    KeywordEnum,
    KeywordErr,
    KeywordReturn,
    KeywordTrait,
    KeywordWith,
    KeywordNone,
    KeywordIf,
    KeywordElif,
    KeywordElse,
    KeywordWhile,
    KeywordFor,
    KeywordIn,
    KeywordBreak,
    KeywordContinue,
    Placeholder(u32),
    Star,
    Ampersand,
    Pipe,
    Caret,
    Tilde,
    ShiftLeft,
    ShiftRight,
    At,
    Question,
    Bang,
    BangBang,
    Colon,
    Comma,
    Dot,
    Plus,
    Minus,
    Slash,
    Percent,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TemplateFragment {
    Literal { value: String, span: Span },
    Expr { source: String, span: Span },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndentStyle {
    Spaces,
    Tabs,
}

pub type LexResult<T> = Result<T, Diagnostic>;

pub fn lex(source: &str) -> LexResult<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut pending = VecDeque::new();
    let mut indent_stack = vec![0usize];
    let mut indent_style_stack: Vec<Option<IndentStyle>> = vec![None];
    let mut line_start = true;
    let mut pos = 0usize;
    let bytes = source.as_bytes();
    let len = bytes.len();

    while pos < len {
        if let Some(tok) = pending.pop_front() {
            tokens.push(tok);
            continue;
        }

        let ch = source[pos..].chars().next().unwrap();
        let ch_len = ch.len_utf8();

        if ch == '\r' {
            pos += ch_len;
            continue;
        }

        if line_start {
            let indent_start = pos;
            let mut indent_width = 0usize;
            let mut saw_space = false;
            let mut saw_tab = false;
            while pos < len {
                let c = source[pos..].chars().next().unwrap();
                if c == ' ' {
                    indent_width += 1;
                    pos += 1;
                    saw_space = true;
                } else if c == '\t' {
                    indent_width += 4;
                    pos += 1;
                    saw_tab = true;
                } else {
                    break;
                }
            }

            if saw_space && saw_tab {
                return Err(
                    Diagnostic::new(
                        "mixed indentation (tabs and spaces) is not allowed",
                        Span::new(indent_start, pos),
                    )
                    .with_help("Choose either tabs or spaces for indentation within the same block."),
                );
            }
            let line_style = if saw_space {
                Some(IndentStyle::Spaces)
            } else if saw_tab {
                Some(IndentStyle::Tabs)
            } else {
                None
            };

            // Peek next significant character
            let next_char = if pos < len {
                source[pos..].chars().next()
            } else {
                None
            };

            if next_char == Some('\n') || next_char == Some('\r') || next_char.is_none() {
                // blank line, indentation resets but we do not emit indent/dedent
            } else {
                let current_indent = *indent_stack.last().unwrap();
                if indent_width > current_indent {
                    let style = line_style.unwrap_or(IndentStyle::Spaces);
                    indent_stack.push(indent_width);
                    indent_style_stack.push(Some(style));
                    pending.push_back(Token::new(
                        TokenKind::Indent,
                        Span::new(indent_start, indent_start),
                    ));
                } else {
                    while indent_width < *indent_stack.last().unwrap() {
                        indent_stack.pop();
                        indent_style_stack.pop();
                        pending.push_back(Token::new(
                            TokenKind::Dedent,
                            Span::new(indent_start, indent_start),
                        ));
                    }
                    if indent_width != *indent_stack.last().unwrap() {
                        return Err(
                            Diagnostic::new(
                                "indentation must align with a previous indent level",
                                Span::new(indent_start, pos),
                            )
                            .with_help(
                                "Use consistent indentation widths for all nested blocks.",
                            ),
                        );
                    }
                    if let Some(Some(expected_style)) = indent_style_stack.last().copied() {
                        if let Some(actual_style) = line_style {
                            if actual_style != expected_style {
                                return Err(
                                    Diagnostic::new(
                                        "mixed indentation (tabs and spaces) is not allowed",
                                        Span::new(indent_start, pos),
                                    )
                                    .with_help("Choose either tabs or spaces for indentation within the same block."),
                                );
                            }
                        }
                    }
                }
            }
            line_start = false;
            continue;
        }

        match ch {
            '\n' => {
                tokens.push(Token::new(
                    TokenKind::Newline,
                    Span::new(pos, pos + ch_len),
                ))
                ;
                pos += ch_len;
                line_start = true;
            }
            '#' => {
                // Skip comment until end of line
                pos += ch_len;
                while pos < len {
                    let c = source[pos..].chars().next().unwrap();
                    if c == '\n' {
                        break;
                    }
                    pos += c.len_utf8();
                }
            }
            ' ' | '\t' => {
                pos += ch_len;
            }
            '0'..='9' => {
                let start = pos;
                pos += ch_len;

                // Check for radix prefix: 0x, 0b, 0o
                if ch == '0' && pos < len {
                    let next = source[pos..].chars().next().unwrap();
                    match next {
                        'x' | 'X' => {
                            pos += 1; // consume 'x'
                            let digit_start = pos;
                            while pos < len {
                                let c = source[pos..].chars().next().unwrap();
                                if c.is_ascii_hexdigit() || c == '_' {
                                    pos += 1;
                                } else {
                                    break;
                                }
                            }
                            let digits: String = source[digit_start..pos].chars().filter(|c| *c != '_').collect();
                            if digits.is_empty() {
                                return Err(Diagnostic::new("hex literal requires at least one digit", Span::new(start, pos)));
                            }
                            let value = i64::from_str_radix(&digits, 16).map_err(|_| {
                                Diagnostic::new("invalid hex literal", Span::new(start, pos))
                            })?;
                            tokens.push(Token::new(TokenKind::Integer(value), Span::new(start, pos)));
                            continue;
                        }
                        'b' | 'B' => {
                            // Disambiguate: 0b"..." is bytes literal starting with 0, 0b1010 is binary
                            if pos + 1 < len {
                                let after_b = source[pos + 1..].chars().next().unwrap();
                                if after_b == '0' || after_b == '1' || after_b == '_' {
                                    pos += 1; // consume 'b'
                                    let digit_start = pos;
                                    while pos < len {
                                        let c = source[pos..].chars().next().unwrap();
                                        if c == '0' || c == '1' || c == '_' {
                                            pos += 1;
                                        } else {
                                            break;
                                        }
                                    }
                                    let digits: String = source[digit_start..pos].chars().filter(|c| *c != '_').collect();
                                    if digits.is_empty() {
                                        return Err(Diagnostic::new("binary literal requires at least one digit", Span::new(start, pos)));
                                    }
                                    let value = i64::from_str_radix(&digits, 2).map_err(|_| {
                                        Diagnostic::new("invalid binary literal", Span::new(start, pos))
                                    })?;
                                    tokens.push(Token::new(TokenKind::Integer(value), Span::new(start, pos)));
                                    continue;
                                }
                            }
                            // Not a binary literal — fall through to normal number parsing
                        }
                        'o' | 'O' => {
                            pos += 1; // consume 'o'
                            let digit_start = pos;
                            while pos < len {
                                let c = source[pos..].chars().next().unwrap();
                                if ('0'..='7').contains(&c) || c == '_' {
                                    pos += 1;
                                } else {
                                    break;
                                }
                            }
                            let digits: String = source[digit_start..pos].chars().filter(|c| *c != '_').collect();
                            if digits.is_empty() {
                                return Err(Diagnostic::new("octal literal requires at least one digit", Span::new(start, pos)));
                            }
                            let value = i64::from_str_radix(&digits, 8).map_err(|_| {
                                Diagnostic::new("invalid octal literal", Span::new(start, pos))
                            })?;
                            tokens.push(Token::new(TokenKind::Integer(value), Span::new(start, pos)));
                            continue;
                        }
                        _ => {}
                    }
                }

                // Regular decimal number (with underscore separator support)
                let mut has_dot = false;
                while pos < len {
                    let c = source[pos..].chars().next().unwrap();
                    if c.is_ascii_digit() || c == '_' {
                        pos += 1;
                    } else if c == '.' && !has_dot {
                        // Peek ahead: if the character after '.' is a digit, it's a float.
                        // Otherwise, it might be a method call like `42.method()`.
                        if pos + 1 < len {
                            let after_dot = source[pos + 1..].chars().next().unwrap();
                            if after_dot.is_ascii_digit() {
                                has_dot = true;
                                pos += 1;
                            } else {
                                break;
                            }
                        } else {
                        // Dot at end of input — treat as method access dot, not decimal point.
                        // `123.` at EOF is an integer `123` followed by a dot token.
                        break;
                    }
                    } else {
                        break;
                    }
                }
                let raw = &source[start..pos];
                let cleaned: String = raw.chars().filter(|c| *c != '_').collect();
                if has_dot {
                    let value = cleaned.parse::<f64>().map_err(|_| {
                        Diagnostic::new("invalid float literal", Span::new(start, pos))
                    })?;
                    tokens.push(Token::new(TokenKind::Float(value), Span::new(start, pos)));
                } else {
                    let value = cleaned.parse::<i64>().map_err(|_| {
                        Diagnostic::new("invalid integer literal", Span::new(start, pos))
                    })?;
                    tokens.push(Token::new(TokenKind::Integer(value), Span::new(start, pos)));
                }
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let start = pos;
                pos += ch_len;
                while pos < len {
                    let c = source[pos..].chars().next().unwrap();
                    if c.is_ascii_alphanumeric() || c == '_' {
                        pos += 1;
                    } else {
                        break;
                    }
                }
                let slice = &source[start..pos];
                if (slice == "b" || slice == "B") && pos < len {
                    if let Some('"') = source[pos..].chars().next() {
                        pos += 1;
                        let mut bytes = Vec::new();
                        let mut closed = false;
                        while pos < len {
                            let c = source[pos..].chars().next().unwrap();
                            if c == '"' {
                                pos += c.len_utf8();
                                closed = true;
                                break;
                            }
                            if c == '\\' {
                                pos += 1;
                                if pos >= len {
                                    return Err(Diagnostic::new(
                                        "unterminated escape sequence",
                                        Span::new(start, pos),
                                    ));
                                }
                                let esc = source[pos..].chars().next().unwrap();
                                let byte = match esc {
                                    'n' => b'\n',
                                    'r' => b'\r',
                                    't' => b'\t',
                                    '0' => b'\0',
                                    '\\' => b'\\',
                                    '"' => b'"',
                                    other => {
                                        return Err(Diagnostic::new(
                                            format!("unknown escape sequence `\\{other}`"),
                                            Span::new(pos - 1, pos + other.len_utf8()),
                                        ).with_help("Valid escapes are: \\n, \\r, \\t, \\0, \\\\\\\", \\\"."));
                                    }
                                };
                                bytes.push(byte);
                                pos += esc.len_utf8();
                                continue;
                            }
                            let mut buf = [0u8; 4];
                            let encoded = c.encode_utf8(&mut buf);
                            bytes.extend_from_slice(encoded.as_bytes());
                            pos += c.len_utf8();
                        }
                        if !closed {
                            return Err(Diagnostic::new(
                                "unterminated bytes literal",
                                Span::new(start, pos),
                            ));
                        }
                        tokens.push(Token::new(TokenKind::Bytes(bytes), Span::new(start, pos)));
                        continue;
                    }
                }
                let kind = match slice {
                    "type" => TokenKind::KeywordType,
                    "store" => TokenKind::KeywordStore,
                    "persist" => TokenKind::KeywordPersist,
                    "actor" => TokenKind::KeywordActor,
                    "match" => TokenKind::KeywordMatch,
                    "fn" => TokenKind::KeywordFn,
                    "and" => TokenKind::KeywordAnd,
                    "or" => TokenKind::KeywordOr,
                    "true" => TokenKind::KeywordTrue,
                    "false" => TokenKind::KeywordFalse,
                    "is" => TokenKind::KeywordIs,
                    "isnt" => TokenKind::KeywordIsnt,
                    "extern" => TokenKind::KeywordExtern,
                    "unsafe" => TokenKind::KeywordUnsafe,
                    "asm" => TokenKind::KeywordAsm,
                    "ptr" => TokenKind::KeywordPtr,
                    "enum" => TokenKind::KeywordEnum,
                    "err" => TokenKind::KeywordErr,
                    "return" => TokenKind::KeywordReturn,
                    "trait" => TokenKind::KeywordTrait,
                    "with" => TokenKind::KeywordWith,
                    "none" => TokenKind::KeywordNone,
                    "if" => TokenKind::KeywordIf,
                    "elif" => TokenKind::KeywordElif,
                    "else" => TokenKind::KeywordElse,
                    "while" => TokenKind::KeywordWhile,
                    "for" => TokenKind::KeywordFor,
                    "in" => TokenKind::KeywordIn,
                    "break" => TokenKind::KeywordBreak,
                    "continue" => TokenKind::KeywordContinue,
                    _ => TokenKind::Identifier(slice.to_string()),
                };
                tokens.push(Token::new(kind, Span::new(start, pos)));
            }
            '"' => {
                let start = pos;
                pos += ch_len;
                let mut closed = false;
                let mut value = String::new();
                while pos < len {
                    let c = source[pos..].chars().next().unwrap();
                    if c == '"' {
                        pos += c.len_utf8();
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        pos += 1;
                        if pos >= len {
                            return Err(Diagnostic::new(
                                "unterminated escape sequence",
                                Span::new(start, pos),
                            ));
                        }
                        let esc = source[pos..].chars().next().unwrap();
                        let ch = match esc {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            '0' => '\0',
                            '\\' => '\\',
                            '\'' => '\'',
                            '"' => '"',
                            other => {
                                return Err(Diagnostic::new(
                                    format!("unknown escape sequence `\\{other}`"),
                                    Span::new(pos - 1, pos + other.len_utf8()),
                                ).with_help("Valid escapes are: \\n, \\r, \\t, \\0, \\\\, \\', \\\"."));
                            }
                        };
                        value.push(ch);
                        pos += esc.len_utf8();
                    } else {
                        value.push(c);
                        pos += c.len_utf8();
                    }
                }
                if !closed {
                    return Err(Diagnostic::new(
                        "unterminated string literal",
                        Span::new(start, pos),
                    ));
                }
                tokens.push(Token::new(TokenKind::String(value), Span::new(start, pos)));
            }
            '\'' => {
                let start = pos;
                pos += ch_len;
                let span_start = start;
                let mut fragments = Vec::new();
                let mut literal = String::new();
                let mut has_expr = false;
                let mut literal_start = pos;
                let mut closed = false;
                while pos < len {
                    let c = source[pos..].chars().next().unwrap();
                    if c == '\'' {
                        pos += c.len_utf8();
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        pos += 1;
                        if pos >= len {
                            return Err(Diagnostic::new(
                                "unterminated escape sequence",
                                Span::new(start, pos),
                            ));
                        }
                        let esc = source[pos..].chars().next().unwrap();
                        match esc {
                            '{' | '}' | '\'' | '"' | '\\' => {
                                literal.push(esc);
                                pos += esc.len_utf8();
                            }
                            'n' => {
                                literal.push('\n');
                                pos += 1;
                            }
                            'r' => {
                                literal.push('\r');
                                pos += 1;
                            }
                            't' => {
                                literal.push('\t');
                                pos += 1;
                            }
                            '0' => {
                                literal.push('\0');
                                pos += 1;
                            }
                            other => {
                                return Err(Diagnostic::new(
                                    format!("unknown escape sequence `\\{other}`"),
                                    Span::new(pos - 1, pos + other.len_utf8()),
                                ).with_help("Valid escapes are: \\n, \\r, \\t, \\0, \\\\, \\{, \\}, \\', \\\"."));
                            }
                        }
                        continue;
                    }
                    if c == '{' {
                        if !literal.is_empty() {
                            let literal_value = std::mem::take(&mut literal);
                            fragments.push(TemplateFragment::Literal {
                                value: literal_value,
                                span: Span::new(literal_start, pos),
                            });
                        }
                        let (expr_source, expr_span, new_pos) =
                            lex_interpolation_expression(source, pos, len)?;
                        has_expr = true;
                        fragments.push(TemplateFragment::Expr {
                            source: expr_source,
                            span: expr_span,
                        });
                        pos = new_pos;
                        literal_start = pos;
                        continue;
                    }
                    literal.push(c);
                    pos += c.len_utf8();
                }
                if !closed {
                    return Err(Diagnostic::new(
                        "unterminated string literal",
                        Span::new(start, pos),
                    ));
                }
                if !literal.is_empty() {
                    let literal_value = std::mem::take(&mut literal);
                    let literal_end = if closed {
                        pos.saturating_sub(1)
                    } else {
                        pos
                    };
                    fragments.push(TemplateFragment::Literal {
                        value: literal_value,
                        span: Span::new(literal_start, literal_end),
                    });
                }
                if !has_expr {
                    let combined = fragments
                        .into_iter()
                        .map(|fragment| match fragment {
                            TemplateFragment::Literal { value, .. } => value,
                            _ => unreachable!(),
                        })
                        .collect::<String>();
                    tokens.push(Token::new(TokenKind::String(combined), Span::new(start, pos)));
                } else {
                    tokens.push(Token::new(
                        TokenKind::TemplateString(fragments),
                        Span::new(span_start, pos),
                    ));
                }
            }
            '(' => {
                tokens.push(Token::new(TokenKind::LParen, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            ')' => {
                tokens.push(Token::new(TokenKind::RParen, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '[' => {
                tokens.push(Token::new(TokenKind::LBracket, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            ']' => {
                tokens.push(Token::new(TokenKind::RBracket, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            ',' => {
                tokens.push(Token::new(TokenKind::Comma, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            ':' => {
                tokens.push(Token::new(TokenKind::Colon, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '.' => {
                tokens.push(Token::new(TokenKind::Dot, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '+' => {
                tokens.push(Token::new(TokenKind::Plus, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '-' => {
                tokens.push(Token::new(TokenKind::Minus, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '*' => {
                tokens.push(Token::new(TokenKind::Star, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '/' => {
                tokens.push(Token::new(TokenKind::Slash, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '%' => {
                tokens.push(Token::new(TokenKind::Percent, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '>' => {
                let start = pos;
                pos += ch_len;
                if pos < len {
                    if let Some('>') = source[pos..].chars().next() {
                        pos += 1;
                        tokens.push(Token::new(TokenKind::ShiftRight, Span::new(start, pos)));
                        continue;
                    } else if let Some('=') = source[pos..].chars().next() {
                        pos += 1;
                        tokens.push(Token::new(TokenKind::GreaterEq, Span::new(start, pos)));
                        continue;
                    }
                }
                tokens.push(Token::new(TokenKind::Greater, Span::new(start, pos)));
            }
            '<' => {
                let start = pos;
                pos += ch_len;
                if pos < len {
                    if let Some('<') = source[pos..].chars().next() {
                        pos += 1;
                        tokens.push(Token::new(TokenKind::ShiftLeft, Span::new(start, pos)));
                        continue;
                    } else if let Some('=') = source[pos..].chars().next() {
                        pos += 1;
                        tokens.push(Token::new(TokenKind::LessEq, Span::new(start, pos)));
                        continue;
                    }
                }
                tokens.push(Token::new(TokenKind::Less, Span::new(start, pos)));
            }
            '&' => {
                tokens.push(Token::new(TokenKind::Ampersand, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '|' => {
                tokens.push(Token::new(TokenKind::Pipe, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '^' => {
                tokens.push(Token::new(TokenKind::Caret, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '~' => {
                tokens.push(Token::new(TokenKind::Tilde, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '?' => {
                tokens.push(Token::new(TokenKind::Question, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '!' => {
                let start = pos;
                pos += ch_len;
                if pos < len {
                    if let Some('!') = source[pos..].chars().next() {
                        pos += 1;
                        tokens.push(Token::new(TokenKind::BangBang, Span::new(start, pos)));
                        continue;
                    }
                }
                tokens.push(Token::new(TokenKind::Bang, Span::new(start, pos)));
            }
            '$' => {
                let start = pos;
                pos += ch_len;
                let mut value: u32 = 0;
                let mut has_digits = false;
                while pos < len {
                    let c = source[pos..].chars().next().unwrap();
                    if c.is_ascii_digit() {
                        has_digits = true;
                        let digit = (c as u8 - b'0') as u32;
                        value = value.saturating_mul(10).saturating_add(digit);
                        pos += 1;
                    } else {
                        break;
                    }
                }
                let placeholder_index = if has_digits { value } else { 0 };
                tokens.push(Token::new(
                    TokenKind::Placeholder(placeholder_index),
                    Span::new(start, pos),
                ));
            }
            '@' => {
                tokens.push(Token::new(TokenKind::At, Span::new(pos, pos + ch_len)));
                pos += ch_len;
            }
            '=' => {
                let start = pos;
                pos += ch_len;
                if pos < len {
                    if let Some('=') = source[pos..].chars().next() {
                        pos += 1;
                        return Err(Diagnostic::new(
                            "unexpected `==`; use `.equals()` for equality comparison or `is` for binding",
                            Span::new(start, pos),
                        ));
                    }
                }
                return Err(Diagnostic::new(
                    "unexpected `=`; use `is` for binding",
                    Span::new(start, pos),
                ));
            }
            other => {
                return Err(
                    Diagnostic::new(format!("unexpected character `{other}`"), Span::new(pos, pos + ch_len))
                );
            }
        }
    }

    if !matches!(tokens.last(), Some(Token { kind: TokenKind::Newline, .. })) {
        tokens.push(Token::new(
            TokenKind::Newline,
            Span::new(len, len),
        ));
    }

    while indent_stack.len() > 1 {
        indent_stack.pop();
        indent_style_stack.pop();
        tokens.push(Token::new(TokenKind::Dedent, Span::new(len, len)));
    }

    tokens.push(Token::new(TokenKind::Eof, Span::new(len, len)));

    Ok(tokens)
}

fn lex_interpolation_expression(
    source: &str,
    brace_pos: usize,
    len: usize,
) -> LexResult<(String, Span, usize)> {
    let mut pos = brace_pos;
    let brace_char = source[pos..].chars().next().unwrap();
    debug_assert_eq!(brace_char, '{');
    pos += brace_char.len_utf8();
    let mut depth = 1usize;
    let expr_start = pos;
    let mut expr = String::new();
    while pos < len {
        let c = source[pos..].chars().next().unwrap();
        if c == '\\' {
            expr.push(c);
            pos += c.len_utf8();
            if pos < len {
                let next = source[pos..].chars().next().unwrap();
                expr.push(next);
                pos += next.len_utf8();
            }
            continue;
        }
        if c == '{' {
            depth += 1;
            expr.push(c);
            pos += c.len_utf8();
            continue;
        }
        if c == '}' {
            depth -= 1;
            if depth == 0 {
                let closing_len = c.len_utf8();
                let braces_span = Span::new(brace_pos, pos + closing_len);
                if expr.trim().is_empty() {
                    return Err(
                        Diagnostic::new("empty interpolation expression", braces_span)
                            .with_help("Provide an expression between `{` and `}`."),
                    );
                }
                let expr_span = Span::new(expr_start, pos);
                return Ok((expr, expr_span, pos + closing_len));
            }
            expr.push(c);
            pos += c.len_utf8();
            continue;
        }
        expr.push(c);
        pos += c.len_utf8();
    }
    Err(
        Diagnostic::new(
            "unterminated interpolation expression",
            Span::new(brace_pos, len),
        )
        .with_help("Add a closing `}` to finish this interpolation."),
    )
}
