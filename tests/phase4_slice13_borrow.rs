//! Phase 4 Slice 13, phase 2 (R-B8): the shared read witness. `first`
//! declares a poly borrow (`&['T 4]` is legal to *declare* since Part A;
//! Part B teaches the checker to *produce* one), borrows its array local,
//! bounds-checks a literal index against the concrete length, and fetches
//! the element through `@` -- all still generic, monomorphized at `i64` by
//! this call, which is also the "concrete twin" R-B8 asks the golden to
//! exercise (there is exactly one instantiation here, and it both checks
//! and lowers).

mod common;

use std::process::Command;

#[test]
fn first_reads_an_array_element_through_a_poly_borrow_and_prints_it() {
    let binary = common::build_example("examples/poly_borrow_first.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    // The computed value (`10 4 fill` seeds every slot with 10, `first`
    // reads slot 0), not merely a successful exit -- a placebo hazard this
    // project has shipped before.
    assert_eq!(stdout, "10\n");
    assert_eq!(output.status.code(), Some(0));
}
