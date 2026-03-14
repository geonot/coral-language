use coralc::Compiler;
use coralc::module_loader::ModuleLoader;
use std::path::PathBuf;

fn try_compile(name: &str) {
    let ws = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(ws).join(name);
    let mut loader = ModuleLoader::with_default_std();
    let source = loader.load(&path).unwrap();

    // Find line number from byte offset
    let find_line = |offset: usize| -> (usize, String) {
        let mut line = 1;
        for (i, c) in source.char_indices() {
            if i >= offset {
                break;
            }
            if c == '\n' {
                line += 1;
            }
        }
        let line_text = source.lines().nth(line - 1).unwrap_or("").to_string();
        (line, line_text)
    };

    let compiler = Compiler;
    match compiler.compile_to_ir(&source) {
        Ok(ir) => println!("{}: OK ({} bytes)", name, ir.len()),
        Err(e) => {
            let (line, text) = find_line(e.diagnostic.span.start);
            println!("{}: ERROR at line {} (expanded source)", name, line);
            println!("  Stage: {:?}", e.stage);
            let msg = &e.diagnostic.message;
            let msg_short = if msg.len() > 300 { &msg[..300] } else { msg };
            println!("  Message: {}", msg_short);
            println!("  Line text: {:?}", text);
            if let Some(h) = &e.diagnostic.help {
                println!("  Help: {}", h);
            }
            // Print 3 lines before and after
            let lines: Vec<&str> = source.lines().collect();
            let start = if line > 4 { line - 4 } else { 0 };
            let end = std::cmp::min(lines.len(), line + 3);
            for i in start..end {
                let marker = if i + 1 == line { ">>>" } else { "   " };
                println!("  {} {:4}: {}", marker, i + 1, lines[i]);
            }
        }
    }
}

#[test]
fn diagnose_compile_errors() {
    try_compile("self_hosted/lower.coral");
    try_compile("self_hosted/module_loader.coral");
    try_compile("self_hosted/semantic.coral");
    try_compile("self_hosted/codegen.coral");
    try_compile("self_hosted/compiler.coral");
}

#[test]
fn dump_codegen_expanded() {
    let ws = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(ws).join("self_hosted/codegen.coral");
    let mut loader = ModuleLoader::with_default_std();
    let source = loader.load(&path).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    for i in 580..640 {
        if i < lines.len() {
            let tabs = lines[i].len() - lines[i].trim_start_matches('\t').len();
            let display = if lines[i].len() > 60 {
                // Find a safe UTF-8 boundary at or before byte 60
                let mut end = 60;
                while end > 0 && !lines[i].is_char_boundary(end) {
                    end -= 1;
                }
                &lines[i][..end]
            } else {
                lines[i]
            };
            println!("{:4} (t={}): {}", i + 1, tabs, display);
        }
    }
}
