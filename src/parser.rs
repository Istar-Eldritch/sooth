//! Parser: tokens -> AST.
//!
//! Grammar (Phase 0, plus the Slice 3 `type:` production):
//!   module   := (worddef | typedef)*
//!   worddef  := ':' Word '(' effect ')' locals? term* ';'
//!   typedef  := 'type:' Word (Word Word)* ';'
//!   effect   := slot* '--' slot*
//!   slot     := Word (':' Word)?
//!   locals   := '|' Word* '|'
//!   term     := Int | Word | if
//!   if       := 'if' term* ('else' term*)? 'then'

use crate::ast::{
    Line, Module, Span, StackEffect, StructDecl, Term, TermKind, Type, TypedSlot, WordDef,
};
use crate::lexer::Token;

/// Pre-pass: scan the whole token stream for every `type: Name` and return
/// its name and span, in source order. Malformed occurrences (a
/// missing/non-word name) are left for the real `type:` production to report;
/// this pass only needs to register the names so forward/self references
/// resolve regardless of declaration order, so it never errors itself.
fn prepass_struct_names(tokens: &[(Token, Span)]) -> Vec<(String, Span)> {
    let mut names = Vec::new();
    for i in 0..tokens.len() {
        if let (Token::Word(w), _) = &tokens[i] {
            if w == "type:" {
                if let Some((Token::Word(name), span)) = tokens.get(i + 1) {
                    names.push((name.clone(), *span));
                }
            }
        }
    }
    names
}

/// Build the initial struct registry (names only, fields filled in once the
/// real `type:` bodies are parsed) from the pre-pass names, leaking each name
/// once so every `Type::Struct` naming it renders without a registry.
fn build_struct_registry(names: &[(String, Span)]) -> Vec<StructDecl> {
    names
        .iter()
        .map(|(name, span)| StructDecl {
            name: name.clone(),
            name_static: Box::leak(name.clone().into_boxed_str()),
            fields: Vec::new(),
            span: *span,
        })
        .collect()
}

pub fn parse(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let names = prepass_struct_names(tokens);
    let mut structs = build_struct_registry(&names);
    let mut words = Vec::new();
    let mut fields_by_decl = Vec::with_capacity(structs.len());
    {
        let mut parser = Parser {
            tokens,
            pos: 0,
            structs: &structs,
        };
        while parser.pos < parser.tokens.len() {
            if matches!(parser.peek(), Some((Token::Word(w), _)) if w == "type:") {
                fields_by_decl.push(parser.parse_typedef()?);
            } else {
                words.push(parser.parse_worddef()?);
            }
        }
    }
    for (idx, fields) in fields_by_decl.into_iter().enumerate() {
        structs[idx].fields = fields;
    }
    Ok(Module { words, structs })
}

/// Parse a single REPL line: a `:`-led definition, or a bare term sequence run
/// to end of input. One line is one complete unit (an unterminated def is a
/// normal parse error).
pub fn parse_line(tokens: &[(Token, Span)]) -> Result<Line, String> {
    parse_line_with_structs(tokens, &[])
}

/// Parse a REPL line resolving struct type names in a `:` definition's effect
/// against `structs` (the session's registry), so a word may take or
/// return a previously-declared struct. A bare expression carries no type
/// names, so `structs` is unused there.
pub fn parse_line_with_structs(
    tokens: &[(Token, Span)],
    structs: &[StructDecl],
) -> Result<Line, String> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
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
) -> Result<Vec<(String, Type)>, String> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        structs,
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

struct Parser<'t> {
    tokens: &'t [(Token, Span)],
    pos: usize,
    /// The struct registry (names always populated by the pre-pass, fields
    /// populated for the `type:` bodies already parsed at the point of
    /// lookup, but resolution only needs the id/name so declaration order
    /// among structs doesn't matter). Empty for a REPL line (struct
    /// declarations are not yet supported at REPL scope).
    structs: &'t [StructDecl],
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
        let name = self.expect_word_any()?;
        self.expect(Token::LParen)?;
        let effect = self.parse_effect()?;
        self.expect(Token::RParen)?;
        let locals = self.parse_locals_opt()?;
        let body = self.parse_terms("`;`", |tok| matches!(tok, Token::Semicolon))?;
        self.expect(Token::Semicolon)?;
        Ok(WordDef {
            name,
            effect,
            locals,
            body,
        })
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
        let (text, span) = self.expect_word_any_spanned()?;
        if matches!(self.peek(), Some((Token::Word(w), _)) if w == ":") {
            self.pos += 1;
            let (ty_name, ty_span) = self.expect_word_any_spanned()?;
            let ty = self.resolve_type(&ty_name, ty_span)?;
            Ok(TypedSlot {
                name: Some(text),
                ty,
            })
        } else {
            let ty = self.resolve_type(&text, span)?;
            Ok(TypedSlot { name: None, ty })
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
                    let (ty_name, ty_span) = self.expect_field_type_token()?;
                    let ty = self.resolve_type(&ty_name, ty_span)?;
                    fields.push((field_name, ty));
                }
                None => return Err(self.eof_error("`;` (unterminated `type:` declaration)")),
            }
        }
        self.expect(Token::Semicolon)?;
        Ok(fields)
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
        crate::ast::resolve_type_name(self.structs, name).ok_or_else(|| {
            format!(
                "error: unknown type `{name}` at line {}, col {}",
                span.line, span.col
            )
        })
    }

    fn parse_locals_opt(&mut self) -> Result<Vec<String>, String> {
        if !matches!(self.peek(), Some((Token::Pipe, _))) {
            return Ok(Vec::new());
        }
        self.pos += 1;
        let mut names = Vec::new();
        loop {
            match self.peek() {
                Some((Token::Pipe, _)) => {
                    self.pos += 1;
                    break;
                }
                Some((Token::Word(w), _)) => {
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
                    .parse_terms("`else` or `then` (unterminated `if`)", |tok| {
                        is_word(tok, "else") || is_word(tok, "then")
                    })?;
                let else_branch = if matches!(self.peek(), Some((tok, _)) if is_word(tok, "else")) {
                    self.pos += 1;
                    self.parse_terms("`then` (unterminated `if`/`else`)", |tok| {
                        is_word(tok, "then")
                    })?
                } else {
                    Vec::new()
                };
                self.expect_word("then")?;
                Ok(Term {
                    kind: TermKind::If {
                        then_branch,
                        else_branch,
                    },
                    span,
                })
            }
            Token::Word(w) if w == "then" || w == "else" => Err(format!(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse_src(src: &str) -> Result<Module, String> {
        let tokens = lex(src).unwrap();
        parse(&tokens)
    }

    #[test]
    fn parse_gcd_shape_matches_ast() {
        let src = std::fs::read_to_string("examples/gcd.sth").unwrap();
        let module = parse_src(&src).unwrap();
        assert_eq!(module.words.len(), 2);

        let gcd = &module.words[0];
        assert_eq!(gcd.name, "gcd");
        assert!(gcd.locals.is_empty());
        assert_eq!(gcd.effect.inputs.len(), 2);
        assert_eq!(gcd.effect.outputs.len(), 1);

        // dup 0 = if drop else swap over mod gcd then
        assert_eq!(gcd.body.len(), 4);
        assert!(matches!(&gcd.body[0].kind, TermKind::Call(w) if w == "dup"));
        assert!(matches!(gcd.body[1].kind, TermKind::IntLit(0)));
        assert!(matches!(&gcd.body[2].kind, TermKind::Call(w) if w == "="));
        match &gcd.body[3].kind {
            TermKind::If {
                then_branch,
                else_branch,
            } => {
                assert_eq!(then_branch.len(), 1);
                assert!(matches!(&then_branch[0].kind, TermKind::Call(w) if w == "drop"));
                assert_eq!(else_branch.len(), 4);
            }
            other => panic!("expected If, got {other:?}"),
        }

        let main = &module.words[1];
        assert_eq!(main.name, "main");
        assert!(main.locals.is_empty());
    }

    #[test]
    fn parse_locals_block_populates_locals() {
        let src = std::fs::read_to_string("examples/lerp.sth").unwrap();
        let module = parse_src(&src).unwrap();
        let lerp = module.words.iter().find(|w| w.name == "lerp").unwrap();
        assert_eq!(lerp.locals, vec!["a", "b", "t"]);
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
        let body = &module.words[0].body;
        assert!(matches!(body[0].kind, TermKind::BoolLit(true)));
        assert!(matches!(body[1].kind, TermKind::BoolLit(false)));
    }

    #[test]
    fn parse_if_without_else_has_empty_else_branch() {
        let module = parse_src(": w ( i64 -- i64 ) if 1 then ;").unwrap();
        let body = &module.words[0].body;
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
    fn parse_then_without_if_is_error() {
        let result = parse_src(": w ( -- ) then ;");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("then"));
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
}
