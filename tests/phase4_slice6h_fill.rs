//! Phase 4 slice 6h: `fill`'s re-lowering goldens. `examples/fill_relower.sth`
//! runs a scalar seed and a struct (aggregate) seed both reaching the array's
//! last slot, and a `fill` inside a self-tail loop. The last two are the loop's
//! new exposure: `fill` never opened a loop before, so the save/restore
//! loop-state hygiene and the `terminated` reset are exercised here for the
//! first time (a broken self-tail seal panics; a missing `terminated` reset
//! drops the terms after `fill`, so the countdown would print nothing).

use std::process::Command;

mod common;

#[test]
fn fill_relowering_scalar_struct_and_tail_recursive() {
    let binary = common::build_example("examples/fill_relower.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    // scalar_last: 42; struct_last `. .` prints the last slot's fields top-
    // first (b=4 then a=3); countdown: 3 2 1 0 after the tail-recursive fill.
    assert_eq!(stdout, "42\n4\n3\n3\n2\n1\n0\n");
    assert_eq!(output.status.code(), Some(0));
}
