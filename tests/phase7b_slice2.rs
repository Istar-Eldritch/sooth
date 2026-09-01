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

/// The printing-golden runner, from `tests/phase7b_slice1.rs`: builds, runs
/// the binary, asserts exit 0, and returns the exact stdout.
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

/// The hosted twin of `single_file`, from `tests/phase7b_slice1.rs`: adds
/// the hosted manifest (a bare package cannot import `core` --
/// `anonymous_package_error`) and the selective `hosted::show | . |` import
/// (P7.S7d retired the `.` intrinsic onto `hosted::show`'s ordinary word).
/// Core modules are NOT pre-imported: a fixture declaring a local twin of a
/// core type (`Result` in golden #7) must not collide with an import, so
/// each fixture's src pulls its own imports.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs2 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
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

// ---- Phase 2: target and member-word construction + the F14 arm fix ----

/// Golden (positive #5): a bare ctor impl target (`for Box`) desugars to the
/// ctor applied to fresh pattern variables (S2-4, m2's shape as a permanent
/// golden), registers, and dispatches at a concrete operand through the
/// existing applied-var machinery. Pre-S2-4 this died at the shared arity
/// gate (`generic type Box declares 1 type variable, but none were
/// supplied`). The declared-sig helper `mk` is the pinned ctor-in-`main`
/// idiom (F12).
#[test]
fn bare_ctor_impl_target_resolves_and_dispatches() {
    let src = "\
type: Box['T] v 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Box
  : size drop 9 ;
;
: sized['F: Functor] ( 'F -- i64 ) size ;
: mk ( i64 -- Box[i64] ) Box ;
: main ( -- ) 5 mk sized . ;
";
    let (_t, entry) = single_file_hosted("s2-4-bare-ctor-target", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "9\n");
}

/// Golden (positive #7): a partially-applied ctor target (`for Result[i64]`,
/// the S2-4 extension) desugars to the explicit prefix plus fresh variables
/// for the remaining slots (`Result[i64 'ctor1]`), and the pinned prefix
/// binds at dispatch: two impls differing only in the explicit prefix
/// (`Result[i64]` vs `Result[Flag]`) are distinct targets, each dispatching
/// its own member at the operand whose leading slot matches the pin. The
/// desugar dropping the pin (minting an all-variable pattern) would make
/// the two impls alpha-equivalent duplicates and this program would not
/// build. The ctor is a local 2-arg twin of `core::result`'s `Result` (the
/// spec's literal `Result` spelling): the S1-era module-identity convention
/// mismatches a lib-declared header's declaring-module pattern against a
/// user-module-named instantiation's recorded module (pre-existing, F-level,
/// reachable only via imported headers) -- a local twin keeps this golden
/// on S2-4's mechanics.
#[test]
fn partially_applied_ctor_impl_target_binds_explicit_prefix() {
    let src = "\
type: Result['T 'E] | Ok 'T | Err 'E ;
type: Flag | yes | no ;
trait: Prj['F] : proj ( 'F -- i64 ) ; ;
impl: Prj for Result[i64]
  : proj ~[ ( Ok ) Ok> ] ~[ ( Err ) drop -1 ] Result? ;
;
impl: Prj for Result[Flag]
  : proj ~[ ( Ok ) drop 42 ] ~[ ( Err ) drop -2 ] Result? ;
;
: callproj['F: Prj] ( 'F -- i64 ) proj ;
: mki64ok ( i64 -- Result[i64 i64] ) Ok ;
: mkflagok ( Flag -- Result[Flag i64] ) Ok ;
: main ( -- )
  1 mki64ok callproj .
  yes mkflagok callproj . ;
";
    let (_t, entry) = single_file_hosted("s2-4-partial-ctor-prefix", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "1\n42\n");
}

/// Golden (positive #6, S2-13/F14): a zero-field variant ctor in a
/// polymorphic arm unifies with the ambient type variable. The `mapover`
/// poly word is W2's member body pinned as a standalone word (over a local
/// twin of `core::option`, keeping the fixture self-contained -- the F14
/// mechanics are the ctor's, not the module's); the `mknone` declaration
/// registers a concrete `Option[i64]` instantiation, whose generated
/// zero-field `None` ctor word used to capture the poly arm's call (an
/// empty input row trivially exact-matches) and mint the mono
/// `Option[i64]` against the Some arm's `Option['U]` -- the arms-disagree
/// error. The symbolic construction now binds the header argument from the
/// declared output, so the arms agree (`Option['U]`).
#[test]
fn zero_field_ctor_unifies_with_ambient_var_in_poly_arm() {
    let src = "\
type: Option['T] | None | Some 'T ;
: mapover['T 'U] ( Option['T] [ 'T -- 'U ] -- Option['U] )
  swap
  ~[ ( Some ) Some> swap call Some ]
  ~[ ( None ) drop drop None ]
  Option? ;
: mknone ( -- Option[i64] ) None ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("s2-13-zero-field-ctor-poly-arm", src);
    build_ok(&entry);
}

/// Golden (positive #9, S2-11): a ctor-abstract impl (`for Box`) living in
/// the *constructor's* module satisfies the orphan rule -- the same two
/// homes a concrete target gets. Pre-S2-11 a generic target was treated
/// like a scalar (trait-module-only), so this exact program was an orphan
/// rejection.
#[test]
fn ctor_impl_in_ctor_module_satisfies_orphan_rule() {
    let t = Tree::new("s2-11-orphan-ctor-module");
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs2 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    t.write(
        "trait.sth",
        "trait: Functor['F] : size ( 'F -- i64 ) ; ;\nexport: Functor ;\n",
    );
    t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\nimport: self::trait * ;\n\
         type: Box['T] v 'T ;\n\
         impl: Functor for Box\n  : size drop 9 ;\n;\n\
         : sized['F: Functor] ( 'F -- i64 ) size ;\n\
         : mk ( i64 -- Box[i64] ) Box ;\n\
         : main ( -- ) 5 mk sized . ;\n",
    );
    let entry = t.0.join("main.sth");
    let out = build_and_run(&entry);
    assert_eq!(out, "9\n");
}

/// Golden (error #3, S2-15.c/S2-7): a member application applying the trait
/// variable to more arguments than the impl target constructor declares is
/// a located parse-time error, raised by the grounding App arm before its
/// `targetArgs[n..]` slice could panic. The member's own application arity
/// is self-consistent within the trait (it *establishes* the header
/// variable's kind, S1-3), so the fit against the ctor is only checkable at
/// the impl's desugar -- the only point where both arities are in hand
/// (S2-7) -- and `Option`'s one slot cannot take the two-argument
/// application. The error locates at the impl member (where the failing
/// fit is declared), not the trait declaration.
#[test]
fn member_app_arity_exceeding_target_ctor_arity_is_error() {
    let src = "\
type: Option['T] | None | Some 'T ;
trait: P2['F: * -> * -> *] : pairup ( 'F['A 'B] -- ) ; ;
impl: P2 for Option['O]
  : pairup drop ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("s2-15c-arity-exceeds", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: trait member `pairup` of `P2` (line 5, col 5) applies the trait \
             variable `'F` to 2 type arguments, but the impl target constructor `Option` \
             declares 1"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "an application of the trait variable may not exceed the constructor's \
             declared type-parameter count"
        ),
        "{err}"
    );
}

// ---- Phase 3: dispatch and the member-call path ----

/// Golden (positive #2, W2): `map` over `Option[i64]` with a `[ i64 -- Bool ]`
/// quotation dispatches to the Option impl and produces `Option[Bool]` --
/// exit criterion #3's Option half. The observable is the fixture-local
/// printer (`hosted::show`'s type-directed Bool dot behind an eliminator
/// unwrap): `true` on stdout proves the mapped inner value IS a `Bool`, which
/// only a dispatched member call producing `Option[Bool]` can typecheck.
///
/// Fixture notes, all pinned by earlier findings: `Option` is a local twin
/// (the S1-era module-identity wart on imported headers, as in goldens
/// #6/#7); `mkopt` is the declared-sig helper (F12's ctor-in-`main` idiom);
/// the call carries the explicit instantiation `map[i64 Bool]` -- `map`'s
/// `'U` is bound only by the quotation's rows, and a mono caller binds row
/// variables through the P7.S3t seeded θ (the established remedy for a
/// variable no input binds), positionally over the member word's own union
/// id space (`'ctor0` then `'U`).
#[test]
fn functor_map_over_option_dispatches_and_produces_option_of_bool() {
    let src = "\
import: core::bool | Bool | ;
type: Opt['T] | None | Some 'T ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Opt
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;
;
: showopt ( Opt[Bool] -- )
  ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Opt? ;
: mkopt ( i64 -- Opt[i64] ) Some ;
: main ( -- ) 3 mkopt [ drop True ] map[i64 Bool] showopt ;
";
    let (_t, entry) = single_file_hosted("w2-functor-map-option", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "true\n");
}

/// Golden (error #5, S2-15.e): a bare-variable impl target for an HKT trait
/// cannot capture a constructor-keyed operand. The capture is refused at the
/// earliest possible point -- the desugar's S2-6/S2-15.e grounding twin
/// (`member_app_abstract_target_error`): the member's dispatchable input is
/// application-headed, and a fully-abstract target names no constructor for
/// the application to dissolve into, so the impl cannot even register. The
/// dispatch-side twin (the matcher's `for 'T` guard against a `CtorImage`
/// ty, S2-8) is pinned at unit level in `check/poly.rs`; the two together
/// are S2-15.e ("the dispatch-side twin is S2-8's `for 'T` guard", S2-6).
#[test]
fn bare_var_impl_target_does_not_capture_ctor_image() {
    let src = "\
type: Opt['T] | None | Some 'T ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for 'T
  : map drop ;
;
: main ( -- ) ;
";
    let (_t, entry) = single_file("s2-15e-bare-var-capture", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: trait member `map` of `Functor` (line 7, col 5) applies the trait \
             variable `'F`, but the impl target `'T` at line 6, col 19 is not a constructor"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "a fully-abstract target names no constructor for the application to \
             dissolve into"
        ),
        "{err}"
    );
}

// ---- Phase 4: end-to-end goldens and non-regression ----

// The W3/W4 twins. Both witnesses run over fixture-local twins of
// `core::option`/`core::result` rather than the lib types themselves: a
// lib-declared generic header cannot take a ctor-keyed `impl:` from a user
// module yet. The S1-era module-identity convention mints the operand's
// instantiation at the *naming* module (`resolve_type_or_apply` ->
// `instantiate_*` with the parsing module, `parser.rs:6862-6880`; memo key
// `(idx, module, args, lens)`), while the impl target pattern records the
// header's *declaring* module, and both dispatch paths compare the two for
// equality (`match_impl_target_rec`'s `Generic` arm; the CtorImage identity
// match feeds on the same recorded module). Observed verbatim: a mono caller
// reports "no `impl:` in this program dispatches on these operands"; a poly
// caller reports "cannot instantiate `'F` ... does not satisfy `Functor`".
// This is the wart documented at golden #7 (in this file) and deferred to a
// future ruling (slice2-spec.md, Open questions); committed W2 (golden #2
// above) set the twin precedent. What W3/W4 prove -- leading-slot
// displacement and shared-bound dispatch -- is the machinery under test,
// not the type's provenance.

/// Golden (positive #3, W3): `map` over `Result[i64 i64]` dispatches to the
/// Result impl and passes `Err` through untouched -- exit criterion #4's
/// separate-impl half, and the witness that rules out R2(c) (multi-arg ctors
/// are not fenced). The Ok path prints the *mapped* payload (`1` with
/// `[ 1 sub ]` applied: `0`); the Err path prints the *original* payload
/// (`2` -- the map never touched it), so the exact stdout pins both the
/// dispatch and the pass-through.
///
/// The trait is `Functor['F: * -> * -> *]` with member
/// `map ( 'F['T 'E] [ 'T -- 'U ] -- 'F['U 'E] )`: a `* -> *` Functor's
/// `'F['T]` cannot unify against a two-argument `Result` operand (App-vs-App
/// unification requires equal argument counts), so W3's trait declares the
/// two-parameter kind, as the S2-8 tie-rule unit fixtures do. The Err
/// pass-through IS the S2-6 leading-slot displacement reading: the output
/// `'F['U 'E]` displaces slot 0 with `'U` while slot 1 stays the target's
/// own `'E`. The Err arm destructures before reconstructing
/// (`Err> swap drop Err`) because an arm-bound variant value cannot escape
/// the arm that bound it; the quotation between the scrutinee and the
/// payload is dropped explicitly (the pinned body idiom's `drop drop`, split
/// around a field-carrying payload). The call carries the explicit
/// instantiation `map[i64 i64 i64]` over the member word's union id space
/// (target vars `'ctor0 'ctor1` first, the appended local `'U` last -- the
/// dispatch machinery is fully exercised either way; this is recorded W2
/// deviation (1), slice2-spec.md Open questions).
#[test]
fn functor_map_over_result_dispatches_to_the_result_impl_and_passes_err_through() {
    let src = "\
type: Result['T 'E] | Ok 'T | Err 'E ;
trait: Functor['F: * -> * -> *] :
  map ( 'F['T 'E] [ 'T -- 'U ] -- 'F['U 'E] ) ;
;
impl: Functor for Result
  : map swap ~[ ( Ok ) Ok> swap call Ok ] ~[ ( Err ) Err> swap drop Err ] Result? ;
;
: showres ( Result[i64 i64] -- )
  ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Result? ;
: mkok ( i64 -- Result[i64 i64] ) Ok ;
: mkerr ( i64 -- Result[i64 i64] ) Err ;
: main ( -- ) 1 mkok [ 1 sub ] map[i64 i64 i64] showres
  2 mkerr [ 1 sub ] map[i64 i64 i64] showres ;
";
    let (_t, entry) = single_file_hosted("w3-functor-map-result", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "0\n2\n");
}

/// Golden (positive #4, W4): `twice` -- a poly body holding ONE shared
/// `Functor` bound -- calls `map` twice, and the bound dispatches per
/// constructor at the call sites: the Opt impl serves the `Opt` operand, the
/// Res impl the `Res` operand (exit criterion #4's shared-bound dogfood,
/// dispatched through one bound word).
///
/// Machinery under test: at `twice`'s body check the member call is
/// App-vs-App unification against the *declared* sig -- the member's header
/// variable binds to the caller's bound `'F` and the member's locals to the
/// caller's slot arguments; no `CtorImage` exists yet. The caller's own
/// instantiation (bare `twice` calls from mono `main`, theta seeded through
/// the App arm as a `CtorImage`) grounds the head, and S2-9's per-site
/// re-grounding rebuilds each site's theta_call from the obligation's slot
/// record -- one monomorph per (word, theta_call), never the empty subst.
/// Member calls never reach the S2-10 cross-call fence
/// (`poly_trait_member_call` fronts them), so this golden only exercises
/// that the fence is not tripped, not that it fired.
///
/// Sketch deltas from the brief's W4, recorded in slice2-spec.md's Open
/// questions: (1) the sketch's `map map` consumes the quotation parameter on
/// the first call; plain quotations are `Copy`, so the working form binds it
/// to a local and re-reads it (`| q | q map q map`). (2) The sketch's one
/// `Functor['F: * -> *]` over both `Some` and `Ok` is kind-inconsistent (see
/// golden #3), so the shared trait is `* -> * -> *` and both ctor twins are
/// two-parameter types -- `Opt`'s `Some` carries both slots and `None`
/// exercises golden #6's zero-field arm unification against the ambient
/// variable. (3) Local twins per the W3/W4 note above.
#[test]
fn functor_map_through_shared_bound_dispatches_per_constructor_from_poly_body() {
    let src = "\
type: Opt['T 'E] | None | Some 'T 'E ;
type: Res['T 'E] | Ok 'T | Err 'E ;
trait: Functor['F: * -> * -> *] :
  map ( 'F['T 'E] [ 'T -- 'U ] -- 'F['U 'E] ) ;
;
impl: Functor for Opt
  : map swap ~[ ( Some ) Some> swap rot call swap Some ] ~[ ( None ) drop drop None ] Opt? ;
;
impl: Functor for Res
  : map swap ~[ ( Ok ) Ok> swap call Ok ] ~[ ( Err ) Err> swap drop Err ] Res? ;
;
: twice['F: Functor 'T 'E] ( 'F['T 'E] [ 'T -- 'T ] -- 'F['T 'E] )
  | q |
  q map
  q map ;
: showopt ( Opt[i64 i64] -- ) ~[ ( Some ) Some> drop . ] ~[ ( None ) drop ] Opt? ;
: showres ( Res[i64 i64] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Res? ;
: mkopt ( i64 -- Opt[i64 i64] ) dup Some ;
: mkres ( i64 -- Res[i64 i64] ) Ok ;
: main ( -- ) 1 mkopt [ 1 sub ] twice showopt
  5 mkres [ 1 sub ] twice showres ;
";
    let (_t, entry) = single_file_hosted("w4-shared-bound-twice", src);
    let out = build_and_run(&entry);
    // `twice` applies `[ 1 sub ]` twice: the Some payload 1 -> -1, the Ok
    // payload 5 -> 3. Two different impls served the two calls through the
    // one bound word.
    assert_eq!(out, "-1\n3\n");
}

/// Golden (positive #8): a concrete-pinned impl (`for Opt[i64]`) wins over
/// the ctor-abstract impl (`for Opt`) at an `Opt[i64]` operand -- extends
/// p11c's specificity control with the bare-ctor desugar as the generic
/// side. The operand is fully concrete, so dispatch runs the existing
/// `Concrete`/`Generic` matcher arms and `select_most_specific` (S2-8's
/// dispatch rule: the CtorImage arm never sees a mono call); the printed `1`
/// proves the concrete impl's member served the call (`for Opt` prints 2).
/// The member is the Star-trait `size`: an application-headed member has no
/// monomorphic representation against a `Concrete` target (the S2-6 fence),
/// so the specificity contest at a concrete operand is p11c's Star shape
/// with the ctor target as the generic side. Local twin per the W3/W4 note
/// above (golden #7's documented wart).
#[test]
fn concrete_impl_wins_over_ctor_impl_by_specificity() {
    let src = "\
type: Opt['T] | None | Some 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Opt[i64]
  : size drop 1 ;
;
impl: Functor for Opt
  : size drop 2 ;
;
: sized['F: Functor] ( 'F -- i64 ) size ;
: mkopt ( i64 -- Opt[i64] ) Some ;
: main ( -- ) 5 mkopt sized . ;
";
    let (_t, entry) = single_file_hosted("s2-specificity-concrete-vs-ctor", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "1\n");
}

/// Golden (positive #10, S2-12): two same-named constructors in two modules,
/// one `Functor` impl each, both dispatched in one program -- a's `Widget`
/// prints 1, b's prints 2. The two impls' member words share one synthesized
/// name (`size;Functor;<trait module>;Widget['T0]` -- the shape render
/// carries no module identity), so `overload_symbols` suffixes them `$$0` /
/// `$$1`; distinct lowering symbols are what keeps the two dispatches from
/// monomorphizing to one specialization (which would print the same number
/// twice -- the S1-12 residual hazard S2-12 closes).
///
/// Two pre-existing behaviors the fixture works around (documented, not
/// hidden): (a) a *mono* caller of these member words gets
/// `mono_member_unroutable_error` -- the colliding words' `$$N` overload
/// symbols are not `poly_env` keys (that map is keyed by the bare word
/// name) -- so the dispatch goes through the poly bounded caller `sized`,
/// whose bound dispatch is whole-program; (b) the generated-ctor `env`
/// dispatch is module-blind name+input-shape first-match (the F12 ctor-word
/// family), so identical payload types would cross-pick across modules --
/// b's `Widget` carries a `str` payload so each module's ctor call matches
/// only its own candidate. Each module constructs through a private
/// declared-sig helper (`mk`, the F12 idiom) and exports only `run`, whose
/// effect names no private type.
#[test]
fn same_named_ctors_in_two_modules_dispatch_distinct_impls() {
    let t = Tree::new("s2-12-same-named-ctors");
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs2 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    t.write(
        "f.sth",
        "import: intrinsics * ;\n\
         trait: Functor['F] : size ( 'F -- i64 ) ; ;\n\
         : sized['F: Functor] ( 'F -- i64 ) size ;\n\
         export: Functor sized ;\n",
    );
    t.write(
        "a.sth",
        "import: intrinsics * ;\nimport: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Functor for Widget\n  : size drop 1 ;\n;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : run ( i64 -- i64 ) mk sized ;\n\
         export: run ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ;\nimport: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Functor for Widget\n  : size drop 2 ;\n;\n\
         : mk ( str -- Widget[str] ) Widget ;\n\
         : run ( str -- i64 ) mk sized ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n\
         import: self::f * ;\nimport: self::a ;\nimport: self::b ;\n\
         : main ( -- ) 5 a::run . \"x\" b::run . ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(out, "1\n2\n");
}

/// Non-regression (W6, p2): the applied-target Functor control -- an S1-era
/// applied target (`for Box[i64]`, a `Concrete` pattern) dispatching through
/// a bounded poly caller at a concrete operand (theta('F) is a concrete
/// `Struct`, a Star variable -- no CtorImage anywhere) -- is unchanged. The
/// impl target's own parse mints `Box[i64]`, which is why `5 Box` in `main`
/// resolves (the F12 ctor-word wart fires only when nothing has named the
/// instantiation).
#[test]
fn applied_target_functor_dispatch_unchanged() {
    let src = "\
type: Box['T] v 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Box[i64]
  : size drop 1 ;
;
: sized['F: Functor] ( 'F -- i64 ) size ;
: main ( -- ) 5 Box sized . ;
";
    let (_t, entry) = single_file_hosted("w6-applied-target-control", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "1\n");
}

/// Non-regression (W6): the S3t-style explicit instantiation -- theta seeded
/// from the call site's `[...]` list over a declared quotation parameter,
/// sorted, and minted (the S1 golden's `pairwise[i64 f64]` shape) -- still
/// parses, checks, and monomorphs unchanged, in a program that also runs an
/// S2 member call. The bare-ctor spelling inside an instantiation list
/// (`twice[Opt i64]`) stays gated (p9g: the shared arity gate), which is why
/// the HKT-bound word `twice` in golden #4 is called bare.
#[test]
fn s3t_explicit_instantiation_spelling_unchanged() {
    let src = "\
type: Opt['T] | None | Some 'T ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Opt
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Opt? ;
;
: q['T 'U] ( 'T [ 'T -- 'U ] -- 'U ) call ;
: mkopt ( i64 -- Opt[i64] ) Some ;
: showopt ( Opt[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Opt? ;
: main ( -- )
  1 mkopt [ 1 sub ] map[i64 i64] showopt
  2 [ 1 sub ] q[i64 i64] . ;
";
    let (_t, entry) = single_file_hosted("w6-s3t-explicit-instantiation", src);
    let out = build_and_run(&entry);
    // The member call maps 1 -> 0 (prints 0); the explicitly-instantiated
    // `q[i64 i64]` applies `[ 1 sub ]` to 2 (prints 1).
    assert_eq!(out, "0\n1\n");
}

// Non-regression (W6, suite-level): `tests/phase7b_slice1.rs` staying green
// is the trio's third leg -- covered by the full `cargo test` run, not by a
// test in this file.
