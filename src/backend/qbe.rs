//! QBE backend: emit QBE IL text from the neutral IR.
//!
//! Driver then pipes this through `qbe` (-> assembly) and `cc` (-> native binary).
//! QBE gives arm64/x86_64/riscv64 and C-ABI struct classification for free; costs
//! accepted are i128 synthesised in the frontend and atomics via C11 FFI.

use std::fmt::Write;

use crate::ir::{BinOp, BlockId, CmpOp, Instr, IrFunc, IrModule, IrType, Terminator, Value};

pub fn emit(ir: &IrModule) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("data $fmt = { b \"%ld\\n\", b 0 }\n");
    for func in &ir.funcs {
        out.push('\n');
        emit_func(&mut out, func);
    }
    Ok(out)
}

/// The Sooth `main` word is emitted as `sooth_main`; the C shim owns `main`.
fn qbe_name(name: &str) -> &str {
    if name == "main" {
        "sooth_main"
    } else {
        name
    }
}

fn val(v: Value) -> String {
    format!("%v{}", v.0)
}

fn label(id: BlockId) -> String {
    if id.0 == 0 {
        "@start".to_string()
    } else {
        format!("@blk{}", id.0)
    }
}

/// The QBE base-type letter for an `IrType`, derived here (not in the IR, R15):
/// `Bool` is a 4-byte `w` (0/1); an integer is `w` for `bits <= 32` and `l` for
/// `64`; a float is `s` (32) or `d` (64); `Ptr` is the 8-byte `l` used by the
/// buffer and C ABI. This is the only place the `s`/`d` register class is
/// spelled (NF2).
fn width(ty: IrType) -> &'static str {
    match ty {
        IrType::Bool => "w",
        IrType::Int { bits, .. } => {
            if bits <= 32 {
                "w"
            } else {
                "l"
            }
        }
        IrType::Float { bits } => {
            if bits == 32 {
                "s"
            } else {
                "d"
            }
        }
        IrType::Ptr => "l",
    }
}

/// A sub-word integer type (`bits < 32`, i.e. `i8`/`i16`/`u8`/`u16`) whose value
/// can carry dirty high bits in its `w` register after a width-overflowing op.
/// `i32`/`u32` fill the `w` register exactly and need no canonicalization.
fn sub_word(ty: IrType) -> Option<(u8, bool)> {
    match ty {
        IrType::Int { bits, signed } if bits < 32 => Some((bits, signed)),
        _ => None,
    }
}

/// The single sub-word canonicalization point (R15): normalize `src`'s
/// out-of-width bits into `dst` at register width `w`. A signed type
/// sign-extends from its low `bits` (`extsb`/`extsh`); an unsigned type masks
/// off everything above `bits`. Every dirtying op (sub-word arithmetic here,
/// narrowing conversion in the conversion lowering) routes through this so no
/// two code paths disagree on a value's high bits.
fn emit_canonicalize(
    out: &mut String,
    dst: &str,
    src: &str,
    w: &str,
    bits: u8,
    signed: bool,
) -> std::fmt::Result {
    if signed {
        let ext = match bits {
            8 => "extsb",
            16 => "extsh",
            _ => unreachable!("sub_word only yields bits 8/16"),
        };
        writeln!(out, "\t{dst} ={w} {ext} {src}")
    } else {
        let mask = (1u32 << bits) - 1;
        writeln!(out, "\t{dst} ={w} and {src}, {mask}")
    }
}

/// Lower a numeric conversion `dst = convert(src)` (R18), dispatching on the
/// source/target `IrType` classes: int->int (the Slice-2 path, unchanged),
/// int->float, float->float, float->int. The frontend never spells the QBE op;
/// the register class (`s`/`d`) is derived here (NF2).
fn emit_conv(
    out: &mut String,
    dst: Value,
    src: Value,
    value_types: &[IrType],
    ext_id: &mut u32,
) -> std::fmt::Result {
    let src_ty = ty_of(value_types, src);
    let dst_ty = ty_of(value_types, dst);
    match (src_ty, dst_ty) {
        (IrType::Int { .. }, IrType::Int { .. }) => {
            emit_conv_int(out, dst, src, value_types, ext_id)
        }
        (
            IrType::Int {
                bits: sb,
                signed: ss,
            },
            IrType::Float { .. },
        ) => {
            // int -> float: the mnemonic picks source width (`w` for bits <= 32,
            // `l` for 64) and source signedness; the result letter (`s`/`d`)
            // selects the target float width. A sub-word source is already
            // canonical in its `w` carrier (R15), so `swtof`/`uwtof` read it
            // directly. Exact when representable, else round to nearest.
            let dw = width(dst_ty);
            let op = match (sb <= 32, ss) {
                (true, true) => "swtof",
                (true, false) => "uwtof",
                (false, true) => "sltof",
                (false, false) => "ultof",
            };
            writeln!(out, "\t{} ={dw} {op} {}", val(dst), val(src))
        }
        (IrType::Float { bits: sb }, IrType::Float { bits: db }) => {
            // float -> float: widen is exact (`exts`), narrow rounds to nearest
            // (`truncd`); a same-width `>fN` on its own type is a bit relabel.
            let dw = width(dst_ty);
            let m = if db > sb {
                "exts"
            } else if db < sb {
                "truncd"
            } else {
                "copy"
            };
            writeln!(out, "\t{} ={dw} {m} {}", val(dst), val(src))
        }
        (IrType::Float { bits: sb }, IrType::Int { .. }) => {
            // float -> int: truncate toward zero to the 32/64 integer carrier
            // (`stosi`/`dtosi` signed, `stoui`/`dtoui` unsigned), then the
            // shared canonicalization point (R15) for a sub-word target.
            // Out-of-range/NaN is unspecified this slice (D7).
            let ds = matches!(dst_ty, IrType::Int { signed: true, .. });
            let op = match (sb == 32, ds) {
                (true, true) => "stosi",
                (true, false) => "stoui",
                (false, true) => "dtosi",
                (false, false) => "dtoui",
            };
            match sub_word(dst_ty) {
                Some((bits, signed)) => {
                    let tmp = format!("%conv{ext_id}");
                    *ext_id += 1;
                    writeln!(out, "\t{tmp} =w {op} {}", val(src))?;
                    emit_canonicalize(out, &val(dst), &tmp, "w", bits, signed)
                }
                None => {
                    let dw = width(dst_ty);
                    writeln!(out, "\t{} ={dw} {op} {}", val(dst), val(src))
                }
            }
        }
        (s, d) => unreachable!("conversion endpoints are numeric, got {s:?} -> {d:?}"),
    }
}

/// Lower an integer conversion `dst = convert(src)` (R6), the Slice-2 path
/// unchanged. Widening extends by the *source* signedness (`exts*` signed,
/// `extu*` unsigned) from the source width; if the *target* is sub-word, that
/// extend is re-canonicalized to the target's own convention (R15), because the
/// source-signed extend is only accidentally canonical for the target: a signed
/// source widened to an unsigned sub-word target (e.g. `i8 >u16`) sign-extends
/// into bits the target requires to be zero, which a later in-register unsigned
/// compare would read as dirty. Narrowing keeps the low `dst` bits: for a
/// sub-word target that routes through the shared canonicalization point (R15),
/// otherwise a `w`-width `copy` truncates a `64 -> 32` step. Same-width is a
/// relabel: a plain `copy` when the target fills its register, but a sub-word
/// signedness flip (`u8 >i8`, `i8 >u8`) still re-canonicalizes to the new
/// convention so a later widen/compare reads the right high bits (Q5).
fn emit_conv_int(
    out: &mut String,
    dst: Value,
    src: Value,
    value_types: &[IrType],
    ext_id: &mut u32,
) -> std::fmt::Result {
    let db = match ty_of(value_types, dst) {
        IrType::Int { bits, .. } => bits,
        other => unreachable!("conversion target is always an integer, got {other:?}"),
    };
    let (sb, ss) = match ty_of(value_types, src) {
        IrType::Int { bits, signed } => (bits, signed),
        other => unreachable!("conversion source is always an integer, got {other:?}"),
    };
    let dw = width(ty_of(value_types, dst));
    if db > sb {
        // Widen: sign-/zero-extend from the source width by the source sign.
        let ext = match (sb, ss) {
            (8, true) => "extsb",
            (8, false) => "extub",
            (16, true) => "extsh",
            (16, false) => "extuh",
            (32, true) => "extsw",
            (32, false) => "extuw",
            _ => unreachable!("widening source is 8/16/32 bits, got {sb}"),
        };
        match sub_word(ty_of(value_types, dst)) {
            Some((bits, signed)) => {
                let tmp = format!("%widen{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{tmp} ={dw} {ext} {}", val(src))?;
                emit_canonicalize(out, &val(dst), &tmp, dw, bits, signed)
            }
            None => writeln!(out, "\t{} ={dw} {ext} {}", val(dst), val(src)),
        }
    } else {
        // Narrow or same-width: the value already sits in `src`'s low `db` bits.
        // Canonicalize a sub-word target; otherwise a `copy` fills (and, for a
        // `64 -> 32` narrowing, truncates) the register.
        match sub_word(ty_of(value_types, dst)) {
            Some((bits, signed)) => emit_canonicalize(out, &val(dst), &val(src), dw, bits, signed),
            None => writeln!(out, "\t{} ={dw} copy {}", val(dst), val(src)),
        }
    }
}

fn ty_of(value_types: &[IrType], v: Value) -> IrType {
    value_types[v.0 as usize]
}

fn emit_func(out: &mut String, func: &IrFunc) {
    let ret_ty = match func.ret {
        Some(ty) => format!("{} ", width(ty)),
        None => String::new(),
    };
    let params: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(i, ty)| format!("{} %v{i}", width(*ty)))
        .collect();
    writeln!(
        out,
        "export function {ret_ty}${}({}) {{",
        qbe_name(&func.name),
        params.join(", ")
    )
    .unwrap();
    let mut ext_id = 0u32;
    for block in &func.blocks {
        writeln!(out, "{}", label(block.id)).unwrap();
        for instr in &block.instrs {
            emit_instr(out, instr, &func.value_types, &mut ext_id);
        }
        emit_term(out, &block.term);
    }
    out.push_str("}\n");
}

fn emit_instr(out: &mut String, instr: &Instr, value_types: &[IrType], ext_id: &mut u32) {
    match instr {
        Instr::Const(v, n) => {
            let w = width(ty_of(value_types, *v));
            writeln!(out, "\t{} ={w} copy {n}", val(*v))
        }
        Instr::ConstF(v, x) => {
            // QBE float constants carry an `s_`/`d_` prefix; Rust's `f64`
            // `Display` renders round-trippable text QBE parses (R14).
            let ty = ty_of(value_types, *v);
            let w = width(ty);
            let prefix = if matches!(ty, IrType::Float { bits: 32 }) {
                "s_"
            } else {
                "d_"
            };
            writeln!(out, "\t{} ={w} copy {prefix}{x}", val(*v))
        }
        Instr::Bin(v, op, a, b) => {
            // The op runs at the result's register width; a sub-word result can
            // overflow its width, so canonicalize it (R15) via the shared point.
            let ty = ty_of(value_types, *v);
            let w = width(ty);
            let m = match op {
                BinOp::Add => "add",
                BinOp::Sub => "sub",
                BinOp::Mul => "mul",
                // `div` is emitted only for floats (no integer `/`, R16); it
                // runs at the operand's `s`/`d` width like the other arms.
                BinOp::Div => "div",
                BinOp::Rem if matches!(ty, IrType::Int { signed: false, .. }) => "urem",
                BinOp::Rem => "rem",
            };
            if let Some((bits, signed)) = sub_word(ty) {
                let tmp = format!("%bin{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{tmp} ={w} {m} {}, {}", val(*a), val(*b)).unwrap();
                emit_canonicalize(out, &val(*v), &tmp, w, bits, signed)
            } else {
                writeln!(out, "\t{} ={w} {m} {}, {}", val(*v), val(*a), val(*b))
            }
        }
        Instr::Cmp(v, op, a, b) => {
            // Signedness and operand width come from the operand type (R10),
            // not the result (always `Bool`/`w`): `<`/`>` pick signed
            // (`cslt`/`csgt`) vs unsigned (`cult`/`cugt`); `=` is
            // signedness-agnostic (`ceq`). The mnemonic's width suffix is the
            // operand width.
            let operand = ty_of(value_types, *a);
            let ow = width(operand);
            let is_float = matches!(operand, IrType::Float { .. });
            let signed = matches!(operand, IrType::Int { signed: true, .. });
            // Float compares are the ordered forms (`clt`/`cgt`/`ceq` + `s`/`d`),
            // false against NaN, so `x = x` is a valid NaN test (R17, RISK 1).
            let m = match op {
                CmpOp::Eq => "ceq",
                CmpOp::Lt if is_float => "clt",
                CmpOp::Lt if signed => "cslt",
                CmpOp::Lt => "cult",
                CmpOp::Gt if is_float => "cgt",
                CmpOp::Gt if signed => "csgt",
                CmpOp::Gt => "cugt",
            };
            let w = width(ty_of(value_types, *v));
            writeln!(out, "\t{} ={w} {m}{ow} {}, {}", val(*v), val(*a), val(*b))
        }
        Instr::Call(ret, f, args) => {
            let a: Vec<String> = args
                .iter()
                .map(|x| format!("{} {}", width(ty_of(value_types, *x)), val(*x)))
                .collect();
            match ret {
                Some(r) => {
                    let w = width(ty_of(value_types, *r));
                    writeln!(
                        out,
                        "\t{} ={w} call ${}({})",
                        val(*r),
                        qbe_name(f),
                        a.join(", ")
                    )
                }
                None => writeln!(out, "\tcall ${}({})", qbe_name(f), a.join(", ")),
            }
        }
        Instr::Print(v) => writeln!(out, "\tcall $printf(l $fmt, l {}, ...)", val(*v)),
        Instr::PtrOffset(dst, base, bytes) => {
            writeln!(out, "\t{} =l add {}, {bytes}", val(*dst), val(*base))
        }
        Instr::Load(dst, ptr) => writeln!(out, "\t{} =l loadl {}", val(*dst), val(*ptr)),
        Instr::Store(ptr, v) => {
            // The 8-byte buffer slot is always an `l` sink (R4); any `w`-width
            // value (`Bool`, or an integer with `bits <= 32`) must be widened to
            // `l` before it lands there. A signed integer sign-extends (its `w`
            // register already holds canonical, correctly-signed bits, R15);
            // `Bool` and an unsigned integer zero-extend.
            let ty = ty_of(value_types, *v);
            if width(ty) == "w" {
                let signed = matches!(ty, IrType::Int { signed: true, .. });
                let ext_op = if signed { "extsw" } else { "extuw" };
                let ext = format!("%ext{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{ext} =l {ext_op} {}", val(*v)).unwrap();
                writeln!(out, "\tstorel {}, {}", ext, val(*ptr))
            } else {
                writeln!(out, "\tstorel {}, {}", val(*v), val(*ptr))
            }
        }
        Instr::Conv(dst, src) => emit_conv(out, *dst, *src, value_types, ext_id),
        Instr::Phi(r, arms) => {
            let a: Vec<String> = arms
                .iter()
                .map(|(b, v)| format!("{} {}", label(*b), val(*v)))
                .collect();
            let w = width(ty_of(value_types, *r));
            writeln!(out, "\t{} ={w} phi {}", val(*r), a.join(", "))
        }
    }
    .unwrap();
}

fn emit_term(out: &mut String, term: &Terminator) {
    match term {
        Terminator::Ret(Some(v)) => writeln!(out, "\tret {}", val(*v)),
        Terminator::Ret(None) => writeln!(out, "\tret"),
        Terminator::Jnz(c, t, e) => {
            writeln!(out, "\tjnz {}, {}, {}", val(*c), label(*t), label(*e))
        }
        Terminator::Jmp(b) => writeln!(out, "\tjmp {}", label(*b)),
    }
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Line;
    use crate::ast::Type;
    use crate::check::check;
    use crate::ir::{lower, lower_line, IrModule};
    use crate::lexer::lex;
    use crate::parser::{parse, parse_line};
    use std::collections::HashMap;

    fn emit_src(src: &str) -> String {
        let tokens = lex(src).unwrap();
        let module = parse(&tokens).unwrap();
        check(&module).unwrap();
        let ir = lower(&module).unwrap();
        emit(&ir).unwrap()
    }

    fn emit_line(src: &str, entry_depth: usize) -> String {
        let tokens = lex(src).unwrap();
        let terms = match parse_line(&tokens).unwrap() {
            Line::Expr(terms) => terms,
            other => panic!("expected Expr, got {other:?}"),
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let entry_types = vec![Type::I64; entry_depth];
        let (func, _m) = lower_line(0, &terms, entry_depth, &entry_types, &env, &resolve);
        emit(&IrModule { funcs: vec![func] }).unwrap()
    }

    #[test]
    fn emit_square_contains_mul_and_ret() {
        let il = emit_src(": sq ( i64 -- i64 ) | n | n n * ;");
        assert!(il.contains("mul"));
        assert!(il.contains("ret "));
    }

    #[test]
    fn emit_print_uses_printf_and_fmt() {
        let il = emit_src(": w ( i64 -- ) . ;");
        assert!(il.contains("data $fmt = { b \"%ld\\n\", b 0 }"));
        assert!(il.contains("call $printf(l $fmt,"));
        assert!(il.contains(", ...)"));
    }

    #[test]
    fn emit_if_has_jnz_and_phi() {
        let il = emit_src(": w ( bool -- i64 ) if 1 else 2 then ;");
        assert!(il.contains("jnz "));
        assert!(il.contains("phi "));
    }

    #[test]
    fn emit_main_becomes_sooth_main() {
        let il = emit_src(": main ( -- ) 5 . ;");
        assert!(il.contains("$sooth_main"));
        assert!(!il.contains("$main("));
    }

    #[test]
    fn emit_bool_value_uses_w_width() {
        let il = emit_src(": w ( -- bool ) true ;");
        assert!(il.contains("=w copy 1"), "unexpected IL: {il}");
    }

    #[test]
    fn emit_comparison_line_stores_bool_via_extension() {
        // `5 3 >` from D=0 leaves a `bool` on top; the line-wrapper epilogue
        // must widen it (`extuw`) before the fixed 8-byte `storel` (R4/RK1).
        let il = emit_line("5 3 >", 0);
        assert!(il.contains("=w csgtl"), "unexpected IL: {il}");
        assert!(il.contains("extuw"), "expected a w->l extension: {il}");
        assert!(il.contains("storel"), "expected a storel: {il}");
    }

    #[test]
    fn emit_wrapper_signature_takes_stack_and_top() {
        let il = emit_line("2 3 +", 0);
        assert!(
            il.contains("export function l $sooth_line_0(l %v0, l %v1)"),
            "unexpected signature: {il}"
        );
    }

    #[test]
    fn emit_line_wrapper_has_load_and_store() {
        // `+` from a carried depth of 2 loads the two slots and stores the result.
        let il = emit_line("+", 2);
        assert!(il.contains("loadl "), "expected a load: {il}");
        assert!(il.contains("storel "), "expected a store: {il}");
    }

    fn int(bits: u8, signed: bool) -> IrType {
        IrType::Int { bits, signed }
    }

    /// Emit a single-block function over hand-built value types and instrs,
    /// returning `v2` (the result of a binary/compare op). Hand-built types
    /// isolate the bare sub-word/unsigned codegen path per operand pairing.
    fn emit_binary(operand: IrType, result: IrType, instr: Instr) -> String {
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![],
            ret: Some(result),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![Instr::Const(Value(0), 5), Instr::Const(Value(1), 3), instr],
                term: Terminator::Ret(Some(Value(2))),
            }],
            value_types: vec![operand, operand, result],
        };
        emit(&IrModule { funcs: vec![func] }).unwrap()
    }

    #[test]
    fn qbe_width_u8_is_w_expected() {
        assert_eq!(width(int(8, false)), "w");
        assert_eq!(width(int(16, true)), "w");
        assert_eq!(width(int(32, false)), "w");
    }

    #[test]
    fn qbe_width_i64_is_l_expected() {
        assert_eq!(width(int(64, true)), "l");
        assert_eq!(width(int(64, false)), "l");
    }

    #[test]
    fn qbe_width_float_is_s_and_d_expected() {
        assert_eq!(width(IrType::Float { bits: 32 }), "s");
        assert_eq!(width(IrType::Float { bits: 64 }), "d");
    }

    #[test]
    fn emit_float_literal_uses_d_prefix() {
        let il = emit_src(": w ( -- f64 ) 3.14 ;");
        assert!(il.contains("=d copy d_3.14"), "unexpected IL: {il}");
    }

    #[test]
    fn emit_float_add_runs_at_d_width() {
        let f64_ty = IrType::Float { bits: 64 };
        let il = emit_binary(
            f64_ty,
            f64_ty,
            Instr::Bin(Value(2), BinOp::Add, Value(0), Value(1)),
        );
        assert!(il.contains("=d add"), "expected a d-width add: {il}");
        assert!(!il.contains("and"), "floats never canonicalize: {il}");
    }

    #[test]
    fn emit_float_div_emits_div() {
        let f32_ty = IrType::Float { bits: 32 };
        let il = emit_binary(
            f32_ty,
            f32_ty,
            Instr::Bin(Value(2), BinOp::Div, Value(0), Value(1)),
        );
        assert!(il.contains("=s div"), "expected an s-width div: {il}");
    }

    #[test]
    fn emit_float_compare_uses_ordered_mnemonic() {
        // `<` on `f64` operands lowers to `cltd` (ordered, false against NaN),
        // producing a `w`-width `bool` (R17, RISK 1).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Lt, Value(0), Value(1)),
        );
        assert!(il.contains("=w cltd"), "expected an ordered compare: {il}");
    }

    #[test]
    fn emit_float_eq_is_ordered_ceq() {
        // `=` on floats is the ordered `ceqd`, so a NaN compares false to
        // itself and `x = x` is a valid NaN test (R17/D3).
        let il = emit_binary(
            IrType::Float { bits: 64 },
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Eq, Value(0), Value(1)),
        );
        assert!(il.contains("ceqd"), "expected an ordered eq: {il}");
    }

    #[test]
    fn emit_cmp_signed_uses_cslt() {
        let il = emit_binary(
            int(32, true),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Lt, Value(0), Value(1)),
        );
        assert!(il.contains("csltw"), "expected a signed compare: {il}");
    }

    #[test]
    fn emit_cmp_unsigned_uses_cult() {
        let il = emit_binary(
            int(32, false),
            IrType::Bool,
            Instr::Cmp(Value(2), CmpOp::Lt, Value(0), Value(1)),
        );
        assert!(il.contains("cultw"), "expected an unsigned compare: {il}");
    }

    #[test]
    fn emit_unsigned_mod_uses_urem() {
        let u32_ty = int(32, false);
        let il = emit_binary(
            u32_ty,
            u32_ty,
            Instr::Bin(Value(2), BinOp::Rem, Value(0), Value(1)),
        );
        assert!(il.contains("urem"), "expected an unsigned rem: {il}");
    }

    #[test]
    fn emit_signed_mod_uses_rem() {
        let i32_ty = int(32, true);
        let il = emit_binary(
            i32_ty,
            i32_ty,
            Instr::Bin(Value(2), BinOp::Rem, Value(0), Value(1)),
        );
        assert!(il.contains(" rem "), "expected a signed rem: {il}");
        assert!(!il.contains("urem"), "unexpected urem: {il}");
    }

    /// Emit a single-block function `src (v0) -> Conv -> v1`, returning the IL.
    /// Hand-built types isolate the bare conversion codegen path per cell,
    /// rather than needing a matching Sooth program for every width/sign pair.
    fn emit_conv_il(src_ty: IrType, dst_ty: IrType) -> String {
        let src_const = if matches!(src_ty, IrType::Float { .. }) {
            Instr::ConstF(Value(0), 5.0)
        } else {
            Instr::Const(Value(0), 5)
        };
        let func = IrFunc {
            name: "t".to_string(),
            params: vec![],
            ret: Some(dst_ty),
            blocks: vec![crate::ir::Block {
                id: crate::ir::BlockId(0),
                instrs: vec![src_const, Instr::Conv(Value(1), Value(0))],
                term: Terminator::Ret(Some(Value(1))),
            }],
            value_types: vec![src_ty, dst_ty],
        };
        emit(&IrModule { funcs: vec![func] }).unwrap()
    }

    fn f32() -> IrType {
        IrType::Float { bits: 32 }
    }

    fn f64() -> IrType {
        IrType::Float { bits: 64 }
    }

    #[test]
    fn emit_conv_signed_int_to_float_uses_swtof_sltof() {
        // i32 -> f64 reads the `w` source as signed; i64 -> f32 reads `l`.
        assert!(
            emit_conv_il(int(32, true), f64()).contains("=d swtof"),
            "expected swtof to double"
        );
        assert!(
            emit_conv_il(int(64, true), f32()).contains("=s sltof"),
            "expected sltof to single"
        );
    }

    #[test]
    fn emit_conv_unsigned_int_to_float_uses_uwtof_ultof() {
        // A sub-word unsigned source rides its canonical `w` carrier (uwtof).
        assert!(
            emit_conv_il(int(8, false), f64()).contains("=d uwtof"),
            "expected uwtof to double"
        );
        assert!(
            emit_conv_il(int(64, false), f32()).contains("=s ultof"),
            "expected ultof to single"
        );
    }

    #[test]
    fn emit_conv_float_widen_is_exts() {
        // f32 >f64 is the exact single->double extend.
        let il = emit_conv_il(f32(), f64());
        assert!(il.contains("=d exts"), "expected an exts: {il}");
    }

    #[test]
    fn emit_conv_float_narrow_is_truncd() {
        // f64 >f32 rounds to nearest via truncd.
        let il = emit_conv_il(f64(), f32());
        assert!(il.contains("=s truncd"), "expected a truncd: {il}");
    }

    #[test]
    fn emit_conv_float_to_int_truncates_toward_zero() {
        // f64 >i64 truncates toward zero (dtosi to the `l` carrier); f32 >i32
        // uses stosi to the `w` carrier.
        assert!(
            emit_conv_il(f64(), int(64, true)).contains("=l dtosi"),
            "expected dtosi to long"
        );
        assert!(
            emit_conv_il(f32(), int(32, true)).contains("=w stosi"),
            "expected stosi to word"
        );
    }

    #[test]
    fn emit_conv_float_to_unsigned_int_uses_toui() {
        // An unsigned int target selects the `*toui` mnemonic.
        let il = emit_conv_il(f64(), int(64, false));
        assert!(il.contains("=l dtoui"), "expected dtoui: {il}");
    }

    #[test]
    fn emit_conv_float_to_subword_int_canonicalizes() {
        // f64 >u8 truncates to the `w` carrier then masks to the low byte (R15).
        let il = emit_conv_il(f64(), int(8, false));
        assert!(
            il.contains("dtoui") || il.contains("dtosi"),
            "expected a float->int trunc: {il}"
        );
        assert!(
            il.contains("and") && il.contains("255"),
            "expected a u8 mask after the trunc: {il}"
        );
    }

    #[test]
    fn emit_conv_narrow_truncates_and_canonicalizes() {
        // i64 -> u8 keeps the low byte via the unsigned canonicalization mask.
        let il = emit_conv_il(int(64, true), int(8, false));
        assert!(
            il.contains("and") && il.contains("255"),
            "expected a low-byte mask: {il}"
        );
    }

    #[test]
    fn emit_conv_signed_widen_sign_extends() {
        // i16 -> i64 sign-extends from the source width.
        let il = emit_conv_il(int(16, true), int(64, true));
        assert!(il.contains("=l extsh"), "expected a sign-extend: {il}");
    }

    #[test]
    fn emit_conv_unsigned_widen_zero_extends() {
        // u8 -> u32 zero-extends by the (unsigned) source signedness.
        let il = emit_conv_il(int(8, false), int(32, false));
        assert!(il.contains("=w extub"), "expected a zero-extend: {il}");
    }

    #[test]
    fn emit_conv_signed_widen_to_unsigned_subword_canonicalizes() {
        // i8 -> u16: extsb sign-extends into bits the target (u16) requires to
        // be zero, so the widen must be re-canonicalized to an unsigned mask
        // rather than trusted as-is (this is the dirty-high-bits cell).
        let il = emit_conv_il(int(8, true), int(16, false));
        assert!(
            il.contains("extsb"),
            "expected the source-signed extend: {il}"
        );
        assert!(
            il.contains("and") && il.contains("65535"),
            "expected a u16 mask after the extend: {il}"
        );
    }

    #[test]
    fn emit_conv_same_width_is_relabel() {
        // i32 >u32 fills its register either way: a pure bit relabel (`copy`).
        let il = emit_conv_il(int(32, true), int(32, false));
        assert!(il.contains("=w copy"), "expected a copy relabel: {il}");
        assert!(
            !il.contains("ext"),
            "a same-width relabel extends nothing: {il}"
        );
    }

    #[test]
    fn emit_subword_arith_canonicalizes() {
        // An unsigned sub-word add masks its result to the low `bits`.
        let u8 = int(8, false);
        let il = emit_binary(u8, u8, Instr::Bin(Value(2), BinOp::Add, Value(0), Value(1)));
        assert!(il.contains("add"), "expected the add: {il}");
        assert!(
            il.contains("and") && il.contains("255"),
            "expected a mask: {il}"
        );

        // A signed sub-word add sign-extends its result from `bits`.
        let i8 = int(8, true);
        let il = emit_binary(i8, i8, Instr::Bin(Value(2), BinOp::Add, Value(0), Value(1)));
        assert!(il.contains("extsb"), "expected a sign-extend: {il}");
    }
}
