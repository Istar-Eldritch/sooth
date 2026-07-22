//! QBE backend: emit QBE IL text from the neutral IR.
//!
//! Driver then pipes this through `qbe` (-> assembly) and `cc` (-> native binary).
//! QBE gives arm64/x86_64/riscv64 and C-ABI struct classification for free; costs
//! accepted are i128 synthesised in the frontend and atomics via C11 FFI.

use crate::ir::IrModule;

pub fn emit(_ir: &IrModule) -> Result<String, String> {
    todo!("Phase 0: emit QBE IL")
}
