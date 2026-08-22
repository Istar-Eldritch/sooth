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
/// never an infinite check-time loop and never a backend panic. This is D1's
/// actual termination witness: `'T` recursing at `Box['T]` is the shape the
/// roadmap's polymorphic-recursion hazard is about (a self-call at a
/// *different* type argument, which under monomorphizing codegen would
/// demand a fresh instantiation per level). An ordinary concrete-vs-concrete
/// mismatch (e.g. `bool` vs `i64`) exercises the same code path but is not
/// evidence for D1 specifically, since it says nothing about recursion at a
/// different type argument.
#[test]
fn self_call_recursing_at_a_different_type_argument_is_located_type_error() {
    let err = check_err(
        "type: Box 'T | Box 'T ;\n\
         : rec ( 'T i64 -- 'T )\n\
           drop Box 3 rec\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert_eq!(
        err,
        "error: type mismatch in `rec` (line 3)\n  `rec` expected `'T`, found `Box['T]`\n  note: declared ( -- )",
        "{err}"
    );
}

/// Negative: an ordinary concrete-vs-concrete operand mismatch ahead of a
/// self-call is likewise a located `type mismatch`, not a bespoke self-call
/// diagnostic -- the self-call arm reuses the same per-slot comparison any
/// other operand/signature mismatch goes through.
#[test]
fn self_call_concrete_operand_mismatch_is_located_type_error() {
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

#[test]
fn tmp_probe_add() {
    let err = check_err(
        ": add ( 'T: Copy 'T i64 -- 'T )\n\
           drop 3 add\n\
         ;\n\
         : main ( -- ) 1 3 add drop ;\n",
    );
    panic!("{err}");
}
