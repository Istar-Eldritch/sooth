//! P7 slice 3i exit goldens: `bool` is an ordinary `core::bool` enum.
//!
//! The compiler injects no boolean type into anything any more, so every
//! fixture here is written **verbatim** (`write_raw`) rather than through
//! `common::write_fixture`: the harness appends the `import:` lines a fixture's
//! text implies, which would silently supply the very import these goldens are
//! about (`common::write_fixture`'s own caveat).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

/// A temp package tree: a `sooth.pkg` naming `core` at this checkout's `lib/`,
/// so a fixture can write `import: core::bool ;` and have it resolve the way a
/// real program's would.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3i-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sooth.pkg"), common::fixture_package("p7s3i")).unwrap();
        Tree(dir)
    }

    /// Write `contents` verbatim -- no harness-appended imports.
    fn write_raw(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        !build.status.success(),
        "build should have failed: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should have succeeded: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the binary should run");
    assert!(run.status.success(), "the binary should exit 0");
    String::from_utf8(run.stdout).expect("stdout should be utf8")
}

// -- G1: nothing boolean resolves without an import -------------------------

/// G1: `true` is a call to `core::bool`'s `True` constructor, so with no import
/// it is an unknown *word*. Nothing is ambient.
#[test]
fn true_without_importing_core_bool_is_an_unknown_word() {
    let t = Tree::new("g1-word");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n: main ( -- ) true drop ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("unknown word `True`"),
        "unexpected diagnostic: {err}"
    );
}

/// G1: `bool` in an effect is a type name, so with no import it is a located
/// unknown *type* -- the diagnostic differs by grammar position, and both point
/// at the same missing import.
#[test]
fn bool_in_an_effect_without_importing_core_bool_is_an_unknown_type() {
    let t = Tree::new("g1-type");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n: w ( bool -- ) drop ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("unknown type `bool` at line 2"),
        "unexpected diagnostic: {err}"
    );
}

/// The prelude hub carries the `true`/`false` constructors but *cannot* carry
/// the type: a type name resolves against its declaring module rather than
/// following a re-export. This is why every corpus file that spells `bool` in an
/// effect imports `core::bool` alongside the prelude, and it is a rule worth
/// pinning: if type re-export through a hub ever lands, this golden is the one
/// that says so.
#[test]
fn the_prelude_hub_carries_the_constructors_but_not_the_type_name() {
    let t = Tree::new("g1-hub");
    let ok = t.write_raw(
        "ctors.sth",
        "import: intrinsics * ;\nimport: core::prelude * ;\n: main ( -- ) true drop false drop ;\n",
    );
    let build = sooth_build(&ok);
    assert!(
        build.status.success(),
        "the constructors re-export: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let entry = t.write_raw(
        "named.sth",
        "import: intrinsics * ;\nimport: core::prelude * ;\n: w ( bool -- ) drop ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("unknown type `bool` at line 3"),
        "unexpected diagnostic: {err}"
    );
}

// -- G2: with the import, everything boolean works --------------------------

/// G2: one `import: core::bool ;` line brings the type, both constructors, the
/// typed branch and the `.` overload, and the program runs.
#[test]
fn importing_core_bool_gives_the_type_constructors_branch_and_print() {
    let t = Tree::new("g2");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         import: core::bool b | bool False True if unless . | ;\n\
         : flip ( bool -- bool )\n\
           ~[ False ] ~[ True ] if ;\n\
         : main ( -- )\n\
           true .\n\
           false .\n\
           true flip .\n\
           false ~[ 1 ] ~[ 2 ] unless . ;\n",
    );
    assert_eq!(build_and_run(&entry), "true\nfalse\nfalse\n1\n");
}

// -- G3 (R1): a boolean static requires the import too ----------------------

/// R1: the static's *type annotation* is the gate. Without `core::bool` in
/// scope it is a located unknown type at the annotation, and the `= true`
/// initializer is never reached -- the same rule the body position obeys, not a
/// second initializer-specific check.
#[test]
fn boolean_static_without_importing_core_bool_is_an_unknown_type() {
    let t = Tree::new("g3-missing");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\nstatic: FLAG bool = true ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("unknown type `bool` at line 2"),
        "unexpected diagnostic: {err}"
    );
}

/// R1, the other side: with the import the static declares, initializes and
/// reads back as `true`.
#[test]
fn boolean_static_with_the_import_holds_its_initializer() {
    let t = Tree::new("g3-present");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         import: core::bool b | bool False True if . | ;\n\
         static: FLAG bool = true ;\n\
         : main ( -- )\n\
           &!FLAG @ .\n\
           &!FLAG @ ~[ 10 ] ~[ 20 ] if . ;\n",
    );
    assert_eq!(build_and_run(&entry), "true\n10\n");
}

/// R1's shape half: `bool` resolves through the registry now, so a module can
/// declare an enum of its own under that name -- and a static may not be
/// declared at one whose variants carry a payload, whatever it is called. The
/// parser cannot see this (variant fields are filled in after declaration
/// parsing), so the check lives in `check_static_decls`.
#[test]
fn static_at_a_payload_carrying_enum_named_bool_is_an_error() {
    let t = Tree::new("g3-forged");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         type: bool | A n i64 | B ;\n\
         static: FLAG bool = true ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("static `FLAG` has a non-scalar type `bool`")
            && err.contains("must carry no payload"),
        "unexpected diagnostic: {err}"
    );
}

// -- R2: the REPL seeds `core::bool` itself ---------------------------------

fn run_session(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    stdin
        .write_all((lines.join("\n") + "\n").as_bytes())
        .expect("writing stdin should succeed");
    drop(stdin);
    let out = child.wait_with_output().expect("repl should exit cleanly");
    assert!(out.status.success(), "repl exited with {:?}", out.status);
    String::from_utf8(out.stdout).expect("stdout should be utf8")
}

/// R2: a session resolves no package-name import at all, so it seeds
/// `core::bool` at startup from the real `lib/bool.sth`. `true` works on the
/// first line, `.` prints it as `true`/`false`, and `:stack` renders it the same
/// way -- with no import written.
#[test]
fn repl_seeds_core_bool_so_a_bare_true_works_on_the_first_line() {
    let out = run_session(&["true", ":stack", "true .", "false .", "true false"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "stack: true",
            "stack: true",
            "true",
            "stack: true",
            "false",
            "stack: true",
            "stack: true true false",
        ],
        "unexpected session transcript: {out}"
    );
}

/// R2/R5: the session registry holds no reserved slot either, so a session's own
/// `type:` lands where it was declared and still constructs and eliminates
/// across lines.
#[test]
fn repl_session_enum_constructs_and_eliminates_across_lines() {
    let out = run_session(&[
        "type: Color | Red | Green ;",
        ": which ( Color -- i64 ) ~[ ( Green ) drop 2 ] ~[ ( Red ) drop 1 ] Color? ;",
        "Green which .",
        "Red which .",
        "true",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Color",
            "defined which",
            "2",
            "stack: (empty)",
            "1",
            "stack: (empty)",
            "stack: true",
        ],
        "unexpected session transcript: {out}"
    );
}
