use anyhow::{bail, Context};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashSet;

const STD_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/std");

/// Loads Coral source files while expanding `use module_name` directives.
/// A directive simply splices the referenced module's contents into the caller,
/// similar to a lightweight include, and prevents duplicate inclusions.
pub struct ModuleLoader {
    std_paths: Vec<PathBuf>,
}

impl ModuleLoader {
    pub fn new(std_paths: Vec<PathBuf>) -> Self {
        Self { std_paths }
    }

    pub fn with_default_std() -> Self {
        Self::new(vec![PathBuf::from(STD_PATH)])
    }

    pub fn load(&self, entry: &Path) -> anyhow::Result<String> {
        let mut cache = HashMap::new();
        let mut stack = Vec::new();
        let mut included = HashSet::new();
        self.load_recursive(entry, &mut stack, &mut cache, &mut included)
    }

    fn load_recursive(
        &self,
        path: &Path,
        stack: &mut Vec<PathBuf>,
        cache: &mut HashMap<PathBuf, String>,
        included: &mut HashSet<PathBuf>,
    ) -> anyhow::Result<String> {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if included.contains(&canonical) {
            return Ok(String::new());
        }
        if let Some(existing) = cache.get(&canonical) {
            included.insert(canonical.clone());
            return Ok(existing.clone());
        }
        if stack.contains(&canonical) {
            let cycle = stack
                .iter()
                .chain(std::iter::once(&canonical))
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> ");
            bail!("cyclic use detected: {}", cycle);
        }
        stack.push(canonical.clone());
        let source = fs::read_to_string(path).with_context(|| {
            format!("failed to read module {}", path.display())
        })?;

        let mut expanded = String::new();
        included.insert(canonical.clone());
        for (index, line) in source.lines().enumerate() {
            if let Some(module_name) = Self::parse_use_directive(line) {
                let module_path = self
                    .resolve_module(path, &module_name)
                    .with_context(|| {
                        format!(
                            "failed to resolve module `{}` referenced at {}:{}",
                            module_name,
                            path.display(),
                            index + 1
                        )
                    })?;
                let module_source = self.load_recursive(&module_path, stack, cache, included)?;
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
        cache.insert(canonical.clone(), expanded.clone());
        Ok(expanded)
    }

    fn parse_use_directive(line: &str) -> Option<String> {
        let trimmed = line.trim();
        if !trimmed.starts_with("use ") {
            return None;
        }
        let module = trimmed[4..].trim();
        if module.is_empty() {
            return None;
        }
        Some(module.to_string())
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
            let inner = if relative.starts_with("std/") {
                &relative[4..]
            } else {
                &relative
            };
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

        let loader = ModuleLoader::new(vec![temp_dir.path().to_path_buf()]);
    let expanded = loader.load(&entry).expect("expanded source");
    // module defines a function named increment using Coral function syntax
    assert!(expanded.contains("*increment"));
    assert!(!expanded.contains("use utils"));
    }
}
