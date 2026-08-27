//! P7.S10 golden: a recursive `impl: Ord` -- one whose `cmp` compares its own
//! type with a surface comparison instead of delegating to its fields --
//! makes lowering re-splice the member without bound. This pins the fix: a
//! bounded splice-depth guard in `lower_resolved_word_call` turns the
//! unbounded recursion into a located diagnostic and a clean non-zero exit,
//! never a `SIGABRT`.
//!
//! Scaffolding (`Tree`/`sooth_build`) mirrors `tests/phase7_slice3s_flip.rs`;
//! `tests/common/mod.rs::fixture_package` is the source of truth for a
//! fixture package body.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s10-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sooth.pkg"), common::fixture_package(tag)).unwrap();
        Tree(dir)
    }

    fn entry(&self, src: &str) -> PathBuf {
        let path = self.0.join("main.sth");
        std::fs::write(&path, src).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn program(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.entry(src);
    (t, entry)
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

/// The witness: `Wrap`'s `impl: Ord` compares its own type with `lt` instead
/// of delegating to its field, so `cmp` dispatches back to itself. `n`
/// blank lines are inserted before the `impl:` block so a caller can shift
/// the `: cmp` declaration's line by a known amount (R4.1 assertion 3b).
///
/// Built by joining a `Vec<&str>` rather than a `\`-continued literal: Rust
/// strips leading whitespace after a line-continuation backslash, which would
/// collapse `: cmp`'s two-space indent and shift the asserted column. Joining
/// keeps the exact indentation a user's source file would have.
fn wrap_source(n: u32) -> String {
    let mut lines: Vec<&str> = vec![
        "import: intrinsics * ;",
        "import: core::prelude | if Bool Ord lt gt | ;",
        "import: core::cmp | Ordering Less Equal Greater | ;",
        "type: Wrap v i64 ;",
    ];
    for _ in 0..n {
        lines.push("");
    }
    lines.extend([
        "impl: Ord for Wrap",
        "  : cmp",
        "    | a b |",
        "    a b lt ~[ Less ] ~[ Equal ] if ;",
        ";",
        ": main ( -- )",
        "  1 Wrap 2 Wrap lt ~[ 1 ] ~[ 0 ] if . ;",
    ]);
    let mut src = lines.join("\n");
    src.push('\n');
    src
}

/// `: cmp`'s own declaration line in `wrap_source`, matching the fixture the
/// test itself writes rather than a value copied from the spec document.
fn wrap_cmp_line(n: u32) -> u32 {
    6 + n
}

const SPLICE_BUDGET: u32 = 64;

fn assert_located_diagnostic(stderr: &str, line: u32) {
    let expected = format!(
        "error: a trait member cannot dispatch back to itself (lowering would splice it forever): `cmp` (member of trait `Ord` for `Wrap`) exceeded the splice budget of {SPLICE_BUDGET} (line {line}, col 5)"
    );
    assert!(
        stderr.contains(&expected),
        "stderr did not contain the expected diagnostic.\nexpected: {expected}\nstderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("error: ").count(),
        1,
        "the rendered stderr should carry exactly one \"error: \" prefix; stderr: {stderr}"
    );
}

/// R4.1: the exit criterion. A recursive `impl: Ord` produces a located
/// diagnostic on stderr (assertions 1-3b) and a clean non-zero exit, never a
/// signal death (assertion 4).
#[test]
fn a_recursive_impl_ord_is_a_located_diagnostic_not_a_stack_overflow() {
    let (_t, entry) = program("recursive-impl-ord", &wrap_source(0));
    let build = sooth_build(&entry);
    assert!(
        !build.status.success(),
        "build should have failed; stdout: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
    assert_located_diagnostic(&stderr, wrap_cmp_line(0));
    assert_eq!(
        build.status.code(),
        Some(1),
        "exit should be 1, not a signal death (134/SIGABRT); status: {:?}",
        build.status
    );
}

/// R4.1 assertion 3b: a second fixture variant, identical but with two blank
/// lines inserted before the `impl:` block, reports a line greater by exactly
/// that offset. A pinned constant alone could satisfy assertion 3; this pair
/// is how the span half of the mechanism is actually verified (R4.5's third
/// mutation targets exactly this pairing).
#[test]
fn a_recursive_impl_ord_diagnostic_line_tracks_the_declaration_site() {
    let (_t, entry) = program("recursive-impl-ord-shifted", &wrap_source(2));
    let build = sooth_build(&entry);
    assert!(
        !build.status.success(),
        "build should have failed; stdout: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
    assert_located_diagnostic(&stderr, wrap_cmp_line(2));
    assert_eq!(build.status.code(), Some(1));
}
