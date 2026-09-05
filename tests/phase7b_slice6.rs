//! P7b.S6 Phase 1 exit golden (R2.a): the M3 concrete-effect quotation
//! operand at an HKT member -- pre-fix an ICE in `trait_member_operand_error`
//! (`substitute_member_var` leaving a member-local quotation var untouched
//! and indexing off the end of the caller's `ty_var_names`), post-fix a
//! located diagnostic. Harness style from `tests/phase7b_slice4.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs6-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `tests/phase7b_slice4.rs`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs6 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

fn build_error(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// Build `src` and run the produced binary, returning its stdout. A build
/// failure panics with the compiler's stderr: every caller of this helper
/// expects a value, so a rejection is itself the regression to surface.
fn build_and_run(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should have succeeded, stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    String::from_utf8(run.stdout).expect("stdout should be utf8")
}

/// M3/Phase 2 (R2.b): `bump`'s forwarded quotation parameter has a
/// *concrete* effect (`[ i64 -- i64 ]`) rather than the member-local
/// `'T`/`'U` spelling, so the operand slot folds to
/// `PolyType::Concrete(Type::Quotation(..))` while `map`'s declared input
/// stays a `PolyType::Quotation(..)`. Pre-Phase-2 this fell through
/// `unify_member_operand`'s catch-all (a located error, Phase 1's fix over
/// what used to panic); Phase 2's cross-representation bridge arm now binds
/// `map`'s `'T`/`'U` to `i64` and dispatches, printing the bumped value --
/// the dead criterion this golden replaces (a rename, not a new fixture) is
/// the old "still rejected" assertion.
#[test]
fn poly_body_forwards_a_concrete_effect_quotation_parameter_to_a_member() {
    let stdout = build_and_run(
        "m3-concrete-effect",
        "\
import: core::option * ;\n\
trait: Functor['F: * -> *] :\n\
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
;\n\
impl: Functor for Option\n\
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
;\n\
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;\n\
: bump['F: Functor] ( 'F[i64] [ i64 -- i64 ] -- 'F[i64] ) map ;\n\
: main ( -- ) 3 Some [ 1 sub ] bump showopt ;\n\
",
    );
    assert_eq!(stdout, "2\n");
}

/// Non-regression (R2.b): the already-working generic-effect quotation
/// parameter (M3's headline case, the existing `Quotation`/`Quotation` arm)
/// still dispatches after Phase 2's new arm lands beside it.
#[test]
fn poly_body_forwards_a_generic_effect_quotation_parameter() {
    let stdout = build_and_run(
        "m3-generic-effect",
        "\
import: core::option * ;\n\
trait: Functor['F: * -> *] :\n\
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
;\n\
impl: Functor for Option\n\
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
;\n\
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;\n\
: bump['F: Functor 'A] ( 'F['A] [ 'A -- 'A ] -- 'F['A] ) map ;\n\
: main ( -- ) 3 Some [ 1 sub ] bump showopt ;\n\
",
    );
    assert_eq!(stdout, "2\n");
}

/// M3/R2.b: a written quotation **literal** (as opposed to a forwarded
/// parameter) at the same member call stays a located error -- no
/// materialization is attempted at a poly member call site this slice, and
/// the fixture must neither panic nor silently dispatch.
#[test]
fn poly_body_quotation_literal_member_operand_is_located_error() {
    let stderr = build_error(
        "m3-quot-literal",
        "\
import: core::option * ;\n\
trait: Functor['F: * -> *] :\n\
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
;\n\
impl: Functor for Option\n\
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
;\n\
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;\n\
: bump['F: Functor] ( 'F[i64] -- 'F[i64] ) [ 1 sub ] map ;\n\
: main ( -- ) 3 Some bump showopt ;\n\
",
    );
    assert!(
        stderr.contains("`map` of `Functor`"),
        "expected a located `map`/`Functor` mismatch, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "the quotation-literal fixture must not panic, got: {stderr}"
    );
}
