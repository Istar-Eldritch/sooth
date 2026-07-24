//! Tokeniser. Phase 0: `: ;`, integers, words, `( ... )` stack effects, `| ... |`.
//! `:` is not a delimiter (Slice 3, R1): `:` and `type:` lex as whole word
//! tokens on surrounding whitespace, so the parser keys on `Word(":")` /
//! `Word("type:")` rather than a dedicated token.

use crate::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Semicolon,
    LParen,
    RParen,
    Pipe,
    Int(i64),
    Float(f64),
    Word(String),
}

fn is_delimiter(c: char) -> bool {
    matches!(c, ';' | '(' | ')' | '|')
}

fn is_int_literal(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// A float literal is `<digits>.<digits>` with an optional `[eE][+-]?<digits>`
/// exponent. Digits are required on both sides of the dot (`3.` and `.5` are
/// not float literals) so a literal can never collide with the `.` print word.
/// A magnitude beyond `f64` range parses to `inf`/`0.0` rather than erroring
/// (Rust's `f64::from_str` never fails on this grammar), which matches the
/// language's own silent-inf-propagation semantics rather than fighting them.
fn is_float_literal(text: &str) -> bool {
    let text = text.strip_prefix('-').unwrap_or(text);
    let Some(dot) = text.find('.') else {
        return false;
    };
    let (int_part, rest) = text.split_at(dot);
    let frac_and_exp = &rest[1..];
    if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let frac_end = frac_and_exp.find(['e', 'E']).unwrap_or(frac_and_exp.len());
    let (frac_part, exp_part) = frac_and_exp.split_at(frac_end);
    if frac_part.is_empty() || !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if exp_part.is_empty() {
        return true;
    }
    let exp_digits = exp_part[1..]
        .strip_prefix(['+', '-'])
        .unwrap_or(&exp_part[1..]);
    !exp_digits.is_empty() && exp_digits.chars().all(|c| c.is_ascii_digit())
}

pub fn lex(src: &str) -> Result<Vec<(Token, Span)>, String> {
    let mut tokens = Vec::new();
    let mut chars = src.chars().peekable();
    let mut line: u32 = 1;
    let mut col: u32 = 1;

    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
                if c == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
            }
            ';' | '(' | ')' | '|' => {
                let span = Span { line, col };
                let tok = match c {
                    ';' => Token::Semicolon,
                    '(' => Token::LParen,
                    ')' => Token::RParen,
                    '|' => Token::Pipe,
                    _ => unreachable!(),
                };
                chars.next();
                col += 1;
                tokens.push((tok, span));
            }
            _ => {
                let start = Span { line, col };
                let mut text = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || is_delimiter(c) {
                        break;
                    }
                    text.push(c);
                    chars.next();
                    col += 1;
                }

                if text == "\\" {
                    while let Some(&c) = chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        chars.next();
                    }
                    continue;
                }

                if is_int_literal(&text) {
                    let n = text.parse::<i64>().map_err(|_| {
                        format!(
                            "lex error: integer literal '{text}' out of range at line {}, col {}",
                            start.line, start.col
                        )
                    })?;
                    tokens.push((Token::Int(n), start));
                } else if is_float_literal(&text) {
                    let v = text.parse::<f64>().expect(
                        "is_float_literal validates a grammar f64::from_str always accepts",
                    );
                    tokens.push((Token::Float(v), start));
                } else {
                    tokens.push((Token::Word(text), start));
                }
            }
        }
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(tokens: &[(Token, Span)]) -> Vec<Token> {
        tokens.iter().map(|(t, _)| t.clone()).collect()
    }

    #[test]
    fn lex_word_definition_tokenises() {
        let src = ": sq ( i64 -- i64 ) | n | n n * ;";
        let tokens = lex(src).unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::Word(":".into()),
                Token::Word("sq".into()),
                Token::LParen,
                Token::Word("i64".into()),
                Token::Word("--".into()),
                Token::Word("i64".into()),
                Token::RParen,
                Token::Pipe,
                Token::Word("n".into()),
                Token::Pipe,
                Token::Word("n".into()),
                Token::Word("n".into()),
                Token::Word("*".into()),
                Token::Semicolon,
            ]
        );
    }

    #[test]
    fn lex_typedef_tokenises_as_single_word() {
        let src = "type: Vec2 x i64 y i64 ;";
        let tokens = lex(src).unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::Word("type:".into()),
                Token::Word("Vec2".into()),
                Token::Word("x".into()),
                Token::Word("i64".into()),
                Token::Word("y".into()),
                Token::Word("i64".into()),
                Token::Semicolon,
            ]
        );
    }

    #[test]
    fn lex_negative_integer_is_int() {
        let tokens = lex("-5 -").unwrap();
        assert_eq!(
            words(&tokens),
            vec![Token::Int(-5), Token::Word("-".into())]
        );
    }

    #[test]
    fn lex_backslash_comment_skips_to_eol() {
        let src = "1 \\ this is a comment\n2";
        let tokens = lex(src).unwrap();
        assert_eq!(words(&tokens), vec![Token::Int(1), Token::Int(2)]);
    }

    #[test]
    fn lex_integer_overflow_is_error() {
        let src = "99999999999999999999";
        let err = lex(src).unwrap_err();
        assert!(err.contains("out of range"), "unexpected message: {err}");
    }

    #[test]
    fn lex_nonascii_whitespace_is_skipped() {
        let tokens = lex("1\u{a0}2").unwrap();
        assert_eq!(words(&tokens), vec![Token::Int(1), Token::Int(2)]);
    }

    #[test]
    fn lex_float_literal_is_float() {
        let tokens = lex("2.5 0.5 1.5e-3 1.0e9").unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::Float(2.5),
                Token::Float(0.5),
                Token::Float(1.5e-3),
                Token::Float(1.0e9),
            ]
        );
    }

    #[test]
    fn lex_float_overflow_saturates_to_inf() {
        let tokens = lex("1.0e999").unwrap();
        assert_eq!(words(&tokens), vec![Token::Float(f64::INFINITY)]);
    }

    #[test]
    fn lex_dangling_dot_not_float() {
        let tokens = lex("3. .5").unwrap();
        assert_eq!(
            words(&tokens),
            vec![Token::Word("3.".into()), Token::Word(".5".into())]
        );
    }

    #[test]
    fn lex_plain_integer_still_int() {
        let tokens = lex("42").unwrap();
        assert_eq!(words(&tokens), vec![Token::Int(42)]);
    }

    #[test]
    fn lex_int_then_print_word_expected() {
        let tokens = lex("5 .").unwrap();
        assert_eq!(words(&tokens), vec![Token::Int(5), Token::Word(".".into())]);
    }
}
