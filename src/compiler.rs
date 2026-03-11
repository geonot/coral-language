use crate::ast::{BinaryOp, Expression, Module, Program, UnaryOp};
use crate::codegen::{CodeGenerator, InlineAsmMode};
use crate::diagnostics::{CompileError, Stage};
use crate::lexer;
use crate::lower;
use crate::module_loader::ModuleSource;
use crate::parser::Parser;
use crate::semantic;
use inkwell::context::Context;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

// ═══════════════════════════════════════════════════════════════════════
// CC3.5: Incremental Compilation — Module Cache
// ═══════════════════════════════════════════════════════════════════════

/// Manages a disk-based compilation cache in `.coral-cache/`.
/// Caches the final LLVM IR keyed by a fingerprint of all source modules.
#[derive(Debug)]
pub struct ModuleCache {
    cache_dir: PathBuf,
}

impl ModuleCache {
    /// Create a cache backed by `base_dir/.coral-cache/`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            cache_dir: base_dir.join(".coral-cache"),
        }
    }

    /// Compute a combined fingerprint of all module sources.
    pub fn fingerprint(sources: &[ModuleSource]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for ms in sources {
            ms.name.hash(&mut hasher);
            ms.source.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Look up cached IR for the given fingerprint.
    /// Returns `Some(ir_string)` on cache hit, `None` on miss.
    pub fn get(&self, fingerprint: u64) -> Option<String> {
        let ir_path = self.cache_dir.join(format!("{:016x}.ll", fingerprint));
        std::fs::read_to_string(ir_path).ok()
    }

    /// Store compiled IR for the given fingerprint.
    pub fn put(&self, fingerprint: u64, ir: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let ir_path = self.cache_dir.join(format!("{:016x}.ll", fingerprint));
        let mut f = std::fs::File::create(ir_path)?;
        f.write_all(ir.as_bytes())?;
        Ok(())
    }

    /// Invalidate the entire cache.
    pub fn invalidate_all(&self) -> std::io::Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }
}

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

// ═══════════════════════════════════════════════════════════════════════
// C4.4: Link-Time Optimization (LTO)
// ═══════════════════════════════════════════════════════════════════════

/// Optimization level for LTO passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LtoOptLevel {
    O1,
    O2,
    O3,
}

impl LtoOptLevel {
    /// The LLVM pass pipeline string for the new pass manager.
    pub fn pipeline_string(self) -> &'static str {
        match self {
            LtoOptLevel::O1 => "default<O1>",
            LtoOptLevel::O2 => "default<O2>",
            LtoOptLevel::O3 => "default<O3>",
        }
    }
}

/// Run LLVM optimization passes on an already-compiled module.
/// Uses the new LLVM pass manager (`Module::run_passes`).
/// Returns the optimized IR string.
pub fn optimize_module(ir: &str, opt_level: LtoOptLevel) -> Result<String, String> {
    use inkwell::targets::{InitializationConfig, Target, TargetMachine, TargetTriple};
    use inkwell::OptimizationLevel;
    use inkwell::passes::PassBuilderOptions;

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("failed to initialize native target: {}", e))?;

    let context = Context::create();
    let memory_buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
        ir.as_bytes(),
        "input_ir",
    );
    let module = context.create_module_from_ir(memory_buffer)
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

/// C4.5: Instrument an LLVM IR module for profile collection (PGO generation).
///
/// Inserts profiling counters into the IR so that running the resulting binary
/// produces a `default.profraw` file.  The raw profile can be merged with
/// `llvm-profdata merge` into a `.profdata` file for use with
/// [`optimize_with_profile`].
///
/// The pass pipeline is `"pgo-instr-gen,instrprof"`.
pub fn instrument_for_pgo(ir: &str) -> Result<String, String> {
    use inkwell::targets::{InitializationConfig, Target, TargetMachine};
    use inkwell::OptimizationLevel;
    use inkwell::passes::PassBuilderOptions;

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("failed to initialize native target: {}", e))?;

    let context = Context::create();
    let memory_buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
        ir.as_bytes(),
        "input_ir",
    );
    let module = context.create_module_from_ir(memory_buffer)
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

/// C4.5: Optimize an LLVM IR module using collected profile data (PGO use).
///
/// Reads a `.profdata` file (produced by `llvm-profdata merge`) and applies
/// profile-guided optimizations: hot paths get aggressive inlining and
/// vectorization while cold paths are optimized for size.
pub fn optimize_with_profile(ir: &str, profdata_path: &str, opt_level: LtoOptLevel) -> Result<String, String> {
    use inkwell::targets::{InitializationConfig, Target, TargetMachine};
    use inkwell::OptimizationLevel;
    use inkwell::passes::PassBuilderOptions;

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("failed to initialize native target: {}", e))?;

    let context = Context::create();
    let memory_buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
        ir.as_bytes(),
        "input_ir",
    );
    let module = context.create_module_from_ir(memory_buffer)
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

    // Pipeline: apply PGO instrumentation-use pass, then default optimization
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
    pub fn compile_to_ir_with_warnings(&self, source: &str) -> Result<(String, Vec<crate::diagnostics::Diagnostic>), CompileError> {
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
        let warnings: Vec<crate::diagnostics::Diagnostic> = model.warnings.clone();
        
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

    /// Compile a list of modules (from `ModuleLoader::load_modules()`) to LLVM IR.
    /// Each module is parsed independently, then all ASTs are merged into a single
    /// Program for semantic analysis and codegen. Returns (IR, warnings).
    pub fn compile_modules_to_ir(&self, module_sources: &[ModuleSource]) -> Result<(String, Vec<crate::diagnostics::Diagnostic>), CompileError> {
        // Parse each module independently and build Module ASTs
        let mut modules = Vec::new();
        // We also need the concatenated source for error reporting
        let mut all_source = String::new();

        for ms in module_sources {
            let tokens = lexer::lex(&ms.source).map_err(|diag| {
                CompileError::with_source(Stage::Lex, diag, &ms.source)
            })?;
            let parser = Parser::new(tokens, ms.source.len());
            let parsed = parser.parse().map_err(|diag| {
                CompileError::with_source(Stage::Parse, diag, &ms.source)
            })?;

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

        // Merge into a single flat Program (backward compatible with semantic/codegen)
        let program = Program::from_modules(modules);
        let program = lower::lower(program)
            .map_err(|diag| CompileError::with_source(Stage::Parse, diag, &all_source))?;
        let mut model = semantic::analyze(program)
            .map_err(|diag| CompileError::with_source(Stage::Semantic, diag, &all_source))?;
        self.maybe_emit_alloc_report(&model);

        let warnings: Vec<crate::diagnostics::Diagnostic> = model.warnings.clone();
        Self::fold_expressions(&mut model);

        let context = Context::create();
        let inline_mode = match std::env::var("CORAL_INLINE_ASM") {
            Ok(val) if val.eq_ignore_ascii_case("allow-noop") => InlineAsmMode::Noop,
            Ok(val) if val.eq_ignore_ascii_case("emit") => InlineAsmMode::Emit,
            _ => InlineAsmMode::Deny,
        };
        let mut generator = CodeGenerator::new(&context, "coral_module")
            .with_inline_asm_mode(inline_mode);
        if std::env::var("CORAL_DEBUG_INFO").map_or(false, |v| !v.is_empty()) {
            generator = generator.with_debug_info("coral_module.coral", &all_source);
        }
        let module = generator
            .compile(&model)
            .map_err(|diag| CompileError::with_source(Stage::Codegen, diag, &all_source))?;
        Ok((module.print_to_string().to_string(), warnings))
    }

    /// CC3.5: Compile modules with disk caching. If a cached IR exists for the
    /// same set of module sources, returns the cached version. Otherwise compiles
    /// and stores the result in the cache. Pass `cache_dir` as the project root.
    pub fn compile_modules_to_ir_cached(
        &self,
        module_sources: &[ModuleSource],
        cache_dir: &Path,
    ) -> Result<(String, Vec<crate::diagnostics::Diagnostic>, bool), CompileError> {
        let cache = ModuleCache::new(cache_dir);
        let fingerprint = ModuleCache::fingerprint(module_sources);

        // Check cache
        if let Some(cached_ir) = cache.get(fingerprint) {
            return Ok((cached_ir, vec![], true));
        }

        // Cache miss — full compilation
        let (ir, warnings) = self.compile_modules_to_ir(module_sources)?;

        // Store in cache (best-effort — don't fail compilation if cache write fails)
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
        let mut new_stmts: Vec<crate::ast::Statement> = Vec::new();
        for stmt in std::mem::take(&mut block.statements) {
            match stmt {
                crate::ast::Statement::Binding(mut binding) => {
                    binding.value = Self::fold_expr(binding.value.clone());
                    new_stmts.push(crate::ast::Statement::Binding(binding));
                }
                crate::ast::Statement::Expression(expr) => {
                    let folded = Self::fold_expr(expr);
                    new_stmts.push(crate::ast::Statement::Expression(folded));
                }
                crate::ast::Statement::Return(expr, span) => {
                    let folded = Self::fold_expr(expr);
                    new_stmts.push(crate::ast::Statement::Return(folded, span));
                }
                crate::ast::Statement::If { condition, mut body, elif_branches, else_body, span } => {
                    let folded_cond = Self::fold_expr(condition);
                    Self::fold_block(&mut body);

                    // C5.2: Dead branch elimination — if condition is constant bool,
                    // inline the taken branch and discard the rest.
                    if let Expression::Bool(true, _) = &folded_cond {
                        // Condition always true — inline the body
                        for s in body.statements {
                            new_stmts.push(s);
                        }
                        // If body has a trailing value, wrap as expression stmt
                        if let Some(val) = body.value {
                            new_stmts.push(crate::ast::Statement::Expression(*val));
                        }
                        continue;
                    }
                    if let Expression::Bool(false, _) = &folded_cond {
                        // Condition always false — check elif branches
                        let mut taken = false;
                        let mut else_body = else_body; // shadow to mut
                        let mut remaining_elifs: Vec<(Expression, crate::ast::Block)> = Vec::new();
                        let mut pass_through = false;
                        for (cond, mut blk) in elif_branches {
                            if pass_through {
                                let fc = Self::fold_expr(cond);
                                Self::fold_block(&mut blk);
                                remaining_elifs.push((fc, blk));
                                continue;
                            }
                            let folded_elif = Self::fold_expr(cond);
                            Self::fold_block(&mut blk);
                            if let Expression::Bool(true, _) = &folded_elif {
                                for s in blk.statements {
                                    new_stmts.push(s);
                                }
                                if let Some(val) = blk.value {
                                    new_stmts.push(crate::ast::Statement::Expression(*val));
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
                            let folded_else = else_body.take().map(|mut blk| { Self::fold_block(&mut blk); blk });
                            new_stmts.push(crate::ast::Statement::If {
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
                                Self::fold_block(&mut else_blk);
                                for s in else_blk.statements {
                                    new_stmts.push(s);
                                }
                                if let Some(val) = else_blk.value {
                                    new_stmts.push(crate::ast::Statement::Expression(*val));
                                }
                            }
                        }
                        continue;
                    }

                    // Non-constant condition — fold sub-expressions and keep
                    let mut folded_elifs = Vec::new();
                    for (cond, mut blk) in elif_branches {
                        let fc = Self::fold_expr(cond);
                        Self::fold_block(&mut blk);
                        folded_elifs.push((fc, blk));
                    }
                    let folded_else = else_body.map(|mut blk| { Self::fold_block(&mut blk); blk });
                    new_stmts.push(crate::ast::Statement::If {
                        condition: folded_cond,
                        body,
                        elif_branches: folded_elifs,
                        else_body: folded_else,
                        span,
                    });
                }
                crate::ast::Statement::While { condition, mut body, span } => {
                    let folded_cond = Self::fold_expr(condition);
                    Self::fold_block(&mut body);
                    new_stmts.push(crate::ast::Statement::While { condition: folded_cond, body, span });
                }
                crate::ast::Statement::For { iterable, mut body, variable, span } => {
                    let folded_iter = Self::fold_expr(iterable);
                    Self::fold_block(&mut body);
                    new_stmts.push(crate::ast::Statement::For { iterable: folded_iter, body, variable, span });
                }
                crate::ast::Statement::ForKV { iterable, mut body, key_var, value_var, span } => {
                    let folded_iter = Self::fold_expr(iterable);
                    Self::fold_block(&mut body);
                    new_stmts.push(crate::ast::Statement::ForKV { iterable: folded_iter, body, key_var, value_var, span });
                }
                crate::ast::Statement::ForRange { start, end, step, mut body, variable, span } => {
                    let folded_start = Self::fold_expr(start);
                    let folded_end = Self::fold_expr(end);
                    let folded_step = step.map(Self::fold_expr);
                    Self::fold_block(&mut body);
                    new_stmts.push(crate::ast::Statement::ForRange { start: folded_start, end: folded_end, step: folded_step, body, variable, span });
                }
                stmt @ (crate::ast::Statement::Break(_) | crate::ast::Statement::Continue(_)) => {
                    new_stmts.push(stmt);
                }
                crate::ast::Statement::FieldAssign { target, field, value, span } => {
                    let folded_val = Self::fold_expr(value);
                    new_stmts.push(crate::ast::Statement::FieldAssign { target, field, value: folded_val, span });
                }
                crate::ast::Statement::PatternBinding { pattern, value, span } => {
                    let folded_val = Self::fold_expr(value);
                    new_stmts.push(crate::ast::Statement::PatternBinding { pattern, value: folded_val, span });
                }
            }
        }
        block.statements = new_stmts;
        
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

                    // C5.2: assert_static() — compile-time assertion
                    // If the argument folds to false, emit a compile-time panic.
                    // If it folds to true, eliminate the call entirely (return unit).
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
                                panic!("assert_static: {}", msg);
                            }
                            Expression::Bool(true, _) => {
                                // Assertion passed at compile time — elide the call
                                return Expression::Unit;
                            }
                            _ => {
                                // Not a constant — leave as runtime call
                            }
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
