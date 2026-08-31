//! P7.S4b Phase 2 goldens: recursive per-instantiation bound discharge with
//! cycle detection.
//!
//! A generic `impl:` whose `where`-clause bounds its own type variable (e.g.
//! `impl: Show for array['T 4] where 'T: Print`) dispatches the member body's
//! trait-member calls on that variable through recursive `find_bound_impl`
//! discharge: at `array[Point 4]` the bound becomes `Point: Print`, and the helper
//! finds `impl: Print for Point` in the registry. Omitting that concrete impl
//! produces the located unsatisfied-bound error. A self-referential bound
//! cycle (`impl: Show for 'T where 'T: Show`) is a located error, not a hang.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sooth-p7s4b-{tag}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
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
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg("--manifest")
        .arg(common::fixture_manifest())
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

/// R9: a bounded generic `impl: Show for array['T 4] where 'T: Print` instantiated
/// at `array[Point 4]` (where `Point` has `impl: Print`) compiles, runs, and
/// produces output identical to a hand-written per-element concrete
/// counterpart.
///
/// Uses `Print` (a separate trait) rather than `Show` in the `where`-clause
/// because `rewrite_member_self_calls` rewrites any call to the member's own
/// name (`show`) to the synthesized self-word symbol, preventing
/// `poly_trait_member_call` from recognizing it as trait dispatch. A
/// different trait's member name (`print`) is not rewritten and is correctly
/// recognized.
#[test]
fn bounded_generic_impl_runs_identically_to_concrete_counterpart() {
    let trait_decls = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
trait: Print['T] : print ( &'T -- ) ; ;\n\
impl: Print for Point\n\
  : print | p | 42 . p drop ;\n\
;\n";

    let generic_src = format!(
        "{trait_decls}\
         impl: Show for array['T 4] where 'T: Print\n\
           : show | a | a 0 >usize &> print a 1 >usize &> print a 2 >usize &> print a 3 >usize &> print a drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- )\n\
           1 2 Point |p|\n\
           p 4 fill |arr|\n\
           &arr shows\n\
           arr drop\n\
           p drop\n\
         ;\n"
    );

    let counterpart_src = format!(
        "{trait_decls}\
         : show_arr ( &array[Point 4] -- )\n\
           | a | 42 . 42 . 42 . 42 . a drop ;\n\
         : main ( -- )\n\
           1 2 Point |p|\n\
           p 4 fill |arr|\n\
           &arr show_arr\n\
           arr drop\n\
           p drop\n\
         ;\n"
    );

    let (_tg, entry_generic) = single_file("generic", &generic_src);
    let (_tc, entry_counterpart) = single_file("counterpart", &counterpart_src);

    let out_generic = build_and_run(&entry_generic);
    let out_counterpart = build_and_run(&entry_counterpart);

    assert_eq!(
        out_generic, out_counterpart,
        "bounded generic impl and concrete counterpart should produce identical output"
    );
    assert!(out_generic.contains("42"), "{out_generic}");
}

/// R9 negative: omitting `Point`'s `impl: Print` produces the located
/// unsatisfied-bound error at the `shows` call site, not a crash or hang.
#[test]
fn omitting_element_impl_produces_unsatisfied_bound_error() {
    let src = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
trait: Print['T] : print ( &'T -- ) ; ;\n\
\\ No impl: Print for Point — the bound 'T: Print won't discharge\n\
impl: Show for array['T 4] where 'T: Print\n\
  : show | a | a 0 >usize &> print a 1 >usize &> print a 2 >usize &> print a 3 >usize &> print a drop ;\n\
;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  1 2 Point |p|\n\
  p 4 fill |arr|\n\
  &arr shows\n\
  arr drop\n\
  p drop\n\
;\n";
    let (_t, entry) = single_file("neg", src);
    let err = build_error(&entry);
    assert!(
        err.contains("cannot instantiate `'T` of `shows` with `array[Point 4]`"),
        "{err}"
    );
    assert!(
        err.contains("`array[Point 4]` does not satisfy `Show`"),
        "{err}"
    );
}

/// R10: a self-referential bound cycle — `impl: Show for 'T where 'T: Show`
/// — produces the located cycle error at the impl declaration, not a stack
/// overflow or hang.
#[test]
fn self_referential_bound_cycle_is_located_error() {
    let src = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
impl: Show for 'T where 'T: Show\n\
  : show | a | a drop ;\n\
;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  1 2 Point |p|\n\
  &p shows\n\
  p drop\n\
;\n";
    let (_t, entry) = single_file("cycle", src);
    let err = build_error(&entry);
    assert!(err.contains("bound-discharge cycle"), "{err}");
    assert!(
        err.contains("`impl: Show for 'T` requires `Show for Point`"),
        "{err}"
    );
    // The error is located at the impl declaration (line 4 in the file:
    // line 1 is the `import:` preamble, lines 2-3 are the type/trait decls).
    assert!(err.contains("line 4, col 1"), "{err}");
}

/// R8 parity: a bounded generic impl dispatches correctly and the existing
/// `impl:` examples compile unchanged. This is a smoke test that the
/// `where`-clause threading doesn't break unbounded generic impls.
#[test]
fn unbounded_generic_impl_still_dispatches() {
    let src = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
impl: Show for array['T 4]\n\
  : show | a | a drop ;\n\
;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  1 2 Point |p|\n\
  p 4 fill |arr|\n\
  &arr shows\n\
  arr drop\n\
  p drop\n\
;\n";
    let (_t, entry) = single_file("parity", src);
    let out = build_and_run(&entry);
    // No crash, no output from the no-op show, just a clean exit.
    assert!(out.is_empty() || !out.contains("error"), "{out}");
}

// P7.S4b Phase 3 goldens: the specificity bound-set tiebreak (R5, R11).
//
// A bounded generic impl (`impl: Show for ['T N] where 'T: Print`) overrides
// an unbounded generic impl (`impl: Show for ['T N]`) at instantiations where
// the bound is satisfied (the element type has `impl: Print`). At
// instantiations where the bound is not satisfied, the bounded candidate is
// excluded and the unbounded one dispatches.

/// R11: a bounded generic impl overrides an unbounded generic impl with the
/// same pattern at instantiations where the bound is satisfied. At `array[Point 4]`
/// (where `Point` has `impl: Print`), the bounded impl wins (prints `1`). At
/// `array[i64 4]` (where `i64` has no `impl: Print`), the bounded candidate is
/// excluded and the unbounded impl wins (prints `2`).
#[test]
fn bounded_impl_overrides_unbounded_at_satisfied_instantiation() {
    let src = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
trait: Print['T] : print ( &'T -- ) ; ;\n\
impl: Print for Point\n\
  : print | p | 42 . p drop ;\n\
;\n\
impl: Show for array['T 'N] where 'T: Print\n\
  : show | a | 1 . a drop ;\n\
;\n\
impl: Show for array['T 'N]\n\
  : show | a | 2 . a drop ;\n\
;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  1 2 Point |p|\n\
  p 4 fill |arr|\n\
  &arr shows\n\
  arr drop\n\
  p drop\n\
  0 4 fill |iarr|\n\
  &iarr shows\n\
  iarr drop\n\
;\n";
    let (_t, entry) = single_file("tiebreak", src);
    let out = build_and_run(&entry);
    assert!(
        out.contains("1\n"),
        "bounded impl should win at array[Point 4]: {out}"
    );
    assert!(
        out.contains("2\n"),
        "unbounded impl should win at array[i64 4]: {out}"
    );
}

/// R11 (reversed declaration order): the same scenario as above, but with
/// the unbounded impl declared first and the bounded impl second. The winner
/// must depend on specificity, not declaration order.
#[test]
fn bounded_impl_overrides_unbounded_reversed_declaration_order() {
    let src = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
trait: Print['T] : print ( &'T -- ) ; ;\n\
impl: Print for Point\n\
  : print | p | 42 . p drop ;\n\
;\n\
impl: Show for array['T 'N]\n\
  : show | a | 2 . a drop ;\n\
;\n\
impl: Show for array['T 'N] where 'T: Print\n\
  : show | a | 1 . a drop ;\n\
;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  1 2 Point |p|\n\
  p 4 fill |arr|\n\
  &arr shows\n\
  arr drop\n\
  p drop\n\
  0 4 fill |iarr|\n\
  &iarr shows\n\
  iarr drop\n\
;\n";
    let (_t, entry) = single_file("tiebreak_rev", src);
    let out = build_and_run(&entry);
    assert!(
        out.contains("1\n"),
        "bounded impl should win at array[Point 4] regardless of declaration order: {out}"
    );
    assert!(
        out.contains("2\n"),
        "unbounded impl should win at array[i64 4]: {out}"
    );
}

/// R5: two bounded impls with the same pattern but incomparable bound sets
/// (where 'T: Print vs where 'T: Display) at an instantiation where both
/// bounds discharge produce the existing located ambiguity error.
#[test]
fn incomparable_bound_sets_produce_ambiguity_error() {
    let src = "\
type: Point x i64 y i64 ;\n\
trait: Show['T] : show ( &'T -- ) ; ;\n\
trait: Print['T] : print ( &'T -- ) ; ;\n\
trait: Display['T] : display ( &'T -- ) ; ;\n\
impl: Print for Point\n\
  : print | p | p drop ;\n\
;\n\
impl: Display for Point\n\
  : display | p | p drop ;\n\
;\n\
impl: Show for array['T 'N] where 'T: Print\n\
  : show | a | 1 . a drop ;\n\
;\n\
impl: Show for array['T 'N] where 'T: Display\n\
  : show | a | 2 . a drop ;\n\
;\n\
: shows ['T: Show] ( &'T -- ) show ;\n\
: main ( -- )\n\
  1 2 Point |p|\n\
  p 4 fill |arr|\n\
  &arr shows\n\
  arr drop\n\
  p drop\n\
;\n";
    let (_t, entry) = single_file("incomparable", src);
    let err = build_error(&entry);
    assert!(err.contains("ambiguous"), "{err}");
    assert!(err.contains("Show"), "{err}");
}
