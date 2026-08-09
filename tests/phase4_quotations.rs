//! Phase 4 Slice 7a goldens: a non-capturing quotation literal reaching a
//! materialization boundary (a struct field, an array element, a word output,
//! or a branch join) becomes a runtime `(code, env)` value, and `call`/`times`
//! on an erased quotation whose identity the checker cannot resolve emits an
//! indirect call. A capturing literal at a boundary is rejected (7b); a
//! capturing literal at a direct `call` still splices, unaffected. The branch
//! join materializes each arm against the enclosing declared output row (R11);
//! the same literal in both arms stays a splice.

use sooth::ir::Instr;
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

/// Whether the lowered module emits at least one indirect call: the witness
/// that a `call` resolved to a runtime dispatch through a materialized value,
/// not a compile-time body splice.
fn emits_call_indirect(src: &str) -> bool {
    count_call_indirect(src) > 0
}

/// The total number of indirect calls in the lowered module: `times` over an
/// erased quotation must emit exactly one (in the loop body, not once per
/// iteration).
fn count_call_indirect(src: &str) -> usize {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
    let ir = sooth::ir::lower(&module).expect("lower should succeed");
    ir.funcs
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::CallIndirect(..)))
        .count()
}

// -- T-field: a quotation stored in a struct field, called back out ----------

#[test]
fn quotation_stored_in_struct_field_compiles_and_calls() {
    let src = "type: Holder q [ i64 -- i64 ] ;\n\
               : main ( -- ) [ 1 + ] Holder Holder>q 4 swap call . ;\n";
    let (stdout, code) = run_src("qfield", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "the store/getter path must lower `call` to an indirect call"
    );
}

// -- T-array: two quotations in an array, each indirect-called ---------------

#[test]
fn quotation_in_array_element_indirect_calls() {
    // The seed element comes from a word returning an *erased* quotation (a
    // declared-effect context `fill` can intern an element type from); the
    // second element is a literal stored through the array's `&!` referent,
    // which carries the declared `[ i64 -- i64 ]` the store materializes it
    // against. A bare literal to `fill` has no effect context and is rejected.
    let src = ": one ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- )\n\
               one 2 fill | a |\n\
               &!a 1 >usize &!> [ 2 + ] !\n\
               &a 0 &> @ 4 swap call .\n\
               &a 1 &> @ 4 swap call . ;\n";
    let (stdout, code) = run_src("qarray", src);
    assert_eq!(stdout, "5\n6\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "reading an element back and calling it must be an indirect call"
    );
}

// -- T-return: a word returning a quotation, indirect-called at the call site -

#[test]
fn quotation_returned_from_word_indirect_calls() {
    let src = ": mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n\
               : main ( -- ) mk 4 swap call . ;\n";
    let (stdout, code) = run_src("qreturn", src);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a returned quotation is erased at the call site, so `call` is indirect"
    );
}

// -- T-cap-store: a capturing literal at a store boundary is rejected (7b) ----

#[test]
fn capturing_literal_stored_is_error_naming_7b() {
    // `[ x + ]` reads the enclosing local `x`, so storing it into a field
    // would capture an environment 7a does not represent. The message is
    // distinct from the pre-existing `:7024` "escaping quotations are slice 7"
    // wording, so it is asserted whole rather than by `.contains`.
    let err = check_error(
        "type: Holder q [ i64 -- i64 ] ;\n\
         : main ( -- ) 10 | x | [ x + ] Holder drop ;\n",
    );
    assert_eq!(
        err,
        "error: a capturing quotation cannot be stored (capturing closures are slice 7b) (line 2)"
    );
}

// -- M3: capture through a *nested* quotation is still caught -----------------

#[test]
fn capturing_through_nested_quotation_is_error() {
    // `x` is read inside a nested `[ x + ]`, not at the stored quotation's own
    // top level. The capture predicate must recurse into nested quotation
    // bodies (D4) or this store wrongly materializes.
    let err = check_error(
        "type: Holder q [ i64 -- i64 ] ;\n\
         : main ( -- ) 10 | x | [ [ x + ] call ] Holder drop ;\n",
    );
    assert_eq!(
        err,
        "error: a capturing quotation cannot be stored (capturing closures are slice 7b) (line 2)"
    );
}

// -- T-splice-cap: a capturing literal at a direct `call` still splices -------

#[test]
fn capturing_literal_spliced_still_works() {
    // Unchanged from the combinator slices: a capturing literal consumed by a
    // direct `call` is spliced in place (no materialization, no boundary), so
    // it reads the enclosing local and runs.
    let src = ": main ( -- ) 10 | x | 4 [ x + ] call . ;\n";
    let (stdout, code) = run_src("qsplice", src);
    assert_eq!(stdout, "14\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "a spliced literal must not lower to an indirect call"
    );
}

// -- T-join: two differing quotation arms materialize against the declared ---
// -- output row and are indirect-called ---------------------------------------

#[test]
fn two_differing_quotation_arms_materialize_and_call() {
    // Each arm leaves a *different* literal; the join has no single body to
    // splice, so it materializes each against the word's declared
    // `[ i64 -- i64 ]` output (R11) and `call` at the site dispatches
    // indirectly through whichever aggregate the branch left.
    let src = ": pick ( bool -- [ i64 -- i64 ] ) if [ 1 + ] else [ 2 + ] end ;\n\
               : main ( -- ) true pick 4 swap call . false pick 4 swap call . ;\n";
    let (stdout, code) = run_src("qjoin", src);
    assert_eq!(stdout, "5\n6\n");
    assert_eq!(code, 0);
    assert!(
        emits_call_indirect(src),
        "a materialized branch-join quotation is indirect-called"
    );
}

// -- T-join-same: one literal in both arms stays a splice (no materialize) ----

#[test]
fn same_quotation_both_arms_still_splices() {
    // One literal bound before the `if`, named in both arms: the equal `Known`
    // ids forward the marker (no `Phi`, no materialization), so `call` after
    // the join splices exactly as today and emits no indirect call.
    let src = ": main ( -- ) [ 1 + ] | q | true if q else q end 5 swap call . ;\n";
    let (stdout, code) = run_src("qjoinsame", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
    assert!(
        !emits_call_indirect(src),
        "the same literal in both arms is spliced, never indirect-called"
    );
}

// -- T-cap-join: a capturing literal in one arm of a materializing join -------

#[test]
fn capturing_literal_at_join_is_error_naming_7b() {
    // A materializing join (the word declares a `[ i64 -- i64 ]` output) with a
    // capturing literal in one arm. R11's ordering pin: the capture check runs
    // before the id/expected-type resolution, so this fires the capturing-
    // quotation error rather than `different_quotations_at_join_error`.
    let err = check_error(
        ": pick ( bool -- [ i64 -- i64 ] ) 10 | x | if [ x + ] else [ 2 + ] end ;\n\
         : main ( -- ) true pick 4 swap call . ;\n",
    );
    assert_eq!(
        err,
        "error: a capturing quotation cannot be left on a branch (capturing closures are slice 7b) (line 1)"
    );
}

// -- T-times: `times` over an erased quotation is one indirect call in a loop -

#[test]
fn times_over_erased_quotation_runs_constant_stack() {
    // `acc` returns an *erased* quotation (a word-output materialization
    // boundary); `times` drives it. The checker accepts the abstract
    // `[ i64 i64 -- i64 ]` effect and lowering emits exactly one `CallIndirect`
    // inside the loop body (D6: constant stack, one indirect call per
    // iteration), summing 0+1+2+3+4 = 10.
    let src = ": acc ( -- [ i64 i64 -- i64 ] ) [ + ] ;\n\
               : main ( -- ) 0 5 acc times . ;\n";
    let (stdout, code) = run_src("qtimes", src);
    assert_eq!(stdout, "10\n");
    assert_eq!(code, 0);
    assert_eq!(
        count_call_indirect(src),
        1,
        "a `times`-erased loop has exactly one indirect call, in the body"
    );
}
