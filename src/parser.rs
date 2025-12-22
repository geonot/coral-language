use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::lexer::{self, Token, TokenKind, TemplateFragment};
use crate::span::Span;

pub type ParseResult<T> = Result<T, Diagnostic>;

pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
    source_len: usize,
    pending_error: Option<Diagnostic>,
    layout_depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, source_len: usize) -> Self {
        Self {
            tokens,
            index: 0,
            source_len,
            pending_error: None,
            layout_depth: 0,
        }
    }

    pub fn parse(mut self) -> ParseResult<Program> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.check(TokenKind::Eof) {
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                continue;
            }
            let item = self.parse_item()?;
            items.push(item);
            self.skip_newlines();
        }
        let program = Program::new(items, Span::new(0, self.source_len));
        if let Some(error) = self.pending_error {
            Err(error)
        } else {
            Ok(program)
        }
    }

    pub fn parse_inline_expression(mut self) -> ParseResult<Expression> {
        self.skip_newlines();
        let expr = self.parse_expression()?;
        self.skip_newlines();
        if !self.check(TokenKind::Eof) {
            Err(self.error_here("unexpected tokens after expression"))
        } else if let Some(error) = self.pending_error {
            Err(error)
        } else {
            Ok(expr)
        }
    }

    fn parse_item(&mut self) -> ParseResult<Item> {
        match self.peek_kind() {
            TokenKind::KeywordType => self.parse_type_def().map(Item::Type),
            TokenKind::KeywordStore => self.parse_store_def().map(Item::Store),
            TokenKind::KeywordActor => self.parse_actor_def().map(Item::Store),
            TokenKind::Star => self.parse_function(FunctionKind::Free).map(Item::Function),
            TokenKind::BangBang => self.parse_taxonomy_node().map(Item::Taxonomy),
            TokenKind::KeywordMatch => self.parse_expression().map(Item::Expression),
            TokenKind::KeywordExtern => self.parse_extern_function().map(Item::ExternFunction),
            TokenKind::Identifier(_) => {
                if self.peek_is_binding() {
                    self.parse_binding().map(Item::Binding)
                } else {
                    self.parse_expression().map(Item::Expression)
                }
            }
            _ => Err(self.error_here("unexpected token at top-level")),
        }
    }

    fn parse_binding(&mut self) -> ParseResult<Binding> {
        let (name, name_span) = self.consume_identifier()?;
        let type_annotation = self.parse_optional_type_annotation()?;
        self.expect(TokenKind::KeywordIs, "expected `is` in binding")?;
        self.skip_newlines();
        let value = self.parse_expression()?;
        let mut span = name_span;
        if let Some(annotation) = &type_annotation {
            span = span.join(annotation.span);
        }
        span = span.join(value.span());
        Ok(Binding { name, type_annotation, value, span })
    }

    fn parse_type_def(&mut self) -> ParseResult<TypeDefinition> {
        let start = self.advance().span;
        let (name, name_span) = self.consume_identifier()?;
        let (fields, methods, end_span) = self.parse_composite_body()?;
        let span = start.join(name_span).join(end_span);
        Ok(TypeDefinition {
            name,
            fields,
            methods,
            span,
        })
    }

    fn parse_store_def(&mut self) -> ParseResult<StoreDefinition> {
        let start = self.advance().span;
        let is_actor = self.matches(TokenKind::KeywordActor);
        let (name, name_span) = self.consume_identifier()?;
        let (fields, methods, end_span) = self.parse_composite_body()?;
        let span = start.join(name_span).join(end_span);
        Ok(StoreDefinition {
            name,
            fields,
            methods,
            is_actor,
            span,
        })
    }

    fn parse_actor_def(&mut self) -> ParseResult<StoreDefinition> {
        let start = self.advance().span; // consume `actor`
        let (name, name_span) = self.consume_identifier()?;
        let (fields, methods, end_span) = self.parse_composite_body()?;
        let span = start.join(name_span).join(end_span);
        Ok(StoreDefinition {
            name,
            fields,
            methods,
            is_actor: true,
            span,
        })
    }

    fn parse_optional_type_annotation(&mut self) -> ParseResult<Option<TypeAnnotation>> {
        if !self.matches(TokenKind::Colon) {
            return Ok(None);
        }
        let annotation = self.parse_type_annotation()?;
        Ok(Some(annotation))
    }

    fn parse_type_annotation(&mut self) -> ParseResult<TypeAnnotation> {
        let (first, mut span) = self.consume_type_identifier()?;
        let mut segments = vec![first];
        while self.matches(TokenKind::Dot) {
            let (segment, segment_span) = self.consume_type_identifier()?;
            span = span.join(segment_span);
            segments.push(segment);
        }
        Ok(TypeAnnotation { segments, span })
    }

    fn parse_taxonomy_node(&mut self) -> ParseResult<TaxonomyNode> {
        let start_token = self.advance();
        let (name, name_span) = self.consume_identifier()?;
        self.expect(TokenKind::Newline, "expected newline after taxonomy name")?;
        let body_start = match self.consume_indent_with_recovery(
            "expected indented taxonomy body",
            "Indent taxonomy children or bindings",
        ) {
            Some(span) => span,
            None => {
                return Ok(TaxonomyNode {
                    name,
                    children: Vec::new(),
                    bindings: Vec::new(),
                    span: start_token.span.join(name_span),
                });
            }
        };
        let mut children = Vec::new();
        let mut bindings = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(body_start, "missing dedent to close taxonomy body");
                break;
            }
            match self.peek_kind() {
                TokenKind::BangBang => {
                    let node = self.parse_taxonomy_node()?;
                    children.push(node);
                }
                TokenKind::Identifier(_) if self.peek_is_binding() => {
                    let binding = self.parse_binding()?;
                    bindings.push(binding);
                }
                _ => return Err(self.error_here("unexpected token in taxonomy body")),
            }
        }
        let span = start_token.span.join(self.previous_span());
        Ok(TaxonomyNode {
            name,
            children,
            bindings,
            span,
        })
    }

    fn parse_composite_body(&mut self) -> ParseResult<(Vec<Field>, Vec<Function>, Span)> {
        self.expect(TokenKind::Newline, "expected newline before body")?;
        let body_start = match self.consume_indent_with_recovery(
            "expected indentation for body",
            "Indent type/store fields with spaces or a tab",
        ) {
            Some(span) => span,
            None => {
                return Ok((Vec::new(), Vec::new(), self.previous_span()));
            }
        };
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                let end_span = self.previous_span();
                return Ok((fields, methods, end_span));
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(body_start, "missing dedent to close body");
                return Ok((fields, methods, self.previous_span()));
            }
            match self.peek_kind() {
                TokenKind::Star => {
                    let func = self.parse_function(FunctionKind::Method)?;
                    methods.push(func);
                }
                TokenKind::At => {
                    let func = self.parse_function(FunctionKind::ActorMessage)?;
                    methods.push(func);
                }
                TokenKind::Ampersand | TokenKind::Identifier(_) => {
                    let field = self.parse_field()?;
                    fields.push(field);
                }
                _ => return Err(self.error_here("unexpected token in type body")),
            }
        }
    }

    fn parse_field(&mut self) -> ParseResult<Field> {
        let start_span = self.current_span();
        let is_reference = self.matches(TokenKind::Ampersand);
        let (name, name_span) = self.consume_identifier()?;
        let mut default = None;
        if self.matches(TokenKind::Question) || self.matches(TokenKind::KeywordIs) {
            self.skip_newlines();
            default = Some(self.parse_expression()?);
        }
        self.skip_newlines();
        let span = start_span.join(name_span);
        Ok(Field {
            name,
            is_reference,
            default,
            span,
        })
    }

    fn parse_function(&mut self, default_kind: FunctionKind) -> ParseResult<Function> {
        let token = self.advance();
        let mut kind = default_kind;
        if matches!(token.kind, TokenKind::At) {
            kind = FunctionKind::ActorMessage;
        }
        let (name, name_span) = self.consume_identifier()?;
        self.expect(TokenKind::LParen, "expected `(` after function name")?;
        let params = self.parse_parameters()?;
        self.expect(TokenKind::Newline, "expected newline before function body")?;
        let body = self.parse_block()?;
        let span = token.span.join(name_span).join(body.span);
        Ok(Function {
            name,
            params,
            body,
            kind,
            span,
        })
    }

    fn parse_parameters(&mut self) -> ParseResult<Vec<Parameter>> {
        let mut params = Vec::new();
        if self.matches(TokenKind::RParen) {
            return Ok(params);
        }
        loop {
            let (name, span) = self.consume_identifier()?;
            let type_annotation = self.parse_optional_type_annotation()?;
            let mut default = None;
            if self.matches(TokenKind::Question) {
                default = Some(self.parse_expression()?);
            }
            params.push(Parameter {
                name,
                type_annotation,
                default,
                span,
            });
            if self.matches(TokenKind::Comma) {
                continue;
            }
            self.expect(TokenKind::RParen, "expected `)` to close parameters")?;
            break;
        }
        Ok(params)
    }

    fn parse_block(&mut self) -> ParseResult<Block> {
        let block_start = match self.consume_indent_with_recovery(
            "expected indentation for block",
            "Indent block contents with spaces or a tab",
        ) {
            Some(span) => span,
            None => {
                return Ok(Block {
                    statements: Vec::new(),
                    value: None,
                    span: self.previous_span(),
                });
            }
        };
        let mut statements = Vec::new();
        let mut trailing_value = None;
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(block_start, "missing dedent to close block");
                break;
            }
            let stmt = self.parse_statement()?;
            match stmt {
                Statement::Expression(expr) => {
                    trailing_value = Some(expr);
                }
                other => {
                    if let Some(value) = trailing_value.take() {
                        statements.push(Statement::Expression(value));
                    }
                    statements.push(other);
                }
            }
        }
        let span = block_start.join(self.previous_span());
        Ok(Block {
            statements,
            value: trailing_value.map(Box::new),
            span,
        })
    }

    fn parse_statement(&mut self) -> ParseResult<Statement> {
        let stmt = match self.peek_kind() {
            TokenKind::Identifier(_) if self.peek_is_binding() => {
                Statement::Binding(self.parse_binding()?)
            }
            _ => Statement::Expression(self.parse_expression()?),
        };
        self.skip_newlines();
        Ok(stmt)
    }

    fn parse_expression(&mut self) -> ParseResult<Expression> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_logic_or()?;
        if self.matches(TokenKind::Question) {
            let then_branch = self.parse_expression()?;
            self.expect(TokenKind::Bang, "expected `!` in ternary expression")?;
            let else_branch = self.parse_expression()?;
            let span = expr.span().join(else_branch.span());
            expr = Expression::Ternary {
                condition: Box::new(expr),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_logic_or(&mut self) -> ParseResult<Expression> {
            let mut expr = self.parse_logic_and()?;
        while self.matches(TokenKind::KeywordOr) {
            let rhs = self.parse_logic_and()?;
            let span = expr.span().join(rhs.span());
            expr = Expression::Binary {
                op: BinaryOp::Or,
                left: Box::new(expr),
                right: Box::new(rhs),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_logic_and(&mut self) -> ParseResult<Expression> {
            let mut expr = self.parse_bitwise_or()?;
        while self.matches(TokenKind::KeywordAnd) {
                let rhs = self.parse_bitwise_or()?;
            let span = expr.span().join(rhs.span());
            expr = Expression::Binary {
                op: BinaryOp::And,
                left: Box::new(expr),
                right: Box::new(rhs),
                span,
            };
        }
        Ok(expr)
    }

        fn parse_bitwise_or(&mut self) -> ParseResult<Expression> {
            let mut expr = self.parse_bitwise_xor()?;
            while self.matches(TokenKind::Pipe) {
                let rhs = self.parse_bitwise_xor()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op: BinaryOp::BitOr,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            }
            Ok(expr)
        }

        fn parse_bitwise_xor(&mut self) -> ParseResult<Expression> {
            let mut expr = self.parse_bitwise_and()?;
            while self.matches(TokenKind::Caret) {
                let rhs = self.parse_bitwise_and()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op: BinaryOp::BitXor,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            }
            Ok(expr)
        }

        fn parse_bitwise_and(&mut self) -> ParseResult<Expression> {
            let mut expr = self.parse_equality()?;
            while self.matches(TokenKind::Ampersand) {
                let rhs = self.parse_equality()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op: BinaryOp::BitAnd,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            }
            Ok(expr)
        }

    fn parse_equality(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_comparison()?;
        loop {
            let matched_equals = self.matches(TokenKind::Equals);
            let matched_is = !matched_equals && self.matches(TokenKind::KeywordIs);
            if matched_equals || matched_is {
                let rhs = self.parse_comparison()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op: BinaryOp::Equals,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_shift()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Greater => Some(BinaryOp::Greater),
                TokenKind::GreaterEq => Some(BinaryOp::GreaterEq),
                TokenKind::Less => Some(BinaryOp::Less),
                TokenKind::LessEq => Some(BinaryOp::LessEq),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                    let rhs = self.parse_shift()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

        fn parse_shift(&mut self) -> ParseResult<Expression> {
            let mut expr = self.parse_term()?;
            loop {
                let op = match self.peek_kind() {
                    TokenKind::ShiftLeft => Some(BinaryOp::ShiftLeft),
                    TokenKind::ShiftRight => Some(BinaryOp::ShiftRight),
                    _ => None,
                };
                if let Some(op) = op {
                    self.advance();
                    let rhs = self.parse_term()?;
                    let span = expr.span().join(rhs.span());
                    expr = Expression::Binary {
                        op,
                        left: Box::new(expr),
                        right: Box::new(rhs),
                        span,
                    };
                } else {
                    break;
                }
            }
            Ok(expr)
        }

    fn parse_term(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_factor()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => Some(BinaryOp::Add),
                TokenKind::Minus => Some(BinaryOp::Sub),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let rhs = self.parse_factor()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_factor(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => Some(BinaryOp::Mul),
                TokenKind::Slash => Some(BinaryOp::Div),
                TokenKind::Percent => Some(BinaryOp::Mod),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let rhs = self.parse_unary()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> ParseResult<Expression> {
        if self.check(TokenKind::Bang) {
            if matches!(self.peek_next_kind(), Some(TokenKind::BangBang)) {
                let bang_span = self.current_span();
                self.advance();
                let value = self.parse_unary()?;
                let span = bang_span.join(value.span());
                return Ok(Expression::Throw {
                    value: Box::new(value),
                    span,
                });
            }
        }
        if self.matches(TokenKind::Minus) {
            let expr = self.parse_unary()?;
            let span = self.previous_span().join(expr.span());
            return Ok(Expression::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            });
        }
        if self.matches(TokenKind::Bang) {
            let expr = self.parse_unary()?;
            let span = self.previous_span().join(expr.span());
            return Ok(Expression::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            });
        }
        if self.matches(TokenKind::Tilde) {
            let expr = self.parse_unary()?;
            let span = self.previous_span().join(expr.span());
            return Ok(Expression::Unary {
                op: UnaryOp::BitNot,
                expr: Box::new(expr),
                span,
            });
        }
        self.parse_call()
    }

    fn parse_call(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.matches(TokenKind::LParen) {
                let args = self.parse_arguments()?;
                let span = expr.span().join(self.previous_span());
                expr = Expression::Call {
                    callee: Box::new(expr),
                    args,
                    span,
                };
            } else if self.matches(TokenKind::Dot) {
                let (name, span) = self.consume_identifier()?;
                let span = expr.span().join(span);
                expr = Expression::Member {
                    target: Box::new(expr),
                    property: name,
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_arguments(&mut self) -> ParseResult<Vec<Expression>> {
        let mut args = Vec::new();
        if self.matches(TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expression()?);
            if self.matches(TokenKind::Comma) {
                continue;
            }
            self.expect(TokenKind::RParen, "expected `)` to close arguments")?;
            break;
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> ParseResult<Expression> {
        match self.peek_kind() {
            TokenKind::Integer(_) => {
                let token = self.advance();
                if let TokenKind::Integer(value) = token.kind {
                    Ok(Expression::Integer(value, token.span))
                } else {
                    unreachable!()
                }
            }
            TokenKind::Float(_) => {
                let token = self.advance();
                if let TokenKind::Float(value) = token.kind {
                    Ok(Expression::Float(value, token.span))
                } else {
                    unreachable!()
                }
            }
            TokenKind::KeywordTrue => {
                let token = self.advance();
                Ok(Expression::Bool(true, token.span))
            }
            TokenKind::KeywordFalse => {
                let token = self.advance();
                Ok(Expression::Bool(false, token.span))
            }
            TokenKind::String(_) => {
                let token = self.advance();
                if let TokenKind::String(value) = &token.kind {
                    Ok(Expression::String(value.clone(), token.span))
                } else {
                    unreachable!()
                }
            }
            TokenKind::Bytes(_) => {
                let token = self.advance();
                if let TokenKind::Bytes(value) = token.kind {
                    Ok(Expression::Bytes(value, token.span))
                } else {
                    unreachable!()
                }
            }
            TokenKind::Star => {
                if matches!(self.peek_next_kind(), Some(TokenKind::KeywordFn)) {
                    return self.parse_lambda_expression();
                }
                return Err(self.error_here("unexpected `*` in expression"));
            }
            TokenKind::TemplateString(_) => {
                let token = self.advance();
                if let TokenKind::TemplateString(fragments) = token.kind {
                    self.parse_template_literal(fragments, token.span)
                } else {
                    unreachable!()
                }
            }
            TokenKind::Placeholder(_) => {
                let token = self.advance();
                if let TokenKind::Placeholder(index) = token.kind {
                    Ok(Expression::Placeholder(index, token.span))
                } else {
                    unreachable!()
                }
            }
            TokenKind::BangBang => self.parse_taxonomy_path_expression(),
            TokenKind::Identifier(_) => {
                let (name, span) = self.consume_identifier()?;
                if name == "map" && self.check(TokenKind::LParen) {
                    self.parse_map_literal(span)
                } else {
                    Ok(Expression::Identifier(name, span))
                }
            }
            TokenKind::LParen => {
                self.advance();
                self.skip_newlines();
                if self.matches(TokenKind::RParen) {
                    return Ok(Expression::Unit);
                }
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen, "expected closing )")?;
                Ok(expr)
            }
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::KeywordMatch => self.parse_match_expression(),
            TokenKind::KeywordUnsafe => self.parse_unsafe_block(),
            TokenKind::KeywordAsm => self.parse_inline_asm(),
            TokenKind::At => self.parse_ptr_load(),
            _ => Err(self.error_here("unexpected token in expression")),
        }
    }

    fn parse_map_literal(&mut self, name_span: Span) -> ParseResult<Expression> {
        let open = self.expect(TokenKind::LParen, "expected `(` after map literal")?;
        let start = name_span.join(open.span);
        self.skip_newlines();
        let mut entries = Vec::new();
        if self.matches(TokenKind::RParen) {
            return Ok(Expression::Map(entries, start.join(self.previous_span())));
        }
        loop {
            self.skip_newlines();
            let key = self.parse_unary()?;
            self.skip_newlines();
            self.expect(TokenKind::KeywordIs, "expected `is` between map key and value")?;
            self.skip_newlines();
            let value = self.parse_expression()?;
            entries.push((key, value));
            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
                continue;
            }
            let end = self.expect(TokenKind::RParen, "expected `)` to close map literal")?.span;
            return Ok(Expression::Map(entries, start.join(end)));
        }
    }

    fn parse_template_literal(
        &mut self,
        fragments: Vec<TemplateFragment>,
        span: Span,
    ) -> ParseResult<Expression> {
        let mut parts = Vec::new();
        for fragment in fragments {
            match fragment {
                TemplateFragment::Literal { value, span } => {
                    parts.push(Expression::String(value, span));
                }
                TemplateFragment::Expr { source, span } => {
                    let expr = self.parse_fragment_expression(&source, span)?;
                    parts.push(expr);
                }
            }
        }
        if parts.is_empty() {
            return Ok(Expression::String(String::new(), span));
        }
        if !matches!(parts.first(), Some(Expression::String(_, _))) {
            parts.insert(0, Expression::String(String::new(), Span::new(span.start, span.start)));
        }
        let mut iter = parts.into_iter();
        let mut acc = iter.next().unwrap();
        for part in iter {
            let combined_span = acc.span().join(part.span());
            acc = Expression::Binary {
                op: BinaryOp::Add,
                left: Box::new(acc),
                right: Box::new(part),
                span: combined_span,
            };
        }
        Ok(acc)
    }

    fn parse_taxonomy_path_expression(&mut self) -> ParseResult<Expression> {
        let start = self.advance().span;
        let (name, name_span) = self.consume_identifier()?;
        let mut segments = vec![name];
        let mut end_span = name_span;
        while self.matches(TokenKind::Colon) {
            let (segment, seg_span) = self.consume_identifier()?;
            end_span = seg_span;
            segments.push(segment);
        }
        Ok(Expression::TaxonomyPath {
            segments,
            span: start.join(end_span),
        })
    }

    fn parse_fragment_expression(&self, source: &str, span: Span) -> ParseResult<Expression> {
        let mut tokens = lexer::lex(source).map_err(|diag| diag.shift(span.start))?;
        for token in tokens.iter_mut() {
            token.span = token.span.shift(span.start);
        }
        Parser::new(tokens, self.source_len).parse_inline_expression()
    }

    fn parse_lambda_expression(&mut self) -> ParseResult<Expression> {
        let star = self.advance();
        self.expect(TokenKind::KeywordFn, "expected `fn` after `*`")?;
        self.expect(TokenKind::LParen, "expected `(` after fn")?;
        let params = self.parse_parameters()?;
        if self.matches(TokenKind::Newline) {
            let body = self.parse_block()?;
            let span = star.span.join(body.span);
            Ok(Expression::Lambda { params, body, span })
        } else {
            let expr = self.parse_expression()?;
            let body = Block::from_expression(expr);
            let span = star.span.join(body.span);
            Ok(Expression::Lambda { params, body, span })
        }
    }

    fn parse_list_literal(&mut self) -> ParseResult<Expression> {
        let start = self.advance().span;
        let mut items = Vec::new();
        if self.matches(TokenKind::RBracket) {
            return Ok(Expression::List(items, start));
        }
        self.skip_newlines();
        loop {
            items.push(self.parse_expression()?);
            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
                continue;
            }
            let end = self.expect(TokenKind::RBracket, "expected ]")?.span;
            return Ok(Expression::List(items, start.join(end)));
        }
    }

    fn parse_match_expression(&mut self) -> ParseResult<Expression> {
        let match_span = self.advance().span;
        let value = self.parse_expression()?;
        self.expect(TokenKind::Newline, "expected newline after match condition")?;
        let arms_start = match self.consume_indent_with_recovery(
            "expected indented match arms",
            "Indent each match arm under the match expression",
        ) {
            Some(span) => span,
            None => {
                return Ok(Expression::Match(Box::new(MatchExpression {
                    value: Box::new(value),
                    arms: Vec::new(),
                    default: None,
                    span: match_span,
                })));
            }
        };
        let mut arms = Vec::new();
        let mut default = None;
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(arms_start, "missing dedent to close match arms");
                break;
            }
            if self.matches(TokenKind::Bang) {
                let expr = self.parse_expression()?;
                default = Some(Box::new(Block::from_expression(expr)));
                self.skip_newlines();
                continue;
            }
            let pattern = self.parse_match_pattern()?;
            self.expect(TokenKind::Question, "expected `?` in match arm")?;
            let expr = self.parse_expression()?;
            arms.push(MatchArm {
                pattern,
                body: Block::from_expression(expr),
            });
            self.skip_newlines();
        }
        let span = match_span.join(self.previous_span());
        Ok(Expression::Match(Box::new(MatchExpression {
            value: Box::new(value),
            arms,
            default,
            span,
        })))
    }

    fn parse_match_pattern(&mut self) -> ParseResult<MatchPattern> {
        match self.peek_kind() {
            TokenKind::Integer(_) => {
                let token = self.advance();
                if let TokenKind::Integer(value) = token.kind {
                    Ok(MatchPattern::Integer(value))
                } else {
                    unreachable!()
                }
            }
            TokenKind::KeywordTrue => {
                let _token = self.advance();
                Ok(MatchPattern::Bool(true))
            }
            TokenKind::KeywordFalse => {
                let _token = self.advance();
                Ok(MatchPattern::Bool(false))
            }
            TokenKind::String(_) => {
                let token = self.advance();
                if let TokenKind::String(value) = token.kind {
                    Ok(MatchPattern::String(value))
                } else {
                    unreachable!()
                }
            }
            TokenKind::LBracket => {
                let expr = self.parse_list_literal()?;
                if let Expression::List(items, _) = expr {
                    Ok(MatchPattern::List(items))
                } else {
                    unreachable!()
                }
            }
            TokenKind::Identifier(_) => {
                let (name, _) = self.consume_identifier()?;
                Ok(MatchPattern::Identifier(name))
            }
            _ => Err(self.error_here("invalid match pattern")),
        }
    }

    fn consume_identifier(&mut self) -> ParseResult<(String, Span)> {
        match self.peek_kind() {
            TokenKind::Identifier(_) => {
                let token = self.advance();
                if let TokenKind::Identifier(name) = token.kind {
                    Ok((name, token.span))
                } else {
                    unreachable!()
                }
            }
            _ => Err(self.error_here("expected identifier")),
        }
    }

    fn consume_type_identifier(&mut self) -> ParseResult<(String, Span)> {
        match self.peek_kind() {
            TokenKind::Identifier(_) => {
                let token = self.advance();
                if let TokenKind::Identifier(name) = token.kind {
                    Ok((name, token.span))
                } else {
                    unreachable!()
                }
            }
            TokenKind::KeywordPtr => {
                let token = self.advance();
                Ok(("ptr".to_string(), token.span))
            }
            _ => Err(self.error_here("expected type identifier")),
        }
    }

    fn consume_indent_with_recovery(&mut self, message: &str, help: &str) -> Option<Span> {
        if self.matches(TokenKind::Indent) {
            self.layout_depth += 1;
            Some(self.previous_span())
        } else {
            let span = self.current_span();
            let diagnostic = Diagnostic::new(message, span).with_help(help);
            self.record_error(diagnostic);
            self.recover_to_layout_boundary();
            None
        }
    }

    fn report_missing_dedent(&mut self, start_span: Span, context: &str) {
        let span = start_span.join(self.previous_span());
        let diagnostic = Diagnostic::new(context, span)
            .with_help("Add a matching dedent/outdent to close this block.");
        self.record_error(diagnostic);
        self.recover_to_layout_boundary();
        if self.layout_depth > 0 {
            self.layout_depth -= 1;
        }
    }

    fn leave_layout_block(&mut self, span: Span) {
        if self.layout_depth == 0 {
            self.report_unexpected_dedent(span);
        } else {
            self.layout_depth -= 1;
        }
    }

    fn report_unexpected_dedent(&mut self, span: Span) {
        let diagnostic = Diagnostic::new("unexpected dedent", span)
            .with_help("Remove the extra outdent or ensure there's a matching indented block.");
        self.record_error(diagnostic);
    }

    fn record_error(&mut self, diagnostic: Diagnostic) {
        if self.pending_error.is_none() {
            self.pending_error = Some(diagnostic);
        }
    }

    fn recover_to_layout_boundary(&mut self) {
        while !self.check(TokenKind::Eof) {
            match self.peek_kind() {
                TokenKind::Newline => {
                    self.advance();
                    break;
                }
                TokenKind::Indent | TokenKind::Dedent => break,
                TokenKind::KeywordType
                | TokenKind::KeywordStore
                | TokenKind::KeywordMatch
                | TokenKind::Star => break,
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_newlines(&mut self) {
        while self.check(TokenKind::Newline) {
            self.advance();
        }
    }

    fn peek_is_binding(&self) -> bool {
        let mut offset = 1usize;
        let mut in_annotation = false;
        while let Some(token) = self.tokens.get(self.index + offset) {
            match &token.kind {
                TokenKind::KeywordIs => return true,
                TokenKind::Colon if !in_annotation => {
                    in_annotation = true;
                    offset += 1;
                }
                TokenKind::Identifier(_) | TokenKind::Dot if in_annotation => {
                    offset += 1;
                }
                TokenKind::Newline => offset += 1,
                _ => return false,
            }
        }
        false
    }

    fn parse_extern_function(&mut self) -> ParseResult<ExternFunction> {
        let start = self.advance().span;
        self.expect(TokenKind::KeywordFn, "expected `fn` after `extern`")?;
        let (name, name_span) = self.consume_identifier()?;
        self.expect(TokenKind::LParen, "expected `(` after extern function name")?;
        let params = self.parse_parameters()?;
        let return_type = if self.matches(TokenKind::Colon) {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };
        let span = start.join(name_span).join(
            return_type.as_ref().map(|t| t.span).unwrap_or(name_span)
        );
        Ok(ExternFunction {
            name,
            params,
            return_type,
            span,
        })
    }

    fn parse_unsafe_block(&mut self) -> ParseResult<Expression> {
        let start = self.advance().span;
        self.expect(TokenKind::Newline, "expected newline after `unsafe`")?;
        let block = self.parse_block()?;
        let span = start.join(block.span);
        Ok(Expression::Unsafe { block, span })
    }

    fn parse_inline_asm(&mut self) -> ParseResult<Expression> {
        let start = self.advance().span;
        self.expect(TokenKind::LParen, "expected `(` after `asm`")?;
        if !matches!(self.peek_kind(), TokenKind::String(_)) {
            return Err(self.error_here("expected string template for asm"));
        }
        let template_token = self.advance();
        let template = if let TokenKind::String(s) = template_token.kind {
            s
        } else {
            return Err(self.error_here("expected string template for asm"));
        };
        let mut inputs = Vec::new();
        let outputs = Vec::new();
        if self.matches(TokenKind::Comma) {
            loop {
                self.skip_newlines();
                if self.check(TokenKind::RParen) {
                    break;
                }
                let (constraint, _) = self.consume_identifier()?;
                self.expect(TokenKind::Colon, "expected `:` after constraint")?;
                let expr = self.parse_expression()?;
                inputs.push((constraint, expr));
                if !self.matches(TokenKind::Comma) {
                    break;
                }
            }
        }
        let end = self.expect(TokenKind::RParen, "expected `)` to close asm")?;
        Ok(Expression::InlineAsm {
            template,
            inputs,
            outputs,
            span: start.join(end.span),
        })
    }

    fn parse_ptr_load(&mut self) -> ParseResult<Expression> {
        let start = self.advance().span;
        let address = Box::new(self.parse_primary()?);
        let span = start.join(address.span());
        Ok(Expression::PtrLoad { address, span })
    }

    fn peek_kind(&self) -> TokenKind {
        self.tokens
            .get(self.index)
            .map(|t| t.kind.clone())
            .unwrap_or(TokenKind::Eof)
    }

    fn peek_next_kind(&self) -> Option<TokenKind> {
        self.tokens.get(self.index + 1).map(|t| t.kind.clone())
    }

    fn check(&self, kind: TokenKind) -> bool {
        matches!(self.tokens.get(self.index), Some(token) if token.kind == kind)
    }

    fn matches(&mut self, kind: TokenKind) -> bool {
        if self.check(kind.clone()) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &str) -> ParseResult<Token> {
        if self.check(kind.clone()) {
            Ok(self.advance())
        } else {
            Err(self.error_here(message))
        }
    }

    fn advance(&mut self) -> Token {
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| Token::new(TokenKind::Eof, Span::new(self.source_len, self.source_len)));
        self.index += 1;
        token
    }

    fn previous_span(&self) -> Span {
        if self.index == 0 {
            Span::new(0, 0)
        } else {
            self.tokens
                .get(self.index - 1)
                .map(|t| t.span)
                .unwrap_or_else(|| Span::new(0, 0))
        }
    }

    fn current_span(&self) -> Span {
        self.tokens
            .get(self.index)
            .map(|t| t.span)
            .unwrap_or_else(|| Span::new(self.source_len, self.source_len))
    }

    fn error_here(&self, message: &str) -> Diagnostic {
        Diagnostic::new(message, self.current_span())
    }
}
