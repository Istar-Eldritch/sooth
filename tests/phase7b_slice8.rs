//! P7b.S8 exit goldens. Phase 1: the `member_shape_is_supported` `Generic`
//! arm lift (REQ-1/REQ-2) plus the new `core::iterator` protocol module
//! (`Step`, `Iterator`, `impl: Iterator for List`, REQ-3/REQ-4/REQ-NFR3).
//! Phase 2: the consumers written once against the bound (`for_each`/`fold`,
//! REQ-5/REQ-6) and the linearity teeth (REQ-NFR4).
//! Phase 3: the S2-6 lift (`impl: Iterator for Range[i64]`, REQ-7/REQ-9) and
//! its fences (REQ-8), with `Range` shipped in `core::range`.
//! Phase 4: the REQ-11 IR pin (one-frame loop with a back-edge, `next` a real
//! frame — facts only, no fusion verdict; REQ-10/REQ-12/REQ-13 verification).
//! Harness style from `tests/phase7b_slice4.rs`/`tests/phase7b_slice6.rs`.

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
            std::env::temp_dir().join(format!("sooth-p7bs8-{}-{tag}-{seq}", std::process::id()));
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
            "package: p7bs8 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

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

/// Build `src` and assert it succeeds, discarding the binary and its output.
/// Used by the admission/placement checks below, which only care that the
/// build does not fail (or, for the panic-fence case, does not panic).
fn build_ok(tag: &str, src: &str) {
    let (_t, entry) = single_file_hosted(tag, src);
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
}

/// Build `src`, assert it fails *located* (a clean non-zero exit with a
/// `error: ...` stderr), and return that stderr. Distinct from a panic: a
/// panicking build also exits non-zero, so every caller additionally checks
/// `!stderr.contains("panicked")` -- the admission-safety sweep's whole
/// point (REQ-1's own risk note) is that a newly-admitted shape may fail,
/// but must never panic.
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
        "the admission-safety sweep must never panic, got: {stderr}"
    );
    stderr
}

/// (a) REQ-1/REQ-3/REQ-4: the protocol *shape* (the P8-4a3 shape) -- a
/// `Step`-row `Iterator` trait declared and implemented for `List` inline in
/// the fixture module, deliberately an own copy rather than an import of
/// `core::iterator` so the gate-lift goldens do not depend on lib
/// registration; the shipped module is exercised end-to-end by
/// `core_iterator_module_drains_a_list_through_its_cross_module_impl`. A
/// mono call to `next` drains a 3-element `List[i64]` all the way to `Done`.
/// Fails to build pre-lift (`Option['T]`/`Step[...]`'s ctor-headed row was
/// rejected); the lift admits it and the List impl runs with no S6-wall
/// panic (`Done` needs no remainder construction).
#[test]
fn iterator_trait_declares_and_list_next_drains_at_a_mono_call_site() {
    let src = "\
import: core::list * ;
type: Step['T 'Rest] | Done | More 'T 'Rest ;
trait: Iterator['It: * -> *] :
  next ( 'It['T] -- Step['T 'It['T]] ) ;
;
impl: Iterator for List
  : next
    ~[ ( Nil ) drop Done ]
    ~[ ( Cons ) Cons> | v rest | v rest ^> More ]
    List? ;
;
: drain ( List[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> | v rest | v . rest drain ]
  Step? ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  drain ;
";
    let (_t, _binary, stdout) = build_run_keep("p1-iterator-list-drain", src);
    assert_eq!(stdout, "1\n2\n3\n");
}

/// (b) REQ-2: a plain-slot member-local-headed App (`'G['T]`, non-nested)
/// still raises `unsupported_trait_member_shape_error` with the S7-reworded
/// text, byte-exact (measured from the live binary, not the spec's
/// transcription).
#[test]
fn plain_slot_member_local_headed_app_is_unsupported_shape_error() {
    let stderr = build_error_located(
        "p1-plain-slot-local-app",
        "trait: Functor['F: * -> *] : m ( 'G['T] -- ) ; ;\n: main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application, in a plain slot or a quotation row -- are supported)"
        ),
        "{stderr}"
    );
    // Located at the member (`m`), not just named -- pins REQ-4's "located,
    // not silent" (the S7 precedent, `tests/phase7b_slice7.rs:169`).
    assert!(stderr.contains("line 3, col 30"), "{stderr}");
}

/// (b) REQ-2: a ctor-headed row **containing** a member-local-headed App
/// argument (`Step['G['T] 'F['T]]`) stays fenced post-lift -- the lifted
/// `Generic` arm's own recursion into args keeps this rejected, same
/// message as the plain-slot case above.
#[test]
fn ctor_headed_row_containing_local_headed_app_argument_is_unsupported_shape_error() {
    let stderr = build_error_located(
        "p1-ctor-row-local-app-arg",
        "type: Step['T 'Rest] | Done | More 'T 'Rest ;\n\
         trait: Functor['F: * -> *] : m ( Step['G['T] 'F['T]] -- ) ; ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application, in a plain slot or a quotation row -- are supported)"
        ),
        "{stderr}"
    );
    // Located at the member (`m`), not just named -- pins REQ-4's "located,
    // not silent" (the S7 precedent, `tests/phase7b_slice7.rs:203`).
    assert!(stderr.contains("line 4, col 30"), "{stderr}");
}

/// (b) REQ-2: a row-nested member-local-headed App (`'G['U]`) still raises
/// `app_in_member_quotation_row_error` (S7's G4b precedent), byte-exact.
#[test]
fn row_nested_member_local_headed_app_is_quotation_row_error() {
    let stderr = build_error_located(
        "p1-row-nested-local-app",
        "trait: Functor['F: * -> *] : m ( [ 'G['U] -- ] -- ) ; ;\n: main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "applies a type variable inside a quotation row (`'G[...]` may not appear inside `[ ... ]` unless `'G` is the trait's own type variable)"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "note: an App inside a quotation row must be headed by the trait's own type variable"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("at line 3, col 30"),
        "the S7 twin pins locatedness; this pin must too: {stderr}"
    );
}

/// (b) REQ-2: a nested `'F['G['T]]` member row (trait-var-headed outer App
/// containing a member-local-headed nested App) is legal today
/// (`ground_member_poly`'s `head != 0` arm, `src/ast.rs:2346-2351`) and
/// stays legal -- REQ-1's lift does not touch this shape, it is not on the
/// reject list.
#[test]
fn nested_trait_var_headed_app_over_member_local_headed_app_still_builds() {
    build_ok(
        "p1-nested-trait-var-over-local",
        "trait: Functor['F: * -> *] : m ( 'F['G['T]] -- ) ; ;\n: main ( -- ) ;\n",
    );
}

/// (c) admission-safety sweep: one newly-admitted non-Iterator shape (the
/// probe's Interlude-B `Box2['T]` member row -- a ctor-headed output over a
/// dispatchable `'F['T]` input) either grounds or fails located -- never
/// panics. The body is deliberately **empty** (not a deliberately-wrong
/// body of the right shape) so the mismatch names the grounded ctor-headed
/// output itself, proving `Box2['T]` actually grounded to `Box2['ctor0]`
/// rather than merely asserting *some* body-level mismatch fired.
#[test]
fn admitted_non_iterator_ctor_headed_shape_grounds_or_fails_located_never_panics() {
    let stderr = build_error_located(
        "p1-admission-sweep-box2",
        "import: core::list * ;\n\
         type: Box2['T] v 'T ;\n\
         trait: Wrap['F: * -> *] : wrap ( 'F['T] -- Box2['T] ) ; ;\n\
         impl: Wrap for List\n\
           : wrap ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains("body leaves `List['ctor0]`, but the declared outputs are `Box2['ctor0]`"),
        "{stderr}"
    );
}

/// (c) REQ-1: type arguments only -- a `Len::Var` among a ctor application's
/// own length arguments (a `Buf['T 'N]`-shaped member row) stays rejected
/// located with the same unsupported-shape message, mirroring the Array
/// arm's own `Len::Var` refusal.
#[test]
fn len_var_ctor_application_argument_stays_rejected() {
    let stderr = build_error_located(
        "p1-admission-sweep-buf-len-var",
        "type: Buf['T 'N: Len] data array['T 'N] ;\n\
         trait: Bufify['F: * -> *] : bufify ( 'F['T] -- Buf['T 'N] ) ; ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application, in a plain slot or a quotation row -- are supported)"
        ),
        "{stderr}"
    );
}

/// (c) REQ-1's panic-fence note: a ctor-headed member row (`Option['T]`)
/// against a **concrete** impl target (`for i64`) is fenced located in
/// `parse_impl_member_body`'s concrete branch, not the `ground_member_type`
/// `unreachable!` (`src/ast.rs:2209-2211`), which must stay unreachable.
/// Message reworded by review finding 5 (new this phase, not S7-frozen): it
/// now names the offending ctor application itself (`Option['T]`) rather
/// than the trait's header variable (`'T`), which for a ctor-headed slot
/// named nothing useful.
#[test]
fn ctor_headed_member_row_over_concrete_impl_target_is_located_not_a_panic() {
    let stderr = build_error_located(
        "p1-admission-sweep-concrete-ctor-panic-fence",
        "type: Option['T] | None | Some 'T ;\n\
         trait: Wrapper['T] : wrap ( 'T -- Option['T] ) ; ;\n\
         impl: Wrapper for i64\n\
           : wrap Some ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "declares the ctor-headed application `Option['T]`, but the impl target `i64` is concrete"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "a ctor-headed member row has no monomorphic representation here (`ground_member_type` grounds concrete/array/reference/quotation shapes only); implement the trait for a constructor target with a type variable instead"
        ),
        "{stderr}"
    );
}

/// (c) review finding 1/2: a bound over a trait with a ctor-headed member
/// row, instantiated at a type with no impl, fails located via the
/// `try_ground_member_type` `Generic` fallback (`src/ast.rs:2233` region) --
/// not the `ground_member_type` `unreachable!` (`src/ast.rs:2209`), which
/// this admission-safety sweep never exercised before (the sweep only
/// probed declaration/desugar paths, not a bound over a newly-admitted
/// trait). HKT shape: `core::iterator`'s own shipped `Iterator` trait, bound
/// at a non-iterator type -- this is the exact repro that panicked in
/// review before the `Generic => None` fallback landed.
#[test]
fn bound_over_ctor_headed_member_trait_at_unimplemented_hkt_type_is_located_not_a_panic() {
    let stderr = build_error_located(
        "p1-bound-unsatisfied-ctor-headed-hkt",
        "import: core::iterator * ;\n\
         type: Box['T] v 'T ;\n\
         : mkbox ( -- Box[i64] ) 1 Box ;\n\
         : f ['It: Iterator 'T] ( 'It['T] -- ) drop ;\n\
         : main ( -- ) mkbox f ;\n",
    );
    assert!(
        stderr.contains("does not satisfy `Iterator`: no `( 'It['T] -- Step['T 'It['T]] )` found"),
        "{stderr}"
    );
    assert!(
        stderr.contains("(line 7, col 21)"),
        "locatedness: the bound-unsatisfied diagnostic names its site: {stderr}"
    );
}

/// (c) review finding 1/2: the non-HKT twin -- a plain (non-`* -> *`) trait
/// whose member row is ctor-headed, bound at a concrete type with no impl.
/// Same fallback, same never-a-panic guarantee, over the simpler kind.
#[test]
fn bound_over_ctor_headed_member_trait_at_unimplemented_concrete_type_is_located_not_a_panic() {
    let stderr = build_error_located(
        "p1-bound-unsatisfied-ctor-headed-concrete",
        "type: Option['T] | None | Some 'T ;\n\
         trait: Wrapper['T] : wrap ( 'T -- Option['T] ) ; ;\n\
         : f ['T: Wrapper] ( 'T -- 'T ) ;\n\
         : main ( -- ) 1 f drop ;\n",
    );
    assert!(
        stderr.contains("does not satisfy `Wrapper`: no `( i64 -- Option['T] )` found"),
        "{stderr}"
    );
    assert!(
        stderr.contains("(line 6, col 17)"),
        "locatedness: the bound-unsatisfied diagnostic names its site: {stderr}"
    );
}

/// (d) REQ-4/REQ-NFR3: the shipped `core::iterator` module, end to end. A
/// consumer imports the trait and its ctors over the measured surface (the
/// trait name + `Step`/`Done`/`More`, never the member name -- bare `next`
/// resolves through trait dispatch) and drains a `List[i64]` built from
/// `core::list`, so the module's own `impl: Iterator for List` is what runs.
/// This also pins the cross-module placement rule: `check_impl_decls`'
/// orphan rule accepts an own-module-trait impl of a foreign (`core::list`)
/// type (`src/check/declarations.rs:588`'s trait-module arm, the `cmp.sth`
/// `impl: Ord for i64` precedent).
#[test]
fn core_iterator_module_drains_a_list_through_its_cross_module_impl() {
    let src = "\
import: core::list | List Nil Cons | ;
import: core::iterator | Step Done More Iterator | ;
: drain ( List[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> | v rest | v . rest drain ]
  Step? ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  2 mkempty ^ Cons
  1 swap ^ Cons
  drain ;
";
    let (_t, _binary, stdout) = build_run_keep("p1-core-iterator-cross-module-drain", src);
    assert_eq!(stdout, "1\n2\n");
}

/// (phase 2, REQ-5/REQ-6) `for_each` written once against the `Iterator`
/// bound -- it names no List word -- drains a `List[i64]` built 1,2,3,
/// printing `1\n2\n3`. This is REQ-6's answer in golden form: the member
/// call `next` inside a poly body, dispatched through the bound over the
/// ctor-headed compound return `Step['T 'It['T]]`, is *not* fenced by
/// `poly_cross_call_unsupported_error` (`src/check/poly.rs:4371`). The
/// import line is the measured surface for a pure consumer: `for_each`
/// alone, no trait and no ctors, since this fixture writes no dispatch arm.
#[test]
fn for_each_drains_a_list_through_the_iterator_bound() {
    let src = "\
import: core::list | List Nil Cons | ;
import: core::iterator | for_each | ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ . ] for_each ;
";
    let (_t, _binary, stdout) = build_run_keep("p2-for-each-list", src);
    assert_eq!(stdout, "1\n2\n3\n");
}

/// (phase 2, REQ-5) the generic `fold` over the same List sums to 6. The
/// spelling is the house one -- accumulator under the element, so the
/// quotation is `[ 'A 'T -- 'A ]` and the call reads `0 [ add ] fold`
/// (`lib/core/combinators.sth:53`). `[ 0 add ]` is *not* an alternative
/// spelling: it cannot type against that row.
#[test]
fn fold_sums_a_list_through_the_iterator_bound() {
    let src = "\
import: core::list | List Nil Cons | ;
import: core::iterator | fold | ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  0 [ add ] fold . ;
";
    let (_t, _binary, stdout) = build_run_keep("p2-fold-list", src);
    assert_eq!(stdout, "6\n");
}

/// (phase 2, REQ-NFR4) the linearity teeth. This fixture is
/// `lib/core/iterator.sth`'s shipped `for_each` verbatim but for one
/// deleted `drop`: its `Done` arm leaves the `Step` shell on the stack.
/// That is a located compile error, so the protocol's "the final drop lives
/// in one canonical site" is enforced, not merely intended -- the passing
/// twin is `for_each_drains_a_list_through_the_iterator_bound` above.
///
/// The live text is the *variant-escape* rule (the poly arm the spec names,
/// `src/check/poly.rs:11551`), not the quotation-shape join
/// (`src/check.rs:2983`): the escape check runs first and catches the
/// undropped shell before the arms' shapes are joined. Pinned byte-exact
/// from the binary, including the line, which locates the offending arm
/// (line 8 is `~[ ( Done ) ]`, counting the harness's two prelude lines).
#[test]
fn consumer_arm_leaving_the_step_shell_undropped_is_a_compile_error() {
    let stderr = build_error_located(
        "p2-teeth-undropped-step-shell",
        "import: core::list | List Nil Cons | ;\n\
         import: core::iterator | Step Done More Iterator | ;\n\
         : leaky ['It: Iterator 'T] ( 'It['T] [ 'T -- ] -- )\n\
           | f |\n\
           next\n\
           ~[ ( Done ) ]\n\
           ~[ ( More ) More> | v rest | v f call rest f leaky ]\n\
           Step? ;\n\
         : main ( -- ) Nil [ drop ] leaky ;\n",
    );
    assert!(
        stderr.contains(
            "error: an arm of `Step?` leaves `Step.Done` on the stack in `leaky` (line 8)\n  a variant-typed value is reachable only inside the arm that bound it; consume it there, or leave its fields instead"
        ),
        "{stderr}"
    );
}

/// (phase 3, REQ-7/REQ-9) the phase's first artifact and its whole
/// justification: `next` over `Range[i64]` dispatches at a **plain mono call
/// site**. `Range[i64]` is a fully-applied all-concrete ctor impl target --
/// the S2-6 shape that was a located rejection before Delta B ("the impl
/// target `Range[i64]` is concrete") -- and its App-headed member now
/// grounds as a monomorphic instantiation. Both arms are observable: two
/// `More` steps print `0` and `1`, then the exhausted `Done` arm prints
/// `999`, so the drain is not merely "does not crash".
///
/// This shape panicked `resolve_mono_member_call`'s else-branch
/// `debug_assert!` ("a dispatched impl's member word is always in the
/// whole-program poly_env") before the lifted-target arm landed: the member
/// word is `poly: None`, and `poly_env` holds only `poly: Some` words.
#[test]
fn range_next_dispatches_at_a_mono_call_site() {
    let src = "\
import: core::iterator | Step Done More Iterator | ;
import: core::range | Range | ;
: report ( Range[i64] -- )
  next
  ~[ ( Done ) drop 999 . ]
  ~[ ( More ) More> | v rest | v . rest report ]
  Step? ;
: main ( -- ) 0 2 Range report ;
";
    let (_t, _binary, stdout) = build_run_keep("p3-range-mono-call-site", src);
    assert_eq!(stdout, "0\n1\n999\n");
}

/// (phase 3, REQ-10 in golden form) the consumers written once against the
/// bound drain and fold `Range[i64]` with no per-impl copy: `0\n1\n2`, and
/// `0 [ add ] fold` sums it to 3. Dispatch here is the *bound* path, not the
/// mono one: `for_each`'s `'It` is bound to a `Type::CtorImage`, so
/// `resolve_user_bound`'s ctor-image arm serves the site -- and a lifted
/// target's mono member word dispatches under its **bare** symbol there,
/// because there is nothing to monomorphize (`impl_mono_seed` only ever
/// seeds `poly: Some` words). Without that, the build failed with
/// "`impl: Iterator for Range` binds no word for member `next`".
#[test]
fn for_each_and_fold_drain_a_range_through_the_iterator_bound() {
    let src = "\
import: core::iterator | for_each fold | ;
import: core::range | Range | ;
: main ( -- )
  0 3 Range [ . ] for_each
  0 3 Range 0 [ add ] fold . ;
";
    let (_t, _binary, stdout) = build_run_keep("p3-range-through-the-bound", src);
    assert_eq!(stdout, "0\n1\n2\n3\n");
}

/// (phase 3, REQ-8) the first fence pin: a **plain** concrete target keeps
/// today's byte-exact S2-6 message. `impl: Iterator for i64` names no
/// constructor at all, so the trait's App-headed `next` still has no
/// monomorphic representation there -- the lift admits ctor applications,
/// not concreteness.
#[test]
fn app_headed_member_over_plain_concrete_target_still_raises_the_s2_6_error() {
    let stderr = build_error_located(
        "p3-fence-plain-concrete-target",
        "import: core::iterator | Step Done More Iterator | ;\n\
         impl: Iterator for i64\n\
           : next drop Done ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "error: trait member `next` of `Iterator` (line 5, col 3) applies the trait variable `'It`, but the impl target `i64` is concrete\n  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead"
        ),
        "{stderr}"
    );
}

/// (phase 3, REQ-8) the second fence pin: an **App-headed** impl target
/// (`for 'F['T]`, a constructor-abstract target) still raises
/// `impl_target_app_unsupported_error` byte-exact. Delta B intercepts the
/// target *fold*, never this fence, which sits after it.
#[test]
fn app_headed_impl_target_still_raises_the_unsupported_target_error() {
    let stderr = build_error_located(
        "p3-fence-app-headed-target",
        "import: core::iterator | Step Done More Iterator | ;\n\
         impl: Iterator for 'F['T]\n\
           : next drop Done ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "error: an `impl:` target may not apply its own type variable (`'F[...]` at line 4, col 20); a constructor-abstract impl target is not supported this slice"
        ),
        "{stderr}"
    );
}

/// (phase 3, REQ-8) the third fence pin, in runnable form: a **mixed**
/// (partially-applied) target keeps the generic path -- its padded slot is a
/// variable, so its member word stays polymorphic and dispatches through the
/// existing generic machinery. `Pair[i64 'ctor1]` here implements the
/// non-HKT `Peek`, and the golden proves the pre-lift route still runs end
/// to end. (The parse-level twin is
/// `parse_impl_target_plain_and_partial_shapes_are_not_lifted`,
/// `src/parser.rs`.)
#[test]
fn partially_applied_ctor_target_still_dispatches_through_the_generic_path() {
    let src = "\
type: Pair['A 'B] a 'A b 'B ;
trait: Peek['T] : peek ( 'T -- i64 ) ; ;
impl: Peek for Pair[i64]
  : peek drop 7 ;
;
: mkpair ( i64 i64 -- Pair[i64 i64] ) Pair ;
: main ( -- ) 7 8 mkpair peek . ;
";
    let (_t, _binary, stdout) = build_run_keep("p3-partial-target-generic-path", src);
    assert_eq!(stdout, "7\n");
}

/// (phase 3 review, finding 1) the ICE repro itself: a lifted-target
/// member's grounded signature can itself be `PolyType::Quotation` (`ap`'s
/// `~[ 'T -- ]` parameter, its `'T` identified with `Box[i64]`'s own
/// concrete argument through the dispatchable-input union), and
/// `ground_mono_member_slots` (`src/parser.rs`) hands that var-free
/// quotation slot to `ground_var_free` -> `substitute_generic_field`
/// (`src/ast.rs`), which had no `Quotation` arm and panicked at its
/// wildcard `unreachable!`. Declaration only, no call site -- the panic
/// fired grounding the member body's own effect, before any caller exists.
/// Fixed by giving `substitute_generic_field` a `Quotation` arm mirroring
/// `ground_member_type`'s S2-3 arm (honoring `is_inline`). The member's
/// `~[ 'T -- ]` (an inline-quotation parameter) additionally requires `ap`
/// itself to declare `inline` -- a real, located, pre-existing rule
/// (`declares_inline`, `src/parser.rs`) the trait member here does not
/// satisfy, so the fixed grounding reaches that ordinary diagnostic rather
/// than building.
#[test]
fn lifted_target_member_with_inline_quotation_slot_grounds_mono_not_panics() {
    let stderr = build_error_located(
        "p3-quotation-slot-mono-ground",
        "type: Box['T] v 'T ;\n\
         trait: Apply['T] : ap ( 'T ~[ 'T -- ] -- ) ; ;\n\
         impl: Apply for Box[i64]\n\
           : ap | q | | b | b q call ;\n\
         ;\n",
    );
    assert!(
        stderr.contains(
            "error: word `ap` (member of trait `Apply` for `Box[i64]`) declares an inline-quotation parameter `~[ Box[i64] -- ]` but is not `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare `inline` (line 6, col 3)"
        ),
        "{stderr}"
    );
}

/// (phase 3 review, finding 1) the non-lifted twin: the identical shape over
/// a hand-written concrete struct (`for Pt`, not a ctor-application target)
/// reaches the same diagnostic, byte for byte but for the target name --
/// proof the fix restores parity with the pre-lift concrete route rather
/// than special-casing the lifted one.
#[test]
fn inline_quotation_slot_over_plain_concrete_target_reaches_the_same_diagnostic() {
    let stderr = build_error_located(
        "p3-quotation-slot-plain-concrete",
        "type: Pt x i64 y i64 ;\n\
         trait: Apply2['T] : ap2 ( 'T ~[ 'T -- ] -- ) ; ;\n\
         impl: Apply2 for Pt\n\
           : ap2 | q | | b | b q call ;\n\
         ;\n",
    );
    assert!(
        stderr.contains(
            "error: word `ap2` (member of trait `Apply2` for `Pt`) declares an inline-quotation parameter `~[ Pt -- ]` but is not `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare `inline` (line 6, col 3)"
        ),
        "{stderr}"
    );
}

/// (phase 3 review, finding 1) the HKT twin, literally as the review named
/// it: an App-headed slot (`'It['T]`) at the same top-level position that
/// *identifies* `'T` with the target's own concrete argument (`Box[i64]`'s
/// `i64`) -- so the whole signature grounds var-free, exactly like
/// `core::iterator`'s `next`/`Range[i64]` (REQ-7's own golden,
/// `range_next_dispatches_at_a_mono_call_site` above). This reaches an
/// ordinary, located, mono type error (never a panic, never the S2-6 fence):
/// grounding a var-free HKT member dispatches mono by design, the same as
/// any other identified local.
#[test]
fn hkt_member_with_identified_local_grounds_mono_reaches_ordinary_type_error() {
    let stderr = build_error_located(
        "p3-hkt-identified-local-mono",
        "type: Box['T] v 'T ;\n\
         trait: Each['It: * -> *] : each ( 'It['T] [ 'T -- ] -- ) ; ;\n\
         impl: Each for Box[i64]\n\
           : each | q | | b | b q call ;\n\
         ;\n",
    );
    assert!(
        stderr.contains(
            "error: type mismatch in `each` (member of trait `Each` for `Box[i64]`) (line 6)\n  `call` expected `i64`, found `Box[i64]`"
        ),
        "{stderr}"
    );
}

/// (phase 3 review, finding 1) the HKT twin that genuinely reaches the S2-6
/// fence: `'U` never occurs in an identifying (dispatchable-input App
/// argument) position, so the union build appends it unbound, the grounded
/// signature is *not* var-free, and the lifted-target route falls through
/// to the plain concrete-target checks below it -- the same fence
/// `app_headed_member_over_plain_concrete_target_still_raises_the_s2_6_error`
/// pins for a non-ctor target, here firing for a ctor one instead. Proof the
/// fence survives the lift for the shape it actually guards (an
/// ungroundable member local), not just the identified-local shape above.
#[test]
fn hkt_member_with_unidentified_local_still_raises_the_s2_6_fence() {
    let stderr = build_error_located(
        "p3-hkt-unidentified-local-fence",
        "type: Box['T] v 'T ;\n\
         trait: Each['It: * -> *] : each ( 'It['T] ~[ 'U -- ] -- ) ; ;\n\
         impl: Each for Box[i64]\n\
           : each | q | | b | b q call ;\n\
         ;\n",
    );
    assert!(
        stderr.contains(
            "error: trait member `each` of `Each` (line 6, col 3) applies the trait variable `'It`, but the impl target `Box[i64]` is concrete\n  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead"
        ),
        "{stderr}"
    );
}

/// (phase 4, REQ-11) the IR evidence pin, recorded facts only -- no fusion
/// verdict. Captured through `driver::emit_ssa_with_manifest`
/// (`src/driver.rs:897`) rather than `ir::lower` directly: this fixture
/// `import:`s the real `core::iterator`/`core::range` library modules, and
/// assembling a real import closure is a `driver`-internal step
/// (`assemble_module` is `pub(crate)`) that only the `emit_ssa*` entry
/// points expose to an external test crate; `poly_self_tail_call_lowers_to_
/// loop_back_edge` (`src/ir/driver.rs:1093`) is the pattern this pin
/// follows, not its placement, so `src/ir/` itself stays diff-empty
/// (REQ-NFR2).
///
/// The consuming loop is `for_each`'s monomorphized instantiation over
/// `Range[i64]`: its self-call in `for_each`'s own `More` arm sits in tail
/// position (`lib/core/iterator.sth`'s `for_each`), so P7.S3g's transform
/// applies exactly as it does for an ordinary self-tail poly word -- one
/// emitted function, no `call` back into itself, and a backward `jmp`
/// closing the loop. `next` over `Range[i64]` is asserted as its own,
/// separate emitted function, called (not inlined/spliced) from inside the
/// loop: the P8-6 fact that the consuming loop is one frame while `next` is
/// a real monomorphized frame of its own, not a claim about whether the two
/// fuse.
#[test]
fn consuming_loop_over_range_is_one_frame_with_a_back_edge_and_next_is_a_real_frame() {
    let src = "\
import: core::iterator | for_each | ;
import: core::range | Range | ;
: main ( -- )
  0 3 Range [ drop ] for_each ;
";
    let path = std::env::temp_dir().join(format!("sooth-p7bs8-ir-pin-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing the fixture should succeed");
    let ssa = sooth::driver::emit_ssa_with_manifest(&path, common::manifest_for(&path).as_deref())
        .unwrap_or_else(|e| panic!("emitting the fixture should succeed: {e}"));
    std::fs::remove_file(&path).ok();

    // Every `export function`/`function` header line in the emitted module,
    // paired with its whole block (through the closing `\n}\n`), so a header
    // substring search cannot also match an unrelated `call` inside some
    // other function's body.
    fn function_blocks(ssa: &str) -> Vec<&str> {
        let mut header_starts = Vec::new();
        for prefix in ["\nexport function ", "\nfunction "] {
            let mut idx = 0;
            while let Some(rel) = ssa[idx..].find(prefix) {
                header_starts.push(idx + rel + 1);
                idx += rel + prefix.len();
            }
        }
        header_starts.sort_unstable();
        header_starts
            .into_iter()
            .map(|start| {
                let rel_end = ssa[start..]
                    .find("\n}\n")
                    .expect("every emitted function block closes");
                &ssa[start..start + rel_end + 3]
            })
            .collect()
    }
    fn header_line(b: &str) -> &str {
        b.lines().next().unwrap_or("")
    }
    fn only_block_with<'a>(blocks: &[&'a str], needles: &[&str]) -> &'a str {
        let hits: Vec<&&str> = blocks
            .iter()
            .filter(|b| needles.iter().all(|n| header_line(b).contains(n)))
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one emitted function header containing {needles:?}, found {}: {:?}",
            hits.len(),
            blocks.iter().map(|b| header_line(b)).collect::<Vec<_>>()
        );
        hits[0]
    }

    let blocks = function_blocks(&ssa);

    // The consuming loop: `for_each`'s monomorphized instantiation over
    // `Range[i64]` -- one emitted function.
    let loop_fn = only_block_with(&blocks, &["for_each", "Range"]);
    let loop_symbol = loop_fn
        .lines()
        .next()
        .unwrap()
        .split(|c: char| c == '(' || c.is_whitespace())
        .find(|tok| tok.starts_with('$'))
        .expect("the header line names the function's own symbol");
    assert!(
        !loop_fn.contains(&format!("call {loop_symbol}(")),
        "the loop must not recurse by call, its self-call is the back-edge: {loop_fn}"
    );
    // The entry falls into `@blk1` once (`@start`'s own `jmp`); a *second*
    // `jmp @blk1` appearing after the `@blk1:` label itself is the back-edge.
    let header_pos = loop_fn
        .find("\n@blk1\n")
        .expect("the loop opens a `@blk1` header block");
    assert!(
        loop_fn[header_pos..].contains("\tjmp @blk1\n"),
        "the loop must back-edge to its header block: {loop_fn}"
    );

    // `next` over `Range[i64]`: its own separate, real monomorphized frame,
    // called (not spliced) from inside the loop above.
    let next_fn = only_block_with(&blocks, &["next", "Range"]);
    assert_ne!(
        next_fn, loop_fn,
        "`next` must be a distinct frame from the consuming loop"
    );
    assert!(
        next_fn.lines().count() > 3,
        "`next` must be a real body, not a trivial/spliced stub: {next_fn}"
    );
    let next_symbol = next_fn
        .lines()
        .next()
        .unwrap()
        .split(|c: char| c == '(' || c.is_whitespace())
        .find(|tok| tok.starts_with('$'))
        .expect("the header line names the function's own symbol");
    assert!(
        loop_fn.contains(&format!("call {next_symbol}(")),
        "the loop must call `next` as a real function, not inline it: {loop_fn}"
    );
}

/// P7b.S8 integrated review: the three behavioral claims the write-downs make
/// about the module surface, pinned so a regression cannot ship silently.
/// (1) `fold` is exported by both `core::combinators` (array fold) and
/// `core::iterator`; wildcard-importing both fails closed with a
/// duplicate-binding error at the import site, never a silent shadow.
#[test]
fn wildcard_importing_both_fold_exporters_fails_closed_at_the_import_site() {
    let stderr = build_error_located(
        "p4-fold-wildcard-collision",
        "import: intrinsics * ;\n\
         import: core::combinators * ;\n\
         import: core::iterator * ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains(
            "error: wildcard import of `fold` (line 5, col 1) collides with the wildcard import of `fold`"
        ),
        "{stderr}"
    );
}

/// (2) The import-surface rule: a trait member's synthesized word is never
/// the bare member name, so naming `next` in an import list fails located —
/// import the trait and call the bare name at the dispatch site instead.
#[test]
fn naming_the_member_in_an_import_list_is_not_exported_error() {
    let stderr = build_error_located(
        "p4-next-not-exported",
        "import: intrinsics * ;\n\
         import: core::iterator | Step Done More Iterator next | ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        stderr.contains("error: `next` is not exported from module `iterator` at line 4, col 50"),
        "{stderr}"
    );
}

/// (3) The other half of the import surface: a consumer that never imports
/// `core::iterator` cannot resolve `next` at all — `unknown word`, located.
#[test]
fn bare_next_without_the_iterator_import_is_unknown_word_error() {
    let stderr = build_error_located(
        "p4-next-unknown-word",
        "import: intrinsics * ;\n\
         import: core::list | List Nil Cons | ;\n\
         : f ( List[i64] -- ) next drop ;\n\
         : main ( -- ) Nil f ;\n",
    );
    assert!(
        stderr.contains("error: unknown word `next` in `f` (line 5)"),
        "{stderr}"
    );
}

/// P7b.S8 integrated review: the positive twin of the Quotation-arm pins —
/// an inline-declared lifted member whose quotation slot GROUNDS. The trait
/// row carries `inline` (impl members inherit it); the member body checks
/// through `substitute_generic_field`'s Quotation arm (the phase-3 ICE fix's
/// build path), and the impl builds. The member cannot yet be *called* with
/// a quotation (the P7-era rule that only `call` accepts one fires at the
/// call site, pre-existing, out of S8's scope) — this pin witnesses the
/// grounding/build path the error twins don't reach.
#[test]
fn inline_declared_lifted_member_with_quotation_slot_builds_not_panics() {
    build_ok(
        "p4-inline-quotation-slot-build-path",
        "import: intrinsics * ;\n\
         type: Box['T] v 'T ;\n\
         trait: Apply['T] : ap inline ( 'T ~[ 'T -- ] -- ) ; ;\n\
         impl: Apply for Box[i64]\n\
           : ap | q | | b | b q call ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
}
