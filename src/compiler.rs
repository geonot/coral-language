use crate::ast::{BinaryOp, Expression, UnaryOp};
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
        
        // Fold constant expressions (1 + 2 → 3, true and false → false, etc.)
        Self::fold_expressions(&mut model);

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
    
    /// Fold constant expressions in the semantic model.
    /// This includes arithmetic on literals (e.g., 1 + 2 → 3) and boolean operations.
    fn fold_expressions(model: &mut semantic::SemanticModel) {
        // Fold globals
        for binding in &mut model.globals {
            binding.value = Self::fold_expr(binding.value.clone());
        }
        // Fold function bodies
        for func in &mut model.functions {
            Self::fold_block(&mut func.body);
        }
    }
    
    fn fold_block(block: &mut crate::ast::Block) {
        for stmt in &mut block.statements {
            match stmt {
                crate::ast::Statement::Binding(binding) => {
                    binding.value = Self::fold_expr(binding.value.clone());
                }
                crate::ast::Statement::Expression(expr) => {
                    *expr = Self::fold_expr(expr.clone());
                }
                crate::ast::Statement::Return(expr, _) => {
                    *expr = Self::fold_expr(expr.clone());
                }
            }
        }
        if let Some(value) = &mut block.value {
            *value = Box::new(Self::fold_expr(*value.clone()));
        }
    }
    
    /// Recursively fold constant expressions.
    fn fold_expr(expr: Expression) -> Expression {
        match expr {
            Expression::Binary { op, left, right, span } => {
                let left = Box::new(Self::fold_expr(*left));
                let right = Box::new(Self::fold_expr(*right));
                
                // Try to fold numeric operations
                match (left.as_ref(), right.as_ref()) {
                    (Expression::Integer(a, _), Expression::Integer(b, _)) => {
                        match op {
                            BinaryOp::Add => return Expression::Integer(a + b, span),
                            BinaryOp::Sub => return Expression::Integer(a - b, span),
                            BinaryOp::Mul => return Expression::Integer(a * b, span),
                            BinaryOp::Div if *b != 0 => return Expression::Integer(a / b, span),
                            BinaryOp::Mod if *b != 0 => return Expression::Integer(a % b, span),
                            BinaryOp::BitAnd => return Expression::Integer(a & b, span),
                            BinaryOp::BitOr => return Expression::Integer(a | b, span),
                            BinaryOp::BitXor => return Expression::Integer(a ^ b, span),
                            BinaryOp::Equals => return Expression::Bool(a == b, span),
                            BinaryOp::Less => return Expression::Bool(a < b, span),
                            BinaryOp::LessEq => return Expression::Bool(a <= b, span),
                            BinaryOp::Greater => return Expression::Bool(a > b, span),
                            BinaryOp::GreaterEq => return Expression::Bool(a >= b, span),
                            _ => {}
                        }
                    }
                    (Expression::Float(a, _), Expression::Float(b, _)) => {
                        match op {
                            BinaryOp::Add => return Expression::Float(a + b, span),
                            BinaryOp::Sub => return Expression::Float(a - b, span),
                            BinaryOp::Mul => return Expression::Float(a * b, span),
                            BinaryOp::Div => return Expression::Float(a / b, span),
                            BinaryOp::Less => return Expression::Bool(a < b, span),
                            BinaryOp::LessEq => return Expression::Bool(a <= b, span),
                            BinaryOp::Greater => return Expression::Bool(a > b, span),
                            BinaryOp::GreaterEq => return Expression::Bool(a >= b, span),
                            _ => {}
                        }
                    }
                    (Expression::Bool(a, _), Expression::Bool(b, _)) => {
                        match op {
                            BinaryOp::And => return Expression::Bool(*a && *b, span),
                            BinaryOp::Or => return Expression::Bool(*a || *b, span),
                            BinaryOp::Equals => return Expression::Bool(a == b, span),
                            _ => {}
                        }
                    }
                    (Expression::String(a, _), Expression::String(b, _)) => {
                        if op == BinaryOp::Add {
                            // String concatenation at compile time
                            let mut result = a.clone();
                            result.push_str(b);
                            return Expression::String(result, span);
                        }
                    }
                    _ => {}
                }
                
                Expression::Binary { op, left, right, span }
            }
            Expression::Unary { op, expr, span } => {
                let inner = Box::new(Self::fold_expr(*expr));
                
                match (op, inner.as_ref()) {
                    (UnaryOp::Neg, Expression::Integer(n, _)) => {
                        return Expression::Integer(-n, span);
                    }
                    (UnaryOp::Neg, Expression::Float(n, _)) => {
                        return Expression::Float(-n, span);
                    }
                    (UnaryOp::Not, Expression::Bool(b, _)) => {
                        return Expression::Bool(!b, span);
                    }
                    (UnaryOp::BitNot, Expression::Integer(n, _)) => {
                        return Expression::Integer(!n, span);
                    }
                    _ => {}
                }
                
                Expression::Unary { op, expr: inner, span }
            }
            Expression::Ternary { condition, then_branch, else_branch, span } => {
                let cond = Box::new(Self::fold_expr(*condition));
                let then_b = Box::new(Self::fold_expr(*then_branch));
                let else_b = Box::new(Self::fold_expr(*else_branch));
                
                // If condition is a constant bool, return the appropriate branch
                if let Expression::Bool(b, _) = cond.as_ref() {
                    return if *b { *then_b } else { *else_b };
                }
                
                Expression::Ternary { condition: cond, then_branch: then_b, else_branch: else_b, span }
            }
            Expression::Call { callee, args, span } => {
                let callee = Box::new(Self::fold_expr(*callee));
                let args: Vec<_> = args.into_iter().map(Self::fold_expr).collect();
                Expression::Call { callee, args, span }
            }
            Expression::List(items, span) => {
                let items: Vec<_> = items.into_iter().map(Self::fold_expr).collect();
                Expression::List(items, span)
            }
            Expression::Map(entries, span) => {
                let entries: Vec<_> = entries.into_iter()
                    .map(|(k, v)| (Self::fold_expr(k), Self::fold_expr(v)))
                    .collect();
                Expression::Map(entries, span)
            }
            Expression::Member { target, property, span } => {
                let target = Box::new(Self::fold_expr(*target));
                Expression::Member { target, property, span }
            }
            Expression::Lambda { params, mut body, span } => {
                Self::fold_block(&mut body);
                Expression::Lambda { params, body, span }
            }
            // Pass through literals and other expressions unchanged
            other => other,
        }
    }
}
