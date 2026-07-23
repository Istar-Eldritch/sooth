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

/// The QBE base-type letter for an `IrType`, derived here (not in the IR, R14):
/// `Bool` is a 4-byte `w` (0/1); an integer is `w` for `bits <= 32` and `l` for
/// `64`; `Ptr` is the 8-byte `l` used by the buffer and C ABI.
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
        Instr::Bin(v, op, a, b) => {
            // The op runs at the result's register width; a sub-word result can
            // overflow its width, so canonicalize it (R15) via the shared point.
            let ty = ty_of(value_types, *v);
            let w = width(ty);
            let m = match op {
                BinOp::Add => "add",
                BinOp::Sub => "sub",
                BinOp::Mul => "mul",
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
            let signed = matches!(operand, IrType::Int { signed: true, .. });
            let m = match op {
                CmpOp::Eq => "ceq",
                CmpOp::Lt if signed => "cslt",
                CmpOp::Lt => "cult",
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
            // The 8-byte buffer slot is always an `l` sink (R4); a `Bool` (`w`)
            // value is zero-extended before it lands there (RK1).
            if ty_of(value_types, *v) == IrType::Bool {
                let ext = format!("%ext{ext_id}");
                *ext_id += 1;
                writeln!(out, "\t{ext} =l extuw {}", val(*v)).unwrap();
                writeln!(out, "\tstorel {}, {}", ext, val(*ptr))
            } else {
                writeln!(out, "\tstorel {}, {}", val(*v), val(*ptr))
            }
        }
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
        let (func, _m) = lower_line(0, &terms, entry_depth, &env, &resolve);
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
    /// returning `v2` (the result of a binary/compare op). Lets Phase 3 exercise
    /// sub-word/unsigned codegen with no source path to produce those types yet.
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
