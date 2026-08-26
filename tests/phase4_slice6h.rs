//! Phase 4 slice 6h goldens: the `fill` zero-seed array constructor.
//! Each probe in `examples/array_ctor.sth` runs after a dirtier that fills a
//! nonzero seed into the same stack region and returns, so a zero read proves
//! `fill`'s store loop ran rather than fresh stack residue (`Alloc` never
//! zeroes). The dirtier is sized to each probe's own element width (`dirty`
//! for `i64` slots, `dirty_i8`/`dirty_bool` for the byte-granular ones),
//! since an `0 10 fill` dirtier's residue only lands on 8-byte-strided offsets
//! and never overlaps an `0 >i8 10 fill`/`False 4 fill` slot. The single
//! expected-output assertion covers the `0 10 fill`, `0 >i8 10 fill`+neighbour,
//! `False 4 fill` variant-0, after-the-loop (`terminated` reset), `times`-
//! composition, and combinator-body (D5) cases at once, since they share one
//! deterministic program.

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
        "0\n5\n0\nFalse\nFalse\nFalse\nFalse\n100\n0\n1\n2\n200\n5\n3\n2\n1\n0\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
