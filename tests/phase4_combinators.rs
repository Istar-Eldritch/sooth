//! Phase 4 Slice 6a, phase 1 goldens: quotation *types* become nameable in a
//! word's declared effect, and a monomorphic quotation-taking word (a
//! combinator) is inlined (term-spliced) at its call sites, minting no
//! `IrFunc`. Value/effect goldens go through `run_src`; diagnostic goldens
//! through the sanctioned `check_error`/`parse_error` helpers. Every negative
//! golden asserts the message text and the named identifiers, never an op name
//! or an exit code.

use std::io::BufReader;

use sooth::ast::Type;
use sooth::{check, lexer, parser};

/// Compile and run `src`, returning stdout and the exit code. `name`
/// distinguishes the temp source per test (the goldens run in parallel).
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();
    (
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

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    parser::parse(&tokens).expect_err("parsing should fail")
}

/// The linear stand-in: a one-field struct with a `drop` overload, so a
/// capture of it is a linear (not `Copy`) capture (D3).
const SPY_DEF: &str = "type: Spy tag i64 ;\n\
    : drop ( Spy -- )  | s | s Spy>tag . ;\n";

// -- criterion 1: the type parses --------------------------------------------

#[test]
fn quotation_type_in_signature_parses() {
    // `[ i64 -- i64 ]` and the nil effect `[ -- ]` both parse to a quotation
    // *type* in an input slot, not an array.
    let src = ": a ( [ i64 -- i64 ] -- ) drop ;\n\
               : b ( [ -- ] -- ) drop ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let a_in = module.words[0].effect.inputs[0].ty;
    let b_in = module.words[1].effect.inputs[0].ty;
    assert!(
        matches!(a_in, Type::Quotation(_)),
        "`[ i64 -- i64 ]` should parse to a quotation type, got {a_in:?}"
    );
    assert!(
        matches!(b_in, Type::Quotation(_)),
        "`[ -- ]` should parse to a quotation type, got {b_in:?}"
    );
}

// -- criterion 1b: `[i64]` stays the array diagnostic -------------------------

#[test]
fn array_type_without_arrow_stays_array_diagnostic() {
    // No top-depth `--`, so the scan takes the array branch and reaches
    // `parse_array_count`: the disambiguation must not swallow this.
    let err = parse_error(": main ( [i64] -- ) drop ;\n");
    assert!(
        err.contains("array count must be a decimal literal"),
        "`[i64]` should still be the array-count diagnostic, got: {err}"
    );
}

// -- criterion 1c: a malformed effect is a located parse error ----------------

#[test]
fn malformed_quotation_type_is_located_parse_error() {
    // A `--` (so the quotation branch is taken) with an unterminated bracket
    // is a located parse error, not a silent array fall-through or a panic.
    let err = parse_error(": main ( [ i64 -- i64 -- ) drop ;\n");
    assert!(
        err.contains("line 1"),
        "a malformed quotation effect should be a located parse error, got: {err}"
    );
}

// -- criterion 2b: the type-position audit (table-driven) ---------------------

#[test]
fn quotation_type_is_rejected_at_every_audited_position() {
    // R7a: exactly one position accepts a quotation type (a direct word
    // parameter); every other is a located rejection naming the position and
    // slice 7. One row per audited position: deleting any one rejection must
    // flip its row from Err to Ok and fail this test, which is what makes
    // R7's `unreachable!` mangling/`IrType` arms sound.
    struct Row {
        src: &'static str,
        position: &'static str,
    }
    let rows = [
        Row {
            src: "type: S f [ i64 -- ] ;\n: main ( -- ) ;\n",
            position: "the field `f` of struct `S`",
        },
        Row {
            src: "type: E | v p [ i64 -- ] ;\n: main ( -- ) ;\n",
            position: "the field `p` of enum variant `E::v`",
        },
        Row {
            src: ": main ( [ [ i64 -- ] 3 ] -- ) drop ;\n",
            position: "an array element",
        },
        Row {
            src: ": main ( ^[ i64 -- ] -- ) drop ;\n",
            position: "an owned-cell payload",
        },
        Row {
            src: ": main ( &[ i64 -- ] -- ) drop ;\n",
            position: "a reference referent",
        },
        Row {
            src: ": mut ( &![ i64 -- ] -- ) drop ;\n: main ( -- ) ;\n",
            position: "a reference referent",
        },
        Row {
            src: ": f ( -- [ i64 -- ] ) [ drop ] ;\n: main ( -- ) ;\n",
            position: "the output of `f`",
        },
        Row {
            src: "extern: c ( [ i64 -- ] -- ) \"c_fn\" ;\n: main ( -- ) ;\n",
            position: "an `extern:` boundary type of `c`",
        },
        Row {
            src: "extern: c ( -- [ i64 -- ] ) \"c_fn\" ;\n: main ( -- ) ;\n",
            position: "an `extern:` boundary type of `c`",
        },
        Row {
            src: ": main ( [ i64 -- ] -- ) drop ;\n",
            position: "an input of `main`",
        },
        Row {
            src: ": nest ( [ [ i64 -- ] -- ] -- ) drop ;\n: main ( -- ) ;\n",
            position: "nested inside a quotation effect",
        },
    ];
    for Row { src, position } in rows {
        let err = check_error(src);
        assert!(
            err.contains("a quotation type") && err.contains(position),
            "audited position `{position}` should be a located quotation-type rejection, got: {err}"
        );
        assert!(
            err.contains("slice 7"),
            "audited position `{position}` should name slice 7 as the lift, got: {err}"
        );
    }
}

// -- criterion 2c: array-of-quotation is the array-element rejection ----------

#[test]
fn array_of_quotation_type_is_located_rejection() {
    // `[ [ i64 -- ] 3 ]` has no top-depth `--` (the inner one is at depth 1),
    // so it takes the array branch and parses as an array of quotations; it
    // must reject at the array-element position, not error as an array count
    // and not panic in layout.
    let err = check_error(": main ( [ [ i64 -- ] 3 ] -- ) drop ;\n");
    assert!(
        err.contains("a quotation type") && err.contains("an array element"),
        "an array-of-quotation type should reject at the array-element position, got: {err}"
    );
    assert!(
        !err.contains("array count"),
        "it must not be the array-count diagnostic, got: {err}"
    );
}

// -- criterion 3: the monomorphic combinator inlines and runs -----------------

#[test]
fn monomorphic_quotation_taking_word_inlines_and_runs() {
    let src = ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
               : main ( -- ) 3 [ 1 + ] apply . ;\n";
    let (stdout, code) = run_src("apply", src);
    assert_eq!(stdout, "4\n");
    assert_eq!(code, 0);
}

// -- criterion 4: a literal disagreeing with the parameter effect -------------

#[test]
fn literal_effect_mismatch_against_parameter_is_error() {
    // `[ 1 + . ]` has effect `[ i64 -- ]`, disagreeing with the declared
    // `[ i64 -- i64 ]`; the error names the word, both effects.
    let err = check_error(
        ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
         : main ( -- ) 3 [ 1 + . ] apply . ;\n",
    );
    assert!(
        err.contains("`apply`") && err.contains("[ i64 -- i64 ]") && err.contains("[ i64 -- ]"),
        "the mismatch should name `apply` and both effects, got: {err}"
    );
}

// -- criterion 5 / 5b: the D3 capture pair ------------------------------------

#[test]
fn quotation_literal_capturing_linear_local_is_error() {
    // A literal that consumes a linear enclosing local (`s`, a `Spy`) is a
    // located D3 rejection naming the local. Asserts the R12-specific
    // wording ("consumes the enclosing local" / "(D3)"), not just `s` and
    // "linear": the inliner's second (splice-time) run of the literal body
    // also rejects a moved `s` with a generic use-after-move message that
    // shares those two substrings, so a weaker assertion here would stay
    // green even if R12's own capture guard were deleted.
    let src = format!(
        "{SPY_DEF}\
         : apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
         : main ( -- ) 7 Spy | s | 3 [ s Spy>tag + ] apply . ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("`s`") && err.contains("consumes the enclosing local") && err.contains("(D3)"),
        "capturing a linear local should be R12's located D3 rejection naming `s`, got: {err}"
    );
}

#[test]
fn quotation_literal_borrowing_enclosing_place_is_error() {
    // The other half of criterion 5: a literal that leaves a borrow of an
    // enclosing place (`v`, a struct local) on its exit row, rather than
    // consuming it outright, is also a located D3 rejection naming the local.
    let src = "type: V x i64 y i64 ;\n\
               : apply ( [ -- &V ] -- ) call drop ;\n\
               : main ( -- ) 1 2 V | v | [ &v ] apply ;\n";
    let err = check_error(src);
    assert!(
        err.contains("`v`") && err.contains("borrows the enclosing place") && err.contains("(D3)"),
        "borrowing an enclosing place should be a located D3 rejection naming `v`, got: {err}"
    );
}

#[test]
fn quotation_literal_capturing_copy_local_runs() {
    // Reading a `Copy` enclosing local (`n`, an `i64`) by value is accepted
    // and runs.
    let src = ": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
               : main ( -- ) 10 | n | 3 [ n + ] apply . ;\n";
    let (stdout, code) = run_src("copy_capture", src);
    assert_eq!(stdout, "13\n");
    assert_eq!(code, 0);
}

// -- criterion 6: a bound quotation parameter used twice splices twice --------

#[test]
fn quotation_parameter_used_twice_splices_twice() {
    // `| f | f call f call` splices the body twice; the second run observes
    // the first's effect (3 -> 4 -> 5).
    let src = ": twice ( i64 [ i64 -- i64 ] -- i64 ) | f | f call f call ;\n\
               : main ( -- ) 3 [ 1 + ] twice . ;\n";
    let (stdout, code) = run_src("twice", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

// -- criterion 7: a quotation against a non-quotation parameter ---------------

#[test]
fn quotation_against_non_quotation_parameter_is_error() {
    // Still rejected, reworded off "Phase 6" to slice 7 (R26).
    let err = check_error(
        ": f ( i64 -- i64 ) ;\n\
         : main ( -- ) [ 1 + ] f . ;\n",
    );
    assert!(
        err.contains("a quotation cannot be passed to `f`") && err.contains("slice 7"),
        "a non-quotation parameter should reject, reworded to slice 7, got: {err}"
    );
    assert!(
        !err.contains("Phase 6"),
        "the diagnostic must not point at the stale `Phase 6`, got: {err}"
    );
}

// -- criterion 17 (phase 3): the stale "Phase 6" diagnostics are reworded -----

#[test]
fn stale_phase6_diagnostics_are_reworded() {
    // R26: eight diagnostics that used to say "higher-order values are Phase
    // 6" now name slice 7 (a *runtime* quotation value), since this slice
    // makes the type nameable and a quotation-taking word a library word --
    // "Phase 6" was never a real milestone name and is now flatly wrong.
    let checked_rows: &[(&str, &str)] = &[
        // `call` without a resolvable quotation on top.
        (
            ": main ( -- ) 5 call ;\n",
            "expects a quotation on the stack",
        ),
        // `times` without a resolvable quotation on top.
        (
            ": main ( -- ) 3 5 times ;\n",
            "expects a quotation on the stack",
        ),
        // an operator operand.
        (
            ": main ( -- ) [ + ] 1 + ;\n",
            "cannot take a quotation as an operand",
        ),
        // a stored quotation (`fill`'s element).
        (
            ": main ( -- ) [ + ] 8 fill drop ;\n",
            "a quotation cannot be stored",
        ),
        // two different quotations at an `if` join.
        (
            ": main ( -- ) true if [ 1 + ] else [ 1 - ] end drop ;\n",
            "leave different quotations",
        ),
        // a quotation on one `if` arm, a value on the other.
        (
            ": main ( -- ) true if [ 1 + ] else \"x\" cstr end drop ;\n",
            "leaves a quotation and the other does not",
        ),
    ];
    for (src, phrase) in checked_rows {
        let err = check_error(src);
        assert!(
            err.contains(phrase),
            "row `{src}` should still produce its phrase `{phrase}`, got: {err}"
        );
        assert!(
            err.contains("slice 7"),
            "row `{src}` should name slice 7, got: {err}"
        );
        assert!(
            !err.contains("Phase 6"),
            "row `{src}` must not point at the stale `Phase 6`, got: {err}"
        );
    }

    // The ninth site (R19's residual REPL line) is checked through the REPL,
    // not `check_error`.
    let transcript = repl_error("1 [ + ]\n:quit\n");
    assert!(
        transcript.contains("a quotation cannot be left on the stack at the end of a line"),
        "the residual-line rejection should still fire, got: {transcript}"
    );
    assert!(
        transcript.contains("slice 7"),
        "the residual-line rejection should name slice 7, got: {transcript}"
    );
    assert!(
        !transcript.contains("Phase 6"),
        "the residual-line rejection must not point at the stale `Phase 6`, got: {transcript}"
    );
}

// -- criterion 8 / 8b: the cycle rejection ------------------------------------

#[test]
fn recursive_quotation_taking_word_is_located_error() {
    // A self-recursive combinator would splice forever; a self-edge is itself
    // the error (unlike a self-tail-recursive ordinary word, which loops).
    let err = check_error(
        ": loopy ( i64 [ i64 -- i64 ] -- i64 ) loopy ;\n\
         : main ( -- ) 3 [ 1 + ] loopy . ;\n",
    );
    assert!(
        err.contains("`loopy`") && err.contains("recursive"),
        "a self-recursive combinator should be a located cycle rejection naming it, got: {err}"
    );
}

#[test]
fn clause_bodied_quotation_taking_word_is_located_rejection() {
    // R18/R7a: a clause body cannot be term-spliced, so a monomorphic word
    // taking a quotation with a clause body is rejected here rather than left
    // to panic at lowering (`ir_type_of` on the quotation parameter). Without
    // this guard the program below is a compiler panic, the exact failure R7a
    // exists to prevent.
    let err = check_error(
        "type: Opt | None | Some ;\n\
         : pick ( [ i64 -- i64 ] Opt -- i64 )\n\
         | None drop 0\n\
         | Some drop 1\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("`pick`") && err.contains("clause body") && err.contains("slice 7"),
        "a clause-bodied quotation-taking word should be a located rejection, got: {err}"
    );
}

#[test]
fn quotation_taking_word_cycle_names_members() {
    // A two-word cycle (non-tail, so it is the splice-forever rejection, not
    // mutual tail recursion) names both members.
    let err = check_error(
        ": a ( i64 [ i64 -- i64 ] -- i64 ) [ 1 + ] b 1 + ;\n\
         : b ( i64 [ i64 -- i64 ] -- i64 ) [ 1 + ] a 1 + ;\n\
         : main ( -- ) 3 [ 1 + ] a . ;\n",
    );
    assert!(
        err.contains("`a`") && err.contains("`b`") && err.contains("recursive"),
        "a two-word combinator cycle should name both members, got: {err}"
    );
}

// -- criterion 15 (phase 3): a session line defining a quotation-taking word -

fn repl_error(input: &str) -> String {
    let reader = BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sooth::repl::run(reader, &mut out).expect("the REPL loop itself should not error");
    String::from_utf8(out).expect("REPL output should be utf8")
}

#[test]
fn repl_quotation_taking_definition_is_rejected() {
    // R23/D7: the inliner needs a callee's AST body threaded into every call
    // site, but a session discards a defining line's body once it compiles (the
    // 6c retention problem), so a quotation-taking word is a located REPL
    // rejection naming the word, not a silent miscompile.
    let transcript = repl_error(": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n:quit\n");
    assert!(
        transcript.contains("`apply`") && transcript.contains("not yet supported at the REPL"),
        "located rejection naming the word: {transcript}"
    );
}

#[test]
fn repl_poly_quotation_taking_definition_is_rejected() {
    // The same rejection reached through the polymorphic definition path
    // (`eval_poly_def`), since a poly word's declared effect lives in
    // `word.poly`, not `word.effect`.
    let transcript = repl_error(
        ": each ( ['T 'N] [ 'T -- ] -- ) | f | len >i64 | count | | arr | count [ | i | &arr i >usize &> @ f call ] times arr drop ;\n:quit\n",
    );
    assert!(
        transcript.contains("`each`") && transcript.contains("not yet supported at the REPL"),
        "located rejection naming the word: {transcript}"
    );
}

// -- criterion 18 (phase 4): dogfood, the combinator rewrite ------------------

#[test]
fn combinators_dogfood_matches_hand_threaded() {
    // R27: `examples/array_totals.sth` sums and doubles a small array through
    // `fold`/`map`/`each` imported from `lib/combinators.sth`. This asserts
    // it builds and runs to the same total and doubled elements as its
    // hand-threaded twin, `examples/array_totals_hand.sth`: three manual
    // `times` loops over the same array, each with a synthesized `>usize`
    // index, a `&arr i &> @` read, and (for the doubling loop) a
    // `&!arr i &!> v !` write, the exact shape the inliner must produce
    // (recon 2). Both are real committed files, not a string invented inside
    // this test, so the equivalence is pinned against an actual baseline.
    fn build_and_run(path: &str) -> (String, Option<i32>) {
        let binary =
            sooth::driver::build(std::path::Path::new(path)).expect("example should build");
        let output = std::process::Command::new(&binary)
            .output()
            .expect("binary should run");
        std::fs::remove_file(&binary).ok();
        (
            String::from_utf8(output.stdout).expect("stdout should be utf8"),
            output.status.code(),
        )
    }

    let (hand_stdout, hand_code) = build_and_run("examples/array_totals_hand.sth");
    assert_eq!(hand_code, Some(0));
    assert_eq!(hand_stdout, "25\n6\n14\n2\n18\n10\n");

    let (combinator_stdout, combinator_code) = build_and_run("examples/array_totals.sth");
    assert_eq!(combinator_stdout, hand_stdout);
    assert_eq!(combinator_code, Some(0));
}

// -- criterion 16 (phase 3): importing a quotation-exporting closure ----------

/// Write `contents` to a uniquely-named temp `.sth` file and return its path.
/// Each test names its file distinctly so the goldens can run in parallel.
fn temp_lib(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, contents).expect("writing temp library should succeed");
    path
}

#[test]
fn repl_import_exporting_quotation_word_is_rejected() {
    // R24/D7: importing a closure that *exports* a quotation-taking word is a
    // located rejection at import time, naming the file and the word. The
    // session retains no body to inline for it (a quotation-taking word mints
    // no `IrFunc`, R20), so without this the import succeeds and the failure
    // surfaces later as a misdirected `unknown word` or an internal mangled
    // name leaking into the diagnostic.
    let path = temp_lib(
        "crit16-exports",
        "export: apply_each ;\n: apply_each ( i64 [ i64 -- i64 ] -- i64 ) call ;\n",
    );
    let transcript = repl_error(&format!("import: c \"{}\" ;\n:quit\n", path.display()));
    std::fs::remove_file(&path).ok();
    assert!(
        transcript.contains("apply_each")
            && transcript.contains(&path.display().to_string())
            && transcript.contains("quotation parameter"),
        "located rejection naming the file and the word: {transcript}"
    );
    // A user-facing diagnostic must never leak a compiler-internal mangled
    // name (`__import`, `__inl`, `__m0`, `quo__`).
    assert!(
        !transcript.contains("__import")
            && !transcript.contains("__inl")
            && !transcript.contains("__m0")
            && !transcript.contains("quo__"),
        "no mangled internal name in the diagnostic: {transcript}"
    );

    // A quotation-taking word used purely *internally* to an imported closure
    // (not exported) must still import and run fine: it inlines during the
    // closure's own native compilation.
    let internal = temp_lib(
        "crit16-internal",
        "export: bump ;\n: ap ( i64 [ i64 -- i64 ] -- i64 ) call ;\n: bump ( i64 -- i64 ) [ 1 + ] ap ;\n",
    );
    // Leave the result on the stack (no `.`): a runtime `.` prints to the real
    // process stdout, not this captured writer, but the REPL's own residual
    // stack render goes to the writer, so `stack: 6` witnesses that `bump`
    // imported, inlined `ap`, and ran.
    let ok = repl_error(&format!(
        "import: c \"{}\" ;\n5 c::bump\n:quit\n",
        internal.display()
    ));
    std::fs::remove_file(&internal).ok();
    assert!(
        ok.contains("imported c") && ok.contains("stack: 6") && !ok.contains("error"),
        "internal-only quotation word imports and runs: {ok}"
    );
}
