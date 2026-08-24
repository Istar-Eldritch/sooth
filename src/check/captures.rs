use super::*;
use crate::ast::OWNING_QUOTATION_KEYWORD;

/// R6 (Q1): does the quotation `body` read any name in `enclosing` that the
/// body does not itself bind? The cheap boolean the D3 materialization line
/// needs (no captures / captures), strictly less work than 7b's capture *set*.
/// Mirrors `alpha_rename_locals`'s walk (ast.rs): a `Call` strips a leading
/// `&!`/`&` exactly as `rename_call`, and a nested `TermKind::Quotation` / `if`
/// arm is walked carrying the body-bound names *by value*, so a read of an
/// outer name from inside a nested quotation still counts (D4's
/// capture-into-another-quotation case). Pure over the term tree: it inspects
/// no `Slot`/`Deriv` state, so it is testable in isolation.
pub(super) fn body_captures_enclosing(body: &[Term], enclosing: &HashSet<String>) -> bool {
    fn walk(terms: &[Term], enclosing: &HashSet<String>, bound: &mut Vec<String>) -> bool {
        for term in terms {
            match &term.kind {
                TermKind::Bind(names) => {
                    for n in names {
                        bound.push(n.clone());
                    }
                }
                TermKind::Call(name) => {
                    let stripped = name
                        .strip_prefix("&!")
                        .or_else(|| name.strip_prefix('&'))
                        .unwrap_or(name);
                    if enclosing.contains(stripped) && !bound.iter().any(|b| b == stripped) {
                        return true;
                    }
                }
                TermKind::Quotation(inner, _, _) => {
                    let mut inner_bound = bound.clone();
                    if walk(inner, enclosing, &mut inner_bound) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(body, enclosing, &mut Vec::new())
}

/// R24: an escaping closure captures a local of the *current* frame whose
/// storage dies at return. Wording modeled on `reference_across_back_edge_error`
/// (`:5501`): the same "a local of this frame ... does not survive" shape, one
/// frame instead of one loop iteration. Fires for `make-a` (R15) and, in
/// Phase 2, for a frame capture escaping through a returned carrier (R22).
///
/// P7.S3h: a *linear* capture gets the remedy named. `owning [ ... ]` is a
/// boundary that owns its captures and disposes them by running the body, so
/// it is exactly the fix here -- and it is not the fix for a `Copy` aggregate
/// or a borrow, whose problem is a pointer into dead frame storage that no
/// disposal obligation addresses, so those keep the bare message.
pub(super) fn past_owning_frame_error(ctx: &Ctx, span: Span, name: &str, linear: bool) -> String {
    let _ = ctx;
    let remedy = match linear {
        true => format!(
            "\n  declare the boundary `{OWNING_QUOTATION_KEYWORD} [ ... ]` to hand the closure ownership of `{name}`, so calling it disposes `{name}`"
        ),
        false => String::new(),
    };
    format!(
        "error: an escaping closure captures `{name}`, a local of this frame, whose storage does not survive the return (line {}){remedy}",
        span.line,
    )
}

/// R24: a captured reference is read after its referent's last use -- the
/// referent is consumed or exclusively re-borrowed while an erased closure
/// still holds a borrow of it (kept live past the store by R20's surviving-set
/// union). Fires only for a capture the surviving set added; a still-`Known`
/// closure or a genuinely-live borrow keeps `conflicting_borrow_error`.
pub(super) fn past_last_use_error(ctx: &Ctx, span: Span, name: &str) -> String {
    let _ = ctx;
    format!(
        "error: a captured reference to `{name}` is read after its last use (line {})",
        span.line,
    )
}

/// R18: an escaping closure captures more than one value. Phase 1 stores a
/// single word-sized capture inline in the `env` slot; a 2+-capture escaping
/// closure needs a heap env, deferred with the rest of the heap-env case.
/// Review fix: also fires at R22 when a 2+-capture closure's stack-allocated
/// env bundle (R16) escapes transitively through a returned carrier -- the
/// bundle's storage dies at return exactly as the direct-return case would.
pub(super) fn multi_capture_escaping_error(ctx: &Ctx, span: Span) -> String {
    let _ = ctx;
    format!(
        "error: an escaping closure may capture at most one reference (a heap env is deferred) (line {})",
        span.line,
    )
}

/// R15 case 4: a captured quotation-typed name. Admitting it would need a
/// two-word `(code, env)` env slot and a recursive surviving-set fold no exit
/// criterion requires, so it is deferred, parallel to the 2+-capture deferral.
pub(super) fn captured_quotation_name_deferred_error(ctx: &Ctx, span: Span) -> String {
    let _ = ctx;
    format!(
        "error: capturing a quotation value by name is deferred (line {})",
        span.line,
    )
}

/// Slice 10a (R2): the fifth materialization boundary. Distinct wording from
/// `captured_quotation_name_deferred_error` -- an ordinary quotation capture
/// is *deferred* (unimplemented, might land later); a `~` capture is *banned*
/// (materialization is exactly what `~` forbids, permanently).
fn captured_inline_quotation_error(ctx: &Ctx, span: Span) -> String {
    let where_ = ctx.rendered_word_or("`<line>`");
    format!(
        "error: a `~` quotation cannot be captured in {where_} (line {})",
        span.line,
    )
}

/// R15: how a captured name's referent is rooted, which decides whether it may
/// outlive the closure's calls.
pub(super) enum CaptureClass {
    /// A scalar local: snapshotted into the env (D4 amendment), so it can never
    /// dangle and is admissible at every boundary.
    Scalar,
    /// An aggregate value or borrow whose referent lives in the current frame:
    /// its storage dies at return.
    FrameRooted,
    /// An aggregate value or borrow rooted in a parameter or global: its
    /// referent outlives the frame.
    OuterRooted,
}

/// Whether a reference's root names a *current-frame* local -- the same
/// frame-vs-outer test `classify_capture`'s `Type::Ref` arm applies, reused so
/// a `!`/`+!` store through a reference computes its own escaping boundary
/// the same way (a store through a reference rooted outside the frame is a
/// materialization boundary the closure can outlive, exactly like a return).
pub(super) fn ref_root_is_in_frame(
    deriv: Option<DerivId>,
    prov: &Provenance,
    scope: &Scope,
) -> bool {
    match deriv {
        Some(id) => match &prov.deriv(id).owned_root {
            Some(place) => scope.local(place).is_some(),
            None => false,
        },
        None => false,
    }
}

/// P7.S3h: is a captured aggregate-arm type *scalar-represented*, i.e. held in
/// the one-word env slot as a value rather than as a pointer into frame
/// storage? A struct and an array are unconditionally pointer-backed
/// (`is_aggregate`, `ir/func_builder/mod.rs`), and an owned cell is a pointer
/// to a heap block whose ownership cannot be snapshotted, so this is true for
/// exactly one shape: an enum none of whose variants carries a payload.
///
/// Computed from the *declaration*, not from `ir::layout`, following slice
/// 10c's `tag` domain check (`word_families.rs`) and for the same reason: the
/// admission this gates must be decidable before any lowering runs.
///
/// No `is_copy` conjunct, deliberately: `is_copy`'s enum arm folds over the
/// variant fields, so a payload-free enum is `Copy` by construction and the
/// conjunct could never change an answer.
fn capture_is_scalar_represented(ty: Type, enums: &[EnumDecl]) -> bool {
    match ty {
        Type::Enum(id, _) => enums[id.index()]
            .variants
            .iter()
            .all(|v| v.fields.is_empty()),
        _ => false,
    }
}

/// R15: classify a captured name by its binding (case 4 quotation-typed names
/// are peeled off before this runs). The frame-rooted/outer-rooted split reuses
/// the exact `owned_root`-vs-current-frame test the R12 exit-row check
/// (`:6108`) and `check_reference_across_back_edge` (`:5519`) already apply.
pub(super) fn classify_capture(
    b: &Binding,
    prov: &Provenance,
    scope: &Scope,
    enums: &[EnumDecl],
) -> CaptureClass {
    match b.ty {
        // P7.S3h: a scalar-represented member of the aggregate arm below is a
        // *value* in the one-word env slot, not a pointer into frame storage,
        // so it can no more dangle than an `i64` can and takes case 1. Only a
        // payload-free enum qualifies -- `bool` since S3i, the motivating case:
        // the blanket arm below rejected it at every escaping boundary purely
        // for being spelled as an enum.
        Type::Enum(..) if capture_is_scalar_represented(b.ty, enums) => CaptureClass::Scalar,
        // Case 2: an aggregate value read directly (no deriv). A scope-bound
        // aggregate is owned by (and dies with) this frame; a global aggregate
        // is not in scope and never reaches here.
        //
        // Review note (deliberate narrowing, not a bug): R15 case 2 also names
        // a by-value aggregate PARAMETER or global as outer-rooted (its
        // storage belongs to the caller, not this frame), which this arm does
        // not distinguish from a locally-constructed aggregate -- both reach
        // here with `deriv: None`, and telling them apart would need a new
        // provenance tag threaded from `check_terms_word`'s initial stack
        // through every bind/shuffle (the same weight as `Deriv` tracking for
        // `Type::Ref`, case 3's own mechanism). Unbuilt: this arm always
        // returns `FrameRooted`, so an aggregate parameter capture is
        // over-rejected at an escaping boundary rather than admitted -- sound
        // (it never under-rejects), just more conservative than case 2's full
        // rule. See `docs/roadmap/P4/slice7b-spec.md`'s R15 section.
        Type::Struct(..) | Type::Enum(..) | Type::Array(..) | Type::OwnedCell(..) => {
            CaptureClass::FrameRooted
        }
        // Case 3: a borrow. A `Deriv` whose `owned_root` names a current-frame
        // local is frame-rooted; a `&T` parameter carries no deriv (or a
        // reborrow with no owned root) and is outer-rooted by construction,
        // matching `check_reference_across_back_edge`'s own `deriv` handling.
        //
        // P7 slice 3c (R8.1): a slice is a borrow too, and takes this arm
        // rather than the scalar wildcard below -- a captured view analysed as
        // a scalar would be snapshotted into the env and admitted at every
        // materialization boundary, including one it can outlive.
        Type::Ref(..) | Type::Slice(..) => {
            if ref_root_is_in_frame(b.deriv, prov, scope) {
                CaptureClass::FrameRooted
            } else {
                CaptureClass::OuterRooted
            }
        }
        // Case 1: any other local is a scalar.
        _ => CaptureClass::Scalar,
    }
}

/// R15: admit or reject a capturing quotation at a materialization boundary
/// (7b, replacing 7a's blanket R12 rejection). `escaping` is true at a
/// word-output boundary (`be returned`, or a differing-arm `if` join feeding
/// the declared output), false at an in-frame store. A four-way classification
/// on capture kind (D3's cached free-name set is the source, no new analysis):
/// a scalar snapshot admits everywhere; an outer-rooted aggregate/borrow admits
/// everywhere; a frame-rooted one is past-owning-frame at a word-output
/// boundary (R24) and admitted at an in-frame one (R21, Phase 2); a captured
/// quotation-typed name is deferred everywhere (case 4). An escaping closure's
/// inline env holds one word, so a 2+-capture escaping closure is deferred
/// (R18); an in-frame one takes a stack bundle (R16), so 2+ is admitted.
///
/// Returns the interned surviving capture set (R19): the admitted
/// aggregate/borrow captures whose referents must outlive the closure's calls,
/// each tagged frame-rooted (for the R22 escape guard). A scalar-only closure
/// returns `None` -- a snapshot has no referent that can go dead.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_capture_admission(
    id: QuotId,
    escaping: bool,
    owning: bool,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &mut Provenance,
    scope: &Scope,
) -> Result<Option<SurvivingCaptureSetId>, String> {
    let enums = ctx.enums();
    // Only names bound in the enclosing scope are real captures; a free global
    // word resolves at the call and needs no env.
    let mut names: Vec<String> = prov
        .quotation_captures(id)
        .iter()
        .filter(|n| scope.local(n).is_some())
        .cloned()
        .collect();
    names.sort_unstable();
    if names.is_empty() {
        return Ok(None);
    }
    // Case 4: a captured quotation-typed name (a `Known` literal, `quot.is_some()`,
    // or an already-erased `Type::Quotation`) is deferred at every boundary.
    for name in &names {
        let b = scope.local(name).expect("filtered to a bound name");
        // Slice 10a (R2): the fifth materialization boundary. A `~` local is
        // exactly a quotation-typed binding; without the accessor it passes
        // this guard, is recorded in a surviving capture set, and lowering
        // materializes it into an env bundle -- the one thing `~` forbids.
        // Phase 2 replaces the message with a `~`-specific one plus a golden.
        if matches!(b.ty, Type::InlineQuotation(_)) {
            return Err(captured_inline_quotation_error(ctx, span));
        }
        if b.quot.is_some() || crate::ast::is_quotation_type(b.ty).is_some() {
            return Err(captured_quotation_name_deferred_error(ctx, span));
        }
    }
    // Classify each capture and, for an aggregate/borrow one, record it as a
    // surviving-set member (R19). A scalar snapshot is never a member (D4
    // amendment); a frame-rooted capture escaping a word-output boundary is
    // rejected here (R24) before any set is built (make-a).
    let mut members: Vec<SurvivingCapture> = Vec::new();
    for name in &names {
        let b = scope.local(name).expect("filtered to a bound name");
        match classify_capture(b, prov, scope, enums) {
            CaptureClass::Scalar => {}
            CaptureClass::FrameRooted => {
                // `arrays` is threaded in for exactly this: `is_linear`
                // resolves an array element through the registry, and `Ctx`
                // exposes only `structs`/`enums`.
                let linear = is_linear(b.ty, ctx.structs(), enums, arrays);
                // P7.S3h: at an `owning` boundary a *linear* capture is moved
                // into the closure's heap env, which outlives the frame, so
                // the frame's storage dying at return is no longer the
                // question -- and the frame no longer owns the value either,
                // so it is not a surviving-set member. The narrowing to linear
                // is deliberate: a `Copy` aggregate is not moved, so admitting
                // one here would leave the frame and the env each holding a
                // copy with no rule saying which is authoritative.
                if owning && linear {
                    continue;
                }
                if escaping {
                    return Err(past_owning_frame_error(ctx, span, name, linear));
                }
                members.push(SurvivingCapture {
                    name: name.clone(),
                    frame_rooted: true,
                });
            }
            CaptureClass::OuterRooted => members.push(SurvivingCapture {
                name: name.clone(),
                frame_rooted: false,
            }),
        }
    }
    // R18: an escaping closure's inline env holds one word; a 2+-capture
    // escaping closure needs a heap env, deferred. An in-frame one takes the
    // stack bundle (R16), so any count is admitted here -- but review fix:
    // the bundle marker rides onto the interned set regardless, since the
    // in-frame admission is only sound until this closure escapes through a
    // later carrier, which the R22 guard checks at the word-output boundary.
    // P7.S3h: an owning closure's env is a heap block, never the stack bundle
    // this marker names, so both the 2+-capture deferral and the R22
    // transitive-escape guard it feeds are inapplicable to it.
    let bundle = names.len() >= 2 && !owning;
    if escaping && bundle {
        return Err(multi_capture_escaping_error(ctx, span));
    }
    Ok(prov.intern_surviving_set(members, bundle))
}

/// R7/R15/D4: a materialization boundary. Materialize a non-capturing `Known`
/// literal into a runtime quotation value, or run the R15 admission rule on a
/// capturing one. (i) run the boolean capture gate (R6); (ii) if it captures,
/// admit or reject per R15 against the caller-computed `escaping` (true at a
/// word-output/branch-join boundary the closure cannot outlive its call from,
/// or at a store through a reference rooted outside the current frame; false
/// at a genuinely in-frame boundary); (iii) confirm the literal against the
/// boundary's expected `Type::Quotation(eff)` via
/// `check_literal_against_declared_effect`, and return the slot *erased*
/// (`quot: None`, a real `Type::Quotation`) -- the signal `call`/`times` read
/// to emit an indirect call rather than a splice.
#[allow(clippy::too_many_arguments)]
pub(super) fn materialize_quotation_at_boundary(
    id: QuotId,
    eff: &'static QuotEffect,
    owning: bool,
    escaping: bool,
    word: &str,
    span: Span,
    ctx: &Ctx,
    env: &HashMap<String, Vec<Overload>>,
    arrays: &mut Vec<ArrayDecl>,
    cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    prov: &mut Provenance,
    scope: &mut Scope,
    poly: &mut PolyCtx,
) -> Result<Slot, String> {
    let enclosing: HashSet<String> = scope.bound.iter().map(|b| b.name.clone()).collect();
    let body = prov.quotations[id.0].body.clone();
    let captures = body_captures_enclosing(&body, &enclosing);
    let surviving = if captures {
        check_capture_admission(id, escaping, owning, span, ctx, arrays, prov, scope)?
    } else {
        None
    };
    // Slice 10a (R9): an `eff` reaching an erasure boundary is a `QuotEffect`
    // with no row, so the row grounds to the empty region.
    check_literal_against_declared_effect(
        id,
        eff,
        false,
        &[],
        word,
        span,
        ctx,
        env,
        arrays,
        cells,
        refs,
        slices,
        prov,
        scope,
        poly,
        &HashSet::new(),
        LiteralBoundary {
            shape_changing: false,
            is_arm: false,
            caller_tail: false,
            finalize: false,
            owning,
        },
        None,
    )?;
    // P7.S3h: an owning literal *must* consume every linear value it captures.
    // The boundary lifted the R12 blanket ban on consuming an enclosing linear
    // local (that consumption is the ownership transfer), which leaves the
    // opposite hazard: a capture the body merely reads leaves the frame still
    // owning the value while the env holds its bytes, so it would be disposed
    // twice. Checked after the body walk, since the move state is what answers
    // it.
    if owning && captures {
        check_owning_captures_consumed(id, span, ctx, arrays, prov, scope)?;
    }
    // R7 (P7 slice 1 phase 6): a declared quotation effect returning a
    // reference used to reach `lower_reference_word`'s `referent_of` and
    // panic ("checked: every reference value records its referent") instead
    // of being rejected here, where the declared shape is already known.
    // Second-class references are a DESIGN.md invariant; this is the
    // quotation-effect twin of `check_reference_free_signature`
    // (`word_entry.rs`), reusing its wording. Runs after the capture checks
    // above (D3's own borrow-of-enclosing-place rejection, inside
    // `check_literal_against_declared_effect`, is the more specific
    // diagnostic when the returned reference is also a captured borrow).
    check_quotation_reference_free_effect(eff, ctx, span, arrays)?;
    // R19: the erased slot carries the surviving capture set in place of the
    // dropped `Known` marker, the signal `capture_alive_names` (R20) and the
    // R22 escape guard read once the identity is gone.
    let ty = match owning {
        true => Type::OwningQuotation(eff),
        false => Type::Quotation(eff),
    };
    Ok(Slot {
        surviving,
        ..Slot::computed(ty)
    })
}

/// P7.S3h: every linear local an owning literal captured must have been
/// consumed by the literal's body, so ownership genuinely moved into the env
/// rather than being duplicated. `MaybeMoved` (consumed on one arm of an `if`
/// inside the body) fails for the same reason it fails everywhere else: it is
/// neither a completed move nor a live value.
pub(super) fn check_owning_captures_consumed(
    id: QuotId,
    span: Span,
    ctx: &Ctx,
    arrays: &[ArrayDecl],
    prov: &Provenance,
    scope: &Scope,
) -> Result<(), String> {
    let mut names: Vec<&String> = prov
        .quotation_captures(id)
        .iter()
        .filter(|n| scope.local(n).is_some())
        .collect();
    names.sort_unstable();
    for name in names {
        let b = scope.local(name).expect("filtered to a bound name");
        if !is_linear(b.ty, ctx.structs(), ctx.enums(), arrays) {
            continue;
        }
        if !matches!(scope.moves.states.get(name), Some(MoveState::Moved(_))) {
            return Err(owning_capture_not_consumed_error(ctx, span, name, b.ty));
        }
    }
    Ok(())
}

/// P7.S3h: an owning closure captured a linear local without consuming it.
fn owning_capture_not_consumed_error(ctx: &Ctx, span: Span, name: &str, ty: Type) -> String {
    format!(
        "error: an `{OWNING_QUOTATION_KEYWORD}` closure captures `{name}`, a linear `{ty}`, without consuming it{} (line {}): an owning closure takes ownership of what it captures, so its body must dispose or return each linear capture exactly once",
        in_word(ctx),
        span.line,
    )
}

/// R7: the quotation-effect twin of `check_reference_free_signature`
/// (`word_entry.rs`) -- a materialized quotation is itself a callable with
/// its own frame, so a declared output that transitively contains a
/// reference borrows a local of that frame, gone by the time whatever calls
/// the quotation value reads it, reusing `stored_reference_output_error`'s
/// wording (`builtins.rs`). Unlike the word-level check, there is no input
/// arm: an *aggregate* input carrying a nested reference is already rejected
/// at its struct/array declaration (a field or element typed `&T` is a
/// located error there), which is the only shape this literal boundary sees.
/// A declared quotation *parameter* whose own effect returns a reference
/// (`( &!Sprite [ &!Sprite -- &!i64 ] -- i64 )`) never reaches a
/// materialization boundary and still panics in `referent_of`: an indirect
/// `call` produces a reference with no recorded referent, so closing it needs
/// a consumer-side rule, out of scope here.
fn check_quotation_reference_free_effect(
    eff: &QuotEffect,
    ctx: &Ctx,
    span: Span,
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    for ty in &eff.outputs {
        if contains_reference(*ty, ctx.structs(), ctx.enums(), arrays) {
            let location = format!("{} (line {})", in_word(ctx), span.line);
            return Err(stored_reference_output_error(
                eff.name_static,
                *ty,
                &location,
            ));
        }
    }
    Ok(())
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

    #[test]
    fn quotation_effect_returning_a_reference_is_a_located_error() {
        // R7: a materialized quotation whose declared effect returns a
        // reference used to reach `referent_of`'s `.expect("checked: every
        // reference value records its referent")` and panic. It must instead
        // be a located check-time error, reusing `stored_reference_output_error`'s
        // wording -- and it must never reach `lower`, so this only calls `check`.
        let err = check_src(
            "type: Sprite hp i64 ;\n: lens-hp ( -- [ &!Sprite -- &!i64 ] ) [ &!hp ] ;\n: main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("error: a reference cannot be stored")
                && err.contains("&!i64")
                && err.contains("in `lens-hp`")
                && err.contains("line 2"),
            "unexpected message: {err}"
        );
    }

    fn bare_word(name: &str, module: u32) -> WordDef {
        WordDef {
            name: name.to_string(),
            effect: StackEffect::default(),
            body: Vec::new(),
            poly: None,
            declares_inline: false,
            module,
            span: Span::default(),
            declared_globals: None,
        }
    }
    fn capture_binding(name: &str, ty: Type, deriv: Option<DerivId>) -> Binding {
        Binding {
            name: name.to_string(),
            ty,
            aliases: None,
            deriv,
            quot: None,
            surviving: None,
        }
    }
    #[test]
    fn classify_capture_splits_scalar_aggregate_and_borrow_roots() {
        // U-classify (R15): the four-way capture classifier, each arm on its
        // own, since only case 2 and one case-3 direction are reachable from a
        // golden (make-a hits the aggregate arm, the case-3 golden the
        // frame-rooted borrow; the outer-rooted and no-deriv arms ride only on
        // make-b end to end).
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let mut refs: Vec<RefDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let ref_ty = intern_ref_type(&mut refs, arr_ty, false);
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };

        // Case 1: a scalar local -> Scalar (snapshotted, never dangles).
        let prov = Provenance::default();
        let empty = Scope::default();
        let scalar = capture_binding("x", Type::I64, None);
        assert!(matches!(
            classify_capture(&scalar, &prov, &empty, &[]),
            CaptureClass::Scalar
        ));

        // Case 2: a by-value aggregate -> FrameRooted (owned by, dies with,
        // this frame).
        let agg = capture_binding("arr", arr_ty, None);
        assert!(matches!(
            classify_capture(&agg, &prov, &empty, &[]),
            CaptureClass::FrameRooted
        ));

        // Case 3a: a borrow whose `owned_root` names a current-frame local ->
        // FrameRooted.
        let mut prov = Provenance::default();
        let d = prov.borrow("arr", false, false, span);
        let mut framed = Scope::default();
        framed.bound.push(capture_binding("arr", arr_ty, None));
        let borrow_local = capture_binding("r", ref_ty, Some(d));
        assert!(matches!(
            classify_capture(&borrow_local, &prov, &framed, &[]),
            CaptureClass::FrameRooted
        ));

        // Case 3b: the same borrow, but its `owned_root` is not in this scope
        // (rooted in an ancestor frame) -> OuterRooted.
        assert!(matches!(
            classify_capture(&borrow_local, &prov, &empty, &[]),
            CaptureClass::OuterRooted
        ));

        // Case 3c: a `&T` parameter reborrow carrying no owned root at all ->
        // OuterRooted by construction, without consulting the scope.
        let mut prov = Provenance::default();
        let d = prov.add(Deriv {
            place: "p".to_string(),
            owned_root: None,
            static_root: false,
            reborrow: true,
            mutable: false,
            projected: false,
            span,
        });
        let param_ref = capture_binding("r", ref_ty, Some(d));
        assert!(matches!(
            classify_capture(&param_ref, &prov, &empty, &[]),
            CaptureClass::OuterRooted
        ));
    }

    /// P7 slice 3c (R8.1): a captured slice takes the borrow arm, not the
    /// scalar wildcard. Both roots are exercised, since "not `Scalar`" alone
    /// would pass on a single arm answering `FrameRooted` unconditionally.
    #[test]
    fn classify_capture_slice_is_reference_not_scalar() {
        let mut arrays = Vec::new();
        let mut slices = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let slice_ty = crate::ast::intern_slice_type(&mut slices, Type::I64, true);
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };

        // Rooted in a current-frame local: the view dies with the storage it
        // views, so it may not escape.
        let mut prov = Provenance::default();
        let d = prov.borrow("arr", true, false, span);
        let mut framed = Scope::default();
        framed.bound.push(capture_binding("arr", arr_ty, None));
        let framed_view = capture_binding("s", slice_ty, Some(d));
        assert!(matches!(
            classify_capture(&framed_view, &prov, &framed, &[]),
            CaptureClass::FrameRooted
        ));

        // The same view whose root is not this frame's: outer-rooted, so it
        // outlives the closure's calls.
        assert!(matches!(
            classify_capture(&framed_view, &prov, &Scope::default(), &[]),
            CaptureClass::OuterRooted
        ));
    }
    /// P7.S3h: the aggregate arm splits on scalar representation, not on the
    /// four type constructors. A payload-free enum is a value in the one-word
    /// env slot, so it takes case 1; a payload-carrying one is tagged storage
    /// reached by pointer and keeps the frame-rooted rejection. Both
    /// directions are pinned: an arm answering `Scalar` for every enum would
    /// pass on the first assertion alone.
    #[test]
    fn classify_capture_payload_free_enum_is_scalar_not_frame_rooted() {
        let variant = |name: &'static str, fields: Vec<(String, Type)>| VariantDecl {
            name: name.to_string(),
            name_static: name,
            display_static: name,
            fields,
            span: Span::default(),
        };
        let enums = vec![
            EnumDecl {
                name: "Bool".to_string(),
                name_static: "Bool",
                variants: vec![variant("False", vec![]), variant("True", vec![])],
                span: Span::default(),
                module: 0,
            },
            EnumDecl {
                name: "Item".to_string(),
                name_static: "Item",
                variants: vec![
                    variant("Empty", vec![]),
                    variant("Full", vec![("v".to_string(), Type::I64)]),
                ],
                span: Span::default(),
                module: 0,
            },
        ];
        let prov = Provenance::default();
        let scope = Scope::default();

        let payload_free = capture_binding("b", Type::Enum(EnumId::from_index(0), "Bool"), None);
        assert!(matches!(
            classify_capture(&payload_free, &prov, &scope, &enums),
            CaptureClass::Scalar
        ));

        // `Item` is `Copy` (its one payload field is an `i64`), so `is_copy`
        // alone would admit it -- the scalar-representation half is the whole
        // guard here.
        let payload = capture_binding("e", Type::Enum(EnumId::from_index(1), "Item"), None);
        assert!(matches!(
            classify_capture(&payload, &prov, &scope, &enums),
            CaptureClass::FrameRooted
        ));
    }

    /// P7.S3h: the pointer-backed members of the aggregate arm are unmoved by
    /// the enum narrowing, and a `Copy` one is the case that matters -- the
    /// env slot holds one word, and a struct's or an array's value is a
    /// pointer into frame storage, so snapshotting one at an escaping
    /// boundary would be a use-after-return however `Copy` it is.
    #[test]
    fn classify_capture_copy_aggregate_stays_frame_rooted() {
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let structs = vec![StructDecl {
            name: "P".to_string(),
            name_static: "P",
            fields: vec![("x".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let struct_ty = Type::Struct(StructId::from_index(0), "P");
        assert!(is_copy(arr_ty, &structs, &[], &arrays));
        assert!(is_copy(struct_ty, &structs, &[], &arrays));

        let prov = Provenance::default();
        let scope = Scope::default();
        for ty in [arr_ty, struct_ty] {
            assert!(matches!(
                classify_capture(&capture_binding("a", ty, None), &prov, &scope, &[]),
                CaptureClass::FrameRooted
            ));
        }
    }

    #[test]
    fn check_capture_admission_gates_each_capture_kind() {
        // U-admit (R15): the admission gate around `classify_capture`. Each
        // deferral/rejection is its own row, since a dropped guard is a silent
        // accept the well-typed goldens never trip.
        fn prov_with(names: &[&str]) -> Provenance {
            let mut prov = Provenance::default();
            prov.quotation_captures
                .push(names.iter().map(|s| s.to_string()).collect());
            prov
        }
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let mut arrays: Vec<ArrayDecl> = Vec::new();
        let arr_ty = intern_array_type(&mut arrays, Type::I64, 4);
        let admit = |prov: &mut Provenance, escaping, scope: &Scope| {
            check_capture_admission(QuotId(0), escaping, false, span, &ctx, &arrays, prov, scope)
        };

        // No real capture: an empty set, and a free global not in scope, both
        // admit (a free word resolves at the call and needs no env).
        assert!(admit(&mut prov_with(&[]), true, &Scope::default()).is_ok());
        assert!(admit(&mut prov_with(&["some-word"]), true, &Scope::default()).is_ok());

        // A single scalar capture admits at every boundary and, being a
        // snapshot, contributes no surviving-set member (R19 / D4 amendment).
        let mut scope = Scope::default();
        scope.bound.push(capture_binding("x", Type::I64, None));
        assert_eq!(admit(&mut prov_with(&["x"]), true, &scope), Ok(None));

        // A frame-rooted aggregate is past-owning-frame when escaping (R24),
        // and admitted in-frame (R21) with a frame-rooted surviving member.
        let mut scope = Scope::default();
        scope.bound.push(capture_binding("arr", arr_ty, None));
        let escaping = admit(&mut prov_with(&["arr"]), true, &scope).unwrap_err();
        assert!(
            escaping.contains("`arr`") && escaping.contains("does not survive the return"),
            "escaping frame-rooted capture is past-owning-frame: {escaping}"
        );
        let mut prov = prov_with(&["arr"]);
        let set = admit(&mut prov, false, &scope)
            .expect("in-frame frame capture admits (R21)")
            .expect("an aggregate capture is a surviving-set member");
        let members = prov.surviving_set(set);
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "arr");
        assert!(
            members[0].frame_rooted,
            "an in-frame aggregate is frame-rooted"
        );

        // Case 4: a quotation-typed name is deferred at every boundary.
        let mut scope = Scope::default();
        scope.bound.push(Binding {
            name: "q".to_string(),
            ty: Type::I64,
            aliases: None,
            deriv: None,
            quot: Some(QuotRef::Known(QuotId(0))),
            surviving: None,
        });
        let quot_name = admit(&mut prov_with(&["q"]), true, &scope).unwrap_err();
        assert!(
            quot_name.contains("capturing a quotation value by name is deferred"),
            "a captured quotation-typed name is deferred: {quot_name}"
        );

        // Two scalar captures escaping need a heap env, deferred (R18); the
        // same two admit in-frame as a stack bundle (R16, R21).
        let mut scope = Scope::default();
        scope.bound.push(capture_binding("x", Type::I64, None));
        scope.bound.push(capture_binding("y", Type::I64, None));
        let multi = admit(&mut prov_with(&["x", "y"]), true, &scope).unwrap_err();
        assert!(
            multi.contains("at most one reference"),
            "a 2+-capture escaping closure is deferred: {multi}"
        );
        // In-frame, the same two admit -- but as a stack bundle (R16), so the
        // interned set has no members yet must survive with `bundle = true`,
        // else the bundle-escape-via-carrier signal is lost before R22.
        let mut prov = prov_with(&["x", "y"]);
        let set = admit(&mut prov, false, &scope)
            .expect("two scalar captures admit in-frame (R21)")
            .expect("an all-scalar stack bundle keeps a set to carry the bundle signal");
        assert!(
            prov.surviving_set(set).is_empty(),
            "a scalar snapshot is never a surviving member (D4)"
        );
        assert!(
            prov.surviving_set_is_bundle(set),
            "the all-scalar 2-capture stack bundle marks its set as a bundle"
        );
    }
    /// P7.S3h: the remedy line is conditioned on linearity rather than bolted
    /// onto every past-owning-frame rejection. `owning [ ... ]` is the fix for
    /// a *linear* capture, because an owning closure takes ownership and
    /// disposes it by running; it fixes nothing for a `Copy` aggregate, whose
    /// problem is a pointer into dead frame storage that no disposal obligation
    /// addresses. Both directions, since dropping the condition would put a
    /// wrong remedy on the `Copy` case.
    #[test]
    fn past_owning_frame_names_owning_only_for_a_linear_capture() {
        let structs = vec![
            StructDecl {
                name: "Spy".to_string(),
                name_static: "Spy",
                fields: vec![("tag".to_string(), Type::I64)],
                span: Span::default(),
                has_drop_overload: true,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "P".to_string(),
                name_static: "P",
                fields: vec![("x".to_string(), Type::I64)],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
        ];
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let cases = [
            (Type::Struct(StructId::from_index(0), "Spy"), true),
            (Type::Struct(StructId::from_index(1), "P"), false),
        ];
        for (ty, wants_remedy) in cases {
            assert_eq!(is_linear(ty, &structs, &enums, &[]), wants_remedy);
            let mut prov = Provenance::default();
            prov.quotation_captures
                .push(std::iter::once("v".to_string()).collect());
            let mut scope = Scope::default();
            scope.bound.push(capture_binding("v", ty, None));
            let err =
                check_capture_admission(QuotId(0), true, false, span, &ctx, &[], &mut prov, &scope)
                    .expect_err("a frame-rooted capture escaping is past-owning-frame");
            assert!(
                err.contains("does not survive the return"),
                "both cases keep the base rejection: {err}"
            );
            assert_eq!(
                err.contains(OWNING_QUOTATION_KEYWORD),
                wants_remedy,
                "remedy presence for `{ty}`: {err}"
            );
        }
    }

    /// Slice 10a (R2): the fifth materialization boundary, pinned directly.
    /// A poly signature cannot place an ordinary quotation anywhere but a
    /// direct top-level parameter (`reject_poly_quotation_anywhere`), so a
    /// `~` local can never reach the escaping-closure output/store boundaries
    /// through a real `.sth` program in this slice; the guard is exercised
    /// directly instead, the same way phase 1 pinned the routing predicates.
    #[test]
    fn check_capture_admission_rejects_captured_inline_quotation() {
        let mut scope = Scope::default();
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], vec![Type::I64]);
        scope.bound.push(Binding {
            name: "f".to_string(),
            ty: inl,
            aliases: None,
            deriv: None,
            quot: None,
            surviving: None,
        });
        let mut prov = Provenance::default();
        prov.quotation_captures
            .push(std::iter::once("f".to_string()).collect());
        let structs: Vec<StructDecl> = Vec::new();
        let enums: Vec<EnumDecl> = Vec::new();
        let ctx = Ctx::Line {
            structs: &structs,
            enums: &enums,
        };
        let span = Span {
            line: 1,
            col: 1,
            module: 0,
        };
        let err =
            check_capture_admission(QuotId(0), true, false, span, &ctx, &[], &mut prov, &scope)
                .expect_err("a captured `~` local must be rejected");
        assert!(
            err.contains("`~`") && err.contains("captured"),
            "a captured `~` local should be its own located rejection, not the ordinary \
             quotation deferral, got: {err}"
        );

        // The same rejection under `Ctx::Word` must name the enclosing word,
        // not fall back to `<line>` -- the only other call site exercises
        // `Ctx::Line`, which can't discriminate a discarded `Ctx` parameter
        // from a used one.
        let word = bare_word("outer", 0);
        let word_ctx = word_ctx(
            &word,
            &structs,
            &enums,
            &[],
            None,
            &CombinatorIndex::new(),
            None,
        );
        let word_err = check_capture_admission(
            QuotId(0),
            true,
            false,
            span,
            &word_ctx,
            &[],
            &mut prov,
            &scope,
        )
        .expect_err("a captured `~` local must be rejected");
        assert!(
            word_err.contains("`outer`"),
            "a captured `~` local under `Ctx::Word` should name the enclosing word: {word_err}"
        );
    }
}
