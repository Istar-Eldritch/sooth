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

// ---------------------------------------------------------------------------
// Phase 3: representation, the call-once lifecycle, and env disposal.
//
// An owning closure's env is a heap block holding every capture by value. The
// compiled body copies each capture into its own frame, frees the block, and
// then consumes the captures exactly as a word body consumes a linear
// parameter. `call` is the consuming use of the closure value itself (it pops
// its receiver and never re-pushes it), so the inherited linear machinery is
// what forces the body to run exactly once.
// ---------------------------------------------------------------------------

/// The slice's headline program, and the one the exactly-once claim rests on.
/// `mk` moves a linear `Spy` into a closure it returns; the closure outlives
/// `mk`'s frame; calling it disposes the `Spy`. **One** observation, not zero
/// (the block freed without running the destructor) and not two (the frame and
/// the env each disposing their own copy).
#[test]
fn an_owning_closure_disposes_its_captured_linear_value_exactly_once() {
    let prog = Scratch::write(
        "dispose-once",
        &format!(
            "{SPY_DEF}: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n\
             : main ( -- ) 7 Spy mk call ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n");
}

/// The capture is genuinely *moved*: `mk`'s frame no longer owns the `Spy`, so
/// `Scope::leave`'s unconsumed-local check is satisfied without the frame
/// disposing anything. Two closures over two distinct `Spy`s, called in the
/// opposite order to the order they were built, pin that each env holds its own
/// value rather than aliasing one frame slot.
#[test]
fn two_owning_closures_each_own_their_own_capture() {
    let prog = Scratch::write(
        "two-closures",
        &format!(
            "{SPY_DEF}: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n\
             : main ( -- ) 1 Spy mk 2 Spy mk | a b | b call a call ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 2\ndrop 1\n");
}

/// Two linear captures, which the plain env cannot hold at an escaping boundary
/// at all (`multi_capture_escaping_error` defers a 2+-capture closure whose env
/// is the one inline word). A heap block has room for both, so the deferral is
/// lifted for the owning path only, and both are disposed exactly once.
#[test]
fn an_owning_closure_over_two_linear_captures_disposes_both_exactly_once() {
    let prog = Scratch::write(
        "two-captures",
        &format!(
            "{SPY_DEF}: mk ( Spy Spy -- owning [ -- ] ) | s t | [ s drop t drop ] ;\n\
             : main ( -- ) 7 Spy 9 Spy mk call ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\ndrop 9\n");
}

/// `drop` cannot discharge the obligation: disposing the captures means running
/// the body, which is code only the closure has, and `emit_drop`'s match has no
/// arm that could run one. Without this rejection the `drop` is silently a
/// no-op -- the obligation discharged, the `Spy` and the env block both leaked.
#[test]
fn dropping_an_owning_closure_is_a_located_rejection() {
    let prog = Scratch::write(
        "drop-owning",
        &format!(
            "{SPY_DEF}: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n\
             : main ( -- ) 7 Spy mk drop ;\n"
        ),
    );
    assert_eq!(
        build_error(prog.path()),
        "error: cannot `drop` a value of type `owning [ -- ]` in `main` (line 4): an owning closure disposes its captures by running, so `call` it -- no destructor can run a closure body"
    );
}

/// The generic-body twin of the same rejection. A generic word cannot *declare*
/// an owning parameter, but it can call a word that returns one, so the value
/// arrives through the body rather than the signature and reaches the poly
/// walk's own `drop` arm -- which fails open without its own gate, since the
/// monomorphic one never runs on a poly body.
#[test]
fn dropping_an_owning_closure_in_a_generic_body_is_a_located_rejection() {
    let prog = Scratch::write(
        "drop-owning-poly",
        ": mk ( -- owning [ -- ] ) [ 1 . ] ;\n\
         : g ( 'T: Copy -- 'T ) | x | mk drop x ;\n\
         : main ( -- ) 5 g . ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: cannot `drop` a value of type `owning [ -- ]` in `g` (line 2): an owning closure disposes its captures by running, so `call` it -- no destructor can run a closure body"
    );
}

/// The call-once lifecycle needs no checker code of its own: the marker makes
/// the value linear, and the pre-existing consumed-on-every-path check does the
/// rest. Calling on one arm only is that error verbatim; calling on both arms
/// builds, runs, and disposes once whichever arm ran.
#[test]
fn an_owning_closure_must_be_called_on_every_path() {
    let one_arm = Scratch::write(
        "one-arm",
        &format!(
            "{SPY_DEF}: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n\
             : main ( -- ) 7 Spy mk | q | True ~[ q call ] ~[ 0 . ] if ;\n"
        ),
    );
    let err = build_error(one_arm.path());
    assert!(
        err.contains("linear value `q` is not consumed on every path"),
        "unexpected message: {err}"
    );

    let both_arms = Scratch::write(
        "both-arms",
        &format!(
            "{SPY_DEF}: mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n\
             : main ( -- ) 7 Spy mk | q | True ~[ q call ] ~[ q call ] if ;\n"
        ),
    );
    assert_eq!(build_and_run(both_arms.path()), "drop 7\n");
}

/// The mirror obligation of lifting the R12 ban on consuming an enclosing
/// linear local: the literal *must* consume every linear value it captures. A
/// body that only reads one through a borrow leaves the frame still owning the
/// value while the env holds its bytes, which is a double disposal -- and the
/// R12 lift is exactly what would otherwise let it through silently.
#[test]
fn an_owning_closure_that_does_not_consume_a_linear_capture_is_rejected() {
    let prog = Scratch::write(
        "unconsumed-capture",
        &format!(
            "{SPY_DEF}: mk ( Spy -- owning [ -- ] ) | s | [ &s &tag @ . ] ;\n\
             : main ( -- ) 7 Spy mk call ;\n"
        ),
    );
    let err = build_error(prog.path());
    assert!(
        err.contains("an `owning` closure captures `s`, a linear `Spy`, without consuming it")
            && err.contains("in `mk` (line 3)"),
        "unexpected message: {err}"
    );
}

/// The same obligation at an `if`-join rather than a lone literal: each arm's
/// materialized closure is checked for its own consumption of `s` before the
/// two arms are joined (`check_branch_join`), not only for a single top-level
/// literal. Here the first arm (`a`) is the one that leaves `s` unconsumed.
#[test]
fn an_owning_closure_joins_first_arm_unconsumed_is_rejected() {
    let prog = Scratch::write(
        "join-first-arm-unconsumed",
        &format!(
            "{SPY_DEF}: mk ( Spy Bool -- owning [ -- ] ) | s c |\n  \
               c ~[ [ &s &tag @ . ] ] ~[ [ s drop ] ] if\n\
             ;\n\
             : main ( -- ) 7 Spy True mk call ;\n"
        ),
    );
    let err = build_error(prog.path());
    assert!(
        err.contains("an `owning` closure captures `s`, a linear `Spy`, without consuming it"),
        "unexpected message: {err}"
    );
}

/// The mirror of the above: the *second* arm (`b`) is the one that leaves `s`
/// unconsumed, reached only once the first arm's own check has already passed.
#[test]
fn an_owning_closure_joins_second_arm_unconsumed_is_rejected() {
    let prog = Scratch::write(
        "join-second-arm-unconsumed",
        &format!(
            "{SPY_DEF}: mk ( Spy Bool -- owning [ -- ] ) | s c |\n  \
               c ~[ [ s drop ] ] ~[ [ &s &tag @ . ] ] if\n\
             ;\n\
             : main ( -- ) 7 Spy True mk call ;\n"
        ),
    );
    let err = build_error(prog.path());
    assert!(
        err.contains("an `owning` closure captures `s`, a linear `Spy`, without consuming it"),
        "unexpected message: {err}"
    );
}

/// The admission lift is narrowed to *linear* captures on purpose. A `Copy`
/// aggregate is not moved by the capture, so admitting it would leave the frame
/// and the env each holding a copy with no rule saying which is authoritative
/// -- and its problem (a pointer into dead frame storage) is not one a disposal
/// obligation addresses. It keeps the plain past-owning-frame rejection, and
/// without the remedy line, at an `owning` boundary too.
#[test]
fn a_copy_aggregate_capture_at_an_owning_boundary_still_rejects() {
    let prog = Scratch::write(
        "copy-aggregate",
        "type: P x i64 y i64 ;\n\
         : mk ( -- owning [ -- i64 ] ) 1 2 P | p | [ p .x ] ;\n\
         : main ( -- ) mk call . ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: an escaping closure captures `p`, a local of this frame, whose storage does not survive the return (line 2)"
    );
}

/// A declared `owning` *parameter*, which reaches `ir_type_of` through
/// signature lowering rather than through any materialization boundary. The
/// caller's literal is materialized at the argument slot with the owning env,
/// and the callee's `call` disposes the capture.
#[test]
fn a_declared_owning_parameter_takes_a_literal_and_disposes_its_capture() {
    let prog = Scratch::write(
        "owning-param",
        &format!(
            "{SPY_DEF}: use ( owning [ -- ] -- ) call ;\n\
             : main ( -- ) 7 Spy | s | [ s drop ] use ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n");
}

/// A *spliced* word may not declare one. The splice route never materializes:
/// it inlines the caller's literal in place and compares only the
/// inline-versus-plain axis, so with this rejection removed a plain `[ 1 . ]`
/// literal satisfies an `owning` slot and builds and runs -- the type
/// inequality the whole containment story rests on, silently gone.
#[test]
fn a_spliced_word_may_not_declare_an_owning_parameter() {
    let prog = Scratch::write(
        "spliced-owning",
        ": f inline ( owning [ -- ] -- ) | q | q call ;\n: main ( -- ) [ 1 . ] f ;\n",
    );
    let err = build_error(prog.path());
    assert!(
        err.contains("`f` is spliced (`inline`) and declares `owning [ -- ]`"),
        "unexpected message: {err}"
    );
}

/// A non-capturing owning literal: nothing to own, no allocation, and still
/// linear, so it must be called. The shape phase 2 could only reject, and the
/// one a capture-side guard alone would never have seen (`body_captures_enclosing`
/// is false for it).
#[test]
fn a_non_capturing_owning_literal_builds_and_runs() {
    let prog = Scratch::write(
        "non-capturing",
        ": mk ( -- owning [ -- ] ) [ 1 . ] ;\n: main ( -- ) mk call ;\n",
    );
    assert_eq!(build_and_run(prog.path()), "1\n");
}

/// The `if`-join at an `owning` output: two *different* literals, each
/// materialized in its own arm and phi-joined, with the flavour read off the
/// declared output rather than off either literal.
#[test]
fn two_differing_arms_joining_at_an_owning_output_each_materialize() {
    let prog = Scratch::write(
        "join-owning",
        &format!(
            "{SPY_DEF}: mk ( Spy Bool -- owning [ -- ] )\n  \
               | s c |\n  \
               c ~[ [ s drop ] ] ~[ [ s drop 9 . ] ] if\n\
             ;\n\
             : main ( -- ) 7 Spy True mk call 8 Spy False mk call ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\ndrop 8\n9\n");
}

// ---------------------------------------------------------------------------
// Emitted-IL assertions.
// ---------------------------------------------------------------------------

/// Emit the QBE IL for a self-contained source, through the same path
/// `tests/qbe_baseline.rs` uses.
fn emit_il(src: &str) -> String {
    use sooth::{backend, check, ir, lexer, test_support};
    let tokens = lexer::lex(src).expect("source should lex");
    let mut module = test_support::parse_with_core(&tokens).expect("source should parse");
    check::check(&mut module).expect("source should check");
    let ir = ir::lower(&module).expect("source should lower");
    backend::qbe::emit(&ir).expect("QBE IL emission should succeed")
}

/// One function block of the emitted IL, by its symbol. Matched on ` $symbol(`
/// rather than on `function $symbol(`, since a return type sits between the two
/// (`export function :Q0 $mk(...)`).
fn function_block<'a>(il: &'a str, symbol: &str) -> &'a str {
    let head = format!(" ${symbol}(");
    let start = il
        .find(&head)
        .unwrap_or_else(|| panic!("expected a `{symbol}` function in:\n{il}"));
    let end = il[start..]
        .find("\n}\n")
        .map(|rel| start + rel)
        .expect("the function block closes");
    &il[start..end]
}

/// The env free, asserted the only way it is checkable: a leaked heap block has
/// no observable effect in a normal run and the harness has no allocator
/// accounting, so the assertion is on the *emitted body*. Stubbing the free out
/// fails here.
///
/// The matching allocation is asserted at the boundary in the same breath, since
/// an emitted `sooth_free` over a block nobody allocated would satisfy half of
/// this on its own.
#[test]
fn an_owning_closure_allocates_its_env_and_the_body_frees_it() {
    let il = emit_il(
        "type: Spy tag i64 ;\n\
         : drop ( Spy -- ) Spy> . ;\n\
         : mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n\
         : main ( -- ) 7 Spy mk call ;\n",
    );
    let boundary = function_block(&il, "mk");
    assert!(
        boundary.contains("call $sooth_alloc("),
        "the boundary allocates the env block: {boundary}"
    );
    assert!(
        !boundary.contains("call $sooth_free("),
        "the boundary must not free the block it just handed to the closure: {boundary}"
    );
    let body = function_block(&il, "mk__quot0");
    assert!(
        body.contains("call $sooth_free("),
        "the compiled body frees its own env block: {body}"
    );
}

/// The non-obvious reach path into `ir_type_of`, which is why the guard phase 3
/// lifted had to cover declarations and not only materialization. An `impl:`
/// block's member is a synthesized `WordDef` under an unforgeable
/// `member;Trait;Type` name, inheriting the trait member's signature with `'T`
/// substituted -- so an `owning` slot arrives in a *lowered* signature without
/// any word in the source spelling it, and it is lowered whether or not
/// anything calls it. Asserting the emitted parameter spelling is the point:
/// `ir_type_of` ran over the owning slot and answered the quotation aggregate.
#[test]
fn an_owning_parameter_inherited_by_an_impl_member_lowers_to_the_quotation_aggregate() {
    let il = emit_il(
        "type: W x i64 ;\n\
         trait: Own 'T\n  use ( 'T owning [ -- ] -- )\n;\n\
         impl: Own for W\n  : use | w q | w drop q call ;\n;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        il.contains("type :Q0 = { l, l }"),
        "the owning slot interned the quotation signature: {il}"
    );
    assert!(
        il.contains("(:W %v0, :Q0 %v1)"),
        "the synthesized member lowers its owning parameter as the quotation aggregate: {il}"
    );
}

/// The invariant every program that exists today depends on: a plain quotation
/// is unchanged. Same two-word `{ l, l }` aggregate, the `code` slot still at
/// offset 0, the env still the capture's live value stored inline -- and no
/// allocation anywhere, which is what would show if the owning path had been
/// generalized to every closure.
#[test]
fn a_plain_quotation_keeps_its_two_word_layout_and_gains_no_allocation() {
    let il = emit_il(
        ": mk ( i64 -- [ -- i64 ] ) | n | [ n ] ;\n\
         : main ( -- ) 5 mk call . ;\n",
    );
    assert!(
        il.contains("type :Q0 = { l, l }"),
        "the quotation aggregate is unchanged: {il}"
    );
    let boundary = function_block(&il, "mk");
    assert!(
        !boundary.contains("call $sooth_alloc("),
        "a plain closure allocates nothing: {boundary}"
    );
    // The `code` slot is written at offset 0 off the freshly allocated
    // quotation value, exactly as before the slice.
    assert!(
        boundary.contains("=l alloc8 16"),
        "the quotation value is a 16-byte, 8-aligned frame slot: {boundary}"
    );
    let body = function_block(&il, "mk__quot0");
    assert!(
        !body.contains("call $sooth_free("),
        "a plain closure's body frees nothing: {body}"
    );
}
