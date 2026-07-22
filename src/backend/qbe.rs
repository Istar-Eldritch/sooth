//! QBE backend: emit QBE IL text from the neutral IR.
//!
//! Driver then pipes this through `qbe` (-> assembly) and `cc` (-> native binary).
//! QBE gives arm64/x86_64/riscv64 and C-ABI struct classification for free; costs
//! accepted are i128 synthesised in the frontend and atomics via C11 FFI.

use std::fmt::Write;

use crate::ir::{BinOp, BlockId, CmpOp, Instr, IrFunc, IrModule, Terminator, Value};

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

fn emit_func(out: &mut String, func: &IrFunc) {
    let ret_ty = if func.ret.is_some() { "l " } else { "" };
    let params: Vec<String> = (0..func.params.len()).map(|i| format!("l %v{i}")).collect();
    writeln!(
        out,
        "export function {ret_ty}${}({}) {{",
        qbe_name(&func.name),
        params.join(", ")
    )
    .unwrap();
    for block in &func.blocks {
        writeln!(out, "{}", label(block.id)).unwrap();
        for instr in &block.instrs {
            emit_instr(out, instr);
        }
        emit_term(out, &block.term);
    }
    out.push_str("}\n");
}

fn emit_instr(out: &mut String, instr: &Instr) {
    match instr {
        Instr::Const(v, n) => writeln!(out, "\t{} =l copy {n}", val(*v)),
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
            writeln!(out, "\t{} =l {m} {}, {}", val(*v), val(*a), val(*b))
        }
        Instr::Call(ret, f, args) => {
            let a: Vec<String> = args.iter().map(|x| format!("l {}", val(*x))).collect();
            match ret {
                Some(r) => writeln!(
                    out,
                    "\t{} =l call ${}({})",
                    val(*r),
                    qbe_name(f),
                    a.join(", ")
                ),
                None => writeln!(out, "\tcall ${}({})", qbe_name(f), a.join(", ")),
            }
        }
        Instr::Print(v) => writeln!(out, "\tcall $printf(l $fmt, l {}, ...)", val(*v)),
        Instr::PtrOffset(dst, base, bytes) => {
            writeln!(out, "\t{} =l add {}, {bytes}", val(*dst), val(*base))
        }
        Instr::Load(dst, ptr) => writeln!(out, "\t{} =l loadl {}", val(*dst), val(*ptr)),
        Instr::Store(ptr, v) => writeln!(out, "\tstorel {}, {}", val(*v), val(*ptr)),
        Instr::Phi(r, arms) => {
            let a: Vec<String> = arms
                .iter()
                .map(|(b, v)| format!("{} {}", label(*b), val(*v)))
                .collect();
            writeln!(out, "\t{} =l phi {}", val(*r), a.join(", "))
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
        let il = emit_src(": w ( i64 -- i64 ) if 1 else 2 then ;");
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
