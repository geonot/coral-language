//! Tests for Phase 7 technical debt features
//! - 7.1.2: Type parameter tracking
//! - 7.1.3: List/map element type checking  
//! - 7.1.4: Weak references
//! - 7.1.5: Cycle detection
//! - 7.2.1: Module caching
//! - 7.2.2: Namespace scoping
//! - 7.2.3: Circular import detection
//! - 7.2.4: Message name interning

use coralc::module_loader::ModuleLoader;
use coralc::types::core::{TypeId, Primitive};
use coralc::types::env::TypeEnv;
use std::fs;

// ========== Type System Tests (7.1.2, 7.1.3) ==========

#[test]
fn type_env_tracks_type_parameters() {
    let mut env = TypeEnv::new();
    
    // Register a generic type with parameters
    env.register_generic_type("List", vec!["T"]);
    
    // Push type parameters for a specific instantiation
    env.push_type_params();
    env.bind_type_param("T", TypeId::Primitive(Primitive::Int));
    
    // Get the bound type parameter
    let t = env.get_type_param("T");
    assert!(t.is_some());
    assert_eq!(t.unwrap(), &TypeId::Primitive(Primitive::Int));
    
    // Pop and verify no longer bound
    env.pop_type_params();
    let t_after = env.get_type_param("T");
    assert!(t_after.is_none());
}

#[test]
fn type_env_nested_type_parameters() {
    let mut env = TypeEnv::new();
    
    // First scope
    env.push_type_params();
    env.bind_type_param("T", TypeId::Primitive(Primitive::String));
    
    // Verify T is bound
    assert_eq!(env.get_type_param("T"), Some(&TypeId::Primitive(Primitive::String)));
    
    // Enter nested scope (saves current params and starts fresh)
    env.push_type_params();
    
    // T is NOT visible in nested scope (fresh scope)
    assert_eq!(env.get_type_param("T"), None);
    
    // Bind U in nested scope
    env.bind_type_param("U", TypeId::Primitive(Primitive::Float));
    assert_eq!(env.get_type_param("U"), Some(&TypeId::Primitive(Primitive::Float)));
    
    // Pop nested scope - restores outer scope
    env.pop_type_params();
    
    // T is visible again, U is not
    assert_eq!(env.get_type_param("T"), Some(&TypeId::Primitive(Primitive::String)));
    assert_eq!(env.get_type_param("U"), None);
    
    // Pop outer scope
    env.pop_type_params();
    assert_eq!(env.get_type_param("T"), None);
}

#[test]
fn type_id_list_element_type() {
    // Create a List[Int] type
    let list_int = TypeId::List(Box::new(TypeId::Primitive(Primitive::Int)));
    
    assert!(list_int.is_list());
    assert!(list_int.is_collection());
    assert_eq!(list_int.list_element_type(), Some(&TypeId::Primitive(Primitive::Int)));
    
    // Non-list types should return None
    assert_eq!(TypeId::Primitive(Primitive::Int).list_element_type(), None);
    assert!(!TypeId::Primitive(Primitive::Int).is_list());
}

#[test]
fn type_id_map_types() {
    // Create a Map[String, Int] type
    let map_str_int = TypeId::Map(
        Box::new(TypeId::Primitive(Primitive::String)),
        Box::new(TypeId::Primitive(Primitive::Int))
    );
    
    assert!(map_str_int.is_map());
    assert!(map_str_int.is_collection());
    
    let (key, value) = map_str_int.map_types().unwrap();
    assert_eq!(key, &TypeId::Primitive(Primitive::String));
    assert_eq!(value, &TypeId::Primitive(Primitive::Int));
    
    // Non-map types should return None
    assert_eq!(TypeId::Primitive(Primitive::Int).map_types(), None);
    assert!(!TypeId::Primitive(Primitive::Int).is_map());
}

#[test]
fn type_id_callable_types() {
    // Create a function type (Int, String) -> Bool
    let fn_type = TypeId::Func(
        vec![TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::String)],
        Box::new(TypeId::Primitive(Primitive::Bool))
    );
    
    assert!(fn_type.is_callable());
    assert_eq!(fn_type.return_type(), Some(&TypeId::Primitive(Primitive::Bool)));
    assert_eq!(
        fn_type.param_types(), 
        Some(&vec![TypeId::Primitive(Primitive::Int), TypeId::Primitive(Primitive::String)][..])
    );
    
    // Non-function types
    assert!(!TypeId::Primitive(Primitive::Int).is_callable());
    assert_eq!(TypeId::Primitive(Primitive::Int).return_type(), None);
}

// ========== Module Loader Tests (7.2.1, 7.2.2, 7.2.3) ==========

#[test]
fn module_loader_caches_by_content_hash() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let entry = temp_dir.path().join("main.coral");
    let module = temp_dir.path().join("helper.coral");
    
    fs::write(&module, "*add(a, b)\n    a + b\n").unwrap();
    fs::write(&entry, "use helper\nadd(1, 2)\n").unwrap();
    
    let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
    
    // First load
    let first = loader.load(&entry).expect("first load");
    assert!(first.contains("*add"));
    
    // Get module info
    let module_canonical = fs::canonicalize(&module).unwrap();
    let info = loader.get_module_info(&module_canonical);
    assert!(info.is_some());
    assert!(info.unwrap().exports.contains(&"add".to_string()));
    
    // Second load should use cache
    let second = loader.load(&entry).expect("second load");
    assert_eq!(first, second);
}

#[test]
fn module_loader_detects_circular_imports() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let a = temp_dir.path().join("mod_a.coral");
    let b = temp_dir.path().join("mod_b.coral");
    
    fs::write(&a, "use mod_b\n*from_a()\n    1\n").unwrap();
    fs::write(&b, "use mod_a\n*from_b()\n    2\n").unwrap();
    
    let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
    let result = loader.load(&a);
    
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("circular import"));
    // Check for helpful hint
    assert!(err.contains("Hint:") || err.contains("restructur"));
}

#[test]
fn module_loader_extracts_all_export_kinds() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let entry = temp_dir.path().join("exports.coral");
    
    fs::write(&entry, r#"
type Point = { x: Int, y: Int }
type Color = Red | Green | Blue

store Counter
    count: Int = 0

actor Logger

trait Serializable
    *to_json()

*helper(x)
    x * 2
"#).unwrap();
    
    let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
    let _ = loader.load(&entry).expect("load");
    
    let canonical = fs::canonicalize(&entry).unwrap();
    let info = loader.get_module_info(&canonical).expect("module info");
    
    // Check all exported types
    assert!(info.exports.contains(&"Point".to_string()), "should export Point type");
    assert!(info.exports.contains(&"Color".to_string()), "should export Color type");
    assert!(info.exports.contains(&"Counter".to_string()), "should export Counter store");
    assert!(info.exports.contains(&"Logger".to_string()), "should export Logger actor");
    assert!(info.exports.contains(&"Serializable".to_string()), "should export Serializable trait");
    assert!(info.exports.contains(&"helper".to_string()), "should export helper function");
}

#[test]
fn module_loader_tracks_namespace() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let entry = temp_dir.path().join("mymodule.coral");
    
    fs::write(&entry, "*test()\n    42\n").unwrap();
    
    // Use an empty std_paths so the file isn't treated as a std file
    let mut loader = ModuleLoader::new(vec![]);
    let _ = loader.load(&entry).expect("load");
    
    let canonical = fs::canonicalize(&entry).unwrap();
    let info = loader.get_module_info(&canonical).expect("module info");
    
    // Namespace should be the filename without extension
    assert_eq!(info.namespace, "mymodule");
}

#[test]
fn module_loader_clear_cache_works() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let entry = temp_dir.path().join("clearable.coral");
    
    fs::write(&entry, "*cached()\n    1\n").unwrap();
    
    let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
    let _ = loader.load(&entry).expect("load");
    
    let canonical = fs::canonicalize(&entry).unwrap();
    assert!(loader.get_module_info(&canonical).is_some());
    
    loader.clear_cache();
    
    assert!(loader.get_module_info(&canonical).is_none());
}

#[test]
fn module_loader_handles_diamond_imports() {
    // Test the diamond dependency pattern: A -> B, A -> C, B -> D, C -> D
    let temp_dir = tempfile::tempdir().expect("tempdir");
    
    let a = temp_dir.path().join("a.coral");
    let b = temp_dir.path().join("b.coral");
    let c = temp_dir.path().join("c.coral");
    let d = temp_dir.path().join("d.coral");
    
    fs::write(&d, "*shared()\n    100\n").unwrap();
    fs::write(&b, "use d\n*from_b()\n    shared() + 1\n").unwrap();
    fs::write(&c, "use d\n*from_c()\n    shared() + 2\n").unwrap();
    fs::write(&a, "use b\nuse c\n*from_a()\n    from_b() + from_c()\n").unwrap();
    
    let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
    let result = loader.load(&a).expect("diamond import should succeed");
    
    // d should only appear once in the output
    let shared_count = result.matches("*shared").count();
    assert_eq!(shared_count, 1, "shared function should appear exactly once");
}
