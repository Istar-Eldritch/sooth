use super::*;

/// Walk a term sequence. `scope` is the names in scope and the move-state of
/// the linear ones, mutated in place as terms bind and mention names; `tail`
/// marks the sequence as
/// occupying its word's tail position, so its final term (and, recursively,
/// both arms of a final `if`) sits on the self-tail-call back-edge. The rule
/// mirrors `tail_position_calls`/`lower_terms`; all three must stay in
/// lockstep.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_terms(
    terms: &[Term],
    stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
) -> Result<Vec<Slot>, String> {
    check_terms_relaxed(
        terms,
        stack,
        ctx,
        env,
        arrays,
        cells,
        refs,
        slices,
        prov,
        scope,
        tail,
        poly,
        &HashSet::new(),
        false,
    )
}

/// D6 relaxation entry point: `outer_releasable` is the set of ancestor-bound
/// names this invocation's caller has already proven have no residual use
/// past this block (`releasable_into`), and `back_edge` is whether this
/// invocation's own body can run more than once or be entered from elsewhere
/// (a spliced quotation or combinator body), which changes how a granted
/// name's use inside is tracked (see the `Liveness` struct doc).
/// `check_terms` above is the plain entry point every root invocation (a
/// word body, a `case` clause) uses: nothing is ancestor to those, so both
/// are empty/`false`.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_terms_relaxed(
    terms: &[Term],
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
    outer_releasable: &HashSet<String>,
    back_edge: bool,
) -> Result<Vec<Slot>, String> {
    let last = terms.len().wrapping_sub(1);
    // Q1/D3: last-use over *this* invocation's term list, in its own index
    // space. Nested bodies re-enter `check_terms` and rebuild their own.
    let live = Liveness::scan(terms, outer_releasable, back_edge);
    // The depth this invocation was entered at: a binding at or past this
    // position was made *within* this invocation (nothing outside it could
    // ever need it), a binding before it is ancestor-bound and only a
    // recursion candidate if it is also in `outer_releasable` (`releasable_into`).
    let base_depth = scope.depth();
    for (i, term) in terms.iter().enumerate() {
        stack = check_term(
            term,
            stack,
            ctx,
            env,
            arrays,
            cells,
            refs,
            slices,
            prov,
            scope,
            tail && i == last,
            poly,
            &live,
            i,
            terms,
            base_depth,
            outer_releasable,
        )?;
    }
    Ok(stack)
}

#[allow(clippy::too_many_arguments)]
fn check_term(
    term: &Term,
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
    live: &Liveness,
    at: usize,
    siblings: &[Term],
    base_depth: usize,
    outer_releasable: &HashSet<String>,
) -> Result<Vec<Slot>, String> {
    let span = term.span;
    match &term.kind {
        TermKind::IntLit(n) => {
            // A bare integer literal is the one D8 source: fresh off the
            // term, it may still silently fill a `usize` position. Its value
            // is retained for the compile-time-count array positions (M1, X4).
            stack.push(Slot {
                ty: Type::I64,
                literal: true,
                int_val: Some(*n),
                variant_idx: None,
                alias: None,
                deriv: None,
                quot: None,
                surviving: None,
            });
            Ok(stack)
        }
        TermKind::FloatLit(_) => {
            stack.push(Slot::computed(Type::F64));
            Ok(stack)
        }
        TermKind::StrLit(_) => {
            stack.push(Slot::computed(Type::Str));
            Ok(stack)
        }
        TermKind::Bind(names) => {
            // R1: pop one value per name at this point, leftmost name deepest,
            // the same shape whether this is the entry binding or a mid-body
            // one. R5: the frame floor is whatever the stack holds here, which
            // in a word body is its declared inputs and nothing beneath them.
            let mut seen = HashSet::new();
            for name in names {
                reject_variant_local(ctx, name, "local")?;
                reject_duplicate_local(ctx, name, span, &mut seen)?;
                let mangled = crate::resolve::mangle(name, span.module);
                let collides = is_builtin_word_name(name)
                    || env.contains_key(name)
                    || env.contains_key(&mangled)
                    || poly.env.contains_key(name)
                    || poly.env.contains_key(&mangled)
                    || poly.combinators.contains_key(name)
                    || poly.combinators.contains_key(&mangled);
                if collides {
                    return Err(callable_local_error(ctx, name, span));
                }
                if scope.local_type(name).is_some() {
                    return Err(rebound_local_error(ctx, span, name));
                }
            }
            if stack.len() < names.len() {
                let op = format!("| {} |", names.join(" "));
                return Err(underflow_error(ctx, span, &op, names.len(), stack.len()));
            }
            let bound = stack.split_off(stack.len() - names.len());
            for (name, slot) in names.iter().zip(bound) {
                let linear = ctx.with_extended_type_slices(|structs, enums| {
                    is_linear(slot.ty, structs, enums, arrays)
                });
                scope.bind(name, slot, linear, prov);
            }
            Ok(stack)
        }
        TermKind::Call(name, type_args, len_args) => {
            // P7.S3t (R3): exactly one of this arm's dispatch routes consumes
            // an explicit type-argument list -- the polymorphic-call
            // interception far below. The guard is written as an *allow*, and
            // only rejects a route added later by default if that route is
            // one `poly_call_takes_type_args` cannot mistake for the poly
            // interception: a route added *ahead* of the poly path that
            // claims a name `poly.env` also holds (as the eliminator and
            // combinator routes already do) fails open instead, exactly as
            // it does for those two today. A route that silently dropped the
            // list would link the wrong specialization, which is a
            // miscompile rather than a diagnostic, so a genuinely new route
            // needs its own exclusion here, not just an assumption that the
            // guard already covers it.
            if (!type_args.is_empty() || !len_args.is_empty())
                && !poly_call_takes_type_args(
                    name,
                    span,
                    &stack,
                    ctx,
                    env,
                    arrays,
                    cells,
                    refs,
                    scope,
                    poly,
                    poly.trait_resolve.impls,
                    poly.trait_resolve.traits,
                )
            {
                return Err(no_type_arguments_error(
                    span,
                    name,
                    !type_args.is_empty(),
                    !len_args.is_empty(),
                ));
            }
            if let Some(binding) = scope.local(name) {
                let (ty, aliases, held, quot, surviving) = (
                    binding.ty,
                    binding.aliases,
                    binding.deriv,
                    binding.quot,
                    binding.surviving,
                );
                // P7 slice 3c (R12): a slice local reborrows here too. It is
                // not a `Type::Ref`, so `ref_parts` alone would send a view
                // down the owned-value arm below, where a non-linear value is
                // "merely read" -- and a mutable view named twice would be two
                // live `&!` into one buffer with nothing to say so.
                match borrow_mutability(ty, refs) {
                    // Naming a reference local is a reborrow, not a move.
                    // A mutable one suspends its place: a second reborrow while
                    // anything derived from the first is still live would be two
                    // live mutable references into one place.
                    Some(mutable) => {
                        if mutable {
                            if let Some(id) = live_deriv(&stack, scope, prov, live, at, |d| {
                                d.reborrow && d.place == *name
                            }) {
                                return Err(suspended_place_error(ctx, span, name, prov.deriv(id)));
                            }
                        }
                        let deriv = prov.reborrow(name, held, mutable, span);
                        stack.push(Slot::derived(ty, Some(deriv)));
                    }
                    None => {
                        // Consuming a place while a reference derived from
                        // it is live would leave that reference aimed at storage
                        // its owner has given away. Only a linear local is
                        // consumed by being named; a Copy one is merely read.
                        if ctx.with_extended_type_slices(|structs, enums| {
                            is_linear(ty, structs, enums, arrays)
                        }) {
                            if let Some(id) = live_borrow_of(&stack, scope, prov, live, at, name) {
                                return Err(consume_of_borrowed_place_error(
                                    ctx,
                                    span,
                                    name,
                                    ty,
                                    prov.deriv(id),
                                ));
                            }
                        }
                        // Mentioning a linear local moves its value
                        // out; a second mention names the site that already
                        // consumed it.
                        if let Err(site) = scope.moves.take(name, span) {
                            return Err(use_after_move_error(ctx, span, name, ty, site));
                        }
                        // The direction symmetric with the check at the
                        // borrow: this naming would be the *second* name for
                        // storage a live `&!` already reaches, so the mutation
                        // is just as silently observable as if the naming had
                        // come first. Only an aggregate has a region, and so
                        // only an aggregate can be a second name for one.
                        if aliases.is_some() {
                            if let Some(id) =
                                live_mutable_borrow_of(&stack, scope, prov, live, at, name)
                            {
                                return Err(naming_aliases_borrowed_place_error(
                                    ctx,
                                    span,
                                    name,
                                    prov.deriv(id),
                                ));
                            }
                        }
                        // Naming an aggregate does not copy it, so the
                        // pushed value denotes the local's own region, located
                        // here so a later borrow can point at this naming.
                        stack.push(Slot {
                            alias: aliases.map(|set| Alias { set, span }),
                            quot,
                            // 7b/R19: forward a stored closure's (or its
                            // carrier's) surviving set across the read so the
                            // captured referents stay live to the call.
                            surviving,
                            // P7b.S6d-PREREQ (REQ-4d site 7): naming a
                            // reference-bearing aggregate must not sever its
                            // borrow chain. A *reference*-typed local takes
                            // the reborrow arm above, which inherits; this arm
                            // is where an aggregate carrying a slice arrives,
                            // and dropping the deriv here would make `w |x|`
                            // a one-token launder.
                            deriv: scope.local(name).and_then(|b| b.deriv),
                            ..Slot::computed(ty)
                        });
                    }
                }
                return Ok(stack);
            }
            // P8 S2 (R2): an intrinsic name this module never imported is not
            // a builtin *here*. Every builtin-dispatch arm below is skipped
            // for it, so the name falls through to the ordinary env/overload
            // path, and that fall-through reports the missing import rather
            // than `unknown word`. The env lookup never actually claims the
            // name: a module's own word under a builtin spelling arrives
            // mangled, so a gated name reaching here has no candidate and the
            // fall-through is always the diagnostic.
            let gated = intrinsic_is_gated_out(ctx, span, name);
            // Slice 10c (R-P3-1a): `branch` is intercepted here, beside
            // `call`, for the same reason: it is a compiler-known word whose
            // operands are quotations, so every builtin family below it would
            // reject them under R11's default-deny. It is the *single*
            // sanctioned exemption from that deny; nothing else in the
            // language takes a quotation operand to a builtin name.
            if name == "branch" && !gated {
                return check_branch(
                    stack,
                    span,
                    ctx,
                    env,
                    arrays,
                    cells,
                    refs,
                    slices,
                    prov,
                    scope,
                    tail,
                    poly,
                    live,
                    at,
                    siblings,
                    base_depth,
                    outer_releasable,
                );
            }
            // R6: `call` is a compiler-known word intercepted before every
            // builtin family and user-word lookup (a local named `call`
            // already won above). It requires a statically-known
            // quotation literal on top (D4) and splices its interned body
            // against the live stack, so `[ 1 add ] call` checks as `1 add` (D3).
            if name == "call" {
                let Some(top) = stack.pop() else {
                    return Err(underflow_error(ctx, span, "call", 1, 0));
                };
                // R8: an *abstract* quotation (typed by a declared parameter,
                // `Slot.ty == Type::Quotation`, no `Known` literal) checks
                // against its declared effect directly: pop `eff.inputs`
                // deepest-first, push `eff.outputs`. This is the standalone
                // (def-site) check of a quotation-taking word (D4): `f call`
                // checks against `f`'s declared effect exactly as an ordinary
                // word call checks against its `Sig`, with no splice.
                if top.quot.is_none() {
                    // Slice 10a (R2): `call` on a `~` is accepted and is
                    // statically always a splice; the accessor treats a `~`
                    // abstract parameter exactly like an ordinary one here.
                    if let Some(eff) = crate::ast::is_quotation_type(top.ty) {
                        return check_abstract_quotation_call(eff, span, stack, ctx, "call");
                    }
                    return Err(call_needs_quotation_error(ctx, span));
                }
                let Some(QuotRef::Known(id)) = top.quot else {
                    return Err(call_needs_quotation_error(ctx, span));
                };
                // Slice 12 (R-C2): `call` is not a flavour boundary. Nothing
                // is materialized here -- a literal is spliced under either
                // spelling -- so `~` decides nothing and both are accepted.
                //
                // Splice the body against the current locals/scope in lexical
                // extent (capture is free, recon 9), bracketed like an `if`
                // arm so a body that binds does not leak past the `call` and a
                // linear value bound inside it is caught by `leave_block`
                // (R6). Slice 10c (R-P1-6): `tail` is threaded, not pinned
                // `false` -- a spliced literal runs in place of the `call`, so
                // at a tail `call` its own tail terms are the enclosing word's
                // and a self-call there is the back-edge. This is how tail
                // position reaches a combinator's quotation parameter (`t call`
                // in a hand-written `if`); lowering threads the same flag
                // through the same splice.
                //
                // D6: a quotation body can be called from elsewhere too, so a
                // granted outer name is tracked as a back-edge body (used
                // anywhere inside pins it live throughout, unused kills it
                // throughout), never at its own last use inside.
                let body = prov.quotations[id.0].body.clone();
                let depth = scope.depth();
                let granted = releasable_into(
                    scope,
                    base_depth,
                    outer_releasable,
                    &siblings[at + 1..],
                    live,
                    at,
                );
                stack = check_terms_relaxed(
                    &body, stack, ctx, env, arrays, cells, refs, slices, prov, scope, tail, poly,
                    &granted, true,
                )?;
                leave_block(
                    ctx,
                    scope,
                    depth,
                    BlockEnd::Arm {
                        token: "call",
                        span,
                    },
                )?;
                return Ok(stack);
            }
            if let Some(stack) = check_reference_word(
                name,
                span,
                &mut stack,
                ctx,
                scope,
                arrays,
                cells,
                refs,
                slices,
                prov,
                live,
                at,
                poly.resolved_fields,
                poly.resolved_variant_fields,
            )? {
                return Ok(stack);
            }
            // R8 (D4): `!`/`+!` into a `&!Type::Quotation` referent is a
            // materialization boundary (an array element or a struct field via
            // reference). Materialize a `Known` literal in place before
            // `check_access_word` (whose bare-quotation store guard would else
            // reject it), running the R15 admission rule on a capturing one.
            // The store is only an in-frame boundary when the `&!` referent's
            // own root is a local of *this* frame (R21) -- a `&!` reached
            // through a parameter/global-rooted reference chain writes into
            // storage this frame does not own, so a frame-rooted capture
            // stored there escapes exactly as if it had been returned (B1:
            // otherwise a closure over a frame-local borrow, stored through a
            // `&!` parameter, would outlive the frame that owns its referent).
            // The referent's declared effect is the boundary's expected effect.
            if matches!(name.as_str(), "!" | "+!") && stack.len() >= 2 {
                let vi = stack.len() - 1;
                if let Some(QuotRef::Known(id)) = stack[vi].quot {
                    // No owning arm here. P7.S3v (R6) admits an owning
                    // quotation as a struct field, a variant field and a cell
                    // payload, so a `&!` projection of one does reach this
                    // point -- but it needs no materialization boundary: the
                    // owning flavour is `is_linear`, so `check_access_word`
                    // rejects the store outright ("cannot access the linear
                    // referent") because overwriting would leak the closure
                    // being replaced.
                    if let Some((Type::Quotation(eff), _)) = ref_parts(stack[vi - 1].ty, refs) {
                        let qspan = prov.quotations[id.0].span;
                        let escaping = !ref_root_is_in_frame(stack[vi - 1].deriv, prov, scope);
                        stack[vi] = materialize_quotation_at_boundary(
                            id, eff, false, escaping, name, qspan, ctx, env, arrays, cells, refs,
                            slices, prov, scope, poly,
                        )?;
                    }
                }
                // Review fix: the gate above only fires for a value that is
                // still a literal `Known` quotation. A value already erased
                // into a struct/array/cell carrier (its `surviving` set is
                // non-empty but `quot` is `None`) escapes exactly the same
                // way if the store's referent is rooted outside this frame --
                // check that here, before the carried set is ever unioned
                // onto anything. Guarded on `ref_parts` succeeding so a
                // malformed non-reference operand still falls through to
                // `check_access_word`'s ordinary type-mismatch diagnostic.
                if let Some(set) = stack[vi].surviving {
                    if ref_parts(stack[vi - 1].ty, refs).is_some()
                        && !ref_root_is_in_frame(stack[vi - 1].deriv, prov, scope)
                    {
                        if let Some(member) =
                            prov.surviving_set(set).iter().find(|m| m.frame_rooted)
                        {
                            // No `owning` remedy: this is the transitive case,
                            // where the closure is already stored in a
                            // carrier, and an owning closure may not be stored
                            // in an aggregate at all (the containment rule).
                            return Err(past_owning_frame_error(ctx, span, &member.name, false));
                        }
                        if prov.surviving_set_is_bundle(set) {
                            return Err(multi_capture_escaping_error(ctx, span));
                        }
                    }
                    // R19/R22: storing an erased closure through a `&!` referent
                    // makes the referent's owning aggregate its carrier -- the
                    // surviving set rides onto that root binding so the captures
                    // stay live to a later fetch-and-`call` (R20) and cannot
                    // silently escape by returning the aggregate (R22).
                    let root = stack[vi - 1]
                        .deriv
                        .and_then(|did| prov.deriv(did).owned_root.clone());
                    if let Some(root) = root {
                        let existing = scope.local(&root).and_then(|b| b.surviving);
                        let unioned = prov.union_surviving(existing, Some(set));
                        if let Some(b) = scope.bound.iter_mut().find(|b| b.name == root) {
                            b.surviving = unioned;
                        }
                    }
                }
                // P7b.S6d-PREREQ (REQ-4d site 5): the field store had no
                // provenance handling at all, and a shared `Slice[T]`
                // referent passes `@`/`!`'s `Copy` gate, so once REQ-5 admits
                // a slice field this is an unguarded write into a
                // reference-bearing aggregate. Placed here, in the store's
                // existing provenance block, because the machinery the two
                // rules need is already here -- the surviving-set escape
                // guard above is rule (i)'s structural twin, and the
                // root-binding join directly above it is rule (ii)'s.
                let bearing = ctx.with_extended_type_slices(|structs, enums| {
                    contains_reference(stack[vi].ty, structs, enums, arrays)
                });
                if bearing && ref_parts(stack[vi - 1].ty, refs).is_some() {
                    // (i) The receiver's root is not a local of this frame, so
                    // the container outlives the frame whose storage the
                    // stored view points into. The surviving-set guard above
                    // cannot see this: it is gated on the stored value having
                    // a surviving set, and a slice carries none. Ruling B's
                    // input ban exempts a top-level reference, so a non-inline
                    // word taking `&!Window` is exactly the reachable shape.
                    if !ref_root_is_in_frame(stack[vi - 1].deriv, prov, scope) {
                        return Err(stored_view_escapes_frame_error(
                            ctx,
                            span,
                            name,
                            stack[vi].ty,
                        ));
                    }
                    // (ii) In-frame: the stored value's provenance joins the
                    // receiver's root binding, so the container now tracks the
                    // borrow it holds. Ruling F applies here as much as at a
                    // construction -- storing a view of another place into an
                    // already-rooted container would leave the container's
                    // deriv naming a place it no longer views, which is worse
                    // than carrying none (site 2 would then propagate the
                    // *wrong* root on every read).
                    // The container is the place the receiver reference was
                    // taken from (`Deriv.place`, copied through every
                    // projection step), *not* `owned_root`: site 3's
                    // inheritance means `&!w`'s root already names the array
                    // the container views, which is the thing being compared
                    // against rather than the thing being updated.
                    let container = stack[vi - 1].deriv.map(|did| prov.deriv(did).place.clone());
                    if let Some(container) = container {
                        let held = scope.local(&container).and_then(|b| b.deriv);
                        let held_root = held.and_then(|id| prov.deriv(id).owned_root.clone());
                        let stored_root = stack[vi]
                            .deriv
                            .and_then(|id| prov.deriv(id).owned_root.clone());
                        if let (Some(h), Some(n)) = (&held_root, &stored_root) {
                            if h != n {
                                return Err(distinct_root_error(ctx, span, name, h, n));
                            }
                        }
                        let alias = match (
                            scope.local(&container).and_then(|b| b.aliases),
                            stack[vi].alias,
                        ) {
                            (Some(a), Some(b)) => Some(prov.alias_union(a, b.set)),
                            (Some(a), None) => Some(a),
                            (None, Some(b)) => Some(b.set),
                            (None, None) => None,
                        };
                        if let Some(b) = scope.bound.iter_mut().find(|b| b.name == container) {
                            b.deriv = stack[vi].deriv.or(b.deriv);
                            b.aliases = alias;
                        }
                    }
                }
            }
            if let Some(stack) =
                check_access_word(name, span, &mut stack, ctx, arrays, refs, scope, prov)?
            {
                return Ok(stack);
            }
            if let Some(stack) = (!gated)
                .then(|| check_shuffle(name, span, &mut stack, ctx, arrays, prov, scope, live, at))
                .transpose()?
                .flatten()
            {
                return Ok(stack);
            }
            // R12 (slice 8b, 8a): a bare operator resolves against the
            // overloads visible to the calling module, not the flat `env`
            // lookup that misses a per-module-mangled decl in a multi-module
            // build. `None` (single-module, or a unit test's `word_ctx` with
            // no module view) falls back to the flat lookup unchanged.
            // P8 S2 (R2): an unimported intrinsic gets no operator candidates
            // either. The operator dispatch is the intrinsics' own machinery --
            // a module-visible overload of `add` is an overload *of the
            // builtin*, and the compiler-injected `bool` `.` sits in the same
            // candidate set -- so leaving it populated would answer an
            // unimported `1 .` with that overload's own type mismatch instead
            // of the missing import.
            let scoped_ops = match gated {
                true => None,
                false => scoped_operator_overloads(ctx, env, name),
            };
            let op_candidates = match &scoped_ops {
                Some(v) => Some(&v[..]),
                None => env.get(name).map(|v| &v[..]),
            };
            let dispatch = match gated {
                true => OpDispatch::NotOperator,
                false => check_operator(name, span, &mut stack, ctx, op_candidates)?,
            };
            match dispatch {
                OpDispatch::Builtin(stack) => return Ok(stack),
                // Slice 8a phase 2 (R6/R7): a builtin operator name whose
                // operands match a user overload exactly dispatches to the
                // user word. Record the site so lowering emits an `Instr::Call`
                // here (R7), then fall through: the operands stay on the stack
                // and the ordinary `env` word-call path below performs the
                // dispatch (arity/type checks, move/borrow discipline, output
                // push) exactly as for any user word.
                OpDispatch::UserOverload(symbol) => {
                    poly.builtin_overloads.insert(span, symbol);
                }
                OpDispatch::NotOperator => {}
            }
            if let Some(stack) = (!gated)
                .then(|| check_tag_word(name, span, &mut stack, ctx))
                .transpose()?
                .flatten()
            {
                return Ok(stack);
            }
            if let Some(stack) = (!gated)
                .then(|| check_str_word(name, span, &mut stack, ctx))
                .transpose()?
                .flatten()
            {
                return Ok(stack);
            }
            let granted = releasable_into(
                scope,
                base_depth,
                outer_releasable,
                &siblings[at + 1..],
                live,
                at,
            );
            if let Some(stack) = (!gated)
                .then(|| {
                    check_array_word(
                        name, span, &mut stack, ctx, arrays, refs, slices, prov, env, cells, scope,
                        poly, &granted,
                    )
                })
                .transpose()?
                .flatten()
            {
                return Ok(stack);
            }
            if let Some(stack) = check_owned_cell_word(
                name, span, &mut stack, ctx, arrays, cells, prov, scope, live, at,
            )? {
                return Ok(stack);
            }
            // D3 (slice 8b): ahead of the ordinary env call path below, so it
            // catches a moving destructure (`S>`) of a drop-overloaded struct.
            check_destructure_drop_guard(name, span, ctx)?;
            // Phase 6 slice 3 (R3): a generated eliminator (`Shape?`) is
            // routed ahead of the env/combinator/poly paths. It has no body,
            // so it is not a `Combinator` and must never be spliced; and its
            // arms are matched to variants by annotation tag rather than by
            // slot position, so the ordinary poly-call unification it is
            // registered under is never what checks a call site.
            //
            // P7.S12 (R2.4): the registry now also keys a generic header no
            // instantiation has grounded, and this is the *concrete* consumer:
            // a concrete body's scrutinee is always some monomorph, so a
            // `Generic` entry means no monomorph is registered as reachable
            // here -- not that none exists. `eliminator_registry` is built
            // before the poly pre-pass mints monomorphs, so a header can have
            // a real instantiation elsewhere in the program that is simply
            // invisible to the registry at this point (a separate,
            // out-of-scope timing gap; see `concrete_body_generic_eliminator_error`).
            // Located and named rather than fallen through to the unknown-word
            // path or the adjacency message, both of which describe a
            // different mistake. `check_poly_combinator_standalone`'s i64
            // stand-in body reaches this too, and has no instantiator to
            // ground a scrutinee with even in principle.
            if let Some(target) = poly.eliminators.get(name).copied() {
                let enum_id = match target {
                    EliminatorTarget::Concrete(enum_id) => enum_id,
                    // P7.S11-follow (Part 3): the registry's `Generic` entry
                    // is frozen at pre-loop `eliminator_registry` build time
                    // and never re-consulted after a later check-time mint --
                    // but the scrutinee already on the stack may itself name a
                    // real, since-minted instantiation. Recover it from the
                    // live stack rather than erroring immediately.
                    EliminatorTarget::Generic { .. } => {
                        match scrutinee_enum_id_of_family(&stack, prov, refs, ctx) {
                            Some(id) => id,
                            None => {
                                return Err(concrete_body_generic_eliminator_error(ctx, span, name))
                            }
                        }
                    }
                };
                let granted = releasable_into(
                    scope,
                    base_depth,
                    outer_releasable,
                    &siblings[at + 1..],
                    live,
                    at,
                );
                return check_eliminator_call(
                    enum_id, name, span, stack, ctx, env, arrays, cells, refs, slices, prov, scope,
                    poly, &granted, tail,
                );
            }
            // R6-R9: a tail-position call, inside a self-tail combinator
            // body splice, to that same combinator is the loop back-edge, not
            // a re-splice (which would recurse forever). Intercepted before
            // the combinator dispatch below. It discharges the two
            // move/borrow obligations at the self-call (the stack-row identity
            // obligation is left to the ordinary stack-effect and `if`-join
            // discipline, R7), checks its arguments against the ground declared
            // inputs (R12), and produces the ground declared outputs (R11) --
            // then terminates this branch. A non-tail self-call never reaches
            // here: R4 rejected it at `check_combinator_cycles` before any
            // splice.
            let back_edge = tail
                && prov
                    .self_tail_combinator
                    .as_ref()
                    .is_some_and(|m| m.name == *name);
            if back_edge {
                let marker = prov
                    .self_tail_combinator
                    .as_ref()
                    .expect("back-edge marker set");
                let n = marker.input_count;
                // Cloned out so `stack`/`prov` stay mutably usable below (the
                // ground shape is small: a handful of `Type` and indices).
                let ground_inputs = marker.ground_inputs.clone();
                let ground_outputs = marker.ground_outputs.clone();
                let index_map = marker.index_map.clone();
                if stack.len() < n {
                    return Err(underflow_error(ctx, span, name, n, stack.len()));
                }
                let base = stack.len() - n;
                // R8: no linear value live across the edge (below the args, or
                // an unconsumed frame local). `base_depth` is this `if` arm's
                // entry depth; it is passed here and nowhere else (see
                // `check_linear_across_back_edge`).
                check_linear_across_back_edge(
                    ctx,
                    span,
                    name,
                    &stack[..base],
                    scope,
                    arrays,
                    Some(base_depth),
                )?;
                // R9: no reference into a frame local carried by the args.
                check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
                // R12: the self-call's arguments are checked against the ground
                // declared inputs. Rewriting the arm to produce the declared
                // outputs (below) removed the transitive check the `if`-join
                // used to get from the produced-inputs fiction, so this is made
                // explicit. Sound because the marker matches only in tail
                // position. A quotation-typed declared input matches any
                // quotation-carrying arg (its own `call` already checked the
                // body); everything else matches by type.
                for (i, want) in ground_inputs.iter().enumerate() {
                    let found = stack[base + i];
                    if crate::ast::is_quotation_type(*want).is_some() {
                        if found.quot.is_none() && crate::ast::is_quotation_type(found.ty).is_none()
                        {
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
                // R11: the arm produces the ground declared outputs, not the
                // non-quotation inputs (right only for `while`'s state-threading
                // shape, false for a loop that consumes its counters). The
                // carried non-quotation inputs feed provenance forwarding along
                // the index map (phase 6, R14); a quotation arg carries no
                // loop-phi state and is dropped.
                let carried: Vec<Slot> = stack[base..]
                    .iter()
                    .copied()
                    .filter(|s| s.quot.is_none() && crate::ast::is_quotation_type(s.ty).is_none())
                    .collect();
                let outs = back_edge_outs(&ground_outputs, &index_map, &carried);
                stack.truncate(base);
                stack.extend(outs);
                return Ok(stack);
            }
            // R18: a call to a quotation-taking word is inlined (term-splice)
            // rather than looked up in `env` and lowered to a call: it mints
            // no `IrFunc` (R20). One name can carry several candidates
            // exactly as an ordinary overloaded word can (R1); a single one
            // resolves exactly as before, a set resolves against the live
            // stack. Copy the chosen `Combinator` out of the borrowed map
            // first (it is two pointers) so `poly` can be reborrowed mutably
            // for the splice.
            // Slice 10c: a name can now be *both* a polymorphic library word
            // and a concrete user overload -- `core::cmp`'s `'T: Copy Ord`
            // comparisons against a user's `: lt ( Vec2 Vec2 -- bool )` for
            // their own type, which slice 8a shipped. The library candidate's
            // `Ord` bound excludes `Vec2`, so when no polymorphic candidate
            // admits these operands the call falls through to the concrete
            // lookup, exactly as a builtin row's exact miss fell through to a
            // user overload (8a R2). Computed once and consulted by both the
            // combinator interception and the poly-call one below, since a
            // poly combinator sits in both tables.
            let fall_through_to_env = env.contains_key(name)
                && poly.env.get(name).is_some_and(|cands| {
                    !cands.iter().any(|sig| {
                        poly_sig_could_match(
                            sig,
                            &stack,
                            name,
                            span,
                            ctx,
                            arrays,
                            cells,
                            refs,
                            poly.trait_resolve.impls,
                            poly.trait_resolve.traits,
                        )
                    })
                });
            if let Some(candidates) = poly.combinators.get(name) {
                let chosen = match candidates.as_slice() {
                    [only] if !fall_through_to_env => Some(*only),
                    _ => resolve_combinator_overload(
                        candidates, &stack, span, ctx, arrays, cells, refs,
                    ),
                };
                match chosen {
                    Some(chosen) => {
                        let granted = releasable_into(
                            scope,
                            base_depth,
                            outer_releasable,
                            &siblings[at + 1..],
                            live,
                            at,
                        );
                        return inline_combinator(
                            &chosen, span, stack, ctx, env, arrays, cells, refs, slices, prov,
                            scope, poly, &granted, tail, None, false,
                        );
                    }
                    None if fall_through_to_env => {}
                    None => {
                        return Err(no_combinator_overload_matches_error(
                            ctx, span, name, candidates,
                        ))
                    }
                }
            }
            // R5/R14: a call to a polymorphic word is intercepted before the
            // concrete `env` lookup and unified against the concrete stack;
            // its `Sig` is per-instantiation, not name-keyed.
            //
            // P7.S3t (R3): the one route that consumes an explicit
            // type-argument list, which the allow-guard at the top of this arm
            // has already established is where a non-empty one may arrive.
            if poly.env.contains_key(name) && !fall_through_to_env {
                return check_poly_call(
                    name, span, type_args, len_args, None, &mut stack, ctx, env, scope, arrays,
                    cells, refs, slices, prov, live, at, poly,
                );
            }
            // P7.S3o Phase 3: a bare trait member call (like `cmp` directly)
            // inside a spliced combinator body resolves against the concrete θ
            // at the splice site, where the bound's `impl:` is knowable. This
            // is the splice-path dispatch injection: `check_terms_relaxed`
            // currently has zero bound-dispatch calls, so without this a bare
            // member falls through to `env.get` as an unknown word. The
            // `combinator_sig`/`combinator_subst` on `PolyCtx` are set by
            // `inline_combinator` (real splice) and
            // `check_poly_combinator_standalone` (i64 stand-in); when unset,
            // `resolve_splice_member_call` returns `Ok(None)` and ordinary
            // dispatch proceeds unchanged.
            // P7b.S3 (S3-1.c): `granted`/`tail` are threaded so an `inline`
            // member routes onto `inline_combinator` -- computed the same way
            // the `poly.combinators` hit above computes them.
            let member_granted = releasable_into(
                scope,
                base_depth,
                outer_releasable,
                &siblings[at + 1..],
                live,
                at,
            );
            if let Some(stack) = resolve_splice_member_call(
                name,
                span,
                &mut stack,
                ctx,
                env,
                scope,
                arrays,
                cells,
                refs,
                slices,
                poly,
                prov,
                live,
                at,
                &member_granted,
                tail,
            )? {
                return Ok(stack);
            }
            // R1/R2: one name can carry several candidates. A single one is
            // the ordinary case and resolves by name at lowering exactly as
            // before; an overload set resolves by exact operand match here and
            // records the chosen candidate's symbol, so lowering calls that
            // definition rather than whichever body the name alone would find.
            //
            // R12 (slice 8b, 8a): a bare operator whose `check_operator` arm
            // returned `UserOverload` falls through to here to reuse the
            // move/borrow discipline, but its decl is mangled per module in a
            // multi-module build, so the flat `env.get(name)` misses it.
            // `scoped_ops` (computed once above) is `Some` exactly for an
            // operator name under module scoping and carries the caller-visible
            // overloads; every other name still resolves through `env`.
            let fallback_storage;
            let mut from_fallback = false;
            let candidates = match &scoped_ops {
                Some(v) => v.as_slice(),
                None => match env.get(name) {
                    Some(v) => {
                        // P7b.S6d (REQ-4, the pre-existing S8b-class clobber,
                        // G7): the env-hit arm used to return the flushed
                        // parse-time candidates alone. A check-time monomorph
                        // of the SAME generated enum -- minted into the live
                        // cell after `env` was assembled (a bare trait
                        // member's mono dispatch minting `Step[i64 List[i64]]`
                        // beside a parse-time `Step[i64 Slice[i64]]`
                        // signature mention, `probes/s6d_m_clobber_touch.sth`)
                        // -- was invisible here, so the single flushed
                        // candidate resolved every bare-name site and the
                        // sig check re-typed the site to the wrong
                        // monomorph. Union the flushed candidates with the
                        // live check-time mints, keyed per-monomorph: each
                        // candidate carries its own mangled symbol, and the
                        // two sources are disjoint (the mint keys dedupe, so
                        // one monomorph never registers twice, and the
                        // pending tail is exactly the not-yet-flushed rest),
                        // so the multi-candidate dispatch below
                        // operand-filters and the span-keyed record pins
                        // each bare-name site to its own. Provenance: the
                        // mints' `.module` is unverified
                        // (`select_overload_fallback_sourced`'s doc), so a
                        // union containing any mint dispatches under the
                        // fallback-sourced policy; with nothing pending this
                        // arm is byte-identical to the plain env hit.
                        let pending = ctx.generics().is_some_and(|g| {
                            let g = g.borrow();
                            !g.inst_structs.is_empty() || !g.inst_enums.is_empty()
                        });
                        if !pending {
                            v.as_slice()
                        } else {
                            let mints = mint_fallback_candidates(name, ctx);
                            if mints.is_empty() {
                                v.as_slice()
                            } else {
                                from_fallback = true;
                                fallback_storage =
                                    v.iter().cloned().chain(mints).collect::<Vec<Overload>>();
                                fallback_storage.as_slice()
                            }
                        }
                    }
                    // P7.S11-follow (Part 4): an ordinary `env` miss may still
                    // name the generated constructor/accessor of a check-time
                    // monomorph, minted (by Part 1's splice-site grounding or
                    // an ordinary mid-word poly call's own `apply_subst`)
                    // into the live `generics_cell` after `env` was built and
                    // so invisible to it -- re-derive it here, read-through,
                    // mutating nothing.
                    None => {
                        let mints = mint_fallback_candidates(name, ctx);
                        from_fallback = true;
                        if mints.is_empty() {
                            // P7b.S2 (S2-16, mono caller): the bare member
                            // lookup, after the check-time monomorph mints
                            // (above, which take precedence) and before the
                            // unchanged unknown-word/intrinsic fallthrough
                            // below, which still fires on no-match so every
                            // existing unknown-word golden holds. The member
                            // word is module-qualified, so the bare name has
                            // no `env` entry; the lookup keys on the member
                            // name plus the operand's dispatchable type
                            // through the whole-program trait/impl tables.
                            if let Some(next) = resolve_mono_member_call(
                                name, span, type_args, len_args, &mut stack, ctx, env, scope,
                                arrays, cells, refs, slices, prov, live, at, poly,
                            )? {
                                return Ok(next);
                            }
                            // P7b.S11 (R-2/R-7; strict amendment 260910): the
                            // zero-candidate arm splits. A name with no
                            // generic header of this module's own declines
                            // below to the unchanged unknown-word/intrinsic
                            // fallthrough, so every existing golden holds; a
                            // bare constructor of one grounds from the call
                            // site alone -- a θ the consumer or the operands
                            // fully determine succeeds here with no monomorph
                            // in scope at all (G5), and an undetermined
                            // parameter is reported as itself rather than as
                            // an unknown word (G1). Scope is never consulted
                            // to fill a parameter (strict amendment).
                            match ground_bare_generic_ctor(
                                CtorCallSite {
                                    name,
                                    span,
                                    candidates: &[],
                                    type_args,
                                    len_args,
                                    stack: &stack,
                                    siblings,
                                    at,
                                    tail,
                                },
                                ctx,
                                env,
                                scope,
                                poly,
                                arrays,
                                cells,
                                refs,
                            )? {
                                Some(g) => {
                                    fallback_storage = vec![g];
                                    fallback_storage.as_slice()
                                }
                                None => {
                                    return Err(match gated {
                                        // P8 S2 (R6a): the name is a real
                                        // intrinsic that nothing else claimed,
                                        // so the remedy is the import, not a
                                        // definition.
                                        true => ungated_intrinsic_error(ctx, span, name),
                                        false => unknown_word_error(ctx, span, name),
                                    });
                                }
                            }
                        } else {
                            fallback_storage = mints;
                            fallback_storage.as_slice()
                        }
                    }
                },
            };
            // P7b.S9 Phase 2 (R1.1a): the single-candidate case may be
            // another module's eager mint of a same-named-but-distinct
            // generic header (Phase-1 verdict, slice9-probes.md -- a's own
            // instantiation is simply never minted, so there is never a
            // second candidate here to filter). Ground at the caller's own
            // header before taking the single-candidate arm, never silently
            // substituting the borrowed mint.
            let ground_storage;
            let candidates: &[Overload] = if let [only] = candidates {
                match bare_generated_word_own_module_grounding(
                    only, name, span, ctx, arrays, cells, refs,
                )? {
                    Some(g) => {
                        ground_storage = g;
                        std::slice::from_ref(&ground_storage)
                    }
                    None => candidates,
                }
            } else {
                candidates
            };
            // P7b.S11 (R-1; strict amendment 260910): a bare generic
            // constructor grounds from its own call-site information alone --
            // explicit args, its consumer's declared signature, its operand
            // literals (R-2's order). A fully bound θ names the monomorph
            // outright; any parameter those three inputs leave undetermined is
            // the located unbound-parameter error. Module scope is never
            // consulted to fill, disambiguate, or veto a parameter. Declining
            // (`None`) happens only where this name has no groundable
            // own-module ctor header, or the category fence preserves a
            // same-named non-ctor candidate -- leaving this arm's selection
            // (the S8b span-keyed pin and the S3 splice redirect below among
            // it) byte-for-byte as it was.
            let s11_storage;
            let mut s11_grounded = false;
            let candidates: &[Overload] = match ground_bare_generic_ctor(
                CtorCallSite {
                    name,
                    span,
                    candidates,
                    type_args,
                    len_args,
                    stack: &stack,
                    siblings,
                    at,
                    tail,
                },
                ctx,
                env,
                scope,
                poly,
                arrays,
                cells,
                refs,
            )? {
                Some(g) => {
                    s11_grounded = true;
                    s11_storage = g;
                    std::slice::from_ref(&s11_storage)
                }
                None => candidates,
            };
            let chosen = match candidates {
                [only] => {
                    // P7b.S3 (S3-1.e): inside a combinator splice, a generated
                    // enum word's resolution is recorded per `(uid, span)` --
                    // the span-keyed `builtin_overloads` (and the bare-key
                    // family lookup that serves an unrecorded site) are
                    // last-write-wins across splices at two θ. A redirect,
                    // mirroring `check_poly_call`'s `splice_records` one; the
                    // non-enum records below stay untouched even in a splice.
                    if let Some((uid, id)) = splice_enum_site(name, only, ctx, prov) {
                        poly.splice_enum_words.insert((uid, span), id);
                    } else if poly.combinators.contains_key(name) {
                        // Slice 10c: reaching here on a name that is *also* an
                        // always-spliced word means the fall-through above
                        // fired (no polymorphic candidate admits these
                        // operands). Record the site so lowering emits a real
                        // call to this word; without the record `lower_call`
                        // still finds the name in its combinator env and
                        // splices the library definition, so the checker and
                        // the backend would disagree about which `lt` a
                        // `Vec2 Vec2 lt` site means.
                        poly.builtin_overloads.insert(span, only.symbol.clone());
                    } else if s11_grounded || is_generated_enum_word(name, only, ctx) {
                        // P7b.S11 Phase 1 (R-1): an S11-grounded site's
                        // resolution is a function of the *call site*, not of
                        // the name -- and grounding collapses a site the
                        // multi-candidate arm below used to resolve (and
                        // record) down to a single candidate. Recording here
                        // keeps that arm's span-keyed record, without which
                        // lowering's bare-key map (last-write-wins across
                        // instantiations, `src/ir/layout.rs`) re-types the
                        // site to whichever monomorph was registered last.
                        //
                        // P7b.S8b Phase 1 (R5): a generated enum word chosen
                        // by bare name while only one instantiation of its
                        // header existed. Lowering's own map keys every
                        // variant under both the mangled and the bare
                        // spelling, and the bare one is last-write-wins
                        // across instantiations (`src/ir/layout.rs`), so a
                        // *later* mint -- one the frozen `env` never saw,
                        // grounded mid-check -- silently re-typed this site's
                        // field shapes. Recording the resolved mangled symbol
                        // pins the instantiation the checker actually chose,
                        // the same span-keyed channel the multi-candidate arm
                        // below already uses; lowering needs no change.
                        poly.builtin_overloads.insert(span, only.symbol.clone());
                    }
                    only
                }
                _ => {
                    let operands: Vec<Type> = stack.iter().map(|s| s.ty).collect();
                    // P7b.S5 (R3.5): the caller's *lexically-declaring*
                    // module, never `ctx.module()` -- inside an inline
                    // combinator splice, `ctx.module()` is re-scoped to the
                    // callee's module (`combinators.rs`'s `with_module`),
                    // which would make tier 1 prefer the splice target's
                    // module and reintroduce the silent cross-pick this
                    // policy exists to kill.
                    let caller_module = span.module;
                    // P7b.S5 (R4/Fix D): `mint_fallback_candidates`'s
                    // `Overload.module` is not reliably the declaring module
                    // (see `select_overload_fallback_sourced`'s doc comment),
                    // so that provenance gets tier 1 only, never tiers 2/3.
                    let pick = if from_fallback {
                        select_overload_fallback_sourced(candidates, &operands, caller_module)
                    } else {
                        match ctx.modules() {
                            Some(modules) => {
                                // R3.5's `caller_visible` predicate reads
                                // `modules[caller].selective`, which is keyed
                                // by the *surface* (bare) name -- a
                                // module-scoped operator's `name` here may
                                // already be mangled per-module
                                // (`resolve::mangle`, the comment above this
                                // arm), so demangle before the visibility
                                // lookup or every mangled candidate reads as
                                // invisible.
                                let bare_name = crate::resolve::demangle_call(name);
                                select_overload(candidates, &operands, caller_module, |m| {
                                    is_name_visible_to_module(modules, caller_module, m, &bare_name)
                                })
                            }
                            // R3.7: no import-closure data to consult, so
                            // tier 2 degenerates to "exactly one in matching".
                            None => select_overload(candidates, &operands, caller_module, |_| true),
                        }
                    };
                    let chosen = match pick {
                        OverloadPick::Pick(hit) => hit,
                        OverloadPick::Ambiguous => {
                            return Err(no_overload_matches_error(ctx, span, name, candidates))
                        }
                    };
                    // S3-1.e: the same redirect as the single-candidate arm.
                    match splice_enum_site(name, chosen, ctx, prov) {
                        Some((uid, id)) => {
                            poly.splice_enum_words.insert((uid, span), id);
                        }
                        None => {
                            poly.builtin_overloads.insert(span, chosen.symbol.clone());
                        }
                    }
                    chosen
                }
            };
            let sig = &chosen.sig;
            let n = sig.inputs.len();
            if stack.len() < n {
                return Err(underflow_error(ctx, span, name, n, stack.len()));
            }
            let base = stack.len() - n;
            for (i, want) in sig.inputs.iter().enumerate() {
                let found = stack[base + i];
                // R8 (D4): a declared `Type::Quotation` parameter is a
                // materialization boundary. This is the site a struct
                // *constructor* call (`[ 1 add ] Holder`) and a generated setter
                // reach; a `Known` literal is materialized (validated here,
                // lowered to a `(code, env)` value), a capturing one run through
                // the R15 admission rule (a parameter is an in-frame boundary).
                // Gated strictly on
                // `want`'s type, so it covers a constructor, a setter, and an
                // ordinary user word declaring a quotation parameter alike; an
                // `extern` never reaches here (its declared effect cannot name
                // a `Type::Quotation`, rejected at declaration).
                // P7.S3h: an `owning` parameter is the same boundary with the
                // owning env. It is reachable only on this real-call route:
                // a spliced or generic word declaring one is rejected at its
                // declaration, because neither route materializes.
                let declared = match *want {
                    Type::Quotation(eff) => Some((eff, false)),
                    Type::OwningQuotation(eff) => Some((eff, true)),
                    _ => None,
                };
                if let Some((eff, owning)) = declared {
                    if let Some(QuotRef::Known(id)) = found.quot {
                        stack[base + i] = materialize_quotation_at_boundary(
                            id, eff, owning, false, name, span, ctx, env, arrays, cells, refs,
                            slices, prov, scope, poly,
                        )?;
                        continue;
                    }
                    // An already-erased runtime quotation value falls through
                    // to the ordinary `match_slot` (Exact) below.
                }
                // R9: a quotation argument rejects before ordinary unification,
                // so the message names the word rather than mismatching the
                // `Cstr` placeholder. Also covers generated struct
                // constructors/setters and `extern` args (all `env` words).
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
                        return Err(type_mismatch_error(ctx, span, name, *want, found.ty));
                    }
                }
            }
            // Slice 10c (review fix, Phase 1): `tail` alone is the syntactic
            // position, which `TailWalk` can see further into than lowering's
            // own splice ever will (a mid-body local forwarding a literal
            // through a combinator, R-P1-3). `ctx.is_self_tail_call()` is the
            // same `has_self_tail_call` predicate lowering consults to decide
            // whether *this word* actually gets the loop shape, so gating on
            // both together means this guard fires exactly where lowering
            // back-edges, never on a call that lowers as ordinary recursion.
            if tail && ctx.mangled_name() == name.as_str() && ctx.is_self_tail_call() {
                check_linear_across_back_edge(
                    ctx,
                    span,
                    name,
                    &stack[..base],
                    scope,
                    arrays,
                    None,
                )?;
                check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
            }
            // R19/R22: a struct/enum constructor consuming an erased closure
            // becomes its carrier -- the surviving capture set rides onto the
            // aggregate output so the captures stay live (R20) and the
            // word-output escape guard (R22) can see a frame capture leaving
            // through the carrier. The union is `None` for the overwhelming
            // majority of calls (no closure argument), a no-op there.
            //
            // Review fix: this same generic dispatch also handles a
            // destructure with a `Quotation`-typed field (not `is_aggregate`)
            // -- e.g. `Holder>` on `type: Holder q [ i64 -- i64 ] ;`. A
            // quotation-typed output legitimately carries the closure onward
            // exactly as an aggregate output does, so it forwards too.
            // `OwningQuotation` forwards for the same reason: an owning
            // output carries the closure onward exactly as a plain quotation
            // or an aggregate output does, so dropping the set here would
            // blind R22 to a frame capture leaving via that output. P7.S3v
            // (R6) made an aggregate a legal carrier too, so this is no
            // longer a value's *only* carrier -- over-forwarding the set is
            // harmless, under-forwarding is not.
            // Review fix (P7 slice 1): an ordinary word call consumes its
            // operands just as `drop` does, so a struct operand a live
            // projection still reaches (the owned-receiver projection arm's
            // `Slot.alias`, which carries no `Deriv` to be caught by the
            // named-place consume checks) cannot be moved into the call out
            // from under that reference. The stranded reference is as often
            // another operand of this same call (`mk &data &^ eat`) as a value
            // left below it, so the scan covers the whole stack bar the
            // operand being consumed itself.
            for i in base..stack.len() {
                let origin = consumed_place_conflict(
                    stack[i],
                    &stack[..i],
                    ctx,
                    arrays,
                    scope,
                    prov,
                    live,
                    at,
                )
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
            let carried = (base..stack.len())
                .fold(None, |acc, i| prov.union_surviving(acc, stack[i].surviving));
            // P7b.S6d-PREREQ (REQ-4d site 1): a struct/enum constructor is a
            // generated `env` word, so a construction packing a slice into an
            // aggregate is an ordinary word call and its output push is this
            // one. `surviving` was already folded across the operands just
            // above; the borrow provenance is folded the same way, and is what
            // keeps the constructed value visible to the exclusivity scans.
            let (carried_deriv, carried_alias) =
                carried_borrow(ctx, span, name, &stack[base..], &sig.outputs, arrays, prov)?;
            // R3: detect a nullary variant constructor to set `variant_idx`
            // on the output slot, so `fill`'s element gate can admit a linear
            // nullary-variant seed (a nullary variant has no payload to
            // replicate). `chosen.symbol` is the variant's own `name` from
            // `enum_generated_sigs`, never mangled, so it matches the enum
            // declaration's `variant.name` directly.
            let nullary_variant_idx = if sig.inputs.is_empty() && sig.outputs.len() == 1 {
                if let Type::Enum(id, _) = sig.outputs[0] {
                    ctx.enums()[id.index()]
                        .variants
                        .iter()
                        .position(|v| v.fields.is_empty() && v.name == chosen.symbol)
                        .map(|vi| vi as u32)
                } else {
                    None
                }
            } else {
                None
            };
            stack.truncate(base);
            for ty in &sig.outputs {
                let surviving = if carried.is_some()
                    && (ty.is_aggregate()
                        || matches!(ty, Type::Quotation(_) | Type::OwningQuotation(_)))
                {
                    carried
                } else {
                    None
                };
                // Only a reference-bearing output can carry a borrow onward,
                // so a plain `i64`/`Point` output keeps `Slot::computed`'s
                // empty provenance even when an operand had some: forwarding
                // there would root a value that views nothing.
                let bearing = ctx.with_extended_type_slices(|structs, enums| {
                    contains_reference(*ty, structs, enums, arrays)
                });
                stack.push(Slot {
                    surviving,
                    variant_idx: nullary_variant_idx,
                    deriv: bearing.then_some(carried_deriv).flatten(),
                    alias: bearing.then_some(carried_alias).flatten(),
                    ..Slot::computed(*ty)
                });
            }
            Ok(stack)
        }
        // R5: a quotation literal interns its body into the side table and
        // pushes a compile-time-only marker (D1/D2). The body is *not* checked
        // here (D3): a bare body's input row is unknown until its consumption
        // site (`call`) -- unless the literal declares one, which is exactly
        // what Phase 6 slice 1's annotation supplies. The placeholder `ty` is
        // `Cstr`, a registry-free scalar no user op accepts once R11's
        // default-deny is in place (R4).
        TermKind::Quotation(body, is_inline, annot) => {
            // Phase 6 slice 1 (R1/R3): an annotated literal is checked against
            // its own annotation right here, where it is written, so a
            // body/annotation disagreement is an error independent of whether
            // the literal ever fills a parameter.
            let annot = match annot {
                Some(annot) => {
                    let resolved = resolve_annotation(ctx, annot)?;
                    // Phase 6 slice 3 (R1): an eliminator arm's annotation
                    // elides both rows (`( Circle )` is
                    // `( ..a Shape.Circle -- ..b )`), so it has no standalone
                    // fixed point to run the body against -- the caller region
                    // it reaches into and the shape it leaves are both
                    // supplied by the eliminator call site, where
                    // `check_eliminator_call` runs the same directional check
                    // against the real stack.
                    match &resolved.variant_tag {
                        None => check_literal_against_annotation(
                            &resolved, body, *is_inline, ctx, env, arrays, cells, refs, slices,
                            prov, scope, poly,
                        )?,
                        // Review fix (Phase 6 slice 3, finding 3): the arm
                        // above skips the standalone annotation check entirely
                        // for *every* tagged literal, on the premise that
                        // `check_eliminator_call` checks it instead -- true
                        // only when this literal is actually collected as an
                        // arm. A tagged literal that never reaches an
                        // eliminator call (a typo'd call name, or a tagged
                        // literal used as an ordinary value) was silently
                        // never checked at all, magic that this rejects.
                        Some(tag) => match tagged_literal_reaches_an_eliminator_call(
                            siblings,
                            at,
                            poly.eliminators,
                        ) {
                            EliminatorArmDest::Reached => {}
                            EliminatorArmDest::NotAdjacent => {
                                return Err(eliminator_arm_outside_call_error(
                                    ctx,
                                    resolved.span,
                                    &tag.name,
                                ));
                            }
                            // P7.S12 (R7.2): the call this literal is adjacent
                            // to names no eliminator at all -- a typo'd call
                            // name, most likely -- which is not an adjacency
                            // mistake and must not be told it is one.
                            EliminatorArmDest::NamesNoEliminator(call) => {
                                return Err(eliminator_arm_names_no_eliminator_error(
                                    ctx,
                                    resolved.span,
                                    &tag.name,
                                    &call,
                                ));
                            }
                        },
                    }
                    Some(resolved)
                }
                None => None,
            };
            let id = QuotId(prov.quotations.len());
            prov.quotations.push(QuotBody {
                body: body.clone(),
                span,
                is_inline: *is_inline,
                annot,
            });
            prov.quotation_captures.push(capture_names(body));
            stack.push(Slot {
                quot: Some(QuotRef::Known(id)),
                ..Slot::computed(Type::Cstr)
            });
            Ok(stack)
        }
    }
}

/// P7.S3t (R3): whether a call of `name` reaches the polymorphic-call
/// interception in `check_term`'s `Call` arm, the sole route that consumes an
/// explicit type-argument list. Every other route -- a local read, `call` and
/// `branch`, a reference/access/shuffle/tag/str/array/cell builtin, a cast, an
/// eliminator, a combinator splice or its self-tail back edge, a concrete `env`
/// word -- is denied, and its call site reports `no_type_arguments_error`.
///
/// The four exclusions before the `poly.env` lookup are the routes that would
/// otherwise claim a name the polymorphic env *also* holds. Two are witnessed
/// by a mutation test that flips a build to exit 0: an eliminator
/// (`Shape?[f64]`) and a combinator (`lt[i64]`) can each share a name with a
/// `poly.env` entry, so their exclusions are load-bearing. The other two are
/// not: `dup`/`x`-shaped names never enter `poly.env` at all, so the
/// name-dispatched-builtin exclusion is redundant with that lookup, and the
/// local-read exclusion is unreachable -- a local named after a poly word is
/// already rejected at its binding site (`callable_local_error`). Both stay
/// because the routes they describe are real (a builtin and a local *do* run
/// ahead of the poly interception in `check_term`'s `Call` arm), even though
/// nothing here currently depends on either clause to reject a program. The
/// four together also cover the operator route, which is the one that runs
/// *between* them and the poly interception: every builtin operator name is a
/// name-dispatched builtin except the six comparisons, and those are
/// always-spliced `lib/` combinators, so the combinator clause has already
/// denied them. The final clause is `fall_through_to_env`'s negation: a name
/// with both polymorphic and concrete candidates is a polymorphic call only
/// while some polymorphic candidate could admit these operands.
#[allow(clippy::too_many_arguments)]
fn poly_call_takes_type_args(
    name: &str,
    span: Span,
    stack: &[Slot],
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    scope: &Scope,
    poly: &PolyCtx,
    impls: &[crate::ast::ImplDecl],
    traits: &[crate::ast::TraitDecl],
) -> bool {
    scope.local(name).is_none()
        && !crate::ast::is_name_dispatched_builtin(name)
        && !poly.eliminators.contains_key(name)
        && !poly.combinators.contains_key(name)
        && (poly.env.get(name).is_some_and(|candidates| {
            !env.contains_key(name)
                || candidates.iter().any(|sig| {
                    poly_sig_could_match(
                        sig, stack, name, span, ctx, arrays, cells, refs, impls, traits,
                    )
                })
        }) || (!env.contains_key(name)
            && poly.trait_resolve.traits.iter().any(|t| {
                // P7b.S2 (S2-16, mono caller): a trait member name takes the
                // explicit-instantiation spelling too -- the mono member path
                // (env-miss branch) routes it to the member word's `check_poly
                // _call`, whose θ seeding is the pinned remedy for a variable
                // only a quotation's rows mention (`map`'s `'U`). The clause
                // only widens the allow for names no earlier route claims
                // (members are never locals/builtins/eliminators, and the
                // six comparison names are combinators, excluded above) --
                // and a plain-`env` word of the same name IS such a claim:
                // the env route wins the bare call and cannot read an
                // argument list, so the pre-widening rejection stands for it
                // (final-review fix; the clause used to admit the colliding
                // spelling and the env call silently dropped the list).
                t.members.iter().any(|m| m.name == name)
            }))
            // P7b.S11 Phase 2 (R-6): the third admitted category -- a bare
            // generic ctor/destructure name paired with a matching header.
            // Full-arity validation is not this predicate's job (it has no
            // access to the argument list, only to whether the category
            // exists at all, and no knowledge of which route a name that is
            // *also* a poly word or trait member will take): an admitted
            // list that no earlier route consumed reaches
            // `ground_bare_generic_ctor`, which either consumes it as R-2's
            // first input or rejects it -- so a wrong-arity list is a
            // located error rather than a silently dropped one.
            || explicit_args_ctor_header(name, ctx).is_some())
}

/// P7.S12 (R7.1): the three outcomes of scanning forward from a tagged
/// literal for the eliminator call it is meant to feed. `Reached` is the
/// success case; the other two used to collapse into one `bool` `false`, and
/// the whole point of splitting them is that they are different mistakes.
/// `NotAdjacent` is a genuine written-adjacency problem -- the run of tagged
/// literals is not immediately followed by any call at all (an intervening
/// term, or running off the end of the body) -- and keeps the existing
/// adjacency message. `NamesNoEliminator` is a call that *is* right there,
/// adjacent as written, but names nothing the eliminator registry holds (a
/// typo'd `Optionn?`, most likely): telling that a story about adjacency
/// would be wrong, since the arms are exactly where they should be.
pub(super) enum EliminatorArmDest {
    Reached,
    NotAdjacent,
    NamesNoEliminator(String),
}

/// Phase 6 slice 3 review fix (finding 3): which of `EliminatorArmDest` the
/// tagged literal at `siblings[at]` reaches. Forward-scans past every
/// immediately-following tagged quotation literal and reads the call that
/// ends the run, if any.
///
/// This is *written adjacency*, deliberately stricter than
/// `check_eliminator_call`'s own stack-based collection: a stack-neutral term
/// written between two arms (`~[ ( Circle ) .. ] 4 drop ~[ ( Rect ) .. ]
/// Shape?`) leaves the arms adjacent on the stack but not in the source, and
/// is rejected here. Deciding the stack-level question syntactically is not
/// possible, and the looser rule that would admit it (scan forward past
/// anything until *some* eliminator call) re-opens the hole this exists to
/// close: it would accept a tagged literal that is dropped, never checked,
/// merely because an unrelated eliminator call follows it later in the body.
pub(super) fn tagged_literal_reaches_an_eliminator_call(
    siblings: &[Term],
    at: usize,
    eliminators: &HashMap<String, EliminatorTarget>,
) -> EliminatorArmDest {
    let mut j = at + 1;
    while let Some(term) = siblings.get(j) {
        match &term.kind {
            TermKind::Quotation(_, _, Some(annot)) if annot.variant_tag.is_some() => {
                j += 1;
            }
            TermKind::Call(name, _, _) => {
                return if eliminators.contains_key(name) {
                    EliminatorArmDest::Reached
                } else if name.ends_with('?') {
                    // P7.S12 (R7.2): `?` is the generated-eliminator naming
                    // convention (`eliminator_registry`'s own key shape,
                    // `{surface}?`) -- an ordinary call never ends in it (R8.7's
                    // own fixtures, `drop`/`swap`, are the negative witnesses),
                    // so a `?`-suffixed call absent from the registry reads as
                    // a typo'd eliminator name, not a written-adjacency
                    // mistake.
                    EliminatorArmDest::NamesNoEliminator(name.clone())
                } else {
                    EliminatorArmDest::NotAdjacent
                };
            }
            _ => return EliminatorArmDest::NotAdjacent,
        }
    }
    EliminatorArmDest::NotAdjacent
}

/// Phase 6 slice 3 review fix (finding 3): a variant-tagged quotation literal
/// that is not consumed as an eliminator arm. The tag is only meaningful as
/// arm-to-variant routing (R1/R4); anywhere else it is a stray annotation this
/// checker must not silently let through unchecked.
pub(super) fn eliminator_arm_outside_call_error(ctx: &Ctx, span: Span, tag: &str) -> String {
    format!(
        "error: this quotation is annotated `( {tag} )`, an eliminator-arm tag, but it is not consumed by a call to a generated eliminator{} (line {})\n  arms are written together, immediately before the call: `~[ ( A ) .. ] ~[ ( B ) .. ] Enum?`",
        in_word(ctx),
        span.line,
    )
}

/// P7.S12 (R7.2): a variant-tagged quotation literal written immediately
/// before a call, exactly where an eliminator arm belongs -- but that call
/// names no eliminator at all, neither a monomorph nor a generic header, so
/// there is nothing for these arms to be arms *of*. Its own message: the
/// adjacency text (`eliminator_arm_outside_call_error`) would tell the
/// caller to fix something that is already right.
pub(super) fn eliminator_arm_names_no_eliminator_error(
    ctx: &Ctx,
    span: Span,
    tag: &str,
    call: &str,
) -> String {
    let call = crate::resolve::demangle_call(call);
    format!(
        "error: this quotation is annotated `( {tag} )`, an eliminator-arm tag, but the call `{call}` it is adjacent to names no eliminator in scope{} (line {})",
        in_word(ctx),
        span.line,
    )
}

/// P7b.S9 Phase 2 (R1.1a): a bare generated struct word's single `env`
/// candidate may be another module's eager mint of a same-named-but-distinct
/// generic header -- see the Phase-1 verdict (`slice9-probes.md`): `a`'s own
/// instantiation is never minted at all when only `b` spells the concrete
/// type explicitly, so `env` never holds more than the one, borrowed,
/// candidate to filter. When the caller's own module declares its own generic
/// struct header under that name, distinct from the candidate's owning
/// header, mint (or find) the caller's own instantiation at the same concrete
/// arguments and hand back *its* generated word instead of the borrowed one.
/// `Ok(None)` when there is nothing to ground against (an ordinary concrete
/// struct, a word that is not one of this struct's generated pair, or a
/// caller with no header of its own under this name -- Phase 4's D3
/// territory) -- the existing candidate is used unchanged.
///
/// Both members of the pair ground, not just the constructor: the caller's
/// own mint is minted mid-word and so has no `env` entry of its own (`env` is
/// built before check, and `mint_fallback_candidates` fires only on an `env`
/// *miss*), so a destructure applied to a caller-grounded operand would
/// otherwise select the borrowed module's generated word and fail with a
/// self-contradictory `expected Widget[i64], found Widget[i64]` -- two live
/// decls sharing one surface name.
///
/// The whole grounded `Overload` -- `Sig`, lowering symbol and module -- is
/// re-derived from the caller's *own* minted decl through
/// `struct_generated_sigs_of`, the same rule env registration uses. Keeping
/// any part of the borrowed candidate would mix provenances: its field list
/// on a constructor whose output is the caller's mint silently consumes (and
/// forgets) an operand the caller's own header has no field for.
#[allow(clippy::too_many_arguments)]
fn bare_generated_word_own_module_grounding(
    only: &Overload,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<Option<Overload>, String> {
    // The two shapes `struct_generated_sigs` registers -- a constructor
    // `S ( fields -- S )` and a destructure `S> ( S -- fields )` -- told
    // apart by the surface key, the only spelling a source term can carry.
    let (header_name, destructure) = match name.strip_suffix('>') {
        Some(base) => (base, true),
        None => (name, false),
    };
    let slot = match destructure {
        true => only.sig.inputs.first(),
        false => only.sig.outputs.first(),
    };
    let (Some(Type::Struct(id, instantiated)), Some(cell)) = (slot.copied(), ctx.generics()) else {
        return Ok(None);
    };
    let caller_module = span.module;
    // The registry filters first, and the allocating identity check below
    // only for a candidate that survives them. Every early exit here is a
    // plain `Ok(None)` -- but the order is no longer free across the whole
    // function (P7b.S10, REQ-2): a headerless caller must pass through the
    // candidate-identity check *before* the ambiguity arm can run, so the
    // "ordinary user word whose output happens to be another module's
    // instantiation" shape keeps its own resolution even where the new check
    // would otherwise fire. `struct_instantiation_of` -> `None` and the
    // own-module reject stay interchangeable; this is the reject route for
    // every generated-word call in a program with no same-named headers at
    // all.
    let (gi, declaring_module, args, lens) = {
        let guard = cell.borrow();
        // A hand-written concrete `type:` has no header to re-ground at.
        let Some((gi, owning_module, args, lens)) = guard.struct_instantiation_of(id) else {
            return Ok(None);
        };
        if owning_module == caller_module {
            return Ok(None);
        }
        // R2: the header's true *declaring* module (`GenericStructDecl.module`),
        // not `struct_instantiation_of`'s *instantiating* module -- a qualified
        // spelling in a foreign module's own signature instantiates a foreign
        // header, so the two can differ. They coincide across every path that
        // reaches the exemptions below today, but the exemptions compare
        // against this component, never the instantiating one.
        let declaring_module = guard.structs[gi].module;
        (gi, declaring_module, args.to_vec(), lens.to_vec())
    };
    // The candidate must actually *be* that struct's generated word: an
    // ordinary user word whose output (or operand) happens to be another
    // module's instantiation keeps its own resolution. Moved ahead of the
    // own-header branch (P7b.S10, REQ-2): the old control flow returned
    // `Ok(None)` at the missing-own-header fall-through before ever reaching
    // this check, so the headerless shape the ambiguity check governs would
    // never have survived it.
    let Some((key, symbol, _, _)) = generated_word_entry(ctx, id, destructure) else {
        return Ok(None);
    };
    if key != name || symbol != only.symbol {
        return Ok(None);
    }
    let own_idx = cell.borrow().find_struct(header_name, caller_module);
    let Some(own_idx) = own_idx else {
        // P7b.S10 (R1): headerless caller, one foreign env candidate that has
        // survived both the instantiating-module check and the
        // candidate-identity check. Either an exemption licenses borrowing it
        // unchanged, or the grounding is a located compile-time error -- never
        // a silent pick of whichever module happened to spell the
        // instantiation eagerly.
        foreign_single_candidate_grounding(
            cell,
            ctx,
            span,
            header_name,
            instantiated,
            declaring_module,
        )?;
        return Ok(None);
    };
    if own_idx == gi {
        return Ok(None);
    }
    let (own_ty_vars, declared_lens) = {
        let guard = cell.borrow();
        let own = &guard.structs[own_idx];
        // Cloned rather than borrowed across the mint below, which needs the
        // cell mutably.
        let own_ty_vars: Vec<(String, Option<crate::ast::Kind>)> = own
            .ty_var_names
            .iter()
            .enumerate()
            .map(|(i, v)| (v.clone(), own.ty_kinds.get(i).cloned()))
            .collect();
        (own_ty_vars, own.len_var_names.len())
    };
    // `substitute_generic_field` indexes the argument list raw (`args[v]`,
    // `src/ast.rs`), so minting the caller's own header against the
    // *candidate's* argument list panics outright on a header of a different
    // parameter count. Validate before minting -- and report rather than fall
    // back to the borrowed mint, which is the silent cross-module borrow
    // R1.1 forbids.
    if own_ty_vars.len() != args.len() {
        return Err(own_header_cannot_ground_error(
            ctx,
            span,
            header_name,
            &format!(
                "it declares {} type parameter{}, but the only `{header_name}` instantiation in scope supplies {}",
                own_ty_vars.len(),
                plural_s(own_ty_vars.len()),
                args.len(),
            ),
        ));
    }
    if declared_lens != lens.len() {
        return Err(own_header_cannot_ground_error(
            ctx,
            span,
            header_name,
            &format!(
                "it declares {declared_lens} length parameter{}, but the only `{header_name}` instantiation in scope supplies {}",
                plural_s(declared_lens),
                lens.len(),
            ),
        ));
    }
    // The counts agreeing is not enough: `substitute_generic_field`'s `App`
    // arm applies a header variable's binding *as a constructor*, and falls
    // back to returning the binding unapplied on anything that is not a
    // `CtorImage` -- a fall-back whose own doc calls itself unreachable
    // because `validate_ctor_arg_kinds` (`parser.rs`) rejects a kind mismatch
    // at the use site. Grounding here mints against an argument list written
    // at *another module's* use site, which that guard never saw, so the same
    // rule is re-applied to it: an HKT header grounded at a `*`-kinded
    // argument would otherwise take that fall-back and silently give the
    // field a wrong type.
    for ((var, kind), arg) in own_ty_vars.iter().zip(args.iter()) {
        let wants_ctor = matches!(kind, Some(crate::ast::Kind::Arrow { .. }));
        let is_ctor = matches!(arg, Type::CtorImage(_, _));
        if wants_ctor != is_ctor {
            let detail = match wants_ctor {
                true => format!(
                    "its `{var}` takes a type constructor, but the only `{header_name}` instantiation in scope supplies the concrete type `{arg}`"
                ),
                false => format!(
                    "its `{var}` takes a concrete type, but the only `{header_name}` instantiation in scope supplies the type constructor `{arg}`"
                ),
            };
            return Err(own_header_cannot_ground_error(
                ctx,
                span,
                header_name,
                &detail,
            ));
        }
    }
    let ty = {
        let mut guard = cell.borrow_mut();
        let regs = crate::ast::MutRegistries {
            structs: ctx.structs(),
            enums: ctx.enums(),
            arrays,
            cells,
            refs,
        };
        guard.instantiate_struct(own_idx, &args, &lens, caller_module, regs)
    };
    let Type::Struct(new_id, _) = ty else {
        unreachable!("instantiate_struct always returns a Type::Struct")
    };
    let Some((_, symbol, module, sig)) = generated_word_entry(ctx, new_id, destructure) else {
        unreachable!(
            "the caller's own mint was just registered in the live cell, so its generated-word entry resolves"
        )
    };
    Ok(Some(Overload {
        sig,
        symbol,
        module,
    }))
}

/// P7b.S10 (R1/R2): a headerless caller's single foreign env candidate, after
/// it has survived both the instantiating-module check (`owning_module ==
/// caller_module`) and the candidate-identity check. Either an exemption
/// licenses borrowing the candidate unchanged (`Ok(())`), or the grounding is
/// a located compile-time error -- never a silent pick of whichever module
/// happened to spell the instantiation eagerly (the S9 Residual this slice
/// closes).
///
/// The exemptions (R1, in the spec's order):
/// 1. own header -- unreachable here (the caller is headerless; the own-header
///    path branched off above);
/// 2. at most one same-named header reachable **and** the sole candidate's
///    declaring module itself reachable, over the fully-resolved reachable set
///    (raw `imports` ∪ `selective` targets, name-independent -- GO -- plus the
///    export-origin walk-extension -- GN);
/// 3. multi-candidate arm -- unreachable here (the caller is in the
///    single-candidate arm by construction);
/// 4. a *named* selective import of this surface name (never a wildcard
///    desugar -- GP) whose raw target, resolved through any hub re-export
///    chain first (GL), lands on the sole candidate's declaring module.
fn foreign_single_candidate_grounding(
    cell: &std::cell::RefCell<crate::ast::GenericTypes>,
    ctx: &Ctx,
    span: Span,
    header_name: &str,
    instantiated: &str,
    declaring_module: u32,
) -> Result<(), String> {
    // R2: no import-closure data, no check -- the same "reads it and never
    // fires when it is absent" discipline the D1 `drop` gate follows
    // (`engine.rs`'s `modules` doc): there is no import set to test
    // reachability against.
    let Some(modules) = ctx.modules() else {
        return Ok(());
    };
    let guard = cell.borrow();
    // The whole-program list of modules declaring a same-named generic header
    // (`ctx.generics().structs` -- complete at env-build time, probe P6).
    let declarers: HashSet<u32> = guard
        .structs
        .iter()
        .filter(|d| d.name == header_name)
        .map(|d| d.module)
        .collect();
    let caller = &modules[span.module as usize];
    // Exemption 2's reachable set: the caller's own import set -- plain
    // imports and selective targets alike, one hop, regardless of which name
    // each selective entry was keyed by (GO: a selective import of a
    // *different* name still makes its target reachable) -- plus, for every
    // module in that raw set, whatever module the export-origin walk resolves
    // the surface name to when started there (GN: a re-exporting hub with no
    // header of its own still chains through to the declaring module).
    let raw: Vec<u32> = caller
        .imports
        .values()
        .copied()
        .chain(caller.selective.values().copied())
        .collect();
    let mut reachable: HashSet<u32> = raw.iter().copied().collect();
    for start in &raw {
        if let Some(origin) = walk_generic_header_origin(*start, header_name, &declarers, modules) {
            reachable.insert(origin);
        }
    }
    // Exemption 2, both halves required: at most one same-named header
    // reachable, **and** the sole candidate's declaring module among them. A
    // reachable header that never mints does not entitle the caller to borrow
    // an unreachable module's instantiation instead (GM), and a second header
    // declared by a module the caller never imports does not count toward the
    // threshold (GH -- the reachability-scoping witness).
    let reachable_declaring: Vec<u32> = declarers.intersection(&reachable).copied().collect();
    if reachable_declaring.len() <= 1 && reachable.contains(&declaring_module) {
        return Ok(());
    }
    // Exemption 4: a *named* selective import of this surface name, its raw
    // target resolved through any hub re-export chain first, landing on the
    // sole candidate's declaring module. `named_selective` excludes wildcard
    // desugars (GP); the walk resolves a re-exporting hub down to the true
    // declaring module (GL); when it cannot resolve (a cycle or dead end)
    // the exemption does not apply -- the general rule decides (R2's `None`
    // arm). The match test is soundness-critical: a selective import the
    // caller believes already disambiguated the shape must not silently hand
    // it a *different* module's instantiation (GK).
    if let Some(&raw_target) = caller.named_selective.get(header_name) {
        if walk_generic_header_origin(raw_target, header_name, &declarers, modules)
            == Some(declaring_module)
        {
            return Ok(());
        }
    }
    // The general rule: a located error. At least two reachable declaring
    // headers is the ambiguity shape (GA); otherwise the sole candidate's
    // declaring module is simply unreachable from the caller (GM) -- a reach
    // failure, not an ambiguity, so the message does not claim several
    // competing candidates.
    if reachable_declaring.len() >= 2 {
        Err(ambiguous_generic_headers_error(
            ctx,
            span,
            header_name,
            instantiated,
            caller,
            &reachable_declaring,
        ))
    } else {
        Err(unreachable_declaring_module_error(
            ctx,
            span,
            header_name,
            instantiated,
        ))
    }
}

/// P7b.S10 (R2): the generic-header twin of `driver.rs`'s
/// `walk_type_export_origin` -- the same chase (follow an unqualified
/// selective re-export, else the declaring import target whose qualifier
/// key sorts lexicographically smallest -- keys are source text (the same
/// strings the diagnostic renderer uses as display names), while module
/// ids follow import-discovery order, so keying the single origin on ids
/// would flip it with the hub's source import order; `None` on a cycle or
/// a dead end), but over the generic header registry rather than the concrete
/// `StructDecl`/`EnumDecl` scan, which never sees a generic `type:` header
/// (`parser.rs` excludes it from that scan, and it lives in the generic
/// registry instead). Structurally the same walk, not a call into that one
/// -- and not the precomputed `type_origin` table either: both are
/// concrete-type-only, so neither can resolve a generic header through a hub
/// at all (GL/GN would fail if built on either).
fn walk_generic_header_origin(
    start: u32,
    name: &str,
    declarers: &HashSet<u32>,
    modules: &[ModuleInfo],
) -> Option<u32> {
    let mut visited: HashSet<u32> = HashSet::new();
    let mut current = start;
    loop {
        if !visited.insert(current) {
            return None;
        }
        if declarers.contains(&current) {
            return Some(current);
        }
        let info = &modules[current as usize];
        current = info.selective.get(name).copied().or_else(|| {
            // Several plain imports may declare the same header name; the
            // single origin this walk yields must be deterministic. Module
            // ids follow import-discovery order, so a smallest-id pick
            // would flip with the hub's source import order; the qualifier
            // *key* is source text -- the same strings the diagnostic
            // renderer uses as display names -- so pick the declaring
            // target whose qualifier sorts lexicographically smallest.
            // Keys are unique, so the pick is total and HashMap iteration
            // order never leaks in (both call sites above share this walk).
            info.imports
                .iter()
                .filter(|(_, &target)| declarers.contains(&target))
                .min_by_key(|(qualifier, _)| qualifier.as_str())
                .map(|(_, &target)| target)
        })?;
    }
}

/// P7b.S10 (R4): how the caller names a declaring module in the ambiguity
/// message. `ModuleInfo` carries no module name of its own, so every
/// existing diagnostic that names a foreign module renders the caller's own
/// import qualifier for it (`declarations.rs`/`word_families.rs` precedent);
/// a module bound only per-export through a `*` wildcard has no qualifier
/// and falls back to the existing wildcard-import phrasing
/// (`selective_not_exported_error`'s); a module the caller never imports in
/// any form (a walk-resolved re-export origin) is named structurally, the
/// `drop`-visibility diagnostic's precedent for exactly this gap -- never a
/// fabricated name.
fn module_display_name(caller: &ModuleInfo, target: u32) -> String {
    if let Some((qualifier, _)) = caller.imports.iter().find(|(_, &t)| t == target) {
        format!("`{qualifier}`")
    } else if caller.selective.values().any(|&t| t == target) {
        "its wildcard-imported module".to_string()
    } else {
        "a module this module never imports directly".to_string()
    }
}

/// P7b.S10 (R3/R4/R5): the ambiguity shape's message. Names the surface
/// name, the call site, and every reachable declaring module by the caller's
/// own qualifier for it, joined lexicographically (R4 -- not registry,
/// import, or mint order, so the text is byte-identical across import
/// orders and minter placements, REQ-5), and points at the remedies that
/// actually cure the shape (R5's two-part remedy 2 -- selective import alone
/// cures only when the named module itself instantiates; otherwise the type
/// must also be spelled in the caller's own signature).
fn ambiguous_generic_headers_error(
    ctx: &Ctx,
    span: Span,
    header_name: &str,
    instantiated: &str,
    caller: &ModuleInfo,
    reachable_declaring: &[u32],
) -> String {
    let mut named: Vec<String> = reachable_declaring
        .iter()
        .map(|&m| module_display_name(caller, m))
        .collect();
    named.sort();
    // The remedy example spells the lexically-first bound qualifier. A
    // declaring module with no bound qualifier at all (the both-wildcard
    // sub-case, unexercised by the goldens) drops the worked example rather
    // than fabricating a spelling (R4's flagged, non-blocking gap).
    let example = named
        .iter()
        .find_map(|n| n.strip_prefix('`').and_then(|r| r.strip_suffix('`')));
    let remedy_tail = match example {
        Some(q) => format!(
            "also spell the type in your own word's signature (`import: self::{q} | {header_name} | ;` then `: mk ( i64 -- {instantiated} ) {header_name} ;`)"
        ),
        None => "also spell the type in your own word's signature".to_string(),
    };
    format!(
        "error: `{header_name}`{} (line {}, col {}) is ambiguous: declared in modules {}, and {}'s module declares no `{header_name}`\n  note: declare your own `{header_name}` header and impl, or selectively import the module whose `{header_name}` you want -- if that module does not itself instantiate `{instantiated}`, {remedy_tail}",
        in_word(ctx),
        span.line,
        span.col,
        join_module_display_names(&named),
        ctx.rendered_word(),
    )
}

/// P7b.S10 (R3/R5): the reach-failure shape's message (GM). One candidate
/// exists, but its declaring module is one the caller's module does not
/// import in any form -- not "ambiguous", so the wording must not claim
/// several competing candidates. The module is never named by qualifier: an
/// unreachable module has none by construction, and fabricating one is
/// exactly the `drop`-diagnostic's rejected predecessor.
fn unreachable_declaring_module_error(
    ctx: &Ctx,
    span: Span,
    header_name: &str,
    instantiated: &str,
) -> String {
    format!(
        "error: `{header_name}`{} (line {}, col {}) is unresolved: the only `{instantiated}` instantiation in scope is declared in a module {}'s module does not import\n  note: import the module that declares the instantiation you want, or declare and instantiate your own `{header_name}` header",
        in_word(ctx),
        span.line,
        span.col,
        ctx.rendered_word(),
    )
}

/// P7b.S10 (R4): join the collected module display names -- two with "and",
/// three or more with `, ` between items and a final `, and`. The caller
/// sorts them lexicographically before this runs.
fn join_module_display_names(named: &[String]) -> String {
    match named.len() {
        1 => named[0].clone(),
        2 => format!("{} and {}", named[0], named[1]),
        _ => format!(
            "{}, {and} {}",
            named[..named.len() - 1].join(", "),
            named[named.len() - 1],
            and = "and",
        ),
    }
}

/// P7b.S9 Phase 2 (R1.1a): one struct's own generated constructor (or, with
/// `destructure`, its destructure) entry, read through the accessor that sees
/// both a still-pending mid-word mint and one `check`'s per-word bracket has
/// already flushed into the live registry -- a second bare-ctor site in one
/// module reaches its own module's mint through the flushed prefix, and the
/// unflushed-only `GenericTypes::struct_decl` would miss it and fail open
/// onto the borrowed candidate.
fn generated_word_entry(
    ctx: &Ctx,
    id: StructId,
    destructure: bool,
) -> Option<(String, String, u32, Sig)> {
    ctx.with_struct_decl_or_generic(id, |d| {
        d.map(|d| {
            let [ctor, destr] = struct_generated_sigs_of(id, d);
            match destructure {
                true => destr,
                false => ctor,
            }
        })
    })
}

/// P7b.S9 Phase 2 (R1.1a): a bare generated-word call in a module that
/// declares its own generic header under that name, where the only
/// instantiation in scope is another module's, minted at an argument list
/// this module's header cannot be applied to -- a differing parameter count,
/// or a kind its `App` field would misread. There is nothing to ground and
/// nothing safe to borrow, so the call site is reported. `detail` names which
/// of the two it was.
fn own_header_cannot_ground_error(ctx: &Ctx, span: Span, header: &str, detail: &str) -> String {
    format!(
        "error: `{header}`{} (line {}) cannot ground at this module's own header: {detail}\n  note: name an instantiation of this module's own `{header}` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's",
        in_word(ctx),
        span.line,
    )
}

fn plural_s(n: usize) -> &'static str {
    match n {
        1 => "",
        _ => "s",
    }
}

/// P7.S11-follow (Part 4): a shared `env`-miss fallback for the generated
/// constructor/accessor of a check-time-only monomorph. Both originating
/// mints -- a combinator's own splice-site output grounding (Part 1) and an
/// ordinary mid-word poly call's `apply_subst` mint -- land in the same live
/// `generics_cell` regardless of call shape, so this one fallback at the
/// point of use covers both. Read-through, mutates nothing.
///
/// The id-derivation runs over the *extended* slice (`ctx.enums()`/
/// `ctx.structs()` ++ the live cell's unflushed pending tail), never the
/// pending tail alone -- `struct_generated_sigs`/`enum_generated_sigs`/
/// `variant_generated_sigs` mint each candidate's own id from `enumerate()`
/// over whatever slice they are handed, so run alone over the tail every
/// candidate would mint at `from_index(0..)`, a wrong, colliding id
/// (`enum_generated_sigs_over_an_extended_slice_carries_the_monomorphs_own_id`).
/// Skipping the flushed prefix's own sig count is exact because the flushed
/// prefix and the pending tail meet with no gap or overlap on every live path
/// (`enum_base == ctx.enums().len()`, resp. `struct_base ==
/// ctx.structs().len()`) -- see the spec's invariant note.
///
/// Returns *all* pending mints whose surface name matches `name` --
/// variant-ctor env keys are module-blind, so two pending mints can in
/// principle generate the same surface name. Dispatch over the result
/// follows the existing env-overload discipline, and this fallback still
/// invents no rule of its own: the candidates it yields are treated exactly
/// as a present `env` entry's would be.
///
/// P7b.S11 (strict-grounding amendment 260910): this fallback still serves
/// every NON-ctor bare name exactly as before -- dispatch over its result
/// follows the existing env-overload discipline. For a bare generic
/// constructor it no longer has any grounding role at all: the strict ladder
/// in `ground_bare_generic_ctor` derives θ from the call site alone and
/// never consults module mints to fill, disambiguate, or veto a parameter
/// (the former candidate-filter/ambiguity machinery was retired with the
/// ruling), so this function's output reaches the ctor path only through the
/// category fence's identity check.
///
/// P7b.S5 (R4/Fix D, Phase 2b's mint_fallback module-provenance probe --
/// VERDICT: NOT reliably the declaring module). Each returned `Overload`'s
/// `.module` traces to the `module` argument `instantiate_struct`/
/// `instantiate_enum` was minted under. Tracing the one live construction
/// path that reaches a still-pending mint through this fallback
/// (`poly_construct_generic`, `poly.rs:5900`+): it takes the module from
/// `poly_construction_fallback` (the enclosing word's declared *output*
/// naming this header, its own module) when one exists, but falls back to
/// `ctx.module()` when it does not (`poly.rs`, the `(module, output_args)`
/// match arm). `ctx.module()` is the exact splice-rescoped value R3.5 rules
/// `span.module` around for tier 1's own-module comparison. So this
/// provenance's `.module` is reliable in the declared-output case and
/// unreliable (a splice-scoped value) in the no-fallback case, with no way
/// to tell the two apart from the `Overload` alone -- call-site dispatch
/// therefore treats every fallback-sourced candidate as unverified
/// (`select_overload_fallback_sourced`, `builtins.rs`): tier 1 still runs
/// (safe regardless, since a wrong module id can only fail to match, never
/// falsely match another module's id), but tiers 2/3 are excluded in favour
/// of the pre-existing permissive first-match dispatch.
fn mint_fallback_candidates(name: &str, ctx: &Ctx) -> Vec<Overload> {
    ctx.with_extended_type_slices(|structs, enums| {
        let mut out = Vec::new();
        let struct_skip = struct_generated_sigs(ctx.structs()).len();
        for (n, symbol, module, sig) in struct_generated_sigs(structs).into_iter().skip(struct_skip)
        {
            if n == name {
                out.push(Overload {
                    sig,
                    symbol,
                    module,
                });
            }
        }
        let enum_skip = enum_generated_sigs(ctx.enums()).len();
        for (n, symbol, module, sig) in enum_generated_sigs(enums).into_iter().skip(enum_skip) {
            if n == name {
                out.push(Overload {
                    sig,
                    symbol,
                    module,
                });
            }
        }
        let variant_skip = variant_generated_sigs(ctx.enums()).len();
        for (n, symbol, module, sig) in variant_generated_sigs(enums).into_iter().skip(variant_skip)
        {
            if n == name {
                out.push(Overload {
                    sig,
                    symbol,
                    module,
                });
            }
        }
        out
    })
}

/// P7b.S11 Phase 2 (R-6): the generic header a bare ctor/destructure name
/// names, for the explicit-args gate only -- deliberately wider than
/// `ctor_grounding_header` below, whose fences (own module, constructors
/// only, star-kinded, length-free) belong to Phase 1's *grounding* ladder,
/// not to the question this answers ("can this name even carry a `[...]`
/// list"). A destructure or a foreign header still gets its arity checked
/// even though the ladder declines to ground it; a *concrete* (non-generic)
/// header is not the category at all -- its generated words take no type
/// arguments, and keep the pre-S11 `no_type_arguments_error` spelling.
/// `None` on no match (a genuinely undefined name, R-7) or on 2+ same-named
/// headers (ambiguous which arity applies; declining leaves the call to
/// `no_type_arguments_error`'s pre-existing rejection).
struct ExplicitCtorHeader {
    /// The header's own declared spelling (`Res`), for diagnostics.
    header: String,
    /// The header's type-parameter names, `'`-prefixed, in binding order.
    var_names: Vec<String>,
    /// The header's length-parameter names, in binding order -- a
    /// length-parameterized header is ungroundable through this route
    /// wholesale (Phase 1's fence), so its list keeps the baseline
    /// rejection rather than an arity verdict.
    len_var_names: Vec<String>,
}

fn explicit_args_ctor_header(name: &str, ctx: &Ctx) -> Option<ExplicitCtorHeader> {
    let base = name.strip_suffix('>').unwrap_or(name);
    let guard = ctx.generics()?.borrow();
    let mut found: Option<ExplicitCtorHeader> = None;
    for d in guard.enums.iter() {
        if d.ty_var_names.is_empty() {
            continue;
        }
        if d.variants.iter().any(|v| v.name == base) {
            if found.is_some() {
                return None;
            }
            found = Some(ExplicitCtorHeader {
                header: d.name.clone(),
                var_names: d.ty_var_names.clone(),
                len_var_names: d.len_var_names.clone(),
            });
        }
    }
    for d in guard.structs.iter() {
        if d.name == base && !d.ty_var_names.is_empty() {
            if found.is_some() {
                return None;
            }
            found = Some(ExplicitCtorHeader {
                header: d.name.clone(),
                var_names: d.ty_var_names.clone(),
                len_var_names: d.len_var_names.clone(),
            });
        }
    }
    found
}

/// P7b.S11 Phase 2 (R-6): a bare ctor/destructure name's explicit type-argument
/// list must supply exactly one argument per the header's declared
/// parameters -- prefix pinning (`Ok[i64]` meaning `Ok[i64 'E]`) is out of
/// scope. Replaces nothing (Phase 1 had no explicit-args category at all,
/// R-6); measured and pinned fresh.
fn explicit_ctor_arity_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    header: &str,
    var_names: &[String],
    got_type: usize,
    got_len: usize,
) -> String {
    let demangled = crate::resolve::demangle_call(name);
    let want = var_names.len();
    if got_type != want {
        format!(
            "error: `{demangled}`{} (line {}) takes {want} type argument{} (`{header}[{}]`), but {got_type} {} supplied",
            in_word(ctx),
            span.line,
            plural_s(want),
            var_names.join(" "),
            if got_type == 1 { "was" } else { "were" },
        )
    } else {
        // Right type arity, but the call also carried a length sublist. The
        // caller only asks here for length-free headers, so the list names
        // something the header does not declare at all.
        format!(
            "error: `{demangled}`{} (line {}) takes no length arguments (`{header}[{}]` declares type parameters only), but {got_len} {} supplied",
            in_word(ctx),
            span.line,
            var_names.join(" "),
            if got_len == 1 { "was" } else { "were" },
        )
    }
}

/// P7b.S11 Phase 1 (R-1): the generic header a bare generated *constructor*
/// call grounds at, plus the header's own shape. Declining (`None`) leaves the
/// call to its pre-S11 resolution byte-for-byte, and the fences are
/// deliberately narrow:
///
/// - **own module only.** A foreign header's mints are S9/S10's territory
///   (NFR-4); strict grounding never consults scope, so the only headers
///   whose bare calls this ladder ever re-grounds are ones this module
///   declares itself.
/// - **constructors only.** A destructure's single operand *is* the
///   monomorph, so the existing exact-operand match already grounds it and no
///   parameter can be left undetermined -- there is nothing here to add.
/// - **no length or higher-kinded parameters.** θ below reasons in the type
///   domain over plain `Type` arguments; a `Len` parameter or an `Arrow`
///   kind would need the `CtorImage`/`Len` reasoning
///   `bare_generated_word_own_module_grounding` carries, and those headers
///   keep their pre-S11 resolution instead.
/// - **one claimant.** Two own-module headers whose variants share a surface
///   name are declined rather than picked between: which header the name
///   means is not this slice's question.
struct CtorHeader {
    is_enum: bool,
    /// Index into `GenericTypes::enums` / `structs` per `is_enum`.
    gi: usize,
    /// The header's own declared spelling (`Res`), for diagnostics.
    header: String,
    /// The header's type-parameter names, `'`-prefixed, in binding order --
    /// the id space a field's `PolyType::Var` indexes into.
    var_names: Vec<String>,
    /// The constructor's declared operand types, first field deepest, in the
    /// header's own variable space.
    fields: Vec<PolyType>,
}

fn ctor_grounding_header(name: &str, span: Span, ctx: &Ctx) -> Option<CtorHeader> {
    if name.ends_with('>') {
        return None;
    }
    let guard = ctx.generics()?.borrow();
    let groundable = |vars: &[String], kinds: &[crate::ast::Kind], lens: &[String], module: u32| {
        module == span.module
            && !vars.is_empty()
            && lens.is_empty()
            && kinds.iter().all(|k| matches!(k, crate::ast::Kind::Star))
    };
    let mut found: Option<CtorHeader> = None;
    for (gi, d) in guard.enums.iter().enumerate() {
        if !groundable(&d.ty_var_names, &d.ty_kinds, &d.len_var_names, d.module) {
            continue;
        }
        for v in d.variants.iter().filter(|v| v.name == name) {
            if found.is_some() {
                return None;
            }
            found = Some(CtorHeader {
                is_enum: true,
                gi,
                header: d.name.clone(),
                var_names: d.ty_var_names.clone(),
                fields: v.fields.iter().map(|(_, p)| p.clone()).collect(),
            });
        }
    }
    for (gi, d) in guard.structs.iter().enumerate() {
        if d.name != name || !groundable(&d.ty_var_names, &d.ty_kinds, &d.len_var_names, d.module) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(CtorHeader {
            is_enum: false,
            gi,
            header: d.name.clone(),
            var_names: d.ty_var_names.clone(),
            fields: d.fields.iter().map(|(_, p)| p.clone()).collect(),
        });
    }
    found
}

/// P7b.S11 (R-3 retired 260910): every existing monomorph of `h` whose
/// generated constructor `name` names, paired with the concrete argument list
/// it was instantiated at. Read over the *extended* type slices, so a mint
/// `env` never saw -- still pending in the live cell, or flushed into the
/// registry after `env` was built (the gap `generated_word_entry`'s doc
/// describes) -- is a candidate here too.
///
/// Under the strict-grounding amendment this list no longer filters or binds
/// anything: its one surviving reader is `ground_bare_generic_ctor`'s
/// category fence, which checks *identity* (are the pre-existing resolution's
/// candidates this header's mints, or a same-named user word / foreign
/// generated word to be left alone?) and never reads parameter values off
/// it.
fn header_mint_candidates(name: &str, h: &CtorHeader, ctx: &Ctx) -> Vec<(Overload, Vec<Type>)> {
    // Collected out of the `with_extended_type_slices` closure: that helper
    // holds a shared borrow of the live cell for the closure's whole extent,
    // and resolving each candidate's argument list borrows it again.
    let sigs = ctx.with_extended_type_slices(|structs, enums| match h.is_enum {
        true => enum_generated_sigs(enums),
        false => struct_generated_sigs(structs),
    });
    let mut out = Vec::new();
    for (n, symbol, module, sig) in sigs {
        if n != name {
            continue;
        }
        let o = Overload {
            sig,
            symbol,
            module,
        };
        let Some(args) = o
            .sig
            .outputs
            .first()
            .copied()
            .and_then(|t| header_args_of_type(t, h, ctx))
        else {
            continue;
        };
        out.push((o, args));
    }
    out
}

/// The concrete argument list `ty` instantiates `h` at, or `None` when `ty`
/// is not a monomorph of this header at all.
fn header_args_of_type(ty: Type, h: &CtorHeader, ctx: &Ctx) -> Option<Vec<Type>> {
    let guard = ctx.generics()?.borrow();
    let (gi, _, args, lens) = match (ty, h.is_enum) {
        (Type::Enum(id, _), true) => guard.enum_instantiation_of(id)?,
        (Type::Struct(id, _), false) => guard.struct_instantiation_of(id)?,
        _ => return None,
    };
    (gi == h.gi && lens.is_empty() && args.len() == h.var_names.len()).then(|| args.to_vec())
}

/// P7b.S11 Phase 1 (R-2): the substitution this call site determines, in the
/// header's own parameter order, plus the monomorph a consumer named outright
/// (`pinned`).
///
/// `pinned` is grounded *at that very id*, never re-minted:
/// `instantiate_enum`/`instantiate_struct` dedup on `(header, instantiating
/// module, arguments)`, so re-minting a monomorph another module instantiated
/// under the caller's own module id would fork a second, divergent monomorph
/// of one `(word, θ)` -- exactly what R-8 forbids.
struct CtorTheta {
    args: Vec<Option<Type>>,
    pinned: Option<Type>,
}

#[allow(clippy::too_many_arguments)]
fn derive_ctor_theta(
    h: &CtorHeader,
    type_args: &[Type],
    stack: &[Slot],
    siblings: &[Term],
    at: usize,
    tail: bool,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    scope: &Scope,
    poly: &PolyCtx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> CtorTheta {
    let mut theta = CtorTheta {
        args: vec![None; h.var_names.len()],
        pinned: None,
    };
    // R-2 step 1 (explicit type args, P7b.S11 Phase 2/R-6): the gate
    // admitted the list and the ladder validated full arity, so the list
    // pins every parameter outright -- position `i` binds parameter `i`, the
    // same positional contract `check_poly_call` seeds by (P7.S3t). Nothing
    // is left for the consumer or the operands to determine, and the fully
    // bound θ grounds below through the same lookup-or-mint every other
    // route uses (R-8) -- so `Ok[i64 i64]` is accepted with no consumer at
    // all (dp_e2) and with a competing mint in scope (the args'
    // instantiation is minted fresh rather than borrowed).
    if !type_args.is_empty() {
        for (slot, t) in theta.args.iter_mut().zip(type_args) {
            *slot = Some(*t);
        }
        return theta;
    }
    // R-2 step 2 (consumer constraints). Reached only on a bare call: with
    // explicit type args the step above has already pinned every parameter
    // and returned.
    if let Some(ty) = consumer_expected_type(
        siblings, at, tail, ctx, env, scope, poly, arrays, cells, refs,
    ) {
        if let Some(args) = header_args_of_type(ty, h, ctx) {
            for (slot, a) in theta.args.iter_mut().zip(args) {
                *slot = Some(a);
            }
            theta.pinned = Some(ty);
        }
    }
    // R-2 step 3: literal-driven partial inference from the operand types
    // already at the call site, which this path discarded before. Only a
    // field that *is* a bare header variable pins one: a variable nested
    // inside an array/reference/cell shape would need real unification, and
    // reading it wrongly would ground the site at the wrong monomorph, so
    // those positions stay wildcards: under the strict-grounding amendment
    // (260910) an undetermined parameter is the located unbound-parameter
    // error, never a guess from scope.
    if stack.len() >= h.fields.len() {
        let base = stack.len() - h.fields.len();
        for (i, f) in h.fields.iter().enumerate() {
            if let PolyType::Var(v) = f {
                let slot = &mut theta.args[*v as usize];
                if slot.is_none() {
                    *slot = Some(stack[base + i].ty);
                }
            }
        }
    }
    theta
}

/// P7b.S11 Phase 1 (R-2, consumer constraints): the concrete type the value
/// this constructor is about to push is required to have by the first term
/// that consumes it.
///
/// Deliberately narrow. Only *pure pushes* (a literal, a quotation literal, a
/// named local) are stepped over, and the first real call must consume the
/// constructed slot directly; nothing here simulates a call's net stack
/// effect, so a consumer further down the term list yields no constraint
/// rather than a guessed one. Grounding the site at a guessed monomorph would
/// be a miscompile, not a diagnostic.
///
/// Three flavors (R-2). A **monomorphic** consumer's `env` signature names the
/// type outright (dp_g2's `only_takes_cstr_err` pins both parameters). A
/// **polymorphic** consumer's own signature names it once its explicit type
/// arguments are applied, through `apply_subst` -- the same route the
/// consumer's own `check_poly_call` takes, so when that input is a
/// `PolyType::Generic` the monomorph minted here and the one the consumer
/// resolves are one monomorph (R-8). A call whose consumer was already
/// determined upstream (explicit type args, a poly consumer's own type
/// arguments) is pinned by those before any fallback below runs.
///
/// Running off the end of the term list in **tail** position reaches a third
/// consumer: the enclosing word's own declared output, which is the
/// "expectation flows in from a concretely-typed helper's declared output
/// effect" the dp_a control describes. It is the only pin a zero-field
/// variant constructor (`None`) can have, since it has no operands to infer
/// from; a wrong read here cannot escape, because the word-exit output check
/// compares that very slot against that very declaration.
///
/// Strict amendment (260910), the spliced-body half of the same channel:
/// inside a poly-combinator splice, running off the *spliced body's* term
/// list means the combinator's own declared output consumes the value -- the
/// direct consumer of the body's result (the tail channel above reads the
/// *caller's* outputs, the consumer one step removed, and only in tail
/// position). The combinator's declared output is instantiated through the
/// splice's own substitution (`apply_subst`, the same grounding Part 1
/// performs ahead of the splice, so the mint dedups onto it), and a bare
/// ctor at a spliced body's tail is use-determined by that signature --
/// never by module scope. Mono combinators carry no `combinator_sig` and
/// decline here byte-identically; this is a fallback, so a site the tail
/// channel already pins keeps today's bytes.
#[allow(clippy::too_many_arguments)]
fn consumer_expected_type(
    siblings: &[Term],
    at: usize,
    tail: bool,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    scope: &Scope,
    poly: &PolyCtx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Option<Type> {
    // Slots pushed between the construction and its consumer, so the
    // consumer's own input window can be indexed from the top.
    let mut depth = 0usize;
    for term in siblings.get(at + 1..)? {
        let (cname, type_args, len_args) = match &term.kind {
            TermKind::IntLit(_)
            | TermKind::FloatLit(_)
            | TermKind::StrLit(_)
            | TermKind::Quotation(..) => {
                depth += 1;
                continue;
            }
            TermKind::Bind(_) => return None,
            TermKind::Call(n, t, l) => (n, t, l),
        };
        if scope.local_type(cname).is_some() {
            depth += 1;
            continue;
        }
        // A builtin, an operator, an eliminator and a combinator are all
        // intercepted upstream of `env`/`poly.env`, so any window read off a
        // signature here would be a guess about a route this lookahead does
        // not model.
        if is_builtin_word_name(cname)
            || is_builtin_operator_name(cname)
            || poly.eliminators.contains_key(cname)
            || poly.combinators.contains_key(cname)
        {
            return None;
        }
        if let Some([only]) = env.get(cname).map(|v| v.as_slice()) {
            let n = only.sig.inputs.len();
            return (depth < n).then(|| only.sig.inputs[n - 1 - depth]);
        }
        let [sig] = poly.env.get(cname)?.as_slice() else {
            return None;
        };
        // A row-carrying or length-parameterized consumer is out of scope:
        // `row_in` makes the input window's *depth* a function of the call
        // site rather than of `inputs.len()`.
        if sig.row_in.is_some()
            || !sig.len_var_names.is_empty()
            || !len_args.is_empty()
            || type_args.len() != sig.ty_var_names.len()
            || depth >= sig.inputs.len()
        {
            return None;
        }
        // P7.S3t's positional contract: written argument `i` binds variable
        // `i`, pushed in ascending id exactly as `check_poly_call` seeds it.
        let subst = Subst {
            ty: type_args
                .iter()
                .enumerate()
                .map(|(v, t)| (v as u32, *t))
                .collect(),
            len: Vec::new(),
        };
        let slot = &sig.inputs[sig.inputs.len() - 1 - depth];
        return apply_subst(
            sig, slot, &subst, cname, term.span, ctx, arrays, cells, refs,
        )
        .ok();
    }
    // Nothing consumes it inside this term list. In tail position the
    // enclosing word's declared output is what does.
    let outputs = ctx.declared_outputs();
    if tail && depth < outputs.len() {
        return Some(outputs[outputs.len() - 1 - depth].ty);
    }
    // Strict amendment (260910), the spliced-body consumer half: see this
    // function's doc. A poly combinator's declared output, instantiated
    // through the splice's own substitution, is what consumes the spliced
    // body's result; `apply_subst` errors (an output variable the splice did
    // not bind) decline the pin, as does a mono combinator (`combinator_sig`
    // is `None`) or an empty output window at this depth.
    if let Some(sig) = poly.combinator_sig.as_ref() {
        if let (Some(subst), Some(cname)) = (&poly.combinator_subst, &poly.combinator_name) {
            if depth < sig.outputs.len() {
                let slot = &sig.outputs[sig.outputs.len() - 1 - depth];
                if let Ok(ty) = apply_subst(
                    sig,
                    slot,
                    subst,
                    cname,
                    siblings[at].span,
                    ctx,
                    arrays,
                    cells,
                    refs,
                ) {
                    return Some(ty);
                }
            }
        }
    }
    None
}

/// P7b.S11 Phase 1 (R-8): lookup-or-mint of `h` at a fully bound θ, through
/// the same `(header, module, arguments)`-keyed instantiator every other mint
/// goes through, so one `(word, θ)` keeps one symbol.
fn mint_header_instantiation(
    h: &CtorHeader,
    args: &[Type],
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Option<Type> {
    let cell = ctx.generics()?;
    let mut guard = cell.borrow_mut();
    let regs = crate::ast::MutRegistries {
        structs: ctx.structs(),
        enums: ctx.enums(),
        arrays,
        cells,
        refs,
    };
    Some(match h.is_enum {
        true => guard.instantiate_enum(h.gi, args, &[], span.module, regs),
        false => guard.instantiate_struct(h.gi, args, &[], span.module, regs),
    })
}

/// The generated constructor `name` of the monomorph `ty`, re-derived from
/// the registered decl through the same rule env registration uses, so the
/// whole `Overload` (`Sig`, lowering symbol, module) has one provenance.
fn ground_ctor_overload(name: &str, ty: Type, ctx: &Ctx) -> Option<Overload> {
    match ty {
        Type::Struct(id, _) => {
            generated_word_entry(ctx, id, false).and_then(|(k, symbol, module, sig)| {
                (k == name).then_some(Overload {
                    sig,
                    symbol,
                    module,
                })
            })
        }
        Type::Enum(id, _) => ctx.with_extended_type_slices(|_, enums| {
            enum_generated_sigs(enums)
                .into_iter()
                .find(|(n, _, _, sig)| {
                    n == name && matches!(sig.outputs.first(), Some(Type::Enum(e, _)) if *e == id)
                })
                .map(|(_, symbol, module, sig)| Overload {
                    sig,
                    symbol,
                    module,
                })
        }),
        _ => None,
    }
}

/// P7b.S11 (R-1, R-2, R-6, R-7, R-8; strict-grounding amendment 260910):
/// per-call-site grounding for a bare generic constructor call.
/// `Ok(Some(o))` grounds the site at `o`; `Err` is the located
/// unbound-parameter diagnostic. `Ok(None)` declines only for a name with no
/// groundable own-module constructor header here (a destructure, a foreign
/// header, a length or higher-kinded parameter, two claimants) or a same-named
/// non-ctor candidate the category fence below preserves -- never to let
/// module scope fill a parameter.
///
/// The outcome ladder (R-2's precedence order, strict per the 260910
/// ruling -- determined by use, or an error):
/// 1. explicit type args (full arity) pin every parameter outright;
/// 2. a consumer's declared signature pins θ (ruling A: a mono consumer
///    statically, a poly consumer through its own `check_poly_call` route,
///    the enclosing word's declared output in tail position);
/// 3. operand literals pin the leading *bare* header variables they cover.
///
/// Outcome on the θ derived: **fully bound** → ground directly --
/// lookup-or-mint, no candidate selection at all, so a site with no monomorph
/// in scope can still succeed (G5's fresh mid-check mint, G9's mid-check
/// lookup). **Any parameter left undetermined** by those three inputs is a
/// located `unbound_type_parameter_error` -- the module's mint registry is
/// never consulted to fill, disambiguate, or veto a bare ctor call's
/// parameters (retiring the 2+-mint ambiguity and sole-mint
/// incompatible-grounding diagnostics of the original ladder).
#[allow(clippy::too_many_arguments)]
fn ground_bare_generic_ctor(
    call: CtorCallSite<'_>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    scope: &Scope,
    poly: &PolyCtx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<Option<Overload>, String> {
    let CtorCallSite {
        name,
        span,
        candidates,
        type_args,
        len_args,
        stack,
        siblings,
        at,
        tail,
    } = call;
    // P7b.S11 Phase 2 (R-6): an explicit-args call on a name the gate's
    // ctor clause admitted. Every earlier route that reads an argument list
    // (a poly word's interception, member dispatch) has already taken such
    // a call; one reaching here would otherwise flow into a resolution that
    // drops the list in silence, so this ladder consumes it or rejects it.
    let explicit = !type_args.is_empty() || !len_args.is_empty();
    let Some(h) = ctor_grounding_header(name, span, ctx) else {
        if !explicit {
            return Ok(None);
        }
        // The gate admitted the list because *a* matching header exists, but
        // the name grounds at no own-module constructor here: a destructure,
        // a foreign header, a length or higher-kinded parameter, or two
        // claimants. The list is still checked for arity (against the wide
        // lookup, and only where the header has no length parameters -- one
        // of those is ungroundable through this route wholesale, so its list
        // keeps the baseline rejection), then the spelling keeps the same
        // `no_type_arguments_error` it met before this slice.
        if let Some(w) = explicit_args_ctor_header(name, ctx) {
            if w.len_var_names.is_empty()
                && (type_args.len() != w.var_names.len() || !len_args.is_empty())
            {
                return Err(explicit_ctor_arity_error(
                    ctx,
                    span,
                    name,
                    &w.header,
                    &w.var_names,
                    type_args.len(),
                    len_args.len(),
                ));
            }
        }
        return Err(no_type_arguments_error(
            span,
            name,
            !type_args.is_empty(),
            !len_args.is_empty(),
        ));
    };
    // R-6: full arity, exact -- prefix pinning (`Ok[i64]` meaning
    // `Ok[i64 'E]`) is out of scope. A groundable header is length-free, so
    // a length sublist names nothing the header declares at any arity.
    if explicit && (type_args.len() != h.var_names.len() || !len_args.is_empty()) {
        return Err(explicit_ctor_arity_error(
            ctx,
            span,
            name,
            &h.header,
            &h.var_names,
            type_args.len(),
            len_args.len(),
        ));
    }
    let mints = header_mint_candidates(name, &h, ctx);
    // The category fence: with candidates in hand, S11 only ever redirects a
    // call the pre-existing resolution would itself have resolved to a
    // monomorph of this header. A same-named user word, or another module's
    // generated word, keeps its own resolution. An *empty* candidate list is
    // the zero-candidate site whose only pre-S11 outcome was `unknown word`
    // (R-7), so there is nothing there to preserve.
    if !candidates.is_empty()
        && !candidates
            .iter()
            .any(|c| mints.iter().any(|(m, _)| m.symbol == c.symbol))
    {
        // R-6: with explicit args a decline here would drop the list in
        // silence -- the resolution that owns these candidates (a same-named
        // user word's own `env` entry) reads no argument list -- so the
        // spelling keeps its pre-S11 rejection instead.
        if explicit {
            return Err(no_type_arguments_error(
                span,
                name,
                !type_args.is_empty(),
                !len_args.is_empty(),
            ));
        }
        return Ok(None);
    }
    let theta = derive_ctor_theta(
        &h, type_args, stack, siblings, at, tail, ctx, env, scope, poly, arrays, cells, refs,
    );
    if let Some(ty) = theta.pinned {
        return Ok(ground_ctor_overload(name, ty, ctx));
    }
    // Strict grounding (maintainer ruling, 260910): the three θ inputs above
    // are the only things that can determine a parameter. Whatever mints the
    // module happens to carry are never consulted to fill the rest, so an
    // undetermined parameter is the located unbound-parameter error -- a
    // genuinely undefined name has no header and never reaches here (R-7),
    // so the two stay distinguishable.
    let Some(unbound) = theta.args.iter().position(|t| t.is_none()) else {
        let args: Vec<Type> = theta.args.iter().filter_map(|t| *t).collect();
        let ty = mint_header_instantiation(&h, &args, span, ctx, arrays, cells, refs);
        return Ok(ty.and_then(|ty| ground_ctor_overload(name, ty, ctx)));
    };
    Err(unbound_type_parameter_error(ctx, span, name, &h, unbound))
}

/// The call-site facts `ground_bare_generic_ctor` reads, grouped so the
/// argument list stays legible at both of its call sites.
struct CtorCallSite<'a> {
    name: &'a str,
    span: Span,
    /// What the pre-existing resolution had to work with -- empty at the
    /// zero-candidate arm.
    candidates: &'a [Overload],
    /// The call's explicit type/length argument lists, empty on a bare
    /// call. P7b.S11 Phase 2 (R-6): a non-empty list on this name is the
    /// explicit-args category -- either consumed here as R-2's first input
    /// or rejected (wrong arity, or a header this ladder cannot ground),
    /// never dropped in silence by the pre-S11 resolution.
    type_args: &'a [Type],
    len_args: &'a [crate::ast::Len],
    stack: &'a [Slot],
    siblings: &'a [Term],
    at: usize,
    /// Whether this term is the enclosing word's syntactic tail (the
    /// syntactic `tail` flag, not the runtime tail-call back-edge that the
    /// lowering pass tracks separately) -- the condition under which the
    /// term's declared output is the consumer.
    tail: bool,
}

/// The header as declared, `Res['T 'E]`.
fn rendered_header(h: &CtorHeader) -> String {
    format!("{}[{}]", h.header, h.var_names.join(" "))
}

/// P7b.S11 (R-5; strict-grounding amendment 260910): a bare generic
/// constructor with a type parameter this call site does not determine --
/// regardless of what monomorphs of its header exist in module scope, which
/// strict grounding never consults. Replaces the `unknown_word_error` this
/// shape used to borrow, which named the wrong word and was
/// indistinguishable from a genuinely undefined name (R-7 keeps that one for
/// the headerless case).
fn unbound_type_parameter_error(
    ctx: &Ctx,
    span: Span,
    name: &str,
    h: &CtorHeader,
    unbound: usize,
) -> String {
    let name = crate::resolve::demangle_call(name);
    format!(
        "error: `{name}`{} (line {}) cannot be grounded here: `{}`'s type parameter `{}` (parameter {} of {}) is determined by neither this call site's operands nor its consumer\n  note: pass the value to a consumer whose declared parameter names a concrete `{}[...]`, or name that instantiation in a signature so this call has one to ground at",
        in_word(ctx),
        span.line,
        rendered_header(h),
        h.var_names[unbound],
        unbound + 1,
        h.var_names.len(),
        h.header,
    )
}

/// P7b.S8b Phase 1 (R5): whether `chosen` is a generated enum word --
/// `(name, symbol)` membership in `enum_generated_sigs` (constructors) or
/// `variant_generated_sigs` (destructures) over the extended type slices.
/// The non-splice twin of `splice_enum_site`'s own membership test, which
/// needs the operative `EnumId` on top because a splice redirects per
/// `(uid, span)`; outside a splice the resolved symbol is the whole record.
/// An eliminator (`Opt?`) is deliberately absent: those sites are
/// intercepted by `check_eliminator_call` and never reach this arm.
fn is_generated_enum_word(name: &str, chosen: &Overload, ctx: &Ctx) -> bool {
    // P7b.S8b Phase 1 review (P2-7): a non-generic instantiation's mangled
    // symbol IS its bare surface name (`enum_generated_sigs` yields
    // `variant.name`, R5's own doc comment), so `chosen.symbol == name` can
    // only be that already-correctly-resolved bare-key case -- the dual-map
    // scan below only ever matters for a generic instantiation, whose
    // mangled symbol differs from the surface name. Skipping here is
    // behavior-preserving (the bare-key path already resolves those sites)
    // and shrinks the per-call `enum_generated_sigs`/`variant_generated_sigs`
    // build to generic instantiations only.
    if chosen.symbol == name {
        return false;
    }
    ctx.with_extended_type_slices(|_, enums| {
        enum_generated_sigs(enums)
            .into_iter()
            .chain(variant_generated_sigs(enums))
            .any(|(n, symbol, _, _)| n == name && symbol == chosen.symbol)
    })
}

/// P7b.S3 (S3-1.e): inside a combinator splice (`prov.splice_uid` is `Some`),
/// the operative `EnumId` of a chosen candidate that is a *generated enum
/// word* -- `(name, symbol)` membership in `enum_generated_sigs` (ctors) or
/// `variant_generated_sigs` (destructures) over the extended type slices, the
/// same source `mint_fallback_candidates` reads. Membership, not sig shape: an
/// `Overload` carries only `sig` + `symbol` and a user word's sig can also
/// output an enum. The id itself is read off the *chosen* candidate's own
/// generated sig (the ctor-output read `nullary_variant_idx` already uses):
/// the ctor carries it in its output, the destructure in its input -- two
/// monomorphs of one family share `(name, symbol)`, so the table's own id
/// would be the wrong one.
fn splice_enum_site(
    name: &str,
    chosen: &Overload,
    ctx: &Ctx,
    prov: &Provenance,
) -> Option<(u32, EnumId)> {
    let uid = prov.splice_uid?;
    let enum_id_at = |slots: &[Type]| {
        slots.iter().find_map(|t| match t {
            // A destructure's input is the *variant* type, a ctor's output
            // the enum -- both carry the operative id.
            Type::Enum(id, _) | Type::Variant(id, _, _) => Some(*id),
            _ => None,
        })
    };
    let id = ctx.with_extended_type_slices(|_, enums| {
        if enum_generated_sigs(enums)
            .into_iter()
            .any(|(n, symbol, _, _)| n == name && symbol == chosen.symbol)
        {
            return enum_id_at(&chosen.sig.outputs);
        }
        if variant_generated_sigs(enums)
            .into_iter()
            .any(|(n, symbol, _, _)| n == name && symbol == chosen.symbol)
        {
            return enum_id_at(&chosen.sig.inputs);
        }
        None
    })?;
    Some((uid, id))
}

/// P7.S11-follow (Part 3): whether `top` is a tagged quotation literal that
/// counts as an eliminator arm, and if so its `(QuotId, VariantTag)` -- the
/// shared stop condition for both `check_eliminator_call`'s destructive
/// arm-collection scan (`src/check.rs`) and `scrutinee_enum_id_of_family`'s
/// non-destructive peek below. Neither pops or otherwise mutates the stack
/// here; only the `check.rs` caller pops, over the count this decides.
pub(super) fn eliminator_arm_at(top: Slot, prov: &Provenance) -> Option<(QuotId, VariantTag)> {
    let Some(QuotOperand::Literal(qid)) = resolve_quotation_operand(top) else {
        return None;
    };
    let tag = prov.quotations[qid.0]
        .annot
        .as_ref()?
        .variant_tag
        .as_ref()?
        .clone();
    Some((qid, tag))
}

/// P7.S11-follow (Part 3): a non-destructive peek from the top of `stack`,
/// walking past tagged-quotation-literal arms (via `eliminator_arm_at`) to
/// the scrutinee slot -- the same stop condition `check_eliminator_call`'s
/// destructive scan uses, but here run *before* `check_eliminator_call` is
/// even invoked, so the stack must stay intact. Returns the scrutinee's slot,
/// or `None` if the stack holds only arms (or is empty).
fn peek_eliminator_scrutinee(stack: &[Slot], prov: &Provenance) -> Option<Slot> {
    let mut idx = stack.len();
    while idx > 0 {
        let top = stack[idx - 1];
        if eliminator_arm_at(top, prov).is_none() {
            return Some(top);
        }
        idx -= 1;
    }
    None
}

/// P7.S11-follow (Part 3): when the eliminator registry's classification for
/// a call is the frozen `Generic` entry, recover the scrutinee's own concrete
/// `Type::Enum(id, _)` from the live stack instead of erroring immediately --
/// a check-time mint elsewhere in the word, later than
/// `eliminator_registry` was built, may already have grounded this
/// instantiation. Confirms `id` resolves to a *real, minted* decl (Part 2's
/// `with_enum_decl_or_generic`) rather than merely a concrete-looking,
/// still-ungrounded `Type::Enum` -- a poly call's own unification can leave
/// one on the stack independent of whether anything actually grounded it, and
/// requiring an actual mint keeps a genuinely-ungrounded call getting the
/// honest "cannot eliminate it while it is ungrounded" diagnostic.
fn scrutinee_enum_id_of_family(
    stack: &[Slot],
    prov: &Provenance,
    refs: &[RefDecl],
    ctx: &Ctx,
) -> Option<EnumId> {
    let scrutinee = peek_eliminator_scrutinee(stack, prov)?;
    let referent = match ref_parts(scrutinee.ty, refs) {
        Some((referent, _)) => referent,
        None => scrutinee.ty,
    };
    let Type::Enum(id, _) = referent else {
        return None;
    };
    ctx.with_enum_decl_or_generic(id, |d| d.is_some())
        .then_some(id)
}

/// P7.S12 (R2.4/R7.4): a call to a generic enum's eliminator from a body the
/// *concrete* checker walks -- an ordinary monomorphic word, or
/// `check_poly_combinator_standalone`'s i64 stand-in. The registry keys the
/// header now (R2.1), so the call *name* resolves; what cannot is the
/// scrutinee. A concrete body's operand is always some monomorph, and a
/// `Generic` registry entry means no monomorph of this header exists to be
/// one (R2.3 registers `Concrete` whenever one does); the stand-in has no
/// instantiator to ground one with even in principle.
///
/// Its own message rather than the adjacency one: the arms are written
/// correctly, immediately before the call, and nothing here is a
/// written-adjacency mistake.
fn concrete_body_generic_eliminator_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let call = crate::resolve::demangle_call(name);
    let enum_name = call.trim_end_matches('?');
    format!(
        "error: `{call}` names the generic enum `{enum_name}`, but a concrete body cannot eliminate it while it is ungrounded{} (line {})\n  a concrete body eliminates a grounded instantiation of `{enum_name}`, never the header itself: an ungrounded scrutinee needs a polymorphic body",
        in_word(ctx),
        span.line,
    )
}

/// R15 (D8): a linear value live across the self-tail-call back-edge, which the
/// loop lowering would carry into the next iteration with nobody responsible
/// for disposing it. Deferred to a later Phase 3 slice, as a located error
/// rather than silence. Copy loops are untouched.
fn linear_across_back_edge_error(ctx: &Ctx, span: Span, callee: &str, ty: Type) -> String {
    let callee = crate::resolve::demangle_call(callee);
    format!(
            "error: linear values across a loop are not supported yet in {} (line {})\n  a `{}` is live across the self-tail-call back-edge to `{}`: consume it before the recursive call\n  note: declared {}",
            ctx.rendered_word(), span.line, ty, callee, effect_str(ctx.effect()))
}

/// R15: reject a linear value that would survive the back-edge of a
/// self-tail-call, either stranded on the stack below the call's arguments or
/// held by a local that was never consumed. A value *moved into* the call's
/// arguments is forwarded, not live across the edge, so it stays legal.
///
/// `frame_floor` is `Some` only at a spliced self-tail combinator's site, where
/// it is the entry depth of the `if` arm the back-edge sits in; a local bound
/// below it is exempt from the second clause. That clause is not what makes
/// disposal safe: an unconsumed linear is caught anyway by end-of-scope
/// disposal and the branch-join `MaybeMoved` guard, and a self-tail call has no
/// position after it, so the clause's only job is to *locate* that same
/// rejection at the back-edge. Below the floor the location is wrong: the loop
/// neither rebinds nor carries the local, and the enclosing word still owns and
/// disposes it. At the whole-word TCO site there is nothing below the floor to
/// admit (a self-call must supply the word's full declared inputs), so passing
/// a floor there would only open a hole.
fn check_linear_across_back_edge(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    below_args: &[Slot],
    scope: &Scope,
    arrays: &[ArrayDecl],
    frame_floor: Option<usize>,
) -> Result<(), String> {
    if let Some(slot) = below_args.iter().find(|s| {
        ctx.with_extended_type_slices(|structs, enums| is_linear(s.ty, structs, enums, arrays))
    }) {
        return Err(linear_across_back_edge_error(ctx, span, callee, slot.ty));
    }
    // `position` resolves to the *first* matching binding; this is only correct
    // because Sooth forbids rebinding a live name, so `name` names at most one
    // live position at a time. If that rule were ever relaxed, a shadowing
    // local could inherit an ancestor's floor-exempt index here.
    let below_floor = |name: &str| match frame_floor {
        Some(floor) => scope
            .bound
            .iter()
            .position(|b| b.name == name)
            .is_some_and(|at| at < floor),
        None => false,
    };
    if let Some(local) = scope
        .moves
        .unconsumed()
        .into_iter()
        .find(|name| !below_floor(name))
    {
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
    format!(
            "error: `call` in {} (line {}) expects a quotation on the stack (a quotation cannot be a runtime value; a runtime quotation value is slice 7)",
            ctx.rendered_word(), span.line
        )
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

/// Slice 10c (R-P3-1a): the two-way branch-and-join every conditional in the
/// language now goes through. Each arm advances its own clone of the
/// move-state and its own copy of the live stack; the join reconciles the two
/// (`MaybeMoved` where they disagree about a move, a merged alias/derivation
/// set, one `Slot` per position) or rejects them for disagreeing in depth,
/// type, quotation identity or suspended place.
///
/// Each `*_end` is the token that closes that arm and where it sits, for
/// `leave_block`'s unconsumed-linear diagnostic.
#[allow(clippy::too_many_arguments)]
fn check_branch_join(
    then_body: &[Term],
    then_end: (&'static str, Span),
    else_body: &[Term],
    else_end: (&'static str, Span),
    stack: Vec<Slot>,
    span: Span,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
    live: &Liveness,
    at: usize,
    siblings: &[Term],
    base_depth: usize,
    outer_releasable: &HashSet<String>,
) -> Result<Vec<Slot>, String> {
    // R14: each arm advances its own copy of the move-state; the join
    // reconciles them into `MaybeMoved` wherever they disagree. R2:
    // each arm is also a block, so a name it binds is gone by the join
    // and the two arms' name sets agree there again.
    let depth = scope.depth();
    // D6: `releasable_into` (see its doc) decides what's safe to grant
    // into either arm; an arm executes exactly once, so it may die at
    // its own last use inside (`back_edge = false`).
    let granted = releasable_into(
        scope,
        base_depth,
        outer_releasable,
        &siblings[at + 1..],
        live,
        at,
    );
    let mut then_scope = scope.clone();
    let mut else_scope = scope.clone();
    let then_stack = check_terms_relaxed(
        then_body,
        stack.clone(),
        ctx,
        env,
        arrays,
        cells,
        refs,
        slices,
        prov,
        &mut then_scope,
        tail,
        poly,
        &granted,
        false,
    )?;
    leave_block(
        ctx,
        &mut then_scope,
        depth,
        BlockEnd::Arm {
            token: then_end.0,
            span: then_end.1,
        },
    )?;
    let else_stack = check_terms_relaxed(
        else_body,
        stack,
        ctx,
        env,
        arrays,
        cells,
        refs,
        slices,
        prov,
        &mut else_scope,
        tail,
        poly,
        &granted,
        false,
    )?;
    leave_block(
        ctx,
        &mut else_scope,
        depth,
        BlockEnd::Arm {
            token: else_end.0,
            span: else_end.1,
        },
    )?;
    scope.moves = Moves::join(then_scope.moves, else_scope.moves);
    if then_stack.len() != else_stack.len() {
        return Err(branch_mismatch_error(
            ctx,
            span,
            then_stack.len(),
            else_stack.len(),
        ));
    }
    let mut merged = Vec::with_capacity(then_stack.len());
    for (i, (t_then, t_else)) in then_stack.iter().zip(&else_stack).enumerate() {
        // R7/R11: a branch merge cannot carry a quotation whose
        // identity is ambiguous *unless* the enclosing context declares
        // its type, in which case the join materializes each arm into a
        // runtime `(code, env)` value (D4). Two arms carrying the
        // *same* literal stay a forwarded marker (`lower_if`'s `t == e`
        // fast path emits no `Phi`, splice preserved). The `Cstr`
        // placeholder makes an arm's real `Cstr` compare equal to a
        // quotation, so the ordinary `ty` mismatch below never catches
        // the one-quotation shape; this guard has both phrasings.
        let (quot, erased_ty, surviving) = match (t_then.quot, t_else.quot) {
            (None, None) => (
                None,
                None,
                prov.union_surviving(t_then.surviving, t_else.surviving),
            ),
            (Some(QuotRef::Known(a)), Some(QuotRef::Known(b))) if a == b => {
                (Some(QuotRef::Known(a)), None, None)
            }
            (Some(QuotRef::Known(a)), Some(QuotRef::Known(b))) => {
                // R11 ordering pin: the capture admission runs before
                // the id/expected-type resolution, so a rejected
                // capturing arm raises R15 rather than falling through
                // to `different_quotations_at_join_error`. `escaping`
                // is true only at a word-body tail (the join feeds the
                // declared output); an in-frame join whose expected
                // type comes from a consumer is not escaping.
                let escaping = tail;
                // The expected quotation type threaded from the
                // enclosing declared context. At a word-body tail the
                // merged slot maps to the declared output at index `i`.
                // Otherwise the join may feed an in-frame store
                // `&!ref if..end !`, whose `&!Quotation` referent sits
                // directly below the merged slot and gives the erased
                // value its type (the "or field" the diagnostic
                // promises); an in-frame boundary is not escaping, so
                // the R21 admission above already ran with `escaping =
                // tail = false`. Without either the join cannot type the
                // erased value, so it stays a located error.
                let expected = if tail {
                    ctx.declared_outputs().get(i).map(|slot| slot.ty)
                } else {
                    i.checked_sub(1)
                        .and_then(|below| ref_parts(then_stack[below].ty, refs))
                        .map(|(referent, _)| referent)
                        .filter(|t| matches!(t, Type::Quotation(_)))
                };
                // P7.S3h: the declared type decides the flavour here exactly
                // as it does at every other materialization boundary, and it
                // has to be known *before* the capture admission below: an
                // owning boundary admits a linear capture the plain one
                // rejects.
                let expected = match expected {
                    Some(Type::OwningQuotation(eff)) => Some((eff, true)),
                    Some(Type::Quotation(eff)) => Some((eff, false)),
                    _ => None,
                };
                // R11 ordering pin (continued): the admission still runs
                // before the id/expected-type resolution's error arms below.
                let enclosing: HashSet<String> =
                    scope.bound.iter().map(|bnd| bnd.name.clone()).collect();
                let owning = matches!(expected, Some((_, true)));
                let mut arm_sets: Vec<SurvivingCaptureSetId> = Vec::new();
                for id in [a, b] {
                    let body = prov.quotations[id.0].body.clone();
                    if body_captures_enclosing(&body, &enclosing) {
                        let span = prov.quotations[id.0].span;
                        if let Some(set) = check_capture_admission(
                            id, escaping, owning, span, ctx, arrays, prov, scope,
                        )? {
                            arm_sets.push(set);
                        }
                    }
                }
                match expected {
                    Some((eff, owning)) => {
                        // `literal_effect_mismatch_error` renders what it is
                        // handed (`render_word`), so this hands it the mangled
                        // name rather than a pre-demangled one.
                        let word = ctx.mangled_name();
                        let a_span = prov.quotations[a.0].span;
                        let b_span = prov.quotations[b.0].span;
                        // P7.S3h: the two literals are *alternatives*, and an
                        // owning one consumes what it captures -- so walking
                        // them back to back on one move state has the second
                        // arm report use-after-move on a local the first arm
                        // consumed on the path the second never takes. Each
                        // arm starts from the same state and the two are
                        // joined, exactly as the `if`'s own arm walk above
                        // already does. Only the owning boundary needs this:
                        // at a plain one R12 rejects any consumption outright,
                        // so neither walk moves anything.
                        let moves_before = scope.moves.states.clone();
                        // P7.S3o Phase 4 (R5): set `in_materialized_quot`
                        // around both arm body checks so a bare trait member
                        // call inside either materialized arm is rejected.
                        let saved_in_materialized = prov.in_materialized_quot;
                        prov.in_materialized_quot = true;
                        // Slice 10a (R9): the `if`-join's expected
                        // effect is a `QuotEffect` (no row), so both
                        // arms ground to the empty region.
                        check_literal_against_declared_effect(
                            a,
                            eff,
                            false,
                            &[],
                            word,
                            a_span,
                            ctx,
                            env,
                            arrays,
                            cells,
                            refs,
                            slices,
                            prov,
                            scope,
                            poly,
                            &HashSet::new(),
                            LiteralBoundary {
                                shape_changing: false,
                                is_arm: false,
                                caller_tail: false,
                                finalize: false,
                                owning,
                            },
                            None,
                        )?;
                        let a_moves = match owning {
                            true => {
                                check_owning_captures_consumed(
                                    a, a_span, ctx, arrays, prov, scope,
                                )?;
                                Some(std::mem::replace(&mut scope.moves.states, moves_before))
                            }
                            false => None,
                        };
                        check_literal_against_declared_effect(
                            b,
                            eff,
                            false,
                            &[],
                            word,
                            b_span,
                            ctx,
                            env,
                            arrays,
                            cells,
                            refs,
                            slices,
                            prov,
                            scope,
                            poly,
                            &HashSet::new(),
                            LiteralBoundary {
                                shape_changing: false,
                                is_arm: false,
                                caller_tail: false,
                                finalize: false,
                                owning,
                            },
                            None,
                        )?;
                        if let Some(states) = a_moves {
                            check_owning_captures_consumed(b, b_span, ctx, arrays, prov, scope)?;
                            scope.moves =
                                Moves::join(Moves { states }, std::mem::take(&mut scope.moves));
                        }
                        prov.in_materialized_quot = saved_in_materialized;
                        // R23: the merged erased slot's surviving set is
                        // the union of both arms' -- a fresh interned
                        // set, never a mutation of either arm's (keeps
                        // the field `Copy`-compatible).
                        let merged_set = arm_sets
                            .into_iter()
                            .fold(None, |acc, s| prov.union_surviving(acc, Some(s)));
                        // Erased: a runtime `(code, env)` value with a
                        // real quotation type, no `Known` marker.
                        let ty = match owning {
                            true => Type::OwningQuotation(eff),
                            false => Type::Quotation(eff),
                        };
                        (None, Some(ty), merged_set)
                    }
                    _ => return Err(different_quotations_at_join_error(ctx, span)),
                }
            }
            _ => return Err(quotation_versus_value_at_join_error(ctx, span)),
        };
        if erased_ty.is_none() && t_then.ty != t_else.ty {
            return Err(branch_type_mismatch_error(ctx, span, t_then.ty, t_else.ty));
        }
        // The type-only join above already rejects two arms whose
        // stacks disagree in shape; it says nothing about *which place*
        // a live reference's suspension is attributed to. Two arms of
        // identical shape can each suspend a different place (one
        // derives from local `x`, the other from `y`), which the merge
        // must reject rather than silently pick one arm's answer — a
        // later hazard check would then reason about the wrong arm's
        // runtime path. A place is either arm's owned root or the
        // reference local a mutable reborrow suspends: two arms
        // reborrowing *different* reference parameters have no owned
        // root at all and still disagree.
        let deriv = match (t_then.deriv, t_else.deriv) {
            (None, None) => None,
            (Some(a), Some(b)) if prov.deriv(a).suspension() == prov.deriv(b).suspension() => {
                Some(a)
            }
            _ => {
                return Err(borrow_join_disagreement_error(
                    ctx,
                    span,
                    t_then.deriv.map(|id| prov.deriv(id)),
                    t_else.deriv.map(|id| prov.deriv(id)),
                ));
            }
        };
        // A merged slot is a coercible literal only if *both* arms
        // leave a literal there: a value computed on either runtime
        // path is computed after the merge, so it can't silently fill
        // a `usize`/`isize` position without an explicit conversion
        // (D8/X10).
        // Keep every region either arm could have left, since the merge
        // cannot know which one ran: dropping one would let a later
        // borrow of a name bound to the merge mutate storage a live name
        // still denotes on the path that was dropped.
        let alias = match (t_then.alias, t_else.alias) {
            (None, None) => None,
            (Some(a), None) | (None, Some(a)) => Some(a),
            (Some(a), Some(b)) => Some(Alias {
                set: prov.alias_union(a.set, b.set),
                span: a.span,
            }),
        };
        merged.push(Slot {
            // R11: a materialized join slot carries the declared
            // quotation type in place of the arms' `Cstr` placeholder.
            ty: erased_ty.unwrap_or(t_then.ty),
            literal: t_then.literal && t_else.literal,
            // A value merged from two branches is never a single
            // known literal, so it can't feed a compile-time count.
            int_val: None,
            // R3: a merged value loses nullary-variant provenance —
            // either arm may have transformed it.
            variant_idx: None,
            alias,
            deriv,
            // R7: only a marker both arms agree on survives the join.
            quot,
            // R23: the union of both arms' surviving capture sets.
            surviving,
        });
    }
    Ok(merged)
}

/// Slice 10c (R-P3-1/R-P3-1a): `branch`, the machine-level two-way conditional
/// and the single builtin exempt from R11's quotation-operand default-deny.
/// `( ..a u32 ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`: it knows a 32-bit
/// flag, never `bool`, which is what removes the last typed construct from the
/// compiler -- `bool` is an ordinary library enum and `if` an ordinary library
/// word that reads its discriminant with `tag` and branches on the result.
/// Nonzero is true.
///
/// The two branch operands arrive on the stack in both of the forms
/// `resolve_quotation_operand` classifies, and both are load-bearing: literals
/// at every real call site, and abstract `~` parameters while `if`'s own body
/// is checked standalone at its definition.
#[allow(clippy::too_many_arguments)]
fn check_branch(
    mut stack: Vec<Slot>,
    span: Span,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    tail: bool,
    poly: &mut PolyCtx,
    live: &Liveness,
    at: usize,
    siblings: &[Term],
    base_depth: usize,
    outer_releasable: &HashSet<String>,
) -> Result<Vec<Slot>, String> {
    if stack.len() < 3 {
        return Err(underflow_error(ctx, span, "branch", 3, stack.len()));
    }
    let else_slot = stack.pop().expect("branch: else operand");
    let then_slot = stack.pop().expect("branch: then operand");
    let cond = stack.pop().expect("branch: condition");
    if cond.quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, "branch"));
    }
    if cond.ty != Type::U32 {
        return Err(type_mismatch_error(ctx, span, "branch", Type::U32, cond.ty));
    }
    let then_op = resolve_quotation_operand(then_slot)
        .ok_or_else(|| branch_needs_quotation_error(ctx, span, then_slot.ty))?;
    let else_op = resolve_quotation_operand(else_slot)
        .ok_or_else(|| branch_needs_quotation_error(ctx, span, else_slot.ty))?;
    match (then_op, else_op) {
        (QuotOperand::Literal(t), QuotOperand::Literal(e)) => {
            let then_body = prov.quotations[t.0].body.clone();
            let then_end = prov.quotations[t.0].span;
            let else_body = prov.quotations[e.0].body.clone();
            let else_end = prov.quotations[e.0].span;
            // The join's diagnostics are located at the first arm, not at
            // `branch` itself: the arms are the *caller's* literals, while
            // `branch` is reached through `core::bool`'s `if`, whose span
            // would point a user at library source they did not write.
            check_branch_join(
                &then_body,
                ("branch arm", then_end),
                &else_body,
                ("branch arm", else_end),
                stack,
                then_end,
                ctx,
                env,
                arrays,
                cells,
                refs,
                slices,
                prov,
                scope,
                tail,
                poly,
                live,
                at,
                siblings,
                base_depth,
                outer_releasable,
            )
        }
        // R21: at least one arm is an abstract quotation parameter, with a
        // declared effect and no body to splice. This is the standalone
        // def-site check of a word like `if` forwarding its own `~`
        // parameters, so the arm is checked against that declared effect
        // exactly as `f call` is -- both arms declare the same effect, so
        // applying either one's is the same answer.
        (QuotOperand::Forwarded(eff), _) | (_, QuotOperand::Forwarded(eff)) => {
            check_abstract_quotation_call(eff, span, stack, ctx, "branch")
        }
    }
}

/// `branch` handed something that is not a quotation in either branch slot.
fn branch_needs_quotation_error(ctx: &Ctx, span: Span, found: Type) -> String {
    format!(
            "error: type mismatch in {} (line {})\n  `branch` requires two quotation operands, found `{}`\n  note: declared {}",
            ctx.rendered_word(), span.line, found, effect_str(ctx.effect()))
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

/// The borrow-suspension bookkeeping must agree at a branch join, real
/// content the type-only shape unification above does not supply. One arm
/// suspending a place the other leaves unsuspended (or suspending a
/// *different* place) is rejected rather than silently picking one arm's
/// answer, since a later hazard check would then reason about the wrong arm's
/// runtime path.
pub(super) fn borrow_join_disagreement_error(
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
    format!(
            "error: borrow state disagrees at the branch join in {} (line {})\n  the first arm leaves {}, the second arm leaves {}: both arms must agree on which place, if any, stays borrowed past the join\n  note: declared {}",
            ctx.rendered_word(), span.line, describe(t_then), describe(t_else), effect_str(ctx.effect()))
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
/// P7b.S6d-PREREQ (REQ-4d site 5, rule (i)): storing a reference-bearing
/// value through a receiver whose root is not a local of this frame. The
/// structural twin of the surviving-set escape guard, over a channel that has
/// no surviving set: the stored view points into storage that dies with this
/// frame, while the container it lands in does not.
fn stored_view_escapes_frame_error(ctx: &Ctx, span: Span, name: &str, ty: Type) -> String {
    format!(
        "error: `{name}` cannot store the borrow-carrying `{ty}`{} (line {}, col {}): the receiver is not rooted in this frame\n  the container outlives this frame, so the view it would hold points into storage that does not\n  return the view instead, or take the storage as an input",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::GenericTypes;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = crate::lexer::lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::check::check(&mut module)
    }

    /// `check_src` keeping the checked module, so a unit can read back what
    /// the run *minted* and *recorded* rather than only whether it passed.
    fn checked_module(src: &str) -> Module {
        let tokens = crate::lexer::lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        crate::check::check(&mut module).expect("the fixture should check");
        module
    }

    /// The `Res['T 'E]` header every P7b.S11 unit below shares, verbatim from
    /// `probes/dp_*.sth`.
    const RES: &str = "type: Res['T 'E] | Ok 'T | Err 'E ;\n";

    /// P7b.S11 Phase 1 (R-2/R-7), the `env.get`-miss zero-candidate arm: with
    /// no monomorph of `Res` anywhere and nothing pinning `'E`, the parameter
    /// is named as itself. Before this the arm borrowed `unknown word `Ok``,
    /// which blamed the wrong word.
    #[test]
    fn bare_ctor_zero_mints_with_an_unbound_parameter_names_the_parameter() {
        let err = check_src(&format!("{RES}: main ( -- ) 1 Ok drop ;\n"))
            .expect_err("`'E` is determined by nothing here");
        assert!(
            err.contains("`Res['T 'E]`'s type parameter `'E` (parameter 2 of 2)"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("unknown word"), "unexpected message: {err}");
    }

    /// R-7's other half: a name no header claims never reaches the grounding
    /// ladder, so the unchanged `unknown_word_error` still fires and the two
    /// outcomes stay distinguishable.
    #[test]
    fn bare_call_with_no_generic_header_is_still_the_unknown_word_error() {
        let err = check_src(&format!("{RES}: main ( -- ) 1 Nope drop ;\n"))
            .expect_err("`Nope` is defined nowhere");
        assert!(err.contains("unknown word `Nope`"), "unexpected: {err}");
    }

    /// The strict-grounding amendment (260910), witnessed on dp_d's shape:
    /// the operand pins `'T` to `Res[i64 i64]`, but `'E` is determined by
    /// nothing at the call site -- and the sole in-scope mint is never
    /// consulted to fill it, so the located unbound-parameter error names
    /// `'E` (the retired incompatible-grounding diagnostic, which did name
    /// the mint, is gone with the scope consultation).
    #[test]
    fn bare_ctor_whose_operand_leaves_a_parameter_undetermined_names_it_whatever_mints_exist() {
        let err = check_src(&format!(
            "{RES}: mkok ( i64 -- Res[i64 i64] ) Ok ;\n: main ( -- ) 1 mkok Ok drop ;\n"
        ))
        .expect_err("'E` is determined by neither operand nor consumer");
        assert!(
            err.contains("`Res['T 'E]`'s type parameter `'E` (parameter 2 of 2)"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains("instantiation in scope"),
            "scope is never consulted: {err}"
        );
    }

    /// The strict-grounding amendment (260910), witnessed on dp_f's shape: an
    /// unused, uncalled sibling's sole mint is irrelevant -- `'E` is
    /// undetermined at the call site, so the call is the unbound-parameter
    /// error no matter what module scope carries.
    #[test]
    fn bare_ctor_with_a_sole_sibling_mint_and_no_determining_input_is_an_unbound_parameter_error() {
        let err = check_src(&format!(
            "{RES}: unused ( Res[i64 i64] -- ) drop ;\n: main ( -- ) 1 Ok drop ;\n"
        ))
        .expect_err("the sibling's mint is never a parameter source");
        assert!(
            err.contains("type parameter `'E`"),
            "unexpected message: {err}"
        );
    }

    /// R-2's ruling (A), the monomorphic-consumer flavor (dp_g2/dp_g3): the
    /// consumer's declared input pins *both* parameters statically, so a
    /// fully bound θ grounds directly and the two competing mints are never
    /// selected between at all. Acceptance is the proof of which monomorph
    /// was chosen: `only_cstr` takes `Res[i64 cstr]` alone.
    #[test]
    fn bare_ctor_with_a_determining_mono_consumer_grounds_in_either_order() {
        let program = |first: &str, second: &str| {
            format!(
                "{RES}: {first} ;\n: {second} ;\n\
                 : only_cstr ( Res[i64 cstr] -- ) drop ;\n\
                 : main ( -- ) 1 Ok only_cstr ;\n"
            )
        };
        let a = "unused_a ( Res[i64 i64] -- ) drop";
        let b = "unused_b ( Res[i64 cstr] -- ) drop";
        check_src(&program(a, b)).expect("the consumer pins θ, i64-first order");
        check_src(&program(b, a)).expect("the consumer pins θ, cstr-first order");
    }

    /// R-2's other consumer flavor: a *polymorphic* consumer whose declared
    /// input names the header and whose explicit type arguments ground it.
    /// Nothing in the program spells a concrete `Res[...]`, so this is the
    /// zero-mint arm succeeding on a monomorph minted mid-check. The
    /// quotation literal between the two calls is stepped over by the
    /// lookahead, which reads the consumer's input window one slot down.
    #[test]
    fn bare_ctor_grounds_from_a_poly_consumers_explicit_type_arguments() {
        check_src(&format!(
            "{RES}: apply2 ( Res['T 'E] [ i64 -- i64 ] -- i64 ) | f | drop 41 f call ;\n\
             : main ( -- ) 1 Ok [ 1 add ] apply2[i64 i64] drop ;\n"
        ))
        .expect("the poly consumer's type arguments pin both parameters");
    }

    // ------------------------------------------------------------------
    // P7b.S11 Phase 2 (R-6): the explicit-args category.
    // ------------------------------------------------------------------

    /// The category itself: full-arity explicit args on a bare ctor with no
    /// monomorph anywhere mint the named instantiation mid-check. `drop` is
    /// invisible to the consumer lookahead (a builtin returns no constraint),
    /// so acceptance here is attributable to the args alone -- without them
    /// this exact shape is G1's unbound-parameter error.
    #[test]
    fn explicit_args_ctor_with_full_arity_grounds_with_no_mint() {
        check_src(&format!("{RES}: main ( -- ) 1 Ok[i64 i64] drop ;\n"))
            .expect("full-arity args pin every parameter directly");
    }

    /// R-6's arity rule: a prefix list is out of scope, so `Ok[i64]` is a
    /// located error naming the header's full shape rather than a partial
    /// pin. Byte-exact text is pinned by the G4-adjacent integration golden;
    /// this unit pins the mechanism beside the site.
    #[test]
    fn explicit_args_ctor_with_wrong_arity_is_a_located_error() {
        let err = check_src(&format!("{RES}: main ( -- ) 1 Ok[i64] drop ;\n"))
            .expect_err("one arg for two parameters is a prefix pin, out of scope");
        assert_eq!(
            err,
            "error: `Ok` in `main` (line 2) takes 2 type arguments (`Res['T 'E]`), but 1 was supplied"
        );
    }

    /// The category's destructure half: the gate admits the spelling for a
    /// destructure name too, so a wrong-arity list on one is the same located
    /// arity error rather than the generic takes-no-type-arguments text.
    #[test]
    fn explicit_args_destructure_with_wrong_arity_names_the_header() {
        let err = check_src(&format!("{RES}: t ( Res[i64 i64] -- ) Ok>[i64] drop ;\n"))
            .expect_err("the destructure names the same 2-parameter header");
        assert_eq!(
            err,
            "error: `Ok>` in `t` (line 2) takes 2 type arguments (`Res['T 'E]`), but 1 was supplied"
        );
    }

    /// A groundable header is length-free, so a length sublist names nothing
    /// the header declares at any type arity.
    #[test]
    fn explicit_args_length_sublist_on_a_length_free_header_is_rejected() {
        let err = check_src(&format!("{RES}: main ( -- ) 1 Ok[i64 i64 3] drop ;\n"))
            .expect_err("the header declares no length parameters");
        assert_eq!(
            err,
            "error: `Ok` in `main` (line 2) takes no length arguments (`Res['T 'E]` declares type parameters only), but 1 was supplied"
        );
    }

    /// dp_e2's substance: the args ground the construction with **no
    /// consumer at all** -- the word declares the constructed value as its
    /// output, so nothing is forgotten and the ordinary forgetting check is
    /// satisfied. (The consumer-less literal probe shape, `1 Ok[i64 i64] ;`
    /// in a `( -- )` word, is the next unit.)
    #[test]
    fn explicit_args_ctor_grounds_with_no_consumer_and_nothing_forgotten() {
        check_src(&format!(
            "{RES}: main ( -- Res[i64 i64] ) 1 Ok[i64 i64] ;\n"
        ))
        .expect("the construction grounds on the args alone");
    }

    /// The literal dp_e2 probe shape, recorded (spec open question): with the
    /// category admitted, the fully concrete construction grounds and the
    /// value then reaches the ordinary forgetting check, which reports the
    /// leftover value -- the diagnostic dp_e2.sth now produces in place of
    /// the old gate rejection.
    #[test]
    fn dp_e2_literal_no_output_shape_reaches_the_forgetting_check() {
        let err = check_src(&format!("{RES}: main ( -- ) 1 Ok[i64 i64] ;\n"))
            .expect_err("the constructed value is forgotten");
        assert!(
            err.contains("body leaves 1 values"),
            "unexpected message: {err}"
        );
    }

    /// R-8/precedence under explicit args: a competing mint in scope does
    /// not become the verdict -- the fully bound θ names the args'
    /// instantiation, minted fresh beside it, so the call still grounds at
    /// `Res[i64 i64]` and both monomorphs exist.
    #[test]
    fn explicit_args_ctor_with_a_competing_mint_mints_the_args_instantiation() {
        let module = checked_module(&format!(
            "{RES}: unused ( Res[i64 cstr] -- ) drop ;\n\
             : main ( -- ) 1 Ok[i64 i64] drop ;\n"
        ));
        let minted: Vec<&str> = module
            .enums
            .iter()
            .map(|e| e.name.as_str())
            .filter(|n| n.starts_with("Res["))
            .collect();
        assert!(
            minted.contains(&"Res[i64 i64]"),
            "the args' instantiation is minted, not borrowed: {minted:?}"
        );
        assert!(
            minted.contains(&"Res[i64 cstr]"),
            "the competing mint still exists: {minted:?}"
        );
    }

    /// R-2's third consumer: the enclosing word's own declared output, in
    /// tail position. It is the only pin a *zero-field* variant constructor
    /// can have -- it has no operands to infer from -- and without it two
    /// monomorphs of one header make every bare `None` a tie.
    #[test]
    fn zero_field_variant_ctor_grounds_at_the_declared_output() {
        check_src(
            "type: Opt['T] | None | Some 'T ;\n\
             : somei ( i64 -- Opt[i64] ) Some ;\n\
             : someb ( u32 -- Opt[u32] ) Some ;\n\
             : nonei ( -- Opt[i64] ) None ;\n\
             : main ( -- ) 1 somei drop 2 >u32 someb drop nonei drop ;\n",
        )
        .expect("`nonei`'s declared output pins `'T`, with two monomorphs in scope");
    }

    /// The strict-grounding amendment (260910), on the shape that used to
    /// resolve through the retired scope tie-break: the operand is an
    /// `array['T 3]`, and θ's literal-driven step reads only a field that
    /// *is* a bare header variable, so `'T` stays undetermined. Two mints in
    /// scope used to be separated by the exact-operand match; scope is never
    /// consulted now, so the call is the unbound-parameter error naming `'T`
    /// (spell it `Box[i64]` or add a determining consumer to ground it).
    #[test]
    fn bare_ctor_with_an_operand_nested_variable_and_two_mints_names_the_unbound_parameter() {
        let err = check_src(
            "type: Box['T] slot array['T 3] ;\n\
             : take_i ( Box[i64] -- ) Box> drop ;\n\
             : take_u ( Box[u32] -- ) Box> drop ;\n\
             : main ( -- ) 0 3 fill Box drop ;\n",
        )
        .expect_err("the operand's nested `'T` is not read, and scope is not consulted");
        assert!(
            err.contains("`Box['T]`'s type parameter `'T` (parameter 1 of 1)"),
            "unexpected message: {err}"
        );
    }

    /// P7b.S11 Phase 1 (R-8), the mid-check minting case P7.S3t's identity
    /// discipline exists for: two sites grounding the same `(header, θ)`
    /// mid-check must reuse one monomorph, not mint two. Counted on the
    /// checked module's own registry, so a forked mint (a second `Res[i64
    /// i64]` under a different instantiating-module key, which would render
    /// the *same* mangled name and hijack its symbol) fails here rather than
    /// at link time.
    #[test]
    fn mid_check_grounding_reuses_one_monomorph_per_header_and_theta() {
        let module = checked_module(&format!(
            "{RES}: apply2 ( Res['T 'E] [ i64 -- i64 ] -- i64 ) | f | drop 41 f call ;\n\
             : main ( -- )\n  \
               1 Ok [ 1 add ] apply2[i64 i64] drop\n  \
               2 Ok [ 1 add ] apply2[i64 i64] drop ;\n"
        ));
        let minted: Vec<&str> = module
            .enums
            .iter()
            .map(|e| e.name.as_str())
            .filter(|n| n.starts_with("Res["))
            .collect();
        assert_eq!(minted, ["Res[i64 i64]"], "one mint per (header, θ)");
    }

    /// The span-keyed record grounding *collapses a multi-candidate site into
    /// the single-candidate arm* and therefore has to make itself: a bare
    /// generated **struct** word is not an `is_generated_enum_word`, so the
    /// `[only]` arm records nothing for it, and lowering's bare-key map is
    /// last-write-wins across instantiations (`src/ir/layout.rs`). Two
    /// monomorphs exist here and the two sites ground to different ones.
    ///
    /// Measured mutation: drop the S11 half of that arm's condition and
    /// `projection_resolves_per_instantiation` (`tests/phase7_slice1.rs`)
    /// prints a garbage field read for the first site -- a miscompile, not a
    /// missing diagnostic. This unit is that guard's local witness.
    #[test]
    fn grounded_generated_struct_word_records_its_span_keyed_symbol() {
        let module = checked_module(
            "type: S1 a i64 ;\ntype: S2 a i64 b i64 ;\n\
             type: Box['T] val 'T tag i64 ;\n\
             : show1 ( Box[S1] -- ) Box> drop drop ;\n\
             : show2 ( Box[S2] -- ) Box> drop drop ;\n\
             : main ( -- ) 1 S1 11 Box show1 2 3 S2 22 Box show2 ;\n",
        );
        // The two `Box>` destructure sites record through the pre-existing
        // multi-candidate arm and are not this guard's subject.
        let mut pinned: Vec<&str> = module
            .builtin_overloads
            .values()
            .map(|s| s.as_str())
            .filter(|s| s.starts_with("Box[") && !s.ends_with('>'))
            .collect();
        pinned.sort();
        assert_eq!(pinned, ["Box[S1]", "Box[S2]"]);
    }

    /// P7b.S8b's span-keyed pin, which the restructuring had to keep (G6's
    /// gate): a bare generated enum word whose resolution is a function of
    /// the *call site* records its resolved mangled symbol, for the same
    /// last-write-wins reason. Two monomorphs exist here and the two sites
    /// ground to different ones, so a dropped record silently re-types one of
    /// them.
    #[test]
    fn grounded_generated_enum_word_records_its_span_keyed_symbol() {
        let module = checked_module(&format!(
            "{RES}: take_i ( Res[i64 i64] -- ) drop ;\n\
             : take_c ( Res[i64 cstr] -- ) drop ;\n\
             : main ( -- ) 1 Ok take_i 2 Ok take_c ;\n"
        ));
        let mut pinned: Vec<&str> = module
            .builtin_overloads
            .values()
            .map(|s| s.as_str())
            .filter(|s| s.starts_with("Ok["))
            .collect();
        pinned.sort();
        assert_eq!(pinned, ["Ok[i64 cstr]", "Ok[i64 i64]"]);
    }

    /// The header fence: a length-parameterized header is declined outright
    /// by `ctor_grounding_header`, so such a site keeps its pre-S11
    /// resolution and θ never has to reason in the `Len` domain.
    ///
    /// Witnessed by the *outcome*, not by the absence of a message. The
    /// operand is an `array[i64 3]`, so the exact-operand match resolves
    /// `Buf` to `Buf[i64 3]` and the ordinary mismatch against `take_a`'s
    /// declared `Buf[i64 2]` follows -- if the ladder ran, the consumer would
    /// instead have pinned θ to `Buf[i64 2]` and grounded there, and this
    /// exact text could not appear.
    #[test]
    fn length_parameterized_header_is_declined_by_the_grounding_ladder() {
        let err = check_src(
            "type: Buf['T 'N: Len] slot array['T 'N] ;\n\
             : take_a ( Buf[i64 2] -- ) Buf> drop ;\n\
             : take_b ( Buf[i64 3] -- ) Buf> drop ;\n\
             : main ( -- ) 0 3 fill Buf take_a ;\n",
        )
        .expect_err("the operand selects `Buf[i64 3]`, which `take_a` refuses");
        assert!(
            err.contains("`take_a` expected `Buf[i64 2]`, found `Buf[i64 3]`"),
            "a length-parameterized header must not reach the ladder: {err}"
        );
    }

    /// P7 slice 3c (R12): naming a slice local is a *reborrow*, like naming a
    /// reference local -- so a mutable view whose derivation is still live
    /// cannot be named again, and two live mutable sub-views of one buffer are
    /// the coarse borrow table's rejection rather than a silent accept.
    ///
    /// The shared row is what stops the arm from simply suspending every
    /// slice: a `Slice[T]` is `Copy`, so naming one twice is as free as naming
    /// a `&T` twice.
    #[test]
    fn naming_a_mutable_slice_local_reborrows_and_suspends_its_place() {
        check_src(
            ": main ( -- )\n  0 4 fill | buf |\n  &buf slice | s |\n  \
             s 0 >usize 2 >usize subslice | a |\n  \
             s 2 >usize 2 >usize subslice | b |\n  \
             a len drop b len drop\n  buf drop\n;\n",
        )
        .expect("two live shared sub-views of one buffer conflict with nothing");
        let err = check_src(
            ": main ( -- )\n  0 4 fill | buf |\n  &!buf slice | s |\n  \
             s 0 >usize 2 >usize subslice | a |\n  \
             s 2 >usize 2 >usize subslice | b |\n  \
             a len drop b len drop\n  buf drop\n;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot reborrow `s` in `main` while a reference derived from it is live"),
            "unexpected message: {err}"
        );
    }

    /// Slice 10c (R-P3-1a): `branch`'s two branch operands arrive in *both*
    /// forms `resolve_quotation_operand` classifies, and both are load-bearing.
    ///
    /// The first is the ordinary one: literals at a real call site. The second
    /// is the R21 abstract-forward case, which only a word's own definition
    /// exercises -- `myif`'s body forwards its declared `~` parameters into
    /// `branch`, and at *its* definition site those are abstract quotation
    /// slots with no interned body. Measured mutation: route `branch`'s arm
    /// through the `QuotRef::Known` case alone and `core::bool`'s own `if`
    /// stops checking, so *nothing* builds -- the abstract-forward case is not
    /// an edge case, it is the case the whole slice rests on.
    #[test]
    fn check_branch_accepts_a_literal_and_a_forwarded_quotation_operand() {
        check_src(": w ( i64 i64 -- i64 ) ueq [ 1 ] [ 2 ] branch ;\n: main ( -- ) 1 2 w drop ;\n")
            .expect("two quotation literals splice at the call site");
        check_src(
            ": myif inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )\n  \
             | e | | t | | c | c tag t e branch ;\n\
             : main ( -- ) 1 2 eq ~[ 7 ] ~[ 8 ] myif drop ;\n",
        )
        .expect("`myif`'s own definition forwards its abstract `~` parameters into `branch`");
    }

    /// The third, *mixed* shape: one literal arm and one forwarded parameter.
    /// The `Forwarded` arm wins the match, so the declared effect is applied
    /// and the literal arm is never walked at all -- `[ 999 888 ]` disagrees
    /// with the declared `~[ -- i64 ]` and `w`'s own definition still passes.
    ///
    /// Recorded rather than fixed. It is not unsound: nothing reaches the
    /// unchecked body without a real caller, and the caller's splice checks
    /// both arms for real (asserted below). The fix would be to check the
    /// literal against the sibling's declared effect via
    /// `check_literal_against_declared_effect`, which is exactly the helper
    /// the spec's out-of-scope section documents as re-splicing without bound
    /// on a combinator's own body -- this shape -- overflowing the stack with
    /// no diagnostic. So the def-site check is *partial* here, and the test
    /// says so instead of reading as full coverage.
    #[test]
    fn check_branch_leaves_a_literal_arm_unchecked_beside_a_forwarded_one() {
        check_src(
            ": w inline ( u32 ~[ -- i64 ] -- i64 ) | t | t [ 999 888 ] branch ;\n\
             : main ( -- ) ;\n",
        )
        .expect("the mismatched literal arm goes unchecked at `w`'s own definition");
        let err = check_src(
            ": w inline ( u32 ~[ -- i64 ] -- i64 ) | t | t [ 999 888 ] branch ;\n\
             : main ( -- ) 1 2 ueq ~[ 5 ] w drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("the two branch arms leave different stack depths"),
            "a real caller splices both arms and catches it: {err}"
        );
    }

    /// `branch` is the *single* builtin exempt from R11's quotation-operand
    /// default-deny, and only for its two branch slots: a quotation in the
    /// condition position is still rejected, naming `branch`.
    #[test]
    fn check_branch_rejects_a_quotation_condition_and_a_non_quotation_arm() {
        let cond = check_src(": main ( -- ) [ add ] [ 1 ] [ 2 ] branch drop ;\n").unwrap_err();
        assert!(
            cond.contains("`branch`") && cond.contains("cannot take a quotation as an operand"),
            "unexpected message: {cond}"
        );
        let arm = check_src(": main ( -- ) 1 2 ueq 3 [ 2 ] branch drop ;\n").unwrap_err();
        assert!(
            arm.contains("`branch` requires two quotation operands"),
            "unexpected message: {arm}"
        );
        let flag = check_src(": main ( -- ) True [ 1 ] [ 2 ] branch drop ;\n").unwrap_err();
        assert!(
            flag.contains("`branch`") && flag.contains("`u32`"),
            "`branch` knows a 32-bit flag, not `Bool`: {flag}"
        );
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

    /// P7b.S6d-PREREQ (Deferred, the back-edge note): `back_edge_outs` above
    /// forwards `surviving` alone and drops `deriv`, a latent twin of the
    /// dispatch-push laundering -- unreachable only because
    /// `check_reference_across_back_edge` rejects a deriv-carrying argument
    /// first. Now that REQ-4d propagates a deriv onto a slice-bearing
    /// aggregate, this test asserts the *rejection*, not the forward: if that
    /// guard is ever narrowed, the hole opens with nothing else watching.
    ///
    /// It is also the stated capability boundary: a tainted `Window` can never
    /// cross a self-tail back edge, which S6d's loop-shaped consumers will
    /// meet until a loop-aware borrow story exists.
    #[test]
    fn back_edge_rejects_a_deriv_carrying_aggregate_argument() {
        let err = check_src(
            "type: Window view Slice[i64] lo usize ;\n\
             : sum inline ( Window i64 -- i64 )\n  | w acc |\n  \
             acc 0 gt ~[ w acc 1 sub sum ] ~[ acc ] if\n;\n\
             : main ( -- ) 0 4 fill |a| &a slice 0 >usize Window 3 sum drop a drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("a reference to a local cannot cross a loop")
                && err.contains("a reference derived from `a`"),
            "{err}"
        );
    }

    /// A scratch `MutRegistries` over empty `arrays`/`cells`/`refs`, mirroring
    /// `ast.rs`'s test-only `ScratchRegs` (private there) -- kept live across
    /// a test's calls so an interned shape from one instantiation is visible
    /// to the next.
    #[derive(Default)]
    struct ScratchRegs {
        arrays: Vec<ArrayDecl>,
        cells: Vec<OwnedCellDecl>,
        refs: Vec<RefDecl>,
    }

    impl ScratchRegs {
        fn regs(&mut self) -> crate::ast::MutRegistries<'_> {
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut self.arrays,
                cells: &mut self.cells,
                refs: &mut self.refs,
            }
        }
    }

    fn generic_enum_decl(name: &str, variant: &str) -> crate::ast::GenericEnumDecl {
        crate::ast::GenericEnumDecl {
            name: name.to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: Vec::new(),
            len_var_names: vec![],
            variants: vec![crate::ast::GenericVariantDecl {
                name: variant.to_string(),
                fields: vec![("val".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        }
    }

    /// P7.S11-follow (Part 4, happy path): a pending, unflushed enum mint's
    /// generated constructor is found by name, id-correct (its own `EnumId`,
    /// not `from_index(0)`).
    #[test]
    fn mint_fallback_candidates_finds_an_unflushed_constructor() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
        generics.enums.push(generic_enum_decl("Result", "Ok"));
        let mut scratch = ScratchRegs::default();
        let minted = generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let Type::Enum(id, _) = minted else {
            panic!("expected a Type::Enum")
        };
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let candidates = mint_fallback_candidates("Ok", &ctx);
        assert_eq!(
            candidates.len(),
            1,
            "expected exactly the one pending mint's constructor"
        );
        assert_eq!(
            candidates[0].sig.outputs,
            vec![Type::Enum(id, "Result[i64]")]
        );
    }

    /// P7.S11-follow (Part 4, edge case): nothing pending -- an empty live
    /// cell, or no `generics` at all -- returns no candidates.
    #[test]
    fn mint_fallback_candidates_empty_when_nothing_pending() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let generics = GenericTypes::with_bases(structs.len(), enums.len());
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        assert!(mint_fallback_candidates("Ok", &ctx).is_empty());

        let ctx_no_generics = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            None,
        );
        assert!(mint_fallback_candidates("Ok", &ctx_no_generics).is_empty());
    }

    /// P7.S11-follow (Part 4, name-collision probe): variant-ctor env keys
    /// are module-blind, so two pending mints in one live cell can in
    /// principle generate the same surface name -- built unconditionally
    /// (a hand-crafted `GenericTypes` is always constructible), asserting
    /// *both* overloads are returned. Dispatch over the pair is first-wins
    /// per the downstream-dispatch ruling; this test pins the return, not
    /// the selection.
    #[test]
    fn mint_fallback_candidates_at_a_colliding_name_returns_both() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
        generics.enums.push(generic_enum_decl("Left", "Same"));
        generics.enums.push(generic_enum_decl("Right", "Same"));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        generics.instantiate_enum(1, &[Type::I64], &[], 0, scratch.regs());
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let candidates = mint_fallback_candidates("Same", &ctx);
        assert_eq!(
            candidates.len(),
            2,
            "expected both same-surface-name pending mints, not a last-write truncation"
        );
    }

    /// P7b.S6d (REQ-4, the S8b-class clobber G7 pins, unit): a bare variant
    /// word must resolve to its OWN monomorph when a second monomorph of the
    /// same generated enum is live. `touch`'s signature mention mints
    /// `Step[i64 cstr]` at parse (flushed into the ordinary registries by
    /// `parse` itself, so `env` carries its `More>` candidate);
    /// `1 pack` instantiates the poly helper at `i64` mid-`main`, minting
    /// `Step[i64 i64]` into the live cell (pending, invisible to `env`).
    /// The bare `More>` in the More arm sits on the pending monomorph's
    /// narrowed variant; before the env-hit union it resolved to the only
    /// env candidate (`Step[i64 cstr]`'s) and the sig check rejected the
    /// site -- byte-for-byte the `s6d_m` repro's shape (`More>` expected
    /// `Step[i64 Slice[i64]].More`, found `Step[i64 List[i64]].More`).
    /// Post-fix the union offers both candidates, the operand filter picks
    /// the mint, and the span-keyed record pins the site to the i64
    /// monomorph's own symbol.
    #[test]
    fn variant_word_resolution_survives_a_second_monomorph_of_the_same_enum() {
        let module = checked_module(
            "type: Step['T 'Rest] | Done | More 'T 'Rest ;\n\
             : touch ( Step[i64 cstr] -- ) drop ;\n\
             : pack ['R] ( 'R -- Step[i64 'R] ) drop Done ;\n\
             : main ( -- )\n\
             1 pack\n\
             ~[ ( Done ) drop ]\n\
             ~[ ( More ) More> drop drop ]\n\
             Step? ;\n",
        );
        assert_eq!(
            module.builtin_overloads.len(),
            1,
            "the bare-name site records on exactly one span: {:?}",
            module.builtin_overloads
        );
        assert_eq!(
            module.builtin_overloads.values().next(),
            Some(&"More[i64 i64]>".to_string()),
            "the site records the check-time monomorph's own symbol, never \
             the parse-time mint's: {:?}",
            module.builtin_overloads
        );
    }

    /// P7b.S8b Phase 1 (R5, happy path): a generated enum word is recognised
    /// by `(name, symbol)` membership over the extended type slices, for the
    /// constructor and for its destructure twin -- the two sources the
    /// single-candidate arm's new record has to cover. The mint here is
    /// deliberately *unflushed*, the same state `mint_fallback_candidates`
    /// serves from, since that is the route the record's symbol can arrive
    /// by.
    #[test]
    fn is_generated_enum_word_identifies_a_pending_variant_constructor_and_destructure() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
        generics.enums.push(generic_enum_decl("Result", "Ok"));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let ctors = mint_fallback_candidates("Ok", &ctx);
        let [ctor] = ctors.as_slice() else {
            panic!("expected exactly the one pending mint's constructor")
        };
        assert_eq!(ctor.symbol, "Ok[i64]", "the mangled registry spelling");
        assert!(is_generated_enum_word("Ok", ctor, &ctx));
        let destructures = mint_fallback_candidates("Ok>", &ctx);
        let [destructure] = destructures.as_slice() else {
            panic!("expected exactly the one pending mint's destructure")
        };
        assert!(is_generated_enum_word("Ok>", destructure, &ctx));
    }

    /// P7b.S8b Phase 1 (R5, negative): membership is on `(name, symbol)`, not
    /// on the name alone -- a user word that happens to share a variant's
    /// surface name is not a generated enum word, so it takes none of the
    /// span-keyed record and keeps resolving by name at lowering.
    #[test]
    fn is_generated_enum_word_rejects_a_user_word_sharing_the_surface_name() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
        generics.enums.push(generic_enum_decl("Result", "Ok"));
        let mut scratch = ScratchRegs::default();
        generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let user_word = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![Type::I64],
            },
            symbol: "Ok__m0".to_string(),
            module: 0,
        };
        assert!(!is_generated_enum_word("Ok", &user_word, &ctx));
    }

    /// P7b.S8b Phase 1 (R5, review round FIX 3): the single-candidate arm's
    /// new record lands the resolved mangled symbol in `module.builtin_
    /// overloads`, not merely a boolean predicate -- a full `check()` pass
    /// over a mono body constructing a generated enum word, read back the
    /// same way `checked_like_a_build` hands a `Module` to its callers
    /// (`poly.rs:12248`).
    #[test]
    fn is_generated_enum_word_site_records_its_mangled_symbol_in_builtin_overloads() {
        let mut module = crate::test_support::parse_with_core(
            &crate::lexer::lex(
                "type: Opt['T] | None | Some 'T ;\n\
                 : mkopt ( i64 -- Opt[i64] ) Some ;\n\
                 : main ( -- ) 1 mkopt drop ;\n",
            )
            .unwrap(),
        )
        .unwrap();
        crate::check::check(&mut module).expect("the mkopt fixture checks");
        assert_eq!(
            module.builtin_overloads.len(),
            1,
            "the record lands on exactly one span (key coverage): {:?}",
            module.builtin_overloads
        );
        assert!(
            module
                .builtin_overloads
                .values()
                .any(|symbol| symbol == "Some[i64]"),
            "the construction site records its resolved mangled symbol: {:?}",
            module.builtin_overloads
        );
    }

    /// P7.S11-follow (Part 3, happy path): a scrutinee whose `Type::Enum` id
    /// names a real, unflushed pending mint resolves.
    #[test]
    fn scrutinee_enum_id_of_family_reads_a_minted_scrutinee() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
        generics.enums.push(generic_enum_decl("Result", "Ok"));
        let mut scratch = ScratchRegs::default();
        let minted = generics.instantiate_enum(0, &[Type::I64], &[], 0, scratch.regs());
        let Type::Enum(id, name) = minted else {
            panic!("expected a Type::Enum")
        };
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let stack = vec![Slot::computed(Type::Enum(id, name))];
        let prov = Provenance::default();
        assert_eq!(
            scrutinee_enum_id_of_family(&stack, &prov, &[], &ctx),
            Some(id)
        );
    }

    /// P7.S11-follow (Part 3, the permissiveness guard): a concrete-looking
    /// `Type::Enum` whose id resolves to *nothing* -- neither the flushed
    /// slice nor the live cell's pending tail -- must not be treated as
    /// grounded. A poly call's own unification can leave such a type on the
    /// stack independent of whether anything actually minted it.
    #[test]
    fn scrutinee_enum_id_of_family_none_when_type_looks_concrete_but_unminted() {
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let generics = GenericTypes::with_bases(structs.len(), enums.len());
        let cell = RefCell::new(generics);
        let word = crate::test_support::bare_word("main", 0);
        let ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let unminted_id = EnumId::from_index(0);
        let stack = vec![Slot::computed(Type::Enum(unminted_id, "Result[i64]"))];
        let prov = Provenance::default();
        assert_eq!(scrutinee_enum_id_of_family(&stack, &prov, &[], &ctx), None);
    }

    /// P7b.S9 Phase 2 (R1.1a): a generic struct header declared separately in
    /// two modules under the same bare name, mirroring `pb2`'s `a`/`b`
    /// `Widget['T]`. `ty_vars`/`fields` vary per fixture: the point of most of
    /// these units is that two same-named headers need not agree on either.
    fn generic_struct_decl(
        name: &str,
        module: u32,
        ty_vars: &[&str],
        fields: &[(&str, PolyType)],
    ) -> crate::ast::GenericStructDecl {
        crate::ast::GenericStructDecl {
            name: name.to_string(),
            ty_var_names: ty_vars.iter().map(|v| v.to_string()).collect(),
            ty_kinds: Vec::new(),
            len_var_names: vec![],
            fields: fields
                .iter()
                .map(|(f, ty)| (f.to_string(), ty.clone()))
                .collect(),
            span: Span::default(),
            module,
        }
    }

    /// The `pb2` grounding fixture: module 3 (the caller) declares
    /// `Widget['T] v 'T`, module 4 declares a *differently shaped*
    /// `Widget['T] v 'T w i64`, and only module 4's is eagerly minted. Hands
    /// back the live cell and module 4's mint.
    fn two_header_widget_cell() -> (RefCell<GenericTypes>, Type) {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            3,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[
                ("v", PolyType::Var(0)),
                ("w", PolyType::Concrete(Type::I64)),
            ],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 4, scratch.regs());
        (RefCell::new(generics), borrowed)
    }

    /// Module 4's generated constructor (or, with `destructure`, its
    /// destructure) of `borrowed` -- the single `env` candidate a bare call in
    /// module 3 actually sees.
    fn borrowed_widget_candidate(borrowed: Type, destructure: bool) -> Overload {
        let fields = vec![Type::I64, Type::I64];
        match destructure {
            true => Overload {
                sig: Sig {
                    inputs: vec![borrowed],
                    outputs: fields,
                },
                symbol: "Widget[i64]>".to_string(),
                module: 4,
            },
            false => Overload {
                sig: Sig {
                    inputs: fields,
                    outputs: vec![borrowed],
                },
                symbol: "Widget[i64]".to_string(),
                module: 4,
            },
        }
    }

    fn ground_in_module_3(
        only: &Overload,
        name: &str,
        cell: &RefCell<GenericTypes>,
        structs: &[StructDecl],
    ) -> Result<Option<Overload>, String> {
        ground_in_module_3_view(only, name, cell, structs, None)
    }

    /// The `ground_in_module_3` harness with the caller's import view
    /// supplied. P7b.S10's units exercise paths only reachable with
    /// `ctx.modules = Some(..)`: the default harness passes `None`, and the
    /// ambiguity check reads it and never fires when it is absent (R2's
    /// D1-gate discipline).
    fn ground_in_module_3_view(
        only: &Overload,
        name: &str,
        cell: &RefCell<GenericTypes>,
        structs: &[StructDecl],
        modules: Option<&[ModuleInfo]>,
    ) -> Result<Option<Overload>, String> {
        let word = crate::test_support::bare_word("run", 3);
        let ctx = word_ctx(
            &word,
            structs,
            &[],
            &[],
            modules,
            &CombinatorIndex::new(),
            Some(cell),
        );
        let span = Span {
            line: 4,
            col: 1,
            module: 3,
        };
        bare_generated_word_own_module_grounding(
            only,
            name,
            span,
            &ctx,
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
        )
    }

    /// P7b.S9 Phase 2 (R1.1a, happy path): the Phase-1 verdict's exact shape --
    /// two modules each declare their own `Widget['T]` header, only one has
    /// been eagerly minted (module 4's), and the caller (module 3) is
    /// grounding a bare `Widget` ctor call. The caller's own header must be
    /// minted (or found) and used, never the borrowed candidate -- *including*
    /// its `Sig`: the two headers here have different field lists, so a
    /// grounded ctor keeping the borrowed candidate's `inputs` would take two
    /// operands into a one-field struct and silently forget one.
    #[test]
    fn bare_ctor_operand_provenance_is_callers_own_header_not_a_borrowed_mint() {
        let (cell, borrowed) = two_header_widget_cell();
        let Type::Struct(borrowed_id, _) = borrowed else {
            panic!("expected a Type::Struct")
        };
        let only = borrowed_widget_candidate(borrowed, false);
        let grounded = ground_in_module_3(&only, "Widget", &cell, &[])
            .expect("the two headers agree on parameter count, so this must not error")
            .expect("the caller's own module declares its own header, so this must ground");
        assert_eq!(
            grounded.module, 3,
            "the grounded candidate's module must be the caller's own, not the borrowed mint's"
        );
        let Type::Struct(grounded_id, _) = grounded.sig.outputs[0] else {
            panic!("expected a Type::Struct")
        };
        assert_ne!(
            grounded_id, borrowed_id,
            "the caller must get its own instantiation, never the borrowed module's id"
        );
        assert_eq!(
            grounded.sig.inputs,
            vec![Type::I64],
            "the ctor's inputs are the caller's own header's fields, not the borrowed decl's"
        );
        assert_eq!(
            grounded.symbol, "Widget[i64]",
            "the lowering symbol is the caller's own minted decl's name"
        );
        let guard = cell.borrow();
        let minted = guard
            .struct_decl(grounded_id)
            .expect("the caller's own mint is still pending in the live cell");
        assert_eq!(
            minted.fields,
            vec![("v".to_string(), Type::I64)],
            "the minted decl is the caller's own one-field header at `i64`"
        );
    }

    /// P7b.S9 Phase 2 (R1.1a): the constructor's sibling. The caller's own
    /// mint is minted mid-word and has no `env` entry, so a destructure of a
    /// caller-grounded operand sees only the borrowed module's generated word
    /// -- and that one's input is the *other* `Widget[i64]`. It grounds by the
    /// same rule, with the caller's own fields as its outputs.
    #[test]
    fn bare_destructure_grounds_at_the_callers_own_header_too() {
        let (cell, borrowed) = two_header_widget_cell();
        let only = borrowed_widget_candidate(borrowed, true);
        let grounded = ground_in_module_3(&only, "Widget>", &cell, &[])
            .expect("no arity mismatch here")
            .expect("the caller's own module declares its own header, so this must ground");
        assert_eq!(grounded.module, 3);
        assert_eq!(
            grounded.symbol, "Widget[i64]>",
            "the destructure's lowering symbol is the caller's own decl name plus `>`"
        );
        assert_eq!(
            grounded.sig.outputs,
            vec![Type::I64],
            "a destructure yields the caller's own header's fields"
        );
        assert_ne!(
            grounded.sig.inputs, only.sig.inputs,
            "and it consumes the caller's own instantiation, not the borrowed one"
        );
    }

    /// P7b.S9 Phase 2 (R1.1a): the caller's own header cannot be applied to
    /// the borrowed mint's argument list at all -- `substitute_generic_field`
    /// indexes it raw, so minting would panic. A located error, never a
    /// silent fall-back to the borrowed mint.
    #[test]
    fn bare_ctor_own_header_of_a_different_arity_is_error() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            3,
            &["'T", "'U"],
            &[("v", PolyType::Var(0)), ("w", PolyType::Var(1))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![borrowed],
            },
            symbol: "Widget[i64]".to_string(),
            module: 4,
        };
        let err = ground_in_module_3(&only, "Widget", &cell, &[])
            .expect_err("a header of a different arity cannot ground here");
        assert_eq!(
            err,
            "error: `Widget` in `run` (line 4) cannot ground at this module's own header: it declares 2 type parameters, but the only `Widget` instantiation in scope supplies 1\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's"
        );
    }

    /// P7b.S9 Phase 2 (R1.1a): the counts agree and the *kinds* do not -- the
    /// caller's own header binds `'F` at `* -> *` and applies it in a field,
    /// the borrowed mint supplies a plain `i64`.
    /// `substitute_generic_field`'s `App` arm returns a non-`CtorImage`
    /// binding unapplied (its own doc calls that fall-back unreachable,
    /// because `validate_ctor_arg_kinds` rejects a kind mismatch at the use
    /// site -- but the use site here is another module's), so without this
    /// guard the field silently takes a wrong type.
    #[test]
    fn bare_ctor_own_header_of_a_higher_kind_is_error() {
        let mut generics = GenericTypes::with_bases(0, 0);
        let mut own = generic_struct_decl(
            "Widget",
            3,
            &["'F"],
            &[(
                "v",
                PolyType::App {
                    head: 0,
                    args: vec![PolyType::Concrete(Type::I64)],
                },
            )],
        );
        own.ty_kinds = vec![crate::ast::Kind::Arrow {
            domains: vec![crate::ast::Kind::Star],
            result: Box::new(crate::ast::Kind::Star),
        }];
        generics.structs.push(own);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![borrowed],
            },
            symbol: "Widget[i64]".to_string(),
            module: 4,
        };
        let err = ground_in_module_3(&only, "Widget", &cell, &[])
            .expect_err("a header of a higher kind cannot ground at a concrete argument");
        assert_eq!(
            err,
            "error: `Widget` in `run` (line 4) cannot ground at this module's own header: its `'F` takes a type constructor, but the only `Widget` instantiation in scope supplies the concrete type `i64`\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's"
        );
    }

    /// P7b.S9 Phase 2 (R1.1a): the length-argument twin of the arity error --
    /// the caller's own header binds a length variable the borrowed mint's
    /// argument list has no value for, which `substitute_generic_field`'s
    /// `Len::Var` arm would index past.
    #[test]
    fn bare_ctor_own_header_needing_a_length_argument_is_error() {
        let mut generics = GenericTypes::with_bases(0, 0);
        let mut own = generic_struct_decl("Widget", 3, &["'T"], &[("v", PolyType::Var(0))]);
        own.len_var_names = vec!["'N".to_string()];
        generics.structs.push(own);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![borrowed],
            },
            symbol: "Widget[i64]".to_string(),
            module: 4,
        };
        let err = ground_in_module_3(&only, "Widget", &cell, &[])
            .expect_err("a header needing a length argument cannot ground here");
        assert_eq!(
            err,
            "error: `Widget` in `run` (line 4) cannot ground at this module's own header: it declares 1 length parameter, but the only `Widget` instantiation in scope supplies 0\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's"
        );
    }

    /// P7b.S9 Phase 2 (R1.1a, edge case): the caller's own module already
    /// owns the single candidate (the ordinary, non-cross-module case) --
    /// nothing to ground, the borrowed candidate is used unchanged.
    #[test]
    fn bare_ctor_own_module_grounding_none_when_caller_already_owns_the_mint() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            3,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let minted = generics.instantiate_struct(0, &[Type::I64], &[], 3, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![minted],
            },
            symbol: "Widget[i64]".to_string(),
            module: 3,
        };
        assert!(ground_in_module_3(&only, "Widget", &cell, &[])
            .expect("no error on the ordinary same-module case")
            .is_none());
    }

    /// P7b.S9 Phase 2 (R1.1a): the discriminating case for the owning-module
    /// check. Two same-named headers exist (module 3's and module 4's) and the
    /// caller is module 3, but the *mint* was keyed to module 3 already -- so
    /// there is nothing to re-ground even though `find_struct` finds a
    /// different header index for this module. Dropping the owning-module
    /// comparison flips this to a spurious second mint of module 3's own
    /// header at another header's field shape.
    #[test]
    fn bare_ctor_own_module_grounding_none_when_the_mint_is_already_the_callers_own_module() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            3,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        // Module 4's *header* (gi = 1), minted under module 3.
        let minted = generics.instantiate_struct(1, &[Type::I64], &[], 3, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![minted],
            },
            symbol: "Widget[i64]".to_string(),
            module: 3,
        };
        assert!(ground_in_module_3(&only, "Widget", &cell, &[])
            .expect("no error")
            .is_none());
    }

    /// P7b.S9 Phase 2 (R1.1a): a caller with no header of its own under this
    /// name has nothing to ground at -- Phase 4's D3 territory. Pinned
    /// existing behaviour: the borrowed candidate is used unchanged.
    #[test]
    fn bare_ctor_own_module_grounding_none_when_the_caller_declares_no_header() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![borrowed],
            },
            symbol: "Widget[i64]".to_string(),
            module: 4,
        };
        assert!(ground_in_module_3(&only, "Widget", &cell, &[])
            .expect("no error")
            .is_none());
    }

    /// P7b.S9 Phase 2 (R1.1a): an ordinary *user* word whose output happens to
    /// be another module's instantiation is not a generated word of it, and
    /// keeps its own resolution -- re-grounding it would rewrite a call to a
    /// word in `b` into a construction in `a`. The candidate here is reached
    /// under the header's own surface name, so only the symbol tells the two
    /// apart.
    #[test]
    fn bare_ctor_own_module_grounding_none_for_a_user_word_returning_the_borrowed_mint() {
        let (cell, borrowed) = two_header_widget_cell();
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64, Type::I64],
                outputs: vec![borrowed],
            },
            symbol: "makeit__m4".to_string(),
            module: 4,
        };
        assert!(ground_in_module_3(&only, "Widget", &cell, &[])
            .expect("no error")
            .is_none());
    }

    /// P7b.S9 Phase 2 (R1.1a): the candidate's mint is of the caller's *own*
    /// header, keyed to another module (an application written at a use site
    /// elsewhere). There is nothing to re-ground: minting again under the
    /// caller's module would hand the same header at the same arguments a
    /// second, distinct `StructId` -- the very identity split this fix exists
    /// to close.
    #[test]
    fn bare_ctor_own_module_grounding_none_when_the_mint_is_the_callers_own_header() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            3,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let minted = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![minted],
            },
            symbol: "Widget[i64]".to_string(),
            module: 4,
        };
        assert!(ground_in_module_3(&only, "Widget", &cell, &[])
            .expect("no error")
            .is_none());
    }

    // ===== P7b.S10 (R1/R2/R3) units: the headerless single-candidate
    // grounding arm's exemptions and its two error shapes. All of these run
    // through `ground_in_module_3_view` with `ctx.modules = Some(..)`: the
    // default harness passes `None`, which never reaches the new check at
    // all (R2's absent-import-view discipline). Module layout convention
    // matches the S9 units: the caller is module 3, foreign modules are 4+.

    /// The single env candidate: `minter`'s eagerly minted `Widget[i64]`
    /// constructor (one `i64` field), the only entry a bare `Widget` call in
    /// the headerless caller module sees.
    fn widget_ctor_candidate(borrowed: Type, minter: u32) -> Overload {
        Overload {
            sig: Sig {
                inputs: vec![Type::I64],
                outputs: vec![borrowed],
            },
            symbol: "Widget[i64]".to_string(),
            module: minter,
        }
    }

    /// A `ModuleInfo` view for the S10 units: `imports` are plain
    /// `import: q ;` bindings (qualifier -> target), `named` are explicit
    /// `| name |` selective entries, `wild` are `*`-wildcard-desugared
    /// entries -- they populate `selective` identically but never
    /// `named_selective` (the assembly-time distinction the raw map cannot
    /// recover).
    fn module_view(
        imports: &[(&str, u32)],
        named: &[(&str, u32)],
        wild: &[(&str, u32)],
    ) -> ModuleInfo {
        let mut selective = HashMap::new();
        let mut named_selective = HashMap::new();
        for (n, t) in named {
            selective.insert(n.to_string(), *t);
            named_selective.insert(n.to_string(), *t);
        }
        for (n, t) in wild {
            selective.insert(n.to_string(), *t);
        }
        ModuleInfo {
            imports: imports.iter().map(|(q, t)| (q.to_string(), *t)).collect(),
            exports: Vec::new(),
            selective,
            named_selective,
            intrinsics: crate::ast::IntrinsicVisibility::None,
        }
    }

    /// A minimal view list for `count` modules: the caller's view at 3 and
    /// `Default::default()` everywhere else (the walks consult only the
    /// modules a fixture gives qualifiers to).
    fn module_views(caller: ModuleInfo, count: usize) -> Vec<ModuleInfo> {
        let mut modules = vec![ModuleInfo::default(); count];
        modules[3] = caller;
        modules
    }

    /// Two same-named `Widget['T]` headers, declared in modules 4 and 5, and
    /// module 4's eager mint of `Widget[i64]` -- the GA-shape program seen
    /// from a headerless caller in module 3.
    fn two_foreign_header_cell() -> (RefCell<GenericTypes>, Type) {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            5,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        (RefCell::new(generics), borrowed)
    }

    /// R1 (GA's shape at unit level): headerless caller, two same-named
    /// headers from reachable distinct modules, one env candidate -- the
    /// grounding path errors instead of falling through to the borrowed
    /// mint.
    #[test]
    fn ambiguous_foreign_headers_grounding_is_located_error() {
        let (cell, borrowed) = two_foreign_header_cell();
        let only = widget_ctor_candidate(borrowed, 4);
        let modules = module_views(module_view(&[("a", 4), ("b", 5)], &[], &[]), 6);
        let err = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect_err("two reachable headers must not silently borrow the single mint");
        assert!(
            err.contains("is ambiguous: declared in modules"),
            "unexpected message: {err}"
        );
    }

    /// R1 exemption 2's permissive half (GD's shape at unit level): a
    /// foreign single candidate whose declaring module is reachable and the
    /// only same-named header anywhere -- the existing borrow, unchanged.
    #[test]
    fn single_foreign_header_grounding_still_borrows() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 4);
        let modules = module_views(module_view(&[("lib", 4)], &[], &[]), 5);
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("one reachable header whose declaring module mints it exempts the call");
        assert!(
            grounded.is_none(),
            "the exemption borrows the candidate unchanged, never re-grounds"
        );
    }

    /// R1 exemption 1 (the caller-owns tier, untouched): the caller declares
    /// its own header, so R1.1a grounds the call at its own mint no matter
    /// how many foreign same-named headers its imports can reach.
    #[test]
    fn own_header_still_grounded_first() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            3,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            5,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 4);
        let modules = module_views(module_view(&[("a", 4), ("b", 5)], &[], &[]), 6);
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("the two headers agree on parameter count, so this must not error")
            .expect("the caller's own header grounds the call before any exemption runs");
        assert_eq!(
            grounded.module, 3,
            "the grounded candidate is the caller's own mint, never the borrowed one"
        );
    }

    /// Three same-named `Widget['T]` headers, declared in modules 4, 5, and
    /// 6, and module 4's eager mint of `Widget[i64]` -- the 3+-reachable
    /// shape no golden exercises (reviewer note carried forward from the
    /// S10 review).
    fn three_foreign_header_cell() -> (RefCell<GenericTypes>, Type) {
        let mut generics = GenericTypes::with_bases(0, 0);
        for module in [4u32, 5, 6] {
            generics.structs.push(generic_struct_decl(
                "Widget",
                module,
                &["'T"],
                &[("v", PolyType::Var(0))],
            ));
        }
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        (RefCell::new(generics), borrowed)
    }

    /// R3/R4: the rendered ambiguity message contains the surface name, both
    /// declaring modules (lexicographically ordered), and the call site.
    #[test]
    fn ambiguous_header_error_names_declaring_modules() {
        let (cell, borrowed) = two_foreign_header_cell();
        let only = widget_ctor_candidate(borrowed, 4);
        // The qualifiers are bound in reverse lexicographic order on purpose:
        // the message must sort them regardless of import order.
        let modules = module_views(module_view(&[("beta", 5), ("alpha", 4)], &[], &[]), 6);
        let err = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect_err("the GA shape errors");
        assert!(
            err.contains("declared in modules `alpha` and `beta`"),
            "module names must be lexicographically sorted: {err}"
        );
        assert!(
            err.contains("`Widget` in `run` (line 4, col 1)"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("and `run`'s module declares no `Widget`"),
            "unexpected message: {err}"
        );
    }

    /// R4's 3+-branch rendering, pinned byte-exact through the same harness:
    /// three reachable declaring modules join with `, ` between items and a
    /// final `, and` -- the spacing no golden exercises (reviewer note
    /// carried forward from the S10 review).
    #[test]
    fn ambiguous_header_error_joins_three_declaring_modules_with_commas() {
        let (cell, borrowed) = three_foreign_header_cell();
        let only = widget_ctor_candidate(borrowed, 4);
        let modules = module_views(module_view(&[("a", 4), ("b", 5), ("c", 6)], &[], &[]), 7);
        let err = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect_err("three reachable headers must not silently borrow the single mint");
        assert_eq!(
            err,
            "error: `Widget` in `run` (line 4, col 1) is ambiguous: declared in modules `a`, `b`, and `c`, and `run`'s module declares no `Widget`\n  note: declare your own `Widget` header and impl, or selectively import the module whose `Widget` you want -- if that module does not itself instantiate `Widget[i64]`, also spell the type in your own word's signature (`import: self::a | Widget | ;` then `: mk ( i64 -- Widget[i64] ) Widget ;`)",
            "the 3-module join must render `a`, `b`, and `c` byte-exact"
        );
    }

    /// R1 exemption 2's scoping half (GH's mechanism at unit level): a
    /// second same-named header declared by a module absent from the
    /// caller's own imports/selective maps does not count toward the >= 2
    /// threshold -- a program-wide count would wrongly flag this.
    #[test]
    fn reachable_header_count_excludes_unimported_declaring_modules() {
        let (cell, borrowed) = two_foreign_header_cell();
        let only = widget_ctor_candidate(borrowed, 4);
        // The caller imports only module 4; module 5 declares a header it
        // never sees.
        let modules = module_views(module_view(&[("lib", 4)], &[], &[]), 6);
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("the unimported second header must not count toward the threshold");
        assert!(
            grounded.is_none(),
            "the sole reachable candidate is borrowed unchanged"
        );
    }

    /// R1 exemption 4's positive case (GI's mechanism at unit level): the
    /// caller's `selective` map, resolved through any hub chain, names
    /// exactly the sole candidate's declaring module -- exempt.
    #[test]
    fn matching_selective_import_exempts_the_ambiguity_check() {
        let (cell, borrowed) = two_foreign_header_cell();
        let only = widget_ctor_candidate(borrowed, 4);
        // Two reachable headers (a plain import of 5, a named selective of
        // 4); the named selective matches the sole minting module.
        let modules = module_views(module_view(&[("b", 5)], &[("Widget", 4)], &[]), 6);
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("the matching named selective import exempts the ambiguity check");
        assert!(
            grounded.is_none(),
            "the matched candidate is borrowed unchanged"
        );
    }

    /// R1 exemption 4's soundness-critical case (GK's mechanism at unit
    /// level): the caller's selective map names a *different* reachable
    /// module than the sole candidate's declaring module -- the exemption
    /// must not fire, and the error must still raise.
    #[test]
    fn mismatched_selective_import_does_not_exempt_the_ambiguity_check() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            5,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        // Only module 5 ever eagerly mints.
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 5, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 5);
        // The caller selectively imported 4's Widget -- but 5 is the sole
        // actual candidate. The caller's own selection must not silently
        // override it.
        let modules = module_views(module_view(&[("b", 5)], &[("Widget", 4)], &[]), 6);
        let err = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect_err("a selective import naming a non-minting module must not exempt");
        assert!(
            err.contains("is ambiguous: declared in modules"),
            "unexpected message: {err}"
        );
    }

    /// R2/GL's mechanism at unit level: the caller's raw selective value
    /// names a re-exporting hub with no header of its own; the match test
    /// must resolve through the hub's own chain to the true declaring
    /// module before comparing, or a program that resolves correctly today
    /// would wrongly error.
    #[test]
    fn hub_reexported_selective_import_resolves_through_origin_walk() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            5,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 4);
        // Module 6 is the hub: it selectively imports Widget from 4 and
        // re-exports it. The caller selectively imports Widget *from the
        // hub*, plus plain imports of both headers' modules.
        let hub = module_view(&[("a", 4)], &[("Widget", 4)], &[]);
        let mut modules =
            module_views(module_view(&[("a", 4), ("b", 5)], &[("Widget", 6)], &[]), 7);
        modules[6] = hub;
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("the hub-resolved selective import matches the sole declaring module");
        assert!(
            grounded.is_none(),
            "the matched candidate is borrowed unchanged"
        );
    }

    /// R1 exemption 2's tightened half (GM's mechanism at unit level): at
    /// most one same-named header is reachable, but the sole env candidate's
    /// declaring module is a different, unreachable module -- both halves
    /// are required, so the error still raises, named structurally because
    /// the caller has no qualifier for that module at all.
    #[test]
    fn unreachable_declaring_module_of_the_sole_candidate_is_still_an_error() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            6,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        // Only module 6 ever eagerly mints; the caller imports only module 4.
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 6, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 6);
        let modules = module_views(module_view(&[("lib", 4)], &[], &[]), 7);
        let err = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect_err("an unreachable minter must not be silently borrowed");
        assert!(
            err.contains("is unresolved: the only `Widget[i64]` instantiation in scope is declared in a module `run`'s module does not import"),
            "the reach-failure shape, named structurally: {err}"
        );
        assert!(
            !err.contains("ambiguous"),
            "one candidate is a reach failure, not an ambiguity: {err}"
        );
    }

    /// R1/GO's mechanism at unit level: a module that is the target of a
    /// selective import for a name *other than* the surface name under check
    /// is still part of the reachable set -- reachability is name-independent
    /// at the raw layer.
    #[test]
    fn reachable_set_includes_selective_targets_regardless_of_selected_name() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 4);
        // The caller has zero plain imports; module 4 is reachable only as
        // the target of a selective import of `Gadget`, a different name.
        let modules = module_views(module_view(&[], &[("Gadget", 4)], &[]), 5);
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("the selective target is reachable regardless of the selected name");
        assert!(
            grounded.is_none(),
            "the sole reachable candidate is borrowed unchanged"
        );
    }

    /// R1/GN's mechanism at unit level: a raw-reachable module with no
    /// header of its own, whose own selective map chases through to a module
    /// that does declare the surface name, extends the reachable set to
    /// include that declaring module too -- exemption 2 sees a header behind
    /// a hub the raw layer alone would miss.
    #[test]
    fn reachable_set_extends_through_reexport_origin_walk() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(0, &[Type::I64], &[], 4, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 4);
        // The caller plainly imports only the hub (module 6), which has no
        // header of its own but selectively imports Widget from 4.
        let mut modules = module_views(module_view(&[("h", 6)], &[], &[]), 7);
        modules[6] = module_view(&[("a", 4)], &[("Widget", 4)], &[]);
        let grounded = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect("the walk-extension resolves the hub to the declaring module");
        assert!(
            grounded.is_none(),
            "one header exists program-wide, nothing to mis-dispatch to"
        );
    }

    /// R2/GP's mechanism at unit level -- the soundness-critical case for the
    /// wildcard exclusion: a `selective` entry populated by a `*` wildcard's
    /// per-export desugar does not satisfy exemption 4, even though it is
    /// indistinguishable from a named selective import in the raw map. The
    /// match test requires the richer, assembly-time `named_selective`
    /// signal.
    #[test]
    fn wildcard_desugar_is_not_explicit_resolution() {
        // Module 5's header (gi 1) is the sole eager minter.
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(generic_struct_decl(
            "Widget",
            4,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        generics.structs.push(generic_struct_decl(
            "Widget",
            5,
            &["'T"],
            &[("v", PolyType::Var(0))],
        ));
        let mut scratch = ScratchRegs::default();
        let borrowed = generics.instantiate_struct(1, &[Type::I64], &[], 5, scratch.regs());
        let cell = RefCell::new(generics);
        let only = widget_ctor_candidate(borrowed, 5);
        // The caller plainly imports 4 and wildcard-imports 5: the desugar
        // puts Widget -> 5 into `selective` but never into `named_selective`.
        let modules = module_views(module_view(&[("a", 4)], &[], &[("Widget", 5)]), 6);
        let err = ground_in_module_3_view(&only, "Widget", &cell, &[], Some(&modules))
            .expect_err("a wildcard desugar must not count as explicit resolution");
        assert!(
            err.contains("is ambiguous: declared in modules"),
            "unexpected message: {err}"
        );
    }

    /// P7b.S6d-PREREQ (REQ-4d site 1): the construction push. A struct
    /// constructor is a generated `env` word, so packing a view into an
    /// aggregate goes through the ordinary word-call output push -- which
    /// forwarded `surviving` and dropped `deriv`, leaving the constructed
    /// value invisible to every exclusivity scan.
    ///
    /// Measured mutation: forcing `deriv: None` on this push makes the
    /// rejection below build.
    #[test]
    fn construction_push_forwards_the_operands_borrow_provenance() {
        let err = check_src(
            "type: Holder val Slice[i64] ;\n\
             : main ( -- )\n  0 4 fill | a |\n  &a slice Holder | h |\n  \
             &!a | r |\n  r 0 >usize &!> 7 !\n  &h &val @ len drop\n  a drop\n;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("`&!a` conflicts with a live borrow of `a`"),
            "{err}"
        );
        // A non-reference-bearing output inherits nothing: a word taking a
        // view and returning a plain `usize` roots no value, so the second
        // borrow is free once the view itself is dead.
        check_src(
            ": main ( -- )\n  0 4 fill | a |\n  &a slice len drop\n  \
             &!a | r |\n  r 0 >usize &!> 7 !\n  a drop\n;\n",
        )
        .expect("a scalar output carries no borrow onward");
    }

    /// P7b.S6d-PREREQ (REQ-4d site 1, Ruling F): `Slot.deriv` is one id and
    /// `Deriv.owned_root` one place, so a value viewing two arrays cannot be
    /// represented -- and silently keeping one root would leave the other
    /// freely re-borrowable, which is the laundering hole at arity two. Both
    /// sides are pinned, since a blanket ban on two-slice aggregates would
    /// pass the rejection half alone.
    #[test]
    fn construction_push_rejects_operands_rooted_at_two_places() {
        let pair = "type: Pair a Slice[i64] b Slice[i64] ;\n";
        let err = check_src(&format!(
            "{pair}: main ( -- )\n  0 4 fill | p |\n  0 4 fill | q |\n  \
             &p slice &q slice Pair drop\n  p drop q drop\n;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("would leave a value viewing both `p` and `q`"),
            "{err}"
        );
        check_src(&format!(
            "{pair}: main ( -- )\n  0 4 fill | p |\n  \
             &p slice &p slice Pair drop\n  p drop\n;\n"
        ))
        .expect("two views of one array agree on the root");
        // A call that hands out nothing reference-bearing insists on no single
        // root at all: `flush ( &!'S &!StrBuf -- )` takes two references
        // rooted at two places and is how `core::show` prints.
        check_src(
            ": both ( &i64 &i64 -- ) drop drop ;\n\
             : main ( -- )\n  0 4 fill | p |\n  0 4 fill | q |\n  \
             &p 0 &> &q 0 &> both\n  p drop q drop\n;\n",
        )
        .expect("no reference-bearing output, so no single-root rule");
    }

    /// P7b.S6d-PREREQ (REQ-4d site 7): the name-read push. A *reference*-typed
    /// local takes the reborrow arm above, which inherits; an aggregate takes
    /// this push, and dropping the deriv here made `w |x|` a one-token
    /// launder. Measured mutation: deleting the `deriv` field from the push
    /// makes this build.
    #[test]
    fn name_read_push_forwards_an_aggregates_borrow_provenance() {
        let window = "type: Window view Slice[i64] lo usize ;\n";
        let err = check_src(&format!(
            "{window}: main ( -- )\n  0 4 fill | a |\n  \
             &a slice 0 >usize Window | w |\n  w | x |\n  \
             &!a | r |\n  r 0 >usize &!> 7 !\n  &x &view @ len drop\n  a drop\n;\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("`&!a` conflicts with a live borrow of `a`"),
            "{err}"
        );
    }

    /// P7b.S6d-PREREQ (REQ-4d site 5): the field store's two rules. (i) A
    /// receiver whose root is not a local of this frame is an escape -- the
    /// pre-existing surviving-set guard cannot see it, since a slice carries
    /// no surviving set, and Ruling B's top-level-reference exemption is what
    /// makes `&!Window` a reachable input. (ii) In-frame, a view of a
    /// *different* place is Ruling F again: propagating the wrong root on
    /// every later read is worse than propagating none.
    #[test]
    fn field_store_rejects_an_out_of_frame_receiver_and_a_second_root() {
        let window = "type: Window view Slice[i64] lo usize ;\n";
        let escape = check_src(&format!(
            "{window}: stash ( &!Window -- )\n  | w |\n  0 4 fill | own |\n  \
             w &!view &own slice !\n  own drop\n;\n"
        ))
        .unwrap_err();
        assert!(
            escape.contains("cannot store the borrow-carrying `Slice[i64]`")
                && escape.contains("the receiver is not rooted in this frame"),
            "{escape}"
        );

        let second_root = check_src(&format!(
            "{window}: main ( -- )\n  0 4 fill | b |\n  0 4 fill | a |\n  \
             &b slice 0 >usize Window | w |\n  &!w &!view &a slice !\n  \
             &w &view @ len drop\n  a drop b drop\n;\n"
        ))
        .unwrap_err();
        assert!(
            second_root.contains("would leave a value viewing both `b` and `a`"),
            "{second_root}"
        );

        // A non-reference-bearing store through the same out-of-frame receiver
        // stays legal: rule (i) is about the borrow the stored value carries,
        // not about the receiver's rootedness on its own.
        check_src(
            "type: Box v i64 ;\n\
             : set ( &!Box -- ) | b | b &!v 7 ! ;\n\
             : main ( -- ) 0 Box | b | &!b set b drop ;\n",
        )
        .expect("storing an `i64` through a reference input is unaffected");
    }
}
