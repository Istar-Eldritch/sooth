//! P7.S3n phase 1 goldens: a generic `type:` field wrapping the
//! declaration's own type variable, and the owned-cell `PolyType`/`RawTy`
//! variant that makes a self-referential generic type expressible at all.
//!
//! Phase 1 is a *parser* phase: `substitute_generic_field` still has no arms
//! for the new field shapes, so a **concrete instantiation** of one aborts
//! until phase 2. Everything asserted here therefore either stops at
//! declaration (an uninstantiated generic header reaches no substitution) or
//! is a word signature, where the substitution path is `apply_subst`, which
//! phase 1 does complete. The unit tests beside `parse_generic_field_type_expr`
//! carry the `PolyType`-tree assertions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3n-{}-{tag}-{seq}", std::process::id()));
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

/// Build `contents` as a single-file program, returning the driver's result.
/// The scratch tree is torn down before returning, so the caller gets the
/// diagnostic (or the fact of success) and nothing runnable; use
/// `build_and_run` when the binary itself is needed.
fn build(tag: &str, contents: &str) -> Result<PathBuf, String> {
    let prog = Scratch::write(tag, contents);
    driver::build_with_manifest(prog.path(), common::manifest_for(prog.path()).as_deref())
}

/// Build and run, returning `(stdout, exit code)`. The scratch tree outlives
/// the run: the built binary sits inside it.
fn build_and_run(tag: &str, contents: &str) -> (String, i32) {
    let prog = Scratch::write(tag, contents);
    let binary =
        driver::build_with_manifest(prog.path(), common::manifest_for(prog.path()).as_deref())
            .expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code().expect("process should exit normally"),
    )
}

/// R3, the word-signature side, end to end: a `^`-typed parameter over a type
/// variable is declarable, callable, and lowers. This is a whole-pipeline
/// witness, not a parse check -- calling it exercises `unify_poly_input`'s new
/// cell arm (which reads the payload out of the cell registry), `apply_subst`'s
/// (which interns the ground shape) and `subst_polytype`'s (which looks it up
/// at lowering). Any one of those missing and this fails rather than merely
/// mis-typing.
#[test]
fn owned_cell_type_variable_in_word_signature_builds_and_runs() {
    let (stdout, code) = build_and_run(
        "idc",
        ": idc ( ^'T -- ^'T ) ;\n\
         : main ( -- ) 7 ^ idc ^> . ;\n",
    );
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\n",
        "the cell must round-trip through the polymorphic word unchanged"
    );
}

/// R3: the same word at a *second*, differently-shaped payload. One
/// instantiation cannot tell a correct cell arm from one that ignores the
/// payload entirely and grounds every `^'T` to the same shape.
#[test]
fn owned_cell_type_variable_instantiates_at_two_distinct_payloads() {
    let (stdout, code) = build_and_run(
        "idc2",
        ": idc ( ^'T -- ^'T ) ;\n\
         : main ( -- )\n\
           7 ^ idc ^> .\n\
           1 >u8 ^ idc ^> . ;\n",
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n1\n");
}

/// R3/N1: a `^` with the stack-effect separator behind it has no payload. A
/// located error, not a blame on `--` as an unknown type name.
#[test]
fn owned_cell_without_payload_in_signature_is_located_error() {
    let err = build("nopayload", ": f ( ^ -- 'T ) ;\n: main ( -- ) ;\n")
        .expect_err("a payloadless cell must be rejected");
    assert!(err.contains("has no payload type"), "unexpected: {err}");
}

/// R1: each of the five field shapes the slice adds *declares* cleanly. Before
/// the recursive descent every one of these was `error: unknown type 'T` --
/// the variable sat one token deeper than the old single-`if` production
/// looked. None is instantiated here: substitution is phase 2.
#[test]
fn generic_field_shapes_wrapping_own_ty_var_declare() {
    for (tag, decl) in [
        ("arr", "type: Pair 'T items ['T 2] ;"),
        ("nest", "type: NestArr 'T grid [['T 2] 3] ;"),
        ("cell", "type: Cell 'T c ^'T ;"),
        ("ref", "type: Box 'T r &'T ;"),
        (
            "app",
            "type: Ent 'K 'V k 'K v 'V ;\ntype: Wrap 'K 'V e Ent['K 'V] ;",
        ),
    ] {
        let src = format!("{decl}\n: main ( -- ) ;\n");
        build(tag, &src).unwrap_or_else(|e| panic!("`{decl}` should declare cleanly: {e}"));
    }
}

/// R2: a header must be registered before its *own* field list is parsed.
/// The argument here is fully concrete, so this needs none of R1's descent --
/// it is R2's own witness, and it was `error: unknown type 'L'` before the
/// two-stage split.
///
/// It also witnesses R2's *second* half. `L[i64]` is minted while `L`'s header
/// is still a placeholder with no fields, and the field list is owed and paid
/// off on fill. Without that, `L[i64]` stays permanently fieldless, its cycle
/// is invisible, and this program builds -- so the `check_recursion`
/// diagnostic below is what proves the deferred fill ran.
#[test]
fn concrete_generic_self_reference_resolves_and_reaches_recursion_check() {
    let err = build(
        "selfref",
        "type: L 'T v 'T next L[i64] ;\n: main ( -- ) ;\n",
    )
    .expect_err("a by-value self-reference has infinite size");
    assert!(
        err.contains("recursive struct definition (infinite size)"),
        "the self-reference must resolve and reach `check_recursion`, not \
         report an unknown type: {err}"
    );
    assert!(
        !err.contains("unknown type"),
        "the header must be findable from inside its own field list: {err}"
    );
}

/// R8: a growing self-referential application is a parse-time rejection
/// naming the restriction, never a hang. Each hop wraps `'T` in another cell,
/// so `L` would need instantiating at a strictly larger argument forever --
/// with no `Generic`-in-`Generic` nesting anywhere, so the pre-existing
/// depth rule (D5) never sees it.
#[test]
fn growing_generic_self_reference_is_rejected_at_declaration() {
    let err = build(
        "growing",
        "type: L 'T v 'T next ^L[^'T] ;\n: main ( -- ) ;\n",
    )
    .expect_err("a growing self-reference must be rejected");
    assert!(
        err.contains("fully concrete or a bare type variable"),
        "the diagnostic must name the restriction so a non-recursive type is \
         not told it is recursive: {err}"
    );
}

/// R8's accept side: a *non*-growing self-reference behind a cell declares
/// cleanly. Without this the rule reads as a blanket ban on self-reference,
/// which would defeat the slice's whole point.
#[test]
fn non_growing_cell_self_reference_declares() {
    build(
        "nongrowing",
        "type: L 'T v 'T next ^L['T] ;\n: main ( -- ) ;\n",
    )
    .expect("a bare-variable argument is not growing");
    build(
        "permuting",
        "type: A 'K 'V k 'K v 'V next ^A['V 'K] ;\n: main ( -- ) ;\n",
    )
    .expect("a permuting self-reference alternates between two instantiations");
}

/// R7: a quotation field naming the declaration's own type variable is out of
/// scope, rejected with a located message rather than misreporting `'T` as an
/// unknown concrete type -- and a *concrete* quotation field, legal today,
/// still declares. Both halves matter: the `[`-arm has to replicate
/// `quotation_type_ahead`'s disambiguation, or it misparses the concrete one
/// as a malformed array.
#[test]
fn variable_quotation_field_is_rejected_and_concrete_one_still_declares() {
    let err = build("quotvar", "type: QF 'T f [ 'T -- 'T ] ;\n: main ( -- ) ;\n")
        .expect_err("a variable-bearing quotation field is out of scope");
    assert!(err.contains("quotation field"), "unexpected: {err}");
    assert!(
        !err.contains("unknown type"),
        "the variable must be recognised, not misreported: {err}"
    );
    build(
        "quotconcrete",
        "type: Q 'T v 'T f [ i64 -- i64 ] ;\n: main ( -- ) ;\n",
    )
    .expect("a concrete quotation field is unchanged by this slice");
}
