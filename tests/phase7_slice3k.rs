//! P7.S3k goldens: a non-inline generic word calling *another* generic word.
//!
//! Before this slice every such call -- same-module or imported, user-declared
//! or a library word -- was a located rejection
//! (`poly_calls_poly_word_error`). It grounds now: the callee's declared
//! signature is fetched from the same `poly_env` a monomorphic caller
//! dispatches through, and its rigid type variables are related to the
//! caller's symbolically, since at check time the caller has no `θ` either.
//!
//! A callee lowering **splices** (an `inline` library word like `lt`/`gt`)
//! needs no monomorph of its own. A non-inline one needs one composed
//! instantiation per concrete type the caller reaches, discovered by the
//! checker's transitive fixpoint and routed per caller instantiation.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

/// A scratch tree of `.sth` files outside any package, removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3k-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(
            &path,
            format!("{contents}{}", common::printing_import(contents)),
        )
        .unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sooth_build(entry: &Path) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sooth"));
    cmd.arg("build").arg(entry);
    // A single-file fixture in a temp directory needs the shared manifest to
    // name `core` at all; a multi-file tree wrote its own `sooth.pkg`, and
    // `--manifest` would override it out of `self::` reach.
    if let Some(manifest) = common::manifest_for(entry) {
        cmd.arg("--manifest").arg(manifest);
    }
    cmd.output().expect("sooth build should spawn")
}

fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let out = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(out.status.code(), Some(0), "binary should exit clean");
    String::from_utf8(out.stdout).expect("stdout should be utf8")
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        !build.status.success(),
        "build should have failed; stdout: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    String::from_utf8_lossy(&build.stderr).into_owned()
}

/// Build, then read the monomorph symbols out of the linked binary with `nm`
/// (the pattern `tests/phase7_slice3a.rs` and `tests/symbol_hijack.rs` use).
/// Which instantiation ran is unobservable at runtime -- a generic body cannot
/// print its own `'T` -- so callee identity has to be asserted in the emitted
/// artefact.
fn monomorph_symbols(entry: &Path) -> Vec<String> {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let nm = Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_file(&binary).ok();
    let mut symbols: Vec<String> = String::from_utf8_lossy(&nm.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .filter(|s| s.starts_with("sooth_mono_"))
        // P7.S7d: every printing fixture links `hosted::show`, whose per-type
        // dots monomorphize `core::show`'s `render` and `flush` once each.
        // Those are the library's, not the subject's.
        .filter(|s| !s.starts_with("sooth_mono_render__") && !s.starts_with("sooth_mono_flush__"))
        .map(str::to_string)
        .collect();
    symbols.sort();
    symbols
}

/// The generic body under test in both goldens below: `mylt` compares its own
/// `'T` through `core::cmp`'s imported, generic `lt`. That is exactly the
/// program `tests/phase8_slice2.rs`'s retired
/// `a_poly_word_calling_an_imported_poly_word_names_the_narrowing` pinned as a
/// located error.
const MYLT: &str = "import: intrinsics * ;\n\
     import: core::prelude * ;\n";

/// R1/R3: the capability. An imported generic callee is reached from a
/// non-inline generic body, at two distinct instantiations, and the program
/// runs.
///
/// Run rather than merely built, and at both `i64` and `f64`: `lt` is
/// `Copy Ord`-generic over the whole numeric tower and lowers to a
/// type-directed `ult` intrinsic, so a call that reached the wrong
/// instantiation would compare the wrong way (or the wrong width) rather than
/// fail to link. `2.0 1.0 lt` is deliberately false while `1 2 lt` is true, so
/// one instantiation's answer cannot stand in for the other's.
#[test]
fn a_generic_body_compares_its_own_variable_through_an_imported_generic_word() {
    let t = Tree::new("mylt");
    let entry = t.write(
        "main.sth",
        &format!(
            "{MYLT}: mylt ['T: Copy Ord] ( 'T 'T -- Bool ) lt ;\n\
             : main ( -- )\n\
               1 2 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               5 4 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               1.0 2.0 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               2.0 1.0 mylt ~[ 1 . ] ~[ 0 . ] if\n\
               ;\n"
        ),
    );
    assert_eq!(build_and_run(&entry), "1\n0\n1\n0\n");
}

/// R3: the callee's bounds are discharged against the *caller's* declared
/// ones, at the call site. `lt` needs `Copy Ord`; a caller declaring only
/// `Copy` is rejected where the call is written, naming both variables and the
/// missing bound -- not at whatever type a later caller instantiates `mylt`
/// with, and never as a monomorphization-time failure (N1).
#[test]
fn a_bound_the_caller_does_not_declare_is_a_located_call_site_error() {
    let t = Tree::new("mylt-unbounded");
    let entry = t.write(
        "main.sth",
        &format!(
            "{MYLT}: mylt ['T: Copy] ( 'T 'T -- Bool ) lt ;\n\
             : main ( -- ) 1 2 mylt drop ;\n"
        ),
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`'T` of `lt` requires `Ord`, which `'T` in `mylt` does not declare"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        err.contains("declare `'T: Ord`"),
        "the diagnostic should name the remedy: {err}"
    );
    assert!(
        !err.contains("__m"),
        "the diagnostic must not leak a mangled spelling: {err}"
    );
}

/// R6: a cross-call whose operand wraps one of the caller's own variables is a
/// located rejection at the call site. `Box['T]` grows by one constructor per
/// hop, so a recursive cross-call of this shape has no finite set of
/// instantiations and no dedup would ever fire.
///
/// The wrapper is a generic **enum** on purpose: array construction inside a
/// polymorphic body is refused by a pre-existing guard, so an array-based
/// witness would never reach the growth rule at all. Sooth has no generic
/// structs.
#[test]
fn a_cross_call_growing_the_type_is_a_located_rejection() {
    let t = Tree::new("growing");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         type: Box['T] | Box 'T ;\n\
         : h ( 'U -- 'U ) ;\n\
         : g ( 'T -- ) Box h drop ;\n\
         : main ( -- ) 1 g ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("cannot pass `Box['T]` to `'U` of the polymorphic word `h`")
            && err.contains("builds a larger type at every hop"),
        "unexpected diagnostic: {err}"
    );
}

// -- phase 2: the non-inline callee, monomorphized ---------------------------

/// R4: the slice's headline shape end to end. `g` is generic and non-inline,
/// so it is monomorphized once per θ it is called at; each of those monomorphs
/// calls `id`, which is generic and non-inline too and so needs a monomorph of
/// its own -- one that no *concrete* call site in the program names. Two
/// asymmetric instantiations, since one alone cannot tell "a monomorph per θ"
/// from "one monomorph, reached twice".
#[test]
fn a_non_inline_generic_callee_is_monomorphized_once_per_reached_instantiation() {
    let t = Tree::new("id-g");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : g ( 'T -- 'T ) id ;\n\
         : main ( -- ) 7 g . \"x\" g drop ;\n",
    );
    assert_eq!(
        monomorph_symbols(&entry),
        [
            "sooth_mono_g__m0__t0_i64",
            "sooth_mono_g__m0__t0_str",
            "sooth_mono_id__m0__t0_i64",
            "sooth_mono_id__m0__t0_str",
        ]
    );
}

/// R4: the cross-module half of the same thing, which phase 1 could only pin
/// through the mangler at unit level. The callee's symbol carries its own
/// declaring module (`__m1`), so the composed instantiation is minted against
/// the imported word rather than against a same-module lookalike.
#[test]
fn an_imported_non_inline_generic_callee_is_monomorphized() {
    let t = Tree::new("imported-callee");
    t.write("sooth.pkg", &common::fixture_package("p7s3k"));
    t.write(
        "boxed.sth",
        "import: intrinsics * ;\n\
         export: myid ;\n\
         : myid ( 'T -- 'T ) ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: self::boxed b | myid | ;\n\
         : g ( 'T -- 'T ) myid ;\n\
         : main ( -- ) 7 g . ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n");
    assert_eq!(
        monomorph_symbols(&entry),
        ["sooth_mono_g__m0__t0_i64", "sooth_mono_myid__m1__t0_i64"]
    );
}

/// R5/N3: a mutual generic pair, each hop a pure variable renaming. The
/// fixpoint revisits `(g, i64)` on the second hop, mints the symbol it already
/// claimed, and stops -- so compilation *terminates* with no depth cap, and one
/// monomorph exists per word rather than one per hop. The `0 drop` after each
/// call keeps the cycle out of tail position, where `check_tail_call_cycles`
/// rejects it before this machinery is reached at all.
///
/// Run, not merely built: the runtime cycle is bounded by the `i64` counter, so
/// a wrongly-composed callee would recurse on the un-decremented value and
/// never return rather than print.
#[test]
fn a_mutual_non_growing_generic_pair_compiles_runs_and_terminates() {
    let t = Tree::new("mutual");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         : h ['U: Copy] ( 'U i64 -- 'U ) dup 0 gt ~[ 1 sub g 0 drop ] ~[ drop ] if ;\n\
         : g ['T: Copy] ( 'T i64 -- 'T ) dup 0 gt ~[ 1 sub h 0 drop ] ~[ drop ] if ;\n\
         : main ( -- ) 7 5 g . ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n");
    // P7.S3s (R5): `gt` is no longer `inline`, so `h`/`g`'s cross-call to it
    // (itself a `Bound::User` obligation, R2) is composed and monomorphized
    // like any other reachable generic callee -- a third symbol, not codegen
    // parity with the pre-flip splice (exit criterion 8 scopes parity to
    // behaviour/stdout, not IL).
    assert_eq!(
        monomorph_symbols(&entry),
        [
            "sooth_mono_g__m0__t0_i64",
            "sooth_mono_gt__m4__t0_i64",
            "sooth_mono_h__m0__t0_i64"
        ]
    );
}

/// R4/R8: a composed callee returning two or more values gets the same interned
/// return bundle a concrete instantiation's callee gets. The bundle is interned
/// by the fixpoint, not by the concrete `out_arity >= 2` loop, which never sees
/// this callee -- without it the call site would lower against `bundle: None`
/// and read a single scalar back out of a two-field aggregate.
#[test]
fn a_composed_callee_returning_a_bundle_is_laid_out_like_any_other() {
    let t = Tree::new("bundle");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         : two ( 'T -- 'T i64 ) 9 ;\n\
         : g ( 'T -- 'T i64 ) two ;\n\
         : main ( -- ) 7 g . . ;\n",
    );
    assert_eq!(build_and_run(&entry), "9\n7\n");
}

/// R4/N1: a cross-call whose caller or callee name is a polymorphic overload
/// set. Both candidates' records merge under one name and each indexes its own
/// signature's variables, so composing would ground the wrong monomorph
/// silently. Rejected at the call site instead -- and note this is a *build*
/// golden: an unrouted cross-call is a panic in lowering, not a diagnostic, so
/// the rejection has to happen at check time to be observable at all.
#[test]
fn a_cross_call_through_an_overloaded_generic_word_is_a_located_rejection() {
    let t = Tree::new("overloaded");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : id ( 'A 'B -- 'A 'B ) swap swap ;\n\
         : g ( 'T -- 'T ) id ;\n\
         : main ( -- ) 7 g . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("`g` cannot call the polymorphic word `id` (line 4, col 18)")
            && err.contains("`id` names more than one polymorphic word"),
        "unexpected diagnostic: {err}"
    );
}

/// R4, at the linked artefact: a two-variable callee reached both from a
/// concrete caller and through a cross-call links **one** monomorph, not two.
/// Asymmetric (`i64`, `str`) on purpose -- a composed θ ordered the other way
/// would mint `…__t0_str_t1_i64` and land a second, redundant `IrFunc`, which a
/// symmetric pair could not distinguish.
#[test]
fn a_callee_reached_concretely_and_across_a_cross_call_links_one_monomorph() {
    let t = Tree::new("shared-callee");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         : swap2 ( 'A 'B -- 'B 'A ) swap ;\n\
         : g ( 'X 'Y -- 'Y 'X ) swap2 ;\n\
         : main ( -- ) 1 \"s\" swap2 drop drop 2 \"t\" g drop drop ;\n",
    );
    assert_eq!(
        monomorph_symbols(&entry),
        [
            "sooth_mono_g__m0__t0_i64_t1_str",
            "sooth_mono_swap2__m0__t0_i64_t1_str",
        ]
    );
}
