//! Phase 4 slice 5a goldens (phase 1): native multi-file compilation. Each
//! positive golden writes a closure of `.sth` files into a temp dir and asserts
//! the built binary's stdout; each negative golden asserts the distinguishing
//! wording of a located driver error, never a bare non-zero exit.

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
fn unexported_word_is_not_exported_error() {
    // Criterion 10: `grow` exists in `queue` but is never exported, so a
    // qualified call to it is a `not exported` error, distinct from unknown
    // word.
    let c = Closure::new("unexported-word");
    c.write(
        "queue.sth",
        ": grow ( -- i64 ) 1 ;\n: p ( -- i64 ) 42 ;\nexport: p ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: queue \"queue.sth\" ;\n: main ( -- ) queue::grow . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("not exported"),
        "distinguishing wording: {err}"
    );
    assert!(err.contains("grow"), "names the word: {err}");
    assert!(err.contains("queue"), "names the module: {err}");
    assert!(
        !err.contains("unknown word"),
        "must not be the unknown-word error: {err}"
    );
}

#[test]
fn absent_word_in_module_is_unknown_not_unexported() {
    // Criterion 11: `missing` does not exist in `queue` at all, so the error
    // is the ordinary unknown-word error, not `not exported` (differs from
    // criterion 10).
    let c = Closure::new("absent-word");
    c.write("queue.sth", ": p ( -- i64 ) 42 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        "import: queue \"queue.sth\" ;\n: main ( -- ) queue::missing . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        !err.contains("not exported"),
        "an absent name is not a visibility error: {err}"
    );
}

#[test]
fn qualified_accessors_get_set_peek_all_resolve() {
    // Criterion 12: an exported type's getter, setter, and peek accessors
    // (`>`, `<`, `|>`) all resolve when qualified.
    let c = Closure::new("accessors");
    c.write("geo.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: geo \"geo.sth\" ;\n: main ( -- ) 1 2 geo::Point geo::Point|>x . 9 geo::Point<x geo::Point>x . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn unexported_type_is_not_exported_error() {
    // Criterion 13: `geo::Point` is never exported, so naming it qualified is
    // a `not exported` error, not unknown word/type.
    let c = Closure::new("unexported-type");
    c.write("geo.sth", "type: Point x i64 ;\n");
    let entry = c.write(
        "main.sth",
        "import: geo \"geo.sth\" ;\n: mk ( -- geo::Point ) 3 geo::Point ;\n: main ( -- ) mk drop ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("not exported"),
        "distinguishing wording: {err}"
    );
    assert!(err.contains("Point"), "names the type: {err}");
    assert!(err.contains("geo"), "names the module: {err}");
}

#[test]
fn unexported_type_named_only_in_an_effect_is_not_exported_error() {
    // Criterion 13 (isolation): `geo::Point` never appears in a body call
    // here, only in `takes`'s effect, so this exercises the parser's own
    // effect-time visibility check independently of the body-call rewrite
    // path `unexported_type_is_not_exported_error` also exercises.
    let c = Closure::new("unexported-type-effect-only");
    c.write("geo.sth", "type: Point x i64 ;\n");
    let entry = c.write(
        "main.sth",
        "import: geo \"geo.sth\" ;\n: takes ( geo::Point -- ) drop ;\n: main ( -- ) 0 . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("not exported"),
        "distinguishing wording: {err}"
    );
    assert!(err.contains("Point"), "names the type: {err}");
    assert!(err.contains("geo"), "names the module: {err}");
}

#[test]
fn malformed_import_form_is_located_parse_error() {
    // Criterion 14: a malformed `import:` (missing qualifier or path string,
    // unterminated before `;`) is a located parse error naming the construct
    // and what it expected, not a generic token-level message.
    let c = Closure::new("malformed-import");

    // Missing qualifier: `import:` followed straight by the path string.
    let missing_qualifier = c.write("mq.sth", "import: \"lib.sth\" ;\n: main ( -- ) 0 . ;\n");
    let err = build_err(&missing_qualifier);
    assert!(
        err.contains("parse error") && err.contains("`import:`") && err.contains("qualifier"),
        "names `import:` and the missing qualifier: {err}"
    );
    assert!(err.contains("line 1"), "locates the qualifier error: {err}");

    // Missing path string: qualifier present, then `;` with no `\"...\"`.
    let missing_path = c.write("mp.sth", "import: lib ;\n: main ( -- ) 0 . ;\n");
    let err = build_err(&missing_path);
    assert!(
        err.contains("parse error") && err.contains("`import:`") && err.contains("path"),
        "names `import:` and the missing path string: {err}"
    );
    assert!(err.contains("line 1"), "locates the path error: {err}");

    // Unterminated before `;`: qualifier and path present, no terminator.
    let unterminated = c.write("un.sth", "import: lib \"lib.sth\"\n: main ( -- ) 0 . ;\n");
    let err = build_err(&unterminated);
    assert!(
        err.contains("parse error") && err.contains("`import:`") && err.contains("`;`"),
        "names `import:` and the missing terminator: {err}"
    );
    assert!(err.contains("line "), "locates the terminator error: {err}");
}

#[test]
fn malformed_export_form_is_located_parse_error() {
    // Criterion 14 (R9, `export:` half): a stray non-word token before `;` in
    // an `export:` list is a located parse error naming `export:`, not the
    // generic `expected Semicolon`.
    let c = Closure::new("malformed-export");
    let entry = c.write("main.sth", "export: \"oops\" ;\n: main ( -- ) 0 . ;\n");
    let err = build_err(&entry);
    assert!(
        err.contains("parse error") && err.contains("`export:`") && err.contains("`;`"),
        "names `export:` and the expected terminator: {err}"
    );
    assert!(err.contains("line 1"), "locates the export error: {err}");
}

#[test]
fn exported_word_naming_private_type_is_error() {
    // Criterion 15: `lib` exports `mk` but never exports `Res`, the struct
    // `mk`'s own effect returns -- the module author's bug, caught at `mk`'s
    // declaration.
    let c = Closure::new("private-in-export");
    c.write(
        "lib.sth",
        "type: Res n i64 ;\n: mk ( -- Res ) 1 Res ;\nexport: mk ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: lib \"lib.sth\" ;\n: main ( -- ) 0 . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("not exported"),
        "distinguishing wording: {err}"
    );
    assert!(err.contains("mk"), "names the word: {err}");
    assert!(err.contains("Res"), "names the private type: {err}");
}

#[test]
fn exported_word_naming_exported_type_is_accepted() {
    // Criterion 16: exporting `Res` too satisfies the rule (positive).
    let c = Closure::new("private-in-export-fixed");
    c.write(
        "lib.sth",
        "type: Res n i64 ;\n: mk ( -- Res ) 1 Res ;\nexport: mk Res ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: lib \"lib.sth\" ;\n: main ( -- ) lib::mk lib::Res>n . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn imported_linear_type_is_disposed_by_drop() {
    // Criterion 17: `Res` is linear (a `drop` overload) and exported; the
    // consumer disposes one with a bare `drop`, whose destructor glue runs
    // whether or not it was itself exported (D6/R19).
    let c = Closure::new("imported-linear-drop");
    c.write(
        "lib.sth",
        "type: Res n i64 ;\n: mk ( -- Res ) 7 Res ;\n: drop ( Res -- ) | r | r Res>n . ;\nexport: mk Res ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: lib \"lib.sth\" ;\n: main ( -- ) lib::mk drop ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "7\n", "the module's own destructor observably ran");
    assert_eq!(code, 0);
}

// Phase 4 goldens: selective import (the optional additive `| name... |`
// clause). A selectively imported name is exposed unqualified in addition to
// the always-available qualifier (R20), a private one is a visibility error,
// and a collision (with another selective import or a local word) is a located
// error naming both sources (R21). A selectively imported type brings its
// generated words unqualified too (R15c).

#[test]
fn selective_import_exposes_names_unqualified() {
    // Criterion 18: `p` is exposed unqualified by the `| p |` clause, and the
    // `lib::p` qualifier is still available alongside it.
    let c = Closure::new("selective-word");
    c.write("lib.sth", ": p ( -- i64 ) 42 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        "import: lib | p | \"lib.sth\" ;\n: main ( -- ) p . lib::p . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "42\n42\n", "unqualified and qualified both resolve");
    assert_eq!(code, 0);
}

#[test]
fn selective_import_of_private_name_is_error() {
    // Criterion 19: `grow` exists in `lib` but is never exported, so listing it
    // in the selective clause is the R16 visibility error (R20).
    let c = Closure::new("selective-private");
    c.write(
        "lib.sth",
        ": grow ( -- i64 ) 1 ;\n: p ( -- i64 ) 42 ;\nexport: p ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: lib | grow | \"lib.sth\" ;\n: main ( -- ) grow . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("not exported"),
        "distinguishing wording: {err}"
    );
    assert!(err.contains("grow"), "names the name: {err}");
    assert!(err.contains("lib"), "names the module: {err}");
}

#[test]
fn colliding_selective_imports_are_error_at_second() {
    // Criterion 20: two modules both expose `p` unqualified; the second
    // selective import is a located collision error naming both modules.
    let c = Closure::new("selective-collide");
    c.write("a.sth", ": p ( -- i64 ) 1 ;\nexport: p ;\n");
    c.write("b.sth", ": p ( -- i64 ) 2 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        "import: a | p | \"a.sth\" ;\nimport: b | p | \"b.sth\" ;\n: main ( -- ) p . ;\n",
    );
    let err = build_err(&entry);
    assert!(err.contains("collides"), "distinguishing wording: {err}");
    assert!(err.contains("`p`"), "names the colliding name: {err}");
    assert!(err.contains("`a`"), "names the first module: {err}");
    assert!(err.contains("`b`"), "names the second module: {err}");
}

#[test]
fn selective_import_colliding_with_local_word_is_error() {
    // Criterion 21: a selectively-exposed name that collides with a locally
    // defined word is the same located error, naming both sources.
    let c = Closure::new("selective-local-collide");
    c.write("lib.sth", ": p ( -- i64 ) 1 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        "import: lib | p | \"lib.sth\" ;\n: p ( -- i64 ) 2 ;\n: main ( -- ) p . ;\n",
    );
    let err = build_err(&entry);
    assert!(err.contains("collides"), "distinguishing wording: {err}");
    assert!(err.contains("`p`"), "names the colliding name: {err}");
    assert!(err.contains("`lib`"), "names the import source: {err}");
    assert!(err.contains("local"), "names the local definition: {err}");
}

#[test]
fn selective_import_of_type_exposes_members_unqualified() {
    // Criterion 21a: selectively importing `Point` exposes the type unqualified
    // and its generated words unqualified too (constructor, peek `|>`, set `<`,
    // get `>`), as one unit (R15c).
    let c = Closure::new("selective-type");
    c.write("geo.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: geo | Point | \"geo.sth\" ;\n: main ( -- ) 1 2 Point Point|>x . 9 Point<x Point>x . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n9\n", "peek/set/get all resolve unqualified");
    assert_eq!(code, 0);
}

#[test]
fn selective_type_import_member_collision_is_error() {
    // Criterion 21b: two modules each expose a `Point`; selectively importing
    // both collides on the base name (and thus on every generated member),
    // a located error at the second naming both modules.
    let c = Closure::new("selective-type-collide");
    c.write("a.sth", "type: Point x i64 ;\nexport: Point ;\n");
    c.write("b.sth", "type: Point v i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: a | Point | \"a.sth\" ;\nimport: b | Point | \"b.sth\" ;\n: main ( -- ) 0 . ;\n",
    );
    let err = build_err(&entry);
    assert!(err.contains("collides"), "distinguishing wording: {err}");
    assert!(err.contains("`Point`"), "names the colliding type: {err}");
    assert!(err.contains("`a`"), "names the first module: {err}");
    assert!(err.contains("`b`"), "names the second module: {err}");
}

#[test]
fn modules_example_builds_and_runs() {
    // Criterion 22: the committed dogfood, `examples/modules.sth` importing a
    // type from `examples/modules_point.sth` and words from
    // `examples/modules_ops.sth`, builds, links, and runs.
    let binary = sooth::driver::build(std::path::Path::new("examples/modules.sth"))
        .expect("the dogfood closure should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "4\n52\n");
    assert_eq!(output.status.code().unwrap(), 0);
}
