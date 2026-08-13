//! Phase 4 slice 6g goldens: the ancestor half of the D6 grant becomes
//! loop-aware (R1), and combinator splices learn the same granting rule the
//! `call`/`times`/`if` doorways already follow.
//!
//! The four ancestor-grant rejects below (T-wrap, T-danger, T-bcall, T-d2)
//! are behaviour *changes*: each compiles on the pre-6g compiler. The first
//! two (T-wrap, T-danger) are genuine wrong-value witnesses -- they run and
//! print `0` then `9`, a mutation through one name visible through another
//! across a loop back-edge. The other two (T-bcall, T-d2) are the accepted
//! cost of R1's conservative approximation: sound programs rejected because
//! the checker cannot distinguish "mentioned inside the granted-into term"
//! from "read across the edge". T-doorway-no is pre-existing behaviour,
//! unchanged by R1. `releasable_into` asked only "is this name used in the
//! remaining sibling terms", which is the wrong question inside a body that
//! wraps around to its own first term.

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
    let tokens = sooth::lexer::lex(src).expect("lexing should succeed");
    let mut module = sooth::parser::parse(&tokens).expect("parsing should succeed");
    sooth::check::check(&mut module).expect_err("check should fail")
}

/// The aliasing rejection every R1 golden expects, named identifiers and all:
/// the borrowed name, the alias it shares a region with, and the borrow verb.
fn assert_aliased_by(err: &str, borrowed: &str, alias: &str) {
    assert!(
        err.contains("cannot borrow") && err.contains(&format!("`{borrowed}`")),
        "expected a borrow rejection naming `{borrowed}`: {err}"
    );
    assert!(
        err.contains(&format!("it is aliased by `{alias}`")),
        "expected the rejection to name the alias `{alias}`: {err}"
    );
}

// -- R1: the ancestor grant is withheld inside a back-edge body --------------

#[test]
fn if_inside_a_loop_reading_an_alias_is_an_error() {
    // T-wrap. `a` is read at the top of the `times` body and re-bound to `arr`,
    // which the `if` arm mutates. The grant into that arm used to look only at
    // the terms *after* it, missing the wrap-around: iteration 2's read through
    // `a` sees iteration 1's write through `arr`. Accepted pre-6g, printing
    // `0` then `9`.
    let err = check_error(
        ": main ( -- )\n\
         0 4 fill | a |\n\
         2 [ | i | a | arr | &a 0 >usize &> @ . true if &!arr 0 >usize &!> 9 ! else end arr drop ] times ;\n",
    );
    assert_aliased_by(&err, "arr", "a");
}

#[test]
fn read_and_mutate_inside_a_looped_grant_is_an_error() {
    // T-danger. The same wrong value with both the read and the write inside
    // the granted-into term, so the use is invisible to a rule that only scans
    // siblings. Accepted pre-6g, printing `0` then `9`. This shape is why R1
    // cannot be refined to "unmentioned in the siblings before and after".
    let err = check_error(
        ": main ( -- )\n\
         0 4 fill | a |\n\
         2 [ | i | true if a | arr | &a 0 >usize &> @ . &!arr 0 >usize &!> 9 ! arr drop else end ] times ;\n",
    );
    assert_aliased_by(&err, "arr", "a");
}

#[test]
fn single_call_body_naming_the_alias_is_an_error() {
    // T-bcall, the accepted cost of R1: one invocation, no wrap-around, `a`
    // named nowhere else -- sound, and accepted pre-6g (printing `9`), but
    // rejected now. A `call`ed quotation is checked as a back-edge body (it can
    // be invoked from elsewhere), so any mention of a granted ancestor name
    // inside it pins that name live throughout. Pinned so the behaviour change
    // is recorded rather than discovered later.
    let err = check_error(
        ": main ( -- )\n\
         0 4 fill | a |\n\
         [ true if a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop else end ] call ;\n",
    );
    assert_aliased_by(&err, "arr", "a");
}

#[test]
fn write_only_across_a_back_edge_is_an_error() {
    // T-d2, the same accepted cost at a loop: the body writes through `arr`
    // every iteration and nothing ever reads the stale value, so no wrong value
    // is observable. Accepted pre-6g. The checker cannot tell "mentioned inside
    // the granted-into term" from "read across the edge" without machinery
    // beyond this slice, so this rejects too.
    let err = check_error(
        ": main ( -- )\n\
         0 4 fill | a |\n\
         2 [ | i | true if a | arr | &!arr 0 >usize &!> 9 ! arr drop else end ] times ;\n",
    );
    assert_aliased_by(&err, "arr", "a");
}

// -- R1 does not over-tighten ------------------------------------------------

#[test]
fn two_level_execute_once_grant_still_accepted() {
    // T-nest2. Two levels of execute-once nesting with `a` used only inside the
    // innermost, so the grant must still chain down. This is what pins R1's
    // index at `at + 1` rather than `at`: a use inside `terms[at]` is
    // attributed to `at` itself, so asking `dead(name, at)` would withhold the
    // grant from exactly the block that wanted it, and this program would
    // reject.
    let (out, code) = run_src(
        "6g-nest2",
        ": main ( -- )\n\
         0 4 fill | a |\n\
         true if\n\
         true if\n\
         a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop\n\
         else end\n\
         else end ;\n",
    );
    assert_eq!(out, "9\n");
    assert_eq!(code, 0);
}

#[test]
fn times_doorway_grants_the_bound_alias() {
    // T-doorway-ok. The `times` doorway grants `a` into the loop body -- `a` is
    // never mentioned again anywhere -- so the aliased mutable borrow of `arr`
    // is allowed. Accepted before and after R1; also the witness that R1's two
    // filter branches are not interchangeable, since routing a
    // bound-in-this-invocation name through the ancestor branch reds it.
    let (out, code) = run_src(
        "6g-doorway-ok",
        ": main ( -- )\n\
         0 4 fill | a |\n\
         a | arr |\n\
         4 [ | i | &!arr i >usize &!> i ! ] times\n\
         &arr 2 >usize &> @ .\n\
         arr drop ;\n",
    );
    assert_eq!(out, "2\n");
    assert_eq!(code, 0);
}

#[test]
fn later_use_withholds_the_times_grant() {
    // T-doorway-no. The same program with one later use of `a`, which withholds
    // the doorway's grant. Pre-existing behaviour, unchanged by R1 (the
    // `references(rest, name)` rule still governs a name bound in this
    // invocation): a boundary guard against future drift, not a 6g pin.
    let err = check_error(
        ": main ( -- )\n\
         0 4 fill | a |\n\
         a | arr |\n\
         4 [ | i | &!arr i >usize &!> i ! ] times\n\
         arr drop\n\
         &a 0 >usize &> @ .\n\
         a drop ;\n",
    );
    assert_aliased_by(&err, "arr", "a");
}
