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
/// Decl names arrive mangled (`resolve` rewrites every declared name in
/// place), so every diagnostic here renders the name through `render_word` --
/// the user wrote `puts`, never `puts__m0`.
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
        check_extern_boundary_types(decl, enums)?;
    }
    Ok(())
}

/// R1: a located error for an `extern:` declaration redeclaring a name
/// already registered as a builtin, a user `:` word, or another `extern:`.
fn extern_redeclaration_error(decl: &ExternDecl) -> String {
    format!(
        "error: extern: {} redeclares an existing word (line {}, col {})",
        crate::resolve::render_word(&decl.name),
        decl.span.line,
        decl.span.col
    )
}

/// R8 (slice 8b): no C function returns two values, so a declared output
/// arity above one describes no callable prototype. Left unrejected it lowers
/// to a discarded result (`lower_call` binds a return only for `out_arity ==
/// 1`) and panics in the *next* consumer of the value that was never pushed,
/// which points at the wrong term entirely.
fn extern_multi_output_error(decl: &ExternDecl) -> String {
    format!(
        "error: extern: {} declares {} outputs (line {}, col {})\n  no C function returns more than one value; declare at most one output",
        crate::resolve::render_word(&decl.name),
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
///
/// P7 slice 3i (R4): `bool` is `core::bool`'s enum, so admitting it takes the
/// registry the build resolved it in rather than a constant pattern. Only that
/// one enum is admitted, not every payload-free enum: widening the FFI boundary
/// to user enums is a rule change with no client here.
fn is_extern_boundary_scalar(ty: Type, enums: &[EnumDecl]) -> bool {
    if Some(ty) == resolve_bool_type(enums) {
        return true;
    }
    matches!(
        ty,
        Type::Int(_) | Type::Float(_) | Type::Usize | Type::Isize | Type::Ref(..) | Type::Cstr
    )
}

/// R3: each `extern:` boundary-type rejection not already covered by
/// `check_reference_free_signature` (which independently rejects any
/// reference-containing output before this ever runs). An owned aggregate
/// (struct/enum/array/`^T`) is rejected in either position: ownership across
/// the FFI boundary has no answer and no client. A `^T` specifically in
/// output position gets its own message, since forging ownership of memory
/// the allocator did not hand out is a sharper reason than the generic one.
fn check_extern_boundary_types(decl: &ExternDecl, enums: &[EnumDecl]) -> Result<(), String> {
    for slot in &decl.effect.inputs {
        if matches!(slot.ty, Type::Str) {
            return Err(extern_str_input_error(decl));
        }
        if matches!(slot.ty, Type::Slice(..)) {
            return Err(extern_slice_input_error(decl, slot.ty));
        }
        if !is_extern_boundary_scalar(slot.ty, enums) {
            return Err(extern_owned_aggregate_error(decl, slot.ty, "input"));
        }
    }
    for slot in &decl.effect.outputs {
        if is_extern_boundary_scalar(slot.ty, enums) {
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
        "error: extern: {} declares the input `str` (line {}, col {})\n  a `str` is a pointer and a length, which matches no C parameter; declare `cstr` and convert with `cstr` at the call site",
        crate::resolve::render_word(&decl.name),
        decl.span.line,
        decl.span.col
    )
}

/// P7 slice 3c (R7): a slice input at the FFI boundary, for the same reason
/// `str` has its own message -- it is a two-word aggregate view, not a scalar
/// or a single opaque `Ptr`, so C would receive a pointer to a descriptor
/// rather than the element pointer it wants. Only the input direction needs an
/// arm: `check_reference_free_signature` runs first and rejects a slice
/// *output* through the ordinary no-stored-reference ban (R5).
fn extern_slice_input_error(decl: &ExternDecl, ty: Type) -> String {
    format!(
        "error: extern: {} declares the input `{}` (line {}, col {})\n  a slice is a pointer and a length, which matches no C parameter; declare `&T` and pass the length as a separate `usize`",
        crate::resolve::render_word(&decl.name),
        ty,
        decl.span.line,
        decl.span.col
    )
}

/// R11: a returned `str` would be a `str` not built from a literal, which is
/// the invariant R10's `Copy`/non-escaping status rests on. C supplies no
/// length either, so there is nothing to build one from.
fn extern_str_output_error(decl: &ExternDecl) -> String {
    format!(
        "error: extern: {} cannot return a `str` (line {}, col {})\n  a `str` may point at static data only, and C supplies no length; declare `cstr`",
        crate::resolve::render_word(&decl.name),
        decl.span.line,
        decl.span.col
    )
}

fn extern_owned_aggregate_error(decl: &ExternDecl, ty: Type, position: &str) -> String {
    format!(
        "error: extern: {} declares the {position} `{}`, an owned aggregate (line {}, col {})\n  ownership across the C boundary has no answer and no client; only the numeric tower, `&T`/`&!T`, and `cstr` may cross",
        crate::resolve::render_word(&decl.name),
        ty,
        decl.span.line,
        decl.span.col
    )
}

fn extern_owned_pointer_output_error(decl: &ExternDecl, ty: Type) -> String {
    format!(
        "error: extern: {} cannot return the owned pointer `{}` (line {}, col {})\n  it would forge ownership of memory the allocator did not hand out",
        crate::resolve::render_word(&decl.name),
        ty,
        decl.span.line,
        decl.span.col
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

/// Phase 7 slice 2: a `static:` declaration's own name rules, checked on the
/// raw, pre-mangle module beside `check_exported_signatures`. A static shares
/// one name category with a module's words and types -- `&NAME` resolves
/// against all three (`resolve::NameTables`) -- so a repeat declaration, or a
/// name a word/extern/type of the same module already holds, is rejected here
/// rather than left to shadow silently at the borrow site.
pub fn check_static_decls(module: &Module) -> Result<(), String> {
    let mut seen: HashMap<(u32, &str), Span> = HashMap::new();
    for decl in &module.statics {
        if let Some(first) = seen.insert((decl.module, decl.name.as_str()), decl.span) {
            return Err(duplicate_static_error(decl, first));
        }
    }
    for decl in &module.statics {
        if let Some(kind) = colliding_name_kind(&decl.name, decl.module, decl.span, module) {
            return Err(static_name_collision_error(decl, kind));
        }
    }
    // P7 slice 3i (R1): `bool` is the one entry in the parser's scalar
    // allow-list that resolves through the type registry, and there it can only
    // check the *name*: an enum's variant payloads are not filled in until
    // after declaration parsing, and `resolve_bool_type`'s exactly-two-variants
    // shape test has nothing to run against yet either. So the shape is
    // re-checked here, against the same `resolve_bool_type` the logical
    // operators and `extern:` boundary rest on: a same-named enum that is not
    // *the* resolved bool (a payload-carrying variant, or a third variant) is
    // not a scalar, and a `= true` initializer would write a bare discriminant
    // over storage whose layout or discriminant space it does not own.
    for decl in &module.statics {
        if let Type::Enum(_, name) = decl.ty {
            if Some(decl.ty) != resolve_bool_type(&module.enums) {
                return Err(static_non_scalar_enum_error(decl, name));
            }
        }
    }
    Ok(())
}

/// P7 slice 3i (R1): a static declared at an enum type whose variants are not
/// all payload-free.
fn static_non_scalar_enum_error(decl: &StaticDecl, name: &str) -> String {
    format!(
        "error: static `{}` has a non-scalar type `{name}` (line {}, col {}): a static's enum type must be exactly two variants, each carrying no payload\n  note: only `i64`, `u32`, `Bool` and `str` are supported this slice",
        decl.name, decl.span.line, decl.span.col
    )
}

/// What else already holds `name`, if anything -- the shared cross-kind
/// collision precedent (originally `StaticDecl`-only, generalized by P7.S3e
/// R1 so a new `trait:` declaration can be checked against every other kind,
/// including another trait). `span` is the *target's own* declaration span,
/// so the `traits` arm can exclude a self-match; there is deliberately no
/// `statics` arm, since a static-vs-static duplicate is `check_static_decls`'
/// own `seen` pass and a static-vs-trait collision is reached from the static
/// call site (which runs first) via the `traits` arm below.
///
/// A pre-seeded `Copy`/`Ord` trait entry (`RESERVED_TRAIT_MODULE`) collides
/// with every module (R2, decision 2): a user `trait: Copy` in any module is
/// rejected, since the reserved entry participates in no real module's
/// ownership.
fn colliding_name_kind(
    name: &str,
    decl_module: u32,
    span: Span,
    module: &Module,
) -> Option<&'static str> {
    let owns = |m: u32| m == decl_module;
    if module
        .words
        .iter()
        .any(|w| owns(w.module) && w.name == name)
    {
        return Some("word");
    }
    if module
        .externs
        .iter()
        .any(|x| owns(x.module) && x.name == name)
    {
        return Some("extern");
    }
    if module
        .structs
        .iter()
        .any(|s| owns(s.module) && s.name_static == name)
        || module
            .enums
            .iter()
            .any(|e| owns(e.module) && e.name_static == name)
    {
        return Some("type");
    }
    if module.traits.iter().any(|t| {
        (owns(t.module) || t.module == RESERVED_TRAIT_MODULE) && t.name == name && t.span != span
    }) {
        return Some("trait");
    }
    None
}

/// P7.S3e (R1): every `trait:` declaration's own name rules, mirroring
/// `check_static_decls` -- a same-module duplicate trait name (including a
/// user `trait: Copy`/`trait: Ord` colliding with the reserved predicate
/// entries) and a cross-kind collision (`trait: Point` alongside `type:
/// Point`) are both rejected here, pre-mangle.
pub fn check_trait_decls(module: &Module) -> Result<(), String> {
    let mut seen: HashMap<(u32, &str), Span> = HashMap::new();
    for decl in &module.traits {
        if decl.module == RESERVED_TRAIT_MODULE {
            continue;
        }
        if let Some(first) = seen.insert((decl.module, decl.name.as_str()), decl.span) {
            return Err(duplicate_trait_error(decl, first));
        }
    }
    for decl in &module.traits {
        if decl.module == RESERVED_TRAIT_MODULE {
            continue;
        }
        if let Some(kind) = colliding_name_kind(&decl.name, decl.module, decl.span, module) {
            return Err(trait_name_collision_error(decl, kind));
        }
    }
    for decl in &module.traits {
        if decl.module == RESERVED_TRAIT_MODULE {
            continue;
        }
        for member in &decl.members {
            if !member_binds_trait_var(member) {
                return Err(nested_receiver_member_error(decl, member));
            }
        }
    }
    Ok(())
}

/// P7.S3p (ruling 5): a member must take the trait's own type variable (id 0
/// in the member's own `PolySig`) as *some* input, bare `'T` or `&'T`, at any
/// position -- bound dispatch (`poly_trait_member_call`, `src/check/poly.rs`)
/// finds the variable by member name off the body's bounds and matches the
/// whole declared input list, so a non-trailing receiver dispatches fine.
///
/// P7.S3t: an **empty** input list is admitted too. A nullary member has no
/// operand to ground `'T` from, but a call to the *wrapping* generic word can
/// now supply it explicitly (`f[Point]`, seeded into `Subst` ahead of operand
/// unification in `check_poly_call`), so the member is dispatchable and the
/// gate no longer needs to shut the door before that mechanism runs.
///
/// P7b.S2 (S2-2, R4): the rule is HKT-aware. A member of a higher-kinded
/// trait (`trait: Functor['F: * -> *]`) dispatches on an **application
/// headed by the trait var** (`'F['T]`) -- its bare `'F` mention would be a
/// kind error (S2-15.b) and its dispatchable input is App-headed, so the
/// old bare-only acceptance would reject every HKT member (m1). `&'F` stays
/// dispatchable (dropping it would break every S1-style ref-member trait,
/// `Shw`), and the same courtesy extends to `&'F['T]`: ref-ness is an
/// addressing mode, not a type identity, so a reference to a
/// trait-var-headed application dispatches on the same head.
///
/// A member that mentions the variable only *nested* inside a composite
/// input still cannot dispatch: grounding that would need structural
/// unification through the array type, which dispatch does not attempt.
/// Rejected here, at `trait:` declaration time, rather than left to
/// mis-dispatch at the call.
/// S2-2: the dispatchable head -- the trait var bare, or heading an
/// application. A member local's head (`'G['T]`) dispatches on nothing.
fn dispatchable_head(t: &PolyType) -> bool {
    matches!(t, PolyType::Var(0) | PolyType::App { head: 0, .. })
}

fn member_binds_trait_var(member: &TraitMember) -> bool {
    member.sig.inputs.is_empty()
        || member.sig.inputs.iter().any(|input| match input {
            PolyType::Ref(referent, _) => dispatchable_head(referent),
            other => dispatchable_head(other),
        })
}

/// S2-15.a (review fix): whether the member's *inputs* mention the trait var
/// (bare, or heading an application) somewhere nested -- an array element, a
/// deeper reference, a cell payload, or a quotation row. That is the
/// near-miss the note in `nested_receiver_member_error` explains, so the note
/// is appended only for it. A var at a dispatchable position (top level, or
/// under a single `&`) never reaches the error at all, and a var mentioned
/// only in the outputs (golden #1's `pick ( 'T -- 'F['T] )`) is not an input
/// near-miss -- neither case wants the note.
fn member_inputs_nest_trait_var(member: &TraitMember) -> bool {
    fn mentions_trait_var(t: &PolyType) -> bool {
        match t {
            PolyType::Var(0) | PolyType::App { head: 0, .. } => true,
            PolyType::Array(elem, _) => mentions_trait_var(elem),
            PolyType::Ref(referent, _) => mentions_trait_var(referent),
            PolyType::OwnedCell(inner) => mentions_trait_var(inner),
            PolyType::Quotation(ins, outs, ..) => ins.iter().chain(outs).any(mentions_trait_var),
            PolyType::Generic { args, .. } | PolyType::GenericVariant { args, .. } => {
                args.iter().any(mentions_trait_var)
            }
            PolyType::Concrete(_) | PolyType::Var(_) | PolyType::App { .. } | PolyType::QuotLit => {
                false
            }
        }
    }
    // By the time this runs, no input is dispatchable (the gate has already
    // rejected the member), so any mention found here is necessarily a
    // nested one.
    member.sig.inputs.iter().any(mentions_trait_var)
}

/// P7b.S2 (S2-15.a): the HKT-aware replacement of m1's captured message. The
/// member is the offending position; the parenthetical names the expected
/// shape in the trait-var-headed form the trait's kind actually calls for.
/// The nested-composite note rides along only when the member's inputs
/// actually nest the trait var under a composite shape -- it is advice about
/// that specific near-miss, not part of the spec's pinned S2-15.a text.
fn nested_receiver_member_error(decl: &TraitDecl, member: &TraitMember) -> String {
    // The trait header var is always var 0 of each member's sig, per the
    // parser's `debug_assert_eq!(id, 0, ...)` at src/parser.rs:3931.
    let head = member
        .sig
        .ty_var_names
        .first()
        .map(String::as_str)
        .unwrap_or("'F");
    let arg = if head == "'T" { "'U" } else { "'T" };
    let mut msg = format!(
        "error: trait member `{}` of `{}` (line {}, col {}) has no input for a call to dispatch on \
         (expected the trait's variable `{head}` bare or heading an application like `{head}[{arg}]`)",
        member.name, decl.name, member.span.line, member.span.col
    );
    if member_inputs_nest_trait_var(member) {
        msg.push_str(
            "\n  note: a variable nested inside a composite input (an array element, say) does not count",
        );
    }
    msg
}

fn duplicate_trait_error(decl: &TraitDecl, first: Span) -> String {
    format!(
        "error: duplicate trait `{}` (line {}, col {}); first declared at line {}, col {}",
        decl.name, decl.span.line, decl.span.col, first.line, first.col
    )
}

fn trait_name_collision_error(decl: &TraitDecl, kind: &str) -> String {
    format!(
        "error: trait `{}` (line {}, col {}) is already the name of a {kind} in this module",
        decl.name, decl.span.line, decl.span.col
    )
}

/// P7.S3e (R4/R11, decision 1/2): the declaring module of a struct/enum
/// target type, for the orphan rule -- `None` for a scalar or any other
/// builtin-shaped type, which declares no module of its own (an `impl:`
/// naming one satisfies the orphan rule only by living in the trait's own
/// module; builtin-shaped ctor images such as `impl: Functor for usize`
/// take that same `None` path, so such impls are trait-module-only --
/// coherent, and now stated).
///
/// P7.S4 (R4): a generic target (`PolyType` not `Concrete(Struct/Enum)`) is
/// treated exactly like a scalar: `None`, so the orphan rule requires the impl
/// to live in the trait's own module. A concrete struct/enum target keeps
/// the existing rule (the target's module **or** the trait's module).
///
/// P7b.S2 (S2-11): a ctor-abstract target (`for Box`, `for Result[i64]` --
/// the S2-4 desugar's `Generic` pattern) names its constructor, so the same
/// two homes a concrete target gets apply: the constructor's module or the
/// trait's module. `Generic.module` is the declaring module `poly_generic_
/// header` resolved the header under (the same `(idx, module)` identity
/// `Type::CtorImage` carries), so the arm is exact, not a heuristic.
fn impl_target_module(target: &crate::ast::ImplTarget, module: &Module) -> Option<u32> {
    match target.pattern {
        crate::ast::PolyType::Concrete(crate::ast::Type::Struct(id, _)) => {
            Some(module.structs[id.index()].module)
        }
        crate::ast::PolyType::Concrete(crate::ast::Type::Enum(id, _)) => {
            Some(module.enums[id.0].module)
        }
        crate::ast::PolyType::Generic {
            module: ctor_module,
            ..
        } => Some(ctor_module),
        _ => None,
    }
}

/// P7.S4b (R4): compare two bound sets for equality as unordered sets of
/// `(u32, Bound)` pairs. Since `PolyType` structural equality already
/// normalises variable indices across alpha-equivalent targets, the indices
/// are positionally consistent and compared directly.
fn bounds_eq(a: &[(u32, Bound)], b: &[(u32, Bound)]) -> bool {
    a.len() == b.len() && a.iter().all(|p| b.contains(p))
}

/// Entry in the duplicate-impl scan: `(TraitId, PolyType)` plus the bound
/// set and span, kept as a struct to satisfy clippy's type-complexity lint.
struct SeenImpl {
    trait_id: TraitId,
    pattern: PolyType,
    bounds: Vec<(u32, Bound)>,
    span: Span,
}

/// P7.S3e (R4/R11, decision 1): every `impl:` declaration's own checks --
/// duplicate `(TraitId, PolyType)`, the orphan rule, and every required
/// member bound exactly once (no missing member, no repeat). Each member's
/// word is the parser's synthesized body word, so there is no signature to
/// verify here; this only resolves it to a `Module::words` index (R8).
///
/// P7.S4 (R7): the duplicate check compares `target.pattern` (the `PolyType`),
/// NOT the whole `ImplTarget` — comparing `ty_var_names`/`len_var_names` would
/// make `['T N]` and `['U M]` unequal (different name strings) and break
/// alpha-equivalence. `PolyType` derives `PartialEq`, so structural equality
/// gives alpha-equivalence for free: both fold to `Array(Var(0), Var(0))`.
pub fn check_impl_decls(module: &mut Module) -> Result<(), String> {
    // `PolyType` derives no `Hash`, so the duplicate check is a linear scan
    // rather than a `HashMap` -- `impl:` counts are small, so this stays cheap.
    //
    // P7.S4b (R4): the key widens to `(TraitId, PolyType, BoundSet)` so a
    // bounded and unbounded impl at the same pattern are distinct (not a
    // duplicate). The bound set is compared by `(u32, Bound)` index pairs —
    // since `PolyType` structural equality already normalises variable
    // indices across alpha-equivalent targets (`['T N]` == `['U M]`), the
    // indices are positionally consistent and need no name-table lookup.
    let mut seen: Vec<SeenImpl> = Vec::new();
    for imp in &module.impls {
        if let Some(first) = seen
            .iter()
            .find(|s| {
                s.trait_id == imp.trait_id
                    && s.pattern == imp.target.pattern
                    && bounds_eq(&s.bounds, &imp.target.bounds)
            })
            .map(|s| s.span)
        {
            return Err(duplicate_impl_error(imp, first));
        }
        seen.push(SeenImpl {
            trait_id: imp.trait_id,
            pattern: imp.target.pattern.clone(),
            bounds: imp.target.bounds.clone(),
            span: imp.span,
        });
    }
    for i in 0..module.impls.len() {
        let (trait_id, target, impl_module, impl_span, bindings) = {
            let imp = &module.impls[i];
            (
                imp.trait_id,
                &imp.target,
                imp.module,
                imp.span,
                imp.bindings.clone(),
            )
        };
        let trait_decl_module = module.traits[trait_id.index()].module;
        let trait_name = module.traits[trait_id.index()].name.clone();
        let target_module = impl_target_module(target, module);
        if impl_module != trait_decl_module && Some(impl_module) != target_module {
            return Err(impl_orphan_error(
                &trait_name,
                target,
                target_module.is_some(),
                impl_span,
            ));
        }
        let mut bound_names: HashSet<&str> = HashSet::new();
        // R8: the word each member binds, as an index into `module.words`,
        // resolved here (pre-mangle, where `word_name` and `WordDef::name`
        // still agree) and read back at a bound-directed call site.
        //
        // P7.S3r (R2): `bindings` comes from the parser's body-form desugar,
        // which validates each member against the trait and splices a
        // word carrying that member's grounded signature into this same
        // module. The name lookup is therefore exact and total, and the one
        // thing left to reject is a member bound twice.
        let mut resolved: Vec<(String, usize)> = Vec::new();
        for (member_name, word_name) in &bindings {
            if !bound_names.insert(member_name.as_str()) {
                return Err(impl_duplicate_member_error(
                    &trait_name,
                    member_name,
                    impl_span,
                ));
            }
            let idx = module
                .words
                .iter()
                .position(|w| w.module == impl_module && &w.name == word_name)
                .expect("the desugar splices the synthesized member word into this module");
            resolved.push((member_name.clone(), idx));
        }
        let trait_decl = &module.traits[trait_id.index()];
        for member in &trait_decl.members {
            if !bound_names.contains(member.name.as_str()) {
                return Err(impl_missing_member_error(
                    &trait_name,
                    &member.name,
                    impl_span,
                ));
            }
        }
        module.impls[i].resolved = resolved;
    }
    Ok(())
}

fn duplicate_impl_error(imp: &ImplDecl, first: Span) -> String {
    format!(
        "error: duplicate `impl:` for `{}` (line {}, col {}); first declared at line {}, col {}",
        impl_target_str(&imp.target),
        imp.span.line,
        imp.span.col,
        first.line,
        first.col
    )
}

/// P7.S3e (R4/R11, decision 1/2): the orphan rule's rejection. A target type
/// that declares no module of its own (`impl_target_module` is `None`: a
/// scalar or other builtin shape) leaves exactly one legal home, so pointing
/// at "the module declaring `i64`" would be unactionable.
///
/// P7.S4 (R4): a generic target (non-concrete `PolyType`) also declares no
/// module of its own, so the same `None` path already enforces the trait-
/// module rule. Phase 3 updates only the diagnostic rendering; the logic is
/// correct from Phase 1.
fn impl_orphan_error(
    trait_name: &str,
    target: &crate::ast::ImplTarget,
    target_declares_module: bool,
    span: Span,
) -> String {
    let target_str = impl_target_str(target);
    let homes = if target_declares_module {
        format!(
            "must live in the module declaring `{trait_name}` or the module declaring `{target_str}`"
        )
    } else {
        format!(
            "must live in the module declaring `{trait_name}` (`{target_str}` declares no module of its own)"
        )
    };
    format!(
        "error: `impl: {trait_name} for {target_str}` at line {}, col {} {homes}",
        span.line, span.col
    )
}

fn impl_duplicate_member_error(trait_name: &str, member: &str, span: Span) -> String {
    format!(
        "error: `impl: {trait_name}` binds member `{member}` more than once at line {}, col {}",
        span.line, span.col
    )
}

fn impl_missing_member_error(trait_name: &str, member: &str, span: Span) -> String {
    format!(
        "error: `impl: {trait_name}` at line {}, col {} does not bind required member `{member}`",
        span.line, span.col
    )
}

fn duplicate_static_error(decl: &StaticDecl, first: Span) -> String {
    format!(
        "error: duplicate static `{}` (line {}, col {}); first declared at line {}, col {}",
        decl.name, decl.span.line, decl.span.col, first.line, first.col
    )
}

fn static_name_collision_error(decl: &StaticDecl, kind: &str) -> String {
    format!(
        "error: static `{}` (line {}, col {}) is already the name of a {} in this module\n  a `&{}` borrow would have two things to resolve to: rename one of them",
        decl.name, decl.span.line, decl.span.col, kind, decl.name
    )
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
        // Slice 13 (R-A9): a private type named behind a `&` is still named,
        // so export-privacy must see it.
        PolyType::Ref(referent, _) => collect_poly_concrete(referent, out),
        // P7.S3n (R3): likewise a private type named as a cell payload.
        PolyType::OwnedCell(payload) => collect_poly_concrete(payload, out),
        // Slice 6a (R5): a declared quotation effect's rows may name concrete
        // types (`[ i64 -- ]`); collect them so export-privacy still sees a
        // private type mentioned inside an effect.
        PolyType::Quotation(ins, outs, _, _, _) => {
            for t in ins.iter().chain(outs) {
                collect_poly_concrete(t, out);
            }
        }
        // P7 slice 3b: a body-only marker, never in a declared signature.
        PolyType::QuotLit => unreachable!("a quotation-literal marker never reaches a signature"),
        // P7 slice 3a: recurse into args only -- a variable-bearing generic
        // names no concrete `Type` of its own to contribute (its header is a
        // separate, currently-unclosed export-privacy gap; see the spec's
        // "export-privacy gap" note). A concrete argument still needs the
        // ordinary check, e.g. `Box[PrivateStruct]`.
        PolyType::Generic { args, .. } => {
            for a in args {
                collect_poly_concrete(a, out);
            }
        }
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a declared signature this walk sees.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches a declared signature"
        ),
        // P7b.S1 (S1-16): same reasoning as `Generic` -- the applied
        // variable itself names no concrete `Type`, but a concrete argument
        // still needs the ordinary check.
        PolyType::App { args, .. } => {
            for a in args {
                collect_poly_concrete(a, out);
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
/// only the base name appears here; a member (`Type>`) can only collide when
/// its base does. `qualifier` is `None` for a P8 S2 wildcard-desugared entry
/// (R1): a wildcard binds no qualifier, so its diagnostics use a
/// wildcard-specific wording rather than a fabricated qualifier that would
/// misdescribe the import shape.
pub struct SelectiveName {
    pub name: String,
    pub qualifier: Option<String>,
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
        let mut seen: HashMap<&str, Option<&str>> = HashMap::new();
        for entry in entries {
            let exports = &module.modules[entry.target as usize].exports;
            if !exports.iter().any(|(n, _)| n == &entry.name) {
                return Err(selective_not_exported_error(
                    &entry.name,
                    entry.qualifier.as_deref(),
                    entry.span,
                ));
            }
            // P7.S3q (R5): an entry naming an intrinsic the source module
            // effectively admits is exempt from both collision rules, because
            // it binds no word and carries no module identity -- neither rule
            // has a subject. A local `: drop ( Fd -- )` and the intrinsic
            // `drop` already coexist in one file (`resolve::mangle` exempts
            // `drop`), and two hubs both admitting `drop` union to one set, so
            // an ambiguity error would name an ambiguity with no two answers.
            // Evaluated *after* the not-exported check above, so a hub that
            // does not export the name still fails there first.
            //
            // A source that *declares* the name is never exempt, however much
            // it also admits the intrinsic: `resolve_export_origins` resolves
            // its `export:` entry to that declaration, so the entry does bind
            // a word and both rules have their subject back. Without this the
            // exemption swallows a real ambiguity -- an importer declaring its
            // own `dup` alongside an imported `dup` would silently shadow one
            // with the other instead of being told.
            //
            // Review fix: a source that *re-exports* a real declaration of
            // the name (its own `selective` map carries an entry for it) is
            // exactly as unexempt as one that declares it directly --
            // `resolve_export_origins` now checks that same `selective` entry
            // ahead of its own intrinsic-admit branch, so the source's
            // `export:` entry binds the re-exported word, not the intrinsic,
            // and this check must agree or it would wave through the same
            // shadowing ambiguity one level removed (an importer's own `dup`
            // silently shadowing a `dup` the source re-exports from a further
            // module, rather than colliding as it must).
            let admitted_intrinsic = crate::ast::is_name_dispatched_builtin(&entry.name)
                && module.modules[entry.target as usize]
                    .intrinsics
                    .admits(&entry.name)
                && !local_decl_names(module, entry.target).contains(entry.name.as_str())
                && !module.modules[entry.target as usize]
                    .selective
                    .contains_key(entry.name.as_str());
            if !admitted_intrinsic {
                if locals.contains(entry.name.as_str()) {
                    return Err(selective_collides_with_local_error(
                        &entry.name,
                        entry.qualifier.as_deref(),
                        entry.span,
                    ));
                }
                if let Some(first) = seen.insert(entry.name.as_str(), entry.qualifier.as_deref()) {
                    return Err(selective_collision_error(
                        &entry.name,
                        first,
                        entry.qualifier.as_deref(),
                        entry.span,
                    ));
                }
            }
            // R4 (import mirror): a selective import naming a builtin
            // operator must agree with it on input count. The other two
            // collision checks above forbid an import from sharing a name with
            // a local decl or with another selective import outright -- except
            // an admitted-intrinsic entry (R5), which skips both, so a local or
            // sibling `drop` can still reach this arity check alongside it.
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
                            entry.qualifier.as_deref(),
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
    // P7.S3e (R1, round-2 finding): a locally declared trait colliding with a
    // selectively imported one of the same name must be caught here too, or
    // `check_selective_imports` leaves two live, uncaught bindings of one
    // name. The reserved `Copy`/`Ord` entries are excluded: they carry no
    // real module, and a real module can never selectively import them
    // (they are not declared by any module's `trait:`).
    for t in &module.traits {
        if t.module == m {
            names.insert(t.name.as_str());
        }
    }
    names
}

/// P8 S2 (R1): a selective-import diagnostic names its source either as
/// `module `<qualifier>`` or, for a wildcard-desugared entry with no
/// qualifier, as a wildcard import -- so a fabricated qualifier never
/// misdescribes the import shape.
fn selective_source_phrase(name: &str, qualifier: Option<&str>) -> String {
    match qualifier {
        Some(q) => format!("selective import of `{name}` from module `{q}`"),
        None => format!("wildcard import of `{name}`"),
    }
}

/// R20: a selectively imported name absent from its source module's `export:`
/// list is the R16 visibility error, same wording as a qualified private
/// reference.
pub(crate) fn selective_not_exported_error(
    name: &str,
    qualifier: Option<&str>,
    span: Span,
) -> String {
    let source = match qualifier {
        Some(q) => format!("module `{q}`"),
        None => "its wildcard-imported module".to_string(),
    };
    format!(
        "error: `{name}` is not exported from {source} at line {}, col {}",
        span.line, span.col
    )
}

/// R21: a second selective import exposing a name a prior one already exposed,
/// naming both source modules. No precedence, no shadowing: the collision is
/// the error.
fn selective_collision_error(
    name: &str,
    first: Option<&str>,
    second: Option<&str>,
    span: Span,
) -> String {
    let second_phrase = selective_source_phrase(name, second);
    let first_phrase = match first {
        Some(q) => format!("the selective import of `{name}` from module `{q}`"),
        None => format!("the wildcard import of `{name}`"),
    };
    format!(
        "error: {second_phrase} (line {}, col {}) collides with {first_phrase}",
        span.line, span.col
    )
}

/// R21: a selective import exposing a name the importing module already defines
/// locally, naming the source module and the local definition.
fn selective_collides_with_local_error(name: &str, qualifier: Option<&str>, span: Span) -> String {
    format!(
        "error: {} (line {}, col {}) collides with a local definition of `{name}`",
        selective_source_phrase(name, qualifier),
        span.line,
        span.col
    )
}

/// R4 (import mirror): a selective import of a builtin-named word whose input
/// count disagrees with the builtin's -- the arity clash rejected at the
/// second candidate's site (`overload_arity_clash_error`'s local twin), here
/// the import site rather than a definition site.
fn selective_arity_clash_error(
    name: &str,
    qualifier: Option<&str>,
    span: Span,
    arity: usize,
    builtin_arity: usize,
) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!(
        "error: {} (line {}, col {}) takes {arity} input{} but the builtin `{name}` takes {builtin_arity}; all overloads of a name must agree on input count",
        selective_source_phrase(name, qualifier),
        span.line,
        span.col,
        plural(arity),
    )
}

/// Type-level checks that must pass before any generated-word signature or
/// word body is type-checked: no two `type:` declarations share a name across
/// the combined struct+enum registries, and no struct or enum contains itself
/// by value, directly or transitively, through the combined type graph (D9,
/// D10, R8, R10). `generic_structs`/`generic_enums` take part only in the
/// duplicate-name check (Phase 5 slice 1 fix): a generic header mints no
/// concrete registry entry the recursion/stored-reference/linear-array passes
/// could walk, so those three stay unchanged and un-widened.
pub fn check_types(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    generic_structs: &[GenericStructDecl],
    generic_enums: &[GenericEnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    slices: &[SliceDecl],
) -> Result<(), String> {
    check_duplicate_type_names(structs, enums, generic_structs, generic_enums)?;
    check_recursion(structs, enums, arrays)?;
    check_no_stored_references(structs, enums, arrays, cells)?;
    check_slice_element_gate(structs, enums, arrays, slices)?;
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
                    &format!("field `{field}` of type `{}`", decl.name_static),
                    *ty,
                    Some(decl.span),
                ));
            }
        }
    }
    for decl in enums {
        for variant in &decl.variants {
            for (idx, (field, ty)) in variant.fields.iter().enumerate() {
                if contains_reference(*ty, structs, enums, arrays) {
                    return Err(stored_reference_error(
                        &format!(
                            "payload {} of variant `{}` of type `{}`",
                            super::variant_field_desc(field, idx),
                            variant.name,
                            decl.name_static
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

/// The struct-only projection of `check_types` (no enums/arrays/generics), for
/// callers that don't yet declare any of them.
pub fn check_structs(structs: &[StructDecl]) -> Result<(), String> {
    check_types(structs, &[], &[], &[], &[], &[], &[])
}

/// P7 slice 3c (R1.2, phase 3 review fix): a slice's element must be
/// concrete and `Copy` -- the rule the exit notes claimed was unreachable
/// because `slice`'s only *construction* source is an array reference, which
/// misses the *type spelling* route: `Slice[T]` interns straight from the
/// parser (`intern_slice_type`), so `Slice[Slice[i64]]`, `Slice[&i64]`,
/// `Slice[&!i64]`, and `Slice[^i64]` all reach this registry with no array in
/// sight. Mirrors the two array sweeps above over `module.slices` instead of
/// `module.arrays`: `contains_reference` catches a reference or nested-slice
/// element (a slice is itself reference-shaped, `contains_reference`'s
/// `Type::Slice(..) => true` arm), `is_copy` catches a linear one (a cell or a
/// linear struct/enum). Without this, a disallowed element reaches
/// `slice_layout`/`scalar_size_align` uncaught and the layout builder panics.
pub(crate) fn check_slice_element_gate(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    slices: &[SliceDecl],
) -> Result<(), String> {
    for decl in slices {
        if contains_reference(decl.element, structs, enums, arrays) {
            return Err(stored_reference_error(
                &format!("element of slice type `{}`", decl.name_static),
                decl.element,
                None,
            ));
        }
        if !is_copy(decl.element, structs, enums, arrays) {
            return Err(format!(
                "error: linear slice elements are not supported yet: slice type `{}` has element `{}`, which is linear and has no `Copy` instance",
                decl.name_static,
                decl.element.name(),
            ));
        }
    }
    Ok(())
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

/// A duplicate type name across the combined struct, enum, generic-struct and
/// generic-enum registries (D10, X2; widened for generic headers by the
/// Phase 5 slice 1 review) is a sharp located error naming the type: a name
/// used by any two of a struct, an enum, a generic struct, or a generic enum.
/// A generic header mints no concrete registry entry (`prepass_type_decls`
/// skips it), so without this it could collide with an existing concrete
/// type, or with another generic header, and neither would be caught here,
/// only surfacing later as an ambiguous name for Phase 2's instantiation
/// lookup to pick between arbitrarily. Delegates the struct-only pass to
/// `check_duplicate_struct_names` rather than re-scanning `structs` twice.
pub(super) fn check_duplicate_type_names(
    structs: &[StructDecl],
    enums: &[EnumDecl],
    generic_structs: &[GenericStructDecl],
    generic_enums: &[GenericEnumDecl],
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
    for decl in generic_structs {
        if seen.insert((decl.module, decl.name.as_str()), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name, decl.span.line, decl.span.col
            ));
        }
    }
    for decl in generic_enums {
        if seen.insert((decl.module, decl.name.as_str()), ()).is_some() {
            return Err(format!(
                "error: duplicate type `{}` (line {}, col {})",
                decl.name, decl.span.line, decl.span.col
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
                "error: duplicate word {} (line {}, col {}); first defined at line {}, col {}",
                crate::resolve::render_word(&word.name),
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
        "error: overload of {} (line {}, col {}) has the same input types ({}) as a builtin",
        crate::resolve::render_word(name),
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
    let name = crate::resolve::render_word(name);
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!(
        "error: overload of {name} (line {}, col {}) takes {arity} input{} but another {name} takes {other_arity}; all overloads of a name must agree on input count",
        span.line,
        span.col,
        plural(arity),
    )
}

/// R5: a name with both a generic (poly) candidate and a concrete candidate
/// (a builtin row or a local monomorphic word) of the same input arity is
/// rejected -- there is no specialization ordering that could otherwise pick
/// between `: add ( 'T 'T -- 'T )` and `: add ( i64 i64 -- i64 )` (or a builtin
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
pub(super) fn check_generic_concrete_overlap(
    words: &[WordDef],
    traits: &[TraitDecl],
    impls: &[ImplDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> Result<(), String> {
    let builtins = builtin_table();
    let mut builtin_arity: HashMap<&str, usize> = HashMap::new();
    for (name, rows) in builtins.iter() {
        builtin_arity.insert(name.as_str(), rows[0].inputs.len());
    }
    let mut concrete_inputs: HashMap<(u32, &str), Vec<Type>> = HashMap::new();
    for word in words {
        if word.name != "drop" && word.poly.is_none() {
            concrete_inputs
                .entry((word.module, word.name.as_str()))
                .or_insert_with(|| word.effect.inputs.iter().map(|s| s.ty).collect());
        }
    }
    for word in words {
        let Some(sig) = &word.poly else { continue };
        let arity = sig.inputs.len();
        let concrete_overlaps = concrete_inputs
            .get(&(word.module, word.name.as_str()))
            .is_some_and(|inputs| {
                inputs.len() == arity
                    && poly_admits(sig, inputs, impls, traits, arrays, cells, refs)
            });
        let overlaps = builtin_arity.get(word.name.as_str()) == Some(&arity) || concrete_overlaps;
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
/// scope.
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

/// Whether a call matching a concrete candidate's declared `inputs` could
/// *also* match `sig` — the question the overlap rule is really asking. A
/// bound the concrete type fails means no call site can reach both, so there
/// is nothing to disambiguate.
///
/// Slice 10c: this is what lets `core::cmp`'s `: <['T: Copy Ord] ( 'T 'T --
/// bool )` coexist with a user's `: < ( Vec2 Vec2 -- bool )`, which slice 8a
/// shipped and an arity-only test would now reject outright. Only the `Ord`
/// bound is consulted (P7.S3s: an `impl: Ord` registry lookup, `Ord` no
/// longer being a reserved predicate): `Copy` needs the struct/enum
/// registries this pass does not carry, and leaving it out only ever keeps
/// the rule *stricter*.
fn poly_admits(
    sig: &PolySig,
    inputs: &[Type],
    impls: &[ImplDecl],
    traits: &[TraitDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
) -> bool {
    inputs.iter().zip(&sig.inputs).all(|(ty, pin)| match pin {
        PolyType::Concrete(want) => want == ty,
        // `ord_trait_id` resolves *this* `sig`'s own `Ord` bound (review
        // fix: not a whole-program name search, which fails open under a
        // module-local `trait: Ord`).
        PolyType::Var(v) => match crate::check::ord_trait_id(sig, *v, traits) {
            Some(ord) => impls.iter().any(|i| {
                i.trait_id == ord
                    && crate::check::poly::match_impl_target(
                        &i.target.pattern,
                        *ty,
                        arrays,
                        cells,
                        refs,
                        None,
                    )
                    .is_some()
            }),
            None => true,
        },
        _ => true,
    })
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
        // A bare variant is never a field: the enum it belongs to (and that
        // enum's fields) are the graph nodes, so it closes no by-value
        // containment cycle (Phase 6 slice 2, R3).
        Type::Variant(..) => None,
        // A reference is a pointer, not an inline copy, so it closes no
        // by-value cycle — and the no-stored-reference rule keeps one out of
        // every field position
        // anyway.
        Type::Ref(..) => None,
        // P7 slice 3c (R1.3): a slice is reference-shaped -- it views storage
        // it does not own, so it holds no inline copy of its element and
        // closes no size cycle. The same no-stored-reference rule keeps it out
        // of every field position too (`contains_reference` reports it).
        Type::Slice(..) => None,
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
        // P7.S3v (R6): an `owning` quotation *is* a legal field, so this arm
        // is load-bearing rather than vacuous -- and it is still a leaf. The
        // value is a fixed three-slot aggregate (code, env, disposer) whose
        // captures live behind the env pointer, so like a reference it holds
        // no inline copy and closes no size cycle: `type: Box q owning
        // [ Box -- ] ;` is finite.
        | Type::Quotation(_)
        | Type::InlineQuotation(_)
        | Type::OwningQuotation(_) => None,
        // P7b.S1 (S1-12): a `CtorImage` is not a concrete value type (any
        // matcher that would treat it as one routes to S1-15.g), so it
        // names no field/containment node here either.
        Type::CtorImage(_, _) => None,
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
        // `array[T N]` contains a `T` by value, so a cycle through an array element
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
        TypeNode::Struct(i) => structs[i].name_static,
        TypeNode::Enum(i) => enums[i].name_static,
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
/// `S ( T1 … Tn -- S )` and a destructure `S> ( S -- T1 … Tn )`. Per-field
/// access goes through a receiver-directed projection (`&f`/`&!f`,
/// `check/word_families.rs`) instead of a generated word, since the field
/// name alone is not globally unique the way `decl.name` is. These join the
/// env alongside user words, so applying one to the wrong arity or operand
/// type is caught by the same arity/type-mismatch path as any other word
/// call.
///
/// D7: each entry is `(env key, lowering symbol, Sig)`. For a hand-written
/// concrete `type:` the two names coincide (`decl.name` carries no `[...]`
/// suffix). For a monomorphized instantiation `decl.name` is the mangled
/// registry identity (`Box[i64]`) -- unspellable by any source term, since
/// `[` is a lexer delimiter -- so the env key is instead the bare surface
/// name (`generic_surface_name`, `Box`) every use site actually calls,
/// while the symbol stays the mangled spelling so two instantiations'
/// generated words keep distinct lowering identities (R5).
pub fn struct_generated_sigs(structs: &[StructDecl]) -> Vec<(String, String, u32, Sig)> {
    let mut sigs = Vec::new();
    for (idx, decl) in structs.iter().enumerate() {
        let struct_ty = Type::Struct(StructId::from_index(idx), decl.name_static);
        let field_types: Vec<Type> = decl.fields.iter().map(|(_, ty)| *ty).collect();
        let surface = generic_surface_name(&decl.name);

        sigs.push((
            surface.to_string(),
            decl.name.clone(),
            decl.module,
            Sig {
                inputs: field_types.clone(),
                outputs: vec![struct_ty],
            },
        ));
        sigs.push((
            format!("{surface}>"),
            format!("{}>", decl.name),
            decl.module,
            Sig {
                inputs: vec![struct_ty],
                outputs: field_types.clone(),
            },
        ));
    }
    sigs
}

/// Synthesize the generated-word `Sig` for every registered enum variant
/// (D2, R9): a constructor `Variant ( T1 … Tn -- Enum )`, fields in declared
/// order (first field deepest), a zero-field variant being `Variant ( --
/// Enum )`. The destructure lives beside this in `variant_generated_sigs`
/// (phase 6 slice 2 reverses D2's "a variant has no destructure": it is a
/// standalone `Type::Variant` now); a variant still has no setter (R8), and
/// per-field access goes through a receiver-directed projection (`&f`,
/// P7 slice 1) rather than a generated getter. These join the env alongside
/// user words and struct-generated words, so a constructor's arity/field-type
/// misuse (X9) falls out of the existing call-check path.
pub fn enum_generated_sigs(enums: &[EnumDecl]) -> Vec<(String, String, u32, Sig)> {
    let mut sigs = Vec::new();
    for (idx, decl) in enums.iter().enumerate() {
        let enum_ty = Type::Enum(EnumId::from_index(idx), decl.name_static);
        for variant in &decl.variants {
            let field_types: Vec<Type> = variant.fields.iter().map(|(_, ty)| *ty).collect();
            let surface = generic_surface_name(&variant.name);
            sigs.push((
                surface.to_string(),
                variant.name.clone(),
                decl.module,
                Sig {
                    inputs: field_types,
                    outputs: vec![enum_ty],
                },
            ));
        }
    }
    sigs
}

/// Phase 6 slice 2 (R6): per enum variant, the whole-variant destructure
/// `Variant> ( Variant -- T1 … Tn )`, fields in declared order (first field
/// deepest). A zero-field variant registers only the (no-op) destructure
/// (R7); no variant gets a setter, and per-field access is a receiver-
/// directed projection (P7 slice 1, R4) rather than a generated getter: a
/// `Type::Variant` value has no legal destination outside the arm that bound
/// it (R8).
///
/// The input slot is built through `variant_type`, the sole constructor of a
/// `Type::Variant`, so its leaked `Enum.Variant` display name has one origin
/// and every construction of the same `(EnumId, vi)` compares equal (R1).
/// Keying follows `struct_generated_sigs`' D7 rule: the env key is the bare
/// surface variant name, the lowering symbol the mangled registry spelling.
pub fn variant_generated_sigs(enums: &[EnumDecl]) -> Vec<(String, String, u32, Sig)> {
    let mut sigs = Vec::new();
    // Value-mode projection interns nothing, so this scratch registry stays
    // empty; the helper is shared with the accessor path so a destructure's
    // outputs and a field projection follow one rule (R4).
    let mut no_refs = Vec::new();
    for (idx, decl) in enums.iter().enumerate() {
        let id = EnumId::from_index(idx);
        for (vi, variant) in decl.variants.iter().enumerate() {
            let variant_ty = variant_type(enums, id, vi);
            let field_types = variant_field_projection(variant, None, &mut no_refs);
            let surface = generic_surface_name(&variant.name);
            sigs.push((
                format!("{surface}>"),
                format!("{}>", variant.name),
                decl.module,
                Sig {
                    inputs: vec![variant_ty],
                    outputs: field_types,
                },
            ));
        }
    }
    sigs
}

/// Phase 6 slice 3 (R2): per enum, the eliminator word `Enum?`'s `PolySig`:
/// `( ..a Enum ~[ ..a Enum.V1 -- ..b ] … ~[ ..a Enum.Vn -- ..b ] -- ..b )`.
/// One arm parameter per declared variant, in declaration order, each a `~`
/// (inline) quotation whose single fixed input is that variant and whose rows
/// are the two shared ones -- the arm reaches into the caller's region below
/// the scrutinee (`..a`) and leaves whatever the call leaves (`..b`).
///
/// The only free variables are those two rows: every arm input is a concrete
/// `Type::Variant` built through `variant_type`, so nothing per-arm unifies
/// and the signature carries no bounds, length variables, or type variables.
///
/// This signature is registration, not the call-site rule: `check_eliminator_call`
/// intercepts an eliminator call before the ordinary poly-call path ever
/// unifies against it, and resolves the scrutinee's mode (owning / `&` / `&!`)
/// per call site, which is why the arm inputs here are owning.
///
/// Keying follows `struct_generated_sigs`' D7 rule: the env key is the bare
/// surface name (`Shape?`) every call site writes, the lowering symbol the
/// mangled registry spelling (`Result[i64 i64]?`), so two instantiations of
/// one generic enum keep distinct **lowering** identities.
///
/// Review fix (Phase 2, smaller point 2): that distinctness is this `Sig`'s
/// alone. `eliminator_registry`, below, keys by `generic_surface_name` too,
/// so `Result[i64 i64]?` and `Result[bool i64]?` both collapse to the one
/// checker-side registry entry `"Result?"` -- last one registered wins, and
/// a call site can only ever reach whichever instantiation that was. This is
/// unreachable today only because a generic-elimination call site itself
/// cannot parse (`( Ok )` fails with "unknown type Ok", the standing
/// generic-enum-elimination blocker); Phase 4, wherever it closes that parse
/// gap, meets this collision too and needs a registry keyed by the mangled
/// spelling, not the bare surface name.
pub fn enum_eliminator_sigs(enums: &[EnumDecl]) -> Vec<(String, String, PolySig)> {
    // The two shared rows, in the signature's own id space.
    const ROW_IN: u32 = 0;
    const ROW_OUT: u32 = 1;
    let mut sigs = Vec::new();
    for (idx, decl) in enums.iter().enumerate() {
        let id = EnumId::from_index(idx);
        let mut inputs = vec![PolyType::Concrete(Type::Enum(id, decl.name_static))];
        for vi in 0..decl.variants.len() {
            inputs.push(PolyType::Quotation(
                vec![PolyType::Concrete(variant_type(enums, id, vi))],
                vec![],
                true,
                Some(ROW_IN),
                Some(ROW_OUT),
            ));
        }
        sigs.push((
            format!("{}?", generic_surface_name(&decl.name)),
            format!("{}?", decl.name),
            PolySig {
                row_in: Some(ROW_IN),
                inputs,
                outputs: vec![],
                row_out: Some(ROW_OUT),
                bounds: vec![],
                ty_kinds: vec![],
                ty_var_names: vec![],
                ty_var_spans: vec![],
                len_var_names: vec![],
                len_var_spans: vec![],
                row_var_names: vec!["..a".to_string(), "..b".to_string()],
            },
        ));
    }
    sigs
}

/// P7.S12 (R2.1): what a name in the eliminator registry names -- one
/// monomorph, or a generic header no instantiation has grounded yet. `idx` is
/// a `u32`, matching `PolyType::Generic`'s own field type, and indexes the
/// instantiator's `enums`.
///
/// `PolyType::Generic`'s other identity half, `module`, is deliberately *not*
/// carried: declaring-module identity is the wrong notion at an instantiation
/// site, so no consumer reads it, and nothing in R1-R7 needs to disambiguate
/// two same-named headers from different modules. A future phase that does can
/// add it back with a multi-module fixture to witness it.
///
/// R2 second half: this value never decides *identity*. Both consumers read
/// the operative header off the scrutinee's own type
/// (`check_eliminator_call`'s S3b R5 rule, restated in `poly_eliminator_call`
/// by R1.1); the registry entry gates the call *name* and supplies the
/// family-invariant variant count the underflow diagnostic needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EliminatorTarget {
    Concrete(EnumId),
    Generic { idx: u32 },
}

/// Phase 6 slice 3 (R3): the checker-side eliminator registry, bare surface
/// name (`Shape?`) -> the enum family it eliminates. `check_term` consults
/// this before the ordinary env/combinator/poly paths, since an eliminator
/// call is neither a user word nor an ordinary poly call: its arms are matched
/// to variants by annotation tag, not by slot position.
///
/// P7.S12 (R2.1): built from the module's monomorphized `enums` **and** from
/// the instantiator's own `GenericEnumDecl` headers, so `Option?` is a key
/// even where nothing in the program instantiates `Option` concretely -- the
/// brief's whole defect A.
///
/// R2.3: a header that also has monomorphs registers **once**, as `Concrete`.
/// That is what the concrete consumer expects, and R1.1/R5.1 make the choice
/// non-load-bearing: whichever way a name is classified, the branch a call
/// site takes is selected by its scrutinee, not by this value. The key stays
/// `generic_surface_name`-only, so every monomorph of one header shares one
/// entry -- a family gate, deliberately, not an identity.
pub fn eliminator_registry(
    enums: &[EnumDecl],
    generic_enums: &[GenericEnumDecl],
) -> HashMap<String, EliminatorTarget> {
    let mut registry: HashMap<String, EliminatorTarget> = enums
        .iter()
        .enumerate()
        .map(|(idx, decl)| {
            (
                format!("{}?", generic_surface_name(&decl.name)),
                EliminatorTarget::Concrete(EnumId::from_index(idx)),
            )
        })
        .collect();
    for (idx, decl) in generic_enums.iter().enumerate() {
        registry
            .entry(format!("{}?", generic_surface_name(&decl.name)))
            .or_insert(EliminatorTarget::Generic { idx: idx as u32 });
    }
    registry
}

/// Phase 6 slice 3 review fix (smaller point 1): a user word whose surface
/// name equals a generated eliminator's (`"{Enum}?"`) would be silently
/// unreachable -- `check_term`'s eliminator interception runs ahead of the
/// ordinary env lookup, so every call to that name always routes to the
/// eliminator and the user's own declaration can never be reached. An
/// eliminator has no overload mechanism to fall back through the way a
/// generated destructure (`P>`) does, so rejecting the declaration, the same
/// as `check_duplicate_word_names` rejects any other name collision before
/// either candidate enters an environment, is the only alternative to
/// shadowing it. Compares demangled surface names, so this holds whether
/// `words`/`enums` are still raw (`check_src`-style tests) or already
/// mangled (a real build, where the two collide under different manglings --
/// `Shape?__m0` vs `Shape__m0?` -- and would otherwise go undetected here).
pub fn check_no_word_shadows_eliminator(
    words: &[WordDef],
    enums: &[EnumDecl],
) -> Result<(), String> {
    for enum_decl in enums {
        let eliminator_name = format!(
            "{}?",
            crate::resolve::demangle_word(generic_surface_name(&enum_decl.name))
        );
        if let Some(word) = words.iter().find(|w| {
            w.module == enum_decl.module
                && crate::resolve::demangle_word(&w.name) == eliminator_name
        }) {
            return Err(word_shadows_eliminator_error(
                &eliminator_name,
                word_span(word),
                crate::resolve::demangle_word(generic_surface_name(&enum_decl.name)),
            ));
        }
    }
    Ok(())
}

/// The rejection above, as a message.
pub(crate) fn word_shadows_eliminator_error(
    eliminator_name: &str,
    span: Span,
    enum_name: &str,
) -> String {
    format!(
        "error: word `{eliminator_name}` (line {}, col {}) has the same name as the generated eliminator for enum `{enum_name}`; rename one",
        span.line, span.col,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    /// P7 slice 3c (R7): a slice at the FFI boundary is rejected with its own
    /// located message, not silently classified as a scalar and not
    /// mis-described as an "owned aggregate" (a view owns nothing). Only the
    /// input direction reaches here: `check_extern_decls` runs
    /// `check_reference_free_signature` first, which rejects a slice *output*
    /// through the ordinary no-stored-reference ban (R5).
    #[test]
    fn extern_boundary_rejects_slice_with_located_error() {
        let mut slices = Vec::new();
        let slice = crate::ast::intern_slice_type(&mut slices, Type::I64, false);
        let decl = ExternDecl {
            name: "consume".to_string(),
            symbol: "consume".to_string(),
            effect: StackEffect {
                inputs: vec![TypedSlot {
                    name: None,
                    ty: slice,
                }],
                outputs: Vec::new(),
            },
            span: Span {
                line: 3,
                col: 1,
                module: 0,
            },
            module: 0,
        };
        assert_eq!(
            check_extern_boundary_types(&decl, &[]).unwrap_err(),
            "error: extern: `consume` declares the input `Slice[i64]` (line 3, col 1)\n  a slice is a pointer and a length, which matches no C parameter; declare `&T` and pass the length as a separate `usize`"
        );
        // The reference it views crosses fine, which is what the message
        // directs the reader to.
        let mut refs = Vec::new();
        let ok = ExternDecl {
            effect: StackEffect {
                inputs: vec![
                    TypedSlot {
                        name: None,
                        ty: crate::ast::intern_ref_type(&mut refs, Type::I64, false),
                    },
                    TypedSlot {
                        name: None,
                        ty: Type::Usize,
                    },
                ],
                outputs: Vec::new(),
            },
            ..decl
        };
        check_extern_boundary_types(&ok, &[])
            .expect("`&i64` plus a `usize` length is a C prototype");
    }

    #[test]
    fn enum_eliminator_sig_is_two_rows_over_concrete_variant_arms() {
        // R2/OQ3: the generated signature's only free variables are the two
        // shared rows -- no bounds, no length variables, no type variables --
        // and each arm input is the *variant*, not the enum, in declaration
        // order. A regression to `Type::Enum` arm inputs (or a dropped row)
        // would make every arm interchangeable and lose the narrowing the
        // eliminator exists for.
        let src = "type: Shape | Circle r i64 | Rect w i64 h i64 ;\n: main ( -- ) ;\n";
        let module = parse(&lex(src).unwrap()).unwrap();
        let (key, symbol, sig) = enum_eliminator_sigs(&module.enums)
            .into_iter()
            .find(|(k, _, _)| k == "Shape?")
            .expect("every enum generates an eliminator");
        assert_eq!(symbol, "Shape?");
        assert_eq!(key, "Shape?");
        assert_eq!(sig.row_var_names.len(), 2);
        assert!(sig.bounds.is_empty());
        assert!(sig.len_var_names.is_empty());
        assert!(sig.ty_var_names.is_empty());
        assert!(sig.outputs.is_empty());
        assert_ne!(sig.row_in, sig.row_out);
        assert!(sig.row_in.is_some() && sig.row_out.is_some());

        let id = module
            .enums
            .iter()
            .position(|e| e.name == "Shape")
            .map(EnumId::from_index)
            .unwrap();
        assert_eq!(
            sig.inputs[0],
            PolyType::Concrete(Type::Enum(id, module.enums[id.index()].name_static))
        );
        let arms: Vec<&PolyType> = sig.inputs[1..].iter().collect();
        assert_eq!(arms.len(), 2);
        for (vi, arm) in arms.iter().enumerate() {
            let PolyType::Quotation(ins, outs, is_inline, row_in, row_out) = arm else {
                panic!("an arm parameter is a quotation: {arm:?}")
            };
            assert_eq!(
                ins,
                &vec![PolyType::Concrete(variant_type(&module.enums, id, vi))]
            );
            assert!(outs.is_empty());
            assert!(*is_inline, "an arm is a `~` quotation");
            assert_eq!(*row_in, sig.row_in);
            assert_eq!(*row_out, sig.row_out);
        }
        assert_ne!(arms[0], arms[1], "the two arms narrow to distinct variants");
    }

    #[test]
    fn eliminator_registry_keys_the_bare_surface_name() {
        let src = "type: Shape | Circle r i64 | Rect w i64 h i64 ;\n: main ( -- ) ;\n";
        let module = parse(&lex(src).unwrap()).unwrap();
        let registry = eliminator_registry(&module.enums, &[]);
        let id = module
            .enums
            .iter()
            .position(|e| e.name == "Shape")
            .map(EnumId::from_index)
            .unwrap();
        assert_eq!(
            registry.get("Shape?"),
            Some(&EliminatorTarget::Concrete(id))
        );
    }

    /// P7.S12 (R2.3, R8.8): a generic header with a monomorph registers as
    /// `Concrete` -- once, under the one shared surface key -- and one with
    /// no monomorph registers as `Generic`. Both directions, since the whole
    /// point of the `Generic` arm is the header nothing has grounded.
    #[test]
    fn eliminator_registry_prefers_a_monomorph_over_its_own_header() {
        let src = "type: Grounded['A] | GNil | GOne 'A ;\n\
             type: Bare['A] | BNil | BOne 'A ;\n\
             : mk ( i64 -- Grounded[i64] ) GOne ;\n\
             : main ( -- ) ;\n";
        let module = parse(&lex(src).unwrap()).unwrap();
        let generics = &module.generics;
        let registry = eliminator_registry(&module.enums, &generics.enums);
        let mono = module
            .enums
            .iter()
            .position(|e| generic_surface_name(&e.name) == "Grounded")
            .map(EnumId::from_index)
            .expect("`Grounded[i64]` is instantiated at parse time");
        assert_eq!(
            registry.get("Grounded?"),
            Some(&EliminatorTarget::Concrete(mono)),
            "a header with a monomorph registers as that monomorph"
        );
        let bare = generics
            .enums
            .iter()
            .position(|e| generic_surface_name(&e.name) == "Bare")
            .expect("`Bare`'s header is staged by the parse-time prepass");
        assert_eq!(
            registry.get("Bare?"),
            Some(&EliminatorTarget::Generic { idx: bare as u32 }),
            "a header with no monomorph registers as the header itself"
        );
    }

    #[test]
    fn variant_accessor_sigs_reach_the_module_env() {
        // R6/R-OQ2: the whole-variant destructure registers globally now,
        // with no eliminator to mint an operand. Nothing can call one
        // legally yet, so what discriminates "registered" from "never wired
        // into `check`" is *which* diagnostic a bare call gets: a registered
        // word underflows, an unregistered one is an unknown word. The
        // zero-field row is R7: `Dot>` registers.
        let err =
            check_src("type: Shape | Circle r i64 | Dot ;\n: main ( -- ) Circle> ;\n").unwrap_err();
        assert!(
            err.contains("`Circle>` needs 1 values"),
            "unexpected message: {err}"
        );
        let err =
            check_src("type: Shape | Circle r i64 | Dot ;\n: main ( -- ) Dot> ;\n").unwrap_err();
        assert!(err.contains("`Dot>` needs 1 values"), "unexpected: {err}");
        // Per-field access is a receiver-directed projection now (R4), not a
        // generated word, so `Circle>r`/`Dot>x` are simply unknown.
        let err = check_src("type: Shape | Circle r i64 | Dot ;\n: main ( -- ) Circle>r ;\n")
            .unwrap_err();
        assert!(err.contains("unknown word `Circle>r`"), "unexpected: {err}");
        let err =
            check_src("type: Shape | Circle r i64 | Dot ;\n: main ( -- ) Dot>x ;\n").unwrap_err();
        assert!(err.contains("unknown word `Dot>x`"), "unexpected: {err}");
    }

    #[test]
    fn type_node_treats_variant_as_a_non_edge_leaf() {
        // Phase 6 slice 2 (R3): a bare `Type::Variant` is never a struct/enum
        // field, so it closes no by-value containment cycle -- treated as a
        // leaf, exactly like `Ref`/`Quotation`.
        let ty = Type::Variant(EnumId::from_index(0), 0, "Shape.Circle");
        assert!(type_node(&ty).is_none());
    }

    #[test]
    fn collect_poly_concrete_sees_through_a_reference() {
        // Slice 13 (R-A9): a concrete type named behind a `&` is still named,
        // so export-privacy (which reads this collection) must see it. Only a
        // *variable-bearing* referent reaches this arm -- a fully concrete one
        // folds to `Concrete(Type::Ref)` at parse time -- so the test builds
        // that shape directly rather than through a source signature.
        let inner = PolyType::Array(Box::new(PolyType::Concrete(Type::I64)), Len::Var(0));
        let pt = PolyType::Ref(Box::new(inner), false);
        let mut out = Vec::new();
        collect_poly_concrete(&pt, &mut out);
        assert_eq!(out, vec![Type::I64]);
    }

    #[test]
    fn collect_poly_concrete_sees_through_an_owned_cell() {
        // P7.S3n (R3): the cell twin of the arm above, and for the same
        // reason -- a private type named as a `^` payload is still named.
        // Built directly for the same reason too: a fully-concrete payload
        // folds to `Concrete(Type::OwnedCell)` at parse time, so only a
        // variable-bearing one reaches this arm, and reaching it with a
        // *concrete leaf* inside needs a shape (a generic application's mixed
        // argument list) that no source signature spells in one token.
        let payload = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0), PolyType::Concrete(Type::I64)],
            len_args: vec![],
            name: "Ent",
        };
        let pt = PolyType::OwnedCell(Box::new(payload));
        let mut out = Vec::new();
        collect_poly_concrete(&pt, &mut out);
        assert_eq!(out, vec![Type::I64]);
    }

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module)
    }
    #[test]
    fn stored_reference_in_a_positional_variant_field_is_named_by_index() {
        // OQ4/Phase 1: an attributeless field's stored name is an internal
        // placeholder, so every diagnostic that prints a variant field name
        // must identify it by position instead of leaking the placeholder.
        let err = check_src(
            "type: Option['T] | None | Some 'T ;\n: w ( Option[&i64] -- ) drop ;\n: main ( -- ) ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("payload field 0 of variant `Some[&i64]`"),
            "unexpected message: {err}"
        );
        assert!(
            !err.contains(crate::parser::POSITIONAL_FIELD_NAME),
            "the internal placeholder leaked into a diagnostic: {err}"
        );
    }

    /// A checked module, for the tests that read a type fact back out of the
    /// registries rather than only asserting a diagnostic.
    fn checked_module(src: &str) -> Module {
        let tokens = lex(src).unwrap();
        let mut module = crate::test_support::parse_with_core(&tokens).unwrap();
        check(&mut module).unwrap();
        module
    }
    /// `File`, whose only field is an `i64`, with a `drop` overload: the shape
    /// every R3/R4 test turns on, since the structural fold alone would call
    /// it `Copy`.
    const FILE_RESOURCE: &str = "type: File fd i64 ; : drop ( File -- ) | f | f File> drop ;";
    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | s Spy> drop ;\n";
    fn struct_ty(module: &Module, name: &str) -> Type {
        let idx = module
            .structs
            .iter()
            .position(|s| s.name == name)
            .expect("declared struct");
        Type::Struct(StructId::from_index(idx), module.structs[idx].name_static)
    }
    /// U7 (R18): the exported-signature helper flags a word whose effect
    /// names a private type of its own module, and clears once that type is
    /// exported too (the positive half, R18's own escape hatch).
    #[test]
    fn exported_signature_rule_flags_private_type() {
        use crate::ast::{ModuleInfo, TypedSlot};
        let structs = vec![StructDecl {
            name: "Res".to_string(),
            name_static: "Res",
            fields: vec![("n".to_string(), Type::I64)],
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let mk_word = WordDef {
            name: "mk".to_string(),
            effect: StackEffect {
                inputs: Vec::new(),
                outputs: vec![TypedSlot {
                    name: None,
                    ty: Type::Struct(StructId::from_index(0), "Res"),
                }],
            },
            body: Vec::new(),
            poly: None,
            declares_inline: false,
            module: 0,
            span: Span::default(),
            declared_globals: None,
            is_trait_member: false,
        };
        let mut module = Module {
            words: vec![mk_word],
            structs,
            enums: Vec::new(),
            arrays: Vec::new(),
            owned_cells: Vec::new(),
            refs: Vec::new(),
            slices: Vec::new(),
            generic_structs: Vec::new(),
            generic_enums: Vec::new(),
            generics: crate::ast::GenericTypes::default(),
            externs: Vec::new(),
            instantiations: HashMap::new(),
            poly_cross_calls: HashMap::new(),
            transitive_instantiations: Vec::new(),
            splice_records: std::collections::HashMap::new(),
            splice_trait_calls: std::collections::HashMap::new(),
            splice_enum_words: std::collections::HashMap::new(),
            builtin_overloads: HashMap::new(),
            resolved_fields: HashMap::new(),
            resolved_variant_fields: HashMap::new(),
            modules: vec![ModuleInfo {
                exports: vec![("mk".to_string(), Span::default())],
                ..ModuleInfo::default()
            }],
            statics: Vec::new(),
            traits: Vec::new(),
            impls: Vec::new(),
        };

        let err = check_exported_signatures(&module).unwrap_err();
        assert!(err.contains("mk"), "names the word: {err}");
        assert!(err.contains("Res"), "names the private type: {err}");

        module.modules[0]
            .exports
            .push(("Res".to_string(), Span::default()));
        assert!(
            check_exported_signatures(&module).is_ok(),
            "exporting the type clears the rule"
        );
    }
    /// U8 (R20/R21): the selective-import validator rejects a name absent from
    /// its source module's export list (R20), two selective imports of one name
    /// (R21, naming both sources), and a selective name colliding with a local
    /// word (R21), while a clean import passes.
    #[test]
    fn selective_import_collision_is_rejected() {
        use crate::ast::ModuleInfo;

        fn info(exports: &[&str]) -> ModuleInfo {
            ModuleInfo {
                exports: exports
                    .iter()
                    .map(|n| (n.to_string(), Span::default()))
                    .collect(),
                ..ModuleInfo::default()
            }
        }
        fn word(name: &str, module: u32) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect::default(),
                body: Vec::new(),
                poly: None,
                declares_inline: false,
                module,
                span: Span::default(),
                declared_globals: None,
                is_trait_member: false,
            }
        }
        fn module_with(words: Vec<WordDef>, modules: Vec<ModuleInfo>) -> Module {
            Module {
                words,
                structs: Vec::new(),
                enums: Vec::new(),
                arrays: Vec::new(),
                owned_cells: Vec::new(),
                refs: Vec::new(),
                slices: Vec::new(),
                generic_structs: Vec::new(),
                generic_enums: Vec::new(),
                generics: crate::ast::GenericTypes::default(),
                externs: Vec::new(),
                instantiations: HashMap::new(),
                poly_cross_calls: HashMap::new(),
                transitive_instantiations: Vec::new(),
                splice_records: std::collections::HashMap::new(),
                splice_trait_calls: std::collections::HashMap::new(),
                splice_enum_words: std::collections::HashMap::new(),
                builtin_overloads: HashMap::new(),
                resolved_fields: HashMap::new(),
                resolved_variant_fields: HashMap::new(),
                modules,
                statics: Vec::new(),
                traits: Vec::new(),
                impls: Vec::new(),
            }
        }
        fn sel(name: &str, qualifier: &str, target: u32, line: u32) -> SelectiveName {
            SelectiveName {
                name: name.to_string(),
                qualifier: Some(qualifier.to_string()),
                target,
                span: Span {
                    line,
                    col: 1,
                    module: 0,
                },
            }
        }
        fn wildcard_sel(name: &str, target: u32, line: u32) -> SelectiveName {
            SelectiveName {
                name: name.to_string(),
                qualifier: None,
                target,
                span: Span {
                    line,
                    col: 1,
                    module: 0,
                },
            }
        }

        // R21: modules 1 and 2 each export `p`; module 0 selectively imports it
        // from both, colliding at the second.
        let m = module_with(
            vec![word("p", 1), word("p", 2)],
            vec![info(&[]), info(&["p"]), info(&["p"])],
        );
        let entries = vec![
            vec![sel("p", "a", 1, 1), sel("p", "b", 2, 2)],
            Vec::new(),
            Vec::new(),
        ];
        let err = check_selective_imports(&m, &entries).unwrap_err();
        assert!(err.contains("collides"), "selective collision: {err}");
        assert!(
            err.contains("`a`") && err.contains("`b`"),
            "names both sources: {err}"
        );

        // R20: a name absent from its source's export list is the visibility
        // error, distinct from a collision.
        let m = module_with(vec![word("grow", 1)], vec![info(&[]), info(&[])]);
        let err =
            check_selective_imports(&m, &[vec![sel("grow", "lib", 1, 1)], Vec::new()]).unwrap_err();
        assert!(err.contains("not exported"), "R20 export gate: {err}");
        assert!(!err.contains("collides"), "not the collision error: {err}");

        // R21: a selective name colliding with the importer's own local word.
        let m = module_with(
            vec![word("p", 0), word("p", 1)],
            vec![info(&[]), info(&["p"])],
        );
        let err =
            check_selective_imports(&m, &[vec![sel("p", "lib", 1, 1)], Vec::new()]).unwrap_err();
        assert!(
            err.contains("collides") && err.contains("local"),
            "local collision: {err}"
        );

        // A clean selective import of an exported, non-colliding name passes.
        let m = module_with(vec![word("p", 1)], vec![info(&[]), info(&["p"])]);
        assert!(check_selective_imports(&m, &[vec![sel("p", "lib", 1, 1)], Vec::new()]).is_ok());

        // P8 S2 (R1): a wildcard-desugared entry (no qualifier) colliding
        // with a local declaration is a wildcard-specific wording, not a
        // fabricated qualifier.
        let m = module_with(
            vec![word("p", 0), word("p", 1)],
            vec![info(&[]), info(&["p"])],
        );
        let err =
            check_selective_imports(&m, &[vec![wildcard_sel("p", 1, 1)], Vec::new()]).unwrap_err();
        assert!(
            err.contains("wildcard import of `p`") && err.contains("collides with a local"),
            "wildcard collision wording: {err}"
        );

        // P7.S3q (R5), both ways on one fixture pair. Module 0 declares its own
        // destructor `drop` and imports `drop` from two hubs -- a local
        // collision and a diamond at once. While the hubs admit the intrinsic
        // the entries bind no word and carry no module identity, so neither
        // rule has a subject; strip the admission and both rules apply again,
        // proving the exemption is what carries this and not the name.
        let hubs = |admits: bool| {
            let hub = || match admits {
                true => ModuleInfo {
                    intrinsics: crate::ast::IntrinsicVisibility::Only(["drop".to_string()].into()),
                    ..info(&["drop"])
                },
                false => info(&["drop"]),
            };
            module_with(vec![word("drop", 0)], vec![info(&[]), hub(), hub()])
        };
        let entries = vec![
            vec![sel("drop", "a", 1, 1), sel("drop", "b", 2, 2)],
            Vec::new(),
            Vec::new(),
        ];
        check_selective_imports(&hubs(true), &entries)
            .expect("an admitted intrinsic collides with neither the local decl nor its twin");
        let err = check_selective_imports(&hubs(false), &entries).unwrap_err();
        assert!(
            err.contains("collides with a local definition of `drop`"),
            "without the admission the ordinary rules still apply: {err}"
        );

        // P7.S3q (R5): admission alone is not the exemption. A hub that also
        // *declares* `drop` exports that declaration, so the entry binds a
        // word again and the local collision is real.
        let declaring_hub = ModuleInfo {
            intrinsics: crate::ast::IntrinsicVisibility::Only(["drop".to_string()].into()),
            ..info(&["drop"])
        };
        let m = module_with(
            vec![word("drop", 0), word("drop", 1)],
            vec![info(&[]), declaring_hub],
        );
        let err =
            check_selective_imports(&m, &[vec![sel("drop", "a", 1, 1)], Vec::new()]).unwrap_err();
        assert!(
            err.contains("collides with a local definition of `drop`"),
            "a declaring source is not exempt: {err}"
        );

        // Review fix, P0 #2: admission alone is not the exemption when the
        // source *re-exports* a real declaration either -- a source that
        // selectively imports the name from a further module (rather than
        // declaring it itself) exports that re-exported word, so the entry
        // binds a word and the local collision is real. Module 1 is the hub:
        // it admits `dup` as an intrinsic and selectively re-exports module
        // 2's real declaration of `dup`; module 0 has its own local `dup`
        // and selectively imports the hub's, which must collide.
        let re_exporting_hub = ModuleInfo {
            intrinsics: crate::ast::IntrinsicVisibility::Only(["dup".to_string()].into()),
            selective: HashMap::from([("dup".to_string(), 2u32)]),
            ..info(&["dup"])
        };
        let m = module_with(
            vec![word("dup", 0), word("dup", 2)],
            vec![info(&[]), re_exporting_hub, info(&["dup"])],
        );
        let err =
            check_selective_imports(&m, &[vec![sel("dup", "h", 1, 1)], Vec::new(), Vec::new()])
                .unwrap_err();
        assert!(
            err.contains("collides with a local definition of `dup`"),
            "a re-exporting source is not exempt either: {err}"
        );
    }

    /// U3 (R12): the duplicate-type-name check partitions by owning module, so
    /// two modules each declaring `Point` is not a duplicate, while two `Point`
    /// decls in one module still is (reported by the raw `name_static`, not the
    /// resolver's mangled `name`).
    #[test]
    fn duplicate_type_check_is_per_module() {
        let mk = |module: u32| StructDecl {
            name: format!("Point__m{module}"),
            name_static: "Point",
            fields: Vec::new(),
            span: crate::ast::Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module,
        };
        // Two modules, one `Point` each: not a duplicate.
        assert!(check_duplicate_type_names(&[mk(0), mk(1)], &[], &[], &[]).is_ok());
        // Same module, two `Point`: a duplicate, named by the raw surface name.
        let same_module = vec![
            StructDecl {
                name: "Point".to_string(),
                name_static: "Point",
                fields: Vec::new(),
                span: crate::ast::Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
            StructDecl {
                name: "Point".to_string(),
                name_static: "Point",
                fields: Vec::new(),
                span: crate::ast::Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: 0,
            },
        ];
        let err = check_duplicate_type_names(&same_module, &[], &[], &[]).unwrap_err();
        assert!(err.contains("duplicate type `Point`"), "raw name: {err}");
    }
    /// Phase 5 slice 1 review fix: a generic header mints no concrete
    /// registry entry (`prepass_type_decls` skips it), so without threading
    /// `generic_structs`/`generic_enums` through this check, a generic
    /// `Box` could collide with a concrete `Box` -- or another generic
    /// `Box` -- undetected.
    #[test]
    fn duplicate_type_check_includes_generic_headers() {
        let concrete_box = StructDecl {
            name: "Box".to_string(),
            name_static: "Box",
            fields: Vec::new(),
            span: crate::ast::Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        };
        let generic_box = |module: u32| crate::ast::GenericStructDecl {
            name: "Box".to_string(),
            ty_var_names: vec!["'T".to_string()],
            ty_kinds: vec![crate::ast::Kind::Star],
            len_var_names: vec![],
            fields: Vec::new(),
            span: crate::ast::Span::default(),
            module,
        };
        // A generic `Box` colliding with a concrete `Box` in the same module.
        let err = check_duplicate_type_names(
            std::slice::from_ref(&concrete_box),
            &[],
            &[generic_box(0)],
            &[],
        )
        .unwrap_err();
        assert!(
            err.contains("duplicate type `Box`"),
            "generic vs concrete: {err}"
        );
        // Two generic `Box` headers in the same module.
        let err = check_duplicate_type_names(&[], &[], &[generic_box(0), generic_box(0)], &[])
            .unwrap_err();
        assert!(
            err.contains("duplicate type `Box`"),
            "generic vs generic: {err}"
        );
        // The same generic `Box` split across two modules is not a duplicate.
        assert!(
            check_duplicate_type_names(&[], &[], &[generic_box(0), generic_box(1)], &[]).is_ok()
        );
    }
    /// Round-2 review fix: `check_duplicate_type_names` seeing a generic
    /// vs. generic collision (above) is only load-bearing if two literal
    /// `type:` headers of the same name actually both reach the registry.
    /// The parse-time idempotency guard added for slice 2's whole-closure
    /// pre-pass (`generic_header_at_cursor_is_registered`) used to conflate
    /// "registered by an earlier pass" with "registered by an earlier header
    /// in this very pass", so the second of two real declarations vanished
    /// before this check ever ran on it -- the direct-construction test above
    /// can't see that, since it builds both `GenericStructDecl`s by hand.
    #[test]
    fn duplicate_generic_struct_header_through_real_source_is_error() {
        let err = check_src("type: Box['T] val 'T ;\ntype: Box['T] val 'T ;\n").unwrap_err();
        assert!(err.contains("duplicate type `Box`"), "unexpected: {err}");
    }
    #[test]
    fn duplicate_generic_enum_header_through_real_source_is_error() {
        let err = check_src("type: E['T] | V a 'T ;\ntype: E['T] | V a 'T ;\n").unwrap_err();
        assert!(err.contains("duplicate type `E`"), "unexpected: {err}");
    }
    /// Two words of the same name in one module are rejected; the same pair
    /// split across two modules is not (mirrors `duplicate_type_check_is_per_module`).
    #[test]
    fn duplicate_word_name_is_rejected_only_within_one_module() {
        fn word_at(name: &str, module: u32, line: u32) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect::default(),
                body: Vec::new(),
                poly: None,
                declares_inline: false,
                module,
                span: Span {
                    line,
                    col: 1,
                    module: 0,
                },
                declared_globals: None,
                is_trait_member: false,
            }
        }
        fn word(name: &str, module: u32) -> WordDef {
            word_at(name, module, 0)
        }

        // Two modules, one `push` each: not a duplicate.
        assert!(check_duplicate_word_names(&[word("push", 0), word("push", 1)]).is_ok());

        // Same module, two `push`: a duplicate, naming both locations.
        let err = check_duplicate_word_names(&[word_at("push", 0, 1), word_at("push", 0, 2)])
            .unwrap_err();
        assert!(
            err.contains("duplicate word `push`") && err.contains("line 2"),
            "names the repeat's location: {err}"
        );
        assert!(
            err.contains("first defined at line 1"),
            "also names the first definition's location: {err}"
        );

        // A repeat `main` in one module is caught too: nothing else validates
        // `main`'s multiplicity within a module.
        let err = check_duplicate_word_names(&[word("main", 0), word("main", 0)]).unwrap_err();
        assert!(err.contains("duplicate word `main`"), "names main: {err}");

        // Two `drop`s sharing a module are *not* rejected here: distinct-struct
        // overloading is `find_drop_overloads`'s job, keyed by struct id, not
        // this check's; re-flagging by name alone would reject Phase 3 slice
        // 8b's legitimate multi-type overloading.
        assert!(check_duplicate_word_names(&[word("drop", 0), word("drop", 0)]).is_ok());
    }
    #[test]
    fn extern_error_names_the_surface_word_not_the_mangled_decl() {
        // `resolve` mangles every declared name in place (`puts` ->
        // `puts__m0`) before the checker runs; the diagnostic must name the
        // word the user wrote (`render_word`), never the resolved symbol.
        let decl = ExternDecl {
            name: "puts__m0".to_string(),
            symbol: "puts".to_string(),
            effect: StackEffect {
                inputs: vec![TypedSlot {
                    name: None,
                    ty: Type::Str,
                }],
                outputs: Vec::new(),
            },
            span: Span {
                line: 1,
                col: 1,
                module: 0,
            },
            module: 0,
        };
        let err = check_extern_boundary_types(&decl, &[]).unwrap_err();
        assert!(err.contains("extern: `puts` "), "unexpected message: {err}");
        assert!(!err.contains("__m0"), "unexpected message: {err}");
    }

    #[test]
    fn check_extern_redeclaring_a_word_is_error() {
        // Criterion 5/R1: an `extern:` naming an already-registered word (a
        // user `:` word here) is a located error.
        let src = ": foo ( i64 -- i64 ) ;\nextern: foo ( i64 -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("foo"), "unexpected message: {err}");
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_redeclaring_a_builtin_is_error() {
        // Criterion 5/R1: every builtin `check_term` dispatches by name before
        // the env lookup, plus the `>`-prefixed conversion family. None is in
        // `builtin_table`, so without the `BUILTIN_WORDS` gate the declaration
        // would be accepted, never consulted, and silently do nothing.
        for name in crate::ast::BUILTIN_WORDS
            .iter()
            .copied()
            .chain([">u8", ">f64"])
        {
            let src = format!("extern: {name} ( i64 -- i64 ) \"s\" ;");
            let Err(err) = check_src(&src) else {
                panic!("`extern: {name}` was accepted");
            };
            assert!(
                err.contains("redeclares"),
                "unexpected message for `{name}`: {err}"
            );
        }
    }
    #[test]
    fn overload_exact_input_match_is_error() {
        // R1: two definitions with identical `(module, name, input_types)`
        // still hit the `duplicate word` message, byte-for-byte.
        let src = "type: Vec2 x i64 y i64 ;\n\
: dist ( Vec2 Vec2 -- i64 ) drop drop 0 ;\n\
: dist ( Vec2 Vec2 -- i64 ) drop drop 1 ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("duplicate word `dist`"),
            "unexpected message: {err}"
        );

        // R1: an overload whose input types exactly match a builtin row is a
        // located error too, naming the operand types.
        let src = "type: Vec2 x i64 y i64 ;\n\
: add ( i64 i64 -- i64 ) drop ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("overload of `add`") && err.contains("as a builtin"),
            "unexpected message: {err}"
        );
        assert!(err.contains("i64 i64"), "names the operand types: {err}");

        // Mutation check: two overloads of one name with *different* input
        // types no longer collide as a duplicate (the whole reason for R1's
        // widened key).
        let src = "type: Vec2 x i64 y i64 ;\n\
: dist ( Vec2 Vec2 -- i64 ) drop drop 0 ;\n\
: dist ( Vec2 -- i64 ) drop 0 ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            !err.contains("duplicate word"),
            "different input types must not collide as a duplicate: {err}"
        );
    }
    #[test]
    fn overload_arity_clash_is_error() {
        // R4: a local overload of `add` whose arity disagrees with the
        // builtin's is rejected at its own definition site.
        let src = "type: Vec2 x i64 y i64 ;\n\
: add ( Vec2 -- Vec2 ) ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("overload of `add`")
                && err.contains("takes 1 input but another `add` takes 2")
                && err.contains("must agree on input count"),
            "unexpected message: {err}"
        );

        // R4: two local overloads of a non-builtin name disagreeing on
        // arity, rejected at the second's site.
        let src = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( Vec2 Vec2 -- Vec2 ) drop ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("overload of `bump`") && err.contains("takes 2 input"),
            "unexpected message: {err}"
        );

        // Mutation check: two overloads agreeing on arity (even with
        // different input types) never hit this check.
        let ok = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( i64 -- i64 ) ;\n\
: main ( -- ) ;\n";
        check_src(ok).expect("same-arity overloads must not trip the arity check");
    }
    #[test]
    fn overload_generic_and_concrete_overlap_is_error() {
        // R5: a poly candidate overlapping the builtin `add` of the same
        // arity is rejected -- no specialization ordering.
        let src = ": add ( 'T 'T -- 'T ) drop ;\n: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("generic overload")
                && err.contains("overlaps a concrete overload of `add`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains(": add ( 'T 'T -- 'T )"),
            "renders the poly signature: {err}"
        );

        // R5: a poly candidate overlapping a *local* concrete overload of
        // the same name and arity.
        let src = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( 'T -- 'T ) ;\n\
: main ( -- ) ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("generic overload")
                && err.contains("overlaps a concrete overload of `bump`"),
            "unexpected message: {err}"
        );

        // Mutation check: a poly candidate of a *different* arity than
        // every concrete candidate for the name never trips the check.
        let ok = "type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( 'T 'T -- 'T ) drop ;\n\
: main ( -- ) ;\n";
        check_src(ok).expect("a differing-arity poly candidate must not trip the overlap check");
    }
    /// Fix 3 (R5, module-scoped): `check_generic_concrete_overlap` operates
    /// directly on `WordDef`s carrying pre-mangle bare names and hand-set
    /// module ids -- unlike a `check_src` scenario (always module 0), this
    /// exercises the cross-module key `resolve::mangle` would otherwise
    /// disambiguate before `check` ever ran on a real multi-file program, so
    /// the two candidates' bare names can actually collide by string here.
    #[test]
    fn overload_generic_and_concrete_overlap_is_module_scoped() {
        fn concrete_word(name: &str, module: u32, arity: usize) -> WordDef {
            WordDef {
                name: name.to_string(),
                effect: StackEffect {
                    inputs: (0..arity)
                        .map(|_| TypedSlot {
                            name: None,
                            ty: Type::I64,
                        })
                        .collect(),
                    outputs: Vec::new(),
                },
                body: Vec::new(),
                poly: None,
                declares_inline: false,
                module,
                span: Span::default(),
                declared_globals: None,
                is_trait_member: false,
            }
        }
        fn poly_word(name: &str, module: u32, arity: usize) -> WordDef {
            let sig = PolySig {
                row_in: None,
                inputs: (0..arity as u32).map(PolyType::Var).collect(),
                outputs: Vec::new(),
                row_out: None,
                bounds: Vec::new(),
                ty_kinds: vec![crate::ast::Kind::Star; arity],
                ty_var_names: (0..arity).map(|i| format!("'T{i}")).collect(),
                ty_var_spans: vec![Span::default(); arity],
                len_var_names: Vec::new(),
                len_var_spans: Vec::new(),
                row_var_names: Vec::new(),
            };
            WordDef {
                name: name.to_string(),
                effect: StackEffect {
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
                body: Vec::new(),
                poly: Some(Box::new(sig)),
                declares_inline: false,
                module,
                span: Span::default(),
                declared_globals: None,
                is_trait_member: false,
            }
        }

        // R5, module-scoped: an unrelated concrete `bump` in module 1 does
        // not overlap a poly `bump` of the same arity declared in module 0 --
        // pre-fix, both were keyed by the bare name alone, globally, so this
        // combination was rejected even though nothing imports across the
        // two modules.
        let words = vec![concrete_word("bump", 1, 1), poly_word("bump", 0, 1)];
        check_generic_concrete_overlap(&words, &[], &[], &[], &[], &[])
            .expect("an unrelated same-name concrete word in a different module must not overlap");

        // Mutation check: the *same*-module case must still be rejected --
        // module-scoping narrows the key, it does not disable the check.
        let words = vec![concrete_word("bump", 0, 1), poly_word("bump", 0, 1)];
        let err = check_generic_concrete_overlap(&words, &[], &[], &[], &[], &[]).unwrap_err();
        assert!(
            err.contains("generic overload")
                && err.contains("overlaps a concrete overload of `bump`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn overload_missing_at_call_site_is_error() {
        // R3: a builtin operator's exact-match miss falls back to its
        // existing operand-class diagnostic, byte-for-byte, even when a
        // *different* struct's overload of the same name exists in the
        // module (importing `Vec2` does not bring `add` for it, and a local
        // `Vec2 add` overload does not answer an `i64 Bool` call site
        // either).
        let src = "type: Vec2 x i64 y i64 ;\n\
: add ( Vec2 Vec2 -- Vec2 ) drop ;\n\
: main ( -- ) 1 True add drop ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("requires two operands of the same numeric type")
                && err.contains("`i64`")
                && err.contains("`Bool`"),
            "unexpected message: {err}"
        );

        // R3: a user-overloaded, non-operator name called with operands
        // that match no candidate names the operand types, the same as any
        // ordinary word call.
        let src = "type: Vec2 x i64 y i64 ;\n\
: describe ( Vec2 -- ) drop ;\n\
: main ( -- ) 1 describe ;\n";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("expected `Vec2`") && err.contains("found `i64`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_extern_shadowing_a_builtin_does_not_change_its_meaning() {
        // R1's reason for existing: before the gate, this compiled, and `dup`
        // at the call site still meant the builtin with no diagnostic at all.
        let src = "extern: dup ( i64 -- i64 ) \"mydup\" ;\n: main ( -- ) 1 dup drop drop ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_registers_its_effect_at_call_sites() {
        // Criterion 4/R1: registration is what makes the existing arity and
        // type checks apply to a foreign call unchanged. Parsing it is not
        // enough, so assert the effect is actually consulted.
        let ok =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) \"hi\" cstr strlen drop ;";
        check_src(ok).unwrap();
        let underflow =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) strlen drop ;";
        let err = check_src(underflow).unwrap_err();
        assert!(err.contains("strlen"), "unexpected message: {err}");
        let wrong_type =
            "extern: strlen ( cstr -- usize ) \"strlen\" ;\n: main ( -- ) True strlen drop ;";
        let err = check_src(wrong_type).unwrap_err();
        assert!(err.contains("strlen"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_redeclaring_another_extern_is_error() {
        let src = "extern: foo ( i64 -- i64 ) \"foo\" ;\nextern: foo ( i64 -- i64 ) \"bar\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("redeclares"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_with_drop_overload_is_linear() {
        // Criterion 1/R3: the override forces linearity, so a struct whose
        // every field is `Copy` is not `Copy`. Without the override the same
        // declaration folds to `Copy`, which is what makes this a real
        // decision rather than a restatement of the field fold.
        let module = checked_module(&format!("{FILE_RESOURCE} : main ( -- ) 1 File drop ;"));
        let file = struct_ty(&module, "File");
        assert!(!is_copy(
            file,
            &module.structs,
            &module.enums,
            &module.arrays
        ));
        assert!(is_linear(
            file,
            &module.structs,
            &module.enums,
            &module.arrays
        ));

        let plain = checked_module("type: File fd i64 ; : main ( -- ) 1 File drop ;");
        assert!(is_copy(
            struct_ty(&plain, "File"),
            &plain.structs,
            &plain.enums,
            &plain.arrays
        ));
    }
    #[test]
    fn check_extern_accepts_the_full_r2_boundary_type_set() {
        // R2: the numeric tower, `bool`, `&T`/`&!T`, and `cstr` may all cross
        // an `extern:` boundary in either position.
        let src = "extern: f1 ( i64 u8 usize isize f64 f32 Bool -- i64 ) \"f1\" ;\nextern: f2 ( &i64 &!i64 -- i64 ) \"f2\" ;\nextern: f3 ( cstr -- cstr ) \"f3\" ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_extern_with_str_parameter_is_error() {
        // R2/R7: a `str` is a descriptor handle (R4), not a scalar or a
        // single opaque `Ptr`, so it matches no C parameter; the rejection
        // names the total conversion to `cstr`.
        let src = "extern: f ( str -- i64 ) \"f\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("matches no C parameter") && err.contains("`cstr`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_extern_returning_str_is_error() {
        // R11: a returned `str` would be one not built from a literal, which
        // is the invariant R10's `Copy`/non-escaping status rests on.
        let src = "extern: f ( -- str ) \"f\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("cannot return a `str`") && err.contains("static data only"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_extern_with_aggregate_parameter_is_error() {
        // Criterion 11/R3: an owned aggregate (struct/enum/array) as an
        // `extern:` input is rejected at the declaration.
        let src = "type: Point x i64 y i64 ;\nextern: foo ( Point -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
        assert!(err.contains("Point"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_with_array_parameter_is_error() {
        let src = "extern: foo ( array[i64 4] -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_with_owned_pointer_parameter_is_error() {
        // R3: `^T` is an owned aggregate too, rejected in input position
        // with the generic aggregate message (the output-specific
        // "forge ownership" message is only for the output position).
        let src = "extern: foo ( ^i64 -- i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("owned aggregate"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_cannot_express_a_variadic_c_function() {
        // R3: `extern:`'s grammar has no syntax for a variadic parameter
        // list, so `printf` cannot be usefully declared: only a fixed
        // effect can be spelled, e.g. one `cstr` and nothing else.
        let src = "extern: printf ( cstr -- i64 ) \"printf\" ;";
        check_src(src).unwrap();
        let err =
            crate::parser::parse(&lex("extern: printf ( cstr ... -- i64 ) \"printf\" ;").unwrap())
                .unwrap_err();
        assert!(
            err.contains("unknown type `...`"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_extern_multi_output_is_error() {
        // Criterion 18/R8: a two-output `extern:` describes no C prototype.
        // Unrejected it lowered to a discarded result and panicked in the
        // *next* consumer of the value that was never pushed, naming the
        // wrong term; the diagnostic sits at the declaration instead.
        let src = "extern: two ( i64 -- i64 i64 ) \"two\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("extern: `two` declares 2 outputs")
                && err.contains("no C function returns more than one value"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_extern_returning_owned_pointer_is_error() {
        // Criterion 12/R3: an `extern:` returning `^T` is rejected: it would
        // forge ownership of memory the allocator did not hand out.
        let src = "extern: foo ( i64 -- ^i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("forge ownership"), "unexpected message: {err}");
    }
    #[test]
    fn check_extern_returning_a_reference_is_error() {
        // Criterion 13/R3: reusing the existing no-declared-output-reference
        // message rather than duplicating it.
        let src = "extern: foo ( i64 -- &i64 ) \"foo\" ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("a reference cannot be stored"),
            "unexpected message: {err}"
        );
    }
    // P7.S5 (R13): the `[Type; Count]` array constructor is deleted. These
    // tests verify the old body-level syntax no longer parses (the `[`
    // falls through to the quotation-literal arm, where `;` is an
    // unexpected token).

    #[test]
    fn array_constructor_i64_no_longer_parses() {
        let tokens = lex(": w ( -- ) [ i64 ; 10 ] drop ;").unwrap();
        let err = crate::test_support::parse_with_core(&tokens).unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }
    #[test]
    fn array_constructor_str_element_no_longer_parses() {
        let tokens = lex(": w ( -- ) [ str ; 4 ] drop ;").unwrap();
        let err = crate::test_support::parse_with_core(&tokens).unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }
    #[test]
    fn array_constructor_linear_element_no_longer_parses() {
        let src = format!("{SPY_DEF}: w ( -- ) [ Spy ; 4 ] drop ;");
        let tokens = lex(&src).unwrap();
        let err = crate::test_support::parse_with_core(&tokens).unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }
    #[test]
    fn fill_still_accepts_a_str_element() {
        // D4: `fill` replicates a real seed, so it keeps accepting `str`/
        // `cstr`/a quotation.
        check_src(": main ( -- ) \"hi\" 3 fill drop ;").unwrap();
    }
    #[test]
    fn check_value_recursion_through_array_element_is_error() {
        // X5/R14/M3: a struct containing itself via an array element is a
        // recursive definition (infinite size), caught by the DFS.
        let err = check_src("type: Node kids array[Node 4] ;").unwrap_err();
        assert!(err.contains("recursive"), "unexpected message: {err}");
        assert!(err.contains("Node"), "should name the cycle: {err}");
    }
    #[test]
    fn check_struct_generated_words_flat_struct_ok() {
        check_src(
            "type: Vec2 x i64 y i64 ;
             : main ( -- ) 1 2 Vec2 &x @ drop &y @ drop drop ;",
        )
        .unwrap();
    }
    #[test]
    fn check_struct_generated_words_nested_struct_ok() {
        check_src(
            "type: Vec2 x i64 y i64 ;
             type: Segment from Vec2 to Vec2 ;
             : main ( -- ) 1 2 Vec2 3 4 Vec2 Segment &from &x @ drop Segment> drop drop ;",
        )
        .unwrap();
    }
    #[test]
    fn check_struct_zero_field_registers_only_ctor_and_destructure() {
        check_src("type: Unit ; : main ( -- ) Unit Unit> ;").unwrap();
    }
    #[test]
    fn check_struct_per_field_accessor_spellings_are_unknown_words() {
        // P7 slice 1 (D1/R11 deletion guard): the only struct-generated words
        // are the constructor and the whole-struct destructure. Reinstating a
        // per-field row in `struct_generated_sigs` would type-check these
        // three spellings again with no lowering arm behind them, so this is
        // what discriminates "retired" from "still registered": a registered
        // word gets an arity/type diagnostic, a retired one is unknown. The
        // matching variant-side pin is `variant_accessor_sigs_reach_the_module_env`.
        for call in ["Vec2>x", "Vec2<x", "Vec2|>x"] {
            let err = check_src(&format!(
                "type: Vec2 x i64 y i64 ;\n: main ( -- ) 1 2 Vec2 {call} ;\n"
            ))
            .unwrap_err();
            assert!(
                err.contains(&format!("unknown word `{call}`")),
                "{call}: unexpected message: {err}"
            );
        }
    }
    #[test]
    fn check_struct_setter_returns_updated_struct_ok() {
        check_src("type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 2 Vec2 &!x 3 ! ;").unwrap();
    }
    #[test]
    fn check_struct_peek_copy_field_leaves_struct_live_ok() {
        // D2: `&x`'s owned-receiver arm is non-consuming, so the struct is
        // still on the stack for the second read and the trailing `Vec2>`
        // destructure.
        check_src("type: Vec2 x i64 y i64 ; : main ( -- ) 1 2 Vec2 &x @ drop Vec2> drop drop ;")
            .unwrap();
    }
    #[test]
    fn check_struct_peek_of_linear_field_is_error_at_the_read_not_the_projection() {
        // D2: producing `&a` is legal even for a linear field (it borrows,
        // never duplicates); the gate that used to sit on the retired
        // `S|>fi` peek now sits on `@` reading the linear referent instead,
        // the same rejection `!`'s D3 store-gate is symmetric with.
        let err = check_src(&format!(
            "{SPY_DEF}type: Holds a Spy b i64 ; : main ( -- ) 7 Spy 1 Holds &a @ drop drop ;"
        ))
        .unwrap_err();
        assert!(err.contains("`@`"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_duplicate_type_name_is_error() {
        // X2: two `type:` declarations sharing a name name that type.
        let err = check_src("type: Vec2 x i64 ; type: Vec2 y i64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Vec2"), "unexpected message: {err}");
    }
    #[test]
    fn check_recursion_by_value_self_cycle_is_error() {
        // X3/M5: a directly self-referential struct (no `^` anywhere
        // on the cycle) is an error naming the full path (a bare string,
        // no span), and this test itself is proof the checker terminated
        // rather than hung.
        let err = check_src("type: Loop next Loop ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Loop -> Loop"), "unexpected message: {err}");
    }
    #[test]
    fn check_recursion_by_value_mutual_cycle_is_error() {
        // X3/M5: a mutually-recursive pair of structs, no `^`
        // anywhere, names the full path A -> B -> A.
        let err = check_src("type: A b B ; type: B a A ;").unwrap_err();
        assert!(
            err.contains("recursive struct"),
            "unexpected message: {err}"
        );
        assert!(err.contains("A -> B -> A"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_direct_recursion_is_error_not_hang() {
        // X3/M5: a directly self-referential enum (a variant field of its own
        // type) is an error naming the cycle (bare, no span), and this
        // test's return is proof the DFS terminated rather than hung.
        let err = check_src("type: Loop | Wrap next Loop | End ;").unwrap_err();
        assert!(err.contains("recursive enum"), "unexpected message: {err}");
        assert!(err.contains("Loop"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_mutual_recursion_is_error_not_hang() {
        // X3/M5: a mutually-recursive pair of enums, names both in the cycle.
        let err = check_src("type: A | Ta x B ; type: B | Tb y A ;").unwrap_err();
        assert!(err.contains("recursive enum"), "unexpected message: {err}");
        assert!(err.contains('A'), "unexpected message: {err}");
        assert!(err.contains('B'), "unexpected message: {err}");
    }
    #[test]
    fn check_recursion_cell_cycle_in_struct_field_is_ok() {
        // A `^` edge through a struct field is legal, not just through
        // an enum variant payload -- the rule is about size finiteness, not
        // idiom.
        check_src("type: Node v i64 next ^Node ;").unwrap();
    }
    #[test]
    fn check_recursion_cell_cycle_in_enum_variant_is_ok() {
        // The same `^` cycle acceptance in enum variant position,
        // mirroring check_recursion_cell_cycle_in_struct_field_is_ok.
        check_src("type: List | Nil | Cons v i64 next ^List ;").unwrap();
    }
    #[test]
    fn check_recursion_array_element_cell_is_cut_then_ok() {
        // The `^` edge is cut inside an array element too, so this
        // definition survives the recursion rule.  The array destructor
        // (Phase 4) disposes each `^Node` cell, so the linear element is now
        // admitted rather than rejected.
        check_src("type: Node kids array[^Node 4] ;").unwrap();
    }
    #[test]
    fn check_struct_enum_mixed_recursion_is_error_not_hang() {
        // D9/X3: a struct field of enum type closing a cycle back to the
        // struct is caught by the combined-graph DFS.
        let err = check_src("type: S f E ; type: E | V g S ;").unwrap_err();
        assert!(err.contains("recursive"), "unexpected message: {err}");
        assert!(err.contains('S'), "unexpected message: {err}");
        assert!(err.contains('E'), "unexpected message: {err}");
    }
    #[test]
    fn linear_array_element_in_struct_field_is_ok() {
        // The parser cannot reject `[Spy N]` (struct fields aren't resolved
        // until the whole module is parsed).  The synthesized array destructor
        // (Phase 4) disposes each linear element, so this now compiles.
        check_src(&format!(
            "{SPY_DEF}type: Bag xs array[Spy 2] ; : main ( -- ) 0 drop ;"
        ))
        .unwrap();
    }
    #[test]
    fn linear_array_element_in_word_signature_is_ok() {
        check_src(&format!(
            "{SPY_DEF}: w ( array[Spy 2] -- ) | a | a drop ; : main ( -- ) 0 drop ;"
        ))
        .unwrap();
    }
    #[test]
    fn linear_array_element_indirect_via_linear_struct_field_is_ok() {
        // `Arr`'s element (`Holds`) is not itself `Spy`, but contains one
        // transitively; the array destructor calls `emit_drop` on each
        // element, which dispatches to `Holds`'s struct destructor.
        check_src(&format!(
            "{SPY_DEF}type: Holds s Spy ; type: Arr a array[Holds 2] ; : main ( -- ) 0 drop ;"
        ))
        .unwrap();
    }
    #[test]
    fn linear_array_element_indirect_via_linear_struct_in_signature_is_ok() {
        check_src(&format!(
            "{SPY_DEF}type: Holds s Spy ; : w ( array[Holds 2] -- ) | a | a drop ; : main ( -- ) 0 drop ;"
        ))
        .unwrap();
    }
    #[test]
    fn copy_array_element_is_ok() {
        check_src("type: V xs array[i64 4] ; : main ( -- ) 0 drop ;").unwrap();
    }
    /// P7 slice 3c (R1.2 phase 3 review fix): unlike `array_of_owned_is_error`,
    /// this element reaches `module.slices` through the *type spelling*
    /// (`Slice[^i64]` interned straight from the parser), with no `slice`
    /// construction call and no array in sight -- the route the original
    /// exit notes missed.
    #[test]
    fn check_slice_element_gate_owned_element_is_error() {
        let err =
            check_src(": w ( Slice[^i64] -- usize ) len ; : main ( -- ) 0 drop ;").unwrap_err();
        assert!(
            err.contains("linear slice elements are not supported yet"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`^i64`"), "unexpected message: {err}");
    }
    #[test]
    fn check_slice_element_gate_reference_element_is_error() {
        for elem in ["&i64", "&!i64"] {
            let src = format!(": w ( Slice[{elem}] -- usize ) len ; : main ( -- ) 0 drop ;");
            let err = check_src(&src).unwrap_err();
            assert!(
                err.contains("a reference cannot be stored"),
                "unexpected message for {elem}: {err}"
            );
        }
    }
    #[test]
    fn check_slice_element_gate_nested_slice_element_is_error() {
        let err = check_src(": w ( Slice[Slice[i64]] -- usize ) len ; : main ( -- ) 0 drop ;")
            .unwrap_err();
        assert!(
            err.contains("a reference cannot be stored"),
            "unexpected message: {err}"
        );
    }
    #[test]
    fn check_slice_element_gate_copy_element_is_ok() {
        check_src(": w ( Slice[i64] -- usize ) len ; : main ( -- ) 0 drop ;").unwrap();
    }
    #[test]
    fn array_of_owned_is_ok() {
        // An array of owning cells: the array destructor (Phase 4) calls
        // `emit_drop` on each `^i64`, which dispatches to the cell destructor.
        check_src(": w ( array[^i64 4] -- ) drop ; : main ( -- ) 0 drop ;").unwrap();
    }
    #[test]
    fn owned_of_linear_array_is_ok() {
        // An owning cell wrapping a linear array: the cell destructor drops
        // the payload (the array), which dispatches to the array destructor.
        check_src(&format!(
            "{SPY_DEF}: w ( ^array[Spy 2] -- ) drop ; : main ( -- ) 0 drop ;"
        ))
        .unwrap();
    }
    #[test]
    fn nested_array_of_owned_is_ok() {
        // A cell wrapping an array of cells: the cell destructor drops the
        // array, whose destructor drops each `^i64` cell.
        check_src(": w ( ^array[^i64 4] -- ) drop ; : main ( -- ) 0 drop ;").unwrap();
    }
    #[test]
    fn array_of_struct_holding_owned_is_ok() {
        // An array whose element only holds a cell transitively: the array
        // destructor disposes each `Holds`, whose struct destructor disposes
        // the `^i64` field.
        check_src("type: Holds c ^i64 ; type: Arr a array[Holds 2] ; : main ( -- ) 0 drop ;")
            .unwrap();
    }
    #[test]
    fn check_struct_and_enum_duplicate_name_across_registries_is_error() {
        // X2: a name used by one struct and one enum names that type.
        let err = check_src("type: Dup x i64 ; type: Dup | V ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Dup"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_nested_aggregate_fields_ok() {
        // D9: a variant may carry a struct, and a struct may carry an enum,
        // acyclically — no recursion error.
        check_src(
            "type: Vec2 x f64 y f64 ;
             type: Shape | Dot p Vec2 | Empty ;
             type: Tagged k Shape n i64 ;",
        )
        .unwrap();
    }
    #[test]
    fn check_struct_constructor_arity_mismatch_is_error() {
        // X4: too few values fed to the constructor, naming the struct.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 Vec2 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_constructor_field_type_mismatch_is_error() {
        // X4: a `bool` where an `i64` field is expected, naming struct+field type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 True Vec2 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Vec2"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`Bool`"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_equality_operator_is_error() {
        // X7, revised under P7.S3s R5: `eq` is now the library's non-inline
        // `'T: Copy Ord` word, so a struct with no `impl: Ord` is rejected at
        // the bound, not at a scalar-only operand-pair guard.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Bool ) 1 2 Vec2 1 2 Vec2 eq ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("does not satisfy `Ord`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_arithmetic_operator_is_error() {
        // X7: `add` on two structs is scalar-only, naming the struct type.
        let src = "type: Vec2 x i64 y i64 ; : main ( -- Vec2 ) 1 2 Vec2 1 2 Vec2 add ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`Vec2`"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_unifies_through_if_else_join_ok() {
        // R10: a struct type flows through an `if`/`else` join like any Type.
        check_src(
            "type: Vec2 x i64 y i64 ;
             : pick ( Bool -- Vec2 ) ~[ 1 2 Vec2 ] ~[ 3 4 Vec2 ] if ;",
        )
        .unwrap();
    }
    #[test]
    fn check_struct_moves_through_shuffles_ok() {
        // R10: dup/drop/swap/over move a struct value with no special case.
        check_src(
            "type: Vec2 x i64 y i64 ;
             : main ( -- Vec2 ) 1 2 Vec2 3 4 Vec2 swap drop dup drop ;",
        )
        .unwrap();
    }
    #[test]
    fn check_enum_zero_field_variant_constructor_ok() {
        check_src("type: Cmd | Halt ; : main ( -- Cmd ) Halt ;").unwrap();
    }
    #[test]
    fn check_enum_multi_field_variant_constructor_ok() {
        check_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : main ( -- Shape ) 2.0 Circle ;",
        )
        .unwrap();
    }
    #[test]
    fn check_enum_used_in_word_effect_ok() {
        check_src("type: Shape | Circle r f64 ; : id ( Shape -- Shape ) ;").unwrap();
    }
    #[test]
    fn check_enum_single_variant_newtype_ok() {
        // M3: a single-variant enum is allowed.
        check_src("type: Id | Wrap v i64 ; : main ( -- Id ) 5 Wrap ;").unwrap();
    }
    #[test]
    fn check_enum_duplicate_type_name_across_two_enums_is_error() {
        // X2: two enum `type:` declarations sharing a name.
        let err =
            check_src("type: Shape | Circle r f64 ; type: Shape | Square s f64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_duplicate_type_name_against_struct_is_error() {
        // X2: a struct and an enum sharing a name, across the combined
        // struct+enum registry (D10).
        let err = check_src("type: Vec2 x i64 y i64 ; type: Vec2 | Only v i64 ;").unwrap_err();
        assert!(err.contains("duplicate type"), "unexpected message: {err}");
        assert!(err.contains("Vec2"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_constructor_arity_mismatch_is_error() {
        // X9: too few values fed to a variant constructor, naming the enum.
        let src = "type: Shape | Rect w f64 h f64 ; : main ( -- Shape ) 1.0 Rect ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("Shape"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_constructor_field_type_mismatch_is_error() {
        // X9: a `bool` where an `f64` field is expected, naming both types.
        let src = "type: Shape | Circle r f64 ; : main ( -- Shape ) True Circle ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`f64`"), "unexpected message: {err}");
        assert!(err.contains("`Bool`"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_unifies_through_if_else_join_ok() {
        // R10: an enum type flows through an `if`/`else` join like any Type.
        check_src(
            "type: Shape | Circle r f64 | Square s f64 ;
             : pick ( Bool -- Shape ) ~[ 1.0 Circle ] ~[ 2.0 Square ] if ;",
        )
        .unwrap();
    }
    #[test]
    fn check_enum_moves_through_shuffles_ok() {
        // R10: dup/drop/swap/over move an enum value with no special case.
        check_src(
            "type: Shape | Circle r f64 | Square s f64 ;
             : main ( -- Shape ) 1.0 Circle 2.0 Square swap drop dup drop ;",
        )
        .unwrap();
    }
    #[test]
    fn check_enum_struct_and_enum_coexist_ok() {
        // D10: a distinct registry per kind; structs and enums both resolve
        // and both generate correctly-typed words in the same module.
        check_src(
            "type: Vec2 x i64 y i64 ;
             type: Shape | Circle r f64 ;
             : main ( -- Vec2 Shape ) 1 2 Vec2 3.0 Circle ;",
        )
        .unwrap();
    }
    #[test]
    fn check_enum_equality_operator_is_error() {
        // X10/M2, revised under P7.S3s R5: `eq` is the library's non-inline
        // `'T: Copy Ord` word, so an enum with no `impl: Ord` is rejected at
        // the bound, not at the operand-pair guard.
        let err =
            check_src("type: Shape | Circle r f64 ; : w ( Shape Shape -- Bool ) eq ;").unwrap_err();
        assert!(
            err.contains("does not satisfy `Ord`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }
    #[test]
    fn check_enum_arithmetic_operator_is_error() {
        // X10/M2: arithmetic on an enum reaches the operand-pair guard.
        let err = check_src("type: Shape | Circle r f64 ; : w ( Shape Shape -- Shape ) add ;")
            .unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("Shape"), "unexpected message: {err}");
    }
    #[test]
    fn check_struct_constructor_takes_a_matching_i64_field_ok() {
        check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;")).unwrap();
    }
    #[test]
    fn check_struct_constructor_on_a_float_field_is_error() {
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7.5 Spy drop ;")).unwrap_err();
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    /// P7.S3e: `check_trait_decls`/`check_impl_decls` run from `driver.rs`,
    /// not from `check::check` -- this drives them directly against a
    /// parsed (never-driver-assembled) `Module`, the way `driver.rs`'s own
    /// pre-mangle checks do.
    fn trait_check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        check_trait_decls(&module)
    }

    fn impl_check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        check_trait_decls(&module)?;
        check_impl_decls(&mut module)
    }

    #[test]
    fn check_trait_decls_duplicate_same_module_is_error() {
        let err = trait_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\ntrait: Show['T] : show ( &'T -- ) ; ;",
        )
        .unwrap_err();
        assert!(err.contains("duplicate trait `Show`"), "{err}");
    }

    #[test]
    fn check_trait_decls_collides_with_a_type_of_the_same_name() {
        // R1: the cross-kind collision `colliding_name_kind` was generalized
        // to catch (`trait: Point` alongside `type: Point`), via the new
        // `check_trait_decls` call site, not a bare `traits` arm.
        let err = trait_check_src("type: Point x i64 ;\ntrait: Point['T] : foo ( &'T -- ) ; ;")
            .unwrap_err();
        assert!(
            err.contains("already the name of a type"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn check_trait_decls_ok_for_a_clean_declaration() {
        trait_check_src("trait: Show['T] : show ( &'T -- ) ; ;").unwrap();
    }

    /// P7.S3p (ruling 5) + P7.S3t: the gate is "takes the trait variable
    /// *directly* as some input, or takes no input at all", so a
    /// non-trailing receiver (an index/lookup shape), a trailing one, and a
    /// nullary member all pass. A signature that mentions the variable only
    /// nested inside a composite input is the sole rejected shape (tested by
    /// `check_trait_decls_rejects_a_receiver_nested_in_an_array_input`).
    #[test]
    fn member_binds_trait_var_accepts_any_receiver_position() {
        let tokens = lex(
            "trait: T['T] : at ( &'T i64 -- i64 ) ; : sink ( i64 'T -- ) ; : fresh ( -- i64 ) ; ;",
        )
        .unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let members = &module
            .traits
            .iter()
            .find(|t| t.name == "T")
            .expect("the trait parsed")
            .members;
        let binds: Vec<(&str, bool)> = members
            .iter()
            .map(|m| (m.name.as_str(), member_binds_trait_var(m)))
            .collect();
        assert_eq!(binds, vec![("at", true), ("sink", true), ("fresh", true)]);
    }

    #[test]
    fn check_trait_decls_accepts_a_member_binding_no_receiver() {
        trait_check_src("trait: Show['T] : fresh ( -- i64 ) ; ;").unwrap();
    }

    /// P7b.S2 (S2-2): the replacement dispatchability rule -- an application
    /// headed by the trait var is a dispatchable input (the HKT form), and
    /// so is a reference to one (`& 'F['T]`: ref-ness is an addressing
    /// mode, not a type identity). This is W1's trait, accepted end to end.
    #[test]
    fn check_trait_decls_accepts_trait_var_headed_application_inputs() {
        trait_check_src(
            "trait: Functor['F: * -> *] : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             : refmap ( & 'F['T] -- ) ;\n\
             ;",
        )
        .unwrap();
    }

    /// P7b.S2 (S2-2/S2-15.a): with the member single-var gate lifted (S2-1),
    /// a member whose only inputs are member locals (or a member-local-headed
    /// application) has nothing to dispatch on -- a located declaration-time
    /// error naming the member, not the old single-var rejection.
    #[test]
    fn check_trait_decls_rejects_member_with_no_dispatchable_input() {
        let err = trait_check_src(
            "trait: Functor['F: * -> *] : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             : pick ( 'T -- 'F['T] ) ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`pick` of `Functor`")
                && err.contains("has no input for a call to dispatch on"),
            "{err}"
        );
        assert!(
            !err.contains("more than one type variable"),
            "the lifted member gate must not fire: {err}"
        );
        // S2-15.a review fix: the nested-composite note is conditional -- it
        // rides along only when the inputs actually nest the trait var. Here
        // the var appears only in the *outputs* (`'F['T]`), so no note.
        assert!(!err.contains("note:"), "{err}");
        // Located at the member (`pick` on line 2), not the trait header.
        assert!(err.contains("line 2, col 3"), "member position: {err}");
    }

    /// The gate is syntactic, so a receiver mentioned only *nested* inside a
    /// composite input is rejected too -- grounding it would need structural
    /// unification through the array type. The diagnostic must say so instead
    /// of claiming the signature mentions `'T` nowhere, which is false here,
    /// and must not conflate this shape with the (now-legal) nullary case.
    /// P7b.S2 (S2-15.a): the HKT-aware text, located at the member (not the
    /// trait), with the trait-var-headed expected shape in the parenthetical.
    #[test]
    fn check_trait_decls_rejects_a_receiver_nested_in_an_array_input() {
        let err =
            trait_check_src("trait: Show['T] : sum ( array[ 'T 4 ] -- i64 ) ; ;").unwrap_err();
        assert!(
            err.contains("`sum` of `Show`")
                && err.contains("has no input for a call to dispatch on"),
            "{err}"
        );
        assert!(
            err.contains("heading an application like `'T['U]`"),
            "the expected shape is named in the parenthetical: {err}"
        );
        assert!(
            err.contains("nested inside a composite input"),
            "the nested shape must not read as the nullary case: {err}"
        );
        // Located at the member, not the trait header (S2-15.a's position).
        assert!(err.contains("line 1, col 19"), "member position: {err}");
    }

    #[test]
    fn nested_receiver_member_error_interpolates_a_non_t_header_var() {
        let err =
            trait_check_src("trait: Show['G] : sum ( array[ 'G 4 ] -- i64 ) ; ;").unwrap_err();
        assert!(
            err.contains("heading an application like `'G['T]`")
                && err.contains("expected the trait's variable `'G` bare"),
            "{err}"
        );
    }

    #[test]
    fn nested_receiver_member_error_uses_u_placeholder_when_header_var_is_t() {
        let err =
            trait_check_src("trait: Show['T] : sum ( array[ 'T 4 ] -- i64 ) ; ;").unwrap_err();
        assert!(
            err.contains("heading an application like `'T['U]`")
                && err.contains("expected the trait's variable `'T` bare"),
            "{err}"
        );
    }

    #[test]
    fn check_impl_decls_ok_for_a_matching_member_body() {
        impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for i64\n\
               : show | p | p drop ;\n\
             ;",
        )
        .unwrap();
    }

    #[test]
    fn check_impl_decls_orphan_scalar_target_names_only_the_trait_module() {
        // A scalar declares no module of its own, so "or the module declaring
        // `i64`" would name a home that cannot exist.
        let tokens = lex("trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for i64\n\
               : show | p | p drop ;\n\
             ;")
        .unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        check_trait_decls(&module).unwrap();
        module
            .traits
            .iter_mut()
            .find(|t| t.name == "Show")
            .unwrap()
            .module = 1;
        let err = check_impl_decls(&mut module).unwrap_err();
        assert!(err.contains("declares no module of its own"), "{err}");
        assert!(!err.contains("or the module declaring"), "{err}");
    }

    #[test]
    fn check_static_decls_rejects_a_static_colliding_with_a_trait() {
        // R1 wants statics in the trait scan set. `colliding_name_kind` has no
        // `statics` arm: `check_static_decls` runs first (driver.rs), so the
        // clash is caught from the static's own call site via the `traits`
        // arm, whichever order the two are declared in.
        let tokens = lex("trait: Show['T] : show ( &'T -- ) ; ;\nstatic: Show i64 = 0 ;").unwrap();
        let module = crate::parser::parse(&tokens).unwrap();
        let err = check_static_decls(&module).unwrap_err();
        assert!(err.contains("already the name of a trait"), "{err}");
    }

    #[test]
    fn check_impl_decls_missing_member_is_error() {
        let err = impl_check_src(
            "trait: Eq['T] : eq ( &'T &'T -- ) ; : hash ( &'T -- ) ; ;\n\
             impl: Eq for i64\n\
               : eq | a b | a drop b drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("does not bind required member `hash`"),
            "{err}"
        );
    }

    #[test]
    fn check_impl_decls_duplicate_member_is_error() {
        let err = impl_check_src(
            "trait: Getter['T] : get ( &'T -- i64 ) ; ;\n\
             type: Point n i64 ;\n\
             impl: Getter for Point\n\
               : get | p | p &n @ ;\n\
               : get | p | p &n @ ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("binds member `get` more than once"), "{err}");
    }

    #[test]
    fn check_impl_decls_duplicate_impl_is_error() {
        let err = impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for i64\n\
               : show | p | p drop ;\n\
             ;\n\
             impl: Show for i64\n\
               : show | p | p drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("duplicate `impl:` for `i64`"), "{err}");
    }

    // P7.S4 (R7): duplicate check compares `target.pattern` (the `PolyType`),
    // so alpha-equivalent generic targets are duplicates.

    #[test]
    fn check_impl_decls_alpha_equivalent_generic_targets_are_duplicate() {
        let err = impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;\n\
             impl: Show for array['U 'M]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("duplicate `impl:`"), "{err}");
    }

    // P7.S4b (R4): a bounded and unbounded impl at the same pattern are
    // distinct declarations (not a duplicate), but two impls with the same
    // pattern and the same bound set are still a duplicate.

    #[test]
    fn check_impl_decls_bounded_and_unbounded_at_same_pattern_are_distinct() {
        impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;\n\
             impl: Show for array['T 'N] where 'T: Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .expect("bounded and unbounded impls at the same pattern are distinct");
    }

    #[test]
    fn check_impl_decls_same_pattern_same_bounds_is_duplicate() {
        let err = impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'T: Show\n\
               : show | a | a drop ;\n\
             ;\n\
             impl: Show for array['T 'N] where 'T: Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("duplicate `impl:`"), "{err}");
    }

    #[test]
    fn check_impl_decls_alpha_equivalent_bounded_targets_are_duplicate() {
        // Alpha-equivalent targets with the same bound set are duplicates,
        // even though the variable names differ ('T vs 'U).
        let err = impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'T: Show\n\
               : show | a | a drop ;\n\
             ;\n\
             impl: Show for array['U 'M] where 'U: Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("duplicate `impl:`"), "{err}");
    }

    #[test]
    fn check_impl_decls_overlapping_unequal_targets_accepted() {
        // `['T N]` and `array['T 4]` overlap at `array[i64 4]` but are not equal, so
        // they are accepted as declarations (the overlap is resolved by
        // specificity at the dispatch site, not rejected here).
        impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;\n\
             impl: Show for array['T 4]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .expect("overlapping but unequal targets should be accepted");
    }

    // P7.S4 (R4): a generic `impl:` outside the trait's module is rejected
    // (orphan rule: a generic target names no struct/enum, so the impl must
    // live in the trait's own module). The existing orphan arm already
    // enforces this via the `None` path.

    #[test]
    fn check_impl_decls_generic_impl_outside_trait_module_is_orphan_error() {
        // `impl: Show for array['T 'N]` in module 0 (the default), with `Show` in
        // module 1. A generic target names no struct/enum, so
        // `impl_target_module` returns `None` and the orphan rule requires the
        // impl to live in the trait's module (module 1).
        let src = "trait: Show['T] : show ( &'T -- ) ; ;\n\
             type: Foo x i64 ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;\n";
        let tokens = crate::lexer::lex(src).unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        // Simulate the trait being in a different module: set the trait's
        // module to 1 while the impl stays in module 0. The `traits` vector
        // starts with the seeded `Copy` entry, so `Show` is at index 1.
        module
            .traits
            .iter_mut()
            .find(|t| t.name == "Show")
            .expect("the Show trait parsed")
            .module = 1;
        let err = check_impl_decls(&mut module).unwrap_err();
        assert!(
            err.contains("must live in the module declaring"),
            "unexpected message: {err}"
        );
        assert!(err.contains("declares no module of its own"), "{err}");
        // R9: the diagnostic must name the target shape family (e.g. `array['T 'N]`),
        // not a scalar, so the user can see *what* was orphaned.
        assert!(
            err.contains("array['T 'N]"),
            "should name the target shape: {err}"
        );
    }

    #[test]
    fn check_impl_decls_generic_impl_in_trait_module_accepted() {
        // The same generic `impl: Show for array['T 'N]` in the trait's own module
        // (module 0 for both) is accepted — the orphan rule's legal home.
        impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;
\
             impl: Show for array['T 'N]
\
               : show | a | a drop ;
\
             ;",
        )
        .expect("generic impl in the trait's own module should be accepted");
    }

    // R4 (concrete parity): a concrete `impl: Show for Point` keeps the
    // existing rule — accepted in `Point`'s module or `Show`'s module,
    // rejected elsewhere. No regression from the `ImplTarget` change.

    #[test]
    fn check_impl_decls_concrete_impl_in_trait_module_accepted() {
        // Trait and impl in module 0; struct also in module 0 (trait's module
        // == impl's module → accepted).
        impl_check_src(
            "trait: Show['T] : show ( &'T -- ) ; ;
\
             type: Point x i64 ;
\
             impl: Show for Point
\
               : show | p | p drop ;
\
             ;",
        )
        .expect("concrete impl in the trait's module should be accepted");
    }

    #[test]
    fn check_impl_decls_concrete_impl_in_target_module_accepted() {
        // Trait in module 1, struct and impl in module 0 (target's module
        // == impl's module → accepted).
        let src = "trait: Show['T] : show ( &'T -- ) ; ;
\
             type: Point x i64 ;
\
             impl: Show for Point
\
               : show | p | p drop ;
\
             ;";
        let tokens = crate::lexer::lex(src).unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        check_trait_decls(&module).unwrap();
        module
            .traits
            .iter_mut()
            .find(|t| t.name == "Show")
            .expect("the Show trait parsed")
            .module = 1;
        check_impl_decls(&mut module)
            .expect("concrete impl in the target's module should be accepted");
    }

    #[test]
    fn check_impl_decls_concrete_impl_outside_both_modules_is_orphan_error() {
        // Trait in module 1, struct in module 2, impl in module 0 (neither
        // the trait's nor the target's module → rejected).
        let src = "trait: Show['T] : show ( &'T -- ) ; ;
\
             type: Point x i64 ;
\
             impl: Show for Point
\
               : show | p | p drop ;
\
             ;";
        let tokens = crate::lexer::lex(src).unwrap();
        let mut module = crate::parser::parse(&tokens).unwrap();
        check_trait_decls(&module).unwrap();
        module
            .traits
            .iter_mut()
            .find(|t| t.name == "Show")
            .expect("the Show trait parsed")
            .module = 1;
        // The struct is the first user struct; find it and set its module.
        let struct_idx = module
            .structs
            .iter()
            .position(|s| s.name == "Point")
            .expect("the Point struct parsed");
        module.structs[struct_idx].module = 2;
        let err = check_impl_decls(&mut module).unwrap_err();
        assert!(
            err.contains(
                "must live in the module declaring `Show` or the module declaring `Point`"
            ),
            "should name both legal homes: {err}"
        );
    }

    fn mk_enum(name: &str, variants: &[(&str, Vec<Type>)]) -> EnumDecl {
        EnumDecl {
            name: name.to_string(),
            name_static: Box::leak(name.to_string().into_boxed_str()),
            variants: variants
                .iter()
                .map(|(vname, field_tys)| VariantDecl {
                    name: vname.to_string(),
                    name_static: Box::leak(vname.to_string().into_boxed_str()),
                    display_static: Box::leak(format!("{name}.{vname}").into_boxed_str()),
                    fields: field_tys
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| (format!("f{i}"), *ty))
                        .collect(),
                    span: Span::default(),
                })
                .collect(),
            span: Span::default(),
            module: 0,
        }
    }

    /// P7.S11 (R4): `enum_generated_sigs`/`variant_generated_sigs` derive a
    /// variant's constructor/destructure output type from the *slice
    /// position* the decl is found at, not from any id carried by the decl
    /// itself (`EnumId::from_index(idx)`, `src/check/declarations.rs:1755`).
    /// R4 relies on this: it must generate over the *extended* `local.enums`
    /// slice, never over a fresh one-element slice, or a grounded monomorph's
    /// constructor would register at id 0 no matter where it actually landed.
    #[test]
    fn enum_generated_sigs_over_an_extended_slice_carries_the_monomorphs_own_id() {
        let base = vec![mk_enum("Shape", &[("Circle", vec![Type::I64])])];
        let grounded = mk_enum(
            "Result[i64 i64]",
            &[("Ok", vec![Type::I64]), ("Err", vec![Type::I64])],
        );
        let mut extended = base.clone();
        extended.push(grounded.clone());

        let (_, _, _, sig) = enum_generated_sigs(&extended)
            .into_iter()
            .find(|(name, _, _, _)| name == "Ok")
            .expect("the extended slice generates Ok's constructor");
        assert_eq!(
            sig.outputs[0],
            Type::Enum(EnumId::from_index(base.len()), "Result[i64 i64]")
        );

        // The one-element-slice failure mode this test forbids: the same
        // decl alone yields id 0, which is why R4 must generate over the
        // whole extended slice, not a fresh slice of just the mint.
        let (_, _, _, alone_sig) = enum_generated_sigs(std::slice::from_ref(&grounded))
            .into_iter()
            .find(|(name, _, _, _)| name == "Ok")
            .expect("a one-element slice still generates Ok's constructor");
        assert_eq!(
            alone_sig.outputs[0],
            Type::Enum(EnumId::from_index(0), "Result[i64 i64]")
        );
    }

    /// P7.S11 (R4): the skip count R4 subtracts off `enum_generated_sigs`/
    /// `variant_generated_sigs` over the extended slice is *not* a per-decl
    /// arithmetic count (a one-variant and a three-variant enum contribute
    /// different numbers of entries) -- it is the length of the same helper
    /// run over the base slice, which is a prefix of the extended run because
    /// both are a single in-order `enumerate().push()` loop over a shared
    /// prefix of decls.
    #[test]
    fn enum_generated_sigs_prefix_is_stable_under_extension() {
        let base = vec![
            mk_enum("One", &[("A", vec![])]),
            mk_enum("Three", &[("X", vec![]), ("Y", vec![]), ("Z", vec![])]),
        ];
        let mut extended = base.clone();
        extended.push(mk_enum("Extra", &[("E", vec![Type::I64])]));

        let base_sigs = enum_generated_sigs(&base);
        let extended_sigs = enum_generated_sigs(&extended);
        assert_eq!(&extended_sigs[..base_sigs.len()], &base_sigs[..]);

        let base_variant_sigs = variant_generated_sigs(&base);
        let extended_variant_sigs = variant_generated_sigs(&extended);
        assert_eq!(
            &extended_variant_sigs[..base_variant_sigs.len()],
            &base_variant_sigs[..]
        );
    }
}

/// P7b.S2 (S2-11): a ctor-abstract target names its constructor's module
/// for the orphan rule -- the same two homes a concrete target gets. A
/// scalar stays `None` (trait-module-only), now stated for builtin-shaped
/// ctor images too.
#[cfg(test)]
mod s2_orphan_tests {
    use super::*;
    use crate::ast::ImplTarget;

    fn module_with_ctor(ctor_module: u32) -> Module {
        Module {
            structs: vec![StructDecl {
                name: "Box".to_string(),
                name_static: "Box",
                fields: vec![("v".to_string(), crate::ast::Type::I64)],
                span: Span::default(),
                has_drop_overload: false,
                is_bundle: false,
                module: ctor_module,
            }],
            enums: Vec::new(),
            ..Module::default()
        }
    }

    #[test]
    fn impl_target_module_generic_ctor_target_names_the_ctor_module() {
        let target = ImplTarget {
            pattern: crate::ast::PolyType::Generic {
                is_enum: false,
                idx: 0,
                module: 3,
                args: vec![crate::ast::PolyType::Var(0)],
                len_args: vec![],
                name: "Box",
            },
            ty_var_names: vec!["'T".to_string()],
            ty_var_spans: vec![Span::default()],
            ty_kinds: vec![crate::ast::Kind::Star],
            len_var_names: vec![],
            len_var_spans: vec![],
            user_spelling: None,
            bounds: vec![],
        };
        let module = module_with_ctor(7);
        // The Generic pattern's module is the ctor's declaring module, so
        // the arm is exact: the impl may live in the ctor's module (or the
        // trait's, by the rule's other arm).
        assert_eq!(impl_target_module(&target, &module), Some(3));
    }

    #[test]
    fn impl_target_module_scalar_target_stays_trait_module_only() {
        let target = ImplTarget {
            pattern: crate::ast::PolyType::Concrete(crate::ast::Type::I64),
            ty_var_names: vec![],
            ty_var_spans: vec![],
            ty_kinds: vec![],
            len_var_names: vec![],
            len_var_spans: vec![],
            user_spelling: None,
            bounds: vec![],
        };
        let module = module_with_ctor(7);
        assert_eq!(impl_target_module(&target, &module), None);
    }
}
