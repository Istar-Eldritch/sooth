//! Phase 4 Slice 13, phase 2 (R-B8): the shared read witness. `first`
//! declares a poly borrow (`&array['T 4]` is legal to *declare* since Part A;
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
    // Every slot holds a distinct value (10, 20, 30, 40), so this discriminates
    // *which* slot `first` reads (index 0) rather than merely succeeding --
    // a uniform-fill array (the prior version of this golden) cannot tell
    // reading slot 0 apart from slot 1, 2 or 3, a placebo hazard this project
    // has shipped before.
    assert_eq!(stdout, "10\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn setat_writes_an_array_element_through_a_poly_mutable_borrow_and_leaves_the_rest_alone() {
    // Phase 3 (R-B8): the write witness. `main` prefills four distinct
    // values, hands the array to the generic `setat`, and reads two slots
    // back: the one `setat` wrote (99) and an untouched neighbour (20). The
    // neighbour is what discriminates a store through the *element* borrow
    // from one that overwrote the array wholesale or wrote the wrong slot.
    // `setat` is then called again at `Vec2` (two i64 fields), which
    // discriminates a stride bug: a wrong element size would clobber the
    // neighbour's fields rather than merely writing the wrong scalar.
    let binary = common::build_example("examples/poly_borrow_setat.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert_eq!(stdout, "99\n20\n91\n92\n21\n22\n");
    assert_eq!(output.status.code(), Some(0));
}
