//! Diagnostics name the spelling the user wrote, never the resolved symbol.
//!
//! `resolve` mangles every declared name in place (`puts` -> `puts__m0`)
//! before the checker runs, so a formatter that prints a decl's `.name` raw
//! shows a spelling the user cannot type. The checker's own unit tests run
//! pre-resolve, where `.name == .name_static` and the wart is invisible --
//! which is exactly how these slipped through review twice. Every golden here
//! therefore goes through the real `sooth build` binary (resolve included)
//! and pins the exact diagnostic, asserting the surface name appears and the
//! mangled suffix never does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch tree of `.sth` files outside any package. Removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sooth-diag-names-{}-{tag}-{seq}",
            std::process::id()
        ));
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

fn build_error(entry: &Path) -> String {
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr)
        .expect("stderr should be utf8")
        .trim_end()
        .to_string()
}

fn assert_surface(entry: &Path, expected_line: &str) {
    let err = build_error(entry);
    assert!(
        err.contains(expected_line),
        "expected the surface-name diagnostic:\n  {expected_line}\nactual:\n{err}"
    );
    assert!(
        !err.contains("__m0"),
        "diagnostic leaks a mangled name:\n{err}"
    );
}

#[test]
fn extern_boundary_error_names_surface_word_not_mangled_decl() {
    let t = Tree::new("extern-str");
    let entry = t.write(
        "main.sth",
        "extern: puts ( str -- ) \"puts\" ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_surface(
        &entry,
        "error: extern: `puts` declares the input `str` (line 1, col 1)",
    );
}

#[test]
fn struct_stored_reference_error_names_surface_type_not_mangled_decl() {
    let t = Tree::new("struct-stored-ref");
    let entry = t.write("main.sth", "type: Bad tag &i64 ;\n: main ( -- ) 1 drop ;\n");
    assert_surface(
        &entry,
        "error: a reference cannot be stored: field `tag` of type `Bad` has type `&i64` (line 1, col 7)",
    );
}

#[test]
fn enum_stored_reference_error_names_surface_type_not_mangled_decl() {
    let t = Tree::new("enum-stored-ref");
    let entry = t.write(
        "main.sth",
        "type: E | V r &i64 | W ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_surface(
        &entry,
        "error: a reference cannot be stored: payload field `r` of variant `V` of type `E` has type `&i64` (line 1, col 11)",
    );
}

#[test]
fn recursive_struct_cycle_names_surface_type_not_mangled_decl() {
    let t = Tree::new("struct-cycle");
    let entry = t.write(
        "main.sth",
        "type: Node next Node ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_surface(
        &entry,
        "error: recursive struct definition (infinite size): Node -> Node",
    );
}

#[test]
fn duplicate_drop_overload_error_names_surface_type_not_mangled_decl() {
    let t = Tree::new("dup-drop");
    let entry = t.write(
        "main.sth",
        "type: T x i64 ;\n: drop ( T -- ) drop ;\n: drop ( T -- ) drop ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_surface(
        &entry,
        "error: `T` already defines its own `drop` (line 3, col 3)",
    );
}

#[test]
fn extern_quotation_audit_names_surface_word_not_mangled_decl() {
    let t = Tree::new("extern-quot");
    let entry = t.write(
        "main.sth",
        "extern: f ( [ i64 -- i64 ] -- ) \"f\" ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_surface(
        &entry,
        "error: a quotation type `[ i64 -- i64 ]` cannot appear as an `extern:` boundary type of `f`: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
    );
}

#[test]
fn enum_variant_quotation_audit_names_surface_type_not_mangled_decl() {
    let t = Tree::new("enum-quot");
    let entry = t.write(
        "main.sth",
        "type: E | V q [ i64 -- i64 ] | W ;\n: main ( -- ) 1 drop ;\n",
    );
    assert_surface(
        &entry,
        "error: a quotation type `[ i64 -- i64 ]` cannot appear as the field `q` of enum variant `E::V`: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7",
    );
}

#[test]
fn recursive_drop_overload_names_surface_type_and_typable_remedy() {
    // The chain runs through a helper word, so the message cites a drop
    // overload, a plain word, and the remedy `T>` -- every citation must be a
    // spelling the user wrote (the old form rendered `T__m0>` in the remedy).
    let t = Tree::new("drop-cycle");
    let entry = t.write(
        "main.sth",
        "import: intrinsics | drop | ;\ntype: T x i64 ;\n: drop ( T -- ) helper ;\n: helper ( T -- ) drop ;\n: main ( -- ) 1 drop ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: recursive `drop` overload for `T`: `drop ( T -- )` -> `helper` -> `drop ( T -- )` (line 3, col 3)"
        ),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("destructure it with `T>`"),
        "remedy must name the typable spelling: {err}"
    );
    assert!(
        !err.contains("__m0"),
        "diagnostic leaks a mangled name:\n{err}"
    );
}
