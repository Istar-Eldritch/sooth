//! P7b.S6 Phase 1 exit golden (R2.a): the M3 concrete-effect quotation
//! operand at an HKT member -- pre-fix an ICE in `trait_member_operand_error`
//! (`substitute_member_var` leaving a member-local quotation var untouched
//! and indexing off the end of the caller's `ty_var_names`), post-fix a
//! located diagnostic. Harness style from `tests/phase7b_slice4.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs6-{}-{tag}-{seq}", std::process::id()));
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

/// `tests/phase7b_slice4.rs`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs6 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

fn build_error(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// Build `src` and run the produced binary, returning its stdout. A build
/// failure panics with the compiler's stderr: every caller of this helper
/// expects a value, so a rejection is itself the regression to surface.
fn build_and_run(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should have succeeded, stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    String::from_utf8(run.stdout).expect("stdout should be utf8")
}

/// M3/Phase 2 (R2.b): `bump`'s forwarded quotation parameter has a
/// *concrete* effect (`[ i64 -- i64 ]`) rather than the member-local
/// `'T`/`'U` spelling, so the operand slot folds to
/// `PolyType::Concrete(Type::Quotation(..))` while `map`'s declared input
/// stays a `PolyType::Quotation(..)`. Pre-Phase-2 this fell through
/// `unify_member_operand`'s catch-all (a located error, Phase 1's fix over
/// what used to panic); Phase 2's cross-representation bridge arm now binds
/// `map`'s `'T`/`'U` to `i64` and dispatches, printing the bumped value --
/// the dead criterion this golden replaces (a rename, not a new fixture) is
/// the old "still rejected" assertion.
#[test]
fn poly_body_forwards_a_concrete_effect_quotation_parameter_to_a_member() {
    let stdout = build_and_run(
        "m3-concrete-effect",
        "\
import: core::option * ;\n\
trait: Functor['F: * -> *] :\n\
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
;\n\
impl: Functor for Option\n\
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
;\n\
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;\n\
: bump['F: Functor] ( 'F[i64] [ i64 -- i64 ] -- 'F[i64] ) map ;\n\
: main ( -- ) 3 Some [ 1 sub ] bump showopt ;\n\
",
    );
    assert_eq!(stdout, "2\n");
}

/// Non-regression (R2.b): the already-working generic-effect quotation
/// parameter (M3's headline case, the existing `Quotation`/`Quotation` arm)
/// still dispatches after Phase 2's new arm lands beside it.
#[test]
fn poly_body_forwards_a_generic_effect_quotation_parameter() {
    let stdout = build_and_run(
        "m3-generic-effect",
        "\
import: core::option * ;\n\
trait: Functor['F: * -> *] :\n\
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
;\n\
impl: Functor for Option\n\
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
;\n\
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;\n\
: bump['F: Functor 'A] ( 'F['A] [ 'A -- 'A ] -- 'F['A] ) map ;\n\
: main ( -- ) 3 Some [ 1 sub ] bump showopt ;\n\
",
    );
    assert_eq!(stdout, "2\n");
}

/// M3/R2.b: a written quotation **literal** (as opposed to a forwarded
/// parameter) at the same member call stays a located error -- no
/// materialization is attempted at a poly member call site this slice, and
/// the fixture must neither panic nor silently dispatch.
#[test]
fn poly_body_quotation_literal_member_operand_is_located_error() {
    let stderr = build_error(
        "m3-quot-literal",
        "\
import: core::option * ;\n\
trait: Functor['F: * -> *] :\n\
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
;\n\
impl: Functor for Option\n\
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
;\n\
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;\n\
: bump['F: Functor] ( 'F[i64] -- 'F[i64] ) [ 1 sub ] map ;\n\
: main ( -- ) 3 Some bump showopt ;\n\
",
    );
    assert!(
        stderr.contains("`map` of `Functor`"),
        "expected a located `map`/`Functor` mismatch, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "the quotation-literal fixture must not panic, got: {stderr}"
    );
}

/// Phase 3 (M4/R3/R7): the exit-criterion fixture -- `impl: Foldable for
/// List` with a non-inline body that actually recurses through the
/// self-referencing `^List['T]` field (`Cons> | v rest | ... rest ^> ...
/// fold`), pre-fix an `unreachable!` panic on the missing `OwnedCell` arm
/// in both `substitute_generic_variant_field` and `poly_bind_construction_arg`.
/// Sums a 3-element `List[i64]`; no recursion wall per R3 (a non-inline
/// member's self-call mints an ordinary `IrFunc`, no combinator budget).
#[test]
fn impl_foldable_for_list_dispatches() {
    let stdout = build_and_run(
        "p3-foldable-list",
        "\
 import: core::list * ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] i64 [ i64 'T -- i64 ] -- i64 ) ;\n\
 ;\n\
 impl: Foldable for List\n\
   : fold | f | | acc |\n\
     ~[ ( Nil ) drop acc ]\n\
     ~[ ( Cons ) Cons> | v rest |\n\
        acc v f call rest ^> swap f fold ]\n\
     List? ;\n\
 ;\n\
 : mkempty ( -- List[i64] ) Nil ;\n\
 : main ( -- )\n\
   3 mkempty ^ Cons\n\
   2 swap ^ Cons\n\
   1 swap ^ Cons\n\
   0 [ add ] fold . ;\n",
    );
    assert_eq!(stdout, "6\n");
}

/// Phase 3 (R7): the destructor witness -- a multi-element `List[str]`
/// (a linear payload, unlike `i64`) disposed via a trailing `drop`. Verbatim
/// the probe round's `p5_list_str_payload.sth` (`slice6-probes.md`), but
/// importing `core::list` rather than declaring the type inline, so this
/// also exercises the promoted module. Builds, runs, exits 0 with no
/// output -- no leak, no double-free.
#[test]
fn multi_element_list_of_str_drops_clean() {
    let stdout = build_and_run(
        "p3-list-str-drop",
        "\
 import: core::list * ;\n\
 : mkempty ( -- List[str] ) Nil ;\n\
 : main ( -- )\n\
   \"c\" mkempty ^ Cons\n\
   \"b\" swap ^ Cons\n\
   \"a\" swap ^ Cons\n\
   drop ;\n",
    );
    assert_eq!(stdout, "");
}

/// Phase 3 (R7): the promotion witness -- `List['T]` is usable from an
/// importing module (not just declared inline in the same file), pinning
/// the `sooth.pkg` `module:` wiring. The self-reference builds and runs
/// exactly as the probe round's `p5_list_selfref.sth` did with an inline
/// declaration.
#[test]
fn list_self_reference_builds_across_the_core_import() {
    let stdout = build_and_run(
        "p3-list-core-import",
        "\
 import: core::list * ;\n\
 : mklist ( i64 -- List[i64] ) Nil ^ Cons ;\n\
 : main ( -- ) 5 mklist drop ;\n",
    );
    assert_eq!(stdout, "");
}

/// Phase 4 (R4): `Monoid.empty` has no dispatchable input -- `'T` never
/// appears in an input position, so a mono body's ordinary operand-dispatch
/// loop can never win a candidate for it. An explicit `empty[i64]` grounds
/// `'T` directly from the call site's type argument instead, dispatching to
/// the `i64` impl (`empty` = 5) and feeding `combine` (`add`): `7 + 5 = 12`.
#[test]
fn nullary_trait_member_grounds_from_explicit_instantiation() {
    let stdout = build_and_run(
        "p4-nullary-explicit",
        "\
 trait: Monoid['T] :\n\
   empty ( -- 'T ) ;\n\
   : combine ( 'T 'T -- 'T ) ;\n\
 ;\n\
 impl: Monoid for i64\n\
   : empty 5 ;\n\
   : combine add ;\n\
 ;\n\
 : main ( -- ) 7 empty[i64] combine . ;\n",
    );
    assert_eq!(stdout, "12\n");
}

/// Phase 4 (R5): Q1 rules out consuming-context inference for this slice --
/// bare `empty` (no explicit instantiation) in a mono body is a located
/// error citing the `empty[i64]`-style remedy, not a panic and not a silent
/// accept.
#[test]
fn bare_nullary_member_without_instantiation_is_located_error() {
    let stderr = build_error(
        "p4-nullary-bare",
        "\
 trait: Monoid['T] :\n\
   empty ( -- 'T ) ;\n\
   : combine ( 'T 'T -- 'T ) ;\n\
 ;\n\
 impl: Monoid for i64\n\
   : empty 5 ;\n\
   : combine add ;\n\
 ;\n\
 : main ( -- ) empty drop ;\n",
    );
    assert!(
        stderr.contains("empty[i64]"),
        "expected the explicit-instantiation remedy, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "bare `empty` must not panic, got: {stderr}"
    );
}

/// Phase 5 (R6): `Monoid for i64` -- the measured checklist row (M1/M2),
/// exercised as a real `combine`/`combine`/`empty[i64]` chain rather than
/// the single `empty[i64] combine` shape Phase 4's golden already pins.
/// `3 + 4 = 7`, `7 + 0 = 7`.
#[test]
fn monoid_for_i64_combine_and_empty_dispatch() {
    let stdout = build_and_run(
        "p5-monoid-i64",
        "\
 trait: Monoid['T] :\n\
   empty ( -- 'T ) ;\n\
   : combine ( 'T 'T -- 'T ) ;\n\
 ;\n\
 impl: Monoid for i64\n\
   : empty 0 ;\n\
   : combine add ;\n\
 ;\n\
 : main ( -- ) 3 4 combine empty[i64] combine . ;\n",
    );
    assert_eq!(stdout, "7\n");
}

/// Phase 5 (R6a): `mconcat` per the bound-quotation-parameter spelling,
/// dispatching `Foldable.fold`/`Monoid.empty`/`combine` together over the
/// real `core::option`. `Some(5)` folds to `5` (the `None` case never
/// reached, so `empty`'s `0` never surfaces).
#[test]
fn mconcat_over_option_dispatches() {
    let stdout = build_and_run(
        "p5-mconcat-option",
        "\
 import: core::option * ;\n\
 trait: Monoid['T] :\n\
   empty ( -- 'T ) ;\n\
   : combine ( 'T 'T -- 'T ) ;\n\
 ;\n\
 impl: Monoid for i64\n\
   : empty 0 ;\n\
   : combine add ;\n\
 ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] 'A [ 'A 'T -- 'A ] -- 'A ) ;\n\
 ;\n\
 impl: Foldable for Option\n\
   : fold | f | | acc |\n\
     ~[ ( Some ) Some> acc swap f call ]\n\
     ~[ ( None ) drop acc ]\n\
     Option? ;\n\
 ;\n\
 : mkopt ( i64 -- Option[i64] ) Some ;\n\
 : mconcat['F: Foldable 'T: Monoid] ( 'F['T] [ 'T 'T -- 'T ] -- 'T ) empty swap fold ;\n\
 : main ( -- ) 5 mkopt [ combine ] mconcat . ;\n",
    );
    assert_eq!(stdout, "5\n");
}

/// Phase 5 (R6a): `mconcat` over the real `core::list`, summing a 3-element
/// `List[i64]` (`1 + 2 + 3 = 6`) through the same bound-parameter spelling.
/// `fold`'s body only destructures and never reconstructs a `List`, so it
/// does not hit the Phase 5 construction wall (closed by P7b.S8b; see
/// `monoid_for_list_append_construction_builds_and_runs_clean` below).
#[test]
fn mconcat_over_list_dispatches() {
    let stdout = build_and_run(
        "p5-mconcat-list",
        "\
 import: core::list * ;\n\
 trait: Monoid['T] :\n\
   empty ( -- 'T ) ;\n\
   : combine ( 'T 'T -- 'T ) ;\n\
 ;\n\
 impl: Monoid for i64\n\
   : empty 0 ;\n\
   : combine add ;\n\
 ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] 'A [ 'A 'T -- 'A ] -- 'A ) ;\n\
 ;\n\
 impl: Foldable for List\n\
   : fold | f | | acc |\n\
     ~[ ( Nil ) drop acc ]\n\
     ~[ ( Cons ) Cons> | v rest |\n\
        acc v f call rest ^> swap f fold ]\n\
     List? ;\n\
 ;\n\
 : mkempty ( -- List[i64] ) Nil ;\n\
 : mconcat['F: Foldable 'T: Monoid] ( 'F['T] [ 'T 'T -- 'T ] -- 'T ) empty swap fold ;\n\
 : main ( -- )\n\
   3 mkempty ^ Cons\n\
   2 swap ^ Cons\n\
   1 swap ^ Cons\n\
   [ combine ] mconcat . ;\n",
    );
    assert_eq!(stdout, "6\n");
}

/// Phase 5 (R6) / P7b.S8b (PB-1): `Monoid for List['T]` -- a real linear
/// merge over two spines, `combine` recursing through the self-reference
/// and reconstructing a `Cons` on the way back out. This was the S6
/// *recorded wall*: the declared `^List['T]` self-reference field arrived
/// at `poly_bind_construction_arg` as a bare `PolyType::Generic`, which no
/// arm covered, and the catch-all panicked. S8b's `Generic` field arm
/// (bind positionally against a same-identity `Generic` operand) removes
/// the wall, so the fixture is now the positive golden it was always meant
/// to be: the build grounds, the run appends the two spines, and `main`
/// drops the result (empty stdout -- the element-carrying goldens pin
/// visible output elsewhere). The pre-S8b panic text
/// (`a generic \`type:\` field is never Generic`) is pinned verbatim in
/// docs/roadmap/P7b/slice8-probes.md (P8-2b) and slice8b-probes.md.
#[test]
fn monoid_for_list_append_construction_builds_and_runs_clean() {
    let (_t, entry) = single_file_hosted(
        "p5-monoid-list-wall",
        "\
 import: core::list * ;\n\
 trait: Monoid['T] :\n\
   empty ( -- 'T ) ;\n\
   : combine ( 'T 'T -- 'T ) ;\n\
 ;\n\
 impl: Monoid for List\n\
   : empty Nil ;\n\
   : combine\n\
     swap\n\
     ~[ ( Nil ) drop ]\n\
     ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]\n\
     List? ;\n\
 ;\n\
 : mkempty ( -- List[i64] ) Nil ;\n\
 : main ( -- )\n\
   3 mkempty ^ Cons 2 swap ^ Cons 1 swap ^ Cons\n\
   3 mkempty ^ Cons 5 swap ^ Cons\n\
   combine drop ;\n",
    );
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "the S8b arm removed the construction wall; build should succeed, got: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(
        run.status.success(),
        "the appended list should drop cleanly (exit 0)"
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "");
}

/// Phase 5 (R1 exit criterion): a single program that `map`s and `fold`s
/// over `Option`, `Result`, and `List` through shared `Functor`/`Foldable`
/// bounds, with impls on the real lib types. `Functor for List` is omitted
/// here -- S6 recorded it against the same construction wall as `Monoid for
/// List` (any trait-member body over `List` that builds a `Cons`), a wall
/// P7b.S8b closes; its dropped golden lands in S8b's own Phase 3. This
/// program witnesses the non-array clauses that ground: `map` over `Option`,
/// `fold` over all three.
#[test]
fn dogfood_maps_and_folds_over_option_result_and_list_through_shared_bounds() {
    let stdout = build_and_run(
        "p5-dogfood",
        "\
 import: core::option * ;\n\
 import: core::result * ;\n\
 import: core::list * ;\n\
 trait: Functor['F: * -> *] :\n\
   map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;\n\
 ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] 'A [ 'A 'T -- 'A ] -- 'A ) ;\n\
 ;\n\
 impl: Functor for Option\n\
   : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;\n\
 ;\n\
 impl: Foldable for Option\n\
   : fold | f | | acc |\n\
     ~[ ( Some ) Some> acc swap f call ]\n\
     ~[ ( None ) drop acc ]\n\
     Option? ;\n\
 ;\n\
 impl: Foldable for Result\n\
   : fold | f | | acc |\n\
     ~[ ( Ok ) Ok> acc swap f call ]\n\
     ~[ ( Err ) drop acc ]\n\
     Result? ;\n\
 ;\n\
 impl: Foldable for List\n\
   : fold | f | | acc |\n\
     ~[ ( Nil ) drop acc ]\n\
     ~[ ( Cons ) Cons> | v rest |\n\
        acc v f call rest ^> swap f fold ]\n\
     List? ;\n\
 ;\n\
 : mkopt ( i64 -- Option[i64] ) Some ;\n\
 : mkres ( i64 -- Result[i64 i64] ) Ok ;\n\
 : mkempty ( -- List[i64] ) Nil ;\n\
 : main ( -- )\n\
   3 mkopt [ 1 add ] map[i64 i64] 0 [ add ] fold .\n\
   10 mkres 0 [ add ] fold .\n\
   3 mkempty ^ Cons 2 swap ^ Cons 1 swap ^ Cons\n\
   0 [ add ] fold . ;\n",
    );
    assert_eq!(stdout, "4\n10\n6\n");
}

/// Phase 5 (R9): the first of the two genuinely-rejecting linearity
/// fixtures the probe log's p3 section measured -- a `Foldable.fold` arm
/// that never consumes the destructured payload (no `drop`, no call).
/// Rejected by the pre-existing per-arm variant-consumption / arm-shape-
/// parity check (`Option?`'s two arms must leave the same stack shape),
/// not by any Foldable-specific rule -- S6 adds none (R9). No per-rule
/// mutation claim: deleting the general arm-parity check breaks the whole
/// suite and cannot discriminate a Foldable-only rule that does not exist.
#[test]
fn fold_body_never_consuming_the_payload_is_arm_shape_parity_error() {
    let stderr = build_error(
        "p5-r9-never-drop",
        "\
 import: core::option * ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] i64 [ i64 'T -- i64 ] -- i64 ) ;\n\
 ;\n\
 impl: Foldable for Option\n\
   : fold | f | | acc |\n\
     ~[ ( Some ) Some> acc ]\n\
     ~[ ( None ) drop acc ]\n\
     Option? ;\n\
 ;\n\
 : mkopt ( i64 -- Option[i64] ) Some ;\n\
 : main ( -- ) 3 mkopt 10 [ add ] fold . ;\n",
    );
    assert!(
        stderr.contains("leave different stack shapes"),
        "expected the arm-shape-parity rejection, got: {stderr}"
    );
}

/// Phase 5 (R9): the second genuinely-rejecting fixture -- a `Foldable.fold`
/// arm calling the linear accumulator quotation `f` twice. Rejected by
/// ordinary `call` arity underflow (the first `call` already consumed the
/// stack `f` needed), not a linearity rule specific to `Foldable` -- same
/// no-per-rule-mutation-claim rationale as above (R9).
#[test]
fn fold_body_calling_accumulator_quotation_twice_is_arity_error() {
    let stderr = build_error(
        "p5-r9-double-use",
        "\
 import: core::option * ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] i64 [ i64 'T -- i64 ] -- i64 ) ;\n\
 ;\n\
 impl: Foldable for Option\n\
   : fold | f | | acc |\n\
     ~[ ( Some ) Some> acc swap f call f call ]\n\
     ~[ ( None ) drop acc ]\n\
     Option? ;\n\
 ;\n\
 : mkopt ( i64 -- Option[i64] ) Some ;\n\
 : main ( -- ) 3 mkopt 10 [ add ] fold . ;\n",
    );
    assert!(
        stderr.contains("needs 2 values, but the stack holds 1"),
        "expected the `call` arity-underflow rejection, got: {stderr}"
    );
}

/// Phase 5 (R9, noted not enforced): forgetting to consume a destructured
/// payload via an explicit `drop` (as opposed to never touching it at all,
/// the previous test) is **legal** -- DESIGN.md's explicit-destructor rule
/// makes `drop` the deliberate discard, not a linearity violation. S6 does
/// not make this an error, and no golden claims otherwise; this pins the
/// legal behaviour so a future change does not silently start rejecting it.
#[test]
fn fold_body_dropping_the_payload_is_legal_not_an_error() {
    let stdout = build_and_run(
        "p5-r9-forget-via-drop",
        "\
 import: core::option * ;\n\
 trait: Foldable['F: * -> *] :\n\
   fold ( 'F['T] i64 [ i64 'T -- i64 ] -- i64 ) ;\n\
 ;\n\
 impl: Foldable for Option\n\
   : fold | f | | acc |\n\
     ~[ ( Some ) Some> drop acc ]\n\
     ~[ ( None ) drop acc ]\n\
     Option? ;\n\
 ;\n\
 : mkopt ( i64 -- Option[i64] ) Some ;\n\
 : main ( -- ) 3 mkopt 10 [ add ] fold . ;\n",
    );
    assert_eq!(stdout, "10\n");
}

/// Post-implementation review fix: a poly-body `^`-built `OwnedCell` whose
/// payload never reaches the word's own declared input/output signature (a
/// body-internal temporary, immediately unwrapped and dropped) used to
/// panic at lowering for every monomorphization -- `apply_subst`'s
/// `OwnedCell` arm, the only place that mints an `OwnedCellId`, runs only
/// while substituting a *declared* input/output `PolyType`, so a cell shape
/// invisible to the signature was never interned and `cell_id_of`'s
/// structural lookup had nothing to match. This pins the minimal repro
/// (`Copy` payload) building and running clean.
#[test]
fn poly_body_internal_owned_cell_temporary_interns_and_runs() {
    let stdout = build_and_run(
        "cell-temp-copy",
        "\
: leak['T] ( 'T -- )\n\
  ^ drop\n\
;\n\
: main ( -- ) 5 leak \"ok\" . ;\n",
    );
    assert_eq!(stdout, "ok");
}

/// The non-`Copy` variant of the fix above: a heap-owning payload (`str`)
/// that is never returned to the caller still needs its destructor to run
/// correctly through the interned-on-demand cell, not just build without
/// panicking.
#[test]
fn poly_body_internal_owned_cell_temporary_str_payload_drops_clean() {
    let stdout = build_and_run(
        "cell-temp-str",
        "\
: leak['T] ( 'T -- )\n\
  ^ drop\n\
;\n\
: main ( -- ) \"temp\" leak \"ok\" . ;\n",
    );
    assert_eq!(stdout, "ok");
}
