//! Phase 4 slice 6h goldens: the raw array constructor `[ Type ; Count ]`.
//! Each probe in `examples/array_ctor.sth` runs after a `dirty` preamble that
//! fills a same-or-larger array with a nonzero seed and returns, so a zero read
//! proves the constructor's zero-init loop ran rather than fresh stack residue
//! (`Alloc` never zeroes). The single expected-output assertion covers the
//! `[i64;10]`, `[i8;10]`+neighbour, `[bool;4]` variant-0, after-the-loop
//! (`terminated` reset), `times`-composition, and combinator-body (D5) cases at
//! once, since they share one deterministic program.

use std::process::Command;

mod common;

#[test]
fn array_constructor_zero_inits_across_the_probe_suite() {
    let binary = common::build_example("examples/array_ctor.sth");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    // probe10 (0); probe_i8 neighbour then value (5, 0); probe_bool variant 0
    // four times; probe_after's after-the-loop 100, the times' 0 1 2, then 200;
    // withbuf's index-0 zero plus f's 5; recur's tail-recursive 3 2 1 0.
    assert_eq!(
        stdout,
        "0\n5\n0\nfalse\nfalse\nfalse\nfalse\n100\n0\n1\n2\n200\n5\n3\n2\n1\n0\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
