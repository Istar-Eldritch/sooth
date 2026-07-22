//! Backend-neutral IR.
//!
//! The compile-time virtual stack is lowered to SSA-shaped values here, and each
//! word becomes a function taking N inputs and returning M outputs. Control words
//! become basic blocks and branches. This IR feeds QBE today and a WASM sibling
//! lowering later, so it stays neutral: in particular `Ptr` is an opaque handle,
//! never assumed to be a native `u64`, so QBE (native pointers) and WASM
//! (linear-memory offsets) can each concretise it.

use crate::ast::Module;

#[derive(Debug, Default)]
pub struct IrModule {
    pub funcs: Vec<IrFunc>,
}

#[derive(Debug)]
pub struct IrFunc {
    pub name: String,
    // blocks, params, results: filled in during Phase 0.
}

/// Opaque pointer handle. Concretised per backend; do not assume a width here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ptr(pub u32);

pub fn lower(_module: &Module) -> Result<IrModule, String> {
    todo!("Phase 0: lower AST -> backend-neutral IR")
}
