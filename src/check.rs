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
mod declarations;
mod drop_graph;
mod engine;
mod poly;
mod terms;
mod word_entry;

use self::audits::*;
pub(crate) use self::audits::{
    audit_quotation_type_registries, audit_word_quotation_positions, drop_overload_struct_id,
    find_drop_overloads,
};
use self::builtins::*;
pub(crate) use self::builtins::{is_copy, is_linear, sig_of, Overload, Sig};
pub use self::declarations::check_structs;
use self::declarations::*;
pub(crate) use self::declarations::{
    check_exported_signatures, check_selective_imports, check_types, enum_generated_sigs,
    selective_not_exported_error, struct_generated_sigs, SelectiveName,
};
use self::drop_graph::*;
pub(crate) use self::drop_graph::{check_drop_overload_reachability, has_self_tail_call};
use self::engine::*;
use self::poly::*;
pub(crate) use self::poly::{check_poly_body, check_poly_combinator_repl, poly_type_str};
use self::terms::check_terms;
use self::word_entry::{check_reference_free_signature, check_word};

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

/// R18/R6a: every quotation-taking word registered under one name. A name
/// can carry more than one candidate exactly as an ordinary overloaded word
/// can (R1); resolving which one a call splices needs the live stack's
/// operand types, the same shape as `Overload`/poly-candidate resolution --
/// a single-value map here would silently shadow a second combinator
/// overload exactly as env's `Sig` did before B1.
pub(crate) type CombinatorEnv<'a> = HashMap<String, Vec<Combinator<'a>>>;
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

/// Slice 6a (R18): one monomorphic quotation-taking word available to inline.
/// Both fields are shared references into the module, so a `Combinator` is a
/// pair of pointers (`Copy`), which lets a call site copy it out of the
/// borrowed map and then reborrow `PolyCtx` mutably for the splice.
#[derive(Clone, Copy)]
pub(crate) struct Combinator<'a> {
    word: &'a WordDef,
    terms: &'a [Term],
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

/// One interned quotation literal: its body terms (spliced at `call`/`times`)
/// and the literal's span, for a located diagnostic.
#[derive(Debug, Clone)]
struct QuotBody {
    body: Vec<Term>,
    #[allow(dead_code)]
    span: Span,
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
    check_tail_call_cycles(words, &drop_overload_indices)?;
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
            "error: a quotation cannot be left on the stack at the end of a line: the session carries it into the next line, and only `call` and `times` accept a quotation (a runtime quotation value is slice 7)".to_string(),
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

/// D8's clause-vs-binding disambiguation is global (`is_variant_name` scans
/// every enum in scope), so a `|` followed by a registered variant always
/// opens the next clause and is never read as a binding (R8). Every clause
/// the parser produces leads with a registered name, so this note states the
/// rule that was applied wherever a clause's variant is rejected.
fn clause_variant_ambiguity_note(name: &str) -> String {
    format!(
        "\n  note: `| {name}` is read as a clause because `{name}` is a variant name; a binding may not lead with one"
    )
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

/// Both-operand type mismatch for a homogeneous operator (`+ - * = < >`):
/// mixed int/float, mixed integer widths/signs, mixed float widths, or a
/// `bool` operand, name both operand types (X1, X2).
fn operand_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same numeric type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires two operands of the same numeric type, found `{a}` and `{b}`"
        ),
    }
}

/// `/` applied to a non-float or mixed-float-type pair (X3): `/` is
/// float-only, integer division is unsupported.
fn div_requires_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `/` requires two operands of the same float type (integer division is unsupported), found `{}` and `{}`\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `/` requires two operands of the same float type (integer division is unsupported), found `{a}` and `{b}`"
        ),
    }
}

/// `mod` applied to a non-integer or mixed-integer-type pair (X4): `mod`
/// stays integer-only.
fn mod_requires_int_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `mod` requires two operands of the same integer type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `mod` requires two operands of the same integer type, found `{a}` and `{b}`"
        ),
    }
}

/// `max` applied to a float operand (X9): `max` is integer-only (D6);
/// naming `max-total` is the point of the message, not just the mismatch.
fn max_over_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `max` does not support float operands (found `{}` and `{}`); use `max-total` for a total-ordered float maximum\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `max` does not support float operands (found `{a}` and `{b}`); use `max-total` for a total-ordered float maximum"
        ),
    }
}

/// `max-total` applied to a non-float or mixed-float-type pair (X10):
/// `max-total` is float-only; naming `max` is the point of the message.
fn max_total_requires_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `max-total` requires two operands of the same float type, found `{}` and `{}`; use `max` for integers\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `max-total` requires two operands of the same float type, found `{a}` and `{b}`; use `max` for integers"
        ),
    }
}

/// `and`/`or`/`xor` applied to a non-integer/non-bool or mixed-type pair:
/// bitwise ops are homogeneous over the integer types and `bool`, same shape
/// as `mod_requires_int_error`.
fn bitwise_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same integer or bool type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires two operands of the same integer or bool type, found `{a}` and `{b}`"
        ),
    }
}

/// `not` applied to a non-integer, non-bool operand.
fn bitwise_not_requires_int_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `not` requires an integer or bool operand, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `not` requires an integer or bool operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a non-integer value operand.
fn shift_value_requires_int_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an integer value operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires an integer value operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a shift count that is not `i64`.
fn shift_count_requires_i64_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an `i64` shift count, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires an `i64` shift count, found `{found}`"
        ),
    }
}

/// A conversion word (`>iN`/`>uN`/`>f32`/`>f64`) applied to a non-numeric
/// (`bool`) source (X5).
fn conversion_source_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires a numeric source, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires a numeric source, found `{found}`")
        }
    }
}

/// `.` applied to a non-printable value. Every current primitive `Type` (the
/// integer tower, the float tower) is printable via a builtin row, and `bool`
/// is printable via the library overload injected by `bool_print_word_def`,
/// so this path has no reachable golden yet; it exists for the day a
/// non-printable scalar (e.g. a future `Ptr`) enters the type system.
fn print_requires_printable_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `.` requires a printable scalar, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `.` requires a printable scalar, found `{found}`")
        }
    }
}

/// `cstr` applied to something other than `str` (R7): the only legal source
/// for the discard-the-length conversion, so the error names it by name
/// rather than as a generic type mismatch.
fn cstr_conversion_source_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `cstr` converts a `str`, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `cstr` converts a `str`, found `{found}`")
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

/// R15 (D8): a linear value live across the self-tail-call back-edge, which the
/// loop lowering would carry into the next iteration with nobody responsible
/// for disposing it. Deferred to a later Phase 3 slice, as a located error
/// rather than silence. Copy loops are untouched.
fn linear_across_back_edge_error(ctx: &Ctx, span: Span, callee: &str, ty: Type) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: linear values across a loop are not supported yet in `{}` (line {})\n  a `{}` is live across the self-tail-call back-edge to `{}`: consume it before the recursive call\n  note: declared {}",
            name, span.line, ty, callee, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: linear values across a loop are not supported yet: a `{ty}` is live across the back-edge to `{callee}`"
        ),
    }
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

/// R15: reject a linear value that would survive the back-edge of a
/// self-tail-call, either stranded on the stack below the call's arguments or
/// held by a local that was never consumed. A value *moved into* the call's
/// arguments is forwarded, not live across the edge, so it stays legal.
fn check_linear_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    below_args: &[Slot],
    scope: &Scope,
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    if let Some(slot) = below_args
        .iter()
        .find(|s| is_linear(s.ty, ctx.structs(), ctx.enums(), arrays))
    {
        return Err(linear_across_back_edge_error(ctx, span, callee, slot.ty));
    }
    if let Some(local) = scope.moves.unconsumed().first() {
        let ty = scope
            .local_type(local)
            .expect("a tracked local is in scope");
        return Err(linear_across_back_edge_error(ctx, span, callee, ty));
    }
    Ok(())
}

/// R4: a binding naming something already in scope. For a linear value the
/// rejection is forced (the earlier binding would become unreachable, and its
/// value could then never be consumed), and applying it to Copy values too
/// keeps one rule and one message instead of two.
/// `call` reached without a statically-known quotation literal on top (D4):
/// the value there is not traceable to a single literal.
fn call_needs_quotation_error(ctx: &Ctx, span: Span) -> String {
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `call` in `{}` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            name, span.line
        ),
        Ctx::Line { .. } => format!(
            "error: `call` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            span.line
        ),
    }
}

/// R8: check a call of an *abstract* quotation (one typed only by a declared
/// `Type::Quotation` parameter, with no known literal body) against its
/// declared effect: consume `eff.inputs` deepest-first, then push
/// `eff.outputs`. No splice happens; the declared effect *is* the contract.
/// This is how a quotation-taking word's own body type-checks at its
/// definition site (D4), independent of any call site's literal.
fn check_abstract_quotation_call(
    eff: &QuotEffect,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    op: &str,
) -> Result<Vec<Slot>, String> {
    let n = eff.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, op, n, stack.len()));
    }
    let base = stack.len() - n;
    for (i, want) in eff.inputs.iter().enumerate() {
        let found = stack[base + i];
        match match_slot(found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            _ => return Err(type_mismatch_error(ctx, span, op, *want, found.ty)),
        }
    }
    stack.truncate(base);
    for out in &eff.outputs {
        stack.push(Slot::computed(*out));
    }
    Ok(stack)
}

/// R9: check `f times` for an *abstract* quotation `f`. The count is already
/// verified as an `i64` by the caller path's guard below; here the declared
/// effect must be row-preserving with a trailing `i64` index
/// (`inputs == outputs ++ [i64]`), and the row on the stack is left unchanged.
fn check_abstract_quotation_times(
    eff: &QuotEffect,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
) -> Result<Vec<Slot>, String> {
    let Some(count) = stack.pop() else {
        return Err(underflow_error(ctx, span, "times", 2, 1));
    };
    if count.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, "times"));
    }
    if count.ty != Type::I64 {
        return Err(type_mismatch_error(ctx, span, "times", Type::I64, count.ty));
    }
    let row_preserving = eff.inputs.last() == Some(&Type::I64)
        && eff.inputs.len() == eff.outputs.len() + 1
        && eff.inputs[..eff.outputs.len()] == eff.outputs[..];
    if !row_preserving {
        return Err(times_body_row_effect_error(ctx, span));
    }
    let row_len = eff.outputs.len();
    if stack.len() < row_len {
        return Err(underflow_error(ctx, span, "times", row_len, stack.len()));
    }
    let base = stack.len() - row_len;
    for (i, want) in eff.outputs.iter().enumerate() {
        let found = stack[base + i];
        match match_slot(found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            _ => return Err(type_mismatch_error(ctx, span, "times", *want, found.ty)),
        }
    }
    Ok(stack)
}

/// R18: gather the quotation-taking `WordBody::Terms` words, mono and poly
/// alike (`is_combinator` does not filter on `word.poly`), keyed by name, so a
/// call to one is intercepted and its body spliced (the inliner) rather than
/// lowered to a call to a word that mints no `IrFunc` (R20). `inline_combinator`
/// branches on `word.poly` internally to pick the mono or poly splice path.
fn collect_combinators(words: &[WordDef]) -> CombinatorEnv<'_> {
    let mut map: CombinatorEnv<'_> = HashMap::new();
    for word in words {
        if !is_combinator(word) {
            continue;
        }
        if let WordBody::Terms { terms } = &word.body {
            map.entry(word.name.clone())
                .or_default()
                .push(Combinator { word, terms });
        }
    }
    map
}

/// R2 (Slice 6c): the checker's inline view for one retained combinator, the
/// per-`WordDef` analogue of `collect_combinators`, so the REPL can project its
/// session store into the `HashMap<String, Combinator>` the inline path reads
/// without reaching into `Combinator`'s private fields. `None` for a
/// clause-bodied word (never a combinator: `is_combinator` requires
/// `WordBody::Terms`).
pub(crate) fn combinator_of(word: &WordDef) -> Option<Combinator<'_>> {
    match &word.body {
        WordBody::Terms { terms } => Some(Combinator { word, terms }),
        WordBody::Clauses(_) => None,
    }
}

/// R18/R20: a combinator is a **monomorphic** `WordBody::Terms` word with a
/// `Type::Quotation` input. The checker inlines a call to one (splicing its
/// body) and lowering mints no `IrFunc` for it, so `check` and `ir::lower`
/// must agree on the predicate exactly; it lives here as the single source.
/// Slice 6a phase 2: a **polymorphic** quotation-taking word (`each`/`map`/
/// `fold`) is a combinator too. It never monomorphizes to a standalone
/// `IrFunc` (R20); its body is spliced concretely at each call site, where the
/// element/length variables become the caller's concrete types, so the same
/// splice mechanism serves both the mono and poly cases (the poly signature
/// only drives the standalone def-site check, R17). The quotation parameter
/// sits in `sig.inputs` as either a variable-bearing `PolyType::Quotation` or,
/// when its effect is fully concrete, a `Concrete(Type::Quotation)`.
pub(crate) fn is_combinator(word: &WordDef) -> bool {
    matches!(word.body, WordBody::Terms { .. }) && word_declares_quotation_parameter(word)
}

/// R23 (D7): whether a word's declared effect names a quotation parameter,
/// regardless of body kind (a clause body is rejected separately by
/// `clause_bodied_quotation_word_error`, and a session never reaches a clause
/// body via `eval_def`/`eval_poly_def` at all -- this is the coarser gate the
/// REPL uses, since it cannot retain *any* quotation-taking word's body past
/// the defining line, term-body or not).
pub(crate) fn word_declares_quotation_parameter(word: &WordDef) -> bool {
    match &word.poly {
        None => word
            .effect
            .inputs
            .iter()
            .any(|s| crate::ast::is_quotation_type(s.ty).is_some()),
        Some(sig) => sig.inputs.iter().any(poly_input_is_quotation),
    }
}

/// A polymorphic input slot that declares a quotation parameter: either a
/// variable-bearing effect (`[ 'T -- ]`) or a fully-concrete one that folded
/// to `Concrete(Type::Quotation)`.
fn poly_input_is_quotation(p: &PolyType) -> bool {
    match p {
        PolyType::Quotation(..) => true,
        // Slice 10a (R1): a fully-concrete `~` folds to `Concrete(~)` on the
        // same footing as a fully-concrete ordinary quotation, so the accessor
        // recognizes both. Failing to recognize a `~` here makes the word not a
        // combinator, so it is lowered as an ordinary call and reaches
        // `ir_type_of`'s `unreachable!` -- the ICE this predicate guards.
        PolyType::Concrete(t) => crate::ast::is_quotation_type(*t).is_some(),
        _ => false,
    }
}

/// R22 (D5)/R4 (D5 relaxed): reject a cycle in the quotation-taking-word call
/// subgraph. Edge `A -> B` iff combinator `A`'s body names combinator `B`
/// (any position; a call to a quotation-taking word necessarily passes it a
/// quotation). Since the inliner splices `B`'s body into `A`'s, a cycle would
/// inline forever, so unlike `check_tail_call_cycles` a self-edge is normally
/// the error.
///
/// R4 relaxes this for one shape only: a **self-tail** combinator, whose every
/// self-occurrence is in tail position, gets no self-edge, because the loop
/// transform lowers that self-call to a back-edge (a finite loop) rather than
/// re-splicing forever. A self-name in *any* non-tail position (`all_calls`
/// count exceeds `tail_position_calls` count) keeps its self-edge and stays a
/// cycle error, and every cycle of length >= 2 (a mutual cycle) is untouched.
/// Reuses `check_tail_call_cycles`'s 3-colour DFS shape (recon 8).
pub(crate) fn check_combinator_cycles(combinators: &CombinatorEnv) -> Result<(), String> {
    let members: Vec<&Combinator> = combinators.values().flatten().collect();
    // Slice 8a: two combinators may now share a name (an overload set, R1),
    // so a bare callee name can name more than one node. Unlike
    // `check_tail_call_cycles`'s diagnostic -- where treating an ambiguous
    // name as no edge at all merely costs a runtime optimization on the rare
    // program that hits it -- a missed edge here is not a missed diagnostic,
    // it is the inliner splicing a real cycle forever. So this pass
    // over-approximates: an ambiguous name is an edge to *every* candidate
    // that shares it, never to none, which can only reject a cycle-free
    // program that happens to share a combinator name (rare, and equivalent
    // to renaming one of the two), never miss a real one.
    let mut idx: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in members.iter().enumerate() {
        idx.entry(c.word.name.as_str()).or_default().push(i);
    }
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); members.len()];
    for (i, c) in members.iter().enumerate() {
        let self_name = c.word.name.as_str();
        let self_all = all_calls(&c.word.body)
            .iter()
            .filter(|&&n| n == self_name)
            .count();
        let self_tail = tail_position_calls(&c.word.body)
            .iter()
            .filter(|&&n| n == self_name)
            .count();
        // R4: a tail-only self-edge (every self-occurrence in tail position,
        // and at least one) is permitted -- the loop transform makes it finite.
        let tail_only_self = self_all > 0 && self_all == self_tail;
        for callee in all_calls(&c.word.body) {
            let Some(targets) = idx.get(callee) else {
                continue;
            };
            for &j in targets {
                if i == j && tail_only_self {
                    continue;
                }
                if !adj[i].contains(&j) {
                    adj[i].push(j);
                }
            }
        }
    }
    let mut color = vec![0u8; members.len()];
    let mut path: Vec<usize> = Vec::new();
    for start in 0..members.len() {
        if color[start] == 0 {
            if let Some(cycle) = find_combinator_cycle(start, &adj, &mut color, &mut path) {
                return Err(combinator_cycle_error(&members, &cycle));
            }
        }
    }
    Ok(())
}

/// 3-colour DFS returning the members of the first cycle reached. Unlike
/// `find_tail_cycle`, a self-edge (`v == u`) is a cycle, not skipped.
fn find_combinator_cycle(
    u: usize,
    adj: &[Vec<usize>],
    color: &mut [u8],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    color[u] = 1;
    path.push(u);
    for &v in &adj[u] {
        if color[v] == 1 {
            let start = path.iter().position(|&x| x == v).unwrap();
            return Some(path[start..].to_vec());
        }
        if color[v] == 0 {
            if let Some(cycle) = find_combinator_cycle(v, adj, color, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    color[u] = 2;
    None
}

/// R22: a located cycle rejection naming the members in order and closing the
/// loop back to the first (`` `rec` -> `rec` `` for the self-recursive case).
fn combinator_cycle_error(members: &[&Combinator], cycle: &[usize]) -> String {
    let mut chain: Vec<&str> = cycle
        .iter()
        .map(|&i| crate::resolve::demangle_word(members[i].word.name.as_str()))
        .collect();
    chain.push(chain[0]);
    let rendered = chain
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(" -> ");
    let span = word_span(members[cycle[0]].word);
    format!(
        "error: a quotation-taking word cannot be recursive (the inliner would splice it forever): {} (line {}, col {})",
        rendered, span.line, span.col
    )
}

/// Slice 10a (R11): the back-edge arm's result -- one `Slot` per ground
/// declared output. Extracted as a named, callable function (R14a) so phase 6
/// can drive it from a white-box test: `#[ignore]` skips execution, not
/// compilation, so the test needs a real symbol to call. R14: the `surviving`
/// capture set is forwarded from `carried_inputs` along `index_map`
/// (bottom-aligned: ground output `i` <- `carried_inputs[j]` when
/// `index_map[i] == Some(j)`), so an aggregate carrying an erased quotation
/// across the back-edge keeps its escape obligation (`d1b3f0a`/`bee407c`: a
/// `Slot::computed` drops it, so a bare forward would leak the obligation).
/// `carried_inputs` is itself filtered to non-quotation slots at the call
/// site, so `quot` is always `None` there and never needs forwarding. An
/// output with no source (`None`) is a fresh type-only slot.
fn back_edge_outs(
    ground_outputs: &[Type],
    index_map: &[Option<usize>],
    carried_inputs: &[Slot],
) -> Vec<Slot> {
    ground_outputs
        .iter()
        .enumerate()
        .map(|(i, &ty)| {
            let mut out = Slot::computed(ty);
            if let Some(src) = index_map.get(i).copied().flatten() {
                out.surviving = carried_inputs[src].surviving;
            }
            out
        })
        .collect()
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

/// R18: inline a call to a monomorphic quotation-taking word. Validate each
/// declared input against the caller's live slot (a quotation parameter takes
/// a `Known` literal, checked directionally with the D3 capture check, R11/R12;
/// every other parameter is matched as usual), then splice the callee body
/// against the live stack (bracketed like a `call`, `tail = false`), so the
/// callee's own `call`/`times` fuse against the caller's literals. R22
/// guarantees termination.
#[allow(clippy::too_many_arguments)]
fn inline_combinator(
    comb: &Combinator,
    span: Span,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let name = comb.word.name.as_str();
    // A polymorphic combinator (`each`/`map`/`fold`, or any `'T`-carrying
    // quotation-taking word) keeps its signature in `word.poly`, not
    // `word.effect` (which is empty), so the monomorphic argument loop below
    // would run zero checks and skip R11/R12 entirely (item 3). Route it
    // through the poly-argument check, which resolves the parameter's declared
    // effect against the live stack and runs the *same* directional + D3
    // check, so the two paths agree.
    let poly_subst = if let Some(sig) = comb.word.poly.as_ref() {
        Some(check_poly_combinator_args(
            sig, span, &stack, name, ctx, env, arrays, cells, refs, prov, scope, poly,
        )?)
    } else {
        let inputs: Vec<Type> = comb.word.effect.inputs.iter().map(|s| s.ty).collect();
        let n = inputs.len();
        if stack.len() < n {
            return Err(underflow_error(ctx, span, name, n, stack.len()));
        }
        let base = stack.len() - n;
        for (i, want) in inputs.iter().enumerate() {
            let found = stack[base + i];
            if let Some(eff) = crate::ast::is_quotation_type(*want) {
                if let Some(QuotRef::Known(id)) = found.quot {
                    // Slice 10a (R9, context 4): a monomorphic word's declared
                    // quotation parameter is a `Type::Quotation`/`InlineQuotation`
                    // whose `QuotEffect` carries no row, so the row grounds to
                    // the empty region. (Unreachable for a `~`: `inline_combinator`
                    // routes any poly word here to `check_poly_combinator_args`.)
                    check_literal_against_declared_effect(
                        id,
                        eff,
                        false,
                        &[],
                        name,
                        span,
                        ctx,
                        env,
                        arrays,
                        cells,
                        refs,
                        prov,
                        scope,
                        poly,
                    )?;
                } else if crate::ast::is_quotation_type(found.ty).is_some() {
                    // R21: forwarding an abstract quotation parameter. `found`
                    // is itself a declared quotation parameter of the enclosing
                    // combinator (a `Type::Quotation` slot with no `Known`
                    // literal -- the only way such a slot arises), reached only
                    // while checking that enclosing combinator standalone. At a
                    // real call site the substitution has already bound it to
                    // the caller's literal, so it carries a `Known` marker and
                    // splices there; here, at the def site, accept it when its
                    // declared effect matches the callee parameter, so `outer`
                    // may pass its own `f` to `inner`. The spliced callee
                    // body's own `f call`/`f times` then check the forwarded
                    // parameter against its declared effect (R8/R9).
                    if found.ty != *want {
                        return Err(quotation_argument_required_error(
                            ctx, span, name, *want, found.ty,
                        ));
                    }
                } else {
                    return Err(quotation_argument_required_error(
                        ctx, span, name, *want, found.ty,
                    ));
                }
            } else if found.quot.is_some() {
                return Err(reject_quotation_argument(ctx, span, name));
            } else {
                match match_slot(found, *want) {
                    SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                    SlotMatch::NeedsSizeConversion => {
                        return Err(size_conversion_needed_error(ctx, span, name, *want));
                    }
                    SlotMatch::NeedsStrToCstrConversion => {
                        return Err(str_needs_cstr_conversion_error(ctx, span, name));
                    }
                    SlotMatch::Mismatch => {
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
        }
        None
    };
    // R6: a self-tail combinator opens a splice-time loop. Its body is spliced
    // with `tail = true` so its own tail-position self-call is recognized as
    // the back-edge (above). 6d/R6: the nested-loop rejection is retired --
    // lowering's hoist-target split keeps a nested loop constant-stack -- so
    // opening this loop inside another is now legal and `splice_tail` is just
    // whether this is a self-tail combinator.
    let self_tail = crate::check::is_combinator(comb.word) && has_self_tail_call(comb.word);
    let splice_tail = self_tail;
    let input_count = match comb.word.poly.as_ref() {
        Some(sig) => sig.inputs.len(),
        None => comb.word.effect.inputs.len(),
    };
    // Slice 10a (R11): a self-tail combinator's back-edge needs its ground
    // declared shape (inputs for R12's argument check, outputs and the
    // bottom-aligned index map for the arm's result), which only this set
    // site can compute -- the arm deep in the splice has no `sig`/`Subst`.
    let (ground_inputs, ground_outputs, index_map) = if self_tail {
        back_edge_declared_shape(comb.word, poly_subst.as_ref(), name, span, ctx, arrays)?
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    // R18/R21: splice the callee body, alpha-renamed so its `| ... |` locals
    // cannot collide with a caller local or, under transitive inlining, an
    // outer combinator's locals already in scope. Lowering renames identically
    // (`ir`), so a passed-down literal's captured name stays lexical.
    let uid = prov.inline_uid;
    prov.inline_uid += 1;
    let renamed = crate::ast::alpha_rename_locals(comb.terms, uid);
    let depth = scope.depth();
    let saved_marker = if self_tail {
        let saved = prov.self_tail_combinator.take();
        prov.self_tail_combinator = Some(SelfTailMarker {
            name: name.to_string(),
            input_count,
            ground_inputs,
            ground_outputs,
            index_map,
        });
        Some(saved)
    } else {
        None
    };
    // D1 fix (slice 8b, bug 3): the spliced body is `comb.word`'s own, so a
    // module-scoped visibility gate inside it (D1's drop-import check, 8a's
    // operator scoping) must resolve against the module that declares *it*,
    // not `ctx.module()` -- otherwise a library combinator disposing its own
    // resource gets attributed to whichever module happened to call it.
    let spliced_ctx = ctx.with_module(comb.word.module);
    let result = check_terms(
        &renamed,
        stack,
        &spliced_ctx,
        env,
        arrays,
        cells,
        refs,
        prov,
        scope,
        splice_tail,
        poly,
    );
    if let Some(saved) = saved_marker {
        prov.self_tail_combinator = saved;
    }
    stack = result?;
    leave_block(
        ctx,
        scope,
        depth,
        BlockEnd::Arm {
            token: "inline",
            span,
        },
    )?;
    Ok(stack)
}

/// R11/R12 (poly, item 3): the polymorphic twin of `inline_combinator`'s
/// monomorphic argument loop. A poly combinator's declared inputs live in
/// `sig.inputs`, not `word.effect`, so without this the directional (R11) and
/// D3 capture (R12) checks never ran on the poly argument path -- a caller
/// literal borrowing an enclosing place was silently accepted, a mono/poly
/// divergence in the premise D3 rests on. Resolve the parameter's declared
/// effect against the live stack (`unify_poly_input` binds any variable a
/// non-quotation input carries, e.g. `'T` in `['T ...] [ 'T -- &i64 ]`), then
/// ground the quotation effect and run the *same* `check_literal_against_
/// declared_effect` the monomorphic path uses, so the two agree.
#[allow(clippy::too_many_arguments)]
fn check_poly_combinator_args(
    sig: &PolySig,
    span: Span,
    stack: &[Slot],
    name: &str,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<Subst, String> {
    let n = sig.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, name, n, stack.len()));
    }
    let base = stack.len() - n;
    // Pass 1: unify the non-quotation inputs to resolve theta first, so a
    // variable a quotation effect mentions (`'T` in `[ 'T -- &i64 ]`) is
    // already bound when the effect is grounded in pass 2, whatever the
    // parameter order.
    let mut subst = Subst::default();
    for (i, pin) in sig.inputs.iter().enumerate() {
        if poly_input_is_quotation(pin) {
            continue;
        }
        let found = stack[base + i];
        if found.quot.is_some() {
            return Err(reject_quotation_argument(ctx, span, name));
        }
        unify_poly_input(sig, pin, found.ty, name, span, ctx, arrays, &mut subst)?;
    }
    // Pass 2: ground each quotation parameter and run the directional + D3
    // check on its caller literal.
    for (i, pin) in sig.inputs.iter().enumerate() {
        if !poly_input_is_quotation(pin) {
            continue;
        }
        let found = stack[base + i];
        let concrete = apply_subst(sig, pin, &subst, name, span, ctx, arrays)?;
        // Slice 10a (R1): `apply_subst` grounds an ordinary quotation parameter
        // to `Type::Quotation` and (phase 2) a `~` parameter to
        // `Type::InlineQuotation`; the accessor accepts both, so this let-else
        // never becomes a spurious `unreachable!` once `~` grounding lands.
        let Some(eff) = crate::ast::is_quotation_type(concrete) else {
            unreachable!("a quotation input grounds to a quotation type (apply_subst)")
        };
        // Slice 10a (R9, context 1): a row-bearing declared quotation parameter
        // grounds its row to the concrete caller-stack region below the
        // combinator's fixed inputs (`stack[..base]`). Per R4 that row is the
        // signature's own top-level row, so it grounds to the same region the
        // top-level row does. A parameter that declared no row grounds against
        // the empty region. `apply_subst` deliberately left the row off the
        // interned `eff` (splicing it would mint an effect no literal equals),
        // so it is reconstructed here, at the callee, and only type-only
        // (`Slot::computed`, dropping provenance, R16).
        let row: Vec<Type> = match pin {
            PolyType::Quotation(_, _, _, Some(_), _) => {
                stack[..base].iter().map(|s| s.ty).collect()
            }
            _ => Vec::new(),
        };
        if let Some(QuotRef::Known(id)) = found.quot {
            let is_inline = matches!(concrete, Type::InlineQuotation(_));
            check_literal_against_declared_effect(
                id, eff, is_inline, &row, name, span, ctx, env, arrays, cells, refs, prov, scope,
                poly,
            )?;
        } else if crate::ast::is_quotation_type(found.ty).is_some() {
            // R21 (poly): a forwarded abstract quotation parameter, accepted
            // when its declared effect matches (the spliced body's own
            // `call`/`times` re-checks it, R8/R9).
            if found.ty != concrete {
                return Err(quotation_argument_required_error(
                    ctx, span, name, concrete, found.ty,
                ));
            }
        } else {
            return Err(quotation_argument_required_error(
                ctx, span, name, concrete, found.ty,
            ));
        }
    }
    // Slice 10a (R11): the resolved `θ` is no longer discarded -- the back-edge
    // marker grounds the declared outputs through it (`inline_combinator`).
    Ok(subst)
}

/// R11/R12: check a quotation *literal* against a declared quotation parameter
/// directionally (slice 4 D3): seed a fresh sub-stack with the declared input
/// row, run the literal's body against it, and require the exit row to equal
/// the declared output row (no standalone effect is inferred). Enforce the D3
/// capture restriction here (R12): a read that consumes a non-`Copy` enclosing
/// local, or a borrow of an enclosing place left on the row, is rejected; a
/// `Copy` local read by value is allowed.
#[allow(clippy::too_many_arguments)]
fn check_literal_against_declared_effect(
    id: QuotId,
    eff: &QuotEffect,
    is_inline: bool,
    row: &[Type],
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
) -> Result<(), String> {
    let body = prov.quotations[id.0].body.clone();
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
    let mut fresh: Vec<Slot> = row.iter().map(|t| Slot::computed(*t)).collect();
    fresh.extend(eff.inputs.iter().map(|t| Slot::computed(*t)));
    let depth = scope.depth();
    let result = check_terms(
        &body, fresh, ctx, env, arrays, cells, refs, prov, scope, false, poly,
    )?;
    // R12: a linear enclosing local the literal consumed (move-state changed
    // from `Live`).
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
    // R12: a borrow of an enclosing place left live on the literal's exit row.
    for slot in &result {
        if let Some(did) = slot.deriv {
            if let Some(place) = &prov.deriv(did).owned_root {
                if outer_locals.contains(place) {
                    return Err(quotation_borrows_place_error(ctx, span, word, place));
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
    // R11: the literal's exit row must equal the grounded declared output row:
    // the same carried region `row` followed by the declared outputs. N=0
    // leaves the region untouched and N≥2 feeds one iteration's output into the
    // next, so the carried region is a fixed point (spec: "one row, the same on
    // both sides").
    let expected_out: Vec<Type> = row
        .iter()
        .copied()
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
    Ok(())
}

/// R6 (Q1): does the quotation `body` read any name in `enclosing` that the
/// body does not itself bind? The cheap boolean the D3 materialization line
/// needs (no captures / captures), strictly less work than 7b's capture *set*.
/// Mirrors `alpha_rename_locals`'s walk (ast.rs): a `Call` strips a leading
/// `&!`/`&` exactly as `rename_call`, and a nested `TermKind::Quotation` / `if`
/// arm is walked carrying the body-bound names *by value*, so a read of an
/// outer name from inside a nested quotation still counts (D4's
/// capture-into-another-quotation case). Pure over the term tree: it inspects
/// no `Slot`/`Deriv` state, so it is testable in isolation.
fn body_captures_enclosing(body: &[Term], enclosing: &HashSet<String>) -> bool {
    fn walk(terms: &[Term], enclosing: &HashSet<String>, bound: &mut Vec<String>) -> bool {
        for term in terms {
            match &term.kind {
                TermKind::Bind(names) => {
                    for n in names {
                        bound.push(n.clone());
                    }
                }
                TermKind::Call(name) => {
                    let stripped = name
                        .strip_prefix("&!")
                        .or_else(|| name.strip_prefix('&'))
                        .unwrap_or(name);
                    if enclosing.contains(stripped) && !bound.iter().any(|b| b == stripped) {
                        return true;
                    }
                }
                TermKind::Quotation(inner) => {
                    let mut inner_bound = bound.clone();
                    if walk(inner, enclosing, &mut inner_bound) {
                        return true;
                    }
                }
                TermKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    let mut tb = bound.clone();
                    let mut eb = bound.clone();
                    if walk(then_branch, enclosing, &mut tb) {
                        return true;
                    }
                    if walk(else_branch, enclosing, &mut eb) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(body, enclosing, &mut Vec::new())
}

/// R24: an escaping closure captures a local of the *current* frame whose
/// storage dies at return. Wording modeled on `reference_across_back_edge_error`
/// (`:5501`): the same "a local of this frame ... does not survive" shape, one
/// frame instead of one loop iteration. Fires for `make-a` (R15) and, in
/// Phase 2, for a frame capture escaping through a returned carrier (R22).
fn past_owning_frame_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let _ = ctx;
    format!(
        "error: an escaping closure captures `{name}`, a local of this frame, whose storage does not survive the return (line {})",
        span.line,
    )
}

/// R24: a captured reference is read after its referent's last use -- the
/// referent is consumed or exclusively re-borrowed while an erased closure
/// still holds a borrow of it (kept live past the store by R20's surviving-set
/// union). Fires only for a capture the surviving set added; a still-`Known`
/// closure or a genuinely-live borrow keeps `conflicting_borrow_error`.
fn past_last_use_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let _ = ctx;
    format!(
        "error: a captured reference to `{name}` is read after its last use (line {})",
        span.line,
    )
}

/// R18: an escaping closure captures more than one value. Phase 1 stores a
/// single word-sized capture inline in the `env` slot; a 2+-capture escaping
/// closure needs a heap env, deferred with the rest of the heap-env case.
/// Review fix: also fires at R22 when a 2+-capture closure's stack-allocated
/// env bundle (R16) escapes transitively through a returned carrier -- the
/// bundle's storage dies at return exactly as the direct-return case would.
fn multi_capture_escaping_error(ctx: &Ctx, span: Span) -> String {
    let _ = ctx;
    format!(
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line {})",
        span.line,
    )
}

/// R15 case 4: a captured quotation-typed name. Admitting it would need a
/// two-word `(code, env)` env slot and a recursive surviving-set fold no exit
/// criterion requires, so it is deferred, parallel to the 2+-capture deferral.
fn captured_quotation_name_deferred_error(ctx: &Ctx, span: Span) -> String {
    let _ = ctx;
    format!(
        "error: capturing a quotation value by name is deferred (line {})",
        span.line,
    )
}

/// Slice 10a (R2): the fifth materialization boundary. Distinct wording from
/// `captured_quotation_name_deferred_error` -- an ordinary quotation capture
/// is *deferred* (unimplemented, might land later); a `~` capture is *banned*
/// (materialization is exactly what `~` forbids, permanently).
fn captured_inline_quotation_error(ctx: &Ctx, span: Span) -> String {
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: a `~` quotation cannot be captured in `{where_}` (line {})",
        span.line,
    )
}

/// R15: how a captured name's referent is rooted, which decides whether it may
/// outlive the closure's calls.
enum CaptureClass {
    /// A scalar local: snapshotted into the env (D4 amendment), so it can never
    /// dangle and is admissible at every boundary.
    Scalar,
    /// An aggregate value or borrow whose referent lives in the current frame:
    /// its storage dies at return.
    FrameRooted,
    /// An aggregate value or borrow rooted in a parameter or global: its
    /// referent outlives the frame.
    OuterRooted,
}

/// Whether a reference's root names a *current-frame* local -- the same
/// frame-vs-outer test `classify_capture`'s `Type::Ref` arm applies, reused so
/// a `!`/`+!` store through a reference computes its own escaping boundary
/// the same way (a store through a reference rooted outside the frame is a
/// materialization boundary the closure can outlive, exactly like a return).
fn ref_root_is_in_frame(deriv: Option<DerivId>, prov: &Provenance, scope: &Scope) -> bool {
    match deriv {
        Some(id) => match &prov.deriv(id).owned_root {
            Some(place) => scope.local(place).is_some(),
            None => false,
        },
        None => false,
    }
}

/// R15: classify a captured name by its binding (case 4 quotation-typed names
/// are peeled off before this runs). The frame-rooted/outer-rooted split reuses
/// the exact `owned_root`-vs-current-frame test the R12 exit-row check
/// (`:6108`) and `check_reference_across_back_edge` (`:5519`) already apply.
fn classify_capture(b: &Binding, prov: &Provenance, scope: &Scope) -> CaptureClass {
    match b.ty {
        // Case 2: an aggregate value read directly (no deriv). A scope-bound
        // aggregate is owned by (and dies with) this frame; a global aggregate
        // is not in scope and never reaches here.
        //
        // Review note (deliberate narrowing, not a bug): R15 case 2 also names
        // a by-value aggregate PARAMETER or global as outer-rooted (its
        // storage belongs to the caller, not this frame), which this arm does
        // not distinguish from a locally-constructed aggregate -- both reach
        // here with `deriv: None`, and telling them apart would need a new
        // provenance tag threaded from `check_terms_word`'s initial stack
        // through every bind/shuffle (the same weight as `Deriv` tracking for
        // `Type::Ref`, case 3's own mechanism). Unbuilt: this arm always
        // returns `FrameRooted`, so an aggregate parameter capture is
        // over-rejected at an escaping boundary rather than admitted -- sound
        // (it never under-rejects), just more conservative than case 2's full
        // rule. See `docs/phase4-slice7b-spec.md`'s R15 section.
        Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..) => {
            CaptureClass::FrameRooted
        }
        // Case 3: a borrow. A `Deriv` whose `owned_root` names a current-frame
        // local is frame-rooted; a `&T` parameter carries no deriv (or a
        // reborrow with no owned root) and is outer-rooted by construction,
        // matching `check_reference_across_back_edge`'s own `deriv` handling.
        Type::Ref(..) => {
            if ref_root_is_in_frame(b.deriv, prov, scope) {
                CaptureClass::FrameRooted
            } else {
                CaptureClass::OuterRooted
            }
        }
        // Case 1: any other local is a scalar.
        _ => CaptureClass::Scalar,
    }
}

/// R15: admit or reject a capturing quotation at a materialization boundary
/// (7b, replacing 7a's blanket R12 rejection). `escaping` is true at a
/// word-output boundary (`be returned`, or a differing-arm `if` join feeding
/// the declared output), false at an in-frame store. A four-way classification
/// on capture kind (D3's cached free-name set is the source, no new analysis):
/// a scalar snapshot admits everywhere; an outer-rooted aggregate/borrow admits
/// everywhere; a frame-rooted one is past-owning-frame at a word-output
/// boundary (R24) and admitted at an in-frame one (R21, Phase 2); a captured
/// quotation-typed name is deferred everywhere (case 4). An escaping closure's
/// inline env holds one word, so a 2+-capture escaping closure is deferred
/// (R18); an in-frame one takes a stack bundle (R16), so 2+ is admitted.
///
/// Returns the interned surviving capture set (R19): the admitted
/// aggregate/borrow captures whose referents must outlive the closure's calls,
/// each tagged frame-rooted (for the R22 escape guard). A scalar-only closure
/// returns `None` -- a snapshot has no referent that can go dead.
fn check_capture_admission(
    id: QuotId,
    escaping: bool,
    span: Span,
    ctx: &Ctx,
    prov: &mut Provenance,
    scope: &Scope,
) -> Result<Option<SurvivingCaptureSetId>, String> {
    // Only names bound in the enclosing scope are real captures; a free global
    // word resolves at the call and needs no env.
    let mut names: Vec<String> = prov
        .quotation_captures(id)
        .iter()
        .filter(|n| scope.local(n).is_some())
        .cloned()
        .collect();
    names.sort_unstable();
    if names.is_empty() {
        return Ok(None);
    }
    // Case 4: a captured quotation-typed name (a `Known` literal, `quot.is_some()`,
    // or an already-erased `Type::Quotation`) is deferred at every boundary.
    for name in &names {
        let b = scope.local(name).expect("filtered to a bound name");
        // Slice 10a (R2): the fifth materialization boundary. A `~` local is
        // exactly a quotation-typed binding; without the accessor it passes
        // this guard, is recorded in a surviving capture set, and lowering
        // materializes it into an env bundle -- the one thing `~` forbids.
        // Phase 2 replaces the message with a `~`-specific one plus a golden.
        if matches!(b.ty, Type::InlineQuotation(_)) {
            return Err(captured_inline_quotation_error(ctx, span));
        }
        if b.quot.is_some() || crate::ast::is_quotation_type(b.ty).is_some() {
            return Err(captured_quotation_name_deferred_error(ctx, span));
        }
    }
    // Classify each capture and, for an aggregate/borrow one, record it as a
    // surviving-set member (R19). A scalar snapshot is never a member (D4
    // amendment); a frame-rooted capture escaping a word-output boundary is
    // rejected here (R24) before any set is built (make-a).
    let mut members: Vec<SurvivingCapture> = Vec::new();
    for name in &names {
        let b = scope.local(name).expect("filtered to a bound name");
        match classify_capture(b, prov, scope) {
            CaptureClass::Scalar => {}
            CaptureClass::FrameRooted => {
                if escaping {
                    return Err(past_owning_frame_error(ctx, span, name));
                }
                members.push(SurvivingCapture {
                    name: name.clone(),
                    frame_rooted: true,
                });
            }
            CaptureClass::OuterRooted => members.push(SurvivingCapture {
                name: name.clone(),
                frame_rooted: false,
            }),
        }
    }
    // R18: an escaping closure's inline env holds one word; a 2+-capture
    // escaping closure needs a heap env, deferred. An in-frame one takes the
    // stack bundle (R16), so any count is admitted here -- but review fix:
    // the bundle marker rides onto the interned set regardless, since the
    // in-frame admission is only sound until this closure escapes through a
    // later carrier, which the R22 guard checks at the word-output boundary.
    let bundle = names.len() >= 2;
    if escaping && bundle {
        return Err(multi_capture_escaping_error(ctx, span));
    }
    Ok(prov.intern_surviving_set(members, bundle))
}

/// R7/R15/D4: a materialization boundary. Materialize a non-capturing `Known`
/// literal into a runtime quotation value, or run the R15 admission rule on a
/// capturing one. (i) run the boolean capture gate (R6); (ii) if it captures,
/// admit or reject per R15 against the caller-computed `escaping` (true at a
/// word-output/branch-join boundary the closure cannot outlive its call from,
/// or at a store through a reference rooted outside the current frame; false
/// at a genuinely in-frame boundary); (iii) confirm the literal against the
/// boundary's expected `Type::Quotation(eff)` via
/// `check_literal_against_declared_effect`, and return the slot *erased*
/// (`quot: None`, a real `Type::Quotation`) -- the signal `call`/`times` read
/// to emit an indirect call rather than a splice.
#[allow(clippy::too_many_arguments)]
fn materialize_quotation_at_boundary(
    id: QuotId,
    eff: &'static QuotEffect,
    escaping: bool,
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
) -> Result<Slot, String> {
    let enclosing: HashSet<String> = scope.bound.iter().map(|b| b.name.clone()).collect();
    let body = prov.quotations[id.0].body.clone();
    let surviving = if body_captures_enclosing(&body, &enclosing) {
        check_capture_admission(id, escaping, span, ctx, prov, scope)?
    } else {
        None
    };
    // Slice 10a (R9): an `eff` reaching an erasure boundary is a `QuotEffect`
    // with no row, so the row grounds to the empty region.
    check_literal_against_declared_effect(
        id,
        eff,
        false,
        &[],
        word,
        span,
        ctx,
        env,
        arrays,
        cells,
        refs,
        prov,
        scope,
        poly,
    )?;
    // R19: the erased slot carries the surviving capture set in place of the
    // dropped `Known` marker, the signal `capture_alive_names` (R20) and the
    // R22 escape guard read once the identity is gone.
    Ok(Slot {
        surviving,
        ..Slot::computed(Type::Quotation(eff))
    })
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

/// R18: `times` reached without a statically-known quotation literal on top
/// (D4). Parallel to `call_needs_quotation_error`.
fn times_needs_quotation_error(ctx: &Ctx, span: Span) -> String {
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `times` in `{}` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            name, span.line
        ),
        Ctx::Line { .. } => format!(
            "error: `times` (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            span.line
        ),
    }
}

/// R18: the body is spliced once but runs N times, so a linear outer local it
/// consumes would be disposed of more than once. The single most important
/// `times` checker rule.
fn times_body_consumes_local_error(ctx: &Ctx, span: Span, name: &str) -> String {
    format!(
        "error: a `times` body cannot consume `{name}`{} (line {}): the body runs more than once, so the value would be disposed of more than once",
        in_word(ctx),
        span.line,
    )
}

/// R18: a reference the body derives would cross the back-edge into the next
/// iteration. A borrow is idempotent per iteration, so a well-formed body
/// leaves `live_derivs` unchanged; this fires when it does not.
fn times_body_borrow_across_loop_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: a `times` body cannot leave a reference live across the loop{} (line {}): the local it borrows does not survive to the next iteration",
        in_word(ctx),
        span.line,
    )
}

/// R18/D6: the body's net effect on the row is not identity -- it must consume
/// the index and return the row it received unchanged.
fn times_body_row_effect_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: a `times` body must leave the row unchanged{} (line {}): it takes `( ..s i64 -- ..s )`, consuming the index and returning the same row",
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

/// An unknown type name in a conversion word (X6), e.g. `>i128`.
fn conversion_unknown_type_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown type `{name}` in `{wname}` (line {})",
            span.line
        ),
        Ctx::Line { .. } => format!("error: unknown type `{name}`"),
    }
}

fn branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `if` branches leave different stack depths (then: {}, else: {})\n  note: declared {}",
            name, span.line, d_then, d_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `if` branches leave different stack depths (then: {d_then}, else: {d_else})"
        ),
    }
}

fn branch_type_mismatch_error(ctx: &Ctx, span: Span, t_then: Type, t_else: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `if` branches leave different types (then: `{}`, else: `{}`)\n  note: declared {}",
            name, span.line, t_then, t_else, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `if` branches leave different types (then: `{t_then}`, else: `{t_else}`)"
        ),
    }
}

/// The borrow-suspension bookkeeping must agree at a branch join, real
/// content the type-only shape unification above does not supply. One arm
/// suspending a place the other leaves unsuspended (or suspending a
/// *different* place) is rejected rather than silently picking one arm's
/// answer, since a later hazard check would then reason about the wrong arm's
/// runtime path.
fn borrow_join_disagreement_error(
    ctx: &Ctx,
    span: Span,
    t_then: Option<&Deriv>,
    t_else: Option<&Deriv>,
) -> String {
    let describe = |d: Option<&Deriv>| match d.map(Deriv::suspension) {
        None => "no live borrow".to_string(),
        Some((Some(root), Some(place))) => {
            format!("a borrow of `{root}` reborrowed from `{place}`")
        }
        Some((Some(root), None)) => format!("a borrow of `{root}`"),
        Some((None, Some(place))) => format!("a reborrow of `{place}`"),
        Some((None, None)) => "a borrow with no local root".to_string(),
    };
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: borrow state disagrees at the `if`/`else` join in `{}` (line {})\n  the `then` arm leaves {}, the `else` arm leaves {}: both arms must agree on which place, if any, stays borrowed past the join\n  note: declared {}",
            name, span.line, describe(t_then), describe(t_else), effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: borrow state disagrees at the `if`/`else` join (line {})\n  the `then` arm leaves {}, the `else` arm leaves {}",
            span.line, describe(t_then), describe(t_else),
        ),
    }
}

/// Apply an arithmetic/comparison/conversion operator if `name` is one,
/// returning `Some(stack)`; `None` if the name is none of those (the caller
/// then looks it up in the env). `+ - *` are homogeneous over the numeric
/// types (int or float, `bool` is never numeric): both operands must be the
/// *same* type, producing that type; no implicit promotion (R6). `/` is
/// float-only: both operands must be the same float type (R7). `mod` stays
/// integer-only: both operands must be the same integer type (R8). `= < >`
/// generalise the same way as `+ - *` but always produce `bool` (R9). A
/// conversion word is `>` followed by a known numeric type name
/// (`>i8`..`>u64`, `>f32`, `>f64`): pop one numeric value, push the named
/// target (R10). `and`/`or`/`xor` are homogeneous over the integer types and
/// `bool` (float is rejected), same shape as `mod`; on two `bool`s they *are*
/// logical and/or/xor, since a stack language evaluates both operands eagerly
/// so bitwise-on-0/1 and logical coincide. `not` is unary: integer or `bool`
/// in, same type out (int stays bitwise complement, `bool` is logical
/// negation; the difference is only in how `lower_call` codegens it).
/// `shl`/`shr` take an integer value and always an `i64` shift count,
/// producing the value's type. `<= >= <>` generalise the same way as `= < >`:
/// numeric-only (never `bool`), same type, producing `bool`. `.` is
/// type-directed over any primitive printable scalar (every integer width or
/// either float width): pops one, produces nothing; the concrete type picks
/// the print codegen (signed/unsigned decimal, or `%g` float) at the call
/// site. `bool` is not a row here (slice 9 R6): `true .`/`false .` fall
/// through to the injected library overload below, which prints
/// `true`/`false` by delegating to the `str` row.
/// The outcome of resolving an operator name against `BUILTIN_TABLE` and, on a
/// builtin-row exact miss, an optional same-named user overload (slice 8a
/// phase 2, R6). The single resolution entry point both `check_term`'s probe
/// chain and `poly_delegate_op` route through.
enum OpDispatch {
    /// Resolved to a builtin row (or the `>T` conversion), carrying the new
    /// stack; the caller pushes nothing further.
    Builtin(Vec<Slot>),
    /// A user overload of this builtin name matched the operands exactly and
    /// beats the numeric coercion fallback (R2). Carries the candidate's
    /// lowering symbol, which the caller records for the site (R7) before
    /// dispatching through the ordinary `env` word-call path; the stack is
    /// left untouched here.
    UserOverload(String),
    /// The name is not a table operator (nor a `>T` conversion): fall through
    /// to the next probe in the chain.
    NotOperator,
}

fn check_operator(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    user_overload: Option<&[Overload]>,
) -> Result<OpDispatch, String> {
    // R11: every operator this function handles reads the top slot, so a
    // quotation on top is always an operand of it. Guard once, gated on the
    // name being one we handle (else fall through so a later dispatcher can
    // claim it), before the type-directed reads that would otherwise spell the
    // `Cstr` placeholder into a mismatch.
    // This name list mirrors `BUILTIN_TABLE`'s keys (plus the `>T` conversions,
    // which are name-parsed, not table rows). Keep it in sync when a table
    // operator is added. It is not derived from `BUILTIN_TABLE.contains_key`
    // on purpose: the guard must also cover `>T`, and `is_unary` below can't be
    // read off row arity without changing `>=` (which the `>`-prefix test
    // already treats as unary here).
    let is_operator = matches!(
        name,
        "+" | "-"
            | "*"
            | "/"
            | "mod"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "shl"
            | "shr"
            | "="
            | "<"
            | ">"
            | "<="
            | ">="
            | "<>"
            | "max"
            | "max-total"
            | "."
    ) || name.strip_prefix('>').is_some_and(|r| !r.is_empty());
    // The unary members (`not`, print, the `>T` conversions) read only the
    // top; every other operator reads a pair, so its deeper operand at
    // `stack[n - 2]` is an operand of it too. Guarding the top alone lets a
    // quotation there fall through to `operand_pair_mismatch_error`, which
    // spells the `Cstr` placeholder into the message the audit exists to keep
    // hidden.
    let is_unary =
        matches!(name, "not" | ".") || name.strip_prefix('>').is_some_and(|r| !r.is_empty());
    if is_operator && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if is_operator && !is_unary && stack.len() >= 2 && stack[stack.len() - 2].quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    // Unify a homogeneous binary op's operand pair, honoring D8's literal
    // coercion (`Ok`); `Err(Some(target))` is the size-type/computed-`i64`
    // X10 case, naming which size type (`usize`/`isize`) needed the explicit
    // conversion; `Err(None)` is a plain mismatch the caller reports with its
    // own op-specific diagnostic.
    let unify = |a: Slot, b: Slot| -> Result<Type, Option<Type>> {
        match unify_pair(a, b) {
            PairMatch::Ok(ty) => Ok(ty),
            PairMatch::NeedsSizeConversion(target) => Err(Some(target)),
            PairMatch::Mismatch => Err(None),
        }
    };

    // Slice 8a (R6/Q-A): dispatch selection is table-driven. Every operator
    // this function handles has one or more concrete rows in `BUILTIN_TABLE`;
    // a call resolves by an exact operand-type lookup there first, so a user
    // overload of the name can later shadow a call site (phase 2). Only on an
    // exact miss does the numeric operand-class guard + `unify_pair` coercion
    // below run, as a hand-written fallback whose diagnostics are preserved
    // byte-for-byte (Q-B). `not`'s in-place identity became a `(T -- T)` row,
    // so the exact hit pushes a fresh slot; the corpus never feeds a literal
    // `not` to a compile-time count, so the dropped literal flag is invisible.
    let Some(rows) = BUILTIN_TABLE.get(name) else {
        // Not a table operator: the `>T` numeric conversions stay hand-written
        // (R0), dispatched by parsing the target type out of the name rather
        // than keyed on operand type, so no row can hold them.
        let Some(rest) = name.strip_prefix('>').filter(|r| !r.is_empty()) else {
            return Ok(OpDispatch::NotOperator);
        };
        let target = match Type::from_name(rest) {
            Some(ty) if ty.is_numeric() => ty,
            _ => return Err(conversion_unknown_type_error(ctx, span, rest)),
        };
        let source = *stack.last().ok_or_else(|| need(name, 1, stack.len()))?;
        if !source.ty.is_numeric() {
            return Err(conversion_source_error(ctx, span, name, source.ty));
        }
        stack.pop();
        stack.push(Slot::computed(target));
        return Ok(OpDispatch::Builtin(std::mem::take(stack)));
    };
    // Every row for one name agrees on arity (R4), so the first row's input
    // count is the operand count to read.
    let arity = rows[0].inputs.len();
    if stack.len() < arity {
        return Err(need(name, arity, stack.len()));
    }
    let base = stack.len() - arity;
    let operands: Vec<Type> = stack[base..].iter().map(|s| s.ty).collect();
    if let Some(hit) = rows.iter().find(|r| r.inputs == operands) {
        stack.truncate(base);
        stack.extend(hit.outputs.iter().map(|ty| Slot::computed(*ty)));
        return Ok(OpDispatch::Builtin(std::mem::take(stack)));
    }

    // Slice 8a phase 2 (R2/R6): a user overload of this builtin name whose
    // inputs match the operands exactly beats the numeric coercion fallback
    // below, so a call the builtin already answers is untouched (corpus
    // byte-for-byte, since no corpus word overloads a builtin name) while a
    // `Vec2 +` site is redirected to the user word. Checked only on a
    // builtin-row exact miss. Both callers pass their candidate set, so a
    // poly body's delegated operator resolves an overload the same way a
    // monomorphic call site does.
    if let Some(candidates) = user_overload {
        if let Some(chosen) = resolve_overload(candidates, &operands) {
            return Ok(OpDispatch::UserOverload(chosen.symbol.clone()));
        }
    }

    match name {
        "+" | "-" | "*" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_numeric() || !b.ty.is_numeric() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "/" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_float() || !b.ty.is_float() || a.ty != b.ty {
                return Err(div_requires_float_error(ctx, span, a.ty, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "mod" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_int() || !b.ty.is_int() {
                return Err(mod_requires_int_error(ctx, span, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => mod_requires_int_error(ctx, span, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "and" | "or" | "xor" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !(a.ty.is_int() || a.ty.is_bool()) || !(b.ty.is_int() || b.ty.is_bool()) {
                return Err(bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "not" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(name, 1, n));
            }
            let a = stack[n - 1];
            if !(a.ty.is_int() || a.ty.is_bool()) {
                return Err(bitwise_not_requires_int_error(ctx, span, a.ty));
            }
        }
        "shl" | "shr" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_int() {
                return Err(shift_value_requires_int_error(ctx, span, name, a.ty));
            }
            if b.ty != Type::I64 {
                return Err(shift_count_requires_i64_error(ctx, span, name, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "=" | "<" | ">" | "<=" | ">=" | "<>" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_numeric() || !b.ty.is_numeric() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(Type::BOOL));
        }
        // R12 (S6): `max ( 'T 'T -- 'T )`, an internal `Ord` bound resolved
        // against the integer tower (`is_int`, which already includes
        // `usize`/`isize`, D7). A float pair is rejected by name (X9),
        // directing to `max-total` (R13) rather than pretending IEEE `>` is
        // total (D6); the pair must still agree on one concrete type exactly
        // like `+`/`>`.
        "max" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if a.ty.is_float() || b.ty.is_float() {
                return Err(max_over_float_error(ctx, span, a.ty, b.ty));
            }
            if !a.ty.is_int() || !b.ty.is_int() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        // R13 (S6): `max-total ( 'F 'F -- 'F )`, `f32`/`f64` only, ordered by
        // the `total_cmp` bit-pattern rule rather than IEEE `>` (D6). An
        // integer pair is rejected by name (X10), directing to `max`.
        "max-total" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_float() || !b.ty.is_float() {
                return Err(max_total_requires_float_error(ctx, span, a.ty, b.ty));
            }
            if a.ty != b.ty {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "." => {
            let n = stack.len();
            if n < 1 {
                return Err(need(".", 1, n));
            }
            let a = stack[n - 1];
            if !a.ty.is_numeric() && !a.ty.is_bool() && !matches!(a.ty, Type::Str | Type::Cstr) {
                return Err(print_requires_printable_error(ctx, span, a.ty));
            }
            stack.truncate(n - 1);
        }
        _ => unreachable!("BUILTIN_TABLE holds only these operator names"),
    }
    Ok(OpDispatch::Builtin(std::mem::take(stack)))
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

/// `S|>fi` (R10) applied to a linear field: unlike `S>fi`, a peek must leave
/// the aggregate live, so it can't also transfer ownership of a linear
/// field's value; the workaround is `S>` (destructure the whole aggregate).
fn peek_of_linear_field_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: cannot `{}` a linear field in `{}` (line {})\n  the field has type `{}`, which is linear and has no `Copy` instance, so it cannot be peeked without consuming the aggregate; use `S>` to destructure instead\n  note: declared {}",
            op, name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `{op}` a linear field: the field has type `{found}`, which is linear and has no `Copy` instance"
        ),
    }
}

/// An owning-cell word (`^>`/`^|>`) applied to a non-cell operand: names the
/// word and the offending operand type, mirroring `array_word_operand_error`.
fn owned_cell_word_operand_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an owning-cell operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires an owning-cell operand, found `{found}`")
        }
    }
}

/// `^|>` on a linear payload: the cell stays live afterward, so peeking
/// would leave a second, unowned reference to a resource the cell still
/// owns. `^>` (consuming unwrap) is the workaround.
fn peek_of_linear_owned_payload_error(
    ctx: &Ctx,
    span: Span,
    cell_ty: Type,
    payload: Type,
) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: cannot `^|>` a linear payload in `{}` (line {})\n  `{}` holds a payload of type `{}`, which is linear and has no `Copy` instance, so it cannot be peeked without consuming the cell; use `^>` to unwrap instead\n  note: declared {}",
            name, span.line, cell_ty, payload, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `^|>` a linear payload: `{cell_ty}` holds a payload of type `{payload}`, which is linear and has no `Copy` instance"
        ),
    }
}

/// A constant (literal) index out of range for a `[T N]` (X4, R11): a compile
/// error naming the length `N` and the offending index.
fn array_index_out_of_range_error(ctx: &Ctx, span: Span, count: u32, index: i64) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: array index out of range in `{}` (line {})\n  index {} is out of bounds for length {}\n  note: declared {}",
            name, span.line, index, count, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: array index out of range: index {index} is out of bounds for length {count}"
        ),
    }
}

/// `fill` given a *computed* (non-literal) count (M1): the count must be a
/// compile-time literal, since there is no comptime interpreter to fold it.
fn fill_count_not_literal_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `fill` requires a literal count, found a computed `{}` (no const-expr eval)\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `fill` requires a literal count, found a computed `{found}` (no const-expr eval)"
        ),
    }
}

/// `fill` given a literal count `< 1` (or `> u32::MAX`): an array length must
/// be `>= 1` (X2, M1), named against the offending count.
fn fill_count_out_of_range_error(ctx: &Ctx, span: Span, count: i64) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: invalid array length in `{}` (line {})\n  `fill` count {} is invalid (an array length must be >= 1 and <= {})\n  note: declared {}",
            name, span.line, count, u32::MAX, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: `fill` count {count} is invalid (an array length must be >= 1 and <= {})",
            u32::MAX
        ),
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

/// An exact `usize` is a runtime index; a bare integer literal coerces and
/// gets a compile-time bounds check; a computed `i64` needs an explicit
/// `>usize`; anything else is a plain type mismatch.
fn check_array_index(
    index: Slot,
    count: u32,
    ctx: &Ctx,
    span: Span,
    op: &str,
) -> Result<(), String> {
    match match_slot(index, Type::Usize) {
        SlotMatch::Exact => Ok(()),
        SlotMatch::LiteralSizeType => {
            let idx = index.int_val.expect("a literal slot carries its value");
            if idx < 0 || idx >= i64::from(count) {
                return Err(array_index_out_of_range_error(ctx, span, count, idx));
            }
            Ok(())
        }
        SlotMatch::NeedsSizeConversion => {
            Err(size_conversion_needed_error(ctx, span, op, Type::Usize))
        }
        // A `str` index is a plain mismatch: the str-to-cstr case can only
        // arise where a `cstr` is wanted, and an index always wants `usize`.
        SlotMatch::NeedsStrToCstrConversion | SlotMatch::Mismatch => {
            Err(type_mismatch_error(ctx, span, op, Type::Usize, index.ty))
        }
    }
}

/// The referent of a reference type, and whether it is mutable.
fn ref_parts(ty: Type, refs: &[RefDecl]) -> Option<(Type, bool)> {
    match ty {
        Type::Ref(id, mutable, _) => Some((refs[id.index()].referent, mutable)),
        _ => None,
    }
}

/// `&x`/`&!x` applied to something that is not a local. A place is a
/// local name and nothing more, so the diagnostic names what was found there
/// and points at the binding that would make it one.
fn borrow_of_non_place_error(ctx: &Ctx, span: Span, spelled: &str, found: &str) -> String {
    format!(
        "error: `{spelled}` does not borrow a place{} (line {}, col {})\n  {found}\n  a place is a local name; bind the value with `| name |` first, then borrow that name",
        in_word(ctx),
        span.line,
        span.col
    )
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
/// `ty` names itself through this one guard instead. Only `call`/`times`
/// consume a quotation; the shuffles forward it and `drop` discards it.
fn reject_quotation_operand(ctx: &Ctx, span: Span, op: &str) -> String {
    format!(
        "error: `{op}`{} (line {}) cannot take a quotation as an operand; only `call` and `times` accept a quotation (a runtime quotation value is slice 7)",
        in_word(ctx),
        span.line,
    )
}

/// R8: a quotation stored into an array (`fill`'s element) or through a
/// reference (`!`/`+!`'s value, whether the referent is an array slot, a
/// struct field, or an owned cell) would have to become a runtime value,
/// which this slice cannot represent. The wording names no container because
/// two of the three store paths have none. Shared by all of them (D4).
fn reject_quotation_stored(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: a quotation cannot be stored (escaping quotations are slice 7){} (line {})",
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
        "error: a quotation cannot be passed to `{word}`; only `call` and `times` accept one (a runtime quotation value is slice 7){} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// R7, both arms leave a quotation but not the *same* literal: a quotation's
/// body must be statically known where it is used, and a branch merge that
/// picked one arm's would need a runtime code value (D4). Fires at the join,
/// not at consumption (R12's containment rests on it).
fn different_quotations_at_join_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: these two branches leave different quotations at line {}{}; give the quotation a declared type (a word output or field) so it can be materialized, or make both arms the same literal (a runtime quotation value is slice 7)",
        span.line,
        in_word(ctx),
    )
}

/// R7, one arm leaves a quotation and the other a value: the `Cstr`
/// placeholder makes the two `ty`s compare equal, so the ordinary branch-type
/// mismatch never catches this; the join guard does.
fn quotation_versus_value_at_join_error(ctx: &Ctx, span: Span) -> String {
    format!(
        "error: one branch of the `if` at line {}{} leaves a quotation and the other does not; a quotation cannot be a runtime value (a runtime quotation value is slice 7)",
        span.line,
        in_word(ctx),
    )
}

/// Only an aggregate or cell local may be borrowed. A scalar local is an
/// SSA temporary with no address, and giving it one is work no criterion
/// needs.
fn borrow_of_scalar_local_error(ctx: &Ctx, span: Span, local: &str, ty: Type) -> String {
    format!(
        "error: cannot borrow the scalar local `{local}` of type `{ty}`{} (line {}, col {})\n  a scalar has no address; borrow a field or an aggregate instead",
        in_word(ctx),
        span.line,
        span.col
    )
}

/// `&x`/`&!x` applied to a local that is *already* a reference. A borrow
/// is only ever taken of a plain aggregate local, and the remedy is to drop
/// the sigil: naming a reference local reborrows it.
fn borrow_of_reference_local_error(ctx: &Ctx, span: Span, local: &str, ty: Type) -> String {
    format!(
        "error: cannot borrow `{local}`{}: it is already the reference `{ty}` (line {}, col {})\n  write `{local}`, not `{spelled}{local}`; naming a reference local reborrows it",
        in_word(ctx),
        span.line,
        span.col,
        spelled = if matches!(ty, Type::Ref(_, true, _)) { "&!" } else { "&" },
    )
}

/// A reference-mode word applied to something that is not the reference shape
/// it projects through (`&[T N]` for `&>`, `&^T` for `&^`, `&T` for `@`).
fn reference_word_operand_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    expected: &str,
    found: Type,
) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{name}` (line {})\n  `{op}` expected {expected}, found `{found}`\n  note: declared {}",
            span.line,
            effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` expected {expected}, found `{found}`")
        }
    }
}

/// `!`/`+!` through a shared reference. Storing through a `&T` is
/// meaningless, and the mutable spelling is right there.
fn store_through_shared_reference_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    format!(
        "error: `{op}` cannot store through the shared reference `{found}`{} (line {})\n  borrow it mutably with `&!` (and project with the `&!`-spelled accessors) to write through it",
        in_word(ctx),
        span.line
    )
}

/// `@`/`!`/`+!` are restricted to a `Copy` referent. Fetching a linear
/// value through a reference would manufacture a second owner; storing over
/// one would silently leak the value being overwritten (nothing auto-drops).
fn access_of_linear_referent_error(ctx: &Ctx, span: Span, op: &str, referent: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    let why = if op == "@" {
        "fetching one would make a second owner of a value that is used exactly once"
    } else {
        "storing over one would silently leak the value being overwritten; nothing auto-drops"
    };
    format!(
        "error: `{op}` cannot access the linear referent `{referent}`{} (line {})\n  {why}",
        in_word(ctx),
        span.line
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

/// Naming a `&!` local reborrows it, and a reborrow may not be taken
/// while anything derived from the previous one is still live — the two would be
/// two simultaneous mutable references into the same place.
fn suspended_place_error(ctx: &Ctx, span: Span, place: &str, live: &Deriv) -> String {
    format!(
        "error: cannot reborrow `{place}`{} while a reference derived from it is live (line {}, col {})\n  the derivation taken at line {}, col {} is still live\n  a mutable borrow suspends its place until every reference derived from it is consumed",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// Consuming a place — moving it into a word, or disposing of it — while a
/// reference derived from it is still live. The reference would be left aimed at
/// storage its owner has given away.
fn consume_of_borrowed_place_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    ty: Type,
    live: &Deriv,
) -> String {
    let held = if live.mutable { "mutable" } else { "shared" };
    format!(
        "error: cannot consume the borrowed local `{place}` of type `{ty}`{} (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  a place stays borrowed until every reference derived from it is consumed",
        in_word(ctx),
        span.line,
        span.col,
        live.span.line,
        live.span.col,
    )
}

/// A mutable borrow of a place a second live name denotes. Naming an
/// aggregate does not copy it, so two locals — or a local and a value still on
/// the virtual stack — can denote one region; mutating through one would then be
/// silently observable through the other, which is exactly the class of silent
/// failure the language exists to reject.
fn aliased_place_borrow_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    origin: &AliasOrigin<'_>,
) -> String {
    let (alias, other, remedy) = match origin {
        AliasOrigin::Name(name) => (
            format!("`{name}`"),
            format!("`{name}`"),
            "use `dup` for an independent copy",
        ),
        AliasOrigin::Stack(pushed) => (
            format!(
                "a value on the stack (pushed at line {}, col {})",
                pushed.line, pushed.col
            ),
            "that value".to_string(),
            "`dup` that value for an independent copy, or consume it before taking the borrow",
        ),
    };
    format!(
        "error: cannot borrow `{place}` mutably{} (line {}, col {}): it is aliased by {alias}\n  both denote one region of memory, so a mutation through `{place}` would be silently visible through {other}\n  {remedy}",
        in_word(ctx),
        span.line,
        span.col,
    )
}

/// The symmetric direction: naming an aggregate while a mutable borrow of
/// its storage is live. The converse of an exclusivity rule is
/// easy to omit, and this is that omission: checking only at the borrow
/// catches `v ... &!v` and misses `&!v ... v`, which is the same hazard with the
/// two terms swapped.
fn naming_aliases_borrowed_place_error(ctx: &Ctx, span: Span, name: &str, live: &Deriv) -> String {
    format!(
        "error: cannot name `{name}`{} (line {}, col {}): a mutable borrow of it is still live (line {}, col {})\n  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates\n  finish with the borrow first, or `dup` for an independent copy",
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

/// Every `&`-led word — the two prefix borrow operators and the
/// reference-mode accessor family. Returns `None` if `name` is not `&`-led
/// (the caller falls through to the ordinary lookup chain).
///
/// One spelling per shape *and* per mutability: the mutability is in the
/// token, never inherited from the receiver, so a reader gets reference-ness,
/// mutability and arity from the word alone. Every accessor consumes its
/// reference argument the way any word consumes its arguments.
#[allow(clippy::too_many_arguments)]
fn check_reference_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    scope: &Scope,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &mut Vec<RefDecl>,
    prov: &mut Provenance,
    live: &Liveness,
    at: usize,
) -> Result<Option<Vec<Slot>>, String> {
    if !name.starts_with('&') {
        return Ok(None);
    }
    let mutable = name.starts_with("&!");
    let rest = &name[if mutable { 2 } else { 1 }..];
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);

    match rest {
        ">" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            if stack[n - 1].quot.is_some() || stack[n - 2].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            let index = stack[n - 1];
            let Some((referent, recv_mut)) = ref_parts(stack[n - 2].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an array",
                    stack[n - 2].ty,
                ));
            };
            let Type::Array(id, _) = referent else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an array",
                    stack[n - 2].ty,
                ));
            };
            if recv_mut != mutable {
                let want = intern_ref_type(refs, referent, mutable);
                return Err(type_mismatch_error(ctx, span, name, want, stack[n - 2].ty));
            }
            let (count, elem) = (arrays[id.index()].count, arrays[id.index()].element);
            check_array_index(index, count, ctx, span, name)?;
            let out = intern_ref_type(refs, elem, mutable);
            let deriv = prov.project(stack[n - 2].deriv);
            stack.truncate(n - 2);
            stack.push(Slot::derived(out, deriv));
        }
        "^" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(name, 1, n));
            }
            if stack[n - 1].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            let Some((referent, recv_mut)) = ref_parts(stack[n - 1].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an owning cell",
                    stack[n - 1].ty,
                ));
            };
            let Type::OwnedCell(cell_id, _) = referent else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a reference to an owning cell",
                    stack[n - 1].ty,
                ));
            };
            if recv_mut != mutable {
                let want = intern_ref_type(refs, referent, mutable);
                return Err(type_mismatch_error(ctx, span, name, want, stack[n - 1].ty));
            }
            let payload = cells[cell_id.index()].payload;
            let out = intern_ref_type(refs, payload, mutable);
            let deriv = prov.project(stack[n - 1].deriv);
            stack.truncate(n - 1);
            stack.push(Slot::derived(out, deriv));
        }
        _ => {
            if let Some((struct_name, field_name)) = rest.split_once('>') {
                if let Some(idx) = ctx.structs().iter().position(|d| d.name == struct_name) {
                    let decl = &ctx.structs()[idx];
                    if let Some(field_ty) = decl
                        .fields
                        .iter()
                        .find(|(f, _)| f == field_name)
                        .map(|(_, ty)| *ty)
                    {
                        let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
                        let want = intern_ref_type(refs, struct_ty, mutable);
                        let n = stack.len();
                        if n < 1 {
                            return Err(need(name, 1, n));
                        }
                        if stack[n - 1].quot.is_some() {
                            return Err(reject_quotation_operand(ctx, span, name));
                        }
                        if stack[n - 1].ty != want {
                            return Err(type_mismatch_error(
                                ctx,
                                span,
                                name,
                                want,
                                stack[n - 1].ty,
                            ));
                        }
                        let out = intern_ref_type(refs, field_ty, mutable);
                        let deriv = prov.project(stack[n - 1].deriv);
                        stack.truncate(n - 1);
                        stack.push(Slot::derived(out, deriv));
                        return Ok(Some(std::mem::take(stack)));
                    }
                }
            }
            // Everything else is a prefix borrow of a local, and only of a
            // local.
            if rest.is_empty() {
                return Err(borrow_of_non_place_error(
                    ctx,
                    span,
                    name,
                    "it names nothing (a bare sigil cannot borrow whatever happens to be on the stack)",
                ));
            }
            let Some(local_ty) = scope.local_type(rest) else {
                let found = if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    format!("`{rest}` is a literal, not a local")
                } else {
                    format!("`{rest}` is not a local in scope")
                };
                return Err(borrow_of_non_place_error(ctx, span, name, &found));
            };
            // R11: `&q` on a quotation local currently reaches
            // `borrow_of_scalar_local_error`, whose message lies about the
            // `Cstr` placeholder; reject with the named-op wording instead.
            if scope.local(rest).is_some_and(|b| b.quot.is_some()) {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            if local_ty.is_ref() {
                return Err(borrow_of_reference_local_error(ctx, span, rest, local_ty));
            }
            if !matches!(
                local_ty,
                Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..)
            ) {
                return Err(borrow_of_scalar_local_error(ctx, span, rest, local_ty));
            }
            // Borrowing is not a move, but the referent still has to be
            // there. A local consumed earlier holds nothing, and borrowing it
            // would read (and project through) storage its owner has already
            // freed.
            if let Some(site) = scope.moves.moved_site(rest) {
                return Err(use_after_move_error(ctx, span, rest, local_ty, site));
            }
            // Exclusivity, symmetric. A new mutable borrow conflicts with
            // any live borrow of the place; a new shared one conflicts with a
            // live mutable borrow. Per place, never a global counter: two live
            // `&!` rooted at different locals do not conflict.
            if let Some(id) = live_deriv(stack, scope, prov, live, at, |d| {
                d.owned_root.as_deref() == Some(rest) && (mutable || d.mutable)
            }) {
                // R24: if the conflicting borrow is live *only* because an
                // erased closure's surviving set keeps its holder alive past
                // its last syntactic use (R20), this borrow reads a captured
                // reference past that last use -> past-last-use, naming the
                // captured reference. A still-`Known` closure or a genuinely
                // live borrow keeps the conflicting-borrow wording.
                if let Some(captured) = past_last_use_capture(stack, scope, prov, live, at, id) {
                    return Err(past_last_use_error(ctx, span, &captured));
                }
                return Err(conflicting_borrow_error(
                    ctx,
                    span,
                    rest,
                    mutable,
                    prov.deriv(id),
                ));
            }
            // A second live name for one region makes a mutation through
            // this borrow silently observable through that name. Checked here
            // *and* symmetrically at the naming: a naming that comes first is
            // caught here, one that comes later is caught there. Naming an
            // aggregate with no `&!` anywhere near it stays free either way.
            if mutable {
                if let Some(origin) = aliasing_origin(stack, scope, prov, live, at, rest) {
                    return Err(aliased_place_borrow_error(ctx, span, rest, &origin));
                }
            }
            let out = intern_ref_type(refs, local_ty, mutable);
            let deriv = prov.borrow(rest, mutable, span);
            stack.push(Slot::derived(out, Some(deriv)));
        }
    }
    Ok(Some(std::mem::take(stack)))
}

/// `@` fetches, `!` stores, `+!` adds in place. All three are restricted
/// to a `Copy` referent, which covers a Copy *aggregate* as well as a Copy
/// scalar; `@` is typed for both `&T` and `&!T` directly, so there is no
/// `&!T -> &T` demotion coercion to write.
#[allow(clippy::too_many_arguments)]
fn check_access_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    refs: &[RefDecl],
    scope: &Scope,
    prov: &Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "@" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("@", 1, n));
            }
            if stack[n - 1].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "@"));
            }
            let Some((referent, _)) = ref_parts(stack[n - 1].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    "@",
                    "a reference",
                    stack[n - 1].ty,
                ));
            };
            if !is_copy(referent, ctx.structs(), ctx.enums(), arrays) {
                return Err(access_of_linear_referent_error(ctx, span, "@", referent));
            }
            // Review fix: `@` reads an *element* of an aggregate the
            // reference roots into (an array slot, a struct field), not the
            // whole named place, so it never sees a `surviving` set that
            // rides on a `Slot` directly (only a store onto the root binding,
            // R20, records one). Look the root binding up by the reference's
            // own provenance (`owned_root`, generic over array/struct/cell
            // chains) and forward its surviving set (if any) onto the
            // fetched value -- the same fetch-side half of the store-side
            // union `!`/`+!` already performs.
            let surviving = stack[n - 1]
                .deriv
                .and_then(|id| prov.deriv(id).owned_root.clone())
                .and_then(|root| scope.local(&root))
                .and_then(|b| b.surviving);
            stack.truncate(n - 1);
            stack.push(Slot {
                surviving,
                ..Slot::computed(referent)
            });
        }
        "!" | "+!" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let value = stack[n - 1];
            // R8r: guard the stored value strictly above the `match_slot`
            // below, which returns `Exact` on the `Cstr` placeholder into a
            // `&!Cstr` referent (a silent accept) rather than a mismatch. The
            // receiver operand is an ordinary R11 default-deny.
            if value.quot.is_some() {
                return Err(reject_quotation_stored(ctx, span));
            }
            if stack[n - 2].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, name));
            }
            let Some((referent, mutable)) = ref_parts(stack[n - 2].ty, refs) else {
                return Err(reference_word_operand_error(
                    ctx,
                    span,
                    name,
                    "a mutable reference",
                    stack[n - 2].ty,
                ));
            };
            if !mutable {
                return Err(store_through_shared_reference_error(
                    ctx,
                    span,
                    name,
                    stack[n - 2].ty,
                ));
            }
            if !is_copy(referent, ctx.structs(), ctx.enums(), arrays) {
                return Err(access_of_linear_referent_error(ctx, span, name, referent));
            }
            if name == "+!" && !referent.is_int() {
                return Err(type_mismatch_error(ctx, span, "+!", Type::I64, referent));
            }
            match match_slot(value, referent) {
                SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
                SlotMatch::NeedsSizeConversion => {
                    return Err(size_conversion_needed_error(ctx, span, name, referent));
                }
                SlotMatch::NeedsStrToCstrConversion => {
                    return Err(str_needs_cstr_conversion_error(ctx, span, name));
                }
                SlotMatch::Mismatch => {
                    return Err(type_mismatch_error(ctx, span, name, referent, value.ty));
                }
            }
            stack.truncate(n - 2);
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

/// Apply an array word (`fill`/`len`) if `name` is one, returning
/// `Some(stack)`; `None` if the name is not an array word (the caller then
/// looks it up in the env). These are generic over the array shape, so
/// (like the shuffles and numeric operators) they dispatch on the concrete
/// operand types rather than a fixed env signature (R6, R10):
///
/// - `fill ( T -- [T N] )`: the top slot is the compile-time count `N` (a
///   literal, M1), the slot below is the element `T`; interns the `(T, N)`
///   shape (R3) and pushes it.
/// - `len ( [T N] -- usize )`: **non-consuming**, folds to the constant `N`.
///
/// Element access is a reference word (`&>`/`&!>` then `@`/`!`), not an
/// array word: it goes through `check_access_word` instead.
/// The two `str`-only words: `len ( str -- usize )` (R8) and `cstr
/// ( str -- cstr )` (R7, the one explicit `str` -> `cstr` conversion — there
/// is no reverse). Tried before `check_array_word`, whose own `len` claims
/// the name unconditionally otherwise: returning `None` here when the
/// operand isn't a `str` lets that array path still see it.
fn check_str_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
) -> Result<Option<Vec<Slot>>, String> {
    // R11: `len`/`cstr` inspect the top operand's `ty`; reject a quotation
    // here (before `len` falls through to the array path on a non-`str`).
    if matches!(name, "len" | "cstr") && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    match name {
        "len" => {
            let Some(top) = stack.last() else {
                return Ok(None);
            };
            if top.ty != Type::Str {
                return Ok(None);
            }
            stack.pop();
            stack.push(Slot::computed(Type::Usize));
        }
        "cstr" => {
            let n = stack.len();
            if n < 1 {
                return Err(underflow_error(ctx, span, "cstr", 1, n));
            }
            let top = stack[n - 1];
            if top.ty != Type::Str {
                return Err(cstr_conversion_source_error(ctx, span, top.ty));
            }
            stack.truncate(n - 1);
            stack.push(Slot::computed(Type::Cstr));
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

fn check_array_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
) -> Result<Option<Vec<Slot>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "fill" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("fill", 2, n));
            }
            let count = stack[n - 1];
            let element = stack[n - 2];
            // R8f: a quotation element would have to become a runtime array
            // value. Guarded strictly above `contains_reference` below, whose
            // registry index would panic on an aggregate placeholder (R4); the
            // `Cstr` placeholder is registry-free but the guard order is what
            // R4's reasoning pins. A quotation count is a plain operand (R11).
            if element.quot.is_some() {
                return Err(reject_quotation_stored(ctx, span));
            }
            if count.quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "fill"));
            }
            let Some(count_val) = count.int_val else {
                return Err(fill_count_not_literal_error(ctx, span, count.ty));
            };
            if !(1..=i64::from(u32::MAX)).contains(&count_val) {
                return Err(fill_count_out_of_range_error(ctx, span, count_val));
            }
            // A construction site the declaration-site rule cannot reach: `fill` accepts
            // any `Copy` element, and `&T` is `Copy`, so the declaration-site
            // sweep never sees this shape. D2's shared gate owns this check
            // (no zero-safety: `fill` replicates a real seed, D4).
            check_array_element_gate(
                ctx,
                span,
                "fill",
                element.ty,
                ctx.structs(),
                ctx.enums(),
                arrays,
                false,
            )?;
            let array_ty = intern_array_type(arrays, element.ty, count_val as u32);
            // Review fix: forward the element's surviving set (R19) onto the
            // array -- `fill` replicates one closure-carrying element N
            // times, so the array as a whole is that closure's carrier
            // exactly as a struct/enum constructor's output is.
            let surviving = element.surviving;
            stack.truncate(n - 2);
            stack.push(Slot {
                surviving,
                ..Slot::computed(array_ty)
            });
        }
        "len" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("len", 1, n));
            }
            if stack[n - 1].quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "len"));
            }
            if !matches!(stack[n - 1].ty, Type::Array(..)) {
                return Err(array_word_operand_error(ctx, span, "len", stack[n - 1].ty));
            }
            // Non-consuming: the array stays; `len` folds to the constant `N`.
            stack.push(Slot::computed(Type::Usize));
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

/// The three owning-cell access words: `^ ( T -- ^T )` constructs a cell,
/// `^> ( ^T -- T )` consumes it and yields the payload, `^|> ( ^T -- ^T T )`
/// is a non-consuming peek restricted to a `Copy` payload. Matched by exact
/// name only, so `^>x`/`^|>x` fall through to the ordinary unknown-word error.
fn check_owned_cell_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &mut Vec<OwnedCellDecl>,
) -> Result<Option<Vec<Slot>>, String> {
    // R11: `^`/`^>`/`^|>` each inspect the top operand's `ty`.
    if matches!(name, "^" | "^>" | "^|>") && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "^" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^", 1, n));
            }
            let payload = stack[n - 1].ty;
            // Another construction site the declaration-site rule cannot reach: `^` interns a
            // cell over any payload type with no filter of its own.
            if contains_reference(payload, ctx.structs(), ctx.enums(), arrays) {
                return Err(constructed_reference_error(
                    ctx,
                    span,
                    "the payload `^` would store",
                    payload,
                ));
            }
            // Review fix: forward the payload's surviving set (R19) onto the
            // cell -- `^` allocating a closure-carrying value must keep it
            // visible to R22's return guard exactly as a struct/enum
            // constructor does.
            let surviving = stack[n - 1].surviving;
            let cell_ty = intern_owned_cell_type(cells, payload);
            stack.truncate(n - 1);
            stack.push(Slot {
                surviving,
                ..Slot::computed(cell_ty)
            });
        }
        "^>" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^>", 1, n));
            }
            let Type::OwnedCell(id, _) = stack[n - 1].ty else {
                return Err(owned_cell_word_operand_error(
                    ctx,
                    span,
                    "^>",
                    stack[n - 1].ty,
                ));
            };
            // Review fix: forward the cell's own surviving set onto the
            // extracted payload -- the inverse of `^`'s forward above.
            let surviving = stack[n - 1].surviving;
            let payload = cells[id.index()].payload;
            stack.truncate(n - 1);
            stack.push(Slot {
                surviving,
                ..Slot::computed(payload)
            });
        }
        "^|>" => {
            let n = stack.len();
            if n < 1 {
                return Err(need("^|>", 1, n));
            }
            let cell_ty = stack[n - 1].ty;
            let Type::OwnedCell(id, _) = cell_ty else {
                return Err(owned_cell_word_operand_error(ctx, span, "^|>", cell_ty));
            };
            let payload = cells[id.index()].payload;
            if !is_copy(payload, ctx.structs(), ctx.enums(), arrays) {
                return Err(peek_of_linear_owned_payload_error(
                    ctx, span, cell_ty, payload,
                ));
            }
            // Non-consuming: the cell stays, the payload copy is pushed atop it.
            // Review fix: forward the cell's surviving set (R19) onto the
            // peeked copy too, same as `^>`'s consuming fetch.
            stack.push(Slot {
                surviving: stack[n - 1].surviving,
                ..Slot::computed(payload)
            });
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
}

/// `S|>fi` (R10): a new non-consuming `( S -- S field )` peek, keyed by the
/// per-struct-per-field name (unlike `fill`, it is not generic over a
/// shape, so it is not a fixed entry in `struct_generated_sigs`
/// either: it is looked up by parsing the `Struct|>field` name against the
/// struct registry, same as the IR's `structs.words` map). `None` if `name`
/// doesn't split on `|>` or doesn't resolve to a known struct+field (the
/// caller falls through to the env lookup, so an unrelated word still gets
/// the ordinary unknown-word error). A linear field is rejected outright
/// (R10): the peek would leave a second, unowned reference to a resource the
/// aggregate still owns, with no reference machinery to make that legal.
fn check_struct_peek_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let Some((struct_name, field_name)) = name.split_once("|>") else {
        return Ok(None);
    };
    let structs = ctx.structs();
    let Some(idx) = structs.iter().position(|d| d.name == struct_name) else {
        return Ok(None);
    };
    let decl = &structs[idx];
    let Some((_, field_ty)) = decl.fields.iter().find(|(f, _)| f == field_name) else {
        return Ok(None);
    };
    let field_ty = *field_ty;
    if !is_copy(field_ty, structs, ctx.enums(), arrays) {
        return Err(peek_of_linear_field_error(ctx, span, name, field_ty));
    }
    let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
    let n = stack.len();
    if n < 1 {
        return Err(underflow_error(ctx, span, name, 1, n));
    }
    let top = stack[n - 1];
    if top.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if top.ty != struct_ty {
        return Err(type_mismatch_error(ctx, span, name, struct_ty, top.ty));
    }
    // The peek is non-consuming and pushes the field's *interior address*,
    // so two peeks of one field of one struct are two names for one region.
    let alias = peek_region(&mut stack[n - 1], field_ty, field_name, span, prov);
    // Review fix: forward the struct operand's surviving set (R19) onto the
    // peeked field -- a closure the struct carries stays visible through a
    // peek exactly as it would through the consuming getter below.
    stack.push(Slot {
        alias,
        surviving: top.surviving,
        ..Slot::computed(field_ty)
    });
    Ok(Some(std::mem::take(stack)))
}

/// `S>fi` (R21's third route): the ordinary, consuming field getter, already
/// registered in `struct_generated_sigs` and otherwise left to the generic
/// env-based dispatch. That generic path pushes a plain `Slot::computed`
/// with no alias, but for an aggregate field this getter's IR lowering
/// pushes the field's *interior address* rather than copying it out (same
/// device as `S|>fi`'s peek), so the struct operand and the extracted field
/// alias one region exactly as two peeks would. `None` for a scalar field
/// (no region to alias) or an unresolved name, so every other call site is
/// untouched. Consuming, unlike the peek: the struct operand is popped, not
/// left on the stack, but the aliasing hazard is unaffected by that, since
/// the operand's own local binding (if it is named) keeps the same region
/// regardless of what happens to the stack-level copy of its slot.
fn check_struct_get_word(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let Some((struct_name, field_name)) = name.split_once('>') else {
        return Ok(None);
    };
    let structs = ctx.structs();
    let Some(idx) = structs.iter().position(|d| d.name == struct_name) else {
        return Ok(None);
    };
    let decl = &structs[idx];
    let Some((_, field_ty)) = decl.fields.iter().find(|(f, _)| f == field_name) else {
        return Ok(None);
    };
    let field_ty = *field_ty;
    if !field_ty.is_aggregate() {
        return Ok(None);
    }
    let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
    let n = stack.len();
    if n < 1 {
        return Err(underflow_error(ctx, span, name, 1, n));
    }
    let top = stack[n - 1];
    if top.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if top.ty != struct_ty {
        return Err(type_mismatch_error(ctx, span, name, struct_ty, top.ty));
    }
    let alias = peek_region(&mut stack[n - 1], field_ty, field_name, span, prov);
    stack.truncate(n - 1);
    // Review fix: forward the struct operand's surviving set (R19) onto the
    // extracted field -- an aggregate field carrying a closure (a nested
    // struct/array/cell) must keep that closure visible to R22's return
    // guard past this getter.
    stack.push(Slot {
        alias,
        surviving: top.surviving,
        ..Slot::computed(field_ty)
    });
    Ok(Some(std::mem::take(stack)))
}

/// Apply a stack shuffle if `name` is one, returning `Some(stack)`; `None` if
/// the name is not a shuffle (the caller then looks it up in the env). Shuffles
/// move concrete slot types with no fixed signature: `dup` of a `bool` yields
/// two `bool`s, `swap` reorders whatever two types are on top, etc.
/// R1 (slice 8b, D2): the sole authority on whether a scoped name is visible to
/// a module. A name owned by `defining` is visible to `caller` iff `defining` is
/// the caller's own module, or the caller selectively imported that bare name
/// from that module. A qualified-only import (`import: lib "lib.sth"`) makes
/// nothing visible by bare name, so it is not a route here. Consumed by D1's
/// `drop` gate and (phase 3) 8a's operator fix; neither invents its own rule.
fn is_name_visible_to_module(
    modules: &[ModuleInfo],
    caller: u32,
    defining: u32,
    name: &str,
) -> bool {
    defining == caller || modules[caller as usize].selective.get(name) == Some(&defining)
}

/// R12 (slice 8b, 8a): the operator overloads of `name` visible to the calling
/// module. `None` means "module scoping does not apply -- use the flat
/// `env.get(name)`": the REPL path (`ctx.modules()` is `None`) and a
/// single-module build, where `resolve_modules` leaves an operator decl bare
/// (`+`, not `+__m0`) so the flat lookup already finds the own overload. In a
/// multi-module build every operator decl is mangled per module, so a bare
/// lookup of `+` is `None`; assemble the caller's own overload (under
/// `mangle(name, M)`) plus one it selectively imported, membership decided by
/// `is_name_visible_to_module` (R1), never re-derived.
fn scoped_operator_overloads(
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    name: &str,
) -> Option<Vec<Overload>> {
    // Only a builtin operator name is left bare by `resolve_modules` and so
    // scoped here; every other bare call was already rewritten to its mangled
    // spelling, and re-mangling that would only miss (`foo__m0__m0`). This is
    // also what keeps the fall-through env-call path (which reads this result)
    // from corrupting an ordinary word's candidate lookup.
    if !BUILTIN_TABLE.contains_key(name) {
        return None;
    }
    let modules = ctx.modules()?;
    if modules.len() < 2 {
        return None;
    }
    let caller = ctx.module();
    let mut defining = vec![caller];
    if let Some(&k) = modules[caller as usize].selective.get(name) {
        defining.push(k);
    }
    let mut out: Vec<Overload> = Vec::new();
    for d in defining {
        if is_name_visible_to_module(modules, caller, d, name) {
            if let Some(cands) = env.get(&crate::resolve::mangle(name, d)) {
                out.extend(cands.iter().cloned());
            }
        }
    }
    Some(out)
}

/// R4 (slice 8b, D1): reject a bare `drop` of an imported resource type whose
/// `drop` override is not visible to the calling module. The name checked is the
/// struct's demangled source spelling, since `ModuleInfo::selective` is keyed by
/// source names while `decl.name` is mangled (`Res__m1`) in a >=2-module build.
fn check_drop_import_visibility(
    ctx: &Ctx,
    span: Span,
    m: &[ModuleInfo],
    decl: &StructDecl,
) -> Result<(), String> {
    let source = crate::resolve::demangle_word(&decl.name);
    if is_name_visible_to_module(m, ctx.module(), decl.module, source) {
        Ok(())
    } else {
        Err(drop_import_visibility_error(ctx, span, m, decl, source))
    }
}

/// R5 (slice 8b): the located diagnostic for a `drop` whose destructor lives in
/// a module the caller imported qualified-only. Names the demangled type under
/// the qualifier the caller binds it (the qualifier whose import maps to the
/// declaring module) and the remedy: import the type by name. The `Ctx::Line`
/// arm drops the enclosing-word clause, though the REPL path never reaches the
/// gate (`ctx.modules()` is `None` there, R8).
fn drop_import_visibility_error(
    ctx: &Ctx,
    span: Span,
    m: &[ModuleInfo],
    decl: &StructDecl,
    source: &str,
) -> String {
    let caller = ctx.module() as usize;
    let qualifier = m[caller]
        .imports
        .iter()
        .find(|(_, &target)| target == decl.module)
        .map(|(q, _)| q.as_str());
    // `ModuleInfo` carries no name or path of its own (only its import map,
    // exports, and selective names), so a struct reachable only
    // *transitively* -- the caller imports some module that imports the
    // declaring one, but never imports the declaring one itself -- has no
    // qualifier to name here. Naming the struct's own bare name as if it
    // were a module qualifier (the prior behavior) reads as a valid import
    // spelling that silently fails; say plainly that the path is transitive
    // instead of fabricating one.
    let (ty_name, note) = match qualifier {
        Some(qualifier) => (
            format!("{qualifier}::{source}"),
            format!(
                "disposing it runs a `drop` destructor declared in module `{qualifier}`, which this module has not imported by name\n  note: add `{source}` to the import (`import: {qualifier} | {source} | \"...\"`), or dispose it in a module that declares `{source}`"
            ),
        ),
        None => (
            source.to_string(),
            format!(
                "disposing it runs a `drop` destructor declared in a module this module never imports directly -- it is only reachable transitively, through another module's import\n  note: import the module that declares `{source}` directly, then add `{source}` to that import"
            ),
        ),
    };
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot `drop` a value of type `{ty_name}` in `{name}` (line {})\n  {note}",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot `drop` a value of type `{ty_name}` (line {})\n  {note}",
            span.line
        ),
    }
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

    /// Slice 10a (R1): a fully-concrete `~` folds to `Concrete(InlineQuotation)`,
    /// which the routing predicate must recognize -- else the word is not a
    /// combinator, is lowered as an ordinary call, and reaches `ir_type_of`'s
    /// `unreachable!`. Constructed directly, no parser.
    #[test]
    fn poly_input_is_quotation_recognizes_inline() {
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let ord = crate::ast::quotation_type(vec![Type::I64], Vec::new());
        assert!(poly_input_is_quotation(&PolyType::Concrete(inl)));
        assert!(poly_input_is_quotation(&PolyType::Concrete(ord)));
        assert!(!poly_input_is_quotation(&PolyType::Concrete(Type::I64)));
    }

    /// Slice 10a (R1): a monomorphic word whose input is a `~` still counts as
    /// declaring a quotation parameter (accessor-routed), so it is inlined
    /// rather than lowered to a call.
    #[test]
    fn word_declares_quotation_parameter_recognizes_inline() {
        use crate::ast::TypedSlot;
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let w = WordDef {
            name: "w".to_string(),
            effect: StackEffect {
                inputs: vec![TypedSlot {
                    name: None,
                    ty: inl,
                }],
                outputs: Vec::new(),
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            module: 0,
            span: Span::default(),
        };
        assert!(word_declares_quotation_parameter(&w));
    }

    /// Slice 10a (R2): the declaration-position rejection is no longer fail-open
    /// for a `~` -- it used to return `Ok` (`if let Type::Quotation`), letting a
    /// `~` slip past silently. Constructed directly.
    #[test]
    fn reject_quotation_type_position_rejects_inline() {
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let err = reject_quotation_type_position(inl, "a struct field").unwrap_err();
        assert!(err.contains("~[ i64 -- ]"), "names the `~` type: {err}");
        assert!(err.contains("a struct field"), "names the position: {err}");
        // The ordinary quotation is still rejected in this position too.
        let ord = crate::ast::quotation_type(vec![Type::I64], Vec::new());
        assert!(reject_quotation_type_position(ord, "a struct field").is_err());
    }

    /// Slice 10a (R2): a word declaring a `~` *output* is rejected by the audit,
    /// where a bare `Type::Quotation` output is allowed (a materialization
    /// boundary). The `~` cannot be materialized, so it is never a legal output.
    #[test]
    fn audit_rejects_inline_quotation_output_but_allows_ordinary() {
        use crate::ast::TypedSlot;
        let mk = |ty: Type| WordDef {
            name: "w".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: vec![TypedSlot { name: None, ty }],
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            module: 0,
            span: Span::default(),
        };
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let err = audit_word_quotation_positions(&mk(inl)).unwrap_err();
        assert!(err.contains("the output of `w`"), "locates it: {err}");
        let ord = crate::ast::quotation_type(vec![Type::I64], Vec::new());
        assert!(audit_word_quotation_positions(&mk(ord)).is_ok());
    }

    /// U7 (R18): the exported-signature helper flags a word whose effect
    /// names a private type of its own module, and clears once that type is
    /// exported too (the positive half, R18's own escape hatch).
    #[test]
    fn exported_signature_rule_flags_private_type() {
        use crate::ast::{ModuleInfo, TypedSlot};
        let structs = vec![StructDecl {
            name: "Res".to_string(),
            name_static: "Res",
            fields: vec![("n".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let mk_word = WordDef {
            name: "mk".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: vec![TypedSlot {
                    name: None,
                    ty: Type::Struct(StructId::from_index(0), "Res"),
                }],
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            module: 0,
            span: Span::default(),
        };
        let mut module = Module {
            words: vec![mk_word],
            structs,
            enums: Vec::new(),
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            externs: Vec::new(),
            instantiations: HashMap::new(),
            builtin_overloads: HashMap::new(),
            modules: vec![ModuleInfo {
                imports: HashMap::new(),
                exports: vec![("mk".to_string(), Span::default())],
                selective: HashMap::new(),
            }],
        };

        let err = check_exported_signatures(&module).unwrap_err();
        assert!(err.contains("mk"), "names the word: {err}");
        assert!(err.contains("Res"), "names the private type: {err}");

        module.modules[0]
            .exports
            .push(("Res".to_string(), Span::default()));
        assert!(
            check_exported_signatures(&module).is_ok(),
            "exporting the type clears the rule"
        );
    }

    /// U8 (R20/R21): the selective-import validator rejects a name absent from
    /// its source module's export list (R20), two selective imports of one name
    /// (R21, naming both sources), and a selective name colliding with a local
    /// word (R21), while a clean import passes.
    #[test]
    fn selective_import_collision_is_rejected() {
        use crate::ast::ModuleInfo;

        fn info(exports: &[&str]) -> ModuleInfo {
            ModuleInfo {
                imports: HashMap::new(),
                exports: exports
                    .iter()
                    .map(|n| (n.to_string(), Span::default()))
                    .collect(),
                selective: HashMap::new(),
            }
        }
        fn word(name: &str, module: u32) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect::default(),
                body: WordBody::Terms { terms: Vec::new() },
                poly: None,
                module,
                span: Span::default(),
            }
        }
        fn module_with(words: Vec<WordDef>, modules: Vec<ModuleInfo>) -> Module {
            Module {
                words,
                structs: Vec::new(),
                enums: Vec::new(),
                arrays: Vec::new(),
                owned_cells: Vec::new(),
                refs: Vec::new(),
                externs: Vec::new(),
                instantiations: HashMap::new(),
                builtin_overloads: HashMap::new(),
                modules,
            }
        }
        fn sel(name: &str, qualifier: &str, target: u32, line: u32) -> SelectiveName {
            SelectiveName {
                name: name.to_string(),
                qualifier: qualifier.to_string(),
                target,
                span: Span {
                    line,
                    col: 1,
                    module: 0,
                },
            }
        }

        // R21: modules 1 and 2 each export `p`; module 0 selectively imports it
        // from both, colliding at the second.
        let m = module_with(
            vec![word("p", 1), word("p", 2)],
            vec![info(&[]), info(&["p"]), info(&["p"])],
        );
        let entries = vec![
            vec![sel("p", "a", 1, 1), sel("p", "b", 2, 2)],
            Vec::new(),
            Vec::new(),
        ];
        let err = check_selective_imports(&m, &entries).unwrap_err();
        assert!(err.contains("collides"), "selective collision: {err}");
        assert!(
            err.contains("`a`") && err.contains("`b`"),
            "names both sources: {err}"
        );

        // R20: a name absent from its source's export list is the visibility
        // error, distinct from a collision.
        let m = module_with(vec![word("grow", 1)], vec![info(&[]), info(&[])]);
        let err =
            check_selective_imports(&m, &[vec![sel("grow", "lib", 1, 1)], Vec::new()]).unwrap_err();
        assert!(err.contains("not exported"), "R20 export gate: {err}");
        assert!(!err.contains("collides"), "not the collision error: {err}");

        // R21: a selective name colliding with the importer's own local word.
        let m = module_with(
            vec![word("p", 0), word("p", 1)],
            vec![info(&[]), info(&["p"])],
        );
        let err =
            check_selective_imports(&m, &[vec![sel("p", "lib", 1, 1)], Vec::new()]).unwrap_err();
        assert!(
            err.contains("collides") && err.contains("local"),
            "local collision: {err}"
        );

        // A clean selective import of an exported, non-colliding name passes.
        let m = module_with(vec![word("p", 1)], vec![info(&[]), info(&["p"])]);
        assert!(check_selective_imports(&m, &[vec![sel("p", "lib", 1, 1)], Vec::new()]).is_ok());
    }

    /// U12 (R13): an `[i64 8]` array shape declared in two files interns into
    /// the one shared registry the driver assembles across the closure,
    /// deduping to a single `ArrayId` rather than one per file.
    #[test]
    fn array_shape_dedupes_across_files() {
        use crate::parser::parse_bodies;
        let a = lex(": fa ( [i64 8] -- ) drop ;").unwrap();
        let b = lex(": fb ( [i64 8] -- ) drop ;").unwrap();
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let no_imports = HashMap::new();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        parse_bodies(
            &a,
            &structs,
            &enums,
            0,
            &no_imports,
            &[],
            &no_imports,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .unwrap();
        parse_bodies(
            &b,
            &structs,
            &enums,
            1,
            &no_imports,
            &[],
            &no_imports,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .unwrap();
        assert_eq!(
            arrays.len(),
            1,
            "two files' [i64 8] dedupe to one ArrayId in the shared registry"
        );
    }

    /// U3 (R12): the duplicate-type-name check partitions by owning module, so
    /// two modules each declaring `Point` is not a duplicate, while two `Point`
    /// decls in one module still is (reported by the raw `name_static`, not the
    /// resolver's mangled `name`).
    #[test]
    fn duplicate_type_check_is_per_module() {
        let mk = |module: u32| StructDecl {
            name: format!("Point__m{module}"),
            name_static: "Point",
            fields: Vec::new(),
            span: crate::ast::Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        // Two modules, one `Point` each: not a duplicate.
        assert!(check_duplicate_type_names(&[mk(0), mk(1)], &[]).is_ok());
        // Same module, two `Point`: a duplicate, named by the raw surface name.
        let same_module = vec![
            StructDecl {
                name: "Point".to_string(),
                name_static: "Point",
                fields: Vec::new(),
                span: crate::ast::Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Point".to_string(),
                name_static: "Point",
                fields: Vec::new(),
                span: crate::ast::Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
        ];
        let err = check_duplicate_type_names(&same_module, &[]).unwrap_err();
        assert!(err.contains("duplicate type `Point`"), "raw name: {err}");
    }

    /// Two words of the same name in one module are rejected; the same pair
    /// split across two modules is not (mirrors `duplicate_type_check_is_per_module`).
    #[test]
    fn duplicate_word_name_is_rejected_only_within_one_module() {
        fn word_at(name: &str, module: u32, line: u32) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect::default(),
                body: WordBody::Terms { terms: Vec::new() },
                poly: None,
                module,
                span: Span {
                    line,
                    col: 1,
                    module: 0,
                },
            }
        }
        fn word(name: &str, module: u32) -> WordDef {
            word_at(name, module, 0)
        }

        // Two modules, one `push` each: not a duplicate.
        assert!(check_duplicate_word_names(&[word("push", 0), word("push", 1)]).is_ok());

        // Same module, two `push`: a duplicate, naming both locations.
        let err = check_duplicate_word_names(&[word_at("push", 0, 1), word_at("push", 0, 2)])
            .unwrap_err();
        assert!(
            err.contains("duplicate word `push`") && err.contains("line 2"),
            "names the repeat's location: {err}"
        );
        assert!(
            err.contains("first defined at line 1"),
            "also names the first definition's location: {err}"
        );

        // A repeat `main` in one module is caught too: nothing else validates
        // `main`'s multiplicity within a module.
        let err = check_duplicate_word_names(&[word("main", 0), word("main", 0)]).unwrap_err();
        assert!(err.contains("duplicate word `main`"), "names main: {err}");

        // Two `drop`s sharing a module are *not* rejected here: distinct-struct
        // overloading is `find_drop_overloads`'s job, keyed by struct id, not
        // this check's; re-flagging by name alone would reject Phase 3 slice
        // 8b's legitimate multi-type overloading.
        assert!(check_duplicate_word_names(&[word("drop", 0), word("drop", 0)]).is_ok());
    }

    // A one-field struct with a `drop` overload: linear for the same reason any
    // resource is, used to force the `Copy`-bound failure (X5).
    const SPY: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | s Spy>tag drop ;\n";

    #[test]
    fn quotation_survives_dup_swap_and_bind() {
        // Cu1 (D2/R4): a quotation `Slot` is `Copy`, so a shuffle moves it (and
        // its `quot` marker) verbatim; a bind carries the marker into the
        // `Binding`, from which a local read reconstructs it (the read-back is
        // witnessed end-to-end by `quotation_forwarded_through_bind_still_calls`).
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let arrays: Vec<ArrayDecl> = Vec::new();
        let mut prov = Provenance::default();
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let marker = Some(QuotRef::Known(QuotId(0)));
        let quot = Slot {
            quot: marker,
            ..Slot::computed(Type::Cstr)
        };

        // Every shuffle keeps the marker on the slot it moves.
        for name in ["dup", "swap", "over", "rot"] {
            let mut stack = match name {
                "swap" | "over" => vec![Slot::computed(Type::I64), quot],
                "rot" => vec![Slot::computed(Type::I64), Slot::computed(Type::I64), quot],
                _ => vec![quot],
            };
            let out = check_shuffle(name, span, &mut stack, &ctx, &arrays, &mut prov)
                .unwrap()
                .unwrap();
            assert!(
                out.iter().any(|s| s.quot == marker),
                "`{name}` dropped the quotation marker"
            );
        }

        // A bind carries the marker into the `Binding`.
        let mut scope = Scope::default();
        scope.bind("q", quot, false, &mut prov);
        assert_eq!(scope.local("q").unwrap().quot, marker);
    }

    // --- Slice 8b, D2/D1: the module-visibility primitive and `drop` gate. ---

    /// R1: the primitive is a pure function of `(modules, caller, defining,
    /// name)`; construct `ModuleInfo` directly rather than route through a build.
    #[test]
    fn visibility_own_module_is_visible() {
        let modules = vec![ModuleInfo::default(), ModuleInfo::default()];
        assert!(is_name_visible_to_module(&modules, 1, 1, "Res"));
    }

    #[test]
    fn visibility_selectively_imported_is_visible() {
        let mut caller = ModuleInfo::default();
        caller.selective.insert("Res".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(is_name_visible_to_module(&modules, 1, 0, "Res"));
    }

    #[test]
    fn visibility_qualified_only_import_is_not_visible() {
        // A qualified-only import binds the qualifier but no bare name.
        let mut caller = ModuleInfo::default();
        caller.imports.insert("lib".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(!is_name_visible_to_module(&modules, 1, 0, "Res"));
    }

    #[test]
    fn visibility_unrelated_module_is_not_visible() {
        let modules = vec![
            ModuleInfo::default(),
            ModuleInfo::default(),
            ModuleInfo::default(),
        ];
        assert!(!is_name_visible_to_module(&modules, 1, 2, "Res"));
    }

    fn bare_word(name: &str, module: u32) -> WordDef {
        WordDef {
            name: name.to_string(),
            effect: StackEffect::default(),
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            module,
            span: Span::default(),
        }
    }

    /// R2: `Ctx::Word` carries its word's owning module; `Ctx::Line` denotes 0.
    #[test]
    fn ctx_word_carries_owning_module() {
        let word = bare_word("main", 3);
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = word_ctx(&word, &structs, &enums, None);
        assert_eq!(ctx.module(), 3);
        assert!(ctx.modules().is_none());
    }

    #[test]
    fn ctx_line_is_module_zero() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        assert_eq!(ctx.module(), 0);
        assert!(ctx.modules().is_none());
    }

    /// A `Res` owned by `defining`, mangled as `resolve` would in a multi-module
    /// build (`Res__m{defining}`), so `check_drop_import_visibility`'s demangle
    /// is exercised for real.
    fn res_struct(defining: u32, has_drop_overload: bool) -> StructDecl {
        StructDecl {
            name: format!("Res__m{defining}"),
            name_static: "Res",
            fields: vec![("n".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload,
            is_bundle: false,
            module: defining,
        }
    }

    /// Run `check_shuffle`'s `"drop"` arm on a single `Res` operand under a
    /// caller-module `Ctx::Word` built with `modules`.
    fn drop_res(
        structs: &[StructDecl],
        modules: Option<&[ModuleInfo]>,
        caller: u32,
    ) -> Result<Option<Vec<Slot>>, String> {
        let word = bare_word("main", caller);
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = word_ctx(&word, structs, &enums, modules);
        let arrays: Vec<ArrayDecl> = Vec::new();
        let mut prov = Provenance::default();
        let span = Span {
            line: 2,
            col: 1,
            module: 0,
        };
        let mut stack = vec![Slot::computed(Type::Struct(StructId::from_index(0), "Res"))];
        check_shuffle("drop", span, &mut stack, &ctx, &arrays, &mut prov)
    }

    #[test]
    fn drop_of_locally_declared_override_is_ok() {
        // caller == defining: the override is the caller's own, always visible.
        let structs = vec![res_struct(0, true)];
        let modules = vec![ModuleInfo::default()];
        assert!(drop_res(&structs, Some(&modules), 0).is_ok());
    }

    #[test]
    fn drop_of_selectively_imported_type_is_ok() {
        let structs = vec![res_struct(0, true)];
        let mut caller = ModuleInfo::default();
        caller.selective.insert("Res".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(drop_res(&structs, Some(&modules), 1).is_ok());
    }

    #[test]
    fn drop_of_qualified_only_imported_type_is_error() {
        let structs = vec![res_struct(0, true)];
        let mut caller = ModuleInfo::default();
        caller.imports.insert("lib".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        let err = drop_res(&structs, Some(&modules), 1).unwrap_err();
        // R5: the exact located diagnostic, not merely that it fails.
        assert_eq!(
            err,
            "error: cannot `drop` a value of type `lib::Res` in `main` (line 2)\n  disposing it runs a `drop` destructor declared in module `lib`, which this module has not imported by name\n  note: add `Res` to the import (`import: lib | Res | \"...\"`), or dispose it in a module that declares `Res`"
        );
    }

    #[test]
    fn drop_of_transitively_reachable_type_with_no_direct_import_is_error() {
        // Round-2 fix: `Res` is declared by module 0 (`deep`), reached by the
        // caller (module 2, `main`) only through module 1 (`mid`), which
        // `main` never imports directly. The caller's import map has no
        // qualifier mapping to module 0, so the diagnostic must not fabricate
        // one -- naming the struct's own bare name as if it were a module
        // qualifier (the pre-fix behavior) read as a valid but wrong import
        // spelling.
        let structs = vec![res_struct(0, true)];
        let mid = ModuleInfo::default();
        let mut caller = ModuleInfo::default();
        caller.imports.insert("mid".to_string(), 1);
        let modules = vec![ModuleInfo::default(), mid, caller];
        let err = drop_res(&structs, Some(&modules), 2).unwrap_err();
        assert_eq!(
            err,
            "error: cannot `drop` a value of type `Res` in `main` (line 2)\n  disposing it runs a `drop` destructor declared in a module this module never imports directly -- it is only reachable transitively, through another module's import\n  note: import the module that declares `Res` directly, then add `Res` to that import"
        );
    }

    #[test]
    fn drop_of_plain_struct_no_override_is_ungated() {
        // No override: the gate is never reached, the value disposes structurally.
        let structs = vec![res_struct(0, false)];
        let mut caller = ModuleInfo::default();
        caller.imports.insert("lib".to_string(), 0);
        let modules = vec![ModuleInfo::default(), caller];
        assert!(drop_res(&structs, Some(&modules), 1).is_ok());
    }

    #[test]
    fn check_shuffle_with_no_modules_is_ungated() {
        // R8's contract: with `modules: None` (the REPL path) an override is
        // never gated -- disposing it is byte-for-byte what it was before 8b.
        let structs = vec![res_struct(0, true)];
        assert!(drop_res(&structs, None, 1).is_ok());
    }

    /// D3's leaf resource: one field, a `drop` override implemented exactly
    /// as `examples/resources.sth`'s `Fd` (extracting the field via `Fd>n`
    /// inside `drop`'s own body -- exempted, since a word literally named
    /// `drop` can only be the recognized override for the struct its declared
    /// effect names).
    const FD_DEF: &str = "type: Fd n i64 ;\n: drop ( Fd -- ) | h | h Fd>n drop ;\n";

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
    fn poly_body_destructuring_drop_overloaded_type_is_error() {
        // Bug 2 (round-1 review): `poly_call_term` resolved a generated
        // accessor through the ordinary `env` lookup with no D3 guard at all,
        // so a generic word could destructure any drop-overloaded type and
        // skip its destructor.
        let err = check_src(&format!(
            "{FD_DEF}: sneak ( 'T -- 'T i64 ) 7 Fd Fd>n ;\n: main ( -- ) 1 sneak drop drop ;\n"
        ))
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot destructure `Fd` in `sneak` (line 3): it defines `drop`, so moving its fields out would skip its destructor\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out"
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

    fn capture_binding(name: &str, ty: Type, deriv: Option<DerivId>) -> Binding {
        Binding {
            name: name.to_string(),
            ty,
            aliases: None,
            deriv,
            quot: None,
            surviving: None,
        }
    }

    #[test]
    fn classify_capture_splits_scalar_aggregate_and_borrow_roots() {
        // U-classify (R15): the four-way capture classifier, each arm on its
        // own, since only case 2 and one case-3 direction are reachable from a
        // golden (make-a hits the aggregate arm, the case-3 golden the
        // frame-rooted borrow; the outer-rooted and no-deriv arms ride only on
        // make-b end to end).
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let ref_ty = intern_ref_type(&mut refs, arr_ty, false);
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };

        // Case 1: a scalar local -> Scalar (snapshotted, never dangles).
        let prov = Provenance::default();
        let empty = Scope::default();
        let scalar = capture_binding("x", Type::I64, None);
        assert!(matches!(
            classify_capture(&scalar, &prov, &empty),
            CaptureClass::Scalar
        ));

        // Case 2: a by-value aggregate -> FrameRooted (owned by, dies with,
        // this frame).
        let agg = capture_binding("arr", arr_ty, None);
        assert!(matches!(
            classify_capture(&agg, &prov, &empty),
            CaptureClass::FrameRooted
        ));

        // Case 3a: a borrow whose `owned_root` names a current-frame local ->
        // FrameRooted.
        let mut prov = Provenance::default();
        let d = prov.borrow("arr", false, span);
        let mut framed = Scope::default();
        framed.bound.push(capture_binding("arr", arr_ty, None));
        let borrow_local = capture_binding("r", ref_ty, Some(d));
        assert!(matches!(
            classify_capture(&borrow_local, &prov, &framed),
            CaptureClass::FrameRooted
        ));

        // Case 3b: the same borrow, but its `owned_root` is not in this scope
        // (rooted in an ancestor frame) -> OuterRooted.
        assert!(matches!(
            classify_capture(&borrow_local, &prov, &empty),
            CaptureClass::OuterRooted
        ));

        // Case 3c: a `&T` parameter reborrow carrying no owned root at all ->
        // OuterRooted by construction, without consulting the scope.
        let mut prov = Provenance::default();
        let d = prov.add(Deriv {
            place: "p".to_string(),
            owned_root: None,
            reborrow: true,
            mutable: false,
            projected: false,
            span,
        });
        let param_ref = capture_binding("r", ref_ty, Some(d));
        assert!(matches!(
            classify_capture(&param_ref, &prov, &empty),
            CaptureClass::OuterRooted
        ));
    }

    #[test]
    fn check_capture_admission_gates_each_capture_kind() {
        // U-admit (R15): the admission gate around `classify_capture`. Each
        // deferral/rejection is its own row, since a dropped guard is a silent
        // accept the well-typed goldens never trip.
        fn prov_with(names: &[&str]) -> Provenance {
            let mut prov = Provenance::default();
            prov.quotation_captures
                .push(names.iter().map(|s| s.to_string()).collect());
            prov
        }
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let admit = |prov: &mut Provenance, escaping, scope: &Scope| {
            check_capture_admission(QuotId(0), escaping, span, &ctx, prov, scope)
        };

        // No real capture: an empty set, and a free global not in scope, both
        // admit (a free word resolves at the call and needs no env).
        assert!(admit(&mut prov_with(&[]), true, &Scope::default()).is_ok());
        assert!(admit(&mut prov_with(&["some-word"]), true, &Scope::default()).is_ok());

        // A single scalar capture admits at every boundary and, being a
        // snapshot, contributes no surviving-set member (R19 / D4 amendment).
        let mut scope = Scope::default();
        scope.bound.push(capture_binding("x", Type::I64, None));
        assert_eq!(admit(&mut prov_with(&["x"]), true, &scope), Ok(None));

        // A frame-rooted aggregate is past-owning-frame when escaping (R24),
        // and admitted in-frame (R21) with a frame-rooted surviving member.
        let mut scope = Scope::default();
        scope.bound.push(capture_binding("arr", arr_ty, None));
        let escaping = admit(&mut prov_with(&["arr"]), true, &scope).unwrap_err();
        assert!(
            escaping.contains("`arr`") && escaping.contains("does not survive the return"),
            "escaping frame-rooted capture is past-owning-frame: {escaping}"
        );
        let mut prov = prov_with(&["arr"]);
        let set = admit(&mut prov, false, &scope)
            .expect("in-frame frame capture admits (R21)")
            .expect("an aggregate capture is a surviving-set member");
        let members = prov.surviving_set(set);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "arr");
        assert!(
            members[0].frame_rooted,
            "an in-frame aggregate is frame-rooted"
        );

        // Case 4: a quotation-typed name is deferred at every boundary.
        let mut scope = Scope::default();
        scope.bound.push(Binding {
            name: "q".to_string(),
            ty: Type::I64,
            aliases: None,
            deriv: None,
            quot: Some(QuotRef::Known(QuotId(0))),
            surviving: None,
        });
        let quot_name = admit(&mut prov_with(&["q"]), true, &scope).unwrap_err();
        assert!(
            quot_name.contains("capturing a quotation value by name is deferred"),
            "a captured quotation-typed name is deferred: {quot_name}"
        );

        // Two scalar captures escaping need a heap env, deferred (R18); the
        // same two admit in-frame as a stack bundle (R16, R21).
        let mut scope = Scope::default();
        scope.bound.push(capture_binding("x", Type::I64, None));
        scope.bound.push(capture_binding("y", Type::I64, None));
        let multi = admit(&mut prov_with(&["x", "y"]), true, &scope).unwrap_err();
        assert!(
            multi.contains("at most one reference"),
            "a 2+-capture escaping closure is deferred: {multi}"
        );
        // In-frame, the same two admit -- but as a stack bundle (R16), so the
        // interned set has no members yet must survive with `bundle = true`,
        // else the bundle-escape-via-carrier signal is lost before R22.
        let mut prov = prov_with(&["x", "y"]);
        let set = admit(&mut prov, false, &scope)
            .expect("two scalar captures admit in-frame (R21)")
            .expect("an all-scalar stack bundle keeps a set to carry the bundle signal");
        assert!(
            prov.surviving_set(set).is_empty(),
            "a scalar snapshot is never a surviving member (D4)"
        );
        assert!(
            prov.surviving_set_is_bundle(set),
            "the all-scalar 2-capture stack bundle marks its set as a bundle"
        );
    }

    /// Slice 10a (R2): the fifth materialization boundary, pinned directly.
    /// A poly signature cannot place an ordinary quotation anywhere but a
    /// direct top-level parameter (`reject_poly_quotation_anywhere`), so a
    /// `~` local can never reach the escaping-closure output/store boundaries
    /// through a real `.sth` program in this slice; the guard is exercised
    /// directly instead, the same way phase 1 pinned the routing predicates.
    #[test]
    fn check_capture_admission_rejects_captured_inline_quotation() {
        let mut scope = Scope::default();
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], vec![Type::I64]);
        scope.bound.push(Binding {
            name: "f".to_string(),
            ty: inl,
            aliases: None,
            deriv: None,
            quot: None,
            surviving: None,
        });
        let mut prov = Provenance::default();
        prov.quotation_captures
            .push(std::iter::once("f".to_string()).collect());
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let err = check_capture_admission(QuotId(0), true, span, &ctx, &mut prov, &scope)
            .expect_err("a captured `~` local must be rejected");
        assert!(
            err.contains("`~`") && err.contains("captured"),
            "a captured `~` local should be its own located rejection, not the ordinary \
             quotation deferral, got: {err}"
        );

        // The same rejection under `Ctx::Word` must name the enclosing word,
        // not fall back to `<line>` -- the only other call site exercises
        // `Ctx::Line`, which can't discriminate a discarded `Ctx` parameter
        // from a used one.
        let word = bare_word("outer", 0);
        let word_ctx = word_ctx(&word, &structs, &enums, None);
        let word_err = check_capture_admission(QuotId(0), true, span, &word_ctx, &mut prov, &scope)
            .expect_err("a captured `~` local must be rejected");
        assert!(
            word_err.contains("`outer`"),
            "a captured `~` local under `Ctx::Word` should name the enclosing word: {word_err}"
        );
    }

    #[test]
    fn times_typing_obligations() {
        // R18u: the three `times` typing obligations, each its own row, since a
        // missed guard is a silent accept (the well-typed witness never trips
        // them). Move-state identity, the whole-row guard, and row-effect
        // equality.

        // A well-typed `times` accepts (the body consumes the index and returns
        // the row unchanged, touching no linear local).
        check_src(": main ( -- ) 0 10 [ + ] times . ;\n").unwrap();

        // (1) Move-state identity: consuming an outer linear local is rejected,
        // named, with the repeated-disposal reason.
        let consume = check_src(&format!(
            "{SPY}: main ( -- ) 5 Spy | s | 0 10 [ | i | i s drop + ] times . ;\n"
        ))
        .expect_err("consuming a linear local should be rejected");
        assert!(
            consume.contains("a `times` body cannot consume `s`")
                && consume.contains("the body runs more than once"),
            "move-state identity should name `s`, got: {consume}"
        );

        // (2) Whole-row guard: a quotation anywhere in the row, not just the
        // consumed top, is rejected.
        let row_quot = check_src(": main ( -- ) [ + ] 3 [ drop ] times ;\n")
            .expect_err("a quotation in the row should be rejected");
        assert!(
            row_quot.contains("`times`")
                && row_quot.contains("cannot take a quotation as an operand"),
            "whole-row guard should reject a row quotation, got: {row_quot}"
        );

        // (3) Row-effect equality: a body that changes the row's depth is
        // rejected.
        let row_effect = check_src(": main ( -- ) 0 10 [ + 1 ] times . ;\n")
            .expect_err("a body that changes the row should be rejected");
        assert!(
            row_effect.contains("`times` body must leave the row unchanged"),
            "row-effect equality should reject a changed row, got: {row_effect}"
        );
    }

    #[test]
    fn merged_quotations_are_rejected_at_the_join() {
        // Cu2 (R7): two *different* quotations merged at an `if` join are
        // rejected at the join (not at consumption), because `lower_if` would
        // otherwise build a `Phi` over two phantoms. The *same* `Known` id in
        // both arms (one literal bound before the `if`, read in each) is safe:
        // `lower_if`'s `t == e` fast path emits no `Phi`, so it must not error.
        let different = check_src(": main ( -- ) true if [ 1 + ] else [ 1 - ] end drop ;\n")
            .expect_err("two different quotations at a join should be rejected");
        assert!(
            different.contains("these two branches leave different quotations"),
            "the join guard should fire, got: {different}"
        );
        check_src(": main ( -- ) [ + ] | q | true if q else q end drop ;\n")
            .expect("the same `Known` id in both arms is safe and must not error");
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
    fn check_poly_call_rejects_a_quotation_argument() {
        // R9p: `check_poly_call` reads only `stack[base + i].ty`, so a quotation
        // does not *fail* unification, it *succeeds* binding `'T` to the
        // placeholder and monomorphizes a real call over a phantom. The guard
        // before `unify_poly_input` is what makes the R9 rejection reachable.
        let err = check_src(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) [ + ] dupit drop drop ;\n",
        )
        .expect_err("a quotation passed to a polymorphic word should be rejected");
        assert!(
            err.contains("a quotation cannot be passed to `dupit`"),
            "check_poly_call should name `dupit`, got: {err}"
        );
    }

    #[test]
    fn poly_term_rejects_a_quotation_literal() {
        // R5p: a quotation literal in a polymorphic body is rejected eagerly at
        // the literal (the polymorphic path cannot yet carry the marker).
        let err = check_src(
            ": bad ( 'T: Copy -- 'T ) [ + ] drop ;\n\
             : main ( -- ) 1 bad . ;\n",
        )
        .expect_err("a quotation literal in a polymorphic body should be rejected");
        assert!(
            err.contains("a quotation in the polymorphic body of `bad`")
                && err.contains("not yet supported"),
            "poly_term should name `bad`, got: {err}"
        );
    }

    #[test]
    fn poly_term_rejects_an_array_constructor() {
        // Slice 6h: an array constructor in a polymorphic body is rejected
        // eagerly, mirroring the quotation rejection above (no interning
        // route exists for a body-internal shape absent from the signature).
        let err = check_src(
            ": bad ( 'T: Copy -- 'T ) [ i64 ; 4 ] drop ;\n\
             : main ( -- ) 1 bad . ;\n",
        )
        .expect_err("an array constructor in a polymorphic body should be rejected");
        assert!(
            err.contains("an array constructor in the polymorphic body of `bad`")
                && err.contains("not yet supported"),
            "poly_term should name `bad`, got: {err}"
        );
    }

    #[test]
    fn quotation_as_operand_is_rejected_at_every_audited_site() {
        // R11t: the audit is a *test artifact*, not prose. A missed guard on the
        // `Cstr` placeholder is a silent accept (R4), so every default-deny site
        // gets a row here: a new consumer added later without a guard turns one
        // row from `Err` to `Ok` and fails the test. The one `is_line` row is the
        // REPL residual, checked through `infer_line` rather than `check`.
        //
        // Each row asserts TWO substrings, and this is load-bearing. `site` is
        // the token the message names (the op, or the word for the argument
        // family); `phrase` is text only the quotation rejection produces. The
        // pre-existing generic diagnostics (`operand_pair_mismatch`,
        // `type_mismatch`, `array_word_operand`, `reference_word_operand`,
        // `fill_count_not_literal`, ...) all print the op in backticks too, so a
        // `site`-only row stays green when its guard is removed and the fallback
        // fires: it names the same op. Requiring `phrase` as well is what turns a
        // removed guard from green to red. Every operand-family row shares the
        // one `reject_quotation_operand` phrase; the store/argument/output/
        // residual families carry their own wording no generic diagnostic emits.
        //
        // FIX 2 (verified, no row): the only `check_operator` op that would
        // accept a `Cstr` operand if its guard were removed is `.` (print, whose
        // printable set includes `Str`/`Cstr`), and it already has the `.` row.
        // Every comparison (`=`/`<`/`>`/...), like every arithmetic/bitwise/
        // shift op, requires `is_numeric`/`is_int`/`is_float` and rejects a
        // `cstr` outright, so there is no silent-accept comparison path to row.
        struct Row {
            source: &'static str,
            site: &'static str,
            phrase: &'static str,
            is_line: bool,
        }
        const OPERAND: &str = "cannot take a quotation as an operand";
        // Operand-family row: `site` is the op, `phrase` is the shared wording.
        let op = |source, site| Row {
            source,
            site,
            phrase: OPERAND,
            is_line: false,
        };
        // Any other family: spell both substrings out.
        let w = |source, site, phrase| Row {
            source,
            site,
            phrase,
            is_line: false,
        };
        let rows = [
            // check_operator, both operand positions, plus print.
            op(": main ( -- ) 1 [ + ] + ;\n", "`+`"),
            op(": main ( -- ) [ + ] 1 - . ;\n", "`-`"),
            op(": main ( -- ) [ + ] . ;\n", "`.`"),
            // the `if` condition, before the `bool` mismatch.
            op(": main ( -- ) [ + ] if 1 . else 2 . end ;\n", "`if`"),
            // check_str_word (`len`/`cstr`).
            op(": main ( -- ) [ + ] len ;\n", "`len`"),
            op(": main ( -- ) [ + ] cstr ;\n", "`cstr`"),
            // check_array_word: the `fill` count operand and the stored element.
            op(": main ( -- ) 5 [ + ] fill ;\n", "`fill`"),
            w(
                ": main ( -- ) [ + ] 8 fill drop ;\n",
                "a quotation cannot be stored",
                "escaping quotations are slice 7",
            ),
            // check_array_index, reached through the `&>` reference word.
            op(
                "type: V x i64 ;\n: main ( -- ) 1 2 V | v | &v &V>x [ + ] &> drop drop ;\n",
                "`&>`",
            ),
            // check_owned_cell_word.
            op(": main ( -- ) [ + ] ^ ;\n", "`^`"),
            // check_reference_word's `&q` prefix-borrow-of-a-local form.
            op(": main ( -- ) [ + ] | q | &q drop ;\n", "`&q`"),
            // check_struct_peek_word and check_struct_get_word (an aggregate
            // field, so the getter is intercepted here, not by the env loop).
            op("type: V x i64 ;\n: main ( -- ) [ + ] V|>x ;\n", "`V|>x`"),
            op(
                "type: Inner a i64 ;\ntype: Outer b Inner ;\n: main ( -- ) [ + ] Outer>b ;\n",
                "`Outer>b`",
            ),
            // check_access_word's store paths: the value and the receiver.
            w(
                "type: Box s cstr ;\n: main ( -- ) \"hi\" cstr Box | b | &!b &!Box>s [ + ] ! b drop ;\n",
                "a quotation cannot be stored",
                "escaping quotations are slice 7",
            ),
            op(": main ( -- ) [ + ] 1 ! ;\n", "`!`"),
            // the env argument loop and check_poly_call's input loop (R9/R9p).
            w(
                ": foo ( i64 -- i64 ) ;\n: main ( -- ) [ + ] foo drop ;\n",
                "passed to `foo`",
                "only `call` and `times` accept one",
            ),
            w(
                ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n: main ( -- ) [ + ] dupit drop drop ;\n",
                "passed to `dupit`",
                "only `call` and `times` accept one",
            ),
            // check_outputs (R10) and the `times` body-output row (blocker 2).
            w(
                ": f ( -- i64 ) [ + ] ;\n",
                "declared output",
                "leaves a quotation on the stack",
            ),
            op(
                ": main ( -- ) \"x\" cstr 0 [ drop drop [ + ] ] times drop ;\n",
                "`times`",
            ),
            // the REPL residual (R19), checked through `infer_line`.
            Row {
                source: "1 [ + ]",
                site: "end of a line",
                phrase: "a quotation cannot be left on the stack",
                is_line: true,
            },
        ];
        for Row {
            source,
            site,
            phrase,
            is_line,
        } in rows
        {
            let err = match is_line {
                true => infer_src(source, &[])
                    .expect_err("an audited site must reject a quotation, not silently accept it"),
                false => check_src(source)
                    .expect_err("an audited site must reject a quotation, not silently accept it"),
            };
            assert!(
                err.contains(site),
                "audited site `{site}` was not named, got: {err}"
            );
            assert!(
                err.contains(phrase),
                "audited site `{site}` did not produce its quotation-rejection phrase `{phrase}`, got: {err}"
            );
        }
    }

    #[test]
    fn check_poly_copy_word_accepts_and_instantiates() {
        // R1/R4–R7: a `'T: Copy` word `dup`s its variable and is called at a
        // concrete `Copy` type; the body and the instantiation both check.
        check_src(": dupit ( 'T: Copy -- 'T 'T ) dup ;\n: main ( -- ) 5 dupit drop drop ;")
            .unwrap();
    }

    #[test]
    fn check_poly_word_records_one_instantiation_per_concrete_shape() {
        // R8/R14: each distinct ground θ is recorded once, keyed by call span.
        let module = checked_module(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit drop drop true dupit drop drop ;",
        );
        // Two call sites, two distinct θ (i64 and bool): two instantiations.
        let symbols: std::collections::HashSet<&str> = module
            .instantiations
            .values()
            .map(|c| c.symbol.as_str())
            .collect();
        assert_eq!(module.instantiations.len(), 2);
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn check_poly_ord_word_accepts_comparison_body() {
        // R7: a `'T: Ord` variable may be compared; the body and a numeric
        // instantiation both check.
        check_src(": less ( 'T: Ord 'T -- bool ) > ;\n: main ( -- ) 3 4 less drop ;").unwrap();
    }

    #[test]
    fn check_poly_length_word_accepts_and_monomorphizes_len() {
        // R1/R5/R9: a length variable is opaque through `len`; the same word
        // instantiates at `[i64 4]` and `[i64 8]`.
        check_src(
            ": alen ( [i64 'N] -- [i64 'N] usize ) len ;\n\
             : main ( -- ) 5 4 fill alen . drop 5 8 fill alen . drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_poly_row_word_accepts_and_expands_outputs() {
        // R1/R5/R7: a row-variable word passes its deeper stack through
        // untouched and duplicates the two `Copy` variables; the resolved
        // instantiation has four concrete outputs, so it interns a bundle.
        let module = checked_module(
            ": dup2 ( ..s 'a: Copy 'b: Copy -- ..s 'a 'b 'a 'b ) over over ;\n\
             : main ( -- ) 1 2 dup2 . . . . ;",
        );
        assert_eq!(module.instantiations.len(), 1);
        let inst = module.instantiations.values().next().unwrap();
        assert_eq!(inst.out_arity, 4);
        assert!(inst.bundle.is_some());
    }

    #[test]
    fn check_x4_type_variable_forced_to_two_concretes_names_both() {
        // X4: one `'T` unified to both `i64` and `bool` at one call site names
        // both concrete types.
        let err = check_src(": pairwise ( 'T 'T -- ) drop drop ;\n: main ( -- ) 1 true pairwise ;")
            .unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("i64"), "unexpected message: {err}");
        assert!(err.contains("bool"), "unexpected message: {err}");
    }

    #[test]
    fn check_x5_copy_bound_on_linear_type_names_variable_type_and_reason() {
        // X5: instantiating a `'T: Copy` word with a linear type is a located
        // call-site error naming the variable, the type, and the linear reason.
        let src = format!("{SPY}: idc ( 'T: Copy -- 'T ) ;\n: main ( -- ) 0 Spy idc drop ;");
        let err = check_src(&src).unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Spy"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_x6_ord_bound_on_non_ord_type_is_error() {
        // X6: instantiating a `'T: Ord` requirement with a non-`Ord` type is a
        // located error.
        let err =
            check_src(": less ( 'T: Ord 'T -- bool ) > ;\n: main ( -- ) true false less drop ;")
                .unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Ord"), "unexpected message: {err}");
    }

    #[test]
    fn check_x7_dup_of_unbounded_variable_names_missing_copy_bound() {
        // X7: `dup` of an unbounded `'T` inside a body names the variable and
        // the missing `Copy` bound.
        let err = check_src(": bad ( 'T -- 'T 'T ) dup ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Copy"), "unexpected message: {err}");
    }

    #[test]
    fn check_x8_compare_of_unbounded_variable_requires_ord() {
        // X8: `>` on an unbounded `'T` inside a body requires an `Ord` bound.
        let err = check_src(": bad ( 'T 'T -- bool ) > ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Ord"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_local_bound_and_never_read_is_unconsumed_error() {
        // A `'T` bound to a local and never read leaks: the polymorphic body
        // checker rejects it exactly as the monomorphic sibling rejects
        // `( ^i64 -- ) | x | ;`, naming the variable.
        let err = check_src(": leaky ( 'T -- ) | x | ;\n: main ( -- ) ;").unwrap_err();
        assert!(
            err.contains("linear value `x` is never consumed"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_poly_local_read_twice_is_use_after_move() {
        // Reading a non-`Copy` local a second time is use-after-move: the
        // polymorphic checker rejects it as the monomorphic sibling rejects
        // `( ^i64 -- ^i64 ^i64 ) | x | x x ;`, naming the variable.
        let err = check_src(": twice ( 'T -- 'T 'T ) | x | x x ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("local `x`"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_local_rebound_while_in_scope_is_error() {
        // R4 twin of the monomorphic rebinding rejection: a second `| x |`
        // while `x` is still in scope would orphan the first binding, leaking
        // the non-`Copy` value parked in it. Reject at compile time, naming the
        // variable, exactly as `( ^i64 ^i64 -- ^i64 ) | x | | x | x ;` is.
        let err =
            check_src(": shadow ( 'T 'T -- 'T ) | x | | x | x ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("already bound"), "unexpected message: {err}");
        assert!(err.contains('x'), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_duplicate_local_in_bind_group_is_error() {
        // A name repeated inside one bind group (`| x x |`) orphans the first
        // binding before the cross-group rebind guard can see it: the poly
        // checker rejects it as the monomorphic sibling rejects
        // `( ^i64 ^i64 -- ^i64 ) | x x | x ;`, naming the variable.
        let err = check_src(": bad ( 'T 'T -- 'T ) | x x | x ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("duplicate local"), "unexpected message: {err}");
        assert!(err.contains('x'), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_local_named_after_variant_is_error() {
        // A local named after a registered variant would make the clause-vs-
        // locals `|` disambiguation ambiguous: the poly binder rejects it as
        // the monomorphic sibling `( i64 i64 -- i64 )` of the same body does,
        // naming the collision.
        let err = check_src(
            "type: Maybe | None | Some v i64 ;\n: f ( 'T i64 -- 'T ) drop | Some | Some ;\n: main ( -- ) 1 2 f drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("collides with the variant name `Some`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_poly_body_with_if_accepts_choose() {
        // T1: a polymorphic body may branch. `choose` consumes `a` and `b` on
        // both arms but at different sites; the move-join must recognise
        // `Moved`+`Moved` as consumed-once (not a leak), or `choose` would be
        // wrongly rejected at the word end (M1).
        assert!(
            check_src(
                ": choose ( 'T 'T bool -- 'T ) | a b flag | flag if a b drop else b a drop end ;\n: main ( -- ) 1 2 true choose drop ;",
            )
            .is_ok(),
            "choose should type-check"
        );
    }

    #[test]
    fn check_poly_arm_local_unconsumed_is_error() {
        // T2: `y` is bound inside the `then` arm and never consumed in it;
        // `leave_arm` must catch the arm-local leak (M2).
        let err = check_src(
            ": arm_leak ( 'T 'T bool -- 'T ) | a b flag | flag if a b | y | else a drop b end ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(err.contains('y'), "names the arm-local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_if_moved_on_both_arms_is_accepted() {
        // T3: `a`/`b` consumed on both arms (`Moved`+`Moved` => `Moved`), so
        // nothing leaks at the word end.
        assert!(
            check_src(
                ": both ( 'T 'T bool -- ) | a b flag | flag if a drop b drop else b drop a drop end ;\n: main ( -- ) ;",
            )
            .is_ok(),
            "both should type-check"
        );
    }

    #[test]
    fn check_poly_if_moved_on_one_arm_leaks() {
        // T4: `x` consumed on the `then` arm only (`Moved`+`Live` =>
        // `MaybeMoved`), which the leak check must count as still-unconsumed
        // (M3).
        let err =
            check_src(": one ( 'T bool -- ) | x flag | flag if x drop else end ;\n: main ( -- ) ;")
                .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_if_moved_on_neither_arm_leaks() {
        // T5: `x` untouched on both arms (`Live`+`Live` => `Live`); a value
        // parked in a local across an `if` still leaks at the word end (M4).
        let err = check_src(": none ( 'T bool -- ) | x flag | flag if else end ;\n: main ( -- ) ;")
            .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_if_condition_not_bool_is_error() {
        // T6: the `if` condition must be `bool`; here the popped condition is
        // the type variable `'T`, so the condition guard fires before anything
        // else (an output-mismatch never mentions `if`).
        let err = check_src(": bad ( 'T 'T -- 'T ) if drop else drop end ;\n: main ( -- ) ;")
            .unwrap_err();
        assert!(err.contains("if"), "names the `if`: {err}");
        assert!(err.contains("'T"), "names the variable condition: {err}");
    }

    #[test]
    fn check_poly_if_branch_depth_mismatch_is_error() {
        // T7: the arms leave different stack depths (then: 1, else: 2). `'T`
        // carries a `Copy` bound so the repeated reads are not use-after-move,
        // leaving the depth mismatch as the sole failure this test proves.
        let err = check_src(
            ": bad ( 'T: Copy bool -- 'T ) | x flag | flag if x else x x end ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(
            err.contains("different stack depths"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_poly_if_use_after_join_is_error() {
        // T8: both arms consume `x` (the join is `Moved`), so the `x drop`
        // after `end` is a second read: use-after-move, not a leak.
        let err = check_src(
            ": bad ( 'T bool -- ) | x flag | flag if x drop else x drop end x drop ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains('x'), "names the moved local: {err}");
    }

    #[test]
    fn check_poly_dup_of_variable_element_array_names_type_variable() {
        // R7/`poly_copy_gate` array arm: `dup` of an array whose element is an
        // unbounded `'T` recurses to the element and names the variable, not a
        // fabricated `i64`.
        let err =
            check_src(": bad ( ['T 'N] -- ['T 'N] ['T 'N] ) dup ;\n: main ( -- ) ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Copy"), "unexpected message: {err}");
    }

    #[test]
    fn check_poly_dup_of_linear_element_array_names_element_type() {
        // `poly_copy_gate` array arm: `dup` of a length-variable array whose
        // element is a concrete linear struct names that struct, never `i64`.
        let err = check_src(&format!(
            "{SPY}: bad ( [Spy 'N] -- [Spy 'N] [Spy 'N] ) dup ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
        assert!(err.contains("Spy"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }

    #[test]
    fn check_two_output_word_interns_its_return_bundle() {
        // R8/R10: a word with two outputs gets a bundle struct in the same
        // registry the layout pass reads, flagged as a bundle and carrying the
        // output tuple in order (deepest output first).
        let module = checked_module(": pair ( -- i64 bool ) 1 true ; : main ( -- ) ;");
        let bundles: Vec<&StructDecl> = module.structs.iter().filter(|d| d.is_bundle).collect();
        assert_eq!(bundles.len(), 1);
        assert_eq!(
            bundles[0]
                .fields
                .iter()
                .map(|(_, ty)| *ty)
                .collect::<Vec<Type>>(),
            vec![Type::I64, Type::BOOL]
        );
    }

    #[test]
    fn check_one_output_word_interns_no_bundle() {
        // R2: nothing changes for a word the aggregate ABI does not apply to.
        let module = checked_module(": inc ( i64 -- i64 ) 1 + ; : main ( -- ) ;");
        assert!(module.structs.iter().all(|d| !d.is_bundle));
    }

    #[test]
    fn check_two_words_of_one_output_shape_share_one_bundle() {
        // R8: interning dedups structurally on the output tuple, so two words
        // of the same shape share a bundle and a differing shape gets its own.
        let module = checked_module(
            ": pair ( i64 -- i64 i64 ) dup ;\n\
             : twice ( i64 -- i64 i64 ) dup ;\n\
             : flags ( -- i64 bool ) 1 true ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(module.structs.iter().filter(|d| d.is_bundle).count(), 2);
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
    fn check_extern_redeclaring_a_word_is_error() {
        // Criterion 5/R1: an `extern:` naming an already-registered word (a
        // user `:` word here) is a located error.
        let src = ": foo ( i64 -- i64 ) ;\nextern: foo ( i64 -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("foo"), "unexpected message: {err}");
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_redeclaring_a_builtin_is_error() {
        // Criterion 5/R1: every builtin `check_term` dispatches by name before
        // the env lookup, plus the `>`-prefixed conversion family. None is in
        // `builtin_table`, so without the `BUILTIN_WORDS` gate the declaration
        // would be accepted, never consulted, and silently do nothing.
        for name in BUILTIN_WORDS.iter().copied().chain([">u8", ">f64"]) {
            let src = format!("extern: {name} ( i64 -- i64 ) \"s\" ;");
            let Err(err) = check_src(&src) else {
                panic!("`extern: {name}` was accepted");
            };
            assert!(
                err.contains("redeclares"),
                "unexpected message for `{name}`: {err}"
            );
        }
    }

    #[test]
    fn overload_exact_input_match_is_error() {
        // R1: two definitions with identical `(module, name, input_types)`
        // still hit the `duplicate word` message, byte-for-byte.
        let src = "type: Vec2 x i64 y i64 ;\n\
: dist ( Vec2 Vec2 -- i64 ) drop drop 0 ;\n\
: dist ( Vec2 Vec2 -- i64 ) drop drop 1 ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("duplicate word `dist`"),
            "unexpected message: {err}"
        );

        // R1: an overload whose input types exactly match a builtin row is a
        // located error too, naming the operand types.
        let src = "type: Vec2 x i64 y i64 ;\n\
: + ( i64 i64 -- i64 ) drop ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("overload of `+`") && err.contains("as a builtin"),
            "unexpected message: {err}"
        );
        assert!(err.contains("i64 i64"), "names the operand types: {err}");

        // Mutation check: two overloads of one name with *different* input
        // types no longer collide as a duplicate (the whole reason for R1's
        // widened key).
        let src = "type: Vec2 x i64 y i64 ;\n\
: dist ( Vec2 Vec2 -- i64 ) drop drop 0 ;\n\
: dist ( Vec2 -- i64 ) drop 0 ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            !err.contains("duplicate word"),
            "different input types must not collide as a duplicate: {err}"
        );
    }

    #[test]
    fn overload_arity_clash_is_error() {
        // R4: a local overload of `+` whose arity disagrees with the
        // builtin's is rejected at its own definition site.
        let src = "type: Vec2 x i64 y i64 ;\n\
: + ( Vec2 -- Vec2 ) ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("overload of `+`")
                && err.contains("takes 1 input but another `+` takes 2")
                && err.contains("must agree on input count"),
            "unexpected message: {err}"
        );

        // R4: two local overloads of a non-builtin name disagreeing on
        // arity, rejected at the second's site.
        let src = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( Vec2 Vec2 -- Vec2 ) drop ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("overload of `bump`") && err.contains("takes 2 input"),
            "unexpected message: {err}"
        );

        // Mutation check: two overloads agreeing on arity (even with
        // different input types) never hit this check.
        let ok = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( i64 -- i64 ) ;\n\
: main ( -- ) ;\n";
        check_src(ok).expect("same-arity overloads must not trip the arity check");
    }

    #[test]
    fn overload_generic_and_concrete_overlap_is_error() {
        // R5: a poly candidate overlapping the builtin `+` of the same
        // arity is rejected -- no specialization ordering.
        let src = ": + ( 'T 'T -- 'T ) drop ;\n: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("generic overload") && err.contains("overlaps a concrete overload of `+`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains(": + ( 'T 'T -- 'T )"),
            "renders the poly signature: {err}"
        );

        // R5: a poly candidate overlapping a *local* concrete overload of
        // the same name and arity.
        let src = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( 'T -- 'T ) ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("generic overload")
                && err.contains("overlaps a concrete overload of `bump`"),
            "unexpected message: {err}"
        );

        // Mutation check: a poly candidate of a *different* arity than
        // every concrete candidate for the name never trips the check.
        let ok = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( 'T 'T -- 'T ) drop ;\n\
: main ( -- ) ;\n";
        check_src(ok).expect("a differing-arity poly candidate must not trip the overlap check");
    }

    /// Fix 3 (R5, module-scoped): `check_generic_concrete_overlap` operates
    /// directly on `WordDef`s carrying pre-mangle bare names and hand-set
    /// module ids -- unlike a `check_src` scenario (always module 0), this
    /// exercises the cross-module key `resolve::mangle` would otherwise
    /// disambiguate before `check` ever ran on a real multi-file program, so
    /// the two candidates' bare names can actually collide by string here.
    #[test]
    fn overload_generic_and_concrete_overlap_is_module_scoped() {
        fn concrete_word(name: &str, module: u32, arity: usize) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect {
                    inputs: (0..arity)
                        .map(|_| TypedSlot {
                            name: None,
                            ty: Type::I64,
                        })
                        .collect(),
                    outputs: Vec::new(),
                },
                body: WordBody::Terms { terms: Vec::new() },
                poly: None,
                module,
                span: Span::default(),
            }
        }
        fn poly_word(name: &str, module: u32, arity: usize) -> WordDef {
            let sig = PolySig {
                row_in: None,
                inputs: (0..arity as u32).map(PolyType::Var).collect(),
                outputs: Vec::new(),
                row_out: None,
                bounds: Vec::new(),
                ty_var_names: (0..arity).map(|i| format!("'T{i}")).collect(),
                len_var_names: Vec::new(),
                row_var_names: Vec::new(),
            };
            WordDef {
                name: name.to_string(),
                effect: StackEffect {
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
                body: WordBody::Terms { terms: Vec::new() },
                poly: Some(Box::new(sig)),
                module,
                span: Span::default(),
            }
        }

        // R5, module-scoped: an unrelated concrete `bump` in module 1 does
        // not overlap a poly `bump` of the same arity declared in module 0 --
        // pre-fix, both were keyed by the bare name alone, globally, so this
        // combination was rejected even though nothing imports across the
        // two modules.
        let words = vec![concrete_word("bump", 1, 1), poly_word("bump", 0, 1)];
        check_generic_concrete_overlap(&words)
            .expect("an unrelated same-name concrete word in a different module must not overlap");

        // Mutation check: the *same*-module case must still be rejected --
        // module-scoping narrows the key, it does not disable the check.
        let words = vec![concrete_word("bump", 0, 1), poly_word("bump", 0, 1)];
        let err = check_generic_concrete_overlap(&words).unwrap_err();
        assert!(
            err.contains("generic overload")
                && err.contains("overlaps a concrete overload of `bump`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn overload_missing_at_call_site_is_error() {
        // R3: a builtin operator's exact-match miss falls back to its
        // existing operand-class diagnostic, byte-for-byte, even when a
        // *different* struct's overload of the same name exists in the
        // module (importing `Vec2` does not bring `+` for it, and a local
        // `Vec2 +` overload does not answer an `i64 bool` call site
        // either).
        let src = "type: Vec2 x i64 y i64 ;\n\
: + ( Vec2 Vec2 -- Vec2 ) drop ;\n\
: main ( -- ) 1 true + drop ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("requires two operands of the same numeric type")
                && err.contains("`i64`")
                && err.contains("`bool`"),
            "unexpected message: {err}"
        );

        // R3: a user-overloaded, non-operator name called with operands
        // that match no candidate names the operand types, the same as any
        // ordinary word call.
        let src = "type: Vec2 x i64 y i64 ;\n\
: describe ( Vec2 -- ) drop ;\n\
: main ( -- ) 1 describe ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("expected `Vec2`") && err.contains("found `i64`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_shadowing_a_builtin_does_not_change_its_meaning() {
        // R1's reason for existing: before the gate, this compiled, and `dup`
        // at the call site still meant the builtin with no diagnostic at all.
        let src = "extern: dup ( i64 -- i64 ) \"mydup\" ;\n: main ( -- ) 1 dup . . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_registers_its_effect_at_call_sites() {
        // Criterion 4/R1: registration is what makes the existing arity and
        // type checks apply to a foreign call unchanged. Parsing it is not
        // enough, so assert the effect is actually consulted.
        let ok =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) \"hi\" cstr strlen . ;";
        check_src(ok).unwrap();
        let underflow = "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) strlen . ;";
        let err = check_src(underflow).unwrap_err();
        assert!(err.contains("strlen"), "unexpected message: {err}");
        let wrong_type =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) true strlen . ;";
        let err = check_src(wrong_type).unwrap_err();
        assert!(err.contains("strlen"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_redeclaring_another_extern_is_error() {
        let src = "extern: foo ( i64 -- i64 ) \"foo\" ;\nextern: foo ( i64 -- i64 ) \"bar\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_overload_on_non_struct_input_is_error() {
        // Criterion 5/R1: an enum, an array, or a scalar input is rejected
        // exactly as a non-struct input would be, with a located error.
        let enum_input = "type: E | V ; : drop ( E -- ) drop ;";
        let err = check_src(enum_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("type:"), "unexpected message: {err}");

        let array_input = ": drop ( [i64 4] -- ) drop ;";
        let err = check_src(array_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");

        let scalar_input = ": drop ( i64 -- ) drop ;";
        let err = check_src(scalar_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_overload_with_wrong_arity_is_error() {
        // R1: a `drop` overload declaring anything other than exactly one
        // input is a located error, distinct from the non-struct-input and
        // output rejections tested above.
        let src = "type: T x i64 ; : drop ( T T -- ) drop drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(
            err.contains("must declare exactly one input"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_drop_overload_with_output_is_error() {
        // Criterion 6/R1: a `drop` overload declaring an output is a located
        // error, regardless of whether it also declares an input.
        let src = "type: T x i64 ; : drop ( T -- i64 ) drop 0 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("output"), "unexpected message: {err}");
    }

    #[test]
    fn check_duplicate_drop_overload_for_one_struct_is_error() {
        // Criterion 7/R1: two `drop` overloads for the same struct id is a
        // located error naming that struct, even though the two words'
        // bodies are otherwise unrelated. Both bodies destructure rather
        // than self-recurse: a self-recursive body would let R6's own
        // recursion check produce a message containing both "T" and "drop"
        // even if the duplicate-override rejection this test targets were
        // deleted entirely, since `find_drop_overloads` runs and returns
        // before either body is ever checked.
        let src = "type: T x i64 ; : drop ( T -- ) | a | a T>x drop ; \
                   : drop ( T -- ) | a | a T>x drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("`T` already defines its own `drop`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_drop_overloads_for_different_structs_both_land_in_the_registry() {
        // Criterion 16's check-side half: two overrides for different
        // structs coexist with distinct `StructId` keys, with no collision
        // reported (the module checks fine), and the registry carries one
        // entry per struct.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) | a | a A>x . ; : drop ( B -- ) | b | b B>y . ; \
                   : main ( -- ) 1 A drop 2 B drop ;";
        check_src(src).unwrap();

        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let registry = find_drop_overloads(&module.words, &module.structs).unwrap();
        assert_eq!(
            registry.len(),
            2,
            "expected one entry per struct: {registry:?}"
        );
    }

    #[test]
    fn check_drop_overloads_are_excluded_from_env() {
        // Stage-test obligation (criterion 16's check-side half): neither
        // override lands in `env` under the shared literal name `"drop"` --
        // if it did, the second override registered would silently clobber
        // the first with no diagnostic, since `check`'s env-registration
        // loop has no redeclaration check for ordinary `:` words the way
        // `check_extern_decls` has for `extern:`. Mirrors `check`'s own
        // filtered registration loop rather than calling it directly, since
        // `env` is internal to `check`.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) drop ; : drop ( B -- ) drop ; \
                   : main ( -- ) 1 A drop 2 B drop ;";
        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let registry = find_drop_overloads(&module.words, &module.structs).unwrap();
        let overload_indices: HashSet<usize> = registry.values().copied().collect();
        let mut env: HashMap<String, Vec<Overload>> = HashMap::new();
        for (idx, word) in module.words.iter().enumerate() {
            if overload_indices.contains(&idx) {
                continue;
            }
            env.insert(
                word.name.clone(),
                vec![Overload {
                    sig: sig_of(&word.effect),
                    symbol: word.name.clone(),
                }],
            );
        }
        assert!(
            !env.contains_key("drop"),
            "a `drop` overload leaked into env: {env:?}"
        );
    }

    #[test]
    fn check_drop_overload_with_self_recursive_struct_is_still_a_declaration_error_not_overflow() {
        // R1's ordering-hazard caveat: a self-recursive struct with a
        // malformed `drop` override naming that very struct (here, an
        // extra output) must still produce this pre-pass's own located
        // diagnostic, not overflow the stack inside `is_copy`/
        // `check_recursion` -- the pre-pass runs before `check_types`
        // (where `check_recursion` lives) and never calls `is_copy` on the
        // declared input type itself.
        let src = "type: Loop | Wrap next Loop | End ; : drop ( Loop -- i64 ) drop 0 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("output"), "unexpected message: {err}");
    }

    /// A checked module, for the tests that read a type fact back out of the
    /// registries rather than only asserting a diagnostic.
    fn checked_module(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }

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

    fn struct_ty(module: &Module, name: &str) -> Type {
        let idx = module
            .structs
            .iter()
            .position(|s| s.name == name)
            .expect("declared struct");
        Type::Struct(StructId::from_index(idx), module.structs[idx].name_static)
    }

    #[test]
    fn check_struct_with_drop_overload_is_linear() {
        // Criterion 1/R3: the override forces linearity, so a struct whose
        // every field is `Copy` is not `Copy`. Without the override the same
        // declaration folds to `Copy`, which is what makes this a real
        // decision rather than a restatement of the field fold.
        let module = checked_module(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = struct_ty(&module, "File");
        assert!(!is_copy(
            file,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(is_linear(
            file,
            &module.structs,
            &module.enums,
            &module.arrays
        ));

        let plain = checked_module("type: File fd i64 ; : main ( -- ) 1 File drop ;");
        assert!(is_copy(
            struct_ty(&plain, "File"),
            &plain.structs,
            &plain.enums,
            &plain.arrays
        ));
    }

    #[test]
    fn check_dup_of_drop_overload_type_names_the_cause() {
        // Criterion 2/R4: the reason-carrying cause, in both `Ctx` arms. The
        // generic linear wording ("no bits to copy") would be actively
        // misleading here: `File`'s bits are one plain `i64`, and its own
        // `: drop` declaration is the whole reason they may not be copied.
        let err = check_src(&format!(
            "{FILE_RESOURCE} : main ( -- ) 1 File dup drop drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
        assert!(
            err.contains("`File` is linear because it defines `drop`"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("no bits to copy"),
            "the generic linear cause was used: {err}"
        );

        // The `Ctx::Line` arm: the same fact reaches a bare REPL line, whose
        // carried `File` slot is linear for the same reason.
        let module = checked_module(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let tokens = lex("dup").unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let err = infer_line(
            &terms,
            &[struct_ty(&module, "File")],
            &HashMap::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &module.structs,
            &module.enums,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(
            err.contains("`File` is linear because it defines `drop`"),
            "unexpected message: {err}"
        );
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
    fn check_double_drop_of_all_copy_resource_is_use_after_move_error() {
        // Criterion 4/R3: a second `drop` of the same resource is a compile
        // error rather than a runtime double-close, which is the whole point
        // of forcing linearity on a struct the field fold calls `Copy`.
        let err = check_src(&format!(
            "{FILE_RESOURCE} : main ( -- ) 1 File | f | f drop f drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("local `f`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_body_must_consume_linear_fields() {
        // Criterion 12/R5/R9: an override body is checked like any other word
        // body, so a resource holding a linear field is already forced to
        // account for it -- no scalar-only restriction, and no new check.
        let src = format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : drop ( Res -- ) | r | r Res> drop ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        );
        check_src(&src).unwrap();

        let forgotten = format!(
            "{SPY_DEF}type: Inner s Spy ; type: Res i Inner ; \
             : drop ( Res -- ) | r | ; \
             : main ( -- ) 1 Spy Inner Res drop ;"
        );
        let err = check_src(&forgotten).unwrap_err();
        assert!(
            err.contains("linear value `r` is never consumed"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_drop_body_direct_self_recursion_is_error() {
        // Criterion 8/R6: a `drop` body that drops its own receiver is a
        // cycle of length one. The message names the chain and `File>` as the
        // remedy, since destructuring is what the user has to do instead.
        let src = "type: File fd i64 ; : drop ( File -- ) drop ; : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_body_indirect_self_recursion_through_helper_is_error() {
        // Criterion 9/R6: the same rejection through one helper word, which is
        // why this is reachability over the whole call graph rather than a
        // self-call test. The chain names the helper it goes through.
        let src = "type: File fd i64 ; \
                   : shut ( File -- ) drop ; \
                   : drop ( File -- ) shut ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`shut`"), "unexpected message: {err}");
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_body_recursion_inside_an_if_arm_is_error() {
        // R6: the call graph is over calls in *any* position, so the walker
        // has to visit both `if` arms and every term after them --
        // `tail_position_calls` only ever reads `terms.last()`, and would see
        // neither of these.
        let src = "type: File fd i64 ; \
                   : shut ( File -- ) drop ; \
                   : drop ( File -- ) | f | true if f shut else f shut end 1 . ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`shut`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_of_copy_scalar_inside_drop_body_is_not_a_cycle() {
        // Criterion 10/R6: the dogfood's own shape. Its body ends in a `drop`
        // of the `Copy` `i64` its extern call returns, which a name-keyed
        // graph would read as a call to the override itself and reject.
        let src = "type: File fd i64 ; \
                   : drop ( File -- ) | f | f File>fd drop ; \
                   : main ( -- ) 1 File drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_drop_of_different_resource_inside_another_drop_body_is_ok() {
        // Criterion 11/R6: dispatch is per struct id, so `drop@A` disposing a
        // `B` is an edge to `drop@B` and nothing more -- no cycle, since
        // `drop@B` reaches nothing back.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) | a | a A>x B drop ; \
                   : drop ( B -- ) | b | b B>y drop ; \
                   : main ( -- ) 1 A drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_drop_body_recursion_through_a_containing_aggregate_is_error() {
        // Criterion 21/R6 case (b): `Box` has no override, so dropping one
        // runs generic field glue that disposes its `File` field through
        // `File`'s own override -- unbounded recursion at runtime, invisible
        // to a graph that only looked at directly dropped types.
        let src = "type: File fd i64 ; type: Box f File ; \
                   : drop ( File -- ) | f | f Box drop ; \
                   : main ( -- ) 1 File drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("recursive `drop` overload for `File`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`File>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_drop_of_an_overridden_aggregate_disposing_its_overridden_field_is_not_a_cycle() {
        // R6: case (b) must not fire when the dropped type is *itself*
        // overridden -- `B`'s own body is its whole disposal, so the graph
        // must reflect only the `drop` calls that body actually makes, never
        // a synthesized walk of its fields. D3 requires `B`'s override to
        // dispose its drop-overloaded `a` field with a real `drop` call
        // (destructuring it apart from calling `drop` would itself be D3's
        // own rejection), forming exactly one edge, `B` -> `A`; since `A`'s
        // own override never calls back into `B`, this is not a cycle.
        let src = "type: A x i64 ; type: B a A ; \
                   : drop ( A -- ) | a | a A>x drop ; \
                   : drop ( B -- ) | b | b B>a drop ; \
                   : main ( -- ) 1 A B drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn collect_drop_targets_stops_descending_at_an_overridden_struct() {
        // R6 case (b), on `collect_drop_targets` directly. Post-D3, no legal
        // Sooth program can discriminate this rule any more (see the
        // `check_src`-based test above): disposing an overridden field always
        // requires a real `drop` call, which already contributes the same
        // edge a field-walk would synthesize, so a mutated (fields-walking)
        // version of this function passes every `check_src` test just as the
        // correct one does. Hand-build the registries instead: `B` overrides
        // `drop` and has a field of type `A`, which also overrides `drop`.
        // Walking `B`'s targets must add `B`'s own override and nothing else
        // -- never descend into the overridden field to add `A`'s too.
        let a = StructDecl {
            name: "A".to_string(),
            name_static: "A",
            fields: vec![("x".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: true,
            is_bundle: false,
            module: 0,
        };
        let b = StructDecl {
            name: "B".to_string(),
            name_static: "B",
            fields: vec![("a".to_string(), Type::Struct(StructId::from_index(0), "A"))],
            span: Span::default(),
            has_drop_overload: true,
            is_bundle: false,
            module: 0,
        };
        let structs = vec![a, b];
        let mut overloads = HashMap::new();
        overloads.insert(StructId::from_index(0), 0usize);
        overloads.insert(StructId::from_index(1), 1usize);

        let mut found = Vec::new();
        collect_drop_targets(
            Type::Struct(StructId::from_index(1), "B"),
            &structs,
            &[],
            &[],
            &[],
            &overloads,
            &mut Vec::new(),
            &mut found,
        );

        assert_eq!(
            found,
            vec![1],
            "walking B's targets must stop at B's own override, never also \
             descend into its overridden `A` field"
        );
    }

    #[test]
    fn check_drop_body_sharing_a_helper_with_another_word_is_not_a_cycle() {
        // R6: reachability is over the whole call graph, so a helper called
        // both from an override and from elsewhere must not read as a cycle
        // just for being reachable from two places.
        let src = "type: File fd i64 ; \
                   : show ( i64 -- ) . ; \
                   : drop ( File -- ) | f | f File>fd show ; \
                   : main ( -- ) 1 File drop 2 show ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_a_word_named_drop_contributes_no_tail_call_edge() {
        // A `drop` term never resolves to a user word (`check_shuffle`
        // intercepts it first), so the tail-call graph must not treat one as a
        // call to a `drop` overload: `helper`'s trailing `drop` of an `i64`
        // would otherwise close a fabricated mutual cycle with the override
        // that tail-calls `helper`.
        let src = "type: T x i64 ; \
                   : helper ( i64 -- ) drop ; \
                   : drop ( T -- ) | t | t T>x helper ; \
                   : main ( -- ) 1 T drop ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_extern_accepts_the_full_r2_boundary_type_set() {
        // R2: the numeric tower, `bool`, `&T`/`&!T`, and `cstr` may all cross
        // an `extern:` boundary in either position.
        let src = "extern: f1 ( i64 u8 usize isize f64 f32 bool -- i64 ) \"f1\" ;\nextern: f2 ( &i64 &!i64 -- i64 ) \"f2\" ;\nextern: f3 ( cstr -- cstr ) \"f3\" ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_extern_with_str_parameter_is_error() {
        // R2/R7: a `str` is a descriptor handle (R4), not a scalar or a
        // single opaque `Ptr`, so it matches no C parameter; the rejection
        // names the total conversion to `cstr`.
        let src = "extern: f ( str -- i64 ) \"f\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("matches no C parameter") && err.contains("`cstr`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_returning_str_is_error() {
        // R11: a returned `str` would be one not built from a literal, which
        // is the invariant R10's `Copy`/non-escaping status rests on.
        let src = "extern: f ( -- str ) \"f\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("cannot return a `str`") && err.contains("static data only"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_with_aggregate_parameter_is_error() {
        // Criterion 11/R3: an owned aggregate (struct/enum/array) as an
        // `extern:` input is rejected at the declaration.
        let src = "type: Point x i64 y i64 ;\nextern: foo ( Point -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
        assert!(err.contains("Point"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_with_array_parameter_is_error() {
        let src = "extern: foo ( [i64 4] -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_with_owned_pointer_parameter_is_error() {
        // R3: `^T` is an owned aggregate too, rejected in input position
        // with the generic aggregate message (the output-specific
        // "forge ownership" message is only for the output position).
        let src = "extern: foo ( ^i64 -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_cannot_express_a_variadic_c_function() {
        // R3: `extern:`'s grammar has no syntax for a variadic parameter
        // list, so `printf` cannot be usefully declared: only a fixed
        // effect can be spelled, e.g. one `cstr` and nothing else.
        let src = "extern: printf ( cstr -- i64 ) \"printf\" ;";
        check_src(src).unwrap();
        let err =
            crate::parser::parse(&lex("extern: printf ( cstr ... -- i64 ) \"printf\" ;").unwrap())
                .unwrap_err();
        assert!(
            err.contains("unknown type `...`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_multi_output_is_error() {
        // Criterion 18/R8: a two-output `extern:` describes no C prototype.
        // Unrejected it lowered to a discarded result and panicked in the
        // *next* consumer of the value that was never pushed, naming the
        // wrong term; the diagnostic sits at the declaration instead.
        let src = "extern: two ( i64 -- i64 i64 ) \"two\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("`extern: two` declares 2 outputs")
                && err.contains("no C function returns more than one value"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_extern_returning_owned_pointer_is_error() {
        // Criterion 12/R3: an `extern:` returning `^T` is rejected: it would
        // forge ownership of memory the allocator did not hand out.
        let src = "extern: foo ( i64 -- ^i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("forge ownership"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_returning_a_reference_is_error() {
        // Criterion 13/R3: reusing the existing no-declared-output-reference
        // message rather than duplicating it.
        let src = "extern: foo ( i64 -- &i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("a reference cannot be stored"),
            "unexpected message: {err}"
        );
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
    fn check_len_on_str_types_as_usize() {
        // R8: `check_str_word` claims `len` on a `str` operand before the
        // array path ever sees it, consuming the `str` and typing the result
        // `usize` (not the array `len`'s non-consuming signature).
        check_src(": w ( -- usize ) \"hi\" len ;").unwrap();
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
        let src = ": w ( bool -- i64 ) if 1 1 else 1 end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different stack depths"));
    }

    #[test]
    fn check_branch_join_types_agree_ok() {
        // Both arms leave a single `i64`: the join unifies cleanly.
        check_src(": w ( bool -- i64 ) if 1 else 2 end ;").unwrap();
    }

    #[test]
    fn check_branch_join_type_mismatch_is_error() {
        // `then` leaves an `i64`, `else` leaves a `bool`: same depth, different type.
        let src = ": w ( bool -- i64 ) if 1 else true end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different types"), "unexpected message: {err}");
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
    fn scope_leave_reports_the_unconsumed_linear_local() {
        // `leave_block`'s diagnostic depends on this return value. Extent is
        // enforced by checking each arm on its own `scope.clone()`, not by the
        // `bound` truncation `leave` performs as a side effect, so the extent
        // rule is covered end to end by the goldens rather than here.
        let mut scope = Scope::default();
        let prov = &mut Provenance::default();
        scope.bind("a", Slot::computed(Type::I64), false, prov);
        let depth = scope.depth();
        scope.bind("b", Slot::computed(Type::I64), false, prov);
        assert!(scope.leave(depth).is_none(), "a Copy local leaves cleanly");

        // R6: a linear name leaving scope with its value still held is what the
        // block-end firing site reports. `bind`'s `linear` flag is passed
        // explicitly by the caller (not derived from the `Type` via
        // `is_copy`), so any type distinct from `a`'s suffices here.
        scope.bind("s", Slot::computed(Type::BOOL), true, prov);
        let leaked = scope.leave(depth).expect("an unconsumed linear local");
        assert_eq!((leaked.0.as_str(), leaked.1), ("s", Type::BOOL));
        assert_eq!(leaked.2, MoveState::Live);
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
    fn check_main_linear_output_is_error() {
        let err = check_src(&format!("{SPY_DEF}: main ( -- Spy ) 7 Spy ;")).unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_main_linear_input_is_error() {
        let err = check_src(&format!("{SPY_DEF}: main ( Spy -- ) | s | s drop ;")).unwrap_err();
        assert!(
            err.contains("cannot declare a linear type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
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
    fn check_main_copy_effect_is_ok() {
        check_src(": main ( i64 -- i64 ) 1 + ;").unwrap();
        // The misfire risk is `is_copy`'s recursive struct/enum arms, not the
        // scalar arm: a Copy struct in `main`'s effect must not be rejected.
        check_src("type: P a i64 b i64 ; : main ( P -- ) P> drop drop ;").unwrap();
    }

    #[test]
    fn check_clause_body_duplicate_local_is_error() {
        let src = "type: Shape | Circle r f64 s f64 ;
             : area ( Shape -- f64 ) | Circle | a a | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("duplicate local"), "unexpected message: {err}");
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`area`"), "unexpected message: {err}");
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
        check_src(": sign ( i64 -- i64 ) 0 > if 1 else 0 end ;").unwrap();
    }

    #[test]
    fn check_if_condition_not_bool_is_error() {
        let src = ": w ( -- i64 ) 5 if 1 else 2 end ;";
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

    #[test]
    fn check_declared_output_type_mismatch_is_error() {
        let src = ": w ( i64 -- bool ) 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("type mismatch"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffle_dup_bool_is_type_transparent() {
        // `dup` of a `bool` yields two `bool`s and satisfies the declaration.
        check_src(": w ( bool -- bool bool ) dup ;").unwrap();
    }

    #[test]
    fn check_arith_same_width_ok() {
        check_src(": w ( -- i32 ) 1 >i32 2 >i32 + ;").unwrap();
    }

    #[test]
    fn check_arith_mixed_width_is_error() {
        // An `i32` and an `i64` fed to `+` names both differing types, via
        // the operand-pair-mismatch diagnostic specifically (not just any error
        // that happens to mention both type names).
        let src = ": f ( -- i32 ) 1 >i32 5 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_mixed_sign_is_error() {
        // `u8` and `i8` fed to `<` names both differing operand types, via
        // the same operand-pair-mismatch diagnostic.
        let src = ": w ( -- bool ) 200 >u8 5 >i8 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`u8`"), "unexpected message: {err}");
        assert!(err.contains("`i8`"), "unexpected message: {err}");
    }

    #[test]
    fn check_arith_mixed_int_float_is_error() {
        // X1: mixed int/float arithmetic names both operand types.
        let src = ": f ( -- f64 ) 1 >i32 5.0 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_mixed_float_width_is_error() {
        // X2: mixed float-width comparison names both operand types.
        let src = ": w ( -- bool ) 1.0 >f32 2.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_div_same_float_type_ok() {
        check_src(": w ( -- f64 ) 1.0 2.0 / ;").unwrap();
    }

    #[test]
    fn check_div_on_ints_is_error() {
        // X3: `/` requires floats; integer operands are a sharp error.
        let src = ": w ( -- i64 ) 4 2 / ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`/`"), "unexpected message: {err}");
        assert!(err.contains("float"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_mod_same_int_type_ok() {
        check_src(": w ( -- i64 ) 5 2 mod ;").unwrap();
    }

    #[test]
    fn check_mod_on_floats_is_error() {
        // X4: `mod` requires integers; float operands are a sharp error.
        let src = ": w ( -- f64 ) 5.0 2.0 mod ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`mod`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_max_same_int_type_ok() {
        check_src(": w ( -- i64 ) 3 5 max ;").unwrap();
    }

    #[test]
    fn check_max_on_floats_is_error() {
        // X9: `max` is integer-only; a float pair names `max-total`.
        let src = ": w ( -- f64 ) 3.0 5.0 max ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`max`"), "unexpected message: {err}");
        assert!(err.contains("`max-total`"), "unexpected message: {err}");
    }

    #[test]
    fn check_max_total_same_float_type_ok() {
        check_src(": w ( -- f64 ) 3.0 5.0 max-total ;").unwrap();
    }

    #[test]
    fn check_max_total_on_ints_is_error() {
        // X10: `max-total` is float-only; an integer pair names `max`.
        let src = ": w ( -- i64 ) 3 5 max-total ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`max-total`"), "unexpected message: {err}");
        assert!(err.contains("`max`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_or_xor_same_type_ok() {
        check_src(": w ( -- i32 ) 1 >i32 2 >i32 and 3 >i32 or 4 >i32 xor ;").unwrap();
    }

    #[test]
    fn check_bitwise_and_mixed_width_is_error() {
        let src = ": w ( -- i64 ) 1 >i32 2 and ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same integer or bool type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_or_xor_on_bool_is_ok() {
        // Bool is now an accepted homogeneous operand class for `and`/`or`/`xor`
        // (logical-and on two 0/1 bools coincides with bitwise-and).
        check_src(": w ( -- bool ) true false and true false or drop true false xor drop ;")
            .unwrap();
    }

    #[test]
    fn check_bitwise_and_mixed_bool_int_is_error() {
        let src = ": w ( -- bool ) true 5 and ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same integer or bool type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`bool`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_on_float_is_error() {
        let src = ": w ( -- f64 ) 3.0 5.0 and ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_not_same_type_ok() {
        check_src(": w ( -- u8 ) 5 >u8 not ;").unwrap();
    }

    #[test]
    fn check_not_on_float_is_error() {
        let src = ": w ( -- f64 ) 3.0 not ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`not`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_not_on_bool_is_ok() {
        // `not` is type-directed: on a `bool` it is logical negation, not
        // the integer bitwise complement (R9-ext).
        check_src(": w ( -- bool ) true not ;").unwrap();
    }

    #[test]
    fn check_cmp_le_ge_ne_numeric_same_type_ok() {
        check_src(": w ( -- bool bool bool ) 1 2 <= 1 2 >= 1 2 <> ;").unwrap();
    }

    #[test]
    fn check_cmp_le_ge_ne_on_bool_is_error() {
        // Comparisons stay numeric-only: `bool` is never accepted, even
        // though it now is for `and`/`or`/`xor`.
        let src = ": w ( -- bool ) true false <= ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_ne_mixed_type_is_error() {
        let src = ": w ( -- bool ) 1 >i32 2 <> ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shl_shr_i64_count_ok() {
        check_src(": w ( -- u8 ) 1 >u8 3 shl ;").unwrap();
        check_src(": w ( -- u8 ) 200 >u8 3 shr ;").unwrap();
    }

    #[test]
    fn check_shl_count_not_i64_is_error() {
        let src = ": w ( -- u8 ) 1 >u8 3 >i32 shl ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`shl`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`i32`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shr_value_not_int_is_error() {
        let src = ": w ( -- f64 ) 3.0 2 shr ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`shr`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_is_recognised_as_a_type_name() {
        check_src(": w ( -- usize ) 5 ;").unwrap();
    }

    #[test]
    fn check_usize_arithmetic_and_comparison_ok() {
        check_src(": w ( -- usize ) 5 3 >usize + ;").unwrap();
        check_src(": w ( -- bool ) 5 3 >usize < ;").unwrap();
    }

    #[test]
    fn check_usize_literal_coerces_into_usize_position_ok() {
        // D8: a bare integer literal fills a `usize` position on either side
        // of a homogeneous binary op, no `>usize` required.
        check_src(": w ( -- usize ) 3 >usize 5 + ;").unwrap();
        check_src(": w ( -- usize ) 5 3 >usize + ;").unwrap();
    }

    #[test]
    fn check_usize_computed_value_without_conversion_is_error() {
        // X10: `1 1 +` is a *computed* i64 (no constant folding), so mixing
        // it with a `usize` still needs an explicit `>usize`.
        let src = ": w ( -- usize ) 3 >usize 1 1 + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_to_int_and_int_to_usize_conversions_ok() {
        check_src(": w ( -- i64 ) 5 >usize >i64 ;").unwrap();
        check_src(": w ( -- usize ) 5 >usize ;").unwrap();
    }

    #[test]
    fn check_usize_print_is_type_directed_ok() {
        check_src(": w ( -- ) 5 >usize . ;").unwrap();
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

    // Slice 6h phase 2: D2's shared gate plus the constructor's own D3
    // zero-validity predicate.

    #[test]
    fn array_constructor_i64_ten_yields_slot() {
        check_src(": w ( -- ) [ i64 ; 10 ] drop ;").unwrap();
    }

    #[test]
    fn array_constructor_str_element_is_rejected() {
        let err = check_src(": w ( -- ) [ str ; 4 ] drop ;").unwrap_err();
        assert!(
            err.contains("cannot zero-initialize"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("transitively contains `str` (directly)"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn array_constructor_struct_containing_str_element_is_rejected() {
        let err = check_src("type: HasStr s str ; : w ( -- ) [ HasStr ; 4 ] drop ;").unwrap_err();
        assert!(
            err.contains("cannot zero-initialize a `HasStr`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("transitively contains `str` (via field `s`)"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn array_constructor_depth_two_struct_containing_str_is_rejected() {
        // Proves recursion, not one-level field iteration: `str` is two
        // struct fields deep (`Outer.i.s`), so deleting the struct-field
        // recursion arm (keeping only a one-level check) must fail this.
        let err =
            check_src("type: Inner s str ; type: Outer i Inner ; : w ( -- ) [ Outer ; 4 ] drop ;")
                .unwrap_err();
        assert!(
            err.contains("cannot zero-initialize a `Outer`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("transitively contains `str` (via field `i` -> field `s`)"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn array_constructor_struct_with_array_of_str_field_is_rejected() {
        // The predicate's array arm: the offending `str` is reached only
        // through a struct field that is itself an array. Deleting the
        // array-element recursion arm must fail this test.
        let err = check_src("type: Wrap arr [str 4] ; : w ( -- ) [ Wrap ; 4 ] drop ;").unwrap_err();
        assert!(
            err.contains("cannot zero-initialize a `Wrap`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("transitively contains `str` (via field `arr` -> array element)"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn array_constructor_enum_with_str_on_a_nonzero_variant_is_rejected() {
        // Pins the conservative all-variant recursion: `str` lives on `B`,
        // not the zero-tag `A`, so a variant-0-only walk would miss it.
        let err = check_src("type: E | A | B s str ; : w ( -- ) [ E ; 4 ] drop ;").unwrap_err();
        assert!(
            err.contains("cannot zero-initialize a `E`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("transitively contains `str` (via variant `B` field `s`)"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn array_constructor_struct_containing_quotation_element_is_rejected() {
        let err = check_src("type: Boxed f [ i64 -- i64 ] ; : w ( -- ) [ Boxed ; 4 ] drop ;")
            .unwrap_err();
        assert!(
            err.contains("cannot zero-initialize a `Boxed`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("transitively contains `[ i64 -- i64 ]` (via field `f`)"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn array_constructor_linear_element_is_rejected() {
        // Preempted by the module-wide `check_no_linear_array_elements` sweep
        // (D1 interns the shape unconditionally at parse time, before any
        // body is checked), rather than by the new per-site gate -- but
        // still a located rejection, not a silent accept.
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) [ Spy ; 4 ] drop ;")).unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn fill_still_accepts_a_str_element() {
        // D4: `fill` replicates a real seed and never mints one from zeroed
        // memory, so it keeps accepting `str`/`cstr`/a quotation -- the
        // shared gate's zero-safety branch is off for `fill`.
        check_src(": main ( -- ) \"hi\" 3 fill drop ;").unwrap();
    }

    #[test]
    fn fill_diagnostics_unchanged_after_site_parameterization() {
        // D2: `fill`'s rendered diagnostics must stay byte-identical to
        // before the shared gate existed. Assert the full strings, not
        // `contains("fill")`.
        let linear_err =
            check_src(&format!("{SPY_DEF}: w ( -- ) 0 Spy 3 fill drop ;")).unwrap_err();
        assert_eq!(
            linear_err,
            "error: linear array elements are not supported yet in `w` (line 3)\n  `fill` would replicate a `Spy` across every slot, but `Spy` is linear and has no `Copy` instance\n  note: declared ( -- )"
        );
        let ref_err = check_src(": w ( &i64 -- ) 3 fill drop ;").unwrap_err();
        assert_eq!(
            ref_err,
            "error: a reference cannot be stored in `w` (line 1)\n  the element `fill` would store has type `&i64`\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow"
        );
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
    fn check_print_on_array_is_error() {
        // X6/R13: `.` on an array is a sharp located error naming `[T N]`.
        let err = check_src(": w ( -- ) 0 4 fill . ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
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
    fn check_value_recursion_through_array_element_is_error() {
        // X5/R14/M3: a struct containing itself via an array element is a
        // recursive definition (infinite size), caught by the DFS.
        let err = check_src("type: Node kids [Node 4] ;").unwrap_err();
        assert!(err.contains("recursive"), "unexpected message: {err}");
        assert!(err.contains("Node"), "should name the cycle: {err}");
    }

    #[test]
    fn check_usize_mixed_with_bool_is_error() {
        // X9: `usize` mixed with a non-coercible operand (`bool`) names both.
        let src = ": w ( -- usize ) 5 >usize true and ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_mixed_with_float_is_error() {
        // X9: `usize` mixed with `f64` (both numeric, not coercible).
        let src = ": w ( -- bool ) 5 >usize 1.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_declared_output_needs_conversion_is_error() {
        // X10 at a declared-output position: a computed `i64` doesn't
        // silently satisfy a declared `usize` output.
        let src = ": w ( -- usize ) 1 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_isize_mixed_with_usize_is_error() {
        // `usize` and `isize` are sibling size types but do not coerce
        // into each other; mixing them is a plain type mismatch naming both
        // backticked types.
        let src = ": w ( -- bool ) 5 >usize 3 >isize < ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`isize`"), "unexpected message: {err}");
    }

    #[test]
    fn check_isize_declared_output_needs_conversion_is_error() {
        // X10 at a declared-output position, mirroring
        // check_usize_declared_output_needs_conversion_is_error: a computed
        // `i64` doesn't silently satisfy a declared `isize` output, and the
        // message names the backticked `isize` form rather than `usize`.
        let src = ": w ( -- isize ) 1 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`isize`"), "unexpected message: {err}");
        assert!(err.contains(">isize"), "unexpected message: {err}");
    }

    #[test]
    fn check_usize_branch_merge_keeps_computed_arm_non_coercible_is_error() {
        // A literal in one arm and a computed value in the other must NOT
        // merge to a coercible literal: on the computed arm's runtime path a
        // computed `i64` would fill the `usize` output without `>usize` (X10).
        for src in [
            ": w ( bool -- usize ) if 5 else 1 1 + end ;",
            ": w ( bool -- usize ) if 1 1 + else 5 end ;",
        ] {
            let err = check_src(src).unwrap_err();
            assert!(err.contains("usize"), "unexpected message: {err}");
            assert!(err.contains(">usize"), "unexpected message: {err}");
        }
    }

    #[test]
    fn check_usize_branch_merge_both_literals_coerces_ok() {
        // Both arms leave a literal, so the merged slot stays a coercible
        // literal and fills the `usize` output.
        check_src(": w ( bool -- usize ) if 5 else 6 end ;").unwrap();
    }

    #[test]
    fn check_usize_call_argument_literal_coerces_ok() {
        // A bare literal fills a declared `usize` parameter without `>usize`.
        let src = ": at ( usize -- usize ) ; : w ( -- usize ) 5 at ;";
        check_src(src).unwrap();
    }

    #[test]
    fn check_usize_call_argument_computed_needs_conversion_is_error() {
        let src = ": at ( usize -- usize ) ; : w ( -- usize ) 1 1 + at ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_int_to_float_ok() {
        check_src(": w ( -- f64 ) 5 >f64 ;").unwrap();
    }

    #[test]
    fn check_conv_float_to_int_ok() {
        check_src(": w ( -- i64 ) 5.0 >i64 ;").unwrap();
    }

    #[test]
    fn check_conv_float_target_of_bool_is_error() {
        // X5: a conversion to a float target applied to a `bool` source.
        let src = ": w ( -- f64 ) true >f64 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_unknown_float_target_is_error() {
        // X6: `>f128` reads as an unknown conversion target.
        let src = ": w ( -- f64 ) 5.0 >f128 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("f128"), "unexpected message: {err}");
    }

    #[test]
    fn check_float_lit_types_as_f64() {
        check_src(": w ( -- f64 ) 3.14 ;").unwrap();
    }

    #[test]
    fn check_branch_join_float_widths_mismatch_is_error() {
        // `if` branches leaving `f32` vs `f64` disagree at the join (R12).
        let src = ": w ( bool -- f64 ) if 1.0 >f32 else 2.0 end ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different types"), "unexpected message: {err}");
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_branch_join_float_types_agree_ok() {
        check_src(": w ( bool -- f64 ) if 1.0 else 2.0 end ;").unwrap();
    }

    #[test]
    fn check_shuffle_dup_float_is_type_transparent() {
        check_src(": w ( -- f64 f64 ) 1.0 dup ;").unwrap();
    }

    #[test]
    fn check_conv_from_any_int_ok() {
        check_src(": w ( -- u8 ) 5 >i32 >u8 ;").unwrap();
    }

    #[test]
    fn check_conv_of_bool_is_error() {
        // A conversion applied to `bool` is a type error (X5).
        let src = ": w ( -- i32 ) true >i32 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_declared_output_needs_conversion_is_error() {
        // X3: the literal is `i64`, the declared output is `u8`.
        let src = ": f ( -- u8 ) 5 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`u8`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_unknown_target_is_error() {
        // X6: `>i128` reads as an unknown conversion target.
        // (this test predates R10's float target; kept for the integer case)
        let src = ": w ( -- i64 ) 5 >i128 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("i128"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffle_dup_u8_is_transparent() {
        check_src(": w ( -- u8 u8 ) 5 >u8 dup ;").unwrap();
    }

    #[test]
    fn check_shuffle_swap_mixed_types_is_type_transparent() {
        // `swap` reorders a mixed `bool`/`i64` pair with no fixed signature.
        check_src(": w ( bool i64 -- i64 bool ) swap ;").unwrap();
    }

    #[test]
    fn check_print_accepts_every_printable_scalar() {
        // `.` is type-directed over the whole integer tower, both float
        // widths, and `bool`, not just `i64`.
        check_src(": w ( -- ) 5 . ;").unwrap();
        check_src(": w ( -- ) 5 >u8 . ;").unwrap();
        check_src(": w ( -- ) 5 >i32 . ;").unwrap();
        check_src(": w ( -- ) -1 >u64 . ;").unwrap();
        check_src(": w ( -- ) 3.14 . ;").unwrap();
        check_src(": w ( -- ) 3.14 >f32 . ;").unwrap();
        check_src(": w ( -- ) true . ;").unwrap();
    }

    #[test]
    fn builtin_table_plus_has_a_row_per_numeric_type() {
        // Q-A: `+` resolves by exact operand type, so the table carries one
        // homogeneous `(T T -- T)` row for every numeric type and nothing
        // else. Anchored to the literal count (12: eight fixed-width ints,
        // usize/isize, two floats) rather than `numeric_types()` itself, so
        // shrinking that function can't shrink both sides of the comparison
        // together and hide a wiring bug.
        let table = builtin_table();
        let rows = table.get("+").expect("`+` is a builtin operator");
        assert_eq!(rows.len(), 12, "12 numeric rows");
        let mut got: Vec<Type> = rows
            .iter()
            .map(|r| {
                assert_eq!(
                    r.inputs,
                    vec![r.outputs[0], r.outputs[0]],
                    "a `+` row is homogeneous `(T T -- T)`"
                );
                assert_eq!(r.lower, BuiltinLower::Add);
                r.outputs[0]
            })
            .collect();
        let mut want = numeric_types();
        got.sort_by_key(|t| t.name());
        want.sort_by_key(|t| t.name());
        assert_eq!(got, want, "one `+` row per numeric type, no more");
    }

    /// Assert `name`'s rows are exactly a homogeneous `(T T -- T)` set, one
    /// per type in `want`, all lowering `lower`. Shared shape for the
    /// binary numeric-tower operators, mirroring
    /// `builtin_table_plus_has_a_row_per_numeric_type`.
    fn assert_homogeneous_binary_rows(name: &str, want: Vec<Type>, lower: BuiltinLower) {
        let table = builtin_table();
        let rows = table
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` is a builtin operator"));
        assert_eq!(rows.len(), want.len(), "row count for `{name}`");
        let mut got: Vec<Type> = rows
            .iter()
            .map(|r| {
                assert_eq!(
                    r.inputs,
                    vec![r.outputs[0], r.outputs[0]],
                    "a `{name}` row is homogeneous `(T T -- T)`"
                );
                assert_eq!(r.lower, lower, "`{name}` lowers `{lower:?}`");
                r.outputs[0]
            })
            .collect();
        let mut want = want;
        got.sort_by_key(|t| t.name());
        want.sort_by_key(|t| t.name());
        assert_eq!(got, want, "one `{name}` row per expected type, no more");
    }

    #[test]
    fn builtin_table_sub_has_a_row_per_numeric_type() {
        assert_homogeneous_binary_rows("-", numeric_types(), BuiltinLower::Sub);
    }

    #[test]
    fn builtin_table_mul_has_a_row_per_numeric_type() {
        assert_homogeneous_binary_rows("*", numeric_types(), BuiltinLower::Mul);
    }

    #[test]
    fn builtin_table_div_has_a_row_per_float_type() {
        // `/` is float-only (D7): the integer tower divides via a separate
        // hand-written path this table does not cover.
        assert_homogeneous_binary_rows("/", float_types(), BuiltinLower::DivFloat);
    }

    #[test]
    fn builtin_table_mod_has_a_row_per_int_type() {
        assert_homogeneous_binary_rows("mod", int_types(), BuiltinLower::Mod);
    }

    #[test]
    fn builtin_table_max_has_a_row_per_int_type() {
        // `max` is integer-only: `max-total` is the float twin (D7).
        assert_homogeneous_binary_rows("max", int_types(), BuiltinLower::Max);
    }

    #[test]
    fn builtin_table_max_total_has_a_row_per_float_type() {
        assert_homogeneous_binary_rows("max-total", float_types(), BuiltinLower::MaxTotal);
    }

    #[test]
    fn builtin_table_and_has_a_row_per_int_type_plus_bool() {
        // `and`/`or`/`xor` are bitwise on every integer width and logical on
        // `bool` (eager evaluation makes bitwise-on-0/1 coincide with
        // logical), so their domain is `int_types()` plus one `bool` row.
        let mut want = int_types();
        want.push(Type::BOOL);
        assert_homogeneous_binary_rows("and", want, BuiltinLower::And);
    }

    #[test]
    fn builtin_table_or_has_a_row_per_int_type_plus_bool() {
        let mut want = int_types();
        want.push(Type::BOOL);
        assert_homogeneous_binary_rows("or", want, BuiltinLower::Or);
    }

    #[test]
    fn builtin_table_xor_has_a_row_per_int_type_plus_bool() {
        let mut want = int_types();
        want.push(Type::BOOL);
        assert_homogeneous_binary_rows("xor", want, BuiltinLower::Xor);
    }

    #[test]
    fn builtin_table_not_has_a_row_per_int_type_plus_bool() {
        // `not` is unary, so it does not fit `assert_homogeneous_binary_rows`
        // (a `(T -- T)` shape, not `(T T -- T)`).
        let table = builtin_table();
        let rows = table.get("not").expect("`not` is a builtin operator");
        let mut want = int_types();
        want.push(Type::BOOL);
        assert_eq!(rows.len(), want.len(), "row count for `not`");
        let mut got: Vec<Type> = rows
            .iter()
            .map(|r| {
                assert_eq!(r.inputs, vec![r.outputs[0]], "a `not` row is `(T -- T)`");
                assert_eq!(r.lower, BuiltinLower::Not);
                r.outputs[0]
            })
            .collect();
        got.sort_by_key(|t| t.name());
        want.sort_by_key(|t| t.name());
        assert_eq!(got, want, "one `not` row per int type plus `bool`, no more");
    }

    /// Assert `name`'s rows are the irregular `(T, i64 -- T)` shape (the
    /// shift-amount operand is always `i64` regardless of `T`), one per
    /// `int_types()`, lowering `lower`. Nothing before slice 8a fix 4 checked
    /// that the *second* input type is right: a row wrongly shaped `(T, T --
    /// T)` would still pass `builtin_table_plus_has_a_row_per_numeric_type`-
    /// style checks that only compare `inputs[0]`/`inputs[1]` against
    /// `outputs[0]`.
    fn assert_shift_rows(name: &str, lower: BuiltinLower) {
        let table = builtin_table();
        let rows = table
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` is a builtin operator"));
        let want = int_types();
        assert_eq!(rows.len(), want.len(), "row count for `{name}`");
        let mut got: Vec<Type> = rows
            .iter()
            .map(|r| {
                assert_eq!(
                    r.inputs,
                    vec![r.outputs[0], Type::I64],
                    "a `{name}` row is `(T, i64 -- T)`, the count is always `i64`"
                );
                assert_eq!(r.lower, lower, "`{name}` lowers `{lower:?}`");
                r.outputs[0]
            })
            .collect();
        let mut want = want;
        got.sort_by_key(|t| t.name());
        want.sort_by_key(|t| t.name());
        assert_eq!(got, want, "one `{name}` row per int type, no more");
    }

    #[test]
    fn builtin_table_shl_rows_take_an_i64_count_regardless_of_element_type() {
        assert_shift_rows("shl", BuiltinLower::Shl);
    }

    #[test]
    fn builtin_table_shr_rows_take_an_i64_count_regardless_of_element_type() {
        assert_shift_rows("shr", BuiltinLower::Shr);
    }

    #[test]
    fn builtin_table_comparisons_have_a_row_per_numeric_type() {
        use crate::ir::CmpOp;
        let table = builtin_table();
        for (op, cmp) in [
            ("=", CmpOp::Eq),
            ("<", CmpOp::Lt),
            (">", CmpOp::Gt),
            ("<=", CmpOp::Le),
            (">=", CmpOp::Ge),
            ("<>", CmpOp::Ne),
        ] {
            let rows = table
                .get(op)
                .unwrap_or_else(|| panic!("`{op}` is a builtin operator"));
            let want = numeric_types();
            assert_eq!(rows.len(), want.len(), "row count for `{op}`");
            let mut got: Vec<Type> = rows
                .iter()
                .map(|r| {
                    assert_eq!(r.outputs, vec![Type::BOOL], "`{op}` produces `bool`");
                    assert_eq!(r.inputs.len(), 2, "`{op}` is binary");
                    assert_eq!(r.inputs[0], r.inputs[1], "a `{op}` row is homogeneous");
                    assert_eq!(
                        r.lower,
                        BuiltinLower::Cmp(cmp),
                        "`{op}` lowers `Cmp({cmp:?})`"
                    );
                    r.inputs[0]
                })
                .collect();
            let mut want = want;
            got.sort_by_key(|t| t.name());
            want.sort_by_key(|t| t.name());
            assert_eq!(got, want, "one `{op}` row per numeric type, no more");
        }
    }

    #[test]
    fn check_not_on_literal_count_is_not_a_literal_for_fill() {
        // The retired hand-written `not` arm left its operand slot in place,
        // preserving `literal`/`int_val` (so a `not`'d literal fed to `fill`
        // would have used the *pre-negation* value, silently wrong). The
        // table row it was replaced with emits `Slot::computed`, so `fill`
        // now correctly refuses a `not`'d literal as a non-literal count
        // instead of miscounting.
        let err = check_src(": w ( -- ) 0 4 not fill drop ;").unwrap_err();
        assert!(err.contains("literal count"), "unexpected message: {err}");
    }

    #[test]
    fn builtin_table_has_a_row_per_printable_type_for_print() {
        // Rule 6: `.` dispatches over 14 printable types, each a `(T -- )` row
        // lowering a `Print`. Mutation-check: dropping the printable loop or a
        // `push` in `printable_types` fails this. `bool` is not among them
        // (slice 9 R6): it dispatches through the injected library overload.
        let table = builtin_table();
        let rows = table.get(".").expect("`.` is a builtin operator");
        assert_eq!(rows.len(), 14, "14 printable rows");
        let mut got: Vec<Type> = rows
            .iter()
            .map(|r| {
                assert_eq!(r.outputs, Vec::<Type>::new(), "`.` produces nothing");
                assert_eq!(r.lower, BuiltinLower::Print);
                assert_eq!(r.inputs.len(), 1, "`.` is unary");
                r.inputs[0]
            })
            .collect();
        let mut want = printable_types();
        got.sort_by_key(|t| t.name());
        want.sort_by_key(|t| t.name());
        assert_eq!(got, want);
    }

    #[test]
    fn operator_dispatch_resolves_the_exact_row_type() {
        // Guards that resolution yields the right stack-effect type: a
        // homogeneous op over `u8` yields `u8`, a comparison yields `bool`,
        // `.` yields nothing. Note these all resolve identically through the
        // numeric fallback too, so this does *not* prove the table pass is
        // used; `check_not_on_literal_count_is_not_a_literal_for_fill` is the
        // guard that the exact-match table row actually drives dispatch.
        assert_eq!(
            infer_src("5 >u8 3 >u8 +", &[]).unwrap(),
            vec![Type::from_name("u8").unwrap()]
        );
        assert_eq!(infer_src("5 >u8 3 >u8 <", &[]).unwrap(), vec![Type::BOOL]);
        assert_eq!(infer_src("5 .", &[]).unwrap(), Vec::<Type>::new());
    }

    #[test]
    fn check_print_accepts_str_and_cstr() {
        // `.`'s printable-scalar guard also accepts `str`/`cstr` (R9), matched
        // by name rather than `is_numeric`/`is_bool`, since neither is numeric.
        check_src(": w ( -- ) \"hi\" . ;").unwrap();
        check_src(": w ( -- ) \"hi\" cstr . ;").unwrap();
    }

    #[test]
    fn check_print_on_empty_stack_is_underflow_error() {
        let src = ": w ( -- ) . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`.`"), "unexpected message: {err}");
        assert!(err.contains("needs 1 values"), "unexpected message: {err}");
    }

    fn infer_src(src: &str, entry: &[Type]) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        // `bool` is `Type::Enum(BOOL_ENUM_ID, ..)` (Slice 9): a real REPL
        // session seeds this at index 0 (`Session::new`); this bare-line
        // helper mirrors that so a `bool`-producing comparison resolves.
        let bool_enums = [crate::ast::bool_enum_decl()];
        infer_line(
            &terms,
            entry,
            &HashMap::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &[],
            &bool_enums,
            &HashMap::new(),
            &HashMap::new(),
        )
        .map(|(stack, _insts, _overloads)| stack)
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
    fn check_struct_generated_words_flat_struct_ok() {
        check_src(
            "type: Vec2 x i64 y i64 ;
             : main ( -- ) 1 2 Vec2 dup Vec2>x drop Vec2>y drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_generated_words_nested_struct_ok() {
        check_src(
            "type: Vec2 x i64 y i64 ;
             type: Segment from Vec2 to Vec2 ;
             : main ( -- ) 1 2 Vec2 3 4 Vec2 Segment dup Segment>from Vec2>x drop Segment> drop drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_zero_field_registers_only_ctor_and_destructure() {
        check_src("type: Unit ; : main ( -- ) Unit Unit> ;").unwrap();
    }

    #[test]
    fn check_struct_setter_returns_updated_struct_ok() {
        check_src("type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 2 Vec2 3 Vec2<x ;").unwrap();
    }

    #[test]
    fn check_struct_peek_copy_field_leaves_struct_live_ok() {
        // R10: `Vec2|>x` is non-consuming, so the struct is still on the
        // stack for the second peek and the trailing `Vec2>` destructure.
        check_src("type: Vec2 x i64 y i64 ; : main ( -- ) 1 2 Vec2 Vec2|>x drop Vec2> drop drop ;")
            .unwrap();
    }

    #[test]
    fn check_struct_peek_on_linear_field_is_error() {
        // R10: a linear field can't be peeked (workaround: `S>`).
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : main ( -- ) 7 Spy 1 Holds Holds|>a drop drop ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("cannot `Holds|>a`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("`S>`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_peek_on_wrong_type_is_error() {
        // A peek word applied to a value that isn't its struct: names the
        // peek word and both types, same shape as the getter/setter checks.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- i64 ) 5 Vec2|>x drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2|>x"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_duplicate_type_name_is_error() {
        // X2: two `type:` declarations sharing a name name that type.
        let err = check_src("type: Vec2 x i64 ; type: Vec2 y i64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Vec2"), "unexpected message: {err}");
    }

    #[test]
    fn check_recursion_by_value_self_cycle_is_error() {
        // X3/M5: a directly self-referential struct (no `^` anywhere
        // on the cycle) is an error naming the full path (a bare string,
        // no span), and this test itself is proof the checker terminated
        // rather than hung.
        let err = check_src("type: Loop next Loop ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Loop -> Loop"), "unexpected message: {err}");
    }

    #[test]
    fn check_recursion_by_value_mutual_cycle_is_error() {
        // X3/M5: a mutually-recursive pair of structs, no `^`
        // anywhere, names the full path A -> B -> A.
        let err = check_src("type: A b B ; type: B a A ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("A -> B -> A"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_direct_recursion_is_error_not_hang() {
        // X3/M5: a directly self-referential enum (a variant field of its own
        // type) is an error naming the cycle (bare, no span), and this
        // test's return is proof the DFS terminated rather than hung.
        let err = check_src("type: Loop | Wrap next Loop | End ;").unwrap_err();
        assert!(err.contains("recursive enum"), "unexpected message: {err}");
        assert!(err.contains("Loop"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_mutual_recursion_is_error_not_hang() {
        // X3/M5: a mutually-recursive pair of enums, names both in the cycle.
        let err = check_src("type: A | Ta x B ; type: B | Tb y A ;").unwrap_err();
        assert!(err.contains("recursive enum"), "unexpected message: {err}");
        assert!(err.contains('A'), "unexpected message: {err}");
        assert!(err.contains('B'), "unexpected message: {err}");
    }

    #[test]
    fn check_recursion_cell_cycle_in_struct_field_is_ok() {
        // A `^` edge through a struct field is legal, not just through
        // an enum variant payload -- the rule is about size finiteness, not
        // idiom.
        check_src("type: Node v i64 next ^Node ;").unwrap();
    }

    #[test]
    fn check_recursion_cell_cycle_in_enum_variant_is_ok() {
        // The same `^` cycle acceptance in enum variant position,
        // mirroring check_recursion_cell_cycle_in_struct_field_is_ok.
        check_src("type: List | Nil | Cons v i64 next ^List ;").unwrap();
    }

    #[test]
    fn check_recursion_array_element_cell_is_cut_then_rejected_as_linear() {
        // The `^` edge is cut inside an array element too, so this
        // definition survives the recursion rule and reaches the linear
        // array-element rule instead of "recursive array definition".
        let err = check_src("type: Node kids [^Node 4] ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^Node`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_enum_mixed_recursion_is_error_not_hang() {
        // D9/X3: a struct field of enum type closing a cycle back to the
        // struct is caught by the combined-graph DFS.
        let err = check_src("type: S f E ; type: E | V g S ;").unwrap_err();
        assert!(err.contains("recursive"), "unexpected message: {err}");
        assert!(err.contains('S'), "unexpected message: {err}");
        assert!(err.contains('E'), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_direct_element_in_struct_field_is_error() {
        // The parser cannot reject `[Spy N]` (struct fields aren't resolved
        // until the whole module is parsed), so this is the checker's job.
        let err = check_src(&format!(
            "{SPY_DEF}type: Bag xs [Spy 2] ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_direct_element_in_word_signature_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( [Spy 2] -- ) | a | a drop ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_indirect_via_linear_struct_field_is_error() {
        // `Arr`'s element (`Holds`) is not itself `Spy`, but contains one
        // transitively; `is_copy` already sees through that, so the sweep
        // over `module.arrays` must too.
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds s Spy ; type: Arr a [Holds 2] ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_indirect_via_linear_struct_in_signature_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds s Spy ; : w ( [Holds 2] -- ) | a | a drop ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_no_linear_array_elements_copy_element_is_ok() {
        check_src("type: V xs [i64 4] ; : main ( -- ) 0 . ;").unwrap();
    }

    #[test]
    fn array_of_owned_is_error() {
        let err = check_src(": w ( [^i64 4] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^i64`"), "unexpected message: {err}");
    }

    #[test]
    fn owned_of_linear_array_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( ^[Spy 2] -- ) drop ; : main ( -- ) 0 . ;"
        ))
        .unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn nested_array_of_owned_is_error() {
        let err = check_src(": w ( ^[^i64 4] -- ) drop ; : main ( -- ) 0 . ;").unwrap_err();
        assert!(
            err.contains("linear array elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^i64`"), "unexpected message: {err}");
    }

    #[test]
    fn array_of_struct_holding_owned_is_error() {
        // Keeps `emit_drop`'s linear-array `unreachable!` guard valid now that
        // cells are a second linear type: an array whose element only holds a
        // cell transitively must be rejected here too, or lowering would reach
        // that arm with an array needing drop glue.
        let err = check_src("type: Holds c ^i64 ; type: Arr a [Holds 2] ; : main ( -- ) 0 . ;")
            .unwrap_err();
        assert!(err.contains("linear array elements are not supported yet"));
        assert!(err.contains("`Holds`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_and_enum_duplicate_name_across_registries_is_error() {
        // X2: a name used by one struct and one enum names that type.
        let err = check_src("type: Dup x i64 ; type: Dup | V ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Dup"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_nested_aggregate_fields_ok() {
        // D9: a variant may carry a struct, and a struct may carry an enum,
        // acyclically — no recursion error.
        check_src(
            "type: Vec2 x f64 y f64 ;
             type: Shape | Dot p Vec2 | Empty ;
             type: Tagged k Shape n i64 ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_constructor_arity_mismatch_is_error() {
        // X4: too few values fed to the constructor, naming the struct.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 Vec2 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_constructor_field_type_mismatch_is_error() {
        // X4: a `bool` where an `i64` field is expected, naming struct+field type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 true Vec2 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_accessor_on_wrong_type_is_error() {
        // X5: `Vec2>x` applied to a bare `i64` names the accessor and both types.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- i64 ) 5 Vec2>x ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2>x"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_accessor_on_other_struct_is_error() {
        // X5: a `Vec2` accessor applied to a `Segment` names both struct types.
        let src = "type: Vec2 x i64 y i64 ; type: Segment from Vec2 to Vec2 ;
            : main ( -- i64 ) 1 2 Vec2 3 4 Vec2 Segment Vec2>x ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2>x"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
        assert!(err.contains("`Segment`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_print_is_error() {
        // X6: `.` on a struct reaches `print_requires_printable`, naming it.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- ) 1 2 Vec2 . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_equality_operator_is_error() {
        // X7: `=` on two structs is scalar-only, naming the struct type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- bool ) 1 2 Vec2 1 2 Vec2 = ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_arithmetic_operator_is_error() {
        // X7: `+` on two structs is scalar-only, naming the struct type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 2 Vec2 1 2 Vec2 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }

    #[test]
    fn check_struct_unifies_through_if_else_join_ok() {
        // R10: a struct type flows through an `if`/`else` join like any Type.
        check_src(
            "type: Vec2 x i64 y i64 ;
             : pick ( bool -- Vec2 ) if 1 2 Vec2 else 3 4 Vec2 end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_struct_moves_through_shuffles_ok() {
        // R10: dup/drop/swap/over move a struct value with no special case.
        check_src(
            "type: Vec2 x i64 y i64 ;
             : main ( -- Vec2 ) 1 2 Vec2 3 4 Vec2 swap drop dup drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_zero_field_variant_constructor_ok() {
        check_src("type: Cmd | Halt ; : main ( -- Cmd ) Halt ;").unwrap();
    }

    #[test]
    fn check_enum_multi_field_variant_constructor_ok() {
        check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : main ( -- Shape ) 2.0 Circle ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_used_in_word_effect_ok() {
        check_src("type: Shape | Circle r f64 ; : id ( Shape -- Shape ) ;").unwrap();
    }

    #[test]
    fn check_enum_single_variant_newtype_ok() {
        // M3: a single-variant enum is allowed.
        check_src("type: Id | Wrap v i64 ; : main ( -- Id ) 5 Wrap ;").unwrap();
    }

    #[test]
    fn check_enum_duplicate_type_name_across_two_enums_is_error() {
        // X2: two enum `type:` declarations sharing a name.
        let err =
            check_src("type: Shape | Circle r f64 ; type: Shape | Square s f64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_duplicate_type_name_against_struct_is_error() {
        // X2: a struct and an enum sharing a name, across the combined
        // struct+enum registry (D10).
        let err = check_src("type: Vec2 x i64 y i64 ; type: Vec2 | Only v i64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Vec2"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_constructor_arity_mismatch_is_error() {
        // X9: too few values fed to a variant constructor, naming the enum.
        let src = "type: Shape | Rect w f64 h f64 ; : main ( -- Shape ) 1.0 Rect ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Shape"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_constructor_field_type_mismatch_is_error() {
        // X9: a `bool` where an `f64` field is expected, naming both types.
        let src = "type: Shape | Circle r f64 ; : main ( -- Shape ) true Circle ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`f64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_unifies_through_if_else_join_ok() {
        // R10: an enum type flows through an `if`/`else` join like any Type.
        check_src(
            "type: Shape | Circle r f64 | Square s f64 ;
             : pick ( bool -- Shape ) if 1.0 Circle else 2.0 Square end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_moves_through_shuffles_ok() {
        // R10: dup/drop/swap/over move an enum value with no special case.
        check_src(
            "type: Shape | Circle r f64 | Square s f64 ;
             : main ( -- Shape ) 1.0 Circle 2.0 Square swap drop dup drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_enum_struct_and_enum_coexist_ok() {
        // D10: a distinct registry per kind; structs and enums both resolve
        // and both generate correctly-typed words in the same module.
        check_src(
            "type: Vec2 x i64 y i64 ;
             type: Shape | Circle r f64 ;
             : main ( -- Vec2 Shape ) 1 2 Vec2 3.0 Circle ;",
        )
        .unwrap();
    }

    #[test]
    fn check_clause_word_multi_and_zero_field_ok() {
        // R11: a clause per variant, each leaving the single declared output;
        // a clause-body `| w h |` binds the payload, a zero-field clause with
        // a value flowing underneath the scrutinee type-checks.
        check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             type: MaybeInt | None | Some v i64 ;
             : area ( Shape -- f64 ) | Circle dup * 3.14159 * | Rect | w h | w h * ;
             : unwrap-or ( i64 MaybeInt -- i64 ) | None | Some swap drop ;",
        )
        .unwrap();
    }

    #[test]
    fn check_clause_word_non_exhaustive_names_missing_variant() {
        // X4: a clause word missing a variant names the missing one.
        let err = check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             : area ( Shape -- f64 ) | Circle dup * ;",
        )
        .unwrap_err();
        assert!(err.contains("non-exhaustive"), "unexpected message: {err}");
        assert!(err.contains("Rect"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_word_duplicate_clause_names_variant() {
        // X5: two clauses for the same variant names it.
        let err = check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             : area ( Shape -- f64 ) | Circle dup * | Circle dup * | Rect | w h | w h * ;",
        )
        .unwrap_err();
        assert!(
            err.contains("duplicate clause"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Circle"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_word_unknown_variant_names_it_and_enum() {
        // X6: a clause naming a non-variant of the scrutinee enum.
        let err = check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             type: Other | Blob b i64 ;
             : area ( Shape -- f64 ) | Circle dup * | Rect | w h | w h * | Blob 0.0 ;",
        )
        .unwrap_err();
        assert!(err.contains("unknown variant"), "unexpected message: {err}");
        assert!(err.contains("Blob"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_word_on_non_enum_top_input_is_error() {
        // X7: a clause body whose top input is a scalar (not an enum).
        let err = check_src(
            "type: Circle | C r f64 ;
             : bad ( i64 -- i64 ) | C 0 ;",
        )
        .unwrap_err();
        assert!(err.contains("not an enum"), "unexpected message: {err}");
        assert!(err.contains("bad"), "unexpected message: {err}");
    }

    #[test]
    fn check_clause_body_violating_declared_output_is_error() {
        // X8/M6: a clause whose body leaves a type other than the single
        // declared output effect.
        let err = check_src(
            "type: MaybeInt | None | Some v i64 ;
             : bad ( MaybeInt -- i64 ) | None true | Some ;",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_parameter_named_after_variant_is_error() {
        // X12 (D8 backstop): a binding name equal to a registered variant
        // name is rejected. A parameter name is the reachable case — a `|`
        // local named after a variant is instead read as a clause by D8, so
        // the parameter slot is where the collision actually surfaces.
        let err = check_src(
            "type: Shape | Circle r f64 ;
             : bad ( Circle : i64 -- i64 ) drop 0 ;",
        )
        .unwrap_err();
        assert!(err.contains("collides"), "unexpected message: {err}");
        assert!(err.contains("Circle"), "unexpected message: {err}");
    }

    #[test]
    fn check_term_word_with_entry_locals_still_ok() {
        // Regression: a plain term word with `| ... |` entry locals is
        // unaffected by the clause-body path (no enum in scope).
        check_src(": sq ( i64 -- i64 ) | n | n n * ;").unwrap();
    }

    #[test]
    fn check_enum_print_is_error() {
        // X10/M2: `.` on an enum reaches the printable guard, naming the enum.
        let err = check_src("type: Shape | Circle r f64 ; : w ( Shape -- ) . ;").unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_equality_operator_is_error() {
        // X10/M2: `=` on two enums reaches the operand-pair guard.
        let err =
            check_src("type: Shape | Circle r f64 ; : w ( Shape Shape -- bool ) = ;").unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    #[test]
    fn check_enum_arithmetic_operator_is_error() {
        // X10/M2: arithmetic on an enum reaches the operand-pair guard.
        let err =
            check_src("type: Shape | Circle r f64 ; : w ( Shape Shape -- Shape ) + ;").unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }

    fn first_word(src: &str) -> WordDef {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        module.words.into_iter().next().unwrap()
    }

    #[test]
    fn tail_position_final_self_call_is_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec ;");
        assert_eq!(tail_position_calls(&w.body), vec!["rec"]);
        assert!(has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_trailing_arithmetic_is_not_tail() {
        // `rec *`: the final term is `*`, so the self-call is not in tail
        // position (classic non-tail recursion).
        let w = first_word(": rec ( i64 -- i64 ) rec * ;");
        assert_eq!(tail_position_calls(&w.body), vec!["*"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_trailing_swap_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec swap ;");
        assert_eq!(tail_position_calls(&w.body), vec!["swap"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_trailing_drop_is_not_tail() {
        let w = first_word(": rec ( i64 -- i64 ) rec drop ;");
        assert_eq!(tail_position_calls(&w.body), vec!["drop"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_builtin_named_word_trailing_its_own_name_is_not_self_tail() {
        // Slice 8a made every builtin name overloadable, so a builtin-named
        // word ending in that same name is resolving against the builtin
        // table, not recursing: `<` here compares the two extracted `i64`s.
        // `tail_position_calls` still reports the name (it is syntactic);
        // only the self-call conclusion changes.
        let w = first_word(
            "type: Vec2 x i64 y i64 ; : < ( Vec2 Vec2 -- bool ) | a b | a Vec2>x b Vec2>x < ;",
        );
        assert_eq!(tail_position_calls(&w.body), vec!["<"]);
        assert!(!has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_both_terminal_if_arms_are_tail() {
        // A terminal `if` hands tail position to the last term of both arms.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > if rec else rec end ;");
        assert_eq!(tail_position_calls(&w.body), vec!["rec", "rec"]);
        assert!(has_self_tail_call(&w));
    }

    #[test]
    fn tail_position_non_terminal_if_self_call_is_not_tail() {
        // The `if` is followed by more terms, so it is non-terminal and its
        // arms are not in tail position.
        let w = first_word(": rec ( i64 -- i64 ) dup 0 > if rec else 0 end drop 5 ;");
        assert!(!has_self_tail_call(&w));
        assert!(!tail_position_calls(&w.body).contains(&"rec"));
    }

    #[test]
    fn tail_position_clause_body_final_self_call_is_tail() {
        let w = first_word("type: E | A | B ; : w ( E -- E ) | A w | B w ;");
        assert_eq!(tail_position_calls(&w.body), vec!["w", "w"]);
        assert!(has_self_tail_call(&w));
    }

    #[test]
    fn check_mutual_tail_recursion_is_error() {
        // X1: A tail-calls B, B tail-calls A -> located error naming the cycle.
        let err = check_src(": a ( i64 -- i64 ) b ; : b ( i64 -- i64 ) a ;").unwrap_err();
        assert!(
            err.contains("mutual tail recursion"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`a`"), "unexpected message: {err}");
        assert!(err.contains("`b`"), "unexpected message: {err}");
    }

    #[test]
    fn check_non_tail_mutual_recursion_is_ok() {
        // Both words call each other only in non-tail position (`x 1 +`), so no
        // tail-call edge exists and X1 must not fire (R4 no-false-positive).
        check_src(
            ": a ( i64 -- i64 ) dup 0 > if b 1 + else drop 0 end ; \
             : b ( i64 -- i64 ) dup 0 > if a 1 + else drop 0 end ;",
        )
        .unwrap();
    }

    #[test]
    fn check_self_tail_recursion_is_allowed() {
        // A self-loop (`gcd -> gcd`) is tier-1 and must not be flagged as a
        // mutual cycle.
        check_src(&std::fs::read_to_string("examples/gcd.sth").unwrap()).unwrap();
    }

    // Phase 3 Slice 1: the linear core on bare linear values.

    #[test]
    fn is_copy_every_scalar_is_copy_and_a_drop_overloaded_struct_is_not() {
        // `bool` is `Type::Enum(BOOL_ENUM_ID, ..)` (Slice 9): its registry
        // entry must be present, exactly as `assemble_module`/the REPL
        // session seed it, for `is_copy`'s enum arm to resolve it.
        let bool_enums = [crate::ast::bool_enum_decl()];
        for name in ["i8", "u64", "f32", "f64", "bool", "usize"] {
            assert!(
                is_copy(Type::from_name(name).unwrap(), &[], &bool_enums, &[]),
                "{name} is Copy"
            );
        }
        // R3 (slice 8b): a struct with a user `drop` overload is linear
        // whatever its fields say -- built directly here since this test
        // exercises `is_copy`'s own signature, not a checked module.
        let structs = vec![StructDecl {
            name: "Res".to_string(),
            name_static: "Res",
            fields: vec![("tag".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: true,
            is_bundle: false,
            module: 0,
        }];
        let res = Type::Struct(StructId::from_index(0), "Res");
        assert!(!is_copy(res, &structs, &[], &[]));
    }

    #[test]
    fn is_copy_owned_cell_is_never_copy_regardless_of_payload() {
        // R4: always linear, no payload lookup, even over a Copy payload.
        let mut cells = Vec::new();
        let ty = crate::ast::intern_owned_cell_type(&mut cells, Type::I64);
        assert!(!is_copy(ty, &[], &[], &[]));
    }

    #[test]
    fn check_owned_cell_underflow_is_error_for_all_three_words() {
        // `^`, `^>`, `^|>` each underflow the same way as any other word.
        for (op, src) in [
            ("^", ": w ( -- ^i64 ) ^ ;"),
            ("^>", ": w ( -- i64 ) ^> ;"),
            ("^|>", ": w ( -- i64 ) ^|> ;"),
        ] {
            let err = check_src(src).unwrap_err();
            assert!(
                err.contains(&format!("`{op}`")),
                "{op}: unexpected message: {err}"
            );
            assert!(
                err.contains("needs 1 values"),
                "{op}: unexpected message: {err}"
            );
            assert!(err.contains("holds 0"), "{op}: unexpected message: {err}");
        }
    }

    #[test]
    fn check_unwrap_of_non_cell_is_error() {
        // `^>` on a plain `i64` names the word and the offending type.
        let err = check_src(": w ( -- i64 ) 5 ^> ;").unwrap_err();
        assert!(err.contains("`^>`"), "unexpected message: {err}");
        assert!(
            err.contains("requires an owning-cell operand"),
            "unexpected message: {err}"
        );
        assert!(err.contains("found `i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_peek_of_non_cell_is_error() {
        // `^|>` on a plain `bool` names the word and the offending type.
        let err = check_src(": w ( -- bool bool ) true ^|> ;").unwrap_err();
        assert!(err.contains("`^|>`"), "unexpected message: {err}");
        assert!(
            err.contains("requires an owning-cell operand"),
            "unexpected message: {err}"
        );
        assert!(err.contains("found `bool`"), "unexpected message: {err}");
    }

    #[test]
    fn is_copy_struct_is_linear_iff_a_field_is_transitively() {
        // R7/R8 (Phase 2): a struct with no linear field is Copy; one with a
        // linear field (direct or nested) is linear, transitively. `^i64`
        // (an owning cell, always linear regardless of payload) stands in
        // for a direct linear leaf field, since this test exercises
        // `is_copy`'s own fold directly rather than through a checked module.
        let mut owned_cells = Vec::new();
        let cell_ty = intern_owned_cell_type(&mut owned_cells, Type::I64);
        let structs = vec![
            StructDecl {
                name: "Plain".to_string(),
                name_static: "Plain",
                fields: vec![("x".to_string(), Type::I64), ("y".to_string(), Type::I64)],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Holds".to_string(),
                name_static: "Holds",
                fields: vec![("a".to_string(), cell_ty), ("b".to_string(), Type::I64)],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Wraps".to_string(),
                name_static: "Wraps",
                fields: vec![(
                    "h".to_string(),
                    Type::Struct(StructId::from_index(1), "Holds"),
                )],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
        ];
        let plain = Type::Struct(StructId::from_index(0), "Plain");
        let holds = Type::Struct(StructId::from_index(1), "Holds");
        let wraps = Type::Struct(StructId::from_index(2), "Wraps");
        assert!(is_copy(plain, &structs, &[], &[]));
        assert!(!is_copy(holds, &structs, &[], &[]));
        assert!(!is_copy(wraps, &structs, &[], &[]));
    }

    #[test]
    fn is_copy_enum_is_linear_iff_a_variant_field_is_transitively() {
        // R7/R12 (Phase 4): an enum with no linear variant field is Copy; one
        // with a linear field (direct in one variant, or nested through a
        // struct in another) is linear, transitively. `Plain` has no linear
        // variant, `Item` carries a linear field (an owning cell) directly in
        // `Full`, `Boxed` carries one nested inside `Holds`. Built by hand
        // rather than parsed, for the same reason as the struct fold above.
        let mut owned_cells = Vec::new();
        let cell_ty = intern_owned_cell_type(&mut owned_cells, Type::I64);
        let structs = vec![StructDecl {
            name: "Holds".to_string(),
            name_static: "Holds",
            fields: vec![("a".to_string(), cell_ty), ("b".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let variant = |name: &'static str, fields: Vec<(String, Type)>| VariantDecl {
            name: name.to_string(),
            name_static: name,
            fields,
            span: Span::default(),
        };
        let enums = vec![
            EnumDecl {
                name: "Plain".to_string(),
                name_static: "Plain",
                variants: vec![variant("A", vec![]), variant("B", vec![])],
                span: Span::default(),
                module: 0,
            },
            EnumDecl {
                name: "Item".to_string(),
                name_static: "Item",
                variants: vec![
                    variant("Empty", vec![]),
                    variant("Full", vec![("v".to_string(), cell_ty)]),
                ],
                span: Span::default(),
                module: 0,
            },
            EnumDecl {
                name: "Boxed".to_string(),
                name_static: "Boxed",
                variants: vec![
                    variant(
                        "Some",
                        vec![(
                            "h".to_string(),
                            Type::Struct(StructId::from_index(0), "Holds"),
                        )],
                    ),
                    variant("None", vec![]),
                ],
                span: Span::default(),
                module: 0,
            },
        ];
        let plain = Type::Enum(EnumId::from_index(0), "Plain");
        let item = Type::Enum(EnumId::from_index(1), "Item");
        let boxed = Type::Enum(EnumId::from_index(2), "Boxed");
        assert!(is_copy(plain, &structs, &enums, &[]));
        assert!(!is_copy(item, &structs, &enums, &[]));
        assert!(!is_copy(boxed, &structs, &enums, &[]));
    }

    #[test]
    fn check_struct_constructor_takes_a_matching_i64_field_ok() {
        check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;")).unwrap();
    }

    #[test]
    fn check_struct_constructor_on_a_float_field_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7.5 Spy drop ;")).unwrap_err();
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
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
    fn check_print_on_linear_value_is_error() {
        // R16: `.` is a printable-scalar path, and a linear value is not one
        // (the backend's `unreachable!` guard depends on this).
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy . ;")).unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
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
    fn check_second_mention_of_a_copy_local_is_ordinary_reuse() {
        // The move-state tracks linear locals only: a Copy local stays usable.
        check_src(": w ( i64 -- i64 ) | n | n n + ;").unwrap();
    }

    #[test]
    fn check_unconsumed_linear_local_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( Spy -- )\n  | s |\n  1 . ;")).unwrap_err();
        assert!(err.contains("never consumed"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(
            err.contains("`s`"),
            "the error should name the local: {err}"
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
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c if s drop else s drop end ;"
        ))
        .unwrap();
    }

    #[test]
    fn check_linear_local_moved_in_one_arm_then_used_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c if s drop else 1 . end\n  s drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("use after move"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }

    #[test]
    fn check_linear_local_moved_in_one_arm_and_dropped_nowhere_is_error() {
        let err = check_src(&format!(
            "{SPY_DEF}: w ( Spy bool -- )\n  | s c |\n  c if s drop else 1 . end ;"
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
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 = if s drop 0 else 9 Spy n 1 - spin end ;"
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
            "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n  n 0 = if s drop 0 else s n 1 - spin end ;"
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

    #[test]
    fn shared_reference_is_copy_and_mutable_reference_is_neither() {
        // The soundness question here: getting either wrong silently misclassifies
        // a reference as duplicable-and-droppable, or as owing a linear drop it
        // must never receive.
        let mut refs = Vec::new();
        let shared = intern_ref_type(&mut refs, Type::I64, false);
        let mutable = intern_ref_type(&mut refs, Type::I64, true);
        assert_ne!(shared, mutable);
        assert_eq!(shared.name(), "&i64");
        assert_eq!(mutable.name(), "&!i64");

        assert!(is_copy(shared, &[], &[], &[]));
        assert!(!is_copy(mutable, &[], &[], &[]));
        // Neither is linear: a reference owns nothing, so neither enters move
        // tracking nor owes a disposal.
        assert!(!is_linear(shared, &[], &[], &[]));
        assert!(!is_linear(mutable, &[], &[], &[]));
    }

    #[test]
    fn intern_ref_type_dedups_per_referent_and_mutability() {
        let mut refs = Vec::new();
        let a = intern_ref_type(&mut refs, Type::I64, true);
        let b = intern_ref_type(&mut refs, Type::I64, true);
        let c = intern_ref_type(&mut refs, Type::BOOL, true);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn provenance_interns_one_region_per_parent_and_segment() {
        // The peek route rests on this: two non-consuming projections of one
        // field of one parent must be recognised as one region, or the aliasing
        // they create is invisible.
        let mut prov = Provenance::default();
        let s = prov.fresh_region();
        let other = prov.fresh_region();
        assert_ne!(s, other);
        assert_eq!(prov.field_region(s, "a"), prov.field_region(s, "a"));
        assert_ne!(prov.field_region(s, "a"), prov.field_region(s, "b"));
        assert_ne!(prov.field_region(s, "a"), prov.field_region(other, "a"));
    }

    #[test]
    fn provenance_regions_overlap_along_the_field_chain() {
        // The alias check reads this: a field region is still an alias of
        // its parent (and transitively, its parent's parent), while two
        // fields of unrelated parents share no ancestry at all.
        let mut prov = Provenance::default();
        let s = prov.fresh_region();
        let other = prov.fresh_region();
        let a = prov.field_region(s, "a");
        let ab = prov.field_region(a, "b");
        assert!(
            prov.regions_overlap(s, s),
            "a region always overlaps itself"
        );
        assert!(prov.regions_overlap(s, a), "a field overlaps its parent");
        assert!(prov.regions_overlap(a, s), "overlap is symmetric");
        assert!(
            prov.regions_overlap(s, ab),
            "overlap reaches through a grandparent"
        );
        assert!(
            !prov.regions_overlap(other, a),
            "unrelated parents share no ancestry"
        );
    }

    #[test]
    fn scope_bind_keeps_the_reborrow_and_the_owned_root() {
        // The fix this replaces: a bound reference used to release the place
        // it was reborrowed from, which silently dropped protection for a
        // reborrow of a reference *parameter* (no `owned_root` either, so
        // nothing was left to suspend). Binding must be a no-op on
        // provenance now: what ends a suspension is last-use liveness (6f),
        // not the bind. `Scope::bind` stores `slot.deriv` verbatim (no
        // `Provenance` transform in between), so this asserts that directly.
        let mut prov = Provenance::default();
        let mut scope = Scope::default();
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let fresh = prov.borrow("v", true, span);
        let reborrow = prov.reborrow("r", Some(fresh), true, span);
        let projected = prov.project(Some(reborrow)).expect("a projection");
        assert!(prov.deriv(projected).reborrow, "still suspends `r`");
        assert!(prov.deriv(projected).projected, "R7's note is apt here");
        assert_eq!(prov.deriv(projected).owned_root.as_deref(), Some("v"));

        scope.bind(
            "e",
            Slot::derived(Type::I64, Some(projected)),
            false,
            &mut prov,
        );
        let bound = scope
            .local("e")
            .and_then(|b| b.deriv)
            .expect("a bound deriv");
        assert!(
            prov.deriv(bound).reborrow,
            "`r` stays suspended after binding"
        );
        assert_eq!(
            prov.deriv(bound).owned_root.as_deref(),
            Some("v"),
            "`v` is still borrowed by the local"
        );
    }

    #[test]
    fn provenance_suspension_key_covers_a_reborrow_with_no_owned_root() {
        // The join key: a reborrow of a reference *parameter* has no owned
        // root, so keying the join on `owned_root` alone would make two arms
        // reborrowing two different parameters look identical.
        let mut prov = Provenance::default();
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let p = prov.reborrow("p", None, true, span);
        let q = prov.reborrow("q", None, true, span);
        assert_eq!(prov.deriv(p).owned_root, prov.deriv(q).owned_root);
        assert_ne!(prov.deriv(p).suspension(), prov.deriv(q).suspension());

        // A shared reborrow suspends nothing: `&T` is Copy, so two arms
        // reborrowing different shared parameters still agree.
        let p = prov.reborrow("p", None, false, span);
        let q = prov.reborrow("q", None, false, span);
        assert_eq!(prov.deriv(p).suspension(), prov.deriv(q).suspension());
    }

    #[test]
    fn contains_reference_sees_through_a_struct_field() {
        // The predicate is transitive: a struct that merely *reaches* a
        // reference is rejected wherever a bare one would be.
        let tokens = lex("type: Plain x i64 ;\n").unwrap();
        let module = parse(&tokens).unwrap();
        let mut refs = Vec::new();
        let plain = Type::Struct(StructId::from_index(0), "Plain");
        assert!(!contains_reference(
            plain,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        let mut structs = module.structs;
        structs[0].fields.push((
            "r".to_string(),
            intern_ref_type(&mut refs, Type::I64, false),
        ));
        assert!(contains_reference(
            plain,
            &structs,
            &module.enums,
            &module.arrays
        ));
    }

    #[test]
    fn quotation_effect_unifies_and_binds_variable() {
        // Criterion 2 (R6): a declared `[ 'T -- ]` unified against a concrete
        // `[ i64 -- ]` binds `'T = i64`; an arity mismatch is a located type
        // mismatch, never a silent bind. Exercises `unify_poly_input`'s
        // `PolyType::Quotation` arm directly (the concrete poly path is Phase
        // 2), so deleting the pointwise-row unify makes this fail.
        use crate::ast::quotation_type;
        let sig = PolySig {
            row_in: None,
            inputs: vec![PolyType::Quotation(
                vec![PolyType::Var(0)],
                Vec::new(),
                false,
                None,
                None,
            )],
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: Vec::new(),
            row_var_names: Vec::new(),
        };
        let structs: [StructDecl; 0] = [];
        let enums: [EnumDecl; 0] = [];
        let arrays: [ArrayDecl; 0] = [];
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let mut subst = Subst::default();
        unify_poly_input(
            &sig,
            &sig.inputs[0],
            quotation_type(vec![Type::I64], Vec::new()),
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &mut subst,
        )
        .expect("`[ 'T -- ]` should unify against `[ i64 -- ]`");
        assert_eq!(subst.ty_of(0), Some(Type::I64), "`'T` should bind to `i64`");

        let mut subst2 = Subst::default();
        let err = unify_poly_input(
            &sig,
            &sig.inputs[0],
            quotation_type(vec![Type::I64, Type::I64], Vec::new()),
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &mut subst2,
        )
        .expect_err("an arity mismatch must be a located type mismatch");
        // Slice 10a (R10/R20): pin the *exact* mismatch text. The expected
        // side must render the declared `PolyType` (`[ 'T -- ]`) through
        // `poly_type_str`, never a fabricated `[ -- ]`; a substring like
        // "`f`" would survive that rendering vanishing, so it is not enough.
        assert_eq!(
            err,
            "error: type mismatch: `f` expected `[ 'T -- ]`, found `[ i64 i64 -- ]`",
        );
        assert!(
            subst2.ty_of(0).is_none(),
            "an arity mismatch must not silently bind `'T`"
        );

        // Slice 10a (R10): the `is_quotation_type` let-else arm -- a
        // non-quotation slot against a declared quotation parameter -- routes
        // through the same row-aware renderer, so its expected side is the
        // declared `[ 'T -- ]`, not a fabricated quotation `Type`.
        let mut subst3 = Subst::default();
        let err = unify_poly_input(
            &sig,
            &sig.inputs[0],
            Type::I64,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &mut subst3,
        )
        .expect_err("a non-quotation slot must be a located type mismatch");
        assert_eq!(
            err,
            "error: type mismatch: `f` expected `[ 'T -- ]`, found `i64`",
        );
        assert!(
            subst3.ty_of(0).is_none(),
            "a non-quotation slot must not silently bind `'T`"
        );
    }

    #[test]
    fn poly_type_str_renders_a_quotation_row() {
        // Slice 10a (R10): the row is a separate field on `PolyType::Quotation`,
        // not a slot in `ins`/`outs`, so `poly_type_str` must render it
        // explicitly as the leading element of each side -- dropping that
        // rendering must leave no trace of the row name in the output.
        let sig = PolySig {
            row_in: Some(0),
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: Some(0),
            bounds: Vec::new(),
            ty_var_names: Vec::new(),
            len_var_names: Vec::new(),
            row_var_names: vec!["..s".to_string()],
        };
        let quot = PolyType::Quotation(
            vec![PolyType::Concrete(Type::I64)],
            Vec::new(),
            true,
            Some(0),
            Some(0),
        );
        assert_eq!(poly_type_str(&quot, &sig), "~[ ..s i64 -- ..s ]");
    }

    #[test]
    fn quotation_parameter_is_copy_no_move_obligation() {
        // Criterion 6b: a quotation parameter is `Copy` (it registers no move
        // obligation), so a body that *binds* its quotation param and never
        // consumes it still checks -- forgetting is only an error for a linear
        // value. The body must bind and drop-on-the-floor (`| f |`, not an
        // explicit `drop`), or the `drop` discharges the obligation and the
        // test cannot detect the property it names: making quotation types
        // linear would leave a `drop`-bodied version green.
        check_src(": ignore ( i64 [ i64 -- i64 ] -- i64 ) | f | ;\n")
            .expect("an unused quotation parameter is not a linear-forgetting error");
    }

    #[test]
    fn quotation_taking_word_mints_no_symbol() {
        // U20: a monomorphic quotation-taking word is a combinator, so it is
        // inlined and mints no `IrFunc`; `is_combinator` (the single predicate
        // `check` and `ir::lower` share) recognizes it and excludes an
        // ordinary word. Deleting the `Type::Quotation` clause makes `apply`
        // stop being a combinator and mint a symbol (a link error, since its
        // body is a bare `call` over a phantom).
        let src = ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
                   : plain ( i64 -- i64 ) 1 + ;\n";
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        let apply = module.words.iter().find(|w| w.name == "apply").unwrap();
        let plain = module.words.iter().find(|w| w.name == "plain").unwrap();
        assert!(is_combinator(apply), "`apply` is a combinator (no symbol)");
        assert!(!is_combinator(plain), "`plain` is an ordinary word");
    }

    /// Slice 10a (R11): the bottom-aligned index map, exercised directly on
    /// `back_edge_declared_shape` (a monomorphic effect grounds with no
    /// `Subst`, so no parser is needed). Covers the `times`-shape (empty),
    /// the `while`-shape (1<->1), an asymmetric shape (deepest output <-
    /// deepest carried input), a longer output list (overflow -> `None`), and
    /// a type mismatch at the aligned position (-> `None`).
    #[test]
    fn back_edge_index_map_is_bottom_aligned() {
        let quot = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        fn imap(inputs: Vec<Type>, outputs: Vec<Type>) -> Vec<Option<usize>> {
            use crate::ast::{StackEffect, TypedSlot};
            let w = WordDef {
                name: "w".to_string(),
                effect: StackEffect {
                    inputs: inputs
                        .into_iter()
                        .map(|ty| TypedSlot { name: None, ty })
                        .collect(),
                    outputs: outputs
                        .into_iter()
                        .map(|ty| TypedSlot { name: None, ty })
                        .collect(),
                },
                body: WordBody::Terms { terms: Vec::new() },
                poly: None,
                module: 0,
                span: Span::default(),
            };
            let ctx = Ctx::Line {
                structs: &[],
                enums: &[],
            };
            let mut arrays = Vec::new();
            back_edge_declared_shape(&w, None, "w", Span::default(), &ctx, &mut arrays)
                .unwrap()
                .2
        }
        // `times`-shape: zero fixed outputs -> empty map.
        assert_eq!(
            imap(vec![Type::I64, quot], Vec::new()),
            Vec::<Option<usize>>::new()
        );
        // `while`-shape: one carried in, one out, same type.
        assert_eq!(imap(vec![Type::I64, quot], vec![Type::I64]), vec![Some(0)]);
        // Asymmetric: two carried, one out -> output 0 <- deepest carried.
        assert_eq!(
            imap(vec![Type::I64, Type::I64, quot], vec![Type::I64]),
            vec![Some(0)]
        );
        // More outputs than carried inputs: the overflowing output is `None`.
        assert_eq!(
            imap(vec![Type::I64, quot], vec![Type::I64, Type::I64]),
            vec![Some(0), None]
        );
        // Type differs at the aligned position -> `None`.
        assert_eq!(imap(vec![Type::I64, quot], vec![Type::Str]), vec![None]);
    }

    /// Slice 10a (R11): the recon-4 `my-times` -- which *consumes* its counters
    /// -- used to fail with a spurious `if` branch-depth mismatch, because the
    /// back-edge produced the non-quotation inputs instead of the (empty,
    /// row-only) ground declared outputs. It now checks.
    #[test]
    fn back_edge_produces_ground_declared_outputs() {
        let src = ": my-times ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
                   | f | | to | | from |\n\
                   from to < if\n\
                   from f call\n\
                   from 1 + to f my-times\n\
                   else\n\
                   end ;\n\
                   : main ( -- ) 0 0 5 [ + ] my-times . ;\n";
        check_src(src)
            .expect("my-times checks: the back-edge produces the ground declared outputs");
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
        let src = ": loopy ( ..s 'a i64 ~[ ..s 'a -- ..s ] -- ..s )\n\
                   | f | | n | | acc |\n\
                   n 0 > if\n\
                   acc f call\n\
                   5 n 1 - f loopy\n\
                   else\n\
                   end ;\n\
                   : main ( -- ) \"x\" 3 [ drop ] loopy ;\n";
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
        check_src(": while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;\n")
            .expect("`while` still type-checks after the back-edge rewrite");
    }

    /// Slice 10a (R14): white-box proof that `back_edge_outs` forwards the
    /// surviving capture set along the index map. The witness is an aggregate
    /// carrying an erased quotation (`ty` a struct, `surviving: Some(..)`,
    /// `quot: None`), and the shape yields a `Some(0)` map entry, so the
    /// produced output must inherit the carried input's surviving set --
    /// bypassing `union_surviving`, which a conditional join would otherwise
    /// use to reconstruct the set from a sibling arm and mask a dropped
    /// forward (`d1b3f0a`/`bee407c`).
    #[test]
    fn back_edge_outs_forwards_surviving_set_along_index_map() {
        let set = SurvivingCaptureSetId(0);
        let agg = Type::Struct(crate::ast::StructId::from_index(0), "Agg");
        let carried = vec![Slot {
            surviving: Some(set),
            ..Slot::computed(agg)
        }];
        let ground_outputs = vec![agg];
        let index_map = vec![Some(0)];
        let outs = back_edge_outs(&ground_outputs, &index_map, &carried);
        assert_eq!(
            outs[0].surviving,
            Some(set),
            "the aggregate's surviving capture set must ride across the back-edge"
        );
    }
}
