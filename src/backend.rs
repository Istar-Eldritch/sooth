//! Backends. QBE is the decided native backend; WASM is a planned sibling lowering
//! off the same neutral IR (via binaryen), not routed through QBE.

pub mod qbe;
