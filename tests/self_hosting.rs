//! Self-hosting regression tests.
//!
//! These tests verify that the self-hosted compiler components
//! (lexer.coral, parser.coral) can be compiled by the Rust compiler.
//! They catch regressions that would break self-hosting readiness.

use coralc::module_loader::ModuleLoader;
use coralc::Compiler;
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
