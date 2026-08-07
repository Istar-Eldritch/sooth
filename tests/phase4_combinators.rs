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
            src: "type: S f [ i64 -- ] ;\n1\n",
            position: "the field `f` of struct `S`",
        },
        Row {
            src: "type: E | v p [ i64 -- ] ;\n1\n",
            position: "the field `p` of enum variant `E::v`",
        },
        Row {
            src: ": w ( [ [ i64 -- ] 3 ] -- ) drop ;\n1\n",
            position: "an array element",
        },
        Row {
            src: ": w ( ^[ i64 -- ] -- ) drop ;\n1\n",
            position: "an owned-cell payload",
        },
        Row {
            src: ": w ( &[ i64 -- ] -- ) drop ;\n1\n",
            position: "a reference referent",
        },
        Row {
            src: ": w ( -- [ i64 -- ] ) [ drop ] ;\n1\n",
            position: "the output of `w`",
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
    let item1_rows = [
        BrickRow {
            src: "type: P x [ [ i64 -- ] 3 ] ;\n40 2 +\n",
            msg: "a quotation type",
        },
        BrickRow {
            src: "type: P x ^[ i64 -- ] ;\n40 2 +\n",
            msg: "a quotation type",
        },
        BrickRow {
            src: "type: P x &[ i64 -- ] ;\n40 2 +\n",
            msg: "a reference cannot be stored",
        },
        BrickRow {
            src: "type: Q | Mk a [ [ i64 -- ] 3 ] ;\n40 2 +\n",
            msg: "a quotation type",
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
    check_ok(each);

    // Pin the isolated copy to the committed library: if the real file stopped
    // checking standalone, the copy above could silently drift from it.
    let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/combinators.sth"))
        .expect("the combinator library should be readable");
    check_ok(&lib);
}

// -- criterion 9b: `map`/`fold` check compositionally ------------------------

#[test]
fn map_and_fold_check_compositionally() {
    // Criterion 9b (respecified, item 4): `map`/`fold` are NOT built on `each`
    // (the library header explains why: `each`'s `[ 'T -- ]` element quotation
    // hands neither the array nor the index, so a write-back or an accumulator
    // needs either a captured mutable borrow (D3-forbidden) or a row variable
    // in the effect (R28, out of scope)). Each is a leaf combinator driving its
    // own `times`. "Compositional" (D4) therefore means each body checks
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
    check_ok(map);
    check_ok(fold);

    // ...and the compositional check bites at the def site: declaring `f` as
    // `[ 'T -- 'T ]` (producing a value) but leaving that value on the floor
    // unbalances the `times` row. That this is located -- naming `m`, at its
    // own def site -- is proof `f call` was checked to *produce* a `'T` per the
    // declared effect, not rubber-stamped.
    let err = check_error(
        ": m ( ['T 'N] [ 'T -- 'T ] -- )\n\
         | f | len >i64 | count | | arr |\n\
         count [ | i | &arr i >usize &> @ f call ] times\n\
         arr drop ;\n",
    );
    assert!(
        err.contains("`m`") && err.contains("times") && err.contains("leave the row unchanged"),
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
        "{SPY_DEF}\
         : bad ( ['T 'N] Spy [ 'T -- ] -- )\n\
         | f | | s | len >i64 | count | | arr |\n\
         count [ | i | &arr i >usize &> @ f call s drop ] times\n\
         arr drop ;\n"
    );
    let err = check_error(&src);
    assert!(
        err.contains("`bad`") && err.contains("cannot consume `s`"),
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
    let src = "type: V x i64 ;\n\
               : bad ( ['T 'N] V [ 'T -- ] -- )\n\
               | f | | v | len >i64 | count | | arr |\n\
               count [ | i | &arr i >usize &> @ f call &v ] times\n\
               arr drop v drop ;\n";
    let err = check_error(src);
    assert!(
        err.contains("`bad`") && err.contains("cannot leave a reference live across the loop"),
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
    let src = "type: Box v i64 ;\n\
               : refout ( ['T 4] [ 'T -- &i64 ] -- )\n\
               | f | | arr |\n\
               4 [ | i | &arr i >usize &> @ f call drop ] times\n\
               arr drop ;\n\
               : main ( -- )\n\
               7 Box | b |\n\
               0 4 fill [ | x | &b &Box>v ] refout\n\
               b drop ;\n";
    let err = check_error(src);
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
        ": main ( -- ) 1 {N} fill | arr | 0 {N} [ | i | &arr i >usize &> @ + ] times . arr drop ;\n"
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
    // A self-recursive combinator `self`; the cycle renders `` `self` ->
    // `self` `` and must not carry `self__m0`.
    let err = build_error_with_import(
        "m0-r22",
        ": self ( i64 [ i64 -- i64 ] -- i64 ) self ;\n: main ( -- ) 3 [ 1 + ] self . ;\n",
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
             : main ( -- ) 7 Spy | s | 3 [ s Spy>tag + ] c . ;\n"
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
    // A word `c` with a quotation in its *output* row (R7a audit).
    let err = build_error_with_import(
        "m0-r7a",
        ": c ( i64 -- [ i64 -- i64 ] ) [ 1 + ] ;\n: main ( -- ) ;\n",
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
