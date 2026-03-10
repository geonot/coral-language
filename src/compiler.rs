use crate::ast::{BinaryOp, Expression, UnaryOp};
use crate::codegen::{CodeGenerator, InlineAsmMode};
use crate::diagnostics::{CompileError, Stage};
use crate::lexer;
use crate::lower;
use crate::parser::Parser;
use crate::semantic;
use inkwell::context::Context;

/// Purity classification for functions (C1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purity {
    /// No side effects — eligible for comptime evaluation and reordering.
    Pure,
    /// Reads external state but doesn't mutate it.
    ReadOnly,
    /// Has side effects (I/O, mutation, actor messaging).
    Effectful,
}

/// Known pure builtin functions and their constant-evaluation rules.
fn is_pure_builtin(name: &str) -> Option<Purity> {
    match name {
        // Math builtins (pure)
        "sqrt" | "abs" | "floor" | "ceil" | "round" | "sin" | "cos" | "tan"
        | "asin" | "acos" | "atan" | "atan2" | "exp" | "ln" | "log2" | "log10"
        | "pow" | "min" | "max" | "clamp" => Some(Purity::Pure),
        // Type checks (pure)
        "is_number" | "is_string" | "is_bool" | "is_list" | "is_map" | "is_none"
        | "is_err" | "is_some" => Some(Purity::Pure),
        // String operations (pure)
        "length" | "to_string" | "number_to_string" | "char_at" | "char_code"
        | "from_char_code" => Some(Purity::Pure),
        // I/O and mutation (effectful)
        "log" | "print" | "println" | "read_file" | "write_file" | "append_file"
        | "exit" | "push" | "pop" | "set" => Some(Purity::Effectful),
        _ => None,
    }
}

/// Evaluate a pure math builtin on a constant f64 argument at compile time (C1.1).
fn eval_math_const(name: &str, arg: f64) -> Option<f64> {
    match name {
        "sqrt" if arg >= 0.0 => Some(arg.sqrt()),
        "abs" => Some(arg.abs()),
        "floor" => Some(arg.floor()),
        "ceil" => Some(arg.ceil()),
        "round" => Some(arg.round()),
        "sin" => Some(arg.sin()),
        "cos" => Some(arg.cos()),
        "tan" => Some(arg.tan()),
        "asin" if (-1.0..=1.0).contains(&arg) => Some(arg.asin()),
        "acos" if (-1.0..=1.0).contains(&arg) => Some(arg.acos()),
        "atan" => Some(arg.atan()),
        "exp" => Some(arg.exp()),
        "ln" if arg > 0.0 => Some(arg.ln()),
        "log2" if arg > 0.0 => Some(arg.log2()),
        "log10" if arg > 0.0 => Some(arg.log10()),
        _ => None,
    }
}

/// Evaluate a pure 2-arg math builtin at compile time.
fn eval_math_const2(name: &str, a: f64, b: f64) -> Option<f64> {
    match name {
        "pow" => Some(a.powf(b)),
        "min" => Some(a.min(b)),
        "max" => Some(a.max(b)),
        "atan2" => Some(a.atan2(b)),
        _ => None,
    }
}

pub struct Compiler;

impl Compiler {
    pub fn compile_to_ir(&self, source: &str) -> Result<String, CompileError> {
        let (ir, _warnings) = self.compile_to_ir_with_warnings(source)?;
        Ok(ir)
    }

    /// Compile source to LLVM IR and return any warnings collected during analysis.
    pub fn compile_to_ir_with_warnings(&self, source: &str) -> Result<(String, Vec<String>), CompileError> {
        let tokens = lexer::lex(source).map_err(|diag| CompileError::with_source(Stage::Lex, diag, source))?;
        let parser = Parser::new(tokens, source.len());
        let program = parser
            .parse()
            .map_err(|diag| CompileError::with_source(Stage::Parse, diag, source))?;
        let program = lower::lower(program)
            .map_err(|diag| CompileError::with_source(Stage::Parse, diag, source))?;
        let mut model = semantic::analyze(program)
            .map_err(|diag| CompileError::with_source(Stage::Semantic, diag, source))?;
        self.maybe_emit_alloc_report(&model);
        
        // Collect warnings before folding
        let warnings: Vec<String> = model.warnings.iter().map(|w| w.message.clone()).collect();
        
        // Fold constant expressions (1 + 2 → 3, true and false → false, etc.)
        Self::fold_expressions(&mut model);

        let context = Context::create();
        let inline_mode = match std::env::var("CORAL_INLINE_ASM") {
            Ok(val) if val.eq_ignore_ascii_case("allow-noop") => InlineAsmMode::Noop,
            Ok(val) if val.eq_ignore_ascii_case("emit") => InlineAsmMode::Emit,
            _ => InlineAsmMode::Deny,
        };
        let mut generator = CodeGenerator::new(&context, "coral_module")
            .with_inline_asm_mode(inline_mode);
        // CC2.3: Enable DWARF debug info when CORAL_DEBUG_INFO is set.
        if std::env::var("CORAL_DEBUG_INFO").map_or(false, |v| !v.is_empty()) {
            generator = generator.with_debug_info("coral_module.coral", source);
        }
        let module = generator
            .compile(&model)
            .map_err(|diag| CompileError::with_source(Stage::Codegen, diag, source))?;
        Ok((module.print_to_string().to_string(), warnings))
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
        // First fold all expressions within statements.
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
                crate::ast::Statement::If { condition, body, elif_branches, else_body, .. } => {
                    *condition = Self::fold_expr(condition.clone());
                    Self::fold_block(body);
                    for (cond, blk) in elif_branches.iter_mut() {
                        *cond = Self::fold_expr(cond.clone());
                        Self::fold_block(blk);
                    }
                    if let Some(else_blk) = else_body {
                        Self::fold_block(else_blk);
                    }
                }
                crate::ast::Statement::While { condition, body, .. } => {
                    *condition = Self::fold_expr(condition.clone());
                    Self::fold_block(body);
                }
                crate::ast::Statement::For { iterable, body, .. } => {
                    *iterable = Self::fold_expr(iterable.clone());
                    Self::fold_block(body);
                }
                crate::ast::Statement::ForKV { iterable, body, .. } => {
                    *iterable = Self::fold_expr(iterable.clone());
                    Self::fold_block(body);
                }
                crate::ast::Statement::ForRange { start, end, step, body, .. } => {
                    *start = Self::fold_expr(start.clone());
                    *end = Self::fold_expr(end.clone());
                    if let Some(s) = step {
                        *s = Self::fold_expr(s.clone());
                    }
                    Self::fold_block(body);
                }
                crate::ast::Statement::Break(_) | crate::ast::Statement::Continue(_) => {}
                crate::ast::Statement::FieldAssign { value, .. } => {
                    *value = Self::fold_expr(value.clone());
                }
                crate::ast::Statement::PatternBinding { value, .. } => {
                    *value = Self::fold_expr(value.clone());
                }
            }
        }
        
        // C1.5: Dead expression elimination — remove pure expression statements
        // whose results are unused (literals, identifiers, operations with no side effects).
        block.statements.retain(|stmt| {
            if let crate::ast::Statement::Expression(expr) = stmt {
                !is_pure_dead_expression(expr)
            } else {
                true
            }
        });
        
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
            Expression::Call { callee, args, arg_names, span, .. } => {
                let callee = Box::new(Self::fold_expr(*callee));
                let args: Vec<_> = args.into_iter().map(Self::fold_expr).collect();
                
                // C1.1: Fold pure builtin math calls on constant arguments.
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    // Single-arg pure math: sqrt(4.0) → 2.0
                    if args.len() == 1 {
                        let const_val = match &args[0] {
                            Expression::Float(f, _) => Some(*f),
                            Expression::Integer(i, _) => Some(*i as f64),
                            _ => None,
                        };
                        if let Some(val) = const_val {
                            if let Some(result) = eval_math_const(name, val) {
                                // Return integer if result is whole, otherwise float
                                if result == (result as i64) as f64 && result.abs() < i64::MAX as f64 {
                                    return Expression::Integer(result as i64, span);
                                }
                                return Expression::Float(result, span);
                            }
                        }
                    }
                    // Two-arg pure math: pow(2.0, 10.0) → 1024.0, min(3, 7) → 3
                    if args.len() == 2 {
                        let a_val = match &args[0] {
                            Expression::Float(f, _) => Some(*f),
                            Expression::Integer(i, _) => Some(*i as f64),
                            _ => None,
                        };
                        let b_val = match &args[1] {
                            Expression::Float(f, _) => Some(*f),
                            Expression::Integer(i, _) => Some(*i as f64),
                            _ => None,
                        };
                        if let (Some(a), Some(b)) = (a_val, b_val) {
                            if let Some(result) = eval_math_const2(name.as_str(), a, b) {
                                if result == (result as i64) as f64 && result.abs() < i64::MAX as f64 {
                                    return Expression::Integer(result as i64, span);
                                }
                                return Expression::Float(result, span);
                            }
                        }
                    }
                    // C1.1: length() on string literal → fold to integer
                    if name == "length" && args.len() == 1 {
                        if let Expression::String(ref s, _) = args[0] {
                            return Expression::Integer(s.len() as i64, span);
                        }
                        // length() on list literal → number of items
                        if let Expression::List(ref items, _) = args[0] {
                            return Expression::Integer(items.len() as i64, span);
                        }
                    }
                }
                
                Expression::Call { callee, args, arg_names, span }
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
                // C1.1: Fold .length() on constant strings and list literals.
                if property == "length" {
                    match target.as_ref() {
                        Expression::String(s, _) => {
                            return Expression::Integer(s.len() as i64, span);
                        }
                        Expression::List(items, _) => {
                            return Expression::Integer(items.len() as i64, span);
                        }
                        _ => {}
                    }
                }
                Expression::Member { target, property, span }
            }
            Expression::Index { target, index, span } => {
                let target = Box::new(Self::fold_expr(*target));
                let index = Box::new(Self::fold_expr(*index));
                Expression::Index { target, index, span }
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

/// C1.5: Check if an expression is pure (no side effects) and thus safe to eliminate
/// when its result is unused in statement position.
fn is_pure_dead_expression(expr: &Expression) -> bool {
    match expr {
        // Literals are always pure
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::String(_, _)
        | Expression::Unit => true,
        // Identifiers (just reading a variable) are pure
        Expression::Identifier(_, _) => true,
        // Arithmetic/logic on pure exprs is pure
        Expression::Binary { left, right, .. } => {
            is_pure_dead_expression(left) && is_pure_dead_expression(right)
        }
        Expression::Unary { expr, .. } => is_pure_dead_expression(expr),
        // List/Map literals with all-pure elements are pure
        Expression::List(items, _) => items.iter().all(is_pure_dead_expression),
        Expression::Map(entries, _) => entries.iter().all(|(k, v)| {
            is_pure_dead_expression(k) && is_pure_dead_expression(v)
        }),
        // Member access on a pure target is pure (reading a field)
        Expression::Member { target, .. } => is_pure_dead_expression(target),
        // Index access on pure target/index is pure
        Expression::Index { target, index, .. } => {
            is_pure_dead_expression(target) && is_pure_dead_expression(index)
        }
        // Calls are NOT pure by default (might have side effects)
        // Exception: known pure builtins with pure args
        Expression::Call { callee, args, .. } => {
            if let Expression::Identifier(name, _) = callee.as_ref() {
                if let Some(Purity::Pure) = is_pure_builtin(name) {
                    return args.iter().all(is_pure_dead_expression);
                }
            }
            false
        }
        _ => false,
    }
}
