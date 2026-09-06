//! P7b.S8 Phase 1 exit goldens: the `member_shape_is_supported` `Generic`
//! arm lift (REQ-1/REQ-2) plus the new `core::iterator` protocol module
//! (`Step`, `Iterator`, `impl: Iterator for List`, REQ-3/REQ-4/REQ-NFR3).
//! Harness style from `tests/phase7b_slice4.rs`/`tests/phase7b_slice6.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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
