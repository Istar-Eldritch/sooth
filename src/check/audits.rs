//! Drop-overload discovery and the R7a quotation-type-position audits: does a
//! word literally named `drop` declare a legal override shape, and does a
//! quotation type appear only where slice 7's inliner can splice it away?
//! Engine-independent: neither pass touches `Ctx`, `Scope`, or `Moves`.

use std::collections::HashMap;

use super::*;

/// R1: recognize every user-defined `drop` overload -- a word literally
/// named `drop` whose declared effect is exactly one struct input and zero
/// outputs -- in its own pre-pass, before `check_types` and before any name
/// registration. Dispatch on a recognized override happens entirely through
/// the returned `StructId -> word index` table, never through a name lookup
/// on the string `"drop"`: `check_shuffle`'s `"drop"` arm (and `lower_call`'s
/// mirror of it) intercepts every `drop` call site before any name
/// resolution reaches `env`, so a word literally named `"drop"` registered
/// into `env` the ordinary way would be dead on arrival (see the Grounding
/// facts in the slice 8b spec).
///
/// This validates only the override's *declared shape*, never
/// `is_copy`/`is_linear` on the input type itself: that fold's own
/// termination argument depends on `check_recursion` having already run,
/// which happens inside `check_types`, after this pre-pass -- calling it
/// early would turn a cyclic struct declaration into a stack overflow
/// instead of a diagnostic.
///
/// A `HashMap<&str, usize>` keyed on the shared literal name `"drop"` (the
/// shape `check_tail_call_cycles`'s own `name_to_idx` uses) would silently
/// keep only the last `drop` word seen and must not be used here: the
/// registry is keyed by `StructId`, so overrides for distinct structs coexist
/// with no collision, and a second override for the *same* struct is instead
/// a located error.
pub fn find_drop_overloads(
    words: &[WordDef],
    structs: &[StructDecl],
) -> Result<HashMap<StructId, usize>, String> {
    let mut registry: HashMap<StructId, usize> = HashMap::new();
    for (idx, word) in words.iter().enumerate() {
        if word.name != "drop" {
            continue;
        }
        let id = drop_overload_struct_id(word)?;
        if registry.contains_key(&id) {
            return Err(duplicate_drop_overload_error(word, &structs[id.index()]));
        }
        registry.insert(id, idx);
    }
    Ok(registry)
}

/// R1: validate a `: drop` word's declared shape and return the struct id it
/// overrides, or a located error citing the word's own declaration --
/// modeled on `check_main_effect`'s shape (find the offending word by name,
/// report its span).
///
/// R11: the REPL calls this directly on its one entered `: drop` line, so a
/// line-at-a-time override gets exactly the declaration-shape rule a compiled
/// program's does; only the duplicate-override rejection differs, since a
/// second REPL `: drop` for one struct is a redefinition, not a collision.
pub fn drop_overload_struct_id(word: &WordDef) -> Result<StructId, String> {
    if !word.effect.outputs.is_empty() {
        return Err(drop_overload_output_error(word));
    }
    if word.effect.inputs.len() != 1 {
        return Err(drop_overload_arity_error(word));
    }
    match word.effect.inputs[0].ty {
        Type::Struct(id, _) => Ok(id),
        found => Err(drop_overload_non_struct_input_error(word, found)),
    }
}

/// R1: a `drop` overload declaring one or more outputs.
fn drop_overload_output_error(word: &WordDef) -> String {
    let span = word_span(word);
    format!(
        "error: `drop` overload (line {}, col {}) must declare zero outputs, found {}",
        span.line,
        span.col,
        effect_str(&word.effect)
    )
}

/// R1: a `drop` overload not declaring exactly one input.
fn drop_overload_arity_error(word: &WordDef) -> String {
    let span = word_span(word);
    format!(
        "error: `drop` overload (line {}, col {}) must declare exactly one input, found {}",
        span.line,
        span.col,
        effect_str(&word.effect)
    )
}

/// R1: a `drop` overload whose one input is not a `type:`-declared struct --
/// an enum, an array, a scalar, or a reference all land here.
fn drop_overload_non_struct_input_error(word: &WordDef, found: Type) -> String {
    let span = word_span(word);
    format!(
        "error: `drop` overload (line {}, col {}) must take a `type:`-declared struct, found `{}`",
        span.line, span.col, found
    )
}

/// R1: a second `drop` overload naming a struct that already has one.
fn duplicate_drop_overload_error(word: &WordDef, target: &StructDecl) -> String {
    let span = word_span(word);
    format!(
        "error: `{}` already defines its own `drop` (line {}, col {})",
        target.name, span.line, span.col
    )
}

/// Takes `&mut Module` because an array word (`fill`) interns its result
/// shape `[T N]` into `module.arrays` during checking (R3, R10): the same
/// registry `ir::lower` then reads, so the checker and the layout builder
/// share one `ArrayId` numbering. `check` runs before `lower`, so the
/// interned shapes are present when codegen consults them.
/// R7a: the type-position audit. A quotation type reaches every type position
/// the parser accepts (R2 routes it through `parse_type_expr`), but this slice
/// gives it a runtime representation at none of them: the one legal position
/// is a **direct input in a word's declared effect** (the quotation parameter
/// this slice adds). Every other position is a located rejection naming the
/// position and the offending type, pointing at slice 7 as the lift. This is
/// what makes R7's `unreachable!` arms sound rather than hopeful (the slice-4
/// audit-sweep shape, now for quotation *types*).
pub(super) fn audit_quotation_type_positions(module: &Module) -> Result<(), String> {
    audit_quotation_type_registries(
        &module.structs,
        &module.enums,
        &module.arrays,
        &module.owned_cells,
        &module.refs,
    )?;
    for w in &module.words {
        audit_word_quotation_positions(w, &module.structs, &module.enums, &module.arrays)?;
    }
    for decl in &module.externs {
        for slot in decl.effect.inputs.iter().chain(&decl.effect.outputs) {
            reject_quotation_type_position(
                slot.ty,
                &format!("an `extern:` boundary type of `{}`", decl.name),
            )?;
        }
    }
    Ok(())
}

/// R7a (REPL, item 2): the registry half of the audit, over exactly the shared
/// type registries. A quotation type never legally enters any of these (its
/// one legal home is a direct word parameter, stored in the word's `Sig`, and
/// a declared effect is interned separately), so re-scanning them per REPL
/// line is a safe, idempotent invariant. Split out so the REPL's `type:` and
/// `:` chokepoints run the same rejections as the native `check`, which the
/// REPL's `check_types`-only path skipped (a quotation in an audited position
/// then reached `ir_type_of`'s `unreachable!`, bricking the session).
pub(crate) fn audit_quotation_type_registries(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> Result<(), String> {
    for s in structs {
        for (fname, fty) in &s.fields {
            // R8 (D4): a quotation type is legal as a struct field this slice
            // (a materialization boundary); the store of a literal into it is
            // checked at the constructor/setter call site (R7). Every other
            // registry position below stays rejected.
            if matches!(fty, Type::Quotation(_)) {
                continue;
            }
            reject_quotation_type_position(
                *fty,
                &format!("the field `{fname}` of struct `{}`", s.name),
            )?;
        }
    }
    for e in enums {
        for v in &e.variants {
            for (idx, (fname, fty)) in v.fields.iter().enumerate() {
                reject_quotation_type_position(
                    *fty,
                    &format!(
                        "the {} of enum variant `{}::{}`",
                        super::variant_field_desc(fname, idx),
                        e.name,
                        v.name
                    ),
                )?;
            }
        }
    }
    for a in arrays {
        // R8 (D4): a quotation is legal as an array element this slice (a
        // materialization boundary, checked at `fill`/`!`); a cell payload and
        // a reference referent below are not D4 boundaries and stay rejected.
        if matches!(a.element, Type::Quotation(_)) {
            continue;
        }
        reject_quotation_type_position(a.element, "an array element")?;
    }
    for c in cells {
        reject_quotation_type_position(c.payload, "an owned-cell payload")?;
    }
    for r in refs {
        reject_quotation_type_position(r.referent, "a reference referent")?;
    }
    Ok(())
}

/// R7a (REPL, item 2): the per-word half of the audit -- a quotation in a
/// word's *output* row, a clause-bodied combinator, `main` taking one, or a
/// quotation nested inside a declared effect. A direct quotation *parameter*
/// (the one legal position) is accepted here and rejected separately at the
/// REPL (R23), which discards word bodies the inliner needs.
///
/// Also runs the poly twin of the no-stored-reference signature rule
/// (`audit_poly_reference_free_signature`): `structs`/`enums`/`arrays` are
/// needed only for that half (a poly word's output/input may fold to a fully
/// concrete `Type` nesting a reference inside a struct/enum/array, which
/// `contains_reference` resolves through the registries).
pub(crate) fn audit_word_quotation_positions(
    w: &WordDef,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let word = crate::resolve::demangle_word(&w.name);
    for slot in &w.effect.outputs {
        // R8 (D4): a monomorphic word may declare a `Type::Quotation` output (a
        // materialization boundary, checked at the exit row by `check_outputs`).
        // The poly path below still rejects a quotation output: polymorphic
        // quotation *values* are out of scope this slice.
        if matches!(slot.ty, Type::Quotation(_)) {
            continue;
        }
        reject_quotation_type_position(slot.ty, &format!("the output of `{word}`"))?;
    }
    // R18/R7a: a monomorphic word taking a quotation is a combinator,
    // which the inliner supports only with a *term* body (it splices the
    // body against the live stack); a clause body cannot be spliced, so
    // such a word would mint an `IrFunc` with a quotation parameter and
    // reach `ir_type_of`'s `unreachable!` arm (R7). Reject it here, with
    // the type positions, so that arm stays unreached. (A poly word's
    // effect is empty and is checked on the poly path, phase 2.)
    if w.poly.is_none()
        && matches!(w.body, WordBody::Clauses(_))
        && w.effect
            .inputs
            .iter()
            .any(|s| crate::ast::is_quotation_type(s.ty).is_some())
    {
        return Err(clause_bodied_quotation_word_error(word));
    }
    for slot in &w.effect.inputs {
        // Slice 10a (R2): a `~` input is recognized here too (accessor), so a
        // `~` to `main` and a quotation nested inside a `~` effect are rejected
        // exactly as for an ordinary quotation parameter.
        if let Some(eff) = crate::ast::is_quotation_type(slot.ty) {
            // `main` takes no quotation: it is an entry point, not a
            // combinator (D6/R28).
            if w.name == "main" {
                reject_quotation_type_position(slot.ty, "an input of `main`")?;
            }
            // A quotation nested inside a quotation effect (a quotation
            // taking a quotation) is deferred to slice 7, rejected rather
            // than half-supported.
            for t in eff.inputs.iter().chain(&eff.outputs) {
                reject_quotation_type_position(*t, "nested inside a quotation effect")?;
            }
        }
    }
    // A polymorphic word carries its signature in `w.poly`, not `w.effect`
    // (which is empty), so the output-position and nested-in-effect audits
    // above never see it. Run the same rejections over the poly signature,
    // driven by one recursive enumeration (item 2): a quotation may hide in a
    // poly *array element* (`[ [ 'T -- ] 3 ]`), which the earlier shallow
    // audit never descended into.
    if let Some(sig) = &w.poly {
        for pt in &sig.outputs {
            reject_poly_quotation_anywhere(pt, sig, &format!("the output of `{word}`"))?;
        }
        for pt in &sig.inputs {
            audit_poly_input_quotation(pt, sig)?;
        }
    }
    audit_poly_reference_free_signature(w, word, structs, enums, arrays)?;
    Ok(())
}

/// Phase 2 fix: the poly twin of `check_reference_free_signature`
/// (`word_entry.rs`). That check runs on `word.effect`, which is empty for a
/// poly word, so it never sees a poly signature at all -- Phase 2 made `&'T`
/// a producible output (`peek`'s `&['T 4]`), and nothing rejected the
/// signature the monomorphic checker forbids outright (`peeki`'s `&[i64 4]`),
/// so the escaping reference reached lowering and hit
/// `checked: every reference value records its referent`. Skipped for a
/// combinator, mirroring `check_word`'s own skip: a spliced word has no frame
/// of its own for the returned reference to outlive.
fn audit_poly_reference_free_signature(
    w: &WordDef,
    word: &str,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let Some(sig) = &w.poly else {
        return Ok(());
    };
    if is_combinator(w) {
        return Ok(());
    }
    for pt in &sig.outputs {
        if contains_poly_reference(pt, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{word}` declares the output `{}`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead",
                poly_type_str(pt, sig)
            ));
        }
    }
    for pt in &sig.inputs {
        // A top-level borrow is a legal input; only a *nested* one is not.
        // Both spellings count as top-level: a variable-bearing `&'T` stays
        // `PolyType::Ref`, while a fully concrete `&i64` folds to
        // `Concrete(Type::Ref)` (R-A4). Testing only the former rejected the
        // concrete spelling the monomorphic rule (`!slot.ty.is_ref()`,
        // `word_entry.rs`) accepts.
        let top_level_ref =
            matches!(pt, PolyType::Ref(..)) || matches!(pt, PolyType::Concrete(t) if t.is_ref());
        if !top_level_ref && contains_poly_reference(pt, structs, enums, arrays) {
            return Err(format!(
                "error: a reference cannot be stored: `{word}` declares the input `{}`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate",
                poly_type_str(pt, sig)
            ));
        }
    }
    Ok(())
}

/// Whether `pt` **transitively contains** a reference, the poly-signature
/// twin of `contains_reference` (`builtins.rs`), which the `Concrete` arm
/// delegates to (a fully-concrete `Type` may still nest a reference inside a
/// struct field, enum variant or array element -- the same shapes
/// `contains_reference` already resolves through the registries). A
/// `PolyType::Quotation` is not descended into: a word carrying one is a
/// combinator (the direct parameter is the only legal position,
/// `audit_poly_input_quotation`), and this whole audit is skipped for a
/// combinator above -- so a non-combinator word can never reach this arm
/// with a `Quotation` at all, let alone one hiding a reference.
fn contains_poly_reference(
    pt: &PolyType,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> bool {
    match pt {
        PolyType::Ref(..) => true,
        PolyType::Concrete(ty) => contains_reference(*ty, structs, enums, arrays),
        PolyType::Array(elem, _) => contains_poly_reference(elem, structs, enums, arrays),
        PolyType::Var(_) | PolyType::Quotation(..) => false,
        // P7 slice 3a: a generic carrying `&'T` (e.g. `Box[&'T]`) must not
        // escape the Copy-containment audit through an argument.
        PolyType::Generic { args, .. } => args
            .iter()
            .any(|a| contains_poly_reference(a, structs, enums, arrays)),
    }
}

/// R7a (poly path, item 2): audit a poly word *input*, where a direct
/// quotation is the one legal position (the combinator's parameter). The
/// parameter itself is accepted, but a quotation buried inside it -- an array
/// element (`[ [ 'T -- ] 3 ]`), or nested in the parameter's own effect rows
/// -- is rejected.
fn audit_poly_input_quotation(pt: &PolyType, sig: &PolySig) -> Result<(), String> {
    match pt {
        PolyType::Quotation(ins, outs, _, _, _) => {
            for t in ins.iter().chain(outs) {
                reject_poly_quotation_anywhere(t, sig, "nested inside a quotation effect")?;
            }
            Ok(())
        }
        PolyType::Array(elem, _) => reject_poly_quotation_anywhere(elem, sig, "an array element"),
        // A `~` parameter always folds to `Concrete` (its effect has no row),
        // so the nested-effect rejection must live here too, not only in the
        // variable-bearing `Quotation` arm above -- otherwise a `~` (or an
        // ordinary quotation) buried in a concrete parameter's effect slips
        // past silently.
        PolyType::Concrete(ty) => {
            if let Some(eff) = crate::ast::is_quotation_type(*ty) {
                for t in eff.inputs.iter().chain(&eff.outputs) {
                    reject_quotation_type_position(*t, "nested inside a quotation effect")?;
                }
            }
            Ok(())
        }
        PolyType::Var(_) => Ok(()),
        // Slice 13 (R-A9): a quotation buried behind a `&` is still a nested
        // effect position, so recurse rather than accepting the referent
        // unseen.
        PolyType::Ref(referent, _) => {
            reject_poly_quotation_anywhere(referent, sig, "a reference's referent")
        }
        // P7 slice 3a: a quotation smuggled in as a generic argument
        // (`Box[[ 'T -- ]]`) is still nested inside the parameter.
        PolyType::Generic { args, .. } => {
            for a in args {
                reject_poly_quotation_anywhere(a, sig, "a generic type argument")?;
            }
            Ok(())
        }
    }
}

/// R7a (poly path, item 2): reject a quotation type appearing *anywhere*
/// inside `pt` -- as the whole position, as an array element, or nested in a
/// quotation effect -- naming the innermost position. Driving every poly
/// non-parameter position from one recursive enumeration is what keeps R7's
/// default-deny `unreachable!` arms sound: a quotation buried in a poly array
/// element must not slip past the audit and reach `ir_type_of`. A
/// fully-concrete quotation folds to `Concrete(Type::Quotation)`, so route
/// that through the monomorphic rejection to share the rendering.
fn reject_poly_quotation_anywhere(
    pt: &PolyType,
    sig: &PolySig,
    position: &str,
) -> Result<(), String> {
    match pt {
        PolyType::Concrete(ty) => reject_quotation_type_position(*ty, position),
        PolyType::Var(_) => Ok(()),
        PolyType::Array(elem, _) => reject_poly_quotation_anywhere(elem, sig, "an array element"),
        PolyType::Ref(referent, _) => {
            reject_poly_quotation_anywhere(referent, sig, "a reference's referent")
        }
        // P7 slice 3a: recurse into a generic's arguments, so a quotation
        // smuggled in as one (`Box[[ 'T -- ]]`) is still rejected.
        PolyType::Generic { args, .. } => {
            for a in args {
                reject_poly_quotation_anywhere(a, sig, "a generic type argument")?;
            }
            Ok(())
        }
        PolyType::Quotation(..) => Err(format!(
            "error: a quotation type `{}` cannot appear as {position}: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
            poly_type_str(pt, sig),
        )),
    }
}

/// R18/R7a: a monomorphic quotation-taking word with a clause body cannot be
/// inlined (a clause body is not a splice-able term list), so it is rejected
/// rather than left to panic at lowering. Slice 7's runtime quotation value
/// lifts it (the word would then `call` a real value, no inlining needed).
fn clause_bodied_quotation_word_error(word: &str) -> String {
    format!(
        "error: the quotation-taking word `{word}` has a clause body; a quotation parameter is only supported on a word with a term body this slice (its body is inlined at each call site, and a clause body cannot be spliced), and a runtime quotation value is slice 7",
    )
}

/// R7a: reject `ty` if it is a quotation type, naming the position and slice 7.
pub(super) fn reject_quotation_type_position(ty: Type, position: &str) -> Result<(), String> {
    // Slice 10a (R2): both variants are rejected here. An ordinary
    // `Type::Quotation` reaches this only after the legal-position `continue`s
    // above have skipped the field/output/array cases; a `~` never has such a
    // legal position, so it always lands here. Accessor, not a second arm, so
    // the `~` case cannot be silently dropped.
    if let Some(eff) = crate::ast::is_quotation_type(ty) {
        return Err(format!(
            "error: a quotation type `{}` cannot appear as {position}: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
            eff.name_static,
        ));
    }
    Ok(())
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
    fn quotation_in_a_positional_variant_field_is_named_by_index() {
        // OQ4/Phase 1: the type-position audit prints a variant field name
        // too, so it must name an attributeless field by position rather than
        // leaking the internal placeholder.
        let err = check_src(
            "type: Option 'T | None | Some 'T ;\n: w ( Option[[ i64 -- i64 ]] -- ) drop ;\n: main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("the field 0 of enum variant"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains(crate::parser::POSITIONAL_FIELD_NAME),
            "the internal placeholder leaked into a diagnostic: {err}"
        );
    }

    #[test]
    fn poly_quotation_behind_a_reference_is_rejected() {
        // Slice 13 (R-A9): the default-deny recurses through a reference's
        // referent, so a quotation buried behind a `&` cannot slip past the
        // audit and reach `ir_type_of`.
        let err = check_src(": f ( &[ 'T -- ] -- ) drop ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a quotation type `[ 'T -- ]` cannot appear as a reference's referent: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
        );
    }

    #[test]
    fn poly_quotation_behind_a_reference_inside_an_array_element_is_rejected() {
        // Slice 13 (review fix): the test above reaches `audit_poly_input_quotation`'s
        // own `Ref` arm, but never `reject_poly_quotation_anywhere`'s `Ref` arm --
        // reached only when a `Ref` shows up *inside* a position that arm is
        // already recursing through (here, an array element). Stubbing that
        // arm to `Ok(())` lets `[&[ 'T -- ] 4]` sail through the checker.
        let err = check_src(": f ( [&[ 'T -- ] 4] -- ) drop ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a quotation type `[ 'T -- ]` cannot appear as a reference's referent: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
        );
    }

    #[test]
    fn poly_output_borrow_that_outlives_its_frame_is_rejected() {
        // Phase 2 review blocker: `word.effect` is empty for a poly word, so
        // `check_reference_free_signature` (`word_entry.rs`) never saw this
        // shape and the escaping reference reached lowering, which panicked
        // ("checked: every reference value records its referent"). The
        // monomorphic twin of this signature is already rejected at
        // declaration; the poly path must match.
        let err = check_src(
            ": peek ( ['T: Copy 4] -- &['T 4] ) | a | &a ;\n: main ( -- ) 10 4 fill peek drop ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `peek` declares the output `&['T 4]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
    }

    #[test]
    fn poly_input_carrying_a_nested_reference_is_rejected() {
        // Phase 2 review blocker: an input may *be* a reference at the top
        // level, but not carry one nested inside an aggregate -- the
        // monomorphic checker already rejects `[&i64 4]` this way, and the
        // poly path was missing the same check for `[&'T 4]`.
        let err = check_src(": g ( 'T: Copy [&'T 4] -- ) drop ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `g` declares the input `[&'T 4]`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate"
        );
    }

    #[test]
    fn quotation_smuggled_as_generic_arg_is_rejected() {
        // P7 slice 3a: a quotation type as a generic type argument is still
        // nested inside the enclosing parameter, exactly as an array element
        // is -- `reject_poly_quotation_anywhere`'s `Generic` arm recurses
        // into `args` rather than accepting them unseen.
        let err =
            check_src("type: Box 'T val 'T ;\n: f ( Box[[ 'T -- 'T ]] -- ) drop ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a quotation type `[ 'T -- 'T ]` cannot appear as a generic type argument: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
        );
    }

    #[test]
    fn ref_bearing_generic_in_copy_position_is_rejected() {
        // P7 slice 3a: `contains_poly_reference`'s `Generic` arm recurses
        // into `args`, so a reference carried inside a generic argument
        // (`Box[&'T]`) still trips the reference-cannot-be-stored audit on
        // an output, exactly as a bare `&'T` output does.
        let err = check_src("type: Box 'T val 'T ;\n: f ( &'T -- Box[&'T] ) | r | Box r ;\n")
            .unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `f` declares the output `Box[&'T]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
    }

    #[test]
    fn poly_input_that_is_a_concrete_reference_is_accepted() {
        // Phase 2 review blocker: a fully concrete `&i64` input folds to
        // `Concrete(Type::Ref)`, not `PolyType::Ref` (R-A4), so a top-level
        // test written only against the latter rejected a signature the
        // monomorphic rule accepts.
        check_src(": g ( &i64 'T: Copy -- 'T ) | r t | r drop t ;\n")
            .expect("a top-level concrete reference input is legal");
    }

    /// Slice 10a (R2): the declaration-position rejection is no longer fail-open
    /// for a `~` -- it used to return `Ok` (`if let Type::Quotation`), letting a
    /// `~` slip past silently. Constructed directly.
    #[test]
    fn reject_quotation_type_position_rejects_inline() {
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let err = reject_quotation_type_position(inl, "a struct field").unwrap_err();
        assert!(err.contains("~[ i64 -- ]"), "names the `~` type: {err}");
        assert!(err.contains("a struct field"), "names the position: {err}");
        // The ordinary quotation is still rejected in this position too.
        let ord = crate::ast::quotation_type(vec![Type::I64], Vec::new());
        assert!(reject_quotation_type_position(ord, "a struct field").is_err());
    }
    /// Slice 10a (R2): a word declaring a `~` *output* is rejected by the audit,
    /// where a bare `Type::Quotation` output is allowed (a materialization
    /// boundary). The `~` cannot be materialized, so it is never a legal output.
    #[test]
    fn audit_rejects_inline_quotation_output_but_allows_ordinary() {
        use crate::ast::TypedSlot;
        let mk = |ty: Type| WordDef {
            name: "w".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: vec![TypedSlot { name: None, ty }],
            },
            body: WordBody::Terms { terms: Vec::new() },
            poly: None,
            declares_inline: false,
            module: 0,
            span: Span::default(),
            declared_globals: None,
        };
        let inl = crate::ast::inline_quotation_type(vec![Type::I64], Vec::new());
        let err = audit_word_quotation_positions(&mk(inl), &[], &[], &[]).unwrap_err();
        assert!(err.contains("the output of `w`"), "locates it: {err}");
        let ord = crate::ast::quotation_type(vec![Type::I64], Vec::new());
        assert!(audit_word_quotation_positions(&mk(ord), &[], &[], &[]).is_ok());
    }
    #[test]
    fn check_drop_overload_on_non_struct_input_is_error() {
        // Criterion 5/R1: an enum, an array, or a scalar input is rejected
        // exactly as a non-struct input would be, with a located error.
        let enum_input = "type: E | V ; : drop ( E -- ) drop ;";
        let err = check_src(enum_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("type:"), "unexpected message: {err}");

        let array_input = ": drop ( [i64 4] -- ) drop ;";
        let err = check_src(array_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");

        let scalar_input = ": drop ( i64 -- ) drop ;";
        let err = check_src(scalar_input).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
    }
    #[test]
    fn check_drop_overload_with_wrong_arity_is_error() {
        // R1: a `drop` overload declaring anything other than exactly one
        // input is a located error, distinct from the non-struct-input and
        // output rejections tested above.
        let src = "type: T x i64 ; : drop ( T T -- ) drop drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(
            err.contains("must declare exactly one input"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_drop_overload_with_output_is_error() {
        // Criterion 6/R1: a `drop` overload declaring an output is a located
        // error, regardless of whether it also declares an input.
        let src = "type: T x i64 ; : drop ( T -- i64 ) drop 0 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("output"), "unexpected message: {err}");
    }
    #[test]
    fn check_duplicate_drop_overload_for_one_struct_is_error() {
        // Criterion 7/R1: two `drop` overloads for the same struct id is a
        // located error naming that struct, even though the two words'
        // bodies are otherwise unrelated. Both bodies destructure rather
        // than self-recurse: a self-recursive body would let R6's own
        // recursion check produce a message containing both "T" and "drop"
        // even if the duplicate-override rejection this test targets were
        // deleted entirely, since `find_drop_overloads` runs and returns
        // before either body is ever checked.
        let src = "type: T x i64 ; : drop ( T -- ) | a | a T> drop ; \
                   : drop ( T -- ) | a | a T> drop ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("`T` already defines its own `drop`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_drop_overloads_for_different_structs_both_land_in_the_registry() {
        // Criterion 16's check-side half: two overrides for different
        // structs coexist with distinct `StructId` keys, with no collision
        // reported (the module checks fine), and the registry carries one
        // entry per struct.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) | a | a A> . ; : drop ( B -- ) | b | b B> . ; \
                   : main ( -- ) 1 A drop 2 B drop ;";
        check_src(src).unwrap();

        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let registry = find_drop_overloads(&module.words, &module.structs).unwrap();
        assert_eq!(
            registry.len(),
            2,
            "expected one entry per struct: {registry:?}"
        );
    }
    #[test]
    fn check_drop_overloads_are_excluded_from_env() {
        // Stage-test obligation (criterion 16's check-side half): neither
        // override lands in `env` under the shared literal name `"drop"` --
        // if it did, the second override registered would silently clobber
        // the first with no diagnostic, since `check`'s env-registration
        // loop has no redeclaration check for ordinary `:` words the way
        // `check_extern_decls` has for `extern:`. Mirrors `check`'s own
        // filtered registration loop rather than calling it directly, since
        // `env` is internal to `check`.
        let src = "type: A x i64 ; type: B y i64 ; \
                   : drop ( A -- ) drop ; : drop ( B -- ) drop ; \
                   : main ( -- ) 1 A drop 2 B drop ;";
        let tokens = crate::lexer::lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let registry = find_drop_overloads(&module.words, &module.structs).unwrap();
        let overload_indices: HashSet<usize> = registry.values().copied().collect();
        let mut env: HashMap<String, Vec<Overload>> = HashMap::new();
        for (idx, word) in module.words.iter().enumerate() {
            if overload_indices.contains(&idx) {
                continue;
            }
            env.insert(
                word.name.clone(),
                vec![Overload {
                    sig: sig_of(&word.effect),
                    symbol: word.name.clone(),
                }],
            );
        }
        assert!(
            !env.contains_key("drop"),
            "a `drop` overload leaked into env: {env:?}"
        );
    }
    #[test]
    fn check_drop_overload_with_self_recursive_struct_is_still_a_declaration_error_not_overflow() {
        // R1's ordering-hazard caveat: a self-recursive struct with a
        // malformed `drop` override naming that very struct (here, an
        // extra output) must still produce this pre-pass's own located
        // diagnostic, not overflow the stack inside `is_copy`/
        // `check_recursion` -- the pre-pass runs before `check_types`
        // (where `check_recursion` lives) and never calls `is_copy` on the
        // declared input type itself.
        let src = "type: Loop | Wrap next Loop | End ; : drop ( Loop -- i64 ) drop 0 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("drop"), "unexpected message: {err}");
        assert!(err.contains("output"), "unexpected message: {err}");
    }
}
