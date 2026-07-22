//! Parser: tokens -> AST.
//!
//! Grammar (Phase 0):
//!   module   := worddef*
//!   worddef  := ':' Word '(' effect ')' locals? term* ';'
//!   effect   := slot* '--' slot*
//!   slot     := Word (':' Word)?
//!   locals   := '|' Word* '|'
//!   term     := Int | Word | if
//!   if       := 'if' term* ('else' term*)? 'then'

use crate::ast::{Line, Module, Span, StackEffect, Term, TermKind, TypedSlot, WordDef};
use crate::lexer::Token;

pub fn parse(tokens: &[(Token, Span)]) -> Result<Module, String> {
    let mut parser = Parser { tokens, pos: 0 };
    let mut module = Module::default();
    while parser.pos < parser.tokens.len() {
        module.words.push(parser.parse_worddef()?);
    }
    Ok(module)
}

/// Parse a single REPL line: a `:`-led definition, or a bare term sequence run
/// to end of input. One line is one complete unit (an unterminated def is a
/// normal parse error).
pub fn parse_line(tokens: &[(Token, Span)]) -> Result<Line, String> {
    let mut parser = Parser { tokens, pos: 0 };
    if matches!(parser.peek(), Some((Token::Colon, _))) {
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

struct Parser<'t> {
    tokens: &'t [(Token, Span)],
    pos: usize,
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
        self.expect(Token::Colon)?;
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
        let text = self.expect_word_any()?;
        if matches!(self.peek(), Some((Token::Colon, _))) {
            self.pos += 1;
            let ty = self.expect_word_any()?;
            Ok(TypedSlot {
                name: Some(text),
                ty,
            })
        } else {
            Ok(TypedSlot {
                name: None,
                ty: text,
            })
        }
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
    fn parse_if_without_else_has_empty_else_branch() {
        let module = parse_src(": w ( int -- int ) if 1 then ;").unwrap();
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
    fn parse_line_colon_is_def() {
        match parse_line_src(": sq ( int -- int ) dup * ;").unwrap() {
            Line::Def(def) => assert_eq!(def.name, "sq"),
            other => panic!("expected Def, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_trailing_tokens_after_def_is_error() {
        let result = parse_line_src(": sq ( int -- int ) dup * ; 5 sq");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("after `;`"), "unexpected message: {err}");
    }

    #[test]
    fn parse_line_unterminated_def_is_error() {
        let result = parse_line_src(": sq ( int -- int ) dup *");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unexpected end of input"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`;`"), "unexpected message: {err}");
    }
}
