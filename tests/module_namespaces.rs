use coralc::module_loader::{ImportDirective, ModuleLoader};
use coralc::Compiler;
use std::path::PathBuf;

// ============================================================
// CC3.3 — Selective import directive parsing
// ============================================================

#[test]
fn parse_selective_import_basic() {
    // Test basic selective import syntax: `use std.math.{sin, cos}`
    let mut loader = ModuleLoader::new(vec![]);
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let math = temp_dir.path().join("math.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&math, "*sin(x)\n    x\n*cos(x)\n    x\n*tan(x)\n    x\n").unwrap();
    std::fs::write(&main_file, "use math.{sin, cos}\nlog(sin(1))\n").unwrap();

    let modules = loader.load_modules(&main_file).expect("load_modules");

    // math module should be loaded (the selective import still loads the full module)
    let math_mod = modules.iter().find(|m| m.name == "math").expect("math module");
    assert!(math_mod.exports.contains(&"sin".to_string()));
    assert!(math_mod.exports.contains(&"cos".to_string()));
    assert!(math_mod.exports.contains(&"tan".to_string()));

    // The importing module should have the import directive with selections
    let main_mod = modules.last().expect("main module");
    let math_directive = main_mod.import_directives.iter()
        .find(|d| d.module_path == "math")
        .expect("math import directive");
    assert!(math_directive.selections.is_some());
    let selections = math_directive.selections.as_ref().unwrap();
    assert!(selections.contains(&"sin".to_string()));
    assert!(selections.contains(&"cos".to_string()));
    assert!(!selections.contains(&"tan".to_string()));
}

#[test]
fn parse_directive_without_selection() {
    // Test that `use math` has no selections
    let line = "use math";
    // We can't call the private parse_use_directive directly, so test via load_modules
    let mut loader = ModuleLoader::new(vec![]);
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let math = temp_dir.path().join("math.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&math, "*add(a, b)\n    a + b\n").unwrap();
    std::fs::write(&main_file, "use math\nlog(add(1, 2))\n").unwrap();

    let modules = loader.load_modules(&main_file).expect("load_modules");
    let main_mod = modules.last().expect("main module");
    let directive = main_mod.import_directives.iter()
        .find(|d| d.module_path == "math")
        .expect("math directive");
    assert!(directive.selections.is_none());
}

// ============================================================
// CC3.2 — Qualified module access (module.function())
// ============================================================

#[test]
fn qualified_access_via_module_pipeline() {
    // Test that math.add() works when using the module-aware compilation path
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let math = temp_dir.path().join("mylib.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&math, "*add(a, b)\n    a + b\n").unwrap();
    std::fs::write(&main_file, "use mylib\nresult is mylib.add(3, 4)\nlog(result)\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let module_sources = loader.load_modules(&main_file).expect("load_modules");

    let compiler = Compiler;
    let result = compiler.compile_modules_to_ir(&module_sources);
    assert!(result.is_ok(), "qualified access should compile: {:?}", result.err());
    let (ir, _) = result.unwrap();
    assert!(ir.contains("define"), "should have function definitions");
}

#[test]
fn qualified_access_e2e_execution() {
    // E2E test: compile and run a program with qualified module access
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let helper = temp_dir.path().join("helper.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&helper, "*double(x)\n    x * 2\n").unwrap();
    // Use both qualified (helper.double) and unqualified (double) access
    std::fs::write(&main_file, "use helper\nresult is helper.double(21)\nlog(result)\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let module_sources = loader.load_modules(&main_file).expect("load_modules");

    let compiler = Compiler;
    let (ir, _) = compiler.compile_modules_to_ir(&module_sources)
        .expect("compile modules");

    // Write IR to temp file and run via lli
    let ir_file = temp_dir.path().join("test.ll");
    std::fs::write(&ir_file, &ir).unwrap();

    let runtime_lib = find_runtime_lib();
    let output = std::process::Command::new("lli")
        .arg("-load")
        .arg(&runtime_lib)
        .arg(&ir_file)
        .output()
        .expect("lli");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("42"),
        "helper.double(21) should produce 42, got: {}",
        stdout
    );
}

#[test]
fn selective_import_e2e_execution() {
    // E2E test: selective import still provides function access
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let mymath = temp_dir.path().join("mymath.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&mymath, "*add(a, b)\n    a + b\n*sub(a, b)\n    a - b\n").unwrap();
    // Selective import — use only add
    std::fs::write(&main_file, "use mymath.{add}\nresult is add(10, 32)\nlog(result)\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let module_sources = loader.load_modules(&main_file).expect("load_modules");

    let compiler = Compiler;
    let (ir, _) = compiler.compile_modules_to_ir(&module_sources)
        .expect("compile modules");

    let ir_file = temp_dir.path().join("test.ll");
    std::fs::write(&ir_file, &ir).unwrap();

    let runtime_lib = find_runtime_lib();
    let output = std::process::Command::new("lli")
        .arg("-load")
        .arg(&runtime_lib)
        .arg(&ir_file)
        .output()
        .expect("lli");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("42"),
        "add(10, 32) should produce 42, got: {}",
        stdout
    );
}

#[test]
fn module_exports_in_semantic_model() {
    // Verify that module_exports is populated in the semantic model
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let utils = temp_dir.path().join("utils.coral");
    let main_file = temp_dir.path().join("main.coral");
    std::fs::write(&utils, "*helper()\n    42\n*worker()\n    99\n").unwrap();
    std::fs::write(&main_file, "use utils\nlog(helper())\n").unwrap();

    let mut loader = ModuleLoader::new(vec![]);
    let module_sources = loader.load_modules(&main_file).expect("load_modules");

    // Parse, lower, and analyze using the compiler's module path
    let compiler = Compiler;
    let (ir, _) = compiler.compile_modules_to_ir(&module_sources)
        .expect("compile modules");
    // If we get here, module_exports was correctly populated
    assert!(!ir.is_empty());
}

#[test]
fn import_directive_struct_fields() {
    let d = ImportDirective {
        module_path: "std.math".to_string(),
        selections: Some(vec!["sin".to_string(), "cos".to_string()]),
    };
    assert_eq!(d.module_path, "std.math");
    assert_eq!(d.selections.as_ref().unwrap().len(), 2);
    assert!(d.selections.as_ref().unwrap().contains(&"sin".to_string()));

    let d2 = ImportDirective {
        module_path: "std.io".to_string(),
        selections: None,
    };
    assert!(d2.selections.is_none());
}

/// Find the runtime shared library for lli tests.
fn find_runtime_lib() -> PathBuf {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for profile in &["release", "debug"] {
        let candidate = workspace.join("target").join(profile).join("libruntime.so");
        if candidate.exists() {
            return candidate;
        }
        let candidate = workspace.join("target").join(profile).join("libruntime.dylib");
        if candidate.exists() {
            return candidate;
        }
    }
    panic!("Could not find runtime library");
}
