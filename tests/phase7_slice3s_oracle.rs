//! P7.S3o R7: differential-oracle harness for the inline `mymax`/`mymax3`
//! motivating program.
//!
//! Phase 2 flips `mymax`/`mymax3` in `examples/poly_if.sth` back to `inline`
//! with `'T: Copy Ord`. This harness validates that flip by differential
//! testing: the inline candidate (the same source with `inline` restored)
//! must produce byte-identical stdout to the non-inline baseline (the same
//! source with `inline` stripped), and a `gt` -> `lt` swap control on each
//! side must change that side's own stdout -- proving a comparison-direction
//! swap stays visible regardless of which symbols either side happens to
//! mint (P7.S8 phase 1; see review round 4 below).
//!
//! A fixture calling *both* `mymax` and `mymax3` at two types (i64 and f64)
//! is needed because `examples/poly_if.sth`'s `main` calls `mymax3` only;
//! `mymax` mints no monomorph and the harness never sees it. The fixture is
//! generated at test time with `inline` on or off, so the two variants share
//! one source of truth and cannot drift.
//!
//! Review round 1 (Phase 3): the first cut of this harness filtered `nm`'s
//! whole-binary output for any symbol containing `Ord` and `cmp`. That is a
//! property of `lib/cmp.sth` being linked in at all, not of which `impl:`
//! any particular splice site actually dispatches to. Fixed by walking the
//! actual call graph (`objdump -d`) out of `sooth_main`, so the diff is over
//! *resolved dispatch targets*, not *what the module happens to link*.
//!
//! Review round 2: the reached *`impl:`* set alone still cannot see a
//! comparison swap. Every surface comparison in `lib/cmp.sth` funnels
//! through the one `cmp` impl per type, so a build with every `gt` swapped
//! to `lt` reaches the same `cmp.<Ord>.<width>` symbol and produces a
//! byte-identical map. The walk therefore records the monomorphized
//! comparison words it passes through (`sooth_mono_gt__*` vs
//! `sooth_mono_lt__*`) as well, which is what makes the swap visible.
//!
//! Review round 3 (S3o Phase 2): the non-inline baseline has `sooth_mono_
//! mymax*` call-frame functions that the inline candidate eliminates by
//! splicing. Those are structural, not dispatch, so they are filtered out
//! of the diff: the comparison is over the `gt` monomorphs and `impl: Ord`
//! bodies that both versions must reach, not the call frames one version
//! happens to inline away.
//!
//! Review round 4 (P7.S8 phase 1): P7.S8 inlines the six surface comparisons
//! themselves. Once `gt` is `inline` too, a call to it from an already-inline
//! `mymax` collapses to a pure lowering-time splice that mints no symbol at
//! all, so "both versions reach the monomorphized `gt`" and "the two
//! versions' dispatch-target sets are equal" both go permanently false --
//! not an edge case, a structural certainty once the caller and callee are
//! both combinators. Both assertions are replaced by a `gt` -> `lt` swap
//! control applied independently to each side (`fixture_source_with_cmp`):
//! whatever either side mints or doesn't, swapping the comparison direction
//! must change that side's own stdout, or the harness is diffing a program
//! against itself. The byte-identical stdout comparison between the two
//! unswapped sides, and the i64/f64 monomorph-presence checks on the
//! baseline, are unaffected and stay.

mod common;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use common::call_graph;

/// The dispatch targets transitively reachable from `entry` by following
/// `graph`'s call edges (BFS, cycle-safe via `seen`): the monomorphized
/// comparison words reached on the way there. A monomorph carries the
/// `sooth_mono_` prefix, so a substring filter needs no knowledge of the
/// mangling scheme itself -- only the walk needs to start from the entry
/// point under test rather than the whole binary.
///
/// P7.S3s-follow: `cmp` is an `inline` trait member, so the `impl: Ord`
/// bodies are spliced into the comparison monomorphs and mint no symbol of
/// their own. The walk therefore no longer collects `impl: Ord` symbols
/// (they are unreachable in the call graph); the comparison monomorphs,
/// which carry `cmp`'s body inlined, are the dispatch targets that remain.
///
/// `sooth_mono_mymax*` symbols are excluded: they are the call frames the
/// non-inline baseline emits and the inline candidate eliminates by splicing.
/// They are structural, not dispatch, so including them would make the diff
/// non-constant by design rather than revealing a miscompile.
fn reached_dispatch_targets(graph: &HashMap<String, Vec<String>>, entry: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([entry.to_string()]);
    let mut hits = Vec::new();
    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let is_mono = name.starts_with("sooth_mono_") && !name.starts_with("sooth_mono_mymax");
        if is_mono {
            hits.push(name.clone());
        }
        if let Some(callees) = graph.get(&name) {
            queue.extend(callees.iter().cloned());
        }
    }
    hits.sort();
    hits
}

/// Walk `binary`'s call graph from `sooth_main` and return the sorted set of
/// dispatch targets reached (comparison monomorphs, excluding `mymax*`
/// call frames).
fn main_dispatch_targets(binary: &Path) -> Vec<String> {
    let graph = call_graph(binary);
    reached_dispatch_targets(&graph, "sooth_main")
}

/// The fixture source: `mymax` and `mymax3` with `'T: Copy Ord`, plus a
/// `main` calling both words at two types (i64 and f64). When `inline` is
/// true, both words are declared `inline`; otherwise they are ordinary poly
/// words with a real call frame per instantiation. The source carries its
/// own imports so it builds against the fixture manifest without
/// `fixture_source` auto-injection.
fn fixture_source(inline: bool) -> String {
    fixture_source_with_cmp(inline, "gt")
}

/// `fixture_source` parameterized over which comparison `mymax`/`mymax3` call,
/// so the swap control (`gt` -> `lt`, picking the min instead of the max) can
/// share this source of truth rather than a hand-duplicated string.
fn fixture_source_with_cmp(inline: bool, cmp: &str) -> String {
    let kw = if inline { "inline " } else { "" };
    format!(
        "\
import: intrinsics * ;
import: core::prelude * ;

: mymax {kw}['T: Copy Ord] ( 'T 'T -- 'T ) over over {cmp} ~[ drop ] ~[ swap drop ] if ;

: choose inline ( 'T 'T Bool -- 'T ) | a b flag | flag ~[ a b drop ] ~[ b a drop ] if ;

: mymax3 {kw}['T: Copy Ord] ( 'T 'T 'T -- 'T )
  | a b c |
  a b {cmp} ~[
    a c {cmp} ~[ a ] ~[ c ] if
  ] ~[
    b c {cmp} ~[ b ] ~[ c ] if
  ] if ;

: main ( -- )
  2 5 mymax .
  2 5 9 mymax3 .
  2.5 5.5 mymax .
  2.5 5.5 9.5 mymax3 . ;
"
    )
}

/// A scratch single-file program written to a temp directory, removed on
/// drop. The temp directory has no ancestor `sooth.pkg`, so the fixture
/// manifest (`tests/fixtures/sooth.pkg`) is passed via `--manifest`.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, src: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3o-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creating temp dir");
        let path = dir.join("prog.sth");
        std::fs::write(&path, format!("{src}{}", common::printing_import(src)))
            .expect("writing fixture source");
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

/// Build `src`, run it, and read back the dispatch targets reached from
/// `sooth_main`. Returns `(stdout, dispatch_targets)`.
fn build_run_and_dispatch_targets(src: &str, tag: &str) -> (String, Vec<String>) {
    let scratch = Scratch::write(tag, src);
    let binary = sooth::driver::build_with_manifest(
        scratch.path(),
        common::manifest_for(scratch.path()).as_deref(),
    )
    .unwrap_or_else(|e| panic!("building {tag}: {e}"));
    let run = Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("running {tag}: {e}"));
    assert!(
        run.status.success(),
        "{tag} exited {}; stderr: {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    let targets = main_dispatch_targets(&binary);
    std::fs::remove_file(&binary).ok();
    (String::from_utf8_lossy(&run.stdout).into_owned(), targets)
}

/// R7: the inline candidate (`mymax`/`mymax3` with `inline`) must produce
/// byte-identical stdout as the non-inline baseline (the same source without
/// `inline`), at two splices of `mymax3` and two types (i64 and f64), with
/// `mymax` also called at both types so it mints monomorphs in the baseline.
#[test]
fn inline_mymax_mymax3_matches_noninline_baseline() {
    let baseline_src = fixture_source(false);
    let candidate_src = fixture_source(true);

    let (baseline_stdout, baseline_targets) =
        build_run_and_dispatch_targets(&baseline_src, "noninline-baseline");
    let candidate_stdout = build_run_and_dispatch_targets(&candidate_src, "inline-candidate").0;

    // R8: stdout is byte-identical.
    assert_eq!(
        baseline_stdout, candidate_stdout,
        "inline and non-inline must produce byte-identical stdout"
    );

    // The fixture calls both words at two types, so the output has four
    // lines: mymax i64, mymax3 i64, mymax f64, mymax3 f64. The f64 operands
    // are fractional on purpose (P7b.S3 review, S5): `5.5`/`9.5` are values
    // only the f64 path can print, so the two-types reach is witnessed by
    // stdout itself, not by monomorph symbols a diverted `inline` combinator
    // no longer mints.
    assert_eq!(
        baseline_stdout, "5\n9\n5.5\n9.5\n",
        "the fixture should print max(2,5)=5, max(2,5,9)=9, max(2.5,5.5)=5.5, max(2.5,5.5,9.5)=9.5"
    );

    // The harness must find dispatch targets, or it is diffing nothing.
    assert!(
        !baseline_targets.is_empty(),
        "the harness must find dispatch targets from `sooth_main`, or it is diffing \
         nothing: baseline stdout was {baseline_stdout:?}"
    );

    // P7.S8 phase 1: `sooth_mono_gt` reachability and dispatch-target-set
    // equality (`baseline_targets == candidate_targets`) stop being usable
    // discriminators once the library inlines its comparisons: an inline
    // `mymax` calling an inline `gt` collapses to a pure lowering-time
    // splice that mints no symbol at all, so the candidate side would mint
    // zero comparison monomorphs while the baseline still mints its own real
    // ones -- the equality goes permanently false, not an edge case a
    // symbol-name check happens to expose. A `gt` -> `lt` swap control on
    // *each* side, independent of what either side mints, is what proves a
    // comparison-direction swap stays visible.
    let baseline_swap_stdout = build_run_and_dispatch_targets(
        &fixture_source_with_cmp(false, "lt"),
        "noninline-baseline-swap",
    )
    .0;
    assert_ne!(
        baseline_stdout, baseline_swap_stdout,
        "swapping `gt` for `lt` must change the non-inline baseline's stdout, or the \
         harness cannot tell a comparison-direction miscompile from a correct build"
    );
    let candidate_swap_stdout = build_run_and_dispatch_targets(
        &fixture_source_with_cmp(true, "lt"),
        "inline-candidate-swap",
    )
    .0;
    assert_ne!(
        candidate_stdout, candidate_swap_stdout,
        "swapping `gt` for `lt` must change the inline candidate's stdout, or the \
         harness cannot tell a comparison-direction miscompile from a correct build"
    );

    // R10 (amended by P7b.S3, S3-1.d): the comparison monomorphs used to be
    // the two-types witness, but a diverted `inline` combinator instantiation
    // (`gt` at i64 and f64) mints no symbol on either side now -- the
    // two-types reach is witnessed behaviourally by the stdout assertion
    // above, whose `5.5`/`9.5` lines only an f64 comparison path produces
    // (the migrated form of the deleted i64/f64 monomorph-presence checks),
    // and the swap controls keep the comparison identity observable.
}
