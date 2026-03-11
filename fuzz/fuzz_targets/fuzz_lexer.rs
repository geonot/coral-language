#![no_main]
//! Fuzz target for the Coral lexer.
//!
//! Feeds arbitrary UTF-8 strings into the lexer. Must never panic —
//! all inputs should produce either Ok(tokens) or Err(diagnostic).
//!
//! Run with: cargo fuzz run fuzz_lexer -- -max_len=4096

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Only fuzz valid UTF-8 to avoid spam on the string conversion
    if let Ok(source) = std::str::from_utf8(data) {
        // The lexer must not panic on any input
        let _ = coralc::lexer::lex(source);
    }
});
