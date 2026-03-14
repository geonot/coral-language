use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DocItem {
    pub name: String,
    pub kind: DocKind,
    pub doc_comment: String,
    pub params: Vec<String>,
    pub source_file: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocKind {
    Function,
    Store,
    Enum,
    Trait,
    Module,
}

pub fn extract_docs(source: &str, filename: &str) -> Vec<DocItem> {
    let mut items = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("# ") || line == "#" {
            let _comment_start = i;
            let mut doc_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l.starts_with("# ") {
                    doc_lines.push(&l[2..]);
                } else if l == "#" {
                    doc_lines.push("");
                } else {
                    break;
                }
                i += 1;
            }

            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }

            if i < lines.len() {
                let decl = lines[i].trim();
                if let Some(item) = parse_declaration(decl, &doc_lines, filename, i + 1) {
                    items.push(item);
                }
                i += 1;
            }
            continue;
        }

        let decl = lines[i].trim();
        if let Some(mut item) = parse_declaration(decl, &[], filename, i + 1) {
            item.doc_comment = String::new();
            items.push(item);
        }

        i += 1;
    }

    items
}

fn parse_declaration(line: &str, doc_lines: &[&str], filename: &str, line_num: usize) -> Option<DocItem> {
    if line.starts_with('*') {
        let rest = &line[1..];
        let (name, params) = if let Some(paren) = rest.find('(') {
            let name = rest[..paren].trim().to_string();
            let params_str = rest[paren + 1..].trim_end_matches(')');
            let params: Vec<String> = params_str
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            (name, params)
        } else {
            (rest.trim().to_string(), Vec::new())
        };

        return Some(DocItem {
            name,
            kind: DocKind::Function,
            doc_comment: doc_lines.join("\n"),
            params,
            source_file: filename.to_string(),
            line: line_num,
        });
    }

    if line.starts_with("store ") {
        let name = line["store ".len()..].trim().to_string();
        return Some(DocItem {
            name,
            kind: DocKind::Store,
            doc_comment: doc_lines.join("\n"),
            params: Vec::new(),
            source_file: filename.to_string(),
            line: line_num,
        });
    }

    if line.starts_with("type ") && line.contains(" is ") {
        let name = line["type ".len()..].split(" is ").next()?.trim().to_string();
        return Some(DocItem {
            name,
            kind: DocKind::Enum,
            doc_comment: doc_lines.join("\n"),
            params: Vec::new(),
            source_file: filename.to_string(),
            line: line_num,
        });
    }

    if line.starts_with("trait ") {
        let name = line["trait ".len()..].trim().to_string();
        return Some(DocItem {
            name,
            kind: DocKind::Trait,
            doc_comment: doc_lines.join("\n"),
            params: Vec::new(),
            source_file: filename.to_string(),
            line: line_num,
        });
    }

    None
}

pub fn generate_markdown(items: &[DocItem], module_name: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", module_name));

    let functions: Vec<&DocItem> = items.iter().filter(|i| i.kind == DocKind::Function).collect();
    let stores: Vec<&DocItem> = items.iter().filter(|i| i.kind == DocKind::Store).collect();
    let enums: Vec<&DocItem> = items.iter().filter(|i| i.kind == DocKind::Enum).collect();
    let traits: Vec<&DocItem> = items.iter().filter(|i| i.kind == DocKind::Trait).collect();

    if !stores.is_empty() {
        out.push_str("## Stores\n\n");
        for item in &stores {
            out.push_str(&format!("### `store {}`\n\n", item.name));
            if !item.doc_comment.is_empty() {
                out.push_str(&item.doc_comment);
                out.push_str("\n\n");
            }
        }
    }

    if !enums.is_empty() {
        out.push_str("## Types\n\n");
        for item in &enums {
            out.push_str(&format!("### `type {}`\n\n", item.name));
            if !item.doc_comment.is_empty() {
                out.push_str(&item.doc_comment);
                out.push_str("\n\n");
            }
        }
    }

    if !traits.is_empty() {
        out.push_str("## Traits\n\n");
        for item in &traits {
            out.push_str(&format!("### `trait {}`\n\n", item.name));
            if !item.doc_comment.is_empty() {
                out.push_str(&item.doc_comment);
                out.push_str("\n\n");
            }
        }
    }

    if !functions.is_empty() {
        out.push_str("## Functions\n\n");
        for item in &functions {
            let params_str = item.params.join(", ");
            out.push_str(&format!("### `*{}({})`\n\n", item.name, params_str));
            if !item.doc_comment.is_empty() {
                out.push_str(&item.doc_comment);
                out.push_str("\n\n");
            }
            out.push_str(&format!(
                "*Defined in `{}` line {}*\n\n",
                item.source_file, item.line
            ));
            out.push_str("---\n\n");
        }
    }

    out
}

pub fn generate_docs_for_directory(input_dir: &Path, output_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;
    let mut generated = Vec::new();

    for entry in fs::read_dir(input_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "coral") {
            let filename = path.file_stem().unwrap().to_string_lossy().to_string();
            let source = fs::read_to_string(&path)?;
            let items = extract_docs(&source, &path.to_string_lossy());
            if !items.is_empty() {
                let markdown = generate_markdown(&items, &filename);
                let out_path = output_dir.join(format!("{}.md", filename));
                fs::write(&out_path, &markdown)?;
                generated.push(out_path);
            }
        }
    }

    Ok(generated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_function_with_doc() {
        let source = r#"
# Read entire file contents as string
*read(path)
        fs_read(path)
"#;
        let items = extract_docs(source, "test.coral");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "read");
        assert_eq!(items[0].kind, DocKind::Function);
        assert_eq!(items[0].params, vec!["path"]);
        assert!(items[0].doc_comment.contains("Read entire file"));
    }

    #[test]
    fn extract_function_no_doc() {
        let source = "*add(a, b)\n        a + b\n";
        let items = extract_docs(source, "test.coral");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "add");
        assert_eq!(items[0].params, vec!["a", "b"]);
    }

    #[test]
    fn extract_store() {
        let source = "# A user store\nstore User\n    name\n    age\n";
        let items = extract_docs(source, "test.coral");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "User");
        assert_eq!(items[0].kind, DocKind::Store);
    }

    #[test]
    fn extract_enum() {
        let source = "# Option type\ntype Option is Some(value) or None\n";
        let items = extract_docs(source, "test.coral");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Option");
        assert_eq!(items[0].kind, DocKind::Enum);
    }

    #[test]
    fn generate_markdown_output() {
        let items = vec![DocItem {
            name: "greet".to_string(),
            kind: DocKind::Function,
            doc_comment: "Say hello to someone".to_string(),
            params: vec!["name".to_string()],
            source_file: "greet.coral".to_string(),
            line: 5,
        }];
        let md = generate_markdown(&items, "greet");
        assert!(md.contains("# greet"));
        assert!(md.contains("`*greet(name)`"));
        assert!(md.contains("Say hello"));
    }

    #[test]
    fn multiline_doc_comment() {
        let source = "# First line\n# Second line\n#\n# After blank\n*foo()\n    42\n";
        let items = extract_docs(source, "test.coral");
        assert_eq!(items.len(), 1);
        assert!(items[0].doc_comment.contains("First line"));
        assert!(items[0].doc_comment.contains("Second line"));
        assert!(items[0].doc_comment.contains("After blank"));
    }
}
