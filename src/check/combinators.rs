use super::*;

/// R18/R6a: every quotation-taking word registered under one name. A name
/// can carry more than one candidate exactly as an ordinary overloaded word
/// can (R1); resolving which one a call splices needs the live stack's
/// operand types, the same shape as `Overload`/poly-candidate resolution --
/// a single-value map here would silently shadow a second combinator
/// overload exactly as env's `Sig` did before B1.
pub(crate) type CombinatorEnv<'a> = HashMap<String, Vec<Combinator<'a>>>;

/// Slice 6a (R18): one monomorphic quotation-taking word available to inline.
/// Both fields are shared references into the module, so a `Combinator` is a
/// pair of pointers (`Copy`), which lets a call site copy it out of the
/// borrowed map and then reborrow `PolyCtx` mutably for the splice.
#[derive(Clone, Copy)]
pub(crate) struct Combinator<'a> {
    pub(super) word: &'a WordDef,
    pub(super) terms: &'a [Term],
}

/// R18: gather the quotation-taking `WordBody::Terms` words, mono and poly
/// alike (`is_combinator` does not filter on `word.poly`), keyed by name, so a
/// call to one is intercepted and its body spliced (the inliner) rather than
/// lowered to a call to a word that mints no `IrFunc` (R20). `inline_combinator`
/// branches on `word.poly` internally to pick the mono or poly splice path.
pub(super) fn collect_combinators(words: &[WordDef]) -> CombinatorEnv<'_> {
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
pub(super) fn poly_input_is_quotation(p: &PolyType) -> bool {
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

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_combinator(
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
