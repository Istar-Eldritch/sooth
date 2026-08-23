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
        "import: intrinsics * ;\n: main ( -- ) True drop ;\n",
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
        "import: intrinsics * ;\n: w ( Bool -- ) drop ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("unknown type `Bool` at line 2"),
        "unexpected diagnostic: {err}"
    );
}

/// P7.S3q-follow: the prelude hub carries the `true`/`false` constructors
/// *and* the type name `Bool` now -- a struct/enum name reached only through
/// a hub's `export:` list resolves in an effect signature the same way it
/// already did in term position, closing the gap this golden used to pin the
/// other side of (`git log` on this test's prior body has the pre-fix
/// diagnostic). The `.` overload stays the one thing that still cannot cross
/// a hub: an operator overload's candidate lookup considers the calling
/// module and the module it selectively imported the name from, one hop, so
/// a hub in between still hides the declaring module -- an orthogonal
/// mechanism, not narrowed by this fix.
#[test]
fn the_prelude_hub_carries_the_constructors_and_the_type_name_but_not_the_print_overload() {
    let t = Tree::new("g1-hub");
    let ok = t.write_raw(
        "ctors.sth",
        "import: intrinsics * ;\nimport: core::prelude * ;\n: main ( -- ) True drop False drop ;\n",
    );
    let build = sooth_build(&ok);
    assert!(
        build.status.success(),
        "the constructors re-export: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let named = t.write_raw(
        "named.sth",
        "import: intrinsics * ;\nimport: core::prelude * ;\n: w ( Bool -- ) drop ;\n: main ( -- ) True w ;\n",
    );
    let named_build = sooth_build(&named);
    assert!(
        named_build.status.success(),
        "the type name re-exports into an effect signature too: {}",
        String::from_utf8_lossy(&named_build.stderr)
    );

    // The `.` overload is the one thing that still does not cross the hub:
    // it resolves against its declaring module, not a re-exporting hub, so
    // the type name working above does not mean printing does.
    let print_entry = t.write_raw(
        "print.sth",
        "import: intrinsics * ;\nimport: core::prelude * ;\n: main ( -- ) True . ;\n",
    );
    let print_err = build_error(&print_entry);
    assert!(
        print_err.contains("`.` requires a printable scalar, found `Bool`"),
        "unexpected diagnostic: {print_err}"
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
         import: core::bool b | Bool False True if unless . | ;\n\
         : flip ( Bool -- Bool )\n\
           ~[ False ] ~[ True ] if ;\n\
         : main ( -- )\n\
           True .\n\
           False .\n\
           True flip .\n\
           False ~[ 1 ] ~[ 2 ] unless . ;\n",
    );
    assert_eq!(build_and_run(&entry), "True\nFalse\nFalse\n1\n");
}

// -- G3 (R1): a boolean static requires the import too ----------------------

/// R1: the static's *type annotation* is the gate. Without `core::bool` in
/// scope it is a located unknown type at the annotation, and the `= True`
/// initializer is never reached -- the same rule the body position obeys, not a
/// second initializer-specific check.
#[test]
fn boolean_static_without_importing_core_bool_is_an_unknown_type() {
    let t = Tree::new("g3-missing");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\nstatic: FLAG Bool = True ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("unknown type `Bool` at line 2"),
        "unexpected diagnostic: {err}"
    );
}

/// R1, the other side: with the import the static declares, initializes and
/// reads back as `True`.
#[test]
fn boolean_static_with_the_import_holds_its_initializer() {
    let t = Tree::new("g3-present");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         import: core::bool b | Bool False True if . | ;\n\
         static: FLAG Bool = True ;\n\
         : main ( -- )\n\
           &!FLAG @ .\n\
           &!FLAG @ ~[ 10 ] ~[ 20 ] if . ;\n",
    );
    assert_eq!(build_and_run(&entry), "True\n10\n");
}

/// R1's shape half: `Bool` resolves through the registry now, so a module can
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
         type: Bool | A n i64 | B ;\n\
         static: FLAG Bool = True ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("static `FLAG` has a non-scalar type `Bool`")
            && err.contains("must be exactly two variants, each carrying no payload"),
        "unexpected diagnostic: {err}"
    );
}

/// R1's shape half, the payload-free forgery: a same-named enum with three
/// payload-free variants passes the old "all variants payload-free" test but
/// is still not the logical two-variant `Bool` `resolve_bool_type` resolves,
/// so a `= True` initializer would write a bare 0/1 discriminant into a type
/// whose third variant gives that discriminant space a different meaning.
#[test]
fn static_at_a_three_variant_enum_named_bool_is_an_error() {
    let t = Tree::new("g3-forged-three-variant");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         type: Bool | Maybe | False | True ;\n\
         static: FLAG Bool = True ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("static `FLAG` has a non-scalar type `Bool`")
            && err.contains("must be exactly two variants, each carrying no payload"),
        "unexpected diagnostic: {err}"
    );
}

/// A same-named enum whose variants carry a payload is not the logical `Bool`:
/// `resolve_bool_type` requires both variants payload-free, the shape that makes
/// a Bool a register-resident scalar. `not` on a two-cell tagged aggregate
/// therefore falls through to the bitwise-only path and is rejected, rather than
/// `xor 1`-ing whichever word the discriminant happens to occupy.
#[test]
fn not_on_a_payload_carrying_enum_named_bool_is_an_error() {
    let t = Tree::new("g3-payload");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         type: Bool | A n i64 | B ;\n\
         : main ( -- ) 1 A not drop ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`not` requires an integer or Bool operand, found `Bool`"),
        "unexpected diagnostic: {err}"
    );
}

/// A same-named enum with a third payload-free variant is not the logical `Bool`
/// either: `resolve_bool_type` requires exactly two variants (the count the
/// `xor 1` lowering of `not` assumes), so `not` on it is rejected rather than
/// silently misrouting a three-way discriminant through a two-way eliminator.
#[test]
fn not_on_a_three_variant_enum_named_bool_is_an_error() {
    let t = Tree::new("g3-tristate");
    let entry = t.write_raw(
        "main.sth",
        "import: intrinsics * ;\n\
         type: Bool | A | B | C ;\n\
         : main ( -- ) C not drop ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`not` requires an integer or Bool operand, found `Bool`"),
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
/// `core::bool` at startup from the real `lib/bool.sth`. `True` works on the
/// first line, `.` prints it as `True`/`False`, and `:stack` renders it the same
/// way -- with no import written.
#[test]
fn repl_seeds_core_bool_so_a_bare_true_works_on_the_first_line() {
    let out = run_session(&["True", ":stack", "True .", "False .", "True False"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "stack: True",
            "stack: True",
            "True",
            "stack: True",
            "False",
            "stack: True",
            "stack: True True False",
        ],
        "unexpected session transcript: {out}"
    );
}

/// R2: an imported closure's own `core::bool` folds onto the one the session
/// seeded, rather than being appended as a second, non-equal type. That fold is
/// what lets a session `True` reach an imported `if` at all, and it is the
/// behaviour the shape test below has to keep while refusing a stranger.
#[test]
fn repl_imported_core_bool_folds_onto_the_session_seed() {
    let import = format!(
        "import: \"{}/lib/bool.sth\" b | if | ;",
        env!("CARGO_MANIFEST_DIR")
    );
    let out = run_session(&[&import, "True ~[ 1 ] ~[ 2 ] if ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["imported b", "1", "stack: (empty)"],
        "unexpected session transcript: {out}"
    );
}

/// R2: the fold is keyed on the shape test `resolve_bool_type` applies, never on
/// the name `bool` alone. An imported module's own `bool` carrying a payload is
/// an unrelated type that happens to share a name: folded onto the session's
/// one-cell scalar, its two-cell tagged aggregate is read at the wrong width
/// (`5 mk` rendered as `false`). It keeps an id of its own instead -- its
/// payload survives a round trip through the module's own eliminator, `:stack`
/// renders it as the aggregate it is, and the session's own `true` is untouched.
#[test]
fn repl_imported_enum_named_bool_is_not_the_session_bool() {
    let t = Tree::new("r2-stranger");
    let sibling = t.write_raw(
        "mybool.sth",
        "import: intrinsics | drop | ;\n\
         export: Bool mk un ;\n\
         type: Bool | A n i64 | B ;\n\
         : mk ( i64 -- Bool ) A ;\n\
         : un ( Bool -- i64 ) ~[ ( A ) A> ] ~[ ( B ) drop 0 ] Bool? ;\n",
    );
    let import = format!("import: \"{}\" m | mk un | ;", sibling.display());
    let out = run_session(&[&import, "5 mk un .", "True .", "5 mk"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "imported m",
            "5",
            "stack: (empty)",
            "True",
            "stack: (empty)",
            "stack: <Bool>",
        ],
        "unexpected session transcript: {out}"
    );
}

/// R2/R4 (fix): the fold's shape test alone is not enough to keep a stranger
/// out. An unrelated module can declare its own `type: bool | On | Off ;` --
/// two payload-free variants, the same name, exactly `resolve_bool_type`'s
/// shape -- and nothing forces that to collide with the session's `core::bool`
/// any more (the injection that used to guarantee it is gone). Folding it
/// anyway would alias the session's own `true`/`false` onto the stranger's
/// `On`/`Off` tags: before this fix, `true`/`false` type-checked as valid
/// arguments to the stranger's own `f`, silently routed through the wrong
/// discriminant (`true`, tag 1, hit the `Off` arm), and a residual stranger
/// value rendered on `:stack` as `false`. The fold now also requires the
/// candidate's variant spellings, in order, to match the session's own `bool`
/// (`False`, `True`) -- so the stranger keeps an id of its own: the session's
/// `true`/`false` are rejected as a type mismatch against `f`, and a residual
/// stranger value renders as the unrelated aggregate it is.
#[test]
fn repl_imported_enum_shaped_like_bool_but_named_differently_is_not_the_session_bool() {
    let t = Tree::new("r2-shape-stranger");
    let sibling = t.write_raw(
        "mybool.sth",
        "import: intrinsics | drop | ;\n\
         export: Bool mkon mkoff f ;\n\
         type: Bool | On | Off ;\n\
         : mkon ( -- Bool ) On ;\n\
         : mkoff ( -- Bool ) Off ;\n\
         : f ( Bool -- i64 ) ~[ ( On ) drop 1 ] ~[ ( Off ) drop 0 ] Bool? ;\n",
    );
    let import = format!("import: \"{}\" m | mkon mkoff f | ;", sibling.display());
    let out = run_session(&[
        &import,
        "mkon f .",
        "mkoff f .",
        "True f .",
        "False f .",
        "mkon",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..5],
        ["imported m", "1", "stack: (empty)", "0", "stack: (empty)"],
        "unexpected session transcript: {out}"
    );
    assert!(
        lines[5].contains("type mismatch") && lines[5].contains("::f"),
        "session `True` should be rejected as a type mismatch against the \
         stranger's `f`, not silently folded onto it: {out}"
    );
    assert!(
        lines[6].contains("type mismatch") && lines[6].contains("::f"),
        "session `False` should likewise be rejected: {out}"
    );
    assert_eq!(
        lines.last(),
        Some(&"stack: <Bool>"),
        "a residual stranger value must render as the unrelated aggregate it \
         is, not be misread as the session's own `Bool`: {out}"
    );
}

/// R2: the folded slot is *skipped* when the closure's enums are appended, so
/// every enum after it shifts by one less than the ones before. Shifting them
/// all alike renames each later enum to its neighbour: here the closure's
/// `Color` reference pointed one past `Color`'s appended row, and `top` crashed
/// the checker rather than returning a `Color`.
#[test]
fn repl_import_shifts_an_enum_declared_after_the_folded_bool() {
    let t = Tree::new("r2-after-fold");
    t.write_raw(
        "col.sth",
        "export: Color Red Green mkred ;\n\
         type: Color | Red | Green ;\n\
         : mkred ( -- Color ) Red ;\n",
    );
    let entry = t.write_raw(
        "pair.sth",
        "import: core::bool * ;\n\
         import: self::col c | Color mkred | ;\n\
         export: pick top ;\n\
         : pick ( Bool -- i64 ) ~[ 7 ] ~[ 9 ] if ;\n\
         : top ( -- Color ) mkred ;\n",
    );
    let import = format!("import: \"{}\" p | pick top | ;", entry.display());
    let out = run_session(&[&import, "top", "True p::pick ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["imported p", "stack: <Color>", "7", "stack: <Color>",],
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
        "True",
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
            "stack: True",
        ],
        "unexpected session transcript: {out}"
    );
}
