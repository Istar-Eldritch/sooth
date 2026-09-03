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

use std::path::{Path, PathBuf};
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

/// Build `entry` (a fixture file already written by the caller), run the
/// produced binary, and keep both: `(binary, stdout)`. The multi-module
/// twin of `build_run_keep` (whose per-file `t.write` calls live in the
/// test, per the `tests/phase7b_slice2.rs:644` pattern). The binary is
/// deleted with the `Tree`; the identity goldens read its symbols before
/// the test scope ends.
fn build_run_keep_entry(entry: &Path) -> (PathBuf, String) {
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
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
    (binary, stdout)
}

/// The probe fixtures' `sooth.pkg`: hosted layer with the core/hosted path
/// depends (the same shape `single_file_hosted` writes, spelled out for
/// multi-module fixtures).
fn write_hosted_pkg(t: &Tree) {
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs4 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
}

/// `binary`'s symbol names: `nm`'s last whitespace-separated field per
/// line (`tests/phase7b_slice3.rs`'s helper, verbatim but for the sanity
/// target). The sanity clause keeps the zero-`sooth_mono_` assertion from
/// passing vacuously on an nm failure.
fn symbols(binary: &Path) -> Vec<String> {
    let nm = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm should run");
    let text = String::from_utf8_lossy(&nm.stdout).into_owned();
    let names: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect();
    assert!(
        names.iter().any(|s| s == "sooth_main"),
        "sanity: nm reads this binary's symbols at all:\n{text}"
    );
    names
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

/// Golden #5 (S4-8): two user modules each naming `Option[i64]` -- mod_a
/// and mod_b with identical `mk`/`un`/`run`, one per-file `t.write` each
/// per the `tests/phase7b_slice2.rs:644` pattern -- build and run on ONE
/// truthful shared mint. Pre-change (the naming-module mint) this exact
/// shape failed to build with `type mismatch in `mk``: same-rendering,
/// distinct-mint `Type` handles (F3) -- the module naming the instantiation
/// held a different handle than the declaring-keyed producers minted, and
/// `Type` equality is handle identity. S4-1 keys the naming producers on
/// the declaring module, so both modules' spellings meet one mint. The nm
/// clause is m5(b)'s evidence, in-repo for good: the binary carries zero
/// `sooth_mono_*` symbols.
#[test]
fn two_modules_naming_option_i64_share_one_mint_and_mint_zero_monomorphs() {
    let t = Tree::new("s4-8-two-modules-one-mint");
    write_hosted_pkg(&t);
    let module = "\
import: intrinsics * ;\n\
import: core::option * ;\n\
: mk ( i64 -- Option[i64] ) Some ;\n\
: un ( Option[i64] -- i64 ) ~[ ( Some ) Some> ] ~[ ( None ) drop 0 ] Option? ;\n\
: run ( i64 -- i64 ) mk un ;\n\
export: run ;\n\
";
    t.write("mod_a.sth", module);
    t.write("mod_b.sth", module);
    let entry = t.write(
        "main.sth",
        "\
import: intrinsics * ;\n\
import: self::mod_a ;\n\
import: self::mod_b ;\n\
: main ( -- ) 42 mod_a::run drop 43 mod_b::run drop ;\n\
",
    );
    let (binary, _stdout) = build_run_keep_entry(&entry);
    let monos: Vec<String> = symbols(&binary)
        .into_iter()
        .filter(|s| s.starts_with("sooth_mono_"))
        .collect();
    assert!(
        monos.is_empty(),
        "one shared mint builds with zero `sooth_mono_` symbols; nm found: {monos:#?}"
    );
}

/// Golden #6 (S4-10): a recursive generic header declared in one module
/// (`type: L['T] | Nil | Cons 'T rest ^L['T] ;` -- the `^` indirection
/// spelling is mandatory and kept exactly: the direct self-field spelling
/// is the pre-existing infinite-size rejection,
/// `src/check/declarations.rs:1785`) and exported, then named from another
/// module (`main`'s `mk` spells `L[i64]` across the boundary). Pre-change
/// the build failed with two identical `Cons` overloads -- `candidate:
/// `i64` `^L[i64]`` twice, the outer `L[i64]` naming mint via
/// `resolve_type_or_apply` plus the inner `^L['T]` self-reference minted by
/// `substitute_generic_field` (already declaring-keyed); S4-1 keys the
/// outer mint on the declaring module and both spellings meet one mint
/// (m5-t6-pre/post). Pin: builds, runs, exits 0.
#[test]
fn recursive_generic_header_named_across_modules_builds_and_runs() {
    let t = Tree::new("s4-10-t6-recursive");
    write_hosted_pkg(&t);
    t.write(
        "rec.sth",
        "type: L['T] | Nil | Cons 'T rest ^L['T] ;\nexport: L ;\n",
    );
    let entry = t.write(
        "main.sth",
        "\
import: intrinsics * ;\n\
import: self::rec * ;\n\
: mk ( i64 -- L[i64] ) Nil ^ Cons ;\n\
: main ( -- ) 5 mk drop ;\n\
",
    );
    let (_binary, _stdout) = build_run_keep_entry(&entry);
}
