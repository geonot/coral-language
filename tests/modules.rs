use coralc::module_loader::ModuleLoader;
use coralc::Compiler;
use std::path::PathBuf;

#[test]
fn compiles_program_using_std_modules() {
    let mut loader = ModuleLoader::with_default_std();
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
    let mut loader = ModuleLoader::with_default_std();
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
    let mut loader = ModuleLoader::with_default_std();
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
    assert!(ir.contains("@__Counter_handler_invoke"), "actor handler invoke should be generated");
}

#[test]
fn compiles_actor_with_state_and_self_access() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/actor_state.coral");
    let source = loader.load(&path).expect("load actor state program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile actor state program");

    // Actor constructor should create state map
    assert!(ir.contains("@coral_make_map"), "actor should create state map");
    // State should have field initialized
    assert!(ir.contains("@coral_map_set"), "actor should initialize fields in state");
    // self.field access should use map_get
    assert!(ir.contains("@coral_map_get"), "self.field should emit map_get");
    // Handler should pass state to methods (now returns i64 NaN-boxed values)
    assert!(ir.contains("define i64 @Greeter_say_hello(i64"), 
        "message handlers should take state i64 as first param");
    // Handler invoke should be generated
    assert!(ir.contains("@__Greeter_handler_invoke"), "actor handler invoke should be generated");
    // Handler release should be generated
    assert!(ir.contains("@__Greeter_handler_release"), "actor handler release should be generated");
}

#[test]
fn compiles_actor_handler_with_param() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/actor_with_param.coral");
    let source = loader.load(&path).expect("load actor with param program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile actor with param program");

    // Handler with 1 param should take state i64 + i64 (NaN-boxed values)
    assert!(ir.contains("define i64 @Counter_set_value(i64 %0, i64 %1)"), 
        "handler with param should take state i64 and param");
    // Dispatch no longer needs to convert msg_data to number (passed as NaN-boxed now)
    // Handler with no params should only take state
    assert!(ir.contains("define i64 @Counter_ping(i64 %0)"),
        "handler with no params should only take state i64");
}

#[test]
fn compiles_program_using_runtime_memory() {
    let mut loader = ModuleLoader::with_default_std();
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
    let mut loader = ModuleLoader::with_default_std();
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
    let mut loader = ModuleLoader::with_default_std();
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

#[test]
fn compiles_store_with_fields() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/store_simple.coral");
    let source = loader.load(&path).expect("load store program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile store program");

    // Store constructor should create a Map with __type__ and fields
    assert!(ir.contains("@make_Point"), "constructor should be generated");
    assert!(ir.contains("@coral_make_map"), "store should use Map internally");
    assert!(ir.contains("@coral_map_set"), "fields should be set in constructor");
    // Field access should use map_get
    assert!(ir.contains("@coral_map_get"), "field access should use map_get");
}

#[test]
fn compiles_store_with_methods() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/store_method.coral");
    let source = loader.load(&path).expect("load store method program");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile store method program");

    // Store methods should be generated
    assert!(ir.contains("@Counter_increment"), "increment method should be generated");
    assert!(ir.contains("@Counter_add"), "add method should be generated");
    // Method calls should include self parameter
    assert!(ir.contains("store_method_call"), "method calls should go through store dispatch");
    // self.field assignment should use map_set
    assert!(ir.contains("@coral_map_set"), "self.field assignment should use map_set");
}

#[test]
fn compiles_program_using_std_string() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std_string.coral");
    let source = loader.load(&path).expect("load string module");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile string program");
    assert!(ir.contains("@coral_string_to_upper"), "should call to_upper");
}

#[test]
fn compiles_program_using_std_list() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std_list.coral");
    let source = loader.load(&path).expect("load list module");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile list program");
    assert!(ir.contains("@coral_list_map"), "should call list_map");
}

#[test]
fn compiles_program_using_std_map() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std_map.coral");
    let source = loader.load(&path).expect("load map module");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile map program");
    assert!(ir.contains("@coral_map_get"), "should call map_get");
}

#[test]
fn compiles_program_using_std_set() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std_set.coral");
    let source = loader.load(&path).expect("load set module");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile set program");
    assert!(ir.contains("@coral_map_set"), "should use map_set for set operations");
}

#[test]
fn compiles_program_using_std_math() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std_math.coral");
    let source = loader.load(&path).expect("load math module");

    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(&source)
        .expect("failed to compile math program");
    // distance function uses sqrt internally
    assert!(ir.contains("@coral_math_sqrt"), "distance should call sqrt");
}
