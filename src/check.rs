//! Stack-effect checker. Simulates a compile-time virtual stack of concrete
//! `Type`s through each word body and verifies the net effect matches the
//! declared signature.
//!
//! Every operand is checked against the type its consumer expects, so a
//! `bool` where `+` wants an `i64` is a located compile error (Forth's silent
//! coercion failure mode becomes a diagnostic here). Branch join points unify
//! on both depth and per-slot type: the `then` and `else` arms must leave the
//! same stack shape.

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

/// The builtin word -> typed-effect table, as the seed of a checking env.
/// Every builtin word is structural and handled directly in `check_term`
/// (`check_shuffle`/`check_operator`): the stack shuffles, the numeric-tower
/// operators, and `.` (type-directed over any printable scalar, not a fixed
/// `( i64 -- )`) all dispatch on the concrete operand type rather than a
/// fixed signature, so this table is currently empty.
pub fn builtin_table() -> HashMap<String, Sig> {
    HashMap::new()
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

/// Both-operand type mismatch for a homogeneous operator (`+ - * = < >`):
/// mixed int/float, mixed integer widths/signs, mixed float widths, or a
/// `bool` operand, name both operand types (X1, X2).
fn operand_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same numeric type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `{op}` requires two operands of the same numeric type, found `{a}` and `{b}`"
        ),
    }
}

/// `/` applied to a non-float or mixed-float-type pair (X3): `/` is
/// float-only, integer division is unsupported.
fn div_requires_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `/` requires two operands of the same float type (integer division is unsupported), found `{}` and `{}`\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `/` requires two operands of the same float type (integer division is unsupported), found `{a}` and `{b}`"
        ),
    }
}

/// `mod` applied to a non-integer or mixed-integer-type pair (X4): `mod`
/// stays integer-only.
fn mod_requires_int_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `mod` requires two operands of the same integer type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `mod` requires two operands of the same integer type, found `{a}` and `{b}`"
        ),
    }
}

/// `and`/`or`/`xor` applied to a non-integer/non-bool or mixed-type pair:
/// bitwise ops are homogeneous over the integer types and `bool`, same shape
/// as `mod_requires_int_error`.
fn bitwise_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same integer or bool type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `{op}` requires two operands of the same integer or bool type, found `{a}` and `{b}`"
        ),
    }
}

/// `not` applied to a non-integer, non-bool operand.
fn bitwise_not_requires_int_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `not` requires an integer or bool operand, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `not` requires an integer or bool operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a non-integer value operand.
fn shift_value_requires_int_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an integer value operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `{op}` requires an integer value operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a shift count that is not `i64`.
fn shift_count_requires_i64_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an `i64` shift count, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: type mismatch: `{op}` requires an `i64` shift count, found `{found}`"
        ),
    }
}

/// A conversion word (`>iN`/`>uN`/`>f32`/`>f64`) applied to a non-numeric
/// (`bool`) source (X5).
fn conversion_source_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires a numeric source, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line => {
            format!("error: type mismatch: `{op}` requires a numeric source, found `{found}`")
        }
    }
}

/// `.` applied to a non-printable value. Every current frontend `Type` (the
/// integer tower, the float tower, `bool`) is printable, so this path has no
/// reachable golden yet; it exists for the day a non-printable scalar (e.g. a
/// future `Ptr`) enters the type system.
fn print_requires_printable_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `.` requires a printable scalar, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line => {
            format!("error: type mismatch: `.` requires a printable scalar, found `{found}`")
        }
    }
}

/// An unknown type name in a conversion word (X6), e.g. `>i128`.
fn conversion_unknown_type_error(ctx: &Ctx, span: Span, name: &str) -> String {
    match ctx {
        Ctx::Word { name: wname, .. } => format!(
            "error: unknown type `{name}` in `{wname}` (line {})",
            span.line
        ),
        Ctx::Line => format!("error: unknown type `{name}`"),
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

fn branch_type_mismatch_error(ctx: &Ctx, span: Span, t_then: Type, t_else: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `if` branches leave different types (then: `{}`, else: `{}`)\n  note: declared {}",
            name, span.line, t_then, t_else, effect_str(effect),
        ),
        Ctx::Line => format!(
            "error: `if` branches leave different types (then: `{t_then}`, else: `{t_else}`)"
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
        TermKind::FloatLit(_) => {
            stack.push(Type::F64);
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
            if let Some(stack) = check_operator(name, span, &mut stack, ctx)? {
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
            for (t_then, t_else) in then_stack.iter().zip(&else_stack) {
                if t_then != t_else {
                    return Err(branch_type_mismatch_error(ctx, span, *t_then, *t_else));
                }
            }
            Ok(then_stack)
        }
    }
}

/// Apply an arithmetic/comparison/conversion operator if `name` is one,
/// returning `Some(stack)`; `None` if the name is none of those (the caller
/// then looks it up in the env). `+ - *` are homogeneous over the numeric
/// types (int or float, `bool` is never numeric): both operands must be the
/// *same* type, producing that type; no implicit promotion (R6). `/` is
/// float-only: both operands must be the same float type (R7). `mod` stays
/// integer-only: both operands must be the same integer type (R8). `= < >`
/// generalise the same way as `+ - *` but always produce `bool` (R9). A
/// conversion word is `>` followed by a known numeric type name
/// (`>i8`..`>u64`, `>f32`, `>f64`): pop one numeric value, push the named
/// target (R10). `and`/`or`/`xor` are homogeneous over the integer types and
/// `bool` (float is rejected), same shape as `mod`; on two `bool`s they *are*
/// logical and/or/xor, since a stack language evaluates both operands eagerly
/// so bitwise-on-0/1 and logical coincide. `not` is unary: integer or `bool`
/// in, same type out (int stays bitwise complement, `bool` is logical
/// negation; the difference is only in how `lower_call` codegens it).
/// `shl`/`shr` take an integer value and always an `i64` shift count,
/// producing the value's type. `<= >= <>` generalise the same way as `= < >`:
/// numeric-only (never `bool`), same type, producing `bool`. `.` is
/// type-directed over any printable scalar (every integer width, either
/// float width, or `bool`): pops one, produces nothing; the concrete type
/// picks the print codegen (signed/unsigned decimal, `%g` float, or
/// `true`/`false`) at the call site, same dispatch shape as the rest of this
/// function.
fn check_operator(
    name: &str,
    span: Span,
    stack: &mut Vec<Type>,
    ctx: &Ctx,
) -> Result<Option<Vec<Type>>, String> {
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    match name {
        "+" | "-" | "*" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.is_numeric() || !b.is_numeric() || a != b {
                return Err(operand_pair_mismatch_error(ctx, span, name, a, b));
            }
            stack.truncate(n - 2);
            stack.push(a);
        }
        "/" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.is_float() || !b.is_float() || a != b {
                return Err(div_requires_float_error(ctx, span, a, b));
            }
            stack.truncate(n - 2);
            stack.push(a);
        }
        "mod" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.is_int() || !b.is_int() || a != b {
                return Err(mod_requires_int_error(ctx, span, a, b));
            }
            stack.truncate(n - 2);
            stack.push(a);
        }
        "and" | "or" | "xor" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !(a.is_int() || a.is_bool()) || !(b.is_int() || b.is_bool()) || a != b {
                return Err(bitwise_pair_mismatch_error(ctx, span, name, a, b));
            }
            stack.truncate(n - 2);
            stack.push(a);
        }
        "not" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(name, 1, n));
            }
            let a = stack[n - 1];
            if !(a.is_int() || a.is_bool()) {
                return Err(bitwise_not_requires_int_error(ctx, span, a));
            }
        }
        "shl" | "shr" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.is_int() {
                return Err(shift_value_requires_int_error(ctx, span, name, a));
            }
            if b != Type::I64 {
                return Err(shift_count_requires_i64_error(ctx, span, name, b));
            }
            stack.truncate(n - 2);
            stack.push(a);
        }
        "=" | "<" | ">" | "<=" | ">=" | "<>" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.is_numeric() || !b.is_numeric() || a != b {
                return Err(operand_pair_mismatch_error(ctx, span, name, a, b));
            }
            stack.truncate(n - 2);
            stack.push(Type::Bool);
        }
        "." => {
            let n = stack.len();
            if n < 1 {
                return Err(need(".", 1, n));
            }
            let a = stack[n - 1];
            if !a.is_numeric() && !a.is_bool() {
                return Err(print_requires_printable_error(ctx, span, a));
            }
            stack.truncate(n - 1);
        }
        _ => {
            let Some(rest) = name.strip_prefix('>').filter(|r| !r.is_empty()) else {
                return Ok(None);
            };
            let target = match Type::from_name(rest) {
                Some(ty) if ty.is_numeric() => ty,
                _ => return Err(conversion_unknown_type_error(ctx, span, rest)),
            };
            let source = *stack.last().ok_or_else(|| need(name, 1, stack.len()))?;
            if !source.is_numeric() {
                return Err(conversion_source_error(ctx, span, name, source));
            }
            stack.pop();
            stack.push(target);
        }
    }
    Ok(Some(std::mem::take(stack)))
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
    fn check_branch_join_types_agree_ok() {
        // Both arms leave a single `i64`: the join unifies cleanly.
        check_src(": w ( bool -- i64 ) if 1 else 2 then ;").unwrap();
    }

    #[test]
    fn check_branch_join_type_mismatch_is_error() {
        // `then` leaves an `i64`, `else` leaves a `bool`: same depth, different type.
        let src = ": w ( bool -- i64 ) if 1 else true then ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different types"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
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
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
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

    #[test]
    fn check_arith_same_width_ok() {
        check_src(": w ( -- i32 ) 1 >i32 2 >i32 + ;").unwrap();
    }

    #[test]
    fn check_arith_mixed_width_is_error() {
        // An `i32` and an `i64` fed to `+` names both differing types, via
        // the operand-pair-mismatch diagnostic specifically (not just any error
        // that happens to mention both type names).
        let src = ": f ( -- i32 ) 1 >i32 5 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_mixed_sign_is_error() {
        // `u8` and `i8` fed to `<` names both differing operand types, via
        // the same operand-pair-mismatch diagnostic.
        let src = ": w ( -- bool ) 200 >u8 5 >i8 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`u8`"), "unexpected message: {err}");
        assert!(err.contains("`i8`"), "unexpected message: {err}");
    }

    #[test]
    fn check_arith_mixed_int_float_is_error() {
        // X1: mixed int/float arithmetic names both operand types.
        let src = ": f ( -- f64 ) 1 >i32 5.0 + ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_mixed_float_width_is_error() {
        // X2: mixed float-width comparison names both operand types.
        let src = ": w ( -- bool ) 1.0 >f32 2.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_div_same_float_type_ok() {
        check_src(": w ( -- f64 ) 1.0 2.0 / ;").unwrap();
    }

    #[test]
    fn check_div_on_ints_is_error() {
        // X3: `/` requires floats; integer operands are a sharp error.
        let src = ": w ( -- i64 ) 4 2 / ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`/`"), "unexpected message: {err}");
        assert!(err.contains("float"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_mod_same_int_type_ok() {
        check_src(": w ( -- i64 ) 5 2 mod ;").unwrap();
    }

    #[test]
    fn check_mod_on_floats_is_error() {
        // X4: `mod` requires integers; float operands are a sharp error.
        let src = ": w ( -- f64 ) 5.0 2.0 mod ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`mod`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_or_xor_same_type_ok() {
        check_src(": w ( -- i32 ) 1 >i32 2 >i32 and 3 >i32 or 4 >i32 xor ;").unwrap();
    }

    #[test]
    fn check_bitwise_and_mixed_width_is_error() {
        let src = ": w ( -- i64 ) 1 >i32 2 and ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same integer or bool type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_or_xor_on_bool_is_ok() {
        // Bool is now an accepted homogeneous operand class for `and`/`or`/`xor`
        // (logical-and on two 0/1 bools coincides with bitwise-and).
        check_src(": w ( -- bool ) true false and true false or drop true false xor drop ;")
            .unwrap();
    }

    #[test]
    fn check_bitwise_and_mixed_bool_int_is_error() {
        let src = ": w ( -- bool ) true 5 and ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same integer or bool type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`bool`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_bitwise_and_on_float_is_error() {
        let src = ": w ( -- f64 ) 3.0 5.0 and ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_not_same_type_ok() {
        check_src(": w ( -- u8 ) 5 >u8 not ;").unwrap();
    }

    #[test]
    fn check_not_on_float_is_error() {
        let src = ": w ( -- f64 ) 3.0 not ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`not`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_not_on_bool_is_ok() {
        // `not` is type-directed: on a `bool` it is logical negation, not
        // the integer bitwise complement (R9-ext).
        check_src(": w ( -- bool ) true not ;").unwrap();
    }

    #[test]
    fn check_cmp_le_ge_ne_numeric_same_type_ok() {
        check_src(": w ( -- bool bool bool ) 1 2 <= 1 2 >= 1 2 <> ;").unwrap();
    }

    #[test]
    fn check_cmp_le_ge_ne_on_bool_is_error() {
        // Comparisons stay numeric-only: `bool` is never accepted, even
        // though it now is for `and`/`or`/`xor`.
        let src = ": w ( -- bool ) true false <= ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_cmp_ne_mixed_type_is_error() {
        let src = ": w ( -- bool ) 1 >i32 2 <> ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("same numeric type"),
            "unexpected message: {err}"
        );
        assert!(err.contains("`i32`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shl_shr_i64_count_ok() {
        check_src(": w ( -- u8 ) 1 >u8 3 shl ;").unwrap();
        check_src(": w ( -- u8 ) 200 >u8 3 shr ;").unwrap();
    }

    #[test]
    fn check_shl_count_not_i64_is_error() {
        let src = ": w ( -- u8 ) 1 >u8 3 >i32 shl ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`shl`"), "unexpected message: {err}");
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`i32`"), "unexpected message: {err}");
    }

    #[test]
    fn check_shr_value_not_int_is_error() {
        let src = ": w ( -- f64 ) 3.0 2 shr ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`shr`"), "unexpected message: {err}");
        assert!(err.contains("integer"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_int_to_float_ok() {
        check_src(": w ( -- f64 ) 5 >f64 ;").unwrap();
    }

    #[test]
    fn check_conv_float_to_int_ok() {
        check_src(": w ( -- i64 ) 5.0 >i64 ;").unwrap();
    }

    #[test]
    fn check_conv_float_target_of_bool_is_error() {
        // X5: a conversion to a float target applied to a `bool` source.
        let src = ": w ( -- f64 ) true >f64 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_unknown_float_target_is_error() {
        // X6: `>f128` reads as an unknown conversion target.
        let src = ": w ( -- f64 ) 5.0 >f128 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("f128"), "unexpected message: {err}");
    }

    #[test]
    fn check_float_lit_types_as_f64() {
        check_src(": w ( -- f64 ) 3.14 ;").unwrap();
    }

    #[test]
    fn check_branch_join_float_widths_mismatch_is_error() {
        // `if` branches leaving `f32` vs `f64` disagree at the join (R12).
        let src = ": w ( bool -- f64 ) if 1.0 >f32 else 2.0 then ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("different types"), "unexpected message: {err}");
        assert!(err.contains("`f32`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }

    #[test]
    fn check_branch_join_float_types_agree_ok() {
        check_src(": w ( bool -- f64 ) if 1.0 else 2.0 then ;").unwrap();
    }

    #[test]
    fn check_shuffle_dup_float_is_type_transparent() {
        check_src(": w ( -- f64 f64 ) 1.0 dup ;").unwrap();
    }

    #[test]
    fn check_conv_from_any_int_ok() {
        check_src(": w ( -- u8 ) 5 >i32 >u8 ;").unwrap();
    }

    #[test]
    fn check_conv_of_bool_is_error() {
        // A conversion applied to `bool` is a type error (X5).
        let src = ": w ( -- i32 ) true >i32 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("numeric"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }

    #[test]
    fn check_declared_output_needs_conversion_is_error() {
        // X3: the literal is `i64`, the declared output is `u8`.
        let src = ": f ( -- u8 ) 5 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`i64`"), "unexpected message: {err}");
        assert!(err.contains("`u8`"), "unexpected message: {err}");
    }

    #[test]
    fn check_conv_unknown_target_is_error() {
        // X6: `>i128` reads as an unknown conversion target.
        // (this test predates R10's float target; kept for the integer case)
        let src = ": w ( -- i64 ) 5 >i128 ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("unknown type"), "unexpected message: {err}");
        assert!(err.contains("i128"), "unexpected message: {err}");
    }

    #[test]
    fn check_shuffle_dup_u8_is_transparent() {
        check_src(": w ( -- u8 u8 ) 5 >u8 dup ;").unwrap();
    }

    #[test]
    fn check_shuffle_swap_mixed_types_is_type_transparent() {
        // `swap` reorders a mixed `bool`/`i64` pair with no fixed signature.
        check_src(": w ( bool i64 -- i64 bool ) swap ;").unwrap();
    }

    #[test]
    fn check_print_accepts_every_printable_scalar() {
        // `.` is type-directed over the whole integer tower, both float
        // widths, and `bool`, not just `i64`.
        check_src(": w ( -- ) 5 . ;").unwrap();
        check_src(": w ( -- ) 5 >u8 . ;").unwrap();
        check_src(": w ( -- ) 5 >i32 . ;").unwrap();
        check_src(": w ( -- ) -1 >u64 . ;").unwrap();
        check_src(": w ( -- ) 3.14 . ;").unwrap();
        check_src(": w ( -- ) 3.14 >f32 . ;").unwrap();
        check_src(": w ( -- ) true . ;").unwrap();
    }

    #[test]
    fn check_print_on_empty_stack_is_underflow_error() {
        let src = ": w ( -- ) . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`.`"), "unexpected message: {err}");
        assert!(err.contains("needs 1 values"), "unexpected message: {err}");
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
