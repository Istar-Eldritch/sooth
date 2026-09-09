//! P7b.S6d-PREREQ goldens, Phase 1 and Phase 2: reference-bearing aggregates,
//! and the multi-output return bundle when one of its members carries a slice.
//!
//! Phase 1: a shared `Slice[T]` is a legal struct field and enum payload now
//! (REQ-1/2/5, Ruling A), which means every escape ban stated over
//! `contains_reference` has to hold over the *containing* value as well
//! (REQ-4), and every in-frame site that hands the borrow from one value to
//! another has to propagate its provenance (REQ-4d's seven sites) or the
//! value it produces launders the borrow.
//!
//! Phase 2: the return bundle a multi-output call synthesizes (REQ-3) is laid
//! out and passed like any other struct once a member is a slice, over both
//! the monomorphic and polymorphic (per-instantiation and splice-record)
//! interning sites, with no destructor synthesized over the bundle itself.
//!
//! Fixtures are written verbatim (no harness-appended imports) so every
//! `(line N)` in an assertion means the line the fixture literally shows.
//! Harness style from `tests/phase7b_slice6.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs6dp-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs6dp ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write("main.sth", src);
    (t, entry)
}

fn build_error(tag: &str, src: &str) -> String {
    let (_t, entry) = fixture(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        !build.status.success(),
        "build should have failed, stdout: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn build_and_run(tag: &str, src: &str) -> String {
    let (_t, entry) = fixture(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should have succeeded, stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(entry.with_extension(""))
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the binary should exit zero");
    String::from_utf8(run.stdout)
        .expect("stdout should be utf8")
        .trim()
        .to_string()
}

/// The one wording every declaration-site escape rejection shares.
const STORED: &str = "error: a reference cannot be stored:";

// ---------------------------------------------------------------------------
// G1: the capability itself.
// ---------------------------------------------------------------------------

/// G1: a declared struct field of type shared `Slice[i64]` lays out, is
/// constructed, is projected back out through `@`, and is read to the end of
/// the view. The loop is the hand-written `times`/`&>` shape from
/// `examples/slices.sth` -- no `Iterator` impl is used or implied.
///
/// `total` takes `Window` by value and never `drop`s it, which is correct as
/// written and not an omission: a shared-slice-only container with a plain
/// `usize` field is `Copy` (Ruling A), so it carries no drop obligation.
#[test]
fn g1_declared_slice_field_builds_and_runs() {
    let out = build_and_run(
        "g1",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::combinators c | times | ;

type: Window view Slice[i64] lo usize ;

: total inline ( Window -- i64 )
  |w|
  &w &view @ | s |
  0 s len >i64 ~[ |i| s i >usize &> @ add ] times
;

: main ( -- )
  0 5 fill | a |
  &a slice 0 >usize Window | w |
  w total .
  a drop
;
",
    );
    assert_eq!(out, "0");
}

/// G1's enum twin: an enum *payload* field of slice type, constructed, matched
/// through owning arms, and its payload projected. Build-and-run only -- a
/// passing build proves nothing about whether the arm-bound payload is
/// borrow-tracked, which is `g_alias_iv`'s job.
#[test]
fn g1_enum_payload_slice_field_builds_and_runs() {
    let out = build_and_run(
        "g1e",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Cell | Empty | Full v Slice[i64] ;

: main ( -- )
  0 4 fill | a |
  &a slice Full | c |
  c ~[ ( Empty ) drop 0 >usize ] ~[ ( Full ) &v @ swap drop len ] Cell? >i64 .
  a drop
;
",
    );
    assert_eq!(out, "4");
}

/// G1-nested (REQ-5): the admit-and-taint predicate is recursive, so a struct
/// nesting a slice-bearing struct is admitted too -- the naive "reject unless
/// the field type is exactly `Type::Slice`" would reject this composition.
#[test]
fn g1_nested_slice_bearing_struct_field_builds_and_runs() {
    let out = build_and_run(
        "g1n",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Holder val Slice[i64] ;
type: Outer h Holder ;

: main ( -- )
  0 4 fill | a |
  &a slice Holder Outer | o |
  &o &h &val @ len >i64 .
  a drop
;
",
    );
    assert_eq!(out, "4");
}

/// G1-input (Ruling B): the capability is body-local. A slice-bearing
/// aggregate may not be an *input* to a non-combinator word either, and the
/// rejection is the pre-existing input arm's own unlocated, type-naming text
/// -- not the located, field-naming class.
#[test]
fn g1_input_slice_bearing_aggregate_to_a_noninline_word_is_error() {
    let err = build_error(
        "g1i",
        "\
import: intrinsics * ;

type: Window view Slice[i64] lo usize ;

: takes ( Window -- ) drop ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} `takes` declares the input `Window`, which contains a reference\n  an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate"
        )),
        "{err}"
    );
    assert!(!err.contains("line"), "the input arm is unlocated: {err}");
}

/// G1-mut-hard-ban (Ruling A): `!Slice[T]` stays a hard reject as a declared
/// field, with today's text unchanged. `@` is the only value-fetch through a
/// field reference and it gates on `is_copy` of the referent, which `!Slice`
/// is not, and Sooth has no move-out-of-a-field.
#[test]
fn g1_mutable_slice_field_stays_hard_banned() {
    let err = build_error(
        "g1m",
        "\
import: intrinsics * ;

type: MutHolder val !Slice[i64] ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} field `val` of type `MutHolder` has type `!Slice[i64]` (line 3, col 7)\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow"
        )),
        "{err}"
    );
}

/// G2 (REQ-4a): a non-inline word declaring a slice-bearing aggregate
/// *output* is rejected by the pre-existing output arm, now reached over an
/// aggregate rather than a bare reference type. The text is reused verbatim.
#[test]
fn g2_noninline_slice_bearing_output_is_error() {
    let err = build_error(
        "g2",
        "\
import: intrinsics * ;

type: Window view Slice[i64] lo usize ;

: mk ( -- Window ) 0 4 fill |a| &a slice 0 >usize Window ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} `mk` declares the output `Window`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        )),
        "{err}"
    );
}

/// G-poly-twin (REQ-4a): the poly signature audit's `PolyType::Concrete` arm
/// sees a slice-bearing aggregate the same way the monomorphic check does.
/// The witness is a *declared concrete* output; a declared generic slot is
/// `PolyType::Var`, which the audit returns `false` for unconditionally, and
/// no per-instantiation audit exists (a named gap, deferred).
#[test]
fn g_poly_twin_concrete_slice_bearing_output_is_error() {
    let err = build_error(
        "gpt",
        "\
import: intrinsics * ;

type: Window view Slice[i64] lo usize ;

: w ( 'T -- Window ) drop 0 4 fill |a| &a slice 0 >usize Window ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!("{STORED} `w` declares the output `Window`")),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// REQ-4b: the materialization fence (Ruling D).
// ---------------------------------------------------------------------------

/// G-capture-fence (Ruling D): the exact fixture that ICEs at `qbe.rs:526`
/// before this slice -- a bare `Slice[i64]` local captured into a materialized
/// closure at a **plain, non-escaping, in-frame** boundary, which neither G3
/// twin reaches. The golden pins the fence, not the ICE: the cause is an
/// IR-encoding limit (one word per capture, two words per slice), so the
/// message is the fence's own, not an escape rejection's.
#[test]
fn g_capture_fence_bare_slice_in_frame_capture_is_error() {
    let err = build_error(
        "gcf",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: use ( [ -- ] -- ) call ;

: main ( -- )
  0 4 fill | a |
  &a slice | v |
  [ v len >i64 . ] use
  a drop
;
",
    );
    assert!(
        err.contains("error: a closure cannot capture `v`, whose type `Slice[i64]` carries a slice (line 10)\n  a captured value gets one word in the closure's env block and a slice is two; pass the view as an argument instead"),
        "{err}"
    );
}

/// G3 (i): the same fence at an **escaping** boundary, over a slice-bearing
/// *aggregate* rather than a bare slice. Under Ruling D the fence is
/// unconditional, so this twin is witnessed by the same mechanism as
/// `g_capture_fence` -- deliberately, since the one-word env slot cannot hold
/// a two-word value on any path.
#[test]
fn g3_escaping_capture_of_a_slice_bearing_aggregate_is_error() {
    let err = build_error(
        "g3e",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: mk ( -- [ -- i64 ] )
  0 4 fill | a |
  &a slice 0 >usize Window | w |
  [ &w &view @ len >i64 ]
;

: main ( -- ) mk call . ;
",
    );
    assert!(
        err.contains(
            "error: a closure cannot capture `w`, whose type `Window` carries a slice (line 10)"
        ),
        "{err}"
    );
}

/// G3 (ii), the owning-closure twin: `owning [ ... ]` moves its captures into
/// a heap env, which answers the *escape* question but not the encoding one,
/// so the fence still fires. This is the twin the earlier draft wanted to
/// close with an `owning && linear` exclusion; the blanket fence subsumes it.
#[test]
fn g3_owning_capture_of_a_slice_bearing_aggregate_is_error() {
    let err = build_error(
        "g3o",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: mk ( -- owning [ -- ] )
  0 4 fill | a |
  &a slice 0 >usize Window | w |
  [ &w &view @ len >i64 . a drop ]
;

: main ( -- ) mk call ;
",
    );
    assert!(
        err.contains(
            "error: a closure cannot capture `w`, whose type `Window` carries a slice (line 10)"
        ),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// REQ-4d: in-frame borrow propagation, one golden per site.
// ---------------------------------------------------------------------------

/// G-alias-ii (site 1, the write side): a shared slice packed into a `Holder`,
/// then a second `&!` of the root array while the `Holder` is still live (read
/// afterwards, since `conflicts`/`live.dead` are last-use). Without site 1's
/// deriv forward onto the constructed slot the `Holder` is invisible and the
/// second `&!` is admitted.
#[test]
fn g_alias_ii_second_mutable_borrow_while_a_holder_is_live_is_error() {
    let err = build_error(
        "ga2",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Holder val Slice[i64] ;

: main ( -- )
  0 4 fill | a |
  &a slice Holder | h |
  &!a | r |
  r 0 >usize &!> 7 !
  &h &val @ len >i64 .
  a drop
;
",
    );
    assert!(
        err.contains("error: `&!a` conflicts with a live borrow of `a` in `main` (line 10, col 3)"),
        "{err}"
    );
    assert!(
        err.contains("the shared borrow taken at line 9, col 3 is still live"),
        "{err}"
    );
}

/// G-alias-iii (site 2, the read side): the slice is `@`-projected out of the
/// aggregate -- G1's own idiom -- and a second `&!` of the root is taken while
/// that projection is live. Without `@`'s deriv forward the fetched `s` is
/// invisible to every guard and this golden would pass as an inert accept.
///
/// The chain runs through site 3 as well: `&w` inherits the `Window`
/// binding's provenance, so the deriv `@` forwards is still rooted at `a`.
#[test]
fn g_alias_iii_second_mutable_borrow_while_a_projection_is_live_is_error() {
    let err = build_error(
        "ga3",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: main ( -- )
  0 4 fill | a |
  &a slice 0 >usize Window | w |
  &w &view @ | s |
  &!a | r |
  r 0 >usize &!> 7 !
  s 0 >usize &> @ .
  a drop
;
",
    );
    assert!(
        err.contains("error: `&!a` conflicts with a live borrow of `a` in `main` (line 11, col 3)"),
        "{err}"
    );
}

/// G-alias-iv (site 6 plus the `Type::Variant` taint arm): the payload is
/// extracted through an owning eliminator, whose arms bind the scrutinee at
/// `Type::Variant`. This fails without either half of the fix -- the taint arm
/// (which reports a slice-bearing variant reference-bearing at all) or the
/// anonymous-receiver arm's deriv forward -- which is why G1's enum twin
/// cannot stand in for it. Both arms are spelled, one `~[...]` each with
/// uniform outputs; the single-arm spelling leaves the variant on the stack.
#[test]
fn g_alias_iv_second_mutable_borrow_while_a_variant_payload_is_live_is_error() {
    let err = build_error(
        "ga4",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Cell | Empty | Full v Slice[i64] ;

: main ( -- )
  0 4 fill | a |
  &a slice Full | c |
  c ~[ ( Empty ) drop &a slice ] ~[ ( Full ) &v @ swap drop ] Cell? | s |
  &!a | r |
  r 0 >usize &!> 7 !
  s 0 >usize &> @ .
  a drop
;
",
    );
    assert!(
        err.contains("error: `&!a` conflicts with a live borrow of `a` in `main` (line 11, col 3)"),
        "{err}"
    );
}

/// G-alias-v (site 7): naming a reference-bearing aggregate into a second
/// local. A *reference*-typed local takes the reborrow path, which inherits;
/// an aggregate takes the name-read push, which carried no deriv at all, so
/// `w |x|` was a one-token launder.
#[test]
fn g_alias_v_second_mutable_borrow_while_a_renamed_aggregate_is_live_is_error() {
    let err = build_error(
        "ga5",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: main ( -- )
  0 4 fill | a |
  &a slice 0 >usize Window | w |
  w | x |
  &!a | r |
  r 0 >usize &!> 7 !
  &x &view @ len >i64 .
  a drop
;
",
    );
    assert!(
        err.contains("error: `&!a` conflicts with a live borrow of `a` in `main` (line 11, col 3)"),
        "{err}"
    );
    assert!(
        err.contains("the shared borrow taken at line 9, col 3 is still live"),
        "{err}"
    );
}

/// G-poly-launder twin (i) (site 4), the today-red regression test: this
/// program needs **no new capability** and printed `99` before this slice --
/// a write through the exclusive `&!a` observed through the shared view `s`,
/// which the `( 'T -- 'T )` pass-through laundered. Deleting `thru` from the
/// same program already produced the rejection, which is what proves the
/// dispatch push is the laundering site.
#[test]
fn g_poly_launder_bare_slice_passthrough_is_error() {
    let err = build_error(
        "gpl1",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: thru ( 'T -- 'T ) ;

: main ( -- )
  0 4 fill | a |
  &a slice thru | s |
  &!a | r |
  r 0 >usize &!> 99 !
  s 0 >usize &> @ .
  a drop
;
",
    );
    assert!(
        err.contains("error: `&!a` conflicts with a live borrow of `a` in `main` (line 10, col 3)"),
        "{err}"
    );
}

/// G-poly-launder twin (ii) (site 4): the same shape with the view packed into
/// a `Window` first -- the aggregate merely widens the same hole.
#[test]
fn g_poly_launder_aggregate_passthrough_is_error() {
    let err = build_error(
        "gpl2",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: thru ( 'T -- 'T ) ;

: main ( -- )
  0 4 fill | a |
  &a slice 0 >usize Window thru | w |
  &!a | r |
  r 0 >usize &!> 99 !
  &w &view @ 0 >usize &> @ .
  a drop
;
",
    );
    assert!(
        err.contains("error: `&!a` conflicts with a live borrow of `a` in `main` (line 12, col 3)"),
        "{err}"
    );
}

/// G-distinct-root (Ruling F): a two-slice-field struct built from views of
/// two *different* arrays. `Deriv.owned_root` is one place, so such a value
/// could protect at most one of them -- and silently leaving the second
/// re-borrowable is the laundering hole again, at arity two.
#[test]
fn g_distinct_root_pair_over_two_arrays_is_error() {
    let err = build_error(
        "gdr1",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Pair a Slice[i64] b Slice[i64] ;

: main ( -- )
  0 4 fill | p |
  0 4 fill | q |
  &p slice &q slice Pair | r |
  &r &a @ len >i64 .
  p drop q drop
;
",
    );
    assert!(
        err.contains("error: `Pair` would leave a value viewing both `p` and `q` in `main` (line 10, col 21)"),
        "{err}"
    );
    assert!(
        err.contains("a value tracks a single borrowed place, so two views of two different places cannot be packed into one"),
        "{err}"
    );
}

/// Ruling F's boundary, not a blanket ban on multi-slice aggregates: the same
/// `Pair` built from two views of **one** array builds and runs, since the
/// operands agree on the root and the choice of which deriv to keep has no
/// observable content.
#[test]
fn g_distinct_root_pair_over_one_array_builds_and_runs() {
    let out = build_and_run(
        "gdr2",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Pair a Slice[i64] b Slice[i64] ;

: main ( -- )
  0 4 fill | p |
  &p slice &p slice Pair | r |
  &r &a @ len >i64 .
  p drop
;
",
    );
    assert_eq!(out, "4");
}

/// G-store-escape (site 5, rule (i)): the cross-frame stash. Ruling B's input
/// ban exempts a *top-level* reference, so a non-inline word may take
/// `&!Window` and store into its slice field a view of its own frame's array
/// -- a dangling view produced without crossing any REQ-4a/b/c boundary. The
/// `i64`-field analogue of this program builds and runs, so the golden pins a
/// new rejection on a reachable path rather than a hypothetical one.
#[test]
fn g_store_escape_cross_frame_stash_is_error() {
    let err = build_error(
        "gse",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: stash ( &!Window -- )
  | w |
  0 4 fill | own |
  w &!view &own slice !
  own drop
;

: main ( -- )
  0 4 fill | a |
  &a slice 0 >usize Window | w |
  &!w stash
  &w &view @ len >i64 .
  a drop
;
",
    );
    assert!(
        err.contains("error: `!` cannot store the borrow-carrying `Slice[i64]` in `stash` (line 10, col 23): the receiver is not rooted in this frame"),
        "{err}"
    );
    assert!(
        err.contains("the container outlives this frame, so the view it would hold points into storage that does not"),
        "{err}"
    );
}

/// G-store-distinct-root (site 5, rule (ii) under Ruling F): in-frame, a
/// `Window` rooted at `b` receiving a view of `a`. Left unrejected, site 2's
/// `@` forward would then propagate the *wrong* root on every read, which is
/// worse than propagating none.
#[test]
fn g_store_distinct_root_field_store_of_another_view_is_error() {
    let err = build_error(
        "gsdr",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: main ( -- )
  0 4 fill | b |
  0 4 fill | a |
  &b slice 0 >usize Window | w |
  &!w &!view &a slice !
  &w &view @ len >i64 .
  a drop b drop
;
",
    );
    assert!(
        err.contains(
            "error: `!` would leave a value viewing both `b` and `a` in `main` (line 11, col 23)"
        ),
        "{err}"
    );
}

/// Rule (ii)'s accept half, over the hoisted spelling: a `Slice` view of the
/// container's own root taken *before* the container exists, bound to a
/// local, then stored through the container's own `&!` -- no fresh `&b`
/// competes with the live `&!w`, so the exclusivity scan at site 3 has
/// nothing to reject and the join at rule (ii) runs. Storing the same root a
/// value already carries is a no-op on its provenance, so this builds and
/// runs rather than tripping `distinct_root_error`.
#[test]
fn g_store_distinct_root_field_store_of_same_root_is_accepted() {
    let out = build_and_run(
        "gsdr2",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: main ( -- )
  0 4 fill | b |
  &b slice | s |
  &b slice 0 >usize Window | w |
  &!w &!view s !
  &w &view @ len >i64 .
  b drop
;
",
    );
    assert_eq!(out, "4");
}

/// A narrower fact than the retracted "no accepted store" claim above: *this
/// particular spelling* -- a fresh `&b` competing with the already-live
/// `&!w` -- is rejected by the exclusivity scan rather than by Ruling F,
/// because `&!w` inherits the container's provenance (site 3) and so reads as
/// a live *mutable* borrow of `b`, which a shared `&b` then conflicts with.
/// `g_store_distinct_root_field_store_of_same_root_is_accepted` is the
/// accepted twin: hoist the view out to a local first and rule (ii)'s join
/// runs instead.
#[test]
fn g_store_same_root_is_rejected_by_the_exclusivity_scan() {
    let err = build_error(
        "gssr",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: main ( -- )
  0 4 fill | b |
  &b slice 0 >usize Window | w |
  &!w &!view &b slice !
  &w &view @ len >i64 .
  b drop
;
",
    );
    assert!(
        err.contains("error: `&b` conflicts with a live borrow of `b` in `main` (line 10, col 14)"),
        "{err}"
    );
    assert!(
        err.contains("the mutable borrow taken at line 10, col 3 is still live"),
        "{err}"
    );
}

/// P1-1: `live_borrow_of` (`engine.rs`) is the *consuming* twin of site 3's
/// exclusivity scan, and until this fix it was keyed on `owned_root` alone --
/// exactly the disjunct the scan itself needed widening for. `&!m` on a
/// slice-bearing linear struct inherits the container's root (site 3), so its
/// deriv is `place: "m", owned_root: "a"`; consuming `m` via `drop` while that
/// borrow is live has to be caught by matching `d.place`, since `d.owned_root`
/// names the array, not `m`. Left unwidened this builds and runs silently:
/// `m drop` frees `m`'s storage while `r` still derives from it.
#[test]
fn g_consume_of_borrowed_place_reaches_through_site3_inheritance_is_error() {
    let err = build_error(
        "gcbp",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Mixed view Slice[i64] cell ^i64 ;

: main ( -- )
  0 4 fill | a |
  &a slice 7 ^ Mixed | m |
  &!m | r |
  m drop
  r &!view @ len >i64 .
  a drop
;
",
    );
    assert!(
        err.contains("error: cannot consume the borrowed local `m` of type `Mixed` in `main` (line 11, col 3)"),
        "{err}"
    );
    assert!(
        err.contains("the mutable borrow taken at line 10, col 3 is still live"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// REQ-4c: the closed enumeration of guard sites, each re-verified over a
// slice-bearing type. None of these guards changed; the point is that each
// still fires now that the declaration sweep admits the field.
// ---------------------------------------------------------------------------

/// G-sweep-array: a slice-bearing type as an interned array *element*.
#[test]
fn g_sweep_array_slice_bearing_element_is_error() {
    let err = build_error(
        "gsa",
        "\
import: intrinsics * ;

type: Holder val Slice[i64] ;

: mk inline ( Holder -- array[Holder 3] ) 3 fill ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} element of array type `array[Holder 3]` has type `Holder`"
        )),
        "{err}"
    );
}

/// G-sweep-cell: a slice-bearing type as an interned cell payload. The only
/// heap-storage guard -- `contains_reference` deliberately does not follow
/// `Type::OwnedCell` payloads (a cell may close a type cycle), so this sweep
/// alone keeps a slice out of heap storage.
#[test]
fn g_sweep_cell_slice_bearing_payload_is_error() {
    let err = build_error(
        "gsc",
        "\
import: intrinsics * ;

type: Holder val Slice[i64] ;

: keep inline ( ^Holder -- ^Holder ) ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} payload of cell type `^Holder` has type `Holder`"
        )),
        "{err}"
    );
}

/// G-sweep-slice-element (Ruling E): the slice-element gate stays a hard
/// reject, so a slice whose *element* is reference-shaped is refused even
/// though the struct sweep now admits a slice as a field. Witnessed directly
/// over `Slice[&i64]`, a parseable, interned spelling -- the round-2 draft's
/// "`Holder` holding a `&T`" witness is unconstructible, since a `&T` field
/// stays hard-banned and that `Holder` could never be declared.
#[test]
fn g_sweep_slice_element_reference_element_is_error() {
    let err = build_error(
        "gsse",
        "\
import: intrinsics * ;

: w inline ( Slice[&i64] -- Slice[&i64] ) ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} element of slice type `Slice[&i64]` has type `&i64`"
        )),
        "{err}"
    );
}

/// Ruling E's other consequence, pinned so it is not mistaken for an
/// oversight: `type: Holder val Slice[Holder] ;` stays undeclarable. The
/// recursion check passes it (a slice closes no by-value size cycle) and the
/// relaxed struct sweep admits `Holder` as a field type, but the element gate
/// rejects `Slice[Holder]` because `Holder` is reference-bearing.
#[test]
fn g_sweep_slice_element_recursive_holder_stays_rejected() {
    let err = build_error(
        "gssr2",
        "\
import: intrinsics * ;

type: Holder val Slice[Holder] ;

: main ( -- ) 1 drop ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} element of slice type `Slice[Holder]` has type `Holder`"
        )),
        "{err}"
    );
}

/// G-sweep-fill: `fill`'s element gate -- a construction site with no
/// declaration anywhere for the declaration sweep to have caught.
#[test]
fn g_sweep_fill_slice_element_is_error() {
    let err = build_error(
        "gsf",
        "\
import: intrinsics * ;

: main ( -- )
  0 4 fill | a |
  &a slice 3 fill drop
  a drop
;
",
    );
    assert!(
        err.contains("error: a reference cannot be stored in `main` (line 5)\n  the element `fill` would store has type `Slice[i64]`"),
        "{err}"
    );
}

/// G-sweep-cellctor: `^`'s payload gate, the other declaration-free
/// construction site.
#[test]
fn g_sweep_cellctor_slice_payload_is_error() {
    let err = build_error(
        "gscc",
        "\
import: intrinsics * ;

: main ( -- )
  0 4 fill | a |
  &a slice ^ drop
  a drop
;
",
    );
    assert!(
        err.contains("the payload `^` would store has type `Slice[i64]`"),
        "{err}"
    );
}

/// G-quot-effect-output: a materialized quotation declaring a slice-bearing
/// output, rejected by the quotation-effect twin of the word-level output arm.
#[test]
fn g_quot_effect_slice_bearing_output_is_error() {
    let err = build_error(
        "gqeo",
        "\
import: intrinsics * ;

type: Window view Slice[i64] lo usize ;

: use ( [ -- Window ] -- ) drop ;

: main ( -- ) [ ( -- Window ) 0 4 fill |b| &b slice 0 >usize Window ] use ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} `[ -- Window ]` declares the output `Window` in `main` (line 7)"
        )),
        "{err}"
    );
}

/// G-quot-effect-input (REQ-4c's addendum): the input arm this boundary did
/// not have. Its absence rested on "an aggregate input carrying a nested
/// reference is already rejected at its struct declaration", which REQ-5
/// falsifies for a shared-slice-bearing aggregate.
#[test]
fn g_quot_effect_slice_bearing_input_is_error() {
    let err = build_error(
        "gqei",
        "\
import: intrinsics * ;

type: Window view Slice[i64] lo usize ;

: use ( [ Window -- ] -- ) drop ;

: main ( -- ) [ ( Window -- ) drop ] use ;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} `[ Window -- ]` declares the input `Window`, which contains a reference in `main` (line 7)"
        )),
        "{err}"
    );
    assert!(
        err.contains(
            "an input may *be* a `&T`/`&!T`, but not carry one nested inside an aggregate"
        ),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// G6: the IR pin.
// ---------------------------------------------------------------------------

/// G6: the layout/codegen pin, over a **mixed** linear+slice struct (a slice
/// field alongside an owning-cell field) so a mutation deleting the slice's
/// linearity handling cannot hide behind the struct's own flag.
///
/// Three things at once: the slice field is spelled as the shared two-word
/// aggregate and QBE sees `:sooth.slice` declared before the struct that
/// references it; the cell field sits at offset 16, which is only true if the
/// slice field occupied two words; and the synthesized destructor touches
/// offset 16 alone, so drop really is a no-op over the slice slot even though
/// the containing struct is linear.
#[test]
fn g6_slice_field_lays_out_two_words_and_drop_skips_the_slot() {
    let (_t, entry) = fixture(
        "g6",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Mixed view Slice[i64] cell ^i64 ;

: main ( -- )
  0 4 fill | a |
  &a slice 7 ^ Mixed | m |
  &m &view @ len >i64 .
  m drop
  a drop
;
",
    );
    let ssa = sooth::driver::emit_ssa(&entry)
        .unwrap_or_else(|e| panic!("emitting the fixture should succeed: {e}"));

    let slice_at = ssa
        .find("type :sooth.slice = { l, l }")
        .expect("the shared slice aggregate should be declared");
    let mixed_at = ssa
        .find("type :Mixed = { :sooth.slice, l }")
        .unwrap_or_else(|| panic!("the slice field should be a member of `Mixed`: {ssa}"));
    assert!(
        slice_at < mixed_at,
        "QBE needs the member type declared first: {ssa}"
    );

    let drop_glue = ssa
        .split("export function $sooth_struct_drop_0(:Mixed %v0) {")
        .nth(1)
        .and_then(|rest| rest.split_once('}').map(|(body, _)| body.to_string()))
        .unwrap_or_else(|| panic!("`Mixed` is linear, so it gets a destructor: {ssa}"));
    assert!(
        drop_glue.contains("add %v0, 16"),
        "the cell field sits past the two-word slice slot: {drop_glue}"
    );
    assert!(
        drop_glue.contains("call $sooth_cell_drop_0"),
        "the cell field is still disposed: {drop_glue}"
    );
    assert_eq!(
        drop_glue.matches("add %v0,").count(),
        1,
        "drop is a no-op over the slice slot: only the cell field is reached: {drop_glue}"
    );
}

// ---------------------------------------------------------------------------
// Phase 2 (REQ-3): the synthesized return bundle. Unlike a declared aggregate
// (Ruling A) a bundle may carry a `!Slice[T]` too -- its unpack is positional
// at the return boundary, so `@`'s `is_copy` gate is never involved. The
// bundle is interned after every declaration check, so IR layout and this
// calling convention are its only gates.
// ---------------------------------------------------------------------------

/// G4: the inline `>= 2`-output slice word, verbatim from the spec. This exact
/// fixture panics at `layout.rs:271` without the two-word slot arm.
#[test]
fn g4_inline_two_output_slice_word_builds_and_runs() {
    let out = build_and_run(
        "g4",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: two-out inline ( Slice[i64] -- i64 Slice[i64] )
  |s| 0 s
;

: main ( -- )
  0 3 fill | a |
  &a slice two-out | x r |
  x .
  a drop
;
",
    );
    assert_eq!(out, "0");
}

/// G5: G4's body without `inline`. The output arm of
/// `check_reference_free_signature` rejects it with the same unlocated,
/// type-naming text G2 gets -- byte-identical, since both reach the one call
/// site (REQ-6). Asserted against G2's own wording, not a paraphrase.
#[test]
fn g5_noninline_two_output_slice_word_is_error() {
    let err = build_error(
        "g5",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: two-out ( Slice[i64] -- i64 Slice[i64] )
  |s| 0 s
;

: main ( -- )
  0 3 fill | a |
  &a slice two-out | x r |
  x .
  a drop
;
",
    );
    assert!(
        err.contains(&format!(
            "{STORED} `two-out` declares the output `Slice[i64]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        )),
        "{err}"
    );
}

/// G4's capturing-closure twin: unpacking a slice off the bundle produces an
/// ordinary bare-slice local, so Ruling D's fence still owns it. Worth its own
/// golden because the bundle is the one path that hands a program a slice it
/// never spelled a `slice` call for.
#[test]
fn g4_capture_of_a_bundle_output_slice_is_error() {
    let err = build_error(
        "g4c",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: pair ( 'T -- i64 'T )
  0 swap
;

: use ( [ -- ] -- ) call ;

: main ( -- )
  0 3 fill | a |
  &a slice pair | x r |
  x .
  [ r len >i64 . ] use
  a drop
;
",
    );
    assert!(
        err.contains("error: a closure cannot capture `r`, whose type `Slice[i64]` carries a slice (line 15)\n  a captured value gets one word in the closure's env block and a slice is two; pass the view as an argument instead"),
        "{err}"
    );
}

/// G-poly-bundle: the per-instantiation bundle site. A declared `'T` output is
/// `PolyType::Var`, which the signature audit returns `false` for, so this is
/// the one shape that reaches a real (non-spliced) call returning a bundle
/// with a slice in it -- the ABI this phase is about. Both slice flavours,
/// since a bundle carries `!Slice[T]` too.
#[test]
fn g_poly_bundle_instantiated_slice_output_builds_and_runs() {
    for (tag, borrow) in [("gpb-shared", "&a"), ("gpb-mut", "&!a")] {
        let out = build_and_run(
            tag,
            &format!(
                "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: pair ( 'T -- i64 'T )
  0 swap
;

: main ( -- )
  0 3 fill | a |
  {borrow} slice pair | x r |
  x .
  r len >i64 .
  a drop
;
"
            ),
        );
        assert_eq!(out, "0\n3", "{tag}");
    }
}

/// G-poly-bundle, the splice-record half: the same call made from inside a
/// combinator body, whose inner `CallInst` is keyed by `(inline_uid, span)` in
/// `splice_records` rather than by span in `instantiations`, and whose bundle
/// is interned on that record. Three sites intern it, redundantly: `check.rs`'s
/// own loop (runs unconditionally, before `discover_transitive_instantiations`
/// is even called), `poly.rs`'s early-return branch (dead for this fixture,
/// since `outer` calling `pair` makes `poly_cross_calls` non-empty, so the
/// fixpoint runs instead), and `poly.rs`'s post-fixpoint pass (which does run
/// here, over every `splice_records` entry unconditionally). Verified
/// load-bearing by stubbing all three at once: only then does this golden go
/// red. Stubbing `check.rs`'s loop alone, or the post-fixpoint pass alone,
/// each leave it green, since the other still covers this fixture's record.
#[test]
fn g_poly_bundle_splice_record_slice_output_builds_and_runs() {
    let out = build_and_run(
        "gpbs",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: pair ( 'T -- i64 'T )
  0 swap
;

: outer inline ( 'T -- i64 'T )
  pair
;

: main ( -- )
  0 3 fill | a |
  &a slice outer | x r |
  x .
  r len >i64 .
  a drop
;
",
    );
    assert_eq!(out, "0\n3");
}

/// G-poly-bundle, the aggregate half: a slice-bearing *struct* travelling in
/// the bundle rather than a bare view. The projection afterwards proves the
/// field survived the pack/return/unpack round trip intact.
#[test]
fn g_poly_bundle_slice_bearing_aggregate_output_builds_and_runs() {
    let out = build_and_run(
        "gpba",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

type: Window view Slice[i64] lo usize ;

: pair ( 'T -- i64 'T )
  0 swap
;

: main ( -- )
  0 3 fill | a |
  &a slice 0 >usize Window pair | x w |
  x .
  &w &view @ len >i64 .
  a drop
;
",
    );
    assert_eq!(out, "0\n3");
}

/// The bundle's ABI pin, G6's synthesized-aggregate twin: the bundle spells
/// its slice member as `:sooth.slice` in member position (not a raw one-word
/// view), the shared aggregate is declared before the bundle that references
/// it, and the bundle is returned by value across a real call. The only
/// `:sooth.slice` in ABI (parameter) position here is pre-existing
/// `qbe_abi_ty` behavior, asserted below purely as context; the member
/// position and the return are what this golden is actually pinning.
#[test]
fn g_poly_bundle_spells_the_slice_member_and_returns_by_value() {
    let (_t, entry) = fixture(
        "gpbabi",
        "\
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;

: pair ( 'T -- 'T i64 )
  0
;

: main ( -- )
  0 3 fill | a |
  &a slice pair | r x |
  x .
  r len >i64 .
  a drop
;
",
    );
    let ssa = sooth::driver::emit_ssa(&entry)
        .unwrap_or_else(|e| panic!("emitting the fixture should succeed: {e}"));

    let slice_at = ssa
        .find("type :sooth.slice = { l, l }")
        .expect("the shared slice aggregate should be declared");
    let bundle_decl = ssa
        .match_indices("= { :sooth.slice, l }")
        .next()
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("the bundle should spell its slice member: {ssa}"));
    assert!(
        slice_at < bundle_decl,
        "QBE needs the member type declared first: {ssa}"
    );

    let sig = ssa
        .lines()
        .find(|l| l.starts_with("export function") && l.contains("$sooth_mono_pair"))
        .unwrap_or_else(|| panic!("one instantiation at `Slice[i64]`: {ssa}"));
    assert!(
        sig.contains("(:sooth.slice %"),
        "pre-existing qbe_abi_ty behavior, asserted only as context -- the \
         parameter is passed as the two-word aggregate: {sig}"
    );
    assert!(
        sig.contains("export function :__ret_"),
        "the instantiation returns its bundle by value: {sig}"
    );
    assert!(
        !ssa.lines()
            .any(|l| l.contains("sooth_struct_drop") && l.contains(":__ret_")),
        "a bundle owes no destructor over the slice slot (guaranteed by the \
         !bundle filter in destructors.rs; the slot-level claim itself is \
         pinned by layout.rs's !b.is_linear assertion): {ssa}"
    );
}
