use std::cell::RefCell;
use std::cmp::Ordering;

use crate::ast::GenericTypes;

use super::*;

/// P7.S3e (R7): a trait-member call recorded abstractly while a polymorphic
/// body is walked -- which trait, which member, on which of the walked word's
/// own type variables. The *symbol* is deliberately absent: `'T` is still
/// abstract here, so only the obligation is knowable. `check_poly_call`
/// resolves it against a concrete `θ` (R8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraitObligation {
    pub span: Span,
    /// Index into the *walked word's* `PolySig::ty_var_names`.
    pub var: u32,
    pub trait_id: TraitId,
    pub member: String,
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
    pub recorded: &'a [WordObligations],
    /// P7.S12 (R1.2): the generated-enum-word call sites recorded for every
    /// non-combinator polymorphic word, resolved against a concrete θ at
    /// `check_poly_call` the same way `recorded` is.
    pub enum_sites_recorded: &'a [WordEnumSites],
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
            recorded: &[],
            enum_sites_recorded: &[],
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
#[allow(clippy::too_many_arguments)]
pub(super) fn check_poly_combinator_standalone(
    word: &WordDef,
    sig: &PolySig,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    structs: &[StructDecl],
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    const STANDALONE_LEN: u32 = 4;
    // P7 slice 3a: construction (R3) is scoped to an ordinary poly word's
    // own body, not a combinator's standalone stand-in check -- `None` here,
    // never threaded in from a caller, keeps that scope decision in one
    // place rather than relying on every caller to also decline it.
    let ctx = word_ctx(
        word,
        structs,
        enums,
        statics,
        modules,
        poly.combinators.tail(),
        None,
    );
    let span = word_span(word);
    let mut subst = Subst::default();
    for v in 0..sig.ty_var_names.len() as u32 {
        subst.ty.push((v, Type::I64));
    }
    for ln in 0..sig.len_var_names.len() as u32 {
        subst.len.push((ln, STANDALONE_LEN));
    }
    let mut inputs = Vec::with_capacity(sig.inputs.len());
    for pty in &sig.inputs {
        let ty = apply_subst(
            sig, pty, &subst, &word.name, span, &ctx, arrays, cells, refs,
        )?;
        inputs.push(TypedSlot { name: None, ty });
    }
    let mut outputs = Vec::with_capacity(sig.outputs.len());
    for pty in &sig.outputs {
        let ty = apply_subst(
            sig, pty, &subst, &word.name, span, &ctx, arrays, cells, refs,
        )?;
        outputs.push(TypedSlot { name: None, ty });
    }
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
        enums,
        env,
        arrays,
        cells,
        refs,
        slices,
        structs,
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
    result
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
    let eliminators = eliminator_registry(enums);
    for (at, term) in terms.iter().enumerate() {
        if let TermKind::Quotation(_, _, Some(annot)) = &term.kind {
            if let Some(tag) = &annot.variant_tag {
                if !tagged_literal_reaches_an_eliminator_call(terms, at, &eliminators) {
                    return Err(eliminator_arm_outside_call_error(
                        ctx, annot.span, &tag.name,
                    ));
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
        TermKind::Call(name, type_args) => {
            // P7.S3t (R1/R3): a call inside a polymorphic word's own body is
            // checked symbolically -- there is no `Subst` here to seed, and
            // reaching one would be the multi-hop forwarding case R7 leaves
            // out of the slice. Rejected rather than dropped: a dropped list
            // links whatever the symbolic path resolved instead.
            if !type_args.is_empty() {
                return Err(type_arguments_in_poly_body_error(ctx, span, name));
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

/// P7.S3p (ruling 4, amended): the one candidate whose declared operands fit
/// the stack window, when the candidates span *more than one* type variable.
///
/// Which variable's obligation a call means is decided by which operands it
/// consumes -- as it was under S3e's top-of-stack discovery, which is what
/// keeps `f ( &'T: A &'U: A -- ) ta ta` (one trait, two variables, two
/// obligations resolved against their own thetas) callable. Candidates sharing
/// one variable are never separated this way: there the operands are the same
/// either way and picking a trait by shape would be an overload resolution the
/// language does not have, so `'T: A B` calling a shared member stays the
/// ambiguity error S3e made it.
///
/// The fit must be unique: two candidates on one variable fit the same
/// operands (that is the same-variable ambiguity, reachable here through a
/// mixed set -- two traits on `'T` plus one on `'U`), and no candidate fits a
/// call whose operands are wrong for every one of them. Those two failure
/// modes are not the same diagnostic: the first is resolved by a module
/// qualifier (it narrows *which* trait), the second is not (no candidate's
/// declared operands match, so naming a trait changes nothing) -- `CandidateFit`
/// keeps them apart for the caller.
fn candidate_fitting_the_operands<'a>(
    stack: &[PolySlot],
    candidates: &[(u32, TraitId, &'a TraitMember)],
) -> CandidateFit<'a> {
    if candidates.windows(2).all(|w| w[0].0 == w[1].0) {
        return CandidateFit::Ambiguous;
    }
    let mut fitting = candidates.iter().filter(|(var, _, member)| {
        let inputs = &member.sig.inputs;
        stack.len() >= inputs.len() && {
            let base = stack.len() - inputs.len();
            inputs
                .iter()
                .enumerate()
                .all(|(i, t)| stack[base + i].pt == substitute_member_var(t, *var))
        }
    });
    match (fitting.next(), fitting.next()) {
        (Some(one), None) => CandidateFit::Unique(*one),
        (Some(_), Some(_)) => CandidateFit::Ambiguous,
        (None, _) => CandidateFit::NoFit,
    }
}

enum CandidateFit<'a> {
    Unique((u32, TraitId, &'a TraitMember)),
    Ambiguous,
    NoFit,
}

/// P7.S3e (R7): a trait member's signature is written over the trait's own
/// single type variable (id 0 in the member's own `PolySig`); dispatching it
/// through a bound rewrites that variable to the bounded variable of the
/// *calling* word's signature, so the result is comparable against the walk's
/// stack directly.
fn substitute_member_var(t: &PolyType, var: u32) -> PolyType {
    match t {
        PolyType::Var(_) => PolyType::Var(var),
        PolyType::Ref(referent, mutable) => {
            PolyType::Ref(Box::new(substitute_member_var(referent, var)), *mutable)
        }
        PolyType::Array(elem, len) => {
            PolyType::Array(Box::new(substitute_member_var(elem, var)), len.clone())
        }
        other => other.clone(),
    }
}

/// P7.S3o Phase 3: resolve a bare trait member call at a combinator splice
/// site, where θ is concrete. Mirrors `poly_trait_member_call`'s matching
/// logic (find the member in the sig's `Bound::User` bounds), but operates on
/// the concrete `Slot` stack and resolves the `impl:` symbol immediately
/// (rather than recording an abstract obligation) because θ is known here.
///
/// Returns `Ok(Some(stack))` if `name` matched a bare member and was
/// dispatched; `Ok(None)` if it did not match (and ordinary dispatch should
/// proceed). The resolved `impl:` symbol is recorded in
/// `poly.splice_trait_calls` keyed by `(uid, span)` when `prov.splice_uid` is
/// `Some` (a real splice); during the standalone check (`splice_uid` is
/// `None`) the symbol is resolved for stack-effect accounting only and not
/// recorded.
/// P7.S3o Phase 3: resolve a bare trait member call at a combinator splice
/// site, where θ is concrete. Mirrors `poly_trait_member_call`'s matching
/// logic (find the member in the sig's `Bound::User` bounds), but operates on
/// the concrete `Slot` stack and resolves the `impl:` symbol immediately
/// (rather than recording an abstract obligation) because θ is known here.
///
/// Returns `Ok(Some(stack))` if `name` matched a bare member and was
/// dispatched; `Ok(None)` if it did not match (and ordinary dispatch should
/// proceed). The resolved `impl:` symbol is recorded in
/// `poly.splice_trait_calls` keyed by `(uid, span)` when `prov.splice_uid` is
/// `Some` (a real splice); during the standalone check (`splice_uid` is
/// `None`) the symbol is resolved for stack-effect accounting only and not
/// recorded.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_splice_member_call(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    refs: &mut Vec<RefDecl>,
    poly: &mut PolyCtx,
    prov: &mut Provenance,
) -> Result<Option<Vec<Slot>>, String> {
    let Some(sig) = poly.combinator_sig.clone() else {
        return Ok(None);
    };
    let Some(subst) = poly.combinator_subst.clone() else {
        return Ok(None);
    };
    let traits = poly.trait_resolve.traits;
    // R12/decision 6: a qualified call names a *module* alias, never a trait
    // namespace. Same split as `poly_trait_member_call`.
    let (qualifier, member) = match name.split_once("::") {
        Some((q, m)) => (Some(q), m),
        None => (None, name),
    };
    let qualified_target = match qualifier {
        Some(q) => match ctx
            .modules()
            .and_then(|ms| ms.get(ctx.module() as usize))
            .and_then(|m| m.imports.get(q))
        {
            Some(&target) => Some(target),
            None => return Ok(None),
        },
        None => None,
    };
    // Find the matching (var, trait_id, member_decl) from the sig's bounds.
    // Same dedup and ambiguity logic as `poly_trait_member_call`.
    let mut seen: Vec<(u32, TraitId)> = Vec::new();
    let mut matched: Vec<(u32, TraitId, &TraitMember)> = Vec::new();
    for (v, bound) in &sig.bounds {
        let Bound::User(tid) = bound else { continue };
        if qualified_target.is_some_and(|t| traits[tid.index()].module != t)
            || seen.contains(&(*v, *tid))
        {
            continue;
        }
        seen.push((*v, *tid));
        if let Some(m) = traits[tid.index()]
            .members
            .iter()
            .find(|m| m.name == member)
        {
            matched.push((*v, *tid, m));
        }
    }
    let (var, trait_id, member_decl) = match matched.as_slice() {
        [] => return Ok(None),
        [one] => *one,
        many => {
            // Disambiguate by concrete operand shape: ground each
            // candidate's input types against the concrete θ and check
            // which match the live stack.
            let mut fitting: Vec<(u32, TraitId, &TraitMember)> = Vec::new();
            for (v, tid, m) in many {
                let Some(ty) = subst.ty_of(*v) else { continue };
                let inputs: Vec<Type> = m
                    .sig
                    .inputs
                    .iter()
                    .map(|t| crate::ast::ground_member_type(t, ty, arrays, refs))
                    .collect();
                if stack.len() >= inputs.len() {
                    let base = stack.len() - inputs.len();
                    if inputs.iter().enumerate().all(|(i, want)| {
                        matches!(
                            match_slot(stack[base + i], *want),
                            SlotMatch::Exact | SlotMatch::LiteralSizeType
                        )
                    }) {
                        fitting.push((*v, *tid, *m));
                    }
                }
            }
            match fitting.as_slice() {
                [] => {
                    let shapes: Vec<(&str, &str, String)> = matched
                        .iter()
                        .map(|(v, tid, m)| {
                            let inputs: Vec<String> = m
                                .sig
                                .inputs
                                .iter()
                                .map(|t| poly_type_str(&substitute_member_var(t, *v), &sig))
                                .collect();
                            (
                                traits[tid.index()].name.as_str(),
                                sig.ty_var_names[*v as usize].as_str(),
                                inputs.join(" "),
                            )
                        })
                        .collect();
                    return Err(no_candidate_fits_operands_error(ctx, span, member, &shapes));
                }
                [one] => *one,
                _ => {
                    let named: Vec<(&str, &str)> = matched
                        .iter()
                        .map(|(v, tid, _)| {
                            (
                                traits[tid.index()].name.as_str(),
                                sig.ty_var_names[*v as usize].as_str(),
                            )
                        })
                        .collect();
                    return Err(ambiguous_trait_member_error(span, member, &named));
                }
            }
        }
    };
    // P7.S3o Phase 4 (R5): reject bound dispatch inside a materialized
    // quotation from a bounded combinator. The materialized quotation gets
    // its own `IrFunc` with `inline_uid: 0` during lowering, so the per-splice
    // `splice_trait_calls` key `(uid, span)` recorded here would not be found
    // — two splices at different types would silently miscompile. The flag
    // is set by `materialize_quotation_at_boundary` (and the branch join)
    // around the quotation body check; `splice_uid` being `Some` means this
    // is a real splice, not the standalone check.
    if prov.in_materialized_quot && prov.splice_uid.is_some() {
        let comb_name = poly
            .combinator_name
            .as_deref()
            .map(|n| crate::resolve::render_word(n).into_owned())
            .unwrap_or_else(|| "a bounded combinator".to_string());
        return Err(format!(
            "error: bound trait member `{member}` (line {}) is called inside a materialized quotation within the combinator {comb_name}, but bound dispatch in materialized quotations is unsupported\n\
             note: a materialized quotation is lowered to its own function with no splice-site prefix, so two splices at different concrete types would collide on the dispatch key",
            span.line
        ));
    }
    // Ground the member's input/output types against the concrete θ.
    let Some(ty) = subst.ty_of(var) else {
        return Ok(None);
    };
    let input_types: Vec<Type> = member_decl
        .sig
        .inputs
        .iter()
        .map(|t| crate::ast::ground_member_type(t, ty, arrays, refs))
        .collect();
    let output_types: Vec<Type> = member_decl
        .sig
        .outputs
        .iter()
        .map(|t| crate::ast::ground_member_type(t, ty, arrays, refs))
        .collect();
    // Check operands against the grounded input types.
    let n_in = input_types.len();
    if stack.len() < n_in {
        return Err(underflow_error(ctx, span, member, n_in, stack.len()));
    }
    let base = stack.len() - n_in;
    for (i, want) in input_types.iter().enumerate() {
        let found = stack[base + i];
        if found.quot.is_some() {
            return Err(reject_quotation_argument(ctx, span, name));
        }
        match match_slot(found, *want) {
            SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
            SlotMatch::NeedsSizeConversion => {
                return Err(size_conversion_needed_error(ctx, span, name, *want));
            }
            SlotMatch::NeedsStrToCstrConversion => {
                return Err(str_needs_cstr_conversion_error(ctx, span, name));
            }
            SlotMatch::Mismatch => {
                return Err(trait_member_operand_error(
                    ctx,
                    span,
                    member,
                    &traits[trait_id.index()].name,
                    &PolyType::Concrete(*want),
                    &PolyType::Concrete(found.ty),
                    &sig,
                ));
            }
        }
    }
    // Resolve the `impl:` symbol for this concrete type. At a real splice
    // (`splice_uid` is `Some`), the bound must be satisfied by the concrete
    // type and the resolved symbol is recorded for lowering. During the
    // standalone check (`splice_uid` is `None`), the i64 stand-in may not
    // satisfy the bound (there is no `impl: Show for i64`), so the impl
    // resolution is skipped — only the stack-effect accounting matters
    // here; the member call is re-checked at each real splice site where θ is
    // concrete.
    if let Some(uid) = prov.splice_uid {
        let trait_decl = &poly.trait_resolve.traits[trait_id.index()];
        let Some(imp) = poly.trait_resolve.impls.iter().find(|i| {
            i.trait_id == trait_id
                && match_impl_target(&i.target.pattern, ty, &[], &[], &[], None).is_some()
        }) else {
            return Err(unsatisfied_user_bound_error(
                ctx,
                span,
                name,
                &sig.ty_var_names[var as usize],
                trait_decl,
                ty,
                arrays,
                refs,
            ));
        };
        let symbol = imp
            .resolved
            .iter()
            .find(|(mname, _)| mname == member)
            .and_then(|(_, idx)| poly.trait_resolve.word_symbols.get(*idx));
        let Some(symbol) = symbol else {
            return Err(unresolved_trait_obligation_error(
                ctx,
                span,
                name,
                &trait_decl.name,
                member,
                ty,
                span,
            ));
        };
        poly.splice_trait_calls.insert((uid, span), symbol.clone());
    }
    // Consume operands, produce outputs.
    stack.truncate(base);
    for ty in &output_types {
        stack.push(Slot::computed(*ty));
    }
    Ok(Some(std::mem::take(stack)))
}
/// P7.S3e (R7/R12): the bound-directed dispatch branch. `Ok(None)` means this
/// is no trait-member obligation and ordinary dispatch proceeds: no
/// `Bound::User` on any of this word's type variables declares a member of
/// this name.
///
/// P7.S3p (ruling 1/2): the candidate variable comes from the *bounds*, found
/// by member name, never from the stack -- so a member dispatches whatever
/// input position its receiver sits at.
///
/// P7.S3p (ruling 4, amended): selection reads operand shape only when the
/// name-based search above finds candidates spanning more than one variable
/// (`candidate_fitting_the_operands`); a single-variable match never touches
/// operand shape at all. The per-input check below stays the sole place an
/// operand *mismatch on the resolved candidate* is reported -- shape only
/// ever disambiguates *which* candidate, never validates the one it settles
/// on.
///
/// The obligation records *which trait, which member, which variable* and no
/// symbol: `'T` is still abstract here, so the implementing word is unknowable
/// until a call site grounds it (R8).
fn poly_trait_member_call(
    name: &str,
    span: Span,
    stack: &mut Vec<PolySlot>,
    sig: &PolySig,
    ctx: &Ctx,
    tctx: &mut TraitCtx,
) -> Result<Option<Vec<PolySlot>>, String> {
    let traits = tctx.traits;
    // R12/decision 6: a qualified call names a *module* alias, never a trait
    // namespace -- it restricts the search to traits that module declares.
    // `Resolver::rewrite` leaves an unrecognized qualified word raw, which is
    // exactly what this needs.
    //
    // R18: matched raw, never demangled. `rewrite` runs before the checker, so
    // a member name that also names a word the target module declares or
    // re-exports has already been rewritten to that word's mangled symbol by
    // the time control reaches here -- and nothing downstream can tell a trait
    // member was intended. Un-mangling here would make the trait silently win
    // that collision; leaving it mangled falls through to ordinary dispatch,
    // which is the ruled rejection.
    let (qualifier, member) = match name.split_once("::") {
        Some((q, m)) => (Some(q), m),
        None => (None, name),
    };
    let qualified_target = match qualifier {
        Some(q) => match ctx
            .modules()
            .and_then(|ms| ms.get(ctx.module() as usize))
            .and_then(|m| m.imports.get(q))
        {
            Some(&target) => Some(target),
            None => return Ok(None),
        },
        None => None,
    };
    // `'T: A A` parses, so one trait can appear twice on one variable; without
    // the dedupe it reads as its own ambiguity ("required by both `A` and
    // `A`").
    let mut seen: Vec<(u32, TraitId)> = Vec::new();
    let mut matched: Vec<(u32, TraitId, &TraitMember)> = Vec::new();
    for (v, bound) in &sig.bounds {
        let Bound::User(tid) = bound else { continue };
        if qualified_target.is_some_and(|t| traits[tid.index()].module != t)
            || seen.contains(&(*v, *tid))
        {
            continue;
        }
        seen.push((*v, *tid));
        if let Some(m) = traits[tid.index()]
            .members
            .iter()
            .find(|m| m.name == member)
        {
            matched.push((*v, *tid, m));
        }
    }
    let (var, trait_id, member_decl) = match matched.as_slice() {
        [] => return Ok(None),
        [one] => *one,
        // R12/decision 5: composing two traits that happen to share a member
        // name is legal to declare; only the ambiguous *call* is the error.
        many => match candidate_fitting_the_operands(stack, many) {
            CandidateFit::Unique(one) => one,
            CandidateFit::Ambiguous => {
                let named: Vec<(&str, &str)> = matched
                    .iter()
                    .map(|(v, tid, _)| {
                        (
                            traits[tid.index()].name.as_str(),
                            sig.ty_var_names[*v as usize].as_str(),
                        )
                    })
                    .collect();
                return Err(ambiguous_trait_member_error(span, member, &named));
            }
            CandidateFit::NoFit => {
                let shapes: Vec<(&str, &str, String)> = matched
                    .iter()
                    .map(|(v, tid, m)| {
                        let inputs: Vec<String> = m
                            .sig
                            .inputs
                            .iter()
                            .map(|t| poly_type_str(&substitute_member_var(t, *v), sig))
                            .collect();
                        (
                            traits[tid.index()].name.as_str(),
                            sig.ty_var_names[*v as usize].as_str(),
                            inputs.join(" "),
                        )
                    })
                    .collect();
                return Err(no_candidate_fits_operands_error(ctx, span, member, &shapes));
            }
        },
    };
    let inputs: Vec<PolyType> = member_decl
        .sig
        .inputs
        .iter()
        .map(|t| substitute_member_var(t, var))
        .collect();
    if stack.len() < inputs.len() {
        return Err(underflow_error(
            ctx,
            span,
            member,
            inputs.len(),
            stack.len(),
        ));
    }
    let base = stack.len() - inputs.len();
    for (i, expected) in inputs.iter().enumerate() {
        if &stack[base + i].pt != expected {
            return Err(trait_member_operand_error(
                ctx,
                span,
                member,
                &traits[trait_id.index()].name,
                expected,
                &stack[base + i].pt,
                sig,
            ));
        }
    }
    stack.truncate(base);
    for out in &member_decl.sig.outputs {
        stack.push(PolySlot::new(substitute_member_var(out, var)));
    }
    tctx.obligations.push(TraitObligation {
        span,
        var,
        trait_id,
        member: member.to_string(),
    });
    Ok(Some(std::mem::take(stack)))
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
    // builtin's own hardcoded `Instr::Bin`/`Instr::Cmp`/`Instr::Print` arm.
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
    if let Some(id) = eliminator_registry(enums).get(name).copied() {
        return poly_eliminator_call(
            id,
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
    let chosen = env.get(name).and_then(|candidates| match &candidates[..] {
        [only] => Some(only),
        _ => candidates.iter().find(|o| {
            stack.len() >= o.sig.inputs.len()
                && stack[stack.len() - o.sig.inputs.len()..]
                    .iter()
                    .zip(&o.sig.inputs)
                    .all(|(s, inp)| matches!(&s.pt, PolyType::Concrete(t) if t == inp))
        }),
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

/// P7.S3k (R1-R3/R6): a call from one polymorphic body to a *different*
/// polymorphic word. Neither side has a θ here, so nothing is unified against
/// a concrete type: the callee's declared inputs are matched **structurally**
/// against the caller's operand slots, and what comes out is a
/// variable-to-variable mapping (R2) recorded for later composition, never a
/// `Subst`. The self-call arm above is not an instance of this -- it reuses
/// the walk's own `sig` and needs no mapping at all.
#[allow(clippy::too_many_arguments)]
fn poly_cross_call(
    name: &str,
    span: Span,
    mut stack: Vec<PolySlot>,
    sig: &PolySig,
    ctx: &Ctx,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    traits: &[TraitDecl],
    candidates: &[PolySig],
    cross: &mut CrossCtx,
) -> Result<Vec<PolySlot>, String> {
    // R2: which candidate this operand run selects. A lone candidate is the
    // ordinary case and is used as-is, so its own rejection is what the caller
    // is told; an overload set is resolved by trying each in declaration
    // order, the first-match-wins rule `resolve_combinator_overload` already
    // applies -- with no ground type in hand there is nothing to rank
    // candidates by.
    let callee_sig = match candidates {
        [only] => only,
        _ => {
            let matched = candidates
                .iter()
                .find(|csig| poly_cross_relate(csig, name, &stack, span, ctx, sig).is_ok());
            match matched {
                Some(csig) => csig,
                None => return Err(no_poly_overload_matches_error(ctx, span, name, candidates)),
            }
        }
    };
    let mapping = poly_cross_relate(callee_sig, name, &stack, span, ctx, sig)?;
    // R3: every bound the callee declares on a mapped variable, discharged
    // here, at the call site. For a variable image this is a *symbolic*
    // discharge against the caller's own declared bounds -- the caller's own
    // concrete instantiation is what checks those against a real type
    // (`check_poly_call`'s bound loop), so satisfaction transfers -- and for a
    // concrete image it is the ordinary predicate, run on the spot.
    for (v, bound) in &callee_sig.bounds {
        // A variable no declared input mentions skips bound checking, exactly
        // as the concrete path's own bound loop skips an ungrounded one.
        let Some((_, image)) = mapping.iter().find(|(id, _)| id == v) else {
            continue;
        };
        let var = &callee_sig.ty_var_names[*v as usize];
        let unsatisfied = match (image, bound) {
            (Image::CallerVar(t), _) => (!sig.has_bound(*t, *bound)).then(|| {
                poly_cross_bound_error(
                    ctx,
                    span,
                    name,
                    var,
                    &sig.ty_var_names[*t as usize],
                    *bound,
                    traits,
                )
            }),
            // N1: `is_copy` resolves a struct/enum id by indexing, and a
            // generic instantiation this body's *own* walk minted (`1 Box`)
            // is not in these slices yet -- `check::check` appends it only
            // once `check_poly_body` returns, so the id sits past the end and
            // indexing it panics. Rejected honestly, for the same reason
            // R6's concrete-compound case is: deciding it needs a registry
            // the walk does not hold. Depth one is the whole test -- a decl
            // already in the registries had its fields resolved before this
            // body started, so it cannot reach one minted during it.
            (Image::Concrete(ty), Bound::Copy) if !type_is_registered(*ty, structs, enums) => {
                Some(poly_cross_call_unsupported_error(
                    ctx,
                    span,
                    name,
                    &format!(
                        "discharging `Copy` on the body-local generic instantiation `{}`",
                        ty.name()
                    ),
                ))
            }
            (Image::Concrete(ty), Bound::Copy) => (!is_copy(*ty, structs, enums, arrays))
                .then(|| poly_copy_bound_error(ctx, span, name, var, *ty)),
            // Not resolved here: `compose` grounds every mapping entry --
            // this `Image::Concrete` case and `Image::CallerVar` alike --
            // into a real `ty` before its own `resolve_user_bound` loop runs
            // over `sig.bounds`, so that loop re-derives and checks this
            // exact obligation for every reachable cross-call once the
            // caller is grounded. This walk-time site defers to it.
            //
            // Residual gap, deliberately unenforced (P7.S3s R3): this arm
            // always records `None` and defers to `compose`'s own loop, but
            // `compose` only runs against a caller instantiation that
            // actually exists. A `g` whose body builds its own unimplementing
            // type and hands it to a `Bound::User`-bounded callee (this arm's
            // exact shape) is never flagged if nothing in the program ever
            // instantiates `g` -- no θ to ground the check against, so it
            // never runs. Builds clean today; not this slice's to close.
            (Image::Concrete(_), Bound::User(_)) => None,
        };
        if let Some(err) = unsatisfied {
            return Err(err);
        }
    }
    let n_in = callee_sig.inputs.len();
    let base = stack.len() - n_in;
    // The callee's declared outputs, read back into the *caller's* variable
    // space through the mapping -- the symbolic twin of `apply_subst`.
    let mut outputs = Vec::with_capacity(callee_sig.outputs.len());
    for declared in &callee_sig.outputs {
        outputs.push(poly_cross_output(
            declared, &mapping, callee_sig, name, span, ctx,
        )?);
    }
    cross.calls.push(PolyCrossCall {
        callee: name.to_string(),
        span,
        mapping,
    });
    stack.truncate(base);
    for out in outputs {
        stack.push(PolySlot::new(out));
    }
    Ok(stack)
}

/// P7.S3k (N1): whether `ty`'s own registry entry is one the poly-body walk
/// can already index. A struct and an enum are the only two answers that can
/// be `false`: they are the only things `GenericTypes` mints, and the only two
/// `is_copy` indexes a registry for that a body-local mint can reach (its
/// `Type::Array` arm indexes too, but no array is minted during the walk --
/// there is no mutable array registry here to mint into). `Type::Variant` is
/// deliberately absent: `is_copy` has no arm for it and so never indexes one.
fn type_is_registered(ty: Type, structs: &[StructDecl], enums: &[EnumDecl]) -> bool {
    match ty {
        Type::Struct(id, _) => id.index() < structs.len(),
        Type::Enum(id, _) => id.index() < enums.len(),
        _ => true,
    }
}

/// P7.S3k (R2): relate `callee`'s declared inputs to the caller's operand
/// slots, yielding each callee type variable's image in the caller's world.
/// Total: every shape it cannot represent is a located rejection at the call
/// site, never a deferred one (N1).
fn poly_cross_relate(
    callee_sig: &PolySig,
    callee: &str,
    stack: &[PolySlot],
    span: Span,
    ctx: &Ctx,
    caller_sig: &PolySig,
) -> Result<Vec<(u32, Image)>, String> {
    poly_cross_signature_supported(callee_sig, callee, span, ctx)?;
    let n_in = callee_sig.inputs.len();
    if stack.len() < n_in {
        return Err(underflow_error(ctx, span, callee, n_in, stack.len()));
    }
    let base = stack.len() - n_in;
    let mut mapping = Vec::new();
    for (i, declared) in callee_sig.inputs.iter().enumerate() {
        poly_cross_match(
            declared,
            &stack[base + i].pt,
            &mut mapping,
            callee_sig,
            caller_sig,
            callee,
            span,
            ctx,
        )?;
    }
    Ok(mapping)
}

/// P7.S3k (R2/R6): match one declared callee input against the caller's slot,
/// binding each callee variable it reaches. The recursion is what separates
/// R6's two look-alike cases: a *declared* compound (`&'U`) is decomposed, so
/// a caller passing `&'T` binds `'U` to the bare `'T` and nothing grew; a
/// declared bare `'U` facing a compound operand is the caller having wrapped
/// its own variable, which is growth.
#[allow(clippy::too_many_arguments)]
fn poly_cross_match(
    declared: &PolyType,
    supplied: &PolyType,
    mapping: &mut Vec<(u32, Image)>,
    callee_sig: &PolySig,
    caller_sig: &PolySig,
    callee: &str,
    span: Span,
    ctx: &Ctx,
) -> Result<(), String> {
    let mismatch = || {
        poly_rendered_type_mismatch_error(
            ctx,
            span,
            callee,
            &poly_type_str(declared, callee_sig),
            &poly_type_str(supplied, caller_sig),
        )
    };
    match (declared, supplied) {
        (PolyType::Var(v), _) => {
            let image = match supplied {
                PolyType::Concrete(t) => Image::Concrete(*t),
                PolyType::Var(w) => Image::CallerVar(*w),
                // Not a value at all, so it can fill no declared position:
                // the same rejection the operand-window guard renders for a
                // literal it cannot ground.
                PolyType::QuotLit => {
                    return Err(poly_op_on_variable_error(
                        ctx,
                        span,
                        callee,
                        &PolyType::QuotLit,
                        caller_sig,
                    ))
                }
                PolyType::Quotation(..) => {
                    return Err(poly_cross_call_unsupported_error(
                        ctx,
                        span,
                        callee,
                        "passing a quotation to a polymorphic word",
                    ))
                }
                // R6 fires only when the compound image actually mentions a
                // caller variable (the caller wrapped its own variable
                // before handing it in). A fully concrete compound (`&i64`,
                // built by `&n` on a scalar static, say) mentions none, so
                // it is not growth -- but folding it into `Image::Concrete`
                // would have to mint a fresh `RefId`/`ArrayId`, and nothing
                // in the poly-body walk holds a mutable array/ref registry
                // to do that with (only `structs`/`enums` get that, via
                // `ctx.generics()`). Rejected honestly as unsupported rather
                // than mischaracterized as growth; lifting it needs a
                // `refs`/`arrays` registry cell threaded into `Ctx` the way
                // `generics()` already is -- its own slice.
                _ if !poly_type_mentions_caller_var(supplied) => {
                    return Err(poly_cross_call_unsupported_error(
                        ctx,
                        span,
                        callee,
                        &format!(
                            "passing the concrete compound value `{}`",
                            poly_type_str(supplied, caller_sig)
                        ),
                    ))
                }
                // R6: the caller built a larger type over one of its own
                // variables and handed that in.
                _ => {
                    return Err(poly_growing_cross_call_error(
                        ctx,
                        span,
                        callee,
                        &callee_sig.ty_var_names[*v as usize],
                        &poly_type_str(supplied, caller_sig),
                    ))
                }
            };
            match mapping.iter().find(|(id, _)| id == v) {
                // R2's consistency requirement, the symbolic twin of
                // `unify_poly_input`'s `poly_var_conflict_error`: one callee
                // variable pinned to two different caller images cannot be
                // one type at any instantiation.
                Some((_, prev)) if *prev != image => Err(poly_cross_var_conflict_error(
                    ctx,
                    span,
                    callee,
                    &callee_sig.ty_var_names[*v as usize],
                    &poly_image_str(prev, caller_sig),
                    &poly_image_str(&image, caller_sig),
                )),
                Some(_) => Ok(()),
                None => {
                    mapping.push((*v, image));
                    Ok(())
                }
            }
        }
        (PolyType::Concrete(a), PolyType::Concrete(b)) => match a == b {
            true => Ok(()),
            false => Err(mismatch()),
        },
        (PolyType::Array(de, dl), PolyType::Array(se, sl)) if dl == sl => {
            poly_cross_match(de, se, mapping, callee_sig, caller_sig, callee, span, ctx)
        }
        (PolyType::Ref(de, dm), PolyType::Ref(se, sm)) if dm == sm => {
            poly_cross_match(de, se, mapping, callee_sig, caller_sig, callee, span, ctx)
        }
        // Same header, argument by argument. `name` carries no identity (see
        // `PolyType::Generic`'s own doc), so it takes no part in the compare.
        (
            PolyType::Generic {
                is_enum: de,
                idx: di,
                module: dm,
                args: da,
                ..
            },
            PolyType::Generic {
                is_enum: se,
                idx: si,
                module: sm,
                args: sa,
                ..
            },
        ) if (de, di, dm, da.len()) == (se, si, sm, sa.len()) => {
            for (d, sup) in da.iter().zip(sa) {
                poly_cross_match(d, sup, mapping, callee_sig, caller_sig, callee, span, ctx)?;
            }
            Ok(())
        }
        _ => Err(mismatch()),
    }
}

/// P7.S3k: one declared callee *output*, read back into the caller's variable
/// space. A compound output is rejected for the mirror of R6's reason plus one
/// of its own: a declared compound always mentions a variable (a fully
/// concrete one folds to `Concrete` at parse), so substituting the mapping
/// into it either grows a type over a caller variable or needs the registry
/// interning `apply_subst` does for a *ground* θ and nothing here can do
/// symbolically.
fn poly_cross_output(
    declared: &PolyType,
    mapping: &[(u32, Image)],
    callee_sig: &PolySig,
    callee: &str,
    span: Span,
    ctx: &Ctx,
) -> Result<PolyType, String> {
    match declared {
        PolyType::Concrete(t) => Ok(PolyType::Concrete(*t)),
        PolyType::Var(v) => match mapping.iter().find(|(id, _)| id == v) {
            Some((_, Image::Concrete(t))) => Ok(PolyType::Concrete(*t)),
            Some((_, Image::CallerVar(w))) => Ok(PolyType::Var(*w)),
            // An output variable no declared input pins. The callee's own body
            // check rejects a signature it cannot produce, so this is a
            // backstop rather than a shape source can reach.
            None => Err(poly_cross_call_unsupported_error(
                ctx,
                span,
                callee,
                &format!(
                    "an output type variable (`{}`) that the callee's inputs do not determine",
                    callee_sig.ty_var_names[*v as usize]
                ),
            )),
        },
        _ => Err(poly_cross_call_unsupported_error(
            ctx,
            span,
            callee,
            &format!(
                "returning the compound type `{}` from a polymorphic word",
                poly_type_str(declared, callee_sig)
            ),
        )),
    }
}

/// P7.S3k: the callee signature shapes a symbolic mapping cannot carry, each
/// a located rejection at the call site rather than a shape admitted and
/// mis-lowered later.
///
/// This is the residual of the gap this slice closes, not a restatement of it:
/// it fires for three specific declared shapes, where the deleted
/// `poly_calls_poly_word_error` fired for *every* cross-call.
///
/// - A row (`..s`) has no image kind to map to, and a row-typed `inline`
///   combinator is spliced by `poly_combinator_call` above rather than called.
/// - A quotation parameter has no runtime representation to pass across a
///   real call.
/// - A length variable is a second, separate id space `Image` does not model.
///
/// A `Bound::User` on a mapped variable is *not* rejected here (P7.S3s R2):
/// the symbolic discharge loop in `poly_cross_call` checks it against the
/// caller's own declared bounds for a forwarded variable, and `compose`
/// resolves it against a concrete θ once the caller is grounded -- the same
/// two-stage shape every other bound already goes through.
fn poly_cross_signature_supported(
    callee_sig: &PolySig,
    callee: &str,
    span: Span,
    ctx: &Ctx,
) -> Result<(), String> {
    let unsupported = |what: &str| Err(poly_cross_call_unsupported_error(ctx, span, callee, what));
    if callee_sig.row_in.is_some() || callee_sig.row_out.is_some() {
        return unsupported("calling a row-polymorphic word");
    }
    let slots = || callee_sig.inputs.iter().chain(&callee_sig.outputs);
    if slots().any(poly_input_is_quotation) {
        return unsupported("passing a quotation to a polymorphic word");
    }
    if slots().any(poly_mentions_len_var) {
        return unsupported("a length variable in the callee's signature");
    }
    Ok(())
}

/// R6: whether `pt` mentions a caller type variable at any depth --
/// exactly the predicate that discriminates growth (a compound wrapping a
/// caller variable) from a fully concrete compound (an `&i64`, say) that
/// merely cannot yet be folded to `Image::Concrete` for want of a mutable
/// array/ref registry in the poly-body walk. `QuotLit`/`Quotation` never
/// reach this: the `Var` arm's match handles them ahead of the wildcard
/// that calls this.
fn poly_type_mentions_caller_var(pt: &PolyType) -> bool {
    match pt {
        PolyType::Var(_) => true,
        PolyType::Concrete(_) | PolyType::QuotLit => false,
        PolyType::Array(elem, _) => poly_type_mentions_caller_var(elem),
        PolyType::Ref(referent, _) => poly_type_mentions_caller_var(referent),
        PolyType::OwnedCell(payload) => poly_type_mentions_caller_var(payload),
        PolyType::Generic { args, .. } => args.iter().any(poly_type_mentions_caller_var),
        PolyType::Quotation(ins, outs, ..) => {
            ins.iter().chain(outs).any(poly_type_mentions_caller_var)
        }
    }
}

/// Whether a declared slot mentions a length variable at any depth.
fn poly_mentions_len_var(pt: &PolyType) -> bool {
    match pt {
        PolyType::Array(elem, len) => matches!(len, Len::Var(_)) || poly_mentions_len_var(elem),
        PolyType::Ref(referent, _) => poly_mentions_len_var(referent),
        PolyType::OwnedCell(payload) => poly_mentions_len_var(payload),
        PolyType::Generic { args, .. } => args.iter().any(poly_mentions_len_var),
        PolyType::Quotation(ins, outs, ..) => ins.iter().chain(outs).any(poly_mentions_len_var),
        PolyType::Concrete(_) | PolyType::Var(_) | PolyType::QuotLit => false,
    }
}

/// P7.S3k: one callee variable's image, in the *caller's* spellings -- what
/// the conflict diagnostic names the two sides with.
fn poly_image_str(image: &Image, caller_sig: &PolySig) -> String {
    match image {
        Image::Concrete(t) => t.name().to_string(),
        Image::CallerVar(v) => caller_sig.ty_var_names[*v as usize].clone(),
    }
}

/// P7.S3k (R6): the caller wrapped one of its own type variables in a larger
/// type before handing it to a callee that declared a bare variable. Rejected
/// because a recursive cross-call of this shape composes a structurally larger
/// type at every hop, so its set of instantiations need not be finite and no
/// dedup ever fires.
///
/// Deliberately shape-directed, not cycle-directed: a single, non-recursive
/// wrap would terminate, and is rejected too. That over-rejection buys a
/// check-time structural rule with no cycle detection, and the remedy named
/// below (declare the parameter at the shape the caller actually has) is the
/// accepted form of the same call.
fn poly_growing_cross_call_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    callee_var: &str,
    supplied: &str,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let caller = ctx.rendered_word();
    format!(
        "error: {caller} cannot pass `{supplied}` to `{callee_var}` of the polymorphic word `{callee}` (line {}, col {})\n  a polymorphic call site may pass a type variable only bare: wrapping it in `{supplied}` builds a larger type at every hop of a recursive call, which has no finite set of instantiations\n  declare `{callee}`'s parameter as `{supplied}` so the shape is matched structurally, or call it from a monomorphic word",
        span.line, span.col
    )
}

/// P7.S3k (R3): the callee needs a bound on a variable the caller passes one
/// of its own for, and the caller's signature does not declare it. Located at
/// the call site: the caller's own instantiations are what would otherwise
/// discover this, far away and per instantiation.
fn poly_cross_bound_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    callee_var: &str,
    caller_var: &str,
    bound: Bound,
    traits: &[TraitDecl],
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let caller = ctx.rendered_word();
    let bound = match bound {
        Bound::Copy => "Copy".to_string(),
        Bound::User(id) => traits
            .get(id.index())
            .map_or_else(|| "a user trait".to_string(), |t| t.name.clone()),
    };
    format!(
        "error: `{callee_var}` of `{callee}` requires `{bound}`, which `{caller_var}` in {caller} does not declare (line {}, col {})\n  declare `{caller_var}: {bound}` so every instantiation of {caller} satisfies `{callee}`",
        span.line, span.col
    )
}

/// P7.S3k (R2): one callee variable matched against two different caller
/// images at one call site. The symbolic twin of `poly_var_conflict_error`:
/// the callee declared one variable in both positions, so no instantiation can
/// make the two operands agree.
fn poly_cross_var_conflict_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    callee_var: &str,
    a: &str,
    b: &str,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let caller = ctx.rendered_word();
    format!(
        "error: `{callee}` in {caller} (line {}, col {}) matched `{callee_var}` to both `{a}` and `{b}`",
        span.line, span.col
    )
}

/// P7.S3k: a cross-call whose callee signature is outside the symbolic
/// mapping's reach (`poly_cross_signature_supported`, and the operand/output
/// shapes that need the same interning). Names the specific shape, so it is
/// never mistaken for the whole-feature narrowing it replaced.
fn poly_cross_call_unsupported_error(ctx: &Ctx, span: Span, callee: &str, what: &str) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let caller = ctx.rendered_word();
    format!(
        "error: {caller} cannot call the polymorphic word `{callee}` (line {}, col {})\n  {what} is not yet supported from a polymorphic body\n  call `{callee}` from a monomorphic word instead",
        span.line, span.col
    )
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
#[allow(clippy::too_many_arguments)]
fn poly_eliminator_call(
    gate_id: EnumId,
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
    // P7.S12 (R1.1): `gate_id` is the registry's entry for this call name --
    // a base-family key, since two monomorphs of one generic enum share one
    // registry entry (last write wins). It gates that this call names an
    // eliminator of the right family and supplies the variant count for the
    // underflow diagnostic (identical across every instantiation of the
    // family); the operative `EnumId` this call actually eliminates is read
    // off the scrutinee's own type once found, mirroring `check_eliminator_call`
    // (S3b R5) one path over.
    let enum_decl = &enums[gate_id.index()];
    let enum_name = crate::resolve::demangle_word(generic_surface_name(&enum_decl.name));
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
        return Err(underflow_error(
            ctx,
            span,
            name,
            enum_decl.variants.len() + 1,
            held,
        ));
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
    // P7.S12 (R1.1): the operative `EnumId` is the scrutinee's own -- not
    // `gate_id` -- so two asymmetric monomorphs of one generic enum
    // eliminate independently in the same poly body: the registry's one
    // entry is only consulted by the caller to reach this call at all, never
    // to decide which instantiation it narrows to. A non-enum scrutinee, or
    // one whose enum is not a member of this call name's base family, is the
    // same `type_mismatch_error` as before.
    let id = match &scrutinee.pt {
        PolyType::Concrete(Type::Enum(found, _))
            if generic_surface_name(&enums[found.index()].name)
                == generic_surface_name(&enum_decl.name) =>
        {
            *found
        }
        // P7 slice 3c (R1.4): a slice reaches the `Concrete(_)` reference arm
        // below through the widened `is_ref()`, but the advice there ("pass
        // the owned `Enum` instead") names nothing real for a view over a
        // buffer. It gets the plain mismatch instead -- the same message the
        // concrete path already gives a slice scrutinee.
        PolyType::Concrete(t) if !t.is_ref() || matches!(t, Type::Slice(..)) => {
            return Err(type_mismatch_error(
                ctx,
                span,
                name,
                Type::Enum(gate_id, enum_decl.name_static),
                *t,
            ));
        }
        // A *reference* scrutinee is the concrete path's decision 6, and it
        // buys nothing here: reading a field out of the narrowed variant it
        // would hand each arm needs the projection accessors a generic body
        // does not have yet (P7 slice 1), so every arm it could reach is
        // already unwritable. Located rather than silently narrowed to an
        // owning scrutinee, which would let an arm consume a borrowed enum.
        PolyType::Ref(..) | PolyType::Concrete(_) => {
            return Err(poly_reference_scrutinee_error(ctx, span, name, enum_name));
        }
        // OQ2: an abstract scrutinee is a `'T` that is *some* enum, which is
        // not constructible without an enum-kind bound (P7.S3d).
        _ => {
            return Err(poly_abstract_enum_scrutinee_error(
                ctx,
                span,
                name,
                &poly_type_str(&scrutinee.pt, sig),
            ));
        }
    };
    // R1.4: the family id (`gate_id`) locates and gates; the recorded id
    // (`id`, read off the scrutinee) dispatches. Everything below --
    // exhaustiveness, arm narrowing, arm walk -- resolves against the
    // scrutinee's own monomorph, never the gate.
    let enum_decl = &enums[id.index()];
    // P7.S12 (R1.2): record this eliminator call site so `check_poly_call`
    // can ground it against a concrete θ later. At this phase the scrutinee
    // is always already concrete (a generic scrutinee is P7.S12 R5, phase 3),
    // so the recorded shape is trivially ground already -- `apply_subst`'s
    // `Concrete` arm is a no-op -- but it still needs the same map entry so
    // lowering does not have to special-case "resolved at check time, not θ".
    tctx.enum_sites.push((
        span,
        PolyType::Concrete(Type::Enum(id, enum_decl.name_static)),
    ));

    // Step 3: exhaustiveness and duplication, in written source order and
    // before any arm body is checked.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut variant_indices = Vec::with_capacity(arms.len());
    for (quot, tag) in &arms {
        let literal_span = scope.quotation(*quot).span;
        let Some(vi) = enum_decl
            .variants
            .iter()
            .position(|v| generic_surface_name(&v.name) == tag.name)
        else {
            return Err(eliminator_unknown_variant_error(
                ctx,
                literal_span,
                name,
                &tag.name,
                enum_name,
            ));
        };
        if !seen.insert(generic_surface_name(&enum_decl.variants[vi].name)) {
            return Err(eliminator_duplicate_arm_error(
                ctx,
                literal_span,
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
            let narrowed = variant_type(enums, id, *vi);
            let mut input = row.clone();
            input.push(PolySlot::new(PolyType::Concrete(narrowed)));
            PolyArm {
                quot: *quot,
                input,
                declared_inputs: vec![narrowed],
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
    declared_inputs: Vec<Type>,
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
        if !is_inline {
            return Err(ordinary_literal_at_inline_param_error(
                ctx,
                literal_span,
                name,
                crate::ast::inline_quotation_type(arm.declared_inputs, vec![]),
            ));
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
        // A `Type::Variant` may not leave the call. Every type-directed
        // predicate outside the eliminator is written over `Type::Enum`, so
        // `is_copy` reads an escaped variant as trivially `Copy` and a later
        // `dup` double-drops a linear payload.
        for slot in &exit {
            let escaping = match &slot.pt {
                PolyType::Concrete(t) => Some(*t),
                PolyType::Ref(referent, _) => match referent.as_ref() {
                    PolyType::Concrete(t) => Some(*t),
                    _ => None,
                },
                _ => None,
            };
            if let Some(escaping @ Type::Variant(..)) = escaping {
                return Err(eliminator_variant_escape_error(
                    ctx,
                    literal_span,
                    name,
                    escaping,
                ));
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
            declared_inputs: decl.ins,
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

/// P7 slice 3a (R3): the generic header `called` names as a constructor --
/// a struct whose bare name is `called`, or an enum with a variant of that
/// name -- searched over every generic header this module has declared
/// (not scoped to the enclosing word's own signature): the *identity* of
/// what `called` constructs does not depend on whether this particular word
/// happens to declare a matching output, only the argument *values* do (see
/// `poly_construction_fallback`). A module's own headers are preferred over
/// an imported one of the same bare name, mirroring type-name resolution
/// itself; ties beyond that take the first declared.
fn poly_construction_header(
    generics: &GenericTypes,
    called: &str,
    module: u32,
) -> Option<(bool, usize, usize)> {
    let enum_hit = generics.enums.iter().enumerate().find_map(|(idx, d)| {
        d.variants
            .iter()
            .position(|v| v.name == called)
            .map(|vi| (idx, vi, d.module == module))
    });
    let struct_hit = generics
        .structs
        .iter()
        .position(|d| d.name == called)
        .map(|idx| (idx, 0usize, generics.structs[idx].module == module));
    match (enum_hit, struct_hit) {
        (Some((idx, vi, true)), _) => Some((true, idx, vi)),
        (_, Some((idx, _, true))) => Some((false, idx, 0)),
        (Some((idx, vi, false)), _) => Some((true, idx, vi)),
        (_, Some((idx, _, false))) => Some((false, idx, 0)),
        (None, None) => None,
    }
}

/// P7 slice 3a (R3): the enclosing word's own declared *output* naming this
/// exact generic header, if any -- the phantom-argument fallback source
/// (see the module doc) and the module identity a fresh instantiation is
/// minted under. Absent when this word's output does not name the header at
/// all (a value constructed and consumed entirely within the body, never
/// returned): the naming-site module then falls back to the enclosing word's
/// own module (`ctx.module()`), and every argument must come from the
/// operands alone.
fn poly_construction_fallback(
    sig: &PolySig,
    is_enum: bool,
    idx: usize,
) -> Option<(u32, &[PolyType], &'static str)> {
    sig.outputs.iter().find_map(|pty| match pty {
        PolyType::Generic {
            is_enum: oe,
            idx: oidx,
            module,
            args,
            name,
        } if *oe == is_enum && *oidx as usize == idx => Some((*module, args.as_slice(), *name)),
        _ => None,
    })
}

/// P7 slice 3a (R3): bind one constructor payload field's declared `PolyType`
/// against the operand `PolyType` on the stack, recording the header
/// variable it determines. `substitute_generic_field`'s own doc: a generic
/// `type:` field is always exactly one of these two shapes.
fn poly_bind_construction_arg(
    field_pty: &PolyType,
    operand: &PolyType,
    args: &mut [Option<PolyType>],
    sig: &PolySig,
    ctx: &Ctx,
    span: Span,
    name: &str,
) -> Result<(), String> {
    match field_pty {
        PolyType::Var(v) => {
            let slot = &mut args[*v as usize];
            match slot {
                Some(existing) if existing != operand => Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(existing, sig),
                    &poly_type_str(operand, sig),
                )),
                _ => {
                    *slot = Some(operand.clone());
                    Ok(())
                }
            }
        }
        PolyType::Concrete(t) => {
            if operand == &PolyType::Concrete(*t) {
                Ok(())
            } else {
                Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(field_pty, sig),
                    &poly_type_str(operand, sig),
                ))
            }
        }
        other => unreachable!("a generic `type:` field is never {other:?}"),
    }
}

/// P7 slice 3a (R3): whether `name` already resolves through the ordinary
/// concrete `env` for these exact operand types -- the already-working
/// concrete case (a fully-concrete generic instantiation minted at parse
/// time, R1's fold), which must reach the pre-existing dispatch below
/// unaffected rather than this arm's own resolution (which has no source
/// for a phantom argument once the enclosing output has already folded to
/// `Concrete` and so carries no `PolyType::Generic` to fall back on).
fn poly_env_exact_match(
    env: &HashMap<String, Vec<Overload>>,
    name: &str,
    stack: &[PolySlot],
) -> bool {
    env.get(name).is_some_and(|candidates| {
        candidates.iter().any(|o| {
            let n = o.sig.inputs.len();
            stack.len() >= n
                && stack[stack.len() - n..]
                    .iter()
                    .zip(&o.sig.inputs)
                    .all(|(s, inp)| matches!(&s.pt, PolyType::Concrete(t) if t == inp))
        })
    })
}

/// P7 slice 3a (R3): a call to `name` naming a generic struct's constructor
/// or a generic enum's variant, in a polymorphic body. `Ok(None)` if `name`
/// names no generic header at all, or if an exact concrete `env` candidate
/// already covers this call (`poly_env_exact_match`) -- either way the
/// caller's existing dispatch handles it unchanged.
#[allow(clippy::too_many_arguments)]
fn poly_construct_generic(
    name: &str,
    span: Span,
    stack: &mut Vec<PolySlot>,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    tctx: &mut TraitCtx,
) -> Result<Option<Vec<PolySlot>>, String> {
    let Some(cell) = ctx.generics() else {
        return Ok(None);
    };
    if poly_env_exact_match(env, name, stack) {
        return Ok(None);
    }
    let generics = cell.borrow();
    let Some((is_enum, idx, variant)) = poly_construction_header(&generics, name, ctx.module())
    else {
        return Ok(None);
    };
    let fallback = poly_construction_fallback(sig, is_enum, idx);
    // Leaked regardless of whether a fallback exists: operand-only
    // determination can still leave a symbolic result (a header variable
    // bound to the enclosing word's own `'T`, never grounded to a `Type`
    // here), which needs this name too, not only the output-fallback path.
    let header_name: &'static str = if is_enum {
        Box::leak(generics.enums[idx].name.clone().into_boxed_str())
    } else {
        Box::leak(generics.structs[idx].name.clone().into_boxed_str())
    };
    let (module, output_args) = match fallback {
        Some((module, args, _)) => (module, args.to_vec()),
        None => (ctx.module(), Vec::new()),
    };
    let field_ptys: Vec<PolyType> = if is_enum {
        generics.enums[idx].variants[variant]
            .fields
            .iter()
            .map(|(_, p)| p.clone())
            .collect()
    } else {
        generics.structs[idx]
            .fields
            .iter()
            .map(|(_, p)| p.clone())
            .collect()
    };
    let arity = if is_enum {
        generics.enums[idx].ty_var_names.len()
    } else {
        generics.structs[idx].ty_var_names.len()
    };
    drop(generics);

    let n = stack.len();
    if n < field_ptys.len() {
        return Err(underflow_error(ctx, span, name, field_ptys.len(), n));
    }
    let base = n - field_ptys.len();
    let mut args: Vec<Option<PolyType>> = vec![None; arity];
    for (i, field_pty) in field_ptys.iter().enumerate() {
        let operand = stack[base + i].pt.clone();
        poly_bind_construction_arg(field_pty, &operand, &mut args, sig, ctx, span, name)?;
    }
    // R3: an argument the operands leave undetermined (a phantom for this
    // variant, e.g. `Err`'s payload never mentions `Result`'s `'T`) is taken
    // from the enclosing word's own declared output naming this header, when
    // there is one -- sound because it is phantom for the value just
    // constructed (`substitute_generic_field` only ever substitutes a field
    // that actually exists), and the *determined* arguments still get
    // unified against this same declared output at word exit
    // (`unify_poly_input`'s `Generic` arm), so a wrong inferred position
    // surfaces there, located, rather than silently miscompiling.
    for (v, slot) in args.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = output_args.get(v).cloned();
        }
    }
    let mut resolved = Vec::with_capacity(arity);
    for (v, slot) in args.into_iter().enumerate() {
        match slot {
            Some(pt) => resolved.push(pt),
            None => {
                let generics = cell.borrow();
                let var_name = if is_enum {
                    generics.enums[idx].ty_var_names[v].clone()
                } else {
                    generics.structs[idx].ty_var_names[v].clone()
                };
                return Err(poly_generic_constructor_undetermined_error(
                    ctx, span, name, &var_name,
                ));
            }
        }
    }

    stack.truncate(base);
    let all_concrete: Option<Vec<Type>> = resolved
        .iter()
        .map(|p| match p {
            PolyType::Concrete(t) => Some(*t),
            _ => None,
        })
        .collect();
    let result_pt = if let Some(concrete_args) = all_concrete {
        // P7.S3n (R5): the live cell/ref registries, not the `&[]`/`&[]`
        // throwaway this used to build -- an instantiation whose field wraps
        // the header's own variable interns as it grounds, and its rendered
        // name reads back a cell- or ref-payload argument.
        let regs = crate::ast::MutRegistries {
            structs,
            enums,
            arrays,
            cells,
            refs,
        };
        let mut g = cell.borrow_mut();
        let ty = if is_enum {
            g.instantiate_enum(idx, &concrete_args, module, regs)
        } else {
            g.instantiate_struct(idx, &concrete_args, module, regs)
        };
        PolyType::Concrete(ty)
    } else {
        PolyType::Generic {
            is_enum,
            idx: idx as u32,
            module,
            args: resolved,
            name: header_name,
        }
    };
    // P7.S12 (R1.2/R6.1): record this generated-enum-word call site so
    // `check_poly_call` can ground it against a concrete θ later -- a struct
    // constructor has no per-monomorph identity for lowering to lose (a
    // struct is laid out by field order alone), so only the enum case is
    // recorded.
    if is_enum {
        // R1.5: a combinator's own construction of a still-ungrounded enum
        // varies per splice, which the `Span`-keyed record cannot represent.
        if tctx.is_combinator_splice && matches!(result_pt, PolyType::Generic { .. }) {
            return Err(poly_combinator_generic_enum_construction_error(
                ctx,
                span,
                name,
                header_name,
            ));
        }
        tctx.enum_sites.push((span, result_pt.clone()));
    }
    stack.push(PolySlot::new(result_pt));
    Ok(Some(std::mem::take(stack)))
}

/// P7 slice 3a (R5.2): a generic constructor call whose header type variable
/// is determined by neither its operands nor the enclosing word's declared
/// output -- a located error, not a latent failure at monomorphization.
fn poly_generic_constructor_undetermined_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    var: &str,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{op}` in {where_} (line {}) leaves the type variable `{var}` undetermined\n  neither the operands nor the declared output fix `{var}`; a generic constructor needs every argument determined",
        span.line
    )
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
                Len::Concrete(count) => count,
                // D6: a fully generic-length array's element cannot be
                // statically bounds-checked; a dependent-bounds problem this
                // slice defers (its own slice, `'N`-length element access).
                Len::Var(v) => {
                    return Err(poly_generic_length_index_error(
                        ctx,
                        span,
                        &sig.len_var_names[v as usize],
                    ));
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
/// shuffle leaves it `None`.
#[allow(clippy::too_many_arguments)]
fn check_poly_array_index(
    index_pt: &PolyType,
    index_lit: Option<i64>,
    count: u32,
    ctx: &Ctx,
    span: Span,
    op: &str,
    sig: &PolySig,
) -> Result<(), String> {
    match index_pt {
        PolyType::Concrete(Type::Usize) => Ok(()),
        PolyType::Concrete(Type::I64) => match index_lit {
            Some(idx) if idx >= 0 && idx < i64::from(count) => Ok(()),
            Some(idx) => Err(array_index_out_of_range_error(ctx, span, count, idx)),
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

/// R5/R6/R14: a call to a polymorphic word from a concrete body. Unifies the
/// word's `PolySig` against the concrete top of stack (deepest-first),
/// building the ground substitution `θ`, checks `θ` against the declared
/// bounds (X5/X6), records the per-call-site `CallInst` for lowering (R14),
/// and pushes the substituted concrete outputs. The row variable is a pure
/// pass-through: the stack beneath the fixed inputs is untouched, so `θ`
/// carries no row types and the word's ABI never sees the caller's deeper
/// stack (S2 rejected the carried runtime stack).
/// R5/R14 (Slice 8a): the outcome of resolving a name with more than one
/// polymorphic candidate. Kept distinct from a plain `None` so the caller can
/// still raise R9p's specific rejection (a quotation operand disqualifies
/// every candidate the same way, since binding `'T` to the placeholder would
/// monomorphize a call over a phantom) rather than a generic no-match message
/// that would misdescribe the reason.
pub(super) enum PolyOverloadMiss {
    Quotation,
    NoMatch,
}

/// The first candidate among `candidates` whose declared inputs unify against
/// the tail of `stack`, tried in declaration order -- the same shape as
/// `resolve_overload`'s exact-match resolution for concrete words, adapted to
/// unification since a poly input may be a type variable rather than a fixed
/// `Type`. Only reached with 2+ candidates; the caller keeps the exact
/// existing single-candidate path (and its position-specific diagnostics)
/// untouched. Bounds (R6) are checked only against the chosen candidate by
/// the caller, matching the single-candidate path: they gate a resolved
/// instantiation, not resolution itself.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_poly_overload(
    candidates: &[PolySig],
    stack: &[Slot],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> Result<PolySig, PolyOverloadMiss> {
    let mut saw_quotation = false;
    for sig in candidates {
        let n_in = sig.inputs.len();
        if stack.len() < n_in {
            continue;
        }
        let base = stack.len() - n_in;
        if stack[base..].iter().any(|s| s.quot.is_some()) {
            saw_quotation = true;
            continue;
        }
        if poly_sig_unifies(sig, stack, name, span, ctx, arrays, cells, refs) {
            return Ok(sig.clone());
        }
    }
    Err(if saw_quotation {
        PolyOverloadMiss::Quotation
    } else {
        PolyOverloadMiss::NoMatch
    })
}

/// R5/R14: a named call matching no polymorphic candidate's declared inputs,
/// listing each candidate's whole signature (`poly_sig_str`) the way
/// `no_overload_matches_error` lists concrete candidates' input shapes.
pub(super) fn no_poly_overload_matches_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    candidates: &[PolySig],
) -> String {
    let demangled = crate::resolve::demangle_call(name);
    let mut shapes: Vec<String> = candidates
        .iter()
        .map(|sig| poly_sig_str(name, sig))
        .collect();
    shapes.sort();
    let listed = shapes
        .iter()
        .map(|s| format!("\n  candidate: {s}"))
        .collect::<String>();
    format!(
        "error: no overload of `{demangled}` in {wname} (line {}) accepts these operands{listed}",
        span.line,
        wname = ctx.rendered_word()
    )
}

/// P7.S3o recon: two splices of the same poly combinator at two different
/// concrete types insert `CallInst`s for the same inner poly call at the
/// *same* body span — last write wins, and lowering reads only the survivor,
/// so the losing splice dispatches to the wrong monomorph. This turns that
/// collision into a located error, suggesting the enclosing combinator be
/// made non-inline as a workaround (the shape S3s ships `mymax`/`mymax3` in
/// precisely to avoid this hole).
pub(super) fn splice_collision_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let demangled = crate::resolve::demangle_call(name);
    format!(
            "error: `{demangled}` in {} (line {}) is instantiated at two different types from the same source span inside an inline combinator splice; make the enclosing combinator non-inline to avoid the collision",
            ctx.rendered_word(), span.line
        )
}

/// Whether `sig`'s declared inputs unify against the tail of `stack`,
/// without committing any successful bindings past this call: a fresh
/// `Subst` per attempt, discarded either way. The shared predicate behind
/// resolving an overloaded polymorphic word (`resolve_poly_overload`) and an
/// overloaded polymorphic combinator (`resolve_combinator_overload`) alike --
/// unlike `resolve_poly_overload`'s own caller, a combinator's declared
/// inputs legitimately include a quotation type, so this makes no R9p
/// judgement about one; that stays the caller's decision.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_sig_unifies(
    sig: &PolySig,
    stack: &[Slot],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> bool {
    let n_in = sig.inputs.len();
    if stack.len() < n_in {
        return false;
    }
    let base = stack.len() - n_in;
    let mut subst = Subst::default();
    (0..n_in).all(|i| {
        unify_poly_input(
            sig,
            &sig.inputs[i],
            stack[base + i].ty,
            name,
            span,
            ctx,
            arrays,
            cells,
            refs,
            &mut subst,
            &[],
        )
        .is_ok()
    })
}

/// Whether `sig`'s declared inputs *could* match the tail of `stack`, for
/// selecting among 2+ poly combinator candidates only. A declared quotation
/// position (`poly_input_is_quotation`) contributes no constraint and is
/// skipped: a stack slot standing for a quotation carries a placeholder `ty`
/// (`Slot::quot`'s own doc -- "no user op accepts" it) rather than the
/// literal's real effect until `inline_combinator` materializes it, and
/// checking that for real means running the literal's body, which this
/// selection step must not do speculatively once per candidate (unlike a
/// concrete type or a bare `'T`, a quotation's real effect is not known
/// without side-effecting work). Every other declared position unifies for
/// real. Whichever candidate this selects still has every declared position,
/// quotation included, validated for real exactly once by the existing
/// single-candidate path this only decides which candidate reaches.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_sig_could_match(
    sig: &PolySig,
    stack: &[Slot],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    impls: &[ImplDecl],
    traits: &[TraitDecl],
) -> bool {
    let n_in = sig.inputs.len();
    if stack.len() < n_in {
        return false;
    }
    let base = stack.len() - n_in;
    let mut subst = Subst::default();
    (0..n_in).all(|i| {
        if poly_input_is_quotation(&sig.inputs[i]) {
            return true;
        }
        // Slice 10c: an `Ord`-bounded variable admits only a type with its own
        // `impl: Ord` (P7.S3s: an ordinary `impl:` registry lookup, `Ord` no
        // longer being a reserved numeric-only predicate), and the bound is
        // what keeps `core::cmp`'s `: lt ['T: Copy Ord] ( 'T 'T -- bool )` from
        // claiming a call site meant for a user's `: lt ( Vec2 Vec2 -- bool )`.
        // Unification alone binds `'T` to anything at all, so without this the
        // library word swallows every operand type. `ord_trait_id` resolves
        // *this* `sig`'s own `Ord` bound (review fix: not a whole-program
        // name search, which fails open under a module-local `trait: Ord`).
        if let PolyType::Var(v) = &sig.inputs[i] {
            if let Some(ord) = ord_trait_id(sig, *v, traits) {
                let generics = ctx.generics().map(|c| c.borrow());
                if !impls.iter().any(|imp| {
                    imp.trait_id == ord
                        && match_impl_target(
                            &imp.target.pattern,
                            stack[base + i].ty,
                            arrays,
                            cells,
                            refs,
                            generics.as_deref(),
                        )
                        .is_some()
                }) {
                    return false;
                }
            }
        }
        unify_poly_input(
            sig,
            &sig.inputs[i],
            stack[base + i].ty,
            name,
            span,
            ctx,
            arrays,
            cells,
            refs,
            &mut subst,
            &[],
        )
        .is_ok()
    })
}

/// The first candidate among `candidates` whose declared shape matches the
/// tail of `stack`, tried in declaration order: exact type match for a mono
/// candidate (the same exact-match philosophy `resolve_overload` uses for an
/// ordinary word, R2), a could-match probe for a poly one. `inline_combinator`
/// already branches the same way for the sole-candidate case; this only
/// decides which candidate reaches that branch. A declared quotation
/// position never distinguishes a mono candidate either, for the identical
/// placeholder-`ty` reason `poly_sig_could_match` skips one -- treated as a
/// wildcard on both branches. Only reached with 2+ candidates sharing one
/// name.
///
/// Accepted narrowing: two candidates identical in every *non*-quotation
/// position, differing only in a declared quotation's effect, are
/// indistinguishable here; the first declared wins, the same trade
/// `resolve_poly_overload` already accepts on an ambiguous unification.
pub(super) fn resolve_combinator_overload<'a>(
    candidates: &[Combinator<'a>],
    stack: &[Slot],
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> Option<Combinator<'a>> {
    for comb in candidates {
        let matched = match comb.word.poly.as_ref() {
            Some(sig) => poly_sig_could_match(
                sig,
                stack,
                comb.word.name.as_str(),
                span,
                ctx,
                arrays,
                cells,
                refs,
                // A combinator's own type variable may carry a `Bound::User`
                // (an `inline` trait member like `cmp`), but
                // `resolve_combinator_overload` only matches the stack shape
                // against the signature here; the bound is resolved later at
                // the call site via the per-splice trait-call map, so there
                // is nothing for an `impl:` registry lookup to filter here.
                &[],
                &[],
            ),
            None => {
                let inputs: Vec<Type> = comb.word.effect.inputs.iter().map(|s| s.ty).collect();
                let n = inputs.len();
                stack.len() >= n
                    && stack[stack.len() - n..]
                        .iter()
                        .zip(inputs.iter())
                        .all(|(s, want)| {
                            crate::ast::is_quotation_type(*want).is_some() || s.ty == *want
                        })
            }
        };
        if matched {
            return Some(*comb);
        }
    }
    None
}

/// R18: a call to an overloaded combinator name matching no candidate's
/// declared shape, listing each candidate the way an ordinary overload's miss
/// does -- a rendered signature for a poly candidate, input types for a mono
/// one.
pub(super) fn no_combinator_overload_matches_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    candidates: &[Combinator],
) -> String {
    let demangled = crate::resolve::demangle_call(name);
    let mut shapes: Vec<String> = candidates
        .iter()
        .map(|comb| match comb.word.poly.as_ref() {
            Some(sig) => poly_sig_str(name, sig),
            None => {
                let inputs: Vec<String> = comb
                    .word
                    .effect
                    .inputs
                    .iter()
                    .map(|s| format!("`{}`", s.ty))
                    .collect();
                match inputs.is_empty() {
                    true => "no operands".to_string(),
                    false => inputs.join(" "),
                }
            }
        })
        .collect();
    shapes.sort();
    let listed = shapes
        .iter()
        .map(|s| format!("\n  candidate: {s}"))
        .collect::<String>();
    format!(
        "error: no overload of `{demangled}` in {wname} (line {}) accepts these operands{listed}",
        span.line,
        wname = ctx.rendered_word()
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_poly_call(
    name: &str,
    span: Span,
    type_args: &[Type],
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
        for (v, ty) in type_args.iter().enumerate() {
            subst.ty.push((v as u32, *ty));
            seeded.push(v as u32);
        }
    }
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
    // `'N` (id 0) -- but not *divergently*: R4 gives it no seed path, so its
    // order is a function of the callee's signature and every minting path
    // agrees. Sorted anyway because `Subst`'s derived `Eq`, which
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
    // Review fix (P7 slice 1): a polymorphic word consumes its operands
    // exactly as a concrete one does, so it needs the same guard against
    // moving a place a live projection still reaches -- `'T` binds to the
    // receiver's struct type as readily as a declared `Point` does.
    for i in base..stack.len() {
        let origin = consumed_place_conflict(stack[i], &stack[..i], scope, prov, live, at)
            .or_else(|| consumed_place_conflict(stack[i], &stack[i + 1..], scope, prov, live, at));
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
            stack.truncate(base);
            for ty in outputs {
                stack.push(Slot::computed(ty));
            }
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
    stack.truncate(base);
    for ty in outputs {
        stack.push(Slot::computed(ty));
    }
    Ok(std::mem::take(stack))
}

/// P7.S3k (R4/N3): the monomorphs reachable only *through* a generic body's
/// call to another generic word, discovered once every concrete call site has
/// been recorded.
///
/// A concrete caller's `CallInst` says that `(w, θ_w)` exists; `w`'s own body
/// may call another generic word, and that call was recorded only symbolically
/// (`Module::poly_cross_calls`) because `w`'s variables were still rigid when
/// its body was walked. Composing a record's mapping with a caller θ is what
/// grounds it, so discovery belongs here, where the concrete θs live and where
/// `apply_subst`'s registry interning is still reachable -- not at lowering,
/// which only ever looks an already-interned shape up.
///
/// Terminates with no depth cap (N3): R6 rejects a compound image over a
/// caller variable at the call site, so a composed θ assigns each callee
/// variable either a fixed concrete type or the caller θ's image of one caller
/// variable -- never a constructor applied to one. Every reachable θ therefore
/// draws its types from the finite pool the seed instantiations introduced, the
/// reachable `(word, θ)` set is finite, and the symbol dedup below reaches a
/// fixpoint. A mutual `g <-> h` cycle revisits `(g, θ)` at the *same* θ and
/// stops.
///
/// `word_symbols` and `trait_obligations` (P7.S3s R2) are what `compose`
/// needs to resolve a composed callee's own `Bound::User` obligations against
/// a concrete θ, mirroring `check_poly_call`'s bound loop -- passed in rather
/// than read off `module`, since the `TraitResolveCtx` built from them below
/// must not borrow `module` while the destructure above still needs `&mut`
/// access to it.
#[allow(clippy::too_many_arguments)]
pub(super) fn discover_transitive_instantiations(
    module: &mut Module,
    insts: &mut HashMap<Span, CallInst>,
    splice_records: &mut HashMap<(u32, Span), CallInst>,
    word_symbols: &[String],
    trait_obligations: &[WordObligations],
    word_enum_sites: &[WordEnumSites],
    impl_monos: Vec<(String, Subst)>,
    generics: Option<&RefCell<GenericTypes>>,
) -> Result<Vec<CallInst>, String> {
    // P7.S4 (R6): build CallInsts for generic-impl member-word monomorphs
    // before the fixpoint, so their bodies' poly cross-calls get composed.
    // Even when poly_cross_calls is empty (no generic-calls-generic), the
    // member-word monomorphs themselves still need to be emitted by lowering.
    let Module {
        words,
        structs,
        enums,
        arrays,
        refs,
        owned_cells,
        statics,
        modules,
        poly_cross_calls,
        traits,
        impls,
        ..
    } = module;
    // Built here rather than accepted from the caller (R2): a caller-side
    // `TraitResolveCtx` would borrow `module` immutably for the whole call,
    // and this function still needs `&mut module` through the destructure
    // above.
    let tr = TraitResolveCtx {
        traits,
        impls,
        word_symbols,
        recorded: trait_obligations,
        enum_sites_recorded: word_enum_sites,
    };
    let ground = CrossGround {
        words,
        structs,
        enums,
        statics,
        modules: Some(modules),
        records: poly_cross_calls,
        tr,
        generics,
    };
    // P7.S4 (R6): build CallInsts for each generic-impl member-word monomorph.
    let mut extra_seeds: Vec<CallInst> = Vec::new();
    let mut seed_discovered: Vec<(String, Subst)> = Vec::new();
    for (word_name, subst) in &impl_monos {
        let symbol = instantiation_symbol(word_name, subst);
        if extra_seeds.iter().any(|s| s.symbol == symbol) {
            continue;
        }
        if let Some(seed) = ground.impl_mono_seed(
            word_name,
            subst,
            arrays,
            refs,
            owned_cells,
            &mut seed_discovered,
        )? {
            extra_seeds.push(seed);
        }
    }
    if poly_cross_calls.is_empty() && extra_seeds.is_empty() {
        // Still need to intern bundles for splice_records before returning.
        for inst in splice_records.values_mut() {
            if inst.out_arity >= 2 {
                inst.bundle = Some(intern_bundle_struct(structs, &inst.output_types));
            }
        }
        return Ok(Vec::new());
    }
    let mut transitive = ground.fixpoint(
        insts,
        splice_records,
        extra_seeds,
        arrays,
        refs,
        owned_cells,
        seed_discovered,
    )?;
    // R8/R14, the composed twin of `check`'s own `out_arity >= 2` loop: a
    // composed callee returning a bundle is laid out like any other. Run as a
    // post-pass because interning needs `&mut structs`, which the grounding
    // above holds immutably through `Ctx`. `intern_bundle_struct` dedups by
    // output tuple, so the flat entry and the routing copy of one `(h, θ_h)`
    // land on the same `StructId`.
    for inst in transitive.iter_mut() {
        intern_composed_bundles(structs, inst);
    }
    for inst in insts.values_mut() {
        intern_composed_bundles(structs, inst);
    }
    for inst in splice_records.values_mut() {
        intern_composed_bundles(structs, inst);
    }
    // P7.S12 (R1.2a): flush whatever `compose`/`impl_mono_seed` minted while
    // grounding `enum_words`, after the fixpoint (so a mint mid-fixpoint
    // never needs to be visible to a sibling grounding step within it, which
    // this slice does not attempt) and before layout reads `module.enums`.
    if let Some(cell) = generics {
        let mut g = cell.borrow_mut();
        g.flush_structs_into(structs);
        g.flush_enums_into(enums);
    }
    Ok(transitive)
}

/// The bundle of a composed `CallInst` and of every cross-call its body
/// routes. The nested copies get one too: `lower_poly_call` reads `bundle`
/// off the *call site's* record to know what shape came back.
fn intern_composed_bundles(structs: &mut Vec<StructDecl>, inst: &mut CallInst) {
    if inst.out_arity >= 2 {
        inst.bundle = Some(intern_bundle_struct(structs, &inst.output_types));
    }
    for callee in inst.poly_calls.values_mut() {
        if callee.out_arity >= 2 {
            callee.bundle = Some(intern_bundle_struct(structs, &callee.output_types));
        }
    }
}

/// P7.S3k (R4): everything the composition step reads. Bundled because it is
/// threaded through a worklist loop that also carries the two registries
/// grounding *writes* to (`arrays`/`refs`), which cannot ride here.
struct CrossGround<'a> {
    words: &'a [WordDef],
    structs: &'a [StructDecl],
    enums: &'a [EnumDecl],
    statics: &'a [StaticDecl],
    modules: Option<&'a [ModuleInfo]>,
    records: &'a HashMap<String, Vec<PolyCrossCall>>,
    tr: TraitResolveCtx<'a>,
    /// P7.S12 (R1.2a): the live instantiator, threaded here rather than left
    /// `None` as `word_ctx`'s two call sites below used to. `compose` grounds
    /// a cross-called generic word's own body spans (`enum_words`), which
    /// needs the same find-or-mint door `check_poly_call` uses -- there is no
    /// find-only alternative (`apply_subst`'s `Generic` arm).
    generics: Option<&'a RefCell<GenericTypes>>,
}

impl CrossGround<'_> {
    /// Seed from the concrete instantiations, then compose until no new
    /// `(word, θ)` appears. Each taken instantiation's routing map is written
    /// back onto it, so a seed learns what its own body calls and a composed
    /// entry carries the map its body will be lowered against.
    #[allow(clippy::too_many_arguments)]
    fn fixpoint(
        &self,
        insts: &mut HashMap<Span, CallInst>,
        splice_records: &mut HashMap<(u32, Span), CallInst>,
        extra_seeds: Vec<CallInst>,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        initial_discovered: Vec<(String, Subst)>,
    ) -> Result<Vec<CallInst>, String> {
        let mut seen: HashSet<String> = insts.values().map(|i| i.symbol.clone()).collect();
        let mut frontier: Vec<CallInst> = Vec::new();
        // P7.S4 (R6, review fix): a composed cross-call's own `Bound::User`
        // obligation can resolve to a *generic*-impl member word, discovering
        // a monomorph mid-composition rather than at a top-level call site
        // (e.g. `outer` calls `inner` calls `show`, and only `inner`'s own
        // bound resolves the generic `impl:`). `cross_calls_of` drains every
        // such discovery into this accumulator so the loop below can seed it
        // too -- otherwise the member word's body is never emitted and
        // lowering panics on the dangling symbol.
        let mut discovered: Vec<(String, Subst)> = initial_discovered;
        for inst in insts.values_mut() {
            inst.poly_calls = self.cross_calls_of(inst, arrays, refs, cells, &mut discovered)?;
            enqueue_new(&inst.poly_calls, &mut seen, &mut frontier);
        }
        // P7.S3o (R1/R2): splice-derived CallInsts are concrete instantiations
        // of poly words called from combinator bodies. They seed the fixpoint
        // just like ordinary instantiations, so a poly chain (combinator →
        // p1 → p2) discovers p2 through p1's cross-calls.
        for inst in splice_records.values_mut() {
            inst.poly_calls = self.cross_calls_of(inst, arrays, refs, cells, &mut discovered)?;
            enqueue_new(&inst.poly_calls, &mut seen, &mut frontier);
        }
        // P7.S4 (R6): seed the generic-impl member-word monomorphs into the
        // fixpoint, so their bodies' poly cross-calls get composed and their
        // own `poly_calls` routing maps are filled.
        for mut inst in extra_seeds {
            if !seen.insert(inst.symbol.clone()) {
                continue;
            }
            inst.poly_calls = self.cross_calls_of(&inst, arrays, refs, cells, &mut discovered)?;
            enqueue_new(&inst.poly_calls, &mut seen, &mut frontier);
            frontier.push(inst);
        }
        let mut transitive: Vec<CallInst> = Vec::new();
        loop {
            while !frontier.is_empty() {
                let mut next = Vec::new();
                for mut inst in frontier {
                    inst.poly_calls =
                        self.cross_calls_of(&inst, arrays, refs, cells, &mut discovered)?;
                    enqueue_new(&inst.poly_calls, &mut seen, &mut next);
                    transitive.push(inst);
                }
                frontier = next;
            }
            // Seed anything composition discovered along the way and loop
            // again if that introduces new work.
            for (word_name, subst) in std::mem::take(&mut discovered) {
                let symbol = instantiation_symbol(&word_name, &subst);
                if !seen.insert(symbol) {
                    continue;
                }
                if let Some(seed) =
                    self.impl_mono_seed(&word_name, &subst, arrays, refs, cells, &mut discovered)?
                {
                    frontier.push(seed);
                }
            }
            if frontier.is_empty() {
                break;
            }
        }
        // `insts` iterates a `HashMap`, so the discovery order is randomized;
        // sort so the recorded set is a deterministic sequence and not merely
        // a deterministic set. Lowering sorts by symbol too, but a test
        // reading this field should not have to.
        transitive.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        Ok(transitive)
    }

    /// Build the seed `CallInst` for one generic-impl member-word monomorph
    /// `(word_name, subst)` discovered by resolving a `Bound::User`
    /// obligation -- either a top-level call's or a composed cross-call's.
    /// `None` when the name resolves to no polymorphic word, which should not
    /// happen: `resolve_user_bound` only ever pushes a resolved impl's own
    /// member word.
    fn impl_mono_seed(
        &self,
        word_name: &str,
        subst: &Subst,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        impl_monos: &mut Vec<(String, Subst)>,
    ) -> Result<Option<CallInst>, String> {
        let Some(word) = self.words.iter().find(|w| w.name == word_name) else {
            return Ok(None);
        };
        let Some(sig) = &word.poly else {
            return Ok(None);
        };
        let combs = CombinatorIndex::new();
        // P7.S12 (R1.2a): `self.generics` rather than `None` -- this member
        // word's own body may hold a generated enum word call site
        // (`enum_words`), and grounding one needs the live instantiator, not
        // just a declared output shape.
        let ctx = word_ctx(
            word,
            self.structs,
            self.enums,
            self.statics,
            self.modules,
            &combs,
            self.generics,
        );
        let mut outputs = Vec::with_capacity(sig.outputs.len());
        for pty in &sig.outputs {
            outputs.push(apply_subst(
                sig, pty, subst, word_name, word.span, &ctx, arrays, cells, refs,
            )?);
        }
        // P7.S4b (R6): resolve the member word's own `where`-clause bounds
        // at this concrete instantiation, filling `trait_calls` the same
        // way `compose` does for cross-calls. A member word whose body calls
        // a trait member on its own bounded variable (e.g. `print` on `'T`
        // where `'T: Print`) records an obligation during `check_poly_body`;
        // that obligation is resolved here against the concrete θ, and the
        // dispatched symbol rides the `CallInst` so lowering finds it. Without
        // this step, the seed's `trait_calls` stays empty and lowering panics
        // on the unresolved trait-member call.
        let mut trait_calls: HashMap<Span, String> = HashMap::new();
        for (v, bound) in &sig.bounds {
            let Bound::User(trait_id) = bound else {
                continue;
            };
            let Some(ty) = subst.ty_of(*v) else {
                continue;
            };
            resolve_user_bound(
                *trait_id,
                *v,
                ty,
                sig,
                word_name,
                word.span,
                &ctx,
                &self.tr,
                arrays,
                cells,
                refs,
                &mut trait_calls,
                impl_monos,
            )?;
        }
        // P7.S12 (R1.2/R1.2a): the same grounding `check_poly_call` does for
        // an ordinary instantiation, run here for the member word's own
        // monomorph.
        let mut enum_words: HashMap<Span, EnumId> = HashMap::new();
        for (site_span, site_pty) in self.tr.enum_sites_of(word_name, sig) {
            let grounded = apply_subst(
                sig, site_pty, subst, word_name, word.span, &ctx, arrays, cells, refs,
            )?;
            if let Type::Enum(found, _) = grounded {
                enum_words.insert(*site_span, found);
            }
        }
        let symbol = instantiation_symbol(word_name, subst);
        Ok(Some(CallInst {
            callee: word_name.to_string(),
            subst: subst.clone(),
            symbol,
            out_arity: outputs.len(),
            output_types: outputs,
            bundle: None,
            quot_inputs: Vec::new(),
            trait_calls,
            poly_calls: HashMap::new(),
            enum_words,
        }))
    }

    /// Ground every cross-call recorded in `caller`'s body against `caller`'s
    /// own θ, keyed by the body span each one sits at -- the routing map
    /// lowering consults while lowering that one instantiation.
    fn cross_calls_of(
        &self,
        caller: &CallInst,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        impl_monos: &mut Vec<(String, Subst)>,
    ) -> Result<HashMap<Span, CallInst>, String> {
        let Some(records) = self.records.get(&caller.callee) else {
            return Ok(HashMap::new());
        };
        let caller_word = self.sole_poly_word(&caller.callee).ok_or_else(|| {
            overloaded_cross_call_error(
                &caller.callee,
                &records[0].callee,
                &caller.callee,
                records[0].span,
            )
        })?;
        // Every diagnostic a grounding failure raises names the *caller's*
        // body, since that is where the call site is; `word_ctx`'s combinator
        // view is only read by the back-edge guards, which nothing here
        // reaches. P7.S12 (R1.2a): `self.generics` rather than `None` --
        // `compose` used to ground only a callee's declared *output*, but now
        // also grounds the callee body's own `enum_words` sites, which needs
        // the live instantiator (there is no find-only door).
        let ctx = word_ctx(
            caller_word,
            self.structs,
            self.enums,
            self.statics,
            self.modules,
            &CombinatorIndex::new(),
            self.generics,
        );
        let mut routed = HashMap::new();
        for record in records {
            let Some(callee_word) = self.sole_poly_word(&record.callee) else {
                return Err(overloaded_cross_call_error(
                    &caller.callee,
                    &record.callee,
                    &record.callee,
                    record.span,
                ));
            };
            // An `inline` callee with a `Bound::User` is composed like any
            // other cross-call: its body was walked by `check_poly_body`
            // (the pre-pass runs for combinator words with user bounds), so
            // its trait obligations are recorded and `compose` resolves them
            // against the caller's concrete θ. Lowering finds the composed
            // `CallInst` in `poly_calls` (checked before the combinator
            // splice), so the call routes to a real function and the splice
            // is never reached.
            //
            // An `inline` callee *without* a `Bound::User` keeps the old
            // behavior: its body was not walked by `check_poly_body` (it may
            // use builtins `poly_call_term` does not dispatch, like `if`'s
            // `tag`/`branch`), so its own cross-calls are unrecorded and the
            // fixpoint cannot compose them. Splicing is safe only if the
            // body calls no polymorphic word; otherwise it is a located
            // error rather than a silent miscompile.
            if is_combinator(callee_word) {
                let callee_sig = callee_word
                    .poly
                    .as_ref()
                    .expect("sole_poly_word yields a polymorphic word");
                if !callee_sig
                    .bounds
                    .iter()
                    .any(|(_, b)| matches!(b, Bound::User(_)))
                {
                    if body_calls_a_poly_word(&callee_word.body, self.words) {
                        return Err(inline_callee_cross_call_error(
                            &caller.callee,
                            &record.callee,
                            record.span,
                        ));
                    }
                    continue;
                }
            }
            let sig = callee_word
                .poly
                .as_ref()
                .expect("sole_poly_word yields a polymorphic word");
            routed.insert(
                record.span,
                self.compose(record, caller, sig, &ctx, arrays, refs, cells, impl_monos)?,
            );
        }
        Ok(routed)
    }

    /// The single polymorphic word declared under `name`, or `None` when the
    /// name is a polymorphic overload set. An overload set cannot be routed:
    /// `Module::poly_cross_calls` merges both candidates' records under the
    /// one name while each record's mapping indexes its *own* candidate's
    /// variables, and nothing on a `CallInst` says which candidate it came
    /// from -- so composing would silently pick the wrong signature.
    fn sole_poly_word(&self, name: &str) -> Option<&WordDef> {
        let mut candidates = self
            .words
            .iter()
            .filter(|w| w.name == name && w.poly.is_some());
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first)
    }

    /// θ_h = θ_w ∘ mapping: each callee variable takes either the concrete
    /// type the caller supplied outright or `θ_w`'s image of the caller
    /// variable it was matched against. Entry order follows the mapping's,
    /// which is the order the callee's declared inputs first mention each
    /// variable -- and since an id *is* an index handed out in first-mention
    /// order, and `poly_cross_signature_supported` keeps a quotation
    /// parameter (the one shape whose grounding is deferred to a second pass)
    /// off this path entirely, that order is ascending by id. So it agrees
    /// with the sort `check_poly_call` applies to its own θ, and a
    /// `(callee, θ)` reached both ways mints one symbol and one `IrFunc`.
    ///
    /// The callee's declared *inputs* are deliberately not ground here. The
    /// only reason to would be `apply_subst`'s interning, and a cross-call's
    /// input shapes mirror the caller's operand slots (`poly_cross_match`
    /// decomposes structurally, and R6 rejects a compound the caller built
    /// itself), which the caller's own instantiation already interned.
    #[allow(clippy::too_many_arguments)]
    fn compose(
        &self,
        record: &PolyCrossCall,
        caller: &CallInst,
        sig: &PolySig,
        ctx: &Ctx,
        arrays: &mut Vec<ArrayDecl>,
        refs: &mut Vec<RefDecl>,
        cells: &mut Vec<OwnedCellDecl>,
        impl_monos: &mut Vec<(String, Subst)>,
    ) -> Result<CallInst, String> {
        let mut subst = Subst::default();
        for (v, image) in &record.mapping {
            let ty = match image {
                Image::Concrete(t) => *t,
                // A caller variable no input mentions is rejected at the
                // caller's own declaration (its body could not produce one),
                // so θ_w binds every variable an image can name.
                Image::CallerVar(u) => caller.subst.ty_of(*u).expect(
                    "a caller type variable an operand carried is mentioned by the caller's inputs, so its own instantiation bound it",
                ),
            };
            subst.ty.push((*v, ty));
        }
        let mut outputs = Vec::with_capacity(sig.outputs.len());
        for pty in &sig.outputs {
            outputs.push(apply_subst(
                sig,
                pty,
                &subst,
                &record.callee,
                record.span,
                ctx,
                arrays,
                cells,
                refs,
            )?);
        }
        // R2: the same bound loop `check_poly_call` runs against a concrete
        // caller, run here once `subst` is grounded -- a `Bound::User` on a
        // composed callee's own variable is resolved against `self.tr`'s
        // whole-program tables, the identical diagnostic and the identical
        // span (`record.span`) a rejection would have used either way.
        let mut trait_calls: HashMap<Span, String> = HashMap::new();
        for (v, bound) in &sig.bounds {
            let Bound::User(trait_id) = bound else {
                continue;
            };
            // Ungrounded variables skip resolution for the same reason they do
            // in `check_poly_call`: no obligation can name a variable the body
            // could not have dispatched on.
            let Some(ty) = subst.ty_of(*v) else { continue };
            resolve_user_bound(
                *trait_id,
                *v,
                ty,
                sig,
                &record.callee,
                record.span,
                ctx,
                &self.tr,
                arrays,
                cells,
                refs,
                &mut trait_calls,
                impl_monos,
            )?;
        }
        // P7.S12 (R1.2/R1.2a): the same grounding `check_poly_call` does for
        // an ordinary instantiation, run here for the composed callee.
        let mut enum_words: HashMap<Span, EnumId> = HashMap::new();
        for (site_span, site_pty) in self.tr.enum_sites_of(&record.callee, sig) {
            let grounded = apply_subst(
                sig,
                site_pty,
                &subst,
                &record.callee,
                record.span,
                ctx,
                arrays,
                cells,
                refs,
            )?;
            if let Type::Enum(found, _) = grounded {
                enum_words.insert(*site_span, found);
            }
        }
        let symbol = instantiation_symbol(&record.callee, &subst);
        Ok(CallInst {
            callee: record.callee.clone(),
            subst,
            symbol,
            out_arity: outputs.len(),
            output_types: outputs,
            bundle: None,
            // A quotation parameter is a located rejection on the cross-call
            // path (`poly_cross_signature_supported`), so a composed callee
            // has none to record.
            quot_inputs: Vec::new(),
            trait_calls,
            poly_calls: HashMap::new(),
            enum_words,
        })
    }
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
        TermKind::Call(name, _) => words.iter().any(|w| w.poly.is_some() && &w.name == name),
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

/// P7.S4 (R8, Phase 1): a basic diagnostic for more than one matching impl
/// target. Phase 2 replaces this with the specificity partial order and
/// ambiguity error.
/// P7.S4 (R3): select the unique most-specific candidate from the matching
/// `impl:` set, or return a located ambiguity error naming every competing
/// target and the concrete type. A candidate is maximal (most specific) if
/// no other candidate is strictly more specific than it. A unique maximal
/// wins; two or more maximals are an ambiguity.
#[allow(clippy::too_many_arguments)]
fn select_most_specific(
    candidates: &[(&ImplDecl, Subst)],
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
    trait_name: &str,
    span: Span,
) -> Result<usize, String> {
    // Find maximal candidates: those not strictly dominated by any other.
    let maximal: Vec<usize> = (0..candidates.len())
        .filter(|&i| {
            (0..candidates.len()).all(|j| {
                j == i
                    || specificity(
                        &candidates[j].0.target.pattern,
                        &candidates[i].0.target.pattern,
                        &candidates[j].0.target.bounds,
                        &candidates[i].0.target.bounds,
                        ty,
                        arrays,
                        cells,
                        refs,
                        generics,
                    ) != Some(Ordering::Less)
            })
        })
        .collect();

    if maximal.len() == 1 {
        return Ok(maximal[0]);
    }

    Err(ambiguity_error(trait_name, ty, candidates, &maximal, span))
}

/// P7.S4 (R8): a located ambiguity error at the dispatch site naming the
/// trait, every competing target (rendered via `poly_type_str`), and the
/// concrete instantiation.
fn ambiguity_error(
    trait_name: &str,
    ty: Type,
    candidates: &[(&ImplDecl, Subst)],
    maximal: &[usize],
    span: Span,
) -> String {
    let targets: Vec<String> = maximal
        .iter()
        .map(|&i| format!("    `{}`", impl_target_str(&candidates[i].0.target)))
        .collect();
    format!(
        "error: ambiguous `impl:` dispatch for `{trait_name}` at `{ty}` (line {}, col {})\n  matching targets:\n{}\n  no target is more specific than another; write a more specific `impl:` to resolve",
        span.line,
        span.col,
        targets.join("\n"),
    )
}

/// P7.S4b (R7): a located cycle error for a bound-discharge cycle — an impl
/// whose `where`-clause bound requires its own `(TraitId, Type)` pair, directly
/// or transitively. Reported at the `ImplDecl.span` of the impl whose bound
/// creates the back-edge (`source_impl_idx`). Mirrors the path-scoped DFS shape
/// in `check_combinator_cycles` (`src/check/combinators.rs:207`): a pair
/// already in the path-scoped visited-set on entry is a back-edge = cycle.
fn bound_cycle_error(
    tr: &TraitResolveCtx,
    trait_id: TraitId,
    ty: Type,
    source_impl_idx: Option<usize>,
) -> String {
    let trait_name = tr
        .traits
        .get(trait_id.index())
        .map(|t| t.name.as_str())
        .unwrap_or("<unknown>");
    let (target_str, span) = source_impl_idx
        .and_then(|idx| tr.impls.get(idx))
        .map(|imp| (crate::check::impl_target_str(&imp.target), imp.span))
        .unwrap_or_else(|| (ty.name().to_string(), Span::default()));
    format!(
        "error: bound-discharge cycle: `impl: {trait_name} for {target_str}` requires `{trait_name} for {ty}` which is already being resolved (line {}, col {})",
        span.line,
        span.col,
    )
}

/// P7.S4b (R6): check whether every bound on a candidate impl's own type
/// variables discharges at the concrete instantiation. A `Bound::User` is
/// discharged by a recursive `find_bound_impl` call — side-effect-free, so
/// non-winning candidates need no rollback. A `Bound::Copy` is checked by
/// `is_copy`. Returns `Ok(true)` if all discharge, `Ok(false)` if any fails
/// (the caller excludes the candidate), or `Err` for a cycle (propagates).
#[allow(clippy::too_many_arguments)]
fn candidate_bounds_discharge(
    impl_idx: usize,
    imp: &ImplDecl,
    subst: &Subst,
    ctx: &Ctx,
    tr: &TraitResolveCtx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    visited: &mut Vec<(TraitId, Type)>,
) -> Result<bool, String> {
    for (var, bound) in &imp.target.bounds {
        // An ungrounded variable (no input mentions it) skips bound checking
        // for the same reason the top-level bound loop does: no obligation
        // could name a variable the body could not have dispatched on.
        let Some(ty) = subst.ty_of(*var) else {
            continue;
        };
        match bound {
            Bound::Copy => {
                if !is_copy(ty, ctx.structs(), ctx.enums(), arrays) {
                    return Ok(false);
                }
            }
            Bound::User(tid) => {
                match find_bound_impl(
                    *tid,
                    ty,
                    Some(impl_idx),
                    imp.span,
                    ctx,
                    tr,
                    arrays,
                    cells,
                    refs,
                    visited,
                )? {
                    Some(_) => {}
                    None => return Ok(false),
                }
            }
        }
    }
    Ok(true)
}

/// P7.S4b (R6/R7): side-effect-free candidate-finding + selection, factored
/// out of `resolve_user_bound`. Returns `Ok(Some((idx, subst)))` when a
/// winning impl is found, `Ok(None)` when no candidate matches (or every
/// matching candidate's own declared bounds fail to discharge at this
/// instantiation), or `Err` for a bound-discharge cycle or an ambiguity.
///
/// R6: a candidate whose own `where`-clause bounds fail to discharge at the
/// concrete instantiation is excluded from the match set — the recursive
/// discharge calls this helper directly to check existence without side
/// effects (no `trait_calls`/`impl_monos` mutation), so non-winning
/// candidates need no rollback. The recursion happens through the existing
/// `impl_monos` → `cross_calls_of` → `compose` → `resolve_user_bound` chain:
/// the winning impl's member word is monomorphized, `compose` iterates its
/// `sig.bounds`, and `resolve_user_bound` discharges each.
///
/// R7: a path-scoped `(TraitId, Type)` visited-set is inserted on entry and
/// removed on back-track (return), so the set tracks only the current DFS
/// path, not all visited nodes. A pair already in the set on entry is a
/// back-edge = cycle, reported at `ImplDecl.span` of the impl whose bound
/// creates the edge (`source_impl_idx`). This prevents false-positives on
/// diamond-shaped shared resolutions while catching true cycles.
#[allow(clippy::too_many_arguments)]
fn find_bound_impl(
    trait_id: TraitId,
    ty: Type,
    source_impl_idx: Option<usize>,
    span: Span,
    ctx: &Ctx,
    tr: &TraitResolveCtx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    visited: &mut Vec<(TraitId, Type)>,
) -> Result<Option<(usize, Subst)>, String> {
    // R7: cycle detection — (trait_id, ty) already on the current DFS path
    // is a back-edge = cycle.
    if visited.contains(&(trait_id, ty)) {
        return Err(bound_cycle_error(tr, trait_id, ty, source_impl_idx));
    }
    visited.push((trait_id, ty));

    let trait_decl = tr.traits.get(trait_id.index()).expect(
        "a bound's `TraitId` indexes the whole-program trait table, so a call site resolving one must be given that table and not a scratch one",
    );
    let generics = ctx.generics().map(|c| c.borrow());

    // P7.S4 (R2): one-way match the concrete type against each impl target
    // pattern, collecting (impl, subst) candidates.
    let candidates: Vec<(usize, &ImplDecl, Subst)> = tr
        .impls
        .iter()
        .enumerate()
        .filter_map(|(idx, i)| {
            if i.trait_id != trait_id {
                return None;
            }
            match_impl_target(
                &i.target.pattern,
                ty,
                arrays,
                cells,
                refs,
                generics.as_deref(),
            )
            .map(|s| (idx, i, s))
        })
        .collect();

    // R6: filter out candidates whose own declared bounds fail to discharge
    // at this concrete instantiation. A candidate with no `where`-clause
    // bounds is always kept.
    let mut discharging: Vec<(usize, &ImplDecl, Subst)> = Vec::new();
    for (idx, imp, subst) in candidates {
        if candidate_bounds_discharge(idx, imp, &subst, ctx, tr, arrays, cells, refs, visited)? {
            discharging.push((idx, imp, subst));
        }
    }

    let result = if discharging.is_empty() {
        Ok(None)
    } else {
        let candidates_ref: Vec<(&ImplDecl, Subst)> = discharging
            .iter()
            .map(|(_, imp, s)| (*imp, s.clone()))
            .collect();
        let winner = if candidates_ref.len() > 1 {
            // P7.S4 (R3): select the unique most-specific candidate via the
            // equivalence-class refinement partial order.
            select_most_specific(
                &candidates_ref,
                ty,
                arrays,
                cells,
                refs,
                generics.as_deref(),
                &trait_decl.name,
                span,
            )?
        } else {
            0
        };
        Ok(Some((discharging[winner].0, discharging[winner].2.clone())))
    };

    visited.pop();
    result
}

/// P7.S3e (R8): one `Bound::User` at a call site whose θ is known -- the
/// `impl:` registry lookup that decides satisfaction, then the resolution of
/// every obligation the callee's body recorded on this variable to the
/// implementing word's lowering symbol, keyed by the body span that dispatched
/// it.
///
/// P7.S4 (R2/R6): the registry one-way-matches the concrete instantiation
/// `Type` against each `impl:` target `PolyType` via `match_impl_target`,
/// producing a `Subst` per match. Exactly one match → dispatch. Zero → the
/// existing `unsatisfied_user_bound_error`. More than one → the specificity
/// partial order (Phase 2, R3) selects the unique most-specific candidate;
/// two or more incomparable maxima are a located ambiguity error (R8). For a
/// generic winner (target not `PolyType::Concrete`), the dispatched symbol is
/// `instantiation_symbol(word_symbols[idx], &subst)` (not the bare
/// `word_symbols[idx]`), and the `(member_word, subst)` pair is recorded for
/// lowering. A concrete winner keeps the bare `word_symbols[idx]` path.
///
/// P7.S4b (R6/R7): the candidate-finding + selection is factored into
/// `find_bound_impl` (side-effect-free, with recursive bound discharge and
/// cycle detection); this function calls it, then does the obligation
/// routing. A candidate whose own `where`-clause bounds fail to discharge at
/// the concrete instantiation is excluded before selection; if that leaves
/// no candidate, the existing `unsatisfied_user_bound_error` fires.
#[allow(clippy::too_many_arguments)]
fn resolve_user_bound(
    trait_id: TraitId,
    v: u32,
    ty: Type,
    sig: &PolySig,
    name: &str,
    span: Span,
    ctx: &Ctx,
    tr: &TraitResolveCtx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &[OwnedCellDecl],
    refs: &mut Vec<RefDecl>,
    trait_calls: &mut HashMap<Span, String>,
    impl_monos: &mut Vec<(String, Subst)>,
) -> Result<(), String> {
    let trait_decl = tr.traits.get(trait_id.index()).expect(
        "a bound's `TraitId` indexes the whole-program trait table, so a call site resolving one must be given that table and not a scratch one",
    );
    // P7.S4b (R6/R7): find the winning impl via the factored helper, which
    // also discharges each candidate's own bounds recursively and detects
    // cycles via a path-scoped visited-set.
    let mut visited: Vec<(TraitId, Type)> = Vec::new();
    let winner = find_bound_impl(
        trait_id,
        ty,
        None,
        span,
        ctx,
        tr,
        arrays,
        cells,
        refs,
        &mut visited,
    )?;
    let (imp_idx, subst) = match winner {
        Some((idx, subst)) => (idx, subst),
        None => {
            return Err(unsatisfied_user_bound_error(
                ctx,
                span,
                name,
                &sig.ty_var_names[v as usize],
                trait_decl,
                ty,
                arrays,
                refs,
            ));
        }
    };
    let imp = &tr.impls[imp_idx];
    let is_generic = !imp.target.is_concrete();
    for ob in tr
        .obligations_of(name, sig)
        .iter()
        .filter(|o| o.trait_id == trait_id && o.var == v)
    {
        let Some((_, idx)) = imp.resolved.iter().find(|(member, _)| *member == ob.member) else {
            return Err(unresolved_trait_obligation_error(
                ctx,
                span,
                name,
                &trait_decl.name,
                &ob.member,
                ty,
                ob.span,
            ));
        };
        let word_sym = &tr.word_symbols[*idx];
        let symbol = if is_generic {
            // P7.S4 (R6): mint the dispatched symbol as the instantiation of
            // the member word at the matched substitution.
            let s = instantiation_symbol(word_sym, &subst);
            impl_monos.push((word_sym.clone(), subst.clone()));
            s
        } else {
            word_sym.clone()
        };
        trait_calls.insert(ob.span, symbol);
    }
    Ok(())
}

/// R8: the concrete type a bounded variable was instantiated with has no
/// `impl:` for the trait. Names the trait, the type, and every member
/// signature the missing impl would have to provide, grounded at that type --
/// grounding interns, but only on this failure path, where the compile is
/// already over.
#[allow(clippy::too_many_arguments)]
fn unsatisfied_user_bound_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    trait_decl: &TraitDecl,
    ty: Type,
    arrays: &mut Vec<ArrayDecl>,
    refs: &mut Vec<RefDecl>,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let sigs: Vec<String> = trait_decl
        .members
        .iter()
        .map(|m| {
            let ins: Vec<String> = m
                .sig
                .inputs
                .iter()
                .map(|t| ground_member_type(t, ty, arrays, refs).name().to_string())
                .collect();
            let outs: Vec<String> = m
                .sig
                .outputs
                .iter()
                .map(|t| ground_member_type(t, ty, arrays, refs).name().to_string())
                .collect();
            match (ins.is_empty(), outs.is_empty()) {
                (true, true) => "( -- )".to_string(),
                (true, false) => format!("( -- {} )", outs.join(" ")),
                (false, true) => format!("( {} -- )", ins.join(" ")),
                (false, false) => format!("( {} -- {} )", ins.join(" "), outs.join(" ")),
            }
        })
        .collect();
    let missing = format!(
        "`{ty}` does not satisfy `{}`: no `{}` found",
        trait_decl.name,
        sigs.join("`, `")
    );
    format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}` in {name} (line {}, col {})\n  {missing}",
            span.line, span.col
        , name = ctx.rendered_word())
}

/// R17: the backstop for a satisfied bound whose recorded obligation resolves
/// to nothing -- an `impl:` that binds no word for a member its trait
/// requires. `check_impl_decls` rejects that impl at its declaration site, so
/// reaching here means the two disagree; say so, located, rather than drop the
/// call and leave lowering to emit nothing.
fn unresolved_trait_obligation_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    trait_name: &str,
    member: &str,
    ty: Type,
    member_span: Span,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let site = { format!(" in {name}", name = ctx.rendered_word()) };
    format!(
        "error: `impl: {trait_name} for {ty}` binds no word for member `{member}`, dispatched at line {}, col {} in the body of `{callee}` (instantiated at line {}, col {}{site})",
        member_span.line, member_span.col, span.line, span.col
    )
}

/// R5: unify one declared input `PolyType` against a concrete slot type,
/// extending `subst`. A repeated variable forced to two concretes is X4; a
/// non-array where an array is declared, or a mismatched concrete, is the
/// ordinary type-mismatch error.
///
/// P7.S3t (R5): `seeded` names the variables an explicit type-argument list
/// bound before unification began (empty at every other caller). It exists
/// P7.S4 (R2): one-way match a concrete `Type` against an `impl:` target's
/// `PolyType` pattern, producing a `Subst` in the impl's own variable
/// namespace. The reverse of `apply_subst` (PolyType→Type) and structurally
/// a sibling of `unify_poly_input` (which extends a `Subst` in place), but
/// keyed to the impl's variable namespace (no `PolySig`/`seeded`) and
/// returning a fresh `Subst` rather than extending one. `None` means the
/// pattern does not match this concrete type.
///
/// `PolyType::Concrete(t)` matches only on `t == ty` (the existing exact
/// path, unchanged for concrete-only impls). A `Var` or `Len::Var` already
/// bound in a prior position must match consistently — if the subst already
/// maps the variable to a different value, the match fails.
///
/// The `Generic` arm needs `GenericTypes` to reverse the instantiation
/// (`struct_instantiation_of`/`enum_instantiation_of`); when `generics` is
/// `None` (e.g. in `poly_admits`, which has no generic table), a `Generic`
/// pattern simply does not match.
pub(crate) fn match_impl_target(
    pattern: &PolyType,
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
) -> Option<Subst> {
    let mut subst = Subst::default();
    match_impl_target_rec(pattern, ty, arrays, cells, refs, generics, &mut subst)?;
    Some(subst)
}

fn match_impl_target_rec(
    pattern: &PolyType,
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
    subst: &mut Subst,
) -> Option<()> {
    match pattern {
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        PolyType::Concrete(t) => {
            if *t == ty {
                Some(())
            } else {
                None
            }
        }
        PolyType::Var(v) => {
            if let Some(prev) = subst.ty_of(*v) {
                if prev == ty {
                    Some(())
                } else {
                    None
                }
            } else {
                let pos = subst.ty.partition_point(|(id, _)| *id < *v);
                subst.ty.insert(pos, (*v, ty));
                Some(())
            }
        }
        PolyType::Array(elem, len) => {
            let Type::Array(id, _) = ty else {
                return None;
            };
            let (elem_ty, count) = (arrays[id.index()].element, arrays[id.index()].count);
            match_impl_target_rec(elem, elem_ty, arrays, cells, refs, generics, subst)?;
            match len {
                Len::Concrete(k) => {
                    if *k == count {
                        Some(())
                    } else {
                        None
                    }
                }
                Len::Var(ln) => {
                    if let Some(prev) = subst.len_of(*ln) {
                        if prev == count {
                            Some(())
                        } else {
                            None
                        }
                    } else {
                        subst.len.push((*ln, count));
                        Some(())
                    }
                }
            }
        }
        PolyType::Quotation(ins, outs, _, _, _) => {
            let eff = crate::ast::is_quotation_type(ty)?;
            if ins.len() != eff.inputs.len() || outs.len() != eff.outputs.len() {
                return None;
            }
            for (p, c) in ins.iter().zip(&eff.inputs) {
                match_impl_target_rec(p, *c, arrays, cells, refs, generics, subst)?;
            }
            for (p, c) in outs.iter().zip(&eff.outputs) {
                match_impl_target_rec(p, *c, arrays, cells, refs, generics, subst)?;
            }
            Some(())
        }
        PolyType::Ref(referent, mutable) => {
            let (slot_referent, slot_mutable) = ref_parts(ty, refs)?;
            if slot_mutable != *mutable {
                return None;
            }
            match_impl_target_rec(
                referent,
                slot_referent,
                arrays,
                cells,
                refs,
                generics,
                subst,
            )
        }
        PolyType::OwnedCell(payload) => {
            let Type::OwnedCell(id, _) = ty else {
                return None;
            };
            let slot_payload = cells[id.index()].payload;
            match_impl_target_rec(payload, slot_payload, arrays, cells, refs, generics, subst)
        }
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            name: _,
        } => {
            let generics = generics?;
            let found = if *is_enum {
                let Type::Enum(id, _) = ty else {
                    return None;
                };
                generics.enum_instantiation_of(id)
            } else {
                let Type::Struct(id, _) = ty else {
                    return None;
                };
                generics.struct_instantiation_of(id)
            };
            let (found_idx, found_module, found_args) = found?;
            if found_idx != *idx as usize
                || found_module != *module
                || found_args.len() != args.len()
            {
                return None;
            }
            let found_args = found_args.to_vec();
            for (arg_pty, arg_ty) in args.iter().zip(found_args.iter()) {
                match_impl_target_rec(
                    arg_pty,
                    *arg_ty,
                    arrays,
                    cells,
                    refs,
                    Some(generics),
                    subst,
                )?;
            }
            Some(())
        }
    }
}

// P7.S4 Phase 2 (R3): the specificity partial order — equivalence-class
// refinement over shared variables.

/// A flattened position in a specificity comparison. Each leaf in a pattern's
/// structural walk against the matched concrete type is either a concrete
/// value (a singleton equivalence class) or a variable (shared class with
/// every other position holding the same variable id). Type variables and
/// length variables are separate namespaces, so a `Position` carries the
/// namespace tag to keep the two partitions distinct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Position {
    /// A type-position leaf: concrete or a type variable.
    TyConcrete,
    TyVar(u32),
    /// A length-position leaf: concrete or a length variable.
    LenConcrete,
    LenVar(u32),
}

/// Walk a `PolyType` pattern against the concrete `Type` it matched,
/// collecting one `Position` per leaf. `Concrete(t)` is unfolded by walking
/// `ty`'s own structure (every leaf becomes concrete), so a concrete target
/// produces the same positions as a structural pattern matching the same
/// type. Returns `None` if the pattern's structure is incompatible with `ty`
/// (should not happen for a pattern that already matched via
/// `match_impl_target`, but handled for safety).
#[allow(clippy::too_many_arguments)]
fn collect_positions(
    pattern: &PolyType,
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
    out: &mut Vec<Position>,
) -> Option<()> {
    match pattern {
        PolyType::QuotLit => {
            unreachable!("a quotation-literal marker never reaches a signature")
        }
        PolyType::Concrete(_) => collect_concrete_positions(ty, arrays, cells, refs, generics, out),
        PolyType::Var(v) => {
            out.push(Position::TyVar(*v));
            Some(())
        }
        PolyType::Array(elem, len) => {
            let Type::Array(id, _) = ty else {
                return None;
            };
            let elem_ty = arrays[id.index()].element;
            collect_positions(elem, elem_ty, arrays, cells, refs, generics, out)?;
            match len {
                Len::Concrete(_) => out.push(Position::LenConcrete),
                Len::Var(ln) => out.push(Position::LenVar(*ln)),
            }
            Some(())
        }
        PolyType::Ref(referent, mutable) => {
            let (slot_referent, slot_mutable) = ref_parts(ty, refs)?;
            if slot_mutable != *mutable {
                return None;
            }
            collect_positions(referent, slot_referent, arrays, cells, refs, generics, out)
        }
        PolyType::OwnedCell(payload) => {
            let Type::OwnedCell(id, _) = ty else {
                return None;
            };
            let slot_payload = cells[id.index()].payload;
            collect_positions(payload, slot_payload, arrays, cells, refs, generics, out)
        }
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            name: _,
        } => {
            let generics = generics?;
            let found = if *is_enum {
                let Type::Enum(id, _) = ty else {
                    return None;
                };
                generics.enum_instantiation_of(id)
            } else {
                let Type::Struct(id, _) = ty else {
                    return None;
                };
                generics.struct_instantiation_of(id)
            };
            let (found_idx, found_module, found_args) = found?;
            if found_idx != *idx as usize
                || found_module != *module
                || found_args.len() != args.len()
            {
                return None;
            }
            let found_args = found_args.to_vec();
            for (arg_pty, arg_ty) in args.iter().zip(found_args.iter()) {
                collect_positions(arg_pty, *arg_ty, arrays, cells, refs, Some(generics), out)?;
            }
            Some(())
        }
        PolyType::Quotation(ins, outs, _, _, _) => {
            let eff = crate::ast::is_quotation_type(ty)?;
            if ins.len() != eff.inputs.len() || outs.len() != eff.outputs.len() {
                return None;
            }
            for (p, c) in ins.iter().zip(&eff.inputs) {
                collect_positions(p, *c, arrays, cells, refs, generics, out)?;
            }
            for (p, c) in outs.iter().zip(&eff.outputs) {
                collect_positions(p, *c, arrays, cells, refs, generics, out)?;
            }
            Some(())
        }
    }
}

/// Unfold a concrete `Type` into leaf positions, pushing a concrete
/// `Position` at every leaf. This is the `Concrete(t)` arm of
/// `collect_positions`: since the whole type is concrete, every position is
/// a concrete singleton.
#[allow(clippy::too_many_arguments)]
fn collect_concrete_positions(
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
    out: &mut Vec<Position>,
) -> Option<()> {
    match ty {
        Type::Array(id, _) => {
            let elem_ty = arrays[id.index()].element;
            collect_concrete_positions(elem_ty, arrays, cells, refs, generics, out)?;
            out.push(Position::LenConcrete);
            Some(())
        }
        Type::Ref(id, _, _) => {
            let referent = refs[id.index()].referent;
            collect_concrete_positions(referent, arrays, cells, refs, generics, out)
        }
        Type::OwnedCell(id, _) => {
            let payload = cells[id.index()].payload;
            collect_concrete_positions(payload, arrays, cells, refs, generics, out)
        }
        Type::Struct(id, _) => {
            if let Some(generics) = generics {
                if let Some((_, _, args)) = generics.struct_instantiation_of(id) {
                    if !args.is_empty() {
                        for arg_ty in args {
                            collect_concrete_positions(
                                *arg_ty,
                                arrays,
                                cells,
                                refs,
                                Some(generics),
                                out,
                            )?;
                        }
                        return Some(());
                    }
                }
            }
            out.push(Position::TyConcrete);
            Some(())
        }
        Type::Enum(id, _) => {
            if let Some(generics) = generics {
                if let Some((_, _, args)) = generics.enum_instantiation_of(id) {
                    if !args.is_empty() {
                        for arg_ty in args {
                            collect_concrete_positions(
                                *arg_ty,
                                arrays,
                                cells,
                                refs,
                                Some(generics),
                                out,
                            )?;
                        }
                        return Some(());
                    }
                }
            }
            out.push(Position::TyConcrete);
            Some(())
        }
        Type::Quotation(eff) | Type::InlineQuotation(eff) | Type::OwningQuotation(eff) => {
            for c in &eff.inputs {
                collect_concrete_positions(*c, arrays, cells, refs, generics, out)?;
            }
            for c in &eff.outputs {
                collect_concrete_positions(*c, arrays, cells, refs, generics, out)?;
            }
            Some(())
        }
        _ => {
            out.push(Position::TyConcrete);
            Some(())
        }
    }
}

/// Whether two positions share the same equivalence class: both are the
/// same variable id in the same namespace (type or length). Concrete
/// positions are singletons — they share a class with no other position.
fn same_class(a: &Position, b: &Position) -> bool {
    match (a, b) {
        (Position::TyVar(x), Position::TyVar(y)) => x == y,
        (Position::LenVar(x), Position::LenVar(y)) => x == y,
        _ => false,
    }
}

/// Review fix (P7.S4 Phase 2): a leaf reached while walking two patterns
/// *together* against the concrete `ty` they both matched. Most leaves are
/// `Aligned` -- both patterns bottom out here (at a variable or a concrete
/// value), feeding the equivalence-class machinery below. But when one
/// side is a bare variable and the other is genuinely structured (an
/// `Array`/`Generic`/`Ref`/`OwnedCell`/`Quotation`, or a `Concrete` folding
/// a *structured* type), the variable places no constraint anywhere within
/// that structure, so the structured side is unconditionally more specific
/// over the whole region -- however unconstrained *its own* internals may
/// be. Independently flattening each pattern into same-length position
/// vectors (the pre-fix approach) can't express this: a bare `'T` and a
/// compound `array['T 'N]` flatten to different lengths and were reported
/// incomparable no matter how the rest of the patterns compared.
enum PairedLeaf {
    Aligned(Position, Position),
    AMoreSpecific,
    BMoreSpecific,
}

/// One side of a pairing is a bare variable (`var_pos`); `other` is
/// whatever the counterpart pattern is at this exact position. If `other`
/// also bottoms out at a single leaf (another variable, or `Concrete` on a
/// scalar/opaque type), this is an ordinary aligned pair. If `other` is
/// genuinely structured here, the variable side contributes nothing and
/// `other`'s side wins this whole region unconditionally. `var_is_b`
/// selects which side of the `Aligned`/`*MoreSpecific` pair the variable
/// occupies.
#[allow(clippy::too_many_arguments)]
fn collect_var_vs_other(
    var_pos: Position,
    other: &PolyType,
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
    out: &mut Vec<PairedLeaf>,
    var_is_b: bool,
) -> Option<()> {
    let mut other_pos = Vec::new();
    collect_positions(other, ty, arrays, cells, refs, generics, &mut other_pos)?;
    if other_pos.len() == 1 {
        out.push(if var_is_b {
            PairedLeaf::Aligned(other_pos[0], var_pos)
        } else {
            PairedLeaf::Aligned(var_pos, other_pos[0])
        });
    } else {
        out.push(if var_is_b {
            PairedLeaf::AMoreSpecific
        } else {
            PairedLeaf::BMoreSpecific
        });
    }
    Some(())
}

/// The array/ref/cell/quotation twins below extract a pattern's sub-parts
/// for paired recursion, transparently unfolding a folded `Concrete` leaf
/// against `ty`'s own shape (R1: a fully concrete target folds to one
/// `Concrete` node, so it must decompose exactly like the structural
/// pattern it stands in for). The caller has already excluded `Var` (via
/// `collect_var_vs_other`) and both-`Concrete` (nothing to recurse into),
/// so `pattern` is always either the matching compound variant or a
/// `Concrete` leaf here.
fn array_elem_len(pattern: &PolyType, elem_ty: Type, count: u32) -> (PolyType, Len) {
    match pattern {
        PolyType::Array(e, l) => ((**e).clone(), l.clone()),
        PolyType::Concrete(_) => (PolyType::Concrete(elem_ty), Len::Concrete(count)),
        _ => unreachable!("ty is array-shaped, so a matching pattern is Array or Concrete"),
    }
}

fn len_position(len: Len) -> Position {
    match len {
        Len::Concrete(_) => Position::LenConcrete,
        Len::Var(v) => Position::LenVar(v),
    }
}

fn ref_referent(pattern: &PolyType, referent_ty: Type) -> PolyType {
    match pattern {
        PolyType::Ref(r, _) => (**r).clone(),
        PolyType::Concrete(_) => PolyType::Concrete(referent_ty),
        _ => unreachable!("ty is ref-shaped, so a matching pattern is Ref or Concrete"),
    }
}

fn cell_payload(pattern: &PolyType, payload_ty: Type) -> PolyType {
    match pattern {
        PolyType::OwnedCell(p) => (**p).clone(),
        PolyType::Concrete(_) => PolyType::Concrete(payload_ty),
        _ => {
            unreachable!("ty is owned-cell-shaped, so a matching pattern is OwnedCell or Concrete")
        }
    }
}

fn generic_args_of(pattern: &PolyType, ty_args: &[Type]) -> Vec<PolyType> {
    match pattern {
        PolyType::Generic { args, .. } => args.clone(),
        PolyType::Concrete(_) => ty_args.iter().map(|t| PolyType::Concrete(*t)).collect(),
        _ => unreachable!("ty is generic-shaped, so a matching pattern is Generic or Concrete"),
    }
}

fn quotation_parts(pattern: &PolyType, eff: &QuotEffect) -> (Vec<PolyType>, Vec<PolyType>) {
    match pattern {
        PolyType::Quotation(ins, outs, _, _, _) => (ins.clone(), outs.clone()),
        PolyType::Concrete(_) => (
            eff.inputs.iter().map(|t| PolyType::Concrete(*t)).collect(),
            eff.outputs.iter().map(|t| PolyType::Concrete(*t)).collect(),
        ),
        _ => unreachable!("ty is quotation-shaped, so a matching pattern is Quotation or Concrete"),
    }
}

/// Walk `a` and `b` *together* against the concrete `ty` they both matched,
/// producing one `PairedLeaf` per point where at least one side stops
/// recursing. This is `collect_positions` run twice, but paired rather than
/// independent: a depth mismatch (one side a bare variable, the other
/// still-structured) is classified explicitly via `collect_var_vs_other`
/// instead of silently producing differently-sized position vectors.
#[allow(clippy::too_many_arguments)]
fn collect_paired_positions(
    a: &PolyType,
    b: &PolyType,
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
    out: &mut Vec<PairedLeaf>,
) -> Option<()> {
    match (a, b) {
        (PolyType::Var(va), PolyType::Var(vb)) => {
            out.push(PairedLeaf::Aligned(
                Position::TyVar(*va),
                Position::TyVar(*vb),
            ));
            return Some(());
        }
        (PolyType::Var(v), other) => {
            return collect_var_vs_other(
                Position::TyVar(*v),
                other,
                ty,
                arrays,
                cells,
                refs,
                generics,
                out,
                false,
            );
        }
        (other, PolyType::Var(v)) => {
            return collect_var_vs_other(
                Position::TyVar(*v),
                other,
                ty,
                arrays,
                cells,
                refs,
                generics,
                out,
                true,
            );
        }
        (PolyType::Concrete(_), PolyType::Concrete(_)) => return Some(()),
        _ => {}
    }
    match ty {
        Type::Array(id, _) => {
            let elem_ty = arrays[id.index()].element;
            let count = arrays[id.index()].count;
            let (ea, la) = array_elem_len(a, elem_ty, count);
            let (eb, lb) = array_elem_len(b, elem_ty, count);
            collect_paired_positions(&ea, &eb, elem_ty, arrays, cells, refs, generics, out)?;
            out.push(PairedLeaf::Aligned(len_position(la), len_position(lb)));
            Some(())
        }
        Type::Ref(id, _, _) => {
            let referent_ty = refs[id.index()].referent;
            let ra = ref_referent(a, referent_ty);
            let rb = ref_referent(b, referent_ty);
            collect_paired_positions(&ra, &rb, referent_ty, arrays, cells, refs, generics, out)
        }
        Type::OwnedCell(id, _) => {
            let payload_ty = cells[id.index()].payload;
            let pa = cell_payload(a, payload_ty);
            let pb = cell_payload(b, payload_ty);
            collect_paired_positions(&pa, &pb, payload_ty, arrays, cells, refs, generics, out)
        }
        Type::Struct(id, _) => {
            let generics = generics?;
            let (_, _, ty_args) = generics.struct_instantiation_of(id)?;
            if ty_args.is_empty() {
                return Some(());
            }
            let ty_args = ty_args.to_vec();
            let a_args = generic_args_of(a, &ty_args);
            let b_args = generic_args_of(b, &ty_args);
            for ((pa, pb), arg_ty) in a_args.iter().zip(b_args.iter()).zip(ty_args.iter()) {
                collect_paired_positions(
                    pa,
                    pb,
                    *arg_ty,
                    arrays,
                    cells,
                    refs,
                    Some(generics),
                    out,
                )?;
            }
            Some(())
        }
        Type::Enum(id, _) => {
            let generics = generics?;
            let (_, _, ty_args) = generics.enum_instantiation_of(id)?;
            if ty_args.is_empty() {
                return Some(());
            }
            let ty_args = ty_args.to_vec();
            let a_args = generic_args_of(a, &ty_args);
            let b_args = generic_args_of(b, &ty_args);
            for ((pa, pb), arg_ty) in a_args.iter().zip(b_args.iter()).zip(ty_args.iter()) {
                collect_paired_positions(
                    pa,
                    pb,
                    *arg_ty,
                    arrays,
                    cells,
                    refs,
                    Some(generics),
                    out,
                )?;
            }
            Some(())
        }
        Type::Quotation(eff) | Type::InlineQuotation(eff) | Type::OwningQuotation(eff) => {
            let (a_ins, a_outs) = quotation_parts(a, eff);
            let (b_ins, b_outs) = quotation_parts(b, eff);
            if a_ins.len() != eff.inputs.len()
                || a_outs.len() != eff.outputs.len()
                || b_ins.len() != eff.inputs.len()
                || b_outs.len() != eff.outputs.len()
            {
                return None;
            }
            for ((pa, pb), c) in a_ins.iter().zip(b_ins.iter()).zip(eff.inputs.iter()) {
                collect_paired_positions(pa, pb, *c, arrays, cells, refs, generics, out)?;
            }
            for ((pa, pb), c) in a_outs.iter().zip(b_outs.iter()).zip(eff.outputs.iter()) {
                collect_paired_positions(pa, pb, *c, arrays, cells, refs, generics, out)?;
            }
            Some(())
        }
        // Neither side is `Var` and not both are `Concrete` (excluded
        // above), yet `ty` is a scalar/opaque leaf -- unreachable, since
        // only `Concrete`/`Var` can match a scalar `ty` in the first place.
        _ => Some(()),
    }
}

/// P7.S4 (R3): the specificity partial order over two `impl:` target
/// patterns, both of which matched the same concrete `ty`. Walks the two
/// patterns together (`collect_paired_positions`), then compares
/// equivalence classes over the aligned leaves and combines that with any
/// one-sided (depth-mismatch) evidence.
///
/// Returns `Some(Less)` if A is strictly more specific than B (A ≺ B),
/// `Some(Greater)` if B is strictly more specific than A (B ≺ A), or `None`
/// if neither is strictly more specific (equal or incomparable).
///
/// P7.S4b (R5): when patterns are equal (neither strictly dominates the
/// other and they are structurally identical), the bound-set tiebreak
/// applies: a strictly more constrained bound set (proper superset) wins.
/// `impl: Eq for ['T N] where 'T: Eq` is more specific than `impl: Eq for
/// ['T N]` with no bounds. Variable identity is normalized across
/// alpha-equivalent targets via `PolyType` structural equality (both
/// `['T N]` and `['U M]` fold to `Array(Var(0), Var(0))`), so `(u32, Bound)`
/// index pairs are positionally consistent and need no name-table lookup.
///
/// Over the aligned leaves, pattern A ≺ B iff:
/// (1) every position where B has a concrete value, A also has a concrete
/// value (A doesn't relax B's concreteness);
/// (2) B's equivalence classes refine A's — every pair sharing a class in B
/// also shares a class in A (A is coarser/more-merged/more-constraining);
/// (3) A is strictly more constrained somewhere — A has concrete where B has
/// a variable, or B has a strictly finer partition (A has a coarser one).
///
/// A one-sided leaf (a bare variable on one side against structure on the
/// other) contributes unconditional evidence for whichever side has the
/// structure; conflicting one-sided evidence (each side wins some region)
/// is incomparable outright, and consistent one-sided evidence combines
/// with the aligned-leaf verdict, disagreeing only if the aligned leaves
/// favor the *other* side.
///
/// Concrete positions are singletons. Type-variable and length-variable
/// equivalence classes are separate namespaces (type var 0 and length var 0
/// are distinct), matching `Subst`'s separate `ty`/`len` maps.
#[allow(clippy::too_many_arguments)]
fn specificity(
    a: &PolyType,
    b: &PolyType,
    a_bounds: &[(u32, Bound)],
    b_bounds: &[(u32, Bound)],
    ty: Type,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    generics: Option<&GenericTypes>,
) -> Option<Ordering> {
    let mut paired = Vec::new();
    collect_paired_positions(a, b, ty, arrays, cells, refs, generics, &mut paired)?;
    let mut a_leaves = Vec::new();
    let mut b_leaves = Vec::new();
    let mut a_one_sided = false;
    let mut b_one_sided = false;
    for leaf in &paired {
        match leaf {
            PairedLeaf::Aligned(pa, pb) => {
                a_leaves.push(*pa);
                b_leaves.push(*pb);
            }
            PairedLeaf::AMoreSpecific => a_one_sided = true,
            PairedLeaf::BMoreSpecific => b_one_sided = true,
        }
    }
    if a_one_sided && b_one_sided {
        return None;
    }
    let a_over_b = is_strictly_more_specific(&a_leaves, &b_leaves);
    let b_over_a = is_strictly_more_specific(&b_leaves, &a_leaves);
    if a_one_sided {
        return if b_over_a { None } else { Some(Ordering::Less) };
    }
    if b_one_sided {
        return if a_over_b {
            None
        } else {
            Some(Ordering::Greater)
        };
    }
    let pattern_result = match (a_over_b, b_over_a) {
        (true, false) => Some(Ordering::Less),
        (false, true) => Some(Ordering::Greater),
        _ => None,
    };
    // P7.S4b (R5): bound-set tiebreak. When patterns are structurally equal
    // (neither dominates), a strictly more constrained bound set is a
    // specificity tiebreak. Incomparable patterns (which also produce
    // `None`) are not equal, so `a == b` distinguishes the two cases.
    if pattern_result.is_none() && a == b {
        return bound_set_tiebreak(a_bounds, b_bounds);
    }
    pattern_result
}

/// P7.S4b (R5): the bound-set tiebreak for two candidates with equal
/// patterns. Returns `Some(Less)` if `a_bounds` is a strict superset of
/// `b_bounds` (A is more constrained → more specific), `Some(Greater)` if
/// `b_bounds` is a strict superset of `a_bounds`, or `None` if neither
/// dominates. Compared as unordered sets of `(u32, Bound)` pairs, mirroring
/// the duplicate-check's `bounds_eq` (variable indices are positionally
/// consistent across alpha-equivalent targets via `PolyType` equality).
fn bound_set_tiebreak(a_bounds: &[(u32, Bound)], b_bounds: &[(u32, Bound)]) -> Option<Ordering> {
    let a_sup_b = b_bounds.iter().all(|p| a_bounds.contains(p))
        && a_bounds.iter().any(|p| !b_bounds.contains(p));
    let b_sup_a = a_bounds.iter().all(|p| b_bounds.contains(p))
        && b_bounds.iter().any(|p| !a_bounds.contains(p));
    match (a_sup_b, b_sup_a) {
        (true, false) => Some(Ordering::Less),
        (false, true) => Some(Ordering::Greater),
        _ => None,
    }
}

/// The three-condition check for A ≺ B over parallel position vectors.
fn is_strictly_more_specific(a: &[Position], b: &[Position]) -> bool {
    let n = a.len();
    debug_assert_eq!(n, b.len());

    // (1) A doesn't relax B's concreteness: every position where B is
    // concrete, A is also concrete.
    let cond1 = b.iter().zip(a).all(|(b_pos, a_pos)| {
        !matches!(b_pos, Position::TyConcrete | Position::LenConcrete)
            || matches!(a_pos, Position::TyConcrete | Position::LenConcrete)
    });
    if !cond1 {
        return false;
    }

    // (2) B's classes refine A's: every pair sharing a class in B also
    // shares a class in A.
    let cond2 =
        (0..n).all(|i| (i + 1..n).all(|j| !same_class(&b[i], &b[j]) || same_class(&a[i], &a[j])));
    if !cond2 {
        return false;
    }

    // (3) A is strictly more constrained somewhere: A has concrete where B
    // has a variable, or B has a strictly finer partition (some pair in A's
    // class is not in B's class).
    let cond3a = a.iter().zip(b).any(|(a_pos, b_pos)| {
        matches!(a_pos, Position::TyConcrete | Position::LenConcrete)
            && matches!(b_pos, Position::TyVar(_) | Position::LenVar(_))
    });
    let cond3b =
        (0..n).any(|i| (i + 1..n).any(|j| same_class(&a[i], &a[j]) && !same_class(&b[i], &b[j])));

    cond3a || cond3b
}

/// only to tell the two conflicts apart: two operands disagreeing is a
/// symmetric "resolved `'T` to both", while an operand disagreeing with an
/// instantiation has a written end and an inferred end, and the user needs to
/// know which is which. Passed rather than recorded on `Subst`, since nothing
/// downstream compares substitutions -- the symbol is rendered from `ty`/`len`
/// in vector order -- so a provenance field there would be inert.
#[allow(clippy::too_many_arguments)]
pub(super) fn unify_poly_input(
    sig: &PolySig,
    pty: &PolyType,
    slot_ty: Type,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    subst: &mut Subst,
    seeded: &[u32],
) -> Result<(), String> {
    match pty {
        // P7 slice 3b: `pty` is the callee's *declared* input, which a
        // body-only marker never reaches.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        PolyType::Concrete(t) => {
            if *t != slot_ty {
                return Err(type_mismatch_error(ctx, span, name, *t, slot_ty));
            }
        }
        PolyType::Var(v) => {
            if let Some(prev) = subst.ty_of(*v) {
                if prev != slot_ty {
                    let var = &sig.ty_var_names[*v as usize];
                    return Err(match seeded.contains(v) {
                        true => explicit_instantiation_conflict_error(
                            ctx, span, name, var, prev, slot_ty,
                        ),
                        false => poly_var_conflict_error(ctx, span, name, var, prev, slot_ty),
                    });
                }
            } else {
                // Inserted in `v`-sorted position, not push order: `Subst`'s
                // own doc comment says it is kept sorted so the mangled
                // symbol is deterministic, but a caller processing inputs
                // out of declared order (D8's literal deferral, the
                // quotation two-pass split) would otherwise push out of
                // order and mint a second monomorph for the same theta.
                let pos = subst.ty.partition_point(|(id, _)| *id < *v);
                subst.ty.insert(pos, (*v, slot_ty));
            }
        }
        PolyType::Array(elem, len) => {
            let Type::Array(id, _) = slot_ty else {
                return Err(poly_array_expected_error(ctx, span, name, slot_ty));
            };
            let (elem_ty, count) = (arrays[id.index()].element, arrays[id.index()].count);
            unify_poly_input(
                sig, elem, elem_ty, name, span, ctx, arrays, cells, refs, subst, seeded,
            )?;
            match len {
                Len::Concrete(k) => {
                    if *k != count {
                        return Err(poly_array_expected_error(ctx, span, name, slot_ty));
                    }
                }
                Len::Var(ln) => {
                    if let Some(prev) = subst.len_of(*ln) {
                        if prev != count {
                            return Err(poly_len_conflict_error(
                                ctx,
                                span,
                                name,
                                &sig.len_var_names[*ln as usize],
                                prev,
                                count,
                            ));
                        }
                    } else {
                        subst.len.push((*ln, count));
                    }
                }
            }
        }
        // Slice 6a (R6): a declared quotation parameter unifies against a
        // concrete quotation slot by matching rows pointwise, binding any
        // variable a row mentions (`[ 'T -- ]` against `[ i64 -- ]` binds
        // `'T = i64`). Equal arity is required on both sides; else it is a
        // located mismatch, never a silent bind.
        PolyType::Quotation(ins, outs, _, _, _) => {
            // Slice 10a (R1): accept a `~` slot as well as an ordinary
            // quotation slot (accessor), so a declared quotation parameter
            // unifies against either. Slice 10a (R10): the mismatch
            // messages below render the declared `PolyType` (`pty`) itself
            // through `poly_type_str`, rather than fabricating a `Type` --
            // `Type::Quotation`'s `QuotEffect` has no row field to hold R7's
            // row, so an expected type nobody wrote (e.g. `[ -- ]`) or a
            // rendering that silently drops the row are both avoided by
            // never building one.
            let Some(eff) = crate::ast::is_quotation_type(slot_ty) else {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &slot_ty.to_string(),
                ));
            };
            // Slice 10a (R8): the row is a separate field, never a slot in
            // `ins`/`outs`, so this arity check already excludes it.
            if ins.len() != eff.inputs.len() || outs.len() != eff.outputs.len() {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &slot_ty.to_string(),
                ));
            }
            for (p, c) in ins.iter().zip(&eff.inputs) {
                unify_poly_input(
                    sig, p, *c, name, span, ctx, arrays, cells, refs, subst, seeded,
                )?;
            }
            for (p, c) in outs.iter().zip(&eff.outputs) {
                unify_poly_input(
                    sig, p, *c, name, span, ctx, arrays, cells, refs, subst, seeded,
                )?;
            }
        }
        // Slice 13 (R-A6): a declared `&`-slot unifies only against a
        // reference slot of the *same* mutability -- a shared argument cannot
        // fill a `&!` parameter, nor the reverse -- then recurses on the
        // referent the registry names.
        PolyType::Ref(referent, mutable) => {
            let Some((slot_referent, slot_mutable)) = ref_parts(slot_ty, refs) else {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &slot_ty.to_string(),
                ));
            };
            if slot_mutable != *mutable {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &slot_ty.to_string(),
                ));
            }
            unify_poly_input(
                sig,
                referent,
                slot_referent,
                name,
                span,
                ctx,
                arrays,
                cells,
                refs,
                subst,
                seeded,
            )?;
        }
        // P7.S3n (R3): the cell twin of the `Ref` arm -- a declared `^`-slot
        // unifies only against a concrete owning cell, then recurses on the
        // payload the registry names. There is no mutability bit to agree on.
        PolyType::OwnedCell(payload) => {
            let Type::OwnedCell(id, _) = slot_ty else {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &slot_ty.to_string(),
                ));
            };
            let slot_payload = cells[id.index()].payload;
            unify_poly_input(
                sig,
                payload,
                slot_payload,
                name,
                span,
                ctx,
                arrays,
                cells,
                refs,
                subst,
                seeded,
            )?;
        }
        // P7 slice 3a phase 2 (R2): a concrete `Type::Struct`/`Type::Enum`
        // slot unifies against a declared `Result['T 'E]`-shaped input by
        // reversing the mint: `struct_instantiation_of`/`enum_instantiation_of`
        // recover the `(header idx, module, concrete args)` the slot's own id
        // was minted from, and each declared argument then unifies
        // positionally against the recovered concrete one (binding `'T`/`'E`
        // the same way an ordinary `Var` arm does). A slot whose id is not any
        // instantiation of this exact header (wrong id, wrong header, or a
        // hand-written concrete type sharing no dedup key at all) is a
        // rendered mismatch, never a panic.
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            name: _,
        } => {
            let Some(cell) = ctx.generics() else {
                return Err(poly_generic_not_yet_groundable_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                ));
            };
            let mismatch = || {
                poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &slot_ty.to_string(),
                )
            };
            let generics = cell.borrow();
            let found = if *is_enum {
                let Type::Enum(id, _) = slot_ty else {
                    return Err(mismatch());
                };
                generics.enum_instantiation_of(id)
            } else {
                let Type::Struct(id, _) = slot_ty else {
                    return Err(mismatch());
                };
                generics.struct_instantiation_of(id)
            };
            let Some((found_idx, found_module, found_args)) = found else {
                return Err(mismatch());
            };
            if found_idx != *idx as usize
                || found_module != *module
                || found_args.len() != args.len()
            {
                return Err(mismatch());
            }
            let found_args = found_args.to_vec();
            drop(generics);
            for (arg_pty, arg_ty) in args.iter().zip(found_args.iter()) {
                unify_poly_input(
                    sig, arg_pty, *arg_ty, name, span, ctx, arrays, cells, refs, subst, seeded,
                )?;
            }
        }
    }
    Ok(())
}

/// P7 slice 3a phase 1: a call site whose declared input/output names a
/// generic type applied to a variable (`Result['T 'E]`), which nothing can
/// yet ground to a concrete monomorph -- that needs the live `GenericTypes`
/// instantiator threaded through check (R2/phase 2). Distinct from an
/// ordinary type mismatch: the shape is legal, just not yet actionable.
pub(super) fn poly_generic_not_yet_groundable_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    ty: &str,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{op}` in {where_} (line {}) names the generic type `{ty}`, which cannot yet be instantiated at a variable-bearing application\n  grounding a generic over its own type variable is not yet implemented",
        span.line
    )
}

/// Slice 10a (R10): `type_mismatch_error`'s twin for a declared mismatch
/// whose expected side has no `Type` to name, taking it as an already-rendered
/// `PolyType` string (`poly_type_str`) instead. A row cannot live in a
/// `Type::Quotation`'s `QuotEffect`, and Slice 13's `PolyType::Ref` has no
/// `RefId` until its referent grounds, so neither can be rendered as a `Type`.
/// The *found* side is rendered too, for the same reason: a poly-body operand
/// (`&>`'s receiver) is a `PolyType` that may never ground to a `Type`.
pub(super) fn poly_rendered_type_mismatch_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    expected: &str,
    found: &str,
) -> String {
    let op = crate::resolve::demangle_call(op);
    format!(
            "error: type mismatch in {} (line {})\n  `{}` expected `{}`, found `{}`\n  note: declared {}",
            ctx.rendered_word(), span.line, op, expected, found, effect_str(ctx.effect()))
}

/// R5: apply the ground `θ` to a declared output `PolyType`, yielding a
/// concrete `Type`. A variable-bearing array folds to a concrete interned
/// array shape. A variable the inputs never bound is an under-determined
/// signature (a located error rather than a panic).
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_subst(
    sig: &PolySig,
    pty: &PolyType,
    subst: &Subst,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<Type, String> {
    match pty {
        PolyType::Concrete(t) => Ok(*t),
        // P7 slice 3b: `pty` is a declared signature slot, which a body-only
        // marker never reaches.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        // P7.S3t (R9): `f[SomeType]` is now the way to ground such a call, and
        // the remedy this names. Not, contra the spec, reachable for the first
        // time: pass 2 grounds a declared *quotation input* through this same
        // walk (P7.S3l), so `q ( [ 'T -- ] 'U -- 'U )` called bare has always
        // landed here and been told `'T` is an output it appears nowhere in.
        // R9 freezes the text, so that misdescription stays an open gap.
        PolyType::Var(v) => subst.ty_of(*v).ok_or_else(|| {
            poly_unbound_output_ty_error(
                ctx,
                span,
                name,
                &sig.ty_var_names[*v as usize],
                sig.ty_var_names.len(),
            )
        }),
        PolyType::Array(elem, len) => {
            let elem_ty = apply_subst(sig, elem, subst, name, span, ctx, arrays, cells, refs)?;
            let count = match len {
                Len::Concrete(k) => *k,
                Len::Var(ln) => subst.len_of(*ln).ok_or_else(|| {
                    poly_unbound_output_error(ctx, span, name, &sig.len_var_names[*ln as usize])
                })?,
            };
            Ok(intern_array_type(arrays, elem_ty, count))
        }
        // Slice 6a (R6): substitute both rows of a declared quotation effect,
        // yielding a concrete `Type::Quotation`. Slice 10a (R9): a row on
        // this `PolyType::Quotation` is left ungrounded here -- splicing a
        // caller region into an *interned* effect would mint one no literal
        // could ever equal; grounding happens at the callee side instead
        // (`check_literal_against_declared_effect`, phase 4).
        PolyType::Quotation(ins, outs, is_inline, _, _) => {
            let mut cins = Vec::with_capacity(ins.len());
            for p in ins {
                cins.push(apply_subst(
                    sig, p, subst, name, span, ctx, arrays, cells, refs,
                )?);
            }
            let mut couts = Vec::with_capacity(outs.len());
            for p in outs {
                couts.push(apply_subst(
                    sig, p, subst, name, span, ctx, arrays, cells, refs,
                )?);
            }
            // Slice 10a (R1): ground a `~` effect to `Type::InlineQuotation`
            // rather than `Type::Quotation`, so the materialization
            // boundaries reject it by type inequality.
            Ok(if *is_inline {
                crate::ast::inline_quotation_type(cins, couts)
            } else {
                crate::ast::quotation_type(cins, couts)
            })
        }
        // Slice 13 (R-A7/D4): grounding the referent is what finally mints a
        // `RefId` -- the shape may be one no call site has interned yet, so
        // this is the interning side of the pair (`subst_polytype`, at
        // lowering, only looks a shape up).
        PolyType::Ref(referent, mutable) => {
            let referent = apply_subst(sig, referent, subst, name, span, ctx, arrays, cells, refs)?;
            Ok(crate::ast::intern_ref_type(refs, referent, *mutable))
        }
        // P7.S3n (R3): the cell twin of the `Ref` arm -- grounding the
        // payload is what mints the `OwnedCellId`, so this is the interning
        // side of the pair `subst_polytype` only looks up.
        PolyType::OwnedCell(payload) => {
            let payload = apply_subst(sig, payload, subst, name, span, ctx, arrays, cells, refs)?;
            Ok(crate::ast::intern_owned_cell_type(cells, payload))
        }
        // P7 slice 3a phase 2 (R2): mint (or find) the ground monomorph
        // through the live instantiator -- the write side of the pair
        // `unify_poly_input`'s `Generic` arm reads. Substituting every
        // argument first (recursively) means a nested variable-bearing
        // argument grounds bottom-up, exactly as `Array`'s element does.
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            name: _,
        } => {
            let Some(cell) = ctx.generics() else {
                return Err(poly_generic_not_yet_groundable_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                ));
            };
            let mut concrete_args = Vec::with_capacity(args.len());
            for a in args {
                concrete_args.push(apply_subst(
                    sig, a, subst, name, span, ctx, arrays, cells, refs,
                )?);
            }
            // P7.S3n (R3): `cells` is a live parameter now, so the
            // instantiation name renders a cell-payload argument against the
            // real registry rather than the empty slice this used to throw
            // away -- which would have panicked once a cell entry existed to
            // look up. P7.S3n (R5): mutable, because the instantiation's own
            // field substitution interns the shapes it grounds.
            let regs = crate::ast::MutRegistries {
                structs: ctx.structs(),
                enums: ctx.enums(),
                arrays,
                cells,
                refs,
            };
            let mut g = cell.borrow_mut();
            Ok(if *is_enum {
                g.instantiate_enum(*idx as usize, &concrete_args, *module, regs)
            } else {
                g.instantiate_struct(*idx as usize, &concrete_args, *module, regs)
            })
        }
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
    format!(
        "error: `{member}` is required by {} in {where_} (line {}, col {})\n  note: the operands at this call match none of their declared shapes: {}",
        joined_with_and(&each),
        span.line,
        span.col,
        joined_with_and(&shapes)
    )
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
    found: &PolyType,
    sig: &PolySig,
) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: `{member}` of `{trait_name}` in {where_} (line {}, col {}) expects `{}`, found `{}`",
        span.line,
        span.col,
        poly_type_str(expected, sig),
        poly_type_str(found, sig),
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

/// P7.S12 (R1.5): a generated enum word constructed with an ungrounded
/// generic argument inside a combinator's own body. The construction's
/// concrete identity depends on the combinator's own type variable, so it
/// varies per splice -- and `CallInst::enum_words` is keyed by `Span` alone
/// (not `(uid, span)`, R1.5's deliberate non-goal), which cannot hold two
/// splices' distinct resolutions for the one body span this call sits at. A
/// located restriction rather than a silent last-write-wins collision.
pub(super) fn poly_combinator_generic_enum_construction_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
    header: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{word}` in {where_} (line {}) constructs `{header}` with a type this combinator's own splice determines\n  a generic enum constructed inside a combinator body is not yet supported: each splice would need its own resolution, and none is recorded",
        span.line
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

/// Slice 13 (E3/D6): `&>`/`&!>` on a generic-length array (`array['T 'N]`) -- the
/// element cannot be statically bounds-checked without a known count.
pub(super) fn poly_generic_length_index_error(ctx: &Ctx, span: Span, len_var: &str) -> String {
    let where_ = ctx.rendered_word();
    format!(
        "error: cannot index a generic-length array in {where_} (line {}, col {})\n  the array's length is the type variable `{len_var}`, so its element cannot be statically bounds-checked; index a concrete-length array (`array['T 4]`), or use a fixed length in this word's signature",
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
    let note = match sig.len_var_names.is_empty() && sig.row_var_names.is_empty() {
        true => String::new(),
        false => "\n  note: a length (`'N`) or row (`..s`) variable is not named by an explicit instantiation; only type variables are".to_string(),
    };
    format!(
        "error: `{callee}` (line {}) declares {declared} but was given {given} type argument{plural}{note}",
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

/// Render a `PolyType` for a diagnostic: a variable by its declared spelling,
/// a concrete type by its name, an array structurally.
pub(crate) fn poly_type_str(pt: &PolyType, sig: &PolySig) -> String {
    match pt {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => sig.ty_var_names[*v as usize].clone(),
        // P7 slice 3b (R2): no effect to render (that is the point of the
        // marker), so it renders as what it is.
        PolyType::QuotLit => "a quotation literal".to_string(),
        PolyType::Array(elem, len) => {
            let l = match len {
                Len::Concrete(n) => n.to_string(),
                Len::Var(id) => sig.len_var_names[*id as usize].clone(),
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
                    parts.push(sig.row_var_names[v as usize].clone());
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
        PolyType::Generic { name, args, .. } => {
            let args: Vec<String> = args.iter().map(|a| poly_type_str(a, sig)).collect();
            format!("{name}[{}]", args.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ArrayDecl, ArrayId, OwnedCellDecl};
    use crate::lexer::lex;

    /// P7.S3k: the callee-side walk context for a fixture that drives
    /// `poly_term`/`poly_walk_arms` directly. The registry is empty, so no
    /// cross-call arm can fire and no record survives -- both of which every
    /// one of those fixtures wants. A macro rather than a function because
    /// `CrossCtx` borrows two temporaries, which only live as long as the
    /// statement they are written in.
    macro_rules! scratch_cross {
        () => {
            &mut CrossCtx {
                env: &PolyEnv::new(),
                calls: &mut Vec::new(),
            }
        };
    }

    fn check_src(src: &str) -> Result<(), String> {
        checked_like_a_build(src).map(|_| ())
    }

    /// A source checked the way `driver::assemble_module` checks one: the
    /// declaration checks first (P7.S3e -- `check_impl_decls` is what resolves
    /// each `impl:` binding to the word it names, and no call site can resolve
    /// an obligation without it, which `parse_with_core` now runs for every
    /// caller), then the body/call-site pass, returning the checked module
    /// alongside what R17's pre-pass recorded.
    fn checked_like_a_build(src: &str) -> Result<(Module, Vec<WordObligations>), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        let recorded = super::super::check_module(&mut module)?;
        Ok((module, recorded))
    }

    /// A source checked the way a real build checks one, *including* the
    /// per-module name mangling every build applies even to a single file
    /// (`assemble_module`'s `always_mangle`). `check_src` skips
    /// `resolve_modules`, so a call to a `lib/` word arrives under its bare
    /// spelling there -- the test-harness artefact that kept the deleted
    /// six-name comparison carve-out (P7.S3k R7) looking alive. Through here,
    /// `gt` arrives as `gt__mN`, which is what a real call site holds.
    ///
    /// The declaration pre-passes run *before* `resolve_modules`, inside
    /// `parse_with_core`, exactly as `assemble_module` orders them:
    /// `check_impl_decls` resolves each binding by the name the parser's
    /// desugar synthesized, which only agrees with `WordDef::name` pre-mangle.
    fn check_src_mangled(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::resolve::resolve_modules(&mut module, true).unwrap();
        super::super::check_module(&mut module).map(|_| ())
    }

    /// The cross-call records a checked source produced, keyed by the
    /// polymorphic word whose body made them (P7.S3k R2).
    fn cross_calls_of(src: &str) -> HashMap<String, Vec<PolyCrossCall>> {
        let (module, _) = checked_like_a_build(src).expect("the fixture checks");
        module.poly_cross_calls
    }

    /// P7.S3e (R7/R17): the obligations the pre-pass recorded, keyed by the
    /// polymorphic word whose body recorded them.
    fn obligations_of(src: &str) -> HashMap<String, Vec<TraitObligation>> {
        let (_, recorded) = checked_like_a_build(src).expect("the fixture checks");
        recorded
            .into_iter()
            .map(|w| (w.name, w.obligations))
            .collect()
    }

    /// A trait, a concrete implementing word, and the `impl:` binding them --
    /// the preamble every bound-dispatch fixture below needs. `Point` rather
    /// than `i64` because a scalar local has no address to borrow.
    const SHOW: &str = "type: Point x i64 y i64 ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n";

    /// P7.S3e (R7): a bounded body's member call records an obligation --
    /// which trait, which member, which of the word's own type variables --
    /// and no symbol: `'T` is still abstract at this point.
    #[test]
    fn trait_member_call_records_an_obligation() {
        let recorded = obligations_of(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n: main ( -- ) ;\n"
        ));
        let obs = recorded
            .get("shows")
            .expect("the bounded word was pre-passed");
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].var, 0);
        assert_eq!(obs[0].member, "show");
        assert_eq!(obs[0].span.line, 6);
        // Index 1: the pre-seeded `Copy` predicate entry occupies 0, `Show`
        // (declared in this fixture's own source, ahead of the appended
        // `core::cmp`, which declares its own `Ord` after it) is next -- a
        // whole-program `TraitId` is what was recorded, not a per-module or
        // per-word one.
        assert_eq!(obs[0].trait_id, TraitId::from_index(1));
    }

    /// The obligation list is keyed by every non-combinator poly word the
    /// pre-pass reached, not only the ones that recorded something -- an
    /// absent key means "never pre-passed", which is what makes R17's
    /// order-independence claim checkable rather than indistinguishable from
    /// "recorded nothing".
    #[test]
    fn the_prepass_keys_every_noncombinator_poly_word() {
        let recorded = obligations_of(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : ident ( 'U -- 'U ) ;\n\
             : main ( -- ) ;\n"
        ));
        assert_eq!(recorded["ident"], Vec::new());
        assert_eq!(recorded["shows"].len(), 1);
    }

    /// R17/decision 10: the obligation is recorded in both source orders --
    /// the bounded body is reached whether its monomorphic caller is declared
    /// before or after it.
    ///
    /// This does *not* pin the hoist. The map is fully populated by the time
    /// `check_module` returns either way, so relocating the pre-pass to after
    /// the main word loop leaves this test green; that the obligation is
    /// recorded *early enough* only becomes observable in Phase 3, once the
    /// call-site bound loop consumes it. The hoist's own witness is
    /// `check::tests::a_poly_body_diagnostic_precedes_a_monomorphic_one_declared_before_it`.
    #[test]
    fn the_obligation_is_recorded_in_either_declaration_order() {
        let caller_first = format!(
            "{SHOW}: main ( -- ) 1 2 Point |p| &p shows p drop ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n"
        );
        let callee_first = format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
        );
        assert_eq!(obligations_of(&caller_first)["shows"].len(), 1);
        assert_eq!(obligations_of(&callee_first)["shows"].len(), 1);
    }

    /// R17: the pre-pass hoist *replaces* the in-loop `check_poly_body` call
    /// rather than supplementing it (documented at `src/check.rs`, above the
    /// pre-pass loop). A bounded body that also mints a concrete generic
    /// struct instantiation pins the claim directly: if the deleted in-loop
    /// call were mistakenly restored alongside the pre-pass, the body would
    /// be checked twice. (In practice `GenericTypes::instantiate_struct`
    /// dedupes structurally by `(idx, module, args)` across flushes, so a
    /// duplicate check of the *same* body is currently idempotent even under
    /// that mutation -- confirmed by hand -- but this pins the doc's literal
    /// "observed exactly once" claim and would catch a future change to that
    /// dedup key.)
    #[test]
    fn a_generic_struct_referenced_by_a_bounded_body_mints_exactly_once() {
        let src = format!(
            "{SHOW}type: Box['T] val 'T ;\n\
             : shows ['T: Show] ( &'T -- ) show 7 Box drop ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
        );
        let (module, _) = checked_like_a_build(&src).expect("the fixture checks");
        // `Point` (SHOW's own preamble) plus exactly one `Box[i64]`
        // instantiation -- two concrete structs, not three.
        assert_eq!(
            module.structs.len(),
            2,
            "Box[i64] should mint exactly once: {:#?}",
            module.structs.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    /// R7: the member's declared outputs are pushed, with the trait's own type
    /// variable rewritten to the bounded variable being dispatched on.
    #[test]
    fn trait_member_call_pushes_the_declared_outputs() {
        check_src(
            "type: Point x i64 y i64 ;\n\
             trait: Clone['T] : clone ( &'T -- 'T ) ; ;\n\
             impl: Clone for Point\n\
               : clone | p | p drop 1 2 Point ;\n\
             ;\n\
             : cloned ['T: Clone] ( &'T -- 'T ) clone ;\n\
             : main ( -- ) ;\n",
        )
        .expect("the member's `'T` output grounds to the caller's own variable");
    }

    /// The output really is the *member's*, not a pass-through: declaring the
    /// wrong one is a stack-shape mismatch. Asserted on the residual-stack
    /// line, not just on the word name: with dispatch disabled the same
    /// fixture fails as an unknown-word error that names `cloned` too.
    #[test]
    fn trait_member_output_is_the_declared_one() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             trait: Clone['T] : clone ( &'T -- 'T ) ; ;\n\
             impl: Clone for Point\n\
               : clone | p | p drop 1 2 Point ;\n\
             ;\n\
             : cloned ['T: Clone] ( &'T -- ) clone ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("cloned") && err.contains("body leaves `'T`"),
            "{err}"
        );
    }

    /// R7: an operand that does not match the member's declared signature is a
    /// located rejection naming the trait and the member.
    #[test]
    fn trait_member_operand_mismatch_is_located() {
        let err = check_src(&format!(
            "{SHOW}: shows ['T: Show] ( 'T -- ) show ;\n: main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`show` of `Show` in `shows` (line 6, col 30) expects `&'T`, found `'T`"),
            "{err}"
        );
    }

    /// P7.S3p: bound-directed dispatch runs ahead of `env.get`, so a concrete
    /// word of the member's name does not capture the call -- the guard the
    /// S3e review's mis-dispatch counterexample asked for. Pinned here rather
    /// than as a golden: a real build mangles the call site to the concrete
    /// word's own symbol first (S3e R18's ruled outcome), which is what
    /// `tests/phase7_slice3e.rs` covers.
    #[test]
    fn a_member_call_beats_a_concrete_word_of_the_same_name() {
        let recorded = obligations_of(
            "type: Point x i64 y i64 ;\n\
             trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
             impl: Indexable for Point\n\
               : at | p n | n drop p &x @ ;\n\
             ;\n\
             : at ( i64 -- i64 ) 900 add ;\n\
             : uses ['T: Indexable] ( &'T -- i64 ) 0 at ;\n\
             : main ( -- ) ;\n",
        );
        let obs = &recorded["uses"];
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].member, "at");
    }

    /// P7.S3p (ruling 1): the receiver need not be the member's last input.
    /// `at`'s receiver is `inputs[0]`, with an `i64` above it, and the
    /// dispatched variable comes off the *bound* -- pinned by declaring the
    /// bounded variable second (id 1), so the stack slot a top-of-stack read
    /// would have seen is the `i64` operand and no dispatch would happen at
    /// all.
    #[test]
    fn a_non_trailing_receiver_dispatches_on_the_bound_variable() {
        let recorded = obligations_of(
            "type: Point x i64 y i64 ;\n\
             trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
             impl: Indexable for Point\n\
               : at | p n | n drop p &x @ ;\n\
             ;\n\
             : uses ['T: Indexable] ( 'U &'T -- 'U i64 ) 0 at ;\n\
             : main ( -- ) ;\n",
        );
        let obs = &recorded["uses"];
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].member, "at");
        assert_eq!(obs[0].var, 1);
    }

    /// P7.S3p (ruling 2): selection is by member name alone -- a mismatched
    /// operand at the receiver's position is the located member diagnostic,
    /// never a fall-through to ordinary dispatch (which would report `at` as
    /// an unknown word and lose the trait entirely). The bound is on a bare
    /// `'T` where `at` declares `&'T`, so a structural *selection* gate would
    /// decline the candidate here.
    #[test]
    fn a_non_trailing_receiver_mismatch_is_located_not_unknown() {
        let err = check_src(
            "trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
             : uses ['T: Indexable] ( 'T -- i64 ) 0 at ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains(
                "`at` of `Indexable` in `uses` (line 2, col 40) expects `&'T`, found `'T`"
            ),
            "{err}"
        );
    }

    /// `substitute_member_var` witness: every other fixture in this file puts
    /// the bound variable at index 0, identical to the trait member's own
    /// `'T` (also id 0 in its own `PolySig`), so an identity-stubbed rewrite
    /// would pass them all. Here the bound variable (`'T`) is declared
    /// *second*, at id 1, behind an unrelated `'U` at id 0 -- an identity
    /// rewrite would leave `show`'s declared `&'T` input at var 0, which is
    /// `'U` in this signature, not the bound `'T`, and the call would be
    /// rejected as a mismatch it is not.
    #[test]
    fn trait_member_dispatch_rewrites_a_non_first_bound_variable() {
        check_src(&format!(
            "{SHOW}: shows ['T: Show] ( 'U &'T -- 'U ) show ;\n: main ( -- ) ;\n"
        ))
        .expect("show's receiver rewrites to 'T (id 1), not 'U (id 0)");
    }

    /// R12/decision 5: composing two traits that happen to require the same
    /// member name is legal to *declare* -- the rejection belongs to the
    /// ambiguous call, not to a body that never makes one.
    #[test]
    fn two_bounds_sharing_a_member_name_are_legal_to_declare() {
        check_src(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A B] ( &'T -- ) drop ;\n\
             : main ( -- ) ;\n",
        )
        .expect("declaring both bounds is legal without calling the shared member");
    }

    /// R12/decision 5: the ambiguous *call* is the error, naming both traits,
    /// the member, and the bound variable.
    #[test]
    fn ambiguous_trait_member_call_is_rejected() {
        let err = check_src(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A B] ( &'T -- ) t1 ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`t1` is required by both `A` and `B` on 'T (line 3, col 26)"),
            "{err}"
        );
        assert!(
            err.contains(
                "a member required by two of a variable's bounds cannot be called unqualified"
            ),
            "{err}"
        );
    }

    /// P7.S3p (ruling 4, amended): two traits declaring the member on
    /// *different* variables are separated by the operands the call consumes,
    /// the same way S3e's top-of-stack discovery separated them -- `&'U` on
    /// top means `B`'s, and the `&'T` left below means `A`'s. Both dispatch,
    /// each recording its own variable.
    #[test]
    fn cross_variable_candidates_are_separated_by_the_operands() {
        let recorded = obligations_of(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A 'U: B] ( &'T &'U -- ) t1 t1 ;\n\
             : main ( -- ) ;\n",
        );
        let vars: Vec<u32> = recorded["f"].iter().map(|o| o.var).collect();
        assert_eq!(vars, vec![1, 0]);
    }

    /// P7.S3p (ruling 4): a candidate declaring more inputs than the stack
    /// holds cannot fit, so it does not compete -- `B`'s three-input `t1` is
    /// out, leaving `A`'s the unique fit. The length check is not a narrowing
    /// nicety: the window base is `stack.len() - inputs.len()`, so without it a
    /// candidate wider than the stack underflows and the compiler panics on a
    /// program that otherwise checks fine.
    #[test]
    fn a_candidate_wider_than_the_stack_does_not_fit() {
        let recorded = obligations_of(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T i64 i64 -- ) ; ;\n\
             : f ['U: B 'T: A] ( &'U &'T -- ) t1 drop ;\n\
             : main ( -- ) ;\n",
        );
        let vars: Vec<u32> = recorded["f"].iter().map(|o| o.var).collect();
        assert_eq!(vars, vec![1]);
    }

    /// P7.S3p (ruling 4): a third bound on another variable does not turn the
    /// same-variable ambiguity into a resolvable call -- `'T`'s two candidates
    /// fit the operands equally, so the fit is not unique and the call is
    /// still rejected. Without the uniqueness requirement one of the two is
    /// picked by bound declaration order.
    #[test]
    fn a_same_variable_ambiguity_survives_a_bound_on_another_variable() {
        let err = check_src(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             trait: C['T] : t1 ( &'T -- ) ; ;\n\
             : f ['U: C 'T: A B] ( &'U &'T -- ) t1 drop ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`t1` is required by `C` on 'U, `A` on 'T and `B` on 'T"),
            "{err}"
        );
    }

    /// P7.S3p (ruling 4, amended): when the operands fit *no* candidate,
    /// nothing at the call site picks -- but this is not the same failure as
    /// an ambiguous call: no module qualifier would fix it, since every
    /// candidate's declared operands already disagree with the stack. The
    /// diagnostic says so instead of the "cannot be called unqualified" note,
    /// which is only true when qualifying would actually help.
    #[test]
    fn a_cross_variable_member_call_fitting_no_candidate_names_the_operand_mismatch() {
        let err = check_src(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A 'U: B] ( &'T &'U i64 -- ) t1 ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`t1` is required by `A` on 'T and `B` on 'U in `f` (line 3, col 38)"),
            "{err}"
        );
        assert!(
            err.contains(
                "the operands at this call match none of their declared shapes: \
                 `A` on 'T expects `&'T` and `B` on 'U expects `&'U`"
            ),
            "{err}"
        );
        assert!(!err.contains("cannot be called unqualified"), "{err}");
    }

    /// P7.S3p (ruling 4, amended): the no-fit path is reachable for a member
    /// whose receiver is *trailing*, a shape S3e already admitted, where it
    /// used to report the single declared shape it checked against. It must
    /// stay as actionable now that there are several: the note names every
    /// candidate's substituted input list, and the message keeps the enclosing
    /// word.
    #[test]
    fn a_no_fit_call_on_a_trailing_receiver_member_names_every_declared_shape() {
        let err = check_src(
            "trait: A['T] : t1 ( i64 &'T -- ) ; ;\n\
             : f ['T: A 'U: A] ( &'T &'U -- ) t1 ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`t1` is required by `A` on 'T and `A` on 'U in `f` (line 2, col 34)"),
            "{err}"
        );
        assert!(
            err.contains(
                "none of their declared shapes: \
                 `A` on 'T expects `i64 &'T` and `A` on 'U expects `i64 &'U`"
            ),
            "{err}"
        );
    }

    /// P7.S3p (ruling 4): position is not a disambiguator either. `A`'s `t1`
    /// takes the receiver last, `B`'s takes it first, both on one variable --
    /// still the single-variable ambiguity, rendered exactly as the
    /// same-position case.
    #[test]
    fn a_multi_position_duplicate_member_is_the_single_variable_ambiguity() {
        let err = check_src(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             trait: B['T] : t1 ( &'T i64 -- ) ; ;\n\
             : f ['T: A B] ( &'T -- ) t1 ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`t1` is required by both `A` and `B` on 'T (line 3, col 26)"),
            "{err}"
        );
    }

    /// One trait named twice on one variable is not an ambiguity with itself:
    /// `parse_capabilities` admits the repeat, and the two `Bound::User`
    /// entries would otherwise reach the ambiguity arm naming `A` twice.
    #[test]
    fn a_repeated_bound_is_not_ambiguous_with_itself() {
        let recorded = obligations_of(
            "trait: A['T] : t1 ( &'T -- ) ; ;\n\
             : f ['T: A A] ( &'T -- ) t1 ;\n\
             : main ( -- ) ;\n",
        );
        assert_eq!(recorded["f"].len(), 1);
        assert_eq!(recorded["f"][0].member, "t1");
    }

    /// P7.S3o Phase 3: a bare member call (`show`) inside a bounded inline
    /// combinator now resolves at the splice site via dispatch injection into
    /// `check_terms_relaxed`. The standalone check (i64 stand-in) accounts for
    /// the member's stack effect without requiring an `impl: Show for i64`, and
    /// the actual dispatch happens at each real splice site where θ is
    /// concrete. This test now compiles successfully — the gate rejection is
    /// gone, and the bare member is no longer an "unknown word".
    #[test]
    fn user_bound_on_a_combinator_compiles() {
        check_src(&format!(
            "{SHOW}: shows inline ['T: Show] ( &'T -- ) show ;\n: main ( -- ) ;\n"
        ))
        .expect("a bounded inline combinator calling a bare member compiles");
    }

    /// R10 barrier 1: a plain (non-builtin) member name over a *bare* type
    /// variable. Before this slice such an operand reached
    /// `poly_var_to_concrete_error` unconditionally, so bound-directed
    /// dispatch takes nothing an ordinary `env` lookup could have had.
    /// (Barrier 3, a `Ref`-to-variable operand, is
    /// `trait_member_call_records_an_obligation` above; the coexistence half
    /// of R10 needs real name mangling and lives in `driver`'s tests.)
    #[test]
    fn bound_dispatch_on_a_bare_variable_receiver() {
        let recorded = obligations_of(
            "trait: Eat['T] : eat ( 'T -- ) ; ;\n\
             : eats ['T: Eat] ( 'T -- ) eat ;\n\
             : main ( -- ) ;\n",
        );
        assert_eq!(recorded["eats"].len(), 1);
        assert_eq!(recorded["eats"][0].member, "eat");
    }

    /// R10 barrier 2: a member spelled as a name the dispatch cascade
    /// intercepts ahead of bound-directed dispatch. Before
    /// `poly_trait_member_call` moved to the front of `poly_call_term`, this
    /// member was unreachable: the comparisons block (`matches!(name, "eq" |
    /// ...)`) intercepted it first and demanded an `Ord` bound the trait never
    /// declared. `main` carries R10's coexistence half for this barrier: the
    /// builtin still wins a concrete receiver.
    ///
    /// The six surface comparisons are the whole of that barrier now. P7.S3r
    /// (R4) rejects a member spelled as a *name-dispatched* builtin at the
    /// `trait:` declaration, so the operator-spelled sibling of this fixture (a
    /// `Sum` trait with an `add` member) is no longer declarable -- and it never
    /// exercised the partition anyway, since no dispatch-cascade arm matches
    /// `add`.
    #[test]
    fn bound_dispatch_reaches_a_member_named_after_an_intercepting_builtin() {
        let recorded = obligations_of(
            "trait: Eq['T] : eq ( 'T 'T -- i64 ) ; ;\n\
             : eqs ['T: Eq] ( 'T 'T -- i64 ) eq ;\n\
             : main ( -- ) 1 2 eq drop ;\n",
        );
        assert_eq!(recorded["eqs"].len(), 1);
        assert_eq!(recorded["eqs"][0].member, "eq");
    }

    /// Two types, both satisfying one trait through their own `impl:` -- the
    /// preamble the call-site resolution tests need (R8). A `shows` declared
    /// after it lands on line 10, and its `show` call is the obligation's
    /// span.
    const TWO_SHOWS: &str = "type: Point x i64 y i64 ;\n\
         type: Blip n i64 ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n\
         impl: Show for Blip\n\
           : show | b | b drop ;\n\
         ;\n";

    /// P7.S3e (R8/R9): the load-bearing new mechanism, read directly rather
    /// than through a golden (a bound-directed call does not lower until
    /// Phase 4). The call site resolves the obligation its callee's body
    /// recorded against its own theta and records the implementing word's
    /// lowering symbol, keyed by the *body* span that dispatched it -- never
    /// the caller's.
    #[test]
    fn a_satisfied_bound_resolves_to_the_implementing_words_symbol() {
        let (module, _) = checked_like_a_build(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
        ))
        .expect("the fixture checks");
        let inst = module
            .instantiations
            .values()
            .find(|i| i.callee == "shows")
            .expect("the call site recorded an instantiation");
        let resolved: Vec<(u32, &str)> = inst
            .trait_calls
            .iter()
            .map(|(span, symbol)| (span.line, symbol.as_str()))
            .collect();
        assert_eq!(resolved, vec![(6, "show;Show;0;Point")]);
    }

    /// R8: two instantiations of one bounded word resolve to two distinct
    /// symbols -- the same body span, under each instantiation's own
    /// `CallInst`, which is what "per-instantiation" means.
    #[test]
    fn two_instantiations_resolve_to_two_distinct_symbols() {
        let (module, _) = checked_like_a_build(&format!(
            "{TWO_SHOWS}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop 7 Blip |b| &b shows b drop ;\n"
        ))
        .expect("the fixture checks");
        let mut resolved: Vec<(String, u32, String)> = module
            .instantiations
            .values()
            .filter(|i| i.callee == "shows")
            .flat_map(|i| {
                let ty = i.subst.ty_of(0).expect("'T is grounded");
                i.trait_calls
                    .iter()
                    .map(move |(span, symbol)| (ty.name().to_string(), span.line, symbol.clone()))
            })
            .collect();
        resolved.sort();
        assert_eq!(
            resolved,
            vec![
                ("Blip".to_string(), 10, "show;Show;0;Blip".to_string()),
                ("Point".to_string(), 10, "show;Show;0;Point".to_string()),
            ]
        );
    }

    /// R8: which member the obligation names selects the binding. A trait with
    /// two members, a body calling only the second, and two distinct member
    /// bodies: resolving by position rather than by member name would
    /// dispatch `hash` to `eq`'s synthesized word.
    #[test]
    fn the_obligations_member_name_selects_the_binding() {
        let (module, _) = checked_like_a_build(
            "type: Point x i64 y i64 ;\n\
             trait: Eq['T] : eq ( &'T &'T -- i64 ) ; : hash ( &'T -- i64 ) ; ;\n\
             impl: Eq for Point\n\
               : eq | a b | a drop b drop 1 ;\n\
               : hash | p | p drop 7 ;\n\
             ;\n\
             : hashes ['T: Eq] ( &'T -- i64 ) hash ;\n\
             : main ( -- ) 1 2 Point |p| &p hashes drop p drop ;\n",
        )
        .expect("the fixture checks");
        let resolved: Vec<&String> = module
            .instantiations
            .values()
            .filter(|i| i.callee == "hashes")
            .flat_map(|i| i.trait_calls.values())
            .collect();
        assert_eq!(resolved, vec!["hash;Eq;0;Point"]);
    }

    /// R8: a polymorphic *overload set* -- two bounded words sharing one name,
    /// which is legal since their declared inputs differ. Each call site must
    /// read back its own callee's obligations: they are recorded per
    /// `(name, signature)`, and a name-keyed lookup would hand both call sites
    /// the first candidate's obligation, resolving a body span belonging to a
    /// word that was never called.
    #[test]
    fn each_overload_of_one_name_resolves_its_own_bodys_obligation() {
        let (module, _) = checked_like_a_build(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : shows ['T: Show] ( &'T i64 -- ) drop show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows &p 3 shows p drop ;\n"
        ))
        .expect("the fixture checks");
        let mut sites: Vec<(u32, Vec<u32>)> = module
            .instantiations
            .iter()
            .filter(|(_, i)| i.callee == "shows")
            .map(|(span, i)| {
                let mut lines: Vec<u32> = i.trait_calls.keys().map(|s| s.line).collect();
                lines.sort();
                (span.col, lines)
            })
            .collect();
        sites.sort();
        // The one-input `shows` is called first, so it holds the lower column;
        // its `show` is on line 6, the two-input one's on line 7.
        assert_eq!(sites.len(), 2, "{sites:?}");
        assert_eq!(sites[0].1, vec![6], "{sites:?}");
        assert_eq!(sites[1].1, vec![7], "{sites:?}");
    }

    /// R8: two distinct bound variables on one word, each obligated to a
    /// different trait, both resolved in one call -- the trait axis of
    /// `resolve_user_bound`'s `.filter(|o| o.trait_id == trait_id && o.var ==
    /// v)`. The two obligations differ in *both* conjuncts here, so this
    /// fixture alone does not discriminate: the variable axis is pinned by
    /// `one_trait_on_two_variables_resolves_each_span_against_its_own_theta`
    /// and the trait axis by
    /// `two_traits_on_one_variable_resolve_against_their_own_impl`.
    #[test]
    fn two_bounds_on_distinct_variables_each_resolve_their_own_obligation() {
        let (module, _) = checked_like_a_build(
            "type: PA n i64 ;\n\
             type: PB n i64 ;\n\
             trait: A['T] : ta ( &'T -- ) ; ;\n\
             trait: B['T] : tb ( &'T -- ) ; ;\n\
             impl: A for PA\n\
               : ta | p | p drop ;\n\
             ;\n\
             impl: B for PB\n\
               : tb | p | p drop ;\n\
             ;\n\
             : f ['T: A 'U: B] ( &'T &'U -- ) tb ta ;\n\
             : main ( -- ) 1 PA |a| 1 PB |b| &a &b f a drop b drop ;\n",
        )
        .expect("the fixture checks");
        let inst = module
            .instantiations
            .values()
            .find(|i| i.callee == "f")
            .expect("the call site recorded an instantiation");
        let mut resolved: Vec<&str> = inst.trait_calls.values().map(String::as_str).collect();
        resolved.sort();
        assert_eq!(resolved, vec!["ta;A;0;PA", "tb;B;0;PB"]);
    }

    /// R8: one trait, two bound variables, instantiated at two types that
    /// each implement it. Both obligations name the same trait, so
    /// `o.var == v` is the only conjunct separating them: without it each
    /// bound's loop resolves *both* body spans against its own theta, and the
    /// `'T` dispatch silently gets `'U`'s implementing word.
    #[test]
    fn one_trait_on_two_variables_resolves_each_span_against_its_own_theta() {
        let (module, _) = checked_like_a_build(
            "type: PA n i64 ;\n\
             type: PB n i64 ;\n\
             trait: A['T] : ta ( &'T -- ) ; ;\n\
             impl: A for PA\n\
               : ta | p | p drop ;\n\
             ;\n\
             impl: A for PB\n\
               : ta | p | p drop ;\n\
             ;\n\
             : f ['T: A 'U: A] ( &'T &'U -- ) ta ta ;\n\
             : main ( -- ) 1 PA |a| 1 PB |b| &a &b f a drop b drop ;\n",
        )
        .expect("the fixture checks");
        let inst = module
            .instantiations
            .values()
            .find(|i| i.callee == "f")
            .expect("the call site recorded an instantiation");
        let mut resolved: Vec<(u32, &str)> = inst
            .trait_calls
            .iter()
            .map(|(span, symbol)| (span.col, symbol.as_str()))
            .collect();
        resolved.sort();
        // The body's first `ta` (col 26) consumes the top input, `'U` = `PB`;
        // the second (col 29) consumes `'T` = `PA`.
        assert_eq!(resolved, vec![(34, "ta;A;0;PB"), (37, "ta;A;0;PA")]);
    }

    /// R8: two traits bounding *one* variable, both implemented for the type
    /// it is instantiated at. Each bound's loop must see only its own trait's
    /// obligation and only its own trait's `impl:`: dropping either
    /// `trait_id` comparison (the obligation filter's or the registry
    /// lookup's) makes one loop hunt for a member the other trait's impl
    /// binds, and this legal program is rejected by R17's
    /// internal-consistency error.
    #[test]
    fn two_traits_on_one_variable_resolve_against_their_own_impl() {
        let (module, _) = checked_like_a_build(
            "type: PA n i64 ;\n\
             trait: A['T] : ta ( &'T -- ) ; ;\n\
             trait: B['T] : tb ( &'T -- ) ; ;\n\
             impl: A for PA\n\
               : ta | p | p drop ;\n\
             ;\n\
             impl: B for PA\n\
               : tb | p | p drop ;\n\
             ;\n\
             : f ['T: A B] ( &'T &'T -- ) tb ta ;\n\
             : main ( -- ) 1 PA |a| &a &a f a drop ;\n",
        )
        .expect("the fixture checks");
        let inst = module
            .instantiations
            .values()
            .find(|i| i.callee == "f")
            .expect("the call site recorded an instantiation");
        let mut resolved: Vec<(u32, &str)> = inst
            .trait_calls
            .iter()
            .map(|(span, symbol)| (span.col, symbol.as_str()))
            .collect();
        resolved.sort();
        assert_eq!(resolved, vec![(30, "tb;B;0;PA"), (33, "ta;A;0;PA")]);
    }

    /// R8: a polymorphic combinator's body calling a bounded poly word. The
    /// combinator itself is checked standalone and records nothing that
    /// survives, but the bound it dispatches through is real -- so that path
    /// must be handed the whole-program trait/impl tables, not the scratch
    /// ones (whose trait table a user `TraitId` indexes past the end).
    #[test]
    fn a_bounded_call_inside_a_combinator_body_resolves() {
        checked_like_a_build(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : appq inline ( &Point ~[ -- ] -- ) | f | f call shows ;\n\
             : main ( -- ) 1 2 Point |p| &p ~[ ] appq p drop ;\n"
        ))
        .expect("a satisfied bound dispatched from a combinator body checks");
    }

    /// R8/R9: `CallInst::trait_calls` is a pure function of `(callee, theta)`
    /// -- its keys are the callee's own body spans, never the caller's -- so
    /// two call sites at one instantiation record identical maps. Phase 4's
    /// symbol-dedup step reads whichever it reaches first, which is only
    /// sound if that holds.
    #[test]
    fn two_call_sites_at_one_instantiation_record_identical_maps() {
        let (module, _) = checked_like_a_build(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows &p shows p drop ;\n"
        ))
        .expect("the fixture checks");
        let maps: Vec<&HashMap<Span, String>> = module
            .instantiations
            .values()
            .filter(|i| i.callee == "shows")
            .map(|i| &i.trait_calls)
            .collect();
        assert_eq!(maps.len(), 2, "two call sites");
        assert_eq!(maps[0], maps[1]);
        assert_eq!(
            maps[0].values().collect::<Vec<_>>(),
            vec!["show;Show;0;Point"]
        );
    }

    /// R8: the concrete type a bounded variable was instantiated with has no
    /// `impl:` for the trait the bound names.
    #[test]
    fn an_unsatisfied_user_bound_names_the_missing_member_signature() {
        let err = check_src(&format!(
            "{SHOW}type: Blip n i64 ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 Blip |b| &b shows b drop ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains(
                "error: cannot instantiate `'T` of `shows` with `Blip` in `main` (line 8, col 29)"
            ),
            "{err}"
        );
        assert!(
            err.contains("`Blip` does not satisfy `Show`: no `( &Blip -- )` found"),
            "{err}"
        );
    }

    /// R8: every member's grounded signature is listed, not only the first --
    /// an unsatisfied bound says what an `impl:` would have to provide in
    /// full.
    #[test]
    fn an_unsatisfied_multi_member_bound_lists_every_member_signature() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             trait: Eq['T] : eq ( &'T &'T -- i64 ) ; : hash ( &'T -- i64 ) ; ;\n\
             : eqs ['T: Eq] ( &'T &'T -- i64 ) eq ;\n\
             : main ( -- ) 1 2 Point |p| &p &p eqs drop p drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains(
                "`Point` does not satisfy `Eq`: no `( &Point &Point -- i64 )`, `( &Point -- i64 )` found"
            ),
            "{err}"
        );
    }

    /// R17: the backstop for a satisfied bound whose recorded obligation
    /// resolves to nothing. `check_impl_decls` rejects an `impl:` binding no
    /// word for a required member at its own declaration site, so the only way
    /// into this state is to skip that check -- which is what this asserts:
    /// the fixture that resolves cleanly in
    /// `a_satisfied_bound_resolves_to_the_implementing_words_symbol` becomes a
    /// located error, not a silently dropped call, when the two disagree.
    ///
    /// P7.S3s (review fix): `parse_with_core` now runs `check_impl_decls` on
    /// every caller's module (needed so `core::cmp`'s own `impl: Ord` blocks
    /// resolve `cmp`), so this fixture's `impl: Show for Point` -- which does
    /// bind `show` correctly -- resolves cleanly through that same call, the
    /// same as it would in a real build. Reaching R17's backstop deliberately
    /// now means undoing just that one resolution afterwards, on the `Show`
    /// impl alone, rather than skipping the whole-module check `core::cmp`
    /// still needs.
    #[test]
    fn an_unresolvable_obligation_on_a_satisfied_bound_is_a_located_error() {
        let src = format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- ) 1 2 Point |p| &p shows p drop ;\n"
        );
        let tokens = lex(&src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        let show_impl = module
            .impls
            .iter_mut()
            .find(|i| {
                i.target.pattern
                    == crate::ast::PolyType::Concrete(crate::ast::Type::Struct(
                        StructId::from_index(0),
                        "Point",
                    ))
            })
            .expect("the fixture's `impl: Show for Point` parsed");
        assert!(
            !show_impl.resolved.is_empty(),
            "parse_with_core's check_impl_decls should have resolved `show`"
        );
        show_impl.resolved.clear();
        let err = check(&mut module).unwrap_err();
        assert_eq!(
            err,
            "error: `impl: Show for Point` binds no word for member `show`, dispatched at line 6, col 31 in the body of `shows` (instantiated at line 7, col 32 in `main`)"
        );
    }

    // P7.S4b (R12): unit tests for the factored `find_bound_impl`, the
    // `candidate_bounds_discharge` filter, and the `bound_cycle_error`
    // cycle detector.

    /// R6: a bounded generic impl's `where`-clause bound discharges at a
    /// concrete instantiation via recursive `find_bound_impl` — the element
    /// type has its own concrete impl, so the candidate is kept and the program
    /// compiles.
    #[test]
    fn find_bound_impl_recursive_discharge_succeeds() {
        let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Print['T] : print ( &'T -- ) ; ;\n\
             impl: Print for Point\n\
               : print | p | p drop ;\n\
             ;\n\
             impl: Show for array['T 4] where 'T: Print\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               p 4 fill |arr|\n\
               &arr shows\n\
               arr drop\n\
               p drop\n\
             ;\n";
        check_src(src).expect("bounded generic impl should compile when the bound discharges");
    }

    /// R6 edge: a bounded generic impl whose `where`-clause bound fails to
    /// discharge (no matching impl for the concrete element type) is excluded
    /// from the candidate set, leaving no candidate and producing the
    /// unsatisfied-bound error.
    #[test]
    fn find_bound_impl_recursive_discharge_fails_when_bound_unmet() {
        let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Print['T] : print ( &'T -- ) ; ;\n\
             impl: Show for array['T 4] where 'T: Print\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               p 4 fill |arr|\n\
               &arr shows\n\
               arr drop\n\
               p drop\n\
             ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("cannot instantiate `'T` of `shows` with `array[Point 4]`"),
            "{err}"
        );
        assert!(
            err.contains("`array[Point 4]` does not satisfy `Show`"),
            "{err}"
        );
    }

    /// R7: a self-referential bound cycle — `impl: Show for 'T where 'T: Show`
    /// — is a located error at the impl declaration, not a stack overflow or
    /// hang.
    #[test]
    fn bound_cycle_error_self_referential_impl_is_located_error() {
        let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for 'T where 'T: Show\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               &p shows\n\
               p drop\n\
             ;\n";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("bound-discharge cycle"), "{err}");
        assert!(
            err.contains("`impl: Show for 'T` requires `Show for Point`"),
            "{err}"
        );
    }

    /// R7: a transitive cycle — `impl: A for 'T where 'T: B` and `impl: B for
    /// 'T where 'T: A` — is a located error, not a hang.
    #[test]
    fn bound_cycle_error_transitive_cycle_is_located_error() {
        let src = "type: Point x i64 y i64 ;\n\
             trait: A['T] : a ( &'T -- ) ; ;\n\
             trait: B['T] : b ( &'T -- ) ; ;\n\
             impl: A for 'T where 'T: B\n\
               : a | x | x drop ;\n\
             ;\n\
             impl: B for 'T where 'T: A\n\
               : b | x | x drop ;\n\
             ;\n\
             : calls_a ['T: A] ( &'T -- ) a ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               &p calls_a\n\
               p drop\n\
             ;\n";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("bound-discharge cycle"), "{err}");
    }

    /// R6: a `Copy` bound on a candidate's own variable is checked during
    /// candidate filtering — a linear element type fails the `Copy` bound,
    /// excluding the candidate.
    #[test]
    fn candidate_bounds_discharge_copy_bound_excludes_linear_type() {
        let src = "type: Spy tag i64 ;\n\
             : drop ( Spy -- ) | s | s Spy> drop ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for 'T where 'T: Copy\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               0 Spy |s|\n\
               &s shows\n\
               s drop\n\
             ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("cannot instantiate `'T` of `shows` with `Spy`"),
            "{err}"
        );
    }

    /// R7: two independent calls to a bounded word both resolve the same
    /// `(TraitId, Type)` via `find_bound_impl`, and neither interferes with
    /// the other — the path-scoped visited-set is fresh for each top-level
    /// `resolve_user_bound` call, so no false-positive cycle.
    #[test]
    fn bound_cycle_no_false_positive_on_independent_resolutions() {
        let src = "type: Point x i64 y i64 ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Print['T] : print ( &'T -- ) ; ;\n\
             impl: Print for Point\n\
               : print | p | p drop ;\n\
             ;\n\
             impl: Show for array['T 4] where 'T: Print\n\
               : show | a | a drop ;\n\
             ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : main ( -- )\n\
               1 2 Point |p|\n\
               p 4 fill |arr|\n\
               &arr shows\n\
               &arr shows\n\
               arr drop\n\
               p drop\n\
             ;\n";
        check_src(src).expect("two independent resolutions should not be a cycle");
    }

    // A one-field struct with a `drop` overload: linear for the same reason any
    // resource is, used to force the `Copy`-bound failure (X5).
    const SPY: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | s Spy> drop ;\n";
    /// D3's leaf resource: one field, a `drop` override implemented exactly
    /// as `examples/resources.sth`'s `Fd` (extracting the field via `Fd>`
    /// inside `drop`'s own body -- exempted, since a word literally named
    /// `drop` can only be the recognized override for the struct its declared
    /// effect names).
    const FD_DEF: &str = "type: Fd n i64 ;\n: drop ( Fd -- ) | h | h Fd> drop ;\n";
    fn probe_word() -> WordDef {
        crate::test_support::bare_word("probe", 0)
    }
    /// `probe_word`'s `Ctx`. Separate from `probe_word` because `word_ctx`
    /// borrows the `WordDef`, so the caller has to own it.
    fn probe_ctx(word: &WordDef) -> Ctx<'_> {
        word_ctx(word, &[], &[], &[], None, &CombinatorIndex::new(), None)
    }
    /// A signature over no variables, for the unit tests that drive
    /// `poly_term` directly rather than through a source program.
    fn bare_sig() -> PolySig {
        PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: Vec::new(),
            len_var_names: Vec::new(),
            row_var_names: Vec::new(),
        }
    }
    /// A checked module, for the tests that read a type fact back out of the
    /// registries rather than only asserting a diagnostic.
    fn checked_module(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }
    /// P7 slice 3a phase 2 (R2/R4): the anti-placebo test for asymmetric
    /// instantiation -- `unify_poly_input`'s `Generic` arm must bind each
    /// header argument *positionally*, not just check that some binding
    /// exists. A poly word consuming `Result['T 'E]` is called at both
    /// `Result[i64 str]` and its swap `Result[str i64]`; if the arm collapsed
    /// positional order (bound `'T`/`'E` from the wrong slot, or from a
    /// symmetric key that cannot tell the two apart), one of the two calls
    /// would bind `'T`/`'E` to the wrong concrete type, and this checks the
    /// whole program still type-checks and runs.
    #[test]
    fn unify_poly_generic_binds_arguments_positionally() {
        // `show_is`/`show_si`'s own fully-concrete signatures mint
        // `Result[i64 str]`/`Result[str i64]` at parse time (R1's fold), the
        // same route `Err`'s two calls below resolve their constructor
        // through -- this test is about `reorder`'s own `unify_poly_input`
        // arm binding each swapped instantiation's arguments correctly, not
        // about R3 construction.
        let module = checked_module(
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : reorder ( 'T Result['T 'E] -- Result['T 'E] 'T ) swap ;\n\
             : show_is ( Result[i64 str] -- ) drop ;\n\
             : show_si ( Result[str i64] -- ) drop ;\n\
             : main ( -- )\n\
               1 \"boom\" Err reorder drop show_is\n\
               \"one\" 2 Err reorder drop show_si ;\n",
        );
        assert!(module
            .words
            .iter()
            .any(|w| w.name == "reorder" && w.poly.is_some()));
    }

    /// P7 slice 3a (R3): a poly word constructs a generic value whose header
    /// argument the operand alone does not determine (`Err`'s payload never
    /// mentions `Result`'s `'T`) -- the load-bearing case for the phantom-
    /// argument backstop: the missing argument is recovered from the
    /// enclosing word's own declared output naming the same header.
    #[test]
    fn poly_body_constructor_resolves_arguments_from_the_declared_output() {
        check_src(
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : wrap ( 'T -- Result['T i64] ) Ok ;\n\
             : main ( -- ) True wrap drop ;\n",
        )
        .expect("a phantom argument recovers from the declared output");
    }

    /// P7 slice 3a (R5.2): a generic constructor call whose header variable
    /// is determined by neither its operands nor the enclosing word's
    /// declared output (which does not name the header at all here) is a
    /// located error, not a latent monomorphization failure.
    #[test]
    fn poly_body_constructor_undetermined_argument_is_error() {
        let err = check_src(
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             : bad ( 'T i64 -- 'T ) Err drop ;\n\
             : main ( -- ) 1 2 bad drop ;\n",
        )
        .unwrap_err();
        assert!(err.contains("leaves the type variable"), "{err}");
        assert!(err.contains("'T"), "{err}");
    }

    /// P7 slice 3a (R5.3): a generic constructor call whose operands
    /// disagree with each other over the header argument they both bind
    /// (two fields sharing one type variable, called with two different
    /// concrete types) is reported at the constructor call, during body
    /// check, never deferred into a later synthesis/monomorphization step.
    #[test]
    fn poly_body_constructor_operand_mismatch_is_error() {
        let err = check_src(
            "type: Pair['T] val1 'T val2 'T ;\n\
             : mk ( 'T -- Pair['T] ) 1 swap Pair ;\n\
             : main ( -- ) \"oops\" mk drop ;\n",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch in `mk`"), "{err}");
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
    #[test]
    fn poly_body_destructuring_drop_overloaded_type_is_error() {
        // Bug 2 (round-1 review): `poly_call_term` resolved a generated
        // accessor through the ordinary `env` lookup with no D3 guard at all,
        // so a generic word could destructure any drop-overloaded type and
        // skip its destructor.
        let err = check_src(&format!(
            "{FD_DEF}: sneak ( 'T -- 'T i64 ) 7 Fd Fd> ;\n: main ( -- ) 1 sneak drop drop ;\n"
        ))
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot destructure `Fd` in `sneak` (line 3): it defines `drop`, so moving its fields out would skip its destructor\n  note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out"
        );
    }
    #[test]
    fn check_poly_call_rejects_a_quotation_argument() {
        // R9p: `check_poly_call` reads only `stack[base + i].ty`, so a quotation
        // does not *fail* unification, it *succeeds* binding `'T` to the
        // placeholder and monomorphizes a real call over a phantom. The guard
        // before `unify_poly_input` is what makes the R9 rejection reachable.
        let err = check_src(
            ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) [ add ] dupit drop drop ;\n",
        )
        .expect_err("a quotation passed to a polymorphic word should be rejected");
        assert!(
            err.contains("a quotation cannot be passed to `dupit`"),
            "check_poly_call should name `dupit`, got: {err}"
        );
    }
    /// R4: `reject_quotation_argument`'s new exact wording at `check_poly_call`'s
    /// own R9p call site (a bare `PolyType::Var` position) -- the "slice 7"
    /// parenthetical is retired, and the rest of the message is unchanged.
    #[test]
    fn reject_quotation_argument_wording_at_poly_var_position() {
        let err = check_src(
            ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) [ add ] dupit drop drop ;\n",
        )
        .expect_err("a quotation passed to a polymorphic word should be rejected");
        assert_eq!(
            err,
            "error: a quotation cannot be passed to `dupit`; only `call` accepts one in `main` (line 2)"
        );
    }
    /// P7 slice 3f (R1/R2): a `Known` literal quotation argument at a declared
    /// ground `Type::Quotation` input materializes and the call succeeds, with
    /// the quotation-typed input first among the declared inputs.
    #[test]
    fn check_poly_call_materializes_ground_quotation_first_position() {
        check_src(
            ": run_it_first ['T: Copy] ( [ i64 -- i64 ] 'T -- 'T ) swap drop ;\n\
             : main ( -- ) [ 1 add ] 7 run_it_first drop ;\n",
        )
        .expect("a ground quotation argument in the first position should materialize");
    }
    #[test]
    fn check_poly_call_materializes_ground_quotation_middle_position() {
        check_src(
            ": run_it_mid ['T: Copy] ( 'T [ i64 -- i64 ] Bool -- 'T ) drop drop ;\n\
             : main ( -- ) 7 [ 1 add ] True run_it_mid drop ;\n",
        )
        .expect("a ground quotation argument in the middle position should materialize");
    }
    #[test]
    fn check_poly_call_materializes_ground_quotation_last_position() {
        check_src(
            ": run_it_last ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T ) drop ;\n\
             : main ( -- ) 7 [ 1 add ] run_it_last drop ;\n",
        )
        .expect("a ground quotation argument in the last position should materialize");
    }
    /// P7.S3l phase 2 (R9p closure): flipped from a rejection to an accept
    /// case. A declared quotation whose brackets mention `'T` no longer
    /// falls through R9p's blanket rejection once every variable the row
    /// mentions is already bound by an earlier input (here, `'T` is bound by
    /// the plain `7` argument before the quotation argument is reached) --
    /// the literal is grounded through that `subst` and materialized exactly
    /// as a ground declared quotation slot is.
    #[test]
    fn check_poly_call_materializes_an_abstract_quotation_argument() {
        check_src(
            ": run_abstract ['T: Copy] ( 'T [ 'T -- 'T ] -- 'T ) drop ;\n\
             : main ( -- ) 7 [ ] run_abstract drop ;\n",
        )
        .expect("a literal argument at an abstract quotation position should materialize");
    }
    /// P7.S3l phase 2 review: the two-pass split (mirroring
    /// `check_poly_combinator_args`) makes grounding order-independent -- the
    /// declared quotation slot comes *before* the plain input that binds
    /// `'T`, the reverse of the sibling test above.
    #[test]
    fn check_poly_call_materializes_an_abstract_quotation_argument_declared_first() {
        check_src(
            ": run_abstract_first ['T: Copy] ( [ 'T -- 'T ] 'T -- 'T ) swap drop ;\n\
             : main ( -- ) [ ] 7 run_abstract_first drop ;\n",
        )
        .expect("grounding a quotation slot declared before its binding input should succeed");
    }
    /// R2: a capturing literal at the argument boundary runs the existing R15
    /// admission path. An in-frame (non-escaping) capture is admitted; this
    /// alone survives stubbing out the `check_capture_admission` call at this
    /// call site, so it does not by itself prove the path is wired up -- see
    /// the escaping-capture rejection below for that proof.
    #[test]
    fn check_poly_call_admits_a_capturing_literal_argument() {
        check_src(
            ": run_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T ) drop ;\n\
             : main ( -- ) 3 | n | 7 [ n add ] run_it drop ;\n",
        )
        .expect("an in-frame capturing literal should be admitted at the argument boundary");
    }
    /// R2, discriminating: an escaping capture at the argument boundary must
    /// hit `check_capture_admission`'s existing rejection -- proof this new
    /// call site actually invokes it, not just present in the diff.
    #[test]
    fn check_poly_call_rejects_an_escaping_capturing_literal_argument() {
        let err = check_src(
            ": run_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T ) drop ;\n\
             : main ( -- ) [ 1 add ] | q | 7 [ q call ] run_it drop ;\n",
        )
        .expect_err("an escaping capturing literal must be rejected at the argument boundary");
        assert!(
            err.contains("capturing a quotation value by name is deferred"),
            "{err}"
        );
    }
    /// P7 slice 3f (R3): `call` on a genuine ground `Type::Quotation`
    /// parameter -- a real value with no interned body to splice -- honours the
    /// declared effect, popping its inputs and pushing its outputs.
    #[test]
    fn poly_call_term_calls_a_ground_quotation_param() {
        check_src(
            ": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T i64 ) 1 swap call ;\n\
             : main ( -- ) 7 [ 1 add ] call_it drop drop ;\n",
        )
        .expect("`call` on a ground quotation parameter should honour its declared effect");
    }
    /// R3's *ordering*, the output side: the declared outputs are pushed in
    /// declaration order, so the first one lands deepest. Every other R3 test
    /// declares a single output, which cannot tell the push order from its
    /// reverse. Checker-only rather than a golden because a quotation effect
    /// with two outputs cannot yet be lowered (see the phase 3 note on
    /// `intern_output_bundles`); the input side gets the golden instead.
    #[test]
    fn poly_call_on_a_ground_quotation_param_pushes_outputs_in_order() {
        check_src(": call_it ['T: Copy] ( 'T [ -- i64 Bool ] -- 'T i64 Bool ) call ;\n")
            .expect("the first declared output must land deepest");
    }
    /// R3's negative, the `PolyType::Concrete` renderer arm: a ground operand
    /// at a popped position that simply is not the declared input type is a
    /// located rejection, not a panic and not a silent coercion.
    #[test]
    fn poly_call_on_a_ground_quotation_param_ground_mismatch_is_error() {
        let err = check_src(
            ": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- 'T i64 ) True swap call ;\n\
             : main ( -- ) 7 [ 1 add ] call_it drop drop ;\n",
        )
        .expect_err("a wrong operand type at a declared input must be rejected");
        assert_eq!(
            err,
            "error: type mismatch in `call_it` (line 1)\n  \
             `call` expected `i64`, found `Bool`\n  note: declared ( -- )"
        );
    }
    /// R3's negative, the `poly_rendered_type_mismatch_error` arm: an operand
    /// with no ground `Type` to render (here a bare `PolyType::Var`) at a
    /// popped position. `type_mismatch_error` cannot render this side at all,
    /// which is why the two arms exist.
    #[test]
    fn poly_call_on_a_ground_quotation_param_variable_operand_is_error() {
        let err = check_src(": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- i64 ) call ;\n")
            .expect_err("a type variable at a declared input must be rejected");
        assert_eq!(
            err,
            "error: type mismatch in `call_it` (line 1)\n  \
             `call` expected `i64`, found `'T`\n  note: declared ( -- )"
        );
    }
    /// R3's underflow arm, distinct from the bare-`call`-with-an-empty-stack
    /// rejection above it: the quotation is there, the operands its declared
    /// effect demands are not.
    #[test]
    fn poly_call_on_a_ground_quotation_param_underflow_is_error() {
        let err = check_src(": call_it ['T: Copy] ( 'T [ i64 -- i64 ] -- i64 ) swap drop call ;\n")
            .expect_err("a declared input with nothing beneath the quotation must be rejected");
        assert!(
            err.contains("`call` needs 1 values, but the stack holds 0"),
            "{err}"
        );
    }
    /// P7.S3l (R1/R2): flipped from S3f's pinned rejection -- an abstract
    /// declared quotation parameter (`[ i64 -- 'T ]`, its output still
    /// carrying the word's own type variable) is now `call`-able from the
    /// word's own body. `swap drop` first discards the word's other `'T`
    /// input so the quotation's own `'T` output is the sole thing left on
    /// exit, matching the declared single-`'T` output -- the near miss
    /// `check_poly_call_materializes_an_abstract_quotation_argument` (a
    /// different guard, at the argument boundary, also flipped by this
    /// slice's R5) does not cover: only the *output* side carries the
    /// variable, so a dispatch
    /// predicate that checked the declared inputs were ground (they are, a
    /// single `i64`) would wrongly claim this one.
    #[test]
    fn poly_call_on_an_abstract_quotation_param_is_accepted() {
        check_src(": call_it ['T: Copy] ( 'T [ i64 -- 'T ] -- 'T ) swap drop 1 swap call ;\n")
            .expect("an abstract quotation parameter is now call-able");
    }
    /// R3's underflow arm, the abstract twin of
    /// `poly_call_on_a_ground_quotation_param_underflow_is_error`: the
    /// quotation is there, the operand its declared input demands is not.
    #[test]
    fn poly_call_on_an_abstract_quotation_param_underflow_is_error() {
        let err = check_src(": call_it ['T: Copy] ( 'T [ i64 -- 'T ] -- 'T ) swap drop call ;\n")
            .expect_err("a declared input with nothing beneath the quotation must be rejected");
        assert!(
            err.contains("`call` needs 1 values, but the stack holds 0"),
            "{err}"
        );
    }
    /// R2's ordering, the input side: two heterogeneous declared inputs, so
    /// consuming them in the wrong order (rather than merely miscounting
    /// them) is discriminated. Every other R2/R3 test in this file declares
    /// a single input, which cannot tell deepest-first from its reverse.
    #[test]
    fn poly_call_on_an_abstract_quotation_param_pops_declared_inputs_deepest_first() {
        check_src(
            ": call_it ['T: Copy] ( 'T [ i64 Bool -- 'T ] -- 'T )\n\
               swap drop 1 True rot call\n\
             ;\n",
        )
        .expect("the deepest operand must satisfy the first declared input");
    }
    /// R2's ordering, the output side: two heterogeneous declared outputs,
    /// so pushing them in the wrong order is discriminated. The existing
    /// `poly_call_on_a_ground_quotation_param_pushes_outputs_in_order` pins
    /// this for the ground twin only; the abstract arm has its own push loop
    /// (`poly_call_abstract_quotation_param`) and needs its own witness.
    #[test]
    fn poly_call_on_an_abstract_quotation_param_pushes_outputs_in_order() {
        check_src(
            ": call_it ['T: Copy] ( 'T [ -- 'T Bool ] -- 'T Bool )\n\
               swap drop call\n\
             ;\n",
        )
        .expect("the first declared output must land deepest");
    }
    /// R3's mismatch arm: an operand whose `PolyType` is not structurally
    /// equal to the declared input.
    #[test]
    fn poly_call_on_an_abstract_quotation_param_mismatch_is_error() {
        let err = check_src(
            ": call_it ['T: Copy] ( 'T [ i64 -- 'T ] -- 'T ) swap drop True swap call ;\n",
        )
        .expect_err("an operand not structurally equal to the declared input must be rejected");
        assert_eq!(
            err,
            "error: type mismatch in `call_it` (line 1)\n  \
             `call` expected `i64`, found `Bool`\n  note: declared ( -- )"
        );
    }
    /// L1's other side: the new arm is gated on the operand being a ground
    /// quotation, not merely on it not being a `QuotLit` marker -- `call` on a
    /// body local bound to a bare `'T` keeps its own rejection.
    #[test]
    fn poly_call_on_a_variable_local_is_still_error() {
        let err = check_src(": call_it ['T: Copy] ( 'T -- ) | a | a call ;\n")
            .expect_err("`call` on a type variable stays rejected");
        assert_eq!(
            err,
            "error: `call` is not permitted on the type variable `'T` in `call_it` (line 1)"
        );
    }
    #[test]
    fn poly_term_admits_a_quotation_literal_as_a_marker_slot() {
        // P7 slice 3b (R2): the literal pushes a slot carrying its identity in
        // `quot` and a `pt` that is not a value type. Checked at the
        // `poly_term` level because no source program can observe the marker
        // directly: every route out of the body rejects it.
        let sig = bare_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let mut overloads = HashMap::new();
        let quot_term = Term {
            kind: TermKind::Quotation(Vec::new(), true, None),
            span: Span::default(),
        };
        let stack = poly_term(
            &quot_term,
            Vec::new(),
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut overloads,
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            false,
        )
        .expect("a quotation literal is admitted in a polymorphic body");
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].pt, PolyType::QuotLit);
        assert_eq!(
            stack[0].quot,
            Some(PolyQuotRef(0)),
            "the literal's identity rides the slot, not its `PolyType`"
        );
    }
    #[test]
    fn poly_quotation_identity_moves_with_the_slot_under_swap() {
        // S3b L3: the literal's identity rides the slot, so a shuffle reorders
        // the indices with no special handling. Pinned here rather than through a
        // source program: a *tagged* literal must reach its eliminator by
        // written adjacency (the concrete path's rule), so no program can put
        // a shuffle between two arms.
        let sig = bare_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let mut overloads = HashMap::new();
        let quot = Term {
            kind: TermKind::Quotation(Vec::new(), true, None),
            span: Span::default(),
        };
        let swap = Term {
            kind: TermKind::Call("swap".to_string(), Vec::new()),
            span: Span::default(),
        };
        let mut stack = Vec::new();
        for term in [&quot, &quot, &swap] {
            stack = poly_term(
                term,
                stack,
                &mut scope,
                &sig,
                &ctx,
                &env,
                &CombinatorEnv::default(),
                &[],
                &[],
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut overloads,
                &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
                scratch_cross!(),
                false,
            )
            .expect("two literals then a swap");
        }
        assert_eq!(
            stack.iter().map(|s| s.quot).collect::<Vec<_>>(),
            vec![Some(PolyQuotRef(1)), Some(PolyQuotRef(0))],
            "`swap` reorders the identities with the slots"
        );
    }
    #[test]
    fn poly_quotation_slot_is_not_copy() {
        // R2: the marker is not a value, so it is never `Copy` -- `dup` on one
        // must not silently mint a second slot pointing at one interned body.
        assert!(!poly_is_copy(
            &PolyType::QuotLit,
            &bare_sig(),
            &[],
            &[],
            &[]
        ));
        assert!(!is_reference_slot(&PolyType::QuotLit));
    }
    /// The enum the eliminator unit tests below write their arms against.
    /// Declared `Circle` first so a test can write its arms `Rect`-first and
    /// still be correct: arms are matched by annotation tag, never by slot
    /// position.
    const SHAPE: &str = "type: Shape | Circle r i64 | Rect w i64 h i64 ;\n";
    #[test]
    fn poly_eliminator_registry_intercept_precedes_env_dispatch() {
        // R2: the eliminator is intercepted by name ahead of the ordinary
        // `env` dispatch. The arms here are written in the reverse of the
        // enum's declaration order, so an implementation that paired arms to
        // variants positionally (which is what the `PolySig` the eliminator is
        // registered under would do) checks `( Rect )`'s `Rect>` against a
        // narrowed `Shape.Circle` and fails.
        //
        // Anti-placebo note: deleting the intercept does *not* reach env
        // dispatch on this path at all -- `poly_call_term` has no `PolyCtx`,
        // so the eliminator's `PolySig` (registered in `poly_env`) is
        // unreachable and the call falls through to `unknown word`. So the
        // mutation flips accept -> reject, just not via the positional
        // mismatch; the reversed arm order is what makes the *accept* here
        // evidence of tag matching rather than of position matching.
        assert!(
            check_src(&format!(
                "{SHAPE}\
                 : pick ( 'T Shape -- 'T )\n\
                   ~[ ( Rect )   Rect> mul drop ]\n\
                   ~[ ( Circle ) Circle> dup mul 3 mul drop ]\n\
                   Shape? ;\n\
                 : main ( -- ) 1 5 Circle pick . ;\n"
            ))
            .is_ok(),
            "arms are matched by annotation tag, in any written order"
        );
    }
    #[test]
    fn poly_arm_join_rejects_rigid_type_variable_disagreement() {
        // S3b L1: `'T` stays rigid across arms. One arm leaving `'T` and
        // another `i64` is a located rejection naming both sides in order, never a
        // mid-body bind of `'T := i64`.
        let err = check_src(&format!(
            "{SHAPE}\
             : bad ['T: Copy] ( 'T Shape -- 'T )\n\
               ~[ ( Rect )   Rect> drop drop dup ]\n\
               ~[ ( Circle ) Circle> ]\n\
               Shape? drop ;\n\
             : main ( -- ) ;\n"
        ))
        .expect_err("two arms leaving different types disagree");
        assert!(
            err.contains("an earlier one leaves `'T`, this one leaves `i64`"),
            "the pairing is asserted, not just the failure: {err}"
        );
    }
    #[test]
    fn poly_arm_join_unions_borrows() {
        // S3b L4 (restated as S3b-follow L3): the arms' borrow tables are
        // unioned, not picked between. A missing record reads as "no
        // conflict", so dropping either arm's is a silent False accept -- both
        // directions are asserted, since "pick arm A" keeps `x` and drops `y`
        // and an `x`-only assertion would not flip.
        let program = |later: &str| {
            format!(
                "type: P a i64 ;\n\
                 {SHAPE}\
                 : bad ['T: Copy] ( 'T P P Shape -- 'T )\n\
                   | x y s | s\n\
                   ~[ ( Rect )   Rect> drop drop &!x ]\n\
                   ~[ ( Circle ) Circle> drop &!y ]\n\
                   Shape?\n\
                   {later} drop drop ;\n\
                 : main ( -- ) ;\n"
            )
        };
        for place in ["x", "y"] {
            let err = check_src(&program(place))
                .expect_err("both arms' borrows survive the merge, so either use conflicts");
            assert!(
                err.contains(&format!("cannot name `{place}`"))
                    && err.contains("a mutable borrow of it is still live"),
                "the `{place}` record must survive the union: {err}"
            );
        }
    }
    /// P7 slice 3b-follow (R1): the pieces `poly_walk_arms` needs from a
    /// caller that is not the eliminator -- a one-variable signature (so a
    /// bare `'T` slot is non-`Copy` and carries a move obligation) and an arm
    /// that binds one local and reads it back.
    fn one_var_sig() -> PolySig {
        PolySig {
            ty_var_names: vec!["T".to_string()],
            ..bare_sig()
        }
    }
    fn arm_binding(local: &str, consume: bool) -> Vec<Term> {
        let mut body = vec![Term {
            kind: TermKind::Bind(vec![local.to_string()]),
            span: Span::default(),
        }];
        if consume {
            body.push(Term {
                kind: TermKind::Call(local.to_string(), Vec::new()),
                span: Span::default(),
            });
        }
        body
    }
    fn interned_arm(scope: &mut PolyScope, body: Vec<Term>) -> PolyArm {
        let quot = scope.intern_quotation(PolyQuotLit {
            body,
            span: Span::default(),
            is_inline: true,
            annot: None,
        });
        PolyArm {
            quot,
            input: vec![PolySlot::new(PolyType::Var(0))],
            declared_inputs: vec![Type::I64],
            tail: false,
        }
    }
    #[test]
    fn poly_walk_arms_truncates_arm_locals_before_joining_moves() {
        // R1: the `Scope::leave` analogue is what makes the N-arm
        // `Moves::join` sound -- it indexes each later arm's map by the first
        // arm's keys, so two arms binding *different* names panic outright
        // unless every arm is truncated back to the enclosing key set first.
        // Driven through the shared helper rather than an eliminator so the
        // machinery is pinned independently of its one caller today.
        let sig = one_var_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let arms = vec![
            interned_arm(&mut scope, arm_binding("a", true)),
            interned_arm(&mut scope, arm_binding("b", true)),
        ];
        let mut exits: Vec<Vec<PolyType>> = Vec::new();
        poly_walk_arms(
            arms,
            "consumer",
            Span::default(),
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut HashMap::new(),
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            &mut |_, exit| {
                exits.push(exit.into_iter().map(|slot| slot.pt).collect());
                Ok(())
            },
        )
        .expect("two arms binding different locals join once both are truncated");
        assert_eq!(
            exits,
            vec![vec![PolyType::Var(0)], vec![PolyType::Var(0)]],
            "every arm's exit reaches the cross-arm rule, in written order"
        );
        assert!(
            scope.moves.states.is_empty() && scope.locals.is_empty(),
            "an arm-local never reaches the enclosing scope: {:?}",
            scope.moves.states
        );
    }
    #[test]
    fn poly_walk_arms_rejects_an_arm_local_left_unconsumed() {
        // R1: the leak is rejected *before* the truncation erases it -- the
        // poly walk has no block scope, so nothing else would ever notice a
        // linear local bound inside an arm and dropped on the floor there.
        let sig = one_var_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let arms = vec![interned_arm(&mut scope, arm_binding("a", false))];
        let err = poly_walk_arms(
            arms,
            "consumer",
            Span::default(),
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut HashMap::new(),
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            &mut |_, _| Ok(()),
        )
        .expect_err("a linear local bound in an arm and never read leaks");
        assert!(
            err.contains("the local `a` of type `T`, bound in an arm of `consumer`")
                && err.contains("is never consumed"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn poly_row_combinator_admits_only_a_row_typed_inline_declaration() {
        // S3b-follow (R2): the dispatch's entry condition, decided from the
        // callee's declaration alone. `rowed` qualifies; `rowless` is the
        // concrete-consumer shape (P7.S3d) and must keep the rejection it has
        // today rather than be admitted through row machinery; `plain` is an
        // ordinary word. `if` is checked too, declared verbatim rather than
        // relying on injection: P8.S2 deleted the prelude, so `if` is an
        // ordinary `core::bool` word now (`lib/bool.sth`) and this test's
        // hand-parsed source has to declare it itself to have it registered
        // in `combinators` at all.
        let src = "type: Bool | False | True ;\n\
                   : rowed inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call ;\n\
                   : rowless inline ( array['T 4] ~[ 'T -- 'T ] -- array['T 4] ) | f | f call ;\n\
                   : plain ( i64 -- i64 ) 1 add ;\n\
                   : if inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )\n\
                     | e | | t | | c | c tag t e branch ;\n";
        let tokens = lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let combinators = collect_combinators(&module.words);
        assert!(
            poly_row_combinator(&combinators, "rowed").is_some(),
            "a row on a quotation parameter is what the dispatch grounds"
        );
        assert!(
            poly_row_combinator(&combinators, "rowless").is_none(),
            "a rowless quotation parameter is P7.S3d's shape, not this dispatch's"
        );
        assert!(poly_row_combinator(&combinators, "plain").is_none());
        assert!(
            poly_row_combinator(&combinators, "if").is_some(),
            "a same-module `if` (or the real `core::bool` one, mangled per module) still \
             registers under a single-module program's bare spelling"
        );
    }
    #[test]
    fn poly_combinator_dispatch_precedes_the_quotlit_operand_window() {
        // S3b-follow (R2): the dispatch must sit ahead of *both* rejections
        // that used to catch this family. `unless` never reached the name
        // guard at all -- it is not one of the names that guard lists -- and
        // landed on the `QuotLit` operand window instead. Moving the dispatch
        // below that window makes this body fail again.
        let body = "over over gt ~[ drop ] ~[ swap drop ] unless";
        check_src(&format!(
            ": mymin ['T: Copy Ord] ( 'T 'T -- 'T ) {body} ;\n: main ( -- ) 2 9 mymin . ;\n"
        ))
        .expect("`unless` reaches the dispatch");
        // The accept alone would also be satisfied by an implementation that
        // stopped checking the arms, so the arm rule is asserted to still
        // report through *this* dispatch rather than the operand window.
        let err = check_src(
            ": bad ['T: Copy Ord] ( 'T 'T -- 'T ) over over gt ~[ drop ] ~[ swap ] unless ;\n",
        )
        .expect_err("the arms leave different shapes");
        assert!(
            err.contains("the quotations passed to `unless` leave different stack shapes"),
            "`unless`'s arms must be checked by the dispatch: {err}"
        );
    }
    #[test]
    fn poly_combinator_routes_by_the_declared_row_pair() {
        // R3: one dispatch, two routes. A parameter whose declared rows are
        // the same on both sides is held to its *declaration* (the seeded
        // entry row), and one whose rows differ is held only to its *sibling*
        // arms -- so the same arm body is legal under one and rejected under
        // the other. `same` and `differ` are otherwise identical, which is
        // what makes this a routing test rather than two unrelated checks.
        const BOTH: &str =
            ": same   inline ( ..a Bool ~[ ..a -- ..a ] ~[ ..a -- ..a ] -- ..a )\n\
               | same--e | | same--t | | same--c | same--c tag same--t same--e branch ;\n\
             : differ inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )\n\
               | differ--e | | differ--t | | differ--c | differ--c tag differ--t differ--e branch ;\n";
        // Both arms consume a slot of the row they entered with: a shape
        // change the siblings agree on, and a violation of a row declared the
        // same on both sides.
        let body = "over over gt ~[ drop ] ~[ swap drop ]";
        check_src(&format!(
            "{BOTH}: g ['T: Copy Ord] ( 'T 'T -- 'T ) {body} differ ;\n"
        ))
        .expect("the shape-changing route holds the arms to each other");
        let err = check_src(&format!(
            "{BOTH}: g ['T: Copy Ord] ( 'T 'T -- 'T 'T ) {body} same ;\n"
        ))
        .expect_err("a row declared the same on both sides fixes the exit");
        assert!(
            err.contains(
                "was declared `~[ ..a -- ..a ]`, but it leaves `'T` where that requires `'T 'T`"
            ),
            "the non-shape-changing route holds the arm to its declaration: {err}"
        );
    }
    #[test]
    fn poly_combinator_shape_changing_exit_row_is_what_the_arms_agreed() {
        // R3: the exit row of a shape-changing call is the arms' agreed exit,
        // not the row the call was entered with. Pinned by the *caller's*
        // declared outputs: this body's arms each consume one slot of the two
        // they enter with, so an exit taken from the entry row would leave `'T
        // 'T` and disagree with the signature.
        let body =
            ": g ['T: Copy Ord] ( 'T 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;\n";
        check_src(body).expect("the exit row is the arms' own");
        let err = check_src(&body.replace("-- 'T )", "-- 'T 'T )"))
            .expect_err("the entry row is not handed back");
        assert!(
            err.contains("body leaves `'T`, but the declared outputs are `'T 'T`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn poly_combinator_declaring_a_row_no_arm_produces_is_located() {
        // R3: a signature promising an output row that none of its quotation
        // parameters produces has no account of that row at all -- the arms
        // agreed on nothing to hand back. Located, and named against the row
        // itself, rather than answered with the entry row the declaration
        // explicitly differs from.
        let err = check_src(
            ": weird inline ( ..a Bool ~[ ..a -- ..a ] -- ..b ) | weird--f | | weird--c | weird--c tag weird--f weird--f branch ;\n\
             : g ['T: Copy Ord] ( 'T 'T -- 'T ) over over gt ~[ ] weird ;\n",
        )
        .expect_err("the declared output row is ungroundable");
        assert!(
            err.contains(
                "`weird` declares `..b`, which a call in the polymorphic body of `g` (line 2) cannot ground"
            ),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn poly_combinator_grounds_the_row_to_the_caller_region() {
        // R3/L2: the declared row grounds to `stack[..base]` -- the caller
        // region *below* the combinator's fixed inputs -- once, at the
        // dispatch site. Pinned from both sides, since grounding it to the
        // whole stack or to nothing each breaks only one of them: the arm can
        // shuffle exactly the two slots the region holds, and reaching one
        // slot deeper underflows inside the arm.
        const TWICE: &str = ": twice inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call f call ;\n";
        check_src(&format!(
            "{TWICE}: g ['T: Copy Ord] ( 'T 'T -- 'T 'T ) ~[ swap ] twice ;\n"
        ))
        .expect("the arm walks over the grounded row");
        let err = check_src(&format!(
            "{TWICE}: g ['T: Copy Ord] ( 'T 'T -- 'T 'T ) ~[ drop drop drop ] twice ;\n"
        ))
        .expect_err("the region is the caller row, not the whole stack");
        assert!(
            err.contains("`drop` needs 1 values, but the stack holds 0"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn poly_eliminator_arm_leaving_its_own_variant_is_error() {
        // R2 step 5b: with two arms, R3's rigid-arm-disagreement check
        // (different exit shapes) fires before this guard ever gets a chance
        // to look at the escaping `Type::Variant`, so a two-arm repro cannot
        // exercise it. A single-variant enum is exhaustive with one arm and
        // reaches this guard directly. Stubbing it out here builds and
        // double-drops the linear `Spy` payload underneath, since `is_copy`
        // falls through `Type::Variant` to `True`.
        let err = check_src(&format!(
            "{SPY}\
             type: One | A p Spy ;\n\
             : bad ['T: Copy] ( 'T One -- 'T ) ~[ ( A ) ] One? ;\n\
             : main ( -- ) 1 9 Spy A bad drop ;\n"
        ))
        .expect_err("an arm leaving its own narrowed variant unconsumed is an escape");
        assert!(
            err.contains("an arm of `One?` leaves `One.A` on the stack"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn polyslot_int_val_folds_lits() {
        // R1: `int_val` carries what the deleted `lits` shadow did -- set on
        // `IntLit`, `None` elsewhere, truncated on `Bind`. Round-tripped
        // directly at the `poly_term` level since a bound local's own
        // literal-ness is discarded (D6), not observable through `check_src`.
        let sig = bare_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let mut overloads = HashMap::new();
        let lit_term = Term {
            kind: TermKind::IntLit(9),
            span: Span::default(),
        };
        let stack = poly_term(
            &lit_term,
            Vec::new(),
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut overloads,
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            false,
        )
        .expect("an int literal should push a slot");
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].pt, PolyType::Concrete(Type::I64));
        assert_eq!(stack[0].int_val, Some(9));

        let bind_term = Term {
            kind: TermKind::Bind(vec!["x".to_string()]),
            span: Span::default(),
        };
        let stack = poly_term(
            &bind_term,
            stack,
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut overloads,
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            false,
        )
        .expect("binding the literal should consume the slot");
        assert!(
            stack.is_empty(),
            "the bound literal's slot leaves the stack"
        );
        assert_eq!(scope.locals["x"], PolyType::Concrete(Type::I64));
    }
    #[test]
    fn check_poly_copy_word_accepts_and_instantiates() {
        // R1/R4–R7: a `'T: Copy` word `dup`s its variable and is called at a
        // concrete `Copy` type; the body and the instantiation both check.
        check_src(": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n: main ( -- ) 5 dupit drop drop ;")
            .unwrap();
    }
    #[test]
    fn check_poly_word_records_one_instantiation_per_concrete_shape() {
        // R8/R14: each distinct ground θ is recorded once, keyed by call span.
        let module = checked_module(
            ": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit drop drop True dupit drop drop ;",
        );
        // Two call sites, two distinct θ (i64 and Bool): two instantiations.
        let symbols: std::collections::HashSet<&str> = module
            .instantiations
            .values()
            .map(|c| c.symbol.as_str())
            .collect();
        assert_eq!(module.instantiations.len(), 2);
        assert_eq!(symbols.len(), 2);
    }
    /// P7.S3t (R4/R6/R9): the seed is what a call site's explicit type
    /// argument *does*. The fixture is the one shape inference cannot reach:
    /// `'T` appears only in an output, so with no list `apply_subst` reports
    /// it and no instantiation is recorded at all. (A word with no operand can
    /// declare a `'T` output only if its body produces one, which before a
    /// nullary trait member -- phase 3 -- means its own self-call.)
    #[test]
    fn an_explicit_instantiation_seeds_the_recorded_substitution() {
        let module = checked_module(": f ( -- 'T ) f ;\n: main ( -- ) f[i64] drop ;");
        let inst = module
            .instantiations
            .values()
            .find(|i| i.callee == "f")
            .expect("the seeded call site recorded an instantiation");
        assert_eq!(inst.subst.ty_of(0), Some(Type::I64));
        assert!(
            inst.symbol.contains("i64"),
            "the seeded theta mints the specialization: {}",
            inst.symbol
        );
        // The same call without the list: R9's revived diagnostic at the site
        // R9 describes, `apply_subst` walking the *declared outputs*. The
        // message also fires from pass 2's quotation grounding, where it
        // misdescribes an input as an output, so both witnesses use the
        // output shape and neither leans on that path.
        let bare =
            check_src(": f ( -- 'T ) f ;\n: main ( -- ) f drop ;").expect_err("nothing binds `'T`");
        assert_eq!(
            bare,
            "error: `f` in `main` (line 2) has output variable `'T` that no input binds\n  \
             note: supply it explicitly: `f[SomeType]`"
        );
    }
    /// P7.S3t (R9): the remedy names one `SomeType` per declared type
    /// variable, not a single one regardless of arity -- a two-variable
    /// callee's note must read `f[SomeType SomeType]`, since `f[SomeType]`
    /// would just fail again on arity.
    #[test]
    fn an_unbound_output_note_matches_a_two_variable_signatures_arity() {
        let bare = check_src(": f ( 'T -- 'U ) f ;\n: main ( -- ) 1 f drop ;")
            .expect_err("nothing binds `'U`");
        assert_eq!(
            bare,
            "error: `f` in `main` (line 2) has output variable `'U` that no input binds\n  \
             note: supply it explicitly: `f[SomeType SomeType]`"
        );
    }
    /// P7.S3t (R5/R6): a seeded and an inferred call at the same θ are the
    /// same specialization. `instantiation_symbol` renders `subst.ty` in
    /// vector order, so the two paths agree only if both leave it sorted by
    /// variable id -- and the *inferred* path does not: `check_poly_call`
    /// defers a quotation input to pass 2, so `r` binds `'U` (id 1) before
    /// `'T` (id 0). `id ( 'T -- 'T )` cannot witness this, having one
    /// variable and so only one possible order; `r` is the smallest signature
    /// that can. Without the sort, this call pair mints
    /// `..._t1_f64_t0_i64` and `..._t0_i64_t1_f64` and monomorphizes `r`
    /// twice.
    #[test]
    fn a_seeded_and_an_inferred_call_at_one_type_mint_one_symbol() {
        let module = checked_module(
            ": r ( [ 'T -- ] 'U 'T -- 'U ) drop swap drop ;\n\
             : main ( -- ) [ drop ] 2.5 7 r . [ drop ] 2.5 7 r[i64 f64] . ;",
        );
        let insts: Vec<&CallInst> = module
            .instantiations
            .values()
            .filter(|i| i.callee == "r")
            .collect();
        assert_eq!(insts.len(), 2, "two call sites are recorded separately");
        for inst in &insts {
            assert_eq!(
                inst.subst.ty,
                vec![(0, Type::I64), (1, Type::F64)],
                "theta is kept sorted by variable id: {:?}",
                inst.subst.ty
            );
        }
        let symbols: std::collections::HashSet<&str> =
            insts.iter().map(|i| i.symbol.as_str()).collect();
        assert_eq!(symbols.len(), 1, "one theta, one symbol: {symbols:?}");
    }
    /// P7.S3t (R5): `subst.len` goes out of order for the same reason `ty`
    /// does -- pass 2 defers the quotation input, so `r` binds `'M` (id 1)
    /// before `'N` (id 0) -- and is sorted for a weaker one. No seed path can
    /// make two minting paths disagree on a length (R4), so this pins the
    /// normalization `Subst`'s order-sensitive derived `Eq` documents, not a
    /// divergence. The two lengths differ so the assertion cannot pass on a
    /// vector that is merely the right size.
    #[test]
    fn a_deferred_quotation_input_leaves_the_length_substitution_sorted() {
        let module = checked_module(
            ": r ( [ array['T 'N] -- ] array['U 'M] array['T 'N] -- ) drop drop drop ;\n\
             : main ( -- ) [ drop ] 0 2 fill 0 3 fill r ;",
        );
        let inst = module
            .instantiations
            .values()
            .find(|i| i.callee == "r")
            .expect("the call site recorded an instantiation");
        assert_eq!(
            inst.subst.len,
            vec![(0, 3), (1, 2)],
            "theta is kept sorted by length-variable id: {:?}",
            inst.subst.len
        );
    }
    /// P7.S3t (R4): exact arity over the callee's declared *type* variables,
    /// both directions, with the declared ones named -- the list is a
    /// statement about the callee's signature, so the message renders that
    /// signature's variables rather than just counting them.
    #[test]
    fn a_wrong_arity_instantiation_is_rejected() {
        let too_many = check_src(": id ( 'T -- 'T ) ;\n: main ( -- ) 7 id[i64 f64] . ;")
            .expect_err("one declared variable, two given");
        assert_eq!(
            too_many,
            "error: `id` (line 2) declares 1 type variable (`'T`) but was given 2 type arguments"
        );
        let too_few =
            check_src(": pairwise ( 'T 'U -- ) drop drop ;\n: main ( -- ) 1 2.5 pairwise[i64] ;")
                .expect_err("two declared variables, one given");
        assert_eq!(
            too_few,
            "error: `pairwise` (line 2) declares 2 type variables (`'T`, `'U`) but was given 1 type argument"
        );
    }
    /// P7.S3t (R4): a length variable is not addressable by the list, so a
    /// callee whose only variable is one declares *no* type variables. Said
    /// outright, since `alen[i64]` is the natural thing to try.
    #[test]
    fn an_instantiation_of_a_length_variable_is_rejected() {
        let err = check_src(
            ": alen ( array[i64 'N] -- array[i64 'N] usize ) len ;\n\
             : main ( -- ) 5 4 fill alen[i64] . drop ;",
        )
        .expect_err("a length variable cannot be given explicitly");
        assert_eq!(
            err,
            "error: `alen` (line 2) declares no type variables but was given 1 type argument\n  note: a length (`'N`) or row (`..s`) variable is not named by an explicit instantiation; only type variables are"
        );
    }
    /// P7.S3t (R5): the redirect itself, at `unify_poly_input`. One prior
    /// binding, one disagreeing operand, two messages -- the seeded one names
    /// which end was written, and the unseeded one is byte-identical to what
    /// two disagreeing operands have always reported.
    #[test]
    fn unify_poly_input_finding_a_seeded_variable_names_the_instantiation() {
        let sig = PolySig {
            row_in: None,
            inputs: vec![PolyType::Var(0)],
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: Vec::new(),
            row_var_names: Vec::new(),
        };
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let arrays: [ArrayDecl; 0] = [];
        let cells: [OwnedCellDecl; 0] = [];
        let refs: [RefDecl; 0] = [];
        let conflict = |seeded: &[u32]| {
            let mut subst = Subst::default();
            subst.ty.push((0, Type::F64));
            unify_poly_input(
                &sig,
                &sig.inputs[0],
                Type::I64,
                "f",
                Span::default(),
                &ctx,
                &arrays,
                &cells,
                &refs,
                &mut subst,
                seeded,
            )
            .expect_err("a disagreeing operand is a conflict either way")
        };
        assert_eq!(
            conflict(&[0]),
            "error: `f` in `probe` (line 0) was instantiated at `'T` = `f64` but its operand is `i64`"
        );
        assert_eq!(
            conflict(&[]),
            "error: `f` in `probe` (line 0) resolved `'T` to both `f64` and `i64`"
        );
    }
    #[test]
    fn check_generic_comparison_body_with_ord_checks_clean() {
        // P7.S3k (R1/R3/R7): re-expresses the retired
        // `check_poly_ord_word_accepts_comparison_body`. A generic body may
        // compare its own `'T` -- but through `lib/cmp.sth`'s real
        // `: gt ['T: Copy Ord] ( 'T 'T -- Bool )`, reached as a generic callee like
        // any other, not through a name-matched carve-out. Checked *mangled*,
        // so the callee arrives as `gt__mN` exactly as it does in a real
        // build; that is the shape the deleted carve-out could never see, and
        // the reason it was dead code.
        check_src_mangled(
            ": less ['T: Copy Ord] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) 3 4 less drop ;",
        )
        .unwrap();
    }
    #[test]
    fn check_generic_comparison_body_without_ord_is_error() {
        // P7.S3k (R3): the same body without the bound is a located call-site
        // error naming the missing `Ord`, not a deferred failure at whatever
        // type `less` is later instantiated at.
        let err = check_src_mangled(": less ['T: Copy] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) ;")
            .unwrap_err();
        assert!(
            err.contains("requires `Ord`") && err.contains("`less`"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("__m"), "a mangled spelling leaked: {err}");
    }
    #[test]
    fn check_concrete_overload_is_selected_over_an_ord_bounded_generic() {
        // P7.S3s (R6), the call-site half of criterion 6: `poly_sig_could_match`
        // is what makes a `Vec2 Vec2` call fall through the `Ord`-bounded
        // generic candidate to the concrete one of the same name. `Vec2` has no
        // `impl: Ord`, so the generic candidate must not admit these operands
        // even though unification alone would bind `'T` to anything.
        //
        // Asserted at the checker boundary rather than on a built program on
        // purpose: selecting the concrete candidate is the *correct* answer,
        // and a program that then calls it panics at lowering (`checked user
        // word exists`, `ir/func_builder/calls.rs`) because
        // `ast::overload_symbols` counts poly words when deciding a name is
        // overloaded, so the concrete word carries a `$$0`-suffixed symbol the
        // call site never records. That gap is pre-existing -- reproduced at
        // this slice's parent commit with `Bound::Ord` still in place -- and
        // orthogonal to `Ord`, so there is no run golden to pair with this;
        // `tests/phase7_slice3s_flip.rs` covers the declaration-time half.
        //
        // Mutation-verified: dropping `poly_sig_could_match`'s `impl:`
        // registry filter makes the generic candidate win, and this fails with
        // "`Vec2` does not satisfy `Ord`" from inside its instantiated body.
        check_src_mangled(
            "type: Vec2 x i64 y i64 ;\n\
             : mylt ['T: Copy Ord] ( 'T 'T -- Bool ) lt ;\n\
             : mylt ( Vec2 Vec2 -- Bool )\n\
               | a b | &a &x @ &b &x @ lt | r | a drop b drop r ;\n\
             : main ( -- ) 1 1 Vec2 2 2 Vec2 mylt drop ;\n",
        )
        .unwrap();
    }
    #[test]
    fn check_poly_length_word_accepts_and_monomorphizes_len() {
        // R1/R5/R9: a length variable is opaque through `len`; the same word
        // instantiates at `array[i64 4]` and `array[i64 8]`.
        check_src(
            ": alen ( array[i64 'N] -- array[i64 'N] usize ) len ;\n\
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
            ": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;\n\
             : main ( -- ) 1 2 dup2 . . . . ;",
        );
        assert_eq!(module.instantiations.len(), 1);
        let inst = module.instantiations.values().next().unwrap();
        assert_eq!(inst.out_arity, 4);
        assert!(inst.bundle.is_some());
    }
    #[test]
    fn check_x4_type_variable_forced_to_two_concretes_names_both() {
        // X4: one `'T` unified to both `i64` and `Bool` at one call site names
        // both concrete types.
        let err = check_src(": pairwise ( 'T 'T -- ) drop drop ;\n: main ( -- ) 1 True pairwise ;")
            .unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("i64"), "unexpected message: {err}");
        assert!(err.contains("Bool"), "unexpected message: {err}");
    }
    #[test]
    fn check_x5_copy_bound_on_linear_type_names_variable_type_and_reason() {
        // X5: instantiating a `'T: Copy` word with a linear type is a located
        // call-site error naming the variable, the type, and the linear reason.
        let src = format!("{SPY}: idc ['T: Copy] ( 'T -- 'T ) ;\n: main ( -- ) 0 Spy idc drop ;");
        let err = check_src(&src).unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Spy"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }
    #[test]
    fn check_x6_ord_bound_on_non_ord_type_is_error() {
        // X6: instantiating a `'T: Ord` requirement with a non-`Ord` type is a
        // located error.
        // P7.S3k (R7): `Copy` joins the declaration. `gt` is `lib/cmp.sth`'s
        // `['T: Copy Ord] ( 'T 'T -- Bool )`, and the body's comparison now
        // discharges that whole bound set across the call (R3) instead of
        // being special-cased by name against `Ord` alone. The subject is
        // unchanged: `Bool` is `Copy` but not `Ord`, so it is `less`'s own
        // instantiation that fails, at `main`'s call site.
        let err = check_src(
            ": less ['T: Copy Ord] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) True False less drop ;",
        )
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
        // X8: `gt` on an unbounded `'T` inside a body requires an `Ord` bound.
        //
        // P7.S3k (R7): declared `'T: Copy` so `Ord` is the *only* bound
        // missing. The rule is now `gt`'s own declared bound set discharged
        // against this word's (R3), so an entirely unbounded `'T` names
        // whichever of the two comes first and would not pin `Ord`.
        let err =
            check_src(": bad ['T: Copy] ( 'T 'T -- Bool ) gt ;\n: main ( -- ) ;").unwrap_err();
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
        // A local named after a registered variant shadows the value that
        // name constructs: the poly binder rejects it as the monomorphic
        // sibling `( i64 i64 -- i64 )` of the same body does, naming the
        // collision.
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
    fn poly_self_call_structural_match_produces_outputs() {
        // P7 slice 3g (R1): a self-call whose operand window structurally
        // matches the walking word's own `sig.inputs` truncates and pushes
        // `sig.outputs`, letting the body finish typechecking to the
        // declared effect.
        check_src(": rec ( 'T i64 -- 'T ) drop 3 rec ;\n: main ( -- ) ;\n")
            .expect("a structurally matching self-call typechecks");
    }
    #[test]
    fn poly_self_call_operand_mismatch_is_located_error() {
        // D1's termination witness: a self-call whose operand window does
        // not structurally match `sig.inputs` is an ordinary located type
        // mismatch (`poly_rendered_type_mismatch_error`), never a check-time
        // loop or a backend panic.
        let err =
            check_src(": rec ( 'T i64 -- 'T ) drop True rec ;\n: main ( -- ) ;\n").unwrap_err();
        assert!(
            err.contains("type mismatch in `rec`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("`rec` expected `i64`, found `Bool`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn poly_self_call_underflow_reuses_arity_error() {
        // Too few operands ahead of a self-call is an ordinary arity
        // shortfall: the same underflow diagnostic any other operand-arity
        // gap produces, not a bespoke self-call message.
        let err = check_src(": rec ( 'T i64 -- 'T ) drop rec ;\n: main ( -- ) ;\n").unwrap_err();
        assert!(
            err.contains("`rec` needs 2 values, but the stack holds 1"),
            "unexpected message: {err}"
        );
    }
    /// P7.S3g-follow (1c): the one shape that can put a *body-derived*
    /// reference in a poly self-call's argument window, so every back-edge
    /// reference test is built from it. A declared `&!array['T 4]` is the only
    /// reference type a poly-body borrow can ever match: a body borrow is
    /// always `PolyType::Ref`, while a fully concrete `&!Cell` parameter folds
    /// to `Concrete(Type::Ref(..))` at parse time and the two never compare
    /// equal, so the referent has to stay variable-bearing. An array is then
    /// the only borrowable local a generic body admits (a bare `'T` might
    /// instantiate to a scalar, and a `Generic` application is not on the
    /// borrowable list).
    ///
    /// `'T: Copy` is an input slot of its own, so the leading `| r a b n |`
    /// binds four of the five declared inputs and each arm opens over a
    /// residual `'T`. Two array parameters, because the borrowed one cannot
    /// also be named for the value slot (that is the ordinary aliasing
    /// rejection, not this guard).
    fn self_tail_ref_loop(name: &str, recursive_arm: &str) -> String {
        format!(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : {name} ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ {recursive_arm} ] if ;\n\
             : main ( -- ) ;\n"
        )
    }
    #[test]
    fn poly_self_tail_reference_to_a_local_across_the_back_edge_is_error() {
        // P7.S3g-follow (1c): `&!a` is derived from a local of this frame, and
        // the self-call it rides is the loop's back-edge, where the header
        // rebinds every local -- so next iteration the name denotes different
        // storage than the reference points at. Clean at HEAD before this
        // guard, while the monomorphic twin of the same body was already
        // rejected by `check_reference_across_back_edge`.
        let err = check_src(&self_tail_ref_loop(
            "loopg",
            "r drop &!a b dup n 1 sub loopg",
        ))
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 4)\n  a reference derived from `a`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }
    #[test]
    fn poly_self_tail_reference_to_a_local_across_the_back_edge_through_an_eliminator_arm_is_error()
    {
        // P7.S3g-follow (1a) follow-up: an eliminator arm inherits the call's
        // own tail-ness in `poly_eliminator_call` ("every eliminator arm runs
        // ... in the call's own position"), exactly as an `if`/`unless` arm
        // does, so the same back-edge hazard reached through `Bool?` instead
        // of `if` must be caught by the same guard. Same body as
        // `poly_self_tail_reference_to_a_local_across_the_back_edge_is_error`,
        // with the `if` swapped for the `Bool?` eliminator `if` lowers
        // through, and a `drop` in each arm for the narrowed variant `Bool?`
        // hands each arm (payload-less, but still a stack value) that `if`
        // never gives its arms.
        let err = check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero\n\
             ~[ ( False ) drop r drop &!a b dup n 1 sub loopg ]\n\
             ~[ ( True ) drop drop r drop 0 ]\n\
             Bool? ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 5)\n  a reference derived from `a`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }
    #[test]
    fn poly_self_tail_reference_rooted_in_a_spliced_block_local_is_error() {
        // P7.S3g-follow (1c): the same hazard one `call`-splice deeper. `x` is
        // a local of the *spliced* block, whose storage is a slot of this same
        // frame, so a reference to it dies at the back-edge exactly as `&!a`
        // does. The splice exit `retain`s the enclosing locals but keeps the
        // borrow records, so `x` is gone from `scope.locals` by the time the
        // self-call is checked -- which is why the guard reads
        // `PolyBorrow::static_rooted` instead of looking the place up.
        let err = check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ r drop a ~[ | x | &!x ] call b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 4)\n  a reference derived from `x`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }
    #[test]
    fn poly_self_tail_reference_rooted_in_a_local_shadowing_a_static_is_error() {
        // The second reason the guard cannot re-look-up the place: `A` names
        // both a static and a local here, and the borrow site resolves locals
        // first, so the record is rooted in the frame's slot however the name
        // reads from outside. Answering "is this a static?" by name at the
        // self-call would exempt it and let a dead reference cross.
        let err = check_src(
            "static: A i64 = 0 ;\n\
             : iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r A b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ r drop &!A b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 5)\n  a reference derived from `A`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }
    #[test]
    fn poly_self_tail_dropped_borrow_then_forwarded_ref_is_over_conservative() {
        // `poly_combinator_call`'s doc comment on `tail_slots` (and the spec)
        // claim a concrete cost for crediting every arm with the call's own
        // tail-ness instead of refining per arm: a body that borrows a local
        // in *any* arm and then tail-recurses is rejected whichever arm the
        // borrow sat in, even when that borrow is dropped before the
        // back-edge and only a forwarded parameter reference actually rides
        // it. Pinned here rather than left to a doc comment, so a later
        // per-arm refinement has something to flip green. The monomorphic
        // twin (same body, `'T` replaced by `i64` and no residual bound slot)
        // accepts it: `check_reference_across_back_edge` sees the borrow of
        // `a` already dropped and only `r` -- an incoming reference parameter
        // -- crossing.
        let err = check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ drop r drop 0 ] ~[ &!a drop r b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference to a local cannot cross a loop in `loopg` (line 4)\n  a reference derived from `a`, a local of this frame, crosses the self-tail-call back-edge to `loopg`: that local's storage does not survive to the next iteration\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
        check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ( &!array[i64 4] array[i64 4] array[i64 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ r drop 0 ] ~[ &!a drop r b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .expect("the monomorphic twin's dropped borrow does not ride the back-edge");
    }
    #[test]
    fn poly_self_tail_reference_parameter_forwarded_across_the_back_edge_is_ok() {
        // The accept case the guard must not swallow: `r` is the *incoming*
        // reference parameter, whose referent lives in an ancestor frame that
        // outlives every iteration. Nothing in this body borrows, so nothing
        // is recorded to reject.
        check_src(&self_tail_ref_loop("loopg", "r b dup n 1 sub loopg"))
            .expect("a reference parameter may cross the back-edge");
    }
    #[test]
    fn poly_non_tail_self_call_carrying_a_local_reference_is_ok() {
        // The per-term half of the gate (1a): the first arm's self-call has a
        // term after it, so it is *not* the back-edge -- it lowers as ordinary
        // recursion, into a fresh frame whose locals are bound once, and a
        // reference to a local is fine there. Written out rather than built by
        // `self_tail_ref_loop` because both halves of the gate have to be true
        // at once: the *second* arm carries the word's real back-edge (with a
        // parameter reference, which is legal), so `is_self_tail_call` holds
        // and `tail` is the only thing telling the two calls apart.
        check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T &!array['T 4] array['T 4] array['T 4] i64 -- i64 )\n\
             | r a b n |\n\
             n iszero ~[ r drop &!a b dup n 1 sub loopg drop 0 ]\n\
             ~[ r b dup n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .expect("a non-tail self-call is not a back-edge");
    }
    #[test]
    fn poly_self_tail_call_in_a_builtin_named_word_skips_the_back_edge_guard() {
        // The word-level half of the gate (1a): `has_self_tail_call` refuses
        // every builtin spelling, so lowering gives a generic `lt` no loop
        // header however its body is written. The guard must agree, or it
        // rejects a program that lowers as ordinary recursion. Same body as
        // the rejection above, renamed.
        check_src(&self_tail_ref_loop("lt", "r drop &!a b dup n 1 sub lt"))
            .expect("a builtin-named word never gets the loop shape");
    }
    #[test]
    fn poly_self_tail_reference_rooted_in_a_static_is_ok() {
        // A static's data-segment storage survives every iteration, unlike a
        // local's slot, so its borrow record must not be read as a hazard --
        // the poly twin of `static_ref_crosses_self_tail_call_back_edge_ok`.
        // The recorded `&!COUNT` is what the guard sees here; the reference
        // actually crossing is the parameter `r`.
        check_src(&format!(
            "static: COUNT i64 = 0 ;\n{}",
            self_tail_ref_loop("loopg", "&!COUNT drop r b dup n 1 sub loopg")
        ))
        .expect("a static-rooted borrow is not a local of this frame");
    }
    #[test]
    fn poly_self_tail_call_with_no_reference_argument_ignores_a_live_local_borrow() {
        // The other half of the rule: a live borrow of a local is only a
        // hazard when a reference actually rides the back-edge. Here `&a` is
        // parked in a `Copy` local (a shared reference, so nothing demands it
        // be consumed) which keeps the record live under the coarse liveness,
        // while the call's own window carries no reference at all.
        check_src(
            ": iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T array['T 4] i64 -- i64 )\n\
             | a n |\n\
             n iszero ~[ drop 0 ] ~[ &a | p | a n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n",
        )
        .expect("a live local borrow alone is not a back-edge hazard");
    }
    #[test]
    fn poly_self_tail_linear_forwarded_into_the_call_window_is_ok() {
        // The linear counterpart of the accept case: a `Spy` moved *into* the
        // recursive call's own argument window is forwarded, not stranded, so
        // the loop carries it as a back-edge operand with its single owner
        // intact. This is the whole of what a linear value can do at a poly
        // self-tail call -- see the two tests below.
        check_src(&format!(
            "{SPY}: iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( Spy 'T i64 -- Spy 'T )\n\
               dup iszero ~[ drop ] ~[ dup . 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n"
        ))
        .expect("a linear value forwarded into the window stays legal");
    }
    #[test]
    fn poly_self_tail_unconsumed_linear_local_is_error() {
        // Why there is no poly port of the monomorphic guard's *second*
        // clause (an unconsumed linear local at the back-edge): the general
        // end-of-body check already rejects the shape, one arm having
        // consumed `s` and the other not. The monomorphic clause exists only
        // to relocate that same rejection at the call (its own doc says so),
        // and a second message for an already-rejected program is not worth a
        // second rule. Pinned here so loosening the general check cannot open
        // the hole silently.
        let err = check_src(&format!(
            "{SPY}: iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( Spy 'T i64 -- 'T )\n\
               | s t n |\n\
               n iszero ~[ s drop t ] ~[ 9 Spy t n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert_eq!(
            err,
            "error: linear value `s` is never consumed in `loopg`\n  `s` has type `Spy`, which is linear: drop it or return it (nothing is dropped for you)"
        );
    }
    #[test]
    fn poly_self_tail_linear_stranded_below_the_call_window_is_not_well_typed() {
        // Why there is no poly port of the guard's *first* clause (a linear
        // value stranded below the argument window) either: a tail self-call
        // is the last term of a context whose exit row is the word's declared
        // outputs, and the call itself pushes exactly those outputs -- so
        // `stranded ++ outputs == outputs`, and nothing can be stranded in a
        // well-typed body. A generic body cannot even reach the shape from
        // below: unlike an inline combinator it walks no caller row, its
        // stack starts at `sig.inputs`.
        //
        // A tripwire, not a test of this slice's code: the day that exit-row
        // rule loosens, the stranded clause has to be written.
        let err = check_src(&format!(
            "{SPY}: iszero ( i64 -- Bool ) 0 eq ;\n\
             : loopg ['T: Copy] ( 'T i64 -- Spy 'T )\n\
               | t n |\n\
               n iszero ~[ 9 Spy t ] ~[ 9 Spy t n 1 sub loopg ] if ;\n\
             : main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains(
                "the quotations passed to `if` leave different stack shapes: an earlier one leaves `Spy 'T`, this one leaves `Spy Spy 'T`"
            ),
            "the stranded `Spy` shows up as the arms disagreeing: {err}"
        );
    }
    // -- P7.S3k: a generic word calling another generic word ---------------

    /// P7.S3k (R1): the slice's headline shape. Replaces the retired
    /// `poly_different_word_call_still_rejects` (and its `tests/` twins), which
    /// pinned the `poly_calls_poly_word_error` narrowing this closes.
    #[test]
    fn check_generic_word_calls_same_module_generic_grounds() {
        check_src(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) ;\n").unwrap();
    }

    /// P7.S3k (R1): the same call under the per-module mangling every real
    /// build applies, which is the only thing that distinguishes an *imported*
    /// callee from a same-module one at this level -- the arm dispatches on
    /// `poly_env`, whose keys are post-mangle names, never on a spelling. The
    /// end-to-end cross-module build golden is `tests/phase7_slice3k.rs`'s.
    #[test]
    fn check_generic_word_calls_mangled_generic_grounds() {
        check_src_mangled(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) ;\n").unwrap();
    }

    /// P7.S3k (R2): what the walk actually produces -- one symbolic record per
    /// grounded cross-call, keyed by the *containing* word, mapping the
    /// callee's own variable to the caller's. Phase 2 composes exactly this
    /// against a concrete θ, so its shape is the contract, not an incidental.
    #[test]
    fn check_generic_cross_call_records_the_caller_var_mapping() {
        let recorded =
            cross_calls_of(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) ;\n");
        let calls = recorded.get("g").expect("`g`'s body made the call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].callee, "id");
        assert_eq!(calls[0].mapping, vec![(0, Image::CallerVar(0))]);
        // Keyed by the caller, not the callee: `id`'s own body calls nothing.
        assert!(!recorded.contains_key("id"), "recorded: {recorded:?}");
    }

    /// P7.S3k (R2/R6, the accept side of the growth rule): a *concrete* image.
    /// The caller hands over an `i64` it built itself, so `'U` maps to a
    /// ground type rather than to one of the caller's variables -- legal, and
    /// the case a growth rule written as "reject anything but a bare variable"
    /// would wrongly refuse.
    #[test]
    fn check_generic_cross_call_with_a_concrete_operand_grounds() {
        let recorded =
            cross_calls_of(": h ( 'U -- 'U ) ;\n: g ( 'T -- 'T ) 1 h drop ;\n: main ( -- ) ;\n");
        let calls = recorded.get("g").expect("`g`'s body made the call");
        assert_eq!(calls[0].mapping, vec![(0, Image::Concrete(Type::I64))]);
    }

    /// P7.S3k (R6, the distinction an implementer must not confuse): a callee
    /// declaring its *own* compound parameter is structurally decomposed, so
    /// the image of `'U` is the bare `'T` and nothing grew. The mirror of
    /// `check_growing_cross_call_is_error` below, whose only difference is
    /// which side wrote the wrapper.
    #[test]
    fn check_generic_cross_call_forwarding_a_reference_grounds() {
        let recorded =
            cross_calls_of(": peek ( &'U -- ) drop ;\n: g ( &'T -- ) peek ;\n: main ( -- ) ;\n");
        let calls = recorded.get("g").expect("`g`'s body made the call");
        assert_eq!(calls[0].mapping, vec![(0, Image::CallerVar(0))]);
    }

    /// P7.S3k (R3): a bound the callee needs and the caller does not declare
    /// is a located error at the call site -- the user-declared-callee twin of
    /// `check_generic_comparison_body_without_ord_is_error`'s library one.
    #[test]
    fn check_generic_cross_call_bound_mismatch_is_error() {
        let err = check_src(
            ": biggest ['U: Ord] ( 'U -- 'U ) ;\n: g ( 'T -- 'T ) biggest ;\n: main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`'U` of `biggest` requires `Ord`, which `'T` in `g` does not declare"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3s (R2, `Bound::User` half of `poly_cross_bound_error`): the same
    /// walk-time `Image::CallerVar` arm as `check_generic_cross_call_bound_
    /// mismatch_is_error` above, but with a *user* trait rather than the
    /// library `Ord` -- `g` forwards its own `'T` to `shows` without
    /// declaring `Show`, so the callee's bound has no caller obligation to
    /// discharge against. Pins the real trait name rendering in `bound`'s
    /// `Bound::User(id)` arm, which had no coverage: collapsing it to the
    /// placebo `"a user trait"` string left the whole suite green.
    #[test]
    fn check_generic_cross_call_forwarded_user_bound_mismatch_is_error() {
        let err = check_src(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : g ( &'T -- ) shows ;\n\
             : main ( -- ) ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`'T` of `shows` requires `Show`, which `'T` in `g` does not declare"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k (R3): the same discharge against a *concrete* image runs the
    /// ordinary predicate on the spot, so the caller's own bounds are not the
    /// only route to a rejection. `Bool` is `Copy` but not `Ord`.
    ///
    /// P7.S3s (review fix): `Ord` is now a `Bound::User`, and `compose`'s own
    /// `resolve_user_bound` loop -- not the walk-time `Image::Concrete` arm,
    /// which defers to it (R3) -- is what catches this. `compose` only runs
    /// for a cross-call reached from `discover_transitive_instantiations`'s
    /// fixpoint, itself seeded from a real instantiation, so `g` must be
    /// called from `main`; the old fixture's `g` was dead code, which the
    /// old `is_ord` (no registry, no reachability gate) still caught but
    /// this design deliberately does not (R3's named residual).
    #[test]
    fn check_generic_cross_call_concrete_operand_failing_a_bound_is_error() {
        let err = check_src(
            ": biggest ['U: Ord] ( 'U -- 'U ) ;\n\
             : g ( 'T -- 'T ) True biggest drop ;\n\
             : main ( -- ) 1 g drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot instantiate `'U` of `biggest` with `Bool`")
                && err.contains("does not satisfy `Ord`"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k (N1, review fix): the `Copy` discharge indexes the registries to
    /// resolve a struct/enum image, and an instantiation this body's own walk
    /// minted is not in them yet -- `check::check` appends it only after
    /// `check_poly_body` returns. Both fixtures panicked inside `is_copy`'s
    /// indexing before the guard, which N1 forbids outright; both are now
    /// located. `Ord` needs no arm here (`is_ord` reads no registry) and a user
    /// bound never reaches the loop (`poly_cross_signature_supported` rejects
    /// one first).
    ///
    /// The wider bug is not this slice's: `dup` on the same body-local
    /// instantiation still panics through `poly_is_copy`, on a program with no
    /// cross-call at all. See the spec's phase 2 finding 7.
    #[test]
    fn check_cross_call_copy_bound_on_a_body_local_instantiation_is_unsupported() {
        for (wrapper, ctor, rendered) in [
            ("type: Box['T] | Box 'T ;\n", "Box", "Box[i64]"),
            ("type: Cell['T] val 'T ;\n", "Cell", "Cell[i64]"),
        ] {
            let src = format!(
                "{wrapper}\
                 : h ['U: Copy] ( 'U -- ) drop ;\n\
                 : g ( 'T -- 'T ) 1 {ctor} h ;\n\
                 : main ( -- ) ;\n"
            );
            let err = check_src(&src).unwrap_err();
            assert!(
                err.contains(&format!(
                    "discharging `Copy` on the body-local generic instantiation `{rendered}`"
                )) && err.contains("is not yet supported from a polymorphic body"),
                "unexpected message for {ctor}: {err}"
            );
        }
    }

    /// P7.S3k (R6): the caller wraps its own `'T` before handing it over, so
    /// the image of the callee's bare `'U` is a compound over a caller
    /// variable -- the growing case, rejected at the call site.
    ///
    /// The wrapper is a generic **enum**, deliberately. An array wrapper
    /// would be a placebo: the `[Type; Count]` array constructor is deleted
    /// (P7.S5), so array construction in a polymorphic body has no surface
    /// syntax and the growth rule would never be consulted. Sooth has no
    /// generic structs, so a single-variant generic enum is the only
    /// constructible wrapper. Do not add an array-based "second witness".
    #[test]
    fn check_growing_cross_call_is_error() {
        let err = check_src(
            "type: Box['T] | Box 'T ;\n\
             : h ( 'U -- 'U ) ;\n\
             : g ( 'T -- ) Box h drop ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot pass `Box['T]` to `'U` of the polymorphic word `h`")
                && err.contains("builds a larger type at every hop"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k (R6, review fix): a *fully concrete* compound image (`&i64`,
    /// built by `&n` on a scalar `static`) mentions no caller variable, so it
    /// is not the growing case -- but folding it into `Image::Concrete` would
    /// mint a fresh `RefId`, and the poly-body walk holds no mutable ref
    /// registry to do that with. Pins the honest rejection
    /// (`poly_cross_call_unsupported_error`) against a regression back to the
    /// growth diagnostic, which would misdescribe this call as building a
    /// larger type at every hop when nothing wraps `'T` at all.
    #[test]
    fn check_growing_cross_call_concrete_reference_is_unsupported_not_growth() {
        let err = check_src(
            "static: n i64 = 0 ;\n\
             : h ( 'U -- ) drop ;\n\
             : g ( 'T -- 'T ) &n h ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot call the polymorphic word `h`")
                && err.contains("passing the concrete compound value `&i64`")
                && !err.contains("builds a larger type at every hop"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k (R2, review fix): two generic enums of the same arity (`Box`,
    /// `Wrap`) are still distinguished by `(is_enum, idx, module)`, not just
    /// arity -- the guard mutation a reviewer probed (weakening the compare
    /// to arity-only) makes this silently compose `Wrap`'s variable against
    /// `Box`'s argument instead of raising the ordinary mismatch below.
    #[test]
    fn check_generic_cross_call_same_arity_different_header_is_type_mismatch() {
        let err = check_src(
            "type: Box['T] | Box 'T ;\n\
             type: Wrap['U] | Wrap 'U ;\n\
             : unwrap ( Wrap['A] -- Wrap['A] ) ;\n\
             : g ( 'T -- ) Box unwrap drop ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("expected `Wrap['A]`") && err.contains("found `Box['T]`"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k (R2, review fix): the four structural guards in
    /// `poly_cross_match`/`poly_cross_relate` that no test reached. Each was
    /// deletable with the whole suite green, and each fails *open* -- the
    /// operand-count guard into a subtract-overflow panic, the other three into
    /// an accepted call: a `Bool` filling an `i64` parameter, a shared borrow
    /// filling a mutable one, and a `array['T 4]` filling a `array['U 3]`. Table-driven
    /// because the four share one shape (a callee the caller's operands cannot
    /// fill) and differ only in which conjunct rejects them.
    #[test]
    fn check_cross_call_operands_the_callee_cannot_accept_are_errors() {
        for (fixture, expected) in [
            (
                ": h ( 'U 'U 'U -- ) drop drop drop ;\n\
                 : g ( 'T -- ) h ;\n: main ( -- ) ;\n",
                "`h` needs 3 values, but the stack holds 1",
            ),
            (
                ": h ( 'U i64 -- 'U ) drop ;\n\
                 : g ( 'T -- 'T ) True h ;\n: main ( -- ) ;\n",
                "`h` expected `i64`, found `Bool`",
            ),
            (
                ": h ( &!'U -- ) drop ;\n\
                 : g ( &'T -- ) h ;\n: main ( -- ) ;\n",
                "`h` expected `&!'U`, found `&'T`",
            ),
            (
                ": h ( &array['U 3] -- ) drop ;\n\
                 : g ( &array['T 4] -- ) h ;\n: main ( -- ) ;\n",
                "`h` expected `array['U 3]`, found `array['T 4]`",
            ),
        ] {
            let err = check_src(fixture).unwrap_err();
            assert!(err.contains(expected), "expected `{expected}`, got: {err}");
        }
    }

    /// P7.S3k (R2, the consistency requirement): one callee variable matched
    /// against two different caller variables cannot be one type at any
    /// instantiation. The symbolic twin of `poly_var_conflict_error`, which the
    /// concrete path raises for the same shape against two ground types.
    #[test]
    fn check_inconsistent_cross_call_mapping_is_error() {
        let err = check_src(
            ": pair ( 'U 'U -- 'U 'U ) ;\n\
             : g ( 'A 'B -- 'A 'B ) pair ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("matched `'U` to both `'A` and `'B`"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k: the callee signature shapes a symbolic mapping cannot carry are
    /// each a located rejection naming that shape, not the whole-feature
    /// narrowing they replaced. Three of the four are pinned here (review
    /// fix: the row case was missing, contradicting the deviations doc's
    /// claim that it was already pinned); the fourth, a declared quotation
    /// parameter, is pinned separately by
    /// `poly_quotlit_against_legal_inline_quotation_param_rejects_at_the_cross_call`,
    /// which needs a `~[ ]` fixture this table's plain `check_src` shape
    /// cannot build. A fifth shape, a `Bound::User` on a mapped variable,
    /// used to be pinned here too; P7.S3s R2 deletes that rejection outright
    /// (`check_generic_cross_call_discharges_a_forwarded_user_bound` and
    /// `check_generic_cross_call_concrete_image_with_no_impl_is_a_located_error`
    /// pin the resolution it replaces), so it is no longer a member of this
    /// table.
    #[test]
    fn check_cross_call_unsupported_callee_shapes_name_themselves() {
        for (fixture, what) in [
            (
                ": alen ( array['E 'N] -- array['E 'N] usize ) len ;\n\
                 : g ( array['T 4] -- array['T 4] ) alen drop ;\n: main ( -- ) ;\n",
                "a length variable in the callee's signature",
            ),
            (
                "type: Box['T] | Box 'T ;\n\
                 : box ( 'U -- Box['U] ) Box ;\n\
                 : g ( 'T -- ) box drop ;\n: main ( -- ) ;\n",
                "returning the compound type `Box['U]` from a polymorphic word",
            ),
            (
                ": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;\n\
                 : g ['T: Copy] ( 'T -- 'T 'T 'T 'T ) dup2 ;\n: main ( -- ) ;\n",
                "calling a row-polymorphic word",
            ),
        ] {
            let src = format!("{SHOW}{fixture}");
            let err = check_src(&src).unwrap_err();
            assert!(
                err.contains(what) && err.contains("is not yet supported from a polymorphic body"),
                "expected `{what}`, got: {err}"
            );
        }
    }

    /// P7.S3s (R2): the headline capability -- a polymorphic body may call
    /// another polymorphic word carrying a `Bound::User` on a forwarded
    /// variable, and the composed callee's `trait_calls` resolves to the
    /// implementing word's own lowering symbol, not merely a program that
    /// type-checks. `g` forwards its own `'T: Show` to `shows`; `main`
    /// instantiates `g` at `Point`, and the fixpoint must ground `shows`'s
    /// body-recorded `show` obligation against that same `Point`.
    #[test]
    fn check_generic_cross_call_discharges_a_forwarded_user_bound() {
        let (module, _) = checked_like_a_build(&format!(
            "{SHOW}: shows ['T: Show] ( &'T -- ) show ;\n\
             : g ['T: Show] ( &'T -- ) shows ;\n\
             : main ( -- ) 1 2 Point |p| &p g p drop ;\n"
        ))
        .expect("the fixture checks");
        let composed = module
            .transitive_instantiations
            .iter()
            .find(|i| i.callee == "shows")
            .expect("g's cross-call to shows composed a monomorph");
        let resolved: Vec<&str> = composed.trait_calls.values().map(String::as_str).collect();
        assert_eq!(resolved, vec!["show;Show;0;Point"]);
    }

    /// P7.S3s (R3): the concrete-image half of the same loop. `g`'s body
    /// builds `Other` itself, a struct already in the registry with no
    /// `impl: Show`, and hands `&o` to `shows` -- the mapped operand is
    /// `Image::Concrete(Other)` at walk time, never a forwarded caller
    /// variable. `compose`'s `resolve_user_bound` loop is what raises this,
    /// not a code change to the `(Image::Concrete(_), Bound::User(_))`
    /// walk-time arm (R3): that arm still records `None` and defers.
    #[test]
    fn check_generic_cross_call_concrete_image_with_no_impl_is_a_located_error() {
        let err = check_src(&format!(
            "{SHOW}type: Other n i64 ;\n\
             : shows ['T: Show] ( &'T -- ) show ;\n\
             : g ( 'T -- ) drop 7 Other |o| &o shows o drop ;\n\
             : main ( -- ) 1 g ;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`Other` does not satisfy `Show`: no `( &Other -- )` found"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3k (R2/N1): a grounded cross-call records *only* the symbolic
    /// mapping. It mints no instantiation of its own, so nothing in the
    /// existing `Span`-keyed table moves -- phase 2's composition is what adds
    /// one, from this record.
    #[test]
    fn check_generic_cross_call_records_no_instantiation() {
        let (module, _) = checked_like_a_build(
            ": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) 1 g drop ;\n",
        )
        .expect("the fixture checks");
        // `main`'s call to `g` is the one instantiation; `g`'s call to `id`
        // is not one yet.
        let callees: Vec<&str> = module
            .instantiations
            .values()
            .map(|c| c.callee.as_str())
            .collect();
        assert_eq!(callees, vec!["g"]);
        assert_eq!(module.poly_cross_calls["g"].len(), 1);
    }

    /// The composed monomorphs of a checked source, symbol-keyed, and the
    /// routing map each concrete instantiation carries -- the two records
    /// phase 2's fixpoint writes.
    fn transitive_of(src: &str) -> (Vec<String>, Vec<Vec<String>>) {
        let (module, _) = checked_like_a_build(src).expect("the fixture checks");
        let mut routed: Vec<Vec<String>> = module
            .instantiations
            .values()
            .map(|inst| {
                let mut syms: Vec<String> =
                    inst.poly_calls.values().map(|c| c.symbol.clone()).collect();
                syms.sort();
                syms
            })
            .collect();
        routed.sort();
        (
            module
                .transitive_instantiations
                .iter()
                .map(|c| c.symbol.clone())
                .collect(),
            routed,
        )
    }

    /// P7.S3k (R4): the composition step. `main` instantiates `g` at `i64`,
    /// `g`'s body calls `id`, and the fixpoint grounds that record into a real
    /// `(id, i64)` monomorph -- routed at `g`'s own body span, and recorded
    /// flat so lowering emits one `IrFunc` for it.
    #[test]
    fn transitive_discovery_composes_the_callee_instantiation() {
        let (transitive, routed) =
            transitive_of(": id ( 'T -- 'T ) ;\n: g ( 'T -- 'T ) id ;\n: main ( -- ) 1 g drop ;\n");
        assert_eq!(transitive, vec!["sooth_mono_id__t0_i64"]);
        assert_eq!(routed, vec![vec!["sooth_mono_id__t0_i64"]]);
    }

    /// P7.S3k (R4): once per *distinct* instantiation the caller reaches. Two
    /// call sites at the same θ compose the same symbol, so the fixpoint's
    /// dedup collapses them -- without it lowering would emit the callee's
    /// `IrFunc` twice under one name and the assembler would refuse the module.
    #[test]
    fn transitive_discovery_dedups_repeated_instantiation_symbol() {
        let (transitive, routed) = transitive_of(
            ": id ( 'T -- 'T ) ;\n\
             : g ( 'T -- 'T ) id ;\n\
             : main ( -- ) 1 g drop 2 g drop ;\n",
        );
        assert_eq!(transitive, vec!["sooth_mono_id__t0_i64"]);
        // Both call sites still route -- deduping the *emit* set must not drop
        // either site's routing entry.
        assert_eq!(
            routed,
            vec![vec!["sooth_mono_id__t0_i64"], vec!["sooth_mono_id__t0_i64"]]
        );
    }

    /// P7.S3k (R4): and one *per* distinct θ. Three types, asymmetric with
    /// each other, since two cannot tell "sorted" from "the 50% of
    /// `HashMap` iteration orders that happen to already be sorted" -- the
    /// dedup source (`insts`) iterates a map, so an unsorted `transitive`
    /// vec is only randomized, not reliably out of order, at n=2.
    #[test]
    fn transitive_discovery_composes_one_monomorph_per_distinct_theta() {
        let (transitive, _) = transitive_of(
            ": id ( 'T -- 'T ) ;\n\
             : g ( 'T -- 'T ) id ;\n\
             : main ( -- ) 1 g drop \"x\" g drop 1 2 eq g drop ;\n",
        );
        assert_eq!(
            transitive,
            vec![
                "sooth_mono_id__t0_Bool",
                "sooth_mono_id__t0_i64",
                "sooth_mono_id__t0_str"
            ]
        );
    }

    /// P7.S3k (R5/N3): a mutual cycle terminates because the second visit to
    /// `(g, θ)` mints the symbol the first already claimed. One monomorph per
    /// word, and `g`'s own seed entry is not duplicated into the flat set.
    #[test]
    fn transitive_discovery_terminates_on_a_mutual_cycle() {
        let (transitive, _) = transitive_of(
            ": h ( 'U -- 'U ) g 0 drop ;\n\
             : g ( 'T -- 'T ) h 0 drop ;\n\
             : main ( -- ) 1 g drop ;\n",
        );
        assert_eq!(transitive, vec!["sooth_mono_h__t0_i64"]);
    }

    /// P7.S3k (R4, phase 1 finding 1): an `inline` callee is *spliced* at the
    /// call site and mints no `IrFunc`, so the fixpoint must compose nothing
    /// for it. Routing the span would emit a call to a symbol that never
    /// exists -- a link failure for a case that already worked. The record
    /// itself is still made: how `h` is lowered is not the record's business.
    #[test]
    fn transitive_discovery_skips_an_inline_callee() {
        let src = ": h inline ( 'U -- 'U ) ;\n: g ( 'T -- 'T ) h ;\n: main ( -- ) 1 g drop ;\n";
        let (module, _) = checked_like_a_build(src).expect("the fixture checks");
        assert_eq!(module.poly_cross_calls["g"].len(), 1);
        assert!(
            module.transitive_instantiations.is_empty(),
            "composed: {:?}",
            module.transitive_instantiations
        );
        for inst in module.instantiations.values() {
            assert!(inst.poly_calls.is_empty(), "routed: {:?}", inst.poly_calls);
        }
    }

    /// P7.S3k (R4, review finding 1): unlike `h` above, `h`'s own body here
    /// calls a polymorphic word. Nothing composes a θ for that inner call --
    /// `h` is never walked by `check_poly_body` at all (it is checked
    /// standalone, at a concrete dummy type), so the fixpoint has no record
    /// of what `h` calls -- and lowering really does splice `h`'s generic
    /// body at `g`'s call site. A located rejection, not the panic
    /// ("checked user word exists") or the silent wrong-symbol routing this
    /// gap used to reach.
    #[test]
    fn transitive_discovery_rejects_an_inline_callee_whose_body_calls_a_poly_word() {
        let src = ": id ( 'T -- 'T ) ;\n: h inline ( 'U -- 'U ) id ;\n: g ( 'T -- 'T ) h ;\n: main ( -- ) 1 g drop ;\n";
        let err =
            checked_like_a_build(src).expect_err("an inline callee's own cross-call is unroutable");
        assert!(
            err.contains("cannot call the polymorphic word `h`") && err.contains("inline"),
            "unexpected error: {err}"
        );
    }

    /// The same gap, one `inline` hop further: `g` calls `h`, `h` is `inline`
    /// and itself calls `k`, another `inline` word whose own body calls a
    /// polymorphic word. The rejection fires at `g`'s call to `h` -- a call
    /// to *any* polymorphic word, inline or not, is what `body_calls_a_poly_word`
    /// looks for, so it does not need to recurse into `k`'s body itself.
    #[test]
    fn transitive_discovery_rejects_an_inline_callee_two_hops_from_the_poly_call() {
        let src = ": id ( 'T -- 'T ) ;\n\
             : k inline ( 'V -- 'V ) id ;\n\
             : h inline ( 'U -- 'U ) k ;\n\
             : g ( 'T -- 'T ) h ;\n\
             : main ( -- ) 1 g drop ;\n";
        let err = checked_like_a_build(src).expect_err("a two-hop inline chain is unroutable too");
        assert!(
            err.contains("cannot call the polymorphic word `h`"),
            "unexpected error: {err}"
        );
    }

    /// P7.S3k (R4/R8): a composed callee returning two or more values carries
    /// its interned return bundle on *both* records -- the flat emit entry and
    /// the routing copy -- and they agree, since `intern_bundle_struct` dedups
    /// by output tuple. Only the routing copy is read today (`lower_poly_call`
    /// asks the call site what shape came back); pinned on both so a reader of
    /// `transitive_instantiations` cannot find the invariant broken on one.
    #[test]
    fn transitive_discovery_interns_a_composed_bundle_on_both_records() {
        let (module, _) = checked_like_a_build(
            ": two ( 'T -- 'T i64 ) 9 ;\n\
             : g ( 'T -- 'T i64 ) two ;\n\
             : main ( -- ) 1 g drop drop ;\n",
        )
        .expect("the fixture checks");
        let [composed] = &module.transitive_instantiations[..] else {
            panic!(
                "one composed callee: {:?}",
                module.transitive_instantiations
            );
        };
        assert_eq!(composed.out_arity, 2);
        let bundle = composed.bundle.expect("the flat entry carries its bundle");
        let routed: Vec<Option<StructId>> = module
            .instantiations
            .values()
            .flat_map(|inst| inst.poly_calls.values().map(|c| c.bundle))
            .collect();
        assert_eq!(routed, vec![Some(bundle)]);
    }

    /// P7.S3k (R4/R2): a two-variable callee reached *both* from a concrete
    /// caller and through a cross-call is monomorphized once. That holds only
    /// if a composed θ orders its entries the way `unify_poly_input` does for
    /// the concrete path, since `instantiation_symbol` reads them positionally.
    /// Asymmetric (`i64`, `str`) on purpose: a symmetric pair cannot tell the
    /// two orders apart.
    #[test]
    fn transitive_discovery_shares_a_callee_monomorph_with_the_concrete_path() {
        let (module, _) = checked_like_a_build(
            ": swap2 ( 'A 'B -- 'B 'A ) swap ;\n\
             : g ( 'X 'Y -- 'Y 'X ) swap2 ;\n\
             : main ( -- ) 1 \"s\" swap2 drop drop 2 \"t\" g drop drop ;\n",
        )
        .expect("the fixture checks");
        let concrete: Vec<&str> = module
            .instantiations
            .values()
            .filter(|i| i.callee == "swap2")
            .map(|i| i.symbol.as_str())
            .collect();
        assert_eq!(concrete, vec!["sooth_mono_swap2__t0_i64_t1_str"]);
        let routed: Vec<&str> = module
            .instantiations
            .values()
            .flat_map(|inst| inst.poly_calls.values().map(|c| c.symbol.as_str()))
            .collect();
        assert_eq!(routed, concrete, "the cross-call reaches that same symbol");
        // And so needs no monomorph of its own: the fixpoint's seed dedup
        // already claimed it. A composed θ ordered the other way would mint
        // `..._t0_str_t1_i64`, miss the dedup, and land a second `IrFunc` here.
        assert!(
            module.transitive_instantiations.is_empty(),
            "composed: {:?}",
            module.transitive_instantiations
        );
    }

    /// P7.S3k (N2): a program with no cross-call records composes nothing and
    /// routes nothing, which is what makes the emitted IL of the existing
    /// corpus identical by construction rather than by inspection.
    #[test]
    fn transitive_discovery_is_empty_without_a_cross_call() {
        let (transitive, routed) =
            transitive_of(": id ( 'T -- 'T ) ;\n: main ( -- ) 1 id drop ;\n");
        assert!(transitive.is_empty(), "composed: {transitive:?}");
        assert_eq!(routed, vec![Vec::<String>::new()]);
    }

    /// P7.S3k (R4/N1, phase 1 finding 2): a polymorphic overload set merges
    /// into one `poly_cross_calls` entry, and each record's mapping indexes its
    /// *own* candidate's variables -- so composing one candidate's mapping
    /// against the other's θ would mint a wrong monomorph silently. Rejected,
    /// located, from either side of the call.
    #[test]
    fn check_cross_call_through_an_overloaded_generic_word_is_error() {
        for (fixture, overloaded) in [
            (
                ": id ( 'T -- 'T ) ;\n\
                 : f ( 'T -- 'T ) id ;\n\
                 : f ( 'A 'B -- 'A 'B ) swap swap ;\n\
                 : main ( -- ) 1 f drop ;\n",
                "`f` names more than one polymorphic word",
            ),
            (
                ": id ( 'T -- 'T ) ;\n\
                 : id ( 'A 'B -- 'A 'B ) swap swap ;\n\
                 : g ( 'T -- 'T ) id ;\n\
                 : main ( -- ) 1 g drop ;\n",
                "`id` names more than one polymorphic word",
            ),
        ] {
            let err = check_src(fixture).unwrap_err();
            assert!(
                err.contains(overloaded),
                "expected `{overloaded}`, got: {err}"
            );
        }
    }

    #[test]
    fn check_poly_body_with_if_accepts_choose() {
        // T1: a polymorphic body may branch. Slice 10c: `inline`, because
        // `if` is an ordinary word taking two quotation literals and a
        // non-spliced polymorphic body rejects a quotation outright; the
        // branch now runs through the ordinary splice, which is what makes the
        // move-join below the thing under test. `choose` consumes `a` and `b` on
        // both arms but at different sites; the move-join must recognise
        // `Moved`+`Moved` as consumed-once (not a leak), or `choose` would be
        // wrongly rejected at the word end (M1).
        assert!(
            check_src(
                ": choose inline ( 'T 'T Bool -- 'T ) | a b flag | flag ~[ a b drop ] ~[ b a drop ] if ;\n: main ( -- ) 1 2 True choose drop ;",
            )
            .is_ok(),
            "choose should type-check"
        );
    }
    /// Slice 10c: T2/T4/T5/T8 below drove `poly_walk`'s own `PolyType` move
    /// tracker, which went with its branch arm -- `if` is an ordinary word
    /// taking two quotations now, and a spliced body's arms are tracked by the
    /// shared branch-and-join over concrete `Slot`s. Each is retargeted onto a
    /// concrete linear type, so the guarantee it pinned (an arm-local leak, a
    /// one-armed consume joining to `MaybeMoved`, a use after a `Moved` join)
    /// is still guarded, at the site that now decides it. A stand-in `'T` no
    /// longer works: the def-site check instantiates it at `i64`, which is
    /// `Copy`, so nothing could leak.
    #[test]
    fn check_arm_local_unconsumed_is_error() {
        // T2: `y` is bound inside the `then` arm and never consumed in it.
        let err = check_src(&format!(
            "{SPY}: arm_leak ( Spy Spy Bool -- Spy ) | a b flag | flag ~[ a b | y | ] ~[ a drop b ] if ;\n: main ( -- ) ;",
        ))
        .unwrap_err();
        assert!(err.contains('y'), "names the arm-local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }
    #[test]
    fn check_poly_branch_moved_on_both_arms_is_accepted() {
        // T3: `a`/`b` consumed on both arms (`Moved`+`Moved` => `Moved`), so
        // nothing leaks at the word end.
        assert!(
            check_src(
                ": both inline ( 'T 'T Bool -- ) | a b flag | flag ~[ a drop b drop ] ~[ b drop a drop ] if ;\n: main ( -- ) ;",
            )
            .is_ok(),
            "both should type-check"
        );
    }
    #[test]
    fn check_branch_moved_on_one_arm_leaks() {
        // T4: `x` consumed on the `then` arm only (`Moved`+`Live` =>
        // `MaybeMoved`), which the leak check must count as still-unconsumed
        // (M3).
        let err = check_src(&format!(
            "{SPY}: one ( Spy Bool -- ) | x flag | flag ~[ x drop ] ~[ ] if ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(
            err.contains("is not consumed on every path"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_branch_moved_on_neither_arm_leaks() {
        // T5: `x` untouched on both arms (`Live`+`Live` => `Live`); a value
        // parked in a local across a branch still leaks at the word end (M4).
        let err = check_src(&format!(
            "{SPY}: none ( Spy Bool -- ) | x flag | flag ~[ ] ~[ ] if ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }
    #[test]
    fn check_branch_condition_not_bool_is_error() {
        // T6: `if`'s condition must be a `Bool`. Slice 10c: the guard is now
        // `if`'s own declared parameter type rather than a hand-written arm,
        // and a spliced poly body reports the operand at its instantiated
        // stand-in type.
        let err =
            check_src(": bad inline ( 'T 'T -- 'T ) ~[ drop ] ~[ drop ] if ;\n: main ( -- ) ;")
                .unwrap_err();
        assert!(err.contains("if"), "names the `if`: {err}");
        assert!(err.contains("`Bool`"), "names the expected type: {err}");
    }
    #[test]
    fn check_branch_depth_mismatch_is_error() {
        // T7: the arms leave different stack depths (then: 1, else: 2). Slice
        // 10c catches that at the *argument* site (R-P2-3), comparing one arm
        // literal's actual exit shape against its sibling's, rather than at a
        // join after both were walked. `'T`
        // carries a `Copy` bound so the repeated reads are not use-after-move,
        // leaving the depth mismatch as the sole failure this test proves.
        let err = check_src(
            ": bad inline ['T: Copy] ( 'T Bool -- 'T ) | x flag | flag ~[ x ] ~[ x x ] if ;\n: main ( -- ) ;",
        )
        .unwrap_err();
        assert!(
            err.contains("leave different stack shapes"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_branch_use_after_join_is_error() {
        // T8: both arms consume `x` (the join is `Moved`), so the `x drop`
        // after the branch is a second read: use-after-move, not a leak.
        let err = check_src(&format!(
            "{SPY}: bad ( Spy Bool -- ) | x flag | flag ~[ x drop ] ~[ x drop ] if x drop ;\n: main ( -- ) ;"
        ))
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
            check_src(": bad ( array['T 'N] -- array['T 'N] array['T 'N] ) dup ;\n: main ( -- ) ;")
                .unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Copy"), "unexpected message: {err}");
    }
    #[test]
    fn check_poly_dup_of_linear_element_array_names_element_type() {
        // `poly_copy_gate` array arm: `dup` of a length-variable array whose
        // element is a concrete linear struct names that struct, never `i64`.
        let err = check_src(&format!(
            "{SPY}: bad ( array[Spy 'N] -- array[Spy 'N] array[Spy 'N] ) dup ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
        assert!(err.contains("Spy"), "unexpected message: {err}");
        assert!(err.contains("linear"), "unexpected message: {err}");
    }
    #[test]
    fn check_poly_dup_of_an_owning_cell_is_rejected() {
        // P7.S3n (R3): both halves of the cell's `Copy` answer at once.
        // `poly_is_copy` must say `false` -- a `^T` owns its payload at every
        // instantiation, so answering `true` would let two names free one
        // allocation -- and `poly_copy_gate` must then have an arm to render
        // it, since without one it falls to whichever sibling arm claims the
        // shape.
        let err = check_src(": bad ( ^'T -- ^'T ^'T ) dup ;\n: main ( -- ) ;").unwrap_err();
        assert_eq!(
            err,
            "error: cannot `dup` an owning cell in `bad` (line 1)\n  `^'T` is linear: it owns its payload, so duplicating it would free the same allocation twice"
        );
    }
    #[test]
    fn poly_op_on_variable_error_names_a_reference() {
        // Slice 13 (review fix): `poly_op_on_variable_error`'s `Ref` describer
        // (`"a reference"`) is reachable from source -- `len` rejects a
        // reference the same way it rejects a bare type variable -- but had
        // no test asserting the exact wording.
        let err = check_src(": f ( &array['T 4] -- usize ) len ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: `len` is not permitted on a reference in `f` (line 1)"
        );
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
        let arrays: [ArrayDecl; 0] = [];
        let cells: [OwnedCellDecl; 0] = [];
        let refs: [RefDecl; 0] = [];
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let mut subst = Subst::default();
        unify_poly_input(
            &sig,
            &sig.inputs[0],
            quotation_type(vec![Type::I64], Vec::new()),
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
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
            &cells,
            &refs,
            &mut subst2,
            &[],
        )
        .expect_err("an arity mismatch must be a located type mismatch");
        // Slice 10a (R10/R20): pin the *exact* mismatch text. The expected
        // side must render the declared `PolyType` (`[ 'T -- ]`) through
        // `poly_type_str`, never a fabricated `[ -- ]`; a substring like
        // "`f`" would survive that rendering vanishing, so it is not enough.
        assert_eq!(
            err,
            "error: type mismatch in `probe` (line 0)\n  `f` expected `[ 'T -- ]`, found `[ i64 i64 -- ]`\n  note: declared ( -- )",
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
            &cells,
            &refs,
            &mut subst3,
            &[],
        )
        .expect_err("a non-quotation slot must be a located type mismatch");
        assert_eq!(
            err,
            "error: type mismatch in `probe` (line 0)\n  `f` expected `[ 'T -- ]`, found `i64`\n  note: declared ( -- )",
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

    /// A signature over one type variable `'T` and one length variable `'N`,
    /// the shape every Slice 13 reference test names its referent from.
    fn ref_sig() -> PolySig {
        PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            len_var_names: vec!["'N".to_string()],
            row_var_names: Vec::new(),
        }
    }

    fn poly_ref(referent: PolyType, mutable: bool) -> PolyType {
        PolyType::Ref(Box::new(referent), mutable)
    }

    #[test]
    fn poly_type_str_renders_a_reference() {
        // Slice 13 (R-A9): the sigil is glued to the referent's own rendering,
        // so a poly reference reads back exactly as it was written.
        let sig = ref_sig();
        assert_eq!(
            poly_type_str(&poly_ref(PolyType::Var(0), false), &sig),
            "&'T"
        );
        assert_eq!(
            poly_type_str(&poly_ref(PolyType::Var(0), true), &sig),
            "&!'T"
        );
        let arr = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        assert_eq!(poly_type_str(&poly_ref(arr, false), &sig), "&array['T 4]");
    }

    #[test]
    fn poly_type_str_renders_a_generic_application() {
        // P7 slice 3a: `Name['A 'B]` in the signature's own variable
        // spellings -- `name` is cached on the variant, so no registry
        // lookup is needed to render it.
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string(), "'E".to_string()],
            len_var_names: Vec::new(),
            row_var_names: Vec::new(),
        };
        let result = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0), PolyType::Var(1)],
            name: "Result",
        };
        assert_eq!(poly_type_str(&result, &sig), "Result['T 'E]");
    }

    #[test]
    fn poly_generic_receiver_is_aggregate_projection() {
        // P7 slice 3a: an ungrounded generic application is a struct or enum
        // header, so it is a projection receiver exactly as a concrete
        // `Type::Struct`/`Type::Enum` is (R1's table).
        let stack = vec![PolySlot::new(PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: Vec::new(),
            name: "Result",
        })];
        assert!(receiver_is_aggregate_projection(&stack));
    }

    #[test]
    fn poly_generic_slot_is_not_copy() {
        // P7 slice 3a (D5): a generic applied to a variable is
        // conservatively linear -- `dup`/`over` on it is rejected outright,
        // never derived per-argument.
        let err =
            check_src("type: Box['T] val 'T ;\n: dup-box ( Box['T] -- Box['T] Box['T] ) dup ;\n")
                .unwrap_err();
        assert_eq!(
            err,
            "error: cannot `dup` a generic type applied to a variable in `dup-box` (line 2)\n  `Box['T]` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated"
        );
    }

    #[test]
    fn declared_poly_reference_signature_round_trips() {
        // Slice 13 (R-A10, the Part A exit criterion): a poly word may
        // *declare* a borrow, and the declaration survives parse + fold +
        // rendering unchanged. Producing one is Part B.
        let tokens = lex(": peek ( array['T 4] -- &array['T 4] ) ;").unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(poly_type_str(&sig.outputs[0], sig), "&array['T 4]");
    }

    /// P7 slice 3c (R8.3): a slice inside a polymorphic body is the point of
    /// the type, so each poly predicate is pinned over one. All four answer
    /// through `PolyType::Concrete` delegation -- the element is concrete by
    /// construction, so a slice never takes a poly shape of its own -- and
    /// that delegation is exactly what these guard: deleting the monomorphic
    /// `is_copy`/`is_ref` slice arms breaks the poly path too, silently.
    #[test]
    fn poly_is_copy_mutable_slice_is_not() {
        let mut slices = Vec::new();
        let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
        let sig = bare_sig();
        assert!(poly_is_copy(
            &PolyType::Concrete(shared),
            &sig,
            &[],
            &[],
            &[]
        ));
        assert!(!poly_is_copy(
            &PolyType::Concrete(mutable),
            &sig,
            &[],
            &[],
            &[]
        ));
        // The gate `dup`/`over` run: a mutable view is refused with the
        // exclusivity wording, not the linear-ownership wording, since a view
        // owns nothing.
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let err = poly_copy_gate(
            &PolyType::Concrete(mutable),
            "dup",
            &sig,
            &ctx,
            Span::default(),
            &[],
            &[],
            &[],
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot `dup` a value of type `!Slice[i64]` in `probe` (line 0)\n  `!Slice[i64]` is exclusive: at most one may be live for a place, so copying it would make a second one; use it where it is, or borrow again once it is consumed\n  note: declared ( -- )"
        );
        poly_copy_gate(
            &PolyType::Concrete(shared),
            "dup",
            &sig,
            &ctx,
            Span::default(),
            &[],
            &[],
            &[],
        )
        .expect("a shared view is `Copy`");
    }

    /// P7 slice 3c (R8.3): a live slice keeps the borrow it was built from
    /// observable, so `prune_dead_borrows` must not forget that borrow while
    /// one is on the stack.
    #[test]
    fn is_reference_slot_true_for_slice() {
        let mut slices = Vec::new();
        let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
        assert!(is_reference_slot(&PolyType::Concrete(shared)));
        assert!(is_reference_slot(&PolyType::Concrete(mutable)));
        assert!(!is_reference_slot(&PolyType::Concrete(Type::I64)));
    }

    /// P7 slice 3c (R8.3): the poly renderer spells a slice the way the
    /// signature does, so a diagnostic naming one is copy-pasteable source.
    #[test]
    fn poly_type_str_renders_slice() {
        let mut slices = Vec::new();
        let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let mutable = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
        let sig = bare_sig();
        assert_eq!(
            poly_type_str(&PolyType::Concrete(shared), &sig),
            "Slice[i64]"
        );
        assert_eq!(
            poly_type_str(&PolyType::Concrete(mutable), &sig),
            "!Slice[i64]"
        );
    }

    /// P7 slice 3c (R9.1, poly half): `len` answers a slice's carried length
    /// and consumes the slot, like `str` and unlike the array arms -- an array
    /// is a place that stays put, a slice is a value on the stack, so leaving
    /// it would strand a residual slot in `0 s len >i64`.
    #[test]
    fn poly_len_over_a_slice_ok() {
        let mut slices = Vec::new();
        let shared = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let sig = bare_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let mut overloads = HashMap::new();
        let stack = poly_term(
            &Term {
                kind: TermKind::Call("len".to_string(), Vec::new()),
                span: Span::default(),
            },
            vec![PolySlot::new(PolyType::Concrete(shared))],
            &mut scope,
            &sig,
            &ctx,
            &env,
            &CombinatorEnv::default(),
            &[],
            &[],
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut overloads,
            &mut TraitCtx::scratch(&mut Vec::new(), &mut Vec::new()),
            scratch_cross!(),
            false,
        )
        .expect("`len` answers a slice");
        assert_eq!(
            stack.iter().map(|s| s.pt.clone()).collect::<Vec<_>>(),
            vec![PolyType::Concrete(Type::Usize)]
        );
    }

    /// P7 slice 3c (R9.2/R10, phase 4): the poly walk's own slice arms --
    /// `&>` on a view, `subslice`, and `slice` off a body borrow. Phase 3 left
    /// all three as rejections (`&>` fell to `poly_op_on_variable_error`,
    /// the two words to `unknown word`), so a generic body could `len` a view
    /// and nothing else.
    #[test]
    fn poly_slice_words_index_subrange_and_construct() {
        // A view indexed inside a generic body yields an element reference,
        // and the sub-view keeps the receiver's own type.
        check_src(": f ( Slice[i64] 'T -- i64 'T ) | x | 0 >usize &> @ x ;\n").unwrap();
        check_src(": f ( Slice[i64] 'T -- usize 'T ) | x | 0 >usize 2 >usize subslice len x ;\n")
            .unwrap();
        // `slice` off a borrow taken in the body: the length may be generic
        // (a view erases it into a runtime length), the element may not.
        check_src(": f ( array[i64 3] 'T -- array[i64 3] usize 'T ) | x | | a | &a slice len a swap x ;\n")
            .unwrap();
        check_src(
            ": f ( array['T 3] 'T -- array['T 3] usize 'T ) | x | | a | &a slice len a swap x ;\n",
        )
        .unwrap_err();
        // ...and the view it builds inherits the borrow's mutability: a
        // shared one could not be written through here.
        check_src(
            ": f ( array[i64 3] i64 'T -- array[i64 3] 'T ) | x | | v | | a | \
             &!a slice 0 >usize &!> v ! a x ;\n",
        )
        .unwrap();
    }

    /// R1.2: a generic *element* is a locked non-goal, and the rejection says
    /// so by name rather than reporting an unrelated shape mismatch -- a
    /// generic *length* is fine, which is the whole point of a view.
    #[test]
    fn poly_slice_over_a_generic_element_is_a_located_error() {
        let err = check_src(
            ": f ( array['T 3] 'T -- array['T 3] usize 'T ) | x | | a | &a slice len a swap x ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `slice` over an array of `'T` in `f` (line 1) is not supported\n  \
             a view's element type must be concrete; only its length may be generic"
        );
    }

    /// R9.2 (poly half): the mutability of a slice receiver is part of the
    /// match, exactly as it is on the concrete path, and the wording is the
    /// same one -- the two paths must not disagree about one spelling. (The
    /// empty `note: declared ( -- )` is the poly path's own: a polymorphic
    /// word carries no concrete effect for the note to render, and every
    /// mismatch it reports says the same.)
    #[test]
    fn poly_index_of_a_slice_matches_on_mutability() {
        check_src(": f ( !Slice[i64] i64 'T -- 'T ) | x | | v | 0 >usize &!> v ! x ;\n").unwrap();
        let err =
            check_src(": f ( !Slice[i64] 'T -- i64 'T ) | x | 0 >usize &> @ x ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: type mismatch in `f` (line 1)\n  \
             `&>` expected a slice, found `!Slice[i64]`\n  \
             note: declared ( -- )"
        );
        let err = check_src(": f ( Slice[i64] i64 'T -- 'T ) | x | | v | 0 >usize &!> v ! x ;\n")
            .unwrap_err();
        assert!(
            err.contains("`&!>` expected a mutable slice, found `Slice[i64]`"),
            "unexpected message: {err}"
        );
    }

    /// The poly twin of `check_slice_offset`: a `usize` passes, an `i64`
    /// literal passes (the monomorphic path admits one too), a computed `i64`
    /// needs the explicit conversion, and a bare variable is a located error.
    /// Unlike the array twin there is no count to bound a literal against.
    #[test]
    fn check_poly_slice_offset_admits_usize_and_literals_only() {
        let sig = ref_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let span = Span::default();
        let slot = |pt: PolyType, lit: Option<i64>| PolySlot {
            pt,
            int_val: lit,
            quot: None,
        };
        check_poly_slice_offset(
            &slot(PolyType::Concrete(Type::Usize), None),
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect("a `usize` offset passes");
        check_poly_slice_offset(
            &slot(PolyType::Concrete(Type::I64), Some(9999)),
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect("a literal needs no compile-time bound: the trap is at runtime");
        check_poly_slice_offset(
            &slot(PolyType::Concrete(Type::I64), None),
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect_err("a computed i64 needs the explicit `>usize`");
        check_poly_slice_offset(&slot(PolyType::Var(0), None), &ctx, span, "&>", &sig)
            .expect_err("a bare type variable is not an offset");
    }

    /// Call-site witness for `check_poly_slice_offset`: the direct-call test
    /// above proves the function's own logic, but not that the three sites
    /// wiring it in (`subslice`'s start and length, `&>`'s index) actually
    /// call it. A computed (non-literal) `i64` operand at each site must be
    /// rejected the same way a bare direct call is.
    #[test]
    fn poly_slice_offset_sites_reject_a_computed_i64() {
        let err = check_src(
            ": f ( Slice[i64] i64 'T -- usize 'T ) | x | | k | k 2 >usize subslice len x ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`subslice` mixes `usize` with a computed `i64`"),
            "unexpected message: {err}"
        );
        let err = check_src(
            ": f ( Slice[i64] i64 'T -- usize 'T ) | x | | k | 0 >usize k subslice len x ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`subslice` mixes `usize` with a computed `i64`"),
            "unexpected message: {err}"
        );
        let err =
            check_src(": f ( Slice[i64] i64 'T -- i64 'T ) | x | | k | k &> @ x ;\n").unwrap_err();
        assert!(
            err.contains("`&>` mixes `usize` with a computed `i64`"),
            "unexpected message: {err}"
        );
    }

    /// R12 (poly half): a mutable view is exclusivity-tracked in a generic
    /// body too -- but by the poly walk's *move* tracking, not by a reborrow:
    /// a non-`Copy` local is consumed on read there, so a mutable view is
    /// single-use per binding where the concrete path reborrows it. Ruled on
    /// here rather than discovered in a golden (see the phase's exit notes).
    #[test]
    fn poly_mutable_slice_local_is_single_use() {
        check_src(": f ( Slice[i64] 'T -- usize usize 'T ) | x | | s | s len s len x ;\n").unwrap();
        let err =
            check_src(": f ( !Slice[i64] 'T -- usize usize 'T ) | x | | s | s len s len x ;\n")
                .unwrap_err();
        assert!(
            err.contains("use after move in `f`") && err.contains("local `s` is linear"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn poly_is_copy_tracks_a_reference_mutability_not_its_referent() {
        // Slice 13 (D3/R-A5): mirrors the monomorphic `is_copy` on
        // `Type::Ref` -- shared is `Copy`, mutable is not, and the referent's
        // own linearity is irrelevant either way. Answering `True`
        // unconditionally would let a generic body freely `dup` an exclusive
        // borrow, an acceptance every concrete instantiation rejects.
        let sig = ref_sig();
        let linear_referent = PolyType::Var(0); // no `Copy` bound
        assert!(poly_is_copy(
            &poly_ref(linear_referent.clone(), false),
            &sig,
            &[],
            &[],
            &[]
        ));
        assert!(!poly_is_copy(
            &poly_ref(linear_referent, true),
            &sig,
            &[],
            &[],
            &[]
        ));
    }

    #[test]
    fn poly_copy_gate_rejects_a_mutable_reference() {
        // Slice 13 (E1): the gate's reference arm is a real located
        // diagnostic, not an `unreachable!` -- `dup` on a `&!` must reject,
        // and on a `&` must still pass (the positive control).
        let sig = ref_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let span = Span {
            line: 7,
            col: 3,
            ..Span::default()
        };
        let err = poly_copy_gate(
            &poly_ref(PolyType::Var(0), true),
            "dup",
            &sig,
            &ctx,
            span,
            &[],
            &[],
            &[],
        )
        .expect_err("`dup` of a mutable reference must be rejected");
        assert_eq!(
            err,
            "error: cannot `dup` a mutable reference in `probe` (line 7)\n  `&!'T` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow",
        );
        poly_copy_gate(
            &poly_ref(PolyType::Var(0), false),
            "dup",
            &sig,
            &ctx,
            span,
            &[],
            &[],
            &[],
        )
        .expect("`dup` of a shared reference is permitted");
    }

    #[test]
    fn unify_poly_input_matches_a_declared_reference_slot() {
        // Slice 13 (R-A6): a declared `&array['T 4]` binds `'T` through the
        // registry's referent; a mutability mismatch and a non-reference slot
        // are located mismatches, never a silent bind.
        let sig = ref_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let cells: [OwnedCellDecl; 0] = [];
        let mut refs: Vec<RefDecl> = Vec::new();
        let shared = crate::ast::intern_ref_type(&mut refs, arr_ty, false);
        let mutable = crate::ast::intern_ref_type(&mut refs, arr_ty, true);
        let declared = poly_ref(
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4)),
            false,
        );

        let mut subst = Subst::default();
        unify_poly_input(
            &sig,
            &declared,
            shared,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
        )
        .expect("`&array['T 4]` should unify against `&array[i64 4]`");
        assert_eq!(subst.ty_of(0), Some(Type::I64), "`'T` should bind to `i64`");

        let mut subst2 = Subst::default();
        let err = unify_poly_input(
            &sig,
            &declared,
            mutable,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst2,
            &[],
        )
        .expect_err("a mutability mismatch must be a located type mismatch");
        assert_eq!(
            err,
            "error: type mismatch in `probe` (line 0)\n  `f` expected `&array['T 4]`, found `&!array[i64 4]`\n  note: declared ( -- )",
        );
        assert!(
            subst2.ty_of(0).is_none(),
            "a mutability mismatch must not silently bind `'T`"
        );

        let mut subst3 = Subst::default();
        let err = unify_poly_input(
            &sig,
            &declared,
            arr_ty,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst3,
            &[],
        )
        .expect_err("a non-reference slot must be a located type mismatch");
        assert_eq!(
            err,
            "error: type mismatch in `probe` (line 0)\n  `f` expected `&array['T 4]`, found `array[i64 4]`\n  note: declared ( -- )",
        );
        assert!(
            subst3.ty_of(0).is_none(),
            "a non-reference slot must not silently bind `'T`"
        );
    }

    #[test]
    fn apply_subst_grounds_a_reference_by_interning() {
        // Slice 13 (R-A7/D4): grounding is what mints the `RefId` -- the
        // shape may be one no call site has interned yet, so the check side
        // interns it (and the lowering side then only looks it up).
        let sig = ref_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let ty = apply_subst(
            &sig,
            &poly_ref(PolyType::Var(0), true),
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect("a bound referent grounds");
        assert_eq!(ty.name(), "&!i64");
        assert_eq!(refs.len(), 1, "the shape must be interned exactly once");
        assert_eq!(refs[0].referent, Type::I64);
        assert!(refs[0].mutable);
    }

    // -- Phase 2 (R-B1..R-B6): production and checking --------------------

    #[test]
    fn first_reads_an_array_element_through_a_poly_borrow() {
        // R-B2/R-B3/R-B4: the P2 read witness type-checks -- `&a` borrows
        // the aggregate local, `0` is a literal index bounds-checked against
        // the concrete length 4, and `@` fetches the `Copy` element.
        check_src(
            ": first['T: Copy] ( array['T 4] -- 'T ) | a | &a 0 &> @ ;\n\
             : main ( -- ) 10 4 fill first drop ;\n",
        )
        .expect("a shared prefix borrow, array-element ref, and fetch should check");
    }

    #[test]
    fn poly_reference_word_rejects_borrowing_a_bare_variable_local() {
        // E2/D5: a bare `'T` local might instantiate to a scalar, which has
        // no address, so it is refused uniformly rather than deferred.
        let err = check_src(": badvar ['T: Copy] ( 'T -- 'T )\n  | t |\n  &t\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: cannot borrow the local `t` of type `'T` in `badvar` (line 3, col 3)\n  `'T` might instantiate to a scalar, which has no address; borrow an aggregate (a struct, enum, array, or owning cell) instead"
        );
    }

    #[test]
    fn poly_reference_word_rejects_borrowing_a_concrete_scalar_local() {
        // Phase 2 review: D5's aggregate gate has two arms and only the
        // bare-variable one was covered. A concrete scalar local is not an
        // aggregate either, and takes the non-variable arm.
        let err =
            check_src(": g ['T: Copy] ( i64 'T -- 'T ) | n t | &n drop n drop t ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: cannot borrow the local `n` of type `i64` in `g` (line 1, col 41)\n  only an aggregate (a struct, enum, array, or owning cell) is borrowable; `i64` is not"
        );
    }

    #[test]
    fn borrowing_a_quotation_local_is_rejected() {
        // R-B8's `&q` witness. UPDATED after the slice 12 rebase: slice 12
        // retired `is_combinator`'s quotation-parameter inference leg (a word
        // now splices only when it *declares* `inline`), so `ap`'s ordinary,
        // non-`inline` `[ 'T -- 'T ]` parameter no longer makes it a
        // combinator -- it is checked as a genuine poly body, and
        // `poly_reference_word` itself rejects the quotation-typed local `f`
        // directly, rather than the splice path naming a monomorphic
        // instantiation. Second update (review): a quotation gets its own
        // wording (`poly_borrow_of_quotation_local_error`), not the generic
        // "not an aggregate" text -- a non-`inline` word's ordinary `[ ... ]`
        // parameter *is* a three-word aggregate at the ABI level, so that claim
        // is False at the representation the backend emits even though the
        // type system still refuses the borrow.
        let err = check_src(
            ": ap ( 'T [ 'T -- 'T ] -- 'T ) | x f | f &f drop x swap call ;\n: main ( -- ) 3 [ 1 add ] ap . ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot borrow the local `f` of type `[ 'T -- 'T ]` in `ap` (line 1, col 42)\n  a quotation is not borrowable in a generic body"
        );
    }

    /// P7 slice 3c (R1.4): the widened `is_ref()` routes a slice scrutinee
    /// into the *reference*-scrutinee arm, whose advice ("pass the owned
    /// `Enum` instead") names nothing real for a view over a buffer. It gets
    /// the plain mismatch instead -- the very message the concrete path
    /// already gives the same scrutinee, so the two paths agree.
    #[test]
    fn poly_eliminator_with_a_slice_scrutinee_reports_a_plain_mismatch() {
        let err = check_src(
            "type: Shape | Circle r i64 | Square s i64 ;\n\
             : g ( 'T Slice[i64] -- 'T )\n  \
               ~[ ( Circle ) Circle> drop ] ~[ ( Square ) Square> drop ] Shape? ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: type mismatch in `g` (line 3)\n  \
             `Shape?` expected `Shape`, found `Slice[i64]`\n  \
             note: declared ( -- )"
        );
    }

    #[test]
    fn poly_reference_word_rejects_indexing_a_generic_length_array() {
        // E3/D6: `array['T 'N]` has no known count, so its element cannot be
        // statically bounds-checked; only a concrete-length array's element
        // is accessible this slice.
        let err = check_src(": badidx ( array['T 'N] -- 'T )\n  | a |\n  &a 0\n  &>\n  @\n;\n")
            .unwrap_err();
        assert_eq!(
            err,
            "error: cannot index a generic-length array in `badidx` (line 4, col 3)\n  the array's length is the type variable `'N`, so its element cannot be statically bounds-checked; index a concrete-length array (`array['T 4]`), or use a fixed length in this word's signature"
        );
    }

    #[test]
    fn poly_reference_word_rejects_borrowing_a_moved_local() {
        // E5: borrowing is not a move, but the referent still has to be
        // there -- a local already consumed holds nothing.
        let err = check_src(": badmove ( array['T 4] -- 'T )\n  | a |\n  a drop\n  &a 0 &> @\n;\n")
            .unwrap_err();
        assert_eq!(
            err,
            "error: use after move in `badmove` (line 4)\n  local `a` is linear and was moved at line 3, col 3, so it is used exactly once"
        );
    }

    #[test]
    fn poly_reference_word_rejects_owning_cell_accessor_in_a_generic_body() {
        // R-B6/E4: `&^` never produces a variable-referent ref (no generic
        // structs/enums this slice), so it is out of scope regardless of
        // mutability -- a located error, not a silent unknown-word one.
        let err = check_src(": badcell ( 'T -- 'T )\n  &^\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: `&^` is not yet supported in a generic body, in `badcell` (line 2)\n  monomorphize this word (or write a concrete wrapper) to use `&^` today"
        );
    }

    #[test]
    fn poly_reference_word_rejects_a_fused_accessor_spelling_in_a_generic_body() {
        // R-B6/E4: a leftover `&Struct>field` (retired in P7 slice 1) still
        // lexes as one `&`-prefixed token, and the `>`-bearing guard keeps it a
        // located error rather than a bare unknown-word one. The surviving
        // spelling's case is `projection_on_generic_receiver_body_is_error`.
        let err = check_src(": badfield ( 'T -- 'T )\n  &Point>x\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: `&Point>x` is not yet supported in a generic body, in `badfield` (line 2)\n  monomorphize this word (or write a concrete wrapper) to use `&Point>x` today"
        );
    }

    #[test]
    fn poly_reference_word_rejects_add_in_place_in_a_generic_body() {
        // R-B4/R-B6: `+!` is permanently out of scope, unlike `!` (Phase 3).
        let err = check_src(": badaddstore ( 'T -- 'T )\n  +!\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: `+!` is not yet supported in a generic body, in `badaddstore` (line 2)\n  monomorphize this word (or write a concrete wrapper) to use `+!` today"
        );
    }

    #[test]
    fn array_index_out_of_bounds_error_names_the_new_spelling() {
        // R9/R8: the array index out-of-bounds diagnostic names the new
        // `array[T N]` spelling in its effect annotation, per CLAUDE.md's
        // "diagnostics are behaviour".
        let err =
            check_src(": oob ( array[i64 4] -- i64 )\n  | a |\n  &a 9 &> @\n;\n").unwrap_err();
        assert!(err.contains("array index out of range"), "{err}");
        assert!(
            err.contains("array[i64 4]"),
            "error should name the new array spelling: {err}"
        );
    }

    #[test]
    fn poly_reference_word_rejects_out_of_range_literal_index() {
        // R-B3: the literal `9` is statically bounds-checked against the
        // array's known length 4, mirroring the monomorphic `check_array_index`.
        let err = check_src(": oob['T: Copy] ( array['T 4] -- 'T )\n  | a |\n  &a 9 &> @\n;\n")
            .unwrap_err();
        assert!(err.contains("array index out of range"), "{err}");
        assert!(err.contains("index 9"), "{err}");
        assert!(err.contains("length 4"), "{err}");
    }

    // -- Phase 3 (R-B3..R-B5): the mutable path and exclusivity -----------

    #[test]
    fn setat_writes_an_element_through_a_poly_mutable_borrow() {
        // R-B8's write witness, at the checker: `&!a` borrows mutably, `&!>`
        // takes a mutable element reference, `!` stores the `Copy` value
        // through it, and the array is returned afterwards -- the borrow is
        // dead by then, so naming `a` again is not a second name for
        // borrowed storage.
        check_src(
            ": setat['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &!a 2 &!> v ! a ;\n\
             : main ( -- ) 0 4 fill 99 setat drop ;\n",
        )
        .expect("a mutable prefix borrow, element ref, and store should check");
    }

    #[test]
    fn poly_reference_word_rejects_two_live_mutable_borrows() {
        // E6/R-B5/OQ1: the hazard the poly body must catch itself, since a
        // plain generic word is checked once and never re-checked at its
        // instantiations. Rejected *at the second borrow site* (line 3, col
        // 7), naming the first (line 3, col 3).
        let err = check_src(
            ": twomut['T: Copy] ( array['T 4] -- array['T 4] )\n  | a |\n  &!a &!a drop drop a\n;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `&!a` conflicts with a live borrow of `a` in `twomut` (line 3, col 7)\n  the mutable borrow taken at line 3, col 3 is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }

    #[test]
    fn poly_reference_word_rejects_a_shared_borrow_beside_a_live_mutable_one() {
        // E6, the other symmetric direction: a new *shared* borrow conflicts
        // with a live mutable one (never with another shared one).
        let err = check_src(
            ": mixed['T: Copy] ( array['T 4] -- array['T 4] )\n  | a |\n  &!a &a drop drop a\n;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `&a` conflicts with a live borrow of `a` in `mixed` (line 3, col 7)\n  the mutable borrow taken at line 3, col 3 is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }

    #[test]
    fn poly_reference_word_accepts_two_live_shared_borrows() {
        // The positive control for the two rejections above: with no mutable
        // borrow in play there is nothing for exclusivity to protect, so two
        // live `&a` are fine. Without this, a rule that rejected *every*
        // second borrow would pass both negatives.
        check_src(
            ": twoshared['T: Copy] ( array['T 4] -- array['T 4] ) | a | &a &a drop drop a ;\n",
        )
        .expect("two shared borrows of one place do not conflict");
    }

    #[test]
    fn poly_borrow_liveness_releases_a_borrow_once_its_reference_is_consumed() {
        // R-B5: the liveness approximation is not "live until the word ends"
        // -- `!` consumes the element reference, leaving no reference value
        // anywhere, so the first borrow is provably dead and the second
        // write is accepted. A word that can write only one element would be
        // a much weaker capability than the slice claims.
        check_src(
            ": settwo['T: Copy] ( array['T 4] 'T -- array['T 4] )\n  | a v |\n  &!a 0 &!> v !\n  &!a 1 &!> v !\n  a\n;\n",
        )
        .expect("a borrow whose reference is consumed is dead");
    }

    #[test]
    fn poly_borrow_liveness_sees_a_reference_parked_in_a_local() {
        // R-B5: `prune_dead_borrows` scans the locals as well as the stack.
        // Binding the first `&!a` to `r` empties the stack while the
        // reference is still perfectly usable, so a stack-only scan would
        // call the borrow dead and admit a genuine second mutable borrow of
        // `a` -- two live `&!` to one place, the exact hazard R-B5 exists to
        // stop. Every other liveness case parks its reference on the stack.
        let err = check_src(
            ": hidden['T: Copy] ( array['T 4] 'T -- array['T 4] )\n  | a v |\n  &!a | r |\n  &!a 0 &!> v !\n  r 1 &!> v !\n  a\n;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `&!a` conflicts with a live borrow of `a` in `hidden` (line 4, col 3)\n  the mutable borrow taken at line 3, col 3 is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }

    #[test]
    fn poly_call_term_accepts_naming_a_local_beside_a_live_shared_borrow() {
        // The positive control for the two naming-side rejections above, and
        // the mirror of `poly_reference_word_accepts_two_live_shared_borrows`
        // at the other site: naming a `Copy` aggregate neither moves it nor
        // aliases anything a *shared* borrow could mutate, so only a live
        // *mutable* borrow conflicts here. Without this, a naming check that
        // ignored the direction bit would pass both negatives.
        check_src(
            ": sharedname['T: Copy] ( array['T 4] -- array['T 4] 'T )\n  | a |\n  &a 0 &> @ | e |\n  &a a swap drop\n  e\n;\n",
        )
        .expect("a shared borrow does not stop a non-consuming name of its place");
    }

    #[test]
    fn poly_borrow_liveness_is_coarse_across_places() {
        // R-B5's permitted conservatism, pinned as intentional rather than
        // left as an accidental divergence: `prune_dead_borrows` releases
        // *all* recorded borrows or none, so an unrelated live `&b` keeps
        // `a`'s already-consumed borrow recorded and the second `&!a` is
        // refused. The monomorphic checker accepts the same shape (its
        // `live_deriv` is per place), so this is an over-rejection, never a
        // missed hazard -- and it is legible as such from the note.
        let err = check_src(
            ": coarse['T: Copy] ( array['T 4] array['T 4] 'T -- array['T 4] array['T 4] )\n  | a b v |\n  &b\n  &!a 0 &!> v !\n  &!a 1 &!> v !\n  drop a b\n;\n",
        )
        .unwrap_err();
        assert!(
            err.starts_with(
                "error: `&!a` conflicts with a live borrow of `a` in `coarse` (line 5, col 3)"
            ),
            "{err}"
        );
        assert!(err.contains("conservatively treated as live"), "{err}");
    }

    #[test]
    fn poly_call_term_rejects_consuming_a_borrowed_local() {
        // R-B5, the naming side: reading a linear local moves it out, and a
        // reference derived from it would be left aimed at storage its owner
        // gave away. Checking only at the borrow catches `a ... &!a` and
        // misses this, the same hazard with the terms swapped.
        let err =
            check_src(": consume ( array['T 4] -- array['T 4] )\n  | a |\n  &a a swap drop\n;\n")
                .unwrap_err();
        assert_eq!(
            err,
            "error: cannot consume the borrowed local `a` of type `array['T 4]` in `consume` (line 3, col 6)\n  the shared borrow taken at line 3, col 3 is still live\n  a place stays borrowed until every reference derived from it is consumed\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }

    #[test]
    fn poly_call_term_rejects_naming_a_mutably_borrowed_local() {
        // R-B5, the naming side for a `Copy` aggregate: the read does not
        // consume it, but the name still denotes the storage the live `&!`
        // mutates.
        let err = check_src(
            ": alias['T: Copy] ( array['T 4] -- array['T 4] 'T )\n  | a |\n  &!a a swap 0 &!> @\n;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot name `a` in `alias` (line 3, col 7): a mutable borrow of it is still live (line 3, col 3)\n  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates\n  finish with the borrow first, or `dup` for an independent copy\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }

    #[test]
    fn poly_body_rejects_dup_of_a_mutable_borrow_and_accepts_dup_of_a_shared_one() {
        // E1/D3/R-A5, now reachable end to end (Phase 1 could only reach the
        // gate directly, since no body could produce a `&!`): duplicating an
        // exclusive borrow would let two names mutate through it. The shared
        // half is the positive control -- a `&x` *is* `Copy`, so a rule that
        // rejected every `dup` of a reference would pass the negative alone.
        let err = check_src(
            ": dupmut['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &!a dup 0 &!> v ! drop a ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot `dup` a mutable reference in `dupmut` (line 1)\n  `&!array['T 4]` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow"
        );
        check_src(
            ": dupshared['T: Copy] ( array['T 4] -- array['T 4] 'T ) | a | &a dup drop 0 &> @ | e | a e ;\n",
        )
        .expect("a shared reference is `Copy` and may be duplicated");
    }

    #[test]
    fn poly_body_store_rejects_a_shared_receiver() {
        // R-B4: `!` is `( &!T T -- )`. A shared receiver is a mutability
        // mismatch rendered off the receiver's own referent, the same shape
        // `&>` uses for the mirror-image mismatch.
        let err = check_src(
            ": rdstore['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &a 0 &> v ! a ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: type mismatch in `rdstore` (line 1)\n  `!` expected `&!'T`, found `&'T`\n  note: declared ( -- )"
        );
    }

    #[test]
    fn poly_body_store_rejects_a_value_of_another_type() {
        // R-B4: the stored value must unify with the referent -- an `i64`
        // literal is not a `'T`, at any instantiation but one.
        let err = check_src(
            ": wrongval['T: Copy] ( array['T 4] 'T -- array['T 4] ) | a v | &!a 0 &!> 5 ! v drop a ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: type mismatch in `wrongval` (line 1)\n  `!` expected `'T`, found `i64`\n  note: declared ( -- )"
        );
    }

    #[test]
    fn poly_body_store_rejects_a_non_copy_referent() {
        // R-B4: storing overwrites the old value, so a linear referent would
        // lose its drop obligation silently. Same X7 gate `@` already uses.
        let err =
            check_src(": linstore ( array['T 4] 'T -- array['T 4] ) | a v | &!a 0 &!> v ! a ;\n")
                .unwrap_err();
        assert_eq!(
            err,
            "error: cannot `!` the type variable `'T` in `linstore` (line 1)\n  `'T` has no `Copy` bound, and a linear value cannot be duplicated; declare `'T: Copy` if every instantiation is `Copy`"
        );
    }

    #[test]
    fn poly_body_at_rejects_a_non_copy_referent() {
        // Phase 2 review: `@`'s `poly_copy_gate` call was reachable but
        // untested -- deleting it broke no test. A bare `'T` (no `Copy`
        // bound) fetched through a reference must still be rejected, the
        // same X7 reason `dup`/`over` already cover for a bare variable.
        let err = check_src(
            ": g ( array['T 4] -- 'T ) | a | &a 0 &> @ ;\n: main ( -- ) 10 4 fill g drop ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot `@` the type variable `'T` in `g` (line 1)\n  `'T` has no `Copy` bound, and a linear value cannot be duplicated; declare `'T: Copy` if every instantiation is `Copy`"
        );
    }

    /// P7 slice 2 review: `poly_reference_word`'s local-only lookup left a
    /// generic word unable to borrow a module static at all, though R1 has no
    /// monomorphic-only carve-out. `bump` never names `COUNT` as a local, so
    /// this only type-checks if the static fallback fires.
    #[test]
    fn poly_body_can_borrow_a_module_static() {
        check_src(
            "static: COUNT i64 = 0 ;\n\
             : bump ['T: Copy] ( 'T -- 'T ) | v | &!COUNT @ 1 add &!COUNT swap ! v ;\n\
             : main ( -- ) 5 bump drop ;",
        )
        .unwrap();
    }

    /// The exclusivity scan applies to a poly-body static borrow exactly as
    /// it does to a local's: two simultaneously live `&!COUNT` conflict.
    #[test]
    fn poly_body_two_live_mutable_static_borrows_conflict() {
        let err = check_src(
            "static: COUNT i64 = 0 ;\n\
             : bump ['T: Copy] ( 'T -- 'T ) &!COUNT &!COUNT drop drop ;\n\
             : main ( -- ) 5 bump drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!COUNT` conflicts with a live borrow of `COUNT`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn poly_reference_word_rejects_shared_accessor_on_a_mutable_receiver() {
        // Phase 2 review: the `recv_mut != mutable` guard in `&>`'s arm was
        // reachable (a declared `&![...]` input) but untested -- deleting it
        // broke no test. `&>` on a mutable reference must still be rejected
        // rather than silently reading through it, and it names both sides
        // the way the monomorphic twin does (`&>` expected `&array[i64 4]`, found
        // `&!array[i64 4]`) rather than the operand-family text, which reads as
        // if `&>` never accepts a reference at all.
        let err = check_src(": rd['T: Copy] ( &!array['T 4] -- 'T )\n  0 &> @\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: type mismatch in `rd` (line 2)\n  `&>` expected `&array['T 4]`, found `&!array['T 4]`\n  note: declared ( -- )"
        );
    }

    #[test]
    fn check_poly_array_index_bounds_checks_a_literal_and_requires_conversion_otherwise() {
        // R-B3, direct unit coverage of the helper mutation testing would
        // otherwise miss: a literal within range passes, one out of range
        // rejects, and a computed (non-literal) `i64` needs the explicit
        // `>usize` conversion the monomorphic checker also requires.
        let sig = ref_sig();
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let span = Span::default();
        check_poly_array_index(
            &PolyType::Concrete(Type::I64),
            Some(2),
            4,
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect("an in-range literal should pass");
        check_poly_array_index(
            &PolyType::Concrete(Type::I64),
            Some(9),
            4,
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect_err("an out-of-range literal should reject");
        check_poly_array_index(
            &PolyType::Concrete(Type::I64),
            None,
            4,
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect_err("a computed i64 needs the explicit >usize conversion");
        check_poly_array_index(
            &PolyType::Concrete(Type::Usize),
            None,
            4,
            &ctx,
            span,
            "&>",
            &sig,
        )
        .expect("an already-usize index needs no literal at all");
    }

    /// P7 slice 1 (R1): a field projection inside a generic body. `&f` carries
    /// no `>`, so the pre-slice accessor guard (which tested `rest.contains('>')`)
    /// no longer sees it, and without the receiver check the site falls through
    /// to the local/static arm and reports "`x` is not a local" -- a wrong
    /// diagnostic for a construct that is rejected for a different reason.
    #[test]
    fn projection_on_generic_receiver_body_is_error() {
        let err = check_src(
            "type: Point x i64 y i64 ;\n\
             : peek ( 'T -- 'T ) 3 4 Point &x @ . drop ;\n\
             : main ( -- ) 7 peek . ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`&x` is not yet supported in a generic body"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("is not a local"),
            "the local/static arm must not claim this site: {err}"
        );
    }

    /// P7 slice 3d (C1/R1): `call` on a body-local literal splices its body
    /// in place against the live poly stack -- the poly analogue of
    /// `check_terms_relaxed`'s own literal `call` splice.
    #[test]
    fn poly_call_on_literal_splices_body_in_place_ok() {
        check_src(
            ": bump ['T: Copy] ( 'T -- 'T 'T ) | x | [ x x ] call ;\n\
             : main ( -- ) 5 bump drop drop ;\n",
        )
        .expect("a literal's body should splice in place against the live stack");
    }

    /// P7 slice 3d (C1/R1): the splice is flavour-neutral, matching the
    /// concrete path (`call` is not a materialization boundary, so `~` decides
    /// nothing there either). This is the shape `tests/phase7_slice3b.rs`'s
    /// deferred-combinator test used to carry before R1 narrowed that guard
    /// off `call`, so without this nothing pins the `~[ ]` half.
    #[test]
    fn poly_call_on_inline_literal_splices_body_in_place_ok() {
        check_src(
            ": bump ['T: Copy] ( 'T -- 'T 'T ) | x | ~[ x x ] call ;\n\
             : main ( -- ) 5 bump drop drop ;\n",
        )
        .expect("an inline literal's body should splice in place too");
    }

    /// P7 slice 3d (C1/R1): `call` on a **non-literal** quotation operand
    /// -- a declared parameter whose effect still carries a free `'T`, so it
    /// stays `PolyType::Quotation` rather than folding to `PolyType::Concrete`
    /// -- is a located outcome, never a panic and never `unknown word`.
    /// P7.S3l flips this from a rejection: R1's new arm now dispatches this
    /// shape (both sides of the declared effect carry `'T`) to
    /// `poly_call_abstract_quotation_param`, which consumes/produces it
    /// cleanly, so the body no longer rejects it at all.
    #[test]
    fn poly_call_on_non_literal_quotation_operand_is_accepted() {
        check_src(": caller ( 'T [ 'T -- 'T ] -- 'T ) call ;\n")
            .expect("a non-literal quotation operand at `call` is now accepted");
    }

    /// P7 slice 3d (C1/R1): `call` on an *empty* stack reports the ordinary
    /// arity underflow, not `unknown word`. R1's arm owns every `call` in a
    /// poly body now, so it owns this report too: before the arm existed the
    /// name fell through to the env lookup, which has no `call` candidate.
    #[test]
    fn poly_call_on_empty_stack_is_underflow_error() {
        let err = check_src(
            ": uf ['T: Copy] ( 'T -- 'T )\n\
               | x | call x\n\
             ;\n\
             : main ( -- ) 1 uf drop ;\n",
        )
        .expect_err("`call` with nothing on the stack should underflow");
        assert!(
            err.contains("`call` needs 1 values, but the stack holds 0"),
            "{err}"
        );
        assert!(!err.contains("unknown word"), "{err}");
    }

    /// P7 slice 3d (C1/R1, L1): `call` on a plain non-quotation operand (a
    /// concrete `i64`, not a quotation of any kind) falls into the same
    /// `poly_op_on_variable_error` else-arm as the non-literal-quotation
    /// case above -- `PolyType::Concrete` renders `` `i64` ``, never
    /// `unknown word`. Untested behavioural change flagged in the Phase 1
    /// review: before R1's arm existed, `call` fell through to the ordinary
    /// env lookup and any operand type produced `unknown word`.
    #[test]
    fn poly_call_on_non_quotation_operand_is_located_error() {
        let err = check_src(
            ": caller ( 'T i64 -- 'T i64 )\n\
               call\n\
             ;\n\
             : main ( -- ) 1 2 caller drop drop ;\n",
        )
        .expect_err("a non-quotation operand at `call` should be rejected");
        assert_eq!(
            err,
            "error: `call` is not permitted on `i64` in `caller` (line 2)"
        );
        assert!(!err.contains("unknown word"), "{err}");
    }

    /// P7 slice 3d (C1/R1): the splice's own teardown, the poly analogue of
    /// `Scope::leave` -- a linear local bound *inside* the spliced literal and
    /// never consumed there leaks past `call` unless this is rejected before
    /// the retain, exactly as an eliminator arm's own unconsumed binding does.
    /// Reuses `poly_arm_local_not_consumed_error` rather than a fresh
    /// message, so the report names `call` as the binding site.
    #[test]
    fn poly_call_on_literal_leaked_local_is_error() {
        let err = check_src(&format!(
            "{SPY}: bad ['T: Copy] ( 'T Spy -- 'T )\n\
               [ | s | ] call\n\
             ;\n\
             : main ( -- ) 7 Spy 1 swap bad drop ;\n"
        ))
        .expect_err("a local bound inside the splice and never consumed should be rejected");
        assert!(
            err.contains("the local `s` of type `Spy`, bound in an arm of `call` in `bad`")
                && err.contains("is never consumed"),
            "{err}"
        );
    }

    /// P7 slice 3d (R1): the splice's teardown also retains `scope.locals`/
    /// `scope.moves` down to the pre-splice snapshot, the poly analogue of
    /// `Scope::leave`'s own truncation -- without it, a local bound and
    /// consumed *inside* the splice would still be registered in the
    /// enclosing scope afterward, so a second reference to that name would
    /// resolve as a stale local instead of the ordinary-word lookup it should
    /// fall through to.
    #[test]
    fn poly_call_on_literal_retains_locals_past_splice() {
        let err = check_src(
            ": leaks ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | 3 [ | y | ] call x y\n\
             ;\n\
             : main ( -- ) 7 leaks drop drop ;\n",
        )
        .expect_err("`y` must not remain a resolvable local past the splice's own scope");
        assert!(err.contains("unknown word `y`"), "{err}");
    }

    /// P7 slice 3d (C2/R2): a body-local literal passed to a concrete `env`
    /// word whose declared parameter is a ground `Type::Quotation` grounds
    /// against that effect and the ordinary call proceeds.
    #[test]
    fn poly_quotlit_grounds_against_concrete_quotation_param_ok() {
        check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] 2 run1 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect("a literal grounding against a concrete quotation parameter should be accepted");
    }

    /// P7 slice 3d (C2/R2): a `~[ ]` parameter can never reach the grounding
    /// arm at all, so this does *not* pin the arm's own `Type::Quotation`
    /// (not `Type::InlineQuotation`) exclusion -- R6's declaration gate
    /// (`word_entry.rs`) rejects `run1`'s own declaration first, since a
    /// non-inline word may not declare a `~[ ]` parameter, before any call
    /// site is even checked. Confirmed even a *legal* `inline run1` never
    /// reaches `chosen`: an inline word is spliced, not dispatched through
    /// `env` by name, so the call instead falls through to `unknown word
    /// run1__m0`. Both are pinned here so this rejection is never
    /// mistaken for evidence of the grounding arm's own exclusion.
    #[test]
    fn poly_quotlit_against_declared_inline_quotation_param_rejects_at_declaration() {
        let err = check_src(
            ": run1 ( ~[ i64 -- i64 ] i64 -- i64 ) swap call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] 2 run1 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("a non-inline word may not declare a `~[ ]` parameter");
        assert!(
            err.contains("declares an inline-quotation parameter") && err.contains("not `inline`"),
            "{err}"
        );
    }

    #[test]
    fn poly_quotlit_against_legal_inline_quotation_param_rejects_at_the_cross_call() {
        // Pre-existing/unrelated to the slice that added this: any `~[ ]`-
        // bearing signature routes to the poly parser regardless of whether
        // it declares a type variable, so `run1` here lands in `poly_env`
        // despite carrying no `'T`.
        //
        // P7.S3k: `poly_call_term` *does* read `poly_env` now, so this shape
        // reaches the generic-callee arm rather than falling through it. It
        // is still a rejection, for a narrower and now accurate reason: a
        // quotation parameter has no runtime representation to pass across a
        // real call, so it is one of the declared shapes a symbolic mapping
        // cannot carry. Asserted on that reason and not just on the shared
        // first line, which both the old whole-feature narrowing and this
        // gate would satisfy.
        let err = check_src(
            ": run1 inline ( ~[ i64 -- i64 ] i64 -- i64 ) swap call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] 2 run1 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("a quotation cannot be passed across a polymorphic call");
        assert!(
            err.contains("cannot call the polymorphic word `run1`")
                && err.contains("passing a quotation to a polymorphic word"),
            "{err}"
        );
    }

    /// P7 slice 3d (R2): the load-bearing shape for the operand-window
    /// carve-out -- a non-builtin name's window is exactly one slot (its
    /// top), so the carve-out only ever matters when the literal itself is
    /// that top slot (the conventional quotation-last API shape). Every
    /// other test in this file parks the literal *underneath* the window
    /// (`x [ .. ] 2 run1`), where the window guard never even inspects it;
    /// forcing the carve-out to always reject fails none of those, only
    /// this one.
    #[test]
    fn poly_quotlit_grounds_when_it_is_the_top_of_window_operand_ok() {
        check_src(
            ": run0 ( i64 [ i64 -- i64 ] -- i64 ) call ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x 2 [ 1 add ] run0 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect("a top-of-window literal must ground against a concrete quotation parameter");
    }

    /// Review fix (Bug 1): binding a quotation literal to a local and
    /// reading it back loses its `PolyQuotRef` identity -- `| names |`
    /// only records a bound slot's `PolyType`, never its `quot` (`poly.rs`'s
    /// own local-binding loop), so a local read pushes a fresh
    /// `PolySlot::new(pt)` with `quot: None` (the local-read arm, same
    /// file). A re-read `QuotLit` slot reaching the grounding arm is
    /// therefore not a value this slice can ground; it must be the located
    /// rejection the operand-window guard already renders for an
    /// ungroundable `QuotLit`, never the panic the unguarded `.expect(...)`
    /// used to produce.
    #[test]
    fn poly_quotlit_bound_and_reread_is_located_error_not_panic() {
        let err = check_src(
            ": run0 ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x |\n\
               [ 1 add ] | q |\n\
               x 2 q run0\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err(
            "a re-read quotation-literal marker must not reach the `.expect` it used to panic on",
        );
        assert_eq!(
            err, "error: `run0` is not permitted on a quotation literal in `apply` (line 5)",
            "{err}"
        );
    }

    /// Review fix (Bug 2): `poly_ground_quotation_literal` ported no flavour
    /// check, so an inline `~[ ]` literal at a concrete word's ordinary
    /// `Type::Quotation` parameter silently grounded and compiled -- the
    /// mono twin (`check_literal_against_declared_effect`,
    /// `literal_is_inline != is_inline`) rejects the identical shape with
    /// `inline_literal_at_ordinary_param_error`.
    #[test]
    fn poly_quotlit_inline_literal_at_ordinary_param_is_error() {
        let err = check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x ~[ 1 add ] 2 run1 ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("an inline literal at an ordinary quotation parameter must be rejected");
        assert!(
            err.contains("this quotation is inline `~[ ... ]`")
                && err.contains("`run1` expects `[ i64 -- i64 ]`"),
            "{err}"
        );
    }

    /// Review fix (Bug 3): `poly_ground_quotation_literal` never reconciled
    /// an annotated literal against the declared parameter effect, so a
    /// literal annotated with a disagreeing effect silently grounded and
    /// compiled -- the mono twin runs `reconcile_annotation_with_parameter`
    /// immediately after the flavour check and rejects the identical shape.
    #[test]
    fn poly_quotlit_disagreeing_annotation_is_error() {
        let err = check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ ( Bool -- Bool ) dup drop ] 2 run1 ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err(
            "an annotation disagreeing with the declared parameter effect must be rejected",
        );
        assert!(
            err.contains("annotated") && err.contains("but `run1` declares it"),
            "{err}"
        );
    }

    /// P7 slice 3d (C2): `poly_ground_quotation_literal`'s own teardown --
    /// the poly analogue of `Scope::leave` -- rejects a non-`Copy` local the
    /// grounded literal binds and never consumes, exactly as R1's splice
    /// teardown does for `call`.
    #[test]
    fn poly_ground_quotation_literal_leaked_local_is_error() {
        let err = check_src(&format!(
            "{SPY}: run1 ( [ Spy -- i64 ] Spy -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ | s | 1 ] 1 Spy run1\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n"
        ))
        .expect_err("a local bound in the grounded literal and never consumed should be rejected");
        assert!(
            err.contains("the local `s` of type `Spy`, bound in an arm of `run1` in `apply`")
                && err.contains("is never consumed"),
            "{err}"
        );
    }

    /// P7 slice 3d (C2, R12): the poly twin of the concrete argument site's
    /// D3 capture rule. The callee materializes the literal and `call`s it
    /// twice, so a linear enclosing local consumed inside it is freed twice;
    /// before the port this compiled clean and died with `free(): double free
    /// detected in tcache 2` at run time, while the monomorphic twin of the
    /// same body rejected it.
    #[test]
    fn poly_ground_quotation_literal_consuming_enclosing_linear_local_is_error() {
        let err = check_src(
            ": twice ( [ i64 -- i64 ] i64 -- i64 ) swap | q | q call q call ;\n\
             : ap ['T: Copy] ( 'T ^i64 -- 'T i64 ) | x c | x [ c drop 1 add ] 2 twice ;\n\
             : main ( -- ) 5 7 ^ ap . . ;\n",
        )
        .expect_err("a grounded literal consuming an enclosing linear local must be rejected");
        assert!(
            err.contains("the quotation passed to `twice` consumes the enclosing local `c`")
                && err.contains("(D3)"),
            "{err}"
        );
    }

    /// P7 slice 3d (C2, R12): the companion permissive half -- R12 forbids
    /// *consuming* an enclosing local, not reading a `Copy` one, so the
    /// capture check must not reject the shape a grounded literal exists
    /// for. Widening the check from the consumed locals to every enclosing
    /// name fails here.
    #[test]
    fn poly_ground_quotation_literal_reading_enclosing_copy_local_ok() {
        check_src(
            ": twice ( [ i64 -- i64 ] i64 -- i64 ) swap | q | q call q call ;\n\
             : ap ['T: Copy] ( 'T i64 -- 'T i64 ) | x c | x [ c add ] 2 twice ;\n\
             : main ( -- ) 5 7 ap . . ;\n",
        )
        .expect("a grounded literal may read an enclosing `Copy` local by value");
    }

    /// P7 slice 3d (C2, R12): what the capture check's `MoveState::Live`
    /// precondition buys. The rule is about a *transition* across the
    /// literal, not about the post-state: `c` is already `Moved` when the
    /// literal is grounded and the literal never mentions it, so there is no
    /// capture. Matching the post-state alone rejects this legal program.
    #[test]
    fn poly_ground_quotation_literal_local_consumed_before_the_literal_ok() {
        check_src(
            ": twice ( [ i64 -- i64 ] i64 -- i64 ) swap | q | q call q call ;\n\
             : ap ['T: Copy] ( 'T ^i64 -- 'T i64 ) | x c | c drop x [ 1 add ] 2 twice ;\n\
             : main ( -- ) 5 7 ^ ap . . ;\n",
        )
        .expect("a local consumed before the literal is not captured by it");
    }

    /// P7 slice 3d (C2, R12): the other half of D3 -- a grounded literal
    /// that leaves a borrow of an enclosing place on its exit row -- is
    /// rejected, but only because a `PolyType::Ref` slot can satisfy no
    /// declared concrete output. The concrete twin of this body reports the
    /// D3 rule by name; this asserts only that the program is refused, so a
    /// future change that lets the two reference representations unify fails
    /// here instead of silently admitting the capture.
    #[test]
    fn poly_ground_quotation_literal_borrowing_enclosing_place_is_error() {
        let err = check_src(
            "type: Pair a i64 b i64 ;\n\
             : takes ( [ -- &Pair ] -- ) drop ;\n\
             : ap ['T: Copy] ( 'T Pair -- 'T ) | x p | x [ &p ] takes ;\n\
             : main ( -- ) 5 1 2 Pair ap drop ;\n",
        )
        .expect_err("a grounded literal may not leave a borrow of an enclosing place on its row");
        assert!(err.contains("`takes` expected `[ -- &Pair ]`"), "{err}");
    }

    /// P7 slice 3d (C2): the teardown also retains `scope.locals`/
    /// `scope.moves` back down to the pre-grounding snapshot -- without it,
    /// a `Copy` local bound inside the grounded literal would still be
    /// registered afterward, so a second reference to that name would
    /// resolve as a stale local rather than the ordinary lookup it should
    /// fall through to.
    #[test]
    fn poly_ground_quotation_literal_retains_locals_after_grounding() {
        let err = check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : leaks ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ | y | 3 ] 2 run1 y\n\
             ;\n\
             : main ( -- ) 7 leaks drop drop ;\n",
        )
        .expect_err("`y` must not remain a resolvable local past the grounded literal's own scope");
        assert!(err.contains("unknown word `y`"), "{err}");
    }

    /// P7 slice 3d (C2): the grounded literal's exit stack must match
    /// `eff.outputs` pointwise, not merely in arity -- a literal that leaves
    /// the declared output type but the wrong shape is still a type
    /// mismatch, not a silent pass.
    #[test]
    fn poly_ground_quotation_literal_output_mismatch_is_error() {
        let err = check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ True ] 2 run1\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("a literal whose grounded body leaves the wrong output shape must be rejected");
        assert!(
            err.contains("`run1` expected `[ i64 -- i64 ]`") && err.contains("found `i64 Bool`"),
            "{err}"
        );
    }

    /// P7 slice 3d (C2): a same-arity mismatch -- the grounded literal
    /// leaves exactly as many outputs as `eff.outputs` declares, but the
    /// wrong type at that position, so only the pointwise half of the
    /// check (not the arity half the mismatch test above exercises) can
    /// catch it. Without it this would compile a type-confused program
    /// silently.
    #[test]
    fn poly_ground_quotation_literal_output_type_mismatch_same_arity_is_error() {
        let err = check_src(
            ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
             : apply ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ drop True ] 2 run1\n\
             ;\n\
             : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err(
            "a same-arity output whose type differs from the declared effect must still be rejected",
        );
        assert!(
            err.contains("`run1` expected `[ i64 -- i64 ]`") && err.contains("found `Bool`"),
            "{err}"
        );
    }

    /// P7 slice 3d (R2): the retained combinator guard still rejects
    /// `branch`/`if`/`times`/`tag` on a quotation, unaffected by the
    /// operand-window carve-out this phase adds.
    #[test]
    fn poly_call_term_still_rejects_branch_on_quotation() {
        // `times` no longer belongs to this test: S3b-follow's real
        // `poly_row_combinator` dispatch handles it (and `if`) whenever the
        // combinator is actually registered, which this minimal `check_src`
        // harness (`parse_with_core`, no library source) never does for a
        // bare name -- so the old scenario now falls through to `unknown
        // word` rather than exercising this guard at all. `branch` is a
        // compiler-known primitive, never a `CombinatorEnv` entry, so it
        // still reaches this guard regardless of what the harness loads.
        let err = check_src(
            ": apply ['T: Copy] ( 'T -- 'T ) | x | True [ ] [ ] branch drop ;\n : main ( -- ) 5 apply drop ;\n",
        )
        .expect_err("`branch` on a quotation should stay rejected");
        assert!(
            err.contains("is not yet supported") && err.contains("name no follow-up slice yet"),
            "{err}"
        );
    }

    /// P7 slice 3d (R2): a literal passed to an *overloaded* concrete name
    /// is out of this slice's scope (the completeness gap/scoping note) --
    /// the pre-existing operand-window guard still catches it when the
    /// literal is the sole (top-of-window) operand, never `unknown word`.
    #[test]
    fn poly_quotlit_to_overloaded_concrete_name_is_located_rejection() {
        let err = check_src(
            ": run2 ( [ i64 -- i64 ] -- i64 ) 1 swap call ;\n : run2 ( i64 -- i64 ) 1 add ;\n : apply ['T: Copy] ( 'T -- 'T i64 ) | x | x [ 1 add ] run2 ;\n : main ( -- ) 5 apply drop drop ;\n",
        )
        .expect_err("an overloaded concrete name must not ground a quotation literal");
        assert_eq!(
            err,
            "error: `run2` is not permitted on a quotation literal in `apply` (line 3)"
        );
    }

    // P7.S4 (R2): `match_impl_target` unit tests.

    #[test]
    fn match_impl_target_var_elem_and_len_matches() {
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let pattern = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let subst = match_impl_target(&pattern, ty, &arrays, &[], &[], None).expect("should match");
        assert_eq!(subst.ty_of(0), Some(Type::I64));
        assert_eq!(subst.len_of(0), Some(4));
    }

    #[test]
    fn match_impl_target_concrete_elem_var_len_matches() {
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let pattern = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let subst = match_impl_target(&pattern, ty, &arrays, &[], &[], None).expect("should match");
        assert_eq!(subst.len_of(0), Some(4));
        assert!(subst.ty.is_empty());
    }

    #[test]
    fn match_impl_target_concrete_elem_wrong_type_no_match() {
        let arrays = vec![ArrayDecl {
            element: Type::U32,
            count: 4,
            name_static: "array",
        }];
        let pattern = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let ty = Type::Array(ArrayId::from_index(0), "array");
        assert!(match_impl_target(&pattern, ty, &arrays, &[], &[], None).is_none());
    }

    #[test]
    fn match_impl_target_var_elem_concrete_len_matches() {
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let pattern = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let subst = match_impl_target(&pattern, ty, &arrays, &[], &[], None).expect("should match");
        assert_eq!(subst.ty_of(0), Some(Type::I64));
        assert!(subst.len.is_empty());
    }

    #[test]
    fn match_impl_target_var_elem_concrete_len_wrong_no_match() {
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 2,
            name_static: "array",
        }];
        let pattern = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        let ty = Type::Array(ArrayId::from_index(0), "array");
        assert!(match_impl_target(&pattern, ty, &arrays, &[], &[], None).is_none());
    }

    #[test]
    fn match_impl_target_concrete_struct_matches_only_same() {
        let pattern = PolyType::Concrete(Type::Struct(StructId::from_index(0), "Point"));
        assert!(match_impl_target(
            &pattern,
            Type::Struct(StructId::from_index(0), "Point"),
            &[],
            &[],
            &[],
            None
        )
        .is_some());
        assert!(match_impl_target(&pattern, Type::I64, &[], &[], &[], None).is_none());
    }

    #[test]
    fn match_impl_target_shared_var_consistency() {
        // Review fix (P2): the previous version of this test matched a
        // *fresh* `Subst` twice, so the `prev == ty` consistency branch in
        // `match_impl_target_rec`'s `Var` arm was never exercised -- both
        // calls simply bound an empty variable. A `Quotation` pattern lets
        // the *same* var appear at two independent positions within one
        // match, so consistency actually gets checked: `'T` used at both
        // input slots matches `[i64 -- ]` but not the shape below.
        use crate::ast::quotation_type;
        let pattern = PolyType::Quotation(
            vec![PolyType::Var(0), PolyType::Var(0)],
            Vec::new(),
            false,
            None,
            None,
        );
        let same = quotation_type(vec![Type::I64, Type::I64], Vec::new());
        let subst = match_impl_target(&pattern, same, &[], &[], &[], None).expect("should match");
        assert_eq!(subst.ty_of(0), Some(Type::I64));
        // `'T` bound to `i64` at the first slot must hold at the second too.
        let mismatched = quotation_type(vec![Type::I64, Type::U32], Vec::new());
        assert!(match_impl_target(&pattern, mismatched, &[], &[], &[], None).is_none());
    }

    #[test]
    fn match_impl_target_shared_len_var_consistency() {
        // The `Len::Var` twin: `array[array[i64 'N] 'N]` (outer length shares the
        // inner array's length variable) matches `array[array[i64 4] 4]` but not
        // `array[array[i64 4] 2]`, exercising `Len::Var`'s own `prev == count` branch.
        let arrays = vec![
            ArrayDecl {
                element: Type::I64,
                count: 4,
                name_static: "inner",
            },
            ArrayDecl {
                element: Type::Array(ArrayId::from_index(0), "inner"),
                count: 4,
                name_static: "outer_matching",
            },
            ArrayDecl {
                element: Type::Array(ArrayId::from_index(0), "inner"),
                count: 2,
                name_static: "outer_mismatched",
            },
        ];
        let pattern = PolyType::Array(
            Box::new(PolyType::Array(
                Box::new(PolyType::Concrete(Type::I64)),
                Len::Var(0),
            )),
            Len::Var(0),
        );
        let matching_ty = Type::Array(ArrayId::from_index(1), "outer_matching");
        let subst =
            match_impl_target(&pattern, matching_ty, &arrays, &[], &[], None).expect("len 4 == 4");
        assert_eq!(subst.len_of(0), Some(4));
        let mismatched_ty = Type::Array(ArrayId::from_index(2), "outer_mismatched");
        assert!(match_impl_target(&pattern, mismatched_ty, &arrays, &[], &[], None).is_none());
    }

    // P7.S4 Phase 2 (R3): `specificity` / `is_strictly_more_specific` unit
    // tests.
    //
    // The position-vector tests exercise the pure partial-order algorithm
    // directly, covering the shared-variable scenarios that need
    // `GenericTypes` to reach through `specificity`'s `collect_positions`
    // walk. The `specificity` tests below exercise the full
    // pattern-against-`ty` collection path for `Array` patterns (which need
    // only an `ArrayDecl`).

    #[test]
    fn specificity_concrete_elem_more_specific_than_var_elem() {
        // `[i64 N]` ≺ `['T N]`
        let a = [Position::TyConcrete, Position::LenVar(0)];
        let b = [Position::TyVar(0), Position::LenVar(0)];
        assert!(is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_concrete_len_more_specific_than_var_len() {
        // `array['T 4]` ≺ `['T N]`
        let a = [Position::TyVar(0), Position::LenConcrete];
        let b = [Position::TyVar(0), Position::LenVar(0)];
        assert!(is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_concrete_elem_vs_concrete_len_incomparable() {
        // `[i64 N]` ⊥ `array['T 4]` (neither more specific)
        let a = [Position::TyConcrete, Position::LenVar(0)];
        let b = [Position::TyVar(0), Position::LenConcrete];
        assert!(!is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_equal_patterns_not_strictly_more_specific() {
        // `['T N]` vs `['T N]` — equal, neither strictly more specific
        let a = [Position::TyVar(0), Position::LenVar(0)];
        assert!(!is_strictly_more_specific(&a, &a));
    }

    #[test]
    fn specificity_box_concrete_more_specific_than_box_var() {
        // `Box[i64]` ≺ `Box['T]`
        let a = [Position::TyConcrete];
        let b = [Position::TyVar(0)];
        assert!(is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_shared_ty_var_more_specific_than_distinct() {
        // `Map['T 'T]` ≺ `Map['T 'U]`: A's partition {{0,1}} is coarser than
        // B's {{0},{1}}; B's classes refine A's.
        let a = [Position::TyVar(0), Position::TyVar(0)];
        let b = [Position::TyVar(0), Position::TyVar(1)];
        assert!(is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_concrete_one_slot_vs_shared_var_incomparable() {
        // `Map[i64 'T]` ⊥ `Map['T 'T]`: A more constrained at position 0
        // (Concrete vs Var), B more constrained via sharing (positions linked
        // vs independent) — incomparable.
        let a = [Position::TyConcrete, Position::TyVar(0)];
        let b = [Position::TyVar(0), Position::TyVar(0)];
        assert!(!is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_nested_shared_len_more_specific() {
        // `[[i64 N] N]` ≺ `[[i64 N] M]`: inner length = outer length, linked
        // vs separate — the linked version is coarser/more-constraining, so
        // more specific.
        let a = [
            Position::TyConcrete,
            Position::LenVar(0),
            Position::LenVar(0),
        ];
        let b = [
            Position::TyConcrete,
            Position::LenVar(0),
            Position::LenVar(1),
        ];
        assert!(is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_nested_shared_len_vs_concrete_inner_incomparable() {
        // `[['T N] N]` ⊥ `[array['T 4] N]`: length variable N shared in A vs
        // concrete in B; concrete inner length in B vs variable in A —
        // incomparable.
        let a = [Position::TyVar(0), Position::LenVar(0), Position::LenVar(0)];
        let b = [
            Position::TyVar(0),
            Position::LenConcrete,
            Position::LenVar(0),
        ];
        assert!(!is_strictly_more_specific(&a, &b));
        assert!(!is_strictly_more_specific(&b, &a));
    }

    #[test]
    fn specificity_array_concrete_elem_vs_var_elem() {
        // `[i64 N]` ≺ `['T N]` via the full `specificity` function.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let a = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let b = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        assert_eq!(
            specificity(&a, &b, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Less)
        );
        assert_eq!(
            specificity(&b, &a, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn specificity_array_concrete_len_vs_var_len() {
        // `array['T 4]` ≺ `['T N]` via the full `specificity` function.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let a = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        let b = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        assert_eq!(
            specificity(&a, &b, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn specificity_array_concrete_elem_vs_concrete_len_incomparable() {
        // `[i64 N]` ⊥ `array['T 4]` via the full `specificity` function.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let a = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let b = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        assert_eq!(
            specificity(&a, &b, &[], &[], ty, &arrays, &[], &[], None),
            None
        );
        assert_eq!(
            specificity(&b, &a, &[], &[], ty, &arrays, &[], &[], None),
            None
        );
    }

    #[test]
    fn specificity_concrete_target_vs_generic_target() {
        // `Concrete(array[i64 4])` ≺ `['T N]`: the concrete target unfolds to
        // the same positions as the array pattern, all concrete.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let a = PolyType::Concrete(ty);
        let b = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        assert_eq!(
            specificity(&a, &b, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Less)
        );
    }

    // Review fix (P7.S4 Phase 2): `specificity` used to flatten each pattern
    // independently and reject any pair of different lengths as
    // incomparable, so a bare `'T` (1 position) could never lose to a
    // deeper structural pattern (2+ positions) even when the deeper one is
    // strictly more specific. `collect_paired_positions` walks both
    // patterns together instead, so a depth mismatch is classified
    // explicitly rather than silently producing differently-sized vectors.

    #[test]
    fn specificity_bare_var_vs_array_pattern_depth_mismatch() {
        // `'T` (1 position) vs `array['T 'N]` (2 positions): `array['T 'N]` wins --
        // a bare variable is unconditionally less specific than any
        // structural pattern, regardless of the depth mismatch.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let a = PolyType::Var(0);
        let b = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        assert_eq!(
            specificity(&a, &b, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Greater)
        );
        assert_eq!(
            specificity(&b, &a, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn specificity_bare_var_vs_fully_concrete_array_depth_mismatch() {
        // `'T` (1 position) vs the folded `Concrete(array[i64 4])` (2 positions
        // once unfolded): the fully concrete impl wins.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let a = PolyType::Var(0);
        let b = PolyType::Concrete(ty);
        assert_eq!(
            specificity(&a, &b, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Greater)
        );
        assert_eq!(
            specificity(&b, &a, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn specificity_nested_array_more_specific_than_shallow_at_mismatched_depth() {
        // `array['T 'N]` vs `array[array['T 'N] 'M]` at `array[array[i64 4] 2]`: the nested pattern's
        // element position is itself an array pattern where the shallow
        // pattern only has a bare variable -- a depth mismatch one level
        // in, not just at the top. The nested pattern wins.
        let inner_arr = ArrayId::from_index(0);
        let outer_arr = ArrayId::from_index(1);
        let arrays = vec![
            ArrayDecl {
                element: Type::I64,
                count: 4,
                name_static: "inner",
            },
            ArrayDecl {
                element: Type::Array(inner_arr, "inner"),
                count: 2,
                name_static: "outer",
            },
        ];
        let ty = Type::Array(outer_arr, "outer");
        // `array['T 'N]`
        let shallow = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        // `array[array['T 'N] 'M]`
        let nested = PolyType::Array(
            Box::new(PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0))),
            Len::Var(1),
        );
        assert_eq!(
            specificity(&shallow, &nested, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Greater)
        );
        assert_eq!(
            specificity(&nested, &shallow, &[], &[], ty, &arrays, &[], &[], None),
            Some(Ordering::Less)
        );
    }

    // P7.S4b Phase 3 (R5/R12): bound-set tiebreak unit tests.

    #[test]
    fn specificity_bounded_beats_unbounded_at_equal_pattern() {
        // `impl: Eq for ['T N] where 'T: Eq` is more specific than
        // `impl: Eq for ['T N]` (no bounds) at equal pattern — the bounded
        // candidate's bound set is a strict superset of the unbounded one's.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let pat = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let bounded = vec![(0u32, Bound::User(TraitId::from_index(1)))];
        let unbounded: Vec<(u32, Bound)> = vec![];
        // Bounded (A) vs unbounded (B): A ⊃ B → A is more specific → Less.
        assert_eq!(
            specificity(
                &pat,
                &pat,
                &bounded,
                &unbounded,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            Some(Ordering::Less)
        );
        // Unbounded (A) vs bounded (B): B ⊃ A → B is more specific → Greater.
        assert_eq!(
            specificity(
                &pat,
                &pat,
                &unbounded,
                &bounded,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn specificity_incomparable_bound_sets_stay_incomparable() {
        // Two bound sets where neither is a superset of the other: 'T: Eq
        // vs 'T: Show — incomparable, so `specificity` returns `None`.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let pat = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let a_bounds = vec![(0u32, Bound::User(TraitId::from_index(1)))];
        let b_bounds = vec![(0u32, Bound::User(TraitId::from_index(2)))];
        assert_eq!(
            specificity(
                &pat,
                &pat,
                &a_bounds,
                &b_bounds,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            None
        );
        assert_eq!(
            specificity(
                &pat,
                &pat,
                &b_bounds,
                &a_bounds,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            None
        );
    }

    #[test]
    fn specificity_equal_bound_sets_not_strictly_more_specific() {
        // Same pattern, same bounds — neither strictly more constrained.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let pat = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let bounds = vec![(0u32, Bound::User(TraitId::from_index(1)))];
        assert_eq!(
            specificity(&pat, &pat, &bounds, &bounds, ty, &arrays, &[], &[], None),
            None
        );
    }

    #[test]
    fn specificity_pattern_domination_ignores_bounds() {
        // When one pattern is strictly more specific, bounds don't matter:
        // `[i64 N]` (concrete elem) beats `['T N]` (var elem) even if the
        // less-specific pattern has a bound and the more-specific one
        // doesn't.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let concrete = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let var = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let no_bounds: Vec<(u32, Bound)> = vec![];
        let with_bound = vec![(0u32, Bound::User(TraitId::from_index(1)))];
        // Concrete pattern (A, no bounds) is still more specific than var
        // pattern (B, with bound) — pattern dominates.
        assert_eq!(
            specificity(
                &concrete,
                &var,
                &no_bounds,
                &with_bound,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn specificity_bound_tiebreak_with_two_bounds_vs_one() {
        // `where 'T: Eq 'T: Show` (two bounds) is strictly more constrained
        // than `where 'T: Eq` (one bound) at equal pattern.
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 4,
            name_static: "array",
        }];
        let ty = Type::Array(ArrayId::from_index(0), "array");
        let pat = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let two_bounds = vec![
            (0u32, Bound::User(TraitId::from_index(1))),
            (0u32, Bound::User(TraitId::from_index(2))),
        ];
        let one_bound = vec![(0u32, Bound::User(TraitId::from_index(1)))];
        assert_eq!(
            specificity(
                &pat,
                &pat,
                &two_bounds,
                &one_bound,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            specificity(
                &pat,
                &pat,
                &one_bound,
                &two_bounds,
                ty,
                &arrays,
                &[],
                &[],
                None
            ),
            Some(Ordering::Greater)
        );
    }
}
