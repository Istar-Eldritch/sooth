//! Phase 4 slice 5a goldens (phase 1): native multi-file compilation. Each
//! positive golden writes a closure of `.sth` files into a temp dir and asserts
//! the built binary's stdout; each negative golden asserts the distinguishing
//! wording of a located driver error, never a bare non-zero exit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

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
        std::fs::write(&path, common::fixture_source(name, contents)).unwrap();
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
    let binary = driver::build_with_manifest(entry, common::manifest_for(entry).as_deref())
        .expect("build should succeed");
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
    match driver::build_with_manifest(entry, common::manifest_for(entry).as_deref()) {
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
        "import: \"lib.sth\" lib ;\n: main ( -- ) lib::p . ;\n",
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
        "import: \"geo.sth\" geo ;\n: mk ( -- geo::Point ) 3 4 geo::Point ;\n: main ( -- ) mk &x @ swap drop . ;\n",
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
        "import: \"a.sth\" a ;\nimport: \"b.sth\" b ;\n: main ( -- ) 1 a::Point &x @ swap drop . 2 b::Point &v @ swap drop . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(code, 0);
}

#[test]
fn import_cycle_is_located_error_naming_both() {
    // Criterion 4: a mutual import is a located cycle error naming both files.
    let c = Closure::new("cycle");
    c.write("a.sth", "import: \"b.sth\" b ;\n: main ( -- ) 0 . ;\n");
    c.write("b.sth", "import: \"a.sth\" a ;\n: q ( -- i64 ) 1 ;\n");
    let err = build_err(&c.path("a.sth"));
    assert!(err.contains("cycle"), "names the failure: {err}");
    assert!(err.contains("a.sth"), "names the first file: {err}");
    assert!(err.contains("b.sth"), "names the second file: {err}");
}

#[test]
fn self_import_is_located_error() {
    // Criterion 5: a file importing itself is the degenerate cycle.
    let c = Closure::new("self-import");
    let entry = c.write("a.sth", "import: \"a.sth\" self ;\n: main ( -- ) 0 . ;\n");
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
        "import: \"nope.sth\" x ;\n: main ( -- ) 0 . ;\n",
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
        "import: \"base.sth\" base ;\n: lf ( -- i64 ) base::b ;\nexport: lf ;\n",
    );
    c.write(
        "right.sth",
        "import: \"base.sth\" base ;\n: rt ( -- i64 ) base::b ;\nexport: rt ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"left.sth\" l ;\nimport: \"right.sth\" r ;\n: main ( -- ) l::lf r::rt add . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "200\n");
    assert_eq!(code, 0);
}

#[test]
fn core_word_called_from_an_imported_module_resolves() {
    // P8.S2 (R3/R8): `if`/`eq` are `core` words reached by `import:` now, not a
    // prelude injected into every module, so the interesting witness moved: an
    // *imported* module importing `core` itself and using both. It is a real
    // package because `--manifest` resolves the entry file only (S1b R3), so a
    // dependency that names `core` needs an ancestor manifest of its own --
    // which also makes its sibling import a module name rather than a path.
    let c = Closure::new("core-import");
    c.write("sooth.pkg", &common::fixture_package("modfx"));
    c.write(
        "parity.sth",
        "import: core::prelude * ;\n\
         : parity ( i64 -- i64 ) 2 mod 0 eq ~[ 10 ] ~[ 20 ] if ;\nexport: parity ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: self::parity p ;\n: main ( -- ) 7 p::parity . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "20\n");
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
        "import: \"leaf.sth\" leaf ;\n: v ( -- i64 ) leaf::w ;\nexport: v ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"sub/mid.sth\" m ;\n: main ( -- ) m::v . ;\n",
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
        "import: \"queue.sth\" queue ;\n: main ( -- ) queue::grow . ;\n",
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
        "import: \"queue.sth\" queue ;\n: main ( -- ) queue::missing . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        !err.contains("not exported"),
        "an absent name is not a visibility error: {err}"
    );
}

#[test]
fn qualified_constructor_destructure_and_projection_all_resolve() {
    // Criterion 12: an exported type's generated words (`Point`, `Point>`) all
    // resolve when qualified, and a projection off the imported type resolves by
    // receiver, so it needs no qualification of its own.
    let c = Closure::new("accessors");
    c.write("geo.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: \"geo.sth\" geo ;\n: main ( -- ) 1 2 geo::Point &x @ . &!x 9 ! &x @ . geo::Point> drop drop ;\n",
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
        "import: \"geo.sth\" geo ;\n: mk ( -- geo::Point ) 3 geo::Point ;\n: main ( -- ) mk drop ;\n",
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
        "import: \"geo.sth\" geo ;\n: takes ( geo::Point -- ) drop ;\n: main ( -- ) 0 . ;\n",
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
    // Criterion 14: a malformed `import:` (no target at all, or unterminated
    // before `;`) is a located parse error naming the construct and what it
    // expected, not a generic token-level message. Neither an elided qualifier
    // (`import: "lib.sth" ;`, defaulting to the file stem) nor a bare module
    // name (`import: lib ;`) is malformed under the P8 slice 1a grammar.
    let c = Closure::new("malformed-import");

    // No target: `import:` followed straight by its terminator.
    let missing_target = c.write("mt.sth", "import: ;\n: main ( -- ) 0 . ;\n");
    let err = build_err(&missing_target);
    assert!(
        err.contains("parse error") && err.contains("`import:`") && err.contains("target"),
        "names `import:` and the missing target: {err}"
    );
    assert!(err.contains("line 1"), "locates the target error: {err}");

    // Unterminated before `;`: target and qualifier present, no terminator.
    let unterminated = c.write("un.sth", "import: \"lib.sth\" lib \n: main ( -- ) 0 . ;\n");
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
        "import: \"lib.sth\" lib ;\n: main ( -- ) 0 . ;\n",
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
        "import: \"lib.sth\" lib ;\n: main ( -- ) lib::mk lib::Res> . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn imported_linear_type_dropped_without_importing_it_is_error() {
    // Slice 8b, R6 (supersedes slice 5a Criterion 17): disposing an imported
    // resource type with a bare `drop` runs a destructor declared in another
    // module, so under a qualified-only import it is a located error naming the
    // remedy -- importing the type by name.
    let c = Closure::new("imported-linear-drop-ungated");
    c.write(
        "lib.sth",
        "type: Res n i64 ;\n: mk ( -- Res ) 7 Res ;\n: drop ( Res -- ) | r | r Res> . ;\nexport: mk Res ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" lib ;\n: main ( -- ) lib::mk drop ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("cannot `drop` a value of type `lib::Res` in `main`"),
        "names the type, qualifier, and caller: {err}"
    );
    assert!(
        err.contains("declared in module `lib`, which this module has not imported by name"),
        "names the declaring module and the cause: {err}"
    );
    assert!(
        err.contains("add `Res` to the import (`import: \"...\" lib | Res | ;`)"),
        "names the remedy: {err}"
    );
}

#[test]
fn imported_linear_type_dropped_after_selective_import_ok() {
    // Slice 8b, R6 (positive companion): importing `Res` by name makes its
    // override visible, so a bare `drop` runs `lib`'s destructor (prints `7`).
    let c = Closure::new("imported-linear-drop-selective");
    c.write(
        "lib.sth",
        "type: Res n i64 ;\n: mk ( -- Res ) 7 Res ;\n: drop ( Res -- ) | r | r Res> . ;\nexport: mk Res ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" lib | Res | ;\n: main ( -- ) lib::mk drop ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "7\n", "the module's own destructor observably ran");
    assert_eq!(code, 0);
}

#[test]
fn imported_resource_qualified_only_non_disposal_uses_compile() {
    // Slice 8b, R7: only a bare `drop` reaches the gate. Under a qualified-only
    // import, naming the type in an effect, holding a value, forwarding it to
    // another word, and `&`-reading a field all still compile -- the value is
    // disposed in `lib`, which declares `Res`.
    let c = Closure::new("imported-linear-nondisposal");
    c.write(
        "lib.sth",
        concat!(
            "type: Res n i64 ;\n",
            ": mk ( -- Res ) 7 Res ;\n",
            ": sink ( Res -- ) drop ;\n",
            ": peek ( &Res -- i64 ) &n @ ;\n",
            ": drop ( Res -- ) | r | r Res> . ;\n",
            "export: mk Res sink peek ;\n",
        ),
    );
    let entry = c.write(
        "main.sth",
        concat!(
            "import: \"lib.sth\" lib ;\n",
            ": hold ( -- lib::Res ) lib::mk ;\n",
            ": main ( -- ) hold | r | &r lib::peek . r lib::sink ;\n",
        ),
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "7\n7\n", "the borrow reads, then `lib` disposes it");
    assert_eq!(code, 0);
}

#[test]
fn library_combinator_disposing_its_own_resource_compiles_under_qualified_only_import() {
    // Round-1 review bug 3: a quotation-taking word (`with`) is a combinator,
    // so it is never checked standalone -- only spliced into each caller's
    // own body. Before this fix the splice reused the caller's `Ctx` whole,
    // so D1's drop-visibility gate ran `with`'s internal `r drop` against
    // `main`'s module instead of `lib`'s (the module that actually declares
    // `Res` and its `drop` override), rejecting code that never names `Res`
    // at all. `main` imports `lib` qualified-only -- no `Res`, no `drop` --
    // since disposing the resource is entirely `with`'s own affair.
    let c = Closure::new("library-combinator-self-dispose");
    c.write(
        "lib.sth",
        concat!(
            "type: Res n i64 ;\n",
            ": mk ( -- Res ) 1 Res ;\n",
            ": drop ( Res -- ) | r | r Res> . ;\n",
            ": with inline ( [ i64 -- i64 ] -- i64 ) | q | mk | r | &r &n @ q call r drop ;\n",
            "export: with ;\n",
        ),
    );
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" lib ;\n: main ( -- ) [ 1 add ] lib::with . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(
        stdout, "1\n2\n",
        "with's destructor ran (prints 1), then its own result (2)"
    );
    assert_eq!(code, 0);
}

// Slice 8b goldens (R13): 8a's operator module-scoping fix. A bare operator
// resolves against the overloads visible to the calling module, so a module's
// own overload is reachable bare even in a >=2-module build (where the decl is
// mangled per module), a selectively-unimported overload does not leak, and the
// single-module corpus is byte-for-byte unchanged.

#[test]
fn own_module_operator_overload_reachable_bare_in_multi_module() {
    // R13: `main` declares `add` for its own `Vec2`, with `lib` in the closure
    // forcing the operator decl to mangle to `add__m{k}`. The bare `add` now
    // resolves to the own overload; before the fix `env.get("+")` missed it and
    // the call fell to the builtin, which rejects the struct operands.
    let c = Closure::new("own-operator-multi");
    c.write("lib.sth", ": p ( -- i64 ) 0 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        concat!(
            "import: \"lib.sth\" lib ;\n",
            "type: Vec2 x i64 y i64 ;\n",
            ": add ( Vec2 Vec2 -- Vec2 ) drop ;\n",
            ": main ( -- ) lib::p . 1 2 Vec2 3 4 Vec2 add &x @ . drop ;\n",
        ),
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(
        stdout, "0\n1\n",
        "the own `add` overload dispatched, keeping the first operand's x"
    );
    assert_eq!(code, 0);
}

#[test]
fn own_module_operator_overload_reachable_bare_in_multi_module_poly_body() {
    // R13: same fix, exercised through `poly_delegate_op` (the poly-body
    // operator path) rather than `check_term`'s concrete path. `probe` is
    // polymorphic (`'T`), so its body is checked by `check_poly_body`; the
    // `add` call inside it is on a fully-concrete suffix (two `Vec2`s), so it
    // still needs the calling module's scoped candidates to find `main`'s
    // own mangled `add` overload.
    let c = Closure::new("own-operator-multi-poly");
    c.write("lib.sth", ": p ( -- i64 ) 0 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        concat!(
            "import: \"lib.sth\" lib ;\n",
            "type: Vec2 x i64 y i64 ;\n",
            ": add ( Vec2 Vec2 -- Vec2 ) drop ;\n",
            ": probe ( 'T -- 'T i64 ) 1 2 Vec2 3 4 Vec2 add Vec2> drop ;\n",
            ": main ( -- ) lib::p . 42 probe . . ;\n",
        ),
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(
        stdout, "0\n1\n42\n",
        "the own `+` overload dispatched inside the poly body, keeping the first operand's x"
    );
    assert_eq!(code, 0);
}

#[test]
fn single_module_operator_overload_from_poly_body_dispatches() {
    // The single-module half of the poly-body operator path, whose only golden
    // was the >=2-module one above. R10 routes a one-module closure through
    // `scoped_operator_overloads` as well, so `poly_delegate_op` now receives a
    // `Some(candidates)` set where it used to fall back to the flat
    // `env.get(name)` -- and its `UserOverload` arm looks the chosen symbol back
    // up in that same set, `expect`-ing a hit. Restoring the `modules.len() < 2`
    // bail makes the set empty and the overload invisible, failing this test.
    let c = Closure::new("single-operator-poly");
    let entry = c.write(
        "main.sth",
        concat!(
            "type: Vec2 x i64 y i64 ;\n",
            ": add ( Vec2 Vec2 -- Vec2 ) drop ;\n",
            ": probe ( 'T -- 'T i64 ) 1 2 Vec2 3 4 Vec2 add Vec2> drop ;\n",
            ": main ( -- ) 42 probe . . ;\n",
        ),
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(
        stdout, "1\n42\n",
        "the own `add` overload dispatched inside the poly body, keeping the \
         first operand's x, and the passthrough `'T` survived"
    );
    assert_eq!(code, 0);
}

#[test]
fn selectively_imported_operator_does_not_hijack_unrelated_module() {
    // R13: `main` imports `x`'s type qualified-only, so it holds `x::XT` values
    // but never imported `x`'s `add` by name. A bare `add` on two XT operands is the
    // ordinary operand-mismatch error, not a silent dispatch to `x`'s word.
    let c = Closure::new("operator-no-hijack");
    c.write(
        "x.sth",
        concat!(
            "type: XT v i64 ;\n",
            ": mk ( i64 -- XT ) XT ;\n",
            ": add ( XT XT -- XT ) drop ;\n",
            "export: XT mk ;\n",
        ),
    );
    let entry = c.write(
        "main.sth",
        concat!(
            "import: \"x.sth\" x ;\n",
            ": main ( -- ) 1 x::mk 2 x::mk add &v @ swap drop . ;\n",
        ),
    );
    let err = build_err(&entry);
    assert!(
        err.contains("`add` requires two operands of the same numeric type"),
        "the bare `add` falls to the builtin, not x's overload: {err}"
    );
    assert!(
        err.contains("found `XT` and `XT`"),
        "names the rejected struct operand type: {err}"
    );
}

#[test]
fn selectively_imported_operator_does_not_hijack_own_modules_plain_use() {
    // Regression: `main` selectively imports `v`'s `add` overload for `Vec2`
    // *and* uses plain `add` on two `i64`s elsewhere. Before the fix, the
    // selective-import rewrite branch mangled every bare `add` in `main` to
    // `v`'s overload unconditionally (no `is_operator_dispatch_name` guard,
    // unlike the own-module branch), so the plain `i64 add` failed a type
    // mismatch expecting `Vec2`. Both uses must now resolve correctly: the
    // `Vec2` pair to `v`'s overload, the `i64` pair to the builtin.
    let c = Closure::new("operator-selective-no-self-hijack");
    c.write(
        "v.sth",
        concat!(
            "type: Vec2 x i64 y i64 ;\n",
            ": add ( Vec2 Vec2 -- Vec2 )\n",
            "  | a b |\n",
            "  a &x @ swap drop b &x @ swap drop add\n",
            "  a &y @ swap drop b &y @ swap drop add\n",
            "  Vec2 ;\n",
            "export: Vec2 add ;\n",
        ),
    );
    let entry = c.write(
        "main.sth",
        concat!(
            "import: \"v.sth\" v | Vec2 add | ;\n",
            ": main ( -- )\n",
            "  1 2 Vec2 3 4 Vec2 add &x @ swap drop .\n",
            "  1 2 add . ;\n",
        ),
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(
        stdout, "4\n3\n",
        "the Vec2 pair dispatches to v's overload (4), the i64 pair to the builtin (3)"
    );
    assert_eq!(code, 0);
}

#[test]
fn single_module_operator_overload_unchanged() {
    // R13 (regression / mutation guard): a single-file program overloading
    // `add` on a struct compiles and runs exactly as before. Its decl is
    // mangled like any other (`add__m0`, so the word cannot own a bare libc
    // symbol) while the call site stays bare, so this passes only if
    // `scoped_operator_overloads` assembles the candidate under the mangled
    // key for a one-module closure as well as a multi-module one.
    let c = Closure::new("single-operator");
    let entry = c.write(
        "main.sth",
        concat!(
            "type: Vec2 x i64 y i64 ;\n",
            ": add ( Vec2 Vec2 -- Vec2 ) drop ;\n",
            ": main ( -- ) 1 2 Vec2 3 4 Vec2 add &x @ swap drop . ;\n",
        ),
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "1\n", "the single-file `add` overload dispatched");
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
        "import: \"lib.sth\" lib | p | ;\n: main ( -- ) p . lib::p . ;\n",
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
        "import: \"lib.sth\" lib | grow | ;\n: main ( -- ) grow . ;\n",
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
        "import: \"a.sth\" a | p | ;\nimport: \"b.sth\" b | p | ;\n: main ( -- ) p . ;\n",
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
        "import: \"lib.sth\" lib | p | ;\n: p ( -- i64 ) 2 ;\n: main ( -- ) p . ;\n",
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
    // and its generated words unqualified too (constructor, destructure `>`), as
    // one unit (R15c).
    let c = Closure::new("selective-type");
    c.write("geo.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = c.write(
        "main.sth",
        "import: \"geo.sth\" geo | Point | ;\n: main ( -- ) 1 2 Point &x @ . &!x 9 ! &x @ . Point> drop drop ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(
        stdout, "1\n9\n",
        "constructor and destructure resolve unqualified"
    );
    assert_eq!(code, 0);
}

#[test]
fn selective_import_of_builtin_name_with_mismatched_arity_is_error() {
    // Phase 4 slice 8a, R4 (import mirror): `lib` overloads `add` unary on
    // `Vec2`, but the builtin `add` is binary; nothing else forbids this
    // import outright (no local `add`, no other selective import of `add`), so
    // the arity mismatch against the builtin is the only thing left to
    // reject it, at the import site rather than surfacing as an ambiguity
    // at a call site.
    let c = Closure::new("selective-arity-clash");
    c.write(
        "lib.sth",
        "type: Vec2 x i64 y i64 ;\n: add ( Vec2 -- Vec2 ) ;\nexport: add Vec2 ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" lib | add | ;\n: main ( -- ) ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("selective import of `add`") && err.contains("lib"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("takes 1 input")
            && err.contains("the builtin `add` takes 2")
            && err.contains("agree on input count"),
        "unexpected message: {err}"
    );
}

#[test]
fn qualified_call_to_builtin_named_overload_dispatches_to_user_word() {
    // A module-boundary counterpart to slice 8a's builtin-name overloading:
    // `lib` overloads `add` on `Vec2` and its own body sums the two fields with
    // the plain `i64` `add` -- a bare use of the builtin from *inside* the
    // overload's own declaring module. The resolver must leave that bare use
    // unrewritten (deferring to `check_operator`'s operand-type dispatch) even
    // though it eagerly rewrites `main`'s qualified `v::add` to the mangled
    // overload; rewriting the bare use too would force it onto the `Vec2`
    // signature and misreport the `i64` operands as a type mismatch.
    let c = Closure::new("qualified-builtin-overload");
    c.write(
        "lib.sth",
        "export: Vec2 add ;\n\
         type: Vec2 x i64 y i64 ;\n\
         : add ( Vec2 Vec2 -- Vec2 ) | a b | a &x @ swap drop b &x @ swap drop add a &y @ swap drop b &y @ swap drop add Vec2 ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" v | Vec2 | ;\n\
         : main ( -- ) 1 2 Vec2 3 4 Vec2 v::add &x @ swap drop . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "4\n");
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
        "import: \"a.sth\" a | Point | ;\nimport: \"b.sth\" b | Point | ;\n: main ( -- ) 0 . ;\n",
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
    let binary = common::build_example("examples/modules.sth");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "4\n52\n");
    assert_eq!(output.status.code().unwrap(), 0);
}

#[test]
fn unrelated_modules_generic_and_concrete_same_name_do_not_collide() {
    // Slice 8a fix 3 (R5, module-scoped): a poly word in one module and an
    // unrelated concrete word of the same name and arity in a *different*
    // module that does not import it must not trip the generic/concrete
    // overlap check -- it was keyed by name alone before this fix, global
    // across the whole program. `main` imports both `g` and `c`, but `g` and
    // `c` do not import each other.
    //
    // This is a shape check, not the guard: it stays green with the fix
    // reverted, because `resolve::mangle` is unconditional per-module, so two
    // modules' bare `bump`s never collide by string in a real multi-file
    // build and the global key could not fire here. The discriminating test
    // is the direct `WordDef`-construction unit test in `check.rs`, which
    // does fail when reverted. Kept because it pins the user-visible
    // behaviour end to end.
    let c = Closure::new("overlap-unrelated-modules");
    c.write("g.sth", ": bump ( 'T -- 'T ) ;\nexport: bump ;\n");
    c.write(
        "c.sth",
        "type: Vec2 x i64 y i64 ;\n: bump ( Vec2 -- Vec2 ) ;\nexport: bump Vec2 ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"g.sth\" g ;\nimport: \"c.sth\" c ;\n: main ( -- ) 5 g::bump . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

#[test]
fn same_module_generic_and_concrete_overlap_still_rejected_across_files() {
    // Slice 8a fix 3 (R5, module-scoped): the module-scoping in the fix above
    // must not weaken a genuine same-module collision -- a poly `bump` and a
    // concrete `bump` of the same arity declared in the *same* module (here,
    // the importer's own `main.sth`) are still rejected, even though the
    // program also spans an unrelated second file.
    let c = Closure::new("overlap-same-module");
    c.write("lib.sth", ": p ( -- i64 ) 1 ;\nexport: p ;\n");
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" lib ;\n\
type: Vec2 x i64 y i64 ;\n\
: bump ( Vec2 -- Vec2 ) ;\n\
: bump ( 'T -- 'T ) ;\n\
: main ( -- ) lib::p . ;\n",
    );
    let err = build_err(&entry);
    assert!(
        err.contains("generic overload") && err.contains("overlaps a concrete overload of `bump`"),
        "unexpected message: {err}"
    );
}

#[test]
fn unused_import_with_a_colliding_span_does_not_swallow_a_later_print() {
    // A poly instantiation site is recorded keyed by its call site's `Span`
    // alone; without a `module` id in that key, an imported (but never
    // called) library's own poly call site can land on the identical
    // (line, col) as an unrelated call in the importing file, and lowering
    // then misreads the importer's call through the library's instantiation
    // record instead of the importer's own dispatch. `lib.sth`'s `1.5 q` sits
    // at line 4, col 7; `main.sth`'s `7 p . ;` is laid out so its `.` lands at
    // the same (line 4, col 7) -- column-for-column identical to `lib.sth`'s
    // `q`, in a different file. `useq` (the only imported name) is never
    // called; only the local, unrelated poly word `p` is.
    let c = Closure::new("unused-import-span-collision");
    c.write(
        "lib.sth",
        ": q ( 'T -- 'T ) ;\n\
         \n\
         : useq ( -- f64 )\n\
         \x20 1.5 q ;\n\
         export: useq ;\n",
    );
    let entry = c.write(
        "main.sth",
        "import: \"lib.sth\" lib | useq | ;\n\
         : p ( 'T -- 'T ) ;\n\
         : main ( -- )\n\
         \x20 7 p . ;\n",
    );
    let (stdout, code) = build_and_run(&entry);
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}
