//! P7b.S4 exit goldens: declaring-module identity for generic
//! instantiations. Phase 2 lands the real-type dispatch goldens (S4-4 and
//! S4-5): a user module wildcard-imports `core::option`, declares its own
//! `Functor` trait with an `impl: Functor for Option`, and dispatches member
//! calls against the real lib type -- a mono call with the
//! explicit-instantiation spelling `map[i64 i64]`, and a call through a
//! shared-bound poly word (`twice`). Under the S1-era naming-module mint
//! these shapes were twin-fenced (the operand's instantiation was minted at
//! the *naming* module while the impl target recorded the *declaring* one,
//! so no impl matched); S4-1 keys the mint on the declaring module and the
//! probes measured both shapes flipping to the exact pins below. Later
//! phases extend this file: Phase 3 adds the single-mint identity goldens
//! (P4, T6 -- their `nm` clause is why the harness keeps the binary),
//! Phase 4 the fence goldens (S5 marker, twin-impl ambiguity, variant tag).
//! Harness style from `tests/phase7b_slice3.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs4-{}-{tag}-{seq}", std::process::id()));
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

/// `tests/phase7b_slice2.rs`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs4 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

/// Build, run, and keep the binary: `(binary, stdout)`. The binary is
/// deleted with the `Tree`; Phase 3's identity goldens read its symbols.
fn build_run_keep(tag: &str, src: &str) -> (Tree, PathBuf, String) {
    let (t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the binary should run");
    assert_eq!(
        run.status.code(),
        Some(0),
        "the built binary should exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    (t, binary, stdout)
}

/// Golden #1 (S4-4): a user module that wildcard-imports `core::option`,
/// declares `trait: Functor['F: * -> *]` with member `map` plus
/// `impl: Functor for Option`, dispatches a *mono* member call on an
/// `Option[i64]` operand named in that module -- the explicit-instantiation
/// spelling `map[i64 i64]`. The shape is the committed W2 golden
/// (`tests/phase7b_slice2.rs`'s
/// `functor_map_over_option_dispatches_and_produces_option_of_bool`)
/// re-spelled with the fixture-local twin swapped for the real lib type;
/// under the naming-module mint this exact shape failed mono dispatch with
/// "no `impl:` in this program dispatches on these operands" (the W3/W4
/// note's wart), and S4-1's declaring-module mint flips it to the pin.
#[test]
fn mono_member_call_dispatches_over_the_real_core_option() {
    let src = "\
import: core::option * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Option
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;
;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;
: mkopt ( i64 -- Option[i64] ) Some ;
: main ( -- ) 3 mkopt [ 1 sub ] map[i64 i64] showopt ;
";
    let (_t, _binary, stdout) = build_run_keep("s4-4-mono-real-option", src);
    // `map` applies `[ 1 sub ]` to the `Some` payload: 3 -> 2.
    assert_eq!(stdout, "2\n");
}

/// Golden #2 (S4-5): the S4-4 shape plus the shared-bound poly word
/// `twice['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] )` -- ONE
/// `Functor` bound in a poly body, `map` called twice through it. Under the
/// naming-module mint the bound never discharged: "cannot instantiate `'F`
/// ... does not satisfy `Functor`". Same twin-for-lib-type re-spelling as
/// S4-4; the shared-bound machinery itself is W4's (migrated in
/// `tests/phase7b_slice2.rs`, S4-7), this pins it over the real lib type.
#[test]
fn shared_bound_poly_word_dispatches_over_the_real_core_option() {
    let src = "\
import: core::option * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Option
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;
;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;
: mkopt ( i64 -- Option[i64] ) Some ;
: twice['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] )
  | q |
  q map
  q map ;
: main ( -- ) 3 mkopt [ 1 sub ] twice showopt ;
";
    let (_t, _binary, stdout) = build_run_keep("s4-5-poly-real-option", src);
    // `twice` applies `[ 1 sub ]` twice: 3 -> 2 -> 1.
    assert_eq!(stdout, "1\n");
}
