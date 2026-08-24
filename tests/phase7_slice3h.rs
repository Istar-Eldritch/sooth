//! P7.S3h phase 1 goldens: an escaping closure may capture a scalar-represented
//! value, even one spelled as an enum.
//!
//! `classify_capture`'s aggregate arm used to answer `FrameRooted` for every
//! `Struct`/`Enum`/`Array`/`OwnedCell` capture unconditionally, so a captured
//! `Bool` -- a payload-free, structurally-`Copy` enum since S3i -- was rejected
//! at every escaping boundary for being spelled as an enum rather than for
//! anything about its storage. The arm now splits on scalar representation:
//! a payload-free enum is a *value* in the one-word env slot and admits, while
//! a struct, an array and a payload-carrying enum are pointers into frame
//! storage and keep rejecting however `Copy` they are.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3c.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3h-{}-{tag}-{seq}", std::process::id()));
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
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    assert!(output.status.success(), "the built binary should exit 0");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn build_error(src: &Path) -> String {
    driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect_err("program should not build")
}

/// The motivating case, end to end: `mk` captures its `Bool` parameter into a
/// closure it returns, and the closure is called after `mk`'s frame is gone.
/// Both discriminants are threaded through so the assertion pins the captured
/// *value*, not merely that something built -- a snapshot that read the wrong
/// word would print one answer twice.
#[test]
fn escaping_closure_over_a_bool_local_admits_and_snapshots_it() {
    let prog = Scratch::write(
        "bool",
        ": mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;\n\
         : main ( -- ) True mk call . False mk call . ;\n",
    );
    assert_eq!(build_and_run(prog.path()), "True\nFalse\n");
}

/// The second `check_capture_admission` call site (`check_branch_join`), which
/// the return-boundary golden above never reaches: two *different* quotation
/// literals joining at a word tail, each capturing a `Bool` local of that
/// frame. Before this slice the join rejected at the first arm.
#[test]
fn branch_join_of_two_bool_capturing_arms_admits() {
    let prog = Scratch::write(
        "join",
        ": pick ( Bool Bool Bool -- [ -- Bool ] )\n  \
           | s a b |\n  \
           s ~[ [ a ] ] ~[ [ b ] ] if\n\
         ;\n\
         : main ( -- )\n  \
           True False True pick call .\n  \
           False False True pick call .\n\
         ;\n",
    );
    assert_eq!(build_and_run(prog.path()), "False\nTrue\n");
}

/// The narrowing's guard on the enum side: `Item` is `Copy` (its one payload
/// field is an `i64`), so an `is_copy`-only predicate would admit it -- but a
/// payload-carrying enum lives in tagged storage reached by pointer, and
/// snapshotting that pointer into the env would outlive the frame it points
/// into. `escaping_closure_over_frame_local_is_past_owning_frame`
/// (`tests/phase4_quotations.rs`) is the array-shaped twin of this, unchanged
/// by the slice.
#[test]
fn escaping_closure_over_a_payload_carrying_enum_local_still_rejects() {
    let prog = Scratch::write(
        "payload",
        "type: Item | Empty | Full v i64 ;\n\
         : mk ( Item -- [ -- Item ] ) | e | [ e ] ;\n\
         : main ( -- ) Empty mk call | r | r ~[ 1 . ] ~[ 0 . ] if ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: an escaping closure captures `e`, a local of this frame, whose storage does not survive the return (line 2)"
    );
}

/// The narrowing's guard on the struct side. `P` is all-`i64`, so it is `Copy`
/// too, and it is still pointer-backed: `is_aggregate` is unconditionally true
/// for a struct.
#[test]
fn escaping_closure_over_a_copy_struct_local_still_rejects() {
    let prog = Scratch::write(
        "struct",
        "type: P x i64 y i64 ;\n\
         : mk ( -- [ -- i64 ] ) 1 2 P | p | [ p .x ] ;\n\
         : main ( -- ) mk call . ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: an escaping closure captures `p`, a local of this frame, whose storage does not survive the return (line 2)"
    );
}
