use crate::span::{LineIndex, Span};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
    Suggestion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningCategory {
    UnusedVariable,
    DeadCode,
    ShadowedBinding,
    TypeMismatchBranch,
    UnreachableCode,
    Nullability,
    General,
}

impl WarningCategory {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "unused_variable" | "unused" => Some(Self::UnusedVariable),
            "dead_code" => Some(Self::DeadCode),
            "shadowed_binding" | "shadow" => Some(Self::ShadowedBinding),
            "type_mismatch_branch" | "branch_types" => Some(Self::TypeMismatchBranch),
            "unreachable_code" | "unreachable" => Some(Self::UnreachableCode),
            "nullability" | "nullable" => Some(Self::Nullability),
            "general" => Some(Self::General),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::UnusedVariable => "unused_variable",
            Self::DeadCode => "dead_code",
            Self::ShadowedBinding => "shadowed_binding",
            Self::TypeMismatchBranch => "type_mismatch_branch",
            Self::UnreachableCode => "unreachable_code",
            Self::Nullability => "nullability",
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

    pub category: Option<WarningCategory>,

    pub related: Vec<Diagnostic>,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Error,
            category: None,
            related: Vec::new(),
        }
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            related: Vec::new(),
            severity: Severity::Warning,
            category: None,
        }
    }

    pub fn categorized_warning(
        message: impl Into<String>,
        span: Span,
        category: WarningCategory,
    ) -> Self {
        Self {
            message: message.into(),
            span,
            help: None,
            severity: Severity::Warning,
            category: Some(category),
            related: Vec::new(),
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

    pub fn type_mismatch(expected: &str, found: &str, span: Span) -> Self {
        Self::new(
            format!("type mismatch: expected `{}`, found `{}`", expected, found),
            span,
        )
        .with_help(format!(
            "consider converting the value or checking if the types should match\n\
             expected: {}\n\
             found:    {}",
            expected, found
        ))
    }

    pub fn undefined_variable(name: &str, span: Span, suggestions: Vec<String>) -> Self {
        let mut diag = Self::new(format!("undefined variable `{}`", name), span);

        if !suggestions.is_empty() {
            let suggestion_list = suggestions
                .iter()
                .take(3)
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

    source: Option<String>,
}

impl CompileError {
    pub fn new(stage: Stage, diagnostic: Diagnostic) -> Self {
        Self {
            stage,
            diagnostic,
            source: None,
        }
    }

    pub fn with_source(stage: Stage, diagnostic: Diagnostic, source: &str) -> Self {
        Self {
            stage,
            diagnostic,
            source: Some(source.to_owned()),
        }
    }

    pub fn with_context(stage: Stage, diagnostic: Diagnostic, source: &str) -> Self {
        let enhanced = Self::add_source_context(diagnostic, source);
        Self {
            stage,
            diagnostic: enhanced,
            source: Some(source.to_owned()),
        }
    }

    fn add_source_context(mut diagnostic: Diagnostic, source: &str) -> Diagnostic {
        let idx = LineIndex::new(source);
        let span = diagnostic.span;
        let (line_num, col_num) = idx.line_col(span.start);
        let line_content = idx.line_text(source, span.start);
        let underline_len = if span.end > span.start {
            span.end - span.start
        } else {
            1
        };

        let context_help = format!(
            "at line {}, column {}\n{}\n{}{}",
            line_num,
            col_num,
            line_content,
            " ".repeat(col_num.saturating_sub(1)),
            "^".repeat(
                underline_len.min(
                    line_content
                        .len()
                        .saturating_sub(col_num.saturating_sub(1))
                        .max(1)
                )
            )
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
                stage_name, self.diagnostic.span, self.diagnostic.message
            )?;
        }
        if let Some(help) = &self.diagnostic.help {
            write!(f, "\nhelp: {}", help)?;
        }

        for related in &self.diagnostic.related {
            if let Some(src) = &self.source {
                let idx = LineIndex::new(src);
                write!(
                    f,
                    "\n{} error at {}: {}",
                    stage_name,
                    idx.fmt_span(related.span),
                    related.message
                )?;
            } else {
                write!(
                    f,
                    "\n{} error at {}: {}",
                    stage_name, related.span, related.message
                )?;
            }
            if let Some(help) = &related.help {
                write!(f, "\nhelp: {}", help)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}
