//! Phase 7 Slice 3c goldens: a borrowed, length-carrying view over a buffer.
//!
//! Phase 3 ships the **shared** half: `slice` from a `&[T N]`, `subslice`,
//! `len` answering a runtime length, and the shared `&>` receiver bounds-
//! checked against that length. Phase 4 adds the mutable half -- `slice` off a
//! `&![T N]`, mutable `subslice`, the `&!>` receiver -- together with the
//! exclusivity rules that make it safe and the reborrow-chain test (R13).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3a.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3c-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, contents).unwrap();
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

/// Build and run, returning `(stdout, stderr, exit code)`. A build failure
/// panics with the diagnostic: every program here is well-typed, so a build
/// error is itself the regression.
fn build_and_run(src: &Path) -> (String, String, i32) {
    let binary = driver::build(src).unwrap_or_else(|e| panic!("program should build: {e}"));
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        String::from_utf8(output.stderr).expect("stderr should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn import_times() -> String {
    format!(
        "import: \"{}/lib/combinators.sth\" c | times | ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The exit criterion. `sum` is a plain (non-`inline`) word taking
/// `Slice[i64]`: it names no length variable, threads no length parameter, and
/// indexes against the view's runtime length. It is diffed against
/// `sum_len`, the length-threading twin that is the only shape writable
/// without slices -- both must print `25`, so the slice signature is proven
/// equivalent, not merely accepted.
#[test]
fn sum_over_a_slice_noninline_prints_twentyfive() {
    let src = format!(
        "{}\
         : sum ( Slice[i64] -- i64 )\n  \
           | s |\n  \
           0 s len >i64 ~[ | i | s i >usize &> @ add ] times\n\
         ;\n\
         : sum_len ( &[i64 5] usize -- i64 )\n  \
           | n | | a |\n  \
           0 n >i64 ~[ | i | a i >usize &> @ add ] times\n\
         ;\n\
         : main ( -- )\n  \
           5 5 fill | buf |\n  \
           &buf slice sum .\n  \
           &buf 5 >usize sum_len .\n  \
           buf drop\n\
         ;\n",
        import_times()
    );
    let prog = Scratch::write("sum", &src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "25\n25\n");
    assert_eq!(code, 0);
}

/// R10.3: `subslice` re-derives a fresh view rather than nesting a borrow,
/// which is the only reading under which a recursive consumer can take
/// `s 0 mid subslice` and hand it to itself. Divide-and-conquer over the two
/// halves, all shared; the mutable-half twin lands in Phase 4.
#[test]
fn recursive_divide_and_conquer_over_shared_subslices_runs() {
    let src = "\
: rec ( Slice[i64] -- i64 )
  | s |
  s len | n |
  n 0 >usize eq ~[
    0
  ] ~[
    n 1 >usize eq ~[
      s 0 >usize &> @
    ] ~[
      n >i64 1 shr >usize | mid |
      s 0 >usize mid subslice rec
      s mid n mid sub subslice rec
      add
    ] if
  ] if
;

: main ( -- )
  3 7 fill | buf |
  &buf slice rec .
  buf drop
;
";
    let prog = Scratch::write("rec", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "21\n");
    assert_eq!(code, 0);
}

/// R9.2/R14: an out-of-range index traps at runtime with the same located
/// message array indexing produces -- there is no fallible `Option`/`Result`
/// accessor, and this slice does not add one. The length in the message is the
/// *carried* one (4), which a compile-time count could not have supplied.
#[test]
fn slice_out_of_range_index_traps_at_runtime() {
    let src = "\
: main ( -- )
  7 4 fill | buf |
  &buf slice | s |
  s 9 >usize &> @ .
  buf drop
;
";
    let prog = Scratch::write("oob", src);
    let (stdout, stderr, code) = build_and_run(prog.path());
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "sooth: array index out of range (line 4)\n  index 9 is out of bounds for length 4\n"
    );
    assert_eq!(code, 1);
}

/// `subslice` gets its own trap, not the index one: a sub-view may not reach
/// past the end of the view it is cut from, and the failure has no index to
/// report -- the message names the requested start and length against the
/// view's length. `3 + 3` over a length-4 buffer traps rather than minting a
/// view onto memory the buffer does not own.
#[test]
fn subslice_past_the_end_traps_at_runtime() {
    let src = "\
: main ( -- )
  7 4 fill | buf |
  &buf slice 3 >usize 3 >usize subslice | s |
  s len .
  buf drop
;
";
    let prog = Scratch::write("subrange", src);
    let (stdout, stderr, code) = build_and_run(prog.path());
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "sooth: subslice out of range (line 3)\n  start 3 length 3 exceeds view length 4\n"
    );
    assert_eq!(code, 1);
}

/// The range check must not compute `start + len` (that addition can wrap
/// past `usize::MAX`, e.g. from `0 1 sub` underflowing to `usize::MAX`, and
/// pass a naive `end <= recv_len` check): a wrapped `start` would mint a view
/// whose base pointer sits *before* the buffer it was cut from. The reported
/// start is the unwrapped `usize::MAX`, which is what makes the message
/// evidence that no addition happened.
#[test]
fn subslice_start_plus_len_overflow_traps_instead_of_wrapping() {
    let src = "\
: main ( -- )\n  7 4 fill | buf |\n  &buf slice 0 1 sub >usize 5 >usize subslice | s |\n  s len .\n  buf drop\n;\n";
    let prog = Scratch::write("wrap", src);
    let (stdout, stderr, code) = build_and_run(prog.path());
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "sooth: subslice out of range (line 3)\n  \
         start 18446744073709551615 length 5 exceeds view length 4\n"
    );
    assert_eq!(code, 1);
}

/// R4: a **shared** view is `Copy`, so `dup` on one is accepted and both
/// copies stay usable -- the second `len` reads the same carried length as the
/// first. Phase 4 owes the `dup_on_mutable_slice_is_error` twin: a mutable
/// view cannot be written in this build at all.
#[test]
fn dup_on_shared_slice_ok() {
    let src = "\
: main ( -- )
  7 4 fill | buf |
  &buf slice | s |
  s dup len . len .
  buf drop
;
";
    let prog = Scratch::write("dup", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "4\n4\n");
    assert_eq!(code, 0);
}

/// R5/R12: the declaration-level output ban covers a slice exactly as it
/// covers a `&T`. Writable as source only now that a signature can spell
/// `Slice[T]` at all; Phase 1 proved the same claim by a direct call.
#[test]
fn declared_slice_output_is_stored_reference_error() {
    let prog = Scratch::write("output", ": bad ( -- Slice[i64] ) ;\n: main ( -- ) ;\n");
    let err = driver::build(prog.path()).unwrap_err();
    assert!(
        err.contains("a reference cannot be stored")
            && err.contains("declares the output `Slice[i64]`"),
        "unexpected message: {err}"
    );
}

/// The brief's locked quotation-parameter-input-row decision: a slice is a
/// legal *input* inside a declared `~[ ... ]` parameter row, distinct from the
/// `sum` golden, which captures a slice into a quotation *literal*. The word
/// must be `inline`: a `~[ ]` parameter can only ever be spliced, so a
/// non-`inline` spelling is refused before the row's contents matter at all.
#[test]
fn slice_through_a_declared_quotation_parameter_row_runs() {
    let src = "\
: apply inline ( Slice[i64] ~[ Slice[i64] -- ] -- )
  | f | | s | s f call
;
: show ( Slice[i64] -- ) | s | s 0 >usize &> @ . ;
: main ( -- )
  9 2 fill | buf |
  &buf slice ~[ show ] apply
  buf drop
;
";
    let prog = Scratch::write("quotrow", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);

    let non_inline = Scratch::write(
        "quotrow-noninline",
        ": apply ( Slice[i64] ~[ Slice[i64] -- ] -- ) | f | | s | s f call ;\n: main ( -- ) ;\n",
    );
    let err = driver::build(non_inline.path()).unwrap_err();
    assert!(
        err.contains(
            "declares an inline-quotation parameter `~[ Slice[i64] -- ]` but is not `inline`"
        ),
        "unexpected message: {err}"
    );
}

/// R1.1: `Slice` is intercepted by name ahead of every user type registry, so
/// it must be unclaimable as a declared name -- otherwise a `type: Slice ...`
/// would be silently unreachable rather than merely shadowed.
#[test]
fn declaring_a_type_named_slice_is_rejected() {
    let prog = Scratch::write("reserved", "type: Slice a i64 ;\n: main ( -- ) ;\n");
    let err = driver::build(prog.path()).unwrap_err();
    assert!(
        err.contains("`Slice` is reserved for the slice type syntax (`Slice[T]`)"),
        "unexpected message: {err}"
    );
}

// --- Phase 4: mutable views, their borrow rules, and the reborrow chain. ---

/// R10.3/R12, the mutable twin of the shared divide-and-conquer golden: `dbl`
/// takes one mutable half at a time, so the coarse borrow table never sees two
/// live `&!` views of the same buffer, and the mutation each leaf performs is
/// visible through the buffer afterwards. The view is built one frame up, in
/// `dbl_all`, off a declared `&![i64 5]` *parameter*: the mutability of a
/// reference that arrives as a parameter comes from its declared type, not
/// from the borrow that produced it.
#[test]
fn recursive_divide_and_conquer_over_mutable_subslices_runs() {
    let src = "\
: dbl ( !Slice[i64] -- )
  | s |
  s len | n |
  n 0 >usize eq ~[
  ] ~[
    n 1 >usize eq ~[
      s 0 >usize &!> | e | e @ | v | e v v add !
    ] ~[
      n >i64 1 shr >usize | mid |
      s 0 >usize mid subslice dbl
      s mid n mid sub subslice dbl
    ] if
  ] if
;

: dbl_all ( &![i64 5] -- ) slice dbl ;

: main ( -- )
  3 5 fill | buf |
  &!buf dbl_all
  &buf 0 >usize &> @ .
  &buf 4 >usize &> @ .
  buf drop
;
";
    let prog = Scratch::write("recmut", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "6\n6\n");
    assert_eq!(code, 0);
}

/// R4: a **mutable** view is not `Copy`, so `dup` on one is the exclusivity
/// error a `&!T` gets, word for word. The shared twin (`dup_on_shared_slice_ok`
/// above) is what keeps the arm from simply answering "never `Copy`".
#[test]
fn dup_on_mutable_slice_is_error() {
    let prog = Scratch::write(
        "dupmut",
        ": main ( -- )\n  7 4 fill | buf |\n  &!buf slice | s |\n  s dup len . len .\n  buf drop\n;\n",
    );
    let err = driver::build(prog.path()).unwrap_err();
    assert_eq!(
        err,
        "error: cannot `dup` a value of type `!Slice[i64]` in `main` (line 4)\n  \
         `!Slice[i64]` is exclusive: at most one may be live for a place, so copying it \
         would make a second one; use it where it is, or borrow again once it is consumed\n  \
         note: declared ( -- )"
    );
}

/// R12: two mutable sub-views of one buffer, both live at once, are rejected
/// by the coarse borrow table -- range-awareness (they are disjoint here) is
/// deliberately out of scope. The rejection lands on the *second* derivation:
/// naming `s` again reborrows a place the first sub-view still holds.
#[test]
fn two_simultaneous_mutable_subslices_is_error() {
    let src = "\
: main ( -- )
  0 4 fill | buf |
  &!buf slice | s |
  s 0 >usize 2 >usize subslice | a |
  s 2 >usize 2 >usize subslice | b |
  a len . b len .
  buf drop
;
";
    let prog = Scratch::write("twomut", src);
    let err = driver::build(prog.path()).unwrap_err();
    assert!(
        err.contains("cannot reborrow `s` in `main` while a reference derived from it is live")
            && err.contains("a mutable borrow suspends its place"),
        "unexpected message: {err}"
    );
    // The shared twin is legal: two live shared views of one buffer conflict
    // with nothing, so the rejection above is about mutability, not about
    // sub-viewing twice.
    let shared = src.replace("&!buf", "&buf");
    let prog = Scratch::write("twoshared", &shared);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "2\n2\n");
    assert_eq!(code, 0);
}

/// R12: a shared view cannot coexist with a mutable one over the same buffer.
/// The view carries its receiver's region, so the second borrow of `buf` is
/// what reports -- the same diagnostic two overlapping `&`/`&!` borrows get.
#[test]
fn a_shared_view_alongside_a_mutable_view_is_error() {
    let src = "\
: main ( -- )
  0 4 fill | buf |
  &!buf slice | m |
  &buf slice | s |
  m len . s len .
  buf drop
;
";
    let prog = Scratch::write("mixed", src);
    let err = driver::build(prog.path()).unwrap_err();
    assert!(
        err.contains("`&buf` conflicts with a live borrow of `buf`")
            && err.contains("never a `&` alongside a `&!`"),
        "unexpected message: {err}"
    );
}

/// R13 (OQ3): the reborrow chain the brief's probe never covered --
/// `&!buffer -> &!Slice -> &!sub-slice -> &!element` -- mutating through the
/// **innermost** hop while both outer hops are still live (each is read after
/// the write). The middle hops are references stored *inside an aggregate*
/// (a `{ptr, len}` view), which is the shape no earlier chain had.
#[test]
fn mutate_innermost_hop_of_buffer_slice_subslice_element_chain_while_outer_live() {
    let src = "\
: main ( -- )
  0 4 fill | buf |
  &!buf slice | s |
  s 1 >usize 2 >usize subslice | half |
  half 1 >usize &!> | e |
  e 9 !
  half 0 >usize &!> @ .
  s len .
  &buf 2 >usize &> @ .
  buf drop
;
";
    let prog = Scratch::write("chain", src);
    let (stdout, _, code) = build_and_run(prog.path());
    // The sub-view starts at buffer index 1, so its element 1 is buffer
    // element 2: the write lands there and nowhere else, and both outer hops
    // still answer for themselves afterwards.
    assert_eq!(stdout, "0\n4\n9\n");
    assert_eq!(code, 0);
}

/// R11: the concrete-element poly consumer. A *generic* word indexes and
/// sub-ranges a `Slice[i64]` -- the shape Phase 3 could only reject -- and
/// stores through a `!Slice[i64]`, so the poly walk's slice arms are exercised
/// end to end rather than only at the checker.
///
/// `set_head` writes with a single derivation on purpose: in a generic body a
/// non-`Copy` local is move-tracked per binding, so a mutable view (and the
/// `&!` element reference cut from it) is single-use there, where the
/// monomorphic path reborrows a named local. A read-modify-write through a
/// mutable view therefore needs the concrete path; see the phase's exit notes.
#[test]
fn poly_body_indexes_subslices_and_mutates_a_slice() {
    let src = "\
: head_of ( Slice[i64] 'T -- i64 'T )
  | mark |
  1 >usize 2 >usize subslice
  0 >usize &> @
  mark
;
: set_head ( !Slice[i64] i64 'T -- 'T )
  | mark | | v |
  0 >usize &!> v !
  mark
;
: main ( -- )
  4 3 fill | buf |
  &buf slice 0 head_of drop .
  &!buf slice 5 0 set_head drop
  &buf 0 >usize &> @ .
  buf drop
;
";
    let prog = Scratch::write("polyslice", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "4\n5\n");
    assert_eq!(code, 0);
}

/// R1.1 (phase 4): the mutable spelling is reserved for the same reason the
/// shared one is -- `!Slice[T]` is intercepted ahead of every user registry,
/// so a declaration under that name would be unreachable, not shadowed.
#[test]
fn declaring_a_type_named_mutable_slice_is_rejected() {
    let prog = Scratch::write("reserved-mut", "type: !Slice a i64 ;\n: main ( -- ) ;\n");
    let err = driver::build(prog.path()).unwrap_err();
    assert!(
        err.contains("`!Slice` is reserved for the slice type syntax"),
        "unexpected message: {err}"
    );
}

/// R12/R2.2: a mutable buffer reference captured into a *materialized*
/// quotation, then sliced and written through inside it. The capture crosses
/// an env boundary, where the reference is a bare pointer and its mutability
/// travels beside it -- the one path where the view's mutability is not
/// readable off the value at all.
#[test]
fn a_materialized_quotation_slices_a_captured_mutable_reference() {
    let src = "\
: apply ( [ -- ] -- ) call ;
: main ( -- )
  0 4 fill | buf |
  &!buf | r |
  [ r slice 0 >usize &!> 5 ! ] apply
  &buf 0 >usize &> @ .
  buf drop
;
";
    let prog = Scratch::write("capture", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

/// R12: a mutable buffer reference that reaches `slice` through a **branch
/// join** -- the arms project two different fields of one owner, so the joined
/// value really is a `Phi` of two distinct pointers (two arms yielding the
/// *same* value are joined without one, and would not exercise this). A reference lowers to an opaque pointer, so the join is
/// the second place (with the env capture above) where the view's mutability
/// has to be carried alongside the value rather than read off it.
#[test]
fn a_view_built_from_a_joined_mutable_reference_writes_through() {
    let src = "\
type: W a [i64 4] b [i64 4] ;
: main ( -- )
  0 4 fill 0 4 fill W | w |
  1 1 eq ~[ &!w &!a ] ~[ &!w &!b ] if slice | s |
  s 0 >usize &!> 9 !
  &w &a 0 >usize &> @ .
  w drop
;
";
    let prog = Scratch::write("joined", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}

/// R12: two more routes a reference reaches `slice` by, beyond the four the
/// exit notes name -- `&>`'s array-element projection (a nested array, the
/// only shape where that projection can feed `slice` at all) and `&^`'s
/// owned-cell payload projection. Both derive a fresh reference whose
/// mutability is the sigil's own, same as a prefix borrow, but through a
/// different `push_reference` call site; each half writes through its view
/// and reads back through the original owner to prove the view really
/// aliases it.
#[test]
fn a_view_built_from_a_nested_array_element_or_owned_cell_payload_writes_through() {
    let src = "\
: main ( -- )
  0 2 fill 2 fill | n |
  &!n 0 >usize &!> slice 0 >usize &!> 7 !
  &!n 0 >usize &!> slice 0 >usize &!> @ .
;
";
    let prog = Scratch::write("nestedarray", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);

    let src = "\
type: Buf data ^[i64 4] ;
: main ( -- )
  0 4 fill ^ Buf | b |
  &!b &!data &!^ slice 0 >usize &!> 9 !
  &!b &!data &!^ slice 0 >usize &!> @ .
  b drop
;
";
    let prog = Scratch::write("ownedcell", src);
    let (stdout, _, code) = build_and_run(prog.path());
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}
