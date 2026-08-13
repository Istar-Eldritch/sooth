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
/// (`times`/quotation), which changes how a granted name's use inside is
/// tracked (see the `Liveness` struct doc). `check_terms` above is the plain
/// entry point every root invocation (a word body, a REPL line, a `case`
/// clause) uses: nothing is ancestor to those, so both are empty/`false`.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_terms_relaxed(
    terms: &[Term],
    mut stack: Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
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
                let linear = is_linear(slot.ty, ctx.structs(), ctx.enums(), arrays);
                scope.bind(name, slot, linear, prov);
            }
            Ok(stack)
        }
        TermKind::Call(name) => {
            if let Some(binding) = scope.local(name) {
                let (ty, aliases, held, quot, surviving) = (
                    binding.ty,
                    binding.aliases,
                    binding.deriv,
                    binding.quot,
                    binding.surviving,
                );
                match ref_parts(ty, refs) {
                    // Naming a reference local is a reborrow, not a move.
                    // A mutable one suspends its place: a second reborrow while
                    // anything derived from the first is still live would be two
                    // live mutable references into one place.
                    Some((_, mutable)) => {
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
                        if is_linear(ty, ctx.structs(), ctx.enums(), arrays) {
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
                            ..Slot::computed(ty)
                        });
                    }
                }
                return Ok(stack);
            }
            // R6: `call`/`times` are compiler-known words intercepted before
            // every builtin family and user-word lookup (a local named `call`
            // already won above). `call` requires a statically-known
            // quotation literal on top (D4) and splices its interned body
            // against the live stack, so `[ 1 + ] call` checks as `1 +` (D3).
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
                // Splice the body against the current locals/scope in lexical
                // extent (capture is free, recon 9), bracketed like an `if`
                // arm so a body that binds does not leak past the `call` and a
                // linear value bound inside it is caught by `leave_block`
                // (R6). `tail` is pinned `false`: lowering emits a real call
                // here, never a self-tail back-edge (R6/R13).
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
                    &body, stack, ctx, env, arrays, cells, refs, prov, scope, false, poly,
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
            // R18: `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )`, the
            // constant-stack loop primitive. Intercepted alongside `call`. The
            // body is spliced against the row plus a synthesized index and must
            // return the row unchanged (D6); nested `times` is rejected here,
            // not in lowering, which has no error channel (R14 step 0).
            if name == "times" {
                let Some(top) = stack.pop() else {
                    return Err(underflow_error(ctx, span, "times", 2, 0));
                };
                // R9: an *abstract* quotation (a declared parameter, no known
                // literal) checks against its declared effect: pop the count,
                // require the effect be row-preserving with a trailing `i64`
                // index (`[ ..row i64 -- ..row ]`), and leave the row
                // unchanged. The three `times` obligations reduce to checks on
                // the declared rows (a declared effect names no local and
                // captures no borrow, so move/borrow identity hold trivially).
                if top.quot.is_none() {
                    // Slice 10a (R2): a `~` abstract parameter is accepted by
                    // `times` exactly like an ordinary quotation parameter.
                    if let Some(eff) = crate::ast::is_quotation_type(top.ty) {
                        return check_abstract_quotation_times(eff, span, stack, ctx);
                    }
                    return Err(times_needs_quotation_error(ctx, span));
                }
                let Some(QuotRef::Known(id)) = top.quot else {
                    return Err(times_needs_quotation_error(ctx, span));
                };
                let Some(count) = stack.pop() else {
                    return Err(underflow_error(ctx, span, "times", 2, 1));
                };
                // The count is also a type-directed read, so a quotation there
                // is the default-deny wording, not a `Cstr`-placeholder mismatch.
                if count.quot.is_some() {
                    return Err(reject_quotation_operand(ctx, span, "times"));
                }
                if count.ty != Type::I64 {
                    return Err(type_mismatch_error(ctx, span, "times", Type::I64, count.ty));
                }
                // 6d/R6: a `times` nested in a loop -- inside a self-tail word,
                // another `times` body, or a self-tail combinator body -- is no
                // longer rejected. Lowering's hoist-target split (R1-R3) keeps
                // every such nesting constant-stack, so the old R18/R14b
                // rejection and its `loop_depth` bookkeeping are gone.
                // R18: the row is the remaining stack; a quotation anywhere in
                // it would reach `begin_loop`'s phi over a phantom (R14). Guard
                // the whole row, not just the consumed top.
                if stack.iter().any(|s| s.quot.is_some()) {
                    return Err(reject_quotation_operand(ctx, span, "times"));
                }
                // R18: the body is spliced once but runs N times, so it must be
                // identity on the move/borrow state (clone-and-compare), or a
                // linear local it consumes would be disposed N times. Snapshot
                // before the splice; `leave_block` drops the body's own
                // bindings, so what remains changed is an *outer* local.
                let moves_before = scope.moves.states.clone();
                let derivs_before: HashSet<DerivId> =
                    live_derivs(&stack, scope, prov, live, at).collect();
                let row = stack.clone();
                // Splice the body against the row plus a synthesized index (the
                // body's top input), bracketed like `call` (R6), `tail = false`.
                // 6d/R6: a `times` nested in the body is now legal, so no
                // `loop_depth` is raised across the splice.
                stack.push(Slot::computed(Type::I64));
                let body = prov.quotations[id.0].body.clone();
                let depth = scope.depth();
                // D6: the row-preservation guard below already rejects a
                // body that carries a reference across the back-edge, so a
                // granted outer name is tracked the same back-edge way as a
                // quotation's own body (used anywhere inside pins it live
                // throughout, per-iteration re-entry means it cannot die
                // early inside).
                let granted = releasable_into(
                    scope,
                    base_depth,
                    outer_releasable,
                    &siblings[at + 1..],
                    live,
                    at,
                );
                let result = check_terms_relaxed(
                    &body, stack, ctx, env, arrays, cells, refs, prov, scope, false, poly,
                    &granted, true,
                )?;
                leave_block(
                    ctx,
                    scope,
                    depth,
                    BlockEnd::Arm {
                        token: "times",
                        span,
                    },
                )?;
                // R18: identity on the move state. A body's own bindings are
                // already gone (`leave_block`), so a local left `Moved`/
                // `MaybeMoved` where it was `Live` is an outer linear local the
                // body consumed; name the first such one.
                if let Some(local) = moves_before.iter().find_map(|(n, before)| {
                    match (before, scope.moves.states.get(n)) {
                        (MoveState::Live, Some(MoveState::Moved(_) | MoveState::MaybeMoved(_))) => {
                            Some(n.clone())
                        }
                        _ => None,
                    }
                }) {
                    return Err(times_body_consumes_local_error(ctx, span, &local));
                }
                // R18: identity on the borrow state. A borrow is idempotent per
                // iteration, so a well-formed body leaves `live_derivs`
                // unchanged; a difference means a reference would cross the
                // back-edge into the next iteration.
                let derivs_after: HashSet<DerivId> =
                    live_derivs(&result, scope, prov, live, at).collect();
                if derivs_after != derivs_before {
                    return Err(times_body_borrow_across_loop_error(ctx, span));
                }
                // D6: the body's net effect on the row must equal the row.
                let same_shape = row.len() == result.len()
                    && result.iter().zip(&row).all(|(found, want)| {
                        matches!(
                            match_slot(*found, want.ty),
                            SlotMatch::Exact | SlotMatch::LiteralSizeType
                        )
                    });
                if !same_shape {
                    return Err(times_body_row_effect_error(ctx, span));
                }
                // R18: the whole-row guard runs on the *entry* row, but a body
                // that consumes a real value and constructs a quotation into
                // its place leaves a phantom in the output row that `match_slot`
                // accepts as `Exact` against the `Cstr` placeholder. That
                // phantom would be carried into the loop's back-edge phis, so
                // reject it here with the same whole-row wording.
                if result.iter().any(|s| s.quot.is_some()) {
                    return Err(reject_quotation_operand(ctx, span, "times"));
                }
                stack = result;
                return Ok(stack);
            }
            if let Some(stack) = check_reference_word(
                name, span, &mut stack, ctx, scope, arrays, cells, refs, prov, live, at,
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
                    if let Some((Type::Quotation(eff), _)) = ref_parts(stack[vi - 1].ty, refs) {
                        let qspan = prov.quotations[id.0].span;
                        let escaping = !ref_root_is_in_frame(stack[vi - 1].deriv, prov, scope);
                        stack[vi] = materialize_quotation_at_boundary(
                            id, eff, escaping, name, qspan, ctx, env, arrays, cells, refs, prov,
                            scope, poly,
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
                            return Err(past_owning_frame_error(ctx, span, &member.name));
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
            }
            if let Some(stack) =
                check_access_word(name, span, &mut stack, ctx, arrays, refs, scope, prov)?
            {
                return Ok(stack);
            }
            if let Some(stack) = check_shuffle(name, span, &mut stack, ctx, arrays, prov)? {
                return Ok(stack);
            }
            // R12 (slice 8b, 8a): a bare operator resolves against the
            // overloads visible to the calling module, not the flat `env`
            // lookup that misses a per-module-mangled decl in a multi-module
            // build. `None` (REPL / single-module) falls back to the flat
            // lookup unchanged.
            let scoped_ops = scoped_operator_overloads(ctx, env, name);
            let op_candidates = match &scoped_ops {
                Some(v) => Some(&v[..]),
                None => env.get(name).map(|v| &v[..]),
            };
            match check_operator(name, span, &mut stack, ctx, op_candidates)? {
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
            if let Some(stack) = check_str_word(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            if let Some(stack) = check_array_word(name, span, &mut stack, ctx, arrays)? {
                return Ok(stack);
            }
            if let Some(stack) = check_owned_cell_word(name, span, &mut stack, ctx, arrays, cells)?
            {
                return Ok(stack);
            }
            if let Some(stack) = check_struct_peek_word(name, span, &mut stack, ctx, arrays, prov)?
            {
                return Ok(stack);
            }
            // D3 (slice 8b): ahead of both the aggregate-field getter below
            // and the ordinary env call path further down, so it catches a
            // moving accessor of a drop-overloaded struct regardless of the
            // extracted field's own type.
            check_destructure_drop_guard(name, span, ctx)?;
            if let Some(stack) = check_struct_get_word(name, span, &mut stack, ctx, prov)? {
                return Ok(stack);
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
                // an unconsumed frame local).
                check_linear_across_back_edge(ctx, span, name, &stack[..base], scope, arrays)?;
                // R9: no reference into a frame local carried by the args.
                check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
                // R12: the self-call's arguments are checked against the ground
                // declared inputs. Rewriting the arm to produce the declared
                // outputs (below) removed the transitive check the `if`-join
                // used to get from the produced-inputs fiction, so this is made
                // explicit. Sound because the marker matches only in tail
                // position. A quotation-typed declared input matches any
                // quotation-carrying arg (its own `call`/`times` already
                // checked the body); everything else matches by type.
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
            if let Some(candidates) = poly.combinators.get(name) {
                let chosen = match candidates.as_slice() {
                    [only] => *only,
                    _ => resolve_combinator_overload(candidates, &stack, span, ctx, arrays)
                        .ok_or_else(|| {
                            no_combinator_overload_matches_error(ctx, span, name, candidates)
                        })?,
                };
                return inline_combinator(
                    &chosen, span, stack, ctx, env, arrays, cells, refs, prov, scope, poly,
                );
            }
            // R5/R14: a call to a polymorphic word is intercepted before the
            // concrete `env` lookup and unified against the concrete stack;
            // its `Sig` is per-instantiation, not name-keyed.
            if poly.env.contains_key(name) {
                return check_poly_call(name, span, &mut stack, ctx, arrays, poly);
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
            let candidates = match &scoped_ops {
                Some(v) => v.as_slice(),
                None => env
                    .get(name)
                    .ok_or_else(|| unknown_word_error(ctx, span, name))?,
            };
            let chosen = match candidates {
                [only] => only,
                _ => {
                    let operands: Vec<Type> = stack.iter().map(|s| s.ty).collect();
                    let hit = candidates.iter().find(|o| {
                        operands.len() >= o.sig.inputs.len()
                            && operands[operands.len() - o.sig.inputs.len()..] == o.sig.inputs[..]
                    });
                    let chosen =
                        hit.ok_or_else(|| no_overload_matches_error(ctx, span, name, candidates))?;
                    poly.builtin_overloads.insert(span, chosen.symbol.clone());
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
                // *constructor* call (`[ 1 + ] Holder`) and a generated setter
                // reach; a `Known` literal is materialized (validated here,
                // lowered to a `(code, env)` value), a capturing one run through
                // the R15 admission rule (a parameter is an in-frame boundary).
                // Gated strictly on
                // `want`'s type, so it covers a constructor, a setter, and an
                // ordinary user word declaring a quotation parameter alike; an
                // `extern` never reaches here (its declared effect cannot name
                // a `Type::Quotation`, rejected at declaration).
                if let Type::Quotation(eff) = *want {
                    if let Some(QuotRef::Known(id)) = found.quot {
                        stack[base + i] = materialize_quotation_at_boundary(
                            id, eff, false, name, span, ctx, env, arrays, cells, refs, prov, scope,
                            poly,
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
            if tail && ctx.mangled_name() == Some(name.as_str()) {
                check_linear_across_back_edge(ctx, span, name, &stack[..base], scope, arrays)?;
                check_reference_across_back_edge(ctx, span, name, &stack[base..], prov)?;
            }
            // R19/R22: a struct/enum constructor consuming an erased closure
            // becomes its carrier -- the surviving capture set rides onto the
            // aggregate output so the captures stay live (R20) and the
            // word-output escape guard (R22) can see a frame capture leaving
            // through the carrier. The union is `None` for the overwhelming
            // majority of calls (no closure argument), a no-op there.
            //
            // Review fix: this same generic dispatch also handles a struct
            // field getter whose field type is `Quotation` (not `is_aggregate`)
            // -- e.g. `Holder>q`, left to the env path because
            // `check_struct_get_word` only claims an aggregate-typed field. A
            // quotation-typed output legitimately carries the closure onward
            // exactly as an aggregate output does, so it forwards too.
            let carried = (base..stack.len())
                .fold(None, |acc, i| prov.union_surviving(acc, stack[i].surviving));
            stack.truncate(base);
            for ty in &sig.outputs {
                let surviving = if carried.is_some()
                    && (ty.is_aggregate() || matches!(ty, Type::Quotation(_)))
                {
                    carried
                } else {
                    None
                };
                stack.push(Slot {
                    surviving,
                    ..Slot::computed(*ty)
                });
            }
            Ok(stack)
        }
        TermKind::If {
            then_branch,
            else_branch,
            else_span,
            end_span,
        } => {
            let cond = stack
                .pop()
                .ok_or_else(|| underflow_error(ctx, span, "if", 1, 0))?;
            // R11: guard before the `Bool` mismatch, or the generic message
            // names the `Cstr` placeholder instead of the `if` condition.
            if cond.quot.is_some() {
                return Err(reject_quotation_operand(ctx, span, "if"));
            }
            if cond.ty != Type::BOOL {
                return Err(type_mismatch_error(ctx, span, "if", Type::BOOL, cond.ty));
            }
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
                then_branch,
                stack.clone(),
                ctx,
                env,
                arrays,
                cells,
                refs,
                prov,
                &mut then_scope,
                tail,
                poly,
                &granted,
                false,
            )?;
            let (then_token, then_at) = match else_span {
                Some(at) => ("else", *at),
                None => ("end", *end_span),
            };
            leave_block(
                ctx,
                &mut then_scope,
                depth,
                BlockEnd::Arm {
                    token: then_token,
                    span: then_at,
                },
            )?;
            let else_stack = check_terms_relaxed(
                else_branch,
                stack,
                ctx,
                env,
                arrays,
                cells,
                refs,
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
                    token: "end",
                    span: *end_span,
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
                        let enclosing: HashSet<String> =
                            scope.bound.iter().map(|bnd| bnd.name.clone()).collect();
                        let mut arm_sets: Vec<SurvivingCaptureSetId> = Vec::new();
                        for id in [a, b] {
                            let body = prov.quotations[id.0].body.clone();
                            if body_captures_enclosing(&body, &enclosing) {
                                let span = prov.quotations[id.0].span;
                                if let Some(set) =
                                    check_capture_admission(id, escaping, span, ctx, prov, scope)?
                                {
                                    arm_sets.push(set);
                                }
                            }
                        }
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
                            ctx.declared_outputs()
                                .and_then(|outs| outs.get(i))
                                .map(|slot| slot.ty)
                        } else {
                            i.checked_sub(1)
                                .and_then(|below| ref_parts(then_stack[below].ty, refs))
                                .map(|(referent, _)| referent)
                                .filter(|t| matches!(t, Type::Quotation(_)))
                        };
                        match expected {
                            Some(Type::Quotation(eff)) => {
                                let word = ctx.word_name().unwrap_or("the branch");
                                let a_span = prov.quotations[a.0].span;
                                let b_span = prov.quotations[b.0].span;
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
                                    prov,
                                    scope,
                                    poly,
                                )?;
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
                                    prov,
                                    scope,
                                    poly,
                                )?;
                                // R23: the merged erased slot's surviving set is
                                // the union of both arms' -- a fresh interned
                                // set, never a mutation of either arm's (keeps
                                // the field `Copy`-compatible).
                                let merged_set = arm_sets
                                    .into_iter()
                                    .fold(None, |acc, s| prov.union_surviving(acc, Some(s)));
                                // Erased: a runtime `(code, env)` value with a
                                // real `Type::Quotation`, no `Known` marker.
                                (None, Some(Type::Quotation(eff)), merged_set)
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
                    (Some(a), Some(b))
                        if prov.deriv(a).suspension() == prov.deriv(b).suspension() =>
                    {
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
        // R5: a quotation literal interns its body into the side table and
        // pushes a compile-time-only marker (D1/D2). The body is *not* checked
        // here (D3): a bare body's input row is unknown until its consumption
        // site (`call`/`times`). The placeholder `ty` is `Cstr`, a
        // registry-free scalar no user op accepts once R11's default-deny is
        // in place (R4).
        TermKind::Quotation(body) => {
            let id = QuotId(prov.quotations.len());
            prov.quotations.push(QuotBody {
                body: body.clone(),
                span,
            });
            prov.quotation_captures.push(capture_names(body));
            stack.push(Slot {
                quot: Some(QuotRef::Known(id)),
                ..Slot::computed(Type::Cstr)
            });
            Ok(stack)
        }
        // D2/D3 (slice 6h phase 2): the shared gate, with the zero-validity
        // predicate turned on -- unlike `fill`, an all-zero slot here is
        // never a replicated real seed.
        TermKind::ArrayCtor(ty) => {
            let Type::Array(id, _) = *ty else {
                unreachable!("the parser only ever interns a Type::Array for an ArrayCtor term")
            };
            let element = arrays[id.index()].element;
            check_array_element_gate(
                ctx,
                span,
                "the array constructor",
                element,
                ctx.structs(),
                ctx.enums(),
                arrays,
                true,
            )?;
            stack.push(Slot::computed(*ty));
            Ok(stack)
        }
    }
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
