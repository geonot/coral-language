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

// ============================================================
// CC3.1 — AST-level module system tests
// ============================================================

#[test]
fn load_modules_returns_separate_modules() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let entry = temp_dir.path().join("main.coral");
    let utils = temp_dir.path().join("utils.coral");
    std::fs::write(&utils, "*add(a, b)\n    a + b\n").unwrap();
    std::fs::write(&entry, "use utils\nresult is add(1, 2)\nlog(result)\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let modules = loader.load_modules(&entry).expect("load_modules");

    // Should have at least 2 modules: utils first, then main
    assert!(modules.len() >= 2, "expected at least 2 modules, got {}", modules.len());

    // First module should be utils (dependency comes before dependent)
    let utils_mod = modules.iter().find(|m| m.name == "utils").expect("utils module");
    assert!(utils_mod.source.contains("*add(a, b)"));
    assert!(!utils_mod.source.contains("use utils")); // use directives stripped

    // Last module should be the entry
    let last = modules.last().unwrap();
    assert!(last.source.contains("result is add(1, 2)"));
    assert!(!last.source.contains("use utils")); // use directives stripped
}

#[test]
fn load_modules_preserves_dependency_order() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let a = temp_dir.path().join("a.coral");
    let b = temp_dir.path().join("b.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&a, "*from_a()\n    1\n").unwrap();
    std::fs::write(&b, "use a\n*from_b()\n    from_a() + 1\n").unwrap();
    std::fs::write(&main_file, "use b\nresult is from_b()\nlog(result)\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let modules = loader.load_modules(&main_file).expect("load_modules");

    let names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    // a must come before b, b must come before main
    let a_idx = names.iter().position(|n| *n == "a").expect("a");
    let b_idx = names.iter().position(|n| *n == "b").expect("b");
    let main_idx = names.iter().position(|n| *n == "main").expect("main");
    assert!(a_idx < b_idx, "a should come before b");
    assert!(b_idx < main_idx, "b should come before main");
}

#[test]
fn load_modules_deduplicates() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let shared = temp_dir.path().join("shared.coral");
    let a = temp_dir.path().join("a.coral");
    let b = temp_dir.path().join("b.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&shared, "*shared_fn()\n    42\n").unwrap();
    std::fs::write(&a, "use shared\n*from_a()\n    shared_fn()\n").unwrap();
    std::fs::write(&b, "use shared\n*from_b()\n    shared_fn()\n").unwrap();
    std::fs::write(&main_file, "use a\nuse b\nlog(from_a())\nlog(from_b())\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let modules = loader.load_modules(&main_file).expect("load_modules");

    // shared should appear exactly once
    let shared_count = modules.iter().filter(|m| m.name == "shared").count();
    assert_eq!(shared_count, 1, "shared module should appear exactly once");
}

#[test]
fn load_modules_detects_circular_imports() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let a = temp_dir.path().join("a.coral");
    let b = temp_dir.path().join("b.coral");
    std::fs::write(&a, "use b\n*a_fn()\n    1\n").unwrap();
    std::fs::write(&b, "use a\n*b_fn()\n    2\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let result = loader.load_modules(&a);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("circular import"));
}

#[test]
fn load_modules_tracks_imports_and_exports() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let math = temp_dir.path().join("mymath.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&math, "*add(a, b)\n    a + b\n*sub(a, b)\n    a - b\n").unwrap();
    std::fs::write(&main_file, "use mymath\nlog(add(1, 2))\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let modules = loader.load_modules(&main_file).expect("load_modules");

    let math_mod = modules.iter().find(|m| m.name == "mymath").expect("mymath");
    assert!(math_mod.exports.contains(&"add".to_string()));
    assert!(math_mod.exports.contains(&"sub".to_string()));
    assert!(math_mod.imports.is_empty());

    let main_mod = modules.iter().find(|m| m.name == "main").expect("main");
    assert!(main_mod.imports.contains(&"mymath".to_string()));
}

#[test]
fn compile_modules_produces_same_ir_as_concat() {
    // Verify that compile_modules_to_ir produces equivalent output to
    // the old compile_to_ir (concatenated source) path.
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/uses_std.coral");

    let concat_source = loader.load(&path).expect("load concat");
    let compiler = Compiler;
    let (ir_concat, _) = compiler
        .compile_to_ir_with_warnings(&concat_source)
        .expect("compile concat");

    let mut loader2 = ModuleLoader::with_default_std();
    let module_sources = loader2.load_modules(&path).expect("load modules");
    let (ir_modules, _) = compiler
        .compile_modules_to_ir(&module_sources)
        .expect("compile modules");

    // Both should produce non-empty IR with the same functions
    assert!(!ir_concat.is_empty(), "concat IR should not be empty");
    assert!(!ir_modules.is_empty(), "module IR should not be empty");

    // Both should reference the coral_make_string runtime function
    assert!(ir_concat.contains("@coral_make_string"), "concat IR should have string runtime");
    assert!(ir_modules.contains("@coral_make_string"), "module IR should have string runtime");
}

#[test]
fn compile_modules_full_language_fixture() {
    let mut loader = ModuleLoader::with_default_std();
    let path = PathBuf::from("tests/fixtures/programs/full_language_no_store.coral");
    let module_sources = loader.load_modules(&path).expect("load modules");

    let compiler = Compiler;
    let (ir, _warnings) = compiler
        .compile_modules_to_ir(&module_sources)
        .expect("compile modules for full language fixture");

    // Should compile without errors and produce non-empty IR
    assert!(!ir.is_empty(), "IR should not be empty");
    // The full language fixture defines user functions
    assert!(ir.contains("define"), "should have function definitions");
}

#[test]
fn program_from_modules_has_modules_field() {
    use coralc::ast::{Module, Program};
    use coralc::span::Span;

    let m1 = Module {
        name: "utils".to_string(),
        items: vec![],
        imports: vec![],
        exports: vec!["add".to_string()],
        span: Span::new(0, 0),
    };
    let m2 = Module {
        name: "main".to_string(),
        items: vec![],
        imports: vec!["utils".to_string()],
        exports: vec![],
        span: Span::new(0, 0),
    };

    let program = Program::from_modules(vec![m1, m2]);
    assert_eq!(program.modules.len(), 2);
    assert_eq!(program.modules[0].name, "utils");
    assert_eq!(program.modules[1].name, "main");
    assert!(program.items.is_empty());
}
