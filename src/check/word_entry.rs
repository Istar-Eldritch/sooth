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
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    // A parameter name equal to a registered variant name is rejected (X12)
    // regardless of body form.
    let ctx = word_ctx(
        word,
        structs,
        enums,
        statics,
        modules,
        poly.combinators.tail(),
    );
    for slot in &word.effect.inputs {
        if let Some(name) = &slot.name {
            reject_variant_local(&ctx, name, "parameter")?;
        }
    }
    // Slice 11 (R5/D5): the no-stored-reference signature check is skipped for
    // an always-spliced word, phrased over the shared `is_combinator` predicate so
    // the exemption covers a mono combinator, an `inline` word and (via the
    // concrete stand-in `check_poly_combinator_standalone` builds) a poly
    // combinator uniformly. Its own message names the fault it guards -- "a
    // `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the
    // time the caller reads it" -- and a spliced word has no such frame:
    // `alpha_rename_locals` makes the callee's locals caller locals, so the
    // returned reference cannot outlive the frame that owns its referent. Every
    // lifetime and linearity pass that makes the relaxation safe still runs on
    // both the standalone body and each spliced copy (the must-consume rule, the
    // capture/escape guards, the loop back-edge reference guard); a real
    // (non-combinator) word declaring a reference output is still rejected.
    if !is_combinator(word) {
        check_reference_free_signature(&word.name, &word.effect, structs, enums, arrays)?;
    }
    match &word.body {
        WordBody::Terms { terms } => check_terms_word(
            word, enums, terms, env, arrays, cells, refs, structs, statics, modules, dropped, poly,
        ),
        WordBody::Clauses(clauses) => check_clause_word(
            word, enums, clauses, env, arrays, cells, refs, structs, statics, modules, dropped,
            poly,
        ),
    }
}

/// Slice 11 (R3): the shapes a declared `inline` cannot deliver on. The
/// guarantee is unconditional (D2), so each is a located error at the
/// definition rather than a silent fall-back to a real call: a clause-bodied
/// word is not a combinator (`is_combinator` requires `WordBody::Terms`) and
/// would lower as an ordinary clause word; `main` is an entry point, not a
/// combinator, so splicing it away leaves the runtime shim's call to it
/// unresolved at link time; a builtin-operator name is claimed by
/// `check_operator` before the splice is reached, and the two then disagree;
/// and a variable-bearing signature is excluded by Decision 3.
///
/// Slice 10c (R-P3-3b): a polymorphic signature is **no longer** excluded. The
/// rule that excluded it was a policy one, not a soundness one -- the splice
/// already handles a variable-bearing body, so lifting it needed no lowering
/// work -- and slice 10c ships its first consumers: the six comparison words
/// (`: = inline ( 'T: Copy Ord 'T -- bool ) u= [ true ] [ false ] branch ;`),
/// which must be both `'T: Copy Ord`-polymorphic, to keep covering the whole
/// numeric tower, and `inline`, or every comparison in the language becomes a
/// real call with a frame. The builtin-name rule below is a *soundness* rule
/// and stays; the six escape it because their rows left `BUILTIN_TABLE`.
pub(crate) fn check_inline_declaration(word: &WordDef) -> Result<(), String> {
    if !word.declares_inline {
        return Ok(());
    }
    let name = crate::resolve::demangle_word(&word.name);
    let span = word.span;
    // The same `main`-is-not-a-combinator invariant
    // `audit_word_quotation_positions` enforces on the quotation route ("an
    // input of `main`", D6/R28); the declared flag is a second route to it.
    if word.name == "main" {
        return Err(format!(
            "error: `inline` on `main`, which is the program entry point; the entry point is called by the runtime shim and cannot be spliced (line {}, col {})",
            span.line, span.col
        ));
    }
    if matches!(word.body, WordBody::Clauses(_)) {
        return Err(format!(
            "error: `inline` on `{name}`, which has a clause body; `inline` requires a term body (line {}, col {})",
            span.line, span.col
        ));
    }
    // A builtin-operator name reaches `check_operator` first, which resolves
    // the overload and records `poly.builtin_overloads[span]` so lowering emits
    // a real `Instr::Call`; only then does the call fall through to the
    // combinator interception, which splices instead. The record survives the
    // splice, and lowering trusts it and looks the symbol up in an `env` a
    // combinator is excluded from -- a checker contradicting itself, and a
    // panic downstream. Widening `is_combinator` (R2) made the shape
    // reachable: an operator call site rejects a quotation operand outright,
    // so a builtin-name overload could not previously be a combinator.
    if BUILTIN_TABLE.contains_key(name) {
        return Err(format!(
            "error: `inline` on `{name}`, which overloads a builtin operator name; a call site of a builtin operator name dispatches through a real call and cannot be spliced (line {}, col {})",
            span.line, span.col
        ));
    }
    Ok(())
}

/// Slice 12 (R-B1): a `~[ ... ]` (`Type::InlineQuotation`) parameter is
/// unrepresentable at runtime -- it can only ever be spliced -- so a word
/// declaring one must also declare `inline`. This is the mirror rule of
/// `check_inline_declaration` above: that function rejects an `inline` the
/// splice cannot deliver on; this one rejects a `~` parameter the splice is
/// the *only* way to deliver on, absent `inline`. Phrased over
/// `Type::InlineQuotation` specifically, not `word_declares_quotation_parameter`
/// (which also matches an ordinary `Type::Quotation`): an ordinary `[ ... ]`
/// parameter is representable, so it is a real call by default (part D) and
/// needs no `inline`.
pub(crate) fn check_inline_quotation_requires_inline(word: &WordDef) -> Result<(), String> {
    if word.declares_inline {
        return Ok(());
    }
    let param = match &word.poly {
        None => word.effect.inputs.iter().find_map(|s| match s.ty {
            Type::InlineQuotation(_) => Some(s.ty.name().to_string()),
            _ => None,
        }),
        Some(sig) => sig.inputs.iter().find_map(|p| match p {
            PolyType::Quotation(_, _, true, ..) => Some(poly_type_str(p, sig)),
            PolyType::Concrete(t @ Type::InlineQuotation(_)) => Some(t.name().to_string()),
            _ => None,
        }),
    };
    let Some(param) = param else {
        return Ok(());
    };
    let name = crate::resolve::demangle_word(&word.name);
    let span = word.span;
    Err(format!(
        "error: word `{name}` declares an inline-quotation parameter `{param}` but is not `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare `inline` (line {}, col {})",
        span.line, span.col
    ))
}

/// The effect-signature half of the no-stored-reference rule: no declared
/// **output** may transitively
/// contain a reference (returning one would outlive the frame that owns the
/// referent), and an **input** may only be a reference at the top level — a
/// type that merely *contains* one nested inside an array or a cell is
/// rejected there too, so the carve-out stays closed if a future aggregate
/// constructor arrives.
///
/// Slice 11 (R5): `check_word` skips this whole check for a combinator, which
/// has no frame of its own to outlive. `check_extern_decls`
/// (`declarations.rs`) has no splice to exempt and always runs it.
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
    statics: &[StaticDecl],
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

    let ctx = word_ctx(
        word,
        structs,
        enums,
        statics,
        modules,
        poly.combinators.tail(),
    );
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
    statics: &[StaticDecl],
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
    // Surface name for diagnostics and clause matching: a monomorphized
    // generic enum stores mangled names (`Result[i64 i64]__m0`, `Ok[i64 i64]`)
    // so its per-instantiation constructor `Sig`s and variant word map do not
    // collide, but a clause names the bare surface variant (`Ok`). For a
    // concrete enum `generic_surface_name` is the identity.
    let enum_name = crate::ast::generic_surface_name(enum_decl.name.as_str());

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
            .position(|v| crate::ast::generic_surface_name(&v.name) == clause.variant)
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
        let variant_surface = crate::ast::generic_surface_name(&variant.name);
        if !seen.contains_key(variant_surface) {
            return Err(format!(
                "error: non-exhaustive clause-style `{}`: missing variant `{}` of enum `{}`",
                word.name, variant_surface, enum_name
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
            statics,
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
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    ref_mutable: Option<bool>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
) -> Result<(), String> {
    let ctx = word_ctx(
        word,
        structs,
        enums,
        statics,
        modules,
        poly.combinators.tail(),
    );
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

/// D8's clause-vs-binding disambiguation is global (`is_variant_name` scans
/// every enum in scope), so a `|` followed by a registered variant always
/// opens the next clause and is never read as a binding (R8). Every clause
/// the parser produces leads with a registered name, so this note states the
/// rule that was applied wherever a clause's variant is rejected.
fn clause_variant_ambiguity_note(name: &str) -> String {
    format!(
        "\n  note: `| {name}` is read as a clause because `{name}` is a variant name; a binding may not lead with one"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
    }
    #[test]
    fn check_term_word_with_entry_locals_still_ok() {
        // Regression: a plain term word with `| ... |` entry locals is
        // unaffected by the clause-body path (no enum in scope).
        check_src(": sq ( i64 -- i64 ) | n | n n * ;").unwrap();
    }

    /// Slice 12 (R-B1, M-B): a `~[ ... ]` parameter without `inline` is a
    /// located error naming the word and the parameter's rendered spelling.
    /// The mono and poly (variable-bearing) shapes both fire it; the same
    /// shape with `inline` added is accepted, so the rejection is keyed on
    /// the missing keyword, not the `~` parameter itself.
    #[test]
    fn check_missing_inline_on_tilde_parameter_is_error() {
        let err = check_src(
            ": twice ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;\n: main ( -- ) ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: word `twice` declares an inline-quotation parameter `~[ i64 -- i64 ]` but is not `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare `inline` (line 1, col 3)"
        );
        check_src(
            ": twice inline ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;\n: main ( -- ) ;\n",
        )
        .expect("the identical shape with `inline` declared is accepted");

        let poly_err = check_src(
            ": each ( ['T 'N] ~[ 'T -- ] -- )\n\
             | f | len >i64 | count | | arr | count [ | i | drop f call ] times\n\
             arr drop ;\n\
             : main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            poly_err.contains("declares an inline-quotation parameter")
                && poly_err.contains("`each`")
                && poly_err.contains("must declare `inline`"),
            "a variable-bearing `~` parameter is rejected too, got: {poly_err}"
        );

        // An ordinary `[ ... ]` parameter needs no `inline` at all: it is a
        // real call (part D's territory), so this gate must not fire on it.
        check_src(": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n: main ( -- ) ;\n")
            .expect("an ordinary `[ ... ]` parameter is exempt from this gate");
    }

    /// Slice 11 (R3): a clause-bodied `inline` word is `is_combinator == false`
    /// (the predicate requires `WordBody::Terms`), so accepting it would lower
    /// it as an ordinary clause word -- the silent fall-back to a real call D2
    /// forbids. It is a located error instead.
    #[test]
    fn check_inline_clause_body_is_error() {
        let err = check_src(
            "type: E | A | B ;\n\
             : pick inline ( E -- i64 )\n\
             | A  1\n\
             | B  2\n\
             ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `inline` on `pick`, which has a clause body; `inline` requires a term body (line 2, col 3)"
        );
        // The same clause body without `inline` is accepted, so the rejection is
        // the keyword's, not the body's.
        check_src(
            "type: E | A | B ;\n\
             : pick ( E -- i64 )\n\
             | A  1\n\
             | B  2\n\
             ;\n",
        )
        .expect("an ordinary clause word is unaffected");
    }

    /// Slice 10c (E-P3-7): **retargeted, not deleted.** Slice 11's rejection
    /// of an `inline` polymorphic signature was a *policy* rule by its own
    /// admission -- the splice already handles a variable-bearing body -- and
    /// R-P3-3b deliberately reverses it, because the six comparison words must
    /// be both `'T: Copy Ord`-polymorphic (to keep covering the numeric tower)
    /// and `inline` (or every comparison becomes a real call with a frame).
    /// The other half of the original pair, a `~`-bearing but variable-free
    /// effect, is unaffected and stays.
    ///
    /// The witness is a word named `=`, not a neutral name: a neutral name is
    /// claimed by no builtin, so it slips past the *second* (soundness)
    /// `inline` gate, `BUILTIN_TABLE.contains_key`, and would pass whether or
    /// not the real comparison words can ever be `inline`. Restoring the
    /// polymorphic gate rejects with `requires a monomorphic effect`; leaving
    /// the six rows in `BUILTIN_TABLE` under their old names rejects with
    /// `overlaps a concrete overload of `=``.
    #[test]
    fn check_inline_polymorphic_signature_is_accepted() {
        check_src(": id inline ( 'T -- 'T ) ;\n: main ( -- ) ;")
            .expect("`inline` on a polymorphic signature is a splice, not a rejection");
        // The witness is `lib/core.sth`'s own `=`, driven straight through
        // both gates: it cannot be *redeclared* in a test source (that is a
        // duplicate overload of the injected one), and a neutral stand-in
        // would not exercise the builtin-name gate at all.
        let eq = crate::parser::prelude_words()
            .into_iter()
            .find(|w| w.name == "=")
            .expect("`lib/core.sth` defines `=`");
        assert!(eq.declares_inline, "`=` is declared `inline`");
        let sig = eq.poly.as_ref().expect("`=` is polymorphic");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        check_inline_declaration(&eq)
            .expect("the real witness: a builtin-operator-named polymorphic `inline` word");
        check_src(": main ( -- ) 1 2 = drop 1 >u32 2 >u32 = drop ;")
            .expect("and it resolves across two distinct numeric types");
        check_src(": twice inline ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;")
            .expect("a `~`-bearing but variable-free `inline` effect is monomorphic");
    }

    /// Slice 11 (R5/D5): the no-reference-output rule is skipped for an
    /// always-spliced word. Post-R-A1 the skip's `is_combinator` and
    /// `declares_inline` are extensionally identical for anything `check_src`
    /// can reach here (a `~[ ... ]` parameter without `inline` is rejected
    /// first, by `check_inline_declaration`'s R-B1 neighbour), so the middle
    /// case below no longer discriminates the two phrasings; it still stands
    /// as a poly-combinator regression. The third case is the outermost
    /// boundary -- a real word, which does have a frame of its own to lose, is
    /// still rejected.
    #[test]
    fn check_reference_free_signature_skipped_for_combinator() {
        check_src("type: P n u32 ;\n: pick inline ( &!P -- &!u32 ) | p | p &!P>n ;\n")
            .expect("an `inline` word may declare a reference output");
        check_src(
            "type: P n u32 ;\n: pick inline ( &!P ~[ -- ] -- &!u32 ) | p f | f call p &!P>n ;\n",
        )
        .expect("a quotation-taking word is exempt too (the skip reads `is_combinator`)");
        // A *poly* combinator takes the same exemption by the same guard: it
        // reaches `check_word` through the concrete stand-in
        // `check_poly_combinator_standalone` builds, which carries the quotation
        // parameter (and the flag) across and so is itself `is_combinator`.
        check_src(
            "type: P n u32 ;\n: pick inline ( 'T &!P ~[ 'T -- ] -- &!u32 ) | v p f | v f call p &!P>n ;\n",
        )
        .expect("a poly combinator is exempt through its concrete stand-in");
        let err =
            check_src("type: P n u32 ;\n: pick ( &!P -- &!u32 ) | p | p &!P>n ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `pick` declares the output `&!u32`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
    }

    /// Slice 11 (R3): a builtin-operator name is resolved by `check_operator`,
    /// which records the site for a real call, *before* the same call falls
    /// through to the combinator splice -- so an `inline` overload of one leaves
    /// the checker contradicting itself and lowering panicking. Rejected at the
    /// definition instead. The name is demangled first: `mangle` suffixes an
    /// operator name per module (`+__m0`), so a raw comparison never matches.
    #[test]
    fn check_inline_builtin_operator_overload_is_error() {
        let err = check_src(
            "type: A n i64 ;\n\
             : + inline ( A A -- i64 ) | x y | x A>n drop y A>n drop 1000 ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `inline` on `+`, which overloads a builtin operator name; a call site of a builtin operator name dispatches through a real call and cannot be spliced (line 2, col 3)"
        );
        // A non-operator name with the identical shape is accepted, so the
        // rejection is keyed on the name, not on the overload.
        check_src(
            "type: A n i64 ;\n\
             : add inline ( A A -- i64 ) | x y | x A>n drop y A>n drop 1000 ;\n",
        )
        .expect("an `inline` word whose name no operator claims is accepted");
    }
}
