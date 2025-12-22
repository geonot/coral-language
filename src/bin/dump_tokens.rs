use anyhow::{anyhow, Context, Result};
use coralc::lexer;
use coralc::parser::Parser;
use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let path = env::args().nth(1).expect("usage: dump_tokens <file>");
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path))?;
    let tokens = lexer::lex(&contents).map_err(|diag| anyhow!(diag.message.clone()))?;
    println!("tokens for {}:", Path::new(&path).display());
    for token in &tokens {
        println!("{:?} {:?}", token.kind, token.span);
    }
    let parser = Parser::new(tokens.clone(), contents.len());
    match parser.parse() {
        Ok(_) => println!("parse ok"),
        Err(err) => println!("parse error: {:?}", err),
    }
    Ok(())
}
