//! P7.S4 Phase 1 goldens: generic `impl:` targets parse, dispatch, and run.
//!
//! Driven through the real `sooth` binary, so a generic `impl:` exercises the
//! whole check → lower → link → run pipeline. A single generic `impl: Show for
//! array['T 'N]` compiles and runs identically to a hand-written `impl: Show for
//! array[i64 4]` (R11), and two `impl:` blocks with overlapping-but-unequal targets
//! are accepted as declarations (R7).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sooth-p7s4-{tag}-{seq}", seq = seq));
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

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// R11: a single generic `impl: Show for array['T 'N]` compiles, links, runs, and
/// produces output identical to the same program with a hand-written
/// `impl: Show for array[i64 4]`.
#[test]
fn generic_impl_runs_identically_to_concrete_impl() {
    let trait_and_words = "\
trait: Show['T] : show ( &'T -- ) ; ;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  42 .\n\
  0 4 fill |a|\n\
  &a shows\n\
  a drop\n\
  99 .\n\
;\n";

    let generic_src =
        format!("impl: Show for array['T 'N]\n  : show | a | a drop ;\n;\n{trait_and_words}");
    let concrete_src =
        format!("impl: Show for array[i64 4]\n  : show | a | a drop ;\n;\n{trait_and_words}");

    let (_tg, entry_generic) = single_file("generic", &generic_src);
    let (_tc, entry_concrete) = single_file("concrete", &concrete_src);

    let out_generic = build_and_run(&entry_generic);
    let out_concrete = build_and_run(&entry_concrete);

    assert_eq!(
        out_generic, out_concrete,
        "generic and concrete impls should produce identical output"
    );
    assert!(out_generic.contains("42"), "{out_generic}");
    assert!(out_generic.contains("99"), "{out_generic}");
}

/// Review fix: a generic-impl dispatch resolved while composing a poly
/// word's cross-call (here, `outer` calls `inner`, and only `inner`'s own
/// bound resolves the `impl:`) must still have its member-word monomorph
/// recorded for lowering. Previously the compose path's discovery was
/// dropped, and lowering panicked with "checked resolved call exists".
#[test]
fn generic_impl_dispatch_via_composed_cross_call_runs() {
    let (_t, entry) = single_file(
        "composed_dispatch",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | a drop ;\n\
         ;\n\
         : inner ['T: Show] ( &'T -- ) show ;\n\
         : outer ['T: Show] ( &'T -- ) inner ;\n\
         : main ( -- )\n\
           42 .\n\
           0 4 fill |a|\n\
           &a outer\n\
           a drop\n\
           99 .\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    assert!(out.contains("42"), "{out}");
    assert!(out.contains("99"), "{out}");
}

/// R1: `impl: Show for 'T` resolves the type variable instead of erroring
/// "unknown type `'T`".
#[test]
fn generic_impl_target_var_parses_and_runs() {
    let (_t, entry) = single_file(
        "var_target",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for 'T\n\
           : show | a | a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           42 .\n\
           0 4 fill |a|\n\
           &a shows\n\
           a drop\n\
           99 .\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    assert!(out.contains("42"), "{out}");
    assert!(out.contains("99"), "{out}");
}

/// R7: two `impl:` blocks with overlapping-but-unequal targets are accepted
/// as declarations (the overlap is resolved by specificity at the dispatch
/// site, not rejected at declaration time).
#[test]
fn overlapping_unequal_targets_accepted_as_declarations() {
    let (_t, entry) = single_file(
        "overlap",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | a drop ;\n\
         ;\n\
         impl: Show for array['T 4]\n\
           : show | a | a drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let build = sooth_build(&entry);
    assert!(
        build.status.success(),
        "overlapping but unequal targets should be accepted; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::fs::remove_file(entry.with_extension("")).ok();
}

/// R7: two `impl:` blocks with alpha-equivalent generic targets are a
/// duplicate error.
#[test]
fn alpha_equivalent_generic_targets_are_duplicate_error() {
    let (_t, entry) = single_file(
        "dup",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | a drop ;\n\
         ;\n\
         impl: Show for array['U 'M]\n\
           : show | a | a drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("duplicate `impl:`"), "{err}");
}

/// R4: a generic `impl:` in the trait's own module (single file, both in
/// module 0) is accepted.
#[test]
fn generic_impl_in_trait_module_accepted() {
    let (_t, entry) = single_file(
        "orphan_ok",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | a drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let build = sooth_build(&entry);
    assert!(
        build.status.success(),
        "generic impl in the trait's own module should be accepted; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::fs::remove_file(entry.with_extension("")).ok();
}

/// R13: a two-module program where a generic `impl:` sits in the wrong
/// module (not the trait's) fails to compile with the located orphan error
/// naming the trait and the target shape.
#[test]
fn generic_impl_outside_trait_module_is_orphan_error() {
    let t = Tree::new("orphan-generic");
    t.write(
        "trait.sth",
        "trait: Show['T] : show ( &'T -- ) ; ;\nexport: Show ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"trait.sth\" t | Show | ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | a drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("must live in the module declaring `Show`"),
        "should name the trait: {err}"
    );
    assert!(
        err.contains("declares no module of its own"),
        "should explain the generic target has no home module: {err}"
    );
    assert!(
        err.contains("array['T 'N]"),
        "should name the target shape family: {err}"
    );
}

// P7.S4 Phase 2 goldens: the specificity partial order and the ambiguity
// error (R3, R8, R12).

/// R12: a concrete `impl: Show for array[i64 4]` overrides a generic
/// `impl: Show for array['T 'N]` at `array[i64 4]`; the generic covers `array[i64 2]`.
/// The concrete impl prints `1`, the generic prints `2`, so the output
/// proves which target dispatched.
#[test]
fn concrete_impl_overrides_generic_at_shared_instantiation() {
    let (_t, entry) = single_file(
        "override",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array[i64 4]\n\
           : show | a | 1 . a drop ;\n\
         ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | 2 . a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           0 4 fill |a|\n\
           &a shows\n\
           a drop\n\
           0 2 fill |b|\n\
           &b shows\n\
           b drop\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    // The concrete impl (array[i64 4]) prints 1; the generic (array['T 'N]) prints 2.
    assert!(
        out.contains("1\n"),
        "concrete impl should win at array[i64 4]: {out}"
    );
    assert!(
        out.contains("2\n"),
        "generic impl should cover array[i64 2]: {out}"
    );
}

/// Review fix (tests): the same scenario as
/// `concrete_impl_overrides_generic_at_shared_instantiation`, but with the
/// generic `impl:` declared *first* and the concrete one second. Both
/// declaration orders must pick the same winner by specificity, not by
/// which one was written first -- a test that only ever declares the
/// winner first can't tell `select_most_specific` apart from a plain
/// first-match rule.
#[test]
fn concrete_impl_overrides_generic_reversed_declaration_order() {
    let (_t, entry) = single_file(
        "override_reversed",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | 2 . a drop ;\n\
         ;\n\
         impl: Show for array[i64 4]\n\
           : show | a | 1 . a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           0 4 fill |a|\n\
           &a shows\n\
           a drop\n\
           0 2 fill |b|\n\
           &b shows\n\
           b drop\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    assert!(
        out.contains("1\n"),
        "concrete impl should win at array[i64 4] regardless of declaration order: {out}"
    );
    assert!(
        out.contains("2\n"),
        "generic impl should cover array[i64 2]: {out}"
    );
}

/// Review fix (R3): a bare-variable target (`'T`, one structural position)
/// loses to a structurally deeper target (`array['T 'N]`, two positions) at a
/// shared instantiation, even though the two targets don't flatten to the
/// same length. Before the fix, `specificity` rejected any differently-
/// sized pair as incomparable, so this raised a spurious ambiguity error
/// instead of dispatching to the array impl.
#[test]
fn array_impl_overrides_bare_var_impl_depth_mismatch() {
    let (_t, entry) = single_file(
        "depth_mismatch",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for 'T\n\
           : show | a | 1 . a drop ;\n\
         ;\n\
         impl: Show for array['T 'N]\n\
           : show | a | 2 . a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           0 4 fill |a|\n\
           &a shows\n\
           a drop\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(
        out, "2\n",
        "the array impl should win over the bare var: {out}"
    );
}

/// R12: two incomparable matching targets (`array[i64 'N]` vs `array['T 4]` at
/// `array[i64 4]`) produce a located ambiguity error naming both targets and
/// the concrete type.
#[test]
fn incomparable_targets_produce_ambiguity_error() {
    let (_t, entry) = single_file(
        "ambiguity",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array[i64 'N]\n\
           : show | a | a drop ;\n\
         ;\n\
         impl: Show for array['T 4]\n\
           : show | a | a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           0 4 fill |a|\n\
           &a shows\n\
           a drop\n\
         ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("ambiguous"), "{err}");
    assert!(err.contains("Show"), "{err}");
    // The concrete type `array[i64 4]` must appear in the diagnostic.
    assert!(err.contains("array[i64 4]"), "{err}");
    // Review fix (R8): both competing target *patterns* must be named, not
    // merely subsumed by the concrete-instantiation check above (which
    // `[i64` / `4]` substring checks were -- both are satisfied by
    // `array[i64 4]` alone and would pass even if neither target rendered).
    assert!(err.contains("array[i64 'N]"), "{err}");
    assert!(err.contains("array['T 4]"), "{err}");
}

/// R12: `array[array['T 'N] 'N]` and `array[array['T 'N] 'M]` both match `array[array[i64 4] 4]` and
/// the more specific `array[array['T 'N] 'N]` wins (its partition forces the inner and
/// outer lengths equal, which is the more constrained match). The
/// shared-var impl prints `1`, the distinct-var impl prints `2`. This is
/// the shared-variable golden test (the `Map['T 'T]` vs `Map['T 'U]`
/// scenario from the spec, exercised through arrays since the language's
/// generic-type constructor path is out of scope for this slice).
#[test]
fn shared_var_target_more_specific_wins() {
    let (_t, entry) = single_file(
        "shared_var",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array[array['T 'N] 'N]\n\
           : show | a | 1 . a drop ;\n\
         ;\n\
         impl: Show for array[array['T 'N] 'M]\n\
           : show | a | 2 . a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           0 4 fill |inner|\n\
           inner 4 fill |outer|\n\
           &outer shows\n\
           outer drop\n\
           inner drop\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    assert!(
        out.contains("1\n"),
        "array[array['T 'N] 'N] should win at array[array[i64 4] 4]: {out}"
    );
    assert!(
        !out.contains("2\n"),
        "array[array['T 'N] 'M] should not dispatch: {out}"
    );
}

/// Review fix (tests): the same scenario as
/// `shared_var_target_more_specific_wins`, but with the less-specific
/// (distinct-var) impl declared first and the more-specific (shared-var)
/// impl second. The winner must depend on specificity, not declaration
/// order.
#[test]
fn shared_var_target_more_specific_reversed_declaration_order() {
    let (_t, entry) = single_file(
        "shared_var_reversed",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for array[array['T 'N] 'M]\n\
           : show | a | 2 . a drop ;\n\
         ;\n\
         impl: Show for array[array['T 'N] 'N]\n\
           : show | a | 1 . a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           0 4 fill |inner|\n\
           inner 4 fill |outer|\n\
           &outer shows\n\
           outer drop\n\
           inner drop\n\
         ;\n",
    );
    let out = build_and_run(&entry);
    assert!(
        out.contains("1\n"),
        "array[array['T 'N] 'N] should win at array[array[i64 4] 4] regardless of declaration order: {out}"
    );
    assert!(
        !out.contains("2\n"),
        "array[array['T 'N] 'M] should not dispatch: {out}"
    );
}
