pub mod ast;
pub mod codegen;
pub mod compiler;
pub mod diagnostics;
pub mod doc_gen;
pub mod lexer;
pub mod lower;
pub mod module_loader;
pub mod package;
pub mod parser;
pub mod semantic;
pub mod span;
pub mod types;

pub use compiler::Compiler;
