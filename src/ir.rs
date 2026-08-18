//! Backend-neutral IR.
//!
//! The compile-time virtual stack is lowered to SSA-shaped values here, and each
//! word becomes a function taking N inputs and returning M outputs. Control words
//! become basic blocks and branches. This IR feeds QBE today and a WASM sibling
//! lowering later, so it stays neutral: in particular `Ptr` is an opaque handle,
//! never assumed to be a native `u64`, so QBE (native pointers) and WASM
//! (linear-memory offsets) can each concretise it.

use std::collections::{HashMap, HashSet};
use std::mem;

use crate::ast::{
    generic_surface_name, ArrayDecl, ArrayId, CallInst, Clause, EnumDecl, EnumId, GenericTypes,
    Len, Module, OwnedCellDecl, OwnedCellId, PolySig, PolyType, QuotEffect, RefDecl, RefId, Span,
    StackEffect, StaticDecl, StaticInit, StructDecl, StructId, Subst, Term, TermKind, Type,
    TypedSlot, VariantTag, VariantTagMode, WordBody, WordDef,
};

mod destructors;
mod driver;
mod func_builder;
mod layout;
mod types;

pub(crate) use self::destructors::synthesize_aggregate_destructors;
use self::destructors::PathStep;
use self::func_builder::{lower_materialized, lower_word_parts, word_ret_ty, EnvPlan, FuncBuilder};

use self::types::QuotId;
pub use self::types::{
    ir_type_of, quot_input_slots, quotation_layout, Arity, BinOp, Block, BlockId, CmpOp, Instr,
    IrFunc, IrModule, IrType, QuotSigId, QuotSigLayout, Resolver, StaticData, StaticValue,
    Terminator, Value, ALLOC_SYMBOL, FREE_SYMBOL, OOB_TRAP_SYMBOL, TRACE_ALLOC_ENV, WORD_WIDTH,
};

pub(crate) use self::layout::{
    build_registries, build_statics, carried_slot_bytes, empty_statics, ArrayLayout, Arrays, Cells,
    DropOverride, DropOverrides, EnumLayout, Enums, FieldLayout, Refs, Registries, Statics,
    StructLayout, Structs,
};
// `VariantLayout` has no non-test caller anywhere in the crate today (only a
// `repl.rs` test constructs one); it is still part of the historical `ir::*`
// re-export contract (spec surface list), so it stays reachable for tests
// without tripping `unused_imports` on a plain (non-test) build.
pub(crate) use self::driver::{collect_quot_sigs, lower_instantiation, lower_word};
pub use self::driver::{lower, lower_line};
#[cfg(test)]
pub(crate) use self::layout::VariantLayout;
use self::layout::{
    cell_drop_symbol, enum_drop_symbol, field_is_linear, scalar_size_align, struct_drop_symbol,
    EnumWord, StructWord,
};

/// A shared empty instantiation table for lowering paths with no polymorphic
/// call sites (the REPL, D2; destructor synthesis; unit tests), so
/// `FuncBuilder::new` can hand out a valid reference without every caller
/// threading one.
fn empty_instantiations() -> &'static HashMap<Span, CallInst> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, CallInst>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// Slice 8a phase 2: the builtin-overload companion of `empty_instantiations`,
/// handed to every lowering path with no user builtin overloads (the REPL,
/// destructor synthesis, unit tests, and every corpus program).
fn empty_builtin_overloads() -> &'static HashMap<Span, String> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, String>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// P7 slice 1 (R2): the resolved-field companion of `empty_instantiations`,
/// handed to every lowering path that resolved no field projection
/// (destructor synthesis and unit tests).
pub(crate) fn empty_resolved_fields() -> &'static HashMap<Span, (StructId, usize)> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, (StructId, usize)>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// Phase 6 slice 3 (R6): the resolved-variant-field companion of
/// `empty_resolved_fields`, handed to every lowering path that resolved no
/// variant-field projection (destructor synthesis and unit tests).
pub(crate) fn empty_resolved_variant_fields() -> &'static HashMap<Span, (EnumId, usize, usize)> {
    static EMPTY: std::sync::OnceLock<HashMap<Span, (EnumId, usize, usize)>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// The poly-arity companion of `empty_instantiations`.
fn empty_poly_arities() -> &'static HashMap<String, usize> {
    static EMPTY: std::sync::OnceLock<HashMap<String, usize>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

/// R19: the combinator-body companion of `empty_instantiations`. A path with
/// no monomorphic quotation-taking words to inline (the REPL, D2; destructor
/// synthesis; unit tests) hands out this empty map.
fn empty_combinators() -> &'static crate::check::CombinatorIndex {
    static EMPTY: std::sync::OnceLock<crate::check::CombinatorIndex> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

#[cfg(test)]
mod test_helpers;
