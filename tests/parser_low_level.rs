use coralc::lexer;
use coralc::parser::Parser;

#[test]
fn test_extern_fn_declaration() {
    let source = r#"
extern fn coral_malloc(size: usize) : usize
extern fn coral_free(p: usize)
"#;
    let tokens = lexer::lex(source).unwrap();
    let program = Parser::new(tokens, source.len()).parse();
    assert!(program.is_ok(), "extern fn should parse: {:?}", program.err());
    let prog = program.unwrap();
    assert_eq!(prog.items.len(), 2);
}

#[test]
fn test_unsafe_block() {
    let source = r#"
*test()
    unsafe
        x is 42
"#;
    let tokens = lexer::lex(source).unwrap();
    let program = Parser::new(tokens, source.len()).parse();
    assert!(program.is_ok(), "unsafe block should parse: {:?}", program.err());
}

#[test]
fn test_asm_expression() {
    let source = r#"
*nop()
    asm("nop")
"#;
    let tokens = lexer::lex(source).unwrap();
    let program = Parser::new(tokens, source.len()).parse();
    assert!(program.is_ok(), "asm expression should parse: {:?}", program.err());
}

#[test]
fn test_ptr_load() {
    let source = r#"
*read_byte(addr)
    @addr
"#;
    let tokens = lexer::lex(source).unwrap();
    let program = Parser::new(tokens, source.len()).parse();
    assert!(program.is_ok(), "ptr load should parse: {:?}", program.err());
}
