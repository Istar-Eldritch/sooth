//! P7.S6b exit golden: explicit length arguments at word call sites
//! (`sum[i64 4]` binding `'T = i64` and `'N = 4` against a word declared
//! `sum['T 'N: Len] ( array['T 'N] -- usize )`). Driven through the real
//! `sooth` binary, so the whole pipeline is exercised, not just individual
//! stages.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s6b-{}-{tag}-{seq}", std::process::id()));
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

/// The exit-criterion fixture itself (spec's Phase 4): deliberately
/// non-`inline` and reads the length back via `len` rather than indexing.
const SUM_WORD: &str = "\
: sum['T 'N: Len] ( array['T 'N] -- usize ) len swap drop ;\n";

/// Accept dogfood: `sum[i64 4]` called over a length-4 `array[i64]` checks,
/// builds, and runs, with the resulting length witnessed at runtime.
#[test]
fn explicit_length_argument_matching_operand_builds_and_runs() {
    let src = format!(
        "{SUM_WORD}\
         : main ( -- )
           0 4 fill sum[i64 4] .
         ;\n"
    );
    let (_t, entry) = single_file("accept", &src);
    let out = build_and_run(&entry);
    assert_eq!(out, "4\n");
}

/// R4/R5: an explicit `sum[i64 4]` called over an array whose actual length
/// is `8` is rejected with the *explicit-instantiation* conflict message,
/// not the generic `poly_len_conflict_error` ("resolved length `'N` to both ...").
#[test]
fn explicit_length_argument_conflicting_with_operand_is_rejected() {
    let src = format!(
        "{SUM_WORD}\
         : main ( -- )
           0 8 fill sum[i64 4] .
         ;\n"
    );
    let (_t, entry) = single_file("conflict", &src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`sum` in `main` (line 4) was instantiated at length `'N` = `4` but its operand is `8`"
        ),
        "{err}"
    );
    assert!(!err.contains("resolved length `'N` to both"), "{err}");
}

/// R3: `sum[i64 4 4]` gives two length arguments against a callee declaring
/// only one length variable (`'N`) -- the length-arity error, not the
/// type-argument arity error.
#[test]
fn wrong_length_argument_count_is_rejected() {
    let src = format!(
        "{SUM_WORD}\
         : main ( -- )
           0 4 fill sum[i64 4 4] .
         ;\n"
    );
    let (_t, entry) = single_file("arity", &src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`sum` (line 4) declares 1 length variable (`'N`) but was given 2 length arguments"
        ),
        "{err}"
    );
}
