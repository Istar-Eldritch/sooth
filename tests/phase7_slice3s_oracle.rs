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
//! run, `nm`/`objdump`) works and reports a clean diff before there is
//! anything real to compare, which is the point (R9): S3o inherits a
//! mechanical diff, not a design to invent from scratch.
//!
//! Review round 1 (Phase 3): the first cut of this harness filtered `nm`'s
//! whole-binary output for any symbol containing `Ord` and `cmp`. That is a
//! property of `lib/cmp.sth` being linked in at all, not of which `impl:`
//! any particular splice site actually dispatches to -- a build with every
//! comparison in `examples/poly_if.sth` swapped (`gt` -> `lt`) links exactly
//! the same set of defined symbols. Fixed by keying the diff per entry point
//! (`sooth_mono_mymax*`) and walking the actual call graph (`objdump -d`) out
//! of each one to the `impl: Ord` symbol it resolves to, so the diff is over
//! *resolved dispatch targets*, not *what the module happens to link*.
//!
//! `mymax` itself (as opposed to `mymax3`) is never called from
//! `examples/poly_if.sth`'s `main`, so it mints no monomorph and this harness
//! never sees it -- a pre-existing R3 dead-code residual. Not fixed here:
//! `tests/corpus_stdout/poly_if.txt` must stay byte-identical (R7), which
//! rules out adding a call in the example itself, and inventing a second
//! harness-local fixture is S3o's own scope, not this skeleton's. Left as a
//! recommendation for whichever S3o phase actually diffs a splice variant:
//! it will need a source calling both words to exercise the "at two splices,
//! at three" wording in the R9 spec text at all.

mod common;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Command;

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

/// The `impl: Ord` symbols transitively reachable from `entry` by following
/// `graph`'s call edges (BFS, cycle-safe via `seen`). Mangled `impl:` names
/// carry `Ord` and `cmp` (the trait's sole member) verbatim
/// (`cmp.<mangled Ord>.<width>`), so a substring filter on the reached set
/// needs no knowledge of the mangling scheme itself -- only the walk needs
/// to start from the entry point under test rather than the whole binary.
fn reached_ord_impls(graph: &HashMap<String, Vec<String>>, entry: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([entry.to_string()]);
    let mut hits = Vec::new();
    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if name.contains("Ord") && name.contains("cmp") {
            hits.push(name.clone());
        }
        if let Some(callees) = graph.get(&name) {
            queue.extend(callees.iter().cloned());
        }
    }
    hits.sort();
    hits
}

/// Every `sooth_mono_mymax*` entry point in `binary`'s symbol table (one per
/// instantiation of `mymax`/`mymax3`, however many `main` calls into), mapped
/// to the `impl: Ord` symbols its own call graph resolves to.
fn mymax_entry_ord_impls(binary: &Path) -> BTreeMap<String, Vec<String>> {
    let nm = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm should run");
    let entries: Vec<String> = String::from_utf8_lossy(&nm.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .filter(|s| s.starts_with("sooth_mono_mymax"))
        .collect();
    let graph = call_graph(binary);
    entries
        .into_iter()
        .map(|entry| {
            let impls = reached_ord_impls(&graph, &entry);
            (entry, impls)
        })
        .collect()
}

/// Build `examples/poly_if.sth` fresh, run it, and read back each `mymax*`
/// splice site's resolved `impl: Ord` symbols. Each call gets its own
/// copy/binary (`common::build_example`), so two calls in the same test
/// never race each other.
fn build_run_and_ord_symbols() -> (String, BTreeMap<String, Vec<String>>) {
    let binary = common::build_example("examples/poly_if.sth");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    let symbols = mymax_entry_ord_impls(&binary);
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
        "the harness must find at least one `mymax*` splice site to diff, or it is diffing \
         nothing: baseline stdout was {baseline_stdout:?}"
    );
    for (entry, impls) in &baseline_symbols {
        assert!(
            !impls.is_empty(),
            "{entry} must resolve to at least one `impl: Ord` symbol through its own call \
             graph, not just link one somewhere in the binary"
        );
    }
    assert_eq!(
        baseline_stdout, candidate_stdout,
        "two builds of the same source must produce byte-identical stdout"
    );
    assert_eq!(
        baseline_symbols, candidate_symbols,
        "two builds of the same source must resolve the same `impl: Ord` symbols at each \
         `mymax*` splice site"
    );
}
