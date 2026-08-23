//! Parser: tokens -> AST.
//!
//! Grammar (Phase 0, plus the Slice 3/4 `type:` production and the Slice 8a
//! `extern:` production):
//!   module   := (worddef | typedef | externdef)*
//!   worddef  := ':' Word '(' effect ')' term* ';'
//!   typedef  := struct-typedef | enum-typedef
//!   struct-typedef := 'type:' Word (Word Word)* ';'
//!   enum-typedef    := 'type:' Word '|'? variant ('|' variant)* ';'
//!   variant         := Word (Word Word)*
//!   externdef       := 'extern:' Word '(' effect ')' Str ';'
//!   effect   := slot* '--' slot*
//!   slot     := Word (':' Word)?
//!   binding  := '|' Word+ '|'
//!   term     := Int | Word | binding | if
//!   if       := 'if' term* ('else' term*)? 'end'

use crate::ast::{
    ground_member_type, intern_array_type, is_name_dispatched_builtin, ArrayDecl, Bound, EnumDecl,
    ExternDecl, GenericTypes, GlobalEntry, GlobalMode, ImplDecl, Import, ImportAnchor,
    ImportBinding, ImportTarget, IntrinsicVisibility, Len, Line, Module, ModuleInfo, ModuleName,
    NameRegistries, OwnedCellDecl, PolySig, PolyType, QuotAnnot, RefDecl, SliceDecl, Span,
    StackEffect, StaticDecl, StaticInit, StructDecl, Term, TermKind, TraitDecl, TraitId, TraitKind,
    TraitMember, Type, TypedSlot, VariantDecl, VariantTag, VariantTagMode, WordDef,
};
use crate::lexer::Token;
use std::collections::HashMap;

/// Whether a `type:` body (starting at `body_start`, the token just after the
/// declared name) is an enum: it contains a `Pipe` before its terminating
/// `Semicolon`, D1's `|`-separated-variants marker. Shared by the pre-pass
/// (which never errors, malformed bodies are left for the real production)
/// and the parser's own lookahead, so both classify a body identically.
fn body_has_pipe_before_semicolon(tokens: &[(Token, Span)], mut i: usize) -> bool {
    while let Some((tok, _)) = tokens.get(i) {
        match tok {
            Token::Semicolon => return false,
            Token::Pipe => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// One `type:` decl as classified by the pre-pass: a Slice 3 struct, or an
/// enum with its variant `(name, span)` list in source order (D8's variant
/// pre-pass — variant names are known before any word body is parsed,
/// regardless of `type:` declaration order).
enum TypeDeclKind {
    Struct,
    Enum(Vec<(String, Span)>),
}

/// Pre-pass: scan the whole token stream for every `type: Name`, classify
/// its body per `body_has_pipe_before_semicolon`, and for an enum also
/// collect each variant name: the word immediately following each `|`, or
/// (D1's optional leading `|`) the very first body token when the body has
/// no leading `|`. Malformed occurrences are left for the real `type:`
/// production to report; this pass only registers names, so it never errors
/// itself.
fn prepass_type_decls(
    tokens: &[(Token, Span)],
) -> Result<Vec<(String, Span, TypeDeclKind)>, String> {
    let mut decls = Vec::new();
    for i in 0..tokens.len() {
        if let (Token::Word(w), _) = &tokens[i] {
            if w == "type:" {
                if let Some((Token::Word(name), span)) = tokens.get(i + 1) {
                    reject_reserved_name("type", name, *span)?;
                    // Phase 5 slice 1 (R1/D5): a generic header mints no
                    // concrete struct/enum registry entry here -- its full
                    // variable-scoped shape is parsed into
                    // `Module::generic_structs`/`generic_enums` by
                    // `prepass_generic_typedefs`, run over every file in the
                    // closure once this pass has registered every concrete
                    // name a generic field might forward-reference.
                    if header_ty_var_count(tokens, i + 2) > 0 {
                        continue;
                    }
                    let kind = if body_has_pipe_before_semicolon(tokens, i + 2) {
                        let variants = scan_variant_names(tokens, i + 2);
                        for (vname, vspan) in &variants {
                            reject_reserved_name("variant", vname, *vspan)?;
                        }
                        TypeDeclKind::Enum(variants)
                    } else {
                        TypeDeclKind::Struct
                    };
                    decls.push((name.clone(), *span, kind));
                }
            }
        }
    }
    Ok(decls)
}

/// The count of `'`-prefixed tokens starting at `start` (Phase 5 slice 1,
/// R1/D2): a `type:` header's bound type variables, immediately following its
/// declared name, zero for a concrete (non-generic) declaration. Shared by
/// the pre-pass (which skips registering a generic header into the concrete
/// registries) and the parser's own lookahead before dispatching to the
/// generic or concrete production.
fn header_ty_var_count(tokens: &[(Token, Span)], start: usize) -> usize {
    let mut n = 0;
    while matches!(tokens.get(start + n), Some((Token::Word(w), _)) if w.starts_with('\'')) {
        n += 1;
    }
    n
}

/// A located error for a name reserved by the owning-cell syntax (`^`, `^>`,
/// `^|>`, or any name beginning with `^`), used at every declaration site it
/// can arise: a `type:` name, a `:` word name, a local binding, or the
/// REPL's own `type:`-line path.
fn reserved_caret_name_error(kind: &str, name: &str, span: Span) -> String {
    format!(
        "error: `{name}` is reserved for the owning-cell syntax (`^`, `^>`, `^|>`) and cannot be used as a {kind} name at line {}, col {}",
        span.line, span.col
    )
}

/// Whether `name` collides with the owning-cell syntax (`^`, `^>`,
/// `^|>`) or would shadow/be shadowed by it: any name beginning with `^` is
/// reserved. Sooth has no notion of an identifier — a `type:`/`:` name or a
/// local binding is otherwise just a bare word — so this is a plain prefix
/// check, not a fixed set of three spellings.
fn is_reserved_caret_name(name: &str) -> bool {
    name.starts_with('^')
}

/// A located error for a name reserved by the reference syntax, the same
/// shape `reserved_caret_name_error` applies to `^`-led names.
fn reserved_ref_name_error(kind: &str, name: &str, span: Span) -> String {
    format!(
        "error: `{name}` is reserved for the reference syntax (`&`, `&!`, `&>`, `&^`) and cannot be used as a {kind} name at line {}, col {}",
        span.line, span.col
    )
}

/// Whether `name` collides with the reference syntax: any name beginning with
/// `&` is reserved, exactly as any `^`-led name is reserved for owning cells.
fn is_reserved_ref_name(name: &str) -> bool {
    name.starts_with('&')
}

/// The three exact-name access builtins this slice introduces. A `:` word
/// declaration naming one of them would silently change its meaning for every
/// later caller, so it is rejected rather than shadowed.
const ACCESS_WORDS: [&str; 3] = ["@", "!", "+!"];

/// A located error for a `:` word declaration that would shadow one of the
/// access builtins.
fn shadowed_access_word_error(name: &str, span: Span) -> String {
    format!(
        "error: `{name}` is a builtin access word (`@`, `!`, `+!`) and cannot be redefined at line {}, col {}",
        span.line, span.col
    )
}

/// D1/OQ1: a `static:` declaration's type token is outside the fixed scalar
/// keyword set (`i64`/`u32`/`bool`/`str`). Allow-list-based, not
/// struct-detection-based (see `Parser::parse_static_decl`): a genuine struct
/// type and a mistyped or forward-referenced user type are indistinguishable
/// here and get the same message.
fn static_scalar_type_error(name: &str, ty_name: &str, span: Span) -> String {
    format!(
        "error: static `{name}` has a non-scalar type `{ty_name}` (only `i64`, `u32`, `Bool`, and `str` are supported this slice; struct-typed statics are deferred) at line {}, col {}",
        span.line, span.col
    )
}

/// D1/D3: a `static: X u32 = N;` initialiser whose literal is outside
/// `u32`'s representable range -- mirrors the array-count range check
/// (`parse_poly_array`) rather than deferring to Phase 4's lowering, which
/// would otherwise silently truncate.
fn static_u32_init_range_error(n: i64, span: Span) -> String {
    format!(
        "error: static initializer {n} is out of range for `u32` at line {}, col {} (requires 0 <= N <= {})",
        span.line,
        span.col,
        u32::MAX
    )
}

/// D2: two `global:` entries written without the separating `,`. Without this
/// the clause just ends at the first entry and the rest becomes body terms,
/// reported far away as an unknown word -- exactly the silent truncation this
/// language exists to turn into a sharp error.
fn missing_global_comma_error(name: &str, span: Span) -> String {
    format!(
        "parse error: missing `,` between global-set entries, before `{name}` at line {}, col {}",
        span.line, span.col
    )
}

/// D2: a `global:` entry's mode token is neither `r` nor `w` (nor `r,`/`w,`).
fn invalid_global_mode_error(found: &str, span: Span) -> String {
    format!(
        "parse error: expected a global-set mode (`r` or `w`), found `{found}` at line {}, col {}",
        span.line, span.col
    )
}

/// The one reserved-name gate every declaration site calls: a `^`-led name
/// (owning cells) or a `&`-led name (references). P7 slice 3c: a `type:` or
/// `variant` name may not be `Slice` or `!Slice` either --
/// `resolve_type_or_apply` intercepts both spellings ahead of every user
/// registry, so a declaration under either name would be silently unreachable
/// rather than merely shadowed.
pub fn reject_reserved_name(kind: &str, name: &str, span: Span) -> Result<(), String> {
    if is_reserved_caret_name(name) {
        return Err(reserved_caret_name_error(kind, name, span));
    }
    if is_reserved_ref_name(name) {
        return Err(reserved_ref_name_error(kind, name, span));
    }
    if matches!(kind, "type" | "variant") && matches!(name, SLICE_TYPE_NAME | MUT_SLICE_TYPE_NAME) {
        return Err(format!(
            "error: `{name}` is reserved for the slice type syntax (`{SLICE_TYPE_NAME}[T]`) and cannot be used as a {kind} name at line {}, col {}",
            span.line, span.col
        ));
    }
    Ok(())
}

/// P7 slice 3c (R1.1): the one surface spelling of a slice type. Not a
/// registered `type:` name and not a generic header: `Slice[T]` resolves
/// through the interned slice registry, so it is intercepted by name ahead of
/// every user lookup.
pub const SLICE_TYPE_NAME: &str = "Slice";

/// P7 slice 3c (R1.1, phase 4): the mutable view's spelling, `!Slice[T]`. The
/// `!` marks mutability exactly as it does in `&!T`, and it is glued to the
/// name because a type expression has no other place to put it.
pub const MUT_SLICE_TYPE_NAME: &str = "!Slice";

/// Phase 5 slice 1: a `'`-prefixed word inside a `type:` body is a type
/// variable, never a field name. Rejected at every named-field-name position
/// so the rule holds uniformly -- a generic header consumes its `'`-prefixed
/// words before any field is read, which would otherwise leave `'x` legal as
/// a field name everywhere except directly after the type name. One caller
/// (`parse_generic_variant_fields`, Phase 5 slice 2) never actually needs
/// this check: its `'`-prefixed arm already diverts every such token to the
/// attributeless-field path before the named-field arm is reached.
fn reject_ty_var_field_name(name: &str, span: Span) -> Result<(), String> {
    if name.starts_with('\'') {
        return Err(format!(
            "error: `{name}` at line {}, col {} cannot be a field name (a `'`-prefixed word is a type variable)",
            span.line, span.col
        ));
    }
    Ok(())
}

/// OQ4: the internal name stored for an attributeless (positional) generic
/// variant field. It contains a space, which the lexer never produces inside
/// a single `Word` token (words are whitespace-delimited), so this string can
/// never be typed, matched as a field name, or collide with a real one.
/// `pub(crate)` so `check::variant_field_desc` can recognize it and report the
/// field's position instead of this literal string.
pub(crate) const POSITIONAL_FIELD_NAME: &str = "$positional field$";

/// R12: the `extern:` symbol string is emitted verbatim as `call $<symbol>`
/// once lowered, so it must already be a valid C identifier here at the
/// declaration — the trust boundary — rather than surfacing as broken QBE
/// output or an empty symbol name later.
fn is_valid_c_symbol(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// A located error for an `extern:` C-symbol string that is not a valid C
/// identifier.
fn invalid_c_symbol_error(symbol: &str, span: Span) -> String {
    format!(
        "error: `{symbol}` is not a valid C symbol name at line {}, col {}\n  a C symbol must be non-empty and match `[A-Za-z_][A-Za-z0-9_]*`",
        span.line, span.col
    )
}

/// P7.S3e (R4/R8, decision 8): a trait member signature is restricted to
/// concrete/array/reference shapes over `'T` this slice -- `ast`'s
/// `ground_member_type`, which the body-form desugar grounds each member with
/// against a concrete `impl:` target, handles exactly these and nothing else
/// (a quotation or generic-application shape has no forcing consumer this
/// phase and no grounding rule).
fn member_shape_is_supported(t: &PolyType) -> bool {
    match t {
        PolyType::Concrete(_) | PolyType::Var(_) => true,
        PolyType::Array(elem, Len::Concrete(_)) => member_shape_is_supported(elem),
        PolyType::Array(_, Len::Var(_)) => false,
        PolyType::Ref(referent, _) => member_shape_is_supported(referent),
        // P7.S3n (R3): the new owned-cell shape is deliberately *not* added
        // to the supported set -- `ground_member_type` has no cell arm, so a
        // `^'T` member would ground to nothing. A located rejection, not a
        // wildcard fall-through.
        PolyType::OwnedCell(_)
        | PolyType::Quotation(..)
        | PolyType::Generic { .. }
        | PolyType::QuotLit => false,
    }
}

/// P7.S3e (R4/R8): a trait member signature mentions an unsupported shape
/// (a quotation or generic-application type) -- see `member_shape_is_supported`.
fn unsupported_trait_member_shape_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}`'s member at line {}, col {} has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable are supported this slice)",
        span.line, span.col
    )
}

/// P7.S3e (R4/R8, R9's combinator descope): a trait member declaring a
/// top-level row variable. A row is a `PolySig` field, not a slot shape, so
/// `member_shape_is_supported` cannot see it and the body-form desugar grounds
/// `inputs`/`outputs` alone -- without this rejection the row would be dropped
/// from the synthesized word's effect and a stack-polymorphic member would
/// silently check as an ordinary one.
fn row_typed_trait_member_error(trait_name: &str, row: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}`'s member at line {}, col {} declares the row variable `{row}` (a stack-polymorphic member is not supported this slice)",
        span.line, span.col
    )
}

/// P7.S3e (R2): an `impl:` naming a reserved predicate trait (`Copy`/`Ord`).
/// Their satisfaction is structural (`is_copy`/`is_ord`), and the reserved
/// table entry has no real declaring module, so it can be neither implemented
/// nor meaningfully orphan-checked.
fn impl_for_predicate_trait_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}` cannot be implemented at line {}, col {} (it is a built-in predicate, satisfied by a type's own shape)",
        span.line, span.col
    )
}

/// P7.S3e (R16): a `trait:` header naming a second type variable, or a
/// member signature mentioning a variable other than the header's --
/// single-type-variable traits only this slice.
fn multi_variable_trait_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}` names more than one type variable at line {}, col {} (only single-type-variable traits are supported)",
        span.line, span.col
    )
}

/// P7.S3e (R1): a `trait:` declaration with zero required members.
fn trait_zero_members_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}` declares no members at line {}, col {} (a trait must require at least one member)",
        span.line, span.col
    )
}

/// P7.S3e (R4): an `impl:` naming a trait that resolves to nothing in scope
/// (unknown, or not imported).
fn unknown_trait_error(name: &str, span: Span) -> String {
    format!(
        "error: unknown trait `{name}` at line {}, col {}",
        span.line, span.col
    )
}

/// P7.S3e (R4): an `impl:` block with zero member bindings.
fn impl_zero_bindings_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: `impl: {trait_name}` binds no members at line {}, col {} (an impl must bind at least one)",
        span.line, span.col
    )
}

/// P7.S3r (R4): a `trait:` member spelled as a name `check_term` dispatches
/// ahead of the word environment. A member becomes a word when implemented,
/// and inside its own body the member name binds to that word (R4a's rewrite),
/// so such a member would shadow a builtin there -- wider than the
/// construct-scoped shadowing the body form admits. Rejected at the
/// declaration, where the unimplementable member is written, rather than at
/// each impl body that discovers it.
fn builtin_named_trait_member_error(trait_name: &str, member: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}` declares a member named `{member}`, which is a builtin word (line {}, col {})\n  note: a trait member becomes a word when implemented, and inside its own body the name would shadow the builtin",
        span.line, span.col
    )
}

/// P7.S3r (R2): an `impl:` body member restating its signature. The
/// synthesized word's effect is the trait member's, grounded at the `for`
/// type, so a restated one is a second spelling of the same thing (and one
/// that could disagree).
fn impl_member_restated_signature_error(member: &str, trait_name: &str, span: Span) -> String {
    format!(
        "error: impl member `{member}` must not restate its signature at line {}, col {} (it is inherited from trait `{trait_name}`'s `{member}` with the `for` type)",
        span.line, span.col
    )
}

/// P7.S3r (R6): an `impl:` body declaring a word that is not a member of the
/// implemented trait. There is no member to bind, and a free module-private
/// word here would silently swallow a misspelled member name.
fn impl_non_member_body_error(member: &str, trait_name: &str, span: Span) -> String {
    format!(
        "error: `{member}` is not a member of trait `{trait_name}` at line {}, col {}",
        span.line, span.col
    )
}

/// P7.S3r (R4a): a `| ... |` binder inside an impl member's own body sharing
/// that member's name. The self-call rewrite is unconditional token equality,
/// so the binder and the recursive call cannot coexist; a silent winner either
/// way is the shadowing this language refuses.
fn impl_member_binder_shadows_itself_error(member: &str, span: Span) -> String {
    format!(
        "error: `{member}` binds a local inside its own impl body at line {}, col {}, where the name already refers to the member itself",
        span.line, span.col
    )
}

/// P7.S3r: the internal name of the word an `impl:` body member desugars to,
/// `member;Trait;trait-module;Type`. Trait-qualified because two traits may
/// require a same-named member with the same grounded signature for one type,
/// and unforgeable because `;` is a hard lexer delimiter: no source token can
/// contain one. Never parsed back, so the components only need to be injective
/// per implemented member.
///
/// The trait component carries the declaring module's id, not just the bare
/// declared name: two same-named traits from different modules can both be
/// implemented for one type in one module, and the bare name alone would make
/// those two members' synthesized names collide.
///
/// The `Type` component is only the *rendered* type name, so two same-named
/// types from different modules do share a synthesized name. That case is
/// currently carried by the ordinary overload-suffix path, which needs the two
/// grounded signatures to differ -- guaranteed here, because a member's last
/// input must be `'T`/`&'T` (`check::declarations::non_trailing_receiver_error`)
/// and so every grounded signature mentions the `for` type.
fn synth_member_word_name(
    member: &str,
    trait_name: &str,
    trait_module: u32,
    target: Type,
) -> String {
    format!("{member};{trait_name};{trait_module};{}", target.name())
}

/// P7.S3r (R4a): rewrite every call of `member` inside its own desugared body
/// (nested quotations included) to the synthesized word's name, so a member
/// body can recurse. A `| ... |` binder of the same name is rejected rather
/// than silently shadowing or being silently rewritten away.
fn rewrite_member_self_calls(
    terms: &[Term],
    member: &str,
    synth: &str,
) -> Result<Vec<Term>, String> {
    terms
        .iter()
        .map(|term| {
            let kind = match &term.kind {
                TermKind::Bind(names) if names.iter().any(|n| n == member) => {
                    return Err(impl_member_binder_shadows_itself_error(member, term.span));
                }
                TermKind::Call(name) if name == member => TermKind::Call(synth.to_string()),
                TermKind::Quotation(inner, is_inline, annot) => TermKind::Quotation(
                    rewrite_member_self_calls(inner, member, synth)?,
                    *is_inline,
                    annot.clone(),
                ),
                other => other.clone(),
            };
            Ok(Term {
                kind,
                span: term.span,
            })
        })
        .collect()
}

/// P7.S3e (R4, decision 4): resolve a trait name to a `TraitId`, module-aware
/// exactly like `resolve_type_name_in_module` -- own module first, then a
/// pre-seeded reserved-module entry (`Copy`/`Ord`, visible everywhere), then
/// a `qualifier::Base` mapped through `imports`, then a bare name reached via
/// a selective import.
pub(crate) fn find_trait_in_module(
    traits: &[TraitDecl],
    name: &str,
    module: u32,
    imports: &HashMap<String, u32>,
    selective: &HashMap<String, u32>,
) -> Option<TraitId> {
    if let Some((qualifier, base)) = name.split_once("::") {
        let target = *imports.get(qualifier)?;
        return traits
            .iter()
            .position(|t| t.name == base && t.module == target)
            .map(TraitId::from_index);
    }
    if let Some(idx) = traits
        .iter()
        .position(|t| t.name == name && t.module == module)
    {
        return Some(TraitId::from_index(idx));
    }
    if let Some(idx) = traits
        .iter()
        .position(|t| t.name == name && t.module == crate::ast::RESERVED_TRAIT_MODULE)
    {
        return Some(TraitId::from_index(idx));
    }
    if let Some(&target) = selective.get(name) {
        return traits
            .iter()
            .position(|t| t.name == name && t.module == target)
            .map(TraitId::from_index);
    }
    None
}

/// P7.S3e (R2): whether `name` matches a pre-seeded `Predicate`-kind trait
/// table entry (`Copy`/`Ord`), and if so, the `Bound` it produces.
/// `parse_capabilities`'s one lookup point, replacing the two hardcoded
/// string compares.
fn predicate_bound(traits: &[TraitDecl], name: &str) -> Option<Bound> {
    traits.iter().find_map(|t| match t.kind {
        TraitKind::Predicate(b) if t.name == name => Some(b),
        _ => None,
    })
}

/// The one gate every `extern:` symbol string passes through.
fn reject_invalid_c_symbol(symbol: &str, span: Span) -> Result<(), String> {
    if is_valid_c_symbol(symbol) {
        Ok(())
    } else {
        Err(invalid_c_symbol_error(symbol, span))
    }
}

/// Collect variant `(name, span)` pairs from an enum `type:` body: the word
/// following each `|`, plus the very first body token when there is no
/// leading `|`.
fn scan_variant_names(tokens: &[(Token, Span)], start: usize) -> Vec<(String, Span)> {
    let mut variants = Vec::new();
    let mut expect_variant_name = true;
    let mut i = start;
    while let Some((tok, span)) = tokens.get(i) {
        match tok {
            Token::Semicolon => break,
            Token::Pipe => expect_variant_name = true,
            Token::Word(w) if expect_variant_name => {
                variants.push((w.clone(), *span));
                expect_variant_name = false;
            }
            _ => {}
        }
        i += 1;
    }
    variants
}

/// Build the initial struct and enum registries (names, and for an enum its
/// variant names, populated by the pre-pass; fields filled in once the real
/// `type:` bodies are parsed) from the pre-pass decls, leaking each name once
/// so every `Type::Struct`/`Type::Enum` naming it renders without a registry.
fn build_registries_into(
    decls: &[(String, Span, TypeDeclKind)],
    module: u32,
    structs: &mut Vec<StructDecl>,
    enums: &mut Vec<EnumDecl>,
) {
    for (name, span, kind) in decls {
        match kind {
            TypeDeclKind::Struct => {
                structs.push(StructDecl {
                    name: name.clone(),
                    name_static: Box::leak(name.clone().into_boxed_str()),
                    fields: Vec::new(),
                    span: *span,
                    has_drop_overload: false,
                    is_bundle: false,
                    module,
                });
            }
            TypeDeclKind::Enum(variant_names) => {
                let variants = variant_names
                    .iter()
                    .map(|(vname, vspan)| VariantDecl {
                        name: vname.clone(),
                        name_static: Box::leak(vname.clone().into_boxed_str()),
                        display_static: Box::leak(format!("{name}.{vname}").into_boxed_str()),
                        fields: Vec::new(),
                        span: *vspan,
                    })
                    .collect();
                enums.push(EnumDecl {
                    name: name.clone(),
                    name_static: Box::leak(name.clone().into_boxed_str()),
                    variants,
                    span: *span,
                    module,
                });
            }
        }
    }
}

/// The words, externs, and per-`type:`-body field lists parsed from one file's
/// tokens, plus its `export:` list. The field lists are in this module's
/// `type:` declaration order, so a caller fills them back into the registry at
/// this module's base offset.
pub struct ParsedBodies {
    pub words: Vec<WordDef>,
    pub externs: Vec<ExternDecl>,
    pub struct_fields_by_decl: Vec<Vec<(String, Type)>>,
    pub enum_fields_by_decl: Vec<Vec<Vec<(String, Type)>>>,
    pub exports: Vec<(String, Span)>,
    /// Phase 7 slice 2 (D1/D4): one entry per `static:` declaration, in
    /// source order.
    pub statics: Vec<StaticDecl>,
    /// P7.S3e (R4): one entry per `impl:` declaration, in source order.
    /// `trait:` declarations are not collected here -- they are fully parsed
    /// by the whole-closure `prepass_trait_decls` pass before any body
    /// parses (mirroring `prepass_generic_typedefs`), so this loop only ever
    /// skips past one (`skip_typedef`, reused verbatim: it just advances to
    /// the next `;`).
    pub impls: Vec<ImplDecl>,
}

/// Parse one module's bodies (R3): the word/extern definitions and `type:`
/// field bodies, resolving type names module-aware against the already-merged
/// `structs`/`enums` (own module first, then imports). `import:` forms are
/// consumed and discarded (the driver resolved the graph from a prior scan);
/// `export:` forms accumulate into the returned list (R7). Array/cell/ref
/// shapes intern into the shared registries so two files' `[i64 8]` dedupe to
/// one `ArrayId` (R13).
#[allow(clippy::too_many_arguments)]
pub fn parse_bodies(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    module: u32,
    imports: &HashMap<String, u32>,
    exports: &[Vec<(String, Span)>],
    selective: &HashMap<String, u32>,
    type_origin: &[HashMap<String, u32>],
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    generics: &mut GenericTypes,
    traits: &[TraitDecl],
) -> Result<ParsedBodies, String> {
    let mut out = ParsedBodies {
        words: Vec::new(),
        externs: Vec::new(),
        struct_fields_by_decl: Vec::new(),
        enum_fields_by_decl: Vec::new(),
        exports: Vec::new(),
        statics: Vec::new(),
        impls: Vec::new(),
    };
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices,
        module,
        imports,
        exports,
        selective,
        type_origin,
        generics,
        traits,
    };
    parser.parse_generic_typedefs()?;
    while parser.pos < parser.tokens.len() {
        if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "type:") {
            if parser.current_typedef_is_generic() {
                parser.skip_typedef();
            } else if parser.current_typedef_is_enum() {
                out.enum_fields_by_decl.push(parser.parse_enum_typedef()?);
            } else {
                out.struct_fields_by_decl.push(parser.parse_typedef()?);
            }
        } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "extern:") {
            out.externs.push(parser.parse_extern_decl()?);
        } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "trait:") {
            // P7.S3e (R1): already fully parsed by the whole-closure
            // `prepass_trait_decls` pass (mirroring how a generic `type:`
            // header is already fully parsed by `parse_generic_typedefs`
            // above); this loop only skips past it.
            parser.skip_typedef();
        } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "impl:") {
            let (imp, members) = parser.parse_impl_decl()?;
            out.impls.push(imp);
            out.words.extend(members);
        } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "import:") {
            parser.parse_import()?;
        } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "export:") {
            out.exports.extend(parser.parse_export()?);
        } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "static:") {
            out.statics.push(parser.parse_static_decl()?);
        } else {
            out.words.push(parser.parse_worddef()?);
        }
    }
    Ok(out)
}

/// Phase 5 slice 2 (OQ1): register one file's generic `type:` headers into the
/// shared `generics` registry. The driver runs this over every file in the
/// closure, after `prepass_and_register` has named every concrete type and
/// before any file's body parses, so a qualified application (`q::Box[i64]`)
/// resolves whatever the discovery order put the declaring file at.
///
/// Takes the same name environment `parse_bodies` will: a generic
/// declaration's field can name an imported concrete type, and it resolves the
/// same either side of the split.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepass_generic_typedefs(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    module: u32,
    imports: &HashMap<String, u32>,
    exports: &[Vec<(String, Span)>],
    selective: &HashMap<String, u32>,
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    generics: &mut GenericTypes,
) -> Result<(), String> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices,
        module,
        imports,
        exports,
        selective,
        generics,
        type_origin: &[],
        traits: crate::ast::predicate_traits(),
    };
    parser.parse_generic_typedefs()
}

/// Run one file's type pre-pass and append its structs/enums (names only,
/// fields filled later by `parse_bodies`) to the shared merged registries under
/// module id `module` (R3/R10). The driver calls this once per file across the
/// whole closure before any body parses.
pub fn prepass_and_register(
    tokens: &[(Token, Span)],
    module: u32,
    structs: &mut Vec<StructDecl>,
    enums: &mut Vec<EnumDecl>,
) -> Result<(), String> {
    let decls = prepass_type_decls(tokens)?;
    build_registries_into(&decls, module, structs, enums);
    Ok(())
}

/// P7.S3e (R1/R3, decision 4): register one file's `trait:` declarations
/// (full member signatures, not names only -- there is nothing further to
/// fill in later, unlike a concrete `type:`'s fields) into the shared
/// whole-program `traits` registry under module id `module`. The driver runs
/// this over every file in the closure -- after `prepass_and_register` and
/// the import/export/selective maps are known, alongside
/// `prepass_generic_typedefs` -- and before any file's body parses, so an
/// `impl:` binding (parsed in-line by `parse_bodies`) can resolve a
/// cross-module trait name regardless of which order the closure discovered
/// its files in (module 0, the entry file, is parsed *first*, and may import
/// a trait declared in a file discovered after it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepass_trait_decls(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    module: u32,
    imports: &HashMap<String, u32>,
    exports: &[Vec<(String, Span)>],
    selective: &HashMap<String, u32>,
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    generics: &mut GenericTypes,
    traits: &mut Vec<TraitDecl>,
) -> Result<(), String> {
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], (Token::Word(w), _) if w == "trait:") {
            let mut parser = Parser {
                tokens,
                pos: i,
                structs,
                enums,
                arrays,
                owned_cells,
                refs,
                slices,
                module,
                imports,
                exports,
                selective,
                generics,
                type_origin: &[],
                // A trait member's own signature can still name a bound
                // (`'T: Copy`) inside its `( ... )` effect, so this needs the
                // reserved-predicate table even though it never looks up a
                // user trait declared earlier in the registry-so-far.
                traits: crate::ast::predicate_traits(),
            };
            let decl = parser.parse_trait_decl()?;
            i = parser.pos;
            traits.push(decl);
            continue;
        }
        i += 1;
    }
    Ok(())
}

/// Parse one file's tokens into a whole `Module`: the single-file path, with
/// no import closure around it. The driver's multi-file path builds its own
/// `Module` around `parse_bodies` instead.
pub fn parse(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    prepass_and_register(tokens, 0, &mut structs, &mut enums)?;
    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    let mut slices = Vec::new();
    let no_imports: HashMap<String, u32> = HashMap::new();
    let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
    let mut traits = crate::ast::seed_predicate_traits();
    prepass_trait_decls(
        tokens,
        &structs,
        &enums,
        0,
        &no_imports,
        &[],
        &no_imports,
        &mut arrays,
        &mut owned_cells,
        &mut refs,
        &mut slices,
        &mut generics,
        &mut traits,
    )?;
    let bodies = parse_bodies(
        tokens,
        &structs,
        &enums,
        0,
        &no_imports,
        &[],
        &no_imports,
        &[],
        &mut arrays,
        &mut owned_cells,
        &mut refs,
        &mut slices,
        &mut generics,
        &traits,
    )?;
    for (idx, fields) in bodies.struct_fields_by_decl.into_iter().enumerate() {
        structs[idx].fields = fields;
    }
    for (idx, variant_fields) in bodies.enum_fields_by_decl.into_iter().enumerate() {
        for (vidx, fields) in variant_fields.into_iter().enumerate() {
            enums[idx].variants[vidx].fields = fields;
        }
    }
    // R4/D5: every monomorphized instantiation lands in the ordinary
    // registries, after the pre-pass entries its `StructId`/`EnumId` was
    // computed against, so the layout/accessor/destructor machinery walks it
    // like any hand-written concrete `type:`.
    //
    // P7 slice 3a phase 2 (R2): flushed and rebased, not dropped -- this
    // single-file path is a real check/lower entry too (used directly by
    // tests and by the `lib/` core modules' own parse), so it keeps `generics` alive
    // the same way `driver::assemble_module` does.
    generics.flush_structs_into(&mut structs);
    generics.flush_enums_into(&mut enums);
    generics.rebase(structs.len(), enums.len());
    Ok(Module {
        words: bodies.words,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices,
        generic_structs: generics.structs.clone(),
        generic_enums: generics.enums.clone(),
        externs: bodies.externs,
        instantiations: HashMap::new(),
        builtin_overloads: HashMap::new(),
        resolved_fields: HashMap::new(),
        generics,
        resolved_variant_fields: HashMap::new(),
        modules: vec![ModuleInfo {
            imports: HashMap::new(),
            exports: bodies.exports,
            selective: HashMap::new(),
            // P8 S2 (R2): the single-file, no-driver path (the REPL's own
            // parse, and every in-process test). It resolves no `import:` at
            // all, so it has nothing to derive a gate from; the driver builds
            // its own `ModuleInfo` per file in `assemble_module` and never
            // reaches this one, so the gate still holds where files are built.
            intrinsics: IntrinsicVisibility::All,
        }],
        statics: bodies.statics,
        traits,
        impls: bodies.impls,
    })
}

/// Split an import target word into its anchor and `::`-joined segments (F2).
/// A `self::` prefix is the SelfPackage anchor; anything else is
/// Dependency-anchored, its first segment naming a `depends:` entry. Bare
/// `self` with no `::` is an ordinary package name, not the prefix.
fn parse_module_name(word: &str, span: Span) -> Result<ModuleName, String> {
    let (anchor, rest) = match word.strip_prefix("self::") {
        Some(rest) => (ImportAnchor::SelfPackage, rest),
        None => (ImportAnchor::Dependency, word),
    };
    let segments: Vec<String> = rest.split("::").map(str::to_string).collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(format!(
            "parse error: import target `{word}` has an empty module-name segment at line {}, col {}",
            span.line, span.col
        ));
    }
    Ok(ModuleName { anchor, segments })
}

/// The qualifier an import binds when its source elides one (OQ3): a module
/// target's last segment, or a quoted path's file stem.
fn default_qualifier(target: &ImportTarget) -> Option<String> {
    match target {
        ImportTarget::Module(m) => m.segments.last().cloned(),
        ImportTarget::Path(p) => std::path::Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string),
    }
}

/// Scan a file's tokens for its `import:` forms (R2), parsing each in place so
/// the driver can resolve the import graph before any body parses. Mirrors
/// `prepass_type_decls`: it jumps to each `import:` keyword and parses the
/// whole form, so it needs no registries.
pub fn scan_imports(tokens: &[(Token, Span)]) -> Result<Vec<Import>, String> {
    let mut imports = Vec::new();
    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    let mut slices = Vec::new();
    // `structs`/`enums` below are `&[]`, so an instantiation would be
    // appended onto empty registries.
    let mut generics = GenericTypes::with_bases(0, 0);
    let no_imports: HashMap<String, u32> = HashMap::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], (Token::Word(w), _) if w == "import:") {
            let mut parser = Parser {
                tokens,
                pos: i,
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                owned_cells: &mut owned_cells,
                refs: &mut refs,
                slices: &mut slices,
                module: 0,
                imports: &no_imports,
                exports: &[],
                selective: &no_imports,
                generics: &mut generics,
                type_origin: &[],
                traits: crate::ast::predicate_traits(),
            };
            imports.push(parser.parse_import()?);
            i = parser.pos;
            continue;
        }
        i += 1;
    }
    Ok(imports)
}

/// Scan a file's tokens for its `export:` forms (R7), parsing each in place
/// so the driver can learn every module's export list ahead of any body
/// parse: an importer's effect may name a cross-module type before the
/// exporting file's own body has been parsed (`parse_bodies` runs per file in
/// discovery order, not necessarily dependency order). Multiple `export:`
/// lines accumulate (R7). Mirrors `scan_imports`.
pub fn scan_exports(tokens: &[(Token, Span)]) -> Result<Vec<(String, Span)>, String> {
    let mut exports = Vec::new();
    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    let mut slices = Vec::new();
    // `structs`/`enums` below are `&[]`, so an instantiation would be
    // appended onto empty registries.
    let mut generics = GenericTypes::with_bases(0, 0);
    let no_imports: HashMap<String, u32> = HashMap::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], (Token::Word(w), _) if w == "export:") {
            let mut parser = Parser {
                tokens,
                pos: i,
                structs: &[],
                enums: &[],
                arrays: &mut arrays,
                owned_cells: &mut owned_cells,
                refs: &mut refs,
                slices: &mut slices,
                module: 0,
                imports: &no_imports,
                exports: &[],
                selective: &no_imports,
                generics: &mut generics,
                type_origin: &[],
                traits: crate::ast::predicate_traits(),
            };
            exports.extend(parser.parse_export()?);
            i = parser.pos;
            continue;
        }
        i += 1;
    }
    Ok(exports)
}

/// Parse a single REPL line: a `:`-led definition, or a bare term sequence run
/// to end of input. One line is one complete unit (an unterminated def is a
/// normal parse error).
pub fn parse_line(tokens: &[(Token, Span)]) -> Result<Line, String> {
    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    let mut slices = Vec::new();
    parse_line_with_structs(
        tokens,
        &[],
        &[],
        &mut arrays,
        &mut owned_cells,
        &mut refs,
        &mut slices,
        ImportCtx::empty(),
    )
}

/// The three module-resolution tables a REPL parser entry point threads to
/// `Parser::resolve_type` (slice 5b, R8d): the qualifier->module `imports`
/// map, the selective bare-name->module map, and the per-module `export:`
/// lists that gate a qualified type reference. They always travel together, so
/// they ride one borrowed struct rather than three parallel parameters.
pub struct ImportCtx<'a> {
    pub imports: &'a HashMap<String, u32>,
    pub selective: &'a HashMap<String, u32>,
    pub exports: &'a [Vec<(String, Span)>],
}

impl ImportCtx<'_> {
    /// The no-import context: the native `.sth` parse path and any
    /// import-free REPL line resolve types with empty maps, exactly as before
    /// slice 5b threaded real ones.
    pub fn empty() -> ImportCtx<'static> {
        static NO_IMPORTS: std::sync::OnceLock<HashMap<String, u32>> = std::sync::OnceLock::new();
        let no_imports = NO_IMPORTS.get_or_init(HashMap::new);
        ImportCtx {
            imports: no_imports,
            selective: no_imports,
            exports: &[],
        }
    }
}

/// Parse a REPL line resolving struct and enum type names in a `:`
/// definition's effect against the session's registries, so a word may take
/// or return a previously-declared struct or enum. A bare expression carries
/// no type names, so the registries are unused there. `arrays` is the
/// session's interned array-type registry (R22/R23): a `[T N]` in a word
/// effect interns into it in place, so the `ArrayId` it returns stays valid
/// for later lines in the same session.
#[allow(clippy::too_many_arguments)]
pub fn parse_line_with_structs(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    slices: &mut Vec<SliceDecl>,
    ctx: ImportCtx,
) -> Result<Line, String> {
    // The REPL has no generic `type:` declarations (they are rejected at
    // declaration), so nothing here can apply one: a scratch registry, never read.
    let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices,
        module: 0,
        imports: ctx.imports,
        exports: ctx.exports,
        selective: ctx.selective,
        generics: &mut generics,
        type_origin: &[],
        // P7.S3e (R2): a REPL word def still needs `'T: Copy Ord` to work;
        // a user `trait:` declaration is not yet supported at REPL scope, so
        // the reserved predicate-only table is all this context ever sees.
        traits: crate::ast::predicate_traits(),
    };
    if matches!(parser.peek(), Some((Token::Word(w), _)) if w == ":") {
        let def = parser.parse_worddef()?;
        if let Some((tok, span)) = parser.peek() {
            return Err(format!(
                "parse error: unexpected {tok:?} after `;` at line {}, col {} (one line is one complete unit)",
                span.line, span.col
            ));
        }
        return Ok(Line::Def(def));
    }
    let mut terms = Vec::new();
    while parser.pos < parser.tokens.len() {
        terms.push(parser.parse_term()?);
    }
    Ok(Line::Expr(terms))
}

/// Parse a single REPL `type:` line into its ordered `(field-name, Type)`
/// list, resolving field types against `structs` (the session's accumulated
/// registry, with the just-declared name already appended so a self-reference
/// resolves, which the checker then rejects as recursion). Trailing
/// tokens after `;` are a located error (one line is one complete unit).
pub fn parse_typedef_line(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    ctx: ImportCtx,
) -> Result<Vec<(String, Type)>, String> {
    // The REPL has no generic `type:` declarations (they are rejected at
    // declaration), so nothing here can apply one: a scratch registry, never read.
    // A `type:` field can never be a slice (a slice is banned from every
    // field position, so the checker rejects one), and nothing else in a
    // typedef line resolves a slice type: a scratch registry, never read.
    let mut slices = Vec::new();
    let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices: &mut slices,
        module: 0,
        imports: ctx.imports,
        exports: ctx.exports,
        selective: ctx.selective,
        generics: &mut generics,
        type_origin: &[],
        traits: crate::ast::predicate_traits(),
    };
    reject_generic_typedef_in_repl(&parser)?;
    let fields = parser.parse_typedef()?;
    if let Some((tok, span)) = parser.peek() {
        return Err(format!(
            "parse error: unexpected {tok:?} after `;` at line {}, col {} (one line is one complete unit)",
            span.line, span.col
        ));
    }
    Ok(fields)
}

/// Phase 5 slice 1: a generic `type:` header (`type: Box 'T ...`) has no REPL
/// support yet -- `parse_typedef_line`/`parse_enum_typedef_line` only ever
/// reach the concrete productions, so without this gate a generic header runs
/// straight into `parse_typedef`'s/`parse_enum_typedef`'s field loop and
/// reports a nonsense "unknown type" error naming a type variable. `parser`
/// must not have consumed any tokens yet (`self.pos` still points at `type:`).
fn reject_generic_typedef_in_repl(parser: &Parser) -> Result<(), String> {
    if parser.current_typedef_is_generic() {
        let (_, span) = parser.tokens[parser.pos];
        return Err(format!(
            "error: generic `type:` declarations are not supported in the REPL yet at line {}, col {}",
            span.line, span.col
        ));
    }
    Ok(())
}

/// Whether a `type:` line is an enum declaration (a `|`-bearing body, D1), so
/// the REPL routes it to the enum registry rather than the struct one.
/// `tokens` must start at `type:`.
pub fn typedef_line_is_enum(tokens: &[(Token, Span)]) -> bool {
    body_has_pipe_before_semicolon(tokens, 2)
}

/// The `(name, span)` of every variant in a `type:` enum line, in source
/// order (D8's variant pre-pass at REPL scope), so the REPL can register the
/// variant-name skeleton before parsing variant fields. `tokens` must start
/// at `type:`.
pub fn enum_variant_names(tokens: &[(Token, Span)]) -> Vec<(String, Span)> {
    scan_variant_names(tokens, 2)
}

/// Parse a single REPL `type:` enum line into its ordered per-variant
/// `(field-name, Type)` lists, resolving field types against the session's
/// registries (the just-declared enum already appended so a self-reference
/// resolves, which the checker then rejects as recursion). Trailing tokens
/// after `;` are a located error.
pub fn parse_enum_typedef_line(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
    ctx: ImportCtx,
) -> Result<Vec<Vec<(String, Type)>>, String> {
    // The REPL has no generic `type:` declarations (they are rejected at
    // declaration), so nothing here can apply one: a scratch registry, never read.
    // A `type:` field can never be a slice (a slice is banned from every
    // field position, so the checker rejects one), and nothing else in a
    // typedef line resolves a slice type: a scratch registry, never read.
    let mut slices = Vec::new();
    let mut generics = GenericTypes::with_bases(structs.len(), enums.len());
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        slices: &mut slices,
        module: 0,
        imports: ctx.imports,
        exports: ctx.exports,
        selective: ctx.selective,
        generics: &mut generics,
        type_origin: &[],
        traits: crate::ast::predicate_traits(),
    };
    reject_generic_typedef_in_repl(&parser)?;
    let variant_fields = parser.parse_enum_typedef()?;
    if let Some((tok, span)) = parser.peek() {
        return Err(format!(
            "parse error: unexpected {tok:?} after `;` at line {}, col {} (one line is one complete unit)",
            span.line, span.col
        ));
    }
    Ok(variant_fields)
}

/// P7.S3n (R2): a generic `type:` header on its own -- the declared name, the
/// bound type variables with their spans (for the phantom and duplicate
/// diagnostics), and the `type:` keyword's span. Registered as a placeholder
/// by `parse_generic_typedefs`' stage (a), then handed back to stage (b) to
/// parse that header's own field/variant list against.
type GenericHeader = (String, Vec<(String, Span)>, Span);

/// A parsed polymorphic type before folding to `PolyType`: a concrete type, a
/// type variable (already interned to its id), or an array whose element
/// and/or count may itself be variable.
enum RawTy {
    Concrete(Type),
    Var(u32),
    Array(Box<RawTy>, RawLen),
    /// Slice 6a (R5): a quotation effect whose rows may mention the
    /// signature's variables, folded to `PolyType::Quotation` (or
    /// `Concrete(Type::Quotation)` when fully concrete) by `raw_to_poly_type`.
    /// The trailing `bool` is Slice 10a (R1): whether the effect opened on a
    /// `~[` token rather than a plain `[`. The two trailing `Option<u32>`s
    /// are Slice 10a (R7): the input/output row variable this quotation
    /// effect declared, if any (R4: it can only be the signature's own
    /// top-level row, so it is already an id in that row id space).
    Quotation(Vec<RawTy>, Vec<RawTy>, bool, Option<u32>, Option<u32>),
    /// Slice 13 (R-A2): a `&`-led slot whose referent may itself be variable
    /// (`&'T`, `&!['T 'N]`), folded to `PolyType::Ref` -- or to
    /// `Concrete(Type::Ref)` when the referent folds fully concrete -- by
    /// `raw_to_poly_type`.
    Ref(Box<RawTy>, bool),
    /// P7.S3n (R3): a `^`-led slot whose payload may itself be variable
    /// (`^'T`, `^['T 4]`), folded to `PolyType::OwnedCell` -- or to
    /// `Concrete(Type::OwnedCell)` when the payload folds fully concrete --
    /// by `raw_to_poly_type`, exactly as `Ref` folds.
    OwnedCell(Box<RawTy>),
    /// P7 slice 3a (R1): a generic type applied to poly slots
    /// (`Result['T 'E]`), folded to `PolyType::Generic` -- or to a plain
    /// `Concrete` by instantiating through `GenericTypes`, exactly as the
    /// concrete path already does -- by `raw_to_poly_type`. `name` is the
    /// header's own declared spelling, carried through for diagnostics and
    /// for the depth > 1 rejection (D5), which needs both the outer and the
    /// inner header's name.
    Generic {
        is_enum: bool,
        idx: usize,
        module: u32,
        args: Vec<RawTy>,
        name: String,
        span: Span,
    },
}

enum RawLen {
    Concrete(u32),
    Var(u32),
}

/// The kind a `'`-name was first used as: X1 rejects the same name appearing
/// as both a type variable and a length variable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VarKind {
    Ty,
    Len,
}

/// Accumulates a polymorphic signature's variables as an effect is parsed
/// left-to-right. Variable id spaces are per-signature and assigned in
/// binding (first-mention) order; the `*_names` vectors are the id -> spelling
/// tables the diagnostics and `PolySig` carry.
#[derive(Default)]
struct PolyBuilder {
    ty_names: Vec<String>,
    len_names: Vec<String>,
    ty_index: HashMap<String, u32>,
    len_index: HashMap<String, u32>,
    kind: HashMap<String, VarKind>,
    bounds: Vec<(u32, Bound)>,
    row_in: Option<u32>,
    row_out: Option<u32>,
    /// Slice 10a (R7): the row-name id table, shared by the top-level row
    /// (`row_in`/`row_out` above) and by any row mentioned inside a
    /// quotation effect (R4), so both live in the same id space.
    row_names: Vec<String>,
    row_index: HashMap<String, u32>,
    /// Slice 10c (R-P2-1): a row named on a quotation effect's *output* side
    /// may denote the signature's own top-level output row named only later
    /// in the signature (e.g. `..o` in `~[ ..i -- ..o ]`, on a word whose
    /// top-level output is `-- ..o`). Such a mention is interned optimistically
    /// and its (id, name, span) recorded here; `validate_pending_quotation_rows`
    /// checks each one once the whole signature is known, since only then can
    /// "is this the signature's own top-level row" be answered.
    pending_quotation_rows: Vec<(u32, String, Span)>,
}

impl PolyBuilder {
    /// Intern a row name, assigning it a fresh id on first sight.
    fn row_id(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.row_index.get(name) {
            return id;
        }
        let id = self.row_names.len() as u32;
        self.row_names.push(name.to_string());
        self.row_index.insert(name.to_string(), id);
        id
    }

    /// Record a side's row variable (`..s`), rejecting a second one on the
    /// same side (X2). Placement (deepest only) is enforced by the caller.
    fn set_row(&mut self, is_output: bool, name: String, span: Span) -> Result<(), String> {
        if if is_output { self.row_out } else { self.row_in }.is_some() {
            return Err(row_var_misplaced_error(&name, span));
        }
        let id = self.row_id(&name);
        if is_output {
            self.row_out = Some(id);
        } else {
            self.row_in = Some(id);
        }
        Ok(())
    }

    /// Slice 10a (R4): a `..`-prefixed name mentioned inside a quotation
    /// effect's *input* side must already denote the signature's own
    /// top-level row -- a fresh name, or any row when the signature declared
    /// none at top level, is a located error. This stays strict/immediate
    /// (unlike the output-side sibling below): the stack region present when
    /// a quotation *begins* executing can only ever be a row already declared
    /// by this point in the signature (10c R-P2-1's forward reference is
    /// specifically for a row named only later, which describes what a
    /// quotation *leaves*, never what it starts with).
    ///
    /// Review fix: this checks `self.row_in`/`self.row_out` directly, not
    /// mere presence in `row_index`. `row_index` also holds names optimistically
    /// interned by `quotation_row_id_deferred` for a *sibling* quotation's
    /// output side, not yet confirmed by `validate_pending_quotation_rows`; a
    /// bare `row_index` lookup let one quotation's still-unconfirmed output
    /// mention leak into a later quotation's strict input-side check, accepting
    /// or rejecting the same signature depending on clause order.
    fn quotation_row_id(&mut self, name: &str, span: Span) -> Result<u32, String> {
        self.row_index
            .get(name)
            .copied()
            .filter(|id| Some(*id) == self.row_in || Some(*id) == self.row_out)
            .ok_or_else(|| quotation_row_not_top_level_error(name, span))
    }

    /// Slice 10c (R-P2-1): a `..`-prefixed name mentioned on a quotation
    /// effect's *output* side may denote the signature's own top-level output
    /// row, named only later in the signature (`~[ ..i -- ..o ]` parsed while
    /// still inside the word's input side, `..o` not yet bound by `set_row`).
    /// Intern it optimistically and defer the "is this actually one of the
    /// signature's own top-level rows" check to `validate_pending_quotation_rows`,
    /// once the whole signature -- including a top-level row declared after
    /// this point -- is known.
    fn quotation_row_id_deferred(&mut self, name: &str, span: Span) -> u32 {
        let id = self.row_id(name);
        self.pending_quotation_rows
            .push((id, name.to_string(), span));
        id
    }

    /// Slice 10c (R-P2-1): resolve every deferred output-side row mention
    /// against the now-complete signature. A mention that is neither the
    /// top-level input nor output row is a fresh name (or a row belonging to
    /// no top-level side at all) and is rejected exactly as the strict,
    /// immediate check rejects one on the input side.
    fn validate_pending_quotation_rows(&self) -> Result<(), String> {
        for (id, name, span) in &self.pending_quotation_rows {
            if Some(*id) != self.row_in && Some(*id) != self.row_out {
                return Err(quotation_row_not_top_level_error(name, *span));
            }
        }
        Ok(())
    }

    /// Intern a type variable, returning its id and whether this is its
    /// binding (first) occurrence. A name already seen in a count position is
    /// X1.
    fn intern_ty_var(&mut self, name: &str, span: Span) -> Result<(u32, bool), String> {
        if self.kind.get(name) == Some(&VarKind::Len) {
            return Err(var_kind_conflict_error(name, span));
        }
        self.kind.insert(name.to_string(), VarKind::Ty);
        if let Some(&id) = self.ty_index.get(name) {
            return Ok((id, false));
        }
        let id = self.ty_names.len() as u32;
        self.ty_names.push(name.to_string());
        self.ty_index.insert(name.to_string(), id);
        Ok((id, true))
    }

    /// Intern a length variable (an array count `'N`). A name already seen in
    /// a type position is X1.
    fn intern_len_var(&mut self, name: &str, span: Span) -> Result<u32, String> {
        if self.kind.get(name) == Some(&VarKind::Ty) {
            return Err(var_kind_conflict_error(name, span));
        }
        self.kind.insert(name.to_string(), VarKind::Len);
        if let Some(&id) = self.len_index.get(name) {
            return Ok(id);
        }
        let id = self.len_names.len() as u32;
        self.len_names.push(name.to_string());
        self.len_index.insert(name.to_string(), id);
        Ok(id)
    }

    fn finish(self, inputs: Vec<PolyType>, outputs: Vec<PolyType>) -> PolySig {
        PolySig {
            row_in: self.row_in,
            inputs,
            outputs,
            row_out: self.row_out,
            bounds: self.bounds,
            ty_var_names: self.ty_names,
            len_var_names: self.len_names,
            row_var_names: self.row_names,
        }
    }
}

/// Slice 10a (R2): a `~[` reached from a monomorphic type-expression
/// context -- a struct/array/cell field, a ref/owning-cell referent, an
/// `extern:` parameter, or a non-poly word's own slot. `~` is legal only as
/// a poly combinator's own declared parameter, spelled through
/// `parse_poly_slot`, never through `parse_type_expr`/`parse_slot`/
/// `parse_field_type_expr`.
fn tilde_quotation_position_error(span: Span) -> String {
    format!(
        "error: a `~` quotation cannot appear here at line {}, col {} (`~` is only legal as a word's own declared quotation parameter, never a field, output, referent, or extern parameter type)",
        span.line, span.col
    )
}

fn row_var_misplaced_error(name: &str, span: Span) -> String {
    format!(
        "error: row variable `{name}` at line {}, col {} may appear only once, at the deepest (leftmost) slot of a side",
        span.line, span.col
    )
}

/// Phase 6 slice 1 (R6): a quotation annotation parses only in its full form,
/// so a parenthesized list that reaches `)` without a `--` -- including the
/// empty `( )` -- is rejected rather than read as an elided effect.
fn annotation_missing_arrow_error(span: Span) -> String {
    format!(
        "parse error: a quotation annotation must be written in full as `( inputs -- outputs )`, found `)` with no `--` at line {}, col {}",
        span.line, span.col
    )
}

/// Phase 6 slice 1 (R2): a quotation annotation's own variable tables, minted
/// per literal and disjoint from any enclosing signature's `PolySig` -- an
/// annotation has no signature to borrow an id space from.
#[derive(Default)]
struct AnnotVars {
    ty_names: Vec<String>,
    row_names: Vec<String>,
    row_in: Option<u32>,
    row_out: Option<u32>,
}

/// Intern a variable spelling, assigning a fresh id on first sight.
fn intern_var_name(table: &mut Vec<String>, name: &str) -> u32 {
    if let Some(i) = table.iter().position(|n| n == name) {
        return i as u32;
    }
    table.push(name.to_string());
    (table.len() - 1) as u32
}

/// Slice 10a (R4): a `..`-prefixed name inside a quotation effect that does
/// not yet denote a row already declared at the signature's own top level --
/// either a fresh name, or a row declared only later in the signature (e.g.
/// an output-side row named from inside the input side), which is not the
/// stack region present when the quotation executes.
fn quotation_row_not_top_level_error(name: &str, span: Span) -> String {
    format!(
        "error: row variable `{name}` at line {}, col {} inside a quotation effect must be the signature's own top-level row, already declared by this point (a row named only later in the signature does not count)",
        span.line, span.col
    )
}

/// Slice 10a (R5): a quotation effect's row appears on one side only.
fn quotation_row_one_sided_error(name: &str, span: Span) -> String {
    format!(
        "error: row variable `{name}` at line {}, col {} must appear on both sides of the quotation effect, or neither",
        span.line, span.col
    )
}

/// Slice 10a (R5): a quotation effect declares a differing input/output row
/// -- fixed exact text, since 10a's loop-body shape (the back-edge fixed
/// point) requires the same row on both sides; only 10c, for a word without
/// a back-edge, lifts this.
fn quotation_row_shape_change_error(row_in: &str, row_out: &str) -> String {
    format!(
        "error: a loop body cannot change the shape of the carried region: `{row_in}` in, `{row_out}` out\nnote: 10c lifts this for a word without a back-edge"
    )
}

/// Slice 10a (R5, review fix): a quotation effect's row has an unknown size
/// at runtime, so only an inline (`~[ ... ]`) quotation -- spliced at its
/// call site, never materialized -- may carry one. An ordinary quotation
/// with a row would need to ground that row at a real value the checker
/// cannot produce.
fn quotation_row_requires_inline_error(name: &str, span: Span) -> String {
    format!(
        "error: a quotation effect with a row (`{name}`) must be inline (`~[ ... ]`) at line {}, col {}: a row's size is unknown at runtime, so only a splice-only quotation may carry one",
        span.line, span.col
    )
}

/// A `&`/`&!` sigil with nothing after it to borrow. Shared by the concrete
/// type-expression path and Slice 13's poly-slot interception so both spell
/// the same fact the same way.
fn ref_no_referent_error(word: &str, span: Span) -> String {
    format!(
        "error: reference type `{word}` has no referent type at line {}, col {} (write `{word}T` for some type T)",
        span.line, span.col
    )
}

fn bound_on_use_error(name: &str, span: Span) -> String {
    format!(
        "error: bound on `{name}` at line {}, col {} must be written at its binding (first) occurrence, not a use",
        span.line, span.col
    )
}

/// R16 (phase 2): a qualified reference to a name that exists in the target
/// module but is not on its `export:` list. Distinct wording from an unknown
/// name (which has its own error, `resolve_type`'s `unknown type` and
/// `check.rs`'s `unknown_word_error`), so the two cases are never conflated.
fn not_exported_error(name: &str, qualifier: &str, span: Span) -> String {
    format!(
        "error: `{name}` is not exported from module `{qualifier}` at line {}, col {}",
        span.line, span.col
    )
}

fn unknown_capability_error(name: &str, span: Span) -> String {
    format!(
        "error: unknown capability `{name}` at line {}, col {} (a bound names `Copy`, `Ord`, or a trait in scope)",
        span.line, span.col
    )
}

/// P7.S3e (R18): a bound naming a qualified trait (`'T: q::Show`) whose
/// qualifier is not one of this module's import aliases. `parse_capabilities`
/// has no `resolve_type` to delegate to (the way `type_is_exported`'s callers
/// do), so the unbound qualifier needs its own located rejection here.
fn unbound_bound_qualifier_error(qualifier: &str, base: &str, span: Span) -> String {
    format!(
        "error: unknown module qualifier `{qualifier}` in bound `{qualifier}::{base}` at line {}, col {} (a qualified bound names an import alias)",
        span.line, span.col
    )
}

fn var_kind_conflict_error(name: &str, span: Span) -> String {
    format!(
        "error: variable `{name}` at line {}, col {} is used as both a type variable and a length variable; these are two different variables",
        span.line, span.col
    )
}

/// Phase 5 slice 1 (R1, round-3 review): a generic `type:` header binding
/// the same variable name twice (`type: Bad 'T 'T ...`). Caught here, at the
/// binding site, rather than left to surface as an unbound-or-phantom error
/// once a field references the name: the second binding shadows nothing (the
/// header has no scoping), so a field naming it would otherwise resolve
/// against whichever entry `position()` finds first and silently mark the
/// duplicate entry itself as the phantom.
fn duplicate_generic_ty_var_error(name: &str, decl_name: &str, span: Span) -> String {
    format!(
        "error: type variable `{name}` at line {}, col {} is bound twice by `type: {decl_name}`'s header",
        span.line, span.col
    )
}

/// Phase 5 slice 1 (R1): a generic `type:` field naming a `'`-prefixed
/// variable its header never bound.
fn unbound_generic_ty_var_error(name: &str, decl_name: &str, span: Span) -> String {
    format!(
        "error: `{name}` at line {}, col {} is not a type variable bound by `type: {decl_name}`'s header",
        span.line, span.col
    )
}

/// Phase 5 slice 1 (R1, added during round-2 review): a generic `type:`
/// header binds a variable no field ever references. Rejected at
/// declaration time so R5's instantiation dispatch -- which disambiguates
/// two instantiations by their generated constructor's *input* types alone
/// -- never has to handle two instantiations whose constructors agree on
/// every input and differ only in a phantom output type.
fn phantom_ty_var_error(decl_name: &str, name: &str, span: Span) -> String {
    format!(
        "error: type variable `{name}` at line {}, col {} is bound by `type: {decl_name}`'s header but appears in no field (a phantom parameter cannot be disambiguated at a call site)",
        span.line, span.col
    )
}

/// Phase 5 slice 1: the generic path's twin of the concrete odd-field-count
/// error. It names the header's bound variables because the likeliest way to
/// reach it is writing a `'`-prefixed *field* name directly after the type
/// name (`type: Foo 'bar i64 ;`), which the header scan consumes as a type
/// parameter -- leaving the plain message pointing at a token the author never
/// got wrong.
fn generic_odd_field_count_error(
    decl_name: &str,
    ty_vars: &[(String, Span)],
    field_name: &str,
    before: &str,
    span: Span,
) -> String {
    let header: Vec<&str> = ty_vars.iter().map(|(n, _)| n.as_str()).collect();
    format!(
        "parse error: field `{field_name}` has no type before `{before}` at line {}, col {} (odd field-token count in the body of generic `type: {decl_name} {}`; a `'`-prefixed word after the type name binds a type parameter)",
        span.line,
        span.col,
        header.join(" "),
    )
}

/// Phase 5 slice 1 (R3): a generic-type application whose argument count
/// doesn't match its header's, naming the type, the number of variables the
/// header declares, and the number of arguments the use site supplied. A bare
/// generic name with no `[...]` at all reports as zero arguments: a generic
/// type is never a type by itself.
fn generic_arity_error(name: &str, declared: usize, supplied: usize, span: Span) -> String {
    let declared_str = if declared == 1 {
        "1 type variable".to_string()
    } else {
        format!("{declared} type variables")
    };
    let supplied_str = match supplied {
        0 => "none were".to_string(),
        1 => "1 was".to_string(),
        n => format!("{n} were"),
    };
    format!(
        "error: generic type `{name}` declares {declared_str}, but {supplied_str} supplied at line {}, col {} (apply it as `{name}[{}]`, one type argument per declared variable)",
        span.line,
        span.col,
        vec!["T"; declared].join(" "),
    )
}

/// P7 slice 3a (D5): a generic type argument that is itself a generic
/// application (`Box[Box['T]]`), rejected at nesting depth > 1 -- v1
/// represents this shape but never grounds it, and no consumer forces it
/// (the brief's OQ4).
/// P7.S3n (R1): the surface spelling of a generic `type:` field's type, in
/// the declaration's own variable spellings. `PolyType` carries variable
/// *indices*, so rendering one needs the header's `ty_vars` table; the
/// checker's `poly_type_str` does the same job against a `PolySig` instead.
fn generic_field_type_str(pty: &PolyType, ty_vars: &[(String, Span)]) -> String {
    match pty {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => ty_vars[*v as usize].0.clone(),
        PolyType::Array(elem, len) => {
            let n = match len {
                Len::Concrete(n) => n.to_string(),
                // N3: a struct header binds no length variable, so R1's
                // array arm only ever builds `Len::Concrete`.
                Len::Var(_) => unreachable!("a generic `type:` field has no length variable"),
            };
            format!("[{} {}]", generic_field_type_str(elem, ty_vars), n)
        }
        PolyType::Ref(referent, mutable) => format!(
            "&{}{}",
            if *mutable { "!" } else { "" },
            generic_field_type_str(referent, ty_vars)
        ),
        PolyType::OwnedCell(payload) => {
            format!("^{}", generic_field_type_str(payload, ty_vars))
        }
        PolyType::Generic { name, args, .. } => {
            let args: Vec<String> = args
                .iter()
                .map(|a| generic_field_type_str(a, ty_vars))
                .collect();
            format!("{name}[{}]", args.join(" "))
        }
        // R7 rejects a variable-bearing quotation field at the parser, and a
        // concrete one folds to `Concrete`, so neither reaches here.
        PolyType::Quotation(..) | PolyType::QuotLit => {
            unreachable!("a generic `type:` field is never a quotation shape")
        }
    }
}

/// P7.S3n (R7): a quotation-typed field naming the declaration's own type
/// variable. Out of scope this slice, and a located rejection rather than an
/// `unknown type` misreport or a panic downstream.
fn quotation_field_ty_var_error(decl_name: &str, var: &str, span: Span) -> String {
    format!(
        "error: a quotation field naming `{decl_name}`'s type variable `{var}` at line {}, col {} is not supported\n  a quotation type in a generic `type:` field may only mention concrete types",
        span.line, span.col
    )
}

/// P7.S3n (R8): reject a **growing** generic application anywhere in a
/// field's type tree -- an application one of whose arguments is *compound*
/// and mentions one of the declaration's own type variables (`L[['T 2]]`,
/// `L[^'T]`, `L[&'T]`). Each such hop would instantiate the header at a
/// strictly larger argument than the last, forever.
///
/// The walk descends through every wrapper, not just a field whose own top
/// level is an application: `^L[^'T]` is a cell over the application, and
/// `[Ent['K 'V] 8]` is an array over one. An argument that is fully concrete
/// at any depth (`L[[i64 2]]`) is inert -- it carries no variable to grow --
/// and a bare `'T` argument passes through unchanged, so both are admitted.
///
/// Accepted over-rejection, stated so a future slice can lift it: a
/// *non-recursive* wrapping application (`Outer 'T f Ent[['T 2] i64]`, where
/// `Ent` never names `Outer` back) also terminates, and is rejected here
/// anyway. Admitting it needs an SCC pass over a header-level dependency
/// graph, which nothing currently wanted requires -- `Map`'s backing store is
/// `[Ent['K 'V] N]`, an array *of* an application with bare-variable
/// arguments.
fn reject_growing_generic_argument(
    decl_name: &str,
    pty: &PolyType,
    span: Span,
) -> Result<(), String> {
    match pty {
        PolyType::Concrete(_) | PolyType::Var(_) => Ok(()),
        PolyType::Array(elem, _) => reject_growing_generic_argument(decl_name, elem, span),
        PolyType::Ref(referent, _) => reject_growing_generic_argument(decl_name, referent, span),
        PolyType::OwnedCell(payload) => reject_growing_generic_argument(decl_name, payload, span),
        PolyType::Generic { name, args, .. } => {
            for arg in args {
                if !matches!(arg, PolyType::Concrete(_) | PolyType::Var(_)) {
                    return Err(growing_generic_self_reference_error(
                        decl_name, name, arg, span,
                    ));
                }
            }
            Ok(())
        }
        PolyType::Quotation(..) | PolyType::QuotLit => Ok(()),
    }
}

/// P7.S3n (R8)'s diagnostic. Names the *restriction* explicitly rather than
/// just saying "recursive": the type may well not be recursive at all (see
/// `reject_growing_generic_argument`'s accepted over-rejection), and a user
/// who hits this needs to know which shape is refused, not to be told
/// something false about their declaration.
fn growing_generic_self_reference_error(
    decl_name: &str,
    applied: &str,
    arg: &PolyType,
    span: Span,
) -> String {
    // The argument is compound and mentions a variable, so it renders
    // through the shape names alone -- `ty_vars` spellings are not needed to
    // say *which shape* is refused.
    let shape = match arg {
        PolyType::Array(..) => "an array of",
        PolyType::Ref(..) => "a reference to",
        PolyType::OwnedCell(_) => "an owning cell over",
        PolyType::Generic { .. } => "another generic application over",
        _ => "a compound type over",
    };
    format!(
        "error: `{decl_name}` applies `{applied}` to {shape} one of its own type variables at line {}, col {}\n  a generic argument must be either fully concrete or a bare type variable: wrapping one grows the type at every instantiation, so it could never be laid out",
        span.line, span.col
    )
}

/// A `^`-led type with nothing following it to own. Shared by the concrete
/// splitter (`split_owning_cell_word`) and P7.S3n's poly `^`-arm, so both
/// paths report the same wording for the same defect.
fn owned_cell_no_payload_error(word: &str, span: Span) -> String {
    format!(
        "error: owning-cell type `{word}` has no payload type at line {}, col {} (write `{word}T` for some type T)",
        span.line, span.col
    )
}

fn generic_nesting_depth_error(outer: &str, inner: &str, span: Span) -> String {
    format!(
        "error: `{outer}[...]` at line {}, col {} names `{inner}[...]` as a type argument, but a generic applied to another generic (nesting depth > 1) is not yet supported",
        span.line, span.col
    )
}

/// Phase 5 slice 1 (R1): the one phantom-variable gate both
/// `parse_generic_typedef_fields` and `parse_generic_enum_typedef_variants`
/// call once their whole field/variant list is known.
fn check_no_phantom_ty_var(
    decl_name: &str,
    ty_vars: &[(String, Span)],
    used: &[bool],
) -> Result<(), String> {
    if let Some(idx) = used.iter().position(|&u| !u) {
        let (name, span) = &ty_vars[idx];
        return Err(phantom_ty_var_error(decl_name, name, *span));
    }
    Ok(())
}

struct Parser<'t> {
    tokens: &'t [(Token, Span)],
    pos: usize,
    /// The struct registry (names always populated by the pre-pass, fields
    /// populated for the `type:` bodies already parsed at the point of
    /// lookup, but resolution only needs the id/name so declaration order
    /// among structs doesn't matter). Empty for a REPL line (struct
    /// declarations are not yet supported at REPL scope).
    structs: &'t [StructDecl],
    /// The enum registry, parallel to `structs` (names, and each enum's
    /// variant names, always populated by the pre-pass; empty for a REPL
    /// line, enum declarations are not yet supported at REPL scope).
    enums: &'t [EnumDecl],
    /// The interned array-type registry (D3, M1): unlike `structs`/`enums`,
    /// an array shape has no declared name a pre-pass could register ahead
    /// of time, so this grows during type-expression resolution rather than
    /// being pre-populated. A mutable borrow of the caller's registry (the
    /// whole-module `Module.arrays` for a native build, the session's
    /// `arrays` for a REPL line), so interning persists across REPL lines
    /// (R22/R23).
    arrays: &'t mut Vec<ArrayDecl>,
    /// The interned owning-cell registry, mirroring `arrays` for the same
    /// reason: a `^T` shape has no declared name a pre-pass could register
    /// ahead of time, so it grows during type-expression resolution and
    /// persists across REPL lines exactly like `arrays`.
    owned_cells: &'t mut Vec<OwnedCellDecl>,
    /// The interned reference registry, mirroring `owned_cells`: a `&T`/`&!T`
    /// shape has no declared name either, so it grows as type expressions
    /// resolve and persists across REPL lines.
    refs: &'t mut Vec<RefDecl>,
    /// P7 slice 3c (R1.2): the interned slice registry, mirroring `refs` --
    /// a `Slice[T]` shape has no declared name, so it grows as type
    /// expressions resolve. The checker's `slice`/`subslice` words intern
    /// into the same registry, so a view built at check time and one spelled
    /// in a signature share a `SliceId`.
    slices: &'t mut Vec<SliceDecl>,
    /// Phase 4 slice 5a (R11): the module id whose body this parser is
    /// currently reading. `0` for a single-file program and every REPL line;
    /// the driver's closure assembly sets it per file. An unqualified type
    /// name resolves against this module first.
    module: u32,
    /// Phase 4 slice 5a (R8): this module's qualifier->module import map, used
    /// to resolve a `q::Type` type name. Empty for a single-file program and
    /// REPL line.
    imports: &'t std::collections::HashMap<String, u32>,
    /// Phase 4 slice 5a phase 2 (R16): every module's `export:` list, indexed
    /// by module id, scanned ahead of any body parse (`scan_exports`) so a
    /// cross-module type name in an effect can be visibility-checked even
    /// though the exporting file's own body may not have parsed yet. Empty for
    /// a single-file program and every REPL line, where no qualified name can
    /// occur.
    exports: &'t [Vec<(String, Span)>],
    /// Phase 4 slice 5a phase 4 (R20/R15c): this module's selectively-imported
    /// unqualified names, each mapping to the target module it resolves in. A
    /// bare `Type` (or word) exposed by `import: "..." q | Type | ` resolves
    /// here after the own-module lookup fails (own-module-first, R11). Empty
    /// for a single-file program and every REPL line.
    selective: &'t std::collections::HashMap<String, u32>,
    /// P7.S3q-follow: for a module reached through `imports`/`selective`,
    /// the true declaring module of a name on *its* `export:` list, when that
    /// name is a re-export rather than something it declares itself --
    /// closing the gap where a type name reached only through a hub resolved
    /// fine in term position (the late, whole-program `resolve.rs` pass
    /// already walks a hub chain there) but not in an effect signature,
    /// which resolves during this early parse via a single hop. Indexed by
    /// module id, empty for a REPL line and any parse path with no real
    /// cross-module data.
    type_origin: &'t [std::collections::HashMap<String, u32>],
    /// Phase 5 slice 1 (R2/D5): the generic `type:` declarations in scope and
    /// the concrete struct/enum registry each application of one mints. A
    /// mutable borrow for the same reason `arrays` is one: an instantiation
    /// is minted *while* a field or slot type expression resolves. Empty (and
    /// never written) for a REPL line and for the import/export scans, which
    /// have no generic declaration to apply.
    generics: &'t mut GenericTypes,
    /// P7.S3e (R3): the whole-program trait registry (pre-seeded `Copy`/`Ord`
    /// plus every user `trait:` declaration in the closure), populated by
    /// `prepass_trait_decls` before any body parses -- mirrors `structs`/
    /// `enums`. Empty for a REPL line (`trait:` is not yet supported at REPL
    /// scope, the same bypass pattern `structs`/`enums` already follow
    /// there).
    traits: &'t [TraitDecl],
}

impl<'t> Parser<'t> {
    fn peek(&self) -> Option<&(Token, Span)> {
        self.tokens.get(self.pos)
    }

    fn eof_error(&self, expected: &str) -> String {
        let span = self.tokens.last().map(|(_, s)| *s).unwrap_or_default();
        format!(
            "parse error: unexpected end of input, expected {expected} (last token at line {}, col {})",
            span.line, span.col
        )
    }

    fn expect(&mut self, expected: Token) -> Result<Span, String> {
        match self.peek() {
            Some((tok, span)) if *tok == expected => {
                let span = *span;
                self.pos += 1;
                Ok(span)
            }
            Some((tok, span)) => Err(format!(
                "parse error: expected {expected:?}, found {tok:?} at line {}, col {}",
                span.line, span.col
            )),
            None => Err(self.eof_error(&format!("{expected:?}"))),
        }
    }

    fn expect_word_any(&mut self) -> Result<String, String> {
        match self.peek() {
            Some((Token::Word(w), _)) => {
                let w = w.clone();
                self.pos += 1;
                Ok(w)
            }
            Some((tok, span)) => Err(format!(
                "parse error: expected a word, found {tok:?} at line {}, col {}",
                span.line, span.col
            )),
            None => Err(self.eof_error("a word")),
        }
    }

    fn expect_word(&mut self, expected: &str) -> Result<Span, String> {
        match self.peek() {
            Some((Token::Word(w), span)) if w == expected => {
                let span = *span;
                self.pos += 1;
                Ok(span)
            }
            Some((tok, span)) => Err(format!(
                "parse error: expected `{expected}`, found {tok:?} at line {}, col {}",
                span.line, span.col
            )),
            None => Err(self.eof_error(&format!("`{expected}`"))),
        }
    }

    fn parse_worddef(&mut self) -> Result<WordDef, String> {
        self.expect_word(":")?;
        let (name, name_span) = self.expect_word_any_spanned()?;
        reject_reserved_name("word", &name, name_span)?;
        if ACCESS_WORDS.contains(&name.as_str()) {
            return Err(shadowed_access_word_error(&name, name_span));
        }
        // Slice 11 (R1): the optional `inline` keyword sits in the one slot
        // between a word's name and its `(`, where nothing else can appear, so
        // it needs no global reservation: the name is consumed above, and
        // `: inline ( -- ) ;` still defines a word *named* `inline`. Only one is
        // consumed, so a second falls through to the `(` below and fails there.
        let declares_inline = matches!(self.peek(), Some((Token::Word(w), _)) if w == "inline");
        if declares_inline {
            self.pos += 1;
        }
        self.expect(Token::LParen)?;
        // R1/R2: a variable-bearing effect (`'T`, `'N`, `..s`) parses into a
        // `PolySig`; every other effect stays a concrete `StackEffect`, byte
        // for byte as before (the whole regression guarantee, R15).
        let (effect, poly) = if self.effect_has_variable() {
            let sig = self.parse_poly_effect()?;
            (StackEffect::default(), Some(Box::new(sig)))
        } else {
            (self.parse_effect()?, None)
        };
        self.expect(Token::RParen)?;
        // D2: the optional trailing `global:` clause, its own keyword-headed
        // clause sitting right after the effect's closing `)` and before the
        // body -- mirrors the `declares_inline` peek above, not a change to
        // `parse_effect`/`parse_poly_effect` themselves.
        let declared_globals = if matches!(self.peek(), Some((Token::Word(w), _)) if w == "global:")
        {
            Some(self.parse_global_clause()?)
        } else {
            None
        };
        let body = self.parse_terms("`;`", |tok| matches!(tok, Token::Semicolon))?;
        self.expect(Token::Semicolon)?;
        Ok(WordDef {
            name,
            effect,
            body,
            poly,
            declares_inline,
            module: self.module,
            span: name_span,
            declared_globals,
        })
    }

    /// Parse one `import:` form (R6, regrammared by P8 slice 1a OQ3):
    /// `import: <target> [<qualifier>] [ | <name>... | ] ;`, plus the wildcard
    /// shape `import: <target> * ;`. The target comes first so the common case
    /// (no renaming) needs no qualifier at all. `self.pos` must point at
    /// `import:`.
    fn parse_import(&mut self) -> Result<Import, String> {
        let span = self.expect_word("import:")?;
        // R9: the target, qualifier, and terminating `;` each fail with a
        // located error naming `import:` and the missing part, not the generic
        // token-level message (which would say `expected a word` or wrongly
        // name the C symbol borrowed from `extern:`).
        let target = match self.peek() {
            Some((Token::Str(s), _)) => {
                let s = s.clone();
                self.pos += 1;
                ImportTarget::Path(s)
            }
            Some((Token::Word(w), wspan)) => {
                let w = w.clone();
                let wspan = *wspan;
                self.pos += 1;
                ImportTarget::Module(parse_module_name(&w, wspan)?)
            }
            _ => {
                return Err(self
                    .form_error("an import target: a module name, or a quoted path in `import:`"))
            }
        };
        let binding = self.parse_import_binding(&target)?;
        match self.peek() {
            Some((Token::Semicolon, _)) => self.pos += 1,
            _ => return Err(self.form_error("`;` terminating `import:`")),
        }
        Ok(Import {
            target,
            binding,
            span,
        })
    }

    /// The part of an `import:` after its target: an optional qualifier, an
    /// optional `| name... |` selective list, or the bare `*` wildcard. The
    /// qualifier is defaulted here rather than left optional, so nothing past
    /// the parser has to know it was elided.
    fn parse_import_binding(&mut self, target: &ImportTarget) -> Result<ImportBinding, String> {
        // A bare `*` in the qualifier position with nothing but `;` after it is
        // the wildcard shape (OQ3). `*` before a `|` is an ordinary qualifier,
        // and `*` inside `| ... |` is the multiplication word being selectively
        // imported, so the two forms never collide.
        if matches!(self.peek(), Some((Token::Word(w), _)) if w == "*")
            && matches!(self.tokens.get(self.pos + 1), Some((Token::Semicolon, _)))
        {
            self.pos += 1;
            return Ok(ImportBinding::Wildcard);
        }
        let qualifier = match self.peek() {
            Some((Token::Word(w), _)) => {
                let w = w.clone();
                self.pos += 1;
                w
            }
            _ => default_qualifier(target).ok_or_else(|| {
                self.form_error("a qualifier in `import:` (the target has no name to default to)")
            })?,
        };
        let mut selective = Vec::new();
        if matches!(self.peek(), Some((Token::Pipe, _))) {
            self.expect(Token::Pipe)?;
            while let Some((Token::Word(w), wspan)) = self.peek() {
                let name = w.clone();
                let wspan = *wspan;
                self.pos += 1;
                selective.push((name, wspan));
            }
            self.expect(Token::Pipe)?;
        }
        Ok(ImportBinding::Qualified {
            qualifier,
            selective,
        })
    }

    /// A located parse error for a malformed `import:`/`export:` form (R9),
    /// naming the construct and what it expected. Reads the current token
    /// without advancing; on end of input it defers to `eof_error`.
    fn form_error(&self, expected: &str) -> String {
        match self.peek() {
            Some((tok, span)) => format!(
                "parse error: expected {expected}, found {tok:?} at line {}, col {}",
                span.line, span.col
            ),
            None => self.eof_error(expected),
        }
    }

    /// Parse one `export:` form (R7): `export: <name>... ;`. Returns the named
    /// words/types with their spans; the list is recorded now and enforced in
    /// phase 2. `self.pos` must point at `export:`.
    fn parse_export(&mut self) -> Result<Vec<(String, Span)>, String> {
        self.expect_word("export:")?;
        let mut names = Vec::new();
        while let Some((Token::Word(w), wspan)) = self.peek() {
            let name = w.clone();
            let wspan = *wspan;
            self.pos += 1;
            names.push((name, wspan));
        }
        // R9: a stray non-word token before `;` is a located error naming
        // `export:`, not the generic `expected Semicolon`.
        match self.peek() {
            Some((Token::Semicolon, _)) => self.pos += 1,
            _ => return Err(self.form_error("`;` terminating `export:`")),
        }
        Ok(names)
    }

    /// `extern:` declaration (R1): a top-level foreign-call binding. Grammar
    /// mirrors `worddef` except the body is a single explicit C symbol
    /// string rather than terms — a symbol string rather than the word name
    /// reused, since a Sooth name may use characters C cannot (`^|>`), and
    /// binding a C name like `open` to a differently-spelled Sooth word must
    /// be possible.
    fn parse_extern_decl(&mut self) -> Result<ExternDecl, String> {
        let span = self.expect_word("extern:")?;
        let (name, name_span) = self.expect_word_any_spanned()?;
        reject_reserved_name("word", &name, name_span)?;
        if ACCESS_WORDS.contains(&name.as_str()) {
            return Err(shadowed_access_word_error(&name, name_span));
        }
        self.expect(Token::LParen)?;
        let effect = self.parse_effect()?;
        self.expect(Token::RParen)?;
        let (symbol, symbol_span) = self.expect_str_literal()?;
        reject_invalid_c_symbol(&symbol, symbol_span)?;
        self.expect(Token::Semicolon)?;
        Ok(ExternDecl {
            name,
            symbol,
            effect,
            span,
            module: self.module,
        })
    }

    /// P7.S3e (R1/R3, decision 1): `trait: TraitName 'T member ( &'T ... --
    /// ... ) member2 ( ... ) ... ;` -- a trait name, its single (implicit)
    /// type variable header, then one or more member signatures over that
    /// variable. Single-type-variable traits only (R16): a second header
    /// variable is a located error here; a member signature introducing a
    /// variable other than the header's is rejected once its own effect is
    /// fully parsed (`parse_trait_member_effect`).
    fn parse_trait_decl(&mut self) -> Result<TraitDecl, String> {
        let span = self.expect_word("trait:")?;
        let (name, name_span) = self.expect_word_any_spanned()?;
        reject_reserved_name("trait", &name, name_span)?;
        let (ty_var, ty_var_span) = match self.peek() {
            Some((Token::Word(w), s)) if w.starts_with('\'') => {
                let (w, s) = (w.clone(), *s);
                self.pos += 1;
                (w, s)
            }
            Some((tok, s)) => {
                return Err(format!(
                    "parse error: expected a type variable (`'T`) after `trait: {name}`, found {tok:?} at line {}, col {}",
                    s.line, s.col
                ));
            }
            None => return Err(self.eof_error("a type variable (`'T`)")),
        };
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('\'')) {
            return Err(multi_variable_trait_error(&name, ty_var_span));
        }
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some((Token::Word(_), _)) => {
                    let (member_name, member_span) = self.expect_word_any_spanned()?;
                    // P7.S3r (R4): a member becomes a word when implemented, so
                    // it inherits `parse_worddef`'s reserved-name policy, plus a
                    // rejection of every name dispatched ahead of the word
                    // environment (which an impl body's own member name would
                    // shadow inside that body).
                    reject_reserved_name("word", &member_name, member_span)?;
                    if ACCESS_WORDS.contains(&member_name.as_str()) {
                        return Err(shadowed_access_word_error(&member_name, member_span));
                    }
                    if is_name_dispatched_builtin(&member_name) {
                        return Err(builtin_named_trait_member_error(
                            &name,
                            &member_name,
                            member_span,
                        ));
                    }
                    self.expect(Token::LParen)?;
                    let sig = self.parse_trait_member_effect(&ty_var, &name, member_span)?;
                    self.expect(Token::RParen)?;
                    members.push(TraitMember {
                        name: member_name,
                        sig,
                    });
                }
                Some((tok, s)) => {
                    return Err(format!(
                        "parse error: expected a member name or `;`, found {tok:?} at line {}, col {}",
                        s.line, s.col
                    ));
                }
                None => return Err(self.eof_error("`;` (unterminated `trait:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        if members.is_empty() {
            return Err(trait_zero_members_error(&name, span));
        }
        Ok(TraitDecl {
            name,
            kind: TraitKind::Nominal,
            members,
            module: self.module,
            span: name_span,
        })
    }

    /// One trait member's signature, positioned just past its opening `(`:
    /// an ordinary poly effect, except the trait's own type variable is
    /// pre-interned at id 0 (its binding occurrence is the trait header, not
    /// here), so every `'`-mention inside the member is a *use*. A member
    /// mentioning a second, genuinely new variable name is rejected here
    /// (R16, single-type-variable traits only), once the whole effect --
    /// including a use *before* the point a hypothetical second binding
    /// would occur -- has been parsed.
    fn parse_trait_member_effect(
        &mut self,
        ty_var: &str,
        trait_name: &str,
        member_span: Span,
    ) -> Result<PolySig, String> {
        let mut builder = PolyBuilder::default();
        builder.intern_ty_var(ty_var, member_span)?;
        let raw_in = self.parse_poly_slots(&mut builder, false, |tok| {
            matches!(tok, Token::RParen) || is_word(tok, "--")
        })?;
        self.expect_word("--")?;
        let raw_out =
            self.parse_poly_slots(&mut builder, true, |tok| matches!(tok, Token::RParen))?;
        builder.validate_pending_quotation_rows()?;
        let inputs = raw_in
            .into_iter()
            .map(|r| self.raw_to_poly_type(r))
            .collect::<Result<_, _>>()?;
        let outputs = raw_out
            .into_iter()
            .map(|r| self.raw_to_poly_type(r))
            .collect::<Result<_, _>>()?;
        let sig = builder.finish(inputs, outputs);
        if sig.ty_var_names.len() > 1 {
            return Err(multi_variable_trait_error(trait_name, member_span));
        }
        for t in sig.inputs.iter().chain(&sig.outputs) {
            if !member_shape_is_supported(t) {
                return Err(unsupported_trait_member_shape_error(
                    trait_name,
                    member_span,
                ));
            }
        }
        if let Some(row) = sig.row_in.or(sig.row_out) {
            return Err(row_typed_trait_member_error(
                trait_name,
                &sig.row_var_names[row as usize],
                member_span,
            ));
        }
        Ok(sig)
    }

    /// P7.S3e (R4/R11, decision 1) / P7.S3r (R1): `impl: Trait for Type ... ;`.
    /// `Trait` resolves against the whole-program trait registry (module-aware,
    /// mirroring a qualified type name); `Type` resolves exactly as any other
    /// type expression does. Orphan-rule/missing-member validation is a
    /// check-time concern (`check::check_impl_decls`), not here: by the time
    /// that check runs the whole program's `traits`/`impls`/`structs` are fully
    /// assembled, regardless of this file's own declaration order.
    ///
    /// The body is a sequence of `: member ... ;` members (R1/R5), each
    /// desugared to a synthesized top-level `WordDef` returned alongside the
    /// decl; the decl itself carries only the `(member, synth-name)` pairs
    /// `check_impl_decls` resolves.
    fn parse_impl_decl(&mut self) -> Result<(ImplDecl, Vec<WordDef>), String> {
        let span = self.expect_word("impl:")?;
        let (trait_name, trait_span) = self.expect_word_any_spanned()?;
        self.expect_word("for")?;
        let target_ty = self.parse_type_expr()?;
        let trait_id = find_trait_in_module(
            self.traits,
            &trait_name,
            self.module,
            self.imports,
            self.selective,
        )
        .ok_or_else(|| unknown_trait_error(&trait_name, trait_span))?;
        if let TraitKind::Predicate(_) = self.traits[trait_id.index()].kind {
            return Err(impl_for_predicate_trait_error(&trait_name, trait_span));
        }
        if let Some((qualifier, base)) = trait_name.split_once("::") {
            if !self.type_is_exported(qualifier, base) {
                return Err(not_exported_error(base, qualifier, trait_span));
            }
        }
        let mut bindings = Vec::new();
        let mut words = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some(_) => {
                    let (member_name, word) = self.parse_impl_member_body(trait_id, target_ty)?;
                    bindings.push((member_name, word.name.clone()));
                    words.push(word);
                }
                None => return Err(self.eof_error("`;` (unterminated `impl:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        if bindings.is_empty() {
            return Err(impl_zero_bindings_error(&trait_name, span));
        }
        Ok((
            ImplDecl {
                trait_id,
                target_ty,
                module: self.module,
                span,
                bindings,
                resolved: Vec::new(),
            },
            words,
        ))
    }

    /// P7.S3r (R2/R4a/R5/R6): one `: member [| binders |] body ;` inside an
    /// `impl:` block, desugared to the top-level word the member binds to. The
    /// declared effect is the trait member's signature grounded at the `for`
    /// type through `ast`'s `ground_member_type`; there is no `(` to parse,
    /// since restating the inherited signature is rejected.
    fn parse_impl_member_body(
        &mut self,
        trait_id: TraitId,
        target_ty: Type,
    ) -> Result<(String, WordDef), String> {
        self.expect_word(":")?;
        let (member_name, member_span) = self.expect_word_any_spanned()?;
        let trait_name = self.traits[trait_id.index()].name.clone();
        let trait_module = self.traits[trait_id.index()].module;
        let Some(sig) = self.traits[trait_id.index()]
            .members
            .iter()
            .find(|m| m.name == member_name)
            .map(|m| m.sig.clone())
        else {
            return Err(impl_non_member_body_error(
                &member_name,
                &trait_name,
                member_span,
            ));
        };
        if let Some((Token::LParen, s)) = self.peek() {
            return Err(impl_member_restated_signature_error(
                &member_name,
                &trait_name,
                *s,
            ));
        }
        let ground = |slots: &[PolyType], arrays: &mut Vec<ArrayDecl>, refs: &mut Vec<RefDecl>| {
            slots
                .iter()
                .map(|t| TypedSlot {
                    name: None,
                    ty: ground_member_type(t, target_ty, arrays, refs),
                })
                .collect()
        };
        let effect = StackEffect {
            inputs: ground(&sig.inputs, self.arrays, self.refs),
            outputs: ground(&sig.outputs, self.arrays, self.refs),
        };
        let body = self.parse_terms("`;`", |tok| matches!(tok, Token::Semicolon))?;
        self.expect(Token::Semicolon)?;
        let name = synth_member_word_name(&member_name, &trait_name, trait_module, target_ty);
        let body = rewrite_member_self_calls(&body, &member_name, &name)?;
        Ok((
            member_name,
            WordDef {
                name,
                effect,
                body,
                poly: None,
                declares_inline: false,
                module: self.module,
                span: member_span,
                declared_globals: None,
            },
        ))
    }

    /// `static:` declaration (D1): a module-level place, scalar-only this
    /// slice. Grammar mirrors `extern:`'s shape (a keyword, a name, then the
    /// declared shape) but with a type instead of an effect and an optional
    /// `= literal` initialiser instead of a mandatory C symbol.
    fn parse_static_decl(&mut self) -> Result<StaticDecl, String> {
        self.expect_word("static:")?;
        let (name, name_span) = self.expect_word_any_spanned()?;
        reject_reserved_name("static", &name, name_span)?;
        if ACCESS_WORDS.contains(&name.as_str()) {
            return Err(shadowed_access_word_error(&name, name_span));
        }
        let (ty_name, ty_span) = self.expect_word_any_spanned()?;
        // D1/OQ1: an allow-list of the fixed scalar keyword set, not a
        // struct-detection check -- the parser is single-pass with no type
        // table at declaration-parse time (a `type:` may follow a `static:`
        // in the same file), so a genuine struct type and a mistyped or
        // forward-referenced user type both fall through to the same "not a
        // scalar" error here.
        //
        // P7 slice 3i (R1): `bool` is the one entry that is not a keyword. It
        // is `core::bool`'s enum, so it resolves through the ordinary
        // type-name path here -- and so a boolean static requires the
        // enclosing module to import `core::bool`, exactly as naming `bool` in
        // an effect does. Without the import this is a located `unknown type
        // bool` at the annotation and the initializer is never read.
        let ty = match ty_name.as_str() {
            "i64" => Type::I64,
            "u32" => Type::U32,
            "str" => Type::Str,
            "Bool" => self.resolve_type(&ty_name, ty_span)?,
            _ => return Err(static_scalar_type_error(&name, &ty_name, ty_span)),
        };
        let init = if matches!(self.peek(), Some((Token::Word(w), _)) if w == "=") {
            self.pos += 1;
            self.parse_static_init(ty)?
        } else {
            StaticInit::Zero
        };
        self.expect(Token::Semicolon)?;
        Ok(StaticDecl {
            name,
            ty,
            init,
            module: self.module,
            // The name, not the `static:` keyword, matching `WordDef.span`:
            // Phase 3's duplicate-declaration error points at the name it names.
            span: name_span,
        })
    }

    /// D1/D3: the `= literal` initialiser, one literal matching the static's
    /// declared scalar type -- no arithmetic, no reference to another
    /// static, no struct-literal aggregate.
    ///
    /// The boolean arm keys on `ty` being *an enum*, which the allow-list in
    /// `parse_static_decl` only ever admits through its `bool` entry: an
    /// enum's variant payloads are not filled in until after declaration
    /// parsing, so this is the sharpest test available here, and
    /// `check_static_decls` re-tests the resolved shape once they are.
    fn parse_static_init(&mut self, ty: Type) -> Result<StaticInit, String> {
        match self.peek() {
            Some((Token::Int(n), _)) if ty == Type::I64 => {
                let n = *n;
                self.pos += 1;
                Ok(StaticInit::Int(n))
            }
            Some((Token::Int(n), span)) if ty == Type::U32 => {
                if !(0..=i64::from(u32::MAX)).contains(n) {
                    return Err(static_u32_init_range_error(*n, *span));
                }
                let n = *n;
                self.pos += 1;
                Ok(StaticInit::Int(n))
            }
            Some((Token::Str(s), _)) if ty == Type::Str => {
                let s = s.clone();
                self.pos += 1;
                Ok(StaticInit::Str(s))
            }
            Some((Token::Word(w), _))
                if matches!(ty, Type::Enum(..)) && (w == "True" || w == "False") =>
            {
                let b = w == "True";
                self.pos += 1;
                Ok(StaticInit::Bool(b))
            }
            Some((tok, span)) => Err(format!(
                "parse error: expected a literal matching the static's declared type `{}`, found {tok:?} at line {}, col {}",
                ty.name(),
                span.line,
                span.col
            )),
            None => Err(self.eof_error("a literal initializer")),
        }
    }

    /// D2: a word's trailing `global:` clause, right after the effect's
    /// closing `)` and before the body -- its own keyword-headed clause, not
    /// nested inside the stack-effect parens. `self.pos` must point at
    /// `global:`.
    fn parse_global_clause(&mut self) -> Result<Vec<GlobalEntry>, String> {
        self.expect_word("global:")?;
        let mut entries = Vec::new();
        loop {
            let (name, name_span) = self.expect_word_any_spanned()?;
            let (mode_word, mode_span) = self.expect_word_any_spanned()?;
            let (mode_str, glued_comma) = match mode_word.strip_suffix(',') {
                Some(m) => (m, true),
                None => (mode_word.as_str(), false),
            };
            let mode = match mode_str {
                "r" => GlobalMode::R,
                "w" => GlobalMode::W,
                _ => return Err(invalid_global_mode_error(&mode_word, mode_span)),
            };
            entries.push(GlobalEntry {
                name,
                mode,
                span: name_span,
            });
            if glued_comma {
                continue;
            }
            if matches!(self.peek(), Some((Token::Word(w), _)) if w == ",") {
                self.pos += 1;
                continue;
            }
            // A body opening `NAME r`/`NAME w` is a dropped separator far more
            // often than it is two real terms, and letting it through defers
            // the report to an unknown-word error on the *next* line.
            if let (Some((Token::Word(name), span)), Some((Token::Word(mode), _))) =
                (self.tokens.get(self.pos), self.tokens.get(self.pos + 1))
            {
                let mode = mode.strip_suffix(',').unwrap_or(mode);
                if mode == "r" || mode == "w" {
                    return Err(missing_global_comma_error(name, *span));
                }
            }
            break;
        }
        Ok(entries)
    }

    /// The `extern:` declaration's C-symbol string literal (R1): an explicit
    /// `"..."`, not a bare word, so the checker never has to guess whether a
    /// word-shaped token is the symbol or a stray extra token.
    fn expect_str_literal(&mut self) -> Result<(String, Span), String> {
        match self.peek() {
            Some((Token::Str(s), span)) => {
                let s = s.clone();
                let span = *span;
                self.pos += 1;
                Ok((s, span))
            }
            Some((tok, span)) => Err(format!(
                "parse error: expected a string literal naming the C symbol, found {tok:?} at line {}, col {}",
                span.line, span.col
            )),
            None => Err(self.eof_error("a string literal naming the C symbol")),
        }
    }

    fn parse_effect(&mut self) -> Result<StackEffect, String> {
        let inputs = self.parse_slots(|tok| matches!(tok, Token::RParen) || is_word(tok, "--"))?;
        self.expect_word("--")?;
        let outputs = self.parse_slots(|tok| matches!(tok, Token::RParen))?;
        Ok(StackEffect { inputs, outputs })
    }

    /// R1: whether the effect the parser is positioned at (just after `(`)
    /// mentions any variable form, scanning to the matching `)`. `'` and `..`
    /// are not lexer delimiters, so each form arrives as one `Word` token; a
    /// `'`-led word is a type/length variable and a `..`-led word is the row
    /// variable. A no-variable effect takes the concrete path untouched (R2).
    /// Slice 10a (R1): a `~[` token also routes to the poly parser -- a `~`
    /// is poly-forced even when its effect is otherwise fully concrete, since
    /// `WordDef.poly = Some(..)` is what R9 context 4's unreachability
    /// depends on.
    /// Slice 13 (R-A3, review fix): a glued `&'T`/`&!'T` is one `Word` token
    /// starting with `&`, not `'`, so it was missed by this pre-scan and the
    /// whole effect took the concrete path -- where `parse_ref_type_expr`
    /// then resolves `'T` as an (unknown) concrete type name. A glued
    /// referent must be recognized here too, or `parse_poly_slot`'s glued
    /// branch (R-A3) is only ever reached when some other slot in the same
    /// effect is independently variable-bearing.
    fn effect_has_variable(&self) -> bool {
        let mut i = self.pos;
        while let Some((tok, _)) = self.tokens.get(i) {
            match tok {
                Token::RParen => return false,
                Token::TildeLBracket => return true,
                Token::Word(w) if w.starts_with('\'') || w.starts_with("..") => return true,
                Token::Word(w)
                    if w.strip_prefix('&')
                        .map(|r| r.strip_prefix('!').unwrap_or(r))
                        .is_some_and(|r| r.starts_with('\'')) =>
                {
                    return true;
                }
                // P7.S3n (R3): the identical miss for a glued `^'T`/`^^'T`,
                // one `Word` token starting with `^` rather than `'`.
                Token::Word(w)
                    if w.starts_with('^') && w.trim_start_matches('^').starts_with('\'') =>
                {
                    return true;
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// R1/R3: parse a variable-bearing effect into a `PolySig`. Runs the
    /// binding-occurrence analysis (X1/X3) and the row-variable placement rule
    /// (X2) as it goes, left-to-right, inputs then outputs, so the first
    /// (leftmost, deepest-first) mention of a `'`-name is its binding
    /// occurrence and every later one a use.
    fn parse_poly_effect(&mut self) -> Result<PolySig, String> {
        let mut builder = PolyBuilder::default();
        let raw_in = self.parse_poly_slots(&mut builder, false, |tok| {
            matches!(tok, Token::RParen) || is_word(tok, "--")
        })?;
        self.expect_word("--")?;
        let raw_out =
            self.parse_poly_slots(&mut builder, true, |tok| matches!(tok, Token::RParen))?;
        // Slice 10c (R-P2-1): the whole signature is known now -- resolve
        // every quotation effect's deferred output-side row mention against
        // it (a mention that turned out to name neither top-level row is a
        // located error here, not at the point it was first seen).
        builder.validate_pending_quotation_rows()?;
        let inputs = raw_in
            .into_iter()
            .map(|r| self.raw_to_poly_type(r))
            .collect::<Result<_, _>>()?;
        let outputs = raw_out
            .into_iter()
            .map(|r| self.raw_to_poly_type(r))
            .collect::<Result<_, _>>()?;
        Ok(builder.finish(inputs, outputs))
    }

    /// Parse one side's slots into `RawTy`s, interning every variable into
    /// `builder`. A leading `..s` (deepest slot) is the side's row variable;
    /// a `..s` anywhere else, or a second one, is X2.
    fn parse_poly_slots(
        &mut self,
        builder: &mut PolyBuilder,
        is_output: bool,
        stop: impl Fn(&Token) -> bool,
    ) -> Result<Vec<RawTy>, String> {
        if let Some((Token::Word(w), span)) = self.peek() {
            if w.starts_with("..") && !is_word(&Token::Word(w.clone()), "--") {
                let (w, span) = (w.clone(), *span);
                self.pos += 1;
                builder.set_row(is_output, w, span)?;
            }
        }
        let mut slots = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error("`)` or `--`")),
                Some((tok, _)) if stop(tok) => break,
                Some((Token::Word(w), span)) if w.starts_with("..") => {
                    return Err(row_var_misplaced_error(w, *span));
                }
                _ => slots.push(self.parse_poly_slot(builder, is_output)?),
            }
        }
        Ok(slots)
    }

    /// One polymorphic type slot: an array (whose element and/or count may be a
    /// variable), a type variable (with an optional bound at its binding
    /// occurrence), or a plain concrete type expression. `word_is_output` is
    /// which top-level side of the *enclosing word's* signature this slot sits
    /// on -- threaded down to a nested quotation effect, since Slice 10c
    /// (R-P2-2) only lifts the same-row rule for a quotation on the word's
    /// input side (a parameter), never its output side (R-P2-5).
    fn parse_poly_slot(
        &mut self,
        builder: &mut PolyBuilder,
        word_is_output: bool,
    ) -> Result<RawTy, String> {
        // Slice 10a (R1): `~[` has already consumed the opening bracket as
        // one token, so its entry point skips straight to the inner parse
        // rather than going through `parse_poly_quotation`'s own
        // `expect(LBracket)`.
        if matches!(self.peek(), Some((Token::TildeLBracket, _))) {
            self.pos += 1;
            return self.parse_poly_quotation_inner(builder, true, word_is_output);
        }
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            if self.quotation_type_ahead() {
                return self.parse_poly_quotation(builder, word_is_output);
            }
            return self.parse_poly_array(builder, word_is_output);
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('\'')) {
            let (w, span) = self.expect_word_any_spanned()?;
            return self.parse_poly_ty_var(builder, &w, span);
        }
        // Slice 13 (R-A3): a `&`-led slot, intercepted *before* the
        // `parse_type_expr` fallthrough -- which resolves a reference's
        // referent concretely, so `&'T` would die on `'T` as an unknown type.
        // Only the two poly-relevant shapes are taken here; a glued concrete
        // referent (`&Foo`, `&!^List`) still falls through to
        // `parse_ref_type_expr` and folds to `Concrete`.
        if let Some((Token::Word(w), span)) = self.peek() {
            let (w, span) = (w.clone(), *span);
            if w.starts_with('&') {
                let sigil_len = if w.starts_with("&!") { 2 } else { 1 };
                let mutable = sigil_len == 2;
                let remainder = &w[sigil_len..];
                // Bare sigil (`& 'T`, `&['T 4]`): the referent is a genuine
                // following token, so recurse into it as a poly slot.
                if remainder.is_empty() {
                    self.pos += 1;
                    if matches!(self.peek(), Some((Token::Word(n), _)) if n == "--")
                        || self.peek().is_none()
                    {
                        return Err(ref_no_referent_error(&w, span));
                    }
                    let inner = self.parse_poly_slot(builder, word_is_output)?;
                    return Ok(RawTy::Ref(Box::new(inner), mutable));
                }
                // Glued sigil+variable (`&'T`, `&!'T: Copy`): the referent is
                // a substring, not a token, so the variable is interned from
                // the remainder rather than recursed on.
                if remainder.starts_with('\'') {
                    let remainder = remainder.to_string();
                    let remainder_span = Span {
                        col: span.col + sigil_len as u32,
                        ..span
                    };
                    self.pos += 1;
                    let inner = self.parse_poly_ty_var(builder, &remainder, remainder_span)?;
                    return Ok(RawTy::Ref(Box::new(inner), mutable));
                }
            }
        }
        // P7.S3n (R3): a `^`-led slot, intercepted before the
        // `parse_type_expr` fallthrough for the same reason the `&` arm above
        // is -- that path resolves a cell's payload concretely, so `^'T`
        // would die on `'T` as an unknown type. Only the two poly-relevant
        // shapes are taken here; a glued concrete payload (`^Foo`) still
        // falls through to `parse_owning_cell_type_expr` and folds to
        // `Concrete`.
        if let Some((Token::Word(w), span)) = self.peek() {
            let (w, span) = (w.clone(), *span);
            if w.starts_with('^') {
                let run_len = w.chars().take_while(|&c| c == '^').count();
                let remainder = &w[run_len..];
                // Bare run (`^ 'T`, `^['T 4]`): the payload is a genuine
                // following token, so recurse into it as a poly slot.
                let inner = if remainder.is_empty() {
                    self.pos += 1;
                    if matches!(self.peek(), Some((Token::Word(n), _)) if n == "--")
                        || self.peek().is_none()
                    {
                        return Err(owned_cell_no_payload_error(&w, span));
                    }
                    Some(self.parse_poly_slot(builder, word_is_output)?)
                } else if remainder.starts_with('\'') {
                    // Glued run+variable (`^'T`, `^^'T: Copy`): the variable
                    // is a substring, not a token, so it is interned from the
                    // remainder rather than recursed on.
                    let remainder = remainder.to_string();
                    let remainder_span = Span {
                        col: span.col + run_len as u32,
                        ..span
                    };
                    self.pos += 1;
                    Some(self.parse_poly_ty_var(builder, &remainder, remainder_span)?)
                } else {
                    None
                };
                if let Some(mut inner) = inner {
                    for _ in 0..run_len {
                        inner = RawTy::OwnedCell(Box::new(inner));
                    }
                    return Ok(inner);
                }
            }
        }
        // P7 slice 3a (R1): a generic type applied to poly slots
        // (`Result['T 'E]`, `Box['T]`), intercepted ahead of the
        // `parse_type_expr` fallthrough below -- which resolves arguments
        // concretely only, so `'T` would die there as an unknown type. A
        // word naming a generic header immediately followed by `[` takes
        // this arm; a bare header with no following `[`, or a name that
        // names no generic header at all, falls through unchanged (the
        // former is `parse_type_expr`'s arity error to report).
        if let Some((Token::Word(w), span)) = self.peek() {
            if !w.starts_with('\'') && !w.starts_with('&') && !w.starts_with('^') {
                let (w, span) = (w.clone(), *span);
                if matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _))) {
                    if let Some((is_enum, idx, module)) = self.poly_generic_header(&w, span)? {
                        self.pos += 1;
                        return self.parse_poly_generic_application(
                            builder,
                            word_is_output,
                            &w,
                            is_enum,
                            idx,
                            module,
                            span,
                        );
                    }
                }
            }
        }
        let ty = self.parse_type_expr()?;
        Ok(RawTy::Concrete(ty))
    }

    /// P7 slice 3a (R1): a generic-type application's bracketed argument
    /// list, each argument a poly slot rather than a concrete type
    /// expression -- the poly-slot twin of `parse_type_arguments`, reusing
    /// only its arity check, never its concrete-only argument parser.
    #[allow(clippy::too_many_arguments)]
    fn parse_poly_generic_application(
        &mut self,
        builder: &mut PolyBuilder,
        word_is_output: bool,
        name: &str,
        is_enum: bool,
        idx: usize,
        module: u32,
        span: Span,
    ) -> Result<RawTy, String> {
        let arity = if is_enum {
            self.generics.enums[idx].ty_var_names.len()
        } else {
            self.generics.structs[idx].ty_var_names.len()
        };
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            return Err(generic_arity_error(name, arity, 0, span));
        }
        self.pos += 1;
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated generic type application)"));
                }
                _ => args.push(self.parse_poly_slot(builder, word_is_output)?),
            }
        }
        if args.len() != arity {
            return Err(generic_arity_error(name, arity, args.len(), span));
        }
        Ok(RawTy::Generic {
            is_enum,
            idx,
            module,
            args,
            name: name.to_string(),
            span,
        })
    }

    /// One type-variable slot, already lexed: `'T`, with an optional bound at
    /// its binding occurrence (`'T: Copy`, glued or spaced). Shared by the
    /// bare slot arm and Slice 13's glued `&'T`, so a bound behind a sigil
    /// binds exactly as it does without one.
    fn parse_poly_ty_var(
        &mut self,
        builder: &mut PolyBuilder,
        word: &str,
        span: Span,
    ) -> Result<RawTy, String> {
        let glued_colon = word.ends_with(':') && word.len() > 1;
        let name = if glued_colon {
            word[..word.len() - 1].to_string()
        } else {
            word.to_string()
        };
        let bound_follows =
            glued_colon || matches!(self.peek(), Some((Token::Word(c), _)) if c == ":");
        if bound_follows && !glued_colon {
            self.pos += 1; // the standalone `:`
        }
        let bounds = if bound_follows {
            Some(self.parse_capabilities(span)?)
        } else {
            None
        };
        let (id, is_binding) = builder.intern_ty_var(&name, span)?;
        if let Some(bounds) = bounds {
            if !is_binding {
                return Err(bound_on_use_error(&name, span));
            }
            for b in bounds {
                builder.bounds.push((id, b));
            }
        }
        Ok(RawTy::Var(id))
    }

    /// Slice 6a (R2/R5): a polymorphic quotation effect `[ <in> -- <out> ]`
    /// whose rows recurse through `parse_poly_slot`, so a `'T` element variable
    /// is interned into `builder` exactly as it is in an ordinary slot.
    fn parse_poly_quotation(
        &mut self,
        builder: &mut PolyBuilder,
        word_is_output: bool,
    ) -> Result<RawTy, String> {
        self.expect(Token::LBracket)?;
        self.parse_poly_quotation_inner(builder, false, word_is_output)
    }

    /// The body of a polymorphic quotation effect, positioned just past its
    /// opening bracket -- which `parse_poly_quotation` consumes as a plain
    /// `Token::LBracket` and `parse_poly_slot`'s `~[` arm consumes as part of
    /// `Token::TildeLBracket` (Slice 10a R1). Split out so the token that
    /// already ate the bracket has somewhere to resume.
    #[allow(clippy::type_complexity)]
    fn parse_poly_quotation_inner(
        &mut self,
        builder: &mut PolyBuilder,
        is_inline: bool,
        word_is_output: bool,
    ) -> Result<RawTy, String> {
        let (inputs, row_in, row_in_span) =
            self.parse_poly_quot_list(builder, true, word_is_output)?;
        self.expect_word("--")?;
        let (outputs, row_out, row_out_span) =
            self.parse_poly_quot_list(builder, false, word_is_output)?;
        self.expect(Token::RBracket)?;
        // R5: both sides or neither. For 10a's loop-body shape (a back-edge
        // fixed point) the row must be the same on both sides; Slice 10c
        // (R-P2-2) lifts that for an *input-side* quotation parameter of a
        // quotation-taking (always-inlined) word, whose shape change is
        // splice-local (INV-INLINE-COMBINATOR) and never rides a back-edge.
        // An output-side quotation (R-P2-5, a word *returning* an inline
        // quotation) keeps the same-row rule: it is not a parameter, so the
        // splice-local justification does not apply.
        let shape_change_lifted = is_inline && !word_is_output;
        match (row_in, row_out) {
            (Some(a), Some(b)) if a != b && !shape_change_lifted => {
                return Err(quotation_row_shape_change_error(
                    &builder.row_names[a as usize],
                    &builder.row_names[b as usize],
                ));
            }
            (Some(_), None) => {
                let (name, span) = row_in_span.expect("row_in set implies a span");
                return Err(quotation_row_one_sided_error(&name, span));
            }
            (None, Some(_)) => {
                let (name, span) = row_out_span.expect("row_out set implies a span");
                return Err(quotation_row_one_sided_error(&name, span));
            }
            (Some(_), Some(_)) if !is_inline => {
                let (name, span) = row_in_span.expect("row_in set implies a span");
                return Err(quotation_row_requires_inline_error(&name, span));
            }
            _ => {}
        }
        Ok(RawTy::Quotation(
            inputs, outputs, is_inline, row_in, row_out,
        ))
    }

    /// One side of a polymorphic quotation effect, stopping on the top-depth
    /// `--` (inputs) or `]` (outputs). A leading `..`-prefixed name is R4's
    /// row mention: it must already denote the signature's own top-level
    /// row, and its name/span are returned alongside so the caller can
    /// render R5's one-sided-row error.
    #[allow(clippy::type_complexity)]
    fn parse_poly_quot_list(
        &mut self,
        builder: &mut PolyBuilder,
        stop_on_arrow: bool,
        word_is_output: bool,
    ) -> Result<(Vec<RawTy>, Option<u32>, Option<(String, Span)>), String> {
        let mut row = None;
        let mut row_span = None;
        if let Some((Token::Word(w), span)) = self.peek() {
            if w.starts_with("..") {
                let (w, span) = (w.clone(), *span);
                self.pos += 1;
                // Slice 10c (R-P2-1): the quotation's own *input* side must
                // already denote a known row (strict, immediate); its *output*
                // side may forward-reference the signature's own top-level
                // output row, named only later (deferred, validated once the
                // whole signature is parsed).
                row = Some(if stop_on_arrow {
                    builder.quotation_row_id(&w, span)?
                } else {
                    builder.quotation_row_id_deferred(&w, span)
                });
                row_span = Some((w, span));
            }
        }
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error(if stop_on_arrow { "`--`" } else { "`]`" })),
                Some((Token::Word(w), _)) if stop_on_arrow && w == "--" => break,
                Some((Token::RBracket, _)) if !stop_on_arrow => break,
                Some((Token::Word(w), span)) if w.starts_with("..") => {
                    return Err(row_var_misplaced_error(w, *span));
                }
                _ => out.push(self.parse_poly_slot(builder, word_is_output)?),
            }
        }
        Ok((out, row, row_span))
    }

    /// A polymorphic array `[ elem count ]`: `elem` recurses (so `['T 'N]`
    /// nests a variable element), `count` is a decimal literal or a length
    /// variable `'N`.
    fn parse_poly_array(
        &mut self,
        builder: &mut PolyBuilder,
        word_is_output: bool,
    ) -> Result<RawTy, String> {
        self.expect(Token::LBracket)?;
        let elem = self.parse_poly_slot(builder, word_is_output)?;
        let count = match self.peek().cloned() {
            Some((Token::Word(w), span)) if w.starts_with('\'') => {
                self.pos += 1;
                let id = builder.intern_len_var(&w, span)?;
                RawLen::Var(id)
            }
            Some((Token::Int(n), _)) if (1..=i64::from(u32::MAX)).contains(&n) => {
                self.pos += 1;
                RawLen::Concrete(n as u32)
            }
            Some((Token::Int(n), span)) => {
                self.pos += 1;
                return Err(format!(
                    "error: array type has invalid length {n} at line {}, col {} (`[T N]` requires 1 <= N <= {})",
                    span.line, span.col, u32::MAX
                ));
            }
            Some((tok, span)) => {
                return Err(format!(
                    "error: array count must be a decimal literal or a length variable `'N`, found `{}` at line {}, col {}",
                    describe_token(&tok), span.line, span.col
                ));
            }
            None => return Err(self.eof_error("an array count literal or `'N`")),
        };
        self.expect(Token::RBracket)?;
        Ok(RawTy::Array(Box::new(elem), count))
    }

    /// The capability list after a bound colon (`'T: Copy Ord`): at least one
    /// capability, then greedily every following capability word. The first
    /// non-capability word after the colon is X3 (unknown capability), since
    /// the colon has already committed to a bound.
    ///
    /// P7.S3e (R2): a single trait-table lookup replaces the two hardcoded
    /// string compares -- `Copy`/`Ord` are pre-seeded `Predicate`-kind
    /// entries (`seed_predicate_traits`), so a user `trait: Copy` collides
    /// with them as an ordinary duplicate declaration (`check_trait_decls`),
    /// not a bespoke reserved-word check. A `Nominal` (user-declared) trait
    /// name resolves through the same table (R18, `bound_trait_id`) and
    /// yields `Bound::User`.
    fn parse_capabilities(&mut self, colon_span: Span) -> Result<Vec<Bound>, String> {
        let mut out = Vec::new();
        while let Some((Token::Word(c), span)) = self.peek() {
            let (c, span) = (c.clone(), *span);
            if let Some(bound) = predicate_bound(self.traits, &c) {
                self.pos += 1;
                out.push(bound);
                continue;
            }
            match self.bound_trait_id(&c, span, out.is_empty())? {
                Some(id) => {
                    self.pos += 1;
                    out.push(Bound::User(id));
                }
                // Not a trait name: the greedy list ends here and the word is
                // the enclosing signature's next slot -- unless nothing has
                // been read yet, where the colon has already committed to a
                // bound (X3).
                None if out.is_empty() => return Err(unknown_capability_error(&c, span)),
                None => break,
            }
        }
        if out.is_empty() {
            return Err(unknown_capability_error("<none>", colon_span));
        }
        Ok(out)
    }

    /// P7.S3e (R18): resolve a bound's trait name at parse time, through the
    /// same `self.imports`/`type_is_exported` gate `resolve_type_or_apply`
    /// already uses for a qualified generic type header. A bound is baked
    /// into `Bound::User(TraitId)` here, before `Resolver::rewrite` runs, so
    /// there is never a trait-name token left for `rewrite` to see.
    ///
    /// `is_first` mirrors `parse_capabilities`' own `out.is_empty()` gate: an
    /// unrecognized *qualifier* (review finding 2) is only a bound-parsing
    /// error when nothing has been read yet -- past the first bound, this
    /// word may just be the enclosing signature's next slot (`q::Point` as a
    /// plain input type), and that slot's own parsing already reports an
    /// unknown-type error for it. A qualifier that *is* recognized but whose
    /// target module doesn't export the named trait (`not_exported_error`)
    /// stays an unconditional error either way: `find_trait_in_module`
    /// already matched a real trait under that name, so this can never be a
    /// plain type slot in disguise.
    fn bound_trait_id(
        &self,
        name: &str,
        span: Span,
        is_first: bool,
    ) -> Result<Option<TraitId>, String> {
        let id = find_trait_in_module(self.traits, name, self.module, self.imports, self.selective);
        let Some((qualifier, base)) = name.split_once("::") else {
            return Ok(id);
        };
        if !self.imports.contains_key(qualifier) {
            if is_first {
                return Err(unbound_bound_qualifier_error(qualifier, base, span));
            }
            return Ok(None);
        }
        if id.is_some() && !self.type_is_exported(qualifier, base) {
            return Err(not_exported_error(base, qualifier, span));
        }
        Ok(id)
    }

    /// Fold a parsed `RawTy` to a `PolyType`, interning any fully-concrete
    /// array shape into the array registry so it becomes a plain
    /// `PolyType::Concrete(Type::Array(..))`; only a variable-bearing array
    /// stays `PolyType::Array` (R4). P7 slice 3a: now fallible -- a nested
    /// (depth > 1) generic application is a located rejection here (D5).
    fn raw_to_poly_type(&mut self, raw: RawTy) -> Result<PolyType, String> {
        Ok(match raw {
            RawTy::Concrete(t) => PolyType::Concrete(t),
            RawTy::Var(id) => PolyType::Var(id),
            RawTy::Array(elem, count) => {
                let elem = self.raw_to_poly_type(*elem)?;
                let len = match count {
                    RawLen::Concrete(n) => Len::Concrete(n),
                    RawLen::Var(id) => Len::Var(id),
                };
                if let (PolyType::Concrete(et), Len::Concrete(n)) = (&elem, &len) {
                    PolyType::Concrete(intern_array_type(self.arrays, *et, *n))
                } else {
                    PolyType::Array(Box::new(elem), len)
                }
            }
            RawTy::Quotation(ins, outs, is_inline, row_in, row_out) => {
                let ins: Vec<PolyType> = ins
                    .into_iter()
                    .map(|r| self.raw_to_poly_type(r))
                    .collect::<Result<_, _>>()?;
                let outs: Vec<PolyType> = outs
                    .into_iter()
                    .map(|r| self.raw_to_poly_type(r))
                    .collect::<Result<_, _>>()?;
                // Fold a fully-concrete effect to `Concrete(Type::Quotation)`
                // exactly as an array shape folds; only a variable-bearing
                // effect stays `PolyType::Quotation` (R5). Slice 10a (R1): a
                // `~` effect folds to `Concrete(Type::InlineQuotation)`
                // instead -- a fully-concrete `~` effect is representable as
                // a `Type`, it just isn't `Type::Quotation`. Slice 10a (R6):
                // a row-bearing effect is suppressed from the fold
                // regardless of concreteness -- `QuotEffect` has nowhere to
                // put the row, so folding would silently discard it.
                let concrete = |row: &[PolyType]| {
                    row.iter()
                        .map(|p| match p {
                            PolyType::Concrete(t) => Some(*t),
                            _ => None,
                        })
                        .collect::<Option<Vec<Type>>>()
                };
                let has_row = row_in.is_some() || row_out.is_some();
                match (concrete(&ins), concrete(&outs)) {
                    (Some(ci), Some(co)) if !has_row => PolyType::Concrete(if is_inline {
                        crate::ast::inline_quotation_type(ci, co)
                    } else {
                        crate::ast::quotation_type(ci, co)
                    }),
                    _ => PolyType::Quotation(ins, outs, is_inline, row_in, row_out),
                }
            }
            // Slice 13 (R-A4): a `&`-led slot folds like an array -- a fully
            // concrete referent interns to a real `Type::Ref`, so only a
            // variable-bearing referent stays `PolyType::Ref`.
            RawTy::Ref(inner, mutable) => {
                let inner = self.raw_to_poly_type(*inner)?;
                if let PolyType::Concrete(t) = inner {
                    PolyType::Concrete(crate::ast::intern_ref_type(self.refs, t, mutable))
                } else {
                    PolyType::Ref(Box::new(inner), mutable)
                }
            }
            // P7.S3n (R3): the cell twin of the `Ref` fold above.
            RawTy::OwnedCell(inner) => {
                let inner = self.raw_to_poly_type(*inner)?;
                if let PolyType::Concrete(t) = inner {
                    PolyType::Concrete(crate::ast::intern_owned_cell_type(self.owned_cells, t))
                } else {
                    PolyType::OwnedCell(Box::new(inner))
                }
            }
            // P7 slice 3a (R1): the fold mirrors the array fold exactly --
            // if every argument is `PolyType::Concrete`, instantiate through
            // `GenericTypes` (byte-for-byte the same as `resolve_type_or_
            // apply`'s existing concrete path) and yield `Concrete`;
            // otherwise keep `PolyType::Generic`. D5: an argument that is
            // itself a generic application (depth > 1, e.g. `Box[Box['T]]`)
            // is rejected here, naming both headers, rather than silently
            // accepted as representable-but-unconsumed.
            RawTy::Generic {
                is_enum,
                idx,
                module,
                args,
                name,
                span,
            } => {
                let args: Vec<PolyType> = args
                    .into_iter()
                    .map(|r| self.raw_to_poly_type(r))
                    .collect::<Result<_, _>>()?;
                if let Some(inner_name) = args.iter().find_map(|a| match a {
                    PolyType::Generic { name, .. } => Some(*name),
                    _ => None,
                }) {
                    return Err(generic_nesting_depth_error(&name, inner_name, span));
                }
                let concrete: Option<Vec<Type>> = args
                    .iter()
                    .map(|a| match a {
                        PolyType::Concrete(t) => Some(*t),
                        _ => None,
                    })
                    .collect();
                if let Some(concrete) = concrete {
                    let regs = NameRegistries {
                        structs: self.structs,
                        enums: self.enums,
                        arrays: self.arrays,
                        cells: self.owned_cells,
                        refs: self.refs,
                    };
                    PolyType::Concrete(if is_enum {
                        self.generics.instantiate_enum(idx, &concrete, module, regs)
                    } else {
                        self.generics
                            .instantiate_struct(idx, &concrete, module, regs)
                    })
                } else {
                    let name: &'static str = Box::leak(name.into_boxed_str());
                    PolyType::Generic {
                        is_enum,
                        idx: idx as u32,
                        module,
                        args,
                        name,
                    }
                }
            }
        })
    }

    fn parse_slots(&mut self, stop: impl Fn(&Token) -> bool) -> Result<Vec<TypedSlot>, String> {
        let mut slots = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error("`)` or `--`")),
                Some((tok, _)) if stop(tok) => break,
                _ => slots.push(self.parse_slot()?),
            }
        }
        Ok(slots)
    }

    fn parse_slot(&mut self) -> Result<TypedSlot, String> {
        // Slice 10a (R2): this is a monomorphic slot -- an `extern:` param or
        // a non-poly word's own slot -- and `~` is only legal as a poly
        // combinator's own declared parameter (`parse_poly_slot`), never
        // reached here.
        if let Some((Token::TildeLBracket, span)) = self.peek() {
            return Err(tilde_quotation_position_error(*span));
        }
        // An array type has no name of its own to lead with (`[i64 4]` opens
        // on `[`, not a word), so an unnamed array slot is recognised before
        // the usual name-then-optional-`:type` read (R3, R7).
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            let ty = if self.quotation_type_ahead() {
                self.parse_quotation_type_expr()?
            } else {
                self.parse_array_type_expr()?
            };
            return Ok(TypedSlot { name: None, ty });
        }
        // An owning-cell type is likewise nameless, so it too is recognised
        // before the name-then-optional-`:type` read. But a `^`-led word
        // immediately followed by `:` is the *name* half of a `name : type`
        // slot, not a bare owning-cell type expression; report the
        // reserved-name error here rather than falling through to
        // `parse_type_expr`, which would try to resolve the `:` itself as an
        // unknown type name.
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^') || w.starts_with('&'))
        {
            if matches!(self.tokens.get(self.pos + 1), Some((Token::Word(w), _)) if w == ":") {
                let (name, span) = self.expect_word_any_spanned()?;
                return Err(if is_reserved_caret_name(&name) {
                    reserved_caret_name_error("slot", &name, span)
                } else {
                    reserved_ref_name_error("slot", &name, span)
                });
            }
            let ty = self.parse_type_expr()?;
            return Ok(TypedSlot { name: None, ty });
        }
        let (text, span) = self.expect_word_any_spanned()?;
        if matches!(self.peek(), Some((Token::Word(w), _)) if w == ":") {
            self.pos += 1;
            let ty = self.parse_type_expr()?;
            Ok(TypedSlot {
                name: Some(text),
                ty,
            })
        } else {
            let ty = self.resolve_type_or_apply(&text, span)?;
            Ok(TypedSlot { name: None, ty })
        }
    }

    /// A type expression: a single word (scalar/struct/enum,
    /// resolved via `resolve_type`), a bracketed array type `[ elem count ]`
    /// (`elem` itself a type expression, nested arrays recurse), or a
    /// `^`-led owning-cell type (nested cells recurse the same way).
    fn parse_type_expr(&mut self) -> Result<Type, String> {
        if let Some((Token::TildeLBracket, span)) = self.peek() {
            return Err(tilde_quotation_position_error(*span));
        }
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            if self.quotation_type_ahead() {
                self.parse_quotation_type_expr()
            } else {
                self.parse_array_type_expr()
            }
        } else if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('&')) {
            self.parse_ref_type_expr()
        } else if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^')) {
            self.parse_owning_cell_type_expr()
        } else {
            let (name, span) = self.expect_word_any_spanned()?;
            self.resolve_type_or_apply(&name, span)
        }
    }

    /// `^` is not a lexer delimiter, so `^^i64` arrives as one word.
    fn parse_owning_cell_type_expr(&mut self) -> Result<Type, String> {
        let (word, span) = self.expect_word_any_spanned()?;
        self.split_owning_cell_word(&word, span)
    }

    /// Resolve a `^`-led type word already lifted off the stream: count the
    /// leading `^`-run, resolve the remainder (recursing into the ongoing
    /// token stream when the run is bare, e.g. `^[u8 4]`), then wrap once per
    /// `^`. Split from `parse_owning_cell_type_expr` so the reference splitter
    /// can hand it a `^`-led *remainder* of its own word (`&!^List`) rather
    /// than a token.
    fn split_owning_cell_word(&mut self, word: &str, span: Span) -> Result<Type, String> {
        let run_len = word.chars().take_while(|&c| c == '^').count();
        let remainder = &word[run_len..];
        let mut inner = if remainder.is_empty() {
            // A bare `^`-run followed by `--` has no following type
            // expression to recurse into, and `--` is the stack-effect
            // separator, never a type name; without this check it falls
            // through to `resolve_type` and blames `--` as an unknown type.
            if matches!(self.peek(), Some((Token::Word(w), _)) if w == "--") {
                return Err(owned_cell_no_payload_error(word, span));
            }
            self.parse_type_expr()?
        } else {
            // `span` names the whole word (e.g. `^Nope` starts at the `^`);
            // point at the remainder's own column so the error names and
            // locates the same text.
            let remainder_span = Span {
                col: span.col + run_len as u32,
                ..span
            };
            self.resolve_type_or_apply(remainder, remainder_span)?
        };
        for _ in 0..run_len {
            inner = crate::ast::intern_owned_cell_type(self.owned_cells, inner);
        }
        Ok(inner)
    }

    /// A `&`-led type expression, in the three shapes the lexer can hand it
    /// over. Neither `&` nor `!` nor `^` is a delimiter but `[` is, so:
    /// `&!Buf` arrives as one word and splits within itself; `&!^List` also
    /// arrives as one word and hands its `^`-led remainder to the owning-cell
    /// splitter; `&![u8 64]` splits *across* tokens and recurses into the
    /// ongoing stream, exactly as a bare `^`-run does.
    fn parse_ref_type_expr(&mut self) -> Result<Type, String> {
        let (word, span) = self.expect_word_any_spanned()?;
        let sigil_len = if word.starts_with("&!") { 2 } else { 1 };
        let mutable = sigil_len == 2;
        let remainder = &word[sigil_len..];
        let remainder_span = Span {
            col: span.col + sigil_len as u32,
            ..span
        };
        let referent = if remainder.is_empty() {
            if matches!(self.peek(), Some((Token::Word(w), _)) if w == "--") {
                return Err(ref_no_referent_error(&word, span));
            }
            self.parse_type_expr()?
        } else if remainder.starts_with('^') {
            self.split_owning_cell_word(remainder, remainder_span)?
        } else {
            self.resolve_type_or_apply(remainder, remainder_span)?
        };
        Ok(crate::ast::intern_ref_type(self.refs, referent, mutable))
    }

    /// The array-type-expression production `[ elem count ]` (D2, D3, M1):
    /// `elem` is a nested type expression, `count` a decimal literal `>= 1`
    /// with no const-expr evaluation. Resolving it interns the `(element,
    /// count)` shape (structurally deduped) and returns the resulting
    /// `Type::Array`. A linear `elem` is not rejected here: struct/enum field
    /// lists aren't resolved until after the whole module is parsed (see
    /// `parse`), so the parser cannot yet know whether a named type is
    /// linear. The checker rejects it once `is_copy` is answerable.
    fn parse_array_type_expr(&mut self) -> Result<Type, String> {
        self.expect(Token::LBracket)?;
        let element = self.parse_type_expr()?;
        let count = self.parse_array_count(element.name())?;
        self.expect(Token::RBracket)?;
        Ok(crate::ast::intern_array_type(self.arrays, element, count))
    }

    /// Slice 6h (D1): whether the `[` just consumed by `parse_term` opens a
    /// **raw array constructor** (`[ Type ; Count ]`) rather than a
    /// quotation literal, decided by scanning to the matching `]` for a
    /// **top-depth `;`**. Mirrors `quotation_type_ahead`'s depth scan, but
    /// `self.pos` already points *past* the leading `[` (`parse_term`
    /// advances unconditionally before dispatching on the token), so the
    /// scan starts at depth `1` rather than `0`. EOF returns `false`, so an
    /// unterminated quotation with no `;` keeps today's "unterminated
    /// quotation" message.
    fn array_ctor_ahead(&self) -> bool {
        let mut depth = 1i32;
        let mut i = self.pos;
        while let Some((tok, _)) = self.tokens.get(i) {
            match tok {
                Token::LBracket => depth += 1,
                Token::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                Token::Semicolon if depth == 1 => return true,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// Slice 6h (D1): parse the body of a `[ Type ; Count ]` array
    /// constructor once `array_ctor_ahead` has committed to it. The element
    /// is a single word token (a compound element type is therefore a
    /// located parse error at `expect_word_any_spanned`, not a new check),
    /// resolved via `resolve_type` exactly as a declared field type is;
    /// `Count` is validated as a literal in `1..=u32::MAX` by
    /// `parse_array_count` before interning, since interning takes a `u32`.
    /// The shape is interned through the parser's own `arrays` registry
    /// (`intern_array_type`), so the term carries a finished `Type::Array`
    /// and lowering never needs a structural `array_id_of` search. `span` is
    /// the already-consumed leading `[`'s span.
    fn parse_array_ctor_term(&mut self, span: Span) -> Result<Term, String> {
        let (name, name_span) = self.expect_word_any_spanned()?;
        let element = self.resolve_type_or_apply(&name, name_span)?;
        self.expect(Token::Semicolon)?;
        let count = self.parse_array_count(element.name())?;
        self.expect(Token::RBracket)?;
        let ty = crate::ast::intern_array_type(self.arrays, element, count);
        Ok(Term {
            kind: TermKind::ArrayCtor(ty),
            span,
        })
    }

    /// Slice 6a (R1): whether the `[` the parser is positioned on opens a
    /// **quotation effect** rather than an array type, decided by scanning to
    /// its matching `]` for a **top-depth `--`**. An array type can never
    /// contain a `--` (arrays hold no quotations, slice 4), and a quotation
    /// effect always contains exactly one at depth 1, so the scan is local and
    /// unambiguous with no new token or sigil. A nested `[ [ i64 -- ] 3 ]` has
    /// its inner `--` at depth 2, so the outer `[` reads as an array (R7a then
    /// rejects the array-of-quotation at check time). Caller has already
    /// confirmed `self.peek()` is `[`.
    fn quotation_type_ahead(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        while let Some((tok, _)) = self.tokens.get(i) {
            match tok {
                Token::LBracket => depth += 1,
                Token::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                Token::Word(w) if w == "--" && depth == 1 => return true,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// Slice 6a (R2): parse `[ <in-types> -- <out-types> ]` into a
    /// `Type::Quotation`. Each side is a possibly-empty list of ordinary type
    /// expressions (reusing `parse_type_expr`, so a nested array/ref/effect is
    /// read the same way), so the nil effect `[ -- ]` is legal. Only called
    /// once `quotation_type_ahead` has confirmed a top-depth `--` exists, so
    /// the input-list scan always terminates on it.
    fn parse_quotation_type_expr(&mut self) -> Result<Type, String> {
        self.expect(Token::LBracket)?;
        let inputs = self.parse_quot_type_list(true)?;
        self.expect_word("--")?;
        let outputs = self.parse_quot_type_list(false)?;
        self.expect(Token::RBracket)?;
        Ok(crate::ast::quotation_type(inputs, outputs))
    }

    /// One side of a quotation effect: type expressions until the delimiter
    /// (`--` for the input side, `]` for the output side). A malformed type on
    /// either side is a located parse error from `parse_type_expr` (R3).
    fn parse_quot_type_list(&mut self, stop_on_arrow: bool) -> Result<Vec<Type>, String> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error(if stop_on_arrow { "`--`" } else { "`]`" })),
                Some((Token::Word(w), _)) if stop_on_arrow && w == "--" => break,
                Some((Token::RBracket, _)) if !stop_on_arrow => break,
                _ => out.push(self.parse_type_expr()?),
            }
        }
        Ok(out)
    }

    /// Phase 6 slice 1 (D1): the optional effect a quotation literal declares
    /// inside its own brackets, read once the opening bracket is consumed. A
    /// leading `(` is the sole disambiguator and is unambiguous: `parse_term`
    /// has no `Token::LParen` arm, so no body term can begin with one. A
    /// literal with no leading `(` parses exactly as before.
    fn parse_optional_quot_annotation(&mut self) -> Result<Option<QuotAnnot>, String> {
        if !matches!(self.peek(), Some((Token::LParen, _))) {
            return Ok(None);
        }
        self.parse_quot_annotation().map(Some)
    }

    /// The annotation itself, `( inputs -- outputs )`. Follows the shape of
    /// `parse_quot_type_list` (a list, `--`, a list) but is deliberately not
    /// that reader: this one stops on `)` rather than `]`, admits the variable
    /// spellings `'T`/`..a`, and mints their ids into the literal's own name
    /// tables rather than any enclosing signature's `PolySig` (R2). R6: only
    /// the full four-part form parses, so `( )` and a parenthesized list with
    /// no `--` are both located errors.
    ///
    /// Phase 6 slice 3 (R1): a leading token naming a known variant (bare,
    /// `&`-prefixed, or `&!`-prefixed) is an eliminator arm's routing tag,
    /// consumed before the ordinary input-list reader sees it and recorded on
    /// `variant_tag`. The lone-variant-name form (`( Circle )`) is the one
    /// place the arrow is elidable: reaching `)` immediately after that one
    /// token is a complete elided arm, not the located `( )` rejection.
    ///
    /// Slice 3b (R1): a tag declares no input slot here. The variant type an
    /// arm receives is built at check time from the scrutinee's own enum, so a
    /// tagged annotation's `inputs` are empty (lone form) or hold only the
    /// ordinary post-`--` slots (escalated form).
    fn parse_quot_annotation(&mut self) -> Result<QuotAnnot, String> {
        let span = self.expect(Token::LParen)?;
        let mut vars = AnnotVars::default();
        let variant_tag = self.parse_leading_variant_slot();
        if variant_tag.is_some() && matches!(self.peek(), Some((Token::RParen, _))) {
            self.pos += 1;
            return Ok(QuotAnnot {
                inputs: vec![],
                outputs: vec![],
                row_in: None,
                row_out: None,
                ty_var_names: vec![],
                row_var_names: vec![],
                span,
                variant_tag,
            });
        }
        let inputs = self.parse_annot_slots(&mut vars, false, |tok| {
            matches!(tok, Token::RParen) || is_word(tok, "--")
        })?;
        if let Some((Token::RParen, span)) = self.peek() {
            return Err(annotation_missing_arrow_error(*span));
        }
        self.expect_word("--")?;
        let outputs =
            self.parse_annot_slots(&mut vars, true, |tok| matches!(tok, Token::RParen))?;
        self.expect(Token::RParen)?;
        Ok(QuotAnnot {
            inputs,
            outputs,
            row_in: vars.row_in,
            row_out: vars.row_out,
            ty_var_names: vars.ty_names,
            row_var_names: vars.row_names,
            span,
            variant_tag,
        })
    }

    /// Phase 6 slice 3b (R1): consume a leading `Variant`/`&Variant`/
    /// `&!Variant` token opening an annotation, if the (sigil-stripped) word
    /// names a variant visible here. Recognition is by *name* only -- the
    /// variant type the arm receives is synthesized at check time from the
    /// scrutinee's enum -- so a generic enum's variant is recognized on equal
    /// footing with a concrete one, though it has no concrete `Type::Variant`
    /// until an instantiation supplies its arguments. The tag records the bare
    /// name (never the sigil-carrying spelling, since routing matches variant
    /// names, which never carry a sigil in a `type:` declaration) and the mode
    /// the sigil spelled. Returns `None`, consuming nothing, when the leading
    /// token isn't a visible variant name at all: an ordinary annotation's
    /// first input slot.
    fn parse_leading_variant_slot(&mut self) -> Option<VariantTag> {
        let word = match self.peek() {
            Some((Token::Word(w), _)) => w.clone(),
            _ => return None,
        };
        let (mode, sigil_len) = if word.starts_with("&!") {
            (VariantTagMode::RefMut, 2)
        } else if word.starts_with('&') {
            (VariantTagMode::Ref, 1)
        } else {
            (VariantTagMode::Owning, 0)
        };
        let bare = &word[sigil_len..];
        if !self.variant_name_is_visible(bare) {
            return None;
        }
        let name = bare.to_string();
        self.pos += 1;
        Some(VariantTag { name, mode })
    }

    /// Whether a bare variant name is visible as a routing tag here, module-
    /// scoped exactly like `resolve_type_name_in_module` one level down (own
    /// module first, falling back to a selectively-imported variant's target
    /// module). Deliberately *not* `is_variant_name`, which matches a variant
    /// of any enum in any module: an annotation's leading slot must not be
    /// captured by a variant declared in a module this one never imported.
    /// An in-scope struct/enum of the same name takes precedence: a variant
    /// name is a routing tag only where no ordinary type resolves.
    fn variant_name_is_visible(&self, name: &str) -> bool {
        if crate::ast::resolve_type_name_in_module(
            self.structs,
            self.enums,
            name,
            self.module,
            self.imports,
            self.selective,
            self.type_origin,
        )
        .is_some()
        {
            return false;
        }
        self.module_declares_variant(name, self.module)
            || self
                .selective
                .get(name)
                .is_some_and(|target| self.module_declares_variant(name, *target))
    }

    /// One module's worth of `variant_name_is_visible`'s search, over the
    /// concrete and the generic enum registry alike. The concrete side
    /// mirrors `find_type_in_module`'s `name_static` match (R8d, slice 5b): a
    /// REPL-spliced enum's `.name` carries an import-epoch tag but its
    /// variants' `.name_static` stays the user-typed spelling.
    fn module_declares_variant(&self, name: &str, module: u32) -> bool {
        self.enums
            .iter()
            .any(|e| e.module == module && e.variants.iter().any(|v| v.name_static == name))
            || self
                .generics
                .enums
                .iter()
                .any(|e| e.module == module && e.variants.iter().any(|v| v.name == name))
    }

    /// One side of an annotation. A leading `..s` (the deepest slot) is the
    /// side's row variable, interned into the annotation's own row table; a
    /// `..s` anywhere else, or a second one, is the same misplacement error a
    /// signature's rows raise.
    fn parse_annot_slots(
        &mut self,
        vars: &mut AnnotVars,
        is_output: bool,
        stop: impl Fn(&Token) -> bool,
    ) -> Result<Vec<PolyType>, String> {
        if let Some((Token::Word(w), _)) = self.peek() {
            if w.starts_with("..") {
                let w = w.clone();
                self.pos += 1;
                let id = intern_var_name(&mut vars.row_names, &w);
                if is_output {
                    vars.row_out = Some(id);
                } else {
                    vars.row_in = Some(id);
                }
            }
        }
        let mut slots = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error(if is_output { "`)`" } else { "`)` or `--`" })),
                Some((tok, _)) if stop(tok) => break,
                Some((Token::Word(w), span)) if w.starts_with("..") => {
                    return Err(row_var_misplaced_error(w, *span));
                }
                Some((Token::Word(w), _)) if w.starts_with('\'') => {
                    let (w, _) = self.expect_word_any_spanned()?;
                    slots.push(PolyType::Var(intern_var_name(&mut vars.ty_names, &w)));
                }
                _ => slots.push(PolyType::Concrete(self.parse_type_expr()?)),
            }
        }
        Ok(slots)
    }

    /// The array count token: a decimal literal `>= 1` and `<= u32::MAX`
    /// (M1: no const-expr eval, so a non-literal count is always a located
    /// error naming the offending token). A literal `< 1` or `> u32::MAX` is
    /// a located error naming the full `[T N]` spelling and the invalid
    /// length (X2).
    fn parse_array_count(&mut self, element: &str) -> Result<u32, String> {
        match self.peek().cloned() {
            Some((Token::Int(n), _span)) if (1..=i64::from(u32::MAX)).contains(&n) => {
                self.pos += 1;
                Ok(n as u32)
            }
            Some((Token::Int(n), span)) if n > i64::from(u32::MAX) => {
                self.pos += 1;
                Err(format!(
                    "error: array type `[{element} {n}]` has invalid length {n} at line {}, col {} (`[T N]` requires N <= {})",
                    span.line, span.col, u32::MAX
                ))
            }
            Some((Token::Int(n), span)) => {
                self.pos += 1;
                Err(format!(
                    "error: array type `[{element} {n}]` has invalid length {n} at line {}, col {} (`[T N]` requires N >= 1)",
                    span.line, span.col
                ))
            }
            Some((tok, span)) => Err(format!(
                "error: array count must be a decimal literal, found `{}` at line {}, col {} (`[T N]` requires a literal N, no const-expr eval)",
                describe_token(&tok), span.line, span.col
            )),
            None => Err(self.eof_error("an array count literal")),
        }
    }

    /// The `type:` production: `type: Name (field-name field-type)* ;`. The
    /// name was already registered by the pre-pass; this parses and returns
    /// the ordered field list. An odd field-token count, a delimiter/
    /// defining-word field type, or a missing `;` is a located parse error.
    fn parse_typedef(&mut self) -> Result<Vec<(String, Type)>, String> {
        self.expect_word("type:")?;
        self.expect_word_any()?; // the struct name; already registered by the pre-pass
        let mut fields = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some(_) => {
                    let (field_name, field_span) = self.expect_word_any_spanned()?;
                    reject_ty_var_field_name(&field_name, field_span)?;
                    if let Some((Token::Semicolon, span)) = self.peek() {
                        return Err(format!(
                            "parse error: field `{field_name}` has no type before `;` at line {}, col {} (odd field-token count in `type:` body)",
                            span.line, span.col
                        ));
                    }
                    let ty = self.parse_field_type_expr()?;
                    fields.push((field_name, ty));
                }
                None => return Err(self.eof_error("`;` (unterminated `type:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        Ok(fields)
    }

    /// A field-type expression: an array type `[ elem count ]`, or a plain
    /// field-type word (rejecting `type:`/`:` as before via
    /// `expect_field_type_token`).
    fn parse_field_type_expr(&mut self) -> Result<Type, String> {
        if let Some((Token::TildeLBracket, span)) = self.peek() {
            return Err(tilde_quotation_position_error(*span));
        }
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            return if self.quotation_type_ahead() {
                self.parse_quotation_type_expr()
            } else {
                self.parse_array_type_expr()
            };
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^')) {
            return self.parse_owning_cell_type_expr();
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('&')) {
            return self.parse_ref_type_expr();
        }
        let (ty_name, ty_span) = self.expect_field_type_token()?;
        self.resolve_type_or_apply(&ty_name, ty_span)
    }

    /// A field-type token: a plain word, but not `type:`/`:` (a malformed
    /// declaration naming a defining word where a type belongs). A delimiter
    /// (`(`/`)`/`|`) is rejected by the existing "expected a word" path.
    fn expect_field_type_token(&mut self) -> Result<(String, Span), String> {
        match self.peek() {
            Some((Token::Word(w), span)) if w == "type:" || w == ":" => {
                let (w, span) = (w.clone(), *span);
                Err(format!(
                    "parse error: expected a field type, found `{w}` at line {}, col {} (malformed `type:` declaration)",
                    span.line, span.col
                ))
            }
            _ => self.expect_word_any_spanned(),
        }
    }

    fn expect_word_any_spanned(&mut self) -> Result<(String, Span), String> {
        match self.peek() {
            Some((Token::Word(w), span)) => {
                let (w, span) = (w.clone(), *span);
                self.pos += 1;
                Ok((w, span))
            }
            Some((tok, span)) => Err(format!(
                "parse error: expected a word, found {tok:?} at line {}, col {}",
                span.line, span.col
            )),
            None => Err(self.eof_error("a word")),
        }
    }

    fn resolve_type(&self, name: &str, span: Span) -> Result<Type, String> {
        // Unknown-type is a semantic error, not a syntax error, so it uses the
        // `error:` prefix (matching check.rs) rather than `parse error:`.
        let ty = crate::ast::resolve_type_name_in_module(
            self.structs,
            self.enums,
            name,
            self.module,
            self.imports,
            self.selective,
            self.type_origin,
        )
        .ok_or_else(|| {
            format!(
                "error: unknown type `{name}` at line {}, col {}",
                span.line, span.col
            )
        })?;
        // R14/R16 (phase 2): a qualified type name resolved above only
        // because it exists in the target module's registry; it must also be
        // exported, distinct from not existing at all (which the branch above
        // already rejected as `unknown type`).
        if let Some((qualifier, base)) = name.split_once("::") {
            if !self.type_is_exported(qualifier, base) {
                return Err(not_exported_error(base, qualifier, span));
            }
        }
        Ok(ty)
    }

    /// Whether `base` is named in `qualifier`'s target module's `export:`
    /// list (R16). The qualifier is assumed to already resolve (the caller
    /// only reaches here after `resolve_type_name_in_module` succeeded via the
    /// qualified branch), so a missing qualifier maps to `true`: nothing to
    /// gate.
    fn type_is_exported(&self, qualifier: &str, base: &str) -> bool {
        match self.imports.get(qualifier) {
            Some(&target) => self
                .exports
                .get(target as usize)
                .is_some_and(|list| list.iter().any(|(n, _)| n == base)),
            None => true,
        }
    }

    /// Lookahead (no consumption): whether the `type:` decl at the current
    /// position is an enum (D1's `|`-separated-variants body), per
    /// `body_has_pipe_before_semicolon`. `self.pos` must point at `type:`.
    /// `body_has_pipe_before_semicolon` scans forward for the first `Pipe` or
    /// `Semicolon` and ignores every other token, so a generic header's bound
    /// type variables (Phase 5 slice 1, always plain `Word` tokens) in the
    /// scanned range don't change the verdict; the search need not skip past
    /// them first.
    fn current_typedef_is_enum(&self) -> bool {
        body_has_pipe_before_semicolon(self.tokens, self.pos + 2)
    }

    /// Lookahead (no consumption): whether the `type:` decl at the current
    /// position is generic (Phase 5 slice 1, R1/D2) -- its header binds one or
    /// more type variables. `self.pos` must point at `type:`.
    fn current_typedef_is_generic(&self) -> bool {
        header_ty_var_count(self.tokens, self.pos + 2) > 0
    }

    /// The enum `type:` production (D1, M3): `type: Name '|'? variant ('|'
    /// variant)* ;`, `variant := Word (field-name field-type)*`. The name and
    /// every variant name were already registered by the pre-pass; this
    /// parses and returns the ordered per-variant field list. Zero variants
    /// (an optional leading `|` with nothing after it, or a body with no
    /// variant at all) is a located malformed-declaration error (M3).
    fn parse_enum_typedef(&mut self) -> Result<Vec<Vec<(String, Type)>>, String> {
        let type_span = self.expect_word("type:")?;
        let name = self.expect_word_any()?; // the enum name; already registered by the pre-pass
        if matches!(self.peek(), Some((Token::Pipe, _))) {
            self.pos += 1;
        }
        let mut variants = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some((Token::Word(_), _)) => {
                    variants.push(self.parse_variant_fields()?);
                    if matches!(self.peek(), Some((Token::Pipe, _))) {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Some((tok, span)) => {
                    return Err(format!(
                        "parse error: expected a variant name, found {tok:?} at line {}, col {}",
                        span.line, span.col
                    ));
                }
                None => return Err(self.eof_error("`;` (unterminated `type:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        if variants.is_empty() {
            return Err(format!(
                "error: malformed `type:` declaration `{name}` (zero variants) at line {}, col {}",
                type_span.line, type_span.col
            ));
        }
        Ok(variants)
    }

    /// One variant's field list: a variant name (already consumed by the
    /// caller's boundary handling elsewhere — here we consume it directly)
    /// followed by `(field-name field-type)*` up to the next `|` or `;`. An
    /// odd field-token count or a malformed field type is a located parse
    /// error, matching `parse_typedef`'s struct-field diagnostics.
    fn parse_variant_fields(&mut self) -> Result<Vec<(String, Type)>, String> {
        self.expect_word_any()?; // the variant name; already registered by the pre-pass
        let mut fields = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) | Some((Token::Pipe, _)) => break,
                Some(_) => {
                    let (field_name, field_span) = self.expect_word_any_spanned()?;
                    reject_ty_var_field_name(&field_name, field_span)?;
                    if let Some((tok, span)) = self.peek() {
                        if matches!(tok, Token::Semicolon | Token::Pipe) {
                            return Err(format!(
                                "parse error: field `{field_name}` has no type before `{tok:?}` at line {}, col {} (odd field-token count in `type:` body)",
                                span.line, span.col
                            ));
                        }
                    }
                    let ty = self.parse_field_type_expr()?;
                    fields.push((field_name, ty));
                }
                None => return Err(self.eof_error("`;` or `|` (unterminated `type:` declaration)")),
            }
        }
        Ok(fields)
    }

    /// Phase 5 slice 1 (R1): the generic struct `type:` production, `type:
    /// Name ('var)+ (field-name field-type)* ;`. `current_typedef_is_generic`
    /// only peeks at the header to classify the declaration; it consumes no
    /// tokens, so the bound type variables are parsed here, then each field's
    /// type resolves through
    /// `parse_generic_field_type_expr` against that variable table. A bound
    /// variable never referenced by any field (a phantom parameter) is a
    /// located error (added during round-2 review): R5's instantiation
    /// dispatch disambiguates two instantiations by their constructor's
    /// *input* types alone, which a phantom variable (varying only the
    /// output) breaks.
    ///
    /// P7.S3n (R2): split in half. `parse_generic_header` reads the header
    /// alone, so `parse_generic_typedefs`' stage (a) can register a
    /// placeholder for every header in the file before any *field list* is
    /// parsed; this half parses one already-registered header's field list,
    /// resuming at the token position stage (a) recorded. Splitting it is
    /// what lets a field name its own declaration (`next ^L['T]`), or a
    /// header declared further down.
    fn parse_generic_typedef_fields(
        &mut self,
        name: &str,
        ty_vars: &[(String, Span)],
    ) -> Result<Vec<(String, PolyType)>, String> {
        let mut used = vec![false; ty_vars.len()];
        let mut fields = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some(_) => {
                    let (field_name, field_span) = self.expect_word_any_spanned()?;
                    reject_ty_var_field_name(&field_name, field_span)?;
                    if let Some((Token::Semicolon, span)) = self.peek() {
                        return Err(generic_odd_field_count_error(
                            name,
                            ty_vars,
                            &field_name,
                            ";",
                            *span,
                        ));
                    }
                    let ty = self.parse_generic_field_type_expr(name, ty_vars, &mut used)?;
                    fields.push((field_name, ty));
                }
                None => return Err(self.eof_error("`;` (unterminated `type:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        check_no_phantom_ty_var(name, ty_vars, &used)?;
        Ok(fields)
    }

    /// The enum twin of `parse_generic_typedef_fields` (D1, M3, R1): `'|'?
    /// variant ('|' variant)* ;`, resuming past an already-registered header.
    fn parse_generic_enum_typedef_variants(
        &mut self,
        name: &str,
        ty_vars: &[(String, Span)],
        type_span: Span,
    ) -> Result<Vec<crate::ast::GenericVariantDecl>, String> {
        if matches!(self.peek(), Some((Token::Pipe, _))) {
            self.pos += 1;
        }
        let mut used = vec![false; ty_vars.len()];
        let mut variants = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some((Token::Word(_), _)) => {
                    variants.push(self.parse_generic_variant_fields(name, ty_vars, &mut used)?);
                    if matches!(self.peek(), Some((Token::Pipe, _))) {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Some((tok, span)) => {
                    return Err(format!(
                        "parse error: expected a variant name, found {tok:?} at line {}, col {}",
                        span.line, span.col
                    ));
                }
                None => return Err(self.eof_error("`;` (unterminated `type:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        if variants.is_empty() {
            return Err(format!(
                "error: malformed `type:` declaration `{name}` (zero variants) at line {}, col {}",
                type_span.line, type_span.col
            ));
        }
        check_no_phantom_ty_var(name, ty_vars, &used)?;
        Ok(variants)
    }

    /// P7.S3n (R2): a generic `type:`'s header alone -- `type: Name ('var)+`
    /// -- leaving the cursor at the first field/variant token. Everything a
    /// placeholder registration needs (name, bound variables, span) is known
    /// here; nothing a field list needs is missing.
    fn parse_generic_header(&mut self) -> Result<GenericHeader, String> {
        let type_span = self.expect_word("type:")?;
        let (name, _) = self.expect_word_any_spanned()?;
        let ty_vars = self.parse_generic_header_vars(&name)?;
        Ok((name, ty_vars, type_span))
    }

    /// One generic variant's field list, mirroring `parse_variant_fields`
    /// with fields resolved through `parse_generic_field_type_expr` instead.
    /// The reserved-name gate runs here rather than in a pre-pass: the
    /// module-level pre-pass skips every generic header entirely (its
    /// variant names are only ever seen by this parser), so this is the one
    /// site that can reject a generic variant named `^Evil`.
    fn parse_generic_variant_fields(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
    ) -> Result<crate::ast::GenericVariantDecl, String> {
        let (vname, vspan) = self.expect_word_any_spanned()?;
        reject_reserved_name("variant", &vname, vspan)?;
        let mut fields = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) | Some((Token::Pipe, _)) => break,
                // OQ4: a bare `'`-prefixed token is never a field name
                // (`reject_ty_var_field_name`), so it unambiguously opens an
                // attributeless field -- no lookahead needed, unlike the
                // named-field-missing-its-type case below.
                Some((Token::Word(w), _)) if w.starts_with('\'') => {
                    let ty = self.parse_generic_field_type_expr(decl_name, ty_vars, used)?;
                    fields.push((POSITIONAL_FIELD_NAME.to_string(), ty));
                }
                Some(_) => {
                    // The `'`-prefixed arm above already consumes every
                    // type-variable token, so `field_name` here can never
                    // start with `'`; unlike the other three field-name
                    // sites, `reject_ty_var_field_name` would be dead code.
                    let field_name = self.expect_word_any()?;
                    if let Some((tok, span)) = self.peek() {
                        if matches!(tok, Token::Semicolon | Token::Pipe) {
                            return Err(generic_odd_field_count_error(
                                decl_name,
                                ty_vars,
                                &field_name,
                                if matches!(tok, Token::Semicolon) {
                                    ";"
                                } else {
                                    "|"
                                },
                                *span,
                            ));
                        }
                    }
                    let ty = self.parse_generic_field_type_expr(decl_name, ty_vars, used)?;
                    fields.push((field_name, ty));
                }
                None => return Err(self.eof_error("`;` or `|` (unterminated `type:` declaration)")),
            }
        }
        Ok(crate::ast::GenericVariantDecl {
            name: vname,
            fields,
            span: vspan,
        })
    }

    /// Phase 5 slice 1 (R2): parse every generic `type:` declaration in the
    /// file ahead of the ordinary body pass, wherever it sits in the source,
    /// so an application (`Box[i64]`) resolves against a generic type declared
    /// further down -- the order-independence the concrete pre-pass already
    /// gives a plain `type:` name. The body pass skips each declaration when
    /// it reaches it, so every one is parsed exactly once.
    ///
    /// Idempotent (slice 2, OQ1): a header already registered *before this
    /// pass began* is skipped rather than reparsed, so the driver's
    /// whole-closure `prepass_generic_typedefs` and the `parse_bodies` call
    /// below can both run over the same tokens without the second call
    /// swallowing its own declaration. The snapshot (`already`) is what keeps
    /// this from also swallowing a genuine second header for the same name
    /// within this very pass: without it, the first header's own push would
    /// make the second look pre-registered, so a real duplicate never reached
    /// `check_duplicate_type_names` (round-2 review fix). The single-file and
    /// direct-`parse_bodies` paths have no whole-closure pre-pass and register
    /// here alone.
    ///
    /// P7.S3n (R2): two-stage. Stage (a) walks the whole token stream and
    /// registers a *placeholder* for every header -- name and bound variables
    /// only, empty field list -- recording where that header's field list
    /// starts. Stage (b) revisits each recorded position and parses only the
    /// field/variant list, filling the placeholder in place. A single loop
    /// registering each header immediately before parsing its own fields is
    /// not enough: a mutual cycle (`A` naming `B`, declared later, and `B`
    /// naming `A`) needs *both* headers registered before *either* field list
    /// is read.
    ///
    /// Diagnostic-ordering consequence, stated rather than hidden: a
    /// header-level error (a duplicate `'`-var, a malformed header) now
    /// surfaces during stage (a), so on a multi-error file it can precede an
    /// earlier declaration's field-level error. Every error still fires.
    fn parse_generic_typedefs(&mut self) -> Result<(), String> {
        let already = (self.generics.structs.len(), self.generics.enums.len());
        // (is_enum, registry index, the field/variant list's token position,
        // the header itself)
        let mut headers: Vec<(bool, usize, usize, GenericHeader)> = Vec::new();
        let mut i = 0;
        while i < self.tokens.len() {
            if matches!(&self.tokens[i], (Token::Word(w), _) if w == "type:")
                && header_ty_var_count(self.tokens, i + 2) > 0
            {
                self.pos = i;
                if self.generic_header_at_cursor_is_registered(already) {
                    self.skip_typedef();
                } else {
                    let is_enum = self.current_typedef_is_enum();
                    let (name, ty_vars, type_span) = self.parse_generic_header()?;
                    let ty_var_names = ty_vars.iter().map(|(n, _)| n.clone()).collect();
                    let idx = if is_enum {
                        self.generics
                            .push_enum_placeholder(crate::ast::GenericEnumDecl {
                                name: name.clone(),
                                ty_var_names,
                                variants: Vec::new(),
                                span: type_span,
                                module: self.module,
                            })
                    } else {
                        self.generics
                            .push_struct_placeholder(crate::ast::GenericStructDecl {
                                name: name.clone(),
                                ty_var_names,
                                fields: Vec::new(),
                                span: type_span,
                                module: self.module,
                            })
                    };
                    headers.push((is_enum, idx, self.pos, (name, ty_vars, type_span)));
                    self.skip_typedef();
                }
                i = self.pos;
                continue;
            }
            i += 1;
        }
        for (is_enum, idx, pos, (name, ty_vars, type_span)) in headers {
            self.pos = pos;
            if is_enum {
                let variants =
                    self.parse_generic_enum_typedef_variants(&name, &ty_vars, type_span)?;
                // Disjoint field borrows: `regs` reads the concrete
                // registries, `generics` is a separate field.
                let regs = NameRegistries {
                    structs: self.structs,
                    enums: self.enums,
                    arrays: self.arrays,
                    cells: self.owned_cells,
                    refs: self.refs,
                };
                self.generics.fill_enum_variants(idx, variants, regs);
            } else {
                let fields = self.parse_generic_typedef_fields(&name, &ty_vars)?;
                self.generics.fill_struct_fields(idx, fields);
            }
        }
        self.pos = 0;
        Ok(())
    }

    /// Whether the generic `type:` header at the cursor already had a
    /// declaration in this module's `generics` registry *before this pass
    /// began* -- `already` is `(structs.len(), enums.len())` snapshotted at
    /// entry to `parse_generic_typedefs`, so an index registered by an
    /// earlier iteration of this same pass does not count, and a true
    /// duplicate header still reaches the real production (and downstream,
    /// `check_duplicate_type_names`). `self.pos` must point at `type:`; a
    /// header whose name token is missing is left unregistered for the real
    /// production to reject.
    fn generic_header_at_cursor_is_registered(&self, already: (usize, usize)) -> bool {
        let Some((Token::Word(name), _)) = self.tokens.get(self.pos + 1) else {
            return false;
        };
        self.generics
            .find_struct(name, self.module)
            .is_some_and(|idx| idx < already.0)
            || self
                .generics
                .find_enum(name, self.module)
                .is_some_and(|idx| idx < already.1)
    }

    /// Advance past a whole `type:` declaration without parsing it. An
    /// unterminated one needs no error here: `parse_generic_typedefs` already
    /// parsed (and would already have rejected) every declaration this is
    /// called for.
    fn skip_typedef(&mut self) {
        while let Some((tok, _)) = self.tokens.get(self.pos) {
            let terminator = matches!(tok, Token::Semicolon);
            self.pos += 1;
            if terminator {
                break;
            }
        }
    }

    /// R2/R3/R4: resolve a type name that may be a generic type applied to
    /// concrete type arguments (`Box[i64]`). A generic name must be applied
    /// where it is named -- bare `Box` names no concrete type, which is why an
    /// unapplied generic reports the argument-count error rather than
    /// `unknown type`. Every other name resolves exactly as it did before.
    ///
    /// Slice 2 (OQ1): a `q::Box[i64]` name maps `q` through the import map and
    /// looks the header up in that module, mirroring how
    /// `resolve_type_name_in_module` reaches a concrete cross-module type. A
    /// bare name resolves against this module, or, failing that, against the
    /// module it is selectively imported from -- again as for a concrete type.
    ///
    /// R14/R16 (phase 2 review fix): a declared-but-unexported generic header
    /// is gated here, mirroring `resolve_type`'s check for a concrete
    /// cross-module type -- otherwise a private generic type would be
    /// reachable from another module while a private concrete one is not. A
    /// bare selectively imported name needs no gate here: `check`'s
    /// `check_selective_imports` rejects a private one post-assembly, which is
    /// how a concrete selective import is validated too.
    fn resolve_type_or_apply(&mut self, name: &str, span: Span) -> Result<Type, String> {
        // P7 slice 3c (R1.1): `Slice[T]` is spelled like a generic
        // application but resolves through the interned slice registry, not
        // through a declared header, so it is intercepted ahead of every user
        // lookup (`reject_reserved_name` keeps the name unclaimable).
        if matches!(name, SLICE_TYPE_NAME | MUT_SLICE_TYPE_NAME) {
            let args = self.parse_type_arguments(name, 1, span)?;
            let mutable = name == MUT_SLICE_TYPE_NAME;
            return Ok(crate::ast::intern_slice_type(self.slices, args[0], mutable));
        }
        let (base, owner, qualifier) = match name.split_once("::") {
            Some((qualifier, base)) => match self.imports.get(qualifier) {
                Some(&target) => (base, target, Some(qualifier)),
                // An unbound qualifier is `resolve_type`'s error to report.
                None => return self.resolve_type(name, span),
            },
            None => (name, self.bare_generic_owner(name), None),
        };
        if let Some(qualifier) = qualifier {
            if self.generic_is_declared(base, owner) && !self.type_is_exported(qualifier, base) {
                return Err(not_exported_error(base, qualifier, span));
            }
        }
        if let Some(idx) = self.generics.find_struct(base, owner) {
            let arity = self.generics.structs[idx].ty_var_names.len();
            let args = self.parse_type_arguments(name, arity, span)?;
            let regs = NameRegistries {
                structs: self.structs,
                enums: self.enums,
                arrays: self.arrays,
                cells: self.owned_cells,
                refs: self.refs,
            };
            return Ok(self
                .generics
                .instantiate_struct(idx, &args, self.module, regs));
        }
        if let Some(idx) = self.generics.find_enum(base, owner) {
            let arity = self.generics.enums[idx].ty_var_names.len();
            let args = self.parse_type_arguments(name, arity, span)?;
            let regs = NameRegistries {
                structs: self.structs,
                enums: self.enums,
                arrays: self.arrays,
                cells: self.owned_cells,
                refs: self.refs,
            };
            return Ok(self
                .generics
                .instantiate_enum(idx, &args, self.module, regs));
        }
        self.resolve_type(name, span)
    }

    /// P7 slice 3a (R1): the generic-header twin of `resolve_type_or_apply`'s
    /// name resolution -- the same qualifier split, own-module/selective
    /// fallback, and privacy gate (`:3130-3143` above) -- but stopping short
    /// of parsing arguments: `parse_poly_slot`'s new arm parses each argument
    /// as a poly slot, not through `parse_type_arguments`'s concrete-only
    /// parser. `Ok(None)` means `name` names no generic header at all (an
    /// unbound qualifier, or an ordinary concrete/unknown name), so the
    /// caller falls through to `parse_type_expr` unchanged.
    fn poly_generic_header(
        &self,
        name: &str,
        span: Span,
    ) -> Result<Option<(bool, usize, u32)>, String> {
        let (base, owner, qualifier) = match name.split_once("::") {
            Some((qualifier, base)) => match self.imports.get(qualifier) {
                Some(&target) => (base, target, Some(qualifier)),
                None => return Ok(None),
            },
            None => (name, self.bare_generic_owner(name), None),
        };
        if let Some(qualifier) = qualifier {
            if self.generic_is_declared(base, owner) && !self.type_is_exported(qualifier, base) {
                return Err(not_exported_error(base, qualifier, span));
            }
        }
        if let Some(idx) = self.generics.find_struct(base, owner) {
            return Ok(Some((false, idx, owner)));
        }
        if let Some(idx) = self.generics.find_enum(base, owner) {
            return Ok(Some((true, idx, owner)));
        }
        Ok(None)
    }

    /// R15c: the module a bare generic name is declared in -- this one, or,
    /// when this one declares no such header, the module the name is
    /// selectively imported from (`import: "box.sth" q | Box | `). Own module
    /// first, exactly as `resolve_type_name_in_module` orders the two for a
    /// concrete name, so a local header shadows a selectively imported one.
    fn bare_generic_owner(&self, name: &str) -> u32 {
        if self.generic_is_declared(name, self.module) {
            return self.module;
        }
        self.selective.get(name).copied().unwrap_or(self.module)
    }

    /// Whether `module` declares a generic `type:` header named `name`, of
    /// either shape.
    fn generic_is_declared(&self, name: &str, module: u32) -> bool {
        self.generics.find_struct(name, module).is_some()
            || self.generics.find_enum(name, module).is_some()
    }

    /// R2/R3: a generic-type application's bracketed argument list,
    /// `[ type-expr* ]`, each argument a full type expression (so
    /// `Wrap[Box[i64]]` and `Buf[[i64 4]]` fall out of the recursion).
    ///
    /// Bracketed rather than juxtaposed (`Box i64`) because R3's
    /// argument-count error has to be *decidable*: juxtaposed, a signature
    /// slot list `( Box i64 bool -- )` reads identically as an over-applied
    /// `Box` and as a correctly applied one beside a `bool` slot, so an extra
    /// argument could never be diagnosed there. Brackets also match how
    /// ROADMAP.md spells a use site (`Option['T]`, `Map['K 'V]`), and `[` is
    /// already the type sublanguage's own delimiter.
    fn parse_type_arguments(
        &mut self,
        name: &str,
        arity: usize,
        span: Span,
    ) -> Result<Vec<Type>, String> {
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            return Err(generic_arity_error(name, arity, 0, span));
        }
        self.pos += 1;
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated generic type application)"));
                }
                _ => args.push(self.parse_type_expr()?),
            }
        }
        if args.len() != arity {
            return Err(generic_arity_error(name, arity, args.len(), span));
        }
        Ok(args)
    }

    /// A generic `type:` header's bound type variables (R1/D2): one or more
    /// `'`-prefixed words immediately following the declared name, each
    /// interned with its span for the phantom-variable diagnostic. The
    /// caller (`current_typedef_is_generic`) has already established at
    /// least one is present. A name bound twice in one header is rejected
    /// here, at the binding site, rather than left for a field reference to
    /// misreport as unbound or phantom.
    fn parse_generic_header_vars(
        &mut self,
        decl_name: &str,
    ) -> Result<Vec<(String, Span)>, String> {
        let mut ty_vars: Vec<(String, Span)> = Vec::new();
        while matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('\'')) {
            let (w, span) = self.expect_word_any_spanned()?;
            if ty_vars.iter().any(|(n, _)| *n == w) {
                return Err(duplicate_generic_ty_var_error(&w, decl_name, span));
            }
            ty_vars.push((w, span));
        }
        Ok(ty_vars)
    }

    /// A generic `type:` field's type (R1): a recursive descent over the
    /// shapes that can wrap one of the header's bound variables -- array
    /// (`['T 2]`, nested to any depth), reference (`&'T`, `&!'T`), owning
    /// cell (`^'T`), and generic application (`Ent['K 'V]`) -- with every
    /// leaf `'name` resolved against `ty_vars` and marked used (for the
    /// phantom check, at whatever depth it sits). A fully-concrete field
    /// falls through to `parse_field_type_expr`, which resolves it exactly
    /// as a non-generic `type:`'s field. A `'name` not found in `ty_vars` is
    /// a located error naming the declaration -- distinct from an unbound
    /// variable in a word signature, which errors through `PolyBuilder`.
    ///
    /// P7.S3n (R8): the finished type tree is walked for a *growing*
    /// self-reference before it is returned, so a declaration that could
    /// only instantiate forever is rejected here rather than hanging later.
    fn parse_generic_field_type_expr(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
    ) -> Result<PolyType, String> {
        let span = self.peek().map(|(_, s)| *s).unwrap_or_default();
        let pty = self.parse_generic_field_shape(decl_name, ty_vars, used)?;
        reject_growing_generic_argument(decl_name, &pty, span)?;
        Ok(pty)
    }

    /// R1's descent proper, split from `parse_generic_field_type_expr` so
    /// R8's whole-tree growth check runs once per *field* rather than once
    /// per node the recursion visits.
    fn parse_generic_field_shape(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
    ) -> Result<PolyType, String> {
        if let Some((Token::TildeLBracket, span)) = self.peek() {
            return Err(tilde_quotation_position_error(*span));
        }
        // A `[` opens either a quotation effect or an array type, decided by
        // `quotation_type_ahead`'s top-depth `--` scan exactly as
        // `parse_field_type_expr` decides it -- without this the array
        // production would misparse a legal concrete quotation field.
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            if self.quotation_type_ahead() {
                // R7: a quotation field naming the declaration's own type
                // variable is out of scope, rejected here rather than left
                // to misreport `'T` as an unknown concrete type. A quotation
                // field over concrete types alone still parses.
                if let Some((var, span)) = self.quotation_effect_ty_var_ahead(ty_vars) {
                    return Err(quotation_field_ty_var_error(decl_name, &var, span));
                }
                return Ok(PolyType::Concrete(self.parse_quotation_type_expr()?));
            }
            return self.parse_generic_field_array(decl_name, ty_vars, used);
        }
        if let Some((Token::Word(w), span)) = self.peek() {
            let (w, span) = (w.clone(), *span);
            if w.starts_with('\'') {
                self.pos += 1;
                return Ok(PolyType::Var(
                    self.resolve_field_ty_var(decl_name, ty_vars, used, &w, span)?,
                ));
            }
            // A `&`-led field: intercepted before the concrete fall-through,
            // which resolves the referent concretely and would blame `'T` as
            // an unknown type. The field does not *build* (a reference can
            // never be stored, `check_no_stored_reference`), but it must fail
            // with that rule's message rather than `unknown type`.
            if w.starts_with('&') {
                let sigil_len = if w.starts_with("&!") { 2 } else { 1 };
                let mutable = sigil_len == 2;
                let remainder = &w[sigil_len..];
                if remainder.is_empty() {
                    self.pos += 1;
                    let inner = self.parse_generic_field_shape(decl_name, ty_vars, used)?;
                    return Ok(self.fold_field_ref(inner, mutable));
                }
                if remainder.starts_with('\'') {
                    let remainder = remainder.to_string();
                    let remainder_span = Span {
                        col: span.col + sigil_len as u32,
                        ..span
                    };
                    self.pos += 1;
                    let id = self.resolve_field_ty_var(
                        decl_name,
                        ty_vars,
                        used,
                        &remainder,
                        remainder_span,
                    )?;
                    return Ok(self.fold_field_ref(PolyType::Var(id), mutable));
                }
            }
            // A `^`-led field. `^` is the only indirection a field can
            // actually store (a reference never can, and an array does not
            // break a recursion), so this arm carries the shapes the slice
            // exists for: a bare run whose payload is the following token, a
            // run glued to a variable, and a run glued to a generic header
            // that is then applied (`^L['T]`, `^Ent['K 'V]`).
            if w.starts_with('^') {
                let run_len = w.chars().take_while(|&c| c == '^').count();
                let remainder = w[run_len..].to_string();
                let remainder_span = Span {
                    col: span.col + run_len as u32,
                    ..span
                };
                let inner = if remainder.is_empty() {
                    self.pos += 1;
                    if matches!(self.peek(), Some((Token::Semicolon | Token::Pipe, _)))
                        || self.peek().is_none()
                    {
                        return Err(owned_cell_no_payload_error(&w, span));
                    }
                    Some(self.parse_generic_field_shape(decl_name, ty_vars, used)?)
                } else if remainder.starts_with('\'') {
                    self.pos += 1;
                    let id = self.resolve_field_ty_var(
                        decl_name,
                        ty_vars,
                        used,
                        &remainder,
                        remainder_span,
                    )?;
                    Some(PolyType::Var(id))
                } else if matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _))) {
                    match self.poly_generic_header(&remainder, remainder_span)? {
                        Some((is_enum, idx, module)) => {
                            self.pos += 1;
                            Some(self.parse_generic_field_application(
                                decl_name,
                                ty_vars,
                                used,
                                &remainder,
                                is_enum,
                                idx,
                                module,
                                remainder_span,
                            )?)
                        }
                        None => None,
                    }
                } else {
                    None
                };
                if let Some(mut inner) = inner {
                    for _ in 0..run_len {
                        inner = self.fold_field_owned_cell(inner);
                    }
                    return Ok(inner);
                }
            }
            // A generic type applied to field types (`Ent['K 'V]`), including
            // this declaration's own header (`L['T]`, `L[i64]`) -- which
            // stage (a) has already registered a placeholder for, so it
            // resolves here rather than reporting an unknown type.
            if !w.starts_with('^')
                && !w.starts_with('&')
                && matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _)))
            {
                if let Some((is_enum, idx, module)) = self.poly_generic_header(&w, span)? {
                    self.pos += 1;
                    return self.parse_generic_field_application(
                        decl_name, ty_vars, used, &w, is_enum, idx, module, span,
                    );
                }
            }
        }
        let ty = self.parse_field_type_expr()?;
        Ok(PolyType::Concrete(ty))
    }

    /// One `'name` leaf in a generic field type: its index in `ty_vars`,
    /// marking it used. Shared by every arm of R1's descent, so a variable
    /// nested three deep counts against the phantom check exactly as a bare
    /// one does.
    fn resolve_field_ty_var(
        &self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        name: &str,
        span: Span,
    ) -> Result<u32, String> {
        let idx = ty_vars
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| unbound_generic_ty_var_error(name, decl_name, span))?;
        used[idx] = true;
        Ok(idx as u32)
    }

    /// A generic field's array type `[ elem count ]`, `elem` recursing so a
    /// nested `[['T 2] 2]` falls out. N3: a struct header binds no *length*
    /// variable, so the count is always a literal here -- `parse_array_count`
    /// is reused verbatim and no `Len::Var` path exists.
    fn parse_generic_field_array(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
    ) -> Result<PolyType, String> {
        self.expect(Token::LBracket)?;
        let elem = self.parse_generic_field_shape(decl_name, ty_vars, used)?;
        // `parse_array_count`'s linear-element rejection needs a concrete
        // element type; over a variable element there is none to give it, so
        // the count is read as a bare literal and the element's linearity is
        // left to the checker, exactly as it is for a concrete array field.
        let count = match &elem {
            PolyType::Concrete(t) => self.parse_array_count(t.name())?,
            elem => self.parse_array_count(&generic_field_type_str(elem, ty_vars))?,
        };
        self.expect(Token::RBracket)?;
        Ok(match elem {
            PolyType::Concrete(t) => {
                PolyType::Concrete(crate::ast::intern_array_type(self.arrays, t, count))
            }
            elem => PolyType::Array(Box::new(elem), Len::Concrete(count)),
        })
    }

    /// A generic field's generic-type application, each argument a field type
    /// rather than a concrete type expression -- the field-parser twin of
    /// `parse_type_arguments`, reusing only its arity check. A fully-concrete
    /// argument list instantiates immediately and folds to `Concrete`,
    /// byte-for-byte as `resolve_type_or_apply` already does; otherwise the
    /// application stays `PolyType::Generic` for substitution to ground.
    #[allow(clippy::too_many_arguments)]
    fn parse_generic_field_application(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        name: &str,
        is_enum: bool,
        idx: usize,
        module: u32,
        span: Span,
    ) -> Result<PolyType, String> {
        let arity = if is_enum {
            self.generics.enums[idx].ty_var_names.len()
        } else {
            self.generics.structs[idx].ty_var_names.len()
        };
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            return Err(generic_arity_error(name, arity, 0, span));
        }
        self.pos += 1;
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated generic type application)"));
                }
                _ => args.push(self.parse_generic_field_shape(decl_name, ty_vars, used)?),
            }
        }
        if args.len() != arity {
            return Err(generic_arity_error(name, arity, args.len(), span));
        }
        let concrete: Option<Vec<Type>> = args
            .iter()
            .map(|a| match a {
                PolyType::Concrete(t) => Some(*t),
                _ => None,
            })
            .collect();
        if let Some(concrete) = concrete {
            let regs = NameRegistries {
                structs: self.structs,
                enums: self.enums,
                arrays: self.arrays,
                cells: self.owned_cells,
                refs: self.refs,
            };
            return Ok(PolyType::Concrete(if is_enum {
                self.generics.instantiate_enum(idx, &concrete, module, regs)
            } else {
                self.generics
                    .instantiate_struct(idx, &concrete, module, regs)
            }));
        }
        Ok(PolyType::Generic {
            is_enum,
            idx: idx as u32,
            module,
            args,
            name: Box::leak(name.to_string().into_boxed_str()),
        })
    }

    /// Fold a `&`-wrapped field type, interning a fully-concrete referent
    /// into a real `Type::Ref` exactly as `raw_to_poly_type` folds one.
    fn fold_field_ref(&mut self, inner: PolyType, mutable: bool) -> PolyType {
        match inner {
            PolyType::Concrete(t) => {
                PolyType::Concrete(crate::ast::intern_ref_type(self.refs, t, mutable))
            }
            inner => PolyType::Ref(Box::new(inner), mutable),
        }
    }

    /// The owning-cell twin of `fold_field_ref`.
    fn fold_field_owned_cell(&mut self, inner: PolyType) -> PolyType {
        match inner {
            PolyType::Concrete(t) => {
                PolyType::Concrete(crate::ast::intern_owned_cell_type(self.owned_cells, t))
            }
            inner => PolyType::OwnedCell(Box::new(inner)),
        }
    }

    /// R7: the first of the declaration's own type variables mentioned inside
    /// the quotation effect the cursor is positioned on, scanning to its
    /// matching `]`. An *unbound* `'`-name is deliberately not reported here:
    /// it falls through to the concrete parser's own unknown-type error,
    /// which is the right message for a typo.
    fn quotation_effect_ty_var_ahead(&self, ty_vars: &[(String, Span)]) -> Option<(String, Span)> {
        let mut depth = 0i32;
        let mut i = self.pos;
        while let Some((tok, span)) = self.tokens.get(i) {
            match tok {
                Token::LBracket => depth += 1,
                Token::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return None;
                    }
                }
                Token::Word(w) => {
                    // A leading `^` or `&`/`&!` glues onto the variable's own
                    // token (`^'T`, `&'T`), so the bare name must be peeled
                    // off before comparing -- otherwise this scan is blind
                    // to exactly the shapes `fold_field_ref`/`fold_field_
                    // owned_cell` exist to admit, and the variable falls
                    // through to the concrete parser's misleading `unknown
                    // type 'T` instead of this rule's own message.
                    let bare = w
                        .strip_prefix('^')
                        .or_else(|| w.strip_prefix("&!"))
                        .or_else(|| w.strip_prefix('&'))
                        .unwrap_or(w);
                    if ty_vars.iter().any(|(n, _)| n == bare) {
                        return Some((bare.to_string(), *span));
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// Parse a `| names |` binding at the current `|`. At least one name is
    /// required (R1): `| |` is a parse error, not a no-op, so a stray pipe pair
    /// cannot silently mean nothing.
    fn parse_binding_names(&mut self) -> Result<Vec<String>, String> {
        let open = match self.peek() {
            Some((Token::Pipe, span)) => *span,
            _ => unreachable!("parse_binding_names is only called at a `|`"),
        };
        self.pos += 1;
        let mut names = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Pipe, _)) => {
                    self.pos += 1;
                    if names.is_empty() {
                        return Err(format!(
                            "parse error: `| |` binds nothing at line {}, col {}\n  a binding must name at least one local",
                            open.line, open.col
                        ));
                    }
                    break;
                }
                Some((Token::Word(w), span)) => {
                    reject_reserved_name("local", w, *span)?;
                    names.push(w.clone());
                    self.pos += 1;
                }
                Some((tok, span)) => {
                    return Err(format!(
                        "parse error: expected a local name or `|`, found {tok:?} at line {}, col {}",
                        span.line, span.col
                    ));
                }
                None => return Err(self.eof_error("`|`")),
            }
        }
        Ok(names)
    }

    fn parse_terms(
        &mut self,
        expected: &str,
        stop: impl Fn(&Token) -> bool,
    ) -> Result<Vec<Term>, String> {
        let mut terms = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error(expected)),
                Some((tok, _)) if stop(tok) => break,
                _ => terms.push(self.parse_term()?),
            }
        }
        Ok(terms)
    }

    fn parse_term(&mut self) -> Result<Term, String> {
        let (tok, span) = self
            .peek()
            .cloned()
            .ok_or_else(|| self.eof_error("a term"))?;
        // R1: a `|` at any term position opens a binding.
        if matches!(tok, Token::Pipe) {
            let names = self.parse_binding_names()?;
            return Ok(Term {
                kind: TermKind::Bind(names),
                span,
            });
        }
        self.pos += 1;
        match tok {
            Token::Int(n) => Ok(Term {
                kind: TermKind::IntLit(n),
                span,
            }),
            Token::Float(v) => Ok(Term {
                kind: TermKind::FloatLit(v),
                span,
            }),
            Token::Str(s) => Ok(Term {
                kind: TermKind::StrLit(s),
                span,
            }),
            // `true`/`false` are no longer accepted surface spellings
            // (the parser sugar that rewrote them to `True`/`False` calls is
            // deleted); they are ordinary words now and resolve only if
            // imported.  `True`/`False` fall through to the generic
            // `Token::Word` arm below like any other word call.
            //
            // Slice 10c (R-P3-5): `if`/`else`/`end` was the last construct the
            // grammar knew. `if` is an ordinary `lib/` word now, spelled
            // postfix over two quotations (`[ T ] [ E ] if`), so `if` here is
            // just a `Call` like any other and `else`/`end` mean nothing at
            // all -- named explicitly rather than left to the generic
            // unknown-word error, since that is the diagnostic a source
            // written against the old grammar needs.
            Token::Word(w) if w == "end" || w == "else" => Err(format!(
                "parse error: `{w}` is not a word; `if` is an ordinary word taking two quotations (`~[ then ] ~[ else ] if`) at line {}, col {}",
                span.line, span.col
            )),
            Token::Word(w) => Ok(Term {
                kind: TermKind::Call(w),
                span,
            }),
            // R2: the term-level `[` is unambiguous against the type-level
            // `[` since every type-position bracket reader is reached only
            // from signature/type parsing, never from `parse_term`.
            //
            // Slice 6h (D1): a top-depth `;` before the matching `]` means an
            // array constructor rather than a quotation literal, mirroring
            // `quotation_type_ahead`'s depth scan. Once seen, the parse
            // commits to the constructor, so a malformed element/count is a
            // located, constructor-specific error rather than the generic
            // quotation-body one.
            Token::LBracket if self.array_ctor_ahead() => self.parse_array_ctor_term(span),
            Token::LBracket => {
                let annotation = self.parse_optional_quot_annotation()?;
                let body = self.parse_terms("`]` (unterminated quotation)", |tok| {
                    matches!(tok, Token::RBracket)
                })?;
                self.expect(Token::RBracket)?;
                Ok(Term {
                    kind: TermKind::Quotation(body, false, annotation),
                    span,
                })
            }
            // Slice 12 (R-C1): a `~[ ... ]` body literal, the inline-only
            // flavour. Mints the same `TermKind::Quotation` shape as the
            // ordinary `[ ... ]` arm above, with the flavour flag set; R-C2
            // requires this flavour to match the consuming parameter's
            // declared `Type::InlineQuotation`/`Type::Quotation` at each
            // argument-matching site.
            Token::TildeLBracket => {
                let annotation = self.parse_optional_quot_annotation()?;
                let body = self.parse_terms("`]` (unterminated quotation)", |tok| {
                    matches!(tok, Token::RBracket)
                })?;
                self.expect(Token::RBracket)?;
                Ok(Term {
                    kind: TermKind::Quotation(body, true, annotation),
                    span,
                })
            }
            // R3: a stray `]` with no opening `[`, parallel to the stray
            // `end`/`else` arm above.
            Token::RBracket => Err(format!(
                "parse error: `]` without a matching `[` at line {}, col {}",
                span.line, span.col
            )),
            other => Err(format!(
                "parse error: unexpected token {other:?} at line {}, col {}",
                span.line, span.col
            )),
        }
    }
}

fn is_word(tok: &Token, text: &str) -> bool {
    matches!(tok, Token::Word(w) if w == text)
}

/// A short, human-readable rendering of a token for a diagnostic (e.g. the
/// offending non-literal array count in X3): a word or numeric literal
/// renders as its source text, everything else falls back to `Debug`.
fn describe_token(tok: &Token) -> String {
    match tok {
        Token::Word(w) => w.clone(),
        Token::Int(n) => n.to_string(),
        Token::Float(v) => v.to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{EnumId, StructId};
    use crate::lexer::lex;

    fn parse_src(src: &str) -> Result<Module, String> {
        let tokens = lex(src).unwrap();
        parse(&tokens)
    }

    /// P7 slice 3i: `core::bool`'s declaration, for a fixture whose subject is
    /// the boolean type. `bool` is an ordinary declared enum now, and this path
    /// resolves no `import:`, so a source that names it declares it. Appended
    /// by `parse_src_with_bool`, never prepended, so a fixture's own line
    /// numbers stay the ones its diagnostics report.
    const BOOL_DEF: &str = "\ntype: Bool | False | True ;\n";

    fn parse_src_with_bool(src: &str) -> Result<Module, String> {
        parse_src(&format!("{src}{BOOL_DEF}"))
    }

    /// The parsed module's boolean type.
    fn bool_ty(module: &Module) -> Type {
        crate::ast::resolve_bool_type(&module.enums).expect("the fixture declares `Bool`")
    }

    /// The qualifier and selective names of a `Qualified` import, for the
    /// import-parsing tests below.
    fn qualified(imp: &Import) -> (&str, Vec<&str>) {
        match &imp.binding {
            ImportBinding::Qualified {
                qualifier,
                selective,
            } => (
                qualifier.as_str(),
                selective.iter().map(|(n, _)| n.as_str()).collect(),
            ),
            ImportBinding::Wildcard => panic!("expected a qualified import, got a wildcard"),
        }
    }

    fn scan_one_import(src: &str) -> Import {
        let tokens = lex(src).unwrap();
        let mut imports = scan_imports(&tokens).unwrap();
        assert_eq!(imports.len(), 1, "one import in {src:?}");
        imports.pop().unwrap()
    }

    /// U11 (R6/R7): the `import:` and `export:` forms parse into their records,
    /// including the optional selective-import name list. P8 slice 1a (OQ3):
    /// the target leads the form, the qualifier follows it.
    #[test]
    fn import_and_export_forms_parse() {
        let tokens =
            lex("import: \"lib/queue.sth\" queue | push pop | ;\nexport: Queue drain ;\n").unwrap();
        let imports = scan_imports(&tokens).unwrap();
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.target, ImportTarget::Path("lib/queue.sth".to_string()));
        assert_eq!(qualified(imp), ("queue", vec!["push", "pop"]));
        assert_eq!(imp.span.line, 1, "the import span locates `import:`");

        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let no_imports = HashMap::new();
        let bodies = parse_bodies(
            &tokens,
            &[],
            &[],
            0,
            &no_imports,
            &[],
            &no_imports,
            &[],
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
        )
        .unwrap();
        let exports: Vec<&str> = bodies.exports.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(exports, vec!["Queue", "drain"]);
    }

    /// Phase 5 slice 1 review fix: `parse_generic_typedefs`' stage (a) stamps
    /// `module: self.module` on the placeholder decl it registers, but every
    /// other test in this file drives `parse_bodies`
    /// with `module: 0`, so a hard-coded `0` there would pass unnoticed (this
    /// exact class of gap -- a span or id silently defaulting to module 0 --
    /// has bitten this codebase before). Drives `parse_bodies` directly with a
    /// non-zero module id and asserts both generic registries carry it.
    #[test]
    fn parse_generic_typedef_and_enum_stamp_the_parser_module_id() {
        let tokens =
            lex("type: Box 'T val 'T ; type: Result 'T 'E | Ok val 'T | Err val 'E ;\n").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let no_imports = HashMap::new();
        parse_bodies(
            &tokens,
            &[],
            &[],
            7,
            &no_imports,
            &[],
            &no_imports,
            &[],
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
        )
        .unwrap();
        assert_eq!(generics.structs[0].module, 7);
        assert_eq!(generics.enums[0].module, 7);
    }

    /// R9: a malformed `import:` (nothing at all after the keyword) is a
    /// located parse error. `import: q ;` is not the witness under the OQ3
    /// grammar: that is a legal Dependency-anchored import of module `q`.
    #[test]
    fn malformed_import_missing_target_is_located_error() {
        let tokens = lex("import: ;\n").unwrap();
        let err = scan_imports(&tokens).unwrap_err();
        assert!(err.contains("parse error"), "located parse error: {err}");
        assert!(err.contains("line 1"), "located parse error: {err}");
    }

    /// OQ3: an explicit qualifier after the target binds the given name.
    #[test]
    fn parse_import_explicit_qualifier_binds_given_name() {
        let imp = scan_one_import("import: core::cmp c ;\n");
        assert_eq!(qualified(&imp), ("c", vec![]));
    }

    /// OQ3: an elided qualifier defaults to the target's last segment.
    #[test]
    fn parse_import_omitted_qualifier_defaults_to_last_segment() {
        let imp = scan_one_import("import: core::cmp ;\n");
        assert_eq!(qualified(&imp), ("cmp", vec![]));
    }

    /// F2: a `self::` prefix records the SelfPackage anchor; the remaining
    /// segments are package-root-relative.
    #[test]
    fn parse_import_self_prefix_sets_self_anchor() {
        let imp = scan_one_import("import: self::text::ascii a ;\n");
        assert_eq!(
            imp.target,
            ImportTarget::Module(ModuleName {
                anchor: ImportAnchor::SelfPackage,
                segments: vec!["text".to_string(), "ascii".to_string()],
            })
        );
        assert_eq!(qualified(&imp), ("a", vec![]));
    }

    #[test]
    fn parse_import_omitted_qualifier_self_defaults_to_last_segment() {
        let imp = scan_one_import("import: self::text::ascii ;\n");
        assert_eq!(qualified(&imp), ("ascii", vec![]));
    }

    /// A bare first segment is Dependency-anchored: bare `self` (no `::`) is
    /// an ordinary package name, not the prefix.
    #[test]
    fn parse_import_bare_first_segment_is_dependency_anchored() {
        let imp = scan_one_import("import: self ;\n");
        assert_eq!(
            imp.target,
            ImportTarget::Module(ModuleName {
                anchor: ImportAnchor::Dependency,
                segments: vec!["self".to_string()],
            })
        );
    }

    /// OQ3: target, then `*`, then `;` is the wildcard shape.
    #[test]
    fn parse_import_bare_wildcard_builds_wildcard_variant() {
        let imp = scan_one_import("import: intrinsics * ;\n");
        assert_eq!(imp.binding, ImportBinding::Wildcard);
    }

    /// OQ3: `*` inside `| ... |` is the ordinary multiplication word being
    /// selectively imported, never the wildcard.
    #[test]
    fn parse_import_selective_list_star_is_literal_word() {
        let imp = scan_one_import("import: core::cmp | * | ;\n");
        assert_eq!(qualified(&imp), ("cmp", vec!["*"]));
    }

    /// OQ3: `*` followed by a `|` is an ordinary qualifier named `*`, not the
    /// wildcard shape -- the wildcard test above requires `*` then `;` with
    /// nothing between, so this and that test guard both halves of the
    /// disambiguation.
    #[test]
    fn parse_import_qualifier_star_before_selective_list_is_literal_qualifier() {
        let imp = scan_one_import("import: core::cmp * | a b | ;\n");
        assert_eq!(qualified(&imp), ("*", vec!["a", "b"]));
    }

    #[test]
    fn parse_import_selective_with_explicit_qualifier_ok() {
        let imp = scan_one_import("import: core::text s | split trim | ;\n");
        assert_eq!(qualified(&imp), ("s", vec!["split", "trim"]));
    }

    /// The manifest-less quoted-path form still parses, with the path moved to
    /// first position; an elided qualifier defaults to the file stem.
    #[test]
    fn parse_import_quoted_path_target_parses() {
        let imp = scan_one_import("import: \"lib/queue.sth\" ;\n");
        assert_eq!(imp.target, ImportTarget::Path("lib/queue.sth".to_string()));
        assert_eq!(qualified(&imp), ("queue", vec![]));
    }

    /// An empty `::` segment names no module and is a located parse error
    /// rather than an unresolvable target the driver has to describe.
    #[test]
    fn malformed_import_empty_segment_is_located_error() {
        let tokens = lex("import: core:: ;\n").unwrap();
        let err = scan_imports(&tokens).unwrap_err();
        assert!(
            err.contains("empty module-name segment") && err.contains("line 1"),
            "located parse error: {err}"
        );
    }

    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is, not by
    /// any compiler-known bit.
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";

    /// The terms of a word body.
    fn terms_body(word: &WordDef) -> &[Term] {
        &word.body
    }

    /// The names bound by a word's *entry* binding: the leading `Bind` term, if
    /// the body opens with one.
    fn entry_locals(word: &WordDef) -> &[String] {
        match terms_body(word).first().map(|t| &t.kind) {
            Some(TermKind::Bind(names)) => names,
            _ => &[],
        }
    }

    #[test]
    fn parse_gcd_shape_matches_ast() {
        let src = std::fs::read_to_string("examples/gcd.sth").unwrap();
        let module = parse_src(&src).unwrap();
        assert_eq!(module.words.len(), 2);

        let gcd = &module.words[0];
        assert_eq!(gcd.name, "gcd");
        let gcd_body = terms_body(gcd);
        assert_eq!(entry_locals(gcd), ["a", "b"]);
        assert_eq!(gcd.effect.inputs.len(), 2);
        assert_eq!(gcd.effect.outputs.len(), 1);

        // | a b | b 0 = [ a ] [ b a b mod gcd ] if
        assert_eq!(gcd_body.len(), 7);
        assert!(matches!(&gcd_body[0].kind, TermKind::Bind(_)));
        assert!(matches!(&gcd_body[1].kind, TermKind::Call(w) if w == "b"));
        assert!(matches!(&gcd_body[2].kind, TermKind::IntLit(0)));
        assert!(matches!(&gcd_body[3].kind, TermKind::Call(w) if w == "eq"));
        match &gcd_body[4].kind {
            TermKind::Quotation(then_branch, is_inline, _) => {
                assert_eq!(then_branch.len(), 1);
                assert!(is_inline, "gcd.sth writes `if`'s arms `~[ ... ]` (R-C3)");
                assert!(matches!(&then_branch[0].kind, TermKind::Call(w) if w == "a"));
            }
            other => panic!("expected the `then` quotation, got {other:?}"),
        }
        match &gcd_body[5].kind {
            TermKind::Quotation(else_branch, is_inline, _) => {
                assert_eq!(else_branch.len(), 5);
                assert!(is_inline, "gcd.sth writes `if`'s arms `~[ ... ]` (R-C3)");
            }
            other => panic!("expected the `else` quotation, got {other:?}"),
        }
        assert!(matches!(&gcd_body[6].kind, TermKind::Call(w) if w == "if"));

        let main = &module.words[1];
        assert_eq!(main.name, "main");
        assert!(entry_locals(main).is_empty());
    }

    #[test]
    fn parse_locals_block_populates_locals() {
        let src = std::fs::read_to_string("examples/lerp.sth").unwrap();
        let module = parse_src(&src).unwrap();
        let lerp = module.words.iter().find(|w| w.name == "lerp").unwrap();
        assert_eq!(entry_locals(lerp), ["a", "b", "t"]);
    }

    #[test]
    fn parse_mid_body_binding_produces_bind_term() {
        // R1: a `|` at a term position is a binding term, not a body prologue.
        let module = parse_src(": w ( -- i64 ) 5 | a | a ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 3);
        assert!(matches!(body[0].kind, TermKind::IntLit(5)));
        match &body[1].kind {
            TermKind::Bind(names) => assert_eq!(names, &["a"]),
            other => panic!("expected Bind, got {other:?}"),
        }
        assert!(matches!(&body[2].kind, TermKind::Call(w) if w == "a"));
    }

    #[test]
    fn parse_empty_binding_is_error() {
        let err = parse_src(": w ( -- ) | | ;").unwrap_err();
        assert!(err.contains("binds nothing"), "unexpected message: {err}");
    }

    #[test]
    fn parse_slot_resolves_i64_and_bool_expected() {
        let module = parse_src_with_bool(": w ( i64 Bool -- Bool ) drop ;").unwrap();
        let w = &module.words[0];
        assert_eq!(w.effect.inputs[0].ty, Type::I64);
        assert_eq!(w.effect.inputs[1].ty, bool_ty(&module));
        assert_eq!(w.effect.outputs[0].ty, bool_ty(&module));
    }

    #[test]
    fn parse_slot_resolves_new_int_widths_expected() {
        let module = parse_src(": w ( u8 i16 -- i32 u64 ) drop drop 0 0 ;").unwrap();
        let w = &module.words[0];
        assert_eq!(w.effect.inputs[0].ty, Type::from_name("u8").unwrap());
        assert_eq!(w.effect.inputs[1].ty, Type::from_name("i16").unwrap());
        assert_eq!(w.effect.outputs[0].ty, Type::from_name("i32").unwrap());
        assert_eq!(w.effect.outputs[1].ty, Type::from_name("u64").unwrap());
    }

    #[test]
    fn parse_slot_unknown_type_name_is_error() {
        let result = parse_src(": w ( foo -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("foo"), "unexpected message: {err}");
    }

    #[test]
    fn parse_true_false_construct_bool_variants() {
        let module = parse_src_with_bool(": w ( -- Bool Bool ) True False ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert!(matches!(&body[0].kind, TermKind::Call(w) if w == "True"));
        assert!(matches!(&body[1].kind, TermKind::Call(w) if w == "False"));
    }

    /// Slice 10c (E-P3-2): the `if`/`else`/`end` grammar is gone. `else`/`end`
    /// are not words, and a source written against the old grammar gets a
    /// diagnostic naming the replacement rather than a bare unknown word.
    #[test]
    fn parse_if_else_end_grammar_is_error() {
        let err = parse_src_with_bool(": w ( Bool -- i64 ) if 1 else 2 end ;").unwrap_err();
        assert!(err.contains("`else`"), "unexpected message: {err}");
        assert!(
            err.contains("~[ then ] ~[ else ] if"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn quotation_literal_parses_into_quotation_term() {
        // R1: `[ ... ]` parses into `TermKind::Quotation`, nested by
        // construction since the element list is `parse_terms`.
        let module = parse_src(": w ( -- ) [ 1 add ] drop [ [ ] ] drop ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 4);
        match &body[0].kind {
            TermKind::Quotation(terms, is_inline, _) => {
                assert_eq!(terms.len(), 2);
                assert!(!is_inline, "an ordinary `[ ... ]` literal");
                assert!(matches!(terms[0].kind, TermKind::IntLit(1)));
                assert!(matches!(&terms[1].kind, TermKind::Call(ref w) if w == "add"));
            }
            other => panic!("expected Quotation, got {other:?}"),
        }
        match &body[2].kind {
            TermKind::Quotation(outer, is_inline, _) => {
                assert_eq!(outer.len(), 1);
                assert!(!is_inline, "an ordinary `[ ... ]` literal");
                match &outer[0].kind {
                    TermKind::Quotation(inner, is_inline, _) => {
                        assert!(inner.is_empty());
                        assert!(!is_inline, "an ordinary `[ ... ]` literal");
                    }
                    other => panic!("expected nested Quotation, got {other:?}"),
                }
            }
            other => panic!("expected Quotation, got {other:?}"),
        }
    }

    /// Slice 12 (R-C1, X6): the new `Token::TildeLBracket` arm mints the same
    /// `TermKind::Quotation` shape as the ordinary `[ ... ]` arm, flagged
    /// `is_inline`, and a `~[ ... ]` in a body position no longer falls to the
    /// generic `other =>` "unexpected token" arm.
    #[test]
    fn tilde_quotation_literal_parses_into_an_inline_quotation_term() {
        let module = parse_src(": w ( -- ) ~[ 1 add ] drop ~[ ~[ ] ] drop ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 4);
        match &body[0].kind {
            TermKind::Quotation(terms, is_inline, _) => {
                assert_eq!(terms.len(), 2);
                assert!(is_inline, "a `~[ ... ]` literal");
                assert!(matches!(terms[0].kind, TermKind::IntLit(1)));
                assert!(matches!(&terms[1].kind, TermKind::Call(ref w) if w == "add"));
            }
            other => panic!("expected Quotation, got {other:?}"),
        }
        match &body[2].kind {
            TermKind::Quotation(outer, is_inline, _) => {
                assert_eq!(outer.len(), 1);
                assert!(is_inline, "a `~[ ... ]` literal");
                match &outer[0].kind {
                    TermKind::Quotation(inner, is_inline, _) => {
                        assert!(inner.is_empty());
                        assert!(is_inline, "a `~[ ... ]` literal");
                    }
                    other => panic!("expected nested Quotation, got {other:?}"),
                }
            }
            other => panic!("expected Quotation, got {other:?}"),
        }
    }

    /// Phase 6 slice 1 (D1/D4): the full four-part annotation reads into the
    /// literal's own `QuotAnnot`, concrete on both sides, leaving both rows
    /// and both name tables empty.
    #[test]
    fn parse_quotation_annotation_full_form_ok() {
        let module =
            parse_src_with_bool(": w ( -- ) [ ( i64 -- Bool ) dup 10 lt ] drop ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(terms, is_inline, Some(annot)) => {
                assert_eq!(terms.len(), 3, "the body is read by the untouched reader");
                assert!(!is_inline, "an ordinary `[ ... ]` literal");
                assert_eq!(annot.inputs, vec![PolyType::Concrete(Type::I64)]);
                assert_eq!(annot.outputs, vec![PolyType::Concrete(bool_ty(&module))]);
                assert_eq!((annot.row_in, annot.row_out), (None, None));
                assert!(annot.ty_var_names.is_empty());
                assert!(annot.row_var_names.is_empty());
            }
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    /// The `~[ ... ]` flavour reads the same annotation and keeps its flag.
    #[test]
    fn parse_quotation_annotation_inline_flavour_ok() {
        let module =
            parse_src_with_bool(": w ( -- ) ~[ ( i64 -- Bool ) dup 10 lt ] drop ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(terms, is_inline, Some(annot)) => {
                assert_eq!(terms.len(), 3);
                assert!(is_inline, "a `~[ ... ]` literal");
                assert_eq!(annot.inputs, vec![PolyType::Concrete(Type::I64)]);
                assert_eq!(annot.outputs, vec![PolyType::Concrete(bool_ty(&module))]);
            }
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    /// D1's additive-parse guard: a literal with no leading `(` parses exactly
    /// as before, carrying no annotation.
    #[test]
    fn parse_quotation_no_annotation_unchanged() {
        let module = parse_src(": w ( -- ) [ dup 10 lt ] drop ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(terms, _, annotation) => {
                assert_eq!(terms.len(), 3);
                assert!(annotation.is_none(), "an unannotated literal");
            }
            other => panic!("expected Quotation, got {other:?}"),
        }
    }

    /// R6: only the full form parses, so a parenthesized list reaching `)`
    /// with no `--` is a located error rather than an elided effect.
    #[test]
    fn parse_quotation_annotation_missing_arrow_is_error() {
        let err = parse_src_with_bool(": w ( -- ) [ ( i64 Bool ) dup 10 lt ] drop ;").unwrap_err();
        assert!(err.contains("( inputs -- outputs )"), "unexpected: {err}");
        assert!(err.contains("line 1, col 25"), "unexpected: {err}");
    }

    /// R6: the empty `( )` is the same rejection, not a nil effect.
    #[test]
    fn parse_quotation_annotation_elided_is_error() {
        let err = parse_src(": w ( -- ) [ ( ) dup ] drop ;").unwrap_err();
        assert!(err.contains("( inputs -- outputs )"), "unexpected: {err}");
        assert!(err.contains("line 1, col 16"), "unexpected: {err}");
    }

    /// Phase 6 slice 3b (R1/R2): the lone-variant-name elided form records the
    /// bare tag in owning mode and declares *no* input slot -- the variant type
    /// the arm receives is the checker's to synthesize, from the scrutinee's
    /// own enum. The arrow and outputs are both elided.
    #[test]
    fn parse_quotation_annotation_variant_tag_owning_ok() {
        let module = parse_src(
            "type: Shape | Circle | Rect w i64 h i64 ; : w ( -- ) [ ( Circle ) drop ] drop ;",
        )
        .unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(_, _, Some(annot)) => {
                assert_eq!(
                    annot.variant_tag,
                    Some(VariantTag {
                        name: "Circle".to_string(),
                        mode: VariantTagMode::Owning,
                    })
                );
                assert!(annot.inputs.is_empty(), "unexpected: {:?}", annot.inputs);
                assert!(annot.outputs.is_empty());
                assert_eq!((annot.row_in, annot.row_out), (None, None));
            }
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    /// Phase 6 slice 3b (R1/R2, decision 6): the `&!`-prefixed elided form
    /// records the same bare name (the sigil must not leak into the routing
    /// name) with the mutable-reference mode, and still declares no input slot
    /// -- no `Type::Ref` is interned at parse time.
    #[test]
    fn parse_quotation_annotation_variant_tag_mut_ref_ok() {
        let module = parse_src(
            "type: Shape | Circle | Rect w i64 h i64 ; : w ( -- ) [ ( &!Circle ) drop ] drop ;",
        )
        .unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(_, _, Some(annot)) => {
                assert_eq!(
                    annot.variant_tag,
                    Some(VariantTag {
                        name: "Circle".to_string(),
                        mode: VariantTagMode::RefMut,
                    })
                );
                assert!(annot.inputs.is_empty(), "unexpected: {:?}", annot.inputs);
            }
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    /// Phase 6 slice 3 (R1): a partial arm with a second token and no arrow
    /// is the same located elided-form error as a bare `( )`, not accepted
    /// as a partial arm.
    #[test]
    fn parse_quotation_annotation_variant_tag_extra_token_no_arrow_is_error() {
        let err = parse_src_with_bool(
            "type: Shape | Circle | Rect w i64 h i64 ; : w ( -- ) [ ( Circle Bool ) drop ] drop ;",
        )
        .unwrap_err();
        assert!(err.contains("( inputs -- outputs )"), "unexpected: {err}");
    }

    /// Review fix (blocker 1, part 2): an in-scope struct keeps precedence
    /// over a variant of the same name -- recognition must not hijack a name
    /// ordinary type resolution already owns. Parses a body against a struct
    /// `Circle` and an enum `Shape` whose variant is also `Circle`, in scope
    /// together (`parse_src` has no way to declare both without one shadowing
    /// the other in a single `type:` prepass).
    fn parse_body_with_a_struct_and_a_variant_named_circle(src: &str) -> Result<QuotAnnot, String> {
        let circle_static: &'static str = Box::leak("Circle".to_string().into_boxed_str());
        let shape_static: &'static str = Box::leak("Shape".to_string().into_boxed_str());
        let structs = vec![StructDecl {
            name: "Circle".to_string(),
            name_static: circle_static,
            fields: Vec::new(),
            span: Span::default(),
            has_drop_overload: false,
            is_bundle: false,
            module: 0,
        }];
        let enums = vec![EnumDecl {
            name: "Shape".to_string(),
            name_static: shape_static,
            variants: vec![VariantDecl {
                name: "Circle".to_string(),
                name_static: circle_static,
                display_static: "Shape.Circle",
                fields: Vec::new(),
                span: Span::default(),
            }],
            span: Span::default(),
            module: 0,
        }];
        let tokens = lex(src).unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let no_imports = HashMap::new();
        let bodies = parse_bodies(
            &tokens,
            &structs,
            &enums,
            0,
            &no_imports,
            &[],
            &no_imports,
            &[],
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
        )?;
        match &terms_body(&bodies.words[0])[0].kind {
            TermKind::Quotation(_, _, Some(annot)) => Ok(annot.clone()),
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    #[test]
    fn parse_leading_variant_slot_struct_of_same_name_takes_precedence() {
        let annot = parse_body_with_a_struct_and_a_variant_named_circle(
            ": w ( -- ) [ ( Circle -- ) drop ] drop ;",
        )
        .unwrap();
        assert_eq!(annot.variant_tag, None);
        assert_eq!(
            annot.inputs,
            vec![PolyType::Concrete(Type::Struct(StructId(0), "Circle"))]
        );
    }

    /// Slice 3b (R1/OQ1): the escalated spelling resolves the same way -- the
    /// struct takes the leading slot and the declared outputs are the ones
    /// after the arrow, no tag anywhere.
    #[test]
    fn parse_leading_variant_slot_struct_of_same_name_takes_precedence_with_outputs() {
        let annot = parse_body_with_a_struct_and_a_variant_named_circle(
            ": w ( -- ) [ ( Circle -- i64 ) drop 1 ] drop ;",
        )
        .unwrap();
        assert_eq!(annot.variant_tag, None);
        assert_eq!(
            annot.inputs,
            vec![PolyType::Concrete(Type::Struct(StructId(0), "Circle"))]
        );
        assert_eq!(annot.outputs, vec![PolyType::Concrete(Type::I64)]);
    }

    /// The corollary of the two above: only a *tag* may elide the arrow, so a
    /// leading token the struct claimed leaves `( Circle )` the ordinary
    /// missing-arrow rejection rather than a one-slot arm annotation.
    #[test]
    fn parse_leading_variant_slot_struct_of_same_name_may_not_elide_the_arrow() {
        let err = parse_body_with_a_struct_and_a_variant_named_circle(
            ": w ( -- ) [ ( Circle ) drop ] drop ;",
        )
        .unwrap_err();
        assert!(err.contains("( inputs -- outputs )"), "unexpected: {err}");
    }

    /// Review fix (blocker 1, part 1): a variant declared in another module,
    /// not imported here, must not resolve as this module's routing tag --
    /// the pre-fix bug let any variant name anywhere in the program capture
    /// every annotation's leading slot. Module 0 has no `Circle` of any kind,
    /// so the leading token falls through to ordinary annotation parsing,
    /// which reports it as an unknown type rather than routing to module 1's
    /// variant.
    #[test]
    fn parse_leading_variant_slot_other_module_variant_is_not_visible() {
        let circle_static: &'static str = Box::leak("Circle".to_string().into_boxed_str());
        let shape_static: &'static str = Box::leak("Shape".to_string().into_boxed_str());
        let enums = vec![EnumDecl {
            name: "Shape".to_string(),
            name_static: shape_static,
            variants: vec![VariantDecl {
                name: "Circle".to_string(),
                name_static: circle_static,
                display_static: "Shape.Circle",
                fields: Vec::new(),
                span: Span::default(),
            }],
            span: Span::default(),
            module: 1,
        }];
        let tokens = lex(": w ( -- ) [ ( Circle ) drop ] drop ;").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let no_imports = HashMap::new();
        let result = parse_bodies(
            &tokens,
            &[],
            &enums,
            0,
            &no_imports,
            &[],
            &no_imports,
            &[],
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
        );
        let err = match result {
            Ok(_) => panic!("expected an unknown-type error"),
            Err(e) => e,
        };
        assert!(err.contains("unknown type `Circle`"), "unexpected: {err}");
    }

    /// The generic half of the rule above (R1/F1). `module_declares_variant`
    /// searches the generic enum registry too, and that search is scoped by
    /// the same `module` filter -- an unimported generic enum's variant name
    /// is no more visible as a routing tag than a concrete one's.
    #[test]
    fn parse_leading_variant_slot_other_module_generic_variant_is_not_visible() {
        let tokens = lex(": w ( -- ) [ ( Circle ) drop ] drop ;").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        generics.enums.push(crate::ast::GenericEnumDecl {
            name: "Shape".to_string(),
            ty_var_names: vec!["T".to_string()],
            variants: vec![crate::ast::GenericVariantDecl {
                name: "Circle".to_string(),
                fields: vec![("r".to_string(), PolyType::Var(0))],
                span: Span::default(),
            }],
            span: Span::default(),
            module: 1,
        });
        let no_imports = HashMap::new();
        let result = parse_bodies(
            &tokens,
            &[],
            &[],
            0,
            &no_imports,
            &[],
            &no_imports,
            &[],
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
        );
        let err = match result {
            Ok(_) => panic!("expected an unknown-type error"),
            Err(e) => e,
        };
        assert!(err.contains("unknown type `Circle`"), "unexpected: {err}");
    }

    /// R2: a row spelling is admitted and interned into the literal's own row
    /// table; the same name on both sides is one id (a passthrough row).
    #[test]
    fn parse_quotation_annotation_row_ok() {
        let module = parse_src(": w ( -- ) [ ( ..a i64 -- ..a ) drop ] drop ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(_, _, Some(annot)) => {
                assert_eq!(annot.row_var_names, vec!["..a".to_string()]);
                assert_eq!((annot.row_in, annot.row_out), (Some(0), Some(0)));
                assert_eq!(annot.inputs, vec![PolyType::Concrete(Type::I64)]);
                assert!(annot.outputs.is_empty());
            }
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    /// R2: a shape-changing row is two distinct per-literal ids (the checker,
    /// not the parser, decides what a standalone one means).
    #[test]
    fn parse_quotation_annotation_distinct_rows_are_distinct_ids() {
        let module = parse_src(": w ( -- ) [ ( ..a -- ..b ) drop ] drop ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Quotation(_, _, Some(annot)) => {
                assert_eq!(
                    annot.row_var_names,
                    vec!["..a".to_string(), "..b".to_string()]
                );
                assert_eq!((annot.row_in, annot.row_out), (Some(0), Some(1)));
            }
            other => panic!("expected an annotated Quotation, got {other:?}"),
        }
    }

    /// R2: type variables are minted into a **per-literal** space, so two
    /// sibling literals each start at `Var(0)` and neither borrows the
    /// enclosing word's `PolySig`.
    #[test]
    fn parse_quotation_annotation_ty_vars_are_per_literal() {
        let module =
            parse_src(": w ( -- ) [ ( 'T 'U -- 'T ) drop ] drop [ ( 'X -- 'X ) ] drop ;").unwrap();
        let body = terms_body(&module.words[0]);
        match (&body[0].kind, &body[2].kind) {
            (TermKind::Quotation(_, _, Some(first)), TermKind::Quotation(_, _, Some(second))) => {
                assert_eq!(first.ty_var_names, vec!["'T".to_string(), "'U".to_string()]);
                assert_eq!(
                    first.inputs,
                    vec![PolyType::Var(0), PolyType::Var(1)],
                    "a repeated `'T` reuses its id"
                );
                assert_eq!(first.outputs, vec![PolyType::Var(0)]);
                assert_eq!(second.ty_var_names, vec!["'X".to_string()]);
                assert_eq!(second.inputs, vec![PolyType::Var(0)]);
            }
            other => panic!("expected two annotated Quotations, got {other:?}"),
        }
    }

    #[test]
    fn unterminated_quotation_is_located_parse_error() {
        let result = parse_src(": w ( -- ) [ 1 add");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unexpected end of input"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("unterminated quotation"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn stray_closing_bracket_is_located_parse_error() {
        let result = parse_src(": w ( -- ) 1 ] drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("`]`"), "unexpected message: {err}");
        assert!(
            err.contains("without a matching `[`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_missing_semicolon_is_error() {
        let result = parse_src(": w ( -- ) 1");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unexpected end of input"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`;`"), "unexpected message: {err}");
    }

    #[test]
    fn parse_end_without_if_is_error() {
        let result = parse_src(": w ( -- ) end ;");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("end"));
    }

    /// The retired surface spelling `>=` still lexes as an ordinary word
    /// (nothing reserves it): it is spelled with a leading `>`, and
    /// `conversion_target_name`'s hand-written carve-out (R8) keeps
    /// `check_operator`'s `>T` conversion prefix test from claiming it.
    /// Without that carve-out a bare `>=` reads as a conversion to a type
    /// named `=`, reported as an unknown *type*; with it, `>=` falls through
    /// to the ordinary word lookup and is reported as an unknown *word*
    /// (`gte` is its bound replacement, R-P3-3/Decision 2).
    #[test]
    fn ge_is_not_read_as_a_type_conversion() {
        let module = parse_src_with_bool(": w ( i64 i64 -- Bool ) >= ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert!(matches!(&body[0].kind, TermKind::Call(w) if w == ">="));
        let err = crate::check::check(
            &mut parse_src_with_bool(": w ( i64 i64 -- Bool ) >= ;\n: main ( -- ) 1 2 w drop ;")
                .unwrap(),
        )
        .unwrap_err();
        assert!(
            err.contains("unknown word `>=`"),
            "the carve-out must route `>=` to the word lookup, not a type conversion: {err}"
        );
    }

    /// Slice 10c (E-P3-2): `if` is an ordinary word now, so it opens nothing
    /// and there is no unterminated form to report. What used to be
    /// `parse_then_no_longer_closes_if` / `..._unterminated_if_...` — both
    /// guards on the deleted grammar's terminator handling — becomes this: the
    /// bare word parses, and the arity failure is the checker's, at the call
    /// site, not the parser's.
    #[test]
    fn parse_bare_if_is_an_ordinary_call() {
        let module = parse_src(": w ( i64 -- i64 ) if ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0].kind, TermKind::Call(w) if w == "if"));
    }

    fn parse_line_src(src: &str) -> Result<Line, String> {
        let tokens = lex(src).unwrap();
        parse_line(&tokens)
    }

    #[test]
    fn parse_line_bare_expression_is_expr() {
        match parse_line_src("2 3 add").unwrap() {
            Line::Expr(terms) => {
                assert_eq!(terms.len(), 3);
                assert!(matches!(terms[0].kind, TermKind::IntLit(2)));
                assert!(matches!(&terms[2].kind, TermKind::Call(w) if w == "add"));
            }
            other => panic!("expected Expr, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_float_lit_is_float_lit() {
        match parse_line_src("2.5").unwrap() {
            Line::Expr(terms) => {
                assert_eq!(terms.len(), 1);
                assert!(matches!(terms[0].kind, TermKind::FloatLit(v) if v == 2.5));
            }
            other => panic!("expected Expr, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_colon_is_def() {
        match parse_line_src(": sq ( i64 -- i64 ) dup mul ;").unwrap() {
            Line::Def(def) => assert_eq!(def.name, "sq"),
            other => panic!("expected Def, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_trailing_tokens_after_def_is_error() {
        let result = parse_line_src(": sq ( i64 -- i64 ) dup mul ; 5 sq");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("after `;`"), "unexpected message: {err}");
    }

    #[test]
    fn parse_line_unterminated_def_is_error() {
        let result = parse_line_src(": sq ( i64 -- i64 ) dup mul");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unexpected end of input"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`;`"), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_flat_struct_registers_fields() {
        let module = parse_src("type: Vec2 x i64 y i64 ;").unwrap();
        assert_eq!(module.structs.len(), 1);
        let decl = &module.structs[0];
        assert_eq!(decl.name, "Vec2");
        assert_eq!(decl.name_static, "Vec2");
        assert_eq!(decl.fields.len(), 2);
        assert_eq!(decl.fields[0], ("x".to_string(), Type::I64));
        assert_eq!(decl.fields[1], ("y".to_string(), Type::I64));
    }

    #[test]
    fn parse_typedef_zero_field_struct_registers_empty_fields() {
        let module = parse_src("type: Unit ;").unwrap();
        assert_eq!(module.structs.len(), 1);
        assert_eq!(module.structs[0].name, "Unit");
        assert!(module.structs[0].fields.is_empty());
    }

    #[test]
    fn parse_typedef_field_may_reference_a_struct_declared_later() {
        let module =
            parse_src("type: Segment from Vec2 to Vec2 ; type: Vec2 x i64 y i64 ;").unwrap();
        assert_eq!(module.structs.len(), 2);
        let segment = &module.structs[0];
        assert_eq!(segment.name, "Segment");
        match segment.fields[0].1 {
            Type::Struct(_, name) => assert_eq!(name, "Vec2"),
            other => panic!("expected Type::Struct(Vec2), got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_self_referential_field_resolves_to_own_type() {
        let module = parse_src("type: Loop next Loop ;").unwrap();
        assert_eq!(module.structs.len(), 1);
        match module.structs[0].fields[0].1 {
            Type::Struct(_, name) => assert_eq!(name, "Loop"),
            other => panic!("expected Type::Struct(Loop), got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_used_in_word_effect_resolves() {
        let module = parse_src("type: Vec2 x i64 y i64 ; : id ( Vec2 -- Vec2 ) ;").unwrap();
        let id = &module.words[0];
        match id.effect.inputs[0].ty {
            Type::Struct(_, name) => assert_eq!(name, "Vec2"),
            other => panic!("expected Type::Struct(Vec2), got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_odd_field_token_count_is_error() {
        let result = parse_src("type: Bad x i64 y ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("odd field-token count"),
            "unexpected message: {err}"
        );
        assert!(err.contains('y'), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_missing_semicolon_is_error() {
        let result = parse_src("type: Bad x i64");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unexpected end of input"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`;`"), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_delimiter_field_type_is_error() {
        let result = parse_src("type: Bad x ( ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("expected a word"), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_colon_field_type_is_error() {
        let result = parse_src("type: Bad x : ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("expected a field type"),
            "unexpected message: {err}"
        );
        assert!(err.contains(':'), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_unknown_field_type_is_error() {
        let result = parse_src("type: Bad x Nope ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("Nope"), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_enum_with_leading_pipe_registers_variants() {
        let module = parse_src("type: Shape | Circle r f64 | Rect w f64 h f64 ;").unwrap();
        assert!(module.structs.is_empty());
        // P7 slice 3i (R5): no registry slot is reserved for `bool` any more, so
        // a source module's first declared enum lands at index 0 and its
        // registry holds exactly what the source declared.
        assert_eq!(module.enums.len(), 1);
        let shape = &module.enums[0];
        assert_eq!(shape.name, "Shape");
        assert_eq!(shape.variants.len(), 2);
        assert_eq!(shape.variants[0].name, "Circle");
        // Phase 6 slice 2 (R1): the leaked `Enum.Variant` display name that a
        // `Type::Variant` renders, built once here at declaration time.
        assert_eq!(shape.variants[0].display_static, "Shape.Circle");
        assert_eq!(shape.variants[0].fields, vec![("r".to_string(), Type::F64)]);
        assert_eq!(shape.variants[1].name, "Rect");
        assert_eq!(shape.variants[1].display_static, "Shape.Rect");
        assert_eq!(
            shape.variants[1].fields,
            vec![("w".to_string(), Type::F64), ("h".to_string(), Type::F64)]
        );
    }

    #[test]
    fn parse_typedef_enum_without_leading_pipe_registers_first_variant() {
        let module = parse_src("type: MaybeInt None | Some v i64 ;").unwrap();
        assert_eq!(module.enums.len(), 1);
        let maybe = &module.enums[0];
        assert_eq!(maybe.variants.len(), 2);
        assert_eq!(maybe.variants[0].name, "None");
        assert!(maybe.variants[0].fields.is_empty());
        assert_eq!(maybe.variants[1].name, "Some");
        assert_eq!(maybe.variants[1].fields, vec![("v".to_string(), Type::I64)]);
    }

    #[test]
    fn parse_typedef_enum_single_variant_newtype_ok() {
        // M3: a single-variant enum is allowed.
        let module = parse_src("type: Id | Wrap v i64 ;").unwrap();
        assert_eq!(module.enums.len(), 1);
        assert_eq!(module.enums[0].variants.len(), 1);
    }

    #[test]
    fn parse_typedef_enum_zero_variants_is_error() {
        // M3: a `|`-bearing body with no variant name is malformed.
        let result = parse_src("type: Empty | ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("malformed"), "unexpected message: {err}");
        assert!(err.contains("zero variants"), "unexpected message: {err}");
        assert!(
            err.contains("Empty"),
            "diagnostic should name the type: {err}"
        );
    }

    #[test]
    fn parse_typedef_enum_odd_field_token_count_is_error() {
        let result = parse_src("type: Bad | V x i64 y | Other ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("odd field-token count"),
            "unexpected message: {err}"
        );
        assert!(err.contains('y'), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_enum_unknown_variant_field_type_is_error() {
        let result = parse_src("type: Bad | V x Nope ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("Nope"), "unexpected message: {err}");
    }

    #[test]
    fn parse_typedef_enum_self_referential_field_resolves_to_own_type() {
        let module = parse_src("type: Loop | Next n Loop | Stop ;").unwrap();
        assert_eq!(module.enums.len(), 1);
        match module.enums[0].variants[0].fields[0].1 {
            Type::Enum(_, name) => assert_eq!(name, "Loop"),
            other => panic!("expected Type::Enum(Loop), got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_enum_used_in_word_effect_resolves() {
        let module = parse_src("type: Shape | Circle r f64 ; : id ( Shape -- Shape ) ;").unwrap();
        let id = &module.words[0];
        match id.effect.inputs[0].ty {
            Type::Enum(_, name) => assert_eq!(name, "Shape"),
            other => panic!("expected Type::Enum(Shape), got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_struct_and_enum_coexist_in_source_order() {
        let module =
            parse_src("type: Vec2 x i64 y i64 ; type: Shape | Circle r f64 | Rect w f64 h f64 ;")
                .unwrap();
        assert_eq!(module.structs.len(), 1);
        assert_eq!(module.structs[0].name, "Vec2");
        assert_eq!(module.enums.len(), 1);
        assert_eq!(module.enums[0].name, "Shape");
    }

    #[test]
    fn parse_generic_typedef_single_var_registers_decl() {
        let module = parse_src("type: Box 'T val 'T ;").unwrap();
        // A generic header mints no concrete struct entry (R1/D5): only the
        // side registry gains an entry.
        assert!(module.structs.is_empty());
        assert_eq!(module.generic_structs.len(), 1);
        let decl = &module.generic_structs[0];
        assert_eq!(decl.name, "Box");
        assert_eq!(decl.ty_var_names, ["'T"]);
        assert_eq!(decl.fields.len(), 1);
        assert_eq!(decl.fields[0].0, "val");
        assert_eq!(decl.fields[0].1, PolyType::Var(0));
    }

    #[test]
    fn parse_generic_typedef_multi_var_binds_each_field_to_its_own_variable() {
        let module = parse_src("type: Pair 'A 'B a 'A b 'B ;").unwrap();
        assert_eq!(module.generic_structs.len(), 1);
        let decl = &module.generic_structs[0];
        assert_eq!(decl.name, "Pair");
        assert_eq!(decl.ty_var_names, ["'A", "'B"]);
        assert_eq!(decl.fields[0], ("a".to_string(), PolyType::Var(0)));
        assert_eq!(decl.fields[1], ("b".to_string(), PolyType::Var(1)));
    }

    #[test]
    fn parse_generic_typedef_concrete_field_resolves_alongside_a_variable_field() {
        let module = parse_src("type: Wrap 'T tag i64 val 'T ;").unwrap();
        let decl = &module.generic_structs[0];
        assert_eq!(
            decl.fields[0],
            ("tag".to_string(), PolyType::Concrete(Type::I64))
        );
        assert_eq!(decl.fields[1], ("val".to_string(), PolyType::Var(0)));
    }

    #[test]
    fn parse_generic_typedef_field_naming_unbound_variable_is_error() {
        // R1: `'E` is never bound by `Box`'s header.
        let result = parse_src("type: Box 'T val 'E ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("'E"), "unexpected message: {err}");
        assert!(err.contains("Box"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 18"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_typedef_phantom_variable_is_error() {
        // R1 (round 2): `'T` is bound but never used in any field.
        let result = parse_src("type: Phantom 'T x i64 ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("phantom"), "unexpected message: {err}");
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Phantom"), "unexpected message: {err}");
        // The location points at the binding site, not at the declaration.
        assert!(err.contains("line 1, col 15"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_typedef_header_binding_same_variable_twice_is_error() {
        // Phase 5 slice 1 review fix (round 3): without a dedicated check, the
        // second `'T` shadows nothing (the header has no scoping), so a field
        // referencing `'T` used to resolve against the first entry and leave
        // the second entry's `used` flag false, misreporting this as a
        // phantom-variable error rather than naming the real fault.
        let result = parse_src("type: Bad 'T 'T x 'T ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("bound twice"), "unexpected message: {err}");
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("Bad"), "unexpected message: {err}");
        // Col 14 is the *second* `'T`, the one that is the duplicate.
        assert!(err.contains("line 1, col 14"), "unlocated: {err}");
    }

    #[test]
    fn parse_typedef_tick_prefixed_field_name_is_error() {
        // Phase 5 slice 1 review fix: `'`-prefixed words are type variables,
        // so they are rejected at every field-name position. Before this,
        // `'y` was accepted as an ordinary field name here while the same
        // spelling directly after the type name was read as a type parameter.
        let result = parse_src("type: Foo x i64 'y i64 ;");
        let err = result.unwrap_err();
        assert!(err.contains("'y"), "unexpected message: {err}");
        assert!(err.contains("type variable"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 17"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_variant_unbound_type_variable_is_error() {
        // Not the field-name gate: since positional fields landed, a trailing
        // `'z` in a variant body opens a positional field, so what rejects
        // this is `'z` not being bound by the `E 'T` header. Binding it (`type:
        // E 'T 'z | Ok v 'T 'z ;`) parses clean.
        let result = parse_src("type: E 'T | Ok v 'T 'z ;");
        let err = result.unwrap_err();
        assert!(err.contains("'z"), "unexpected message: {err}");
        assert!(err.contains("bound by"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 22"), "unlocated: {err}");
        assert!(
            parse_src("type: E 'T 'z | Ok v 'T 'z ;").is_ok(),
            "binding `'z` in the header should make the same body legal"
        );
    }

    #[test]
    fn parse_generic_typedef_odd_field_count_names_the_generic_header() {
        // `'bar` reads as a type parameter, so the trailing `i64` is a field
        // name with no type. The plain odd-field-count message would name
        // `i64`, a token the author never got wrong; this one says the header
        // was read as generic over `'bar`.
        let result = parse_src("type: Foo 'bar i64 ;");
        let err = result.unwrap_err();
        assert!(
            err.contains("generic `type: Foo 'bar`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("line 1, col 20"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_registers_decl() {
        let module = parse_src("type: Result 'T 'E | Ok val 'T | Err val 'E ;").unwrap();
        // A generic header mints no concrete enum entry at all (R1/D5).
        assert!(module.enums.is_empty());
        assert_eq!(module.generic_enums.len(), 1);
        let decl = &module.generic_enums[0];
        assert_eq!(decl.name, "Result");
        assert_eq!(decl.ty_var_names, ["'T", "'E"]);
        assert_eq!(decl.variants.len(), 2);
        assert_eq!(decl.variants[0].name, "Ok");
        assert_eq!(
            decl.variants[0].fields,
            [("val".to_string(), PolyType::Var(0))]
        );
        assert_eq!(decl.variants[1].name, "Err");
        assert_eq!(
            decl.variants[1].fields,
            [("val".to_string(), PolyType::Var(1))]
        );
    }

    #[test]
    fn parse_generic_enum_typedef_without_leading_pipe_registers_first_variant() {
        let module = parse_src("type: Maybe 'T None | Some v 'T ;").unwrap();
        let decl = &module.generic_enums[0];
        assert_eq!(decl.variants[0].name, "None");
        assert!(decl.variants[0].fields.is_empty());
        assert_eq!(decl.variants[1].name, "Some");
        assert_eq!(
            decl.variants[1].fields,
            [("v".to_string(), PolyType::Var(0))]
        );
    }

    #[test]
    fn parse_generic_enum_typedef_field_naming_unbound_variable_is_error() {
        let result = parse_src("type: Result 'T 'E | Ok val 'T | Err val 'X ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("'X"), "unexpected message: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_phantom_variable_is_error() {
        let result = parse_src("type: Result 'T 'E | Ok val 'T | Err other i64 ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("phantom"), "unexpected message: {err}");
        assert!(err.contains("'E"), "unexpected message: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_zero_variants_is_error() {
        let result = parse_src("type: Empty 'T | ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("zero variants"), "unexpected message: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_reserved_variant_name_is_error() {
        // Phase 5 slice 1 review fix: the module-level pre-pass skips a
        // generic header entirely, so the reserved-name gate on a variant
        // must run inside `parse_generic_variant_fields` itself, not rely on
        // the pre-pass having already caught it.
        let result = parse_src("type: Bad 'T | ^Evil val 'T ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("^Evil"), "unexpected message: {err}");
        assert!(err.contains("reserved"), "unexpected message: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_attributeless_variants_parse_positionally() {
        // OQ4/Phase 1: `Option`'s exact shape -- a zero-field variant
        // (`None`) and a one-field variant with no leading field name
        // (`Some 'T`).
        let module = parse_src("type: Option 'T | None | Some 'T ;").unwrap();
        let decl = &module.generic_enums[0];
        assert_eq!(decl.variants[0].name, "None");
        assert!(decl.variants[0].fields.is_empty());
        assert_eq!(decl.variants[1].name, "Some");
        assert_eq!(decl.variants[1].fields.len(), 1);
        assert_eq!(decl.variants[1].fields[0].1, PolyType::Var(0));
        // The placeholder name is not a word the lexer can ever produce (it
        // contains a space), so it can never be referenced as an accessor
        // and never collides with a real field name.
        assert!(decl.variants[1].fields[0].0.contains(' '));
    }

    #[test]
    fn parse_generic_enum_typedef_attributeless_two_var_variants_parse() {
        // `Result`'s exact shape: both arms attributeless.
        let module = parse_src("type: Result 'T 'E | Ok 'T | Err 'E ;").unwrap();
        let decl = &module.generic_enums[0];
        assert_eq!(decl.variants[0].name, "Ok");
        assert_eq!(decl.variants[0].fields.len(), 1);
        assert_eq!(decl.variants[0].fields[0].1, PolyType::Var(0));
        assert_eq!(decl.variants[1].name, "Err");
        assert_eq!(decl.variants[1].fields.len(), 1);
        assert_eq!(decl.variants[1].fields[0].1, PolyType::Var(1));
    }

    #[test]
    fn parse_generic_enum_typedef_mixed_named_and_attributeless_fields_parses() {
        // A named field followed by an attributeless field in the same
        // variant: each field position is disambiguated independently (a
        // leading `'` always opens an attributeless field, regardless of
        // what came before it in the same variant), so this is accepted
        // rather than rejected.
        let module = parse_src("type: E 'T 'U | V val 'T 'U ;").unwrap();
        let decl = &module.generic_enums[0];
        assert_eq!(decl.variants[0].name, "V");
        assert_eq!(decl.variants[0].fields.len(), 2);
        assert_eq!(decl.variants[0].fields[0].0, "val");
        assert_eq!(decl.variants[0].fields[0].1, PolyType::Var(0));
        assert_ne!(decl.variants[0].fields[1].0, "val");
        assert_eq!(decl.variants[0].fields[1].1, PolyType::Var(1));
    }

    #[test]
    fn parse_generic_typedef_declared_but_never_used_parses_clean() {
        // Phase 1: a generic type declared and never applied parses without
        // error alongside an ordinary word def. The end-to-end claim (a whole
        // program with this shape builds and runs) is the actual golden,
        // `tests/phase5_slice1.rs::generic_type_declared_but_never_used_builds_and_runs`.
        let module = parse_src("type: Box 'T val 'T ; : main ( -- ) ;").unwrap();
        assert_eq!(module.generic_structs.len(), 1);
        assert!(module.words.iter().any(|w| w.name == "main"));
    }

    #[test]
    fn parse_generic_typedef_does_not_shadow_a_concrete_typedef_registered_after_it() {
        // A generic header is skipped entirely by the concrete pre-pass, so a
        // concrete `type:` elsewhere in the file is unaffected.
        let module = parse_src("type: Box 'T val 'T ; type: Vec2 x i64 y i64 ;").unwrap();
        assert_eq!(module.structs.len(), 1);
        assert_eq!(module.structs[0].name, "Vec2");
        assert_eq!(module.generic_structs.len(), 1);
    }

    // -- P7.S3n phase 1 (R1/R2/R3/R7/R8) -----------------------------------

    /// The sole field type of the sole generic struct `parse_src` registered.
    fn sole_generic_field(src: &str) -> PolyType {
        let module = parse_src(src).unwrap_or_else(|e| panic!("{src} should parse: {e}"));
        let decl = &module.generic_structs[0];
        assert_eq!(decl.fields.len(), 1, "fixture declares one field");
        decl.fields[0].1.clone()
    }

    #[test]
    fn parse_generic_field_array_of_ty_var_builds_array_polytype() {
        // R1: the shape the whole slice exists for. Before the recursive
        // descent this was `error: unknown type 'T` -- the variable sits one
        // token deeper than the old single-`if` production could look.
        assert_eq!(
            sole_generic_field("type: Pair 'T items ['T 2] ;"),
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(2))
        );
    }

    #[test]
    fn parse_generic_field_nested_array_of_ty_var_builds_nested_polytype() {
        // R1: the descent recurses, so depth is unbounded rather than one.
        assert_eq!(
            sole_generic_field("type: NestArr 'T grid [['T 2] 3] ;"),
            PolyType::Array(
                Box::new(PolyType::Array(
                    Box::new(PolyType::Var(0)),
                    Len::Concrete(2)
                )),
                Len::Concrete(3)
            )
        );
    }

    #[test]
    fn parse_generic_field_owned_cell_of_ty_var_builds_cell_polytype() {
        // R3: `^'T` arrives as one glued token, so the payload is a substring
        // rather than a following token.
        assert_eq!(
            sole_generic_field("type: Cell 'T c ^'T ;"),
            PolyType::OwnedCell(Box::new(PolyType::Var(0)))
        );
        // A `^`-run nests, one wrapper per caret.
        assert_eq!(
            sole_generic_field("type: Cell2 'T c ^^'T ;"),
            PolyType::OwnedCell(Box::new(PolyType::OwnedCell(Box::new(PolyType::Var(0)))))
        );
    }

    #[test]
    fn parse_generic_field_ref_of_ty_var_builds_ref_polytype() {
        // R1/R10: `&'T` *parses* -- it does not build, but the rejection must
        // come from the no-stored-reference rule rather than `unknown type`.
        assert_eq!(
            sole_generic_field("type: Box 'T r &'T ;"),
            PolyType::Ref(Box::new(PolyType::Var(0)), false)
        );
        assert_eq!(
            sole_generic_field("type: BoxM 'T r &!'T ;"),
            PolyType::Ref(Box::new(PolyType::Var(0)), true)
        );
    }

    #[test]
    fn parse_generic_field_generic_application_of_ty_vars_builds_generic_polytype() {
        // R1: each argument is a field type in its own right, so the header's
        // variables reach an application's argument list.
        let module =
            parse_src("type: Ent 'K 'V k 'K v 'V ;\ntype: Wrap 'K 'V e Ent['K 'V] ;\n").unwrap();
        let wrap = module
            .generic_structs
            .iter()
            .find(|d| d.name == "Wrap")
            .expect("Wrap is registered");
        match &wrap.fields[0].1 {
            PolyType::Generic { args, name, .. } => {
                assert_eq!(*name, "Ent");
                assert_eq!(args, &[PolyType::Var(0), PolyType::Var(1)]);
            }
            other => panic!("expected a Generic application, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_field_generic_application_mixed_args_builds_mixed_polytype() {
        // R1: a concrete argument beside a variable one. Asymmetric on
        // purpose -- `Ent['K i64]` and `Ent[i64 'K]` are distinguishable,
        // which a same-type pair would not be.
        let module =
            parse_src("type: Ent 'K 'V k 'K v 'V ;\ntype: W 'K e Ent['K i64] ;\n").unwrap();
        let w = module
            .generic_structs
            .iter()
            .find(|d| d.name == "W")
            .expect("W is registered");
        match &w.fields[0].1 {
            PolyType::Generic { args, .. } => {
                assert_eq!(args, &[PolyType::Var(0), PolyType::Concrete(Type::I64)]);
            }
            other => panic!("expected a Generic application, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_field_unbound_ty_var_inside_array_is_error() {
        // R1: the leaf error is the field parser's own, naming the
        // declaration -- not `PolyBuilder`'s word-signature wording, and not
        // a bare `unknown type`.
        let err = parse_src("type: Box 'T items ['E 2] ;").unwrap_err();
        assert!(err.contains("'E"), "unexpected message: {err}");
        assert!(err.contains("Box"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 21"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_field_ty_var_used_only_inside_array_is_not_phantom() {
        // N4: `check_no_phantom_ty_var` reads the `used` bitmap alone, so the
        // descent has to set it at the leaf, at whatever depth. Without that
        // this fixture is rejected as a phantom parameter.
        assert!(parse_src("type: Pair 'T items ['T 2] ;").is_ok());
        assert!(parse_src("type: Cell 'T c ^'T ;").is_ok());
        assert!(parse_src("type: Deep 'T g [[^'T 2] 3] ;").is_ok());
    }

    #[test]
    fn parse_generic_enum_variant_named_field_array_of_ty_var_builds_array_polytype() {
        // R1: the enum twin shares the field parser, but routes through
        // `parse_generic_variant_fields` rather than the struct field loop.
        let module = parse_src("type: Buf 'T | Some xs ['T 2] | None ;").unwrap();
        let decl = &module.generic_enums[0];
        assert_eq!(
            decl.variants[0].fields[0].1,
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(2))
        );
    }

    #[test]
    fn parse_generic_typedef_concrete_self_reference_resolves() {
        // R2: a header must be registered *before* its own field list is
        // parsed. Nothing here needs R1's descent -- the argument is fully
        // concrete -- which is why this is R2's own witness. Before the
        // two-stage split this was `error: unknown type 'L'`.
        let module = parse_src("type: L 'T v 'T next ^L[i64] ;").unwrap();
        let decl = &module.generic_structs[0];
        match decl.fields[1].1 {
            PolyType::Concrete(Type::OwnedCell(..)) => {}
            ref other => panic!("expected a concrete `^L[i64]`, got {other:?}"),
        }
        // R2's second half: the instantiation was minted while `L`'s own
        // header was still a placeholder, so its fields were owed and paid
        // off on fill. A fieldless `L[i64]` here is the silent-wrong-mint
        // failure the pending machinery exists to prevent.
        let inst = module
            .structs
            .iter()
            .find(|d| d.name == "L[i64]")
            .expect("the concrete self-reference minted `L[i64]`");
        assert_eq!(
            inst.fields.len(),
            2,
            "the deferred fill must have paid off `L[i64]`'s field list"
        );
    }

    #[test]
    fn parse_generic_enum_typedef_concrete_self_reference_resolves() {
        // R2's enum twin: `instantiate_enum` mints `L[i64]` while `L`'s own
        // header is still pending, so the variant list must come from the
        // deferred fill, not from the empty list minted at that moment. A
        // variant-less `L[i64]` here is the silent-wrong-mint failure the
        // enum-side pending machinery exists to prevent.
        let module = parse_src("type: L 'T | Node v 'T n ^L[i64] | Nil ;").unwrap();
        let inst = module
            .enums
            .iter()
            .find(|d| d.name == "L[i64]")
            .expect("the concrete self-reference minted `L[i64]`");
        assert_eq!(
            inst.variants.len(),
            2,
            "the deferred fill must have paid off `L[i64]`'s variant list"
        );
    }

    #[test]
    fn parse_generic_typedef_duplicate_header_still_rejected_after_self_registration() {
        // R2 hazard: stage (a) pushes a placeholder for the first `Box`, and
        // `generic_header_at_cursor_is_registered`'s snapshot must not let
        // that push make the *second* `Box` look pre-registered -- which
        // would swallow it before `check_duplicate_type_names` ever saw it.
        let module = parse_src("type: Box 'T v 'T ; type: Box 'T w 'T ;").unwrap();
        assert_eq!(
            module.generic_structs.len(),
            2,
            "both headers must reach the registry for the duplicate check"
        );
    }

    #[test]
    fn parse_generic_typedef_mutual_self_reference_resolves_both_directions() {
        // R2's load-bearing case for the *two-stage* split specifically: `A`
        // names `B`, declared after it. A single loop registering each header
        // immediately before parsing its own fields still fails here, because
        // `B` has no placeholder yet when `A`'s field list is read.
        let module =
            parse_src("type: A 'T v 'T next ^B['T] ;\ntype: B 'T w 'T back ^A['T] ;\n").unwrap();
        let a = module
            .generic_structs
            .iter()
            .find(|d| d.name == "A")
            .expect("A is registered");
        match &a.fields[1].1 {
            PolyType::OwnedCell(inner) => match inner.as_ref() {
                PolyType::Generic { name, .. } => assert_eq!(*name, "B"),
                other => panic!("expected `^B['T]`, got a cell over {other:?}"),
            },
            other => panic!("expected `^B['T]`, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_field_growing_generic_argument_is_error() {
        // R8: each hop wraps `'T` in another cell, so `L` would need
        // instantiating at a strictly larger argument forever. Rejected
        // structurally, at the field, with no instantiation involved.
        let err = parse_src("type: L 'T v 'T next ^L[^'T] ;").unwrap_err();
        assert!(err.contains("owning cell over"), "unexpected: {err}");
        assert!(
            err.contains("fully concrete or a bare type variable"),
            "the message must name the restriction, not just say `recursive`: {err}"
        );
        assert!(err.contains("line 1, col 22"), "unlocated: {err}");
        // The array and reference wrappers are the same rule.
        let err = parse_src("type: L 'T v 'T next ^L[['T 2]] ;").unwrap_err();
        assert!(err.contains("array of"), "unexpected: {err}");
        let err = parse_src("type: L 'T v 'T next ^L[&'T] ;").unwrap_err();
        assert!(err.contains("reference to"), "unexpected: {err}");
    }

    #[test]
    fn parse_generic_field_compound_concrete_argument_beside_a_variable_is_ok() {
        // R8's accept side: an argument that is compound but *fully concrete*
        // at any depth is inert -- it carries no variable to grow -- so it is
        // admitted; only a compound argument mentioning one of the header's
        // own variables is refused.
        //
        // The argument list has to be *mixed* for this to witness anything.
        // An all-concrete list (`^L[Ent[i64 u32]]`, `^L[[i64 2]]`) folds the
        // whole application to `PolyType::Concrete` at parse time, leaving no
        // `Generic` node in the tree for R8's walk to reach -- so such a
        // fixture passes with the accept clause deleted outright, and is a
        // placebo. Here the bare `'K` is what keeps the application unfolded,
        // and the compound concrete argument beside it is what the clause has
        // to admit.
        assert!(
            parse_src("type: Ent 'K 'V k 'K v 'V ;\ntype: W 'K e Ent['K [i64 2]] ;\n").is_ok(),
            "a concrete array argument beside a variable one is inert"
        );
        assert!(
            parse_src("type: Ent 'K 'V k 'K v 'V ;\ntype: W2 'K e Ent['K Ent[i64 u32]] ;\n")
                .is_ok(),
            "a concrete *nested application* argument is inert too -- and is \
             deliberately asymmetric with the word-signature path, which \
             rejects the same nesting for D5's unrelated depth reason"
        );
    }

    #[test]
    fn parse_generic_field_concrete_referent_folds_to_a_concrete_ref() {
        // R1: the `&`-arm's fold, mirroring `raw_to_poly_type`'s. A bare `&`
        // sigil whose referent turns out concrete must intern a real
        // `Type::Ref` rather than leaving a second representation of one
        // shape (`Ref(Concrete(..))`) for substitution to trip over.
        let module = parse_src("type: B 'T v 'T r & [i64 2] ;").unwrap();
        match module.generic_structs[0].fields[1].1 {
            PolyType::Concrete(Type::Ref(..)) => {}
            ref other => panic!("expected a folded concrete reference, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_field_variable_quotation_is_error() {
        // R7: out of scope, and a located rejection rather than `'T` being
        // misreported as an unknown concrete type.
        let err = parse_src("type: QF 'T f [ 'T -- 'T ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(err.contains("'T"), "unexpected: {err}");
        assert!(err.contains("QF"), "unexpected: {err}");
        assert!(err.contains("line 1, col 17"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_field_variable_after_a_nested_bracket_in_a_quotation_is_error() {
        // R7: the scan for the declaration's own variables has to track
        // bracket depth. Stopping at the first `]` -- an *inner* one here --
        // ends the scan before `'T`, and the field then falls through to the
        // concrete parser, which misreports `'T` as an unknown type instead of
        // naming the unsupported shape.
        let err = parse_src("type: QF 'T f [ [i64 2] -- 'T ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(
            !err.contains("unknown type"),
            "the variable sits past a nested bracket, not past the effect: {err}"
        );
    }

    #[test]
    fn parse_generic_field_glued_sigil_variable_in_quotation_is_error() {
        // R7: `^'T` and `&'T` glue the cell/ref sigil onto the variable's own
        // token, so the scan must peel the sigil off before comparing against
        // `ty_vars` -- otherwise the variable is invisible to this rule and
        // falls through to the concrete parser's misleading `unknown type`.
        let err = parse_src("type: QF 'T f [ ^'T -- ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(err.contains("'T"), "unexpected: {err}");
        assert!(
            !err.contains("unknown type"),
            "the glued cell sigil must not hide the variable: {err}"
        );

        let err = parse_src("type: QF 'T f [ &'T -- ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(err.contains("'T"), "unexpected: {err}");
        assert!(
            !err.contains("unknown type"),
            "the glued ref sigil must not hide the variable: {err}"
        );
    }

    #[test]
    fn parse_generic_field_concrete_quotation_still_parses() {
        // R7/N4: the `[`-arm has to replicate `quotation_type_ahead`'s
        // top-depth `--` scan, or the array production misparses a legal
        // concrete quotation field. `Q` needs a variable-bearing field of its
        // own too, else the phantom check rejects the fixture for an
        // unrelated reason.
        let module = parse_src("type: Q 'T v 'T f [ i64 -- i64 ] ;").unwrap();
        let decl = &module.generic_structs[0];
        match decl.fields[1].1 {
            PolyType::Concrete(Type::Quotation(_)) => {}
            ref other => panic!("expected a concrete quotation field, got {other:?}"),
        }
    }

    #[test]
    fn parse_poly_slot_owned_cell_of_ty_var_builds_cell_rawty() {
        // R3, word-signature side: `parse_poly_slot`'s new `^`-arm. `^` is
        // not a lexer delimiter, so this also depends on
        // `effect_has_variable` recognising a glued `^'T` -- without that the
        // whole effect takes the concrete path and dies on `'T`.
        let module = parse_src(": idc ( ^'T -- ^'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("`idc` is polymorphic");
        assert_eq!(
            sig.inputs[0],
            PolyType::OwnedCell(Box::new(PolyType::Var(0)))
        );
        assert_eq!(
            sig.outputs[0],
            PolyType::OwnedCell(Box::new(PolyType::Var(0)))
        );
    }

    #[test]
    fn parse_poly_slot_owned_cell_of_concrete_payload_folds_to_concrete() {
        // R3: the fold mirrors `Ref`'s -- a fully-concrete payload interns a
        // real `Type::OwnedCell` rather than staying `PolyType::OwnedCell`,
        // so nothing downstream has two representations of one shape.
        let module = parse_src(": f ( ^i64 'T -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("`f` is polymorphic");
        match sig.inputs[0] {
            PolyType::Concrete(Type::OwnedCell(..)) => {}
            ref other => panic!("expected a folded concrete cell, got {other:?}"),
        }
    }

    #[test]
    fn parse_poly_slot_owned_cell_without_payload_is_error() {
        // R3/N1: a bare `^` run with the stack-effect separator behind it has
        // no payload to recurse into; located, not a blame on `--` as an
        // unknown type.
        let err = parse_src(": f ( ^ -- 'T ) ;").unwrap_err();
        assert!(err.contains("has no payload type"), "unexpected: {err}");
    }

    /// The registered struct named `name`, with its `StructId`.
    fn struct_by_name<'m>(module: &'m Module, name: &str) -> (StructId, &'m StructDecl) {
        let idx = module
            .structs
            .iter()
            .position(|d| d.name == name)
            .unwrap_or_else(|| panic!("no struct named `{name}`"));
        (StructId::from_index(idx), &module.structs[idx])
    }

    #[test]
    fn parse_generic_application_at_a_field_mints_a_concrete_struct() {
        // R2/R4/R5: `Box[i64]` in a field position mints an ordinary concrete
        // `StructDecl` -- substituted fields, appended after every pre-pass
        // entry -- and the field carries its `StructId`.
        let module = parse_src("type: Box 'T val 'T ;\ntype: Wrap x Box[i64] ;").unwrap();
        assert_eq!(module.structs.len(), 2);
        let (box_id, boxed) = struct_by_name(&module, "Box[i64]");
        assert_eq!(boxed.fields, vec![("val".to_string(), Type::I64)]);
        assert_eq!(boxed.module, 0);
        let (_, wrap) = struct_by_name(&module, "Wrap");
        assert_eq!(
            wrap.fields,
            vec![("x".to_string(), Type::Struct(box_id, "Box[i64]"))]
        );
    }

    #[test]
    fn parse_generic_application_with_distinct_arguments_mints_distinct_structs() {
        // R4: two applications of one generic type are two registry entries
        // with their own field layouts, not one shared entry.
        let module =
            parse_src("type: Box 'T val 'T ;\ntype: Wrap x Box[i64] y Box[u32] ;").unwrap();
        let (int_id, int_box) = struct_by_name(&module, "Box[i64]");
        let (u32_id, u32_box) = struct_by_name(&module, "Box[u32]");
        assert_ne!(int_id, u32_id);
        assert_eq!(int_box.fields[0].1, Type::I64);
        assert_eq!(u32_box.fields[0].1, Type::U32);
    }

    #[test]
    fn parse_generic_application_repeated_dedups_to_one_struct_id() {
        // R4: structural dedup on `(generic name, concrete arguments)`, the
        // direct assertion (mirrors `intern_bundle_struct`'s own dedup test):
        // three uses across two declarations and a signature, one entry.
        let module = parse_src(
            "type: Box 'T val 'T ;\ntype: A x Box[i64] ;\ntype: B y Box[i64] ;\n: f ( Box[i64] -- ) drop ;",
        )
        .unwrap();
        assert_eq!(
            module
                .structs
                .iter()
                .filter(|d| d.name == "Box[i64]")
                .count(),
            1
        );
        let (box_id, _) = struct_by_name(&module, "Box[i64]");
        assert_eq!(
            struct_by_name(&module, "A").1.fields[0].1,
            Type::Struct(box_id, "Box[i64]")
        );
        assert_eq!(
            struct_by_name(&module, "B").1.fields[0].1,
            Type::Struct(box_id, "Box[i64]")
        );
    }

    #[test]
    fn parse_generic_application_resolves_at_a_word_signature_slot() {
        // R2: a signature slot is a distinct parser call site from a field
        // (`parse_slot`, not `parse_field_type_expr`).
        let module = parse_src("type: Box 'T val 'T ;\n: f ( Box[i64] -- Box[i64] ) ;").unwrap();
        let (box_id, _) = struct_by_name(&module, "Box[i64]");
        let f = module.words.iter().find(|w| w.name == "f").unwrap();
        assert_eq!(f.effect.inputs[0].ty, Type::Struct(box_id, "Box[i64]"));
        assert_eq!(f.effect.outputs[0].ty, Type::Struct(box_id, "Box[i64]"));
    }

    #[test]
    fn parse_generic_application_resolves_at_a_polymorphic_signature_slot() {
        // R2: the third call site, `parse_poly_slot`'s concrete fallthrough --
        // a variable-bearing signature never reaches `parse_slot` at all.
        let module =
            parse_src("type: Box 'T val 'T ;\n: f ( Box[i64] 'A -- 'A Box[i64] ) ;").unwrap();
        let (box_id, _) = struct_by_name(&module, "Box[i64]");
        let f = module.words.iter().find(|w| w.name == "f").unwrap();
        let poly = f.poly.as_ref().expect("a `'A` slot makes the word poly");
        assert_eq!(
            poly.inputs[0],
            PolyType::Concrete(Type::Struct(box_id, "Box[i64]"))
        );
    }

    #[test]
    fn parse_generic_application_nests_concretely() {
        // R6: an argument is a full type expression, so a concrete
        // application inside another resolves by recursion, minting the inner
        // instantiation first.
        let module = parse_src("type: Box 'T val 'T ;\ntype: W x Box[Box[i64]] ;").unwrap();
        let (inner_id, _) = struct_by_name(&module, "Box[i64]");
        let (_, outer) = struct_by_name(&module, "Box[Box[i64]]");
        assert_eq!(outer.fields[0].1, Type::Struct(inner_id, "Box[i64]"));
    }

    #[test]
    fn parse_generic_application_resolves_a_generic_declared_later_in_the_file() {
        // The generic declarations are parsed ahead of the body pass, so an
        // application need not follow its declaration -- the order
        // independence a concrete `type:` name already has from the pre-pass.
        let module = parse_src("type: Wrap x Box[i64] ;\ntype: Box 'T val 'T ;").unwrap();
        let (box_id, _) = struct_by_name(&module, "Box[i64]");
        assert_eq!(
            struct_by_name(&module, "Wrap").1.fields[0].1,
            Type::Struct(box_id, "Box[i64]")
        );
    }

    #[test]
    fn parse_generic_application_with_no_arguments_is_a_located_error() {
        // R3: a generic name is never a type by itself.
        let err = parse_src("type: Box 'T val 'T ;\ntype: Wrap x Box ;").unwrap_err();
        assert!(
            err.contains("generic type `Box` declares 1 type variable"),
            "unexpected message: {err}"
        );
        assert!(err.contains("none were supplied"), "unexpected: {err}");
        assert!(err.contains("line 2, col 14"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_application_with_too_many_arguments_is_a_located_error() {
        // R3: the over-applied case, decidable only because the argument list
        // is bracketed.
        let err = parse_src("type: Box 'T val 'T ;\ntype: Wrap x Box[i64 u32] ;").unwrap_err();
        assert!(
            err.contains("generic type `Box` declares 1 type variable"),
            "unexpected message: {err}"
        );
        assert!(err.contains("2 were supplied"), "unexpected: {err}");
        assert!(err.contains("line 2, col 14"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_application_with_too_few_arguments_is_a_located_error() {
        // R3 for a multi-variable header: the count is what's checked, not
        // merely the presence of a bracket.
        let err =
            parse_src("type: Pair 'A 'B a 'A b 'B ;\n: f ( Pair[i64] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("generic type `Pair` declares 2 type variables"),
            "unexpected message: {err}"
        );
        assert!(err.contains("1 was supplied"), "unexpected: {err}");
        assert!(err.contains("line 2, col 7"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_application_argument_order_is_part_of_the_identity() {
        // R4: the instantiation key is the ordered argument list, so the two
        // orderings are two entries with mirrored field types.
        let module =
            parse_src("type: Pair 'A 'B a 'A b 'B ;\ntype: W x Pair[i64 u32] y Pair[u32 i64] ;")
                .unwrap();
        let (first, _) = struct_by_name(&module, "Pair[i64 u32]");
        let (second, _) = struct_by_name(&module, "Pair[u32 i64]");
        assert_ne!(first, second);
        assert_eq!(
            struct_by_name(&module, "Pair[i64 u32]").1.fields,
            vec![("a".to_string(), Type::I64), ("b".to_string(), Type::U32)]
        );
        assert_eq!(
            struct_by_name(&module, "Pair[u32 i64]").1.fields,
            vec![("a".to_string(), Type::U32), ("b".to_string(), Type::I64)]
        );
    }

    #[test]
    fn parse_generic_enum_application_mints_a_concrete_enum() {
        // R2/R4/R5 on the enum side: variants carry the same argument
        // spelling as their enum, so two instantiations' `Ok` constructors
        // can't clobber each other in a name-keyed registry.
        let module =
            parse_src("type: Res 'T 'E | Ok val 'T | Err val 'E ;\ntype: W r Res[i64 u32] ;")
                .unwrap();
        assert_eq!(module.enums.len(), 1);
        let minted = &module.enums[0];
        assert_eq!(minted.name, "Res[i64 u32]");
        assert_eq!(minted.module, 0);
        assert_eq!(minted.variants[0].name, "Ok[i64 u32]");
        assert_eq!(
            minted.variants[0].fields,
            vec![("val".to_string(), Type::I64)]
        );
        assert_eq!(minted.variants[1].name, "Err[i64 u32]");
        assert_eq!(
            minted.variants[1].fields,
            vec![("val".to_string(), Type::U32)]
        );
        assert_eq!(
            struct_by_name(&module, "W").1.fields[0].1,
            Type::Enum(EnumId::from_index(0), "Res[i64 u32]")
        );
    }

    /// D4: a generic type is applicable only within its declaring module,
    /// even though every module's instantiations share one registry. Drives
    /// `parse_bodies` directly, since a single-file parse has one module and
    /// so cannot discriminate.
    #[test]
    fn parse_generic_application_from_another_module_is_unknown() {
        let owner = lex("type: Box 'T val 'T ;\n").unwrap();
        let other = lex(": f ( Box[i64] -- ) drop ;\n").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let no_imports = HashMap::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let mut run = |tokens: &[(Token, Span)], module: u32| {
            parse_bodies(
                tokens,
                &[],
                &[],
                module,
                &no_imports,
                &[],
                &no_imports,
                &[],
                &mut arrays,
                &mut cells,
                &mut refs,
                &mut slices,
                &mut generics,
                &[],
            )
            .map(|_| ())
        };
        run(&owner, 0).unwrap();
        let err = run(&other, 1).unwrap_err();
        assert!(err.contains("unknown type `Box`"), "unexpected: {err}");
    }

    /// Slice 2 (OQ1): the positive twin of the bare-name rejection above -- a
    /// `q::Box[i64]` application maps `q` through the import map, finds the
    /// header in the target module, and monomorphizes there. The minted
    /// instantiation is stamped with the *applying* module, not the declaring
    /// one, exactly as a same-module application is. `owner` exports `Box`
    /// (R16, round-2 review fix): a qualified generic application is gated on
    /// export exactly like a concrete cross-module type, so this positive
    /// case needs the export to reach the application at all.
    #[test]
    fn parse_qualified_generic_application_from_another_module_resolves() {
        let owner = lex("type: Box 'T val 'T ;\nexport: Box ;\n").unwrap();
        let other = lex(": f ( b::Box[i64] -- ) drop ;\n").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let no_imports = HashMap::new();
        let imports = HashMap::from([("b".to_string(), 0u32)]);
        let exports = vec![vec![("Box".to_string(), Span::default())]];
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        {
            let mut run =
                |tokens: &[(Token, Span)], module: u32, imports: &HashMap<String, u32>| {
                    parse_bodies(
                        tokens,
                        &[],
                        &[],
                        module,
                        imports,
                        &exports,
                        &no_imports,
                        &[],
                        &mut arrays,
                        &mut cells,
                        &mut refs,
                        &mut slices,
                        &mut generics,
                        &[],
                    )
                    .map(|_| ())
                };
            run(&owner, 0, &no_imports).unwrap();
            run(&other, 1, &imports).unwrap();
        }
        assert_eq!(generics.inst_structs.len(), 1);
        assert_eq!(generics.inst_structs[0].name, "Box[i64]");
        assert_eq!(generics.inst_structs[0].module, 1);
        assert_eq!(
            generics.inst_structs[0].fields,
            vec![("val".to_string(), Type::I64)]
        );
    }

    /// R14/R16 (round-2 review fix): a generic header with no `export:` line
    /// is gated exactly like a private concrete type -- reachable through
    /// `resolve_type_or_apply`'s own-module `find_struct`/`find_enum` lookup
    /// (the same registry `resolve_type` never consults), so without this
    /// gate a private generic type would be importable while a private
    /// concrete one is not.
    #[test]
    fn parse_qualified_generic_application_of_unexported_type_is_not_exported() {
        let owner = lex("type: Box 'T val 'T ;\n").unwrap();
        let other = lex(": f ( b::Box[i64] -- ) drop ;\n").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let no_imports = HashMap::new();
        let imports = HashMap::from([("b".to_string(), 0u32)]);
        let no_exports: Vec<Vec<(String, Span)>> = vec![Vec::new()];
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let mut run = |tokens: &[(Token, Span)], module: u32, imports: &HashMap<String, u32>| {
            parse_bodies(
                tokens,
                &[],
                &[],
                module,
                imports,
                &no_exports,
                &no_imports,
                &[],
                &mut arrays,
                &mut cells,
                &mut refs,
                &mut slices,
                &mut generics,
                &[],
            )
            .map(|_| ())
        };
        run(&owner, 0, &no_imports).unwrap();
        let err = run(&other, 1, &imports).unwrap_err();
        assert!(
            err.contains("`Box` is not exported from module `b`"),
            "unexpected: {err}"
        );
    }

    /// A qualifier bound by no `import:` is an ordinary unknown type, not a
    /// panic or a silent own-module fallback that would let `q::Box` reach a
    /// local `Box`.
    #[test]
    fn parse_generic_application_with_unbound_qualifier_is_unknown_type() {
        let err = parse_src("type: Box 'T val 'T ;\n: f ( q::Box[i64] -- ) drop ;").unwrap_err();
        assert!(err.contains("unknown type `q::Box`"), "unexpected: {err}");
    }

    /// The minted instantiation carries the *instantiating* module's id, not
    /// a hard-coded `0` (the same defaulting hazard
    /// `parse_generic_typedef_and_enum_stamp_the_parser_module_id` guards on
    /// the declaration side).
    #[test]
    fn parse_generic_application_stamps_the_instantiating_module_id() {
        let tokens = lex(
            "type: Box 'T val 'T ;\ntype: Res 'T | Ok v 'T ;\n: f ( Box[i64] Res[u32] -- ) drop drop ;\n",
        )
        .unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let no_imports = HashMap::new();
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        parse_bodies(
            &tokens,
            &[],
            &[],
            7,
            &no_imports,
            &[],
            &no_imports,
            &[],
            &mut arrays,
            &mut cells,
            &mut refs,
            &mut slices,
            &mut generics,
            &[],
        )
        .unwrap();
        assert_eq!(generics.inst_structs[0].module, 7);
        assert_eq!(generics.inst_enums[0].module, 7);
    }

    #[test]
    fn parse_word_with_leading_locals_binds_entry_locals() {
        // A leading `| a b |` is the word body's entry binding, with an enum
        // in scope and a variant name available to be misread as one.
        let module = parse_src(
            "type: Shape | Circle r f64 ;
             : sq ( i64 -- i64 ) | n | n n mul ;",
        )
        .unwrap();
        let sq = module.words.iter().find(|w| w.name == "sq").unwrap();
        assert_eq!(entry_locals(sq), ["n"]);
    }

    #[test]
    fn parse_slot_array_type_resolves_and_interns() {
        let module = parse_src(": w ( [i64 4] -- i64 ) drop 0 ;").unwrap();
        let w = &module.words[0];
        assert_eq!(module.arrays.len(), 1);
        match w.effect.inputs[0].ty {
            Type::Array(id, name) => {
                assert_eq!(id.index(), 0);
                assert_eq!(name, "[i64 4]");
            }
            other => panic!("expected Type::Array, got {other:?}"),
        }
        assert_eq!(module.arrays[0].count, 4);
        assert_eq!(module.arrays[0].element, Type::I64);
    }

    #[test]
    fn parse_slot_array_type_same_shape_dedups_to_one_array_id() {
        let module =
            parse_src(": a ( [i64 4] -- i64 ) drop 0 ; : b ( [i64 4] -- i64 ) drop 0 ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        let a_ty = module.words[0].effect.inputs[0].ty;
        let b_ty = module.words[1].effect.inputs[0].ty;
        assert_eq!(a_ty, b_ty);
    }

    /// P7 slice 3c (R1.1/R1.2): `Slice[T]` is spelled like a generic
    /// application but resolves through the interned slice registry, so two
    /// signatures naming the same element share one `SliceId` and a different
    /// element mints a second.
    #[test]
    fn parse_slot_slice_type_interns_by_element() {
        let module = parse_src(
            ": a ( Slice[i64] -- ) drop ; : b ( Slice[i64] -- ) drop ; : c ( Slice[f64] -- ) drop ;",
        )
        .unwrap();
        assert_eq!(module.slices.len(), 2);
        assert_eq!(
            module.words[0].effect.inputs[0].ty,
            module.words[1].effect.inputs[0].ty
        );
        match module.words[0].effect.inputs[0].ty {
            Type::Slice(_, mutable, name) => {
                assert_eq!(name, "Slice[i64]");
                assert!(!mutable, "the surface spelling builds a shared view");
            }
            other => panic!("expected Type::Slice, got {other:?}"),
        }
        assert_ne!(
            module.words[0].effect.inputs[0].ty,
            module.words[2].effect.inputs[0].ty
        );
    }

    /// P7 slice 3c (R1.1, phase 4): `!Slice[T]` is the mutable spelling, the
    /// same `!` mutability marker `&!T` carries. It interns a *distinct*
    /// `SliceId` from the shared view of the same element -- the two are
    /// different types, and only the registry key says so.
    #[test]
    fn parse_slot_mutable_slice_type_interns_separately() {
        let module =
            parse_src(": a ( Slice[i64] -- ) drop ; : b ( !Slice[i64] -- ) drop ;").unwrap();
        assert_eq!(module.slices.len(), 2);
        assert_ne!(
            module.words[0].effect.inputs[0].ty,
            module.words[1].effect.inputs[0].ty
        );
        match module.words[1].effect.inputs[0].ty {
            Type::Slice(_, mutable, name) => {
                assert_eq!(name, "!Slice[i64]");
                assert!(mutable, "`!Slice[T]` builds a mutable view");
            }
            other => panic!("expected Type::Slice, got {other:?}"),
        }
    }

    /// The arity is fixed at one: `Slice` never reaches a user registry, so
    /// nothing else would report a wrong argument count for it.
    #[test]
    fn parse_slot_slice_type_with_two_arguments_is_error() {
        let err = parse_src(": a ( Slice[i64 f64] -- ) drop ;").unwrap_err();
        assert!(err.contains("Slice"), "unexpected message: {err}");
    }

    #[test]
    fn parse_slot_nested_array_type_resolves_both_shapes() {
        let module = parse_src(": w ( [[i64 4] 4] -- i64 ) drop 0 ;").unwrap();
        assert_eq!(module.arrays.len(), 2);
        match module.words[0].effect.inputs[0].ty {
            Type::Array(_, name) => assert_eq!(name, "[[i64 4] 4]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_array_field_resolves() {
        let module = parse_src("type: Buf items [i64 16] top i64 ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        match module.structs[0].fields[0].1 {
            Type::Array(_, name) => assert_eq!(name, "[i64 16]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
        assert_eq!(module.structs[0].fields[1].1, Type::I64);
    }

    #[test]
    fn parse_typedef_enum_variant_array_field_resolves() {
        let module = parse_src("type: Shape | Poly pts [f64 3] ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        match module.enums[0].variants[0].fields[0].1 {
            Type::Array(_, name) => assert_eq!(name, "[f64 3]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_array_type_unknown_element_is_error() {
        // X1: an unknown element type in `[T N]` names the unknown element.
        let result = parse_src(": w ( [Nope 4] -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("Nope"), "unexpected message: {err}");
    }

    #[test]
    fn parse_array_type_zero_length_is_error() {
        // X2: a zero (or negative) length names the type and the invalid length.
        let result = parse_src(": w ( [i64 0] -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("[i64 0]"), "unexpected message: {err}");
        assert!(err.contains(">= 1"), "unexpected message: {err}");
    }

    #[test]
    fn parse_array_type_non_literal_count_is_error() {
        // X3: a non-literal count names the offending count token.
        let result = parse_src(": w ( [i64 n] -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("decimal literal"), "unexpected message: {err}");
        assert!(err.contains('n'), "unexpected message: {err}");
    }

    #[test]
    fn parse_array_type_missing_rbracket_is_error() {
        let result = parse_src(": w ( [i64 4 -- ) drop ;");
        assert!(result.is_err());
    }

    #[test]
    fn parse_array_type_count_exceeding_u32_max_is_error() {
        // A count above u32::MAX is a located error, not a silent truncation.
        let result = parse_src(": w ( [i64 4294967297] -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("4294967297"), "unexpected message: {err}");
        assert!(err.contains("4294967295"), "unexpected message: {err}");
    }

    #[test]
    fn parse_array_type_linear_element_in_signature_parses_ok() {
        // The parser cannot know `Spy` is linear until the checker resolves
        // it (a struct's `has_drop_overload` bit isn't set until `check` sees
        // its `drop` overload, and array-of-linear rejection runs there, not
        // here); the name itself resolves from the parser's name pre-pass.
        let result = parse_src(&format!("{SPY_DEF}: w ( [Spy 2] -- ) drop ;"));
        assert!(result.is_ok(), "unexpected error: {:?}", result.err());
    }

    #[test]
    fn parse_typedef_linear_array_field_parses_ok() {
        let result = parse_src(&format!("{SPY_DEF}type: Bag xs [Spy 2] ;"));
        assert!(result.is_ok(), "unexpected error: {:?}", result.err());
    }

    #[test]
    fn parse_owning_cell_slot_resolves_and_interns() {
        let module = parse_src(": w ( ^i64 -- i64 ) ^> ;").unwrap();
        assert_eq!(module.owned_cells.len(), 1);
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(id, name) => {
                assert_eq!(id.index(), 0);
                assert_eq!(name, "^i64");
            }
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
        assert_eq!(module.owned_cells[0].payload, Type::I64);
    }

    #[test]
    fn parse_owning_cell_same_payload_dedups_to_one_id() {
        let module = parse_src(": a ( ^i64 -- ^i64 ) ; : b ( ^i64 -- ^i64 ) ;").unwrap();
        assert_eq!(module.owned_cells.len(), 1);
        let a_ty = module.words[0].effect.inputs[0].ty;
        let b_ty = module.words[1].effect.inputs[0].ty;
        assert_eq!(a_ty, b_ty);
    }

    #[test]
    fn parse_owning_cell_struct_type_resolves() {
        let module = parse_src("type: Point x i64 y i64 ; : w ( ^Point -- ) ;").unwrap();
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^Point"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_nested_scalar_is_two_layers() {
        let module = parse_src(": w ( ^^i64 -- ) ;").unwrap();
        assert_eq!(module.owned_cells.len(), 2);
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^^i64"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
        assert_eq!(module.owned_cells[0].payload, Type::I64);
        match module.owned_cells[1].payload {
            Type::OwnedCell(_, name) => assert_eq!(name, "^i64"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_array_buffer_type_resolves() {
        // R1: a fixed-capacity heap buffer is `^[u8 N]`, distinct from `^T`
        // over a scalar/struct.
        let module = parse_src(": w ( ^[u8 4] -- ) ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        assert_eq!(module.owned_cells.len(), 1);
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^[u8 4]"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
        match module.owned_cells[0].payload {
            Type::Array(_, name) => assert_eq!(name, "[u8 4]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_nested_array_buffer_type_resolves() {
        let module = parse_src(": w ( ^^[u8 4] -- ) ;").unwrap();
        assert_eq!(module.owned_cells.len(), 2);
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^^[u8 4]"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_type_resolves_in_struct_field_position() {
        // R19: without the field position, `type: Buf b ^[u8 4] ;` fails to
        // parse; this is the buffer case R1 advertises.
        let module = parse_src("type: Buf b ^[u8 4] ;").unwrap();
        match module.structs[0].fields[0].1 {
            Type::OwnedCell(_, name) => assert_eq!(name, "^[u8 4]"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_type_resolves_in_enum_variant_field_position() {
        let module = parse_src("type: Shape | Boxed b ^i64 ;").unwrap();
        match module.enums[0].variants[0].fields[0].1 {
            Type::OwnedCell(_, name) => assert_eq!(name, "^i64"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_named_slot_resolves() {
        // The named-slot path (`name : type`) also recognises `^T`, not just
        // the unnamed-slot shortcut.
        let module = parse_src(": w ( c : ^i64 -- ) ;").unwrap();
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^i64"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_bare_caret_with_no_payload_is_error() {
        let err = parse_src(": w ( ^ -- ) ;").unwrap_err();
        assert!(
            err.contains("no payload type") && err.contains('^'),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_bare_double_caret_with_no_payload_is_error() {
        let err = parse_src(": w ( ^^ -- ) ;").unwrap_err();
        assert!(
            err.contains("no payload type") && err.contains("^^"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_bare_caret_field_with_no_payload_is_error() {
        let err = parse_src("type: Bad b ^ ;").unwrap_err();
        assert!(err.contains("expected a word"), "unexpected message: {err}");
    }

    #[test]
    fn parse_owning_cell_unknown_payload_type_names_remainder_not_whole_word() {
        // The `^` sits at col 7, `Nope` at col 8; the error must name and
        // locate the same text rather than blaming `Nope` at the `^`'s span.
        let err = parse_src(": w ( ^Nope -- ) ;").unwrap_err();
        assert!(
            err.contains("unknown type `Nope`") && err.contains("col 8"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn reserved_caret_type_name_is_error() {
        let err = parse_src("type: ^ x i64 ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
        assert!(
            err.contains("line 1, col 7"),
            "the error should be located: {err}"
        );
    }

    #[test]
    fn reserved_caret_prefixed_type_name_is_error() {
        let err = parse_src("type: ^Foo x i64 ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains("^Foo"), "unexpected message: {err}");
    }

    #[test]
    fn reserved_caret_word_name_is_error() {
        let err = parse_src(": ^ ( -- ) ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
    }

    #[test]
    fn reserved_caret_word_peek_spelling_is_error() {
        let err = parse_src(": ^|> ( -- ) ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains("^|>"), "unexpected message: {err}");
    }

    #[test]
    fn reserved_caret_variant_name_is_error() {
        // A variant name is a word-generating declaration site too: an enum
        // variant named `^` would otherwise become a callable constructor
        // colliding exactly with the cell's own `^` spelling (R12a).
        let err = parse_src("type: E | ^ x i64 | B y i64 ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
    }

    #[test]
    fn reserved_caret_named_slot_is_error() {
        // The name-then-`:type` slot form is a local binding too; without
        // this check `^` intercepted as a bare type expression and the `:`
        // surfaced as an unrelated "unknown type" error.
        let err = parse_src(": w ( ^ : i64 -- ) drop ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
    }

    #[test]
    fn reserved_caret_local_is_error() {
        let err = parse_src(": w ( i64 -- i64 ) | ^ | ^ ;").unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
        assert!(
            err.contains("line 1, col 22"),
            "the error should be located: {err}"
        );
    }

    #[test]
    fn parse_named_slot_array_type_resolves() {
        // The named-slot path (`name : type`) also recognises `[T N]`, not
        // just the unnamed-slot shortcut.
        let module = parse_src(": w ( arr : [i64 4] -- i64 ) drop 0 ;").unwrap();
        let w = &module.words[0];
        assert_eq!(w.effect.inputs[0].name.as_deref(), Some("arr"));
        match w.effect.inputs[0].ty {
            Type::Array(_, name) => assert_eq!(name, "[i64 4]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_reference_type_splits_within_one_word() {
        // `&` and `!` are not delimiters, so `&!Buf` arrives as one `Word`
        // token and splits within itself.
        let module = parse_src("type: Buf n usize ;\n: w ( &!Buf &Buf -- ) drop drop ;").unwrap();
        let w = &module.words[0];
        assert_eq!(w.effect.inputs[0].ty.name(), "&!Buf");
        assert_eq!(w.effect.inputs[1].ty.name(), "&Buf");
        assert_ne!(w.effect.inputs[0].ty, w.effect.inputs[1].ty);
    }

    #[test]
    fn parse_reference_to_owning_cell_type_hands_remainder_to_caret_splitter() {
        // The three-case splitter's `^`-led-remainder case: `&!^List` is one
        // token whose remainder `^List` is the *existing* caret splitter's
        // input, not `resolve_type`'s. Reachable in the dogfood only via a
        // reference-mode eliminator arm, so it gets a unit test of its own.
        let module =
            parse_src("type: List | Nil | Cons v i64 next ^List ;\n: w ( &!^List -- ) drop ;")
                .unwrap();
        assert_eq!(module.words[0].effect.inputs[0].ty.name(), "&!^List");
    }

    #[test]
    fn parse_reference_to_array_type_splits_across_tokens() {
        // `[` *is* a delimiter, so this case recurses into the ongoing token
        // stream instead of splitting within one word.
        let module = parse_src(": w ( &![u8 64] -- ) drop ;").unwrap();
        assert_eq!(module.words[0].effect.inputs[0].ty.name(), "&![u8 64]");
    }

    #[test]
    fn parse_reference_type_with_no_referent_is_error() {
        let err = parse_src(": w ( &! -- ) ;").unwrap_err();
        assert!(
            err.contains("has no referent type"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn reserved_reference_name_is_error_at_every_declaration_site() {
        for src in [
            ": &grab ( -- ) ;",
            "type: &Thing x i64 ;",
            "type: Shape | &Odd ;",
            ": w ( i64 -- ) | &a | ;",
            ": w ( &x : i64 -- ) drop ;",
            "static: &X i64 ;",
        ] {
            let err = parse_src(src).unwrap_err();
            assert!(
                err.contains("reserved for the reference syntax"),
                "unexpected message for `{src}`: {err}"
            );
        }
    }

    #[test]
    fn redefining_an_access_word_is_error() {
        for name in ["@", "!", "+!"] {
            for src in [
                format!(": {name} ( i64 -- ) . ;"),
                format!("static: {name} i64 ;"),
            ] {
                let err = parse_src(&src).unwrap_err();
                assert!(
                    err.contains("is a builtin access word"),
                    "unexpected message for `{src}`: {err}"
                );
            }
        }
    }

    #[test]
    fn parse_extern_declaration_records_its_effect() {
        // Criterion 4/R1, parse half: `extern:` parses at top level and its
        // effect is recorded verbatim, alongside the explicit C symbol string.
        // That the effect is then *registered* is
        // `check_extern_registers_its_effect_at_call_sites`.
        let module = parse_src(r#"extern: strlen ( cstr -- usize ) "strlen" ;"#).unwrap();
        assert_eq!(module.externs.len(), 1);
        let decl = &module.externs[0];
        assert_eq!(decl.name, "strlen");
        assert_eq!(decl.symbol, "strlen");
        assert_eq!(decl.effect.inputs.len(), 1);
        assert_eq!(decl.effect.inputs[0].ty, Type::Cstr);
        assert_eq!(decl.effect.outputs.len(), 1);
        assert_eq!(decl.effect.outputs[0].ty, Type::Usize);
    }

    #[test]
    fn parse_extern_binds_a_different_sooth_name_than_its_c_symbol() {
        // R1: the symbol is an explicit string, not the word name reused, so
        // a Sooth name C cannot spell can still bind a C symbol it can.
        let module = parse_src(r#"extern: open_at ( i64 -- i64 ) "openat" ;"#).unwrap();
        let decl = &module.externs[0];
        assert_eq!(decl.name, "open_at");
        assert_eq!(decl.symbol, "openat");
    }

    #[test]
    fn parse_extern_missing_symbol_string_is_error() {
        let err = parse_src("extern: foo ( i64 -- i64 ) ;").unwrap_err();
        assert!(
            err.contains("string literal naming the C symbol"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_extern_empty_symbol_is_error() {
        // R12: an empty C symbol would lower to `call $`, so it is rejected
        // at the declaration rather than surfacing as broken QBE later.
        let err = parse_src(r#"extern: f ( -- ) "" ;"#).unwrap_err();
        assert!(
            err.contains("not a valid C symbol name"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_trait_decl_records_its_members() {
        // P7.S3e (R1/R3): a trait declaration parses into `Module::traits`,
        // its members sharing one implicit type variable (id 0).
        let module = parse_src("trait: Show 'T show ( &'T -- ) ;").unwrap();
        assert_eq!(module.traits.len(), 3, "Copy/Ord pre-seeded, plus Show");
        let show = module.traits.iter().find(|t| t.name == "Show").unwrap();
        assert_eq!(show.members.len(), 1);
        assert_eq!(show.members[0].name, "show");
        assert!(matches!(
            &show.members[0].sig.inputs[0],
            PolyType::Ref(r, false) if **r == PolyType::Var(0)
        ));
        assert!(show.members[0].sig.outputs.is_empty());
    }

    #[test]
    fn parse_trait_decl_zero_members_is_error() {
        let err = parse_src("trait: Show 'T ;").unwrap_err();
        assert!(err.contains("declares no members"), "{err}");
    }

    #[test]
    fn parse_trait_decl_second_header_variable_is_error() {
        // R16: single-type-variable traits only.
        let err = parse_src("trait: Rel 'T 'U cmp ( &'T &'U -- ) ;").unwrap_err();
        assert!(err.contains("more than one type variable"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_introducing_a_second_variable_is_error() {
        let err = parse_src("trait: Rel 'T cmp ( &'T &'U -- ) ;").unwrap_err();
        assert!(err.contains("more than one type variable"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_with_a_quotation_shape_is_error() {
        // R4/R8: `ground_member_type` (ast.rs) only grounds
        // concrete/array/reference shapes -- a *variable-bearing* quotation
        // shape has no grounding rule and must be rejected here, not left to
        // panic later. (A fully-concrete quotation, with no `'T` inside it,
        // folds to `PolyType::Concrete` at parse time and needs no grounding
        // at all -- not this case.)
        let err = parse_src("trait: Apply 'T run ( &'T [ 'T -- 'T ] -- ) ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_with_an_owned_cell_shape_is_error() {
        // P7.S3n (R3): the new owned-cell shape is deliberately *left out* of
        // the supported set -- `ground_member_type` has no cell arm, so a
        // `^'T` member would ground to nothing. Adding it to the supported
        // list is the mutation this catches, and it is a located rejection
        // rather than a wildcard fall-through.
        let err = parse_src("trait: Sink 'T sink ( ^'T -- ) ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
    }

    #[test]
    fn parse_trait_copy_collides_with_the_reserved_predicate_entry() {
        // R2: `Copy`/`Ord` are pre-seeded trait-table entries, so parsing a
        // user `trait: Copy` succeeds (it is a name, not a reserved-word
        // check) -- the collision is caught by `check_trait_decls`, an
        // ordinary duplicate/collision, at check time.
        let module = parse_src("trait: Copy 'T foo ( &'T -- ) ;").unwrap();
        let err = crate::check::check_trait_decls(&module).unwrap_err();
        assert!(
            err.contains("already the name of a trait"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_trait_ord_collides_with_the_reserved_predicate_entry() {
        // Mirrors the `Copy` case above -- `Ord` is the other pre-seeded
        // predicate entry and was previously untested.
        let module = parse_src("trait: Ord 'T foo ( &'T -- ) ;").unwrap();
        let err = crate::check::check_trait_decls(&module).unwrap_err();
        assert!(
            err.contains("already the name of a trait"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_trait_decl_member_with_a_row_variable_is_error() {
        // A row is a `PolySig` field, not a slot shape, so
        // `member_shape_is_supported` never sees it and the body-form desugar
        // grounds `inputs`/`outputs` alone -- unrejected, the row would be
        // dropped from the synthesized word's effect.
        let err = parse_src("trait: F 'T go ( ..a &'T -- ..a ) ;").unwrap_err();
        assert!(err.contains("declares the row variable `..a`"), "{err}");
        // Input-side only, so the `row_in` arm is what rejects it (the case
        // above sets `row_out` too, and would still be caught by that alone).
        let err = parse_src("trait: F 'T go ( ..a &'T -- ) ;").unwrap_err();
        assert!(err.contains("declares the row variable `..a`"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_with_an_output_only_row_variable_is_error() {
        // The output side carries its own `row_out`, reached only when the
        // input side declares none.
        let err = parse_src("trait: F 'T go ( &'T -- ..b ) ;").unwrap_err();
        assert!(err.contains("declares the row variable `..b`"), "{err}");
    }

    #[test]
    fn parse_impl_decl_for_a_reserved_predicate_trait_is_error() {
        // R2: the reserved `Copy`/`Ord` entries participate in no orphan-rule
        // or export check, so an `impl: Copy for i64` used to fall through to
        // the orphan rule and demand a module that cannot exist.
        let err = parse_src("impl: Copy for i64\n  : show | p | p drop ;\n;").unwrap_err();
        assert!(err.contains("trait `Copy` cannot be implemented"), "{err}");
        assert!(err.contains("built-in predicate"), "{err}");
    }

    #[test]
    fn parse_impl_decl_for_reserved_ord_is_error() {
        let err = parse_src("impl: Ord for i64\n  : show | p | p drop ;\n;").unwrap_err();
        assert!(err.contains("trait `Ord` cannot be implemented"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_with_a_length_variable_array_shape_is_error() {
        // A length-variable array (`&['T 'N]`) is not a supported member
        // shape: `ground_member_type` only grounds `Len::Concrete`, so it must
        // be rejected here at the trait decl -- otherwise the body-form desugar
        // panics grounding it.
        let err = parse_src("trait: Foo 'T bar ( &['T 'N] -- ) ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_bound_reports_bound_on_use_not_unknown_capability() {
        // `prepass_trait_decls` used to build its inner `Parser` with an
        // empty `traits` slice, so a bound (`'T: Copy`) inside a member
        // signature saw no predicate-trait table and reported "unknown
        // capability `Copy`" instead of the located bound-on-use error.
        let err = parse_src("trait: Show 'T show ( 'T: Copy -- ) ;").unwrap_err();
        assert!(
            err.contains("must be written at its binding"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3r (R2): the body form's whole desugar, read off the AST -- the
    /// binding pair `check_impl_decls` will resolve, and the synthesized word
    /// carrying the trait member's signature grounded at the `for` type
    /// (concrete, never a `PolySig`, since there is no signature to restate).
    #[test]
    fn parse_impl_body_synthesizes_a_word_with_the_inherited_effect() {
        let module = parse_src(
            "trait: Show 'T show ( &'T -- i64 ) ;\n\
             impl: Show for i64\n\
               : show | p | p drop 7 ;\n\
             ;",
        )
        .unwrap();
        assert_eq!(
            module.impls[0].bindings,
            vec![("show".to_string(), "show;Show;0;i64".to_string())]
        );
        let synth = module
            .words
            .iter()
            .find(|w| w.name == "show;Show;0;i64")
            .expect("the member body is spliced in as a top-level word");
        assert!(synth.poly.is_none());
        assert!(!synth.declares_inline);
        assert_eq!(
            synth
                .effect
                .inputs
                .iter()
                .map(|s| s.ty.name())
                .collect::<Vec<_>>(),
            vec!["&i64"]
        );
        assert_eq!(
            synth
                .effect
                .outputs
                .iter()
                .map(|s| s.ty.name())
                .collect::<Vec<_>>(),
            vec!["i64"]
        );
    }

    /// P7.S3r (R4a): the member's own name binds to the synthesized word
    /// throughout its body, nested quotations included -- otherwise a recursive
    /// call would resolve against module scope, where the member name is not a
    /// word at all.
    #[test]
    fn parse_impl_body_rewrites_the_members_own_name_inside_a_quotation() {
        let module = parse_src(
            "trait: Show 'T show ( &'T -- i64 ) ;\n\
             impl: Show for i64\n\
               : show | p | ~[ p show ] drop ;\n\
             ;",
        )
        .unwrap();
        let synth = module
            .words
            .iter()
            .find(|w| w.name == "show;Show;0;i64")
            .unwrap();
        let inner = synth
            .body
            .iter()
            .find_map(|t| match &t.kind {
                TermKind::Quotation(inner, ..) => Some(inner),
                _ => None,
            })
            .expect("the body's quotation literal");
        let calls: Vec<&str> = inner
            .iter()
            .filter_map(|t| match &t.kind {
                TermKind::Call(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec!["p", "show;Show;0;i64"]);
    }

    /// P7.S3r (R4a): the rewrite is unconditional token equality, so a binder
    /// of the same name cannot coexist with it -- and silently letting either
    /// one win is the shadowing this language refuses.
    #[test]
    fn parse_impl_body_binder_named_after_the_member_is_error() {
        let err = parse_src(
            "trait: Show 'T show ( &'T -- i64 ) ;\n\
             impl: Show for i64\n\
               : show | show | show drop 7 ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("`show` binds a local inside its own impl body"),
            "{err}"
        );
    }

    #[test]
    fn parse_impl_decl_unknown_trait_is_error() {
        let err = parse_src("impl: Show for i64\n  : show | p | p drop ;\n;").unwrap_err();
        assert!(err.contains("unknown trait `Show`"), "{err}");
    }

    #[test]
    fn parse_impl_decl_zero_bindings_is_error() {
        let err = parse_src("trait: Show 'T show ( &'T -- ) ;\nimpl: Show for i64 ;").unwrap_err();
        assert!(err.contains("binds no members"), "{err}");
    }

    #[test]
    fn find_trait_in_module_resolves_own_module_then_qualified() {
        let show = crate::ast::TraitDecl {
            name: "Show".to_string(),
            kind: TraitKind::Nominal,
            members: Vec::new(),
            module: 1,
            span: Span::default(),
        };
        let traits = vec![show];
        let mut imports = HashMap::new();
        imports.insert("lib".to_string(), 1u32);
        let no_selective = HashMap::new();
        assert!(find_trait_in_module(&traits, "Show", 0, &imports, &no_selective).is_none());
        assert_eq!(
            find_trait_in_module(&traits, "lib::Show", 0, &imports, &no_selective),
            Some(TraitId::from_index(0))
        );
        assert_eq!(
            find_trait_in_module(&traits, "Show", 1, &imports, &no_selective),
            Some(TraitId::from_index(0))
        );
    }

    #[test]
    fn parse_extern_symbol_with_illegal_characters_is_error() {
        // R12: a symbol containing a newline or quote would corrupt the
        // generated `call $<symbol>` instruction if emitted verbatim.
        let err = parse_src(r#"extern: g ( -- ) "a\nb\"c" ;"#).unwrap_err();
        assert!(
            err.contains("not a valid C symbol name"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_extern_malformed_effect_is_error() {
        let err = parse_src(r#"extern: foo ( i64 -- "strlen" ;"#).unwrap_err();
        assert!(err.starts_with("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn parse_extern_nested_inside_a_word_body_is_rejected() {
        let err =
            parse_src(": main ( -- )\n  extern: foo ( i64 -- i64 ) \"foo\" ;\n;").unwrap_err();
        assert!(err.starts_with("parse error"), "unexpected message: {err}");
    }

    // -- Phase 7 slice 2 (D1/D2): `static:` and the `global:` clause --------

    #[test]
    fn parse_static_scalar_with_initializer_ok() {
        let module = parse_src("static: LIMIT i64 = 10 ;").unwrap();
        assert_eq!(module.statics.len(), 1);
        let decl = &module.statics[0];
        assert_eq!(decl.name, "LIMIT");
        assert_eq!(decl.ty, Type::I64);
        assert_eq!(decl.init, StaticInit::Int(10));
    }

    #[test]
    fn parse_static_zero_elided_initializer_ok() {
        let module = parse_src("static: COUNT i64 ;").unwrap();
        assert_eq!(module.statics[0].init, StaticInit::Zero);
    }

    #[test]
    fn parse_static_bool_elided_zero_ok() {
        let module = parse_src_with_bool("static: FLAG Bool ;").unwrap();
        let decl = &module.statics[0];
        assert_eq!(decl.ty, bool_ty(&module));
        assert_eq!(decl.init, StaticInit::Zero);
    }

    #[test]
    fn parse_static_str_elided_zero_is_empty_ok() {
        // D1/D3: `str`'s zero value is the empty string, distinct from `Zero`
        // meaning "uninitialised" -- `Zero` is the marker the checker/lowering
        // reads *as* `""` for a `str` static.
        let module = parse_src("static: NAME str ;").unwrap();
        let decl = &module.statics[0];
        assert_eq!(decl.ty, Type::Str);
        assert_eq!(decl.init, StaticInit::Zero);
    }

    #[test]
    fn parse_static_bool_and_str_initializer_ok() {
        let module =
            parse_src_with_bool("static: FLAG Bool = True ;\nstatic: TAG str = \"x\" ;").unwrap();
        assert_eq!(module.statics[0].init, StaticInit::Bool(true));
        assert_eq!(module.statics[1].init, StaticInit::Str("x".to_string()));
    }

    #[test]
    fn parse_static_decl_span_points_at_the_name() {
        let module = parse_src("  static: COUNT i64 ;").unwrap();
        let span = module.statics[0].span;
        assert_eq!((span.line, span.col), (1, 11));
    }

    #[test]
    fn parse_static_struct_type_is_error() {
        // OQ1/D1: allow-list-based, not struct-detection-based -- a genuine
        // struct type and a mistyped/forward-referenced user type produce the
        // same "non-scalar type" error.
        let err = parse_src("type: Uart n i64 ;\nstatic: U Uart ;").unwrap_err();
        assert!(err.contains("non-scalar"), "unexpected message: {err}");
        assert!(err.contains("`Uart`"), "names the type: {err}");
        assert!(err.contains("static `U`"), "names the static: {err}");
    }

    #[test]
    fn parse_static_u32_init_out_of_range_is_error() {
        let err = parse_src("static: X u32 = -5 ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
        let err = parse_src("static: X u32 = 99999999999 ;").unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
    }

    #[test]
    fn parse_global_clause_records_entries() {
        let module = parse_src(": tick ( -- i64 ) global: COUNT w, LIMIT r 0 ;").unwrap();
        let entries = module.words[0].declared_globals.as_ref().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "COUNT");
        assert_eq!(entries[0].mode, GlobalMode::W);
        assert_eq!(entries[1].name, "LIMIT");
        assert_eq!(entries[1].mode, GlobalMode::R);
    }

    #[test]
    fn parse_global_clause_accepts_a_free_standing_comma() {
        // The separator may be spaced off its mode token (`w ,`) as well as
        // glued to it (`w,`); both reach the same entry list.
        let module = parse_src(": tick ( -- i64 ) global: COUNT w , LIMIT r 0 ;").unwrap();
        let entries = module.words[0].declared_globals.as_ref().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "COUNT");
        assert_eq!(entries[0].mode, GlobalMode::W);
        assert_eq!(entries[1].name, "LIMIT");
        assert_eq!(entries[1].mode, GlobalMode::R);
    }

    #[test]
    fn parse_global_clause_missing_comma_is_error() {
        // Without the guard the clause silently ends at `COUNT w` and
        // `LIMIT r` becomes body terms, reported as an unknown word.
        let err = parse_src(": tick ( -- i64 ) global: COUNT w LIMIT r 0 ;").unwrap_err();
        assert!(err.contains("missing `,`"), "unexpected message: {err}");
        assert!(err.contains("LIMIT"), "points at the second entry: {err}");
    }

    #[test]
    fn parse_global_clause_empty_is_error() {
        let err = parse_src(": tick ( -- ) global: ;").unwrap_err();
        assert!(err.starts_with("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn parse_global_clause_invalid_mode_is_error() {
        let err = parse_src(": tick ( -- ) global: COUNT x 0 ;").unwrap_err();
        assert!(err.starts_with("parse error"), "unexpected message: {err}");
        assert!(err.contains('x'), "names the bad mode token: {err}");
    }

    #[test]
    fn parse_effect_without_global_clause_unchanged() {
        let module = parse_src(": inc ( i64 -- i64 ) 1 add ;").unwrap();
        assert!(module.words[0].declared_globals.is_none());
        assert_eq!(module.words[0].effect.inputs.len(), 1);
        assert_eq!(module.words[0].effect.outputs.len(), 1);
    }

    #[test]
    fn parse_global_clause_on_poly_effect_ok() {
        // The clause reads the same after a variable-bearing effect (the
        // `parse_poly_effect` path), unaffected by D2's byte-for-byte
        // guarantee on the effect reader itself.
        let module = parse_src(": dupit ( 'T: Copy -- 'T 'T ) global: COUNT w dup ;").unwrap();
        let entries = module.words[0].declared_globals.as_ref().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "COUNT");
        assert_eq!(entries[0].mode, GlobalMode::W);
        assert!(module.words[0].poly.is_some());
    }

    #[test]
    fn parse_monomorphic_word_carries_no_poly_sig() {
        // R2: an effect with no variable is unchanged — the polymorphic
        // representation is attached only when a variable is present.
        let module = parse_src(": inc ( i64 -- i64 ) 1 add ;").unwrap();
        assert!(module.words[0].poly.is_none());
    }

    #[test]
    fn parse_poly_type_variable_word_attaches_a_poly_sig() {
        // R1/R4: a `'T` effect parses into a `PolySig`, and the concrete
        // effect is left empty (its whole signature lives in `poly`).
        let module = parse_src(": dupit ( 'T: Copy -- 'T 'T ) dup ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert_eq!(sig.inputs.len(), 1);
        assert_eq!(sig.outputs.len(), 2);
        assert!(sig.has_bound(0, Bound::Copy));
        assert!(module.words[0].effect.inputs.is_empty());
    }

    #[test]
    fn parse_capabilities_still_folds_copy_ord_byte_for_byte() {
        // P7.S3e (R2): `parse_capabilities`'s rewrite (a trait-table lookup
        // replacing the two hardcoded string compares) must not change
        // `'T: Copy Ord`'s existing parse result -- the highest-blast-radius
        // regression this phase's Codebase Map calls out.
        let module = parse_src(": f ( 'T: Copy Ord -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.bounds, vec![(0, Bound::Copy), (0, Bound::Ord)]);
    }

    #[test]
    fn parse_capabilities_unknown_name_is_still_an_error() {
        // A name that resolves to neither a pre-seeded predicate entry nor a
        // declared trait is still X3.
        let err = parse_src(": f ( 'T: Nope -- 'T ) ;").unwrap_err();
        assert!(err.contains("unknown capability"), "{err}");
    }

    #[test]
    fn parse_capabilities_resolves_a_declared_trait_to_a_user_bound() {
        // P7.S3e (R6/R18): a nominal trait name in a bound resolves against
        // the same table `Copy`/`Ord` do, at parse time, and is baked into
        // `Bound::User(TraitId)` before `Resolver::rewrite` ever runs. Index 2
        // because the two pre-seeded predicate entries occupy 0 and 1.
        let module =
            parse_src("trait: Show 'T show ( &'T -- ) ;\n: f ( 'T: Show -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.bounds, vec![(0, Bound::User(TraitId::from_index(2)))]);
    }

    #[test]
    fn parse_capabilities_composes_a_predicate_and_a_user_trait() {
        // R5: the capability list stays greedy across the two kinds, in
        // source order.
        let module =
            parse_src("trait: Order 'T cmp ( &'T &'T -- i64 ) ;\n: f ( 'T: Copy Order -- 'T ) ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(
            sig.bounds,
            vec![(0, Bound::Copy), (0, Bound::User(TraitId::from_index(2)))]
        );
    }

    #[test]
    fn parse_capabilities_stops_before_a_following_type_slot() {
        // The greedy list ends at the first word the trait table does not
        // know, which is then the enclosing signature's next input slot --
        // not a capability, and not an error.
        let module =
            parse_src("trait: Show 'T show ( &'T -- ) ;\n: f ( 'T: Show i64 -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.bounds, vec![(0, Bound::User(TraitId::from_index(2)))]);
        assert_eq!(
            sig.inputs,
            vec![PolyType::Var(0), PolyType::Concrete(Type::I64)]
        );
    }

    #[test]
    fn parse_capabilities_rejects_an_unbound_qualifier_in_a_bound() {
        // R18(a): `parse_capabilities` has no `resolve_type` to delegate to,
        // so an unresolvable qualifier needs its own located rejection rather
        // than falling through to the generic unknown-capability message.
        let err = parse_src(": f ( 'T: q::Show -- 'T ) ;").unwrap_err();
        assert!(err.contains("unknown module qualifier `q`"), "{err}");
        assert!(err.contains("`q::Show`"), "{err}");
    }

    #[test]
    fn parse_capabilities_unbound_qualifier_after_a_bound_is_the_next_slot() {
        // Review finding 2: an unresolvable qualifier used to raise the
        // unbound-qualifier error unconditionally, even past the first bound
        // -- so a legal signature whose next input happens to be a qualified
        // type (unrelated to any bound) was misdiagnosed as a bad bound.
        let err =
            parse_src("trait: Copy2 'T dummy ( 'T -- ) ;\n: f ( 'T: Copy2 q::Point -- 'T ) drop ;")
                .unwrap_err();
        // `q::Point` is not itself resolvable to anything here (no `q`
        // import exists), so this must fail as an ordinary unknown-type
        // error on the next slot, never as an unbound-bound-qualifier one.
        assert!(!err.contains("in bound"), "{err}");
        assert!(err.contains("q::Point"), "{err}");
    }

    #[test]
    fn parse_length_variable_in_count_position() {
        // R1: `'N` in an array count slot is a length variable, lexically
        // identical to a type variable but distinguished by position.
        let module = parse_src(": alen ( [i64 'N] -- [i64 'N] usize ) len ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.len_var_names, vec!["'N".to_string()]);
        assert!(sig.ty_var_names.is_empty());
        assert!(matches!(sig.inputs[0], PolyType::Array(_, Len::Var(0))));
    }

    #[test]
    fn effect_has_variable_recognizes_a_glued_only_referent() {
        // Slice 13 (R-A3, review fix): `parse_poly_ref_slot_with_glued_variable_referent`
        // below has a bare `'T` input that trips the pre-scan on its own,
        // masking a gap where `&'T` is the *only* variable mention in the
        // effect. Without the fix this signature took the concrete path and
        // `'T` failed to resolve as an unknown type.
        let module = parse_src(": f ( &'T -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(
            matches!(&sig.inputs[0], PolyType::Ref(r, false) if **r == PolyType::Var(0)),
            "`&'T` should fold to a shared `PolyType::Ref` over `'T`"
        );
    }

    #[test]
    fn parse_poly_ref_slot_with_glued_variable_referent() {
        // Slice 13 (R-A3, glued case): `&'T` lexes as one word (`&`/`'` are
        // not delimiters), so the referent is a substring, not a token to
        // recurse on. Both mutabilities ride the variant.
        let module = parse_src(": f ( 'T -- &'T &!'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(
            matches!(&sig.outputs[0], PolyType::Ref(r, false) if **r == PolyType::Var(0)),
            "`&'T` should fold to a shared `PolyType::Ref` over `'T`"
        );
        assert!(
            matches!(&sig.outputs[1], PolyType::Ref(r, true) if **r == PolyType::Var(0)),
            "`&!'T` should fold to a mutable `PolyType::Ref` over `'T`"
        );
    }

    #[test]
    fn parse_poly_ref_slot_binds_a_bound_on_a_glued_variable() {
        // Slice 13 (R-A3): the glued case interns its variable through the
        // same path a bare slot does, so a bound at a `&'T` *binding*
        // occurrence attaches to `'T` itself. Splitting the sigil without
        // that shared path would intern a variable spelled `'T:`, silently
        // distinct from every later `'T`.
        let module = parse_src(": f ( &'T: Copy -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert!(sig.has_bound(0, Bound::Copy));
        assert!(matches!(sig.outputs[0], PolyType::Var(0)));
    }

    #[test]
    fn parse_poly_ref_slot_with_bare_sigil_recurses_on_the_next_token() {
        // Slice 13 (R-A3, bare-sigil case): `[` *is* a delimiter, so `&['T 4]`
        // arrives as a lone `&` followed by a genuine array token, which
        // recurses as a poly slot rather than resolving concretely.
        let module = parse_src(": peek ( ['T 4] -- &['T 4] ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Ref(referent, false) = &sig.outputs[0] else {
            panic!(
                "expected a shared `PolyType::Ref`, got {:?}",
                sig.outputs[0]
            );
        };
        assert!(matches!(**referent, PolyType::Array(_, Len::Concrete(4))));
    }

    #[test]
    fn parse_poly_ref_slot_with_concrete_referent_folds_to_a_type() {
        // Slice 13 (R-A4): a `&`-slot whose referent folds fully concrete
        // interns a real `Type::Ref`, exactly as a concrete array shape does;
        // only a variable-bearing referent stays `PolyType::Ref`.
        let module = parse_src(": f ( 'T [i64 4] -- 'T &[i64 4] ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Concrete(ty) = sig.outputs[1] else {
            panic!("expected a folded `Concrete`, got {:?}", sig.outputs[1]);
        };
        assert_eq!(ty.name(), "&[i64 4]");
    }

    #[test]
    fn parse_poly_ref_slot_without_a_referent_is_error() {
        // Slice 13 (R-A3): a bare sigil with nothing to borrow is the same
        // located error the concrete type-expression path already emits.
        let err = parse_src(": f ( 'T & -- 'T ) ;").unwrap_err();
        assert!(
            err.contains("reference type `&` has no referent type"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_poly_generic_over_own_type_variable_ok() {
        // P7 slice 3a (R1): a generic type applied to the enclosing
        // signature's own variables parses, where it used to die on `'T` as
        // an unknown type (probed at HEAD before this slice).
        let module = parse_src(
            "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
             : reorder ( 'T Result['T 'E] -- Result['T 'E] 'T ) swap ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(
            sig.inputs[1],
            PolyType::Generic { is_enum: true, .. }
        ));
        assert!(matches!(
            sig.outputs[0],
            PolyType::Generic { is_enum: true, .. }
        ));
    }

    #[test]
    fn parse_poly_generic_all_concrete_args_folds_to_concrete() {
        // P7 slice 3a (R1): the concrete path is byte-for-byte unchanged --
        // every argument concrete instantiates through `GenericTypes` exactly
        // as `resolve_type_or_apply` already does, yielding `Concrete`, never
        // `PolyType::Generic`. Probed at HEAD: this signature already builds
        // and runs.
        let module = parse_src(
            "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
             : wrap ( 'T -- Result[i64 i64] ) drop 1 Ok ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(sig.outputs[0], PolyType::Concrete(Type::Enum(..))));
    }

    #[test]
    fn parse_poly_generic_nested_depth_two_is_error() {
        // D5: a generic type argument that is itself a generic application
        // is rejected at nesting depth > 1, naming both headers.
        let err =
            parse_src("type: Box 'T val 'T ;\n: f ( 'T Box[Box['T]] -- ) drop drop ;").unwrap_err();
        assert!(
            err.contains("names `Box[...]` as a type argument"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_poly_generic_arity_mismatch_is_error() {
        // R1: the poly-slot argument list reuses `generic_arity_error`
        // exactly as the concrete path does.
        let err = parse_src("type: Box 'T val 'T ;\n: f ( Box['T 'E] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("declares 1 type variable"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_poly_generic_private_header_is_not_exported_error() {
        // R1: the new arm reuses `resolve_type_or_apply`'s header lookup and
        // privacy gate, so a qualified generic application inside a poly
        // slot is rejected exactly as the concrete path already rejects
        // `parse_qualified_generic_application_of_unexported_type_is_not_exported`.
        let owner = lex("type: Box 'T val 'T ;\n").unwrap();
        let other = lex(": f ( 'T b::Box['T] -- ) drop drop ;\n").unwrap();
        let mut arrays = Vec::new();
        let mut cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let no_imports = HashMap::new();
        let imports = HashMap::from([("b".to_string(), 0u32)]);
        let no_exports: Vec<Vec<(String, Span)>> = vec![Vec::new()];
        let mut generics = crate::ast::GenericTypes::with_bases(0, 0);
        let mut run = |tokens: &[(Token, Span)], module: u32, imports: &HashMap<String, u32>| {
            parse_bodies(
                tokens,
                &[],
                &[],
                module,
                imports,
                &no_exports,
                &no_imports,
                &[],
                &mut arrays,
                &mut cells,
                &mut refs,
                &mut slices,
                &mut generics,
                &[],
            )
            .map(|_| ())
        };
        run(&owner, 0, &no_imports).unwrap();
        let err = run(&other, 1, &imports).unwrap_err();
        assert!(
            err.contains("`Box` is not exported from module `b`"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn parse_row_variable_records_both_sides() {
        // R1: a `..s` at the deepest slot of each side is the row variable.
        let module =
            parse_src(": dup2 ( ..s 'a: Copy 'b: Copy -- ..s 'a 'b 'a 'b ) over over ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.row_in.is_some());
        assert!(sig.row_out.is_some());
        assert_eq!(sig.inputs.len(), 2);
        assert_eq!(sig.outputs.len(), 4);
    }

    #[test]
    fn parse_x1_name_as_both_type_and_length_variable_is_error() {
        // X1: one `'`-name in both a type slot and a count slot is a located
        // declaration error naming the variable.
        let err = parse_src(": f ( 'N [i64 'N] -- i64 ) drop drop ;").unwrap_err();
        assert!(err.contains("'N"), "unexpected message: {err}");
        assert!(
            err.contains("type variable") && err.contains("length variable"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_x2_row_variable_not_deepest_is_error() {
        // X2: `..s` anywhere but the deepest (leftmost) slot is a located error.
        let err = parse_src(": f ( i64 ..s -- i64 ) ;").unwrap_err();
        assert!(err.contains("row variable"), "unexpected message: {err}");
        assert!(err.contains("deepest"), "unexpected message: {err}");
    }

    #[test]
    fn parse_x2_row_variable_twice_on_one_side_is_error() {
        // X2: a second `..s` on one side is a located error.
        let err = parse_src(": f ( ..s ..t -- i64 ) ;").unwrap_err();
        assert!(err.contains("row variable"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_quotation_effect_survives_the_concrete_fold() {
        // R6: `~[ ..s i64 -- ..s ]` is fully concrete slot-by-slot (`i64`
        // on both sides), so without the row-set exception it would fold to
        // `Concrete(Type::InlineQuotation)`, destroying the row before any
        // splice ever sees it. It must stay `PolyType::Quotation` with both
        // row fields populated (R7).
        let module =
            parse_src(": my-times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s ) drop drop drop ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.row_in.is_some());
        assert!(sig.row_out.is_some());
        let quot = &sig.inputs[1];
        match quot {
            PolyType::Quotation(ins, outs, is_inline, row_in, row_out) => {
                assert!(*is_inline);
                assert_eq!(ins.len(), 1, "the row is a field, not a slot");
                assert!(outs.is_empty());
                assert!(row_in.is_some());
                assert!(row_out.is_some());
                assert_eq!(row_in, row_out, "the same row on both sides");
            }
            other => panic!("expected PolyType::Quotation with a row, got {other:?}"),
        }
    }

    #[test]
    fn parse_row_in_quotation_effect_fresh_name_is_error() {
        // R4: a `..`-prefixed name inside a quotation effect must denote the
        // signature's own top-level row; a fresh name is a located error.
        let err = parse_src(": f ( ..s i64 ~[ ..t i64 -- ..t ] -- ..s ) drop drop ;").unwrap_err();
        assert!(err.contains("..t"), "unexpected message: {err}");
        assert!(err.contains("top-level row"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_quotation_effect_no_top_level_row_is_error() {
        // R4: any row inside a quotation effect is an error when the
        // signature declared no top-level row at all.
        let err = parse_src(": f ( i64 ~[ ..s i64 -- ..s ] -- i64 ) drop drop ;").unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("top-level row"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_quotation_effect_one_sided_is_error() {
        // R5: a row on one side of a quotation effect only is a located
        // error.
        let err = parse_src(": f ( ..s i64 ~[ ..s i64 -- ] -- ..s ) drop drop ;").unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("both sides"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_quotation_effect_differing_output_row_is_error() {
        // R5: for 10a, a loop body's row must be the same on both sides; a
        // differing declared output row is a located error at exact text.
        // The top-level input row (`..s`) and output row (`..t`) are both
        // already known by the time the nested quotation is parsed (each is
        // the leading token of its own top-level side), so the quotation's
        // two row mentions resolve to two distinct known ids rather than
        // tripping R4's fresh-name rejection.
        let err = parse_src(": f ( ..s i64 -- ..t ~[ ..s i64 -- ..t ] ) drop ;").unwrap_err();
        assert_eq!(
            err,
            "error: a loop body cannot change the shape of the carried region: `..s` in, `..t` out\nnote: 10c lifts this for a word without a back-edge"
        );
    }

    #[test]
    fn parse_row_in_quotation_effect_shape_change_for_input_side_combinator_parses() {
        // Slice 10c (R-P2-2/R-P2-5): a quotation *parameter* (input side of
        // the word) of a quotation-taking (always-inlined) word may declare
        // differing rows -- `..i` (already a known top-level row by the time
        // this parameter is reached) and `..o` (a forward reference to the
        // signature's own top-level output row, named only later, admitted by
        // R-P2-1's deferred check). The shape change is splice-local
        // (INV-INLINE-COMBINATOR), never a carried region on a back-edge, so
        // it is not the 10a same-row restriction's concern.
        let module = parse_src_with_bool(
            ": myif ( ..i Bool ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o ) \
             | e | | t | | c | c [ t call ] [ e call ] if ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.row_in.is_some());
        assert!(sig.row_out.is_some());
        assert_ne!(
            sig.row_in, sig.row_out,
            "the word's own top-level rows genuinely differ"
        );
        for input in &sig.inputs {
            if let PolyType::Quotation(_, _, is_inline, row_in, row_out) = input {
                assert!(*is_inline);
                assert_eq!(*row_in, sig.row_in);
                assert_eq!(*row_out, sig.row_out);
            }
        }
    }

    #[test]
    fn parse_row_in_quotation_effect_naming_an_output_only_row_from_the_input_side_is_error() {
        // Review fix (cycle 3): a row named only on the signature's *output*
        // side, referenced from a nested quotation effect on the *input*
        // side, must still be rejected -- the signature declares no
        // top-level input row at all here, so there is no stack region for
        // the quotation to be grounded against when it executes. A prior
        // (unsound) fix made `quotation_row_id` accept any name present
        // anywhere in the row index, which let this compile and run with a
        // row grounded against whatever the caller's actual stack happened
        // to be.
        let err = parse_src(": bad ( i64 ~[ ..s i64 -- ..s ] -- ..s ) | f | f call ;").unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("top-level row"), "unexpected message: {err}");
        assert!(
            !err.contains("none of that name is declared"),
            "message must not claim the name isn't declared anywhere: {err}"
        );
    }

    #[test]
    fn parse_row_in_quotation_effect_second_quotation_input_side_order_dependent_is_error() {
        // Review fix: a prior quotation's *deferred* output-side mention of
        // `..o` (not yet confirmed against the top-level row_out, which is
        // still unset here -- this is entirely on the word's input side)
        // used to leak into `row_index`, letting a *later* sibling
        // quotation's strict input-side check accept `..o` even though no
        // top-level row is bound to it yet at this point in the signature.
        // Removing the first quotation (the only thing that had interned
        // `..o`) must not change whether the second one is accepted.
        let err = parse_src_with_bool(": f ( ..i Bool ~[ ..o -- ..o ] -- ..o ) | c | c call ;")
            .unwrap_err();
        assert!(err.contains("..o"), "unexpected message: {err}");
        assert!(err.contains("top-level row"), "unexpected message: {err}");

        let err = parse_src_with_bool(
            ": f ( ..i Bool ~[ ..i -- ..o ] ~[ ..o -- ..o ] -- ..o ) \
             | d | | c | c call d call ;",
        )
        .unwrap_err();
        assert!(err.contains("..o"), "unexpected message: {err}");
        assert!(err.contains("top-level row"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_quotation_effect_output_side_row_fresh_name_is_error() {
        // Review fix: `validate_pending_quotation_rows` (the deferred
        // check for a row named on a quotation effect's *output* side) had
        // no coverage of its own -- neutering its rejection failed no test
        // in the suite. `..t` is optimistically interned (so it parses past
        // R4's immediate check) but is neither of the signature's own
        // top-level rows (both are `..s`), so it must still be rejected once
        // the whole signature is known.
        let err =
            parse_src(": f ( ..s i64 ~[ ..s i64 -- ..t ] -- ..s ) drop drop drop ;").unwrap_err();
        assert!(err.contains("..t"), "unexpected message: {err}");
        assert!(err.contains("top-level row"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_non_inline_quotation_effect_is_error() {
        // Review fix (post-10a): a row's size is unknown at runtime, so only
        // an inline (`~[ ... ]`) quotation -- spliced at its call site, never
        // materialized -- may carry one. An ordinary (non-`~`) quotation
        // effect with a row on both sides used to be accepted with full
        // row-grounding treatment; it must now be a located parse error.
        let err =
            parse_src(": fx ( ..s i64 [ ..s i64 -- ..s ] -- ..s ) | f | f call ;").unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("inline"), "unexpected message: {err}");
        assert!(err.contains("~["), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_inline_quotation_effect_still_parses() {
        // The `~` spelling of the same signature must still be accepted: R5's
        // new inline-only guard must not reject the case it is meant to keep
        // legal.
        let module =
            parse_src(": fx ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s ) | f | f call ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.row_in.is_some());
        assert!(sig.row_out.is_some());
    }

    #[test]
    fn parse_row_in_non_inline_quotation_effect_nested_inside_an_inline_one_is_error() {
        // Coverage gap (review cycle 3): the inline-only guard must reach a
        // non-inline row-bearing quotation nested *inside* an inline one,
        // not just a top-level non-inline quotation.
        let err = parse_src(": fx ( ..s i64 ~[ ..s [ ..s -- ..s ] -- ..s ] -- ..s ) drop drop ;")
            .unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("inline"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_non_inline_quotation_effect_in_array_element_position_is_error() {
        // Coverage gap (review cycle 3): the guard must also reach a
        // row-bearing quotation appearing as an array element type.
        let err =
            parse_src(": fx ( ..s i64 [ [ ..s -- ..s ] 3 ] -- ..s ) drop drop ;").unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("inline"), "unexpected message: {err}");
    }

    #[test]
    fn parse_row_in_non_inline_quotation_effect_on_output_side_is_error() {
        // Coverage gap (review cycle 3): the guard must also reach a
        // row-bearing quotation on the signature's output side.
        let err = parse_src(": fx ( ..s i64 -- ..s [ ..s -- ..s ] ) drop ;").unwrap_err();
        assert!(err.contains("..s"), "unexpected message: {err}");
        assert!(err.contains("inline"), "unexpected message: {err}");
    }

    #[test]
    fn parse_x3_bound_on_use_occurrence_is_error() {
        // X3: a bound must be written at the binding occurrence, not a use.
        let err = parse_src(": f ( 'T: Copy 'T: Copy -- 'T ) drop ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(err.contains("binding"), "unexpected message: {err}");
    }

    #[test]
    fn parse_x3_unknown_capability_is_error() {
        // X3: an unknown capability name after a bound colon is a located error.
        let err = parse_src(": f ( 'T: Frobnicate -- 'T ) ;").unwrap_err();
        assert!(err.contains("Frobnicate"), "unexpected message: {err}");
        assert!(err.contains("capability"), "unexpected message: {err}");
    }

    // Slice 6h phase 1: the raw array constructor `[ Type ; Count ]`.

    #[test]
    fn array_constructor_with_concrete_type_parses() {
        let module = parse_src(": w ( -- ) [ i64 ; 4 ] drop ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(module.arrays.len(), 1);
        match &body[0].kind {
            TermKind::ArrayCtor(Type::Array(id, name)) => {
                assert_eq!(id.index(), 0);
                assert_eq!(*name, "[i64 4]");
            }
            other => panic!("expected ArrayCtor(Type::Array), got {other:?}"),
        }
    }

    #[test]
    fn array_constructor_interns_the_array_shape_once() {
        let module = parse_src(": w ( -- ) [ i64 ; 4 ] drop [ i64 ; 4 ] drop ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        let body = terms_body(&module.words[0]);
        let ty = |t: &Term| match &t.kind {
            TermKind::ArrayCtor(ty) => *ty,
            other => panic!("expected ArrayCtor, got {other:?}"),
        };
        assert_eq!(ty(&body[0]), ty(&body[2]));
    }

    #[test]
    fn array_constructor_type_declared_later_in_file_resolves() {
        // The type pre-pass registers every struct/enum name before any body
        // parses (driver.rs), so a constructor referring to a type declared
        // later in the same file resolves fine.
        let module = parse_src(
            ": w ( -- ) [ Later ; 4 ] drop ;\n\
             type: Later tag i64 ;\n",
        )
        .unwrap();
        let body = terms_body(&module.words[0]);
        assert!(matches!(
            &body[0].kind,
            TermKind::ArrayCtor(Type::Array(..))
        ));
    }

    #[test]
    fn array_constructor_missing_count_is_parse_error() {
        let err = parse_src(": w ( -- ) [ i64 ; ] drop ;").unwrap_err();
        assert!(err.contains("decimal literal"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_extra_token_after_count_is_parse_error() {
        let err = parse_src(": w ( -- ) [ i64 ; 4 5 ] drop ;").unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_non_literal_count_is_parse_error() {
        let err = parse_src(": w ( -- ) [ i64 ; n ] drop ;").unwrap_err();
        assert!(err.contains("decimal literal"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_zero_count_is_parse_error() {
        let err = parse_src(": w ( -- ) [ i64 ; 0 ] drop ;").unwrap_err();
        assert!(err.contains(">= 1"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_over_u32_max_count_is_parse_error() {
        let err = parse_src(": w ( -- ) [ i64 ; 4294967297 ] drop ;").unwrap_err();
        assert!(err.contains("invalid length"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_compound_element_type_is_parse_error() {
        // D1: the element read expects one word token, so a compound type
        // (`[i64 3]`) fails there rather than needing new logic.
        let err = parse_src(": w ( -- ) [ [i64 3] ; 4 ] drop ;").unwrap_err();
        assert!(err.contains("expected a word"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_bare_reference_element_is_rejected() {
        // Phase 2's exit criteria: a bare-reference element is a located
        // rejection naming the constructor's site, not `fill`'s. Unlike a
        // linear element (a plain word, resolved via `resolve_type` and
        // rejected by check.rs's shared gate), `&i64` is a single lexed word
        // that never reaches a registered type name -- `resolve_type_name_in_module`
        // has no `&`-prefix case (only `parse_type_expr`'s own dedicated arm
        // does), so this is caught here at parse time as an unknown type,
        // never reaching check.rs at all.
        let err = parse_src(": w ( -- ) [ &i64 ; 4 ] drop ;").unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("&i64"), "unexpected message: {err}");
    }

    #[test]
    fn quotation_without_a_semicolon_still_parses_as_a_quotation() {
        let module = parse_src(": w ( -- ) [ 1 2 drop ] drop ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert!(matches!(&body[0].kind, TermKind::Quotation(..)));
    }

    #[test]
    fn unterminated_quotation_without_a_semicolon_still_reports_unterminated() {
        // A `;`-containing unterminated quotation has never reported
        // "unterminated" (the depth scan returns `False` at EOF); this is the
        // no-`;` case, which must keep today's message. No closing `]` and no
        // `;` at all, mirroring the grounding fact's own probe.
        let err = parse_src(": w ( -- ) [ 1 2 drop").unwrap_err();
        assert!(
            err.contains("unterminated quotation"),
            "unexpected message: {err}"
        );
    }

    // -- Slice 11 (R1): the `inline` keyword ---------------------------------

    #[test]
    fn parse_worddef_inline_keyword_sets_flag() {
        let module = parse_src(": ClkDiv inline ( -- i64 i64 ) 8 4 ;").unwrap();
        let word = &module.words[0];
        assert_eq!(word.name, "ClkDiv");
        assert!(word.declares_inline);
        // The keyword is consumed, not read as the effect's first token: the
        // declared effect is still the one written after it.
        assert_eq!(word.effect.inputs.len(), 0);
        assert_eq!(word.effect.outputs.len(), 2);
    }

    #[test]
    fn parse_worddef_no_inline_keyword_flag_false() {
        let module = parse_src(": ClkDiv ( -- i64 i64 ) 8 4 ;").unwrap();
        assert!(!module.words[0].declares_inline);
    }

    #[test]
    fn parse_worddef_word_named_inline_is_not_inline() {
        // The name slot is consumed first, so `inline` in *that* position is an
        // ordinary word name and the definition declares nothing. This is why
        // the keyword needs no global reservation.
        let module = parse_src(": inline ( i64 -- i64 ) 1 add ;").unwrap();
        assert_eq!(module.words[0].name, "inline");
        assert!(!module.words[0].declares_inline);
    }

    #[test]
    fn parse_worddef_double_inline_is_parse_error() {
        // One optional keyword only: the second `inline` is not consumed, so it
        // falls through to `expect(LParen)` and fails there, located.
        let err = parse_src(": foo inline inline ( -- ) ;").unwrap_err();
        assert!(
            err.contains("expected LParen") && err.contains("line 1, col 14"),
            "unexpected message: {err}"
        );
    }
}
