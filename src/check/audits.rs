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
        target.name_static, span.line, span.col
    )
}

/// Takes `&mut Module` because an array word (`fill`) interns its result
/// shape `array[T N]` into `module.arrays` during checking (R3, R10): the same
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
                &format!(
                    "an `extern:` boundary type of {}",
                    crate::resolve::render_word(&decl.name)
                ),
            )?;
        }
    }
    Ok(())
}

/// R7a (item 2): the registry half of the audit, over exactly the shared
/// type registries. A quotation type never legally enters any of these (its
/// one legal home is a direct word parameter, stored in the word's `Sig`, and
/// a declared effect is interned separately), so re-scanning them is a safe,
/// idempotent invariant. Split out from the per-word half so a bare
/// `type:`/`:` chokepoint can run the same rejections a `check_types`-only
/// path would otherwise skip (a quotation in an audited position would then
/// reach `ir_type_of`'s `unreachable!`).
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
            // checked at the constructor/setter call site (R7). P7.S3v (R6)
            // widens the same carve-out to the owning flavour: the container's
            // synthesized destructor disposes it (R5 makes the field linear,
            // `emit_drop`'s disposer arm does the work).
            if matches!(fty, Type::Quotation(_) | Type::OwningQuotation(_)) {
                continue;
            }
            reject_quotation_type_position(
                *fty,
                &format!("the field `{fname}` of struct `{}`", s.name_static),
            )?;
        }
    }
    for e in enums {
        for v in &e.variants {
            for (idx, (fname, fty)) in v.fields.iter().enumerate() {
                // P7.S3v (R6): an owning variant field is admitted for the
                // same reason a struct field is -- the enum's destructor
                // disposes it. This is its own carve-out, not a mirror of the
                // struct one: a *plain* quotation variant field has never been
                // admitted (D4 scoped its boundary to struct fields) and stays
                // rejected below.
                if matches!(fty, Type::OwningQuotation(_)) {
                    continue;
                }
                reject_quotation_type_position(
                    *fty,
                    &format!(
                        "the {} of enum variant `{}::{}`",
                        super::variant_field_desc(fname, idx),
                        e.name_static,
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
        // P7.S3v (R6): the cell's own destructor drops its payload, which for
        // an owning closure is the disposer call. A plain quotation payload is
        // not a D4 boundary and stays rejected.
        if matches!(c.payload, Type::OwningQuotation(_)) {
            continue;
        }
        reject_quotation_type_position(c.payload, "an owned-cell payload")?;
    }
    for r in refs {
        reject_quotation_type_position(r.referent, "a reference referent")?;
    }
    Ok(())
}

/// R7a (item 2): the per-word half of the audit -- a quotation in a
/// word's *output* row, `main` taking one, or a quotation nested inside a
/// declared effect. A direct quotation *parameter* (the one legal position)
/// is accepted here.
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
        // P7.S3h: an `owning` output is *the* way to hand a caller an owning
        // closure, so it joins the plain quotation as a legal output position.
        if matches!(slot.ty, Type::Quotation(_) | Type::OwningQuotation(_)) {
            continue;
        }
        reject_quotation_type_position(slot.ty, &format!("the output of `{word}`"))?;
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
/// a producible output (`peek`'s `&array['T 4]`), and nothing rejected the
/// signature the monomorphic checker forbids outright (`peeki`'s `&array[i64 4]`),
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
        // P7.S3n (R3): a cell payload is followed for the same reason an
        // array element is -- `^&'T` must not launder a reference past the
        // audit through the payload position.
        PolyType::OwnedCell(payload) => contains_poly_reference(payload, structs, enums, arrays),
        PolyType::Var(_) | PolyType::Quotation(..) => false,
        // P7 slice 3b: a body-only marker, never in a declared signature.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        // P7 slice 3a: a generic carrying `&'T` (e.g. `Box[&'T]`) must not
        // escape the Copy-containment audit through an argument.
        PolyType::Generic { args, .. } => args
            .iter()
            .any(|a| contains_poly_reference(a, structs, enums, arrays)),
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this audit walks.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
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
        // P7 slice 3b: a body-only marker, never in a declared signature.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        // Slice 13 (R-A9): a quotation buried behind a `&` is still a nested
        // effect position, so recurse rather than accepting the referent
        // unseen.
        PolyType::Ref(referent, _) => {
            reject_poly_quotation_anywhere(referent, sig, "a reference's referent")
        }
        // P7.S3n (R3): the cell twin of the `&` arm above.
        PolyType::OwnedCell(payload) => {
            reject_poly_quotation_anywhere(payload, sig, "an owned-cell payload")
        }
        // P7 slice 3a: a quotation smuggled in as a generic argument
        // (`Box[[ 'T -- ]]`) is still nested inside the parameter.
        PolyType::Generic { args, .. } => {
            for a in args {
                reject_poly_quotation_anywhere(a, sig, "a generic type argument")?;
            }
            Ok(())
        }
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this audit walks.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
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
        PolyType::OwnedCell(payload) => {
            reject_poly_quotation_anywhere(payload, sig, "an owned-cell payload")
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
        // P7 slice 3b: a body-only marker, never in a declared signature.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this audit walks.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
    }
}

/// P7.S3h: an owning closure cannot cross a *splice* boundary. A spliced
/// (`inline`) word's quotation parameter is never a runtime value -- the
/// caller's literal is inlined in place, so the callee's `call` is a splice and
/// the heap env this slice builds is never constructed at all. That makes the
/// declaration a lie in both directions: the splice route compares only the
/// inline-vs-plain axis, so a plain `[ ... ]` literal silently satisfies an
/// `owning` slot, and an already-materialized owning value handed to the same
/// slot would be forwarded into a splice with no body to inline. Rejecting the
/// *declaration* is what keeps `OwningQuotation(e) != Quotation(e)`
/// load-bearing rather than decorative.
///
/// A *generic* signature is rejected for a neighbouring reason: a polymorphic
/// call site's quotation arguments are materialized off `CallInst::quot_inputs`,
/// which records the effect and not the flavour, so an owning parameter there
/// would be built with a plain closure's frame env. Polymorphism over the
/// flavour is out of scope this slice, so the signature is refused rather than
/// half-supported.
///
/// Called *after* `check_types` rather than from the pre-pass audit: an owning
/// parameter is what makes the inherited linear machinery observable, so
/// `dup`ping such a binding or forgetting it must report its own error rather
/// than be masked by this one.
pub(crate) fn reject_owning_quotation_declarations(word: &WordDef) -> Result<(), String> {
    let name = crate::resolve::demangle_word(&word.name);
    // An `owning` effect always folds to `Concrete`: the parser rejects a
    // variable-bearing one outright, so there is no `PolyType::Quotation`
    // spelling of it to look through.
    let poly = word.poly.as_ref().and_then(|sig| {
        sig.inputs.iter().find_map(|pt| match pt {
            PolyType::Concrete(Type::OwningQuotation(eff)) => Some(*eff),
            _ => None,
        })
    });
    if let Some(eff) = poly {
        return Err(format!(
            "error: `{name}` is generic and declares `{}`: a polymorphic call site materializes its quotation arguments from the declared effect alone, which does not carry the owning flavour",
            eff.name_static
        ));
    }
    if !crate::check::is_combinator(word) {
        return Ok(());
    }
    let mono = word.effect.inputs.iter().find_map(|slot| match slot.ty {
        Type::OwningQuotation(eff) => Some(eff),
        _ => None,
    });
    match mono {
        Some(eff) => Err(format!(
            "error: `{name}` is spliced (`inline`) and declares `{}`: an owning closure is a runtime value, and a spliced quotation parameter is never materialized, so it cannot carry the disposal obligation the type names",
            eff.name_static
        )),
        None => Ok(()),
    }
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

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module)
    }
    #[test]
    fn quotation_in_a_positional_variant_field_is_named_by_index() {
        // OQ4/Phase 1: the type-position audit prints a variant field name
        // too, so it must name an attributeless field by position rather than
        // leaking the internal placeholder.
        let err = check_src(
            "type: Option['T] | None | Some 'T ;\n: w ( Option[[ i64 -- i64 ]] -- ) drop ;\n: main ( -- ) ;\n",
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
        // arm to `Ok(())` lets `array[&[ 'T -- ] 4]` sail through the checker.
        let err = check_src(": f ( array[&[ 'T -- ] 4] -- ) drop ;\n").unwrap_err();
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
            ": peek['T: Copy] ( array['T 4] -- &array['T 4] ) | a | &a ;\n: main ( -- ) 10 4 fill peek drop ;\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `peek` declares the output `&array['T 4]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
    }

    #[test]
    fn poly_input_carrying_a_nested_reference_is_rejected() {
        // Phase 2 review blocker: an input may *be* a reference at the top
        // level, but not carry one nested inside an aggregate -- the
        // monomorphic checker already rejects `array[&i64 4]` this way, and the
        // poly path was missing the same check for `array[&'T 4]`.
        let err = check_src(": g ['T: Copy] ( 'T array[&'T 4] -- ) drop ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `g` declares the input `array[&'T 4]`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate"
        );
    }

    #[test]
    fn quotation_smuggled_as_generic_arg_is_rejected() {
        // P7 slice 3a: a quotation type as a generic type argument is still
        // nested inside the enclosing parameter, exactly as an array element
        // is -- `reject_poly_quotation_anywhere`'s `Generic` arm recurses
        // into `args` rather than accepting them unseen.
        let err =
            check_src("type: Box['T] val 'T ;\n: f ( Box[[ 'T -- 'T ]] -- ) drop ;\n").unwrap_err();
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
        let err = check_src("type: Box['T] val 'T ;\n: f ( &'T -- Box[&'T] ) | r | Box r ;\n")
            .unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `f` declares the output `Box[&'T]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
    }

    #[test]
    fn quotation_behind_an_owned_cell_is_rejected() {
        // P7.S3n (R3): a quotation effect hidden in a cell payload is still a
        // nested effect position, on both audit walks. Reachable from source
        // (a bare `^` sigil whose payload is the following slot), and it
        // shares its "an owned-cell payload" wording with the concrete twin
        // at `:205` rather than inventing a second spelling.
        let err = check_src(": f ( ^ [ 'T -- ] -- ) drop ;\n").unwrap_err();
        assert_eq!(
            err,
            "error: a quotation type `[ 'T -- ]` cannot appear as an owned-cell payload: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
        );
    }

    #[test]
    fn ref_bearing_owned_cell_input_is_rejected() {
        // P7.S3n (R3): `contains_poly_reference`'s cell arm recurses into the
        // payload, so a reference laundered behind a `^` (`^ &'T`, a bare
        // sigil whose payload is the following slot) still trips the
        // reference-cannot-be-stored audit. The concrete twin
        // (`contains_reference`) does not descend into `Type::OwnedCell`, so
        // this arm's answer is not inherited from anywhere -- deleting the
        // recursion admits the signature outright.
        let err = check_src(": f ( ^ &'T -- ) drop ;\n").unwrap_err();
        assert!(
            err.contains("a reference cannot be stored"),
            "unexpected: {err}"
        );
        assert!(err.contains("^&'T"), "the shape must be named: {err}");
    }

    #[test]
    fn poly_input_that_is_a_concrete_reference_is_accepted() {
        // Phase 2 review blocker: a fully concrete `&i64` input folds to
        // `Concrete(Type::Ref)`, not `PolyType::Ref` (R-A4), so a top-level
        // test written only against the latter rejected a signature the
        // monomorphic rule accepts.
        check_src(": g ['T: Copy] ( &i64 'T -- 'T ) | r t | r drop t ;\n")
            .expect("a top-level concrete reference input is legal");
    }

    /// P7 slice 3c (R1.4/R5, poly twin): the poly path answers a slice the
    /// same way `slice_output_is_rejected_and_slice_input_is_admitted`
    /// (`word_entry.rs`) proves the monomorphic path does, and it gets there
    /// by two different routes: the output loop through
    /// `contains_poly_reference`'s `Concrete` delegation, the input loop
    /// through `top_level_ref`'s `Concrete(t) if t.is_ref()` clause. A slice
    /// inside a generic body is the point of the type (R11), so both routes
    /// are load-bearing rather than inherited by accident. Built directly:
    /// the type has no surface spelling until its construction words land.
    #[test]
    fn poly_slice_output_is_rejected_and_poly_slice_input_is_admitted() {
        let mut slices = Vec::new();
        let slice = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let mk = |inputs: Vec<PolyType>, outputs: Vec<PolyType>| WordDef {
            name: "w".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
            body: Vec::new(),
            poly: Some(Box::new(PolySig {
                row_in: None,
                inputs,
                outputs,
                row_out: None,
                bounds: Vec::new(),
                ty_var_names: vec!["'T".to_string()],
                ty_var_spans: Vec::new(),
                ty_kinds: Vec::new(),
                len_var_names: Vec::new(),
                len_var_spans: Vec::new(),
                row_var_names: Vec::new(),
            })),
            declares_inline: false,
            module: 0,
            span: Span::default(),
            declared_globals: None,
        };
        let out = mk(
            vec![PolyType::Var(0)],
            vec![PolyType::Concrete(slice), PolyType::Var(0)],
        );
        let err = audit_poly_reference_free_signature(&out, "w", &[], &[], &[]).unwrap_err();
        assert_eq!(
            err,
            "error: a reference cannot be stored: `w` declares the output `Slice[i64]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        );
        let inp = mk(
            vec![PolyType::Concrete(slice), PolyType::Var(0)],
            vec![PolyType::Var(0)],
        );
        audit_poly_reference_free_signature(&inp, "w", &[], &[], &[])
            .expect("a slice *input* is legal in a poly signature too");
        // The nested-aggregate ban is not widened by the top-level
        // admission: a slice carried inside an array is still rejected.
        let nested = mk(
            vec![PolyType::Array(
                Box::new(PolyType::Concrete(slice)),
                crate::ast::Len::Concrete(4),
            )],
            Vec::new(),
        );
        let nested_err = audit_poly_reference_free_signature(&nested, "w", &[], &[], &[])
            .expect_err("a slice nested inside an aggregate input is still rejected");
        assert!(
            nested_err.contains("which contains a reference"),
            "nested slice input hits the containment wording: {nested_err}"
        );
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
            body: Vec::new(),
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

        let array_input = ": drop ( array[i64 4] -- ) drop ;";
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
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
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
        let module = crate::test_support::parse_with_core(&tokens).unwrap();
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

    /// P7.S3v (R6): the three storage positions this slice admits, and the
    /// mutation guard on their three *separate* carve-outs. Each one is its
    /// own `matches!` in `audit_quotation_type_registries`, so reverting any
    /// one of them alone must fail exactly its row here -- what proves the
    /// three are additive rather than one widened wildcard.
    ///
    /// What makes them sound is not the audit but what sits behind it:
    /// `field_is_linear`/`layout_field_is_linear` answer `true` for an owning
    /// quotation (R5), so `synthesize_aggregate_destructors` emits a
    /// destructor for the container, whose field glue calls `emit_drop`, whose
    /// owning-quotation arm (R3) runs the value's per-construction-site
    /// disposer. Revert any of those three and the container's `drop` silently
    /// becomes a no-op again; the end-to-end goldens in
    /// `tests/phase7_slice3v.rs` are what catch that, since a checker-level
    /// test cannot see a leak.
    ///
    /// The variant-field and cell-payload carve-outs admit the **owning**
    /// flavour only: a plain quotation in either position is not a D4
    /// materialization boundary and stays rejected
    /// (`a_plain_quotation_is_still_rejected_outside_a_struct_field` below).
    #[test]
    fn owning_quotation_is_admitted_in_three_positions() {
        for src in [
            "type: Box q owning [ -- ] ;\n",
            "type: E | None | Some q owning [ -- ] ;\n",
            ": f ( ^ owning [ -- ] -- ) drop ;\n",
        ] {
            check_src(src).unwrap_or_else(|e| panic!("`{src}` should check: {e}"));
        }
    }

    /// The un-widened half of R6's two new carve-outs: they are keyed on
    /// `Type::OwningQuotation` specifically, so a *plain* quotation variant
    /// field or cell payload keeps the rejection it has always had. Without
    /// this, widening either carve-out to `Type::Quotation(_) |
    /// Type::OwningQuotation(_)` (the tempting symmetry with the struct-field
    /// one) would go unnoticed -- and a plain quotation there has no D4 store
    /// check behind it at all.
    #[test]
    fn a_plain_quotation_is_still_rejected_outside_a_struct_field() {
        for (src, position) in [
            (
                "type: E | None | Some q [ -- ] ;\n",
                "the field `q` of enum variant `E",
            ),
            (": f ( ^ [ -- ] -- ) drop ;\n", "an owned-cell payload"),
        ] {
            let err = check_src(src).unwrap_err();
            assert!(
                err.contains("cannot appear as") && err.contains(position),
                "unexpected message for `{src}`: {err}"
            );
        }
    }

    /// P7.S3h's containment rule, minus the three positions P7.S3v (R6)
    /// admits. A reference referent and an `extern:` boundary type stay
    /// rejected: neither owns what it names, so neither can dispose it, and
    /// `reject_quotation_type_position` still reaches both.
    ///
    /// These two are also the blast-radius guard on R6: they must pass
    /// unchanged before and after the carve-outs land.
    ///
    /// One aggregate position is **not** on either list, and honestly so: a
    /// word's synthesized multi-output bundle struct is interned by
    /// `intern_output_bundles` *after* these type-level audits run, so an
    /// `owning` output legitimately reaches that struct as a field. It stays
    /// sound because the bundle is a destructor-free transient ABI carrier
    /// (`is_bundle`-flagged, no synthesized `drop`), unpacked at the call site
    /// the instant the word returns: the owning value flows straight back out
    /// as a linear stack value, so its call-once obligation is never handed to
    /// a container that could no-op its disposal. That accept case is pinned
    /// end-to-end (builds and runs) by
    /// `an_owning_closure_as_one_of_several_outputs_builds_and_runs` in
    /// tests/phase7_slice3h.rs, so it is deliberately not duplicated here.
    #[test]
    fn owning_quotation_is_rejected_in_every_remaining_aggregate_position() {
        for (src, position) in [
            (
                ": f ( & owning [ -- ] -- ) drop ;\n",
                "a reference referent",
            ),
            (
                "extern: f ( owning [ -- ] -- ) \"f\" ;\n",
                "an `extern:` boundary type of `f",
            ),
        ] {
            let err = check_src(src).unwrap_err();
            assert!(
                err.contains("cannot appear as") && err.contains(position),
                "unexpected message for `{src}`: {err}"
            );
        }
    }

    /// P7.S3h: an owning quotation element is rejected in both array and
    /// slice position, but by different guards now that the Phase 4 array
    /// destructor admits linear array elements.
    ///
    /// The array position is swept by the audit itself
    /// (`audit_quotation_type_registries` rejects `OwningQuotation` as an
    /// array element because it is a quotation type, not because it is
    /// linear), so deleting the former linear-array gate still rejects. The
    /// slice position is rejected by `check_slice_element_gate` (the audit
    /// never walks `module.slices`); the gate is the only thing holding that
    /// position -- measured, deleting it reaches `ir_type_of` and ICEs.
    #[test]
    fn owning_quotation_element_is_rejected() {
        // Array: rejected by the audit as a quotation type in an illegal
        // position.
        let arr_err = check_src(": f ( array[owning [ -- ] 4] -- ) drop ;\n").unwrap_err();
        assert!(
            arr_err.contains("owning [ -- ]")
                && arr_err.contains("cannot appear as an array element"),
            "unexpected message for array case: {arr_err}"
        );

        // Slice: rejected by `check_slice_element_gate` as a linear element.
        let slice_err = check_src(": f ( Slice[owning [ -- ]] -- ) drop ;\n").unwrap_err();
        assert!(
            slice_err.contains("owning [ -- ]")
                && slice_err.contains("is linear and has no `Copy` instance"),
            "unexpected message for slice case: {slice_err}"
        );
    }

    /// P7.S3h: the two declaration positions phase 3 made legal -- a word's
    /// declared `owning` input and its declared `owning` output -- reach
    /// `ir_type_of` through signature lowering, so "it checks" is the whole
    /// point here.
    #[test]
    fn declared_owning_quotation_positions_are_accepted() {
        for src in [
            ": f ( owning [ -- ] -- ) call ;\n",
            ": mk ( -- owning [ -- ] ) [ ] ;\n",
        ] {
            check_src(src).unwrap_or_else(|e| panic!("`{src}` should check: {e}"));
        }
    }

    /// P7.S3h: the two routes that never materialize a quotation argument, and
    /// so cannot honour the flavour the type declares.
    ///
    /// A spliced (`inline`) word inlines the caller's literal in place: the
    /// splice route compares only the inline-versus-plain axis, so with this
    /// rejection stubbed out a plain `[ 1 . ]` literal satisfies an `owning`
    /// slot and builds. A generic word's call site materializes off
    /// `CallInst::quot_inputs`, which records the effect and not the flavour,
    /// so it would build a plain closure's frame env for an owning parameter.
    /// Both are what keeps `OwningQuotation(e) != Quotation(e)` load-bearing.
    #[test]
    fn an_owning_parameter_is_rejected_where_it_would_never_be_materialized() {
        let spliced = check_src(": f inline ( owning [ -- ] -- ) | q | q call ;\n").unwrap_err();
        assert!(
            spliced.contains("`f` is spliced (`inline`) and declares `owning [ -- ]`"),
            "unexpected message: {spliced}"
        );
        // The generic body has to be *well-formed* to reach this guard, which
        // runs after `check_types`: a body that `call`s the parameter directly
        // is rejected by the poly walk first, and one that forgets it by the
        // inherited linear check. Forwarding it to a monomorphic consumer is
        // the shape that gets all the way through.
        let generic = check_src(
            ": use ( owning [ -- ] -- ) call ;\n\
             : g ['T: Copy] ( 'T owning [ -- ] -- 'T ) | x q | q use x ;\n",
        )
        .unwrap_err();
        assert!(
            generic.contains("`g` is generic and declares `owning [ -- ]`"),
            "unexpected message: {generic}"
        );
    }
}
