//! Phase 4 Slice 11, phase 1 goldens: `inline` as a *declared* word property.
//! A word marked `inline` is spliced at every call site whatever its
//! parameters, so it mints no `IrFunc`, no symbol, and no `Instr::Call` -- and
//! where splicing is impossible the definition is a located error (D2), never a
//! silent fall-back to a real call. `ClkDiv` is the motivating shape: a
//! constant-producing word an embedded reader must be able to see costs no
//! call, without trusting an optimiser to recognise it.

use std::io::BufReader;

use sooth::ir::{lower, Instr};
use sooth::{check, lexer, parser};

const CLKDIV: &str = ": ClkDiv inline ( -- u32 u32 ) 8 >u32 4 >u32 ;\n";

/// Compile and run `src`, returning the built binary's path, its stdout, and
/// its exit code. The binary is left in place so a caller can inspect its
/// symbol table; `name` distinguishes the temp source per test (the goldens run
/// in parallel).
fn build_and_run(name: &str, src: &str) -> (std::path::PathBuf, String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
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

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// Run a scripted session in-process and return the whole transcript, mirroring
/// `tests/phase4_combinators.rs`'s 6c REPL goldens (a `.` prints to the real
/// process stdout, which this buffer does not see, so a value witness must be
/// left on the residual stack and the exact `stack:` line asserted).
fn repl_transcript(input: &str) -> String {
    let reader = BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sooth::repl::run(reader, &mut out).expect("the REPL loop itself should not error");
    String::from_utf8(out).expect("REPL output should be utf8")
}

#[test]
fn inline_word_mints_no_symbol() {
    // The exit criterion: `ClkDiv` takes no quotation, so before this slice it
    // was an ordinary word with an `IrFunc` and a symbol. It runs (4 then 8: `.`
    // prints the top first) and its name appears nowhere in the binary's symbol
    // table -- the same property `quotation_taking_word_mints_no_symbol`
    // (`src/check/combinators.rs`) asserts at the predicate, here end to end.
    let src = format!("{CLKDIV}: main ( -- ) ClkDiv . . ;\n");
    let (binary, stdout, code) = build_and_run("slice11-no-symbol", &src);
    assert_eq!(stdout, "4\n8\n");
    assert_eq!(code, 0);

    let nm = std::process::Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_file(&binary).ok();
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
        !symbols.contains("ClkDiv"),
        "an `inline` word mints no symbol; nm found:\n{symbols}"
    );
    assert!(
        symbols.contains("main"),
        "sanity: nm reads this binary's symbols at all:\n{symbols}"
    );
}

#[test]
fn inline_word_caller_emits_no_call() {
    // The second exit criterion, asserted on the lowered IR rather than
    // inferred from the output: the caller has no `Instr::Call` at all, so the
    // splice happened in the checker and lowering minted nothing to call. The
    // `>u32` conversions are pure ops, not calls.
    let src = format!("{CLKDIV}: main ( -- ) ClkDiv drop drop ;\n");
    let tokens = lexer::lex(&src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");
    assert!(
        !ir.funcs.iter().any(|f| f.name.contains("ClkDiv")),
        "an `inline` word mints no `IrFunc`: {:?}",
        ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    let main = ir
        .funcs
        .iter()
        .find(|f| f.name == "main")
        .expect("`main` is lowered");
    let calls: Vec<&Instr> = main
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Call(..)))
        .collect();
    assert!(
        calls.is_empty(),
        "the caller of an `inline` word emits no call: {calls:?}"
    );
}

#[test]
fn inline_word_clause_body_is_located_error() {
    // R3: a clause body cannot be spliced (`is_combinator` requires
    // `WordBody::Terms`), so the definition is rejected rather than quietly
    // lowered as an ordinary clause word.
    let err = check_error(
        "type: E | A | B ;\n\
         : pick inline ( E -- i64 )\n\
         | A  1\n\
         | B  2\n\
         ;\n\
         : main ( -- ) A pick . ;\n",
    );
    assert_eq!(
        err,
        "error: `inline` on `pick`, which has a clause body; `inline` requires a term body (line 2, col 3)"
    );
}

#[test]
fn inline_word_polymorphic_signature_is_located_error() {
    // R3: a variable-bearing signature is rejected before the poly checker runs
    // (which would otherwise take it for a legitimate poly combinator). Phrased
    // over declared variables, so the `~` case in
    // `inline_tilde_parameter_word_is_accepted` stays accepted.
    let err = check_error(": id inline ( 'T -- 'T ) ;\n: main ( -- ) 3 id . ;\n");
    assert_eq!(
        err,
        "error: `inline` on `id`, which declares a polymorphic signature; `inline` requires a monomorphic effect (line 1, col 3)"
    );
}

#[test]
fn inline_word_self_nontail_cycle_is_located_error() {
    // R4: an `inline` word inherits the cycle rejection under its reworded
    // umbrella term -- it need not take a quotation, so "a quotation-taking
    // word" no longer names the class the rule covers.
    let err =
        check_error(": loopy inline ( i64 -- i64 ) 1 + loopy 2 * ;\n: main ( -- ) 3 loopy . ;\n");
    assert_eq!(
        err,
        "error: an always-spliced word cannot be recursive (the inliner would splice it forever): `loopy` -> `loopy` (line 1, col 3)"
    );
}

#[test]
fn inline_on_main_is_located_error() {
    // The entry point is called by the runtime shim, so splicing it away leaves
    // that call unresolved: without this rejection the program dies as a raw
    // `ld: undefined reference to `sooth_main'`, not a located Sooth error.
    // `audit_word_quotation_positions` already keeps `main` off the *quotation*
    // route into `is_combinator` ("an input of `main`", D6/R28); the declared
    // flag is a second route to the same shape.
    let err = check_error(": main inline ( -- ) 1 . ;\n");
    assert_eq!(
        err,
        "error: `inline` on `main`, which is the program entry point; the entry point is called by the runtime shim and cannot be spliced (line 1, col 3)"
    );
}

#[test]
fn inline_tilde_parameter_word_is_accepted_and_spliced() {
    // The discriminating positive for R3's monomorphism rule: a `~`-bearing
    // effect is poly-forced by the parser (`effect_has_variable`) but declares
    // no variable, so it is monomorphic for `inline`'s purposes and runs.
    let src = ": twice inline ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;\n\
               : main ( -- ) 3 [ 1 + ] twice . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-tilde", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

#[test]
fn inline_word_self_tail_recursion_runs_as_a_loop() {
    // R4's relaxation, inherited: every self-occurrence in tail position is not
    // a splice-forever cycle, because the loop transform lowers it to a
    // back-edge. `5 down` counts to 0.
    let src = ": down inline ( i64 -- i64 ) dup 0 > if 1 - down else end ;\n\
               : main ( -- ) 5 down . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-self-tail", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "0\n");
    assert_eq!(code, 0);
}

#[test]
fn repl_inline_word_is_retained_not_lowered() {
    // R7: the REPL's retention gate was `word_declares_quotation_parameter`, so
    // an `inline` word taking no quotation fell through to the ordinary
    // lowering path and minted a `.so` and a symbol -- D2's forbidden
    // fall-back, inside the REPL.
    //
    // The witness is freshness, the same discrimination
    // `repl_combinator_splice_sees_current_helper` vs
    // `repl_ordinary_caller_frozen_across_combinator_redefinition`
    // (`tests/phase4_combinators.rs`) draws: a retained word is re-spliced at
    // each later line and sees the *current* `helper` (105 then 205), while a
    // lowered one is frozen into its `.so` and would leave `105 105`.
    let transcript = repl_transcript(
        ": helper ( i64 -- i64 ) 100 + ;\n\
         : bump inline ( i64 -- i64 ) helper ;\n\
         5 bump\n\
         : helper ( i64 -- i64 ) 200 + ;\n\
         5 bump\n:quit\n",
    );
    assert_eq!(
        transcript,
        "defined helper\ndefined bump\nstack: 105\ndefined helper\nstack: 105 205\n"
    );
}

#[test]
fn repl_inline_polymorphic_signature_is_rejected() {
    // R3 at the REPL: the retention route (R7) would otherwise carry a
    // variable-bearing `inline` word into the poly-combinator check as a
    // legitimate poly combinator, so the session runs the same per-word
    // rejection native `check` runs, with the same message.
    let transcript = repl_transcript(": id inline ( 'T -- 'T ) ;\n:quit\n");
    assert_eq!(
        transcript,
        "error: `inline` on `id`, which declares a polymorphic signature; `inline` requires a monomorphic effect (line 1, col 3)\n"
    );
}
