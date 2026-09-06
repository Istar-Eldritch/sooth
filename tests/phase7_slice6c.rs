//! P7.S6c exit golden: runtime bounds-checked indexing (`&>`/`&!>`) of a
//! generic-length array (`array['T 'N]`) in a non-inline poly body, at a
//! non-literal (computed) index. Driven through the real `sooth` binary, so
//! the whole pipeline is exercised, not just the checker.

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
            std::env::temp_dir().join(format!("sooth-p7s6c-{}-{tag}-{seq}", std::process::id()));
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
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    std::fs::remove_file(&binary).ok();
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

/// R2.1's `at` word: a non-inline poly word declaring `array['T 'N]`,
/// indexing it with `&>` at a `usize` local -- a non-literal index, admitted
/// against an unknown length and deferred to the runtime guard.
const AT_WORD: &str = "\
: at['T: Copy 'N: Len] ( array['T 'N] usize -- 'T )
  | arr i |
  &arr i &> @ | v |
  arr drop
  v ;\n";

/// R4.1: `at[i64 4]` called at a known length, in-bounds computed index,
/// reads the element back.
#[test]
fn generic_length_array_indexed_at_a_computed_index_builds_and_runs() {
    let src = format!(
        "{AT_WORD}\
         : main ( -- )
           7 4 fill 2 >usize at[i64 4] .
         ;\n"
    );
    let (_t, entry) = single_file("accept", &src);
    let out = build_and_run(&entry);
    assert_eq!(out, "7\n");
}

/// R4.1: the `&!>` mutate-in-place twin -- `poly_reference_word`'s
/// `mutable` flag is computed once ahead of the count match both sigils
/// share, so `&!>` must reach the identical deferral.
#[test]
fn generic_length_array_mutated_at_a_computed_index_builds_and_runs() {
    let src = format!(
        "{AT_WORD}\
         : setat['N: Len] ( array[i64 'N] usize i64 -- array[i64 'N] )
           | arr i v |
           &!arr i &!> v !
           arr ;
         : main ( -- )
           0 4 fill 2 >usize 9 setat[4] 2 >usize at[i64 4] .
         ;\n"
    );
    let (_t, entry) = single_file("mutate", &src);
    let out = build_and_run(&entry);
    assert_eq!(out, "9\n");
}

/// R4.2: a genuine runtime out-of-range index (produced by `>usize` on a
/// computed value, not a literal the checker could fold) traps via
/// `sooth_oob_trap` -- nonzero exit, and a sentinel before the access prints
/// while one after it does not, proving the trap aborted rather than fell
/// through. Length (4) and index (7) are deliberately distinct.
#[test]
fn generic_length_array_out_of_range_computed_index_traps_at_runtime() {
    let src = format!(
        "{AT_WORD}\
         : main ( -- )
           1 .
           0 4 fill 3 4 add >usize at[i64 4] drop
           99 .
         ;\n"
    );
    let (_t, entry) = single_file("trap", &src);
    let build = sooth_build(&entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(run.stdout).expect("stdout should be utf8");
    let stderr = String::from_utf8(run.stderr).expect("stderr should be utf8");
    let code = run
        .status
        .code()
        .expect("process should exit normally, not die by signal");

    assert_eq!(stdout, "1\n", "sentinel before the trap should print");
    assert!(
        !stdout.contains("99"),
        "sentinel after the trap must not print: {stdout}"
    );
    assert_ne!(code, 0, "an out-of-bounds access must exit nonzero");
    assert!(
        stderr.contains("out of range"),
        "trap message should say it's out of range: {stderr}"
    );
    assert!(
        stderr.contains("index 7"),
        "trap message should name the distinct index (7): {stderr}"
    );
    assert!(
        stderr.contains("length 4"),
        "trap message should name the distinct length (4): {stderr}"
    );
}
