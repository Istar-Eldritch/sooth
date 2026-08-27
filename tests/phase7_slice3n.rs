//! P7.S3n goldens: a generic `type:` field wrapping the declaration's own
//! type variable, and the owned-cell `PolyType`/`RawTy` variant that makes a
//! self-referential generic type expressible at all.
//!
//! Phase 1 was a *parser* phase: everything it asserts stops at declaration
//! or lives in a word signature. Phase 2 added `substitute_generic_field`'s
//! arms, so from here a **concrete instantiation** of each shape is a real
//! claim -- the tests below the phase-1 block instantiate. The `PolyType`-tree
//! assertions live beside `parse_generic_field_type_expr`, and the
//! substituted-`Type` assertions beside `substitute_generic_field`.

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
        ("arr", "type: Pair 'T items array['T 2] ;"),
        ("nest", "type: NestArr 'T grid array[array['T 2] 3] ;"),
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

// ---------------------------------------------------------------------------
// Phase 2: substitution, instantiation ordering, and the diagnostics the two
// together make reachable.
// ---------------------------------------------------------------------------

/// R4, whole pipeline: an array-of-type-variable field instantiated at two
/// differently-sized payloads, constructed from a real array and read back.
/// Two payloads rather than one, because a substitution that ignored the
/// argument and grounded every `array['T 2]` to the same shape would pass with one.
#[test]
fn array_of_ty_var_field_instantiates_and_runs_at_two_payloads() {
    let (stdout, code) = build_and_run(
        "arrfield",
        "import: intrinsics * ;\n\
         type: Pair 'T items array['T 2] ;\n\
         : first ( Pair[i64] -- i64 )\n\
           Pair> | items | &items 0 >usize &> @ items drop ;\n\
         : firstb ( Pair[u8] -- u8 )\n\
           Pair> | items | &items 0 >usize &> @ items drop ;\n\
         : main ( -- )\n\
           7 2 fill Pair first .\n\
           3 >u8 2 fill Pair firstb . ;\n",
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n3\n");
}

/// R4: the nesting claim at instantiation. A one-level array arm that did not
/// recurse would reach `substitute_generic_field`'s `unreachable!` on the
/// inner `array['T 2]` (N1).
#[test]
fn nested_array_of_ty_var_field_instantiates() {
    build(
        "nestfield",
        "type: NestArr 'T grid array[array['T 2] 3] ;\n\
         : f ( NestArr[i64] -- NestArr[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect("a nested array field instantiates to a nested concrete array");
}

/// R4/R3, whole pipeline: an owned-cell field over the header's own variable,
/// instantiated, constructed and unwrapped. `^` is the only indirection a
/// field may hold (a reference cannot be stored, an array does not break a
/// cycle), so this arm is what every self-referential generic type rests on.
#[test]
fn owned_cell_of_ty_var_field_instantiates_and_runs() {
    let (stdout, code) = build_and_run(
        "cellfield",
        "import: intrinsics * ;\n\
         type: Cell 'T c ^'T ;\n\
         : get ( Cell[i64] -- i64 ) Cell> ^> ;\n\
         : main ( -- ) 9 ^ Cell get . ;\n",
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "9\n");
}

/// R10, a diagnostic-quality requirement: `&'T` still does not build, but it
/// now fails for the right reason. Before this slice the parser never resolved
/// the field at all and blamed `'T` as an unknown type; now it grounds to a
/// real `Type::Ref` and meets the pre-existing, unconditional
/// no-stored-reference rule. Asserting only "this fails" would have passed
/// before the fix, so both halves are asserted.
#[test]
fn ref_of_ty_var_field_is_rejected_as_stored_reference() {
    let err = build(
        "reffield",
        "type: Box 'T r &'T ;\n\
         : f ( Box[i64] -- Box[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect_err("a reference cannot be stored in a field");
    assert!(
        err.contains("a reference cannot be stored"),
        "unexpected: {err}"
    );
    assert!(
        !err.contains("unknown type"),
        "the field must resolve before it is rejected: {err}"
    );
}

/// R10's composite-referent half, spelled out separately from the bare
/// `&'T` case above: `&Ent['K i64]` is a `Ref` whose referent is itself a
/// `Generic`, so this is the only fixture that can tell R4's `Ref` arm
/// actually recurses into its `Generic` arm from a `Ref` arm that only ever
/// substitutes a bare variable payload (that narrower arm would `unreachable!`
/// here instead of grounding to `&Ent[i64 i64]`).
#[test]
fn ref_of_composite_generic_field_is_rejected_as_stored_reference() {
    let err = build(
        "refcomposite",
        "type: Ent 'K 'V k 'K v 'V ;\n\
         type: Box 'T r &Ent['T i64] ;\n\
         : f ( Box[i64] -- Box[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect_err("a reference to a composite generic field cannot be stored");
    assert!(
        err.contains("a reference cannot be stored"),
        "unexpected: {err}"
    );
    assert!(
        err.contains("&Ent[i64 i64]"),
        "the referent must ground through the Generic arm, not blame the bare variable: {err}"
    );
}

/// R9: a by-value self-reference is caught by the *existing* `check_recursion`
/// rule, now reachable for a generic header for the first time. It only fires
/// on a post-instantiation concrete decl, so the program has to instantiate
/// `L` at something -- asserting against the bare generic declaration would
/// assert nothing.
#[test]
fn by_value_generic_self_reference_is_infinite_size_error() {
    let err = build(
        "byvalue",
        "type: L 'T v 'T next L['T] ;\n\
         : f ( L[i64] -- L[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect_err("a by-value self-reference has infinite size");
    assert!(
        err.contains("recursive struct definition (infinite size)"),
        "unexpected: {err}"
    );
}

/// R9's other edge kind: an array element does not break the cycle either, so
/// the same diagnostic fires one indirection down. Distinct from the by-value
/// case -- `type_node` reaches it through its `Type::Array` arm.
#[test]
fn array_wrapped_generic_self_reference_is_infinite_size_error() {
    let err = build(
        "arrwrapped",
        "type: L 'T v 'T kids [L['T] 4] ;\n\
         : f ( L[i64] -- L[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect_err("an array-wrapped self-reference has infinite size");
    assert!(
        err.contains("recursive struct definition (infinite size)"),
        "unexpected: {err}"
    );
}

/// R6's termination witness. `^L['T]` at `'T = i64` re-enters
/// `instantiate_struct` for the same `(idx, module, args)` while substituting
/// its own field; the memo pushed before that substitution is what closes the
/// loop.
///
/// Termination is the whole claim, and it needs no timeout wrapper: the
/// recursion is a call chain, so a regression here overflows the stack and
/// aborts (measured, by reverting the ordering) rather than hanging. The
/// runner reports that as a failure like any other.
#[test]
fn cell_wrapped_generic_self_reference_builds_and_terminates() {
    build(
        "cellcycle",
        "type: L 'T v 'T next ^L['T] ;\n\
         : f ( L[i64] -- L[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect("a cell breaks the cycle, so the type has a finite size");
}

/// R6's enum twin: `instantiate_enum` must mint the id, memo key and a
/// fieldless placeholder decl before substituting variants, exactly like the
/// struct half above. Every other termination witness in this file is
/// struct-side; reverting the enum ordering to substitute-then-mint leaves
/// this build (a legal program) overflowing the compiler's own stack.
#[test]
fn cell_wrapped_generic_self_reference_enum_builds_and_terminates() {
    build(
        "cellcycleenum",
        "type: L['T] | Nil | Cons v 'T next ^L['T] ;\n\
         : f ( L[i64] -- L[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect("a cell breaks the cycle for an enum header too, so the type has a finite size");
}

/// The two-header cycle the single-header test cannot cover: the memo key
/// includes the header index, so a mechanism that only recognised a header
/// re-entering *itself* would recurse forever here.
#[test]
fn mutual_cell_wrapped_generic_self_reference_terminates() {
    build(
        "mutualcycle",
        "type: A 'T v 'T next ^B['T] ;\n\
         type: B 'T w 'T back ^A['T] ;\n\
         : f ( A[i64] -- A[i64] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect("a mutual cycle through two cells is finite");
}

/// R8's accept side at instantiation, and the case that distinguishes its rule
/// from a blanket ban on self-reference: `^A['V 'K]` swaps its arguments each
/// hop, so the reachable closure is two instantiations rather than an
/// unbounded chain. The memo key includes `args`, which is what makes the
/// second hop find the first.
#[test]
fn permuting_generic_self_reference_terminates() {
    build(
        "permutecycle",
        "type: A 'K 'V k 'K v 'V next ^A['V 'K] ;\n\
         : f ( A[i64 u8] -- A[i64 u8] ) ;\n\
         : main ( -- ) ;\n",
    )
    .expect("a permuting self-reference alternates between two instantiations");
}

/// R5, in a polymorphic body: `poly_construct_generic` must thread the
/// *live* cell/ref registries into the `MutRegistries` it builds for a
/// fully-concrete constructor call, not a throwaway `&mut vec![]` pair --
/// `type_instantiation_name` unconditionally indexes into them to render a
/// cell- or ref-payload argument's name. `Ent` is constructed and
/// immediately dropped inside `mk`'s body (never named in a declared
/// signature), so this is the only path that mints `Ent[^i64 i64]` through
/// `poly_construct_generic` itself rather than through a signature's own
/// eager instantiation. A throwaway-registry regression here panics with an
/// out-of-bounds index instead of building.
#[test]
fn poly_body_constructs_generic_with_cell_argument() {
    let (stdout, code) = build_and_run(
        "polycellctor",
        "import: intrinsics * ;\n\
         type: Ent 'K 'V k 'K v 'V ;\n\
         : mkcell ( i64 -- ^i64 ) ^ ;\n\
         : mk ( 'T -- 'T ) 1 mkcell 2 Ent drop ;\n\
         : main ( -- ) 5 mk . ;\n",
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "5\n");
}

/// The out-of-scope gap, pinned rather than left silent: an *attributeless*
/// (positional) variant field cannot be an array. Pre-existing and unrelated
/// to type variables -- this fixture has no generic header at all -- and
/// deliberately untouched by this slice. A *named* generic variant field
/// (`Some xs array['T 2]`) is in scope and covered elsewhere.
#[test]
fn attributeless_variant_array_field_is_still_a_parse_error() {
    let err = build(
        "posvariant",
        "type: Foo | Some array[i64 2] | None ;\n: main ( -- ) ;\n",
    )
    .expect_err("a positional array variant field does not parse");
    assert!(
        err.contains("expected a word, found LBracket"),
        "unexpected: {err}"
    );
}
