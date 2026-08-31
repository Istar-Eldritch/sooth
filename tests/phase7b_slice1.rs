//! P7b.S1 exit goldens: kinds and type-level application. This file starts
//! with the Phase 2 goldens (application parsing + compile-forcing); later
//! phases add to it. Driven through the real `sooth` binary, styled after
//! `tests/phase7_slice6a.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs1-{}-{tag}-{seq}", std::process::id()));
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

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// The k8 flip (S1-3/S1-5): `array['T 'N]` with no `'N: Len` annotation
/// anywhere -- `'N`'s kind is inferred from the count position it appears
/// in, so the program builds and runs instead of S6a's mandatory-annotation
/// rejection.
#[test]
fn hkt_len_var_inferred_from_count_position_is_accepted() {
    let src = "\
        : sum['T 'N] ( array['T 'N] -- usize ) len swap drop ;\n\
        : main ( -- )\n          0 >u8 4 fill sum .\n        ;\n";
    let (_t, entry) = single_file("k8-flip", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "4\n");
}

/// S1-6 router (both readings in one effect): `'F['T]` is an application,
/// `[ 'T -- 'U ]` right after it is the *next* slot -- a declared quotation
/// parameter, not part of the application. Pins the router: today's F1
/// error at the first `[` must be gone.
#[test]
fn hkt_var_before_quotation_parameter_still_parses() {
    // The output is dropped rather than declared `'F['U]` (S1-11's App
    // grounding, which would let a body genuinely construct one, is Phase
    // 3): what this golden pins is the *parse*, so the body only needs to
    // consume both readings, not produce an application value.
    let src = "\
: fmap['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- ) drop drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("router", src);
    build_ok(&entry);
}

/// Non-regression: a declared quotation parameter and an S3t-style explicit
/// instantiation still parse -- the router must not disturb either existing
/// shape.
#[test]
fn hkt_concrete_generic_effect_and_explicit_instantiation_unchanged() {
    let src = "\
: q['T 'U] ( 'T [ 'T -- 'U ] -- 'U ) call ;
: pairwise ( 'T 'U -- ) drop drop ;
: main ( -- ) 1 2.5 pairwise[i64 f64] ;
";
    let (_t, entry) = single_file("nonregression", src);
    build_ok(&entry);
}

// ---- Phase 3: checking and grounding ----

/// Positive golden #3 (S1-9 consumer): `'F`'s kind is inferred purely from
/// its application-head usage in the effect -- no bare `'F` anywhere, no
/// annotation.
#[test]
fn hkt_var_kind_inferred_from_application_head_alone() {
    let src = "\
: pass['F 'T] ( 'F['T] -- 'F['T] ) ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-inferred", src);
    build_ok(&entry);
}

/// Positive golden #4: the same signature, with an explicit `* -> *`
/// annotation confirming the inferred kind -- the annotation-fallback
/// criterion.
#[test]
fn hkt_explicit_annotation_confirms_inferred_kind() {
    let src = "\
: pass['F: * -> * 'T] ( 'F['T] -- 'F['T] ) ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-annotated", src);
    build_ok(&entry);
}

/// Kind error #1 (S1-15.a, W3): a `Star`-kind variable (bound bare) applied
/// like a type constructor.
#[test]
fn hkt_star_var_applied_like_constructor_is_located_error() {
    let src = "\
: bad['F 'T] ( 'F 'F['T] -- ) drop drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-error-a", src);
    let err = build_error(&entry);
    assert!(
        err.contains("is applied like a type constructor but has kind `*`"),
        "{err}"
    );
    // S1-15's both-spans rule: the origin clause naming where `'F` was
    // bound bare must be present too, not just the misuse clause.
    assert!(err.contains("bound bare at line 2, col 16"), "{err}");
}

/// Kind error #2 (S1-15.b): an arrow-kind variable (established by an
/// earlier application) used bare.
#[test]
fn hkt_arrow_var_used_bare_is_located_error() {
    let src = "\
: bad['F 'T] ( 'F['T] 'F -- ) drop drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-error-b", src);
    let err = build_error(&entry);
    assert!(
        err.contains("is used as a plain type but has kind `* -> *`"),
        "{err}"
    );
    // S1-15's both-spans rule: the origin clause naming the earlier
    // application that established the arrow kind.
    assert!(
        err.contains("from an application of `'F` at line 2, col 16"),
        "{err}"
    );
}

/// Kind error #3 (S1-15.c): an explicit `* -> *` annotation conflicting
/// with a bare usage in the effect.
#[test]
fn hkt_annotation_conflicting_with_usage_is_error() {
    let src = "\
: bad['F: * -> * 'T] ( 'F 'T -- ) drop drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-error-c", src);
    let err = build_error(&entry);
    assert!(
        err.contains("is used as a plain type but is annotated `* -> *`"),
        "{err}"
    );
}

/// Kind error #4 (S1-15.d): an application's arity (2 arguments) conflicts
/// with the arity (1 argument) an earlier application of the same variable
/// already established.
#[test]
fn hkt_application_arity_conflicts_with_inferred_kind_is_error() {
    let src = "\
: bad['F 'T 'U] ( 'F['T] 'F['T 'U] -- ) drop drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-error-d", src);
    let err = build_error(&entry);
    assert!(
        err.contains("applies `'F` to 2 arguments but its kind takes 1"),
        "{err}"
    );
    // S1-15's both-spans rule: the origin clause naming the earlier
    // (1-argument) application that established the kind.
    assert!(err.contains("from `'F[...]` at line 2, col 19"), "{err}");
}

/// Kind error #5 (S1-15.e, header-field twin of S1-15.a): a header field
/// bare-mentions a variable another field of the same header applies like a
/// constructor.
#[test]
fn hkt_header_field_applies_star_var_is_located_error() {
    let src = "\
type: Bad['F 'T] g 'F f 'F['T] ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-error-e", src);
    let err = build_error(&entry);
    assert!(
        err.contains("applies `'F` like a type constructor in `Bad`'s field"),
        "{err}"
    );
}

/// Kind error #6 (S1-15.f): a use-site constructor argument (`Wrap[Nat
/// i64]`) whose kind (`Nat` is `*`) disagrees with the header variable's
/// declared kind (`'F: * -> *`).
#[test]
fn hkt_use_site_ctor_arg_of_wrong_kind_is_error() {
    let src = "\
type: Nat val i64 ;
type: Wrap['F: * -> * 'T] f 'F['T] ;
: main ( -- ) ;
: use ( Wrap[Nat i64] -- ) drop ;
";
    let (_t, entry) = single_file("kind-error-f", src);
    let err = build_error(&entry);
    assert!(
        err.contains("supplies `Nat` for `'F`") && err.contains("a type constructor is required"),
        "{err}"
    );
}

/// Kind error #8 (S1-15.h): an explicit `* -> Len -> *` annotation is
/// unsatisfiable by any type-only application (`'F['T]` supplies only one
/// type argument, never a length).
#[test]
fn hkt_annotation_arity_unsatisfiable_by_application_is_error() {
    let src = "\
: bad['F: * -> Len -> * 'T] ( 'F['T] -- ) drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("kind-error-h", src);
    let err = build_error(&entry);
    assert!(err.contains("is annotated `* -> Len -> *`"), "{err}");
    // S1-15's both-spans rule: the origin clause naming the application
    // whose inferred kind conflicts with the annotation.
    assert!(
        err.contains("is used as an application of kind `* -> *`"),
        "{err}"
    );
    assert!(err.contains("at line 2, col 31"), "{err}");
}

// ---- Phase 4: IR + end-to-end goldens ----

/// Positive golden #1 (W2): `'F['T]` in a signature's input and output
/// positions -- call-site unification binds `'F := Box`, `'T := i64`, and
/// IR lowering resolves the application through the lookup-only
/// `subst_polytype` arm (S1-13).
#[test]
fn hkt_signature_application_passes_through_at_concrete_call_site() {
    let src = "\
type: Box['T] v 'T ;

: pass['F 'T] ( 'F['T] -- 'F['T] ) ;
: mk ( i64 -- Box[i64] ) Box ;
: main ( -- ) 5 mk pass Box> . ;
";
    let (_t, entry) = single_file("w2-app-passthrough", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "5\n");
}

/// Positive golden #2 (W1): a generic struct field typed `'F['T]`
/// monomorphizes to the applied constructor (`Box[i64]`) via S1-8's
/// instantiation semantics for `App` fields. The boxed field (`f`) and the
/// plain field (`t`) hold *distinct* values (5 and 7) -- a field-order bug
/// in the `App` field's instantiation would go unnoticed if both held the
/// same value, since the wrong field's value would still print correctly.
#[test]
fn hkt_struct_field_monomorphizes_to_the_applied_constructor() {
    let src = "\
type: Box['T] v 'T ;
type: Wrap['F 'T] f 'F['T] t 'T ;

: mk ( -- Wrap[Box i64] ) 5 Box 7 Wrap ;
: main ( -- ) mk Wrap> . Box> . ;
";
    let (_t, entry) = single_file("w1-field-monomorphize", src);
    let out = build_and_run(&entry);
    assert_eq!(
        out, "7\n5\n",
        "t (7) prints first, then f's Box payload (5)"
    );
}

// ---- Review fix (P0): App unification against a length-parameterized ctor ----

/// `'F['T]` unified against a call site whose concrete argument is an
/// instantiation of a *length*-parameterized header (`Buf['T 'N: Len]`)
/// used to panic in `apply_subst` (an out-of-bounds index into a length
/// list `unify_poly_input`'s `App` arm minted empty). S1-7 fences `App` to
/// type arguments only, so this must be a located build error, not a crash.
#[test]
fn hkt_app_against_a_length_parameterized_constructor_is_a_located_error_not_a_panic() {
    let src = "\
type: Buf['T 'N: Len] d array['T 'N] ;
: pass['F 'T] ( 'F['T] -- 'F['T] ) ;
: mk ( -- Buf[i64 4] ) 0 4 fill Buf ;
: main ( -- ) mk pass Buf> drop ;
";
    let (_t, entry) = single_file("len-domain-app", src);
    let err = build_error(&entry);
    assert!(err.contains("declares a length parameter"), "{err}");
}

// ---- Review fix (P1): App-headed `impl:` targets ----

/// `impl: Trait for 'F['T]` used to parse and register silently, then never
/// dispatch (`match_impl_target_rec` never matches an `App` pattern),
/// surfacing a misleading "does not satisfy" error at the *call site*
/// instead. Now a located rejection at the impl target itself.
#[test]
fn hkt_app_headed_impl_target_is_a_located_error() {
    let src = "\
trait: Shw['T] : shw ( &'T -- ) ; ;
: shows['T: Shw] ( &'T -- ) shw ;
impl: Shw for 'F['T] : shw | a | a drop ; ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("impl-target-app", src);
    let err = build_error(&entry);
    assert!(err.contains("may not apply its own type variable"), "{err}");
}
