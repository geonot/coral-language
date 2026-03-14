use coralc::Compiler;
use coralc::module_loader::ModuleLoader;
use std::path::PathBuf;

#[test]
fn core_examples_compile() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/core_examples.coral");
    let source = loader.load(&path).expect("load core program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile Coral core sample");
    assert!(ir.contains("@coral_value_add"));
    assert!(ir.contains("@coral_value_bitand"));
    assert!(ir.contains("@coral_make_bytes"));
}

#[test]
fn compiles_store_with_reference_fields() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/store_reference.coral");
    let source = loader.load(&path).expect("load store_reference program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile store with reference fields");

    // Verify constructor creates map with null, 0 args
    assert!(ir.contains("@coral_make_map(ptr null, i64 0)"));

    // Verify reference field has retain/release
    assert!(ir.contains("@coral_value_retain"));
    assert!(ir.contains("@coral_value_release"));

    // Verify method signatures take i64 parameters and return i64 (NaN-boxed)
    assert!(ir.contains("define i64 @Node_set_next(i64 %0, i64 %1)"));
}

#[test]
fn compiles_index_subscript_syntax() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/index_syntax.coral");
    let source = loader.load(&path).expect("load index_syntax program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile index syntax");

    // Verify subscript desugars to coral_list_get calls
    assert!(ir.contains("@coral_list_get"));
}
