use coralc::module_loader::ModuleLoader;
use coralc::Compiler;
use std::path::PathBuf;

#[test]
fn compiles_program_using_std_modules() {
    let loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std.coral");
    let source = loader.load(&path).expect("load module graph");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile module-enabled program");
    assert!(ir.contains("@coral_make_string"));
}

#[test]
fn compiles_program_using_std_io() {
    let loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std_io.coral");
    let source = loader.load(&path).expect("load module graph");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile io-enabled program");
    assert!(ir.contains("coral_fs_exists"));
}

#[test]
fn compiles_actor_program_and_wires_runtime_calls() {
    let loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/actor_simple.coral");
    let source = loader.load(&path).expect("load actor program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile actor program");

    assert!(
        ir.contains("@__coral_main_handler"),
        "entrypoint should build a main actor handler",
    );
    assert!(
        ir.contains("@coral_actor_spawn"),
        "actor spawn should be emitted for main and constructors",
    );
    assert!(
        ir.contains("@coral_actor_send"),
        "main should send initial message to its handler",
    );
    assert!(ir.contains("@make_Counter"), "actor constructor should be generated");
    assert!(ir.contains("@__Counter_handler"), "actor handler stub should be generated");
}

#[test]
fn compiles_program_using_runtime_memory() {
    let loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_runtime_memory.coral");
    let source = loader.load(&path).expect("load runtime memory program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile runtime memory program");

    assert!(ir.contains("@coral_malloc"));
    assert!(ir.contains("@coral_store_u64"));
    assert!(ir.contains("@coral_load_u64"));
}

#[test]
fn compiles_program_using_runtime_value() {
    let loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_runtime_value.coral");
    let source = loader.load(&path).expect("load runtime value program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile runtime value program");

    assert!(ir.contains("@coral_make_number"));
    assert!(ir.contains("@coral_make_bool"));
    assert!(ir.contains("@coral_value_retain"));
    assert!(ir.contains("@coral_value_release"));
}

#[test]
fn compiles_program_using_runtime_actor() {
    let loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_runtime_actor.coral");
    let source = loader.load(&path).expect("load runtime actor program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile runtime actor program");

    assert!(ir.contains("@coral_actor_spawn"));
    assert!(ir.contains("@coral_actor_send"));
    assert!(ir.contains("@coral_actor_stop"));
    assert!(ir.contains("@coral_actor_self"));
}
