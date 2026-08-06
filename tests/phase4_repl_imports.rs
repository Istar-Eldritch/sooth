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

/// Phase 2: a session driven one line at a time, reading back each line's own
/// output before sending the next, so the test can edit a library file on
/// disk *between* two `import:` lines in the same process (proving R6's
/// frozen-caller behavior against a real reload, not two different paths).
/// `Session`'s writer is `std::io::stdout()`'s `LineWriter`, which flushes on
/// every embedded newline regardless of pipe-vs-tty, so one line in yields
/// one line out.
struct InteractiveRepl {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl InteractiveRepl {
    fn spawn(cwd: &Path) -> InteractiveRepl {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
            .arg("repl")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the sooth binary spawns");
        let stdin = child.stdin.take().unwrap();
        let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        InteractiveRepl {
            child,
            stdin,
            stdout,
        }
    }

    /// Send one line, read back exactly the one line of output it produces.
    fn send(&mut self, line: &str) -> String {
        writeln!(self.stdin, "{line}").expect("stdin writes");
        self.stdin.flush().expect("stdin flushes");
        let mut out = String::new();
        std::io::BufRead::read_line(&mut self.stdout, &mut out).expect("stdout reads");
        out
    }

    /// Close stdin (EOF) and let the session exit.
    fn finish(self) {
        let InteractiveRepl {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        child.wait().expect("the session exits");
    }
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

#[test]
fn repl_reimport_freezes_existing_caller() {
    // Criterion 10: redefine the library, re-import; a caller compiled
    // against the first epoch stays frozen while a fresh reference sees the
    // new resolution (R6). Editing the file *between* the two `import:`
    // lines, in the same process, is the point -- a static two-file rebind
    // would not exercise reload of the *same* path.
    let d = LibDir::new("reimport");
    let lib = d.write("lib.sth", ": w ( -- i64 ) 1 ;\nexport: w ;\n");
    let mut r = InteractiveRepl::spawn(&d.0);

    let out = r.send(&import_line("q", &lib));
    assert!(out.contains("imported q"), "first import: {out}");
    let out = r.send(": caller ( -- i64 ) q::w ;");
    assert!(out.contains("defined caller"), "defines caller: {out}");
    let out = r.send("caller");
    assert!(out.contains('1'), "caller runs the first epoch's w: {out}");

    d.write("lib.sth", ": w ( -- i64 ) 2 ;\nexport: w ;\n");
    let out = r.send(&import_line("q", &lib));
    assert!(out.contains("imported q"), "re-import: {out}");

    let out = r.send("caller");
    assert!(
        out.contains('1'),
        "the already-compiled caller stays frozen on the old epoch: {out}"
    );
    let out = r.send("q::w");
    assert!(
        out.contains('2'),
        "a fresh reference resolves against the new epoch: {out}"
    );
    r.finish();
}

#[test]
fn repl_reimport_of_type_leaves_unrelated_typedef_unaffected() {
    // Criterion 10a: reload a library exporting a type, then declare an
    // unrelated `type:` line; it succeeds -- a regression witness for the
    // duplicate-type-name hazard (each import event's rows carry a fresh
    // module id, so a reload's two same-`name_static` rows never collide as
    // a literal repeat).
    let d = LibDir::new("reimport-type");
    let lib = d.write("lib.sth", "type: T v i64 ;\nexport: T ;\n");
    let mut r = InteractiveRepl::spawn(&d.0);
    r.send(&import_line("q", &lib));
    r.send(&import_line("q", &lib));
    let out = r.send("type: Other z i64 ;");
    assert!(
        out.contains("defined type Other"),
        "an unrelated type: line still succeeds after a reload: {out}"
    );
    r.finish();
}

#[test]
fn repl_reimport_of_type_resolution_does_not_diverge() {
    // Criterion 10b: reload a library exporting a type with a changed shape;
    // a value built via the post-reload `q::T` constructor and a word typed
    // after the reload both agree on the new shape.
    let d = LibDir::new("reimport-shape");
    let lib = d.write("lib.sth", "type: T v i64 ;\nexport: T ;\n");
    let mut r = InteractiveRepl::spawn(&d.0);
    r.send(&import_line("q", &lib));

    d.write("lib.sth", "type: T v i64 w i64 ;\nexport: T ;\n");
    r.send(&import_line("q", &lib));

    let out = r.send("1 2 q::T q::T>w");
    assert!(
        out.contains('2'),
        "the post-reload constructor takes the new shape: {out}"
    );
    let out = r.send(": id ( q::T -- q::T ) ;");
    assert!(
        out.contains("defined id"),
        "a signature typed after the reload resolves against the same new decl: {out}"
    );
    r.finish();
}

#[test]
fn repl_qualifier_rebind_frozen_and_rejudged() {
    // Criterion 11: rebind `q` to a different file; a frozen `q::old` keeps
    // working, while a *new* reference to `q::old` is judged against the new
    // file only (not exported, since the new file declares `old` privately),
    // never a stale hit on the old file's export status.
    let d = LibDir::new("rebind");
    let lib_a = d.write("a.sth", ": old ( -- i64 ) 1 ;\nexport: old ;\n");
    let lib_b = d.write(
        "b.sth",
        ": old ( -- i64 ) 9 ;\n: other ( -- i64 ) 2 ;\nexport: other ;\n",
    );
    let mut r = InteractiveRepl::spawn(&d.0);

    r.send(&import_line("q", &lib_a));
    r.send(": caller ( -- i64 ) q::old ;");
    let out = r.send("caller");
    assert!(
        out.contains('1'),
        "caller runs against the first file: {out}"
    );

    let out = r.send(&import_line("q", &lib_b));
    assert!(
        out.contains("imported q"),
        "rebinds to a different file: {out}"
    );

    let out = r.send("caller");
    assert!(
        out.contains('1'),
        "the frozen caller still runs the first file's `old`: {out}"
    );
    let out = r.send("q::old");
    assert!(
        out.contains("not exported") && out.contains("old"),
        "a new reference is judged against the new file only, which declares `old` privately: {out}"
    );
    let out = r.send("q::other");
    assert!(
        out.contains('2'),
        "the new file's own export resolves: {out}"
    );
    r.finish();
}

#[test]
fn repl_import_of_library_declaring_main_is_rejected() {
    // Criterion 12: an imported file declaring `main` is a located rejection
    // naming the file and the word, and leaves the session untouched.
    let d = LibDir::new("main");
    let lib = d.write(
        "lib.sth",
        ": helper ( -- i64 ) 1 ;\n: main ( -- ) ;\nexport: helper ;\n",
    );
    let out = repl(&format!(
        ": keep ( -- i64 ) 5 ;\n{}\nkeep\n",
        import_line("q", &lib)
    ));
    assert!(
        out.contains("main") && out.contains("lib.sth"),
        "the rejection names the file and the word: {out}"
    );
    assert!(
        out.contains("defined keep") && out.contains('5'),
        "the session survives the rejected import and runs `keep`: {out}"
    );
}
