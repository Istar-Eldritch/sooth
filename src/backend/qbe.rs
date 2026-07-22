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

/// The QBE base-type letter for an `IrType`: `Bool` is a 4-byte `w` (0/1);
/// `Int`/`Ptr` are the 8-byte `l` used by the buffer and C ABI.
fn width(ty: IrType) -> &'static str {
    match ty {
        IrType::Bool => "w",
        IrType::Int | IrType::Ptr => "l",
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
            let m = match op {
                BinOp::Add => "add",
                BinOp::Sub => "sub",
                BinOp::Mul => "mul",
                BinOp::Rem => "rem",
            };
            writeln!(out, "\t{} =l {m} {}, {}", val(*v), val(*a), val(*b))
        }
        Instr::Cmp(v, op, a, b) => {
            let m = match op {
                CmpOp::Eq => "ceql",
                CmpOp::Lt => "csltl",
                CmpOp::Gt => "csgtl",
            };
            // The comparison result is always `Bool`-tagged (`w`); the operand
            // width encoded in the mnemonic (`l`) stays fixed since `= < >`
            // only ever compare `i64` operands (D2/D6).
            let w = width(ty_of(value_types, *v));
            writeln!(out, "\t{} ={w} {m} {}, {}", val(*v), val(*a), val(*b))
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
}
