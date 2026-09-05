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

/// M3: `bump`'s forwarded quotation parameter has a *concrete* effect
/// (`[ i64 -- i64 ]`) rather than the member-local `'T`/`'U` spelling, so
/// the operand slot folds to `PolyType::Concrete(Type::Quotation(..))` while
/// `map`'s declared input stays a `PolyType::Quotation(..)` -- neither
/// matches `unify_member_operand`'s `Quotation`/`Quotation` arm nor any
/// other, so it falls to the catch-all and reports a mismatch. Pre-Phase-1
/// this panicked in `poly_type_str` (a member-local `Var` indexing off the
/// end of the caller's `ty_var_names`); Phase 1 makes it a located error, not
/// a value (Phase 2 is the one that makes this shape dispatch).
#[test]
fn poly_body_concrete_effect_quotation_operand_is_located_error() {
    let stderr = build_error(
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
    assert!(
        stderr.contains("`map` of `Functor`"),
        "expected a located `map`/`Functor` mismatch, got: {stderr}"
    );
    assert!(
        stderr.contains("expects `[ 'T -- 'U ]`, found `[ i64 -- i64 ]`"),
        "expected the member-space/caller-space rendering split, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "the M3 concrete-effect fixture must not panic, got: {stderr}"
    );
}
