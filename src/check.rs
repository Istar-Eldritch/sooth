//! Stack-effect checker. Phase 0: arity only; type unification arrives in Phase 2.
//!
//! Simulates the compile-time virtual stack through each word body and verifies
//! the net effect matches the declared signature, unifying branch/loop join points.
//! Mismatched depth across branches is a compile error (Forth's silent-underflow
//! failure mode becomes a diagnostic here).

use crate::ast::Module;

pub fn check(_module: &Module) -> Result<(), String> {
    todo!("Phase 0: stack-effect (arity) checker")
}
