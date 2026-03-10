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
    /// Accumulated parse errors for multi-error recovery
    errors: Vec<Diagnostic>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, source_len: usize) -> Self {
        Self {
            tokens,
            index: 0,
            source_len,
            pending_error: None,
            layout_depth: 0,
            errors: Vec::new(),
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
            match self.parse_item() {
                Ok(item) => {
                    items.push(item);
                }
                Err(diag) => {
                    self.errors.push(diag);
                    self.synchronize_to_item();
                }
            }
            self.skip_newlines();
        }
        let program = Program::new(items, Span::new(0, self.source_len));
        if let Some(error) = self.pending_error {
            self.errors.insert(0, error);
        }
        if self.errors.is_empty() {
            Ok(program)
        } else {
            // Return the first error for backward compatibility;
            // all errors are accumulated in self.errors
            Err(self.errors.remove(0))
        }
    }

    /// Return all accumulated errors (for multi-error reporting).
    /// Call after parse() returns Err to get additional errors.
    pub fn parse_with_recovery(mut self) -> (Program, Vec<Diagnostic>) {
        let mut items = Vec::new();

        self.skip_newlines();
        while !self.check(TokenKind::Eof) {
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                continue;
            }
            match self.parse_item() {
                Ok(item) => {
                    items.push(item);
                }
                Err(diag) => {
                    self.errors.push(diag);
                    self.synchronize_to_item();
                }
            }
            self.skip_newlines();
        }
        let program = Program::new(items, Span::new(0, self.source_len));
        if let Some(error) = self.pending_error {
            self.errors.insert(0, error);
        }
        (program, self.errors)
    }

    /// Skip tokens until we find a synchronization point (start of a new item).
    /// This allows continuing parsing after an error.
    fn synchronize_to_item(&mut self) {
        loop {
            match self.peek_kind() {
                // Item-start tokens — synchronization points
                TokenKind::Star
                | TokenKind::KeywordType
                | TokenKind::KeywordEnum
                | TokenKind::KeywordStore
                | TokenKind::KeywordPersist
                | TokenKind::KeywordActor
                | TokenKind::KeywordErr
                | TokenKind::KeywordTrait
                | TokenKind::KeywordExtend
                | TokenKind::KeywordExtern
                | TokenKind::Eof => {
                    return;
                }
                // Newline followed by a top-level construct also syncs
                TokenKind::Newline => {
                    self.advance();
                    self.skip_newlines();
                    // Check if the next token starts an item
                    match self.peek_kind() {
                        TokenKind::Star
                        | TokenKind::KeywordType
                        | TokenKind::KeywordEnum
                        | TokenKind::KeywordStore
                        | TokenKind::KeywordPersist
                        | TokenKind::KeywordActor
                        | TokenKind::KeywordErr
                        | TokenKind::KeywordTrait
                        | TokenKind::KeywordExtend
                        | TokenKind::KeywordExtern
                        | TokenKind::Eof
                        | TokenKind::Identifier(_) => {
                            return;
                        }
                        _ => {}
                    }
                }
                _ => {
                    self.advance();
                }
            }
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
            TokenKind::KeywordEnum => self.parse_enum_def().map(Item::Type),
            TokenKind::KeywordStore => self.parse_store_def(false).map(Item::Store),
            TokenKind::KeywordPersist => self.parse_persist_store_def().map(Item::Store),
            TokenKind::KeywordActor => self.parse_actor_def().map(Item::Store),
            TokenKind::KeywordErr => self.parse_error_definition().map(Item::ErrorDefinition),
            TokenKind::KeywordTrait => self.parse_trait_def().map(Item::TraitDefinition),
            TokenKind::KeywordExtend => self.parse_extension_def().map(Item::Extension),
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
            TokenKind::Indent => Err(self.error_here("unexpected indent at top-level")),
            TokenKind::Dedent => Err(self.error_here("unexpected dedent at top-level")),
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
        let type_params = self.parse_type_params()?;
        let (with_traits, fields, methods, end_span) = self.parse_composite_body_with_traits()?;
        let span = start.join(name_span).join(end_span);
        Ok(TypeDefinition {
            name,
            type_params,
            with_traits,
            fields,
            methods,
            variants: Vec::new(),  // ADT variants - only for enums
            span,
        })
    }

    /// Parse enum (sum type / algebraic data type) definition
    /// Syntax:
    ///   enum Option
    ///     Some(value)
    ///     None
    fn parse_enum_def(&mut self) -> ParseResult<TypeDefinition> {
        let start = self.advance().span; // consume `enum`
        let (name, name_span) = self.consume_identifier()?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::Newline, "expected newline after enum name")?;
        
        let body_start = match self.consume_indent_with_recovery(
            "expected indentation for enum variants",
            "Indent enum variants with spaces or a tab",
        ) {
            Some(span) => span,
            None => {
                // Empty enum with no variants
                return Ok(TypeDefinition {
                    name,
                    type_params,
                    with_traits: Vec::new(),
                    fields: Vec::new(),
                    methods: Vec::new(),
                    variants: Vec::new(),
                    span: start.join(name_span),
                });
            }
        };
        
        let mut variants = Vec::new();
        let mut end_span = body_start;
        
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                end_span = self.current_span();
                self.leave_layout_block(end_span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(body_start, "missing dedent to close enum body");
                break;
            }
            
            // Parse variant: VariantName or VariantName(field1, field2, ...)
            let variant = self.parse_type_variant()?;
            variants.push(variant);
        }
        
        let span = start.join(name_span).join(end_span);
        Ok(TypeDefinition {
            name,
            type_params,
            with_traits: Vec::new(),  // Enums don't have traits yet
            fields: Vec::new(),
            methods: Vec::new(),
            variants,
            span,
        })
    }
    
    /// Parse a single type variant
    /// Syntax: VariantName or VariantName(field1, field2, ...)
    fn parse_type_variant(&mut self) -> ParseResult<TypeVariant> {
        let (variant_name, variant_span) = self.consume_identifier()?;
        let mut fields = Vec::new();
        let mut end_span = variant_span;
        
        // Check for fields: VariantName(field1, field2)
        if self.matches(TokenKind::LParen) {
            loop {
                if self.check(TokenKind::RParen) {
                    end_span = self.advance().span;
                    break;
                }
                let (field_name, field_span) = self.consume_identifier()?;
                fields.push(VariantField {
                    name: Some(field_name),
                    type_annotation: None,  // Type annotations could be added later
                    span: field_span,
                });
                
                if !self.matches(TokenKind::Comma) {
                    end_span = self.expect(TokenKind::RParen, "expected `)` after variant fields")?.span;
                    break;
                }
            }
        }
        
        Ok(TypeVariant {
            name: variant_name,
            fields,
            span: variant_span.join(end_span),
        })
    }

    fn parse_store_def(&mut self, is_persistent: bool) -> ParseResult<StoreDefinition> {
        let start = self.advance().span; // consume 'store'
        self.parse_store_body(start, is_persistent, false)
    }

    /// Parse `persist store Name` syntax for persistent stores
    fn parse_persist_store_def(&mut self) -> ParseResult<StoreDefinition> {
        let start = self.advance().span; // consume `persist`
        self.expect(TokenKind::KeywordStore, "expected 'store' after 'persist'")?;
        self.parse_store_body(start, true, false)
    }

    fn parse_actor_def(&mut self) -> ParseResult<StoreDefinition> {
        let start = self.advance().span; // consume `actor`
        self.parse_store_body(start, false, true)
    }

    fn parse_store_body(&mut self, start: Span, is_persistent: bool, is_actor: bool) -> ParseResult<StoreDefinition> {
        let (name, name_span) = self.consume_identifier()?;
        let (with_traits, fields, methods, end_span) = self.parse_composite_body_with_traits()?;
        let span = start.join(name_span).join(end_span);
        Ok(StoreDefinition {
            name,
            with_traits,
            fields,
            methods,
            is_actor,
            is_persistent,
            span,
        })
    }

    /// Parse hierarchical error definition:
    /// ```coral
    /// err Database
    ///     err Connection
    ///         err Timeout
    ///             code is 5001
    ///             message is 'Connection timed out'
    ///         err Refused
    /// ```
    fn parse_error_definition(&mut self) -> ParseResult<ErrorDefinition> {
        let start = self.advance().span; // consume `err`
        let (name, name_span) = self.consume_identifier()?;
        
        // Check if there's an indented body
        let (code, message, children, end_span) = if self.matches(TokenKind::Newline) {
            if self.matches(TokenKind::Indent) {
                let (code, message, children, end) = self.parse_error_body()?;
                self.expect(TokenKind::Dedent, "expected dedent after error definition body")?;
                (code, message, children, end)
            } else {
                (None, None, vec![], name_span)
            }
        } else {
            (None, None, vec![], name_span)
        };
        
        Ok(ErrorDefinition {
            name,
            code,
            message,
            children,
            span: start.join(end_span),
        })
    }
    
    /// Parse the body of an error definition (code, message, and child errors)
    fn parse_error_body(&mut self) -> ParseResult<(Option<i64>, Option<String>, Vec<ErrorDefinition>, Span)> {
        let mut code = None;
        let mut message = None;
        let mut children = Vec::new();
        let mut end_span = self.previous_span();
        
        loop {
            self.skip_newlines();
            
            if self.check(TokenKind::Dedent) || self.check(TokenKind::Eof) {
                break;
            }
            
            // Check for nested error definition
            if self.check(TokenKind::KeywordErr) {
                let child = self.parse_error_definition()?;
                end_span = child.span;
                children.push(child);
                continue;
            }
            
            // Check for `code is <number>`
            if let TokenKind::Identifier(ident) = self.peek_kind() {
                let ident = ident.clone();
                if ident == "code" {
                    self.advance();
                    self.expect(TokenKind::KeywordIs, "expected `is` after `code`")?;
                    if let TokenKind::Integer(n) = self.peek_kind() {
                        code = Some(*n);
                        end_span = self.advance().span;
                        if self.check(TokenKind::Newline) {
                            self.advance();
                        }
                        continue;
                    } else {
                        return Err(self.error_here("expected integer for error code"));
                    }
                } else if ident == "message" {
                    self.advance();
                    self.expect(TokenKind::KeywordIs, "expected `is` after `message`")?;
                    if let TokenKind::String(s) = self.peek_kind() {
                        message = Some(s.clone());
                        end_span = self.advance().span;
                        if self.check(TokenKind::Newline) {
                            self.advance();
                        }
                        continue;
                    } else {
                        return Err(self.error_here("expected string for error message"));
                    }
                }
            }
            
            // Unknown token in error body
            break;
        }
        
        Ok((code, message, children, end_span))
    }

    fn parse_optional_type_annotation(&mut self) -> ParseResult<Option<TypeAnnotation>> {
        if !self.matches(TokenKind::Colon) {
            return Ok(None);
        }
        let annotation = self.parse_type_annotation()?;
        Ok(Some(annotation))
    }

    /// Parse optional type parameters on a type/enum definition.
    /// Syntax: `[A, B, C]` or `[T with Comparable, U with Display]`
    /// T2.4: Supports trait bounds via `with TraitName`.
    fn parse_type_params(&mut self) -> ParseResult<Vec<crate::ast::TypeParam>> {
        if !self.matches(TokenKind::LBracket) {
            return Ok(Vec::new());
        }
        let mut params = Vec::new();
        loop {
            if self.check(TokenKind::RBracket) {
                self.advance();
                break;
            }
            let (param_name, _) = self.consume_identifier()?;
            // T2.4: Check for trait bounds: `T with Comparable`
            let mut bounds = Vec::new();
            if self.matches(TokenKind::KeywordWith) {
                // Parse one or more trait names separated by commas
                // But commas also separate type params, so we parse trait names
                // as long as the next token is an uppercase identifier (trait name)
                // and not followed by `with` (which would mean next type param).
                loop {
                    let (trait_name, _) = self.consume_identifier()?;
                    bounds.push(trait_name);
                    // If next is comma, peek ahead: if followed by uppercase + `with` or `]`,
                    // it's the next type param. If followed by uppercase without `with`, it could
                    // be another bound or next param. Simplification: after `with`, commas always
                    // mean "next type param boundary". Multiple bounds use `with T1 and T2` style.
                    // Actually, let's just use: `T with Comparable` (single bound each for now)
                    // or `T with Comparable with Hashable` for multiple bounds.
                    if self.check(TokenKind::KeywordWith) {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            params.push(crate::ast::TypeParam::new(param_name, bounds));
            if !self.matches(TokenKind::Comma) {
                self.expect(TokenKind::RBracket, "expected `]` after type parameters")?;
                break;
            }
        }
        Ok(params)
    }

    fn parse_type_annotation(&mut self) -> ParseResult<TypeAnnotation> {
        let (first, mut span) = self.consume_type_identifier()?;
        let mut segments = vec![first];
        while self.matches(TokenKind::Dot) {
            let (segment, segment_span) = self.consume_type_identifier()?;
            span = span.join(segment_span);
            segments.push(segment);
        }
        // Parse type arguments: Type[Arg1, Arg2]
        let mut type_args = Vec::new();
        if self.matches(TokenKind::LBracket) {
            loop {
                if self.check(TokenKind::RBracket) {
                    span = span.join(self.advance().span);
                    break;
                }
                let arg = self.parse_type_annotation()?;
                span = span.join(arg.span);
                type_args.push(arg);
                if !self.matches(TokenKind::Comma) {
                    span = span.join(self.expect(TokenKind::RBracket, "expected `]` after type arguments")?.span);
                    break;
                }
            }
        }
        Ok(TypeAnnotation { segments, type_args, span })
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

    /// Parse composite body with optional `with TraitName` clauses
    fn parse_composite_body_with_traits(&mut self) -> ParseResult<(Vec<String>, Vec<Field>, Vec<Function>, Span)> {
        // First, check for `with Trait1, Trait2` on the same line as type/store declaration
        let mut with_traits = Vec::new();
        if self.check(TokenKind::KeywordWith) {
            self.advance(); // consume `with`
            loop {
                let (trait_name, _) = self.consume_identifier()?;
                with_traits.push(trait_name);
                if !self.matches(TokenKind::Comma) {
                    break;
                }
            }
        }
        
        self.expect(TokenKind::Newline, "expected newline before body")?;
        let body_start = match self.consume_indent_with_recovery(
            "expected indentation for body",
            "Indent type/store fields with spaces or a tab",
        ) {
            Some(span) => span,
            None => {
                return Ok((with_traits, Vec::new(), Vec::new(), self.previous_span()));
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
                return Ok((with_traits, fields, methods, end_span));
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(body_start, "missing dedent to close body");
                return Ok((with_traits, fields, methods, self.previous_span()));
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

    /// S4.5: Parse extension definition
    /// ```coral
    /// extend String
    ///     *word_count()
    ///         self.split(" ").length()
    /// ```
    fn parse_extension_def(&mut self) -> ParseResult<ExtensionDefinition> {
        let start = self.advance().span; // consume `extend`
        let (target_type, _) = self.consume_identifier()?;

        self.expect(TokenKind::Newline, "expected newline before extension body")?;
        let body_start = match self.consume_indent_with_recovery(
            "expected indentation for extension body",
            "Indent extension methods with spaces or a tab",
        ) {
            Some(span) => span,
            None => {
                return Ok(ExtensionDefinition {
                    target_type,
                    methods: Vec::new(),
                    span: start,
                });
            }
        };

        let mut methods = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(body_start, "missing dedent to close extension body");
                break;
            }
            match self.peek_kind() {
                TokenKind::Star => {
                    let func = self.parse_function(FunctionKind::Method)?;
                    methods.push(func);
                }
                _ => return Err(self.error_here("expected method definition (starting with *) in extension body")),
            }
        }

        let end_span = self.previous_span();
        Ok(ExtensionDefinition {
            target_type,
            methods,
            span: start.join(end_span),
        })
    }

    /// Parse trait definition
    /// ```coral
    /// trait Printable
    ///     *to_string()
    ///     *print()
    ///         log(to_string())
    /// ```
    fn parse_trait_def(&mut self) -> ParseResult<TraitDefinition> {
        let start = self.advance().span; // consume `trait`
        let (name, name_span) = self.consume_identifier()?;
        
        // Parse optional `with TraitName, TraitName2, ...` dependencies on the same line
        let mut required_traits = Vec::new();
        if self.check(TokenKind::KeywordWith) {
            self.advance(); // consume `with`
            loop {
                let (trait_name, _) = self.consume_identifier()?;
                required_traits.push(trait_name);
                if !self.matches(TokenKind::Comma) {
                    break;
                }
            }
        }
        
        // Check if there's a newline (needed for body or end of definition)
        if !self.matches(TokenKind::Newline) {
            // No newline - trait definition on single line, no body
            return Ok(TraitDefinition {
                name,
                required_traits,
                methods: Vec::new(),
                span: start.join(name_span),
            });
        }
        
        // Check if there's an indented body (methods)
        if !self.check(TokenKind::Indent) {
            // No indent after newline - empty trait body
            return Ok(TraitDefinition {
                name,
                required_traits,
                methods: Vec::new(),
                span: start.join(name_span),
            });
        }
        
        // Consume the indent
        self.advance();
        self.layout_depth += 1;
        let body_start = self.previous_span();
        
        let mut methods = Vec::new();
        let mut end_span = body_start;
        
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                end_span = self.current_span();
                self.leave_layout_block(end_span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(body_start, "missing dedent to close trait body");
                break;
            }
            
            if self.check(TokenKind::Star) {
                let method = self.parse_trait_method()?;
                methods.push(method);
            } else {
                return Err(self.error_here("expected `*` for trait method"));
            }
        }
        
        Ok(TraitDefinition {
            name,
            required_traits,
            methods,
            span: start.join(end_span),
        })
    }
    
    /// Parse a trait method - may have optional default implementation
    fn parse_trait_method(&mut self) -> ParseResult<TraitMethod> {
        let start = self.advance().span; // consume `*`
        let (name, name_span) = self.consume_identifier()?;
        self.expect(TokenKind::LParen, "expected `(` after method name")?;
        let params = self.parse_parameters()?;
        
        // Check if there's a body (default implementation) or just a signature
        let body = if self.matches(TokenKind::Newline) {
            // Check if next line is indented (body) or not (signature only)
            if self.check(TokenKind::Indent) {
                Some(self.parse_block()?)
            } else {
                None
            }
        } else {
            None
        };
        
        let span = start.join(name_span);
        Ok(TraitMethod {
            name,
            params,
            body,
            span,
        })
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
        // Allow keywords as parameter names (e.g., actor, type, etc.)
        let params = self.parse_parameters_allow_keywords()?;
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
        self.parse_parameters_impl(false)
    }

    fn parse_parameters_allow_keywords(&mut self) -> ParseResult<Vec<Parameter>> {
        self.parse_parameters_impl(true)
    }

    fn parse_parameters_impl(&mut self, allow_keywords: bool) -> ParseResult<Vec<Parameter>> {
        let mut params = Vec::new();
        if self.matches(TokenKind::RParen) {
            return Ok(params);
        }
        loop {
            let (name, span) = if allow_keywords {
                self.consume_identifier_or_keyword()?
            } else {
                self.consume_identifier()?
            };
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
                    // Save previous trailing expression as a statement before replacing
                    if let Some(prev) = trailing_value.take() {
                        statements.push(Statement::Expression(prev));
                    }
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
        // Check for self.field is value pattern (field assignment in store methods)
        if self.peek_is_self_field_assignment() {
            return self.parse_self_field_assignment();
        }
        // S1.5: self.field += value (augmented field assignment)
        if self.peek_is_self_field_augmented_assign() {
            return self.parse_self_field_augmented_assign();
        }
        let stmt = match self.peek_kind() {
            TokenKind::Identifier(_) if self.peek_is_binding() => {
                Statement::Binding(self.parse_binding()?)
            }
            // S1.5: Augmented assignment — `x += 1` desugars to `x is x + 1`
            TokenKind::Identifier(_) if self.peek_is_augmented_assign() => {
                return self.parse_augmented_assign();
            }
            // S2.4: List destructuring — [a, b, c] is expr
            TokenKind::LBracket if self.peek_is_pattern_binding() => {
                return self.parse_pattern_binding();
            }
            // S2.4: Constructor destructuring — Some(v) is expr
            TokenKind::Identifier(_) if self.peek_is_constructor_pattern_binding() => {
                return self.parse_pattern_binding();
            }
            TokenKind::KeywordReturn => {
                let span = self.current_span();
                self.advance(); // consume 'return'
                // S5.6: `return if cond` / `return unless cond` — bare return wrapped by postfix handler
                if self.check(TokenKind::KeywordIf) || self.check(TokenKind::KeywordUnless) {
                    Statement::Return(Expression::None(span), span)
                } else {
                    let expr = self.parse_expression()?;
                    Statement::Return(expr, span)
                }
            }
            TokenKind::KeywordIf => {
                return self.parse_if_statement();
            }
            TokenKind::KeywordUnless => {
                return self.parse_unless_statement();
            }
            TokenKind::KeywordWhile => {
                return self.parse_while_statement();
            }
            TokenKind::KeywordUntil => {
                return self.parse_until_statement();
            }
            TokenKind::KeywordLoop => {
                return self.parse_loop_statement();
            }
            TokenKind::KeywordFor => {
                return self.parse_for_statement();
            }
            TokenKind::KeywordBreak => {
                let span = self.current_span();
                self.advance();
                Statement::Break(span)
            }
            TokenKind::KeywordContinue => {
                let span = self.current_span();
                self.advance();
                Statement::Continue(span)
            }
            _ => Statement::Expression(self.parse_expression()?),
        };
        // S5.6: Postfix `if` / `unless` — wraps the preceding statement in a conditional.
        // `log("warning") if debug_mode` → `if debug_mode { log("warning") }`
        // `exit(1) unless valid` → `if not valid { exit(1) }`
        if self.check(TokenKind::KeywordIf) || self.check(TokenKind::KeywordUnless) {
            let is_unless = self.check(TokenKind::KeywordUnless);
            self.advance(); // consume 'if' or 'unless'
            let raw_condition = self.parse_expression()?;
            let condition = if is_unless {
                Expression::Unary {
                    op: UnaryOp::Not,
                    span: raw_condition.span(),
                    expr: Box::new(raw_condition),
                }
            } else {
                raw_condition
            };
            let stmt_span = match &stmt {
                Statement::Return(_, s) | Statement::Break(s) | Statement::Continue(s) => *s,
                Statement::Expression(e) => e.span(),
                Statement::If { span, .. } | Statement::While { span, .. }
                | Statement::For { span, .. } | Statement::ForKV { span, .. }
                | Statement::ForRange { span, .. } => *span,
                Statement::Binding(b) => b.span,
                Statement::FieldAssign { span, .. } => *span,
                Statement::PatternBinding { span, .. } => *span,
            };
            let body = Block {
                statements: vec![stmt],
                value: None,
                span: stmt_span,
            };
            let span = stmt_span.join(condition.span());
            return Ok(Statement::If {
                condition,
                body,
                elif_branches: vec![],
                else_body: None,
                span,
            });
        }
        // Guard statement: `condition ? body` desugars to `if condition { body }`
        // Also handles binding-as-condition: `x is val ? body` → `if (x == val) { body }`
        if self.check(TokenKind::Question) {
            let guard_span = self.current_span();
            self.advance(); // consume ?
            let condition = match stmt {
                Statement::Binding(ref binding) => {
                    // Reinterpret `name is value` binding as `name == value` equality check
                    let name_len = binding.name.len();
                    let name_span = Span::new(binding.span.start, binding.span.start + name_len);
                    Expression::Binary {
                        op: BinaryOp::Equals,
                        left: Box::new(Expression::Identifier(binding.name.clone(), name_span)),
                        right: Box::new(binding.value.clone()),
                        span: binding.span,
                    }
                }
                Statement::Expression(expr) => expr,
                _ => return Err(Diagnostic::new(
                    "guard `?` can only follow an expression or binding".to_string(),
                    guard_span,
                )),
            };
            // Parse the guard body: either a single inline statement or an indented block
            let body = if self.check(TokenKind::Newline) || self.check(TokenKind::Indent) {
                self.skip_newlines();
                self.parse_block()?
            } else {
                let body_stmt = self.parse_statement()?;
                let body_span = match &body_stmt {
                    Statement::Return(_, s) | Statement::Break(s) | Statement::Continue(s) => *s,
                    Statement::Expression(e) => e.span(),
                    Statement::If { span, .. } | Statement::While { span, .. } | Statement::For { span, .. } | Statement::ForKV { span, .. } | Statement::ForRange { span, .. } => *span,
                    Statement::Binding(b) => b.span,
                    Statement::FieldAssign { span, .. } => *span,
                    Statement::PatternBinding { span, .. } => *span,
                };
                Block {
                    statements: vec![body_stmt],
                    value: None,
                    span: body_span,
                }
            };
            let span = condition.span().join(body.span);
            return Ok(Statement::If {
                condition,
                body,
                elif_branches: vec![],
                else_body: None,
                span,
            });
        }
        self.skip_newlines();
        Ok(stmt)
    }

    /// Parse `if condition\n  body\n[elif condition\n  body\n]*[else\n  body\n]`
    fn parse_if_statement(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        self.expect(TokenKind::KeywordIf, "expected 'if'")?;
        let condition = self.parse_expression()?;
        self.skip_newlines();
        let body = self.parse_block()?;

        let mut elif_branches = Vec::new();
        while self.check(TokenKind::KeywordElif) {
            self.advance(); // consume 'elif'
            let elif_cond = self.parse_expression()?;
            self.skip_newlines();
            let elif_body = self.parse_block()?;
            elif_branches.push((elif_cond, elif_body));
        }

        let else_body = if self.check(TokenKind::KeywordElse) {
            self.advance(); // consume 'else'
            self.skip_newlines();
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(Statement::If {
            condition,
            body,
            elif_branches,
            else_body,
            span,
        })
    }

    /// Parse `while condition\n  body`
    fn parse_while_statement(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        self.expect(TokenKind::KeywordWhile, "expected 'while'")?;
        let condition = self.parse_expression()?;
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Statement::While {
            condition,
            body,
            span,
        })
    }

    /// Parse `unless condition\n  body` — desugars to `if !(condition) { body }`
    fn parse_unless_statement(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        self.expect(TokenKind::KeywordUnless, "expected 'unless'")?;
        let raw_condition = self.parse_expression()?;
        let condition = Expression::Unary {
            op: UnaryOp::Not,
            span: raw_condition.span(),
            expr: Box::new(raw_condition),
        };
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Statement::If {
            condition,
            body,
            elif_branches: vec![],
            else_body: None,
            span,
        })
    }

    /// Parse `until condition\n  body` — desugars to `while !(condition) { body }`
    fn parse_until_statement(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        self.expect(TokenKind::KeywordUntil, "expected 'until'")?;
        let raw_condition = self.parse_expression()?;
        let condition = Expression::Unary {
            op: UnaryOp::Not,
            span: raw_condition.span(),
            expr: Box::new(raw_condition),
        };
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Statement::While {
            condition,
            body,
            span,
        })
    }

    /// Parse `loop\n  body` — desugars to `while true { body }`
    fn parse_loop_statement(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        self.expect(TokenKind::KeywordLoop, "expected 'loop'")?;
        let condition = Expression::Bool(true, span);
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Statement::While {
            condition,
            body,
            span,
        })
    }

    /// Parse `for variable in iterable\n  body`
    fn parse_for_statement(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        self.expect(TokenKind::KeywordFor, "expected 'for'")?;
        let variable = match self.peek_kind() {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                name
            }
            _ => {
                return Err(Diagnostic::new(
                    "expected variable name after 'for'".to_string(),
                    self.current_span(),
                ));
            }
        };
        
        // Check for key-value syntax: `for key, value in map`
        if self.matches(TokenKind::Comma) {
            let value_var = match self.peek_kind() {
                TokenKind::Identifier(name) => {
                    let name = name.clone();
                    self.advance();
                    name
                }
                _ => {
                    return Err(Diagnostic::new(
                        "expected second variable name after ',' in for".to_string(),
                        self.current_span(),
                    ));
                }
            };
            self.expect(TokenKind::KeywordIn, "expected 'in' after for variables")?;
            let iterable = self.parse_expression()?;
            self.skip_newlines();
            let body = self.parse_block()?;
            return Ok(Statement::ForKV {
                key_var: variable,
                value_var,
                iterable,
                body,
                span,
            });
        }
        
        self.expect(TokenKind::KeywordIn, "expected 'in' after for variable")?;
        let start_expr = self.parse_expression()?;
        
        // Check for range syntax: `for i in start to end [step s]`
        if self.matches(TokenKind::KeywordTo) {
            let end_expr = self.parse_expression()?;
            let step_expr = if self.matches(TokenKind::KeywordStep) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            self.skip_newlines();
            let body = self.parse_block()?;
            return Ok(Statement::ForRange {
                variable,
                start: start_expr,
                end: end_expr,
                step: step_expr,
                body,
                span,
            });
        }
        
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Statement::For {
            variable,
            iterable: start_expr,
            body,
            span,
        })
    }
    
    /// Check if we have a `self.field is value` pattern
    fn peek_is_self_field_assignment(&self) -> bool {
        // Look for: Identifier("self") Dot Identifier(_) KeywordIs
        let tok0 = self.tokens.get(self.index);
        let tok1 = self.tokens.get(self.index + 1);
        let tok2 = self.tokens.get(self.index + 2);
        let tok3 = self.tokens.get(self.index + 3);
        
        matches!(
            (tok0, tok1, tok2, tok3),
            (
                Some(Token { kind: TokenKind::Identifier(name), .. }),
                Some(Token { kind: TokenKind::Dot, .. }),
                Some(Token { kind: TokenKind::Identifier(_), .. }),
                Some(Token { kind: TokenKind::KeywordIs, .. }),
            ) if name == "self"
        )
    }
    
    /// Parse `self.field is value` as a field assignment statement
    fn parse_self_field_assignment(&mut self) -> ParseResult<Statement> {
        let start_span = self.current_span();
        self.advance(); // consume 'self'
        self.advance(); // consume '.'
        let (field_name, _) = self.consume_identifier()?;
        self.expect(TokenKind::KeywordIs, "expected `is`")?;
        self.skip_newlines();
        let value = self.parse_expression()?;
        let span = start_span.join(value.span());
        
        let self_expr = Expression::Identifier("self".to_string(), start_span);
        
        self.skip_newlines();
        Ok(Statement::FieldAssign {
            target: self_expr,
            field: field_name,
            value,
            span,
        })
    }

    // ─── S1.5: Augmented Assignment ────────────────────────────────

    fn is_augmented_assign_token(kind: &TokenKind) -> Option<BinaryOp> {
        match kind {
            TokenKind::PlusEquals => Some(BinaryOp::Add),
            TokenKind::MinusEquals => Some(BinaryOp::Sub),
            TokenKind::StarEquals => Some(BinaryOp::Mul),
            TokenKind::SlashEquals => Some(BinaryOp::Div),
            _ => None,
        }
    }

    /// Check if current position is `identifier +=` (or `-=`, `*=`, `/=`)
    fn peek_is_augmented_assign(&self) -> bool {
        matches!(
            self.tokens.get(self.index + 1).map(|t| &t.kind),
            Some(TokenKind::PlusEquals)
            | Some(TokenKind::MinusEquals)
            | Some(TokenKind::StarEquals)
            | Some(TokenKind::SlashEquals)
        )
    }

    /// Parse `x += expr` and desugar to `x is x + expr` (Binding)
    fn parse_augmented_assign(&mut self) -> ParseResult<Statement> {
        let (name, name_span) = self.consume_identifier()?;
        let op_token = self.advance(); // consume +=, -=, *=, /=
        let op = Self::is_augmented_assign_token(&op_token.kind)
            .expect("augmented assign token");
        self.skip_newlines();
        let rhs = self.parse_expression()?;
        let span = name_span.join(rhs.span());
        let value = Expression::Binary {
            op,
            left: Box::new(Expression::Identifier(name.clone(), name_span)),
            right: Box::new(rhs),
            span,
        };
        Ok(Statement::Binding(Binding {
            name,
            type_annotation: None,
            value,
            span,
        }))
    }

    /// Check if current position is `self.field +=` (or `-=`, `*=`, `/=`)
    fn peek_is_self_field_augmented_assign(&self) -> bool {
        let tok0 = self.tokens.get(self.index);
        let tok1 = self.tokens.get(self.index + 1);
        let tok2 = self.tokens.get(self.index + 2);
        let tok3 = self.tokens.get(self.index + 3);
        
        let is_self_dot_field = matches!(
            (tok0, tok1, tok2),
            (
                Some(Token { kind: TokenKind::Identifier(name), .. }),
                Some(Token { kind: TokenKind::Dot, .. }),
                Some(Token { kind: TokenKind::Identifier(_), .. }),
            ) if name == "self"
        );
        if !is_self_dot_field { return false; }
        matches!(
            tok3.map(|t| &t.kind),
            Some(TokenKind::PlusEquals)
            | Some(TokenKind::MinusEquals)
            | Some(TokenKind::StarEquals)
            | Some(TokenKind::SlashEquals)
        )
    }

    /// Parse `self.field += expr` and desugar to `self.field is self.field + expr`
    fn parse_self_field_augmented_assign(&mut self) -> ParseResult<Statement> {
        let start_span = self.current_span();
        self.advance(); // consume 'self'
        self.advance(); // consume '.'
        let (field_name, field_span) = self.consume_identifier()?;
        let op_token = self.advance(); // consume +=, -=, *=, /=
        let op = Self::is_augmented_assign_token(&op_token.kind)
            .expect("augmented assign token");
        self.skip_newlines();
        let rhs = self.parse_expression()?;
        let span = start_span.join(rhs.span());

        let self_expr = Expression::Identifier("self".to_string(), start_span);
        // Build `self.field` as the existing value
        let field_access = Expression::Member {
            target: Box::new(self_expr.clone()),
            property: field_name.clone(),
            span: start_span.join(field_span),
        };
        let value = Expression::Binary {
            op,
            left: Box::new(field_access),
            right: Box::new(rhs),
            span,
        };

        self.skip_newlines();
        Ok(Statement::FieldAssign {
            target: self_expr,
            field: field_name,
            value,
            span,
        })
    }

    fn parse_expression(&mut self) -> ParseResult<Expression> {
        self.parse_ternary_or_propagate()
    }

    /// Parse ternary expressions and error propagation:
    /// - `cond ? then ! else` - ternary conditional
    /// - `expr ! return err` - error propagation (return if expr is error)
    /// - `cond ! err Name` - guard clause (return error if cond is false)
    fn parse_ternary_or_propagate(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_pipeline()?;
        if self.check(TokenKind::Question) {
            // Check what follows the `?` to distinguish ternary from guard statement
            let is_guard = match self.tokens.get(self.index + 1).map(|t| &t.kind) {
                Some(TokenKind::KeywordReturn)
                | Some(TokenKind::KeywordBreak)
                | Some(TokenKind::KeywordContinue)
                | Some(TokenKind::Newline) => true,
                _ => false,
            };
            if !is_guard {
                // Ternary: cond ? then ! else
                self.advance(); // consume ?
                // For then branch, use parse_pipeline to avoid consuming `! err` as guard clause
                let then_branch = self.parse_pipeline()?;
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
            // else: guard — don't consume ?, let statement parser handle it
        } else if self.check(TokenKind::Bang) && self.check_ahead(1, TokenKind::KeywordReturn) {
            // Error propagation: expr ! return err
            // Only consume if we see `! return` to avoid conflict with ternary `!`
            self.advance(); // consume `!`
            self.advance(); // consume `return`
            self.expect(TokenKind::KeywordErr, "expected `err` after `! return` in error propagation")?;
            let err_span = self.previous_span();
            let span = expr.span().join(err_span);
            expr = Expression::ErrorPropagate {
                expr: Box::new(expr),
                span,
            };
        } else if self.check(TokenKind::Bang) && self.check_ahead(1, TokenKind::KeywordErr) {
            // Guard clause: cond ! err Name
            // Desugars to: cond ? () ! err Name (but we generate a proper Ternary)
            // Actually: if cond is false, return the error. So: cond ? none ! err Name
            self.advance(); // consume `!`
            self.advance(); // consume `err`
            // Parse the error name (taxonomy path)
            let path = self.parse_error_name()?;
            let err_span = self.previous_span();
            let span = expr.span().join(err_span);
            // Create: condition ? none ! err Name
            // Where `none` means "continue with unit value" if condition is true
            let then_span = Span::new(span.start, span.start); // tiny span
            expr = Expression::Ternary {
                condition: Box::new(expr),
                then_branch: Box::new(Expression::None(then_span)),
                else_branch: Box::new(Expression::ErrorValue { path, span: err_span }),
                span,
            };
        }
        Ok(expr)
    }

    /// Parse pipeline expressions: `expr ~ fn(args)` => `fn(expr, args)`
    /// Left-associative, lower precedence than logic operators
    fn parse_pipeline(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_logic_or()?;
        while self.matches(TokenKind::Tilde) {
            let rhs = self.parse_logic_or()?;
            let span = expr.span().join(rhs.span());
            expr = Expression::Pipeline {
                left: Box::new(expr),
                right: Box::new(rhs),
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
            if self.matches(TokenKind::KeywordIs) {
                let rhs = self.parse_comparison()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op: BinaryOp::Equals,
                    left: Box::new(expr),
                    right: Box::new(rhs),
                    span,
                };
            } else if self.matches(TokenKind::KeywordIsnt) {
                let rhs = self.parse_comparison()?;
                let span = expr.span().join(rhs.span());
                expr = Expression::Binary {
                    op: BinaryOp::NotEquals,
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
        if self.check(TokenKind::Bang) && matches!(self.peek_next_kind(), Some(TokenKind::BangBang)) {
            let bang_span = self.current_span();
            self.advance();
            let value = self.parse_unary()?;
            let span = bang_span.join(value.span());
            return Ok(Expression::Throw {
                value: Box::new(value),
                span,
            });
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
        // Note: ~ is now the pipeline operator, BitNot is removed from Coral
        // (Use integer methods or bitwise functions instead)
        self.parse_call()
    }

    fn parse_call(&mut self) -> ParseResult<Expression> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.matches(TokenKind::LParen) {
                let (args, arg_names) = self.parse_arguments()?;
                let span = expr.span().join(self.previous_span());
                expr = Expression::Call {
                    callee: Box::new(expr),
                    args,
                    arg_names,
                    span,
                };
            } else if self.matches(TokenKind::Dot) {
                let (name, span) = self.consume_identifier_or_keyword()?;
                let span = expr.span().join(span);
                expr = Expression::Member {
                    target: Box::new(expr),
                    property: name,
                    span,
                };
            } else if self.matches(TokenKind::LBracket) {
                let start_expr = self.parse_expression()?;
                // S2.5: Check for `to` keyword → slice syntax
                if self.matches(TokenKind::KeywordTo) {
                    let end_expr = self.parse_expression()?;
                    let end_span = self.current_span();
                    self.expect(TokenKind::RBracket, "expected `]` to close slice")?;
                    let span = expr.span().join(end_span);
                    expr = Expression::Slice {
                        target: Box::new(expr),
                        start: Box::new(start_expr),
                        end: Box::new(end_expr),
                        span,
                    };
                } else {
                    let end_span = self.current_span();
                    self.expect(TokenKind::RBracket, "expected `]` to close subscript")?;
                    let span = expr.span().join(end_span);
                    expr = Expression::Index {
                        target: Box::new(expr),
                        index: Box::new(start_expr),
                        span,
                    };
                }
            } else {
                break;
            }
        }
        Ok(expr)
    }

    /// Parse function call arguments, supporting both positional and named args.
    /// Named args use `name: value` syntax. Positional args must come before named args.
    fn parse_arguments(&mut self) -> ParseResult<(Vec<Expression>, Vec<Option<String>>)> {
        let mut args = Vec::new();
        let mut arg_names: Vec<Option<String>> = Vec::new();
        let mut seen_named = false;
        
        // Handle empty argument list
        if self.matches(TokenKind::RParen) {
            return Ok((args, arg_names));
        }
        
        // Check if arguments are indented (multiline)
        let is_indented = if self.check(TokenKind::Newline) {
            self.advance(); // consume Newline
            if self.check(TokenKind::Indent) {
                self.advance(); // consume Indent
                self.layout_depth += 1;
                true
            } else {
                false
            }
        } else {
            false
        };
        
        loop {
            // In indented mode, skip additional newlines
            if is_indented {
                while self.check(TokenKind::Newline) {
                    self.advance();
                }
                
                // Check for dedent (end of indented arguments)
                if self.check(TokenKind::Dedent) {
                    self.leave_layout_block(self.current_span());
                    self.advance(); // consume Dedent
                    break;
                }
            }
            
            // S4.1: Check for named argument: `identifier: expression`
            // Look ahead for Identifier followed by Colon (not inside taxonomy path)
            let is_named_arg = if let TokenKind::Identifier(_) = self.peek_kind() {
                // Check if next-next token is a Colon
                self.index + 1 < self.tokens.len() && self.tokens[self.index + 1].kind == TokenKind::Colon
            } else {
                false
            };
            
            if is_named_arg {
                let (name, _name_span) = self.consume_identifier_or_keyword()?;
                self.advance(); // consume Colon
                let value = self.parse_expression()?;
                args.push(value);
                arg_names.push(Some(name));
                seen_named = true;
            } else {
                if seen_named {
                    return Err(Diagnostic::new(
                        "positional arguments must come before named arguments",
                        self.current_span(),
                    ));
                }
                // Parse the argument expression
                args.push(self.parse_expression()?);
                arg_names.push(None);
            }
            
            // Handle comma separator
            if self.matches(TokenKind::Comma) {
                continue;
            }
            
            // Handle end of arguments
            if is_indented {
                // In indented mode, expect dedent before rparen
                while self.check(TokenKind::Newline) {
                    self.advance();
                }
                if self.check(TokenKind::Dedent) {
                    self.leave_layout_block(self.current_span());
                    self.advance(); // consume Dedent
                }
            }
            
            // Expect closing parenthesis
            self.expect(TokenKind::RParen, "expected `)` to close arguments")?;
            break;
        }
        
        // Only include arg_names if there are named args
        if !seen_named {
            arg_names.clear();
        }
        
        Ok((args, arg_names))
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
            TokenKind::KeywordNone => {
                let token = self.advance();
                Ok(Expression::None(token.span))
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
                Err(self.error_here("unexpected `*` in expression"))
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
            // Allow keywords as identifiers in expression position.
            // NOTE: KeywordMatch, KeywordUnsafe, KeywordAsm have special semantics
            // and are handled below with their own cases.
            TokenKind::KeywordActor
            | TokenKind::KeywordStore
            | TokenKind::KeywordPersist
            | TokenKind::KeywordType
            | TokenKind::KeywordFn
            | TokenKind::KeywordIs
            | TokenKind::KeywordIsnt
            | TokenKind::KeywordExtern
            | TokenKind::KeywordPtr
            | TokenKind::KeywordEnum
            | TokenKind::KeywordTrait
            | TokenKind::KeywordWith
            | TokenKind::KeywordTo
            | TokenKind::KeywordStep
            | TokenKind::KeywordIn => {
                let (name, span) = self.consume_identifier_or_keyword()?;
                Ok(Expression::Identifier(name, span))
            }
            TokenKind::KeywordMatch => self.parse_match_expression(),
            TokenKind::KeywordWhen => self.parse_when_expression(),
            TokenKind::KeywordUnsafe => self.parse_unsafe_block(),
            TokenKind::KeywordAsm => self.parse_inline_asm(),
            TokenKind::KeywordErr => self.parse_error_value(),
            TokenKind::LParen => {
                let start = self.advance().span;
                self.skip_newlines();
                if self.matches(TokenKind::RParen) {
                    return Ok(Expression::Unit);
                }
                let first = self.parse_expression()?;
                // S2.7: Tuple syntax — (a, b, c) parsed as list literal
                if self.matches(TokenKind::Comma) {
                    let mut items = vec![first];
                    self.skip_newlines();
                    // Handle trailing comma: (a,) is single-element tuple
                    if !self.check(TokenKind::RParen) {
                        loop {
                            items.push(self.parse_expression()?);
                            self.skip_newlines();
                            if !self.matches(TokenKind::Comma) {
                                break;
                            }
                            self.skip_newlines();
                            if self.check(TokenKind::RParen) {
                                break;
                            }
                        }
                    }
                    let end = self.expect(TokenKind::RParen, "expected closing )")?.span;
                    return Ok(Expression::List(items, start.join(end)));
                }
                self.expect(TokenKind::RParen, "expected closing )")?;
                Ok(first)
            }
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::At => self.parse_ptr_load(),
            other => {
                // Debug: dump surrounding tokens
                let start = if self.index > 10 { self.index - 10 } else { 0 };
                let end = std::cmp::min(self.index + 5, self.tokens.len());
                eprintln!("=== TOKEN DUMP at index {} ===", self.index);
                for i in start..end {
                    let t = &self.tokens[i];
                    let marker = if i == self.index { ">>>" } else { "   " };
                    eprintln!("{} [{}] {:?} @ {:?}", marker, i, t.kind, t.span);
                }
                Err(self.error_here(&format!("unexpected token in expression: {:?}", other)))
            }
        }
    }

    /// Parse error name path: `Name` or `Name:SubName:SubSubName`
    /// Returns the path segments (Vec<String>)
    fn parse_error_name(&mut self) -> ParseResult<Vec<String>> {
        // Check for naked error (no name following)
        if !matches!(self.peek_kind(), TokenKind::Identifier(_)) {
            return Ok(vec![]); // Empty path = generic error
        }
        
        let (first, _) = self.consume_identifier()?;
        let mut path = vec![first];
        
        // Parse colon-separated path: Name:SubName:SubSubName
        while self.matches(TokenKind::Colon) {
            let (segment, _) = self.consume_identifier()?;
            path.push(segment);
        }
        
        Ok(path)
    }

    /// Parse error value expression: `err Name` or `err Name:SubName:SubSubName`
    fn parse_error_value(&mut self) -> ParseResult<Expression> {
        let start = self.advance().span; // consume `err`
        let path = self.parse_error_name()?;
        let end_span = self.previous_span();
        let span = if path.is_empty() { start } else { start.join(end_span) };
        
        Ok(Expression::ErrorValue { path, span })
    }

    fn parse_map_literal(&mut self, name_span: Span) -> ParseResult<Expression> {
        let open = self.expect(TokenKind::LParen, "expected `(` after map literal")?;
        let start = name_span.join(open.span);
        
        // Handle indented map entries
        self.skip_newlines();
        let is_indented = if self.check(TokenKind::Indent) {
            self.advance(); // consume Indent
            self.layout_depth += 1;
            true
        } else {
            false
        };
        
        let mut entries = Vec::new();
        if self.matches(TokenKind::RParen) {
            return Ok(Expression::Map(entries, start.join(self.previous_span())));
        }
        
        loop {
            // Handle indented entries
            if is_indented {
                self.skip_newlines();
                // Check for dedent (end of indented entries)
                if self.check(TokenKind::Dedent) {
                    self.leave_layout_block(self.current_span());
                    self.advance(); // consume Dedent
                    break;
                }
            } else {
                self.skip_newlines();
            }
            
            let key = self.parse_unary()?;
            self.skip_newlines();
            // Accept both `:` and `is` as map key-value separator (`:` preferred)
            if !self.matches(TokenKind::Colon) {
                self.expect(TokenKind::KeywordIs, "expected `:` or `is` between map key and value")?;
            }
            self.skip_newlines();
            let value = self.parse_expression()?;
            
            // S2.3: Map comprehension — map(key: value for var in iterable if cond)
            if entries.is_empty() && self.check(TokenKind::KeywordFor) {
                self.advance(); // consume `for`
                let (var, _) = self.consume_identifier()?;
                self.expect(TokenKind::KeywordIn, "expected `in` after comprehension variable")?;
                let iterable = self.parse_expression()?;
                let condition = if self.check(TokenKind::KeywordIf) {
                    self.advance();
                    Some(Box::new(self.parse_expression()?))
                } else {
                    None
                };
                let end = self.expect(TokenKind::RParen, "expected `)` to close map comprehension")?.span;
                return Ok(Expression::MapComprehension {
                    key: Box::new(key),
                    value: Box::new(value),
                    var,
                    iterable: Box::new(iterable),
                    condition,
                    span: start.join(end),
                });
            }
            
            entries.push((key, value));
            
            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
                continue;
            }
            
            // Handle end of map
            if is_indented {
                self.skip_newlines();
                if self.check(TokenKind::Dedent) {
                    self.leave_layout_block(self.current_span());
                    self.advance(); // consume Dedent
                }
            }
            
            let end = self.expect(TokenKind::RParen, "expected `)` to close map literal")?.span;
            return Ok(Expression::Map(entries, start.join(end)));
        }
        
        // If we exited via dedent, still expect the closing paren
        let end = self.expect(TokenKind::RParen, "expected `)` to close map literal")?.span;
        Ok(Expression::Map(entries, start.join(end)))
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
        // Safe: we checked !parts.is_empty() above and inserted at least one element
        let mut acc = iter.next().expect("template parts must be non-empty after processing");
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
        
        // Handle indented list items
        self.skip_newlines();
        let is_indented = if self.check(TokenKind::Indent) {
            self.advance(); // consume Indent
            self.layout_depth += 1;
            true
        } else {
            false
        };
        
        loop {
            // Handle indented items
            if is_indented {
                self.skip_newlines();
                // Check for dedent (end of indented items)
                if self.check(TokenKind::Dedent) {
                    self.leave_layout_block(self.current_span());
                    self.advance(); // consume Dedent
                    break;
                }
            } else {
                self.skip_newlines();
            }
            
            // S2.6: Spread operator — ...expr
            if self.check(TokenKind::Ellipsis) {
                let spread_span = self.advance().span;
                let inner = self.parse_expression()?;
                let span = spread_span.join(inner.span());
                items.push(Expression::Spread(Box::new(inner), span));
            } else {
                items.push(self.parse_expression()?);
            }

            // S2.2: List comprehension — [body for var in iterable if cond]
            if items.len() == 1 && self.check(TokenKind::KeywordFor) {
                self.advance(); // consume `for`
                let (var, _) = self.consume_identifier()?;
                self.expect(TokenKind::KeywordIn, "expected `in` after comprehension variable")?;
                let iterable = self.parse_expression()?;
                let condition = if self.check(TokenKind::KeywordIf) {
                    self.advance(); // consume `if`
                    Some(Box::new(self.parse_expression()?))
                } else {
                    None
                };
                let end = self.expect(TokenKind::RBracket, "expected `]` to close list comprehension")?.span;
                return Ok(Expression::ListComprehension {
                    body: Box::new(items.pop().unwrap()),
                    var,
                    iterable: Box::new(iterable),
                    condition,
                    span: start.join(end),
                });
            }
            
            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
                continue;
            }
            
            // Handle end of list
            if is_indented {
                self.skip_newlines();
                if self.check(TokenKind::Dedent) {
                    self.leave_layout_block(self.current_span());
                    self.advance(); // consume Dedent
                }
            }
            
            let end = self.expect(TokenKind::RBracket, "expected ]")?.span;
            return Ok(Expression::List(items, start.join(end)));
        }
        
        // If we exited via dedent, still expect the closing bracket
        let end = self.expect(TokenKind::RBracket, "expected ]")?.span;
        Ok(Expression::List(items, start.join(end)))
    }

    /// Parse `when` expression — desugars to nested ternary expressions.
    /// Syntax:
    /// ```text
    /// when
    ///   condition1 ? value1
    ///   condition2 ? value2
    ///   _          ? default_value
    /// ```
    /// Desugars to: `condition1 ? value1 ! (condition2 ? value2 ! default_value)`
    fn parse_when_expression(&mut self) -> ParseResult<Expression> {
        let when_span = self.advance().span; // consume 'when'
        self.expect(TokenKind::Newline, "expected newline after 'when'")?;
        let arms_start = match self.consume_indent_with_recovery(
            "expected indented when arms",
            "Indent each when arm under the when expression",
        ) {
            Some(span) => span,
            None => {
                return Ok(Expression::None(when_span));
            }
        };
        
        let mut arms: Vec<(Expression, Expression)> = Vec::new();
        let mut default_expr: Option<Expression> = None;
        
        loop {
            self.skip_newlines();
            if self.check(TokenKind::Dedent) {
                let span = self.current_span();
                self.leave_layout_block(span);
                self.advance();
                break;
            }
            if self.check(TokenKind::Eof) {
                self.report_missing_dedent(arms_start, "missing dedent to close when arms");
                break;
            }
            // Check for wildcard/default arm: `_ ? value`
            if self.check(TokenKind::Identifier("_".into())) {
                let tok = self.peek_kind().clone();
                if let TokenKind::Identifier(name) = &tok {
                    if name == "_" {
                        self.advance(); // consume _
                        self.expect(TokenKind::Question, "expected '?' after '_' in when arm")?;
                        default_expr = Some(self.parse_expression()?);
                        self.skip_newlines();
                        continue;
                    }
                }
            }
            // Normal arm: `condition ? value`
            let condition = self.parse_pipeline()?;
            self.expect(TokenKind::Question, "expected '?' in when arm")?;
            let value = self.parse_expression()?;
            arms.push((condition, value));
            self.skip_newlines();
        }
        
        // Build the expression from the bottom up (right fold into nested ternaries)
        let none_expr = Expression::None(when_span);
        let base = default_expr.unwrap_or(none_expr);
        
        let result = arms.into_iter().rev().fold(base, |else_branch, (condition, then_branch)| {
            let span = condition.span().join(else_branch.span());
            Expression::Ternary {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
                span,
            }
        });
        
        Ok(result)
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
                // S3.1: Default arm also supports multi-statement blocks
                let body = if self.check(TokenKind::Newline) {
                    self.advance(); // consume newline
                    if self.check(TokenKind::Indent) {
                        self.parse_block()?
                    } else {
                        Block {
                            statements: vec![],
                            value: None,
                            span: self.current_span(),
                        }
                    }
                } else {
                    let expr = self.parse_expression()?;
                    Block::from_expression(expr)
                };
                default = Some(Box::new(body));
                self.skip_newlines();
                continue;
            }
            let pattern = self.parse_match_pattern()?;
            // S3.3: Or-patterns — `Pat1 or Pat2 or Pat3 ? body`
            let pattern = if self.check(TokenKind::KeywordOr) {
                let mut alternatives = vec![pattern];
                while self.matches(TokenKind::KeywordOr) {
                    alternatives.push(self.parse_match_pattern()?);
                }
                MatchPattern::Or(alternatives)
            } else {
                pattern
            };
            // S3.2: Optional guard clause — `Pattern if condition ? body`
            // Use parse_pipeline to avoid consuming the `?` as a ternary operator
            let guard = if self.check(TokenKind::KeywordIf) {
                self.advance(); // consume `if`
                Some(Box::new(self.parse_pipeline()?))
            } else {
                None
            };
            self.expect(TokenKind::Question, "expected `?` in match arm")?;
            // S3.1: Multi-statement match arms — if `?` is followed by a
            // newline + indent, parse a full block (multiple statements).
            // Otherwise, parse a single expression as before.
            let body = if self.check(TokenKind::Newline) {
                self.advance(); // consume newline
                if self.check(TokenKind::Indent) {
                    self.parse_block()?
                } else {
                    // Newline but no indent — empty arm body (unit)
                    Block {
                        statements: vec![],
                        value: None,
                        span: self.current_span(),
                    }
                }
            } else {
                let expr = self.parse_expression()?;
                Block::from_expression(expr)
            };
            arms.push(MatchArm {
                pattern,
                guard,
                body,
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
                    // S3.5: Check for range pattern `start to end`
                    if self.check(TokenKind::KeywordTo) {
                        let start_span = token.span;
                        self.advance(); // consume `to`
                        let end_token = self.advance();
                        if let TokenKind::Integer(end_value) = end_token.kind {
                            return Ok(MatchPattern::Range {
                                start: value,
                                end: end_value,
                                span: start_span.join(end_token.span),
                            });
                        } else {
                            return Err(Diagnostic::new("expected integer after `to` in range pattern", end_token.span));
                        }
                    }
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
                self.advance(); // consume `[`
                let mut patterns = Vec::new();
                while !self.check(TokenKind::RBracket) && !self.check(TokenKind::Eof) {
                    // S3.4: Check for rest/spread pattern `...name`
                    if self.check(TokenKind::Ellipsis) {
                        let ellipsis_span = self.advance().span;
                        let (rest_name, rest_span) = self.consume_identifier()?;
                        patterns.push(MatchPattern::Rest(rest_name, ellipsis_span.join(rest_span)));
                        // Rest must be last in the list pattern
                        if !self.check(TokenKind::RBracket) {
                            self.matches(TokenKind::Comma); // consume optional trailing comma
                        }
                        break;
                    }
                    patterns.push(self.parse_match_pattern()?);
                    if !self.matches(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RBracket, "expected `]` after list pattern")?;
                Ok(MatchPattern::List(patterns))
            }
            TokenKind::Identifier(_) => {
                let (name, name_span) = self.consume_identifier()?;
                
                // Check for wildcard pattern `_`
                if name == "_" {
                    return Ok(MatchPattern::Wildcard(name_span));
                }
                
                // Check if this is a constructor pattern: Name(binding1, binding2, ...)
                if self.check(TokenKind::LParen) {
                    self.advance(); // consume `(`
                    let mut fields = Vec::new();
                    
                    loop {
                        if self.check(TokenKind::RParen) {
                            break;
                        }
                        // Recursively parse nested patterns
                        let pattern = self.parse_match_pattern()?;
                        fields.push(pattern);
                        
                        if !self.matches(TokenKind::Comma) {
                            break;
                        }
                    }
                    
                    let end_span = self.expect(TokenKind::RParen, "expected `)` after constructor pattern")?;
                    let span = name_span.join(end_span.span);
                    
                    return Ok(MatchPattern::Constructor {
                        name,
                        fields,
                        span,
                    });
                }
                
                // Check if this looks like a nullary constructor (capitalized) vs a binding variable
                // Convention: Capitalized names are constructors, lowercase are bindings
                if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    // Nullary constructor like `None`
                    return Ok(MatchPattern::Constructor {
                        name,
                        fields: Vec::new(),
                        span: name_span,
                    });
                }
                
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

    /// Consume an identifier or keyword-as-identifier (for parameter names in extern fn).
    fn consume_identifier_or_keyword(&mut self) -> ParseResult<(String, Span)> {
        let kind = self.peek_kind().clone();
        let span = self.current_span();
        match kind {
            TokenKind::Identifier(name) => {
                self.advance();
                Ok((name, span))
            }
            // Allow keywords as identifiers in extern fn parameters.
            TokenKind::KeywordActor => { self.advance(); Ok(("actor".to_string(), span)) }
            TokenKind::KeywordStore => { self.advance(); Ok(("store".to_string(), span)) }
            TokenKind::KeywordPersist => { self.advance(); Ok(("persist".to_string(), span)) }
            TokenKind::KeywordType => { self.advance(); Ok(("type".to_string(), span)) }
            TokenKind::KeywordMatch => { self.advance(); Ok(("match".to_string(), span)) }
            TokenKind::KeywordFn => { self.advance(); Ok(("fn".to_string(), span)) }
            TokenKind::KeywordAnd => { self.advance(); Ok(("and".to_string(), span)) }
            TokenKind::KeywordOr => { self.advance(); Ok(("or".to_string(), span)) }
            TokenKind::KeywordTrue => { self.advance(); Ok(("true".to_string(), span)) }
            TokenKind::KeywordFalse => { self.advance(); Ok(("false".to_string(), span)) }
            TokenKind::KeywordIs => { self.advance(); Ok(("is".to_string(), span)) }
            TokenKind::KeywordIsnt => { self.advance(); Ok(("isnt".to_string(), span)) }
            TokenKind::KeywordExtern => { self.advance(); Ok(("extern".to_string(), span)) }
            TokenKind::KeywordUnsafe => { self.advance(); Ok(("unsafe".to_string(), span)) }
            TokenKind::KeywordAsm => { self.advance(); Ok(("asm".to_string(), span)) }
            TokenKind::KeywordPtr => { self.advance(); Ok(("ptr".to_string(), span)) }
            TokenKind::KeywordErr => { self.advance(); Ok(("err".to_string(), span)) }
            TokenKind::KeywordNone => { self.advance(); Ok(("none".to_string(), span)) }
            TokenKind::KeywordEnum => { self.advance(); Ok(("enum".to_string(), span)) }
            TokenKind::KeywordReturn => { self.advance(); Ok(("return".to_string(), span)) }
            TokenKind::KeywordTrait => { self.advance(); Ok(("trait".to_string(), span)) }
            TokenKind::KeywordWith => { self.advance(); Ok(("with".to_string(), span)) }
            TokenKind::KeywordIf => { self.advance(); Ok(("if".to_string(), span)) }
            TokenKind::KeywordElif => { self.advance(); Ok(("elif".to_string(), span)) }
            TokenKind::KeywordElse => { self.advance(); Ok(("else".to_string(), span)) }
            TokenKind::KeywordWhile => { self.advance(); Ok(("while".to_string(), span)) }
            TokenKind::KeywordFor => { self.advance(); Ok(("for".to_string(), span)) }
            TokenKind::KeywordIn => { self.advance(); Ok(("in".to_string(), span)) }
            TokenKind::KeywordBreak => { self.advance(); Ok(("break".to_string(), span)) }
            TokenKind::KeywordContinue => { self.advance(); Ok(("continue".to_string(), span)) }
            TokenKind::KeywordTo => { self.advance(); Ok(("to".to_string(), span)) }
            TokenKind::KeywordStep => { self.advance(); Ok(("step".to_string(), span)) }
            TokenKind::KeywordUnless => { self.advance(); Ok(("unless".to_string(), span)) }
            TokenKind::KeywordUntil => { self.advance(); Ok(("until".to_string(), span)) }
            TokenKind::KeywordLoop => { self.advance(); Ok(("loop".to_string(), span)) }
            TokenKind::KeywordWhen => { self.advance(); Ok(("when".to_string(), span)) }
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
                | TokenKind::KeywordPersist
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

    /// S2.4: Check if current `[` starts a destructuring pattern binding `[a, b] is expr`
    fn peek_is_pattern_binding(&self) -> bool {
        // Scan forward from [ to find matching ], then check for `is`
        let mut offset = 1usize;
        let mut depth = 1i32;
        while let Some(token) = self.tokens.get(self.index + offset) {
            match &token.kind {
                TokenKind::LBracket => { depth += 1; offset += 1; }
                TokenKind::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        // Check if next non-newline token is `is`
                        offset += 1;
                        while let Some(t) = self.tokens.get(self.index + offset) {
                            if matches!(t.kind, TokenKind::Newline) { offset += 1; continue; }
                            return matches!(t.kind, TokenKind::KeywordIs);
                        }
                        return false;
                    }
                    offset += 1;
                }
                TokenKind::Newline | TokenKind::Eof => return false,
                _ => { offset += 1; }
            }
        }
        false
    }

    /// S2.4: Check if current identifier is a constructor destructuring `Some(x) is expr`
    fn peek_is_constructor_pattern_binding(&self) -> bool {
        // Must be an uppercase identifier followed by `(`
        if let Some(TokenKind::Identifier(name)) = self.tokens.get(self.index).map(|t| &t.kind) {
            if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                // Look for ( after identifier, skip to matching ), then check for `is`
                if let Some(t) = self.tokens.get(self.index + 1) {
                    if matches!(t.kind, TokenKind::LParen) {
                        let mut offset = 2usize;
                        let mut depth = 1i32;
                        while let Some(token) = self.tokens.get(self.index + offset) {
                            match &token.kind {
                                TokenKind::LParen => { depth += 1; offset += 1; }
                                TokenKind::RParen => {
                                    depth -= 1;
                                    if depth == 0 {
                                        offset += 1;
                                        while let Some(t) = self.tokens.get(self.index + offset) {
                                            if matches!(t.kind, TokenKind::Newline) { offset += 1; continue; }
                                            return matches!(t.kind, TokenKind::KeywordIs);
                                        }
                                        return false;
                                    }
                                    offset += 1;
                                }
                                TokenKind::Newline | TokenKind::Eof => return false,
                                _ => { offset += 1; }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// S2.4: Parse a destructuring pattern binding
    fn parse_pattern_binding(&mut self) -> ParseResult<Statement> {
        let span = self.current_span();
        let pattern = self.parse_match_pattern()?;
        self.expect(TokenKind::KeywordIs, "expected `is` after destructuring pattern")?;
        self.skip_newlines();
        let value = self.parse_expression()?;
        Ok(Statement::PatternBinding { pattern, value, span })
    }

    fn parse_extern_function(&mut self) -> ParseResult<ExternFunction> {
        let start = self.advance().span;
        self.expect(TokenKind::KeywordFn, "expected `fn` after `extern`")?;
        let (name, name_span) = self.consume_identifier()?;
        self.expect(TokenKind::LParen, "expected `(` after extern function name")?;
        let params = self.parse_parameters_allow_keywords()?;
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

    fn peek_kind(&self) -> &TokenKind {
        static EOF: TokenKind = TokenKind::Eof;
        self.tokens
            .get(self.index)
            .map(|t| &t.kind)
            .unwrap_or(&EOF)
    }

    fn peek_next_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.index + 1).map(|t| &t.kind)
    }

    fn check(&self, kind: TokenKind) -> bool {
        matches!(self.tokens.get(self.index), Some(token) if token.kind == kind)
    }

    /// Check if the token at `offset` positions ahead matches the given kind.
    /// `check_ahead(0, ...)` is equivalent to `check(...)`.
    fn check_ahead(&self, offset: usize, kind: TokenKind) -> bool {
        matches!(self.tokens.get(self.index + offset), Some(token) if token.kind == kind)
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
