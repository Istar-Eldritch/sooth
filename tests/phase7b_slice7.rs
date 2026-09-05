//! P7b.S7 exit goldens: quotation effects over type constructors (the `call`
//! extension). Phase 1 lifts fence #1 (`member_shape_is_supported`'s
//! Quotation arm) so a trait-var-headed `App` inside a member quotation row
//! parses to a `TraitDecl` with the declared effect intact -- narrowly: an
//! App headed by anything but the trait's own type variable stays rejected.
//! Harness style from `tests/phase7b_slice2.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs7-{}-{tag}-{seq}", std::process::id()));
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

fn build_ok(entry: &Path) {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    std::fs::remove_file(&binary).ok();
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// G1: `monad_bind_declares_app_in_quotation_row` -- `bind`'s `'F['T]` input
/// and the row-nested `'F['U]` output inside its quotation parameter both
/// apply the trait's own `'F`; pre-REQ-1 the row-nested one hit
/// `app_in_member_quotation_row_error` (exit 1), post-REQ-1 the whole
/// declaration parses to a `TraitDecl` with the effect intact.
#[test]
fn monad_bind_declares_app_in_quotation_row() {
    let src = "\
trait: Monad['F: * -> *] :
  bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("g1-monad-bind", src);
    build_ok(&entry);
}

/// G4a (regression pin, reclassified per the paper tests): a bare `'F` used
/// as a plain type in a quotation row's output, after an earlier App-head
/// mention establishes its kind as `* -> *`, is a kind error
/// (`arrow_var_used_bare_error`) that fires independently of, and earlier
/// than, the row-shape gate -- a placebo for the row-fence's narrowness (it
/// would die identically under a hypothetical blanket admit), kept only as
/// a regression pin that the independent kind check still fires.
#[test]
fn kind_incorrect_app_in_quotation_row_is_error() {
    let src = "\
trait: Bad['F: * -> *] :
  m ( 'F['T] [ 'T -- 'F ] -- 'F['T] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("g4a-bare-kind-error", src);
    let err = build_error(&entry);
    assert!(
        err.contains("is used as a plain type but has kind `* -> *`"),
        "{err}"
    );
    // Located at the bare `'F` inside the row (line 3, col 22), not just
    // named -- pins REQ-4's "located, not silent".
    assert!(err.contains("line 3, col 22"), "{err}");
}

/// G4b (the real row-fence-narrowness witness): `'G` is a member local, not
/// the trait's own header variable, heading an application inside the
/// quotation's output row. Still rejected post-REQ-1, via the same
/// `app_in_member_quotation_row_error` dispatch -- `member_shape_is_supported`'s
/// own `App` arm (`head == 0` only) still rejects `'G`'s application when the
/// Quotation arm's recursion reaches it, so the row admits only an App headed
/// by the trait's own variable. The message is the REQ-3-corrected one, not
/// the pre-S7 "keep quotation rows App-free" text (no longer true in general).
#[test]
fn member_local_headed_app_in_quotation_row_is_still_unsupported() {
    let src = "\
trait: Bad2['F: * -> *] :
  m ( 'F['T] [ 'T -- 'G['U] ] -- 'F['T] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("g4b-local-headed-row", src);
    let err = build_error(&entry);
    assert!(
        err.contains("applies a type variable inside a quotation row"),
        "{err}"
    );
    assert!(
        err.contains(
            "an App inside a quotation row must be headed by the trait's own type variable"
        ),
        "{err}"
    );
    // Located at the member (`m`, line 3, col 3), not just named -- pins
    // REQ-4's "located, not silent" (the retired slice2 test's `line 3, col
    // 3` pattern).
    assert!(err.contains("line 3, col 3"), "{err}");
}
