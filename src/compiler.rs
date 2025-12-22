use crate::ast::Expression;
use crate::codegen::{CodeGenerator, InlineAsmMode};
use crate::diagnostics::{CompileError, Stage};
use crate::lexer;
use crate::lower;
use crate::mir_const::ConstEvaluator;
use crate::mir_lower;
use crate::parser::Parser;
use crate::semantic;
use crate::span::Span;
use inkwell::context::Context;

pub struct Compiler;

impl Compiler {
    pub fn compile_to_ir(&self, source: &str) -> Result<String, CompileError> {
        let tokens = lexer::lex(source).map_err(|diag| CompileError::new(Stage::Lex, diag))?;
        let parser = Parser::new(tokens, source.len());
        let program = parser
            .parse()
            .map_err(|diag| CompileError::new(Stage::Parse, diag))?;
        let program = lower::lower(program)
            .map_err(|diag| CompileError::new(Stage::Parse, diag))?;
        let mut model = semantic::analyze(program)
            .map_err(|diag| CompileError::new(Stage::Semantic, diag))?;
        self.maybe_emit_alloc_report(&model);
        let mir_module = mir_lower::lower_semantic_model(&model);
        let const_eval = ConstEvaluator::new(mir_module);
        self.fold_const_globals(&mut model, &const_eval);

        let context = Context::create();
        let inline_mode = match std::env::var("CORAL_INLINE_ASM") {
            Ok(val) if val.eq_ignore_ascii_case("allow-noop") => InlineAsmMode::Noop,
            Ok(val) if val.eq_ignore_ascii_case("emit") => InlineAsmMode::Emit,
            _ => InlineAsmMode::Deny,
        };
        let generator = CodeGenerator::new(&context, "coral_module")
            .with_inline_asm_mode(inline_mode);
        let module = generator
            .compile(&model)
            .map_err(|diag| CompileError::new(Stage::Codegen, diag))?;
        Ok(module.print_to_string().to_string())
    }

    fn fold_const_globals(
        &self,
        model: &mut semantic::SemanticModel,
        evaluator: &ConstEvaluator,
    ) {
        use std::collections::HashSet;

        let zero_arity: HashSet<String> = model
            .functions
            .iter()
            .filter(|func| func.params.is_empty())
            .map(|func| func.name.clone())
            .collect();

        for binding in &mut model.globals {
            let replacement = match &binding.value {
                Expression::Call { callee, args, span } if args.is_empty() => {
                    if let Expression::Identifier(name, _) = callee.as_ref() {
                        if zero_arity.contains(name) {
                            evaluator
                                .eval_zero_arity(name)
                                .map(|value| Self::const_value_to_expression(&value, span.join(binding.span)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(expr) = replacement {
                binding.value = expr;
            }
        }
    }

    fn maybe_emit_alloc_report(&self, model: &semantic::SemanticModel) {
        if let Ok(path) = std::env::var("CORAL_ALLOC_REPORT") {
            if path.is_empty() {
                return;
            }
            let mut out = String::new();
            out.push_str("# Allocation and mutability report\n");
            out.push_str("symbol,mutability,alloc_strategy,reads,mutations,escapes,calls\n");
            for (name, m) in model.mutability.symbols.iter() {
                let alloc = model
                    .allocation
                    .symbols
                    .get(name)
                    .copied()
                    .unwrap_or(crate::types::AllocationStrategy::Unknown);
                let usage = model.usage.symbols.get(name).cloned().unwrap_or_default();
                out.push_str(&format!(
                    "{},{:?},{:?},{},{},{},{}\n",
                    name,
                    m,
                    alloc,
                    usage.reads,
                    usage.mutations,
                    usage.escapes,
                    usage.calls,
                ));
            }
            let _ = std::fs::write(path, out);
        }
    }

    fn const_value_to_expression(value: &crate::mir_interpreter::Value, span: Span) -> Expression {
        match value {
            crate::mir_interpreter::Value::Number(n) => Expression::Float(*n, span),
            crate::mir_interpreter::Value::Bool(b) => Expression::Bool(*b, span),
            crate::mir_interpreter::Value::String(s) => Expression::String(s.clone(), span),
            crate::mir_interpreter::Value::Bytes(bytes) => Expression::Bytes(bytes.clone(), span),
            crate::mir_interpreter::Value::Unit => Expression::Unit,
        }
    }
}
