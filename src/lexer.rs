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
    LBracket,
    RBracket,
    Int(i64),
    Float(f64),
    Word(String),
    /// A `"..."` string literal (R6), already escape-decoded: the raw
    /// content a `str` value carries, never the source spelling.
    Str(String),
    /// `~[`, glued with **zero** intervening whitespace (Slice 10a R1): the
    /// inline-only quotation type's opening sigil-plus-bracket. `~` is not a
    /// delimiter and `[` is, so without this glue `~[` and `~ [` both lex as
    /// `Word("~")` + `LBracket`, discarding adjacency; this token makes `~ [`
    /// a parse error instead of a silently-accepted spaced form.
    TildeLBracket,
}

fn is_delimiter(c: char) -> bool {
    matches!(c, ';' | '(' | ')' | '|' | '[' | ']')
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

/// Every emitted `Span`'s `module` is `0`: the lexer sees one file in
/// isolation and has no closure-wide id to stamp. `driver::make_node`
/// overwrites it on every token with the file's real id right after this
/// returns; a caller that skips that step (a REPL line, a unit test) is
/// always the single-file case, where `0` for every span is already correct.
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
            ';' | '(' | ')' | '|' | '[' | ']' => {
                let span = Span {
                    line,
                    col,
                    module: 0,
                };
                let tok = match c {
                    ';' => Token::Semicolon,
                    '(' => Token::LParen,
                    ')' => Token::RParen,
                    '|' => Token::Pipe,
                    '[' => Token::LBracket,
                    ']' => Token::RBracket,
                    _ => unreachable!(),
                };
                chars.next();
                col += 1;
                tokens.push((tok, span));
            }
            '"' => {
                let start = Span {
                    line,
                    col,
                    module: 0,
                };
                chars.next();
                col += 1;
                let mut s = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => {
                            closed = true;
                            col += 1;
                            break;
                        }
                        '\\' => {
                            col += 1;
                            let Some(esc) = chars.next() else { break };
                            col += 1;
                            match esc {
                                'n' => s.push('\n'),
                                't' => s.push('\t'),
                                '\\' => s.push('\\'),
                                '"' => s.push('"'),
                                '0' => s.push('\0'),
                                other => {
                                    return Err(format!(
                                        "lex error: unknown escape '\\{other}' in string literal at line {}, col {}",
                                        start.line, start.col
                                    ));
                                }
                            }
                        }
                        '\n' => {
                            s.push('\n');
                            line += 1;
                            col = 1;
                        }
                        _ => {
                            s.push(c);
                            col += 1;
                        }
                    }
                }
                if !closed {
                    return Err(format!(
                        "lex error: unterminated string literal starting at line {}, col {}",
                        start.line, start.col
                    ));
                }
                tokens.push((Token::Str(s), start));
            }
            _ => {
                let start = Span {
                    line,
                    col,
                    module: 0,
                };
                let mut text = String::new();
                while let Some(&c) = chars.peek() {
                    // `S|>fi` peek-word glue: `|` joins the current word only
                    // when a word char already precedes it (so `| a |` and a
                    // clause head `| Circle` are untouched, since those hit
                    // `|` as the very first character of a scan) and `>`
                    // immediately follows (so a bare trailing `|` still
                    // delimits normally).
                    if c == '|' && !text.is_empty() {
                        let mut lookahead = chars.clone();
                        lookahead.next();
                        if lookahead.peek() == Some(&'>') {
                            text.push('|');
                            chars.next();
                            col += 1;
                            continue;
                        }
                    }
                    if c.is_whitespace() || is_delimiter(c) {
                        break;
                    }
                    text.push(c);
                    chars.next();
                    col += 1;
                }

                if text == "~" && chars.peek() == Some(&'[') {
                    chars.next();
                    col += 1;
                    tokens.push((Token::TildeLBracket, start));
                    continue;
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
    fn lex_brackets_are_distinct_tokens_expected() {
        let tokens = lex("[i64 4]").unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::LBracket,
                Token::Word("i64".into()),
                Token::Int(4),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn lex_bracket_adjacent_to_word_still_splits_expected() {
        // `[` and `]` are delimiters, so `usize]` and `[usize` split just like
        // `foo;` splits on `;`, with no separating whitespace required.
        let tokens = lex("[usize]").unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::LBracket,
                Token::Word("usize".into()),
                Token::RBracket
            ]
        );
    }

    #[test]
    fn lex_int_then_print_word_expected() {
        let tokens = lex("5 .").unwrap();
        assert_eq!(words(&tokens), vec![Token::Int(5), Token::Word(".".into())]);
    }

    #[test]
    fn lex_peek_word_glues_pipe_gt_into_one_word() {
        let tokens = lex("Point|>x").unwrap();
        assert_eq!(words(&tokens), vec![Token::Word("Point|>x".into())]);
    }

    #[test]
    fn lex_locals_pipes_stay_separate_tokens() {
        let tokens = lex("| n |").unwrap();
        assert_eq!(
            words(&tokens),
            vec![Token::Pipe, Token::Word("n".into()), Token::Pipe]
        );
    }

    #[test]
    fn lex_mid_word_pipe_without_gt_still_delimits() {
        let tokens = lex("a| b").unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::Word("a".into()),
                Token::Pipe,
                Token::Word("b".into())
            ]
        );
    }

    #[test]
    fn lex_clause_head_pipe_stays_separate_token() {
        let tokens = lex("| Circle").unwrap();
        assert_eq!(
            words(&tokens),
            vec![Token::Pipe, Token::Word("Circle".into())]
        );
    }

    #[test]
    fn lex_owning_cell_peek_word_glues_into_one_token() {
        // R12/criterion 20: `^|>` survives the `S|>fi` peek-glue rule as one
        // token, the same way `Point|>x` does.
        let tokens = lex("^|>").unwrap();
        assert_eq!(words(&tokens), vec![Token::Word("^|>".into())]);
    }

    #[test]
    fn lex_nested_owning_cell_scalar_type_is_one_token() {
        // R12/criterion 20: `^` is not a delimiter, so `^^i64` lexes as a
        // single word (the parser's R19 rule then splits the leading `^`-run
        // from the remainder).
        let tokens = lex("^^i64").unwrap();
        assert_eq!(words(&tokens), vec![Token::Word("^^i64".into())]);
    }

    #[test]
    fn lex_owning_cell_array_type_splits_at_bracket() {
        // R12/criterion 20: `[` is a delimiter, so `^[u8 4]` splits into a
        // bare `^`-run word followed by the usual array-type tokens.
        let tokens = lex("^[u8 4]").unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::Word("^".into()),
                Token::LBracket,
                Token::Word("u8".into()),
                Token::Int(4),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn lex_borrow_sigil_glues_to_the_place_it_borrows() {
        // Neither `&` nor `!` is a delimiter, so a prefix borrow and each
        // reference-mode accessor lex as one token: the checker resolves a
        // borrow in one step, like any other word.
        let tokens = lex("&a &!a &!Buf>len &> &!> &^ &!^").unwrap();
        assert_eq!(
            words(&tokens),
            vec![
                Token::Word("&a".into()),
                Token::Word("&!a".into()),
                Token::Word("&!Buf>len".into()),
                Token::Word("&>".into()),
                Token::Word("&!>".into()),
                Token::Word("&^".into()),
                Token::Word("&!^".into()),
            ]
        );
    }

    #[test]
    fn lex_spaced_ampersand_is_not_a_borrow() {
        // The sigil binds tightly: `& a` is two tokens, and `a&!` typed as one
        // run is the single (unknown) word `a&!`, never `a` then `&!`.
        assert_eq!(
            words(&lex("& a").unwrap()),
            vec![Token::Word("&".into()), Token::Word("a".into())]
        );
        assert_eq!(words(&lex("a&!").unwrap()), vec![Token::Word("a&!".into())]);
    }

    #[test]
    fn lex_string_literal_handles_every_escape() {
        // Criterion 1: every declared escape (`\n \t \\ \" \0`) decodes.
        let tokens = lex(r#""a\nb\tc\\d\"e\0f""#).unwrap();
        assert_eq!(
            words(&tokens),
            vec![Token::Str("a\nb\tc\\d\"e\0f".to_string())]
        );
    }

    #[test]
    fn lex_unterminated_string_literal_is_error() {
        // Criterion 2: no closing `"` before EOF is a located error.
        let err = lex(r#""unterminated"#).unwrap_err();
        assert!(err.contains("unterminated"), "unexpected message: {err}");
        assert!(err.contains("line 1"), "unexpected message: {err}");
    }

    #[test]
    fn lex_unknown_string_escape_is_error() {
        // Criterion 3: an escape outside `\n \t \\ \" \0` is a located error.
        let err = lex(r#""bad\zescape""#).unwrap_err();
        assert!(err.contains("unknown escape"), "unexpected message: {err}");
        assert!(err.contains("\\z"), "unexpected message: {err}");
    }

    #[test]
    fn lex_reference_to_array_type_splits_at_bracket() {
        // `[` *is* a delimiter, so `&![u8 64]` splits across tokens while
        // `&!^List` stays whole.
        assert_eq!(
            words(&lex("&![u8 64]").unwrap()),
            vec![
                Token::Word("&!".into()),
                Token::LBracket,
                Token::Word("u8".into()),
                Token::Int(64),
                Token::RBracket,
            ]
        );
        assert_eq!(
            words(&lex("&!^List").unwrap()),
            vec![Token::Word("&!^List".into())]
        );
    }

    #[test]
    fn lex_tilde_bracket_glued_is_one_token() {
        // Slice 10a R1: `~[` with zero intervening whitespace is a single
        // `TildeLBracket`, so adjacency survives into the token stream.
        let tokens = lex("~[").unwrap();
        assert_eq!(words(&tokens), vec![Token::TildeLBracket]);
    }

    #[test]
    fn lex_tilde_bracket_spaced_stays_two_tokens() {
        // Slice 10a R1: a space between `~` and `[` drops the glue, so `~ [`
        // lexes as the plain `Word("~")` + `LBracket` it always did — which
        // the parser then rejects, since nothing declares a bare `~` word.
        let tokens = lex("~ [").unwrap();
        assert_eq!(
            words(&tokens),
            vec![Token::Word("~".into()), Token::LBracket]
        );
    }
}
