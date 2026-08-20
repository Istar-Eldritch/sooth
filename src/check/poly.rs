use std::cell::RefCell;

use crate::ast::GenericTypes;

use super::*;

/// R6: whether a concrete type satisfies an `Ord` bound. The numeric tower
/// (every integer width, `usize`/`isize`, and both floats) is totally ordered
/// for the comparison operators; nothing else is (`bool`, a struct, an array).
/// `max`'s float carve-out (X9) lives at its own builtin arm, not here.
pub(super) fn is_ord(ty: Type) -> bool {
    ty.is_numeric()
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
        // `&['T 4]` is `Copy` even where `['T 4]` is linear.
        PolyType::Ref(_, mutable) => !*mutable,
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

/// Whether a `PolyType` slot holds a reference: a poly one (`&['T 4]`, from a
/// body borrow) or a fully concrete one (`&[i64 4]`, from a declared input).
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
        PolyType::Var(_) | PolyType::Array(..) | PolyType::Quotation(..) => false,
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
        let ty = apply_subst(sig, pty, &subst, &word.name, span, &ctx, arrays, refs)?;
        inputs.push(TypedSlot { name: None, ty });
    }
    let mut outputs = Vec::with_capacity(sig.outputs.len());
    for pty in &sig.outputs {
        let ty = apply_subst(sig, pty, &subst, &word.name, span, &ctx, arrays, refs)?;
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
    check_word(
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
    )
}

/// R9 (Slice 6c): the REPL's entry to the standalone poly-combinator check.
/// Builds a scratch `PolyCtx` (the instantiation records a spliced combinator
/// produces are never lowered, R20) around the session's poly-env and combinator
/// view, so `eval_combinator_def` need not name the private `PolyCtx`. Mirrors
/// native's `is_combinator` branch in `check`, deliberately bypassing
/// `eval_poly_def`'s `>= 2`-output deferral: a combinator is spliced inline and
/// never lowered to a bundle-returning `IrFunc`, so that gate cannot fire.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_poly_combinator_repl(
    word: &WordDef,
    sig: &PolySig,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    structs: &[StructDecl],
    poly_env: &PolyEnv,
    combinators: &CombinatorEnv,
) -> Result<(), String> {
    let mut scratch: HashMap<Span, CallInst> = HashMap::new();
    let mut scratch_overloads: HashMap<Span, String> = HashMap::new();
    let mut scratch_fields: HashMap<Span, (StructId, usize)> = HashMap::new();
    let mut scratch_variant_fields: HashMap<Span, (EnumId, usize, usize)> = HashMap::new();
    let eliminators = eliminator_registry(enums);
    let mut poly = PolyCtx {
        env: poly_env,
        insts: &mut scratch,
        builtin_overloads: &mut scratch_overloads,
        resolved_fields: &mut scratch_fields,
        resolved_variant_fields: &mut scratch_variant_fields,
        combinators,
        eliminators: &eliminators,
    };
    // R8 (slice 8b): the REPL path has no `ModuleInfo` view, so the `drop`
    // import-visibility gate never fires on a session-checked combinator body.
    // A session retains no `static:` declarations either (P7 slice 2 is a
    // build-path feature), so the static table is empty here.
    check_poly_combinator_standalone(
        word,
        sig,
        enums,
        env,
        arrays,
        cells,
        refs,
        slices,
        structs,
        &[],
        None,
        &mut poly,
    )
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
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    builtin_overloads: &mut HashMap<Span, String>,
    generics: Option<&RefCell<GenericTypes>>,
) -> Result<(), String> {
    // R12 (slice 8b, 8a): the caller module's operator visibility rides on
    // `ctx`, so a bare operator in a poly body resolves against the same
    // scoped candidate set a concrete body does. `Some` from `check::check`,
    // `None` from `repl.rs` (the REPL path is unscoped, R8).
    //
    // Slice 10c (review fix, Phase 1): `poly_walk` never reaches the
    // concrete back-edge guard (R15) `ctx.is_self_tail_call()` gates, so an
    // empty index is correct here, not just convenient -- lowering never
    // back-edges a polymorphic instantiation either (`lower_instantiation`
    // hardcodes `self_tail = false`).
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
        &CombinatorIndex::new(),
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
        structs,
        enums,
        arrays,
        builtin_overloads,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn poly_walk(
    terms: &[Term],
    mut stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    builtin_overloads: &mut HashMap<Span, String>,
) -> Result<Vec<PolySlot>, String> {
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
            structs,
            enums,
            arrays,
            builtin_overloads,
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
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    builtin_overloads: &mut HashMap<Span, String>,
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
        TermKind::Call(name) => {
            return poly_call_term(
                name,
                span,
                stack,
                scope,
                sig,
                ctx,
                env,
                structs,
                enums,
                arrays,
                builtin_overloads,
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
        // Slice 6h: no interning route exists for a body-internal array
        // shape absent from a poly signature (`subst_polytype`/`array_id_of`
        // both look up an already-interned shape and panic otherwise), so
        // this is rejected eagerly. The quotation rejection this once
        // mirrored is gone (P7 slice 3b, R5): a quotation *does* have a hang
        // point now, `PolySlot::quot`; an array constructor still has no
        // interning route, which is a separate gap of its own.
        TermKind::ArrayCtor(_) => {
            return Err(format!(
                "error: an array constructor in the polymorphic body of `{}` (line {}) is not yet supported",
                ctx.word_name().unwrap_or("<line>"),
                span.line
            ));
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
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    builtin_overloads: &mut HashMap<Span, String>,
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
    // R-B1 (slice 13): every `&`-led word (the prefix borrow and the
    // reference accessor family) fronts the rest of dispatch, mirroring
    // `check_reference_word`'s own position ahead of the monomorphic
    // family. `Ok(None)` (not `&`-led) falls through unchanged.
    if let Some(next) = poly_reference_word(
        name, span, &mut stack, scope, sig, ctx, structs, enums, arrays,
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
        _ => {}
    }
    // Comparisons: on a bare variable they need `Ord` (X8); on two concrete
    // operands they delegate to the ordinary operator check below.
    if matches!(name, "eq" | "lt" | "gt" | "lte" | "gte" | "ne") {
        let n = stack.len();
        if n >= 2 {
            let a = stack[n - 2].pt.clone();
            let b = stack[n - 1].pt.clone();
            let (av, bv) = (poly_var_id(&a), poly_var_id(&b));
            if av.is_some() || bv.is_some() {
                match (av, bv) {
                    (Some(v), Some(w)) if v == w => {
                        if !sig.has_bound(v, Bound::Ord) {
                            return Err(poly_ord_body_error(
                                ctx,
                                span,
                                name,
                                &sig.ty_var_names[v as usize],
                            ));
                        }
                    }
                    _ => return Err(poly_op_operand_mismatch_error(ctx, span, name, &a, &b, sig)),
                }
                stack.truncate(n - 2);
                stack.push(PolySlot::new(PolyType::Concrete(Type::BOOL)));
                return Ok(stack);
            }
        }
    }
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
    // P7 slice 3b (R4/OQ6): the quotation-*consuming* combinator family is
    // deferred to P7.S3b-follow -- each takes a quotation as a row-typed
    // parameter, which needs row unification against an abstract stack, the
    // machinery an eliminator dispatch deliberately avoids. Located and
    // named here rather than left to fall through to `unknown word`, which
    // is what these emit otherwise (`poly_call_term` cannot see `poly_env`,
    // so none of them is even registered on this path).
    if matches!(name, "call" | "branch" | "if" | "times" | "tag")
        && stack.iter().any(|slot| slot.quot.is_some())
    {
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
            structs,
            enums,
            arrays,
            builtin_overloads,
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
    if stack[stack.len() - operand_window..]
        .iter()
        .any(|slot| matches!(slot.pt, PolyType::QuotLit))
    {
        return Err(poly_op_on_variable_error(
            ctx,
            span,
            name,
            &PolyType::QuotLit,
            sig,
        ));
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
        name, span, &mut stack, sig, ctx, env, structs, enums, arrays,
    )? {
        return Ok(next);
    }
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
    Err(unknown_word_error(ctx, span, name))
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
/// L1: type variables stay rigid. Two arms agree on an exit position iff the
/// `PolyType`s are structurally equal; `'T` against `i64` is a rejection, not
/// a mid-body bind, so no `Subst` is built or applied in the term walk and no
/// per-arm clone can diverge on one.
#[allow(clippy::too_many_arguments)]
fn poly_eliminator_call(
    id: EnumId,
    name: &str,
    span: Span,
    mut stack: Vec<PolySlot>,
    scope: &mut PolyScope,
    sig: &PolySig,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    builtin_overloads: &mut HashMap<Span, String>,
) -> Result<Vec<PolySlot>, String> {
    let enum_decl = &enums[id.index()];
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
        // through a `| q |` bind keeps the one and loses the other (L3:
        // `PolyScope.locals` carries no `QuotRef`), and it is still an
        // untagged arm -- not the abstract-scrutinee case below, which would
        // send it off to ask for an enum-kind bound on a type variable it
        // does not have.
        return Err(eliminator_untagged_arm_error(ctx, span, name));
    }
    match &scrutinee.pt {
        PolyType::Concrete(Type::Enum(found, _)) if *found == id => {}
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
                Type::Enum(id, enum_decl.name_static),
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
    }

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
    // yielding an abstract exit row. Each arm walks its own clone of the
    // enclosing scope, exactly as the concrete path clones `scope` per arm;
    // the join below reconciles the clones.
    let base = stack.len() - 1;
    let row: Vec<PolySlot> = stack[..base].to_vec();
    let enclosing_locals: HashSet<String> = scope.locals.keys().cloned().collect();
    let mut baseline: Option<Vec<PolySlot>> = None;
    let mut arm_moves: Vec<Moves> = Vec::with_capacity(arms.len());
    let mut arm_borrows: Vec<Vec<PolyBorrow>> = Vec::with_capacity(arms.len());
    for ((quot, _), vi) in arms.iter().zip(&variant_indices) {
        let lit = scope.quotation(*quot);
        let (literal_span, body, is_inline) = (lit.span, lit.body.clone(), lit.is_inline);
        let narrowed = variant_type(enums, id, *vi);
        // An eliminator's arm parameters are declared inline
        // (`enum_eliminator_sigs`), so an ordinary `[ ... ]` arm is the wrong
        // bracket here exactly as it is on the concrete path -- same
        // diagnostic, so the two paths do not disagree about one spelling.
        if !is_inline {
            return Err(ordinary_literal_at_inline_param_error(
                ctx,
                literal_span,
                name,
                crate::ast::inline_quotation_type(vec![narrowed], vec![]),
            ));
        }
        let mut arm_stack = row.clone();
        arm_stack.push(PolySlot::new(PolyType::Concrete(narrowed)));
        let mut arm_scope = scope.clone();
        let exit = poly_walk(
            &body,
            arm_stack,
            &mut arm_scope,
            sig,
            ctx,
            env,
            structs,
            enums,
            arrays,
            builtin_overloads,
        )?;
        // Step 5b: a `Type::Variant` may not leave the call. Every
        // type-directed predicate outside the eliminator is written over
        // `Type::Enum`, so `is_copy` reads an escaped variant as trivially
        // `Copy` and a later `dup` double-drops a linear payload.
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
            // L2: nor may a quotation literal, which would then have to be
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
        // R3, the poly analogue of `Scope::leave`. The poly walk has no block
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
        match &baseline {
            None => baseline = Some(exit),
            Some(expected) => {
                if expected.len() != exit.len() {
                    return Err(combinator_branch_output_mismatch_rendered(
                        ctx,
                        literal_span,
                        name,
                        &poly_row_str(expected, sig),
                        &poly_row_str(&exit, sig),
                    ));
                }
                // L1: structural equality under *rigid* variables. `'T` in one
                // arm against `'U`, or against `i64`, disagrees -- binding
                // either would be a mid-body unification this slice does not
                // do (and could not undo across the sibling arms).
                for (a, b) in expected.iter().zip(&exit) {
                    if a.pt != b.pt {
                        return Err(poly_arm_output_disagreement_error(
                            ctx,
                            literal_span,
                            name,
                            &poly_type_str(&a.pt, sig),
                            &poly_type_str(&b.pt, sig),
                        ));
                    }
                }
            }
        }
    }

    // L4: the borrow table is **unioned**, not picked or intersected. It is
    // keyed by place and a *missing* record reads as "no conflict", so
    // dropping one arm's record is a silent false accept: arm A's `&!x` and
    // arm B's `&!y` must both survive, or a later use of whichever was
    // dropped is wrongly admitted. A genuine disagreement (one place, two
    // mutabilities) is rejected rather than erased.
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
    // indexing sound here. A zero-variant enum has no arms and no
    // constructible value, so its call is unreachable and `scope`/`row` are
    // simply left untouched.
    if let Some(joined) = arm_moves.into_iter().reduce(Moves::join) {
        scope.moves = joined;
    }
    Ok(baseline.unwrap_or(row))
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
    arrays: &[ArrayDecl],
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
        let regs = crate::ast::NameRegistries {
            structs,
            enums,
            arrays,
            cells: &[],
            refs: &[],
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` in `{where_}` (line {}) leaves the type variable `{var}` undetermined\n  neither the operands nor the declared output fix `{var}`; a generic constructor needs every argument determined",
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
            let Some((recv_mut, elem, len)) = poly_ref_array_parts(&receiver, arrays) else {
                return Err(poly_op_on_variable_error(ctx, span, name, &receiver, sig));
            };
            if recv_mut != mutable {
                // A mutability mismatch is a *type* mismatch, not "this op
                // rejects references" -- the monomorphic twin says
                // `` `&>` expected `&[i64 4]`, found `&![i64 4]` ``, and the
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
            let referent_pt = if let Some(local_pt) = scope.locals.get(rest).cloned() {
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
                local_pt
            } else if let Some(static_ty) = ctx.static_type(rest) {
                // R1: a *scalar* static is borrowable though a scalar local
                // is not -- a static has a data-symbol address to hand out.
                // Never moved or dropped, so the move gate above has nothing
                // to say about it.
                PolyType::Concrete(static_ty)
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

/// The variable id of a bare `PolyType::Var`, else `None` (a concrete or
/// array slot is not a bare variable).
pub(super) fn poly_var_id(pt: &PolyType) -> Option<u32> {
    match pt {
        PolyType::Var(v) => Some(*v),
        _ => None,
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
        // (a length-variable array is never interned, so the declaration-time
        // `check_no_linear_array_elements` never sees it). Recurse so the
        // error names the real offending element, an unbounded variable or a
        // linear concrete type, never a fabricated one.
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
    // calling module exactly like the concrete path; `None` (the REPL, which
    // runs no mangling pass) falls back to the flat `env.get(name)`.
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
pub(super) fn resolve_poly_overload(
    candidates: &[(PolySig, Option<u64>)],
    stack: &[Slot],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    refs: &[RefDecl],
) -> Result<(PolySig, Option<u64>), PolyOverloadMiss> {
    let mut saw_quotation = false;
    for (sig, generation) in candidates {
        let n_in = sig.inputs.len();
        if stack.len() < n_in {
            continue;
        }
        let base = stack.len() - n_in;
        if stack[base..].iter().any(|s| s.quot.is_some()) {
            saw_quotation = true;
            continue;
        }
        if poly_sig_unifies(sig, stack, name, span, ctx, arrays, refs) {
            return Ok((sig.clone(), *generation));
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
    candidates: &[(PolySig, Option<u64>)],
) -> String {
    let demangled = crate::resolve::demangle_call(name);
    let mut shapes: Vec<String> = candidates
        .iter()
        .map(|(sig, _)| poly_sig_str(name, sig))
        .collect();
    shapes.sort();
    let listed = shapes
        .iter()
        .map(|s| format!("\n  candidate: {s}"))
        .collect::<String>();
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: no overload of `{demangled}` in `{wname}` (line {}) accepts these operands{listed}",
            span.line
        ),
        Ctx::Line { .. } => {
            format!("error: no overload of `{demangled}` accepts these operands{listed}")
        }
    }
}

/// Whether `sig`'s declared inputs unify against the tail of `stack`,
/// without committing any successful bindings past this call: a fresh
/// `Subst` per attempt, discarded either way. The shared predicate behind
/// resolving an overloaded polymorphic word (`resolve_poly_overload`) and an
/// overloaded polymorphic combinator (`resolve_combinator_overload`) alike --
/// unlike `resolve_poly_overload`'s own caller, a combinator's declared
/// inputs legitimately include a quotation type, so this makes no R9p
/// judgement about one; that stays the caller's decision.
pub(super) fn poly_sig_unifies(
    sig: &PolySig,
    stack: &[Slot],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
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
            refs,
            &mut subst,
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
pub(super) fn poly_sig_could_match(
    sig: &PolySig,
    stack: &[Slot],
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    refs: &[RefDecl],
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
        // Slice 10c: an `Ord`-bounded variable admits only the numeric tower
        // (`is_ord` is `is_numeric` and nothing else), and the bound is what
        // keeps `lib/core.sth`'s `: lt ( 'T: Copy Ord 'T -- bool )` from
        // claiming a call site meant for a user's `: lt ( Vec2 Vec2 -- bool )`.
        // Unification alone binds `'T` to anything at all, so without this the
        // library word swallows every operand type.
        if let PolyType::Var(v) = &sig.inputs[i] {
            if sig.has_bound(*v, Bound::Ord) && !stack[base + i].ty.is_numeric() {
                return false;
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
            refs,
            &mut subst,
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
    refs: &[RefDecl],
) -> Option<Combinator<'a>> {
    for comb in candidates {
        let matched = match comb.word.poly.as_ref() {
            Some(sig) => {
                poly_sig_could_match(sig, stack, comb.word.name.as_str(), span, ctx, arrays, refs)
            }
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
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: no overload of `{demangled}` in `{wname}` (line {}) accepts these operands{listed}",
            span.line
        ),
        Ctx::Line { .. } => {
            format!("error: no overload of `{demangled}` accepts these operands{listed}")
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_poly_call(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    scope: &Scope,
    arrays: &mut Vec<ArrayDecl>,
    refs: &mut Vec<RefDecl>,
    prov: &Provenance,
    live: &Liveness,
    at: usize,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    let candidates = poly.env.get(name).expect("caller checked membership");
    let (sig, generation) = match candidates.as_slice() {
        [(sig, generation)] => (sig.clone(), *generation),
        _ => match resolve_poly_overload(candidates, stack, name, span, ctx, arrays, refs) {
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
    for i in 0..n_in {
        // R9p: `unify_poly_input` binds a `Var` to *any* concrete type, so a
        // quotation would silently bind `'T` to the placeholder and
        // monomorphize a call over a phantom. Reject before unification.
        if stack[base + i].quot.is_some() {
            return Err(reject_quotation_argument(ctx, span, name));
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
            refs,
            &mut subst,
        )?;
    }
    // R6: each declared bound must hold of the concrete type `θ` bound the
    // variable to.
    for (v, bound) in &sig.bounds {
        let Some(ty) = subst.ty_of(*v) else { continue };
        let ok = match bound {
            Bound::Copy => is_copy(ty, ctx.structs(), ctx.enums(), arrays),
            Bound::Ord => is_ord(ty),
        };
        if !ok {
            let var = &sig.ty_var_names[*v as usize];
            return Err(match bound {
                Bound::Copy => poly_copy_bound_error(ctx, span, name, var, ty),
                Bound::Ord => poly_ord_bound_error(ctx, span, name, var, ty),
            });
        }
    }
    let mut outputs: Vec<Type> = Vec::with_capacity(sig.outputs.len());
    for pty in &sig.outputs {
        outputs.push(apply_subst(
            &sig, pty, &subst, name, span, ctx, arrays, refs,
        )?);
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
    let symbol = instantiation_symbol(name, &subst, generation);
    poly.insts.insert(
        span,
        CallInst {
            callee: name.to_string(),
            subst,
            symbol,
            out_arity: outputs.len(),
            output_types: outputs.clone(),
            bundle: None,
            generation,
        },
    );
    stack.truncate(base);
    for ty in outputs {
        stack.push(Slot::computed(ty));
    }
    Ok(std::mem::take(stack))
}

/// R5: unify one declared input `PolyType` against a concrete slot type,
/// extending `subst`. A repeated variable forced to two concretes is X4; a
/// non-array where an array is declared, or a mismatched concrete, is the
/// ordinary type-mismatch error.
#[allow(clippy::too_many_arguments)]
pub(super) fn unify_poly_input(
    sig: &PolySig,
    pty: &PolyType,
    slot_ty: Type,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    refs: &[RefDecl],
    subst: &mut Subst,
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
                    return Err(poly_var_conflict_error(
                        ctx,
                        span,
                        name,
                        &sig.ty_var_names[*v as usize],
                        prev,
                        slot_ty,
                    ));
                }
            } else {
                subst.ty.push((*v, slot_ty));
            }
        }
        PolyType::Array(elem, len) => {
            let Type::Array(id, _) = slot_ty else {
                return Err(poly_array_expected_error(ctx, span, name, slot_ty));
            };
            let (elem_ty, count) = (arrays[id.index()].element, arrays[id.index()].count);
            unify_poly_input(sig, elem, elem_ty, name, span, ctx, arrays, refs, subst)?;
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
                unify_poly_input(sig, p, *c, name, span, ctx, arrays, refs, subst)?;
            }
            for (p, c) in outs.iter().zip(&eff.outputs) {
                unify_poly_input(sig, p, *c, name, span, ctx, arrays, refs, subst)?;
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
                refs,
                subst,
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
                unify_poly_input(sig, arg_pty, *arg_ty, name, span, ctx, arrays, refs, subst)?;
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` in `{where_}` (line {}) names the generic type `{ty}`, which cannot yet be instantiated at a variable-bearing application\n  grounding a generic over its own type variable is not yet implemented",
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
    refs: &mut Vec<RefDecl>,
) -> Result<Type, String> {
    match pty {
        PolyType::Concrete(t) => Ok(*t),
        // P7 slice 3b: `pty` is a declared signature slot, which a body-only
        // marker never reaches.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        PolyType::Var(v) => subst.ty_of(*v).ok_or_else(|| {
            poly_unbound_output_error(ctx, span, name, &sig.ty_var_names[*v as usize])
        }),
        PolyType::Array(elem, len) => {
            let elem_ty = apply_subst(sig, elem, subst, name, span, ctx, arrays, refs)?;
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
                cins.push(apply_subst(sig, p, subst, name, span, ctx, arrays, refs)?);
            }
            let mut couts = Vec::with_capacity(outs.len());
            for p in outs {
                couts.push(apply_subst(sig, p, subst, name, span, ctx, arrays, refs)?);
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
            let referent = apply_subst(sig, referent, subst, name, span, ctx, arrays, refs)?;
            Ok(crate::ast::intern_ref_type(refs, referent, *mutable))
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
                concrete_args.push(apply_subst(sig, a, subst, name, span, ctx, arrays, refs)?);
            }
            let regs = crate::ast::NameRegistries {
                structs: ctx.structs(),
                enums: ctx.enums(),
                arrays,
                cells: &[],
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: use after move in `{where_}` (line {})\n  local `{local}` is linear and was moved at line {}, col {}, so it is used exactly once",
        span.line, site.line, site.col,
    )
}

pub(super) fn poly_copy_body_error(ctx: &Ctx, span: Span, op: &str, var: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot `{op}` the type variable `{var}` in `{where_}` (line {})\n  `{var}` has no `Copy` bound, and a linear value cannot be duplicated; declare `{var}: Copy` if every instantiation is `Copy`",
        span.line
    )
}

/// Slice 13 (E1): `dup`/`over` on a mutable reference in a generic body. The
/// same class of fact as `poly_copy_body_error`'s missing `Copy` bound, but
/// the reason is exclusivity rather than an absent bound, so the note names
/// that instead.
pub(super) fn poly_copy_mutable_ref_error(ctx: &Ctx, span: Span, op: &str, ty: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot `{op}` a mutable reference in `{where_}` (line {})\n  `{ty}` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow",
        span.line
    )
}

/// P7 slice 3a (R5.4): `dup`/`over` on a generic type applied to a variable
/// (D5's conservative linearity). The same class of fact as
/// `poly_copy_mutable_ref_error`, naming the type rather than a variable
/// name since a generic application has no single bound to point at.
pub(super) fn poly_copy_generic_error(ctx: &Ctx, span: Span, op: &str, ty: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot `{op}` a generic type applied to a variable in `{where_}` (line {})\n  `{ty}` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated",
        span.line
    )
}

pub(super) fn poly_ord_body_error(ctx: &Ctx, span: Span, op: &str, var: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` on the type variable `{var}` in `{where_}` (line {}) requires an `Ord` bound\n  declare `{var}: Ord` so every instantiation is comparable",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    let what = match pt {
        PolyType::Var(v) => format!("the type variable `{}`", sig.ty_var_names[*v as usize]),
        PolyType::Array(..) => "an array with a variable".to_string(),
        PolyType::Concrete(t) => format!("`{t}`"),
        PolyType::Quotation(..) => "a quotation".to_string(),
        PolyType::QuotLit => "a quotation literal".to_string(),
        PolyType::Ref(..) => "a reference".to_string(),
        // P7 slice 3a: rendered with the application, so the diagnostic
        // names which generic header and which arguments, not just "a
        // generic type".
        PolyType::Generic { .. } => format!("a generic type `{}`", poly_type_str(pt, sig)),
    };
    format!(
        "error: `{op}` is not permitted on {what} in `{where_}` (line {})",
        span.line
    )
}

pub(super) fn poly_op_operand_mismatch_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    a: &PolyType,
    b: &PolyType,
    sig: &PolySig,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` in `{where_}` (line {}) needs two operands of one type, found `{}` and `{}`",
        span.line,
        poly_type_str(a, sig),
        poly_type_str(b, sig),
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{callee}` in `{where_}` (line {}) expects `{expected}`, but the type variable `{var}` is not a concrete type",
        span.line
    )
}

/// Slice 13 (E4/R-B6): an accessor with no poly-body support -- ever
/// (`&^`, a retired fused-accessor spelling), or not yet (e.g. a fully
/// concrete `&![T N]`
/// parameter's accessors, folded to `PolyType::Concrete` and unmatched by
/// any `PolyType::Ref` arm) -- located, never a silent fallthrough to an
/// unknown-word error.
pub(super) fn poly_unsupported_accessor_error(ctx: &Ctx, span: Span, op: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{op}` is not yet supported in a generic body, in `{where_}` (line {})\n  monomorphize this word (or write a concrete wrapper) to use `{op}` today",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{spelled}` does not borrow a place in `{where_}` (line {})\n  it names nothing (a bare sigil cannot borrow whatever happens to be on the stack)",
        span.line
    )
}

/// `&x`/`&!x` where `x` is not a local currently in scope.
fn poly_borrow_of_non_local_error(ctx: &Ctx, span: Span, spelled: &str, local: &str) -> String {
    let spelled = crate::resolve::demangle_word(spelled);
    let local = crate::resolve::demangle_word(local);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{spelled}` does not borrow a place in `{where_}` (line {})\n  `{local}` is not a local in scope",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot borrow the local `{local}` of type `{var}` in `{where_}` (line {}, col {})\n  `{var}` might instantiate to a scalar, which has no address; borrow an aggregate (a struct, enum, array, or owning cell) instead",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot borrow the local `{local}` of type `{ty}` in `{where_}` (line {}, col {})\n  only an aggregate (a struct, enum, array, or owning cell) is borrowable; `{ty}` is not",
        span.line, span.col
    )
}

/// A quotation local, split out from `poly_borrow_of_non_aggregate_local_error`
/// (review, post-slice-12 rebase): a non-`inline` word's ordinary `[ ... ]`
/// parameter lowers to a real two-word `(code, env)` aggregate at the ABI
/// level, so "is not an aggregate" is false at the representation the
/// backend actually emits, even though it is true at the type-system level
/// this slice reasons over. Naming the actual reason -- unsupported, not
/// shapeless -- avoids a claim the ABI contradicts; borrowing a quotation is
/// 7b territory (a first-class capturing closure), not this slice's.
fn poly_borrow_of_quotation_local_error(ctx: &Ctx, span: Span, local: &str, ty: &str) -> String {
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot borrow the local `{local}` of type `{ty}` in `{where_}` (line {}, col {})\n  a quotation is not borrowable in a generic body",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    let sigil = if new_mutable { "&!" } else { "&" };
    let held = if live.mutable { "mutable" } else { "shared" };
    format!(
        "error: `{sigil}{place}` conflicts with a live borrow of `{place}` in `{where_}` (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first{POLY_BORROW_LIVENESS_NOTE}",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    let held = if live.mutable { "mutable" } else { "shared" };
    format!(
        "error: cannot consume the borrowed local `{place}` of type `{ty}` in `{where_}` (line {}, col {})\n  the {held} borrow taken at line {}, col {} is still live\n  a place stays borrowed until every reference derived from it is consumed{POLY_BORROW_LIVENESS_NOTE}",
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot name `{name}` in `{where_}` (line {}, col {}): a mutable borrow of it is still live (line {}, col {})\n  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates\n  finish with the borrow first, or `dup` for an independent copy{POLY_BORROW_LIVENESS_NOTE}",
        span.line, span.col, live.span.line, live.span.col,
    )
}

/// P7 slice 3b (R4/L2): a quotation literal that is still on the stack where
/// it would have to *exist* as a value -- at the polymorphic word's exit, or
/// leaving an eliminator arm. Splice-consumed literals only: a quotation in a
/// generic body has no runtime representation to return, store, or capture,
/// and the one thing that consumes one here is an eliminator in the same body.
pub(super) fn poly_quotation_not_consumed_error(ctx: &Ctx, span: Span) -> String {
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: a quotation in the polymorphic body of `{}` (line {}) is not consumed there\n  only an eliminator call in the same body consumes a quotation in a generic word: it cannot be returned, stored, or captured",
        crate::resolve::demangle_word(where_),
        span.line
    )
}

/// P7 slice 3b (R4/OQ6): `call`/`branch`/`if`/`times`/`tag` applied to a
/// quotation in a generic body. Each takes its quotation as a row-typed
/// parameter, so dispatching one needs row unification against an abstract
/// stack -- the machinery eliminator dispatch is specifically shaped to avoid.
/// Deferred to P7.S3b-follow, and located rather than an `unknown word`.
pub(super) fn poly_quotation_combinator_unsupported_error(
    ctx: &Ctx,
    span: Span,
    word: &str,
) -> String {
    let word = crate::resolve::demangle_call(word);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{word}` on a quotation in the polymorphic body of `{}` (line {}) is not yet supported\n  only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)",
        crate::resolve::demangle_word(where_),
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{word}` in `{}` (line {}) eliminates `{found}`, which is not a concrete enum\n  an abstract scrutinee needs an enum-kind bound on the type variable, which this slice does not have",
        crate::resolve::demangle_word(where_),
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{word}` in `{}` (line {}) eliminates a reference, which is not yet supported in a generic body\n  pass the owned `{enum_name}` instead",
        crate::resolve::demangle_word(where_),
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: the arms of `{word}` in `{}` (line {}) disagree: an earlier one leaves `{expected}`, this one leaves `{found}`\n  a type variable is rigid across arms: it is never bound to the other arm's type",
        crate::resolve::demangle_word(where_),
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    let sigil = |b: &PolyBorrow| if b.mutable { "&!" } else { "&" };
    format!(
        "error: the arms of `{word}` in `{}` (line {}) borrow `{}` differently: `{}{}` (line {}) against `{}{}` (line {})\n  one place is borrowed at one mutability across every arm, or the merged table could not answer a later use of it",
        crate::resolve::demangle_word(where_),
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
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: the local `{local}` of type `{ty}`, bound in an arm of `{word}` in `{}` (line {}), is never consumed\n  nothing is dropped for you: consume it in the arm that binds it",
        crate::resolve::demangle_word(where_),
        span.line
    )
}

/// Slice 13 (E3/D6): `&>`/`&!>` on a generic-length array (`['T 'N]`) -- the
/// element cannot be statically bounds-checked without a known count.
pub(super) fn poly_generic_length_index_error(ctx: &Ctx, span: Span, len_var: &str) -> String {
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: cannot index a generic-length array in `{where_}` (line {}, col {})\n  the array's length is the type variable `{len_var}`, so its element cannot be statically bounds-checked; index a concrete-length array (`['T 4]`), or use a fixed length in this word's signature",
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
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}` in `{name}` (line {})\n  `{ty}` is linear and has no `Copy` instance, so a linear value cannot be duplicated; `{var}: Copy` is unsatisfied",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with linear type `{ty}`: `{var}: Copy` is unsatisfied"
        ),
    }
}

pub(super) fn poly_ord_bound_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    var: &str,
    ty: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}` in `{name}` (line {})\n  `{ty}` is not `Ord`; `{var}: Ord` is unsatisfied",
            span.line
        ),
        Ctx::Line { .. } => format!(
            "error: cannot instantiate `{var}` of `{callee}` with `{ty}`: `{var}: Ord` is unsatisfied"
        ),
    }
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
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `{callee}` in `{name}` (line {line}) resolved `{var}` to both `{a}` and `{b}`"
        ),
        Ctx::Line { .. } => {
            format!("error: `{callee}` resolved `{var}` to both `{a}` and `{b}`")
        }
    }
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
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: `{callee}` in `{name}` (line {line}) resolved length `{var}` to both `{a}` and `{b}`"
        ),
        Ctx::Line { .. } => {
            format!("error: `{callee}` resolved length `{var}` to both `{a}` and `{b}`")
        }
    }
}

pub(super) fn poly_array_expected_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    found: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    match ctx {
        Ctx::Word { name, .. } => format!(
            "error: type mismatch in `{name}` (line {})\n  `{callee}` expected an array operand, found `{found}`",
            span.line
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{callee}` expected an array operand, found `{found}`")
        }
    }
}

pub(super) fn poly_unbound_output_error(ctx: &Ctx, span: Span, callee: &str, var: &str) -> String {
    let callee = crate::resolve::demangle_call(callee);
    let where_ = ctx.word_name().unwrap_or("<line>");
    format!(
        "error: `{callee}` in `{where_}` (line {}) has output variable `{var}` that no input binds",
        span.line
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
            format!("[{} {}]", poly_type_str(elem, sig), l)
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
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
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
        let mut module = parse(&tokens).unwrap();
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
            "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
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
            "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
             : wrap ( 'T -- Result['T i64] ) Ok ;\n\
             : main ( -- ) true wrap drop ;\n",
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
            "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
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
            "type: Pair 'T val1 'T val2 'T ;\n\
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
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) [ add ] dupit drop drop ;\n",
        )
        .expect_err("a quotation passed to a polymorphic word should be rejected");
        assert!(
            err.contains("a quotation cannot be passed to `dupit`"),
            "check_poly_call should name `dupit`, got: {err}"
        );
    }
    #[test]
    fn poly_term_admits_a_quotation_literal_as_a_marker_slot() {
        // P7 slice 3b (R2): the literal pushes a slot carrying its identity in
        // `quot` and a `pt` that is not a value type. Checked at the
        // `poly_term` level because no source program can observe the marker
        // directly: every route out of the body rejects it.
        let sig = bare_sig();
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
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
            &[],
            &[],
            &[],
            &mut overloads,
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
        // L3: the literal's identity rides the slot, so a shuffle reorders the
        // indices with no special handling. Pinned here rather than through a
        // source program: a *tagged* literal must reach its eliminator by
        // written adjacency (the concrete path's rule), so no program can put
        // a shuffle between two arms.
        let sig = bare_sig();
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let mut overloads = HashMap::new();
        let quot = Term {
            kind: TermKind::Quotation(Vec::new(), true, None),
            span: Span::default(),
        };
        let swap = Term {
            kind: TermKind::Call("swap".to_string()),
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
                &[],
                &[],
                &[],
                &mut overloads,
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
        // L1: `'T` stays rigid across arms. One arm leaving `'T` and another
        // `i64` is a located rejection naming both sides in order, never a
        // mid-body bind of `'T := i64`.
        let err = check_src(&format!(
            "{SHAPE}\
             : bad ( 'T: Copy Shape -- 'T )\n\
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
        // L4: the arms' borrow tables are unioned, not picked between. A
        // missing record reads as "no conflict", so dropping either arm's is a
        // silent false accept -- both directions are asserted, since "pick arm
        // A" keeps `x` and drops `y` and an `x`-only assertion would not flip.
        let program = |later: &str| {
            format!(
                "type: P a i64 ;\n\
                 {SHAPE}\
                 : bad ( 'T: Copy P P Shape -- 'T )\n\
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
    #[test]
    fn poly_eliminator_arm_leaving_its_own_variant_is_error() {
        // R2 step 5b: with two arms, R3's rigid-arm-disagreement check
        // (different exit shapes) fires before this guard ever gets a chance
        // to look at the escaping `Type::Variant`, so a two-arm repro cannot
        // exercise it. A single-variant enum is exhaustive with one arm and
        // reaches this guard directly. Stubbing it out here builds and
        // double-drops the linear `Spy` payload underneath, since `is_copy`
        // falls through `Type::Variant` to `true`.
        let err = check_src(&format!(
            "{SPY}\
             type: One | A p Spy ;\n\
             : bad ( 'T: Copy One -- 'T ) ~[ ( A ) ] One? ;\n\
             : main ( -- ) 1 9 Spy A bad drop ;\n"
        ))
        .expect_err("an arm leaving its own narrowed variant unconsumed is an escape");
        assert!(
            err.contains("an arm of `One?` leaves `One.A` on the stack"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn poly_term_rejects_an_array_constructor() {
        // Slice 6h: an array constructor in a polymorphic body is rejected
        // eagerly (no interning route exists for a body-internal shape absent
        // from the signature).
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
    fn polyslot_int_val_folds_lits() {
        // R1: `int_val` carries what the deleted `lits` shadow did -- set on
        // `IntLit`, `None` elsewhere, truncated on `Bind`. Round-tripped
        // directly at the `poly_term` level since a bound local's own
        // literal-ness is discarded (D6), not observable through `check_src`.
        let sig = bare_sig();
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
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
            &[],
            &[],
            &[],
            &mut overloads,
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
            &[],
            &[],
            &[],
            &mut overloads,
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
        check_src(": less ( 'T: Ord 'T -- bool ) gt ;\n: main ( -- ) 3 4 less drop ;").unwrap();
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
            check_src(": less ( 'T: Ord 'T -- bool ) gt ;\n: main ( -- ) true false less drop ;")
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
        let err = check_src(": bad ( 'T 'T -- bool ) gt ;\n: main ( -- ) ;").unwrap_err();
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
                ": choose inline ( 'T 'T bool -- 'T ) | a b flag | flag ~[ a b drop ] ~[ b a drop ] if ;\n: main ( -- ) 1 2 true choose drop ;",
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
            "{SPY}: arm_leak ( Spy Spy bool -- Spy ) | a b flag | flag ~[ a b | y | ] ~[ a drop b ] if ;\n: main ( -- ) ;",
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
                ": both inline ( 'T 'T bool -- ) | a b flag | flag ~[ a drop b drop ] ~[ b drop a drop ] if ;\n: main ( -- ) ;",
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
            "{SPY}: one ( Spy bool -- ) | x flag | flag ~[ x drop ] ~[ ] if ;\n: main ( -- ) ;"
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
            "{SPY}: none ( Spy bool -- ) | x flag | flag ~[ ] ~[ ] if ;\n: main ( -- ) ;"
        ))
        .unwrap_err();
        assert!(err.contains('x'), "names the leaked local: {err}");
        assert!(err.contains("never consumed"), "unexpected message: {err}");
    }
    #[test]
    fn check_branch_condition_not_bool_is_error() {
        // T6: `if`'s condition must be a `bool`. Slice 10c: the guard is now
        // `if`'s own declared parameter type rather than a hand-written arm,
        // and a spliced poly body reports the operand at its instantiated
        // stand-in type.
        let err =
            check_src(": bad inline ( 'T 'T -- 'T ) ~[ drop ] ~[ drop ] if ;\n: main ( -- ) ;")
                .unwrap_err();
        assert!(err.contains("if"), "names the `if`: {err}");
        assert!(err.contains("`bool`"), "names the expected type: {err}");
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
            ": bad inline ( 'T: Copy bool -- 'T ) | x flag | flag ~[ x ] ~[ x x ] if ;\n: main ( -- ) ;",
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
            "{SPY}: bad ( Spy bool -- ) | x flag | flag ~[ x drop ] ~[ x drop ] if x drop ;\n: main ( -- ) ;"
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
    fn poly_op_on_variable_error_names_a_reference() {
        // Slice 13 (review fix): `poly_op_on_variable_error`'s `Ref` describer
        // (`"a reference"`) is reachable from source -- `len` rejects a
        // reference the same way it rejects a bare type variable -- but had
        // no test asserting the exact wording.
        let err = check_src(": f ( &['T 4] -- usize ) len ;\n").unwrap_err();
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
        let structs: [StructDecl; 0] = [];
        let enums: [EnumDecl; 0] = [];
        let arrays: [ArrayDecl; 0] = [];
        let refs: [RefDecl; 0] = [];
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
            &refs,
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
            &refs,
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
            &refs,
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
        assert_eq!(poly_type_str(&poly_ref(arr, false), &sig), "&['T 4]");
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
            check_src("type: Box 'T val 'T ;\n: dup-box ( Box['T] -- Box['T] Box['T] ) dup ;\n")
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
        let tokens = lex(": peek ( ['T 4] -- &['T 4] ) ;").unwrap();
        let module = parse(&tokens).unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(poly_type_str(&sig.outputs[0], sig), "&['T 4]");
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
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
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
            "error: cannot `dup` a value of type `!Slice[i64]`: `!Slice[i64]` is exclusive: at most one may be live for a place, so copying it would make a second one; use it where it is, or borrow again once it is consumed"
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
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
        let env: HashMap<String, Vec<Overload>> = HashMap::new();
        let mut scope = PolyScope::default();
        let mut overloads = HashMap::new();
        let stack = poly_term(
            &Term {
                kind: TermKind::Call("len".to_string()),
                span: Span::default(),
            },
            vec![PolySlot::new(PolyType::Concrete(shared))],
            &mut scope,
            &sig,
            &ctx,
            &env,
            &[],
            &[],
            &[],
            &mut overloads,
        )
        .expect("`len` answers a slice");
        assert_eq!(
            stack.iter().map(|s| s.pt.clone()).collect::<Vec<_>>(),
            vec![PolyType::Concrete(Type::Usize)]
        );
    }

    #[test]
    fn poly_is_copy_tracks_a_reference_mutability_not_its_referent() {
        // Slice 13 (D3/R-A5): mirrors the monomorphic `is_copy` on
        // `Type::Ref` -- shared is `Copy`, mutable is not, and the referent's
        // own linearity is irrelevant either way. Answering `true`
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
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
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
            "error: cannot `dup` a mutable reference in `<line>` (line 7)\n  `&!'T` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow",
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
    fn poly_var_id_on_a_reference_is_none() {
        // Slice 13 (R-A9): a reference is not a bare variable, so the
        // existing `_ => None` already answers correctly and the function
        // needs no new arm. Pinned rather than edited.
        assert_eq!(poly_var_id(&poly_ref(PolyType::Var(0), false)), None);
    }

    #[test]
    fn unify_poly_input_matches_a_declared_reference_slot() {
        // Slice 13 (R-A6): a declared `&['T 4]` binds `'T` through the
        // registry's referent; a mutability mismatch and a non-reference slot
        // are located mismatches, never a silent bind.
        let sig = ref_sig();
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
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
            &refs,
            &mut subst,
        )
        .expect("`&['T 4]` should unify against `&[i64 4]`");
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
            &refs,
            &mut subst2,
        )
        .expect_err("a mutability mismatch must be a located type mismatch");
        assert_eq!(
            err,
            "error: type mismatch: `f` expected `&['T 4]`, found `&![i64 4]`",
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
            &refs,
            &mut subst3,
        )
        .expect_err("a non-reference slot must be a located type mismatch");
        assert_eq!(
            err,
            "error: type mismatch: `f` expected `&['T 4]`, found `[i64 4]`",
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
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let ty = apply_subst(
            &sig,
            &poly_ref(PolyType::Var(0), true),
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
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
            ": first ( ['T: Copy 4] -- 'T ) | a | &a 0 &> @ ;\n\
             : main ( -- ) 10 4 fill first drop ;\n",
        )
        .expect("a shared prefix borrow, array-element ref, and fetch should check");
    }

    #[test]
    fn poly_reference_word_rejects_borrowing_a_bare_variable_local() {
        // E2/D5: a bare `'T` local might instantiate to a scalar, which has
        // no address, so it is refused uniformly rather than deferred.
        let err = check_src(": badvar ( 'T: Copy -- 'T )\n  | t |\n  &t\n;\n").unwrap_err();
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
        let err = check_src(": g ( i64 'T: Copy -- 'T ) | n t | &n drop n drop t ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: cannot borrow the local `n` of type `i64` in `g` (line 1, col 36)\n  only an aggregate (a struct, enum, array, or owning cell) is borrowable; `i64` is not"
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
        // parameter *is* a two-word aggregate at the ABI level, so that claim
        // is false at the representation the backend emits even though the
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
        // E3/D6: `['T 'N]` has no known count, so its element cannot be
        // statically bounds-checked; only a concrete-length array's element
        // is accessible this slice.
        let err =
            check_src(": badidx ( ['T 'N] -- 'T )\n  | a |\n  &a 0\n  &>\n  @\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: cannot index a generic-length array in `badidx` (line 4, col 3)\n  the array's length is the type variable `'N`, so its element cannot be statically bounds-checked; index a concrete-length array (`['T 4]`), or use a fixed length in this word's signature"
        );
    }

    #[test]
    fn poly_reference_word_rejects_borrowing_a_moved_local() {
        // E5: borrowing is not a move, but the referent still has to be
        // there -- a local already consumed holds nothing.
        let err = check_src(": badmove ( ['T 4] -- 'T )\n  | a |\n  a drop\n  &a 0 &> @\n;\n")
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
    fn poly_reference_word_rejects_out_of_range_literal_index() {
        // R-B3: the literal `9` is statically bounds-checked against the
        // array's known length 4, mirroring the monomorphic `check_array_index`.
        let err = check_src(": oob ( ['T: Copy 4] -- 'T )\n  | a |\n  &a 9 &> @\n;\n").unwrap_err();
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
            ": setat ( ['T: Copy 4] 'T -- ['T 4] ) | a v | &!a 2 &!> v ! a ;\n\
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
        let err =
            check_src(": twomut ( ['T: Copy 4] -- ['T 4] )\n  | a |\n  &!a &!a drop drop a\n;\n")
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
        let err =
            check_src(": mixed ( ['T: Copy 4] -- ['T 4] )\n  | a |\n  &!a &a drop drop a\n;\n")
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
        check_src(": twoshared ( ['T: Copy 4] -- ['T 4] ) | a | &a &a drop drop a ;\n")
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
            ": settwo ( ['T: Copy 4] 'T -- ['T 4] )\n  | a v |\n  &!a 0 &!> v !\n  &!a 1 &!> v !\n  a\n;\n",
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
            ": hidden ( ['T: Copy 4] 'T -- ['T 4] )\n  | a v |\n  &!a | r |\n  &!a 0 &!> v !\n  r 1 &!> v !\n  a\n;\n",
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
            ": sharedname ( ['T: Copy 4] -- ['T 4] 'T )\n  | a |\n  &a 0 &> @ | e |\n  &a a swap drop\n  e\n;\n",
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
            ": coarse ( ['T: Copy 4] ['T 4] 'T -- ['T 4] ['T 4] )\n  | a b v |\n  &b\n  &!a 0 &!> v !\n  &!a 1 &!> v !\n  drop a b\n;\n",
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
        let err = check_src(": consume ( ['T 4] -- ['T 4] )\n  | a |\n  &a a swap drop\n;\n")
            .unwrap_err();
        assert_eq!(
            err,
            "error: cannot consume the borrowed local `a` of type `['T 4]` in `consume` (line 3, col 6)\n  the shared borrow taken at line 3, col 3 is still live\n  a place stays borrowed until every reference derived from it is consumed\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local"
        );
    }

    #[test]
    fn poly_call_term_rejects_naming_a_mutably_borrowed_local() {
        // R-B5, the naming side for a `Copy` aggregate: the read does not
        // consume it, but the name still denotes the storage the live `&!`
        // mutates.
        let err =
            check_src(": alias ( ['T: Copy 4] -- ['T 4] 'T )\n  | a |\n  &!a a swap 0 &!> @\n;\n")
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
            ": dupmut ( ['T: Copy 4] 'T -- ['T 4] ) | a v | &!a dup 0 &!> v ! drop a ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: cannot `dup` a mutable reference in `dupmut` (line 1)\n  `&!['T 4]` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow"
        );
        check_src(
            ": dupshared ( ['T: Copy 4] -- ['T 4] 'T ) | a | &a dup drop 0 &> @ | e | a e ;\n",
        )
        .expect("a shared reference is `Copy` and may be duplicated");
    }

    #[test]
    fn poly_body_store_rejects_a_shared_receiver() {
        // R-B4: `!` is `( &!T T -- )`. A shared receiver is a mutability
        // mismatch rendered off the receiver's own referent, the same shape
        // `&>` uses for the mirror-image mismatch.
        let err = check_src(": rdstore ( ['T: Copy 4] 'T -- ['T 4] ) | a v | &a 0 &> v ! a ;\n")
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
            ": wrongval ( ['T: Copy 4] 'T -- ['T 4] ) | a v | &!a 0 &!> 5 ! v drop a ;\n",
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
        let err = check_src(": linstore ( ['T 4] 'T -- ['T 4] ) | a v | &!a 0 &!> v ! a ;\n")
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
        let err =
            check_src(": g ( ['T 4] -- 'T ) | a | &a 0 &> @ ;\n: main ( -- ) 10 4 fill g drop ;\n")
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
             : bump ( 'T: Copy -- 'T ) | v | &!COUNT @ 1 add &!COUNT swap ! v ;\n\
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
             : bump ( 'T: Copy -- 'T ) &!COUNT &!COUNT drop drop ;\n\
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
        // the way the monomorphic twin does (`&>` expected `&[i64 4]`, found
        // `&![i64 4]`) rather than the operand-family text, which reads as
        // if `&>` never accepts a reference at all.
        let err = check_src(": rd ( &!['T: Copy 4] -- 'T )\n  0 &> @\n;\n").unwrap_err();
        assert_eq!(
            err,
            "error: type mismatch in `rd` (line 2)\n  `&>` expected `&['T 4]`, found `&!['T 4]`\n  note: declared ( -- )"
        );
    }

    #[test]
    fn check_poly_array_index_bounds_checks_a_literal_and_requires_conversion_otherwise() {
        // R-B3, direct unit coverage of the helper mutation testing would
        // otherwise miss: a literal within range passes, one out of range
        // rejects, and a computed (non-literal) `i64` needs the explicit
        // `>usize` conversion the monomorphic checker also requires.
        let sig = ref_sig();
        let ctx = Ctx::Line {
            structs: &[],
            enums: &[],
        };
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
}
