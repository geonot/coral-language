use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub entry: String,
    pub dependencies: HashMap<String, DepSpec>,
}

#[derive(Debug, Clone)]
pub struct DepSpec {
    pub version: String,
    pub path: Option<String>,
    pub git: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Lockfile {
    pub entries: Vec<LockEntry>,
}

pub fn parse_manifest(content: &str) -> Result<Package, String> {
    let table: toml::Table = content
        .parse()
        .map_err(|e: toml::de::Error| format!("invalid TOML: {}", e))?;

    let pkg_table = table
        .get("package")
        .and_then(|v| v.as_table())
        .ok_or_else(|| "missing [package] section".to_string())?;

    let name = pkg_table
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "package name is required".to_string())?
        .to_string();

    let version = pkg_table
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.1.0")
        .to_string();

    let description = pkg_table
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let authors = pkg_table
        .get("authors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let entry = pkg_table
        .get("entry")
        .and_then(|v| v.as_str())
        .unwrap_or("main.coral")
        .to_string();

    let mut dependencies: HashMap<String, DepSpec> = HashMap::new();
    if let Some(deps_table) = table.get("dependencies").and_then(|v| v.as_table()) {
        for (dep_name, dep_val) in deps_table {
            let spec = match dep_val {
                toml::Value::String(ver) => DepSpec {
                    version: ver.clone(),
                    path: None,
                    git: None,
                },
                toml::Value::Table(t) => DepSpec {
                    version: t
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*")
                        .to_string(),
                    path: t.get("path").and_then(|v| v.as_str()).map(String::from),
                    git: t.get("git").and_then(|v| v.as_str()).map(String::from),
                },
                _ => {
                    return Err(format!(
                        "dependency '{}' has invalid format",
                        dep_name
                    ));
                }
            };
            dependencies.insert(dep_name.clone(), spec);
        }
    }

    Ok(Package {
        name,
        version,
        description,
        authors,
        entry,
        dependencies,
    })
}

pub fn load_manifest(project_dir: &Path) -> Result<Package, String> {
    let manifest_path = project_dir.join("coral.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "no coral.toml found in {}",
            project_dir.display()
        ));
    }
    let content =
        fs::read_to_string(&manifest_path).map_err(|e| format!("failed to read coral.toml: {}", e))?;
    parse_manifest(&content)
}

pub struct ResolvedDep {
    pub name: String,
    pub version: String,
    pub source_dir: PathBuf,
}

pub fn resolve_dependencies(
    package: &Package,
    project_dir: &Path,
) -> Result<Vec<ResolvedDep>, String> {
    let mut resolved = Vec::new();
    let deps_dir = project_dir.join("coral_deps");

    for (name, spec) in &package.dependencies {
        if let Some(local_path) = &spec.path {
            let dep_dir = project_dir.join(local_path);
            if !dep_dir.exists() {
                return Err(format!(
                    "dependency '{}' path '{}' does not exist",
                    name, local_path
                ));
            }
            // Validate that the dependency path doesn't escape the project root
            let canonical_dep = dep_dir.canonicalize().map_err(|e| {
                format!("dependency '{}' path could not be resolved: {}", name, e)
            })?;
            let canonical_project = project_dir.canonicalize().map_err(|e| {
                format!("project directory could not be resolved: {}", e)
            })?;
            if !canonical_dep.starts_with(&canonical_project) {
                return Err(format!(
                    "dependency '{}' path '{}' resolves to '{}' which is outside the project root",
                    name, local_path, canonical_dep.display()
                ));
            }
            resolved.push(ResolvedDep {
                name: name.clone(),
                version: spec.version.clone(),
                source_dir: canonical_dep,
            });
        } else if spec.git.is_some() {
            return Err(format!(
                "dependency '{}': git dependencies are not yet supported. Use path dependencies instead",
                name
            ));
        } else {
            let dep_dir = deps_dir.join(name);
            if dep_dir.exists() {
                resolved.push(ResolvedDep {
                    name: name.clone(),
                    version: spec.version.clone(),
                    source_dir: dep_dir,
                });
            } else {
                return Err(format!(
                    "dependency '{}' version '{}' not found in coral_deps/",
                    name, spec.version
                ));
            }
        }
    }

    Ok(resolved)
}

pub fn init_project(dir: &Path, name: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;

    let manifest = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
description = ""
entry = "main.coral"

[dependencies]
"#,
        name
    );
    fs::write(dir.join("coral.toml"), manifest)?;

    let main_content = r#"*main()
    log("Hello from " + "{name}!")
"#
    .replace("{name}", name);
    fs::write(dir.join("main.coral"), main_content)?;

    Ok(())
}

pub fn generate_lockfile(package: &Package, resolved: &[ResolvedDep]) -> String {
    let mut out = String::new();
    out.push_str("# coral.lock — auto-generated, do not edit\n\n");
    out.push_str(&format!(
        "[root]\nname = \"{}\"\nversion = \"{}\"\n\n",
        package.name, package.version
    ));

    for dep in resolved {
        out.push_str(&format!(
            "[[dependency]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"{}\"\n\n",
            dep.name,
            dep.version,
            dep.source_dir.display()
        ));
    }

    out
}

pub fn parse_lockfile(content: &str) -> Lockfile {
    let mut entries = Vec::new();
    let mut current_name = String::new();
    let mut current_version = String::new();
    let mut current_source = String::new();
    let mut in_dep = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[dependency]]" {
            if in_dep && !current_name.is_empty() {
                entries.push(LockEntry {
                    name: std::mem::take(&mut current_name),
                    version: std::mem::take(&mut current_version),
                    source: std::mem::take(&mut current_source),
                    checksum: None,
                });
            }
            in_dep = true;
            continue;
        }
        if !in_dep {
            continue;
        }
        if let Some(eq_pos) = trimmed.find(" = ") {
            let key = trimmed[..eq_pos].trim();
            let val = trimmed[eq_pos + 3..].trim().trim_matches('"');
            match key {
                "name" => current_name = val.to_string(),
                "version" => current_version = val.to_string(),
                "source" => current_source = val.to_string(),
                _ => {}
            }
        }
    }
    if in_dep && !current_name.is_empty() {
        entries.push(LockEntry {
            name: current_name,
            version: current_version,
            source: current_source,
            checksum: None,
        });
    }

    Lockfile { entries }
}

pub fn load_lockfile(project_dir: &Path) -> Option<Lockfile> {
    let lock_path = project_dir.join("coral.lock");
    if let Ok(content) = fs::read_to_string(&lock_path) {
        Some(parse_lockfile(&content))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_manifest() {
        let toml = r#"
[package]
name = "myapp"
version = "1.0.0"
description = "A test app"
entry = "app.coral"

[dependencies]
utils = "0.2.0"
"#;
        let pkg = parse_manifest(toml).unwrap();
        assert_eq!(pkg.name, "myapp");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.entry, "app.coral");
        assert_eq!(pkg.dependencies.len(), 1);
        assert_eq!(pkg.dependencies["utils"].version, "0.2.0");
    }

    #[test]
    fn parse_path_dependency() {
        let toml = r#"
[package]
name = "myapp"
version = "0.1.0"

[dependencies]
mylib = { path = "../mylib", version = "0.1.0" }
"#;
        let pkg = parse_manifest(toml).unwrap();
        assert_eq!(pkg.dependencies["mylib"].path, Some("../mylib".to_string()));
    }

    #[test]
    fn parse_missing_name_fails() {
        let toml = "[package]\nversion = \"1.0.0\"\n";
        assert!(parse_manifest(toml).is_err());
    }

    #[test]
    fn init_project_creates_files() {
        let dir = std::env::temp_dir().join("coral_pkg_test_init");
        let _ = std::fs::remove_dir_all(&dir);
        init_project(&dir, "testpkg").unwrap();
        assert!(dir.join("coral.toml").exists());
        assert!(dir.join("main.coral").exists());
        let manifest = fs::read_to_string(dir.join("coral.toml")).unwrap();
        assert!(manifest.contains("testpkg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_local_dependency() {
        let dir = std::env::temp_dir().join("coral_pkg_test_resolve");
        let _ = std::fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("libs/mylib")).unwrap();
        fs::write(
            dir.join("libs/mylib/main.coral"),
            "*helper()\n    42\n",
        )
        .unwrap();

        let pkg = Package {
            name: "app".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            authors: vec![],
            entry: "main.coral".to_string(),
            dependencies: {
                let mut m = HashMap::new();
                m.insert(
                    "mylib".to_string(),
                    DepSpec {
                        version: "0.1.0".to_string(),
                        path: Some("libs/mylib".to_string()),
                        git: None,
                    },
                );
                m
            },
        };

        let resolved = resolve_dependencies(&pkg, &dir).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "mylib");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lockfile_generation() {
        let pkg = Package {
            name: "app".to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            authors: vec![],
            entry: "main.coral".to_string(),
            dependencies: HashMap::new(),
        };
        let deps = vec![ResolvedDep {
            name: "foo".to_string(),
            version: "0.2.0".to_string(),
            source_dir: PathBuf::from("/tmp/deps/foo"),
        }];
        let lock = generate_lockfile(&pkg, &deps);
        assert!(lock.contains("name = \"app\""));
        assert!(lock.contains("name = \"foo\""));
        assert!(lock.contains("version = \"0.2.0\""));
    }
}
