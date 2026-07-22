//! Phase 0 golden tests: the exit criterion is that these two programs compile to a
//! standalone native binary and run correctly. Ignored until the pipeline lands.

#[test]
#[ignore = "Phase 0 pipeline not implemented yet"]
fn gcd_compiles_and_runs() {
    // build examples/gcd.sooth, run with (a, b), assert gcd output.
}

#[test]
#[ignore = "Phase 0 pipeline not implemented yet"]
fn factorial_compiles_and_runs() {
    // build examples/factorial.sooth, run with n, assert factorial output.
}
