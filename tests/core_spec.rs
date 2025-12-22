use coralc::module_loader::ModuleLoader;
use coralc::Compiler;
use std::path::PathBuf;

#[test]
fn core_examples_compile() {
    let loader = ModuleLoader::with_default_std();
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
