use super::*;
use std::cmp::Ordering;
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
pub(super) fn find_bound_impl(
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
///
/// P7b.S2 (S2-8/S2-9): a ctor-abstract dispatch (`ty` a `CtorImage`, generic
/// winner) does not mint here. The per-site composition (re-ground each
/// obligation's slot record through `caller_subst`, unify each identity
/// candidate's member word sig) runs in the obligation loop below; `impl_monos`
/// only ever receives a fully-ground θ_call for a CtorImage winner, never the
/// selection's argless subst.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_user_bound(
    trait_id: TraitId,
    v: u32,
    ty: Type,
    sig: &PolySig,
    name: &str,
    span: Span,
    ctx: &Ctx,
    tr: &TraitResolveCtx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    trait_calls: &mut HashMap<Span, String>,
    impl_monos: &mut Vec<(String, Subst)>,
    caller_subst: &Subst,
) -> Result<(), String> {
    let trait_decl = tr.traits.get(trait_id.index()).expect(
        "a bound's `TraitId` indexes the whole-program trait table, so a call site resolving one must be given that table and not a scratch one",
    );
    // P7.S4b (R6/R7): find the winning impl via the factored helper, which
    // also discharges each candidate's own bounds recursively and detects
    // cycles via a path-scoped visited-set.
    let mut visited: Vec<(TraitId, Type)> = Vec::new();
    // P7b.S2 (S2-8): a ctor-abstract dispatch's selection among identity
    // matches is the compatibility-conditioned tie rule, run per member call
    // site in the obligation loop below -- NOT `find_bound_impl`'s
    // specificity order. That order cannot arbitrate here:
    // `collect_paired_positions` has no arm for a `Type::CtorImage` (a
    // constructor image is not a type to unfold), so at one every same-ctor
    // pair of `Generic` patterns is incomparable and `select_most_specific`
    // would raise the ambiguity error before the per-site rule could
    // disqualify a pin that disagrees with the grounded operands (S2-8's
    // "not a match") or prefer the more-pinned compatible candidate. So for
    // a `CtorImage` ty the identity-matched set is collected directly:
    // empty keeps the None-branch diagnostics below, a single match is the
    // winner outright (unchanged), and two or more hand the choice to the
    // per-site loop, which re-derives the whole list (the nominal winner
    // here only feeds `is_generic`; every identity match is a `Generic`
    // pattern, and the CtorImage-selection subst is argless by design).
    let ctor_ty = matches!(ty, Type::CtorImage(..));
    let identity_matches: Vec<usize> = if ctor_ty {
        tr.impls
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                i.trait_id == trait_id
                    && match_impl_target(&i.target.pattern, ty, arrays, cells, refs, None).is_some()
            })
            .map(|(idx, _)| idx)
            .collect()
    } else {
        Vec::new()
    };
    let winner = if identity_matches.len() > 1 {
        Some((identity_matches[0], Subst::default()))
    } else {
        find_bound_impl(
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
        )?
    };
    let (imp_idx, subst) = match winner {
        Some((idx, subst)) => (idx, subst),
        None => {
            // P7b.S2 (S2-8/S2-15.e): the `for 'T` catch-all guard's diagnostic
            // twin. A bare-var target never matches a `CtorImage` ty (the
            // matcher's guard), so reaching here with a ctor-abstract operand
            // and a bare-var impl in the registry means the capture attempt is
            // the *reason* dispatch failed -- name that, instead of the
            // generic unsatisfied-bound report that would never mention it.
            if matches!(ty, Type::CtorImage(..)) {
                let bare_var = tr.impls.iter().find(|i| {
                    i.trait_id == trait_id && matches!(i.target.pattern, PolyType::Var(_))
                });
                if let Some(imp) = bare_var {
                    return Err(bare_var_impl_target_capture_error(
                        ctx, span, name, trait_decl, imp.span, ty,
                    ));
                }
            }
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
    // P7b.S2 (S2-8/S2-9): a ctor-abstract dispatch -- the operand is a bare
    // `CtorImage` (the App unification bound the head; the args live in the
    // obligation's slot record) and the winner is a generic (ctor) target.
    // The CtorImage-selection subst carries no arg bindings by design, so the
    // generic-winner mint below would record the degenerate `(word, ∅)` and
    // the seed loop would monomorph the member word at the empty subst, where
    // `apply_subst` on `map`'s output `Option[Var('U)]` fails with
    // `poly_unbound_output_ty_error`. Instead, selection and minting are
    // **per member call site**: each obligation's slot record re-grounds
    // through the caller's concrete θ, and each identity-matched candidate's
    // member word sig unifies against the grounded slots -- the
    // compatibility-conditioned tie rule -- producing that site's θ_call.
    let ctor_image_dispatch = is_generic && matches!(ty, Type::CtorImage(..));
    // The identity-matched, bounds-discharging candidate list, once per
    // (trait, bound variable, ty) -- `find_bound_impl` ran the identical
    // selection above and picked the specificity winner; the per-site loop
    // needs the whole list because a pinned candidate can be disqualified at
    // one site and serve at another (S2-9: the tie-break is per site).
    let ctor_candidates: Vec<(usize, usize)> = if ctor_image_dispatch {
        let mut candidates: Vec<(usize, usize)> = tr
            .impls
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                i.trait_id == trait_id
                    && match_impl_target(&i.target.pattern, ty, arrays, cells, refs, None).is_some()
            })
            .map(|(idx, _)| (ctor_pin_count(&tr.impls[idx].target.pattern), idx))
            .collect();
        // Most pins first: among compatible candidates more pins are
        // preferred (the concrete-beats-generic principle, S2-8).
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut discharged = Vec::new();
        for (_, idx) in candidates {
            let imp = &tr.impls[idx];
            let empty = Subst::default();
            if candidate_bounds_discharge(
                idx,
                imp,
                &empty,
                ctx,
                tr,
                arrays,
                cells,
                refs,
                &mut visited,
            )? {
                discharged.push(idx);
            }
        }
        discharged
            .iter()
            .map(|&idx| (ctor_pin_count(&tr.impls[idx].target.pattern), idx))
            .collect()
    } else {
        Vec::new()
    };
    for ob in tr
        .obligations_of(name, sig)
        .iter()
        .filter(|o| o.trait_id == trait_id && o.var == v)
    {
        // The winner's own binding for this member (the non-CtorImage arms).
        // The CtorImage arm re-derives it per candidate instead: identity
        // selection may hand the site to a *different* impl than the
        // specificity winner above picked.
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
        let symbol = if ctor_image_dispatch {
            // S2-9: the per-site composition. Re-ground this call site's slot
            // record through the caller's concrete θ, then try each
            // identity-matched candidate's member word sig against the
            // grounded slots -- incompatible pins disqualify the candidate at
            // this site (it is not a match); among compatible, the ordering
            // above already prefers more pins; two candidates with the same
            // pin count both surviving are S2-8's ambiguity.
            let site_slots: Vec<Type> = ob
                .slots
                .iter()
                .map(|s| apply_subst(sig, s, caller_subst, name, span, ctx, arrays, cells, refs))
                .collect::<Result<Vec<_>, _>>()?;
            // The trailing flag is P7b.S8 (REQ-7): whether the picked
            // candidate's member word is a lifted target's *mono* word, and
            // so dispatches under its bare symbol with nothing to mint.
            let mut picked: Option<(usize, Subst, String, bool)> = None;
            let mut picked_pins = 0usize;
            let mut survived: Vec<usize> = Vec::new();
            for &(_, cidx) in &ctor_candidates {
                let cimp = &tr.impls[cidx];
                let Some((_, widx)) = cimp.resolved.iter().find(|(mname, _)| *mname == ob.member)
                else {
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
                let word_sym = &tr.word_symbols[*widx];
                // P7b.S8 (REQ-7, Delta B dispatch (b)): a lifted target
                // (`for Range[i64]`) matched this site on ctor identity, and
                // its member word is monomorphic -- there is no `PolySig` to
                // unify and nothing to monomorphize. Compatibility is plain
                // type equality against the effect the desugar grounded, and
                // the dispatch symbol is the bare one: the word is lowered
                // once, under its own name, exactly as a concrete target's
                // member word is.
                let lifted_mono = cimp
                    .target
                    .is_mono_ctor_app()
                    .then(|| tr.words.get(*widx).filter(|w| w.poly.is_none()))
                    .flatten();
                if let Some(word) = lifted_mono {
                    let ins: Vec<Type> = word.effect.inputs.iter().map(|s| s.ty).collect();
                    if ins != site_slots {
                        continue;
                    }
                    let pins = ctor_pin_count(&cimp.target.pattern);
                    survived.push(cidx);
                    if picked.as_ref().is_none_or(|_| pins > picked_pins) {
                        picked = Some((cidx, Subst::default(), word_sym.clone(), true));
                        picked_pins = pins;
                    }
                    continue;
                }
                let Some(word_sig) = tr.word_sig_of(word_sym) else {
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
                if word_sig.inputs.len() != site_slots.len() {
                    continue;
                }
                let mut theta = Subst::default();
                let unified = word_sig
                    .inputs
                    .iter()
                    .zip(&site_slots)
                    .all(|(input, slot)| {
                        unify_poly_input(
                            word_sig,
                            input,
                            *slot,
                            word_sym,
                            ob.span,
                            ctx,
                            arrays,
                            cells,
                            refs,
                            &mut theta,
                            &[],
                            &[],
                        )
                        .is_ok()
                    });
                if unified {
                    // Canonical order at construction (the P7.S3t sort
                    // invariant -- the mangled symbol depends on it); without
                    // it two sites could mint two symbols for the same
                    // (word, θ) and monomorphize the specialization twice.
                    theta.ty.sort_by_key(|(v, _)| *v);
                    theta.len.sort_by_key(|(v, _)| *v);
                    let pins = ctor_pin_count(&cimp.target.pattern);
                    survived.push(cidx);
                    if picked.as_ref().is_none_or(|_| pins > picked_pins) {
                        picked = Some((cidx, theta, word_sym.clone(), false));
                        picked_pins = pins;
                    }
                }
            }
            // S2-8: an identical pin-shape tie is the ambiguity error -- two
            // identity-matched targets surviving compatibility at one site.
            let top_tier = survived
                .iter()
                .filter(|&&c| ctor_pin_count(&tr.impls[c].target.pattern) == picked_pins)
                .count();
            if top_tier > 1 {
                let maximal: Vec<usize> = survived
                    .iter()
                    .copied()
                    .filter(|&c| ctor_pin_count(&tr.impls[c].target.pattern) == picked_pins)
                    .collect();
                let cand_refs: Vec<(&ImplDecl, Subst)> = maximal
                    .iter()
                    .map(|&i| (&tr.impls[i], Subst::default()))
                    .collect();
                return Err(ambiguity_error(
                    &trait_decl.name,
                    ty,
                    &cand_refs,
                    &(0..maximal.len()).collect::<Vec<_>>(),
                    ob.span,
                ));
            }
            let Some((_, theta, word_sym, is_mono)) = picked else {
                // Every candidate's member word refused this site's grounded
                // operands: the located member-call operand error, naming the
                // site (the obligation span) and the grounded slot types.
                return Err(ctor_image_member_site_mismatch_error(
                    ctx,
                    ob.span,
                    &ob.member,
                    &trait_decl.name,
                    &site_slots,
                ));
            };
            if is_mono {
                word_sym
            } else {
                let s = instantiation_symbol(&word_sym, &theta);
                impl_monos.push((word_sym, theta));
                s
            }
        } else if is_generic
            && !(imp.target.is_mono_ctor_app()
                && tr.words.get(*idx).is_some_and(|w| w.poly.is_none()))
        {
            // P7.S4 (R6): mint the dispatched symbol as the instantiation of
            // the member word at the matched substitution -- P7b.S8c (D1/
            // REQ-1) *extended*, per site, with what this call site's own
            // operands bind. `find_bound_impl`'s match subst covers only the
            // impl target's variables, so a member row's own free variable
            // (`odd ( 'U &'T -- )`'s `'U`) stayed unbound and lowering's
            // `subst_polytype` expect fired on it (`src/ir/driver.rs:579`).
            // Extending, never replacing, is load-bearing: a nullary member
            // (`empty ( -- 'T )`) has zero obligation slots, so per-site
            // unification contributes nothing and the header variable's
            // binding lives in the match subst alone.
            //
            // P7b.S8c (P2-1): fail closed, mirroring the CtorImage arm above
            // -- a poly:None member word reaching this arm has no `PolySig`
            // to per-site-extend, and minting at the match-only subst would
            // reproduce the pre-slice ICE shape this whole arm exists to
            // fence against.
            let Some(word_sig) = tr.word_sig_of(word_sym) else {
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
            let theta = compose_member_theta(
                word_sig,
                word_sym,
                &trait_decl.name,
                ob,
                sig,
                caller_subst,
                name,
                span,
                ctx,
                arrays,
                cells,
                refs,
                subst.clone(),
            )?;
            let s = instantiation_symbol(word_sym, &theta);
            impl_monos.push((word_sym.clone(), theta));
            s
        } else {
            // A concrete winner keeps the bare symbol path (P7.S4) -- and so
            // does a lifted target's mono member word (P7b.S8, REQ-7): it is
            // a monomorphic word already, lowered under its own symbol, so
            // minting an instantiation of it would name a body that no
            // monomorphization pass will ever emit (`impl_mono_seed`
            // requires `poly: Some`).
            //
            // P7b.S8c (D2/REQ-2): a concrete target's member word carries an
            // already-grounded `StackEffect` in which `ground_member_type`'s
            // `Var` arm (`src/ast.rs:2241`) pinned every free member local to
            // the target type. This arm minted the bare symbol without ever
            // comparing the site's operands against that effect, so an
            // operand of *any* type flowed through such a local slot and the
            // member body computed on its raw slot word -- silent wrong
            // typing, not a panic. Check it, gated on `is_concrete` so the
            // arm's other tenant (a lifted mono ctor-app target, Route E) is
            // untouched: its compatibility is already checked as plain type
            // equality in the CtorImage arm above.
            if imp.target.is_concrete() {
                check_concrete_member_site_slots(
                    &tr.words[*idx],
                    &trait_decl.name,
                    ob,
                    sig,
                    caller_subst,
                    name,
                    span,
                    ctx,
                    arrays,
                    cells,
                    refs,
                )?;
            }
            word_sym.clone()
        };
        trait_calls.insert(ob.span, symbol);
    }
    Ok(())
}

/// P7b.S8c (D2/REQ-2, P2-1): the concrete-winner arm's site-slot
/// compatibility check. `word` is the member word a concrete `impl:` target's
/// desugar registered -- `poly: None`, its `StackEffect` already grounded at
/// the target (`src/parser.rs:4589-4608`), so there is no `PolySig` here and
/// nothing to unify: after re-grounding the obligation's slots through the
/// caller's θ this is plain `Type` equality, the same test the lifted-mono
/// tenant of the same arm makes against its own grounded effect.
///
/// The ruling this enforces: a free member local at a concrete target *stays
/// pinned* to the target type, because that is what `ground_member_type`'s
/// `Var` arm did to it at registration. An operand disagreeing with that pin
/// is rejected; the arm does not become polymorphic and no inference is added.
#[allow(clippy::too_many_arguments)]
fn check_concrete_member_site_slots(
    word: &WordDef,
    trait_name: &str,
    ob: &TraitObligation,
    sig: &PolySig,
    caller_subst: &Subst,
    name: &str,
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<(), String> {
    fence_quotation_literal_slot(ctx, ob, trait_name)?;
    // The obligation carries exactly one slot per declared member input, and
    // the word's effect is a 1:1 ground_member_type map over the same member
    // row (src/parser.rs:4589-4608; a restated signature is rejected), so the
    // lengths agree by construction. Unlike the mint arm there is no residual
    // tail here to fail closed, so the invariant is asserted in debug builds.
    debug_assert_eq!(
        word.effect.inputs.len(),
        ob.slots.len(),
        "member word effect and obligation slots must be 1:1"
    );
    for (slot, (declared, recorded)) in word.effect.inputs.iter().zip(&ob.slots).enumerate() {
        let found = apply_subst(
            sig,
            recorded,
            caller_subst,
            name,
            span,
            ctx,
            arrays,
            cells,
            refs,
        )?;
        if found != declared.ty {
            // Both ends are already concrete, so the two `PolySig`s
            // `trait_member_operand_error` renders against are unread (a
            // `PolyType::Concrete` renders as its own type name) -- the same
            // no-op split the mono branch notes at its own call.
            return Err(trait_member_operand_error(
                ctx,
                ob.span,
                &ob.member,
                trait_name,
                &PolyType::Concrete(declared.ty),
                sig,
                &PolyType::Concrete(found),
                sig,
                slot,
            ));
        }
    }
    Ok(())
}

/// P7b.S2 (S2-8): a `Generic` impl-target pattern's pin count -- how many of
/// the pattern's top-level type arguments are `Concrete` (pinned) rather than
/// variables. The compatibility-conditioned tie rule prefers more pins among
/// compatible candidates (the concrete-beats-generic principle); an identical
/// pin count among candidates that both survive a site's compatibility is the
/// ambiguity error.
pub(super) fn ctor_pin_count(pattern: &PolyType) -> usize {
    match pattern {
        PolyType::Generic { args, .. } => args
            .iter()
            .filter(|a| matches!(a, PolyType::Concrete(_)))
            .count(),
        _ => 0,
    }
}

/// P7b.S2 (S2-8/S2-15.e): a bare-variable impl target met a constructor-keyed
/// dispatch. The matcher refuses the capture (a `for 'T` target can never
/// ground its member against a constructor image -- S1-15.g), so when the
/// registry's only candidate for a ctor-abstract operand is bare-var, the
/// dispatch failure is *this* and the diagnostic names it, located at the
/// dispatch site and carrying the impl's declaration span as the origin.
fn bare_var_impl_target_capture_error(
    ctx: &Ctx,
    span: Span,
    callee: &str,
    trait_decl: &TraitDecl,
    impl_span: Span,
    ty: Type,
) -> String {
    let callee = crate::resolve::demangle_call(callee);
    format!(
        "error: cannot instantiate a bound of `{callee}` with `{ty}` in {name} (line {}, col {})\n  the `impl: {trait} for 'T` declared at line {l}, col {c} cannot capture a constructor-keyed operand: a bare-variable target never matches `{ty}`, which names a constructor rather than a type\n  declare the impl at the constructor it should serve (e.g. `impl: {trait} for {ctor}`)",
        span.line,
        span.col,
        l = impl_span.line,
        c = impl_span.col,
        name = ctx.rendered_word(),
        trait = trait_decl.name,
        ctor = ty.name(),
    )
}

/// P7b.S2 (S2-8/S2-9): every identity-matched candidate's member word refused
/// this call site's grounded operands (each site re-grounds its slot record
/// through the caller's θ; a pinned candidate whose pins disagree with the
/// grounded args is not a match). Located at the member call's own span (the
/// obligation's), naming the trait, the member, and the grounded operand
/// types the site actually presented.
fn ctor_image_member_site_mismatch_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    site_slots: &[Type],
) -> String {
    let operands: Vec<String> = site_slots.iter().map(|t| t.name().to_string()).collect();
    format!(
        "error: `{member}` of `{trait_name}` in {name} (line {}, col {}) has no constructor-keyed impl matching these operands\n  the dispatching impl's member signature does not unify against `{}`; a pinned `impl:` target whose pins disagree with the operand is not a match -- check which impl was intended for this constructor",
        span.line,
        span.col,
        operands.join(" "),
        name = ctx.rendered_word(),
    )
}

/// R8: the concrete type a bounded variable was instantiated with has no
/// `impl:` for the trait. Names the trait, the type, and every member
/// signature the missing impl would have to provide, grounded at that type --
/// grounding interns, but only on this failure path, where the compile is
/// already over.
#[allow(clippy::too_many_arguments)]
pub(super) fn unsatisfied_user_bound_error(
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
    // P7b.S2 (S2-15.f): the error builder grounds member sigs at a ty that
    // can be a `CtorImage` (the dispatch just failed on one), or face an
    // App-headed member slot (an HKT trait's declared signature, which has
    // no mono representation at all -- S2-6), or a ctor-headed one
    // (`Step['T 'It['T]]`, admitted by P7b.S8's Generic gate lift, equally
    // unrepresentable). All three are the non-raising twin's `None`; the
    // builder falls back to the declared `PolyType`'s own rendering rather
    // than raising from inside the error it is building.
    let sigs: Vec<String> = trait_decl
        .members
        .iter()
        .map(|m| {
            let mut render =
                |t: &PolyType| match crate::ast::try_ground_member_type(t, ty, arrays, refs) {
                    Some(g) => g.name().to_string(),
                    None => poly_type_str(t, &m.sig),
                };
            let ins: Vec<String> = m.sig.inputs.iter().map(&mut render).collect();
            let outs: Vec<String> = m.sig.outputs.iter().map(render).collect();
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
pub(super) fn unresolved_trait_obligation_error(
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
            // P7b.S2 (S2-8/S2-15.e): the `for 'T` catch-all guard. A bare-var
            // target never matches a `CtorImage` ty -- it can never ground its
            // member against a constructor image anyway (S1-15.g), so letting
            // it win dispatch by capturing the image would defer the failure
            // to a worse place. The dedicated diagnostic fires at the dispatch
            // site (`resolve_user_bound`'s S2-15.e guard), not here: this arm
            // only makes the non-match.
            if matches!(ty, Type::CtorImage(..)) {
                return None;
            }
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
            len_args,
            name: _,
        } => {
            // P7b.S2 (S2-8): the CtorImage identity arm. A ctor-abstract
            // dispatch (the poly resolve path, where θ of an App-headed bound
            // variable is a bare `Type::CtorImage`) matches a `Generic`
            // target pattern on **constructor identity alone** -- `(idx,
            // module)` == the image's `GenericId` -- without comparing args.
            // The returned subst carries no arg bindings: the member word,
            // polymorphic over the target's variables and the member's
            // locals (S2-5/S2-6), unifies its own slots at the member call
            // from the caller's App-grounded types (S2-9's θ_call). Arg
            // comparison inside a match is R3-rejected. A `Concrete`-pinned
            // target's pins are re-checked per member call site, where the
            // caller's grounded operand args are known (the
            // compatibility-conditioned tie rule, S2-8).
            if let Type::CtorImage(gid, _) = ty {
                let pattern_id = GenericId {
                    is_enum: *is_enum,
                    idx: *idx,
                    module: *module,
                };
                return (pattern_id == gid).then_some(());
            }
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
            let (found_idx, found_module, found_args, found_lens) = found?;
            if found_idx != *idx as usize
                || found_module != *module
                || found_args.len() != args.len()
                || found_lens.len() != len_args.len()
            {
                return None;
            }
            let found_args = found_args.to_vec();
            let found_lens = found_lens.to_vec();
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
            for (len_pat, len_found) in len_args.iter().zip(found_lens.iter()) {
                let Len::Concrete(found_count) = len_found else {
                    unreachable!("an instantiation's own length args are always concrete")
                };
                match len_pat {
                    Len::Concrete(k) => {
                        if *k != *found_count {
                            return None;
                        }
                    }
                    Len::Var(ln) => {
                        if let Some(prev) = subst.len_of(*ln) {
                            if prev != *found_count {
                                return None;
                            }
                        } else {
                            subst.len.push((*ln, *found_count));
                        }
                    }
                }
            }
            Some(())
        }
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in an `impl:` target pattern.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches an impl target pattern"
        ),
        // P7b.S1 review fix (P1): an `impl:` target pattern can never
        // actually carry an `App` -- `parse_impl_target`'s own fence
        // (`impl_target_app_unsupported_error`) rejects one anywhere in the
        // target's structure at parse time, since this matcher has no
        // dispatch story for a constructor-abstract target (S2's). Kept as
        // a real arm, not `unreachable!`, since S1-16's discipline still
        // requires an explicit `App` arm here.
        PolyType::App { .. } => None,
    }
}

/// A flattened position in a specificity comparison. Each leaf in a pattern's
/// structural walk against the matched concrete type is either a concrete
/// value (a singleton equivalence class) or a variable (shared class with
/// every other position holding the same variable id). Type variables and
/// length variables are separate namespaces, so a `Position` carries the
/// namespace tag to keep the two partitions distinct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Position {
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
            len_args,
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
            let (found_idx, found_module, found_args, found_lens) = found?;
            if found_idx != *idx as usize
                || found_module != *module
                || found_args.len() != args.len()
                || found_lens.len() != len_args.len()
            {
                return None;
            }
            let found_args = found_args.to_vec();
            for (arg_pty, arg_ty) in args.iter().zip(found_args.iter()) {
                collect_positions(arg_pty, *arg_ty, arrays, cells, refs, Some(generics), out)?;
            }
            for len_pat in len_args.iter() {
                match len_pat {
                    Len::Concrete(_) => out.push(Position::LenConcrete),
                    Len::Var(ln) => out.push(Position::LenVar(*ln)),
                }
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
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in an `impl:` target pattern.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches an impl target pattern"
        ),
        // P7b.S1 review fix (P1): mirrors `match_impl_target_rec`'s own
        // `App` arm -- an impl-target pattern can never actually carry an
        // `App` (`parse_impl_target`'s fence rejects one anywhere in the
        // target's structure at parse time), so this specificity walk never
        // sees one either. Kept as a real arm for S1-16's discipline.
        PolyType::App { .. } => None,
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
                if let Some((_, _, args, lens)) = generics.struct_instantiation_of(id) {
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
                        for _ in lens {
                            out.push(Position::LenConcrete);
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
                if let Some((_, _, args, lens)) = generics.enum_instantiation_of(id) {
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
                        for _ in lens {
                            out.push(Position::LenConcrete);
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
        // S1-16: an `App`-shaped candidate never reaches specificity
        // comparison -- `poly_cross_match` rejects an `App` slot as
        // unsupported before ordering ever compares two candidates (S1-17.i).
        PolyType::App { .. } => unreachable!(
            "an App-shaped candidate pattern never reaches specificity comparison (S1-17.i)"
        ),
        _ => unreachable!("ty is generic-shaped, so a matching pattern is Generic or Concrete"),
    }
}

/// The length-argument twin of `generic_args_of`: recovers a per-side
/// `Vec<Len>`, either a `Generic` pattern's own `len_args`, or
/// `Len::Concrete`-synthesized from the matched instantiation's own length
/// list (its phase-5-widened fourth tuple element).
pub(super) fn generic_len_args_of(pattern: &PolyType, len_args: &[Len]) -> Vec<Len> {
    match pattern {
        PolyType::Generic { len_args, .. } => len_args.clone(),
        PolyType::Concrete(_) => len_args.to_vec(),
        // S1-16: see `generic_args_of`'s own `App` arm.
        PolyType::App { .. } => unreachable!(
            "an App-shaped candidate pattern never reaches specificity comparison (S1-17.i)"
        ),
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
        // S1-16: an `App` never has a quotation shape, so a candidate
        // reaching a quotation-typed slot as an `App` is unreachable for
        // the same reason `generic_args_of`'s own `App` arm is.
        PolyType::App { .. } => unreachable!(
            "an App-shaped candidate pattern never reaches specificity comparison (S1-17.i)"
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
            let (_, _, ty_args, len_args) = generics.struct_instantiation_of(id)?;
            if ty_args.is_empty() {
                return Some(());
            }
            let ty_args = ty_args.to_vec();
            let len_args = len_args.to_vec();
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
            let a_lens = generic_len_args_of(a, &len_args);
            let b_lens = generic_len_args_of(b, &len_args);
            for (la, lb) in a_lens.iter().zip(b_lens.iter()) {
                out.push(PairedLeaf::Aligned(
                    len_position(la.clone()),
                    len_position(lb.clone()),
                ));
            }
            Some(())
        }
        Type::Enum(id, _) => {
            let generics = generics?;
            let (_, _, ty_args, len_args) = generics.enum_instantiation_of(id)?;
            if ty_args.is_empty() {
                return Some(());
            }
            let ty_args = ty_args.to_vec();
            let len_args = len_args.to_vec();
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
            let a_lens = generic_len_args_of(a, &len_args);
            let b_lens = generic_len_args_of(b, &len_args);
            for (la, lb) in a_lens.iter().zip(b_lens.iter()) {
                out.push(PairedLeaf::Aligned(
                    len_position(la.clone()),
                    len_position(lb.clone()),
                ));
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
pub(super) fn specificity(
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
pub(super) fn is_strictly_more_specific(a: &[Position], b: &[Position]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ArrayDecl, ArrayId, GenericStructDecl, OwnedCellDecl};

    /// A `Buffer['T 'N: Len]`-shaped generic header, for the R8a length-
    /// binding tests below -- the same header `ast.rs`'s own
    /// `instantiate_struct_distinct_lengths_mint_distinct_monomorphs` uses.
    fn buffer_header() -> GenericStructDecl {
        GenericStructDecl {
            name: "Buffer".to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            fields: vec![(
                "data".to_string(),
                PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0)),
            )],
            span: Span::default(),
            module: 0,
        }
    }

    /// P7b.S2 (S2-8): the CtorImage identity arm. A ctor-abstract dispatch
    /// (`ty` a bare `Type::CtorImage` -- the App unification's head binding)
    /// matches a `Generic` target pattern on constructor identity alone;
    /// the subst carries no arg bindings (R3: identity-only matching), the
    /// member call re-derives the args (S2-9's θ_call).
    #[test]
    fn match_impl_target_ctor_image_matches_generic_pattern_on_identity_alone() {
        let pattern = PolyType::Generic {
            is_enum: false,
            idx: 3,
            module: 1,
            args: vec![PolyType::Var(0)],
            len_args: Vec::new(),
            name: "Box",
        };
        let image = Type::CtorImage(
            GenericId {
                is_enum: false,
                idx: 3,
                module: 1,
            },
            "Box",
        );
        let subst = match_impl_target(&pattern, image, &[], &[], &[], None).expect("should match");
        // Identity only: no arg bindings survive the match (R3) -- the
        // caller's grounded args arrive per member call site (S2-9).
        assert!(
            subst.ty.is_empty(),
            "arg bindings must be absent: {subst:?}"
        );
        assert!(subst.len.is_empty());
    }

    #[test]
    fn match_impl_target_ctor_image_of_another_ctor_no_match() {
        let pattern = PolyType::Generic {
            is_enum: false,
            idx: 3,
            module: 1,
            args: vec![PolyType::Var(0)],
            len_args: Vec::new(),
            name: "Box",
        };
        let other = Type::CtorImage(
            GenericId {
                is_enum: false,
                idx: 4,
                module: 1,
            },
            "Bag",
        );
        assert!(match_impl_target(&pattern, other, &[], &[], &[], None).is_none());
    }

    /// P7b.S2 (S2-8/S2-15.e): the `for 'T` catch-all guard. A bare-var
    /// target never matches a `CtorImage` ty -- the capture that would let
    /// dispatch "win by accident" and die at S1-15.g.
    #[test]
    fn match_impl_target_bare_var_pattern_does_not_capture_ctor_image() {
        let image = Type::CtorImage(
            GenericId {
                is_enum: false,
                idx: 0,
                module: 0,
            },
            "Box",
        );
        assert!(match_impl_target(&PolyType::Var(0), image, &[], &[], &[], None).is_none());
    }

    /// P7b.S2 (S2-8): a pinned `Generic` target pattern's pin count -- the
    /// compatibility-conditioned tie rule prefers more pins among
    /// compatible candidates.
    #[test]
    fn ctor_pin_count_counts_concrete_top_level_args() {
        let pinned = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64), PolyType::Var(0)],
            len_args: Vec::new(),
            name: "Pair",
        };
        let all_var = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0), PolyType::Var(1)],
            len_args: Vec::new(),
            name: "Pair",
        };
        assert_eq!(ctor_pin_count(&pinned), 1);
        assert_eq!(ctor_pin_count(&all_var), 0);
        assert_eq!(ctor_pin_count(&PolyType::Var(0)), 0);
    }

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

    /// R8b: `match_impl_target_rec`'s `Generic` arm zips `len_args`
    /// alongside `args`, the same way its `Array` arm zips `len` -- a
    /// `Buffer['T 4]` pattern matches only `Buffer[u8 4]`, never
    /// `Buffer[u8 8]`; must fail against the pre-fix `len_args: _` arm,
    /// which is length-blind and would match either.
    #[test]
    fn match_impl_target_generic_zips_len_args_against_concrete_length() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let buffer_4 = generics.instantiate_struct(
            0,
            &[Type::U32],
            &[Len::Concrete(4)],
            0,
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                cells: &mut cells,
                refs: &mut refs,
            },
        );
        let buffer_8 = generics.instantiate_struct(
            0,
            &[Type::U32],
            &[Len::Concrete(8)],
            0,
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                cells: &mut cells,
                refs: &mut refs,
            },
        );
        let pattern = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Concrete(4)],
            name: "Buffer",
        };
        assert!(
            match_impl_target(&pattern, buffer_4, &arrays, &cells, &refs, Some(&generics))
                .is_some(),
            "`Buffer['T 4]` should match `Buffer[u8 4]`"
        );
        assert!(
            match_impl_target(&pattern, buffer_8, &arrays, &cells, &refs, Some(&generics))
                .is_none(),
            "`Buffer['T 4]` should not match `Buffer[u8 8]`"
        );
    }

    /// The `Len::Var` half: a shared length variable across a `Generic`
    /// pattern's own length arg binds from the concrete instantiation,
    /// mirroring `match_impl_target_shared_len_var_consistency`'s `Array`
    /// case one level up.
    #[test]
    fn match_impl_target_generic_binds_len_var_from_concrete_instantiation() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let buffer_256 = generics.instantiate_struct(
            0,
            &[Type::U32],
            &[Len::Concrete(256)],
            0,
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                cells: &mut cells,
                refs: &mut refs,
            },
        );
        let pattern = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        let subst = match_impl_target(
            &pattern,
            buffer_256,
            &arrays,
            &cells,
            &refs,
            Some(&generics),
        )
        .expect("a concrete Buffer[u8 256] target must bind 'N");
        assert_eq!(subst.len_of(0), Some(256));
    }

    /// R8b: `generic_len_args_of` recovers a length list from both shapes --
    /// a `Generic` pattern's own `len_args`, or `Len::Concrete`-synthesized
    /// from the matched instantiation's length list -- the sibling
    /// `generic_args_of` had this coverage, its length twin previously had
    /// none.
    #[test]
    fn generic_len_args_of_recovers_a_length_list_from_both_shapes() {
        let generic_pattern = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        assert_eq!(
            generic_len_args_of(&generic_pattern, &[Len::Concrete(256)]),
            vec![Len::Var(0)]
        );
        let concrete_pattern = PolyType::Concrete(Type::U32);
        assert_eq!(
            generic_len_args_of(&concrete_pattern, &[Len::Concrete(256)]),
            vec![Len::Concrete(256)]
        );
    }

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

    /// R8b: `collect_paired_positions`'s `Struct` arm pushes a paired length
    /// leaf via `generic_len_args_of`. The scenario that actually exercises
    /// this -- and that two identically-concrete-length patterns cannot,
    /// since `match_impl_target_rec`'s own `Generic` arm already rejects a
    /// mismatched concrete length before either candidate ever reaches
    /// `specificity` -- is a length-*variable* pattern (`Buffer['T 'N]`)
    /// competing against a length-*concrete* pattern (`Buffer['T 4]`) for
    /// the same concrete operand (`Buffer[u8 4]`): both genuinely match, so
    /// `specificity` must rank the concrete length as strictly more
    /// specific than the variable, mirroring `specificity_concrete_target_vs_generic_target`
    /// one level up (a struct's own length position, not a bare array's).
    #[test]
    fn specificity_struct_header_length_positions_are_not_ignored() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let buffer_4 = generics.instantiate_struct(
            0,
            &[Type::U32],
            &[Len::Concrete(4)],
            0,
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                cells: &mut cells,
                refs: &mut refs,
            },
        );
        let a = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Concrete(4)],
            name: "Buffer",
        };
        let b = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        // `a`'s concrete length (4) matches `buffer_4`'s actual length, and
        // `b`'s length variable also matches (it binds anything), so both
        // are genuine candidates at this call site; `a` must win.
        assert_eq!(
            specificity(
                &a,
                &b,
                &[],
                &[],
                buffer_4,
                &arrays,
                &cells,
                &refs,
                Some(&generics)
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            specificity(
                &b,
                &a,
                &[],
                &[],
                buffer_4,
                &arrays,
                &cells,
                &refs,
                Some(&generics)
            ),
            Some(Ordering::Greater)
        );
    }

    /// R8b: `collect_positions`'s `Generic` arm pushes a `Position` for
    /// each of a struct pattern's `len_args`. `Buffer['T 'N]` (both type and
    /// length still variables) flattens to *two* leaves against a bare
    /// `'T`, not one, once its length position is counted -- without the
    /// push it would flatten to a single `TyVar` leaf, tying the bare
    /// variable's own single leaf and yielding `None` (equal, incomparable)
    /// instead of the correct one-sided verdict below.
    #[test]
    fn specificity_bare_var_vs_generic_struct_length_position_depth_mismatch() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let buffer_4 = generics.instantiate_struct(
            0,
            &[Type::U32],
            &[Len::Concrete(4)],
            0,
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                cells: &mut cells,
                refs: &mut refs,
            },
        );
        let a = PolyType::Var(0);
        let b = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        // A bare variable is unconditionally less specific than the
        // structured `Buffer['T 'N]` pattern, which now has *two* leaves
        // (its type argument and its length argument) once the length
        // position is counted, forcing the one-sided `BMoreSpecific`
        // branch in `collect_var_vs_other` rather than a single aligned
        // pair.
        assert_eq!(
            specificity(
                &a,
                &b,
                &[],
                &[],
                buffer_4,
                &arrays,
                &cells,
                &refs,
                Some(&generics)
            ),
            Some(Ordering::Greater)
        );
        assert_eq!(
            specificity(
                &b,
                &a,
                &[],
                &[],
                buffer_4,
                &arrays,
                &cells,
                &refs,
                Some(&generics)
            ),
            Some(Ordering::Less)
        );
    }

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
