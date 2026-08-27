//! Phase 7 Slice 5 Phase 4 goldens: the synthesized array destructor.
//!
//! R11: `tabulate` builds a linear array, the array is dropped whole, and
//! each element is disposed exactly once via the synthesized destructor.
//! R12: `None 3 fill` builds a nullary-variant linear-enum array, dropped,
//! disposing the (empty) `None` slots as a no-payload discriminant write — no
//! leaked linear data, since there is none in a `None` slot.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s5-{tag}-{seq}", tag = tag, seq = seq));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, common::fixture_source("prog.sth", contents)).unwrap();
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

fn build_and_run(src: &Path) -> String {
    let binary = driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .unwrap_or_else(|e| panic!("program should build: {e}"));
    let output = Command::new(&binary).output().expect("binary should run");
    assert!(output.status.success(), "the built binary should exit 0");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// A forced-linear struct with an observable `drop`, the shape the linear core
/// tests use: `drop` prints `"drop "` then the tag, so a leak is silent but a
/// double-dispose prints twice.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | \"drop \" . s Spy> . ;\n";

/// R11: `tabulate` builds a `array[Spy 2]` (a linear array), and dropping it
/// disposes both `Spy` elements exactly once via the synthesized array
/// destructor (R6/R7). Each `Spy`'s `drop` prints `"drop " <tag>`, so the
/// golden output is two disposal lines — the witness that the destructor
/// loop ran and each element was consumed.
#[test]
fn tabulate_builds_linear_array_dropped_whole_disposes_each_element() {
    let prog = Scratch::write(
        "tabulate-drop",
        &format!("{SPY_DEF}: main ( -- )\n  2 ~[ 7 Spy ] tabulate drop ;\n"),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\ndrop 7\n");
}

/// R11 (separate): a `Spy` with a different tag in each position would need
/// the index, which `tabulate`'s quotation does not receive — but a distinct
/// *construction* is enough to prove each slot is a separate value. Two
/// separate `tabulate`/`drop` cycles with different tags confirm the
/// destructor runs per-element, not once for the whole array.
#[test]
fn tabulate_linear_array_drop_disposes_in_order() {
    let prog = Scratch::write(
        "tabulate-order",
        &format!(
            "{SPY_DEF}: main ( -- )\n  1 ~[ 3 Spy ] tabulate drop\n  1 ~[ 9 Spy ] tabulate drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 3\ndrop 9\n");
}

/// R11: a single-element linear array (N=1, the unrolled case) still
/// disposes its one element when dropped — the destructor's loop runs once.
#[test]
fn tabulate_single_element_linear_array_dropped_disposes_once() {
    let prog = Scratch::write(
        "tabulate-one",
        &format!("{SPY_DEF}: main ( -- )\n  1 ~[ 42 Spy ] tabulate drop ;\n"),
    );
    assert_eq!(build_and_run(prog.path()), "drop 42\n");
}

/// R12: `None 3 fill` builds an `array[Opt 3]` where `Opt` is linear (because
/// `Some` carries a `Spy`), and every slot is `None` (a nullary variant with
/// no payload). Dropping the array runs the synthesized destructor, which
/// calls each `Opt`'s enum destructor; the enum destructor dispatches on the
/// tag and finds `None` (no linear fields), so no `Spy` is disposed and the
/// output is empty — no leaked linear data, since there is none in a `None`
/// slot.
#[test]
fn fill_nullary_variant_linear_enum_array_dropped_disposes_nothing() {
    let prog = Scratch::write(
        "fill-none-drop",
        &format!(
            "{SPY_DEF}type: Opt | None | Some val Spy ;\n: main ( -- )\n  None 3 fill drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "");
}

/// R12 (mixed): an `array[Opt 3]` built via `tabulate` where every slot is
/// `Some(5 Spy)` — the enum destructor is called per element (the array
/// destructor's loop calls `emit_drop` on each, which dispatches to the enum
/// destructor), proving the array destructor calls the *enum* destructor
/// (which tag-dispatches) rather than assuming a uniform scalar shape.
#[test]
fn tabulate_enum_array_dropped_disposes_each_payload() {
    let prog = Scratch::write(
        "tabulate-enum",
        &format!(
            "{SPY_DEF}type: Opt | None | Some val Spy ;\n\
             : main ( -- )\n  3 ~[ 5 Spy Some ] tabulate drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 5\ndrop 5\ndrop 5\n");
}

/// R9 (non-regression): a non-linear array (`array[i64 3]`) dropped is a no-op —
/// `emit_drop`'s `_ => {}` arm, unchanged. The `fill`/`len`/array-type paths
/// compile and run identically.
#[test]
fn non_linear_array_drop_is_no_op() {
    let prog = Scratch::write("fill-copy-drop", ": main ( -- )\n  0 3 fill drop 99 . ;\n");
    assert_eq!(build_and_run(prog.path()), "99\n");
}
