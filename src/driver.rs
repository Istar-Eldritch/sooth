//! Pipeline orchestration: the one place that wires the stages together.

use std::path::Path;

use crate::{backend, check, ir, lexer, parser};

/// Compile a source file to a native binary.
///
/// Planned Phase 0 pipeline:
///   let src = std::fs::read_to_string(path)?;
///   let tokens = lexer::lex(&src)?;
///   let module = parser::parse(&tokens)?;
///   check::check(&module)?;
///   let ir = ir::lower(&module)?;
///   let ssa = backend::qbe::emit(&ir)?;
///   // write `ssa` to a temp .ssa, run `qbe` -> .s, run `cc` -> binary
pub fn build(_path: &Path) -> Result<(), String> {
    Err("Phase 0 in progress: source -> IR -> QBE -> native pipeline not implemented yet".into())
}

pub fn run(path: &Path) -> Result<(), String> {
    build(path)?;
    Err("run: not implemented (Phase 0)".into())
}

pub fn repl() -> Result<(), String> {
    Err("repl: not implemented (Phase 1)".into())
}
