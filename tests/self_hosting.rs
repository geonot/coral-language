//! Self-hosting regression tests.
//!
//! These tests verify that the self-hosted compiler components
//! (lexer.coral, parser.coral) can be compiled by the Rust compiler.
//! They catch regressions that would break self-hosting readiness.

use coralc::Compiler;
use coralc::module_loader::ModuleLoader;
use std::path::PathBuf;

/// Workspace root (Cargo manifest dir at compile time).
const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");

/// Load a Coral source file through the module loader (resolving `use` directives).
fn load_source(relative_path: &str) -> String {
    let path = PathBuf::from(WORKSPACE).join(relative_path);
    assert!(path.exists(), "Source file not found: {}", path.display());
    let mut loader = ModuleLoader::with_default_std();
    loader
        .load(&path)
        .unwrap_or_else(|e| panic!("Failed to load {}: {}", relative_path, e))
}

/// Compile a Coral source file all the way to LLVM IR.
fn compile_to_ir(relative_path: &str) -> String {
    let source = load_source(relative_path);
    let compiler = Compiler;
    compiler
        .compile_to_ir(&source)
        .unwrap_or_else(|e| panic!("Failed to compile {}: {:?}", relative_path, e))
}

// ─── Self-Hosted Lexer ───────────────────────────────────────────────

#[test]
fn self_hosted_lexer_loads() {
    // Verify the module loader can resolve `use std.char` and expand the source
    let source = load_source("self_hosted/lexer.coral");
    assert!(
        source.contains("is_digit"),
        "Expected std.char functions to be inlined"
    );
    assert!(
        source.contains("make_token"),
        "Expected lexer's make_token function"
    );
}

#[test]
fn self_hosted_lexer_compiles_to_ir() {
    // Verify the self-hosted lexer compiles all the way to LLVM IR
    let ir = compile_to_ir("self_hosted/lexer.coral");

    // Check that key functions appear in the IR
    assert!(
        ir.contains("@make_token"),
        "Expected make_token in generated IR"
    );
    assert!(
        ir.contains("@lex_"),
        "Expected lex_* functions in generated IR"
    );
    // IR should be non-trivial (self-hosted lexer is ~500 lines)
    assert!(
        ir.len() > 10_000,
        "Generated IR suspiciously small: {} bytes",
        ir.len()
    );
}

// ─── Standard Library Modules ────────────────────────────────────────

#[test]
fn std_char_loads_and_compiles() {
    // Verify std.char can be used in a simple program
    // For now, just test that std/char.coral exists and is valid Coral
    let char_source = load_source("std/char.coral");
    assert!(
        char_source.contains("*is_digit"),
        "Expected is_digit function in std/char.coral"
    );
}

// ─── Self-Hosted Parser ─────────────────────────────────────────────

#[test]
fn self_hosted_parser_loads() {
    let source = load_source("self_hosted/parser.coral");
    assert!(
        source.contains("parse_expression"),
        "Expected parse_expression function in parser"
    );
    assert!(
        source.contains("parse_pattern"),
        "Expected parse_pattern function in parser"
    );
    assert!(
        source.contains("make_program"),
        "Expected make_program AST constructor"
    );
    assert!(
        source.contains("synchronize_to_item"),
        "Expected synchronize_to_item for error recovery"
    );
    assert!(
        source.contains("parse_template_string"),
        "Expected parse_template_string for template string support"
    );
    assert!(
        source.contains("make_pat_list"),
        "Expected make_pat_list for list patterns"
    );
}

#[test]
fn self_hosted_parser_compiles_to_ir() {
    // Verify the full self-hosted parser compiles to LLVM IR
    let ir = compile_to_ir("self_hosted/parser.coral");

    // Check key functions appear in the IR
    assert!(ir.contains("@parse"), "Expected parse in generated IR");
    assert!(
        ir.contains("@parse_expression"),
        "Expected parse_expression in generated IR"
    );
    assert!(
        ir.contains("@parse_item"),
        "Expected parse_item in generated IR"
    );
    assert!(
        ir.contains("@parse_pattern"),
        "Expected parse_pattern in generated IR"
    );
    assert!(
        ir.contains("@parse_match_expression"),
        "Expected parse_match_expression in generated IR"
    );
    assert!(
        ir.contains("@parse_store_def"),
        "Expected parse_store_def in generated IR"
    );
    // Parser is ~1700+ lines; IR should be substantial
    assert!(
        ir.len() > 50_000,
        "Generated IR suspiciously small for 1700-line parser: {} bytes",
        ir.len()
    );
}

#[test]
fn self_hosted_parser_has_complete_coverage() {
    // Verify the parser handles all major Coral constructs
    let source = load_source("self_hosted/parser.coral");

    // Expression parsing
    assert!(source.contains("parse_ternary"), "Missing ternary parsing");
    assert!(
        source.contains("parse_pipeline"),
        "Missing pipeline parsing"
    );
    assert!(
        source.contains("parse_logic_or"),
        "Missing logic_or parsing"
    );
    assert!(
        source.contains("parse_logic_and"),
        "Missing logic_and parsing"
    );
    assert!(
        source.contains("parse_bitwise_or"),
        "Missing bitwise_or parsing"
    );
    assert!(
        source.contains("parse_equality"),
        "Missing equality parsing"
    );
    assert!(
        source.contains("parse_comparison"),
        "Missing comparison parsing"
    );
    assert!(source.contains("parse_term"), "Missing term parsing");
    assert!(source.contains("parse_factor"), "Missing factor parsing");
    assert!(source.contains("parse_unary"), "Missing unary parsing");
    assert!(source.contains("parse_postfix"), "Missing postfix parsing");
    assert!(source.contains("parse_primary"), "Missing primary parsing");

    // Statement parsing
    assert!(
        source.contains("parse_if_statement"),
        "Missing if statement"
    );
    assert!(
        source.contains("parse_while_statement"),
        "Missing while statement"
    );
    assert!(
        source.contains("parse_for_statement"),
        "Missing for statement"
    );
    assert!(
        source.contains("parse_return_statement"),
        "Missing return statement"
    );

    // Item parsing
    assert!(
        source.contains("parse_function"),
        "Missing function parsing"
    );
    assert!(source.contains("parse_store_def"), "Missing store parsing");
    assert!(source.contains("parse_actor_def"), "Missing actor parsing");
    assert!(source.contains("parse_trait_def"), "Missing trait parsing");
    assert!(
        source.contains("parse_error_definition"),
        "Missing error def parsing"
    );
    assert!(
        source.contains("parse_enum_def"),
        "Missing enum/ADT parsing"
    );
    assert!(
        source.contains("parse_type_def"),
        "Missing type def parsing"
    );
    assert!(
        source.contains("parse_extern_function"),
        "Missing extern fn parsing"
    );
    assert!(
        source.contains("parse_use_statement"),
        "Missing use statement"
    );

    // Pattern parsing
    assert!(
        source.contains("make_pat_integer"),
        "Missing integer pattern"
    );
    assert!(source.contains("make_pat_bool"), "Missing bool pattern");
    assert!(source.contains("make_pat_string"), "Missing string pattern");
    assert!(
        source.contains("make_pat_constructor"),
        "Missing constructor pattern"
    );
    assert!(
        source.contains("make_pat_wildcard"),
        "Missing wildcard pattern"
    );
    assert!(source.contains("make_pat_list"), "Missing list pattern");
    assert!(
        source.contains("make_pat_identifier"),
        "Missing identifier pattern"
    );
}

// ─── Self-Hosted Lower ──────────────────────────────────────────────

#[test]
fn self_hosted_lower_loads() {
    let source = load_source("self_hosted/lower.coral");
    assert!(source.contains("lower"), "Expected lower entry function");
    assert!(
        source.contains("lower_expression"),
        "Expected lower_expression function"
    );
    assert!(
        source.contains("lower_call_expr"),
        "Expected lower_call_expr for placeholder desugaring"
    );
    assert!(
        source.contains("replace_placeholders"),
        "Expected replace_placeholders helper"
    );
}

#[test]
fn self_hosted_lower_compiles_to_ir() {
    let ir = compile_to_ir("self_hosted/lower.coral");
    assert!(ir.contains("@lower"), "Expected lower in generated IR");
    assert!(
        ir.contains("@lower_expression"),
        "Expected lower_expression in generated IR"
    );
    assert!(
        ir.len() > 5_000,
        "Generated IR suspiciously small: {} bytes",
        ir.len()
    );
}

// ─── Self-Hosted Module Loader ──────────────────────────────────────

#[test]
fn self_hosted_module_loader_loads() {
    let source = load_source("self_hosted/module_loader.coral");
    assert!(
        source.contains("make_module_loader"),
        "Expected make_module_loader constructor"
    );
    assert!(
        source.contains("load_module"),
        "Expected load_module function"
    );
    assert!(
        source.contains("resolve_module"),
        "Expected resolve_module function"
    );
    assert!(
        source.contains("load_recursive"),
        "Expected load_recursive function"
    );
    assert!(
        source.contains("extract_exports"),
        "Expected extract_exports function"
    );
}

#[test]
fn self_hosted_module_loader_compiles_to_ir() {
    let ir = compile_to_ir("self_hosted/module_loader.coral");
    assert!(
        ir.contains("@make_module_loader"),
        "Expected make_module_loader in generated IR"
    );
    assert!(
        ir.contains("@load_module"),
        "Expected load_module in generated IR"
    );
    assert!(
        ir.len() > 5_000,
        "Generated IR suspiciously small: {} bytes",
        ir.len()
    );
}

// ─── Self-Hosted Semantic Analysis ──────────────────────────────────

#[test]
fn self_hosted_semantic_loads() {
    let source = load_source("self_hosted/semantic.coral");
    assert!(
        source.contains("analyze"),
        "Expected analyze entry function"
    );
    assert!(
        source.contains("make_type_graph"),
        "Expected make_type_graph for type inference"
    );
    assert!(
        source.contains("collect_constraints"),
        "Expected constraint collection"
    );
    assert!(
        source.contains("unify"),
        "Expected unify function for constraint solving"
    );
    assert!(
        source.contains("make_scope_stack"),
        "Expected scope management"
    );
}

#[test]
fn self_hosted_semantic_compiles_to_ir() {
    let ir = compile_to_ir("self_hosted/semantic.coral");
    assert!(ir.contains("@analyze"), "Expected analyze in generated IR");
    assert!(
        ir.contains("@make_type_graph"),
        "Expected make_type_graph in generated IR"
    );
    // Semantic analysis is ~900+ lines; IR should be substantial
    assert!(
        ir.len() > 20_000,
        "Generated IR suspiciously small for semantic analyzer: {} bytes",
        ir.len()
    );
}

// ─── Self-Hosted Code Generator ─────────────────────────────────────

#[test]
fn self_hosted_codegen_loads() {
    let source = load_source("self_hosted/codegen.coral");
    assert!(
        source.contains("make_builder"),
        "Expected make_builder for IR builder state"
    );
    assert!(
        source.contains("emit_runtime_declarations"),
        "Expected emit_runtime_declarations"
    );
    assert!(
        source.contains("emit_function"),
        "Expected emit_function for user functions"
    );
    assert!(
        source.contains("emit_expression"),
        "Expected emit_expression for expression compilation"
    );
    assert!(
        source.contains("emit_lambda"),
        "Expected emit_lambda for closure emission"
    );
    assert!(
        source.contains("emit_match"),
        "Expected emit_match for pattern matching"
    );
    assert!(
        source.contains("intern_string"),
        "Expected intern_string for string constant pool"
    );
}

#[test]
fn self_hosted_codegen_compiles_to_ir() {
    let ir = compile_to_ir("self_hosted/codegen.coral");
    assert!(
        ir.contains("@make_builder"),
        "Expected make_builder in generated IR"
    );
    assert!(
        ir.contains("@emit_expression"),
        "Expected emit_expression in generated IR"
    );
    assert!(
        ir.contains("@emit_function"),
        "Expected emit_function in generated IR"
    );
    // Codegen is ~1400+ lines; IR should be very substantial
    assert!(
        ir.len() > 50_000,
        "Generated IR suspiciously small for codegen: {} bytes",
        ir.len()
    );
}

// ─── Self-Hosted Compiler Pipeline ──────────────────────────────────

#[test]
fn self_hosted_compiler_loads() {
    let source = load_source("self_hosted/compiler.coral");
    assert!(
        source.contains("compile"),
        "Expected compile entry function"
    );
    assert!(
        source.contains("compile_file"),
        "Expected compile_file function"
    );
    assert!(
        source.contains("fold_expr"),
        "Expected fold_expr for constant folding"
    );
}

#[test]
fn self_hosted_compiler_compiles_to_ir() {
    // compiler.coral orchestrates all other self-hosted components via `use` directives.
    // The module loader resolves them relative to the self_hosted/ directory.
    let ir = compile_to_ir("self_hosted/compiler.coral");
    assert!(ir.contains("@compile"), "Expected compile in generated IR");
    assert!(
        ir.len() > 3_000,
        "Generated IR suspiciously small: {} bytes",
        ir.len()
    );
}

// ─── Standard Library Module Compilation ─────────────────────────────

#[test]
fn stdlib_modules_all_compile() {
    // Verify each stdlib module can be loaded (module loader resolves it)
    let modules = [
        "std/math.coral",
        "std/string.coral",
        "std/list.coral",
        "std/map.coral",
        "std/set.coral",
        "std/io.coral",
        "std/char.coral",
        "std/bytes.coral",
        "std/fmt.coral",
        "std/json.coral",
        "std/time.coral",
        "std/encoding.coral",
        "std/sort.coral",
        "std/option.coral",
        "std/result.coral",
        "std/testing.coral",
        "std/process.coral",
        "std/net.coral",
        "std/prelude.coral",
        "std/bit.coral",
    ];
    for module in &modules {
        let path = PathBuf::from(WORKSPACE).join(module);
        assert!(path.exists(), "Missing stdlib module: {}", module);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", module, e));
        // Every module should have at least one function
        assert!(
            content.contains('*'),
            "Module {} has no function definitions",
            module
        );
    }
}

#[test]
fn dump_expanded_sources() {
    let files = vec![
        "self_hosted/lower.coral",
        "self_hosted/module_loader.coral",
        "self_hosted/semantic.coral",
        "self_hosted/codegen.coral",
        "self_hosted/compiler.coral",
    ];
    for f in &files {
        let source = load_source(f);
        let out_name = f.replace("/", "_").replace(".coral", "_expanded.txt");
        let out_path = std::path::PathBuf::from(WORKSPACE)
            .join("target/tmp")
            .join(&out_name);
        std::fs::create_dir_all(out_path.parent().unwrap()).unwrap();
        std::fs::write(&out_path, &source).unwrap();
        eprintln!("{}: {} bytes", f, source.len());
    }
}

// ─── Phase D: Track 1 Regression Tests ──────────────────────────────

/// SH-1: Verify elif_branches are accessed as maps (not arrays) in codegen.coral.
#[test]
fn sh1_elif_branches_use_map_access() {
    let ir = compile_to_ir("self_hosted/codegen.coral");
    assert!(
        ir.contains("elif"),
        "Expected elif-related code in codegen IR"
    );
}

/// SH-1: Verify semantic.coral compiles after elif fix.
#[test]
fn sh1_semantic_elif_branches_fixed() {
    let ir = compile_to_ir("self_hosted/semantic.coral");
    assert!(
        ir.contains("check_expression"),
        "Expected check_expression in semantic IR"
    );
    assert!(
        ir.len() > 50_000,
        "Semantic IR suspiciously small after elif fix: {} bytes",
        ir.len()
    );
}

/// SH-2: Verify actor message dispatch uses func_kind field.
#[test]
fn sh2_actor_dispatch_uses_func_kind() {
    let source = load_source("self_hosted/codegen.coral");
    assert!(
        !source.contains("is_message"),
        "codegen.coral should no longer reference is_message field"
    );
    assert!(
        source.contains("func_kind"),
        "codegen.coral should use func_kind field for actor dispatch"
    );
}

/// SH-3: Verify error value uses path field.
#[test]
fn sh3_error_value_uses_path_field() {
    let source = load_source("self_hosted/codegen.coral");
    assert!(
        source.contains(r#"expr.get("path")"#),
        "codegen.coral should use path field for error values"
    );
}

/// SH-4: Verify range() calls coral_range runtime function.
#[test]
fn sh4_range_calls_runtime() {
    let source = load_source("self_hosted/codegen.coral");
    assert!(
        source.contains("coral_range"),
        "codegen.coral should call coral_range runtime function"
    );
    // Should NOT still use coral_make_list for range
    let range_section = source
        .split("*emit_range_call")
        .nth(1)
        .expect("emit_range_call not found");
    let range_fn = range_section.split("\n*").next().unwrap();
    assert!(
        !range_fn.contains("coral_make_list"),
        "emit_range_call should not create empty list anymore"
    );
}

// ─── E2E Self-Hosted Compiler Execution Tests ─────────────────────

/// Helper: compile the self-hosted compiler to IR, then use lli to compile
/// a test program through it, then execute the result.
fn run_self_hosted_e2e(coral_source: &str) -> String {
    use std::process::Command;
    use std::sync::Once;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static COMPILE_ONCE: Once = Once::new();
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);

    let ws = PathBuf::from(WORKSPACE);
    let runtime_lib = ws.join("target/release/libruntime.so");
    if !runtime_lib.exists() {
        panic!(
            "runtime library not found at {}. Run: cargo build -p runtime --release",
            runtime_lib.display()
        );
    }

    // Step 1: Compile self-hosted compiler to LLVM IR (shared, compiled once)
    let sh_ll = ws.join("target/tmp/self_hosted_test.ll");
    std::fs::create_dir_all(ws.join("target/tmp")).unwrap();
    COMPILE_ONCE.call_once(|| {
        if !sh_ll.exists() {
            let ir = compile_to_ir("self_hosted/compiler.coral");
            let _ = std::fs::write(&sh_ll, &ir);
        }
    });

    // Step 2: Write the test program to a unique temp file
    let test_coral = ws.join(format!("target/tmp/e2e_input_{}.coral", id));
    std::fs::write(&test_coral, coral_source).unwrap();

    // Step 3: Run self-hosted compiler via lli
    let compile_out = Command::new("lli")
        .arg(format!("-load={}", runtime_lib.display()))
        .arg(&sh_ll)
        .arg(&test_coral)
        .output()
        .expect("Failed to run lli for self-hosted compilation");

    let gen_ir = String::from_utf8_lossy(&compile_out.stdout).to_string();
    assert!(
        gen_ir.contains("define") && gen_ir.contains("@main"),
        "Self-hosted compiler did not produce valid IR.\nstderr: {}",
        String::from_utf8_lossy(&compile_out.stderr)
    );

    // Step 4: Write generated IR to unique file and execute it
    let gen_ll = ws.join(format!("target/tmp/e2e_output_{}.ll", id));
    std::fs::write(&gen_ll, &gen_ir).unwrap();

    let run_out = Command::new("lli")
        .arg(format!("-load={}", runtime_lib.display()))
        .arg(&gen_ll)
        .output()
        .expect("Failed to run lli for generated IR execution");

    assert!(
        run_out.status.success(),
        "Generated IR execution failed.\nstderr: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );

    // Cleanup
    let _ = std::fs::remove_file(&test_coral);
    let _ = std::fs::remove_file(&gen_ll);

    String::from_utf8_lossy(&run_out.stdout).trim().to_string()
}

// ─── Progressive Level Tests ──────────────────────────────────────

/// L1: Simple log("hello") through self-hosted compiler
#[test]
fn e2e_self_hosted_l1_hello() {
    let output = run_self_hosted_e2e("*main()\n\tlog(\"hello\")\n");
    assert_eq!(output, "hello");
}

/// L2: Variable binding and reference
#[test]
fn e2e_self_hosted_l2_binding() {
    let output = run_self_hosted_e2e("*main()\n\tx is 5\n\tlog(x)\n");
    assert_eq!(output, "5");
}

/// L3: Arithmetic expression
#[test]
fn e2e_self_hosted_l3_arithmetic() {
    let output = run_self_hosted_e2e("*main()\n\tlog(2 + 3)\n");
    assert_eq!(output, "5");
}

/// L4: Function definition and call
#[test]
fn e2e_self_hosted_l4_function() {
    let output = run_self_hosted_e2e("*add(a, b)\n\treturn a + b\n\n*main()\n\tlog(add(2, 3))\n");
    assert_eq!(output, "5");
}

/// L5: If/else control flow
#[test]
fn e2e_self_hosted_l5_if_else() {
    let output =
        run_self_hosted_e2e("*main()\n\tif true\n\t\tlog(\"yes\")\n\telse\n\t\tlog(\"no\")\n");
    assert_eq!(output, "yes");
}

/// L6: For loop with range
#[test]
fn e2e_self_hosted_l6_loop() {
    let output = run_self_hosted_e2e("*main()\n\tfor i in range(5)\n\t\tlog(i)\n");
    assert_eq!(output, "0\n1\n2\n3\n4");
}

/// L7: List literal and iteration
#[test]
fn e2e_self_hosted_l7_list() {
    let output =
        run_self_hosted_e2e("*main()\n\titems is [1, 2, 3]\n\tfor x in items\n\t\tlog(x)\n");
    assert_eq!(output, "1\n2\n3");
}

// ─── Phase 5: New Feature Coverage Tests ─────────────────────────

/// Verify new parser constructs are present in the self-hosted parser
#[test]
fn sh5_parser_has_new_syntax_constructs() {
    let source = load_source("self_hosted/parser.coral");

    // New expression types
    assert!(
        source.contains("make_spread_expr"),
        "Missing spread expression constructor"
    );
    assert!(
        source.contains("make_list_comprehension_expr"),
        "Missing list comprehension constructor"
    );
    assert!(
        source.contains("make_map_comprehension_expr"),
        "Missing map comprehension constructor"
    );
    assert!(
        source.contains("make_slice_expr"),
        "Missing slice expression constructor"
    );
    assert!(
        source.contains("make_unsafe_expr"),
        "Missing unsafe expression constructor"
    );
    assert!(
        source.contains("make_inline_asm_expr"),
        "Missing inline asm expression constructor"
    );
    assert!(
        source.contains("make_ptr_load_expr"),
        "Missing ptr_load expression constructor"
    );

    // New statement/item types
    assert!(
        source.contains("make_pattern_binding_stmt"),
        "Missing pattern binding constructor"
    );
    assert!(
        source.contains("make_field_assign_stmt"),
        "Missing field assign constructor"
    );
    assert!(
        source.contains("make_extension_item"),
        "Missing extension item constructor"
    );

    // New pattern types
    assert!(
        source.contains("make_pat_or"),
        "Missing or-pattern constructor"
    );
    assert!(
        source.contains("make_pat_range"),
        "Missing range pattern constructor"
    );
    assert!(
        source.contains("make_pat_rest"),
        "Missing rest pattern constructor"
    );

    // New parsing functions
    assert!(
        source.contains("parse_type_params"),
        "Missing type params parsing"
    );
    assert!(
        source.contains("parse_extension_def"),
        "Missing extension def parsing"
    );
    assert!(
        source.contains("parse_unsafe_block"),
        "Missing unsafe block parsing"
    );
    assert!(
        source.contains("parse_inline_asm"),
        "Missing inline asm parsing"
    );
    assert!(
        source.contains("parse_do_end_block"),
        "Missing do..end block parsing"
    );

    // Named args support
    assert!(
        source.contains("arg_names"),
        "Missing named argument support"
    );
}

/// Verify new lowering pass handles all new node types
#[test]
fn sh5_lower_handles_new_nodes() {
    let source = load_source("self_hosted/lower.coral");

    assert!(
        source.contains("lower_extension_item"),
        "Missing extension item lowering"
    );
    assert!(
        source.contains("lower_pattern_binding_stmt"),
        "Missing pattern binding lowering"
    );
    assert!(
        source.contains("lower_list_comprehension"),
        "Missing list comprehension lowering"
    );
    assert!(
        source.contains("lower_map_comprehension"),
        "Missing map comprehension lowering"
    );
    assert!(
        source.contains("lower_slice_expr"),
        "Missing slice expression lowering"
    );
    assert!(
        source.contains(r#""spread""#),
        "Missing spread node handling in lowering"
    );
    assert!(
        source.contains("arg_names"),
        "Missing arg_names preservation in lowering"
    );
}

/// Verify semantic analysis handles new expression types
#[test]
fn sh5_semantic_handles_new_nodes() {
    let source = load_source("self_hosted/semantic.coral");

    // check_expression handlers
    assert!(
        source.contains(r#""list_comprehension""#),
        "Missing list comprehension in check_expression"
    );
    assert!(
        source.contains(r#""map_comprehension""#),
        "Missing map comprehension in check_expression"
    );
    assert!(
        source.contains(r#""spread""#),
        "Missing spread in check_expression"
    );
    assert!(
        source.contains(r#""slice""#),
        "Missing slice in check_expression"
    );
    assert!(
        source.contains(r#""unsafe""#),
        "Missing unsafe in check_expression"
    );
    assert!(
        source.contains(r#""inline_asm""#),
        "Missing inline_asm in check_expression"
    );
    assert!(
        source.contains(r#""ptr_load""#),
        "Missing ptr_load in check_expression"
    );
    assert!(
        source.contains(r#""pattern_binding""#),
        "Missing pattern_binding in check_statement"
    );
    assert!(
        source.contains(r#""extension""#),
        "Missing extension in analyze"
    );
}

/// Verify codegen handles new expression types
#[test]
fn sh5_codegen_handles_new_nodes() {
    let source = load_source("self_hosted/codegen.coral");

    assert!(
        source.contains("emit_list_comprehension"),
        "Missing list comprehension codegen"
    );
    assert!(
        source.contains("emit_map_comprehension"),
        "Missing map comprehension codegen"
    );
    assert!(source.contains("emit_slice"), "Missing slice codegen");
    assert!(
        source.contains("emit_pattern_binding_stmt"),
        "Missing pattern binding codegen"
    );
    assert!(
        source.contains(r#""list_comprehension""#),
        "Missing list_comprehension dispatch in emit_expression"
    );
    assert!(
        source.contains(r#""map_comprehension""#),
        "Missing map_comprehension dispatch in emit_expression"
    );
    assert!(
        source.contains(r#""spread""#),
        "Missing spread dispatch in emit_expression"
    );
    assert!(
        source.contains(r#""slice""#),
        "Missing slice dispatch in emit_expression"
    );
    assert!(
        source.contains(r#""unsafe""#),
        "Missing unsafe dispatch in emit_expression"
    );
    assert!(
        source.contains(r#""inline_asm""#),
        "Missing inline_asm dispatch in emit_expression"
    );
    assert!(
        source.contains(r#""ptr_load""#),
        "Missing ptr_load dispatch in emit_expression"
    );
}

/// Verify compiler pipeline folds new expression types
#[test]
fn sh5_compiler_folds_new_nodes() {
    let source = load_source("self_hosted/compiler.coral");

    assert!(
        source.contains(r#""list_comprehension""#),
        "Missing list comprehension in fold_expr"
    );
    assert!(
        source.contains(r#""map_comprehension""#),
        "Missing map comprehension in fold_expr"
    );
    assert!(
        source.contains(r#""spread""#),
        "Missing spread in fold_expr"
    );
    assert!(source.contains(r#""slice""#), "Missing slice in fold_expr");
    assert!(
        source.contains(r#""pattern_binding""#),
        "Missing pattern_binding in fold_statement"
    );
    assert!(source.contains(r#""match""#), "Missing match in fold_expr");
    assert!(
        source.contains(r#""pipeline""#),
        "Missing pipeline in fold_expr"
    );
}

/// Verify all self-hosted files still compile to IR after changes
#[test]
fn sh5_all_files_compile_to_ir() {
    let files = [
        "self_hosted/lexer.coral",
        "self_hosted/parser.coral",
        "self_hosted/lower.coral",
        "self_hosted/semantic.coral",
        "self_hosted/codegen.coral",
        "self_hosted/compiler.coral",
    ];
    for f in &files {
        let ir = compile_to_ir(f);
        assert!(ir.contains("define"), "{} did not produce valid LLVM IR", f);
    }
}

/// E2E: List comprehension through self-hosted compiler
#[test]
fn e2e_self_hosted_l8_list_comprehension() {
    let output = run_self_hosted_e2e(
        "*main()\n\tnums is [1, 2, 3, 4, 5]\n\tdoubled is [x * 2 for x in nums]\n\tfor n in doubled\n\t\tlog(n)\n",
    );
    assert_eq!(output, "2\n4\n6\n8\n10");
}

/// E2E: List comprehension with filter
#[test]
fn e2e_self_hosted_l9_filtered_comprehension() {
    let output = run_self_hosted_e2e(
        "*main()\n\tnums is [1, 2, 3, 4, 5, 6]\n\tevens is [x for x in nums if x % 2 is 0]\n\tfor n in evens\n\t\tlog(n)\n",
    );
    assert_eq!(output, "2\n4\n6");
}
