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

mod common;

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

/// The check diagnostic for a program that `import:`s the real combinator
/// library: the bare `check_error` above resolves no imports, so a program
/// naming `c::while` needs the full driver (which resolves modules) to fail
/// its check.
fn build_check_error(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build(&path).expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    err
}

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    parser::parse(&tokens).expect_err("parsing should fail")
}

/// Assert `src` type-checks (a positive standalone-check golden).
fn check_ok(src: &str) {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
}

/// An `import:` line for the committed combinator library by *absolute* path,
/// so a temp source built under `temp_dir()` resolves it regardless of cwd.
fn combinators_import(qualifier: &str) -> String {
    format!(
        "import: {qualifier} \"{}/lib/combinators.sth\" ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The linear stand-in: a one-field struct with a `drop` overload, so a
/// capture of it is a linear (not `Copy`) capture (D3).
const SPY_DEF: &str = "type: Spy tag i64 ;\n\
    : drop ( Spy -- )  | s | s Spy>tag . ;\n";

/// `lib/combinators.sth`'s `times`, inlined: `check_error`/`check_ok` run the
/// checker in process, where an `import:` line never resolves, and a REPL
/// session takes one definition per line.
const TIMES_DEF: &str = ": times-helper ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s ) | f | | to | | from | from to < if from f call from 1 + to f times-helper else end ;\n\
    : times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s ) | f | | n | 0 n f times-helper ;\n";

#[test]
fn times_def_hand_copy_is_pinned_to_the_library() {
    common::assert_pinned_to_combinators_lib(TIMES_DEF, &[]);
}

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
    // A top-depth `--` routes `[ i64 -- @ ]` through the quotation branch, so a
    // malformed type on the output side (`@`) is a located parse error naming
    // the offending token, not a silent array fall-through. Asserting only
    // `line 1` is a placebo: with the R1 top-depth-`--` disambiguation
    // disabled the array branch fires instead and still contains `line 1`, on
    // the unrelated `array count must be a decimal literal`. Name the
    // quotation-branch message and token, and assert the array diagnostic is
    // *not* what fired.
    let err = parse_error(": main ( [ i64 -- @ ] -- ) drop ;\n");
    assert!(
        err.contains("unknown type") && err.contains("`@`"),
        "a malformed quotation output type should name the offending token, got: {err}"
    );
    assert!(
        err.contains("line 1") && err.contains("col 19"),
        "the parse error should be located at the offending token, got: {err}"
    );
    assert!(
        !err.contains("array count"),
        "the quotation branch must fire, not the array-count fall-through, got: {err}"
    );
}

// -- criterion 2b: the type-position audit (table-driven) ---------------------

#[test]
fn quotation_type_is_rejected_at_every_audited_position() {
    // R7a, revised for slice 7a: a quotation type is now legal at the three
    // D4 materialization boundaries (a struct field, an array element, a
    // monomorphic word output), so those positions moved to the positive
    // goldens in `tests/phase4_quotations.rs`. Every position *still* rejected
    // in 7a keeps a row here: a direct word parameter is the one accepting
    // position, and each other is a located rejection naming the position and
    // slice 7. Deleting any one rejection must flip its row from Err to Ok and
    // fail this test, which is what keeps R7's `unreachable!` arms sound for
    // the positions 7a does not lift.
    struct Row {
        src: &'static str,
        position: &'static str,
    }
    let rows = [
        Row {
            src: "type: E | v p [ i64 -- ] ;\n: main ( -- ) ;\n",
            position: "the field `p` of enum variant `E::v`",
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
        // The polymorphic twins: a poly word carries its signature in
        // `w.poly`, not `w.effect`, so the output-position and nested-in-effect
        // audits must walk the poly path too or these slip through.
        Row {
            src: ": w ( ['T 'N] [ 'T -- ] -- ['T 'N] [ 'T -- ] ) ;\n: main ( -- ) ;\n",
            position: "the output of `w`",
        },
        Row {
            src: ": w ( ['T 'N] [ [ 'T -- ] -- ] -- ) drop drop ;\n: main ( -- ) ;\n",
            position: "nested inside a quotation effect",
        },
        // Item 2: a quotation hiding in a poly *array element* -- the shallow
        // poly audit walked outputs and effect rows but never descended into
        // an array element, so `[ [ 'T -- ] 3 ]` and its variable-length twin
        // `[ [ 'T -- ] 'N ]` were both accepted, defeating R7's default-deny
        // `unreachable!` arms on the poly path. Their monomorphic twin
        // `[ [ i64 -- ] 3 ]` (an earlier row) was already caught via the
        // interned array registry.
        Row {
            src: ": w ( [ [ 'T -- ] 3 ] -- ) drop ;\n: main ( -- ) ;\n",
            position: "an array element",
        },
        Row {
            src: ": w ( [ [ 'T -- ] 'N ] -- ) drop ;\n: main ( -- ) ;\n",
            position: "an array element",
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

    // Item 2: the same rejections must fire on the REPL's chokepoints, which
    // ran `check_types` alone and skipped the R7a audit, so a quotation in any
    // audited position reached `ir.rs`'s `unreachable!` and bricked the
    // session. These rows cover the audit-reachable REPL positions (a direct
    // quotation *parameter* is instead R23's rejection, and `extern:`/`main`
    // are not REPL forms). Each asserts the located rejection *and* that the
    // session is not bricked: the following `1` line still evaluates.
    let repl_rows = [
        Row {
            src: "type: E | v p [ i64 -- ] ;\n1\n",
            position: "the field `p` of enum variant `E::v`",
        },
        Row {
            src: ": w ( ^[ i64 -- ] -- ) drop ;\n1\n",
            position: "an owned-cell payload",
        },
        Row {
            src: ": w ( &[ i64 -- ] -- ) drop ;\n1\n",
            position: "a reference referent",
        },
    ];
    for Row { src, position } in repl_rows {
        let transcript = repl_error(src);
        assert!(
            transcript.contains("a quotation type")
                && transcript.contains(position)
                && transcript.contains("slice 7"),
            "REPL audited position `{position}` should be a located quotation-type rejection naming slice 7, got: {transcript}"
        );
        assert!(
            transcript.contains("stack: 1"),
            "the REPL session must survive the rejection (the following `1` still runs), got: {transcript}"
        );
    }

    // Item 1: a `type:` line naming a quotation in an *interned* position
    // (array element / cell payload / reference referent, struct or enum) used
    // to brick the session: the failing line rolled back only `self.structs` /
    // `self.enums`, leaving the poisoned interned array/cell/ref entry
    // resident, so the per-line audit re-fired against it forever. Each row
    // asserts the located rejection *and* that a following line still
    // evaluates: `40 2 +` must leave `stack: 42`. A bricked session would
    // re-fire the rejection with `stack: (empty)` instead. The residual-stack
    // line, not the `.` output, is the witness -- `.` prints to process stdout,
    // which `repl_error` does not capture. The `&` shapes reject earlier (a
    // reference may not be stored in a field) but interned the referent all
    // the same, so they brick identically without the registry rollback.
    struct BrickRow {
        src: &'static str,
        msg: &'static str,
    }
    // An array-of-quotation field (struct or enum) is now legal (the array
    // carve-out interns it), so the two `[ [ i64 -- ] 3 ]` rows moved to the
    // positive goldens; a cell-payload quotation and a reference field stay
    // rejected, and must still leave the session usable.
    let item1_rows = [
        BrickRow {
            src: "type: P x ^[ i64 -- ] ;\n40 2 +\n",
            msg: "a quotation type",
        },
        BrickRow {
            src: "type: P x &[ i64 -- ] ;\n40 2 +\n",
            msg: "a reference cannot be stored",
        },
        BrickRow {
            src: "type: X | Mk a ^[ i64 -- ] ;\n40 2 +\n",
            msg: "a quotation type",
        },
        BrickRow {
            src: "type: Y | Mk a &[ i64 -- ] ;\n40 2 +\n",
            msg: "a reference cannot be stored",
        },
    ];
    for BrickRow { src, msg } in item1_rows {
        let transcript = repl_error(src);
        assert!(
            transcript.contains(msg),
            "the `type:` line should be a located rejection (`{msg}`), got: {transcript}"
        );
        assert!(
            transcript.contains("stack: 42"),
            "the session must survive: the following `40 2 +` must leave `stack: 42`, got: {transcript}"
        );
    }
}

// -- criterion 2c: array-of-quotation is the array-element rejection ----------

#[test]
fn array_of_quotation_type_is_a_legal_declaration() {
    // `[ [ i64 -- ] 3 ]` has no top-depth `--` (the inner one is at depth 1),
    // so it takes the array branch and parses as an array of quotations, not
    // an array count. Slice 7a lifts the array-element boundary, so declaring
    // one is now legal (pre-7a this was a located rejection); it must neither
    // error as an array count nor panic in layout.
    let tokens =
        lexer::lex(": main ( [ [ i64 -- i64 ] 3 ] -- ) drop ;\n").expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("an array-of-quotation type is legal to declare in 7a");
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

// -- R21: forwarding an abstract quotation parameter to a nested combinator ----

#[test]
fn abstract_quotation_forward_inlines_and_runs() {
    // R21: `outer` passes its own quotation *parameter* `f` (an abstract
    // `[ i64 -- ]`, not a literal) to `inner`. The def-site check of `outer`
    // accepts the forward and splices `inner`, whose `call` checks `f` against
    // its declared effect; at the real call site the literal `[ 1 + . ]` flows
    // through both frames and prints `8`.
    let src = ": inner ( i64 [ i64 -- ] -- ) call ;\n\
               : outer ( i64 [ i64 -- ] -- ) inner ;\n\
               : main ( -- ) 7 [ 1 + . ] outer ;\n";
    let (stdout, code) = run_src("forward", src);
    assert_eq!(stdout, "8\n");
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
         : main ( -- ) 7 Spy | s | 3 [ s drop 0 + ] apply . ;\n"
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
    // R26: seven diagnostics that used to say "higher-order values are Phase
    // 6" now name slice 7 (a *runtime* quotation value), since this slice
    // makes the type nameable and a quotation-taking word a library word --
    // "Phase 6" was never a real milestone name and is now flatly wrong.
    //
    // An eighth row, `times` without a resolvable quotation on top, went with
    // the intrinsic in 10b: `times` is an ordinary library word now, and its
    // rejection comes from the general inline-quotation-parameter path
    // (`` `times` expects a quotation `~[ i64 -- ]` here, found `i64` ``),
    // which is not one of the R26 sites and names no milestone at all.
    let checked_rows: &[(&str, &str)] = &[
        // `call` without a resolvable quotation on top.
        (
            ": main ( -- ) 5 call ;\n",
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

    // The last site (R19's residual REPL line) is checked through the REPL,
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
    // A combinator with a *non-tail* self-call would splice forever; the
    // self-edge is the error. (Slice 6b's D5 relaxation permits a *tail-only*
    // self-edge, which the loop transform makes finite -- see
    // `self_tail_combinator_edge_is_allowed`.)
    let err = check_error(
        ": loopy ( i64 [ i64 -- i64 ] -- i64 ) loopy drop ;\n\
         : main ( -- ) 3 [ 1 + ] loopy . ;\n",
    );
    assert!(
        err.contains("`loopy`") && err.contains("recursive"),
        "a non-tail self-recursive combinator should be a located cycle rejection naming it, got: {err}"
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

#[test]
fn polymorphic_combinator_cycle_is_located_error() {
    // R22 fires over the *polymorphic* combinator subgraph too: the cycle pass
    // runs on the call graph before any body check, so admitting the abstract
    // quotation forward (R21) does not let a poly combinator cycle slip past.
    // A two-word poly cycle (and a mixed mono/poly one) both name their
    // members.
    let poly = check_error(
        ": a ( ['T 'N] [ 'T -- ] -- ) [ drop ] b ;\n\
         : b ( ['T 'N] [ 'T -- ] -- ) [ drop ] a ;\n\
         : main ( -- ) 0 3 fill [ drop ] a ;\n",
    );
    assert!(
        poly.contains("`a`") && poly.contains("`b`") && poly.contains("recursive"),
        "a polymorphic combinator cycle should name both members, got: {poly}"
    );
    let mixed = check_error(
        ": a ( ['T 'N] [ 'T -- ] -- ) [ drop ] b ;\n\
         : b ( i64 [ i64 -- ] -- ) 0 3 fill [ drop ] a ;\n\
         : main ( -- ) 0 3 fill [ drop ] a ;\n",
    );
    assert!(
        mixed.contains("`a`") && mixed.contains("`b`") && mixed.contains("recursive"),
        "a mixed mono/poly combinator cycle should name both members, got: {mixed}"
    );
}

#[test]
fn combinator_through_helper_recursion_is_not_a_splice_cycle() {
    // R22 tracks only combinator -> combinator edges: a combinator (`comb`)
    // that calls a *non-combinator* helper which calls the combinator back is
    // ordinary runtime recursion, not a splice-forever cycle, so it must still
    // compile. The back-calls are non-tail (`0 drop`) to stay clear of the
    // unrelated mutual-tail-recursion rejection. Admitting the R21 abstract
    // forward must not newly reject this.
    let src = ": helper ( i64 -- )\n\
                 | n |\n\
                 n 0 > if\n\
                   n 1 - [ . ] comb\n\
                   0 drop\n\
                 else\n\
                 end ;\n\
               : comb ( i64 [ i64 -- ] -- )\n\
                 | f | | n |\n\
                 n f call\n\
                 n helper\n\
                 0 drop ;\n\
               : main ( -- ) 3 [ . ] comb ;\n";
    let (stdout, code) = run_src("helper_recursion", src);
    assert_eq!(stdout, "3\n2\n1\n0\n");
    assert_eq!(code, 0);
}

// == phase 2: the polymorphic path and the combinator library ================

// -- criterion 9: `each` checks standalone -----------------------------------

#[test]
fn each_checks_standalone() {
    // Criterion 9 (R16/R17/D4): `each`'s polymorphic body type-checks at its
    // own def site with no call site and no concrete literal, because `f call`
    // in its `times` body checks against `f`'s *declared* effect `[ 'T -- ]`
    // exactly as an ordinary word call checks against a `Sig`. The signature is
    // not documentation over a macro.
    let each = ": each ( ['T 'N] [ 'T -- ] -- )\n\
                | f | len >i64 | count | | arr |\n\
                count [ | i | &arr i >usize &> @ f call ] times\n\
                arr drop ;\n";
    check_ok(&format!("{TIMES_DEF}{each}"));

    // Pin the isolated copy to the committed library: if the real file stopped
    // checking standalone, the copy above could silently drift from it.
    let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/combinators.sth"))
        .expect("the combinator library should be readable");
    check_ok(&lib);
}

// -- criterion 9b: `map`/`fold` check compositionally ------------------------

#[test]
fn map_and_fold_check_compositionally() {
    // Criterion 9b: `map`/`fold` are leaf combinators, each driving its own
    // `times`, rather than being built on `each`. That is a cost choice, not a
    // scope limit: building them on `each` *is* expressible (the accumulator
    // rides a captured one-element array through balanced `&`/`&!` borrows,
    // which D3 as shipped accepts), but inlining is total, so composition
    // depth is code size at every call site, and it costs an extra array copy
    // and a counter cell. "Compositional" (D4) therefore means each body checks
    // `f call` against `f`'s *declared quotation effect*, not a concrete
    // literal body. Both check standalone:
    let map = ": map ( ['T 'N] [ 'T -- 'T ] -- ['T 'N] )\n\
               | f | len >i64 | count | | arr |\n\
               count [ | i | &arr i >usize &> @ f call | v | &!arr i >usize &!> v ! ] times\n\
               arr ;\n";
    let fold = ": fold ( ['T 'N] 'A [ 'A 'T -- 'A ] -- 'A )\n\
                | f | | acc | len >i64 | count | | arr |\n\
                acc count [ | i | &arr i >usize &> @ f call ] times\n\
                arr drop ;\n";
    check_ok(&format!("{TIMES_DEF}{map}"));
    check_ok(&format!("{TIMES_DEF}{fold}"));

    // ...and the compositional check bites at the def site: declaring `f` as
    // `[ 'T -- 'T ]` (producing a value) but leaving that value on the floor
    // unbalances the `times` row. That this is located -- naming `m`, at its
    // own def site -- is proof `f call` was checked to *produce* a `'T` per the
    // declared effect, not rubber-stamped.
    let err = check_error(&format!(
        "{TIMES_DEF}: m ( ['T 'N] [ 'T -- 'T ] -- )\n\
         | f | len >i64 | count | | arr |\n\
         count [ | i | &arr i >usize &> @ f call ] times\n\
         arr drop ;\n"
    ));
    assert!(
        err.contains("`m`")
            && err.contains("the quotation passed to `times` was declared")
            && err.contains("but its body has effect"),
        "mishandling `f`'s declared result should be a located def-site row error naming `m`, got: {err}"
    );
}

// -- criterion 10: `each` over an array inlines and runs ----------------------

#[test]
fn each_over_array_inlines_and_runs() {
    // Criterion 10 (R17/R19/R21): `arr [ . ] c::each` over an `[i64 4]` inlines
    // the imported combinator to a `times` loop and prints each element in
    // order.
    let src = format!(
        "{}: arr ( -- [i64 4] )\n\
         0 4 fill | s |\n\
         &!s 0 >usize &!> 1 !\n\
         &!s 1 >usize &!> 2 !\n\
         &!s 2 >usize &!> 3 !\n\
         &!s 3 >usize &!> 4 !\n\
         s ;\n\
         : main ( -- ) arr [ . ] c::each ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("each_over_array", &src);
    assert_eq!(stdout, "1\n2\n3\n4\n");
    assert_eq!(code, 0);
}

// -- criterion 11: `fold` computes a sum -------------------------------------

#[test]
fn fold_computes_sum() {
    // Criterion 11 (R17/R19): `fold` threads an accumulator across an `[i64 4]`
    // and sums 4 + 8 + 7 + 9 to 28.
    let src = format!(
        "{}: arr ( -- [i64 4] )\n\
         0 4 fill | s |\n\
         &!s 0 >usize &!> 4 !\n\
         &!s 1 >usize &!> 8 !\n\
         &!s 2 >usize &!> 7 !\n\
         &!s 3 >usize &!> 9 !\n\
         s ;\n\
         : main ( -- ) arr 0 [ + ] c::fold . ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("fold_sum", &src);
    assert_eq!(stdout, "28\n");
    assert_eq!(code, 0);
}

// -- criterion 1: `filter` checks standalone ---------------------------------

#[test]
fn filter_checks_standalone() {
    // Criterion 1 (R1/R2): `filter`'s body -- an `if`/`else`/`end` inside a
    // `times` body threading a write cursor below the index -- checks at its
    // own def site with no call site and no compiler change. Combinators
    // splice at the concrete call site (recon 1), so the polymorphic-`if`
    // rejection never gates this.
    let filter = ": filter ( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize )\n\
                  | p | len >i64 | n | | arr |\n\
                  0 n [ | i | &arr i >usize &> @ dup p call if\n\
                          | v | &!arr over >usize &!> v ! 1 +\n\
                        else drop end ] times\n\
                  | wf | arr wf >usize ;\n";
    check_ok(&format!("{TIMES_DEF}{filter}"));
}

// -- criterion 2: `filter` over `[i64 4]` inlines, runs, and compacts --------

#[test]
fn filter_over_array_inlines_and_runs() {
    // Criterion 2 (R1): `arr [ 4 > ] c::filter` over `[i64 8 3 9 1]` inlines
    // through 6a's inliner, prints the kept count `2`, and the array is
    // compacted in place, with `8` and `9` at the front.
    let src = format!(
        "{}: arr ( -- [i64 4] )\n\
         0 4 fill | s |\n\
         &!s 0 >usize &!> 8 !\n\
         &!s 1 >usize &!> 3 !\n\
         &!s 2 >usize &!> 9 !\n\
         &!s 3 >usize &!> 1 !\n\
         s ;\n\
         : main ( -- )\n\
           arr [ 4 > ] c::filter | n | | out |\n\
           n .\n\
           &out 0 >usize &> @ .\n\
           &out 1 >usize &> @ .\n\
           out drop ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("filter_over_array", &src);
    assert_eq!(stdout, "2\n8\n9\n");
    assert_eq!(code, 0);
}

// -- criterion 3: `filter` is element-polymorphic ----------------------------

#[test]
fn filter_is_element_polymorphic() {
    // Criterion 3 (R1): the same `filter` inlines over an `[f64 3]` array with
    // a float predicate, keeping the single element greater than `1.0`.
    let src = format!(
        "{}: arr ( -- [f64 3] )\n\
         0.0 3 fill | s |\n\
         &!s 0 >usize &!> 0.5 !\n\
         &!s 1 >usize &!> 2.5 !\n\
         &!s 2 >usize &!> 0.3 !\n\
         s ;\n\
         : main ( -- )\n\
           arr [ 1.0 > ] c::filter | n | | out |\n\
           n .\n\
           &out 0 >usize &> @ .\n\
           out drop ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("filter_f64", &src);
    assert_eq!(stdout, "1\n2.5\n");
    assert_eq!(code, 0);
}

// -- criteria 4-6: the D5 cycle relaxation admits only a self-tail edge -------

#[test]
fn self_tail_combinator_edge_is_allowed() {
    // Criterion 4 (R4/R5): `while`'s body names itself in tail position only,
    // so the D5-relaxed cycle check adds no self-edge and it checks standalone
    // (the loop transform makes the recursion finite). This is the shape 6a
    // rejected outright; a placebo would be a program that also passes with the
    // tail-only condition deleted, so the twin below
    // (`non_tail_combinator_self_call_is_still_a_cycle_error`) pins that
    // deleting it flips a *non-tail* program from reject to accept.
    check_ok(": while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;\n");
}

#[test]
fn non_tail_combinator_self_call_is_still_a_cycle_error() {
    // Criterion 5 (R4, load-bearing): a self-name in a *non-tail* position
    // keeps its self-edge and stays the cycle rejection, naming the word.
    // Deleting R4's tail-only condition would (wrongly) accept this, so the
    // pair 4/5 is the mutation-test guard on the relaxation's width.
    let err = check_error(": c ( i64 [ i64 -- i64 ] -- i64 ) c drop ;\n: main ( -- ) ;\n");
    assert!(
        err.contains("`c`") && err.contains("recursive"),
        "a non-tail self-recursive combinator is still a located cycle rejection naming it, got: {err}"
    );
}

#[test]
fn mutual_combinator_cycle_through_an_ambiguous_overloaded_name_is_still_caught() {
    // Slice 8a: `a` now carries two candidates sharing the name. A bare
    // callee name can't say which one `b`'s call to `a` reaches statically,
    // so this pass must not resolve it to a *single* index the way it did
    // pre-8a -- doing so could point at the wrong candidate (or, worse,
    // silently miss the real cycle through the other one, which would have
    // the inliner splice forever rather than merely mis-detect a runtime
    // optimization the way the tail-call cycle guard's analogous narrowing
    // does). `b`'s own operand types mean it really does resolve to the
    // `bool` candidate of `a`, closing a genuine cycle: a(bool) -> b -> a(bool).
    // Declared with the cycling candidate *first* and the unrelated one
    // last: a name-to-single-index map built by plain `.collect()` keeps
    // whichever candidate is declared last, so it would point `b`'s edge at
    // the i64 candidate (a leaf, no edge back to `b`) and miss the cycle
    // through the bool one entirely -- accepting a program that should be
    // rejected, rather than merely detecting the wrong optimization
    // opportunity the way the tail-call cycle guard's analogous narrowing
    // does.
    let err = check_error(
        ": a ( bool [ bool -- bool ] -- bool ) b ;\n\
         : a ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
         : b ( bool [ bool -- bool ] -- bool ) a ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("`a`") && err.contains("`b`") && err.contains("recursive"),
        "a mutual cycle through an overloaded combinator name should still be a located rejection, got: {err}"
    );
}

#[test]
fn two_poly_combinators_declaring_the_same_signature_is_a_duplicate_error() {
    // Deferred from round 3, the combinator half of the same gap fixed for
    // ordinary poly words: `is_combinator` doesn't discriminate poly vs.
    // mono, so a poly combinator sharing a name with an identically-signed
    // poly combinator has the same route to silently resolving to whichever
    // is declared first, forever.
    let err = check_error(
        ": apply ( 'T [ 'T -- 'T ] -- 'T ) call ;\n\
         : apply ( 'T [ 'T -- 'T ] -- 'T ) call call ;\n\
         : main ( -- ) 5 [ 2 * ] apply . ;\n",
    );
    assert!(
        err.contains("duplicate overload") && err.contains("apply"),
        "a second poly combinator declaring the same signature should be a duplicate error, got: {err}"
    );
}

#[test]
fn mutual_combinator_cycle_is_still_an_error() {
    // Criterion 6 (R4, load-bearing): a two-combinator mutual cycle is
    // untouched by the relaxation (which skips only a *self*-edge, i==j), so
    // even though both calls are in tail position the cycle stands, naming
    // both members.
    let err = check_error(
        ": a ( i64 [ i64 -- i64 ] -- i64 ) b ;\n\
         : b ( i64 [ i64 -- i64 ] -- i64 ) a ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("`a`") && err.contains("`b`") && err.contains("recursive"),
        "a mutual combinator cycle is still a located rejection naming both members, got: {err}"
    );
}

// -- criteria 7-9: `while` runs, carries an aggregate, and falls through ------

#[test]
fn while_runs_to_a_fixpoint() {
    // Criterion 7 (R10/R13): the canonical fixpoint. `while` threads the
    // counter through the predicate until it reaches 5, then leaves it.
    let src = format!(
        "{}: main ( -- ) 0 [ dup 5 < if 1 + true else false end ] c::while . ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("while_fixpoint", &src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

#[test]
fn while_carrying_an_aggregate_state_runs() {
    // Criterion 8 (R11): the carried state is a `Box` (an aggregate struct),
    // so the back-edge rides the `stage_aggregates` stable-slot path (the
    // slice-3 aggregate-return aliasing fix). The counter reaches 5.
    let src = format!(
        "{}type: Box n i64 ;\n\
         : main ( -- )\n\
           0 Box [ | b | b Box>n dup 5 < if 1 + Box true else Box false end ] c::while\n\
           | r | r Box>n . ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("while_aggregate", &src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

#[test]
fn while_empty_false_arm_falls_through() {
    // Criterion 9 (R12): `while`'s `else end` arm is empty and must fall
    // through leaving the state. A predicate that is false on the first call
    // exits immediately with the initial state (7) untouched, exercising the
    // fall-through arm directly.
    let src = format!(
        "{}: main ( -- ) 7 [ dup 5 < if 1 + true else false end ] c::while . ;\n",
        combinators_import("c")
    );
    let (stdout, code) = run_src("while_falls_through", &src);
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

// -- criteria 10-11: the two back-edge obligations, located at the self-call --

#[test]
fn while_body_linear_local_across_back_edge_is_error() {
    // Criterion 10 (R8), re-pointed by 10b's P0. The shape this used to use
    // (an outer linear parked across the loop and disposed on the next line)
    // now compiles: it was a false rejection, and its own justification --
    // "it would ride into the next iteration with nobody to dispose it" -- was
    // false about its own program. The golden moves to a self-tail combinator
    // whose *own* body binds a linear inside the tail `if` arm and reaches the
    // back-edge with it unconsumed, which is above the floor and still
    // rejected here.
    //
    // What this witnesses is where the rejection is *located*, not that a leak
    // is prevented: delete the combinator-site `check_linear_across_back_edge`
    // call and the program is still rejected, by end-of-scope disposal, losing
    // only the back-edge wording. The combinator is named `while` because the
    // message names the callee, which is what the assertion below reads.
    let src = format!(
        "{SPY_DEF}\
         : while ( i64 [ i64 -- i64 bool ] -- i64 )\n\
           | p | p call if 3 Spy | leak | p while else end ;\n\
         : main ( -- ) 0 [ dup 5 < if 1 + true else false end ] while . ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("`Spy`")
            && err.contains("`while`")
            && err.contains("live across the self-tail-call back-edge"),
        "a linear local live across `while`'s back-edge is located, naming `Spy` and `while`, got: {err}"
    );
}

#[test]
fn while_body_reference_across_back_edge_is_error() {
    // Criterion 11 (R9, load-bearing): the carried state is a reference `&v`
    // to a frame local `v`, whose storage does not survive to the next
    // iteration. Located at the self-call, naming the borrowed place and
    // `while`. Removing the `check_reference_across_back_edge` call from the
    // self-tail splice path would let this through.
    let src = format!(
        "{}type: V x i64 ;\n\
         : main ( -- )\n\
           0 V | v |\n\
           &v [ | r | r true ] c::while\n\
           drop\n\
           v drop ;\n",
        combinators_import("c")
    );
    let err = build_check_error("while_ref_back_edge", &src);
    assert!(
        err.contains("`v`")
            && err.contains("`while`")
            && err.contains("a reference to a local cannot cross a loop"),
        "a reference to a frame local carried across `while`'s back-edge is located, naming `v` and `while`, got: {err}"
    );
}

// -- 6d criteria 5 / 6: `while` and `times` nest, the limit lifted -----------

#[test]
fn while_inside_a_times_body_runs_to_fixpoint() {
    // 6d criterion 5: a `while` opened inside a `times` body used to be the
    // R14a nested-loop rejection; the hoist-target split lifts it, so the two
    // constant-stack loops now share one frame. Each of the 3 outer iterations
    // counts `0` up to `5` with the inner `while` and prints it.
    let src = format!(
        "{}: main ( -- )\n\
           3 [ | i | 0 [ dup 5 < if 1 + true else false end ] c::while . ] c::times ;\n",
        combinators_import("c")
    );
    let binary = build_binary("while_in_times", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out),
        (Some(0), "5\n5\n5".to_string()),
        "a `while` inside a `times` body runs to fixpoint each outer iteration"
    );
}

#[test]
fn times_inside_a_self_tail_combinator_body_runs() {
    // 6d criterion 6: a `times` sited inside `while`'s body (a self-tail
    // combinator splice) used to be the R14b nested-loop rejection; the split
    // lifts it. The inner `times` runs twice per `while` step (its `[ | i | ]`
    // body drops the index and leaves the row unchanged), and the `while`
    // counts `0` up to `5`.
    let src = format!(
        "{}: main ( -- )\n\
           0 [ 2 [ | i | ] c::times dup 5 < if 1 + true else false end ] c::while . ;\n",
        combinators_import("c")
    );
    let binary = build_binary("times_in_while", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out),
        (Some(0), "5".to_string()),
        "a `times` inside a self-tail combinator body runs and the `while` reaches its fixpoint"
    );
}

#[test]
fn while_inside_a_while_body_runs() {
    // 6d criterion 7 (and the recon-10 defect, criterion 8): a `while` nested
    // in a `while` used to be R14b's rejection -- and, worse, reported the
    // *bogus* "a `times` cannot be nested in a loop yet" for a program
    // containing no `times` at all. The rejection is retired, so this now
    // compiles and runs: the outer `while` counts `0` up to `3`, the inner
    // `while` runs to its own fixpoint each step but drops its result.
    let src = format!(
        "{}: main ( -- )\n\
           0 [ dup 3 < if 0 [ dup 2 < if 1 + true else false end ] c::while drop\n\
                        1 + true else false end ] c::while . ;\n",
        combinators_import("c")
    );
    let binary = build_binary("while_in_while", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out),
        (Some(0), "3".to_string()),
        "a `while` nested in a `while` compiles and runs, no longer the bogus `times` rejection"
    );
}

// -- 6d criteria 1-4 / 9c: `times` nests, and the hoist-target split holds ---

#[test]
fn times_nested_in_a_times_runs_with_correct_output() {
    // 6d criterion 1: a `times` nested in a `times` used to be the R18
    // rejection; the split lifts it. The outer counts 3, the inner counts 2
    // and adds 1 each inner iteration, so the accumulator is 3*2 = 6.
    let (out, code) = run_src(
        "times_in_times",
        &format!(
            "{}: main ( -- ) 0 3 [ | i | 2 [ | j | 1 + ] times ] times . ;\n",
            combinators_import("c | times |")
        ),
    );
    assert_eq!((out.as_str(), code), ("6\n", 0));
}

#[test]
fn times_in_times_with_inner_allocation_runs() {
    // 6d criterion 2: the inner body allocates (`0 4 fill`) every inner
    // iteration. The split routes that `Alloc` to the invariant alloca home
    // (reached once per call), so the value is still 3*2 = 6 and the frame
    // does not grow; the constant-stack side is pinned by criterion 9c below.
    let (out, code) = run_src(
        "times_in_times_alloc",
        &format!(
            "{}: main ( -- ) 0 3 [ | i | 2 [ | j | 0 4 fill | a | a drop 1 + ] times ] times . ;\n",
            combinators_import("c | times |")
        ),
    );
    assert_eq!((out.as_str(), code), ("6\n", 0));
}

#[test]
fn reentered_inner_accumulator_reseeds_per_outer_iteration() {
    // 6d criterion 3 (load-bearing, mutation-test-required): recon 5's probe.
    // The inner loop carries an aggregate accumulator seeded from a fresh
    // `0 4 fill` each outer iteration, incremented 3 times, then read back.
    // Because the seeding `Blit` stays in *this* loop's preheader (R3), not
    // the alloca home, it re-seeds per outer entry: both outer iterations
    // print `3`, giving `3\n3`. Hoisting the blit into the alloca home (R3
    // reversed) seeds once per call, so the second outer iteration reads a
    // stale `6` (or garbage) -- either way not `3\n3`. This is the slice-3
    // aliasing class of bug and the single highest-risk regression.
    let (out, code) = run_src(
        "reseed_probe",
        &format!(
            "{}: main ( -- )\n\
               2 [ drop 0 4 fill 3 [ drop | a | &!a 0 >usize &!> 1 +! a ] times\n\
                   | b | &b 0 >usize &> @ . b drop ] times ;\n",
            combinators_import("c | times |")
        ),
    );
    assert_eq!((out.as_str(), code), ("3\n3\n", 0));
}

#[test]
fn three_deep_times_nesting_runs_in_constant_stack() {
    // 6d criterion 4 (Q3): a three-deep `times`-in-`times`-in-`times`, the
    // innermost body allocating each iteration, with a large outermost count.
    // It computes 50_000 * 2 * 2 = 200_000 and runs to completion under a
    // constrained `ulimit -s`, so arbitrary depth falls out of the
    // per-function alloca home plus the recursively-nesting preheader
    // save/restore.
    let src = format!(
        "{}: main ( -- )\n\
         0 50000 [ | i | 2 [ | j | 2 [ | k | 0 8 fill | a | a drop 1 + ] times ] times ] times . ;\n",
        combinators_import("c | times |")
    );
    let binary = build_binary("three_deep", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out),
        (Some(0), "200000".to_string()),
        "a three-deep nesting runs to completion with correct output in constant stack"
    );
}

#[test]
fn nested_times_large_outer_holds_constant_stack() {
    // 6d criterion 9c (load-bearing, mutation-test-required, D5): the frame
    // grows as `outer_iterations * hoisted_bytes`, so the witness needs a
    // *large outer* count and a small inner one, allocating per inner
    // iteration. 200_000 outer * (2 inner * `0 32 fill`) segfaults on the
    // default 8 MB stack with the alloca home reverted to `entry_block`
    // (the hoist lands in the per-outer-iteration preheader); with the split
    // it runs to completion (exit 0, prints 99) even at `ulimit -s 1024`. A
    // large-inner / small-outer shape is explicitly NOT the witness: it
    // passes while the bug is live (recon 4).
    let src = format!(
        "{}: main ( -- ) 200000 [ drop 2 [ drop 0 32 fill | a | a drop ] times ] times 99 . ;\n",
        combinators_import("c | times |")
    );
    let binary = build_binary("nested_big_outer", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out),
        (Some(0), "99".to_string()),
        "a large-outer nested loop allocating per inner iteration runs in constant stack (would SIGSEGV, exit None, with the alloca home reverted to entry_block)"
    );
}

#[test]
fn destructor_call_inside_a_times_body_holds_constant_stack() {
    // 6d criterion 10 (Q4, phase 3): a recursive-enum value is constructed and
    // dropped inside a `times` body, 200_000 times, each round building and
    // freeing a 5-node list (1,000,000 nodes total, the same total work as
    // `tests/phase0.rs`'s `deep_list_disposes_in_constant_stack`, split across
    // rounds). The destructor's fused loop opens at its own `IrFunc`'s true
    // entry, so its preheader and alloca home already coincide exactly as the
    // top-level case does (Q4: it inherits D2 for free) -- a destructor
    // *called* from inside a user loop runs in a fresh per-call frame freed on
    // return, never the nesting case R1-R3 fix. This pins that inheritance
    // rather than re-testing R3 itself (criteria 3 and 9c already do that).
    let src = format!(
        "{}type: List | Nil | Cons v i64 next ^List ;\n\
         : build ( i64 List -- List )\n  \
           | n acc |\n  \
           n 0 = if\n    \
             acc\n  \
           else\n    \
             n 1 - n acc ^ Cons build\n  \
           end ;\n\
         : main ( -- ) 200000 [ drop 5 Nil build drop ] times ;\n",
        combinators_import("c | times |")
    );
    let binary = build_binary("destructor_in_times", &src);
    let (code, out) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        (code, out.as_str()),
        (Some(0), ""),
        "a recursive-enum value built and dropped inside a `times` body runs in constant stack"
    );
}

// -- criterion 13: `while` is constant-stack, agreeing with a hand twin -------

#[test]
fn while_and_hand_threaded_loop_agree_across_stack_limits() {
    // Criterion 13 (R15): `while` lowers to a constant-stack loop, so it and
    // its hand-threaded whole-word self-tail twin (`countup`) behave
    // identically across a `ulimit -s` sweep -- neither grows the stack per
    // iteration, so both complete at every limit with the same output. N is
    // 10_000 to keep the build fast; the *structural* guarantee (loop header +
    // back-edge, no per-iteration `Call`) is carried by the
    // `while_lowers_to_a_back_edge_not_an_infinite_splice` unit.
    const N: usize = 10_000;
    let comb = format!(
        "{}: main ( -- ) 0 [ dup {N} < if 1 + true else false end ] c::while . ;\n",
        combinators_import("c")
    );
    let hand = format!(
        ": countup ( i64 -- i64 ) dup {N} < if 1 + countup else end ;\n\
         : main ( -- ) 0 countup . ;\n"
    );
    let comb_bin = build_binary("wq-comb", &comb);
    let hand_bin = build_binary("wq-hand", &hand);

    for limit in [64u32, 256, 1024] {
        assert_eq!(
            run_at_stack_limit(&comb_bin, limit),
            run_at_stack_limit(&hand_bin, limit),
            "`while` and its hand-threaded twin must behave identically at ulimit -s {limit}"
        );
    }
    // At a generous limit the combinator version runs to completion and counts
    // to N, so the equivalence above cannot pass by both sides being equally
    // wrong.
    assert_eq!(
        run_at_stack_limit(&comb_bin, 1024),
        (Some(0), N.to_string()),
        "at a generous stack limit `while` runs to completion and counts to N"
    );

    std::fs::remove_file(&comb_bin).ok();
    std::fs::remove_file(&hand_bin).ok();
}

// -- criterion 12 / 12b: obligations 1 and 2 discharged at the def site -------

#[test]
fn poly_combinator_consuming_local_is_error() {
    // Criterion 12 (R16, load-bearing): "The hardest question" discharges
    // obligation 1 (move-state identity) at the *def site*. A poly combinator
    // whose `times` body consumes an outer linear local (`s`, a `Spy`) is
    // located there -- the body runs N times, so the linear value would be
    // disposed N times -- with no call site involved. Pins *where* the check
    // lives (def site, not splice site).
    let src = format!(
        "{TIMES_DEF}{SPY_DEF}\
         : bad ( ['T 'N] Spy [ 'T -- ] -- )\n\
         | f | | s | len >i64 | count | | arr |\n\
         count [ | i | &arr i >usize &> @ f call s drop ] times\n\
         arr drop ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("`bad`")
            && err.contains("consumes the enclosing local `s`, which is linear"),
        "consuming a linear local in a poly `times` body should be located at the def site naming `bad` and `s`, got: {err}"
    );
}

#[test]
fn poly_combinator_borrow_across_loop_is_error() {
    // Criterion 12b (R16, load-bearing): obligation 2 (borrow-state identity)
    // for *captured* state is discharged at the def site. A poly combinator
    // whose `times` body leaves a reference to an outer local (`v`) live on the
    // row rides the back-edge into the next iteration, and is located at the
    // def site.
    let src = format!(
        "{TIMES_DEF}type: V x i64 ;\n\
         : bad ( ['T 'N] V [ 'T -- ] -- )\n\
         | f | | v | len >i64 | count | | arr |\n\
         count [ | i | &arr i >usize &> @ f call &v ] times\n\
         arr drop v drop ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("`bad`") && err.contains("borrows the enclosing place `v`"),
        "a borrow crossing the back-edge in a poly `times` body should be located at the def site naming `bad`, got: {err}"
    );
}

#[test]
fn poly_combinator_literal_borrowing_enclosing_place_is_error() {
    // Item 3: R12's D3 borrow check must run on the *polymorphic* argument
    // path, not just the monomorphic one. Minimal pair: identical body,
    // identical caller literal, only `applyr`'s quotation-parameter type
    // differs (`[ i64 -- &i64 ]` vs `[ 'T -- &i64 ]`). Before the fix the poly
    // twin's inline path read `word.effect.inputs` (empty for a poly word),
    // ran zero argument checks, and accepted a literal that borrows the
    // enclosing place `b` -- a silent mono/poly divergence in the premise D3
    // rests on. Both must now be the same located R12 rejection.
    let mono = "type: Box v i64 ;\n\
                : applyr ( i64 [ i64 -- &i64 ] -- )\n\
                  | f | | v | v f call drop ;\n\
                : main ( -- )\n\
                  7 Box | b |\n\
                  0 [ | x | x drop &b &Box>v ] applyr\n\
                  b drop ;\n";
    let poly = "type: Box v i64 ;\n\
                : applyr ( 'T [ 'T -- &i64 ] -- )\n\
                  | f | | v | v f call drop ;\n\
                : main ( -- )\n\
                  7 Box | b |\n\
                  0 [ | x | x drop &b &Box>v ] applyr\n\
                  b drop ;\n";
    for (label, src) in [("mono", mono), ("poly", poly)] {
        let err = check_error(src);
        assert!(
            err.contains("`applyr`")
                && err.contains("borrows the enclosing place `b`")
                && err.contains("(D3)"),
            "the {label} path must reject the borrowing literal with R12's D3 message naming `applyr` and `b`, got: {err}"
        );
    }
}

#[test]
fn literal_created_borrow_across_loop_is_error_at_splice_site() {
    // Criterion 12c (item 4): the spec's "Accepted narrowings" claimed 12c was
    // unreachable because a combinator whose quotation parameter has a
    // reference *output* row is rejected at its own def site. Both halves are
    // false. `refout` (a `[ 'T -- &i64 ]` parameter, a reference output row)
    // compiles clean standalone, and before item 3 a caller literal that
    // creates a borrow of a captured enclosing local and leaves the `&i64` on
    // its output row -- exactly 12c's scenario, inside a `times` loop -- was
    // accepted and ran, printing `7 7 7 7`.
    //
    // After item 3 runs R12 on the poly argument path it is a located
    // rejection, and the diagnostic that fires is R12's borrow-left-on-row
    // check (`quotation_borrows_place_error`) at the splice site (main's
    // call), *not* the `times` back-edge check: the literal's declared effect
    // `[ i64 -- &i64 ]` leaves an `&i64` borrowing the enclosing `b` on its
    // exit row, which R12 rejects before the body is ever spliced into the
    // loop. So obligation 2's literal-created-borrow half is discharged at the
    // argument site, naming `refout` and `b`.
    let src = format!(
        "{TIMES_DEF}type: Box v i64 ;\n\
         : refout ( ['T 4] [ 'T -- &i64 ] -- )\n\
         | f | | arr |\n\
         4 [ | i | &arr i >usize &> @ f call drop ] times\n\
         arr drop ;\n\
         : main ( -- )\n\
         7 Box | b |\n\
         0 4 fill [ | x | &b &Box>v ] refout\n\
         b drop ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("`refout`")
            && err.contains("borrows the enclosing place `b`")
            && err.contains("(D3)"),
        "12c is reachable: the borrow-creating literal must be a located R12 rejection naming `refout` and `b`, got: {err}"
    );
}

// -- criterion 13: a quotation at a runtime-value position in a poly body -----

#[test]
fn quotation_at_runtime_position_in_poly_body_is_error() {
    // Criterion 13 (R14): the poly `quot` marker lets a combinator's body track
    // a quotation literal to its consumption, but a quotation reaching a
    // *runtime-value* position (here a `fill` store) in that poly body is still
    // rejected, reworded to name slice 7 (runtime quotation values). The
    // `!not yet supported` assertion pins that the R14 marker path ran (the
    // word carries a quotation parameter, so the body is checked on the
    // `Slot`-with-`quot` path), not slice 4's blanket `poly_term` rejection.
    let err = check_error(
        ": bad ( ['T 'N] [ 'T -- ] -- ['T 'N] )\n\
         | f arr | [ 1 + ] 4 fill drop arr ;\n",
    );
    assert!(
        err.contains("`bad`")
            && err.contains("a quotation cannot be stored")
            && err.contains("slice 7"),
        "a quotation at a store position in a poly body should be the reworded slice-7 rejection, got: {err}"
    );
    assert!(
        !err.contains("Phase 6") && !err.contains("not yet supported"),
        "it must be the R14 marker-path rejection naming slice 7, not the stale wording, got: {err}"
    );
}

// -- criterion 14: combinator == hand-threaded across stack limits ------------

/// Build `src` to a native binary and return its path (the caller runs it and
/// removes it). Split from the run so criterion 14 compiles each program once
/// and runs it at several stack limits.
fn build_binary(name: &str, src: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    std::fs::remove_file(&path).ok();
    binary
}

/// Run `binary` under `ulimit -s {limit_kb}` (KB), returning the exit code, or
/// `None` if it died by signal (a `SIGSEGV` from an overflowed stack).
// `exec` replaces the `sh`, so a signal death is reported as the *binary's*
// signal and `code()` is `None`; without the `exec` the shell would survive and
// report 128+signo instead.
fn run_at_stack_limit(binary: &std::path::Path, limit_kb: u32) -> (Option<i32>, String) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -s {limit_kb} && exec \"{}\"",
            binary.display()
        ))
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

#[test]
fn combinator_and_hand_threaded_loops_agree_across_stack_limits() {
    // Criterion 14 (respecified, item 7). The original "1M+ elements under a
    // reduced `ulimit`" witness is infeasible and mis-specified: a Sooth array
    // is a *stack* value (1M i64 = 8 MB on the stack before any loop frame
    // exists), and `fill`'s compile cost is superlinear and pre-existing
    // (measured: 10k ~= 0.36 s, 100k ~= 25 s, 1M > 300 s), not caused by the
    // inliner (a hand-threaded `times` twin is equally slow). So this is an
    // *equivalence* witness instead: the combinator version and the
    // hand-threaded `times` twin behave identically across stack limits, which
    // is what "the inliner adds no stack cost" means. N is 10_000 to keep
    // `cargo test` fast (~0.36 s to compile each); the *structural* constant-
    // stack guarantee (loop header + back-edge, no per-element `Call`) is
    // carried by the `each_lowers_to_a_loop_not_a_per_element_call` unit.
    // The array is 1-filled, not 0-filled: the fold's correct answer is then N,
    // so a combinator that computes the wrong sum is caught. A 0-filled array
    // makes the expected value 0, which any broken fold returning garbage-free
    // zero would also produce.
    const N: usize = 10_000;
    let comb = format!(
        "{}: main ( -- ) 1 {N} fill 0 [ + ] c::fold . ;\n",
        combinators_import("c")
    );
    let hand = format!(
        "{}: main ( -- ) 1 {N} fill | arr | 0 {N} [ | i | &arr i >usize &> @ + ] times . arr drop ;\n",
        combinators_import("c | times |")
    );
    let comb_bin = build_binary("eq-comb", &comb);
    let hand_bin = build_binary("eq-hand", &hand);

    // Across a sweep straddling the array-doesn't-fit boundary, the two must
    // cross together: identical exit codes at each limit (a tight limit both
    // die by signal -> `None`; a generous limit both exit 0). Equality holds
    // wherever the boundary sits, so this is robust to the exact machine.
    for limit in [64u32, 256, 1024] {
        assert_eq!(
            run_at_stack_limit(&comb_bin, limit),
            run_at_stack_limit(&hand_bin, limit),
            "combinator and hand-threaded twin must behave identically at ulimit -s {limit}"
        );
    }
    // And at a generous limit the combinator version runs to completion *and
    // computes the right sum*, so the equivalence above cannot pass by both
    // sides being equally wrong.
    assert_eq!(
        run_at_stack_limit(&comb_bin, 1024),
        (Some(0), N.to_string()),
        "at a generous stack limit the combinator version runs to completion and sums to N"
    );

    std::fs::remove_file(&comb_bin).ok();
    std::fs::remove_file(&hand_bin).ok();
}

// -- slice 6c (phase 1): quotation-taking words retained at a session line ---

/// Run a scripted REPL session in-process, returning the whole stdout
/// transcript (`defined …`, printed values, each residual `stack:` line, and
/// any error line). Goldens pin the *exact* transcript rather than a
/// `contains`, since the REPL echoes the whole residual stack after every line
/// and a `contains` would let a placebo pass (the whole-stack echo hazard).
fn repl_error(input: &str) -> String {
    let reader = BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sooth::repl::run(reader, &mut out).expect("the REPL loop itself should not error");
    String::from_utf8(out).expect("REPL output should be utf8")
}

// A polymorphic self-tail `while` and a two-output `filter`, reused across the
// 6c REPL goldens. Their bodies name only builtins, their quotation parameter,
// and (for `filter`, since 10b retired the intrinsic) a session-defined
// `times`, so a session define exercises the splice, not a library import.
const WHILE_DEF: &str =
    ": while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call if p while else end ;\n";
const FILTER_DEF: &str = ": filter ( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize ) | p | len >i64 | n | | arr | 0 n [ | i | &arr i >usize &> @ dup p call if | v | &!arr over >usize &!> v ! 1 + else drop end ] times | wf | arr wf >usize ;\n";

// A REPL expr line's residual stack is what the in-process driver writes to the
// capture buffer; the runtime `.` word prints to the real process stdout, which
// this buffer does not see. So every 6c golden leaves its result *on the stack*
// (no `.`) and pins the exact `stack:` line. The persistent stack accumulates
// across lines, so a second call's `stack:` line shows both results.

#[test]
fn repl_quotation_taking_definition_is_accepted() {
    // R19: the former R23 rejection is now acceptance. A monomorphic
    // quotation-taking word defines at a session line and a *later* bare line
    // calls it, inlined against that line's live env (D1): `5 [ 3 + ] apply`
    // leaves 8. The guarded behavior flips (define-and-call), not vanishes.
    let transcript =
        repl_error(": apply ( i64 [ i64 -- i64 ] -- i64 ) call ;\n5 [ 3 + ] apply\n:quit\n");
    assert_eq!(transcript, "defined apply\nstack: 8\n");
    assert!(
        !transcript.contains("not yet supported at the REPL"),
        "the former R23 rejection must be gone: {transcript}"
    );
}

#[test]
fn repl_poly_quotation_taking_definition_is_accepted() {
    // R19: the former poly rejection (the `eval_poly_def` path) is now
    // acceptance. A *polymorphic* combinator (`apply1 ( 'a [ 'a -- 'a ] -- 'a )`)
    // defines and a later line calls it, leaving 6 -- a value witness, unlike an
    // `each`-shaped combinator that would leave the stack empty.
    let transcript =
        repl_error(": apply1 ( 'a [ 'a -- 'a ] -- 'a ) call ;\n5 [ 1 + ] apply1\n:quit\n");
    assert_eq!(transcript, "defined apply1\nstack: 6\n");
    assert!(
        !transcript.contains("not yet supported at the REPL"),
        "the former poly rejection must be gone: {transcript}"
    );
}

#[test]
fn repl_self_tail_combinator_definition_is_accepted() {
    // R19: the former self-tail rejection is now acceptance. `while` defines at
    // a session line (the self-tail edge is permitted, 6b D5) rather than being
    // rejected on its declared quotation parameter.
    let transcript = repl_error(&format!("{WHILE_DEF}:quit\n"));
    assert_eq!(transcript, "defined while\n");
    assert!(
        !transcript.contains("declares a quotation parameter"),
        "the former self-tail rejection must be gone: {transcript}"
    );
}

#[test]
fn repl_mono_combinator_define_and_call() {
    // Criterion 1: a monomorphic combinator defined at one session line, called
    // from a *later* bare line, inlines and runs. `on_double` applies the
    // quotation then doubles: `(5+1)*2 = 12`.
    let transcript = repl_error(
        ": on_double ( i64 [ i64 -- i64 ] -- i64 ) call 2 * ;\n5 [ 1 + ] on_double\n:quit\n",
    );
    assert_eq!(transcript, "defined on_double\nstack: 12\n");
}

#[test]
fn repl_while_define_runs_to_fixpoint() {
    // Criterion 2 (mutation-pins R5's `lower_line` combinator threading): `while`
    // defined at a session line, then `0 [ dup 5 < if 1 + true else false end ]
    // while` runs to a fixpoint of 5, lowering to a loop back-edge (constant
    // stack), not an infinite splice or a link failure to a never-minted symbol.
    let transcript = repl_error(&format!(
        "{WHILE_DEF}0 [ dup 5 < if 1 + true else false end ] while\n:quit\n"
    ));
    assert_eq!(transcript, "defined while\nstack: 5\n");
}

#[test]
fn repl_two_output_combinator_define_and_call() {
    // Criterion 3 (mutation-pins R9's poly-combinator routing): a two-output
    // poly combinator (`filter` shape) defines and runs; both outputs land on
    // the residual stack (the compacted array and the kept-count). `7 3 fill`
    // is three 7s; `[ 5 > ]` keeps all three, so the residual is the array then
    // `3`. If R9 routed `filter` through `eval_poly_def`, its two outputs would
    // be wrongly deferred as "resolves to 2 outputs".
    let transcript = repl_error(&format!(
        "{TIMES_DEF}{FILTER_DEF}7 3 fill [ 5 > ] filter\n:quit\n"
    ));
    assert_eq!(
        transcript,
        "defined times-helper\ndefined times\ndefined filter\nstack: <[i64 3]> 3\n"
    );
}

#[test]
fn repl_combinator_splice_sees_current_helper() {
    // Criterion 4 (D1 falsifiable): a combinator whose body calls an ordinary
    // helper is called (106 = 5+1+100), the helper is redefined (+200), and the
    // combinator is called again from a *new* line: the new line's splice sees
    // the *new* helper (206 = 5+1+200). This fails if any frozen-resolver/env
    // capture is added for a combinator -- the new line would still see +100.
    let transcript = repl_error(
        ": helper ( i64 -- i64 ) 100 + ;\n\
         : useh ( i64 [ i64 -- i64 ] -- i64 ) call helper ;\n\
         5 [ 1 + ] useh\n\
         : helper ( i64 -- i64 ) 200 + ;\n\
         5 [ 1 + ] useh\n:quit\n",
    );
    // The stack accumulates: the first call leaves 106, the redefinition of
    // `helper` follows, the second call leaves 206 on top. A frozen capture
    // would make the top 106 again (`stack: 106 106`); the `206` is the pin.
    assert_eq!(
        transcript,
        "defined helper\ndefined useh\nstack: 106\ndefined helper\nstack: 106 206\n"
    );
}

#[test]
fn repl_ordinary_caller_frozen_across_combinator_redefinition() {
    // Criterion 5 (R20 frozen `.so`): an ordinary word `w` compiled with the
    // combinator `c` spliced into it keeps its baked result (51 = 5*10+1) across
    // a later redefinition of `c` (+1000 instead of +1). `w`'s `.so` is frozen;
    // only a *new* splice site would see the new `c`.
    let transcript = repl_error(
        ": c ( i64 [ i64 -- i64 ] -- i64 ) call 1 + ;\n\
         : w ( i64 -- i64 ) [ 10 * ] c ;\n\
         5 w\n\
         : c ( i64 [ i64 -- i64 ] -- i64 ) call 1000 + ;\n\
         5 w\n:quit\n",
    );
    // Both calls leave 51: `w`'s `.so` is frozen with the original `c` spliced,
    // so redefining `c` (+1000) changes nothing (`stack: 51 51`, not `51 1051`).
    assert_eq!(
        transcript,
        "defined c\ndefined w\nstack: 51\ndefined c\nstack: 51 51\n"
    );
}

#[test]
fn repl_redefining_combinator_shape_evicts_other_stores() {
    // Criterion 6 (mutation-pins R11's `self.combinators.remove`/`self.env`
    // eviction): redefining `foo` from combinator to ordinary word and back
    // rebinds dispatch to the new shape each time (D4). Combinator dispatch
    // runs first, so a stale entry in the wrong store would silently keep
    // winning: `6 = 5+1` (combinator), then `104 = 5+99` (ordinary, the
    // combinator entry evicted), then `11 = 5*2+1` (combinator again, the
    // ordinary entry evicted).
    let transcript = repl_error(
        ": foo ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
         5 [ 1 + ] foo\n\
         : foo ( i64 -- i64 ) 99 + ;\n\
         5 foo\n\
         : foo ( i64 [ i64 -- i64 ] -- i64 ) call 2 * ;\n\
         5 [ 1 + ] foo\n:quit\n",
    );
    // The stack accumulates across the three shapes: `6` (combinator, 5+1),
    // then `104` (ordinary, 5+99 -- proving the combinator entry was evicted,
    // else `5 foo` with no quotation would type-error), then `12`
    // (combinator again, (5+1)*2 -- proving the ordinary entry was evicted,
    // else `5 [ 1 + ] foo` would be "a quotation cannot be passed to foo").
    assert_eq!(
        transcript,
        "defined foo\nstack: 6\ndefined foo\nstack: 6 104\ndefined foo\nstack: 6 104 12\n"
    );
}

#[test]
fn repl_cross_line_combinator_cycle_is_error() {
    // Criterion 7 (mutation-pins R8's `check_combinator_cycles` at the REPL): a
    // cycle formed *across* session lines (define `a`; define `b` calling `a`;
    // redefine `a` calling `b`) is the same located `combinator_cycle_error`.
    // Without the check the cycle type-checks and then splices forever.
    let transcript = repl_error(
        ": a ( [ -- ] -- ) call ;\n\
         : b ( [ -- ] -- ) call [ ] a ;\n\
         : a ( [ -- ] -- ) call [ ] b ;\n:quit\n",
    );
    // The cycle chain names both words; its starting node depends on hash-map
    // iteration order, so the direction is not pinned.
    assert!(
        transcript.contains("an always-spliced word cannot be recursive")
            && (transcript.contains("`a` -> `b` -> `a`")
                || transcript.contains("`b` -> `a` -> `b`")),
        "the cross-line cycle is the located cycle error: {transcript}"
    );
    // The first two defines succeeded; only the cycle-closing redefinition is
    // rejected, leaving the earlier lines intact.
    assert!(
        transcript.starts_with("defined a\ndefined b\n"),
        "the non-cyclic prefix defines cleanly: {transcript}"
    );
}

#[test]
fn repl_poly_word_calling_a_builtin_named_overload_does_not_segfault() {
    // Round 3 (slice 8a): the REPL analogue of `overload_from_poly_body_
    // dispatches_to_user_word` (tests/phase0.rs). `eval_poly_def`/
    // `lower_instantiation` froze an *empty* overloads map onto every
    // REPL-defined poly word, explicitly marked out of scope for this slice
    // at the time -- so `vsum`'s body called `+` on two `Vec2` operands, the
    // checker correctly resolved that to the user overload, but lowering
    // fell into the builtin numeric `Instr::Bin(Add)` arm on the two struct
    // pointers regardless, segfaulting the whole session (`run`/`build`
    // threaded the real record from the start and never had this bug).
    // Fixed by freezing the body's already-computed resolved-overload
    // record into `PolyWordEntry` alongside `resolver`/`ir_lower_env`.
    //
    // Mutation-tested: reverting `PolyWordEntry`'s `builtin_overloads` field
    // back to a fresh empty map at instantiation time passes the entire
    // existing suite (nothing else exercises this path) and only crashes
    // (SIGSEGV) when this exact session actually runs -- the gap this test
    // closes.
    // 'T is a plain passthrough (dropped last, via `swap drop`, so this
    // still instantiates via `eval_poly_def`/`lower_instantiation` rather
    // than the monomorphic path): the value under test is the *computed*
    // sum, deliberately not just "did this crash" -- a wrong dispatch here
    // is undefined-behaviour pointer arithmetic on aggregate operands,
    // which is not guaranteed to fault every run; asserting the exact
    // arithmetic result (which a wrong dispatch has no real chance of
    // reproducing by accident) is the reliable discriminator, and it still
    // reliably segfaults when mutated back to an empty map.
    let transcript = repl_error(
        "type: Vec2 x i64 y i64 ;\n\
         : + ( Vec2 Vec2 -- Vec2 ) | a b | a Vec2>x b Vec2>x + a Vec2>y b Vec2>y + Vec2 ;\n\
         : vsum ( 'T Vec2 Vec2 -- i64 ) + Vec2> + swap drop ;\n\
         42 1 2 Vec2 3 4 Vec2 vsum\n",
    );
    assert_eq!(
        transcript, "defined type Vec2\ndefined +\ndefined vsum\nstack: 10\n",
        "vsum should compute (1+3)+(2+4)=10 through the user overload, not crash: {transcript}"
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
        let binary = common::build_example(path);
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

// -- criterion 16 (phase 3): the filter/while dogfood ------------------------

#[test]
fn filter_while_dogfood_matches_hand_threaded() {
    // R18: `examples/filter_while.sth` keeps `scores`' elements greater than
    // 4 and counts them via `filter`, then runs a fixpoint loop via `while`,
    // both from `lib/combinators.sth`. This asserts it builds and runs to the
    // same output as its hand-threaded twin, `examples/filter_while_hand.sth`:
    // a manual `times` loop threading a write cursor for the filter, and a
    // hand-written self-tail-recursive word for the fixpoint. `scores`'
    // result is passed straight from the producer word into `filter`, never
    // bound to a local first, so this does not trip 6a's bind-then-pass alias
    // limitation (recon 10).
    fn build_and_run(path: &str) -> (String, Option<i32>) {
        let binary = common::build_example(path);
        let output = std::process::Command::new(&binary)
            .output()
            .expect("binary should run");
        std::fs::remove_file(&binary).ok();
        (
            String::from_utf8(output.stdout).expect("stdout should be utf8"),
            output.status.code(),
        )
    }

    let (hand_stdout, hand_code) = build_and_run("examples/filter_while_hand.sth");
    assert_eq!(hand_code, Some(0));
    assert_eq!(hand_stdout, "3\n5\n");

    let (combinator_stdout, combinator_code) = build_and_run("examples/filter_while.sth");
    assert_eq!(combinator_stdout, hand_stdout);
    assert_eq!(combinator_code, Some(0));
}

// -- criterion 9 (phase 3): the combinator-in-`times` dogfood ----------------

#[test]
fn combinator_in_times_dogfood_matches_hand_threaded() {
    // R9: `examples/combinator_in_times.sth` runs `each` from
    // `lib/combinators.sth` inside an outer `times` body, ROADMAP's own
    // motivating shape for 6d (`2 [ | i | mk [ . ] c::each ] times`), which
    // R18 rejected before this slice lifted the limit. This asserts it
    // builds and runs to the same output as its hand-threaded twin,
    // `examples/combinator_in_times_hand.sth`: an outer `times` over 3 rounds,
    // each building a 3-element array and printing its elements through a
    // manual inner `times` loop with the same `&arr i &> @` read, the exact
    // shape `each`'s internal loop takes once spliced.
    fn build_and_run(path: &str) -> (String, Option<i32>) {
        let binary = common::build_example(path);
        let output = std::process::Command::new(&binary)
            .output()
            .expect("binary should run");
        std::fs::remove_file(&binary).ok();
        (
            String::from_utf8(output.stdout).expect("stdout should be utf8"),
            output.status.code(),
        )
    }

    let (hand_stdout, hand_code) = build_and_run("examples/combinator_in_times_hand.sth");
    assert_eq!(hand_code, Some(0));
    assert_eq!(hand_stdout, "0\n1\n2\n1\n2\n3\n2\n3\n4\n");

    let (combinator_stdout, combinator_code) = build_and_run("examples/combinator_in_times.sth");
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
fn repl_import_exporting_combinator_retains_and_runs() {
    // R12/R13 (slice 6c): the former R24 rejection is gone. Importing a closure
    // that *exports* a quotation-taking word now retains the combinator (D5),
    // and a *later* session line calls it, inlined at that site's own live env.
    // `5 [ 3 + ] c::apply_each` leaves 8. The mono combinator `apply_each` is
    // skipped by the exported-ordinary-word loop (it mints no symbol, R20) and
    // retained by the combinator loop instead.
    let path = temp_lib(
        "crit8-exports",
        "export: apply_each ;\n: apply_each ( i64 [ i64 -- i64 ] -- i64 ) call ;\n",
    );
    let transcript = repl_error(&format!(
        "import: c \"{}\" ;\n5 [ 3 + ] c::apply_each\n:quit\n",
        path.display()
    ));
    std::fs::remove_file(&path).ok();
    assert_eq!(transcript, "imported c\nstack: 8\n");

    // A quotation-taking word used purely *internally* to an imported closure
    // (not exported) still imports and runs fine: it inlines during the
    // closure's own native compilation.
    let internal = temp_lib(
        "crit8-internal",
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

#[test]
fn repl_imported_combinator_body_call_to_private_word_uses_closure_env() {
    // Review fix (slice 6c): an exported combinator's body call to a
    // closure-*private* word must resolve against the closure's own private
    // definition, never against a same-named word the session happens to
    // define. The session defines `priv_calc6c` (+1000) *before* importing a
    // closure whose private `priv_calc6c` is +1 and whose exported `apply2`
    // calls it: `5 [ 10 * ] c::apply2` is `(5 * 10) + 1 = 51` if the
    // closure's own `priv_calc6c` wins, or `1050` if the retained body was
    // left unrewritten and fell through to the session's `priv_calc6c` -- the
    // hygiene break this test pins shut. (Named distinctly from the plainer
    // `helper` other goldens in this file use for their own session-defined
    // word, so this test's generation-0 `.so` symbol can't collide with
    // theirs when `cargo test` runs this file's tests concurrently under one
    // process's shared `dlopen(RTLD_GLOBAL)` namespace.)
    let path = temp_lib(
        "private-body-call",
        ": priv_calc6c ( i64 -- i64 ) 1 + ;\n\
         : apply2 ( i64 [ i64 -- i64 ] -- i64 ) | q | q call priv_calc6c ;\n\
         export: apply2 ;\n",
    );
    let transcript = repl_error(&format!(
        ": priv_calc6c ( i64 -- i64 ) 1000 + ;\nimport: c \"{}\" ;\n5 [ 10 * ] c::apply2\n:quit\n",
        path.display()
    ));
    std::fs::remove_file(&path).ok();
    assert_eq!(transcript, "defined priv_calc6c\nimported c\nstack: 51\n");
}

#[test]
fn repl_imported_combinator_body_call_to_private_word_without_collision_resolves() {
    // FIX D (slice 6c, second review pass): the no-collision sibling of
    // `repl_imported_combinator_body_call_to_private_word_uses_closure_env`
    // above -- same shape, but the session never defines `priv_calc6c_nc`
    // itself, so there is nothing to fall through to even if the rewrite
    // silently failed. `5 [ 10 * ] c::apply2` must still land on `51`
    // ((5 * 10) + 1), and the import must not error.
    let path = temp_lib(
        "private-body-call-no-collision",
        ": priv_calc6c_nc ( i64 -- i64 ) 1 + ;\n\
         : apply2 ( i64 [ i64 -- i64 ] -- i64 ) | q | q call priv_calc6c_nc ;\n\
         export: apply2 ;\n",
    );
    let transcript = repl_error(&format!(
        "import: c \"{}\" ;\n5 [ 10 * ] c::apply2\n:quit\n",
        path.display()
    ));
    std::fs::remove_file(&path).ok();
    assert_eq!(transcript, "imported c\nstack: 51\n");
}

#[test]
fn repl_imported_private_word_still_rejected_by_qualified_name() {
    // FIX D / R15 (slice 6c, second review pass): R14's broadened body-call
    // rewrite binds a module-0 private word into `self.env` under its
    // internal spelling so a retained combinator's body can reach it (the
    // two tests above), but adds no `import_aliases` entry -- R15 privacy
    // still holds, so a session line naming it directly by its qualified
    // name still errors `not exported`, exactly as an ordinary private word
    // does.
    let path = temp_lib(
        "private-body-call-qualified",
        ": privword6c ( i64 -- i64 ) 1 + ;\n\
         : apply2 ( i64 [ i64 -- i64 ] -- i64 ) | q | q call privword6c ;\n\
         export: apply2 ;\n",
    );
    let transcript = repl_error(&format!(
        "import: c \"{}\" ;\n5 c::privword6c\n:quit\n",
        path.display()
    ));
    std::fs::remove_file(&path).ok();
    assert!(
        transcript.contains("imported c"),
        "the import itself must still succeed: {transcript}"
    );
    assert!(
        transcript.contains("not exported") && transcript.contains("privword6c"),
        "the qualified call to the private word is still rejected: {transcript}"
    );
}

#[test]
fn repl_mangled_internal_spelling_in_declared_name_is_rejected() {
    // FIX A (slice 6c, second review pass): a REPL-declared name ending in
    // the resolver's cross-module mangle (`__m{digits}`, `resolve::mangle`)
    // or the import-epoch tag (`__import{digits}`, `import_symbol`) is
    // rejected at definition time. Closes a forgeable-collision hole: a
    // multi-file closure's non-module-0 words are called, from a retained
    // combinator's body, by exactly this mangled spelling
    // (`{raw}__m{module_index}`), which is never rewritten to an internal
    // `{q}::...__import{epoch}` alias (the existing body-call rewrite only
    // covers module-0 words) -- so a same-named session word defined first
    // would otherwise silently win that body call instead of erroring. The
    // guard fires on the name alone, no import needed to reproduce.
    // (Column 3 is the name token itself, not the body's first term: `WordDef`
    // carries its own declaration span now, so `word_span` no longer derives
    // a word's location from its body -- see the `word_span` fix.)
    let transcript = repl_error(": dhelp__m1 ( i64 -- i64 ) 1000 + ;\n:quit\n");
    assert_eq!(
        transcript,
        "error: a REPL-declared word name may not end in a mangled `__m<digits>` or `__import<digits>` spelling (`dhelp__m1` at line 1, col 3)\n",
        "the mangled-suffix name is rejected outright, never defined: {transcript}"
    );
    assert!(
        !transcript.contains("defined dhelp__m1"),
        "must not have been accepted as an ordinary definition: {transcript}"
    );

    let transcript = repl_error(": foo__import7 ( -- i64 ) 1 ;\n:quit\n");
    assert_eq!(
        transcript,
        "error: a REPL-declared word name may not end in a mangled `__m<digits>` or `__import<digits>` spelling (`foo__import7` at line 1, col 3)\n",
        "the import-epoch-suffix name is rejected outright, never defined: {transcript}"
    );
    assert!(
        !transcript.contains("defined foo__import7"),
        "must not have been accepted as an ordinary definition: {transcript}"
    );
}

#[test]
fn repl_imported_while_runs_to_fixpoint() {
    // Criterion 8 (mutation-pins R14's body self-call rewrite): import the real
    // `lib/combinators.sth` and run `while` at a session line to a fixpoint.
    // `while`'s body self-call `while` is rewritten to its internal spelling on
    // import, so the self-tail recognizer fires and it lowers to a loop
    // back-edge (constant stack), not an endless splice. If the rewrite were
    // deleted, the self-call would miss the recognizer and the splice would
    // recurse forever.
    let transcript = repl_error(&format!(
        "{}0 [ dup 5 < if 1 + true else false end ] c::while\n:quit\n",
        combinators_import("c")
    ));
    assert_eq!(transcript, "imported c\nstack: 5\n");
}

#[test]
fn repl_imported_filter_runs() {
    // Criterion 9: import the real `lib/combinators.sth` and run `filter` over
    // an array at a session line. `7 3 fill` is three 7s; `[ 5 > ]` keeps all
    // three, so the residual is the compacted array then the kept-count `3`.
    // The two-output poly combinator lands both outputs on the residual stack,
    // exactly as the session-defined `filter` does.
    let transcript = repl_error(&format!(
        "{}7 3 fill [ 5 > ] c::filter\n:quit\n",
        combinators_import("c")
    ));
    assert_eq!(transcript, "imported c\nstack: <[i64 3]> 3\n");
}

#[test]
fn repl_import_combinator_with_private_type_in_signature_is_rejected() {
    // Criterion 10 (R15 confirm): an exported combinator whose *signature*
    // names a closure-private type is rejected at the closure's own `check`
    // (5a's export rule: a private type reachable through an exported
    // signature), before any REPL-side retention. No REPL-side guard is added;
    // this pins that the closure check already catches it. `run`'s effect names
    // `Secret`, which the closure declares but does not export.
    let path = temp_lib(
        "crit10-private",
        "type: Secret tag i64 ;\n\
         export: run ;\n\
         : run ( Secret [ i64 -- i64 ] -- ) | q | Secret>tag q call drop ;\n",
    );
    let transcript = repl_error(&format!("import: c \"{}\" ;\n:quit\n", path.display()));
    std::fs::remove_file(&path).ok();
    assert!(
        transcript.contains("names private type `Secret`") && transcript.contains("`run`"),
        "the exported combinator naming a private type is rejected at the closure check: {transcript}"
    );
    assert!(
        !transcript.contains("imported c"),
        "the import must not succeed: {transcript}"
    );
}

#[test]
fn repl_combinators_dogfood_matches_native() {
    // Criterion 12: a session transcript importing `lib/combinators.sth` and
    // using `filter`/`while` matches the native example's output. This is the
    // REPL twin of `examples/filter_while.sth` (R18's dogfood): the same
    // `scores` array, the same `[ 4 > ] filter` keeping 3 elements, the same
    // fixpoint `while` loop landing on 5. The native example prints both via
    // runtime `.` to real stdout ("3\n5\n"); a REPL bare line's `.` also goes
    // to real stdout rather than this capture writer (see `repl_error`), so
    // this leaves both results on the residual stack instead of printing them,
    // and pins the same two values in the same order.
    let transcript = repl_error(&format!(
        "{}{}\n{}\n{}\n:quit\n",
        combinators_import("c"),
        ": scores ( -- [i64 5] ) 0 5 fill | s | \
         &!s 0 >usize &!> 3 ! &!s 1 >usize &!> 7 ! &!s 2 >usize &!> 1 ! \
         &!s 3 >usize &!> 9 ! &!s 4 >usize &!> 5 ! s ;",
        "scores [ 4 > ] c::filter | n | | out | out drop n",
        "0 [ dup 5 < if 1 + true else false end ] c::while"
    ));
    assert_eq!(
        transcript,
        "imported c\ndefined scores\nstack: 3\nstack: 3 5\n"
    );
}

// -- the `__m0` monomorphization mangling must not leak into a diagnostic -----

/// Build a two-module program through the native driver and return its check
/// diagnostic. `entry` is the failing entry file's body; a trivial imported
/// library is prepended by `import:`, which is the whole point: with a second
/// module present, `resolve::resolve_modules` mangles module 0's decls to
/// `{name}__m0`, so the returned string is exactly where a mangled word name
/// would leak into user-facing text. The single-file twin of each `entry`
/// prints the clean name, so the import is the only variable.
fn build_error_with_import(name: &str, entry: &str) -> String {
    let lib = temp_lib(
        &format!("{name}-lib"),
        "export: helper ;\n: helper ( i64 -- i64 ) | x | x 1 + ;\n",
    );
    let entry_src = format!("import: lib \"{}\" ;\n{entry}", lib.display());
    let path = temp_lib(&format!("{name}-entry"), &entry_src);
    let err = sooth::driver::build(&path).expect_err("build should fail its check");
    std::fs::remove_file(&lib).ok();
    std::fs::remove_file(&path).ok();
    err
}

#[test]
fn r10_quotation_argument_diagnostic_shows_unmangled_word() {
    // A quotation against a non-quotation parameter `w`. With an import in the
    // module the callee's mangled name is `w__m0`; the diagnostic must name
    // `w`. The clean `` `w` `` cannot be a substring of `` `w__m0` `` (the
    // char after `w` is `_`, not a backtick), so the positive assertion is
    // not satisfied by the leak; the negative assertion pins the leak itself.
    let err = build_error_with_import(
        "m0-r10",
        ": w ( i64 -- i64 ) ;\n: main ( -- ) [ 1 + ] w . ;\n",
    );
    assert!(
        err.contains("a quotation cannot be passed to `w`"),
        "R10 should name the unmangled `w`: {err}"
    );
    assert!(!err.contains("__m0"), "R10 must not leak `__m0`: {err}");
}

#[test]
fn r22_combinator_cycle_diagnostic_shows_unmangled_words() {
    // A combinator `self` with a non-tail self-call; the cycle renders
    // `` `self` -> `self` `` and must not carry `self__m0`. (A tail-only
    // self-edge is now permitted, D5, so the recursion must be non-tail to
    // stay a cycle error.)
    let err = build_error_with_import(
        "m0-r22",
        ": self ( i64 [ i64 -- i64 ] -- i64 ) self drop ;\n: main ( -- ) 3 [ 1 + ] self . ;\n",
    );
    assert!(
        err.contains("`self` -> `self`"),
        "R22 should render the unmangled cycle: {err}"
    );
    assert!(!err.contains("__m0"), "R22 must not leak `__m0`: {err}");
}

#[test]
fn r12_capture_diagnostic_shows_unmangled_word() {
    // A literal consuming a linear enclosing local, passed to combinator `c`.
    let err = build_error_with_import(
        "m0-r12",
        &format!(
            "{SPY_DEF}: c ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
             : main ( -- ) 7 Spy | s | 3 [ s drop 0 + ] c . ;\n"
        ),
    );
    assert!(
        err.contains("the quotation passed to `c`") && err.contains("consumes the enclosing local"),
        "R12 should name the unmangled `c`: {err}"
    );
    assert!(!err.contains("__m0"), "R12 must not leak `__m0`: {err}");
}

#[test]
fn r7a_quotation_output_diagnostic_shows_unmangled_word() {
    // A *polymorphic* word `c` with a quotation in its output row. Slice 7a
    // makes a monomorphic quotation output legal (it materializes), so the
    // still-rejected output position that exercises the R7a audit's naming is
    // a poly one; the diagnostic must name `c`, not the mangled `c__m0`.
    let err = build_error_with_import(
        "m0-r7a",
        ": c ( ['T 'N] -- ['T 'N] [ 'T -- ] ) ;\n: main ( -- ) ;\n",
    );
    assert!(
        err.contains("cannot appear as the output of `c`"),
        "R7a should name the unmangled `c`: {err}"
    );
    assert!(!err.contains("__m0"), "R7a must not leak `__m0`: {err}");
}

#[test]
fn r11_effect_mismatch_diagnostic_shows_unmangled_word() {
    // A literal whose effect disagrees with combinator `show`'s parameter.
    let err = build_error_with_import(
        "m0-r11",
        ": show ( i64 [ i64 -- i64 ] -- i64 ) call ;\n: main ( -- ) 3 [ 1 + . ] show . ;\n",
    );
    assert!(
        err.contains("the quotation passed to `show` was declared"),
        "R11 should name the unmangled `show`: {err}"
    );
    assert!(!err.contains("__m0"), "R11 must not leak `__m0`: {err}");
}

#[test]
fn borrow_of_non_place_diagnostic_shows_unmangled_enclosing_word() {
    // The enclosing word `mymap` reaches the diagnostic through `in_word`; a
    // borrow of a non-place inside it must place itself in `mymap`, not
    // `mymap__m0`.
    let err = build_error_with_import(
        "m0-borrow",
        ": dst ( -- i64 ) 5 ;\n: mymap ( -- ) &!dst drop ;\n: main ( -- ) ;\n",
    );
    assert!(
        err.contains("does not borrow a place in `mymap`"),
        "the borrow diagnostic should name the unmangled `mymap`: {err}"
    );
    assert!(
        !err.contains("__m0"),
        "the borrow diagnostic must not leak `__m0`: {err}"
    );
}

#[test]
fn everyday_diagnostics_show_the_unmangled_enclosing_word() {
    // The leak is not confined to this slice's own messages: with an import
    // present, *every* diagnostic naming the enclosing word rendered `w__m0`.
    // These are the ordinary ones a user meets first, so they are the ones a
    // mangled name is most visible in. Each row is (label, body, expected
    // fragment naming the clean `w`).
    let rows: &[(&str, &str, &str)] = &[
        (
            "declared-outputs",
            ": w ( -- i64 ) ;\n: main ( -- ) ;\n",
            "stack effect mismatch in `w`",
        ),
        (
            "locals-exceed-inputs",
            ": w ( i64 -- ) | a b | drop drop ;\n: main ( -- ) ;\n",
            "stack effect mismatch in `w`",
        ),
        (
            "unknown-word",
            ": w ( -- ) nosuchword ;\n: main ( -- ) ;\n",
            "unknown word `nosuchword` in `w`",
        ),
        (
            "type-mismatch",
            ": w ( -- ) 1 2.0 + drop ;\n: main ( -- ) ;\n",
            "type mismatch in `w`",
        ),
        (
            "duplicate-local",
            ": w ( i64 -- ) | x | | x | drop ;\n: main ( -- ) ;\n",
            "`x` is already bound in `w`",
        ),
    ];
    for (label, body, want) in rows {
        let err = build_error_with_import(&format!("m0-{label}"), body);
        assert!(
            err.contains(want),
            "{label}: expected {want:?} naming the unmangled `w`, got: {err}"
        );
        assert!(
            !err.contains("__m"),
            "{label}: leaked a mangled name: {err}"
        );
    }
}

#[test]
fn cycle_and_accessor_diagnostics_show_unmangled_names() {
    // Two shapes the first sweep missed. The cycle renders a chain of `WordDef`
    // names, which never passed through the rendering boundary; the accessor
    // mangles as `P__m0>x`, so `__m0` sits mid-string and a trailing-suffix
    // strip cannot see it.
    let cycle = build_error_with_import(
        "m0-cycle",
        ": a ( -- ) b ;\n: b ( -- ) a ;\n: main ( -- ) a ;\n",
    );
    assert!(
        cycle.contains("mutual tail recursion `a` -> `b` -> `a`"),
        "the cycle should name unmangled words: {cycle}"
    );
    assert!(
        !cycle.contains("__m"),
        "cycle leaked a mangled name: {cycle}"
    );

    let accessor = build_error_with_import(
        "m0-accessor",
        "type: P x i64 y i64 ;\n: main ( -- ) 1 P>x ;\n",
    );
    assert!(
        accessor.contains("`P>x` expected `P`"),
        "the accessor should render as written: {accessor}"
    );
    assert!(
        !accessor.contains("__m"),
        "accessor leaked a mangled name: {accessor}"
    );
}

#[test]
fn self_tail_back_edge_check_still_fires_under_an_import() {
    // `Ctx` carries the demangled name for rendering and the mangled one for
    // self-tail recognition, which compares against mangled *call* names. Fuse
    // the two and a self-recursive word in an imported module stops matching
    // itself, so the back-edge checks are silently skipped.
    //
    // This must be a program the check *rejects*. A valid self-recursive word
    // builds either way -- recognition only gates extra checks, so losing it
    // costs a legal program nothing -- and would witness nothing at all.
    let err = build_error_with_import(
        "m0-backedge",
        "type: V x i64 ;\n\
         : spin ( &!V i64 -- )\n  | r n |\n  n 0 = if\n  else\n    \
         0 V | x |\n    &!x n 1 - spin\n  end ;\n\
         : main ( -- )\n  0 V | v |\n  &!v 3 spin\n  v drop ;\n",
    );
    assert!(
        err.contains("a reference to a local cannot cross a loop"),
        "the back-edge check must still fire under an import: {err}"
    );
    assert!(
        err.contains("in `spin`") && !err.contains("__m"),
        "and must name the unmangled `spin`: {err}"
    );
}

#[test]
fn combinator_called_from_drop_override_body_lowers_correctly() {
    // Regression (pre-6c bug, still present natively when this landed):
    // `synthesize_struct_destructor_override` lowered a drop override's body
    // through the `lower_word` convenience wrapper, which hardcodes an
    // *empty* combinators map. A native build's own module-level combinators
    // exist by this point, so calling one (`twice`) from a `drop` override's
    // body panicked at lowering ("checked user word exists") instead of
    // splicing the call, exactly like any other word body.
    let (stdout, code) = run_src(
        "drop-override-combinator",
        ": twice ( i64 [ i64 -- i64 ] -- i64 ) | q | q call q call ;\n\
         type: Bx v i64 ;\n\
         : drop ( Bx -- ) Bx>v [ 1 + ] twice . ;\n\
         : main ( -- ) 1 Bx drop ;\n",
    );
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert_eq!(stdout, "3\n");
}

// -- a self-tail combinator whose quotation parameter is never bound ---------

/// A self-tail combinator keeps its quotation parameter loop-invariant by
/// hoisting it out of the carried row. That hoist used to be recognised only
/// when the body *named* the parameter with a leading `| p |` (the `while`
/// idiom); a body that reaches it with `dup` instead left the phantom in the
/// row, where `begin_loop` staged it as an aggregate (`is_aggregate` answers
/// `true` for `IrType::Quotation`) and blitted from a phantom that owns no
/// bytes. The first `call` then found no `quot_bodies` entry, fell to the
/// indirect path, and hit its `unreachable!` -- an ICE, not a diagnostic.
#[test]
fn self_tail_combinator_dups_its_quotation_instead_of_binding_it() {
    let (stdout, code) = run_src(
        "dup-quot-self-tail",
        ": rep ( i64 [ -- ] -- )\n\
         dup call swap 1 - dup 0 > if swap rep else drop drop end ;\n\
         : main ( -- ) 3 [ 7 . ] rep ;\n",
    );
    assert_eq!(code, 0, "stdout was: {stdout}");
    // Three iterations, not two or four: the count proves the back-edge
    // carries the decremented state while the quotation stays invariant.
    assert_eq!(stdout, "7\n7\n7\n");
}

/// The `~` twin of the above: an inline-only quotation parameter takes the
/// same hoist, so retyping a combinator's parameter cannot change its
/// lowering.
#[test]
fn self_tail_combinator_dups_an_inline_quotation_parameter() {
    let (stdout, code) = run_src(
        "dup-inline-quot-self-tail",
        ": rep ( i64 ~[ -- ] -- )\n\
         dup call swap 1 - dup 0 > if swap rep else drop drop end ;\n\
         : main ( -- ) 3 [ 9 . ] rep ;\n",
    );
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert_eq!(stdout, "9\n9\n9\n");
}

/// The hoisted parameter must leave a real loop behind, not recursion: 1M
/// iterations at a 1 MB stack would die by `SIGSEGV` if the self-call still
/// grew a frame.
#[test]
fn dup_quotation_self_tail_loop_runs_in_constant_stack() {
    let binary = build_binary(
        "dup-quot-constant-stack",
        ": rep ( i64 [ -- ] -- )\n\
         dup call swap 1 - dup 0 > if swap rep else drop drop end ;\n\
         : main ( -- ) 1000000 [ ] rep 42 . ;\n",
    );
    let (code, stdout) = run_at_stack_limit(&binary, 1024);
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        code,
        Some(0),
        "died by signal or non-zero; stdout: {stdout}"
    );
    assert_eq!(stdout, "42");
}
