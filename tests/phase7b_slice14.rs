//! P7b.S14 goldens: the narrowed provenance gate on the S9 own-header
//! pre-guard's foreign-arg borrow (`bare_generated_word_own_module_grounding`,
//! `src/check/terms.rs`) -- see `docs/roadmap/P7b/slice14-spec.md`. Styled
//! after `tests/phase7b_slice9.rs`'s `Tree` harness.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs14-{}-{tag}-{seq}", std::process::id()));
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

fn sooth_build(entry: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(entry: &PathBuf) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    std::fs::remove_file(&binary).ok();
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn build_error(entry: &PathBuf) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn write_manifest(t: &Tree) {
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs14 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
}

/// `b`'s shape shared by every fixture below: the eager minter, the only
/// module that spells `Widget[i64]` explicitly (via `mk`'s own signature),
/// then destructures it back out so `b`'s own bare `Widget` call never
/// touches the own-header gate at all (`own_idx == gi`, an early return).
fn write_eager_minter(t: &Tree) {
    t.write(
        "b.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : use2 ( -- i64 ) 7 mk | w | &w &v @ | x | w drop x ;\n\
         export: use2 ;\n",
    );
}

/// G-S14.1: the caller imports the minter, so the borrow's declaring module
/// is reachable -- the bare ctor grounds at the caller's own header and runs
/// clean, unchanged from pre-slice behavior.
#[test]
fn own_header_grounds_when_the_foreign_minter_is_reachable() {
    let t = Tree::new("g-s14-1-reachable");
    write_manifest(&t);
    write_eager_minter(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ; import: self::b ;\n\
         type: Widget['T] v 'T ;\n\
         : mk3 ( i64 -- i64 ) Widget | w | &w &v @ | x | w drop x ;\n\
         export: mk3 ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::mk3 . b::use2 . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "5\n7\n",
        "the caller's own header still grounds against a reachable foreign minter's argument list"
    );
}

/// G-S14.2: the caller does not import the minter and names no explicit
/// instantiation of its own anywhere in its own module -- the new located
/// `own_header_cannot_ground_error`, constructor face. This is the
/// new-rejection witness the slice exists to add; pre-slice, this borrow was
/// silently permitted.
#[test]
fn own_header_cannot_ground_when_the_foreign_minter_is_unreachable_constructor() {
    let t = Tree::new("g-s14-2-unreachable-ctor");
    write_manifest(&t);
    write_eager_minter(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : run ( i64 -- i64 ) Widget | w | &w &v @ | x | w drop x ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::run . b::use2 . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: `Widget` in `run` (line 3) cannot ground at this module's own header: the only `Widget` instantiation in scope is declared in a module this module does not import\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's\n"
    );
}

/// G-S14.3: the working escape hatch. `a` does not import `b`, but its own
/// `mkown` names an explicit instantiation of `a`'s own `Widget` header
/// through its own signature -- this puts a second candidate into
/// `env["Widget"]`, so the `[only]`-candidate pre-guard is never entered at
/// all, and the build succeeds without the import.
#[test]
fn own_header_grounds_via_a_local_explicit_instantiation_with_no_import() {
    let t = Tree::new("g-s14-3-escape-hatch");
    write_manifest(&t);
    write_eager_minter(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mkown ( i64 -- Widget[i64] ) Widget ;\n\
         : mk3 ( i64 -- i64 ) Widget | w | &w &v @ | x | w drop x ;\n\
         export: mk3 ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::mk3 . b::use2 . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "5\n7\n",
        "a local explicit instantiation of the caller's own header is a working, import-free escape hatch"
    );
}

/// G-S14.4: G-S14.2's destructure-face twin (`Widget>`, which traverses the
/// identical pre-guard code path via `name.strip_suffix('>')`) -- proves the
/// gate is not constructor-only.
#[test]
fn own_header_cannot_ground_when_the_foreign_minter_is_unreachable_destructure() {
    let t = Tree::new("g-s14-4-unreachable-destructure");
    write_manifest(&t);
    write_eager_minter(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk3 ( i64 -- i64 ) Widget Widget> ;\n\
         export: mk3 ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::mk3 . b::use2 . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: `Widget` in `mk3` (line 3) cannot ground at this module's own header: the only `Widget` instantiation in scope is declared in a module this module does not import\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's\n"
    );
}
