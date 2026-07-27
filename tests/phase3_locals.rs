//! Phase 3 Slice 5 goldens: `| names |` is a term, legal at any point in a
//! body, with its extent running to the end of the enclosing block.

use std::path::Path;

use sooth::{check, lexer, parser};

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
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

fn check_ok(src: &str) {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
}

#[test]
fn mid_body_binding_consumes_from_the_stack() {
    // Criterion 1: `| a b |` pops two values where it appears, leaving the `1`
    // beneath them on the stack for the term after the binding's users.
    let (stdout, code) = run_src(
        "mid-body-binding-consumes",
        ": main ( -- )\n  1 2 3\n  | a b |\n  a b + .\n  . ;\n",
    );
    assert_eq!(stdout, "5\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn mid_body_binding_leftmost_name_takes_deepest_value() {
    // Criterion 2: the entry form's rule holds mid-body too.
    let (stdout, code) = run_src(
        "mid-body-binding-leftmost",
        ": main ( -- )\n  10 20\n  | a b |\n  a . b . ;\n",
    );
    assert_eq!(stdout, "10\n20\n");
    assert_eq!(code, 0);
}

#[test]
fn local_bound_in_if_arm_is_not_visible_after_end() {
    // Criterion 3: the arm is the extent (R2), so `x` past `end` is not a name
    // at all: it resolves as a word, and there is no such word.
    let err = check_error(": w ( bool -- i64 )\n  if 7 | x | x else 0 end\n  x ;\n");
    assert!(err.contains("unknown word"), "unexpected message: {err}");
    assert!(err.contains("`x`"), "unexpected message: {err}");
}

#[test]
fn name_bound_in_one_arm_can_be_rebound_in_sibling_arm() {
    // Criterion 4: the first `v`'s extent ended at `else`, so the second is a
    // fresh binding, not the re-binding R4 rejects. Proves teardown happens.
    let (stdout, code) = run_src(
        "sibling-arm-rebind",
        ": pick ( bool -- i64 )\n  if 1 | v | v 10 * else 2 | v | v 100 * end ;\n\n\
: main ( -- )\n  true pick .\n  false pick . ;\n",
    );
    assert_eq!(stdout, "10\n200\n");
    assert_eq!(code, 0);
}

#[test]
fn rebinding_a_name_in_scope_is_error() {
    // Criterion 5: forced for a linear value (the first binding would become
    // unreachable and could never be consumed), applied uniformly.
    let err = check_error(": w ( i64 -- i64 )\n  | a |\n  5 | a |\n  a ;\n");
    assert!(err.contains("already bound"), "unexpected message: {err}");
    assert!(err.contains("`a`"), "unexpected message: {err}");
    assert!(err.contains("line 3"), "the error should locate it: {err}");
}

#[test]
fn binding_more_values_than_frame_holds_is_error() {
    // Criterion 6: the existing needs-N-holds-M shape, naming the binding.
    let err = check_error(": w ( i64 -- i64 )\n  5 | a b c |\n  a b c + + ;\n");
    assert!(
        err.contains("`| a b c |` needs 3 values, but the stack holds 2"),
        "unexpected message: {err}"
    );
    assert!(err.contains("line 2"), "the error should locate it: {err}");
}

#[test]
fn binding_cannot_reach_beneath_declared_inputs() {
    // Criterion 7: `inner`'s frame is its one declared input, so its binding
    // cannot reach the `1` the caller left beneath it.
    let err = check_error(
        ": inner ( i64 -- i64 )\n  1 drop | a b |\n  a b + ;\n\
: main ( -- ) 1 2 inner . ;\n",
    );
    assert!(
        err.contains("needs 2 values, but the stack holds 1"),
        "unexpected message: {err}"
    );
}

#[test]
fn entry_binding_keeps_its_declared_input_diagnostic() {
    // Criterion 8: the entry position is the one place the declared effect is
    // the frame, so it keeps the message that cites it (R3) instead of
    // degrading to the generic underflow.
    let err = check_error(": w ( i64 -- i64 ) | a b | a ;");
    assert!(
        err.contains("locals bind 2 value(s), but only 1 input(s) are declared"),
        "unexpected message: {err}"
    );
}

#[test]
fn unconsumed_linear_local_errors_at_block_end() {
    // Criterion 9: the firing site is the arm's terminator, and the message
    // names it, so the fix ("consume it before then") has a location.
    let at_else = check_error(
        ": w ( bool -- )\n  if 7 __spy | s | 0 .\n  else 0 . end ;\n\
: main ( -- ) true w ;\n",
    );
    assert!(
        at_else.contains("linear value `s` is never consumed"),
        "unexpected message: {at_else}"
    );
    assert!(
        at_else.contains("scope ends at the `else` on line 3, col 3"),
        "unexpected message: {at_else}"
    );

    let at_end = check_error(
        ": w ( bool -- )\n  if 0 .\n  else 7 __spy | s | 0 .\n  end ;\n\
: main ( -- ) true w ;\n",
    );
    assert!(
        at_end.contains("scope ends at the `end` on line 4, col 3"),
        "unexpected message: {at_end}"
    );
}

#[test]
fn linear_local_bound_and_consumed_in_arm_is_accepted() {
    // Criterion 10: R6 adds a place where forgetting is caught, not a place
    // where something is dropped for you; disposing it in the arm is fine, and
    // the spy's destructor proves it ran exactly once.
    let (stdout, code) = run_src(
        "linear-local-consumed-in-arm",
        ": w ( bool -- )\n  if 7 __spy | s | s drop\n  else 0 . end ;\n\n\
: main ( -- )\n  true w\n  false w ;\n",
    );
    assert_eq!(stdout, "drop 7\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn empty_binding_with_no_names_is_error() {
    // Criterion 11: a stray pipe pair cannot silently mean nothing (R1).
    let err = parse_error(": w ( -- )\n  | | ;\n");
    assert!(err.contains("binds nothing"), "unexpected message: {err}");
    assert!(
        err.contains("line 2, col 3"),
        "the error should locate it: {err}"
    );
}

#[test]
fn goldens_still_compile_with_locals_as_a_term() {
    // The entry form is now an ordinary binding term (R3's unification), so the
    // Phase 0 goldens that use it are the regression check on that path.
    for example in ["examples/lerp.sth", "examples/gcd.sth"] {
        let src = std::fs::read_to_string(Path::new(example)).expect("example should be readable");
        check_ok(&src);
    }
}
