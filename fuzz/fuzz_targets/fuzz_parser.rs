#![no_main]
//! Fuzz target for the Coral parser.
//!
//! Feeds arbitrary UTF-8 strings through lex → parse pipeline.
//! Must never panic — all inputs should produce either Ok(AST) or Err(diagnostic).
//!
//! Run with: cargo fuzz run fuzz_parser -- -max_len=4096

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        // Lex first — if it fails, that's fine
        if let Ok(tokens) = coralc::lexer::lex(source) {
            // Parse the token stream — must not panic
            let parser = coralc::parser::Parser::new(tokens, source.len());
            let _ = parser.parse();
        }
    }
});
