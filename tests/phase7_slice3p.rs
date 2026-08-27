//! P7.S3p end-to-end golden: the `Indexable`/`at` dogfood from the slice's
//! own brief, compiled and run. `at ( &'T i64 -- i64 )` declares its bound
//! receiver *first*, not last -- the shape S3e's declaration-time rejection
//! used to shut out entirely. Dispatching it through two implementing types
//! whose `at` disagrees on what each index means proves the name-first,
//! position-aware candidate search (`poly.rs`'s `poly_trait_member_call`)
//! reaches the *right* implementation rather than merely being permitted to
//! declare the member. The bounded `uses` body also calls
//! ordinary `eq`/`if` dispatch (unrelated to any bound) before
//! dispatching `at`, pinning that the widened candidate search does not
//! intercept those -- and that they still resolve (mangled) inside a
//! bounded generic body.

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
            std::env::temp_dir().join(format!("sooth-p7s3p-{}-{tag}-{seq}", std::process::id()));
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

/// Names this repo's own `lib/` as `core`, so the fixture can pull `eq`/`if`
/// from `core::prelude`.
fn tree_with_core(tag: &str) -> Tree {
    let t = Tree::new(tag);
    t.write("sooth.pkg", &common::fixture_package(tag));
    t
}

fn build_and_run(entry: &Path) -> String {
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    std::fs::remove_file(&binary).ok();
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// The brief's own probe fixture: `at ( &'T i64 -- i64 )` on `Pair`, its
/// receiver declared first, dispatching index `0`/`1` to `p.a`/`p.b` through
/// a bounded generic body. A second implementing type, `Flip`, inverts the
/// index, so the output distinguishes *which* impl the bounded call reached
/// rather than merely proving that some `at` ran. The bounded `uses` body
/// itself runs ordinary `eq`/`if` (unrelated to the bound) before calling
/// `at`, proving the widened candidate search leaves that dispatch alone.
#[test]
fn indexable_at_on_pair_dispatches_a_non_trailing_receiver() {
    let t = tree_with_core("indexable-pair");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: core::prelude | eq if | ;\n\
         type: Pair a i64 b i64 ;\n\
         type: Flip a i64 b i64 ;\n\
         trait: Indexable 'T : at ( &'T i64 -- i64 ) ; ;\n\
         impl: Indexable for Pair\n\
           : at | n | | p |\n\
             n 0 eq ~[ p &a @ ] ~[ p &b @ ] if ;\n\
         ;\n\
         impl: Indexable for Flip\n\
           : at | n | | p |\n\
             n 0 eq ~[ p &b @ ] ~[ p &a @ ] if ;\n\
         ;\n\
         : uses ( &'T: Indexable i64 -- i64 )\n\
           | n | | p |\n\
           n 0 eq ~[ 0 ] ~[ 1 ] if | i |\n\
           p i at ;\n\
         : main ( -- )\n\
           7 9 Pair |p1| &p1 0 uses . p1 drop\n\
           7 9 Pair |p2| &p2 1 uses . p2 drop\n\
           7 9 Flip |f1| &f1 0 uses . f1 drop\n\
           7 9 Flip |f2| &f2 1 uses . f2 drop\n\
           ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n9\n9\n7\n");
}
