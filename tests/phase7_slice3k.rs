//! P7.S3k goldens: a non-inline generic word calling *another* generic word.
//!
//! Before this slice every such call -- same-module or imported, user-declared
//! or a library word -- was a located rejection
//! (`poly_calls_poly_word_error`). It grounds now: the callee's declared
//! signature is fetched from the same `poly_env` a monomorphic caller
//! dispatches through, and its rigid type variables are related to the
//! caller's symbolically, since at check time the caller has no `θ` either.
//!
//! Phase 1 covers the callees lowering **splices** (an `inline` library word
//! like `lt`/`gt`), which need no monomorph of their own and so work
//! end-to-end already. A non-inline generic callee needs one composed
//! instantiation per concrete type the caller reaches; that is phase 2, and
//! its goldens land here beside these.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

/// A scratch tree of `.sth` files outside any package, removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3k-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let out = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(out.status.code(), Some(0), "binary should exit clean");
    String::from_utf8(out.stdout).expect("stdout should be utf8")
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        !build.status.success(),
        "build should have failed; stdout: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    String::from_utf8_lossy(&build.stderr).into_owned()
}

/// The generic body under test in both goldens below: `mylt` compares its own
/// `'T` through `core::cmp`'s imported, generic `lt`. That is exactly the
/// program `tests/phase8_slice2.rs`'s retired
/// `a_poly_word_calling_an_imported_poly_word_names_the_narrowing` pinned as a
/// located error.
const MYLT: &str = "import: intrinsics * ;\n\
     import: core::prelude * ;\n\
     import: core::bool cb | Bool | ;\n";

/// R1/R3: the capability. An imported generic callee is reached from a
/// non-inline generic body, at two distinct instantiations, and the program
/// runs.
///
/// Run rather than merely built, and at both `i64` and `f64`: `lt` is
/// `Copy Ord`-generic over the whole numeric tower and lowers to a
/// type-directed `ult` intrinsic, so a call that reached the wrong
/// instantiation would compare the wrong way (or the wrong width) rather than
/// fail to link. `2.0 1.0 lt` is deliberately false while `1 2 lt` is true, so
/// one instantiation's answer cannot stand in for the other's.
#[test]
fn a_generic_body_compares_its_own_variable_through_an_imported_generic_word() {
    let t = Tree::new("mylt");
    let entry = t.write(
        "main.sth",
        &format!(
            "{MYLT}: mylt ( 'T: Copy Ord 'T -- Bool ) lt ;\n\
             : main ( -- )\n\
               1 2 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               5 4 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               1.0 2.0 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               2.0 1.0 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               ;\n"
        ),
    );
    assert_eq!(build_and_run(&entry), "1\n0\n1\n0\n");
}

/// R3: the callee's bounds are discharged against the *caller's* declared
/// ones, at the call site. `lt` needs `Copy Ord`; a caller declaring only
/// `Copy` is rejected where the call is written, naming both variables and the
/// missing bound -- not at whatever type a later caller instantiates `mylt`
/// with, and never as a monomorphization-time failure (N1).
#[test]
fn a_bound_the_caller_does_not_declare_is_a_located_call_site_error() {
    let t = Tree::new("mylt-unbounded");
    let entry = t.write(
        "main.sth",
        &format!(
            "{MYLT}: mylt ( 'T: Copy 'T -- Bool ) lt ;\n\
             : main ( -- ) 1 2 mylt drop ;\n"
        ),
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`'T` of `lt` requires `Ord`, which `'T` in `mylt` does not declare"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        err.contains("declare `'T: Ord`"),
        "the diagnostic should name the remedy: {err}"
    );
    assert!(
        !err.contains("__m"),
        "the diagnostic must not leak a mangled spelling: {err}"
    );
}

/// R6: a cross-call whose operand wraps one of the caller's own variables is a
/// located rejection at the call site. `Box['T]` grows by one constructor per
/// hop, so a recursive cross-call of this shape has no finite set of
/// instantiations and no dedup would ever fire.
///
/// The wrapper is a generic **enum** on purpose: array construction inside a
/// polymorphic body is refused by a pre-existing guard, so an array-based
/// witness would never reach the growth rule at all. Sooth has no generic
/// structs.
#[test]
fn a_cross_call_growing_the_type_is_a_located_rejection() {
    let t = Tree::new("growing");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         type: Box 'T | Box 'T ;\n\
         : h ( 'U -- 'U ) ;\n\
         : g ( 'T -- ) Box h drop ;\n\
         : main ( -- ) 1 g ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("cannot pass `Box['T]` to `'U` of the polymorphic word `h`")
            && err.contains("builds a larger type at every hop"),
        "unexpected diagnostic: {err}"
    );
}
