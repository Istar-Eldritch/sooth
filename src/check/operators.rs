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
            | "="
            | "<"
            | ">"
            | "<="
            | ">="
            | "<>"
            | "max"
            | "max-total"
            | "."
    ) || name.strip_prefix('>').is_some_and(|r| !r.is_empty());
    // The unary members (`not`, print, the `>T` conversions) read only the
    // top; every other operator reads a pair, so its deeper operand at
    // `stack[n - 2]` is an operand of it too. Guarding the top alone lets a
    // quotation there fall through to `operand_pair_mismatch_error`, which
    // spells the `Cstr` placeholder into the message the audit exists to keep
    // hidden.
    let is_unary =
        matches!(name, "not" | ".") || name.strip_prefix('>').is_some_and(|r| !r.is_empty());
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
        let Some(rest) = name.strip_prefix('>').filter(|r| !r.is_empty()) else {
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
        "=" | "<" | ">" | "<=" | ">=" | "<>" => {
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
            stack.push(Slot::computed(Type::BOOL));
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
