//! Phase 4 slice 6g goldens: the ancestor half of the D6 grant becomes
//! loop-aware (R1), and combinator splices learn the same granting rule the
//! `call`/`times`/`if` doorways already follow.
//!
//! The four ancestor-grant rejects below (T-wrap, T-danger, T-bcall, T-d2)
//! are behaviour *changes*: each compiles on the pre-6g compiler. The first
//! two (T-wrap, T-danger) are genuine wrong-value witnesses -- they run and
//! print `0` then `9`, a mutation through one name visible through another
//! across a loop back-edge, because `releasable_into` asked only "is this
//! name used in the remaining sibling terms", which is the wrong question
//! inside a body that wraps around to its own first term. The other two
//! (T-bcall, T-d2) are the accepted cost of R1's conservative approximation:
//! sound programs rejected because the checker cannot distinguish "mentioned
//! inside the granted-into term" from "read across the edge". T-doorway-no
//! is pre-existing behaviour, unchanged by R1.
//!
//! T-sort (the `sort` dogfood) is deliberately absent: its subject moved to
//! `examples/experiments/arrays.sth`, an experiment rather than library code,
//! and tests are not written against it. The binding shape it exercised -- a
//! bound array local passed to a combinator, which the pre-6g compiler rejected
//! -- stays covered by `bound_array_passed_to_filter_is_accepted` and the two
//! `while_over_an_aliased_array_local_*` positives over `lib/combinators.sth`.

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

/// An `import:` line for a committed library by *absolute* path, so a temp
/// source built under `temp_dir()` resolves it regardless of cwd. Generalizes
/// `combinators_import` (`tests/phase4_combinators.rs`), which is hardcoded to
/// `lib/combinators.sth` and cannot be repointed at another library file.
fn lib_import(qualifier: &str, lib_file: &str) -> String {
    format!(
        "import: \"{}/{lib_file}\" {qualifier} ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn check_error(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).expect("lexing should succeed");
    let mut module = sooth::parser::parse(&tokens).expect("parsing should succeed");
    sooth::check::check(&mut module).expect_err("check should fail")
}

/// `lib/combinators.sth`'s `times`, inlined: `check_error` runs the checker in
/// process, where an `import:` line never resolves.
const TIMES_DEF: &str = ": times-helper inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | to | | from |\n\
     from to lt ~[ from f call from 1 add to f times-helper ] ~[ ] if ;\n\
     : times inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
     | f | | n | 0 n f times-helper ;\n";

#[test]
fn times_def_hand_copy_is_pinned_to_the_library() {
    common::assert_pinned_to_combinators_lib(TIMES_DEF, &[]);
}

/// The reject twin of `run_src`, for a program whose import needs the
/// multi-module driver but whose check is expected to fail before it runs.
fn build_err_with_import(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build(&path).expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    err
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
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- )\n\
         0 4 fill | a |\n\
         2 ~[ | i | a | arr | &a 0 >usize &> @ . true ~[ &!arr 0 >usize &!> 9 ! ] ~[ ] if arr drop ] times ;\n"
    ));
    assert_aliased_by(&err, "arr", "a");
}

#[test]
fn read_and_mutate_inside_a_looped_grant_is_an_error() {
    // T-danger. The same wrong value with both the read and the write inside
    // the granted-into term, so the use is invisible to a rule that only scans
    // siblings. Accepted pre-6g, printing `0` then `9`. This shape is why R1
    // cannot be refined to "unmentioned in the siblings before and after".
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- )\n\
         0 4 fill | a |\n\
         2 ~[ | i | true ~[ a | arr | &a 0 >usize &> @ . &!arr 0 >usize &!> 9 ! arr drop ] ~[ ] if ] times ;\n"
    ));
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
         [ true ~[ a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop ] ~[ ] if ] call ;\n",
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
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- )\n\
         0 4 fill | a |\n\
         2 ~[ | i | true ~[ a | arr | &!arr 0 >usize &!> 9 ! arr drop ] ~[ ] if ] times ;\n"
    ));
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
         true ~[\n\
         true ~[\n\
         a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop\n\
         ] ~[ ] if\n\
         ] ~[ ] if ;\n",
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
        &format!(
            "{}: main ( -- )\n\
             0 4 fill | a |\n\
             a | arr |\n\
             4 ~[ | i | &!arr i >usize &!> i ! ] times\n\
             &arr 2 >usize &> @ .\n\
             arr drop ;\n",
            lib_import("c | times |", "lib/combinators.sth")
        ),
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
    let err = check_error(&format!(
        "{TIMES_DEF}: main ( -- )\n\
         0 4 fill | a |\n\
         a | arr |\n\
         4 ~[ | i | &!arr i >usize &!> i ! ] times\n\
         arr drop\n\
         &a 0 >usize &> @ .\n\
         a drop ;\n"
    ));
    assert_aliased_by(&err, "arr", "a");
}

// -- D5: a local can never collide with a callable name ---------------------

#[test]
fn binding_a_local_named_after_a_builtin_is_rejected() {
    // T-shadow. `len` is a builtin name. `scope.local` is checked ahead of
    // every builtin/word lookup at a `Call` (see the `TermKind::Call` arm),
    // so a local named `len` would shadow the builtin at every later call to
    // `len` -- recon 10's hygiene defect, generalized: it needs no combinator
    // splice, the local's name alone is enough. D5 rejects the bind itself.
    // Accepted pre-6g, printing `1` silently instead of running the builtin.
    let err = check_error(
        ": main ( -- )\n\
         1 | len |\n\
         len . ;\n",
    );
    assert!(
        err.contains("`len`") && err.contains("collides with the callable name"),
        "expected the D5 collision wording naming `len`: {err}"
    );
}

#[test]
fn binding_a_poly_local_named_after_a_builtin_is_rejected() {
    // The poly-arm twin of T-shadow: a two-site guard needs a mutation
    // target at each site. Deleting only the poly arm's rejection leaves the
    // whole suite green while this builds and prints `2`.
    let err = check_error(
        ": pick ( 'T 'T -- 'T ) | len | | other | other drop len ;\n\
         : main ( -- ) 1 2 pick . ;\n",
    );
    assert!(
        err.contains("`len`") && err.contains("collides with the callable name"),
        "expected the D5 collision wording naming `len`: {err}"
    );
}

// -- D1/R2: a combinator splice learns the same granting rule ---------------

#[test]
fn bound_array_passed_to_filter_is_accepted() {
    // T-splice (P-splice). The identical aliasing shape the `times`/`call`/`if`
    // doorways already grant, routed through a combinator splice instead:
    // `filter` mutates the array in place through its own body-local name, and
    // the caller's `a` is never mentioned again. Rejected pre-D1 (`aliased by
    // a`), because `inline_combinator`'s body-check ran the plain `check_terms`
    // -- the root entry point, which grants nothing -- instead of
    // `check_terms_relaxed` with a `releasable_into`-computed grant. D1 is the
    // whole fix here: `filter`'s own predicate literal `[ 4 gt ]` never mentions
    // the array, so R2's pass has nothing to do with this shape (M-D1 reds
    // this test, M-R2 does not; see Q-witness).
    let (out, code) = run_src(
        "6g-splice",
        &format!(
            "{}\n\
             : main ( -- )\n\
             0 4 fill | a |\n\
             a ~[ 4 gt ] c::filter drop drop ;\n",
            lib_import("c", "lib/combinators.sth")
        ),
    );
    assert_eq!(out, "");
    assert_eq!(code, 0);
}

#[test]
fn while_over_an_aliased_array_local_is_accepted() {
    // T-while. `input` returns a fresh array via a producer word; `a` binds
    // it, then `arr` re-binds it (arrays are `Copy`, so this duplicates the
    // handle, not the storage -- `a` and `arr` now alias one region). `a` is
    // never read again. `while`'s own body threads the loop predicate through
    // `c::while`, writing `arr[i] = 9` for `i` in `0..4`, then prints
    // `arr[0]`. Accepted only under D1+R2: reverting either reds it with the
    // same `aliased by a` error (Q-witness) -- the literal-check pass (R2)
    // sees the rebind directly in the literal it type-checks, and the
    // body-splice pass (D1) re-checks the same literal against the real
    // runtime slots once `while`'s own `p call` actually executes it.
    let (out, code) = run_src(
        "6g-while",
        &format!(
            "{}\n\
             : input ( -- [i64 4] ) 0 4 fill | s | s ;\n\
             : main ( -- )\n\
             input | a |\n\
             a | arr |\n\
             0 ~[ | i | &!arr i >usize &!> 9 ! i 1 add dup 4 lt ] c::while drop\n\
             &arr 0 >usize &> @ . ;\n",
            lib_import("c", "lib/combinators.sth")
        ),
    );
    assert_eq!(out, "9\n");
    assert_eq!(code, 0);
}

#[test]
fn while_over_an_aliased_array_local_rejects_if_the_original_name_is_read_in_the_loop() {
    // M-T-while-bounds variant 1: mention the original name `a` anywhere
    // inside the while-literal (here, a no-op read at the top of the loop
    // body). Must keep rejecting under the full R1+D1+R2 fix -- proves
    // T-while is not accidentally over-permissive.
    // (This still rejects under a full R1 revert and under both `back_edge`
    // flags false, so it is a boundary guard, not a witness that any single
    // mechanism -- `IMMORTAL_IN_BODY` included -- is what does the work.)
    let err = build_err_with_import(
        "6g-while-read",
        &format!(
            "{}\n\
             : input ( -- [i64 4] ) 0 4 fill | s | s ;\n\
             : main ( -- )\n\
             input | a |\n\
             a | arr |\n\
             0 ~[ | i | &a 0 >usize &> @ drop &!arr i >usize &!> 9 ! i 1 add dup 4 lt ] c::while drop\n\
             &arr 0 >usize &> @ . ;\n",
            lib_import("c", "lib/combinators.sth")
        ),
    );
    assert_aliased_by(&err, "arr", "a");
}

#[test]
fn while_over_an_aliased_array_local_rejects_if_the_original_name_is_used_after_the_loop() {
    // M-T-while-bounds variant 2: use `a` again in `main` after the loop. The
    // ordinary `references(rest, name)` rule correctly excludes it from
    // `releasable_into`'s output. Proves T-while is not accidentally
    // over-permissive.
    let err = build_err_with_import(
        "6g-while-after",
        &format!(
            "{}\n\
             : input ( -- [i64 4] ) 0 4 fill | s | s ;\n\
             : main ( -- )\n\
             input | a |\n\
             a | arr |\n\
             0 ~[ | i | &!arr i >usize &!> 9 ! i 1 add dup 4 lt ] c::while drop\n\
             &arr 0 >usize &> @ .\n\
             &a 0 >usize &> @ drop ;\n",
            lib_import("c", "lib/combinators.sth")
        ),
    );
    assert_aliased_by(&err, "arr", "a");
}
