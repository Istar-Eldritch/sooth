use super::*;
/// P7.S3k (R1-R3/R6): a call from one polymorphic body to a *different*
/// polymorphic word. Neither side has a θ here, so nothing is unified against
/// a concrete type: the callee's declared inputs are matched **structurally**
/// against the caller's operand slots, and what comes out is a
/// variable-to-variable mapping (R2) recorded for later composition, never a
/// `Subst`. The self-call arm above is not an instance of this -- it reuses
/// the walk's own `sig` and needs no mapping at all.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_cross_call(
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
pub(super) fn poly_cross_match(
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
                // P7.S12 (R3.4/R6.4): a narrowed generic variant reaching a
                // cross-call is field projection's own out-of-scope
                // territory one door over -- the callee's `'T` has no way to
                // name a payload it would have to be projected out of, and
                // `poly_type_mentions_caller_var` below has no defined
                // answer for a shape that carries no `Type`. Located rather
                // than falling into that call unguarded.
                PolyType::GenericVariant { .. } => {
                    return Err(poly_cross_call_unsupported_error(
                        ctx,
                        span,
                        callee,
                        "passing a generic variant to a cross-called polymorphic word",
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
        // P7.S6a (R8, round-4 fix): `len_args` must match exactly, mirroring
        // the `Array` arm's own `dl == sl` guard one level up -- a header
        // carrying a *concrete* length (spellable since R7) is otherwise
        // invisible here, so a cross-call from a body declared over one
        // concrete length to a callee declared over a different concrete
        // length passed silently, either lowering the wrong monomorph or
        // tripping `subst_polytype`'s `.expect` in `src/ir/driver.rs`.
        (
            PolyType::Generic {
                is_enum: de,
                idx: di,
                module: dm,
                args: da,
                len_args: dl_args,
                ..
            },
            PolyType::Generic {
                is_enum: se,
                idx: si,
                module: sm,
                args: sa,
                len_args: sl_args,
                ..
            },
        ) if (de, di, dm, da.len()) == (se, si, sm, sa.len()) && dl_args == sl_args => {
            for (d, sup) in da.iter().zip(sa) {
                poly_cross_match(d, sup, mapping, callee_sig, caller_sig, callee, span, ctx)?;
            }
            Ok(())
        }
        // S1-17.i: a poly *cross-call* with an `App` slot stays a located
        // "unsupported" rejection -- S2 owns constructor-keyed dispatch.
        (PolyType::App { .. }, _) | (_, PolyType::App { .. }) => {
            Err(poly_cross_call_unsupported_error(
                ctx,
                span,
                callee,
                "a higher-kinded application in a cross-called polymorphic word",
            ))
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
        // P7.S12 phase 2 (R3.4): a `GenericVariant` reaches this arm too
        // (declared output R3.5 never spells one, but a body-mint could in
        // principle be cross-called against), and it is already rejected
        // explicitly, not passed through silently -- the same "compound type
        // unsupported" diagnostic every other multi-field/`Ref` output gets.
        // No conversion needed: the wildcard here is a deliberate catch-all
        // error path, not a silent default.
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
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this predicate walks.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
        // P7b.S1 (S1-16): an application always mentions the applied
        // variable itself (its `head`), regardless of its arguments.
        PolyType::App { .. } => true,
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
