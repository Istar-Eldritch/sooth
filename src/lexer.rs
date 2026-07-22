//! Tokeniser. Phase 0: `: ;`, integers, words, `( ... )` stack effects, `| ... |`.

use crate::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Colon,
    Semicolon,
    LParen,
    RParen,
    Pipe,
    Int(i64),
    Word(String),
}

fn is_delimiter(c: char) -> bool {
    matches!(c, ':' | ';' | '(' | ')' | '|')
}

fn is_int_literal(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
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
            ':' | ';' | '(' | ')' | '|' => {
                let span = Span { line, col };
                let tok = match c {
                    ':' => Token::Colon,
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
                Token::Colon,
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
}
