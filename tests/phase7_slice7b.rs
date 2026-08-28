//! P7.S7b phase 1: `hosted::testing`'s `expect`/`expect-eq` vocabulary --
//! a build-and-run golden, since there is no Rust stage file for a `.sth`
//! module.

use std::path::{Path, PathBuf};
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
        "import: core::prelude * ;\n\
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

//
// Phase 2 -- `driver::test`: discovery, temp-dir builds, protocol counting.
//

fn pkg_manifest() -> String {
    format!(
        "package: p ;\nlayer: hosted ;\ndepends: core path \"{}/lib/core\" ;\ndepends: hosted path \"{}/lib/hosted\" ;\n",
        checkout(),
        checkout()
    )
}

/// A test entry whose `expect`/`expect-eq` all hold.
fn passing_entry(t: &Tree, rel: &str) -> PathBuf {
    t.write(
        rel,
        "import: core::prelude * ;\n\
         import: hosted::testing t | expect expect-eq | ;\n\
         : main ( -- )\n\
         True \"true is true\" expect\n\
         1 1 \"one equals one\" expect-eq ;\n",
    )
}

/// A test entry with a deliberately-failing assertion: the child still exits
/// 0, so a runner keying failure on exit code alone would wrongly pass this.
fn failing_entry(t: &Tree, rel: &str) -> PathBuf {
    t.write(
        rel,
        "import: core::prelude * ;\n\
         import: hosted::testing t | expect expect-eq | ;\n\
         : main ( -- )\n\
         1 2 \"one equals two\" expect-eq ;\n",
    )
}

/// A test entry that exits nonzero directly (no protocol lines at all).
fn crashing_entry(t: &Tree, rel: &str) -> PathBuf {
    t.write(
        rel,
        "import: intrinsics | >i32 | ;\nimport: hosted::libc l | exit | ;\n: main ( -- ) 1 >i32 exit ;\n",
    )
}

/// A test entry that exits zero directly -- must be reported passed, ruling
/// out a runner that keys failure on "any exit call".
fn clean_exit_entry(t: &Tree, rel: &str) -> PathBuf {
    t.write(
        rel,
        "import: intrinsics | >i32 | ;\nimport: hosted::libc l | exit | ;\n: main ( -- ) 0 >i32 exit ;\n",
    )
}

/// A test entry that does not compile: `nope` is no word in scope.
fn uncompilable_entry(t: &Tree, rel: &str) -> PathBuf {
    t.write(rel, "import: core::prelude * ;\n: main ( -- ) nope ;\n")
}

/// A test entry that traps at runtime: an out-of-range index prints a located
/// message on the child's *stderr* and exits nonzero.
fn trapping_entry(t: &Tree, rel: &str) -> PathBuf {
    t.write(
        rel,
        "import: core::prelude * ;\n\
         import: intrinsics | fill slice >usize &> @ . | ;\n\
         : main ( -- )\n\
         7 4 fill | buf |\n\
         &buf slice | s |\n\
         s 9 >usize &> @ .\n\
         buf drop ;\n",
    )
}

/// `driver::test` with both channels captured: the exit code, the report, and
/// the diagnostics.
fn run_test(cwd: &Path, paths: &[PathBuf]) -> (i32, String, String) {
    let mut report = Vec::new();
    let mut diagnostics = Vec::new();
    let code =
        driver::test(cwd, paths, &mut report, &mut diagnostics).expect("test run should succeed");
    (
        code,
        String::from_utf8(report).expect("report is utf8"),
        String::from_utf8(diagnostics).expect("diagnostics are utf8"),
    )
}

#[test]
fn driver_test_pass_reports_green() {
    let t = Tree::new("pass");
    t.write("sooth.pkg", &pkg_manifest());
    passing_entry(&t, "tests/a.sth");
    let entry = t.0.join("tests/a.sth");
    let (code, report, diagnostics) = run_test(&t.0, &[t.0.join("tests")]);
    assert_eq!(code, 0);
    assert_eq!(
        report,
        format!(
            "ok   {}\n1 entries, 0 failed (2 ok, 0 not ok assertions)\n",
            entry.display()
        )
    );
    assert_eq!(diagnostics, "", "a green run diagnoses nothing");
}

/// R1: the runner reads the *protocol*, not just the exit code. The child
/// exits 0 (no trap, no `exit` call) yet the run must still be reported
/// failed because it printed a `not ok` line.
#[test]
fn driver_test_fail_by_protocol_is_reported_failed_despite_zero_exit() {
    let t = Tree::new("fail-protocol");
    t.write("sooth.pkg", &pkg_manifest());
    failing_entry(&t, "tests/a.sth");
    let (code, report, _) = run_test(&t.0, &[t.0.join("tests")]);
    assert_ne!(code, 0);
    assert!(report.contains("FAIL "), "failed verdict line: {report}");
}

/// R1.1: a nonzero child exit is a failure on its own, independent of
/// protocol lines; a zero `exit` call must still be reported passed, ruling
/// out a runner that keys failure on "any exit call" rather than the exit
/// code.
#[test]
fn driver_test_fail_by_crash_and_clean_exit_are_discriminated() {
    let t = Tree::new("fail-crash");
    t.write("sooth.pkg", &pkg_manifest());
    crashing_entry(&t, "tests/crash.sth");
    let (crash_only, report, _) = run_test(&t.0, &[t.0.join("tests/crash.sth")]);
    assert_ne!(crash_only, 0);
    // A crashed entry prints no protocol line at all, so the counts alone
    // ({0 ok, 0 not ok}) say nothing about why it failed: the verdict carries
    // the child's exit status.
    assert!(
        report.contains("exit status: 1"),
        "the verdict names the child's exit status: {report}"
    );

    let t2 = Tree::new("clean-exit");
    t2.write("sooth.pkg", &pkg_manifest());
    clean_exit_entry(&t2, "tests/clean.sth");
    let (clean_only, _, _) = run_test(&t2.0, &[t2.0.join("tests/clean.sth")]);
    assert_eq!(clean_only, 0);
}

/// A trapping entry's own message is the only account of what went wrong: the
/// runner captures the child's output, so a discarded stderr would lose it
/// entirely. It is forwarded to the diagnostic channel, not folded into the
/// verdict line.
#[test]
fn driver_test_forwards_a_trapping_childs_stderr() {
    let t = Tree::new("trap");
    t.write("sooth.pkg", &pkg_manifest());
    trapping_entry(&t, "tests/trap.sth");
    let (code, report, diagnostics) = run_test(&t.0, &[t.0.join("tests")]);
    assert_ne!(code, 0);
    assert_eq!(
        diagnostics,
        "sooth: array index out of range (line 6)\n  index 9 is out of bounds for length 4\n"
    );
    assert!(
        !report.contains("out of range"),
        "the child's message stays off the report channel: {report}"
    );
}

/// R1: a failed *build* is the third failure channel, alongside a `not ok`
/// line and a nonzero exit -- a runner that reported green for a suite that
/// does not compile would be worse than no runner. R1.1 keeps the compiler's
/// diagnostic on its own channel (stderr), so the report carries the verdict
/// and not the error text.
#[test]
fn driver_test_build_failure_is_reported_failed() {
    let t = Tree::new("build-failure");
    t.write("sooth.pkg", &pkg_manifest());
    uncompilable_entry(&t, "tests/broken.sth");
    let (code, report, diagnostics) = run_test(&t.0, &[t.0.join("tests")]);
    assert_ne!(code, 0);
    assert!(
        report.contains("FAIL ") && report.contains("broken.sth") && report.contains("build"),
        "the verdict names the entry and the build: {report}"
    );
    assert!(
        !report.contains("nope"),
        "the compiler's diagnostic stays off the report channel: {report}"
    );
    assert!(
        diagnostics.contains("nope"),
        "the compiler's diagnostic reaches the diagnostic channel: {diagnostics}"
    );
}

/// Exit criterion 2: a summary line reports the totals -- entries, failures,
/// and the assertion counts summed across entries.
#[test]
fn driver_test_summary_line_reports_totals() {
    let t = Tree::new("summary");
    t.write("sooth.pkg", &pkg_manifest());
    passing_entry(&t, "tests/a.sth");
    failing_entry(&t, "tests/b.sth");
    let (code, report, _) = run_test(&t.0, &[t.0.join("tests")]);
    assert_ne!(code, 0);
    let summary = report.lines().last().expect("a summary line");
    assert_eq!(
        summary, "2 entries, 1 failed (2 ok, 1 not ok assertions)",
        "full report: {report}"
    );
}

/// R3.1: no path resolves the package containing `cwd` and takes every
/// `*.sth` under its `tests/` directory -- the entry set, not just a green
/// run.
#[test]
fn driver_test_discovery_no_path_finds_pkgroot_tests_dir() {
    let t = Tree::new("discovery");
    t.write("sooth.pkg", &pkg_manifest());
    passing_entry(&t, "tests/a.sth");
    passing_entry(&t, "tests/b.sth");
    let entries = driver::discover_test_entries(&t.0, &[]).expect("discovery should succeed");
    assert_eq!(
        entries,
        vec![t.0.join("tests/a.sth"), t.0.join("tests/b.sth")]
    );
}

/// R3.2: a named `.sth` file and a named directory each resolve to the right
/// entry set, driven straight against `discover_test_entries` (no CLI
/// involved -- that lands in Phase 3).
#[test]
fn driver_test_discovery_explicit_path_resolves_file_and_dir() {
    let t = Tree::new("explicit-path");
    t.write("sooth.pkg", &pkg_manifest());
    let file = passing_entry(&t, "tests/a.sth");
    passing_entry(&t, "suite/b.sth");
    passing_entry(&t, "suite/c.sth");
    let entries = driver::discover_test_entries(&t.0, &[file.clone(), t.0.join("suite")])
        .expect("discovery should succeed");
    assert_eq!(
        entries,
        vec![t.0.join("suite/b.sth"), t.0.join("suite/c.sth"), file]
    );
}

/// R3.1: a missing `tests/` directory is a usage-level error, not a silent
/// green run.
#[test]
fn driver_test_discovery_missing_tests_dir_is_error() {
    let t = Tree::new("missing-tests");
    t.write("sooth.pkg", &pkg_manifest());
    assert!(driver::discover_test_entries(&t.0, &[]).is_err());
}

/// R3.1: a present-but-empty `tests/` directory is the same usage-level
/// error as a missing one.
#[test]
fn driver_test_discovery_empty_tests_dir_is_error() {
    let t = Tree::new("empty-tests");
    t.write("sooth.pkg", &pkg_manifest());
    std::fs::create_dir_all(t.0.join("tests")).unwrap();
    assert!(driver::discover_test_entries(&t.0, &[]).is_err());
}
