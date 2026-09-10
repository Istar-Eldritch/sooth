use super::*;
/// P7b.S8c (D1/REQ-1, P1-1): the per-site theta composition the generic-mint
/// arm above extends the impl-target match subst with -- fence the written
/// quotation literal, re-ground the obligation's slots through the caller's
/// concrete subst, unify each against the member word's declared inputs, then
/// fail closed if any signature variable is still unbound before minting.
/// Extracted so the wiring between `unify_poly_input`'s per-slot errors,
/// `first_unbound_sig_var`'s fail-closed tail and the sort invariant below it
/// is exercised directly rather than only through its two pure halves.
#[allow(clippy::too_many_arguments)]
pub(in crate::check) fn compose_member_theta(
    word_sig: &PolySig,
    word_sym: &str,
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
    base: Subst,
) -> Result<Subst, String> {
    fence_quotation_literal_slot(ctx, ob, trait_name)?;
    let site_slots: Vec<Type> = ob
        .slots
        .iter()
        .map(|s| apply_subst(sig, s, caller_subst, name, span, ctx, arrays, cells, refs))
        .collect::<Result<Vec<_>, _>>()?;
    let mut theta = base;
    for (input, slot) in word_sig.inputs.iter().zip(&site_slots) {
        // Empty seed lists: `seeded`/`seeded_len` only select which wording
        // renders a conflict, and the prior binding here comes from the
        // impl-target *match*, not a written `[...]` instantiation -- so the
        // unseeded `poly_var_conflict_error` is the honest provenance. The
        // bindings themselves ride in `theta`.
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
        )?;
    }
    // Fail-closed: `concrete_effect` substitutes outputs through the same
    // closure as inputs, so the whole variable space has to be bound. REQ-3
    // keeps `driver.rs:579` untouched, which makes this the only fence
    // against that panic on this route.
    if let Some(unbound) = first_unbound_sig_var(word_sig, &theta) {
        return Err(member_unbound_variable_error(
            ctx, ob.span, &ob.member, trait_name, &unbound,
        ));
    }
    // The P7.S3t sort invariant: canonical order at construction, or two
    // sites mint two symbols for one (word, θ).
    theta.ty.sort_by_key(|(v, _)| *v);
    theta.len.sort_by_key(|(v, _)| *v);
    Ok(theta)
}

/// P7b.S8c (REQ-1): the first variable of `sig` -- across **inputs and
/// outputs** alike, since `concrete_effect` substitutes both through the same
/// closure -- that `theta` does not bind, rendered as its declared name. The
/// walk enumerates `PolyType`'s variants rather than the slot lists alone:
/// `Len::Var` hides inside an `Array`/`Generic`, and an `App`'s head is a
/// variable in its own right. Rows are excluded deliberately: a row is a
/// caller-side pass-through that `concrete_effect` never materializes.
pub(in crate::check) fn first_unbound_sig_var(sig: &PolySig, theta: &Subst) -> Option<String> {
    fn walk(pty: &PolyType, sig: &PolySig, theta: &Subst) -> Option<String> {
        let ty_var = |v: u32| {
            theta.ty_of(v).is_none().then(|| {
                sig.ty_var_names
                    .get(v as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("'{v}"))
            })
        };
        let len_var = |l: &Len| match l {
            Len::Concrete(_) => None,
            Len::Var(ln) => theta.len_of(*ln).is_none().then(|| {
                sig.len_var_names
                    .get(*ln as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("'{ln}"))
            }),
        };
        match pty {
            PolyType::Concrete(_) | PolyType::QuotLit => None,
            PolyType::Var(v) => ty_var(*v),
            PolyType::Array(elem, len) => walk(elem, sig, theta).or_else(|| len_var(len)),
            PolyType::Ref(inner, _) | PolyType::OwnedCell(inner) => walk(inner, sig, theta),
            PolyType::Quotation(ins, outs, ..) => {
                ins.iter().chain(outs).find_map(|p| walk(p, sig, theta))
            }
            PolyType::App { head, args } => {
                ty_var(*head).or_else(|| args.iter().find_map(|p| walk(p, sig, theta)))
            }
            PolyType::Generic { args, len_args, .. }
            | PolyType::GenericVariant { args, len_args, .. } => args
                .iter()
                .find_map(|p| walk(p, sig, theta))
                .or_else(|| len_args.iter().find_map(len_var)),
        }
    }
    sig.inputs
        .iter()
        .chain(&sig.outputs)
        .find_map(|p| walk(p, sig, theta))
}

/// only to tell the two conflicts apart: two operands disagreeing is a
/// symmetric "resolved `'T` to both", while an operand disagreeing with an
/// instantiation has a written end and an inferred end, and the user needs to
/// know which is which. Passed rather than recorded on `Subst`, since nothing
/// downstream compares substitutions -- the symbol is rendered from `ty`/`len`
/// in vector order -- so a provenance field there would be inert.
#[allow(clippy::too_many_arguments)]
pub(in crate::check) fn unify_poly_input(
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
    seeded_len: &[u32],
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
                sig, elem, elem_ty, name, span, ctx, arrays, cells, refs, subst, seeded, seeded_len,
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
                            let var = &sig.len_var_names[*ln as usize];
                            return Err(match seeded_len.contains(ln) {
                                true => explicit_len_instantiation_conflict_error(
                                    ctx, span, name, var, prev, count,
                                ),
                                false => poly_len_conflict_error(ctx, span, name, var, prev, count),
                            });
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
                    sig, p, *c, name, span, ctx, arrays, cells, refs, subst, seeded, seeded_len,
                )?;
            }
            for (p, c) in outs.iter().zip(&eff.outputs) {
                unify_poly_input(
                    sig, p, *c, name, span, ctx, arrays, cells, refs, subst, seeded, seeded_len,
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
                seeded, seeded_len,
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
                seeded, seeded_len,
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
        // P7.S6a (R8a): `len_args` binds against `subst.len` the same way
        // the `Array` arm's own `Len::Var` does, one position per declared
        // length argument.
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            len_args,
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
            let Some((found_idx, found_module, found_args, found_lens)) = found else {
                return Err(mismatch());
            };
            if found_idx != *idx as usize
                || found_module != *module
                || found_args.len() != args.len()
                || found_lens.len() != len_args.len()
            {
                return Err(mismatch());
            }
            let found_args = found_args.to_vec();
            let found_lens = found_lens.to_vec();
            drop(generics);
            for (arg_pty, arg_ty) in args.iter().zip(found_args.iter()) {
                unify_poly_input(
                    sig, arg_pty, *arg_ty, name, span, ctx, arrays, cells, refs, subst, seeded, seeded_len,
                )?;
            }
            for (len, found_len) in len_args.iter().zip(found_lens.iter()) {
                let Len::Concrete(found_len) = found_len else {
                    unreachable!("a minted header instantiation's own length arguments are always concrete")
                };
                let found_len = *found_len;
                match len {
                    Len::Concrete(k) => {
                        if *k != found_len {
                            return Err(mismatch());
                        }
                    }
                    Len::Var(ln) => {
                        if let Some(prev) = subst.len_of(*ln) {
                            if prev != found_len {
                                let var = &sig.len_var_names[*ln as usize];
                                return Err(match seeded_len.contains(ln) {
                                    true => explicit_len_instantiation_conflict_error(
                                        ctx, span, name, var, prev, found_len,
                                    ),
                                    false => poly_len_conflict_error(
                                        ctx, span, name, var, prev, found_len,
                                    ),
                                });
                            }
                        } else {
                            subst.len.push((*ln, found_len));
                        }
                    }
                }
            }
        }
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, which `pty` (the callee's *declared* input) never is.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
        // S1-10: decompose an application against a concrete slot -- the
        // slot must be an instantiation of *some* generic header (struct or
        // enum), whose identity binds `head` to a `Type::CtorImage` in the
        // existing `ty` map, then each applied argument unifies positionally
        // against the recovered concrete argument, exactly as the `Generic`
        // arm above does for a fully-named header.
        PolyType::App { head, args } => {
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
            let (found_is_enum, found_idx, found_module, found_args, found_lens) = match slot_ty {
                Type::Struct(id, _) => {
                    let Some((fi, fm, fargs, flens)) = generics.struct_instantiation_of(id)
                    else {
                        return Err(mismatch());
                    };
                    (false, fi, fm, fargs.to_vec(), flens.to_vec())
                }
                Type::Enum(id, _) => {
                    let Some((fi, fm, fargs, flens)) = generics.enum_instantiation_of(id) else {
                        return Err(mismatch());
                    };
                    (true, fi, fm, fargs.to_vec(), flens.to_vec())
                }
                _ => return Err(mismatch()),
            };
            // S1-7: `App` carries type arguments only -- a `Len`-domain
            // application is fenced to S2+. The concrete slot recovered
            // above may still be an instantiation of a length-parameterized
            // header (`Buf['T 'N: Len]`), which an `App` binding can never
            // re-mint (it has no length argument to supply, unlike the
            // `Generic` arm above, which reads `len_args` off the callee's
            // own declared signature). A located error, not the panic that
            // `apply_subst`'s later `instantiate_*(&[]` call would hit.
            if !found_lens.is_empty() {
                let var = &sig.ty_var_names[*head as usize];
                let ctor_name = if found_is_enum {
                    generics.enums[found_idx].name.clone()
                } else {
                    generics.structs[found_idx].name.clone()
                };
                drop(generics);
                return Err(poly_app_len_domain_unsupported_error(
                    ctx, span, name, var, &ctor_name,
                ));
            }
            if found_args.len() != args.len() {
                drop(generics);
                return Err(mismatch());
            }
            let gid = GenericId {
                is_enum: found_is_enum,
                idx: found_idx as u32,
                module: found_module,
            };
            let ctor = crate::ast::ctor_image_type(&generics, gid);
            drop(generics);
            if let Some(prev) = subst.ty_of(*head) {
                if prev != ctor {
                    let var = &sig.ty_var_names[*head as usize];
                    return Err(match seeded.contains(head) {
                        true => explicit_instantiation_conflict_error(ctx, span, name, var, prev, ctor),
                        false => poly_var_conflict_error(ctx, span, name, var, prev, ctor),
                    });
                }
            } else {
                let pos = subst.ty.partition_point(|(id, _)| *id < *head);
                subst.ty.insert(pos, (*head, ctor));
            }
            for (arg_pty, arg_ty) in args.iter().zip(found_args.iter()) {
                unify_poly_input(
                    sig, arg_pty, *arg_ty, name, span, ctx, arrays, cells, refs, subst, seeded,
                    seeded_len,
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

/// S1-7: `PolyType::App` carries type arguments only -- a `Len`-domain
/// application is fenced to S2+ ("any application attempt raises S1-15.h's
/// arity/kind-mismatch diagnostic"). This is that diagnostic's non-
/// annotation twin: the head variable's binding turned out to be a
/// constructor that itself declares a length parameter, which no `App`
/// (type arguments only) can ever supply. Located, not a panic.
pub(super) fn poly_app_len_domain_unsupported_error(
    ctx: &Ctx,
    span: Span,
    op: &str,
    var: &str,
    ctor_name: &str,
) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{op}` in {where_} (line {}) applies `{var}` to the constructor `{ctor_name}`, which declares a length parameter\n  a type-variable application (`{var}[...]`) supplies type arguments only; a length-parameterized constructor is not supported here",
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
/// The poly-body twin of `constructed_reference_error` (`check.rs`): a
/// construction site with no declaration for `check_no_stored_references` to
/// have caught, reached from a generic body where the payload may still
/// carry an unbound type variable, so it renders through `poly_type_str`
/// rather than `Type`'s `Display`.
pub(super) fn poly_constructed_reference_error(
    ctx: &Ctx,
    span: Span,
    position: &str,
    ty: &PolyType,
    sig: &PolySig,
) -> String {
    format!(
        "error: a reference cannot be stored{} (line {})\n  {position} has type `{}`\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow",
        in_word(ctx),
        span.line,
        poly_type_str(ty, sig)
    )
}

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
pub(in crate::check) fn apply_subst(
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
        PolyType::Var(v) => match subst.ty_of(*v) {
            // S1-15.g: a bare `CtorImage` reaching a value-type position
            // outside `App`-head resolution -- both spans where available:
            // the binding site (`ty_var_spans`, the variable's first
            // mention) and the misuse site (`span`, this grounding call).
            // An internally-created signature with no span table degrades
            // to the misuse span alone. The constructor's name is the one
            // carried on the `CtorImage` itself (`ctor_image_type`'s own
            // `GenericTypes` read at binding time), so no second lookup here.
            Some(Type::CtorImage(_, ctor_name)) => {
                let binding_span = sig.ty_var_spans.get(*v as usize).copied().unwrap_or(span);
                Err(poly_ctor_image_as_type_error(
                    ctx,
                    span,
                    name,
                    &sig.ty_var_names[*v as usize],
                    binding_span,
                    ctor_name,
                ))
            }
            Some(t) => Ok(t),
            None => Err(poly_unbound_output_ty_error(
                ctx,
                span,
                name,
                &sig.ty_var_names[*v as usize],
                sig.ty_var_names.len(),
            )),
        },
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
        // P7.S6a (R8a): `len_args` resolves through `subst.len` the same way
        // `Array`'s own `Len::Var` arm does, before minting.
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args,
            len_args,
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
            let mut concrete_lens = Vec::with_capacity(len_args.len());
            for len in len_args {
                let k = match len {
                    Len::Concrete(k) => *k,
                    Len::Var(ln) => subst.len_of(*ln).ok_or_else(|| {
                        poly_unbound_output_error(ctx, span, name, &sig.len_var_names[*ln as usize])
                    })?,
                };
                concrete_lens.push(Len::Concrete(k));
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
                g.instantiate_enum(*idx as usize, &concrete_args, &concrete_lens, *module, regs)
            } else {
                g.instantiate_struct(*idx as usize, &concrete_args, &concrete_lens, *module, regs)
            })
        }
        // P7.S12 (R3.1/R4.1): ground the header the same way the `Generic`
        // arm does, then read the narrowed variant's `Type::Variant` off the
        // resulting monomorph -- the same `(idx, module, args)` id space, S3a
        // D3. The mint may not be flushed into `ctx.enums()` yet (it can be
        // this very call), so a body-local decl is read through
        // `GenericTypes::enum_decl` first -- guarded on `id.index() >=
        // ctx.enums().len()`, exactly `poly_eliminator_call`'s own guard
        // (R1.1 there): a hand-built `GenericTypes` with `enum_base == 0`
        // and a non-empty `enums` makes `enum_decl(small_id)` compute
        // `id.index() - 0`, which can land inside `inst_enums` and return
        // the *wrong* decl for an id that in fact names an already-flushed
        // entry, rather than falling through to `None`.
        // P7.S6a (R8a): resolves `len_args` through `subst.len` identically
        // to the `Generic` arm above, replacing phase 3's placeholder.
        PolyType::GenericVariant {
            idx,
            module,
            vi,
            args,
            len_args,
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
            let mut concrete_lens = Vec::with_capacity(len_args.len());
            for len in len_args {
                let k = match len {
                    Len::Concrete(k) => *k,
                    Len::Var(ln) => subst.len_of(*ln).ok_or_else(|| {
                        poly_unbound_output_error(ctx, span, name, &sig.len_var_names[*ln as usize])
                    })?,
                };
                concrete_lens.push(Len::Concrete(k));
            }
            let regs = crate::ast::MutRegistries {
                structs: ctx.structs(),
                enums: ctx.enums(),
                arrays,
                cells,
                refs,
            };
            let mut g = cell.borrow_mut();
            let Type::Enum(id, _) =
                g.instantiate_enum(*idx as usize, &concrete_args, &concrete_lens, *module, regs)
            else {
                unreachable!("instantiate_enum always returns Type::Enum")
            };
            let display = if id.index() >= ctx.enums().len() {
                g.enum_decl(id)
                    .map(|d| d.variants[*vi].display_static)
                    .expect(
                        "id past `ctx.enums().len()` names a mint this call's own `g` just made",
                    )
            } else {
                ctx.enums()[id.index()].variants[*vi].display_static
            };
            Ok(Type::Variant(id, *vi, display))
        }
        // S1-11: ground an `App` by resolving `head`'s binding (a
        // `Type::CtorImage`, minted by `unify_poly_input`'s own `App` arm),
        // substituting the application's arguments through the
        // constructor's declared parameters, and delegating to the same
        // instantiator the `Generic` arm above mints through.
        //
        // P7b.S7 (REQ-3): the two `sig.ty_var_names[*head as usize]`
        // indexings below are safe by construction, never bounds-checked --
        // an `App`'s `head` id is always allocated within the same
        // `PolySig`'s own variable space it is matched against here
        // (`apply_subst` is always called with the enclosing word's own
        // `sig`), so `head` can never point outside `ty_var_names`.
        PolyType::App { head, args } => {
            let ctor = subst.ty_of(*head).ok_or_else(|| {
                poly_unbound_output_ty_error(
                    ctx,
                    span,
                    name,
                    &sig.ty_var_names[*head as usize],
                    sig.ty_var_names.len(),
                )
            })?;
            let Type::CtorImage(gid, ctor_name) = ctor else {
                return Err(poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                    &ctor.to_string(),
                ));
            };
            let mut concrete_args = Vec::with_capacity(args.len());
            for a in args {
                concrete_args.push(apply_subst(
                    sig, a, subst, name, span, ctx, arrays, cells, refs,
                )?);
            }
            let Some(cell) = ctx.generics() else {
                return Err(poly_generic_not_yet_groundable_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(pty, sig),
                ));
            };
            // S1-7 defense-in-depth: an explicit call-site instantiation
            // (`pass[Buf i64]`) can seed `head` directly with a
            // length-parameterized constructor's `CtorImage`, bypassing
            // `unify_poly_input`'s own App-arm check entirely -- there is no
            // operand to unify an App-typed *output* against. Guard here
            // too, against the header's own declared length arity, so no
            // path reaches `instantiate_struct`/`instantiate_enum` below
            // with a mismatched (empty) length list.
            {
                let generics = cell.borrow();
                let len_arity = if gid.is_enum {
                    generics.enums[gid.idx as usize].len_var_names.len()
                } else {
                    generics.structs[gid.idx as usize].len_var_names.len()
                };
                if len_arity != 0 {
                    let var = &sig.ty_var_names[*head as usize];
                    return Err(poly_app_len_domain_unsupported_error(
                        ctx, span, name, var, ctor_name,
                    ));
                }
            }
            let regs = crate::ast::MutRegistries {
                structs: ctx.structs(),
                enums: ctx.enums(),
                arrays,
                cells,
                refs,
            };
            let mut g = cell.borrow_mut();
            Ok(if gid.is_enum {
                g.instantiate_enum(gid.idx as usize, &concrete_args, &[], gid.module, regs)
            } else {
                g.instantiate_struct(gid.idx as usize, &concrete_args, &[], gid.module, regs)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        ArrayDecl, ArrayId, GenericEnumDecl, GenericStructDecl, GenericVariantDecl, OwnedCellDecl,
    };
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
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        }
    }

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

    /// A signature over two variables, `'F` (the application head, id 0)
    /// and `'T` (its argument, id 1) -- the fixture the S1-10/S1-11 `App`
    /// unit tests share.
    fn app_sig() -> PolySig {
        PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'F".to_string(), "'T".to_string()],
            ty_var_spans: vec![Span::default(), Span::default()],
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        }
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
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        }
    }

    fn poly_ref(referent: PolyType, mutable: bool) -> PolyType {
        PolyType::Ref(Box::new(referent), mutable)
    }

    /// The row-3 shape every S3-1.c pin below cuts from: an `inline` member
    /// on a generic impl target, reached through a bound on an `inline`
    /// caller.
    fn sized_box_splice_src() -> &'static str {
        "trait: Sized['S] :\n\
         size inline ( 'S -- i64 ) ;\n\
         ;\n\
         type: Box['T] v 'T ;\n\
         impl: Sized for Box['T]\n\
         : size drop 1 ;\n\
         ;\n\
         : usesize inline['S: Sized] ( 'S -- i64 ) size ;\n\
         : mkbox ( i64 -- Box[i64] ) Box ;\n\
         : main ( -- ) 3 mkbox usesize drop ;\n"
    }

    /// A single-field `Box['T]`-shaped generic header, for the S1-10/S1-11
    /// `App` unit tests below -- the plain (length-free) twin of
    /// `buffer_header`.
    fn box_header() -> GenericStructDecl {
        GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        }
    }

    /// S1-10: `unify_poly_input`'s `App` arm decomposes an application
    /// against a concrete slot -- matching `'F['T]` against `Box[i64]`
    /// binds `'F := Type::CtorImage(box_generic_id)` (in the existing `ty`
    /// map) and `'T := i64` positionally.
    #[test]
    fn unify_poly_input_app_decomposition_binds_ctor_and_arg() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(box_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let slot_ty = generics.instantiate_struct(
            0,
            &[Type::I64],
            &[],
            0,
            crate::ast::MutRegistries {
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                cells: &mut cells,
                refs: &mut refs,
            },
        );
        let cell = RefCell::new(generics);
        let sig = app_sig();
        let probe = probe_word();
        let ctx = word_ctx(
            &probe,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        let declared = PolyType::App {
            head: 0,
            args: vec![PolyType::Var(1)],
        };
        unify_poly_input(
            &sig,
            &declared,
            slot_ty,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
            &[],
        )
        .expect("`'F['T]` should unify against `Box[i64]`");
        assert_eq!(
            subst.ty_of(0),
            Some(Type::CtorImage(
                crate::ast::GenericId {
                    is_enum: false,
                    idx: 0,
                    module: 0,
                },
                "Box",
            )),
            "'F should bind to Box's CtorImage"
        );
        assert_eq!(subst.ty_of(1), Some(Type::I64), "'T should bind to i64");
    }

    /// S1-11: `apply_subst`'s `App` arm grounds `'F['T]` to a concrete
    /// `Type` via the `Generic` mint route -- resolving `'F`'s
    /// `Type::CtorImage` binding, substituting `'T`, and minting (or
    /// finding) `Box[i64]`.
    #[test]
    fn apply_subst_app_grounds_via_the_generic_mint_route() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(box_header());
        let cell = RefCell::new(generics);
        let sig = app_sig();
        let probe = probe_word();
        let ctx = word_ctx(
            &probe,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        subst.ty.push((
            0,
            Type::CtorImage(
                crate::ast::GenericId {
                    is_enum: false,
                    idx: 0,
                    module: 0,
                },
                "Box",
            ),
        ));
        subst.ty.push((1, Type::I64));
        let declared = PolyType::App {
            head: 0,
            args: vec![PolyType::Var(1)],
        };
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let ty = apply_subst(
            &sig,
            &declared,
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect("`'F['T]` should ground to `Box[i64]`");
        assert_eq!(ty.name(), "Box[i64]");
    }

    /// P7b.S1 review fix (P0): `unify_poly_input`'s `App` arm used to
    /// discard the recovered instantiation's length arguments entirely --
    /// binding `'F := Type::CtorImage(buffer_gid)` for a *length*-
    /// parameterized header exactly as it would for a plain one. S1-7 fences
    /// `App` to type arguments only, so this must be a located error, not a
    /// silent bind that panics three calls later in `apply_subst`.
    #[test]
    fn unify_poly_input_app_rejects_a_length_parameterized_constructor() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let slot_ty = generics.instantiate_struct(
            0,
            &[Type::I64],
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
        let cell = RefCell::new(generics);
        let sig = app_sig();
        let probe = probe_word();
        let ctx = word_ctx(
            &probe,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        let declared = PolyType::App {
            head: 0,
            args: vec![PolyType::Var(1)],
        };
        let err = unify_poly_input(
            &sig,
            &declared,
            slot_ty,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
            &[],
        )
        .expect_err("'F['T]' must not bind against a length-parameterized constructor");
        assert!(err.contains("declares a length parameter"), "{err}");
        assert_eq!(
            subst.ty_of(0),
            None,
            "a rejected application must not bind 'F"
        );
    }

    /// P7b.S1 review fix (P0), `apply_subst`'s half: defense-in-depth for a
    /// binding that reached `'F := Type::CtorImage(buffer_gid)` by some
    /// route other than `unify_poly_input`'s own (now-guarded) `App` arm --
    /// grounding must still refuse to mint through `instantiate_struct`
    /// with an empty length list rather than let it index out of bounds.
    #[test]
    fn apply_subst_app_rejects_a_length_parameterized_ctor_binding() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let cell = RefCell::new(generics);
        let sig = app_sig();
        let probe = probe_word();
        let ctx = word_ctx(
            &probe,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        subst.ty.push((
            0,
            Type::CtorImage(
                crate::ast::GenericId {
                    is_enum: false,
                    idx: 0,
                    module: 0,
                },
                "Buffer",
            ),
        ));
        subst.ty.push((1, Type::I64));
        let declared = PolyType::App {
            head: 0,
            args: vec![PolyType::Var(1)],
        };
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let err = apply_subst(
            &sig,
            &declared,
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect_err("grounding must reject a length-parameterized ctor binding, not panic");
        assert!(err.contains("declares a length parameter"), "{err}");
    }

    /// S1-15.g: a bare `Type::CtorImage` reaching a value-type position
    /// outside `App`-head resolution is a located error naming both spans
    /// -- the binding site (`'F`'s first mention) and the misuse site (this
    /// grounding call). Parser-side kind consistency (S1-4) means no
    /// *single* signature can ever declare both a bare and an applied use
    /// of the same variable, so this exercises `apply_subst`'s own arm
    /// directly: a hand-built `Subst` binds `'F` to a `CtorImage` exactly
    /// as `unify_poly_input`'s `App` arm would, and a declared bare `'F`
    /// output reads it back.
    #[test]
    fn apply_subst_ctor_image_reaching_bare_var_is_a_located_error() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(box_header());
        let cell = RefCell::new(generics);
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'F".to_string()],
            ty_var_spans: vec![Span {
                line: 3,
                col: 5,
                ..Span::default()
            }],
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let probe = probe_word();
        let ctx = word_ctx(
            &probe,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        subst.ty.push((
            0,
            Type::CtorImage(
                crate::ast::GenericId {
                    is_enum: false,
                    idx: 0,
                    module: 0,
                },
                "Box",
            ),
        ));
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let err = apply_subst(
            &sig,
            &PolyType::Var(0),
            &subst,
            "f",
            Span {
                line: 9,
                col: 1,
                ..Span::default()
            },
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect_err("a bare CtorImage in a value-type position must be rejected");
        assert!(err.contains("Box"), "{err}");
        assert!(err.contains("line 9"), "{err}");
        assert!(err.contains("line 3"), "{err}");
    }

    /// P7.S6a (R8a): `unify_poly_input`'s `Generic` arm binds a declared
    /// length variable from a concrete instantiation's own recovered
    /// length, the same way the neighboring `Array` arm binds a bare
    /// array's `Len::Var` -- without this, `Buffer['T 'N]` in a signature
    /// cannot bind `'N` from a concrete `Buffer[u8 256]` operand.
    #[test]
    fn unify_poly_input_generic_binds_len_arg_from_concrete_instantiation() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let slot_ty = generics.instantiate_struct(
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
        let cell = RefCell::new(generics);
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let pty = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        let word = probe_word();
        let ctx = word_ctx(
            &word,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        unify_poly_input(
            &sig,
            &pty,
            slot_ty,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
            &[],
        )
        .expect("a concrete Buffer[u8 256] slot must bind 'N");
        assert_eq!(subst.ty_of(0), Some(Type::U32));
        assert_eq!(
            subst.len_of(0),
            Some(256),
            "'N must bind to the instantiation's own recovered length"
        );
    }

    /// The conflict twin: two positions both binding `'N` against different
    /// concrete lengths must be a located mismatch, never a silent last-
    /// write-wins overwrite -- exactly `poly_len_conflict_error`'s existing
    /// role for the `Array` arm's own `Len::Var`.
    #[test]
    fn unify_poly_input_generic_len_conflict_is_error() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let slot_ty = generics.instantiate_struct(
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
        let cell = RefCell::new(generics);
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let pty = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        let word = probe_word();
        let ctx = word_ctx(
            &word,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        subst.len.push((0, 512));
        let err = unify_poly_input(
            &sig,
            &pty,
            slot_ty,
            "f",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
            &[],
        )
        .unwrap_err();
        assert!(
            err.contains("'N"),
            "a length conflict must name the conflicting length variable: {err}"
        );
    }

    /// P7.S6b (R4/R5): a length mismatch seeded from an explicit call-site
    /// length argument (`sum[i64 4]`) routes to the caller-context message,
    /// not the generic "conflicting bindings" one -- the `Array` arm's own
    /// `Len::Var` twin of `unify_poly_input_finding_a_seeded_variable_names_the_instantiation`
    /// above.
    #[test]
    fn unify_poly_input_array_seeded_len_conflict_names_the_instantiation() {
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: Vec::new(),
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let pty = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 8,
            name_static: "array",
        }];
        let cells: [OwnedCellDecl; 0] = [];
        let refs: [RefDecl; 0] = [];
        let slot_ty = Type::Array(ArrayId::from_index(0), "array");
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let mut subst = Subst::default();
        subst.len.push((0, 4));
        let err = unify_poly_input(
            &sig,
            &pty,
            slot_ty,
            "sum",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
            &[0],
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `sum` in `probe` (line 0) was instantiated at length `'N` = `4` but its operand is `8`"
        );
    }

    /// The routing negative (spec test notes): the *same* mismatch, with
    /// `'N` bound by ordinary inference rather than an explicit call-site
    /// argument (an empty `seeded_len`), must keep reporting
    /// `poly_len_conflict_error` unchanged -- guards against a placebo
    /// routing that always fires the explicit message regardless of
    /// seeding.
    #[test]
    fn unify_poly_input_array_inferred_len_conflict_is_unrouted() {
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: Vec::new(),
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let pty = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let arrays = vec![ArrayDecl {
            element: Type::I64,
            count: 8,
            name_static: "array",
        }];
        let cells: [OwnedCellDecl; 0] = [];
        let refs: [RefDecl; 0] = [];
        let slot_ty = Type::Array(ArrayId::from_index(0), "array");
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let mut subst = Subst::default();
        subst.len.push((0, 4));
        let err = unify_poly_input(
            &sig,
            &pty,
            slot_ty,
            "sum",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[],
            &[],
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `sum` in `probe` (line 0) resolved length `'N` to both `4` and `8`"
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
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            len_var_spans: Vec::new(),
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
                &[],
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
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            len_var_spans: Vec::new(),
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

    /// P7.S6a (R8a): `apply_subst`'s `Generic` arm resolves a declared
    /// length variable through `subst.len` before minting, so a length-
    /// carrying monomorph grounds to the concrete instantiation that
    /// length actually names -- replacing phase 3's placeholder empty
    /// length list.
    #[test]
    fn apply_subst_grounds_generic_len_args_through_subst() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(buffer_header());
        let cell = RefCell::new(generics);
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let pty = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "Buffer",
        };
        let word = probe_word();
        let ctx = word_ctx(
            &word,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        subst.ty.push((0, Type::U32));
        subst.len.push((0, 256));
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let ty = apply_subst(
            &sig,
            &pty,
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect("a bound length variable grounds the monomorph");
        let Type::Struct(id, _) = ty else {
            panic!("a grounded Generic header is a Type::Struct: {ty:?}")
        };
        let borrowed = cell.borrow();
        let (_, _, _, lens) = borrowed
            .struct_instantiation_of(id)
            .expect("the mint is recoverable through struct_instantiation_of");
        assert_eq!(
            lens,
            &[Len::Concrete(256)],
            "the minted monomorph must carry the length subst.len bound, not an empty placeholder"
        );
    }

    /// P7.S12 phase 2 (R3.3/R4.1): `apply_subst`'s `GenericVariant` arm is
    /// the only arm this phase adds that computes rather than rejects, and
    /// nothing constructs a `GenericVariant` until phase 3 -- so the witness
    /// hand-builds one. It grounds the variant's own `args` through the same
    /// instantiator the `Generic` arm uses, then reads the narrowed
    /// variant's display off the resulting monomorph: through
    /// `GenericTypes::enum_decl` while that mint is still unflushed, and off
    /// `ctx.enums()` once it has been flushed and the base rebased. Both
    /// halves must answer identically, which is what makes the id-range
    /// guard between them invisible to a correctly-based registry.
    #[test]
    fn apply_subst_grounds_a_generic_variant_across_the_flush() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.enums.push(GenericEnumDecl {
            name: "Opt".to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: Vec::new(),
            len_var_names: vec![],
            variants: vec![
                GenericVariantDecl {
                    name: "None".to_string(),
                    fields: Vec::new(),
                    span: Span::default(),
                },
                GenericVariantDecl {
                    name: "Some".to_string(),
                    fields: vec![("0".to_string(), PolyType::Var(0))],
                    span: Span::default(),
                },
            ],
            span: Span::default(),
            module: 0,
        });
        let cell = RefCell::new(generics);
        let pty = crate::ast::generic_variant_type(
            &cell.borrow(),
            0,
            0,
            1,
            vec![PolyType::Var(0)],
            vec![],
        );
        let sig = ref_sig();
        let mut subst = Subst::default();
        subst.ty.push((0, Type::I64));
        let effect = StackEffect {
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let mut enums: Vec<EnumDecl> = Vec::new();
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();

        let unflushed = {
            let ctx = Ctx {
                mangled: "f",
                effect: &effect,
                structs: &[],
                enums: &enums,
                statics: &[],
                module: 0,
                modules: None,
                self_tail_call: false,
                generics: Some(&cell),
            };
            apply_subst(
                &sig,
                &pty,
                &subst,
                "f",
                Span::default(),
                &ctx,
                &mut arrays,
                &mut cells,
                &mut refs,
            )
            .expect("a bound argument grounds the narrowed variant")
        };
        let Type::Variant(id, vi, display) = unflushed else {
            panic!("a narrowed generic variant grounds to a `Type::Variant`: {unflushed:?}")
        };
        assert_eq!(vi, 1, "the arm's own variant index rides through");
        assert_eq!(
            display, "Opt[i64].Some",
            "the display names the monomorph, not the header"
        );
        assert!(
            id.index() >= enums.len(),
            "the mint is still unflushed, so the decl is only readable through `enum_decl`"
        );

        cell.borrow_mut().flush_enums_into(&mut enums);
        cell.borrow_mut().rebase(0, enums.len());
        assert_eq!(enums.len(), 1);
        assert!(
            id.index() < enums.len(),
            "the flush moved the mint into `ctx.enums()`"
        );

        let ctx = Ctx {
            mangled: "f",
            effect: &effect,
            structs: &[],
            enums: &enums,
            statics: &[],
            module: 0,
            modules: None,
            self_tail_call: false,
            generics: Some(&cell),
        };
        let flushed = apply_subst(
            &sig,
            &pty,
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect("the memoized instantiation grounds the same way after the flush");
        assert_eq!(flushed, unflushed);
    }

    /// P7.S6a (R8a, review round 2): `apply_subst`'s `GenericVariant` arm
    /// resolves its own `len_args` through `subst.len` identically to the
    /// `Generic` arm's fix -- `apply_subst_grounds_a_generic_variant_across_
    /// the_flush` above hand-builds `Opt['T]`, a header with no length
    /// variable at all, so it cannot witness this half of the arm. This
    /// test narrows a length-carrying `Buffer['T 'N]` variant and asserts
    /// the minted monomorph carries the bound length, not phase 3's empty
    /// placeholder.
    #[test]
    fn apply_subst_grounds_a_generic_variant_len_args_through_subst() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.enums.push(GenericEnumDecl {
            name: "Buffer".to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            variants: vec![GenericVariantDecl {
                name: "Full".to_string(),
                fields: vec![("0".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        });
        let cell = RefCell::new(generics);
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec!["'N".to_string()],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let pty = crate::ast::generic_variant_type(
            &cell.borrow(),
            0,
            0,
            0,
            vec![PolyType::Var(0)],
            vec![Len::Var(0)],
        );
        let word = probe_word();
        let ctx = word_ctx(
            &word,
            &[],
            &[],
            &[],
            None,
            &CombinatorIndex::new(),
            Some(&cell),
        );
        let mut subst = Subst::default();
        subst.ty.push((0, Type::U32));
        subst.len.push((0, 256));
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let ty = apply_subst(
            &sig,
            &pty,
            &subst,
            "f",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
        )
        .expect("a bound length variable grounds the narrowed variant's monomorph");
        let Type::Variant(id, ..) = ty else {
            panic!("a grounded GenericVariant is a Type::Variant: {ty:?}")
        };
        let borrowed = cell.borrow();
        let (_, _, _, lens) = borrowed
            .enum_instantiation_of(id)
            .expect("the mint is recoverable through enum_instantiation_of");
        assert_eq!(
            lens,
            &[Len::Concrete(256)],
            "the minted monomorph must carry the length subst.len bound, not an empty placeholder"
        );
    }

    /// P7b.S3 (S3-1.c, θ agreement): `find_bound_impl`'s impl `Subst` seeds
    /// the splice's operand-derived θ. Agreement half: the goldens' own shape
    /// checks clean end to end (the seed and the operands bind the target's
    /// variables identically). Contradiction half: unreachable from source --
    /// the impl was selected *because* the operands fit -- so the mechanism
    /// the seed rides on is driven directly: a seeded binding an operand
    /// contradicts is a located error naming the instantiation, and the
    /// seeded binding is not overwritten.
    #[test]
    fn impl_seed_and_operand_theta_agree_and_a_contradiction_is_a_located_error() {
        check_src(sized_box_splice_src()).expect("the seeded splice agrees with its operands");
        let sig = PolySig {
            row_in: None,
            inputs: vec![PolyType::Var(0)],
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: Vec::new(),
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let arrays: [ArrayDecl; 0] = [];
        let cells: [OwnedCellDecl; 0] = [];
        let refs: [RefDecl; 0] = [];
        let mut subst = Subst::default();
        subst.ty.push((0, Type::F64)); // the impl-derived seed
        let err = unify_poly_input(
            &sig,
            &sig.inputs[0],
            Type::I64, // the operand disagrees
            "size",
            Span::default(),
            &ctx,
            &arrays,
            &cells,
            &refs,
            &mut subst,
            &[0],
            &[],
        )
        .expect_err("a seeded binding the operand contradicts is an error");
        assert_eq!(
            err,
            "error: `size` in `probe` (line 0) was instantiated at `'T` = `f64` but its operand is `i64`"
        );
        assert_eq!(
            subst.ty,
            vec![(0, Type::F64)],
            "the seeded binding is never silently overwritten"
        );
    }

    /// P7b.S8c (P1-1): drives `compose_member_theta` directly -- the private
    /// helper the generic-mint arm extracted its body into, so this pins the
    /// tail's *wiring* (per-slot unification into a conflict error, and the
    /// fail-closed check after it) rather than only the two pure halves
    /// (`unify_poly_input`, `first_unbound_sig_var`) each already has their
    /// own unit test. Nothing that ships would fail if the arm called
    /// `unify_poly_input` in a loop and skipped `first_unbound_sig_var`
    /// entirely -- this is the test that would.
    ///
    /// The conflict-provenance ruling also folds in here (the review's
    /// duplicate pin, P2-2): the mint arm unifies with `seeded`/`seeded_len`
    /// **empty**, since those lists are wording selectors, not the channel
    /// prior bindings ride (those ride theta, cloned from the impl-target
    /// match). An empty list never contains the conflicting variable, so a
    /// disagreement against a theta-bound variable renders the *unseeded*
    /// symmetric wording -- the same unseeded wording the precedent
    /// `unify_poly_input_finding_a_seeded_variable_names_the_instantiation`
    /// already pins at the `unify_poly_input` level, here shown surviving
    /// unchanged through the helper the mint arm actually calls.
    #[test]
    fn compose_member_theta_propagates_the_match_subst_conflict_unseeded() {
        let word_sig = PolySig {
            inputs: vec![PolyType::Var(0)],
            ty_var_names: vec!["'T0".to_string()],
            ..bare_sig()
        };
        let ob = TraitObligation {
            span: Span::default(),
            var: 0,
            trait_id: TraitId(0),
            member: "odd".to_string(),
            slots: vec![PolyType::Concrete(Type::I64)],
        };
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let caller_sig = bare_sig();
        let caller_subst = Subst::default();
        let (mut arrays, mut cells, mut refs) = (Vec::new(), Vec::new(), Vec::new());
        // `base` as the mint arm builds it: cloned from the impl-target
        // match, which already bound the target variable to `str`.
        let mut base = Subst::default();
        base.ty.push((0, Type::Str));
        let err = compose_member_theta(
            &word_sig,
            "odd;Odd;0;Box['T0]__m0",
            "Odd",
            &ob,
            &caller_sig,
            &caller_subst,
            "probe",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
            base,
        )
        .expect_err("the site slot disagrees with what the impl-target match bound");
        assert_eq!(
            err,
            "error: `odd;Odd;0;Box['T0]` in `probe` (line 0) resolved `'T0` to both `str` and `i64`"
        );
        assert!(
            !err.contains("was instantiated at"),
            "the impl-target match is not a written instantiation: {err}"
        );
    }

    /// P7b.S8c (P1-1), the tail-wiring's other end: a signature variable that
    /// appears only in the member's *output* (`empty ( -- 'T )`'s `'T`, the
    /// nullary-member shape) and that neither the impl-target match (`base`)
    /// nor this site's own input slots bind. `compose_member_theta` has to
    /// reach `first_unbound_sig_var` and turn its `Some` into the located
    /// error -- the fence REQ-3 relies on to keep `driver.rs:579` untouched.
    #[test]
    fn compose_member_theta_fails_closed_on_a_residual_unbound_variable() {
        let word_sig = PolySig {
            inputs: vec![PolyType::Var(0)],
            outputs: vec![PolyType::Var(1)],
            ty_var_names: vec!["'T".to_string(), "'U".to_string()],
            ..bare_sig()
        };
        let ob = TraitObligation {
            span: Span {
                line: 5,
                col: 46,
                ..Span::default()
            },
            var: 0,
            trait_id: TraitId(0),
            member: "odd".to_string(),
            slots: vec![PolyType::Concrete(Type::I64)],
        };
        let probe = probe_word();
        let ctx = probe_ctx(&probe);
        let caller_sig = bare_sig();
        let caller_subst = Subst::default();
        let (mut arrays, mut cells, mut refs) = (Vec::new(), Vec::new(), Vec::new());
        let err = compose_member_theta(
            &word_sig,
            "odd;Odd;0;Box['T0]__m0",
            "Odd",
            &ob,
            &caller_sig,
            &caller_subst,
            "probe",
            Span::default(),
            &ctx,
            &mut arrays,
            &mut cells,
            &mut refs,
            Subst::default(),
        )
        .expect_err("'U appears only in the output, and neither side binds it");
        assert_eq!(
            err,
            "error: `odd` of `Odd` in `probe` (line 5, col 46) leaves type variable `'U` unbound\n  the impl target's match and this site's operands together determine no type for `'U`, so the member has no instantiation here -- give it an operand position that fixes `'U`"
        );
    }

    /// P7b.S8c (REQ-1), the fail-closed tail's decision function. It walks
    /// the member sig's `inputs` *and* `outputs` -- `concrete_effect`
    /// substitutes both through the same closure -- and enumerates
    /// `PolyType`'s variants rather than the slot lists alone, since a
    /// `Len::Var` hides inside an `Array`/`Generic` and an `App`'s head is a
    /// variable in its own right.
    #[test]
    fn first_unbound_sig_var_reaches_outputs_and_nested_positions() {
        let sig = |inputs: Vec<PolyType>, outputs: Vec<PolyType>| PolySig {
            inputs,
            outputs,
            ty_var_names: vec!["'T".to_string(), "'U".to_string()],
            len_var_names: vec!["'N".to_string()],
            ..bare_sig()
        };
        let mut theta = Subst::default();
        theta.ty.push((0, Type::I64));
        // Bound input, unbound output: the widened quantifier (an output-only
        // variable is what `driver.rs:579` fires on for a nullary member).
        assert_eq!(
            first_unbound_sig_var(&sig(vec![PolyType::Var(0)], vec![PolyType::Var(1)]), &theta),
            Some("'U".to_string())
        );
        // Behind a reference, inside a quotation row.
        assert_eq!(
            first_unbound_sig_var(
                &sig(
                    vec![PolyType::Ref(
                        Box::new(PolyType::Quotation(
                            vec![PolyType::Var(1)],
                            Vec::new(),
                            false,
                            None,
                            None,
                        )),
                        false,
                    )],
                    Vec::new(),
                ),
                &theta
            ),
            Some("'U".to_string())
        );
        // A length variable inside an array element shape.
        assert_eq!(
            first_unbound_sig_var(
                &sig(
                    vec![PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0))],
                    Vec::new()
                ),
                &theta
            ),
            Some("'N".to_string())
        );
        // An application's head is itself a variable.
        assert_eq!(
            first_unbound_sig_var(
                &sig(
                    vec![PolyType::App {
                        head: 1,
                        args: vec![PolyType::Var(0)],
                    }],
                    Vec::new()
                ),
                &theta
            ),
            Some("'U".to_string())
        );
        // Fully determined: nothing to report, and the arm mints.
        assert_eq!(
            first_unbound_sig_var(
                &sig(vec![PolyType::Var(0)], vec![PolyType::Concrete(Type::F64)]),
                &theta
            ),
            None
        );
    }
}
