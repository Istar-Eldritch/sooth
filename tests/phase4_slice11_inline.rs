//! Phase 4 Slice 11 goldens: `inline` as a *declared* word property.
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

/// Build and run a two-file closure, returning the built binary's path, its
/// stdout and its exit code. The binary is built inside the closure dir, so the
/// caller removes that whole dir once it has read the symbol table.
fn build_and_run_closure(name: &str, lib: &str, main: &str) -> (std::path::PathBuf, String, i32) {
    let dir = std::env::temp_dir().join(format!("sooth-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the closure dir should succeed");
    std::fs::write(dir.join("lib.sth"), lib).expect("writing the library should succeed");
    let entry = dir.join("main.sth");
    std::fs::write(&entry, main).expect("writing the entry should succeed");
    let binary = sooth::driver::build(&entry).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
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
fn inline_word_imported_across_modules_is_spliced() {
    // R2 claims the widened predicate reaches call-site splicing with no
    // further plumbing; an imported call site is the route the single-file
    // goldens above cannot reach, and the only one where the splice competes
    // with name mangling. The non-inline sibling is the discriminator: it keeps
    // a symbol out of the very module the `inline` word mints none in, so the
    // absence is the keyword's doing and not a whole-module omission.
    let (binary, stdout, code) = build_and_run_closure(
        "slice11-cross-module",
        ": Fast inline ( -- i64 ) 7 ;\n: slow ( -- i64 ) 5 ;\nexport: Fast slow ;\n",
        "import: \"lib.sth\" lib ;\n: main ( -- ) lib::Fast . lib::slow . ;\n",
    );
    assert_eq!(stdout, "7\n5\n");
    assert_eq!(code, 0);

    let nm = std::process::Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_dir_all(binary.parent().expect("the binary sits in the closure dir")).ok();
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
        !symbols.contains("Fast"),
        "an imported `inline` word mints no symbol; nm found:\n{symbols}"
    );
    assert!(
        symbols.contains("slow"),
        "its non-inline sibling in the same module still does:\n{symbols}"
    );
}

#[test]
fn inline_word_calling_inline_word_splices_transitively() {
    // The inliner walks its own output, so an `inline` word whose body calls
    // another one leaves no residue at either level -- neither a surviving
    // `IrFunc` for the inner word nor a call in the outer one's splice.
    let src = ": inner inline ( i64 -- i64 ) 2 mul ;\n\
               : outer inline ( i64 -- i64 ) inner 2 mul ;\n\
               : main ( -- ) 3 outer . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-transitive", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "12\n");
    assert_eq!(code, 0);

    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");
    let names: Vec<&String> = ir.funcs.iter().map(|f| &f.name).collect();
    assert!(
        !names
            .iter()
            .any(|n| n.contains("inner") || n.contains("outer")),
        "neither level of an `inline` chain mints an `IrFunc`: {names:?}"
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
        "the inner splice leaves no call behind either: {calls:?}"
    );
}

#[test]
fn inline_word_polymorphic_signature_is_accepted() {
    // Slice 10c (R-P3-3b) **reverses R3's polymorphic half**, which its own
    // doc admitted was a policy rule and not a soundness one: the splice
    // already handles a variable-bearing body. The reversal is what lets the
    // six `lib/core.sth` comparison words be both `'T: Copy Ord`-polymorphic
    // and `inline`. R3's other rejections (`main`, a builtin operator name)
    // are untouched, and the tests above still pin them.
    let tokens = lexer::lex(": id inline ( 'T -- 'T ) ;\n: main ( -- ) 3 id . ;\n")
        .expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("a polymorphic `inline` word is spliced, not rejected");
}

#[test]
fn inline_word_self_nontail_cycle_is_located_error() {
    // R4: an `inline` word inherits the cycle rejection under its reworded
    // umbrella term -- it need not take a quotation, so "a quotation-taking
    // word" no longer names the class the rule covers.
    let err = check_error(
        ": loopy inline ( i64 -- i64 ) 1 add loopy 2 mul ;\n: main ( -- ) 3 loopy . ;\n",
    );
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
fn inline_on_builtin_operator_overload_is_located_error() {
    // A builtin-operator name is claimed by `check_operator` first, which
    // records the site for a real `Instr::Call`; the call then *also* falls
    // through to the combinator interception and is spliced, and lowering
    // trusts the stale record and looks the symbol up in an `env` a combinator
    // is excluded from. Before the rejection this panicked in
    // `ir/func_builder/calls.rs` ("checked user overload exists").
    let src = "type: A n i64 ;\n\
               : add inline ( A A -- i64 ) | x y | &x &n @ drop &y &n @ drop 1000 ;\n\
               : main ( -- ) 1 A 2 A add . ;\n";
    let err = check_error(src);
    assert_eq!(
        err,
        "error: `inline` on `add`, which overloads a builtin operator name; a call site of a builtin operator name dispatches through a real call and cannot be spliced (line 2, col 3)"
    );
    // The same overload without `inline` builds and runs, so the rejection is
    // the keyword's, not the overload's.
    let (binary, stdout, code) =
        build_and_run("slice11-op-overload", &src.replace("add inline", "add"));
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "1000\n");
    assert_eq!(code, 0);
}

#[test]
fn inline_tilde_parameter_word_is_accepted_and_spliced() {
    // The discriminating positive for R3's monomorphism rule: a `~`-bearing
    // effect is poly-forced by the parser (`effect_has_variable`) but declares
    // no variable, so it is monomorphic for `inline`'s purposes and runs.
    let src = ": twice inline ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;\n\
               : main ( -- ) 3 ~[ 1 add ] twice . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-tilde", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);

    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");
    assert!(
        !ir.funcs.iter().any(|f| f.name.contains("twice")),
        "an `inline` word with a `~`-bearing effect mints no `IrFunc`: {:?}",
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
        "`twice` is spliced, not called: {calls:?}"
    );
}

#[test]
fn inline_word_self_tail_recursion_runs_as_a_loop() {
    // R4's relaxation, inherited: every self-occurrence in tail position is not
    // a splice-forever cycle, because the loop transform lowers it to a
    // back-edge. `5 down` counts to 0.
    let src = ": down inline ( i64 -- i64 ) dup 0 gt ~[ 1 sub down ] ~[ ] if ;\n\
               : main ( -- ) 5 down . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-self-tail", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "0\n");
    assert_eq!(code, 0);

    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = lower(&module).expect("lowering should succeed");
    assert!(
        !ir.funcs.iter().any(|f| f.name.contains("down")),
        "the self-tail `inline` word mints no `IrFunc`: {:?}",
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
        "the self-tail recursion lowers to a back-edge loop, not a call: {calls:?}"
    );
}

#[test]
fn inline_reference_output_pair() {
    // R5/Feature C: `check_reference_free_signature` rejects a `&T`/`&!T` output
    // because it "borrows a local of the callee's own frame, which is gone by
    // the time the caller reads it". A spliced word has no such frame:
    // `alpha_rename_locals` makes its locals caller locals. So the rule is
    // skipped for a combinator and the pair is the witness that the relaxation
    // is scoped to the splice -- the same word, the same body, differing only in
    // the keyword, is accepted and then rejected.
    //
    // The run proves the returned reference really points at the caller's local
    // rather than at dead frame storage: `+!` through it, and the caller reads
    // the new value back out of its own struct (7 then 12).
    let src = "type: P n u32 ;\n\
               : pick inline ( &!P -- &!u32 ) | p | p &!n ;\n\
               : main ( -- )\n\
                 7 >u32 P | s |\n\
                 &!s pick | r |\n\
                 r @ >i64 .\n\
                 r 5 >u32 +!\n\
                 &s &n @ >i64 .\n\
                 s drop ;\n";
    let (binary, stdout, code) = build_and_run("slice11-ref-output", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "7\n12\n");
    assert_eq!(code, 0);

    let err = check_error(&src.replace("pick inline", "pick"));
    assert_eq!(
        err,
        "error: a reference cannot be stored: `pick` declares the output `&!u32`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
    );
}

#[test]
fn quotation_taking_word_reference_output_is_accepted() {
    // The exemption is phrased over the shared `is_combinator` predicate (D5),
    // not over the new flag, so it covers every always-spliced word uniformly.
    // Slice 12 (R-B1): a `~[ ... ]` parameter now requires `inline` at the
    // definition, so this word declares it too, but the assertion this test
    // guards is unchanged -- the exemption reads `is_combinator`, not the flag
    // directly. It is also the recon-5 shape, whose reference is derived from
    // an *input* reference and so is rooted in the caller either way.
    let src = "type: P n u32 ;\n\
               : pick inline ( &!P ~[ -- ] -- &!u32 ) | p f | f call p &!n ;\n\
               : main ( -- )\n\
                 7 >u32 P | s |\n\
                 &!s ~[ 1 . ] pick | r |\n\
                 r 2 >u32 +!\n\
                 &s &n @ >i64 .\n\
                 s drop ;\n";
    let (binary, stdout, code) = build_and_run("slice11-ref-output-quot", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "1\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn inline_reference_to_linear_local_is_rejected() {
    // The one adversarial shape R5 leaves standing (Feature C): the returned
    // reference borrows a *linear* local the callee itself declared. Post-splice
    // the caller would inherit that local's drop obligation and the shape would
    // be safe, but the standalone def-site check sees the local borrowed and
    // never consumed and rejects it by the pre-existing must-consume rule --
    // reject-safe, and reached with no special case for references. Deleting the
    // borrow-and-return is not what makes this pass; consuming `b` is.
    let src = "type: Buf  data ^[u8 64]  len usize ;\n\
               : fresh inline ( -- &!usize )\n\
                 0 >u8 64 fill ^ 0 >usize Buf | b |\n\
                 &!b &!len ;\n";
    let err = check_error(src);
    assert_eq!(
        err,
        "error: linear value `b` is never consumed in `fresh` (line 4)\n  `b` has type `Buf`, which is linear: drop it or return it (nothing is dropped for you)\n  note: declared ( -- &!usize )"
    );
}

#[test]
fn inline_reference_to_nonlinear_callee_local_is_accepted() {
    // The adversarial shape `inline_reference_output_pair` does *not* cover:
    // there the reference is derived from an *input* reference, so it is
    // caller-rooted whether or not the word is spliced. Here the referent is a
    // struct the callee itself declares and pushes fresh, non-linear (`u32`
    // fields), so this is the shape that actually turns on
    // `alpha_rename_locals`: post-splice, `b` is a caller local, so the
    // returned reference into it survives past the (no longer existing)
    // callee frame. `+!` through the reference and reading the new value back
    // out proves it is not dangling.
    let src = "type: P n u32 ;\n\
               : fresh inline ( -- &!u32 ) 7 >u32 P | b | &!b &!n ;\n\
               : main ( -- ) fresh | r | r @ >i64 . r 5 >u32 +! r @ >i64 . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-ref-callee-local", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "7\n12\n");
    assert_eq!(code, 0);
}

#[test]
fn transitive_inline_reference_output_is_accepted() {
    // Feature C's transitive shape: an `inline` word returning a reference,
    // called by another `inline` word that returns it on. Each splice layer
    // alpha-renames, so the chain bottoms out at `main`, whose frame owns the
    // referent. Both layers are load-bearing -- dropping `inline` from either
    // one rejects the program with the reference-output message. The write in
    // the middle layer and the write in `main` must land in the *same* `P`,
    // which they only do if the two splices resolved to one caller local.
    let src = "type: P n u32 ;\n\
               : fresh inline ( -- &!u32 ) 7 >u32 P | b | &!b &!n ;\n\
               : bump inline ( -- &!u32 ) fresh | r | r 5 >u32 +! r ;\n\
               : main ( -- ) bump | r | r @ >i64 . r 2 >u32 +! r @ >i64 . ;\n";
    let (binary, stdout, code) = build_and_run("slice11-ref-transitive", src);
    std::fs::remove_file(&binary).ok();
    assert_eq!(stdout, "12\n14\n");
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
        ": helper ( i64 -- i64 ) 100 add ;\n\
         : bump inline ( i64 -- i64 ) helper ;\n\
         5 bump\n\
         : helper ( i64 -- i64 ) 200 add ;\n\
         5 bump\n:quit\n",
    );
    assert_eq!(
        transcript,
        "defined helper\ndefined bump\nstack: 105\ndefined helper\nstack: 105 205\n"
    );
}

#[test]
fn repl_inline_polymorphic_signature_is_accepted() {
    // The REPL twin of the reversal above: the retention route (R7) carries a
    // variable-bearing `inline` word into the poly-combinator check, which is
    // now where it belongs.
    let transcript = repl_transcript(": id inline ( 'T -- 'T ) ;\n3 id\n:quit\n");
    assert_eq!(transcript, "defined id\nstack: 3\n");
}

// -- Phase 3 (Feature B): `lib/combinators.sth` retyped to `~[ ... ]` --------

/// `lib/combinators.sth` before this slice's retype (`each`/`map`/`fold`/
/// `filter`/`while` each took `[ ... ]`), embedded so the byte-identical claim
/// below is checked against the actual pre-retype source rather than trusted.
fn combinators_source(quotation_kind: &str) -> String {
    format!(
        r#"export: each map fold filter while ;

: times-helper inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )
  | f | | to | | from |
  from to lt ~[
    from f call
    from 1 add to f times-helper
  ] ~[
  ] if ;

: times inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )
  | f | | n | 0 n f times-helper ;

: each inline ( ['T 'N] {quotation_kind}[ 'T -- ] -- )
  | f | len >i64 | count | | arr |
  count ~[ | i | &arr i >usize &> @ f call ] times
  arr drop ;

: map inline ( ['T 'N] {quotation_kind}[ 'T -- 'T ] -- ['T 'N] )
  | f | len >i64 | count | | arr |
  count ~[ | i | &arr i >usize &> @ f call | v | &!arr i >usize &!> v ! ] times
  arr ;

: fold inline ( ['T 'N] 'A {quotation_kind}[ 'A 'T -- 'A ] -- 'A )
  | f | | acc | len >i64 | count | | arr |
  acc count ~[ | i | &arr i >usize &> @ f call ] times
  arr drop ;

: filter inline ( ['T: Copy 'N] {quotation_kind}[ 'T -- bool ] -- ['T 'N] usize )
  | p | len >i64 | n | | arr |
  0 n ~[ | i | &arr i >usize &> @ dup p call ~[
          | v | &!arr over >usize &!> v ! 1 add
        ] ~[ drop ] if ] times
  | wf | arr wf >usize ;

: while inline ( 'a {quotation_kind}[ 'a -- 'a bool ] -- 'a )
  | p | p call ~[ p while ] ~[ ] if ;
"#
    )
}

/// A corpus program exercising all five combinators (`each`/`map`/`fold`/
/// `filter`/`while`) in a single build. Slice 12 (R-C2): the tilde is now
/// spelling-significant against the callee's declared flavour, so this must
/// be generated per variant too -- the pre-retype build's `each` etc. declare
/// an ordinary `[ ... ]` parameter and reject a `~[ ... ]` argument outright
/// (E3b), so one shared literal `&str` can no longer serve both builds.
fn combinators_main(quotation_kind: &str) -> String {
    format!(
        "\
: mkarr ( -- [i64 4] ) 0 4 fill ;

: main ( -- )
  mkarr {quotation_kind}[ 1 add drop ] each
  mkarr {quotation_kind}[ 2 mul ] map {quotation_kind}[ 1 add drop ] each
  mkarr 0 {quotation_kind}[ add ] fold .
  mkarr {quotation_kind}[ 2 gt ] filter drop drop
  0 {quotation_kind}[ | n | n 3 lt ~[ n 1 add true ] ~[ n false ] if ] while . ;
"
    )
}

#[test]
fn combinators_retype_output_byte_identical() {
    // Feature B: retyping each/map/fold/filter/while's quotation parameter
    // from `[ ... ]` to `~[ ... ]` is a pure library edit -- every call site
    // already only ever passed a literal `[ ... ]`, so the emitted QBE for a
    // program exercising all five combinators must be byte-identical before
    // and after the retype.
    let pre = combinators_source("") + &combinators_main("");
    let post = combinators_source("~") + &combinators_main("~");

    let pre_path =
        std::env::temp_dir().join(format!("sooth-slice11-pre-{}.sth", std::process::id()));
    let post_path =
        std::env::temp_dir().join(format!("sooth-slice11-post-{}.sth", std::process::id()));
    std::fs::write(&pre_path, &pre).expect("writing pre-retype source should succeed");
    std::fs::write(&post_path, &post).expect("writing post-retype source should succeed");

    let pre_ssa =
        sooth::driver::emit_ssa(&pre_path).expect("emitting pre-retype QBE should succeed");
    let post_ssa =
        sooth::driver::emit_ssa(&post_path).expect("emitting post-retype QBE should succeed");
    std::fs::remove_file(&pre_path).ok();
    std::fs::remove_file(&post_path).ok();

    assert_eq!(
        pre_ssa, post_ssa,
        "retyping the quotation parameters to `~[ ... ]` must not change emitted QBE"
    );
}

#[test]
fn combinators_retype_stored_quotation_still_rejected() {
    // A `~[ ... ]` parameter is `Type::InlineQuotation`, distinct from
    // `Type::Quotation` (`PartialEq` gives them unequal, `ast.rs`). A literal
    // `[ ... ]` bound straight to a local before the call still infers
    // `InlineQuotation` from the call site (D3/D6's directional check), but a
    // *genuinely first-class* quotation -- one that already crossed a
    // materialization boundary and so is grounded to ordinary `Type::Quotation`
    // (here, `give`'s declared `[ i64 -- ]` output) -- type-mismatches against
    // `each`'s retyped `~[ 'T -- ]` parameter. A stored/returned quotation still
    // requires an ordinary `[ ... ]`-typed parameter (7b's territory).
    let src = format!(
        "{}\
         : give ( -- [ i64 -- ] ) [ 1 add drop ] ;

         : main ( -- )
  0 4 fill | arr |
  give | f |
  arr f each ;
",
        combinators_source("~")
    );
    let err = check_error(&src);
    assert_eq!(
        err,
        "error: `each` expects a quotation `~[ i64 -- ]` here, found `[ i64 -- ]` in `main` (line 43)"
    );
}

#[test]
fn combinators_library_uses_tilde_quotation_parameters() {
    // The retype itself, checked against the shipped file rather than the
    // isolated copy above: without it, reverting Feature B (`~[` back to `[`
    // in `lib/combinators.sth`) leaves the rest of this suite green, since the
    // byte-identical and stored-quotation-rejection tests above both build
    // their own copy of the library text and never read the file on disk.
    let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/combinators.sth"))
        .expect("the combinator library should be readable");
    for (word, quotation) in [
        ("each", "~[ 'T -- ]"),
        ("map", "~[ 'T -- 'T ]"),
        ("fold", "~[ 'A 'T -- 'A ]"),
        ("filter", "~[ 'T -- bool ]"),
        ("while", "~[ 'a -- 'a bool ]"),
    ] {
        assert!(
            lib.contains(quotation),
            "`{word}` in lib/combinators.sth should declare `{quotation}`, got:\n{lib}"
        );
    }
}
