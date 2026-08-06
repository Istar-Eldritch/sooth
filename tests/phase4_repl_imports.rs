//! Phase 4 slice 5b goldens (phase 1): `import:` at the REPL. Each golden
//! writes a library closure into a temp dir, drives a REPL session over piped
//! stdin, and asserts the session's stdout (distinguishing wording for a
//! diagnostic, a rendered value for a positive path), never an IL string or a
//! bare exit code.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch directory of `.sth` library files, removed on drop.
struct LibDir(PathBuf);

impl LibDir {
    fn new(tag: &str) -> LibDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-replimp-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        LibDir(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for LibDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Drive a REPL session over `input` in a fresh process (so each session's
/// `RTLD_GLOBAL` symbols never leak into another test's), returning its
/// stdout. A fresh process is also how the spec's harness feeds piped stdin.
fn repl(input: &str) -> String {
    repl_in(input, None)
}

fn repl_in(input: &str, cwd: Option<&Path>) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sooth"));
    cmd.arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().expect("the sooth binary spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .expect("stdin writes");
    let out = child.wait_with_output().expect("the session exits");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn import_line(qualifier: &str, path: &Path) -> String {
    format!("import: {qualifier} \"{}\" ;", path.display())
}

#[test]
fn repl_import_word_is_callable_qualified() {
    // Criterion 1: import a two-word library, call `q::w`, run to a value.
    let d = LibDir::new("word");
    let lib = d.write(
        "lib.sth",
        ": w ( -- i64 ) 42 ;\n: other ( -- i64 ) 0 ;\nexport: w other ;\n",
    );
    let out = repl(&format!("{}\nq::w\n", import_line("q", &lib)));
    assert!(out.contains("imported q"), "acknowledges the import: {out}");
    assert!(out.contains("42"), "the qualified word runs to 42: {out}");
}

#[test]
fn repl_import_type_accessor_resolves() {
    // Criterion 2: import a type, name `q::T`, construct it, read `q::T>field`.
    let d = LibDir::new("type");
    let lib = d.write("lib.sth", "type: T v i64 ;\nexport: T ;\n");
    let out = repl(&format!("{}\n10 q::T q::T>v\n", import_line("q", &lib)));
    assert!(out.contains("10"), "constructs and reads the field: {out}");
}

#[test]
fn repl_import_type_resolves_in_signature_and_typedef_position() {
    // Criterion 2a: `q::T` resolves in type *position* -- a word's stack
    // effect and a `type:` field both name it (the regression witness for the
    // parser-level threading, which the body-position rewrite cannot reach).
    let d = LibDir::new("typos");
    let lib = d.write("lib.sth", "type: T v i64 ;\nexport: T ;\n");
    let out = repl(&format!(
        "{}\n: id ( q::T -- q::T ) ;\ntype: Wrap t q::T ;\n",
        import_line("q", &lib)
    ));
    assert!(
        out.contains("defined id"),
        "signature position resolves: {out}"
    );
    assert!(
        out.contains("defined type Wrap"),
        "type: field position resolves: {out}"
    );
}

#[test]
fn repl_double_colon_in_declared_name_is_located_rejection() {
    // Criterion 2b: a REPL-declared name containing `::` is a located
    // rejection naming the offending spelling (closes the internal tag's
    // forgeability).
    let out = repl("type: q::T x i64 ;\n: q::foo ( -- ) ;\n");
    assert!(
        out.contains("q::T") && out.contains("::"),
        "the type name is rejected, naming it: {out}"
    );
    assert!(
        out.contains("q::foo"),
        "the word name is rejected, naming it: {out}"
    );
}

#[test]
fn repl_qualified_private_name_is_not_exported() {
    // Criterion 3: a qualified reference to a real but non-exported name is
    // `not exported`, distinct from a genuinely absent one.
    let d = LibDir::new("priv");
    let lib = d.write(
        "lib.sth",
        ": pub ( -- i64 ) 1 ;\n: secret ( -- i64 ) 2 ;\nexport: pub ;\n",
    );
    let out = repl(&format!(
        "{}\nq::secret\nq::absent\n",
        import_line("q", &lib)
    ));
    assert!(
        out.contains("not exported") && out.contains("secret"),
        "a private name is `not exported`, naming it: {out}"
    );
    assert!(
        out.contains("unknown word") && out.contains("absent"),
        "an absent name is `unknown word`, not `not exported`: {out}"
    );
}

#[test]
fn repl_imported_nested_struct_ids_remap() {
    // Criterion 4: build an imported struct-of-struct, then read the inner
    // field back to its scalar. A local struct declared first forces a
    // non-zero base, so a remap that kept closure-local ids would point
    // `Outer`'s field at `Local` and the `q::Inner>a` read would mistype.
    let d = LibDir::new("nested");
    let lib = d.write(
        "lib.sth",
        "type: Inner a i64 ;\ntype: Outer i Inner ;\nexport: Inner Outer ;\n",
    );
    let out = repl(&format!(
        "type: Local z i64 ;\n{}\n1 q::Inner q::Outer q::Outer>i q::Inner>a\n",
        import_line("q", &lib)
    ));
    assert!(
        out.contains('1') && !out.contains("error"),
        "the nested field reads back to its scalar via remapped ids: {out}"
    );
}

#[test]
fn repl_import_path_is_relative_to_cwd() {
    // Criterion 5: the REPL's own top-level path resolves relative to the
    // process cwd (a relative path, resolved against the child's cwd).
    let d = LibDir::new("cwd");
    d.write("lib.sth", ": w ( -- i64 ) 7 ;\nexport: w ;\n");
    let out = repl_in("import: q \"lib.sth\" ;\nq::w\n", Some(&d.0));
    assert!(out.contains("imported q"), "relative path resolves: {out}");
    assert!(out.contains("7"), "the qualified word runs: {out}");
}

#[test]
fn repl_transitive_reexport_stays_closed() {
    // Criterion 6: a third file imported by the library stays invisible.
    let d = LibDir::new("transitive");
    d.write("base.sth", ": deep ( -- i64 ) 9 ;\nexport: deep ;\n");
    let lib = d.write(
        "lib.sth",
        "import: base \"base.sth\" ;\n: shallow ( -- i64 ) base::deep ;\nexport: shallow ;\n",
    );
    let out = repl(&format!(
        "{}\nq::shallow\nq::deep\n",
        import_line("q", &lib)
    ));
    assert!(
        out.contains("9"),
        "the module-0 word re-exports its result: {out}"
    );
    assert!(
        out.contains("unknown word") && out.contains("deep"),
        "the third file's name never crosses under q: {out}"
    );
}

#[test]
fn repl_import_cycle_and_missing_are_located() {
    // Criterion 7: a cycle and a missing file reuse the located native errors.
    let d = LibDir::new("cycle");
    let a = d.write("a.sth", "import: b \"b.sth\" ;\n: aw ( -- i64 ) 1 ;\n");
    d.write("b.sth", "import: a \"a.sth\" ;\n: bw ( -- i64 ) 2 ;\n");
    let cyc = repl(&format!("{}\n", import_line("q", &a)));
    assert!(
        cyc.contains("cycle"),
        "a cycle is a located cycle error: {cyc}"
    );

    let missing = d.0.join("nope.sth");
    let miss = repl(&format!("{}\n", import_line("q", &missing)));
    assert!(
        miss.contains("nope.sth"),
        "a missing file names the path: {miss}"
    );
}

#[test]
fn repl_malformed_import_is_located_error() {
    // Criterion 8: a malformed `import:` at the REPL is R9's located error.
    let out = repl("import: q ;\n");
    assert!(
        out.contains("line 1") && out.contains("import:"),
        "the malformed form is a located, construct-naming error: {out}"
    );
}

#[test]
fn repl_failed_import_leaves_session_intact() {
    // Criterion 9: a failed import leaves the session untouched -- a word
    // defined before it still runs after.
    let d = LibDir::new("intact");
    let missing = d.0.join("nope.sth");
    let out = repl(&format!(
        ": keep ( -- i64 ) 5 ;\n{}\nkeep\n",
        import_line("q", &missing)
    ));
    assert!(
        out.contains("defined keep"),
        "the prior definition took: {out}"
    );
    assert!(
        out.contains("5"),
        "the session survives the failed import and runs `keep`: {out}"
    );
}
