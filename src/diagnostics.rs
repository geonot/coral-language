use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
    pub severity: Severity,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Error,
        }
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Warning,
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

    /// Create a type mismatch error with helpful suggestions.
    pub fn type_mismatch(expected: &str, found: &str, span: Span) -> Self {
        Self::new(
            format!("type mismatch: expected `{}`, found `{}`", expected, found),
            span,
        ).with_help(format!(
            "consider converting the value or checking if the types should match\n\
             expected: {}\n\
             found:    {}",
            expected, found
        ))
    }

    /// Create an undefined variable error with suggestions.
    pub fn undefined_variable(name: &str, span: Span, suggestions: Vec<String>) -> Self {
        let mut diag = Self::new(
            format!("undefined variable `{}`", name),
            span,
        );
        
        if !suggestions.is_empty() {
            let suggestion_list = suggestions
                .iter()
                .take(3)  // Show max 3 suggestions
                .map(|s| format!("`{}`", s))
                .collect::<Vec<_>>()
                .join(", ");
            diag = diag.with_help(format!("did you mean: {}", suggestion_list));
        }
        
        diag
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

    /// Create a more descriptive error based on the stage and diagnostic.
    pub fn with_context(stage: Stage, diagnostic: Diagnostic, source: &str) -> Self {
        let enhanced = Self::add_source_context(diagnostic, source);
        Self::new(stage, enhanced)
    }

    /// Add source code context to a diagnostic for better error messages.
    fn add_source_context(mut diagnostic: Diagnostic, source: &str) -> Diagnostic {
        let span = diagnostic.span;
        
        // Find the line number and column
        let lines: Vec<&str> = source.lines().collect();
        let mut line_num = 1;
        let mut char_count = 0;
        let mut line_start = 0;
        
        for (i, line) in lines.iter().enumerate() {
            let line_end = char_count + line.len() + 1; // +1 for newline
            if char_count <= span.start && span.start < line_end {
                line_num = i + 1;
                line_start = char_count;
                break;
            }
            char_count = line_end;
        }
        
        let col_num = span.start - line_start + 1;
        let line_content = lines.get(line_num - 1).unwrap_or(&"");
        
        // Enhance help message with source context
        let existing_help = diagnostic.help.unwrap_or_default();
        let context_help = format!(
            "at line {}, column {}\n{}\n{}{}",
            line_num,
            col_num,
            line_content,
            " ".repeat((span.start - line_start).max(0)),
            "^".repeat((span.end - span.start).max(1))
        );
        
        diagnostic.help = Some(if existing_help.is_empty() {
            context_help
        } else {
            format!("{}\n{}", existing_help, context_help)
        });
        
        diagnostic
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
