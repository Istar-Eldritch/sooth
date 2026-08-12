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
        audit_word_quotation_positions(w)?;
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
            for (fname, fty) in &v.fields {
                reject_quotation_type_position(
                    *fty,
                    &format!(
                        "the field `{fname}` of enum variant `{}::{}`",
                        e.name, v.name
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
pub(crate) fn audit_word_quotation_positions(w: &WordDef) -> Result<(), String> {
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
    Ok(())
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
