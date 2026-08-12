use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn check_word(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    modules: Option<&[ModuleInfo]>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // A parameter name equal to a registered variant name is rejected (X12)
    // regardless of body form.
    let ctx = word_ctx(word, structs, enums, modules);
    for slot in &word.effect.inputs {
        if let Some(name) = &slot.name {
            reject_variant_local(&ctx, name, "parameter")?;
        }
    }
    check_reference_free_signature(&word.name, &word.effect, structs, enums, arrays)?;
    match &word.body {
        WordBody::Terms { terms } => check_terms_word(
            word, enums, terms, env, arrays, cells, refs, structs, modules, dropped, poly,
        ),
        WordBody::Clauses(clauses) => check_clause_word(
            word, enums, clauses, env, arrays, cells, refs, structs, modules, dropped, poly,
        ),
    }
}

/// The effect-signature half of the no-stored-reference rule: no declared
/// **output** may transitively
/// contain a reference (returning one would outlive the frame that owns the
/// referent), and an **input** may only be a reference at the top level — a
/// type that merely *contains* one nested inside an array or a cell is
/// rejected there too, so the carve-out stays closed if a future aggregate
/// constructor arrives.
pub(super) fn check_reference_free_signature(
    name: &str,
    effect: &StackEffect,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    for slot in &effect.outputs {
        if contains_reference(slot.ty, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{}` declares the output `{}`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead",
                name, slot.ty
            ));
        }
    }
    for slot in &effect.inputs {
        if !slot.ty.is_ref() && contains_reference(slot.ty, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{}` declares the input `{}`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate",
                name, slot.ty
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_terms_word(
    word: &WordDef,
    enums: &[EnumDecl],
    terms: &[Term],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    modules: Option<&[ModuleInfo]>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // R3: a binding is an ordinary term, but the *entry* one keeps its own
    // diagnostic. Only there is the declared effect the frame, so only there
    // can the message cite it; the generic underflow of R5 covers every other
    // position.
    let inputs = word.effect.inputs.len();
    if let Some(TermKind::Bind(names)) = terms.first().map(|t| &t.kind) {
        if names.len() > inputs {
            return Err(format!(
                "error: stack effect mismatch in `{}`\n  locals bind {} value(s), but only {} input(s) are declared\n  note: declared {}",
                crate::resolve::demangle_word(&word.name),
                names.len(),
                inputs,
                effect_str(&word.effect),
            ));
        }
    }

    // The declared inputs are the word's whole frame: it cannot see beneath
    // them (R5), and an entry binding pops from them like any other binding.
    let initial: Vec<Slot> = word
        .effect
        .inputs
        .iter()
        .map(|s| Slot::computed(s.ty))
        .collect();

    let ctx = word_ctx(word, structs, enums, modules);
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    let mut final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, &mut prov, &mut scope, true, poly,
    )?;

    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();
    let line = terms.last().map(|t| t.span.line).unwrap_or(0);
    // R7/R15/D4: a declared `Type::Quotation` output is a materialization
    // boundary. Materialize each `Known` literal the body leaves there --
    // running the R15 admission rule on a capturing one (`be returned` is an
    // escaping boundary) -- before `check_outputs`, whose bare-quotation guard
    // would otherwise reject it outright.
    for (i, want) in declared.iter().enumerate() {
        if let Type::Quotation(eff) = *want {
            if let Some(QuotRef::Known(id)) = final_stack.get(i).and_then(|s| s.quot) {
                let span = prov.quotations[id.0].span;
                final_stack[i] = materialize_quotation_at_boundary(
                    id, eff, true, &word.name, span, &ctx, env, arrays, cells, refs, &mut prov,
                    &mut scope, poly,
                )?;
            }
        }
    }
    // R22: the word-output escape guard. A returned *carrier* -- a struct or
    // array whose surviving set (R19) picked up a frame-rooted capture at an
    // in-frame store -- would let that capture's frame storage die at return
    // while the stored closure still points into it. `contains_reference`
    // (`check.rs:285`) is a shallow structural walk blind to the erased env, so
    // this is a targeted walk over each returned slot's surviving set. A
    // directly-returned closure with a frame capture never reaches here: its
    // "be returned" boundary already raised past-owning-frame (escaping).
    //
    // Review fix: a second, independent hazard shares this guard. A 2+-total-
    // capture closure materializes a *stack-allocated* env bundle (R16) in
    // the frame that builds it, even when every individual capture is
    // outer-rooted -- the bundle's own storage still dies at return. Built
    // in-frame (a struct/array store), that closure has `escaping = false`,
    // so R18's direct multi-capture rejection never runs; only surfaces once
    // the carrier holding it is itself returned. `surviving_set_is_bundle`
    // carries that signal (independent of `frame_rooted`: a scalar+reference
    // bundle has only one surviving member, so member count cannot recover
    // it) and is checked here, second.
    if let Some(exit) = terms.last().map(|t| t.span) {
        for slot in &final_stack {
            if let Some(set) = slot.surviving {
                if let Some(member) = prov.surviving_set(set).iter().find(|m| m.frame_rooted) {
                    return Err(past_owning_frame_error(&ctx, exit, &member.name));
                }
                if prov.surviving_set_is_bundle(set) {
                    return Err(multi_capture_escaping_error(&ctx, exit));
                }
            }
        }
    }
    dropped.append(&mut prov.dropped);

    check_outputs(word, &final_stack, &declared, line, structs, enums, arrays)?;
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))
}

/// Check a clause-style word (D4, D5, D7, M6, R11): the top input must be an
/// enum (X7), the clauses must cover every variant exactly once (X4/X5/X6),
/// and every clause body must leave the word's single declared output effect
/// (X8).
#[allow(clippy::too_many_arguments)]
fn check_clause_word(
    word: &WordDef,
    enums: &[EnumDecl],
    clauses: &[Clause],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    modules: Option<&[ModuleInfo]>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // The top input may be a plain enum (value mode) or a reference to
    // one (reference mode, `&Enum`/`&!Enum`) — the mode follows the declared
    // type, never inferred. `ref_mutable` is `None` in value mode, `Some`
    // (carrying the reference's mutability) in reference mode.
    let (enum_id, ref_mutable) = match word.effect.inputs.last().map(|s| s.ty) {
        Some(Type::Enum(id, _)) => (id, None),
        Some(Type::Ref(rid, mutable, _)) => match refs[rid.index()].referent {
            Type::Enum(id, _) => (id, Some(mutable)),
            _ => {
                return Err(format!(
                    "error: clause-style body on `{}` whose top input is not an enum\n  note: declared {}",
                    crate::resolve::demangle_word(&word.name),
                    effect_str(&word.effect),
                ));
            }
        },
        _ => {
            return Err(format!(
                "error: clause-style body on `{}` whose top input is not an enum\n  note: declared {}",
                crate::resolve::demangle_word(&word.name),
                effect_str(&word.effect),
            ));
        }
    };
    let enum_decl = &enums[enum_id.index()];
    let enum_name = enum_decl.name.as_str();

    let n_inputs = word.effect.inputs.len();
    let below: Vec<Type> = word.effect.inputs[..n_inputs - 1]
        .iter()
        .map(|s| s.ty)
        .collect();
    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();

    // Validate every clause's variant identity and uniqueness before checking
    // any body (R8): a clause-body binding that leads with a registered
    // variant name is silently reparsed as the next clause, and if that
    // reparse produces an unknown-variant or duplicate-clause problem, it
    // must be reported before a downstream sibling body is checked against
    // the terms the reparse ate out from under it.
    let mut seen: HashMap<&str, ()> = HashMap::new();
    let mut variant_indices = Vec::with_capacity(clauses.len());
    for clause in clauses {
        let Some(vi) = enum_decl
            .variants
            .iter()
            .position(|v| v.name == clause.variant)
        else {
            return Err(format!(
                "error: unknown variant `{}` of enum `{}` in clause-style `{}` (line {}){}",
                clause.variant,
                enum_name,
                crate::resolve::demangle_word(&word.name),
                clause.span.line,
                clause_variant_ambiguity_note(&clause.variant),
            ));
        };
        if seen.insert(clause.variant.as_str(), ()).is_some() {
            return Err(format!(
                "error: duplicate clause for variant `{}` of enum `{}` in `{}` (line {}){}",
                clause.variant,
                enum_name,
                crate::resolve::demangle_word(&word.name),
                clause.span.line,
                clause_variant_ambiguity_note(&clause.variant),
            ));
        }
        variant_indices.push(vi);
    }
    // Exhaustiveness is part of that same pre-pass: a clause body that ate a
    // sibling's terms (a misspelt variant name reads as a binding, D8) leaves
    // that sibling missing, and "missing variant `B`" names the real fault
    // where the swollen body's own arity failure would misattribute it.
    for variant in &enum_decl.variants {
        if !seen.contains_key(variant.name.as_str()) {
            return Err(format!(
                "error: non-exhaustive clause-style `{}`: missing variant `{}` of enum `{}`",
                word.name, variant.name, enum_name
            ));
        }
    }
    for (clause, &vi) in clauses.iter().zip(&variant_indices) {
        check_clause_body(
            word,
            enums,
            clause,
            &below,
            &enum_decl.variants[vi],
            &declared,
            env,
            arrays,
            cells,
            refs,
            structs,
            modules,
            ref_mutable,
            dropped,
            poly,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn check_clause_body(
    word: &WordDef,
    enums: &[EnumDecl],
    clause: &Clause,
    below: &[Type],
    variant: &VariantDecl,
    declared: &[Type],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    structs: &[StructDecl],
    modules: Option<&[ModuleInfo]>,
    ref_mutable: Option<bool>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    let ctx = word_ctx(word, structs, enums, modules);
    let mut seen_locals = HashSet::new();
    for name in &clause.locals {
        reject_variant_local(&ctx, name, "local")?;
        reject_duplicate_local(&ctx, name, clause.span, &mut seen_locals)?;
    }

    // The clause consumes the scrutinee and pushes the variant's fields
    // (first field deepest) atop any inputs below it. In reference mode
    // every field arrives as a reference inheriting the scrutinee's
    // mutability, projecting through it exactly as a struct-field projection
    // would — the payload is never owned, so it is never moved or freed.
    let mut initial = below.to_vec();
    for (_, ty) in &variant.fields {
        let field_ty = match ref_mutable {
            Some(mutable) => intern_ref_type(refs, *ty, mutable),
            None => *ty,
        };
        initial.push(field_ty);
    }

    // Clause-body `| names |` bind the top N (payload then below), leftmost
    // deepest, reusing the word-entry local-binding shape.
    let n = clause.locals.len();
    if n > initial.len() {
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  clause `{}` binds {} value(s), but only {} are available\n  note: declared {}",
            word.name, clause.span.line, clause.variant, n, initial.len(), effect_str(&word.effect),
        ));
    }
    let split = initial.len() - n;
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    for (name, ty) in clause.locals.iter().zip(&initial[split..]) {
        let linear = is_linear(*ty, structs, enums, arrays);
        scope.bind(name, Slot::computed(*ty), linear, &mut prov);
    }
    let stack_after_bind: Vec<Slot> = initial[..split]
        .iter()
        .map(|ty| Slot::computed(*ty))
        .collect();

    let final_stack = check_terms(
        &clause.body,
        stack_after_bind,
        &ctx,
        env,
        arrays,
        cells,
        refs,
        &mut prov,
        &mut scope,
        true,
        poly,
    )?;
    dropped.append(&mut prov.dropped);
    let line = clause
        .body
        .last()
        .map(|t| t.span.line)
        .unwrap_or(clause.span.line);
    check_outputs(word, &final_stack, declared, line, structs, enums, arrays)?;
    leave_block(&ctx, &mut scope, 0, BlockEnd::Body(line))
}
