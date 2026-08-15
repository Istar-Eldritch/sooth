//! Stack-effect checker. Simulates a compile-time virtual stack of concrete
//! `Type`s through each word body and verifies the net effect matches the
//! declared signature.
//!
//! Every operand is checked against the type its consumer expects, so a
//! `bool` where `+` wants an `i64` is a located compile error (Forth's silent
//! coercion failure mode becomes a diagnostic here). Branch join points unify
//! on both depth and per-slot type: the `then` and `else` arms must leave the
//! same stack shape.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    instantiation_symbol, intern_array_type, intern_bundle_struct, intern_owned_cell_type,
    intern_ref_type, ArrayDecl, Bound, CallInst, Clause, EnumDecl, EnumId, ExternDecl, Len, Module,
    ModuleInfo, OwnedCellDecl, PolySig, PolyType, QuotEffect, RefDecl, Span, StackEffect,
    StructDecl, StructId, Subst, Term, TermKind, Type, TypedSlot, VariantDecl, WordBody, WordDef,
};

mod audits;
mod builtins;
mod captures;
mod combinators;
mod declarations;
mod drop_graph;
mod engine;
mod operators;
mod poly;
mod terms;
mod word_entry;
mod word_families;

use self::audits::*;
pub(crate) use self::audits::{
    audit_quotation_type_registries, audit_word_quotation_positions, drop_overload_struct_id,
    find_drop_overloads,
};
pub use self::builtins::builtin_table;
use self::builtins::*;
pub(crate) use self::builtins::{is_copy, is_linear, sig_of, Overload, Sig, COMPARISON_PRIMITIVES};
use self::captures::*;
pub use self::combinators::is_combinator;
use self::combinators::*;
pub(crate) use self::combinators::{
    check_combinator_cycles, combinator_index, combinator_of, word_declares_quotation_parameter,
    CombinatorEnv, CombinatorIndex,
};
pub use self::declarations::check_structs;
use self::declarations::*;
pub(crate) use self::declarations::{
    check_exported_signatures, check_selective_imports, check_types, enum_generated_sigs,
    selective_not_exported_error, struct_generated_sigs, SelectiveName,
};
use self::drop_graph::*;
pub(crate) use self::drop_graph::{
    check_drop_overload_reachability, has_self_tail_call, terms_tail_call_self,
};
use self::engine::*;
use self::operators::*;
use self::poly::*;
pub(crate) use self::poly::{check_poly_body, check_poly_combinator_repl, poly_type_str};
use self::terms::check_terms;
use self::terms::check_terms_relaxed;
pub(crate) use self::word_entry::{
    check_inline_declaration, check_inline_quotation_requires_inline,
};
use self::word_entry::{check_reference_free_signature, check_word};
use self::word_families::*;

/// Slice 8a fix 1 (R1): one candidate registered under a name that may carry
/// more than one -- an overload set. The word env's value type widened from a
/// single `Sig` to `Vec<Overload>` so a name with several same-arity,
/// differing-input-type candidates (R1/R4 already guarantee at most one can
/// match a given call's operand types) keeps every one of them reachable,
/// rather than the env's old bare `HashMap<String, Vec<Overload>>` silently keeping
/// only the last inserted. `symbol` is the distinct lowering symbol this
/// candidate's body was minted under (`ast::overload_symbols`): equal to the
/// surface name unless this name has more than one candidate in scope.
/// The per-call-site records a body walk fills: `CallInst` per polymorphic
/// instantiation (R14) and, since slice 8a, the resolved candidate's lowering
/// symbol per overloaded call (R7). Lowering reads both keyed by `Span`.
type ResolvedCalls = (HashMap<Span, CallInst>, HashMap<Span, String>);

/// `ResolvedCalls` plus the residual stack a REPL line leaves behind.
type InferredLine = (Vec<Type>, HashMap<Span, CallInst>, HashMap<Span, String>);

/// R5/R14: every candidate registered under one polymorphic-word name, its
/// `PolySig` paired with the REPL generation it was retained at (`None` on
/// the native/module path, which has no generations). `check_poly_call`
/// resolves a bare call by trial unification across candidates, the same
/// shape as `Overload`'s exact-match resolution for concrete words -- a
/// name-keyed single-value map here would silently shadow a second
/// polymorphic overload exactly as env's `Sig` did before B1.
pub(crate) type PolyEnv = HashMap<String, Vec<(PolySig, Option<u64>)>>;

/// R5/R14: the polymorphic-call context threaded through the monomorphic body
/// walk: the `PolySig`s of every polymorphic word (looked up before the
/// concrete `env`), and the instantiation table each unified call site writes
/// into. A monomorphic body that never calls a polymorphic word touches
/// neither; the REPL (`infer_line`) passes an empty one, so no `repl.rs`
/// change is needed (D2).
///
/// R2b: each `PolySig` carries its generation alongside it (`None` natively,
/// `Some(g)` for a REPL word retained at generation `g`, Slice 2), so
/// `check_poly_call`'s mint reads both from one lookup with no second
/// channel.
struct PolyCtx<'a> {
    env: &'a PolyEnv,
    insts: &'a mut HashMap<Span, CallInst>,
    /// Slice 8a phase 2 (R7): the call sites this walk resolved to a user
    /// overload of a builtin-named word (`Vec2 +` -> the user `+`), span ->
    /// resolved callee name, relayed onto `Module::builtin_overloads` so
    /// lowering emits an `Instr::Call` there instead of the builtin
    /// instruction. Scratch (discarded) on the REPL/combinator paths, which do
    /// not lower a builtin overload (out of scope this slice).
    builtin_overloads: &'a mut HashMap<Span, String>,
    /// Slice 6a (R18): the monomorphic quotation-taking words, keyed by name,
    /// so a call to one is intercepted and its body spliced against the live
    /// stack (the compiler's only inliner) rather than lowered to an
    /// `Instr::Call` to a word that mints no `IrFunc` (R20). Empty on the REPL
    /// paths, where defining such a word is rejected up front (R23).
    combinators: &'a CombinatorEnv<'a>,
}

/// One simulated stack slot: its concrete `Type`, plus whether it is a bare,
/// as-yet-unconverted integer literal fresh off an `IntLit` term. `Type`
/// alone can't express D8's literal-coercion carve-out (an integer literal
/// unifies with a `usize` position without an explicit `>usize`, but a
/// *computed* `i64` may not, X10), so the checker's internal stack carries
/// this flag alongside every `Type` it already tracked. It never escapes
/// `check.rs`: every external-facing function (`infer_line`, `check_outputs`'
/// callers) still speaks plain `Type`. A shuffle (`dup`/`swap`/`over`/`rot`)
/// moves a `Slot` verbatim, so a literal duplicated by `dup` is still a
/// literal at each copy; any operator, conversion, or word call produces a
/// non-literal result (D8: no constant folding, no comptime interpreter).
/// An index into a per-check `Provenance::quotations` table (D2): a
/// quotation `Slot` carries the identity of the literal body it marks, so
/// `call`/`times` can splice that body at the consumption site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QuotId(usize);

/// The identity a quotation `Slot` carries (D2/R4). A single variant: two
/// *different* quotations at a branch join are rejected at the join (R7), so
/// no poisoned/merged marker is ever carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotRef {
    Known(QuotId),
}

/// 7b/R19: a `Copy` handle into `Provenance::surviving_sets`, the side table
/// of capture sets that outlive erasure. When a capturing literal materialises
/// at a boundary its `QuotRef::Known` marker is dropped (`quot: None`), so the
/// aggregate/borrow captures whose referents must stay live past the call ride
/// this id on the erased `Slot`/`Binding` instead. `Copy` (a `u32`, exactly as
/// `QuotId`), so it does not cost `Slot` its `Copy` derive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurvivingCaptureSetId(u32);

/// 7b/R19: one member of a surviving capture set -- a captured aggregate-value
/// or borrow name whose referent must outlive the closure's calls. A scalar
/// snapshot is never a member (D4 amendment: a snapshot has no referent that
/// can go dead). `frame_rooted` is the R15 classification: a capture rooted in
/// a current-frame local (its storage dies at return), driving the R22
/// word-output escape guard.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SurvivingCapture {
    name: String,
    frame_rooted: bool,
}

/// 7b/R19 + review fix: one interned surviving capture set. `bundle` is R16's
/// env-shape signal -- 2+ *total* captures (scalar or not) build a
/// stack-allocated bundle rather than an inline single-word env -- carried
/// separately from `members`, because a scalar+reference bundle has only one
/// surviving member: member count alone cannot recover it. Drives the R22
/// word-output escape guard for a carrier whose closure needed a bundle,
/// independent of any member's `frame_rooted` classification (the bundle
/// storage itself is frame-local even when every capture it holds is
/// outer-rooted).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SurvivingSet {
    members: Vec<SurvivingCapture>,
    bundle: bool,
}

/// One interned quotation literal: its body terms (spliced at `call`/`times`),
/// the literal's span (for a located diagnostic), and its own source flavour
/// (Slice 12, R-C1/R-C2): `true` for a `~[ ... ]` literal, `false` for an
/// ordinary `[ ... ]`. Checked against the consuming parameter's declared
/// flavour at each argument-matching site.
#[derive(Debug, Clone)]
struct QuotBody {
    body: Vec<Term>,
    #[allow(dead_code)]
    span: Span,
    is_inline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    ty: Type,
    literal: bool,
    /// The integer value of a bare `IntLit` slot (`None` for any computed
    /// value). Load-bearing for the two compile-time-count array positions:
    /// `fill`'s count `N` (M1) and a constant-index bounds check (X4, R11).
    /// Moved verbatim by a shuffle (a duped literal keeps its value), cleared
    /// by any operator/conversion/word call or branch merge (D8: no folding).
    int_val: Option<i64>,
    /// Which region this aggregate value denotes, and where this name for
    /// it was pushed.
    alias: Option<Alias>,
    /// The outstanding derivation a reference-typed value holds.
    deriv: Option<DerivId>,
    /// D2/R4: set iff this is a quotation marker, carrying the identity of the
    /// literal body it stands for. A `Cstr` placeholder `ty` no user op
    /// accepts rides alongside it; a shuffle forwards this verbatim (`Slot` is
    /// `Copy`), and `call`/`times` consume it by splicing the body.
    quot: Option<QuotRef>,
    /// 7b/R19: the surviving capture set of an *erased* capturing quotation
    /// (`quot: None`, `ty == Type::Quotation`), or of an aggregate carrying
    /// one (a struct/array field holds a stored closure). `None` for every
    /// non-quotation, non-carrier value. Forwarded verbatim by a shuffle
    /// (`Slot` is `Copy`) and across a bind, exactly like `quot`.
    surviving: Option<SurvivingCaptureSetId>,
}

impl Slot {
    /// A slot holding a computed (non-literal) value of `ty`: every path but
    /// a bare `IntLit` push produces one of these.
    fn computed(ty: Type) -> Slot {
        Slot {
            ty,
            literal: false,
            int_val: None,
            alias: None,
            deriv: None,
            quot: None,
            surviving: None,
        }
    }

    /// The same value, reached through a reference derived from `deriv`: a
    /// projection's result, which keeps its parent's provenance so the
    /// place it traces back to stays findable however many steps away it is.
    fn derived(ty: Type, deriv: Option<DerivId>) -> Slot {
        Slot {
            deriv,
            ..Slot::computed(ty)
        }
    }
}

/// Whether `ty` is one of the two target-width size types (`usize`/`isize`):
/// both share the D8 literal-coercion carve-out against a bare `i64`
/// literal, so `match_slot`/`unify_pair` gate on this rather than on `Usize`
/// alone. `usize` and `isize` never coerce into *each other* here: the guard
/// only ever fires against `Type::I64`, so mixing the two size types falls
/// through to a plain mismatch, naming both backticked types.
fn is_size_type(ty: Type) -> bool {
    matches!(ty, Type::Usize | Type::Isize)
}

/// The outcome of matching one `Slot` against a single expected `Type`
/// (a word-call argument, a declared output slot, or a binary operator's
/// second operand once the first has picked a target type): exact, D8's
/// literal coercion into a `usize`/`isize` position, the specific "needs an
/// explicit conversion" diagnostic (X10) for a *computed* value in that
/// position, or a plain mismatch.
enum SlotMatch {
    Exact,
    LiteralSizeType,
    NeedsSizeConversion,
    /// R7: a `str` found where a `cstr` is wanted. Never coerces silently
    /// (there is no implicit conversion, only the explicit `cstr` word), so
    /// this is its own case rather than falling into `Mismatch`, exactly as
    /// `NeedsSizeConversion` is split from a plain mismatch above.
    NeedsStrToCstrConversion,
    Mismatch,
}

fn match_slot(found: Slot, want: Type) -> SlotMatch {
    if found.ty == want {
        return SlotMatch::Exact;
    }
    if is_size_type(want) && found.ty == Type::I64 {
        return if found.literal {
            SlotMatch::LiteralSizeType
        } else {
            SlotMatch::NeedsSizeConversion
        };
    }
    if want == Type::Cstr && found.ty == Type::Str {
        return SlotMatch::NeedsStrToCstrConversion;
    }
    SlotMatch::Mismatch
}

/// The result of unifying two `Slot`s for a homogeneous binary operator
/// (`+ - * = < > <= >= <> mod and or xor`): the operands' common `Type` once
/// D8's literal coercion is applied (a `usize`/`isize` paired with a bare
/// integer literal unifies to that size type), the X10 diagnostic's target
/// type for a size type paired with a *computed* `i64` instead, or a plain
/// mismatch.
enum PairMatch {
    Ok(Type),
    NeedsSizeConversion(Type),
    Mismatch,
}

fn unify_pair(a: Slot, b: Slot) -> PairMatch {
    if a.ty == b.ty {
        return PairMatch::Ok(a.ty);
    }
    match (a.ty, b.ty) {
        (w, Type::I64) if is_size_type(w) && b.literal => PairMatch::Ok(w),
        (Type::I64, w) if is_size_type(w) && a.literal => PairMatch::Ok(w),
        (w, Type::I64) | (Type::I64, w) if is_size_type(w) => PairMatch::NeedsSizeConversion(w),
        _ => PairMatch::Mismatch,
    }
}

/// D3 (slice 6h phase 2): whether `ty` transitively contains a pointer-shaped
/// `Copy` type the array constructor cannot safely zero-initialize --
/// `Type::Str`, `Type::Cstr`, `Type::Quotation` -- recursing through struct
/// fields, ALL enum variant fields (conservative: only variant 0's payload is
/// readable in an all-zero value, but any variant's pointer-shaped payload is
/// rejected rather than argue a subtle tag-gating case with no known use),
/// and array elements. Returns the offending inner type and the path to it
/// (outermost first) on a hit. `fill` never calls this: it replicates a real
/// seed and never mints one from zeroed memory, so it keeps accepting these
/// types (D4).
fn find_zero_unsafe_element(
    ty: Type,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Option<(Type, Vec<String>)> {
    match ty {
        Type::Str | Type::Cstr | Type::Quotation(_) => Some((ty, Vec::new())),
        Type::Struct(id, _) => {
            for (fname, fty) in &structs[id.index()].fields {
                if let Some((bad, mut path)) =
                    find_zero_unsafe_element(*fty, structs, enums, arrays)
                {
                    path.insert(0, format!("field `{fname}`"));
                    return Some((bad, path));
                }
            }
            None
        }
        Type::Enum(id, _) => {
            for variant in &enums[id.index()].variants {
                for (fname, fty) in &variant.fields {
                    if let Some((bad, mut path)) =
                        find_zero_unsafe_element(*fty, structs, enums, arrays)
                    {
                        path.insert(0, format!("variant `{}` field `{fname}`", variant.name));
                        return Some((bad, path));
                    }
                }
            }
            None
        }
        Type::Array(id, _) => {
            find_zero_unsafe_element(arrays[id.index()].element, structs, enums, arrays).map(
                |(bad, mut path)| {
                    path.insert(0, "array element".to_string());
                    (bad, path)
                },
            )
        }
        _ => None,
    }
}

/// D2 (slice 6h phase 2): the shared type-directed gate for a construction
/// site that accepts a bare `Type` with no declaration for
/// `check_no_stored_references` to have caught -- `fill`'s element and the
/// array constructor's element. Owns exactly the checks that read a `Type`
/// (never a `Slot`): no stored reference, `Copy`, and (when `zero_safety` is
/// set, the constructor only) D3's zero-validity predicate. It does not own
/// the quotation or literal-count checks (`Slot` fields) or the count range
/// check, which stay at `fill`'s own call site. `site` names the
/// construction site and drives both diagnostics that need one:
/// `fill_of_linear_element_error` renders it as a bare code span, and this
/// composes `constructed_reference_error`'s noun phrase from it the same way
/// -- `fill` passing `"fill"` keeps both byte-identical to before this gate
/// existed.
#[allow(clippy::too_many_arguments)]
fn check_array_element_gate(
    ctx: &Ctx,
    span: Span,
    site: &str,
    element: Type,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    zero_safety: bool,
) -> Result<(), String> {
    if contains_reference(element, structs, enums, arrays) {
        return Err(constructed_reference_error(
            ctx,
            span,
            &format!("the element `{site}` would store"),
            element,
        ));
    }
    if !is_copy(element, structs, enums, arrays) {
        return Err(fill_of_linear_element_error(ctx, span, element, site));
    }
    if zero_safety {
        if let Some((bad, path)) = find_zero_unsafe_element(element, structs, enums, arrays) {
            return Err(array_constructor_zero_unsafe_element_error(
                ctx, span, element, bad, &path,
            ));
        }
    }
    Ok(())
}

pub fn check(module: &mut Module) -> Result<(), String> {
    // R1: recognized ahead of `check_types` so the ordering hazard against
    // `check_recursion` (run inside `check_types`) never arises.
    let drop_overloads = find_drop_overloads(&module.words, &module.structs)?;
    let drop_overload_indices: HashSet<usize> = drop_overloads.values().copied().collect();
    // R3: defining `drop` for a struct forces it linear, so the fact is
    // recorded on the declaration itself rather than re-derived: every
    // `is_copy` call site, `ir`'s layout fold, and the REPL's persistent
    // registries all read the same `StructDecl`.
    for id in drop_overloads.keys() {
        module.structs[id.index()].has_drop_overload = true;
    }

    check_types(
        &module.structs,
        &module.enums,
        &module.arrays,
        &module.owned_cells,
    )?;

    // R7a: a quotation type is legal only as a direct word parameter this
    // slice; reject it at every other position before layout or lowering can
    // see it, so R7's `unreachable!` mangling/`IrType` arms stay unreached.
    audit_quotation_type_positions(module)?;

    // Builtins are resolved by table (`BUILTIN_TABLE`) inside `check_operator`,
    // not by env lookup, so the concrete env holds only user/generated words.
    let mut env: HashMap<String, Vec<Overload>> = HashMap::new();
    for (name, sig) in struct_generated_sigs(&module.structs) {
        let symbol = name.clone();
        env.insert(name, vec![Overload { sig, symbol }]);
    }
    for (name, sig) in enum_generated_sigs(&module.enums) {
        let symbol = name.clone();
        env.insert(name, vec![Overload { sig, symbol }]);
    }

    // R1: an `extern:` declaration is registered into the same word
    // environment as any other word, so every existing arity/type check
    // applies to its call sites unchanged; but first, R1's redeclaration
    // rule and R2/R3's boundary-type rules are checked at the declaration.
    check_extern_decls(
        &module.externs,
        &module.words,
        &env,
        &module.structs,
        &module.enums,
        &module.arrays,
    )?;
    for decl in &module.externs {
        let symbol = decl.name.clone();
        env.insert(
            decl.name.clone(),
            vec![Overload {
                sig: sig_of(&decl.effect),
                symbol,
            }],
        );
    }

    // A duplicate word name in one module is rejected here, before the
    // population loop below would otherwise silently keep only the last one
    // seen and let both bodies reach codegen.
    check_duplicate_word_names(&module.words)?;
    // R5: a generic candidate overlapping a concrete one of the same name and
    // arity (a builtin row or a local monomorphic word) is rejected here too,
    // before either enters `poly_env`/`env` below -- there is no ranking that
    // could otherwise pick between them.
    check_generic_concrete_overlap(&module.words)?;
    // Two poly words (or two poly combinators) declaring the exact same
    // signature under one name are rejected before either enters `poly_env`
    // below -- unresolvable ambiguity, not a legitimate second overload.
    check_duplicate_poly_signatures(&module.words)?;

    // R1: a recognized `drop` overload is excluded from the ordinary word
    // environment -- registering it under the literal name `"drop"` would be
    // either dead (`check_shuffle`'s `"drop"` arm intercepts every call site
    // first) or, for a second overload, a name collision the checker has no
    // reason to reject, since dispatch never goes through this table.
    //
    // R5: a polymorphic word never enters the concrete `env` (its inputs are
    // not concrete `Sig` types); it lives in `poly_env` instead, and a call
    // site is intercepted there before the concrete lookup, where its
    // `PolySig` is unified against the concrete stack.
    let mut poly_env: PolyEnv = HashMap::new();
    // Slice 8a fix 1 (R1): each word's distinct lowering symbol, aligned by
    // index -- equal to its own name unless it shares that name with another
    // word in this module (an overload set), in which case each candidate's
    // `Overload::symbol` diverges from the bare name it is looked up under.
    let symbols = crate::ast::overload_symbols(&module.words);
    for (idx, word) in module.words.iter().enumerate() {
        if drop_overload_indices.contains(&idx) {
            continue;
        }
        if let Some(sig) = &word.poly {
            poly_env
                .entry(word.name.clone())
                .or_default()
                .push(((**sig).clone(), None));
        } else {
            env.entry(word.name.clone()).or_default().push(Overload {
                sig: sig_of(&word.effect),
                symbol: symbols[idx].clone(),
            });
        }
    }

    check_main_effect(
        &module.words,
        &module.structs,
        &module.enums,
        &module.arrays,
    )?;

    // Split the borrow so a word body can intern into `arrays`/`owned_cells`
    // while reading `words`/`enums`/`structs`.
    let Module {
        words,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        externs: _,
        instantiations: _,
        builtin_overloads: _,
        modules,
    } = module;
    // R6: each body's own `drop` call sites, resolved to a concrete operand
    // type by the walk that checks it. Collected per word so the graph below
    // knows which body each site sits in.
    let mut dropped: Vec<Vec<Type>> = Vec::with_capacity(words.len());
    // Slice 11 (R3): reject a declared `inline` the splice cannot deliver on
    // *before* `is_combinator` is consulted, so a clause-bodied or
    // variable-bearing `inline` word never enters the splice env, the cycle
    // graph, or the poly checker (which would take it for a legitimate poly
    // combinator).
    for word in words.iter() {
        check_inline_declaration(word)?;
        check_inline_quotation_requires_inline(word)?;
    }
    // R18: the monomorphic quotation-taking words, gathered once so a call to
    // one is intercepted and inlined (term-splice) rather than lowered to a
    // call. A polymorphic combinator's body is checked by the poly pass, so it
    // is not registered here; only a `WordBody::Terms` monomorphic word with a
    // `Type::Quotation` input qualifies.
    let combinators = collect_combinators(words);
    // R22 (D5): reject a cycle in the quotation-taking-word call subgraph
    // before any body is checked, so the splice below may assume acyclicity.
    // Ordered *before* `check_tail_call_cycles`: a combinator's call to
    // another combinator is inlined (spliced), never lowered as a tail call,
    // so a combinator cycle is a splice-forever error (this pass), not mutual
    // tail recursion -- running the tail-cycle pass first would misreport a
    // combinator cycle whose edges happen to sit in tail position.
    check_combinator_cycles(&combinators)?;
    // Reject mutual tail-recursion cycles (D3, X1) on the whole-module
    // tail-call graph, after signature registration and before body checking.
    check_tail_call_cycles(words, &drop_overload_indices, combinators.tail())?;
    // R14: the per-call-site instantiation table, filled as each monomorphic
    // body's calls to polymorphic words are unified, then stored on the module
    // for lowering.
    let mut insts: HashMap<Span, CallInst> = HashMap::new();
    // Slice 8a phase 2 (R7): the builtin-name overload dispatch sites, filled
    // as each monomorphic body's operator calls resolve, then relayed to the
    // module for lowering (empty for the whole corpus, so its lowering is
    // untouched byte-for-byte).
    let mut builtin_overloads: HashMap<Span, String> = HashMap::new();
    for word in words.iter() {
        let mut sites = Vec::new();
        if let Some(sig) = &word.poly {
            if is_combinator(word) {
                // R14-R17: a polymorphic combinator (`each`/`map`/`fold`) is
                // checked standalone by instantiating its signature at
                // concrete stand-in types and running the ordinary checker on
                // the body, which already handles the abstract quotation
                // `call`/`times` (R8/R9) and the three `times` obligations
                // (R16). It mints no `IrFunc` (R20): a call to it is inlined
                // by term-splice at its concrete call sites, so the
                // instantiation records it produces here are scratch.
                let mut scratch: HashMap<Span, CallInst> = HashMap::new();
                let mut scratch_overloads: HashMap<Span, String> = HashMap::new();
                let mut poly = PolyCtx {
                    env: &poly_env,
                    insts: &mut scratch,
                    builtin_overloads: &mut scratch_overloads,
                    combinators: &combinators,
                };
                check_poly_combinator_standalone(
                    word,
                    sig,
                    enums,
                    &env,
                    arrays,
                    owned_cells,
                    refs,
                    structs,
                    Some(modules),
                    &mut poly,
                )?;
            } else {
                // R7: a polymorphic body is checked over a `PolyType` stack by
                // a dedicated pass, deliberately separate from the concrete
                // walk.
                check_poly_body(
                    word,
                    sig,
                    &env,
                    structs,
                    enums,
                    arrays,
                    Some(modules),
                    &mut builtin_overloads,
                )?;
            }
        } else {
            let mut poly = PolyCtx {
                env: &poly_env,
                insts: &mut insts,
                builtin_overloads: &mut builtin_overloads,
                combinators: &combinators,
            };
            check_word(
                word,
                enums,
                &env,
                arrays,
                owned_cells,
                refs,
                structs,
                Some(modules),
                &mut sites,
                &mut poly,
            )?;
        }
        dropped.push(sites);
    }

    // R6: only now, with every `drop` call site's operand type known, can the
    // `drop`-reachability graph be built.
    let word_refs: Vec<&WordDef> = words.iter().collect();
    check_drop_overload_recursion(
        &word_refs,
        structs,
        enums,
        arrays,
        owned_cells,
        &drop_overloads,
        &dropped,
    )?;

    // R8/R10: the multi-output return bundles, interned into the same
    // `module.structs` the layout pass reads, so a bundle is laid out (and
    // flagged, so no destructor is synthesized for it) like any other struct.
    // Last, after every type-level check and after `struct_generated_sigs`:
    // a bundle is an ABI detail, not a nameable type, so it takes part in
    // neither name resolution nor generated-word registration.
    intern_output_bundles(module);
    // R8/R14: each polymorphic instantiation whose resolved output count is
    // >= 2 needs the same kind of bundle a monomorphic multi-output word gets,
    // interned into the same `module.structs` (reusing the checker's earlier
    // struct interning) so lowering reads it back like any other struct. The
    // table itself is then handed to lowering on the module.
    for inst in insts.values_mut() {
        if inst.out_arity >= 2 {
            inst.bundle = Some(intern_bundle_struct(
                &mut module.structs,
                &inst.output_types,
            ));
        }
    }
    module.instantiations = insts;
    module.builtin_overloads = builtin_overloads;
    Ok(())
}

/// R10: one interned bundle struct per distinct output tuple of length >= 2,
/// over every declared word. Gated on the output count alone, not on anything
/// about the word: a `drop` overload has no outputs and an `extern:` is
/// rejected above one, so neither reaches this.
fn intern_output_bundles(module: &mut Module) {
    let tuples: Vec<Vec<Type>> = module
        .words
        .iter()
        .filter(|w| w.effect.outputs.len() >= 2)
        .map(|w| w.effect.outputs.iter().map(|s| s.ty).collect())
        .collect();
    for outputs in tuples {
        intern_bundle_struct(&mut module.structs, &outputs);
    }
}

/// Check a single word definition against an external env, seeding the env with
/// the word's own signature so self-recursion type-checks. `enums` is the
/// registry the clause-style checks (coverage, scrutinee type, variant-name
/// collision) consult. Also returns this body's recorded overload-dispatch
/// call sites (item 3), so the REPL definition path can thread them into
/// lowering instead of discarding them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_def(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    poly_env: &PolyEnv,
    combinators: &CombinatorEnv,
) -> Result<ResolvedCalls, String> {
    let (_sites, insts, overloads) = check_def_collecting_drop_sites(
        word,
        enums,
        env,
        arrays,
        cells,
        refs,
        structs,
        poly_env,
        combinators,
    )?;
    Ok((insts, overloads))
}

/// R6/R11: `check_def`'s own body-check, but returning this one word's
/// recorded `drop` call sites instead of discarding them. The REPL keeps the
/// result cached per override (`Session::drop_dropped_sites`) so a later
/// line's reachability query (`check_drop_overload_reachability`) never has
/// to re-check an *earlier* override's body against a *later* line's env --
/// the same stale-env hazard R11.2/R11.3 already fixed for lowering. A
/// `drop` call site's resolved operand type does not change once recorded;
/// only whether that type is *currently* overridden can, and that question
/// is answered fresh, from `structs`, every time the graph is built.
///
/// Item 3 (slice 8a fix): also returns this body's recorded overload-dispatch
/// sites (span -> resolved symbol). `eval_def`'s caller threads them into
/// `ir::lower_word` so a REPL definition dispatches an overloaded call
/// exactly like a native one; `compile_drop_overload` (a `drop` override body)
/// has no such threading yet and discards them, an accepted, narrower gap
/// than this fix's crash (see the item 3 report).
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_def_collecting_drop_sites(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    poly_env: &PolyEnv,
    combinators: &CombinatorEnv,
) -> Result<InferredLine, String> {
    let mut env = env.clone();
    let symbol = word.name.clone();
    env.insert(
        word.name.clone(),
        vec![Overload {
            sig: sig_of(&word.effect),
            symbol,
        }],
    );
    let mut sites = Vec::new();
    // R5 (Slice 2): the session poly-env threads through so a defined word's
    // own body can call a retained polymorphic word; the REPL drop-overload
    // collector passes the empty map (a `drop` overload is never polymorphic),
    // keeping the reachability walk byte-identical on the concrete path (D2).
    let mut insts: HashMap<Span, CallInst> = HashMap::new();
    // Item 3: this body's resolved-overload call sites, relayed to the
    // caller so lowering can dispatch through them instead of
    // `empty_builtin_overloads()`.
    let mut overloads: HashMap<Span, String> = HashMap::new();
    // R3 (Slice 6c): the session's retained combinators thread through so a
    // defined word's body can call one and have it inlined, exactly as native
    // inlines one drawn from `module.words`. The build path and unit tests
    // pass the empty map, keeping the concrete path byte-identical.
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut insts,
        builtin_overloads: &mut overloads,
        combinators,
    };
    // R8 (slice 8b): a REPL-defined word body has no `ModuleInfo` view, so the
    // `drop` import-visibility gate never fires on the session path.
    check_word(
        word, enums, &env, arrays, cells, refs, structs, None, &mut sites, &mut poly,
    )?;
    Ok((sites, insts, overloads))
}

/// Infer the net effect of a bare line: simulate the typed stack from
/// `entry_stack` (the carried slot types) and return the resulting typed stack.
/// A type mismatch or underflow against the carried stack is a reported error.
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_line(
    terms: &[Term],
    entry_stack: &[Type],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    poly_env: &PolyEnv,
    combinators: &CombinatorEnv,
) -> Result<InferredLine, String> {
    let initial: Vec<Slot> = entry_stack.iter().map(|ty| Slot::computed(*ty)).collect();
    // A line is one block: names it binds die with it, so its end is a scope
    // end like any other. It is not a word body, so nothing in it is in tail
    // position.
    let ctx = Ctx::Line { structs, enums };
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    // R5 (Slice 2): the session poly-env threads through so a bare line can
    // call a retained polymorphic word; the filled instantiation table is
    // relayed to the caller for lowering. A `build`-path caller passes the
    // empty map (Slice 1's D2 behaviour).
    let mut insts: HashMap<Span, CallInst> = HashMap::new();
    // Item 3: this line's resolved-overload call sites, relayed to the
    // caller so lowering can dispatch through them instead of
    // `empty_builtin_overloads()`.
    let mut overloads: HashMap<Span, String> = HashMap::new();
    // R3 (Slice 6c): the session's retained combinators thread through so a
    // bare line can call one and have it inlined, exactly as native inlines one
    // drawn from `module.words`. The build path and unit tests pass empty.
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut insts,
        builtin_overloads: &mut overloads,
        combinators,
    };
    let final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, &mut prov, &mut scope, false, &mut poly,
    )?;
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))?;
    // R19: a REPL line has no declared outputs (so R10's route never runs),
    // yet the session carries its residual stack into the next line while the
    // `quot` side channel dies at the boundary and lowering has pushed a
    // phantom the spill would marshal. Reject a quotation left here.
    if final_stack.iter().any(|s| s.quot.is_some()) {
        return Err(
            "error: a quotation cannot be left on the stack at the end of a line: the session carries it into the next line, and only `call` accepts a quotation (a runtime quotation value exists since slice 7a/7b, but the REPL line boundary is not yet a materialization boundary, so nothing has been pushed for the session to carry)".to_string(),
        );
    }
    // The sixth position of the no-stored-reference rule: the session's
    // inter-line stack outlives this line's
    // locals, so a reference that survived to here would outlive its referent.
    if let Some(slot) = final_stack
        .iter()
        .find(|s| contains_reference(s.ty, structs, enums, arrays))
    {
        return Err(format!(
            "error: a reference cannot be stored: the line leaves `{}` on the stack, which the session carries into the next line\n  a `&T`/`&!T` borrows a local of this line, and this line's locals are gone by then",
            slot.ty
        ));
    }
    Ok((
        final_stack.into_iter().map(|s| s.ty).collect(),
        insts,
        overloads,
    ))
}

fn effect_str(effect: &StackEffect) -> String {
    let ins: Vec<String> = effect.inputs.iter().map(|s| s.ty.to_string()).collect();
    let outs: Vec<String> = effect.outputs.iter().map(|s| s.ty.to_string()).collect();
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

/// Whether `name` is a registered variant name of any enum (the D8 backstop's
/// lookup set).
fn is_registered_variant(name: &str, enums: &[EnumDecl]) -> bool {
    enums
        .iter()
        .any(|e| e.variants.iter().any(|v| v.name == name))
}

/// A parameter / word-entry / clause-body binding name equal to a registered
/// variant name is a sharp error (D8 backstop, X12): it would make the
/// clause-vs-locals `|` disambiguation ambiguous.
fn reject_variant_local(ctx: &Ctx, name: &str, kind: &str) -> Result<(), String> {
    if !is_registered_variant(name, ctx.enums()) {
        return Ok(());
    }
    Err(match ctx {
        Ctx::Word {
            name: word_name, ..
        } => format!(
            "error: {kind} `{name}` in `{word_name}` collides with the variant name `{name}`"
        ),
        Ctx::Line { .. } => {
            format!("error: {kind} `{name}` collides with the variant name `{name}`")
        }
    })
}

/// D5: the diagnostic for a local whose name collides with a callable name —
/// a builtin, a user word, a poly word, or a combinator. `alpha_rename_locals`
/// (`src/ast.rs`) renames a callee's locals but `rename_call` deliberately
/// leaves a call to a word or builtin untouched, so a caller local sharing a
/// builtin's name is read in place of that builtin inside a spliced body
/// (recon 10). This closes it at the root: it is never legal to bind such a
/// name, on either the mono or poly path, so no splice can ever observe one.
/// Residual gap: a bare-name *selectively imported* foreign word is keyed
/// under the exporting module's mangled form, not the importing module's, so
/// it still isn't caught here (same class as the recorded operator-overload
/// module-scoping gap).
fn callable_local_error(ctx: &Ctx, name: &str, span: Span) -> String {
    match ctx {
        Ctx::Word {
            name: word_name, ..
        } => format!(
            "error: local `{name}` in `{word_name}` collides with the callable name `{name}` (line {})\n  a local cannot shadow a builtin, word, poly word, or combinator name",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: local `{name}` collides with the callable name `{name}` (line {})\n  a local cannot shadow a builtin, word, poly word, or combinator name",
            span.line
        ),
    }
}

/// A name repeated in a binding list (`| a a |`) collapses to last-wins when
/// zipped into the name -> type map, so the earlier binding (and any linear
/// value held in it) is tracked by nothing and never disposed. Reject
/// unconditionally, regardless of the bound type.
fn reject_duplicate_local<'a>(
    ctx: &Ctx,
    name: &'a str,
    span: Span,
    seen: &mut HashSet<&'a str>,
) -> Result<(), String> {
    if seen.insert(name) {
        return Ok(());
    }
    Err(match ctx {
        Ctx::Word {
            name: word_name, ..
        } => format!(
            "error: duplicate local `{name}` in `{word_name}` (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: duplicate local `{name}` (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        ),
    })
}

/// The output-count / output-type mismatch check shared by a term body and a
/// clause body (M6, X8): `final_stack` must match the declared outputs.
/// Honors D8's literal coercion (a bare integer literal satisfies a declared
/// `usize` output) and reports the X10 diagnostic for a computed one.
fn check_outputs(
    word: &WordDef,
    final_stack: &[Slot],
    declared: &[Type],
    line: u32,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    // R10: a quotation left on the exit stack gets its own diagnostic, ahead
    // of both the arity and type-mismatch routes. On a *matching* count the
    // ordinary mismatch would otherwise fire and leak the `Cstr` placeholder
    // spelling; a quotation cannot be a declared output regardless of count.
    if final_stack.iter().any(|s| s.quot.is_some()) {
        return Err(format!(
            "error: `{}` (line {}) leaves a quotation on the stack; a quotation cannot be a declared output",
            word.name, line
        ));
    }
    if final_stack.len() != declared.len() {
        // R13/R2: a *linear* surplus value is the forgotten-disposal case, so it
        // gets the disposal wording (and names its type) before the generic
        // arity error a surplus Copy value keeps.
        if let Some(slot) = final_stack
            .get(declared.len()..)
            .unwrap_or_default()
            .iter()
            .find(|s| is_linear(s.ty, structs, enums, arrays))
        {
            return Err(surplus_linear_value_error(word, slot.ty, line));
        }
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  body leaves {} values, but ( … ) declares {} outputs\n  note: declared {}",
            crate::resolve::demangle_word(&word.name), line, final_stack.len(), declared.len(), effect_str(&word.effect),
        ));
    }
    for (found, want) in final_stack.iter().zip(declared) {
        match match_slot(*found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            SlotMatch::NeedsSizeConversion => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves a computed `i64` where the declaration requires `{}`: convert it explicitly with `>{}` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
                    word.name, line, want, want, effect_str(&word.effect),
                ));
            }
            SlotMatch::NeedsStrToCstrConversion => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves `str` where the declaration requires `cstr`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)\n  note: declared {}",
                    word.name, line, effect_str(&word.effect),
                ));
            }
            SlotMatch::Mismatch => {
                return Err(format!(
                    "error: type mismatch in `{}` (line {})\n  body leaves `{}` where the declaration requires `{}`\n  note: declared {}",
                    word.name, line, found.ty, want, effect_str(&word.effect),
                ));
            }
        }
    }
    Ok(())
}

/// A word's location, derived from the first term (or clause) of its body,
/// for locating a whole-word diagnostic like X1.
pub(crate) fn word_span(word: &WordDef) -> Span {
    word.span
}

fn unknown_word_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown word `{}` in `{}` (line {})",
            name, wname, span.line
        ),
        Ctx::Line { .. } => format!("error: unknown word `{name}`"),
    }
}

/// R3: no candidate of an overloaded name accepts the operands on the stack.
/// Names every candidate's inputs, since the useful question at this call site
/// is which shapes the name does accept.
fn no_overload_matches_error(ctx: &Ctx, span: Span, name: &str, candidates: &[Overload]) -> String {
    let name = crate::resolve::demangle_call(name);
    let mut shapes: Vec<String> = candidates
        .iter()
        .map(|o| {
            let inputs: Vec<String> = o.sig.inputs.iter().map(|t| format!("`{t}`")).collect();
            match inputs.is_empty() {
                true => "no operands".to_string(),
                false => inputs.join(" "),
            }
        })
        .collect();
    shapes.sort();
    let listed = shapes
        .iter()
        .map(|s| format!("\n  candidate: {s}"))
        .collect::<String>();
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: no overload of `{name}` in `{wname}` (line {}) accepts these operands{listed}",
            span.line
        ),
        Ctx::Line { .. } => {
            format!("error: no overload of `{name}` accepts these operands{listed}")
        }
    }
}

fn underflow_error(ctx: &Ctx, span: Span, op: &str, needs: usize, holds: usize) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
            name, span.line, op, needs, holds, effect_str(effect),
        ),
        Ctx::Line { .. } => format!("error: stack underflow: needs {needs} values, but the stack holds {holds}"),
    }
}

/// R7: `str` -> `cstr` is an explicit word, never an implicit conversion; a
/// `str` where a `cstr` is wanted names the fix rather than a plain
/// mismatch, mirroring `size_conversion_needed_error`'s shape.
fn str_needs_cstr_conversion_error(ctx: &Ctx, span: Span, op: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` wants `cstr`, found `str`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)\n  note: declared {}",
            name, span.line, op, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` wants `cstr`, found `str`: convert it explicitly with `cstr` first"
        ),
    }
}

fn type_mismatch_error(ctx: &Ctx, span: Span, op: &str, expected: Type, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` expected `{}`, found `{}`\n  note: declared {}",
            name, span.line, op, expected, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` expected `{expected}`, found `{found}`")
        }
    }
}

/// R4 (D3): `dup`/`over` applied to a non-`Copy` value, in the DESIGN.md form.
/// A linear value has no bits to copy: the only ways to get a second one are to
/// thread this one through or to acquire another explicitly.
///
/// R4 (slice 8b): the linear cause names the `drop` overload when that is what
/// made the type linear. An all-`Copy`-fields resource struct told only that it
/// "has no bits to copy" points at nothing the reader can act on — its bits are
/// plainly copyable, and its own `: drop` declaration is the reason they may not
/// be.
fn cannot_copy_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    let defines_drop =
        matches!(found, Type::Struct(id, _) if ctx.structs()[id.index()].has_drop_overload);
    // A reference is neither `Copy` nor linear, so the ownership wording below
    // would tell the reader the opposite of the type rule.
    let why = if found.is_ref() {
        format!(
            "`{found}` is exclusive: at most one may be live for a place, so copying it would make a second one; use it where it is, or borrow again once it is consumed"
        )
    } else if defines_drop {
        format!(
            "`{found}` is linear because it defines `drop`: its own destructor runs exactly once, so a copy would run it twice; thread the value through instead"
        )
    } else {
        format!(
            "`{found}` is linear: it owns a resource and has no `Copy` instance, so there are no bits to copy; thread the value through instead"
        )
    };
    match ctx {
        Ctx::Word { name, effect, .. } => {
            format!(
            "error: cannot `{}` a value of type `{}` in `{}` (line {})\n  {}\n  note: declared {}",
            op, found, name, span.line, why, effect_str(effect),
        )
        }
        Ctx::Line { .. } => format!("error: cannot `{op}` a value of type `{found}`: {why}"),
    }
}

/// R3 (D2): a linear local mentioned again after its value was moved out, the
/// diagnostic naming the earlier move site.
fn use_after_move_error(ctx: &Ctx, span: Span, local: &str, ty: Type, site: Span) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: use after move in `{}` (line {})\n  local `{}` of type `{}` was moved at line {}, col {}; `{}` is linear, so it is used exactly once\n  note: declared {}",
            name, span.line, local, ty, site.line, site.col, ty, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: use after move: local `{local}` of type `{ty}` was moved at line {}, col {}",
            site.line, site.col
        ),
    }
}

/// R13/R14: a linear local still holding a value at the end of its scope,
/// either never mentioned or consumed on one branch only. Nothing is
/// auto-dropped, so this is an error rather than a compiler-inserted disposal.
fn linear_local_unconsumed_error(ctx: &Ctx, local: &str, ty: Type, line: u32) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear value `{}` is never consumed in `{}` (line {})\n  `{}` has type `{}`, which is linear: drop it or return it (nothing is dropped for you)\n  note: declared {}",
            local, name, line, local, ty, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear value `{local}` is never consumed (line {line})\n  `{local}` has type `{ty}`, which is linear: drop it or leave it on the stack (nothing is dropped for you)"
        ),
    }
}

/// R13/R14: a linear local consumed on one `if` arm but not the other. Unlike
/// `linear_local_unconsumed_error` (never touched at all), this local WAS
/// disposed on one path; the bug is the other arm forgetting it, so the
/// message points at the divergence rather than implying nothing happened.
fn linear_local_maybe_moved_error(ctx: &Ctx, local: &str, ty: Type, line: u32) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear value `{}` is not consumed on every path in `{}` (line {})\n  `{}` has type `{}`, which is linear: it is consumed on one `if` arm but not the other, so drop it (or return it) on every path\n  note: declared {}",
            local, name, line, local, ty, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear value `{local}` is not consumed on every path (line {line})\n  `{local}` has type `{ty}`, which is linear: it is consumed on one `if` arm but not the other, so drop it on every path"
        ),
    }
}

/// R6: a linear value bound inside a block and still holding its value when the
/// block ended. The word-end twins above can only cite the word; a block ends
/// at a token, so this one names it, because that token is where the value
/// became unreachable and the fix belongs before it.
fn linear_local_out_of_scope_error(
    ctx: &Ctx,
    local: &str,
    ty: Type,
    every_path: bool,
    token: &str,
    span: Span,
) -> String {
    let cause = match every_path {
        true => "is not consumed on every path",
        false => "is never consumed",
    };
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear value `{}` {} in `{}` (line {})\n  `{}` has type `{}`, which is linear, and its scope ends at the `{}` on line {}, col {}: consume it before then (nothing is dropped for you)\n  note: declared {}",
            local, cause, name, span.line, local, ty, token, span.line, span.col, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear value `{local}` {cause} (line {})\n  `{local}` has type `{ty}`, which is linear, and its scope ends at the `{token}` on line {}, col {}: consume it before then (nothing is dropped for you)",
            span.line, span.line, span.col,
        ),
    }
}

/// R13 (D7): a linear value left on the stack beyond the declared outputs. The
/// generic arity error (`check_outputs`) already rejects it, but a linear
/// surplus gets its own wording: the fix is disposal, not an extra output slot.
fn surplus_linear_value_error(word: &WordDef, ty: Type, line: u32) -> String {
    format!(
        "error: linear value left on the stack in `{}` (line {})\n  body leaves a `{}` beyond the {} declared output(s): a linear value must be consumed exactly once, so `drop` it or return it\n  note: declared {}",
        crate::resolve::demangle_word(&word.name),
        line,
        ty,
        word.effect.outputs.len(),
        effect_str(&word.effect),
    )
}

/// A reference argument to a self-tail-call whose provenance traces to an
/// owned local of *this* frame — a `place` naming an actual
/// `Deriv::owned_root` — crosses a loop iteration boundary. Locals rebind at
/// the loop header (`carried_slots`), so the storage that local
/// named this iteration is not the storage the same name denotes next
/// iteration, and a reference into it would alias a reused slot. A reference
/// *parameter*, or one derived from it by projection, has no owned root
/// (`owned_root` is `None`, the accept-case) and may cross freely — its
/// referent lives in an ancestor frame that outlives every iteration, which is
/// what keeps `walk ( &!List -- ) ... walk ;` legal.
fn reference_across_back_edge_error(ctx: &Ctx, span: Span, callee: &str, place: &str) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: a reference to a local cannot cross a loop in `{}` (line {})\n  a reference derived from `{place}`, a local of this frame, crosses the self-tail-call back-edge to `{callee}`: that local's storage does not survive to the next iteration\n  note: declared {}",
            name, span.line, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: a reference to a local cannot cross a loop: a reference derived from `{place}` crosses the back-edge to `{callee}`"
        ),
    }
}

/// Reject a reference argument to the recursive call whose derivation's
/// owned root is a local of this frame. Scanned over the call's own arguments
/// (`args`, i.e. `stack[base..]` before the call truncates it) — the values
/// that actually cross the back-edge, as opposed to `check_linear_across_back_edge`'s
/// `below_args`, the values stranded beneath them.
fn check_reference_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    args: &[Slot],
    prov: &Provenance,
) -> Result<(), String> {
    for slot in args {
        if let Some(id) = slot.deriv {
            if let Some(place) = &prov.deriv(id).owned_root {
                return Err(reference_across_back_edge_error(ctx, span, callee, place));
            }
        }
    }
    Ok(())
}

/// Slice 10a (R11): a self-tail combinator's ground declared inputs, ground
/// declared outputs, and bottom-aligned output->carried-input index map --
/// what the back-edge marker carries.
type BackEdgeShape = (Vec<Type>, Vec<Type>, Vec<Option<usize>>);

/// Slice 10a (R11): ground a self-tail combinator's declared inputs and
/// outputs at the marker's set site, plus the bottom-aligned index map. A
/// poly combinator grounds through the `θ` `check_poly_combinator_args`
/// resolved (now returned rather than discarded); a monomorphic one reads its
/// already-concrete `effect`. The index map pairs ground output `i` with the
/// `i`-th non-quotation ground input, both counted from the deepest slot,
/// `None` when `i` is beyond the carried-input count or the types differ
/// (so `times`-shape, with zero fixed outputs, yields an empty map, and
/// `while`'s 1-in/1-out shape yields `[Some(0)]`).
fn back_edge_declared_shape(
    word: &WordDef,
    subst: Option<&Subst>,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
) -> Result<BackEdgeShape, String> {
    let (inputs, outputs): (Vec<Type>, Vec<Type>) = match word.poly.as_ref() {
        Some(sig) => {
            let subst = subst.expect("a poly combinator's marker carries its resolved θ");
            let mut inputs = Vec::with_capacity(sig.inputs.len());
            for p in &sig.inputs {
                inputs.push(apply_subst(sig, p, subst, name, span, ctx, arrays)?);
            }
            let mut outputs = Vec::with_capacity(sig.outputs.len());
            for p in &sig.outputs {
                outputs.push(apply_subst(sig, p, subst, name, span, ctx, arrays)?);
            }
            (inputs, outputs)
        }
        None => (
            word.effect.inputs.iter().map(|s| s.ty).collect(),
            word.effect.outputs.iter().map(|s| s.ty).collect(),
        ),
    };
    let carried: Vec<Type> = inputs
        .iter()
        .copied()
        .filter(|t| crate::ast::is_quotation_type(*t).is_none())
        .collect();
    let index_map = outputs
        .iter()
        .enumerate()
        .map(|(i, out)| match carried.get(i) {
            Some(c) if c == out => Some(i),
            _ => None,
        })
        .collect();
    Ok((inputs, outputs, index_map))
}

/// R11/R12: check a quotation *literal* against a declared quotation parameter
/// directionally (slice 4 D3): seed a fresh sub-stack with the declared input
/// row, run the literal's body against it, and require the exit row to equal
/// the declared output row (no standalone effect is inferred). Enforce the D3
/// capture restriction here (R12): a read that consumes a non-`Copy` enclosing
/// local, or a borrow of an enclosing place left on the row, is rejected; a
/// `Copy` local read by value is allowed.
///
/// Slice 10c: `is_arm` marks a literal filling a parameter slot the callee
/// `call`s in *tail* position -- a branch arm of `if`/`unless`, as opposed to
/// `times`' body. Such a literal runs at most once per entry and inherits the
/// call site's own tail position; `caller_tail` supplies that. See the
/// `check_terms_relaxed` call below.
///
/// Slice 10c (R-P2-3/R-P2-4): `shape_changing` is true for a declared
/// quotation whose input and output rows differ (`..i -- ..o`, `..i != ..o`).
/// There the exit row has no fixed point to check against -- the whole point
/// of the shape change -- so this returns the literal's actual exit types
/// without judging them, and the *caller* (`check_poly_combinator_args`)
/// compares one sibling literal's actual exit types against another's,
/// erroring at whichever literal disagrees (R-P2-4: no row unification,
/// `..o` is discovered by forward checking, never solved for).
#[allow(clippy::too_many_arguments)]
fn check_literal_against_declared_effect(
    id: QuotId,
    eff: &QuotEffect,
    is_inline: bool,
    row: &[Slot],
    word: &str,
    span: Span,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
    granted: &HashSet<String>,
    shape_changing: bool,
    is_arm: bool,
    caller_tail: bool,
) -> Result<Vec<Type>, String> {
    let body = prov.quotations[id.0].body.clone();
    // Slice 12 (R-C2): the literal's own spelling (`~[ ... ]` vs `[ ... ]`)
    // must match the boundary's declared flavour, independent of whatever
    // this literal actually type-checks against. Every argument-matching site
    // (a combinator parameter -- mono or poly -- and every ordinary
    // `Type::Quotation` materialization boundary) funnels through this one
    // function, so this is the single place both directions (E3a/E3b) are
    // enforced.
    let literal_is_inline = prov.quotations[id.0].is_inline;
    if literal_is_inline != is_inline {
        let param = if is_inline {
            crate::ast::inline_quotation_type(eff.inputs.clone(), eff.outputs.clone())
        } else {
            crate::ast::quotation_type(eff.inputs.clone(), eff.outputs.clone())
        };
        let literal_span = prov.quotations[id.0].span;
        return Err(if literal_is_inline {
            inline_literal_at_ordinary_param_error(ctx, literal_span, word, param)
        } else {
            ordinary_literal_at_inline_param_error(ctx, literal_span, word, param)
        });
    }
    let outer_locals: HashSet<String> = scope.bound.iter().map(|b| b.name.clone()).collect();
    let moves_before = scope.moves.states.clone();
    // Slice 10a (R9): ground the declared quotation's row against the concrete
    // caller region `row`, prepended type-only below the declared inputs. The
    // slots are `Slot::computed`, so `deriv`/`surviving`/`quot` are dropped
    // (R16): prepending the caller's real slots would make the exit-row borrow
    // guard below flag a caller borrow riding untouched in the row as
    // `quotation borrows place`, a false positive on correct code. A caller
    // whose effect carries no row (every one but the poly literal path) passes
    // an empty region, leaving `fresh` as before.
    // Slice 10c: a `~` branch arm keeps the caller's real row slots. The
    // erasure below exists so a caller borrow riding untouched in the row is
    // not flagged by the exit-row guard -- a guard this arm already skips
    // (`is_inline`), because the arm runs in place against exactly these
    // slots. Erasing them here instead drops each one's `quot` marker, so a
    // self-tail combinator forwarding its own quotation parameter through an
    // arm sees a bare `Cstr` placeholder where a quotation is declared.
    let mut fresh: Vec<Slot> = match is_inline && is_arm {
        true => row.to_vec(),
        false => row.iter().map(|s| Slot::computed(s.ty)).collect(),
    };
    fresh.extend(eff.inputs.iter().map(|t| Slot::computed(*t)));
    let depth = scope.depth();
    // Slice 10c: an arm occupies the caller's tail position when the call
    // site does. Pinning that `false` made this probe walk a self-recursive
    // branch arm as an ordinary call and splice the enclosing combinator
    // forever, where in tail position the self-call is the back-edge.
    //
    // The `back_edge` flag is the mirror image: a `~` arm runs *at most once*
    // per entry, exactly as the deleted `if`/`else`/`end` arms did (which is
    // why the retired arm walk passed `false` here), so a granted outer name
    // may die at its last use inside it. Every other quotation keeps the
    // conservative `true` -- `times`' body really does wrap around, and a
    // reference inherited into it stays live across the whole body.
    let result = check_terms_relaxed(
        &body,
        fresh,
        ctx,
        env,
        arrays,
        cells,
        refs,
        prov,
        scope,
        is_arm && caller_tail,
        poly,
        granted,
        !(is_inline && is_arm),
    )?;
    // R12: a linear enclosing local the literal consumed (move-state changed
    // from `Live`).
    //
    // Slice 10c: not applied to a `~` *branch arm*. This is a conservative
    // argument-site pre-check whose premise is that the callee may call the
    // quotation any number of times, so one consumption here could be many at
    // run time -- which is exactly right for `times`' body, the shape it
    // exists for, and plainly wrong for a branch arm: an arm consuming an
    // enclosing linear local is what the deleted `if`/`else`/`end` arms did,
    // reconciled at the join into `MaybeMoved`. `is_arm` is the discriminator
    // (the callee `call`s this slot in *tail* position, so it runs at most
    // once per entry), not `is_inline` alone.
    if is_inline && is_arm {
        // The probe must also leave no trace: two sibling arms are
        // alternatives, each starting from the same move-state, and the splice
        // re-checks whichever one runs. Without the restore the second arm
        // sees the first arm's consumption and reports use-after-move.
        scope.moves.states = moves_before.clone();
    } else if let Some(local) =
        moves_before
            .iter()
            .find_map(|(n, before)| match (before, scope.moves.states.get(n)) {
                (MoveState::Live, Some(MoveState::Moved(_) | MoveState::MaybeMoved(_))) => {
                    Some(n.clone())
                }
                _ => None,
            })
    {
        return Err(quotation_captures_local_error(ctx, span, word, &local));
    }
    // R12: a borrow of an enclosing place left live on the literal's exit row.
    // Skipped for a `~` branch arm for the same reason as the linear-capture
    // rule above: the arm runs once, in place, so a borrow left on its exit
    // row is the caller's own, live in the caller's own frame, exactly as it
    // was when a branch arm was an inline block rather than a quotation.
    if !(is_inline && is_arm) {
        for slot in &result {
            if let Some(did) = slot.deriv {
                if let Some(place) = &prov.deriv(did).owned_root {
                    if outer_locals.contains(place) {
                        return Err(quotation_borrows_place_error(ctx, span, word, place));
                    }
                }
            }
        }
    }
    leave_block(
        ctx,
        scope,
        depth,
        BlockEnd::Arm {
            token: "quotation",
            span,
        },
    )?;
    let actual: Vec<Type> = result.iter().map(|s| s.ty).collect();
    if shape_changing {
        // R-P2-4: a shape-changing declared quotation has no fixed exit row
        // to check against as a whole; the caller reconciles sibling
        // literals' full actual shapes against each other (R-P2-3).
        //
        // Review fix: that sibling comparison alone never runs at all when a
        // shape-changing parameter has no sibling sharing its row id (or every
        // sibling happens to agree with each other while still disagreeing
        // with the declaration), so the declared *fixed* trailing outputs
        // (`eff.outputs`, e.g. the `i64` in `~[ ..i -- ..o i64 ]`) were never
        // checked at all -- a mismatch there surfaced later as a generic
        // boundary error, not at this argument site. The row prefix length is
        // genuinely undetermined (that is the point of R-P2-4), but the
        // declared suffix is still a fixed point and is checked here.
        let suffix_len = eff.outputs.len();
        let suffix_matches = result.len() >= suffix_len
            && result[result.len() - suffix_len..]
                .iter()
                .zip(&eff.outputs)
                .all(|(f, w)| {
                    matches!(
                        match_slot(*f, *w),
                        SlotMatch::Exact | SlotMatch::LiteralSizeType
                    )
                });
        if !suffix_matches {
            let actual_outs: Vec<Type> = result
                .iter()
                .skip(result.len().saturating_sub(suffix_len))
                .map(|s| s.ty)
                .collect();
            let declared = if is_inline {
                crate::ast::inline_quotation_type(eff.inputs.clone(), eff.outputs.clone())
            } else {
                crate::ast::quotation_type(eff.inputs.clone(), eff.outputs.clone())
            };
            let actual_effect = crate::ast::quotation_type(eff.inputs.clone(), actual_outs);
            return Err(literal_effect_mismatch_error(
                ctx,
                span,
                word,
                declared,
                actual_effect,
            ));
        }
        return Ok(actual);
    }
    // R11: the literal's exit row must equal the grounded declared output row:
    // the same carried region `row` followed by the declared outputs. N=0
    // leaves the region untouched and N≥2 feeds one iteration's output into the
    // next, so the carried region is a fixed point (spec: "one row, the same on
    // both sides").
    let expected_out: Vec<Type> = row
        .iter()
        .map(|s| s.ty)
        .chain(eff.outputs.iter().copied())
        .collect();
    let matches_out = result.len() == expected_out.len()
        && result.iter().zip(&expected_out).all(|(f, w)| {
            matches!(
                match_slot(*f, *w),
                SlotMatch::Exact | SlotMatch::LiteralSizeType
            )
        });
    if !matches_out {
        // R9/R10: strip the grounded row region before rendering, so the
        // caller's concrete stack never leaks into the printed effect -- the
        // declared/actual types show only the quotation's own fixed slots.
        let actual_outs: Vec<Type> = result.iter().skip(row.len()).map(|s| s.ty).collect();
        let declared = if is_inline {
            crate::ast::inline_quotation_type(eff.inputs.clone(), eff.outputs.clone())
        } else {
            crate::ast::quotation_type(eff.inputs.clone(), eff.outputs.clone())
        };
        let actual = crate::ast::quotation_type(eff.inputs.clone(), actual_outs);
        return Err(literal_effect_mismatch_error(
            ctx, span, word, declared, actual,
        ));
    }
    Ok(actual)
}

/// R10/R21: a quotation parameter position whose argument is not a quotation
/// the callee can consume -- a non-quotation value, or (after R21 admits the
/// abstract forward) a quotation whose *declared effect* disagrees with the
/// callee parameter. Knownness is no longer the complaint: a forwarded abstract
/// parameter is accepted, so `want` and `found` always differ here (a
/// non-quotation type, or a mismatched effect), and the message names both.
fn quotation_argument_required_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    want: Type,
    found: Type,
) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: `{word}` expects a quotation `{want}` here, found `{found}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R11: a quotation literal whose effect disagrees with the declared
/// parameter. Names the word, the declared effect, and the literal's actual
/// effect.
fn literal_effect_mismatch_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    declared: Type,
    actual: Type,
) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: the quotation passed to `{word}` was declared `{declared}` but its body has effect `{actual}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// Slice 12 (R-C2, E3a): an ordinary `[ ... ]` literal at a `~[ ... ]`
/// (`Type::InlineQuotation`) parameter. Located at the argument literal, not
/// the word definition -- the declared flavour is fine, the caller spelled
/// the wrong bracket.
fn ordinary_literal_at_inline_param_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    param: Type,
) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: this argument is an ordinary `[ ... ]` quotation but `{word}` declares parameter `{param}` as inline `~[ ... ]`; write it `~[ ... ]`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// Slice 12 (R-C2, E3b): a `~[ ... ]` literal at an ordinary `Type::Quotation`
/// boundary (parameter, field/array store, word output). The mirror of the
/// error above.
fn inline_literal_at_ordinary_param_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    param: Type,
) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: this argument is an inline `~[ ... ]` quotation but `{word}` declares parameter `{param}` as an ordinary `[ ... ]`; write it `[ ... ]`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// Slice 12 (R-C2, E3b): the direct-`call` boundary. `call` has no declared
/// parameter to name (it splices whatever literal sits on top), so a `~[ ... ]`
/// there is rejected against `call` itself rather than a named parameter --
/// `~` marks a literal spliced only through a declared inline parameter, not
/// general splice-by-`call` spelling.
fn inline_literal_at_call_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: this argument is an inline `~[ ... ]` quotation but `call` splices an ordinary `[ ... ]`; write it `[ ... ]`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// Slice 10c (R-P2-3): two sibling literals passed to the same
/// shape-changing declared quotation parameter (sharing one declared output
/// row `..o`) leave different actual output shapes. Located at the *second*
/// literal's own span -- the argument site -- rather than wherever the
/// spliced callee body would eventually notice the disagreement (recon 8: the
/// diagnostic this restores).
fn combinator_branch_output_mismatch_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    expected: &[Type],
    found: &[Type],
) -> String {
    let word = crate::resolve::demangle_word(word);
    let render = |types: &[Type]| match types.is_empty() {
        true => "nothing".to_string(),
        false => format!(
            "`{}`",
            types
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ),
    };
    format!(
        "error: the quotations passed to `{word}` leave different stack shapes: an earlier one leaves {}, this one leaves {}{} (line {})",
        render(expected),
        render(found),
        in_word(ctx),
        span.line,
    )
}

/// R12: a quotation literal that consumes a linear enclosing local (D3 forbids
/// a linear capture). Names the local and the enclosing word.
fn quotation_captures_local_error(ctx: &Ctx, span: Span, word: &str, local: &str) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: the quotation passed to `{word}` consumes the enclosing local `{local}`, which is linear; a quotation may only read a `Copy` enclosing local by value (D3){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R12: a quotation literal that borrows an enclosing place and leaves the
/// reference on its row (D3 forbids capturing an enclosing borrow).
fn quotation_borrows_place_error(ctx: &Ctx, span: Span, word: &str, place: &str) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: the quotation passed to `{word}` borrows the enclosing place `{place}`; a quotation may not capture a borrow of an enclosing local (D3){} (line {})",
        in_word(ctx),
        span.line,
    )
}

fn rebound_local_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let scope_end = "a name may not be re-bound while it is in scope: the earlier binding would become unreachable, and a linear value in it could then never be consumed";
    match ctx {
        Ctx::Word { name: word, .. } => format!(
            "error: `{name}` is already bound in `{word}` (line {}, col {})\n  {scope_end}",
            span.line, span.col
        ),
        Ctx::Line { .. } => format!(
            "error: `{name}` is already bound (line {}, col {})\n  {scope_end}",
            span.line, span.col
        ),
    }
}

/// R2/R6: take every name bound past `depth` out of scope, and reject a linear
/// value still held when the block ended. Nothing is auto-dropped, so leaving a
/// block is one more place where forgetting a value is *caught*, never a place
/// where one is disposed for you.
fn leave_block(ctx: &Ctx, scope: &mut Scope, depth: usize, at: BlockEnd) -> Result<(), String> {
    let Some((local, ty, state)) = scope.leave(depth) else {
        return Ok(());
    };
    let every_path = matches!(state, MoveState::MaybeMoved(_));
    Err(match at {
        BlockEnd::Body(line) if every_path => linear_local_maybe_moved_error(ctx, &local, ty, line),
        BlockEnd::Body(line) => linear_local_unconsumed_error(ctx, &local, ty, line),
        BlockEnd::Arm { token, span } => {
            linear_local_out_of_scope_error(ctx, &local, ty, every_path, token, span)
        }
    })
}

/// A `usize`/`isize` position (a binary operator's other operand, a
/// word-call argument, or a declared output) fed a *computed* (non-literal)
/// `i64` (X10): unlike a bare integer literal, a computed value doesn't
/// silently coerce, since Sooth has no comptime interpreter to fold it and
/// confirm it fits; names the missing `>usize`/`>isize` conversion
/// explicitly, naming whichever size type `target` is.
fn size_conversion_needed_error(ctx: &Ctx, span: Span, op: &str, target: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` mixes `{}` with a computed `i64`: convert it explicitly with `>{}` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
            name, span.line, op, target, target, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` mixes `{target}` with a computed `i64`: convert it explicitly with `>{target}` first"
        ),
    }
}

/// Slice 10c: the two arms of a `branch` disagree. Named for the *arms*, not
/// for `if`: `branch` is the primitive and `if`/`unless` are ordinary
/// `lib/core.sth` words over it, so by the time this fires the surface word
/// the user wrote has been spliced away and could equally have been `branch`
/// itself. (The span still points at the first arm, which is the user's own
/// literal either way -- see `check_branch_join`.)
fn branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  the two branch arms leave different stack depths (then: {}, else: {})\n  note: declared {}",
            name, span.line, d_then, d_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: the two branch arms leave different stack depths (then: {d_then}, else: {d_else})"
        ),
    }
}

fn branch_type_mismatch_error(ctx: &Ctx, span: Span, t_then: Type, t_else: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  the two branch arms leave different types (then: `{}`, else: `{}`)\n  note: declared {}",
            name, span.line, t_then, t_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: the two branch arms leave different types (then: `{t_then}`, else: `{t_else}`)"
        ),
    }
}

/// An array word (`fill`/`len`) applied to a non-array operand: names the
/// array word and the offending operand type (X8).
fn array_word_operand_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an array operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires an array operand, found `{found}`")
        }
    }
}

/// `fill` given a linear element type: unlike `dup`/`over`, `fill` has no
/// per-slot `Copy` gate today, so it would silently replicate a linear value
/// (and array-element linearity is not tracked transitively yet, so neither
/// `drop` nor a nested struct's `dup` check would ever see the array's real
/// element count). Reject rather than accept a value the rest of the linear
/// checker can't reason about; array-of-linear support is future work.
/// D2 (phase 2): `site` names the construction site (`"fill"`, or the array
/// constructor's own site), rendered as a bare code span, so `fill`'s call
/// passing `"fill"` keeps this byte-identical.
fn fill_of_linear_element_error(ctx: &Ctx, span: Span, elem: Type, site: &str) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear array elements are not supported yet in `{}` (line {})\n  `{}` would replicate a `{}` across every slot, but `{}` is linear and has no `Copy` instance\n  note: declared {}",
            name, span.line, site, elem, elem, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear array elements are not supported yet: `{site}` would replicate a `{elem}` across every slot, but `{elem}` is linear and has no `Copy` instance"
        ),
    }
}

/// D3 (slice 6h phase 2): the array constructor's element transitively
/// contains `str`/`cstr`/a quotation -- all `Copy` and pointer-shaped, so an
/// all-zero slot would be a null pointer whose first read faults. Names the
/// offending inner type and the field/variant/array-element path to it
/// (outermost first); an empty path means the element itself is the
/// offending type.
fn array_constructor_zero_unsafe_element_error(
    ctx: &Ctx,
    span: Span,
    outer: Type,
    bad: Type,
    path: &[String],
) -> String {
    let where_ = if path.is_empty() {
        "directly".to_string()
    } else {
        format!("via {}", path.join(" -> "))
    };
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: cannot zero-initialize a `{}` in `{}` (line {})\n  `{}` transitively contains `{}` ({}), which is pointer-shaped and would zero to a null pointer\n  note: declared {}",
            outer, name, span.line, outer, bad, where_, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: cannot zero-initialize a `{outer}`: it transitively contains `{bad}` ({where_}), which is pointer-shaped and would zero to a null pointer"
        ),
    }
}

/// The referent of a reference type, and whether it is mutable.
fn ref_parts(ty: Type, refs: &[RefDecl]) -> Option<(Type, bool)> {
    match ty {
        Type::Ref(id, mutable, _) => Some((refs[id.index()].referent, mutable)),
        _ => None,
    }
}

/// ` in `word`` for a word body, empty for a bare REPL line: the suffix the
/// slice's own diagnostics use to place themselves the way every other
/// located error here does.
fn in_word(ctx: &Ctx) -> String {
    match ctx {
        Ctx::Word { name, .. } => format!(" in `{name}`"),
        Ctx::Line { .. } => String::new(),
    }
}

/// R11: a quotation used as the operand of any type-directed consumer is an
/// audited default-deny. A quotation is a compile-time-only marker with a
/// `Cstr` placeholder `ty` (R4) that ordinary matching would silently accept
/// or spell into a mismatch, so every consumer that inspects a popped slot's
/// `ty` names itself through this one guard instead. Only `call`
/// consumes a quotation; the shuffles forward it and `drop` discards it.
fn reject_quotation_operand(ctx: &Ctx, span: Span, op: &str) -> String {
    format!(
        "error: `{op}`{} (line {}) cannot take a quotation as an operand; only `call` accepts a quotation (a runtime quotation value is slice 7)",
        in_word(ctx),
        span.line,
    )
}

/// R10/R26: a quotation passed to a parameter position that is *not* a
/// declared `Type::Quotation`. A quotation argument to a declared quotation
/// parameter is now accepted and inlined (R18); this fires only for the other
/// positions (a non-quotation user parameter, a generated constructor/setter
/// slot, an `extern` argument). Only the stale "Phase 6" parenthetical is
/// reworded to point a runtime quotation value at slice 7 (R26).
fn reject_quotation_argument(ctx: &Ctx, span: Span, word: &str) -> String {
    let word = crate::resolve::demangle_word(word);
    format!(
        "error: a quotation cannot be passed to `{word}`; only `call` accepts one (a runtime quotation value is slice 7){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// Exclusivity, in whichever of its two symmetric directions was
/// violated — a new mutable borrow conflicts with any live borrow of the place,
/// a new shared one with a live mutable borrow. When the live borrow is a
/// projection, the note says outright that path disjointness is not modeled,
/// since the two references may well be aimed at different fields.
fn conflicting_borrow_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    new_mutable: bool,
    live: &Deriv,
) -> String {
    let sigil = if new_mutable { "&!" } else { "&" };
    let held = if live.mutable { "mutable" } else { "shared" };
    let note = if live.projected {
        "\n  note: path disjointness is not modeled: a reference projected into one field borrows the whole place"
    } else {
        ""
    };
    format!(
        "error: `{sigil}{place}` conflicts with a live borrow of `{place}`{} (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first{note}",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// Two construction sites the declaration-site rule cannot reach: `fill`'s
/// element and `^`'s payload accept
/// whatever type is on the stack, with no declaration anywhere for
/// `check_no_stored_references` to have caught.
fn constructed_reference_error(ctx: &Ctx, span: Span, position: &str, ty: Type) -> String {
    format!(
        "error: a reference cannot be stored{} (line {})\n  {position} has type `{ty}`\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow",
        in_word(ctx),
        span.line
    )
}

/// D3 (slice 8b): a struct's destructure (`S>`) or field getter (`S>f`) moves
/// fields out of `S`, bypassing whatever `drop` override `S` owns -- the value
/// never reaches a bare `drop` call site for D1's gate to see. `name` is
/// checked as-parsed (mangled in a >=2-module build, matching
/// `struct_generated_sigs`'s own keys), so this runs ahead of both
/// `check_struct_get_word` (which alone claims an aggregate-typed field) and
/// the ordinary `env` call path (every other field type, and the full
/// destructure), catching the accessor before either applies its signature.
/// The functional setter (`S<f`) has no `>` in its name and never matches
/// here; it returns the struct itself, so the value stays live.
fn check_destructure_drop_guard(name: &str, span: Span, ctx: &Ctx) -> Result<(), String> {
    let Some((struct_name, field_name)) = name.split_once('>') else {
        return Ok(());
    };
    let Some((struct_idx, decl)) = ctx
        .structs()
        .iter()
        .enumerate()
        .find(|(_, d)| d.name == struct_name)
    else {
        return Ok(());
    };
    if !decl.has_drop_overload {
        return Ok(());
    }
    // A word literally named `drop` is exempt only for the *one* struct its
    // own declared effect names (`find_drop_overloads` rejects any other
    // input shape before body checking ever starts): its own body is exactly
    // where moving that struct's fields out implements disposal
    // (`examples/resources.sth`'s `Fd>n` inside `: drop`). `resolve::mangle`
    // leaves `drop` unmangled program-wide, so a name-only check would wave
    // through *any* word named `drop`, including one overriding a different
    // struct that destructures this one -- compare the struct identity the
    // enclosing word's effect declares, not the word's name.
    let is_own_drop_body = ctx.mangled_name() == Some("drop")
        && ctx.effect().is_some_and(|eff| {
            matches!(eff.inputs.as_slice(), [input] if matches!(input.ty, Type::Struct(id, _) if id.index() == struct_idx))
        });
    if is_own_drop_body {
        return Ok(());
    }
    let is_destructure = field_name.is_empty();
    let is_field_move = decl.fields.iter().any(|(f, _)| f == field_name);
    if is_destructure || is_field_move {
        return Err(destructure_drop_overloaded_error(ctx, span, decl));
    }
    Ok(())
}

/// R11 (slice 8b, D3): the located diagnostic for destructuring a type whose
/// `drop` override would otherwise be skipped.
fn destructure_drop_overloaded_error(ctx: &Ctx, span: Span, decl: &StructDecl) -> String {
    let source = crate::resolve::demangle_word(&decl.name);
    let note = "\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out";
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot destructure `{source}` in `{name}` (line {}): it defines `drop`, so moving its fields out would skip its destructor{note}",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot destructure `{source}` (line {}): it defines `drop`, so moving its fields out would skip its destructor{note}",
            span.line
        ),
    }
}

fn check_shuffle(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "dup" => {
            let top = *stack.last().ok_or_else(|| need("dup", 1, stack.len()))?;
            // R4 (D3): `dup` is the explicit copy, so it is gated on `Copy`.
            // The pure reorderings below (`swap`/`rot`) move rather than copy
            // and stay legal on a linear value.
            if !is_copy(top.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(cannot_copy_error(ctx, span, "dup", top.ty));
            }
            // `dup` of an aggregate deep-copies it (`Alloc`+`Blit`), so the
            // copy denotes a region of its own — this is the whole remedy for an
            // aliased place. `over` below reuses the value instead, and so
            // deliberately keeps the region it copies.
            stack.push(Slot { alias: None, ..top });
        }
        "drop" => {
            let top = stack.pop().ok_or_else(|| need("drop", 1, 0))?;
            // R6 (slice 8b): a side observation only. `drop` still pops one
            // value of any type with no type check, exactly as before; the
            // recorded type is what lets `check`'s post-pass resolve which
            // concrete override (if any) this call site dispatches to.
            // R11 carve-out: `drop` of a compile-time-only quotation marker
            // discards it with nothing to dispose, and its `Cstr` placeholder
            // is inert in the drop-override graph; skip the push.
            if top.quot.is_none() {
                // D1 (R3/R4): disposing a value whose struct owns a `drop`
                // override runs that destructor, so the override must be
                // visible to the calling module (declared locally or imported
                // by name). On the REPL path `ctx.modules()` is `None` (R8) and
                // the arm is exactly what it was before this slice. The
                // `prov.dropped` recording below is unchanged either way.
                if let (Type::Struct(id, _), Some(m)) = (top.ty, ctx.modules()) {
                    if ctx.structs()[id.index()].has_drop_overload {
                        let decl = &ctx.structs()[id.index()];
                        check_drop_import_visibility(ctx, span, m, decl)?;
                    }
                }
                prov.dropped.push(top.ty);
            }
        }
        "swap" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("swap", 2, n));
            }
            stack.swap(n - 1, n - 2);
        }
        "over" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("over", 2, n));
            }
            let mut below = stack[n - 2];
            // `over` is gated exactly like `dup`.
            if !is_copy(below.ty, ctx.structs(), ctx.enums(), arrays) {
                return Err(cannot_copy_error(ctx, span, "over", below.ty));
            }
            // Unlike `dup`, `over` reuses the value rather than deep-copying it,
            // so both slots denote one address. An anonymous aggregate has no
            // region yet, and binding each slot would otherwise mint a separate
            // one, hiding the aliasing.
            if below.alias.is_none() && below.ty.is_aggregate() {
                let region = prov.fresh_region();
                let set = prov.alias_set_of(region);
                below.alias = Some(Alias { set, span });
                stack[n - 2].alias = below.alias;
            }
            stack.push(below);
        }
        "rot" => {
            let n = stack.len();
            if n < 3 {
                return Err(need("rot", 3, n));
            }
            // a b c -> b c a
            let a = stack[n - 3];
            stack[n - 3] = stack[n - 2];
            stack[n - 2] = stack[n - 1];
            stack[n - 1] = a;
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
    }

    /// Review fix (R-P2-3, declaration half): a shape-changing declared
    /// quotation parameter (`~[ ..i -- ..o i64 ]`, `..i != ..o`) with no
    /// sibling sharing its row id never goes through the sibling comparison
    /// `check_poly_combinator_args` runs (there is nothing to compare
    /// against), so the declared *fixed* trailing output (`i64`) must still be
    /// checked against the literal's own actual trailing output right here.
    /// `g`'s single branch leaves a `bool` where `i64` was declared; this must
    /// be rejected at `g`'s own argument site, not surface later as a generic
    /// mismatch at `demo`'s declaration boundary (the recon-8 loss R-P2-3
    /// exists to prevent).
    #[test]
    fn shape_changing_quotation_with_no_sibling_checks_its_declared_trailing_output() {
        let src = ": g inline ( ..i bool ~[ ..i -- ..o i64 ] -- ..o i64 ) | c | drop c call ;\n\
             : demo ( i64 -- i64 i64 ) true ~[ dup 1 = ] g ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("declared") && err.contains("effect"),
            "expected a literal-effect-mismatch message, got: {err}"
        );
        assert!(
            !err.contains("different stack shapes"),
            "must not be the sibling-comparison message (there is no sibling here): {err}"
        );
        assert!(err.contains("line 2"), "unexpected message: {err}");
    }

    /// D3's leaf resource: one field, a `drop` override implemented exactly
    /// as `examples/resources.sth`'s `Fd` (extracting the field via `Fd>n`
    /// inside `drop`'s own body -- exempted, since a word literally named
    /// `drop` can only be the recognized override for the struct its declared
    /// effect names).
    const FD_DEF: &str = "type: Fd n i64 ;\n: drop ( Fd -- ) | h | h Fd>n drop ;\n";
    /// `File`, whose only field is an `i64`, with a `drop` overload: the shape
    /// every R3/R4 test turns on, since the structural fold alone would call
    /// it `Copy`.
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File>fd . ;";
    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str =
        "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy>tag . ;\n";
    fn infer_src(src: &str, entry: &[Type]) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        // `bool` is `Type::Enum(BOOL_ENUM_ID, ..)` (Slice 9): a real REPL
        // session seeds this at index 0 (`Session::new`); this bare-line
        // helper mirrors that so a `bool`-producing comparison resolves.
        // Slice 10c (R-P3-4): a session also seeds `lib/core.sth`'s words, so
        // a line's `<`/`if` resolves to the library definition; without them a
        // bare comparison is an unknown word.
        let bool_enums = [crate::ast::bool_enum_decl()];
        let prelude = crate::parser::prelude_words();
        let mut combinators = CombinatorEnv::default();
        for word in &prelude {
            let entry = combinator_of(word).expect("a prelude word has a term body");
            combinators.insert(word.name.clone(), vec![entry]);
        }
        // `True`/`False`, which a comparison word's branch-and-construct body
        // calls; a session registers them from the injected `bool` enum.
        let env: HashMap<String, Vec<Overload>> = enum_generated_sigs(&bool_enums)
            .into_iter()
            .map(|(name, sig)| {
                let symbol = name.clone();
                (name, vec![Overload { sig, symbol }])
            })
            .collect();
        infer_line(
            &terms,
            entry,
            &env,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &[],
            &bool_enums,
            &HashMap::new(),
            &combinators,
        )
        .map(|(stack, _insts, _overloads)| stack)
    }
    #[test]
    fn destructure_of_drop_overloaded_type_is_error() {
        let err = check_src(&format!("{FD_DEF}: main ( -- ) 7 Fd Fd> . ;\n")).unwrap_err();
        assert_eq!(
            err,
            "error: cannot destructure `Fd` in `main` (line 3): it defines `drop`, so moving its fields out would skip its destructor\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out"
        );
    }
    #[test]
    fn field_move_of_drop_overloaded_type_is_error() {
        let err = check_src(&format!("{FD_DEF}: main ( -- ) 7 Fd Fd>n . ;\n")).unwrap_err();
        assert_eq!(
            err,
            "error: cannot destructure `Fd` in `main` (line 3): it defines `drop`, so moving its fields out would skip its destructor\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out"
        );
    }
    #[test]
    fn own_drop_body_may_not_destructure_a_different_drop_overloaded_struct() {
        // Bug 1 (round-1 review): the exemption for a word literally named
        // `drop` must be scoped to the *one* struct its own declared effect
        // names, not to the bare name `"drop"` -- `resolve::mangle` leaves
        // `drop` unmangled program-wide, so any struct's own `drop` override
        // could otherwise destructure an unrelated drop-overloaded struct and
        // skip *that* struct's destructor. `Box`'s own `drop` here
        // destructures `Fd`, not `Box`, so it must still be rejected.
        let err = check_src(&format!(
            "{FD_DEF}type: Box b i64 ;\n: drop ( Box -- ) | x | 7 Fd Fd>n drop x Box>b drop ;\n: main ( -- ) 1 Box drop ;\n"
        ))
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot destructure `Fd` in `drop` (line 4): it defines `drop`, so moving its fields out would skip its destructor\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out"
        );
    }
    #[test]
    fn field_move_of_composite_holding_resource_is_ok() {
        // `File` has no override of its own; moving the still-linear `Fd`
        // out of it is unguarded -- D3 fires on `Fd.has_drop_overload`, not
        // on `File`'s. The extracted `Fd` is disposed by an ordinary bare
        // `drop`, unrelated to D3.
        check_src(&format!(
            "{FD_DEF}type: File fd Fd ;\n: main ( -- ) 7 Fd File File>fd drop ;\n"
        ))
        .unwrap();
    }
    #[test]
    fn setter_on_drop_overloaded_type_is_not_guarded() {
        // The functional setter returns `Fd` itself (the value stays live),
        // and its name has no `>`, so it never matches the guard.
        check_src(&format!("{FD_DEF}: main ( -- ) 7 Fd 8 Fd<n drop ;\n")).unwrap();
    }
    #[test]
    fn check_outputs_rejects_a_quotation_left_on_exit() {
        // R10: a matching output *count* means the ordinary path would emit a
        // type mismatch that leaks the `Cstr` placeholder; the dedicated
        // quotation-at-exit branch in `check_outputs` fires first and names the
        // word.
        let err = check_src(": f ( -- i64 ) [ + ] ;\n")
            .expect_err("a quotation left on a word's exit should be rejected");
        assert!(
            err.contains("`f`")
                && err.contains("leaves a quotation on the stack")
                && err.contains("declared output"),
            "check_outputs should name `f` and the output, got: {err}"
        );
    }
    #[test]
    fn infer_line_rejects_a_quotation_left_on_the_residual() {
        // R19: a REPL line has no declared outputs, so R10's route never runs;
        // the `quot` side channel would die at the line boundary while lowering
        // has already pushed a phantom the residual spill would marshal.
        let err = infer_src("1 [ + ]", &[])
            .expect_err("a quotation on a line's residual stack should be rejected");
        assert!(
            err.contains("a quotation cannot be left on the stack at the end of a line"),
            "infer_line should reject the residual quotation, got: {err}"
        );
    }
    #[test]
    fn check_gcd_is_ok() {
        let src = std::fs::read_to_string("examples/gcd.sth").unwrap();
        check_src(&src).unwrap();
    }
    #[test]
    fn check_factorial_is_ok() {
        let src = std::fs::read_to_string("examples/factorial.sth").unwrap();
        check_src(&src).unwrap();
    }
    #[test]
    fn check_lerp_is_ok() {
        let src = std::fs::read_to_string("examples/lerp.sth").unwrap();
        check_src(&src).unwrap();
    }
    #[test]
    fn str_and_cstr_are_copy_and_storable() {
        // Criterion 15/R10: `dup` is accepted on both, and a `str` is
        // storable in a struct field (never seen as containing a
        // reference, and Copy, so no linearity obligation on the field).
        let src = "type: Box s str ;\n\
: main ( -- )\n  \"hi\" dup drop drop\n  \"hi\" cstr dup drop drop\n  \"hi\" Box drop ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_unconsumed_all_copy_resource_at_word_end_is_error() {
        // Criterion 3/R3: the forgotten-disposal check inherits the forced
        // linearity, so an all-`Copy`-fields resource left bound at the end of
        // a body is an error naming it.
        let err = check_src(&format!("{FILE_RESOURCE} : main ( -- ) 1 File | f | ;")).unwrap_err();
        assert!(
            err.contains("linear value `f` is never consumed"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File`"), "unexpected message: {err}");
    }
    #[test]
    fn check_str_where_cstr_declared_is_error() {
        // Criterion 10/R7: passing a `str` where a `cstr` is declared is a
        // type error naming the conversion, not a silent pointer pun.
        let src =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- )\n  \"hi\" strlen drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("cstr"), "unexpected message: {err}");
        assert!(
            err.contains("convert it explicitly"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_stack_underflow_is_error() {
        let src = ": oops ( i64 -- i64 )\n  | a | a a + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("oops"));
        assert!(err.contains("`+`"));
        assert!(err.contains("needs 2 values"));
        assert!(err.contains("holds 1"));
        assert!(err.contains("( i64 -- i64 )"));
    }
    #[test]
    fn check_branch_depth_mismatch_is_error() {
        // Slice 10c: the arms are quotation literals now, so the disagreement
        // is caught at the *argument* site (R-P2-3), comparing one arm's
        // actual exit shape against its sibling's, rather than at the join.
        let src = ": w ( bool -- i64 ) ~[ 1 1 ] ~[ 1 ] if ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("leave different stack shapes"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_branch_arms_disagreeing_at_the_join_name_the_arms_not_if() {
        // The join's two diagnostics are reachable only through a direct
        // `branch`: written as `if`, R-P2-3's argument-site check compares the
        // two arm literals first and reports its own message instead. So a
        // wording that blames `if` is wrong at every site that can actually
        // produce it. Nothing else exercises this path -- the two
        // `check_branch_*_is_error` tests both stop at the argument site -- so
        // the messages are pinned here.
        let depth = check_src(": w ( u32 -- i64 ) | c | c [ 1 1 ] [ 1 ] branch ;").unwrap_err();
        assert!(
            depth.contains("the two branch arms leave different stack depths (then: 2, else: 1)"),
            "unexpected message: {depth}"
        );
        let ty = check_src(": w ( u32 -- i64 ) | c | c [ 1 ] [ true ] branch ;").unwrap_err();
        assert!(
            ty.contains("the two branch arms leave different types (then: `i64`, else: `bool`)"),
            "unexpected message: {ty}"
        );
        for err in [&depth, &ty] {
            assert!(
                !err.contains("`if`"),
                "blames a word the user never wrote: {err}"
            );
        }
    }
    #[test]
    fn check_branch_join_types_agree_ok() {
        // Both arms leave a single `i64`: the join unifies cleanly.
        check_src(": w ( bool -- i64 ) ~[ 1 ] ~[ 2 ] if ;").unwrap();
    }
    #[test]
    fn check_branch_join_type_mismatch_is_error() {
        // `then` leaves an `i64`, `else` leaves a `bool`: same depth, different type.
        let src = ": w ( bool -- i64 ) ~[ 1 ] ~[ true ] if ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("leave different stack shapes"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }
    #[test]
    fn check_declared_output_mismatch_is_error() {
        let src = ": w ( -- i64 ) 1 1 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("body leaves 2 values"));
        assert!(err.contains("declares 1 outputs"));
    }
    #[test]
    fn check_word_duplicate_local_is_error() {
        let src = ": w ( i64 i64 -- i64 ) | a a | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("duplicate local"), "unexpected message: {err}");
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`w`"), "unexpected message: {err}");
    }
    #[test]
    fn check_duplicate_word_in_one_file_is_error() {
        // Two `push` words in one file used to reach codegen silently (the
        // env-population loop kept only the last), surfacing only as a bare
        // linker `symbol already defined` error at the very end of the
        // pipeline. Now it is a located compiler diagnostic.
        let err =
            check_src(": push ( -- i64 ) 1 ;\n: push ( -- i64 ) 2 ;\n: main ( -- ) push drop ;")
                .unwrap_err();
        assert!(
            err.contains("duplicate word `push`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("first defined at line 1"),
            "names the first definition's location too: {err}"
        );
    }
    #[test]
    fn check_duplicate_empty_bodied_word_reports_a_real_location() {
        // Regression: `word_span` used to derive a word's location from its
        // first term/clause, so an empty body (`terms.first()` is `None`)
        // fell back to `Span::default()` -- line 0, col 0 -- for every
        // trivial stub word, `main ( -- )` being the single most common
        // shape that hits it. `WordDef` now carries its own declaration span
        // (the name token), independent of body shape.
        let err = check_src(": main ( -- ) ;\n: main ( -- ) ;\n").unwrap_err();
        assert!(
            err.contains("duplicate word `main`") && err.contains("line 2"),
            "names the repeat's real location: {err}"
        );
        assert!(
            err.contains("first defined at line 1"),
            "names the first definition's real location, not line 0: {err}"
        );
    }
    #[test]
    fn check_unknown_word_is_error() {
        let src = ": w ( i64 -- i64 ) frobnicate ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown word"));
        assert!(err.contains("frobnicate"));
    }
    #[test]
    fn check_locals_exceed_inputs_is_error() {
        let src = ": w ( i64 -- i64 ) | a b | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("locals bind"));
    }
    #[test]
    fn check_type_propagates_through_body_expected() {
        // `0 >` yields a bool that `if` consumes; both arms leave an i64.
        check_src(": sign ( i64 -- i64 ) 0 > ~[ 1 ] ~[ 0 ] if ;").unwrap();
    }
    #[test]
    fn check_if_condition_not_bool_is_error() {
        let src = ": w ( -- i64 ) 5 ~[ 1 ] ~[ 2 ] if ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("expected `bool`"), "unexpected message: {err}");
        assert!(err.contains("found `i64`"), "unexpected message: {err}");
    }
    #[test]
    fn check_operand_type_mismatch_is_error() {
        let src = ": w ( -- i64 ) true 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }
    // Array words: fill / len type-checking.

    #[test]
    fn check_fill_len_happy_path_ok() {
        // `fill` builds `[i64 4]`; `len` is non-consuming (the array stays).
        check_src(": w ( -- ) 7 4 fill len drop drop ;").unwrap();
    }
    #[test]
    fn check_fill_output_type_is_the_array_shape() {
        // `fill` interns `[i64 4]` and the declared output must match it, so
        // this word type-checks with an array-typed output slot (R2/R3/R10).
        check_src(": w ( -- [i64 4] ) 7 4 fill ;").unwrap();
    }
    #[test]
    fn check_len_is_non_consuming_leaves_array_ok() {
        check_src(": w ( [i64 4] -- [i64 4] usize ) | a | a len ;").unwrap();
    }
    #[test]
    fn check_len_on_non_array_is_error() {
        // X8: `len` on a non-array operand names the word and the operand
        // type via `array_word_operand_error`.
        let err = check_src(": w ( i64 -- usize ) len ;").unwrap_err();
        assert!(
            err.contains("`len` requires an array operand"),
            "unexpected message: {err}"
        );
        assert!(err.contains("i64"), "should name the offending type: {err}");
    }
    #[test]
    fn check_constant_index_out_of_range_is_error() {
        // X4/R11: a literal index >= N is a sharp located compile error naming
        // the length and the index. Index (9) and length (4) are deliberately
        // distinct so a swapped-arg diagnostic bug can't hide behind a
        // same-valued assertion.
        let err = check_src(": w ( [i64 4] -- ) | a | &a 9 &> drop ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
        assert!(err.contains('9'), "should name the index: {err}");
        assert!(err.contains('4'), "should name the length: {err}");
    }
    #[test]
    fn check_constant_index_at_length_boundary_is_error() {
        // Index == length is the first invalid index (valid range is
        // 0..length-1); this off-by-one boundary is distinct from the
        // gross-violation case above and must be rejected too.
        let err = check_src(": w ( [i64 4] -- ) | a | &a 4 &> drop ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
        assert!(err.contains("index 4"), "should name the index: {err}");
        assert!(err.contains("length 4"), "should name the length: {err}");
    }
    #[test]
    fn check_computed_index_without_conversion_is_error() {
        // X10: a computed (non-literal) `i64` index needs an explicit `>usize`.
        let err = check_src(": w ( [i64 4] i64 -- ) | a n | &a n &> drop ;").unwrap_err();
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }
    #[test]
    fn check_fill_non_literal_count_is_error() {
        // M1: the count must be a compile-time literal; a computed count errors.
        let err = check_src(": w ( i64 -- ) | n | 0 n fill drop ;").unwrap_err();
        assert!(err.contains("literal count"), "unexpected message: {err}");
    }
    #[test]
    fn check_fill_zero_count_is_error() {
        // A `fill` count < 1 is invalid (an array length must be >= 1).
        let err = check_src(": w ( -- ) 0 0 fill drop ;").unwrap_err();
        assert!(
            err.contains("length must be >= 1"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_fill_of_linear_element_is_error() {
        // `fill` has no per-slot `Copy` gate today (unlike `dup`/`over`), and
        // array-element linearity isn't tracked transitively, so a linear
        // element is rejected rather than silently replicated/leaked.
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 0 Spy 3 fill drop ;")).unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_fill_of_linear_struct_element_is_error() {
        // The same rejection applies transitively: a struct that is linear
        // because one of its fields is (R7) is just as unsupported as a bare
        // `Spy` element.
        let err = check_src(&format!(
            "{SPY_DEF}type: Holder xs Spy ;\n: w ( -- ) 0 Spy Holder 3 fill drop ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holder`"), "unexpected message: {err}");
    }
    #[test]
    fn fill_forwards_surviving_set_so_a_returned_array_rejects_an_escaping_capture() {
        // D4/R19: `check_array_word`'s "fill" arm forwards the element's
        // surviving-capture-set (`let surviving = element.surviving;`) onto
        // the array it produces, exactly as a struct/enum constructor's
        // output does -- the array is the closure's carrier now, having
        // replicated it N times. `Boxed>f` materializes a quotation field
        // getter's output with its surviving set intact (the R19/R22 comment
        // on the generic accessor path), so `b Boxed>f` hands `fill` an
        // already-erased closure whose surviving set has one frame-rooted
        // member (`r`, a reference into `mk`'s own local `arr`). If `fill`
        // did not forward that set onto the array, `mk`'s R22 word-output
        // walk over its final stack would find nothing suspicious and wrongly
        // accept a program that returns a carrier holding a dangling
        // reference into a frame that no longer exists.
        //
        // This is the only place this forwarding can be tested: `Slot`/
        // `surviving` is check-time-only and never reaches the IR, so no
        // IR-level assertion can exercise it (see `ir::tests`'s renamed
        // `fill_lowering_result_reaches_a_reference_consumer`). Mutating
        // `check.rs`'s forwarding line to `let surviving = None;` makes this
        // program wrongly build (verified by hand); restoring it makes this
        // assertion hold.
        let err = check_src(
            "type: Boxed f [ -- i64 ] ;\n\
             : mk ( -- [ [ -- i64 ] 4 ] )\n\
             0 4 fill | arr |\n\
             &arr | r |\n\
             [ r 0 >usize &> @ ] Boxed | b |\n\
             b Boxed>f\n\
             4 fill ;\n\
             : main ( -- ) mk drop ;\n",
        )
        .expect_err(
            "an escaping frame-rooted capture, carried through `fill`'s array, must be rejected",
        );
        assert!(
            err.contains("an escaping closure captures `r`")
                && err.contains("does not survive the return"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_equality_on_array_is_error() {
        // X7/R13: `=` on arrays reaches the operand guard naming the type.
        let err = check_src(": w ( -- bool ) 0 4 fill 0 4 fill = ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }
    #[test]
    fn check_arithmetic_on_array_is_error() {
        // X7/R13: `+` on arrays reaches the operand guard naming the type
        // (the diagnostic covers `=` *and* arithmetic; both are exercised).
        let err = check_src(": w ( -- [i64 4] ) 0 4 fill 0 4 fill + ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }
    #[test]
    fn check_two_spellings_of_same_shape_are_one_type_ok() {
        // R8: structural dedup means `[i64 4]` in two positions is one type, so
        // an `[i64 4]` argument satisfies an `[i64 4]`-typed word.
        check_src(
            ": mk ( -- [i64 4] ) 0 4 fill ;\n: use ( [i64 4] -- i64 ) | a | &a 0 &> @ ;\n: w ( -- i64 ) mk use ;",
        )
        .unwrap();
    }
    #[test]
    fn check_branch_join_float_widths_mismatch_is_error() {
        // `if` branches leaving `f32` vs `f64` disagree at the join (R12).
        let src = ": w ( bool -- f64 ) ~[ 1.0 >f32 ] ~[ 2.0 ] if ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("leave different stack shapes"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }
    #[test]
    fn check_branch_join_float_types_agree_ok() {
        check_src(": w ( bool -- f64 ) ~[ 1.0 ] ~[ 2.0 ] if ;").unwrap();
    }
    #[test]
    fn infer_line_net_effect_expected() {
        assert_eq!(infer_src("2 3 +", &[]).unwrap(), vec![Type::I64]);
    }
    #[test]
    fn infer_line_carries_entry_depth() {
        // `2 +` from a carried `i64`: the literal plus the carried slot are
        // consumed by `+`, leaving one `i64`.
        assert_eq!(infer_src("2 +", &[Type::I64]).unwrap(), vec![Type::I64]);
    }
    #[test]
    fn infer_line_carries_slot_types_expected() {
        // A comparison line leaves a `bool` on the carried stack.
        assert_eq!(infer_src("5 3 >", &[]).unwrap(), vec![Type::BOOL]);
    }
    #[test]
    fn line_underflow_against_carried_stack_is_error() {
        let err = infer_src("+", &[Type::I64]).unwrap_err();
        assert!(err.contains("stack underflow"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
        assert!(err.contains("holds 1"), "unexpected message: {err}");
    }
    #[test]
    fn infer_line_unknown_word_is_error() {
        let err = infer_src("frobnicate", &[]).unwrap_err();
        assert!(err.contains("unknown word"), "unexpected message: {err}");
        assert!(err.contains("frobnicate"), "unexpected message: {err}");
    }
    #[test]
    fn check_dup_of_linear_value_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy dup drop drop ;")).unwrap_err();
        assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }
    #[test]
    fn check_over_of_linear_value_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( -- ) 7 Spy 1 over drop drop drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("cannot `over`"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_shuffles_that_only_reorder_linear_values_are_ok() {
        // `swap`/`rot` move rather than copy, so the `dup`/`over` gate must not
        // over-reach to them.
        check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy 8 Spy swap drop drop ;")).unwrap();
        check_src(&format!(
            "{SPY_DEF}: w ( -- ) 1 Spy 2 Spy 3 Spy rot drop drop drop ;"
        ))
        .unwrap();
    }
    #[test]
    fn check_use_after_move_of_linear_local_names_the_move_site() {
        // `SPY_DEF` is two lines, so `w`'s own line 3 (the first `s drop`)
        // lands on line 5 of the full source.
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy -- )\n  | s |\n  s drop\n  s drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(
            err.contains("moved at line 5, col 3"),
            "the diagnostic should name the move site: {err}"
        );
    }
    #[test]
    fn check_surplus_linear_value_is_a_linear_flavoured_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy ;")).unwrap_err();
        assert!(
            err.contains("linear value left on the stack"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_surplus_copy_value_keeps_the_arity_error() {
        // No misfire: the linear branch must not swallow the Copy surplus case.
        let err = check_src(": w ( -- ) 1 ;").unwrap_err();
        assert!(
            err.contains("body leaves 1 values"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("linear"), "unexpected message: {err}");
    }
    #[test]
    fn check_linear_local_consumed_in_both_arms_is_ok() {
        // R14: `Moved` in both arms joins to `Moved`, not `MaybeMoved`, even
        // though the two move sites differ.
        check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c ~[ s drop ] ~[ s drop ] if ;"
        ))
        .unwrap();
    }
    #[test]
    fn check_linear_local_moved_in_one_arm_then_used_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 1 . ] if\n  s drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_linear_local_moved_in_one_arm_and_dropped_nowhere_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 1 . ] if ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("not consumed on every path"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_linear_value_across_self_tail_call_is_error() {
        // R15: the fresh Spy pushed in the recursive arm leaves `s` live
        // across the back-edge, which the loop lowering cannot dispose yet.
        // `SPY_DEF` is two lines, so `spin`'s own line 3 lands on line 5.
        let err = check_src(&format!(
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 = ~[ s drop 0 ] ~[ 9 Spy n 1 - spin ] if ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("line 5"), "the error should be located: {err}");
    }
    #[test]
    fn check_linear_value_forwarded_into_the_self_tail_call_is_ok() {
        // Moved *into* the recursive call's arguments, the Spy is forwarded,
        // not stranded, so the R15 guard must not fire.
        check_src(&format!(
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 = ~[ s drop 0 ] ~[ s n 1 - spin ] if ;"
        ))
        .unwrap();
    }
    #[test]
    fn check_copy_self_tail_call_is_unaffected_by_the_linear_guard() {
        check_src(&std::fs::read_to_string("examples/countdown.sth").unwrap()).unwrap();
    }
    #[test]
    fn infer_line_consumes_a_carried_linear_slot_ok() {
        // The REPL path: a residual linear slot can be dropped by a later
        // line (no scope-end rule applies to a bare line). `^i64` (an owning
        // cell, always linear) stands in for a linear entry slot, since this
        // test exercises `infer_line` directly with no struct/enum registry.
        let mut owned_cells = Vec::new();
        let cell_ty = intern_owned_cell_type(&mut owned_cells, Type::I64);
        let out = infer_src("drop", &[cell_ty]).unwrap();
        assert!(out.is_empty());
    }
    /// Slice 10a (R12): the self-call's arguments are checked against the
    /// *ground* declared inputs, with a located diagnostic. The witness must
    /// diverge from the standalone check, which binds every type variable to
    /// `i64` and so cannot see a wrong `'a`: `loopy` is polymorphic in `'a`
    /// and its back-edge passes a hardcoded `i64` literal in the `'a` slot.
    /// Standalone (`'a = i64`) accepts it; the instantiation `'a = str` from
    /// `main` grounds the marker at `str`, and only the R12 unify catches the
    /// `i64`. (Removing the R12 loop makes this program compile clean -- a
    /// silent soundness hole -- so the test is not a placebo.)
    #[test]
    fn back_edge_rejects_mismatched_self_call_argument() {
        let src = ": loopy inline ( ..s 'a i64 ~[ ..s 'a -- ..s ] -- ..s )\n\
                   | f | | n | | acc |\n\
                   n 0 > ~[\n\
                   acc f call\n\
                   5 n 1 - f loopy\n\
                   ] ~[\n\
                   ] if ;\n\
                   : main ( -- ) \"x\" 3 ~[ drop ] loopy ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("type mismatch in `main`"),
            "located at the instantiation: {err}"
        );
        assert!(
            err.contains("`loopy` expected `str`, found `i64`"),
            "names the callee and both types: {err}"
        );
    }
    /// Slice 10a (R12): `while` -- the symmetric shape whose back-edge produced
    /// the carried input pre-rewrite and produces the ground declared output
    /// post-rewrite (they agree at 1<->1) -- must still type-check identically.
    #[test]
    fn while_self_tail_still_checks_after_back_edge_rewrite() {
        check_src(
            ": while inline ( 'a ~[ 'a -- 'a bool ] -- 'a ) | p | p call ~[ p while ] ~[ ] if ;\n",
        )
        .expect("`while` still type-checks after the back-edge rewrite");
    }
}
