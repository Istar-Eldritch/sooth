//! P7b.S15 exit goldens: the Applicative.pure surface. Phase 1 started the
//! file with the three fully-measured goldens: the true-shape member
//! `pure ( 'A -- 'F['A] )` declares and its ctor-keyed impl checks (G1,
//! R-30.1), the `ap` parse fence holds (G9, R-30.5), and the zero-input
//! escape-hatch impl mismatch keeps its recorded-not-fixed bytes (G12,
//! R-30.6). Phase 2 adds the call-site goldens (G6/G8/G11.a-c/G13/G14): the
//! R-30.2 output-App route builds and runs `pure[Box[i64]]` at a mono call
//! with no `'F` operand (G14), the bare-call remedy's example becomes
//! achievable (G6, R-30.3), conservative supplies keep their measured bytes
//! (G8/G13), and the twin-class walls stay byte-identical (G11.a-c, R-30.4).
//! Phase 3 adds the lib goldens (G2-G5, G7): the `Applicative` trait ships
//! in `lib/core` (REQ-30.11) with co-located per-ctor impls (REQ-30.12), so
//! a consumer's `5 pure[Option[i64]]`-class call dispatches the real
//! constructors (G2-G4 print `5\n`), one shared-bound definition re-pures
//! two constructors (G5 prints `7\n7\n`, REQ-30.13), and the bare-ctor
//! instantiation fence stays word-general on the lib spelling (G7,
//! REQ-30.14). Driven through the real
//! `sooth` binary, harness helpers copied from `tests/phase7b_slice2.rs`;
//! error goldens keep the minimal two-line prefix so their line/column
//! assertions stay readable against the fixture.

// Each helper carries its own `#[allow(dead_code)]` rather than the module
// taking a blanket one: a phase may use only a subset, but a helper nothing
// uses at all should still be reported. (Convention from tests/common/mod.rs.)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs15-{}-{tag}-{seq}", std::process::id()));
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

/// The printing-golden runner, from `tests/phase7b_slice2.rs`: builds, runs
/// the binary, asserts exit 0, and returns the exact stdout. Phase 2's route
/// goldens (G13/G14) use it; Phase 1's three goldens do not.
#[allow(dead_code)]
fn build_and_run(entry: &Path) -> String {
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

/// The hosted twin of `single_file`, from `tests/phase7b_slice2.rs`: adds
/// the hosted manifest (a bare package cannot import `core`) and the
/// selective `hosted::show | . |` import (P7.S7d retired the `.` intrinsic
/// onto `hosted::show`'s ordinary word, so a printing fixture needs it --
/// which is why the probe-fixture goldens G6/G8/G13/G14 ride this harness
/// rather than `single_file`, matching the probe fixtures' own imports).
/// Phase 2/3's lib goldens (G2-G5, G7) use it; Phase 1's goldens do not.
#[allow(dead_code)]
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs15 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// Golden (G1, REQ-30.1): the true-shape member `pure ( 'A -- 'F['A] )`
/// declares under the R-30.1 output arm, and its ctor-keyed impl
/// (`impl: Applicative for Box : pure MkBox ;`) checks clean -- the impl
/// body's `Box['A]` output unifies against the dissolved declared output
/// through the input var, so the P2e identical-renderings wall does not
/// fire on the true shape (r2a; root-caused r2e). Bytes: the r2 baseline's
/// `soo30r2_a_decl_impl.sth` receipt -- build clean, no stderr, exit 0.
/// Fixture: the r2a probe's shape verbatim (the harness prepends the
/// intrinsics import, keeping the probe's line numbering).
#[test]
fn true_shape_member_declares_and_ctor_impl_checks() {
    let src = "\
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 drop ;
";
    let (_t, entry) = single_file("s15-g1-true-shape-decl-impl", src);
    build_ok(&entry);
}

/// Golden (G9, REQ-30.5): `ap` stays undeclarable -- the S1-6 parse fence
/// fires before any checking, so the shipped trait can declare `pure` only
/// and the impl-coverage question stays moot behind the fence (r1 P5). The
/// slice touches no parser code, so the measured bytes are expected
/// unchanged; the assert pins them anyway (diagnostics are behaviour).
/// Fixture: `probes/soo30_p5_ap_in_trait.sth` verbatim minus the intrinsics
/// import (the harness prepends it, keeping the probe's line numbering).
#[test]
fn ap_declaration_stays_parse_fenced() {
    let src = "\
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
  : ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 drop ;
";
    let (_t, entry) = single_file("s15-g9-ap-fence", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: expected a type, found `[` at line 5, col 14 (a type application's \
             arguments are types, not quotations)"
        ),
        "{err}"
    );
}

/// Golden (G12, REQ-30.4): the zero-input escape hatch's impl mismatch stays
/// byte-identical -- recorded-not-fixed per R-30.6 (r2e root cause:
/// `check_poly_body`'s residual comparison is syntactic `PolyType`
/// `PartialEq`, and the body's minted `Concrete(Box[i64])` vs the declared
/// `Generic{Box, [i64]}` render identically but compare unequal). The
/// zero-input member bypasses the gate via the empty-input arm even
/// pre-relaxation, so these are today-stable bytes the slice must not move.
/// Fixture: `probes/soo30_p2e_applied_instantiation_zero_input.sth` verbatim
/// minus the intrinsics import (the harness prepends it).
#[test]
fn zero_input_escape_hatch_still_fails_impl_check() {
    let src = "\
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( -- 'F[i64] ) ;
;
impl: Applicative for Box
  : pure 42 MkBox ;
;
: main ( -- ) pure[Box[i64]] drop ;
";
    let (_t, entry) = single_file("s15-g12-zero-input-defect", src);
    let err = build_error(&entry);
    assert!(
        err.contains("error: stack effect mismatch in `pure;Applicative;0;Box['T0]`"),
        "{err}"
    );
    assert!(
        err.contains("body leaves `Box[i64]`, but the declared outputs are `Box[i64]`"),
        "{err}"
    );
}

/// Golden (G6, REQ-30.8): the bare-call remedy keeps line 1 byte-identical
/// (r2b-2's template, located at this fixture's own line/col) and its example
/// becomes the achievable output-App spelling `pure[Box[i64]]` (R-30.3) --
/// achievable because G14 proves that exact spelling builds and dispatches.
/// The r2b-2 example `pure[i64]` reproduced the arity wall the route removes;
/// its absence is asserted too. Bytes: `probes/soo30r3_baseline.md` § g6
/// (measured under the round-3 probe). Fixture: `probes/soo30r3_g6.sth` minus
/// its two import lines (the harness prepends both; the probe's comment lines
/// stay, keeping main's span at line 13, col 18 -- the measured bytes).
#[test]
fn bare_call_remedy_example_is_the_achievable_output_app_spelling() {
    let src = "\\ SOO-30 R3-G6 -- the bare-call remedy error with the CORRECTED (output-App)\n\
        \\ example. Line 1 must stay byte-identical to r2b-2's; the example line must\n\
        \\ now name the `pure[Box[i64]]`-shape (R-30.3).\n\
        type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : main ( -- ) 42 pure drop ;\n";
    let (_t, entry) = single_file_hosted("s15-g6-remedy-example", src);
    let err = build_error(&entry);
    assert!(
        err.contains("error: `pure` in `main` (line 13, col 18) is a trait member with no operand to dispatch on"),
        "{err}"
    );
    assert!(
        err.contains(
            "a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `pure[Box[i64]]`"
        ),
        "{err}"
    );
    assert!(
        !err.contains("pure[i64]"),
        "the old unachievable example is gone (and `pure[Box[i64]]` never contains it): {err}"
    );
}

/// Golden (G8, REQ-30.7): a 2-argument POSITIONAL supply is not the
/// single-argument output-App form, so the route never fires -- impl
/// selection stays operand-driven, `find_bound_impl(i64)` finds no impl, and
/// the no-dispatch diagnostic keeps its r2b-1c bytes (R-30.2's conservative
/// face). Bytes: `probes/soo30r3_baseline.md` § g8. Fixture:
/// `probes/soo30r3_g8.sth` minus its two import lines (the harness prepends
/// both, keeping the measured span).
#[test]
fn two_arg_positional_supply_stays_operand_driven_no_dispatch_error() {
    let src = "\\ SOO-30 R3-G8 -- conservative face of R-30.2: a 2-argument POSITIONAL supply\n\
        \\ keeps operand-driven selection; the route must NOT fire here. Expected:\n\
        \\ r2b-1c's no-dispatch bytes, unchanged.\n\
        type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : main ( -- ) 42 pure[i64 i64] drop ;\n";
    let (_t, entry) = single_file_hosted("s15-g8-positional-supply", src);
    let err = build_error(&entry);
    assert!(
        err.contains("error: `pure` in `main` (line 13, col 18) is a trait member of Applicative, but no `impl:` in this program dispatches on these operands"),
        "{err}"
    );
    assert!(
        err.contains(
            "the operand types here are `i64`; declare an impl of one of those traits for the operand's type, or import a word that claims this name"
        ),
        "{err}"
    );
}

/// Golden (G11.a, REQ-30.9): the twin class's arity wall stays
/// byte-identical -- a plain word's output-only bound var is NOT a member
/// call, so R-30.2's route never touches it and the dissolved word's
/// positional arity gate keeps its r2d bytes. Bytes:
/// `probes/soo30r2_baseline.md` § soo30r2_d_twin. Fixture:
/// `probes/soo30r2_d_twin.sth` minus the intrinsics import (the harness
/// prepends it, keeping main at line 10 -- the measured span).
#[test]
fn plain_word_output_only_var_single_arg_supply_is_arity_error() {
    let src = "type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : twin['F: Applicative 'A] ( 'A 'A -- 'F['A] 'F['A] ) pure swap pure ;\n\
        : main ( -- ) 1 2 twin[Box[i64]] drop drop ;\n";
    let (_t, entry) = single_file("s15-g11a-twin-arity", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: `twin` (line 10) declares 2 type variables (`'A`, `'F`) but was given 1 type argument"
        ),
        "{err}"
    );
}

/// Golden (G11.b, REQ-30.9): the twin class's named wall stays
/// byte-identical -- a bare call on a plain word whose output variable no
/// input binds keeps `poly_unbound_output_error`'s r2d bytes. Bytes:
/// `probes/soo30r2_baseline.md` § soo30r2_d2_twin_bare. Fixture:
/// `probes/soo30r2_d2_twin_bare.sth` minus the intrinsics import (the
/// harness prepends it, keeping main at line 10 -- the measured span).
#[test]
fn plain_word_output_only_var_bare_call_is_named_unbound_output_error() {
    let src = "type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : twin['F: Applicative 'A] ( 'A 'A -- 'F['A] 'F['A] ) pure swap pure ;\n\
        : main ( -- ) 1 2 twin drop drop ;\n";
    let (_t, entry) = single_file("s15-g11b-twin-bare", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: `twin` in `main` (line 10) has output variable `'F` that no input binds"
        ),
        "{err}"
    );
    assert!(
        err.contains("note: supply it explicitly: `twin[SomeType SomeType]`"),
        "{err}"
    );
}

/// Golden (G11.c, REQ-30.9): the twin class's unachievable-HKT-remedy wall
/// stays byte-identical -- naming a `* -> *` var's position with a bare ctor
/// head (`twin[i64 Box]`) is parse-refused with the r2d bytes, so the
/// unbound-output remedy is unachievable exactly as measured. Bytes:
/// `probes/soo30r2_baseline.md` § soo30r2_d3_twin_hkt_arg. Fixture:
/// `probes/soo30r2_d3_twin_hkt_arg.sth` minus the intrinsics import (the
/// harness prepends it, keeping main at line 10 -- the measured span).
#[test]
fn plain_word_output_only_var_hkt_remedy_spelling_stays_parse_refused() {
    let src = "type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : twin['F: Applicative 'A] ( 'A 'A -- 'F['A] 'F['A] ) pure swap pure ;\n\
        : main ( -- ) 1 2 twin[i64 Box] drop drop ;\n";
    let (_t, entry) = single_file("s15-g11c-twin-hkt-arg", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: generic type `Box` declares 1 type variable, but none were supplied at line 10, col 28 (apply it as `Box[T]`, one type argument per declared variable)"
        ),
        "{err}"
    );
}

/// Golden (G13, REQ-30.7): the double-wrap operand idiom -- the ONLY working
/// mono route before this slice -- survives the route byte-identically (build
/// clean, run `42\n`): the operand carries the target head, the 2-arg
/// positional supply rides the ordinary S8b seed channel positionally (arity
/// 2 == 2 vars), and the extended-seed route is single-argument only.
/// Bytes: `probes/soo30r3_baseline.md` § g13 (== r2b-3b). Fixture:
/// `probes/soo30r3_g13.sth` minus its two import lines (the harness prepends
/// both).
#[test]
fn double_wrap_operand_idiom_survives_the_route() {
    let src = "\\ SOO-30 R3-G13 -- non-leakage: the double-wrap operand idiom (r2b-3b, the ONLY\n\
        \\ working mono route before round 3) must survive byte-identically. The\n\
        \\ operand carries the target head; the 2-arg positional supply rides the\n\
        \\ ordinary S8b seed channel, not the new route.\n\
        type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : showbox ( Box[i64] -- ) ~[ ( MkBox ) MkBox> . ] Box? ;\n\
        : showbox2 ( Box[Box[i64]] -- ) ~[ ( MkBox ) MkBox> showbox ] Box? ;\n\
        : main ( -- ) 42 MkBox pure[Box[i64] Box[i64]] showbox2 ;\n";
    let (_t, entry) = single_file_hosted("s15-g13-double-wrap", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "42\n");
}

/// Golden (G14, REQ-30.6): THE primary target. `42 pure[Box[i64]]` at a mono
/// call site with no `'F` operand builds and prints `42\n` -- the single
/// explicit instantiation IS the output-App instantiation: impl selection
/// keys on the dissolved ctor head, the seed extension binds the member's
/// residual `'A` from the App's argument, the arity gate exempts the routed
/// supply, and dispatch lowers end-to-end. Fires only on the explicit
/// instantiation (no consuming-context inference, the S6 Q1 rule). Bytes:
/// `probes/soo30r3_baseline.md` § g14 (measured under the round-3 probe).
/// Fixture: `probes/soo30r3_g14.sth` minus its two import lines (the harness
/// prepends both).
#[test]
fn output_app_instantiation_prints_at_mono_call_without_operand() {
    let src = "\\ SOO-30 R3-G14 -- the R-30.2 output-App route (fixture-local Box): a single\n\
        \\ explicit instantiation `pure[Box[i64]]` grounds the return type at a mono\n\
        \\ call site with no 'F operand. THE primary round-3 measurement.\n\
        type: Box['A] | MkBox 'A ;\n\
        trait: Applicative['F: * -> *]\n\
          : pure ( 'A -- 'F['A] ) ;\n\
        ;\n\
        impl: Applicative for Box\n\
          : pure MkBox ;\n\
        ;\n\
        : showbox ( Box[i64] -- ) ~[ ( MkBox ) MkBox> . ] Box? ;\n\
        : main ( -- ) 42 pure[Box[i64]] showbox ;\n";
    let (_t, entry) = single_file_hosted("s15-g14-output-app-route", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "42\n");
}

/// Golden (G2, REQ-30.12 via REQ-30.6's route): the R-30.2 spelling on the
/// real lib Option -- the ticket's central construction surface. `5
/// pure[Option[i64]]` dissolves `'F:=Option, 'A:=i64`, keys impl selection
/// on the dissolved ctor head, and dispatches the shipped `impl: Applicative
/// for Option : pure Some ;` end-to-end, producing `Some(5)`; `showopt`
/// prints the inner `5`. Bytes: `probes/soo30r3_baseline.md` § g2 (measured
/// under the round-3 probe, then re-verified against the real lib at this
/// phase's start). Fixture: `probes/soo30r3_g2.sth` minus its two import
/// lines (the harness prepends both; duplicate imports collide) -- the
/// probe's comment header kept, code lines at column 0 (leading fixture
/// whitespace would shift spans; none are asserted here).
#[test]
fn option_ctor_constructs_through_the_output_app_instantiation() {
    let src = "\
\\ SOO-30 R3-G2 -- lib Option through the R-30.2 route: `5 pure[Option[i64]]`
\\ dispatches the real Some ctor.
import: core::option * ;
import: core::applicative | Applicative | ;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop 0 . ] Option? ;
: main ( -- ) 5 pure[Option[i64]] showopt ;
";
    let (_t, entry) = single_file_hosted("s15-g2-option-pure", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "5\n");
}

/// Golden (G3, REQ-30.12 via REQ-30.6's route): the 2-param ctor through the
/// same 1-arg spelling -- the "partially-applied ctor heads ride along"
/// case. `5 pure[Result[i64 i64]]` fills `'F:=Result`, the seed binds both
/// ctor params from the instantiation's ctor arguments, and the residual
/// `'A` binds from the output App's single argument (r3 findings § g3).
/// Bytes: `probes/soo30r3_baseline.md` § g3. Fixture:
/// `probes/soo30r3_g3.sth` minus its two import lines.
#[test]
fn result_two_arity_ctor_constructs_with_partial_head() {
    let src = "\
\\ SOO-30 R3-G3 -- lib Result (2 params) through the R-30.2 route: the
\\ 2-arity spelling `pure[Result[i64 i64]]` (partial-head ride-along, P3b
\\ receipt class).
import: core::result * ;
import: core::applicative | Applicative | ;
: showres ( Result[i64 i64] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) drop 1 . ] Result? ;
: main ( -- ) 5 pure[Result[i64 i64]] showres ;
";
    let (_t, entry) = single_file_hosted("s15-g3-result-pure", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "5\n");
}

/// Golden (G4, REQ-30.12 via REQ-30.6's route): the allocating ctor --
/// `pure` must produce a genuine `Cons` cell, not a wrapper. The shipped
/// List impl body `Nil ^ Cons` boxes the payload with the owned-cell `^`
/// (`check_owned_cell_word`, src/check/word_families.rs:1153); `showlist`
/// walks the real cell. Unlike the r2c operand-path fixture, no
/// `nile`/`single` helpers are needed: `pure` constructs the list, so
/// `Nil`'s consumer is the impl body, whose dissolved declared output pins
/// it (r2c's impl-check receipt). Bytes: `probes/soo30r3_baseline.md` § g4.
/// Fixture: `probes/soo30r3_g4.sth` minus its two import lines.
#[test]
fn list_ctor_constructs_a_real_cons_cell() {
    let src = "\
\\ SOO-30 R3-G4 -- lib List through the R-30.2 route: `5 pure[List[i64]]`
\\ produces a real Cons cell (the cell-boxing impl body `Nil ^ Cons`),
\\ walked by showlist.
import: core::list * ;
import: core::applicative | Applicative | ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- ) 5 pure[List[i64]] showlist ;
";
    let (_t, entry) = single_file_hosted("s15-g4-list-pure", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "5\n");
}

/// Golden (G5, REQ-30.13): the ticket's "declared on a shared `Applicative`
/// bound ... dispatches per constructor" surface -- ONE poly definition,
/// called BARE at two mono sites with different ctor operands (both vars
/// operand-grounded, no explicit instantiation; the R-30.4 consumer shape
/// with `'F` in an input). The body is the paper's corrected `swap drop
/// pure` (the ticket sketch's `swap pure` would feed `pure` the old
/// container); the `7` (not the operands' `5`) makes dispatch observable:
/// each call drops its ctor operand and re-pures the value through the SAME
/// definition. Bytes: `probes/soo30r3_baseline.md` § g5 (`7\n7\n`; the
/// baseline also records an intermediate probe-authoring bug -- an
/// unconsumed literal before `nile` -- as a main stack-effect mismatch, not
/// a checker wall). Fixture: `probes/soo30r3_g5.sth` minus its two import
/// lines.
#[test]
fn shared_bound_consumer_dispatches_two_ctors_through_one_definition() {
    let src = "\
\\ SOO-30 R3-G5 -- the shared-bound consumer: `repure['F: Applicative 'A]
\\ ( 'F['A] 'A -- 'F['A] ) swap drop pure ;` (the paper's corrected body),
\\ called at two mono sites with Option and List operands.
import: core::option * ;
import: core::list * ;
import: core::applicative | Applicative | ;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop 0 . ] Option? ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
\\ pins Nil to List[i64] so the operand construction is unambiguous (r2c
\\ idiom)
: nile ( -- List[i64] ) Nil ;
: repure['F: Applicative 'A] ( 'F['A] 'A -- 'F['A] ) swap drop pure ;
: main ( -- ) 5 Some 7 repure showopt nile 7 repure showlist ;
";
    let (_t, entry) = single_file_hosted("s15-g5-shared-bound-repure", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "7\n7\n");
}

/// Golden (G7, REQ-30.14): the bare-ctor instantiation fence stays
/// word-general on the lib spelling -- `pure[Option]` is parse-refused
/// before any checking, so the R-30.2 landing does not weaken the S1 fence
/// (the fence precedes module checking, so the shipped core::applicative
/// cannot change it; r1 P3 proved parse precedes the declaration gate the
/// same way). Bytes: the paper's G7 layout measurement, re-measured live at
/// this phase's start under the exact hosted-harness layout (line 5, col 22
/// -- no span shift; both lines + exit 1). Fixture: the paper's G7 text
/// verbatim minus its two import lines (the harness prepends both); no
/// comment header, so main sits at the measured line 5.
#[test]
fn bare_ctor_instantiation_argument_stays_parse_refused() {
    let src = "\
import: core::option * ;
import: core::applicative | Applicative | ;
: main ( -- ) 5 pure[Option] drop ;
";
    let (_t, entry) = single_file_hosted("s15-g7-bare-ctor-fence", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: generic type `Option` declares 1 type variable, but none were supplied at line 5, col 22 (apply it as `Option[T]`, one type argument per declared variable)"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal"
        ),
        "{err}"
    );
}
