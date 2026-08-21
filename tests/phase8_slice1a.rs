//! P8.S1a goldens: cross-package `pkg::module` builds, and the three
//! `check_package_graph` diagnostics (missing `depends:`, private module,
//! layer violation) as located build errors. Every negative golden pins the
//! exact diagnostic substring, never a bare `is_err()`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch tree of packages (each its own directory with a `sooth.pkg` and
/// `.sth` files), removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p8s1a-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, common::fixture_source(rel, contents)).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build and run the entry file, returning `(stdout, exit_code)`.
fn build_and_run(entry: &Path) -> (String, i32) {
    let binary = driver::build_with_manifest(entry, common::manifest_for(entry).as_deref())
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code().expect("process exits normally"),
    )
}

/// Build the entry file, expecting a diagnostic (the closure never links).
fn build_err(entry: &Path) -> String {
    match driver::build_with_manifest(entry, common::manifest_for(entry).as_deref()) {
        Ok(_) => panic!("build should have failed"),
        Err(e) => e,
    }
}

/// Golden 1: a `core` package with a public `cmp` module, and a consumer
/// `app` package (`layer: hosted`, mandatory) that imports `cmp::lt` via
/// `import: core::cmp c ;` and prints the result through `main`.
#[test]
fn cross_package_import_public_module_builds() {
    let t = Tree::new("cross-package-build");
    t.write(
        "core/sooth.pkg",
        "package: core ; layer: core ; module: cmp ;",
    );
    t.write("core/cmp.sth", ": lt ( -- i64 ) 1 ;\nexport: lt ;\n");
    t.write(
        "app/sooth.pkg",
        r#"package: app ; layer: hosted ; depends: core path "../core" ;"#,
    );
    let entry = t.write(
        "app/main.sth",
        "import: core::cmp c ;\n: main ( -- ) c::lt . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

/// Golden 2: `core` (layer `core`) `depends:` on `app` (layer `hosted`), a
/// strictly higher layer. The build fails with the OQ4-B error, pinning both
/// package names and both layer values, and separately the located `depends:`
/// entry -- placed off (1, 1) so a de-located diagnostic can't pass by
/// coincidence.
#[test]
fn layer_violation_core_depends_on_hosted_is_error() {
    let t = Tree::new("layer-violation");
    t.write(
        "core/sooth.pkg",
        "package: core ;\n  layer: core ;\n  depends: app path \"../app\" ;\n",
    );
    t.write(
        "app/sooth.pkg",
        "package: app ; layer: hosted ; module: util ;",
    );
    t.write("app/util.sth", ": uw ( -- i64 ) 2 ;\nexport: uw ;\n");
    let entry = t.write(
        "core/main.sth",
        "import: app::util u ;\n: main ( -- ) u::uw . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("package `core` is layer `core` but depends on `app` which is layer `hosted`"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("line 3, col 3"),
        "expected the offending `depends:` entry's location in: {err}"
    );
}

/// Golden 2b: the identical fixture as Golden 2 but with the `import:
/// app::util u ;` line deleted -- the `depends: app path "../app" ;` line
/// alone must still trip the layer check. If this test and Golden 2 ever
/// disagree, the layer check has silently become import-triggered.
#[test]
fn layer_violation_fires_without_an_import() {
    let t = Tree::new("layer-violation-no-import");
    t.write(
        "core/sooth.pkg",
        "package: core ;\n  layer: core ;\n  depends: app path \"../app\" ;\n",
    );
    t.write("app/sooth.pkg", "package: app ; layer: hosted ;");
    let entry = t.write("core/main.sth", ": main ( -- ) 0 . ;\n");
    let err = build_err(&entry);
    assert!(
        err.contains("package `core` is layer `core` but depends on `app` which is layer `hosted`"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("line 3, col 3"),
        "expected the offending `depends:` entry's location in: {err}"
    );
}

/// Golden 3: a consumer imports `core::detail`, but `core`'s manifest lists
/// only `module: cmp ;`, not `detail`. The build fails with the OQ4-C error,
/// pinning both package name and module name, and separately the located
/// import site -- placed off (1, 1) so a de-located diagnostic can't pass by
/// coincidence.
#[test]
fn cross_package_import_private_module_is_error() {
    let t = Tree::new("private-module");
    t.write(
        "core/sooth.pkg",
        "package: core ; layer: core ; module: cmp ;",
    );
    t.write("core/detail.sth", ": dw ( -- i64 ) 2 ;\nexport: dw ;\n");
    t.write(
        "app/sooth.pkg",
        r#"package: app ; layer: hosted ; depends: core path "../core" ;"#,
    );
    let entry = t.write(
        "app/main.sth",
        "\n  import: core::detail d ;\n: main ( -- ) d::dw . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("module `detail` is not in `core`'s public `module:` list"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("line 2, col 3"),
        "expected the offending import's location in: {err}"
    );
}

/// Golden 4: `app` imports `collections::vec` with no `depends: collections
/// ...` entry in its manifest. The build fails with the OQ4-A error, pinning
/// both package names in one substring, and separately asserts the located
/// import site (`line 2, col 3`) -- placed off (1, 1) so a de-located
/// diagnostic (e.g. a stray `Span::default()`) can't pass by coincidence.
#[test]
fn cross_package_import_no_depends_is_error() {
    let t = Tree::new("no-depends");
    t.write("app/sooth.pkg", "package: app ; layer: hosted ;");
    let entry = t.write(
        "app/main.sth",
        "\n  import: collections::vec v ;\n: main ( -- ) 0 . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("package `app` has no `depends:` entry for `collections`"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("line 2, col 3"),
        "expected the offending import's location in: {err}"
    );
}
