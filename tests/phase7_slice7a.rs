//! P7.S7a phase 2 (R2/R3/R4): `lib/hosted/` as a sibling package exporting
//! `exit ( i32 -- )`, the layer check against it, and the located type
//! error on `1 exit` without `>i32`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch tree of packages, removed on drop. Deliberately not
/// `common::fixture_source`-wrapped: `import: hosted::libc l | exit | ;`
/// itself is what these fixtures are about, so writes go straight through.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s7a-{}-{tag}-{seq}", std::process::id()));
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

fn checkout() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

/// R2.3's manifest, verbatim: **both** `depends:` entries, since the table
/// is not transitive.
fn s7a_manifest() -> String {
    format!(
        "package: s7a ;\nlayer: hosted ;\ndepends: core path \"{}/lib/core\" ;\ndepends: hosted path \"{}/lib/hosted\" ;\n",
        checkout(),
        checkout()
    )
}

fn build_err(entry: &Path) -> String {
    match driver::build(entry) {
        Ok(_) => panic!("build should have failed"),
        Err(e) => e,
    }
}

/// R5.1: a program that imports `hosted::libc`'s `exit`, prints something
/// first, then calls `exit` with its argument -- run with a nonzero and a
/// zero value. The stdout assertion rules out a program that never ran
/// passing case one by exiting 7 for another reason.
fn run_exit(entry: &Path) -> (Option<i32>, String) {
    let binary = driver::build(entry).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        output.status.code(),
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
    )
}

fn exit_fixture(t: &Tree, code: &str) -> PathBuf {
    t.write("sooth.pkg", &s7a_manifest());
    t.write(
        "main.sth",
        &format!(
            "import: intrinsics * ;\nimport: hosted::libc l | exit | ;\nextern: puts ( cstr -- i64 ) \"puts\" ;\n: main ( -- )\n  \"ran\" cstr puts drop\n  {code} >i32 exit ;\n"
        ),
    )
}

#[test]
fn hosted_libc_exit_nonzero_code_observed() {
    let t = Tree::new("exit-nonzero");
    let entry = exit_fixture(&t, "7");
    let (code, stdout) = run_exit(&entry);
    assert_eq!(stdout, "ran\n");
    assert_eq!(code, Some(7));
}

#[test]
fn hosted_libc_exit_zero_code_observed() {
    let t = Tree::new("exit-zero");
    let entry = exit_fixture(&t, "0");
    let (code, stdout) = run_exit(&entry);
    assert_eq!(stdout, "ran\n");
    assert_eq!(code, Some(0));
}

/// R5.3/R5.8(2): a `layer: core` package `depends:` on the real
/// `lib/hosted` is rejected with the layer-violation message naming both
/// layers -- the real checkout path, which a fixture tree cannot stand in
/// for.
#[test]
fn layer_core_depends_on_real_hosted_is_error() {
    let t = Tree::new("layer-violation-real-hosted");
    t.write(
        "sooth.pkg",
        &format!(
            "package: s7a ;\nlayer: core ;\ndepends: hosted path \"{}/lib/hosted\" ;\n",
            checkout()
        ),
    );
    let entry = t.write("main.sth", ": main ( -- ) 0 . ;\n");
    let err = build_err(&entry);
    assert!(
        err.contains(
            "package `s7a` is layer `core` but depends on `hosted` which is layer `hosted`"
        ),
        "unexpected message: {err}"
    );
}

/// R5.4: `1 exit` without the `>i32` cast is a located type error, pinned to
/// the exact two lines (not the doubled `error: ` prefix).
#[test]
fn exit_without_cast_is_located_type_error() {
    let t = Tree::new("exit-no-cast");
    t.write("sooth.pkg", &s7a_manifest());
    let entry = t.write(
        "main.sth",
        "import: hosted::libc l | exit | ;\n: main ( -- ) 1 exit ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("type mismatch in `main` (line 2)"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("`exit` expected `i32`, found `i64`"),
        "unexpected message: {err}"
    );
}

/// R5.5 (missing `depends:`, against the real `lib/hosted`): a package
/// naming only `core` in its own `depends:` table imports `hosted::libc`.
#[test]
fn hosted_import_without_depends_entry_is_error() {
    let t = Tree::new("no-depends-real-hosted");
    t.write(
        "sooth.pkg",
        &format!(
            "package: s7a ;\nlayer: hosted ;\ndepends: core path \"{}/lib/core\" ;\n",
            checkout()
        ),
    );
    let entry = t.write(
        "main.sth",
        "import: hosted::libc l | exit | ;\n: main ( -- ) 0 . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("package `s7a` has no `depends:` entry for `hosted`"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("add `depends: hosted path \"<path>\" ;`"),
        "unexpected message: {err}"
    );
}

/// R5.5 (private module, against a harness-written fixture, never the real
/// `lib/hosted`): a dependency whose `module:` list omits a module file
/// present on disk. `lib/hosted` cannot witness this case at all -- R2.1
/// lists `libc`, R3.4 forbids a second module, and resolution requires the
/// file to exist before consulting `module:`.
#[test]
fn private_module_is_error_against_a_fixture_dependency() {
    let t = Tree::new("private-module-fixture");
    t.write("dep/sooth.pkg", "package: dep ; layer: hosted ;");
    t.write("dep/secret.sth", ": sw ( -- i64 ) 2 ;\nexport: sw ;\n");
    t.write(
        "sooth.pkg",
        &format!(
            "package: s7a ;\nlayer: hosted ;\ndepends: dep path \"{}/dep\" ;\n",
            t.0.display()
        ),
    );
    let entry = t.write(
        "main.sth",
        "\n  import: dep::secret d ;\n: main ( -- ) d::sw . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("module `secret` is not in `dep`'s public `module:` list"),
        "unexpected message: {err}"
    );
}
