use super::*;
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
/// returned): the instantiation is then minted under the header's declaring
/// module, and every argument must come from the operands alone.
fn poly_construction_fallback(
    sig: &PolySig,
    is_enum: bool,
    idx: usize,
) -> Option<(u32, &[PolyType], &'static str)> {
    // P7.S12 phase 2 (R3.4): this scrutinee is a declared *output* shape
    // (R3.5: a `GenericVariant` never spells one), so the `_ =>` here only
    // ever discards `Var`/`Concrete`/`Ref`/other `Generic` headers -- safe
    // as written, no conversion needed.
    sig.outputs.iter().find_map(|pty| match pty {
        PolyType::Generic {
            is_enum: oe,
            idx: oidx,
            module,
            args,
            name,
            ..
        } if *oe == is_enum && *oidx as usize == idx => Some((*module, args.as_slice(), *name)),
        _ => None,
    })
}

/// P7b.S8b (Phase 2 review, P1): a `Generic` construction field naming a
/// non-empty `len_args` -- e.g. a self-referential `Ring['T 'N: Len]`
/// field -- has no slot to bind its length variable into;
/// `poly_construct_generic` infers no lengths at all. Deliberately not
/// `poly_rendered_type_mismatch_error`: this fence runs BEFORE the operand is
/// even destructured or identity-checked, and `poly_type_str` renders name +
/// args but not the module id -- so both a same-header operand and a
/// module-only identity mismatch would print identical text on both sides
/// (`` `Ring` expected `Ring['T 'N]`, found `Ring['T 'N]` ``) --
/// indistinguishable from a real bug in the renderer. This names the header
/// and says what is actually unbindable instead.
fn poly_generic_field_len_unbound_error(ctx: &Ctx, span: Span, op: &str, header: &str) -> String {
    let op = crate::resolve::demangle_call(op);
    let where_ = ctx.rendered_word();
    format!(
        "error: `{op}` in {where_} (line {}) cannot bind `{header}`'s length variable\n  the field names `{header}` with a length parameter, but constructing a value here infers no lengths; only the field's element type variables can be bound this way",
        span.line
    )
}

/// P7 slice 3a (R3): bind one constructor payload field's declared `PolyType`
/// against the operand `PolyType` on the stack, recording the header
/// variable it determines. Five field shapes reach this match today:
/// `Var`, `Concrete`, `App`, `OwnedCell` (P7b.S6), and `Generic`
/// (P7b.S8b Phase 2, the generic field arm) -- the catch-all still
/// rejects everything else (an array-shaped field, per `substitute_generic_field`).
pub(super) fn poly_bind_construction_arg(
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
        // S1-10: a construction field is an application (`f 'F['T]`) --
        // bind the header variable it applies (`head`) to a `Type::CtorImage`
        // naming the operand's own header, then unify its arguments against
        // the operand's own argument list positionally, mirroring
        // `unify_poly_input`'s own `App` decomposition one level up (there,
        // against a concrete `Type`; here, symbolically against a poly-body
        // operand's own `PolyType::Generic` shape).
        PolyType::App {
            head,
            args: field_args,
        } => {
            let mismatch = || {
                poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(field_pty, sig),
                    &poly_type_str(operand, sig),
                )
            };
            let PolyType::Generic {
                is_enum,
                idx,
                module,
                args: op_args,
                ..
            } = operand
            else {
                return Err(mismatch());
            };
            if op_args.len() != field_args.len() {
                return Err(mismatch());
            }
            let gid = GenericId {
                is_enum: *is_enum,
                idx: *idx,
                module: *module,
            };
            let ctor = PolyType::Concrete(match ctx.generics() {
                Some(cell) => crate::ast::ctor_image_type(&cell.borrow(), gid),
                None => Type::CtorImage(gid, "<constructor>"),
            });
            let slot = &mut args[*head as usize];
            match slot {
                Some(existing) if existing != &ctor => {
                    return Err(poly_rendered_type_mismatch_error(
                        ctx,
                        span,
                        name,
                        &poly_type_str(existing, sig),
                        &poly_type_str(&ctor, sig),
                    ));
                }
                _ => *slot = Some(ctor),
            }
            for (fa, oa) in field_args.iter().zip(op_args.iter()) {
                poly_bind_construction_arg(fa, oa, args, sig, ctx, span, name)?;
            }
            Ok(())
        }
        // P7b.S6 (M4/R3): the exact dual of `substitute_generic_variant_field`'s
        // new `OwnedCell` arm -- `^List['T]`'s payload binds header
        // variables from the operand's own `OwnedCell` payload, recursing
        // one level in rather than grounding (grounding is `apply_subst`'s
        // job, same rationale as the sibling function).
        PolyType::OwnedCell(payload) => {
            let mismatch = || {
                poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(field_pty, sig),
                    &poly_type_str(operand, sig),
                )
            };
            let PolyType::OwnedCell(op_payload) = operand else {
                return Err(mismatch());
            };
            poly_bind_construction_arg(payload, op_payload, args, sig, ctx, span, name)
        }
        // P7b.S8b (PB-1): the self-reference field's own payload shape --
        // `^List['T]` unwraps one level up (the `OwnedCell` arm) and this is
        // what remains: a *named* header reference (`List`, not an abstract
        // `'F`). The `App` arm's twin with the header already concrete: the
        // operand must be a `Generic` naming the same header (identity is
        // `(is_enum, idx, module)`), else a located mismatch, mirroring the
        // `App` arm's treatment of non-`Generic` operands -- never a panic,
        // never a silent bind. With identity matched, the bind recurses
        // positionally over the field's arguments against the operand's own
        // (the header itself is already fixed, so only the arguments bind);
        // grounding to the concrete instantiation stays `apply_subst`'s job,
        // as for every sibling arm.
        PolyType::Generic {
            is_enum,
            idx,
            module,
            args: field_args,
            len_args: field_len_args,
            name: field_header_name,
        } => {
            let mismatch = || {
                poly_rendered_type_mismatch_error(
                    ctx,
                    span,
                    name,
                    &poly_type_str(field_pty, sig),
                    &poly_type_str(operand, sig),
                )
            };
            // P7b.S8b (Phase 2 review, P1; amended by the round-1 review):
            // a length *variable* in a construction field has no slot to
            // bind into -- `poly_construct_generic` infers no lengths (its
            // symbolic result carries a permanent empty `len_args`) -- so a
            // field carrying one is rejected here, located, by a dedicated
            // message that names the header and says what is unbindable.
            //
            // The fence runs *before* any `mismatch()` render. That ordering
            // was originally justified as ICE-safety -- a field's
            // length-variable id lives in its own header's declaration space,
            // not in the caller's `sig`, so reaching the renderer indexed out
            // of bounds. It no longer is: the round-3 review found the same
            // panic arriving through a *type* variable, which this fence
            // cannot cover (binding field type variables is the arm's whole
            // job), so the renderer itself was made total
            // (`foreign_var_str`). What the ordering buys now is message
            // quality: a dedicated "cannot bind ...'s length variable" beats
            // a mismatch whose expected side reads `Ring['T '?len0]`. The
            // operand's lengths are read here, ahead of the destructure, for
            // the same reason (`mismatch()` renders both sides).
            // That second half costs one wording imprecision: the message's
            // "the field names ..." clause attributes the length parameter
            // to the field, so an all-concrete field meeting a
            // variable-length *operand* is described slightly wrong. It is
            // still the accurate lead ("cannot bind ...'s length variable":
            // there is one, and it cannot be bound), and one covering fence
            // is worth more than a precise noun split across two.
            //
            // A *concrete* length is a different case and is deliberately
            // not fenced: it names no variable, so there is nothing to bind
            // and nothing to infer. Equal concrete lengths simply ground
            // (the positional argument bind below), and unequal ones are an
            // ordinary located mismatch -- `poly_type_str` *does* render
            // length arguments (`Ring[i64 3]` vs `Ring[i64 5]`, see
            // `poly_type_str_renders_a_generic_application_with_len_args`),
            // so the two sides print distinguishably and the standard
            // renderer is honest for that case where it would not be for a
            // variable.
            let operand_len_args: &[Len] = match operand {
                PolyType::Generic { len_args, .. } => len_args,
                _ => &[],
            };
            if field_len_args
                .iter()
                .chain(operand_len_args)
                .any(|l| matches!(l, Len::Var(_)))
            {
                return Err(poly_generic_field_len_unbound_error(
                    ctx,
                    span,
                    name,
                    field_header_name,
                ));
            }
            let PolyType::Generic {
                is_enum: op_is_enum,
                idx: op_idx,
                module: op_module,
                args: op_args,
                ..
            } = operand
            else {
                return Err(mismatch());
            };
            if *is_enum != *op_is_enum || *idx != *op_idx || *module != *op_module {
                return Err(mismatch());
            }
            // Both sides are all-concrete past the fence above, so this is a
            // plain value comparison and `mismatch()` can render it.
            if field_len_args.as_slice() != operand_len_args {
                return Err(mismatch());
            }
            // Phase 2 review (P2): defense-only past this point -- the
            // identity check above (`is_enum`/`idx`/`module`) already pins
            // one declared header, whose own arity is fixed at
            // registration, so `field_args.len()` and `op_args.len()` can
            // only disagree if the *other* side (the operand) is somehow
            // malformed; a live checker never constructs one.
            if op_args.len() != field_args.len() {
                return Err(mismatch());
            }
            for (fa, oa) in field_args.iter().zip(op_args.iter()) {
                poly_bind_construction_arg(fa, oa, args, sig, ctx, span, name)?;
            }
            Ok(())
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

/// P7.S12 (R6.1/R6.2): the exact dual of `poly_bind_construction_arg` --
/// construction binds header variables *from* operands, this applies them
/// *to* fields. `None` unless the call's own top operand is a narrowed
/// `PolyType::GenericVariant` and `name` is that variant's own `{name}>`
/// destructure spelling: unlike `poly_construction_header`, there is nothing
/// to search here -- the operand already names its header, module and
/// variant index (R5.4's scrutinee `args`, carried unchanged) -- so a bare
/// name match against *this* variant is the whole gate.
///
/// R6.3: a zero-field variant's field list is empty, so the loop below runs
/// zero times and the call destructures to nothing, exactly the concrete
/// rule. R6.4 (out of scope): a generic body has no field-projection route at
/// all, so nothing here needs to reject one specially.
pub(super) fn poly_destructure_generic(
    name: &str,
    span: Span,
    stack: &mut Vec<PolySlot>,
    ctx: &Ctx,
    tctx: &mut TraitCtx,
) -> Option<Vec<PolySlot>> {
    let top = stack.last()?;
    let PolyType::GenericVariant {
        idx,
        module,
        vi,
        args,
        len_args,
        ..
    } = &top.pt
    else {
        return None;
    };
    let (idx, module, vi, args, len_args) = (*idx, *module, *vi, args.clone(), len_args.clone());
    let cell = ctx
        .generics()
        .expect("a `GenericVariant` operand is only built from a live instantiator");
    let generics = cell.borrow();
    let decl = &generics.enums[idx as usize];
    let variant = &decl.variants[vi];
    // R5.2/R6.2: one variant-name rule for both sides of the intercept. The
    // arm-tag side compares through `generic_surface_name`, so this gate does
    // too, rather than against a raw `name` a monomorph-minted header decl
    // would carry a `[...]` suffix on.
    if name
        .strip_suffix('>')
        .is_none_or(|v| v != generic_surface_name(&variant.name))
    {
        return None;
    }
    let field_ptys: Vec<PolyType> = variant.fields.iter().map(|(_, p)| p.clone()).collect();
    let header_name: &'static str = Box::leak(decl.name.clone().into_boxed_str());
    drop(generics);

    stack.pop();
    // R6.1: fields push in declared order, first field deepest -- an
    // ordinary forward loop already leaves the first field pushed deepest,
    // mirroring the concrete `EnumWord::Destructure` lowering.
    for field_pty in &field_ptys {
        let substituted = crate::ast::substitute_generic_variant_field(field_pty, &args);
        stack.push(PolySlot::new(substituted));
    }
    // P7.S12 (R1.2/R6.1): record this call site's header, exactly as
    // `poly_construct_generic` does for a construction call, so lowering can
    // ground the right monomorph's `EnumId` for the bare-key
    // `EnumWord::Destructure` lookup (`apply_subst`'s `Generic` arm grounds
    // this to `Type::Enum`, which is what `check_poly_call`'s `enum_words`
    // fixpoint filters on).
    // P7.S6a (R3): the scrutinee's own `len_args` -- carried forward
    // unchanged, the same shape `generic_variant_type`'s own carry-forward
    // uses one level up.
    tctx.enum_sites.push((
        span,
        PolyType::Generic {
            is_enum: true,
            idx,
            module,
            args,
            len_args,
            name: header_name,
        },
    ));
    Some(std::mem::take(stack))
}

/// P7 slice 3a (R3): a call to `name` naming a generic struct's constructor
/// or a generic enum's variant, in a polymorphic body. `Ok(None)` if `name`
/// names no generic header at all, or if an exact concrete `env` candidate
/// already covers this call (`poly_env_exact_match`) -- either way the
/// caller's existing dispatch handles it unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn poly_construct_generic(
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
    // The header lookup moves ahead of the env-exact-match gate: whether the
    // gate may apply at all depends on the constructor's field count (S2-13).
    let Some((is_enum, idx, variant)) =
        poly_construction_header(&cell.borrow(), name, ctx.module())
    else {
        return Ok(None);
    };
    let fieldless = {
        let generics = cell.borrow();
        if is_enum {
            generics.enums[idx].variants[variant].fields.is_empty()
        } else {
            generics.structs[idx].fields.is_empty()
        }
    };
    // P7b.S2 (S2-13, F14): a *zero-field* constructor's generated word (one
    // exists for every concrete instantiation of the header registered
    // anywhere in the program) has an empty input row, so the exact-match
    // gate below is trivially true and would capture the call, minting that
    // instantiation's mono type where the ambient type variable was expected
    // (the poly `mapover`'s None arm leaving `Option[i64]` against the Some
    // arm's `Option['U]`). The symbolic path must run instead, so the
    // constructor's argument binds from the ambient context exactly as a
    // field-carrying constructor's does -- from its operands, or from the
    // declared output naming this header (the fallback below). A poly body
    // with no ambient determination still gets the honest
    // `poly_generic_constructor_undetermined_error`, never a random
    // instantiation's type.
    if !fieldless && poly_env_exact_match(env, name, stack) {
        return Ok(None);
    }
    let generics = cell.borrow();
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
        None => (
            // P7b.S4 candidate C (m5): key the mint on the header's DECLARING
            // module, not the enclosing word's naming module -- identity of a
            // generic instantiation belongs to the module that declared the
            // header, so every producer and consumer agrees on one mint.
            if is_enum {
                generics.enums[idx].module
            } else {
                generics.structs[idx].module
            },
            Vec::new(),
        ),
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
        // P7.S6a (R5) / P7b.S8b Phase 2 (P3): a length can only ever come
        // from a `Generic` field naming a length variable -- `Var`/`Concrete`/
        // `App`/`OwnedCell` fields carry none by construction, and the
        // `Generic` field arm's own `len_args` fence
        // (`poly_bind_construction_arg`, `poly_generic_field_len_unbound_error`)
        // rejects the one shape that could, located, before reaching here.
        // So this call never has a length to infer -- a permanent empty
        // list, not a phase-scoped placeholder.
        let ty = if is_enum {
            g.instantiate_enum(idx, &concrete_args, &[], module, regs)
        } else {
            g.instantiate_struct(idx, &concrete_args, &[], module, regs)
        };
        PolyType::Concrete(ty)
    } else {
        PolyType::Generic {
            is_enum,
            idx: idx as u32,
            module,
            args: resolved,
            len_args: vec![],
            name: header_name,
        }
    };
    // P7.S12 (R1.2/R6.1): record this generated-enum-word call site so
    // `check_poly_call` can ground it against a concrete θ later -- only the
    // enum case is recorded, this phase's scope (B1/B2/B3 are all enum
    // repros). Review note: a struct constructor is *not* actually safe from
    // the same hazard -- `type: Cell['T] val 'T ;` with a `wrap` word
    // constructing it at two asymmetric monomorphs (`i64` then `Pt`, or the
    // reverse) segfaults or ICEs in the backend today, live at HEAD,
    // measured pre-existing and out of this phase's enum-only scope. A
    // struct twin of R1.1/R1.2/R1.3 is a follow-up phase's to close.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        ArrayDecl, GenericEnumDecl, GenericStructDecl, GenericVariantDecl, OwnedCellDecl,
    };

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

    /// P7b.S4 (S4-1): `poly_construct_generic`'s no-fallback arm (the word's
    /// declared output names no header) keys a fresh mint on the header's
    /// *declaring* module, not the enclosing word's module. Built directly --
    /// the header is declared in module 0 while the poly word (and `ctx`)
    /// live in module 1, a two-module shape `check_src` cannot spell -- so
    /// the all-concrete construction must land in `inst_structs` stamped 0:
    /// the identity every producer and consumer agrees on (S4-2's one-mint
    /// property).
    #[test]
    fn poly_construct_generic_no_fallback_mint_keys_the_declaring_module() {
        let mut generics = GenericTypes::with_bases(0, 0);
        generics.structs.push(GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: Vec::new(),
            len_var_names: vec![],
            fields: vec![("val".to_string(), PolyType::Var(0))],
            span: Span::default(),
            module: 0,
        });
        let cell = RefCell::new(generics);
        // An output naming no header: the fallback arm's `None` case.
        let sig = PolySig {
            row_in: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            row_out: None,
            bounds: Vec::new(),
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: Vec::new(),
            ty_kinds: Vec::new(),
            len_var_names: vec![],
            len_var_spans: Vec::new(),
            row_var_names: Vec::new(),
        };
        let effect = StackEffect {
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let enums: Vec<EnumDecl> = Vec::new();
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut cells: Vec<OwnedCellDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let env = HashMap::new();
        let mut obligations = Vec::new();
        let mut enum_sites = Vec::new();
        let mut tctx = TraitCtx::scratch(&mut obligations, &mut enum_sites);
        let ctx = Ctx {
            mangled: "f",
            effect: &effect,
            structs: &[],
            enums: &enums,
            statics: &[],
            module: 1,
            modules: None,
            self_tail_call: false,
            generics: Some(&cell),
        };
        let mut stack = vec![PolySlot::new(PolyType::Concrete(Type::I64))];
        let next = poly_construct_generic(
            "Box",
            Span::default(),
            &mut stack,
            &sig,
            &ctx,
            &env,
            &[],
            &enums,
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut tctx,
        )
        .expect("a concrete construction in a poly body mints");
        assert!(next.is_some(), "the construction is handled, not delegated");
        let g = cell.borrow();
        assert_eq!(g.inst_structs.len(), 1);
        assert_eq!(g.inst_structs[0].name, "Box[i64]");
        assert_eq!(
            g.inst_structs[0].module, 0,
            "the mint keys on the declaring module (0), not the word's module (1)"
        );
    }

    /// P7b.S6 (M4/R3): the arm this phase adds -- `^List['T]`'s own shape,
    /// binding a header variable through the constructed operand's own
    /// `OwnedCell` payload rather than panicking on the previously-
    /// `unreachable!` catch-all.
    #[test]
    fn poly_bind_construction_arg_owned_cell_binds_var_from_payload() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = bare_sig();
        let field_pty = PolyType::OwnedCell(Box::new(PolyType::Var(0)));
        let operand = PolyType::OwnedCell(Box::new(PolyType::Concrete(Type::I64)));
        let mut args: Vec<Option<PolyType>> = vec![None];
        poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .expect("an OwnedCell field over an OwnedCell operand binds");
        assert_eq!(args[0], Some(PolyType::Concrete(Type::I64)));
    }

    /// The rejecting half: an operand that is not itself an `OwnedCell`
    /// (a plain concrete value where the field declares `^'T`) is a type
    /// mismatch, not a panic.
    #[test]
    fn poly_bind_construction_arg_owned_cell_operand_mismatch_is_error() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::OwnedCell(Box::new(PolyType::Var(0)));
        let operand = PolyType::Concrete(Type::I64);
        let mut args: Vec<Option<PolyType>> = vec![None];
        let err = poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "{err}");
    }

    /// P7b.S8b (PB-1): the new `Generic` field arm -- `^List['T]`'s own
    /// shape once the `OwnedCell` arm has already unwrapped one level.
    /// Same-identity operand binds the header's own argument positionally.
    #[test]
    fn poly_bind_construction_arg_generic_same_identity_binds_arg() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = bare_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: Vec::new(),
            name: "List",
        };
        let operand = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: Vec::new(),
            name: "List",
        };
        let mut args: Vec<Option<PolyType>> = vec![None];
        poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .expect("same-identity Generic operand binds the element var");
        assert_eq!(args[0], Some(PolyType::Concrete(Type::I64)));
    }

    /// A differently-headed `Generic` operand (a distinct `idx`) is a
    /// located mismatch, not a bind -- the rendered mismatch names both
    /// sides.
    #[test]
    fn poly_bind_construction_arg_generic_different_header_is_error() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: Vec::new(),
            name: "List",
        };
        let operand = PolyType::Generic {
            is_enum: true,
            idx: 1,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: Vec::new(),
            name: "Other",
        };
        let mut args: Vec<Option<PolyType>> = vec![None];
        let err = poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "{err}");
    }

    /// PB-3: a `Concrete` operand reaching the `Generic` field arm (not
    /// observed live today, but the arm's own rejection contract) is the
    /// same located mismatch, mirroring the `App` arm's treatment of
    /// non-`Generic` operands -- never a panic, never a silent bind.
    #[test]
    fn poly_bind_construction_arg_generic_concrete_operand_is_error() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: Vec::new(),
            name: "List",
        };
        let operand = PolyType::Concrete(Type::I64);
        let mut args: Vec<Option<PolyType>> = vec![None];
        let err = poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "{err}");
    }

    /// PB-2 / Phase 2 review (P1), amended by the round-1 review: a
    /// `Generic` field carrying a length *variable* has no slot to bind a
    /// length into, so it is rejected -- via the dedicated
    /// `poly_generic_field_len_unbound_error`, not
    /// `poly_rendered_type_mismatch_error`, because a field's length-variable
    /// id lives in its own header's space, which the caller's
    /// `len_var_names` cannot name -- a rendered mismatch would show
    /// `'?len0` where a dedicated message can say what is actually
    /// unbindable. (Rendering it is merely unhelpful, not fatal: the round-3
    /// review made `poly_type_str` total.) The shape is spellable
    /// (`Ring['T 'N: Len] head 'T next ^Ring['T 'N]`, `tests/phase7b_slice8b.rs`'s
    /// `generic_field_with_a_length_variable_is_a_located_error`), not future
    /// work. Its all-concrete siblings are the two tests below: a *concrete*
    /// length names no variable, so it grounds when it agrees and is an
    /// ordinary rendered mismatch when it does not.
    #[test]
    fn poly_bind_construction_arg_generic_nonempty_len_args_is_error() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0)],
            name: "List",
        };
        let operand = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: Vec::new(),
            name: "List",
        };
        let mut args: Vec<Option<PolyType>> = vec![None];
        let err = poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .unwrap_err();
        assert!(
            err.contains("cannot bind") && err.contains("length variable"),
            "{err}"
        );
    }

    /// P7b.S8b (round-1 review, P2): a *concrete* length carries no variable
    /// and needs no binding, so a field naming one grounds against an operand
    /// naming the same length -- the pre-amendment fence rejected this shape
    /// with "cannot bind `Ring`'s length variable", naming a variable that is
    /// not there. `args` records the element binding, proving the arm reached
    /// its positional bind rather than returning early.
    #[test]
    fn poly_bind_construction_arg_generic_matching_concrete_len_binds_arg() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Concrete(3)],
            name: "Ring",
        };
        let operand = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: vec![Len::Concrete(3)],
            name: "Ring",
        };
        let mut args: Vec<Option<PolyType>> = vec![None];
        poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Node",
        )
        .expect("a matching concrete length grounds");
        assert_eq!(args[0], Some(PolyType::Concrete(Type::I64)));
    }

    /// P7b.S8b (round-1 review, P2): concrete lengths that *disagree* are a
    /// real type mismatch, and the standard rendered error is honest for it
    /// -- `poly_type_str` renders length arguments, so the two sides print
    /// distinguishably (`Ring['T 3]` against `Ring[i64 5]`) where a length
    /// variable could not be rendered at all.
    #[test]
    fn poly_bind_construction_arg_generic_differing_concrete_len_is_error() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Concrete(3)],
            name: "Ring",
        };
        let operand = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: vec![Len::Concrete(5)],
            name: "Ring",
        };
        let mut args: Vec<Option<PolyType>> = vec![None];
        let err = poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Node",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "{err}");
        assert!(
            err.contains("3") && err.contains("5"),
            "both lengths: {err}"
        );
    }

    /// Phase 2 review (P2): the arm's identity check is `(is_enum, idx,
    /// module)`, not `(is_enum, idx)` -- an operand naming the same header
    /// index but a *different* declaring module (a same-named generic
    /// header can exist in two modules) is still a located mismatch, not a
    /// silent bind. Deleting `*module != *op_module` from the identity
    /// check survives every other test in this file, since none of them
    /// vary only this component.
    #[test]
    fn poly_bind_construction_arg_generic_different_module_is_error() {
        let word = probe_word();
        let ctx = probe_ctx(&word);
        let sig = ref_sig();
        let field_pty = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: Vec::new(),
            name: "List",
        };
        let operand = PolyType::Generic {
            is_enum: true,
            idx: 0,
            module: 1,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: Vec::new(),
            name: "List",
        };
        let mut args: Vec<Option<PolyType>> = vec![None];
        let err = poly_bind_construction_arg(
            &field_pty,
            &operand,
            &mut args,
            &sig,
            &ctx,
            Span::default(),
            "Cons",
        )
        .unwrap_err();
        assert!(err.contains("type mismatch"), "{err}");
    }

    /// P7.S6a (R3, added review round 4): `poly_destructure_generic`'s own
    /// `enum_sites.push` (`:4406-4412`) rebuilds a `PolyType::Generic` from
    /// the narrowed `PolyType::GenericVariant` operand's own `len_args` --
    /// must fail if this carry-forward is dropped, silently losing the
    /// length the moment lowering re-grounds it.
    #[test]
    fn poly_destructure_generic_enum_sites_push_carries_len_args() {
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
        let scrutinee_len_args = vec![Len::Var(0)];
        let scrutinee_pt = crate::ast::generic_variant_type(
            &cell.borrow(),
            0,
            0,
            0,
            vec![PolyType::Var(0)],
            scrutinee_len_args.clone(),
        );
        let mut stack = vec![PolySlot::new(scrutinee_pt)];
        let effect = StackEffect {
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let enums: Vec<EnumDecl> = Vec::new();
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
        let mut obligations = Vec::new();
        let mut enum_sites = Vec::new();
        let mut tctx = TraitCtx::scratch(&mut obligations, &mut enum_sites);
        poly_destructure_generic("Full>", Span::default(), &mut stack, &ctx, &mut tctx)
            .expect("a `Full>` destructure over the matching narrowed variant must dispatch");
        assert_eq!(enum_sites.len(), 1, "the destructure call site is recorded");
        let PolyType::Generic { len_args, .. } = &enum_sites[0].1 else {
            panic!(
                "the recorded site is a Generic scrutinee: {:?}",
                enum_sites[0].1
            )
        };
        assert_eq!(len_args, &scrutinee_len_args);
    }
}
