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
    ArrayDecl, Clause, EnumDecl, ExternDecl, Line, Module, OwnedCellDecl, RefDecl, Span,
    StackEffect, StructDecl, Term, TermKind, Type, TypedSlot, VariantDecl, WordBody, WordDef,
};
use crate::lexer::Token;

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

/// The one reserved-name gate every declaration site calls: a `^`-led name
/// (owning cells) or a `&`-led name (references).
pub fn reject_reserved_name(kind: &str, name: &str, span: Span) -> Result<(), String> {
    if is_reserved_caret_name(name) {
        return Err(reserved_caret_name_error(kind, name, span));
    }
    if is_reserved_ref_name(name) {
        return Err(reserved_ref_name_error(kind, name, span));
    }
    Ok(())
}

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
fn build_registries(decls: &[(String, Span, TypeDeclKind)]) -> (Vec<StructDecl>, Vec<EnumDecl>) {
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    for (name, span, kind) in decls {
        match kind {
            TypeDeclKind::Struct => {
                structs.push(StructDecl {
                    name: name.clone(),
                    name_static: Box::leak(name.clone().into_boxed_str()),
                    fields: Vec::new(),
                    span: *span,
                    has_drop_overload: false,
                });
            }
            TypeDeclKind::Enum(variant_names) => {
                let variants = variant_names
                    .iter()
                    .map(|(vname, vspan)| VariantDecl {
                        name: vname.clone(),
                        name_static: Box::leak(vname.clone().into_boxed_str()),
                        fields: Vec::new(),
                        span: *vspan,
                    })
                    .collect();
                enums.push(EnumDecl {
                    name: name.clone(),
                    name_static: Box::leak(name.clone().into_boxed_str()),
                    variants,
                    span: *span,
                });
            }
        }
    }
    (structs, enums)
}

pub fn parse(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let decls = prepass_type_decls(tokens)?;
    let (mut structs, mut enums) = build_registries(&decls);
    let mut words = Vec::new();
    let mut externs = Vec::new();
    let mut struct_fields_by_decl = Vec::new();
    let mut enum_fields_by_decl = Vec::new();
    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    {
        let mut parser = Parser {
            tokens,
            pos: 0,
            structs: &structs,
            enums: &enums,
            arrays: &mut arrays,
            owned_cells: &mut owned_cells,
            refs: &mut refs,
        };
        while parser.pos < parser.tokens.len() {
            if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "type:") {
                if parser.current_typedef_is_enum() {
                    enum_fields_by_decl.push(parser.parse_enum_typedef()?);
                } else {
                    struct_fields_by_decl.push(parser.parse_typedef()?);
                }
            } else if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "extern:") {
                externs.push(parser.parse_extern_decl()?);
            } else {
                words.push(parser.parse_worddef()?);
            }
        }
    }
    for (idx, fields) in struct_fields_by_decl.into_iter().enumerate() {
        structs[idx].fields = fields;
    }
    for (idx, variant_fields) in enum_fields_by_decl.into_iter().enumerate() {
        for (vidx, fields) in variant_fields.into_iter().enumerate() {
            enums[idx].variants[vidx].fields = fields;
        }
    }
    Ok(Module {
        words,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
        externs,
    })
}

/// Parse a single REPL line: a `:`-led definition, or a bare term sequence run
/// to end of input. One line is one complete unit (an unterminated def is a
/// normal parse error).
pub fn parse_line(tokens: &[(Token, Span)]) -> Result<Line, String> {
    let mut arrays = Vec::new();
    let mut owned_cells = Vec::new();
    let mut refs = Vec::new();
    parse_line_with_structs(tokens, &[], &[], &mut arrays, &mut owned_cells, &mut refs)
}

/// Parse a REPL line resolving struct and enum type names in a `:`
/// definition's effect against the session's registries, so a word may take
/// or return a previously-declared struct or enum. A bare expression carries
/// no type names, so the registries are unused there. `arrays` is the
/// session's interned array-type registry (R22/R23): a `[T N]` in a word
/// effect interns into it in place, so the `ArrayId` it returns stays valid
/// for later lines in the same session.
pub fn parse_line_with_structs(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &mut Vec<ArrayDecl>,
    owned_cells: &mut Vec<OwnedCellDecl>,
    refs: &mut Vec<RefDecl>,
) -> Result<Line, String> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
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
) -> Result<Vec<(String, Type)>, String> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
    };
    let fields = parser.parse_typedef()?;
    if let Some((tok, span)) = parser.peek() {
        return Err(format!(
            "parse error: unexpected {tok:?} after `;` at line {}, col {} (one line is one complete unit)",
            span.line, span.col
        ));
    }
    Ok(fields)
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
) -> Result<Vec<Vec<(String, Type)>>, String> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
        enums,
        arrays,
        owned_cells,
        refs,
    };
    let variant_fields = parser.parse_enum_typedef()?;
    if let Some((tok, span)) = parser.peek() {
        return Err(format!(
            "parse error: unexpected {tok:?} after `;` at line {}, col {} (one line is one complete unit)",
            span.line, span.col
        ));
    }
    Ok(variant_fields)
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
}

impl<'t> Parser<'t> {
    fn peek(&self) -> Option<&(Token, Span)> {
        self.tokens.get(self.pos)
    }

    fn eof_error(&self, expected: &str) -> String {
        let span = self
            .tokens
            .last()
            .map(|(_, s)| *s)
            .unwrap_or(Span { line: 0, col: 0 });
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
        self.expect(Token::LParen)?;
        let effect = self.parse_effect()?;
        self.expect(Token::RParen)?;
        // D8: a `|` immediately followed by a known variant name opens a
        // clause-style body; otherwise a `|` is an ordinary binding term.
        let body = if self.at_clause_start() {
            WordBody::Clauses(self.parse_clauses()?)
        } else {
            let terms = self.parse_terms("`;`", |tok| matches!(tok, Token::Semicolon))?;
            WordBody::Terms { terms }
        };
        self.expect(Token::Semicolon)?;
        Ok(WordDef { name, effect, body })
    }

    /// `extern:` declaration (R1): a top-level foreign-call binding. Grammar
    /// mirrors `worddef` except the body is a single explicit C symbol
    /// string rather than terms — a symbol string rather than the word name
    /// reused, since a Sooth name may use characters C cannot (`&!S>fi`), and
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
        })
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

    /// Whether `name` is a registered variant name of any enum (D8's variant
    /// pre-pass result), the load-bearing clause-vs-locals discriminator.
    fn is_variant_name(&self, name: &str) -> bool {
        self.enums
            .iter()
            .any(|e| e.variants.iter().any(|v| v.name == name))
    }

    /// Whether the token at `self.pos + offset` is a registered variant name.
    fn token_at_is_variant(&self, offset: usize) -> bool {
        matches!(self.tokens.get(self.pos + offset), Some((Token::Word(w), _)) if self.is_variant_name(w))
    }

    /// D8: the current position opens a clause-style body — a `|` immediately
    /// followed by a known variant name.
    fn at_clause_start(&self) -> bool {
        matches!(self.peek(), Some((Token::Pipe, _))) && self.token_at_is_variant(1)
    }

    /// Parse a clause-style word body (D4, D7, D8): one `|`-led clause per
    /// variant. Each clause is `|` + variant name + an optional clause-body
    /// `| names |` locals block (present iff the `|` after the variant name is
    /// *not* immediately followed by a known variant name) + body terms up to
    /// the next clause-starting `|` or `;`.
    fn parse_clauses(&mut self) -> Result<Vec<Clause>, String> {
        let mut clauses = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Semicolon, _)) => break,
                Some((Token::Pipe, span)) => {
                    let span = *span;
                    self.pos += 1; // the clause-leading `|`
                    let variant = self.expect_word_any()?;
                    // A `|` here opens clause-body locals unless it starts the
                    // next clause (a `|` followed by a known variant name).
                    let locals = if matches!(self.peek(), Some((Token::Pipe, _)))
                        && !self.token_at_is_variant(1)
                    {
                        self.parse_locals_opt()?
                    } else {
                        Vec::new()
                    };
                    let body = self.parse_clause_body_terms()?;
                    clauses.push(Clause {
                        variant,
                        locals,
                        body,
                        span,
                    });
                }
                Some((tok, span)) => {
                    return Err(format!(
                        "parse error: expected a clause `|` or `;`, found {tok:?} at line {}, col {}",
                        span.line, span.col
                    ));
                }
                None => return Err(self.eof_error("`;` (unterminated clause-style word)")),
            }
        }
        Ok(clauses)
    }

    fn parse_effect(&mut self) -> Result<StackEffect, String> {
        let inputs = self.parse_slots(|tok| matches!(tok, Token::RParen) || is_word(tok, "--"))?;
        self.expect_word("--")?;
        let outputs = self.parse_slots(|tok| matches!(tok, Token::RParen))?;
        Ok(StackEffect { inputs, outputs })
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
        // An array type has no name of its own to lead with (`[i64 4]` opens
        // on `[`, not a word), so an unnamed array slot is recognised before
        // the usual name-then-optional-`:type` read (R3, R7).
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            let ty = self.parse_array_type_expr()?;
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
            let ty = self.resolve_type(&text, span)?;
            Ok(TypedSlot { name: None, ty })
        }
    }

    /// A type expression: a single word (scalar/struct/enum,
    /// resolved via `resolve_type`), a bracketed array type `[ elem count ]`
    /// (`elem` itself a type expression, nested arrays recurse), or a
    /// `^`-led owning-cell type (nested cells recurse the same way).
    fn parse_type_expr(&mut self) -> Result<Type, String> {
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            self.parse_array_type_expr()
        } else if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('&')) {
            self.parse_ref_type_expr()
        } else if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^')) {
            self.parse_owning_cell_type_expr()
        } else {
            let (name, span) = self.expect_word_any_spanned()?;
            self.resolve_type(&name, span)
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
                return Err(format!(
                    "error: owning-cell type `{word}` has no payload type at line {}, col {} (write `{word}T` for some type T)",
                    span.line, span.col
                ));
            }
            self.parse_type_expr()?
        } else {
            // `span` names the whole word (e.g. `^Nope` starts at the `^`);
            // point at the remainder's own column so the error names and
            // locates the same text.
            let remainder_span = Span {
                line: span.line,
                col: span.col + run_len as u32,
            };
            self.resolve_type(remainder, remainder_span)?
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
            line: span.line,
            col: span.col + sigil_len as u32,
        };
        let referent = if remainder.is_empty() {
            if matches!(self.peek(), Some((Token::Word(w), _)) if w == "--") {
                return Err(format!(
                    "error: reference type `{word}` has no referent type at line {}, col {} (write `{word}T` for some type T)",
                    span.line, span.col
                ));
            }
            self.parse_type_expr()?
        } else if remainder.starts_with('^') {
            self.split_owning_cell_word(remainder, remainder_span)?
        } else {
            self.resolve_type(remainder, remainder_span)?
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
        let count = self.parse_array_count(element)?;
        self.expect(Token::RBracket)?;
        Ok(crate::ast::intern_array_type(self.arrays, element, count))
    }

    /// The array count token: a decimal literal `>= 1` and `<= u32::MAX`
    /// (M1: no const-expr eval, so a non-literal count is always a located
    /// error naming the offending token). A literal `< 1` or `> u32::MAX` is
    /// a located error naming the full `[T N]` spelling and the invalid
    /// length (X2).
    fn parse_array_count(&mut self, element: Type) -> Result<u32, String> {
        match self.peek().cloned() {
            Some((Token::Int(n), _span)) if (1..=i64::from(u32::MAX)).contains(&n) => {
                self.pos += 1;
                Ok(n as u32)
            }
            Some((Token::Int(n), span)) if n > i64::from(u32::MAX) => {
                self.pos += 1;
                Err(format!(
                    "error: array type `[{} {}]` has invalid length {} at line {}, col {} (`[T N]` requires N <= {})",
                    element.name(), n, n, span.line, span.col, u32::MAX
                ))
            }
            Some((Token::Int(n), span)) => {
                self.pos += 1;
                Err(format!(
                    "error: array type `[{} {}]` has invalid length {} at line {}, col {} (`[T N]` requires N >= 1)",
                    element.name(), n, n, span.line, span.col
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
                    let (field_name, _) = self.expect_word_any_spanned()?;
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
        if matches!(self.peek(), Some((Token::LBracket, _))) {
            return self.parse_array_type_expr();
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('^')) {
            return self.parse_owning_cell_type_expr();
        }
        if matches!(self.peek(), Some((Token::Word(w), _)) if w.starts_with('&')) {
            return self.parse_ref_type_expr();
        }
        let (ty_name, ty_span) = self.expect_field_type_token()?;
        self.resolve_type(&ty_name, ty_span)
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
        crate::ast::resolve_type_name(self.structs, self.enums, name).ok_or_else(|| {
            format!(
                "error: unknown type `{name}` at line {}, col {}",
                span.line, span.col
            )
        })
    }

    /// Lookahead (no consumption): whether the `type:` decl at the current
    /// position is an enum (D1's `|`-separated-variants body), per
    /// `body_has_pipe_before_semicolon`. `self.pos` must point at `type:`.
    fn current_typedef_is_enum(&self) -> bool {
        body_has_pipe_before_semicolon(self.tokens, self.pos + 2)
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
                    let (field_name, _) = self.expect_word_any_spanned()?;
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

    fn parse_locals_opt(&mut self) -> Result<Vec<String>, String> {
        if !matches!(self.peek(), Some((Token::Pipe, _))) {
            return Ok(Vec::new());
        }
        self.parse_binding_names()
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

    /// Parse a clause's body terms, stopping at `;` or a `|` that opens the
    /// next clause (D8's lookahead, applied at every `|` per R8, not only
    /// the first). Any other `|` is an ordinary mid-body binding term,
    /// parsed by `parse_term` like any other position.
    fn parse_clause_body_terms(&mut self) -> Result<Vec<Term>, String> {
        let mut terms = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error("`;` or `|`")),
                Some((Token::Semicolon, _)) => break,
                Some((Token::Pipe, _)) if self.at_clause_start() => break,
                Some((Token::Pipe, _)) => {
                    // D8 found no registered variant after this `|`, so it is
                    // read as a binding. When that read fails, a misspelt
                    // variant name is the likelier cause than a malformed
                    // binding, so name the disambiguation that was applied.
                    let lead = match self.tokens.get(self.pos + 1) {
                        Some((Token::Word(w), _)) => Some(w.clone()),
                        _ => None,
                    };
                    terms.push(self.parse_term().map_err(|e| match lead {
                        Some(name) => format!(
                            "{e}\n  note: `| {name}` opens a binding here, not a clause, because `{name}` is not a variant name; check its spelling"
                        ),
                        None => e,
                    })?);
                }
                _ => terms.push(self.parse_term()?),
            }
        }
        Ok(terms)
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
        // R1: a `|` at any term position opens a binding. A `|` that opens a
        // clause instead is consumed by `parse_clauses`, which never reaches
        // here.
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
            Token::Word(w) if w == "true" => Ok(Term {
                kind: TermKind::BoolLit(true),
                span,
            }),
            Token::Word(w) if w == "false" => Ok(Term {
                kind: TermKind::BoolLit(false),
                span,
            }),
            Token::Word(w) if w == "if" => {
                let then_branch = self
                    .parse_terms("`else` or `end` (unterminated `if`)", |tok| {
                        is_word(tok, "else") || is_word(tok, "end")
                    })?;
                let mut else_span = None;
                let else_branch = match self.peek() {
                    Some((tok, at)) if is_word(tok, "else") => {
                        else_span = Some(*at);
                        self.pos += 1;
                        self.parse_terms("`end` (unterminated `if`/`else`)", |tok| {
                            is_word(tok, "end")
                        })?
                    }
                    _ => Vec::new(),
                };
                let end_span = self.expect_word("end")?;
                Ok(Term {
                    kind: TermKind::If {
                        then_branch,
                        else_branch,
                        else_span,
                        end_span,
                    },
                    span,
                })
            }
            Token::Word(w) if w == "end" || w == "else" => Err(format!(
                "parse error: `{w}` without a matching `if` at line {}, col {}",
                span.line, span.col
            )),
            Token::Word(w) => Ok(Term {
                kind: TermKind::Call(w),
                span,
            }),
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
    use crate::lexer::lex;

    fn parse_src(src: &str) -> Result<Module, String> {
        let tokens = lex(src).unwrap();
        parse(&tokens)
    }

    /// The terms of a `WordBody::Terms`; panics on a clause body.
    fn terms_body(word: &WordDef) -> &[Term] {
        match &word.body {
            WordBody::Terms { terms } => terms,
            WordBody::Clauses(_) => panic!("expected a term body, got clauses"),
        }
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
        assert!(entry_locals(gcd).is_empty());
        assert_eq!(gcd.effect.inputs.len(), 2);
        assert_eq!(gcd.effect.outputs.len(), 1);

        // dup 0 = if drop else swap over mod gcd end
        assert_eq!(gcd_body.len(), 4);
        assert!(matches!(&gcd_body[0].kind, TermKind::Call(w) if w == "dup"));
        assert!(matches!(gcd_body[1].kind, TermKind::IntLit(0)));
        assert!(matches!(&gcd_body[2].kind, TermKind::Call(w) if w == "="));
        match &gcd_body[3].kind {
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                assert_eq!(then_branch.len(), 1);
                assert!(matches!(&then_branch[0].kind, TermKind::Call(w) if w == "drop"));
                assert_eq!(else_branch.len(), 4);
            }
            other => panic!("expected If, got {other:?}"),
        }

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
        let module = parse_src(": w ( i64 bool -- bool ) drop ;").unwrap();
        let w = &module.words[0];
        assert_eq!(w.effect.inputs[0].ty, Type::I64);
        assert_eq!(w.effect.inputs[1].ty, Type::Bool);
        assert_eq!(w.effect.outputs[0].ty, Type::Bool);
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
    fn parse_true_false_are_bool_literals() {
        let module = parse_src(": w ( -- bool bool ) true false ;").unwrap();
        let body = terms_body(&module.words[0]);
        assert!(matches!(body[0].kind, TermKind::BoolLit(true)));
        assert!(matches!(body[1].kind, TermKind::BoolLit(false)));
    }

    #[test]
    fn parse_if_without_else_has_empty_else_branch() {
        let module = parse_src(": w ( i64 -- i64 ) if 1 end ;").unwrap();
        let body = terms_body(&module.words[0]);
        match &body[0].kind {
            TermKind::If { else_branch, .. } => assert!(else_branch.is_empty()),
            other => panic!("expected If, got {other:?}"),
        }
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

    #[test]
    fn parse_then_no_longer_closes_if() {
        let result = parse_src(": w ( i64 -- i64 ) if 1 then ;");
        assert!(
            result.is_err(),
            "`then` must no longer close `if`; got {result:?}"
        );
    }

    #[test]
    fn parse_unterminated_if_reports_if_not_semicolon() {
        let result = parse_src(": w ( -- ) if 1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unterminated `if`"));
    }

    fn parse_line_src(src: &str) -> Result<Line, String> {
        let tokens = lex(src).unwrap();
        parse_line(&tokens)
    }

    #[test]
    fn parse_line_bare_expression_is_expr() {
        match parse_line_src("2 3 +").unwrap() {
            Line::Expr(terms) => {
                assert_eq!(terms.len(), 3);
                assert!(matches!(terms[0].kind, TermKind::IntLit(2)));
                assert!(matches!(&terms[2].kind, TermKind::Call(w) if w == "+"));
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
        match parse_line_src(": sq ( i64 -- i64 ) dup * ;").unwrap() {
            Line::Def(def) => assert_eq!(def.name, "sq"),
            other => panic!("expected Def, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_trailing_tokens_after_def_is_error() {
        let result = parse_line_src(": sq ( i64 -- i64 ) dup * ; 5 sq");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("after `;`"), "unexpected message: {err}");
    }

    #[test]
    fn parse_line_unterminated_def_is_error() {
        let result = parse_line_src(": sq ( i64 -- i64 ) dup *");
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
        assert_eq!(module.enums.len(), 1);
        let shape = &module.enums[0];
        assert_eq!(shape.name, "Shape");
        assert_eq!(shape.variants.len(), 2);
        assert_eq!(shape.variants[0].name, "Circle");
        assert_eq!(shape.variants[0].fields, vec![("r".to_string(), Type::F64)]);
        assert_eq!(shape.variants[1].name, "Rect");
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

    /// The `Clause` list of a `WordBody::Clauses`; panics on a term body.
    fn clauses_body(word: &WordDef) -> &[crate::ast::Clause] {
        match &word.body {
            WordBody::Clauses(clauses) => clauses,
            WordBody::Terms { .. } => panic!("expected clauses, got a term body"),
        }
    }

    #[test]
    fn parse_clause_word_multi_field_with_body_locals() {
        // D8: the first `|` is followed by a known variant, so the body is
        // clauses, not entry-locals; the `Rect` clause's `| w h |` is
        // clause-body locals (the `|` after `Rect` is not a variant).
        let module = parse_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;
             : area ( Shape -- f64 ) | Circle dup * | Rect | w h | w h * ;",
        )
        .unwrap();
        let area = module.words.iter().find(|w| w.name == "area").unwrap();
        let clauses = clauses_body(area);
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].variant, "Circle");
        assert!(clauses[0].locals.is_empty());
        assert_eq!(clauses[1].variant, "Rect");
        assert_eq!(clauses[1].locals, ["w", "h"]);
    }

    #[test]
    fn parse_clause_body_mid_body_pipe_produces_bind_term() {
        // The D8 lookahead applies at every `|` in a clause body, not only
        // the first, so a later `|` not followed by a known variant is an
        // ordinary mid-body binding term rather than a clause boundary.
        let module = parse_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ;\n             : area ( Shape -- f64 ) | Circle dup | r | r * | Rect | w h | w h * ;",
        )
        .unwrap();
        let area = module.words.iter().find(|w| w.name == "area").unwrap();
        let clauses = clauses_body(area);
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].variant, "Circle");
        assert!(clauses[0].locals.is_empty());
        assert_eq!(clauses[0].body.len(), 4, "expected dup, the bind, and r, *");
        assert!(
            matches!(clauses[0].body[1].kind, TermKind::Bind(ref names) if names == &["r".to_string()])
        );
    }

    #[test]
    fn parse_clause_word_empty_clause_before_next_clause() {
        // D8 empty-clause disambiguation: `| None` directly followed by
        // `| Some` (a known variant) is an empty-bodied clause, not locals.
        let module = parse_src(
            "type: MaybeInt | None | Some v i64 ;
             : unwrap-or ( i64 MaybeInt -- i64 ) | None | Some swap drop ;",
        )
        .unwrap();
        let uo = module.words.iter().find(|w| w.name == "unwrap-or").unwrap();
        let clauses = clauses_body(uo);
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].variant, "None");
        assert!(clauses[0].locals.is_empty());
        assert!(clauses[0].body.is_empty());
        assert_eq!(clauses[1].variant, "Some");
    }

    #[test]
    fn parse_term_word_with_leading_locals_is_not_a_clause() {
        // D8: a `|` followed by a non-variant word (with an enum in scope) is
        // entry-locals, not a clause.
        let module = parse_src(
            "type: Shape | Circle r f64 ;
             : sq ( i64 -- i64 ) | n | n n * ;",
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
        // The parser cannot know `__spy` is linear until the checker resolves
        // it (struct/enum field lists aren't filled in until the whole
        // module is parsed); rejection happens later, in the checker.
        let result = parse_src(": w ( [__spy 2] -- ) drop ;");
        assert!(result.is_ok(), "unexpected error: {:?}", result.err());
    }

    #[test]
    fn parse_typedef_linear_array_field_parses_ok() {
        let result = parse_src("type: Bag xs [__spy 2] ;");
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
    fn reserved_caret_clause_body_local_is_error() {
        let src = "type: Shape | Circle r f64 ; : area ( Shape -- f64 ) | Circle | ^ | ^ ;";
        let err = parse_src(src).unwrap_err();
        assert!(err.contains("reserved"), "unexpected message: {err}");
        assert!(err.contains('^'), "unexpected message: {err}");
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
        // input, not `resolve_type`'s. Reachable in the dogfood only via
        // reference-mode clause inference, so it gets a unit test of its own.
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
            let err = parse_src(&format!(": {name} ( i64 -- ) . ;")).unwrap_err();
            assert!(
                err.contains("is a builtin access word"),
                "unexpected message for `{name}`: {err}"
            );
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
}
