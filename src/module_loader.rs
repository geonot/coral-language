use anyhow::{bail, Context};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::time::SystemTime;

const STD_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/std");

/// Cache entry for a loaded module.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ModuleCacheEntry {
    /// The expanded source content
    content: String,
    /// Content hash of the original source file
    content_hash: u64,
    /// Last modified time of the source file
    modified_time: SystemTime,
    /// List of direct dependencies (module paths)
    dependencies: Vec<PathBuf>,
}

/// Compute content hash of a string.
fn content_hash(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Information about a loaded module for namespace tracking.
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Canonical path to the module
    pub path: PathBuf,
    /// Module namespace (derived from file path)
    pub namespace: String,
    /// Exported symbols (function names, type names, etc.)
    pub exports: Vec<String>,
    /// Imported modules
    pub imports: Vec<String>,
}

/// A single module's source and metadata, returned by `load_modules()`.
#[derive(Debug, Clone)]
pub struct ModuleSource {
    /// Module namespace (e.g., "std.math", "std.prelude", "main")
    pub name: String,
    /// Canonical path to the source file
    pub path: PathBuf,
    /// Raw source text for this module (without use directives, after recursive expansion)
    pub source: String,
    /// Full import directives (includes selective import info)
    pub import_directives: Vec<ImportDirective>,
    /// Names of modules this module directly imports (backward compat)
    pub imports: Vec<String>,
    /// Exported symbols extracted from the source
    pub exports: Vec<String>,
}

/// Parsed `use` directive with optional selective imports.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDirective {
    /// The module path (e.g., "std.math" from `use std.math.{sin, cos}`)
    pub module_path: String,
    /// If Some, only these symbols are imported (e.g., `{sin, cos}`).
    /// If None, all symbols are imported.
    pub selections: Option<Vec<String>>,
}

/// Loads Coral source files while expanding `use module_name` directives.
/// A directive simply splices the referenced module's contents into the caller,
/// similar to a lightweight include, and prevents duplicate inclusions.
/// 
/// Features:
/// - Content-based caching with hash validation
/// - Circular import detection with detailed error messages
/// - Namespace tracking for future qualified imports
pub struct ModuleLoader {
    std_paths: Vec<PathBuf>,
    /// Content-based cache for loaded modules
    cache: HashMap<PathBuf, ModuleCacheEntry>,
    /// Module info for namespace tracking
    module_info: HashMap<PathBuf, ModuleInfo>,
}

impl ModuleLoader {
    pub fn new(std_paths: Vec<PathBuf>) -> Self {
        Self { 
            std_paths,
            cache: HashMap::new(),
            module_info: HashMap::new(),
        }
    }

    pub fn with_default_std() -> Self {
        Self::new(vec![PathBuf::from(STD_PATH)])
    }

    /// Check if a cached entry is still valid.
    fn is_cache_valid(&self, path: &Path, entry: &ModuleCacheEntry) -> bool {
        let mut visited = HashSet::new();
        self.is_cache_valid_inner(path, entry, &mut visited)
    }
    
    fn is_cache_valid_inner(&self, path: &Path, entry: &ModuleCacheEntry, visited: &mut HashSet<PathBuf>) -> bool {
        // Prevent infinite recursion for circular dependencies
        if visited.contains(path) {
            return true; // Already validated this path
        }
        visited.insert(path.to_path_buf());
        
        // Check if file still exists and has same modification time
        if let Ok(metadata) = fs::metadata(path) {
            if let Ok(modified) = metadata.modified() {
                if modified != entry.modified_time {
                    return false;
                }
            }
        } else {
            return false;
        }
        
        // Check if any dependencies have changed
        for dep_path in &entry.dependencies {
            if let Some(dep_entry) = self.cache.get(dep_path) {
                if !self.is_cache_valid_inner(dep_path, dep_entry, visited) {
                    return false;
                }
            } else {
                return false;
            }
        }
        
        true
    }

    /// Get module namespace from path.
    fn path_to_namespace(&self, path: &Path) -> String {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        
        // Check if this is a std lib file
        for std_path in &self.std_paths {
            let std_canonical = fs::canonicalize(std_path).unwrap_or_else(|_| std_path.clone());
            if let Ok(relative) = canonical.strip_prefix(&std_canonical) {
                let ns = relative
                    .with_extension("")
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, ".");
                return format!("std.{}", ns);
            }
        }
        
        // For non-std files, use the filename as namespace
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    /// Extract exported symbols from source (functions, types, stores).
    fn extract_exports(&self, source: &str) -> Vec<String> {
        let mut exports = Vec::new();
        
        for line in source.lines() {
            let trimmed = line.trim();
            
            // Function definition: *name(...)
            if trimmed.starts_with('*') {
                if let Some(paren) = trimmed.find('(') {
                    let name = trimmed[1..paren].trim();
                    if !name.is_empty() {
                        exports.push(name.to_string());
                    }
                }
            }
            
            // Type definition: type Name
            if trimmed.starts_with("type ") {
                let rest = trimmed[5..].trim();
                if let Some(end) = rest.find(|c: char| !c.is_alphanumeric() && c != '_') {
                    exports.push(rest[..end].to_string());
                } else if !rest.is_empty() {
                    exports.push(rest.split_whitespace().next().unwrap_or("").to_string());
                }
            }
            
            // Store/actor definition: store Name / actor Name
            if trimmed.starts_with("store ") || trimmed.starts_with("actor ") {
                let rest = trimmed.split_whitespace().nth(1).unwrap_or("");
                if !rest.is_empty() {
                    exports.push(rest.to_string());
                }
            }
            
            // Trait definition: trait Name
            if trimmed.starts_with("trait ") {
                let rest = trimmed[6..].trim();
                if let Some(end) = rest.find(|c: char| !c.is_alphanumeric() && c != '_') {
                    exports.push(rest[..end].to_string());
                } else if !rest.is_empty() {
                    exports.push(rest.to_string());
                }
            }
        }
        
        exports
    }

    /// Get information about loaded modules.
    pub fn get_module_info(&self, path: &Path) -> Option<&ModuleInfo> {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.module_info.get(&canonical)
    }

    /// Clear the cache (useful for testing or forced reload).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.module_info.clear();
    }

    pub fn load(&mut self, entry: &Path) -> anyhow::Result<String> {
        let mut local_cache = HashMap::new();
        let mut stack = Vec::new();
        let mut included = HashSet::new();
        let mut dependencies = Vec::new();
        
        // Auto-include prelude for all user files (not for std lib files)
        let entry_canonical = fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf());
        let is_std_file = self.std_paths.iter().any(|std_path| {
            let std_canonical = fs::canonicalize(std_path).unwrap_or_else(|_| std_path.clone());
            entry_canonical.starts_with(&std_canonical)
        });
        
        if !is_std_file {
            // Try to load prelude first
            if let Some(prelude_path) = self.resolve_module_in_std("prelude") {
                let prelude_source = self.load_recursive(&prelude_path, &mut stack, &mut local_cache, &mut included, &mut dependencies)?;
                if !prelude_source.is_empty() {
                    let mut result = prelude_source;
                    result.push('\n');
                    let user_source = self.load_recursive(entry, &mut stack, &mut local_cache, &mut included, &mut dependencies)?;
                    result.push_str(&user_source);
                    
                    // Update module info for the entry file
                    self.update_module_info(&entry_canonical, &result, &dependencies);
                    
                    return Ok(result);
                }
            }
        }
        
        let result = self.load_recursive(entry, &mut stack, &mut local_cache, &mut included, &mut dependencies)?;
        
        // Update module info for the entry file
        self.update_module_info(&entry_canonical, &result, &dependencies);
        
        Ok(result)
    }

    /// Load the entry file and all its dependencies, returning each module as a separate
    /// `ModuleSource` in topological (dependency) order. The entry file is always last.
    ///
    /// Unlike `load()`, this does NOT concatenate module sources. Each module retains
    /// only its own code (with `use` directives stripped).
    pub fn load_modules(&mut self, entry: &Path) -> anyhow::Result<Vec<ModuleSource>> {
        let mut stack = Vec::new();
        let mut visited = HashSet::new();
        let mut modules = Vec::new();

        let entry_canonical = fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf());
        let is_std_file = self.std_paths.iter().any(|std_path| {
            let std_canonical = fs::canonicalize(std_path).unwrap_or_else(|_| std_path.clone());
            entry_canonical.starts_with(&std_canonical)
        });

        // Auto-include prelude for non-std files
        if !is_std_file {
            if let Some(prelude_path) = self.resolve_module_in_std("prelude") {
                self.collect_modules(&prelude_path, &mut stack, &mut visited, &mut modules)?;
            }
        }

        self.collect_modules(entry, &mut stack, &mut visited, &mut modules)?;
        Ok(modules)
    }

    /// Recursively collect modules in dependency-first (topological) order.
    /// Each module's source has `use` directives stripped but is NOT inlined with deps.
    fn collect_modules(
        &mut self,
        path: &Path,
        stack: &mut Vec<PathBuf>,
        visited: &mut HashSet<PathBuf>,
        modules: &mut Vec<ModuleSource>,
    ) -> anyhow::Result<()> {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Circular import detection with line numbers (CC3.4)
        if stack.contains(&canonical) {
            let cycle_start = stack.iter().position(|p| p == &canonical).unwrap();
            let cycle_paths: Vec<_> = stack[cycle_start..]
                .iter()
                .chain(std::iter::once(&canonical))
                .collect();

            // Build enriched cycle description with line numbers
            let mut cycle_desc = Vec::new();
            for window in cycle_paths.windows(2) {
                let from = &window[0];
                let to = &window[1];
                let from_name = from.file_stem().map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| from.display().to_string());
                let to_name = to.file_stem().map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| to.display().to_string());
                // Find the line number of the `use` directive
                let line_info = Self::find_use_line_number(from, &to_name);
                match line_info {
                    Some(line) => cycle_desc.push(format!("{} (line {}) -> {}", from_name, line, to_name)),
                    None => cycle_desc.push(format!("{} -> {}", from_name, to_name)),
                }
            }

            let cycle_count = cycle_paths.len() - 1;
            let suggestion = if cycle_count == 2 {
                let a_name = cycle_paths[0].file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let b_name = cycle_paths[1].file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                format!(
                    "Suggestion: Extract shared code from `{}` and `{}` into a common module that both can import.",
                    a_name, b_name
                )
            } else {
                "Suggestion: Break the cycle by extracting shared code into a common module, or restructure imports.".to_string()
            };

            bail!(
                "circular import detected:\n  {}\n\n{}\n\
                 Hint: Consider restructuring to break the cycle.",
                cycle_desc.join("\n  "),
                suggestion
            );
        }

        if visited.contains(&canonical) {
            return Ok(());
        }

        stack.push(canonical.clone());
        let source = fs::read_to_string(path).with_context(|| {
            format!("failed to read module {}", path.display())
        })?;

        let mut own_source = String::new();
        let mut import_names = Vec::new();
        let mut import_directives = Vec::new();

        for (index, line) in source.lines().enumerate() {
            if let Some(directive) = Self::parse_use_directive(line) {
                let module_path = self
                    .resolve_module(path, &directive.module_path)
                    .with_context(|| {
                        format!(
                            "failed to resolve module `{}` referenced at {}:{}",
                            directive.module_path,
                            path.display(),
                            index + 1
                        )
                    })?;

                import_names.push(directive.module_path.clone());
                import_directives.push(directive);
                // Recursively collect the dependency first
                self.collect_modules(&module_path, stack, visited, modules)?;
                continue;
            }
            own_source.push_str(line);
            own_source.push('\n');
        }

        stack.pop();
        visited.insert(canonical.clone());

        let namespace = self.path_to_namespace(&canonical);
        let exports = self.extract_exports(&own_source);

        modules.push(ModuleSource {
            name: namespace,
            path: canonical,
            source: own_source,
            import_directives,
            imports: import_names,
            exports,
        });

        Ok(())
    }
    
    /// Update module info after loading.
    fn update_module_info(&mut self, path: &Path, source: &str, dependencies: &[PathBuf]) {
        let namespace = self.path_to_namespace(path);
        let exports = self.extract_exports(source);
        
        // Compute content hash
        let hash = content_hash(source);
        
        // Get modification time
        let modified_time = fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        
        // Extract import names from dependencies
        let imports: Vec<String> = dependencies
            .iter()
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .collect();
        
        // Update cache entry
        self.cache.insert(path.to_path_buf(), ModuleCacheEntry {
            content: source.to_string(),
            content_hash: hash,
            modified_time,
            dependencies: dependencies.to_vec(),
        });
        
        // Update module info
        self.module_info.insert(path.to_path_buf(), ModuleInfo {
            path: path.to_path_buf(),
            namespace,
            exports,
            imports,
        });
    }
    
    /// Resolve a module name in std paths only
    fn resolve_module_in_std(&self, module_name: &str) -> Option<PathBuf> {
        // Handle nested module paths (e.g., "runtime/actor" -> "runtime/actor.coral")
        let module_path = module_name.replace('/', std::path::MAIN_SEPARATOR_STR);
        
        for std_path in &self.std_paths {
            let candidate = std_path.join(format!("{}.coral", module_path));
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    fn load_recursive(
        &mut self,
        path: &Path,
        stack: &mut Vec<PathBuf>,
        local_cache: &mut HashMap<PathBuf, String>,
        included: &mut HashSet<PathBuf>,
        dependencies: &mut Vec<PathBuf>,
    ) -> anyhow::Result<String> {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        
        // Circular import detection with detailed error and line numbers (CC3.4)
        if stack.contains(&canonical) {
            let cycle_start = stack.iter().position(|p| p == &canonical).unwrap();
            let cycle_paths: Vec<_> = stack[cycle_start..]
                .iter()
                .chain(std::iter::once(&canonical))
                .collect();

            let mut cycle_desc = Vec::new();
            for window in cycle_paths.windows(2) {
                let from = &window[0];
                let to = &window[1];
                let from_name = from.file_stem().map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| from.display().to_string());
                let to_name = to.file_stem().map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| to.display().to_string());
                let line_info = Self::find_use_line_number(from, &to_name);
                match line_info {
                    Some(line) => cycle_desc.push(format!("{} (line {}) -> {}", from_name, line, to_name)),
                    None => cycle_desc.push(format!("{} -> {}", from_name, to_name)),
                }
            }

            let cycle_count = cycle_paths.len() - 1;
            let suggestion = if cycle_count == 2 {
                let a_name = cycle_paths[0].file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let b_name = cycle_paths[1].file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                format!(
                    "Suggestion: Extract shared code from `{}` and `{}` into a common module that both can import.",
                    a_name, b_name
                )
            } else {
                "Suggestion: Break the cycle by extracting shared code into a common module, or restructure imports.".to_string()
            };
            
            bail!(
                "circular import detected:\n  {}\n\n{}\n\
                 Hint: Consider restructuring to break the cycle.",
                cycle_desc.join("\n  "),
                suggestion
            );
        }
        
        // Check if already included in this load session (after cycle check!)
        if included.contains(&canonical) {
            return Ok(String::new());
        }
        
        // Check persistent cache first
        if let Some(entry) = self.cache.get(&canonical) {
            if self.is_cache_valid(&canonical, entry) {
                included.insert(canonical.clone());
                dependencies.push(canonical.clone());
                return Ok(entry.content.clone());
            }
        }
        
        // Check local session cache
        if let Some(existing) = local_cache.get(&canonical) {
            included.insert(canonical.clone());
            dependencies.push(canonical.clone());
            return Ok(existing.clone());
        }
        
        stack.push(canonical.clone());
        let source = fs::read_to_string(path).with_context(|| {
            format!("failed to read module {}", path.display())
        })?;

        let mut expanded = String::new();
        let mut module_deps = Vec::new();
        // Don't add to included until AFTER we're done processing
        
        for (index, line) in source.lines().enumerate() {
            if let Some(directive) = Self::parse_use_directive(line) {
                let module_path = self
                    .resolve_module(path, &directive.module_path)
                    .with_context(|| {
                        format!(
                            "failed to resolve module `{}` referenced at {}:{}",
                            directive.module_path,
                            path.display(),
                            index + 1
                        )
                    })?;
                
                // Track this as a dependency
                let dep_canonical = fs::canonicalize(&module_path)
                    .unwrap_or_else(|_| module_path.clone());
                module_deps.push(dep_canonical.clone());
                
                let module_source = self.load_recursive(&module_path, stack, local_cache, included, dependencies)?;
                if !module_source.is_empty() {
                    expanded.push_str(&module_source);
                    expanded.push('\n');
                }
                continue;
            }
            expanded.push_str(line);
            expanded.push('\n');
        }

        stack.pop();
        
        // Now mark as included after fully processing
        included.insert(canonical.clone());
        local_cache.insert(canonical.clone(), expanded.clone());
        dependencies.push(canonical.clone());
        
        // Update module info for this file
        self.update_module_info(&canonical, &expanded, &module_deps);
        
        Ok(expanded)
    }

    /// CC3.4: Find the line number of a `use` directive that imports a given module.
    fn find_use_line_number(file_path: &Path, target_module: &str) -> Option<usize> {
        let source = fs::read_to_string(file_path).ok()?;
        for (i, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("use ") {
                let module_part = trimmed[4..].trim();
                // Check if this use directive references the target module
                // Handle both simple (`use b`) and qualified (`use std.math`) references
                let module_name = module_part.split('.').last().unwrap_or(module_part);
                let module_name = module_name.split('{').next().unwrap_or(module_name).trim();
                if module_name.eq_ignore_ascii_case(target_module) || module_part == target_module {
                    return Some(i + 1); // 1-based line number
                }
            }
        }
        None
    }

    fn parse_use_directive(line: &str) -> Option<ImportDirective> {
        let trimmed = line.trim();
        if !trimmed.starts_with("use ") {
            return None;
        }
        let module = trimmed[4..].trim();
        if module.is_empty() {
            return None;
        }

        // Check for selective imports: `use std.math.{sin, cos}`
        if let Some(brace_start) = module.find('.') {
            let rest = &module[brace_start + 1..];
            if rest.starts_with('{') && rest.ends_with('}') {
                let module_path = module[..brace_start].to_string();
                let symbols_str = &rest[1..rest.len() - 1];
                let selections: Vec<String> = symbols_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                return Some(ImportDirective {
                    module_path,
                    selections: if selections.is_empty() { None } else { Some(selections) },
                });
            }
        }

        // Also check for brace at any depth: `use std.math.{sin, cos}`
        // The module could be "std.math.{sin, cos}" where the brace part comes after the last dot
        if module.contains('{') {
            if let Some(brace_pos) = module.rfind('.') {
                let rest = module[brace_pos + 1..].trim();
                if rest.starts_with('{') && rest.ends_with('}') {
                    let module_path = module[..brace_pos].to_string();
                    let symbols_str = &rest[1..rest.len() - 1];
                    let selections: Vec<String> = symbols_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    return Some(ImportDirective {
                        module_path,
                        selections: if selections.is_empty() { None } else { Some(selections) },
                    });
                }
            }
        }

        Some(ImportDirective {
            module_path: module.to_string(),
            selections: None,
        })
    }

    fn resolve_module(&self, current_file: &Path, module: &str) -> anyhow::Result<PathBuf> {
        let relative = module.replace('.', "/");
        let file_name = format!("{}.coral", relative);
        let mut candidates = Vec::new();

        if let Some(parent) = current_file.parent() {
            candidates.push(parent.join(&file_name));
        }
        candidates.push(PathBuf::from(&file_name));
        for std_path in &self.std_paths {
            // Common case: module = "std.prelude" -> relative = "std/prelude".
            // We want to resolve to <workspace>/std/prelude.coral, not <workspace>/std/std/prelude.coral.
            let inner = relative.strip_prefix("std/").unwrap_or(&relative);
            candidates.push(std_path.join(&file_name));
            candidates.push(std_path.join(format!("{}.coral", inner)));
        }

        for candidate in candidates {
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        bail!(
            "module `{}` not found. Ensure `{}` exists in the current directory or std paths (e.g., {}).",
            module,
            file_name,
            STD_PATH
        );
    }
}

#[cfg(test)]
mod tests {
    use super::ModuleLoader;
    use std::fs;

    #[test]
    fn expands_use_directive() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let entry = temp_dir.path().join("main.coral");
        let module = temp_dir.path().join("utils.coral");
        fs::write(&module, "*increment(v)\n    v + 1\n").unwrap();
        fs::write(&entry, "use utils\nvalue is 1\nincrement(value)\n").unwrap();

        let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
        let expanded = loader.load(&entry).expect("expanded source");
        // module defines a function named increment using Coral function syntax
        assert!(expanded.contains("*increment"));
        assert!(!expanded.contains("use utils"));
    }
    
    #[test]
    fn caches_modules() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let entry = temp_dir.path().join("main.coral");
        let module = temp_dir.path().join("utils.coral");
        fs::write(&module, "*helper()\n    42\n").unwrap();
        fs::write(&entry, "use utils\nhelper()\n").unwrap();

        let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
        
        // First load
        let _ = loader.load(&entry).expect("first load");
        
        // Check that module info was populated
        let module_canonical = fs::canonicalize(&module).unwrap();
        let info = loader.get_module_info(&module_canonical);
        assert!(info.is_some());
        
        let info = info.unwrap();
        assert!(info.exports.contains(&"helper".to_string()));
        
        // Second load should use cache
        let expanded2 = loader.load(&entry).expect("second load");
        assert!(expanded2.contains("*helper"));
    }
    
    #[test]
    fn detects_circular_imports() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let a = temp_dir.path().join("a.coral");
        let b = temp_dir.path().join("b.coral");
        fs::write(&a, "use b\n*a_func()\n    1\n").unwrap();
        fs::write(&b, "use a\n*b_func()\n    2\n").unwrap();

        let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
        let result = loader.load(&a);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("circular import"));
    }
    
    #[test]
    fn extracts_exports() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let entry = temp_dir.path().join("test.coral");
        fs::write(&entry, r#"
type Person = { name: String, age: Int }
store Counter
    count: Int = 0
actor Logger
    ...
*add(a, b)
    a + b
trait Printable
    *print()
"#).unwrap();

        let mut loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
        let _ = loader.load(&entry).expect("load");
        
        let canonical = fs::canonicalize(&entry).unwrap();
        let info = loader.get_module_info(&canonical).expect("module info");
        
        assert!(info.exports.contains(&"Person".to_string()));
        assert!(info.exports.contains(&"Counter".to_string()));
        assert!(info.exports.contains(&"Logger".to_string()));
        assert!(info.exports.contains(&"add".to_string()));
        assert!(info.exports.contains(&"Printable".to_string()));
    }
}
