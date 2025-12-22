pub mod ast;
pub mod codegen;
pub mod compiler;
pub mod diagnostics;
pub mod lexer;
pub mod lower;
pub mod module_loader;
pub mod mir;
pub mod mir_interpreter;
pub mod mir_lower;
pub mod mir_const;
pub mod parser;
pub mod semantic;
pub mod span;
pub mod types;

pub use compiler::Compiler;
