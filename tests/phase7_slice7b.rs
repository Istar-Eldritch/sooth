//! P7.S7b phase 1: `hosted::testing`'s `expect`/`expect-eq` vocabulary --
//! a build-and-run golden, since there is no Rust stage file for a `.sth`
//! module.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch tree, removed on drop, holding the harness fixture that exercises
/// `hosted::testing` -- deliberately not committed under `examples/` (the spec
/// reserves that for the Phase 4 dogfood suite).
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s7b-{}-{tag}-{seq}", std::process::id()));
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

/// A manifest with both a `core` and a `hosted` `depends:` entry, matching the
/// shape a real `hosted`-layer package would carry (`phase7_slice7a.rs`'s
/// `s7a_manifest`).
fn manifest() -> String {
    format!(
        "package: s7b ;\nlayer: hosted ;\ndepends: core path \"{}/lib/core\" ;\ndepends: hosted path \"{}/lib/hosted\" ;\n",
        checkout(),
        checkout()
    )
}

/// One passing and one failing `expect`, and one passing and one failing
/// `expect-eq` -- the four expected protocol lines, in this order.
fn testing_fixture(t: &Tree) -> PathBuf {
    t.write("sooth.pkg", &manifest());
    t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: hosted::testing t | expect expect-eq | ;\n\
         : main ( -- )\n\
         True \"true is true\" expect\n\
         False \"false is true\" expect\n\
         1 1 \"one equals one\" expect-eq\n\
         1 2 \"one equals two\" expect-eq ;\n",
    )
}

#[test]
fn hosted_testing_expect_and_expect_eq_print_the_r1_protocol() {
    let t = Tree::new("expect-protocol");
    let entry = testing_fixture(&t);
    let binary = driver::build(&entry).expect("the fixture should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("the binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert_eq!(
        stdout,
        "ok -- true is true\n\
         not ok -- false is true\n\
         ok -- one equals one\n\
         not ok -- one equals two\n"
    );
    assert!(output.status.success());
}
