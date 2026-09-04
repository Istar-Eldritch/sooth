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
//! (P4, T6 -- only P4 carries the `nm` clause),
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
/// deleted with the `Tree`; the nm clause reads the entry twin's binary
/// (`build_run_keep_entry`), not this helper's.
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
/// line (`tests/phase7b_slice3.rs`'s helper, verbatim including the sanity
/// clause). The sanity clause keeps the zero-`sooth_mono_` assertion from
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

// ---- Phase 4: the fence goldens (S4-11, S4-12, S4-13) ----

/// Run a build expected to fail and return its stderr, mirroring
/// `tests/phase7b_slice3.rs`'s `build_error`.
fn build_error(entry: &Path) -> String {
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// The m4 family's u1: its own `Functor['F: * -> *]` with member `unbox`,
/// its own `impl: Functor for Option`, the shared-bound poly route
/// `go['F: Functor 'T]`, and `run` spelled through `go`. The /tmp probe
/// fixture's `run` carries a stale qualified `Functor::unbox` probe
/// spelling -- whose `unknown word` rejection is the F8 remedy-hole record
/// (ledger item 4, never spelled in a committed golden) -- so both S4-12
/// goldens spell the mutations.md §m4 shape: `run` → poly `go` → `unbox`.
const U1_STH: &str = "\
import: intrinsics * ;
import: core::option * ;
trait: Functor['F: * -> *] :
  unbox ( 'F['T] [ 'T -- i64 ] -- i64 ) ;
;
impl: Functor for Option
  : unbox swap ~[ ( Some ) Some> swap call ] ~[ ( None ) drop drop 0 ] Option? ;
;
: mk ( i64 -- Option[i64] ) Some ;
: run ( i64 -- i64 ) mk [ 3 sub ] go ;
: go['F: Functor 'T] ( 'F['T] [ 'T -- i64 ] -- i64 ) unbox ;
export: run ;
export: go ;
";

/// The m4 family's u2: the same-named trait, member, and route, but an
/// observably different impl body (the payload keeps 100).
const U2_STH: &str = "\
import: intrinsics * ;
import: core::option * ;
trait: Functor['F: * -> *] :
  unbox ( 'F['T] [ 'T -- i64 ] -- i64 ) ;
;
impl: Functor for Option
  : unbox swap ~[ ( Some ) Some> swap call 100 add ] ~[ ( None ) drop drop 0 ] Option? ;
;
: mk ( i64 -- Option[i64] ) Some ;
: run ( i64 -- i64 ) mk [ 3 sub ] go ;
: go['F: Functor 'T] ( 'F['T] [ 'T -- i64 ] -- i64 ) unbox ;
export: run ;
export: go ;
";

/// Golden #7 (S4-12a, the per-trait control): a module holding only its own
/// `Functor` trait with its own `impl: Functor for Option` builds alone and
/// prints `39` through the poly route -- `run` → `go` → `unbox` dispatching
/// on its own impl (42 mapped through `[ 3 sub ]`). This is the m4 record's
/// "per-trait clean" half: two same-named traits in one program are
/// distinct `TraitId`s, so a module seeing only its own trait dispatches
/// exactly as if the twin never existed.
#[test]
fn module_with_only_its_own_trait_builds_and_prints_through_the_poly_route() {
    let t = Tree::new("s4-12a-per-trait-control");
    write_hosted_pkg(&t);
    t.write("u1.sth", U1_STH);
    let entry = t.write(
        "main.sth",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: self::u1 ;
: main ( -- ) 42 u1::run . ;
",
    );
    let (_binary, stdout) = build_run_keep_entry(&entry);
    assert_eq!(stdout, "39\n");
}

/// Golden #8 (S4-12b): with u1 and u2 both present -- two same-named
/// `Functor` traits, each carrying an `impl: Functor for Option` over the
/// same widened identity -- a mono caller in `main` that names
/// `Option[i64]` itself (in `mk`'s declared signature) and calls bare
/// `unbox` gets the located `mono_ambiguous_member_error`, not a silent
/// first-win. This is the golden's pinned delta: pre-change the operand's
/// naming-module mint matched neither impl and the shape failed with
/// `mono_member_no_dispatch_error`; post-change both impls dispatch on the
/// one shared mint and the *ambiguity* error fires (poly.rs:2450, fired at
/// :2250) -- a strict improvement in diagnostic precision, still located.
/// The bytes below are what the binary prints at this tree (measured, and
/// matching the spec's m2b-era inference -- no finding).
#[test]
fn twin_impls_make_a_mono_member_call_a_located_ambiguity_error() {
    let t = Tree::new("s4-12b-twin-impl-ambiguity");
    write_hosted_pkg(&t);
    t.write("u1.sth", U1_STH);
    t.write("u2.sth", U2_STH);
    let entry = t.write(
        "main.sth",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::option * ;
import: self::u1 ;
import: self::u2 ;
: mk ( i64 -- Option[i64] ) Some ;
: main ( -- ) 42 mk [ 3 sub ] unbox . ;
",
    );
    let err = build_error(&entry);
    assert_eq!(
        err,
        "error: `unbox` in `main` (line 7, col 31) is a trait member of both `Functor` and `Functor`\n  an `impl:` of each claiming trait dispatches on this call's operand; qualify the call with the claiming trait's module (`module::unbox`) to name the one you mean\n"
    );
}

/// Golden #9 (S4-11), post-merge with P7b.S5 and P7b.S9. Was the
/// S5-boundary marker: two user modules each declare their own `Widget['T]`
/// plus ctor (identical i64 payloads), both naming them through f's shared
/// `Functor` -- pinned (at S4 time) as a byte-identical `mk` type mismatch,
/// because the generated-ctor env dispatch was a module-blind name+shape
/// first-match. P7b.S5 Phase 2b's tier policy resolves exactly that
/// ambiguity (each module's own mint wins in its own module), so `mk`
/// type-checks in both -- the marker's own boundary moved, as designed, and
/// each caller now dispatches its own `impl:`.
///
/// Same shape as P7b.S9's G2
/// (`cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl`,
/// `tests/phase7b_slice9.rs`), a different trait and text: each keeps its own
/// fixture.
#[test]
fn cross_module_same_shaped_impls_each_dispatch_their_own_impl() {
    let t = Tree::new("s4-11-same-named-ctors");
    write_hosted_pkg(&t);
    t.write(
        "f.sth",
        "\
import: intrinsics * ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
: sized['F: Functor] ( 'F -- i64 ) size ;
export: Functor sized ;
",
    );
    t.write(
        "a.sth",
        "\
import: intrinsics * ;
import: self::f * ;
type: Widget['T] v 'T ;
impl: Functor for Widget
  : size drop 1 ;
;
: mk ( i64 -- Widget[i64] ) Widget ;
: run ( i64 -- i64 ) mk sized ;
export: run ;
",
    );
    t.write(
        "b.sth",
        "\
import: intrinsics * ;
import: self::f * ;
type: Widget['T] v 'T ;
impl: Functor for Widget
  : size drop 2 ;
;
: mk ( i64 -- Widget[i64] ) Widget ;
: run ( i64 -- i64 ) mk sized ;
export: run ;
",
    );
    let entry = t.write(
        "main.sth",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: self::f * ;
import: self::a ;
import: self::b ;
: main ( -- ) 5 a::run . 6 b::run . ;
",
    );
    // Post-merge correction (P7b.S5 then P7b.S9 landed on `main` after this
    // fence was written): P7b.S5 Phase 2b's tier policy resolves the `mk`
    // ctor-mint ambiguity this fixture used to hard-error on (each module's
    // own `Widget[i64]` mint is distinguished, so `mk` type-checks in both
    // `a` and `b`), so this is no longer the S5 boundary marker it was
    // pinned as. The cross-pick that replaced it was never the trait-impl
    // matcher's: `match_impl_target_rec` compares header identity
    // `(idx, module)` and resolves per-module correctly. It was two other
    // mechanisms, both off the matcher -- operand provenance (a bare ctor
    // call borrowing another module's eager mint) and monomorphization
    // identity (`instantiation_symbol` rendering a struct operand by name,
    // so the two groundings of `sized` minted one symbol and lowering's
    // dedup discarded one of them). Both are fixed in P7b.S9, and each
    // caller now dispatches its own impl deterministically.
    let (_binary, out) = build_run_keep_entry(&entry);
    assert_eq!(out, "1\n2\n");
}

/// Golden #11 (S4-13): a leading variant slot in a quotation annotation is
/// visible from a non-declaring module. `showopt` spells the p1 tag model
/// under a wildcard `core::option` import, and the leading `( Some )`
/// parses as a `VariantTag` -- visibility routes through the
/// wildcard-desugared selective map (`driver.rs`'s import desugar) to
/// `module_declares_variant` -- rather than falling through to the ordinary
/// `unknown type` annotation error. Positive twin of the negative pins
/// `parse_leading_variant_slot_other_module_variant_is_not_visible` and its
/// generic twin (parser.rs:9036/:9088). The spec spells the tag
/// `[ ( Some ) ... ]` as shorthand for the p1 model's arm, which is the
/// inline `~[ ... ]` flavour: a plain-`[` arm is the pre-existing R-C2
/// flavour-gate rejection, an orthogonal fence. No code change was
/// expected; the pin is deliberate because no committed positive twin
/// existed (golden #1 exercises the path only incidentally, inside `map`'s
/// impl body).
#[test]
fn leading_variant_slot_tag_is_visible_from_an_importing_module() {
    let src = "\
import: core::option * ;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;
: main ( -- ) 41 Some showopt ;
";
    let (_t, _binary, stdout) = build_run_keep("s4-13-variant-tag", src);
    assert_eq!(stdout, "41\n");
}
