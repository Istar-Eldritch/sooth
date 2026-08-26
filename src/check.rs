//! Stack-effect checker. Simulates a compile-time virtual stack of concrete
//! `Type`s through each word body and verifies the net effect matches the
//! declared signature.
//!
//! Every operand is checked against the type its consumer expects, so a
//! `bool` where `add` wants an `i64` is a located compile error (Forth's silent
//! coercion failure mode becomes a diagnostic here). Branch join points unify
//! on both depth and per-slot type: the `then` and `else` arms must leave the
//! same stack shape.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::ast::{
    generic_surface_name, ground_member_type, instantiation_symbol, intern_array_type,
    intern_bundle_struct, intern_owned_cell_type, intern_ref_type, intern_slice_type,
    is_builtin_word_name, is_name_dispatched_builtin, resolve_bool_type, variant_type, ArrayDecl,
    Bound, CallInst, EnumDecl, EnumId, ExternDecl, GenericEnumDecl, GenericStructDecl, Image,
    ImplDecl, ImplTarget, Len, Module, ModuleInfo, OwnedCellDecl, PolyCrossCall, PolySig, PolyType,
    QuotAnnot, QuotEffect, RefDecl, SliceDecl, Span, StackEffect, StaticDecl, StructDecl, StructId,
    Subst, Term, TermKind, TraitDecl, TraitId, TraitMember, Type, TypedSlot, VariantDecl,
    VariantTag, VariantTagMode, WordDef, RESERVED_TRAIT_MODULE,
};

mod audits;
mod builtins;
mod captures;
mod combinators;
mod declarations;
mod drop_graph;
mod engine;
mod globals;
mod operators;
mod poly;
mod terms;
mod word_entry;
mod word_families;

use self::audits::*;
pub(crate) use self::audits::{
    audit_quotation_type_registries, audit_word_quotation_positions, drop_overload_struct_id,
    find_drop_overloads, reject_owning_quotation_declarations,
};
pub use self::builtins::builtin_table;
use self::builtins::*;
pub(crate) use self::builtins::{
    is_builtin_operator_name, is_copy, is_linear, sig_of, Overload, Sig, COMPARISON_PRIMITIVES,
};
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
    check_exported_signatures, check_impl_decls, check_no_word_shadows_eliminator,
    check_selective_imports, check_slice_element_gate, check_static_decls, check_trait_decls,
    check_types, enum_generated_sigs, selective_not_exported_error, struct_generated_sigs,
    variant_generated_sigs, word_shadows_eliminator_error, SelectiveName,
};
use self::drop_graph::*;
pub(crate) use self::drop_graph::{
    check_drop_overload_reachability, has_self_tail_call, terms_tail_call_self,
};
use self::engine::*;
pub(crate) use self::globals::check_globals;
use self::operators::*;
use self::poly::*;
pub(crate) use self::poly::{
    check_poly_body, check_poly_combinator_repl, poly_type_str, CrossCtx, TraitCtx,
};
use self::terms::borrow_join_disagreement_error;
use self::terms::check_terms;
use self::terms::check_terms_relaxed;
use self::terms::eliminator_arm_outside_call_error;
use self::terms::tagged_literal_reaches_an_eliminator_call;
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
type ResolvedCalls = (
    HashMap<Span, CallInst>,
    HashMap<Span, String>,
    HashMap<Span, (StructId, usize)>,
    HashMap<Span, (EnumId, usize, usize)>,
);

/// `ResolvedCalls` plus the residual stack a REPL line leaves behind and the
/// field projections it resolved (R2), which the session's own lowering path
/// needs exactly as the module path does.
type InferredLine = (
    Vec<Type>,
    HashMap<Span, CallInst>,
    HashMap<Span, String>,
    HashMap<Span, (StructId, usize)>,
    HashMap<Span, (EnumId, usize, usize)>,
);

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
    /// overload of a builtin-named word (`Vec2 add` -> the user `add`), span ->
    /// resolved callee name, relayed onto `Module::builtin_overloads` so
    /// lowering emits an `Instr::Call` there instead of the builtin
    /// instruction. Scratch (discarded) on the REPL/combinator paths, which do
    /// not lower a builtin overload (out of scope this slice).
    builtin_overloads: &'a mut HashMap<Span, String>,
    /// P7 slice 1 (R2): the receiver-directed field projections this walk
    /// resolved (`&hp` against a `&Sprite` on the stack), span -> the struct
    /// and field index, relayed onto `Module::resolved_fields` so lowering can
    /// read back a resolution it cannot re-derive without a checker stack.
    /// Rides `PolyCtx` rather than `Scope` for the same reason
    /// `builtin_overloads` does: an `if` arm clones the scope, and a record
    /// made in one arm must outlive it.
    resolved_fields: &'a mut HashMap<Span, (StructId, usize)>,
    /// Phase 6 slice 3 (R6): the receiver-directed variant-field projections
    /// this walk resolved (`&r` against a `Type::Variant` on the stack), span
    /// -> `(EnumId, variant index, field index)`, relayed onto
    /// `Module::resolved_variant_fields`. Mirrors `resolved_fields` for the
    /// same reason: an `if` arm clones the scope, and a record made in one
    /// arm must outlive it.
    resolved_variant_fields: &'a mut HashMap<Span, (EnumId, usize, usize)>,
    /// Slice 6a (R18): the monomorphic quotation-taking words, keyed by name,
    /// so a call to one is intercepted and its body spliced against the live
    /// stack (the compiler's only inliner) rather than lowered to an
    /// `Instr::Call` to a word that mints no `IrFunc` (R20). Empty on the REPL
    /// paths, where defining such a word is rejected up front (R23).
    combinators: &'a CombinatorEnv<'a>,
    /// Phase 6 slice 3 (R3): the generated eliminator words, bare surface name
    /// (`Shape?`) -> the enum they eliminate, so a call to one is routed to
    /// `check_eliminator_call` ahead of the env/combinator/poly paths. An
    /// eliminator has no body, so it is not a `Combinator` and must never be
    /// spliced.
    eliminators: &'a HashMap<String, EnumId>,
    /// P7.S3e (R8): the tables a `Bound::User` at a resolved call site is
    /// decided and resolved against. `TraitResolveCtx::scratch()` on the REPL
    /// paths, which can carry no user bound at all.
    trait_resolve: TraitResolveCtx<'a>,
    /// P7.S4 (R6): the `(member_word_name, subst)` pairs for generic-impl
    /// dispatches discovered during this walk, collected so lowering can emit
    /// the polymorphic member word's body under the instantiation symbol.
    /// Empty on the REPL/combinator-scratch paths, which declare no `impl:`.
    impl_monos: &'a mut Vec<(String, crate::ast::Subst)>,
    /// P7.S3o (R1/R2): per-splice instantiation records, written by
    /// `check_poly_call` when `prov.splice_uid` is `Some`. Scratch (discarded)
    /// on the standalone/REPL paths; the module-level table on the main path.
    splice_records: &'a mut HashMap<(u32, Span), CallInst>,
    /// P7.S3o Phase 3: per-splice trait-member-call resolutions, written by
    /// the dispatch injection in `check_term` when a bare trait member is
    /// resolved at a splice site. Scratch (discarded) on the standalone/REPL
    /// paths; the module-level table on the main path.
    splice_trait_calls: &'a mut HashMap<(u32, Span), String>,
    /// P7.S3o Phase 3: the combinator's own `PolySig` and concrete θ, set
    /// during both the standalone check (i64 stand-in) and the splice walk
    /// (concrete θ from `check_poly_combinator_args`). When set, a bare trait
    /// member call in the body resolves against this θ instead of falling
    /// through to `env.get` as an unknown word. Owned (cloned) because the
    /// sig/subst are local to the caller and `PolyCtx` outlives them.
    combinator_sig: Option<PolySig>,
    combinator_subst: Option<Subst>,
    /// P7.S3o Phase 4 (R5): the combinator's own name, set alongside
    /// `combinator_sig`/`combinator_subst` during the splice walk and the
    /// standalone check. Used by the materialized-quotation bound-dispatch
    /// rejection to name the combinator in the error message.
    combinator_name: Option<String>,
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
    span: Span,
    is_inline: bool,
    /// Phase 6 slice 1: the literal's own declared effect, already resolved to
    /// concrete types by `resolve_annotation` at the interning site (`None`
    /// for every unannotated literal).
    annot: Option<AnnotEffect>,
}

/// Phase 6 slice 1 (R1): a quotation annotation once every slot has been
/// resolved to a concrete `Type`. A `QuotAnnot` may still name variables the
/// literal itself cannot bind (R2), so the resolution is fallible and happens
/// once, where the literal is interned; everything downstream reads plain
/// types.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnnotEffect {
    inputs: Vec<Type>,
    outputs: Vec<Type>,
    /// The annotation's opening `(`, where both its diagnostics locate.
    span: Span,
    /// Phase 6 slice 3 (R1/R4): the variant an eliminator arm handles, carried
    /// through from `QuotAnnot::variant_tag`. `None` for every plain literal.
    variant_tag: Option<VariantTag>,
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
/// (`add sub mul eq lt gt lte gte ne mod and or xor`): the operands' common `Type` once
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

/// Renders one enum variant payload field for a diagnostic. An attributeless
/// (positional) field has no user-written name, so it is named by its index
/// instead: `POSITIONAL_FIELD_NAME` is an internal placeholder and must never
/// surface in a message.
pub(crate) fn variant_field_desc(field: &str, idx: usize) -> String {
    if field == crate::parser::POSITIONAL_FIELD_NAME {
        format!("field {idx}")
    } else {
        format!("field `{field}`")
    }
}

/// Phase 6 slice 2 (R4): a variant's field types in declared order (first
/// field deepest), value-mode as the plain field type, ref-mode via
/// `intern_ref_type`. Shared by the destructure-word path and the accessor
/// path (Phase 3), so both project a variant's payload identically.
pub(crate) fn variant_field_projection(
    variant: &VariantDecl,
    ref_mutable: Option<bool>,
    refs: &mut Vec<RefDecl>,
) -> Vec<Type> {
    variant
        .fields
        .iter()
        .map(|(_, ty)| match ref_mutable {
            Some(mutable) => intern_ref_type(refs, *ty, mutable),
            None => *ty,
        })
        .collect()
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
        // P7 slice 3c (R6): a slice is pointer-shaped like the three above, so
        // an all-zero one is a null element pointer with a zero length, not an
        // empty view of anything. Named explicitly because the wildcard below
        // treats what it does not name as zero-*safe*.
        Type::Str | Type::Cstr | Type::Quotation(_) | Type::Slice(..) => Some((ty, Vec::new())),
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
                for (idx, (fname, fty)) in variant.fields.iter().enumerate() {
                    if let Some((bad, mut path)) =
                        find_zero_unsafe_element(*fty, structs, enums, arrays)
                    {
                        path.insert(
                            0,
                            format!(
                                "variant `{}` {}",
                                variant.name,
                                variant_field_desc(fname, idx)
                            ),
                        );
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

/// P7.S3s (R6, review fix): `Ord` is no longer a reserved `Bound` variant, so
/// the two overload-admission filters that used to ask `is_ord`
/// (`poly_admits`/`poly_sig_could_match`) need `Ord`'s own `TraitId` to look
/// a candidate type up in the whole-program `impl:` registry. This resolves
/// it from *that specific candidate's own* `sig.bounds` rather than a
/// whole-program "first trait named `Ord`" search: the parser already
/// resolved the correct `Ord` for this declaration (own-module shadowing and
/// Phase 0's hub-reexport walk both included) when it parsed the bound, and
/// recorded it as a `Bound::User(tid)` on the signature. A first-match global
/// search is module-blind and fails open the moment any module in the build
/// declares its own unrelated `trait: Ord` -- the earlier version of this
/// function found *that* trait first and every admission filter using it
/// stopped filtering at all. `None` if `v` carries no bound naming a trait
/// called `Ord`.
fn ord_trait_id(sig: &PolySig, v: u32, traits: &[TraitDecl]) -> Option<TraitId> {
    sig.bounds.iter().find_map(|(bv, bound)| {
        if *bv != v {
            return None;
        }
        match bound {
            Bound::User(tid) if traits.get(tid.index()).is_some_and(|t| t.name == "Ord") => {
                Some(*tid)
            }
            _ => None,
        }
    })
}

pub fn check(module: &mut Module) -> Result<(), String> {
    check_module(module).map(|_| ())
}

/// `check`, plus the trait obligations R17's pre-pass collected -- the one
/// artifact of a check run that is otherwise invisible from outside, since
/// nothing stores it on `Module` (a resolved obligation rides `CallInst`
/// instead).
fn check_module(module: &mut Module) -> Result<Vec<WordObligations>, String> {
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
        &module.generic_structs,
        &module.generic_enums,
        &module.arrays,
        &module.owned_cells,
        &module.slices,
    )?;

    // R7a: a quotation type is legal only as a direct word parameter this
    // slice; reject it at every other position before layout or lowering can
    // see it, so R7's `unreachable!` mangling/`IrType` arms stay unreached.
    audit_quotation_type_positions(module)?;

    // Builtins are resolved by table (`BUILTIN_TABLE`) inside `check_operator`,
    // not by env lookup, so the concrete env holds only user/generated words.
    let mut env: HashMap<String, Vec<Overload>> = HashMap::new();
    // D7/R5: keyed by the bare surface name (`struct_generated_sigs`'s first
    // element), appended rather than inserted -- two instantiations of one
    // generic `type:` share this key, so an `env.insert` here would let the
    // second silently clobber the first's constructor/accessor entry. Each
    // candidate's `Overload::symbol` stays the mangled per-instantiation
    // spelling, so the operand-type match below still picks the right one.
    for (name, symbol, sig) in struct_generated_sigs(&module.structs) {
        env.entry(name).or_default().push(Overload { sig, symbol });
    }
    for (name, symbol, sig) in enum_generated_sigs(&module.enums) {
        env.entry(name).or_default().push(Overload { sig, symbol });
    }
    for (name, symbol, sig) in variant_generated_sigs(&module.enums) {
        env.entry(name).or_default().push(Overload { sig, symbol });
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
    check_generic_concrete_overlap(
        &module.words,
        &module.traits,
        &module.impls,
        &module.arrays,
        &module.owned_cells,
        &module.refs,
    )?;
    // Two poly words (or two poly combinators) declaring the exact same
    // signature under one name are rejected before either enters `poly_env`
    // below -- unresolvable ambiguity, not a legitimate second overload.
    check_duplicate_poly_signatures(&module.words)?;
    // Phase 6 slice 3 review fix (smaller point 1): a word sharing a
    // generated eliminator's name would be silently unreachable -- rejected
    // here, before the eliminator registry below is built, the same as any
    // other name collision this module already rejects up front.
    check_no_word_shadows_eliminator(&module.words, &module.enums)?;

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
    // Phase 6 slice 3 (R2/R3): the generated eliminator words. The signature
    // registers so the name is present in the environment like any other
    // generated word; the registry beside it is what a call site is actually
    // routed by, since an eliminator's arms are matched to variants by
    // annotation tag rather than unified slot by slot.
    //
    // Review fix (cycle 3): so this registration changes no diagnostic and no
    // dispatch -- `check_term`'s interception precedes every env/poly lookup
    // unconditionally, and a user word colliding with the name is rejected
    // outright by `check_no_word_shadows_eliminator` above. It is the
    // generator's only consumer in this phase (the REPL's own env assembly
    // builds the registry without it), and the `PolySig` becomes load-bearing
    // in Phase 4, where the lowering symbol beside it mints `EnumWord::
    // Eliminate`. Deleted, nothing observable changes and the generator R2
    // exists to add becomes dead code.
    for (name, _symbol, sig) in enum_eliminator_sigs(&module.enums) {
        poly_env.entry(name).or_default().push((sig, None));
    }
    let eliminators = eliminator_registry(&module.enums);
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
        slices,
        generic_structs: _,
        generic_enums: _,
        externs: _,
        instantiations: _,
        poly_cross_calls: _,
        transitive_instantiations: _,
        splice_records: _,
        splice_trait_calls: _,
        builtin_overloads: _,
        resolved_fields: _,
        resolved_variant_fields: _,
        modules,
        statics,
        generics,
        traits,
        impls,
    } = module;
    // P7 slice 3a phase 2 (R2): the live instantiator, wrapped so a poly
    // body's own construction (`poly_call_term`'s new arm) and the grounding
    // arms (`unify_poly_input`/`apply_subst`) can mint through `Ctx` despite
    // `Ctx` otherwise only ever borrowing immutably -- see `Ctx::generics`.
    // Taken out of the field (not just re-borrowed) because `structs`/`enums`
    // above are already `&mut Vec<_>` split off the same `&mut Module`; a
    // `RefCell` around a *separate* value sidesteps aliasing either.
    let generics_cell = RefCell::new(std::mem::take(generics));
    // R6: each body's own `drop` call sites, resolved to a concrete operand
    // type by the walk that checks it. Collected per word so the graph below
    // knows which body each site sits in.
    let mut dropped: Vec<Vec<Type>> = Vec::with_capacity(words.len());
    // Slice 11 (R3): reject a declared `inline` the splice cannot deliver on
    // *before* `is_combinator` is consulted, so an `inline` word the splice
    // cannot deliver on never enters the splice env, the cycle graph, or the
    // poly checker (which would take it for a legitimate poly combinator).
    for word in words.iter() {
        check_inline_declaration(word)?;
        check_inline_quotation_requires_inline(word)?;
    }
    // R18: the monomorphic quotation-taking words, gathered once so a call to
    // one is intercepted and inlined (term-splice) rather than lowered to a
    // call. A polymorphic combinator's body is checked by the poly pass, so it
    // is not registered here; only a monomorphic word with a `Type::Quotation`
    // input qualifies.
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
    // P7.S4 (R6): the `(member_word_name, subst)` pairs for generic-impl
    // dispatches discovered during the call-site loop, used to seed
    // `discover_transitive_instantiations` so lowering emits the polymorphic
    // member word's body under the instantiation symbol.
    let mut impl_monos: Vec<(String, crate::ast::Subst)> = Vec::new();
    // P7.S3o (R1/R2): per-splice instantiation records, filled as each
    // spliced combinator body's calls to polymorphic words are unified, then
    // stored on the module for lowering.
    let mut splice_records: HashMap<(u32, Span), CallInst> = HashMap::new();
    // P7.S3o Phase 3: per-splice trait-member-call resolutions (bare member →
    // resolved `impl:` symbol), filled as each spliced combinator body's
    // bare member calls resolve against the concrete splice θ, then relayed
    // to the module for lowering.
    let mut splice_trait_calls: HashMap<(u32, Span), String> = HashMap::new();
    // Slice 8a phase 2 (R7): the builtin-name overload dispatch sites, filled
    // as each monomorphic body's operator calls resolve, then relayed to the
    // module for lowering (empty for the whole corpus, so its lowering is
    // untouched byte-for-byte).
    let mut builtin_overloads: HashMap<Span, String> = HashMap::new();
    // P7 slice 1 (R2): the receiver-directed field projections, filled as each
    // monomorphic body resolves one against its stack, then relayed to the
    // module for lowering.
    let mut resolved_fields: HashMap<Span, (StructId, usize)> = HashMap::new();
    // Phase 6 slice 3 (R6): the receiver-directed variant-field projections,
    // filled as each monomorphic body resolves one against its stack, then
    // relayed to the module for lowering.
    let mut resolved_variant_fields: HashMap<Span, (EnumId, usize, usize)> = HashMap::new();
    // P7.S3e (R17, decision 10): every non-combinator polymorphic body is
    // checked *here*, before the loop below reaches any call site, rather than
    // in source order interleaved with the monomorphic words. `check_poly_body`
    // is what records a body's trait obligations, and a monomorphic word
    // declared ahead of the polymorphic word it calls would otherwise reach the
    // call-site bound loop while its callee's obligation list was still empty --
    // an order-dependent silent miss, not a diagnostic.
    //
    // This *replaces* the in-loop call rather than supplementing it: a body is
    // checked exactly once, just earlier. Two consequences the loop below no
    // longer produces: every poly-body diagnostic in a module now precedes
    // every monomorphic one, and generic-struct ids mint in poly-word order
    // ahead of monomorphic-word order. A combinator stays on the in-loop
    // `check_poly_combinator_standalone` path, which records nothing that
    // survives it (R9's scope cut).
    let mut trait_obligations: Vec<WordObligations> = Vec::new();
    // P7.S3k (R2): the generic-to-generic calls each of those bodies makes,
    // recorded symbolically as it is walked (its own `'T` is still rigid here,
    // so there is no θ to ground them against) and relayed to the module for
    // the composition step.
    let mut poly_cross_calls: HashMap<String, Vec<PolyCrossCall>> = HashMap::new();
    for word in words.iter() {
        let Some(sig) = &word.poly else { continue };
        if is_combinator(word) {
            continue;
        }
        let mut obligations = Vec::new();
        let mut cross_calls = Vec::new();
        // P7 slice 3a phase 2 (R2): `check_poly_body` rebases itself at entry
        // (to the live registries' current length); flushed right after it
        // returns, so a mint this body triggers lands at an id the very next
        // body's check can already see.
        check_poly_body(
            word,
            sig,
            &env,
            &combinators,
            structs,
            enums,
            arrays,
            owned_cells,
            refs,
            slices,
            statics,
            Some(modules),
            &mut builtin_overloads,
            &mut TraitCtx {
                traits,
                obligations: &mut obligations,
            },
            &mut CrossCtx {
                env: &poly_env,
                calls: &mut cross_calls,
            },
            Some(&generics_cell),
        )?;
        {
            let mut g = generics_cell.borrow_mut();
            g.flush_structs_into(structs);
            g.flush_enums_into(enums);
        }
        if !cross_calls.is_empty() {
            poly_cross_calls
                .entry(word.name.clone())
                .or_default()
                .extend(cross_calls);
        }
        trait_obligations.push(WordObligations {
            name: word.name.clone(),
            sig: (**sig).clone(),
            obligations,
        });
    }
    // P7.S3e (R8): the tables every bound-directed call site below resolves
    // against, complete only now that the pre-pass has recorded every
    // non-combinator poly body's obligations. The combinator-standalone path
    // gets the same tables, not scratch ones: its instantiation records are
    // scratch, but its bounds are real, and a combinator body calling a
    // bounded poly word resolves that call through here -- against scratch
    // tables, whose trait table holds only the two predicate entries, a user
    // `TraitId` indexes past the end (pinned by
    // `a_bounded_call_inside_a_combinator_body_resolves`).
    let trait_resolve = TraitResolveCtx {
        traits,
        impls,
        word_symbols: &symbols,
        recorded: &trait_obligations,
    };
    for word in words.iter() {
        let mut sites = Vec::new();
        if let Some(sig) = &word.poly {
            if is_combinator(word) {
                // P7.S3o: the `reject_user_bound_on_combinator` gate is
                // removed — bounded combinators now proceed to the standalone
                // check. A bounded poly word (like `gt` with `'T: Ord`) is
                // handled by the per-splice mechanism: `check_poly_call`
                // redirects the inner CallInst to `splice_records`, carrying
                // the seeded `trait_calls` map. A bare member call (like
                // `cmp` directly) falls through the standalone check's
                // `env.get` as an unknown word until Phase 3's dispatch
                // injection lands.
                // R14-R17: a polymorphic combinator (`each`/`map`/`fold`) is
                // checked standalone by instantiating its signature at
                // concrete stand-in types and running the ordinary checker on
                // the body, which already handles the abstract quotation
                // `call`/`times` (R8/R9) and the three `times` obligations
                // (R16). It mints no `IrFunc` (R20): a call to it is inlined
                // by term-splice at its concrete call sites, so the
                // instantiation records it produces here are scratch.
                let mut scratch: HashMap<Span, CallInst> = HashMap::new();
                let mut scratch_splice: HashMap<(u32, Span), CallInst> = HashMap::new();
                let mut scratch_overloads: HashMap<Span, String> = HashMap::new();
                let mut scratch_monos: Vec<(String, crate::ast::Subst)> = Vec::new();
                let mut scratch_fields: HashMap<Span, (StructId, usize)> = HashMap::new();
                let mut scratch_variant_fields: HashMap<Span, (EnumId, usize, usize)> =
                    HashMap::new();
                let mut scratch_trait_calls: HashMap<(u32, Span), String> = HashMap::new();
                let mut poly = PolyCtx {
                    env: &poly_env,
                    insts: &mut scratch,
                    builtin_overloads: &mut scratch_overloads,
                    resolved_fields: &mut scratch_fields,
                    resolved_variant_fields: &mut scratch_variant_fields,
                    combinators: &combinators,
                    eliminators: &eliminators,
                    trait_resolve,
                    impl_monos: &mut scratch_monos,
                    splice_records: &mut scratch_splice,
                    splice_trait_calls: &mut scratch_trait_calls,
                    combinator_sig: None,
                    combinator_subst: None,
                    combinator_name: None,
                };
                check_poly_combinator_standalone(
                    word,
                    sig,
                    enums,
                    &env,
                    arrays,
                    owned_cells,
                    refs,
                    slices,
                    structs,
                    statics,
                    Some(modules),
                    &mut poly,
                )?;
            }
            // R7: a non-combinator polymorphic body was already checked by
            // the obligation pre-pass above (R17), so there is nothing to do
            // for it here.
        } else {
            let mut poly = PolyCtx {
                env: &poly_env,
                insts: &mut insts,
                builtin_overloads: &mut builtin_overloads,
                resolved_fields: &mut resolved_fields,
                resolved_variant_fields: &mut resolved_variant_fields,
                combinators: &combinators,
                eliminators: &eliminators,
                impl_monos: &mut impl_monos,
                trait_resolve,
                splice_records: &mut splice_records,
                splice_trait_calls: &mut splice_trait_calls,
                combinator_sig: None,
                combinator_subst: None,
                combinator_name: None,
            };
            // P7 slice 3a phase 2 (R2): a monomorphic caller instantiating a
            // poly word can ground a variable-bearing generic for the first
            // time too (`apply_subst`'s `Generic` arm), so this call gets the
            // same rebase/flush bracket as the poly-body one above.
            generics_cell
                .borrow_mut()
                .rebase(structs.len(), enums.len());
            check_word(
                word,
                enums,
                &env,
                arrays,
                owned_cells,
                refs,
                slices,
                structs,
                statics,
                Some(modules),
                &mut sites,
                &mut poly,
                Some(&generics_cell),
            )?;
            let mut g = generics_cell.borrow_mut();
            g.flush_structs_into(structs);
            g.flush_enums_into(enums);
        }
        dropped.push(sites);
    }
    // P7 slice 3a phase 2 (R2): restored onto the module once nothing is
    // still minting, so it survives into `ir::lower` (which reads it
    // read-only, `subst_polytype`'s find-only lookup).
    *generics = generics_cell.into_inner();

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
    // P7.S3o: a splice-derived CallInst with out_arity >= 2 needs the same
    // bundle. This runs before `discover_transitive_instantiations`, whose
    // early return on an empty `poly_cross_calls` table skips the
    // `intern_composed_bundles` pass for splice records — leaving
    // `bundle: None` and panicking at lowering when the multi-output return
    // value is never pushed onto the stack.
    for inst in splice_records.values_mut() {
        if inst.out_arity >= 2 {
            inst.bundle = Some(intern_bundle_struct(
                &mut module.structs,
                &inst.output_types,
            ));
        }
    }
    module.poly_cross_calls = poly_cross_calls;
    // P7.S3k (R4): a generic body's call to another generic word was recorded
    // symbolically, since the caller had no θ of its own when its body was
    // walked. Grounding it needs the concrete instantiations above *and* the
    // registry interning `apply_subst` performs, both of which live here and
    // neither of which lowering can redo -- so the transitive closure of
    // "which monomorphs does this program need" is computed at check time.
    module.transitive_instantiations = discover_transitive_instantiations(
        module,
        &mut insts,
        &mut splice_records,
        &symbols,
        &trait_obligations,
        std::mem::take(&mut impl_monos),
    )?;
    module.instantiations = insts;
    module.splice_records = splice_records;
    module.splice_trait_calls = splice_trait_calls;
    module.builtin_overloads = builtin_overloads;
    module.resolved_fields = resolved_fields;
    module.resolved_variant_fields = resolved_variant_fields;
    // P7.S3h (phase 2): last, after every type-level check, so a real type
    // error at an `owning` slot is reported instead of being masked by the
    // not-built-yet rejection. Phase 3 deletes this and supplies the `IrType`.
    for w in &module.words {
        reject_owning_quotation_declarations(w)?;
    }
    Ok(trait_obligations)
}

/// R10: one interned bundle struct per distinct output tuple of length >= 2,
/// over every declared word. Gated on the output count alone, not on anything
/// about the word: a `drop` overload has no outputs and an `extern:` is
/// rejected above one, so neither reaches this.
///
/// P7.S3m (R1): a *quotation*'s own declared effect needs the same bundle --
/// `lower_indirect_call` asks `bundle_of` for the tuple, and with nothing
/// interned it produced no value at all, so every output past the first was
/// never pushed. So the walk also descends into each declared type looking for
/// a `Type::Quotation` of two or more outputs: a word's inputs as well as its
/// outputs, a struct field and an array element (the two materialization
/// boundaries a quotation is legal at), and `w.poly`, which is where a
/// polymorphic word's shape lives -- `w.effect` is empty for one, so a walk
/// over `w.effect` alone misses every poly quotation parameter.
///
/// Purely additive: a bundle is keyed by its exact type list, so a quotation
/// tuple coinciding with a word's re-interns to the same `StructId`, and the
/// word-output tuples are still collected first, leaving the registry of a
/// program with no multi-output quotation byte-identical.
fn intern_output_bundles(module: &mut Module) {
    let mut tuples: Vec<Vec<Type>> = module
        .words
        .iter()
        .filter(|w| w.effect.outputs.len() >= 2)
        .map(|w| w.effect.outputs.iter().map(|s| s.ty).collect())
        .collect();
    for w in &module.words {
        for slot in w.effect.inputs.iter().chain(&w.effect.outputs) {
            collect_quotation_bundles(slot.ty, &mut tuples);
        }
        // A poly signature can carry a ground quotation in exactly one shape:
        // a top-level `Concrete`. `audit_poly_input_quotation` rejects one in a
        // poly array element, reference referent, cell payload, generic
        // argument and quotation-effect row, and a variable-bearing
        // `PolyType::Quotation` has no ground output tuple to key a bundle by.
        // A *fully* concrete composite (`[ [ i64 -- i64 i64 ] 2 ]`) folds to
        // `Concrete` at parse time, so it arrives here rather than as a
        // `PolyType::Array`.
        for pt in w
            .poly
            .iter()
            .flat_map(|sig| sig.inputs.iter().chain(&sig.outputs))
        {
            if let PolyType::Concrete(ty) = pt {
                collect_quotation_bundles(*ty, &mut tuples);
            }
        }
    }
    // The two composite sites, taken off the registries rather than descended
    // into from a signature: every `Type::Struct`/`Type::Array` naming one is
    // an index into these, so sweeping them covers a quotation nested at any
    // depth without a containment walk of its own.
    for (_, fty) in module.structs.iter().flat_map(|s| &s.fields) {
        collect_quotation_bundles(*fty, &mut tuples);
    }
    for a in &module.arrays {
        collect_quotation_bundles(a.element, &mut tuples);
    }
    for outputs in tuples {
        intern_bundle_struct(&mut module.structs, &outputs);
    }
}

/// P7.S3m (R2): the multi-output quotation effects `ty` itself is or nests. A
/// quotation's own rows are descended (a quotation-typed struct field may take
/// another quotation); every other composite that can legally hold a quotation
/// is a registry the caller sweeps directly.
fn collect_quotation_bundles(ty: Type, found: &mut Vec<Vec<Type>>) {
    // `Type::InlineQuotation` (a `~[ ... ]` parameter) is deliberately excluded:
    // a `~` is always spliced at its call site, never reaches
    // `lower_indirect_call`, and so needs no bundle -- unlike every other site
    // in this file (`is_quotation_type`), which treats both variants alike.
    let Type::Quotation(eff) = ty else {
        return;
    };
    if eff.outputs.len() >= 2 {
        found.push(eff.outputs.clone());
    }
    for nested in eff.inputs.iter().chain(&eff.outputs) {
        collect_quotation_bundles(*nested, found);
    }
}

/// Check a single word definition against an external env, seeding the env with
/// the word's own signature so self-recursion type-checks. `enums` is the
/// registry the elimination checks (arm coverage, scrutinee type,
/// variant-name collision) consult. Also returns this body's recorded overload-dispatch
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
    slices: &mut Vec<SliceDecl>,
    structs: &[StructDecl],
    poly_env: &PolyEnv,
    combinators: &CombinatorEnv,
) -> Result<ResolvedCalls, String> {
    let (_sites, insts, overloads, fields, variant_fields) = check_def_collecting_drop_sites(
        word,
        enums,
        env,
        arrays,
        cells,
        refs,
        slices,
        structs,
        poly_env,
        combinators,
    )?;
    Ok((insts, overloads, fields, variant_fields))
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
    slices: &mut Vec<SliceDecl>,
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
    // P7.S3o: scratch splice records (REPL path, never lowered).
    let mut splice_recs: HashMap<(u32, Span), CallInst> = HashMap::new();
    // P7.S3o Phase 3: scratch per-splice trait-member calls (REPL path).
    let mut splice_trait_recs: HashMap<(u32, Span), String> = HashMap::new();
    // Item 3: this body's resolved-overload call sites, relayed to the
    // caller so lowering can dispatch through them instead of
    // `empty_builtin_overloads()`.
    let mut overloads: HashMap<Span, String> = HashMap::new();
    // R2 (P7 slice 1): this body's receiver-directed field projections,
    // relayed to the caller so the session lowers them like a native build.
    let mut fields: HashMap<Span, (StructId, usize)> = HashMap::new();
    // R6 (Phase 6 slice 3): this body's receiver-directed variant-field
    // projections, relayed to the caller so the session lowers them like a
    // native build.
    let mut variant_fields: HashMap<Span, (EnumId, usize, usize)> = HashMap::new();
    // R3 (Slice 6c): the session's retained combinators thread through so a
    // defined word's body can call one and have it inlined, exactly as native
    // inlines one drawn from `module.words`. The build path and unit tests
    // pass the empty map, keeping the concrete path byte-identical.
    // Phase 6 slice 3 (R3): the eliminator registry is derived from the
    // session's own enums, so a session-defined word eliminates a retained
    // enum exactly as a native one does.
    let eliminators = eliminator_registry(enums);
    let mut scratch_monos: Vec<(String, crate::ast::Subst)> = Vec::new();
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut insts,
        builtin_overloads: &mut overloads,
        resolved_fields: &mut fields,
        resolved_variant_fields: &mut variant_fields,
        combinators,
        eliminators: &eliminators,
        // P7.S3e (R8): a session declares no `trait:`, so no `Bound::User`
        // reaches a REPL-checked body or line -- the same bypass
        // `structs`/`enums` already follow here.
        trait_resolve: TraitResolveCtx::scratch(),
        splice_records: &mut splice_recs,
        impl_monos: &mut scratch_monos,
        splice_trait_calls: &mut splice_trait_recs,
        combinator_sig: None,
        combinator_subst: None,
        combinator_name: None,
    };
    // R8 (slice 8b): a REPL-defined word body has no `ModuleInfo` view, so the
    // `drop` import-visibility gate never fires on the session path.
    // A REPL session declares no `static:` storage (P7 slice 2 is a build-path
    // feature), so the static table is empty here.
    // P7 slice 3a: the REPL never declares its own generic `type:` (D2), so
    // no session poly word's signature can carry a `PolyType::Generic`; `None`
    // here is correct, not a gap.
    check_word(
        word,
        enums,
        &env,
        arrays,
        cells,
        refs,
        slices,
        structs,
        &[],
        None,
        &mut sites,
        &mut poly,
        None,
    )?;
    Ok((sites, insts, overloads, fields, variant_fields))
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
    slices: &mut Vec<SliceDecl>,
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
    // P7.S3o: scratch splice records (REPL path, never lowered).
    let mut splice_recs: HashMap<(u32, Span), CallInst> = HashMap::new();
    // P7.S3o Phase 3: scratch per-splice trait-member calls (REPL path).
    let mut splice_trait_recs: HashMap<(u32, Span), String> = HashMap::new();
    // Item 3: this line's resolved-overload call sites, relayed to the
    // caller so lowering can dispatch through them instead of
    // `empty_builtin_overloads()`.
    let mut overloads: HashMap<Span, String> = HashMap::new();
    // R2 (P7 slice 1): this line's receiver-directed field projections,
    // relayed to the caller so the session lowers them like a native build.
    let mut fields: HashMap<Span, (StructId, usize)> = HashMap::new();
    // R6 (Phase 6 slice 3): this line's receiver-directed variant-field
    // projections, relayed to the caller so the session lowers them like a
    // native build.
    let mut variant_fields: HashMap<Span, (EnumId, usize, usize)> = HashMap::new();
    // R3 (Slice 6c): the session's retained combinators thread through so a
    // bare line can call one and have it inlined, exactly as native inlines one
    // drawn from `module.words`. The build path and unit tests pass empty.
    let eliminators = eliminator_registry(enums);
    let mut scratch_monos: Vec<(String, crate::ast::Subst)> = Vec::new();
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut insts,
        builtin_overloads: &mut overloads,
        resolved_fields: &mut fields,
        resolved_variant_fields: &mut variant_fields,
        combinators,
        eliminators: &eliminators,
        // P7.S3e (R8): a session declares no `trait:`, so no `Bound::User`
        // reaches a REPL-checked body or line -- the same bypass
        // `structs`/`enums` already follow here.
        trait_resolve: TraitResolveCtx::scratch(),
        splice_records: &mut splice_recs,
        impl_monos: &mut scratch_monos,
        splice_trait_calls: &mut splice_trait_recs,
        combinator_sig: None,
        combinator_subst: None,
        combinator_name: None,
    };
    let final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, slices, &mut prov, &mut scope, false,
        &mut poly,
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
        fields,
        variant_fields,
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

/// A parameter or binding name equal to a registered variant name is a sharp
/// error (X12): a variant name in a `|` binding reads as the value it
/// constructs everywhere else in a body, so binding one shadows it.
fn reject_variant_local(ctx: &Ctx, name: &str, kind: &str) -> Result<(), String> {
    if !is_registered_variant(name, ctx.enums()) {
        return Ok(());
    }
    Err(match ctx {
        Ctx::Word { mangled, .. } => format!(
            "error: {kind} `{name}` in {word_name} collides with the variant name `{name}`",
            word_name = crate::resolve::render_word(mangled)
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
        Ctx::Word { mangled, .. } => format!(
            "error: local `{name}` in {word_name} collides with the callable name `{name}` (line {})\n  a local cannot shadow a builtin, word, poly word, or combinator name",
            span.line
        , word_name = crate::resolve::render_word(mangled)),
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
        Ctx::Word { mangled, .. } => format!(
            "error: duplicate local `{name}` in {word_name} (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        , word_name = crate::resolve::render_word(mangled)),
        Ctx::Line { .. } => format!(
            "error: duplicate local `{name}` (line {})\n  `{name}` is bound twice; the second binding shadows the first and silently drops it",
            span.line
        ),
    })
}

/// The output-count / output-type mismatch check for a word body (M6, X8):
/// `final_stack` must match the declared outputs.
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
    let name = crate::resolve::render_word(&word.name);
    if final_stack.iter().any(|s| s.quot.is_some()) {
        return Err(format!(
            "error: {name} (line {}) leaves a quotation on the stack; a quotation cannot be a declared output",
            line
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
            "error: stack effect mismatch in {name} (line {})\n  body leaves {} values, but ( … ) declares {} outputs\n  note: declared {}",
            line, final_stack.len(), declared.len(), effect_str(&word.effect),
        ));
    }
    for (found, want) in final_stack.iter().zip(declared) {
        match match_slot(*found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            SlotMatch::NeedsSizeConversion => {
                return Err(format!(
                    "error: type mismatch in {name} (line {})\n  body leaves a computed `i64` where the declaration requires `{}`: convert it explicitly with `>{}` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
                    line, want, want, effect_str(&word.effect),
                ));
            }
            SlotMatch::NeedsStrToCstrConversion => {
                return Err(format!(
                    "error: type mismatch in {name} (line {})\n  body leaves `str` where the declaration requires `cstr`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)\n  note: declared {}",
                    line, effect_str(&word.effect),
                ));
            }
            SlotMatch::Mismatch => {
                return Err(format!(
                    "error: type mismatch in {name} (line {})\n  body leaves `{}` where the declaration requires `{}`\n  note: declared {}",
                    line, found.ty, want, effect_str(&word.effect),
                ));
            }
        }
    }
    Ok(())
}

/// A word's location, for locating a whole-word diagnostic like X1.
pub(crate) fn word_span(word: &WordDef) -> Span {
    word.span
}

/// P7.S4 (R1/R8): render an `ImplTarget` for diagnostics, using the impl's
/// own variable name tables so `'T`/`'N` spell as the user wrote them.
/// Shared by `declarations.rs`'s duplicate-target error and `poly.rs`'s
/// ambiguity error -- elevated here, their lowest common ancestor.
pub(super) fn impl_target_str(target: &ImplTarget) -> String {
    let sig = PolySig {
        row_in: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        row_out: None,
        bounds: Vec::new(),
        ty_var_names: target.ty_var_names.clone(),
        len_var_names: target.len_var_names.clone(),
        row_var_names: Vec::new(),
    };
    poly::poly_type_str(&target.pattern, &sig)
}

fn unknown_word_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { mangled, .. } => format!(
            "error: unknown word `{}` in {} (line {})",
            name,
            crate::resolve::render_word(mangled),
            span.line
        ),
        Ctx::Line { .. } => format!("error: unknown word `{name}`"),
    }
}

/// P7.S3t (R3): an explicit type-argument list (`f[Point]`) on a call that is
/// not a polymorphic-word call. Every route other than the polymorphic one
/// would have to drop the list, and a dropped instantiation links the wrong
/// specialization rather than reporting anything, so each rejects instead.
fn no_type_arguments_error(span: Span, name: &str) -> String {
    format!(
        "error: `{}` (line {}) takes no type arguments; only a call to a polymorphic word may be explicitly instantiated",
        crate::resolve::demangle_call(name),
        span.line
    )
}

/// P7.S3t (R1/R3): an explicit type-argument list on a call written inside a
/// polymorphic word's own body. That path checks symbolically and has no
/// substitution to seed, so the list is rejected rather than dropped; naming
/// the enclosing word matters because the remedy is at the *caller* of it.
fn type_arguments_in_poly_body_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let enclosing = match ctx {
        Ctx::Word { mangled, .. } => format!(" in {}", crate::resolve::render_word(mangled)),
        Ctx::Line { .. } => String::new(),
    };
    format!(
        "error: `{}`{enclosing} (line {}) cannot be explicitly instantiated inside a polymorphic word's own body\n  note: instantiate the enclosing word at its own call site instead; forwarding a type argument through a polymorphic body is not supported",
        crate::resolve::demangle_call(name),
        span.line
    )
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
        Ctx::Word { mangled, .. } => format!(
            "error: no overload of `{name}` in {wname} (line {}) accepts these operands{listed}",
            span.line,
            wname = crate::resolve::render_word(mangled)
        ),
        Ctx::Line { .. } => {
            format!("error: no overload of `{name}` accepts these operands{listed}")
        }
    }
}

fn underflow_error(ctx: &Ctx, span: Span, op: &str, needs: usize, holds: usize) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: stack effect mismatch in {} (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, op, needs, holds, effect_str(effect)),
        Ctx::Line { .. } => format!("error: stack underflow: needs {needs} values, but the stack holds {holds}"),
    }
}

/// R7: `str` -> `cstr` is an explicit word, never an implicit conversion; a
/// `str` where a `cstr` is wanted names the fix rather than a plain
/// mismatch, mirroring `size_conversion_needed_error`'s shape.
fn str_needs_cstr_conversion_error(ctx: &Ctx, span: Span, op: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: type mismatch in {} (line {})\n  `{}` wants `cstr`, found `str`: convert it explicitly with `cstr` first (there is no implicit `str` -> `cstr` conversion)\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, op, effect_str(effect)),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` wants `cstr`, found `str`: convert it explicitly with `cstr` first"
        ),
    }
}

fn type_mismatch_error(ctx: &Ctx, span: Span, op: &str, expected: Type, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: type mismatch in {} (line {})\n  `{}` expected `{}`, found `{}`\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, op, expected, found, effect_str(effect)),
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
        Ctx::Word {
            mangled, effect, ..
        } => {
            format!(
            "error: cannot `{}` a value of type `{}` in {} (line {})\n  {}\n  note: declared {}",
            op, found, crate::resolve::render_word(mangled), span.line, why, effect_str(effect))
        }
        Ctx::Line { .. } => format!("error: cannot `{op}` a value of type `{found}`: {why}"),
    }
}

/// R3 (D2): a linear local mentioned again after its value was moved out, the
/// diagnostic naming the earlier move site.
fn use_after_move_error(ctx: &Ctx, span: Span, local: &str, ty: Type, site: Span) -> String {
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: use after move in {} (line {})\n  local `{}` of type `{}` was moved at line {}, col {}; `{}` is linear, so it is used exactly once\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, local, ty, site.line, site.col, ty, effect_str(effect)),
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: linear value `{}` is never consumed in {} (line {})\n  `{}` has type `{}`, which is linear: drop it or return it (nothing is dropped for you)\n  note: declared {}",
            local, crate::resolve::render_word(mangled), line, local, ty, effect_str(effect)),
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: linear value `{}` is not consumed on every path in {} (line {})\n  `{}` has type `{}`, which is linear: it is consumed on one `if` arm but not the other, so drop it (or return it) on every path\n  note: declared {}",
            local, crate::resolve::render_word(mangled), line, local, ty, effect_str(effect)),
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: linear value `{}` {} in {} (line {})\n  `{}` has type `{}`, which is linear, and its scope ends at the `{}` on line {}, col {}: consume it before then (nothing is dropped for you)\n  note: declared {}",
            local, cause, crate::resolve::render_word(mangled), span.line, local, ty, token, span.line, span.col, effect_str(effect)),
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
        "error: linear value left on the stack in {} (line {})\n  body leaves a `{}` beyond the {} declared output(s): a linear value must be consumed exactly once, so `drop` it or return it\n  note: declared {}",
        crate::resolve::render_word(&word.name),
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
    let place = crate::resolve::demangle_word(place);
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: a reference to a local cannot cross a loop in {} (line {})\n  a reference derived from `{place}`, a local of this frame, crosses the self-tail-call back-edge to `{callee}`: that local's storage does not survive to the next iteration\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, effect_str(effect)),
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
            let deriv = prov.deriv(id);
            if let Some(place) = &deriv.owned_root {
                // R3: a static's data-segment storage survives every loop
                // iteration, unlike a local's slot (rebound at the loop
                // header); only a genuine local's owned_root crosses the
                // back-edge unsafely.
                if deriv.static_root {
                    continue;
                }
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
#[allow(clippy::too_many_arguments)]
fn back_edge_declared_shape(
    word: &WordDef,
    subst: Option<&Subst>,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<BackEdgeShape, String> {
    let (inputs, outputs): (Vec<Type>, Vec<Type>) = match word.poly.as_ref() {
        Some(sig) => {
            let subst = subst.expect("a poly combinator's marker carries its resolved θ");
            let mut inputs = Vec::with_capacity(sig.inputs.len());
            for p in &sig.inputs {
                inputs.push(apply_subst(
                    sig, p, subst, name, span, ctx, arrays, cells, refs,
                )?);
            }
            let mut outputs = Vec::with_capacity(sig.outputs.len());
            for p in &sig.outputs {
                outputs.push(apply_subst(
                    sig, p, subst, name, span, ctx, arrays, cells, refs,
                )?);
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

/// Phase 6 slice 1 (R2): resolve a parsed annotation's slots to concrete
/// types. A literal is checked against its own body, and a body supplies
/// neither an instantiation for a type variable nor a fixed point for a row
/// that changes shape, so both are located errors here rather than silently
/// admitted spellings.
///
/// Review fix (F2/F3, round 1): a *passthrough* row (`..a -- ..a`) is rejected
/// too, not only a shape-changing one. `AnnotEffect` carries no row field --
/// nothing downstream (`check_literal_against_annotation`,
/// `reconcile_annotation_with_parameter`) ever compares a row against a
/// consuming parameter's row, so admitting one here would silently drop it
/// after this one check: a standalone body could smuggle anything through the
/// unchecked row-typed prefix, and a literal filling a parameter that
/// declares *no* row at all would pass R4's equality vacuously (R5 requires
/// strict equality, which a decorative, uncompared row cannot deliver). This
/// narrows R2 from the spec's original passthrough-is-self-checking claim to
/// unconditional row rejection in this slice; see the amended R2 in
/// `docs/phase6-slice1-spec.md`.
fn resolve_annotation(ctx: &Ctx, annot: &QuotAnnot) -> Result<AnnotEffect, String> {
    if annot.row_in != annot.row_out {
        return Err(shape_changing_row_unbound_error(ctx, annot));
    }
    if annot.row_in.is_some() {
        return Err(row_annotation_unsupported_error(ctx, annot));
    }
    Ok(AnnotEffect {
        inputs: resolve_annot_slots(ctx, annot, &annot.inputs)?,
        outputs: resolve_annot_slots(ctx, annot, &annot.outputs)?,
        span: annot.span,
        variant_tag: annot.variant_tag.clone(),
    })
}

fn resolve_annot_slots(
    ctx: &Ctx,
    annot: &QuotAnnot,
    slots: &[PolyType],
) -> Result<Vec<Type>, String> {
    slots
        .iter()
        .map(|slot| match slot {
            PolyType::Concrete(ty) => Ok(*ty),
            PolyType::Var(v) => Err(unbound_effect_variable_error(ctx, annot, *v)),
            other => unreachable!("the annotation reader mints only `Concrete`/`Var`: {other:?}"),
        })
        .collect()
}

/// R2: a type variable in an annotation. Nothing a freestanding literal can
/// see supplies its instantiation, so it is rejected rather than given an
/// invented meaning.
fn unbound_effect_variable_error(ctx: &Ctx, annot: &QuotAnnot, var: u32) -> String {
    let name = &annot.ty_var_names[var as usize];
    format!(
        "error: effect variable `{name}` in a quotation annotation is unbound{} (line {})\n  an annotation is checked against the literal's own body, which supplies no instantiation for a type variable: write the concrete type",
        in_word(ctx),
        annot.span.line,
    )
}

/// R2: a row that differs between the two sides of an annotation. A
/// shape-changing effect has no fixed point to check a body against (nothing
/// supplies the difference between the two rows).
fn shape_changing_row_unbound_error(ctx: &Ctx, annot: &QuotAnnot) -> String {
    let row = |id: Option<u32>| id.map_or("", |i| annot.row_var_names[i as usize].as_str());
    // Review fix (minor, round 1): either side may be unnamed (`( ..a -- )`),
    // which left a stray space against the surrounding backtick; `trim()`
    // keeps the pair readable without hand-casing which side is empty.
    let spelled = format!("{} -- {}", row(annot.row_in), row(annot.row_out));
    format!(
        "error: shape-changing row `{}` in a quotation annotation is unbound{} (line {})\n  a standalone shape-changing row has no fixed point to check against: only a passthrough row or a concrete effect can be checked against a literal's own body",
        spelled.trim(),
        in_word(ctx),
        annot.span.line,
    )
}

/// Review fix (F2/F3, round 1): a passthrough row (`..a -- ..a`), unlike a
/// shape-changing one, does have a fixed point -- but `AnnotEffect` has no row
/// field to hold it in, so accepting it here would check nothing against it
/// ever again. Rejected outright rather than silently accepted-but-ignored.
fn row_annotation_unsupported_error(ctx: &Ctx, annot: &QuotAnnot) -> String {
    let row = |id: Option<u32>| id.map_or("", |i| annot.row_var_names[i as usize].as_str());
    format!(
        "error: row `{}` in a quotation annotation is not supported{} (line {})\n  a row is not tracked past this check in this slice: write a fully concrete effect (name every input/output type, no `..` row)",
        row(annot.row_in),
        in_word(ctx),
        annot.span.line,
    )
}

/// The `Type::Quotation`/`Type::InlineQuotation` a literal's own flavour
/// renders an effect as, for the two annotation diagnostics below.
fn annotated_effect_type(is_inline: bool, inputs: Vec<Type>, outputs: Vec<Type>) -> Type {
    if is_inline {
        crate::ast::inline_quotation_type(inputs, outputs)
    } else {
        crate::ast::quotation_type(inputs, outputs)
    }
}

/// R3: an annotated literal's body disagrees with its own annotation. R11's
/// shape (declared effect, actual body effect) minus the consuming word: this
/// fires wherever the literal is written, whether or not it ever fills a
/// parameter.
fn annotation_body_mismatch_error(ctx: &Ctx, span: Span, declared: Type, actual: Type) -> String {
    format!(
        "error: this quotation is annotated `{declared}` but its body has effect `{actual}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R4: an annotated literal disagrees with the (already grounded) quotation
/// parameter it fills. Names both effects, so a substitution that grounded the
/// parameter somewhere else in the call is legible at the literal.
fn annotation_parameter_mismatch_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    declared: Type,
    annotated: Type,
) -> String {
    // Phase 6 slice 3 review fix (finding 1): `word` here may be an
    // eliminator's call name, mangled mid-string (`Shape__m0?`) rather than
    // with a trailing group -- `demangle_word` cannot see through that, only
    // `demangle_call` can (see its own doc comment).
    let word = crate::resolve::demangle_call(word);
    format!(
        "error: the quotation passed to `{word}` is annotated `{annotated}` but `{word}` declares it `{declared}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R1/R3: run an annotated literal's body against its own annotation, the same
/// directional check `check_literal_against_declared_effect` runs against a
/// declared parameter -- seed a fresh sub-stack with the annotation's inputs,
/// run the body, require the exit row to equal the annotation's outputs. Runs
/// where the literal is interned, so it is independent of whether the literal
/// ever fills a parameter.
///
/// The consuming site re-runs this body against the caller's own stack, so
/// this probe restores the move states it touched. The R12 capture guards are
/// deliberately not repeated here: they are argument-site rules (whose
/// premise, that the callee may call the literal any number of times, this
/// site cannot judge), and a branch arm is legitimately exempt from them.
#[allow(clippy::too_many_arguments)]
fn check_literal_against_annotation(
    annot: &AnnotEffect,
    body: &[Term],
    is_inline: bool,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    let moves_before = scope.moves.states.clone();
    let depth = scope.depth();
    let fresh: Vec<Slot> = annot.inputs.iter().map(|t| Slot::computed(*t)).collect();
    let result = check_terms_relaxed(
        body,
        fresh,
        ctx,
        env,
        arrays,
        cells,
        refs,
        slices,
        prov,
        scope,
        false,
        poly,
        &HashSet::new(),
        true,
    )?;
    scope.moves.states = moves_before;
    leave_block(
        ctx,
        scope,
        depth,
        BlockEnd::Arm {
            token: "quotation",
            span: annot.span,
        },
    )?;
    let matches_out = result.len() == annot.outputs.len()
        && result.iter().zip(&annot.outputs).all(|(f, w)| {
            matches!(
                match_slot(*f, *w),
                SlotMatch::Exact | SlotMatch::LiteralSizeType
            )
        });
    if matches_out {
        return Ok(());
    }
    let declared = annotated_effect_type(is_inline, annot.inputs.clone(), annot.outputs.clone());
    let actual = annotated_effect_type(
        is_inline,
        annot.inputs.clone(),
        result.iter().map(|s| s.ty).collect(),
    );
    Err(annotation_body_mismatch_error(
        ctx, annot.span, declared, actual,
    ))
}

/// The shorter of two slot vectors matches the other's tail. Stacks grow
/// rightwards, so the tail is the end both sides agree on when one of them
/// extends past a row boundary the other doesn't name.
fn tails_agree(a: &[Type], b: &[Type]) -> bool {
    let n = a.len().min(b.len());
    a[a.len() - n..] == b[b.len() - n..]
}

/// R4/R5: reconcile an annotated literal against the quotation parameter it
/// fills. By this point `eff` is already grounded -- `PolyCtx`'s substitution
/// has replaced each declared type variable with its concrete ground -- so the
/// two effects are compared slot for slot under strict structural equality (no
/// narrowing, no compatible-but-not-identical acceptance).
///
/// This is the one comparison R3 (body vs annotation) and R11 (body vs
/// parameter) cannot make: a polymorphic body absorbs the annotation's claim
/// and the parameter's ground alike without contradiction, so only holding the
/// two declarations against each other sees the conflict.
///
/// A `shape_changing` parameter (`~[ ..i -- ..o ]`, `..i != ..o`) declares only
/// the fixed slots sitting above the row; a literal filling it may legitimately
/// reach further into the row, or leave more behind than the declaration names
/// -- that is the shape change. Only the overlapping tails are determined, so
/// only those are compared: the declared inputs sit directly under the
/// literal's consumption point and the declared outputs directly under its
/// exit, which is fixed point enough to catch a flat contradiction.
fn reconcile_annotation_with_parameter(
    annot: &AnnotEffect,
    eff: &QuotEffect,
    is_inline: bool,
    shape_changing: bool,
    ctx: &Ctx,
    word: &str,
) -> Result<(), String> {
    let agrees = match shape_changing {
        true => {
            tails_agree(&annot.inputs, &eff.inputs) && tails_agree(&annot.outputs, &eff.outputs)
        }
        false => annot.inputs == eff.inputs && annot.outputs == eff.outputs,
    };
    if agrees {
        return Ok(());
    }
    let declared = annotated_effect_type(is_inline, eff.inputs.clone(), eff.outputs.clone());
    let annotated = annotated_effect_type(is_inline, annot.inputs.clone(), annot.outputs.clone());
    Err(annotation_parameter_mismatch_error(
        ctx, annot.span, word, declared, annotated,
    ))
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
/// The four boundary properties `check_literal_against_declared_effect` reads,
/// bundled (Phase 6 slice 3 review fix, cycle 3): four consecutive bare `bool`s
/// at a call site say nothing about which is which, and this function already
/// takes more arguments than a reader can hold in their head.
#[derive(Clone, Copy)]
struct LiteralBoundary {
    /// Slice 10c (R-P2-3/R-P2-4): true for a declared quotation whose input
    /// and output rows differ (`..i -- ..o`, `..i != ..o`). There the exit row
    /// has no fixed point to check against -- the whole point of the shape
    /// change -- so only the declared fixed trailing outputs are checked and
    /// the literal's actual exit types are handed back unjudged.
    shape_changing: bool,
    /// Slice 10c: the literal fills a parameter slot the callee `call`s in
    /// *tail* position -- a branch arm of `if`/`unless`, as opposed to `times`'
    /// body. Such a literal runs at most once per entry.
    is_arm: bool,
    /// Whether the *call site* is itself in tail position, which an `is_arm`
    /// literal inherits.
    caller_tail: bool,
    /// Review fix (Phase 6 slice 3, finding 1): an arm-flavoured caller falls
    /// into one of two shapes. A combinator's argument pre-check (`if`,
    /// `check_poly_combinator_args`) is thrown away -- the splice that follows
    /// re-checks whichever arm actually runs, for real, so this probe's own
    /// move-state consumption must leave no trace (`finalize = false`). The
    /// eliminator (`check_eliminator_call`) never splices: this call *is* the
    /// only accounting the checker ever does for that arm, so its consumption
    /// must survive (`finalize = true`), and the caller reconciles every arm's
    /// surviving state itself (`Moves::join`, generalized to N arms) rather
    /// than losing it to a restore that has nothing later to answer to.
    finalize: bool,
    /// P7.S3h: the boundary declares `owning [ ... ]`. The literal takes
    /// ownership of what it captures and disposes it by running, so the R12
    /// ban below on consuming an enclosing linear local is lifted -- that
    /// consumption *is* the ownership transfer, and it must survive into the
    /// caller's move state so the frame no longer counts the value as its own.
    /// The mirror obligation (every linear capture must actually be consumed)
    /// is checked by the boundary itself, once the body walk has answered it.
    owning: bool,
}

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
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
    granted: &HashSet<String>,
    boundary: LiteralBoundary,
    // Review fix (Phase 6 slice 3, cycle 2): the caller's own slots for the
    // declared *inputs*, when the boundary hands the literal a value it
    // already holds rather than a freshly computed one. `None` (every caller
    // but the eliminator) seeds each declared input as `Slot::computed`, as
    // before. The eliminator's arm receives the caller's actual scrutinee --
    // a `&!Shape` is rooted at a caller place, and an arm handed a
    // provenance-free `&!Shape.Circle` instead borrows nothing, so a
    // reference projected out of it escaped the call unrooted and a second,
    // independent `&!` to the same place was accepted alongside it.
    input_slots: Option<&[Slot]>,
) -> Result<Vec<Slot>, String> {
    let LiteralBoundary {
        shape_changing,
        is_arm,
        caller_tail,
        finalize,
        owning,
    } = boundary;
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
    // Phase 6 slice 1 (R4/R5): an annotated literal must also agree with the
    // parameter it fills.
    if let Some(annot) = prov.quotations[id.0].annot.clone() {
        reconcile_annotation_with_parameter(&annot, eff, is_inline, shape_changing, ctx, word)?;
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
    match input_slots {
        Some(given) => fresh.extend(
            given
                .iter()
                .zip(&eff.inputs)
                .map(|(s, t)| Slot { ty: *t, ..*s }),
        ),
        None => fresh.extend(eff.inputs.iter().map(|t| Slot::computed(*t))),
    }
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
        slices,
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
    if is_inline && is_arm && !finalize {
        // The probe must also leave no trace: two sibling arms are
        // alternatives, each starting from the same move-state, and the splice
        // re-checks whichever one runs. Without the restore the second arm
        // sees the first arm's consumption and reports use-after-move.
        scope.moves.states = moves_before.clone();
    } else if !(owning || is_inline && is_arm) {
        if let Some(local) =
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
        return Ok(result);
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
    Ok(result)
}

/// Phase 6 slice 3 (R4): check a call to a generated eliminator word
/// (`Shape?`). Deliberately *not* a permutation of `check_poly_combinator_args`
/// (decision 4): an eliminator's arms are routed to variants by their
/// annotation tag rather than by slot position, and nothing per-arm unifies
/// (each arm's input is a concrete `Type::Variant` read straight off the enum),
/// so there is no substitution to solve and no `Subst` to hand back. What it
/// does share it *calls*: `check_literal_against_declared_effect` for every arm
/// body and `combinator_branch_output_mismatch_error` for the cross-arm
/// disagreement, so an arm is held to exactly the rules every other quotation
/// literal in the language is held to.
///
/// Decision 6: the scrutinee may be owning (`Shape`) or a reference
/// (`&Shape`/`&!Shape`). The mode is a property of the *call* -- one scrutinee,
/// one mode -- and every arm's expected input is built in it, so an arm
/// annotation that spells the wrong mode is rejected by the shared
/// declared-vs-written comparison rather than by a mode-specific diagnostic.
#[allow(clippy::too_many_arguments)]
fn check_eliminator_call(
    gate_id: EnumId,
    name: &str,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
    granted: &HashSet<String>,
    tail: bool,
) -> Result<Vec<Slot>, String> {
    // R5 (Slice 3b phase 2): `gate_id` is the registry's entry for this call
    // name -- a base-family key, since two instantiations of one generic
    // enum share one registry entry (last write wins). It is used below only
    // to gate that this call is an eliminator of the right family and for
    // the arity of the underflow diagnostic (a variant count every
    // instantiation of the family shares); the operative `EnumId` this call
    // actually eliminates is read off the scrutinee's own type once found.
    let gate_decl = &ctx.enums()[gate_id.index()];
    let held = stack.len();
    // R4 step 1: arm collection is variable-arity, not a fixed
    // `1 + variant_count` pop. A fixed pop cannot tell "an arm is missing"
    // from "the stack is short below the scrutinee", so a missing arm would
    // always present as underflow and the exhaustiveness pass below could
    // never name it. Collection stops at the first operand that is not a
    // tagged quotation literal; that operand is the scrutinee slot.
    let mut arms: Vec<(QuotId, VariantTag)> = Vec::new();
    while let Some(top) = stack.last().copied() {
        let Some(QuotOperand::Literal(qid)) = resolve_quotation_operand(top) else {
            break;
        };
        let Some(tag) = prov.quotations[qid.0]
            .annot
            .as_ref()
            .and_then(|a| a.variant_tag.as_ref())
        else {
            break;
        };
        arms.push((qid, tag.clone()));
        stack.pop();
    }
    // The checker pushes operands in written source order, so popping off the
    // top yielded the arms reversed. Both passes below walk arms in *written*
    // order (decision 5), so the reversal is undone here, once.
    arms.reverse();

    // R4 step 2: the scrutinee, owning or referenced.
    let Some(scrutinee) = stack.last().copied() else {
        return Err(underflow_error(
            ctx,
            span,
            name,
            gate_decl.variants.len() + 1,
            held,
        ));
    };
    if resolve_quotation_operand(scrutinee).is_some() {
        // The operand that stopped collection is a quotation, so it was meant
        // as an arm: either an untagged literal or a forwarded abstract
        // quotation, which carries no annotation to tag at all.
        return Err(eliminator_untagged_arm_error(ctx, span, name));
    }
    let (referent, ref_mutable) = match ref_parts(scrutinee.ty, refs) {
        Some((referent, mutable)) => (referent, Some(mutable)),
        None => (scrutinee.ty, None),
    };
    // R5: the operative `EnumId` is the scrutinee's own -- not `gate_id` --
    // so two asymmetric instantiations of one generic enum eliminate
    // independently in the same word: the registry's one entry is only
    // consulted above to reach this call at all, never to decide which
    // instantiation it narrows to. A non-enum scrutinee, or one whose enum is
    // not a member of this call name's base family, is the same
    // `type_mismatch_error` as before. `gate_decl` names whichever
    // instantiation the registry happened to retain (last write wins), so
    // the "expected" side is rendered under the family's surface name
    // (`Result`), not one arbitrary instantiation's (`Result[bool i64]`).
    let expected_family = Type::Enum(gate_id, generic_surface_name(gate_decl.name_static));
    let Type::Enum(id, _) = referent else {
        return Err(type_mismatch_error(
            ctx,
            span,
            name,
            expected_family,
            scrutinee.ty,
        ));
    };
    if generic_surface_name(&ctx.enums()[id.index()].name) != generic_surface_name(&gate_decl.name)
    {
        return Err(type_mismatch_error(
            ctx,
            span,
            name,
            expected_family,
            scrutinee.ty,
        ));
    }
    let enum_decl = &ctx.enums()[id.index()];
    // An `EnumDecl`'s `name` is the per-module mangled spelling (`Shape__m0`)
    // -- unlike `name_static`, which every `Type::Enum` render already uses.
    // Stripping the `[...]` arguments alone would still leave `Shape__m0` in a
    // diagnostic. An instantiation's mangle lands *after* the arguments
    // (`Result[i64 bool]__m0`), so for those the strip alone already yields a
    // bare `Result` and the demangle is what covers the concrete case.
    let enum_name = crate::resolve::demangle_word(generic_surface_name(&enum_decl.name));

    // R4 step 3: exhaustiveness and duplication, in written source order and
    // before any arm body is checked, so a coverage fault is reported where it
    // is, not as an arity failure inside some sibling arm.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut variant_indices = Vec::with_capacity(arms.len());
    for (qid, tag) in &arms {
        let Some(vi) = enum_decl
            .variants
            .iter()
            .position(|v| generic_surface_name(&v.name) == tag.name)
        else {
            return Err(eliminator_unknown_variant_error(
                ctx,
                prov.quotations[qid.0].span,
                name,
                &tag.name,
                enum_name,
            ));
        };
        if !seen.insert(generic_surface_name(&enum_decl.variants[vi].name)) {
            return Err(eliminator_duplicate_arm_error(
                ctx,
                prov.quotations[qid.0].span,
                name,
                &tag.name,
                enum_name,
            ));
        }
        variant_indices.push(vi);
    }
    for variant in &enum_decl.variants {
        let variant_surface = generic_surface_name(&variant.name);
        if !seen.contains(variant_surface) {
            return Err(eliminator_non_exhaustive_error(
                ctx,
                span,
                name,
                variant_surface,
                enum_name,
            ));
        }
    }

    // R4 steps 4-5: each arm body against its own variant, in written order,
    // with the first arm setting the `..b` baseline every later one must agree
    // with. The row every arm shares is the caller region below the scrutinee.
    //
    // Review fix (findings 1/2): each arm is checked against its own clone of
    // `scope`, exactly as `check_branch_join`'s `then_scope`/`else_scope` are
    // -- unlike a spliced `if`, nothing later re-checks whichever arm actually
    // runs, so this pass is the *only* accounting the checker ever does for
    // it. `finalize = true` disables `check_literal_against_declared_effect`'s
    // probe-and-restore (that restore's premise, a later splice re-checking
    // the real body, does not hold here), so each clone ends with its own
    // arm's real consumed move-state; the loop below joins every arm's
    // (`Moves::join`, generalized here from two arms to N) into `scope` for
    // real, and merges every arm's real output `Slot`s (provenance included)
    // rather than re-deriving the shared exit row from the pre-call `row`.
    let base = stack.len() - 1;
    let row: Vec<Slot> = stack[..base].to_vec();
    let mut baseline: Option<Vec<Slot>> = None;
    let mut arm_moves: Vec<Moves> = Vec::with_capacity(arms.len());
    for ((qid, tag), vi) in arms.iter().zip(&variant_indices) {
        let owned = variant_type(ctx.enums(), id, *vi);
        // Decision 6: the call's one resolved mode, applied uniformly. An arm
        // whose annotation spells the other mode disagrees with this built
        // effect and is rejected by the shared literal check below.
        let narrowed = match ref_mutable {
            Some(mutable) => intern_ref_type(refs, owned, mutable),
            None => owned,
        };
        let declared = crate::ast::inline_quotation_type(vec![narrowed], vec![]);
        let Some(eff) = crate::ast::is_quotation_type(declared) else {
            unreachable!("`inline_quotation_type` builds a quotation type")
        };
        // Slice 3b (R3): the arm's annotation declares no input slot -- a bare
        // tag is not typeable before the scrutinee's enum is known -- so the
        // slot is built here, from this variant in the mode the *user* wrote.
        // The reconciliation inside `check_literal_against_declared_effect`
        // then holds it against `narrowed`, built above in the *call's* mode:
        // both go through the same `variant_type`/`intern_ref_type`, so they
        // are the identical interned `Type` when the modes agree and a plain
        // annotation/parameter mismatch when they disagree (decision 6).
        let written = match tag.mode {
            VariantTagMode::Owning => owned,
            VariantTagMode::Ref => intern_ref_type(refs, owned, false),
            VariantTagMode::RefMut => intern_ref_type(refs, owned, true),
        };
        prov.quotations[qid.0]
            .annot
            .as_mut()
            .expect("an arm was collected by its annotation's variant tag")
            .inputs
            .insert(0, written);
        // The arm receives the caller's *own* scrutinee, retyped to the
        // narrowed variant -- not a fresh provenance-free slot. A reference
        // scrutinee is rooted at a caller place, and an arm that borrowed
        // nothing let a reference projected inside it leave the call
        // unrooted: a second, independent `&!` to that place was then
        // accepted alongside it, and the place itself could be consumed
        // inside the arm while a borrow of it was live. The declared effect
        // stays exactly as built above, so the mode-mismatch comparison is
        // unaffected.
        let received = [Slot {
            ty: narrowed,
            ..scrutinee
        }];
        let literal_span = prov.quotations[qid.0].span;
        let mut arm_scope = scope.clone();
        let arm_result = check_literal_against_declared_effect(
            *qid,
            eff,
            true,
            &row,
            name,
            span,
            ctx,
            env,
            arrays,
            cells,
            refs,
            slices,
            prov,
            &mut arm_scope,
            poly,
            granted,
            LiteralBoundary {
                shape_changing: true,
                is_arm: true,
                caller_tail: tail,
                finalize: true,
                owning: false,
            },
            Some(&received),
        )?;
        // R4 step 5b: a `Type::Variant` may not leave the call. Slice 2 made
        // the type reachable only as an arm's input and the value inside that
        // arm; this phase is the first that can construct one from surface
        // syntax, so it is the first that could let one out. Only a
        // single-variant enum gets this far -- with two or more variants the
        // arms leave different variant types and the cross-arm agreement below
        // rejects the call first -- but letting it out is unsound, not merely
        // untidy: every type-directed predicate outside the eliminator is
        // written over `Type::Enum`, so `is_copy` reads a variant as trivially
        // `Copy` and `dup` on an escaped one double-drops a linear payload.
        for slot in &arm_result {
            let referent = ref_parts(slot.ty, refs).map(|(referent, _)| referent);
            if matches!(slot.ty, Type::Variant(..)) || matches!(referent, Some(Type::Variant(..))) {
                return Err(eliminator_variant_escape_error(
                    ctx,
                    literal_span,
                    name,
                    slot.ty,
                ));
            }
        }
        arm_moves.push(arm_scope.moves);
        match &baseline {
            None => baseline = Some(arm_result),
            Some(expected) => {
                if let Some((baseline_types, arm_types)) =
                    arm_exit_row_mismatch(expected, &arm_result)
                {
                    return Err(combinator_branch_output_mismatch_error(
                        ctx,
                        literal_span,
                        name,
                        &baseline_types,
                        &arm_types,
                    ));
                }
                // Finding 2: type agreement is not evidence the arms leave the
                // same borrow provenance -- reconcile each position the same
                // way `check_branch_join`'s merge does, rejecting a
                // disagreement rather than silently erasing it (which would
                // let an escaped `&!` alias a second, independently-taken
                // one).
                let mut merged = Vec::with_capacity(expected.len());
                for (a, b) in expected.iter().zip(&arm_result) {
                    merged.push(merge_arm_output_slot(ctx, literal_span, a, b, prov)?);
                }
                baseline = Some(merged);
            }
        }
    }

    // R4 step 6: the call leaves the baseline -- the merged exit region every
    // arm agreed on, provenance included (finding 2) -- and joins every arm's
    // real move-state into `scope` (finding 1). A zero-variant enum has no
    // arms and no constructible value (OQ4), so its call is unreachable:
    // `arm_moves` stays empty and `scope`/`row` are simply left untouched.
    if let Some(joined) = arm_moves.into_iter().reduce(Moves::join) {
        scope.moves = joined;
    }
    Ok(baseline.unwrap_or(row))
}

/// R4 step 5: whether the running baseline exit row and one later arm's
/// disagree on types, and if so the pair the diagnostic names -- the baseline
/// (the written-*first* arm's shape, decision 5) as `expected`, the arm being
/// checked as `found`, in that order.
///
/// Split out of `check_eliminator_call` as a pure function so a unit test pins
/// that pairing by structure: two `Type`s can `Display` identically, so a
/// rendered-message assertion alone cannot tell the two apart from a swap
/// between them.
fn arm_exit_row_mismatch(baseline: &[Slot], arm: &[Slot]) -> Option<(Vec<Type>, Vec<Type>)> {
    let expected: Vec<Type> = baseline.iter().map(|s| s.ty).collect();
    let found: Vec<Type> = arm.iter().map(|s| s.ty).collect();
    let agrees = found.len() == expected.len()
        && found.iter().zip(&expected).all(|(f, w)| {
            matches!(
                match_slot(Slot::computed(*f), *w),
                SlotMatch::Exact | SlotMatch::LiteralSizeType
            )
        });
    match agrees {
        true => None,
        false => Some((expected, found)),
    }
}

/// R4 step 6 (review fix, finding 2): reconcile one exit-row position across
/// two arms that already agree on its *type* -- the borrow-suspension
/// bookkeeping still has to agree too, the same real content
/// `check_branch_join`'s merge (`check/terms.rs`) checks for an `if`. One arm
/// leaving a live borrow the other doesn't (or of a different place) is
/// rejected here rather than silently erased to a provenance-free slot.
fn merge_arm_output_slot(
    ctx: &Ctx,
    span: Span,
    a: &Slot,
    b: &Slot,
    prov: &mut Provenance,
) -> Result<Slot, String> {
    let deriv = match (a.deriv, b.deriv) {
        (None, None) => None,
        (Some(x), Some(y)) if prov.deriv(x).suspension() == prov.deriv(y).suspension() => Some(x),
        _ => {
            return Err(borrow_join_disagreement_error(
                ctx,
                span,
                a.deriv.map(|did| prov.deriv(did)),
                b.deriv.map(|did| prov.deriv(did)),
            ));
        }
    };
    let alias = match (a.alias, b.alias) {
        (None, None) => None,
        (Some(x), None) | (None, Some(x)) => Some(x),
        (Some(x), Some(y)) => Some(Alias {
            set: prov.alias_union(x.set, y.set),
            span: x.span,
        }),
    };
    Ok(Slot {
        alias,
        deriv,
        surviving: prov.union_surviving(a.surviving, b.surviving),
        ..Slot::computed(a.ty)
    })
}

/// R4 step 3: an arm annotated with a variant the eliminated enum does not
/// declare. Names both -- a tag naming *another* enum's variant is the shape
/// this catches (an unknown bare name never parses as a tag at all).
fn eliminator_unknown_variant_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    variant: &str,
    enum_name: &str,
) -> String {
    // Finding 4 (Phase 2 review): `word` is a *call* name -- once `resolve`
    // learns the `?` suffix, it arrives mangled (`Shape__m0?`), which
    // `demangle_word` alone cannot see through (it only strips a *trailing*
    // `__mN`, and the suffix sits after it here).
    let word = crate::resolve::demangle_call(word);
    format!(
        "error: unknown variant `{variant}` of enum `{enum_name}` in a call to `{word}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R4 step 3: two arms handling the same variant. The arms are matched by tag,
/// not by position, so a duplicate leaves some other variant unhandled -- named
/// here rather than as the non-exhaustiveness it also is.
fn eliminator_duplicate_arm_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    variant: &str,
    enum_name: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    format!(
        "error: duplicate arm for variant `{variant}` of enum `{enum_name}` in a call to `{word}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R4 step 3: a variant with no arm. Reported by name, which is the whole point
/// of the variable-arity arm collection: a fixed-arity pop would have failed as
/// underflow before this pass could run.
fn eliminator_non_exhaustive_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    variant: &str,
    enum_name: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    format!(
        "error: non-exhaustive call to `{word}`: missing variant `{variant}` of enum `{enum_name}`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R4 step 5b: an arm leaving its variant (or a reference to it) on the exit
/// row. The remedy is phrased in terms of what the arm can leave instead,
/// since the type it is holding cannot be written down: `W.One` is not a
/// spellable type name, which is what already stops it crossing a word
/// boundary.
fn eliminator_variant_escape_error(ctx: &Ctx, span: Span, word: &str, found: Type) -> String {
    let word = crate::resolve::demangle_call(word);
    format!(
        "error: an arm of `{word}` leaves `{found}` on the stack{} (line {})\n  a variant-typed value is reachable only inside the arm that bound it; consume it there, or leave its fields instead",
        in_word(ctx),
        span.line,
    )
}

/// R4 step 1: a quotation operand standing where the scrutinee should be. It
/// was meant as an arm, but carries no variant tag -- either an unannotated (or
/// plainly annotated) literal, or a forwarded abstract quotation, which has no
/// annotation to carry one. Both are rejected identically: an eliminator arm
/// must be a literal spelling the variant it handles.
fn eliminator_untagged_arm_error(ctx: &Ctx, span: Span, word: &str) -> String {
    let word = crate::resolve::demangle_call(word);
    format!(
        "error: an arm of `{word}` requires a variant tag: annotate the quotation with the variant it handles, as in `~[ ( Circle ) ... ]`{} (line {})\n  a forwarded quotation carries no annotation, so it cannot stand in for an arm",
        in_word(ctx),
        span.line,
    )
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
    let word = crate::resolve::render_word(word);
    format!(
        "error: {word} expects a quotation `{want}` here, found `{found}`{} (line {})",
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
    let word = crate::resolve::render_word(word);
    format!(
        "error: the quotation passed to {word} was declared `{declared}` but its body has effect `{actual}`{} (line {})",
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
    // Phase 6 slice 3 review fix (finding 1): same mid-string mangling as
    // `annotation_parameter_mismatch_error` above -- an eliminator arm's
    // literal-flavour check reaches this with the mangled call name.
    let word = crate::resolve::render_call(word);
    format!(
        "error: this argument is an ordinary `[ ... ]` quotation but {word} declares parameter `{param}` as inline `~[ ... ]`; write it `~[ ... ]`{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// Slice 12 (R-C2, E3b): a `~[ ... ]` literal at an ordinary `Type::Quotation`
/// boundary. The mirror of the error above, but phrased over the expectation
/// rather than a parameter declaration: unlike E3a, this fires at all three
/// boundaries, so `word` is as often the returning word or the store operator
/// (`!`) as it is the parameter's word.
fn inline_literal_at_ordinary_param_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    param: Type,
) -> String {
    let word = crate::resolve::render_word(word);
    format!(
        "error: this quotation is inline `~[ ... ]` but {word} expects `{param}`, an ordinary `[ ... ]`; write it `[ ... ]`{} (line {})",
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
    // Phase 6 slice 3 review fix (finding 4): shared with `check_eliminator_call`
    // (decision 4), whose `word` is a call name that may carry the `?` suffix
    // once mangled (`Shape__m0?`) -- `demangle_call` sees through that the
    // same way it already does for a destructure's `>`; an ordinary word name
    // (never carrying either suffix) demangles identically either way.
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
    combinator_branch_output_mismatch_rendered(ctx, span, word, &render(expected), &render(found))
}

/// P7 slice 3b (R3): the same message over already-rendered rows, so the
/// abstract eliminator join can raise the *same* diagnostic for a cross-arm
/// depth mismatch -- its rows are `PolyType`s, which no `Type` slice can hold.
fn combinator_branch_output_mismatch_rendered(
    ctx: &Ctx,
    span: Span,
    word: &str,
    expected: &str,
    found: &str,
) -> String {
    // Phase 6 slice 3 review fix (finding 4): shared with `check_eliminator_call`
    // (decision 4), whose `word` is a call name that may carry the `?` suffix
    // once mangled (`Shape__m0?`) -- `demangle_call` sees through that the
    // same way it already does for a destructure's `>`; an ordinary word name
    // (never carrying either suffix) demangles identically either way.
    let word = crate::resolve::render_call(word);
    format!(
        "error: the quotations passed to {word} leave different stack shapes: an earlier one leaves {expected}, this one leaves {found}{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R12: a quotation literal that consumes a linear enclosing local (D3 forbids
/// a linear capture). Names the local and the enclosing word.
fn quotation_captures_local_error(ctx: &Ctx, span: Span, word: &str, local: &str) -> String {
    let word = crate::resolve::render_word(word);
    format!(
        "error: the quotation passed to {word} consumes the enclosing local `{local}`, which is linear; a quotation may only read a `Copy` enclosing local by value (D3){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R12: a quotation literal that borrows an enclosing place and leaves the
/// reference on its row (D3 forbids capturing an enclosing borrow).
fn quotation_borrows_place_error(ctx: &Ctx, span: Span, word: &str, place: &str) -> String {
    let word = crate::resolve::render_word(word);
    format!(
        "error: the quotation passed to {word} borrows the enclosing place `{place}`; a quotation may not capture a borrow of an enclosing local (D3){} (line {})",
        in_word(ctx),
        span.line,
    )
}

fn rebound_local_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let scope_end = "a name may not be re-bound while it is in scope: the earlier binding would become unreachable, and a linear value in it could then never be consumed";
    match ctx {
        Ctx::Word { mangled, .. } => format!(
            "error: `{name}` is already bound in {word} (line {}, col {})\n  {scope_end}",
            span.line,
            span.col,
            word = crate::resolve::render_word(mangled)
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: type mismatch in {} (line {})\n  `{}` mixes `{}` with a computed `i64`: convert it explicitly with `>{}` first (a bare integer literal coerces automatically, a computed value does not)\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, op, target, target, effect_str(effect)),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` mixes `{target}` with a computed `i64`: convert it explicitly with `>{target}` first"
        ),
    }
}

/// Slice 10c: the two arms of a `branch` disagree. Named for the *arms*, not
/// for `if`: `branch` is the primitive and `if`/`unless` are ordinary
/// `core::bool` words over it, so by the time this fires the surface word
/// the user wrote has been spliced away and could equally have been `branch`
/// itself. (The span still points at the first arm, which is the user's own
/// literal either way -- see `check_branch_join`.)
fn branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: stack effect mismatch in {} (line {})\n  the two branch arms leave different stack depths (then: {}, else: {})\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, d_then, d_else, effect_str(effect)),
        Ctx::Line { .. } => format!(
            "error: the two branch arms leave different stack depths (then: {d_then}, else: {d_else})"
        ),
    }
}

fn branch_type_mismatch_error(ctx: &Ctx, span: Span, t_then: Type, t_else: Type) -> String {
    match ctx {
        Ctx::Word { mangled, effect, .. } => format!(
            "error: type mismatch in {} (line {})\n  the two branch arms leave different types (then: `{}`, else: `{}`)\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, t_then, t_else, effect_str(effect)),
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: type mismatch in {} (line {})\n  `{}` requires an array operand, found `{}`\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, op, found, effect_str(effect)),
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: linear array elements are not supported yet in {} (line {})\n  `{}` would replicate a `{}` across every slot, but `{}` is linear and has no `Copy` instance\n  note: declared {}",
            crate::resolve::render_word(mangled), span.line, site, elem, elem, effect_str(effect)),
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
        Ctx::Word { mangled, effect, .. } => format!(
            "error: cannot zero-initialize a `{}` in {} (line {})\n  `{}` transitively contains `{}` ({}), which is pointer-shaped and would zero to a null pointer\n  note: declared {}",
            outer, crate::resolve::render_word(mangled), span.line, outer, bad, where_, effect_str(effect)),
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

/// P7 slice 3c (R12): whether `ty` is a live borrow of somewhere else -- a
/// `&T`/`&!T` or a `Slice[T]`/`!Slice[T]` view -- and whether that borrow is
/// mutable. `ref_parts`' sibling for the sites that want the *borrow* nature
/// rather than the referent, which a view does not have (its element is not
/// what it points at: it points at a run of them).
fn borrow_mutability(ty: Type, refs: &[RefDecl]) -> Option<bool> {
    match ty {
        Type::Slice(_, mutable, _) => Some(mutable),
        _ => ref_parts(ty, refs).map(|(_, mutable)| mutable),
    }
}

/// ` in `word`` for a word body, empty for a bare REPL line: the suffix the
/// slice's own diagnostics use to place themselves the way every other
/// located error here does.
fn in_word(ctx: &Ctx) -> String {
    match ctx {
        Ctx::Word { mangled, .. } => format!(" in {}", crate::resolve::render_word(mangled)),
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
/// slot, an `extern` argument). P7.S3f (R4) retired the stale "a runtime
/// quotation value is slice 7" parenthetical outright, since 7a/7b gave it a
/// real runtime representation.
fn reject_quotation_argument(ctx: &Ctx, span: Span, word: &str) -> String {
    let word = crate::resolve::render_call(word);
    format!(
        "error: a quotation cannot be passed to {word}; only `call` accepts one{} (line {})",
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
    let place = crate::resolve::demangle_word(place);
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

/// D3 (slice 8b): a struct's destructure (`S>`) moves every field out of
/// `S`, bypassing whatever `drop` override `S` owns -- the value never
/// reaches a bare `drop` call site for D1's gate to see. `name` is checked
/// as-parsed (mangled in a >=2-module build, matching `struct_generated_sigs`'s
/// own keys), so this runs ahead of the ordinary `env` call path that would
/// otherwise apply the destructure's `Sig`.
fn check_destructure_drop_guard(name: &str, span: Span, ctx: &Ctx) -> Result<(), String> {
    let Some(struct_name) = name.strip_suffix('>') else {
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
    // (`examples/resources.sth`'s `Fd>` inside `: drop`). `resolve::mangle`
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
    Err(destructure_drop_overloaded_error(ctx, span, decl))
}

/// R11 (slice 8b, D3): the located diagnostic for destructuring a type whose
/// `drop` override would otherwise be skipped.
fn destructure_drop_overloaded_error(ctx: &Ctx, span: Span, decl: &StructDecl) -> String {
    let source = crate::resolve::demangle_word(&decl.name);
    let note = "\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out";
    match ctx {
        Ctx::Word { mangled, .. } => format!(
            "error: cannot destructure `{source}` in {name} (line {}): it defines `drop`, so moving its fields out would skip its destructor{note}",
            span.line
        , name = crate::resolve::render_word(mangled)),
        Ctx::Line { .. } => format!(
            "error: cannot destructure `{source}` (line {}): it defines `drop`, so moving its fields out would skip its destructor{note}",
            span.line
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn check_shuffle(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
    scope: &Scope,
    live: &Liveness,
    at: usize,
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
            // P7.S3v (R3/R4): `drop` on an owning closure is legal and runs
            // its per-construction-site disposer, which disposes the captures
            // without running the body. `call` remains the other consuming
            // use: it runs the body instead.
            // Review fix (P7 slice 1): dropping a place a live projection
            // still reaches would leave that reference aimed at storage
            // that no longer exists; the anonymous analogue of
            // `consume_of_borrowed_place_error`, keyed by region rather than
            // by a place name.
            if let Some(origin) = consumed_place_conflict(top, stack, scope, prov, live, at) {
                return Err(consuming_borrowed_value_error(ctx, span, "drop", origin));
            }
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

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module)
    }

    /// P7.S3m: the discovery walk alone, over a parsed module with no prelude,
    /// so the interned bundles are exactly the ones this program's declarations
    /// asked for -- a count over a checked module carries the core prelude's
    /// own bundles and could not tell an over-intern from a baseline.
    fn interned_bundles(src: &str) -> Vec<Vec<Type>> {
        let tokens = lex(src).unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        intern_output_bundles(&mut module);
        module
            .structs
            .iter()
            .filter(|s| s.is_bundle)
            .map(|s| s.fields.iter().map(|(_, ty)| *ty).collect())
            .collect()
    }

    /// P7.S3m (R1, site 1): a concrete word's quotation *parameter* -- the
    /// confirmed repro's shape. Its effect's outputs are the bundle
    /// `lower_indirect_call` reads back, and nothing used to intern it.
    #[test]
    fn quotation_param_two_outputs_interns_a_bundle() {
        let bundles = interned_bundles(": call_it ( [ i64 -- i64 i64 ] -- ) ;\n");
        assert_eq!(bundles, vec![vec![Type::I64, Type::I64]]);
    }

    /// P7.S3m (R1, site 4 / R3): the same parameter on a *polymorphic* word,
    /// whose declared shape lives in `w.poly` -- `w.effect` is empty here, so
    /// this passes only if the poly signature is walked. The word's own
    /// outputs are a single `'T`, so no word-level bundle can supply the tuple
    /// by coincidence.
    #[test]
    fn poly_signature_quotation_param_interns_a_bundle() {
        let bundles = interned_bundles(": call_it ( 'T: Copy [ i64 -- i64 i64 ] -- 'T ) ;\n");
        assert_eq!(bundles, vec![vec![Type::I64, Type::I64]]);
    }

    /// P7.S3m (R1, site 2): a struct field, one of the two materialization
    /// boundaries a quotation type is legal at.
    #[test]
    fn struct_field_quotation_two_outputs_interns_a_bundle() {
        let bundles = interned_bundles("type: Handler run [ i64 -- i64 i64 ] ;\n");
        assert_eq!(bundles, vec![vec![Type::I64, Type::I64]]);
    }

    /// P7.S3m (R1, site 3): an array element, the other materialization
    /// boundary. The parameter is a `&[..]`, so the quotation is two composites
    /// deep from the signature and only the array-registry sweep can reach it.
    #[test]
    fn array_element_quotation_two_outputs_interns_a_bundle() {
        let bundles = interned_bundles(": w ( &[ [ i64 -- i64 i64 ] 2 ] -- ) ;\n");
        assert_eq!(bundles, vec![vec![Type::I64, Type::I64]]);
    }

    /// P7.S3m (R2): a quotation's own rows are descended, so a two-output
    /// quotation reached only as *another* quotation's parameter is interned
    /// too. Live via a struct field, which (unlike a word input) is not gated by
    /// the nested-inside-an-effect rejection.
    #[test]
    fn quotation_nested_in_a_quotation_effect_interns_a_bundle() {
        let bundles = interned_bundles("type: H run [ [ i64 -- i64 i64 ] -- ] ;\n");
        assert_eq!(bundles, vec![vec![Type::I64, Type::I64]]);
    }

    /// P7.S3m: the `>= 2` filter holds at the widened sites too -- a
    /// single-output quotation keeps `lower_indirect_call`'s bundle-free path.
    #[test]
    fn quotation_param_single_output_interns_no_bundle() {
        assert!(interned_bundles(": call_it ( [ i64 -- i64 ] -- ) ;\n").is_empty());
    }

    /// P7.S3m: a `~[ ... ]` parameter is `Type::InlineQuotation`, not
    /// `Type::Quotation` -- `collect_quotation_bundles` skips it because a `~`
    /// is always spliced and never reaches `lower_indirect_call`. Pins the
    /// exclusion so widening the guard to `is_quotation_type` (as every other
    /// site in this file does) would be caught here.
    #[test]
    fn inline_quotation_param_two_outputs_interns_no_bundle() {
        assert!(interned_bundles(": call_it inline ( ~[ i64 -- i64 i64 ] -- ) ;\n").is_empty());
    }

    /// P7.S3m (R3): a quotation parameter whose own output row still mentions a
    /// type variable is never interned -- there is no ground tuple to key a
    /// bundle by, and picking the row's concrete slots out would key a bundle by
    /// a shape no call site can ever have. The fixture's row is deliberately
    /// *mixed*: an all-variable row would come out empty under that mistake
    /// too, and so could not tell it apart.
    ///
    /// P7.S3l (landed after this slice's own branch point) legalized `call` on
    /// a poly body's own abstract quotation parameter, so the mixed-row
    /// fixture below now builds rather than being rejected -- but its pushed
    /// outputs stay `PolyType`-abstract (`'T`), never a ground
    /// `Type::Quotation`, so it still keys no bundle. `call` on a fully
    /// variable-bearing (non-abstract-own-param) quotation is unaffected and
    /// still admits only a fully-ground quotation operand.
    #[test]
    fn variable_bearing_poly_quotation_interns_no_bundle() {
        assert!(
            interned_bundles(": call_it ( 'T: Copy [ i64 -- 'T i64 i64 ] -- ) drop ;\n").is_empty()
        );
        assert!(interned_bundles(
            ": call_it ( 'T: Copy [ 'T -- 'T 'T ] 'T -- 'T ) swap call drop drop ;\n"
        )
        .is_empty());
    }

    /// P7.S3e (R17): the obligation pre-pass hoists `check_poly_body` ahead of
    /// the main per-word loop, so every polymorphic body's diagnostic now
    /// precedes every monomorphic word's -- even when the monomorphic word is
    /// declared first. A deliberate ordering change, pinned so it cannot be
    /// reintroduced or reverted silently.
    #[test]
    fn a_poly_body_diagnostic_precedes_a_monomorphic_one_declared_before_it() {
        let err = check_src(
            ": bad-mono ( -- i64 ) ;\n\
             : bad-poly ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("bad-poly") && !err.contains("bad-mono"),
            "the poly body's error must be the first one reported: {err}"
        );
    }

    fn test_variant(fields: Vec<(String, Type)>) -> VariantDecl {
        VariantDecl {
            name: "Circle".to_string(),
            name_static: "Circle",
            display_static: "Shape.Circle",
            fields,
            span: Span::default(),
        }
    }

    #[test]
    fn variant_field_projection_value_mode_is_declared_order() {
        let variant = test_variant(vec![
            ("r".to_string(), Type::F64),
            ("n".to_string(), Type::I64),
        ]);
        let mut refs = Vec::new();
        let projected = variant_field_projection(&variant, None, &mut refs);
        assert_eq!(projected, vec![Type::F64, Type::I64]);
    }

    #[test]
    fn variant_field_projection_ref_mode_interns_each_field() {
        let variant = test_variant(vec![
            ("r".to_string(), Type::F64),
            ("n".to_string(), Type::I64),
        ]);
        let mut refs = Vec::new();
        let projected = variant_field_projection(&variant, Some(true), &mut refs);
        let expected_r = intern_ref_type(&mut refs, Type::F64, true);
        let expected_n = intern_ref_type(&mut refs, Type::I64, true);
        assert_eq!(projected, vec![expected_r, expected_n]);
    }

    #[test]
    fn variant_field_projection_zero_field_is_empty() {
        let variant = test_variant(vec![]);
        let mut refs = Vec::new();
        assert_eq!(variant_field_projection(&variant, None, &mut refs), vec![]);
        assert_eq!(
            variant_field_projection(&variant, Some(false), &mut refs),
            vec![]
        );
    }

    #[test]
    fn variant_field_projection_matches_pre_extraction_inline_loop() {
        // Mutation check (R5): reproduces the loop this helper replaced at
        // the R4 extraction, field by field, in both modes.
        let variant = test_variant(vec![
            ("r".to_string(), Type::F64),
            ("n".to_string(), Type::I64),
        ]);
        for ref_mutable in [None, Some(false), Some(true)] {
            let mut refs = Vec::new();
            let mut inline = Vec::new();
            for (_, ty) in &variant.fields {
                let field_ty = match ref_mutable {
                    Some(mutable) => intern_ref_type(&mut refs, *ty, mutable),
                    None => *ty,
                };
                inline.push(field_ty);
            }
            let mut refs_via_helper = Vec::new();
            let projected = variant_field_projection(&variant, ref_mutable, &mut refs_via_helper);
            assert_eq!(
                projected, inline,
                "mismatch for ref_mutable={ref_mutable:?}"
            );
        }
    }

    /// P7 slice 3c (R12): `borrow_mutability` answers for both borrow shapes.
    /// A slice is not a `Type::Ref`, so `ref_parts` alone reports it as no
    /// borrow at all -- which is what would let a mutable view be named twice
    /// (`terms.rs`' reborrow arm is the one caller that must not miss it).
    #[test]
    fn borrow_mutability_covers_a_slice_as_well_as_a_reference() {
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let shared_ref = intern_ref_type(&mut refs, Type::I64, false);
        let mutable_ref = intern_ref_type(&mut refs, Type::I64, true);
        let shared_view = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let mutable_view = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
        assert_eq!(borrow_mutability(shared_ref, &refs), Some(false));
        assert_eq!(borrow_mutability(mutable_ref, &refs), Some(true));
        assert_eq!(borrow_mutability(shared_view, &refs), Some(false));
        assert_eq!(borrow_mutability(mutable_view, &refs), Some(true));
        // An owned value is no borrow: naming one is a move or a read, never
        // a reborrow.
        assert_eq!(borrow_mutability(Type::I64, &refs), None);
    }

    /// P7 slice 3c (R6): a slice is in the *explicit* zero-unsafe set, since
    /// the wildcard below it treats what it does not name as zero-**safe** --
    /// an all-zero slice is a null element pointer with a zero length, not an
    /// empty view. Asserted through the located diagnostic the array
    /// constructor renders, both directly and one level down a struct field so
    /// the path is built.
    ///
    /// Second line of defence, deliberately: in `check_array_element_gate`
    /// today, R5's `contains_reference` rejects a slice-bearing element before
    /// this predicate is consulted at all. The arm is not redundant -- a
    /// future zero-safety caller that has no reference check to run in front of
    /// it would otherwise admit an all-zero view.
    #[test]
    fn find_zero_unsafe_element_names_slice() {
        let mut slices = Vec::new();
        let slice = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let structs = vec![StructDecl {
            name: "View".to_string(),
            name_static: "View",
            fields: vec![("s".to_string(), slice)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let view = Type::Struct(StructId::from_index(0), "View");
        let (bad, path) =
            find_zero_unsafe_element(slice, &structs, &[], &[]).expect("a slice is zero-unsafe");
        assert_eq!(bad, slice);
        assert!(path.is_empty());
        assert_eq!(
            array_constructor_zero_unsafe_element_error(
                &Ctx::Line {
                    structs: &structs,
                    enums: &[],
                },
                Span::default(),
                slice,
                bad,
                &path
            ),
            "error: cannot zero-initialize a `Slice[i64]`: it transitively contains `Slice[i64]` (directly), which is pointer-shaped and would zero to a null pointer"
        );
        let (bad, path) = find_zero_unsafe_element(view, &structs, &[], &[])
            .expect("a struct reaching a slice is zero-unsafe");
        assert_eq!(
            array_constructor_zero_unsafe_element_error(
                &Ctx::Line {
                    structs: &structs,
                    enums: &[],
                },
                Span::default(),
                view,
                bad,
                &path
            ),
            "error: cannot zero-initialize a `View`: it transitively contains `Slice[i64]` (via field `s`), which is pointer-shaped and would zero to a null pointer"
        );
    }

    #[test]
    fn zero_unsafe_positional_variant_field_is_named_by_index() {
        // OQ4/Phase 1: `find_zero_unsafe_element` builds a path out of variant
        // field names, so an attributeless field must appear as its position
        // rather than as the internal placeholder.
        let err = check_src(
            "type: Option 'T | None | Some 'T ;\n: main ( -- ) [ Option[str] ; 4 ] drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("variant `Some[str]` field 0"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains(crate::parser::POSITIONAL_FIELD_NAME),
            "the internal placeholder leaked into a diagnostic: {err}"
        );
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
        let src = ": g inline ( ..i Bool ~[ ..i -- ..o i64 ] -- ..o i64 ) | c | drop c call ;\n\
             : demo ( i64 -- i64 i64 ) True ~[ dup 1 eq ] g ;\n";
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
    /// as `examples/resources.sth`'s `Fd` (extracting the field via `Fd>`
    /// inside `drop`'s own body -- exempted, since a word literally named
    /// `drop` can only be the recognized override for the struct its declared
    /// effect names).
    const FD_DEF: &str = "type: Fd n i64 ;\n: drop ( Fd -- ) | h | h Fd> drop ;\n";
    /// `File`, whose only field is an `i64`, with a `drop` overload: the shape
    /// every R3/R4 test turns on, since the structural fold alone would call
    /// it `Copy`.
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File> . ;";
    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";
    fn infer_src(src: &str, entry: &[Type]) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        // P7 slice 3i (R2): `bool` is `core::bool`'s enum, which a real REPL
        // session seeds at startup (`Session::new`); this bare-line helper
        // mirrors that seed so a `bool`-producing comparison resolves.
        // P8 S2 (R3/R7): a real session no longer seeds them -- it imports
        // `core::prelude` like a file does -- but this helper resolves no
        // `import:`, so it keeps seeding the typed core in process so a bare
        // line's `lt`/`if` still names something.
        let bool_enums = crate::test_support::core_bool_enums();
        let core = crate::test_support::core_lib_words();
        let mut combinators = CombinatorEnv::default();
        for word in &core {
            combinators.insert(word.name.clone(), vec![combinator_of(word)]);
        }
        // `True`/`False`, which a comparison word's branch-and-construct body
        // calls; a session registers them from the injected `bool` enum.
        let env: HashMap<String, Vec<Overload>> = enum_generated_sigs(&bool_enums)
            .into_iter()
            .map(|(name, symbol, sig)| (name, vec![Overload { sig, symbol }]))
            .collect();
        infer_line(
            &terms,
            entry,
            &env,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &[],
            &bool_enums,
            &HashMap::new(),
            &combinators,
        )
        .map(|(stack, _insts, _overloads, _fields, _variant_fields)| stack)
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
    fn own_drop_body_may_not_destructure_a_different_drop_overloaded_struct() {
        // Bug 1 (round-1 review): the exemption for a word literally named
        // `drop` must be scoped to the *one* struct its own declared effect
        // names, not to the bare name `"drop"` -- `resolve::mangle` leaves
        // `drop` unmangled program-wide, so any struct's own `drop` override
        // could otherwise destructure an unrelated drop-overloaded struct and
        // skip *that* struct's destructor. `Box`'s own `drop` here
        // destructures `Fd`, not `Box`, so it must still be rejected.
        let err = check_src(&format!(
            "{FD_DEF}type: Box b i64 ;\n: drop ( Box -- ) | x | 7 Fd Fd> drop x Box> drop ;\n: main ( -- ) 1 Box drop ;\n"
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
            "{FD_DEF}type: File fd Fd ;\n: main ( -- ) 7 Fd File File> drop ;\n"
        ))
        .unwrap();
    }
    #[test]
    fn borrow_projection_on_drop_overloaded_type_is_not_guarded() {
        // D3: a field projection (`&!n`) never moves the aggregate out --
        // `Fd` stays live and reaches the ordinary `drop` call, so the
        // destructure-drop guard has nothing to say about it.
        check_src(&format!("{FD_DEF}: main ( -- ) 7 Fd &!n 8 ! drop ;\n")).unwrap();
    }
    /// Phase 6 slice 1: the poly quotation parameter the R4 tests turn on.
    /// `inline` is not decoration: `check_inline_quotation_requires_inline`
    /// rejects any word declaring a `~[ ... ]` poly parameter that is not
    /// itself inline, which would fail these tests on the wrong diagnostic.
    const ON_DEF: &str = ": on inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;\n";

    /// R1/R3: an annotation that describes the body is a no-op confirmation.
    /// (`dup 10 lt` leaves the original `i64` under the `bool`, so its effect
    /// is `i64 -- i64 bool`, not `i64 -- bool`.)
    #[test]
    fn check_annotation_matches_body_ok() {
        check_src(": w ( -- ) [ ( i64 -- i64 Bool ) dup 10 lt ] drop ;").unwrap();
    }

    /// R3: the disagreement is located at the annotation and fires with no
    /// consuming parameter anywhere in sight -- the literal is dropped.
    #[test]
    fn check_annotation_disagrees_with_body_is_error() {
        let err = check_src(": w ( -- ) [ ( i64 -- i64 ) dup 10 lt ] drop ;").unwrap_err();
        assert_eq!(
            err,
            "error: this quotation is annotated `[ i64 -- i64 ]` but its body has effect `[ i64 -- i64 Bool ]` in `w` (line 1)"
        );
    }

    /// R4/R5: the annotation's concrete claim against the parameter's already
    /// grounded effect. `true` grounds `'T` to `bool`, so `on`'s parameter is
    /// `~[ bool -- bool ]` while the annotation claims `i64`. The body
    /// `dup drop` is net identity, so it absorbs both claims: R3 (body vs
    /// annotation) and R11 (body vs parameter) each check out, and only R4's
    /// comparison of the two declarations sees the conflict. Asserted on the
    /// exact message, since `is_err()` alone cannot tell R4 firing from an
    /// unrelated rejection firing for the wrong reason.
    #[test]
    fn check_annotation_disagrees_with_poly_parameter_is_error() {
        let src = format!("{ON_DEF}: w ( -- ) True ~[ ( i64 -- i64 ) dup drop ] on drop ;\n");
        let err = check_src(&src).unwrap_err();
        assert_eq!(
            err,
            "error: the quotation passed to `on` is annotated `~[ i64 -- i64 ]` but `on` declares it `~[ Bool -- Bool ]` in `w` (line 2)"
        );
    }

    /// R5: the same call whose annotation matches the grounded parameter is an
    /// identity no-op, not a narrowing to argue about.
    #[test]
    fn check_annotation_agrees_with_poly_parameter_ok() {
        let src = format!("{ON_DEF}: w ( -- ) True ~[ ( Bool -- Bool ) dup drop ] on drop ;\n");
        check_src(&src).unwrap();
    }

    /// Review fix (F1, round 2): R4 was skipped outright for a shape-changing
    /// declared parameter, so an annotation flatly contradicting it went
    /// unchecked -- R3 seeds from the annotation, R11's exit check only holds
    /// the declared *suffix*, and the identity body absorbs both. The declared
    /// `i64` and the annotated `bool` are the same fixed slot, above the row,
    /// so nothing about the shape change makes them incomparable.
    #[test]
    fn check_annotation_disagrees_with_shape_changing_parameter_is_error() {
        let src = ": sc inline ( ..i i64 ~[ ..i i64 -- ..o i64 ] -- ..o i64 ) | f | f call ;\n\
             : w ( -- ) 5 ~[ ( Bool -- Bool ) dup drop ] sc . ;\n";
        let err = check_src(src).unwrap_err();
        assert_eq!(
            err,
            "error: the quotation passed to `sc` is annotated `~[ Bool -- Bool ]` but `sc` declares it `~[ i64 -- i64 ]` in `w` (line 2)"
        );
    }

    /// The other half of that fix: comparing the two effects outright (rather
    /// than their determined tails) would reject both of these correct calls.
    /// A shape-changing parameter names only the slots above the row, so a
    /// literal may reach past them into the row (`( i64 -- )` against a
    /// declared `~[ ..i -- ..o ]`) or leave more behind than the declaration
    /// names (`( -- i64 )`) -- that is the shape change, not a disagreement.
    #[test]
    fn check_annotation_extending_shape_changing_parameter_row_ok() {
        let sc = ": sc inline ( ..i ~[ ..i -- ..o ] -- ..o ) | f | f call ;\n";
        check_src(&format!("{sc}: w ( -- ) ~[ ( -- i64 ) 5 ] sc . ;\n")).unwrap();
        check_src(&format!("{sc}: w ( -- ) 7 ~[ ( i64 -- ) drop ] sc ;\n")).unwrap();
    }

    /// R2: nothing a freestanding literal can see instantiates `'T`. Asserted
    /// on the exact message: an ordinary row/arity mismatch from the standard
    /// body check would satisfy an `is_err()`-only assertion vacuously.
    #[test]
    fn check_standalone_type_variable_annotation_is_unbound_error() {
        let err = check_src(": w ( -- ) [ ( 'T -- 'T ) ] drop ;").unwrap_err();
        assert!(
            err.starts_with(
                "error: effect variable `'T` in a quotation annotation is unbound in `w` (line 1)"
            ),
            "unexpected message: {err}"
        );
    }

    /// R2: a row that differs between the two sides has no fixed point to
    /// check a body against.
    #[test]
    fn check_standalone_shape_changing_row_is_unbound_error() {
        let err = check_src(": w ( -- ) [ ( ..a -- ..b ) ] drop ;").unwrap_err();
        assert!(
            err.starts_with(
                "error: shape-changing row `..a -- ..b` in a quotation annotation is unbound in `w` (line 1)"
            ),
            "unexpected message: {err}"
        );
    }

    /// Review fix (minor, round 1): an unnamed side (`( ..a -- )`) used to
    /// leave a stray space against the closing backtick (`` `..a -- ` ``).
    #[test]
    fn check_standalone_shape_changing_row_with_unnamed_side_has_no_trailing_space() {
        let err = check_src(": w ( -- ) [ ( ..a -- ) ] drop ;").unwrap_err();
        assert!(
            err.starts_with(
                "error: shape-changing row `..a --` in a quotation annotation is unbound in `w` (line 1)"
            ),
            "unexpected message: {err}"
        );
    }

    /// Review fix (F2/F3, round 1): a passthrough row is rejected too, not
    /// only a shape-changing one -- `AnnotEffect` has no row field, so nothing
    /// downstream ever compares it against a consuming parameter's row.
    /// Asserted on the exact message: an ordinary row/arity mismatch from the
    /// standard body-check machinery could otherwise satisfy an
    /// `is_err()`-only assertion vacuously.
    #[test]
    fn check_standalone_passthrough_row_annotation_is_unsupported_error() {
        let err = check_src(": w ( -- ) [ ( ..a -- ..a ) ] drop ;").unwrap_err();
        assert!(
            err.starts_with(
                "error: row `..a` in a quotation annotation is not supported in `w` (line 1)"
            ),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_outputs_rejects_a_quotation_left_on_exit() {
        // R10: a matching output *count* means the ordinary path would emit a
        // type mismatch that leaks the `Cstr` placeholder; the dedicated
        // quotation-at-exit branch in `check_outputs` fires first and names the
        // word.
        let err = check_src(": f ( -- i64 ) [ add ] ;\n")
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
        let err = infer_src("1 [ add ]", &[])
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
        let src = ": oops ( i64 -- i64 )\n  | a | a a add add ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("oops"));
        assert!(err.contains("`add`"));
        assert!(err.contains("needs 2 values"));
        assert!(err.contains("holds 1"));
        assert!(err.contains("( i64 -- i64 )"));
    }
    #[test]
    fn check_branch_depth_mismatch_is_error() {
        // Slice 10c: the arms are quotation literals now, so the disagreement
        // is caught at the *argument* site (R-P2-3), comparing one arm's
        // actual exit shape against its sibling's, rather than at the join.
        let src = ": w ( Bool -- i64 ) ~[ 1 1 ] ~[ 1 ] if ;";
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
        let ty = check_src(": w ( u32 -- i64 ) | c | c [ 1 ] [ True ] branch ;").unwrap_err();
        assert!(
            ty.contains("the two branch arms leave different types (then: `i64`, else: `Bool`)"),
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
        check_src(": w ( Bool -- i64 ) ~[ 1 ] ~[ 2 ] if ;").unwrap();
    }
    #[test]
    fn check_branch_join_type_mismatch_is_error() {
        // `then` leaves an `i64`, `else` leaves a `bool`: same depth, different type.
        let src = ": w ( Bool -- i64 ) ~[ 1 ] ~[ True ] if ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("leave different stack shapes"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`Bool`"), "unexpected message: {err}");
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
        // first term, so an empty body (`terms.first()` is `None`)
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
        // `0 gt` yields a bool that `if` consumes; both arms leave an i64.
        check_src(": sign ( i64 -- i64 ) 0 gt ~[ 1 ] ~[ 0 ] if ;").unwrap();
    }
    #[test]
    fn check_if_condition_not_bool_is_error() {
        let src = ": w ( -- i64 ) 5 ~[ 1 ] ~[ 2 ] if ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("expected `Bool`"), "unexpected message: {err}");
        assert!(err.contains("found `i64`"), "unexpected message: {err}");
    }
    #[test]
    fn check_operand_type_mismatch_is_error() {
        let src = ": w ( -- i64 ) True 1 add ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`Bool`"), "unexpected message: {err}");
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
        // replicated it N times. `Boxed>` materializes a quotation field
        // destructure's output with its surviving set intact (the R19/R22
        // comment on the generic accessor path), so `b Boxed>` hands `fill` an
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
             b Boxed>\n\
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
        // X7/R13: `eq` on arrays reaches the operand guard naming the type.
        let err = check_src(": w ( -- Bool ) 0 4 fill 0 4 fill eq ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }
    #[test]
    fn check_arithmetic_on_array_is_error() {
        // X7/R13: `add` on arrays reaches the operand guard naming the type
        // (the diagnostic covers `eq` *and* arithmetic; both are exercised).
        let err = check_src(": w ( -- [i64 4] ) 0 4 fill 0 4 fill add ;").unwrap_err();
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
        let src = ": w ( Bool -- f64 ) ~[ 1.0 >f32 ] ~[ 2.0 ] if ;";
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
        check_src(": w ( Bool -- f64 ) ~[ 1.0 ] ~[ 2.0 ] if ;").unwrap();
    }
    #[test]
    fn infer_line_net_effect_expected() {
        assert_eq!(infer_src("2 3 add", &[]).unwrap(), vec![Type::I64]);
    }
    #[test]
    fn infer_line_carries_entry_depth() {
        // `2 add` from a carried `i64`: the literal plus the carried slot are
        // consumed by `add`, leaving one `i64`.
        assert_eq!(infer_src("2 add", &[Type::I64]).unwrap(), vec![Type::I64]);
    }
    #[test]
    fn infer_line_carries_slot_types_expected() {
        // A `Bool`-producing line leaves a `Bool` on the carried stack -- the
        // enum `core::bool` declares, which the helper seeds exactly as a
        // session does.
        //
        // Revised under P7.S3s: the six comparisons (`gt` included) now
        // dispatch a real `impl: Ord` trait member (`cmp`), which needs a
        // whole-program `impl:` registry `infer_line` has no parameter for
        // (mirroring the REPL's own loss of `'T: Copy Ord`, R8) -- `and` is a
        // plain `Bool`-typed operator, needing no trait resolution, so it
        // still exercises the same "a line's result type is carried"
        // property this test pins.
        let bool_ty = crate::ast::resolve_bool_type(&crate::test_support::core_bool_enums())
            .expect("`core::bool` declares `Bool`");
        assert_eq!(infer_src("True False and", &[]).unwrap(), vec![bool_ty]);
    }
    #[test]
    fn line_underflow_against_carried_stack_is_error() {
        let err = infer_src("add", &[Type::I64]).unwrap_err();
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
            "{SPY_DEF}: w ( Spy Bool -- )\n  | s c |\n  c ~[ s drop ] ~[ s drop ] if ;"
        ))
        .unwrap();
    }
    #[test]
    fn check_linear_local_moved_in_one_arm_then_used_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy Bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 1 . ] if\n  s drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_linear_local_moved_in_one_arm_and_dropped_nowhere_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy Bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 1 . ] if ;"
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
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 eq ~[ s drop 0 ] ~[ 9 Spy n 1 sub spin ] if ;"
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
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 eq ~[ s drop 0 ] ~[ s n 1 sub spin ] if ;"
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
                   n 0 gt ~[\n\
                   acc f call\n\
                   5 n 1 sub f loopy\n\
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
    /// P7 slice 2 (R3) review: a static's data-segment storage survives
    /// every loop iteration, unlike a local's slot (rebound at the loop
    /// header), so a fresh `&!COUNT` passed across a self-tail-call
    /// back-edge must be accepted, not rejected with a message that calls
    /// `COUNT` "a local of this frame" (false for a static root).
    #[test]
    fn static_ref_crosses_self_tail_call_back_edge_ok() {
        check_src(
            "static: COUNT i64 = 0 ;\n\
             : spin ( &!i64 i64 -- )\n  | c n |\n  c 1 +!\n  \
             n 0 gt ~[ &!COUNT n 1 sub spin ] ~[ ] if ;\n\
             : main ( -- ) &!COUNT 3 spin ;",
        )
        .expect("a static-rooted reference may cross the back-edge freely");
    }
    /// Slice 10a (R12): `while` -- the symmetric shape whose back-edge produced
    /// the carried input pre-rewrite and produces the ground declared output
    /// post-rewrite (they agree at 1<->1) -- must still type-check identically.
    #[test]
    fn while_self_tail_still_checks_after_back_edge_rewrite() {
        check_src(
            ": while inline ( 'a ~[ 'a -- 'a Bool ] -- 'a ) | p | p call ~[ p while ] ~[ ] if ;\n",
        )
        .expect("`while` still type-checks after the back-edge rewrite");
    }

    /// P7 slice 1 (R2): the side table lowering reads back. One entry per
    /// projection call site, each carrying the receiver `StructId` and the
    /// field index -- neither of which the projection's own name states.
    #[test]
    fn resolved_fields_records_one_entry_per_call_site() {
        let tokens = lex("type: A n i64 ;\n\
                          type: B tag i64 n u32 ;\n\
                          : main ( -- ) 1 A &n @ . drop 2 7 >u32 B &n @ . drop ;")
        .unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).expect("both projections resolve");
        let a = module
            .structs
            .iter()
            .position(|d| d.name == "A")
            .expect("`A` is registered");
        let b = module
            .structs
            .iter()
            .position(|d| d.name == "B")
            .expect("`B` is registered");
        let mut recorded: Vec<(usize, usize)> = module
            .resolved_fields
            .values()
            .map(|(id, fi)| (id.index(), *fi))
            .collect();
        recorded.sort_unstable();
        let mut want = vec![(a, 0), (b, 1)];
        want.sort_unstable();
        assert_eq!(
            recorded, want,
            "one entry per site, resolved against each site's own receiver"
        );
    }

    /// Phase 6 slice 3 (R4): the sources every eliminator test below is built
    /// from. Declaration order is `Circle`, `Rect`, which several of these
    /// deliberately disagree with.
    const SHAPE_DECL: &str = "type: Shape | Circle r i64 | Rect w i64 h i64 ;\n";
    const ABC_DECL: &str = "type: Abc | A a i64 | B b i64 | C c i64 ;\n";

    #[test]
    fn check_eliminator_call_accepts_an_exhaustive_owning_call() {
        // The control every rejection test below rests on: with both arms
        // present and correctly tagged, the call type-checks and leaves the
        // arms' shared exit shape. Without this, a checker that rejected
        // *every* eliminator call would pass all the error tests.
        check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .expect("an exhaustive owning-mode eliminator call type-checks");
    }

    #[test]
    fn check_eliminator_call_missing_arm_names_missing_variant() {
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("non-exhaustive call to `Shape?`")
                && err.contains("missing variant `Rect` of enum `Shape`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_duplicate_arm_is_error() {
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Circle ) Circle> ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("duplicate arm for variant `Circle` of enum `Shape`")
                && err.contains("`Shape?`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_unknown_variant_names_it_and_enum() {
        // A bare unknown name never parses as a tag at all (the parser only
        // reads a leading token that names a *declared* variant), so the shape
        // this catches is a tag naming another enum's variant.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             type: Other | Squircle s i64 ;\n\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Squircle ) Squircle> ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("unknown variant `Squircle` of enum `Shape`") && err.contains("`Shape?`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_arm_output_disagreement_is_error() {
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> lt ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("the quotations passed to `Shape?` leave different stack shapes")
                && err.contains("an earlier one leaves `i64`, this one leaves `Bool`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_written_order_sets_baseline() {
        // Decision 5: the written-*first* arm sets the `..b` baseline, not the
        // declaration-first one. `Shape` declares `Circle` first; this call
        // writes the `Rect` arm first, and the two arms leave genuinely
        // distinct concrete types, so the pairing passed to
        // `combinator_branch_output_mismatch_error` (`expected` = the
        // written-first arm's shape, `found` = the offending arm's) is what
        // discriminates the two orderings: iterating in declaration order
        // swaps them.
        //
        // The pairing itself (baseline as `expected`, offender as `found`) is
        // pinned by structure in `arm_exit_row_mismatch_pairs_baseline_first`;
        // here `Bool` and `i64` `Display` distinctly, so the rendered message
        // discriminates the ordering too.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- Bool ) ~[ ( Rect ) Rect> lt ] ~[ ( Circle ) Circle> ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("an earlier one leaves `Bool`, this one leaves `i64`"),
            "the written-first (`Rect`) arm must set the baseline: {err}"
        );
    }

    /// Decision 5's pairing, by structure rather than by rendered message: the
    /// running baseline (the written-first arm) is `expected`, the arm under
    /// check is `found`. A swap between them is invisible to a message
    /// assertion whenever two shapes `Display` identically.
    #[test]
    fn arm_exit_row_mismatch_pairs_baseline_first() {
        let baseline = [Slot::computed(Type::U32)];
        let agreeing = [Slot::computed(Type::U32)];
        let disagreeing = [Slot::computed(Type::I64)];
        assert_eq!(arm_exit_row_mismatch(&baseline, &agreeing), None);
        assert_eq!(
            arm_exit_row_mismatch(&baseline, &disagreeing),
            Some((vec![Type::U32], vec![Type::I64])),
            "the baseline's shape is `expected`, the arm's is `found`"
        );
        assert_eq!(
            arm_exit_row_mismatch(&baseline, &[]),
            Some((vec![Type::U32], vec![])),
            "a differing row *length* is a disagreement, not a prefix match"
        );
    }

    #[test]
    fn check_eliminator_call_pop_order_does_not_set_baseline() {
        // Three arms whose written order (B, C, A), declaration order
        // (A, B, C) and stack-pop order (A, C, B) are pairwise different. The
        // baseline must be the written-*first* arm (`B`, leaving `Bool`).
        // Using the collected arms in pop order -- the order they come off the
        // stack, without the reversal back to written order -- makes `A`'s
        // `i64` the baseline instead, which the written-vs-declaration test
        // above cannot catch (there, pop order and declaration order agree).
        let err = check_src(&format!(
            "{ABC_DECL}\
             : f ( Abc -- Bool ) ~[ ( B ) B> 0 lt ] ~[ ( C ) C> ] ~[ ( A ) A> ] Abc? ;\n\
             : main ( -- ) 3 A f . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("an earlier one leaves `Bool`, this one leaves `i64`"),
            "the written-first (`B`) arm must set the baseline: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_missing_arm_is_error_not_underflow() {
        // R4 step 1: arm collection is variable-arity. With `variant_count - 1`
        // correctly tagged arms above a good scrutinee, the exhaustiveness pass
        // runs and names the missing variant. A fixed-arity pop of
        // `1 + variant_count` would report underflow's generic count mismatch
        // instead, and could never reach the pass that names `C`.
        let err = check_src(&format!(
            "{ABC_DECL}\
             : f ( Abc -- i64 ) ~[ ( A ) A> ] ~[ ( B ) B> ] Abc? ;\n\
             : main ( -- ) 3 A f . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("missing variant `C` of enum `Abc`"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("needs") && !err.contains("the stack holds"),
            "a missing arm is not an underflow: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_forwarded_arm_is_error() {
        // R4 step 1: an arm must be a quotation *literal* carrying a tag. A
        // forwarded abstract quotation parameter carries no annotation, so it
        // can never be routed to a variant; it is rejected exactly as an
        // untagged literal is, rather than silently accepted and left to ICE at
        // lowering. `use` is never called, so the only operand reaching the
        // arm slot is the forwarded parameter itself.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : use inline ( Shape ~[ i64 -- i64 ] -- i64 ) ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("an arm of `Shape?` requires a variant tag"),
            "unexpected message: {err}"
        );
        // The same body with the forwarded quotation consumed before the call
        // (and both arms written out) is legal, so the rejection above is
        // about the forwarded operand, not about the rest of the word.
        check_src(&format!(
            "{SHAPE_DECL}\
             : use inline ( Shape ~[ i64 -- i64 ] -- i64 ) drop ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) ;\n"
        ))
        .expect("the same word with the forwarded quotation consumed first is legal");
    }

    #[test]
    fn check_eliminator_call_untagged_literal_arm_is_error() {
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : f ( Shape -- i64 ) ~[ 1 ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 Circle f . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("an arm of `Shape?` requires a variant tag"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_reference_scrutinee_types_arms_by_reference() {
        // Decision 6: a reference scrutinee gives every arm a reference to the
        // narrowed variant. Both arms read a field through that reference, and
        // the caller still owns its `Shape` afterwards -- nothing was consumed.
        check_src(&format!(
            "{SHAPE_DECL}\
             : first ( &Shape -- i64 ) ~[ ( &Circle ) &r @ ] ~[ ( &Rect ) &w @ ] Shape? ;\n\
             : main ( -- ) 3 Circle | s | &s first . s drop ;\n"
        ))
        .expect("a `&Shape` scrutinee types every arm as a reference to its variant");
        check_src(&format!(
            "{SHAPE_DECL}\
             : bump ( &!Shape -- ) ~[ ( &!Circle ) &!r 1 +! ] ~[ ( &!Rect ) &!w 1 +! ] Shape? ;\n\
             : main ( -- ) 3 Circle | s | &!s bump s drop ;\n"
        ))
        .expect("a `&!Shape` scrutinee types every arm as a mutable reference");
    }

    #[test]
    fn check_eliminator_call_mode_mismatch_is_error() {
        // Decision 6: the arm's annotation must spell the scrutinee's own mode.
        // Nothing coerces -- the expected effect is built in the call's
        // resolved mode and the disagreement is caught by the same
        // declared-vs-written comparison every other annotated literal faces.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : first ( &Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( &Rect ) &w @ ] Shape? ;\n\
             : main ( -- ) 3 Circle | s | &s first . s drop ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("annotated `~[ Shape.Circle -- ]`")
                && err.contains("`Shape?` declares it `~[ &Shape.Circle -- ]`"),
            "unexpected message: {err}"
        );
        // Mode is a property of the call, not of an individual arm: a `&` arm
        // among `&!` siblings is the same rejection.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : bump ( &!Shape -- ) ~[ ( &Circle ) &r @ drop ] ~[ ( &!Rect ) &!w 1 +! ] Shape? ;\n\
             : main ( -- ) 3 Circle | s | &!s bump s drop ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("annotated `~[ &Shape.Circle -- ]`")
                && err.contains("`Shape?` declares it `~[ &!Shape.Circle -- ]`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_arm_extra_declared_input_is_error() {
        // R3 synthesizes the receiver slot but must not replace an arm's own
        // declared inputs: `Rect i64` here declares a second input after the
        // tag, which the call's real effect (`Shape.Rect -- i64`, one input)
        // does not have.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect i64 -- i64 ) &w @ swap &h @ swap drop mul ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("annotated `~[ Shape.Rect i64 -- i64 ]`")
                && err.contains("`Shape?` declares it `~[ Shape.Rect -- ]`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_diagnostics_demangle_the_call_name_in_a_real_build() {
        // Phase 6 slice 3 review fix (finding 1): `check_src` skips
        // `resolve_modules`, so every name is left bare and the two tests
        // above never exercised the mangled call name `Shape__m0?` that a
        // real build produces (the native build path force-mangles even a
        // single module). `demangle_word` only strips a *trailing* `__mN`
        // group, blind to that mid-string one -- these two diagnostics must
        // go through `demangle_call` instead, as the eliminator resolution
        // test above (`eliminator_call_site_mangles_to_match_the_enum_based_key`)
        // already does for the call site itself.
        let src = format!(
            "{SHAPE_DECL}\
             : first ( &Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( &Rect ) &w @ ] Shape? ;\n\
             : main ( -- ) 3 Circle | s | &s first . s drop ;\n"
        );
        let tokens = crate::lexer::lex(&src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::resolve::resolve_modules(&mut module, true).unwrap();
        let err = check(&mut module).unwrap_err();
        assert!(
            !err.contains("__m") && err.contains("`Shape?` declares it"),
            "mangled name leaked into the annotation-mismatch diagnostic: {err}"
        );

        let src = format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) [ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        );
        let tokens = crate::lexer::lex(&src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::resolve::resolve_modules(&mut module, true).unwrap();
        let err = check(&mut module).unwrap_err();
        assert!(
            !err.contains("__m") && err.contains("but `Shape?` declares parameter"),
            "mangled name leaked into the bracket-flavour diagnostic: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_diagnostics_demangle_the_enum_name_in_a_real_build() {
        // The sibling above covers the *call* name; the enum name reaches the
        // same three diagnostics from `EnumDecl::name`, which `resolve`
        // mangles (unlike `name_static`, the spelling every `Type::Enum`
        // render uses). The `check_src` tests for these messages cannot see
        // it: they skip `resolve_modules`, so the name is already bare there
        // and they pass with the demangle removed.
        let src = format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        );
        let tokens = crate::lexer::lex(&src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::resolve::resolve_modules(&mut module, true).unwrap();
        let err = check(&mut module).unwrap_err();
        assert!(
            err.contains("missing variant `Rect` of enum `Shape`"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("__m"),
            "mangled enum name leaked into the non-exhaustiveness diagnostic: {err}"
        );
    }

    /// R4 step 5b: the ruling on whether a `Type::Variant` may leave the call.
    /// It may not -- and the enforcement has to be here, because the type is
    /// unspellable rather than banned: `( W -- W.One )` is `unknown type
    /// W.One`, so nothing stops the value sitting on the caller's own stack.
    #[test]
    fn check_eliminator_call_arm_leaving_its_variant_is_error() {
        let err = check_src(
            "type: W | One a i64 ;\n\
             : main ( -- ) 3 One ~[ ( One ) ] W? drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("an arm of `W?` leaves `W.One` on the stack"),
            "unexpected message: {err}"
        );
    }

    /// The reference-mode half: an arm that leaves the `&W.One` it was handed
    /// escapes the same value by another spelling.
    #[test]
    fn check_eliminator_call_arm_leaving_a_reference_to_its_variant_is_error() {
        let err = check_src(
            "type: W | One a i64 ;\n\
             : main ( -- ) 3 One | w | &w ~[ ( &One ) ] W? drop w drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("an arm of `W?` leaves `&W.One` on the stack"),
            "unexpected message: {err}"
        );
    }

    /// Why the rejection above is soundness, not tidiness: `is_copy` is
    /// written over `Type::Enum` and reads a `Type::Variant` as trivially
    /// `Copy`, so an escaped variant of a linear enum could be `dup`ed and its
    /// payload's `drop` run twice. The parent enum's own `dup` is rejected --
    /// asserted here so this stays a statement about the *variant* escaping
    /// rather than about `W` being linear at all.
    #[test]
    fn check_eliminator_call_escaped_variant_would_bypass_the_dup_linearity_gate() {
        const DECL: &str = "type: R n i64 ;\n\
             : drop ( R -- ) | h | h R> . ;\n\
             type: W | One a R ;\n";
        let escaped = check_src(&format!(
            "{DECL}\
             : main ( -- ) 1 R One ~[ ( One ) ] W? dup drop drop ;\n"
        ))
        .unwrap_err();
        assert!(
            escaped.contains("an arm of `W?` leaves `W.One` on the stack"),
            "unexpected message: {escaped}"
        );
        let parent = check_src(&format!(
            "{DECL}\
             : main ( -- ) 1 R One dup drop drop ;\n"
        ))
        .unwrap_err();
        assert!(
            parent.contains("cannot `dup` a value of type `W`"),
            "unexpected message: {parent}"
        );
    }

    #[test]
    fn check_eliminator_call_non_enum_scrutinee_is_a_type_mismatch() {
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : f ( i64 -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 f . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`Shape?` expected `Shape`, found `i64`"),
            "unexpected message: {err}"
        );
    }

    /// Phase 2 review, finding 2: R5 split the old single referent check into
    /// two -- non-enum (above) and wrong-family, since a generic instantiation
    /// no longer has the same `EnumId` as `gate_id` even when it is a member
    /// of the right family. The comparison is a diagnostic-quality guard, not
    /// a crash guard: deleting it still rejects this program, but as `unknown
    /// variant `Circle` of enum `Abc`` -- blaming the arm for naming a variant
    /// the *scrutinee's* enum lacks, rather than the scrutinee for not being a
    /// member of `Shape?`'s family at all.
    #[test]
    fn check_eliminator_call_wrong_enum_family_scrutinee_is_a_type_mismatch() {
        let err = check_src(&format!(
            "{SHAPE_DECL}{ABC_DECL}\
             : f ( Abc -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 A f . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`Shape?` expected `Shape`, found `Abc`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_arm_double_consume_is_use_after_move() {
        // Phase 2 review, finding 1: an arm that consumes an outer linear
        // local must be reconciled into the caller's move-state exactly as an
        // `if` arm's join already reconciles one (`MaybeMoved` where the
        // arms disagree) -- forgetting it let `f` be dropped once inside the
        // `Circle` arm and, silently, a second time after the call.
        let err = check_src(&format!(
            "{FILE_RESOURCE}\n{SHAPE_DECL}\
             : main ( -- ) 1 File | f | 3 Circle\n\
             \x20 ~[ ( Circle ) Circle> . f drop ] ~[ ( Rect ) Rect> . . ] Shape? f drop ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("use after move") && err.contains('f'),
            "unexpected message (expected a use-after-move on `f`): {err}"
        );
    }

    #[test]
    fn check_eliminator_call_escaped_borrow_conflicts_with_a_second_borrow() {
        // Phase 2 review, finding 2: each arm leaves a live `&!p` borrowing a
        // caller local the scrutinee never touches. The merge
        // (`merge_arm_output_slot`) must carry that borrow's provenance
        // through the call, the same way `check_branch_join`'s merge does for
        // an `if`, so the caller still knows `p` is exclusively borrowed by
        // the escaped reference and a second, independent `&!p` right after
        // the call is rejected rather than silently aliasing it.
        let err = check_src(&format!(
            "type: P x i64 ;\n\
             {SHAPE_DECL}\
             : touch ( &!P -- ) &!x 1 +! ;\n\
             : main ( -- ) 1 P | p | 3 Circle\n\
             \x20 ~[ ( Circle ) Circle> drop &!p ] ~[ ( Rect ) Rect> drop drop &!p ] Shape?\n\
             \x20 &!p touch touch p P> . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("conflicts with a live borrow of `p`"),
            "unexpected message (expected a live-borrow conflict on `p`): {err}"
        );
    }

    #[test]
    fn check_eliminator_call_arm_borrow_disagreement_is_error() {
        // Finding 2 (targeted at `merge_arm_output_slot` itself): two arms
        // leave a live borrow of *different* places at the same, agreeing
        // type (`&!P` either way) -- the type-only baseline comparison alone
        // cannot see this disagreement, only the deriv/suspension comparison
        // does, mirroring `check_branch_join`'s own
        // `borrow_join_disagreement_error`.
        let err = check_src(&format!(
            "type: P x i64 ;\n\
             {SHAPE_DECL}\
             : first ( Shape P P -- ) | q p |\n\
             \x20 ~[ ( Circle ) Circle> drop &!p ] ~[ ( Rect ) Rect> drop drop &!q ] Shape? drop ;\n\
             : main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("borrow state disagrees at the branch join"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_no_word_shadows_eliminator_rejects_a_colliding_word_name() {
        // Phase 2 review, smaller point 1: `check_term`'s eliminator
        // interception runs ahead of the ordinary env lookup, so a user word
        // literally named `Shape?` would be silently unreachable rather than
        // coexisting as an overload (there is no overload mechanism here to
        // fall back through, unlike a generated destructure). Rejected by
        // name instead.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : Shape? ( i64 -- i64 ) 1 add ;\n\
             : main ( -- ) 5 Shape? . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("has the same name as the generated eliminator for enum `Shape`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn eliminator_arm_tag_outside_a_call_is_error() {
        // Finding 3 (Phase 2 review): `check_literal_against_annotation`'s
        // blanket skip for *every* tagged literal is only sound for one that
        // is actually collected as an eliminator arm; a tagged literal that
        // never reaches an eliminator call was previously never checked at
        // all, magic (CLAUDE.md) rather than a located error.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : f ( -- ) ~[ ( Circle ) nonexistent_word_xyz ] drop ;\n\
             : main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains(
                "an eliminator-arm tag, but it is not consumed by a call to a generated eliminator"
            ),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn eliminator_arm_tag_separated_from_its_call_is_error() {
        // The written-adjacency rule stated: `4 drop` is stack-neutral, so
        // this call's arms are still adjacent on the *stack* and
        // `check_eliminator_call` would collect both. The literal-side check
        // is syntactic and cannot see that, so it rejects -- and says why,
        // rather than claiming the arm reaches no eliminator at all.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] 4 drop ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("arms are written together, immediately before the call"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn eliminator_arm_tag_immediately_preceding_its_call_is_not_flagged() {
        // The control for the rejection above: a correctly-formed eliminator
        // call must not trip the new outside-a-call check.
        check_src(&format!(
            "{SHAPE_DECL}\
             : area ( Shape -- i64 ) ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;\n\
             : main ( -- ) 3 Circle area . ;\n"
        ))
        .expect("a correctly-formed eliminator call is not flagged as an arm outside a call");
    }

    #[test]
    fn check_eliminator_call_reference_arm_keeps_the_scrutinee_borrow_rooted() {
        // Phase 2 review cycle 2: an arm's *input* is the caller's own
        // scrutinee slot, not a fresh one. Each arm here projects a reference
        // out of the `&!Shape` it was handed and leaves it live, so the
        // caller still knows `s` is exclusively borrowed and the `&!s` after
        // the call conflicts -- exactly what the spliced-`if` shape
        // (`&!p True ~[ &!x ] ~[ &!x ] if &!p`) already reports, which the
        // eliminator used to accept because it routed the scrutinee through
        // `eff.inputs` (always erased) rather than through the row.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : main ( -- ) 3 Circle | s | &!s\n\
             \x20 ~[ ( &!Circle ) &!r ] ~[ ( &!Rect ) &!w ] Shape? &!s drop drop s drop ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`&!s` conflicts with a live borrow of `s`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_arm_cannot_consume_the_borrowed_scrutinee_root() {
        // The same rooting, seen from the other side: inside an arm holding a
        // reference projected out of the scrutinee, the place that reference
        // is rooted at cannot be consumed. With a provenance-free arm input
        // the borrow pointed at nothing and this was accepted.
        let err = check_src(&format!(
            "{SHAPE_DECL}\
             : main ( -- ) 3 Circle | s | &!s\n\
             \x20 ~[ ( &!Circle ) &!r s drop ] ~[ ( &!Rect ) &!w ] Shape? drop ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("cannot name `s`") && err.contains("a mutable borrow of it is still live"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_eliminator_call_sibling_arms_may_each_consume_one_outer_local() {
        // The other half of finding 1's fix: arms are alternatives, so each
        // is checked against its own clone of the caller `Scope`. Sharing one
        // scope across the loop makes the second arm see the first arm's
        // consumption of `f` and wrongly report use-after-move -- the whole
        // reason `check_branch_join` clones `then_scope`/`else_scope` too.
        check_src(&format!(
            "{FILE_RESOURCE}\n{SHAPE_DECL}\
             : main ( -- ) 1 File | f | 3 Circle\n\
             \x20 ~[ ( Circle ) Circle> . f drop ] ~[ ( Rect ) Rect> . . f drop ] Shape? ;\n"
        ))
        .expect("each arm may consume the same outer local: only one arm runs");
    }
}
