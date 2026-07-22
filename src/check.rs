//! Stack-effect checker. Phase 0: arity only; type unification arrives in Phase 2.
//!
//! Simulates the compile-time virtual stack through each word body and verifies
//! the net effect matches the declared signature, unifying branch/loop join points.
//! Mismatched depth across branches is a compile error (Forth's silent-underflow
//! failure mode becomes a diagnostic here).

use std::collections::HashMap;

use crate::ast::{Module, Span, StackEffect, Term, TermKind, WordDef};

/// Declared arity of a word: (inputs, outputs).
type Arity = (usize, usize);

fn builtin_table() -> HashMap<&'static str, Arity> {
    HashMap::from([
        ("+", (2, 1)),
        ("-", (2, 1)),
        ("*", (2, 1)),
        ("mod", (2, 1)),
        ("=", (2, 1)),
        ("<", (2, 1)),
        (">", (2, 1)),
        (".", (1, 0)),
        ("dup", (1, 2)),
        ("over", (2, 3)),
        ("swap", (2, 2)),
        ("rot", (3, 3)),
        ("drop", (1, 0)),
    ])
}

pub fn check(module: &Module) -> Result<(), String> {
    let mut words: HashMap<String, Arity> = builtin_table()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    for word in &module.words {
        words.insert(
            word.name.clone(),
            (word.effect.inputs.len(), word.effect.outputs.len()),
        );
    }

    for word in &module.words {
        check_word(word, &words)?;
    }
    Ok(())
}

fn effect_str(effect: &StackEffect) -> String {
    let ins: Vec<&str> = effect.inputs.iter().map(|s| s.ty.as_str()).collect();
    let outs: Vec<&str> = effect.outputs.iter().map(|s| s.ty.as_str()).collect();
    format!("( {} -- {} )", ins.join(" "), outs.join(" "))
        .replace("(  -- ", "( -- ")
        .replace(" --  )", " -- )")
}

fn check_word(word: &WordDef, words: &HashMap<String, Arity>) -> Result<(), String> {
    let inputs = word.effect.inputs.len();
    let outputs = word.effect.outputs.len();

    if word.locals.len() > inputs {
        return Err(format!(
            "error: stack effect mismatch in `{}`\n  locals bind {} value(s), but only {} input(s) are declared\n  note: declared {}",
            word.name,
            word.locals.len(),
            inputs,
            effect_str(&word.effect),
        ));
    }

    let depth = inputs - word.locals.len();
    let final_depth = check_terms(&word.body, depth, word, words)?;

    if final_depth != outputs {
        let line = word.body.last().map(|t| t.span.line).unwrap_or(0);
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  body leaves {} values, but ( … ) declares {} outputs\n  note: declared {}",
            word.name, line, final_depth, outputs, effect_str(&word.effect),
        ));
    }

    Ok(())
}

fn underflow_error(word: &WordDef, span: Span, op: &str, needs: usize, holds: usize) -> String {
    format!(
        "error: stack effect mismatch in `{}` (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
        word.name, span.line, op, needs, holds, effect_str(&word.effect),
    )
}

fn check_terms(
    terms: &[Term],
    mut depth: usize,
    word: &WordDef,
    words: &HashMap<String, Arity>,
) -> Result<usize, String> {
    for term in terms {
        depth = check_term(term, depth, word, words)?;
    }
    Ok(depth)
}

fn check_term(
    term: &Term,
    depth: usize,
    word: &WordDef,
    words: &HashMap<String, Arity>,
) -> Result<usize, String> {
    match &term.kind {
        TermKind::IntLit(_) => Ok(depth + 1),
        TermKind::Call(name) => {
            if word.locals.contains(name) {
                return Ok(depth + 1);
            }
            let (in_arity, out_arity) = words.get(name).copied().ok_or_else(|| {
                format!(
                    "error: unknown word `{}` in `{}` (line {})",
                    name, word.name, term.span.line
                )
            })?;
            if depth < in_arity {
                return Err(underflow_error(word, term.span, name, in_arity, depth));
            }
            Ok(depth - in_arity + out_arity)
        }
        TermKind::If {
            then_branch,
            else_branch,
        } => {
            if depth < 1 {
                return Err(underflow_error(word, term.span, "if", 1, depth));
            }
            let post_pop = depth - 1;
            let d_then = check_terms(then_branch, post_pop, word, words)?;
            let d_else = check_terms(else_branch, post_pop, word, words)?;
            if d_then != d_else {
                return Err(format!(
                    "error: stack effect mismatch in `{}` (line {})\n  `if` branches leave different stack depths (then: {}, else: {})\n  note: declared {}",
                    word.name, term.span.line, d_then, d_else, effect_str(&word.effect),
                ));
            }
            Ok(d_then)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        check(&module)
    }

    #[test]
    fn check_gcd_is_ok() {
        let src = std::fs::read_to_string("examples/gcd.sth").unwrap();
        check_src(&src).unwrap();
    }

    #[test]
    fn check_factorial_is_ok() {
        let src = std::fs::read_to_string("examples/factorial.sth").unwrap();
        check_src(&src).unwrap();
    }

    #[test]
    fn check_lerp_is_ok() {
        let src = std::fs::read_to_string("examples/lerp.sth").unwrap();
        check_src(&src).unwrap();
    }

    #[test]
    fn check_stack_underflow_is_error() {
        let src = ": oops ( int -- int )\n  | a | a a + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("oops"));
        assert!(err.contains("`+`"));
        assert!(err.contains("needs 2 values"));
        assert!(err.contains("holds 1"));
        assert!(err.contains("( int -- int )"));
    }

    #[test]
    fn check_branch_depth_mismatch_is_error() {
        let src = ": w ( int -- int ) if 1 1 else 1 then ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different stack depths"));
    }

    #[test]
    fn check_declared_output_mismatch_is_error() {
        let src = ": w ( -- int ) 1 1 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("body leaves 2 values"));
        assert!(err.contains("declares 1 outputs"));
    }

    #[test]
    fn check_unknown_word_is_error() {
        let src = ": w ( int -- int ) frobnicate ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown word"));
        assert!(err.contains("frobnicate"));
    }

    #[test]
    fn check_locals_exceed_inputs_is_error() {
        let src = ": w ( int -- int ) | a b | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("locals bind"));
    }
}
