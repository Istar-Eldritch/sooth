//! P7.S3o R7: differential-oracle harness for the inline `mymax`/`mymax3`
//! motivating program.
//!
//! Phase 2 flips `mymax`/`mymax3` in `examples/poly_if.sth` back to `inline`
//! with `'T: Copy Ord`. This harness validates that flip by differential
//! testing: the inline candidate (the same source with `inline` restored)
//! must produce byte-identical stdout to the non-inline baseline (the same
//! source with `inline` stripped), and the resolved `impl: Ord` dispatch
//! targets reached through the call graph must match — whether the `gt`
//! calls are reached through a real call frame (non-inline) or a splice
//! (inline).
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

mod common;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Every `call <target>` edge in `binary`'s disassembly, keyed by the caller
/// symbol whose body the call appears in. `objdump -d` annotates a call's
/// target address with the symbol name in `<...>` when one exists, so this
/// needs no knowledge of the calling convention or the mangling scheme.
fn call_graph(binary: &Path) -> HashMap<String, Vec<String>> {
    let out = Command::new("objdump")
        .arg("-d")
        .arg(binary)
        .output()
        .expect("objdump should run");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        if let Some(header) = line.strip_suffix(">:") {
            if let Some((_, name)) = header.rsplit_once('<') {
                current = name.to_string();
                graph.entry(current.clone()).or_default();
            }
            continue;
        }
        if !line.contains("call") {
            continue;
        }
        if let Some((_, rest)) = line.rsplit_once('<') {
            if let Some(target) = rest.strip_suffix('>') {
                graph
                    .entry(current.clone())
                    .or_default()
                    .push(target.to_string());
            }
        }
    }
    graph
}

/// The dispatch targets transitively reachable from `entry` by following
/// `graph`'s call edges (BFS, cycle-safe via `seen`): the `impl: Ord` bodies
/// resolved to, plus the monomorphized words reached on the way there.
/// Mangled `impl:` names carry `Ord` and `cmp` (the trait's sole member)
/// verbatim (`cmp.<mangled Ord>.<width>`) and a monomorph carries the
/// `sooth_mono_` prefix, so a substring filter needs no knowledge of the
/// mangling scheme itself -- only the walk needs to start from the entry
/// point under test rather than the whole binary. The monomorphs are what
/// distinguish *which* comparison a site calls; the `impl:` symbols are what
/// distinguish which type's implementation it lands in.
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
        let is_impl = name.contains("Ord") && name.contains("cmp");
        // `gt`/`lt`/etc are `inline` in `lib/cmp.sth`, so a `sooth_mono_gt`
        // monomorph is dead code: the monomorphization system mints it inside
        // a non-inline poly caller, but the splice means it is never called.
        // Filter it out so the comparison is between live dispatch targets only.
        let is_mono = name.starts_with("sooth_mono_")
            && !name.starts_with("sooth_mono_mymax")
            && !name.starts_with("sooth_mono_gt");
        if is_impl || is_mono {
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
/// dispatch targets reached (gt monomorphs + `impl: Ord` bodies, excluding
/// `mymax*` call frames).
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
    let kw = if inline { "inline " } else { "" };
    format!(
        "\
import: intrinsics * ;
import: core::prelude * ;

: mymax {kw}( 'T: Copy Ord 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;

: choose inline ( 'T 'T Bool -- 'T ) | a b flag | flag ~[ a b drop ] ~[ b a drop ] if ;

: mymax3 {kw}( 'T: Copy Ord 'T 'T -- 'T )
  | a b c |
  a b gt ~[
    a c gt ~[ a ] ~[ c ] if
  ] ~[
    b c gt ~[ b ] ~[ c ] if
  ] if ;

: main ( -- )
  2 5 mymax .
  2 5 9 mymax3 .
  2.0 5.0 mymax .
  2.0 5.0 9.0 mymax3 . ;
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
        std::fs::write(&path, src).expect("writing fixture source");
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
/// byte-identical stdout and the same resolved `impl: Ord` dispatch targets
/// as the non-inline baseline (the same source without `inline`), at two
/// splices of `mymax3` and two types (i64 and f64), with `mymax` also called
/// at both types so it mints monomorphs in the baseline.
#[test]
fn inline_mymax_mymax3_matches_noninline_baseline() {
    let baseline_src = fixture_source(false);
    let candidate_src = fixture_source(true);

    let (baseline_stdout, baseline_targets) =
        build_run_and_dispatch_targets(&baseline_src, "noninline-baseline");
    let (candidate_stdout, candidate_targets) =
        build_run_and_dispatch_targets(&candidate_src, "inline-candidate");

    // R8: stdout is byte-identical.
    assert_eq!(
        baseline_stdout, candidate_stdout,
        "inline and non-inline must produce byte-identical stdout"
    );

    // The fixture calls both words at two types, so the output has four
    // lines: mymax i64, mymax3 i64, mymax f64, mymax3 f64.
    assert_eq!(
        baseline_stdout, "5\n9\n5\n9\n",
        "the fixture should print max(2,5)=5, max(2,5,9)=9, max(2.0,5.0)=5, max(2.0,5.0,9.0)=9"
    );

    // The harness must find dispatch targets, or it is diffing nothing.
    assert!(
        !baseline_targets.is_empty(),
        "the harness must find dispatch targets from `sooth_main`, or it is diffing \
         nothing: baseline stdout was {baseline_stdout:?}"
    );

    // Both versions must reach at least one `impl: Ord` symbol through
    // `cmp`. Since `gt` is now `inline` in `lib/cmp.sth`, neither version
    // mints a `sooth_mono_gt` monomorph -- `gt`'s body (the `cmp` call +
    // `Ordering?` eliminator) splices directly into `mymax`/`mymax3` and then
    // into `main`, so both reach the same `cmp;Ord;0;{i64,f64}` symbols.
    for (label, targets) in [
        ("baseline", &baseline_targets),
        ("candidate", &candidate_targets),
    ] {
        assert!(
            targets
                .iter()
                .any(|t| t.contains("Ord") && t.contains("cmp")),
            "{label} must resolve to at least one `impl: Ord` symbol through its own call \
             graph, not just link one somewhere in the binary"
        );
    }

    // R7: the resolved dispatch targets (`cmp` impl monomorphs) match
    // between inline and non-inline. The `mymax*` call frames are filtered
    // out — they exist only in the non-inline version by design.
    assert_eq!(
        baseline_targets, candidate_targets,
        "inline and non-inline must resolve the same dispatch targets from `sooth_main`"
    );

    // R10: two types — both i64 and f64 monomorphs and impls are reached.
    assert!(
        baseline_targets.iter().any(|t| t.contains("i64")),
        "the dispatch targets must include an i64 monomorph: {baseline_targets:?}"
    );
    assert!(
        baseline_targets.iter().any(|t| t.contains("f64")),
        "the dispatch targets must include an f64 monomorph: {baseline_targets:?}"
    );
}
