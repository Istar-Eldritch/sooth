//! Stack-effect checker. Arity only for now; type unification is a later ROADMAP phase.
//!
//! Simulates the compile-time virtual stack through each word body and verifies
//! the net effect matches the declared signature, unifying branch join points.
//! Mismatched depth across branches is a compile error (Forth's silent-underflow
//! failure mode becomes a diagnostic here).

use std::collections::HashMap;

use crate::ast::{Module, Span, StackEffect, Term, TermKind, WordDef};

/// Declared arity of a word: (inputs, outputs).
pub type Arity = (usize, usize);

/// The builtin word -> arity table, as the seed of a checking env.
pub fn builtin_table() -> HashMap<String, Arity> {
    [
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
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// Error context for the shared depth simulation: a full word (with its
/// declared effect and locals) or a bare REPL line (no signature to cite).
enum Ctx<'a> {
    Word {
        name: &'a str,
        effect: &'a StackEffect,
        locals: &'a [String],
    },
    Line,
}

impl Ctx<'_> {
    fn locals(&self) -> &[String] {
        match self {
            Ctx::Word { locals, .. } => locals,
            Ctx::Line => &[],
        }
    }
}

pub fn check(module: &Module) -> Result<(), String> {
    let mut env = builtin_table();
    for word in &module.words {
        env.insert(
            word.name.clone(),
            (word.effect.inputs.len(), word.effect.outputs.len()),
        );
    }

    for word in &module.words {
        check_word(word, &env)?;
    }
    Ok(())
}

/// Check a single word definition against an external env, seeding the env with
/// the word's own declared arity so self-recursion type-checks.
pub fn check_def(word: &WordDef, env: &HashMap<String, Arity>) -> Result<(), String> {
    let mut env = env.clone();
    env.insert(
        word.name.clone(),
        (word.effect.inputs.len(), word.effect.outputs.len()),
    );
    check_word(word, &env)
}

/// Infer the net effect of a bare line: simulate the stack from `entry_depth`
/// (the carried depth) and return the resulting depth. Underflow against the
/// carried stack is a reported error.
pub fn infer_line(
    terms: &[Term],
    entry_depth: usize,
    env: &HashMap<String, Arity>,
) -> Result<usize, String> {
    check_terms(terms, entry_depth, &Ctx::Line, env)
}

fn effect_str(effect: &StackEffect) -> String {
    let ins: Vec<&str> = effect.inputs.iter().map(|s| s.ty.as_str()).collect();
    let outs: Vec<&str> = effect.outputs.iter().map(|s| s.ty.as_str()).collect();
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

fn check_word(word: &WordDef, env: &HashMap<String, Arity>) -> Result<(), String> {
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
    let ctx = Ctx::Word {
        name: &word.name,
        effect: &word.effect,
        locals: &word.locals,
    };
    let final_depth = check_terms(&word.body, depth, &ctx, env)?;

    if final_depth != outputs {
        let line = word.body.last().map(|t| t.span.line).unwrap_or(0);
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  body leaves {} values, but ( … ) declares {} outputs\n  note: declared {}",
            word.name, line, final_depth, outputs, effect_str(&word.effect),
        ));
    }

    Ok(())
}

fn unknown_word_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown word `{}` in `{}` (line {})",
            name, wname, span.line
        ),
        Ctx::Line => format!("error: unknown word `{name}`"),
    }
}

fn underflow_error(ctx: &Ctx, span: Span, op: &str, needs: usize, holds: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `{}` needs {} values, but the stack holds {}\n  note: declared {}",
            name, span.line, op, needs, holds, effect_str(effect),
        ),
        Ctx::Line => format!("error: stack underflow: needs {needs} values, but the stack holds {holds}"),
    }
}

fn branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: stack effect mismatch in `{}` (line {})\n  `if` branches leave different stack depths (then: {}, else: {})\n  note: declared {}",
            name, span.line, d_then, d_else, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: `if` branches leave different stack depths (then: {d_then}, else: {d_else})"
        ),
    }
}

fn check_terms(
    terms: &[Term],
    mut depth: usize,
    ctx: &Ctx,
    env: &HashMap<String, Arity>,
) -> Result<usize, String> {
    for term in terms {
        depth = check_term(term, depth, ctx, env)?;
    }
    Ok(depth)
}

fn check_term(
    term: &Term,
    depth: usize,
    ctx: &Ctx,
    env: &HashMap<String, Arity>,
) -> Result<usize, String> {
    match &term.kind {
        TermKind::IntLit(_) => Ok(depth + 1),
        TermKind::Call(name) => {
            if ctx.locals().contains(name) {
                return Ok(depth + 1);
            }
            let (in_arity, out_arity) = env
                .get(name)
                .copied()
                .ok_or_else(|| unknown_word_error(ctx, term.span, name))?;
            if depth < in_arity {
                return Err(underflow_error(ctx, term.span, name, in_arity, depth));
            }
            Ok(depth - in_arity + out_arity)
        }
        TermKind::If {
            then_branch,
            else_branch,
        } => {
            if depth < 1 {
                return Err(underflow_error(ctx, term.span, "if", 1, depth));
            }
            let post_pop = depth - 1;
            let d_then = check_terms(then_branch, post_pop, ctx, env)?;
            let d_else = check_terms(else_branch, post_pop, ctx, env)?;
            if d_then != d_else {
                return Err(branch_mismatch_error(ctx, term.span, d_then, d_else));
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

    fn infer_src(src: &str, entry_depth: usize) -> Result<usize, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        infer_line(&terms, entry_depth, &builtin_table())
    }

    #[test]
    fn infer_line_net_effect_expected() {
        assert_eq!(infer_src("2 3 +", 0).unwrap(), 1);
    }

    #[test]
    fn infer_line_carries_entry_depth() {
        // `2 +` from a carried depth of 1: the literal plus the carried slot
        // are consumed by `+`, leaving one value.
        assert_eq!(infer_src("2 +", 1).unwrap(), 1);
    }

    #[test]
    fn line_underflow_against_carried_stack_is_error() {
        let err = infer_src("+", 1).unwrap_err();
        assert!(err.contains("stack underflow"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
        assert!(err.contains("holds 1"), "unexpected message: {err}");
    }
}
