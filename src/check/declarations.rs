//! Module-level declaration checks: `extern:` redeclaration/boundary rules,
//! exported-signature and selective-import validation, the combined
//! struct/enum type-level checks (duplicate names, recursion, no-stored-
//! reference, no-linear-array-element), duplicate/overlap checks over the
//! word registry, and the struct/enum generated-word signature synthesis.
//! Engine-independent: none of this touches `Ctx`, `Scope`, or `Moves`.

use std::collections::{HashMap, HashSet};

use super::*;

/// R1/R2/R3/R7/R12/R13/R14: every `extern:` declaration's own checks, run
/// before its signature enters the word environment. R1's redeclaration
/// check runs against the name-dispatched builtins (`BUILTIN_WORDS`),
/// `existing` (`builtin_table`'s seed, empty today, plus the
/// struct/enum-generated words), the user's own `:` words, and every other
/// `extern:` (in that order, first match wins); R2/R3 reject each forbidden
/// boundary type at the declaration rather than at a call site; the
/// output-reference rejection reuses `check_reference_free_signature`'s
/// existing message rather than duplicating it (R3).
///
/// R14: the symbol's *shape* is checked in the parser, but nothing checks
/// that it *exists* — that needs a symbol table the compiler has no access
/// to, so a misspelled symbol is a `cc` linker error, not a diagnostic.
pub(super) fn check_extern_decls(
    externs: &[ExternDecl],
    words: &[WordDef],
    existing: &HashMap<String, Vec<Overload>>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let mut seen: HashSet<&str> = HashSet::new();
    for decl in externs {
        if is_builtin_word_name(decl.name.as_str()) {
            return Err(extern_redeclaration_error(decl));
        }
        if existing.contains_key(decl.name.as_str()) {
            return Err(extern_redeclaration_error(decl));
        }
        if words.iter().any(|w| w.name == decl.name) {
            return Err(extern_redeclaration_error(decl));
        }
        if !seen.insert(decl.name.as_str()) {
            return Err(extern_redeclaration_error(decl));
        }
        if decl.effect.outputs.len() > 1 {
            return Err(extern_multi_output_error(decl));
        }
        check_reference_free_signature(&decl.name, &decl.effect, structs, enums, arrays)?;
        check_extern_boundary_types(decl)?;
    }
    Ok(())
}

/// R1: the builtin words `check_term` dispatches by name, in its probe chain,
/// *before* the word environment is consulted at all. They are absent from
/// `builtin_table` (empty today, since every builtin dispatches on the
/// concrete operand type rather than a fixed signature), so an `extern:`
/// naming one would be registered, never looked up, and silently do nothing. The `^`-led owning-cell words and the `@`/`!`/`+!` access
/// words are dispatched in the same chain but are rejected earlier, against
/// the declaration's name in the parser, so they are not repeated here.
pub(super) const BUILTIN_WORDS: &[&str] = &[
    // check_shuffle
    "dup",
    "drop",
    "swap",
    "over",
    "rot", // check_operator
    "+",
    "-",
    "*",
    "/",
    "mod",
    "and",
    "or",
    "xor",
    "not",
    "shl",
    "shr",
    "=",
    "<",
    ">",
    "<=",
    ">=",
    "<>",
    ".",
    "max",
    "max-total", // check_str_word
    "len",
    "cstr", // check_array_word (`len` is shared with `check_str_word`)
    "fill",
];

/// R1: whether `name` is dispatched as a builtin ahead of any environment
/// lookup. Beyond the fixed names, `check_operator` claims every `>`-prefixed
/// name with a non-empty remainder as a numeric conversion (`>u8`), erroring
/// on an unrecognised target type rather than falling through, so no such
/// name can reach a registered signature either. Bare `>` is the comparison
/// operator, and is in the list.
pub(super) fn is_builtin_word_name(name: &str) -> bool {
    BUILTIN_WORDS.contains(&name) || name.strip_prefix('>').is_some_and(|rest| !rest.is_empty())
}

/// R1: a located error for an `extern:` declaration redeclaring a name
/// already registered as a builtin, a user `:` word, or another `extern:`.
fn extern_redeclaration_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` redeclares an existing word (line {}, col {})",
        decl.name, decl.span.line, decl.span.col
    )
}

/// R8 (slice 8b): no C function returns two values, so a declared output
/// arity above one describes no callable prototype. Left unrejected it lowers
/// to a discarded result (`lower_call` binds a return only for `out_arity ==
/// 1`) and panics in the *next* consumer of the value that was never pushed,
/// which points at the wrong term entirely.
fn extern_multi_output_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` declares {} outputs (line {}, col {})\n  no C function returns more than one value; declare at most one output",
        decl.name,
        decl.effect.outputs.len(),
        decl.span.line,
        decl.span.col
    )
}

/// R2: the boundary type set an `extern:` slot may use in either position —
/// the numeric tower, `bool`, `&T`/`&!T`, and `cstr`. Each is either a scalar
/// or an opaque `Ptr` the backend already passes across a call.
///
/// `str` is excluded despite R2's list naming it, on R2's own criterion: R4
/// makes it a descriptor handle, not a scalar or a single opaque `Ptr`, so C
/// would receive a pointer to a descriptor rather than a `char*`. See
/// `extern_str_input_error`/`extern_str_output_error` for each direction.
fn is_extern_boundary_scalar(ty: Type) -> bool {
    matches!(
        ty,
        Type::Int(_)
            | Type::Float(_)
            | Type::BOOL
            | Type::Usize
            | Type::Isize
            | Type::Ref(..)
            | Type::Cstr
    )
}

/// R3: each `extern:` boundary-type rejection not already covered by
/// `check_reference_free_signature` (which independently rejects any
/// reference-containing output before this ever runs). An owned aggregate
/// (struct/enum/array/`^T`) is rejected in either position: ownership across
/// the FFI boundary has no answer and no client. A `^T` specifically in
/// output position gets its own message, since forging ownership of memory
/// the allocator did not hand out is a sharper reason than the generic one.
fn check_extern_boundary_types(decl: &ExternDecl) -> Result<(), String> {
    for slot in &decl.effect.inputs {
        if matches!(slot.ty, Type::Str) {
            return Err(extern_str_input_error(decl));
        }
        if !is_extern_boundary_scalar(slot.ty) {
            return Err(extern_owned_aggregate_error(decl, slot.ty, "input"));
        }
    }
    for slot in &decl.effect.outputs {
        if is_extern_boundary_scalar(slot.ty) {
            continue;
        }
        if matches!(slot.ty, Type::Str) {
            return Err(extern_str_output_error(decl));
        }
        if matches!(slot.ty, Type::OwnedCell(..)) {
            return Err(extern_owned_pointer_output_error(decl, slot.ty));
        }
        return Err(extern_owned_aggregate_error(decl, slot.ty, "output"));
    }
    Ok(())
}

/// R2/R7: a `str` input has no C prototype (R4 makes it a descriptor handle,
/// not a scalar or a single opaque `Ptr`, so C would receive a pointer to a
/// descriptor rather than a `char*`), and the conversion that gives it one is
/// total — `cstr` is sound for every `str` under R11's static-rooting, a
/// literal being the only constructor — so the rejection names it.
fn extern_str_input_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` declares the input `str` (line {}, col {})\n  a `str` is a pointer and a length, which matches no C parameter; declare `cstr` and convert with `cstr` at the call site",
        decl.name, decl.span.line, decl.span.col
    )
}

/// R11: a returned `str` would be a `str` not built from a literal, which is
/// the invariant R10's `Copy`/non-escaping status rests on. C supplies no
/// length either, so there is nothing to build one from.
fn extern_str_output_error(decl: &ExternDecl) -> String {
    format!(
        "error: `extern: {}` cannot return a `str` (line {}, col {})\n  a `str` may point at static data only, and C supplies no length; declare `cstr`",
        decl.name, decl.span.line, decl.span.col
    )
}

fn extern_owned_aggregate_error(decl: &ExternDecl, ty: Type, position: &str) -> String {
    format!(
        "error: `extern: {}` declares the {position} `{}`, an owned aggregate (line {}, col {})\n  ownership across the C boundary has no answer and no client; only the numeric tower, `&T`/`&!T`, and `cstr` may cross",
        decl.name, ty, decl.span.line, decl.span.col
    )
}

fn extern_owned_pointer_output_error(decl: &ExternDecl, ty: Type) -> String {
    format!(
        "error: `extern: {}` cannot return the owned pointer `{}` (line {}, col {})\n  it would forge ownership of memory the allocator did not hand out",
        decl.name, ty, decl.span.line, decl.span.col
    )
}

/// R18 (phase 4 slice 5a phase 3): an exported word whose stack effect names
/// a non-primitive type of its own module that is not itself exported is a
/// declaration-site error naming the word and the private type. R15 makes a
/// type and its generated words one exported unit, so exporting the type
/// clears every word of its own module that mentions it. Runs on the raw,
/// pre-mangle module the driver assembles (`driver::assemble_module`),
/// before `resolve::resolve_modules` renames decls: the check matches a
/// word's raw name against its own module's raw `export:` list, and both
/// would already be mangled by the time `check::check` runs.
pub fn check_exported_signatures(module: &Module) -> Result<(), String> {
    for word in &module.words {
        let exports = match module.modules.get(word.module as usize) {
            Some(m) => &m.exports,
            None => continue,
        };
        if !exports.iter().any(|(n, _)| n == &word.name) {
            continue;
        }
        for ty in effect_types(word) {
            if let Some(name) = private_type_name(ty, word.module, module) {
                return Err(exported_word_names_private_type_error(word, name));
            }
        }
    }
    Ok(())
}

/// Every concrete `Type` a word's declared effect mentions: its ordinary
/// input/output slots, plus, for a polymorphic word, every `Concrete` leaf
/// its `PolySig` mentions (a type variable itself names no type, so `Var`
/// contributes nothing).
fn effect_types(word: &WordDef) -> Vec<Type> {
    let mut out: Vec<Type> = word
        .effect
        .inputs
        .iter()
        .chain(&word.effect.outputs)
        .map(|slot| slot.ty)
        .collect();
    if let Some(sig) = &word.poly {
        for t in sig.inputs.iter().chain(&sig.outputs) {
            collect_poly_concrete(t, &mut out);
        }
    }
    out
}

fn collect_poly_concrete(t: &PolyType, out: &mut Vec<Type>) {
    match t {
        PolyType::Concrete(ty) => out.push(*ty),
        PolyType::Var(_) => {}
        PolyType::Array(elem, _) => collect_poly_concrete(elem, out),
        // Slice 6a (R5): a declared quotation effect's rows may name concrete
        // types (`[ i64 -- ]`); collect them so export-privacy still sees a
        // private type mentioned inside an effect.
        PolyType::Quotation(ins, outs, _, _, _) => {
            for t in ins.iter().chain(outs) {
                collect_poly_concrete(t, out);
            }
        }
    }
}

/// Whether `ty` is a struct/enum owned by `owner_module` and absent from that
/// module's `export:` list, i.e. the R18 violation. A type owned by a
/// *different* module is not this rule's problem (R16 already gates whether
/// it could even be named here), and a primitive/array/etc. names no
/// declared type at all.
fn private_type_name(ty: Type, owner_module: u32, module: &Module) -> Option<&'static str> {
    // Slice 9 (R2): `bool` is the builtin zero-payload enum injected
    // identically (with `module: 0`) into every assembled module's registry
    // (`bool_enum_decl`, `BOOL_ENUM_ID`); it is never a user-declared,
    // ownable type, so it can never trip the private-type export rule --
    // otherwise any module whose own `owner_module` happens to be `0` would
    // wrongly see its own builtin `bool` usage as an unexported private type.
    if ty == Type::BOOL {
        return None;
    }
    let (decl_module, name) = match ty {
        Type::Struct(id, name) => (module.structs[id.index()].module, name),
        Type::Enum(id, name) => (module.enums[id.index()].module, name),
        _ => return None,
    };
    if decl_module != owner_module {
        return None;
    }
    let exports = &module.modules[decl_module as usize].exports;
    if exports.iter().any(|(n, _)| n == name) {
        return None;
    }
    Some(name)
}

/// R18: a located error naming the exported word and the private type its
/// effect mentions. Exporting the type satisfies the rule.
fn exported_word_names_private_type_error(word: &WordDef, type_name: &str) -> String {
    let span = word_span(word);
    format!(
        "error: exported word `{}` (line {}, col {}) names private type `{}`, which is not exported\n  export `{}` too, or remove it from the effect",
        word.name, span.line, span.col, type_name, type_name
    )
}

/// Phase 4 slice 5a phase 4 (R20/R15c): one selectively-imported name, carried
/// from the driver's closure assembly with the qualifier and target module it
/// came from and the span of the name in the `import:` form, for the R20/R21
/// validation. A type name exposes its generated words as one unit (R15c), so
/// only the base name appears here; a member (`Type>field`) can only collide
/// when its base does.
pub struct SelectiveName {
    pub name: String,
    pub qualifier: String,
    pub target: u32,
    pub span: Span,
}

/// R20/R21: validate every module's selective imports on the raw, pre-mangle
/// module. Each listed name must be exported by its source module (R20, the
/// R16 visibility error). No two selective imports may expose the same
/// unqualified name, and a selective name may not collide with one of the
/// importing module's own words or types (R21, a located error at the second
/// source naming both). The collision is decided on the base name because a
/// selectively imported type and its generated words are one unit (R15c) and a
/// member name collides only when its base does.
pub fn check_selective_imports(
    module: &Module,
    selective_by_module: &[Vec<SelectiveName>],
) -> Result<(), String> {
    let builtins = builtin_table();
    for (m, entries) in selective_by_module.iter().enumerate() {
        let locals = local_decl_names(module, m as u32);
        // name -> the qualifier that first exposed it, for R21's both-sources error.
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for entry in entries {
            let exports = &module.modules[entry.target as usize].exports;
            if !exports.iter().any(|(n, _)| n == &entry.name) {
                return Err(selective_not_exported_error(
                    &entry.name,
                    &entry.qualifier,
                    entry.span,
                ));
            }
            if locals.contains(entry.name.as_str()) {
                return Err(selective_collides_with_local_error(
                    &entry.name,
                    &entry.qualifier,
                    entry.span,
                ));
            }
            if let Some(first) = seen.insert(entry.name.as_str(), entry.qualifier.as_str()) {
                return Err(selective_collision_error(
                    &entry.name,
                    first,
                    &entry.qualifier,
                    entry.span,
                ));
            }
            // R4 (import mirror): a selective import naming a builtin
            // operator must agree with it on input count. The other two
            // collision checks above already forbid an import from sharing a
            // name with a local decl or with another selective import
            // outright, so a builtin row is the only candidate this site can
            // still silently disagree with.
            if let Some(rows) = builtins.get(entry.name.as_str()) {
                let builtin_arity = rows[0].inputs.len();
                if let Some(imported) = module
                    .words
                    .iter()
                    .find(|w| w.module == entry.target && w.name == entry.name && w.poly.is_none())
                {
                    let arity = imported.effect.inputs.len();
                    if arity != builtin_arity {
                        return Err(selective_arity_clash_error(
                            &entry.name,
                            &entry.qualifier,
                            entry.span,
                            arity,
                            builtin_arity,
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Every raw decl name owned by module `m`: its structs, enums, words, and
/// externs, for R21's selective-vs-local collision check. Runs pre-mangle, so
/// the names are the source spellings a selective import would collide with.
fn local_decl_names(module: &Module, m: u32) -> HashSet<&str> {
    let mut names = HashSet::new();
    for s in &module.structs {
        if s.module == m {
            names.insert(s.name.as_str());
        }
    }
    for e in &module.enums {
        if e.module == m {
            names.insert(e.name.as_str());
        }
    }
    for w in &module.words {
        if w.module == m {
            names.insert(w.name.as_str());
        }
    }
    for x in &module.externs {
        if x.module == m {
            names.insert(x.name.as_str());
        }
    }
    names
}

/// R20: a selectively imported name absent from its source module's `export:`
/// list is the R16 visibility error, same wording as a qualified private
/// reference.
pub(crate) fn selective_not_exported_error(name: &str, qualifier: &str, span: Span) -> String {
    format!(
        "error: `{name}` is not exported from module `{qualifier}` at line {}, col {}",
        span.line, span.col
    )
}

/// R21: a second selective import exposing a name a prior one already exposed,
/// naming both source modules. No precedence, no shadowing: the collision is
/// the error.
pub(crate) fn selective_collision_error(
    name: &str,
    first: &str,
    second: &str,
    span: Span,
) -> String {
    format!(
        "error: selective import of `{name}` from module `{second}` (line {}, col {}) collides with the selective import of `{name}` from module `{first}`",
        span.line, span.col
    )
}

/// R21: a selective import exposing a name the importing module already defines
/// locally, naming the source module and the local definition.
pub(crate) fn selective_collides_with_local_error(
    name: &str,
    qualifier: &str,
    span: Span,
) -> String {
    format!(
        "error: selective import of `{name}` from module `{qualifier}` (line {}, col {}) collides with a local definition of `{name}`",
        span.line, span.col
    )
}

/// R4 (import mirror): a selective import of a builtin-named word whose input
/// count disagrees with the builtin's -- the arity clash rejected at the
/// second candidate's site (`overload_arity_clash_error`'s local twin), here
/// the import site rather than a definition site.
fn selective_arity_clash_error(
    name: &str,
    qualifier: &str,
    span: Span,
    arity: usize,
    builtin_arity: usize,
) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!(
        "error: selective import of `{name}` from module `{qualifier}` (line {}, col {}) takes {arity} input{} but the builtin `{name}` takes {builtin_arity}; all overloads of a name must agree on input count",
        span.line,
        span.col,
        plural(arity),
    )
}

/// Type-level checks that must pass before any generated-word signature or
/// word body is type-checked: no two `type:` declarations share a name across
/// the combined struct+enum registries, and no struct or enum contains itself
/// by value, directly or transitively, through the combined type graph (D9,
/// D10, R8, R10).
pub fn check_types(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
) -> Result<(), String> {
    check_duplicate_type_names(structs, enums)?;
    check_recursion(structs, enums, arrays)?;
    check_no_stored_references(structs, enums, arrays, cells)?;
    check_no_linear_array_elements(structs, enums, arrays)?;
    Ok(())
}

/// The declaration-site half of the no-stored-reference rule: a struct field,
/// an enum variant payload field,
/// an interned array element, or an interned cell payload whose type
/// transitively contains a reference is a located error. Runs after
/// `check_recursion`, so the field-graph walk `contains_reference` performs is
/// guaranteed acyclic. The two *construction* sites (`fill`'s element, `^`'s
/// payload) are rejected separately in the body walk: both accept whatever
/// type is on the stack with no declaration in sight.
fn check_no_stored_references(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
) -> Result<(), String> {
    for decl in structs {
        for (field, ty) in &decl.fields {
            if contains_reference(*ty, structs, enums, arrays) {
                return Err(stored_reference_error(
                    &format!("field `{field}` of type `{}`", decl.name),
                    *ty,
                    Some(decl.span),
                ));
            }
        }
    }
    for decl in enums {
        for variant in &decl.variants {
            for (field, ty) in &variant.fields {
                if contains_reference(*ty, structs, enums, arrays) {
                    return Err(stored_reference_error(
                        &format!(
                            "payload field `{field}` of variant `{}` of type `{}`",
                            variant.name, decl.name
                        ),
                        *ty,
                        Some(variant.span),
                    ));
                }
            }
        }
    }
    for decl in arrays {
        if contains_reference(decl.element, structs, enums, arrays) {
            return Err(stored_reference_error(
                &format!("element of array type `{}`", decl.name_static),
                decl.element,
                None,
            ));
        }
    }
    for decl in cells {
        if contains_reference(decl.payload, structs, enums, arrays) {
            return Err(stored_reference_error(
                &format!("payload of cell type `{}`", decl.name_static),
                decl.payload,
                None,
            ));
        }
    }
    Ok(())
}

/// The one wording every escape rejection shares. `position` names the
/// storage slot the reference tried to reach; an array or cell shape has no
/// declared name and so no span to cite.
fn stored_reference_error(position: &str, ty: Type, span: Option<Span>) -> String {
    let located = match span {
        Some(span) => format!(" (line {}, col {})", span.line, span.col),
        None => String::new(),
    };
    format!(
        "error: a reference cannot be stored: {position} has type `{ty}`{located}\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow"
    )
}

/// Arrays of linear elements are not supported yet: rejected here, over the
/// module's interned array registry, rather than in the parser, because
/// linearity (`is_copy`) is only answerable once every struct/enum field list
/// is resolved, which happens after the whole module is parsed. Every array
/// type named anywhere (a word signature slot, a struct field, an enum
/// variant field) is interned into this one registry, and `is_copy` already
/// walks an array's element transitively, so this single sweep catches a
/// direct `[LinearStruct N]` and an indirect one alike. Runs after
/// `check_recursion`, which rules out a self-referential struct/enum/array
/// first, so `is_copy`'s recursion over the field graph is guaranteed to
/// terminate. `ArrayDecl` carries no span (an array shape has no declared
/// name a pre-pass could register), so the error names the array/element
/// types rather than inventing a wrong line number.
fn check_no_linear_array_elements(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    for decl in arrays {
        if !is_copy(decl.element, structs, enums, arrays) {
            return Err(format!(
                "error: linear array elements are not supported yet: array type `{}` has element `{}`, which is linear and has no `Copy` instance",
                decl.name_static,
                decl.element.name(),
            ));
        }
    }
    Ok(())
}

/// The struct-only projection of `check_types` (no enums/arrays), for callers
/// that don't yet declare either.
pub fn check_structs(structs: &[StructDecl]) -> Result<(), String> {
    check_types(structs, &[], &[], &[])
}

/// A duplicate `type:` name is a sharp located error naming the type. R12
/// (phase 4 slice 5a): the check is per-module, so two modules each declaring
/// `Point` is not a duplicate. Keyed by `(module, name_static)`: `name_static`
/// stays the raw surface name even after the resolver mangles `name` for
/// symbol disambiguation, so the error still reads `Point`, not `Point__m1`.
fn check_duplicate_struct_names(structs: &[StructDecl]) -> Result<(), String> {
    let mut seen: HashMap<(u32, &str), ()> = HashMap::new();
    for decl in structs {
        if seen.insert((decl.module, decl.name_static), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name_static, decl.span.line, decl.span.col
            ));
        }
    }
    Ok(())
}

/// A duplicate type name across the *combined* struct + enum registries
/// (D10, X2) is a sharp located error naming the type: a name used by two
/// structs, two enums, or one of each. Delegates the struct-only pass to
/// `check_duplicate_struct_names` (also called directly by struct-only
/// callers, e.g. the REPL, which doesn't yet declare enums) rather than
/// re-scanning `structs` twice.
pub(super) fn check_duplicate_type_names(
    structs: &[StructDecl],
    enums: &[EnumDecl],
) -> Result<(), String> {
    check_duplicate_struct_names(structs)?;
    let mut seen: HashMap<(u32, &str), ()> = structs
        .iter()
        .map(|decl| ((decl.module, decl.name_static), ()))
        .collect();
    for decl in enums {
        if seen.insert((decl.module, decl.name_static), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name_static, decl.span.line, decl.span.col
            ));
        }
    }
    Ok(())
}

/// A duplicate word name within one module leaks past every existing check
/// straight to the linker's bare `symbol already defined` error: nothing
/// before this rejects a repeat `word.name` the way `check_duplicate_type_names`
/// already does for structs/enums, so the word-environment population loop in
/// `check` silently keeps only the last one seen and both bodies still lower
/// to codegen. Keyed by `(module, name)`, mirroring that check exactly, so two
/// modules each declaring `push` is not a duplicate (`resolve::mangle` already
/// disambiguates that pair's symbols; by the time this runs post-`resolve`,
/// their `name`s already differ) while two `push`es in one module still
/// mangle identically and collide here.
///
/// `drop`-named words are skipped entirely, not treated as exempt from the
/// rule: `find_drop_overloads` (run earlier, unconditionally, as the first
/// step of `check`) already owns every `drop` word's multiplicity, keyed by
/// the struct id it overrides rather than by the shared literal name `"drop"`
/// -- two overloads for two distinct structs are not a duplicate and must
/// coexist, while a second overload for the *same* struct already failed
/// there, before this ever runs. Re-checking `drop` here by name alone would
/// reject that legitimate multi-type overloading (Phase 3 slice 8b) as a
/// false positive. `main` gets no such carve-out: nothing else validates a
/// repeat `main` within one module, so it is an ordinary word for this check.
pub(super) fn check_duplicate_word_names(words: &[WordDef]) -> Result<(), String> {
    let builtins = builtin_table();
    // R1: keyed by `(module, name, input_types)`, widened from `(module, name)`
    // so two overloads of one name that differ in input type no longer
    // collide; two with identical inputs still hit the existing `duplicate
    // word` message byte-for-byte. Linear (not hashed): `Type` carries no
    // `Hash`, and a module's word count never approaches a scale where that
    // matters.
    let mut seen: Vec<(u32, &str, Vec<Type>, Span)> = Vec::new();
    // R4: one arity per name in scope. Seeded with each builtin row's arity
    // (rows for one name already agree on arity, `builtin_table`'s own
    // invariant), since a builtin candidate is always in scope; a local
    // overload is the "second candidate" the moment it disagrees.
    let mut arities: HashMap<(u32, &str), (usize, Span)> = HashMap::new();
    for word in words {
        if word.name == "drop" {
            continue;
        }
        // R5 owns generic/concrete overlap; a poly word's `effect` is empty
        // by construction (its signature lives in `poly`), so it has no
        // concrete input types to key or count here.
        if word.poly.is_some() {
            continue;
        }
        let span = word_span(word);
        let input_types: Vec<Type> = word.effect.inputs.iter().map(|s| s.ty).collect();
        if let Some((.., first)) = seen
            .iter()
            .find(|(m, n, ins, _)| *m == word.module && *n == word.name && *ins == input_types)
        {
            let first = *first;
            return Err(format!(
                "error: duplicate word `{}` (line {}, col {}); first defined at line {}, col {}",
                crate::resolve::demangle_word(&word.name),
                span.line,
                span.col,
                first.line,
                first.col
            ));
        }
        if let Some(rows) = builtins.get(word.name.as_str()) {
            if rows.iter().any(|r| r.inputs == input_types) {
                return Err(overload_matches_builtin_error(
                    &word.name,
                    span,
                    &input_types,
                ));
            }
        }
        let arity = input_types.len();
        if let Some(&(first_arity, _)) = arities.get(&(word.module, word.name.as_str())) {
            if first_arity != arity {
                return Err(overload_arity_clash_error(
                    &word.name,
                    span,
                    arity,
                    first_arity,
                ));
            }
        } else {
            let builtin_arity = builtins
                .get(word.name.as_str())
                .map(|rows| rows[0].inputs.len());
            if let Some(builtin_arity) = builtin_arity {
                if builtin_arity != arity {
                    return Err(overload_arity_clash_error(
                        &word.name,
                        span,
                        arity,
                        builtin_arity,
                    ));
                }
            }
            arities.insert((word.module, word.name.as_str()), (arity, span));
        }
        seen.push((word.module, word.name.as_str(), input_types, span));
    }
    Ok(())
}

/// R1: a definition whose `(name, input_types)` exactly matches a builtin
/// row. Distinct from the plain `duplicate word` message: there is no first
/// *definition* to cite, only the builtin's operand types.
fn overload_matches_builtin_error(name: &str, span: Span, input_types: &[Type]) -> String {
    let types = input_types
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "error: overload of `{}` (line {}, col {}) has the same input types ({}) as a builtin",
        crate::resolve::demangle_word(name),
        span.line,
        span.col,
        types
    )
}

/// R4: two candidates for one name (local overloads, or a local overload
/// against a builtin row) that disagree on input count. Rejected at the
/// second candidate's site, never resolved by call-site ranking; the other
/// candidate may be a builtin, which has no declaration site to cite, so the
/// message names it rather than locating it.
fn overload_arity_clash_error(name: &str, span: Span, arity: usize, other_arity: usize) -> String {
    let name = crate::resolve::demangle_word(name);
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!(
        "error: overload of `{name}` (line {}, col {}) takes {arity} input{} but another `{name}` takes {other_arity}; all overloads of a name must agree on input count",
        span.line,
        span.col,
        plural(arity),
    )
}

/// R5: a name with both a generic (poly) candidate and a concrete candidate
/// (a builtin row or a local monomorphic word) of the same input arity is
/// rejected -- there is no specialization ordering that could otherwise pick
/// between `: + ( 'T 'T -- 'T )` and `: + ( i64 i64 -- i64 )` (or a builtin
/// concrete row) at a call site. A poly word's `effect` is empty by
/// construction, so R1's textual key never sees it; this is why the check is
/// separate rather than folded into `check_duplicate_word_names`.
///
/// Module-scoped like `check_duplicate_word_names`' `(module, name)` key: a
/// builtin candidate is in scope everywhere (no module), but a local concrete
/// candidate only overlaps a poly candidate declared in *its own* module. An
/// unrelated same-named concrete word in a different, non-importing module is
/// invisible here by construction; a selectively-imported name that collides
/// with a local poly word is already rejected generically by
/// `check_selective_imports`' local-collision check, regardless of poly vs.
/// concrete, so that case needs no mirror here.
pub(super) fn check_generic_concrete_overlap(words: &[WordDef]) -> Result<(), String> {
    let builtins = builtin_table();
    let mut builtin_arity: HashMap<&str, usize> = HashMap::new();
    for (name, rows) in builtins.iter() {
        builtin_arity.insert(name.as_str(), rows[0].inputs.len());
    }
    let mut concrete_arity: HashMap<(u32, &str), usize> = HashMap::new();
    for word in words {
        if word.name != "drop" && word.poly.is_none() {
            concrete_arity
                .entry((word.module, word.name.as_str()))
                .or_insert_with(|| word.effect.inputs.len());
        }
    }
    for word in words {
        let Some(sig) = &word.poly else { continue };
        let arity = sig.inputs.len();
        let overlaps = builtin_arity.get(word.name.as_str()) == Some(&arity)
            || concrete_arity.get(&(word.module, word.name.as_str())) == Some(&arity);
        if overlaps {
            return Err(generic_concrete_overlap_error(
                &word.name,
                sig,
                word_span(word),
            ));
        }
    }
    Ok(())
}

/// Deferred from round 3's slice 8a review: two poly words (or two poly
/// combinators -- `is_combinator` doesn't discriminate mono vs poly) sharing
/// a name and declaring the exact same signature were legal to define, and
/// resolved to whichever was declared first, the second silently dead rather
/// than reachable-but-wrong. Unlike every other duplicate/overlap case
/// R1/R4/R5 already cover, that traded a clear error for a confusing one --
/// the exact failure this whole slice exists to close. Module-scoped like
/// R5's own key: a poly word only collides with an identically-signed poly
/// word in its own module.
///
/// Structural, not textual: a signature's variable id space is per-signature
/// (`PolySig`'s own doc), and `intern_ty_var`/`intern_len_var` number a
/// variable by first-appearance order, so two declarations spelling an
/// otherwise-identical shape with different variable names (`'T 'T -- 'T`
/// vs. `'U 'U -- 'U`) still produce identical ids and compare equal here --
/// alpha-equivalence, not merely textual identity. `*_var_names` (surface
/// spelling only, irrelevant to what the signature actually accepts) is
/// deliberately excluded from the comparison. Scoped to the native build
/// path only, matching `check_generic_concrete_overlap`'s own existing
/// scope: neither runs at the REPL, where per-definition poly checking
/// stays out of this slice's exit criteria (crashing was never licensed;
/// a missing duplicate-signature diagnostic there is a pre-existing gap
/// this slice doesn't widen).
pub(super) fn check_duplicate_poly_signatures(words: &[WordDef]) -> Result<(), String> {
    let mut seen: Vec<(u32, &str, &PolySig, Span)> = Vec::new();
    for word in words {
        let Some(sig) = &word.poly else { continue };
        if let Some((.., first)) = seen
            .iter()
            .find(|(m, n, s, _)| *m == word.module && *n == word.name && poly_sig_shape_eq(s, sig))
        {
            let first = *first;
            return Err(duplicate_poly_signature_error(
                &word.name,
                sig,
                word_span(word),
                first,
            ));
        }
        seen.push((word.module, word.name.as_str(), sig, word_span(word)));
    }
    Ok(())
}

/// The comparison `check_duplicate_poly_signatures` needs: every field that
/// determines what the signature actually accepts (not `*_var_names`, the
/// surface spelling).
fn poly_sig_shape_eq(a: &PolySig, b: &PolySig) -> bool {
    a.row_in == b.row_in
        && a.inputs == b.inputs
        && a.outputs == b.outputs
        && a.row_out == b.row_out
        && a.bounds == b.bounds
}

fn duplicate_poly_signature_error(name: &str, sig: &PolySig, span: Span, first: Span) -> String {
    format!(
        "error: duplicate overload `{}` (line {}, col {}); another overload of `{}` already declares this exact signature at line {}, col {}",
        poly_sig_str(name, sig),
        span.line,
        span.col,
        crate::resolve::demangle_word(name),
        first.line,
        first.col,
    )
}

/// R5: render the poly candidate's whole declared signature (`: + ( 'T 'T --
/// 'T )`), naming it and the concrete overload it overlaps.
fn generic_concrete_overlap_error(name: &str, sig: &PolySig, span: Span) -> String {
    format!(
        "error: generic overload `{}` (line {}, col {}) overlaps a concrete overload of `{}`; a name cannot mix a generic and a concrete candidate",
        poly_sig_str(name, sig),
        span.line,
        span.col,
        crate::resolve::demangle_word(name),
    )
}

/// Render a poly word's whole declared signature for a diagnostic (R5's
/// overlap error is the only caller that needs the full `( ins -- outs )`
/// shape rather than one `PolyType`, which `poly_type_str` already renders).
pub(super) fn poly_sig_str(name: &str, sig: &PolySig) -> String {
    let render_row = |types: &[PolyType], row_var: Option<u32>| {
        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = row_var {
            parts.push(sig.row_var_names[v as usize].clone());
        }
        parts.extend(types.iter().map(|t| poly_type_str(t, sig)));
        parts.join(" ")
    };
    let ins = render_row(&sig.inputs, sig.row_in);
    let outs = render_row(&sig.outputs, sig.row_out);
    let body = match (ins.is_empty(), outs.is_empty()) {
        (true, true) => "--".to_string(),
        (true, false) => format!("-- {outs}"),
        (false, true) => format!("{ins} --"),
        (false, false) => format!("{ins} -- {outs}"),
    };
    format!(": {} ( {body} )", crate::resolve::demangle_word(name))
}

/// Whether a struct's field-type graph node has been visited by
/// `check_struct_recursion`'s DFS: `InProgress` marks an ancestor on the
/// current path (finding one again is a cycle), `Done` marks a node already
/// proven acyclic. Every node is visited at most once each way, so the DFS
/// always terminates: it never loops on a self- or mutually-recursive
/// `type:`.
#[derive(Clone, Copy, PartialEq)]
enum VisitState {
    Unvisited,
    InProgress,
    Done,
}

/// A node in the combined struct+enum value-containment graph (D9, R10): a
/// struct or an enum, by registry index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeNode {
    Struct(usize),
    Enum(usize),
    Array(usize),
}

/// Detect a struct or enum that contains itself by value, directly or
/// transitively, via cycle detection over the *combined* type graph (D9): a
/// struct's field types and an enum's variant field types are edges, so a
/// struct-of-enum-of-struct cycle is caught the same as a pure-struct one.
fn check_recursion(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Result<(), String> {
    let mut st = RecursionState {
        sstate: vec![VisitState::Unvisited; structs.len()],
        estate: vec![VisitState::Unvisited; enums.len()],
        astate: vec![VisitState::Unvisited; arrays.len()],
        path: Vec::new(),
    };
    for start in 0..structs.len() {
        if st.sstate[start] == VisitState::Unvisited {
            visit_recursion(TypeNode::Struct(start), structs, enums, arrays, &mut st)?;
        }
    }
    for start in 0..enums.len() {
        if st.estate[start] == VisitState::Unvisited {
            visit_recursion(TypeNode::Enum(start), structs, enums, arrays, &mut st)?;
        }
    }
    for start in 0..arrays.len() {
        if st.astate[start] == VisitState::Unvisited {
            visit_recursion(TypeNode::Array(start), structs, enums, arrays, &mut st)?;
        }
    }
    Ok(())
}

/// The per-node visit state + current DFS path, bundled so the traversal
/// signature stays readable now that three registries (struct/enum/array)
/// contribute nodes.
struct RecursionState {
    sstate: Vec<VisitState>,
    estate: Vec<VisitState>,
    astate: Vec<VisitState>,
    path: Vec<TypeNode>,
}

/// The frontend `Type` of a field, mapped to a graph node (a scalar has no
/// edge). By-value containment is the only edge kind this graph models: a
/// struct field, enum variant field or array element of type `T` makes `T`
/// part of the enclosing type's size, so a cycle through any of them is
/// infinite size. `OwnedCell` is excluded **deliberately, not by
/// fall-through**: a `^T` field is a heap pointer, not an inline copy of
/// `T`, so it can close a cycle without making the type infinite, and the
/// recursion rule is exactly "every cycle passes through at least one `^`".
fn type_node(ty: &Type) -> Option<TypeNode> {
    match ty {
        Type::Struct(id, _) => Some(TypeNode::Struct(id.index())),
        Type::Enum(id, _) => Some(TypeNode::Enum(id.index())),
        Type::Array(id, _) => Some(TypeNode::Array(id.index())),
        Type::OwnedCell(_, _) => None,
        // A reference is a pointer, not an inline copy, so it closes no
        // by-value cycle — and the no-stored-reference rule keeps one out of
        // every field position
        // anyway.
        Type::Ref(..) => None,
        // `bool` is `Type::Enum` and so caught by the arm above; a zero-payload
        // enum has no fields, hence no containment edges, so it is a leaf.
        Type::Int(_)
        | Type::Float(_)
        | Type::Usize
        | Type::Isize
        | Type::Str
        | Type::Cstr
        // Slice 6a: a quotation type has no runtime layout (D6), so it is not
        // a value-containment node; like a reference it closes no size cycle.
        // Slice 10a: a `~` is never a field (it cannot be materialized), so it
        // reaches this recursion graph only vacuously; it too is a leaf.
        | Type::Quotation(_)
        | Type::InlineQuotation(_) => None,
    }
}

/// The value-containment edges out of a node: a struct's field types, or every
/// variant field type of an enum.
fn node_edges(
    node: TypeNode,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
) -> Vec<TypeNode> {
    match node {
        TypeNode::Struct(i) => structs[i]
            .fields
            .iter()
            .filter_map(|(_, ty)| type_node(ty))
            .collect(),
        TypeNode::Enum(i) => enums[i]
            .variants
            .iter()
            .flat_map(|v| v.fields.iter())
            .filter_map(|(_, ty)| type_node(ty))
            .collect(),
        // An array's single containment edge is to its element type (M3): a
        // `[T N]` contains a `T` by value, so a cycle through an array element
        // is caught exactly as a struct/enum one, and a nested array bottoms
        // out at a scalar so the DFS terminates.
        TypeNode::Array(i) => type_node(&arrays[i].element).into_iter().collect(),
    }
}

fn node_state(node: TypeNode, st: &mut RecursionState) -> &mut VisitState {
    match node {
        TypeNode::Struct(i) => &mut st.sstate[i],
        TypeNode::Enum(i) => &mut st.estate[i],
        TypeNode::Array(i) => &mut st.astate[i],
    }
}

fn node_name<'a>(
    node: TypeNode,
    structs: &'a [StructDecl],
    enums: &'a [EnumDecl],
    arrays: &'a [ArrayDecl],
) -> &'a str {
    match node {
        TypeNode::Struct(i) => structs[i].name.as_str(),
        TypeNode::Enum(i) => enums[i].name.as_str(),
        TypeNode::Array(i) => arrays[i].name_static,
    }
}

fn visit_recursion(
    node: TypeNode,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    st: &mut RecursionState,
) -> Result<(), String> {
    *node_state(node, st) = VisitState::InProgress;
    st.path.push(node);
    for child in node_edges(node, structs, enums, arrays) {
        match *node_state(child, st) {
            VisitState::Unvisited => visit_recursion(child, structs, enums, arrays, st)?,
            VisitState::InProgress => {
                let cycle_start = st.path.iter().position(|&x| x == child).unwrap();
                let mut names: Vec<&str> = st.path[cycle_start..]
                    .iter()
                    .map(|&n| node_name(n, structs, enums, arrays))
                    .collect();
                names.push(node_name(child, structs, enums, arrays));
                // Key the wording on the repeated node's kind so a pure-struct
                // cycle keeps its Slice 3 message, an enum cycle names an enum
                // (X3), and an array cycle names the array (X5).
                let kind = match child {
                    TypeNode::Struct(_) => "struct",
                    TypeNode::Enum(_) => "enum",
                    TypeNode::Array(_) => "array",
                };
                return Err(format!(
                    "error: recursive {kind} definition (infinite size): {}",
                    names.join(" -> ")
                ));
            }
            VisitState::Done => {}
        }
    }
    st.path.pop();
    *node_state(node, st) = VisitState::Done;
    Ok(())
}

/// Synthesize the generated-word `Sig`s for every registered struct, in
/// declared field order (first field deepest): a constructor
/// `S ( T1 … Tn -- S )`, a destructure `S> ( S -- T1 … Tn )`, and per field a
/// getter `S>fi ( S -- Ti )` and a functional setter `S<fi ( S Ti -- S )`. A
/// zero-field struct registers only the constructor and destructure. These
/// join the env alongside user words, so applying one to the wrong arity or
/// operand type is caught by the same arity/type-mismatch path as any other
/// word call.
pub fn struct_generated_sigs(structs: &[StructDecl]) -> Vec<(String, Sig)> {
    let mut sigs = Vec::new();
    for (idx, decl) in structs.iter().enumerate() {
        let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
        let field_types: Vec<Type> = decl.fields.iter().map(|(_, ty)| *ty).collect();

        sigs.push((
            decl.name.clone(),
            Sig {
                inputs: field_types.clone(),
                outputs: vec![struct_ty],
            },
        ));
        sigs.push((
            format!("{}>", decl.name),
            Sig {
                inputs: vec![struct_ty],
                outputs: field_types.clone(),
            },
        ));
        for (field_name, field_ty) in &decl.fields {
            sigs.push((
                format!("{}>{}", decl.name, field_name),
                Sig {
                    inputs: vec![struct_ty],
                    outputs: vec![*field_ty],
                },
            ));
            sigs.push((
                format!("{}<{}", decl.name, field_name),
                Sig {
                    inputs: vec![struct_ty, *field_ty],
                    outputs: vec![struct_ty],
                },
            ));
        }
    }
    sigs
}

/// Synthesize the generated-word `Sig` for every registered enum variant
/// (D2, R9): a constructor `Variant ( T1 … Tn -- Enum )`, fields in declared
/// order (first field deepest), a zero-field variant being `Variant ( --
/// Enum )`. Unlike a struct, a variant has no destructure/getter/setter
/// (D2: not a standalone type; elimination is clause-style, Phase 4). These
/// join the env alongside user words and struct-generated words, so a
/// constructor's arity/field-type misuse (X9) falls out of the existing
/// call-check path.
pub fn enum_generated_sigs(enums: &[EnumDecl]) -> Vec<(String, Sig)> {
    let mut sigs = Vec::new();
    for (idx, decl) in enums.iter().enumerate() {
        let enum_ty = Type::Enum(EnumId::from_index(idx), decl.name_static);
        for variant in &decl.variants {
            let field_types: Vec<Type> = variant.fields.iter().map(|(_, ty)| *ty).collect();
            sigs.push((
                variant.name.clone(),
                Sig {
                    inputs: field_types,
                    outputs: vec![enum_ty],
                },
            ));
        }
    }
    sigs
}
