use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn shift(mut self, offset: usize) -> Self {
        self.span = self.span.shift(offset);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Lex,
    Parse,
    Semantic,
    Codegen,
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub stage: Stage,
    pub diagnostic: Diagnostic,
}

impl CompileError {
    pub fn new(stage: Stage, diagnostic: Diagnostic) -> Self {
        Self { stage, diagnostic }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} error at {}: {}",
            match self.stage {
                Stage::Lex => "Lexical",
                Stage::Parse => "Parse",
                Stage::Semantic => "Semantic",
                Stage::Codegen => "Codegen",
            },
            self.diagnostic.span,
            self.diagnostic.message
        )?;
        if let Some(help) = &self.diagnostic.help {
            write!(f, "\nhelp: {}", help)?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}
