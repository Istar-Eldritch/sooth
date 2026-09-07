//! P7b.S8b exit goldens. Phase 1: the pre-existing two-defect pair the
//! construction-wall lift exposes -- (a) a nullary trait member over a
//! *generic* impl target seeded its monomorph's θ from the call site's type
//! argument positionally, minting one level too deep (`Opt[Opt[i64]]`), and
//! (b) a checker-resolved enum construction in a mono body recorded nothing,
//! so lowering read it from the bare-name variant map that a later
//! instantiation's mint had already clobbered. Together they SIGSEGV any
//! program pairing a payload construction with an `empty[<inst>]` call.
//! Harness style from `tests/phase7b_slice8.rs`/`tests/phase7b_slice6.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs8b-{}-{tag}-{seq}", std::process::id()));
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

/// `tests/phase7b_slice8.rs:39`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs8b ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

/// Build `src`, run the produced binary, and return its stdout. The exit
/// status is asserted, not just the build: every golden here guards against a
/// *silent* miscompile, so "it built" proves nothing -- the pre-fix failure
/// mode is exit 139.
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
    assert_eq!(
        run.status.code(),
        Some(0),
        "the built binary should exit 0 (a SIGSEGV is code None/139), stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).expect("stdout should be utf8")
}

/// Build `src`, assert it fails with a non-zero exit and no panic, and
/// return that stderr -- callers assert the exact `error:` text themselves.
fn build_error_located(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    let stderr = String::from_utf8(build.stderr).expect("stderr should be utf8");
    assert!(
        !stderr.contains("panicked"),
        "a rejected shape must be a located error, never a panic, got: {stderr}"
    );
    stderr
}

/// The Opt-shaped base repro, verbatim from the probe round's
/// `p8bf-opt-crash.sth` (`docs/roadmap/P7b/slice8b-probes.md`, Step 5): a
/// generic-target `Monoid` impl over a header with **no** self-reference
/// field, a payload construction that runs before anything else, then
/// `empty[Opt[i64]]`. No `List`, so the S6 construction wall is not involved
/// -- this is the two-defect pair on its own.
const OPT_REPRO_SRC: &str = "\
type: Opt['T] | None | Some 'T ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for Opt
  : empty None ;
  : combine drop ;
;
: mkopt ( i64 -- Opt[i64] ) Some ;
: main ( -- )
  1 mkopt drop
  empty[Opt[i64]] drop \"ok\" . ;
";

/// (R6) The arbitrator. At base this builds clean and then SIGSEGVs at
/// `sooth_main+73`, inside the `Some` construction -- code that runs *before*
/// the `empty` call ever executes, because the damage is done at compile time:
/// the wrong-θ mint's variant words re-typed every `Some` in the program to
/// `Opt[Opt[i64]]`'s field shapes, so an 8-byte i64 operand is blitted as a
/// 24-byte payload and address `1` gets dereferenced.
#[test]
fn nullary_member_over_a_generic_impl_target_runs_after_a_prior_construction() {
    assert_eq!(build_and_run("opt-repro", OPT_REPRO_SRC), "ok");
}

/// (R4) The θ pin, at the one place the wrong instantiation is visible without
/// running anything: the emitted monomorph's own QBE type. `empty[Opt[i64]]`
/// must return `Opt[i64]` (`:Opt.5b.i64.5d.`), never the one-level-too-deep
/// `Opt[Opt[i64]]` (`:Opt.5b.Opt.5b.i64.5d..5d.`) the positional seeding
/// minted. Captured through `driver::emit_ssa_with_manifest`
/// (pattern: `tests/phase7b_slice8.rs:742`) so `src/ir/` stays diff-empty.
///
/// The fixture drops the repro's prior construction: the mis-mint does not
/// need it (only the *crash* does), so pinning the symbol here separates the
/// θ defect from the variant-map defect the golden above conflates.
#[test]
fn nullary_member_over_a_generic_impl_target_mints_the_one_level_monomorph() {
    let src = "\
import: intrinsics * ;
import: hosted::show | . | ;
type: Opt['T] | None | Some 'T ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for Opt
  : empty None ;
  : combine drop ;
;
: mkopt ( i64 -- Opt[i64] ) Some ;
: main ( -- )
  empty[Opt[i64]] drop \"ok\" . ;
";
    let path = std::env::temp_dir().join(format!("sooth-p7bs8b-ssa-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing the fixture should succeed");
    let ssa = sooth::driver::emit_ssa_with_manifest(&path, common::manifest_for(&path).as_deref())
        .unwrap_or_else(|e| panic!("emitting the fixture should succeed: {e}"));
    std::fs::remove_file(&path).ok();

    let mono: Vec<&str> = ssa
        .lines()
        .filter(|l| l.contains("$sooth_mono_empty_Monoid"))
        .collect();
    assert_eq!(
        mono.len(),
        2,
        "expected the monomorph's definition and its one call site: {mono:?}"
    );
    for line in &mono {
        assert!(
            line.contains(":Opt.5b.i64.5d."),
            "the `empty` monomorph is typed `Opt[i64]`: {line}"
        );
        assert!(
            !line.contains(":Opt.5b.Opt.5b.i64.5d..5d."),
            "never the positionally-seeded `Opt[Opt[i64]]`: {line}"
        );
    }
    assert!(
        !ssa.contains("Opt.5b.Opt.5b.i64.5d..5d."),
        "no `Opt[Opt[i64]]` instantiation is minted anywhere in the program"
    );
}

/// (R5) The two-mint golden: `mk`'s `Some` is resolved by bare name in a mono
/// body while `Opt[i64]` is the only instantiation the frozen `env` holds, and
/// `Opt[Opt[i64]]` is minted *later*, mid-check, by inferring `wrap`'s `'T`
/// from a `mk` result. Lowering's variant map keys every variant under its
/// bare surface spelling too, last-write-wins across instantiations, so
/// without the per-site record `mk` lowers with `Opt[Opt[i64]]`'s 24-byte
/// payload shape and `unwrap` reads garbage back (in fact: SIGSEGV).
///
/// `Opt` rather than `List`: the second mint has to come from *inference*
/// (an explicit `empty[Opt[Opt[i64]]]`-style type argument mints eagerly at
/// parse time, which makes the site multi-candidate and so already recorded),
/// and the only inference route for a second `List` instantiation is a
/// polymorphic body reconstructing `Cons` -- Phase 2's construction wall.
#[test]
fn bare_variant_construction_keeps_its_shape_when_a_later_instantiation_is_minted() {
    let src = "\
type: Opt['T] | None | Some 'T ;
: mk ( i64 -- Opt[i64] ) Some ;
: unwrap ( Opt[i64] -- i64 )
  ~[ ( None ) drop 0 ]
  ~[ ( Some ) Some> ]
  Opt? ;
: wrap['T] ( 'T -- Opt['T] ) Some ;
: main ( -- )
  7 mk unwrap .
  9 mk wrap drop ;
";
    assert_eq!(build_and_run("two-mint", src), "7\n");
}

/// (R4, byte-unchanged pin) The concrete-target nullary path -- S6's own
/// `empty[i64]` shape -- takes `resolve_mono_member_call`'s mono branch
/// (`ground_member_type`), never the seeding path, so the new channel must
/// leave it exactly as it was. Local twin of
/// `monoid_for_i64_combine_and_empty_dispatch` (`tests/phase7b_slice6.rs`),
/// kept here so a Phase-1 regression is attributed to Phase 1.
#[test]
fn nullary_member_over_a_concrete_impl_target_is_unchanged() {
    let src = "\
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for i64
  : empty 0 ;
  : combine add ;
;
: main ( -- )
  4 5 combine .
  empty[i64] . ;
";
    assert_eq!(build_and_run("concrete-nullary", src), "9\n0\n");
}

/// (Review round, P1-2/P2-6 risk row) A mono `inline` (combinator) word's
/// body is checked as an *ordinary mono word* with the real maps
/// (`check.rs:1036-1053`; the separate standalone combinator walk at
/// `check.rs:992-1032` uses scratch maps and records nothing) even
/// when it is never spliced anywhere in this program -- so R5's new record
/// (`terms.rs:962-985`) can land the construction span in `builtin_overloads`
/// via that standalone walk, the same channel a splice's redirect writes to.
/// This pins the current dispatch outcome for that case rather than fixing
/// it ("New-in-slice caveat", `docs/roadmap/P7b/slice8b-spec.md`): `mkopt`
/// constructs a variant and is never spliced
/// (`main` calls it as an ordinary word, not inline-expanded at a call site
/// shaped like a combinator), so the build+run must still succeed.
#[test]
fn mono_inline_combinator_variant_construction_builds_and_runs() {
    let src = "\
type: Opt['T] | None | Some 'T ;
: mkopt inline ( i64 -- Opt[i64] ) Some ;
: main ( -- )
  1 mkopt drop \"ok\" . ;
";
    assert_eq!(build_and_run("mono-inline-combinator", src), "ok");
}

/// (R4, arity gate untouched) The impl-target seed replaces the *binding* the
/// explicit type-argument list used to make, not the list's own arity check:
/// `check_poly_call`'s `instantiation_arity_error` still reads `type_args`,
/// so a surplus argument is the same located error it always was. Byte-exact,
/// measured from the live binary.
///
/// (Review round, P2-9) The asserted string bakes in internal spellings --
/// `empty;Monoid;0;Opt['T0]`, `'ctor0` -- that diagnostics must eventually
/// render through `rendered_word` (`engine.rs:1111-1116`) rather than the raw
/// mangled name. This is a regression pin on the *current* internal
/// spellings, not a ratification of them: when diagnostic rendering is
/// reworked to go through `rendered_word` here, this assertion is expected to
/// change.
#[test]
fn nullary_member_with_surplus_type_arguments_is_a_located_arity_error() {
    let stderr = build_error_located(
        "nullary-arity",
        "\
type: Opt['T] | None | Some 'T ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for Opt
  : empty None ;
  : combine drop ;
;
: main ( -- )
  empty[Opt[i64] i64] drop ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: `empty;Monoid;0;Opt['T0]` (line 13) declares 1 type variable (`'ctor0`) but was given 2 type arguments"
    );
}

/// The pre-existing fence this slice records rather than fixes (spec Open
/// Questions): a **bare nullary** variant constructor of a generic header, in
/// a mono body, once two instantiations of that header exist. Both candidates
/// take no operands, so operand-directed overload selection cannot separate
/// them and the site is a located error. Byte-identical at base `a1b1276`
/// (measured by stashing this phase's diff), so Phase 1 neither causes nor
/// closes it; pinned here so a later slice that grounds nullary construction
/// from its consuming context has to retire this expectation deliberately.
#[test]
fn two_instantiations_make_a_bare_nullary_variant_a_located_overload_error() {
    let stderr = build_error_located(
        "nullary-fence",
        "\
import: core::list * ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine drop ;
;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  mkempty drop
  empty[List[List[i64]]] drop ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: no overload of `Nil` in `mkempty` (line 12) accepts these operands\n  candidate: no operands\n  candidate: no operands"
    );
}

/// The fence above's verified substitute (the spec's Phase 3 spelling
/// constraint, measured here because Phase 1's `empty` goldens depend on it):
/// keep the program to a *single* `List` instantiation and the same bare `Nil`
/// grounds, the member's own `Nil` grounds through the impl-target seed, and
/// `empty[List[i64]]` returns a list the eliminator reads as empty. This is
/// the byte-unchanged S6 `mconcat_over_list_dispatches` spelling
/// (`: mkempty ( -- List[i64] ) Nil ;`), so it also pins that the fence is
/// about the second instantiation and not about the helper.
#[test]
fn a_single_instantiation_grounds_a_bare_nullary_variant_and_the_member() {
    let src = "\
import: core::list * ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine drop ;
;
: mkempty ( -- List[i64] ) Nil ;
: isnil ( List[i64] -- i64 )
  ~[ ( Nil ) drop 0 ]
  ~[ ( Cons ) Cons> | v rest | v drop rest drop 1 ]
  List? ;
: main ( -- )
  mkempty isnil .
  empty[List[i64]] isnil . ;
";
    assert_eq!(build_and_run("single-inst-nullary", src), "0\n0\n");
}

/// P7b.S8b Phase 2 (R1): the P8-2d2 shape -- a plain generic word (not a
/// trait member) constructing `Cons` through the new `Generic` field arm.
/// Verbatim from the probe round's `p8b-2d2-plain-generic-cons-helper.sth`
/// (`docs/roadmap/P7b/slice8b-probes.md`, Part 1a). The sibling bare-`Nil`
/// main spelling (`p8b-2d2-plain-generic-cons.sth`) is deliberately not a
/// golden here: it fails with `` error: unknown word `Nil` in `main` (line 4) ``
/// because the program never grounds a `List[i64]` instantiation (only
/// `cons2['T]`'s generic signature exists), so no `Nil` variant word is
/// minted for the bare name to resolve to -- unrelated to the arm, and
/// distinct from the recorded nullary-variant fence above (a mis-grounded
/// candidate, not a missing one).
#[test]
fn plain_generic_cons_helper_builds_and_runs_clean() {
    let src = "\
 import: core::list * ;\n\
 : cons2['T] ( 'T List['T] -- List['T] ) ^ Cons ;\n\
 : mknil ( -- List[i64] ) Nil ;\n\
 : main ( -- ) 5 mknil cons2 drop ;\n";
    assert_eq!(build_and_run("2d2-plain-generic-cons-helper", src), "");
}

/// P7b.S8b Phase 2 (R2): the ctor-mismatch shape the new `Generic` field arm
/// must reject -- a differently-headed operand (`Option['T]` where `Cons`'s
/// self-reference field declares `List['T]`) is a located type mismatch,
/// never a panic. `badcons` mirrors the P8-2d2 helper's `cons2['T]` but
/// swaps the tail parameter's header -- the same shape class as the probe
/// round's `p8b-cons-tail-mismatch-clean.sth`
/// (`docs/roadmap/P7b/slice8b-probes.md`, Part 1), adapted to this harness
/// (boxed tail operand, different word/field names), not a byte-for-byte copy.
///
/// (Phase 2 review, P2-4-equivalent risk row) The pinned `note: declared
/// ( -- )` is pre-existing `effect_str` rendering (`poly_rendered_type_mismatch_error`)
/// -- it never renders the word's declared signature (here, `'T Option['T]
/// -- List['T]`), only a placeholder -- but this is the first phase to
/// freeze it byte-exact. Pinned as-is; expected to change when effect
/// rendering is fixed.
#[test]
fn plain_generic_cons_differently_headed_tail_is_a_located_mismatch() {
    let stderr = build_error_located(
        "2d2-ctor-mismatch",
        "\
 import: core::list * ;\n\
 import: core::option * ;\n\
 : badcons['T] ( 'T Option['T] -- List['T] ) ^ Cons ;\n\
 : main ( -- ) 1 None badcons drop ;\n",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: type mismatch in `badcons` (line 5)\n  `Cons` expected `List['T]`, found `Option['T]`\n  note: declared ( -- )"
    );
}

/// P7b.S8b Phase 2 (R1, review round P1): every fixture above exercises a
/// *single*-type-variable generic header (`List['T]`), where the new arm's
/// positional `field_args.iter().zip(op_args.iter())` is order-blind --
/// reversing the zip is a no-op on one element. A two-variable
/// self-referential header (`Pair['A 'B]`, field `rest ^Pair['A 'B]`) is the
/// minimal case that can tell binding order apart: `pcons` grounds `'A :=
/// i64`, `'B := f64` from its own explicit operands before the `rest` field
/// is bound against `mknought`'s already-concrete `Pair[i64 f64]` cell, so a
/// reversed zip would try to re-bind the already-`i64`-bound `'A` to `f64`
/// and take the mismatch branch instead of grounding clean. The output pin
/// (destructuring the built `Node` back out) additionally witnesses the two
/// fields keep their own types and values in declared order, not swapped.
#[test]
fn plain_generic_cons_two_parameter_header_binds_args_in_order() {
    let src = "
 type: Pair['A 'B] | Nought | Node 'A 'B rest ^Pair['A 'B] ;
 : pcons['A 'B] ( 'A 'B ^ Pair['A 'B] -- Pair['A 'B] ) Node ;
 : mknought ( -- Pair[i64 f64] ) Nought ;
 : showpair ( Pair[i64 f64] -- )
   ~[ ( Nought ) drop ]
   ~[ ( Node ) Node> | a b r | a . b . r ^> drop ]
   Pair? ;
 : main ( -- )
   1 2.5 mknought ^ pcons showpair ;
";
    assert_eq!(build_and_run("2param-generic-order", src), "1\n2.5\n");
}

/// P7b.S8b Phase 2 (R3, review round P1): the length-carrying self-reference
/// field the spec's own R3/exit-criteria text called "unspellable in source
/// today" -- false, the review round spelled it. `Ring['T 'N: Len]`'s
/// self-reference field (`next ^Ring['T 'N]`) reaches the new `Generic` field
/// arm's `len_args` fence, verbatim from the probe round's
/// `p8b-lenvar-selfref-fence.sth` (`docs/roadmap/P7b/slice8b-probes.md:67-70`)
/// plus the harness's `: main` entry point (the probe fixture had none).
/// The rejection is right; only the *message* changed -- the probe round's
/// captured text was `` `Ring` expected `Ring['T 'N]`, found `Ring['T 'N]` ``,
/// tautological (this fixture's sides render identically, and a module-only
/// identity mismatch would too, since `poly_type_str` omits the module id).
/// This pins the dedicated fence message instead.
#[test]
fn generic_field_with_a_length_variable_is_a_located_error() {
    let stderr = build_error_located(
        "ring-lenvar-fence",
        "\
 type: Ring['T 'N: Len] head 'T next ^Ring['T 'N] ;\n\
 : mkring['T 'N: Len] ( 'T ^ Ring['T 'N] -- Ring['T 'N] ) Ring ;\n\
 : main ( -- ) ;\n",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: `Ring` in `mkring` (line 4) cannot bind `Ring`'s length variable\n  the field names `Ring` with a length parameter, but constructing a value here infers no lengths; only the field's element type variables can be bound this way"
    );
}

/// P7b.S8b Phase 3 (R8): `Functor for List` -- the S6-dropped `map`
/// golden, now that Phase 2 closes the construction wall `map`'s `Cons`
/// recursion needs. Verbatim from the probe round's
/// `p8b-map-list-end-to-end.sth` (`docs/roadmap/P7b/slice8b-probes.md`, Part
/// 1a), minus the `import: intrinsics *`/`hosted::show` lines
/// `single_file_hosted` already supplies.
#[test]
fn functor_for_list_map_grounds_end_to_end() {
    let src = "
import: core::list * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for List
  : map
    swap
    ~[ ( Nil ) drop drop Nil ]
    ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ 1 add ] map[i64 i64]
  showlist ;
";
    assert_eq!(build_and_run("map-list-e2e", src), "2\n3\n4\n");
}

/// (R8, review round P2) The frame-shape pin the golden above can't give:
/// `impl: Functor for List`'s `map` recurses over its own `Cons` arm
/// (`rot map ^ Cons`), so the only way to tell "one non-inline real frame"
/// from a duplicated-per-instantiation lowering is to look at the emitted
/// symbol itself. Same `emit_ssa_with_manifest` route as
/// `nullary_member_over_a_generic_impl_target_mints_the_one_level_monomorph`
/// above, source byte-identical to the golden this pin covers modulo a
/// uniform one-space indent.
#[test]
fn functor_for_list_map_lowers_as_one_non_inline_frame() {
    let src = "
 import: core::list * ;
 trait: Functor['F: * -> *] :
   map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
 ;
 impl: Functor for List
   : map
     swap
     ~[ ( Nil ) drop drop Nil ]
     ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
     List? ;
 ;
 : mkempty ( -- List[i64] ) Nil ;
 : showlist ( List[i64] -- )
   ~[ ( Nil ) drop ]
   ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
   List? ;
 : main ( -- )
   3 mkempty ^ Cons
   2 swap ^ Cons
   1 swap ^ Cons
   [ 1 add ] map[i64 i64]
   showlist ;
";
    let path =
        std::env::temp_dir().join(format!("sooth-p7bs8b-map-ssa-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing the fixture should succeed");
    let ssa = sooth::driver::emit_ssa_with_manifest(&path, common::manifest_for(&path).as_deref())
        .unwrap_or_else(|e| panic!("emitting the fixture should succeed: {e}"));
    std::fs::remove_file(&path).ok();

    let sites: Vec<&str> = ssa
        .lines()
        .filter(|l| l.contains("sooth_mono_map_Functor"))
        .collect();
    assert_eq!(
        sites.len(),
        3,
        "one definition, the top-level call, and the recursive self-call: {sites:?}"
    );
    let defs = sites.iter().filter(|l| l.contains("function")).count();
    assert_eq!(
        defs, 1,
        "exactly one non-inline frame, not one per call site: {sites:?}"
    );
    let calls = sites.iter().filter(|l| l.contains("call")).count();
    assert_eq!(
        calls, 2,
        "both the outer dispatch and the recursive descent go through a real call, \
         never unrolled: {sites:?}"
    );
}

/// P7b.S8b Phase 3 (R8): the shared-bound variant of the map golden --
/// `map` dispatched twice through a *shared* `Functor` bound (`'U := 'T`
/// specialization, the spellings probe's verified substitute). Verbatim from
/// `p8b-map-shared-bound-twice.sth` (`docs/roadmap/P7b/slice8b-probes.md`,
/// Part 1a), same import elision as above.
#[test]
fn functor_for_list_map_through_a_shared_bound_grounds() {
    let src = "
import: core::list * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for List
  : map
    swap
    ~[ ( Nil ) drop drop Nil ]
    ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: twice['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] )
  | q |
  q map
  q map ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ 1 add ] twice
  showlist ;
";
    assert_eq!(build_and_run("map-shared-bound-twice", src), "3\n4\n5\n");
}

/// P7b.S8b Phase 3 (R9): `combine` through a `Monoid` bound over two
/// `List[i64]` spines, via a poly middleman (`merge['T: Monoid]`) rather than
/// the direct `impl:` body S6's `monoid_for_list_append_construction_builds_and_runs_clean`
/// already pins. Verbatim from `p8b-sp-4-combine-through-bound.sth`
/// (`docs/roadmap/P7b/slice8b-probes.md`, Part 1a / Part 3 item 4). The probe's
/// "green twice" verdict ran the SAME produced binary twice; this golden
/// reproduces that: one build, two runs, identical pinned stdout (the
/// determinism check, not just the output).
#[test]
fn monoid_for_list_combine_through_bound_grounds_and_is_stable() {
    let src = "
import: core::list * ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine
    swap
    ~[ ( Nil ) drop ]
    ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: merge['T: Monoid] ( 'T 'T -- 'T ) combine ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  3 mkempty ^ Cons
  5 swap ^ Cons
  merge
  showlist ;
";
    let (_t, entry) = single_file_hosted("combine-through-bound", src);
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
    for run_idx in 1..=2 {
        let run = Command::new(&binary).output().expect("binary should run");
        assert_eq!(
            run.status.code(),
            Some(0),
            "run {run_idx}: the built binary should exit 0 (a SIGSEGV is code None/139), stderr: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8(run.stdout).expect("stdout should be utf8"),
            "1\n2\n3\n5\n3\n",
            "run {run_idx}: same binary, same output"
        );
    }
}

/// P7b.S8b Phase 3 (R4+R5, regression): the segfault round's minimal repro
/// (`p8b-bisect-v1-drop-then-empty.sth`, `docs/roadmap/P7b/slice8b-probes.md`,
/// Part 1a / Part 2 Step 1) -- a prior `Cons` construction, dropped, then
/// `empty[List[i64]]`. Pre-Phase-1 this SIGSEGVs the direct binary
/// (exit 139) though `sooth run` merely exits 1 silently; Phase 1's
/// two-defect fix (theta seeding + per-instantiation variant words) is what
/// this golden regresses against.
#[test]
fn list_shaped_two_defect_repro_grounds() {
    let src = "
import: core::list * ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine
    swap
    ~[ ( Nil ) drop ]
    ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;
: main ( -- )
  1 mkempty ^ Cons drop
  empty[List[i64]] drop \"ok\" . ;
";
    assert_eq!(build_and_run("bisect-v1-drop-then-empty", src), "ok");
}

/// P7b.S8b Phase 3 (R9, spelling constraint): the explicit-route-only
/// `empty[List[i64]]` mono-main golden -- no prior `List` construction in the
/// program, so it is unaffected by (and does not regress-test) the Phase 1
/// two-defect pair. Note (R9): the S6 golden `mconcat_over_list_dispatches`
/// dispatches `Monoid for i64`'s `empty`, not `List`'s -- bound-directed
/// `empty` resolving at the `List` impl itself remains unverified per R9 and
/// is not claimed here (this golden pins the explicit route only).
/// Shape verified at the probe round's
/// `p8b-side-empty-instantiation-only.sth` (`docs/roadmap/P7b/slice8b-probes.md`,
/// Part 1 battery: build OK, run exit 0, stdout `ok`); this fixture is a
/// minimal from-scratch equivalent (that source is not inlined verbatim in
/// Part 1a).
#[test]
fn explicit_empty_list_i64_mono_main_grounds() {
    let src = "
import: core::list * ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine drop ;
;
: main ( -- ) empty[List[i64]] drop \"ok\" . ;
";
    assert_eq!(build_and_run("empty-list-i64-only", src), "ok");
}

/// P7b.S8b Phase 3 (R10, linearity pin): an undropped `map` result is a
/// located build error, never a silent drop or a panic -- the same
/// `linear value left on the stack` diagnostic S8's own goldens exercise for
/// other types. Byte-exact, measured from the live binary (this fixture's own
/// line numbers, not a probe fixture's).
#[test]
fn undropped_map_result_is_a_located_linear_error() {
    let stderr = build_error_located(
        "undropped-map-result",
        "
import: core::list * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for List
  : map
    swap
    ~[ ( Nil ) drop drop Nil ]
    ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ 1 add ] map[i64 i64] ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: linear value left on the stack in `main` (line 20)\n  body leaves a `List[i64]` beyond the 0 declared output(s): a linear value must be consumed exactly once, so `drop` it or return it\n  note: declared ( -- )"
    );
}

/// P7b.S8b Phase 3 (R10, linearity pin): the `append`/`combine` twin of the
/// pin above -- an undropped `combine` result over `List[i64]` is the same
/// located error, never a silent drop or a panic. Byte-exact.
#[test]
fn undropped_append_result_is_a_located_linear_error() {
    let stderr = build_error_located(
        "undropped-append-result",
        "
import: core::list * ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for List
  : empty Nil ;
  : combine
    swap
    ~[ ( Nil ) drop ]
    ~[ ( Cons ) Cons> | v rest | rest ^> swap combine v swap ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  3 mkempty ^ Cons
  5 swap ^ Cons
  combine ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: linear value left on the stack in `main` (line 24)\n  body leaves a `List[i64]` beyond the 0 declared output(s): a linear value must be consumed exactly once, so `drop` it or return it\n  note: declared ( -- )"
    );
}

/// P7b.S8b Phase 3 (R11, linearity pin): `dup` of a `List['T]` operand in a
/// poly body is the pre-existing `poly_copy_generic_error` fence (S8's
/// carve-out; unrelated to the construction wall) -- byte-exact, the same
/// shape class as the probe round's `p8b-dup-list-operand-fenced.sth`
/// (`docs/roadmap/P7b/slice8b-probes.md`, Part 1), reproduced from scratch
/// (that source is not inlined verbatim in Part 1a) since only the message,
/// not the fixture bytes, is what this pins.
#[test]
fn dup_of_generic_list_operand_is_a_located_copy_error() {
    let stderr = build_error_located(
        "dup-list-operand",
        "
import: core::list * ;
: duplist['T] ( List['T] -- List['T] List['T] ) dup ;
: main ( -- ) ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: cannot `dup` a generic type applied to a variable in `duplist` (line 5)\n  `List['T]` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated"
    );
}

/// P7b.S8b Phase 3 (R12, distinct-`'U` fence): a plain poly word
/// (`mapadd['F: Functor 'T 'U]`) that widens `Functor.map`'s bound to a
/// second, output-only type variable is a located error -- inference does
/// not bind an output-only variable through a shared bound (the spellings
/// round's mechanical finding: a quotation literal only unifies against
/// already-bound row variables). Byte-exact, reproduced from scratch to the
/// same shape class as `p8b-map-distinct-u-unbound.sth`
/// (`docs/roadmap/P7b/slice8b-probes.md`, Part 1); the working substitute
/// this fence's Open-Questions note points to (`'U := 'T` specialization) is
/// already exercised by `functor_for_list_map_grounds_end_to_end` above.
#[test]
fn map_output_variable_unbound_through_a_shared_functor_bound_is_located_error() {
    let stderr = build_error_located(
        "distinct-u-unbound",
        "
import: core::list * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for List
  : map
    swap
    ~[ ( Nil ) drop drop Nil ]
    ~[ ( Cons ) Cons> | v rest | dup v swap call rest ^> rot map ^ Cons ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: mapadd['F: Functor 'T 'U] ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) map ;
: main ( -- )
  3 mkempty ^ Cons
  [ drop \"x\" ] mapadd
  drop ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: `mapadd` in `main` (line 19) has output variable `'U` that no input binds\n  note: supply it explicitly: `mapadd[SomeType SomeType SomeType]`"
    );
}

/// P7b.S8b (round-1 review, P2): the `Generic` field arm's length fence used
/// to fire on *any* non-empty `len_args`, so a field naming a **concrete**
/// length was rejected with "cannot bind `Ring`'s length variable" -- naming a
/// variable the field does not have. `Ring['T 3]` has nothing to bind and
/// nothing to infer, so it grounds. The self-reference `next ^Ring['T 'N]`
/// keeps the header honest (this is the same `Ring` the fence golden above
/// uses); the *constructed* field is `Holder`'s `r`, which is where the
/// concrete-length `Generic` operand reaches the arm.
///
/// `mkholder` is never called: a generic word's body is walked by the poly
/// pre-pass without instantiation (the fence golden above rejects from an
/// empty `main` the same way), so building at all is the witness -- there is
/// no way to *construct* a `Ring` to call it with, since its `next` field
/// makes the type infinitely sized.
#[test]
fn generic_field_with_a_concrete_length_grounds() {
    let src = "\
 type: Ring['T 'N: Len] head 'T next ^Ring['T 'N] ;\n\
 type: Holder['T] r Ring['T 3] ;\n\
 : mkholder['T] ( Ring['T 3] -- Holder['T] ) Holder ;\n\
 : main ( -- ) ;\n";
    assert_eq!(build_and_run("ring-concrete-len", src), "");
}

/// P7b.S8b (round-1 review, P2): a **two-variable** bare impl target is the
/// hole `nullary_member_with_surplus_type_arguments_is_a_located_arity_error`
/// above leaves open. That golden's `empty[Opt[i64] i64]` is caught because
/// `Opt` declares one variable and the list carries two; `Pair` declares two,
/// so the same list *passes* the arity gate, reaches the impl-target seed --
/// which grounds both variables from the dispatch type alone -- and the
/// second written argument was read by nothing. `empty[Pair[i64 i64] str]`
/// and `empty[Pair[i64 i64] i64]` both minted
/// `sooth_mono_empty_Monoid_0_Pair__T0__T1___m0__t0_i64_t1_i64`: one
/// monomorph from two spellings, the `str` silently dropped. It is now a
/// located conflict, in the spirit of
/// `mono_concrete_member_call_with_explicit_type_args_is_error`
/// (`src/check/poly.rs`), which likewise rejects a meaningless list rather
/// than dropping it.
///
/// The asserted string bakes in the same internal spellings P2-9 records
/// above (`empty;Monoid;0;Pair['T0 'T1]`, `'ctor1`): a regression pin on the
/// current rendering, not a ratification of it.
#[test]
fn nullary_member_type_argument_disagreeing_with_its_impl_target_is_an_error() {
    let stderr = build_error_located(
        "seed-conflict",
        "\
type: Pair['A 'B] | Nought | Node 'A 'B ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for Pair
  : empty Nought ;
  : combine drop ;
;
: main ( -- )
  empty[Pair[i64 i64] str] drop ;
",
    );
    assert_eq!(
        stderr.trim_end(),
        "error: `empty;Monoid;0;Pair['T0 'T1]` in `main` (line 13) was written with `'ctor1` = `str`, but its impl target determines `'ctor1` = `i64`"
    );
}

/// P7b.S8b (round-1 review, P2): the accept half of the fix above -- a written
/// argument that *agrees* with what the impl target determines still grounds,
/// so closing the silent drop costs no working spelling. Same program as the
/// conflict golden, `str` replaced by the `i64` the dispatch type implies.
#[test]
fn nullary_member_type_argument_agreeing_with_its_impl_target_grounds() {
    let src = "
type: Pair['A 'B] | Nought | Node 'A 'B ;
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for Pair
  : empty Nought ;
  : combine drop ;
;
: main ( -- )
  empty[Pair[i64 i64] i64] drop ;
";
    assert_eq!(build_and_run("seed-agreeing", src), "");
}

/// P7b.S8b (round-3 review, P0): the three ICE regression pins.
///
/// Letting a *concrete* length mismatch fall through to `mismatch()` (the
/// round-1 amendment above) sent shapes to `poly_type_str` that the old
/// all-lengths fence had pre-empted -- and the renderer indexed
/// `sig.ty_var_names` raw. A construction field carries **its own header's**
/// variable ids, so `Holder['T] r Ring['T 3]` hands `Var(0)` to a caller that
/// declares no type variables at all: index out of bounds, a panic on the
/// error path. This exact program was a clean located error before the
/// amendment and an ICE after it, which is what the review caught.
///
/// The fix is a total renderer (`foreign_var_str`), so the assertion is the
/// full byte-exact diagnostic: it pins both that the build is *located* (the
/// panic is gone) and that the placeholder keeps the two sides readable --
/// `Ring['?0 3]` against `Ring[array[i64 'N] 5]` still shows which lengths
/// disagree.
#[test]
fn concrete_length_mismatch_renders_a_foreign_field_var_as_a_placeholder() {
    let src = "\
 type: Ring['T 'N: Len] head 'T next ^Ring['T 'N] ;\n\
 type: Holder['T] r Ring['T 3] ;\n\
 : mk['N: Len] ( Ring[array[i64 'N] 5] -- Holder[array[i64 'N]] ) Holder ;\n\
 : main ( -- ) ;\n";
    assert_eq!(
        build_error_located("foreign-var-len-mismatch", src),
        "error: type mismatch in `mk` (line 5)\n  `Holder` expected `Ring['?0 3]`, found `Ring[array[i64 'N] 5]`\n  note: declared ( -- )\n"
    );
}

/// P7b.S8b (round-3 review, P0): the same panic class reached by a
/// *differently-headed* operand rather than a length disagreement -- `H`'s
/// field is `L['T]`, the operand is a `Box[...]`, so the identity check
/// rejects and renders. Unlike the golden above this shape ICEd **before**
/// this slice too (the old fence only covered lengths, and there is no length
/// here to fence), so it pins a pre-existing hole the total renderer closes,
/// not a regression of ours.
#[test]
fn header_mismatch_renders_a_foreign_field_var_as_a_placeholder() {
    let src = "\
 type: L['T] | Nil2 | Cons2 'T ;\n\
 type: Box['T] v 'T ;\n\
 type: H['T] r L['T] ;\n\
 : mk2['N: Len] ( Box[array[i64 'N]] -- H[array[i64 'N]] ) H ;\n\
 : main ( -- ) ;\n";
    assert_eq!(
        build_error_located("foreign-var-header-mismatch", src),
        "error: type mismatch in `mk2` (line 6)\n  `H` expected `L['?0]`, found `Box[array[i64 'N]]`\n  note: declared ( -- )\n"
    );
}

/// P7b.S8b (round-3 review, P0): the *partly* foreign case, which is why the
/// placeholder carries the raw id instead of one anonymous marker. `HH['A 'B]`
/// stores `P2[Var(0) Var(1)]`; the caller `mkhh['X]` declares one variable, so
/// `Var(0)` resolves to `'X` and only `Var(1)` falls off the end. A single
/// side can be half nameable, and `P2['X '?1]` says exactly which half.
#[test]
fn header_mismatch_renders_a_partly_foreign_field_var_as_a_placeholder() {
    let src = "\
 type: P2['A 'B] | N2 | C2 'A 'B ;\n\
 type: Box['T] v 'T ;\n\
 type: HH['A 'B] r P2['A 'B] ;\n\
 : mkhh['X] ( Box['X] -- HH['X 'X] ) HH ;\n\
 : main ( -- ) ;\n";
    assert_eq!(
        build_error_located("foreign-var-partial", src),
        "error: type mismatch in `mkhh` (line 6)\n  `HH` expected `P2['X '?1]`, found `Box['X]`\n  note: declared ( -- )\n"
    );
}
