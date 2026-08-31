//! P7.S3s phase 1 goldens: a polymorphic body may call another polymorphic
//! word carrying a `Bound::User` on a forwarded variable, and lowering finds
//! a symbol for the composed callee -- through the real `sooth` binary and
//! its linked output, so a wrong-symbol link is caught, not merely a checked
//! program.
//!
//! `src/check/poly.rs`'s own unit tests (`check_generic_cross_call_
//! discharges_a_forwarded_user_bound`, `check_generic_cross_call_
//! concrete_image_with_no_impl_is_a_located_error`) pin the mechanism
//! directly against `module.transitive_instantiations`; these two pin the
//! end-to-end behaviour the mechanism exists to produce.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3s-p1-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(
            &path,
            format!("{contents}{}", common::printing_import(contents)),
        )
        .unwrap();
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
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .arg(entry)
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

/// R2's headline capability, run rather than merely checked: `g`'s body
/// forwards its own `'T: Show` bound to `shows`, a different polymorphic
/// word. `main` instantiates `g` at `Point`; if the composed callee resolved
/// to the wrong symbol (or none at all), this would fail to link rather than
/// print the wrong thing, so the assertion is on stdout, not exit status.
#[test]
fn a_generic_body_forwards_a_user_bound_to_another_generic_word() {
    let t = Tree::new("forward");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         type: Point x i64 y i64 ;\n\
         impl: Show for Point\n\
           : show | p | p drop 42 . ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : g ['T: Show] ( &'T -- ) shows ;\n\
         : main ( -- ) 1 2 Point |p| &p g p drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "42\n");
}

/// R2/R3's error half: `g`'s body builds `Other` itself and hands a borrow of
/// it to `shows`, a `Bound::User`-bounded callee -- a concrete-image
/// cross-call, not a forwarded caller variable. `Other` has no `impl: Show`,
/// so `compose`'s own bound loop must reject this at check time, before
/// lowering ever runs, with the same diagnostic a direct unsatisfied bound
/// gets.
#[test]
fn a_concrete_image_cross_call_with_no_impl_is_a_located_build_error() {
    let t = Tree::new("concrete-image");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         type: Other n i64 ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : g ( 'T -- ) drop 7 Other |o| &o shows o drop ;\n\
         : main ( -- ) 1 g ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`Other` does not satisfy `Show`: no `( &Other -- )` found"),
        "unexpected diagnostic: {err}"
    );
}
