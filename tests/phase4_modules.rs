//! Phase 4 slice 5a goldens (phase 1): native multi-file compilation. Each
//! positive golden writes a closure of `.sth` files into a temp dir and asserts
//! the built binary's stdout; each negative golden asserts the distinguishing
//! wording of a located driver error, never a bare non-zero exit.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch closure of source files, removed on drop.
struct Closure(PathBuf);

impl Closure {
    fn new(tag: &str) -> Closure {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-mod-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Closure(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Closure {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build and run the entry file, returning `(stdout, exit_code)`.
fn build_and_run(entry: &Path) -> (String, i32) {
    let binary = driver::build(entry).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code().expect("process exits normally"),
    )
}

/// Build the entry file, expecting a diagnostic (the closure never links).
fn build_err(entry: &Path) -> String {
    match driver::build(entry) {
        Ok(_) => panic!("build should have failed"),
        Err(e) => e,
    }
}

#[test]
fn two_files_word_import_compiles_and_runs() {
    // Criterion 1: the importer calls `lib::p` and prints its result.
    let c = Closure::new("word-import");
    c.write("lib.sth", ": p ( -- i64 ) 42 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        "import: lib \"lib.sth\" ;\n: main ( -- ) lib::p . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn imported_type_is_nameable_and_runs() {
    // Criterion 2: the importer names `geo::Point` in an effect, constructs one,
    // reads a field, and prints it.
    let c = Closure::new("type-import");
    c.write("geo.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: geo \"geo.sth\" ;\n: mk ( -- geo::Point ) 3 4 geo::Point ;\n: main ( -- ) mk geo::Point>x . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

#[test]
fn same_named_types_in_two_modules_coexist() {
    // Criterion 3: two modules each declare `Point`; both compile and run under
    // the per-module duplicate rule.
    let c = Closure::new("dup-type");
    c.write("a.sth", "type: Point x i64 ;\nexport: Point ;\n");
    c.write("b.sth", "type: Point v i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: a \"a.sth\" ;\nimport: b \"b.sth\" ;\n: main ( -- ) 1 a::Point a::Point>x . 2 b::Point b::Point>v . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn import_cycle_is_located_error_naming_both() {
    // Criterion 4: a mutual import is a located cycle error naming both files.
    let c = Closure::new("cycle");
    c.write("a.sth", "import: b \"b.sth\" ;\n: main ( -- ) 0 . ;\n");
    c.write("b.sth", "import: a \"a.sth\" ;\n: q ( -- i64 ) 1 ;\n");
    let err = build_err(&c.path("a.sth"));
    assert!(err.contains("cycle"), "names the failure: {err}");
    assert!(err.contains("a.sth"), "names the first file: {err}");
    assert!(err.contains("b.sth"), "names the second file: {err}");
}

#[test]
fn self_import_is_located_error() {
    // Criterion 5: a file importing itself is the degenerate cycle.
    let c = Closure::new("self-import");
    let entry = c.write("a.sth", "import: self \"a.sth\" ;\n: main ( -- ) 0 . ;\n");
    let err = build_err(&entry);
    assert!(err.contains("cycle"), "a self-import is a cycle: {err}");
    assert!(err.contains("itself"), "names the degenerate case: {err}");
    assert!(err.contains("a.sth"), "names the file: {err}");
}

#[test]
fn missing_import_file_is_located_error() {
    // Criterion 6: an import whose path does not exist names the importing site
    // and the path.
    let c = Closure::new("missing");
    let entry = c.write(
        "main.sth",
        "import: x \"nope.sth\" ;\n: main ( -- ) 0 . ;\n",
    );
    let err = build_err(&entry);
    assert!(err.contains("nope.sth"), "names the missing path: {err}");
    assert!(err.contains("main.sth"), "names the importer: {err}");
    assert!(err.contains("line 1"), "locates the import: {err}");
}

#[test]
fn diamond_import_dedupes_by_canonical_path() {
    // Criterion 7: base is reached via both left and right, parsed once, and the
    // program runs.
    let c = Closure::new("diamond");
    c.write("base.sth", ": b ( -- i64 ) 100 ;\nexport: b ;\n");
    c.write(
        "left.sth",
        "import: base \"base.sth\" ;\n: lf ( -- i64 ) base::b ;\nexport: lf ;\n",
    );
    c.write(
        "right.sth",
        "import: base \"base.sth\" ;\n: rt ( -- i64 ) base::b ;\nexport: rt ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: l \"left.sth\" ;\nimport: r \"right.sth\" ;\n: main ( -- ) l::lf r::rt + . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "200\n");
    assert_eq!(code, 0);
}

#[test]
fn import_path_is_relative_to_importing_file() {
    // Criterion 8: `sub/mid.sth` imports `leaf.sth` resolved relative to its own
    // directory (`sub/`), not the entry's. If resolution were entry-relative the
    // build would fail to find `sub/leaf.sth`.
    let c = Closure::new("relative");
    c.write("sub/leaf.sth", ": w ( -- i64 ) 7 ;\nexport: w ;\n");
    c.write(
        "sub/mid.sth",
        "import: leaf \"leaf.sth\" ;\n: v ( -- i64 ) leaf::w ;\nexport: v ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: m \"sub/mid.sth\" ;\n: main ( -- ) m::v . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

#[test]
fn import_at_repl_is_located_rejection() {
    // Criterion 9: `import:` at the REPL is a located rejection, not a silent
    // parse error pointing at the `;`.
    let input = "import: q \"lib.sth\" ;\n:quit\n";
    let reader = BufReader::new(input.as_bytes());
    let mut out: Vec<u8> = Vec::new();
    sooth::repl::run(reader, &mut out).unwrap();
    let transcript = String::from_utf8(out).unwrap();
    assert!(
        transcript.contains("`import:` is not supported at the REPL yet"),
        "located rejection: {transcript}"
    );
    assert!(
        transcript.contains("line 1, col 1"),
        "located: {transcript}"
    );
    assert!(
        !transcript.contains("Semicolon"),
        "not the old misdirected error: {transcript}"
    );
}
