use super::*;

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
/// type-directed over any primitive printable scalar (every integer width or
/// either float width): pops one, produces nothing; the concrete type picks
/// the print codegen (signed/unsigned decimal, or `%g` float) at the call
/// site. `bool` is not a row here (slice 9 R6): `true .`/`false .` fall
/// through to the injected library overload below, which prints
/// `true`/`false` by delegating to the `str` row.
/// The outcome of resolving an operator name against `BUILTIN_TABLE` and, on a
/// builtin-row exact miss, an optional same-named user overload (slice 8a
/// phase 2, R6). The single resolution entry point both `check_term`'s probe
/// chain and `poly_delegate_op` route through.
pub(super) enum OpDispatch {
    /// Resolved to a builtin row (or the `>T` conversion), carrying the new
    /// stack; the caller pushes nothing further.
    Builtin(Vec<Slot>),
    /// A user overload of this builtin name matched the operands exactly and
    /// beats the numeric coercion fallback (R2). Carries the candidate's
    /// lowering symbol, which the caller records for the site (R7) before
    /// dispatching through the ordinary `env` word-call path; the stack is
    /// left untouched here.
    UserOverload(String),
    /// The name is not a table operator (nor a `>T` conversion): fall through
    /// to the next probe in the chain.
    NotOperator,
}

/// The target type name of a `>T` numeric conversion word (`>i8` -> `i8`), or
/// `None` if `name` is not one.
///
/// `>=` is excluded by hand. It is spelled with a leading `>` but is a
/// comparison, not a conversion; while it was a `BUILTIN_TABLE` row the table
/// lookup claimed it before the prefix test ever ran, so the collision was
/// latent. Slice 10c (R-P3-3) moves `>=` out of the table and into `lib/`,
/// which exposes it: without this filter a bare `>=` is read as a conversion
/// to a type named `=` and rejected with `` unknown type `=` `` instead of
/// reaching the library word.
fn conversion_target_name(name: &str) -> Option<&str> {
    name.strip_prefix('>')
        .filter(|rest| !rest.is_empty() && *rest != "=")
}

fn is_conversion_name(name: &str) -> bool {
    conversion_target_name(name).is_some()
}

pub(super) fn check_operator(
    name: &str,
    span: Span,
    stack: &mut Vec<Slot>,
    ctx: &Ctx,
    user_overload: Option<&[Overload]>,
) -> Result<OpDispatch, String> {
    // R11: every operator this function handles reads the top slot, so a
    // quotation on top is always an operand of it. Guard once, gated on the
    // name being one we handle (else fall through so a later dispatcher can
    // claim it), before the type-directed reads that would otherwise spell the
    // `Cstr` placeholder into a mismatch.
    // This name list mirrors `BUILTIN_TABLE`'s keys (plus the `>T` conversions,
    // which are name-parsed, not table rows). Keep it in sync when a table
    // operator is added. It is not derived from `BUILTIN_TABLE.contains_key`
    // on purpose: the guard must also cover `>T`, and `is_unary` below can't be
    // read off row arity without changing `>=` (which the `>`-prefix test
    // already treats as unary here).
    let is_operator = matches!(
        name,
        "+" | "-"
            | "*"
            | "/"
            | "mod"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "shl"
            | "shr"
            | "u="
            | "u<"
            | "u>"
            | "u<="
            | "u>="
            | "u<>"
            | "max"
            | "max-total"
            | "."
    ) || is_conversion_name(name);
    // The unary members (`not`, print, the `>T` conversions) read only the
    // top; every other operator reads a pair, so its deeper operand at
    // `stack[n - 2]` is an operand of it too. Guarding the top alone lets a
    // quotation there fall through to `operand_pair_mismatch_error`, which
    // spells the `Cstr` placeholder into the message the audit exists to keep
    // hidden.
    let is_unary = matches!(name, "not" | ".") || is_conversion_name(name);
    if is_operator && stack.last().is_some_and(|s| s.quot.is_some()) {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    if is_operator && !is_unary && stack.len() >= 2 && stack[stack.len() - 2].quot.is_some() {
        return Err(reject_quotation_operand(ctx, span, name));
    }
    let need = |op: &str, n: usize, holds: usize| underflow_error(ctx, span, op, n, holds);
    // Unify a homogeneous binary op's operand pair, honoring D8's literal
    // coercion (`Ok`); `Err(Some(target))` is the size-type/computed-`i64`
    // X10 case, naming which size type (`usize`/`isize`) needed the explicit
    // conversion; `Err(None)` is a plain mismatch the caller reports with its
    // own op-specific diagnostic.
    let unify = |a: Slot, b: Slot| -> Result<Type, Option<Type>> {
        match unify_pair(a, b) {
            PairMatch::Ok(ty) => Ok(ty),
            PairMatch::NeedsSizeConversion(target) => Err(Some(target)),
            PairMatch::Mismatch => Err(None),
        }
    };

    // Slice 8a (R6/Q-A): dispatch selection is table-driven. Every operator
    // this function handles has one or more concrete rows in `BUILTIN_TABLE`;
    // a call resolves by an exact operand-type lookup there first, so a user
    // overload of the name can later shadow a call site (phase 2). Only on an
    // exact miss does the numeric operand-class guard + `unify_pair` coercion
    // below run, as a hand-written fallback whose diagnostics are preserved
    // byte-for-byte (Q-B). `not`'s in-place identity became a `(T -- T)` row,
    // so the exact hit pushes a fresh slot; the corpus never feeds a literal
    // `not` to a compile-time count, so the dropped literal flag is invisible.
    let Some(rows) = BUILTIN_TABLE.get(name) else {
        // Not a table operator: the `>T` numeric conversions stay hand-written
        // (R0), dispatched by parsing the target type out of the name rather
        // than keyed on operand type, so no row can hold them.
        let Some(rest) = conversion_target_name(name) else {
            return Ok(OpDispatch::NotOperator);
        };
        let target = match Type::from_name(rest) {
            Some(ty) if ty.is_numeric() => ty,
            _ => return Err(conversion_unknown_type_error(ctx, span, rest)),
        };
        let source = *stack.last().ok_or_else(|| need(name, 1, stack.len()))?;
        if !source.ty.is_numeric() {
            return Err(conversion_source_error(ctx, span, name, source.ty));
        }
        stack.pop();
        stack.push(Slot::computed(target));
        return Ok(OpDispatch::Builtin(std::mem::take(stack)));
    };
    // Every row for one name agrees on arity (R4), so the first row's input
    // count is the operand count to read.
    let arity = rows[0].inputs.len();
    if stack.len() < arity {
        return Err(need(name, arity, stack.len()));
    }
    let base = stack.len() - arity;
    let operands: Vec<Type> = stack[base..].iter().map(|s| s.ty).collect();
    if let Some(hit) = rows.iter().find(|r| r.inputs == operands) {
        stack.truncate(base);
        stack.extend(hit.outputs.iter().map(|ty| Slot::computed(*ty)));
        return Ok(OpDispatch::Builtin(std::mem::take(stack)));
    }

    // Slice 8a phase 2 (R2/R6): a user overload of this builtin name whose
    // inputs match the operands exactly beats the numeric coercion fallback
    // below, so a call the builtin already answers is untouched (corpus
    // byte-for-byte, since no corpus word overloads a builtin name) while a
    // `Vec2 +` site is redirected to the user word. Checked only on a
    // builtin-row exact miss. Both callers pass their candidate set, so a
    // poly body's delegated operator resolves an overload the same way a
    // monomorphic call site does.
    if let Some(candidates) = user_overload {
        if let Some(chosen) = resolve_overload(candidates, &operands) {
            return Ok(OpDispatch::UserOverload(chosen.symbol.clone()));
        }
    }

    match name {
        "+" | "-" | "*" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_numeric() || !b.ty.is_numeric() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "/" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_float() || !b.ty.is_float() || a.ty != b.ty {
                return Err(div_requires_float_error(ctx, span, a.ty, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "mod" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_int() || !b.ty.is_int() {
                return Err(mod_requires_int_error(ctx, span, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => mod_requires_int_error(ctx, span, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "and" | "or" | "xor" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !(a.ty.is_int() || a.ty.is_bool()) || !(b.ty.is_int() || b.ty.is_bool()) {
                return Err(bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => bitwise_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        "not" => {
            let n = stack.len();
            if n < 1 {
                return Err(need(name, 1, n));
            }
            let a = stack[n - 1];
            if !(a.ty.is_int() || a.ty.is_bool()) {
                return Err(bitwise_not_requires_int_error(ctx, span, a.ty));
            }
        }
        "shl" | "shr" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_int() {
                return Err(shift_value_requires_int_error(ctx, span, name, a.ty));
            }
            if b.ty != Type::I64 {
                return Err(shift_count_requires_i64_error(ctx, span, name, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "u=" | "u<" | "u>" | "u<=" | "u>=" | "u<>" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_numeric() || !b.ty.is_numeric() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(Type::U32));
        }
        // R12 (S6): `max ( 'T 'T -- 'T )`, an internal `Ord` bound resolved
        // against the integer tower (`is_int`, which already includes
        // `usize`/`isize`, D7). A float pair is rejected by name (X9),
        // directing to `max-total` (R13) rather than pretending IEEE `>` is
        // total (D6); the pair must still agree on one concrete type exactly
        // like `+`/`>`.
        "max" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if a.ty.is_float() || b.ty.is_float() {
                return Err(max_over_float_error(ctx, span, a.ty, b.ty));
            }
            if !a.ty.is_int() || !b.ty.is_int() {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            let ty = unify(a, b).map_err(|size_target| match size_target {
                Some(target) => size_conversion_needed_error(ctx, span, name, target),
                None => operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty),
            })?;
            stack.truncate(n - 2);
            stack.push(Slot::computed(ty));
        }
        // R13 (S6): `max-total ( 'F 'F -- 'F )`, `f32`/`f64` only, ordered by
        // the `total_cmp` bit-pattern rule rather than IEEE `>` (D6). An
        // integer pair is rejected by name (X10), directing to `max`.
        "max-total" => {
            let n = stack.len();
            if n < 2 {
                return Err(need(name, 2, n));
            }
            let (a, b) = (stack[n - 2], stack[n - 1]);
            if !a.ty.is_float() || !b.ty.is_float() {
                return Err(max_total_requires_float_error(ctx, span, a.ty, b.ty));
            }
            if a.ty != b.ty {
                return Err(operand_pair_mismatch_error(ctx, span, name, a.ty, b.ty));
            }
            stack.truncate(n - 2);
            stack.push(Slot::computed(a.ty));
        }
        "." => {
            let n = stack.len();
            if n < 1 {
                return Err(need(".", 1, n));
            }
            let a = stack[n - 1];
            if !a.ty.is_numeric() && !a.ty.is_bool() && !matches!(a.ty, Type::Str | Type::Cstr) {
                return Err(print_requires_printable_error(ctx, span, a.ty));
            }
            stack.truncate(n - 1);
        }
        _ => unreachable!("BUILTIN_TABLE holds only these operator names"),
    }
    Ok(OpDispatch::Builtin(std::mem::take(stack)))
}

/// Both-operand type mismatch for a homogeneous operator (`+ - * = < >`):
/// mixed int/float, mixed integer widths/signs, mixed float widths, or a
/// `bool` operand, name both operand types (X1, X2).
fn operand_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same numeric type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
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
        Ctx::Line { .. } => format!(
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
        Ctx::Line { .. } => format!(
            "error: type mismatch: `mod` requires two operands of the same integer type, found `{a}` and `{b}`"
        ),
    }
}

/// `max` applied to a float operand (X9): `max` is integer-only (D6);
/// naming `max-total` is the point of the message, not just the mismatch.
fn max_over_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `max` does not support float operands (found `{}` and `{}`); use `max-total` for a total-ordered float maximum\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `max` does not support float operands (found `{a}` and `{b}`); use `max-total` for a total-ordered float maximum"
        ),
    }
}

/// `max-total` applied to a non-float or mixed-float-type pair (X10):
/// `max-total` is float-only; naming `max` is the point of the message.
fn max_total_requires_float_error(ctx: &Ctx, span: Span, a: Type, b: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `max-total` requires two operands of the same float type, found `{}` and `{}`; use `max` for integers\n  note: declared {}",
            name, span.line, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
            "error: type mismatch: `max-total` requires two operands of the same float type, found `{a}` and `{b}`; use `max` for integers"
        ),
    }
}

/// `and`/`or`/`xor` applied to a non-integer/non-bool or mixed-type pair:
/// bitwise ops are homogeneous over the integer types and `bool`, same shape
/// as `mod_requires_int_error`.
fn bitwise_pair_mismatch_error(ctx: &Ctx, span: Span, op: &str, a: Type, b: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires two operands of the same integer or bool type, found `{}` and `{}`\n  note: declared {}",
            name, span.line, op, a, b, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
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
        Ctx::Line { .. } => format!(
            "error: type mismatch: `not` requires an integer or bool operand, found `{found}`"
        ),
    }
}

/// `shl`/`shr` applied to a non-integer value operand.
fn shift_value_requires_int_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires an integer value operand, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => format!(
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
        Ctx::Line { .. } => format!(
            "error: type mismatch: `{op}` requires an `i64` shift count, found `{found}`"
        ),
    }
}

/// A conversion word (`>iN`/`>uN`/`>f32`/`>f64`) applied to a non-numeric
/// (`bool`) source (X5).
fn conversion_source_error(ctx: &Ctx, span: Span, op: &str, found: Type) -> String {
    let op = crate::resolve::demangle_call(op);
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `{}` requires a numeric source, found `{}`\n  note: declared {}",
            name, span.line, op, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
            format!("error: type mismatch: `{op}` requires a numeric source, found `{found}`")
        }
    }
}

/// `.` applied to a non-printable value. Every current primitive `Type` (the
/// integer tower, the float tower) is printable via a builtin row, and `bool`
/// is printable via the library overload injected by `bool_print_word_def`,
/// so this path has no reachable golden yet; it exists for the day a
/// non-printable scalar (e.g. a future `Ptr`) enters the type system.
fn print_requires_printable_error(ctx: &Ctx, span: Span, found: Type) -> String {
    match ctx {
        Ctx::Word { name, effect, .. } => format!(
            "error: type mismatch in `{}` (line {})\n  `.` requires a printable scalar, found `{}`\n  note: declared {}",
            name, span.line, found, effect_str(effect),
        ),
        Ctx::Line { .. } => {
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
        Ctx::Line { .. } => format!("error: unknown type `{name}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let tokens = lex(src).unwrap();
        let mut module = parse(&tokens).unwrap();
        check(&mut module)
    }
    /// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
    /// primitive in Slice 8c: an ordinary one-field struct with a `drop`
    /// overload, so it is linear for the same reason any resource is (R3),
    /// not by any compiler-known bit. Always the first struct in a source
    /// string that uses it, so every other struct's `StructId` shifts up by
    /// one relative to a spy-free program.
    const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";
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
        // `u8` and `i8` fed to `<` names both differing operand types. Slice
        // 10c: `<` is a `'T: Copy Ord` library word now, so the rejection is
        // the variable-conflict one rather than the builtin operand-pair one;
        // both operand types are still named.
        let src = ": w ( -- bool ) 200 >u8 5 >i8 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("resolved `'T` to both"),
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
        // X2: mixed float-width comparison names both operand types (slice
        // 10c: through the library `<`'s variable conflict, see
        // `check_cmp_mixed_sign_is_error`).
        let src = ": w ( -- bool ) 1.0 >f32 2.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(
            err.contains("resolved `'T` to both"),
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
    fn check_max_same_int_type_ok() {
        check_src(": w ( -- i64 ) 3 5 max ;").unwrap();
    }
    #[test]
    fn check_max_on_floats_is_error() {
        // X9: `max` is integer-only; a float pair names `max-total`.
        let src = ": w ( -- f64 ) 3.0 5.0 max ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`max`"), "unexpected message: {err}");
        assert!(err.contains("`max-total`"), "unexpected message: {err}");
    }
    #[test]
    fn check_max_total_same_float_type_ok() {
        check_src(": w ( -- f64 ) 3.0 5.0 max-total ;").unwrap();
    }
    #[test]
    fn check_max_total_on_ints_is_error() {
        // X10: `max-total` is float-only; an integer pair names `max`.
        let src = ": w ( -- i64 ) 3 5 max-total ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`max-total`"), "unexpected message: {err}");
        assert!(err.contains("`max`"), "unexpected message: {err}");
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
        // Slice 10c: D8's literal coercion covers the two size types only, so
        // a fresh `2` beside an `i32` is still a mismatch; the rejection is
        // now the library `<>`'s variable conflict.
        let err = check_src(": w ( -- bool ) 1 >i32 2 <> ;").unwrap_err();
        assert!(
            err.contains("resolved `'T` to both"),
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
    fn check_usize_is_recognised_as_a_type_name() {
        check_src(": w ( -- usize ) 5 ;").unwrap();
    }
    #[test]
    fn check_usize_arithmetic_and_comparison_ok() {
        check_src(": w ( -- usize ) 5 3 >usize + ;").unwrap();
        check_src(": w ( -- bool ) 5 3 >usize < ;").unwrap();
    }
    #[test]
    fn check_usize_literal_coerces_into_usize_position_ok() {
        // D8: a bare integer literal fills a `usize` position on either side
        // of a homogeneous binary op, no `>usize` required.
        check_src(": w ( -- usize ) 3 >usize 5 + ;").unwrap();
        check_src(": w ( -- usize ) 5 3 >usize + ;").unwrap();
    }
    #[test]
    fn check_usize_computed_value_without_conversion_is_error() {
        // X10: `1 1 +` is a *computed* i64 (no constant folding), so mixing
        // it with a `usize` still needs an explicit `>usize`.
        let src = ": w ( -- usize ) 3 >usize 1 1 + + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }
    #[test]
    fn check_usize_to_int_and_int_to_usize_conversions_ok() {
        check_src(": w ( -- i64 ) 5 >usize >i64 ;").unwrap();
        check_src(": w ( -- usize ) 5 >usize ;").unwrap();
    }
    #[test]
    fn check_usize_print_is_type_directed_ok() {
        check_src(": w ( -- ) 5 >usize . ;").unwrap();
    }
    #[test]
    fn check_print_on_array_is_error() {
        // X6/R13: `.` on an array is a sharp located error naming `[T N]`.
        let err = check_src(": w ( -- ) 0 4 fill . ;").unwrap_err();
        assert!(err.contains("[i64 4]"), "should name the array type: {err}");
    }
    #[test]
    fn check_usize_mixed_with_bool_is_error() {
        // X9: `usize` mixed with a non-coercible operand (`bool`) names both.
        let src = ": w ( -- usize ) 5 >usize true and ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`bool`"), "unexpected message: {err}");
    }
    #[test]
    fn check_usize_mixed_with_float_is_error() {
        // X9: `usize` mixed with `f64` (both numeric, not coercible).
        let src = ": w ( -- bool ) 5 >usize 1.0 < ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`f64`"), "unexpected message: {err}");
    }
    #[test]
    fn check_usize_declared_output_needs_conversion_is_error() {
        // X10 at a declared-output position: a computed `i64` doesn't
        // silently satisfy a declared `usize` output.
        let src = ": w ( -- usize ) 1 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
    }
    #[test]
    fn check_isize_mixed_with_usize_is_error() {
        // `usize` and `isize` are sibling size types but do not coerce
        // into each other; mixing them is a plain type mismatch naming both
        // backticked types.
        let src = ": w ( -- bool ) 5 >usize 3 >isize < ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`usize`"), "unexpected message: {err}");
        assert!(err.contains("`isize`"), "unexpected message: {err}");
    }
    #[test]
    fn check_isize_declared_output_needs_conversion_is_error() {
        // X10 at a declared-output position, mirroring
        // check_usize_declared_output_needs_conversion_is_error: a computed
        // `i64` doesn't silently satisfy a declared `isize` output, and the
        // message names the backticked `isize` form rather than `usize`.
        let src = ": w ( -- isize ) 1 1 + ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`isize`"), "unexpected message: {err}");
        assert!(err.contains(">isize"), "unexpected message: {err}");
    }
    #[test]
    fn check_usize_branch_merge_keeps_computed_arm_non_coercible_is_error() {
        // A literal in one arm and a computed value in the other must NOT
        // merge to a coercible literal: on the computed arm's runtime path a
        // computed `i64` would fill the `usize` output without `>usize` (X10).
        for src in [
            ": w ( bool -- usize ) ~[ 5 ] ~[ 1 1 + ] if ;",
            ": w ( bool -- usize ) ~[ 1 1 + ] ~[ 5 ] if ;",
        ] {
            let err = check_src(src).unwrap_err();
            assert!(err.contains("usize"), "unexpected message: {err}");
            assert!(err.contains(">usize"), "unexpected message: {err}");
        }
    }
    #[test]
    fn check_usize_branch_merge_both_literals_coerces_ok() {
        // Both arms leave a literal, so the merged slot stays a coercible
        // literal and fills the `usize` output.
        check_src(": w ( bool -- usize ) ~[ 5 ] ~[ 6 ] if ;").unwrap();
    }
    #[test]
    fn check_usize_call_argument_literal_coerces_ok() {
        // A bare literal fills a declared `usize` parameter without `>usize`.
        let src = ": at ( usize -- usize ) ; : w ( -- usize ) 5 at ;";
        check_src(src).unwrap();
    }
    #[test]
    fn check_usize_call_argument_computed_needs_conversion_is_error() {
        let src = ": at ( usize -- usize ) ; : w ( -- usize ) 1 1 + at ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("usize"), "unexpected message: {err}");
        assert!(err.contains(">usize"), "unexpected message: {err}");
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
    fn check_not_on_literal_count_is_not_a_literal_for_fill() {
        // The retired hand-written `not` arm left its operand slot in place,
        // preserving `literal`/`int_val` (so a `not`'d literal fed to `fill`
        // would have used the *pre-negation* value, silently wrong). The
        // table row it was replaced with emits `Slot::computed`, so `fill`
        // now correctly refuses a `not`'d literal as a non-literal count
        // instead of miscounting.
        let err = check_src(": w ( -- ) 0 4 not fill drop ;").unwrap_err();
        assert!(err.contains("literal count"), "unexpected message: {err}");
    }
    #[test]
    fn check_print_accepts_str_and_cstr() {
        // `.`'s printable-scalar guard also accepts `str`/`cstr` (R9), matched
        // by name rather than `is_numeric`/`is_bool`, since neither is numeric.
        check_src(": w ( -- ) \"hi\" . ;").unwrap();
        check_src(": w ( -- ) \"hi\" cstr . ;").unwrap();
    }
    #[test]
    fn check_print_on_empty_stack_is_underflow_error() {
        let src = ": w ( -- ) . ;";
        let err = check_src(src).unwrap_err();
        assert!(err.contains("`.`"), "unexpected message: {err}");
        assert!(err.contains("needs 1 values"), "unexpected message: {err}");
    }
    #[test]
    fn check_print_on_linear_value_is_error() {
        // R16: `.` is a printable-scalar path, and a linear value is not one
        // (the backend's `unreachable!` guard depends on this).
        let err = check_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy . ;")).unwrap_err();
        assert!(err.contains("printable"), "unexpected message: {err}");
        assert!(err.contains("`Spy`"), "unexpected message: {err}");
    }
}
