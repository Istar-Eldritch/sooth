//! P7b.S2 exit goldens: constructor-keyed dispatch and higher-kinded trait
//! declarations. Phase 1 (trait surface) starts the file: the header kind is
//! published and seeded into each member (S2-1), the member dispatchability
//! rule is HKT-aware (S2-2), and the member shape gate gains the App and
//! App-free-quotation arms (S2-3). Later phases add to it. Driven through the
//! real `sooth` binary, styled after `tests/phase7b_slice1.rs`; error goldens
//! keep the minimal two-line prefix so their line/column assertions stay
//! readable against the fixture.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs2-{}-{tag}-{seq}", std::process::id()));
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

/// Golden (positive #1, W1): an HKT trait declaration -- a `* -> *` header
/// variable, a member whose dispatchable input is trait-var-headed
/// (`'F['T]`), a declared quotation parameter with App-free rows
/// (`[ 'T -- 'U ]`), and an App-headed output (`'F['U]`) -- typechecks.
/// Today (pre-S2) this died at `multi_variable_trait_error` (p6a); the
/// member single-var gate is lifted (S2-1), the shapes are supported
/// (S2-3), and the member dispatches on its App-headed input (S2-2).
#[test]
fn hkt_trait_declaration_with_app_and_quotation_member_typechecks() {
    let src = "\
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("w1-hkt-trait-decl", src);
    build_ok(&entry);
}

/// Golden (error #1, S2-15.a): a member of an HKT trait whose only inputs
/// are member locals has nothing for a call to dispatch on -- the lifted
/// single-var gate (S2-1) hands the shape to the HKT-aware dispatchability
/// rule (S2-2), which rejects it as a located declaration-time error naming
/// the member and the expected trait-var-headed form. The asserted text is
/// the spec's pinned S2-15.a line (slice2-spec.md, S2-2) with this fixture's
/// names/spans substituted; the nested-composite note is NOT part of that
/// pinned text and this fixture has no composite input, so it must be
/// absent (it appears only when the inputs actually nest the trait var --
/// pinned at unit level in `check/declarations.rs`).
#[test]
fn hkt_member_without_dispatchable_input_is_located_error() {
    let src = "\
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
  : pick ( 'T -- 'F['T] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("s2-15a-no-dispatchable-input", src);
    let err = build_error(&entry);
    // The spec's pinned S2-15.a text, verbatim for this fixture.
    assert!(
        err.contains(
            "error: trait member `pick` of `Functor` (line 4, col 5) has no input for a call to \
             dispatch on (expected the trait's variable `'F` bare or heading an application like \
             `'F['T]`)"
        ),
        "{err}"
    );
    // The nested-composite note is conditional and this fixture's inputs
    // mention no composite shape -- it must not ride along.
    assert!(!err.contains("note:"), "{err}");
    // Distinguishing fragment: the lifted member gate must not fire --
    // `pick` legitimately declares a local; what fails is dispatchability.
    assert!(!err.contains("more than one type variable"), "{err}");
}

/// Golden (error #2, S2-15.b): the header's kind annotation conflicts with a
/// member's usage. p6c's accepted fixture (`'F: * -> *` header with a *bare*
/// `'F` member) was inert pre-S2 because the parsed kind was discarded; S2-1
/// seeds each member's var 0 with the header kind, so the bare mention is a
/// located error carrying both spans -- the member usage and the header
/// annotation.
#[test]
fn trait_header_kind_conflicting_with_member_usage_is_error() {
    let src = "\
trait: Functor['F: * -> *] :
  size ( 'F -- i64 ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("s2-15b-kind-conflict", src);
    let err = build_error(&entry);
    assert!(
        err.contains("is used as a plain type but has kind `* -> *`"),
        "{err}"
    );
    // Both spans: the member usage (line 3), then the header annotation
    // (line 2) as the origin.
    assert!(err.contains("line 3, col 10"), "{err}");
    assert!(err.contains("line 2, col 16"), "{err}");
}

/// Golden (error #4, S2-15.d, F10): a type application inside a member
/// quotation row is a located fence of its own -- the declaration grammar
/// *represents* the shape, but body-level `call` cannot see through it, so
/// the member gate rejects it instead of leaving it to fail at a (later
/// slice's) consumer. A plain-slot App (`'F['T]` as the first input here)
/// stays legal, pinning that the fence is row-scoped, not signature-scoped.
#[test]
fn app_inside_member_quotation_row_is_fenced() {
    let src = "\
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'F['T] -- 'U ] -- 'F['U] ) ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("s2-15d-app-in-row", src);
    let err = build_error(&entry);
    assert!(
        err.contains("applies a type variable inside a quotation row"),
        "{err}"
    );
    // Located at the member (`map`, line 3), with the row-scoped advice.
    // (The parser-voice errors say "at line L, col C"; the check-side
    // S2-15.a report parenthesizes -- each keeps its stage's house style.)
    assert!(err.contains("line 3, col 3"), "{err}");
    assert!(err.contains("keep quotation rows App-free"), "{err}");
}
