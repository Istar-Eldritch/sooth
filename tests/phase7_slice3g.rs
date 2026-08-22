//! Phase 7 Slice 3g goldens: self-recursion in a non-inline generic body.
//! The behavioural golden (a self-recursive `loopg` compiling, lowering and
//! running to the right result) lands in phase 2, once lowering's self-call
//! arm exists. Phase 1 lands the checker side: a structural operand mismatch
//! at the self-call site is a located type error, not a check-time loop or a
//! panic (D1); a call to a *different* polymorphic word still rejects with
//! `poly_calls_poly_word_error` unchanged (R3).

fn check_err(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    sooth::check::check(&mut module).expect_err("this program should be rejected")
}

/// Negative: a self-call whose operand window does not structurally match
/// the walking word's own `sig.inputs` is a located `type mismatch`, naming
/// the enclosing word, the self-call and the expected/found operand types --
/// never an infinite check-time loop and never a backend panic (D1's
/// termination witness).
#[test]
fn self_call_operand_mismatch_is_located_type_error() {
    let err = check_err(
        ": rec ( 'T i64 -- 'T )\n\
           drop true rec\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert_eq!(
        err,
        "error: type mismatch in `rec` (line 2)\n  `rec` expected `i64`, found `bool`\n  note: declared ( -- )",
        "{err}"
    );
}

/// Regression: a non-inline generic word calling a *second, different*
/// generic word still produces `poly_calls_poly_word_error` with its current
/// wording -- P7.S3k's gap, which this slice must not perturb (R3). Asserted
/// as a stable substring, not the full located string, since the diagnostic
/// interpolates a `(line, col)` that would otherwise bake a brittle position
/// into the golden.
#[test]
fn different_poly_word_call_still_names_the_narrowing() {
    let err = check_err(
        ": other ( 'T -- 'T ) ;\n\
         : caller ( 'T -- 'T ) other ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("cannot call the polymorphic word"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains(
            "a polymorphic word is not yet reachable from another polymorphic word across a module boundary"
        ),
        "unexpected message: {err}"
    );
}
