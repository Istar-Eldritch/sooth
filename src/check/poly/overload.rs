use super::*;
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
///
/// P7b.S5 (R4 audit VERDICT): not reachable for the ctor-collision shape.
/// `candidates` here are declared `PolySig`s of overloaded *polymorphic
/// words* (a different registry than the generated-ctor `Overload`s
/// `struct_generated_sigs`/`enum_generated_sigs`/`variant_generated_sigs`
/// populate), and `PolySig` carries no `module` field to disambiguate on --
/// the ctor-collision fix does not apply to this candidate shape at all.
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
pub(in crate::check) fn poly_sig_could_match(
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
///
/// P7b.S5 (R4 audit VERDICT): not reachable for the ctor-collision shape.
/// `candidates` here are `Combinator`s -- always-spliced words indexed by
/// `CombinatorIndex`, never a generated ctor -- a distinct registry from the
/// `Overload`s the S5 fix disambiguates; a same-shaped generic ctor never
/// registers as a combinator.
pub(in crate::check) fn resolve_combinator_overload<'a>(
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
pub(in crate::check) fn no_combinator_overload_matches_error(
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
