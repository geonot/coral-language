use coralc::ast::{Expression, Function, Item};
use coralc::compiler::Compiler;
use coralc::lexer;
use coralc::parser::Parser;

fn parse_source(src: &str) -> coralc::ast::Program {
    let tokens = lexer::lex(src).expect("lexing failed");
    let parser = Parser::new(tokens, src.len());
    parser.parse().expect("parsing failed")
}

fn find_function<'a>(program: &'a coralc::ast::Program, name: &str) -> &'a Function {
    program
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Function(f) = item {
                Some(f)
            } else {
                None
            }
        })
        .find(|f| f.name == name)
        .expect(&format!("function `{}` not found", name))
}

// ── Parser tests ──────────────────────────────────────────────

#[test]
fn parse_named_args_basic() {
    let src = "*foo(a, b)\n    a + b\n*main()\n    foo(b: 2, a: 1)\n";
    let program = parse_source(src);
    let main_fn = find_function(&program, "main");
    // The call may be the block value (trailing expression) or a statement
    let call_expr = if let Some(val) = &main_fn.body.value {
        val.as_ref()
    } else if let Some(stmt) = main_fn.body.statements.first() {
        match stmt {
            coralc::ast::Statement::Expression(e) => e,
            _ => panic!("expected expression statement"),
        }
    } else {
        panic!("expected a call in main body");
    };
    match call_expr {
        Expression::Call {
            arg_names, args, ..
        } => {
            assert_eq!(arg_names.len(), 2);
            assert_eq!(arg_names[0], Some("b".to_string()));
            assert_eq!(arg_names[1], Some("a".to_string()));
            assert_eq!(args.len(), 2);
        }
        _ => panic!("expected a Call expression, got {:?}", call_expr),
    }
}

#[test]
fn parse_mixed_positional_and_named() {
    let src = "*foo(a, b, c)\n    a + b + c\n*main()\n    foo(1, c: 3, b: 2)\n";
    let program = parse_source(src);
    let main_fn = find_function(&program, "main");
    if let Some(stmt) = main_fn.body.statements.first() {
        match stmt {
            coralc::ast::Statement::Expression(Expression::Call { arg_names, .. }) => {
                assert_eq!(arg_names.len(), 3);
                assert_eq!(arg_names[0], None); // positional
                assert_eq!(arg_names[1], Some("c".to_string()));
                assert_eq!(arg_names[2], Some("b".to_string()));
            }
            _ => panic!("expected a Call expression"),
        }
    }
}

#[test]
fn parse_all_positional_has_empty_names() {
    let src = "*foo(a)\n    a\n*main()\n    foo(42)\n";
    let program = parse_source(src);
    let main_fn = find_function(&program, "main");
    if let Some(stmt) = main_fn.body.statements.first() {
        match stmt {
            coralc::ast::Statement::Expression(Expression::Call { arg_names, .. }) => {
                assert!(
                    arg_names.is_empty(),
                    "all-positional calls should have empty arg_names"
                );
            }
            _ => panic!("expected a Call expression"),
        }
    }
}

#[test]
fn parse_named_after_positional_error() {
    // positional arg after a named arg should be an error
    let src = "*foo(a, b)\n    a\n*main()\n    foo(a: 1, 2)\n";
    let tokens = lexer::lex(src).expect("lexing failed");
    let parser = Parser::new(tokens, src.len());
    let result = parser.parse();
    assert!(result.is_err(), "positional after named should fail");
}

// ── E2E codegen tests ──────────────────────────────────────────

fn run_coral(source: &str) -> String {
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).unwrap_or_else(|e| {
        panic!("compilation failed: {}", e.diagnostic.message);
    });
    let mut ir_file = tempfile::NamedTempFile::new().expect("create temp file");
    std::io::Write::write_all(&mut ir_file, ir.as_bytes()).expect("write IR");
    std::io::Write::flush(&mut ir_file).expect("flush");
    let runtime =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/libruntime.so");
    assert!(
        runtime.exists(),
        "Runtime library not found. Run `cargo build -p runtime` first."
    );
    let output = std::process::Command::new("lli")
        .arg("-load")
        .arg(&runtime)
        .arg(ir_file.path())
        .output()
        .expect("failed to run lli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        panic!(
            "lli failed: {}\nstdout: {}\nstderr: {}",
            output.status, stdout, stderr
        );
    }
    stdout.trim().to_string()
}

#[test]
fn e2e_named_args_reorder() {
    let source = r#"
*greet(name, greeting)
    log(greeting + ', ' + name)

*main()
    greet(greeting: "Hello", name: "World")
"#;
    assert_eq!(run_coral(source), "Hello, World");
}

#[test]
fn e2e_named_args_with_defaults() {
    let source = r#"
*connect(host, port ? 5432, timeout ? 30)
    log(host + ':' + to_string(port) + ':' + to_string(timeout))

*main()
    connect(host: "db.local", timeout: 60)
"#;
    assert_eq!(run_coral(source), "db.local:5432:60");
}

#[test]
fn e2e_mixed_positional_and_named() {
    let source = r#"
*add(a, b, c)
    a + b + c

*main()
    result is add(10, c: 30, b: 20)
    log(to_string(result))
"#;
    assert_eq!(run_coral(source), "60");
}

#[test]
fn e2e_all_named() {
    let source = r#"
*sub(x, y)
    x - y

*main()
    result is sub(y: 3, x: 10)
    log(to_string(result))
"#;
    assert_eq!(run_coral(source), "7");
}

#[test]
fn e2e_named_args_string() {
    let source = r#"
*format_entry(key, value, sep ? ': ')
    key + sep + value

*main()
    log(format_entry(value: "World", key: "Hello", sep: " = "))
"#;
    assert_eq!(run_coral(source), "Hello = World");
}
