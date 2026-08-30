//! P7b.S1 Phase 1 exit golden: the k8 flip from the recon brief -- an
//! unannotated length var in `array['T 'N]` now builds, its kind inferred
//! from the count position, instead of demanding `'N: Len`.
//!
//! Driven through the real `sooth` binary, mirroring `tests/phase7_slice6a.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs1-{}-{tag}-{seq}", std::process::id()));
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

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

#[allow(dead_code)]
fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
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

#[allow(dead_code)]
fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// The k8 flip (S1-3/S1-5): `array['T 'N]` with no `'N: Len` annotation
/// anywhere -- `'N`'s kind is inferred from the count position it appears
/// in, so the program builds and runs instead of S6a's mandatory-annotation
/// rejection.
#[test]
fn hkt_len_var_inferred_from_count_position_is_accepted() {
    let src = "\
        : sum['T 'N] ( array['T 'N] -- usize ) len swap drop ;
\
        : main ( -- )
          0 >u8 4 fill sum .
        ;\n";
    let (_t, entry) = single_file("k8-flip", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "4\n");
}
