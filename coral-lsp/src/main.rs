use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use coralc::Compiler;
use coralc::ast::{Function, Item};
use coralc::lexer;
use coralc::module_loader::ModuleLoader;
use coralc::parser::Parser;
use coralc::span::LineIndex;

use std::collections::HashMap;
use std::sync::Mutex;

/// Cached parse state for a document.
struct DocumentState {
    source: String,
    /// Function/type/etc definitions: name → (signature_string, span_start, span_end)
    symbols: Vec<SymbolInfo>,
}

struct SymbolInfo {
    name: String,
    kind: SymbolKind,
    signature: String,
    span_start: usize,
    span_end: usize,
}

struct CoralLanguageServer {
    client: Client,
    compiler: Compiler,
    loader: Mutex<ModuleLoader>,
    documents: Mutex<HashMap<Url, DocumentState>>,
}

fn format_function_signature(f: &Function) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            if let Some(ann) = &p.type_annotation {
                format!("{}: {}", p.name, ann.segments.join("."))
            } else {
                p.name.clone()
            }
        })
        .collect();
    format!("*{}({})", f.name, params.join(", "))
}

/// Convert 0-based LSP Position to byte offset.
fn lsp_offset(source: &str, pos: Position) -> usize {
    let mut line = 0u32;
    let mut offset = 0usize;
    for (i, b) in source.bytes().enumerate() {
        if line == pos.line {
            return i + pos.character as usize;
        }
        if b == b'\n' {
            line += 1;
        }
        offset = i + 1;
    }
    offset
}

fn word_at_offset(source: &str, offset: usize) -> String {
    let bytes = source.as_bytes();
    if offset >= bytes.len() {
        return String::new();
    }
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    if !is_ident(bytes[offset]) {
        return String::new();
    }
    let mut start = offset;
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    source[start..end].to_string()
}

fn extract_symbols(source: &str) -> Vec<SymbolInfo> {
    let mut symbols = Vec::new();
    let tokens = match lexer::lex(source) {
        Ok(t) => t,
        Err(_) => return symbols,
    };
    let parser = Parser::new(tokens, source.len());
    let (program, _errors) = parser.parse_with_recovery();

    for item in &program.items {
        match item {
            Item::Function(f) => {
                symbols.push(SymbolInfo {
                    name: f.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    signature: format_function_signature(f),
                    span_start: f.span.start,
                    span_end: f.span.end,
                });
            }
            Item::Type(t) => {
                let fields: Vec<&str> = t.fields.iter().map(|f| f.name.as_str()).collect();
                symbols.push(SymbolInfo {
                    name: t.name.clone(),
                    kind: SymbolKind::STRUCT,
                    signature: format!("type {} {{ {} }}", t.name, fields.join(", ")),
                    span_start: t.span.start,
                    span_end: t.span.end,
                });
                for method in &t.methods {
                    symbols.push(SymbolInfo {
                        name: method.name.clone(),
                        kind: SymbolKind::METHOD,
                        signature: format_function_signature(method),
                        span_start: method.span.start,
                        span_end: method.span.end,
                    });
                }
            }
            Item::Store(s) => {
                symbols.push(SymbolInfo {
                    name: s.name.clone(),
                    kind: if s.is_actor {
                        SymbolKind::CLASS
                    } else {
                        SymbolKind::STRUCT
                    },
                    signature: format!("{} {}", if s.is_actor { "actor" } else { "store" }, s.name),
                    span_start: s.span.start,
                    span_end: s.span.end,
                });
                for method in &s.methods {
                    symbols.push(SymbolInfo {
                        name: method.name.clone(),
                        kind: SymbolKind::METHOD,
                        signature: format_function_signature(method),
                        span_start: method.span.start,
                        span_end: method.span.end,
                    });
                }
            }
            Item::TraitDefinition(t) => {
                symbols.push(SymbolInfo {
                    name: t.name.clone(),
                    kind: SymbolKind::INTERFACE,
                    signature: format!("trait {}", t.name),
                    span_start: t.span.start,
                    span_end: t.span.end,
                });
            }
            Item::ErrorDefinition(e) => {
                symbols.push(SymbolInfo {
                    name: e.name.clone(),
                    kind: SymbolKind::ENUM,
                    signature: format!("err {}", e.name),
                    span_start: e.span.start,
                    span_end: e.span.end,
                });
            }
            _ => {}
        }
    }
    symbols
}

impl CoralLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            compiler: Compiler,
            loader: Mutex::new(ModuleLoader::with_default_std()),
            documents: Mutex::new(HashMap::new()),
        }
    }

    fn update_document(&self, uri: &Url, source: String) {
        let symbols = extract_symbols(&source);
        let mut docs = self.documents.lock().unwrap();
        docs.insert(uri.clone(), DocumentState { source, symbols });
    }

    fn diagnose(&self, source: &str) -> Vec<Diagnostic> {
        let idx = LineIndex::new(source);
        match self.compiler.compile_to_ir_with_warnings(source) {
            Ok((_ir, warnings)) => warnings
                .iter()
                .map(|w| Diagnostic {
                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("coral".to_string()),
                    message: w.message.clone(),
                    ..Default::default()
                })
                .collect(),
            Err(err) => {
                let span = err.diagnostic.span;
                let (line, col) = idx.line_col(span.start);
                let (end_line, end_col) = idx.line_col(span.end.max(span.start + 1));
                let line = line.saturating_sub(1) as u32;
                let col = col.saturating_sub(1) as u32;
                let end_line = end_line.saturating_sub(1) as u32;
                let end_col = end_col.saturating_sub(1) as u32;
                vec![Diagnostic {
                    range: Range::new(Position::new(line, col), Position::new(end_line, end_col)),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("coral".to_string()),
                    message: err.diagnostic.message.clone(),
                    ..Default::default()
                }]
            }
        }
    }

    fn diagnose_file(&self, uri: &Url) -> Vec<Diagnostic> {
        let path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return vec![],
        };

        let mut loader = self.loader.lock().unwrap();
        loader.clear_cache();
        match loader.load_modules(&path) {
            Ok(module_sources) => {
                let all_source: String = module_sources
                    .iter()
                    .map(|ms| ms.source.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let idx = LineIndex::new(&all_source);

                match self.compiler.compile_modules_to_ir(&module_sources) {
                    Ok((_ir, warnings)) => warnings
                        .iter()
                        .map(|w| Diagnostic {
                            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                            severity: Some(DiagnosticSeverity::WARNING),
                            source: Some("coral".to_string()),
                            message: w.message.clone(),
                            ..Default::default()
                        })
                        .collect(),
                    Err(err) => {
                        let span = err.diagnostic.span;
                        let (line, col) = idx.line_col(span.start);
                        let (end_line, end_col) = idx.line_col(span.end.max(span.start + 1));
                        let line = line.saturating_sub(1) as u32;
                        let col = col.saturating_sub(1) as u32;
                        let end_line = end_line.saturating_sub(1) as u32;
                        let end_col = end_col.saturating_sub(1) as u32;
                        vec![Diagnostic {
                            range: Range::new(
                                Position::new(line, col),
                                Position::new(end_line, end_col),
                            ),
                            severity: Some(DiagnosticSeverity::ERROR),
                            source: Some("coral".to_string()),
                            message: err.diagnostic.message.clone(),
                            ..Default::default()
                        }]
                    }
                }
            }
            Err(err) => {
                vec![Diagnostic {
                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("coral".to_string()),
                    message: format!("Module loading error: {}", err),
                    ..Default::default()
                }]
            }
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for CoralLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                        ..Default::default()
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Coral LSP initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.update_document(&uri, params.text_document.text.clone());
        let diagnostics = if uri.scheme() == "file" {
            self.diagnose_file(&uri)
        } else {
            self.diagnose(&params.text_document.text)
        };
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.last() {
            // Only update the cached document for hover/symbols/goto-def.
            // Skip expensive full recompilation — diagnostics run on save.
            self.update_document(&uri, change.text.clone());
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(ref text) = params.text {
            self.update_document(&uri, text.clone());
        }
        let diagnostics = if uri.scheme() == "file" {
            self.diagnose_file(&uri)
        } else if let Some(text) = params.text {
            self.diagnose(&text)
        } else {
            return;
        };
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(uri) {
            Some(d) => d,
            None => return Ok(None),
        };

        let offset = lsp_offset(&doc.source, pos);

        // Find the word at the cursor position
        let word = word_at_offset(&doc.source, offset);
        if word.is_empty() {
            return Ok(None);
        }

        // Look up the symbol
        for sym in &doc.symbols {
            if sym.name == word {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```coral\n{}\n```", sym.signature),
                    }),
                    range: None,
                }));
            }
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(uri) {
            Some(d) => d,
            None => return Ok(None),
        };

        let offset = lsp_offset(&doc.source, pos);

        let word = word_at_offset(&doc.source, offset);
        if word.is_empty() {
            return Ok(None);
        }

        let idx = LineIndex::new(&doc.source);
        for sym in &doc.symbols {
            if sym.name == word {
                let (line, col) = idx.line_col(sym.span_start);
                let (end_line, end_col) = idx.line_col(sym.span_end);
                let location = Location {
                    uri: uri.clone(),
                    range: Range::new(
                        Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32),
                        Position::new(
                            end_line.saturating_sub(1) as u32,
                            end_col.saturating_sub(1) as u32,
                        ),
                    ),
                };
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            }
        }

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;

        let docs = self.documents.lock().unwrap();
        let doc = match docs.get(uri) {
            Some(d) => d,
            None => return Ok(None),
        };

        let idx = LineIndex::new(&doc.source);
        let symbols: Vec<SymbolInformation> = doc
            .symbols
            .iter()
            .map(|sym| {
                let (line, col) = idx.line_col(sym.span_start);
                let (end_line, end_col) = idx.line_col(sym.span_end);
                #[allow(deprecated)]
                SymbolInformation {
                    name: sym.name.clone(),
                    kind: sym.kind,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: Range::new(
                            Position::new(
                                line.saturating_sub(1) as u32,
                                col.saturating_sub(1) as u32,
                            ),
                            Position::new(
                                end_line.saturating_sub(1) as u32,
                                end_col.saturating_sub(1) as u32,
                            ),
                        ),
                    },
                    container_name: None,
                }
            })
            .collect();

        Ok(Some(DocumentSymbolResponse::Flat(symbols)))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(CoralLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
