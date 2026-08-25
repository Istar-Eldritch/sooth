//! P7.S3s R9: differential-oracle harness skeleton for P7.S3o.
//!
//! S3o (`reject_user_bound_on_combinator`) is parked waiting for a concrete
//! program to test dispatch against. This slice ships one:
//! `examples/poly_if.sth`'s `mymax`/`mymax3`, both `'T: Copy Ord` bodies
//! forwarding to the library `gt`, non-inline (R5/R7). Once S3o restores
//! `inline` on a `Bound::User`-bounded combinator, flipping these two words
//! back to `inline` on the same source gives this harness a second variant to
//! diff its baseline against: stdout must be byte-identical (R7's own
//! exit criterion) and the resolved `impl: Ord` symbols reached at each
//! splice site must name the same `impl:` bodies, whether reached through a
//! real call frame (today) or a splice (S3o).
//!
//! Until S3o lands there is no second variant, so this test builds the one
//! source twice and diffs it against itself -- proving the plumbing (build,
//! run, `nm`) works and reports a clean diff before there is anything real to
//! compare, which is the point (R9): S3o inherits a mechanical diff, not a
//! design to invent from scratch.

mod common;

use std::path::Path;
use std::process::Command;

/// The `impl: Ord` bodies a build of `examples/poly_if.sth` links in, sorted
/// so two builds' symbol tables compare order-independently. Mangled names
/// carry `Ord` and `cmp` (the trait's sole member) verbatim
/// (`cmp.<mangled Ord>.<width>`), so a substring filter on `nm`'s own output
/// needs no knowledge of the mangling scheme itself.
fn ord_impl_symbols(binary: &Path) -> Vec<String> {
    let nm = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm should run");
    let mut names: Vec<String> = String::from_utf8_lossy(&nm.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .filter(|s| s.contains("Ord") && s.contains("cmp"))
        .collect();
    names.sort();
    names
}

/// Build `examples/poly_if.sth` fresh, run it, and read back its linked `Ord`
/// symbol table. Each call gets its own copy/binary (`common::build_example`),
/// so two calls in the same test never race each other.
fn build_run_and_ord_symbols() -> (String, Vec<String>) {
    let binary = common::build_example("examples/poly_if.sth");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    let symbols = ord_impl_symbols(&binary);
    std::fs::remove_file(&binary).ok();
    (String::from_utf8_lossy(&run.stdout).into_owned(), symbols)
}

/// The harness skeleton itself: two independent builds of the same source,
/// diffed on both axes S3o will need. A future S3o phase swaps the second
/// build for the same source with `inline` restored on `mymax`/`mymax3`;
/// nothing else about this test changes.
#[test]
fn poly_if_oracle_harness_reports_a_clean_diff_against_itself() {
    let (baseline_stdout, baseline_symbols) = build_run_and_ord_symbols();
    let (candidate_stdout, candidate_symbols) = build_run_and_ord_symbols();

    assert!(
        !baseline_symbols.is_empty(),
        "the harness must find real `impl: Ord` symbols to diff, or it is diffing nothing: \
         baseline stdout was {baseline_stdout:?}"
    );
    assert_eq!(
        baseline_stdout, candidate_stdout,
        "two builds of the same source must produce byte-identical stdout"
    );
    assert_eq!(
        baseline_symbols, candidate_symbols,
        "two builds of the same source must resolve the same `impl: Ord` symbols"
    );
}
