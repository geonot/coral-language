use crate::span::{LineIndex, Span};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// CC2.4: Warning categories for filtering via --allow/--warn CLI flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningCategory {
    UnusedVariable,
    DeadCode,
    ShadowedBinding,
    TypeMismatchBranch,
    UnreachableCode,
    General,
}

impl WarningCategory {
    /// Parse a category name from a CLI string (e.g., "dead_code").
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "unused_variable" | "unused" => Some(Self::UnusedVariable),
            "dead_code" => Some(Self::DeadCode),
            "shadowed_binding" | "shadow" => Some(Self::ShadowedBinding),
            "type_mismatch_branch" | "branch_types" => Some(Self::TypeMismatchBranch),
            "unreachable_code" | "unreachable" => Some(Self::UnreachableCode),
            "general" => Some(Self::General),
            _ => None,
        }
    }

    /// Return the canonical name of this category.
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnusedVariable => "unused_variable",
            Self::DeadCode => "dead_code",
            Self::ShadowedBinding => "shadowed_binding",
            Self::TypeMismatchBranch => "type_mismatch_branch",
            Self::UnreachableCode => "unreachable_code",
            Self::General => "general",
        }
    }
}

impl fmt::Display for WarningCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
    pub severity: Severity,
    /// CC2.4: Optional warning category for filtering.
    pub category: Option<WarningCategory>,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Error,
            category: None,
        }
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Warning,
            category: None,
        }
    }

    /// CC2.4: Create a categorized warning.
    pub fn categorized_warning(message: impl Into<String>, span: Span, category: WarningCategory) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Warning,
            category: Some(category),
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
    /// Optional source text for pretty-printing (CC2.1).
    source: Option<String>,
}

impl CompileError {
    pub fn new(stage: Stage, diagnostic: Diagnostic) -> Self {
        Self { stage, diagnostic, source: None }
    }

    /// Create an error that carries the original source for pretty-printing (CC2.1).
    pub fn with_source(stage: Stage, diagnostic: Diagnostic, source: &str) -> Self {
        Self {
            stage,
            diagnostic,
            source: Some(source.to_owned()),
        }
    }

    /// Create a more descriptive error based on the stage and diagnostic.
    pub fn with_context(stage: Stage, diagnostic: Diagnostic, source: &str) -> Self {
        let enhanced = Self::add_source_context(diagnostic, source);
        Self { stage, diagnostic: enhanced, source: Some(source.to_owned()) }
    }

    /// Add source code context to a diagnostic for better error messages.
    fn add_source_context(mut diagnostic: Diagnostic, source: &str) -> Diagnostic {
        let idx = LineIndex::new(source);
        let span = diagnostic.span;
        let (line_num, col_num) = idx.line_col(span.start);
        let line_content = idx.line_text(source, span.start);
        let underline_len = if span.end > span.start { span.end - span.start } else { 1 };

        let context_help = format!(
            "at line {}, column {}\n{}\n{}{}",
            line_num,
            col_num,
            line_content,
            " ".repeat(col_num.saturating_sub(1)),
            "^".repeat(underline_len.min(line_content.len().saturating_sub(col_num.saturating_sub(1)).max(1)))
        );

        let existing_help = diagnostic.help.unwrap_or_default();
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
        let stage_name = match self.stage {
            Stage::Lex => "Lexical",
            Stage::Parse => "Parse",
            Stage::Semantic => "Semantic",
            Stage::Codegen => "Codegen",
        };

        // CC2.1: If we have source, show line:col instead of raw byte offsets.
        if let Some(src) = &self.source {
            let idx = LineIndex::new(src);
            write!(
                f,
                "{} error at {}: {}",
                stage_name,
                idx.fmt_span(self.diagnostic.span),
                self.diagnostic.message
            )?;
        } else {
            write!(
                f,
                "{} error at {}: {}",
                stage_name,
                self.diagnostic.span,
                self.diagnostic.message
            )?;
        }
        if let Some(help) = &self.diagnostic.help {
            write!(f, "\nhelp: {}", help)?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}
