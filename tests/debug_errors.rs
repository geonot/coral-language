use coralc::Compiler;
use coralc::diagnostics::CompileError;
use coralc::module_loader::ModuleLoader;
use std::collections::HashSet;
use std::path::PathBuf;
const WORKSPACE: &str = env!("CARGO_MANIFEST_DIR");
fn load_source(relative_path: &str) -> String {
    let path = PathBuf::from(WORKSPACE).join(relative_path);
    let mut loader = ModuleLoader::with_default_std();
    loader.load(&path).unwrap()
}

fn load_raw(relative_path: &str) -> String {
    std::fs::read_to_string(PathBuf::from(WORKSPACE).join(relative_path)).unwrap()
}

fn try_compile(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    match compiler.compile_to_ir(source) {
        Ok(ir) => Ok(ir),
        Err(e) => Err(format!("{:?}: {}", e.stage, e.diagnostic.message)),
    }
}

fn extract_function_names(source: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('*') && !trimmed.starts_with("**") && trimmed.contains('(') {
            let name = trimmed
                .split('(')
                .next()
                .unwrap_or("")
                .trim_start_matches('*')
                .trim();
            if !name.is_empty() {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn extract_all_identifiers(source: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        // Extract variable bindings: "name is ..."
        if let Some(pos) = trimmed.find(" is ") {
            let lhs = trimmed[..pos].trim();
            if !lhs.contains('(')
                && !lhs.contains('.')
                && !lhs.contains('[')
                && !lhs.starts_with("if")
                && !lhs.starts_with("elif")
                && !lhs.starts_with('#')
                && !lhs.starts_with("return")
                && !lhs.starts_with("*")
            {
                names.insert(lhs.to_string());
            }
        }
        // Extract for loop vars: "for name in ..."
        if trimmed.starts_with("for ") {
            if let Some(var) = trimmed.strip_prefix("for ") {
                if let Some(name) = var.split(' ').next() {
                    names.insert(name.to_string());
                }
            }
        }
        // Extract function params
        if trimmed.starts_with('*') && !trimmed.starts_with("**") && trimmed.contains('(') {
            if let Some(params_part) = trimmed.split('(').nth(1) {
                let params_part = params_part.trim_end_matches(')');
                for p in params_part.split(',') {
                    let param = p.trim();
                    if !param.is_empty() {
                        names.insert(param.to_string());
                    }
                }
            }
        }
    }
    names
}

#[test]
fn find_name_conflicts() {
    let codegen_src = load_raw("self_hosted/codegen.coral");
    let semantic_src = load_raw("self_hosted/semantic.coral");
    let lower_src = load_raw("self_hosted/lower.coral");

    let codegen_fns = extract_function_names(&codegen_src);
    let semantic_fns = extract_function_names(&semantic_src);
    let lower_fns = extract_function_names(&lower_src);

    let codegen_ids = extract_all_identifiers(&codegen_src);
    let semantic_ids = extract_all_identifiers(&semantic_src);
    let lower_ids = extract_all_identifiers(&lower_src);

    // Find: variables in semantic that match function names in codegen
    eprintln!("\n=== Semantic variables that match codegen function names ===");
    let sem_vars: HashSet<_> = semantic_ids.difference(&semantic_fns).cloned().collect();
    for name in sem_vars.intersection(&codegen_fns) {
        eprintln!("  {} (codegen fn, semantic var)", name);
    }

    // Find: variables in codegen that match function names in semantic
    eprintln!("\n=== Codegen variables that match semantic function names ===");
    let cg_vars: HashSet<_> = codegen_ids.difference(&codegen_fns).cloned().collect();
    for name in cg_vars.intersection(&semantic_fns) {
        eprintln!("  {} (semantic fn, codegen var)", name);
    }

    // Find: variables in lower that match function names in codegen
    eprintln!("\n=== Lower variables that match codegen function names ===");
    let lower_vars: HashSet<_> = lower_ids.difference(&lower_fns).cloned().collect();
    for name in lower_vars.intersection(&codegen_fns) {
        eprintln!("  {} (codegen fn, lower var)", name);
    }

    // Find: variables in codegen that match function names in lower
    eprintln!("\n=== Codegen variables that match lower function names ===");
    for name in cg_vars.intersection(&lower_fns) {
        eprintln!("  {} (lower fn, codegen var)", name);
    }
}
