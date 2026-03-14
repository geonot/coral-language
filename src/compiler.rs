use crate::ast::{BinaryOp, Block, Expression, Function, Module, Program, Statement, UnaryOp};
use crate::codegen::{CodeGenerator, InlineAsmMode};
use crate::diagnostics::{CompileError, Stage};
use crate::lexer;
use crate::lower;
use crate::module_loader::ModuleSource;
use crate::parser::Parser;
use crate::semantic;
use inkwell::context::Context;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ModuleCache {
    cache_dir: PathBuf,
}

impl ModuleCache {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            cache_dir: base_dir.join(".coral-cache"),
        }
    }

    pub fn fingerprint(sources: &[ModuleSource]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for ms in sources {
            ms.name.hash(&mut hasher);
            ms.source.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn get(&self, fingerprint: u64) -> Option<String> {
        let ir_path = self.cache_dir.join(format!("{:016x}.ll", fingerprint));
        std::fs::read_to_string(ir_path).ok()
    }

    pub fn put(&self, fingerprint: u64, ir: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let ir_path = self.cache_dir.join(format!("{:016x}.ll", fingerprint));
        let mut f = std::fs::File::create(ir_path)?;
        f.write_all(ir.as_bytes())?;
        Ok(())
    }

    pub fn invalidate_all(&self) -> std::io::Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purity {
    Pure,

    ReadOnly,

    Effectful,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LtoOptLevel {
    O1,
    O2,
    O3,
}

impl LtoOptLevel {
    pub fn pipeline_string(self) -> &'static str {
        match self {
            LtoOptLevel::O1 => "default<O1>",
            LtoOptLevel::O2 => "default<O2>",
            LtoOptLevel::O3 => "default<O3>",
        }
    }
}

pub fn optimize_module(ir: &str, opt_level: LtoOptLevel) -> Result<String, String> {
    use inkwell::OptimizationLevel;
    use inkwell::passes::PassBuilderOptions;
    use inkwell::targets::{InitializationConfig, Target, TargetMachine};

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("failed to initialize native target: {}", e))?;

    let context = Context::create();
    let memory_buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
        ir.as_bytes(),
        "input_ir",
    );
    let module = context
        .create_module_from_ir(memory_buffer)
        .map_err(|e| format!("failed to parse IR: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|e| format!("failed to create target from triple: {}", e))?;
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Aggressive,
            inkwell::targets::RelocMode::Default,
            inkwell::targets::CodeModel::Default,
        )
        .ok_or_else(|| "failed to create target machine".to_string())?;

    let pass_options = PassBuilderOptions::create();
    pass_options.set_verify_each(false);
    pass_options.set_loop_vectorization(true);
    pass_options.set_loop_unrolling(true);
    pass_options.set_merge_functions(true);

    module
        .run_passes(opt_level.pipeline_string(), &machine, pass_options)
        .map_err(|e| format!("LLVM pass pipeline failed: {}", e))?;

    Ok(module.print_to_string().to_string())
}

pub fn instrument_for_pgo(ir: &str) -> Result<String, String> {
    use inkwell::OptimizationLevel;
    use inkwell::passes::PassBuilderOptions;
    use inkwell::targets::{InitializationConfig, Target, TargetMachine};

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("failed to initialize native target: {}", e))?;

    let context = Context::create();
    let memory_buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
        ir.as_bytes(),
        "input_ir",
    );
    let module = context
        .create_module_from_ir(memory_buffer)
        .map_err(|e| format!("failed to parse IR: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|e| format!("failed to create target from triple: {}", e))?;
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            inkwell::targets::RelocMode::Default,
            inkwell::targets::CodeModel::Default,
        )
        .ok_or_else(|| "failed to create target machine".to_string())?;

    let pass_options = PassBuilderOptions::create();
    pass_options.set_verify_each(false);

    module
        .run_passes("pgo-instr-gen,instrprof", &machine, pass_options)
        .map_err(|e| format!("PGO instrumentation pass failed: {}", e))?;

    Ok(module.print_to_string().to_string())
}

pub fn optimize_with_profile(
    ir: &str,
    profdata_path: &str,
    opt_level: LtoOptLevel,
) -> Result<String, String> {
    use inkwell::OptimizationLevel;
    use inkwell::passes::PassBuilderOptions;
    use inkwell::targets::{InitializationConfig, Target, TargetMachine};

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("failed to initialize native target: {}", e))?;

    let context = Context::create();
    let memory_buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
        ir.as_bytes(),
        "input_ir",
    );
    let module = context
        .create_module_from_ir(memory_buffer)
        .map_err(|e| format!("failed to parse IR: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|e| format!("failed to create target from triple: {}", e))?;
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Aggressive,
            inkwell::targets::RelocMode::Default,
            inkwell::targets::CodeModel::Default,
        )
        .ok_or_else(|| "failed to create target machine".to_string())?;

    let pass_options = PassBuilderOptions::create();
    pass_options.set_verify_each(false);
    pass_options.set_loop_vectorization(true);
    pass_options.set_loop_unrolling(true);
    pass_options.set_merge_functions(true);

    let pipeline = format!(
        "pgo-instr-use<profile-file={}>,{}",
        profdata_path,
        opt_level.pipeline_string()
    );

    module
        .run_passes(&pipeline, &machine, pass_options)
        .map_err(|e| format!("PGO optimization pass failed: {}", e))?;

    Ok(module.print_to_string().to_string())
}

fn is_pure_builtin(name: &str) -> Option<Purity> {
    match name {
        "sqrt" | "abs" | "floor" | "ceil" | "round" | "sin" | "cos" | "tan" | "asin" | "acos"
        | "atan" | "atan2" | "exp" | "ln" | "log2" | "log10" | "pow" | "min" | "max" | "clamp" => {
            Some(Purity::Pure)
        }

        "is_number" | "is_string" | "is_bool" | "is_list" | "is_map" | "is_none" | "is_err"
        | "is_some" => Some(Purity::Pure),

        "length" | "to_string" | "number_to_string" | "char_at" | "char_code"
        | "from_char_code" => Some(Purity::Pure),

        "log" | "print" | "println" | "read_file" | "write_file" | "append_file" | "exit"
        | "push" | "pop" | "set" => Some(Purity::Effectful),
        _ => None,
    }
}

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

    pub fn compile_to_ir_with_warnings(
        &self,
        source: &str,
    ) -> Result<(String, Vec<crate::diagnostics::Diagnostic>), CompileError> {
        let tokens = lexer::lex(source)
            .map_err(|diag| CompileError::with_source(Stage::Lex, diag, source))?;
        let parser = Parser::new(tokens, source.len());
        let program = parser
            .parse()
            .map_err(|diag| CompileError::with_source(Stage::Parse, diag, source))?;
        let program = lower::lower(program)
            .map_err(|diag| CompileError::with_source(Stage::Parse, diag, source))?;
        let mut model = semantic::analyze(program)
            .map_err(|diag| CompileError::with_source(Stage::Semantic, diag, source))?;
        self.maybe_emit_alloc_report(&model);

        let warnings: Vec<crate::diagnostics::Diagnostic> = model.warnings.clone();

        Self::fold_expressions(&mut model)
            .map_err(|diag| CompileError::with_source(Stage::Semantic, diag, source))?;

        let context = Context::create();
        let inline_mode = match std::env::var("CORAL_INLINE_ASM") {
            Ok(val) if val.eq_ignore_ascii_case("allow-noop") => InlineAsmMode::Noop,
            Ok(val) if val.eq_ignore_ascii_case("emit") => InlineAsmMode::Emit,
            _ => InlineAsmMode::Deny,
        };
        let mut generator =
            CodeGenerator::new(&context, "coral_module").with_inline_asm_mode(inline_mode);

        if std::env::var("CORAL_DEBUG_INFO").map_or(false, |v| !v.is_empty()) {
            generator = generator.with_debug_info("coral_module.coral", source);
        }
        let module = generator
            .compile(&model)
            .map_err(|diag| CompileError::with_source(Stage::Codegen, diag, source))?;
        Ok((module.print_to_string().to_string(), warnings))
    }

    pub fn compile_modules_to_ir(
        &self,
        module_sources: &[ModuleSource],
    ) -> Result<(String, Vec<crate::diagnostics::Diagnostic>), CompileError> {
        let mut modules = Vec::new();

        let mut all_source = String::new();

        for ms in module_sources {
            let tokens = lexer::lex(&ms.source)
                .map_err(|diag| CompileError::with_source(Stage::Lex, diag, &ms.source))?;
            let parser = Parser::new(tokens, ms.source.len());
            let parsed = parser
                .parse()
                .map_err(|diag| CompileError::with_source(Stage::Parse, diag, &ms.source))?;

            modules.push(Module {
                name: ms.name.clone(),
                items: parsed.items,
                imports: ms.imports.clone(),
                exports: ms.exports.clone(),
                span: parsed.span,
            });
            all_source.push_str(&ms.source);
            all_source.push('\n');
        }

        let program = Program::from_modules(modules);
        let program = lower::lower(program)
            .map_err(|diag| CompileError::with_source(Stage::Parse, diag, &all_source))?;
        let mut model = semantic::analyze(program)
            .map_err(|diag| CompileError::with_source(Stage::Semantic, diag, &all_source))?;
        self.maybe_emit_alloc_report(&model);

        let warnings: Vec<crate::diagnostics::Diagnostic> = model.warnings.clone();
        Self::fold_expressions(&mut model)
            .map_err(|diag| CompileError::with_source(Stage::Semantic, diag, &all_source))?;

        let context = Context::create();
        let inline_mode = match std::env::var("CORAL_INLINE_ASM") {
            Ok(val) if val.eq_ignore_ascii_case("allow-noop") => InlineAsmMode::Noop,
            Ok(val) if val.eq_ignore_ascii_case("emit") => InlineAsmMode::Emit,
            _ => InlineAsmMode::Deny,
        };
        let mut generator =
            CodeGenerator::new(&context, "coral_module").with_inline_asm_mode(inline_mode);
        if std::env::var("CORAL_DEBUG_INFO").map_or(false, |v| !v.is_empty()) {
            generator = generator.with_debug_info("coral_module.coral", &all_source);
        }
        let module = generator
            .compile(&model)
            .map_err(|diag| CompileError::with_source(Stage::Codegen, diag, &all_source))?;
        Ok((module.print_to_string().to_string(), warnings))
    }

    pub fn compile_modules_to_ir_cached(
        &self,
        module_sources: &[ModuleSource],
        cache_dir: &Path,
    ) -> Result<(String, Vec<crate::diagnostics::Diagnostic>, bool), CompileError> {
        let cache = ModuleCache::new(cache_dir);
        let fingerprint = ModuleCache::fingerprint(module_sources);

        if let Some(cached_ir) = cache.get(fingerprint) {
            return Ok((cached_ir, vec![], true));
        }

        let (ir, warnings) = self.compile_modules_to_ir(module_sources)?;

        let _ = cache.put(fingerprint, &ir);

        Ok((ir, warnings, false))
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
                    name, m, alloc, usage.reads, usage.mutations, usage.escapes, usage.calls,
                ));
            }
            let _ = std::fs::write(path, out);
        }
    }

    fn fold_expressions(
        model: &mut semantic::SemanticModel,
    ) -> Result<(), crate::diagnostics::Diagnostic> {
        let func_table: HashMap<String, Function> = model
            .functions
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        let folder = ConstFolder::new(&func_table);

        for binding in &mut model.globals {
            binding.value = folder.fold_expr(binding.value.clone(), 0);
        }

        for func in &mut model.functions {
            folder.fold_block(&mut func.body, 0);
        }

        let errors = folder.errors.into_inner();
        if let Some(err) = errors.into_iter().next() {
            return Err(err);
        }
        Ok(())
    }

}

#[allow(dead_code)]
const MAX_COMPTIME_DEPTH: usize = 16;
#[allow(dead_code)]
const MAX_COMPTIME_STEPS: usize = 10000;

#[allow(dead_code)]
struct ConstFolder<'a> {
    func_table: &'a HashMap<String, Function>,
    errors: std::cell::RefCell<Vec<crate::diagnostics::Diagnostic>>,
}

#[allow(dead_code)]
impl<'a> ConstFolder<'a> {
    fn new(func_table: &'a HashMap<String, Function>) -> Self {
        Self {
            func_table,
            errors: std::cell::RefCell::new(Vec::new()),
        }
    }

    fn fold_block(&self, block: &mut Block, depth: usize) {
        let mut new_stmts: Vec<Statement> = Vec::new();
        for stmt in std::mem::take(&mut block.statements) {
            match stmt {
                Statement::Binding(mut binding) => {
                    binding.value = self.fold_expr(binding.value.clone(), depth);
                    new_stmts.push(Statement::Binding(binding));
                }
                Statement::Expression(expr) => {
                    let folded = self.fold_expr(expr, depth);
                    new_stmts.push(Statement::Expression(folded));
                }
                Statement::Return(expr, span) => {
                    let folded = self.fold_expr(expr, depth);
                    new_stmts.push(Statement::Return(folded, span));
                }
                Statement::If {
                    condition,
                    mut body,
                    elif_branches,
                    else_body,
                    span,
                } => {
                    let folded_cond = self.fold_expr(condition, depth);
                    self.fold_block(&mut body, depth);

                    if let Expression::Bool(true, _) = &folded_cond {
                        for s in body.statements {
                            new_stmts.push(s);
                        }
                        if let Some(val) = body.value {
                            new_stmts.push(Statement::Expression(*val));
                        }
                        continue;
                    }
                    if let Expression::Bool(false, _) = &folded_cond {
                        let mut taken = false;
                        let mut else_body = else_body;
                        let mut remaining_elifs: Vec<(Expression, Block)> = Vec::new();
                        let mut pass_through = false;
                        for (cond, mut blk) in elif_branches {
                            if pass_through {
                                let fc = self.fold_expr(cond, depth);
                                self.fold_block(&mut blk, depth);
                                remaining_elifs.push((fc, blk));
                                continue;
                            }
                            let folded_elif = self.fold_expr(cond, depth);
                            self.fold_block(&mut blk, depth);
                            if let Expression::Bool(true, _) = &folded_elif {
                                for s in blk.statements {
                                    new_stmts.push(s);
                                }
                                if let Some(val) = blk.value {
                                    new_stmts.push(Statement::Expression(*val));
                                }
                                taken = true;
                                break;
                            }
                            if let Expression::Bool(false, _) = &folded_elif {
                                continue;
                            }
                            remaining_elifs.push((folded_elif, blk));
                            pass_through = true;
                        }
                        if !taken && !remaining_elifs.is_empty() {
                            let (first_cond, first_body) = remaining_elifs.remove(0);
                            let folded_else = else_body.take().map(|mut blk| {
                                self.fold_block(&mut blk, depth);
                                blk
                            });
                            new_stmts.push(Statement::If {
                                condition: first_cond,
                                body: first_body,
                                elif_branches: remaining_elifs,
                                else_body: folded_else,
                                span,
                            });
                            taken = true;
                        }
                        if !taken {
                            if let Some(mut else_blk) = else_body {
                                self.fold_block(&mut else_blk, depth);
                                for s in else_blk.statements {
                                    new_stmts.push(s);
                                }
                                if let Some(val) = else_blk.value {
                                    new_stmts.push(Statement::Expression(*val));
                                }
                            }
                        }
                        continue;
                    }

                    let mut folded_elifs = Vec::new();
                    for (cond, mut blk) in elif_branches {
                        let fc = self.fold_expr(cond, depth);
                        self.fold_block(&mut blk, depth);
                        folded_elifs.push((fc, blk));
                    }
                    let folded_else = else_body.map(|mut blk| {
                        self.fold_block(&mut blk, depth);
                        blk
                    });
                    new_stmts.push(Statement::If {
                        condition: folded_cond,
                        body,
                        elif_branches: folded_elifs,
                        else_body: folded_else,
                        span,
                    });
                }
                Statement::While {
                    condition,
                    mut body,
                    span,
                } => {
                    let folded_cond = self.fold_expr(condition, depth);
                    self.fold_block(&mut body, depth);
                    new_stmts.push(Statement::While {
                        condition: folded_cond,
                        body,
                        span,
                    });
                }
                Statement::For {
                    iterable,
                    mut body,
                    variable,
                    span,
                } => {
                    let folded_iter = self.fold_expr(iterable, depth);
                    self.fold_block(&mut body, depth);
                    new_stmts.push(Statement::For {
                        iterable: folded_iter,
                        body,
                        variable,
                        span,
                    });
                }
                Statement::ForKV {
                    iterable,
                    mut body,
                    key_var,
                    value_var,
                    span,
                } => {
                    let folded_iter = self.fold_expr(iterable, depth);
                    self.fold_block(&mut body, depth);
                    new_stmts.push(Statement::ForKV {
                        iterable: folded_iter,
                        body,
                        key_var,
                        value_var,
                        span,
                    });
                }
                Statement::ForRange {
                    start,
                    end,
                    step,
                    mut body,
                    variable,
                    span,
                } => {
                    let folded_start = self.fold_expr(start, depth);
                    let folded_end = self.fold_expr(end, depth);
                    let folded_step = step.map(|s| self.fold_expr(s, depth));
                    self.fold_block(&mut body, depth);
                    new_stmts.push(Statement::ForRange {
                        start: folded_start,
                        end: folded_end,
                        step: folded_step,
                        body,
                        variable,
                        span,
                    });
                }
                stmt @ (Statement::Break(_) | Statement::Continue(_)) => {
                    new_stmts.push(stmt);
                }
                Statement::FieldAssign {
                    target,
                    field,
                    value,
                    span,
                } => {
                    let folded_val = self.fold_expr(value, depth);
                    new_stmts.push(Statement::FieldAssign {
                        target,
                        field,
                        value: folded_val,
                        span,
                    });
                }
                Statement::PatternBinding {
                    pattern,
                    value,
                    span,
                } => {
                    let folded_val = self.fold_expr(value, depth);
                    new_stmts.push(Statement::PatternBinding {
                        pattern,
                        value: folded_val,
                        span,
                    });
                }
            }
        }
        block.statements = new_stmts;

        block.statements.retain(|stmt| {
            if let Statement::Expression(expr) = stmt {
                !is_pure_dead_expression(expr)
            } else {
                true
            }
        });

        if let Some(value) = &mut block.value {
            *value = Box::new(self.fold_expr(*value.clone(), depth));
        }
    }

    fn fold_expr(&self, expr: Expression, depth: usize) -> Expression {
        match expr {
            Expression::Binary {
                op,
                left,
                right,
                span,
            } => {
                let left = Box::new(self.fold_expr(*left, depth));
                let right = Box::new(self.fold_expr(*right, depth));

                match (left.as_ref(), right.as_ref()) {
                    (Expression::Integer(a, _), Expression::Integer(b, _)) => match op {
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
                    },
                    (Expression::Float(a, _), Expression::Float(b, _)) => match op {
                        BinaryOp::Add => return Expression::Float(a + b, span),
                        BinaryOp::Sub => return Expression::Float(a - b, span),
                        BinaryOp::Mul => return Expression::Float(a * b, span),
                        BinaryOp::Div => return Expression::Float(a / b, span),
                        BinaryOp::Less => return Expression::Bool(a < b, span),
                        BinaryOp::LessEq => return Expression::Bool(a <= b, span),
                        BinaryOp::Greater => return Expression::Bool(a > b, span),
                        BinaryOp::GreaterEq => return Expression::Bool(a >= b, span),
                        _ => {}
                    },
                    (Expression::Bool(a, _), Expression::Bool(b, _)) => match op {
                        BinaryOp::And => return Expression::Bool(*a && *b, span),
                        BinaryOp::Or => return Expression::Bool(*a || *b, span),
                        BinaryOp::Equals => return Expression::Bool(a == b, span),
                        _ => {}
                    },
                    (Expression::String(a, _), Expression::String(b, _)) => {
                        if op == BinaryOp::Add {
                            let mut result = a.clone();
                            result.push_str(b);
                            return Expression::String(result, span);
                        }
                    }
                    _ => {}
                }

                Expression::Binary {
                    op,
                    left,
                    right,
                    span,
                }
            }
            Expression::Unary { op, expr, span } => {
                let inner = Box::new(self.fold_expr(*expr, depth));

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

                Expression::Unary {
                    op,
                    expr: inner,
                    span,
                }
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                let cond = Box::new(self.fold_expr(*condition, depth));
                let then_b = Box::new(self.fold_expr(*then_branch, depth));
                let else_b = Box::new(self.fold_expr(*else_branch, depth));

                if let Expression::Bool(b, _) = cond.as_ref() {
                    return if *b { *then_b } else { *else_b };
                }

                Expression::Ternary {
                    condition: cond,
                    then_branch: then_b,
                    else_branch: else_b,
                    span,
                }
            }
            Expression::Call {
                callee,
                args,
                arg_names,
                span,
                ..
            } => {
                let callee = Box::new(self.fold_expr(*callee, depth));
                let args: Vec<_> = args.into_iter().map(|a| self.fold_expr(a, depth)).collect();

                if let Expression::Identifier(name, _) = callee.as_ref() {
                    if args.len() == 1 {
                        let const_val = match &args[0] {
                            Expression::Float(f, _) => Some(*f),
                            Expression::Integer(i, _) => Some(*i as f64),
                            _ => None,
                        };
                        if let Some(val) = const_val {
                            if let Some(result) = eval_math_const(name, val) {
                                if result == (result as i64) as f64
                                    && result.abs() < i64::MAX as f64
                                {
                                    return Expression::Integer(result as i64, span);
                                }
                                return Expression::Float(result, span);
                            }
                        }
                    }

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
                                if result == (result as i64) as f64
                                    && result.abs() < i64::MAX as f64
                                {
                                    return Expression::Integer(result as i64, span);
                                }
                                return Expression::Float(result, span);
                            }
                        }
                    }

                    if name == "length" && args.len() == 1 {
                        if let Expression::String(ref s, _) = args[0] {
                            return Expression::Integer(s.len() as i64, span);
                        }
                        if let Expression::List(ref items, _) = args[0] {
                            return Expression::Integer(items.len() as i64, span);
                        }
                    }

                    if name == "assert_static" && args.len() >= 1 {
                        match &args[0] {
                            Expression::Bool(false, _) => {
                                let msg = if args.len() >= 2 {
                                    if let Expression::String(s, _) = &args[1] {
                                        s.clone()
                                    } else {
                                        "static assertion failed".to_string()
                                    }
                                } else {
                                    "static assertion failed".to_string()
                                };
                                self.errors.borrow_mut().push(
                                    crate::diagnostics::Diagnostic::new(
                                        format!("assert_static: {}", msg),
                                        span,
                                    ),
                                );
                                return Expression::Unit;
                            }
                            Expression::Bool(true, _) => {
                                return Expression::Unit;
                            }
                            _ => {}
                        }
                    }

                    if (name == "regex_match"
                        || name == "regex_find"
                        || name == "regex_find_all"
                        || name == "regex_replace"
                        || name == "regex_split")
                        && !args.is_empty()
                    {
                        if let Expression::String(ref pattern, _) = args[0] {
                            if let Err(e) = validate_regex_syntax(pattern) {
                                self.errors.borrow_mut().push(
                                    crate::diagnostics::Diagnostic::new(
                                        format!(
                                            "invalid regex pattern \"{}\" in call to {}: {}",
                                            pattern, name, e
                                        ),
                                        span,
                                    ),
                                );
                            }
                        }
                    }

                    if name == "to_string" && args.len() == 1 {
                        match &args[0] {
                            Expression::Integer(i, _) => {
                                return Expression::String(i.to_string(), span);
                            }
                            Expression::Float(f, _) => {
                                return Expression::String(f.to_string(), span);
                            }
                            Expression::Bool(b, _) => {
                                return Expression::String(b.to_string(), span);
                            }
                            Expression::String(_, _) => {
                                return args.into_iter().next().unwrap();
                            }
                            _ => {}
                        }
                    }

                    if name == "char_at" && args.len() == 2 {
                        if let (Expression::String(s, _), Expression::Integer(idx, _)) =
                            (&args[0], &args[1])
                        {
                            let idx = *idx as usize;
                            if idx < s.len() {
                                if let Some(ch) = s.chars().nth(idx) {
                                    return Expression::String(
                                        ch.to_string(),
                                        span,
                                    );
                                }
                            }
                        }
                    }

                    if name == "char_code" && args.len() == 1 {
                        if let Expression::String(ref s, _) = args[0] {
                            if let Some(ch) = s.chars().next() {
                                return Expression::Integer(ch as i64, span);
                            }
                        }
                    }

                    if name == "from_char_code" && args.len() == 1 {
                        if let Expression::Integer(code, _) = args[0] {
                            if let Some(ch) = char::from_u32(code as u32) {
                                return Expression::String(ch.to_string(), span);
                            }
                        }
                    }

                }

                Expression::Call {
                    callee,
                    args,
                    arg_names,
                    span,
                }
            }
            Expression::List(items, span) => {
                let items: Vec<_> = items
                    .into_iter()
                    .map(|i| self.fold_expr(i, depth))
                    .collect();
                Expression::List(items, span)
            }
            Expression::Map(entries, span) => {
                let entries: Vec<_> = entries
                    .into_iter()
                    .map(|(k, v)| (self.fold_expr(k, depth), self.fold_expr(v, depth)))
                    .collect();
                Expression::Map(entries, span)
            }
            Expression::Member {
                target,
                property,
                span,
            } => {
                let target = Box::new(self.fold_expr(*target, depth));

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
                Expression::Member {
                    target,
                    property,
                    span,
                }
            }
            Expression::Index {
                target,
                index,
                span,
            } => {
                let target = Box::new(self.fold_expr(*target, depth));
                let index = Box::new(self.fold_expr(*index, depth));
                Expression::Index {
                    target,
                    index,
                    span,
                }
            }
            Expression::Lambda {
                params,
                mut body,
                span,
            } => {
                self.fold_block(&mut body, depth);
                Expression::Lambda { params, body, span }
            }

            other => other,
        }
    }

    fn is_const_expr(expr: &Expression) -> bool {
        matches!(
            expr,
            Expression::Integer(_, _)
                | Expression::Float(_, _)
                | Expression::Bool(_, _)
                | Expression::String(_, _)
                | Expression::Unit
                | Expression::None(_)
        )
    }

    fn is_body_pure(block: &Block) -> bool {
        for stmt in &block.statements {
            match stmt {
                Statement::Binding(b) => {
                    if !Self::is_expr_pure(&b.value) {
                        return false;
                    }
                }
                Statement::Expression(e) => {
                    if !Self::is_expr_pure(e) {
                        return false;
                    }
                }
                Statement::Return(e, _) => {
                    if !Self::is_expr_pure(e) {
                        return false;
                    }
                }
                Statement::If {
                    condition,
                    body,
                    elif_branches,
                    else_body,
                    ..
                } => {
                    if !Self::is_expr_pure(condition) || !Self::is_body_pure(body) {
                        return false;
                    }
                    for (c, b) in elif_branches {
                        if !Self::is_expr_pure(c) || !Self::is_body_pure(b) {
                            return false;
                        }
                    }
                    if let Some(eb) = else_body {
                        if !Self::is_body_pure(eb) {
                            return false;
                        }
                    }
                }
                _ => return false,
            }
        }
        if let Some(v) = &block.value {
            if !Self::is_expr_pure(v) {
                return false;
            }
        }
        true
    }

    fn is_expr_pure(expr: &Expression) -> bool {
        match expr {
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::Bool(_, _)
            | Expression::String(_, _)
            | Expression::Unit
            | Expression::None(_)
            | Expression::Identifier(_, _) => true,
            Expression::Binary { left, right, .. } => {
                Self::is_expr_pure(left) && Self::is_expr_pure(right)
            }
            Expression::Unary { expr, .. } => Self::is_expr_pure(expr),
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                Self::is_expr_pure(condition)
                    && Self::is_expr_pure(then_branch)
                    && Self::is_expr_pure(else_branch)
            }
            Expression::Call { callee, args, .. } => {
                if let Expression::Identifier(name, _) = callee.as_ref() {
                    if is_pure_builtin(name) == Some(Purity::Pure) {
                        return args.iter().all(Self::is_expr_pure);
                    }
                    if let Some(Purity::Effectful | Purity::ReadOnly) = is_pure_builtin(name) {
                        return false;
                    }
                    return args.iter().all(Self::is_expr_pure);
                }
                false
            }
            Expression::List(items, _) => items.iter().all(Self::is_expr_pure),
            Expression::Map(entries, _) => entries
                .iter()
                .all(|(k, v)| Self::is_expr_pure(k) && Self::is_expr_pure(v)),
            _ => false,
        }
    }

    fn try_eval_user_function(
        &self,
        name: &str,
        args: &[Expression],
        _span: crate::span::Span,
        depth: usize,
    ) -> Option<Expression> {
        if !args.iter().all(Self::is_const_expr) {
            return None;
        }

        let func = self.func_table.get(name)?;

        if func.params.len() != args.len() {
            return None;
        }

        if !Self::is_body_pure(&func.body) {
            return None;
        }

        let mut env: HashMap<String, Expression> = HashMap::new();
        for (param, arg) in func.params.iter().zip(args.iter()) {
            env.insert(param.name.clone(), arg.clone());
        }

        let mut steps = 0;
        self.eval_block(&func.body, &mut env, &mut steps, depth + 1)
    }

    fn eval_block(
        &self,
        block: &Block,
        env: &mut HashMap<String, Expression>,
        steps: &mut usize,
        depth: usize,
    ) -> Option<Expression> {
        for stmt in &block.statements {
            *steps += 1;
            if *steps > MAX_COMPTIME_STEPS {
                return None;
            }
            match stmt {
                Statement::Binding(b) => {
                    let val = self.eval_expr(&b.value, env, steps, depth)?;
                    env.insert(b.name.clone(), val);
                }
                Statement::Return(expr, _) => {
                    return self.eval_expr(expr, env, steps, depth);
                }
                Statement::Expression(expr) => {
                    self.eval_expr(expr, env, steps, depth)?;
                }
                Statement::If {
                    condition,
                    body,
                    elif_branches,
                    else_body,
                    ..
                } => {
                    let cond = self.eval_expr(condition, env, steps, depth)?;
                    if let Expression::Bool(true, _) = &cond {
                        if let Some(result) = self.eval_block_for_return(body, env, steps, depth) {
                            return Some(result);
                        }
                    } else if let Expression::Bool(false, _) = &cond {
                        let mut handled = false;
                        for (ec, eb) in elif_branches {
                            let elif_cond = self.eval_expr(ec, env, steps, depth)?;
                            if let Expression::Bool(true, _) = &elif_cond {
                                if let Some(result) =
                                    self.eval_block_for_return(eb, env, steps, depth)
                                {
                                    return Some(result);
                                }
                                handled = true;
                                break;
                            }
                        }
                        if !handled {
                            if let Some(eb) = else_body {
                                if let Some(result) =
                                    self.eval_block_for_return(eb, env, steps, depth)
                                {
                                    return Some(result);
                                }
                            }
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }

        if let Some(val) = &block.value {
            return self.eval_expr(val, env, steps, depth);
        }

        Some(Expression::Unit)
    }

    fn eval_block_for_return(
        &self,
        block: &Block,
        env: &mut HashMap<String, Expression>,
        steps: &mut usize,
        depth: usize,
    ) -> Option<Expression> {
        for stmt in &block.statements {
            *steps += 1;
            if *steps > MAX_COMPTIME_STEPS {
                return None;
            }
            match stmt {
                Statement::Binding(b) => {
                    let val = self.eval_expr(&b.value, env, steps, depth)?;
                    env.insert(b.name.clone(), val);
                }
                Statement::Return(expr, _) => {
                    return Some(self.eval_expr(expr, env, steps, depth)?);
                }
                Statement::Expression(expr) => {
                    self.eval_expr(expr, env, steps, depth)?;
                }
                _ => return None,
            }
        }
        if let Some(val) = &block.value {
            return Some(self.eval_expr(val, env, steps, depth)?);
        }
        None
    }

    fn eval_expr(
        &self,
        expr: &Expression,
        env: &mut HashMap<String, Expression>,
        steps: &mut usize,
        depth: usize,
    ) -> Option<Expression> {
        *steps += 1;
        if *steps > MAX_COMPTIME_STEPS {
            return None;
        }
        match expr {
            Expression::Integer(n, s) => Some(Expression::Integer(*n, s.clone())),
            Expression::Float(f, s) => Some(Expression::Float(*f, s.clone())),
            Expression::Bool(b, s) => Some(Expression::Bool(*b, s.clone())),
            Expression::String(s, sp) => Some(Expression::String(s.clone(), sp.clone())),
            Expression::Unit => Some(Expression::Unit),
            Expression::None(s) => Some(Expression::None(s.clone())),
            Expression::Identifier(name, _) => env.get(name).cloned(),
            Expression::Binary {
                op,
                left,
                right,
                span,
            } => {
                let l = self.eval_expr(left, env, steps, depth)?;
                let r = self.eval_expr(right, env, steps, depth)?;
                let folded = self.fold_expr(
                    Expression::Binary {
                        op: *op,
                        left: Box::new(l),
                        right: Box::new(r),
                        span: span.clone(),
                    },
                    depth,
                );
                if Self::is_const_expr(&folded) {
                    Some(folded)
                } else {
                    None
                }
            }
            Expression::Unary { op, expr: e, span } => {
                let inner = self.eval_expr(e, env, steps, depth)?;
                let folded = self.fold_expr(
                    Expression::Unary {
                        op: *op,
                        expr: Box::new(inner),
                        span: span.clone(),
                    },
                    depth,
                );
                if Self::is_const_expr(&folded) {
                    Some(folded)
                } else {
                    None
                }
            }
            Expression::Ternary {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let cond = self.eval_expr(condition, env, steps, depth)?;
                match &cond {
                    Expression::Bool(true, _) => self.eval_expr(then_branch, env, steps, depth),
                    Expression::Bool(false, _) => self.eval_expr(else_branch, env, steps, depth),
                    _ => None,
                }
            }
            Expression::Call {
                callee,
                args,
                span,
                ..
            } => {
                if let Expression::Identifier(_name, _) = callee.as_ref() {
                    let eval_args: Vec<_> = args
                        .iter()
                        .map(|a| self.eval_expr(a, env, steps, depth))
                        .collect::<Option<Vec<_>>>()?;

                    let folded = self.fold_expr(
                        Expression::Call {
                            callee: callee.clone(),
                            args: eval_args,
                            arg_names: vec![],
                            span: span.clone(),
                        },
                        depth,
                    );
                    if Self::is_const_expr(&folded) {
                        return Some(folded);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

fn is_pure_dead_expression(expr: &Expression) -> bool {
    match expr {
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::String(_, _)
        | Expression::Unit => true,

        Expression::Identifier(_, _) => true,

        Expression::Binary { left, right, .. } => {
            is_pure_dead_expression(left) && is_pure_dead_expression(right)
        }
        Expression::Unary { expr, .. } => is_pure_dead_expression(expr),

        Expression::List(items, _) => items.iter().all(is_pure_dead_expression),
        Expression::Map(entries, _) => entries
            .iter()
            .all(|(k, v)| is_pure_dead_expression(k) && is_pure_dead_expression(v)),

        Expression::Member { target, .. } => is_pure_dead_expression(target),

        Expression::Index { target, index, .. } => {
            is_pure_dead_expression(target) && is_pure_dead_expression(index)
        }

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

fn validate_regex_syntax(pattern: &str) -> Result<(), String> {
    let mut depth: i32 = 0;
    let mut bracket = false;
    let mut escape = false;
    for (i, ch) in pattern.chars().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '[' if !bracket => bracket = true,
            ']' if bracket => bracket = false,
            '(' if !bracket => depth += 1,
            ')' if !bracket => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("unmatched ')' at position {}", i));
                }
            }
            '*' | '+' | '?' if !bracket && i == 0 => {
                return Err(format!(
                    "quantifier '{}' at position {} has nothing to repeat",
                    ch, i
                ));
            }
            _ => {}
        }
    }
    if escape {
        return Err("trailing backslash".to_string());
    }
    if bracket {
        return Err("unclosed character class '['".to_string());
    }
    if depth > 0 {
        return Err(format!("{} unclosed group(s)", depth));
    }
    Ok(())
}
