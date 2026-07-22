//! Phase 0 golden tests: the exit criterion is that `gcd` and `factorial` compile to a
//! standalone native binary and run correctly; `lerp` additionally covers named locals.
//! Ignored until the pipeline lands.

#[test]
#[ignore = "Phase 0 pipeline not implemented yet"]
fn gcd_compiles_and_runs() {
    // build examples/gcd.sth, run with (a, b), assert gcd output.
}

#[test]
#[ignore = "Phase 0 pipeline not implemented yet"]
fn factorial_compiles_and_runs() {
    // build examples/factorial.sth, run with n, assert factorial output.
}

#[test]
#[ignore = "Phase 0 pipeline not implemented yet"]
fn lerp_compiles_and_runs() {
    // build examples/lerp.sth (the locals golden), assert stdout == "30\n".
}
