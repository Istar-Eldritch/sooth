//! P7.S3q goldens: an intrinsic gated into a module with
//! `import: intrinsics | ... | ;`, re-exported through a hub, and called bare
//! by a consumer that has no `import: intrinsics` line of its own.
//!
//! **Every fixture here is written verbatim.** `common::fixture_source` appends
//! `import: intrinsics * ;` to any `.sth` whose text does not already contain
//! `import: intrinsics` -- and *not having that line* is the whole subject of
//! this file, so routing a consumer through it would turn every golden below
//! into a placebo that passes without the feature. The `Tree` helper is
//! `phase8_slice2.rs`'s with that call removed, and this file declares no
//! `mod common;` so the mistake cannot be made by accident.
//!
//! Every golden goes through the real `sooth` binary, and every negative one
//! pins the exact diagnostic rather than a bare `is_err()`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch tree of `.sth` files outside any package, so imports resolve by
/// quoted path and no manifest is involved. Removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3q-{}-{tag}-{seq}", std::process::id()));
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

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

/// Build and run the entry file, returning its stdout. Running, not just
/// building, is what proves the re-exported intrinsic reached real dispatch.
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
    String::from_utf8(build.stderr)
        .expect("stderr should be utf8")
        .trim_end()
        .to_string()
}

/// R1/R4, the headline: the hub gates `drop` in and re-exports it; the consumer
/// has no `import: intrinsics` line at all and still calls it bare.
#[test]
fn hub_re_exporting_a_gated_intrinsic_is_callable_bare() {
    let t = Tree::new("headline");
    t.write("hub.sth", "import: intrinsics | drop | ;\nexport: drop ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"./hub.sth\" hub | drop | ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "");
}

/// R4: a hub of hubs. The middle link effectively admits what it re-exported
/// inward, so it may re-export it outward -- the effective table, not the
/// module's own `import:` lines, is what `export:` consults.
#[test]
fn a_hub_of_hubs_carries_the_intrinsic() {
    let t = Tree::new("hub-of-hubs");
    t.write(
        "inner.sth",
        "import: intrinsics | drop | ;\nexport: drop ;\n",
    );
    t.write(
        "outer.sth",
        "import: \"./inner.sth\" i | drop | ;\nexport: drop ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"./outer.sth\" o | drop | ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "");
}

/// R3: a hub's contribution is enumerated from its `export:` list, never from
/// its own visibility bit, so `IntrinsicVisibility::All` has no path across a
/// hub. The same hub carries `drop` (above) but not `add`.
#[test]
fn a_wildcard_intrinsics_import_does_not_leak_through_a_hub() {
    let t = Tree::new("wildcard-no-leak");
    t.write("hub.sth", "import: intrinsics * ;\nexport: drop ;\n");
    let carries = t.write(
        "carries.sth",
        "import: \"./hub.sth\" hub | drop | ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_eq!(
        build_and_run(&carries),
        "",
        "the exported name does cross, so the negative below is not vacuous"
    );
    let entry = t.write(
        "main.sth",
        "import: \"./hub.sth\" hub | drop | ;\n: main ( -- ) 1 2 add drop ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: error: `add` is an intrinsic and is not imported in `main` (line 2, col 19)\n  add `import: intrinsics * ;` (or `import: intrinsics | add ... | ;`) to this file"
    );
}

/// R4: the accept at `export:` is conditioned on the exporting module actually
/// admitting the name. A hub with no `import: intrinsics` line still fails with
/// the unchanged `export_unknown_name_error`.
#[test]
fn a_hub_without_an_intrinsics_import_cannot_export_one() {
    let t = Tree::new("hub-without-import");
    t.write("hub.sth", "export: drop ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: \"./hub.sth\" hub | drop | ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: error: `drop` in `export:` names nothing declared or imported in this module (line 1, col 9)"
    );
}

/// R2: only the selective/wildcard route carries an intrinsic. A qualified-only
/// import of the hub binds no bare name, and `hub::drop` is not a spelling the
/// builtin dispatch can ever see, so it stays an unknown word.
#[test]
fn a_qualified_hub_import_is_not_a_route() {
    let t = Tree::new("qualified-not-a-route");
    t.write("hub.sth", "import: intrinsics | drop | ;\nexport: drop ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"./hub.sth\" hub ;\n: main ( -- ) 1 hub::drop ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: error: unknown word `hub::drop` in `main` (line 2)"
    );
}

/// R1: the union is per name over a set, so a module that gates `drop` in
/// itself *and* imports a hub admitting it sees one entry, not a duplicate.
#[test]
fn an_own_intrinsics_import_and_a_hub_admitting_the_same_name_agree() {
    let t = Tree::new("own-and-hub");
    t.write("hub.sth", "import: intrinsics | drop | ;\nexport: drop ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics | drop | ;\nimport: \"./hub.sth\" hub | drop | ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "");
}

/// R5: the intrinsic-name entry binds no word, so it does not collide with a
/// local decl. A user destructor and the intrinsic `drop` already coexist in
/// one file (`examples/resources.sth`); a wildcard import of an admitting hub
/// must not make that shape illegal.
#[test]
fn a_local_destructor_coexists_with_a_wildcard_hub_import() {
    let t = Tree::new("local-destructor");
    t.write("hub.sth", "import: intrinsics | drop | ;\nexport: drop ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: \"./hub.sth\" * ;\ntype: Fd n i64 ;\n: drop ( Fd -- ) | h | h Fd> drop ;\n: main ( -- ) 7 Fd drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "");
}

/// R5: two hubs both admitting `drop` union to the same set, so a collision
/// error would report an ambiguity with no two answers.
#[test]
fn two_hubs_admitting_one_intrinsic_are_a_union_not_a_collision() {
    let t = Tree::new("diamond");
    t.write("one.sth", "import: intrinsics | drop | ;\nexport: drop ;\n");
    t.write("two.sth", "import: intrinsics | drop | ;\nexport: drop ;\n");
    let entry = t.write(
        "main.sth",
        "import: \"./one.sth\" a | drop | ;\nimport: \"./two.sth\" b | drop | ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "");
}

/// The R5 exemption's blast radius: an *ordinary* word re-exported by two hubs
/// is still a located collision naming both sources. The exemption is keyed on
/// the name being an intrinsic the source admits, not on re-export as such.
#[test]
fn two_hubs_re_exporting_one_ordinary_word_still_collide() {
    let t = Tree::new("ordinary-collision");
    t.write("dep.sth", ": lw ( -- i64 ) 7 ;\nexport: lw ;\n");
    t.write(
        "one.sth",
        "import: \"./dep.sth\" d | lw | ;\nexport: lw ;\n",
    );
    t.write(
        "two.sth",
        "import: \"./dep.sth\" d | lw | ;\nexport: lw ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: \"./one.sth\" a | lw | ;\nimport: \"./two.sth\" b | lw | ;\n: main ( -- ) lw . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: error: selective import of `lw` from module `b` (line 3, col 25) collides with the selective import of `lw` from module `a`"
    );
}

/// B1: the gate set is not enumerable -- it is `BUILTIN_WORDS` plus every
/// non-empty `>`-prefixed conversion -- so the `export:` accept is a predicate,
/// not a widened name list. `>i64` crosses the hub; the consumer's own
/// `import: intrinsics | . | ;` covers the print and nothing else.
#[test]
fn a_conversion_intrinsic_re_exports_through_a_hub() {
    let t = Tree::new("conversion");
    t.write("hub.sth", "import: intrinsics | >i64 | ;\nexport: >i64 ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics | . | ;\nimport: \"./hub.sth\" h | >i64 | ;\n: main ( -- ) 3.5 >i64 . ;\n",
    );
    assert_eq!(build_and_run(&entry), "3\n");
}
