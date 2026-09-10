use super::*;
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
/// P7b.S2 (S2-16): whether a candidate member's declared inputs fit the
/// call's operand slots, with the header pre-bound to the candidate's own
/// bound variable (`var`) -- the same seed the operand check uses, so the
/// fit test and the check can never disagree about which variable a slot
/// must carry.
fn member_fits_operands(inputs: &[PolyType], slots: &[PolySlot], var: u32) -> bool {
    let mut bindings = vec![(0u32, PolyType::Var(var))];
    inputs
        .iter()
        .zip(slots)
        .all(|(declared, slot)| unify_member_operand(declared, &slot.pt, &mut bindings))
}

fn candidate_fitting_the_operands<'a>(
    stack: &[PolySlot],
    candidates: &[(u32, TraitId, &'a TraitMember)],
) -> CandidateFit<'a> {
    if candidates.windows(2).all(|w| w[0].0 == w[1].0) {
        return CandidateFit::Ambiguous;
    }
    // P7b.S2 (S2-16): the fit test is the same one-way unification the single
    // candidate's operand check uses (`unify_member_operand`, header pre-bound
    // to the candidate's variable), not the `substitute_member_var` rewrite
    // equality it had -- the rewrite conflates a member's own locals with the
    // caller's bound var, so an HKT member's App-headed input could never fit
    // anything and the disambiguation never had a fair test. The pre-bound
    // seed keeps candidates on different variables separated: a slot carrying
    // `'T` cannot fit a candidate dispatched on `'U`.
    let mut fitting = candidates.iter().filter(|(_var, _, member)| {
        let inputs = &member.sig.inputs;
        stack.len() >= inputs.len() && {
            let base = stack.len() - inputs.len();
            member_fits_operands(inputs, &stack[base..], *_var)
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

/// P7b.S2 (S2-16, poly caller): one-way unification of a member's *declared*
/// input `PolyType` (trait space: id 0 is the header, ids 1.. are the
/// member's own locals) against a caller-space operand slot. The declared
/// sig is the source -- not the member word's grounded `PolySig`: at
/// body-check no impl is selected, and the word's sig is ctor-headed (S2-6
/// dissolved `'F`), so unifying it against an abstract App operand would
/// wrongly concretize the caller's bound variable.
///
/// The header's binding is the dispatch itself: callers seed `bindings` with
/// `(0, Var(var))` -- the member header variable binds to the caller's
/// *dispatched* bound variable -- so an App-headed declared input unifies
/// against a caller `App{h, args}` only when `h` is that variable (W4's
/// poly-poly reading: plain App-vs-App unification binds the member's header
/// variable to the caller's bound variable and member locals to the caller's
/// slot arguments; no `CtorImage` exists yet at body-check). Member locals
/// bind to the caller's slot arguments; a declared local (non-id-0) heading
/// an application binds the same way (S2-6's nested-local shape). The seed
/// is also what keeps multi-candidate fitting variable-separated: a
/// candidate on `'U` cannot fit a slot carrying `'T`.
///
/// Conservative on the rest: a declared `Concrete`/`Array`/len against a
/// caller variable is a mismatch (binding would concretize the caller's own
/// variable at body-check, breaking its polymorphism), as is any shape pair
/// the arms below do not align. Returns `false` for mismatch, mirroring the
/// structural-equality check this replaces, so the caller raises the same
/// located `trait_member_operand_error`.
pub(super) fn unify_member_operand(
    declared: &PolyType,
    found: &PolyType,
    bindings: &mut Vec<(u32, PolyType)>,
) -> bool {
    fn bind(bindings: &mut Vec<(u32, PolyType)>, v: u32, pt: PolyType) -> bool {
        match bindings.iter_mut().find(|(id, _)| *id == v) {
            Some((_, prev)) => *prev == pt,
            None => {
                bindings.push((v, pt));
                true
            }
        }
    }
    match (declared, found) {
        (PolyType::Var(v), found) => bind(bindings, *v, found.clone()),
        (PolyType::Concrete(a), PolyType::Concrete(b)) => a == b,
        (
            PolyType::App {
                head: dh,
                args: dargs,
            },
            PolyType::App {
                head: fh,
                args: fargs,
            },
        ) => {
            dargs.len() == fargs.len()
                && bind(bindings, *dh, PolyType::Var(*fh))
                && dargs
                    .iter()
                    .zip(fargs.iter())
                    .all(|(d, f)| unify_member_operand(d, f, bindings))
        }
        (
            PolyType::Quotation(dins, douts, dinline, _, _),
            PolyType::Quotation(fins, fouts, finline, _, _),
        ) => {
            dinline == finline
                && dins.len() == fins.len()
                && douts.len() == fouts.len()
                && dins
                    .iter()
                    .zip(fins)
                    .all(|(d, f)| unify_member_operand(d, f, bindings))
                && douts
                    .iter()
                    .zip(fouts)
                    .all(|(d, f)| unify_member_operand(d, f, bindings))
        }
        (PolyType::Ref(dr, dm), PolyType::Ref(fr, fm)) => {
            dm == fm && unify_member_operand(dr, fr, bindings)
        }
        (PolyType::OwnedCell(dp), PolyType::OwnedCell(fp)) => {
            unify_member_operand(dp, fp, bindings)
        }
        (PolyType::Array(de, dl), PolyType::Array(fe, fl)) => {
            // The member grammar declares no length variables (S2-5: locals
            // are `Star` type variables only), so a declared len must agree
            // structurally; binding one is out of this slice's scope.
            dl == fl && unify_member_operand(de, fe, bindings)
        }
        // P7b.S6 Phase 2 (R2.b, M3): a declared quotation parameter against
        // a fully-concrete operand -- the operand's effect is folded into a
        // `QuotEffect` (`quotation_type`'s doc), so the found side arrives
        // `Concrete`, not `Quotation`, and never reaches the arm above. Undo
        // the fold: reconstruct the operand's own input/output rows as
        // `Concrete` `PolyType`s and unify them structurally against
        // `dins`/`douts`, exactly as the `Quotation`/`Quotation` arm does.
        // `Type::OwningQuotation` never matches here -- a member's declared
        // quotation param can only be spelled plain or `~` (inline), never
        // "owning" (`raw_to_poly_type`'s fold never produces it either), so
        // an owning operand is a flavour mismatch like any other.
        (PolyType::Quotation(dins, douts, dinline, _, _), PolyType::Concrete(found_ty)) => {
            let (eff, found_inline) = match found_ty {
                Type::Quotation(eff) => (eff, false),
                Type::InlineQuotation(eff) => (eff, true),
                _ => return false,
            };
            *dinline == found_inline
                && dins.len() == eff.inputs.len()
                && douts.len() == eff.outputs.len()
                && dins
                    .iter()
                    .zip(&eff.inputs)
                    .all(|(d, f)| unify_member_operand(d, &PolyType::Concrete(*f), bindings))
                && douts
                    .iter()
                    .zip(&eff.outputs)
                    .all(|(d, f)| unify_member_operand(d, &PolyType::Concrete(*f), bindings))
        }
        _ => declared == found,
    }
}

/// P7b.S2 (S2-16, poly caller): render a member's declared output `PolyType`
/// into the caller's variable space through the unification's binding map.
/// A declared variable the call's inputs never bound renders as the caller's
/// bound variable -- the fallback `substitute_member_var` had for every bare
/// mention (and the only rendering a nullary member's fresh local can get);
/// bound variables render as what the call bound them to.
pub(super) fn render_member_decl(t: &PolyType, bindings: &[(u32, PolyType)], var: u32) -> PolyType {
    let render = |t: &PolyType| render_member_decl(t, bindings, var);
    match t {
        PolyType::Var(v) => bindings
            .iter()
            .find(|(id, _)| id == v)
            .map(|(_, pt)| pt.clone())
            .unwrap_or_else(|| PolyType::Var(var)),
        PolyType::App { head, args } => {
            let rendered_head = match bindings.iter().find(|(id, _)| id == head) {
                Some((_, PolyType::Var(u))) => *u,
                _ => var,
            };
            PolyType::App {
                head: rendered_head,
                args: args.iter().map(render).collect(),
            }
        }
        PolyType::Quotation(ins, outs, inline, _, _) => PolyType::Quotation(
            ins.iter().map(render).collect(),
            outs.iter().map(render).collect(),
            *inline,
            None,
            None,
        ),
        PolyType::Ref(r, m) => PolyType::Ref(Box::new(render(r)), *m),
        PolyType::Array(e, l) => PolyType::Array(Box::new(render(e)), l.clone()),
        PolyType::OwnedCell(p) => PolyType::OwnedCell(Box::new(render(p))),
        other => other.clone(),
    }
}

/// P7b.S2 (S2-2/S2-16): the position of a member's dispatchable input -- the
/// trait var bare (Star traits, under one `Ref` layer) or heading an
/// application (HKT traits, again under one `Ref` layer). Mirrors
/// `declarations.rs`'s S2-2 rule, which the declaration gate enforces, so a
/// registered member always has one.
fn dispatchable_input_pos(sig: &PolySig) -> Option<usize> {
    let head = |t: &PolyType| matches!(t, PolyType::Var(0) | PolyType::App { head: 0, .. });
    sig.inputs.iter().position(|input| match input {
        PolyType::Ref(referent, _) => head(referent),
        other => head(other),
    })
}

/// P7.S3e (R7): a trait member's signature is written over the trait's own
/// single type variable (id 0 in the member's own `PolySig`); dispatching it
/// through a bound rewrites that variable to the bounded variable of the
/// *calling* word's signature, so the result is comparable against the walk's
/// stack directly.
///
/// P7b.S2 (S2-16): retained only for the *operand-shape* diagnostics that
/// render a declared sig over the calling word's variables. The operand check
/// itself is now `unify_member_operand`'s one-way unification -- the rewrite
/// conflates every bare variable with the caller's bound var, which for a
/// member with its own locals (`map`'s `'T`/`'U`) is precisely the F6 bug:
/// no caller slot could ever compare equal.
fn substitute_member_var(t: &PolyType, var: u32) -> PolyType {
    match t {
        PolyType::Var(_) => PolyType::Var(var),
        PolyType::Ref(referent, mutable) => {
            PolyType::Ref(Box::new(substitute_member_var(referent, var)), *mutable)
        }
        PolyType::Array(elem, len) => {
            PolyType::Array(Box::new(substitute_member_var(elem, var)), len.clone())
        }
        // S1-16: `head` names one of the trait's own type variables (id 0,
        // the only one a trait member's signature can mention), the same
        // space `Var`'s bare occurrence rewrites above -- an `other =>
        // other.clone()` catch-all would pass an `App` head through
        // unrewritten, silently leaving it pointing at the trait's own
        // variable space instead of the dispatching word's.
        PolyType::App { head, args } => PolyType::App {
            head: if *head == 0 { var } else { *head },
            args: args.iter().map(|a| substitute_member_var(a, var)).collect(),
        },
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
pub(in crate::check) fn resolve_splice_member_call(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    scope: &mut Scope,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    poly: &mut PolyCtx,
    prov: &mut Provenance,
    live: &Liveness,
    at: usize,
    granted: &HashSet<String>,
    tail: bool,
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
            //
            // P7b.S3 (S3-7): a candidate θ does not ground is **recorded with
            // its reason** and excluded from `fitting`, never silently
            // `continue`d. S2-15.f's two bare `continue`s (an App-headed
            // signature; a `try_ground_member_type` `None`) existed only to
            // route around `ground_member_type`'s unreachable arms, and a
            // silent drop is the failure mode that would leave a
            // two-candidate program reporting "no candidate fits" -- or worse,
            // dispatching to the other impl -- on exactly the shapes this
            // slice adds. If `fitting` ends up empty the error now names every
            // candidate *and why each failed to ground*, which is strictly
            // more information than the shape list alone.
            let mut fitting: Vec<(u32, TraitId, &TraitMember)> = Vec::new();
            let mut ungroundable: Vec<(&str, String)> = Vec::new();
            for (v, tid, m) in many {
                let trait_name = traits[tid.index()].name.as_str();
                let Some(ty) = subst.ty_of(*v) else {
                    ungroundable.push((
                        trait_name,
                        format!(
                            "the splice's θ leaves `{}` unbound",
                            sig.ty_var_names[*v as usize]
                        ),
                    ));
                    continue;
                };
                // P7b.S3 (S3-7): the same θ-grounding rule the
                // single-candidate path uses -- a candidate that fails to
                // ground is recorded with its reason and excluded, never
                // silently `continue`d.
                let inputs = match ground_member_sig_via_theta(
                    &m.sig, member, trait_name, ty, stack, span, ctx, arrays, cells, refs,
                ) {
                    Ok((inputs, _)) => inputs,
                    Err(reason) => {
                        ungroundable.push((trait_name, reason));
                        continue;
                    }
                };
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
                    return Err(no_candidate_fits_operands_error(
                        ctx,
                        span,
                        member,
                        &shapes,
                        &ungroundable,
                    ));
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
    // P7b.S3 (S3-7): the single-candidate twin of the disambiguation loop's
    // recorded exclusions. θ leaving the bound variable unbound used to
    // return `Ok(None)`, letting the call fall through to ordinary dispatch
    // and surface as an unknown word at a span that explains nothing. It is a
    // located error naming the member, the trait and the variable instead --
    // the sibling of `splice_member_ctor_image_error`, which covers the case
    // where θ *does* bind the variable but only to a bare constructor.
    let Some(ty) = subst.ty_of(var) else {
        return Err(splice_member_unbound_var_error(
            ctx,
            span,
            member,
            &traits[trait_id.index()].name,
            &sig.ty_var_names[var as usize],
        ));
    };
    // P7b.S3 (S3-8/S3-1.c): at a real splice, resolve the impl *before* any
    // grounding: an `inline` member routes onto the combinator splice path
    // (`inline_combinator`) with the impl's own `Subst` as θ's seed, so its
    // arguments are checked by the machinery every combinator call already
    // uses and the trait-sig grounding below never runs for it. A `None`
    // result is not an error yet: the unsatisfied-bound error keeps firing
    // after the operand check, exactly where it fired before S3-1.c.
    let found_impl = match prov.splice_uid {
        Some(_) => {
            let tr = poly.trait_resolve;
            find_bound_impl(
                trait_id,
                ty,
                None,
                span,
                ctx,
                &tr,
                arrays,
                cells,
                refs,
                &mut Vec::new(),
            )?
        }
        None => None,
    };
    if let Some(uid) = prov.splice_uid {
        if let Some((imp_idx, ref impl_subst)) = found_impl {
            let tr = poly.trait_resolve;
            let imp = &tr.impls[imp_idx];
            if let Some((_, widx)) = imp.resolved.iter().find(|(m, _)| m == member) {
                let mword = &tr.words[*widx];
                // A member-splice cycle (a body whose bound dispatch reaches
                // back to a member already on the active splice chain, at any
                // depth) must not re-splice at check time: fall through to
                // the grounded-record path, and lowering's splice-budget
                // guard reports the recursion, exactly as it did before
                // S3-1.c.
                let re_entry = prov.member_splice_stack.contains(widx);
                if mword.declares_inline && !re_entry {
                    // S3-1.c: key on the member's own `WordDef`, never on
                    // `word_symbols[idx]` -- `overload_symbols` `$$`-suffixes
                    // a colliding name, while `poly.combinators` is keyed on
                    // `word.name`, so the two only coincidentally agree. A
                    // `declares_inline` word absent from the map is a
                    // `collect_combinators` invariant violation, not a
                    // fall-through.
                    let comb = *poly
                        .combinators
                        .get(&mword.name)
                        .and_then(|cands| {
                            cands.iter().find(|c| {
                                c.word.span == mword.span && c.word.module == mword.module
                            })
                        })
                        .expect(
                            "collect_combinators registers every `declares_inline` word under its `word.name`",
                        );
                    // The lowering half of this splice: `lower_call`'s
                    // `(uid, span)` lookup routes the site into
                    // `lower_resolved_word_call`, whose name-keyed
                    // `combinators` hit splices the same body -- without
                    // this record lowering falls through to the bare-name
                    // env lookup and panics.
                    poly.splice_trait_calls
                        .insert((uid, span), mword.name.clone());
                    let live = std::mem::take(stack);
                    prov.member_splice_stack.push(*widx);
                    let spliced = inline_combinator(
                        &comb,
                        span,
                        live,
                        ctx,
                        env,
                        arrays,
                        cells,
                        refs,
                        slices,
                        prov,
                        scope,
                        poly,
                        granted,
                        tail,
                        Some(impl_subst),
                        true,
                    );
                    prov.member_splice_stack.pop();
                    return spliced.map(Some);
                }
            }
        }
    }
    // Ground the member's input/output types at the concrete θ (S3-7:
    // through the caller's θ where the single-`Type` rule cannot).
    let (input_types, output_types) = ground_member_sig_via_theta(
        &member_decl.sig,
        member,
        &traits[trait_id.index()].name,
        ty,
        stack,
        span,
        ctx,
        arrays,
        cells,
        refs,
    )?;
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
                // P7b.S6 Phase 1 (R2.a) per-site verdict: both operands are
                // already `Concrete`, so `expected_sig`/`found_sig` share the
                // same `sig` -- the split is a no-op here.
                return Err(trait_member_operand_error(
                    ctx,
                    span,
                    member,
                    &traits[trait_id.index()].name,
                    &PolyType::Concrete(*want),
                    &sig,
                    &PolyType::Concrete(found.ty),
                    &sig,
                    i,
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
        let trait_decl = &traits[trait_id.index()];
        // P7b.S3 (S3-8, gate G3): the impl was resolved above through
        // `find_bound_impl` -- live registries, R6 candidate-bound discharge,
        // R7 cycle detection and R3 most-specific selection -- so the inline
        // and non-inline callers cannot dispatch to different impls.
        let tr = poly.trait_resolve;
        // P7.S8 follow-up: an unsatisfied bound discovered here is reached
        // only once a library combinator's body (e.g. `lt`) has already been
        // spliced into the caller, so `span`/`name` at this point are
        // `lib/core/cmp.sth`'s own -- the `cmp` call inside `lt`, not the
        // `lt` the user wrote. Name and locate the outermost splice instead,
        // when one is active.
        let (origin_span, origin_name) = prov
            .splice_origin
            .clone()
            .unwrap_or_else(|| (span, name.to_string()));
        let Some((imp_idx, _impl_subst)) = found_impl else {
            return Err(unsatisfied_user_bound_error(
                ctx,
                origin_span,
                &origin_name,
                &sig.ty_var_names[var as usize],
                trait_decl,
                ty,
                arrays,
                refs,
            ));
        };
        let imp = &tr.impls[imp_idx];
        let resolved_idx = imp
            .resolved
            .iter()
            .find(|(mname, _)| mname == member)
            .map(|(_, idx)| *idx);
        let symbol = resolved_idx.and_then(|idx| poly.trait_resolve.word_symbols.get(idx));
        let Some(symbol) = symbol else {
            return Err(unresolved_trait_obligation_error(
                ctx,
                origin_span,
                &origin_name,
                &trait_decl.name,
                member,
                ty,
                origin_span,
            ));
        };
        let symbol = symbol.clone();
        // P7b.S3 (S3-8, matrix row 7): a *generic* impl target's member word
        // is polymorphic -- it mints no `IrFunc` and has no `env` entry, so
        // handing its bare symbol to lowering's ordinary resolved-call path
        // panics (`checked resolved call exists`). A monomorph has to be
        // recorded instead, which is what `check_poly_call` does: it unifies
        // the member word's grounded S2-6 sig against these concrete slots
        // and writes the per-splice `CallInst` (symbol + θ_call). The same
        // route `resolve_mono_member_call`'s generic-impl branch takes, so
        // the inline and non-inline callers agree on the monomorph.
        //
        // A member that is itself a **combinator** reaches here only on the
        // *re-entry* path: the S3-1.c branch above routes every
        // `declares_inline` member onto `inline_combinator` EXCEPT one
        // already on the active splice chain (`prov.member_splice_stack`),
        // where re-splicing at check time would recurse. A cycling
        // generic-target member falls through to `check_poly_call` below; a
        // concrete-target one to the `splice_trait_calls` write at the end
        // of this block (which stores the synth name for exactly this case,
        // see there). Either way termination is lowering's splice-budget
        // guard, which reports the recursion.
        // P7b.S5 Phase 3 (R5): as at the non-inline call site above,
        // `poly_env` is whole-program (`src/check.rs:668-706`), not
        // per-module, so this guard's original "not visible from this
        // module" premise does not describe the current architecture.
        // Unlike the non-inline site, this one is not provably dead: it is
        // an INCONCLUSIVE reachability verdict. `check_combinator_cycles`
        // (`src/check/combinators.rs:186-189`) keys its rejection pass on
        // bare `c.word.name`, while a trait-member combinator is registered
        // under a synthesized `member;Trait;Type` spelling, so a member-to-
        // member call produces no edge in that graph and is not rejected
        // upstream by it -- but every fixture attempt at reaching this arm
        // (direct cross-member call, a bounded helper, a bare-forwarded
        // receiver, and self-recursion) was intercepted by a different,
        // earlier guard first (see the four attempts recorded beside
        // `mono_member_unroutable_error`'s doc comment and R5 in
        // `docs/roadmap/P7b/slice5-spec.md`). No live trigger is known.
        // P7b.S8 (phase 3): a lifted ctor target's `is_concrete()` is false,
        // so without this test the gate below would admit it and raise a
        // false "not visible from this module" on a member that dispatches
        // fine from a mono caller. Mirror `resolve_mono_member_call`'s
        // `lifted_mono` test (the non-inline lifted arm): the desugar
        // grounded the member word, so it is monomorphic, absent from
        // `poly_env`, and reached by bare symbol through `env` -- the record
        // path below. The word's own mono flag is the test rather than the
        // target shape alone: a lifted target whose member kept a free
        // variable stays polymorphic and still belongs to the generic branch.
        let lifted_mono = tr.impls[imp_idx].target.is_mono_ctor_app()
            && resolved_idx.is_some_and(|widx| tr.words[widx].poly.is_none());
        if !tr.impls[imp_idx].target.is_concrete() && !lifted_mono {
            if !poly.env.contains_key(&symbol) {
                return Err(mono_member_unroutable_error(
                    ctx,
                    span,
                    member,
                    &trait_decl.name,
                    &symbol,
                ));
            }
            let next = check_poly_call(
                &symbol,
                span,
                &[],
                &[],
                None,
                stack,
                ctx,
                env,
                scope,
                arrays,
                cells,
                refs,
                slices,
                prov,
                live,
                at,
                poly,
            )?;
            return Ok(Some(next));
        }
        // S4 spelling alignment: lowering's `(uid, span)` reader
        // (`ir/func_builder/calls.rs`) gates a member re-splice on
        // `member_splice_names`, which holds `WordDef::name` spellings (the
        // synth name), and its combinator lookup is keyed the same way. A
        // cycling `declares_inline` member that fell through the re-entry
        // check above must therefore record its synth name, not
        // `word_symbols[idx]` -- `overload_symbols` `$$`-suffixes a colliding
        // name, and a suffixed symbol misses both the combinator map and the
        // cycle gate (the env fallback then panics, since a combinator has no
        // env entry). A non-combinator member keeps the symbol: lowering
        // calls it through `env`, which is symbol-keyed.
        let record = match resolved_idx {
            Some(widx) if tr.words[widx].declares_inline => tr.words[widx].name.clone(),
            _ => symbol,
        };
        poly.splice_trait_calls.insert((uid, span), record);
    }
    // Consume operands, produce outputs. P7b.S6d-PREREQ (review round, P1-2):
    // measured, not assumed -- reverting this call to a bare `Slot::computed`
    // push survives the entire suite, and probing shows why. `bearing` (in
    // `push_dispatch_outputs`) is false unless the output type is itself
    // reference-bearing, and any non-`declares_inline` word with such an
    // output is rejected outright at word-check time
    // (`check_reference_free_signature`, no top-level exemption on the output
    // side). A `declares_inline` member reaching *this* push (rather than the
    // `mword.declares_inline && !re_entry` branch above, which diverts into
    // `inline_combinator` instead) requires `re_entry`, i.e. a member whose
    // own body bound-dispatches back to itself -- and every such shape tried
    // is a self-recursive call, which the self-tail-call back-edge hazard
    // (`poly_self_tail_backedge_hazards`) rejects before this push is ever
    // reached carrying a live borrow. No reference-bearing type has been
    // found that reaches this push with `bearing` true; if one is found later,
    // wire a golden through it.
    push_dispatch_outputs(stack, base, &output_types, name, span, ctx, arrays, prov)?;
    Ok(Some(std::mem::take(stack)))
}

/// P7b.S3 (S3-7): ground a trait member signature's slots at a splice's θ.
/// One rule for the single-candidate path and the disambiguation loop. The
/// fast path is the single-`Type` grounding (`try_ground_member_type`,
/// byte-identical for every shape it already grounded); a slot it cannot
/// ground -- an App-headed slot, or any slot when the bound variable carries
/// a bare `CtorImage` -- is grounded through the caller's θ instead: the
/// trait's header variable (member-sig id 0) is seeded with the bound
/// variable's binding and the slot is unified against the live operand
/// (S2-6's leading-slot rule), so the operand match decides what
/// `splice_member_hkt_error`'s fence used to refuse outright. A slot θ still
/// cannot ground is a located error -- `splice_member_ctor_image_error` for
/// the incomplete-`CtorImage` shape its text names -- never a panic and
/// never a silent skip.
#[allow(clippy::too_many_arguments)]
pub(super) fn ground_member_sig_via_theta(
    member_sig: &PolySig,
    member: &str,
    trait_name: &str,
    ty: Type,
    stack: &[Slot],
    span: Span,
    ctx: &Ctx,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<(Vec<Type>, Vec<Type>), String> {
    let mut theta = Subst::default();
    theta.ty.push((0, ty));
    let n_in = member_sig.inputs.len();
    let base = stack.len().saturating_sub(n_in);
    let ground = |pt: &PolyType,
                  operand: Option<Type>,
                  theta: &mut Subst,
                  arrays: &mut Vec<ArrayDecl>,
                  cells: &mut Vec<OwnedCellDecl>,
                  refs: &mut Vec<RefDecl>|
     -> Result<Type, String> {
        if let Some(t) = crate::ast::try_ground_member_type(pt, ty, arrays, refs) {
            return Ok(t);
        }
        if let Some(op) = operand {
            // A unification failure against a `CtorImage` stand-in is class
            // two (caused by the standalone check's arbitrary representative,
            // not by the body): raise the tagged error so
            // `check_poly_combinator_standalone` rescues it and the body is
            // re-checked at each real splice instead.
            unify_poly_input(
                member_sig,
                pt,
                op,
                member,
                span,
                ctx,
                arrays,
                cells,
                refs,
                theta,
                &[],
                &[],
            )
            .map_err(|e| {
                if matches!(ty, Type::CtorImage(..)) {
                    splice_member_ctor_image_error(ctx, span, member, trait_name, ty)
                } else {
                    e
                }
            })?;
        }
        match apply_subst(
            member_sig, pt, theta, member, span, ctx, arrays, cells, refs,
        ) {
            Ok(t) => Ok(t),
            Err(_) if matches!(ty, Type::CtorImage(..)) => Err(splice_member_ctor_image_error(
                ctx, span, member, trait_name, ty,
            )),
            Err(e) => Err(e),
        }
    };
    let mut inputs = Vec::with_capacity(n_in);
    for (i, pt) in member_sig.inputs.iter().enumerate() {
        let operand = stack.get(base + i).map(|s| s.ty);
        inputs.push(ground(pt, operand, &mut theta, arrays, cells, refs)?);
    }
    let mut outputs = Vec::with_capacity(member_sig.outputs.len());
    for pt in &member_sig.outputs {
        outputs.push(ground(pt, None, &mut theta, arrays, cells, refs)?);
    }
    Ok((inputs, outputs))
}

/// P7b.S3 (S3-7): θ does not bind the splice's bound variable at all, so
/// there is nothing to ground the member signature against. The bare-unbound
/// sibling of `splice_member_ctor_image_error` (which covers θ binding the
/// variable to a constructor that does not complete). Not tagged with
/// [`STAND_IN_GROUNDING_TAG`]: the standalone check seeds *every* declared
/// variable, so an unbound one is not something a stand-in's arbitrary
/// constructor choice can cause.
fn splice_member_unbound_var_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    var: &str,
) -> String {
    format!(
        "error: trait member `{member}` of `{trait_name}` in {where_} (line {}, col {}) cannot be dispatched: the splice's substitution leaves `{var}` unbound\n  a member call is grounded through the variable its bound sits on, so that variable must be determined by the call site's operands",
        span.line,
        span.col,
        where_ = ctx.rendered_word(),
    )
}

/// P7b.S2 (S2-15.f, splice guard b): a member signature grounded at a bound
/// variable that carries a bare `Type::CtorImage` -- the image an App
/// unification bound as a head, which is not itself a type. The S1-15.g twin
/// for the splice path: a located error instead of the unchecked Var arm's
/// silent misclassification.
///
/// P7b.S3 (S3-7, Phase 3): `splice_member_hkt_error` and the `ground_slots`
/// single-`Type` closure are deleted; member slots ground through the
/// caller's θ instead (`ground_member_sig_via_theta`). This error is now
/// raised **deliberately** from that grounding -- a unification or
/// `apply_subst` failure while the bound variable carries a bare `CtorImage`
/// -- never from a fence, and no fence exists. The [`STAND_IN_GROUNDING_TAG`]
/// is load-bearing: at the standalone check the `CtorImage` is S3-6's
/// arbitrary stand-in, so `check_poly_combinator_standalone` rescues the
/// tagged raise and the body is re-checked at each real splice; at a real
/// splice site the same raise is a hard located error (the tag is stripped
/// before rendering).
fn splice_member_ctor_image_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    ty: Type,
) -> String {
    let where_ = ctx.rendered_word();
    let ctor = match ty {
        Type::CtorImage(_, name) => name,
        _ => "a constructor image",
    };
    STAND_IN_GROUNDING_TAG.to_string() + &format!(
        "error: trait member `{member}` of `{trait_name}` in {where_} (line {}, col {}) is grounded against the constructor image `{ctor}`, which is not a type\n  the splice's bound variable carries a bare constructor (an applied head like `'F['T]`), and a member signature cannot ground against it here\n  instantiate the combinator at a complete type (e.g. `comb[Option]`), not at the constructor alone",
        span.line,
        span.col,
        where_ = where_,
    )
}

/// P7b.S2 (S2-16, mono caller): resolve a bare member word in a *monomorphic*
/// body -- the env.get-miss branch's member lookup, sitting after
/// `mint_fallback_candidates` (check-time monomorph mints take precedence)
/// and before the unchanged `unknown_word_error`/`ungated_intrinsic_error`
/// fallthrough, which still fires on no-match so every existing unknown-word
/// golden holds.
///
/// A bare member currently falls through `env.get` as an unknown word: the
/// implementing word is module-qualified (`synth_member_word_name`), so the
/// bare member name is in no `env`. The lookup keys on the member name plus
/// the operand's dispatchable type (S2-12/S2-5), through the whole-program
/// trait/impl tables: with fully concrete operand types it dispatches via
/// `find_bound_impl` on the operand's full grounded type -- the existing
/// `Concrete`/`Generic` arms compare and bind args, no `CtorImage` is
/// constructed for a mono call (S2-8's dispatch rule).
///
/// A generic-impl winner (the member word is a polymorphic word) routes
/// through `check_poly_call` under the member word's own (synthesized) name:
/// it unifies the word's grounded S2-6 sig against the concrete slots,
/// records `(word, θ_call)` as the span-keyed `CallInst` lowering emits --
/// the mono caller's `span → (symbol, θ_call)` record. A concrete-impl
/// winner (a monomorphic member word, no θ to mint) checks the slots
/// against the sig grounded at the target and records `span → symbol` in
/// `builtin_overloads`, the same pattern an operator overload rides.
///
/// No match -- the name is no trait's member -- returns `Ok(None)` and
/// ordinary dispatch proceeds. A name that *is* a member but dispatches to
/// nothing, or is claimed by several traits whose impls all fit, is a
/// located error naming the candidates.
#[allow(clippy::too_many_arguments)]
pub(in crate::check) fn resolve_mono_member_call(
    name: &str,
    span: Span,
    type_args: &[Type],
    len_args: &[Len],
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
) -> Result<Option<Vec<Slot>>, String> {
    let traits = poly.trait_resolve.traits;
    // R12/decision 6, same split as the splice path: a qualified call names a
    // module alias, never a trait namespace.
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
    // Candidate members by name across the whole-program trait registry,
    // deduped by trait id (a member name may be declared by several traits;
    // the operand's dispatch decides, as bound dispatch does).
    let mut candidates: Vec<(TraitId, &TraitMember)> = Vec::new();
    for (tidx, t) in traits.iter().enumerate() {
        if qualified_target.is_some_and(|q| t.module != q) {
            continue;
        }
        if let Some(m) = t.members.iter().find(|m| m.name == member) {
            let tid = TraitId::from_index(tidx);
            if !candidates.iter().any(|(id, _)| *id == tid) {
                candidates.push((tid, m));
            }
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    // Dispatch each candidate on its dispatchable input's operand type (S2-2:
    // the trait var bare or heading an application, under one `Ref` layer --
    // ref-ness is an addressing mode, so a ref slot dispatches on its
    // referent).
    let mut viable: Vec<(TraitId, &TraitMember, usize)> = Vec::new();
    for (tid, m) in &candidates {
        let Some(pos) = dispatchable_input_pos(&m.sig) else {
            continue;
        };
        if stack.len() < m.sig.inputs.len() {
            continue;
        }
        let base = stack.len() - m.sig.inputs.len();
        let mut operand = stack[base + pos].ty;
        if matches!(m.sig.inputs[pos], PolyType::Ref(..)) {
            if let Some((referent, _)) = ref_parts(operand, refs) {
                operand = referent;
            }
        }
        let mut visited: Vec<(TraitId, Type)> = Vec::new();
        if let Some((imp_idx, _)) = find_bound_impl(
            *tid,
            operand,
            None,
            span,
            ctx,
            &poly.trait_resolve,
            arrays,
            cells,
            refs,
            &mut visited,
        )? {
            viable.push((*tid, m, imp_idx));
        }
    }
    // P7b.S6 Phase 4 (R4): a member with no dispatchable input (a nullary
    // trait member such as `Monoid.empty`) can never win the loop above --
    // `dispatchable_input_pos` returned `None` for every such candidate, so
    // each was `continue`d. If exactly one candidate is of that shape and the
    // call site supplies an explicit type argument, that argument is the only
    // way to ground the trait's own type variable; ground it directly and run
    // the same `find_bound_impl` the operand path uses.
    //
    // P7b.S8b Phase 1 (R4): the impl-target equation `find_bound_impl`
    // already computed (`match_impl_target` of the target pattern against
    // the call-site type: `Opt['ctor0]` vs `Opt[i64]` gives `'ctor0 := i64`)
    // is kept, not discarded. For a *generic* target it is the only correct
    // seed for the member word's θ -- the member word's variables are the
    // impl target's own, member locals appended after (`build_member_var_union`,
    // `src/parser.rs:768`), so the call-site type belongs one level below
    // variable #0, not in it. Seeding it positionally minted
    // `Opt[Opt[i64]]`, whose variant words then clobbered lowering's
    // bare-name map and re-typed every `Some`/`Cons` in the program.
    let mut impl_target_seed: Option<Subst> = None;
    if viable.is_empty() {
        if let (Some(&ty), [(zero_tid, zero_m)]) = (
            type_args.first(),
            candidates
                .iter()
                .filter(|(_, m)| dispatchable_input_pos(&m.sig).is_none())
                .copied()
                .collect::<Vec<_>>()
                .as_slice(),
        ) {
            let mut visited: Vec<(TraitId, Type)> = Vec::new();
            if let Some((imp_idx, subst)) = find_bound_impl(
                *zero_tid,
                ty,
                None,
                span,
                ctx,
                &poly.trait_resolve,
                arrays,
                cells,
                refs,
                &mut visited,
            )? {
                viable.push((*zero_tid, *zero_m, imp_idx));
                impl_target_seed = Some(subst);
            }
        }
    }
    match viable.len() {
        0 => {
            if type_args.is_empty()
                && candidates
                    .iter()
                    .any(|(_, m)| dispatchable_input_pos(&m.sig).is_none())
            {
                return Err(mono_nullary_member_no_instantiation_error(
                    ctx, span, member,
                ));
            }
            return Err(mono_member_no_dispatch_error(
                ctx,
                span,
                member,
                &candidates
                    .iter()
                    .map(|(tid, _)| traits[tid.index()].name.as_str())
                    .collect::<Vec<_>>(),
                &stack.iter().map(|s| s.ty).collect::<Vec<_>>(),
            ));
        }
        1 => {}
        _ => {
            return Err(mono_ambiguous_member_error(
                ctx,
                span,
                member,
                &viable
                    .iter()
                    .map(|(tid, _, _)| traits[tid.index()].name.as_str())
                    .collect::<Vec<_>>(),
            ));
        }
    }
    let (trait_id, member_decl, imp_idx) = viable[0];
    let trait_name = &traits[trait_id.index()].name;
    let imp = &poly.trait_resolve.impls[imp_idx];
    let Some((_, widx)) = imp.resolved.iter().find(|(mname, _)| *mname == member) else {
        return Err(unresolved_trait_obligation_error(
            ctx,
            span,
            name,
            trait_name,
            member,
            stack.last().map(|s| s.ty).unwrap_or(Type::I64),
            span,
        ));
    };
    let word_sym = poly.trait_resolve.word_symbols[*widx].clone();
    // P7b.S8 (REQ-7, Delta B dispatch (a)): the lifted-target arm. A
    // fully-applied all-concrete ctor target (`for Range[i64]`) keeps a
    // `Generic` pattern, so `is_concrete()` is false, but its member word is
    // monomorphic (the desugar grounded it, `parse_impl_member_body`) and so
    // is absent from `poly_env` -- the else branch's `debug_assert!` below.
    // The word's own mono flag is the test rather than the target shape
    // alone, which fences this to exactly the words the desugar grounded:
    // a lifted target whose member kept a free variable stays polymorphic
    // and still belongs to the generic branch.
    let lifted_mono = imp.target.is_mono_ctor_app()
        && poly
            .trait_resolve
            .words
            .get(*widx)
            .is_some_and(|w| w.poly.is_none());
    if imp.target.is_concrete() || lifted_mono {
        // S2-16 (final-review fix): a concrete target's member sig has no
        // free variables to bind, so an explicit type/length-argument list
        // is provably meaningless here -- reject it instead of silently
        // dropping it. The generic branch below is the one that reads the
        // list (via `check_poly_call`'s θ seeding); the widened
        // `poly_call_takes_type_args` clause admits the spelling for member
        // names on its behalf.
        //
        // P7b.S6 Phase 4 (R4): narrow carve-out. A zero-dispatchable-input
        // member (`dispatchable_input_pos` is `None`) has no operand to
        // dispatch on at all, so an explicit type argument is the *only* way
        // to reach this branch in the first place -- it is not "meaningless",
        // it is load-bearing. `mono_concrete_member_call_with_explicit_type_args_is_error`
        // pins `size ( 'F -- i64 )`, which has a dispatchable input, so this
        // exception never touches it.
        if (!type_args.is_empty() || !len_args.is_empty())
            && dispatchable_input_pos(&member_decl.sig).is_some()
        {
            return Err(no_type_arguments_error(
                span,
                name,
                !type_args.is_empty(),
                !len_args.is_empty(),
            ));
        }
        // Concrete impl: the member word is monomorphic, grounded at the
        // target (the desugar did it). Check the slots against the grounded
        // sig and record `span -> symbol` in `builtin_overloads` -- the same
        // span-keyed lowering record an operator overload rides, which
        // `lower_call` reads ahead of the name-keyed `env` (the bare member
        // name has no `env` entry).
        // P7b.S8 (REQ-7): a lifted target's grounded effect is read off the
        // member word the desugar already built, not re-derived here. The
        // trait's own member signature cannot be re-grounded at one: it is
        // App-headed (that is the point of the lift) and
        // `ground_member_type`'s App arm is an `unreachable!` with no ctor
        // head to dissolve into. Reading the word is also the stronger
        // guarantee -- the effect a body was checked against and the effect
        // a call site is checked against are then one object, which is
        // exactly what `ground_member_type`'s shared-grounding rule (S3r R2)
        // buys the concrete path.
        let (input_types, output_types): (Vec<Type>, Vec<Type>) = match imp.target.concrete_ty() {
            Some(target_ty) => (
                member_decl
                    .sig
                    .inputs
                    .iter()
                    .map(|t| crate::ast::ground_member_type(t, target_ty, arrays, refs))
                    .collect(),
                member_decl
                    .sig
                    .outputs
                    .iter()
                    .map(|t| crate::ast::ground_member_type(t, target_ty, arrays, refs))
                    .collect(),
            ),
            None => {
                let word = &poly.trait_resolve.words[*widx];
                (
                    word.effect.inputs.iter().map(|s| s.ty).collect(),
                    word.effect.outputs.iter().map(|s| s.ty).collect(),
                )
            }
        };
        let n_in = input_types.len();
        if stack.len() < n_in {
            return Err(underflow_error(ctx, span, member, n_in, stack.len()));
        }
        let base = stack.len() - n_in;
        for (i, want) in input_types.iter().enumerate() {
            let found = stack[base + i];
            // P7b.S2 (S2-16): a declared `Type::Quotation` parameter is a
            // materialization boundary, exactly as the ordinary env word-call
            // path treats one (R8/D4): a `Known` literal fills the slot
            // materialized (a capturing one through the R15 admission rule,
            // an in-frame boundary), and an already-erased runtime quotation
            // value falls through to the ordinary `match_slot` (Exact) below.
            // The reject beneath keeps covering only slots the member does
            // not declare as a quotation.
            let declared = match *want {
                Type::Quotation(eff) => Some((eff, false)),
                Type::OwningQuotation(eff) => Some((eff, true)),
                _ => None,
            };
            if let Some((eff, owning)) = declared {
                if let Some(QuotRef::Known(id)) = found.quot {
                    stack[base + i] = materialize_quotation_at_boundary(
                        id, eff, owning, false, name, span, ctx, env, arrays, cells, refs, slices,
                        prov, scope, poly,
                    )?;
                    continue;
                }
            }
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
                    // P7b.S6 Phase 1 (R2.a) per-site verdict: this is the mono
                    // branch (a concrete target, or P7b.S8's lifted mono one),
                    // so both operands are already `Concrete` and
                    // `expected_sig`/`found_sig` share the same `member_decl.sig`
                    // -- the split is a no-op here.
                    return Err(trait_member_operand_error(
                        ctx,
                        span,
                        member,
                        trait_name,
                        &PolyType::Concrete(*want),
                        &member_decl.sig,
                        &PolyType::Concrete(found.ty),
                        &member_decl.sig,
                        i,
                    ));
                }
            }
        }
        // A polymorphic word consumes its operands exactly as a concrete one
        // does (`check_poly_call`'s own guard): the same move/borrow
        // discipline applies to the member word's consumption.
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
        poly.builtin_overloads.insert(span, word_sym);
        push_dispatch_outputs(stack, base, &output_types, name, span, ctx, arrays, prov)?;
        Ok(Some(std::mem::take(stack)))
    } else {
        // Generic impl: the member word is polymorphic, its grounded S2-6 sig
        // the unification source. `check_poly_call` under the word's own
        // synthesized name does the rest: unification against the concrete
        // slots, the word's own where-bounds at θ_call, and the span-keyed
        // `CallInst` (symbol + θ_call) lowering emits.
        // P7b.S5 Phase 3 (R5): this guard's premise -- a per-module `poly_env`
        // that a found impl's member word might not be visible from -- does not
        // exist. `poly_env` is built once, whole-program, over the fully
        // `assemble_module`-flattened `Module` (`src/check.rs:668-706`), so a
        // member word that reaches this branch (i.e. `find_bound_impl` already
        // matched an impl) is always present. Every cross-module attempt is
        // intercepted upstream, at the zero-viable-candidates branch, by
        // `mono_member_no_dispatch_error` instead (see
        // `cross_module_colliding_mono_call_is_no_dispatch_error`,
        // `tests/phase7b_slice5.rs`).
        debug_assert!(
            poly.env.contains_key(&word_sym),
            "a dispatched impl's member word is always in the whole-program poly_env"
        );
        let next = check_poly_call(
            &word_sym,
            span,
            type_args,
            len_args,
            impl_target_seed.as_ref(),
            stack,
            ctx,
            env,
            scope,
            arrays,
            cells,
            refs,
            slices,
            prov,
            live,
            at,
            poly,
        )?;
        Ok(Some(next))
    }
}

/// P7b.S2 (S2-16, mono caller): the member name exists in the trait registry
/// but no impl of any declaring trait dispatches on these operands.
fn mono_member_no_dispatch_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_names: &[&str],
    operands: &[Type],
) -> String {
    let ops: Vec<String> = operands.iter().map(|t| t.name().to_string()).collect();
    format!(
        "error: `{member}` in {name} (line {}, col {}) is a trait member of {}, but no `impl:` in this program dispatches on these operands\n  the operand types here are `{}`; declare an impl of one of those traits for the operand's type, or import a word that claims this name",
        span.line,
        span.col,
        trait_names.join(", "),
        ops.join(" "),
        name = ctx.rendered_word(),
    )
}

/// P7b.S6 Phase 4 (R5): bare `empty`-shaped call from a mono body -- a
/// zero-dispatchable-input member with no explicit type argument to ground
/// its trait variable. Q1 rules out consuming-context inference for this
/// slice, so this is a located error naming the remedy, not a lookahead.
fn mono_nullary_member_no_instantiation_error(ctx: &Ctx, span: Span, member: &str) -> String {
    format!(
        "error: `{member}` in {name} (line {}, col {}) is a trait member with no operand to dispatch on\n  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `{member}[i64]`",
        span.line,
        span.col,
        name = ctx.rendered_word(),
    )
}

/// P7b.S2 (S2-16, mono caller). `poly_env` is in fact built once,
/// whole-program (`src/check.rs:668-706`), so every member word a found impl
/// dispatches to is present in it -- the non-inline call site (S2-16's
/// generic-impl branch of `resolve_mono_member_call`) has no live caller
/// today and asserts this with a `debug_assert!` instead (P7b.S5 Phase 3, R5).
///
/// The remaining call site -- the inline re-entry path, reached only when a
/// `declares_inline` trait member calls another (or itself) while already on
/// the active splice stack -- is INCONCLUSIVE (P7b.S5 Phase 3, R5). Four
/// fixture attempts, none of which reached it:
///   1. direct cross-member call (`ping` calling `pong` by name in the same
///      impl body): `error: unknown word `pong` in `ping` (member of trait
///      `Foo` for `Box['T0]`)` -- members aren't visible to each other by
///      bare name.
///   2. via a bounded helper (`ping` calling a `['T: Foo] ( 'T -- 'T )`
///      helper with the receiver): `error: `ping` (member of trait `Foo` for
///      `Box['T0]`) cannot pass `Box['T]` to `'T` of the polymorphic word
///      `helper`` -- a poly call site may pass a type variable only bare.
///   3. forwarding the receiver bare (`impl: Foo for 'T` calling the same
///      bounded helper): `error: `'T` of `helper` requires `Foo`, which `'T`
///      in `ping` (member of trait `Foo` for `'T0`) does not declare` -- the
///      member's own header lacks the bound its body's call requires.
///   4. (novel) direct self-recursion (`ping` calling `ping`): `error:
///      `ping;Foo;0;Box['T0]` in `ping` (member of trait `Foo` for
///      `Box['T0]`) names the generic type `Box['T]`, which cannot yet be
///      instantiated at a variable-bearing application` -- grounding a
///      generic over its own type variable is a separate, unimplemented
///      case.
///
/// Every attempt is intercepted by a distinct upstream guard before reaching
/// this call site. See `docs/roadmap/P7b/slice5-spec.md` R5.
fn mono_member_unroutable_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_name: &str,
    word: &str,
) -> String {
    format!(
        "error: `{member}` of `{trait_name}` in {name} (line {}, col {}) resolves to the member word `{word}`, which is not visible from this module\n  a monomorphic body calls the member word of the impl that declared it; declare the impl in a module this one can see",
        span.line,
        span.col,
        name = ctx.rendered_word(),
    )
}

/// P7b.S2 (S2-16, mono caller): several traits declare a member of this
/// name and an impl of each fits the concrete operand at this call. Not
/// `ambiguous_trait_member_error`'s case: there the call sits under a
/// variable carrying both bounds, and the remedy is the poly-bounds one; a
/// mono body has no bounds to consult -- the dispatch is on the operand's
/// concrete type, and the module qualifier (R12/decision 6: a qualifier
/// names a module, never a trait namespace) is what selects the trait whose
/// impl is meant.
fn mono_ambiguous_member_error(
    ctx: &Ctx,
    span: Span,
    member: &str,
    trait_names: &[&str],
) -> String {
    let quoted: Vec<String> = trait_names.iter().map(|t| format!("`{t}`")).collect();
    let listed = if trait_names.len() == 2 {
        format!("both {}", joined_with_and(&quoted))
    } else {
        joined_with_and(&quoted)
    };
    format!(
        "error: `{member}` in {name} (line {}, col {}) is a trait member of {listed}\n  an `impl:` of each claiming trait dispatches on this call's operand; qualify the call with the claiming trait's module (`module::{member}`) to name the one you mean",
        span.line,
        span.col,
        name = ctx.rendered_word(),
    )
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
pub(super) fn poly_trait_member_call(
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
                return Err(no_candidate_fits_operands_error(
                    ctx,
                    span,
                    member,
                    &shapes,
                    &[],
                ));
            }
        },
    };
    // P7b.S2 (S2-16, poly caller): the operand check is one-way unification
    // of the trait's *declared* member sig against the caller's slots -- not
    // structural equality over the `substitute_member_var` rewrite. The
    // declared sig's App-headed dispatchable input (head = the abstract
    // header variable) unifies against the caller's App-headed operand slot,
    // binding the member's header variable to the caller's bound variable and
    // member locals to the caller's slot arguments; the remaining slots
    // (member's quotation params, extra inputs) unify positionally. The
    // member word's grounded `PolySig` is deliberately NOT the source here:
    // at body-check no impl is selected and the word's sig is ctor-headed
    // (S2-6 dissolved `'F`), so unifying it would concretize the caller's
    // bound variable. The success record is the obligation PLUS the call
    // site's slot record (`TraitObligation::slots`, S2-9's data path); the
    // mint is deferred to the resolve loop, where the tie-break and θ_call
    // construction are per call site.
    let inputs = &member_decl.sig.inputs;
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
    // The header pre-binds to the dispatched bound variable (see
    // `unify_member_operand`'s doc): the dispatchable input's App head must
    // be this variable, and a Star member's bare-var input must be it
    // exactly -- the old rewrite-equality behavior, now as a binding.
    let mut bindings: Vec<(u32, PolyType)> = vec![(0u32, PolyType::Var(var))];
    for (i, (declared, slot)) in inputs.iter().zip(&stack[base..]).enumerate() {
        if !unify_member_operand(declared, &slot.pt, &mut bindings) {
            // P7b.S6 Phase 1 (R2.a, M3): `declared` is rendered raw, against
            // the member's own sig -- never rewritten via
            // `substitute_member_var` into the caller's variable space. That
            // rewrite left a member-local `Quotation`'s interior vars
            // untouched (`substitute_member_var`'s `other => other.clone()`
            // arm), so a member-local `Var(n)` could index off the end of
            // the caller `sig`'s `ty_var_names` and panic in `poly_type_str`.
            // Because `declared` is pure member-space and the caller's
            // operand slot is pure caller-space, each renders against its own
            // sig and no index can run off either table.
            return Err(trait_member_operand_error(
                ctx,
                span,
                member,
                &traits[trait_id.index()].name,
                declared,
                &member_decl.sig,
                &slot.pt,
                sig,
                i,
            ));
        }
    }
    let site_slots: Vec<PolyType> = stack[base..].iter().map(|s| s.pt.clone()).collect();
    stack.truncate(base);
    for out in &member_decl.sig.outputs {
        stack.push(PolySlot::new(render_member_decl(out, &bindings, var)));
    }
    tctx.obligations.push(TraitObligation {
        span,
        var,
        trait_id,
        member: member.to_string(),
        slots: site_slots,
    });
    Ok(Some(std::mem::take(stack)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

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
        let recorded = crate::check::check_module(&mut module)?;
        Ok((module, recorded))
    }

    fn probe_word() -> WordDef {
        crate::test_support::bare_word("probe", 0)
    }

    /// `probe_word`'s `Ctx`. Separate from `probe_word` because `word_ctx`
    /// borrows the `WordDef`, so the caller has to own it.
    fn probe_ctx(word: &WordDef) -> Ctx<'_> {
        word_ctx(word, &[], &[], &[], None, &CombinatorIndex::new(), None)
    }

    /// P7b.S3 Phase 2 fixtures: a Functor trait over two structurally
    /// identical single-parameter enums, `Opt` implemented first and `Opt2`
    /// second, in that source order.
    fn functor_two_impls_src(extra: &str) -> String {
        format!(
            "type: Opt['T] | None | Some 'T ;\n\
             type: Opt2['T] | None2 | Some2 'T ;\n\
             trait: Functor['F: * -> *] :\n\
             map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Opt\n\
             : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;\n\
             ;\n\
             impl: Functor for Opt2\n\
             : map swap ~[ ( Some2 ) Some2> swap call Some2 ] ~[ ( None2 ) drop drop None2 ] Opt2? ;\n\
             ;\n\
             {extra}\n\
             : main ( -- ) ;\n"
        )
    }

    #[test]
    fn unify_member_operand_bridges_a_concrete_quotation_and_binds_var() {
        // P7b.S6 Phase 2 (R2.b): a declared member-space `[ 'T -- 'U ]`
        // against a caller's fully-folded `Concrete(Type::Quotation([i64 --
        // i64]))` bridges through the new arm and binds `'T`/`'U` to `i64`
        // each -- the shape `bump`'s forwarded concrete-effect parameter
        // takes at `map`'s call site.
        let declared = PolyType::Quotation(
            vec![PolyType::Var(1)],
            vec![PolyType::Var(2)],
            false,
            None,
            None,
        );
        let found =
            PolyType::Concrete(crate::ast::quotation_type(vec![Type::I64], vec![Type::I64]));
        let mut bindings = Vec::new();
        assert!(unify_member_operand(&declared, &found, &mut bindings));
        assert!(bindings.contains(&(1, PolyType::Concrete(Type::I64))));
        assert!(bindings.contains(&(2, PolyType::Concrete(Type::I64))));
    }

    #[test]
    fn unify_member_operand_rejects_an_arity_mismatched_concrete_quotation() {
        // A declared one-input/one-output quotation against a found
        // zero-input/one-output one is a row-arity mismatch, not a bind.
        let declared = PolyType::Quotation(
            vec![PolyType::Var(1)],
            vec![PolyType::Var(2)],
            false,
            None,
            None,
        );
        let found = PolyType::Concrete(crate::ast::quotation_type(Vec::new(), vec![Type::I64]));
        let mut bindings = Vec::new();
        assert!(!unify_member_operand(&declared, &found, &mut bindings));
    }

    #[test]
    fn unify_member_operand_rejects_a_quotation_flavour_mismatch() {
        // A declared `~` (inline) quotation against a found plain
        // `Concrete(Type::Quotation)` is a flavour mismatch; so is a found
        // `Concrete(Type::OwningQuotation)` against either declared flavour,
        // since a member's declared quotation param can only ever be spelled
        // plain or `~`, never "owning".
        let declared_inline =
            PolyType::Quotation(vec![PolyType::Var(1)], Vec::new(), true, None, None);
        let found_plain =
            PolyType::Concrete(crate::ast::quotation_type(vec![Type::I64], Vec::new()));
        assert!(!unify_member_operand(
            &declared_inline,
            &found_plain,
            &mut Vec::new()
        ));

        let declared_plain =
            PolyType::Quotation(vec![PolyType::Var(1)], Vec::new(), false, None, None);
        let found_owning = PolyType::Concrete(crate::ast::owning_quotation_type(
            vec![Type::I64],
            Vec::new(),
        ));
        assert!(!unify_member_operand(
            &declared_plain,
            &found_owning,
            &mut Vec::new()
        ));
    }

    #[test]
    fn unify_member_operand_rejects_a_literal_quotation_operand() {
        // P7b.S7 REQ-11 (Phase 3 measurement): the fallback (`declared ==
        // found`) has no case pairing a declared `Quotation` against a
        // caller's `PolyType::QuotLit` -- measured real against `bind`'s
        // dogfood shape written directly at a poly member call site
        // (`error: ... expects ... found a quotation literal in operand
        // slot 1`). **Not fixed, deliberately deferred**: admitting this
        // arm would regress `tests/phase7b_slice6.rs`'s
        // `poly_body_quotation_literal_member_operand_is_located_error`
        // (S6, M3/R2.b) -- but that test's own docstring scopes the
        // rejection to "no materialization is attempted at a poly member
        // call site *this slice*", i.e. a slice-scoped stopgap, not a
        // permanent contract. Whether to admit a materialized literal here
        // is S6's call to adjudicate, not this phase's. What *is* settled
        // this phase: `bind`'s own dogfood (`4 Some [ half ] bind`) never
        // reaches this arm at all -- it is a mono (non-generic) call,
        // resolved through explicit instantiation (`bind[i64 i64]`), a
        // wholly different path that never produces a `QuotLit` slot. That
        // measurement is why this phase adds no arm here.
        let declared = PolyType::Quotation(
            vec![PolyType::Var(1)],
            vec![PolyType::App {
                head: 0,
                args: vec![PolyType::Var(2)],
            }],
            false,
            None,
            None,
        );
        let mut bindings = vec![(0, PolyType::Var(10))];
        assert!(!unify_member_operand(
            &declared,
            &PolyType::QuotLit,
            &mut bindings
        ));
    }

    #[test]
    fn unify_member_operand_app_row_slot_binds_dispatched_ctor() {
        // P7b.S7 REQ-5 (verification only, no new logic): the caller path
        // for `bind`'s row-nested `'F['U]` output -- an App inside a
        // Quotation's outs row -- reaches the same App/App arm a plain-slot
        // member (`map`) already exercises. The header is seeded per the
        // dispatch doc comment: `(0, Var(dispatch_var))`, the member header
        // bound to the caller's own bound type variable. Unification must
        // then bind the App's declared head (id 0) to the caller's found
        // head only when they agree (`dispatch_var`), and the row's local
        // (`'U`, id 2) to the caller's slot argument.
        let dispatch_var = 10;
        let caller_u = 11;
        let declared = PolyType::Quotation(
            vec![PolyType::Var(1)],
            vec![PolyType::App {
                head: 0,
                args: vec![PolyType::Var(2)],
            }],
            false,
            None,
            None,
        );
        let found = PolyType::Quotation(
            vec![PolyType::Var(20)],
            vec![PolyType::App {
                head: dispatch_var,
                args: vec![PolyType::Var(caller_u)],
            }],
            false,
            None,
            None,
        );
        let mut bindings = vec![(0, PolyType::Var(dispatch_var))];
        assert!(unify_member_operand(&declared, &found, &mut bindings));
        assert!(bindings.contains(&(2, PolyType::Var(caller_u))));

        // Negative case: the found operand's row-nested App head disagrees
        // with the seeded dispatch-header binding -- unification must reject
        // it, proving the seed actually gates agreement rather than being
        // silently overwritten.
        let other_head = 99;
        assert_ne!(other_head, dispatch_var);
        let found_disagreeing = PolyType::Quotation(
            vec![PolyType::Var(20)],
            vec![PolyType::App {
                head: other_head,
                args: vec![PolyType::Var(caller_u)],
            }],
            false,
            None,
            None,
        );
        let mut bindings_disagreeing = vec![(0, PolyType::Var(dispatch_var))];
        assert!(!unify_member_operand(
            &declared,
            &found_disagreeing,
            &mut bindings_disagreeing
        ));
    }

    #[test]
    fn render_member_decl_app_row_slot_renders_into_caller_space() {
        // P7b.S7 REQ-5 (verification only, no new logic): the same
        // row-nested App, rendered back through the binding map the unify
        // test above produces -- `bind`'s declared output `'F['U]` renders
        // into the caller's own `dispatch_var`/`caller_u` space, exactly as
        // `map`'s plain-slot output already does.
        let dispatch_var = 10;
        let caller_u = 11;
        let declared_output = PolyType::Quotation(
            vec![PolyType::Var(1)],
            vec![PolyType::App {
                head: 0,
                args: vec![PolyType::Var(2)],
            }],
            false,
            None,
            None,
        );
        let bindings = vec![
            (0, PolyType::Var(dispatch_var)),
            (2, PolyType::Var(caller_u)),
        ];
        let rendered = render_member_decl(&declared_output, &bindings, dispatch_var);
        let PolyType::Quotation(_, outs, ..) = rendered else {
            panic!("rendered declared output should stay a Quotation");
        };
        assert_eq!(
            outs[0],
            PolyType::App {
                head: dispatch_var,
                args: vec![PolyType::Var(caller_u)],
            },
            "the row-nested App renders back into the caller's own variable space"
        );
    }

    /// P7b.S3 (S3-7, Phase 3): `splice_member_hkt_error` is deleted, so an
    /// App-headed member call in a combinator body no longer hits a fence at
    /// the standalone check. Grounding it against the `CtorImage` stand-in
    /// raises the *tagged* `splice_member_ctor_image_error` -- class 2, a
    /// failure the stand-in's arbitrary constructor choice causes -- and the
    /// standalone check rescues it: the word is accepted and its body is
    /// re-checked at every real splice site instead.
    #[test]
    fn class_two_member_call_is_rescued_for_recheck_at_the_splice() {
        let src = functor_two_impls_src(
            ": apply inline['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map ;",
        );
        let res = check_src(&src);
        assert!(
            res.is_ok(),
            "an App-headed member call is rescued at the standalone check \
             (re-checked per splice), not fenced: {res:?}"
        );
        // The acceptance above must be the *tagged rescue*, not a silent
        // pass: grounding this member sig at a `CtorImage` stand-in (what the
        // standalone check hands S3-6's Arrow-kinded variable) raises exactly
        // the [`STAND_IN_GROUNDING_TAG`]-stamped `splice_member_ctor_image_
        // error` the rescue strips -- an untagged failure stays hard
        // (`standalone_arity_error_is_not_rescued`).
        let tokens = lex(&src).unwrap();
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
        let sig = module
            .traits
            .iter()
            .find(|t| t.name == "Functor")
            .expect("the fixture declares Functor")
            .members[0]
            .sig
            .clone();
        let idx = module
            .generics
            .enums
            .iter()
            .position(|e| e.name == "Opt")
            .expect("the fixture declares Opt") as u32;
        let stand_in = crate::ast::ctor_image_type(
            &module.generics,
            crate::ast::GenericId {
                is_enum: true,
                idx,
                module: 0,
            },
        );
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let (mut arrays, mut cells, mut refs) = (Vec::new(), Vec::new(), Vec::new());
        let err = ground_member_sig_via_theta(
            &sig,
            "map",
            "Functor",
            stand_in,
            &[],
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect_err("the CtorImage stand-in cannot ground the App-headed member sig");
        let (tagged, msg) = strip_stand_in_tag(err);
        assert!(
            tagged,
            "the stand-in grounding failure carries STAND_IN_GROUNDING_TAG -- the \
             rescue route the acceptance above rode: {msg}"
        );
        assert!(
            msg.contains("map") && msg.contains("constructor image"),
            "the tagged raise is splice_member_ctor_image_error: {msg}"
        );
    }
}
