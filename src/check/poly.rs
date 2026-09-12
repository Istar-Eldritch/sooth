use std::cell::RefCell;
#[cfg(test)]
use std::cmp::Ordering;

use crate::ast::GenericTypes;

use super::*;

// BEGIN poly job-partition modules
mod construction;
mod crosscall;
mod ground;
mod instantiate;
mod overload;
mod r#trait;
mod unify;
use self::construction::*;
use self::crosscall::*;
pub(super) use self::ground::*;
pub(super) use self::instantiate::*;
pub(super) use self::overload::*;
pub(super) use self::r#trait::*;
pub(super) use self::unify::*;
// END poly job-partition modules

/// P7.S3e (R7): a trait-member call recorded abstractly while a polymorphic
/// body is walked -- which trait, which member, on which of the walked word's
/// own type variables. The *symbol* is deliberately absent: `'T` is still
/// abstract here, so only the obligation is knowable. `check_poly_call`
/// resolves it against a concrete `θ` (R8).
///
/// P7b.S2 (S2-16/S2-9): `slots` is the call site's operand record -- the
/// caller-space `PolyType` of each operand the member consumed, in declared
/// order. This is the member-local→caller-slot binding map's carrier: at
/// body-check the caller's slots are still abstract (`App{F, [Var T]}`), so
/// θ_call cannot exist yet; at the resolve loop, where the caller's θ is
/// concrete, each slot re-grounds through θ and the winning member word's own
/// sig unifies against the results, producing θ_call per call site. Slots, not
/// a bare local→slot map, because the winning impl's leftover target slots
/// (`for Result['T 'E]`'s `'E`) bind from the operand's *shape*, which only
/// the full slot type carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraitObligation {
    pub span: Span,
    /// Index into the *walked word's* `PolySig::ty_var_names`.
    pub var: u32,
    pub trait_id: TraitId,
    pub member: String,
    /// P7b.S2 (S2-16/S2-9): the call site's consumed operand slots, caller
    /// space, one per declared member input in declared order (see above).
    pub slots: Vec<PolyType>,
}

/// P7.S3e (R7): the trait-side context a polymorphic body's walk needs --
/// the whole-program trait registry it looks a bound's members up in, and the
/// obligation list it records into. Bundled rather than threaded as two more
/// parameters through the eight functions that already carry
/// `builtin_overloads` along the same path.
pub(crate) struct TraitCtx<'a> {
    pub traits: &'a [TraitDecl],
    pub obligations: &'a mut Vec<TraitObligation>,
    /// P7.S12 (R1.2): the generated struct/enum word call sites recorded
    /// abstractly while this polymorphic body is walked, span -> the header
    /// and (still-abstract) type arguments as `poly_construct_generic` and
    /// `poly_eliminator_call` resolve them. Rides the same struct as
    /// `obligations` for the identical reason: both are per-word records a
    /// call site grounds later against a concrete θ.
    pub enum_sites: &'a mut Vec<(Span, PolyType)>,
    /// P7b.S6 (review fix): the poly-body `^` construction sites recorded
    /// abstractly while this polymorphic body is walked, span -> the
    /// still-abstract payload type. Mirrors `enum_sites` exactly, but for a
    /// different reason: a cell's payload need never appear in the word's
    /// own declared signature (a body-internal temporary, `^ drop`), so
    /// `apply_subst`'s signature walk alone never grounds and interns it.
    /// Recording the site here is what lets a later concrete instantiation
    /// ground it too, exactly as it grounds a declared input/output.
    pub cell_sites: &'a mut Vec<(Span, PolyType)>,
    /// P7.S12 (R1.5): whether the body currently being walked is a
    /// combinator's -- its own generic construction of an enum can vary per
    /// splice, which `enum_sites`/`CallInst::enum_words` (`Span`-keyed, not
    /// `(uid, span)`) cannot represent, so `poly_construct_generic` rejects
    /// that shape here instead of recording it.
    pub is_combinator_splice: bool,
}

/// P7.S3k (R1/R2): the generic-callee side of a polymorphic body's walk --
/// the registry a call to *another* generic word looks its signature up in,
/// and the symbolic cross-call records the walk writes back. Bundled for the
/// same reason `TraitCtx` is: it rides the same eight functions, which
/// already carry `builtin_overloads` and `tctx` along that path.
///
/// This replaces the bare `poly_words: &HashSet<String>` the walk used to
/// carry. That set held callee *names* only, which was enough to name a
/// diagnostic and nothing else; the signature is what a call site needs to
/// dispatch against.
pub(crate) struct CrossCtx<'a> {
    pub env: &'a PolyEnv,
    pub calls: &'a mut Vec<PolyCrossCall>,
}

impl TraitCtx<'_> {
    /// The scratch context for a walk that records no obligation: a unit-test
    /// fixture whose body can carry no `Bound::User` in the first place.
    #[cfg(test)]
    pub(crate) fn scratch<'a>(
        obligations: &'a mut Vec<TraitObligation>,
        enum_sites: &'a mut Vec<(Span, PolyType)>,
    ) -> TraitCtx<'a> {
        TraitCtx {
            traits: crate::ast::predicate_traits(),
            obligations,
            enum_sites,
            cell_sites: Box::leak(Box::new(Vec::new())),
            is_combinator_splice: false,
        }
    }
}

/// P7.S3e (R7/R17): one polymorphic word's recorded obligations, tagged with
/// the identity a call site rediscovers them by. The name alone is not an
/// identity: a single-file build mangles nothing and a polymorphic overload
/// set shares one name across two signatures, so the signature is carried
/// with it -- and since each obligation's `var` indexes *its own* signature's
/// `ty_var_names`, handing a call site another word's obligations would
/// resolve them against the wrong θ silently rather than fail.
#[derive(Debug)]
pub(crate) struct WordObligations {
    pub name: String,
    pub sig: PolySig,
    pub obligations: Vec<TraitObligation>,
}

/// P7.S12 (R1.2): one polymorphic word's recorded generated-enum-word call
/// sites, tagged with the identity a call site rediscovers them by -- the
/// same `(name, sig)` key `WordObligations` uses, and for the identical
/// reason.
#[derive(Debug)]
pub(crate) struct WordEnumSites {
    pub name: String,
    pub sig: PolySig,
    pub sites: Vec<(Span, PolyType)>,
}

/// P7b.S6 (review fix): one polymorphic word's recorded poly-body `^`
/// construction sites, tagged the same `(name, sig)` way `WordEnumSites` is,
/// and for the identical reason.
#[derive(Debug)]
pub(crate) struct WordCellSites {
    pub name: String,
    pub sig: PolySig,
    pub sites: Vec<(Span, PolyType)>,
}

/// P7.S3e (R8): the tables `check_poly_call` resolves a recorded obligation
/// against once θ is concrete -- the trait registry (which the diagnostic for
/// a missing `impl:` reads), the whole-program `impl:` registry, every word's
/// lowering symbol (`ast::overload_symbols`, so a resolved symbol is
/// byte-identical to the one lowering mints), and the obligations themselves.
#[derive(Clone, Copy)]
pub(crate) struct TraitResolveCtx<'a> {
    pub traits: &'a [TraitDecl],
    pub impls: &'a [ImplDecl],
    pub word_symbols: &'a [String],
    /// P7b.S3 (S3-1.c): the whole-program word list `word_symbols` was
    /// computed from. An impl's `resolved` member index recovers the member's
    /// own `WordDef` here, so the `declares_inline` test and the
    /// `poly.combinators` key derive from one object -- never from
    /// `word_symbols[idx]`, whose `overload_symbols` output is `$$`-suffixed
    /// under a name collision and so only coincidentally equals `word.name`.
    pub words: &'a [WordDef],
    pub recorded: &'a [WordObligations],
    /// P7.S12 (R1.2): the generated-enum-word call sites recorded for every
    /// non-combinator polymorphic word, resolved against a concrete θ at
    /// `check_poly_call` the same way `recorded` is.
    pub enum_sites_recorded: &'a [WordEnumSites],
    /// P7b.S6 (review fix): the poly-body `^` construction sites recorded for
    /// every non-combinator polymorphic word, grounded and interned against a
    /// concrete θ the same way `enum_sites_recorded` is.
    pub cell_sites_recorded: &'a [WordCellSites],
}

impl TraitResolveCtx<'_> {
    /// The scratch tables for a walk no `Bound::User` can reach. An empty
    /// `impls` would reject a satisfied bound, so a path that *can* see one --
    /// including native's poly-combinator-standalone check, whose instantiation
    /// records are scratch but whose bounds are real -- must pass the real
    /// tables.
    #[cfg(test)]
    pub(crate) fn scratch() -> TraitResolveCtx<'static> {
        TraitResolveCtx {
            traits: crate::ast::predicate_traits(),
            impls: &[],
            word_symbols: &[],
            words: &[],
            recorded: &[],
            enum_sites_recorded: &[],
            cell_sites_recorded: &[],
        }
    }

    /// The obligations recorded for the callee this call site resolved to.
    /// Empty rather than absent when the callee's body calls no trait member:
    /// the pre-pass records an entry for every non-combinator polymorphic
    /// word, obligations or not, so the two cases are indistinguishable here
    /// and neither is a miss.
    fn obligations_of(&self, name: &str, sig: &PolySig) -> &[TraitObligation] {
        self.recorded
            .iter()
            .find(|w| w.name == name && &w.sig == sig)
            .map(|w| w.obligations.as_slice())
            .unwrap_or(&[])
    }

    /// The generated-enum-word call sites recorded for the callee this call
    /// site resolved to. Mirrors `obligations_of` exactly.
    fn enum_sites_of(&self, name: &str, sig: &PolySig) -> &[(Span, PolyType)] {
        self.enum_sites_recorded
            .iter()
            .find(|w| w.name == name && &w.sig == sig)
            .map(|w| w.sites.as_slice())
            .unwrap_or(&[])
    }

    /// The poly-body `^` construction sites recorded for the callee this call
    /// site resolved to. Mirrors `enum_sites_of` exactly.
    fn cell_sites_of(&self, name: &str, sig: &PolySig) -> &[(Span, PolyType)] {
        self.cell_sites_recorded
            .iter()
            .find(|w| w.name == name && &w.sig == sig)
            .map(|w| w.sites.as_slice())
            .unwrap_or(&[])
    }

    /// P7b.S2 (S2-9): a member word's own `PolySig`, found by lowering symbol
    /// name. Every poly word's body walk records a `WordObligations` entry
    /// keyed by the word's (unmangled) name, so a member word dispatched by a
    /// bound resolution can find its grounded (S2-6) signature here -- the
    /// unification source θ_call is built against. `word_symbols` may carry an
    /// overload `$$N` suffix the recorded name does not, so the lookup strips
    /// one on a second pass (member synth names are unique, so this is belt
    /// and braces, not a live route).
    fn word_sig_of(&self, symbol: &str) -> Option<&PolySig> {
        self.recorded
            .iter()
            .find(|w| w.name == symbol)
            .map(|w| &w.sig)
            .or_else(|| {
                let bare = symbol.split("$$").next()?;
                self.recorded
                    .iter()
                    .find(|w| w.name == bare)
                    .map(|w| &w.sig)
            })
    }
}

/// R7: whether a `PolyType` slot is `Copy`. A bare variable answers *only*
/// from its bound set (never a concrete-type predicate), a concrete slot
/// delegates to `is_copy`, and an array is `Copy` iff its element is.
///
/// P7 slice 3c (R8.3): a slice needs no arm of its own here. Its element is
/// concrete by construction (a generic element is out of scope), so it only
/// ever arrives as `PolyType::Concrete(Type::Slice(..))` and inherits R4's
/// mutability split through the delegation above -- a shared view is `Copy`, a
/// mutable one is not. Pinned by `poly_is_copy_mutable_slice_is_not`.
pub(super) fn poly_is_copy(
    pt: &PolyType,
    sig: &PolySig,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> bool {
    match pt {
        PolyType::Concrete(t) => is_copy(*t, structs, enums, arrays),
        PolyType::Var(v) => sig.has_bound(*v, Bound::Copy),
        PolyType::Array(elem, _) => poly_is_copy(elem, sig, structs, enums, arrays),
        // Slice 6a (D3): a quotation parameter is always `Copy`, so it may be
        // called repeatedly and carries no move obligation.
        PolyType::Quotation(..) => true,
        // P7 slice 3b (R2): a quotation *literal* marker is not a value at
        // all, so it is never `Copy`. `dup`/`over` therefore reject it
        // through `poly_copy_gate`'s own located arm rather than silently
        // minting a second slot pointing at the same interned body.
        PolyType::QuotLit => false,
        // Slice 13 (D3/R-A5): mirrors the monomorphic `is_copy` on
        // `Type::Ref` exactly -- a shared reference is freely duplicated (the
        // exclusivity rule has nothing to protect), a mutable one is not
        // (duplicating it would let two names observe or mutate through one
        // exclusive borrow). The referent's own `Copy`-ness is irrelevant: a
        // `&array['T 4]` is `Copy` even where `array['T 4]` is linear.
        PolyType::Ref(_, mutable) => !*mutable,
        // P7.S3n (R3): mirrors `is_copy` on `Type::OwnedCell` -- always
        // linear regardless of payload, so the payload is not consulted.
        PolyType::OwnedCell(_) => false,
        // P7 slice 3a (D5): conservatively linear -- `Copy`-ness of a
        // generic over variables depends on its arguments' bounds, and a
        // per-argument derivation is a new rule (out of scope for v1); never
        // `Copy` is the conservative answer consistent with the linear spine.
        PolyType::Generic { .. } => false,
        // P7.S12 (R3.1): never `Copy` -- it wraps a possibly-linear payload,
        // and the concrete twin (`Type::Variant`) is never `Copy` either.
        PolyType::GenericVariant { .. } => false,
        // P7b.S1 (S1-16): conservatively linear, mirroring `Generic` --
        // `Copy`-ness of an application depends on its head's binding, which
        // is not resolved here.
        PolyType::App { .. } => false,
    }
}

/// R7 companion to the monomorphic `Scope`/`Moves`: the locals a polymorphic
/// body binds, paired with the move state of the ones that are not `Copy`. A
/// `Copy` local is read freely and never enters `moves`; a non-`Copy` local
/// (a bare variable with no `Copy` bound, or a concrete linear slot) is
/// consumed on its first read, so a second read is use-after-move and never
/// reading it leaks at the word's end (nothing is dropped for you).
#[derive(Debug, Clone, Default)]
pub(super) struct PolyScope {
    locals: HashMap<String, PolyType>,
    moves: Moves,
    /// Slice 13 (R-B5): the prefix borrows this body has taken and not yet
    /// proven dead, in the order they were taken.
    borrows: Vec<PolyBorrow>,
    /// P7 slice 3b (R1): the quotation literals this body has written so far,
    /// the poly twin of `Provenance::quotations`. Append-only and indexed by
    /// `PolyQuotRef`, so it is *not* a parallel stack vector: a slot popped
    /// or shuffled never shrinks it, and an index stays valid for the whole
    /// body. It rides `PolyScope` because that is already `&mut`-threaded
    /// through every walk function, so no stack-threading signature grows a
    /// parameter for it.
    quotations: Vec<PolyQuotLit>,
}

/// P7 slice 3b (R1): one quotation literal encountered in a polymorphic body
/// -- its raw body, the flavour it was written in, and its resolved
/// annotation, whose `variant_tag` is the eliminator arm tag.
#[derive(Debug, Clone)]
pub(super) struct PolyQuotLit {
    body: Vec<Term>,
    span: Span,
    is_inline: bool,
    annot: Option<AnnotEffect>,
}

/// P7 slice 3b (R1): an index into `PolyScope::quotations`, the poly twin of
/// `QuotId`. `Copy`, so a `PolySlot` stays cheap to clone and a `swap` moves
/// the identity with the slot for free (L3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PolyQuotRef(usize);

/// Slice 13 (R-B5): one recorded prefix borrow of a local -- the place, its
/// mutability, and the site, so a later conflict can name the borrow it
/// conflicts with the way the monomorphic `Deriv` does.
#[derive(Debug, Clone)]
pub(super) struct PolyBorrow {
    place: String,
    mutable: bool,
    span: Span,
    /// P7.S3g-follow (1c): whether `place` resolved to a static rather than a
    /// local, decided here at the borrow site because that is the only point
    /// where the answer is reliable. A later lookup cannot reconstruct it: a
    /// `call`-splice or eliminator-arm exit drops the arm's locals from scope
    /// while its borrow records survive, and a local shadowing a static of
    /// the same name resolves to the local here but to the static there.
    static_rooted: bool,
}

/// P7 slice 3b (R1): one entry of the poly walk's virtual stack, replacing
/// the bare `PolyType` plus the parallel `lits: Vec<Option<i64>>` shadow.
/// `int_val` carries exactly what `lits` did (set on `IntLit`, `None`
/// elsewhere, truncated on `Bind`); folding it in here removes the
/// stack/lits length-desync class outright rather than widening it (a third
/// parallel vector for a future `quot` field would only add a second
/// invariant to keep in lock-step).
#[derive(Debug, Clone)]
pub(super) struct PolySlot {
    pub(super) pt: PolyType,
    pub(super) int_val: Option<i64>,
    /// P7 slice 3b (R2): the literal this slot marks, for a slot whose `pt`
    /// is `PolyType::QuotLit`. `None` for every value slot; the two always
    /// agree, which is why the marker is not a value type.
    pub(super) quot: Option<PolyQuotRef>,
}

impl PolySlot {
    fn new(pt: PolyType) -> Self {
        PolySlot {
            pt,
            int_val: None,
            quot: None,
        }
    }

    /// P7 slice 3b (R2): the slot a quotation literal pushes -- the identity
    /// in `quot`, and a `pt` no predicate treats as a value.
    fn quotation(quot: PolyQuotRef) -> Self {
        PolySlot {
            pt: PolyType::QuotLit,
            int_val: None,
            quot: Some(quot),
        }
    }
}

impl PolyScope {
    /// The non-`Copy` locals still holding an unconsumed value, name-sorted so
    /// a body with two of them always reports the same one. A `MaybeMoved`
    /// local (consumed on one `if` arm only) counts as still-unconsumed here,
    /// which is the whole point of tracking three move states (D2).
    fn unconsumed(&self) -> Vec<&str> {
        self.moves.unconsumed()
    }

    /// Slice 13 (R-B5), the conservative borrow liveness OQ1 permits in place
    /// of threading `Provenance`/`Liveness` through the poly walk: a borrow
    /// can only be observed through a *reference value*, and Sooth forbids
    /// storing one anywhere it could outlive the stack (a declared field, a
    /// `fill` element, a `^` payload are all rejected outright), so once no
    /// stack slot and no local holds a reference, every borrow this body has
    /// taken is provably dead and is forgotten here.
    ///
    /// Coarser than the monomorphic per-place `live_deriv`: one unrelated
    /// live reference keeps *every* recorded borrow alive, so a rejection can
    /// be a conservative false positive (pinned by
    /// `poly_borrow_liveness_is_coarse_across_places`). It never misses a
    /// hazard, which is the locked minimum -- a live borrow is never pruned.
    fn prune_dead_borrows(&mut self, stack: &[PolySlot]) {
        if self.borrows.is_empty() {
            return;
        }
        let reachable = stack
            .iter()
            .map(|slot| &slot.pt)
            .chain(self.locals.values())
            .any(is_reference_slot);
        if !reachable {
            self.borrows.clear();
        }
    }

    /// The most recent live borrow of `place` a new borrow (or naming) would
    /// conflict with: any borrow when `mutable_only` is false (the direction a
    /// new `&!` takes), a mutable one otherwise (what a new `&` conflicts
    /// with). Call `prune_dead_borrows` first; a record still here is live.
    fn live_borrow_of(&self, place: &str, mutable_only: bool) -> Option<&PolyBorrow> {
        self.borrows
            .iter()
            .rev()
            .find(|b| b.place == place && (b.mutable || !mutable_only))
    }

    /// P7 slice 3b (R1): record one quotation literal and hand back its
    /// index. Append-only, so every index already handed out stays valid --
    /// including across the per-arm clones `poly_eliminator_call` makes.
    fn intern_quotation(&mut self, lit: PolyQuotLit) -> PolyQuotRef {
        self.quotations.push(lit);
        PolyQuotRef(self.quotations.len() - 1)
    }

    fn quotation(&self, quot: PolyQuotRef) -> &PolyQuotLit {
        &self.quotations[quot.0]
    }
}

/// Whether a `PolyType` slot holds a reference: a poly one (`&array['T 4]`, from a
/// body borrow) or a fully concrete one (`&array[i64 4]`, from a declared input).
/// Both keep a borrow observable, so both count for `prune_dead_borrows`.
///
/// P7 slice 3c (R8.3): a slice counts too, and needs no arm of its own -- it
/// arrives as `Concrete(Type::Slice(..))` and `Type::is_ref` reports it (R1.4).
/// That is the answer `prune_dead_borrows` wants: a live view keeps the borrow
/// it was built from observable exactly as a `&T` does.
fn is_reference_slot(pt: &PolyType) -> bool {
    match pt {
        PolyType::Ref(..) => true,
        PolyType::Concrete(t) => t.is_ref(),
        PolyType::Var(_)
        | PolyType::Array(..)
        | PolyType::Quotation(..)
        | PolyType::OwnedCell(_) => false,
        // P7 slice 3b (R2): not a value type, so it holds nothing, least of
        // all a reference that would keep a borrow observable.
        PolyType::QuotLit => false,
        // P7 slice 3a: a generic application never denotes a reference
        // itself (a reference nested inside one is D5's out-of-scope depth,
        // or, if concrete, was already rejected by the audits below).
        PolyType::Generic { .. } => false,
        // P7.S12 (R3.1): a variant is not a reference itself; a reference
        // nested inside a field is unreachable through this slot.
        PolyType::GenericVariant { .. } => false,
        // P7b.S1 (S1-16): a higher-kinded application never denotes a
        // reference itself, mirroring `Generic`.
        PolyType::App { .. } => false,
    }
}

/// R14-R17: check a polymorphic combinator standalone by instantiating its
/// signature at concrete stand-in types and running the ordinary concrete
/// checker on the body. `i64` is Copy/Ord/numeric, so a body that only moves,
/// reads, and hands an element to its quotation parameter checks exactly as it
/// will at every Copy instantiation; the abstract `call`/`times` paths (R8/R9)
/// type `f call`/`f times` against the declared effect, and the three `times`
/// obligations (R16) fall out of the ordinary `times` check at the def site.
/// Instantiating every type variable at the same `i64` cannot mask a real
/// error the library relies on: the combinators never combine two distinct
/// element/accumulator variables directly (that arithmetic lives in the
/// caller's literal), and a type-specific misuse in some other combinator's
/// body is caught at its concrete splice site, the same place obligation 2's
/// borrow re-check lands (D4/R21). The array length is irrelevant to type
/// checking (`times` supplies a runtime index), so any value serves.
///
/// P7.S11: signature grounding (R1/R2/R3/R6) can mint a monomorph of a
/// generic header the declared output (or a quotation-input effect, an array
/// element, a referent or a cell payload) applies. The mint, and every shape
/// it interns along the way, goes through `ground_into_word_scoped_registries`
/// and lands only in word-scoped copies (`WordScopedRegistries`): this pass
/// records nothing that survives it -- no generic monomorph, no instantiation
/// record, and no interned array/cell/ref/slice shape -- every registry it
/// writes to is a word-scoped copy discarded when the check returns. A
/// top-level generic *input* slot is still rejected before grounding starts
/// (R3): only a slot the signature's own output side applies, or one nested
/// inside a quotation effect/array/reference/cell, is admitted.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_poly_combinator_standalone(
    word: &WordDef,
    sig: &PolySig,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    slices: &[SliceDecl],
    structs: &[StructDecl],
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    poly: &mut PolyCtx,
    generics: Option<&RefCell<GenericTypes>>,
) -> Result<(), String> {
    const STANDALONE_LEN: u32 = 4;
    let span = word_span(word);
    let mut subst = Subst::default();
    // P7b.S3 (S3-6, gate G1): an Arrow-kinded variable gets a *constructor*
    // stand-in, not `Type::I64`. `i64` is not a constructor, so an App-headed
    // declared slot (`'F['T 'E]`) had no representable stand-in operand at
    // all and `apply_subst`'s App arm rightly refused it. Seeded with the
    // `CtorImage` of the first `impl:` of the variable's user bound in
    // declaration order, the slot grounds to a real monomorph and the body is
    // checked against a representative instance -- stronger than the `i64`
    // stand-in, and deterministic.
    let mut stand_in_ctor = false;
    for v in 0..sig.ty_var_names.len() as u32 {
        let arrow = matches!(
            sig.ty_kinds.get(v as usize),
            Some(crate::ast::Kind::Arrow { .. })
        );
        let ty = match arrow {
            false => Type::I64,
            true => match arrow_stand_in(v, sig, poly.trait_resolve.impls, generics) {
                Some(t) => {
                    stand_in_ctor = true;
                    t
                }
                // The named hole (ledger item 2): the bound has no `impl:` in
                // this program, the Arrow variable carries no user bound at
                // all, or every impl of the bound arity-mismatches the
                // Arrow kind so `arrow_stand_in` finds no viable stand-in
                // (pinned by `arity_mismatched_first_impl_is_not_the_stand_in`).
                // The standalone check is skipped and the word is checked only
                // at its splice sites. The no-impl half is unwitnessable by
                // construction (with no impl, no call site can discharge the
                // bound, so the word is never spliced); the no-user-bound half
                // is still callable, and error golden E#1 pins that its broken
                // body is rejected at its first splice site.
                None => return Ok(()),
            },
        };
        subst.ty.push((v, ty));
    }
    for ln in 0..sig.len_var_names.len() as u32 {
        subst.len.push((ln, STANDALONE_LEN));
    }
    // R3: a declared top-level generic input slot is not yet groundable at a
    // combinator's standalone check -- unlike a nested one (a quotation
    // effect's rows, an array element, a referent, a cell payload), which
    // grounds normally below. Checked before any grounding starts, so it
    // never depends on whether grounding would have succeeded.
    //
    // P7b.S3 (S3-9): skipped for a synthesized trait member word. Its inputs
    // are *always* this shape once the `impl:` target is generic (S2-6's
    // bare-ctor desugar binds the trait's header variable to the whole target
    // pattern), so the refusal rejects every generic-target member out of
    // hand -- gate G4. The refusal is unchanged for every other word.
    for pty in sig.inputs.iter() {
        if !word.is_trait_member
            && matches!(
                pty,
                PolyType::Generic { .. } | PolyType::GenericVariant { .. }
            )
        {
            let reject_ctx = word_ctx(
                word,
                structs,
                enums,
                statics,
                modules,
                poly.combinators.tail(),
                None,
            );
            return Err(poly_generic_not_yet_groundable_error(
                &reject_ctx,
                span,
                &word.name,
                &poly_type_str(pty, sig),
            ));
        }
    }
    let grounded = ground_into_word_scoped_registries(
        generics,
        structs,
        enums,
        arrays,
        cells,
        refs,
        slices,
        |scratch, local_arrays, local_cells, local_refs| {
            let ctx = word_ctx(
                word,
                structs,
                enums,
                statics,
                modules,
                poly.combinators.tail(),
                scratch,
            );
            let mut inputs = Vec::with_capacity(sig.inputs.len());
            for pty in &sig.inputs {
                let ty = apply_subst(
                    sig,
                    pty,
                    &subst,
                    &word.name,
                    span,
                    &ctx,
                    local_arrays,
                    local_cells,
                    local_refs,
                )?;
                inputs.push(TypedSlot { name: None, ty });
            }
            let mut outputs = Vec::with_capacity(sig.outputs.len());
            for pty in &sig.outputs {
                let ty = apply_subst(
                    sig,
                    pty,
                    &subst,
                    &word.name,
                    span,
                    &ctx,
                    local_arrays,
                    local_cells,
                    local_refs,
                )?;
                outputs.push(TypedSlot { name: None, ty });
            }
            Ok((inputs, outputs))
        },
    );
    // P7b.S3 (S3-6): the rescue, first class -- a **grounding** failure
    // against the stand-in's constructor. The stand-in is a representative,
    // not a proof obligation: a word whose body is fine at some other impl's
    // constructor must not be rejected because the first impl in declaration
    // order does not fit it. Structural, not text-matched: this is the
    // signature-grounding step, so any failure of it while a `CtorImage`
    // stand-in is in play is that class by construction. With the `i64`
    // stand-in (`stand_in_ctor == false`) the `?` is byte-identical to before.
    //
    // Honest about the shape of this arm (Phase 2 review fix): the `Err(_)`
    // is positional and blanket, not classified by cause -- it rescues
    // *anything* the closure above can raise, not provably only a
    // `CtorImage`-mismatch. The closure calls `apply_subst` once per
    // declared input/output slot, so a single Arrow variable carrying a
    // `CtorImage` stand-in arms this rescue for every slot's grounding, not
    // only the one(s) that actually mention that variable. Measured (see
    // `class_one_grounding_failure_is_skipped`): no legally-parsed fixture
    // was found that makes this arm's `Err` branch fire at all, given
    // `arrow_stand_in`'s own arity-fit check and the shared kind check that
    // rejects a bare Arrow-bound variable outright -- so today it is an
    // untriggered blanket, not a narrow one.
    let ((inputs, outputs), mut local) = match grounded {
        Ok(v) => v,
        Err(_) if stand_in_ctor => return Ok(()),
        Err(e) => return Err(e),
    };
    // P7.S11 (R4): if signature grounding minted a monomorph, its generated
    // constructor/destructure sigs are not yet in `env` -- `env` was built
    // from `module.structs`/`module.enums` before the word loop, and the
    // mint lives only in `local.structs`/`local.enums`. Extend a clone with
    // exactly the newly flushed tail (the base decls' own sigs are already
    // in `env`; re-appending them would double every constructor overload),
    // so a body term naming one of those variants resolves.
    let mut local_env;
    let env: &HashMap<String, Vec<Overload>> =
        if local.enums.len() > enums.len() || local.structs.len() > structs.len() {
            local_env = env.clone();
            let struct_skip = struct_generated_sigs(structs).len();
            for (name, symbol, module, sig) in struct_generated_sigs(&local.structs)
                .into_iter()
                .skip(struct_skip)
            {
                local_env.entry(name).or_default().push(Overload {
                    sig,
                    symbol,
                    module,
                });
            }
            let enum_skip = enum_generated_sigs(enums).len();
            for (name, symbol, module, sig) in enum_generated_sigs(&local.enums)
                .into_iter()
                .skip(enum_skip)
            {
                local_env.entry(name).or_default().push(Overload {
                    sig,
                    symbol,
                    module,
                });
            }
            let variant_skip = variant_generated_sigs(enums).len();
            for (name, symbol, module, sig) in variant_generated_sigs(&local.enums)
                .into_iter()
                .skip(variant_skip)
            {
                local_env.entry(name).or_default().push(Overload {
                    sig,
                    symbol,
                    module,
                });
            }
            &local_env
        } else {
            env
        };
    let terms = &word.body;
    let terms = terms.clone();
    // A concrete stand-in for the combinator, checked by the ordinary path.
    let concrete = WordDef {
        name: word.name.clone(),
        effect: StackEffect { inputs, outputs },
        body: terms,
        poly: None,
        declares_inline: word.declares_inline,
        module: word.module,
        span: word.span,
        declared_globals: word.declared_globals.clone(),
        is_trait_member: word.is_trait_member,
    };
    let mut dropped = Vec::new();
    // P7.S3o Phase 3: thread the combinator's own `PolySig` and the i64
    // stand-in θ into the standalone body walk, so a bare trait member call
    // in the body resolves against the i64 stand-in (the same dispatch
    // injection `check_term` uses at real splice sites). The resolved symbol
    // is scratch (the standalone check never lowers); only the stack-effect
    // accounting matters here — the member call is re-checked at each real
    // splice site where θ is concrete. Saved and restored so the enclosing
    // context is unaffected.
    let saved_comb_sig = poly.combinator_sig.take();
    let saved_comb_subst = poly.combinator_subst.take();
    let saved_comb_name = poly.combinator_name.take();
    poly.combinator_sig = Some(sig.clone());
    poly.combinator_subst = Some(subst.clone());
    poly.combinator_name = Some(word.name.clone());
    let result = check_word(
        &concrete,
        &local.enums,
        env,
        &mut local.arrays,
        &mut local.cells,
        &mut local.refs,
        &mut local.slices,
        &local.structs,
        statics,
        modules,
        &mut dropped,
        poly,
        None,
        0,
    );
    poly.combinator_sig = saved_comb_sig;
    poly.combinator_subst = saved_comb_subst;
    poly.combinator_name = saved_comb_name;
    // P7b.S3 (S3-6): the rescue, second class -- a body failure the stand-in's
    // arbitrary constructor can *cause*, identified by the tag its raise site
    // stamped ([`STAND_IN_GROUNDING_TAG`]), never by the message's wording.
    // Every other body failure is impl-independent -- arity/underflow,
    // linearity, an unknown word, a borrow or move violation, an undischarged
    // bound -- and stays a hard error. A rescued word is not silently
    // accepted: it is re-checked at every splice site, so a genuinely broken
    // body fails at its first splice.
    // The live member-call producer (Phase 3, S3-7): `splice_member_hkt_error`
    // is deleted, so an App-headed member slot grounds through
    // `ground_member_sig_via_theta`, whose failures against a `CtorImage`
    // stand-in raise the tagged `splice_member_ctor_image_error` and land
    // here (see `class_two_member_call_is_rescued_for_recheck_at_the_splice`).
    match result {
        Err(e) if stand_in_ctor => match strip_stand_in_tag(e) {
            (true, _) => Ok(()),
            (false, e) => Err(e),
        },
        other => other,
    }
}

/// R7: check a polymorphic word's body once, over a virtual stack of
/// `PolyType` (never the concrete `Slot` stack, S1/R4). Seeded from the
/// declared fixed inputs; the input row variable is an opaque below-stack
/// marker (the stack beneath the fixed inputs is passed through untouched, so
/// nothing is pushed for it and the residual stack is compared against the
/// declared fixed outputs). A bare variable supports only the five shuffles,
/// an operation its bound set permits (`dup`/`over` need `Copy`, the
/// comparisons need `Ord`), local bind/read, and being returned; every other
/// type-directed operation on it is a located error naming the variable, so a
/// body a real instantiation would reject can never slip through.
#[allow(clippy::too_many_arguments)]
pub fn check_poly_body(
    word: &WordDef,
    sig: &PolySig,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    generics: Option<&RefCell<GenericTypes>>,
) -> Result<(), String> {
    // R12 (slice 8b, 8a): the caller module's operator visibility rides on
    // `ctx`, so a bare operator in a poly body resolves against the same
    // scoped candidate set a concrete body does. `check::check` is the only
    // caller and it passes `Some`; the `Option` is the shared parameter
    // shape, not a live unscoped path.
    //
    // P7.S3g-follow (1a): the *populated* tail index, so
    // `ctx.is_self_tail_call()` answers for a generic body what it answers
    // for a concrete one -- whether this word back-edges at all. It is the
    // word-level half of the back-edge guard below (`poly_call_term`'s
    // self-call arm); the per-term half is the `tail` flag threaded from
    // here through the walk.
    //
    // P7 slice 3a phase 2 (R2): rebased here, at the top of this one body's
    // check, to the live registries' *current* length -- this function's
    // caller (`check::check`) flushes right after it returns, so every mint
    // this body's own construction/grounding triggers counts from the
    // correct, current base regardless of how many earlier words already
    // minted.
    if let Some(cell) = generics {
        cell.borrow_mut().rebase(structs.len(), enums.len());
    }
    let ctx = word_ctx(
        word,
        structs,
        enums,
        statics,
        modules,
        combinators.tail(),
        generics,
    );
    let terms = &word.body;
    let stack: Vec<PolySlot> = sig.inputs.iter().cloned().map(PolySlot::new).collect();
    let mut scope = PolyScope::default();
    let residual = poly_walk(
        terms,
        stack,
        &mut scope,
        sig,
        &ctx,
        env,
        combinators,
        structs,
        enums,
        arrays,
        cells,
        refs,
        slices,
        builtin_overloads,
        tctx,
        cross,
        true,
    )?;
    // P7 slice 3b (R4/L2): splice-consumed quotations only. A literal still
    // on the stack here would have to *be* a value to leave the word, and it
    // has no runtime representation in a generic body. Checked ahead of the
    // output comparison so the diagnostic names the real problem rather than
    // reporting the marker as a stack-shape mismatch.
    if let Some(quot) = residual.iter().find_map(|slot| slot.quot) {
        return Err(poly_quotation_not_consumed_error(
            &ctx,
            scope.quotation(quot).span,
        ));
    }
    let residual_pt: Vec<PolyType> = residual.into_iter().map(|slot| slot.pt).collect();
    if residual_pt != sig.outputs {
        return Err(poly_output_mismatch_error(word, sig, &residual_pt));
    }
    // A non-`Copy` local never read still holds its value here; nothing is
    // auto-dropped, so it leaks. The monomorphic sibling rejects the same
    // shape at `leave_block`; the residual check above cannot see a value
    // parked in a local.
    if let Some(local) = scope.unconsumed().first().map(|s| s.to_string()) {
        let pt = scope.locals[&local].clone();
        return Err(poly_local_unconsumed_error(word, sig, &local, &pt));
    }
    Ok(())
}

/// P7.S12 (R2.2): the eliminator registry over both the monomorphized `enums`
/// this walk carries and the live instantiator's own generic headers, so a
/// call to `Option?` gates even where nothing instantiates `Option`
/// concretely. `Ctx::generics` is `None` on the REPL and stand-in paths, where
/// no generic header exists to name; the borrow is released here, before any
/// caller can re-enter the instantiator.
fn ctx_eliminator_registry(ctx: &Ctx, enums: &[EnumDecl]) -> HashMap<String, EliminatorTarget> {
    match ctx.generics() {
        Some(cell) => eliminator_registry(enums, &cell.borrow().enums),
        None => eliminator_registry(enums, &[]),
    }
}

/// `tail` marks this term sequence as occupying its word's tail position, so
/// its final term sits on the self-tail-call back-edge -- the poly twin of
/// `check_terms_relaxed`'s own `tail`, computed per term the same way
/// (`tail && at == last`) and threaded into a spliced body or a tail-called
/// arm unchanged. Read only by `poly_call_term`'s self-call arm.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_walk(
    terms: &[Term],
    mut stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    tail: bool,
) -> Result<Vec<PolySlot>, String> {
    let last = terms.len().wrapping_sub(1);
    // P7 slice 3b (R2): the same written-adjacency rule the concrete path
    // applies to a variant-tagged literal. A tag is only meaningful as
    // arm-to-variant routing, so a tagged literal that no eliminator call
    // collects is never checked against anything -- and admitting quotation
    // literals here is exactly what would let one through. Applied over this
    // term list, so an arm body re-entering `poly_walk` is held to it too.
    let eliminators = ctx_eliminator_registry(ctx, enums);
    for (at, term) in terms.iter().enumerate() {
        if let TermKind::Quotation(_, _, Some(annot)) = &term.kind {
            if let Some(tag) = &annot.variant_tag {
                match tagged_literal_reaches_an_eliminator_call(terms, at, &eliminators) {
                    EliminatorArmDest::Reached => {}
                    EliminatorArmDest::NotAdjacent => {
                        return Err(eliminator_arm_outside_call_error(
                            ctx, annot.span, &tag.name,
                        ));
                    }
                    // P7.S12 (R7.2): its own message, not the adjacency one.
                    EliminatorArmDest::NamesNoEliminator(call) => {
                        return Err(eliminator_arm_names_no_eliminator_error(
                            ctx, annot.span, &tag.name, &call,
                        ));
                    }
                }
            }
        }
        stack = poly_term(
            term,
            stack,
            scope,
            sig,
            ctx,
            env,
            combinators,
            structs,
            enums,
            arrays,
            cells,
            refs,
            slices,
            builtin_overloads,
            tctx,
            cross,
            tail && at == last,
        )?;
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn poly_term(
    term: &Term,
    mut stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    tail: bool,
) -> Result<Vec<PolySlot>, String> {
    let span = term.span;
    match &term.kind {
        TermKind::IntLit(n) => {
            stack.push(PolySlot {
                pt: PolyType::Concrete(Type::I64),
                int_val: Some(*n),
                quot: None,
            });
        }
        TermKind::FloatLit(_) => {
            stack.push(PolySlot::new(PolyType::Concrete(Type::F64)));
        }
        TermKind::StrLit(_) => {
            stack.push(PolySlot::new(PolyType::Concrete(Type::Str)));
        }
        TermKind::Bind(names) => {
            if stack.len() < names.len() {
                let op = format!("| {} |", names.join(" "));
                return Err(underflow_error(ctx, span, &op, names.len(), stack.len()));
            }
            // R4 twin of the monomorphic binder: a duplicate name inside this
            // one bind group would orphan the earlier binding, and re-binding a
            // name still in scope from an earlier group would do the same; a
            // non-`Copy` value parked in either could then never be consumed (a
            // silent leak).
            let mut seen = HashSet::new();
            for name in names {
                reject_variant_local(ctx, name, "local")?;
                reject_duplicate_local(ctx, name, span, &mut seen)?;
                // D5, poly coverage: builtins and `env` (bare and mangled)
                // only. `poly_term` has no `PolyCtx`, so `poly.env`/
                // `poly.combinators` are unreachable here (recorded gap, D5).
                let mangled = crate::resolve::mangle(name, span.module);
                let collides = is_builtin_word_name(name)
                    || env.contains_key(name)
                    || env.contains_key(&mangled);
                if collides {
                    return Err(callable_local_error(ctx, name, span));
                }
                if scope.locals.contains_key(name) {
                    return Err(rebound_local_error(ctx, span, name));
                }
            }
            let bound = stack.split_off(stack.len() - names.len());
            // A bound local's own literal-ness is not tracked (D6/R-B3 only
            // needs a literal that is still the immediate top of stack); a
            // local read back later carries no int value, same as any other
            // computed slot.
            for (name, slot) in names.iter().zip(bound) {
                let pt = slot.pt;
                // A non-`Copy` binding carries a consume-exactly-once
                // obligation tracked in `moves`; a `Copy` one does not.
                if !poly_is_copy(&pt, sig, structs, enums, arrays) {
                    scope.moves.states.insert(name.clone(), MoveState::Live);
                }
                scope.locals.insert(name.clone(), pt);
            }
        }
        TermKind::Call(name, type_args, len_args) => {
            // P7.S3t (R1/R3): a call inside a polymorphic word's own body is
            // checked symbolically -- there is no `Subst` here to seed, and
            // reaching one would be the multi-hop forwarding case R7 leaves
            // out of the slice. Rejected rather than dropped: a dropped list
            // links whatever the symbolic path resolved instead. P7.S6b (R2a):
            // an explicit length list is rejected the same way.
            if !type_args.is_empty() || !len_args.is_empty() {
                return Err(type_arguments_in_poly_body_error(
                    ctx,
                    span,
                    name,
                    !type_args.is_empty(),
                    !len_args.is_empty(),
                ));
            }
            return poly_call_term(
                name,
                span,
                stack,
                scope,
                sig,
                ctx,
                env,
                combinators,
                structs,
                enums,
                arrays,
                cells,
                refs,
                slices,
                builtin_overloads,
                tctx,
                cross,
                tail,
            );
        }
        // P7 slice 3b (R2): a quotation literal is admitted, interned, and
        // marked on the stack -- the identity rides `PolySlot::quot` and the
        // `pt` is `PolyType::QuotLit`, which is not a value type, so the
        // literal can only ever be consumed by an in-body eliminator (L2). An
        // annotation is resolved here, at the interning site, exactly as the
        // concrete path resolves it: an eliminator arm's `( Rect )` carries
        // no rows, so the resolution is the same one, and its `variant_tag`
        // is what `poly_eliminator_call` matches arms by.
        TermKind::Quotation(body, is_inline, annot) => {
            let annot = match annot {
                Some(annot) => Some(resolve_annotation(ctx, annot)?),
                None => None,
            };
            let quot = scope.intern_quotation(PolyQuotLit {
                body: body.clone(),
                span,
                is_inline: *is_inline,
                annot,
            });
            stack.push(PolySlot::quotation(quot));
        }
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn poly_call_term(
    name: &str,
    span: Span,
    mut stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    tail: bool,
) -> Result<Vec<PolySlot>, String> {
    // A named local reads back its bound `PolyType`. A non-`Copy` local is
    // consumed on read (R3/D2): a second read is use-after-move, exactly as
    // the monomorphic checker treats a linear local; a `Copy` local carries no
    // such obligation and is absent from `moves`.
    if let Some(pt) = scope.locals.get(name).cloned() {
        // Slice 13 (R-B5), the direction symmetric with the check at the
        // borrow: reading a local a live borrow already reaches. Consuming it
        // (a non-`Copy` local, moved by this read) would leave that borrow
        // aimed at storage its owner gave away; merely naming it while a
        // mutable borrow is live makes that borrow's mutation silently
        // observable through a second name. Checking only at the borrow
        // catches `a ... &!a` and misses `&!a ... a`, the same hazard with
        // the two terms swapped.
        scope.prune_dead_borrows(&stack);
        let consumes = !poly_is_copy(&pt, sig, structs, enums, arrays);
        if let Some(live) = scope.live_borrow_of(name, !consumes) {
            let ty = poly_type_str(&pt, sig);
            return Err(if consumes {
                poly_consume_of_borrowed_place_error(ctx, span, name, &ty, live)
            } else {
                poly_naming_aliases_borrowed_place_error(ctx, span, name, live)
            });
        }
        scope
            .moves
            .take(name, span)
            .map_err(|site| poly_use_after_move_error(ctx, span, name, site))?;
        stack.push(PolySlot::new(pt));
        return Ok(stack);
    }
    let need = |n: usize, holds: usize| underflow_error(ctx, span, name, n, holds);
    // Review finding 3 (P7.S3e round-4): bound-directed dispatch (R7/R10)
    // must front every name-based special case below it, not just the
    // ordinary `env` lookup decision 7 originally partitioned against --
    // otherwise a trait member sharing a name with a builtin (`eq`, `len`,
    // `call`, `dup`, ...) is unreachable through its bound, silently running
    // the builtin or failing with a diagnostic that never mentions the
    // trait. `poly_trait_member_call` is a narrow, self-gating probe: it
    // returns `Ok(None)` unless a `Bound::User` on one of this word's own
    // type variables declares a member of this name (P7.S3p: found by name
    // over the bounds, not by matching the stack top).
    //
    // Which names a bound can legally claim here, since P7.S3p selects on
    // name alone and so cannot fall through to the builtin on a shape
    // mismatch:
    //   - `len`/`dup`/`branch`/`tag`/`add`/... are in `BUILTIN_WORDS`, and
    //     `call`, which is not, is special-cased beside it: all are rejected
    //     as member names at `trait:` declaration time, so no bound declares
    //     one and no builtin arm below is shadowed.
    //   - `@`/`!`/`+!` are rejected as member names by the parser's
    //     `ACCESS_WORDS` gate.
    //   - the six surface comparisons (`eq`, `lt`, ...) are legal member
    //     names, and a bound *does* claim them here. That is intended: they
    //     are `lib/` words, not builtins, and a body that imports one
    //     receives it mangled, so the two spellings never collide.
    // Every other user word arrives mangled, never under a bare builtin
    // spelling.
    //
    // It also must run ahead of the intrinsic-import gate immediately below:
    // bound dispatch is whole-program and unscoped by import (decision 9),
    // so a bound call to e.g. `eq` must not be rejected as an unimported
    // comparison intrinsic before dispatch even gets a chance to try it.
    if let Some(next) = poly_trait_member_call(name, span, &mut stack, sig, ctx, tctx)? {
        return Ok(next);
    }
    // P8 S2 (R2): the poly-body twin of `check_term`'s intrinsic-import gate.
    // A generic body dispatches the same builtins on its own path, so without
    // this an unimported `dup`/`add` would be gated in a monomorphic word and
    // free in a polymorphic one.
    //
    // The bare spelling reaching here is always the builtin's own: a word the
    // module declared under that name arrives mangled (`dup__m0`), which
    // `is_name_dispatched_builtin` does not match, and the two un-mangled
    // categories are not in `env` under the bare name either (an operator
    // decl is keyed mangled, and a user `drop` is type-directed, never an
    // `env` entry). So there is no candidate to defer to and nothing to check
    // `env` for.
    if intrinsic_is_gated_out(ctx, span, name) {
        return Err(ungated_intrinsic_error(ctx, span, name));
    }
    // R-B1 (slice 13): every `&`-led word (the prefix borrow and the
    // reference accessor family) fronts the rest of dispatch, mirroring
    // `check_reference_word`'s own position ahead of the monomorphic
    // family. `Ok(None)` (not `&`-led) falls through unchanged.
    if let Some(next) = poly_reference_word(
        name, span, &mut stack, scope, sig, ctx, structs, enums, arrays, slices,
    )? {
        return Ok(next);
    }
    // Slice 13 (R-B4): `@` fetches a `Copy` referent through any reference,
    // shared or mutable -- there is no `&!T -> &T` demotion to write, so both
    // mutabilities are typed identically here.
    if name == "@" {
        let top = stack.last().ok_or_else(|| need(1, stack.len()))?.pt.clone();
        let PolyType::Ref(referent, _) = &top else {
            return Err(poly_op_on_variable_error(ctx, span, "@", &top, sig));
        };
        poly_copy_gate(referent, "@", sig, ctx, span, structs, enums, arrays)?;
        let out = (**referent).clone();
        stack.pop();
        stack.push(PolySlot::new(out));
        return Ok(stack);
    }
    // Slice 13 (R-B4): `!` stores a `Copy` value through a *mutable*
    // reference, `( &!T T -- )`. A shared receiver is a mutability mismatch
    // rendered off the receiver's own referent, exactly as `&>`'s is.
    if name == "!" {
        let n = stack.len();
        if n < 2 {
            return Err(need(2, n));
        }
        let receiver = stack[n - 2].pt.clone();
        let value = stack[n - 1].pt.clone();
        let PolyType::Ref(referent, mutable) = &receiver else {
            return Err(poly_op_on_variable_error(ctx, span, name, &receiver, sig));
        };
        if !*mutable {
            return Err(poly_rendered_type_mismatch_error(
                ctx,
                span,
                name,
                &poly_type_str(&PolyType::Ref(referent.clone(), true), sig),
                &poly_type_str(&receiver, sig),
            ));
        }
        // The referent is overwritten, so whatever was there is forgotten:
        // only a `Copy` referent may be stored into, or the old value's drop
        // obligation would vanish with it (the monomorphic `!` gates the
        // same way).
        poly_copy_gate(referent, name, sig, ctx, span, structs, enums, arrays)?;
        if **referent != value {
            return Err(poly_rendered_type_mismatch_error(
                ctx,
                span,
                name,
                &poly_type_str(referent, sig),
                &poly_type_str(&value, sig),
            ));
        }
        stack.truncate(n - 2);
        return Ok(stack);
    }
    // Slice 13 (R-B6): `+!` never lands in a generic body, so it is a located
    // error rather than an unknown-word one now that `!` is recognised.
    if name == "+!" {
        return Err(poly_unsupported_accessor_error(ctx, span, name));
    }
    // P7b.S6 (R3/R7): the poly-body twin of `check_owned_cell_word`'s `^`/
    // `^>` -- needed so a self-referencing generic constructor
    // (`^List['T]`) can be built and walked from inside a poly body (a
    // recursive `impl:` member over `List['T]`, M4's own exit criterion).
    // `^|>`'s Copy-payload peek has no fixture needing it this slice and is
    // left unrouted, same as `poly_reference_word`'s treatment of `&^`.
    //
    // Review fix (Phase 3, P2): no `consumed_place_conflict` check guards
    // `^`'s consumption of `stack[n - 1]` below, unlike the mono path's `^`
    // arm. This is not a gap: the poly checker has no stack-position
    // provenance/liveness system over anonymous `PolySlot`s at all (see
    // `PolyScope`'s `moves`/`borrows`, both keyed by local *name*, not stack
    // position) -- `drop`, the plain consuming word right above this one,
    // has no such check either. A value born from a live-borrowed local is
    // already caught earlier, when that local is read by name
    // (`scope.live_borrow_of` above in this function); by the time it is an
    // anonymous stack slot, that gate has already run. Porting a
    // stack-position check here alone, with no such system anywhere else
    // in the poly checker, would be inconsistent rather than a fix.
    if name == "^" {
        let n = stack.len();
        if n < 1 {
            return Err(need(1, n));
        }
        if stack[n - 1].quot.is_some() {
            return Err(poly_unsupported_accessor_error(ctx, span, name));
        }
        let payload = stack[n - 1].pt.clone();
        // Review fix (P7b.S6 Phase 3, P0): the poly-side twin of
        // `check_owned_cell_word`'s `contains_reference` guard -- without it
        // a generic word taking `&'T` and calling `^` on it interns a
        // reference-shaped cell that has no equivalent panic-free shape at
        // lowering time (`word_families.rs`'s `^` assumes the checker never
        // let a reference-carrying payload through).
        if ctx.with_extended_type_slices(|structs, enums| {
            contains_poly_reference(&payload, structs, enums, arrays)
        }) {
            return Err(poly_constructed_reference_error(
                ctx,
                span,
                "the payload `^` would store",
                &payload,
                sig,
            ));
        }
        // Review fix (P7b.S6, post-implementation): a body-internal cell --
        // one whose payload never reaches this word's own declared
        // input/output signature (`^ drop`, never returned) -- would
        // otherwise never be interned: `apply_subst` only grounds and
        // interns a signature type, and this construction is invisible to
        // it. Recording the site here lets each concrete instantiation
        // ground and intern it too, the same way `enum_sites` lets a
        // body-internal generic-enum construction ground later.
        tctx.cell_sites
            .push((span, PolyType::OwnedCell(Box::new(payload.clone()))));
        stack.truncate(n - 1);
        stack.push(PolySlot::new(PolyType::OwnedCell(Box::new(payload))));
        return Ok(stack);
    }
    if name == "^>" {
        let n = stack.len();
        if n < 1 {
            return Err(need(1, n));
        }
        let top = stack[n - 1].pt.clone();
        let PolyType::OwnedCell(payload) = top else {
            return Err(poly_rendered_type_mismatch_error(
                ctx,
                span,
                name,
                "^_",
                &poly_type_str(&top, sig),
            ));
        };
        stack.truncate(n - 1);
        stack.push(PolySlot::new(*payload));
        return Ok(stack);
    }
    // The five core shuffles move `PolySlot` slots verbatim; `dup`/`over` gate
    // on `Copy` (a bare variable answers from its bound set, X7).
    match name {
        "dup" => {
            let top = stack.last().ok_or_else(|| need(1, stack.len()))?.clone();
            poly_copy_gate(&top.pt, "dup", sig, ctx, span, structs, enums, arrays)?;
            stack.push(top);
            return Ok(stack);
        }
        "over" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(2, n));
            }
            let below = stack[n - 2].clone();
            poly_copy_gate(&below.pt, "over", sig, ctx, span, structs, enums, arrays)?;
            stack.push(below);
            return Ok(stack);
        }
        "swap" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(2, n));
            }
            stack.swap(n - 1, n - 2);
            return Ok(stack);
        }
        "rot" => {
            let n = stack.len();
            if n < 3 {
                return Err(need(3, n));
            }
            let a = stack.remove(n - 3);
            stack.push(a);
            return Ok(stack);
        }
        "drop" => {
            stack.pop().ok_or_else(|| need(1, 0))?;
            // P7.S3v (R4): the generic-body twin of the monomorphic `drop`
            // arm. A generic word cannot *declare* an owning parameter, but it
            // can call a word that returns one, so an owning closure reaches
            // this arm through the body rather than the signature -- and
            // disposes through the same synthesized disposer either way.
            return Ok(stack);
        }
        "len" => {
            let top = &stack.last().ok_or_else(|| need(1, stack.len()))?.pt;
            match top {
                PolyType::Array(..) | PolyType::Concrete(Type::Array(..)) => {
                    // Non-consuming: the array stays, `len` folds to `usize`.
                    stack.push(PolySlot::new(PolyType::Concrete(Type::Usize)));
                }
                // P7 slice 3c (R9.1): a slice answers its *carried* runtime
                // length, never a scan -- and consumes the slot, like `str`
                // and unlike the array arms above. `len` on an array reads a
                // place that stays where it is; a slice is a value on the
                // stack, so leaving it there would strand a residual slot
                // (`0 s len >i64` must fold to `0 usize`). Nothing is lost:
                // a slice is never move-tracked (`is_linear` is false for
                // it), so the local it came from can be named again.
                PolyType::Concrete(Type::Str | Type::Slice(..)) => {
                    stack.pop();
                    stack.push(PolySlot::new(PolyType::Concrete(Type::Usize)));
                }
                _ => return Err(poly_op_on_variable_error(ctx, span, "len", top, sig)),
            }
            return Ok(stack);
        }
        // P7 slice 3c (R10.1, phase 4): `slice ( &array[T N] -- Slice[T] )` in a
        // generic body. The buffer's *length* may be a variable (`&array[i64 'N]`)
        // -- erasing it into a runtime length is what a view is for -- but its
        // element may not: a generic element is a locked non-goal (R1.2), so a
        // non-concrete one is a located rejection here rather than a shape no
        // instantiation could ground.
        "slice" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(1, n));
            }
            let receiver = stack[n - 1].pt.clone();
            let Some((recv_mut, elem, _)) = poly_ref_array_parts(&receiver, arrays) else {
                return Err(poly_op_on_variable_error(
                    ctx, span, "slice", &receiver, sig,
                ));
            };
            let PolyType::Concrete(element) = elem else {
                return Err(poly_slice_generic_element_error(ctx, span, &elem, sig));
            };
            let out = intern_slice_type(slices, element, recv_mut);
            stack.truncate(n - 1);
            stack.push(PolySlot::new(PolyType::Concrete(out)));
            return Ok(stack);
        }
        // R10.3: `subslice` re-derives a fresh view of the receiver's own
        // type, so it interns nothing and needs no element rule of its own.
        "subslice" => {
            let n = stack.len();
            if n < 3 {
                return Err(need(3, n));
            }
            let receiver = stack[n - 3].pt.clone();
            if !matches!(receiver, PolyType::Concrete(Type::Slice(..))) {
                return Err(poly_op_on_variable_error(
                    ctx, span, "subslice", &receiver, sig,
                ));
            }
            check_poly_slice_offset(&stack[n - 2], ctx, span, "subslice", sig)?;
            check_poly_slice_offset(&stack[n - 1], ctx, span, "subslice", sig)?;
            stack.truncate(n - 3);
            stack.push(PolySlot::new(receiver));
            return Ok(stack);
        }
        // `tag ( E -- u32 )`: pop a payloadless scalar enum, push its u32
        // discriminant. The poly twin of `check_tag_word`, needed so a
        // combinator body like `cmptag`'s -- which calls `cmp` then `tag` on
        // the concrete `Ordering` result -- can be walked by `check_poly_body`.
        // A type variable or any non-enum is a located error; a payload-carrying
        // enum is too.
        "tag" => {
            let top = stack.last().ok_or_else(|| need(1, stack.len()))?.clone();
            // A quotation literal on the operand position keeps the located
            // rejection the name-based guard produced before this arm existed.
            if top.quot.is_some() {
                return Err(poly_quotation_combinator_unsupported_error(
                    ctx, span, "tag",
                ));
            }
            match &top.pt {
                PolyType::Concrete(Type::Enum(id, _)) => {
                    if !enums[id.index()]
                        .variants
                        .iter()
                        .all(|v| v.fields.is_empty())
                    {
                        return Err(poly_op_on_variable_error(ctx, span, "tag", &top.pt, sig));
                    }
                    stack.pop();
                    stack.push(PolySlot::new(PolyType::Concrete(Type::U32)));
                }
                _ => {
                    return Err(poly_op_on_variable_error(ctx, span, "tag", &top.pt, sig));
                }
            }
            return Ok(stack);
        }
        _ => {}
    }
    // P7.S3k (R7): the six-name "comparisons need `Ord`" carve-out that used
    // to sit here is gone. `eq`/`lt`/`gt`/`lte`/`gte`/`ne` are `lib/cmp.sth`
    // words, so a real call arrives module-mangled and the bare-name match
    // never fired outside the unmangled `parse_with_core` test harness. Their
    // one real capability -- a comparison on the body's own `'T`, gated on its
    // bounds -- is now a special case of the generic-callee arm below, driven
    // by `gt`'s declared `['T: Copy Ord] ( 'T 'T -- Bool )` rather than by name.
    // A monomorphic word: its concrete inputs must be met by concrete slots;
    // a bare variable passed to a concrete-typed argument is a located error.
    // Slice 8a fix 2 (R6/R7): a builtin-named env candidate (a user overload
    // of an operator, e.g. `add`) does not intercept here on a *mismatch* --
    // unlike an ordinary word, a builtin name also has `BUILTIN_TABLE` to
    // fall back to, so a mismatched candidate defers to `poly_delegate_op`
    // below instead of erroring outright. An exact match still wins here
    // (R2, same priority as any other env candidate), but the call site is
    // recorded for lowering (R7): the literal name would otherwise hit the
    // builtin's own hardcoded `Instr::Bin`/`Instr::Cmp` arm.
    // R1/R2: resolve among this name's candidates by exact operand match; a
    // lone candidate is the ordinary case and is used as-is, matching the
    // single-signature behaviour this path had before overloading.
    //
    // D3 (slice 8b): this is the poly-body twin of the concrete path's own
    // call ahead of the ordinary env dispatch -- a generated destructure
    // (`S>`) is just another `env` candidate here, so the guard must run
    // before this lookup dispatches one for a drop-overloaded struct, or a
    // generic word could destructure it and skip the destructor.
    check_destructure_drop_guard(name, span, ctx)?;
    // P7 slice 3d (R1): `call` on a quotation *literal* is the one member of
    // this family that never needs a row -- it splices the literal's own
    // body in place, the poly analogue of the concrete path's own literal
    // `call` (`terms.rs:299-357`). Handled ahead of both the S3b-follow
    // combinator dispatch and the retained guard below so a literal never
    // reaches either; a non-literal operand (an abstract or forwarded
    // quotation) is a located rejection, not a splice. `call` is a
    // compiler-known primitive, never a `CombinatorEnv` entry, so this arm
    // and the dispatch below can never both match the same name.
    if name == "call" {
        let Some(top) = stack.last() else {
            return Err(underflow_error(ctx, span, name, 1, 0));
        };
        let Some(quot) = top.quot else {
            // P7.S3f (R3): a genuine ground `Type::Quotation` parameter carries
            // no literal marker (there is nothing spliceable behind it), so it
            // is checked against its own declared effect instead -- the poly
            // twin of `check_abstract_quotation_call`. An abstract
            // `PolyType::Quotation` (still carrying a variable) and every
            // non-quotation operand keep rejecting below (L1).
            if let PolyType::Concrete(Type::Quotation(eff)) = top.pt {
                stack.pop();
                return poly_call_ground_quotation_param(eff, span, stack, ctx, name, sig);
            }
            // P7.S3l (R1): an abstract `PolyType::Quotation` still carrying a
            // variable -- matched unconditionally, no `is_inline`/row guard.
            // A non-combinator word can never carry `is_inline = true` or a
            // row-carrying effect on a declared quotation slot (rejected at
            // parse time by `parse_poly_quotation_inner` and, for `is_inline`,
            // by `check_inline_quotation_requires_inline`), so there is no
            // live shape here for a guard to exclude.
            if let PolyType::Quotation(ins, outs, ..) = &top.pt {
                let ins = ins.clone();
                let outs = outs.clone();
                stack.pop();
                return poly_call_abstract_quotation_param(
                    &ins, &outs, span, stack, ctx, name, sig,
                );
            }
            let pt = top.pt.clone();
            return Err(poly_op_on_variable_error(ctx, span, name, &pt, sig));
        };
        stack.pop();
        let lit = scope.quotation(quot);
        let body = lit.body.clone();
        // R1's teardown, the poly analogue of `Scope::leave`/`leave_block`
        // for a splice with no block of its own (`poly_eliminator_call`
        // takes the same snapshot ahead of each arm walk, `poly.rs:1298`):
        // the poly walk has no block scope, so nothing removes a local this
        // splice binds. Snapshot the enclosing locals, walk the body in
        // place, reject any local leaked past the splice, then retain back
        // down to the snapshot -- never a `Moves::join` (R3), since there is
        // only ever this one body and one continuation.
        let enclosing_locals: HashSet<String> = scope.locals.keys().cloned().collect();
        // The splice runs in place, so a tail `call`'s own tail terms are the
        // enclosing word's: `tail` is threaded unchanged, exactly as the
        // concrete literal-`call` splice threads it.
        stack = poly_walk(
            &body,
            stack,
            scope,
            sig,
            ctx,
            env,
            combinators,
            structs,
            enums,
            arrays,
            cells,
            refs,
            slices,
            builtin_overloads,
            tctx,
            cross,
            tail,
        )?;
        let leaked = scope
            .moves
            .unconsumed()
            .into_iter()
            .find(|local| !enclosing_locals.contains(*local))
            .map(str::to_string);
        if let Some(local) = leaked {
            let pt = scope.locals[&local].clone();
            return Err(poly_arm_local_not_consumed_error(
                ctx,
                span,
                name,
                &local,
                &poly_type_str(&pt, sig),
            ));
        }
        scope.locals.retain(|k, _| enclosing_locals.contains(k));
        scope
            .moves
            .states
            .retain(|k, _| enclosing_locals.contains(k));
        return Ok(stack);
    }
    // P7 slice 3b-follow (R2): a call to a row-typed inline combinator, ahead
    // of both rejections that used to catch this family -- the narrowed name
    // guard below and the `QuotLit` operand window further down, which is
    // where every combinator *not* named in that guard (`unless`, any library
    // or user `inline` word with `~[ ]` parameters) landed. Driven by the
    // callee's declared `PolySig`, not by name, so one dispatch covers all of
    // them. The lookup is by the call's own name: `collect_combinators` keys
    // on `word.name`, and `resolve` rewrites a call site and its callee's
    // declaration identically (`times__m1`), so a prelude `if` and an
    // imported `times` both hit under the spelling that reaches here.
    if let Some(csig) = poly_row_combinator(combinators, name) {
        return poly_combinator_call(
            csig,
            name,
            span,
            stack,
            scope,
            sig,
            ctx,
            env,
            combinators,
            structs,
            enums,
            arrays,
            cells,
            refs,
            slices,
            builtin_overloads,
            tctx,
            cross,
            tail,
        );
    }
    // P7 slice 3b (R4/OQ6), narrowed first by S3b-follow (OQ2) and again by
    // P7.S3d (R1): `call` on a literal now splices above, so only `branch`
    // and `tag` remain -- `branch` is a compiler-known primitive with no
    // `~[ ]` parameter to dispatch off, and `tag` is not a quotation consumer
    // at all (an all-unit enum to `u32`), so it shares no machinery with the
    // dispatch above. Located and named here rather than left to whichever of
    // two unrelated rejections happens to catch the call, neither of which
    // says the consumer is *deferred*: with the quotation on top the
    // `QuotLit` operand window below reports it as a data operand ("`branch`
    // is not permitted on a quotation literal"), and with the quotation
    // deeper than that window it reaches `unknown word` (`poly_call_term`
    // cannot see `poly_env`, so neither is registered on this path).
    if matches!(name, "branch" | "tag") && stack.iter().any(|slot| slot.quot.is_some()) {
        return Err(poly_quotation_combinator_unsupported_error(ctx, span, name));
    }
    // P7 slice 3b (R2): a generated eliminator (`Shape?`) routes ahead of the
    // ordinary `env` dispatch, mirroring `check_term`'s own intercept: its
    // arms are matched to variants by annotation tag, not by slot position,
    // so the `PolySig` it is registered under must never be what checks a
    // call site. Unlike `check_term` there is no `PolyCtx` here to read a
    // precomputed registry off, so it is built from the `enums` this walk
    // already carries -- one keying rule, in `eliminator_registry`.
    if let Some(target) = ctx_eliminator_registry(ctx, enums).get(name).copied() {
        return poly_eliminator_call(
            target,
            name,
            span,
            stack,
            scope,
            sig,
            ctx,
            env,
            combinators,
            structs,
            enums,
            arrays,
            cells,
            refs,
            slices,
            builtin_overloads,
            tctx,
            cross,
            tail,
        );
    }
    // P7 slice 3b (R2/L2): every legal use of a quotation literal has been
    // tried by now -- the shuffles moved it, the deferred family named
    // itself, the eliminator consumed it. What is left is a *data* operand
    // use (a constructor argument, an operator operand), and the marker is
    // not a value type, so this is where that is rejected. Located here
    // rather than left to `poly_delegate_op`, whose maximal-concrete-suffix
    // extraction stops at the marker and would report the operator as
    // underflowing a stack that is not actually short.
    //
    // The window is the *whole* operand run, not the top slot: a binary
    // operator reads `stack[n - 2]` too, so a marker parked there is an
    // operand of it just as much (`1 ~[ .. ] swap add`). This is the concrete
    // path's own rule -- `check_operator` guards the top and, for a
    // non-unary name, the slot beneath it. Arity comes from `BUILTIN_TABLE`,
    // whose rows for one name all agree on it; a name with no row (an
    // ordinary word, a `>T` conversion) reads only the top here, and a
    // deeper marker under an ordinary word is reported by the env dispatch
    // below against the declared input it fails to be.
    let operand_window = BUILTIN_TABLE
        .get(name)
        .map_or(1, |rows| rows[0].inputs.len())
        .min(stack.len());
    // P7 slice 3d (R2): a `QuotLit` slot in the window is not rejected when
    // it sits at the single resolved concrete `env` candidate's own ground
    // `Type::Quotation` input position -- the env-dispatch grounding arm
    // below handles it instead. `env` holds concrete words only (a poly
    // word lives in `poly_env`, never here), so this can never carve out a
    // poly callee; an overloaded name (more than one candidate) never
    // matches `single_candidate` below and keeps the rejection, which is
    // R2's own completeness-gap note, not a bug in this carve-out.
    //
    // P7b.S5 (R4 audit VERDICT): confirmed non-issue. This binds only
    // `Some([only])` -- never a multi-candidate `find` -- so an overloaded
    // name (including a same-shaped cross-module ctor collision) always
    // falls to `None` here and keeps the rejection; it cannot cross-pick.
    // Not widened.
    let single_candidate = match env.get(name).map(Vec::as_slice) {
        Some([only]) => Some(only),
        _ => None,
    };
    let window_base = stack.len() - operand_window;
    for (i, slot) in stack[window_base..].iter().enumerate() {
        if !matches!(slot.pt, PolyType::QuotLit) {
            continue;
        }
        let grounds = single_candidate.is_some_and(|only| {
            only.sig.inputs.len() >= operand_window
                && matches!(
                    only.sig.inputs[only.sig.inputs.len() - operand_window + i],
                    Type::Quotation(_)
                )
        });
        if !grounds {
            return Err(poly_op_on_variable_error(
                ctx,
                span,
                name,
                &PolyType::QuotLit,
                sig,
            ));
        }
    }
    // P7.S12 (R6): the destructure dual of the construction arm just below --
    // tried first because it reads its header off the *operand* (a narrowed
    // `PolyType::GenericVariant`, only ever produced inside an eliminator
    // arm, R3.5) rather than searching the module by name, so there is
    // nothing for `poly_construct_generic`'s own by-name search to find here
    // and the two can never both match one call.
    if let Some(next) = poly_destructure_generic(name, span, &mut stack, ctx, tctx) {
        return Ok(next);
    }
    // P7 slice 3a (R3): a call naming a variant of a generic enum header (or
    // a generic struct's own constructor) is legal in a polymorphic body,
    // tried *before* the ordinary `env` dispatch below: a fully-concrete
    // instantiation (`Result[i64 i64]`'s `Ok`, minted at parse time) folds to
    // `PolyType::Concrete` in the enclosing word's declared output (R1), so
    // `poly_construction_target`'s search for an ungrounded `PolyType::Generic`
    // finds nothing there and this arm is a no-op for that case -- the
    // already-working concrete case is unaffected, exactly as today. Ordered
    // ahead of `env` because a *single* registered concrete candidate under
    // this bare name (e.g. some unrelated `Result[bool i64]` elsewhere in the
    // program) commits unconditionally below and errors on a `'T` operand
    // mismatch rather than falling through.
    if let Some(next) = poly_construct_generic(
        name, span, &mut stack, sig, ctx, env, structs, enums, arrays, cells, refs, tctx,
    )? {
        return Ok(next);
    }
    // P7.S3e (R7/R10): bound-directed dispatch now runs once, up front
    // (review finding 3) -- it already fell through here `Ok(None)` when it
    // didn't apply, so nothing changes for the ordinary `env` dispatch below
    // by having tried it earlier.
    // P7b.S5 (R4 audit VERDICT, corrected -- review round 1 fix): actually
    // reached and actually discriminating, verified with a real fixture, not
    // assumed. `poly_construct_generic` above returns `Ok(None)` and falls
    // through to here whenever the constructor is non-fieldless and its
    // operands already exactly match one of `env`'s generated candidates
    // (`poly_env_exact_match`); a poly (generic) word's own bare ctor call
    // over an already-concrete operand lands here with 2+ same-shaped
    // cross-module candidates in `matching` (confirmed via a temporary
    // `eprintln!`: `matching_len=2`), and the tier pick is load-bearing, not
    // a no-op -- swapping it for `matching.last()` turns a passing build
    // into a type-mismatch error (`tests/phase7b_slice5.rs`,
    // `poly_body_tier_arm_resolves_same_shaped_ctor_to_callers_own_module`).
    // Routed through the shared tier policy (`tier_pick`, not
    // `select_overload` directly): the shared `operand_matching` assumes
    // one uniform-length `Type` operand vector, but this call site's
    // per-candidate window is checked for concreteness against each
    // candidate's OWN arity, matching the original per-slot `matches!`
    // exactly -- a differently-arity-overloaded candidate here whose own
    // (shorter) window is concrete must still be able to match even if an
    // earlier, non-concrete stack slot lies outside that window; a single
    // max-arity window shared across all candidates would wrongly require
    // concreteness beyond what that candidate actually needs.
    let chosen = env.get(name).and_then(|candidates| match &candidates[..] {
        [only] => Some(only),
        candidates => {
            let matching: Vec<&Overload> = candidates
                .iter()
                .filter(|o| {
                    stack.len() >= o.sig.inputs.len()
                        && stack[stack.len() - o.sig.inputs.len()..]
                            .iter()
                            .zip(&o.sig.inputs)
                            .all(|(s, inp)| matches!(&s.pt, PolyType::Concrete(t) if t == inp))
                })
                .collect();
            let caller_module = span.module;
            let pick = match ctx.modules() {
                Some(modules) => {
                    // Same demangle-before-visibility rule as `terms.rs:956`.
                    let bare_name = crate::resolve::demangle_call(name);
                    tier_pick(&matching, caller_module, |m| {
                        is_name_visible_to_module(modules, caller_module, m, &bare_name)
                    })
                }
                None => tier_pick(&matching, caller_module, |_| true),
            };
            match pick {
                OverloadPick::Pick(hit) => Some(hit),
                OverloadPick::Ambiguous => None,
            }
        }
    });
    if let Some(chosen) = chosen {
        let msig = &chosen.sig;
        let n_in = msig.inputs.len();
        let is_builtin_name = BUILTIN_TABLE.contains_key(name);
        let exact = stack.len() >= n_in
            && stack[stack.len() - n_in..]
                .iter()
                .zip(&msig.inputs)
                .all(|(s, inp)| matches!(&s.pt, PolyType::Concrete(t) if t == inp));
        if exact || !is_builtin_name {
            if stack.len() < n_in {
                return Err(need(n_in, stack.len()));
            }
            let base = stack.len() - n_in;
            for (i, inp) in msig.inputs.iter().enumerate() {
                match &stack[base + i].pt {
                    PolyType::Concrete(t) if t == inp => {}
                    PolyType::Var(v) => {
                        return Err(poly_var_to_concrete_error(
                            ctx,
                            span,
                            name,
                            &sig.ty_var_names[*v as usize],
                            *inp,
                        ));
                    }
                    // P7 slice 3d (R2): a body-local literal at a declared
                    // ground `Type::Quotation` input grounds against that
                    // effect instead of erroring -- the pointwise check
                    // ported from `unify_poly_input`'s `Quotation` arm, run
                    // for real against the literal's own body since there is
                    // no declared `PolyType` row to unify against here. The
                    // ordinary call then proceeds exactly as for any other
                    // operand (L1: the literal is consumed, never survives).
                    // `chosen.symbol == name` is R2's "non-overloaded" gate at
                    // the only place it bites: `ast::overload_symbols` suffixes
                    // (`$$0`) a concrete word merely for sharing a name with an
                    // unrelated poly word, and grounding through one records no
                    // `builtin_overloads` entry (the record below is
                    // `exact`-gated, never true for a `QuotLit`), leaving
                    // lowering to resolve a bare name it cannot find. Excluded,
                    // it falls to `other`'s located rejection.
                    PolyType::QuotLit
                        if matches!(inp, Type::Quotation(_)) && chosen.symbol == name =>
                    {
                        let Type::Quotation(eff) = *inp else {
                            unreachable!()
                        };
                        // Review fix (Bug 1): a `QuotLit` slot's identity
                        // does not survive a bind-then-reread (e.g. `[ .. ] |
                        // q | q run0`) -- the local rebinds a fresh marker
                        // slot with `quot: None`. That is not a value this
                        // grounding arm can ground, so it is the located
                        // rejection the operand-window guard above already
                        // renders for an ungroundable `QuotLit`, never a
                        // panic (L1).
                        let Some(quot) = stack[base + i].quot else {
                            return Err(poly_op_on_variable_error(
                                ctx,
                                span,
                                name,
                                &PolyType::QuotLit,
                                sig,
                            ));
                        };
                        poly_ground_quotation_literal(
                            quot,
                            eff,
                            name,
                            span,
                            scope,
                            sig,
                            ctx,
                            env,
                            combinators,
                            structs,
                            enums,
                            arrays,
                            cells,
                            refs,
                            slices,
                            builtin_overloads,
                            tctx,
                            cross,
                        )?;
                    }
                    other => {
                        return Err(poly_op_on_variable_error(ctx, span, name, other, sig));
                    }
                }
            }
            stack.truncate(base);
            for out in &msig.outputs {
                stack.push(PolySlot::new(PolyType::Concrete(*out)));
            }
            if exact && (is_builtin_name || chosen.symbol != name) {
                builtin_overloads.insert(span, chosen.symbol.clone());
            }
            return Ok(stack);
        }
    }
    // Everything else is an ordinary operator over concrete operands. Extract
    // the maximal concrete suffix, run the concrete check, reflect it back; a
    // variable operand (a too-short suffix) surfaces as the op's own error.
    if let Some(next) = poly_delegate_op(name, span, &mut stack, ctx, env, builtin_overloads)? {
        return Ok(next);
    }
    // P7 slice 3g (R1): a self-call -- the term names the very word being
    // walked. `sig` already *is* the callee's signature, with the same rigid
    // type-variable ids the walk is using, so this is a pure structural
    // pointwise match against `sig.inputs`/`sig.outputs`, never a fresh
    // unification or `Subst` (D1): an operand shaped `array['T 2]` does not
    // structurally equal `'T`, so recursing at a different type argument is
    // an ordinary mismatch here, not a request for a new instantiation --
    // this is what keeps the roadmap's termination hazard unreachable
    // through bare self-call syntax. Compared against `ctx.mangled_name()`,
    // never `ctx.rendered_word()`: `resolve::mangle` rewrites a self-call body
    // reference to the mangled spelling `word.name` already carries, so the
    // demangled display name would miss a multi-module closure.
    if ctx.mangled_name() == name {
        let n = sig.inputs.len();
        if stack.len() < n {
            return Err(need(n, stack.len()));
        }
        let base = stack.len() - n;
        for (i, inp) in sig.inputs.iter().enumerate() {
            let found = &stack[base + i].pt;
            if found != inp {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(inp, sig),
                    &poly_type_str(found, sig),
                ));
            }
        }
        // P7.S3g-follow (1c): in tail position, in a word that really
        // back-edges, this call *is* the loop's back-edge, so a reference the
        // body derived from one of its own locals may not ride it. Gated on
        // both halves for the same reason the concrete twin is
        // (`terms.rs`'s R15 site): `tail` is the syntactic position, and
        // `is_self_tail_call` is the predicate lowering consults to decide
        // whether this word gets the loop shape at all, so a non-tail
        // self-call -- which lowers as ordinary recursion, with a fresh frame
        // per level and no rebound slot -- is untouched.
        if tail && ctx.is_self_tail_call() {
            check_poly_reference_across_back_edge(ctx, span, name, &stack[base..], scope)?;
        }
        stack.truncate(base);
        for out in &sig.outputs {
            stack.push(PolySlot::new(out.clone()));
        }
        return Ok(stack);
    }
    // P7.S3k (R1): a call to a *different* polymorphic word. `cross.env` is
    // the same `poly_env` a monomorphic body dispatches through, so this arm
    // reaches every generic callee -- same-module or imported, user-declared
    // or a library word -- and relates its rigid variables to this body's
    // symbolically (R2), the caller having no θ of its own here.
    //
    // Copied out of `cross` first (it is one shared reference) so the record
    // below can borrow `cross.calls` mutably.
    let poly_env = cross.env;
    if let Some(candidates) = poly_env.get(name) {
        return poly_cross_call(
            name,
            span,
            stack,
            sig,
            ctx,
            structs,
            enums,
            arrays,
            tctx.traits,
            candidates,
            cross,
        );
    }
    Err(unknown_word_error(ctx, span, name))
}

/// Whether a declared slot mentions a length variable at any depth.
fn poly_mentions_len_var(pt: &PolyType) -> bool {
    match pt {
        PolyType::Array(elem, len) => matches!(len, Len::Var(_)) || poly_mentions_len_var(elem),
        PolyType::Ref(referent, _) => poly_mentions_len_var(referent),
        PolyType::OwnedCell(payload) => poly_mentions_len_var(payload),
        // P7.S6a (R8a): a header's own `len_args` are length positions too,
        // scanned alongside its type `args` -- a length may live in
        // `len_args` instead of a bare array, and this guard exists
        // specifically to reject that in a poly-body cross-call.
        PolyType::Generic { args, len_args, .. } => {
            args.iter().any(poly_mentions_len_var)
                || len_args.iter().any(|l| matches!(l, Len::Var(_)))
        }
        PolyType::Quotation(ins, outs, ..) => ins.iter().chain(outs).any(poly_mentions_len_var),
        PolyType::Concrete(_) | PolyType::Var(_) | PolyType::QuotLit => false,
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this predicate walks.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
        // P7b.S1 (S1-7): an application's variant carries type args only --
        // a `Len`-domain application is fenced to S2+ -- so it mentions a
        // length variable only through its (type) arguments.
        PolyType::App { args, .. } => args.iter().any(poly_mentions_len_var),
    }
}

/// P7.S3g-follow (1c): the poly twin of `check_reference_across_back_edge` --
/// a reference the *body* derived from one of its own locals, handed to the
/// self-tail call and so carried across the loop's back-edge. Locals are
/// rebound at the loop header, so the storage that local named this iteration
/// is not the storage the same name denotes next iteration.
///
/// Scanned over the call's `args` (`stack[base..]`), the values that actually
/// cross the edge. Two things the concrete twin reads are not available here,
/// which is why this is not a literal port: `PolySlot` carries no `Deriv`, so
/// there is no way to trace *which* argument a recorded borrow flowed into,
/// and a poly-body borrow's provenance is only the side table
/// (`PolyScope::borrows`) with its deliberately coarse liveness. So the rule
/// is the conjunction the available data supports -- a reference among the
/// arguments, and a live borrow of a *local* recorded by this body -- which
/// can reject a program the concrete side would accept (a dead local borrow
/// beside a forwarded parameter reference). That is the same conservatism
/// every other poly borrow diagnostic carries, and it is stated in the
/// message.
///
/// A borrow of a **static** is exempt, exactly as the concrete twin's R3
/// exemption is: a static's data-segment storage survives every iteration. A
/// reference *parameter* (or one projected from it) is exempt for free, since
/// nothing in this body borrowed anything to record.
///
/// The local/static split is read off `PolyBorrow::static_rooted`, not off
/// `scope.locals`: a borrow taken inside a `call`-splice or an eliminator arm
/// outlives the locals of the block that took it (both exits `retain` locals
/// but keep borrow records, and `poly_walk_arms` unions each arm's borrows
/// back into the parent), so a lookup here would read a real frame-local
/// borrow as a static and exempt it.
fn check_poly_reference_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    args: &[PolySlot],
    scope: &PolyScope,
) -> Result<(), String> {
    if !args.iter().any(|slot| is_reference_slot(&slot.pt)) {
        return Ok(());
    }
    // Push order, so a body holding two live borrows names the earlier one.
    let rooted = scope.borrows.iter().find(|borrow| !borrow.static_rooted);
    match rooted {
        Some(borrow) => Err(poly_reference_across_back_edge_error(
            ctx,
            span,
            callee,
            &borrow.place,
        )),
        None => Ok(()),
    }
}

/// P7 slice 3d (R2, C2): ground a body-local quotation literal against a
/// concrete `env` candidate's declared, ground `Type::Quotation` input --
/// the pointwise check `unify_poly_input`'s `Quotation` arm runs for a
/// *declared* poly parameter, ported here to run for real against the
/// literal's own body, since there is no declared `PolyType` row to unify
/// against (a `QuotLit` marker never carries one). Rowless: seeds a fresh
/// walk with `eff.inputs`, walks the body in place (`poly_walk`, not a
/// splice onto the live stack), and requires the exit stack matches
/// `eff.outputs` pointwise -- the same arity-then-pointwise shape
/// `unify_poly_input` checks, but by running the body rather than unifying
/// two signatures.
///
/// Teardown mirrors R1's `call`-splice teardown exactly, for the same
/// reason: this is a straight-line walk with no block scope of its own, so a
/// linear local the body binds and leaves unconsumed would otherwise leak
/// past this call unreported (the poly analogue of `Scope::leave`).
///
/// R12 is ported here too (see the check below). The eliminator-arm walk
/// deliberately skips it and this path must not: an arm runs at most once,
/// in place, whereas the callee this literal is an argument to materializes
/// it and may `call` it any number of times.
#[allow(clippy::too_many_arguments)]
fn poly_ground_quotation_literal(
    quot: PolyQuotRef,
    eff: &'static QuotEffect,
    name: &str,
    span: Span,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
) -> Result<(), String> {
    let lit = scope.quotation(quot).clone();
    // Review fix (Bug 2): the mono twin's flavour funnel
    // (`check_literal_against_declared_effect`, `literal_is_inline !=
    // is_inline`) runs before anything else touches the literal's body.
    // C2's callee is always concrete with a ground `Type::Quotation`
    // input (an `inline` word cannot declare one this grounding arm
    // matches), so `is_inline` is always `false` here -- the only
    // reachable mismatch is an inline `~[ ]` literal at this ordinary
    // parameter.
    if lit.is_inline {
        let param = crate::ast::quotation_type(eff.inputs.clone(), eff.outputs.clone());
        return Err(inline_literal_at_ordinary_param_error(
            ctx, lit.span, name, param,
        ));
    }
    // Review fix (Bug 3): port the mono twin's annotation reconciliation
    // (same function, immediately after the flavour check) -- an
    // annotated literal must agree with the declared parameter effect
    // before its body ever runs. Never shape-changing (C2 grounds
    // against a ground `QuotEffect`, which carries no row).
    if let Some(annot) = lit.annot.clone() {
        reconcile_annotation_with_parameter(&annot, eff, false, false, ctx, name)?;
    }
    let body = lit.body;
    let seeded: Vec<PolySlot> = eff
        .inputs
        .iter()
        .map(|t| PolySlot::new(PolyType::Concrete(*t)))
        .collect();
    let enclosing_locals: HashSet<String> = scope.locals.keys().cloned().collect();
    let moves_before = scope.moves.states.clone();
    // Never tail: this literal is an argument the callee materializes and
    // decides when to run, not a body spliced in place -- the concrete twin
    // pins the same `false` for every non-arm quotation parameter.
    let out = poly_walk(
        &body,
        seeded,
        scope,
        sig,
        ctx,
        env,
        combinators,
        structs,
        enums,
        arrays,
        cells,
        refs,
        slices,
        builtin_overloads,
        tctx,
        cross,
        false,
    )?;
    // R12, the poly twin of the concrete argument site's
    // `quotation_captures_local_error`: a linear *enclosing* local the
    // literal consumed. The callee holds the materialized literal and may
    // `call` it N times, so one consumption here is N frees at run time --
    // without this the double free is silent, the concrete twin of the same
    // body having rejected it. Name-ordered for a deterministic diagnostic
    // when a body consumes two of them.
    let captured = moves_before
        .iter()
        .filter(|(n, before)| {
            matches!(before, MoveState::Live)
                && matches!(
                    scope.moves.states.get(*n),
                    Some(MoveState::Moved(_) | MoveState::MaybeMoved(_))
                )
        })
        .map(|(n, _)| n)
        .min();
    if let Some(local) = captured {
        return Err(quotation_captures_local_error(ctx, span, name, local));
    }
    let leaked = scope
        .moves
        .unconsumed()
        .into_iter()
        .find(|local| !enclosing_locals.contains(*local))
        .map(str::to_string);
    if let Some(local) = leaked {
        let pt = scope.locals[&local].clone();
        return Err(poly_arm_local_not_consumed_error(
            ctx,
            span,
            name,
            &local,
            &poly_type_str(&pt, sig),
        ));
    }
    scope.locals.retain(|k, _| enclosing_locals.contains(k));
    scope
        .moves
        .states
        .retain(|k, _| enclosing_locals.contains(k));
    // R12's other half -- a borrow of an enclosing place left on the exit row
    // -- needs no arm of its own, but only *representationally*: a borrow
    // slot is `PolyType::Ref`, never `PolyType::Concrete(Type::Ref(..))`, so
    // it can satisfy no declared output and the pointwise check below rejects
    // it (as a type mismatch, not as the D3 violation it is). Make the two
    // representations unify and the D3 rule silently evaporates, which is why
    // `poly_ground_quotation_literal_borrowing_enclosing_place_is_error` pins
    // the rejection rather than the message.
    if out.len() != eff.outputs.len()
        || !out
            .iter()
            .zip(&eff.outputs)
            .all(|(slot, t)| matches!(&slot.pt, PolyType::Concrete(u) if u == t))
    {
        let found = out
            .iter()
            .map(|slot| poly_type_str(&slot.pt, sig))
            .collect::<Vec<_>>()
            .join(" ");
        return Err(poly_rendered_type_mismatch_error(
            ctx,
            span,
            name,
            eff.name_static,
            &found,
        ));
    }
    Ok(())
}

/// P7.S3f (R3): `call` on a genuine ground `Type::Quotation` parameter -- a
/// real `(code, env)` value the body cannot splice, only honour. The poly twin
/// of `check_abstract_quotation_call`: consume `eff.inputs` deepest-first,
/// push `eff.outputs`, no body walk and no teardown (L3, there is no body
/// here). A `QuotEffect` carries no row and no variable, so every declared
/// slot on either side is a ground `Type` and no `Subst` is involved.
fn poly_call_ground_quotation_param(
    eff: &QuotEffect,
    span: Span,
    mut stack: Vec<PolySlot>,
    ctx: &Ctx,
    op: &str,
    sig: &PolySig,
) -> Result<Vec<PolySlot>, String> {
    let n = eff.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, op, n, stack.len()));
    }
    let base = stack.len() - n;
    for (i, want) in eff.inputs.iter().enumerate() {
        match &stack[base + i].pt {
            PolyType::Concrete(t) if t == want => {}
            // A ground operand that simply is not the declared type renders
            // through the two-`Type` renderer, matching `unify_poly_input`'s
            // own `Concrete` arm. Anything else (a bare `Var`, an abstract
            // quotation) has no `Type` to hand it, so both sides go through
            // `poly_type_str` instead.
            PolyType::Concrete(t) => return Err(type_mismatch_error(ctx, span, op, *want, *t)),
            found => {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    op,
                    want.name(),
                    &poly_type_str(found, sig),
                ));
            }
        }
    }
    stack.truncate(base);
    for out in &eff.outputs {
        stack.push(PolySlot::new(PolyType::Concrete(*out)));
    }
    Ok(stack)
}

/// P7.S3l (R2/R3): `call` on an **abstract** declared quotation parameter --
/// one whose declared effect still mentions a variable. The abstract twin of
/// `poly_call_ground_quotation_param`, one level earlier in the pipeline: pop
/// the quotation (already done by the caller), consume `ins.len()` operands
/// deepest-first, push `outs`, no body walk and no teardown (there is no body
/// behind the value). Because the body is checked once, generically, with
/// every variable rigid, each declared slot is compared **structurally**
/// against the operand's own `PolyType` via its derived `Eq` -- no `Subst` is
/// built or consulted (S3b's L1 discipline).
fn poly_call_abstract_quotation_param(
    ins: &[PolyType],
    outs: &[PolyType],
    span: Span,
    mut stack: Vec<PolySlot>,
    ctx: &Ctx,
    op: &str,
    sig: &PolySig,
) -> Result<Vec<PolySlot>, String> {
    let n = ins.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, op, n, stack.len()));
    }
    let base = stack.len() - n;
    for (i, want) in ins.iter().enumerate() {
        if &stack[base + i].pt != want {
            return Err(poly_rendered_type_mismatch_error(
                ctx,
                span,
                op,
                &poly_type_str(want, sig),
                &poly_type_str(&stack[base + i].pt, sig),
            ));
        }
    }
    stack.truncate(base);
    for out in outs {
        stack.push(PolySlot::new(out.clone()));
    }
    Ok(stack)
}

/// P7 slice 3b (R2/R3): the abstract twin of `check_eliminator_call`
/// (`src/check.rs`) -- a *concrete* enum eliminated inside a **polymorphic**
/// body, whose arms are quotation literals written in that body.
///
/// It is dispatchable without any of the row-typed combinator machinery `if`
/// and `call` need (OQ1): the scrutinee is concrete, so its `EnumId`, its
/// variant set and every arm's narrowed input type are concrete too, and arm
/// collection, exhaustiveness, duplication and unknown-variant checking are
/// structural over that concrete data -- ported here with the concrete
/// diagnostics reused verbatim. The only abstract data is the caller row
/// *below* the scrutinee and the arms' exit rows, and those are compared
/// **structurally**, never row-unified against an abstract stack.
///
/// S3b L1: type variables stay rigid. Two arms agree on an exit position iff
/// the `PolyType`s are structurally equal; `'T` against `i64` is a rejection,
/// not a mid-body bind, so no `Subst` is built or applied in the term walk and
/// no per-arm clone can diverge on one.
///
/// P7.S12 (R5): the scrutinee no longer has to be concrete. An ungrounded
/// `Option['T]` arrives as `PolyType::Generic { is_enum: true, .. }`, naming
/// the *header*, and each arm narrows to a `GenericVariant` carrying the
/// scrutinee's own arguments unchanged. Everything else -- arm collection,
/// written-order normalization, duplication, unknown-variant, exhaustiveness,
/// the arm walk and the escape check -- is the same code either way, reading
/// the header's variant list instead of the monomorph's (R5.2).
#[allow(clippy::too_many_arguments)]
fn poly_eliminator_call(
    gate: EliminatorTarget,
    name: &str,
    span: Span,
    mut stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    tail: bool,
) -> Result<Vec<PolySlot>, String> {
    // P7.S12 (R1.1/R5.1): `gate` is the registry's entry for this call name --
    // a base-family key, since every monomorph of one generic enum and the
    // header itself share one registry entry (R2.1/R2.3). It gates that this
    // call names an eliminator of the right family and supplies the variant
    // count for the underflow diagnostic (identical across every
    // instantiation of the family, and equal to the header's own); the
    // operative header this call actually eliminates is read off the
    // scrutinee's own type once found, mirroring `check_eliminator_call`
    // (S3b R5) one path over.
    //
    // `gate_ty` is the concrete `Type::Enum` the non-enum-scrutinee mismatch
    // names, and exists only for a `Concrete` gate: a header with no
    // monomorph has no `Type` at all, so that rejection goes through the
    // rendered sibling instead.
    let (family_name, family_variants, gate_ty) = match gate {
        EliminatorTarget::Concrete(gate_id) => {
            let decl = &enums[gate_id.index()];
            (
                generic_surface_name(&decl.name).to_string(),
                decl.variants.len(),
                Some(Type::Enum(gate_id, decl.name_static)),
            )
        }
        EliminatorTarget::Generic { idx } => {
            // `ctx_eliminator_registry` builds the header half of the registry
            // out of `ctx.generics()` itself, so a `Generic` entry reaching
            // this call implies the instantiator is live: the two Ctx-less
            // paths (the REPL, `check_poly_combinator_standalone`'s stand-in)
            // register no header at all.
            let cell = ctx
                .generics()
                .expect("a `Generic` registry entry is only built from a live instantiator");
            let generics = cell.borrow();
            let decl = &generics.enums[idx as usize];
            (
                generic_surface_name(&decl.name).to_string(),
                decl.variants.len(),
                None,
            )
        }
    };
    let enum_name = crate::resolve::demangle_word(&family_name).to_string();
    let held = stack.len();
    // Step 1, the concrete path's variable-arity arm collection: a fixed pop
    // cannot tell "an arm is missing" from "the stack is short below the
    // scrutinee", so a missing arm would always present as underflow and the
    // exhaustiveness pass below could never name it.
    let mut arms: Vec<(PolyQuotRef, VariantTag)> = Vec::new();
    while let Some(quot) = stack.last().and_then(|slot| slot.quot) {
        let Some(tag) = scope
            .quotation(quot)
            .annot
            .as_ref()
            .and_then(|a| a.variant_tag.clone())
        else {
            break;
        };
        arms.push((quot, tag));
        stack.pop();
    }
    // Popping off the top yielded the arms reversed; both passes below walk
    // them in *written* order, so the reversal is undone here, once.
    arms.reverse();

    // Step 2: the scrutinee.
    let Some(scrutinee) = stack.last().cloned() else {
        return Err(underflow_error(ctx, span, name, family_variants + 1, held));
    };
    if scrutinee.quot.is_some() || matches!(scrutinee.pt, PolyType::QuotLit) {
        // The operand that stopped collection is a quotation, so it was meant
        // as an arm but carries no variant tag to match one by. The marker is
        // checked beside the identity because a quotation that has been
        // through a `| q |` bind keeps the one and loses the other (S3b L3:
        // `PolyScope.locals` carries no `QuotRef`), and it is still an
        // untagged arm -- not the abstract-scrutinee case below, which would
        // send it off to ask for an enum-kind bound on a type variable it
        // does not have.
        return Err(eliminator_untagged_arm_error(ctx, span, name));
    }
    // P7.S12 (R1.1/R5.1): the operative header is the scrutinee's own -- not
    // the gate's -- so two asymmetric monomorphs of one generic enum
    // eliminate independently in the same poly body: the registry's one
    // entry is only consulted by the caller to reach this call at all, never
    // to decide which instantiation it narrows to. Which of the two branches
    // below runs is decided here and only here, by the scrutinee's own shape:
    // a `Concrete` gate with a `Generic` scrutinee takes the generic branch,
    // and the reverse takes the concrete one. A non-enum scrutinee, or one
    // whose *family* differs from the gate's, is the same
    // `type_mismatch_error` as before.
    let operative = match &scrutinee.pt {
        // Review fix (R1.1 hazard): `found` can be a monomorph this body's
        // own walk minted moments ago -- `enums` only grows once the whole
        // word returns and `check_module` flushes it (the same staleness
        // `type_is_registered`, P7.S3k N1, guards against), so indexing it
        // unconditionally would panic on exactly that id. Gated on
        // `found.index() < enums.len()`, not on `Type::Enum`'s own carried
        // name: that name is whatever spelling the mint site built the type
        // with, which is not always the registry decl's own `.name` (they
        // can differ in mangling), so comparing the two would compare
        // apples to oranges for an *already-flushed* id -- the common case.
        // Only a body-local mint, whose registry decl is not indexable at
        // all yet, has no other name to fall back on.
        PolyType::Concrete(Type::Enum(found, found_name))
            if found.index() < enums.len()
                && generic_surface_name(&enums[found.index()].name) == family_name =>
        {
            Operative::Concrete(*found)
        }
        PolyType::Concrete(Type::Enum(found, found_name))
            if found.index() >= enums.len() && generic_surface_name(found_name) == family_name =>
        {
            Operative::Concrete(*found)
        }
        // P7.S12 (R5.1): the ungrounded scrutinee this slice exists for --
        // `Option['T]`, naming the header by `(idx, module)` rather than any
        // monomorph by `EnumId`. The family is compared through the carried
        // header spelling, exactly as the out-of-range concrete arm above
        // compares `found_name`: `module` here is the *instantiating* module
        // (`PolyType::Generic`'s own doc), so it is not the declaring
        // module a gate could be matched against.
        PolyType::Generic {
            is_enum: true,
            idx,
            module,
            args,
            len_args,
            name: header,
        } if generic_surface_name(header) == family_name => {
            operative_generic_from_scrutinee(*idx, *module, args, len_args)
        }
        // R5.1: a generic *struct* application, or a generic enum of another
        // family, is an ordinary mismatch -- rendered rather than interned,
        // since neither side need have a `Type`.
        PolyType::Generic { .. } | PolyType::GenericVariant { .. } => {
            return Err(poly_rendered_type_mismatch_error(
                ctx,
                span,
                name,
                &family_name,
                &poly_type_str(&scrutinee.pt, sig),
            ));
        }
        // P7 slice 3c (R1.4): a slice reaches the `Concrete(_)` reference arm
        // below through the widened `is_ref()`, but the advice there ("pass
        // the owned `Enum` instead") names nothing real for a view over a
        // buffer. It gets the plain mismatch instead -- the same message the
        // concrete path already gives a slice scrutinee.
        PolyType::Concrete(t) if !t.is_ref() || matches!(t, Type::Slice(..)) => {
            // P7.S12 (R2.1): a gate with no monomorph has no `Type::Enum` to
            // name as the expectation, so it renders the family instead.
            return Err(match gate_ty {
                Some(want) => type_mismatch_error(ctx, span, name, want, *t),
                None => poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &family_name,
                    &poly_type_str(&scrutinee.pt, sig),
                ),
            });
        }
        // A *reference* scrutinee is the concrete path's decision 6, and it
        // buys nothing here: reading a field out of the narrowed variant it
        // would hand each arm needs the projection accessors a generic body
        // does not have yet (P7 slice 1), so every arm it could reach is
        // already unwritable. Located rather than silently narrowed to an
        // owning scrutinee, which would let an arm consume a borrowed enum.
        PolyType::Ref(..) | PolyType::Concrete(_) => {
            return Err(poly_reference_scrutinee_error(ctx, span, name, &enum_name));
        }
        // OQ2: an abstract scrutinee is a `'T` that is *some* enum, which is
        // not constructible without an enum-kind bound (P7.S3d).
        //
        // P7.S12 phase 3 (R3.4): enumerated rather than left as a `_ =>`, so
        // a `PolyType` added later cannot silently read as "abstract enum".
        // `QuotLit` is already answered above (an untagged arm), and the two
        // narrowed-variant shapes have their own arm; what is left is a bare
        // variable and the three composites, none of which is an enum.
        PolyType::Var(_)
        | PolyType::Array(..)
        | PolyType::Quotation(..)
        | PolyType::OwnedCell(_)
        | PolyType::QuotLit
        // P7b.S1 (S1-16): a higher-kinded application is not itself an enum
        // scrutinee (`'F['T]` names no enum family until grounded), so it
        // reports the same abstract-scrutinee error.
        | PolyType::App { .. } => {
            return Err(poly_abstract_enum_scrutinee_error(
                ctx,
                span,
                name,
                &poly_type_str(&scrutinee.pt, sig),
            ));
        }
    };
    // R1.5: a generic enum eliminated inside an `inline` combinator body
    // varies per splice, and the `Span`-keyed `enum_words` record cannot
    // represent that (`splice_trait_calls` is keyed `(uid, span)` for exactly
    // this reason). Widening the key is out of scope for this slice, so the
    // call is rejected here -- silence would reinstate the B3 miscompile
    // behind a different door. The concrete branch is unaffected: its
    // resolution is already fixed at check time.
    if matches!(operative, Operative::Generic { .. }) && tctx.is_combinator_splice {
        return Err(poly_combinator_generic_enum_elimination_error(
            ctx, span, name, &enum_name,
        ));
    }
    // R1.4: the family gate locates and gates; the operative header, read off
    // the scrutinee, dispatches. Everything below -- exhaustiveness, arm
    // narrowing, arm walk -- resolves against it, never against the gate.
    //
    // R5.2: the variant list is *copied out* here, so both branches hand the
    // shared code below the same two parallel vectors -- surface names for
    // arm-tag matching and exhaustiveness, and each variant's narrowed arm
    // input. Copied rather than borrowed because the generic branch reads it
    // through the instantiator's `RefCell`, and the arm walk below re-enters
    // that instantiator (`poly_construct_generic`'s own `drop(generics)` is
    // the precedent).
    //
    // Review fix (R1.1 hazard): a concrete `id` can itself be a body-local
    // mint not yet flushed into `enums` -- gated the same way
    // `type_is_registered` (P7.S3k N1) gates it, on `id.index() <
    // enums.len()`, not on `GenericTypes::enum_base` alone: a caller that
    // builds its own `GenericTypes` without going through `check::check`'s
    // rebase discipline (several unit tests do) can leave `enum_base` at `0`
    // while `enums` is already non-empty, and indexing `inst_enums` by
    // `id.index() - enum_base` in that mismatched state would silently
    // return the *wrong* decl for a small, already-flushed id instead of
    // falling through.
    let (variant_names, narrowed, site_pty): (Vec<String>, Vec<PolyType>, PolyType) =
        match &operative {
            Operative::Concrete(id) => {
                let generics_guard = ctx
                    .generics()
                    .filter(|_| id.index() >= enums.len())
                    .map(|cell| cell.borrow());
                let enum_decl: &EnumDecl = match &generics_guard {
                    Some(g) => g.enum_decl(*id).expect(
                        "an id past `enums.len()` names a mint this batch's own walk just made",
                    ),
                    None => &enums[id.index()],
                };
                (
                    enum_decl
                        .variants
                        .iter()
                        .map(|v| generic_surface_name(&v.name).to_string())
                        .collect(),
                    enum_decl
                        .variants
                        .iter()
                        .enumerate()
                        // Review fix: `variant_type` indexes `enums`
                        // unconditionally -- the same hazard guarded against just
                        // above -- so this builds the `Type::Variant` directly off
                        // the already-resolved decl instead of re-indexing.
                        .map(|(vi, v)| PolyType::Concrete(Type::Variant(*id, vi, v.display_static)))
                        .collect(),
                    PolyType::Concrete(Type::Enum(*id, enum_decl.name_static)),
                )
            }
            // R5.4: each arm's narrowed input carries the *scrutinee's own*
            // argument list, unchanged. Nothing re-unifies: the scrutinee already
            // carries the substitution.
            Operative::Generic {
                idx,
                module,
                args,
                len_args,
            } => {
                // A `PolyType::Generic` slot exists only where the parser resolved
                // a generic header into `GenericTypes`, which is the build path
                // `check_module` threads the live cell through. The two Ctx-less
                // walks cannot hold one: the REPL declares no generic `type:`
                // (P7.S3a D2) and the stand-in runs the concrete checker.
                let cell = ctx
                    .generics()
                    .expect("a `Generic` scrutinee is only built from a live instantiator");
                let generics = cell.borrow();
                let count = generics.enums[*idx as usize].variants.len();
                (
                    generics.enums[*idx as usize]
                        .variants
                        .iter()
                        .map(|v| generic_surface_name(&v.name).to_string())
                        .collect(),
                    (0..count)
                        .map(|vi| {
                            crate::ast::generic_variant_type(
                                &generics,
                                *idx,
                                *module,
                                vi,
                                args.clone(),
                                len_args.clone(),
                            )
                        })
                        .collect(),
                    scrutinee.pt.clone(),
                )
            }
        };
    // P7.S12 (R1.2): record this eliminator call site so `check_poly_call`
    // can ground it against a concrete θ later. A concrete scrutinee is
    // already ground, and `apply_subst`'s `Concrete` arm is a no-op on it;
    // an ungrounded one grounds through the same `apply_subst` route that
    // mints the monomorph, so lowering reads one map either way.
    tctx.enum_sites.push((span, site_pty));

    // Step 3: exhaustiveness and duplication, in written source order and
    // before any arm body is checked.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut variant_indices = Vec::with_capacity(arms.len());
    for (quot, tag) in &arms {
        let literal_span = scope.quotation(*quot).span;
        let Some(vi) = variant_names.iter().position(|v| *v == tag.name) else {
            return Err(eliminator_unknown_variant_error(
                ctx,
                literal_span,
                name,
                &tag.name,
                &enum_name,
            ));
        };
        // R5.7: this slice narrows a generic scrutinee in the **owning** mode
        // only. A `&`/`&!` tag would need `intern_ref_type` over a shape that
        // has no `Type` yet (R4.3's explicit non-goal), so it is a located
        // rejection rather than silently narrowed to owning -- which would
        // let an arm consume a borrowed enum.
        if matches!(operative, Operative::Generic { .. }) && tag.mode != VariantTagMode::Owning {
            return Err(poly_generic_scrutinee_ref_tag_error(
                ctx,
                literal_span,
                name,
                &tag.name,
                &enum_name,
            ));
        }
        if !seen.insert(variant_names[vi].as_str()) {
            return Err(eliminator_duplicate_arm_error(
                ctx,
                literal_span,
                name,
                &tag.name,
                &enum_name,
            ));
        }
        variant_indices.push(vi);
    }
    for variant_surface in &variant_names {
        if !seen.contains(&variant_surface.as_str()) {
            return Err(eliminator_non_exhaustive_error(
                ctx,
                span,
                name,
                variant_surface,
                &enum_name,
            ));
        }
    }

    // Steps 4-5 (OQ4): there is no declared `~[ ..a -- ..b ]` effect to match
    // an arm against -- an arm is annotated by *variant*, and its input is
    // the concrete narrowed variant this dispatch computes. So the poly
    // analogue of `check_literal_against_declared_effect` is a recursive
    // `poly_walk` of the arm body over `(caller row ++ narrowed variant)`,
    // yielding an abstract exit row: the shared arm machinery, with the
    // narrowed variant as each arm's input.
    let base = stack.len() - 1;
    let row: Vec<PolySlot> = stack[..base].to_vec();
    let walk_arms: Vec<PolyArm> = arms
        .iter()
        .zip(&variant_indices)
        .map(|((quot, _), vi)| {
            let mut input = row.clone();
            input.push(PolySlot::new(narrowed[*vi].clone()));
            PolyArm {
                quot: *quot,
                input,
                declared_inputs: vec![narrowed[*vi].clone()],
                // Every eliminator arm runs at most once, in place, in the
                // call's own position -- so all of them inherit the call
                // site's tail-ness (the concrete twin pins `is_arm: true`
                // for the same reason).
                tail,
            }
        })
        .collect();
    // The cross-arm output rule an eliminator supplies: its arms have no
    // declared output row to be held to, so each is compared against the
    // first arm's exit.
    let mut baseline: Option<Vec<PolySlot>> = None;
    poly_walk_arms(
        walk_arms,
        name,
        span,
        scope,
        sig,
        ctx,
        env,
        combinators,
        structs,
        enums,
        arrays,
        cells,
        refs,
        slices,
        builtin_overloads,
        tctx,
        cross,
        &mut |literal_span, exit| match &baseline {
            None => {
                baseline = Some(exit);
                Ok(())
            }
            Some(expected) => poly_arms_agree(expected, &exit, ctx, literal_span, name, sig),
        },
    )?;
    // A zero-variant enum has no arms and no constructible value, so its call
    // is unreachable and `row` is simply handed back untouched.
    Ok(baseline.unwrap_or(row))
}

/// P7.S12 (R1.1/R5.1): which enum header one eliminator *call site*
/// eliminates, read off its own scrutinee rather than off the registry's
/// family gate. A monomorph names itself by `EnumId`; an ungrounded generic
/// names its header by `(idx, module)` and carries the argument list every arm
/// narrows against (R5.4).
enum Operative {
    Concrete(EnumId),
    Generic {
        idx: u32,
        module: u32,
        args: Vec<PolyType>,
        /// P7.S6a (R3): carried forward from the scrutinee's own
        /// `PolyType::Generic.len_args`, unchanged (R5.4's rule, one level
        /// up: nothing re-unifies here either).
        len_args: Vec<Len>,
    },
}

/// P7.S6a (R3, added review round 4): the `Operative::Generic` construction
/// site `poly_eliminator_call` reaches when its scrutinee is an ungrounded
/// `PolyType::Generic { is_enum: true, .. }` -- carries the scrutinee's own
/// `len_args` forward unchanged, mirroring `generic_variant_type`'s own
/// carry-forward one level up (R5.4's rule: nothing re-unifies here either).
fn operative_generic_from_scrutinee(
    idx: u32,
    module: u32,
    args: &[PolyType],
    len_args: &[Len],
) -> Operative {
    Operative::Generic {
        idx,
        module,
        args: args.to_vec(),
        len_args: len_args.to_vec(),
    }
}

/// P7 slice 3b-follow (R1): one arm handed to `poly_walk_arms` -- the literal
/// to walk, the abstract stack its body walks over, and the inline parameter
/// it stands at.
struct PolyArm {
    quot: PolyQuotRef,
    input: Vec<PolySlot>,
    /// The inputs of the parameter named when the arm was written with an
    /// ordinary `[ ... ]` bracket (S3b-follow L4). Held unbuilt:
    /// `inline_quotation_type` leaks its spelling and its effect for the
    /// program's lifetime, so the `Type` is built only when the diagnostic
    /// fires.
    ///
    /// P7.S12 (R5.3): `PolyType`, not `Type` -- an eliminator arm over an
    /// ungrounded generic scrutinee narrows to a `GenericVariant`, which has
    /// no `Type`. The two pre-existing producers (a concrete narrowed variant,
    /// a combinator's grounded declared row) wrap in `PolyType::Concrete`, and
    /// the consumer keeps today's message for the all-`Concrete` case.
    declared_inputs: Vec<PolyType>,
    /// P7.S3g-follow (1a): whether this arm's body occupies the *caller's*
    /// tail position. Per arm, not per call, exactly as the concrete
    /// `LiteralBoundary::is_arm` is: `if`'s two arms do when the `if` does,
    /// `times`' body never does.
    tail: bool,
}

/// P7 slice 3b-follow (R1): the per-arm machinery every quotation-consuming
/// call in a polymorphic body shares -- the per-arm scope clone, the recursive
/// `poly_walk`, the arm-exit escape checks, the `Scope::leave` analogue, and
/// the join that reconciles the clones. What differs between consumers is
/// supplied by the caller: each arm's *input* row (an eliminator's narrowed
/// variant; a combinator's grounded declared row) in `PolyArm`, and the
/// cross-arm *output* rule in `cross_arm`, which sees each arm's exit in
/// written order and is called before the next arm walks, so a disagreement is
/// reported at the arm that introduces it rather than behind a later arm's own
/// error.
///
/// S3b-follow L3: the borrow table is **unioned** here, and this is the only
/// join. The table is keyed by place and a *missing* record reads as "no
/// conflict" (`live_borrow_of` answers `None`), so a second join that
/// intersects or picks one arm would be a silent false accept, not a false
/// reject.
#[allow(clippy::too_many_arguments)]
fn poly_walk_arms(
    arms: Vec<PolyArm>,
    name: &str,
    span: Span,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    cross_arm: &mut dyn FnMut(Span, Vec<PolySlot>) -> Result<(), String>,
) -> Result<(), String> {
    let enclosing_locals: HashSet<String> = scope.locals.keys().cloned().collect();
    let mut arm_moves: Vec<Moves> = Vec::with_capacity(arms.len());
    let mut arm_borrows: Vec<Vec<PolyBorrow>> = Vec::with_capacity(arms.len());
    for arm in arms {
        let lit = scope.quotation(arm.quot);
        let (literal_span, body, is_inline) = (lit.span, lit.body.clone(), lit.is_inline);
        // S3b-follow L4: an arm stands at a parameter declared inline, so an
        // ordinary `[ ... ]` arm is the wrong bracket here exactly as it is on
        // the concrete path -- same diagnostic, so the two paths do not
        // disagree about one spelling.
        //
        // P7.S12 (R5.3): the parameter is spelled out of `declared_inputs`,
        // which is `PolyType` now. An all-`Concrete` row still builds the real
        // interned `Type::InlineQuotation` and keeps today's message
        // byte-identical; a row holding a narrowed `GenericVariant` has no
        // `Type` to intern, so it renders through the `poly_type_str` sibling.
        if !is_inline {
            let concrete: Option<Vec<Type>> = arm
                .declared_inputs
                .iter()
                .map(|pt| match pt {
                    PolyType::Concrete(t) => Some(*t),
                    _ => None,
                })
                .collect();
            return Err(match concrete {
                Some(ins) => ordinary_literal_at_inline_param_error(
                    ctx,
                    literal_span,
                    name,
                    crate::ast::inline_quotation_type(ins, vec![]),
                ),
                None => poly_ordinary_literal_at_inline_param_error(
                    ctx,
                    literal_span,
                    name,
                    &poly_type_str(
                        &PolyType::Quotation(arm.declared_inputs, Vec::new(), true, None, None),
                        sig,
                    ),
                ),
            });
        }
        // Each arm walks its own clone of the enclosing scope, exactly as the
        // concrete path clones `scope` per arm; the join below reconciles the
        // clones.
        let mut arm_scope = scope.clone();
        let exit = poly_walk(
            &body,
            arm.input,
            &mut arm_scope,
            sig,
            ctx,
            env,
            combinators,
            structs,
            enums,
            arrays,
            cells,
            refs,
            slices,
            builtin_overloads,
            tctx,
            cross,
            arm.tail,
        )?;
        // Neither a `Type::Variant` nor its narrowed-generic twin may leave
        // the call. Every type-directed predicate outside the eliminator is
        // written over `Type::Enum`, so `is_copy` reads an escaped variant as
        // trivially `Copy` and a later `dup` double-drops a linear payload.
        //
        // P7.S12 (R3.4): one exhaustive classification rather than a pair of
        // matches ending in `_ => None`, so a `PolyType` variant added later
        // cannot escape here in silence -- this is the load-bearing site.
        // The two errors differ only in rendering: a `GenericVariant` has no
        // concrete `Type`, so it goes through the `poly_type_str` sibling.
        for slot in &exit {
            // One `Ref` layer is peeled first, matching the depth the
            // concrete arm has always looked to.
            let escaping = match &slot.pt {
                PolyType::Ref(referent, _) => referent.as_ref(),
                pt => pt,
            };
            match escaping {
                PolyType::Concrete(t @ Type::Variant(..)) => {
                    return Err(eliminator_variant_escape_error(ctx, literal_span, name, *t));
                }
                pt @ PolyType::GenericVariant { .. } => {
                    return Err(poly_eliminator_variant_escape_error(
                        ctx,
                        literal_span,
                        name,
                        &poly_type_str(pt, sig),
                    ));
                }
                PolyType::Concrete(_)
                | PolyType::Var(_)
                | PolyType::Array(..)
                | PolyType::Quotation(..)
                | PolyType::Ref(..)
                | PolyType::OwnedCell(_)
                | PolyType::QuotLit
                | PolyType::Generic { .. }
                // P7b.S1 (S1-16): an application is not a narrowed variant
                // (that is `GenericVariant`'s own shape), so it never
                // escapes here either.
                | PolyType::App { .. } => {}
            }
            // S3b L2: nor may a quotation literal, which would then have to be
            // materialised to exist past the arm. Its own span, not the
            // arm's: a quotation nested inside the arm body is not written
            // where the arm literal is.
            if let Some(quot) = slot.quot {
                return Err(poly_quotation_not_consumed_error(
                    ctx,
                    arm_scope.quotation(quot).span,
                ));
            }
        }
        // The poly analogue of `Scope::leave`. The poly walk has no block
        // scope: `poly_term`'s `Bind` inserts into `locals`/`moves` and
        // nothing removes them. Without this, an arm-bound linear local leaks
        // unreported, *and* `Moves::join` (which indexes the other arm's map
        // by the first arm's keys) panics the moment two arms bind different
        // names. Reject first, then truncate, so the leak is a diagnostic
        // rather than something the truncation quietly erases.
        let leaked = arm_scope
            .moves
            .unconsumed()
            .into_iter()
            .find(|local| !enclosing_locals.contains(*local))
            .map(str::to_string);
        if let Some(local) = leaked {
            let pt = arm_scope.locals[&local].clone();
            return Err(poly_arm_local_not_consumed_error(
                ctx,
                literal_span,
                name,
                &local,
                &poly_type_str(&pt, sig),
            ));
        }
        arm_scope.locals.retain(|k, _| enclosing_locals.contains(k));
        arm_scope
            .moves
            .states
            .retain(|k, _| enclosing_locals.contains(k));
        arm_moves.push(arm_scope.moves);
        arm_borrows.push(arm_scope.borrows);
        cross_arm(literal_span, exit)?;
    }

    // S3b-follow L3: the borrow table is **unioned**, not picked or
    // intersected. It is keyed by place and a *missing* record reads as "no
    // conflict", so dropping one arm's record is a silent false accept: arm
    // A's `&!x` and arm B's `&!y` must both survive, or a later use of
    // whichever was dropped is wrongly admitted. A genuine disagreement (one
    // place, two mutabilities) is rejected rather than erased.
    for borrows in arm_borrows {
        for borrow in borrows {
            match scope.borrows.iter().find(|b| b.place == borrow.place) {
                Some(existing) if existing.mutable != borrow.mutable => {
                    return Err(poly_arm_borrow_disagreement_error(
                        ctx, span, name, existing, &borrow,
                    ));
                }
                Some(_) => {}
                None => scope.borrows.push(borrow),
            }
        }
    }
    // The move-state join, generalized from the concrete path's two arms to N
    // by the same reduction. Every arm now presents the enclosing key set
    // (the `leave` analogue above), which is what makes `Moves::join`'s
    // indexing sound here. With no arms at all there is nothing to join and
    // `scope` is left untouched.
    if let Some(joined) = arm_moves.into_iter().reduce(Moves::join) {
        scope.moves = joined;
    }
    Ok(())
}

/// The exit row of one eliminator arm, rendered for the cross-arm shape
/// diagnostic.
fn poly_row_str(row: &[PolySlot], sig: &PolySig) -> String {
    match row.is_empty() {
        true => "nothing".to_string(),
        false => format!(
            "`{}`",
            row.iter()
                .map(|slot| poly_type_str(&slot.pt, sig))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

/// P7 slice 3b-follow (R1/L1): the cross-arm output rule both quotation
/// consumers share -- sibling arms leaving one exit row, compared
/// **structurally under rigid type variables**. `'T` in one arm against `'U`,
/// or against `i64`, disagrees: binding either would be a mid-body
/// unification this slice does not do, and could not undo across the sibling
/// arms already checked.
fn poly_arms_agree(
    want: &[PolySlot],
    found: &[PolySlot],
    ctx: &Ctx,
    span: Span,
    name: &str,
    sig: &PolySig,
) -> Result<(), String> {
    if want.len() != found.len() {
        return Err(combinator_branch_output_mismatch_rendered(
            ctx,
            span,
            name,
            &poly_row_str(want, sig),
            &poly_row_str(found, sig),
        ));
    }
    for (a, b) in want.iter().zip(found) {
        if a.pt != b.pt {
            return Err(poly_arm_output_disagreement_error(
                ctx,
                span,
                name,
                &poly_type_str(&a.pt, sig),
                &poly_type_str(&b.pt, sig),
            ));
        }
    }
    Ok(())
}

/// P7 slice 3b-follow (R2): the row-typed inline combinator `name` resolves
/// to, if any -- the declaration that drives `poly_combinator_call`.
///
/// A *row* on some quotation parameter is the entry condition: this dispatch
/// grounds that row against the abstract stack, and a combinator declaring
/// only rowless quotation parameters is the concrete-consumer shape (P7.S3d),
/// which keeps the located rejection it has today rather than being admitted
/// through machinery built for a different question.
///
/// A name carrying *two* candidates declines: picking between combinator
/// overloads is `resolve_combinator_overload`'s job over concrete operand
/// types, and there is no poly analogue of it. Declining leaves the call with
/// the located rejection it already had, never an accept.
fn poly_row_combinator<'a>(combinators: &'a CombinatorEnv, name: &str) -> Option<&'a PolySig> {
    let [only] = combinators.get(name)?.as_slice() else {
        return None;
    };
    let csig = only.word.poly.as_deref()?;
    csig.inputs
        .iter()
        .any(|pin| {
            matches!(pin, PolyType::Quotation(_, _, _, row_in, row_out) if row_in.is_some() || row_out.is_some())
        })
        .then_some(csig)
}

/// P7 slice 3b-follow (R3): one declared quotation parameter, grounded. The
/// fixed slots are concrete `Type`s (a variable-carrying declaration is
/// rejected before this is built), and `carries_row` is whether the parameter
/// declared the signature's row, which is what decides between grounding
/// against the caller region and grounding against the empty one.
struct DeclaredArm {
    ins: Vec<Type>,
    outs: Vec<Type>,
    carries_row: bool,
    /// The declared *output* row's id when it differs from the input row's:
    /// the shape-changing case (`if`/`unless`), whose exit the declaration
    /// does not fix at all -- only agreement between the sibling arms sharing
    /// this id does (R3). `None` is the non-shape-changing case, whose exit is
    /// the row it entered with, then the declared fixed outputs.
    row_out: Option<u32>,
}

/// P7 slice 3b-follow (R3): what one arm's exit is checked against.
enum ArmRule {
    /// The exit the declaration fixes -- the grounded region the arm entered
    /// with, then the parameter's declared outputs -- built **before any arm
    /// walks** (R3, the soundness point): a single-arm combinator like `times`
    /// has no sibling for a cross-arm rule to compare it against, so nothing
    /// else would hold it to its declared `~[ ..a -- ..a ]` and `~[ dup ]
    /// times` would lower to a loop whose back-edge depth misses its entry.
    /// This is the poly port of `check_literal_against_declared_effect` under
    /// `LiteralBoundary { shape_changing: false }`.
    Fixed {
        want: Vec<PolySlot>,
        declared: String,
    },
    /// The shape-changing case (`if`/`unless`): the declaration fixes no exit
    /// row, only a suffix above it -- the arm's exit is `region ++ suffix`,
    /// checked here against the declared suffix types (`outs`, `declared` for
    /// rendering), and the stripped region is what the arms sharing this
    /// declared output row id (`u32`) are held to against each other.
    Row(u32, Vec<Type>, String),
}

/// Classify one declared parameter of a row-typed combinator. `Ok(None)` is an
/// ordinary value parameter, which the caller matches against its live slot.
fn poly_declared_arm(
    pin: &PolyType,
    csig: &PolySig,
    name: &str,
    ctx: &Ctx,
    span: Span,
) -> Result<Option<DeclaredArm>, String> {
    let abstract_ =
        || poly_combinator_abstract_signature_error(ctx, span, name, &poly_type_str(pin, csig));
    match pin {
        PolyType::Quotation(ins, outs, _, row_in, row_out) => {
            let ground = |slots: &[PolyType]| -> Option<Vec<Type>> {
                slots
                    .iter()
                    .map(|p| match p {
                        PolyType::Concrete(t) => Some(*t),
                        _ => None,
                    })
                    .collect()
            };
            let (Some(ins), Some(outs)) = (ground(ins), ground(outs)) else {
                return Err(abstract_());
            };
            // R3: whether the parameter's two declared rows are the *same*
            // row is what decides how its arm's exit is checked. Nothing here
            // has to relate them to the signature's own rows: the parser
            // already refuses a row inside a quotation effect that is not the
            // signature's own top-level row, and refuses one named on a single
            // side of the parameter.
            let row_out = match (row_in, row_out) {
                (Some(a), Some(b)) if a != b => {
                    // Shape-changing (`if`/`unless`): nothing fixes the exit
                    // row but sibling agreement, and the produced row is read
                    // straight off an arm's exit -- so a declared suffix
                    // above that row (`outs`) is stripped back off it first
                    // (`ArmRule::Row`), the poly port of
                    // `check_literal_against_declared_effect`'s
                    // shape-changing branch (`src/check.rs:2124`).
                    Some(*b)
                }
                // One row on both sides (`times`), or none at all (the P7.S3d
                // rowless shape, reached here only alongside a row-bearing
                // sibling parameter): the declaration fixes the exit.
                _ => None,
            };
            Ok(Some(DeclaredArm {
                ins,
                outs,
                carries_row: row_in.is_some(),
                row_out,
            }))
        }
        // Slice 10a (R1): a fully-concrete declared effect folds to
        // `Concrete`, so this is the same parameter shape with no variable and
        // no row -- it grounds against the empty region (R3).
        PolyType::Concrete(t) => Ok(crate::ast::is_quotation_type(*t).map(|eff| DeclaredArm {
            ins: eff.inputs.clone(),
            outs: eff.outputs.clone(),
            carries_row: false,
            row_out: None,
        })),
        _ => Ok(None),
    }
}

/// P7 slice 3b-follow (R3): the abstract twin of `check_poly_combinator_args`
/// -- a **row-typed inline combinator** called from a non-inline polymorphic
/// body, whose quotation arms are literals written in that body. This is what
/// lets a generic word branch and loop as a monomorphized function instead of
/// forcing every call site to splice its whole body.
///
/// The row grounds **once**, here, to the caller region below the combinator's
/// fixed inputs (S3b-follow L2), and is never solved for; the callee's own
/// declaration must otherwise be concrete, since binding a variable of it
/// would be the mid-body unification L1 forbids. Each arm is then walked over
/// `(grounded region ++ declared inputs)` by the shared arm machinery, which
/// owns the join (L3).
///
/// What an arm's exit is held to is the parameter's declared row *pair*
/// (`ArmRule`): one row on both sides (`times`) fixes the exit, so it is built
/// here before any arm walks; two rows (`if`/`unless`) fix nothing, so the arms
/// sharing that output row are held to each other and their agreed exit *is*
/// the call's exit row.
#[allow(clippy::too_many_arguments)]
fn poly_combinator_call(
    csig: &PolySig,
    name: &str,
    span: Span,
    stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    combinators: &CombinatorEnv,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    builtin_overloads: &mut HashMap<Span, String>,
    tctx: &mut TraitCtx,
    cross: &mut CrossCtx,
    tail: bool,
) -> Result<Vec<PolySlot>, String> {
    // P7.S3g-follow (1a): which of the callee's parameters hold a quotation it
    // `call`s in *tail* position -- `if`'s two arms, never `times`' body. The
    // same set, from the same accessor, the concrete argument-site literal
    // check reads to answer this per parameter, so the poly walk's notion of
    // tail position stays in lockstep with `tail_position_calls`/`lower_terms`.
    //
    // No source program can currently witness the refinement (crediting every
    // arm with the caller's tail position instead passes the whole suite): the
    // only thing that reads `tail` is the back-edge reference guard, and it
    // needs a reference in the self-call's window, which in turn must come
    // from a reference *parameter* or a body borrow -- and either one keeps
    // every recorded borrow live under `prune_dead_borrows`' coarse liveness,
    // so a body that borrows a local in *any* arm and then tail-recurses is
    // rejected whichever arm the borrow sat in.
    let tail_slots = tail_called_param_slots(name, combinators.tail());
    let n = csig.inputs.len();
    if stack.len() < n {
        return Err(underflow_error(ctx, span, name, n, stack.len()));
    }
    let base = stack.len() - n;
    // L2: the declared row grounds here, once, to the caller region below the
    // combinator's fixed inputs -- the same region the concrete path grounds
    // it to (`check_poly_combinator_args`).
    let row: Vec<PolySlot> = stack[..base].to_vec();
    let mut outputs: Vec<PolySlot> = Vec::with_capacity(csig.outputs.len());
    for out in &csig.outputs {
        let PolyType::Concrete(t) = out else {
            return Err(poly_combinator_abstract_signature_error(
                ctx,
                span,
                name,
                &poly_type_str(out, csig),
            ));
        };
        outputs.push(PolySlot::new(PolyType::Concrete(*t)));
    }
    let mut arms: Vec<PolyArm> = Vec::new();
    let mut rules: Vec<ArmRule> = Vec::new();
    for (i, pin) in csig.inputs.iter().enumerate() {
        let Some(decl) = poly_declared_arm(pin, csig, name, ctx, span)? else {
            // An ordinary value parameter (`times`' iteration count), matched
            // against the live slot exactly as the `env` dispatch matches a
            // monomorphic word's declared input.
            let PolyType::Concrete(want) = pin else {
                return Err(poly_combinator_abstract_signature_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pin, csig),
                ));
            };
            match &stack[base + i].pt {
                PolyType::Concrete(t) if t == want => {}
                PolyType::Concrete(t) => {
                    return Err(type_mismatch_error(ctx, span, name, *want, *t))
                }
                PolyType::Var(v) => {
                    return Err(poly_var_to_concrete_error(
                        ctx,
                        span,
                        name,
                        &sig.ty_var_names[*v as usize],
                        *want,
                    ))
                }
                other => return Err(poly_op_on_variable_error(ctx, span, name, other, sig)),
            }
            continue;
        };
        // OQ4: the arm is a splice-consumed literal written at this call site
        // or it is an error. A quotation that lost its identity (bound to a
        // local), a forwarded parameter, or a value that is not a quotation at
        // all is located here rather than carried into lowering, where a
        // materialised quotation in a generic body is a backend panic.
        let Some(quot) = stack[base + i].quot else {
            // A literal that went through a `| f |` bind keeps its type and
            // loses its identity (S3b L3: `PolyScope::locals` carries no
            // `PolyQuotRef`), so rendering that type would answer "needs a
            // quotation literal" with "found a quotation literal".
            let found = match &stack[base + i].pt {
                PolyType::QuotLit => "a quotation read back out of a local".to_string(),
                pt => format!("`{}`", poly_type_str(pt, sig)),
            };
            return Err(poly_combinator_arm_not_a_literal_error(
                ctx, span, name, &found,
            ));
        };
        let region = match decl.carries_row {
            true => row.clone(),
            false => Vec::new(),
        };
        let mut input = region.clone();
        input.extend(
            decl.ins
                .iter()
                .map(|t| PolySlot::new(PolyType::Concrete(*t))),
        );
        rules.push(match decl.row_out {
            Some(rid) => {
                // P7.S3j: the arms sharing an output row feed one join and one
                // continuation, so the suffix declared above that row has to
                // be the *same* suffix on each of them -- and stripping each
                // parameter's own suffix off its own arm is what would
                // otherwise hide a difference from the cross-arm rule below,
                // leaving a slot the call's exit row has no account of.
                // Rejected here, before any arm walks, for `ArmRule::Fixed`'s
                // reason: no arm-against-sibling rule holds a lone arm.
                //
                // Keyed by row id like `shape_baseline` below, though no
                // program can tell the keying from a bare scan today: a
                // parameter's *input* row must already be declared where it is
                // written, so it can only be the signature's top-level input
                // row, and a differing output row can then only be the
                // top-level output one -- every `Row` rule shares that id.
                // `~[ ..b -- ..a i64 ]` is a parse error, not a second id.
                let disagrees = rules.iter().any(|r| {
                    matches!(r, ArmRule::Row(other, outs, _) if *other == rid && *outs != decl.outs)
                });
                if disagrees {
                    return Err(poly_combinator_abstract_signature_error(
                        ctx,
                        span,
                        name,
                        &poly_type_str(pin, csig),
                    ));
                }
                ArmRule::Row(rid, decl.outs.clone(), poly_type_str(pin, csig))
            }
            None => {
                let mut want = region;
                want.extend(
                    decl.outs
                        .iter()
                        .map(|t| PolySlot::new(PolyType::Concrete(*t))),
                );
                ArmRule::Fixed {
                    want,
                    declared: poly_type_str(pin, csig),
                }
            }
        });
        arms.push(PolyArm {
            quot,
            input,
            // R5.3: a combinator's declared row is already ground `Type`s.
            declared_inputs: decl.ins.iter().copied().map(PolyType::Concrete).collect(),
            tail: tail && tail_slots.contains(&i),
        });
    }
    // R3: the shape-changing arms' agreed exit row, keyed by the output row id
    // they share, the first arm to reach it setting the baseline. This is the
    // only thing that fixes a shape-changing exit -- the declaration says only
    // that both `if` arms leave the same `..b`, not what `..b` is.
    let mut shape_baseline: HashMap<u32, Vec<PolySlot>> = HashMap::new();
    // The arms reach the rule below in written order, so the rule the
    // parameter each one stands at carries is read off by position.
    let mut at = 0usize;
    poly_walk_arms(
        arms,
        name,
        span,
        scope,
        sig,
        ctx,
        env,
        combinators,
        structs,
        enums,
        arrays,
        cells,
        refs,
        slices,
        builtin_overloads,
        tctx,
        cross,
        &mut |literal_span, exit| {
            let rule = &rules[at];
            at += 1;
            match rule {
                ArmRule::Fixed { want, declared } => {
                    // L1: structural equality under *rigid* variables -- an
                    // arm leaving `'T` where the row it entered with carries
                    // `i64` disagrees, and binding either would be a mid-body
                    // unification.
                    let agrees = want.len() == exit.len()
                        && want.iter().zip(&exit).all(|(a, b)| a.pt == b.pt);
                    match agrees {
                        true => Ok(()),
                        false => Err(poly_arm_declared_effect_mismatch_error(
                            ctx,
                            literal_span,
                            name,
                            declared,
                            &poly_row_str(&exit, sig),
                            &poly_row_str(want, sig),
                        )),
                    }
                }
                ArmRule::Row(rid, suffix, declared) => {
                    // R2: the arm's exit is `region ++ suffix` -- strip the
                    // declared trailing slots back off first (the poly port
                    // of `check_literal_against_declared_effect`'s
                    // shape-changing branch, `src/check.rs:2124`), and hold
                    // only the stripped region to the cross-arm agreement
                    // below.
                    let split_at = exit.len().saturating_sub(suffix.len());
                    let (region, tail) = exit.split_at(split_at);
                    let suffix_matches = tail.len() == suffix.len()
                        && tail
                            .iter()
                            .zip(suffix)
                            .all(|(s, t)| matches!(s.pt, PolyType::Concrete(u) if u == *t));
                    if !suffix_matches {
                        let want: Vec<PolySlot> = suffix
                            .iter()
                            .map(|t| PolySlot::new(PolyType::Concrete(*t)))
                            .collect();
                        return Err(poly_arm_declared_suffix_mismatch_error(
                            ctx,
                            literal_span,
                            name,
                            declared,
                            &poly_row_str(tail, sig),
                            &poly_row_str(&want, sig),
                        ));
                    }
                    let region = region.to_vec();
                    match shape_baseline.get(rid) {
                        Some(want) => poly_arms_agree(want, &region, ctx, literal_span, name, sig),
                        None => {
                            shape_baseline.insert(*rid, region);
                            Ok(())
                        }
                    }
                }
            }
        },
    )?;
    // The exit row. Non-shape-changing (`csig.row_out` is `csig.row_in`, both
    // of them possibly absent): the declaration's own, the grounded row back
    // untouched. Shape-changing: what the arms agreed on, which is the only
    // account of it there is -- so a signature promising an output row no arm
    // produces has nothing to hand back and keeps the located rejection rather
    // than being answered with the entry row it explicitly differs from.
    let mut exit = if csig.row_out == csig.row_in {
        row
    } else {
        let Some(named) = csig.row_out.or(csig.row_in) else {
            unreachable!("the two rows differ, so one of them is set")
        };
        match csig.row_out.and_then(|rid| shape_baseline.remove(&rid)) {
            Some(agreed) => agreed,
            // No account of the exit row: either no parameter produces the row
            // the signature promises, or the signature takes a row and hands
            // none back, which would drop the caller's region on the floor.
            None => {
                return Err(poly_combinator_abstract_signature_error(
                    ctx,
                    span,
                    name,
                    &csig.row_var_names[named as usize],
                ))
            }
        }
    };
    exit.extend(outputs);
    Ok(exit)
}

/// Slice 13 (R-B1): every `&`-led word reaching a polymorphic body -- the
/// prefix borrow (`&x`/`&!x`) and the array-element accessor (`&>`/`&!>`),
/// plus the permanently out-of-scope owning-cell/struct-field family (`&^`,
/// `&Struct>field`, R-B6) rejected regardless of mutability. Mirrors
/// `check_reference_word` fronting the monomorphic family: `Ok(None)` if
/// `name` is not `&`-led, and the caller then falls through to the ordinary
/// lookup chain.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_reference_word(
    name: &str,
    span: Span,
    stack: &mut Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    _structs: &[StructDecl],
    _enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    slices: &[SliceDecl],
) -> Result<Option<Vec<PolySlot>>, String> {
    if !name.starts_with('&') {
        return Ok(None);
    }
    let mutable = name.starts_with("&!");
    let rest = &name[if mutable { 2 } else { 1 }..];
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);

    // R-B6: `&^` never produces a variable-referent ref (no generic
    // structs/enums this slice) and is out of scope for a generic body
    // regardless of mutability. Any other `>`-bearing name but `&>` (the array
    // index) is a retired fused-accessor spelling. Both are located errors
    // here, never a silent fallthrough to an eventual unknown-word one.
    if rest == "^" || (rest != ">" && rest.contains('>')) {
        return Err(poly_unsupported_accessor_error(ctx, span, name));
    }

    match rest {
        ">" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let index_pt = stack[n - 1].pt.clone();
            let index_lit = stack[n - 1].int_val;
            let receiver = stack[n - 2].pt.clone();
            // P7 slice 3c (R9.2, phase 4): a slice receiver, matched ahead of
            // the array extraction exactly as the monomorphic twin matches
            // ahead of `ref_parts` -- a slice is not a `PolyType::Ref`, so
            // `poly_ref_array_parts` would send it to the "not a reference to
            // an array" error instead of indexing it.
            if let PolyType::Concrete(recv_ty @ Type::Slice(id, recv_mut, _)) = receiver {
                if recv_mut != mutable {
                    return Err(reference_word_operand_error(
                        ctx,
                        span,
                        name,
                        if mutable {
                            "a mutable slice"
                        } else {
                            "a slice"
                        },
                        recv_ty,
                    ));
                }
                check_poly_slice_offset(&stack[n - 1], ctx, span, name, sig)?;
                let elem = slices[id.index()].element;
                stack.truncate(n - 2);
                stack.push(PolySlot::new(PolyType::Ref(
                    Box::new(PolyType::Concrete(elem)),
                    mutable,
                )));
                return Ok(Some(std::mem::take(stack)));
            }
            let Some((recv_mut, elem, len)) = poly_ref_array_parts(&receiver, arrays) else {
                return Err(poly_op_on_variable_error(ctx, span, name, &receiver, sig));
            };
            if recv_mut != mutable {
                // A mutability mismatch is a *type* mismatch, not "this op
                // rejects references" -- the monomorphic twin says
                // `` `&>` expected `&array[i64 4]`, found `&!array[i64 4]` ``, and the
                // two sides here are the same array shape under the two
                // sigils, so both render off one normalized referent.
                let referent = PolyType::Array(Box::new(elem), len);
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(&PolyType::Ref(Box::new(referent.clone()), mutable), sig),
                    &poly_type_str(&PolyType::Ref(Box::new(referent), recv_mut), sig),
                ));
            }
            let count = match len {
                Len::Concrete(count) => Some(count),
                // P7.S6c (R1.1): a literal index against an unknown length
                // still cannot be statically bounds-checked, so it is still
                // rejected here. Anything else (a `usize` local, a `>usize`
                // conversion's result, or a computed non-literal `i64`)
                // defers to the runtime guard exactly as a `Len::Concrete`
                // computed index already does.
                Len::Var(v) => {
                    if index_lit.is_some() {
                        return Err(poly_generic_length_index_error(
                            ctx,
                            span,
                            &sig.len_var_names[v as usize],
                        ));
                    }
                    None
                }
            };
            check_poly_array_index(&index_pt, index_lit, count, ctx, span, name, sig)?;
            stack.truncate(n - 2);
            stack.push(PolySlot::new(PolyType::Ref(Box::new(elem), mutable)));
        }
        _ => {
            if rest.is_empty() {
                return Err(poly_borrow_of_non_place_error(ctx, span, name));
            }
            // R1's resolution order: a bound local first, then a static of
            // this module, mirroring the monomorphic `check_reference_word`.
            let (referent_pt, static_rooted) = if let Some(local_pt) =
                scope.locals.get(rest).cloned()
            {
                // D5: only an aggregate local is borrowable -- a bare type
                // variable might instantiate to a scalar, which has no
                // address, so the conservative rule refuses every
                // bare-variable local uniformly rather than deferring the
                // question to instantiation.
                let is_aggregate = matches!(local_pt, PolyType::Array(..))
                    || matches!(
                        local_pt,
                        PolyType::Concrete(
                            Type::Struct(..)
                                | Type::Enum(..)
                                | Type::Array(..)
                                | Type::OwnedCell(..)
                        )
                    );
                if !is_aggregate {
                    let ty_str = poly_type_str(&local_pt, sig);
                    let is_quotation = matches!(local_pt, PolyType::Quotation(..))
                        || matches!(local_pt, PolyType::Concrete(t) if crate::ast::is_quotation_type(t).is_some());
                    return Err(match local_pt {
                        _ if is_quotation => {
                            poly_borrow_of_quotation_local_error(ctx, span, rest, &ty_str)
                        }
                        PolyType::Var(_) => {
                            poly_borrow_of_variable_local_error(ctx, span, rest, &ty_str)
                        }
                        _ => poly_borrow_of_non_aggregate_local_error(ctx, span, rest, &ty_str),
                    });
                }
                // Borrowing is not a move, but the referent still has to be
                // there: a local already consumed holds nothing, exactly the
                // monomorphic `use_after_move_error` reason for the same op.
                if let Some(site) = scope.moves.moved_site(rest) {
                    return Err(poly_use_after_move_error(ctx, span, rest, site));
                }
                (local_pt, false)
            } else if let Some(static_ty) = ctx.static_type(rest) {
                // R1: a *scalar* static is borrowable though a scalar local
                // is not -- a static has a data-symbol address to hand out.
                // Never moved or dropped, so the move gate above has nothing
                // to say about it.
                (PolyType::Concrete(static_ty), true)
            } else if receiver_is_aggregate_projection(stack) {
                // P7 slice 1 (R1): `&f` carries no `>`, so the guard above
                // cannot see that this is a field projection. Reached only
                // after both the local and the static lookup miss (a real
                // local named `f` is unaffected), so a struct/variant on top
                // of the stack means the name is a projection out of it --
                // still unsupported in a generic body, but it must say so
                // rather than claim `f` is not a local.
                return Err(poly_unsupported_accessor_error(ctx, span, name));
            } else {
                return Err(poly_borrow_of_non_local_error(ctx, span, name, rest));
            };
            // Exclusivity (R-B5/OQ1), symmetric and per place: a new mutable
            // borrow conflicts with any live borrow of this place, a new
            // shared one with a live mutable borrow. Two live `&!` rooted at
            // different locals do not conflict. Enforced here, in the poly
            // body, because a plain generic word is checked once and never
            // re-checked concretely at its instantiations -- a hazard missed
            // here is missed everywhere.
            scope.prune_dead_borrows(stack);
            if let Some(live) = scope.live_borrow_of(rest, !mutable) {
                return Err(poly_conflicting_borrow_error(
                    ctx, span, rest, mutable, live,
                ));
            }
            scope.borrows.push(PolyBorrow {
                place: rest.to_string(),
                mutable,
                span,
                static_rooted,
            });
            stack.push(PolySlot::new(PolyType::Ref(Box::new(referent_pt), mutable)));
        }
    }
    Ok(Some(std::mem::take(stack)))
}

/// Slice 13 (R-B3): the array shape a poly-body `&>`/`&!>` receiver borrows,
/// as `(mutable, element, length)` -- a variable-bearing `PolyType::Array`
/// directly, or a fully concrete array folded to `Concrete(Type::Array)` and
/// looked up in the registry, mirroring the two representations
/// `raw_to_poly_type` leaves for an array shape. `None` if `pt` is not a
/// reference to an array at all.
fn poly_ref_array_parts(pt: &PolyType, arrays: &[ArrayDecl]) -> Option<(bool, PolyType, Len)> {
    let PolyType::Ref(referent, mutable) = pt else {
        return None;
    };
    match referent.as_ref() {
        PolyType::Array(elem, len) => Some((*mutable, (**elem).clone(), len.clone())),
        PolyType::Concrete(Type::Array(id, _)) => {
            let decl = &arrays[id.index()];
            Some((
                *mutable,
                PolyType::Concrete(decl.element),
                Len::Concrete(decl.count),
            ))
        }
        _ => None,
    }
}

/// Slice 13 (R-B3): `&>`/`&!>`'s static bounds check against a concrete
/// count, the poly-body twin of the monomorphic `check_array_index`. The
/// caller passes the index slot's `int_val` alongside its `PolyType`; this is
/// the only consumer of that field, which is why every operator but a bare
/// shuffle leaves it `None`. P7.S6c (R2.1): `count` is `None` for a
/// generic-length array (`Len::Var`), deferring entirely to the runtime
/// bounds guard the same way a `Len::Concrete` computed index already does.
#[allow(clippy::too_many_arguments)]
fn check_poly_array_index(
    index_pt: &PolyType,
    index_lit: Option<i64>,
    count: Option<u32>,
    ctx: &Ctx,
    span: Span,
    op: &str,
    sig: &PolySig,
) -> Result<(), String> {
    match index_pt {
        PolyType::Concrete(Type::Usize) => Ok(()),
        PolyType::Concrete(Type::I64) => match index_lit {
            Some(idx) => match count {
                Some(c) if idx >= 0 && idx < i64::from(c) => Ok(()),
                Some(c) => Err(array_index_out_of_range_error(ctx, span, c, idx)),
                // P7.S6c (R2.2): a literal index against an unknown length is
                // intercepted at the `Len::Var` call site (R1.1) before this
                // function is ever reached.
                None => unreachable!(
                    "a literal index against an unknown length is intercepted at the \
                     Len::Var call site (R1.1) before this function is ever reached"
                ),
            },
            // A computed (non-literal) `i64` index needs the explicit
            // `>usize` conversion the monomorphic checker also requires;
            // there is no value here to bounds-check at compile time.
            None => Err(size_conversion_needed_error(ctx, span, op, Type::Usize)),
        },
        other => Err(poly_op_on_variable_error(ctx, span, op, other, sig)),
    }
}

/// P7 slice 3c (R9.2/R10.1, phase 4): the poly twin of `check_slice_offset`
/// -- a slice index, or `subslice`'s start and length. A slice carries its
/// length at runtime, so unlike `check_poly_array_index` there is no count to
/// bound a literal against and an `i64` literal is admitted the same way the
/// monomorphic path admits one (`match_slot`'s `LiteralSizeType`).
fn check_poly_slice_offset(
    offset: &PolySlot,
    ctx: &Ctx,
    span: Span,
    op: &str,
    sig: &PolySig,
) -> Result<(), String> {
    match &offset.pt {
        PolyType::Concrete(Type::Usize) => Ok(()),
        PolyType::Concrete(Type::I64) if offset.int_val.is_some() => Ok(()),
        PolyType::Concrete(Type::I64) => {
            Err(size_conversion_needed_error(ctx, span, op, Type::Usize))
        }
        other => Err(poly_op_on_variable_error(ctx, span, op, other, sig)),
    }
}

/// R7: `dup`/`over`'s `Copy` gate on a `PolyType` slot. A bare variable
/// missing the `Copy` bound is X7 (naming the variable and the missing bound,
/// with the linear-spine reason); a concrete linear slot reuses the ordinary
/// `cannot_copy` diagnostic.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_copy_gate(
    pt: &PolyType,
    op: &str,
    sig: &PolySig,
    ctx: &Ctx,
    span: Span,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    if poly_is_copy(pt, sig, structs, enums, arrays) {
        return Ok(());
    }
    match pt {
        PolyType::Var(v) => Err(poly_copy_body_error(
            ctx,
            span,
            op,
            &sig.ty_var_names[*v as usize],
        )),
        PolyType::Concrete(t) => Err(cannot_copy_error(ctx, span, op, *t)),
        // P7 slice 3b (R2): `dup`/`over` on a quotation literal. Located, and
        // rendered by the shared operand renderer below.
        PolyType::QuotLit => Err(poly_op_on_variable_error(ctx, span, op, pt, sig)),
        // A variable-bearing array is non-`Copy` exactly when its element is
        // (a length-variable array is never interned, so no declaration-site
        // gate ever sees it). Recurse so the error names the real offending
        // element, an unbounded variable or a linear concrete type, never a
        // fabricated one.
        PolyType::Array(elem, _) => {
            poly_copy_gate(elem, op, sig, ctx, span, structs, enums, arrays)
        }
        // Unreachable: `poly_is_copy` returns `true` for a quotation effect
        // (D3), so this gate returns above before reaching the error arm.
        PolyType::Quotation(..) => {
            unreachable!("a quotation effect is always Copy (D3); the gate returns above")
        }
        // Slice 13 (E1): only the *mutable* arm reaches here -- a shared
        // reference is `Copy` and returned above.
        PolyType::Ref(..) => Err(poly_copy_mutable_ref_error(
            ctx,
            span,
            op,
            &poly_type_str(pt, sig),
        )),
        // P7.S3n (R3): an owning cell is linear at every instantiation --
        // `poly_is_copy` answers `false` unconditionally -- so this always
        // reaches the error, exactly as the concrete `^i64` does through
        // `cannot_copy_error`.
        PolyType::OwnedCell(_) => Err(poly_copy_owned_cell_error(
            ctx,
            span,
            op,
            &poly_type_str(pt, sig),
        )),
        // P7 slice 3a (D5/R5.4): `poly_is_copy` never returns `true` for a
        // generic applied to a variable, so this always reaches the error.
        PolyType::Generic { .. } => Err(poly_copy_generic_error(
            ctx,
            span,
            op,
            &poly_type_str(pt, sig),
        )),
        // P7.S12 (R3.1): `poly_is_copy` never returns `true` for a narrowed
        // generic variant -- it wraps a possibly-linear payload -- so this
        // always reaches the error.
        PolyType::GenericVariant { .. } => Err(poly_copy_generic_variant_error(
            ctx,
            span,
            op,
            &poly_type_str(pt, sig),
        )),
        // P7b.S1 (S1-16): `poly_is_copy` never returns `true` for an
        // application, mirroring `Generic`; reuse its diagnostic.
        PolyType::App { .. } => Err(poly_copy_generic_error(
            ctx,
            span,
            op,
            &poly_type_str(pt, sig),
        )),
    }
}

/// Delegate an operator whose operands are concrete: run it over the maximal
/// concrete suffix of the `PolyType` stack, then map the result back to
/// concrete slots. `None` if the name is not a concrete operator (the caller
/// then reports an unknown word).
pub(super) fn poly_delegate_op(
    name: &str,
    span: Span,
    stack: &mut Vec<PolySlot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    builtin_overloads: &mut HashMap<Span, String>,
) -> Result<Option<Vec<PolySlot>>, String> {
    let mut split = stack.len();
    while split > 0 {
        if matches!(stack[split - 1].pt, PolyType::Concrete(_)) {
            split -= 1;
        } else {
            break;
        }
    }
    let mut cstack: Vec<Slot> = stack[split..]
        .iter()
        .map(|slot| match &slot.pt {
            PolyType::Concrete(t) => Slot::computed(*t),
            _ => unreachable!("suffix is all concrete by construction"),
        })
        .collect();
    // R12 (slice 8b, 8a): the poly operator path scopes candidates to the
    // calling module exactly like the concrete path; `None` (a standalone probe,
    // which runs no mangling pass) falls back to the flat `env.get(name)`.
    let scoped_ops = scoped_operator_overloads(ctx, env, name);
    let op_candidates = match &scoped_ops {
        Some(v) => Some(&v[..]),
        None => env.get(name).map(|v| &v[..]),
    };
    let handled = match check_operator(name, span, &mut cstack, ctx, op_candidates)? {
        OpDispatch::Builtin(s) => {
            cstack = s;
            true
        }
        // R6/R7: a builtin-row exact miss whose operands exactly match one of
        // this name's scoped candidates dispatches to that user word, same as
        // `check_term`'s call site: apply the chosen candidate's own outputs
        // (the resolver already confirmed its inputs equal the operands).
        OpDispatch::UserOverload(symbol) => {
            let sig = &op_candidates
                .and_then(|c| c.iter().find(|o| o.symbol == symbol))
                .expect("check_operator resolved this symbol from this scoped candidate set")
                .sig;
            cstack.truncate(cstack.len() - sig.inputs.len());
            cstack.extend(sig.outputs.iter().map(|ty| Slot::computed(*ty)));
            builtin_overloads.insert(span, symbol);
            true
        }
        OpDispatch::NotOperator => {
            if let Some(s) = check_str_word(name, span, &mut cstack, ctx)? {
                cstack = s;
                true
            } else {
                false
            }
        }
    };
    if !handled {
        return Ok(None);
    }
    stack.truncate(split);
    for slot in cstack {
        stack.push(PolySlot::new(PolyType::Concrete(slot.ty)));
    }
    Ok(Some(std::mem::take(stack)))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_poly_call(
    name: &str,
    span: Span,
    type_args: &[Type],
    len_args: &[Len],
    // P7b.S8b Phase 1 (R4): the impl-target equation for a nullary trait
    // member dispatched over a *generic* impl target, on its own channel.
    // A member word is polymorphic over both its impl target's variables and
    // its own locals, so this seed is partial by nature and cannot ride the
    // `type_args` channel, whose positional contract (P7.S3t, below) binds
    // variable `i` from argument `i`. `None` at every other caller, which
    // keeps that contract exactly as it was.
    impl_target_seed: Option<&Subst>,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    scope: &mut Scope,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    live: &Liveness,
    at: usize,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let candidates = poly.env.get(name).expect("caller checked membership");
    let sig = match candidates.as_slice() {
        [sig] => sig.clone(),
        _ => match resolve_poly_overload(candidates, stack, name, span, ctx, arrays, cells, refs) {
            Ok(chosen) => chosen,
            Err(PolyOverloadMiss::Quotation) => {
                return Err(reject_quotation_argument(ctx, span, name))
            }
            Err(PolyOverloadMiss::NoMatch) => {
                return Err(no_poly_overload_matches_error(ctx, span, name, candidates))
            }
        },
    };
    let n_in = sig.inputs.len();
    if stack.len() < n_in {
        return Err(underflow_error(ctx, span, name, n_in, stack.len()));
    }
    let base = stack.len() - n_in;
    let mut subst = Subst::default();
    // P7.S3t (R4/R5/R6): seed θ from the call site's explicit type-argument
    // list (`f[Point]`) *before* any operand is unified. Position `i` binds
    // the `i`-th declared type variable, whether or not an input grounds it:
    // a prefix rule would make position `i`'s meaning depend on which
    // variables the inputs happen to reach, so adding an input to the callee
    // would silently re-point every existing call site.
    //
    // Seeding first is what makes a disagreeing operand a diagnostic rather
    // than an overwrite: `unify_poly_input` finds the variable already bound
    // and takes its conflict branch, which `seeded` then redirects to the
    // message that names the instantiation.
    //
    // A variable's id *is* its index in `ty_var_names` (`intern_ty_var`
    // hands ids out as `ty_names.len()`), so position `i` binds variable `i`
    // and the seed pushes in ascending id. That alone does *not* make a
    // seeded θ render the same symbol as an inferred one -- see the sort
    // after pass 2.
    let mut seeded: Vec<u32> = Vec::new();
    if !type_args.is_empty() {
        if type_args.len() != sig.ty_var_names.len() {
            return Err(instantiation_arity_error(span, name, &sig, type_args.len()));
        }
        // P7b.S8b Phase 1 (R4): the arity gate above still reads the
        // explicit list -- a wrong-arity `empty[A B]` is the same located
        // error it was -- but an impl-target seed *replaces* the positional
        // binding, because on that path the written type argument is the
        // dispatch operand, not variable #0's value.
        //
        // P7b.S8b Phase 1 review (P2-5): a *concrete*-ctor target (`imp.target.is_concrete()`)
        // never reaches `check_poly_call` at all -- its member is handled entirely by the
        // mono branch above (`resolve_mono_member_call`), which does not consult this seed.
        // This fallback fires for a target whose *pattern* is fully applied to concrete
        // arguments (a lifted target, e.g. `impl: X for Range[i64]`) yet whose member still
        // carries its own free local (`bar['U]`), so the member word stays polymorphic and
        // does reach here (`lifted_mono` is false: the desugar only grounds a member with no
        // free locals of its own). `Range[i64]`'s pattern has no free variables to bind, so
        // `match_impl_target` hands back an *empty* subst -- `Some(empty)`, not `None`.
        // Replacing the positional binding with nothing would silently drop the caller's
        // explicit type argument (meant for `'U`, not for the target). Fall back to the
        // original positional contract whenever the seed carries no bindings; a genuinely
        // generic target's seed is non-empty by construction (it always grounds at least the
        // target's own variable), so that path is unaffected.
        if impl_target_seed
            .as_ref()
            .is_none_or(|seed| seed.ty.is_empty())
        {
            for (v, ty) in type_args.iter().enumerate() {
                subst.ty.push((v as u32, *ty));
                seeded.push(v as u32);
            }
        } else if let Some(seed) = &impl_target_seed {
            // `match_impl_target` keeps `Subst::ty` in variable-id order,
            // which is the ascending-id push order this block owes `Subst`.
            for (v, ty) in &seed.ty {
                subst.ty.push((*v, *ty));
                seeded.push(*v);
            }
            // P7b.S8b (round-1 review, P2): the seed replaces the positional
            // binding, so without this every written argument past the first
            // was dropped in silence. The arity gate above forces the caller
            // to write one argument per declared variable, and for a bare
            // multi-variable target (`impl: Monoid for Pair`, whose member
            // sig is padded to the target's variables by
            // `build_member_var_union`) the seed grounds *all* of them from
            // the dispatch type -- so `empty[Pair[i64 i64] str]` and
            // `empty[Pair[i64 i64] i64]` both minted
            // `..._t0_i64_t1_i64`, one monomorph from two spellings, the
            // second argument never read.
            //
            // Position 0 is exempt and cannot be checked here: on this path
            // it is the *dispatch type* (`type_args.first()` is what
            // `find_bound_impl` matched the target pattern against), not
            // variable #0's value -- that is the whole point of the seed
            // channel, and `empty[List[i64]]` binds `'T := i64` from an
            // argument that reads `List[i64]`. Every later position keeps
            // the ordinary positional contract (position `i` is variable
            // `i`), which is the only reading the arity gate leaves
            // available: a seed-bound variable must agree with what was
            // written, and a variable the seed does not reach (a member's
            // own local, appended after the target's) binds from it.
            for (pos, written) in type_args.iter().enumerate().skip(1) {
                let v = pos as u32;
                match seed.ty_of(v) {
                    Some(determined) if determined != *written => {
                        return Err(impl_target_seed_conflict_error(
                            ctx,
                            span,
                            name,
                            &sig.ty_var_names[pos],
                            determined,
                            *written,
                        ));
                    }
                    Some(_) => {}
                    None => {
                        subst.ty.push((v, *written));
                        seeded.push(v);
                    }
                }
            }
        }
    }
    // P7.S6b (R3): the length twin of the type-argument seeding above. R2b:
    // this slice's parser only ever mints `Len::Concrete` for an explicit
    // call-site argument (an integer token); a `Len::Var` reaching here would
    // mean a route this slice does not build exists, an internal-consistency
    // bug rather than a user-facing diagnostic.
    let mut seeded_len: Vec<u32> = Vec::new();
    if !len_args.is_empty() {
        if len_args.len() != sig.len_var_names.len() {
            return Err(length_instantiation_arity_error(
                span,
                name,
                &sig,
                len_args.len(),
            ));
        }
        for (v, ln) in len_args.iter().enumerate() {
            match ln {
                Len::Concrete(count) => {
                    subst.len.push((v as u32, *count));
                    seeded_len.push(v as u32);
                }
                Len::Var(_) => unreachable!(
                    "R2b: this slice's parser never produces Len::Var at a call-site instantiation"
                ),
            }
        }
    }
    // P7.S6b (R4): `seeded_len` threads into `unify_poly_input`'s `Len::Var`
    // conflict routing below, the length twin of `seeded` above.
    // P7.S3f (R2): the positions materialized against a ground declared
    // `Type::Quotation` input at this call site, threaded onto the recorded
    // `CallInst` so lowering can materialize the caller's phantom argument
    // into a real runtime aggregate before the call.
    let mut quot_inputs: Vec<(usize, &'static QuotEffect)> = Vec::new();
    // Pass 1: unify every non-quotation input first, so a variable a
    // declared quotation slot mentions is already bound by the time pass 2
    // grounds it -- mirroring `check_poly_combinator_args`'s two-pass split,
    // whatever the parameter order.
    //
    // A *fresh integer literal* filling a bare type variable is held back and
    // unified last, against whatever the variable resolved to -- D8's literal
    // coercion, ported from `check_poly_combinator_args` (10c) so a non-inline
    // call (R5's six comparisons, post-flip) keeps `5 3 >usize lt` working:
    // unifying the bare `5` first would pin `'T` to `i64` and read the `usize`
    // operand as a conflict.
    let mut deferred_literals: Vec<usize> = Vec::new();
    for i in 0..n_in {
        if poly_input_is_quotation(&sig.inputs[i]) {
            continue;
        }
        // R9p: `unify_poly_input` binds a `Var` to *any* concrete type, so a
        // quotation would silently bind `'T` to the placeholder and
        // monomorphize a call over a phantom. Reject before unification.
        if let Some(QuotRef::Known(_)) = stack[base + i].quot {
            return Err(reject_quotation_argument(ctx, span, name));
        }
        let slot = stack[base + i];
        if slot.literal && slot.ty == Type::I64 && matches!(sig.inputs[i], PolyType::Var(_)) {
            deferred_literals.push(i);
            continue;
        }
        unify_poly_input(
            &sig,
            &sig.inputs[i],
            slot.ty,
            name,
            span,
            ctx,
            arrays,
            cells,
            refs,
            &mut subst,
            &seeded,
            &seeded_len,
        )?;
    }
    for i in deferred_literals {
        let PolyType::Var(v) = &sig.inputs[i] else {
            unreachable!("only a `Var` parameter is deferred")
        };
        // Exactly D8's domain, no wider (10c): a fresh literal fills a
        // `usize`/`isize` position without an explicit conversion, and
        // nothing else.
        let ty = match subst.ty_of(*v) {
            Some(resolved @ (Type::Usize | Type::Isize)) => resolved,
            _ => stack[base + i].ty,
        };
        unify_poly_input(
            &sig,
            &sig.inputs[i],
            ty,
            name,
            span,
            ctx,
            arrays,
            cells,
            refs,
            &mut subst,
            &seeded,
            &seeded_len,
        )?;
    }
    // Pass 2: materialize each declared quotation input, now that `subst`
    // holds every binding pass 1 could produce.
    for i in 0..n_in {
        if !poly_input_is_quotation(&sig.inputs[i]) {
            continue;
        }
        if let Some(QuotRef::Known(id)) = stack[base + i].quot {
            match &sig.inputs[i] {
                PolyType::Concrete(Type::Quotation(eff)) => {
                    stack[base + i] = materialize_quotation_at_boundary(
                        id, eff, false, false, name, span, ctx, env, arrays, cells, refs, slices,
                        prov, scope, poly,
                    )?;
                    quot_inputs.push((i, eff));
                }
                // P7.S3l phase 2 (R9p closure): a declared quotation slot
                // still carrying a variable -- ground it through `subst`,
                // now complete from pass 1, then materialize exactly as the
                // ground arm does. Scope rules out an inline/row-carrying
                // slot ever reaching a non-combinator word's signature, so
                // `apply_subst`'s `Quotation` arm always grounds to
                // `Type::Quotation` here, never `Type::InlineQuotation`.
                pty @ PolyType::Quotation(..) => {
                    let grounded =
                        apply_subst(&sig, pty, &subst, name, span, ctx, arrays, cells, refs)?;
                    // A `~`/row-carrying slot grounds to `Type::InlineQuotation`
                    // instead, and `check_inline_quotation_requires_inline` keeps
                    // one off a non-combinator word's signature -- an invariant
                    // held two functions away, so this keeps R9p's rejection
                    // rather than asserting it.
                    let Type::Quotation(eff) = grounded else {
                        return Err(reject_quotation_argument(ctx, span, name));
                    };
                    stack[base + i] = materialize_quotation_at_boundary(
                        id, eff, false, false, name, span, ctx, env, arrays, cells, refs, slices,
                        prov, scope, poly,
                    )?;
                    quot_inputs.push((i, eff));
                }
                // `poly_input_is_quotation` also admits a concrete
                // `Type::InlineQuotation`, which this match does not handle;
                // the same non-local declaration guard is what keeps it out.
                _ => return Err(reject_quotation_argument(ctx, span, name)),
            }
        }
        let slot_ty = stack[base + i].ty;
        unify_poly_input(
            &sig,
            &sig.inputs[i],
            slot_ty,
            name,
            span,
            ctx,
            arrays,
            cells,
            refs,
            &mut subst,
            &seeded,
            &seeded_len,
        )?;
    }
    // P7.S3t (R5): restore `Subst`'s documented "kept sorted, the mangled
    // symbol depends on it" invariant, which the two-pass split above breaks:
    // pass 1 skips the quotation inputs, so for `( [ 'T -- ] 'U 'T -- 'U )`
    // inference binds `'U` (id 1) before `'T` (id 0) and pushes them in that
    // order. `instantiation_symbol` renders `subst.ty` in vector order, so
    // without this an inferred call and a `[...]`-seeded call at the *same* θ
    // mint two symbols and monomorphize the same specialization twice -- the
    // divergence R6 makes reachable by allowing a redundant instantiation.
    // `len` reorders the same way -- pass 1 skips the quotation input of
    // `( [ array['T 'N] -- ] array['U 'M] array['T 'N] -- )`, so `'M` (id 1) binds before
    // `'N` (id 0). P7.S6b gives it a seed path too (`len_args`/`seeded_len`
    // above), so divergence is now reachable in principle -- an inferred call
    // and an explicitly-instantiated call at the same theta could push `'N`
    // and `'M` in different orders. The sort below is what actually keeps
    // both paths agreeing, since `Subst`'s derived `Eq`, which
    // specializations dedup on, compares vectors positionally.
    subst.ty.sort_by_key(|(v, _)| *v);
    subst.len.sort_by_key(|(v, _)| *v);
    // P7.S3e (R8/R9): the trait-member calls in the callee's own body that
    // this instantiation's θ resolves, filled by the bound loop below and
    // recorded on the `CallInst` for lowering.
    let mut trait_calls: HashMap<Span, String> = HashMap::new();
    // R6: each declared bound must hold of the concrete type `θ` bound the
    // variable to.
    for (v, bound) in &sig.bounds {
        // An ungrounded variable (one no input mentions and no explicit
        // instantiation named) skips bound checking entirely, as it always
        // has. P7.S3t (R9): the old reason -- "no obligation can name a
        // variable the body could not have dispatched on" -- stops holding
        // once a nullary trait member exists, since such a body dispatches on
        // exactly such a variable. The `continue` is still right for a
        // narrower reason: a variable that reaches an *output* is caught by
        // `apply_subst` below, which reports it and names `f[SomeType]` as the
        // remedy, and a variable that reaches no output constrains nothing
        // this call site could observe.
        let Some(ty) = subst.ty_of(*v) else { continue };
        let var = &sig.ty_var_names[*v as usize];
        let unsatisfied = match bound {
            Bound::Copy => (!is_copy(ty, ctx.structs(), ctx.enums(), arrays))
                .then(|| poly_copy_bound_error(ctx, span, name, var, ty)),
            // P7.S3e (R8): satisfaction of a user trait is an `impl:` registry
            // lookup keyed by `(TraitId, θ(v))`, and each obligation the
            // callee's body recorded on this variable then resolves to a
            // concrete symbol against the same θ -- here, at check time, with
            // the `Subst` in hand, so lowering re-runs no resolution.
            Bound::User(trait_id) => {
                resolve_user_bound(
                    *trait_id,
                    *v,
                    ty,
                    &sig,
                    name,
                    span,
                    ctx,
                    &poly.trait_resolve,
                    arrays,
                    cells,
                    refs,
                    &mut trait_calls,
                    poly.impl_monos,
                    &subst,
                )?;
                None
            }
        };
        if let Some(err) = unsatisfied {
            return Err(err);
        }
    }
    let mut outputs: Vec<Type> = Vec::with_capacity(sig.outputs.len());
    for pty in &sig.outputs {
        outputs.push(apply_subst(
            &sig, pty, &subst, name, span, ctx, arrays, cells, refs,
        )?);
    }
    // P7.S12 (R1.2): the generated-enum-word call sites in the callee's own
    // body that this instantiation's θ resolves, filled the same way and at
    // the same point `trait_calls` is -- θ is complete, and the live
    // instantiator any of these sites needs (`apply_subst`'s `Generic` arm)
    // is exactly what `ctx.generics()` already carries here.
    let mut enum_words: HashMap<Span, EnumId> = HashMap::new();
    for (site_span, site_pty) in poly.trait_resolve.enum_sites_of(name, &sig) {
        let grounded = apply_subst(&sig, site_pty, &subst, name, span, ctx, arrays, cells, refs)?;
        if let Type::Enum(found, _) = grounded {
            enum_words.insert(*site_span, found);
        }
    }
    // P7b.S6 (review fix): ground and intern this callee's own body-internal
    // cell-construction sites (`^` on a value never reaching the declared
    // signature) against this call site's concrete θ -- the same grounding
    // `apply_subst`'s `OwnedCell` arm performs for a declared input/output,
    // run here for a site the signature walk above never reaches. The
    // grounded `Type` is discarded; only the interning side effect matters.
    for (_, site_pty) in poly.trait_resolve.cell_sites_of(name, &sig) {
        apply_subst(&sig, site_pty, &subst, name, span, ctx, arrays, cells, refs)?;
    }
    // Review fix (P7 slice 1): a polymorphic word consumes its operands
    // exactly as a concrete one does, so it needs the same guard against
    // moving a place a live projection still reaches -- `'T` binds to the
    // receiver's struct type as readily as a declared `Point` does.
    for i in base..stack.len() {
        let origin =
            consumed_place_conflict(stack[i], &stack[..i], ctx, arrays, scope, prov, live, at)
                .or_else(|| {
                    consumed_place_conflict(
                        stack[i],
                        &stack[i + 1..],
                        ctx,
                        arrays,
                        scope,
                        prov,
                        live,
                        at,
                    )
                });
        if let Some(origin) = origin {
            return Err(consuming_borrowed_value_error(ctx, span, name, origin));
        }
    }
    // R14: record the instantiation for lowering, keyed by the call-site span.
    // The bundle is filled later (a resolved output count >= 2 interns one).
    let symbol = instantiation_symbol(name, &subst);
    // P7.S3o (R1/R2): when inside a combinator splice, redirect the
    // already-minted CallInst to `splice_records` keyed by `(uid, span)`
    // instead of the span-keyed `insts`. A poly combinator's body terms
    // keep their original spans across every splice (alpha_rename_locals
    // renames locals only), so two splices at two different concrete types
    // would collide on the same span key — last write wins, and lowering
    // reads only the surviving entry. The per-splice key `(uid, span)` is
    // unique because each splice mints a fresh `inline_uid`, so both
    // splices' inner-call instantiations survive independently. The 1a
    // collision guard below is skipped for these calls; it stays as a
    // safety net for any other path that might re-introduce the collision.
    if let Some(uid) = prov.splice_uid {
        // P7.S3o Phase 4 (R5): a poly call inside a materialized quotation
        // within a splice must NOT redirect to `splice_records`. The
        // materialized quotation lowers to its own `IrFunc` with an empty
        // `splice_uid_stack`, so it cannot resolve a `(uid, span)` key and
        // would fall through to `env.get(name).expect(...)` and panic. Let it
        // fall through to the span-keyed `insts` table below instead, which
        // the materialized quotation's `FuncBuilder` reads via
        // `self.instantiations`. The collision guard below still catches two
        // splices at different types producing different symbols for the same
        // span, so there is no silent miscompile.
        if !prov.in_materialized_quot {
            poly.splice_records.insert(
                (uid, span),
                CallInst {
                    callee: name.to_string(),
                    subst,
                    symbol,
                    out_arity: outputs.len(),
                    output_types: outputs.clone(),
                    bundle: None,
                    quot_inputs,
                    trait_calls,
                    poly_calls: HashMap::new(),
                    enum_words,
                },
            );
            push_dispatch_outputs(stack, base, &outputs, name, span, ctx, arrays, prov)?;
            return Ok(std::mem::take(stack));
        }
    }
    // P7.S3o recon: a poly combinator's body terms keep their original spans
    // across every splice (alpha_rename_locals renames locals only), so two
    // splices at two different concrete types insert CallInsts at the *same*
    // span — last write wins, and lowering reads only the surviving entry.
    // The losing splice dispatches to the wrong monomorph and the other type's
    // is never emitted: a silent miscompile. This guard turns that collision
    // into a located error. Two splices at the *same* type produce the same
    // symbol and dedup harmlessly; ordinary (non-combinator) call sites have
    // unique spans and never collide.
    if let Some(existing) = poly.insts.get(&span) {
        if existing.symbol != symbol {
            return Err(splice_collision_error(ctx, span, name));
        }
    }
    poly.insts.insert(
        span,
        CallInst {
            callee: name.to_string(),
            subst,
            symbol,
            out_arity: outputs.len(),
            output_types: outputs.clone(),
            bundle: None,
            quot_inputs,
            trait_calls,
            enum_words,
            // P7.S3k (R4): the concrete path records none. A cross-call is
            // discovered by the transitive fixpoint, which fills this in
            // afterwards for the instantiations whose body has one.
            poly_calls: HashMap::new(),
        },
    );
    push_dispatch_outputs(stack, base, &outputs, name, span, ctx, arrays, prov)?;
    Ok(std::mem::take(stack))
}

/// P7b.S6d-PREREQ (REQ-4d site 4): push a dispatch call's outputs, carrying
/// the operands' borrow provenance onto the reference-bearing ones.
///
/// Every dispatch path truncates the operands and re-pushes outputs, and a
/// bare `Slot::computed` there launders the borrow. This is not a future
/// hazard: a generic `( 'T -- 'T )` pass-through over a bare slice was enough
/// to defeat exclusivity outright before this forward existed (a write through
/// `&!a` was observable through a shared view of `a` that had been handed
/// through the pass-through). All four dispatch pushes go through here so the
/// hole cannot survive on one path.
#[allow(clippy::too_many_arguments)]
fn push_dispatch_outputs(
    stack: &mut Vec<Slot>,
    base: usize,
    outputs: &[Type],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
) -> Result<(), String> {
    let (deriv, alias) = carried_borrow(ctx, span, name, &stack[base..], outputs, arrays, prov)?;
    stack.truncate(base);
    for ty in outputs {
        let bearing = ctx.with_extended_type_slices(|structs, enums| {
            contains_reference(*ty, structs, enums, arrays)
        });
        stack.push(Slot {
            alias: bearing.then_some(alias).flatten(),
            ..Slot::derived(*ty, bearing.then_some(deriv).flatten())
        });
    }
    Ok(())
}

/// Push each not-yet-seen composed callee onto the worklist. Deduping by
/// symbol *before* recursing is what makes a mutual `g <-> h` cycle stop: the
/// second visit to `(g, θ)` mints the same symbol as the first.
fn enqueue_new(
    routed: &HashMap<Span, CallInst>,
    seen: &mut HashSet<String>,
    frontier: &mut Vec<CallInst>,
) {
    for callee in routed.values() {
        if seen.insert(callee.symbol.clone()) {
            frontier.push(callee.clone());
        }
    }
}

/// P7.S3k (R4, review finding 1): whether an `inline` callee's own body -- the
/// terms lowering will splice in place of a call to it -- names any
/// polymorphic word at all. A one-level name match against every polymorphic
/// `WordDef` is enough: if the named callee is itself `inline`, a call to *it*
/// already trips this same check the next time this callee is reached from a
/// caller, so nothing needs to recurse into a nested combinator's body here.
/// Only `Quotation` nests terms; every other `TermKind` is a leaf.
fn body_calls_a_poly_word(body: &[Term], words: &[WordDef]) -> bool {
    body.iter().any(|term| match &term.kind {
        TermKind::Call(name, _, _) => words.iter().any(|w| w.poly.is_some() && &w.name == name),
        TermKind::Quotation(inner, _, _) => body_calls_a_poly_word(inner, words),
        _ => false,
    })
}

/// P7.S3k (R4/N1): a cross-call whose caller or callee name is a polymorphic
/// overload set. Located rather than mis-composed: the two candidates' records
/// merge under one name and each indexes its own signature's variables, so
/// picking either would ground the wrong monomorph without saying so.
fn overloaded_cross_call_error(caller: &str, callee: &str, overloaded: &str, span: Span) -> String {
    format!(
        "error: `{}` cannot call the polymorphic word `{}` (line {}, col {})\n  `{}` names more than one polymorphic word, and a call across two generic words carries no record of which candidate it resolved to\n  give the overloaded word distinct names",
        crate::resolve::demangle_word(caller),
        crate::resolve::demangle_call(callee),
        span.line,
        span.col,
        crate::resolve::demangle_call(overloaded),
    )
}

/// P7.S3k (R4, review finding 1): a cross-call to an `inline` callee whose own
/// body calls another polymorphic word. Lowering splices `h`'s body at this
/// call site, but the checker recorded that inner call as an ordinary
/// cross-call keyed on `h`, not walked in place here -- so nothing composes
/// a θ for it, and routing it anyway would either mint against a symbol that
/// never exists or reuse whatever a different caller of `h` last routed at
/// the same span. Located at the outer call site, since that is the only
/// place a fix (a non-generic `h`, or calling `h` from a monomorphic word)
/// can land.
fn inline_callee_cross_call_error(caller: &str, callee: &str, span: Span) -> String {
    format!(
        "error: `{}` cannot call the polymorphic word `{}` (line {}, col {})\n  `{}` is `inline` and its own body calls another polymorphic word, which a call across two generic words cannot yet route\n  give `{}` a non-generic body, or call it only from a monomorphic word",
        crate::resolve::demangle_word(caller),
        crate::resolve::demangle_call(callee),
        span.line,
        span.col,
        crate::resolve::demangle_call(callee),
        crate::resolve::demangle_call(callee),
    )
}

/// P7b.S8c (REQ-1/REQ-2): the fence both re-grounding arms of
/// `resolve_user_bound` run *before* touching a slot. A written quotation
/// literal can reach a plain member slot (`unify_member_operand`'s
/// `(Var(v), found)` bind arm accepts it, unlike a declared `Quotation`
/// slot), and `apply_subst`'s `QuotLit` arm is `unreachable!` -- so the
/// marker has to be rejected located at the dispatch site instead.
fn fence_quotation_literal_slot(
    ctx: &Ctx,
    ob: &TraitObligation,
    trait_name: &str,
) -> Result<(), String> {
    match ob.slots.iter().position(|s| *s == PolyType::QuotLit) {
        Some(slot) => Err(member_slot_quotation_literal_error(
            ctx, ob.span, &ob.member, trait_name, slot,
        )),
        None => Ok(()),
    }
}

/// P7b.S8c (REQ-1): a written quotation literal occupies a member operand
/// slot that the member declared as an ordinary (non-quotation) input.
/// `unify_member_operand`'s bind arm admits it, so the obligation carries a
/// `PolyType::QuotLit` slot -- which per-site re-grounding would drive into
/// `apply_subst`'s `unreachable!`. Located at the dispatch site, naming the
/// member and the slot, in `trait_member_operand_error`'s wording family.
fn member_slot_quotation_literal_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    slot: usize,
) -> String {
    format!(
        "error: `{member}` of `{trait_name}` in {name} (line {}, col {}) found a quotation literal in operand slot {slot}\n  a bound-dispatched member instantiates its signature at this site's operand types, and a written quotation literal has no type to instantiate at -- declare the slot as a quotation parameter, or pass the literal through one",
        span.line,
        span.col,
        name = ctx.rendered_word(),
    )
}

/// P7b.S8c (REQ-1), the fail-closed tail: neither the impl-target match nor
/// this site's operands determined some variable of the member word's
/// signature, so no instantiation of it exists to dispatch to. Located here
/// rather than left to lowering, where the same shape is
/// `subst_polytype`'s `expect` (`src/ir/driver.rs:579`) -- a raw panic.
fn member_unbound_variable_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    var: &str,
) -> String {
    format!(
        "error: `{member}` of `{trait_name}` in {name} (line {}, col {}) leaves type variable `{var}` unbound\n  the impl target's match and this site's operands together determine no type for `{var}`, so the member has no instantiation here -- give it an operand position that fixes `{var}`",
        span.line,
        span.col,
        name = ctx.rendered_word(),
    )
}

// P7.S4 Phase 2 (R3): the specificity partial order — equivalence-class
// refinement over shared variables.

/// S1-15.g: a bare `Type::CtorImage` reached a value-type position outside
/// `App`-head resolution -- `var` is used as a plain type, but the
/// application walk (`unify_poly_input`, S1-10) bound it to the constructor
/// `ctor_name` instead. Both spans travel: `binding_span` (the variable's
/// first mention, `PolySig::ty_var_spans`) and `span` (this misuse site).
pub(super) fn poly_ctor_image_as_type_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    binding_span: Span,
    ctor_name: &str,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    STAND_IN_GROUNDING_TAG.to_string()
        + &format!(
        "error: `{callee}` in {name} (line {line}) uses type variable `{var}` at line {line}, col {col} as a plain type, but it is bound to the constructor `{ctor_name}` at line {bline}, col {bcol}\n  a constructor is not a type until applied to arguments: `{var}[...]`",
        name = ctx.rendered_word(),
        line = span.line,
        col = span.col,
        bline = binding_span.line,
        bcol = binding_span.col,
    )
}

/// P7b.S3 (S3-6): the marker a raise site stamps on a diagnostic whose cause
/// *can* be the standalone check's arbitrary choice of stand-in constructor
/// -- a grounding failure against that constructor. The standalone check
/// rescues a tagged failure (the word is re-checked at every splice site,
/// where θ is real) and lets every other failure class through as a hard
/// error: arity/underflow, linearity, an unknown word, a borrow or move
/// violation, an undischarged bound are all impl-independent.
///
/// The discriminator is a token the **raise site** owns, not the diagnostic's
/// prose: matching on wording would make every message text load-bearing and
/// would silently widen as messages are edited. The tag is stripped by
/// `strip_stand_in_tag` before a diagnostic reaches a user, so it is never
/// part of a golden's asserted text.
///
/// The tagged set is deliberately the smallest one that covers the stand-in's
/// own arbitrariness. Widening it is a measurement, not a default.
pub(super) const STAND_IN_GROUNDING_TAG: &str = "\u{1}p7bs3-stand-in\u{1}";

/// Whether `err` carries [`STAND_IN_GROUNDING_TAG`], and the message without
/// it. Every escape route for a diagnostic runs through this, so a tag can
/// never leak into rendered output.
pub(super) fn strip_stand_in_tag(err: String) -> (bool, String) {
    match err.strip_prefix(STAND_IN_GROUNDING_TAG) {
        Some(rest) => (true, rest.to_string()),
        None => (false, err),
    }
}

/// R7 twin of `linear_local_unconsumed_error` for the polymorphic body
/// checker: a local bound to a non-`Copy` slot still holds its value at the
/// word's end. Names the local and its slot so the diagnostic matches the one
/// a concrete instantiation would already get from the monomorphic checker.
pub(super) fn poly_local_unconsumed_error(
    word: &WordDef,
    sig: &PolySig,
    local: &str,
    pt: &PolyType,
) -> String {
    format!(
        "error: linear value `{}` is never consumed in `{}`\n  `{}` has type `{}`, which is linear: drop it or return it (nothing is dropped for you)",
        local,
        crate::resolve::demangle_word(&word.name),
        local,
        poly_type_str(pt, sig),
    )
}

/// R7 twin of `use_after_move_error` for the polymorphic body checker: a
/// non-`Copy` local read again after its first read (which consumed it),
/// citing the earlier read site.
pub(super) fn poly_use_after_move_error(ctx: &Ctx, span: Span, local: &str, site: Span) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: use after move in {where_} (line {})\n  local `{local}` is linear and was moved at line {}, col {}, so it is used exactly once",
        span.line, site.line, site.col,
    )
}

pub(super) fn poly_copy_body_error(ctx: &Ctx, span: Span, op: &str, var: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot `{op}` the type variable `{var}` in {where_} (line {})\n  `{var}` has no `Copy` bound, and a linear value cannot be duplicated; declare `{var}: Copy` if every instantiation is `Copy`",
        span.line
    )
}

/// Slice 13 (E1): `dup`/`over` on a mutable reference in a generic body. The
/// same class of fact as `poly_copy_body_error`'s missing `Copy` bound, but
/// the reason is exclusivity rather than an absent bound, so the note names
/// that instead.
/// P7.S3n (R3): `dup`/`over` on an owning cell whose payload is still
/// polymorphic. The monomorphic twin is `cannot_copy_error` on a concrete
/// `^T`; a `^'T` has no concrete `Type` to hand that, so the rendering goes
/// through `poly_type_str` instead.
pub(super) fn poly_copy_owned_cell_error(ctx: &Ctx, span: Span, op: &str, ty: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot `{op}` an owning cell in {where_} (line {})\n  `{ty}` is linear: it owns its payload, so duplicating it would free the same allocation twice",
        span.line
    )
}

pub(super) fn poly_copy_mutable_ref_error(ctx: &Ctx, span: Span, op: &str, ty: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot `{op}` a mutable reference in {where_} (line {})\n  `{ty}` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow",
        span.line
    )
}

/// P7 slice 3a (R5.4): `dup`/`over` on a generic type applied to a variable
/// (D5's conservative linearity). The same class of fact as
/// `poly_copy_mutable_ref_error`, naming the type rather than a variable
/// name since a generic application has no single bound to point at.
pub(super) fn poly_copy_generic_error(ctx: &Ctx, span: Span, op: &str, ty: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot `{op}` a generic type applied to a variable in {where_} (line {})\n  `{ty}` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated",
        span.line
    )
}

/// P7.S12 (R3.1/R5.5): `dup`/`over` on a narrowed generic variant. The same
/// class of fact as `poly_copy_generic_error`, naming the variant rather than
/// the header since a variant's own field types are what may be linear.
pub(super) fn poly_copy_generic_variant_error(ctx: &Ctx, span: Span, op: &str, ty: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot `{op}` a generic variant in {where_} (line {})\n  `{ty}` may carry a linear field at some instantiation, so it cannot be duplicated",
        span.line
    )
}

/// P7 slice 3c (R1.2, phase 4): `slice` over a buffer whose *element* is still
/// generic. The view's length may be a variable -- that is what a view erases
/// -- but its element may not: a generic element is a locked non-goal, so the
/// message names the rule rather than reporting a shape mismatch.
fn poly_slice_generic_element_error(
    ctx: &Ctx,
    span: Span,
    elem: &PolyType,
    sig: &PolySig,
) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: `slice` over an array of `{}` in {where_} (line {}) is not supported\n  a view's element type must be concrete; only its length may be generic",
        poly_type_str(elem, sig),
        span.line
    )
}

pub(super) fn poly_op_on_variable_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    pt: &PolyType,
    sig: &PolySig,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    let what = match pt {
        PolyType::Var(v) => format!("the type variable `{}`", sig.ty_var_names[*v as usize]),
        PolyType::Array(..) => "an array with a variable".to_string(),
        PolyType::Concrete(t) => format!("`{t}`"),
        PolyType::Quotation(..) => "a quotation".to_string(),
        PolyType::QuotLit => "a quotation literal".to_string(),
        PolyType::Ref(..) => "a reference".to_string(),
        PolyType::OwnedCell(_) => "an owning cell".to_string(),
        // P7 slice 3a: rendered with the application, so the diagnostic
        // names which generic header and which arguments, not just "a
        // generic type".
        PolyType::Generic { .. } => format!("a generic type `{}`", poly_type_str(pt, sig)),
        // P7.S12 (R3.1): rendered with the variant, mirroring `Generic`.
        PolyType::GenericVariant { .. } => {
            format!("a generic variant `{}`", poly_type_str(pt, sig))
        }
        // P7b.S1 (S1-16): rendered with the application, mirroring
        // `Generic`.
        PolyType::App { .. } => format!("a higher-kinded application `{}`", poly_type_str(pt, sig)),
    };
    format!(
        "error: `{op}` is not permitted on {what} in {where_} (line {})",
        span.line
    )
}

/// P7.S3e (R12, decision 5): a member required by two of a body's bounds,
/// called unqualified. Composing the traits stays legal; only the call is
/// ambiguous, so the diagnostic sits here and not at the declaration.
///
/// P7.S3p (ruling 4): candidates now span every bounded variable, so each is a
/// `(trait, type-variable)` pair. When they all share one variable it renders
/// as it always did ("required by both `A` and `B` on 'T"); when they differ,
/// each trait is named with its own variable.
fn ambiguous_trait_member_error(span: Span, member: &str, candidates: &[(&str, &str)]) -> String {
    let listed = if candidates.windows(2).all(|w| w[0].1 == w[1].1) {
        let quoted: Vec<String> = candidates.iter().map(|(t, _)| format!("`{t}`")).collect();
        format!(
            "both {} on {}",
            joined_with_and(&quoted),
            candidates.first().map_or("", |(_, v)| v)
        )
    } else {
        let each: Vec<String> = candidates
            .iter()
            .map(|(t, v)| format!("`{t}` on {v}"))
            .collect();
        joined_with_and(&each)
    };
    format!(
        "error: `{member}` is required by {listed} (line {}, col {})\n  note: a member required by two of a variable's bounds cannot be called unqualified",
        span.line, span.col
    )
}

/// P7.S3p (ruling 4, amended): the operands at the call fit none of the
/// candidates spanning more than one variable. This is not the same problem
/// as `ambiguous_trait_member_error`'s: there, a module qualifier resolves
/// the call by naming which trait is meant; here every candidate's declared
/// operands already disagree with the stack, so no qualifier changes that --
/// the call is a plain operand-shape mismatch, just one with more than one
/// declared shape to be wrong against.
///
/// Each candidate's *substituted* input list is rendered, so the note that
/// says the operands match no declared shape also shows the shapes it means
/// (`trait_member_operand_error` names the one shape it checked against; this
/// path checked several and must name them all to be equally actionable).
fn no_candidate_fits_operands_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    candidates: &[(&str, &str, String)],
    ungroundable: &[(&str, String)],
) -> String {
    let where_ = ctx.rendered_word();
    let each: Vec<String> = candidates
        .iter()
        .map(|(t, v, _)| format!("`{t}` on {v}"))
        .collect();
    let shapes: Vec<String> = candidates
        .iter()
        .map(|(t, v, inputs)| format!("`{t}` on {v} expects `{inputs}`"))
        .collect();
    let mut msg = format!(
        "error: `{member}` is required by {} in {where_} (line {}, col {})\n  note: the operands at this call match none of their declared shapes: {}",
        joined_with_and(&each),
        span.line,
        span.col,
        joined_with_and(&shapes)
    );
    // P7b.S3 (S3-7): a candidate θ could not ground was excluded before the
    // operand match ran at all, so "the operands match none of their shapes"
    // is not the whole story for it. Naming the reason is the difference
    // between this and the `continue` it replaced.
    if !ungroundable.is_empty() {
        let reasons: Vec<String> = ungroundable
            .iter()
            .map(|(t, why)| format!("`{t}` could not be grounded here: {why}"))
            .collect();
        msg.push_str(&format!("\n  note: {}", joined_with_and(&reasons)));
    }
    msg
}

fn joined_with_and(items: &[String]) -> String {
    match items.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// P7.S3e (R7): a bound-directed member call whose operands do not match the
/// trait's declared member signature, with the trait's own type variable
/// rewritten to the bounded variable being dispatched on.
#[allow(clippy::too_many_arguments)]
fn trait_member_operand_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    expected: &PolyType,
    expected_sig: &PolySig,
    found: &PolyType,
    found_sig: &PolySig,
    slot: usize,
) -> String {
    let where_ = ctx.rendered_word();
    // P7b.S2 (S2-16): the error names the slot position -- the declared sig
    // is unified slot by slot, so the offending position is knowable and the
    // old position-free wording would leave a multi-input member ambiguous.
    //
    // P7b.S6 Phase 1 (R2.a): `expected` and `found` are rendered against
    // separate sigs by provenance -- `expected` may be pure member-space
    // (the raw declared input, never rewritten into the caller's variable
    // space) while `found` is pure caller-space, so a single shared `sig`
    // can index off the end of whichever table doesn't own the term.
    format!(
        "error: `{member}` of `{trait_name}` in {where_} (line {}, col {}) expects `{}`, found `{}` in operand slot {}",
        span.line,
        span.col,
        poly_type_str(expected, expected_sig),
        poly_type_str(found, found_sig),
        slot,
    )
}

pub(super) fn poly_var_to_concrete_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    expected: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{callee}` in {where_} (line {}) expects `{expected}`, but the type variable `{var}` is not a concrete type",
        span.line
    )
}

/// Slice 13 (E4/R-B6): an accessor with no poly-body support -- ever
/// (`&^`, a retired fused-accessor spelling), or not yet (e.g. a fully
/// concrete `&!array[T N]`
/// parameter's accessors, folded to `PolyType::Concrete` and unmatched by
/// any `PolyType::Ref` arm) -- located, never a silent fallthrough to an
/// unknown-word error.
pub(super) fn poly_unsupported_accessor_error(ctx: &Ctx, span: Span, op: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{op}` is not yet supported in a generic body, in {where_} (line {})\n  monomorphize this word (or write a concrete wrapper) to use `{op}` today",
        span.line
    )
}

/// Whether the receiver a `&f` would project out of is a struct or a variant,
/// rather than a bare type parameter or a scalar. Reference or owned alike:
/// both are receivers of a projection under P7 slice 1's D2.
fn receiver_is_aggregate_projection(stack: &[PolySlot]) -> bool {
    let Some(top) = stack.last().map(|slot| &slot.pt) else {
        return false;
    };
    let referent = match top {
        PolyType::Ref(inner, _) => inner.as_ref(),
        other => other,
    };
    matches!(
        referent,
        PolyType::Concrete(Type::Struct(..) | Type::Enum(..) | Type::Variant(..))
            // P7 slice 3a: an ungrounded generic application is a struct or
            // enum header (never yet a `Type::Struct`/`Type::Enum` to match
            // above), so it is a projection receiver exactly the same way.
            | PolyType::Generic { .. }
            // P7.S12 (R3.4/R6.4): a narrowed generic variant is a projection
            // receiver too -- field projection into one is a standing,
            // separately-tracked gap, but the `&f` guard must say so rather
            // than claim `f` is not a local.
            | PolyType::GenericVariant { .. }
    )
}

/// A bare `&`/`&!` sigil with no referent: names nothing, so there is no
/// place to borrow. Mirrors the monomorphic `borrow_of_non_place_error`'s
/// "a bare sigil cannot borrow whatever happens to be on the stack" case.
fn poly_borrow_of_non_place_error(ctx: &Ctx, span: Span, spelled: &str) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: `{spelled}` does not borrow a place in {where_} (line {})\n  it names nothing (a bare sigil cannot borrow whatever happens to be on the stack)",
        span.line
    )
}

/// `&x`/`&!x` where `x` is not a local currently in scope.
fn poly_borrow_of_non_local_error(ctx: &Ctx, span: Span, spelled: &str, local: &str) -> String {
    let spelled = crate::resolve::demangle_word(spelled);
    let local = crate::resolve::demangle_word(local);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{spelled}` does not borrow a place in {where_} (line {})\n  `{local}` is not a local in scope",
        span.line
    )
}

/// Slice 13 (E2/D5): borrowing a local whose declared type is a bare type
/// variable -- it might instantiate to a scalar, which has no address, so
/// the conservative rule refuses every bare-variable local uniformly rather
/// than deferring "is it an aggregate?" to instantiation. Mirrors the
/// monomorphic `borrow_of_scalar_local_error`'s shape.
pub(super) fn poly_borrow_of_variable_local_error(
    ctx: &Ctx,
    span: Span,
    local: &str,
    var: &str,
) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot borrow the local `{local}` of type `{var}` in {where_} (line {}, col {})\n  `{var}` might instantiate to a scalar, which has no address; borrow an aggregate (a struct, enum, array, or owning cell) instead",
        span.line, span.col
    )
}

/// D5's aggregate gate, the non-variable arm: a concrete scalar, or a local
/// already itself a reference, is not an aggregate either. Not spec-pinned
/// (no required golden exercises it), so the wording is free; it still
/// names the local and its type rather than falling through to a generic
/// diagnostic.
fn poly_borrow_of_non_aggregate_local_error(
    ctx: &Ctx,
    span: Span,
    local: &str,
    ty: &str,
) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot borrow the local `{local}` of type `{ty}` in {where_} (line {}, col {})\n  only an aggregate (a struct, enum, array, or owning cell) is borrowable; `{ty}` is not",
        span.line, span.col
    )
}

/// A quotation local, split out from `poly_borrow_of_non_aggregate_local_error`
/// (review, post-slice-12 rebase): a non-`inline` word's ordinary `[ ... ]`
/// parameter lowers to a real three-word `(code, env, disposer)` aggregate at
/// the ABI level, so "is not an aggregate" is false at the representation the
/// backend actually emits, even though it is true at the type-system level
/// this slice reasons over. Naming the actual reason -- unsupported, not
/// shapeless -- avoids a claim the ABI contradicts; borrowing a quotation is
/// 7b territory (a first-class capturing closure), not this slice's.
fn poly_borrow_of_quotation_local_error(ctx: &Ctx, span: Span, local: &str, ty: &str) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot borrow the local `{local}` of type `{ty}` in {where_} (line {}, col {})\n  a quotation is not borrowable in a generic body",
        span.line, span.col
    )
}

/// Slice 13 (R-B5): every borrow-liveness diagnostic below carries this,
/// because none of them are answered by the monomorphic `Provenance`/
/// `Liveness` pair: `PolyScope` approximates a borrow's lifetime instead
/// (`prune_dead_borrows`), so a rejection here can be conservative. Saying so
/// keeps a false positive legible as a deliberate bound rather than a checker
/// bug.
const POLY_BORROW_LIVENESS_NOTE: &str = "\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local";

/// P7.S3g-follow (1c): a reference derived from a local of this frame handed to
/// the self-tail call. Wording tracks the monomorphic
/// `reference_across_back_edge_error`, minus its `note: declared` line (a
/// generic word's `Ctx::effect` is a placeholder, not its signature) and
/// plus the conservative-liveness note every poly borrow rejection carries.
fn poly_reference_across_back_edge_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    place: &str,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let place = crate::resolve::demangle_word(place);
    let where_ = ctx.rendered_word();
    format!(
        "error: a reference to a local cannot cross a loop in {where_} (line {})\n  a reference derived from `{place}`, a local of this frame, crosses the self-tail-call back-edge to `{callee}`: that local's storage does not survive to the next iteration{POLY_BORROW_LIVENESS_NOTE}",
        span.line,
    )
}

/// Slice 13 (E6/R-B5): exclusivity in a generic body, in whichever of its two
/// symmetric directions was violated -- a new mutable borrow against any live
/// borrow of the place, a new shared one against a live mutable borrow. Same
/// shape as the monomorphic `conflicting_borrow_error`, plus the conservative
/// note.
fn poly_conflicting_borrow_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    new_mutable: bool,
    live: &PolyBorrow,
) -> String {
    let place = crate::resolve::demangle_word(place);
    let where_ = ctx.rendered_word();
    let sigil = if new_mutable { "&!" } else { "&" };
    let held = if live.mutable { "mutable" } else { "shared" };
    format!(
        "error: `{sigil}{place}` conflicts with a live borrow of `{place}` in {where_} (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first{POLY_BORROW_LIVENESS_NOTE}",
        span.line, span.col, live.span.line, live.span.col,
    )
}

/// Slice 13 (R-B5): consuming a local -- reading a linear one moves it out --
/// while a reference derived from it is still live would leave that reference
/// aimed at storage its owner has given away. The monomorphic
/// `consume_of_borrowed_place_error`'s twin.
fn poly_consume_of_borrowed_place_error(
    ctx: &Ctx,
    span: Span,
    place: &str,
    ty: &str,
    live: &PolyBorrow,
) -> String {
    let where_ = ctx.rendered_word();
    let held = if live.mutable { "mutable" } else { "shared" };
    format!(
        "error: cannot consume the borrowed local `{place}` of type `{ty}` in {where_} (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  a place stays borrowed until every reference derived from it is consumed{POLY_BORROW_LIVENESS_NOTE}",
        span.line, span.col, live.span.line, live.span.col,
    )
}

/// Slice 13 (R-B5): the other naming direction -- reading a `Copy` aggregate
/// local while a mutable borrow of it is live. The read does not consume it,
/// but it is a second name for storage that borrow mutates. The monomorphic
/// `naming_aliases_borrowed_place_error`'s twin.
fn poly_naming_aliases_borrowed_place_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    live: &PolyBorrow,
) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot name `{name}` in {where_} (line {}, col {}): a mutable borrow of it is still live (line {}, col {})\n  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates\n  finish with the borrow first, or `dup` for an independent copy{POLY_BORROW_LIVENESS_NOTE}",
        span.line, span.col, live.span.line, live.span.col,
    )
}

/// P7 slice 3b (R4/L2): a quotation literal that is still on the stack where
/// it would have to *exist* as a value -- at the polymorphic word's exit, or
/// leaving an eliminator arm. Splice-consumed literals only: a quotation in a
/// generic body has no runtime representation to return, store, or capture,
/// and the one thing that consumes one here is an eliminator in the same body.
pub(super) fn poly_quotation_not_consumed_error(ctx: &Ctx, span: Span) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: a quotation in the polymorphic body of {} (line {}) is not consumed there\n  only an eliminator call in the same body consumes a quotation in a generic word: it cannot be returned, stored, or captured",
        where_,
        span.line
    )
}

/// P7 slice 3b (R4/OQ6): a quotation consumer in a generic body that this
/// slice's dispatch does not cover -- the compiler-known `call`/`branch`/`tag`
/// primitives, which are not combinator-env words and so carry no declared
/// `PolySig` to drive a dispatch off, and a combinator declaring an output row
/// no arm of it produces. Located rather than left to fall through to
/// `unknown word`.
///
/// `call` on a literal is P7.S3d's own exit criterion. `branch` is the same
/// shape one level down and `tag` is a scalar-primitive port with no arm walk
/// (it consumes no quotation at all -- an all-unit enum to `u32` -- and
/// reaches this only because the guard naming it is name-based), but S3d's own
/// spec excludes both ("stays rejected, unchanged") while pointing them back
/// at this slice, so neither names a slice yet.
pub(super) fn poly_quotation_combinator_unsupported_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
) -> String {
    let demangled = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{demangled}` on a quotation in the polymorphic body of {} (line {}) is not yet supported\n  a generic body consumes a quotation through an enum eliminator, through an always-spliced combinator that declares it as a `~[ ]` parameter, or through `call` on a literal (P7.S3d); the `branch`/`tag` primitives declare nothing to ground and name no follow-up slice yet",
        where_,
        span.line
    )
}

/// P7 slice 3b-follow (R3/L1): a row-typed combinator whose declaration this
/// dispatch cannot ground. Two causes, one message, because the answer is the
/// same: a type or length variable of the callee's own (`each`'s `&['T N]`)
/// would have to be *solved for*, the mid-body unification L1 forbids; and a
/// declared output row no parameter produces leaves the combinator's promised
/// exit and what its arms actually leave unrelated. (P7.S3j closed a third:
/// a slot declared above a row a quotation parameter produces is now
/// stripped back off an arm's exit rather than rejected here.)
fn poly_combinator_abstract_signature_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    declared: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{word}` declares `{declared}`, which a call in the polymorphic body of {} (line {}) cannot ground\n  a generic body consumes a row-typed combinator whose own types are concrete, and whose declared output row one of them produces",
        where_,
        span.line
    )
}

/// P7 slice 3b-follow (R3): a combinator arm whose body does not leave what
/// its declared *non-shape-changing* effect requires -- the grounded row it
/// entered with, then the declared fixed outputs. The poly twin of
/// `literal_effect_mismatch_error` under `LiteralBoundary { shape_changing:
/// false }`, rendered because the actual row holds `PolyType`s no `Type`
/// effect can carry.
///
/// Not a *cross-arm* message: the requirement is the declaration's, so it
/// fires on a lone arm (`times`) where an arm-against-sibling rule never runs.
fn poly_arm_declared_effect_mismatch_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    declared: &str,
    found: &str,
    want: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: the quotation passed to `{word}` in {} (line {}) was declared `{declared}`, but it leaves {found} where that requires {want}\n  a non-shape-changing quotation parameter carries one row, the same on both sides: the arm must leave the row it entered with",
        where_,
        span.line
    )
}

/// P7 slice 3j (R3): the `ArmRule::Row` twin of
/// `poly_arm_declared_effect_mismatch_error` -- a *shape-changing* quotation
/// parameter (`~[ ..a -- ..b T1 .. Tn ]`) declares trailing outputs above the
/// row it produces, stripped back off an arm's exit before the row itself is
/// read (R2). This fires when that stripped suffix disagrees with the
/// declared types, or is too short to carry them at all.
fn poly_arm_declared_suffix_mismatch_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    declared: &str,
    found: &str,
    want: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: the quotation passed to `{word}` in {} (line {}) was declared `{declared}`, but it leaves {found} where that requires {want}\n  a shape-changing quotation parameter declares trailing outputs above the row it produces: the arm must leave those types, in order, above whatever row it leaves",
        where_,
        span.line
    )
}

/// P7 slice 3b-follow (OQ4): a combinator arm operand that is not a
/// splice-consumed quotation *literal* written at the call site -- a quotation
/// read back out of a local (the bind keeps the type and loses the identity),
/// a forwarded parameter, or a value that is not a quotation at all. Located
/// here rather than carried on: a materialised quotation in a generic body has
/// no runtime representation, and reaching lowering with one is a backend
/// panic.
fn poly_combinator_arm_not_a_literal_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    found: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{word}` in the polymorphic body of {} (line {}) needs a quotation literal written at the call site, found {found}\n  a quotation in a generic body is spliced where it is written: it cannot be bound to a local, forwarded, or returned",
        where_,
        span.line
    )
}

/// P7 slice 3b (R2/OQ2): eliminating a scrutinee that is not a concrete enum
/// -- a bare type variable, or a generic application. `'T` is *some* enum only
/// under an enum-kind bound, which is P7.S3d.
pub(super) fn poly_abstract_enum_scrutinee_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    found: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{word}` in {} (line {}) eliminates `{found}`, which is not a concrete enum\n  an abstract scrutinee needs an enum-kind bound on the type variable, which this slice does not have",
        where_,
        span.line
    )
}

/// P7 slice 3b (R2): eliminating *through a reference* inside a generic body.
/// Legal in a concrete body (decision 6), but every arm it could hand a
/// narrowed `&Shape.Rect` to would need the field projections a generic body
/// does not have (P7 slice 1), so it is rejected rather than half-supported.
pub(super) fn poly_reference_scrutinee_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    enum_name: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{word}` in {} (line {}) eliminates a reference, which is not yet supported in a generic body\n  pass the owned `{enum_name}` instead",
        where_,
        span.line
    )
}

/// P7.S12 (R3.4/R5.5): the poly twin of `eliminator_variant_escape_error` --
/// an arm leaves a *narrowed generic* variant (or a reference to one) on the
/// stack. `found` has no concrete `Type` to render, so it takes the already
/// `poly_type_str`-rendered spelling instead, the same rendering split
/// `poly_rendered_type_mismatch_error` draws against `type_mismatch_error`.
pub(super) fn poly_eliminator_variant_escape_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    found: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: an arm of `{word}` leaves `{found}` on the stack in {where_} (line {})\n  a variant-typed value is reachable only inside the arm that bound it; consume it there, or leave its fields instead",
        span.line,
    )
}

/// P7.S12 (R1.5/R7.4): the elimination twin of
/// `poly_combinator_generic_enum_construction_error`, and rejected for the
/// identical reason -- the operative monomorph depends on the combinator's own
/// type variable, so it varies per splice, and `CallInst::enum_words` is keyed
/// by `Span` alone.
pub(super) fn poly_combinator_generic_enum_elimination_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    enum_name: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{word}` in {} (line {}) eliminates `{enum_name}` at a type this combinator's own splice determines\n  a generic enum eliminated inside a combinator body is not yet supported: each splice would need its own resolution, and none is recorded",
        where_,
        span.line
    )
}

/// P7.S12 (R5.7/R7.4): a `&`/`&!`-mode arm tag over an *ungrounded* generic
/// scrutinee. Narrowing one needs `intern_ref_type` over a shape that has no
/// `Type` yet, so this slice admits the owning mode only. Located rather than
/// silently narrowed to owning, which would let an arm consume a value the
/// caller only lent.
pub(super) fn poly_generic_scrutinee_ref_tag_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    tag: &str,
    enum_name: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: this arm of `{word}` is tagged `( &{tag} )`, but `{word}` eliminates the ungrounded `{enum_name}` in {where_} (line {})\n  a reference-mode arm over a generic scrutinee is not yet supported: write `( {tag} )` and take the variant by value",
        span.line,
    )
}

/// P7.S12 (R5.3): the `PolyType` sibling of
/// `ordinary_literal_at_inline_param_error` -- an eliminator arm written with
/// an ordinary `[ ... ]` bracket whose narrowed input is a `GenericVariant`,
/// which has no `Type` to intern a real `~[ ... ]` parameter from. Same
/// rendering split `poly_rendered_type_mismatch_error` draws against
/// `type_mismatch_error`; the concrete case keeps the original message.
pub(super) fn poly_ordinary_literal_at_inline_param_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    param: &str,
) -> String {
    let word = crate::resolve::render_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: this argument is an ordinary `[ ... ]` quotation but {word} declares parameter `{param}` as inline `~[ ... ]`; write it `~[ ... ]` in {where_} (line {})",
        span.line,
    )
}

/// P7 slice 3b (R3/L1): two eliminator arms leaving structurally different
/// types at one exit position, under rigid type variables -- `'T` against
/// `'U`, or `'T` against `i64`. Binding either would be a mid-body
/// unification, which would silently retype the sibling arms already checked.
pub(super) fn poly_arm_output_disagreement_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    expected: &str,
    found: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: the arms of `{word}` in {} (line {}) disagree: an earlier one leaves `{expected}`, this one leaves `{found}`\n  a type variable is rigid across arms: it is never bound to the other arm's type",
        where_,
        span.line
    )
}

/// P7 slice 3b (R3/L4): one place borrowed at two different mutabilities
/// across two arms. The union that merges the arms' borrow tables cannot
/// represent both, and erasing either would read as "no conflict" at a later
/// use of that place -- a false accept, so it is named instead.
pub(super) fn poly_arm_borrow_disagreement_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    a: &PolyBorrow,
    b: &PolyBorrow,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    let sigil = |b: &PolyBorrow| if b.mutable { "&!" } else { "&" };
    format!(
        "error: the arms of `{word}` in {} (line {}) borrow `{}` differently: `{}{}` (line {}) against `{}{}` (line {})\n  one place is borrowed at one mutability across every arm, or the merged table could not answer a later use of it",
        where_,
        span.line,
        a.place,
        sigil(a),
        a.place,
        a.span.line,
        sigil(b),
        b.place,
        b.span.line,
    )
}

/// P7 slice 3b (R3): a linear local bound *inside* an eliminator arm and
/// never consumed there. The concrete path gets this for free from block
/// exit (`Scope::leave`); the poly walk has no block scope, so the arm walk
/// checks it explicitly before truncating the arm's locals away.
pub(super) fn poly_arm_local_not_consumed_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    local: &str,
    ty: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: the local `{local}` of type `{ty}`, bound in an arm of `{word}` in {} (line {}), is never consumed\n  nothing is dropped for you: consume it in the arm that binds it",
        where_,
        span.line
    )
}

/// P7.S6c (R3): `&>`/`&!>` on a generic-length array (`array['T 'N]`) with a
/// **literal** `i64` index -- a `usize` index (bare local, or `>usize`
/// conversion's result) is admitted and defers to the runtime bounds guard;
/// only a literal index against an unknown length still cannot be
/// range-checked at all, since there is no count to check it against and no
/// value to convert.
pub(super) fn poly_generic_length_index_error(ctx: &Ctx, span: Span, len_var: &str) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot index a generic-length array with a literal index in {where_} (line {}, col {})\n  the array's length is the type variable `{len_var}`, so a literal index cannot be statically bounds-checked; use a `usize` index instead (a bound local, or `>usize` on a computed value), which defers the check to runtime",
        span.line, span.col
    )
}

pub(super) fn poly_output_mismatch_error(
    word: &WordDef,
    sig: &PolySig,
    residual: &[PolyType],
) -> String {
    let got: Vec<String> = residual.iter().map(|pt| poly_type_str(pt, sig)).collect();
    let want: Vec<String> = sig
        .outputs
        .iter()
        .map(|pt| poly_type_str(pt, sig))
        .collect();
    format!(
        "error: stack effect mismatch in `{}`\n  body leaves `{}`, but the declared outputs are `{}`",
        crate::resolve::demangle_word(&word.name),
        got.join(" "),
        want.join(" "),
    )
}

pub(super) fn poly_copy_bound_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    ty: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}` in {name} (line {})\n  `{ty}` is linear and has no `Copy` instance, so a linear value cannot be duplicated; `{var}: Copy` is unsatisfied",
            span.line
        , name = ctx.rendered_word())
}

pub(super) fn poly_var_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    a: Type,
    b: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    format!(
        "error: `{callee}` in {name} (line {line}) resolved `{var}` to both `{a}` and `{b}`",
        name = ctx.rendered_word()
    )
}

/// P7.S3t (R5): an operand disagreeing with the type this call site was
/// explicitly instantiated at. `poly_var_conflict_error`'s symmetric "resolved
/// `'T` to both" would be the wrong shape: here one end is written and one is
/// inferred, and the remedy differs by which the caller meant.
pub(super) fn explicit_instantiation_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    instantiated: Type,
    operand: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    format!(
            "error: `{callee}` in {name} (line {line}) was instantiated at `{var}` = `{instantiated}` but its operand is `{operand}`",
            name = ctx.rendered_word()
        )
}

/// P7b.S8b (round-1 review, P2): an explicit type argument that disagrees
/// with what the impl target already determined for that variable
/// (`empty[Pair[i64 i64] str]`: the dispatch type fixes `'B` = `i64`, the
/// list writes `str`). Neither
/// `explicit_instantiation_conflict_error` nor `poly_var_conflict_error`
/// fits: both name an *operand* as one end, and here both ends are the
/// caller's own `[...]` list -- one entry written directly, the other fixed
/// by the dispatch type the list's first entry names -- so "but its operand
/// is" would name a thing the caller never wrote. Says which end is which,
/// since the remedy differs by what the caller meant.
pub(super) fn impl_target_seed_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    determined: Type,
    written: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    format!(
        "error: `{callee}` in {name} (line {line}) was written with `{var}` = `{written}`, but its impl target determines `{var}` = `{determined}`",
        name = ctx.rendered_word()
    )
}

/// P7.S6b (R5): `explicit_instantiation_conflict_error`'s length-typed
/// sibling -- a length conflict compares two `u32`s, not two `Type`s, and
/// `u32`'s `Display` renders identically, so a thin sibling is simpler than
/// generalizing the existing function over a trait.
pub(super) fn explicit_len_instantiation_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    instantiated: u32,
    operand: u32,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    format!(
            "error: `{callee}` in {name} (line {line}) was instantiated at length `{var}` = `{instantiated}` but its operand is `{operand}`",
            name = ctx.rendered_word()
        )
}

/// P7.S3t (R4): an explicit type-argument list whose length is not the
/// callee's declared type-variable count. Exact, never a prefix: a partial
/// list would make position `i`'s meaning depend on which variables the
/// callee's *inputs* ground, so adding an input would re-point every existing
/// call site. Length (`'N`) and row (`..s`) variables are not addressable by
/// this list at all, which is worth saying whenever the callee has one.
pub(super) fn instantiation_arity_error(
    span: Span,
    callee: &str,
    sig: &PolySig,
    given: usize,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let declared = match sig.ty_var_names.len() {
        0 => "no type variables".to_string(),
        1 => format!("1 type variable (`{}`)", sig.ty_var_names[0]),
        n => format!(
            "{n} type variables ({})",
            sig.ty_var_names
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let plural = match given {
        1 => "",
        _ => "s",
    };
    let note = match sig.row_var_names.is_empty() {
        true => String::new(),
        false => "\n  note: a row (`..s`) variable is not named by an explicit instantiation; only type and length variables are".to_string(),
    };
    format!(
        "error: `{callee}` (line {}) declares {declared} but was given {given} type argument{plural}{note}",
        span.line
    )
}

/// P7.S6b (R3): the length twin of `instantiation_arity_error`, mirroring
/// S6a's `generic_arity_error` two-count shape.
pub(super) fn length_instantiation_arity_error(
    span: Span,
    callee: &str,
    sig: &PolySig,
    given: usize,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let declared = match sig.len_var_names.len() {
        0 => "no length variables".to_string(),
        1 => format!("1 length variable (`{}`)", sig.len_var_names[0]),
        n => format!(
            "{n} length variables ({})",
            sig.len_var_names
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let plural = match given {
        1 => "",
        _ => "s",
    };
    format!(
        "error: `{callee}` (line {}) declares {declared} but was given {given} length argument{plural}",
        span.line
    )
}

pub(super) fn poly_len_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    a: u32,
    b: u32,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let line = span.line;
    format!(
        "error: `{callee}` in {name} (line {line}) resolved length `{var}` to both `{a}` and `{b}`",
        name = ctx.rendered_word()
    )
}

pub(super) fn poly_array_expected_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    found: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    format!(
            "error: type mismatch in {name} (line {})\n  `{callee}` expected an array operand, found `{found}`",
            span.line
        , name = ctx.rendered_word())
}

pub(super) fn poly_unbound_output_error(ctx: &Ctx, span: Span, callee: &str, var: &str) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{callee}` in {where_} (line {}) has output variable `{var}` that no input binds",
        span.line
    )
}

/// P7.S3t (R9): the same message for a *type* variable, which an explicit
/// instantiation can now supply. The base text is unchanged; only the remedy
/// is new, and it is not offered for a length variable, which R4 leaves
/// unaddressable. The remedy names one `SomeType` per declared type variable
/// (`ty_var_count`): a callee with more than one would otherwise be told a
/// syntax that immediately re-fails on arity.
pub(super) fn poly_unbound_output_ty_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    ty_var_count: usize,
) -> String {
    let args = vec!["SomeType"; ty_var_count].join(" ");
    format!(
        "{}\n  note: supply it explicitly: `{}[{}]`",
        poly_unbound_output_error(ctx, span, callee, var),
        crate::resolve::demangle_call(callee),
        args
    )
}

/// A variable id `sig` cannot name, rendered rather than indexed.
///
/// P7b.S8b (round-3 review, P0): a `PolyType` reaching a diagnostic does not
/// always belong to the signature it is rendered against. A construction
/// field's variables live in *its own header's* declaration space (`Holder['T]
/// r Ring['T 3]` stores `Var(0)` for `Holder`'s `'T`), and the caller whose
/// `sig` renders the mismatch may declare fewer variables -- or none. Indexing
/// panicked there, which made a *diagnostic* the thing that crashed the
/// compiler: an error path is exactly where the checker must stay total.
///
/// The placeholder is `'?` (a variable this signature cannot name) plus the
/// domain and the raw id, so the two sides of a mismatch stay distinguishable
/// and a reader can tell a type variable from a length one. In-range ids are
/// unaffected: they render by their declared spelling, byte for byte.
fn foreign_var_str(table: &[String], id: u32, domain: &str) -> String {
    match table.get(id as usize) {
        Some(name) => name.clone(),
        None => format!("'?{domain}{id}"),
    }
}

/// Render a `PolyType` for a diagnostic: a variable by its declared spelling,
/// a concrete type by its name, an array structurally. Total on variable ids
/// (`foreign_var_str`): a rendering is never the reason a build panics.
pub(crate) fn poly_type_str(pt: &PolyType, sig: &PolySig) -> String {
    match pt {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => foreign_var_str(&sig.ty_var_names, *v, ""),
        // P7 slice 3b (R2): no effect to render (that is the point of the
        // marker), so it renders as what it is.
        PolyType::QuotLit => "a quotation literal".to_string(),
        PolyType::Array(elem, len) => {
            let l = match len {
                Len::Concrete(n) => n.to_string(),
                Len::Var(id) => foreign_var_str(&sig.len_var_names, *id, "len"),
            };
            format!("array[{} {}]", poly_type_str(elem, sig), l)
        }
        PolyType::Quotation(ins, outs, is_inline, row_in, row_out) => {
            // Slice 10a (R10): the row is a separate field, not a slot in
            // `ins`/`outs`, so it is rendered as the leading element of its
            // side, exactly as `poly_sig_str` renders the top-level row.
            let row = |r: &[PolyType], row_var: Option<u32>| {
                let mut parts: Vec<String> = Vec::new();
                if let Some(v) = row_var {
                    parts.push(foreign_var_str(&sig.row_var_names, v, "row"));
                }
                parts.extend(r.iter().map(|p| poly_type_str(p, sig)));
                parts.join(" ")
            };
            let (i, o) = (row(ins, *row_in), row(outs, *row_out));
            let sigil = if *is_inline { "~" } else { "" };
            match (i.is_empty(), o.is_empty()) {
                (true, true) => format!("{sigil}[ -- ]"),
                (true, false) => format!("{sigil}[ -- {o} ]"),
                (false, true) => format!("{sigil}[ {i} -- ]"),
                (false, false) => format!("{sigil}[ {i} -- {o} ]"),
            }
        }
        // Slice 13 (R-A9): the surface spelling, `&`/`&!` glued to the
        // referent, exactly as `intern_ref_type` names a concrete one.
        PolyType::Ref(referent, mutable) => format!(
            "&{}{}",
            if *mutable { "!" } else { "" },
            poly_type_str(referent, sig)
        ),
        // P7.S3n (R3): the surface spelling, `^` glued to the payload,
        // exactly as `intern_owned_cell_type` names a concrete one.
        PolyType::OwnedCell(payload) => format!("^{}", poly_type_str(payload, sig)),
        // P7 slice 3a: `Name['A 'B]` in the signature's own variable
        // spellings -- `name` is cached on the variant for exactly this
        // (see `PolyType::Generic`'s doc), so no registry lookup is needed.
        PolyType::Generic {
            name,
            args,
            len_args,
            ..
        } => {
            let mut parts: Vec<String> = args.iter().map(|a| poly_type_str(a, sig)).collect();
            parts.extend(len_args.iter().map(|l| match l {
                Len::Concrete(n) => n.to_string(),
                Len::Var(id) => foreign_var_str(&sig.len_var_names, *id, "len"),
            }));
            format!("{name}[{}]", parts.join(" "))
        }
        // P7.S12 (R3.3): `Enum.Variant`, mirroring `Type::Variant`'s own
        // display spelling -- `name` is cached on the variant for exactly
        // this (`generic_variant_type`'s doc), so no registry lookup needed.
        PolyType::GenericVariant { name, .. } => name.to_string(),
        // P7b.S1 (S1-14): `'F['T]` -- the applied variable's own surface
        // spelling, then its arguments.
        PolyType::App { head, args } => {
            let parts: Vec<String> = args.iter().map(|a| poly_type_str(a, sig)).collect();
            format!(
                "{}[{}]",
                foreign_var_str(&sig.ty_var_names, *head, ""),
                parts.join(" ")
            )
        }
    }
}

#[cfg(test)]
mod tests;
