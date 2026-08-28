//! P7.S12 phase 1 goldens (R8.1): the three live-at-`afd3d52` defects the
//! spec's B1/B2/B3 repros name -- a false rejection (`Eliminate`), a
//! miscompiled field read (`Destructure`), and a runtime death or build
//! failure (`Construct`) -- all three the same defect: a poly body's
//! generated-enum-word call resolves through a bare, last-write-wins key
//! shared by every monomorph of one header. Each repro runs in **both**
//! declaration orders: the defect *is* order sensitivity, so a single order
//! cannot witness a last-write-wins key.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s12-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, common::fixture_source("prog.sth", contents)).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

/// Build and run `src`, asserting a clean (signal-free) exit -- a segfault or
/// backend panic is a failure here, not a missing line (R8's own rule).
fn build_and_run(src: &Path) -> (String, i32) {
    let binary = driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

const TYPES: &str = "type: Pair['A] | Nil | One 'A ;\n\
     type: Pt x i64 y i64 ;\n";

/// B1 (`Eliminate`): the `mk1`-then-`mk2` order -- false-rejected at
/// `afd3d52` (R1.1's fix). The registry's bare key must never decide which
/// monomorph a call site's own scrutinee names.
#[test]
fn eliminate_over_asymmetric_monomorphs_builds_mk1_then_mk2() {
    let src = format!(
        "{TYPES}\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) drop 1 ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b1a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
}

/// B1 (`Eliminate`): the `mk2`-then-`mk1` order, live-clean at `afd3d52`.
/// Must build and print identically to the order above.
#[test]
fn eliminate_over_asymmetric_monomorphs_builds_mk2_then_mk1() {
    let src = format!(
        "{TYPES}\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) drop 1 ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b1b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
}

/// B2 (`Destructure`): the `mk1`-then-`mk2` order -- false-rejected at
/// `afd3d52` for the same reason B1's identical order is.
#[test]
fn destructure_over_asymmetric_monomorphs_builds_mk1_then_mk2() {
    let src = format!(
        "{TYPES}\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) One> ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b2a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n");
}

/// B2 (`Destructure`): the `mk2`-then-`mk1` order -- live-clean at `afd3d52`.
/// Must print the same `7` as the order above.
///
/// Review note: this witnesses R1.1 alone, not R1.2/R1.3. `use`'s own
/// eliminator arm narrows `One>`'s operand to a concrete `Type::Variant`
/// already carrying the scrutinee's real id (R1.1's fix, inside
/// `poly_eliminator_call`) -- `use` never re-instantiates at a second enum
/// monomorph, so there is no per-splice ambiguity left for `enum_words` to
/// resolve at lowering. Confirmed by mutation: disabling R1.3's lowering
/// override entirely still leaves both `destructure_*` tests green.
#[test]
fn destructure_over_asymmetric_monomorphs_builds_mk2_then_mk1() {
    let src = format!(
        "{TYPES}\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : mk1 ( i64 -- Pair[i64] ) One ;\n\
         : use ( Pair[i64] 'T -- i64 'T )\n\
           | keep |\n\
           ~[ ( One ) One> ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair?\n\
           keep ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 5 use drop . ;\n"
    );
    let prog = Scratch::write("b2b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n");
}

/// B3 (`Construct`): the `mk2`-then-`mk1` order -- builds clean and then
/// segfaults at `afd3d52` (R1.2/R1.2a/R1.3's fix, since `wrap`'s `One` call
/// needs its per-θ instantiation grounded through the live instantiator). The
/// order that segfaults vs. the order that fails to build is a QBE
/// implementation detail of the last-write-wins collision, not a claim this
/// test depends on -- both orders are asserted clean either way.
#[test]
fn construct_inside_a_generic_word_builds_mk2_then_mk1() {
    let src = format!(
        "{TYPES}\
         : wrap ( 'T -- Pair['T] ) One ;\n\
         : mk2 ( Pt -- Pair[Pt] ) wrap ;\n\
         : mk1 ( i64 -- Pair[i64] ) wrap ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 drop ;\n"
    );
    let prog = Scratch::write("b3a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
}

/// B3 (`Construct`): the `mk1`-then-`mk2` order -- does not even build at
/// `afd3d52` ("an aggregate field is copied by blit, not scalar-stored").
/// Must build and run clean, identically to the order above.
#[test]
fn construct_inside_a_generic_word_builds_mk1_then_mk2() {
    let src = format!(
        "{TYPES}\
         : wrap ( 'T -- Pair['T] ) One ;\n\
         : mk1 ( i64 -- Pair[i64] ) wrap ;\n\
         : mk2 ( Pt -- Pair[Pt] ) wrap ;\n\
         : main ( -- )\n\
           1 2 Pt mk2 drop\n\
           7 mk1 drop ;\n"
    );
    let prog = Scratch::write("b3b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
}

/// R1.5: a combinator (`declares_inline`) whose own body constructs a
/// generic enum still ungrounded at its own type variable. A `Span`-keyed
/// `enum_words` cannot hold two splices' distinct resolutions for the one
/// body span this call sits at (R1.5's deliberate non-goal, no `(uid, span)`
/// widening), so this is a located rejection rather than a silent
/// last-write-wins collision behind a different door.
#[test]
fn combinator_constructing_ungrounded_generic_enum_is_rejected() {
    let src = "import: intrinsics * ;\n\
         type: Pair['A] | Nil | One 'A ;\n\
         trait: Foo['T] : bar inline ( 'T -- 'T ) ; ;\n\
         impl: Foo for i64\n\
           : bar | x | x ;\n\
         ;\n\
         : wrap_it inline ['T: Foo] ( 'T -- Pair['T] ) bar One ;\n\
         : main ( -- ) 1 wrap_it drop ;\n";
    let prog = Scratch::write("r15", src);
    let err =
        driver::build_with_manifest(prog.path(), common::manifest_for(prog.path()).as_deref())
            .expect_err("a combinator constructing an ungrounded generic enum is rejected");
    assert!(
        err.contains("constructs `Pair`")
            && err.contains("this combinator's own splice determines"),
        "expected a located R1.5 rejection naming the enum and the restriction, got: {err}"
    );
}

/// R1.2a: a poly body's *own* generic instantiation, minted mid-walk
/// (`inner`'s `One`, which mints `Pair[Pt]`), must still be visible to the
/// poly-to-poly call that reached it (`outer` calling `inner`). Two halves
/// carry that, and this is the only test load-bearing for either: reverting
/// one leaves the rest of the suite green.
///
/// - `discover_transitive_instantiations` takes the live `generics_cell`
///   rather than `None`. Reverted, this fixture is falsely rejected
///   ("cannot yet be instantiated at a variable-bearing application").
/// - `module.generics = generics_cell.into_inner()` sits *past* that call,
///   so the cell is still live while the fixpoint grounds `enum_words`, and
///   what it mints is flushed at the end of the fixpoint. Reverted, this
///   fixture ICEs in lowering ("checked user word exists").
#[test]
fn poly_to_poly_call_sees_a_generic_instantiation_minted_mid_walk() {
    let src = format!(
        "{TYPES}\
         : inner ( 'T -- i64 ) One drop 5 ;\n\
         : outer ( 'U -- i64 ) inner ;\n\
         : main ( -- ) 1 2 Pt outer . ;\n"
    );
    let prog = Scratch::write("r12a", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "5\n");
}

/// R1.1 review fix: an eliminator whose scrutinee is a monomorph this body's
/// own walk minted moments ago. `rt`'s `One` mints `Pair[i64]` mid-walk while
/// the registry's one flushed entry is `mk2`'s `Pair[Pt]`, so the scrutinee's
/// `EnumId` sits past `enums.len()` until `check_module` flushes. Both
/// id-to-decl lookups in `poly_eliminator_call` therefore have to consult the
/// live `GenericTypes`; indexing `enums` unconditionally (as phase 1 first
/// shipped) panics at either one with "index out of bounds: the len is 1 but
/// the index is 1".
///
/// The rejection asserted below is the pre-existing poly-destructure
/// restriction and is incidental to that point. If a later phase lifts it,
/// switch this to assert a clean build rather than deleting the fixture: no
/// other test reaches the guarded arms.
#[test]
fn eliminating_a_body_local_monomorph_diagnoses_rather_than_ices() {
    let src = format!(
        "{TYPES}\
         : mk2 ( Pt -- Pair[Pt] ) One ;\n\
         : rt ( 'T -- i64 )\n\
           drop 5 One\n\
           ~[ ( One ) One> ]\n\
           ~[ ( Nil ) drop 0 ]\n\
           Pair? ;\n\
         : main ( -- ) 1 2 Pt mk2 drop 1 2 Pt rt . ;\n"
    );
    let prog = Scratch::write("r11-local-mint", &src);
    let err =
        driver::build_with_manifest(prog.path(), common::manifest_for(prog.path()).as_deref())
            .expect_err("the poly-destructure restriction rejects this body");
    assert!(
        err.contains("`One>` is not permitted on `Pair[i64].One`"),
        "expected a diagnostic naming the body-local monomorph, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Phase 3 (R2 + R5): the registry admits a generic header, and an ungrounded
// generic scrutinee is eliminated inside a polymorphic body.
// ---------------------------------------------------------------------------

/// The build error a fixture that must be rejected produces.
fn build_error(src: &Path) -> String {
    driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect_err("this fixture must be rejected")
}

const OPTION: &str = "type: Option['T] | None | Some 'T ;\n\
     type: Pt x i64 y i64 ;\n";

/// The brief's own repro, and this slice's exit criterion (R8.2): `is-some`
/// over an ungrounded `Option['T]`, run at **two** instantiations whose
/// payload layouts differ (`i64`, and a two-field struct). One instantiation
/// cannot witness R1 -- S3a R4's rule: an all-`i64` pair is a layout placebo.
/// Both arms are exercised, so neither the `Some` nor the `None` narrowing is
/// merely registered.
#[test]
fn is_some_over_an_ungrounded_option_runs_at_two_payload_layouts() {
    let src = format!(
        "{OPTION}\
         : is-some ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : nonei ( -- Option[i64] ) None ;\n\
         : mkp ( Pt -- Option[Pt] ) Some ;\n\
         : main ( -- )\n\
           7 mki is-some .\n\
           nonei is-some .\n\
           1 2 Pt mkp is-some . ;\n"
    );
    let prog = Scratch::write("r82", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n1\n");
}

/// R8.2, the other declaration order: the struct instantiation is minted
/// first. The defect this slice closes *is* order sensitivity, so an exit
/// criterion asserted in one order only would not say the registry entry has
/// stopped deciding identity.
#[test]
fn is_some_over_an_ungrounded_option_runs_with_the_struct_monomorph_first() {
    let src = format!(
        "{OPTION}\
         : is-some ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mkp ( Pt -- Option[Pt] ) Some ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- )\n\
           1 2 Pt mkp is-some .\n\
           7 mki is-some . ;\n"
    );
    let prog = Scratch::write("r82b", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n1\n");
}

/// R1.1/R5.1, the branch selection's *other* direction, end to end: nothing
/// in this program instantiates `Option` at parse time, so the registry gate
/// is `EliminatorTarget::Generic` -- yet the scrutinee `probe` eliminates is
/// the concrete `Option[i64]` its own walk minted one term earlier. The
/// concrete branch has to run under a generic gate, which is what "the
/// registry entry gates the name, the scrutinee decides the header" buys.
#[test]
fn a_generic_gate_eliminates_a_scrutinee_its_own_walk_minted() {
    let src = format!(
        "{OPTION}\
         : probe ( 'T -- i64 )\n\
           drop\n\
           5 Some\n\
           ~[ ( Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : main ( -- ) 1 2 Pt probe . ;\n"
    );
    let prog = Scratch::write("r11-generic-gate", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
}

/// R8.6: an ungrounded scrutinee with a missing arm. The exhaustiveness pass
/// reads the *header's* variant list, so it names the absent variant exactly
/// as the concrete branch does.
#[test]
fn ungrounded_scrutinee_with_a_missing_arm_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 1 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r86-miss", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("non-exhaustive call to `Option?`")
            && err.contains("missing variant `None` of enum `Option`"),
        "expected a located rejection naming the enum and the call, got: {err}"
    );
}

/// R8.6: two arms tagged with one variant of an ungrounded scrutinee.
#[test]
fn ungrounded_scrutinee_with_a_duplicate_arm_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 1 ]\n\
           ~[ ( Some ) drop 2 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r86-dup", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("duplicate arm for variant `Some` of enum `Option`")
            && err.contains("call to `Option?`"),
        "expected a located rejection naming the enum and the call, got: {err}"
    );
}

/// R8.6: an arm tagged with a variant that is not the scrutinee's. A *typo'd*
/// tag cannot witness this -- an unresolvable tag is a parse error ("unknown
/// type `Somme`") long before the checker sees it -- so the tag names a real
/// variant of an unrelated enum, which is what the concrete branch's own
/// witness does.
#[test]
fn ungrounded_scrutinee_with_an_unknown_variant_tag_is_rejected() {
    let src = format!(
        "{OPTION}\
         type: Other | Alt n i64 ;\n\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Alt ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r86-unknown", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("unknown variant `Alt` of enum `Option`") && err.contains("call to `Option?`"),
        "expected a located rejection naming the enum and the call, got: {err}"
    );
}

/// R8.6/R5.5: a narrowed `GenericVariant` leaving its arm. This is the
/// load-bearing escape check (R3.4): outside the call every type-directed
/// predicate is written over `Type::Enum`, so an escaped variant reads as
/// trivially `Copy` and a later `dup` double-drops a linear payload. Also the
/// only witness that the arm input really is the narrowed variant -- the
/// message renders it `Option.Some` through `poly_type_str`.
#[test]
fn a_generic_variant_escaping_its_arm_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Some ) ]\n\
           ~[ ( None ) ]\n\
           Option? drop 1 ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r86-escape", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("an arm of `Option?` leaves `Option.Some` on the stack"),
        "expected a located rejection naming the narrowed variant, got: {err}"
    );
}

/// R3.1: `dup` on a narrowed `GenericVariant`, the copy rule the escape check
/// above exists to protect. `poly_is_copy` never admits a narrowed variant --
/// `Option.Some` wraps a payload that is linear at some instantiation -- so the
/// rejection must name the variant rather than fall through to a generic
/// `cannot dup` over the scrutinee's own `Option['T]`.
#[test]
fn duplicating_a_narrowed_generic_variant_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Some ) dup drop drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r31-copy", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("cannot `dup` a generic variant")
            && err.contains("`Option.Some` may carry a linear field"),
        "expected the narrowed-variant copy rejection, got: {err}"
    );
}

/// R8.6/R5.7: a `&`-mode arm tag over a generic scrutinee. Narrowing one
/// needs `intern_ref_type` over a shape with no `Type` yet (R4.3's explicit
/// non-goal), so this slice admits the owning mode only -- a located
/// restriction, not silence and not a fallthrough into the concrete branch,
/// which would let the arm consume a value the caller only lent.
#[test]
fn a_reference_mode_tag_over_a_generic_scrutinee_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( &Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r86-refmode", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("tagged `( &Some )`")
            && err.contains("eliminates the ungrounded `Option`")
            && err.contains("not yet supported"),
        "expected a located rejection naming the tag and the enum, got: {err}"
    );
}

/// R8.6/R2.4: a generic eliminator call from a body the **concrete** checker
/// walks. The registry keys the header, so the call name resolves; what
/// cannot is a scrutinee, since a concrete body's operand is always some
/// monomorph and this header has none. Its own message, not the adjacency one
/// (the arms here are written correctly) and not the unknown-word path.
#[test]
fn a_generic_eliminator_in_a_concrete_body_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe ( i64 -- i64 )\n\
           ~[ ( Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : main ( -- ) 7 probe . ;\n"
    );
    let prog = Scratch::write("r86-concrete", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("`Option?` names the generic enum `Option`")
            && err.contains("cannot eliminate it while it is ungrounded")
            && !err.contains("arms are written together"),
        "expected R2.4's own located rejection, got: {err}"
    );
}

/// R8.6/R2.4, the same rejection reached through
/// `check_poly_combinator_standalone`'s `i64` stand-in body: a combinator is
/// checked standalone by the *concrete* checker, so the widened registry's
/// `Generic` entry reaches that consumer here too, and the stand-in has no
/// instantiator to ground a scrutinee with even in principle. R2.2's withdrawn
/// claim -- that P7.S11's combinator path "shares no code with this" -- is
/// what this pins: the name gate is shared code.
#[test]
fn a_generic_eliminator_in_a_standalone_checked_combinator_is_rejected() {
    let src = format!(
        "{OPTION}\
         : probe inline ( 'T ~[ -- i64 ] -- i64 )\n\
           | f | drop\n\
           ~[ ( Some ) drop f call ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : main ( -- ) 7 ~[ 5 ] probe . ;\n"
    );
    let prog = Scratch::write("r86-standalone", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("`Option?` names the generic enum `Option`")
            && err.contains("cannot eliminate it while it is ungrounded")
            && !err.contains("arms are written together"),
        "expected R2.4's own located rejection in the stand-in body, got: {err}"
    );
}

/// `concrete_body_generic_eliminator_error` used to bake a literal `[i64]`
/// into its second line as though it were a fact read off the program, when
/// it is a hardcoded string the format! call never varies -- and its first
/// line claimed "nothing in this program instantiates" the header even when a
/// monomorph plainly exists here, minted by `wrap`'s own output and reachable
/// from `main`'s call site (`eliminator_registry` is built before the poly
/// pre-pass mints it, a separate, out-of-scope timing gap this message must
/// not paper over by asserting something false). The scrutinee's own type
/// parameter is `f64` here, not `i64`, so a resurrected `[i64]` in the
/// message would be visibly and provably wrong rather than coincidentally
/// right -- the `!contains("i64")` assertion below discriminates on this
/// program precisely because `i64` never appears in it. The fix drops the
/// fabricated instantiation and the false nothing-instantiates claim, and
/// states the one thing that is always true: a concrete body cannot
/// eliminate the header while it is ungrounded.
#[test]
fn concrete_body_generic_eliminator_message_does_not_fabricate_an_instantiation() {
    let src = "type: Pair['A] | Nil | One 'A ;\n\
         : wrap ( 'T -- Pair['T] ) One ;\n\
         : main ( -- ) 7.5 wrap ~[ ( One ) One> ] ~[ ( Nil ) 0.0 ] Pair? . ;\n";
    let prog = Scratch::write("r86-poly-output", src);
    let err = build_error(prog.path());
    assert!(
        err.contains("`Pair?` names the generic enum `Pair`")
            && err.contains("cannot eliminate it while it is ungrounded")
            && !err.contains("nothing in this program instantiates")
            && !err.contains("i64")
            && !err.contains("arms are written together"),
        "expected the honest rejection with no fabricated instantiation, got: {err}"
    );
}

/// R1.5, and the limit of what it can be held to today: a generic enum
/// eliminated inside a combinator body would need a per-splice resolution
/// `enum_words`' `Span` key cannot hold, so `poly_eliminator_call` rejects it
/// -- but that gate is unreachable, exactly as its construction twin is
/// (phase 1's own note at `check_module`'s combinator skip). A combinator
/// carrying an `Option['T]` slot is rejected first, by the standing
/// variable-bearing-application restriction, during its standalone check.
/// R1.5 sharpens a message here; it is not a safety net. If that restriction
/// is lifted, this fixture is what should start reporting R1.5's own text.
#[test]
fn a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire() {
    let src = format!(
        "{OPTION}\
         : probe inline ( Option['T] ~[ -- i64 ] -- i64 )\n\
           | f |\n\
           ~[ ( Some ) drop f call ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki ~[ 5 ] probe . ;\n"
    );
    let prog = Scratch::write("r15-eliminate", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("names the generic type `Option['T]`")
            && err.contains("cannot yet be instantiated at a variable-bearing application"),
        "expected the standing variable-bearing restriction, got: {err}"
    );
}

/// R8.2 with a **two-parameter** header, at two monomorphs that are each
/// other's argument swap (`Res[i64 Pt]`, `Res[Pt i64]`). One poly body
/// eliminates both: the variant list comes off the header, and each arm's
/// narrowed input carries the scrutinee's own two arguments (R5.2/R5.4).
#[test]
fn a_two_parameter_generic_enum_is_eliminated_at_swapped_monomorphs() {
    let src = "type: Res['A 'B] | Ok v 'A | Err e 'B ;\n\
         type: Pt x i64 y i64 ;\n\
         : is-ok ( Res['T 'U] -- i64 )\n\
           ~[ ( Ok ) drop 1 ]\n\
           ~[ ( Err ) drop 0 ]\n\
           Res? ;\n\
         : oki ( i64 -- Res[i64 Pt] ) Ok ;\n\
         : errp ( Pt -- Res[i64 Pt] ) Err ;\n\
         : okp ( Pt -- Res[Pt i64] ) Ok ;\n\
         : main ( -- )\n\
           7 oki is-ok .\n\
           1 2 Pt errp is-ok .\n\
           1 2 Pt okp is-ok . ;\n";
    let prog = Scratch::write("r82-two-params", src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n0\n1\n");
}

/// R5.3: `PolyArm.declared_inputs` is `Vec<PolyType>` now, because an arm over
/// an ungrounded scrutinee narrows to a `GenericVariant`, which has no `Type`
/// to intern a real `~[ ... ]` parameter from. An arm written with an ordinary
/// `[ ... ]` bracket is what makes that field observable: the all-`Concrete`
/// case keeps the original interned message, and this case renders the
/// parameter through `poly_type_str` instead.
#[test]
fn an_ordinary_bracket_arm_over_a_generic_scrutinee_names_the_narrowed_parameter() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           [ ( Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r53-bracket", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("declares parameter `~[ Option.Some -- ]` as inline `~[ ... ]`"),
        "expected the narrowed parameter rendered through `poly_type_str`, got: {err}"
    );
}

/// R5.6: `drop` of a narrowed `GenericVariant` inside its arm is accepted (the
/// concrete twin accepts `drop` of a `Type::Variant`) and lowers per
/// instantiation -- here through a `| v |` bind, so the variant is also
/// move-tracked as a local at two payload layouts.
#[test]
fn a_narrowed_variant_binds_and_drops_at_two_instantiations() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Some ) | v | v drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : mkp ( Pt -- Option[Pt] ) Some ;\n\
         : main ( -- )\n\
           7 mki probe .\n\
           1 2 Pt mkp probe . ;\n"
    );
    let prog = Scratch::write("r56-bind-drop", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n1\n");
}

// ---------------------------------------------------------------------------
// Phase 4 (R6 + R7): the destructure intercept, and the diagnostic split.
// ---------------------------------------------------------------------------

/// R8.3: `unwrap-or`-shaped -- a poly word whose `Some` arm destructures the
/// generic payload with `Some>` and returns it, at two instantiations whose
/// payload layouts differ. This is R6's only end-to-end witness: nothing else
/// in the suite reads a field out of a still-ungrounded `GenericVariant`.
#[test]
fn unwrap_or_destructures_the_payload_at_two_instantiations() {
    let src = format!(
        "{OPTION}\
         : unwrap-or ( 'T Option['T] -- 'T )\n\
           ~[ ( Some ) Some> swap drop ]\n\
           ~[ ( None ) None> ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : mkp ( Pt -- Option[Pt] ) Some ;\n\
         : nonei ( -- Option[i64] ) None ;\n\
         : default-pt ( -- Pt ) 0 0 Pt ;\n\
         : main ( -- )\n\
           0 7 mki unwrap-or .\n\
           9 nonei unwrap-or .\n\
           default-pt 1 2 Pt mkp unwrap-or Pt> drop . ;\n"
    );
    let prog = Scratch::write("r83-unwrap-or", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n9\n1\n");
}

/// R8.4: a poly word that both *constructs* and *eliminates* `Option['T]`,
/// at two instantiations -- unlike phase 1's B3, this is in scope now: R1.2
/// is what makes the construction call inside this same body lowerable, and
/// R6 is what makes the destructure inside the arm it flows into lowerable.
#[test]
fn a_poly_word_constructs_and_eliminates_the_same_generic_enum_at_two_instantiations() {
    let src = format!(
        "{OPTION}\
         : wrap-and-check ( 'T -- i64 )\n\
           Some\n\
           ~[ ( Some ) Some> drop 1 ]\n\
           ~[ ( None ) None> 0 ]\n\
           Option? ;\n\
         : main ( -- )\n\
           1 2 Pt wrap-and-check .\n\
           7 wrap-and-check . ;\n"
    );
    let prog = Scratch::write("r84-construct-eliminate", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n1\n");
}

/// R8.5: `~[ ( None ) None> .. ]` over a generic scrutinee, at two
/// instantiations (R6.3) -- previously unwitnessed. `None` carries no fields,
/// so `None>` destructures to nothing; the real assertion is that the call is
/// accepted and lowers at both payload layouts, `is-none` returning `1` for
/// a `None` scrutinee and `0` for its own `Some` arm.
///
/// Two separate builds, not one program: a *concrete* zero-arity constructor
/// shared bare across two monomorphs of one header (an explicit
/// `Option[Pt]`-returning `None` caller in a build that also mints
/// `Option[i64]` resolves to whichever monomorph minted first, regardless of
/// declaration order) is a pre-existing collision, reproducing identically
/// before this phase's own changes and entirely outside R6's scope -- it is
/// the *construction* side of a zero-field variant across concrete
/// monomorphs, not a poly body's destructure of one. Each build below mints
/// exactly one `Option` instantiation, so that collision never triggers.
#[test]
fn a_zero_field_variant_destructures_to_nothing_at_the_i64_instantiation() {
    let src = format!(
        "{OPTION}\
         : is-none ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 0 ]\n\
           ~[ ( None ) None> 1 ]\n\
           Option? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : nonei ( -- Option[i64] ) None ;\n\
         : main ( -- )\n\
           7 mki is-none .\n\
           nonei is-none . ;\n"
    );
    let prog = Scratch::write("r85-zero-field-i64", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "0\n1\n");
}

/// R8.5, the second instantiation: a struct payload, in its own build for the
/// reason documented above.
#[test]
fn a_zero_field_variant_destructures_to_nothing_at_the_struct_instantiation() {
    let src = format!(
        "{OPTION}\
         : is-none ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 0 ]\n\
           ~[ ( None ) None> 1 ]\n\
           Option? ;\n\
         : mkp ( Pt -- Option[Pt] ) Some ;\n\
         : nonep ( -- Option[Pt] ) None ;\n\
         : main ( -- )\n\
           1 2 Pt mkp is-none .\n\
           nonep is-none . ;\n"
    );
    let prog = Scratch::write("r85-zero-field-pt", &src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "0\n1\n");
}

/// R8.6/R7.2: the remaining R8.6 case -- a genuinely absent eliminator (a
/// typo'd `Optionn?`). The arms are written correctly, immediately before the
/// call, so this must not read as an adjacency mistake: R7.3's own
/// requirement, that a witness for this message be a real typo, not an
/// unrelated call like `drop`/`swap` (which stay the adjacency message, R8.7).
#[test]
fn a_typod_eliminator_call_names_no_eliminator_rather_than_an_adjacency_mistake() {
    let src = format!(
        "{OPTION}\
         : probe ( Option['T] -- i64 )\n\
           ~[ ( Some ) drop 1 ]\n\
           ~[ ( None ) drop 0 ]\n\
           Optionn? ;\n\
         : mki ( i64 -- Option[i64] ) Some ;\n\
         : main ( -- ) 7 mki probe . ;\n"
    );
    let prog = Scratch::write("r72-typo", &src);
    let err = build_error(prog.path());
    assert!(
        err.contains("the call `Optionn?` it is adjacent to names no eliminator in scope"),
        "expected R7.2's own located rejection, got: {err}"
    );
}

/// R7.2's other gate: the *concrete* body arm in `check_term`'s
/// `TermKind::Quotation` handling (`f` below is monomorphic, no `'T`
/// anywhere), which is a separate call site from `poly_walk`'s and was
/// otherwise unwitnessed by the suite.
#[test]
fn a_typod_eliminator_call_in_a_concrete_body_names_no_eliminator_rather_than_an_adjacency_mistake()
{
    let src = "type: Shape | Circle r f64 | Rect w f64 h f64 ;\n\
         : f ( Shape -- i64 )\n\
           ~[ ( Circle ) drop 1 ]\n\
           ~[ ( Rect ) drop drop 0 ]\n\
           Shapee? ;\n\
         : main ( -- ) 2.0 Circle f . ;\n";
    let prog = Scratch::write("r72-typo-concrete", src);
    let err = build_error(prog.path());
    assert!(
        err.contains("the call `Shapee?` it is adjacent to names no eliminator in scope"),
        "expected R7.2's own located rejection from the concrete-body arm, got: {err}"
    );
}

/// R6.1 ("fields push in declared order, first field deepest") and R8.3's
/// positional half, neither previously witnessed: `Option['T]` has only one
/// type parameter, so R8.3's own two-instantiation tests can't tell a
/// positional mixup from an identity. `Two['A 'B]` has two, at a
/// two-*field* variant, so `take-a`/`take-b` each read a different field at
/// a different type: reversing either the field push order or the narrowed
/// scrutinee's own argument list swaps which value each returns.
#[test]
fn a_two_field_variant_destructures_fields_in_declared_order_and_type() {
    let src = "type: Two['A 'B] | Both fst 'A snd 'B ;\n\
         type: Pt x i64 y i64 ;\n\
         : take-a ( Two['A 'B] -- 'A ) ~[ ( Both ) Both> drop ] Two? ;\n\
         : take-b ( Two['A 'B] -- 'B ) ~[ ( Both ) Both> swap drop ] Two? ;\n\
         : mk1 ( -- Two[i64 Pt] ) 7 1 2 Pt Both ;\n\
         : mk2 ( -- Two[i64 Pt] ) 7 1 2 Pt Both ;\n\
         : main ( -- )\n  \
           mk1 take-a .\n  \
           mk2 take-b Pt> swap drop . ;\n";
    let prog = Scratch::write("r61-r83-two-field", src);
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n2\n");
}
