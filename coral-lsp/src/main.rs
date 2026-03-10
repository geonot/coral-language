use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use coralc::module_loader::ModuleLoader;
use coralc::span::LineIndex;
use coralc::Compiler;

use std::sync::Mutex;

struct CoralLanguageServer {
    client: Client,
    compiler: Compiler,
    loader: Mutex<ModuleLoader>,
}

impl CoralLanguageServer {
    fn new(client: Client) -> Self {
        Self {
            client,
            compiler: Compiler,
            loader: Mutex::new(ModuleLoader::with_default_std()),
        }
    }

    /// Compile source text and return diagnostics.
    fn diagnose(&self, source: &str) -> Vec<Diagnostic> {
        let idx = LineIndex::new(source);
        match self.compiler.compile_to_ir_with_warnings(source) {
            Ok((_ir, warnings)) => {
                warnings
                    .iter()
                    .map(|w| Diagnostic {
                        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("coral".to_string()),
                        message: w.message.clone(),
                        ..Default::default()
                    })
                    .collect()
            }
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

    /// Compile using the module-aware path for files on disk.
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
                    Ok((_ir, warnings)) => {
                        warnings
                            .iter()
                            .map(|w| Diagnostic {
                                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                                severity: Some(DiagnosticSeverity::WARNING),
                                source: Some("coral".to_string()),
                                message: w.message.clone(),
                                ..Default::default()
                            })
                            .collect()
                    }
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
            let diagnostics = self.diagnose(&change.text);
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.clone();
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
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(CoralLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
