//! Stack-effect checker. Simulates a compile-time virtual stack of concrete
//! `Type`s through each word body and verifies the net effect matches the
//! declared signature.
//!
//! Every operand is checked against the type its consumer expects, so a
//! `bool` where `+` wants an `i64` is a located compile error (Forth's silent
//! coercion failure mode becomes a diagnostic here). Branch join points still
//! unify on depth only; per-slot type unification at joins is a later phase.

use std::collections::HashMap;

use crate::ast::{Module, Span, StackEffect, Term, TermKind, Type, WordDef};

/// A word's typed stack effect: the concrete input and output slot types,
/// deepest-first (leftmost in `( … )` is deepest on the stack).
#[derive(Debug, Clone)]
pub struct Sig {
    pub inputs: Vec<Type>,
    pub outputs: Vec<Type>,
}

/// The typed effect of a declared word.
pub fn sig_of(effect: &StackEffect) -> Sig {
    Sig {
        inputs: effect.inputs.iter().map(|s| s.ty).collect(),
        outputs: effect.outputs.iter().map(|s| s.ty).collect(),
    }
}

/// The builtin word -> typed-effect table, as the seed of a checking env. The
/// stack shuffles (`dup`/`drop`/`swap`/`over`/`rot`) are deliberately absent:
/// they are structural and type-transparent (they move whatever slot types are
/// present), handled directly in `check_term`, not as fixed signatures.
pub fn builtin_table() -> HashMap<String, Sig> {
    use Type::{Bool, I64};
    let sig = |inputs: &[Type], outputs: &[Type]| Sig {
        inputs: inputs.to_vec(),
        outputs: outputs.to_vec(),
    };
    [
        ("+", sig(&[I64, I64], &[I64])),
        ("-", sig(&[I64, I64], &[I64])),
        ("*", sig(&[I64, I64], &[I64])),
        ("mod", sig(&[I64, I64], &[I64])),
        ("=", sig(&[I64, I64], &[Bool])),
        ("<", sig(&[I64, I64], &[Bool])),
        (">", sig(&[I64, I64], &[Bool])),
        (".", sig(&[I64], &[])),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// Error context for the shared stack simulation: a full word (with its
/// declared effect and typed locals) or a bare REPL line (no signature to cite).
enum Ctx<'a> {
    Word {
        name: &'a str,
        effect: &'a StackEffect,
        locals: &'a HashMap<String, Type>,
    },
    Line,
}

impl Ctx<'_> {
    fn local_type(&self, name: &str) -> Option<Type> {
        match self {
            Ctx::Word { locals, .. } => locals.get(name).copied(),
            Ctx::Line => None,
        }
    }
}

pub fn check(module: &Module) -> Result<(), String> {
    let mut env = builtin_table();
    for word in &module.words {
        env.insert(word.name.clone(), sig_of(&word.effect));
    }

    for word in &module.words {
        check_word(word, &env)?;
    }
    Ok(())
}

/// Check a single word definition against an external env, seeding the env with
/// the word's own signature so self-recursion type-checks.
pub fn check_def(word: &WordDef, env: &HashMap<String, Sig>) -> Result<(), String> {
    let mut env = env.clone();
    env.insert(word.name.clone(), sig_of(&word.effect));
    check_word(word, &env)
}

/// Infer the net effect of a bare line: simulate the typed stack from
/// `entry_stack` (the carried slot types) and return the resulting typed stack.
/// A type mismatch or underflow against the carried stack is a reported error.
pub fn infer_line(
    terms: &[Term],
    entry_stack: &[Type],
    env: &HashMap<String, Sig>,
) -> Result<Vec<Type>, String> {
    check_terms(terms, entry_stack.to_vec(), &Ctx::Line, env)
}

fn effect_str(effect: &StackEffect) -> String {
    let ins: Vec<String> = effect.inputs.iter().map(|s| s.ty.to_string()).collect();
    let outs: Vec<String> = effect.outputs.iter().map(|s| s.ty.to_string()).collect();
    let mut parts = vec!["--".to_string()];
    if !outs.is_empty() {
        parts.push(outs.join(" "));
    }
    if !ins.is_empty() {
        parts.insert(0, ins.join(" "));
    }
    format!("( {} )", parts.join(" "))
}

fn check_word(word: &WordDef, env: &HashMap<String, Sig>) -> Result<(), String> {
    let inputs = word.effect.inputs.len();

    if word.locals.len() > inputs {
        return Err(format!(
            "error: stack effect mismatch in `{}`\n  locals bind {} value(s), but only {} input(s) are declared\n  note: declared {}",
            word.name,
            word.locals.len(),
            inputs,
            effect_str(&word.effect),
        ));
    }

    // Locals bind the topmost inputs; the remaining (deepest) inputs stay on the
    // simulated stack, deepest-first.
    let split = inputs - word.locals.len();
    let initial: Vec<Type> = word.effect.inputs[..split].iter().map(|s| s.ty).collect();
    let mut local_types = HashMap::new();
    for (name, slot) in word.locals.iter().zip(&word.effect.inputs[split..]) {
        local_types.insert(name.clone(), slot.ty);
    }

    let ctx = Ctx::Word {
        name: &word.name,
        effect: &word.effect,
        locals: &local_types,
    };
    let final_stack = check_terms(&word.body, initial, &ctx, env)?;

    let declared: Vec<Type> = word.effect.outputs.iter().map(|s| s.ty).collect();
    let line = word.body.last().map(|t| t.span.line).unwrap_or(0);
    if final_stack.len() != declared.len() {
        return Err(format!(
            "error: stack effect mismatch in `{}` (line {})\n  body leaves {} values, but ( … ) declares {} outputs\n  note: declared {}",
            word.name, line, final_stack.len(), declared.len(), effect_str(&word.effect),
        ));
    }
    for (found, want) in final_stack.iter().zip(&declared) {
        if found != want {
            return Err(format!(
                "error: type mismatch in `{}` (line {})\n  body leaves `{}` where the declaration requires `{}`\n  note: declared {}",
                word.name, line, found, want, effect_str(&word.effect),
            ));
        }
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

fn type_mismatch_error(ctx: &Ctx, span: Span, op: &str, expected: Type, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` expected `{}`, found `{}`\n  note: declared {}",
            name, span.line, op, expected, found, effect_str(effect),
        ),
        Ctx::Line => {
            format!("error: type mismatch: `{op}` expected `{expected}`, found `{found}`")
        }
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
    mut stack: Vec<Type>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
) -> Result<Vec<Type>, String> {
    for term in terms {
        stack = check_term(term, stack, ctx, env)?;
    }
    Ok(stack)
}

fn check_term(
    term: &Term,
    mut stack: Vec<Type>,
    ctx: &Ctx,
    env: &HashMap<String, Sig>,
) -> Result<Vec<Type>, String> {
    let span = term.span;
    match &term.kind {
        TermKind::IntLit(_) => {
            stack.push(Type::I64);
            Ok(stack)
        }
        TermKind::BoolLit(_) => {
            stack.push(Type::Bool);
            Ok(stack)
        }
        TermKind::Call(name) => {
            if let Some(ty) = ctx.local_type(name) {
                stack.push(ty);
                return Ok(stack);
            }
            if let Some(stack) = check_shuffle(name, span, &mut stack, ctx)? {
                return Ok(stack);
            }
            let sig = env
                .get(name)
                .ok_or_else(|| unknown_word_error(ctx, span, name))?;
            let n = sig.inputs.len();
            if stack.len() < n {
                return Err(underflow_error(ctx, span, name, n, stack.len()));
            }
            let base = stack.len() - n;
            for (i, want) in sig.inputs.iter().enumerate() {
                let found = stack[base + i];
                if found != *want {
                    return Err(type_mismatch_error(ctx, span, name, *want, found));
                }
            }
            stack.truncate(base);
            stack.extend(sig.outputs.iter().copied());
            Ok(stack)
        }
        TermKind::If {
            then_branch,
            else_branch,
        } => {
            let cond = stack
                .pop()
                .ok_or_else(|| underflow_error(ctx, span, "if", 1, 0))?;
            if cond != Type::Bool {
                return Err(type_mismatch_error(ctx, span, "if", Type::Bool, cond));
            }
            let then_stack = check_terms(then_branch, stack.clone(), ctx, env)?;
            let else_stack = check_terms(else_branch, stack, ctx, env)?;
            if then_stack.len() != else_stack.len() {
                return Err(branch_mismatch_error(
                    ctx,
                    span,
                    then_stack.len(),
                    else_stack.len(),
                ));
            }
            Ok(then_stack)
        }
    }
}

/// Apply a stack shuffle if `name` is one, returning `Some(stack)`; `None` if
/// the name is not a shuffle (the caller then looks it up in the env). Shuffles
/// move concrete slot types with no fixed signature: `dup` of a `bool` yields
/// two `bool`s, `swap` reorders whatever two types are on top, etc.
fn check_shuffle(
    name: &str,
    span: Span,
    stack: &mut Vec<Type>,
    ctx: &Ctx,
) -> Result<Option<Vec<Type>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "dup" => {
            let top = *stack.last().ok_or_else(|| need("dup", 1, stack.len()))?;
            stack.push(top);
        }
        "drop" => {
            if stack.is_empty() {
                return Err(need("drop", 1, 0));
            }
            stack.pop();
        }
        "swap" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("swap", 2, n));
            }
            stack.swap(n - 1, n - 2);
        }
        "over" => {
            let n = stack.len();
            if n < 2 {
                return Err(need("over", 2, n));
            }
            let below = stack[n - 2];
            stack.push(below);
        }
        "rot" => {
            let n = stack.len();
            if n < 3 {
                return Err(need("rot", 3, n));
            }
            // a b c -> b c a
            let a = stack[n - 3];
            stack[n - 3] = stack[n - 2];
            stack[n - 2] = stack[n - 1];
            stack[n - 1] = a;
        }
        _ => return Ok(None),
    }
    Ok(Some(std::mem::take(stack)))
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
        let src = ": oops ( i64 -- i64 )\n  | a | a a + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("oops"));
        assert!(err.contains("`+`"));
        assert!(err.contains("needs 2 values"));
        assert!(err.contains("holds 1"));
        assert!(err.contains("( i64 -- i64 )"));
    }

    #[test]
    fn check_branch_depth_mismatch_is_error() {
        let src = ": w ( bool -- i64 ) if 1 1 else 1 then ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different stack depths"));
    }

    #[test]
    fn check_declared_output_mismatch_is_error() {
        let src = ": w ( -- i64 ) 1 1 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("body leaves 2 values"));
        assert!(err.contains("declares 1 outputs"));
    }

    #[test]
    fn check_unknown_word_is_error() {
        let src = ": w ( i64 -- i64 ) frobnicate ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown word"));
        assert!(err.contains("frobnicate"));
    }

    #[test]
    fn check_locals_exceed_inputs_is_error() {
        let src = ": w ( i64 -- i64 ) | a b | a ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("locals bind"));
    }

    #[test]
    fn check_type_propagates_through_body_expected() {
        // `0 >` yields a bool that `if` consumes; both arms leave an i64.
        check_src(": sign ( i64 -- i64 ) 0 > if 1 else 0 then ;").unwrap();
    }

    #[test]
    fn check_if_condition_not_bool_is_error() {
        let src = ": w ( -- i64 ) 5 if 1 else 2 then ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("expected `bool`"), "unexpected message: {err}");
        assert!(err.contains("found `i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_operand_type_mismatch_is_error() {
        let src = ": w ( -- i64 ) true 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("expected `i64`"), "unexpected message: {err}");
        assert!(err.contains("found `bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_declared_output_type_mismatch_is_error() {
        let src = ": w ( i64 -- bool ) 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("type mismatch"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffle_dup_bool_is_type_transparent() {
        // `dup` of a `bool` yields two `bool`s and satisfies the declaration.
        check_src(": w ( bool -- bool bool ) dup ;").unwrap();
    }

    fn infer_src(src: &str, entry: &[Type]) -> Result<Vec<Type>, String> {
        let tokens = lex(src).unwrap();
        let terms = match crate::parser::parse_line(&tokens).unwrap() {
            crate::ast::Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        infer_line(&terms, entry, &builtin_table())
    }

    #[test]
    fn infer_line_net_effect_expected() {
        assert_eq!(infer_src("2 3 +", &[]).unwrap(), vec![Type::I64]);
    }

    #[test]
    fn infer_line_carries_entry_depth() {
        // `2 +` from a carried `i64`: the literal plus the carried slot are
        // consumed by `+`, leaving one `i64`.
        assert_eq!(infer_src("2 +", &[Type::I64]).unwrap(), vec![Type::I64]);
    }

    #[test]
    fn infer_line_carries_slot_types_expected() {
        // A comparison line leaves a `bool` on the carried stack.
        assert_eq!(infer_src("5 3 >", &[]).unwrap(), vec![Type::Bool]);
    }

    #[test]
    fn line_underflow_against_carried_stack_is_error() {
        let err = infer_src("+", &[Type::I64]).unwrap_err();
        assert!(err.contains("stack underflow"), "unexpected message: {err}");
        assert!(err.contains("needs 2 values"), "unexpected message: {err}");
        assert!(err.contains("holds 1"), "unexpected message: {err}");
    }

    #[test]
    fn infer_line_unknown_word_is_error() {
        let err = infer_src("frobnicate", &[]).unwrap_err();
        assert!(err.contains("unknown word"), "unexpected message: {err}");
        assert!(err.contains("frobnicate"), "unexpected message: {err}");
    }
}
