//! Phase 4 Slice 12, phase 3 exit criteria (part D).
//!
//! Retiring the inference leg (part A) made an ordinary `[ ... ]` *parameter* a
//! real call for the first time. These are the witnesses that the real-call
//! path actually lowers: the argument reaches `Instr::Call` as the materialized
//! `(code, env)` aggregate, not the phantom `I64` marker a spliced combinator
//! leaves behind (X10/M-D), through two call levels (X11).
//!
//! X12, the REPL's own boundary declining this shape, retired with the REPL.

use sooth::ir::{lower, Instr, IrType};
use sooth::test_support;
use sooth::{check, lexer};

mod common;

/// Recon 5's shape: an ordinary `[ ... ]` parameter, called through, with no
/// `inline`.
const APPLY: &str = ": apply ( [ i64 -- i64 ] i64 -- i64 ) | n | | f | n f call ;\n";

fn build_and_run(name: &str, src: &str) -> (std::path::PathBuf, String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    (
        binary,
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// X10 / M-D, the discriminating half: `main`'s call to `apply` is a real
/// `Instr::Call` whose quotation argument is a materialized `(code, env)`
/// value, asserted on the lowered IR. Skipping the R-D3 materialization leaves
/// the phantom's `I64` placeholder in the argument list, which this reads
/// directly -- "it builds" or "exit 0" does not: QBE rejects the phantom only
/// because this callee's parameter is spelled as an aggregate, and a callee
/// whose parameter classified the same way as the placeholder would link and
/// run wrong instead of failing.
#[test]
fn apply_call_argument_is_a_materialized_quotation() {
    let src = format!("{APPLY}: main ( -- ) [ 1 add ] 5 apply . ;\n");
    let tokens = lexer::lex(&common::silent_prints(&src)).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");

    assert!(
        ir.funcs.iter().any(|f| f.name == "apply"),
        "an ordinary `[ ... ]`-parameter word mints its own `IrFunc`: {:?}",
        ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    let main = ir
        .funcs
        .iter()
        .find(|f| f.name == "main")
        .expect("`main` is lowered");
    let args = main
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .find_map(|i| match i {
            Instr::Call(_, sym, args, _) if sym == "apply" => Some(args),
            _ => None,
        })
        .expect("`main` calls `apply` for real: the splice is retired");
    assert_eq!(args.len(), 2, "the callee's two declared inputs");
    assert!(
        matches!(main.value_types[args[0].0 as usize], IrType::Quotation(_)),
        "the quotation argument must reach the call as a materialized `(code, env)` \
         aggregate, not the phantom placeholder: {:?}",
        main.value_types[args[0].0 as usize]
    );
}

/// X10, the end-to-end half: the same program runs and prints `6`, and `apply`
/// is a real symbol in the binary (the counterpart to slice 11's
/// `inline_word_mints_no_symbol`: a real call has the symbol a splice lacks).
#[test]
fn apply_witness_runs_and_mints_a_symbol() {
    let src = format!("{APPLY}: main ( -- ) [ 1 add ] 5 apply . ;\n");
    let (binary, stdout, code) = build_and_run("slice12-partd-apply", &src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);

    let nm = std::process::Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_file(&binary).ok();
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
        symbols.contains("apply"),
        "a real-call word mints a symbol; nm found:\n{symbols}"
    );
}

/// X11: the quotation survives two real-call levels (the forwarding callee
/// passes on an already-materialized parameter, not a phantom), and a word that
/// calls a quotation and returns its result works on its own.
#[test]
fn quotation_through_two_call_levels_and_a_returning_callee_run() {
    let src = format!(
        "{APPLY}\
         : apply2 ( [ i64 -- i64 ] i64 -- i64 ) apply ;\n\
         : run ( [ -- i64 ] -- i64 ) call ;\n\
         : main ( -- ) [ 1 add ] 5 apply2 . [ 42 ] run . ;\n"
    );
    let (binary, stdout, code) = build_and_run("slice12-partd-levels", &src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "6\n42\n");
    assert_eq!(code, 0);
}

/// R-D3 at the *back edge*: a self tail call is not the ordinary dispatch, it
/// pushes its arguments onto `back_edges` and `finalize_loop` blits each
/// aggregate one into the header's stable slot. A quotation parameter carried
/// around the loop must therefore be materialized there too -- otherwise the
/// phantom's `I64` placeholder is the blit source and QBE rejects the function
/// (`invalid type for first operand in blit0`). `go` rebinds `f` to a fresh
/// quotation each iteration, so the back-edge argument is a live phantom, not
/// the already-materialized parameter forwarded unchanged.
#[test]
fn tail_recursive_quotation_argument_is_materialized_at_the_back_edge() {
    let src = ": go ( [ i64 -- i64 ] i64 -- i64 )\n\
                 | n | | f |\n\
                 n 0 eq ~[ 7 f call ] ~[ f drop [ 2 mul ] n 1 sub go ] if ;\n\
               : main ( -- ) [ 3 mul ] 2 go . ;\n";

    let tokens = lexer::lex(&common::silent_prints(src)).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");
    let go = ir
        .funcs
        .iter()
        .find(|f| f.name == "go")
        .expect("`go` is lowered");
    let bad: Vec<_> = go
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::Blit(src, dst, _)
                if matches!(go.value_types[dst.0 as usize], IrType::Quotation(_))
                    && !matches!(go.value_types[src.0 as usize], IrType::Quotation(_)) =>
            {
                Some(go.value_types[src.0 as usize])
            }
            _ => None,
        })
        .collect();
    assert!(
        bad.is_empty(),
        "every blit into the carried quotation slot must read a materialized \
         `(code, env)`, not the phantom's placeholder: {bad:?}"
    );

    let (binary, stdout, code) = build_and_run("slice12-partd-tailquot", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
}
