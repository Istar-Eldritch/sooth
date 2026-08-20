//! Phase 7 Slice 3c goldens: a borrowed, length-carrying view over a buffer.
//!
//! Phase 3 ships the **shared** half: `slice` from a `&[T N]`, `subslice`,
//! `len` answering a runtime length, and the shared `&>` receiver bounds-
//! checked against that length. Mutable views, their exclusivity tracking, and
//! the reborrow-chain test land in Phase 4.

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

/// The same trap guards `subslice`'s range: a sub-view may not reach past the
/// end of the view it is cut from. `3 + 3` over a length-4 buffer traps rather
/// than minting a view onto memory the buffer does not own.
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
        "sooth: array index out of range (line 3)\n  index 6 is out of bounds for length 4\n"
    );
    assert_eq!(code, 1);
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
