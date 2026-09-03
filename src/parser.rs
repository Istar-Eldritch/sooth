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
//!   slot     := name? Word
//!   name     := Word ':'          \ spaced `a :`; glued `a:` is one Word token ending in ':'
//!   binding  := '|' Word+ '|'
//!   term     := Int | Word | binding | if
//!   if       := 'if' term* ('else' term*)? 'end'

use crate::ast::{
    fence_member_app_against_concrete_target, ground_member_poly, ground_member_type,
    intern_array_type, is_name_dispatched_builtin, member_app_abstract_target_error, ArrayDecl,
    Bound, EnumDecl, ExternDecl, GenericTypes, GlobalEntry, GlobalMode, ImplDecl, ImplTarget,
    Import, ImportAnchor, ImportBinding, ImportTarget, IntrinsicVisibility, Kind, Len,
    MemberGrounding, MemberVarMap, Module, ModuleInfo, ModuleName, MutRegistries, OwnedCellDecl,
    PolySig, PolyType, QuotAnnot, RefDecl, SliceDecl, Span, StackEffect, StaticDecl, StaticInit,
    StructDecl, Term, TermKind, TraitDecl, TraitId, TraitKind, TraitMember, Type, TypedSlot,
    VariantDecl, VariantTag, VariantTagMode, WordDef, OWNING_QUOTATION_KEYWORD,
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
                    if header_is_generic(tokens, i + 2) {
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

/// P7.S6 (R5/R10): whether a `type:` header at `start` is generic -- i.e.
/// whether it opens a bracketed type-variable list (`type: Box['T]`), which
/// is the only spelling. A bare lookahead, no consumption. Shared by the
/// pre-pass (which skips registering a generic header into the concrete
/// registries) and the parser's own lookahead before dispatching to the
/// generic or concrete production, so the three sites can never disagree.
/// The retired postfix form (`type: Box 'T`) is not merely non-generic: the
/// concrete productions raise `reject_postfix_header_var` on it rather than
/// letting it mis-parse as a concrete declaration.
fn header_is_generic(tokens: &[(Token, Span)], start: usize) -> bool {
    matches!(tokens.get(start), Some((Token::LBracket, _)))
}

/// P7.S6 (R10): the retired postfix header form, in which a `type:`/`trait:`
/// declaration bound its type variables as bare `'`-prefixed words after its
/// name. Without this the narrowed `header_is_generic` would silently classify
/// `type: Box 'T val 'T ;` as *concrete* and the field loop would blame `'T`
/// for not being a field name.
fn postfix_header_var_error(kind: &str, decl_name: &str, var: &str, span: Span) -> String {
    format!(
        "error: `{kind} {decl_name} {var}` at line {}, col {} binds its type variables in the retired postfix form; write `{kind} {decl_name}[{var}]`",
        span.line, span.col
    )
}

/// P7.S6 (R10): raise `postfix_header_var_error` when the token at `start` --
/// the one directly following a declaration's name -- is a `'`-prefixed word.
/// Called from both concrete `type:` productions, which is every entry path
/// that reads a `type:` header -- including the REPL's own `type:`-line
/// readers, which arrive without the module pre-pass. Not from the pre-pass:
/// there it would only fire on a `type:` the parser never dispatches to a
/// production at all (one inside a word body), where blaming the header form
/// hides the real defect.
fn reject_postfix_header_var(
    kind: &str,
    decl_name: &str,
    tokens: &[(Token, Span)],
    start: usize,
) -> Result<(), String> {
    if let Some((Token::Word(w), span)) = tokens.get(start) {
        if w.starts_with('\'') {
            return Err(postfix_header_var_error(kind, decl_name, w, *span));
        }
    }
    Ok(())
}

/// A located error for a name reserved by the owning-cell syntax (`^`, `^>`,
/// `^|>`, or any name beginning with `^`), used at every declaration site it
/// can arise: a `type:` name, a `:` word name, or a local binding.
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
    // P7.S6 (R3): `array` is the named array type's spelling, intercepted at
    // every type-position entry ahead of every user registry, so a type or
    // variant declared under that name would be silently unreachable rather
    // than merely shadowed -- the same reason `Slice` is reserved just above.
    if matches!(kind, "type" | "variant") && name == ARRAY_TYPE_NAME {
        return Err(format!(
            "error: `{name}` is reserved for the array type syntax (`{ARRAY_TYPE_NAME}[T N]`) and cannot be used as a {kind} name at line {}, col {}",
            span.line, span.col
        ));
    }
    // P7.S3h: `owning` is intercepted at every type-position entry ahead of
    // every user registry, so a type or variant declared under that name would
    // be silently unreachable rather than merely shadowed -- the same reason
    // `Slice` is reserved just above.
    if matches!(kind, "type" | "variant") && name == OWNING_QUOTATION_KEYWORD {
        return Err(format!(
            "error: `{name}` is reserved for the owning-quotation syntax (`{OWNING_QUOTATION_KEYWORD} [ <in> -- <out> ]`) and cannot be used as a {kind} name at line {}, col {}",
            span.line, span.col
        ));
    }
    // P7.S6a (R2.2): `Len` is the header bracket's kind annotation
    // (`'N: Len`), intercepted ahead of `parse_capabilities` unconditionally
    // by any bound bracket -- a user-declared trait named `Len` would
    // otherwise be silently unreachable from any bracket, the same reason
    // `Slice`/`array` are reserved above.
    if kind == "trait" && name == LEN_KIND_NAME {
        return Err(format!(
            "error: `{name}` is reserved for the header bracket's kind annotation (`'N: {name}`) and cannot be used as a {kind} name at line {}, col {}",
            span.line, span.col
        ));
    }
    Ok(())
}

/// P7.S6a (R2.2): the only spellable header-bracket kind annotation
/// (`'N: Len`). Shared between `reject_reserved_name` (which reserves the
/// name against `trait:`) and `parse_header_bracket` (which accepts the
/// spelling) so the two can't drift apart.
pub const LEN_KIND_NAME: &str = "Len";

/// P7.S6 (R1): the named array type's spelling. `array[T N]` resolves
/// through the interned array registry, so it is intercepted by name ahead
/// of every user lookup exactly as `Slice[T]` is. Reserved against
/// `type:`/variant names by `reject_reserved_name`.
pub const ARRAY_TYPE_NAME: &str = "array";

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
/// concrete/array/reference shapes over `'T` -- `ast`'s
/// `ground_member_type`, which the body-form desugar grounds each member with
/// against a concrete `impl:` target, handles exactly these and nothing else.
///
/// P7b.S2 (S2-3) adds the two HKT arms:
///
/// - `App` -- supported iff the head is the trait var (id 0); the
///   application's arity is validated against the target ctor at grounding
///   time (S2-7), not here (the target is unknown at member parse). An App
///   headed by a member *local* has no dispatch story this slice.
/// - `Quotation` -- supported iff its rows are App-free (and each row is
///   itself a supported shape): an `App` inside a member quotation row is a
///   located fence (S2-15.d, F10 -- declarations represent it, but `call`
///   cannot see through one; a later slice's extension).
fn member_shape_is_supported(t: &PolyType) -> bool {
    match t {
        PolyType::Concrete(_) | PolyType::Var(_) => true,
        PolyType::Array(elem, Len::Concrete(_)) => member_shape_is_supported(elem),
        PolyType::Array(_, Len::Var(_)) => false,
        PolyType::Ref(referent, _) => member_shape_is_supported(referent),
        // P7b.S2 (S2-3): the HKT dispatchable shape -- only the trait's own
        // variable may head an application in a member signature.
        PolyType::App { head, .. } => *head == 0,
        // P7b.S2 (S2-3): a declared quotation parameter whose rows stay
        // App-free (and shape-supported) is representable; anything else in
        // a row goes through the same predicate recursively.
        PolyType::Quotation(ins, outs, ..) => {
            ins.iter().chain(outs).all(member_shape_is_supported)
                && !member_quotation_row_mentions_app(t)
        }
        // P7.S3n (R3): the new owned-cell shape is deliberately *not* added
        // to the supported set -- `ground_member_type` has no cell arm, so a
        // `^'T` member would ground to nothing. A located rejection, not a
        // wildcard fall-through.
        PolyType::OwnedCell(_)
        | PolyType::Generic { .. }
        | PolyType::QuotLit
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in a trait member signature.
        | PolyType::GenericVariant { .. } => false,
        // (P7b.S2 S2-3: an application headed by anything but the trait var
        // -- a member local, say -- is unsupported; that is the `App` arm
        // above returning `*head == 0`, not a wildcard fall-through.)
    }
}

/// P7b.S2 (S2-15.d/F10): whether a type application is reachable anywhere in
/// `t` -- including `t` itself in plain position (the top-level `App` arm).
/// `App` *in a plain slot* is the supported dispatchable shape (S2-3); an App
/// *inside a quotation row* is the fenced one -- the rows are exactly what
/// `call` will pop and run, and it cannot see through a type-level
/// application. The S2-15.d dispatch therefore pairs this predicate with
/// `poly_type_app_head` (which finds a plain-position App head but does not
/// recurse into rows): `mentions_app && app_head.is_none()` isolates the
/// row-nested case. Recurses through nested shapes so an App buried under an
/// array element or a nested quotation is caught too.
fn member_quotation_row_mentions_app(t: &PolyType) -> bool {
    match t {
        PolyType::App { .. } => true,
        PolyType::Concrete(_) | PolyType::Var(_) | PolyType::QuotLit => false,
        PolyType::Array(elem, _) => member_quotation_row_mentions_app(elem),
        PolyType::Ref(referent, _) => member_quotation_row_mentions_app(referent),
        PolyType::OwnedCell(inner) => member_quotation_row_mentions_app(inner),
        PolyType::Quotation(ins, outs, ..) => ins
            .iter()
            .chain(outs)
            .any(member_quotation_row_mentions_app),
        PolyType::Generic { args, .. } | PolyType::GenericVariant { args, .. } => {
            args.iter().any(member_quotation_row_mentions_app)
        }
    }
}

/// P7.S3e (R4/R8): a trait member signature mentions an unsupported shape
/// (an owned cell, a generic application over a non-trait head, or a
/// variable-length array) -- see `member_shape_is_supported`.
fn unsupported_trait_member_shape_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}`'s member at line {}, col {} has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)",
        span.line, span.col
    )
}

/// P7b.S2 (S2-15.d, F10): a type application inside a member quotation row.
/// Declarations *represent* the shape, but body-level `call` cannot see
/// through it -- fenced at the member grammar rather than left to fail at
/// the (later-slice) consumer.
fn app_in_member_quotation_row_error(trait_name: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}`'s member at line {}, col {} applies a type variable inside a quotation row (`'F[...]` may not appear inside `[ ... ]`)\n  note: a type application is supported only in a plain signature slot; keep quotation rows App-free",
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

/// P7.S3s-follow: the retired bare `name ( sig )` trait member form. A word
/// where `:` or `;` is expected is almost always the old grammar, so the
/// error names the replacement rather than reporting a token mismatch.
fn bare_trait_member_error(trait_name: &str, member: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}` declares member `{member}` without a leading `:` at line {}, col {}\n  note: a trait member is declared `: {member} ( ... ) ;`, the same form as a word definition",
        span.line, span.col
    )
}

// P7.S4 (R5/R6): render a `PolyType` target shape for the synthesized member
// word name, so two generic impls for one trait (`['T N]` and `array['T 4]`)
// produce distinct names. Unlike `poly_type_str` (which needs a `PolySig`
// for variable name tables), this uses positional ids (`'T0`, `'N0`) since
// the synth name is a compiler-internal spelling, never shown to the user.
fn poly_type_shape_str(pt: &PolyType) -> String {
    match pt {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => format!("'T{v}"),
        PolyType::Array(elem, len) => {
            let l = match len {
                Len::Concrete(n) => n.to_string(),
                Len::Var(id) => format!("'N{id}"),
            };
            format!("[{} {}]", poly_type_shape_str(elem), l)
        }
        PolyType::Ref(referent, mutable) => {
            format!(
                "&{}{}",
                if *mutable { "!" } else { "" },
                poly_type_shape_str(referent)
            )
        }
        PolyType::OwnedCell(payload) => format!("^{}", poly_type_shape_str(payload)),
        PolyType::Generic {
            name,
            args,
            len_args,
            ..
        } => {
            let mut parts: Vec<String> = args.iter().map(poly_type_shape_str).collect();
            parts.extend(len_args.iter().map(|l| match l {
                Len::Concrete(n) => n.to_string(),
                Len::Var(id) => format!("'N{id}"),
            }));
            format!("{name}[{}]", parts.join(" "))
        }
        PolyType::Quotation(_, _, _, _, _) => "[quotation]".to_string(),
        PolyType::QuotLit => "[quotlit]".to_string(),
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never in an `impl:` target shape.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it never reaches an impl target shape"
        ),
        // P7b.S1: `member_shape_is_supported` rejects an `App` shape before
        // this renderer is ever called on one; kept as a real render (not
        // an `unreachable!`) since it costs nothing and matches `App`'s
        // other renderer, `poly_type_str`.
        PolyType::App { head, args } => {
            let parts: Vec<String> = args.iter().map(poly_type_shape_str).collect();
            format!("'T{head}[{}]", parts.join(" "))
        }
    }
}

/// P7b.S2 (S2-4): what a generic application's bracket does when it closes
/// before the constructor's declared arity. A signature-site application is
/// the shared arity error (`generic_arity_error`, F3); an `impl:` target
/// desugars the missing slots to fresh pattern variables (`for Box` ≡
/// `for Box['ctor0 …]`, `for Result[i64]` ≡ `for Result[i64 'ctor1]`),
/// m2-proven mechanics that make bare ctors expressible as existing S4
/// applied-var patterns with everything downstream unchanged.
enum UnderApplication {
    Error,
    PadImplTarget,
}

/// P7b.S2 (S2-4): what `parse_impl_target_pattern` reads back -- the folded
/// pattern; when the ctor path was taken, the ctor name's own span (the
/// desugared variables' introduction span and the anchor the user-spelling
/// diagnostics render); and the exact number of type/length slots the
/// desugar padded, which the user-spelling renderer consumes in place of
/// the retired name-prefix heuristic (P7b.S2 review).
struct RawImplTargetPattern {
    raw: RawTy,
    ctor_span: Option<Span>,
    padded: (usize, usize),
}

/// P7b.S2 (S2-4): the user's own spelling of a desugared ctor target --
/// the ctor name plus its explicit prefix arguments, rendered from the
/// folded pattern (`Some` only when the desugar actually padded; `padded`
/// is the exact number of type/length slots the desugar filled with fresh
/// variables, threaded from `pad_impl_ctor_slots` -- the review-round fix
/// that retired the old `'ctor`-prefix name heuristic, which misread a
/// fully-applied user spelling as padding). Diagnostics render `Option`,
/// not `Option['ctor0]`.
fn impl_target_user_spelling(
    pattern: &PolyType,
    ty_var_names: &[String],
    len_var_names: &[String],
    padded: (usize, usize),
    span: Span,
) -> Option<(String, Span)> {
    let PolyType::Generic {
        name,
        args,
        len_args,
        ..
    } = pattern
    else {
        return None;
    };
    // The desugar's padding is always a suffix of each argument list, so
    // the explicit prefix is the leading run it did not fill.
    let explicit_ty = args.len().saturating_sub(padded.0);
    let explicit_len = len_args.len().saturating_sub(padded.1);
    if explicit_ty == args.len() && explicit_len == len_args.len() {
        // Nothing was desugared: the pattern already is the user's spelling.
        return None;
    }
    if explicit_ty == 0 && explicit_len == 0 {
        // A bare ctor target (`for Box`): no explicit prefix at all, so the
        // user's spelling is just the name, no empty brackets.
        return Some((name.to_string(), span));
    }
    let mut parts: Vec<String> = args[..explicit_ty]
        .iter()
        .map(|a| render_target_pt(a, ty_var_names, len_var_names))
        .collect();
    parts.extend(len_args[..explicit_len].iter().map(|l| {
        match l {
            Len::Concrete(n) => n.to_string(),
            Len::Var(v) => len_var_names
                .get(*v as usize)
                .cloned()
                .unwrap_or_else(|| format!("'{v}")),
        }
    }));
    Some((format!("{name}[{}]", parts.join(" ")), span))
}

/// P7b.S2 (S2-5): the member-word id-space union for one impl member body.
/// `map` is indexed by the member signature's own variable ids (the header
/// var dissolves; each local renders into the union space), `appended` the
/// freshly appended locals' names and member-sig spans, in S2-5's order.
struct MemberVarUnion {
    map: MemberVarMap,
    appended: Vec<(String, Span)>,
}

/// P7b.S2 (S2-5): union the target's variables with a member signature's
/// own locals. The target's variables keep their ids and order (so every
/// `where`-bound, keyed by target id, survives the merge); within the
/// member sig, a name in an identifying position (an App argument of the
/// dispatchable input) binds to the target slot's contents for the whole
/// sig, and a local not in an identifying position appends after the
/// target's variables, never renumbered -- and must not collide with a
/// target variable name, which would make the merged signature text
/// ambiguous (a located desugar error, outside the S2-15 family).
fn build_member_var_union(
    target: &ImplTarget,
    sig: &PolySig,
    dg: &MemberGrounding,
) -> Result<MemberVarUnion, String> {
    let mut map: Vec<Option<PolyType>> = vec![None; sig.ty_var_names.len()];
    // Identification: every dispatchable input (S2-2 reads one `Ref` layer;
    // ref-ness is an addressing mode, not a type identity) that is an
    // application headed by the trait var binds its argument names to the
    // target slots' contents. Two applications reading the same leading
    // slots is the pinned reading for one trait head -- the slot variables
    // coincide (an Applicative-shaped `ap`); the same local named at two
    // *different* slots is an ambiguity this desugar refuses.
    for input in &sig.inputs {
        let app = match input {
            PolyType::Ref(referent, _) => referent,
            other => other,
        };
        let PolyType::App { head: 0, args } = app else {
            continue;
        };
        let PolyType::Generic {
            args: target_args, ..
        } = &target.pattern
        else {
            // A fully-abstract (`Var`) target names no constructor for the
            // application to dissolve into: raise the located S2-15.e twin
            // here, at the union build (P7b.S2 review), rather than letting
            // the collision check below win -- a member local whose name
            // happens to match the target variable's would otherwise report
            // the name clash when the real problem is the abstract target.
            return Err(member_app_abstract_target_error(dg));
        };
        for (i, arg) in args.iter().enumerate() {
            let PolyType::Var(v) = arg else {
                // A pinned argument (`'F[i64]`) renders as itself; only a
                // *name* in an identifying position binds.
                continue;
            };
            if *v == 0 {
                // The header cannot be its own application's argument (the
                // member parser's kind machinery rejects it).
                continue;
            }
            if i >= target_args.len() {
                // Over-applied: the grounding App arm raises the located
                // S2-15.c arity error; identification reads no slot here.
                continue;
            }
            let rendered = target_args[i].clone();
            match &map[*v as usize] {
                None => map[*v as usize] = Some(rendered),
                Some(existing) if *existing == rendered => {}
                Some(existing) => {
                    return Err(member_local_reidentified_error(
                        dg, target, existing, &rendered,
                    ));
                }
            }
        }
    }
    // Append the unbound locals after the target's variables, in the
    // member sig's own declaration order, never renumbered.
    let mut appended = Vec::new();
    for (v, name) in sig.ty_var_names.iter().enumerate().skip(1) {
        if map[v].is_some() {
            continue;
        }
        if let Some(t) = target.ty_var_names.iter().position(|t| t == name) {
            return Err(member_local_collides_with_target_error(
                dg,
                name,
                sig.ty_var_spans.get(v).copied().unwrap_or_default(),
                &target.ty_var_names[t],
                target.ty_var_spans.get(t).copied().unwrap_or_default(),
            ));
        }
        let id = (target.ty_var_names.len() + appended.len()) as u32;
        map[v] = Some(PolyType::Var(id));
        appended.push((
            name.clone(),
            sig.ty_var_spans.get(v).copied().unwrap_or_default(),
        ));
    }
    Ok(MemberVarUnion { map, appended })
}

/// P7b.S2 (S2-5): one member-local name bound by two different dispatchable
/// input slots. The sig text would claim the local aliases two target slots
/// at once; refused rather than silently last-write-wins. The two slots
/// render over the target's own name tables, so the message names the
/// variables as the user spelled them (P7b.S2 review: the arm's first unit
/// test pins this).
fn member_local_reidentified_error(
    dg: &MemberGrounding,
    target: &ImplTarget,
    first: &PolyType,
    second: &PolyType,
) -> String {
    format!(
        "error: trait member `{}` of `{}` (line {}, col {}) binds the same local name to two different target slots (`{}` and `{}`)\n  a name in an identifying position binds one slot for the whole signature; spell the second occurrence with its own name",
        dg.member,
        dg.trait_name,
        dg.member_span.line,
        dg.member_span.col,
        render_target_pt(first, &target.ty_var_names, &target.len_var_names),
        render_target_pt(second, &target.ty_var_names, &target.len_var_names)
    )
}

/// P7b.S2 (S2-5): a member local that no dispatchable-input argument
/// identifies reuses a target variable's name -- the merged signature's
/// text would be ambiguous between the local and the target variable.
fn member_local_collides_with_target_error(
    dg: &MemberGrounding,
    local: &str,
    local_span: Span,
    target_var: &str,
    target_span: Span,
) -> String {
    format!(
        "error: trait member `{}` of `{}` (line {}, col {}) declares local `{local}` at line {}, col {}, which is also the impl target's variable `{target_var}` at line {}, col {}\n  a member local that no dispatchable-input argument identifies must not reuse a target variable's name; rename the local",
        dg.member,
        dg.trait_name,
        dg.member_span.line,
        dg.member_span.col,
        local_span.line,
        local_span.col,
        target_span.line,
        target_span.col
    )
}

/// P7b.S2 (S2-4): a compact surface rendering of a target-side `PolyType`
/// over the impl target's own name tables -- the user-spelling prefix
/// renderer and the non-desugared target's display for the grounding
/// diagnostics. `GenericVariant`/`QuotLit` are unconstructible in an impl
/// target pattern; an `App` is fenced before any rendering runs.
fn render_target_pt(pt: &PolyType, ty_var_names: &[String], len_var_names: &[String]) -> String {
    let ty = |v: u32| {
        ty_var_names
            .get(v as usize)
            .cloned()
            .unwrap_or_else(|| format!("'{v}"))
    };
    let len = |l: &Len| match l {
        Len::Concrete(n) => n.to_string(),
        Len::Var(v) => len_var_names
            .get(*v as usize)
            .cloned()
            .unwrap_or_else(|| format!("'{v}")),
    };
    match pt {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => ty(*v),
        PolyType::Array(elem, l) => format!(
            "array[{} {}]",
            render_target_pt(elem, ty_var_names, len_var_names),
            len(l)
        ),
        PolyType::Ref(referent, mutable) => format!(
            "&{}{}",
            if *mutable { "!" } else { "" },
            render_target_pt(referent, ty_var_names, len_var_names)
        ),
        PolyType::OwnedCell(payload) => {
            format!(
                "^{}",
                render_target_pt(payload, ty_var_names, len_var_names)
            )
        }
        PolyType::App { head, args } => format!(
            "{}[{}]",
            ty(*head),
            args.iter()
                .map(|a| render_target_pt(a, ty_var_names, len_var_names))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        PolyType::Generic {
            name,
            args,
            len_args,
            ..
        } => {
            let mut parts: Vec<String> = args
                .iter()
                .map(|a| render_target_pt(a, ty_var_names, len_var_names))
                .collect();
            parts.extend(len_args.iter().map(len));
            format!("{name}[{}]", parts.join(" "))
        }
        PolyType::Quotation(..) => "[…]".to_string(),
        PolyType::QuotLit | PolyType::GenericVariant { .. } => {
            unreachable!("unconstructible in an impl target pattern (R3.5/the target grammar)")
        }
    }
}

/// P7.S4 (R1): an `impl:` target with a bound on one of its variables
/// (`'T: Copy`). Bounds on impl variables are out of scope this slice.
fn impl_target_bound_error() -> String {
    "error: an `impl:` target variable may not carry an inline bound; use a `where`-clause instead (e.g. `impl: Show for array['T 'N] where 'T: Show`)".to_string()
}

/// P7.S4 (R1): an `impl:` target with a row variable (`..s`). Row variables
/// are not meaningful in an impl target.
fn impl_target_row_var_error() -> String {
    "error: an `impl:` target may not carry a row variable".to_string()
}

/// P7b.S2 review (S2-4): a user-written variable inside an `impl:` target
/// pattern whose name carries the ctor desugar's reserved prefix. The
/// bare/partial ctor desugar pads missing slots with fresh variables named
/// `'ctorN` (type slots) / `'ctorlenN` (length slots), interning them *by
/// name* -- a user variable with such a name would alias a padded slot (one
/// variable silently standing for two slots, no diagnostic) and would be
/// misread as desugar padding by the user-spelling renderer, so the whole
/// prefix is reserved inside impl targets.
fn impl_target_reserved_var_error(name: &str, span: Span) -> String {
    format!(
        "error: an `impl:` target may not declare the variable `{name}` at line {}, col {}\n  names starting with the reserved prefix `'ctor` (`'ctor0`, `'ctorlen0`, …) belong to the bare/partial ctor desugar's own fresh pattern variables; pick another name",
        span.line, span.col
    )
}

/// P7b.S1 review fix (P1), sibling of S1-17.i's `poly_cross_call_unsupported_
/// error`: an `impl:` target whose pattern applies one of its own variables
/// (`impl: Trait for 'F['T]`) used to parse silently, but
/// `match_impl_target_rec` (S1-16's census) never matches an `App` pattern
/// -- the impl would register and then never dispatch, surfacing as a
/// misleading "does not satisfy" error at an unrelated call site instead of
/// naming the real problem here. A located rejection at parse time instead,
/// naming the applied variable's own binding span.
fn impl_target_app_unsupported_error(var: &str, span: Span) -> String {
    format!(
        "error: an `impl:` target may not apply its own type variable (`{var}[...]` at line {}, col {}); a constructor-abstract impl target is not supported this slice",
        span.line, span.col
    )
}

/// The first `PolyType::App`'s head variable found in `pty`'s *plain*
/// structure, if any -- the predicate half of two dispatches:
/// `parse_impl_target`'s applied-own-variable fence
/// (`impl_target_app_unsupported_error`) and `parse_trait_member_effect`'s
/// S2-15.d row-scoped dispatch, which pairs it with
/// `member_quotation_row_mentions_app` (that predicate answers "an App
/// exists anywhere", so `mentions && app_head.is_none()` is what isolates
/// the row-nested case). Quotation rows are deliberately *not* descended
/// into: a row-nested App must read as `None` here or the row-scoped
/// dispatch could never isolate its case. Exhaustive over `PolyType` so a
/// future variant that can carry an `App` (a new compound shape) is forced
/// to extend this arm rather than silently skip one of the two dispatches.
fn poly_type_app_head(pty: &PolyType) -> Option<u32> {
    match pty {
        PolyType::App { head, .. } => Some(*head),
        PolyType::Array(elem, _) => poly_type_app_head(elem),
        PolyType::Ref(referent, _) => poly_type_app_head(referent),
        PolyType::OwnedCell(payload) => poly_type_app_head(payload),
        PolyType::Generic { args, .. } => args.iter().find_map(poly_type_app_head),
        PolyType::Concrete(_)
        | PolyType::Var(_)
        | PolyType::Quotation(..)
        | PolyType::QuotLit
        | PolyType::GenericVariant { .. } => None,
    }
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
///
/// P7.S3p widened the stakes for `call`, `slice`, and `subslice`. Bound-directed
/// dispatch now selects a member by *name* over the body's bounds, ahead of
/// every builtin arm, so a member sharing one of these names captures every
/// call to that builtin in any body bounded by its trait -- the builtin
/// becomes unreachable there, not merely shadowed inside the impl. The six
/// surface comparisons stay legal (they are `lib/` words, and a body that
/// imports one receives it mangled), but these three need no import and have
/// no mangled spelling, so only rejection closes them.
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
/// grounded signatures to differ -- guaranteed here, because a member must
/// take `'T`/`&'T` as some input
/// (`check::declarations::member_binds_trait_var`) and so every grounded
/// signature mentions the `for` type.
///
/// P7.S4 (R5/R6): for a generic target the `PolyType` shape is rendered via
/// `poly_type_shape_str` so two generic impls for one trait (e.g. `['T N]`
/// and `array['T 4]`) produce distinct synth names. A concrete target keeps the
/// existing `target.name()` rendering.
fn synth_member_word_name(
    member: &str,
    trait_name: &str,
    trait_module: u32,
    target: &ImplTarget,
) -> String {
    let mut ty_part = match &target.pattern {
        PolyType::Concrete(t) => t.name().to_string(),
        other => poly_type_shape_str(other),
    };
    if !target.bounds.is_empty() {
        ty_part.push_str(&bound_set_suffix(&target.bounds));
    }
    format!("{member};{trait_name};{trait_module};{ty_part}")
}

/// P7.S4b (R2): a deterministic rendering of an impl's `where`-clause bound
/// set, appended to the synthesized member word name so two impls sharing a
/// pattern but differing only in bounds (e.g. `for ['T N]` with and without
/// `where 'T: Print`) don't collide (src/parser.rs:516's `synth_member_word_name`).
/// Sorted by `var_idx` so the rendering doesn't depend on `where`-clause
/// source order.
fn bound_set_suffix(bounds: &[(u32, Bound)]) -> String {
    let mut sorted: Vec<&(u32, Bound)> = bounds.iter().collect();
    sorted.sort_by_key(|(idx, _)| *idx);
    let mut out = String::from(";where");
    for (idx, bound) in sorted {
        let bound_part = match bound {
            Bound::Copy => "Copy".to_string(),
            Bound::User(trait_id) => format!("User{}", trait_id.index()),
        };
        out.push_str(&format!(",{idx}:{bound_part}"));
    }
    out
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
                TermKind::Call(name, type_args, len_args) if name == member => {
                    TermKind::Call(synth.to_string(), type_args.clone(), len_args.clone())
                }
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
///
/// P7.S3s (R1/C4): both the qualified and the selective branch are one-hop
/// only, and each falls back to `trait_origin` (mirroring `type_origin`'s
/// fallback in `resolve_type_name_in_module`) when the direct match fails --
/// a trait re-exported through a hub module, not declared there, resolves
/// through the hub's own recorded origin instead of stopping at the hub.
pub(crate) fn find_trait_in_module(
    traits: &[TraitDecl],
    name: &str,
    module: u32,
    imports: &HashMap<String, u32>,
    selective: &HashMap<String, u32>,
    trait_origin: &[HashMap<String, u32>],
) -> Option<TraitId> {
    if let Some((qualifier, base)) = name.split_once("::") {
        let target = *imports.get(qualifier)?;
        return traits
            .iter()
            .position(|t| t.name == base && t.module == target)
            .or_else(|| {
                let origin = *trait_origin.get(target as usize)?.get(base)?;
                traits
                    .iter()
                    .position(|t| t.name == base && t.module == origin)
            })
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
            .or_else(|| {
                let origin = *trait_origin.get(target as usize)?.get(name)?;
                traits
                    .iter()
                    .position(|t| t.name == name && t.module == origin)
            })
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
/// shapes intern into the shared registries so two files' `array[i64 8]` dedupe to
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
    trait_origin: &[HashMap<String, u32>],
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
        trait_origin,
        generics,
        field_kind_marks: std::collections::HashMap::new(),
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
        field_kind_marks: std::collections::HashMap::new(),
        type_origin: &[],
        trait_origin: &[],
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
                field_kind_marks: std::collections::HashMap::new(),
                type_origin: &[],
                trait_origin: &[],
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
        poly_cross_calls: HashMap::new(),
        transitive_instantiations: Vec::new(),
        splice_records: HashMap::new(),
        splice_trait_calls: HashMap::new(),
        splice_enum_words: std::collections::HashMap::new(),
        builtin_overloads: HashMap::new(),
        resolved_fields: HashMap::new(),
        generics,
        resolved_variant_fields: HashMap::new(),
        modules: vec![ModuleInfo {
            imports: HashMap::new(),
            exports: bodies.exports,
            selective: HashMap::new(),
            // P8 S2 (R2): the single-file, no-driver path (`parser::parse`,
            // used by every in-process test). It resolves no `import:` at
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
                field_kind_marks: std::collections::HashMap::new(),
                type_origin: &[],
                trait_origin: &[],
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
                field_kind_marks: std::collections::HashMap::new(),
                type_origin: &[],
                trait_origin: &[],
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

/// P7.S3n (R2): a generic `type:` header on its own -- the declared name, the
/// bound type variables with their spans (for the phantom and duplicate
/// diagnostics), and the `type:` keyword's span. Registered as a placeholder
/// by `parse_generic_typedefs`' stage (a), then handed back to stage (b) to
/// parse that header's own field/variant list against.
type GenericHeader = (
    String,
    Vec<(String, Span)>,
    Vec<Kind>,
    Vec<(String, Span)>,
    Span,
);

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
    /// (`&'T`, `&!array['T 'N]`), folded to `PolyType::Ref` -- or to
    /// `Concrete(Type::Ref)` when the referent folds fully concrete -- by
    /// `raw_to_poly_type`.
    Ref(Box<RawTy>, bool),
    /// P7.S3n (R3): a `^`-led slot whose payload may itself be variable
    /// (`^'T`, `^array['T 4]`), folded to `PolyType::OwnedCell` -- or to
    /// `Concrete(Type::OwnedCell)` when the payload folds fully concrete --
    /// by `raw_to_poly_type`, exactly as `Ref` folds.
    OwnedCell(Box<RawTy>),
    /// P7b.S1 (S1-7): a type variable applied to type arguments
    /// (`'F['T]`), folded to `PolyType::App` by `raw_to_poly_type` -- there
    /// is no all-concrete fold, since a variable head never grounds to a
    /// `Type` at parse time (unlike `Generic`'s named header).
    App {
        head: u32,
        args: Vec<RawTy>,
    },
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
        /// Slice 6a (R7): the header's own length-argument list, parallel
        /// to `args` -- a bare `'N` interns a length variable through the
        /// enclosing `PolyBuilder` and lands as `RawLen::Var`; a literal
        /// count lands as `RawLen::Concrete`.
        len_args: Vec<RawLen>,
        name: String,
        span: Span,
    },
}

enum RawLen {
    Concrete(u32),
    Var(u32),
}

/// `Kind` is `ast::Kind` (P7b.S1 promotion, R1): the parser no longer
/// defines it, since a variable's kind must now travel past the parser (an
/// `Arrow`-kinded variable is still a *type* variable, so the checker needs
/// to see it too).
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
    kind: HashMap<String, Kind>,
    /// P7b.S1 (R2/S1-4): each ty/len var's first-mention span, parallel to
    /// `ty_names`/`len_names`, retained past `finish` for
    /// `attach_bracket_bounds`'s annotation-vs-usage conflict diagnostic.
    ty_var_spans: Vec<Span>,
    len_var_spans: Vec<Span>,
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
    /// P7.S4 (R1): when `true`, only a *glued* `'T:` counts as a bound, so
    /// the `:` starting a member body (`: show ...`) is not mistaken for a
    /// bound colon. Set by `parse_impl_target`. P7.S6 (R7a): it is also the
    /// selector between `parse_poly_ty_var`'s two rejections -- an `impl:`
    /// target gets `impl_target_bound_error`, a word or trait-member effect
    /// gets `bound_in_effect_error`.
    forbid_bounds: bool,
    /// P7b.S1 (S1-3/S1-4, Phase 3): each type variable's kind as
    /// established by its *first* usage-establishing mention (a bare
    /// mention establishes `Star`; an application head establishes
    /// `Arrow`), keyed by ty var id. First mention binds; every later
    /// mention checks against it (`mark_ty_star`/`mark_ty_arrow`) -- the
    /// binding span for a conflict diagnostic is always `ty_var_spans[id]`,
    /// since that is unconditionally the very first mention regardless of
    /// which kind it establishes.
    ty_established_kind: HashMap<u32, Kind>,
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

    /// P7b.S2 (S2-1): pre-establish a variable's kind from its *binding*
    /// site -- the trait header's bracket annotation -- before the effect is
    /// parsed, so `mark_ty_star`/`mark_ty_arrow` check every mention against
    /// the declared kind instead of letting the first mention bind it (the
    /// mechanism `attach_bracket_bounds`' annotation-vs-usage comparison
    /// reads back for word-level bound brackets). Only ever called for a
    /// variable whose id was just freshly interned, so the insert cannot
    /// clobber a genuine usage-derived kind.
    fn seed_ty_kind(&mut self, id: u32, kind: Kind) {
        self.ty_established_kind.insert(id, kind);
    }

    /// Intern a type variable, returning its id and whether this is its
    /// binding (first) occurrence. A name already seen in a count position is
    /// X1.
    fn intern_ty_var(&mut self, name: &str, span: Span) -> Result<(u32, bool), String> {
        if self.kind.get(name) == Some(&Kind::Len) {
            return Err(var_kind_conflict_error(name, span));
        }
        self.kind.insert(name.to_string(), Kind::Star);
        if let Some(&id) = self.ty_index.get(name) {
            return Ok((id, false));
        }
        let id = self.ty_names.len() as u32;
        self.ty_names.push(name.to_string());
        self.ty_index.insert(name.to_string(), id);
        // P7b.S1 (R2/S1-4): the first-mention span, retained past `finish`
        // so `attach_bracket_bounds` can report an annotation-vs-usage
        // conflict (S1-15.c) with both spans.
        self.ty_var_spans.push(span);
        Ok((id, true))
    }

    /// P7b.S1 (S1-3/S1-4): record a *bare* mention of type variable `id` --
    /// a plain type slot, or an application argument (S1-2: an application's
    /// arguments are always `Star`-kind slots). First mention establishes
    /// `Star`; a later bare mention is consistent by construction; a var
    /// already established `Arrow` by an earlier application-head mention
    /// is S1-15.b, "an arrow-kind variable used bare".
    fn mark_ty_star(&mut self, id: u32, span: Span) -> Result<(), String> {
        match self.ty_established_kind.get(&id).cloned() {
            None => {
                self.ty_established_kind.insert(id, Kind::Star);
                Ok(())
            }
            Some(Kind::Star) => Ok(()),
            Some(arrow @ Kind::Arrow { .. }) => Err(arrow_var_used_bare_error(
                &self.ty_names[id as usize],
                span,
                self.ty_var_spans[id as usize],
                &arrow,
            )),
            Some(Kind::Len) => unreachable!(
                "a ty var id never carries Len: intern_ty_var/intern_len_var already reject that at X1"
            ),
        }
    }

    /// P7b.S1 (S1-3/S1-4): record an application-head mention of type
    /// variable `id`, applied to `domain_count` type arguments. First
    /// mention establishes `Arrow { domains: [Star; domain_count], .. }`; a
    /// later application-head mention with the *same* arity is consistent;
    /// a different arity is S1-15.d, "application arity conflicting with
    /// inferred kind"; a var already established `Star` by an earlier bare
    /// mention is S1-15.a, "star-kind variable applied like a constructor".
    fn mark_ty_arrow(&mut self, id: u32, domain_count: usize, span: Span) -> Result<(), String> {
        let new_kind = Kind::Arrow {
            domains: vec![Kind::Star; domain_count],
            result: Box::new(Kind::Star),
        };
        match self.ty_established_kind.get(&id).cloned() {
            None => {
                self.ty_established_kind.insert(id, new_kind);
                Ok(())
            }
            Some(Kind::Arrow { domains, .. }) if domains.len() == domain_count => Ok(()),
            Some(Kind::Arrow { domains, .. }) => Err(application_arity_conflict_error(
                &self.ty_names[id as usize],
                span,
                domain_count,
                self.ty_var_spans[id as usize],
                domains.len(),
            )),
            Some(Kind::Star) => Err(star_applied_like_constructor_error(
                &self.ty_names[id as usize],
                span,
                self.ty_var_spans[id as usize],
            )),
            Some(Kind::Len) => unreachable!(
                "a ty var id never carries Len: intern_ty_var/intern_len_var already reject that at X1"
            ),
        }
    }

    /// P7b.S2 review (S2-4): inside an `impl:` target's builder
    /// (`forbid_bounds`, set by `parse_impl_target`), the ctor desugar's
    /// `'ctor…` name prefixes are reserved for its own generated pattern
    /// variables. Every user-written variable in an impl target passes
    /// through this check before interning -- the type-slot path
    /// (`parse_poly_ty_var`) and both length-slot paths (`parse_poly_array`,
    /// a ctor application's length arguments) -- so a generated pad name can
    /// never alias one (S2-4 review: one variable silently standing for two
    /// slots). Signature-site builders (`forbid_bounds == false`) are
    /// unaffected: a word signature may still spell `'ctor0` itself.
    fn check_impl_target_reserved_name(&self, name: &str, span: Span) -> Result<(), String> {
        if self.forbid_bounds && name.starts_with("'ctor") {
            return Err(impl_target_reserved_var_error(name, span));
        }
        Ok(())
    }

    /// Intern a length variable (an array count `'N`). A name already seen in
    /// a type position is X1.
    fn intern_len_var(&mut self, name: &str, span: Span) -> Result<u32, String> {
        if self.kind.get(name) == Some(&Kind::Star) {
            return Err(var_kind_conflict_error(name, span));
        }
        self.kind.insert(name.to_string(), Kind::Len);
        if let Some(&id) = self.len_index.get(name) {
            return Ok(id);
        }
        let id = self.len_names.len() as u32;
        self.len_names.push(name.to_string());
        self.len_index.insert(name.to_string(), id);
        self.len_var_spans.push(span);
        Ok(id)
    }

    fn finish(self, inputs: Vec<PolyType>, outputs: Vec<PolyType>) -> PolySig {
        // P7b.S1 (S1-3/S1-9, Phase 3 consumer): each var's resolved kind,
        // read back from `mark_ty_star`/`mark_ty_arrow`'s bookkeeping --
        // `Star` for any id that somehow never went through either marker
        // (there is no such path today: every mention reaches one or the
        // other), never a silent default that could mask a real `Arrow`.
        let ty_kinds = (0..self.ty_names.len() as u32)
            .map(|id| {
                self.ty_established_kind
                    .get(&id)
                    .cloned()
                    .unwrap_or(Kind::Star)
            })
            .collect();
        PolySig {
            row_in: self.row_in,
            inputs,
            outputs,
            row_out: self.row_out,
            // P7.S6 (R6/R7): bounds are declared only in a word's bound
            // bracket, which `attach_bracket_bounds` fills in *after* this
            // signature is built (so ids stay effect-derived). Nothing an
            // effect can contain declares one.
            bounds: Vec::new(),
            ty_kinds,
            ty_var_names: self.ty_names,
            ty_var_spans: self.ty_var_spans,
            len_var_names: self.len_names,
            len_var_spans: self.len_var_spans,
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

/// P7.S3h: the `owning` prefix with nothing that opens a quotation effect
/// after it. `owning` is a type-position keyword, never a type name of its
/// own, so `owning Foo`, `owning ~[ -- ]` and a bare trailing `owning` all
/// land here rather than being blamed on the following token.
fn owning_without_effect_error(span: Span) -> String {
    format!(
        "error: `{OWNING_QUOTATION_KEYWORD}` must be followed by a quotation effect (`{OWNING_QUOTATION_KEYWORD} [ <in> -- <out> ]`) at line {}, col {}",
        span.line, span.col
    )
}

/// P7.S3h: an `owning` effect carrying a type variable inside a polymorphic
/// signature. `PolyType::Quotation` has nowhere to record the owning flavour,
/// so folding one would silently produce a plain quotation -- and polymorphism
/// over plain-versus-owning is out of scope, since the type inequality is what
/// stops an owning closure reaching a combinator that cannot dispose it.
fn polymorphic_owning_quotation_error(span: Span) -> String {
    format!(
        "error: an `{OWNING_QUOTATION_KEYWORD}` quotation effect cannot carry a type variable at line {}, col {} (a generic word may declare a fully concrete `{OWNING_QUOTATION_KEYWORD} [ ... ]` parameter only)",
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

/// P7.S6 (R4a): a bracket in a type position holding no top-depth `--`.
/// After R4's retirement a bare `[` opens a quotation effect
/// unconditionally, so this is what an author who meant an array now gets --
/// hence the `array[T N]` half of the advice. That half is dropped when the
/// bracket was opened with `~[`, which has no array reading anywhere in the
/// grammar (every type-position reader rejects a bare `Token::TildeLBracket`
/// outright), so offering it would send the author somewhere the parser
/// refuses.
fn quotation_effect_missing_arrow_error(span: Span, opened_with_tilde: bool) -> String {
    let alternative = if opened_with_tilde {
        ""
    } else {
        " (for an array type write `array[T N]`)"
    };
    format!(
        "parse error: a quotation effect at line {}, col {} must be written in full as `[ inputs -- outputs ]`, found no top-depth `--`{alternative}",
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

/// P7.S6 (R7): a bound written inside a stack effect. Bounds live in the
/// word's own bound bracket and only there, so the effect can never be the
/// place a bound is *declared*. Selected against `impl_target_bound_error` by
/// `PolyBuilder::forbid_bounds` inside `parse_poly_ty_var`, which is the only
/// place that knows which of the two entry paths detected it.
fn bound_in_effect_error(name: &str, span: Span) -> String {
    format!(
        "error: bound on `{name}` at line {}, col {} may not be written inside a stack effect; declare it in the word's bound bracket (e.g. `: f[{name}: Copy] ( ... )`)",
        span.line, span.col
    )
}

/// Named-slot-locals sugar (R2): a single word token holding exactly one
/// non-trailing `:` (`a:i64`) in slot position. A trailing `:` is R1's glued
/// split, not this error, and a `::`-qualified type name never has exactly
/// one `:`.
fn glued_slot_name_needs_space_error(text: &str, span: Span) -> String {
    let (name, rest) = text
        .split_once(':')
        .expect("caller guarantees exactly one `:`");
    format!(
        "error: `{text}` at line {}, col {} looks like a named slot with no space after `:`; write `{name} : {rest}`",
        span.line, span.col
    )
}

/// Named-slot-locals sugar (R1): the glued trailing-colon spelling only
/// makes sense when the sliced-off name half could plausibly be a body-block
/// bind (`| ... |` only ever binds a single `Token::Word`); a name half that
/// does not re-lex, standalone, to exactly one `Word` token can never be
/// referenced by any body term, so minting it as a slot name would silently
/// swallow the slot's argument rather than naming it -- at base `403618f`
/// this was already an error (`resolve_type_or_apply` on the un-split
/// token), so this restores a reject rather than introducing one. Re-lexing
/// (rather than special-casing `is_int_literal`/`is_float_literal`) also
/// catches a standalone `\`, which the lexer reads as a line comment
/// (`src/lexer.rs:203-211`) and which would otherwise silently mint an
/// unreachable local.
fn glued_slot_name_not_a_word_error(text: &str, name: &str, span: Span) -> String {
    format!(
        "error: `{text}` reads as a slot named `{name}`, but `{name}` is not a name a body block could bind at line {}, col {}",
        span.line, span.col
    )
}

/// Named-slot-locals sugar (R11): a poly-effect slot position has no legal
/// spelling with a `:`, so any word followed by one (spaced or glued) is
/// always an attempted slot name.
fn poly_slot_name_not_supported_error(span: Span) -> String {
    format!(
        "error: slot names are not supported in polymorphic effects at line {}, col {}",
        span.line, span.col
    )
}

/// Named-slot-locals sugar (R12): two input slots in one word-definition
/// effect sharing a name. `TypedSlot` carries no per-slot span, so the error
/// cites the word definition's own span.
fn duplicate_slot_name_error(name: &str, word_name: &str, span: Span) -> String {
    format!(
        "error: slot name `{name}` is declared more than once in `{word_name}` (defined at line {}, col {})",
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
        "error: unknown capability `{name}` at line {}, col {} (a bound names `Copy` or a trait in scope)",
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

/// P7.S6a (R2): the header-bracket twin of `var_kind_conflict_error` -- a
/// `type:`/`trait:` header binding one `'`-name once with no annotation
/// (a type variable) and once with `: Len` (`type: Bad['T 'T: Len] ...`).
/// Reported ahead of `duplicate_generic_ty_var_error` (which stays for the
/// same-kind case, `type: Box['T 'T]`) because the two bindings here name two
/// different kinds of variable, not one variable bound twice.
fn header_var_kind_conflict_error(name: &str, decl_name: &str, span: Span) -> String {
    format!(
        "error: `{name}` at line {}, col {} is bound as both a type variable and a length variable in `{decl_name}`'s header; these are two different variables",
        span.line, span.col
    )
}

/// P7.S6a (R2), widened P7b.S1 (R1/S1-9): a header-bracket kind annotation
/// naming anything but `*`, `Len`, or an n-ary arrow of those (`'N: Foo`).
fn header_bracket_unknown_kind_error(found: &str, span: Span) -> String {
    format!(
        "error: unknown kind annotation `{found}` at line {}, col {} (a kind is `*`, `Len`, or an arrow of those, e.g. `'N: Len`, `'F: * -> *`)",
        span.line, span.col
    )
}

/// P7b.S1 (R2/R5, header-field twin of S1-15.a/e): a `type:`/`trait:`
/// header field bare-mentioning a variable the header declared
/// higher-kinded (`type: Box['F: * -> *] ... f 'F ...`). A bare field
/// position is always a `Star` requirement; only Phase 2's application
/// grammar can satisfy an `Arrow`-kinded variable.
fn header_field_kind_conflict_error(
    decl_name: &str,
    name: &str,
    span: Span,
    declared_kind: &str,
) -> String {
    format!(
        "error: `{name}` at line {}, col {} is used as a plain type in `{decl_name}`'s field but is declared kind `{declared_kind}` in its header",
        span.line, span.col
    )
}

/// P7b.S1 (S1-8/S1-15.e, header-field twin of S1-15.a): a header field
/// applies a variable the header declared `Star`-kinded, e.g. `f 'F['T]`
/// where `'F` has no arrow annotation.
fn header_field_applies_star_var_error(
    decl_name: &str,
    name: &str,
    span: Span,
    declared_kind: &str,
) -> String {
    format!(
        "error: `{name}[...]` at line {}, col {} applies `{name}` like a type constructor in `{decl_name}`'s field, but `{name}` is declared kind `{declared_kind}` in its header",
        span.line, span.col
    )
}

/// P7b.S1 (R2/S1-4/S1-15.c): a bracket kind annotation (`'F: * -> *`)
/// conflicting with how the signature's own effect already used the
/// variable -- both spans, the usage mention and the annotation, per the
/// house diagnostic style (`var_kind_conflict_error`).
fn var_kind_annotation_conflict_error(
    name: &str,
    usage_span: Span,
    usage_desc: &str,
    annotation_span: Span,
    annotation_kind: &str,
) -> String {
    format!(
        "error: type variable `{name}` at line {}, col {} is used as {usage_desc} but is annotated `{annotation_kind}` at line {}, col {}",
        usage_span.line, usage_span.col, annotation_span.line, annotation_span.col
    )
}

/// P7b.S1 (R1/S1-14 precursor): render a `Kind` the way a kind expression is
/// spelled in source (`*`, `Len`, `* -> Len -> *`).
fn kind_str(kind: &Kind) -> String {
    match kind {
        Kind::Star => "*".to_string(),
        Kind::Len => "Len".to_string(),
        Kind::Arrow { domains, result } => {
            let mut parts: Vec<String> = domains.iter().map(kind_str).collect();
            parts.push(kind_str(result));
            parts.join(" -> ")
        }
    }
}

/// P7b.S1 (R1/S1-2/S1-9): one atom of a kind expression -- `*` or `Len`.
/// `unknown_kind_error` lets each of the two call sites
/// (`parse_optional_bound_bracket`, `parse_header_bracket`) report in its own
/// established voice (`unknown_capability_error` vs
/// `header_bracket_unknown_kind_error`).
fn parse_kind_atom(
    parser: &mut Parser<'_>,
    unknown_kind_error: impl Fn(&str, Span) -> String,
) -> Result<Kind, String> {
    let (w, span) = parser.expect_word_any_spanned()?;
    if w == "*" {
        Ok(Kind::Star)
    } else if w == LEN_KIND_NAME {
        Ok(Kind::Len)
    } else {
        Err(unknown_kind_error(&w, span))
    }
}

/// P7b.S1 (R1/S1-2): a full kind expression -- one atom, or an n-ary arrow
/// chain (`* -> Len -> *`) folded into `Kind::Arrow { domains, result }`
/// (the last atom is the result; every earlier atom is a domain). Not
/// curried (S1-1): Sooth application splits type slots from length slots at
/// every call site, so `array` is honestly `* -> Len -> *`, a single
/// two-domain arrow, not `* -> (Len -> *)`.
fn parse_kind_expr(
    parser: &mut Parser<'_>,
    unknown_kind_error: impl Fn(&str, Span) -> String + Copy,
) -> Result<Kind, String> {
    let mut atoms = vec![parse_kind_atom(parser, unknown_kind_error)?];
    while matches!(parser.peek(), Some((Token::Word(w), _)) if w == "->") {
        parser.pos += 1;
        atoms.push(parse_kind_atom(parser, unknown_kind_error)?);
    }
    if atoms.len() == 1 {
        Ok(atoms.pop().expect("just pushed"))
    } else {
        let result = atoms.pop().expect("len() > 1");
        Ok(Kind::Arrow {
            domains: atoms,
            result: Box::new(result),
        })
    }
}

/// P7.S6a (R2.1): a non-empty header bracket binding zero type variables
/// (`type: Buf['N: Len] ...`). Ruled out rather than supported: four sites in
/// `src/check/poly.rs` treat an empty type-args list as the signal for "not
/// generic here", a convention only correct today because this shape was
/// previously unconstructible.
fn header_bracket_no_type_variable_error(decl_name: &str, span: Span) -> String {
    format!(
        "error: `{decl_name}`'s header binds a length variable but no type variable at line {}, col {} (a generic header needs at least one type variable)",
        span.line, span.col
    )
}

/// Phase 5 slice 1 (R1, round-3 review): a generic `type:` header binding
/// the same variable name twice (`type: Bad['T 'T] ...`). Caught here, at the
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

/// P7.S6 (R5): an empty type-variable bracket in a `type:`/`trait:` header
/// (`type: Box[]`, `trait: Ord[]`). The bracket is present but binds nothing.
fn empty_header_bracket_error(decl_name: &str, span: Span) -> String {
    format!(
        "error: empty type-variable bracket in `{decl_name}` at line {}, col {} (expected one or more `'`-prefixed variables, e.g. `type: {decl_name}['T]`)",
        span.line, span.col
    )
}

/// P7.S6 (R5): a token inside a header bracket that is neither a `'`-prefixed
/// variable nor `]`. The bracket's contents are `'`-prefixed words only (no
/// bounds on a `type:`/`trait:` header), so anything else is a located error.
fn header_bracket_non_var_error(decl_name: &str, tok: &Token, span: Span) -> String {
    format!(
        "error: expected a type variable (`'T`) or `]` inside `{decl_name}`'s bracket at line {}, col {}, found {tok:?}",
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

/// P7.S6a (R2a): the length-path twin of `unbound_generic_ty_var_error` -- a
/// generic `type:` field naming a `'`-prefixed length variable its header
/// never bound (as a length variable; it may still be an unrelated type
/// variable, which reports the same way `unbound_generic_ty_var_error` does).
fn unbound_generic_len_var_error(name: &str, decl_name: &str, span: Span) -> String {
    format!(
        "error: `{name}` at line {}, col {} is not a length variable bound by `type: {decl_name}`'s header",
        span.line, span.col
    )
}

/// P7.S6a (R2a): the length-path twin of `phantom_ty_var_error` -- a generic
/// `type:` header binds a length variable that never appears in any field's
/// array count (or a nested application's length-argument position).
fn phantom_len_var_error(decl_name: &str, name: &str, span: Span) -> String {
    format!(
        "error: length variable `{name}` at line {}, col {} is bound by `type: {decl_name}`'s header but appears in no field (a phantom parameter cannot be disambiguated at a call site)",
        span.line, span.col
    )
}

/// Phase 5 slice 1: the generic path's twin of the concrete odd-field-count
/// error. It names the header's bound variables because the likeliest way to
/// reach it is writing a `'`-prefixed *field* name inside the header bracket
/// (`type: Foo['bar] i64 ;`), which binds it as a type parameter -- leaving the
/// plain message pointing at a token the author never got wrong.
fn generic_odd_field_count_error(
    decl_name: &str,
    ty_vars: &[(String, Span)],
    field_name: &str,
    before: &str,
    span: Span,
) -> String {
    let header: Vec<&str> = ty_vars.iter().map(|(n, _)| n.as_str()).collect();
    format!(
        "parse error: field `{field_name}` has no type before `{before}` at line {}, col {} (odd field-token count in the body of generic `type: {decl_name}[{}]`; a `'`-prefixed word inside the header bracket binds a type parameter)",
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
///
/// P7.S6a (R7 fix): `declared`/`supplied` are each split into a type-variable
/// count and a length-variable count, since a length-carrying header
/// (`Buffer['T 'N: Len]`) declares one of each, not two type variables --
/// `Buffer[T T]` was never valid syntax for it.
fn generic_arity_error(
    name: &str,
    ty_declared: usize,
    len_declared: usize,
    ty_supplied: usize,
    len_supplied: usize,
    span: Span,
) -> String {
    let declared_str = if len_declared == 0 {
        if ty_declared == 1 {
            "1 type variable".to_string()
        } else {
            format!("{ty_declared} type variables")
        }
    } else {
        let ty_part = if ty_declared == 1 {
            "1 type variable".to_string()
        } else {
            format!("{ty_declared} type variables")
        };
        let len_part = if len_declared == 1 {
            "1 length variable".to_string()
        } else {
            format!("{len_declared} length variables")
        };
        format!("{ty_part} and {len_part}")
    };
    let supplied = ty_supplied + len_supplied;
    let supplied_str = match supplied {
        0 => "none were".to_string(),
        1 => "1 was".to_string(),
        n => format!("{n} were"),
    };
    let example = if len_declared == 0 {
        vec!["T"; ty_declared].join(" ")
    } else {
        let mut parts: Vec<&str> = vec!["T"; ty_declared];
        parts.extend(std::iter::repeat_n("N", len_declared));
        parts.join(" ")
    };
    format!(
        "error: generic type `{name}` declares {declared_str}, but {supplied_str} supplied at line {}, col {} (apply it as `{name}[{}]`, one type argument per declared variable)",
        span.line,
        span.col,
        example,
    )
}

/// P7b.S1 (S1-7): an application supplying zero type arguments (`'F[]`),
/// pinned as an arity error -- an application always names at least one
/// argument, unlike a generic header applied to zero variables (which has
/// no bracket at all).
fn empty_type_application_error(var: &str, span: Span) -> String {
    format!(
        "error: `{var}[]` at line {}, col {} applies `{var}` to zero arguments (an application needs at least one type argument)",
        span.line, span.col
    )
}

/// P7b.S1 (S1-6): a quotation-shaped argument inside a type application's
/// argument list (`'F[[ i64 -- i64 ]]`) -- S1's application arguments are
/// type expressions only.
fn app_arg_quotation_error(span: Span) -> String {
    format!(
        "error: expected a type, found `[` at line {}, col {} (a type application's arguments are types, not quotations)",
        span.line, span.col
    )
}

/// S1-15.a: a type variable bound bare (kind `*`) at an earlier mention,
/// then applied like a constructor (`'F['T]`) at a later one -- located,
/// naming both the misuse site and the binding site.
fn star_applied_like_constructor_error(name: &str, misuse: Span, binding: Span) -> String {
    format!(
        "error: type variable `{name}` at line {}, col {} is applied like a type constructor but has kind `*` (bound bare at line {}, col {}); only a higher-kinded variable can head `{name}[...]`",
        misuse.line, misuse.col, binding.line, binding.col
    )
}

/// S1-15.b: a type variable applied at an earlier mention (kind `* -> ...`),
/// then used bare at a later one -- located, naming both the misuse site and
/// the binding (first application) site.
///
/// S2-15.b (S2-1) reuses this message verbatim for a header-annotated seed:
/// with the header kind published, a member's bare `'F` mention reports the
/// header annotation's span as the "binding" origin. The `(from an
/// application of ...)` clause is then imprecise -- that span is the header's
/// kind annotation, not an application -- but the wording is golden-frozen
/// by P7b.S1 (`phase7b_slice1.rs` kind-error-b) and stays unchanged; a
/// header-aware rewording would be a cross-slice golden break.
fn arrow_var_used_bare_error(name: &str, misuse: Span, binding: Span, kind: &Kind) -> String {
    format!(
        "error: type variable `{name}` at line {}, col {} is used as a plain type but has kind `{}` (from an application of `{name}` at line {}, col {}); a higher-kinded variable never appears bare",
        misuse.line, misuse.col, kind_str(kind), binding.line, binding.col
    )
}

/// S1-15.f: a use-site constructor-header argument (`Wrap[Nat i64]`) whose
/// `CtorImage`-ness disagrees with the header variable's own declared kind --
/// a plain type supplied where the header expects a constructor (`'F: * ->
/// *`), or a constructor supplied where it expects a plain type.
fn validate_ctor_arg_kinds(
    header: &str,
    ty_var_names: &[String],
    ty_kinds: &[Kind],
    args: &[Type],
    span: Span,
) -> Result<(), String> {
    for ((var, kind), arg) in ty_var_names.iter().zip(ty_kinds.iter()).zip(args.iter()) {
        let is_ctor = matches!(arg, Type::CtorImage(_, _));
        let wants_ctor = matches!(kind, Kind::Arrow { .. });
        if is_ctor != wants_ctor {
            return Err(ctor_arg_kind_mismatch_error(header, var, kind, *arg, span));
        }
    }
    Ok(())
}

fn ctor_arg_kind_mismatch_error(
    header: &str,
    var: &str,
    expected: &Kind,
    got: Type,
    span: Span,
) -> String {
    let remedy = if matches!(expected, Kind::Arrow { .. }) {
        "a type constructor is required here, not a concrete type"
    } else {
        "a concrete type is required here, not a type constructor"
    };
    format!(
        "error: `{header}[...]` at line {}, col {} supplies `{got}` for `{var}`, but `{var}` has kind `{}` ({remedy})",
        span.line,
        span.col,
        kind_str(expected)
    )
}

/// S1-15.d: an application's arity conflicts with the arity an earlier
/// application of the same variable already established -- located, naming
/// both the conflicting site and the binding (first application) site.
fn application_arity_conflict_error(
    name: &str,
    misuse: Span,
    got_arity: usize,
    binding: Span,
    expected_arity: usize,
) -> String {
    format!(
        "error: `{name}[...]` at line {}, col {} applies `{name}` to {got_arity} argument{} but its kind takes {expected_arity} (from `{name}[...]` at line {}, col {})",
        misuse.line,
        misuse.col,
        if got_arity == 1 { "" } else { "s" },
        binding.line,
        binding.col
    )
}

/// P7.S6 (R2): `array` in a type position with no following `[` is a located
/// error naming the required form, not "unknown type `array`". Raised at the
/// single funnel `resolve_type_or_apply`, which every bare-word type reader
/// passes through.
fn array_without_bracket_error(span: Span) -> String {
    format!(
        "error: `{ARRAY_TYPE_NAME}` must be followed by `[T N]` to form an array type at line {}, col {}",
        span.line, span.col
    )
}

/// P7.S3t (R2): a glued `[` opened an explicit type instantiation and the end
/// of input arrived before its `]`. Named after the call rather than reported
/// as a generic "expected `]`", since the construct only exists because of the
/// glue and the remedy may well be a space.
fn unterminated_instantiation_error(name: &str, span: Span) -> String {
    format!(
        "error: unterminated explicit type instantiation of `{name}` at line {}, col {} (expected `]`)",
        span.line, span.col
    )
}

/// P7.S3t (R2): `f[]`. An empty list is not "no instantiation": downstream the
/// two are the same empty vector, so the arity check R4 performs against the
/// callee's declared variables would never fire for it.
fn empty_instantiation_error(name: &str, span: Span) -> String {
    format!(
        "error: `{name}[]` at line {}, col {} instantiates nothing; name one concrete type per declared variable, or insert a space for a quotation literal",
        span.line, span.col
    )
}

/// P7.S6 (R6): an empty bound bracket on a word definition (`: f[] ( ... )`).
/// The bracket is present but declares no variables.
fn empty_bound_bracket_error(span: Span) -> String {
    format!(
        "error: empty bound bracket at line {}, col {} (expected one or more `'`-prefixed variable declarations, e.g. `['T: Copy]`)",
        span.line, span.col
    )
}

/// P7.S6 (R6): a token inside a bound bracket that is neither a `'`-prefixed
/// variable nor `]`. Inside the bracket only variable declarations and the
/// closing `]` are legal.
fn bound_bracket_non_var_error(tok: &Token, span: Span) -> String {
    format!(
        "error: expected a type variable (`'T`) or `]` inside the bound bracket at line {}, col {}, found {tok:?}",
        span.line, span.col
    )
}

/// P7.S6 (R6): a bracket-declared variable that never appears in the word's
/// effect. The bracket *adds* a bound declaration; the effect keeps every
/// mention of the variable. A variable that appears in the bracket but not
/// the effect would leave a bound on a variable with no slot.
fn bracket_var_unused_error(name: &str, span: Span, word_name: &str) -> String {
    format!(
        "error: type variable `{name}` declared in the bound bracket of `{word_name}` at line {}, col {} never appears in the effect",
        span.line, span.col
    )
}

/// P7.S6a (R2b): the length-path twin of `bracket_var_unused_error` -- a
/// bracket-declared `'N: Len` whose name never appears in the word's effect
/// in a length position.
fn bracket_len_var_unused_error(name: &str, span: Span, word_name: &str) -> String {
    format!(
        "error: length variable `{name}` declared in the bound bracket of `{word_name}` at line {}, col {} never appears in the effect",
        span.line, span.col
    )
}

/// P7.S3t (R2): the note a malformed *first* element carries. A glued bracket
/// re-points a spelling that used to parse as a call followed by a quotation or
/// array literal, so the element error has to say that the glue is what put the
/// parse in type position.
fn instantiation_element_note() -> &'static str {
    "\n  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal"
}

/// P7.S3t (R7): a type *variable* as an explicit type argument. It has no
/// production in `parse_type_expr`, so without this it reports `unknown type
/// 'U` -- a message that reads as a missing declaration rather than as the
/// unsupported forwarding it is.
fn instantiation_ty_var_error(var: &str, span: Span) -> String {
    format!(
        "error: `{var}` (line {}, col {}) is a type variable; an explicit instantiation takes concrete types\n  note: forwarding a caller's type variable through an explicit instantiation is not supported",
        span.line, span.col
    )
}

/// P7.S6b (R1): once an explicit instantiation list has seen a length
/// argument (a bare integer token), the call-site grammar fixes "types
/// first, then lengths" as its own convention -- a type token appearing
/// after an integer token has nowhere left to go.
fn instantiation_type_after_len_error(word: &str, span: Span, tok: &Token, tspan: Span) -> String {
    format!(
        "error: expected a length argument or `]` in the explicit instantiation of `{word}` at line {}, col {}, found {} at line {}, col {} (type arguments must come before length arguments)",
        span.line, span.col, describe_token(tok), tspan.line, tspan.col
    )
}

/// P7.S6b (R1b): a length argument out of `1..=u32::MAX` at a word call
/// site. Mirrors `parse_array_count`'s range check but named after the call
/// (`sum[i64 0]`), not after an array type (`array[sum 0]`), since the two
/// constructs read differently even though the numeric rule is identical.
fn instantiation_len_range_error(word: &str, span: Span, n: i64, arg_span: Span) -> String {
    format!(
        "error: `{word}[...]` at line {}, col {} instantiates a length argument {n} at line {}, col {} out of range (requires 1 <= N <= {})",
        span.line, span.col, arg_span.line, arg_span.col, u32::MAX
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
fn generic_field_type_str(
    pty: &PolyType,
    ty_vars: &[(String, Span)],
    len_vars: &[(String, Span)],
) -> String {
    match pty {
        PolyType::Concrete(t) => t.name().to_string(),
        PolyType::Var(v) => ty_vars[*v as usize].0.clone(),
        PolyType::Array(elem, len) => {
            let n = match len {
                Len::Concrete(n) => n.to_string(),
                // P7.S6a (R2a): a header-bound length variable, rendered by
                // its own surface spelling exactly as `Var` renders a type
                // variable's -- reachable unconditionally from
                // `parse_generic_field_array`'s own error-string build, not
                // only on a parse error, so this can never be `unreachable!()`.
                Len::Var(v) => len_vars[*v as usize].0.clone(),
            };
            format!(
                "array[{} {}]",
                generic_field_type_str(elem, ty_vars, len_vars),
                n
            )
        }
        PolyType::Ref(referent, mutable) => format!(
            "&{}{}",
            if *mutable { "!" } else { "" },
            generic_field_type_str(referent, ty_vars, len_vars)
        ),
        PolyType::OwnedCell(payload) => {
            format!("^{}", generic_field_type_str(payload, ty_vars, len_vars))
        }
        PolyType::Generic {
            name,
            args,
            len_args,
            ..
        } => {
            let mut parts: Vec<String> = args
                .iter()
                .map(|a| generic_field_type_str(a, ty_vars, len_vars))
                .collect();
            parts.extend(len_args.iter().map(|l| match l {
                Len::Concrete(n) => n.to_string(),
                Len::Var(v) => len_vars[*v as usize].0.clone(),
            }));
            format!("{name}[{}]", parts.join(" "))
        }
        // R7 rejects a variable-bearing quotation field at the parser, and a
        // concrete one folds to `Concrete`, so neither reaches here.
        PolyType::Quotation(..) | PolyType::QuotLit => {
            unreachable!("a generic `type:` field is never a quotation shape")
        }
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never a generic `type:` field's shape.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it is never a generic `type:` field's shape"
        ),
        // P7b.S1 (S1-8): a header/field application (`f 'F['T]`), rendered
        // by the applied variable's own surface spelling plus its arguments.
        PolyType::App { head, args } => {
            let parts: Vec<String> = args
                .iter()
                .map(|a| generic_field_type_str(a, ty_vars, len_vars))
                .collect();
            format!("{}[{}]", ty_vars[*head as usize].0, parts.join(" "))
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
/// and mentions one of the declaration's own type variables (`L[array['T 2]]`,
/// `L[^'T]`, `L[&'T]`). Each such hop would instantiate the header at a
/// strictly larger argument than the last, forever.
///
/// The walk descends through every wrapper, not just a field whose own top
/// level is an application: `^L[^'T]` is a cell over the application, and
/// `[Ent['K 'V] 8]` is an array over one. An argument that is fully concrete
/// at any depth (`L[array[i64 2]]`) is inert -- it carries no variable to grow --
/// and a bare `'T` argument passes through unchanged, so both are admitted.
///
/// Accepted over-rejection, stated so a future slice can lift it: a
/// *non-recursive* wrapping application (`Outer 'T f Ent[array['T 2] i64]`, where
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
        // P7.S12 (R3.5): unconstructible outside an eliminator arm's own
        // input row, never a generic `type:` field's shape.
        PolyType::GenericVariant { .. } => unreachable!(
            "a generic variant is unconstructible outside an eliminator arm's own input row; it is never a generic `type:` field's shape"
        ),
        // P7b.S1 (S1-8): an application field is subject to the same
        // growth restriction as a `Generic` field -- an argument that is
        // itself compound and variable-bearing would grow at every
        // instantiation exactly as a `Generic`'s would.
        PolyType::App { head, args } => {
            for arg in args {
                if !matches!(arg, PolyType::Concrete(_) | PolyType::Var(_)) {
                    return Err(growing_generic_self_reference_error(
                        decl_name,
                        &format!("'F{head}"),
                        arg,
                        span,
                    ));
                }
            }
            Ok(())
        }
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

/// P7.S6a (R2a): the length-path twin of `check_no_phantom_ty_var`, called
/// alongside it once a header's whole field/variant list is known.
fn check_no_phantom_len_var(
    decl_name: &str,
    len_vars: &[(String, Span)],
    used_len: &[bool],
) -> Result<(), String> {
    if let Some(idx) = used_len.iter().position(|&u| !u) {
        let (name, span) = &len_vars[idx];
        return Err(phantom_len_var_error(decl_name, name, *span));
    }
    Ok(())
}

struct Parser<'t> {
    tokens: &'t [(Token, Span)],
    pos: usize,
    /// The struct registry (names always populated by the pre-pass, fields
    /// populated for the `type:` bodies already parsed at the point of
    /// lookup, but resolution only needs the id/name so declaration order
    /// among structs doesn't matter). Empty for the import/export scans
    /// (`scan_imports`/`scan_exports`), which resolve no type name, and in a
    /// unit test that passes no struct registry at all.
    structs: &'t [StructDecl],
    /// The enum registry, parallel to `structs` (names, and each enum's
    /// variant names, always populated by the pre-pass). Empty for the same
    /// reason `structs` is: the import/export scans resolve no type name,
    /// and a unit test may pass no enum registry at all.
    enums: &'t [EnumDecl],
    /// The interned array-type registry (D3, M1): unlike `structs`/`enums`,
    /// an array shape has no declared name a pre-pass could register ahead
    /// of time, so this grows during type-expression resolution rather than
    /// being pre-populated. A mutable borrow of the caller's registry (the
    /// whole-module `Module.arrays` for a native build), so interning
    /// persists across every parse of the same closure (R22/R23).
    arrays: &'t mut Vec<ArrayDecl>,
    /// The interned owning-cell registry, mirroring `arrays` for the same
    /// reason: a `^T` shape has no declared name a pre-pass could register
    /// ahead of time, so it grows during type-expression resolution and
    /// persists exactly like `arrays`.
    owned_cells: &'t mut Vec<OwnedCellDecl>,
    /// The interned reference registry, mirroring `owned_cells`: a `&T`/`&!T`
    /// shape has no declared name either, so it grows as type expressions
    /// resolve and persists across the whole closure's parse.
    refs: &'t mut Vec<RefDecl>,
    /// P7 slice 3c (R1.2): the interned slice registry, mirroring `refs` --
    /// a `Slice[T]` shape has no declared name, so it grows as type
    /// expressions resolve. The checker's `slice`/`subslice` words intern
    /// into the same registry, so a view built at check time and one spelled
    /// in a signature share a `SliceId`.
    slices: &'t mut Vec<SliceDecl>,
    /// Phase 4 slice 5a (R11): the module id whose body this parser is
    /// currently reading. `0` for a single-file program; the driver's closure
    /// assembly sets it per file. An unqualified type name resolves against
    /// this module first.
    module: u32,
    /// Phase 4 slice 5a (R8): this module's qualifier->module import map, used
    /// to resolve a `q::Type` type name. Empty for a single-file program.
    imports: &'t std::collections::HashMap<String, u32>,
    /// Phase 4 slice 5a phase 2 (R16): every module's `export:` list, indexed
    /// by module id, scanned ahead of any body parse (`scan_exports`) so a
    /// cross-module type name in an effect can be visibility-checked even
    /// though the exporting file's own body may not have parsed yet. Empty for
    /// a single-file program, where no qualified name can occur.
    exports: &'t [Vec<(String, Span)>],
    /// Phase 4 slice 5a phase 4 (R20/R15c): this module's selectively-imported
    /// unqualified names, each mapping to the target module it resolves in. A
    /// bare `Type` (or word) exposed by `import: "..." q | Type | ` resolves
    /// here after the own-module lookup fails (own-module-first, R11). Empty
    /// for a single-file program.
    selective: &'t std::collections::HashMap<String, u32>,
    /// P7.S3q-follow: for a module reached through `imports`/`selective`,
    /// the true declaring module of a name on *its* `export:` list, when that
    /// name is a re-export rather than something it declares itself --
    /// closing the gap where a type name reached only through a hub resolved
    /// fine in term position (the late, whole-program `resolve.rs` pass
    /// already walks a hub chain there) but not in an effect signature,
    /// which resolves during this early parse via a single hop. Indexed by
    /// module id, empty for any parse path with no real cross-module data.
    type_origin: &'t [std::collections::HashMap<String, u32>],
    /// P7.S3s (R1): the trait twin of `type_origin` -- for a module reached
    /// through `imports`/`selective`, the true declaring module of a trait
    /// name on *its* `export:` list when that name is a re-export rather
    /// than something it declares itself. Indexed by module id, empty for
    /// any parse path with no real cross-module data.
    trait_origin: &'t [std::collections::HashMap<String, u32>],
    /// Phase 5 slice 1 (R2/D5): the generic `type:` declarations in scope and
    /// the concrete struct/enum registry each application of one mints. A
    /// mutable borrow for the same reason `arrays` is one: an instantiation
    /// is minted *while* a field or slot type expression resolves. Empty (and
    /// never written) for the import/export scans, which have no generic
    /// declaration to apply.
    generics: &'t mut GenericTypes,
    /// P7b.S1 (S1-5/S1-9, Phase 3): each header type variable's kind as
    /// established by its *first* field-usage mention within the
    /// declaration currently being parsed -- the header-field twin of
    /// `PolyBuilder::ty_established_kind`. Reset at the start of each
    /// declaration's field list (`parse_generic_typedef_fields`), since a
    /// ty var id is decl-local and would otherwise collide across decls.
    field_kind_marks: std::collections::HashMap<u32, (Kind, Span)>,
    /// P7.S3e (R3): the whole-program trait registry (pre-seeded `Copy`
    /// plus every user `trait:` declaration in the closure), populated by
    /// `prepass_trait_decls` before any body parses -- mirrors `structs`/
    /// `enums`. Empty only in a unit test that passes no trait registry at
    /// all.
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
        // P7.S6 (R6): the optional bound bracket sits after `inline` and
        // before `(`, e.g. `: max['T: Copy Ord] ( 'T 'T -- 'T )`. Parsed into
        // a side table and attached to effect-derived ids after
        // `parse_poly_effect` (never pre-interned), so ids stay effect-derived.
        let bound_bracket = self.parse_optional_bound_bracket()?;
        // A word carrying a non-empty bracket takes the `PolySig` path
        // regardless of `effect_has_variable`.
        let force_poly = bound_bracket.as_ref().is_some_and(|b| !b.is_empty());
        self.expect(Token::LParen)?;
        // R1/R2: a variable-bearing effect (`'T`, `'N`, `..s`) parses into a
        // `PolySig`; every other effect stays a concrete `StackEffect`, byte
        // for byte as before (the whole regression guarantee, R15).
        let (effect, poly) = if force_poly || self.effect_has_variable() {
            let mut sig = self.parse_poly_effect()?;
            // R6: attach bracket bounds to the ids the effect interned.
            if let Some(bracket) = &bound_bracket {
                self.attach_bracket_bounds(&mut sig, bracket, &name)?;
            }
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
        // Named-slot-locals sugar (R3/R4/R12): runs after the body is fully
        // parsed and only here -- `effect.inputs` is empty for a poly word
        // (its names live nowhere, `PolyType` carries none), so this is a
        // no-op for one.
        let body = Self::desugar_slot_locals(&effect, body, &name, self.enums, name_span)?;
        Ok(WordDef {
            name,
            effect,
            body,
            poly,
            declares_inline,
            module: self.module,
            span: name_span,
            declared_globals,
            is_trait_member: false,
        })
    }

    /// Named-slot-locals sugar: collect every name any `TermKind::Bind` in
    /// `terms` binds, nested quotations included (R6) -- the freshness scan's
    /// view of "bound anywhere in the body", mirroring the walk
    /// `crate::ast::alpha_rename_locals` already does over `Bind`/`Quotation`.
    fn collect_bound_names(terms: &[Term], names: &mut std::collections::HashSet<String>) {
        for term in terms {
            match &term.kind {
                TermKind::Bind(ns) => names.extend(ns.iter().cloned()),
                TermKind::Quotation(inner, _, _) => Self::collect_bound_names(inner, names),
                _ => {}
            }
        }
    }

    /// Named-slot-locals sugar (R3/R4/R5/R6/R7/R12): extract `effect`'s
    /// input-slot names, reject a duplicate (R12: input slots only), and --
    /// when at least one is named -- prepend a `Bind` term binding every
    /// slot from the deepest named one to the top, minting a fresh positional
    /// name (R6) for each unnamed slot in that run and immediately
    /// re-pushing it (R5) so unnamed slots keep their original relative
    /// stack order. A slot named `array` (R13) is unaffected: the desugar
    /// only ever reads `TypedSlot.name`, never clears it. `span` is the word
    /// definition's own span (`TypedSlot` carries none of its own, Open
    /// Questions).
    fn desugar_slot_locals(
        effect: &StackEffect,
        body: Vec<Term>,
        word_name: &str,
        enums: &[EnumDecl],
        span: Span,
    ) -> Result<Vec<Term>, String> {
        let mut seen = std::collections::HashSet::new();
        for slot in &effect.inputs {
            if let Some(name) = &slot.name {
                if !seen.insert(name.clone()) {
                    return Err(duplicate_slot_name_error(name, word_name, span));
                }
            }
        }
        let Some(deepest_named) = effect.inputs.iter().position(|s| s.name.is_some()) else {
            // R18: no named input slot anywhere -- byte-identical to today.
            return Ok(body);
        };
        let run = &effect.inputs[deepest_named..];
        // R6: the freshness set a mint candidate must clear -- every name
        // bound anywhere in the body, the word's own name, every other
        // input-slot name, and every enum variant name in the module.
        let mut fresh = std::collections::HashSet::new();
        fresh.insert(word_name.to_string());
        for slot in &effect.inputs {
            if let Some(n) = &slot.name {
                fresh.insert(n.clone());
            }
        }
        for e in enums {
            for v in &e.variants {
                fresh.insert(v.name.clone());
            }
        }
        Self::collect_bound_names(&body, &mut fresh);

        let mut names = Vec::with_capacity(run.len());
        let mut mints: Vec<String> = Vec::new();
        for (offset, slot) in run.iter().enumerate() {
            match &slot.name {
                Some(n) => names.push(n.clone()),
                None => {
                    let idx = deepest_named + offset;
                    let mut k = idx;
                    let mint = loop {
                        let candidate = format!("__slot{k}");
                        if !fresh.contains(&candidate) {
                            break candidate;
                        }
                        k += 1;
                    };
                    fresh.insert(mint.clone());
                    names.push(mint.clone());
                    mints.push(mint);
                }
            }
        }
        let mut prefix = vec![Term {
            kind: TermKind::Bind(names),
            span,
        }];
        for mint in mints {
            prefix.push(Term {
                kind: TermKind::Call(mint, Vec::new(), Vec::new()),
                span,
            });
        }
        prefix.extend(body);
        Ok(prefix)
    }

    /// P7.S6 (R6): parse the optional bound bracket `[ 'T: Copy 'U: Ord ]` that
    /// sits after `inline` and before `(` in a word definition. Returns
    /// `None` if no bracket is present, or a side table of
    /// `(name, span, Vec<Bound>, is_len_kind)` entries. The bracket is parsed
    /// into a local side table and attached to effect-derived ids *after*
    /// `parse_poly_effect` (never pre-interned), so ids stay effect-derived
    /// and `PolySig.ty_var_names` order is unchanged.
    ///
    /// The bracket's grammar (R6a): `[' var_decl+ ]`, where each `var_decl`
    /// is `'T` or `'T: bound_list`. A `bound_list` ends at the next `'`-prefixed
    /// word or `]`; an unrecognised name inside a bound list is an
    /// unknown-capability error (bracket mode), not a silent break.
    ///
    /// P7.S6a (R2b): a bound position whose colon is immediately followed by
    /// the bare word `Len` and nothing else is a *kind* annotation, not a
    /// capability bound -- the trailing `is_len_kind` flag marks it, and no
    /// `parse_capabilities` call runs (so `Len` never becomes a fake
    /// `Bound`). `Len` can never legitimately name a trait bound: R2.2
    /// reserves it as a trait name, so intercepting it unconditionally here
    /// costs no real capability its spelling.
    #[allow(clippy::type_complexity)]
    fn parse_optional_bound_bracket(
        &mut self,
    ) -> Result<Option<Vec<(String, Span, Vec<Bound>, Option<Kind>)>>, String> {
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            return Ok(None);
        }
        let bracket_span = self.peek().map(|(_, s)| *s).unwrap_or_default();
        self.pos += 1; // consume `[`
        let mut entries: Vec<(String, Span, Vec<Bound>, Option<Kind>)> = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                Some((Token::Word(w), span)) if w.starts_with('\'') => {
                    let glued_colon = w.ends_with(':') && w.len() > 1;
                    let (name, span) = if glued_colon {
                        (w[..w.len() - 1].to_string(), *span)
                    } else {
                        let (nw, ns) = self.expect_word_any_spanned()?;
                        (nw, ns)
                    };
                    if glued_colon {
                        self.pos += 1;
                    }
                    // Check for a standalone `:` (not glued).
                    let bound_follows = if glued_colon {
                        true
                    } else {
                        matches!(self.peek(), Some((Token::Word(c), _)) if c == ":")
                    };
                    if bound_follows && !glued_colon {
                        self.pos += 1;
                    }
                    // P7b.S1 (R1/S1-9): a bound position starting with `*` or
                    // `Len` is a *kind* annotation, not a capability bound --
                    // routed here, before `parse_capabilities` ever sees it,
                    // so `*`/`->` never need reserving as capability names
                    // (they simply never reach that parser).
                    let starts_kind = bound_follows
                        && matches!(self.peek(), Some((Token::Word(w), _)) if w == "*" || w == LEN_KIND_NAME);
                    let (bounds, kind) = if starts_kind {
                        (
                            Vec::new(),
                            Some(parse_kind_expr(self, unknown_capability_error)?),
                        )
                    } else if bound_follows {
                        (self.parse_capabilities(span, true)?, None)
                    } else {
                        (Vec::new(), None)
                    };
                    if entries.iter().any(|(n, _, _, _)| n == &name) {
                        return Err(duplicate_generic_ty_var_error(
                            &name,
                            "<bound bracket>",
                            span,
                        ));
                    }
                    entries.push((name, span, bounds, kind));
                }
                Some((tok, span)) => {
                    return Err(bound_bracket_non_var_error(tok, *span));
                }
                None => return Err(self.eof_error("`]` (unterminated bound bracket)")),
            }
        }
        if entries.is_empty() {
            return Err(empty_bound_bracket_error(bracket_span));
        }
        Ok(Some(entries))
    }

    /// P7.S6 (R6): attach bracket-declared bounds to the ids the effect
    /// interned. Each bracket entry's variable name is looked up in
    /// `sig.ty_var_names`; a name that never appears in the effect is a
    /// located error (it would leave a bound on a variable with no slot).
    ///
    /// P7b.S1 (R2/S1-4, widening P7.S6a's R2b): a `Some(kind)` entry is a
    /// pure validation/annotation, never a bound. The k8 flip (S1-3): a
    /// bare, unannotated entry (`None`) with no bounds no longer assumes
    /// `ty_var_names` -- it resolves against whichever of `ty_var_names`/
    /// `len_var_names` the effect actually used the name as, so an
    /// unannotated length var (`array['T 'N]` with no `'N: Len`) is accepted
    /// with its kind inferred from the count position, annotations becoming
    /// optional-but-available. A `Some(kind)` entry conflicting with the
    /// effect's own usage (S1-15.c) is a located error carrying both spans:
    /// the usage mention (`sig.ty_var_spans`/`len_var_spans`) and the
    /// annotation (`span`).
    fn attach_bracket_bounds(
        &self,
        sig: &mut PolySig,
        bracket: &[(String, Span, Vec<Bound>, Option<Kind>)],
        word_name: &str,
    ) -> Result<(), String> {
        // S1-5: every published kind vector is length-matched to its name
        // table -- `sig.ty_kinds[i]` below indexes on a position recovered
        // from `sig.ty_var_names`, so a drift here would silently index
        // past (or short of) the real per-variable kind.
        debug_assert_eq!(
            sig.ty_var_names.len(),
            sig.ty_kinds.len(),
            "PolySig::ty_kinds must stay parallel to ty_var_names (S1-5)"
        );
        for (name, span, bounds, kind) in bracket {
            let ty_idx = sig.ty_var_names.iter().position(|n| n == name);
            let len_idx = sig.len_var_names.iter().position(|n| n == name);
            match kind {
                Some(k) => match (k, ty_idx, len_idx) {
                    (Kind::Len, _, Some(_)) => {}
                    (Kind::Len, Some(i), None) => {
                        return Err(var_kind_annotation_conflict_error(
                            name,
                            sig.ty_var_spans[i],
                            "a plain type",
                            *span,
                            "Len",
                        ));
                    }
                    (Kind::Len, None, None) => {
                        return Err(bracket_len_var_unused_error(name, *span, word_name));
                    }
                    // P7b.S1 (S1-4 Phase 3): compare against the *resolved*
                    // usage kind, not merely against whether the variable
                    // was mentioned at all -- a var mentioned only as an
                    // application head (`'F['T]`) is still `Some(i)` in
                    // `ty_idx` (identity tracking, not kind tracking), so an
                    // `Arrow` annotation confirming an `Arrow` usage must
                    // not be flagged merely because the variable "is a plain
                    // type" by presence alone.
                    (k, Some(i), None) if *k == sig.ty_kinds[i] => {}
                    (k, Some(i), None) => {
                        let usage_desc = match &sig.ty_kinds[i] {
                            Kind::Star => "a plain type".to_string(),
                            Kind::Len => "a length".to_string(),
                            arrow @ Kind::Arrow { .. } => {
                                format!("an application of kind `{}`", kind_str(arrow))
                            }
                        };
                        return Err(var_kind_annotation_conflict_error(
                            name,
                            sig.ty_var_spans[i],
                            &usage_desc,
                            *span,
                            &kind_str(k),
                        ));
                    }
                    (k, None, Some(i)) => {
                        return Err(var_kind_annotation_conflict_error(
                            name,
                            sig.len_var_spans[i],
                            "a length",
                            *span,
                            &kind_str(k),
                        ));
                    }
                    // P7b.S1 (R2/S1-4): an arrow-kind annotation on a header
                    // var never mentioned anywhere in the effect has no
                    // usage to compare against -- unlike the `Some(i)` arms
                    // above, which do compare against `sig.ty_kinds[i]` --
                    // so it stays a bare declaration, permanently, not a
                    // temporary Phase 1 shortcut.
                    (Kind::Arrow { .. }, None, None) => {}
                    (_, None, None) => {
                        return Err(bracket_var_unused_error(name, *span, word_name));
                    }
                    // Unreachable: `intern_ty_var`/`intern_len_var` reject a
                    // name mentioned in both spaces before a signature can
                    // ever reach here (`var_kind_conflict_error`, X1).
                    (_, Some(_), Some(_)) => {
                        return Err(bracket_var_unused_error(name, *span, word_name));
                    }
                },
                None => {
                    if let Some(i) = ty_idx {
                        for b in bounds {
                            sig.bounds.push((i as u32, *b));
                        }
                    } else if !bounds.is_empty() || len_idx.is_none() {
                        return Err(bracket_var_unused_error(name, *span, word_name));
                    }
                    // `len_idx.is_some()` with no bounds: the k8 flip -- a
                    // bare bracket entry confirming a length variable the
                    // effect already bound, nothing to push.
                }
            }
        }
        Ok(())
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

    /// P7.S3e (R1/R3, decision 1): `trait: TraitName['T] : member ( &'T ... --
    /// ... ) ; : member2 ( ... ) ; ... ;` -- a trait name, its single (implicit)
    /// type variable header, then one or more member signatures over that
    /// variable. Single-type-variable traits only (R16): a second header
    /// variable is a located error here; a member signature introducing a
    /// variable other than the header's is rejected once its own effect is
    /// fully parsed (`parse_trait_member_effect`).
    fn parse_trait_decl(&mut self) -> Result<TraitDecl, String> {
        let span = self.expect_word("trait:")?;
        let (name, name_span) = self.expect_word_any_spanned()?;
        reject_reserved_name("trait", &name, name_span)?;
        // P7.S6 (R5/R10): a `trait:` binds its single type variable in a
        // *mandatory* bracket. There is no such thing as a non-generic trait,
        // so the bracket is not optional the way a `type:`'s is.
        let (ty_var, ty_var_span, ty_var_kind) = match self.peek() {
            Some((Token::LBracket, _)) => {
                let vars = self.parse_header_bracket(&name)?;
                if vars.len() > 1 {
                    return Err(multi_variable_trait_error(&name, vars[1].1));
                }
                let (var, var_span, kind) = vars
                    .into_iter()
                    .next()
                    .expect("parse_header_bracket rejects an empty bracket");
                (var, var_span, kind)
            }
            // R10: the retired postfix variable (`trait: Ord 'T`). Its own
            // error rather than the neither-form one below, which would
            // wrongly claim no type variable was written at all.
            Some((Token::Word(w), s)) if w.starts_with('\'') => {
                return Err(postfix_header_var_error("trait:", &name, w, *s));
            }
            // Neither form: the pre-existing located error, naming the bracket
            // form. Both arms name it, not just the first, so a `trait: Ord`
            // at EOF still advises the bracket rather than the postfix word.
            Some((tok, s)) => {
                return Err(format!(
                    "parse error: expected a type variable or bracketed header (`trait: {name}['T]`) after `trait: {name}`, found {tok:?} at line {}, col {}",
                    s.line, s.col
                ));
            }
            None => {
                return Err(self.eof_error(&format!(
                    "a type variable (`'T`) or bracketed header (`trait: {name}['T]`)"
                )))
            }
        };
        // A `'`-prefixed word *after* the bracket is a second header variable
        // written in the retired postfix form; `multi_variable_trait_error` is
        // the sharper report of the two, since one variable is the hard limit
        // either way.
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('\'')) {
            return Err(multi_variable_trait_error(&name, ty_var_span));
        }
        let mut members = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                // P7.S3s-follow: a trait member is declared `: name ( sig ) ;`,
                // the same form `parse_worddef` uses. `:` is an ordinary word
                // token, so the guard distinguishes it from the retired bare
                // `name ( sig )` form below.
                Some((Token::Word(w), _)) if w == ":" => {
                    self.pos += 1;
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
                    // `call`, `slice`, and `subslice` are tested separately because
                    // none is in `BUILTIN_WORDS`: each is its own arm in
                    // `check_term`/`poly_call_term`, so `is_name_dispatched_builtin`
                    // does not cover them. They cannot join that set either -- that
                    // set is also what the `intrinsics` import gates (P8 S2 R2), and
                    // none of the three is import-gated.
                    if is_name_dispatched_builtin(&member_name)
                        || matches!(member_name.as_str(), "call" | "slice" | "subslice")
                    {
                        return Err(builtin_named_trait_member_error(
                            &name,
                            &member_name,
                            member_span,
                        ));
                    }
                    // P7.S3s-follow: the optional `inline` keyword sits in the
                    // one slot between the member name and its `(`, mirroring
                    // `parse_worddef` exactly. The name is already consumed, so
                    // `: inline ( ... ) ;` still declares a member *named*
                    // `inline`, and a second `inline` falls through to the `(`
                    // and fails there.
                    let declares_inline =
                        matches!(self.peek(), Some((Token::Word(w), _)) if w == "inline");
                    if declares_inline {
                        self.pos += 1;
                    }
                    // P7.S6 (R6): the same bound bracket `parse_worddef`
                    // admits, in the same slot relative to `inline`. A
                    // trait member's implicit header variable is unchanged.
                    let bound_bracket = self.parse_optional_bound_bracket()?;
                    self.expect(Token::LParen)?;
                    let mut sig = self.parse_trait_member_effect(
                        &ty_var,
                        ty_var_span,
                        &ty_var_kind,
                        &name,
                        member_span,
                    )?;
                    if let Some(bracket) = &bound_bracket {
                        self.attach_bracket_bounds(&mut sig, bracket, &member_name)?;
                    }
                    self.expect(Token::RParen)?;
                    self.expect(Token::Semicolon)?;
                    members.push(TraitMember {
                        name: member_name,
                        sig,
                        declares_inline,
                        // P7b.S2 (S2-15): the member's own position, for the
                        // per-member diagnostics (S2-15.a/d).
                        span: member_span,
                    });
                }
                // P7.S3s-follow: a bare word in member position is the retired
                // `name ( sig )` form -- name the replacement, not a token
                // mismatch.
                Some((Token::Word(_), _)) => {
                    let (member_name, member_span) = self.expect_word_any_spanned()?;
                    return Err(bare_trait_member_error(&name, &member_name, member_span));
                }
                Some((tok, s)) => {
                    return Err(format!(
                        "parse error: expected `:` or `;`, found {tok:?} at line {}, col {}",
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
            // P7b.S2 (S2-1): publish the header bracket's kind annotation and
            // the header variable's own span instead of discarding them (F4).
            var_kind: ty_var_kind,
            var_span: ty_var_span,
            members,
            module: self.module,
            span: name_span,
        })
    }

    /// One trait member's signature, positioned just past its opening `(`:
    /// an ordinary poly effect, except the trait's own type variable is
    /// pre-interned at id 0 (its binding occurrence is the trait header, not
    /// here), so every `'`-mention inside the member is a *use*.
    ///
    /// P7b.S2 (S2-1): the header's published kind annotation and the header
    /// variable's own span now ride along. Var 0 is interned with the header
    /// span (so an annotation-vs-usage conflict names the header as origin,
    /// S2-15.b) and the member builder's established-kind table is seeded
    /// with the header kind *before* the effect parse, so `mark_ty_star`/
    /// `mark_ty_arrow` check every mention against the declared kind instead
    /// of letting the first mention bind it. An `'F: * -> *` header with a
    /// bare `'F` mention in a member is now a located error naming both
    /// spans, and a `*`-kinded header with an `'F['T]` member dies the
    /// mirrored way (S2-15.b).
    ///
    /// The member single-variable gate is lifted (S2-1): a member may declare
    /// its own local variables (`map`'s `'T`/`'U`), which intern after the
    /// header var (the header keeps id 0 in each member's sig). The one-var
    /// limit stays enforced on the *header bracket* itself
    /// (`multi_variable_trait_error` in `parse_trait_decl`, unchanged).
    fn parse_trait_member_effect(
        &mut self,
        ty_var: &str,
        ty_var_span: Span,
        header_kind: &Kind,
        trait_name: &str,
        member_span: Span,
    ) -> Result<PolySig, String> {
        let mut builder = PolyBuilder::default();
        let (id, _) = builder.intern_ty_var(ty_var, ty_var_span)?;
        // P7b.S2 (S2-1): seed var 0 with the header's declared kind before
        // the effect parse -- `intern_ty_var` alone never touches
        // `ty_established_kind`, which would let the member's first mention
        // re-bind the kind and silently ignore the header annotation (F4).
        debug_assert_eq!(id, 0, "the trait header var is each member sig's var 0");
        builder.seed_ty_kind(id, header_kind.clone());
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
        for t in sig.inputs.iter().chain(&sig.outputs) {
            if !member_shape_is_supported(t) {
                // P7b.S2 (S2-15.d): an App inside a member quotation row is
                // its own fence (F10 -- declarations represent it, but
                // `call` cannot see through one), distinct from the general
                // unsupported-shape rejection. The fence fires only when the
                // unsupported shape actually routes through a quotation: a
                // plain-slot App (bare or under array/`&`) headed by a member
                // local is unsupported too, but no quotation is involved, so
                // it takes the generic message instead. The row-mentions
                // predicate answers "an App exists somewhere" -- true for a
                // plain App too, the S2-3-supported dispatchable shape -- so
                // the row-nested case is isolated by the *second* conjunct:
                // `poly_type_app_head` finds a plain-position App head but
                // does not recurse into quotation rows, so `None` here means
                // the App is row-nested.
                if member_quotation_row_mentions_app(t) && poly_type_app_head(t).is_none() {
                    return Err(app_in_member_quotation_row_error(trait_name, member_span));
                }
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
    ///
    /// P7.S4 (R1): the target is a `PolyType` pattern over the impl's own
    /// variables (`['T N]`, `'T`, `Box['T]`), not a concrete `Type`. A concrete
    /// target (`Point`, `array[i64 4]`) folds to `PolyType::Concrete(t)` and keeps
    /// the existing monomorphic path; a generic target carries variables and
    /// the member word is polymorphic.
    fn parse_impl_decl(&mut self) -> Result<(ImplDecl, Vec<WordDef>), String> {
        let span = self.expect_word("impl:")?;
        let (trait_name, trait_span) = self.expect_word_any_spanned()?;
        self.expect_word("for")?;
        let mut target = self.parse_impl_target()?;
        target.bounds = self.parse_impl_bounds(&target)?;
        let trait_id = find_trait_in_module(
            self.traits,
            &trait_name,
            self.module,
            self.imports,
            self.selective,
            self.trait_origin,
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
                    let (member_name, word) = self.parse_impl_member_body(trait_id, &target)?;
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
                target,
                module: self.module,
                span,
                bindings,
                resolved: Vec::new(),
            },
            words,
        ))
    }

    /// P7.S4 (R1): parse the target of an `impl:` declaration as a `PolyType`
    /// pattern over the impl's own type/length variables. Reuses the
    /// `parse_poly_slot` machinery (which admits `'T`, `['T N]`, `&'T`,
    /// `^'T`, `Box['T]`) but forbids the bound syntax (`'T: Copy`, the
    /// `:`-bound arm of `parse_poly_ty_var`) and row variables (`..s`), which
    /// are out of scope for an impl target. A concrete target (`Point`,
    /// `array[i64 4]`) folds to `PolyType::Concrete(t)`.
    ///
    /// P7b.S2 (S2-4): a word naming a generic `type:` header, bare or
    /// under-applied, desugars to the ctor applied to fresh pattern
    /// variables, one per declared slot (`for Option` ≡
    /// `for Option['ctor0 …]`, `for Result[i64]` ≡ `for Result[i64
    /// 'ctor1]`) -- m2-proven mechanics making bare ctors expressible as
    /// existing S4 applied-var patterns, everything downstream unchanged.
    /// The user's span and name ride along (`user_spelling`) so diagnostics
    /// render `Option`, not `Option['ctor0]`.
    fn parse_impl_target(&mut self) -> Result<ImplTarget, String> {
        let mut builder = PolyBuilder {
            forbid_bounds: true,
            ..PolyBuilder::default()
        };
        let parsed = self.parse_impl_target_pattern(&mut builder)?;
        if builder.row_in.is_some() || builder.row_out.is_some() {
            return Err(impl_target_row_var_error());
        }
        let pattern = self.raw_to_poly_type(parsed.raw)?;
        if let Some(head) = poly_type_app_head(&pattern) {
            let var = builder.ty_names[head as usize].clone();
            let span = builder.ty_var_spans[head as usize];
            return Err(impl_target_app_unsupported_error(&var, span));
        }
        let user_spelling = parsed.ctor_span.and_then(|span| {
            impl_target_user_spelling(
                &pattern,
                &builder.ty_names,
                &builder.len_names,
                parsed.padded,
                span,
            )
        });
        Ok(ImplTarget {
            pattern,
            ty_kinds: vec![Kind::Star; builder.ty_names.len()],
            ty_var_names: builder.ty_names,
            ty_var_spans: builder.ty_var_spans,
            len_var_names: builder.len_names,
            len_var_spans: builder.len_var_spans,
            user_spelling,
            bounds: Vec::new(),
        })
    }

    /// P7b.S2 (S2-4): the impl-target slot path -- the ordinary poly-slot
    /// reader, plus the ctor intercept: a word naming a generic `type:`
    /// header takes the (padding-aware) application reader, so a bare or
    /// partially-applied ctor desugars instead of dying at the shared arity
    /// gate (F3).
    fn parse_impl_target_pattern(
        &mut self,
        builder: &mut PolyBuilder,
    ) -> Result<RawImplTargetPattern, String> {
        if let Some((Token::Word(w), span)) = self.peek() {
            let (w, span) = (w.clone(), *span);
            // `array[...]` keeps its named-array reader (parse_poly_slot's
            // first arm) -- only a genuine generic `type:` header intercepts.
            if !w.starts_with('\'')
                && !w.starts_with('&')
                && !w.starts_with('^')
                && !self.array_type_ahead()
            {
                if let Some((is_enum, idx, module)) = self.poly_generic_header(&w, span)? {
                    // Consume the ctor name itself -- the application reader
                    // enters positioned at the bracket (or, for a bare
                    // target, at whatever ends the target slot).
                    self.pos += 1;
                    let (raw, padded) = self.parse_poly_generic_application(
                        builder,
                        false,
                        &w,
                        is_enum,
                        idx,
                        module,
                        span,
                        UnderApplication::PadImplTarget,
                    )?;
                    return Ok(RawImplTargetPattern {
                        raw,
                        ctor_span: Some(span),
                        padded,
                    });
                }
            }
        }
        Ok(RawImplTargetPattern {
            raw: self.parse_poly_slot(builder, false)?,
            ctor_span: None,
            padded: (0, 0),
        })
    }

    /// P7.S4b (R1): parse an optional `where`-clause on an `impl:` target,
    /// declaring bounds on the impl's own type variables. The clause reads
    /// `where 'T: Show 'V: Eq` — each variable name is resolved against the
    /// target's already-parsed `ty_var_names` table (erroring on an unknown
    /// name), then `:` and the bound list reuse `parse_capabilities` (the
    /// existing bound-list parser). This deliberately does NOT reuse
    /// `parse_poly_ty_var`: since P7.S6 (R7) that function *rejects* every
    /// bound it detects (bounds belong in a word's bound bracket, and on an
    /// `impl:` target in this `where`-clause), so routing a `where`-clause
    /// through it would reject the one spelling that is legal here. A target
    /// with no `where`-clause behaves exactly as today (`bounds: vec![]`).
    fn parse_impl_bounds(&mut self, target: &ImplTarget) -> Result<Vec<(u32, Bound)>, String> {
        if !matches!(self.peek(), Some((Token::Word(w), _)) if w == "where") {
            return Ok(Vec::new());
        }
        self.pos += 1; // consume `where`
        let mut bounds = Vec::new();
        loop {
            // Each entry: `'T: Cap1 Cap2`. The `:` may be glued to the
            // variable name (`'T:`) or a separate token (`'T :`), exactly as
            // `parse_poly_ty_var` handles the bound colon — the lexer does
            // not split on `:`, so `'T:` is one `Token::Word`.
            let (name, span, glued_colon) = match self.peek() {
                Some((Token::Word(w), s)) if w.starts_with('\'') => {
                    let glued = w.ends_with(':') && w.len() > 1;
                    let name = if glued {
                        w[..w.len() - 1].to_string()
                    } else {
                        w.clone()
                    };
                    (name, *s, glued)
                }
                Some((tok, span)) => {
                    return Err(format!(
                        "error: expected a type variable after `where` at line {}, col {}, found `{}`",
                        span.line,
                        span.col,
                        describe_token(tok)
                    ));
                }
                None => return Err(self.eof_error("a type variable after `where`")),
            };
            self.pos += 1; // consume the variable name (and glued `:` if any)
                           // Resolve against the target's `ty_var_names` (bounds apply to
                           // type variables only; a length variable name is an error here).
            let id = target
                .ty_var_names
                .iter()
                .position(|n| n == &name)
                .ok_or_else(|| {
                    format!(
                        "error: unknown type variable `{name}` in `where` clause at line {}, col {}",
                        span.line, span.col
                    )
                })? as u32;
            // Expect `:` then the bound list. A glued colon was already
            // consumed with the variable name; a standalone `:` is a separate
            // token.
            let colon_span = if glued_colon {
                span
            } else {
                match self.peek() {
                    Some((Token::Word(w), s)) if w == ":" => *s,
                    Some((tok, span)) => {
                        return Err(format!(
                            "error: expected `:` after `{name}` in `where` clause at line {}, col {}, found `{}`",
                            span.line,
                            span.col,
                            describe_token(tok)
                        ));
                    }
                    None => {
                        return Err(self.eof_error("`:` after a type variable in `where` clause"))
                    }
                }
            };
            if !glued_colon {
                self.pos += 1; // consume standalone `:`
            }
            let caps = self.parse_capabilities(colon_span, false)?;
            for b in caps {
                bounds.push((id, b));
            }
            // Continue if another variable name follows; otherwise the
            // `where`-clause is done and the next token starts the member body.
            if !matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('\'')) {
                break;
            }
        }
        Ok(bounds)
    }

    /// P7.S3r (R2/R4a/R5/R6): one `: member [| binders |] body ;` inside an
    /// `impl:` block, desugared to the top-level word the member binds to. The
    /// declared effect is the trait member's signature grounded at the `for`
    /// type through `ast`'s `ground_member_type`; there is no `(` to parse,
    /// since restating the inherited signature is rejected.
    fn parse_impl_member_body(
        &mut self,
        trait_id: TraitId,
        target: &ImplTarget,
    ) -> Result<(String, WordDef), String> {
        self.expect_word(":")?;
        let (member_name, member_span) = self.expect_word_any_spanned()?;
        let trait_name = self.traits[trait_id.index()].name.clone();
        let trait_module = self.traits[trait_id.index()].module;
        // P7.S3s-follow: widen the member lookup to take `(sig,
        // declares_inline)` in one pass, so both branches below inherit the
        // member's `inline` flag instead of hardcoding `false`.
        let Some((sig, declares_inline)) = self.traits[trait_id.index()]
            .members
            .iter()
            .find(|m| m.name == member_name)
            .map(|m| (m.sig.clone(), m.declares_inline))
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
        let body = self.parse_terms("`;`", |tok| matches!(tok, Token::Semicolon))?;
        self.expect(Token::Semicolon)?;
        let name = synth_member_word_name(&member_name, &trait_name, trait_module, target);
        let body = rewrite_member_self_calls(&body, &member_name, &name)?;
        // P7b.S2 (S2-5/S2-6): the grounding diagnostics' shared context --
        // the member's position and the target as the user spelled it (S2-4),
        // so every member-grounding error names its offending position.
        let target_display = target
            .user_spelling
            .clone()
            .map(|(spelling, _)| spelling)
            .unwrap_or_else(|| {
                render_target_pt(&target.pattern, &target.ty_var_names, &target.len_var_names)
            });
        let dg = MemberGrounding {
            trait_name: trait_name.as_str(),
            member: member_name.as_str(),
            member_span,
            head_name: sig.ty_var_names[0].as_str(),
            target_display: target_display.as_str(),
            target_var: match &target.pattern {
                PolyType::Var(0) => Some((
                    target.ty_var_names[0].as_str(),
                    target.ty_var_spans.first().copied().unwrap_or_default(),
                )),
                _ => None,
            },
        };
        if target.is_concrete() {
            // R5 concrete path: the existing monomorphic member word, with
            // the trait member's signature grounded at the concrete `Type`.
            //
            // P7b.S2 (S2-6): an App-headed member has no mono representation
            // against a concrete target (its applied arguments are member
            // locals) -- a located fence before grounding,
            // `ground_member_type`'s own App arm being the unreachable
            // backstop the fence guards.
            fence_member_app_against_concrete_target(&sig.inputs, &sig.outputs, &dg)?;
            let target_ty = target.concrete_ty().expect("checked is_concrete");
            let ground =
                |slots: &[PolyType], arrays: &mut Vec<ArrayDecl>, refs: &mut Vec<RefDecl>| {
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
            Ok((
                member_name,
                WordDef {
                    name,
                    effect,
                    body,
                    poly: None,
                    declares_inline,
                    module: self.module,
                    span: member_span,
                    declared_globals: None,
                    is_trait_member: true,
                },
            ))
        } else {
            // P7.S4 (R5) generic path: the member word is polymorphic, its
            // `PolySig` the trait member's signature grounded over the
            // member word's union id space. P7.S4b (R3): the `PolySig`'s
            // `bounds` are populated from the target's declared
            // `where`-clause bounds (empty when no `where`-clause is
            // present).
            //
            // P7b.S2 (S2-5): the union is load-bearing -- the target's
            // variables keep their ids and order (so every `where`-bound,
            // keyed by target id, survives the merge); the member's own
            // locals append after them, identified ones aliased to the
            // target slot a dispatchable input's application argument names.
            // Every variable gets the span of its introduction (target var →
            // target span; member local → member sig span) -- no more
            // `Span::default()` stamps.
            let union = build_member_var_union(target, &sig, &dg)?;
            let inputs = sig
                .inputs
                .iter()
                .map(|t| ground_member_poly(t, &target.pattern, &union.map, &dg))
                .collect::<Result<Vec<_>, _>>()?;
            let outputs = sig
                .outputs
                .iter()
                .map(|t| ground_member_poly(t, &target.pattern, &union.map, &dg))
                .collect::<Result<Vec<_>, _>>()?;
            let poly_sig = PolySig {
                row_in: None,
                inputs,
                outputs,
                row_out: None,
                bounds: target.bounds.clone(),
                ty_kinds: target
                    .ty_kinds
                    .iter()
                    .cloned()
                    .chain(std::iter::repeat_n(Kind::Star, union.appended.len()))
                    .collect(),
                ty_var_names: target
                    .ty_var_names
                    .iter()
                    .cloned()
                    .chain(union.appended.iter().map(|(n, _)| n.clone()))
                    .collect(),
                ty_var_spans: target
                    .ty_var_spans
                    .iter()
                    .copied()
                    .chain(union.appended.iter().map(|(_, s)| *s))
                    .collect(),
                len_var_names: target.len_var_names.clone(),
                len_var_spans: target.len_var_spans.clone(),
                row_var_names: Vec::new(),
            };
            Ok((
                member_name,
                WordDef {
                    name,
                    effect: StackEffect::default(),
                    body,
                    poly: Some(Box::new(poly_sig)),
                    declares_inline,
                    module: self.module,
                    span: member_span,
                    declared_globals: None,
                    is_trait_member: true,
                },
            ))
        }
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
        // P7.S6 (R1): `array[T N]` -- the named array type. The word
        // `array` followed by `[` enters the poly array reader, which threads
        // `builder` so a variable element/count is preserved.
        if self.array_type_ahead() {
            self.pos += 1;
            return self.parse_poly_array(builder, word_is_output);
        }
        // P7.S6 (R4): a bare `[` is a quotation effect unconditionally; an
        // array is spelled `array[T N]` and was taken by the arm above.
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            return self.parse_poly_quotation(builder, word_is_output);
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('\'')) {
            let (w, span) = self.expect_word_any_spanned()?;
            let raw = self.parse_poly_ty_var(builder, &w, span)?;
            // P7b.S1 (S1-6): the `[`-router. A `[` following the variable is
            // a type application when its bracket holds no top-depth `--`;
            // when it does, this is unchanged -- the `[` opens the *next*
            // slot in the effect (a quotation parameter), not an
            // application on this variable.
            if let RawTy::Var(head) = raw {
                if matches!(self.peek(), Some((Token::LBracket, _)))
                    && !self.top_depth_arrow_present(0)
                {
                    return self.parse_poly_var_application(builder, word_is_output, head, span);
                }
                // P7b.S1 (S1-3/S1-4): a bare mention -- establishes `Star`
                // on first sight, or checks against an already-established
                // `Arrow` (S1-15.b).
                builder.mark_ty_star(head, span)?;
            }
            return Ok(raw);
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
                // Bare sigil (`& 'T`, `&array['T 4]`): the referent is a genuine
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
                    // P7b.S1 review fix: this glued-sigil site used to skip
                    // straight past kind collection -- unlike the spaced `'`
                    // arm above, which marks a bare mention immediately
                    // after interning. A referent behind `&`/`&!` is always
                    // a bare (non-applied) use, so this mirrors that arm's
                    // `mark_ty_star` call, not its `[`-router.
                    if let RawTy::Var(head) = inner {
                        builder.mark_ty_star(head, remainder_span)?;
                    }
                    return Ok(RawTy::Ref(Box::new(inner), mutable));
                }
                // P7.S6 (R1a): `&array['T 4]` -- the `&` and `array` are
                // glued into one word, so the `[`-dispatch sites cannot
                // reach this spelling. Intercept `array` ahead of the
                // concrete-reader fallthrough, dispatching into the *poly*
                // array reader so a variable element is preserved.
                if remainder == ARRAY_TYPE_NAME
                    && matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _)))
                {
                    self.pos += 1;
                    let inner = self.parse_poly_array(builder, word_is_output)?;
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
                // Bare run (`^ 'T`, `^array['T 4]`): the payload is a genuine
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
                    let inner = self.parse_poly_ty_var(builder, &remainder, remainder_span)?;
                    // P7b.S1 review fix: mirrors the `&`-glued arm above --
                    // a payload behind `^`/`^^` is always a bare use, so mark
                    // it here rather than skipping kind collection entirely.
                    if let RawTy::Var(head) = inner {
                        builder.mark_ty_star(head, remainder_span)?;
                    }
                    Some(inner)
                } else if remainder == ARRAY_TYPE_NAME
                    && matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _)))
                {
                    // P7.S6 (R1a): `^array['T 4]` -- same interception as the
                    // `&` arm above, dispatching into the poly array reader.
                    self.pos += 1;
                    Some(self.parse_poly_array(builder, word_is_output)?)
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
        // P7.S3h: an `owning` slot. Unlike the `&` and `^` arms above, this is
        // here *only* for the variable-bearing case: the `parse_type_expr`
        // fallthrough below already folds a fully-concrete `owning` effect
        // correctly, but it resolves the effect's slots concretely, so
        // `owning [ 'T -- ]` would be blamed on `'T` as an unknown type when
        // the real reason is that `PolyType::Quotation` has nowhere to record
        // the owning flavour, and folding one would silently hand the caller a
        // plain quotation.
        if self.owning_quotation_ahead() {
            let (_, span) = self.expect_word_any_spanned()?;
            if !self.quotation_effect_opens_here() {
                return Err(owning_without_effect_error(span));
            }
            self.pos += 1;
            // An `owning` effect is not inline, so `parse_poly_quotation_inner`
            // has already rejected a row on either side.
            let RawTy::Quotation(ins, outs, _, None, None) =
                self.parse_poly_quotation_inner(builder, false, word_is_output)?
            else {
                unreachable!("`parse_poly_quotation_inner` returns a row-free quotation here")
            };
            let concrete = |row: &[RawTy]| {
                row.iter()
                    .map(|r| match r {
                        RawTy::Concrete(t) => Some(*t),
                        _ => None,
                    })
                    .collect::<Option<Vec<Type>>>()
            };
            let (Some(ci), Some(co)) = (concrete(&ins), concrete(&outs)) else {
                return Err(polymorphic_owning_quotation_error(span));
            };
            return Ok(RawTy::Concrete(crate::ast::owning_quotation_type(ci, co)));
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
                        let (raw, _) = self.parse_poly_generic_application(
                            builder,
                            word_is_output,
                            &w,
                            is_enum,
                            idx,
                            module,
                            span,
                            UnderApplication::Error,
                        )?;
                        return Ok(raw);
                    }
                }
            }
        }
        // R11: a poly-effect slot position has no legal spelling with a
        // `:` -- every variable-bearing form (`'T`, `&'T`, `^'T`, ...) has
        // already returned above, so a word containing no `'` that is
        // followed by `:` (spaced) or ends with `:` (glued) here is always
        // an attempted slot name, never a legal type.
        // Not for `builder.forbid_bounds` (an `impl:` target): that route
        // shares this reader for a bare/concrete target pattern with no
        // slot-name concept at all, and the token right after the target
        // can legitimately be the impl body's own leading `:` (Non-goals:
        // impl member bodies are structurally excluded from this sugar).
        if !builder.forbid_bounds {
            if let Some((Token::Word(w), span)) = self.peek() {
                let (w, span) = (w.clone(), *span);
                if !w.contains('\'') {
                    let glued = w.len() > 1 && w.ends_with(':');
                    let spaced = matches!(self.tokens.get(self.pos + 1), Some((Token::Word(c), _)) if c == ":");
                    if glued || spaced {
                        return Err(poly_slot_name_not_supported_error(span));
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
    /// Returns the folded pattern plus the number of type/length slots the
    /// impl-target desugar padded (`(0, 0)` on every signature-site path,
    /// which never pads) -- the exact padding record the user-spelling
    /// renderer consumes (P7b.S2 review: retired the name-prefix heuristic).
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
        under_application: UnderApplication,
    ) -> Result<(RawTy, (usize, usize)), String> {
        // P7.S6a (R7): the signature twin of `resolve_type_or_apply`'s split
        // (R6) -- `ty_arity` type slots, parsed as poly slots so a variable
        // is preserved, followed by `len_arity` length slots, each either a
        // bare `'N` interned as a length variable through `builder` (exactly
        // as `parse_poly_array` already does) or a literal count.
        let (ty_arity, len_arity) = if is_enum {
            (
                self.generics.enums[idx].ty_var_names.len(),
                self.generics.enums[idx].len_var_names.len(),
            )
        } else {
            (
                self.generics.structs[idx].ty_var_names.len(),
                self.generics.structs[idx].len_var_names.len(),
            )
        };
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            if matches!(under_application, UnderApplication::Error) {
                return Err(generic_arity_error(name, ty_arity, len_arity, 0, 0, span));
            }
            // S2-4: a bare ctor target (`for Box`) -- an empty explicit
            // prefix falls through to the padding below, which desugars it
            // to the ctor applied to a fresh pattern variable per declared
            // slot. The bracket-reading loop is skipped entirely.
            let (mut args, mut lens) = (Vec::new(), Vec::new());
            let padded =
                Self::pad_impl_ctor_slots(builder, span, ty_arity, len_arity, &mut args, &mut lens);
            return Ok((
                RawTy::Generic {
                    is_enum,
                    idx,
                    module,
                    args,
                    len_args: lens,
                    name: name.to_string(),
                    span,
                },
                padded,
            ));
        }
        self.pos += 1;
        let mut args = Vec::new();
        let mut lens: Vec<RawLen> = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated generic type application)"));
                }
                _ => {}
            }
            if args.len() < ty_arity {
                args.push(self.parse_poly_slot(builder, word_is_output)?);
                continue;
            }
            if lens.len() < len_arity {
                match self.peek().cloned() {
                    Some((Token::Word(w), wspan)) if w.starts_with('\'') => {
                        self.pos += 1;
                        // P7b.S2 review (S2-4): reserved `'ctor…` prefixes in
                        // an impl target ctor application's length slots (the
                        // twin of the type-slot check in `parse_poly_ty_var`).
                        builder.check_impl_target_reserved_name(&w, wspan)?;
                        lens.push(RawLen::Var(builder.intern_len_var(&w, wspan)?));
                    }
                    _ => lens.push(RawLen::Concrete(self.parse_array_count(name)?)),
                }
                continue;
            }
            // Over-application, beyond both declared arities: consume
            // permissively (as a poly slot) so the arity check below reports
            // the real supplied count instead of a misleading length-literal
            // error.
            args.push(self.parse_poly_slot(builder, word_is_output)?);
        }
        if args.len() != ty_arity || lens.len() != len_arity {
            // S2-4: an under-applied impl target pads the missing slots with
            // fresh pattern variables (`for Result[i64]` ≡ `for Result[i64
            // 'ctor1]`); over-application and every signature-site shape
            // keep the arity error.
            let padded = if matches!(under_application, UnderApplication::PadImplTarget)
                && args.len() < ty_arity
                && lens.len() <= len_arity
            {
                Self::pad_impl_ctor_slots(builder, span, ty_arity, len_arity, &mut args, &mut lens)
            } else {
                return Err(generic_arity_error(
                    name,
                    ty_arity,
                    len_arity,
                    args.len(),
                    lens.len(),
                    span,
                ));
            };
            return Ok((
                RawTy::Generic {
                    is_enum,
                    idx,
                    module,
                    args,
                    len_args: lens,
                    name: name.to_string(),
                    span,
                },
                padded,
            ));
        }
        Ok((
            RawTy::Generic {
                is_enum,
                idx,
                module,
                args,
                len_args: lens,
                name: name.to_string(),
                span,
            },
            (0, 0),
        ))
    }

    /// P7b.S2 (S2-4): fill an under-applied ctor target's remaining type
    /// slots with fresh pattern variables (`'ctor0`, `'ctor1`, …, named by
    /// the slot they fill) and its remaining length slots with fresh length
    /// variables, all spanning the ctor name -- the m2-proven desugar that
    /// makes bare (`for Box`) and partially-applied (`for Result[i64]`)
    /// ctor targets expressible as existing S4 applied-var patterns.
    /// Returns the number of generated type/length variables, so the
    /// user-spelling renderer knows the exact padded suffix without a
    /// name-prefix heuristic (P7b.S2 review).
    fn pad_impl_ctor_slots(
        builder: &mut PolyBuilder,
        span: Span,
        ty_arity: usize,
        len_arity: usize,
        args: &mut Vec<RawTy>,
        lens: &mut Vec<RawLen>,
    ) -> (usize, usize) {
        let mut ty_padded = 0usize;
        while args.len() < ty_arity {
            let slot = args.len();
            // Belt-and-braces: user-written `'ctor…` names are rejected in
            // the impl-target path (`check_impl_target_reserved_name`), so
            // the slot-indexed name is always free -- skip to the next free
            // index anyway, so the generator itself can never alias one
            // variable onto two slots.
            let mut n = slot;
            let name = loop {
                let candidate = format!("'ctor{n}");
                if !builder.ty_index.contains_key(&candidate) {
                    break candidate;
                }
                n += 1;
            };
            // Fresh, internally-generated names cannot collide and cannot
            // carry a bound; the unwrap-equivalent is the `?`'s absence.
            let (id, _) = builder
                .intern_ty_var(&name, span)
                .expect("a generated 'ctorN variable is fresh and boundless");
            args.push(RawTy::Var(id));
            ty_padded += 1;
        }
        let mut len_padded = 0usize;
        while lens.len() < len_arity {
            let slot = lens.len();
            let mut n = slot;
            let name = loop {
                let candidate = format!("'ctorlen{n}");
                if !builder.len_index.contains_key(&candidate) {
                    break candidate;
                }
                n += 1;
            };
            let id = builder
                .intern_len_var(&name, span)
                .expect("a generated 'ctorlenN variable is fresh and Star-free");
            lens.push(RawLen::Var(id));
            len_padded += 1;
        }
        (ty_padded, len_padded)
    }

    /// P7b.S1 (S1-6/S1-7): a type variable applied to type arguments
    /// (`'F['T]`) -- the higher-kinded twin of `parse_poly_generic_application`,
    /// but with no known header to read an arity from (the head is a
    /// *variable*, whose kind is inferred from this very application, S1-3),
    /// so every argument parses as a type slot with no arity bound.
    /// Positioned just past the variable; `span` is the variable's own span,
    /// for the empty-application diagnostic. Arguments are type expressions
    /// only (S1-6): a quotation-shaped argument is fenced by
    /// `parse_poly_app_arg`.
    fn parse_poly_var_application(
        &mut self,
        builder: &mut PolyBuilder,
        word_is_output: bool,
        head: u32,
        span: Span,
    ) -> Result<RawTy, String> {
        self.expect(Token::LBracket)?;
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated type application)"));
                }
                _ => args.push(self.parse_poly_app_arg(builder, word_is_output)?),
            }
        }
        if args.is_empty() {
            let var = builder.ty_names[head as usize].clone();
            return Err(empty_type_application_error(&var, span));
        }
        // P7b.S1 (S1-3/S1-4): an application-head mention -- establishes
        // `Arrow { domains: [Star; args.len()], .. }` on first sight, or
        // checks against an already-established kind (S1-15.a/d).
        builder.mark_ty_arrow(head, args.len(), span)?;
        Ok(RawTy::App { head, args })
    }

    /// P7b.S1 (S1-6): one argument of a type application -- a type
    /// expression only, never a quotation. A bare `[` here would otherwise
    /// be read as a quotation effect by `parse_poly_slot`'s own `[` arm,
    /// which is exactly the shape S1-6 fences: `'F[[ i64 -- i64 ]]` is a
    /// parse error, not an application argument.
    fn parse_poly_app_arg(
        &mut self,
        builder: &mut PolyBuilder,
        word_is_output: bool,
    ) -> Result<RawTy, String> {
        if let Some((tok, span)) = self.peek() {
            if matches!(tok, Token::LBracket | Token::TildeLBracket) {
                return Err(app_arg_quotation_error(*span));
            }
        }
        self.parse_poly_slot(builder, word_is_output)
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
        // P7.S4 (R1): when `forbid_bounds` is set (an `impl:` target), the
        // `:` starting a member body (`: show ...`) must not be consumed as
        // a bound colon. Only a *glued* colon (`'T:`) is still treated as a
        // bound, since it cannot be the member body's `:`.
        let bound_follows = if builder.forbid_bounds {
            glued_colon
        } else {
            glued_colon || matches!(self.peek(), Some((Token::Word(c), _)) if c == ":")
        };
        // P7.S6 (R7/R7a): a bound is detected here but never parsed here --
        // it belongs in the word's own bound bracket. The two entry paths get
        // different errors, and this is the only site that knows which one it
        // is on, so the selection happens here rather than post hoc in
        // `parse_impl_target`.
        if bound_follows {
            return Err(if builder.forbid_bounds {
                impl_target_bound_error()
            } else {
                bound_in_effect_error(&name, span)
            });
        }
        // P7b.S2 review (S2-4): the ctor desugar's `'ctor…` prefixes are
        // reserved inside an `impl:` target (see the builder helper) -- a
        // user variable so named would alias a padded slot and would be
        // misread as desugar padding by the user-spelling renderer.
        builder.check_impl_target_reserved_name(&name, span)?;
        let (id, _) = builder.intern_ty_var(&name, span)?;
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
    ///
    /// P7.S6 (R4a(i)): being entered *past* the opener is why R4a's validator
    /// is called here, once, at depth base `1`, rather than in each of the
    /// three callers.
    #[allow(clippy::type_complexity)]
    fn parse_poly_quotation_inner(
        &mut self,
        builder: &mut PolyBuilder,
        is_inline: bool,
        word_is_output: bool,
    ) -> Result<RawTy, String> {
        self.require_top_depth_arrow(1)?;
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
    /// `--` (inputs) or `]` (outputs). Like its concrete twin
    /// (`parse_quot_type_list`) this loop cannot detect a *missing* `--`:
    /// a bare array count reaches `parse_poly_slot` and fails there before the
    /// `]` is observed, so R4a's validator runs ahead of it.
    /// A leading `..`-prefixed name is R4's
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

    /// A polymorphic array `[ elem count ]`: `elem` recurses (so `array['T 'N]`
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
                // P7b.S2 review (S2-4): reserved `'ctor…` prefixes in an
                // impl target's length positions too (the twin of the
                // type-slot check in `parse_poly_ty_var`).
                builder.check_impl_target_reserved_name(&w, span)?;
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
                    "error: array type has invalid length {n} at line {}, col {} (`array[T N]` requires 1 <= N <= {})",
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
    /// P7.S6 (R6a): `bracket_mode` changes the `None` fallthrough. Outside a
    /// bracket, an unrecognised word is the enclosing signature's next slot,
    /// so the greedy list ends silently (`break`). Inside a bracket there is
    /// no next slot -- the only things that can follow a bound are another
    /// `'`-var (the next `var_decl`) or `]` -- so an unrecognised
    /// non-`'`-prefixed name is an unknown-capability error, not a silent
    /// break. A `'`-prefixed word still breaks (it is the next `var_decl`).
    fn parse_capabilities(
        &mut self,
        colon_span: Span,
        bracket_mode: bool,
    ) -> Result<Vec<Bound>, String> {
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
                None if out.is_empty() => {
                    return Err(unknown_capability_error(&c, span));
                }
                None => {
                    if bracket_mode && !c.starts_with('\'') {
                        return Err(unknown_capability_error(&c, span));
                    }
                    break;
                }
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
        let id = find_trait_in_module(
            self.traits,
            name,
            self.module,
            self.imports,
            self.selective,
            self.trait_origin,
        );
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
            // P7b.S1 (S1-7): no all-concrete fold -- the head names a
            // variable, which never grounds to a `Type` at parse time (that
            // is Phase 3's `apply_subst`/S1-11 grounding via `CtorImage`).
            RawTy::App { head, args } => {
                let args: Vec<PolyType> = args
                    .into_iter()
                    .map(|r| self.raw_to_poly_type(r))
                    .collect::<Result<_, _>>()?;
                PolyType::App { head, args }
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
                len_args,
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
                let len_args: Vec<Len> = len_args
                    .into_iter()
                    .map(|l| match l {
                        RawLen::Concrete(n) => Len::Concrete(n),
                        RawLen::Var(id) => Len::Var(id),
                    })
                    .collect();
                let concrete: Option<Vec<Type>> = args
                    .iter()
                    .map(|a| match a {
                        PolyType::Concrete(t) => Some(*t),
                        _ => None,
                    })
                    .collect();
                // R7: a length arg must also be fully concrete before
                // collapsing -- a variable type with a concrete length
                // (`Buffer['T 4]`) must stay `PolyType::Generic` so `'T` has
                // somewhere to bind.
                let concrete_lens: Option<Vec<Len>> = len_args
                    .iter()
                    .all(|l| matches!(l, Len::Concrete(_)))
                    .then(|| len_args.clone());
                if let (Some(concrete), Some(concrete_lens)) = (concrete, concrete_lens) {
                    let regs = MutRegistries {
                        structs: self.structs,
                        enums: self.enums,
                        arrays: self.arrays,
                        cells: self.owned_cells,
                        refs: self.refs,
                    };
                    PolyType::Concrete(if is_enum {
                        self.generics
                            .instantiate_enum(idx, &concrete, &concrete_lens, module, regs)
                    } else {
                        self.generics.instantiate_struct(
                            idx,
                            &concrete,
                            &concrete_lens,
                            module,
                            regs,
                        )
                    })
                } else {
                    let name: &'static str = Box::leak(name.into_boxed_str());
                    PolyType::Generic {
                        is_enum,
                        idx: idx as u32,
                        module,
                        args,
                        len_args,
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
        // P7.S6 (R1): `array[T N]` -- the named array type. The word `array`
        // followed by `[` enters the concrete array reader. A slot *named*
        // `array` (`array : i64`) has `:` as its next token, so this check
        // declines and `array` is consumed as a slot name below (R1b: no
        // special-case code needed).
        if self.array_type_ahead() {
            self.pos += 1;
            let ty = self.parse_array_type_expr()?;
            return Ok(TypedSlot { name: None, ty });
        }
        // A quotation-effect type has no name of its own to lead with, so a
        // bare `[` slot is recognised before the usual
        // name-then-optional-`:type` read (R3, R7). P7.S6 (R4): it is a
        // quotation effect unconditionally -- the array spelling leads with
        // the word `array` and was taken by the arm above.
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            let ty = self.parse_quotation_type_expr()?;
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
        // P7.S3h: an `owning` quotation type is likewise nameless, so it too is
        // recognised before the name-then-optional-`:type` read. `owning` is an
        // ordinary word to the lexer, so a slot *named* `owning` (`owning :
        // i64`) is still legal and falls through -- the keyword is reserved
        // against `type:`/variant names, which shadow the syntax, not against
        // every use of the spelling.
        if self.owning_quotation_ahead()
            && !matches!(self.tokens.get(self.pos + 1), Some((Token::Word(w), _)) if w == ":")
        {
            let ty = self.parse_owning_quotation_type_expr()?;
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
        } else if text.len() > 1 && text.ends_with(':') && !text[..text.len() - 1].contains(':') {
            // R1: the glued trailing-colon slot-name spelling (`a: i64`),
            // mirroring the glued `'T:` split in `parse_optional_bound_bracket`.
            // The name half must itself be `:`-free -- a qualified-name-shaped
            // token (`q::Point:`, or the degenerate `::`) is not a plausible
            // slot name, so it falls through to `resolve_type_or_apply` and
            // dies as an unknown-type error instead of minting a slot named
            // `:` or `q::Point`.
            let name = text[..text.len() - 1].to_string();
            let is_plain_word = matches!(
                crate::lexer::lex(&name).as_deref(),
                Ok([(Token::Word(_), _)])
            );
            if !is_plain_word {
                return Err(glued_slot_name_not_a_word_error(&text, &name, span));
            }
            let ty = self.parse_type_expr()?;
            Ok(TypedSlot {
                name: Some(name),
                ty,
            })
        } else if text.len() > 1 && text.matches(':').count() == 1 && !text.starts_with(':') {
            // R2: a single non-trailing `:` is a fully-glued slot-name
            // attempt (`a:i64`); a `::`-qualified type name never has
            // exactly one `:`, and a leading-colon token (`:i64`) names
            // nothing — both fall through to the resolver below.
            Err(glued_slot_name_needs_space_error(&text, span))
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
        // P7.S6 (R1): `array[T N]` -- the named array type.
        if self.array_type_ahead() {
            self.pos += 1;
            return self.parse_array_type_expr();
        }
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            // P7.S6 (R4): a bare `[` is a quotation effect unconditionally.
            self.parse_quotation_type_expr()
        } else if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('&')) {
            self.parse_ref_type_expr()
        } else if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^')) {
            self.parse_owning_cell_type_expr()
        } else if self.owning_quotation_ahead() {
            self.parse_owning_quotation_type_expr()
        } else {
            let (name, span) = self.expect_word_any_spanned()?;
            self.resolve_type_or_apply(&name, span)
        }
    }

    /// P7.S3h: whether the parser is positioned on the `owning` prefix. It is
    /// not a lexer delimiter, so it arrives as an ordinary word; type-position
    /// dispatch is first-token only, so every entry point must intercept it
    /// here rather than let it fall through to a type-name lookup (which
    /// reports `unknown type 'owning'`).
    fn owning_quotation_ahead(&self) -> bool {
        matches!(self.peek(), Some((Token::Word(w), _)) if w == OWNING_QUOTATION_KEYWORD)
    }

    /// P7.S3h: parse `owning [ <in> -- <out> ]` into a
    /// `Type::OwningQuotation`. The prefix is its own word token, so the effect
    /// behind it is read by the ordinary effect reader and nests exactly as a
    /// plain `[ ... -- ... ]` does.
    fn parse_owning_quotation_type_expr(&mut self) -> Result<Type, String> {
        let (_, span) = self.expect_word_any_spanned()?;
        if !self.quotation_effect_opens_here() {
            return Err(owning_without_effect_error(span));
        }
        let (inputs, outputs) = self.parse_quotation_effect_rows()?;
        Ok(crate::ast::owning_quotation_type(inputs, outputs))
    }

    /// Whether the very next token opens a quotation *effect*. Since P7.S6
    /// (R4) a bare `[` in a type position is unconditionally a quotation
    /// effect -- an array is spelled `array[T N]` -- so this is a plain peek.
    /// It stays a named predicate because the `owning` readers ask the
    /// question to raise their own error rather than to dispatch.
    fn quotation_effect_opens_here(&self) -> bool {
        matches!(self.peek(), Some((Token::LBracket, _)))
    }

    /// `^` is not a lexer delimiter, so `^^i64` arrives as one word.
    fn parse_owning_cell_type_expr(&mut self) -> Result<Type, String> {
        let (word, span) = self.expect_word_any_spanned()?;
        self.split_owning_cell_word(&word, span)
    }

    /// Resolve a `^`-led type word already lifted off the stream: count the
    /// leading `^`-run, resolve the remainder (recursing into the ongoing
    /// token stream when the run is bare, e.g. `^array[u8 4]`), then wrap once per
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
        } else if remainder == OWNING_QUOTATION_KEYWORD {
            // P7.S3v (R7): `^owning` glues into one word, so the `owning`
            // prefix never reaches `parse_type_expr`'s own
            // `owning_quotation_ahead` dispatch and the remainder resolves as
            // an unknown type name. The effect rows still follow as their own
            // tokens, so read them exactly as the spaced form does.
            let remainder_span = Span {
                col: span.col + run_len as u32,
                ..span
            };
            if !self.quotation_effect_opens_here() {
                return Err(owning_without_effect_error(remainder_span));
            }
            let (inputs, outputs) = self.parse_quotation_effect_rows()?;
            crate::ast::owning_quotation_type(inputs, outputs)
        } else if remainder == ARRAY_TYPE_NAME && matches!(self.peek(), Some((Token::LBracket, _)))
        {
            // P7.S6 (R1a): `^array[T N]` -- same interception as the `&`
            // splitter, dispatching into the concrete array reader.
            self.parse_array_type_expr()?
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
    /// splitter; `&!array[u8 64]` splits *across* tokens and recurses into the
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
        } else if remainder == ARRAY_TYPE_NAME && matches!(self.peek(), Some((Token::LBracket, _)))
        {
            // P7.S6 (R1a): `&array[T N]` -- `&` and `array` are glued into
            // one word, so the `[`-dispatch sites cannot reach this spelling.
            // Intercept `array` ahead of `resolve_type_or_apply`, which would
            // report "unknown type `array`".
            self.parse_array_type_expr()?
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

    /// P7.S6 (R1): whether the parser is positioned on the word `array`
    /// followed by `[`, which opens a named array type (`array[T N]`).
    /// `array` is reserved (R3), so no user type can shadow it and the
    /// dispatch is unambiguous. The existing array readers consume the `[`
    /// themselves, so callers advance past the `array` word before calling
    /// them.
    fn array_type_ahead(&self) -> bool {
        matches!(self.peek(), Some((Token::Word(w), _)) if w == ARRAY_TYPE_NAME)
            && matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _)))
    }

    /// P7.S6 (R4a): the bracket being entered must hold a top-depth `--`,
    /// i.e. it must really be a quotation effect. Since R4 retired the
    /// bare-`[`-as-array spelling, a quotation reader is entered on a bare
    /// `[` unconditionally, so what used to be a *disambiguator*
    /// is a *validator*, not a disambiguator: a bracket with no `--` is a
    /// located error naming the missing `--`, not a silent reroute into the
    /// array production.
    ///
    /// `depth_base` is the bracket nesting the caller has already consumed:
    /// `0` when the parser still sits *on* the opening bracket
    /// (`parse_quotation_effect_rows`), `1` when it has been consumed
    /// (`parse_poly_quotation_inner`, entered past its opener by all three of
    /// its callers). It is explicit rather than defaulted so no call site can
    /// be added without ruling on it: seeded at `0` past the bracket, a legal
    /// `~[ 'T -- Bool ]` meets its closing `]` first, falls to `-1`, never
    /// satisfies the `depth == 0` stop and runs to EOF -- false-rejecting
    /// every inline combinator.
    ///
    /// `Token::TildeLBracket` is a single token that opens a bracket, so the
    /// walk counts it too. Without that the validator fails *open*: a bracket
    /// holding a nested `~[ … -- … ]` passes vacuously on the inner arrow and
    /// then dies further down with a worse diagnostic.
    fn require_top_depth_arrow(&self, depth_base: i32) -> Result<(), String> {
        // R4a(iii): `~[` has no array reading anywhere in the grammar, so the
        // error must not offer `array[T N]` to an author who opened with one.
        // Every base-1 caller consumed exactly one opener token immediately
        // before entry, so the token behind the cursor *is* that opener.
        let opened_with_tilde = depth_base > 0
            && matches!(
                self.tokens.get(self.pos - 1),
                Some((Token::TildeLBracket, _))
            );
        let opener = if depth_base > 0 {
            self.tokens.get(self.pos - 1)
        } else {
            self.tokens.get(self.pos)
        };
        let span = opener.map(|(_, s)| *s).unwrap_or_default();
        if self.top_depth_arrow_present(depth_base) {
            return Ok(());
        }
        Err(quotation_effect_missing_arrow_error(
            span,
            opened_with_tilde,
        ))
    }

    /// P7b.S1 (S1-6): the boolean scan `require_top_depth_arrow` errors on,
    /// extracted as a **router predicate** -- whether the bracket the parser
    /// is positioned at (per `depth_base`, exactly as that function's own
    /// doc explains) holds a top-depth `--`. Used by `parse_poly_ty_var` to
    /// decide whether a `[` following a type variable opens a quotation slot
    /// (present) or a type application (absent); `require_top_depth_arrow`
    /// itself keeps its own `Result`-returning, error-on-absence contract for
    /// its existing callers.
    fn top_depth_arrow_present(&self, depth_base: i32) -> bool {
        let mut depth = depth_base;
        let mut i = self.pos;
        while let Some((tok, _)) = self.tokens.get(i) {
            match tok {
                Token::LBracket | Token::TildeLBracket => depth += 1,
                Token::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        break;
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
    /// read the same way), so the nil effect `[ -- ]` is legal.
    fn parse_quotation_type_expr(&mut self) -> Result<Type, String> {
        let (inputs, outputs) = self.parse_quotation_effect_rows()?;
        Ok(crate::ast::quotation_type(inputs, outputs))
    }

    /// The two rows of `[ <in-types> -- <out-types> ]`, positioned on the `[`.
    /// Shared by the plain and the `owning` reader so the two spellings can
    /// never drift apart on what an effect's rows are.
    fn parse_quotation_effect_rows(&mut self) -> Result<(Vec<Type>, Vec<Type>), String> {
        // R4a: the one depth-base-0 validator call -- the parser is still on
        // the `[`, which the `expect` below consumes.
        self.require_top_depth_arrow(0)?;
        self.expect(Token::LBracket)?;
        let inputs = self.parse_quot_type_list(true)?;
        self.expect_word("--")?;
        let outputs = self.parse_quot_type_list(false)?;
        self.expect(Token::RBracket)?;
        Ok((inputs, outputs))
    }

    /// One side of a quotation effect: type expressions until the delimiter
    /// (`--` for the input side, `]` for the output side). A malformed type on
    /// either side is a located parse error from `parse_type_expr` (R3). This
    /// loop cannot be where a missing `--` is detected: it dispatches every
    /// unmatched token to `parse_type_expr`, which dies on a bare array count
    /// (`4` is a `Token::Int`) before the `]` is ever observed. Hence R4a's
    /// validator ahead of the reader.
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
    /// mirrors `find_type_in_module`'s `name_static` match (R8d, slice 5b): an
    /// imported enum's `.name` carries `resolve::mangle`'s module suffix but
    /// its variants' `.name_static` stays the user-typed spelling.
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
    /// a located error naming the full `array[T N]` spelling and the invalid
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
                    "error: array type `array[{element} {n}]` has invalid length {n} at line {}, col {} (`array[T N]` requires N <= {})",
                    span.line, span.col, u32::MAX
                ))
            }
            Some((Token::Int(n), span)) => {
                self.pos += 1;
                Err(format!(
                    "error: array type `array[{element} {n}]` has invalid length {n} at line {}, col {} (`array[T N]` requires N >= 1)",
                    span.line, span.col
                ))
            }
            Some((tok, span)) => Err(format!(
                "error: array count must be a decimal literal, found `{}` at line {}, col {} (`array[T N]` requires a literal N, no const-expr eval)",
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
        // The struct name is already registered by the pre-pass.
        let name = self.expect_word_any()?;
        // P7.S6 (R10): the REPL's `type:`-line readers reach this production
        // without the module pre-pass, so the postfix-form rejection is raised
        // here too rather than relied on from there.
        reject_postfix_header_var("type:", &name, self.tokens, self.pos)?;
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
        // P7.S6 (R1): `array[T N]` -- the named array type.
        if self.array_type_ahead() {
            self.pos += 1;
            return self.parse_array_type_expr();
        }
        // P7.S6 (R4): a bare `[` is a quotation effect unconditionally.
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            return self.parse_quotation_type_expr();
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^')) {
            return self.parse_owning_cell_type_expr();
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('&')) {
            return self.parse_ref_type_expr();
        }
        // P7.S3h: an `owning` field must *parse*, not be blamed on an unknown
        // type name: the containment rule that rejects it is
        // `audit_quotation_type_registries`, which can only see it once the
        // field resolves to a real `Type::OwningQuotation`.
        if self.owning_quotation_ahead() {
            return self.parse_owning_quotation_type_expr();
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
    /// `Semicolon` and ignores every other token, so a generic header's
    /// bracketed type-variable list in the scanned range doesn't change the
    /// verdict (it holds only `'`-prefixed words and its own brackets); the
    /// search need not skip past it first.
    fn current_typedef_is_enum(&self) -> bool {
        body_has_pipe_before_semicolon(self.tokens, self.pos + 2)
    }

    /// Lookahead (no consumption): whether the `type:` decl at the current
    /// position is generic (Phase 5 slice 1, R1/D2 / P7.S6 R5) -- its header
    /// binds one or more type variables in a bracket (`type: Box['T]`).
    /// `self.pos` must point at `type:`.
    fn current_typedef_is_generic(&self) -> bool {
        header_is_generic(self.tokens, self.pos + 2)
    }

    /// The enum `type:` production (D1, M3): `type: Name '|'? variant ('|'
    /// variant)* ;`, `variant := Word (field-name field-type)*`. The name and
    /// every variant name were already registered by the pre-pass; this
    /// parses and returns the ordered per-variant field list. Zero variants
    /// (an optional leading `|` with nothing after it, or a body with no
    /// variant at all) is a located malformed-declaration error (M3).
    fn parse_enum_typedef(&mut self) -> Result<Vec<Vec<(String, Type)>>, String> {
        let type_span = self.expect_word("type:")?;
        // The enum name is already registered by the pre-pass.
        let name = self.expect_word_any()?;
        // P7.S6 (R10): as in `parse_typedef` -- the REPL's enum `type:`-line
        // reader arrives here without the module pre-pass.
        reject_postfix_header_var("type:", &name, self.tokens, self.pos)?;
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
        len_vars: &[(String, Span)],
    ) -> Result<Vec<(String, PolyType)>, String> {
        let mut used = vec![false; ty_vars.len()];
        let mut used_len = vec![false; len_vars.len()];
        // P7b.S1 (S1-5/S1-9): a fresh per-decl kind side table -- ty var
        // ids are decl-local, so a table left over from a previous
        // declaration's fields would misattribute a stale established kind.
        self.field_kind_marks.clear();
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
                    let ty = self.parse_generic_field_type_expr(
                        name,
                        ty_vars,
                        &mut used,
                        len_vars,
                        &mut used_len,
                    )?;
                    fields.push((field_name, ty));
                }
                None => return Err(self.eof_error("`;` (unterminated `type:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        check_no_phantom_ty_var(name, ty_vars, &used)?;
        check_no_phantom_len_var(name, len_vars, &used_len)?;
        Ok(fields)
    }

    /// The enum twin of `parse_generic_typedef_fields` (D1, M3, R1): `'|'?
    /// variant ('|' variant)* ;`, resuming past an already-registered header.
    fn parse_generic_enum_typedef_variants(
        &mut self,
        name: &str,
        ty_vars: &[(String, Span)],
        len_vars: &[(String, Span)],
        type_span: Span,
    ) -> Result<Vec<crate::ast::GenericVariantDecl>, String> {
        if matches!(self.peek(), Some((Token::Pipe, _))) {
            self.pos += 1;
        }
        let mut used = vec![false; ty_vars.len()];
        let mut used_len = vec![false; len_vars.len()];
        // P7b.S1 (S1-5/S1-9): see `parse_generic_typedef_fields`'s own
        // reset -- shared across every variant of this one enum decl (the
        // header's ty vars are decl-wide, not per-variant).
        self.field_kind_marks.clear();
        let mut variants = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some((Token::Word(_), _)) => {
                    variants.push(self.parse_generic_variant_fields(
                        name,
                        ty_vars,
                        &mut used,
                        len_vars,
                        &mut used_len,
                    )?);
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
        check_no_phantom_len_var(name, len_vars, &used_len)?;
        Ok(variants)
    }

    /// P7.S3n (R2): a generic `type:`'s header alone -- `type: Name ('var)+`
    /// -- leaving the cursor at the first field/variant token. Everything a
    /// placeholder registration needs (name, bound variables, span) is known
    /// here; nothing a field list needs is missing.
    ///
    /// P7.S6a (R2a): `GenericHeader` now carries the header's type-variable
    /// and length-variable lists separately -- `parse_header_bracket`
    /// already tags each entry with its `Kind`; this is where that per-entry
    /// kind gets split into the two lists every downstream field/variant
    /// parser consumes.
    fn parse_generic_header(&mut self) -> Result<GenericHeader, String> {
        let type_span = self.expect_word("type:")?;
        let (name, _) = self.expect_word_any_spanned()?;
        // `header_is_generic` gates every route here, so the `[` is present.
        let vars = self.parse_header_bracket(&name)?;
        // P7b.S1 (R1): an `Arrow`-kinded variable is still a *type* variable
        // (`Kind`'s own doc comment), so it joins `ty_vars`/`ty_kinds`
        // alongside `Star`, never `len_vars`.
        let ty_vars = vars
            .iter()
            .filter(|(_, _, k)| !matches!(k, Kind::Len))
            .map(|(n, s, _)| (n.clone(), *s))
            .collect();
        let ty_kinds = vars
            .iter()
            .filter(|(_, _, k)| !matches!(k, Kind::Len))
            .map(|(_, _, k)| k.clone())
            .collect();
        let len_vars = vars
            .iter()
            .filter(|(_, _, k)| matches!(k, Kind::Len))
            .map(|(n, s, _)| (n.clone(), *s))
            .collect();
        Ok((name, ty_vars, ty_kinds, len_vars, type_span))
    }

    /// One generic variant's field list, mirroring `parse_variant_fields`
    /// with fields resolved through `parse_generic_field_type_expr` instead.
    /// The reserved-name gate runs here rather than in a pre-pass: the
    /// module-level pre-pass skips every generic header entirely (its
    /// variant names are only ever seen by this parser), so this is the one
    /// site that can reject a generic variant named `^Evil`.
    #[allow(clippy::too_many_arguments)]
    fn parse_generic_variant_fields(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
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
                    let ty = self.parse_generic_field_type_expr(
                        decl_name, ty_vars, used, len_vars, used_len,
                    )?;
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
                    let ty = self.parse_generic_field_type_expr(
                        decl_name, ty_vars, used, len_vars, used_len,
                    )?;
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
                && header_is_generic(self.tokens, i + 2)
            {
                self.pos = i;
                if self.generic_header_at_cursor_is_registered(already) {
                    self.skip_typedef();
                } else {
                    let is_enum = self.current_typedef_is_enum();
                    let (name, ty_vars, ty_kinds, len_vars, type_span) =
                        self.parse_generic_header()?;
                    let ty_var_names = ty_vars.iter().map(|(n, _)| n.clone()).collect();
                    let len_var_names = len_vars.iter().map(|(n, _)| n.clone()).collect();
                    let idx = if is_enum {
                        self.generics
                            .push_enum_placeholder(crate::ast::GenericEnumDecl {
                                name: name.clone(),
                                ty_var_names,
                                ty_kinds: ty_kinds.clone(),
                                len_var_names,
                                variants: Vec::new(),
                                span: type_span,
                                module: self.module,
                            })
                    } else {
                        self.generics
                            .push_struct_placeholder(crate::ast::GenericStructDecl {
                                name: name.clone(),
                                ty_var_names,
                                ty_kinds: ty_kinds.clone(),
                                len_var_names,
                                fields: Vec::new(),
                                span: type_span,
                                module: self.module,
                            })
                    };
                    headers.push((
                        is_enum,
                        idx,
                        self.pos,
                        (name, ty_vars, ty_kinds, len_vars, type_span),
                    ));
                    self.skip_typedef();
                }
                i = self.pos;
                continue;
            }
            i += 1;
        }
        for (is_enum, idx, pos, (name, ty_vars, _ty_kinds, len_vars, type_span)) in headers {
            self.pos = pos;
            if is_enum {
                let variants = self
                    .parse_generic_enum_typedef_variants(&name, &ty_vars, &len_vars, type_span)?;
                // Disjoint field borrows: `regs` borrows the concrete
                // registries, `generics` is a separate field.
                let regs = MutRegistries {
                    structs: self.structs,
                    enums: self.enums,
                    arrays: self.arrays,
                    cells: self.owned_cells,
                    refs: self.refs,
                };
                self.generics.fill_enum_variants(idx, variants, regs);
                self.publish_field_inferred_kinds(true, idx);
            } else {
                let fields = self.parse_generic_typedef_fields(&name, &ty_vars, &len_vars)?;
                let regs = MutRegistries {
                    structs: self.structs,
                    enums: self.enums,
                    arrays: self.arrays,
                    cells: self.owned_cells,
                    refs: self.refs,
                };
                self.generics.fill_struct_fields(idx, fields, regs);
                self.publish_field_inferred_kinds(false, idx);
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

    /// Advance past a whole `type:` declaration without parsing it.
    ///
    /// An unterminated one needs no error here, but for two different reasons
    /// depending on the caller. `parse_bodies` and the `trait:`/duplicate-header
    /// arms skip declarations a prior pass has *already* parsed and would
    /// already have rejected. R2's stage (a), by contrast, skips ahead of any
    /// field list having been parsed at all -- there the rejection is still
    /// pending, and comes from stage (b) revisiting the recorded position and
    /// parsing that list for real.
    fn skip_typedef(&mut self) {
        // P7.S3s-follow: a `trait:` declaration's members each carry their own
        // `;` terminator, so the first `;` at depth 0 is a member's, not the
        // trait's. The trait's terminating `;` is the one at depth 0 that is
        // NOT preceded by `)` (every member `;` is). A `type:` declaration
        // still has a single `;`, so the original fast scan applies.
        if matches!(self.tokens.get(self.pos), Some((Token::Word(w), _)) if w == "trait:") {
            let mut depth = 0i32;
            let mut prev_was_rparen = false;
            while let Some((tok, _)) = self.tokens.get(self.pos) {
                match tok {
                    Token::LParen => {
                        depth += 1;
                        prev_was_rparen = false;
                        self.pos += 1;
                    }
                    Token::RParen => {
                        depth -= 1;
                        prev_was_rparen = depth == 0;
                        self.pos += 1;
                    }
                    Token::Semicolon if depth == 0 => {
                        self.pos += 1;
                        if !prev_was_rparen {
                            break;
                        }
                        prev_was_rparen = false;
                    }
                    _ => {
                        prev_was_rparen = false;
                        self.pos += 1;
                    }
                }
            }
        } else {
            while let Some((tok, _)) = self.tokens.get(self.pos) {
                let terminator = matches!(tok, Token::Semicolon);
                self.pos += 1;
                if terminator {
                    break;
                }
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
            let (args, _lens) = self.parse_type_arguments(name, 1, 0, span)?;
            let mutable = name == MUT_SLICE_TYPE_NAME;
            return Ok(crate::ast::intern_slice_type(self.slices, args[0], mutable));
        }
        // P7.S6 (R2): `array` is the named array type's spelling. When
        // followed by `[` it is intercepted at the bracket-dispatch sites
        // (R1) or at the `&`/`^` splitters (R1a) and never reaches here. The
        // only way `array` arrives at this funnel is without a following `[`,
        // which is a located error naming the required form -- not "unknown
        // type `array`".
        if name == ARRAY_TYPE_NAME {
            return Err(array_without_bracket_error(span));
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
            let ty_var_names = self.generics.structs[idx].ty_var_names.clone();
            let ty_kinds = self.generics.structs[idx].ty_kinds.clone();
            let ty_arity = ty_var_names.len();
            let len_arity = self.generics.structs[idx].len_var_names.len();
            let (args, lens) = self.parse_type_arguments(name, ty_arity, len_arity, span)?;
            // S1-15.f: a use-site argument's `CtorImage`-ness must agree with
            // the header variable's own declared kind -- a plain type where a
            // constructor was declared (or the reverse) is a located error,
            // not a silent (wrong) bind at instantiation.
            validate_ctor_arg_kinds(name, &ty_var_names, &ty_kinds, &args, span)?;
            let regs = MutRegistries {
                structs: self.structs,
                enums: self.enums,
                arrays: self.arrays,
                cells: self.owned_cells,
                refs: self.refs,
            };
            return Ok(self
                .generics
                .instantiate_struct(idx, &args, &lens, self.module, regs));
        }
        if let Some(idx) = self.generics.find_enum(base, owner) {
            let ty_var_names = self.generics.enums[idx].ty_var_names.clone();
            let ty_kinds = self.generics.enums[idx].ty_kinds.clone();
            let ty_arity = ty_var_names.len();
            let len_arity = self.generics.enums[idx].len_var_names.len();
            let (args, lens) = self.parse_type_arguments(name, ty_arity, len_arity, span)?;
            validate_ctor_arg_kinds(name, &ty_var_names, &ty_kinds, &args, span)?;
            let regs = MutRegistries {
                structs: self.structs,
                enums: self.enums,
                arrays: self.arrays,
                cells: self.owned_cells,
                refs: self.refs,
            };
            return Ok(self
                .generics
                .instantiate_enum(idx, &args, &lens, self.module, regs));
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
    /// `Wrap[Box[i64]]` and `Buf[array[i64 4]]` fall out of the recursion).
    ///
    /// Bracketed rather than juxtaposed (`Box i64`) because R3's
    /// argument-count error has to be *decidable*: juxtaposed, a signature
    /// slot list `( Box i64 bool -- )` reads identically as an over-applied
    /// `Box` and as a correctly applied one beside a `bool` slot, so an extra
    /// argument could never be diagnosed there. Brackets also match how
    /// ROADMAP.md spells a use site (`Option['T]`, `Map['K 'V]`), and `[` is
    /// already the type sublanguage's own delimiter.
    ///
    /// P7.S6a (R6): widened to split the bracket into `0..ty_arity` type
    /// expressions followed by `ty_arity..ty_arity+len_arity` length
    /// literals -- the concrete-use-site twin of `parse_generic_field_
    /// application`'s own split (that one also accepts a `'`-prefixed length
    /// variable, since it parses inside another header's own field list; a
    /// concrete use site never has a length variable in scope, so this one
    /// only ever reads a literal).
    /// P7b.S1 (S1-8/S1-12): one use-site type argument. A bare word naming
    /// a generic `type:` header, *not* itself followed by `[`, is a
    /// constructor image (`Wrap[Box i64]`'s `Box`) rather than an
    /// under-applied header -- `resolve_type_or_apply` would otherwise
    /// require it to be applied here and report an arity error. Anything
    /// else (a concrete type, or a header immediately applied, `Wrap[Box[i64]
    /// i64]`) parses exactly as before.
    fn parse_type_argument(&mut self) -> Result<Type, String> {
        if let Some((Token::Word(w), span)) = self.peek() {
            if !w.starts_with(['\'', '&', '^']) {
                let (w, span) = (w.clone(), *span);
                if !matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _))) {
                    if let Some((is_enum, idx, module)) = self.poly_generic_header(&w, span)? {
                        self.pos += 1;
                        let gid = crate::ast::GenericId {
                            is_enum,
                            idx: idx as u32,
                            module,
                        };
                        return Ok(crate::ast::ctor_image_type(self.generics, gid));
                    }
                }
            }
        }
        self.parse_type_expr()
    }

    fn parse_type_arguments(
        &mut self,
        name: &str,
        ty_arity: usize,
        len_arity: usize,
        span: Span,
    ) -> Result<(Vec<Type>, Vec<Len>), String> {
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            return Err(generic_arity_error(name, ty_arity, len_arity, 0, 0, span));
        }
        self.pos += 1;
        let mut args = Vec::new();
        let mut lens = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated generic type application)"));
                }
                _ => {}
            }
            if args.len() < ty_arity {
                args.push(self.parse_type_argument()?);
                continue;
            }
            if lens.len() < len_arity {
                lens.push(Len::Concrete(self.parse_array_count(name)?));
                continue;
            }
            // Over-application, beyond both declared arities: consume
            // permissively (as a type expression) so the arity check below
            // reports the real supplied count instead of a misleading
            // length-literal error.
            args.push(self.parse_type_argument()?);
        }
        if args.len() != ty_arity || lens.len() != len_arity {
            return Err(generic_arity_error(
                name,
                ty_arity,
                len_arity,
                args.len(),
                lens.len(),
                span,
            ));
        }
        Ok((args, lens))
    }

    /// P7.S6 (R5), widened P7.S6a (R2): parse a bracketed variable list
    /// `['T 'N: Len]` from a `type:`/`trait:` header. Assumes `[` is the
    /// current token; consumes through the matching `]`. Each entry is a
    /// bare `'`-prefixed word (kind `Star`, the unannotated common case) or
    /// one annotated `: Len` (colon glued or spaced, mirroring a word
    /// bound's `'T: Copy`); no other kind is spellable this slice. An empty
    /// bracket, a duplicate same-kind variable, a name bound as both kinds, a
    /// non-`'`/non-`]` token, an unknown kind annotation, or (R2.1) a
    /// non-empty bracket binding zero type variables are located errors.
    fn parse_header_bracket(
        &mut self,
        decl_name: &str,
    ) -> Result<Vec<(String, Span, Kind)>, String> {
        let bracket_span = self.peek().map(|(_, s)| *s).unwrap_or_default();
        self.pos += 1; // consume `[`
        let mut vars: Vec<(String, Span, Kind)> = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                Some((Token::Word(w), span)) if w.starts_with('\'') => {
                    let glued_colon = w.ends_with(':') && w.len() > 1;
                    let (name, span) = if glued_colon {
                        (w[..w.len() - 1].to_string(), *span)
                    } else {
                        let (nw, ns) = self.expect_word_any_spanned()?;
                        (nw, ns)
                    };
                    if glued_colon {
                        self.pos += 1;
                    }
                    let colon_follows = if glued_colon {
                        true
                    } else {
                        matches!(self.peek(), Some((Token::Word(c), _)) if c == ":")
                    };
                    if colon_follows && !glued_colon {
                        self.pos += 1;
                    }
                    let kind = if colon_follows {
                        parse_kind_expr(self, header_bracket_unknown_kind_error)?
                    } else {
                        Kind::Star
                    };
                    if let Some((_, _, existing_kind)) = vars.iter().find(|(n, _, _)| *n == name) {
                        if *existing_kind != kind {
                            return Err(header_var_kind_conflict_error(&name, decl_name, span));
                        }
                        return Err(duplicate_generic_ty_var_error(&name, decl_name, span));
                    }
                    vars.push((name, span, kind));
                }
                Some((tok, span)) => {
                    return Err(header_bracket_non_var_error(decl_name, tok, *span));
                }
                None => return Err(self.eof_error("`]` (unterminated type-variable list)")),
            }
        }
        if vars.is_empty() {
            return Err(empty_header_bracket_error(decl_name, bracket_span));
        }
        // P7b.S1 (R1): an `Arrow`-kinded variable is still a *type*
        // variable (`Kind`'s own doc comment), so it satisfies this check
        // exactly as `Star` does; only an all-`Len` bracket is rejected.
        if !vars.iter().any(|(_, _, k)| !matches!(k, Kind::Len)) {
            return Err(header_bracket_no_type_variable_error(
                decl_name,
                bracket_span,
            ));
        }
        Ok(vars)
    }

    /// A generic `type:` field's type (R1): a recursive descent over the
    /// shapes that can wrap one of the header's bound variables -- array
    /// (`array['T 2]`, nested to any depth), reference (`&'T`, `&!'T`), owning
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
    #[allow(clippy::too_many_arguments)]
    fn parse_generic_field_type_expr(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
    ) -> Result<PolyType, String> {
        let span = self.peek().map(|(_, s)| *s).unwrap_or_default();
        let pty = self.parse_generic_field_shape(decl_name, ty_vars, used, len_vars, used_len)?;
        reject_growing_generic_argument(decl_name, &pty, span)?;
        Ok(pty)
    }

    /// R1's descent proper, split from `parse_generic_field_type_expr` so
    /// R8's whole-tree growth check runs once per *field* rather than once
    /// per node the recursion visits.
    #[allow(clippy::too_many_arguments)]
    fn parse_generic_field_shape(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
    ) -> Result<PolyType, String> {
        if let Some((Token::TildeLBracket, span)) = self.peek() {
            return Err(tilde_quotation_position_error(*span));
        }
        // P7.S6 (R1): `array[T N]` -- the named array type.
        if self.array_type_ahead() {
            self.pos += 1;
            return self.parse_generic_field_array(decl_name, ty_vars, used, len_vars, used_len);
        }
        // P7.S6 (R4): a bare `[` opens a quotation effect unconditionally;
        // an array field is spelled `array['T N]` and was taken above.
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            // R7: a quotation field naming the declaration's own type
            // variable is out of scope, rejected here rather than left
            // to misreport `'T` as an unknown concrete type. A quotation
            // field over concrete types alone still parses. This scan runs
            // *ahead* of the reader, so it also fires ahead of R4a's
            // missing-`--` validator on a variable-bearing bracket.
            if let Some((var, span)) = self.quotation_effect_ty_var_ahead(ty_vars) {
                return Err(quotation_field_ty_var_error(decl_name, &var, span));
            }
            return Ok(PolyType::Concrete(self.parse_quotation_type_expr()?));
        }
        if let Some((Token::Word(w), span)) = self.peek() {
            let (w, span) = (w.clone(), *span);
            if w.starts_with('\'') {
                self.pos += 1;
                let head = self.resolve_field_ty_var(decl_name, ty_vars, used, &w, span)?;
                // P7b.S1 (S1-8): the variable arm's own bracket continuation
                // -- unlike the `&`/`^` arms, which have always had one for a
                // named generic header. A field `f 'F['T]` applies the
                // header's own type variable to its argument list.
                if matches!(self.peek(), Some((Token::LBracket, _))) {
                    let applied = self.parse_generic_field_var_application(
                        decl_name, ty_vars, used, len_vars, used_len, head, span,
                    )?;
                    let PolyType::App { args, .. } = &applied else {
                        unreachable!("parse_generic_field_var_application always returns App")
                    };
                    self.check_field_arrow_kind(decl_name, head, &w, span, args.len())?;
                    return Ok(applied);
                }
                self.check_field_bare_kind(decl_name, head, &w, span)?;
                return Ok(PolyType::Var(head));
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
                    let inner = self
                        .parse_generic_field_shape(decl_name, ty_vars, used, len_vars, used_len)?;
                    return Ok(self.fold_field_ref(inner, mutable));
                }
                let remainder = remainder.to_string();
                let remainder_span = Span {
                    col: span.col + sigil_len as u32,
                    ..span
                };
                if remainder.starts_with('\'') {
                    self.pos += 1;
                    let id = self.resolve_field_ty_var(
                        decl_name,
                        ty_vars,
                        used,
                        &remainder,
                        remainder_span,
                    )?;
                    self.check_field_bare_kind(decl_name, id, &remainder, remainder_span)?;
                    return Ok(self.fold_field_ref(PolyType::Var(id), mutable));
                }
                // P7.S6 (R1a): `&array['T 4]` -- the `&` and `array` are
                // glued into one word, so the `[`-dispatch sites cannot
                // reach this spelling. Intercept `array` ahead of
                // `poly_generic_header`, which looks it up in the user
                // registries and would misreport "unknown type `array`".
                // Placed before the `poly_generic_header` case, since `array`
                // must be recognised ahead of the user registry exactly as
                // `resolve_type_or_apply` recognises `Slice`.
                if remainder == ARRAY_TYPE_NAME
                    && matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _)))
                {
                    self.pos += 1;
                    let inner = self
                        .parse_generic_field_array(decl_name, ty_vars, used, len_vars, used_len)?;
                    return Ok(self.fold_field_ref(inner, mutable));
                }
                // A run glued to a generic header that is then applied
                // (`&Ent['K i64]`), mirroring the `^` arm below. Without it
                // only the *spaced* `& Ent['K i64]` reaches the application
                // production, and the glued spelling falls through to the
                // concrete parser -- which blames `'K` as an unknown type,
                // the exact misreport this whole arm exists to prevent.
                if matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _))) {
                    if let Some((is_enum, idx, module)) =
                        self.poly_generic_header(&remainder, remainder_span)?
                    {
                        self.pos += 1;
                        let inner = self.parse_generic_field_application(
                            decl_name,
                            ty_vars,
                            used,
                            len_vars,
                            used_len,
                            &remainder,
                            is_enum,
                            idx,
                            module,
                            remainder_span,
                        )?;
                        return Ok(self.fold_field_ref(inner, mutable));
                    }
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
                let inner =
                    if remainder.is_empty() {
                        self.pos += 1;
                        if matches!(self.peek(), Some((Token::Semicolon | Token::Pipe, _)))
                            || self.peek().is_none()
                        {
                            return Err(owned_cell_no_payload_error(&w, span));
                        }
                        Some(self.parse_generic_field_shape(
                            decl_name, ty_vars, used, len_vars, used_len,
                        )?)
                    } else if remainder.starts_with('\'') {
                        self.pos += 1;
                        let id = self.resolve_field_ty_var(
                            decl_name,
                            ty_vars,
                            used,
                            &remainder,
                            remainder_span,
                        )?;
                        self.check_field_bare_kind(decl_name, id, &remainder, remainder_span)?;
                        Some(PolyType::Var(id))
                    } else if remainder == ARRAY_TYPE_NAME
                        && matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _)))
                    {
                        // P7.S6 (R1a): `^array['T 4]` -- same interception as
                        // the `&` arm above, dispatching into the generic-field
                        // array reader.
                        self.pos += 1;
                        Some(self.parse_generic_field_array(
                            decl_name, ty_vars, used, len_vars, used_len,
                        )?)
                    } else if matches!(self.tokens.get(self.pos + 1), Some((Token::LBracket, _))) {
                        match self.poly_generic_header(&remainder, remainder_span)? {
                            Some((is_enum, idx, module)) => {
                                self.pos += 1;
                                Some(self.parse_generic_field_application(
                                    decl_name,
                                    ty_vars,
                                    used,
                                    len_vars,
                                    used_len,
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
                        decl_name, ty_vars, used, len_vars, used_len, &w, is_enum, idx, module,
                        span,
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

    /// P7b.S1 (R2/S1-5/S1-9, Phase 3): a *bare* field mention (no bracket
    /// continuation) -- the header-field twin of `PolyBuilder::mark_ty_star`.
    /// First field-usage mention of this ty var (within the declaration
    /// currently being parsed) establishes `Star`; a later bare mention is
    /// consistent; a var already established `Arrow` by an earlier applied
    /// field mention is the header-field twin of S1-15.b. An *explicit*
    /// `Arrow` header annotation (never a default -- `parse_header_bracket`
    /// only ever defaults to `Star`) is authoritative and pre-empts
    /// inference, so a bare mention against an explicitly higher-kinded
    /// header variable is flagged even on its very first field mention.
    fn check_field_bare_kind(
        &mut self,
        decl_name: &str,
        idx: u32,
        name: &str,
        span: Span,
    ) -> Result<(), String> {
        if let Some(arrow @ Kind::Arrow { .. }) =
            self.header_ty_var_kind(decl_name, idx as usize).cloned()
        {
            return Err(header_field_kind_conflict_error(
                decl_name,
                name,
                span,
                &kind_str(&arrow),
            ));
        }
        match self.field_kind_marks.get(&idx).cloned() {
            None => {
                self.field_kind_marks.insert(idx, (Kind::Star, span));
                Ok(())
            }
            Some((Kind::Star, _)) => Ok(()),
            Some((arrow, _first_span)) => Err(header_field_kind_conflict_error(
                decl_name,
                name,
                span,
                &kind_str(&arrow),
            )),
        }
    }

    /// P7b.S1 (S1-8/S1-15.e, Phase 3): an *applied* field mention (`f
    /// 'F['T]`) -- the header-field twin of `PolyBuilder::mark_ty_arrow`.
    /// First field-usage mention establishes `Arrow { domains: [Star;
    /// domain_count], .. }`, `domain_count` being the application's real
    /// argument count (review fix: this used to hardcode an empty domain
    /// list regardless of arity, so `kind_str` rendered the established
    /// kind as `*` -- self-contradictory in `header_field_kind_conflict_
    /// error`'s message, and a dishonest arity in `publish_field_inferred_
    /// kinds`' published vector). A later applied mention is consistent
    /// regardless of its own arity (no golden requires distinguishing it,
    /// and the header-field path carries no per-mention arity table -- only
    /// *whether* the var is Arrow-kinded, same as before this fix); a var
    /// already established `Star` by an earlier bare field mention is the
    /// header-field twin of S1-15.a.
    fn check_field_arrow_kind(
        &mut self,
        decl_name: &str,
        idx: u32,
        name: &str,
        span: Span,
        domain_count: usize,
    ) -> Result<(), String> {
        match self.field_kind_marks.get(&idx).cloned() {
            None => {
                self.field_kind_marks.insert(
                    idx,
                    (
                        Kind::Arrow {
                            domains: vec![Kind::Star; domain_count],
                            result: Box::new(Kind::Star),
                        },
                        span,
                    ),
                );
                Ok(())
            }
            Some((Kind::Arrow { .. }, _)) => Ok(()),
            Some((Kind::Star, _first_span)) => Err(header_field_applies_star_var_error(
                decl_name, name, span, "*",
            )),
            Some((Kind::Len, _)) => {
                unreachable!("a header ty var never carries Len: X1 already rejects that")
            }
        }
    }

    /// P7b.S1 (S1-5/S1-9, Phase 3): publish this declaration's field-usage-
    /// inferred kinds (`field_kind_marks`, collected while its fields/
    /// variants were just parsed) into the header's own `ty_kinds` -- an
    /// unannotated header var's kind is otherwise stuck at
    /// `parse_header_bracket`'s `Star` default forever, which is wrong the
    /// moment a field actually applies it (S1-9: usage infers a kind, an
    /// annotation is only the fallback). A var the header *explicitly*
    /// annotated `Arrow` is left alone -- `check_field_bare_kind`/
    /// `check_field_arrow_kind` already enforce that every field mention
    /// agrees with it, so overwriting would only ever be a no-op there.
    fn publish_field_inferred_kinds(&mut self, is_enum: bool, idx: usize) {
        for (var_idx, (kind, _span)) in self.field_kind_marks.drain() {
            let ty_kinds = if is_enum {
                &mut self.generics.enums[idx].ty_kinds
            } else {
                &mut self.generics.structs[idx].ty_kinds
            };
            if let Some(slot) = ty_kinds.get_mut(var_idx as usize) {
                if !matches!(slot, Kind::Arrow { .. }) {
                    *slot = kind;
                }
            }
        }
    }

    /// P7b.S1 (R5): the declared kind of ty-var `idx` in `decl_name`'s own
    /// header (struct or enum, whichever this parser's `self.generics`
    /// already has a placeholder for at `self.module` -- registered by
    /// stage (a) of `parse_generic_typedefs` before any field parses).
    /// `None` when the header itself cannot be found (a unit test calling
    /// this parser with no registry), in which case the kind conflict check
    /// is simply skipped.
    fn header_ty_var_kind(&self, decl_name: &str, idx: usize) -> Option<&Kind> {
        if let Some(sidx) = self.generics.find_struct(decl_name, self.module) {
            return self.generics.structs[sidx].ty_kinds.get(idx);
        }
        if let Some(eidx) = self.generics.find_enum(decl_name, self.module) {
            return self.generics.enums[eidx].ty_kinds.get(idx);
        }
        None
    }

    /// P7.S6a (R2a): the length-path twin of `resolve_field_ty_var`. Unlike a
    /// word signature's `intern_len_var` (which *mints* an id on first
    /// sight), a header field only *resolves*: every length variable was
    /// already bound by the header's own bracket (R2), so an unresolvable
    /// `'N` is a located error rather than a fresh interning.
    fn resolve_field_len_var(
        &self,
        decl_name: &str,
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
        name: &str,
        span: Span,
    ) -> Result<u32, String> {
        let idx = len_vars
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| unbound_generic_len_var_error(name, decl_name, span))?;
        used_len[idx] = true;
        Ok(idx as u32)
    }

    /// A generic field's array type `[ elem count ]`, `elem` recursing so a
    /// nested `array[array['T 2] 2]` falls out. P7.S6a (R2a): the count is
    /// either a `'`-prefixed token resolved against the header's own
    /// length-variable list (`Len::Var`), mirroring `parse_poly_array`'s
    /// existing `'N` arm, or a decimal literal read through
    /// `parse_array_count` exactly as before (`Len::Concrete`).
    fn parse_generic_field_array(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
    ) -> Result<PolyType, String> {
        self.expect(Token::LBracket)?;
        let elem = self.parse_generic_field_shape(decl_name, ty_vars, used, len_vars, used_len)?;
        let len = if let Some((Token::Word(w), span)) = self.peek() {
            if w.starts_with('\'') {
                let (w, span) = (w.clone(), *span);
                self.pos += 1;
                Len::Var(self.resolve_field_len_var(decl_name, len_vars, used_len, &w, span)?)
            } else {
                Len::Concrete(self.parse_generic_field_array_count(&elem, ty_vars, len_vars)?)
            }
        } else {
            Len::Concrete(self.parse_generic_field_array_count(&elem, ty_vars, len_vars)?)
        };
        self.expect(Token::RBracket)?;
        Ok(match (elem, len) {
            (PolyType::Concrete(t), Len::Concrete(count)) => {
                PolyType::Concrete(crate::ast::intern_array_type(self.arrays, t, count))
            }
            (elem, len) => PolyType::Array(Box::new(elem), len),
        })
    }

    /// `parse_generic_field_array`'s literal-count read, split out so both
    /// its `'`-prefixed and literal branches share the same
    /// `parse_array_count` call. `parse_array_count`'s linear-element
    /// rejection needs a concrete element type; over a variable element
    /// there is none to give it, so the count is read as a bare literal and
    /// the element's linearity is left to the checker, exactly as it is for
    /// a concrete array field.
    fn parse_generic_field_array_count(
        &mut self,
        elem: &PolyType,
        ty_vars: &[(String, Span)],
        len_vars: &[(String, Span)],
    ) -> Result<u32, String> {
        match elem {
            PolyType::Concrete(t) => self.parse_array_count(t.name()),
            elem => self.parse_array_count(&generic_field_type_str(elem, ty_vars, len_vars)),
        }
    }

    /// A generic field's generic-type application, each argument a field type
    /// rather than a concrete type expression -- the field-parser twin of
    /// `parse_type_arguments`, reusing only its arity check. A fully-concrete
    /// argument list instantiates immediately and folds to `Concrete`,
    /// byte-for-byte as `resolve_type_or_apply` already does; otherwise the
    /// application stays `PolyType::Generic` for substitution to ground.
    ///
    /// P7.S6a (R2a, phase 3): the argument list splits into `0..ty_arity`
    /// field-type arguments (parsed exactly as before) and
    /// `ty_arity..ty_arity+len_arity` length arguments -- a `'`-prefixed
    /// token resolved against *this* header's own length-variable list
    /// (`Len::Var`, `resolve_field_len_var`), or a decimal literal
    /// (`Len::Concrete`), mirroring `parse_generic_field_array`'s own count
    /// reader one level down. The eager concrete-collapse gates on every
    /// length argument being `Len::Concrete` too, not just every type
    /// argument, so a variable type paired with a concrete length
    /// (`Buffer['T 4]`) and a concrete type paired with a variable length
    /// stay `PolyType::Generic` alike.
    /// P7b.S1 (S1-8): a header/field application (`f 'F['T]`), the field-
    /// grammar twin of `parse_poly_var_application` -- no known header to
    /// read an arity from, so every argument parses as a field-shape type
    /// expression with no arity bound. A bare `[` argument (a quotation
    /// shape) is fenced exactly as the effect-grammar production fences it
    /// (S1-6), and an empty application is the same pinned arity error.
    #[allow(clippy::too_many_arguments)]
    fn parse_generic_field_var_application(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
        head: u32,
        span: Span,
    ) -> Result<PolyType, String> {
        self.pos += 1; // consume `[`
        let mut args = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated type application)"));
                }
                Some((Token::LBracket | Token::TildeLBracket, arg_span)) => {
                    return Err(app_arg_quotation_error(*arg_span));
                }
                _ => args.push(
                    self.parse_generic_field_shape(decl_name, ty_vars, used, len_vars, used_len)?,
                ),
            }
        }
        if args.is_empty() {
            let var = &ty_vars[head as usize].0;
            return Err(empty_type_application_error(var, span));
        }
        Ok(PolyType::App { head, args })
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_generic_field_application(
        &mut self,
        decl_name: &str,
        ty_vars: &[(String, Span)],
        used: &mut [bool],
        len_vars: &[(String, Span)],
        used_len: &mut [bool],
        name: &str,
        is_enum: bool,
        idx: usize,
        module: u32,
        span: Span,
    ) -> Result<PolyType, String> {
        let (ty_arity, len_arity) = if is_enum {
            (
                self.generics.enums[idx].ty_var_names.len(),
                self.generics.enums[idx].len_var_names.len(),
            )
        } else {
            (
                self.generics.structs[idx].ty_var_names.len(),
                self.generics.structs[idx].len_var_names.len(),
            )
        };
        if !matches!(self.peek(), Some((Token::LBracket, _))) {
            return Err(generic_arity_error(name, ty_arity, len_arity, 0, 0, span));
        }
        self.pos += 1;
        let mut args = Vec::new();
        let mut lens: Vec<Len> = Vec::new();
        loop {
            match self.peek() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => {
                    return Err(self.eof_error("`]` (unterminated generic type application)"));
                }
                _ => {}
            }
            if args.len() < ty_arity {
                args.push(
                    self.parse_generic_field_shape(decl_name, ty_vars, used, len_vars, used_len)?,
                );
                continue;
            }
            if let Some((Token::Word(w), wspan)) = self.peek() {
                if w.starts_with('\'') {
                    let (w, wspan) = (w.clone(), *wspan);
                    self.pos += 1;
                    lens.push(Len::Var(self.resolve_field_len_var(
                        decl_name, len_vars, used_len, &w, wspan,
                    )?));
                    continue;
                }
            }
            lens.push(Len::Concrete(self.parse_array_count(name)?));
        }
        if args.len() != ty_arity || lens.len() != len_arity {
            return Err(generic_arity_error(
                name,
                ty_arity,
                len_arity,
                args.len(),
                lens.len(),
                span,
            ));
        }
        let concrete: Option<Vec<Type>> = args
            .iter()
            .map(|a| match a {
                PolyType::Concrete(t) => Some(*t),
                _ => None,
            })
            .collect();
        let concrete_lens: Option<Vec<Len>> = lens
            .iter()
            .all(|l| matches!(l, Len::Concrete(_)))
            .then(|| lens.clone());
        if let (Some(concrete), Some(concrete_lens)) = (concrete, concrete_lens) {
            let regs = MutRegistries {
                structs: self.structs,
                enums: self.enums,
                arrays: self.arrays,
                cells: self.owned_cells,
                refs: self.refs,
            };
            return Ok(PolyType::Concrete(if is_enum {
                self.generics
                    .instantiate_enum(idx, &concrete, &concrete_lens, module, regs)
            } else {
                self.generics
                    .instantiate_struct(idx, &concrete, &concrete_lens, module, regs)
            }));
        }
        Ok(PolyType::Generic {
            is_enum,
            idx: idx as u32,
            module,
            args,
            len_args: lens,
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
                    //
                    // The peel loops: sigils stack (`^^'T`, a shape R1 admits
                    // and builds nested cells for), so stripping once leaves
                    // `^'T`, which matches no `ty_vars` entry and reopens the
                    // very misreport above.
                    let mut bare = w.as_str();
                    while let Some(rest) = bare
                        .strip_prefix('^')
                        .or_else(|| bare.strip_prefix("&!"))
                        .or_else(|| bare.strip_prefix('&'))
                    {
                        bare = rest;
                    }
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

    /// P7.S3t (R2): the explicit type-argument list of a call, `f[Point]`,
    /// read at `parse_term`'s `Word` arm with the word already consumed.
    /// Empty for every call not followed by a *glued* `[`.
    ///
    /// Adjacency is decided here, from the two token spans, and the lexer is
    /// untouched: gluing `Word`+`[` into one token there would fire in type
    /// position too, where `Box['T]`, `Slice[T]` and `&![ i64 -- ]` are all an
    /// adjacent word-then-bracket today, and every type reader would have to
    /// learn the new token. `col` is incremented once per char over the whole
    /// word scan and the word text is exactly the chars consumed, so the
    /// arithmetic is exact, not a heuristic.
    ///
    /// This narrows the grammar: `foo[Point]` used to parse as a call followed
    /// by a quotation literal, identically to `foo [Point]`. The spaced
    /// spelling still does; the glued one is now an instantiation. The element
    /// loop mirrors `parse_type_arguments` but is not it -- that function
    /// checks against a type constructor's known arity, and a call site's
    /// arity is a property of the callee's `PolySig`, which the parser does
    /// not have (R4 checks it in the checker).
    fn parse_explicit_type_args(
        &mut self,
        word: &str,
        span: Span,
    ) -> Result<(Vec<Type>, Vec<Len>), String> {
        let glued = matches!(
            self.peek(),
            Some((Token::LBracket, b))
                if b.line == span.line && b.col == span.col + word.chars().count() as u32
        );
        if !glued {
            return Ok((Vec::new(), Vec::new()));
        }
        self.pos += 1;
        // R7 (review fix): a variable anywhere in the list -- top-level or
        // nested inside a generic application/array/ref (`Cell['U]`) -- is
        // found here, before `parse_type_expr` ever runs. `parse_type_expr`
        // has no production for a variable at any depth, so without this a
        // nested one surfaces as its "unknown type" message, with the
        // first-element note wrongly attached on top.
        if let Some((var, vspan)) = self.instantiation_list_ty_var() {
            return Err(instantiation_ty_var_error(&var, vspan));
        }
        let mut ty_args: Vec<Type> = Vec::new();
        let mut len_args: Vec<Len> = Vec::new();
        // P7.S6b (R1): a bare decimal integer token is never a type
        // expression (types are word-shaped: `i64`, `Box[...]`), so the
        // token stream itself decides where the type sublist ends and the
        // length sublist begins. Once an integer token is seen, a type
        // token after it is a parse error rather than an implicit switch
        // back.
        let mut in_len_mode = false;
        loop {
            match self.peek().cloned() {
                Some((Token::RBracket, _)) => {
                    self.pos += 1;
                    break;
                }
                None => return Err(unterminated_instantiation_error(word, span)),
                Some((Token::Int(_), _)) => {
                    in_len_mode = true;
                    len_args.push(self.parse_call_len_arg(word, span)?);
                }
                Some((tok, tspan)) if in_len_mode => {
                    return Err(instantiation_type_after_len_error(word, span, &tok, tspan));
                }
                _ => {
                    let first = ty_args.is_empty() && len_args.is_empty();
                    ty_args.push(self.parse_type_expr().map_err(|e| match first {
                        true => format!("{e}{}", instantiation_element_note()),
                        false => e,
                    })?);
                }
            }
        }
        // R1a: the empty-list guard widens to "both sublists empty" so
        // `sum[4]` (no explicit type, one explicit length) parses; `sum[]`
        // (both empty) still errors.
        if ty_args.is_empty() && len_args.is_empty() {
            return Err(empty_instantiation_error(word, span));
        }
        Ok((ty_args, len_args))
    }

    /// P7.S6b (R1b): a length argument at a word call site, `sum[i64 4]`.
    /// Mirrors `parse_array_count`'s `1..=u32::MAX` range check but with a
    /// call-site-shaped message -- reusing `parse_array_count`'s own message
    /// would misdescribe the construct as an array type (`sum[i64 0]` is not
    /// `array[sum 0]`).
    fn parse_call_len_arg(&mut self, word: &str, span: Span) -> Result<Len, String> {
        match self.peek().cloned() {
            Some((Token::Int(n), _)) if (1..=i64::from(u32::MAX)).contains(&n) => {
                self.pos += 1;
                Ok(Len::Concrete(n as u32))
            }
            Some((Token::Int(n), nspan)) => {
                self.pos += 1;
                Err(instantiation_len_range_error(word, span, n, nspan))
            }
            _ => unreachable!("parse_call_len_arg called only when an int token is peeked"),
        }
    }

    /// The first type-variable token within the coming instantiation list, at
    /// any nesting depth, or `None` if the list (already open, `self.pos`
    /// past its `[`) contains none. Bracket-depth tracked so the scan stops at
    /// the list's own closing `]` rather than reading into whatever follows.
    fn instantiation_list_ty_var(&self) -> Option<(String, Span)> {
        let mut depth: i32 = 0;
        for (tok, tspan) in &self.tokens[self.pos..] {
            match tok {
                Token::Word(w) if w.contains('\'') => return Some((w.clone(), *tspan)),
                Token::LBracket => depth += 1,
                Token::RBracket => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                }
                _ => {}
            }
        }
        None
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
            Token::Word(w) => {
                let (type_args, len_args) = self.parse_explicit_type_args(&w, span)?;
                Ok(Term {
                    kind: TermKind::Call(w, type_args, len_args),
                    span,
                })
            }
            // R2: the term-level `[` is unambiguous against the type-level
            // `[` since every type-position bracket reader is reached only
            // from signature/type parsing, never from `parse_term`. A `[`
            // at term position is always a quotation literal (the
            // `[Type; Count]` array constructor is deleted, P7.S5).
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

    /// P7.S6a (R1/R2): drive `parse_header_bracket` directly (it is
    /// parser-private and, unlike `type:`/`trait:`, has no other route that
    /// preserves per-entry `Kind` -- `GenericHeader`'s own two-list split is
    /// P7.S6a phase 2). `src` is the bracket alone, e.g. `"['T 'N: Len]"`.
    fn header_bracket_vars(src: &str) -> Result<Vec<(String, Span, Kind)>, String> {
        let tokens = lex(src).unwrap();
        let mut arrays = Vec::new();
        let mut owned_cells = Vec::new();
        let mut refs = Vec::new();
        let mut slices = Vec::new();
        let mut generics = GenericTypes::default();
        let imports = HashMap::new();
        let exports: Vec<Vec<(String, Span)>> = Vec::new();
        let selective = HashMap::new();
        let type_origin: Vec<HashMap<String, u32>> = Vec::new();
        let trait_origin: Vec<HashMap<String, u32>> = Vec::new();
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
            structs: &[],
            enums: &[],
            arrays: &mut arrays,
            owned_cells: &mut owned_cells,
            refs: &mut refs,
            slices: &mut slices,
            module: 0,
            imports: &imports,
            exports: &exports,
            selective: &selective,
            type_origin: &type_origin,
            trait_origin: &trait_origin,
            generics: &mut generics,
            field_kind_marks: std::collections::HashMap::new(),
            traits: &[],
        };
        parser.parse_header_bracket("Test")
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
            lex("type: Box['T] val 'T ; type: Result['T 'E] | Ok val 'T | Err val 'E ;\n").unwrap();
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
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | s Spy> drop ;\n";

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
        assert!(matches!(&gcd_body[1].kind, TermKind::Call(w, _, _) if w == "b"));
        assert!(matches!(&gcd_body[2].kind, TermKind::IntLit(0)));
        assert!(matches!(&gcd_body[3].kind, TermKind::Call(w, _, _) if w == "eq"));
        match &gcd_body[4].kind {
            TermKind::Quotation(then_branch, is_inline, _) => {
                assert_eq!(then_branch.len(), 1);
                assert!(is_inline, "gcd.sth writes `if`'s arms `~[ ... ]` (R-C3)");
                assert!(matches!(&then_branch[0].kind, TermKind::Call(w, _, _) if w == "a"));
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
        assert!(matches!(&gcd_body[6].kind, TermKind::Call(w, _, _) if w == "if"));

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
        assert!(matches!(&body[2].kind, TermKind::Call(w, _, _) if w == "a"));
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
        assert!(matches!(&body[0].kind, TermKind::Call(w, _, _) if w == "True"));
        assert!(matches!(&body[1].kind, TermKind::Call(w, _, _) if w == "False"));
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

    /// P7.S3t (R2): the adjacency predicate, both ways, on one source pair.
    /// `foo[i64]` is an explicit type instantiation; `foo [i64]` is the call
    /// followed by a quotation literal it has always been. Both spellings lex
    /// identically, so the whole distinction is the column arithmetic here.
    #[test]
    fn a_glued_bracket_instantiates_and_a_spaced_one_stays_a_quotation() {
        let module = parse_src(": w ( -- ) foo[i64] foo [i64] ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 3);
        match &body[0].kind {
            TermKind::Call(name, args, _) => {
                assert_eq!(name, "foo");
                assert_eq!(args, &vec![Type::I64]);
            }
            other => panic!("expected an instantiated Call, got {other:?}"),
        }
        match &body[1].kind {
            TermKind::Call(name, args, _) => {
                assert_eq!(name, "foo");
                assert!(args.is_empty(), "a spaced bracket instantiates nothing");
            }
            other => panic!("expected a bare Call, got {other:?}"),
        }
        match &body[2].kind {
            TermKind::Quotation(terms, _, _) => {
                assert!(matches!(&terms[0].kind, TermKind::Call(w, _, _) if w == "i64"));
            }
            other => panic!("expected a Quotation, got {other:?}"),
        }
    }

    /// P7.S3t (R2): the list takes several arguments, so the syntax extends to
    /// a multi-variable callee without a second call form.
    #[test]
    fn an_instantiation_reads_several_type_arguments() {
        let module = parse_src(": w ( -- ) foo[i64 f64 i64] ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Call(_, args, _) => {
                assert_eq!(args, &vec![Type::I64, Type::F64, Type::I64]);
            }
            other => panic!("expected an instantiated Call, got {other:?}"),
        }
    }

    /// P7.S3t (R2): the list runs to end of input. The error names the call,
    /// since the construct exists only because of the glue and one remedy is a
    /// space.
    #[test]
    fn an_unterminated_instantiation_names_the_call() {
        let err = parse_src(": w ( -- ) foo[i64").unwrap_err();
        assert!(
            err.contains("unterminated explicit type instantiation of `foo` at line 1, col 12"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3t (R2): `foo[]`. Empty is not the same as absent -- downstream both
    /// are one empty vector -- so the arity rule could never fire for it.
    #[test]
    fn an_empty_instantiation_is_rejected() {
        let err = parse_src(": w ( -- ) foo[] ;").unwrap_err();
        assert!(
            err.contains("`foo[]` at line 1, col 12 instantiates nothing"),
            "unexpected message: {err}"
        );
    }

    /// P7.S6b (R1): a call-site length variable, `sum[i64 4]`, records one
    /// type argument and one length argument.
    #[test]
    fn a_call_site_instantiation_reads_a_type_and_a_length() {
        let module = parse_src(": w ( -- ) sum[i64 4] ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Call(name, ty_args, len_args) => {
                assert_eq!(name, "sum");
                assert_eq!(ty_args, &vec![Type::I64]);
                assert_eq!(len_args, &vec![Len::Concrete(4)]);
            }
            other => panic!("expected an instantiated Call, got {other:?}"),
        }
    }

    /// P7.S6b (R1): `sum[i64]` (no length argument) must stay a pure
    /// type-arg call -- the empty-length path is byte-identical downstream,
    /// so a parser regression here is easy to miss.
    #[test]
    fn a_call_site_instantiation_with_only_a_type_has_no_length() {
        let module = parse_src(": w ( -- ) sum[i64] ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Call(name, ty_args, len_args) => {
                assert_eq!(name, "sum");
                assert_eq!(ty_args, &vec![Type::I64]);
                assert!(len_args.is_empty());
            }
            other => panic!("expected an instantiated Call, got {other:?}"),
        }
    }

    /// P7.S6b (R1a): `sum[4]` -- a bare length argument, no explicit type --
    /// parses; the empty-list guard only fires when both sublists are empty.
    #[test]
    fn a_call_site_instantiation_with_only_a_length_has_no_type() {
        let module = parse_src(": w ( -- ) sum[4] ;").unwrap();
        match &terms_body(&module.words[0])[0].kind {
            TermKind::Call(name, ty_args, len_args) => {
                assert_eq!(name, "sum");
                assert!(ty_args.is_empty());
                assert_eq!(len_args, &vec![Len::Concrete(4)]);
            }
            other => panic!("expected an instantiated Call, got {other:?}"),
        }
    }

    /// P7.S6b (R1a): widening the empty-list guard to "both sublists empty"
    /// must not also widen away the genuinely-empty case -- `sum[]` still
    /// errors exactly as it did before length arguments existed.
    #[test]
    fn an_empty_instantiation_with_a_length_variable_declared_is_still_rejected() {
        let err = parse_src(": w ( -- ) sum[] ;").unwrap_err();
        assert!(
            err.contains("`sum[]` at line 1, col 12 instantiates nothing"),
            "unexpected message: {err}"
        );
    }

    /// P7.S6b (R1): a type token after a length token has nowhere to go --
    /// the call-site grammar fixes "types first, then lengths" as its own
    /// convention, independent of how the callee declared its own bracket.
    #[test]
    fn a_type_token_after_a_length_token_is_a_parse_error() {
        let err = parse_src(": w ( -- ) sum[4 i64] ;").unwrap_err();
        assert!(
            err.contains("expected a length argument or `]`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("sum"), "unexpected message: {err}");
    }

    /// P7.S6b (R1b): a length argument out of `1..=u32::MAX` gets the
    /// call-site-shaped range error, not `parse_array_count`'s array-type
    /// message.
    #[test]
    fn a_call_site_length_argument_out_of_range_is_a_located_error() {
        let too_low = parse_src(": w ( -- ) sum[i64 0] ;").unwrap_err();
        assert!(
            too_low.contains("sum[...]") && too_low.contains("length argument 0"),
            "unexpected message: {too_low}"
        );
        assert!(
            !too_low.contains("array type"),
            "unexpected message: {too_low}"
        );

        let too_high = parse_src(&format!(
            ": w ( -- ) sum[i64 {}] ;",
            u64::from(u32::MAX) + 1
        ))
        .unwrap_err();
        assert!(
            too_high.contains("sum[...]") && too_high.contains("out of range"),
            "unexpected message: {too_high}"
        );
        assert!(
            !too_high.contains("array type"),
            "unexpected message: {too_high}"
        );
    }

    /// P7.S3t (R2): a malformed *first* element carries the note, because the
    /// spelling it re-points (a call plus a quotation or array literal) is one
    /// a space restores. A later element cannot be that mistake -- the parse is
    /// already committed to a type list by then -- so it does not.
    #[test]
    fn a_malformed_first_element_says_the_glue_chose_type_position() {
        let note = "a glued bracket is an explicit type instantiation";
        let first = parse_src(": w ( -- ) foo[Nope] ;").unwrap_err();
        assert!(first.contains("unknown type `Nope`"), "unexpected: {first}");
        assert!(first.contains(note), "unexpected: {first}");
        let later = parse_src(": w ( -- ) foo[i64 Nope] ;").unwrap_err();
        assert!(later.contains("unknown type `Nope`"), "unexpected: {later}");
        assert!(!later.contains(note), "unexpected: {later}");
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
                assert!(matches!(&terms[1].kind, TermKind::Call(ref w, _, _) if w == "add"));
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
                assert!(matches!(&terms[1].kind, TermKind::Call(ref w, _, _) if w == "add"));
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
            ty_kinds: Vec::new(),
            len_var_names: vec![],
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
        assert!(matches!(&body[0].kind, TermKind::Call(w, _, _) if w == ">="));
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
        assert!(matches!(&body[0].kind, TermKind::Call(w, _, _) if w == "if"));
    }

    /// Migrated off the retired bare-line path (R5b): a bare term
    /// sequence's own shape (multiple terms, an int literal, a trailing
    /// call) is a general parsing fact with no line-only content, so it
    /// moves onto a one-word module body.
    #[test]
    fn parse_bare_term_sequence_is_a_multi_term_body() {
        let module = parse_src(": w ( i64 i64 -- i64 ) 2 3 add ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 3);
        assert!(matches!(body[0].kind, TermKind::IntLit(2)));
        assert!(matches!(&body[2].kind, TermKind::Call(w, _, _) if w == "add"));
    }

    /// Migrated off the retired bare-line path (R5b): a float literal
    /// token parses to `TermKind::FloatLit`, a general lexing/parsing fact,
    /// not a line-only one.
    #[test]
    fn parse_float_lit_term_is_float_lit() {
        let module = parse_src(": w ( -- f64 ) 2.5 ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert_eq!(body.len(), 1);
        assert!(matches!(body[0].kind, TermKind::FloatLit(v) if v == 2.5));
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
        let module = parse_src("type: Box['T] val 'T ;").unwrap();
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
        let module = parse_src("type: Pair['A 'B] a 'A b 'B ;").unwrap();
        assert_eq!(module.generic_structs.len(), 1);
        let decl = &module.generic_structs[0];
        assert_eq!(decl.name, "Pair");
        assert_eq!(decl.ty_var_names, ["'A", "'B"]);
        assert_eq!(decl.fields[0], ("a".to_string(), PolyType::Var(0)));
        assert_eq!(decl.fields[1], ("b".to_string(), PolyType::Var(1)));
    }

    #[test]
    fn parse_generic_typedef_concrete_field_resolves_alongside_a_variable_field() {
        let module = parse_src("type: Wrap['T] tag i64 val 'T ;").unwrap();
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
        let result = parse_src("type: Box['T] val 'E ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("'E"), "unexpected message: {err}");
        assert!(err.contains("Box"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 19"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_typedef_phantom_variable_is_error() {
        // R1 (round 2): `'T` is bound but never used in any field.
        let result = parse_src("type: Phantom['T] x i64 ;");
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
        let result = parse_src("type: Bad['T 'T] x 'T ;");
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
        let result = parse_src("type: E['T] | Ok v 'T 'z ;");
        let err = result.unwrap_err();
        assert!(err.contains("'z"), "unexpected message: {err}");
        assert!(err.contains("bound by"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 23"), "unlocated: {err}");
        assert!(
            parse_src("type: E['T 'z] | Ok v 'T 'z ;").is_ok(),
            "binding `'z` in the header should make the same body legal"
        );
    }

    #[test]
    fn parse_generic_typedef_odd_field_count_names_the_generic_header() {
        // `'bar` reads as a type parameter, so the trailing `i64` is a field
        // name with no type. The plain odd-field-count message would name
        // `i64`, a token the author never got wrong; this one says the header
        // was read as generic over `'bar`.
        let result = parse_src("type: Foo['bar] i64 ;");
        let err = result.unwrap_err();
        assert!(
            err.contains("generic `type: Foo['bar]`"),
            "unexpected message: {err}"
        );
        assert!(err.contains("line 1, col 21"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_registers_decl() {
        let module = parse_src("type: Result['T 'E] | Ok val 'T | Err val 'E ;").unwrap();
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
        let module = parse_src("type: Maybe['T] None | Some v 'T ;").unwrap();
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
        let result = parse_src("type: Result['T 'E] | Ok val 'T | Err val 'X ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("'X"), "unexpected message: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_phantom_variable_is_error() {
        let result = parse_src("type: Result['T 'E] | Ok val 'T | Err other i64 ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("phantom"), "unexpected message: {err}");
        assert!(err.contains("'E"), "unexpected message: {err}");
    }

    #[test]
    fn parse_generic_enum_typedef_zero_variants_is_error() {
        let result = parse_src("type: Empty['T] | ;");
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
        let result = parse_src("type: Bad['T] | ^Evil val 'T ;");
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
        let module = parse_src("type: Option['T] | None | Some 'T ;").unwrap();
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
        let module = parse_src("type: Result['T 'E] | Ok 'T | Err 'E ;").unwrap();
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
        let module = parse_src("type: E['T 'U] | V val 'T 'U ;").unwrap();
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
        let module = parse_src("type: Box['T] val 'T ; : main ( -- ) ;").unwrap();
        assert_eq!(module.generic_structs.len(), 1);
        assert!(module.words.iter().any(|w| w.name == "main"));
    }

    #[test]
    fn parse_generic_typedef_does_not_shadow_a_concrete_typedef_registered_after_it() {
        // A generic header is skipped entirely by the concrete pre-pass, so a
        // concrete `type:` elsewhere in the file is unaffected.
        let module = parse_src("type: Box['T] val 'T ; type: Vec2 x i64 y i64 ;").unwrap();
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
            sole_generic_field("type: Pair['T] items array['T 2] ;"),
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(2))
        );
    }

    #[test]
    fn parse_generic_field_nested_array_of_ty_var_builds_nested_polytype() {
        // R1: the descent recurses, so depth is unbounded rather than one.
        assert_eq!(
            sole_generic_field("type: NestArr['T] grid array[array['T 2] 3] ;"),
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
            sole_generic_field("type: Cell['T] c ^'T ;"),
            PolyType::OwnedCell(Box::new(PolyType::Var(0)))
        );
        // A `^`-run nests, one wrapper per caret.
        assert_eq!(
            sole_generic_field("type: Cell2['T] c ^^'T ;"),
            PolyType::OwnedCell(Box::new(PolyType::OwnedCell(Box::new(PolyType::Var(0)))))
        );
    }

    #[test]
    fn parse_generic_field_ref_of_ty_var_builds_ref_polytype() {
        // R1/R10: `&'T` *parses* -- it does not build, but the rejection must
        // come from the no-stored-reference rule rather than `unknown type`.
        assert_eq!(
            sole_generic_field("type: Box['T] r &'T ;"),
            PolyType::Ref(Box::new(PolyType::Var(0)), false)
        );
        assert_eq!(
            sole_generic_field("type: BoxM['T] r &!'T ;"),
            PolyType::Ref(Box::new(PolyType::Var(0)), true)
        );
    }

    #[test]
    fn parse_generic_field_generic_application_of_ty_vars_builds_generic_polytype() {
        // R1: each argument is a field type in its own right, so the header's
        // variables reach an application's argument list.
        let module =
            parse_src("type: Ent['K 'V] k 'K v 'V ;\ntype: Wrap['K 'V] e Ent['K 'V] ;\n").unwrap();
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
            parse_src("type: Ent['K 'V] k 'K v 'V ;\ntype: W['K] e Ent['K i64] ;\n").unwrap();
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
        let err = parse_src("type: Box['T] items array['E 2] ;").unwrap_err();
        assert!(err.contains("'E"), "unexpected message: {err}");
        assert!(err.contains("Box"), "unexpected message: {err}");
        assert!(err.contains("line 1, col 27"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_field_ty_var_used_only_inside_array_is_not_phantom() {
        // N4: `check_no_phantom_ty_var` reads the `used` bitmap alone, so the
        // descent has to set it at the leaf, at whatever depth. Without that
        // this fixture is rejected as a phantom parameter.
        assert!(parse_src("type: Pair['T] items array['T 2] ;").is_ok());
        assert!(parse_src("type: Cell['T] c ^'T ;").is_ok());
        assert!(parse_src("type: Deep['T] g array[array[^'T 2] 3] ;").is_ok());
    }

    #[test]
    fn parse_generic_enum_variant_named_field_array_of_ty_var_builds_array_polytype() {
        // R1: the enum twin shares the field parser, but routes through
        // `parse_generic_variant_fields` rather than the struct field loop.
        let module = parse_src("type: Buf['T] | Some xs array['T 2] | None ;").unwrap();
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
        let module = parse_src("type: L['T] v 'T next ^L[i64] ;").unwrap();
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
        let module = parse_src("type: L['T] | Node v 'T n ^L[i64] | Nil ;").unwrap();
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
        let module = parse_src("type: Box['T] v 'T ; type: Box['T] w 'T ;").unwrap();
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
            parse_src("type: A['T] v 'T next ^B['T] ;\ntype: B['T] w 'T back ^A['T] ;\n").unwrap();
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
        let err = parse_src("type: L['T] v 'T next ^L[^'T] ;").unwrap_err();
        assert!(err.contains("owning cell over"), "unexpected: {err}");
        assert!(
            err.contains("fully concrete or a bare type variable"),
            "the message must name the restriction, not just say `recursive`: {err}"
        );
        assert!(err.contains("line 1, col 23"), "unlocated: {err}");
        // The array and reference wrappers are the same rule.
        let err = parse_src("type: L['T] v 'T next ^L[array['T 2]] ;").unwrap_err();
        assert!(err.contains("array of"), "unexpected: {err}");
        let err = parse_src("type: L['T] v 'T next ^L[&'T] ;").unwrap_err();
        assert!(err.contains("reference to"), "unexpected: {err}");
    }

    #[test]
    fn parse_generic_field_growing_argument_under_each_field_wrapper_is_error() {
        // R8's *descent*, distinct from the sibling test above: that one
        // varies the offending argument's wrapper while pinning the field's
        // own top level as `^`, so it only ever exercises the `OwnedCell`
        // arm of the walk. Both other wrapper arms can be stubbed to `Ok(())`
        // with the whole suite still green, and `array[L[^'T] 2]` -- an array of
        // an application, the shape `Map`'s backing store actually takes --
        // is then silently admitted.
        let err = parse_src("type: L['T] v 'T kids array[L[^'T] 2] ;").unwrap_err();
        assert!(
            err.contains("owning cell over"),
            "the walk must descend through an array field: {err}"
        );
        let err = parse_src("type: L['T] v 'T kids & L[^'T] ;").unwrap_err();
        assert!(
            err.contains("owning cell over"),
            "the walk must descend through a reference field: {err}"
        );
    }

    #[test]
    fn parse_generic_field_compound_concrete_argument_beside_a_variable_is_ok() {
        // R8's accept side: an argument that is compound but *fully concrete*
        // at any depth is inert -- it carries no variable to grow -- so it is
        // admitted; only a compound argument mentioning one of the header's
        // own variables is refused.
        //
        // The argument list has to be *mixed* for this to witness anything.
        // An all-concrete list (`^L[Ent[i64 u32]]`, `^L[array[i64 2]]`) folds the
        // whole application to `PolyType::Concrete` at parse time, leaving no
        // `Generic` node in the tree for R8's walk to reach -- so such a
        // fixture passes with the accept clause deleted outright, and is a
        // placebo. Here the bare `'K` is what keeps the application unfolded,
        // and the compound concrete argument beside it is what the clause has
        // to admit.
        assert!(
            parse_src("type: Ent['K 'V] k 'K v 'V ;\ntype: W['K] e Ent['K array[i64 2]] ;\n")
                .is_ok(),
            "a concrete array argument beside a variable one is inert"
        );
        assert!(
            parse_src("type: Ent['K 'V] k 'K v 'V ;\ntype: W2['K] e Ent['K Ent[i64 u32]] ;\n")
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
        let module = parse_src("type: B['T] v 'T r & array[i64 2] ;").unwrap();
        match module.generic_structs[0].fields[1].1 {
            PolyType::Concrete(Type::Ref(..)) => {}
            ref other => panic!("expected a folded concrete reference, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_field_variable_quotation_is_error() {
        // R7: out of scope, and a located rejection rather than `'T` being
        // misreported as an unknown concrete type.
        let err = parse_src("type: QF['T] f [ 'T -- 'T ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(err.contains("'T"), "unexpected: {err}");
        assert!(err.contains("QF"), "unexpected: {err}");
        assert!(err.contains("line 1, col 18"), "unlocated: {err}");
    }

    #[test]
    fn parse_generic_field_variable_after_a_nested_bracket_in_a_quotation_is_error() {
        // R7: the scan for the declaration's own variables has to track
        // bracket depth. Stopping at the first `]` -- an *inner* one here --
        // ends the scan before `'T`, and the field then falls through to the
        // concrete parser, which misreports `'T` as an unknown type instead of
        // naming the unsupported shape.
        let err = parse_src("type: QF['T] f [ array[i64 2] -- 'T ] ;").unwrap_err();
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
        let err = parse_src("type: QF['T] f [ ^'T -- ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(err.contains("'T"), "unexpected: {err}");
        assert!(
            !err.contains("unknown type"),
            "the glued cell sigil must not hide the variable: {err}"
        );

        let err = parse_src("type: QF['T] f [ &'T -- ] ;").unwrap_err();
        assert!(err.contains("quotation field"), "unexpected: {err}");
        assert!(err.contains("'T"), "unexpected: {err}");
        assert!(
            !err.contains("unknown type"),
            "the glued ref sigil must not hide the variable: {err}"
        );

        // Sigils stack, so peeling exactly one is not enough. `^^'T` is a
        // shape R1 admits outside a quotation (it builds nested cells), so a
        // single-strip peel leaves R7 blind to something R1 accepts.
        for src in [
            "type: QF['T] f [ ^^'T -- ] ;",
            "type: QF['T] f [ ^^^'T -- ] ;",
            "type: QF['T] f [ &!^'T -- ] ;",
        ] {
            let err = parse_src(src).unwrap_err();
            assert!(err.contains("quotation field"), "{src}: unexpected: {err}");
            assert!(
                !err.contains("unknown type"),
                "{src}: stacked sigils must not hide the variable: {err}"
            );
        }
    }

    #[test]
    fn parse_generic_field_glued_reference_to_generic_application_resolves() {
        // R1: `&` glues onto an applied header's token just as `^` does. The
        // `^` arm has an application branch; without its `&` twin only the
        // spaced `& Ent['K i64]` parses, and the glued spelling falls through
        // to the concrete parser's `unknown type 'K` -- the misreport the
        // `&` intercept exists to prevent. The field itself still never
        // builds (R10, phase 2); this is about which diagnostic it reaches.
        let module =
            parse_src("type: Ent['K 'V] k 'K v 'V ;\ntype: W['K] k 'K w &Ent['K i64] ;").unwrap();
        let w = module
            .generics
            .structs
            .iter()
            .find(|d| d.name == "W")
            .expect("`W`'s header is registered");
        match &w.fields[1].1 {
            PolyType::Ref(inner, false) => match inner.as_ref() {
                PolyType::Generic { name, args, .. } => {
                    assert_eq!(*name, "Ent");
                    assert!(
                        matches!(args.as_slice(), [PolyType::Var(0), PolyType::Concrete(_)]),
                        "the mixed argument list must survive the fold: {args:?}"
                    );
                }
                other => panic!("expected `&Ent['K i64]`, got a reference to {other:?}"),
            },
            other => panic!("expected `&Ent['K i64]`, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_field_concrete_quotation_still_parses() {
        // R7/N4: a bare `[` field is a quotation effect (P7.S6 R4), so a
        // legal concrete quotation field must still declare. `Q` needs a
        // variable-bearing field of its own too, else the phantom check
        // rejects the fixture for an unrelated reason.
        let module = parse_src("type: Q['T] v 'T f [ i64 -- i64 ] ;").unwrap();
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
        let module = parse_src("type: Box['T] val 'T ;\ntype: Wrap x Box[i64] ;").unwrap();
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
            parse_src("type: Box['T] val 'T ;\ntype: Wrap x Box[i64] y Box[u32] ;").unwrap();
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
            "type: Box['T] val 'T ;\ntype: A x Box[i64] ;\ntype: B y Box[i64] ;\n: f ( Box[i64] -- ) drop ;",
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
        let module = parse_src("type: Box['T] val 'T ;\n: f ( Box[i64] -- Box[i64] ) ;").unwrap();
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
            parse_src("type: Box['T] val 'T ;\n: f ( Box[i64] 'A -- 'A Box[i64] ) ;").unwrap();
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
        let module = parse_src("type: Box['T] val 'T ;\ntype: W x Box[Box[i64]] ;").unwrap();
        let (inner_id, _) = struct_by_name(&module, "Box[i64]");
        let (_, outer) = struct_by_name(&module, "Box[Box[i64]]");
        assert_eq!(outer.fields[0].1, Type::Struct(inner_id, "Box[i64]"));
    }

    #[test]
    fn parse_generic_application_resolves_a_generic_declared_later_in_the_file() {
        // The generic declarations are parsed ahead of the body pass, so an
        // application need not follow its declaration -- the order
        // independence a concrete `type:` name already has from the pre-pass.
        let module = parse_src("type: Wrap x Box[i64] ;\ntype: Box['T] val 'T ;").unwrap();
        let (box_id, _) = struct_by_name(&module, "Box[i64]");
        assert_eq!(
            struct_by_name(&module, "Wrap").1.fields[0].1,
            Type::Struct(box_id, "Box[i64]")
        );
    }

    #[test]
    fn parse_generic_application_with_no_arguments_is_a_located_error() {
        // R3: a generic name is never a type by itself.
        let err = parse_src("type: Box['T] val 'T ;\ntype: Wrap x Box ;").unwrap_err();
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
        let err = parse_src("type: Box['T] val 'T ;\ntype: Wrap x Box[i64 u32] ;").unwrap_err();
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
            parse_src("type: Pair['A 'B] a 'A b 'B ;\n: f ( Pair[i64] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("generic type `Pair` declares 2 type variables"),
            "unexpected message: {err}"
        );
        assert!(err.contains("1 was supplied"), "unexpected: {err}");
        assert!(err.contains("line 2, col 7"), "unlocated: {err}");
    }

    #[test]
    fn concrete_underapplication_of_a_length_carrying_header_names_both_kinds() {
        // P1-B fix: `Buffer[i64]` under-supplies the length argument.
        // `Buffer` declares 1 type variable and 1 length variable, not "2
        // type variables" (the pre-fix, kind-blind message), and the
        // example syntax must be `Buffer[T N]`, not `Buffer[T T]`.
        let err = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n: f ( Buffer[i64] -- ) drop ;",
        )
        .unwrap_err();
        assert!(
            err.contains("generic type `Buffer` declares 1 type variable and 1 length variable"),
            "unexpected message: {err}"
        );
        assert!(err.contains("1 was supplied"), "unexpected: {err}");
        assert!(
            err.contains("apply it as `Buffer[T N]`"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn concrete_application_of_a_length_carrying_header_parses() {
        // R6: `Buffer[i64 4]` splits the bracket at `ty_arity` (1) into a
        // type expression, then the remaining slot into a length literal --
        // and, both concrete, instantiates exactly as the type-only path
        // does. Before this phase this reached
        // `generic_length_application_unsupported_error`.
        let module = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : f ( Buffer[i64 4] -- ) drop ;\n",
        )
        .unwrap();
        let (_, decl) = struct_by_name(&module, "Buffer[i64 4]");
        assert_eq!(decl.name, "Buffer[i64 4]");
    }

    #[test]
    fn concrete_application_with_non_literal_in_length_position_is_a_located_error() {
        // R6: a non-literal type expression in a length position, at a
        // concrete use site, is the same located error `parse_array_count`
        // gives a non-literal array count.
        let err = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : f ( Buffer[i64 i64] -- ) drop ;\n",
        )
        .unwrap_err();
        assert!(
            err.contains("array count must be a decimal literal"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn signature_application_of_a_length_carrying_header_binds_a_length_variable() {
        // R7: a bare `'N` in a length position interns a length variable
        // through the enclosing `PolyBuilder`, exactly as `parse_poly_array`
        // already does, landing in `PolyType::Generic`'s `len_args`. Before
        // this phase this reached
        // `generic_length_application_unsupported_error`.
        let module = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : f ( 'T Buffer['T 'N] -- 'T ) swap drop ;\n",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Generic {
            is_enum, len_args, ..
        } = &sig.inputs[1]
        else {
            panic!("expected `PolyType::Generic`, got {:?}", sig.inputs[1]);
        };
        assert!(!is_enum);
        assert_eq!(len_args, &[Len::Var(0)]);
    }

    #[test]
    fn signature_application_of_a_length_carrying_header_with_concrete_type_stays_generic() {
        // R7's added ruling: `Buffer['T 4]` has a variable type argument and
        // a concrete length -- the eager-concrete collapse must key off
        // *both* args and lens, or this would wrongly instantiate a concrete
        // struct with nowhere to place `'T`.
        let module = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : f ( 'T Buffer['T 4] -- 'T ) swap drop ;\n",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Generic { args, len_args, .. } = &sig.inputs[1] else {
            panic!("expected `PolyType::Generic`, got {:?}", sig.inputs[1]);
        };
        assert!(matches!(args[0], PolyType::Var(_)));
        assert_eq!(len_args, &[Len::Concrete(4)]);
    }

    #[test]
    fn signature_application_of_a_length_carrying_header_with_concrete_type_variable_length_stays_generic(
    ) {
        // R7's actual collapse-gate witness: `Buffer[i64 'N]` has a concrete
        // type argument (already `PolyType::Concrete` on its own) and a
        // *variable* length -- unlike the `Buffer['T 4]` sibling above,
        // whose variable type arg is caught by the pre-existing type-args-
        // only concreteness check regardless of the length gate. Only this
        // shape can tell the length-concreteness gate apart from "always
        // collapse": without it, this wrongly folds to `PolyType::Concrete`
        // and panics at `substitute_generic_field`'s `Array(_, Len::Var(v))`
        // arm when the field is later substituted.
        let module = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : f ( Buffer[i64 'N] -- ) drop ;\n",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Generic { args, len_args, .. } = &sig.inputs[0] else {
            panic!("expected `PolyType::Generic`, got {:?}", sig.inputs[0]);
        };
        assert_eq!(args, &[PolyType::Concrete(Type::I64)]);
        assert_eq!(len_args, &[Len::Var(0)]);
    }

    #[test]
    fn signature_application_of_a_length_carrying_header_all_concrete_folds() {
        // R7: every arg and length concrete folds to `PolyType::Concrete`,
        // exactly as the type-only fold already does.
        let module = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\n\
             : f ( 'T Buffer[i64 4] -- 'T ) swap drop ;\n",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(
            sig.inputs[1],
            PolyType::Concrete(Type::Struct(..))
        ));
    }

    #[test]
    fn signature_application_of_a_length_carrying_enum_header_binds_a_length_variable() {
        // The enum twin: the same split, read off `enums[idx].len_var_names`.
        let module = parse_src(
            "type: Ring['T 'N: Len] | Full data array['T 'N] | Empty ;\n\
             : f ( 'T Ring['T 'N] -- 'T ) swap drop ;\n",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Generic {
            is_enum, len_args, ..
        } = &sig.inputs[1]
        else {
            panic!("expected `PolyType::Generic`, got {:?}", sig.inputs[1]);
        };
        assert!(is_enum);
        assert_eq!(len_args, &[Len::Var(0)]);
    }

    #[test]
    fn concrete_application_of_a_length_carrying_enum_header_parses() {
        // The enum twin of the struct case above, through `find_enum` rather
        // than `find_struct`.
        let module = parse_src(
            "type: Ring['T 'N: Len] | Full data array['T 'N] | Empty ;\n\
             : f ( Ring[i64 4] -- ) drop ;\n",
        )
        .unwrap();
        assert!(module.enums.iter().any(|e| e.name == "Ring[i64 4]"));
    }

    #[test]
    fn parse_generic_application_argument_order_is_part_of_the_identity() {
        // R4: the instantiation key is the ordered argument list, so the two
        // orderings are two entries with mirrored field types.
        let module =
            parse_src("type: Pair['A 'B] a 'A b 'B ;\ntype: W x Pair[i64 u32] y Pair[u32 i64] ;")
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
            parse_src("type: Res['T 'E] | Ok val 'T | Err val 'E ;\ntype: W r Res[i64 u32] ;")
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
        let owner = lex("type: Box['T] val 'T ;\n").unwrap();
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
        let owner = lex("type: Box['T] val 'T ;\nexport: Box ;\n").unwrap();
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
        let owner = lex("type: Box['T] val 'T ;\n").unwrap();
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
        let err = parse_src("type: Box['T] val 'T ;\n: f ( q::Box[i64] -- ) drop ;").unwrap_err();
        assert!(err.contains("unknown type `q::Box`"), "unexpected: {err}");
    }

    /// The minted instantiation carries the *instantiating* module's id, not
    /// a hard-coded `0` (the same defaulting hazard
    /// `parse_generic_typedef_and_enum_stamp_the_parser_module_id` guards on
    /// the declaration side).
    #[test]
    fn parse_generic_application_stamps_the_instantiating_module_id() {
        let tokens = lex(
            "type: Box['T] val 'T ;\ntype: Res['T] | Ok v 'T ;\n: f ( Box[i64] Res[u32] -- ) drop drop ;\n",
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
        let module = parse_src(": w ( array[i64 4] -- i64 ) drop 0 ;").unwrap();
        let w = &module.words[0];
        assert_eq!(module.arrays.len(), 1);
        match w.effect.inputs[0].ty {
            Type::Array(id, name) => {
                assert_eq!(id.index(), 0);
                assert_eq!(name, "array[i64 4]");
            }
            other => panic!("expected Type::Array, got {other:?}"),
        }
        assert_eq!(module.arrays[0].count, 4);
        assert_eq!(module.arrays[0].element, Type::I64);
    }

    #[test]
    fn parse_slot_array_type_same_shape_dedups_to_one_array_id() {
        let module =
            parse_src(": a ( array[i64 4] -- i64 ) drop 0 ; : b ( array[i64 4] -- i64 ) drop 0 ;")
                .unwrap();
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

    /// P7.S3h: `owning [ ... ]` parses at every type-position entry. Type
    /// dispatch is first-token only, so each entry needs its own prefix branch
    /// -- a missing one reports `unknown type 'owning'` rather than building
    /// the type. Covered here: a word's own slot (`parse_slot`), a struct field
    /// and an enum variant field (`parse_field_type_expr`), a nested effect row
    /// and a nested array element (`parse_type_expr`'s recursion), and a poly
    /// word's slot (`parse_poly_slot`). The *rejection* of the field and
    /// element positions is the checker's registry audit, so they must parse to
    /// reach it.
    #[test]
    fn owning_quotation_parses_at_every_type_position() {
        let module = parse_src(
            ": slot ( owning [ i64 -- ] -- ) drop ;\n\
             type: S q owning [ -- ] ;\n\
             type: E | None | Some q owning [ -- ] ;\n\
             : nested ( [ owning [ -- ] -- ] -- ) drop ;\n\
             : elem ( array[owning [ -- ] 4] -- ) drop ;\n\
             : poly ['T: Copy] ( 'T owning [ -- ] -- 'T ) drop ;\n",
        )
        .unwrap();
        let own_nil = crate::ast::owning_quotation_type(Vec::new(), Vec::new());
        assert_eq!(
            module.words[0].effect.inputs[0].ty,
            crate::ast::owning_quotation_type(vec![Type::I64], Vec::new())
        );
        assert_eq!(module.structs[0].fields[0].1, own_nil);
        assert_eq!(module.enums[0].variants[1].fields[0].1, own_nil);
        let Type::Quotation(eff) = module.words[1].effect.inputs[0].ty else {
            panic!("a `[ ... -- ... ]` slot is a quotation effect");
        };
        assert_eq!(eff.inputs, vec![own_nil]);
        let Type::Array(id, _) = module.words[2].effect.inputs[0].ty else {
            panic!("a `array[T N]` slot is an array");
        };
        assert_eq!(module.arrays[id.index()].element, own_nil);
        assert_eq!(
            module.words[3].poly.as_ref().unwrap().inputs[1],
            PolyType::Concrete(own_nil)
        );
    }

    /// P7.S3v (R7): `^` is not a lexer delimiter, so `^owning` arrives as one
    /// word and never reaches `parse_type_expr`'s `owning_quotation_ahead`
    /// dispatch -- the remainder `owning` used to resolve as an unknown type
    /// name. The spaced form is the control: it is a genuinely different code
    /// path (`split_owning_cell_word`'s empty-remainder branch recurses into
    /// `parse_type_expr`), so the two must be pinned separately, and `^Spy`
    /// pins that an ordinary payload is unaffected.
    #[test]
    fn a_glued_owning_cell_payload_parses_as_an_owning_quotation() {
        let module = parse_src(
            "type: Spy tag i64 ;\n\
             : glued ( ^owning [ -- ] -- ) drop ;\n\
             : spaced ( ^ owning [ -- ] -- ) drop ;\n\
             : ordinary ( ^Spy -- ) drop ;\n",
        )
        .unwrap();
        let own_nil = crate::ast::owning_quotation_type(Vec::new(), Vec::new());
        let payload = |w: usize| {
            let Type::OwnedCell(id, _) = module.words[w].effect.inputs[0].ty else {
                panic!("a `^`-led slot is an owned cell");
            };
            module.owned_cells[id.index()].payload
        };
        assert_eq!(payload(0), own_nil);
        assert_eq!(payload(1), own_nil);
        assert!(matches!(payload(2), Type::Struct(..)));
    }

    /// P7.S3h: `owning` is a type-position keyword, not a type name, so a
    /// following token that opens no effect is blamed on the keyword rather
    /// than reported as an unknown type one token later. P7.S3v (R7) adds the
    /// glued `^owning` form, which reads the same effect rows and so blames
    /// the same keyword -- located at the `owning`, not at the leading `^`.
    #[test]
    fn owning_without_a_quotation_effect_is_located() {
        for src in [
            ": f ( owning i64 -- ) drop ;",
            ": f ( owning array[i64 4] -- ) drop ;",
            ": f ( owning ~[ -- ] -- ) drop ;",
            "type: S q owning i64 ;",
            ": f ( ^owning i64 -- ) drop ;",
        ] {
            let err = parse_src(src).unwrap_err();
            assert!(
                err.contains("`owning` must be followed by a quotation effect"),
                "unexpected message for `{src}`: {err}"
            );
        }
    }

    /// P7.S3h: a variable-bearing `owning` effect is rejected rather than
    /// folded. `PolyType::Quotation` has nowhere to record the owning flavour,
    /// so folding one would silently hand the caller a plain quotation -- and
    /// the type inequality between the flavours is the whole safety story.
    #[test]
    fn owning_quotation_carrying_a_type_variable_is_rejected() {
        let err = parse_src(": f ['T: Copy] ( 'T owning [ 'T -- ] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("cannot carry a type variable"),
            "unexpected message: {err}"
        );
    }

    /// P7.S3h: `owning` is intercepted ahead of every user type registry, so a
    /// type or variant declared under that name would be unreachable rather
    /// than merely shadowed -- the same reason `Slice` is reserved.
    #[test]
    fn a_type_or_variant_named_owning_is_reserved() {
        for (src, kind) in [
            ("type: owning x i64 ;", "type"),
            ("type: E | owning | Other ;", "variant"),
        ] {
            let err = parse_src(src).unwrap_err();
            assert!(
                err.contains("is reserved for the owning-quotation syntax")
                    && err.contains(&format!("as a {kind} name")),
                "unexpected message for `{src}`: {err}"
            );
        }
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
        let module = parse_src(": w ( array[array[i64 4] 4] -- i64 ) drop 0 ;").unwrap();
        assert_eq!(module.arrays.len(), 2);
        match module.words[0].effect.inputs[0].ty {
            Type::Array(_, name) => assert_eq!(name, "array[array[i64 4] 4]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_typedef_array_field_resolves() {
        let module = parse_src("type: Buf items array[i64 16] top i64 ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        match module.structs[0].fields[0].1 {
            Type::Array(_, name) => assert_eq!(name, "array[i64 16]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
        assert_eq!(module.structs[0].fields[1].1, Type::I64);
    }

    #[test]
    fn parse_typedef_enum_variant_array_field_resolves() {
        let module = parse_src("type: Shape | Poly pts array[f64 3] ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        match module.enums[0].variants[0].fields[0].1 {
            Type::Array(_, name) => assert_eq!(name, "array[f64 3]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_array_type_unknown_element_is_error() {
        // X1: an unknown element type in `array[T N]` names the unknown element.
        let result = parse_src(": w ( array[Nope 4] -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("Nope"), "unexpected message: {err}");
    }

    #[test]
    fn parse_array_type_zero_length_is_error() {
        // X2: a zero (or negative) length names the type and the invalid length.
        let result = parse_src(": w ( array[i64 0] -- ) drop ;");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("array[i64 0]"), "unexpected message: {err}");
        assert!(err.contains(">= 1"), "unexpected message: {err}");
    }

    #[test]
    fn parse_array_type_non_literal_count_is_error() {
        // X3: a non-literal count names the offending count token.
        let result = parse_src(": w ( array[i64 n] -- ) drop ;");
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
        let result = parse_src(": w ( array[i64 4294967297] -- ) drop ;");
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
        let result = parse_src(&format!("{SPY_DEF}: w ( array[Spy 2] -- ) drop ;"));
        assert!(result.is_ok(), "unexpected error: {:?}", result.err());
    }

    #[test]
    fn parse_typedef_linear_array_field_parses_ok() {
        let result = parse_src(&format!("{SPY_DEF}type: Bag xs array[Spy 2] ;"));
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
        let module = parse_src(": w ( ^array[u8 4] -- ) ;").unwrap();
        assert_eq!(module.arrays.len(), 1);
        assert_eq!(module.owned_cells.len(), 1);
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^array[u8 4]"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
        match module.owned_cells[0].payload {
            Type::Array(_, name) => assert_eq!(name, "array[u8 4]"),
            other => panic!("expected Type::Array, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_nested_array_buffer_type_resolves() {
        let module = parse_src(": w ( ^^array[u8 4] -- ) ;").unwrap();
        assert_eq!(module.owned_cells.len(), 2);
        match module.words[0].effect.inputs[0].ty {
            Type::OwnedCell(_, name) => assert_eq!(name, "^^array[u8 4]"),
            other => panic!("expected Type::OwnedCell, got {other:?}"),
        }
    }

    #[test]
    fn parse_owning_cell_type_resolves_in_struct_field_position() {
        // R19: without the field position, `type: Buf b ^array[u8 4] ;` fails to
        // parse; this is the buffer case R1 advertises.
        let module = parse_src("type: Buf b ^array[u8 4] ;").unwrap();
        match module.structs[0].fields[0].1 {
            Type::OwnedCell(_, name) => assert_eq!(name, "^array[u8 4]"),
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
        // The named-slot path (`name : type`) also recognises `array[T N]`, not
        // just the unnamed-slot shortcut.
        let module = parse_src(": w ( arr : array[i64 4] -- i64 ) drop 0 ;").unwrap();
        let w = &module.words[0];
        assert_eq!(w.effect.inputs[0].name.as_deref(), Some("arr"));
        match w.effect.inputs[0].ty {
            Type::Array(_, name) => assert_eq!(name, "array[i64 4]"),
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
        let module = parse_src(": w ( &!array[u8 64] -- ) drop ;").unwrap();
        assert_eq!(module.words[0].effect.inputs[0].ty.name(), "&!array[u8 64]");
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
                format!(": {name} ( i64 -- ) drop ;"),
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
        let module = parse_src("trait: Show['T] : show ( &'T -- ) ; ;").unwrap();
        assert_eq!(module.traits.len(), 2, "Copy pre-seeded, plus Show");
        let show = module.traits.iter().find(|t| t.name == "Show").unwrap();
        assert_eq!(show.members.len(), 1);
        assert_eq!(show.members[0].name, "show");
        assert!(matches!(
            &show.members[0].sig.inputs[0],
            PolyType::Ref(r, false) if **r == PolyType::Var(0)
        ));
        assert!(show.members[0].sig.outputs.is_empty());
    }

    /// P7.S3s-follow (R1): the optional `inline` keyword between the member
    /// name and its `(` sets `declares_inline == true` on the member.
    #[test]
    fn parse_trait_decl_records_an_inline_member() {
        let module = parse_src("trait: Ord['T] : cmp inline ( 'T 'T -- i64 ) ; ;").unwrap();
        let ord = module.traits.iter().find(|t| t.name == "Ord").unwrap();
        assert_eq!(ord.members.len(), 1);
        assert_eq!(ord.members[0].name, "cmp");
        assert!(ord.members[0].declares_inline);
    }

    #[test]
    fn parse_trait_decl_zero_members_is_error() {
        let err = parse_src("trait: Show['T] ;").unwrap_err();
        assert!(err.contains("declares no members"), "{err}");
    }

    #[test]
    fn parse_trait_decl_second_header_variable_is_error() {
        // R16: single-type-variable traits only.
        let err = parse_src("trait: Rel['T] 'U : cmp ( &'T &'U -- ) ; ;").unwrap_err();
        assert!(err.contains("more than one type variable"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_introducing_a_local_variable_parses_with_header_var_zero() {
        // P7b.S2 (S2-1): the member single-variable gate is lifted -- a
        // member may declare its own locals (`'U` here), which intern after
        // the header var, and the header var keeps id 0 in the member's sig.
        // (Whether the member can *dispatch* is check-time rule S2-2, not a
        // parse rejection.)
        let module = parse_src("trait: Rel['T] : cmp ( &'T &'U -- ) ; ;").unwrap();
        let rel = module
            .traits
            .iter()
            .find(|t| t.name == "Rel")
            .expect("the trait parsed");
        let sig = &rel.members[0].sig;
        assert_eq!(sig.ty_var_names, vec!["'T", "'U"]);
        assert_eq!(
            sig.inputs[0],
            PolyType::Ref(Box::new(PolyType::Var(0)), false)
        );
        assert_eq!(
            sig.inputs[1],
            PolyType::Ref(Box::new(PolyType::Var(1)), false)
        );
    }

    #[test]
    fn parse_trait_decl_member_with_an_app_free_quotation_shape_parses() {
        // P7b.S2 (S2-3): a declared quotation parameter whose rows are
        // App-free is now a supported member shape (the S2-3 Quotation arm);
        // only an App *inside* a row is fenced (S2-15.d, tested below).
        // (A fully-concrete quotation folds to `PolyType::Concrete` at parse
        // time; a variable-bearing one stays `PolyType::Quotation`.)
        let module = parse_src("trait: Apply['T] : run ( &'T [ 'T -- 'T ] -- ) ; ;").unwrap();
        let apply = module
            .traits
            .iter()
            .find(|t| t.name == "Apply")
            .expect("the trait parsed");
        assert_eq!(
            apply.members[0].sig.inputs[1],
            PolyType::Quotation(
                vec![PolyType::Var(0)],
                vec![PolyType::Var(0)],
                false,
                None,
                None
            )
        );
    }

    #[test]
    fn parse_trait_decl_member_with_an_owned_cell_shape_is_error() {
        // P7.S3n (R3): the new owned-cell shape is deliberately *left out* of
        // the supported set -- `ground_member_type` has no cell arm, so a
        // `^'T` member would ground to nothing. Adding it to the supported
        // list is the mutation this catches, and it is a located rejection
        // rather than a wildcard fall-through.
        let err = parse_src("trait: Sink['T] : sink ( ^'T -- ) ; ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
    }

    #[test]
    fn parse_trait_decl_publishes_header_kind_and_span() {
        // P7b.S2 (S2-1): the header bracket's kind annotation and the header
        // variable's own span are published on `TraitDecl` instead of
        // discarded (F4). An annotated header carries its `Kind::Arrow`;
        // a plain one defaults to `Star`.
        let module = parse_src("trait: Functor['F: * -> *] : map ( 'F['T] -- ) ; ;").unwrap();
        let functor = module
            .traits
            .iter()
            .find(|t| t.name == "Functor")
            .expect("the trait parsed");
        assert_eq!(
            functor.var_kind,
            Kind::Arrow {
                domains: vec![Kind::Star],
                result: Box::new(Kind::Star),
            }
        );
        assert_eq!((functor.var_span.line, functor.var_span.col), (1, 16));
        let plain = parse_src("trait: F['F] : m ( 'F -- ) ; ;").unwrap();
        let f = plain
            .traits
            .iter()
            .find(|t| t.name == "F")
            .expect("the trait parsed");
        assert_eq!(f.var_kind, Kind::Star);
        assert_eq!((f.var_span.line, f.var_span.col), (1, 10));
    }

    #[test]
    fn parse_trait_decl_hkt_member_seeds_header_kind_and_keeps_var_zero() {
        // P7b.S2 (S2-1): each member's builder is seeded with the header
        // kind before the effect parse, so the W1 member -- `map ( 'F['T]
        // [ 'T -- 'U ] -- 'F['U] )` -- parses with the header var at id 0
        // carrying the declared `Arrow` kind and the member locals interning
        // after it as `Star`s. `ty_var_spans[0]` is the *header* span, so an
        // annotation-vs-usage conflict names the header as origin (S2-15.b).
        let module =
            parse_src("trait: Functor['F: * -> *] : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ; ;")
                .unwrap();
        let functor = module
            .traits
            .iter()
            .find(|t| t.name == "Functor")
            .expect("the trait parsed");
        let sig = &functor.members[0].sig;
        assert_eq!(sig.ty_var_names, vec!["'F", "'T", "'U"]);
        assert_eq!(
            sig.ty_kinds,
            vec![
                Kind::Arrow {
                    domains: vec![Kind::Star],
                    result: Box::new(Kind::Star),
                },
                Kind::Star,
                Kind::Star,
            ]
        );
        assert_eq!((sig.ty_var_spans[0].line, sig.ty_var_spans[0].col), (1, 16));
        assert_eq!((sig.ty_var_spans[1].line, sig.ty_var_spans[1].col), (1, 39));
        assert_eq!((sig.ty_var_spans[2].line, sig.ty_var_spans[2].col), (1, 51));
        assert_eq!(
            sig.inputs[0],
            PolyType::App {
                head: 0,
                args: vec![PolyType::Var(1)],
            }
        );
        assert_eq!(
            sig.outputs[0],
            PolyType::App {
                head: 0,
                args: vec![PolyType::Var(2)],
            }
        );
    }

    #[test]
    fn parse_trait_decl_arrow_header_bare_member_mention_names_both_spans() {
        // P7b.S2 (S2-15.b, direction 1): an `'F: * -> *` header with a bare
        // `'F` mention in a member is a located error carrying both spans --
        // the header annotation (the seeded kind's origin) and the member
        // usage. p6c's accepted fixture dies only now that S2-1 publishes
        // the kind.
        let err = parse_src("trait: Functor['F: * -> *] : size ( 'F -- i64 ) ; ;").unwrap_err();
        assert!(
            err.contains("is used as a plain type but has kind `* -> *`"),
            "{err}"
        );
        // Member-usage span, then header-annotation span.
        assert!(err.contains("line 1, col 37"), "{err}");
        assert!(err.contains("line 1, col 16"), "{err}");
    }

    #[test]
    fn parse_trait_decl_star_header_applied_member_mention_names_both_spans() {
        // P7b.S2 (S2-15.b, direction 2): a `*`-kinded header with an
        // `'F['T]` member dies the mirrored way -- the application is the
        // misuse, the header's bare binding the origin.
        let err = parse_src("trait: Wrap['F] : m ( 'F['T] -- ) ; ;").unwrap_err();
        assert!(
            err.contains("is applied like a type constructor but has kind `*`"),
            "{err}"
        );
        assert!(err.contains("line 1, col 23"), "{err}");
        assert!(err.contains("line 1, col 13"), "{err}");
    }

    #[test]
    fn parse_trait_decl_app_inside_member_quotation_row_is_fenced() {
        // P7b.S2 (S2-15.d, F10): an App inside a member quotation row is a
        // located fence of its own -- declarations *represent* the shape,
        // but `call` cannot see through one.
        let err =
            parse_src("trait: Functor['F: * -> *] : map ( 'F['T] [ 'F['T] -- 'U ] -- 'F['U] ) ; ;")
                .unwrap_err();
        assert!(
            err.contains("applies a type variable inside a quotation row"),
            "{err}"
        );
        assert!(err.contains("line 1, col 30"), "{err}");
        assert!(err.contains("keep quotation rows App-free"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_local_headed_app_is_unsupported_shape_error() {
        // P7b.S2 (S2-15.d, review fix): the row fence is for an App *inside a
        // quotation row*. A plain-slot App headed by a member local is also an
        // unsupported shape (the `[`-router accepts any var head), but no
        // quotation is involved -- it must take the generic unsupported-shape
        // message, not the self-contradictory fence text.
        let err = parse_src("trait: Functor['F: * -> *] : m ( 'G['T] -- ) ; ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
        assert!(err.contains("trait-var-headed application"), "{err}");
        assert!(!err.contains("inside a quotation row"), "{err}");
        // The ref-wrapped spelling routes the same way: an App under `&` is
        // still a plain-slot App, not a row-nested one.
        let err = parse_src("trait: Functor['F: * -> *] : m ( & 'G['T] -- ) ; ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
        assert!(!err.contains("inside a quotation row"), "{err}");
    }

    #[test]
    fn member_shape_is_supported_trait_var_headed_app_only() {
        // P7b.S2 (S2-3, App arm): supported iff the head is the trait var
        // (id 0) -- a member-local head has no dispatch story this slice.
        let dispatchable = PolyType::App {
            head: 0,
            args: vec![PolyType::Var(1)],
        };
        let local_headed = PolyType::App {
            head: 1,
            args: vec![PolyType::Var(2)],
        };
        assert!(member_shape_is_supported(&dispatchable));
        assert!(!member_shape_is_supported(&local_headed));
    }

    #[test]
    fn member_shape_is_supported_quotation_arm_fences_app_rows() {
        // P7b.S2 (S2-3, Quotation arm): App-free rows are supported; an App
        // anywhere inside a row -- including buried under an array element
        // or a nested quotation -- is fenced.
        let app_free = PolyType::Quotation(
            vec![PolyType::Var(0)],
            vec![PolyType::Var(1)],
            false,
            None,
            None,
        );
        let app_in_row = PolyType::Quotation(
            vec![PolyType::App {
                head: 0,
                args: vec![PolyType::Var(1)],
            }],
            vec![],
            false,
            None,
            None,
        );
        let app_under_array_in_row = PolyType::Quotation(
            vec![PolyType::Array(
                Box::new(PolyType::App {
                    head: 0,
                    args: vec![PolyType::Var(1)],
                }),
                Len::Concrete(2),
            )],
            vec![],
            false,
            None,
            None,
        );
        let app_in_nested_row = PolyType::Quotation(
            vec![PolyType::Quotation(
                vec![PolyType::App {
                    head: 0,
                    args: vec![PolyType::Var(1)],
                }],
                vec![],
                false,
                None,
                None,
            )],
            vec![],
            false,
            None,
            None,
        );
        assert!(member_shape_is_supported(&app_free));
        assert!(!member_shape_is_supported(&app_in_row));
        assert!(!member_shape_is_supported(&app_under_array_in_row));
        assert!(!member_shape_is_supported(&app_in_nested_row));
        assert!(member_quotation_row_mentions_app(&app_in_row));
        assert!(!member_quotation_row_mentions_app(&app_free));
    }

    #[test]
    fn parse_trait_copy_collides_with_the_reserved_predicate_entry() {
        // R2: `Copy` is a pre-seeded trait-table entry, so parsing a user
        // `trait: Copy` succeeds (it is a name, not a reserved-word check) --
        // the collision is caught by `check_trait_decls`, an ordinary
        // duplicate/collision, at check time. P7.S3s: `Ord` used to collide
        // the same way, but it is no longer a reserved entry (it is an
        // ordinary library trait now), so this still fires for `Copy` alone
        // -- confirmed by asserting `check_trait_decls` still runs the
        // reserved-module branch and not some other collision arm.
        let module = parse_src("trait: Copy['T] : foo ( &'T -- ) ; ;").unwrap();
        assert_eq!(
            module.traits.len(),
            2,
            "Copy pre-seeded, plus the user's own Copy"
        );
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
        let err = parse_src("trait: F['T] : go ( ..a &'T -- ..a ) ; ;").unwrap_err();
        assert!(err.contains("declares the row variable `..a`"), "{err}");
        // Input-side only, so the `row_in` arm is what rejects it (the case
        // above sets `row_out` too, and would still be caught by that alone).
        let err = parse_src("trait: F['T] : go ( ..a &'T -- ) ; ;").unwrap_err();
        assert!(err.contains("declares the row variable `..a`"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_with_an_output_only_row_variable_is_error() {
        // The output side carries its own `row_out`, reached only when the
        // input side declares none.
        let err = parse_src("trait: F['T] : go ( &'T -- ..b ) ; ;").unwrap_err();
        assert!(err.contains("declares the row variable `..b`"), "{err}");
    }

    /// P7.S3s-follow: the retired bare `name ( sig )` trait member form
    /// produces a located diagnostic naming the `: name ( ... ) ;`
    /// replacement, not a generic token mismatch.
    #[test]
    fn parse_trait_decl_bare_member_names_the_colon_form() {
        let err = parse_src("trait: Ord['T] cmp ( 'T 'T -- Ordering ) ;").unwrap_err();
        assert!(
            err.contains("trait `Ord` declares member `cmp` without a leading `:`"),
            "{err}"
        );
        assert!(
            err.contains("a trait member is declared `: cmp ( ... ) ;`"),
            "{err}"
        );
    }

    /// P7.S3s-follow: a member's terminating `;` is required before the
    /// trait's own `;`.
    #[test]
    fn parse_trait_decl_member_missing_terminating_semicolon_is_error() {
        // The member's `;` is required before the trait's own `;`. With only
        // one `;`, the member consumes it and the trait terminator is left
        // missing, so the error is the unterminated-trait EOF path.
        let err = parse_src("trait: Ord['T] : cmp ( 'T 'T -- i64 ) ;").unwrap_err();
        assert!(err.contains("unterminated `trait:`"), "{err}");
    }

    /// P7.S3s-follow: `: inline ( ... ) ;` declares a member *named* `inline`
    /// with `declares_inline == false`, the member-side twin of
    /// `parse_worddef`'s own carve-out. The name is consumed first, so the
    /// `inline` keyword slot sees the `(` that follows, not the name.
    #[test]
    fn parse_trait_decl_member_named_inline_still_parses() {
        let module = parse_src("trait: Show['T] : inline ( &'T -- ) ; ;").unwrap();
        let show = module.traits.iter().find(|t| t.name == "Show").unwrap();
        assert_eq!(show.members.len(), 1);
        assert_eq!(show.members[0].name, "inline");
        assert!(!show.members[0].declares_inline);
    }

    /// P7.S3s-follow: one optional `inline` keyword only. With the keyword
    /// slot in place, `: foo inline inline ( ... )` consumes the name `foo`,
    /// then the first `inline` as the keyword, and the second `inline` falls
    /// through to `expect(LParen)` and fails there, located.
    #[test]
    fn parse_trait_decl_member_double_inline_is_error() {
        let err = parse_src("trait: Show['T] : foo inline inline ( &'T -- ) ; ;").unwrap_err();
        assert!(err.contains("expected LParen"), "{err}");
    }

    #[test]
    fn parse_impl_decl_for_a_reserved_predicate_trait_is_error() {
        // R2: the reserved `Copy` entry participates in no orphan-rule or
        // export check, so an `impl: Copy for i64` used to fall through to
        // the orphan rule and demand a module that cannot exist. P7.S3s:
        // `Ord` used to be rejected the same way; it is now an ordinary
        // library trait (`impl: Ord for i64` is exactly how `core::cmp`
        // satisfies it), so only `Copy` still fires this guard -- confirmed
        // by asserting the guard still names the built-in-predicate reason.
        let err = parse_src("impl: Copy for i64\n  : show | p | p drop ;\n;").unwrap_err();
        assert!(err.contains("trait `Copy` cannot be implemented"), "{err}");
        assert!(err.contains("built-in predicate"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_with_a_length_variable_array_shape_is_error() {
        // A length-variable array (`&array['T 'N]`) is not a supported member
        // shape: `ground_member_type` only grounds `Len::Concrete`, so it must
        // be rejected here at the trait decl -- otherwise the body-form desugar
        // panics grounding it.
        let err = parse_src("trait: Foo['T] : bar ( &array['T 'N] -- ) ; ;").unwrap_err();
        assert!(err.contains("unsupported signature shape"), "{err}");
    }

    #[test]
    fn parse_trait_decl_member_bound_in_effect_is_error() {
        // `prepass_trait_decls` used to build its inner `Parser` with an
        // empty `traits` slice, so a bound (`'T: Copy`) inside a member
        // signature saw no predicate-trait table and reported "unknown
        // capability `Copy`" instead of a located bound diagnostic. That
        // subject -- a member-signature bound must not misreport "unknown
        // capability `Copy`" -- is unchanged; P7.S6 (R7a) only changes *which*
        // located diagnostic it is. A trait-member effect runs with
        // `forbid_bounds == false`, so it is the word-def message, not the
        // `impl:` one.
        let err = parse_src("trait: Show['T] : show ( 'T: Copy -- ) ; ;").unwrap_err();
        assert!(
            err.contains("may not be written inside a stack effect"),
            "unexpected message: {err}"
        );
        assert!(!err.contains("unknown capability"), "{err}");
        assert!(!err.contains("`impl:` target"), "{err}");
    }

    /// P7.S3r (R2): the body form's whole desugar, read off the AST -- the
    /// binding pair `check_impl_decls` will resolve, and the synthesized word
    /// carrying the trait member's signature grounded at the `for` type
    /// (concrete, never a `PolySig`, since there is no signature to restate).
    #[test]
    fn parse_impl_body_synthesizes_a_word_with_the_inherited_effect() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- i64 ) ; ;\n\
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

    /// P7b.S3 (S3-2): the synthesized member word carries
    /// `is_trait_member == true`; an ordinary word defined alongside it does
    /// not. No reader consults the field yet -- this pins the marker itself,
    /// not any behaviour it gates.
    #[test]
    fn synth_member_word_carries_is_trait_member_marker() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- i64 ) ; ;\n\
             impl: Show for i64\n\
               : show | p | p drop 7 ;\n\
             ;\n\
             : plain ( -- i64 ) 1 ;\n",
        )
        .unwrap();
        let synth = module
            .words
            .iter()
            .find(|w| w.name == "show;Show;0;i64")
            .expect("the member body is spliced in as a top-level word");
        assert!(synth.is_trait_member);
        let ordinary = module
            .words
            .iter()
            .find(|w| w.name == "plain")
            .expect("the ordinary word is still parsed");
        assert!(!ordinary.is_trait_member);
    }

    /// P7b.S3 (S3-2): a member word declared `inline` carries both the
    /// marker and `declares_inline` -- the two flags are independent, so a
    /// test asserting one must not accidentally pass because the other was
    /// set instead.
    #[test]
    fn synth_member_word_inline_carries_both_marker_and_declares_inline() {
        let module = parse_src(
            "trait: Ord['T] : cmp inline ( 'T 'T -- i64 ) ; ;\n\
             impl: Ord for i64\n\
               : cmp | a b | a b sub ;\n\
             ;",
        )
        .unwrap();
        let synth = module
            .words
            .iter()
            .find(|w| w.name == "cmp;Ord;0;i64")
            .expect("the member body is spliced in as a top-level word");
        assert!(synth.is_trait_member);
        assert!(synth.declares_inline);
    }

    /// P7b.S3 (S3-2): the generic-target branch of `parse_impl_member_body`
    /// also sets `is_trait_member == true` on the synthesized member word.
    /// Tested independently from the concrete-target case above so reverting
    /// either branch alone is caught -- a pair covered in one half only has
    /// shipped here before.
    #[test]
    fn synth_member_word_generic_target_carries_is_trait_member_marker() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 4]\n\
               : show | p | p drop ;\n\
             ;",
        )
        .unwrap();
        let synth = module
            .words
            .iter()
            .find(|w| w.name.contains("show;Show;"))
            .expect("the member body is spliced in as a top-level word");
        assert!(synth.is_trait_member);
        assert!(synth.poly.is_some(), "generic target stays polymorphic");
    }

    /// P7.S3s-follow: the concrete-target branch of `parse_impl_member_body`
    /// inherits the trait member's `declares_inline` flag instead of
    /// hardcoding `false`. Each branch gets its own test because a twinned
    /// pair covered in one half only has shipped here before.
    #[test]
    fn parse_impl_body_inherits_the_members_inline_flag() {
        let module = parse_src(
            "trait: Ord['T] : cmp inline ( 'T 'T -- i64 ) ; ;
\
             impl: Ord for i64
\
               : cmp | a b | a b sub ;
\
             ;",
        )
        .unwrap();
        let synth = module
            .words
            .iter()
            .find(|w| w.name == "cmp;Ord;0;i64")
            .expect("the member body is spliced in as a top-level word");
        assert!(synth.declares_inline);
        assert!(synth.poly.is_none(), "concrete target stays monomorphic");
    }

    /// P7.S3s-follow: the generic-target branch of `parse_impl_member_body`
    /// inherits the trait member's `declares_inline` flag. Tested
    /// independently from the concrete branch above so reverting either
    /// branch alone is caught.
    #[test]
    fn parse_impl_body_generic_target_inherits_the_members_inline_flag() {
        let module = parse_src(
            "trait: Show['T] : show inline ( &'T -- ) ; ;
\
             impl: Show for array['T 4]
\
               : show | p | p drop ;
\
             ;",
        )
        .unwrap();
        let synth = module
            .words
            .iter()
            .find(|w| w.name.contains("show;Show;"))
            .expect("the member body is spliced in as a top-level word");
        assert!(synth.declares_inline);
        assert!(synth.poly.is_some(), "generic target stays polymorphic");
    }

    /// P7.S3r (R4a): the member's own name binds to the synthesized word
    /// throughout its body, nested quotations included -- otherwise a recursive
    /// call would resolve against module scope, where the member name is not a
    /// word at all.
    #[test]
    fn parse_impl_body_rewrites_the_members_own_name_inside_a_quotation() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- i64 ) ; ;\n\
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
                TermKind::Call(n, _, _) => Some(n.as_str()),
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
            "trait: Show['T] : show ( &'T -- i64 ) ; ;\n\
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
        let err =
            parse_src("trait: Show['T] : show ( &'T -- ) ; ;\nimpl: Show for i64 ;").unwrap_err();
        assert!(err.contains("binds no members"), "{err}");
    }

    #[test]
    fn find_trait_in_module_resolves_own_module_then_qualified() {
        let show = crate::ast::TraitDecl {
            name: "Show".to_string(),
            kind: TraitKind::Nominal,
            var_kind: crate::ast::Kind::Star,
            var_span: Span::default(),
            members: Vec::new(),
            module: 1,
            span: Span::default(),
        };
        let mut traits = crate::ast::seed_predicate_traits();
        traits.push(show);
        let mut imports = HashMap::new();
        imports.insert("lib".to_string(), 1u32);
        let mut selective = HashMap::new();
        selective.insert("Show".to_string(), 1u32);
        let no_trait_origin: [HashMap<String, u32>; 0] = [];
        let no_selective: HashMap<String, u32> = HashMap::new();
        // Own-module: no `Show` declared in module 0, and no selective entry
        // for it here (a selective fallback would otherwise mask this case).
        assert!(find_trait_in_module(
            &traits,
            "Show",
            0,
            &imports,
            &no_selective,
            &no_trait_origin
        )
        .is_none());
        // Qualified, one-hop: `lib::Show` maps `lib` to module 1, which
        // declares `Show` directly.
        assert_eq!(
            find_trait_in_module(
                &traits,
                "lib::Show",
                0,
                &imports,
                &selective,
                &no_trait_origin
            ),
            Some(TraitId::from_index(1))
        );
        // Own module, unqualified: module 1 declares `Show` itself.
        assert_eq!(
            find_trait_in_module(&traits, "Show", 1, &imports, &selective, &no_trait_origin),
            Some(TraitId::from_index(1))
        );
        // Reserved predicate table: visible from any module, with an empty
        // `trait_origin` and no import/selective entry for it. P7.S3s: `Ord`
        // used to be the second reserved entry pinned here; only `Copy`
        // remains reserved.
        assert_eq!(
            find_trait_in_module(&traits, "Copy", 0, &imports, &selective, &no_trait_origin),
            Some(TraitId::from_index(0))
        );
        // One-hop selective: a bare `Show` reached via a selective import
        // targeting module 1, from a module (2) that neither declares nor
        // imports it by qualifier.
        assert_eq!(
            find_trait_in_module(&traits, "Show", 2, &imports, &selective, &no_trait_origin),
            Some(TraitId::from_index(1))
        );
    }

    /// P7.S3s (R1/C4): both `find_trait_in_module` fallback branches --
    /// qualified and selective -- resolve a trait re-exported through a hub
    /// module (declared elsewhere, only named on the hub's `export:` list)
    /// via `trait_origin`, mirroring `resolve_type_name_in_module`'s
    /// `type_origin` fallback.
    #[test]
    fn find_trait_in_module_falls_back_to_trait_origin_through_a_hub() {
        let greet = crate::ast::TraitDecl {
            name: "Greet".to_string(),
            kind: TraitKind::Nominal,
            var_kind: crate::ast::Kind::Star,
            var_span: Span::default(),
            members: Vec::new(),
            module: 0,
            span: Span::default(),
        };
        let traits = vec![greet];
        // Module 2 (consumer) imports module 1 (hub) under qualifier `h`;
        // module 1 does not declare `Greet` itself, it only re-exports it
        // from module 0.
        let mut imports = HashMap::new();
        imports.insert("h".to_string(), 1u32);
        let mut selective = HashMap::new();
        selective.insert("Greet".to_string(), 1u32);
        let mut trait_origin: Vec<HashMap<String, u32>> = vec![HashMap::new(), HashMap::new()];
        trait_origin[1].insert("Greet".to_string(), 0);
        assert_eq!(
            find_trait_in_module(&traits, "h::Greet", 2, &imports, &selective, &trait_origin),
            Some(TraitId::from_index(0))
        );
        assert_eq!(
            find_trait_in_module(&traits, "Greet", 2, &imports, &selective, &trait_origin),
            Some(TraitId::from_index(0))
        );
    }

    /// Pins that both branches consult `trait_origin` rather than scanning
    /// all modules directly: with an empty table, both the qualified and
    /// selective forms fail to resolve past the hub. (The fallback code
    /// itself is mutation-covered by the `hub_reexported_trait_resolves_*`
    /// integration goldens, not by this unit test.)
    #[test]
    fn find_trait_in_module_without_trait_origin_table_cannot_see_past_hub() {
        let greet = crate::ast::TraitDecl {
            name: "Greet".to_string(),
            kind: TraitKind::Nominal,
            var_kind: crate::ast::Kind::Star,
            var_span: Span::default(),
            members: Vec::new(),
            module: 0,
            span: Span::default(),
        };
        let traits = vec![greet];
        let mut imports = HashMap::new();
        imports.insert("h".to_string(), 1u32);
        let mut selective = HashMap::new();
        selective.insert("Greet".to_string(), 1u32);
        let no_trait_origin: [HashMap<String, u32>; 0] = [];
        // With no `trait_origin` table at all, neither the qualified nor the
        // selective form can see past the hub.
        assert!(find_trait_in_module(
            &traits,
            "h::Greet",
            2,
            &imports,
            &selective,
            &no_trait_origin
        )
        .is_none());
        assert!(
            find_trait_in_module(&traits, "Greet", 2, &imports, &selective, &no_trait_origin)
                .is_none()
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
        let module = parse_src(": dupit ['T: Copy] ( 'T -- 'T 'T ) global: COUNT w dup ;").unwrap();
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
        let module = parse_src(": dupit ['T: Copy] ( 'T -- 'T 'T ) dup ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert_eq!(sig.inputs.len(), 1);
        assert_eq!(sig.outputs.len(), 2);
        assert!(sig.has_bound(0, Bound::Copy));
        assert!(module.words[0].effect.inputs.is_empty());
    }

    #[test]
    fn parse_capabilities_still_folds_copy_byte_for_byte() {
        // P7.S3e (R2): `parse_capabilities`'s rewrite (a trait-table lookup
        // replacing the hardcoded string compare) must not change
        // `'T: Copy`'s existing parse result. P7.S3s: `Ord` is no longer a
        // predicate to fold this way at all -- `Bound::Ord` does not exist to
        // construct -- so this test now pins `Copy` alone. The
        // predicate-plus-user-trait companion the flip wanted beside it
        // already exists:
        // `parse_capabilities_composes_a_predicate_and_a_user_trait` below
        // pins `'T: Copy <user trait>` folding to
        // `[Bound::Copy, Bound::User(id)]`.
        let module = parse_src(": f ['T: Copy] ( 'T -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.bounds, vec![(0, Bound::Copy)]);
    }

    #[test]
    fn parse_capabilities_unknown_name_is_still_an_error() {
        // A name that resolves to neither a pre-seeded predicate entry nor a
        // declared trait is still X3.
        let err = parse_src(": f ['T: Nope] ( 'T -- 'T ) ;").unwrap_err();
        assert!(err.contains("unknown capability"), "{err}");
    }

    #[test]
    fn parse_capabilities_resolves_a_declared_trait_to_a_user_bound() {
        // P7.S3e (R6/R18): a nominal trait name in a bound resolves against
        // the same table `Copy` does, at parse time, and is baked into
        // `Bound::User(TraitId)` before `Resolver::rewrite` ever runs. Index 1
        // because the one pre-seeded predicate entry (`Copy`) occupies 0.
        let module =
            parse_src("trait: Show['T] : show ( &'T -- ) ; ;\n: f ['T: Show] ( 'T -- 'T ) ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.bounds, vec![(0, Bound::User(TraitId::from_index(1)))]);
    }

    #[test]
    fn parse_capabilities_composes_a_predicate_and_a_user_trait() {
        // R5: the capability list stays greedy across the two kinds, in
        // source order.
        let module = parse_src(
            "trait: Order['T] : cmp ( &'T &'T -- i64 ) ; ;\n: f ['T: Copy Order] ( 'T -- 'T ) ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(
            sig.bounds,
            vec![(0, Bound::Copy), (0, Bound::User(TraitId::from_index(1)))]
        );
    }

    #[test]
    fn parse_bound_bracket_ends_at_close_and_effect_follows() {
        // P7.S6 (R6a): the greedy bound list inside a bracket ends at `]`, not
        // at the enclosing effect's next input slot (there is no next slot inside
        // a bracket). Retargeted from `parse_capabilities_stops_before_a_following_type_slot`:
        // the old fixture's `'T: Show` token *was* the first input slot, so both
        // spellings have two inputs. The migration strips the `: Show` off that
        // slot and restates it in the bracket; the `sig.inputs` assertion is
        // unchanged, byte for byte.
        let module =
            parse_src("trait: Show['T] : show ( &'T -- ) ; ;\n: f['T: Show] ( 'T i64 -- 'T ) ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.bounds, vec![(0, Bound::User(TraitId::from_index(1)))]);
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
        // P7.S6 (R6a): migrated to bracket spelling.
        let err = parse_src(": f['T: q::Show] ( 'T -- 'T ) ;").unwrap_err();
        assert!(err.contains("unknown module qualifier `q`"), "{err}");
        assert!(err.contains("`q::Show`"), "{err}");
    }

    #[test]
    fn parse_bound_bracket_unknown_name_after_a_bound_is_an_error() {
        // P7.S6 (R6a): inside a bracket there is no next slot to fall through
        // to, so an unrecognised name past the first bound must now error
        // rather than silently breaking. Retired from
        // `parse_capabilities_unbound_qualifier_after_a_bound_is_the_next_slot`,
        // whose subject (an unresolvable qualifier falling through to the next
        // slot) is destroyed by the bracket grammar.
        let err = parse_src(
            "trait: Copy2['T] : dummy ( 'T -- ) ; ;\n: f['T: Copy2 q::Point] ( 'T -- 'T ) ;",
        )
        .unwrap_err();
        // Pinned to the *discriminating* text. Reverting the gate does not
        // make this program build -- the bracket loop's own
        // `bound_bracket_non_var_error` catches the broken-out token -- so an
        // "is an error" assertion, or one satisfied by either message naming
        // `q::Point`, passes with the gate deleted and guards nothing.
        assert!(
            err.contains("unknown capability `q::Point`"),
            "the bound list must reject the name itself: {err}"
        );
        assert!(
            !err.contains("inside the bound bracket"),
            "the name must not break out to the bracket loop's shape error: {err}"
        );
    }

    #[test]
    fn parse_bound_bracket_qualified_type_in_effect_still_resolves_as_a_slot() {
        // P7.S6 (R6a): the companion to the test above, keeping the original's
        // real-world case alive on the effect side. A qualified type in the
        // effect (not the bracket) is still an ordinary input slot, not a
        // bound -- so `q::Point` fails as an unknown-type error on the slot,
        // never as an in-bound error.
        let err = parse_src(
            "trait: Copy2['T] : dummy ( 'T -- ) ; ;\n: f['T: Copy2] ( 'T q::Point -- 'T ) ;",
        )
        .unwrap_err();
        assert!(!err.contains("in bound"), "{err}");
        assert!(err.contains("q::Point"), "{err}");
    }

    #[test]
    fn parse_length_variable_in_count_position() {
        // R1: `'N` in an array count slot is a length variable, lexically
        // identical to a type variable but distinguished by position.
        let module = parse_src(": alen ( array[i64 'N] -- array[i64 'N] usize ) len ;").unwrap();
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
        let module = parse_src(": f ['T: Copy] ( &'T -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert!(sig.has_bound(0, Bound::Copy));
        assert!(matches!(sig.outputs[0], PolyType::Var(0)));
    }

    #[test]
    fn parse_poly_ref_slot_with_bare_sigil_recurses_on_the_next_token() {
        // Slice 13 (R-A3, bare-sigil case): `[` *is* a delimiter, so `&array['T 4]`
        // arrives as a lone `&` followed by a genuine array token, which
        // recurses as a poly slot rather than resolving concretely.
        let module = parse_src(": peek ( array['T 4] -- &array['T 4] ) ;").unwrap();
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
        let module = parse_src(": f ( 'T array[i64 4] -- 'T &array[i64 4] ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        let PolyType::Concrete(ty) = sig.outputs[1] else {
            panic!("expected a folded `Concrete`, got {:?}", sig.outputs[1]);
        };
        assert_eq!(ty.name(), "&array[i64 4]");
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
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
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
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
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
        let err = parse_src("type: Box['T] val 'T ;\n: f ( 'T Box[Box['T]] -- ) drop drop ;")
            .unwrap_err();
        assert!(
            err.contains("names `Box[...]` as a type argument"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_poly_generic_arity_mismatch_is_error() {
        // R1: the poly-slot argument list reuses `generic_arity_error`
        // exactly as the concrete path does.
        let err = parse_src("type: Box['T] val 'T ;\n: f ( Box['T 'E] -- ) drop ;").unwrap_err();
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
        let owner = lex("type: Box['T] val 'T ;\n").unwrap();
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
            parse_src(": dup2 ['a: Copy 'b: Copy] ( ..s 'a 'b -- ..s 'a 'b 'a 'b ) over over ;")
                .unwrap();
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
        let err = parse_src(": f ( 'N array[i64 'N] -- i64 ) drop drop ;").unwrap_err();
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
            parse_src(": fx ( ..s i64 array[ [ ..s -- ..s ] 3 ] -- ..s ) drop drop ;").unwrap_err();
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
    fn parse_worddef_bound_in_effect_is_error() {
        // P7.S6 (R7): a bound written inside an effect is a located error
        // naming the bracket form. Retired from
        // `parse_x3_bound_on_use_occurrence_is_error`, whose subject (a bound
        // legal at a *binding* occurrence but not at a *use*) no longer exists
        // -- neither occurrence may carry one. The `written twice` half of that
        // subject survives in the bracket, as a duplicate declaration.
        let err = parse_src(": f ( 'T: Copy 'T -- 'T ) drop ;").unwrap_err();
        assert!(err.contains("'T"), "unexpected message: {err}");
        assert!(
            err.contains("may not be written inside a stack effect"),
            "unexpected message: {err}"
        );
        assert!(err.contains("bound bracket"), "unexpected message: {err}");
        // The use-occurrence spelling is the same error, not a different one.
        let at_use = parse_src(": f ( 'T 'T: Copy -- 'T ) drop ;").unwrap_err();
        assert!(
            at_use.contains("may not be written inside a stack effect"),
            "unexpected message: {at_use}"
        );
        // And a variable declared twice in the bracket is the duplicate error.
        let dup = parse_src(": f['T: Copy 'T: Copy] ( 'T -- 'T ) drop ;").unwrap_err();
        assert!(
            dup.contains("more than once") || dup.contains("twice"),
            "{dup}"
        );
    }

    #[test]
    fn parse_x3_unknown_capability_is_error() {
        // X3: an unknown capability name after a bound colon is a located error.
        let err = parse_src(": f ['T: Frobnicate] ( 'T -- 'T ) ;").unwrap_err();
        assert!(err.contains("Frobnicate"), "unexpected message: {err}");
        assert!(err.contains("capability"), "unexpected message: {err}");
    }

    // P7.S5 (R13): the `[Type; Count]` array constructor is deleted. A
    // body-level `[ ... ; ... ]` falls through to the quotation-literal arm,
    // where the `;` is an unexpected token (not a silent fallthrough to
    // quotation parsing: the `;` stops the parse with a located error).

    #[test]
    fn array_constructor_syntax_no_longer_parses() {
        let err = parse_src(": w ( -- ) [ i64 ; 4 ] drop ;").unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_missing_count_no_longer_parses() {
        let err = parse_src(": w ( -- ) [ i64 ; ] drop ;").unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_zero_count_no_longer_parses() {
        let err = parse_src(": w ( -- ) [ i64 ; 0 ] drop ;").unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_compound_element_no_longer_parses() {
        let err = parse_src(": w ( -- ) [ array[i64 3] ; 4 ] drop ;").unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
    }

    #[test]
    fn array_constructor_reference_element_no_longer_parses() {
        let err = parse_src(": w ( -- ) [ &i64 ; 4 ] drop ;").unwrap_err();
        assert!(err.contains("parse error"), "unexpected message: {err}");
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

    // P7.S4 (R1): generic `impl:` target parses without "unknown type `'T`".

    #[test]
    fn parse_impl_target_generic_array_var_elem_and_len_parses() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        assert!(module.impls[0].target.is_concrete() == false);
        let word = &module.words[0];
        assert!(
            word.poly.is_some(),
            "generic impl member word should be polymorphic"
        );
    }

    #[test]
    fn parse_impl_target_generic_var_parses() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for 'T\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        assert!(!module.impls[0].target.is_concrete());
        assert!(module.words[0].poly.is_some());
    }

    #[test]
    fn parse_impl_target_concrete_still_parses() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for i64\n\
               : show | p | p drop ;\n\
             ;",
        )
        .unwrap();
        assert!(module.impls[0].target.is_concrete());
        assert!(module.words[0].poly.is_none());
    }

    #[test]
    fn parse_impl_target_bound_on_var_is_error() {
        let err = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T: Copy 'N]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("may not carry an inline bound"), "{err}");
    }

    /// P7b.S1 review fix (P1): an `impl:` target that applies one of its
    /// own variables (`'F['T]`) used to parse and register silently even
    /// though `match_impl_target_rec` never matches an `App` pattern -- a
    /// located rejection at parse time instead, naming the applied
    /// variable and its binding span.
    #[test]
    fn parse_impl_target_app_is_error() {
        let err = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for 'F['T]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("may not apply its own type variable") && err.contains("'F[...]"),
            "{err}"
        );
    }

    /// The concrete-head twin: `impl: Show for Box['T]` still parses --
    /// only a *variable*-headed application is fenced, not `Generic`'s own
    /// existing shape (F5's applied-target impls already worked before
    /// this slice).
    #[test]
    fn parse_impl_target_generic_head_application_still_parses() {
        let module = parse_src(
            "type: Box['T] v 'T ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for Box['T]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        assert!(!module.impls[0].target.is_concrete());
    }

    // P7.S4b (R1): `where`-clause on an impl target parses and threads
    // bounds into the member word's PolySig.

    #[test]
    fn parse_impl_where_clause_single_bound_threads_into_poly_sig() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'T: Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        let target = &module.impls[0].target;
        assert_eq!(target.bounds.len(), 1);
        assert_eq!(target.bounds[0].0, 0, "'T is ty_var_names[0]");
        // The member word's PolySig carries the same bound.
        let sig = module.words[0]
            .poly
            .as_ref()
            .expect("generic impl member word is polymorphic");
        assert_eq!(sig.bounds, target.bounds);
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
    }

    #[test]
    fn parse_impl_where_clause_multiple_bounds_on_one_var() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Eq['T] : eq ( &'T &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'T: Show Eq\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        let target = &module.impls[0].target;
        assert_eq!(target.bounds.len(), 2, "'T: Show Eq → two bounds on var 0");
        assert_eq!(target.bounds[0].0, 0);
        assert_eq!(target.bounds[1].0, 0);
        let sig = module.words[0].poly.as_ref().expect("poly sig");
        assert_eq!(sig.bounds.len(), 2);
    }

    #[test]
    fn parse_impl_where_clause_multiple_variables() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             trait: Eq['T] : eq ( &'T &'T -- ) ; ;\n\
             type: Pair['A 'B] a 'A b 'B ;\n\
             impl: Show for Pair['T 'V] where 'T: Show 'V: Eq\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        let target = &module.impls[0].target;
        assert_eq!(
            target.ty_var_names,
            vec!["'T".to_string(), "'V".to_string()]
        );
        // 'T: Show → (0, Show), 'V: Eq → (1, Eq)
        assert_eq!(target.bounds.len(), 2);
        assert_eq!(target.bounds[0].0, 0, "'T is index 0");
        assert_eq!(target.bounds[1].0, 1, "'V is index 1");
        let sig = module.words[0].poly.as_ref().expect("poly sig");
        assert_eq!(sig.bounds, target.bounds);
    }

    #[test]
    fn parse_impl_where_clause_length_var_is_error() {
        // 'N in array['T 'N] is a length variable, not in ty_var_names, so a
        // `where`-clause bound on it is an unknown-type-variable error.
        let err = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'N: Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("unknown type variable"), "{err}");
        assert!(err.contains("'N"), "{err}");
    }

    #[test]
    fn parse_impl_where_clause_no_where_keeps_bounds_empty() {
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        assert!(module.impls[0].target.bounds.is_empty());
        let sig = module.words[0].poly.as_ref().expect("poly sig");
        assert!(sig.bounds.is_empty());
    }

    #[test]
    fn parse_impl_where_clause_unknown_variable_is_error() {
        let err = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'X: Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("unknown type variable"), "{err}");
        assert!(err.contains("'X"), "{err}");
    }

    #[test]
    fn parse_impl_where_clause_missing_colon_is_error() {
        let err = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N] where 'T Show\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(err.contains("expected `:` after"), "{err}");
    }

    // ---- P7.S6 Phase 1: the named array type (R1/R1a/R1b/R2/R3) ----

    #[test]
    fn parse_poly_slot_named_array_parses() {
        let module = parse_src(": f ( array['T 'N] -- array['T 'N] ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert_eq!(sig.len_var_names, vec!["'N".to_string()]);
        assert!(
            matches!(&sig.inputs[0], PolyType::Array(e, Len::Var(0)) if **e == PolyType::Var(0)),
            "input should be array['T 'N]"
        );
        assert!(
            matches!(&sig.outputs[0], PolyType::Array(e, Len::Var(0)) if **e == PolyType::Var(0)),
            "output should be array['T 'N]"
        );
    }

    #[test]
    fn parse_poly_slot_nested_named_array_parses() {
        let module = parse_src(": f ( array[array['T 2] 3] -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(
            matches!(&sig.inputs[0], PolyType::Array(outer, Len::Concrete(3)) if matches!(&**outer, PolyType::Array(inner, Len::Concrete(2)) if **inner == PolyType::Var(0))),
            "input should be array[array['T 2] 3]"
        );
    }

    #[test]
    fn parse_slot_named_array_parses() {
        let module = parse_src(": f ( array[i64 4] -- ) drop ;").unwrap();
        assert!(
            matches!(module.words[0].effect.inputs[0].ty, Type::Array(_, _)),
            "input should be a Type::Array"
        );
    }

    #[test]
    fn parse_slot_named_array_with_type_annotation_parses() {
        // R1b: a slot *named* `array` (`array : i64`) needs no special-case
        // code. R1's dispatch predicate requires `array` followed by `[`, and
        // here the next token is `:`, so no dispatch is entered. This is a
        // plain regression test, not mutation-testable.
        let module = parse_src(": f ( array : i64 -- ) drop ;").unwrap();
        assert_eq!(
            module.words[0].effect.inputs[0].name.as_deref(),
            Some("array"),
            "`array` should be the slot name"
        );
    }

    #[test]
    fn parse_ref_type_expr_named_array_parses() {
        // R1a concrete path: `&array[i64 4]` -- `&` and `array` are glued
        // into one word, so the `[`-dispatch sites cannot reach this
        // spelling. `parse_ref_type_expr` intercepts `array` and dispatches
        // into `parse_array_type_expr`.
        let module = parse_src(": f ( &array[i64 4] -- ) drop ;").unwrap();
        assert!(
            matches!(module.words[0].effect.inputs[0].ty, Type::Ref(_, false, _)),
            "input should be a Type::Ref over an array"
        );
    }

    #[test]
    fn split_owning_cell_word_named_array_parses() {
        // R1a concrete path: `^array[i64 4]` -- same interception in
        // `split_owning_cell_word`.
        let module = parse_src(": f ( ^array[i64 4] -- ) drop ;").unwrap();
        assert!(
            matches!(module.words[0].effect.inputs[0].ty, Type::OwnedCell(_, _)),
            "input should be a Type::OwnedCell over an array"
        );
    }

    #[test]
    fn parse_poly_slot_ref_named_array_parses() {
        // R1a poly path: `&array['T 4]` inside a PolySig. The `&` and `array`
        // are glued, so `parse_poly_slot`'s `&` arm intercepts `array` and
        // dispatches into `parse_poly_array` (not the concrete array reader,
        // which cannot hold a type-variable element).
        let module = parse_src(": f ( &array['T 4] -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(
            matches!(&sig.inputs[0], PolyType::Ref(r, false) if matches!(&**r, PolyType::Array(e, Len::Concrete(4)) if **e == PolyType::Var(0))),
            "input should be &array['T 4]"
        );
    }

    #[test]
    fn parse_poly_slot_owned_cell_named_array_parses() {
        // R1a poly path: `^array['T 4]` inside a PolySig.
        let module = parse_src(": f ( ^array['T 4] -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(
            matches!(&sig.inputs[0], PolyType::OwnedCell(c) if matches!(&**c, PolyType::Array(e, Len::Concrete(4)) if **e == PolyType::Var(0))),
            "input should be ^array['T 4]"
        );
    }

    #[test]
    fn parse_generic_field_shape_ref_named_array_parses() {
        // R1a generic-field path: `&array['T 4]` in a generic struct field.
        // This is a regression test: today's `&array['T 4]` field spelling builds
        // via the bare-sigil recursion, so migrating to `array[…]` must not
        // break it. The co-assertion that `&array['T 4]` still builds is
        // phases 1–3 only.
        let named = sole_generic_field("type: Box['T] f &array['T 4] ;");
        assert!(
            matches!(&named, PolyType::Ref(r, false) if matches!(&**r, PolyType::Array(e, Len::Concrete(4)) if **e == PolyType::Var(0))),
            "field should be &array['T 4]"
        );
    }

    #[test]
    fn parse_generic_field_shape_owned_cell_named_array_parses() {
        // R1a generic-field path: `^array['T 4]` in a generic struct field.
        // Same rationale as the `&` twin above.
        let named = sole_generic_field("type: Box['T] f ^array['T 4] ;");
        assert!(
            matches!(&named, PolyType::OwnedCell(c) if matches!(&**c, PolyType::Array(e, Len::Concrete(4)) if **e == PolyType::Var(0))),
            "field should be ^array['T 4]"
        );
    }

    #[test]
    fn parse_generic_field_shape_bare_bracket_after_retirement_is_a_quotation_error() {
        // R1a: the successor to the two tests above's phases-1–3 co-assertion
        // that `&['T 4]` still built. After R4 a bare `[` is a quotation
        // effect, so that spelling is now rejected -- with a *pinned* message,
        // since "rejected" alone can hold for an unrelated upstream reason.
        //
        // Two arms, because `parse_generic_field_shape`'s ty-var scan
        // (`quotation_effect_ty_var_ahead`) runs ahead of the reader and so
        // fires ahead of R4a's validator.
        let with_var = parse_src("type: Box['T] f &['T 4] ;").unwrap_err();
        assert!(
            with_var.contains("quotation") && with_var.contains("'T"),
            "the declaration's own variable inside a quotation field: {with_var}"
        );
        let concrete = parse_src("type: Box['T] f &[i64 4] g 'T ;").unwrap_err();
        assert!(
            concrete.contains("must be written in full as `[ inputs -- outputs ]`"),
            "the concrete twin routes through R4a's validator: {concrete}"
        );
        assert!(concrete.contains("array[T N]"), "{concrete}");
    }

    #[test]
    fn parse_type_expr_array_without_bracket_is_error() {
        // R2: `array` in a type position with no following `[` is a located
        // error naming the required form, not "unknown type `array`".
        let err = parse_src(": f ( array -- ) drop ;").unwrap_err();
        assert!(err.contains("`array`"), "names the word: {err}");
        assert!(err.contains("`[T N]`"), "names the required form: {err}");
        assert!(
            !err.contains("unknown type"),
            "not an unknown-type error: {err}"
        );
    }

    #[test]
    fn parse_ref_type_expr_array_without_bracket_is_error() {
        // R2 via the `&` splitter: `&array` with no following `[` falls
        // through to `resolve_type_or_apply`, which is R2's single raise site.
        let err = parse_src(": f ( &array -- ) drop ;").unwrap_err();
        assert!(err.contains("`array`"), "names the word: {err}");
        assert!(err.contains("`[T N]`"), "names the required form: {err}");
        assert!(
            !err.contains("unknown type"),
            "not an unknown-type error: {err}"
        );
    }

    #[test]
    fn reject_reserved_name_array_type_is_error() {
        let err = parse_src("type: array x i64 ;").unwrap_err();
        assert!(
            err.contains("reserved") && err.contains("`array`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn reject_reserved_name_array_variant_is_error() {
        let err = parse_src("type: E | array | Other ;").unwrap_err();
        assert!(
            err.contains("reserved") && err.contains("`array`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn a_word_named_array_still_parses() {
        // R3: `array` is reserved against `type:`/variant names, not against
        // every use of the spelling. A word named `array` is legal.
        let module = parse_src(": array ( i64 -- i64 ) ;").unwrap();
        assert_eq!(module.words[0].name, "array");
    }

    #[test]
    fn parse_impl_target_named_array_parses() {
        // R8 (first half): `impl: Show for array['T 'N]` falls out of R1
        // through `parse_poly_slot`.
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array['T 'N]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        assert!(!module.impls[0].target.is_concrete());
        assert!(
            module.words[0].poly.is_some(),
            "generic impl member word should be polymorphic"
        );
    }

    // ---- P7.S6a Phase 1: R1 `Kind` enum, R2/R2.1/R2.2 header bracket ----

    #[test]
    fn parse_header_bracket_len_annotation_interns_a_length_variable() {
        let vars = header_bracket_vars("['T 'N: Len]").unwrap();
        assert_eq!(
            vars.iter()
                .map(|(n, _, k)| (n.clone(), k.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("'T".to_string(), Kind::Star),
                ("'N".to_string(), Kind::Len)
            ]
        );
    }

    #[test]
    fn parse_header_bracket_bare_var_defaults_to_star() {
        let vars = header_bracket_vars("['T]").unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].0, "'T");
        assert_eq!(vars[0].2, Kind::Star);
    }

    #[test]
    fn parse_header_bracket_unknown_kind_annotation_is_error() {
        let err = header_bracket_vars("['T 'N: Foo]").unwrap_err();
        assert!(err.contains("Len"), "{err}");
        assert!(err.contains("line 1, col"), "{err}");
    }

    #[test]
    fn parse_header_bracket_name_as_both_kinds_is_error() {
        let err = header_bracket_vars("['T 'T: Len]").unwrap_err();
        assert!(
            err.contains("both a type variable and a length variable"),
            "{err}"
        );
    }

    #[test]
    fn parse_header_bracket_length_only_is_error() {
        let err = header_bracket_vars("['N: Len]").unwrap_err();
        assert!(err.contains("no type variable"), "{err}");
    }

    // ---- P7b.S1 Phase 1: R1/R2/R5 kind-expression grammar and collection ----

    #[test]
    fn parse_header_bracket_kind_expr_annotations_parse() {
        let vars = header_bracket_vars("['F: * -> Len -> * 'T]").unwrap();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].0, "'F");
        assert_eq!(
            vars[0].2,
            Kind::Arrow {
                domains: vec![Kind::Star, Kind::Len],
                result: Box::new(Kind::Star),
            }
        );
        assert_eq!(vars[1].0, "'T");
        assert_eq!(vars[1].2, Kind::Star);
    }

    #[test]
    fn poly_sig_ty_kinds_stays_parallel_to_ty_var_names_on_real_parsed_output() {
        // S1-5: every published kind vector is length-matched to its name
        // table. A mixed signature (a plain var, an applied var, an
        // annotated var) exercises every path that can push onto either
        // vector.
        let module = parse_src(": f['F: * -> * 'T 'U] ( 'F['T] 'U -- ) drop drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names.len(), sig.ty_kinds.len());
        assert_eq!(sig.ty_var_names.len(), 3);
    }

    #[test]
    fn owned_cell_glued_sigil_bare_mention_after_application_is_located_error() {
        // P7b.S1 review fix: the `^'F` glued-sigil arm used to call
        // `parse_poly_ty_var` without ever reaching `mark_ty_star`, so a
        // bare mention behind `^` went uncollected and this built clean
        // instead of firing S1-15.b. `'F` is applied at `'F['T]` first
        // (kind `* -> *`), then used bare behind `^`.
        let err = parse_src(": bad['F 'T] ( 'F['T] ^'F -- 'F['T] ) drop ;").unwrap_err();
        assert!(err.contains("'F"), "{err}");
        assert!(
            err.contains("used as a plain type but has kind `* -> *`"),
            "{err}"
        );
        assert!(err.contains("line 1, col 24"), "{err}");
        assert!(err.contains("line 1, col 16"), "{err}");
    }

    #[test]
    fn ref_glued_sigil_bare_mention_after_application_is_located_error() {
        // The `&'F` twin of the `^'F` test above -- the same gap existed in
        // the `&`/`&!`-glued arm.
        let err = parse_src(": bad['F 'T] ( 'F['T] &'F -- 'F['T] ) drop ;").unwrap_err();
        assert!(err.contains("'F"), "{err}");
        assert!(
            err.contains("used as a plain type but has kind `* -> *`"),
            "{err}"
        );
        assert!(err.contains("line 1, col 24"), "{err}");
        assert!(err.contains("line 1, col 16"), "{err}");
    }

    #[test]
    fn parse_optional_bound_bracket_kind_and_bound_coexist() {
        // S1-9: a kind annotation on one variable (`'N: Len`) and a
        // capability bound on another (`'T: Copy`), in the same bracket.
        let module = parse_src(": f['T: Copy 'N: Len] ( array['T 'N] -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.has_bound(0, Bound::Copy));
        assert_eq!(sig.len_var_names, vec!["'N".to_string()]);
    }

    #[test]
    fn attach_bracket_bounds_annotation_conflicting_with_usage_names_both_spans() {
        // S1-15.c: `'F` is used bare (a plain type, `Star`) but annotated a
        // higher kind. The error must carry both the usage mention's span
        // and the annotation's own span.
        let err = parse_src(": bad['F: * -> * 'T] ( 'F 'T -- ) drop drop ;").unwrap_err();
        assert!(err.contains("'F"), "{err}");
        assert!(err.contains("used as a plain type"), "{err}");
        assert!(err.contains("* -> *"), "{err}");
        // The usage mention is `'F` in the effect (line 1, col 24); the
        // annotation is in the bound bracket (line 1, col 7).
        assert!(err.contains("line 1, col 24"), "{err}");
        assert!(err.contains("line 1, col 7"), "{err}");
    }

    #[test]
    fn attach_bracket_bounds_arrow_kind_unused_in_effect_is_accepted() {
        // P7b.S1: `'F: * -> *` is declared but never mentioned in the
        // effect at all -- there is no usage to compare the annotation
        // against, so the annotation alone is presence enough, permanently,
        // and this must parse rather than raise `bracket_var_unused_error`.
        let module = parse_src(": pass['F: * -> * 'T] ( 'T -- 'T ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
    }

    #[test]
    fn header_field_kind_collection_validated_at_decl_end() {
        // S1-5: a header declaring `'F` higher-kinded, then a field
        // bare-mentioning it -- a `Star`-only position -- is a located
        // conflict naming the declaration.
        let err = parse_src("type: Box['F: * -> * 'T] f 'F g 'T ;").unwrap_err();
        assert!(err.contains("'F"), "{err}");
        assert!(err.contains("Box"), "{err}");
        assert!(err.contains("* -> *"), "{err}");
    }

    #[test]
    fn header_field_kind_collection_accepts_a_star_kinded_var() {
        // The non-conflict twin: a `Star`-annotated (or unannotated) header
        // var used bare in a field is unaffected by the new side table.
        let module = parse_src("type: Box['F: * 'T] f 'F g 'T ;").unwrap();
        assert_eq!(module.generic_structs[0].fields.len(), 2);
    }

    /// P7b.S1 review fix (P1): the reversed field order from
    /// `hkt_header_field_applies_star_var_is_located_error`'s golden --
    /// the applied mention (`f 'F['T]`) comes *first*, establishing
    /// `Arrow`, and the bare mention (`g 'F`) comes second, conflicting
    /// with it. This is `check_field_bare_kind`'s `field_kind_marks`
    /// fallback arm (the header's own `ty_kinds` isn't published until decl
    /// end, so this arm -- not `header_ty_var_kind`'s early-return -- is
    /// what fires here), which used to render the established kind through
    /// a zero-domain `Arrow` (`kind_str` renders that as `*`), making the
    /// message assert the opposite of the fact that triggered it. Must now
    /// say `* -> *`, not `*`.
    #[test]
    fn header_field_bare_mention_after_an_earlier_application_names_the_real_arrow_kind() {
        let err = parse_src("type: Bad['F 'T] f 'F['T] g 'F ;").unwrap_err();
        assert!(
            err.contains("is declared kind `* -> *` in its header"),
            "{err}"
        );
    }

    /// P7b.S1 review fix (P1): `publish_field_inferred_kinds` must publish
    /// the field-application's *real* arity, not a zero-arity placeholder --
    /// otherwise S1-5's "no default-to-`Star` shortcut that would drop an
    /// `Arrow` on the floor" is honored in name only (an `Arrow` survives,
    /// but a dishonest one).
    #[test]
    fn publish_field_inferred_kinds_records_the_applications_real_arity() {
        let module = parse_src("type: Bad['F 'T 'U] f 'F['T 'U] ;").unwrap();
        let published = &module.generic_structs[0].ty_kinds[0];
        match published {
            Kind::Arrow { domains, .. } => assert_eq!(
                domains.len(),
                2,
                "'F['T 'U] applies two arguments; the published kind must say so"
            ),
            other => panic!("expected an Arrow kind, got {other:?}"),
        }
    }

    #[test]
    fn reject_reserved_name_rejects_trait_len() {
        let err = parse_src("trait: Len['T] : show ( 'T -- ) ; ;").unwrap_err();
        assert!(err.contains("reserved"), "{err}");
    }

    // ---- P7b.S1 Phase 2: S1-6/S1-7/S1-8 application parsing ----

    #[test]
    fn parse_poly_slot_router_application_before_quotation_parses() {
        // R4 lookahead router, order 1: `'F['T]` applies before a
        // separate quotation slot `[ 'T -- 'U ]` follows.
        let module = parse_src(": fmap['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- ) drop drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(sig.inputs[0], PolyType::App { head: 0, .. }));
        assert!(matches!(sig.inputs[1], PolyType::Quotation(..)));
    }

    #[test]
    fn parse_poly_slot_router_quotation_before_application_parses() {
        // R4 lookahead router, order 2: a quotation slot first, an
        // application afterwards -- both orders must route correctly.
        let module = parse_src(": fmap['F 'T 'U] ( [ 'T -- 'U ] 'F['T] -- ) drop drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(sig.inputs[0], PolyType::Quotation(..)));
        assert!(matches!(sig.inputs[1], PolyType::App { .. }));
    }

    #[test]
    fn parse_generic_field_shape_variable_application_parses_to_app() {
        // S1-8: a field `f 'F['T]` parses to `PolyType::App`.
        let module = parse_src("type: Box['F 'T] f 'F['T] ;").unwrap();
        assert!(matches!(
            module.generic_structs[0].fields[0].1,
            PolyType::App { head: 0, .. }
        ));
    }

    #[test]
    fn parse_type_argument_bare_constructor_parses_to_ctor_image() {
        // S1-8/S1-12: a use-site constructor type argument (`Wrap[Box i64]`)
        // parses to `Type::CtorImage`, which the field's own `App`
        // instantiation semantics (S1-8's stub) then resolves at
        // instantiation time -- minting `Box[i64]` as `Wrap`'s field.
        let module = parse_src(
            "type: Box['T] v 'T ;\ntype: Wrap['F 'T] w 'F['T] ;\n\
             : mk ( -- Wrap[Box i64] ) drop ;",
        )
        .unwrap();
        let word = &module.words[0];
        let ret = word.effect.outputs[0].ty;
        let Type::Struct(id, _) = ret else {
            panic!("expected a struct return, found {ret:?}")
        };
        let inst = &module.structs[id.index()];
        assert_eq!(inst.fields.len(), 1);
        let Type::Struct(field_id, field_name) = inst.fields[0].1 else {
            panic!(
                "expected `w`'s field to have folded to a struct, found {:?}",
                inst.fields[0].1
            )
        };
        assert_eq!(field_name, "Box[i64]");
        assert_eq!(module.structs[field_id.index()].fields[0].1, Type::I64);
    }

    #[test]
    fn parse_poly_var_application_quotation_argument_is_error() {
        // S1-6: an application argument is a type expression only -- a
        // quotation-shaped argument is a parse error.
        let err = parse_src(": f['F 'T] ( 'F[[ i64 -- i64 ]] -- ) drop ;").unwrap_err();
        assert!(err.contains("expected a type, found"), "{err}");
        assert!(err.contains('['), "{err}");
    }

    #[test]
    fn parse_poly_var_application_empty_is_error() {
        // S1-7: `'F[]` is a pinned arity error.
        let err = parse_src(": f['F 'T] ( 'F[] -- ) drop ;").unwrap_err();
        assert!(err.contains("'F[]"), "{err}");
        assert!(err.contains("zero arguments"), "{err}");
    }

    #[test]
    fn raw_to_poly_type_app_fold_has_no_all_concrete_fold() {
        // S1-7: unlike `Generic`, an `App` never folds to `Concrete` -- the
        // head names a variable, which never grounds to a `Type` at parse
        // time. Even a fully-concrete-looking argument list stays `App`.
        let module = parse_src(": f['F] ( 'F[i64] -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(sig.inputs[0], PolyType::App { head: 0, .. }));
    }

    // ---- P7.S6a Phase 2: R2a array-field sub-case, R2b word bound bracket ----

    #[test]
    fn parse_generic_type_header_with_length_parameter_parses() {
        // The exit fixture -- must fail before R2a's `'`-prefixed-token arm
        // exists in `parse_generic_field_array`.
        let module = parse_src("type: Buffer['T 'N: Len] data array['T 'N] ;").unwrap();
        let decl = &module.generic_structs[0];
        assert_eq!(decl.fields.len(), 1);
        assert_eq!(
            decl.fields[0].1,
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0))
        );
    }

    #[test]
    fn parse_generic_field_array_unbound_length_var_is_error() {
        // `'N` used in a field but never bound by the header bracket -- the
        // field-path twin of `unbound_generic_ty_var_error`.
        let err = parse_src("type: Buffer['T] data array['T 'N] ;").unwrap_err();
        assert!(err.contains("'N"), "{err}");
        assert!(err.contains("not a length variable"), "{err}");
    }

    #[test]
    fn parse_generic_typedef_phantom_length_var_is_error() {
        // `'N` bound but never used in any field's array count -- must fail
        // before `used_len` bookkeeping exists.
        let err = parse_src("type: Buffer['T 'N: Len] data 'T ;").unwrap_err();
        assert!(err.contains("'N"), "{err}");
        assert!(err.contains("phantom"), "{err}");
    }

    #[test]
    fn parse_generic_enum_variant_field_binds_a_length_variable() {
        // The enum twin of `parse_generic_type_header_with_length_parameter_parses`:
        // `parse_generic_variant_fields` threads `len_vars`/`used_len` too.
        let module =
            parse_src("type: Ring['T 'N: Len] | Full data array['T 'N] | Empty 'T ;").unwrap();
        let decl = &module.generic_enums[0];
        assert_eq!(
            decl.variants[0].fields[0].1,
            PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0))
        );
    }

    #[test]
    fn parse_generic_enum_typedef_phantom_length_var_is_error() {
        // The enum twin of `parse_generic_typedef_phantom_length_var_is_error`:
        // `check_no_phantom_len_var` is called from
        // `parse_generic_enum_typedef_variants` too, not only from the struct
        // field loop.
        let err = parse_src("type: Ring['T 'N: Len] | Full 'T | Empty 'T ;").unwrap_err();
        assert!(err.contains("'N"), "{err}");
        assert!(err.contains("phantom"), "{err}");
    }

    #[test]
    fn parse_generic_field_array_concrete_element_variable_length_does_not_fold_to_concrete() {
        // A concrete element type, a variable length, plus a `'T` field so
        // R2.1's type-variable-required check and `check_no_phantom_ty_var`
        // both clear: builds `PolyType::Array(Concrete(i64), Len::Var)`, not
        // `PolyType::Concrete(intern_array_type(..))`.
        let module = parse_src("type: Buffer['T 'N: Len] data 'T count array[i64 'N] ;").unwrap();
        let decl = &module.generic_structs[0];
        assert_eq!(decl.fields.len(), 2);
        assert_eq!(
            decl.fields[1].1,
            PolyType::Array(
                Box::new(PolyType::Concrete(Type::from_name("i64").unwrap())),
                Len::Var(0)
            )
        );
    }

    #[test]
    fn generic_field_type_str_renders_a_nested_length_variable_via_parse_error() {
        // P7.S6a (R2a): `generic_field_type_str`'s own `Array` arm is called
        // unconditionally to build `parse_array_count`'s error string, so a
        // nested length-carrying array element must render rather than
        // panic even when the outer count is malformed.
        let err = parse_src("type: Grid['T 'N: Len] rows array[array['T 'N] 0] ;").unwrap_err();
        assert!(err.contains("array['T 'N]"), "{err}");
    }

    // ---- P7.S6a Phase 3: R2a's parse_generic_field_application widening ----

    #[test]
    fn parse_generic_field_application_splits_type_and_length_args() {
        // `Buffer['T 'N]` inside `Pair`'s own field list -- `Buffer`'s
        // trailing `'N` resolves as a length variable against `Pair`'s own
        // header bracket, not an arity error.
        let module =
            parse_src("type: Buffer['T 'N: Len] data array['T 'N] ;\ntype: Pair['T 'N: Len] a Buffer['T 'N] ;")
                .unwrap();
        let pair = &module.generic_structs[1];
        assert_eq!(pair.fields.len(), 1);
        let PolyType::Generic {
            is_enum,
            args,
            len_args,
            ..
        } = &pair.fields[0].1
        else {
            panic!(
                "a nested header application stays PolyType::Generic: {:?}",
                pair.fields[0].1
            )
        };
        assert!(!is_enum);
        assert_eq!(*args, vec![PolyType::Var(0)]);
        assert_eq!(*len_args, vec![Len::Var(0)]);
    }

    #[test]
    fn parse_generic_field_application_concrete_type_variable_length_does_not_collapse() {
        // The field-application twin of R7's collapse-gate witness: a
        // concrete type argument, but a *variable* length, inside a field.
        // (A variable type with a concrete length, e.g. `Buffer['T 4]`, is
        // already caught by the pre-existing type-args-only concreteness
        // check and can't discriminate this length-concreteness gate at
        // all.) Without the gate this wrongly collapses to `PolyType::
        // Concrete` and panics at `substitute_generic_field`'s
        // `Array(_, Len::Var(v))` arm when the field is later substituted.
        let module = parse_src(
            "type: Buffer['T 'N: Len] data array['T 'N] ;\ntype: Pair['T 'N: Len] a Buffer[i64 'N] b 'T ;",
        )
        .unwrap();
        let pair = &module.generic_structs[1];
        let PolyType::Generic { args, len_args, .. } = &pair.fields[0].1 else {
            panic!(
                "a variable length argument must not collapse: {:?}",
                pair.fields[0].1
            );
        };
        assert_eq!(*args, vec![PolyType::Concrete(Type::I64)]);
        assert_eq!(*len_args, vec![Len::Var(0)]);
    }

    #[test]
    fn parse_optional_bound_bracket_len_annotation_validates_against_signature() {
        // R2b; the fixture is array-based per R2b's phase-1
        // self-containment note, not `Buffer`-based (which needs R7, landing
        // two phases later).
        let module =
            parse_src(": capacity['T 'N: Len] ( &array['T 'N] -- usize ) drop 0 ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.len_var_names, vec!["'N".to_string()]);
    }

    #[test]
    fn parse_optional_bound_bracket_len_annotation_unused_is_error() {
        // A bracket `['T 'N: Len]` on a word whose effect never mentions
        // `'N` in a length position -- the length-path twin of
        // `bracket_var_unused_error`.
        let err = parse_src(": f['T 'N: Len] ( 'T -- 'T ) ;").unwrap_err();
        assert!(err.contains("'N"), "{err}");
        assert!(err.contains("never appears in the effect"), "{err}");
    }

    // ---- P7.S6 Phase 2: R5/R5a/R5b/R6/R6a bracket binding sites ----

    #[test]
    fn parse_typedef_generic_header_brackets_parses() {
        // R5: `type: Box['T] val 'T ;` parses as a generic struct with one
        // type variable.
        let module = parse_src("type: Box['T] val 'T ;").unwrap();
        assert_eq!(module.generic_structs.len(), 1);
        assert_eq!(
            module.generic_structs[0].ty_var_names,
            vec!["'T".to_string()]
        );
    }

    #[test]
    fn parse_typedef_generic_header_brackets_enum_parses() {
        // R5: `type: Result['T 'E] | Ok 'T | Err 'E ;` parses as a generic enum.
        let module = parse_src("type: Result['T 'E] | Ok 'T | Err 'E ;").unwrap();
        assert_eq!(module.generic_enums.len(), 1);
        assert_eq!(
            module.generic_enums[0].ty_var_names,
            vec!["'T".to_string(), "'E".to_string()]
        );
    }

    #[test]
    fn parse_typedef_generic_header_empty_bracket_is_error() {
        let err = parse_src("type: Box[] val 'T ;").unwrap_err();
        assert!(err.contains("empty type-variable bracket"), "{err}");
    }

    #[test]
    fn parse_typedef_generic_header_duplicate_var_is_error() {
        let err = parse_src("type: Box['T 'T] val 'T ;").unwrap_err();
        assert!(err.contains("bound twice"), "{err}");
    }

    #[test]
    fn parse_typedef_generic_header_bare_name_is_concrete() {
        // R5: a bare name with no following `[` is a concrete (non-generic)
        // declaration, unchanged.
        let module = parse_src("type: Ordering | Less | Equal | Greater ;").unwrap();
        assert!(module.generic_structs.is_empty());
        assert!(module.generic_enums.is_empty());
        assert_eq!(module.enums.len(), 1);
        assert_eq!(module.enums[0].name, "Ordering");
    }

    #[test]
    fn parse_typedef_generic_header_non_var_token_in_bracket_is_error() {
        // R5: a non-`'`, non-`]` token inside a header bracket is a located
        // error naming the expected form.
        let err = parse_src("type: Box[i64] val 'T ;").unwrap_err();
        assert!(err.contains("expected a type variable"), "{err}");
    }

    #[test]
    fn header_is_generic_rejects_postfix_after_retirement() {
        // R10: the bracket form is the only generic spelling. Replaces
        // `header_is_generic_accepts_both_bracket_and_postfix_during_migration`,
        // whose dual acceptance was a phases-2–3 scaffold.
        let bracket_module = parse_src("type: Box['T] val 'T ;").unwrap();
        assert_eq!(bracket_module.generic_structs.len(), 1);
        assert_eq!(bracket_module.generic_structs[0].ty_var_names, ["'T"]);

        // The postfix form is no longer *classified* as concrete and left to
        // mis-parse: it is its own located error.
        let err = parse_src("type: Box 'T val 'T ;").unwrap_err();
        assert!(err.contains("retired postfix form"), "{err}");
        assert!(err.contains("type: Box['T]"), "{err}");
    }

    #[test]
    fn parse_typedef_postfix_header_var_is_error() {
        // R10: the located error reaches the struct production and the enum
        // production.
        let struct_err = parse_src("type: Box 'T val 'T ;").unwrap_err();
        assert!(struct_err.contains("retired postfix form"), "{struct_err}");
        assert!(struct_err.contains("type: Box['T]"), "{struct_err}");
        // A `'`-prefixed word is not merely rejected as a field name.
        assert!(
            !struct_err.contains("cannot be a field name"),
            "{struct_err}"
        );

        let enum_err = parse_src("type: Result 'T 'E | Ok 'T | Err 'E ;").unwrap_err();
        assert!(enum_err.contains("retired postfix form"), "{enum_err}");
        assert!(enum_err.contains("type: Result['T]"), "{enum_err}");
    }

    #[test]
    fn parse_trait_decl_bracket_header_parses() {
        // R5b: `trait: Ord['T]` parses with the bracketed header form.
        let module = parse_src("trait: Ord['T] : cmp ( &'T &'T -- i64 ) ; ;").unwrap();
        assert_eq!(module.traits.len(), 2, "Copy pre-seeded, plus Ord");
        let ord = module.traits.iter().find(|t| t.name == "Ord").unwrap();
        assert_eq!(ord.members.len(), 1);
    }

    #[test]
    fn parse_trait_decl_two_bracket_vars_is_error() {
        // R5b: a second variable inside the bracket keeps
        // `multi_variable_trait_error`.
        let err = parse_src("trait: Ord['T 'U] : cmp ( &'T &'T -- i64 ) ; ;").unwrap_err();
        assert!(err.contains("more than one type variable"), "{err}");
    }

    #[test]
    fn parse_trait_decl_with_neither_form_is_still_an_error() {
        // R5b: with neither form present the existing located error fires,
        // retargeted in message text to name `trait: Name['T]`.
        let err = parse_src("trait: Ord : cmp ( i64 i64 -- i64 ) ; ;").unwrap_err();
        assert!(err.contains("bracketed header"), "{err}");
        assert!(err.contains("trait: Ord['T]"), "{err}");
    }

    #[test]
    fn parse_trait_decl_postfix_header_var_is_error() {
        // R10: `parse_trait_decl` drops R5b's postfix disjunct. Replaces
        // `parse_trait_decl_accepts_both_bracket_and_postfix_during_migration`.
        // Distinct from the neither-form error above, which would wrongly
        // claim no type variable was written at all.
        let err = parse_src("trait: Ord 'T : cmp ( &'T &'T -- i64 ) ; ;").unwrap_err();
        assert!(err.contains("retired postfix form"), "{err}");
        assert!(err.contains("trait: Ord['T]"), "{err}");
        assert!(!err.contains("expected a type variable"), "{err}");
    }

    #[test]
    fn parse_worddef_bound_bracket_parses() {
        // R6: `: f['T: Copy] ( 'T -- 'T )` parses with the bound bracket.
        let module =
            parse_src("trait: Show['T] : show ( &'T -- ) ; ;\n: f['T: Copy] ( 'T -- 'T ) ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.ty_var_names, vec!["'T".to_string()]);
        assert!(sig.has_bound(0, Bound::Copy));
    }

    #[test]
    fn parse_worddef_bound_bracket_after_inline_parses() {
        // R6: the bracket sits after `inline` and before `(`.
        let module = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n: f inline ['T: Copy] ( 'T -- 'T ) ;",
        )
        .unwrap();
        assert!(module.words[0].declares_inline);
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.has_bound(0, Bound::Copy));
    }

    #[test]
    fn parse_worddef_bound_bracket_var_unused_in_effect_is_error() {
        // R6: a bracket-declared variable that never appears in the effect is
        // a located error.
        let err = parse_src(": f['T: Copy] ( i64 -- i64 ) ;").unwrap_err();
        assert!(err.contains("never appears in the effect"), "{err}");
        assert!(err.contains("'T"), "{err}");
    }

    #[test]
    fn parse_worddef_bound_bracket_var_id_order_follows_effect() {
        // R6: ids stay effect-derived. The bracket must not pre-intern its
        // variables; `ty_var_names` order follows the effect's first-mention
        // order, not the bracket's declaration order.
        let module = parse_src(": f['U: Copy 'T: Copy] ( 'T 'U -- 'T 'U ) ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        // Effect mentions 'T first, then 'U, so id 0 is 'T and id 1 is 'U --
        // even though the bracket declares 'U first.
        assert_eq!(sig.ty_var_names, vec!["'T".to_string(), "'U".to_string()]);
        assert!(sig.has_bound(0, Bound::Copy));
        assert!(sig.has_bound(1, Bound::Copy));
    }

    #[test]
    fn parse_worddef_bound_bracket_preserves_effect_arity() {
        // R6: moving a bound into the bracket never removes a slot. The
        // bound-bearing occurrence is a stack slot and stays one.
        // `: eq['T: Ord] ( 'T 'T -- Bool )` has two inputs (before and after).
        let module = parse_src(
            "type: Bool | False | True ;\ntrait: Ord['T] : cmp ( &'T &'T -- i64 ) ; ;\n: eq['T: Ord] ( 'T 'T -- Bool ) ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert_eq!(sig.inputs.len(), 2, "two inputs preserved");
        assert_eq!(sig.outputs.len(), 1);
    }

    #[test]
    fn parse_bound_bracket_multiple_bound_vars_parse() {
        // R6a: `['T: Copy 'U: Ord]` parses as two var_decls.
        let module = parse_src(
            "trait: Ord['T] : cmp ( &'T &'T -- i64 ) ; ;\n: f['T: Copy 'U: Ord] ( 'T 'U -- 'T ) ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.has_bound(0, Bound::Copy));
        assert!(sig.has_bound(1, Bound::User(TraitId::from_index(1))));
    }

    #[test]
    fn parse_bound_bracket_unbounded_var_parses() {
        // R6a: `['T 'U: Ord]` is legal -- `'T` unbounded, declared for
        // documentation, but it must still appear in the effect.
        let module = parse_src(
            "trait: Ord['T] : cmp ( &'T &'T -- i64 ) ; ;\n: f['T 'U: Ord] ( 'T 'U -- 'T ) ;",
        )
        .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(!sig.has_bound(0, Bound::Copy));
        assert!(sig.has_bound(1, Bound::User(TraitId::from_index(1))));
    }

    #[test]
    fn parse_bound_bracket_empty_is_error() {
        // R6: an empty bound bracket is a located error.
        let err = parse_src(": f[] ( i64 -- i64 ) ;").unwrap_err();
        assert!(err.contains("empty bound bracket"), "{err}");
    }

    #[test]
    fn parse_bound_bracket_non_var_token_is_error() {
        // R6: a non-`'`, non-`]` token inside a bound bracket is a located error.
        let err = parse_src(": f[i64] ( i64 -- i64 ) ;").unwrap_err();
        assert!(err.contains("expected a type variable"), "{err}");
    }

    #[test]
    fn skip_typedef_with_bracket_header_skips_whole_decl() {
        // R5: `skip_typedef` and the pipe/variant scans must remain correct
        // with header brackets present. The bracket contains only
        // `'`-prefixed words, so no Pipe/Semicolon enters the scanned range.
        // This test verifies a bracketed generic `type:` header is correctly
        // skipped by `parse_generic_typedefs` and doesn't leave a residue that
        // breaks the body pass.
        let module = parse_src("type: Box['T] val 'T ;\n: f ( i64 -- i64 ) 1 add ;").unwrap();
        assert_eq!(module.generic_structs.len(), 1);
        assert_eq!(module.words.len(), 1);
        assert_eq!(module.words[0].name, "f");
    }

    #[test]
    fn enum_detection_with_bracket_header_is_correct() {
        // R5: a bracketed generic enum header is correctly classified as an
        // enum (the pipe/variant scan must not be confused by the bracket).
        let module =
            parse_src("type: Result['T 'E] | Ok 'T | Err 'E ;\n: f ( i64 -- i64 ) ;").unwrap();
        assert_eq!(module.generic_enums.len(), 1);
        assert_eq!(module.generic_enums[0].ty_var_names.len(), 2);
        assert_eq!(module.words.len(), 1);
    }

    #[test]
    fn parse_worddef_bound_bracket_with_user_trait_bound_parses() {
        // R6: a user-trait bound inside the bracket resolves at parse time.
        let module =
            parse_src("trait: Show['T] : show ( &'T -- ) ; ;\n: f['T: Show] ( 'T -- 'T ) ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.has_bound(0, Bound::User(TraitId::from_index(1))));
    }

    #[test]
    fn parse_worddef_bound_bracket_glued_colon_parses() {
        // R6a: the `:` may be glued (`'T:`) or spaced, exactly as in the
        // in-effect bound syntax.
        let module =
            parse_src("trait: Show['T] : show ( &'T -- ) ; ;\n: f['T: Copy] ( 'T -- 'T ) ;")
                .unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(sig.has_bound(0, Bound::Copy));
    }

    #[test]
    fn parse_trait_member_bound_bracket_parses() {
        // R6: the same bracket is admitted on a trait member declaration, in
        // the same slot relative to its own `inline` peek.
        let module = parse_src("trait: Show['T] : show ['T: Copy] ( 'T -- ) ; ;").unwrap();
        let show = module.traits.iter().find(|t| t.name == "Show").unwrap();
        let sig = &show.members[0].sig;
        assert!(sig.has_bound(0, Bound::Copy));
    }

    #[test]
    fn generic_field_type_str_renders_named_array() {
        // R8b: `generic_field_type_str` renders `array[...]` (not `[...]`)
        // for an array type, so a generic struct field's surface spelling is
        // copy-pasteable source.
        let ty_vars = [("'T".to_string(), Span::default())];
        let len_vars: [(String, Span); 0] = [];
        let arr = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        assert_eq!(
            generic_field_type_str(&arr, &ty_vars, &len_vars),
            "array['T 4]"
        );
        // A nested array also picks up the new spelling.
        let nested = PolyType::Array(Box::new(arr), Len::Concrete(2));
        assert_eq!(
            generic_field_type_str(&nested, &ty_vars, &len_vars),
            "array[array['T 4] 2]"
        );
    }

    #[test]
    fn generic_field_type_str_renders_a_nested_length_variable() {
        // P7.S6a (R2a): the pre-fix `Len::Var(_) => unreachable!()` arm is
        // reachable unconditionally from `parse_generic_field_array`'s own
        // error-string construction, not only on a parse error -- a nested
        // array field whose *element* is itself a length-carrying array must
        // render, not panic, naming the length variable by its surface
        // spelling exactly as the `Var` arm already renders a type variable.
        let ty_vars = [("'T".to_string(), Span::default())];
        let len_vars = [
            ("'N".to_string(), Span::default()),
            ("'M".to_string(), Span::default()),
        ];
        let inner = PolyType::Array(Box::new(PolyType::Var(0)), Len::Var(0));
        let outer = PolyType::Array(Box::new(inner), Len::Var(1));
        assert_eq!(
            generic_field_type_str(&outer, &ty_vars, &len_vars),
            "array[array['T 'N] 'M]"
        );
    }

    #[test]
    fn generic_field_type_str_renders_generic_len_args() {
        // P7.S6a (R3): the `Generic` arm's `len_args` widening -- a
        // length-carrying generic field type must render its length
        // component (by surface spelling for a `Var`, literally for a
        // `Concrete`) after the type args, not print `Buffer['T]` for
        // `Buffer['T 'N]`.
        let ty_vars = [("'T".to_string(), Span::default())];
        let len_vars = [("'N".to_string(), Span::default())];
        let buffer = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![Len::Var(0), Len::Concrete(4)],
            name: "Buffer",
        };
        assert_eq!(
            generic_field_type_str(&buffer, &ty_vars, &len_vars),
            "Buffer['T 'N 4]"
        );
    }

    #[test]
    fn parse_poly_slot_bare_bracket_is_quotation() {
        // R4: an array-shaped bare bracket in a poly slot is a quotation
        // effect now, not an array -- so it is R4a's missing-`--` error.
        let err = parse_src(": f ( [ 'T 4 ] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("must be written in full as `[ inputs -- outputs ]`"),
            "{err}"
        );
        assert!(err.contains("array[T N]"), "{err}");
        // And the named spelling in the same position still parses as an array.
        let module = parse_src(": f ( array[ 'T 4 ] -- ) drop ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(matches!(
            sig.inputs[0],
            PolyType::Array(_, Len::Concrete(4))
        ));
    }

    #[test]
    fn parse_quotation_effect_missing_arrow_is_error() {
        // R4a, concrete reader, depth base 0, opener `[`. The message *does*
        // name `array[T N]`: a plain `[` is exactly where an author who meant
        // an array lands.
        let err = parse_src(": f ( [ i64 4 ] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("must be written in full as `[ inputs -- outputs ]`"),
            "{err}"
        );
        assert!(
            err.contains("array[T N]"),
            "a plain `[` opener gets the array advice: {err}"
        );
        // Located at the opening bracket, not at EOF.
        assert!(err.contains("line 1, col 7"), "{err}");
    }

    #[test]
    fn parse_poly_quotation_missing_arrow_is_error() {
        // R4a(iii), poly reader, depth base 1, opener `~[`. `~[` has no array
        // reading anywhere in the grammar (every type-position reader rejects
        // a bare `Token::TildeLBracket`), so the advice must NOT offer
        // `array[T N]` -- that would send the author somewhere the parser
        // refuses. Pinned to a different opener than the base-0 test above so
        // both entry points are covered independently.
        let err = parse_src(": f ( ~[ i64 4 ] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("must be written in full as `[ inputs -- outputs ]`"),
            "{err}"
        );
        assert!(
            !err.contains("array[T N]"),
            "a `~[` opener must not be offered the array spelling: {err}"
        );
        // Located at the `~[`, which the base-1 caller has already consumed,
        // so the span comes from behind the cursor rather than at it.
        assert!(err.contains("line 1, col 7"), "{err}");
    }

    #[test]
    fn parse_poly_quotation_legal_inline_effect_still_parses() {
        // R4a(i)'s guard: `parse_poly_quotation_inner` is entered *past* its
        // opener by all three of its callers, so the validator must be seeded
        // at depth base 1. Seeded at 0 this legal `~[ 'T -- Bool ]` meets its
        // closing `]` first, falls to -1, never satisfies the `depth == 0`
        // stop, runs to EOF and is false-rejected -- as would every inline
        // combinator in `lib/combinators.sth`.
        let module = parse_src(": f inline ( 'T ~[ 'T -- i64 ] -- i64 ) call ;").unwrap();
        let sig = module.words[0].poly.as_ref().expect("poly sig present");
        assert!(
            matches!(&sig.inputs[1], PolyType::Quotation(ins, outs, true, _, _)
                if ins.len() == 1 && outs.len() == 1),
            "the inline quotation parameter should survive: {:?}",
            sig.inputs[1]
        );
        // The `owning [ ... ]` and plain `[ ... ]` openers reach the same
        // base-1 site; neither may be false-rejected either.
        parse_src(": g ( [ i64 -- i64 ] 'T -- 'T ) drop ;").expect("a plain poly quotation slot");
    }

    #[test]
    fn require_top_depth_arrow_counts_a_nested_tilde_bracket() {
        // R4a(ii): `Token::TildeLBracket` is a single token that opens a
        // bracket. A walk that counts only `Token::LBracket` fails *open*
        // here -- the inner `~[`'s `--` is seen at depth 1 and the outer
        // bracket passes vacuously, then dies further down with a worse
        // diagnostic. The author meant `array[ ~[ i64 -- i64 ] 4 ]`, and the
        // outer opener is a plain `[`, so the array advice is present and
        // correct.
        let err = parse_src(": f ( [ ~[ i64 -- i64 ] 4 ] -- ) drop ;").unwrap_err();
        assert!(
            err.contains("must be written in full as `[ inputs -- outputs ]`"),
            "{err}"
        );
        assert!(err.contains("array[T N]"), "{err}");
        // The all-`[` twin, which a counter blind to `~[` gets right anyway.
        let plain = parse_src(": f ( [ [ i64 -- i64 ] 4 ] -- ) drop ;").unwrap_err();
        assert!(
            plain.contains("must be written in full as `[ inputs -- outputs ]`"),
            "{plain}"
        );
        // And the named spelling parses, so the fixture's only defect is the
        // missing `array`.
        parse_src(": f ( array[ ~[ i64 -- i64 ] 4 ] -- ) drop ;")
            .expect("the named array of inline quotations parses");
    }

    #[test]
    fn poly_type_shape_str_renders_old_spelling_by_exemption() {
        // R8b: `poly_type_shape_str` is the ruled-on exemption — it renders
        // `[...]` (not `array[...]`) because it keys synthesized member word
        // names, a compiler-internal spelling never shown to the user. Assert
        // *no* change so a well-meaning sweep cannot quietly rename synthesized
        // member words.
        let arr = PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(4));
        assert_eq!(poly_type_shape_str(&arr), "['T0 4]");
    }

    #[test]
    fn poly_type_shape_str_renders_generic_len_args() {
        // P7.S6a (R3): the `Generic` arm's `len_args` widening -- a
        // length-carrying generic application must print its length
        // component after the type args, not silently drop it (`Buffer[i64]`
        // for `Buffer[i64 256]` would misname the very arity/overlap errors
        // this slice's distinct-monomorph feature is likely to trigger).
        let buffer = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Concrete(Type::I64)],
            len_args: vec![Len::Concrete(256), Len::Var(0)],
            name: "Buffer",
        };
        assert_eq!(poly_type_shape_str(&buffer), "Buffer[i64 256 'N0]");
    }

    // ---- P7b.S2 Phase 2: target and member-word construction ----

    /// S2-4: `for Box` desugars to the ctor applied to one fresh pattern
    /// variable per declared slot, named `'ctor{slot}` and spanning the ctor
    /// name; the user's spelling rides along.
    #[test]
    fn parse_impl_target_bare_ctor_desugars_to_fresh_vars_and_user_spelling() {
        let module = parse_src(
            "type: Box['T] v 'T ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for Box\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        let target = &module.impls[0].target;
        assert!(!target.is_concrete());
        let PolyType::Generic { args, name, .. } = &target.pattern else {
            panic!("a desugared bare ctor target stays Generic")
        };
        assert_eq!(*name, "Box");
        assert_eq!(args.len(), 1, "one fresh var per declared type slot");
        assert_eq!(target.ty_var_names, vec!["'ctor0".to_string()]);
        // The fresh variable's introduction span is the ctor name's own.
        assert_eq!(
            target.ty_var_spans[0],
            Span {
                line: 3,
                col: 16,
                ..Span::default()
            }
        );
        assert_eq!(
            target.user_spelling,
            Some((
                "Box".to_string(),
                Span {
                    line: 3,
                    col: 16,
                    ..Span::default()
                }
            ))
        );
    }

    /// S2-4: `for Result[i64]` pads only the missing slots -- the explicit
    /// prefix stays `Concrete`, the fresh var fills slot 1, and the user's
    /// spelling renders the prefix (`Result[i64]`).
    #[test]
    fn parse_impl_target_partial_ctor_binds_explicit_prefix() {
        let module = parse_src(
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for Result[i64]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap();
        let target = &module.impls[0].target;
        let PolyType::Generic { args, .. } = &target.pattern else {
            panic!("a partially-applied ctor target stays Generic")
        };
        assert_eq!(args[0], PolyType::Concrete(Type::I64), "the prefix pins");
        assert_eq!(args[1], PolyType::Var(0), "the remaining slot desugars");
        assert_eq!(target.ty_var_names, vec!["'ctor1".to_string()]);
        assert_eq!(
            target.user_spelling.as_ref().map(|(s, _)| s.as_str()),
            Some("Result[i64]")
        );
    }

    /// S2-5: an HKT member's word sig unions the target's variables (ids and
    /// order kept) with its own locals appended, every variable carrying its
    /// introduction span, and the target's `where`-bounds surviving keyed by
    /// their unchanged target ids. `'F['T]`'s `'T` is identified with the
    /// target slot; `'U` appends.
    #[test]
    fn parse_impl_member_hkt_sig_unions_target_vars_and_locals() {
        let module = parse_src(
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             trait: Functor['F: * -> *] :\n\
               map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Result['R 'E2] where 'R: Copy\n\
               : map | x | x drop ;\n\
             ;",
        )
        .unwrap();
        let word = module
            .words
            .iter()
            .find(|w| w.name.starts_with("map;"))
            .expect("the desugar splices the member word");
        let sig = word
            .poly
            .as_deref()
            .expect("a generic member is polymorphic");
        // Union id space: target vars first ('R, 'E2), then the appended
        // locals that no dispatchable-input argument identified ('U; 'T was
        // identified with target slot 0).
        assert_eq!(sig.ty_var_names, vec!["'R", "'E2", "'U"]);
        assert_eq!(sig.ty_kinds, vec![Kind::Star, Kind::Star, Kind::Star]);
        // Spans: target vars carry their target introduction spans, the
        // appended local its member-sig span ('U's first mention, in the
        // member's own declaration).
        assert_eq!(
            sig.ty_var_spans[0],
            Span {
                line: 5,
                col: 26,
                ..Span::default()
            }
        );
        assert_eq!(
            sig.ty_var_spans[2],
            Span {
                line: 3,
                col: 22,
                ..Span::default()
            }
        );
        // The where-bound survives, keyed by the target var's unchanged id.
        assert_eq!(sig.bounds.len(), 1);
        assert_eq!(sig.bounds[0].0, 0, "'R keeps id 0");
        // The dispatchable input grounds to the whole target pattern; the
        // output displaces slot 0 with 'U and keeps the leftover slot.
        assert_eq!(sig.inputs[0], module.impls[0].target.pattern);
        let PolyType::Generic { args, .. } = &sig.outputs[0] else {
            panic!("the output grounds to the ctor")
        };
        assert_eq!(args[0], PolyType::Var(2), "'U displaces the slot");
        assert_eq!(args[1], PolyType::Var(1), "the leftover slot flows through");
    }

    /// S2-5: a member local that no dispatchable-input argument identifies
    /// must not reuse a target variable's name -- a located desugar error
    /// (outside the S2-15 family).
    #[test]
    fn parse_impl_member_local_colliding_with_target_var_name_is_error() {
        let err = parse_src(
            "type: Result['T 'E] | Ok 'T | Err 'E ;\n\
             trait: Functor['F: * -> *] :\n\
               map ( 'F['T] [ 'T -- 'E ] -- 'F['E] ) ;\n\
             ;\n\
             impl: Functor for Result['R 'E]\n\
               : map | x | x drop ;\n\
             ;",
        )
        .unwrap_err();
        // 'T is identified (slot 0); 'E is not, and 'E is the target's
        // second variable's name -- the collision.
        assert!(
            err.contains("declares local `'E`") && err.contains("the impl target's variable `'E`"),
            "{err}"
        );
    }

    /// S2-6: an App-headed member against a *concrete* target is a located
    /// error (no mono representation for member locals), raised at the
    /// desugar before `ground_member_type` could see it.
    #[test]
    fn parse_impl_member_app_against_concrete_target_is_located_error() {
        let err = parse_src(
            "type: Option['T] | None | Some 'T ;\n\
             trait: Functor['F: * -> *] :\n\
               map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for Option[i64]\n\
               : map | x | x drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains(
                "trait member `map` of `Functor` (line 6, col 3) applies the trait variable \
                 `'F`, but the impl target `Option[i64]` is concrete"
            ),
            "{err}"
        );
    }

    /// P7b.S2 review (S2-4): the ctor desugar's `'ctor…` name prefixes are
    /// reserved inside an `impl:` target. A user variable so named would
    /// alias a padded slot (the pad interns by name, so on the 2-slot
    /// `Pair` the user's `'ctor1` in slot 0 and the pad for slot 1 would
    /// collapse into one variable standing for two slots, silently) and
    /// would be misread as desugar padding by the user-spelling renderer.
    /// All four user spellings reject, located at the offending variable.
    #[test]
    fn parse_impl_target_reserved_ctor_var_names_are_error() {
        // The aliasing proof shape: a user-written `'ctor1` in a partially
        // applied 2-slot ctor target.
        let err = parse_src(
            "type: Pair['A 'B] | P 'A 'B ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for Pair[i64 'ctor1]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("may not declare the variable `'ctor1` at line 3, col 25"),
            "{err}"
        );
        assert!(
            err.contains("reserved prefix `'ctor`"),
            "the message must name the reserved prefix: {err}"
        );
        assert!(
            err.contains("desugar's own fresh pattern variables"),
            "the message must say why: {err}"
        );
        // The fully-applied spelling the renderer used to misread as
        // desugar padding.
        let err = parse_src(
            "type: Box['T] v 'T ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for Box['ctor0]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("may not declare the variable `'ctor0` at line 3, col 20"),
            "{err}"
        );
        // A length-slot spelling inside the target's own array shape.
        let err = parse_src(
            "trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for array[i64 'ctorlen0]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("may not declare the variable `'ctorlen0` at line 2, col 26"),
            "{err}"
        );
        // And inside a ctor application's length slots.
        let err = parse_src(
            "type: Buf['T 'N: Len] d array['T 'N] ;\n\
             trait: Show['T] : show ( &'T -- ) ; ;\n\
             impl: Show for Buf[i64 'ctorlen0]\n\
               : show | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains("may not declare the variable `'ctorlen0` at line 3, col 24"),
            "{err}"
        );
    }

    /// P7b.S2 review (S2-4): the user-spelling renderer's contract -- `Some`
    /// only when the desugar actually padded. A fully-applied ctor pattern
    /// (however its variables happen to be named) is already the user's own
    /// spelling, so the display must render it in full (`Box['ctor0]` via
    /// the `None` fallback), never collapse to the bare ctor name. The
    /// reserved-name rejection keeps a user-typed `'ctor0` out of real
    /// targets; this pins the renderer structurally.
    #[test]
    fn impl_target_user_spelling_fully_applied_pattern_is_not_desugared() {
        let span = Span {
            line: 3,
            col: 16,
            ..Span::default()
        };
        let fully_applied = PolyType::Generic {
            is_enum: false,
            idx: 0,
            module: 0,
            args: vec![PolyType::Var(0)],
            len_args: vec![],
            name: "Box",
        };
        let ctor_named = vec!["'ctor0".to_string()];
        // Nothing was desugared: user_spelling is None, so diagnostics fall
        // back to the full render -- "Box['ctor0]", never the bare "Box".
        assert_eq!(
            impl_target_user_spelling(&fully_applied, &ctor_named, &[], (0, 0), span),
            None
        );
        assert_eq!(
            render_target_pt(&fully_applied, &ctor_named, &[]),
            "Box['ctor0]"
        );
        // The desugar DID pad the one slot: the spelling is the bare ctor
        // name, the fresh variable omitted.
        assert_eq!(
            impl_target_user_spelling(&fully_applied, &ctor_named, &[], (1, 0), span),
            Some(("Box".to_string(), span))
        );
    }

    /// P7b.S2 review (S2-5 ordering): for a fully-abstract (`Var`) target,
    /// a member local whose name matches the target variable's must surface
    /// the S2-15.e abstract-target error -- the real problem -- not the S2-5
    /// name-collision error the append loop would otherwise raise first.
    #[test]
    fn parse_impl_member_app_against_var_target_is_abstract_target_error() {
        let err = parse_src(
            "trait: Functor['F: * -> *] :\n\
               map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
             ;\n\
             impl: Functor for 'T\n\
               : map | x | x drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains(
                "trait member `map` of `Functor` (line 5, col 3) applies the trait variable \
                 `'F`, but the impl target `'T` at line 4, col 19 is not a constructor"
            ),
            "{err}"
        );
        // The S2-5 collision diagnostic must not win the ordering race.
        assert!(!err.contains("the impl target's variable"), "{err}");
        assert!(!err.contains("declares local"), "{err}");
    }

    /// P7b.S2 (S2-5): one member-local name bound at two different target
    /// slots across the dispatchable inputs -- the sig text would claim the
    /// local aliases two slots at once; refused rather than silently
    /// last-write-wins. The two slots render over the target's own name
    /// tables (P7b.S2 review: this arm was the only union error without a
    /// unit test).
    #[test]
    fn parse_impl_member_local_reidentified_across_slots_is_error() {
        let err = parse_src(
            "type: Pair['A 'B] | P 'A 'B ;\n\
             trait: Dist['F: * -> * -> *] :\n\
               both ( 'F['T 'T] -- ) ;\n\
             ;\n\
             impl: Dist for Pair['X 'Y]\n\
               : both | a | a drop ;\n\
             ;",
        )
        .unwrap_err();
        assert!(
            err.contains(
                "trait member `both` of `Dist` (line 6, col 3) binds the same local name to \
                 two different target slots (`'X` and `'Y`)"
            ),
            "{err}"
        );
        assert!(
            err.contains(
                "a name in an identifying position binds one slot for the whole signature"
            ),
            "{err}"
        );
    }

    // ---- Named-slot-locals sugar (Phase 1): desugar shapes ----

    #[test]
    fn parse_worddef_slot_sugar_prepends_bind_in_slot_order_expected() {
        let module = parse_src(": f ( a: i64 b: i64 -- i64 ) a b add ;").unwrap();
        assert!(
            matches!(&module.words[0].body[0].kind, TermKind::Bind(names) if names == &vec!["a".to_string(), "b".to_string()]),
            "expected a leading `Bind([\"a\", \"b\"])`, got {:?}",
            module.words[0].body[0].kind
        );
    }

    #[test]
    fn parse_worddef_slot_sugar_top_contiguous_named_zero_mints_expected() {
        let module = parse_src(": f ( i64 a: i64 b: i64 -- i64 ) a b add ;").unwrap();
        assert!(
            matches!(&module.words[0].body[0].kind, TermKind::Bind(names) if names == &vec!["a".to_string(), "b".to_string()]),
            "the named slots are the top-contiguous run: zero mints, got {:?}",
            module.words[0].body[0].kind
        );
        // No re-push `Call` before the original body: the second term is the
        // user's own first body term.
        assert!(
            matches!(&module.words[0].body[1].kind, TermKind::Call(name, ..) if name == "a"),
            "expected zero mint re-pushes, got {:?}",
            module.words[0].body[1].kind
        );
    }

    #[test]
    fn parse_worddef_slot_sugar_out_of_order_mints_and_repushes_expected() {
        let module = parse_src(": f ( a: i64 i64 -- i64 ) | b | a b add ;").unwrap();
        assert!(
            matches!(&module.words[0].body[0].kind, TermKind::Bind(names) if names == &vec!["a".to_string(), "__slot1".to_string()]),
            "expected the unnamed slot to mint `__slot1`, got {:?}",
            module.words[0].body[0].kind
        );
        assert!(
            matches!(&module.words[0].body[1].kind, TermKind::Call(name, ..) if name == "__slot1"),
            "expected an immediate re-push of the mint, got {:?}",
            module.words[0].body[1].kind
        );
    }

    #[test]
    fn parse_worddef_slot_sugar_mint_bumped_on_body_collision_expected() {
        let module = parse_src(": f ( a: i64 i64 -- i64 ) | __slot1 | a __slot1 add ;").unwrap();
        assert!(
            matches!(&module.words[0].body[0].kind, TermKind::Bind(names) if names == &vec!["a".to_string(), "__slot2".to_string()]),
            "the user body already binds `__slot1`, so the mint should bump to `__slot2`, got {:?}",
            module.words[0].body[0].kind
        );
    }

    #[test]
    fn parse_worddef_slot_sugar_leaves_slot_names_populated_expected() {
        let module = parse_src(": f ( a: i64 i64 -- i64 ) | b | a b add ;").unwrap();
        assert_eq!(module.words[0].effect.inputs[0].name.as_deref(), Some("a"));
        assert_eq!(module.words[0].effect.inputs[1].name, None);
    }

    #[test]
    fn parse_slot_degenerate_double_colon_falls_through_to_unknown_type_expected() {
        let err = parse_src(": f ( :: i64 -- ) drop ;").unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
    }

    #[test]
    fn parse_slot_qualified_name_shaped_glued_colon_falls_through_to_unknown_type_expected() {
        let err = parse_src(": f ( q::Point: i64 -- ) drop ;").unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
    }

    #[test]
    fn parse_slot_leading_colon_glued_falls_through_to_unknown_type_expected() {
        let err = parse_src(": f ( :i64 -- ) drop ;").unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
    }

    #[test]
    fn parse_slot_glued_numeric_name_half_is_error() {
        let err = parse_src(": f ( 1: i64 -- i64 ) 5 ;").unwrap_err();
        assert!(
            err.contains("`1:` reads as a slot named `1`"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("not a name a body block could bind"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_slot_glued_float_name_half_is_error() {
        let err = parse_src(": f ( 1.5: i64 -- i64 ) 5 ;").unwrap_err();
        assert!(
            err.contains("`1.5:` reads as a slot named `1.5`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_slot_glued_negative_int_name_half_is_error() {
        let err = parse_src(": f ( -1: i64 -- i64 ) 5 ;").unwrap_err();
        assert!(
            err.contains("`-1:` reads as a slot named `-1`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_slot_glued_line_comment_name_half_is_error() {
        // A standalone `\` re-lexes to a line comment (zero tokens), not a
        // `Word` -- the twin `| \ |` is a parse error and a bare `\` in a
        // body comments out the rest of the line, so the name half can never
        // be spelled or referenced.
        let err = parse_src(": f ( \\: i64 -- i64 ) drop ;").unwrap_err();
        assert!(
            err.contains("`\\:` reads as a slot named `\\`"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn parse_slot_glued_ordinary_name_still_parses_expected() {
        // Control: an ordinary glued name still re-lexes to exactly one
        // `Word` token, so the gate leaves it alone.
        let module = parse_src(": f ( a: i64 -- i64 ) a ;").unwrap();
        assert_eq!(module.words[0].effect.inputs[0].name.as_deref(), Some("a"));
    }
}
