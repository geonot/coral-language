//! C4.5 Profile-Guided Optimization tests.
//!
//! Tests that the PGO instrumentation and optimization pipelines
//! correctly transform LLVM IR.

use coralc::Compiler;
use coralc::compiler::{LtoOptLevel, instrument_for_pgo};
use coralc::module_loader::ModuleSource;

/// Helper: compile a simple Coral program to IR.
fn compile_to_ir(source: &str) -> String {
    let compiler = Compiler;
    let sources = vec![ModuleSource {
        name: "main".to_string(),
        path: std::path::PathBuf::from("test.coral"),
        source: source.to_string(),
        import_directives: vec![],
        imports: vec![],
        exports: vec![],
    }];
    let (ir, _warnings) = compiler.compile_modules_to_ir(&sources).unwrap();
    ir
}

#[test]
fn c45_pgo_instrumentation_inserts_profiling() {
    // Compile a simple program and instrument it for PGO.
    let ir = compile_to_ir(
        r#"
*main()
    x is 42
    y is x + 1
    y
"#,
    );

    let instrumented = instrument_for_pgo(&ir).expect("PGO instrumentation should succeed");

    // Instrumented IR should contain profiling intrinsics or global counters.
    // LLVM's pgo-instr-gen pass inserts calls to llvm.instrprof.increment
    // and/or __llvm_profile globals.
    let has_prof = instrumented.contains("llvm.instrprof")
        || instrumented.contains("__llvm_profile")
        || instrumented.contains("__profc_")
        || instrumented.contains("__profd_");
    assert!(
        has_prof,
        "instrumented IR should contain profiling intrinsics, got:\n{}",
        &instrumented[..instrumented.len().min(2000)]
    );
}

#[test]
fn c45_pgo_instrumentation_preserves_correctness() {
    // The instrumented IR should still define the same functions.
    let ir = compile_to_ir(
        r#"
*add(a, b)
    a + b

*main()
    result is add(10, 20)
    result
"#,
    );

    let instrumented = instrument_for_pgo(&ir).expect("PGO instrumentation should succeed");

    // Original functions should still be present
    assert!(
        instrumented.contains("define"),
        "instrumented IR should still have function definitions"
    );
}

#[test]
fn c45_pgo_use_with_nonexistent_profile_gracefully_handles() {
    // When --pgo-use is given a non-existent profile, the pass should either
    // succeed (LLVM treats missing profile as all-zero weights) or return an error.
    // Either behavior is acceptable — we just verify no panic.
    let ir = compile_to_ir(
        r#"
*main()
    x is 1 + 2
    x
"#,
    );

    let result = coralc::compiler::optimize_with_profile(
        &ir,
        "/nonexistent/path/profile.profdata",
        LtoOptLevel::O2,
    );

    // LLVM may succeed with zero-weight profile or may fail — both are fine.
    // The key is no panic/crash.
    match result {
        Ok(optimized) => {
            assert!(
                optimized.contains("define"),
                "optimized IR should have function definitions"
            );
        }
        Err(e) => {
            // Expected: LLVM reports the profile file is missing
            assert!(
                e.contains("profile") || e.contains("failed") || e.contains("error"),
                "error should mention profile issue: {}",
                e
            );
        }
    }
}
