//! P8.S2 goldens: re-export (R4) and `export:` validation (R5). A hub module
//! promises names it imported rather than declared, and a consumer reaches them
//! through the hub -- qualified, selectively, or by wildcard. Every golden goes
//! through the real `sooth` binary, and every negative one pins the exact
//! diagnostic rather than a bare `is_err()`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

/// A scratch tree of `.sth` files outside any package, so imports resolve by
/// quoted path, not a package-member path -- these goldens are about the
/// quoted-path regime. `sooth_build` below does pass `--manifest` (P7.S7d),
/// but only so the *entry* file can reach `hosted::show`; that manifest
/// plays no part in how the files in this tree import each other. Removed
/// on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p8s2-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, common::fixture_source(rel, contents)).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// P7.S7d: `--manifest`, so an entry that prints can name `hosted::show`.
/// It covers the entry only -- a transitively imported file re-derives its own
/// (anonymous) package, which is exactly the quoted-path regime these goldens
/// are about, so none of the non-entry fixtures here print.
fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

/// Build and run the entry file, returning its stdout. Running (not just
/// building) is what proves the re-exported call reached the *declaring*
/// module's word rather than any same-named other.
fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(output.status.code(), Some(0), "binary should exit clean");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// R4: a hub imports a dependency's word and re-exports it; a consumer reaches
/// it qualified *through the hub*, which declares nothing of its own.
#[test]
fn hub_re_export_resolves_qualified_through_the_hub() {
    let t = Tree::new("hub-qualified");
    t.write("dep.sth", ": lw ( -- i64 ) 7 ;\nexport: lw ;\n");
    t.write("hub.sth", "import: \"dep.sth\" d | lw | ;\nexport: lw ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" h ;\n: main ( -- ) h::lw . ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n");
}

/// R4: the same re-export reached bare, through a selective import of the hub.
/// `check_selective_imports` already accepts it (the name is on the hub's
/// export list); resolution is what needed the origin table.
#[test]
fn hub_re_export_resolves_bare_through_a_selective_import() {
    let t = Tree::new("hub-selective");
    t.write("dep.sth", ": lw ( -- i64 ) 8 ;\nexport: lw ;\n");
    t.write("hub.sth", "import: \"dep.sth\" d | lw | ;\nexport: lw ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" h | lw | ;\n: main ( -- ) lw . ;\n",
    );
    assert_eq!(build_and_run(&entry), "8\n");
}

/// R1 x R4, the headline: a wildcard import of a *re-exporting* hub binds
/// names the hub does not declare. Phase 1's wildcard desugaring alone leaves
/// this `unknown word`; it resolves only once the hub's export list is mapped
/// to origins.
#[test]
fn wildcard_import_of_a_re_export_binds_the_name() {
    let t = Tree::new("wildcard-re-export");
    t.write("dep.sth", ": lw ( -- i64 ) 9 ;\nexport: lw ;\n");
    t.write("hub.sth", "import: \"dep.sth\" d | lw | ;\nexport: lw ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" * ;\n: main ( -- ) lw . ;\n",
    );
    assert_eq!(build_and_run(&entry), "9\n");
}

/// R4: a hub of hubs. Each link re-exports the link below it, and the call
/// resolves to the one module that declares the word, two hops down.
#[test]
fn hub_of_hubs_re_export_resolves_to_the_origin() {
    let t = Tree::new("hub-of-hubs");
    t.write("deep.sth", ": lw ( -- i64 ) 11 ;\nexport: lw ;\n");
    t.write("mid.sth", "import: \"deep.sth\" d | lw | ;\nexport: lw ;\n");
    t.write("top.sth", "import: \"mid.sth\" m | lw | ;\nexport: lw ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"top.sth\" t ;\n: main ( -- ) t::lw . ;\n",
    );
    assert_eq!(build_and_run(&entry), "11\n");
}

/// R4: a hub that imports its dependency *qualified only* reaches the name as
/// `dep::lw` alone -- no selective entry, no local decl -- and may still
/// re-export it. The origin comes from scanning the qualified-imported
/// module's own declarations, so this is not an existence error.
#[test]
fn qualified_only_import_can_be_re_exported() {
    let t = Tree::new("qualified-only");
    t.write("dep.sth", ": lw ( -- i64 ) 12 ;\nexport: lw ;\n");
    t.write("hub.sth", "import: \"dep.sth\" d ;\nexport: lw ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" h ;\n: main ( -- ) h::lw . ;\n",
    );
    assert_eq!(build_and_run(&entry), "12\n");
}

/// R4: the hub's own re-export does not shadow a consumer's same-named local
/// word -- own-module resolution still wins, so the consumer prints its own
/// value, not the hub's.
#[test]
fn a_consumers_own_word_outranks_a_qualified_re_export() {
    let t = Tree::new("own-word-wins");
    t.write("dep.sth", ": lw ( -- i64 ) 1 ;\nexport: lw ;\n");
    t.write("hub.sth", "import: \"dep.sth\" d | lw | ;\nexport: lw ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" h ;\n: lw ( -- i64 ) 2 ;\n: main ( -- ) lw . h::lw . ;\n",
    );
    assert_eq!(build_and_run(&entry), "2\n1\n");
}

/// R4/R15: a re-exported *type* travels as one unit with its generated words,
/// so a consumer builds and destructures it through the hub -- qualified, and
/// bare through a selective import of the hub.
#[test]
fn hub_re_export_of_a_type_resolves_through_both_branches() {
    let t = Tree::new("hub-type");
    t.write("dep.sth", "type: Point x i64 ;\nexport: Point ;\n");
    t.write(
        "hub.sth",
        "import: \"dep.sth\" d | Point | ;\nexport: Point ;\n",
    );
    let qualified = t.write(
        "qualified.sth",
        "import: \"hub.sth\" h ;\n: main ( -- ) 5 h::Point h::Point> . ;\n",
    );
    assert_eq!(build_and_run(&qualified), "5\n");
    let bare = t.write(
        "bare.sth",
        "import: \"hub.sth\" h | Point | ;\n: main ( -- ) 6 Point Point> . ;\n",
    );
    assert_eq!(build_and_run(&bare), "6\n");
}

/// R5: `export:` legitimately names an enum's *variants* beside the enum
/// itself (`lib/result.sth` exports `Result`, `Ok`, and `Err` that way). A
/// variant is not a top-level declaration, so an existence check keyed on the
/// resolver's mangling tables alone would reject the whole shape.
#[test]
fn export_of_an_enum_variant_is_accepted() {
    let t = Tree::new("export-variant");
    t.write("dep.sth", "type: E | A x i64 | B ;\nexport: E A B ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"dep.sth\" d | E A B | ;\n\
         : main ( -- ) 4 A ~[ ( A ) A> ] ~[ ( B ) drop 0 ] E? . ;\n",
    );
    assert_eq!(build_and_run(&entry), "4\n");
}

/// R5/R6c: an `export:` name that is neither declared nor imported in its file
/// built clean before this slice. It is now a located error.
#[test]
fn export_of_an_unknown_name_is_an_error() {
    let t = Tree::new("export-unknown");
    t.write(
        "dep.sth",
        ": lw ( -- i64 ) 1 ;\nexport: lw ;\nexport: nonexistent ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"dep.sth\" d ;\n: main ( -- ) d::lw . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`nonexistent` in `export:` names nothing declared or imported in this module (line 3, col 9)"
        ),
        "unexpected diagnostic: {err}"
    );
}

/// R4/R6c: a bare `export:` name declared by two qualified-imported modules
/// has no spelling that disambiguates it (there is no `export: dep1::lw ;`), so
/// it is a located error naming both origins -- not a silent first-wins pick.
#[test]
fn re_export_ambiguous_between_two_qualified_deps_is_an_error() {
    let t = Tree::new("ambiguous-re-export");
    t.write("one.sth", ": lw ( -- i64 ) 1 ;\nexport: lw ;\n");
    t.write("two.sth", ": lw ( -- i64 ) 2 ;\nexport: lw ;\n");
    t.write(
        "hub.sth",
        "import: \"one.sth\" dep1 ;\nimport: \"two.sth\" dep2 ;\nexport: lw ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" h ;\n: main ( -- ) h::lw . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "`lw` in `export:` is declared by more than one qualified-imported module (`dep1`, `dep2`) and cannot be re-exported without disambiguation (line 3, col 9)"
        ),
        "unexpected diagnostic: {err}"
    );
}

/// R4: a re-export is still export-gated. A hub may only re-export what its
/// dependency exports, and a name the *hub* does not export stays unreachable
/// through it.
#[test]
fn a_name_the_hub_does_not_export_is_unreachable_through_it() {
    let t = Tree::new("hub-withholds");
    t.write(
        "dep.sth",
        ": lw ( -- i64 ) 1 ;\n: other ( -- i64 ) 2 ;\nexport: lw other ;\n",
    );
    t.write(
        "hub.sth",
        "import: \"dep.sth\" d | lw other | ;\nexport: lw ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"hub.sth\" h ;\n: main ( -- ) h::other . ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("unknown word"), "unexpected diagnostic: {err}");
}

// -- R2/R6a: the intrinsics are a module, and `import:` gates them ------------

/// A fixture written *verbatim*, with no harness-appended imports: every golden
/// below is about which `import:` lines are present, so `Tree::write`'s
/// convenience would erase the thing under test.
fn write_raw(t: &Tree, rel: &str, contents: &str) -> PathBuf {
    let path = t.0.join(rel);
    std::fs::write(&path, contents).unwrap();
    path
}

/// R2/R6a: a bare builtin call in a module with no `intrinsics` import is a
/// located error naming the word and the missing import, not `unknown word`
/// and not a builtin dispatch.
#[test]
fn a_bare_intrinsic_without_an_import_is_a_located_error() {
    let t = Tree::new("intrinsic-ungated");
    let entry = write_raw(&t, "main.sth", ": main ( -- ) 1 2 add . ;\n");
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: `add` is an intrinsic and is not imported in `main` (line 1, col 19)\n  add `import: intrinsics * ;` (or `import: intrinsics | add ... | ;`) to this file"
        ),
        "unexpected diagnostic: {err}"
    );
}

/// R2: both import shapes make a builtin visible, and a selective import that
/// omits the name still refuses it -- the positive control that stops the gate
/// from being satisfied by "nothing ever resolves".
#[test]
fn both_intrinsics_import_shapes_admit_a_builtin_and_a_partial_one_does_not() {
    let t = Tree::new("intrinsic-shapes");
    let wildcard = write_raw(
        &t,
        "wild.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n: main ( -- ) 1 2 add . ;\n",
    );
    assert_eq!(build_and_run(&wildcard), "3\n");

    let selective = write_raw(
        &t,
        "sel.sth",
        "import: intrinsics i | add | ;\nimport: hosted::show | . | ;\n: main ( -- ) 1 2 add . ;\n",
    );
    assert_eq!(build_and_run(&selective), "3\n");

    // The same file with `add` dropped from the list: the subset is a real
    // subset, not a spelling of "all".
    let partial = write_raw(
        &t,
        "partial.sth",
        "import: intrinsics i | drop | ;\nimport: hosted::show | . | ;\n: main ( -- ) 1 2 add . ;\n",
    );
    let err = build_error(&partial);
    assert!(
        err.contains("error: `add` is an intrinsic and is not imported in `main`"),
        "unexpected diagnostic: {err}"
    );
}

/// R2: the six surface comparisons are *not* in the gate set. They are `core`
/// words, so an unimported `lt` must be an unknown word -- pointing at
/// `import: core::prelude`, which the R6(a) wording would not -- even though
/// `BUILTIN_WORDS` still lists them for `has_self_tail_call`'s sake.
///
/// The fixture deliberately carries **no** `import: intrinsics * ;`: that line
/// admits every gated name, so the gate could not fire on `lt` whether or not
/// the six are excluded from the gate set, and the test would pass with the
/// exclusion deleted.
#[test]
fn an_unimported_comparison_is_an_unknown_word_not_an_ungated_intrinsic() {
    let t = Tree::new("cmp-not-intrinsic");
    let entry = write_raw(&t, "main.sth", ": main ( -- ) 1 2 lt drop ;\n");
    let err = build_error(&entry);
    assert!(
        err.contains("error: unknown word `lt`") && !err.contains("is an intrinsic"),
        "unexpected diagnostic: {err}"
    );
}

/// R2/R6a, one representative per gate arm in `check_terms`: `branch` (the
/// quotation-operand exemption), `dup` (`check_shuffle`), `tag`
/// (`check_tag_word`), and `len` (`check_str_word`/`check_array_word`) must
/// each refuse to dispatch as a builtin without an `intrinsics` import, the
/// same as the operator family already covered above. `w` is never called
/// from `main`: checking runs over every declared word regardless of call
/// reachability, so this needs no operand of a type `main` can construct
/// without itself reaching a gated intrinsic.
#[test]
fn every_gate_arm_refuses_its_builtin_without_an_import() {
    let cases: &[(&str, &str)] = &[
        ("branch", ": w ( u32 -- i64 ) [ 1 ] [ 2 ] branch ;\n"),
        ("dup", ": w ( i64 -- i64 i64 ) dup ;\n"),
        (
            "tag",
            "type: Flag | Off | On ;\n: w ( Flag -- u32 ) tag ;\n",
        ),
        ("len", ": w ( str -- usize ) len ;\n"),
    ];
    for (name, body) in cases {
        let t = Tree::new(&format!("gate-arm-{name}"));
        let entry = write_raw(&t, "main.sth", &format!("{body}: main ( -- ) ;\n"));
        let err = build_error(&entry);
        assert!(
            err.contains(&format!(
                "error: `{name}` is an intrinsic and is not imported in `w`"
            )),
            "case `{name}`: unexpected diagnostic: {err}"
        );
    }
}

// -- R3/R8: the prelude is gone; `core` is a package you import ---------------

/// R3: `if` and the comparisons no longer arrive without an `import:`. The same
/// file builds once it names `core::prelude`, so this is the import doing the
/// work rather than the program being wrong.
#[test]
fn the_typed_core_arrives_only_by_import() {
    let t = Tree::new("core-import");
    const BODY: &str = ": main ( -- ) 3 4 lt ~[ 1 ] ~[ 0 ] if . ;\n";
    let without = write_raw(
        &t,
        "without.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{BODY}"),
    );
    let err = build_error(&without);
    assert!(
        err.contains("error: unknown word `lt`"),
        "unexpected diagnostic: {err}"
    );

    // With the import, and resolved through the real `lib/` package rather than
    // a stand-in, so this also pins that `core::prelude`'s re-export chain
    // reaches `core::cmp`/`core::bool`.
    let with = write_raw(
        &t,
        "with.sth",
        &format!(
            "import: intrinsics * ;\nimport: hosted::show | . | ;\nimport: core::prelude * ;\n{BODY}"
        ),
    );
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&with)
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "the same file builds with the import; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = with.with_extension("");
    let out = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n");
}

// -- R6b: the narrowing prelude deletion introduced, since lifted -----------
//
// R6b's own golden (`a_poly_word_calling_an_imported_poly_word_names_the_narrowing`)
// pinned the located error a non-inline polymorphic word got for calling an
// imported polymorphic one. P7.S3k grounds that call instead, so the
// diagnostic and its test are gone; the capability it named is pinned by
// `tests/phase7_slice3k.rs`.

/// R2, the poly-body twin: a generic body dispatches the same builtins on its
/// own path (`poly_call_term`), so without a gate there an unimported `dup`
/// would be refused in a monomorphic word and free in a polymorphic one.
#[test]
fn the_intrinsic_gate_also_covers_a_polymorphic_body() {
    let t = Tree::new("intrinsic-poly");
    // `import: intrinsics i | . | ;` names `.` under the `i` qualifier, so a
    // bare `. .` in `main` is unresolvable post-P7.S7d (`.` moved to
    // `hosted::show`, no longer an intrinsic). That's deliberate, not stale:
    // this test's subject is the poly-body intrinsic gate on `dup` inside
    // `twice`, which fires and aborts the build before word resolution ever
    // reaches the unrelated, unresolvable `.` calls in `main`.
    let entry = write_raw(
        &t,
        "main.sth",
        "import: intrinsics i | . | ;\n\
         : twice ['T: Copy] ( 'T -- 'T 'T ) dup ;\n\
         : main ( -- ) 1 twice . . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("error: `dup` is an intrinsic and is not imported in `twice`"),
        "unexpected diagnostic: {err}"
    );
}

/// R2, the positive control the gate rests on: dropping `&& !env.contains_key(name)`
/// from the gate (commit `fe0e09c`) assumes a module's own word under a gated
/// builtin spelling always arrives *mangled*, so a bare gated name reaching the
/// gate never has an env candidate to defer to. Pin the legal shape: a module
/// that declares its own `dup` and calls it builds and runs its own word (prints
/// `7`, which the shuffle `dup` on an empty stack could not), rather than drawing
/// a spurious `` `dup` is an intrinsic and is not imported `` diagnostic.
#[test]
fn an_own_word_under_a_gated_builtin_spelling_still_resolves() {
    let t = Tree::new("own-word-shadows-builtin");
    let entry = write_raw(
        &t,
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n: dup ( -- i64 ) 7 ;\n: main ( -- ) dup . ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n");
}
