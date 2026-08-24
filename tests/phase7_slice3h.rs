//! P7.S3h phase 1 goldens: an escaping closure may capture a scalar-represented
//! value, even one spelled as an enum.
//!
//! `classify_capture`'s aggregate arm used to answer `FrameRooted` for every
//! `Struct`/`Enum`/`Array`/`OwnedCell` capture unconditionally, so a captured
//! `Bool` -- a payload-free, structurally-`Copy` enum since S3i -- was rejected
//! at every escaping boundary for being spelled as an enum rather than for
//! anything about its storage. The arm now splits on scalar representation:
//! a payload-free enum is a *value* in the one-word env slot and admits, while
//! a struct, an array and a payload-carrying enum are pointers into frame
//! storage and keep rejecting however `Copy` they are.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3c.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3h-{}-{tag}-{seq}", std::process::id()));
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

fn build_and_run(src: &Path) -> String {
    let binary = driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .unwrap_or_else(|e| panic!("program should build: {e}"));
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    assert!(output.status.success(), "the built binary should exit 0");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn build_error(src: &Path) -> String {
    driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect_err("program should not build")
}

/// The motivating case, end to end: `mk` captures its `Bool` parameter into a
/// closure it returns, and the closure is called after `mk`'s frame is gone.
/// Both discriminants are threaded through so the assertion pins the captured
/// *value*, not merely that something built -- a snapshot that read the wrong
/// word would print one answer twice.
#[test]
fn escaping_closure_over_a_bool_local_admits_and_snapshots_it() {
    let prog = Scratch::write(
        "bool",
        ": mk ( Bool -- [ -- Bool ] ) | b | [ b ] ;\n\
         : main ( -- ) True mk call . False mk call . ;\n",
    );
    assert_eq!(build_and_run(prog.path()), "True\nFalse\n");
}

/// The second `check_capture_admission` call site (`check_branch_join`), which
/// the return-boundary golden above never reaches: two *different* quotation
/// literals joining at a word tail, each capturing a `Bool` local of that
/// frame. Before this slice the join rejected at the first arm.
#[test]
fn branch_join_of_two_bool_capturing_arms_admits() {
    let prog = Scratch::write(
        "join",
        ": pick ( Bool Bool Bool -- [ -- Bool ] )\n  \
           | s a b |\n  \
           s ~[ [ a ] ] ~[ [ b ] ] if\n\
         ;\n\
         : main ( -- )\n  \
           True False True pick call .\n  \
           False False True pick call .\n\
         ;\n",
    );
    assert_eq!(build_and_run(prog.path()), "False\nTrue\n");
}

/// The narrowing's guard on the enum side: `Item` is `Copy` (its one payload
/// field is an `i64`), so an `is_copy`-only predicate would admit it -- but a
/// payload-carrying enum lives in tagged storage reached by pointer, and
/// snapshotting that pointer into the env would outlive the frame it points
/// into. `escaping_closure_over_frame_local_is_past_owning_frame`
/// (`tests/phase4_quotations.rs`) is the array-shaped twin of this, unchanged
/// by the slice.
#[test]
fn escaping_closure_over_a_payload_carrying_enum_local_still_rejects() {
    let prog = Scratch::write(
        "payload",
        "type: Item | Empty | Full v i64 ;\n\
         : mk ( Item -- [ -- Item ] ) | e | [ e ] ;\n\
         : main ( -- ) Empty mk call | r | r ~[ 1 . ] ~[ 0 . ] if ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: an escaping closure captures `e`, a local of this frame, whose storage does not survive the return (line 2)"
    );
}

/// The narrowing's guard on the struct side. `P` is all-`i64`, so it is `Copy`
/// too, and it is still pointer-backed: `is_aggregate` is unconditionally true
/// for a struct.
#[test]
fn escaping_closure_over_a_copy_struct_local_still_rejects() {
    let prog = Scratch::write(
        "struct",
        "type: P x i64 y i64 ;\n\
         : mk ( -- [ -- i64 ] ) 1 2 P | p | [ p .x ] ;\n\
         : main ( -- ) mk call . ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: an escaping closure captures `p`, a local of this frame, whose storage does not survive the return (line 2)"
    );
}

// ---------------------------------------------------------------------------
// Phase 2: the `owning [ ... ]` type -- syntax, containment, inherited
// linearity, and the not-built-yet guard.
//
// Two shapes the spec lists are deliberately absent, because phase 2 cannot
// produce an `owning`-typed *value* at all: every materialization boundary
// matches `Type::Quotation` structurally, and nothing infers owningness at a
// literal yet (that is phase 3). So there is no "materialized owning literal"
// to guard and no owning/plain `if`-join to join -- both reduce to the declared
// `owning` type, which the guard below rejects before any of it runs. The
// nearest reachable witness for the `if`-join is
// `plain_arms_joining_at_an_owning_output_hit_the_guard`.
// ---------------------------------------------------------------------------

/// A forced-linear struct with an observable `drop`, the shape `tests/phase0.rs`
/// uses for the linear core.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | \"drop \" . s Spy> . ;\n";

/// The containment rule, end to end and at the position that motivates it.
/// A struct holding an `owning` field is non-`Copy`, so `drop`ping it is a
/// legal consumption -- but `emit_drop`'s `_ => {}` swallows a quotation and
/// `field_is_linear` answers false for one, so no destructor is synthesized at
/// all and the container's `drop` becomes a complete no-op. The rejection is
/// what keeps "the body is the sole disposer" true, and it costs no new gate:
/// `reject_quotation_type_position` dispatches on `is_quotation_type`, whose
/// `owning` answer is `Some`, while the legal-position carve-out matches
/// `Type::Quotation` structurally.
#[test]
fn an_owning_quotation_field_is_rejected() {
    let prog = Scratch::write("field", "type: Box q owning [ -- ] ;\n: main ( -- ) ;\n");
    let err = build_error(prog.path());
    assert!(
        err.contains("a quotation type `owning [ -- ]` cannot appear as the field `q` of struct"),
        "unexpected message: {err}"
    );
}

/// The variant-field half, which is a P0-shaped position exactly like a struct
/// field: enums do support linear variant fields (`examples/list.sth`'s
/// `Cons ... next ^List`), so an owning variant field would be just as linear
/// and just as undisposable.
#[test]
fn an_owning_quotation_variant_field_is_rejected() {
    let prog = Scratch::write(
        "variant-field",
        "type: E | None | Some q owning [ -- ] ;\n: main ( -- ) ;\n",
    );
    let err = build_error(prog.path());
    assert!(
        err.contains(
            "a quotation type `owning [ -- ]` cannot appear as the field `q` of enum variant"
        ),
        "unexpected message: {err}"
    );
}

/// `owning` is intercepted ahead of every user type registry, so a `type:`
/// declared under that name would be silently unreachable rather than merely
/// shadowed.
#[test]
fn a_type_named_owning_is_a_located_reserved_name_rejection() {
    let prog = Scratch::write("reserved", "type: owning x i64 ;\n: main ( -- ) ;\n");
    let err = build_error(prog.path());
    assert!(
        err.contains("`owning` is reserved for the owning-quotation syntax")
            && err.contains("as a type name at line 1"),
        "unexpected message: {err}"
    );
}

/// `owning` is a *type*-position keyword only: owningness is inferred at a
/// literal and declared in a type, so there is no term-level spelling. The
/// sharp case the spec names -- a non-capturing `owning [ 42 ]`, which would
/// bypass capture admission entirely -- is therefore an unknown *word*, and
/// emphatically not a panic.
#[test]
fn owning_in_a_term_position_is_an_unknown_word() {
    let prog = Scratch::write("term", ": main ( -- ) owning [ 42 ] drop ;\n");
    assert_eq!(
        build_error(prog.path()),
        "error: unknown word `owning` in `main` (line 1)"
    );
}

/// The not-built-yet guard, input side. A declared `owning` parameter reaches
/// `ir_type_of` through *signature* lowering without ever crossing a
/// materialization boundary, so a capture-side guard alone would ICE here.
#[test]
fn a_declared_owning_parameter_hits_the_guard_with_a_diagnostic() {
    let prog = Scratch::write(
        "param",
        ": f ( owning [ -- ] -- ) call ;\n: main ( -- ) ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: `f` declares `owning [ -- ]`, which has no runtime representation this slice: an owning closure's env storage and disposal are not built yet"
    );
}

/// The guard's output side, which is where a plain literal at an `owning`
/// boundary lands too: the declaration is rejected before the exit row is ever
/// judged, so `[ 1 drop ]` never gets blamed for the mismatch.
#[test]
fn a_plain_literal_at_an_owning_output_hits_the_guard() {
    let prog = Scratch::write(
        "output",
        ": mk ( -- owning [ -- ] ) [ 1 drop ] ;\n: main ( -- ) mk call ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: `mk` declares `owning [ -- ]`, which has no runtime representation this slice: an owning closure's env storage and disposal are not built yet"
    );
}

/// The nearest reachable form of the `if`-join case: two arms joining at a
/// declared `owning` output. A genuinely mixed owning/plain join needs an
/// owning-typed value, which phase 2 cannot build.
#[test]
fn plain_arms_joining_at_an_owning_output_hit_the_guard() {
    let prog = Scratch::write(
        "join-owning",
        ": mk ( Bool -- owning [ -- ] ) ~[ [ 1 drop ] ] ~[ [ 2 drop ] ] if ;\n\
         : main ( -- ) True mk call ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: `mk` declares `owning [ -- ]`, which has no runtime representation this slice: an owning closure's env storage and disposal are not built yet"
    );
}

/// The marker inherits every exactly-once obligation from `is_copy` answering
/// false, with no code of its own: move tracking, the `dup` gate, the
/// forgotten-value error and the consumed-on-every-path check all fire on an
/// `owning` binding. These three are the evidence for that claim -- each is a
/// diagnostic phase 2 wrote nothing to produce, and each reports *before* the
/// not-built-yet guard, which is why the guard runs after `check_types`.
#[test]
fn an_owning_binding_inherits_the_linear_obligations() {
    for (tag, body, want) in [
        (
            "dup",
            ": f ( owning [ -- ] -- ) | q | q dup call call ;\n",
            "cannot `dup` a value of type `owning [ -- ]`",
        ),
        (
            "forget",
            ": f ( owning [ -- ] -- ) | q | ;\n",
            "linear value `q` is never consumed",
        ),
        (
            "one-arm",
            ": f ( owning [ -- ] Bool -- ) | q c | c ~[ q call ] ~[ 1 . ] if ;\n",
            "linear value `q` is not consumed on every path",
        ),
    ] {
        let prog = Scratch::write(tag, &format!("{body}: main ( -- ) ;\n"));
        let err = build_error(prog.path());
        assert!(err.contains(want), "unexpected message for {tag}: {err}");
    }
}

/// A linear capture at a *plain* boundary names `owning` as the remedy, since
/// an owning boundary is exactly what would let the closure take ownership of
/// the capture and dispose it by running. Conditioned on linearity: the `Copy`
/// struct twin
/// (`escaping_closure_over_a_copy_struct_local_still_rejects`, above) keeps the
/// bare message, because its problem is a pointer into dead frame storage that
/// no disposal obligation addresses.
#[test]
fn a_linear_capture_at_a_plain_boundary_names_owning() {
    let prog = Scratch::write(
        "linear-capture",
        &format!(
            "{SPY_DEF}: mk ( Spy -- [ -- ] ) | s | [ s drop ] ;\n: main ( -- ) 7 Spy mk call ;\n"
        ),
    );
    assert_eq!(
        build_error(prog.path()),
        "error: an escaping closure captures `s`, a local of this frame, whose storage does not survive the return (line 3)\n  declare the boundary `owning [ ... ]` to hand the closure ownership of `s`, so calling it disposes `s`"
    );
}

/// The guard's non-obvious reach path. An `impl:` block's member is a
/// synthesized `WordDef` under an unforgeable `member;Trait;Type` name, and it
/// inherits the trait member's signature with `'T` substituted -- so an
/// `owning` slot can arrive in a lowerable declaration without any word in the
/// source spelling it. A trait member that nothing implements lowers nothing
/// and is left alone; the `impl:` is what would reach `ir_type_of`.
#[test]
fn an_owning_parameter_inherited_by_an_impl_member_hits_the_guard() {
    let prog = Scratch::write(
        "impl-member",
        "type: W x i64 ;\n\
         trait: Own 'T\n  use ( 'T owning [ -- ] -- )\n;\n\
         impl: Own for W\n  : use | w q | w drop q call ;\n;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(prog.path());
    assert!(
        err.contains("declares `owning [ -- ]`")
            && err.contains("no runtime representation this slice"),
        "unexpected message: {err}"
    );
}
