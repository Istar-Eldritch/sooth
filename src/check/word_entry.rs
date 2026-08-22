use std::cell::RefCell;

use crate::ast::GenericTypes;

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn check_word(
    word: &WordDef,
    enums: &[EnumDecl],
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    structs: &[StructDecl],
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
    generics: Option<&RefCell<GenericTypes>>,
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
        generics,
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
    let terms = &word.body;
    check_terms_word(
        word, enums, terms, env, arrays, cells, refs, slices, structs, statics, modules, dropped,
        poly, generics,
    )
}

/// Slice 11 (R3): the shapes a declared `inline` cannot deliver on. The
/// guarantee is unconditional (D2), so each is a located error at the
/// definition rather than a silent fall-back to a real call: `main` is an
/// entry point, not a combinator, so splicing it away leaves the runtime
/// shim's call to it unresolved at link time; and a builtin-operator name is
/// claimed by `check_operator` before the splice is reached, and the two then
/// disagree.
///
/// Slice 10c (R-P3-3b): a polymorphic signature is **no longer** excluded. The
/// rule that excluded it was a policy one, not a soundness one -- the splice
/// already handles a variable-bearing body, so lifting it needed no lowering
/// work -- and slice 10c ships its first consumers: the six comparison words
/// (`: eq inline ( 'T: Copy Ord 'T -- bool ) ueq [ true ] [ false ] branch ;`),
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
            return Err(stored_reference_output_error(name, slot.ty, ""));
        }
    }
    for slot in &effect.inputs {
        if !slot.ty.is_ref() && contains_reference(slot.ty, structs, enums, arrays) {
            let ty = slot.ty;
            return Err(format!(
                "error: a reference cannot be stored: `{name}` declares the input `{ty}`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate"
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
    slices: &mut Vec<SliceDecl>,
    structs: &[StructDecl],
    statics: &[StaticDecl],
    modules: Option<&[ModuleInfo]>,
    dropped: &mut Vec<Type>,
    poly: &mut PolyCtx,
    generics: Option<&RefCell<GenericTypes>>,
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
        generics,
    );
    let mut scope = Scope::default();
    let mut prov = Provenance::default();
    let mut final_stack = check_terms(
        terms, initial, &ctx, env, arrays, cells, refs, slices, &mut prov, &mut scope, true, poly,
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
                    id, eff, true, &word.name, span, &ctx, env, arrays, cells, refs, slices,
                    &mut prov, &mut scope, poly,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module)
    }
    /// P7 slice 3c (R5 + R1.4): the two halves of this function answer a slice
    /// oppositely, and both answers are load-bearing. The output loop tests
    /// `contains_reference` alone, so a declared `( -- Slice[i64] )` is
    /// rejected with the ordinary stored-reference wording; the input loop
    /// guards that test with `is_ref`, so `( Slice[i64] -- i64 )` -- the whole
    /// point of the type -- is admitted. Driven directly rather than through
    /// source: the type has no surface spelling until its construction words
    /// land.
    #[test]
    fn slice_output_is_rejected_and_slice_input_is_admitted() {
        let mut slices = Vec::new();
        let slice = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let out = StackEffect {
            inputs: Vec::new(),
            outputs: vec![TypedSlot {
                name: None,
                ty: slice,
            }],
        };
        let err = check_reference_free_signature("mk", &out, &[], &[], &[]).unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `mk` declares the output `Slice[i64]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
        let inp = StackEffect {
            inputs: vec![TypedSlot {
                name: None,
                ty: slice,
            }],
            outputs: vec![TypedSlot {
                name: None,
                ty: Type::I64,
            }],
        };
        check_reference_free_signature("sum", &inp, &[], &[], &[])
            .expect("a slice *input* is legal: it borrows the caller's storage");
    }

    #[test]
    fn check_term_word_with_entry_locals_still_ok() {
        // Regression: a plain term word with `| ... |` entry locals is
        // unaffected by the clause-body path (no enum in scope).
        check_src(": sq ( i64 -- i64 ) | n | n n mul ;").unwrap();
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

    /// Slice 10c (E-P3-7): **retargeted, not deleted.** Slice 11's rejection
    /// of an `inline` polymorphic signature was a *policy* rule by its own
    /// admission -- the splice already handles a variable-bearing body -- and
    /// R-P3-3b deliberately reverses it, because the six comparison words must
    /// be both `'T: Copy Ord`-polymorphic (to keep covering the numeric tower)
    /// and `inline` (or every comparison becomes a real call with a frame).
    /// The other half of the original pair, a `~`-bearing but variable-free
    /// effect, is unaffected and stays.
    ///
    /// The witness is a word named `eq`, not a neutral name: a neutral name is
    /// claimed by no builtin, so it slips past the *second* (soundness)
    /// `inline` gate, `BUILTIN_TABLE.contains_key`, and would pass whether or
    /// not the real comparison words can ever be `inline`. Restoring the
    /// polymorphic gate rejects with `requires a monomorphic effect`; leaving
    /// the six rows in `BUILTIN_TABLE` under their old names rejects with
    /// `overlaps a concrete overload of `eq``.
    #[test]
    fn check_inline_polymorphic_signature_is_accepted() {
        check_src(": id inline ( 'T -- 'T ) ;\n: main ( -- ) ;")
            .expect("`inline` on a polymorphic signature is a splice, not a rejection");
        // The witness is a word named `eq`, declared here as `core::cmp`
        // declares it. A neutral name would not exercise the builtin-name gate
        // at all. P8 S2 (R3): this is declared in the test source rather than
        // pulled out of the deleted prelude -- with nothing injected, `eq` is
        // an ordinary name a source may define.
        const EQ: &str =
            ": eq inline ( 'T: Copy Ord 'T -- bool ) ueq [ true ] [ false ] branch ;\n";
        let tokens = lex(EQ).unwrap();
        let eq = crate::test_support::parse_with_core(&tokens)
            .unwrap()
            .words
            .into_iter()
            .find(|w| w.name == "eq")
            .expect("the witness source declares `eq`");
        assert!(eq.declares_inline, "`eq` is declared `inline`");
        let sig = eq.poly.as_ref().expect("`eq` is polymorphic");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        check_inline_declaration(&eq)
            .expect("the real witness: a builtin-operator-named polymorphic `inline` word");
        check_src(": main ( -- ) 1 2 eq drop 1 >u32 2 >u32 eq drop ;")
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
        check_src("type: P n u32 ;\n: pick inline ( &!P -- &!u32 ) | p | p &!n ;\n")
            .expect("an `inline` word may declare a reference output");
        check_src(
            "type: P n u32 ;\n: pick inline ( &!P ~[ -- ] -- &!u32 ) | p f | f call p &!n ;\n",
        )
        .expect("a quotation-taking word is exempt too (the skip reads `is_combinator`)");
        // A *poly* combinator takes the same exemption by the same guard: it
        // reaches `check_word` through the concrete stand-in
        // `check_poly_combinator_standalone` builds, which carries the quotation
        // parameter (and the flag) across and so is itself `is_combinator`.
        check_src(
            "type: P n u32 ;\n: pick inline ( 'T &!P ~[ 'T -- ] -- &!u32 ) | v p f | v f call p &!n ;\n",
        )
        .expect("a poly combinator is exempt through its concrete stand-in");
        let err =
            check_src("type: P n u32 ;\n: pick ( &!P -- &!u32 ) | p | p &!n ;\n").unwrap_err();
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
    /// operator name per module (`add__m0`), so a raw comparison never matches.
    #[test]
    fn check_inline_builtin_operator_overload_is_error() {
        let err = check_src(
            "type: A n i64 ;\n\
             : add inline ( A A -- i64 ) | x y | &x &n @ drop &y &n @ drop 1000 ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: `inline` on `add`, which overloads a builtin operator name; a call site of a builtin operator name dispatches through a real call and cannot be spliced (line 2, col 3)"
        );
        // A non-operator name with the identical shape is accepted, so the
        // rejection is keyed on the name, not on the overload.
        check_src(
            "type: A n i64 ;\n\
             : bump inline ( A A -- i64 ) | x y | &x &n @ drop &y &n @ drop 1000 ;\n",
        )
        .expect("an `inline` word whose name no operator claims is accepted");
    }
}
