//! P7b.S12 exit goldens — poly-body App-dispatch output rendering (SOO-39).
//!
//! Phase 1 (`6c75be0`) gave `render_member_decl` its `PolyType::Generic` arm
//! and `substitute_member_var` its mirror arm, with R3's two-tier rule for
//! unbound member variables (verbatim when the admitting structural equality
//! on a ctor-headed input pinned the id; the dispatched header variable when
//! never pinned). These goldens pin the post-fix behaviour end to end:
//!
//! - G1–G5: the S12 probe shapes (`probes/s12_*.sth`, byte-frozen for their
//!   pre-fix baseline in `probes/s12_baseline.md`) as embedded strings —
//!   the leak's three faces (out-of-range id, in-range mis-typing, id
//!   coincidence) now render in caller space.
//! - G6: probe H's List-backed Cursor drain end to end (the phase's exit
//!   criterion), in the house `mkempty` spelling.
//! - G7: a member-sig diagnostic renders `Option['T]`-shaped declared inputs
//!   in caller space (`no_candidate_fits_operands_error`'s shape strings, the
//!   `substitute_member_var` callers).
//! - G8: the cross-call App fence byte-identical to its frozen baseline
//!   (R4 — the fence does not move).
//! - G11: the structurally-pinned subclass (R3 tier 1) stays compiling with
//!   the pinned payload flowing through typed.
//!
//! G9 (`iterator_consumers_survive_the_render_fix`, R6 canary) is asserted by
//! the existing `tests/phase7b_slice8.rs` goldens staying green — not
//! duplicated here; the canaries are
//! `for_each_drains_a_list_through_the_iterator_bound`,
//! `fold_sums_a_list_through_the_iterator_bound`, and
//! `for_each_and_fold_drain_a_range_through_the_iterator_bound`.
//! G10 (`existing_suite_green`) is the full `cargo test` run.
//!
//! Harness style from `tests/phase7b_slice8.rs`. Golden sources are embedded
//! strings, not reads of `probes/` (those files stay byte-frozen as the
//! pre-fix baseline); the harness prelude's `import: intrinsics * ;` replaces
//! the probes' own import line, so a golden that pins stderr line numbers
//! pins the harness-layout value, and G8 embeds its probe byte-verbatim
//! (own import, no manifest) to stay byte-identical to the frozen baseline.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs12-{}-{tag}-{seq}", std::process::id()));
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

/// `tests/phase7b_slice8.rs`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs12 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

fn build_run_keep(tag: &str, src: &str) -> (Tree, PathBuf, String) {
    let (t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the binary should run");
    assert_eq!(
        run.status.code(),
        Some(0),
        "the built binary should exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    (t, binary, stdout)
}

/// Build `src` and assert it succeeds, discarding the binary and its output.
fn build_ok(tag: &str, src: &str) {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
}

/// Build `src`, assert it fails *located* (a clean non-zero exit with a
/// `error: ...` stderr, never a panic), and return that stderr.
fn build_error_located(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    let stderr = String::from_utf8(build.stderr).expect("stderr should be utf8");
    assert!(
        !stderr.contains("panicked"),
        "a check rejection must never panic, got: {stderr}"
    );
    stderr
}

/// Build `src` the way the frozen baseline was captured — the source
/// byte-verbatim (its own `import: intrinsics * ;` line included) in a bare
/// temp dir with no `sooth.pkg`, so line numbers match
/// `probes/s12_baseline.md`'s capture exactly. Used only by G8's
/// byte-identity pin (R4): every other golden rides the harness prelude.
fn build_error_bare(tag: &str, src: &str) -> String {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    let stderr = String::from_utf8(build.stderr).expect("stderr should be utf8");
    assert!(
        !stderr.contains("panicked"),
        "a check rejection must never panic, got: {stderr}"
    );
    stderr
}

/// (G1) The S6d-4-faithful drain fold (`probes/s12_a_drain_fold_option_row.sth`'s
/// body): a `['It: Cursor]` consumer whose `next` row returns the remainder
/// as a bare App-headed value. Pre-fix the member row's element variable
/// landed in the caller's id space unrendered (`'?1`), every downstream
/// concrete-shaped operation choked, and `add` underflowed ("holds 1").
/// Post-fix the Generic arm renders the member row's outputs through the
/// bindings — `Option[i64]` and `'It[i64]` in caller space — so the body
/// checks clean. The check-level pass is the fix's verdict; the bare
/// `: main ( -- ) ;` exists only for the link step.
#[test]
fn bound_generic_drain_fold_option_row_checks_clean() {
    build_ok(
        "g1-drain-fold-option-row",
        "\
type: Option['T]
| None
| Some 'T
;

trait: Cursor['It: * -> *]
  : next ( 'It['T] -- Option['T] 'It['T] ) ;
;

: drain ['It: Cursor] ( i64 'It[i64] -- i64 )
  next swap
  ~[ ( None ) drop drop ]
  ~[ ( Some ) Some> rot add swap drain ]
  Option? ;
: main ( -- ) ;
",
    );
}

/// (G2) Probe A2 (`probes/s12_a2_add_holds_zero.sth`): the add directly on
/// the Some payload. Pre-fix the leaked member-space id made the operator
/// path extract an empty concrete suffix — "holds 0", the S6d-4 message
/// (kept byte-frozen in `probes/s12_baseline.md` as that round's provenance
/// anchor). Post-fix the check error is *honest*: the add genuinely consumes
/// the remainder (not a number), so the message names the real operand
/// underflow ("holds 1", patch-validated). This golden asserts the shift
/// without pinning the new bytes: located, the underflow family naming
/// `add`, no `'?` id anywhere, no panic, and the pre-fix "holds 0" gone.
#[test]
fn add_directly_on_some_payload_reports_holds_zero_no_more() {
    let stderr = build_error_located(
        "g2-add-holds-zero",
        "\
type: Option['T]
| None
| Some 'T
;

trait: Cursor['It: * -> *]
  : next ( 'It['T] -- Option['T] 'It['T] ) ;
;

: drain ['It: Cursor] ( i64 'It[i64] -- i64 )
  next swap
  ~[ ( None ) drop drop ]
  ~[ ( Some ) Some> add drain ]
  Option? ;
",
    );
    assert!(
        stderr.contains("stack effect mismatch in `drain`"),
        "{stderr}"
    );
    assert!(stderr.contains("`add` needs 2 values"), "{stderr}");
    assert!(
        stderr.contains("holds 1"),
        "patch-validated: the underflow now names the real operands: {stderr}"
    );
    assert!(
        !stderr.contains("holds 0"),
        "the S6d-4 empty-suffix message must be gone: {stderr}"
    );
    assert!(
        !stderr.contains("'?"),
        "no member-space id may leak into the diagnostic: {stderr}"
    );
}

/// (G3) Probe B (`probes/s12_b_member_output_residual.sth`): what the member
/// dispatch actually leaves. The fixture's declared outputs deliberately
/// omit the remainder, so the residual mismatch is the golden — and its
/// point is the rendering: pre-fix the residual named the leaked member id
/// (`Option['?1]`, the frozen baseline); post-fix the render_member_decl
/// Generic arm puts the whole leaves-row in caller space. Byte-pinned from
/// the live binary (the message carries no line number, so the harness
/// prelude cannot shift it). The build_ok twin of this shape is G5.
#[test]
fn member_output_residual_renders_caller_space_option() {
    let stderr = build_error_located(
        "g3-member-output-residual",
        "\
type: Option['T]
| None
| Some 'T
;

trait: Cursor['It: * -> *]
  : next ( 'It['T] -- Option['T] 'It['T] ) ;
;

: drainB ['It: Cursor] ( i64 'It[i64] -- i64 Option[i64] )
  next swap ;
",
    );
    assert_eq!(
        stderr,
        "error: stack effect mismatch in `drainB`\n  body leaves `i64 'It[i64] Option[i64]`, but the declared outputs are `i64 Option[i64]`\n"
    );
    assert!(
        !stderr.contains("'?"),
        "no member-space id may leak into the diagnostic: {stderr}"
    );
}

/// (G4) Probe C (`probes/s12_c_arm_payload_mistype.sth`): the face-2
/// silent mis-typing. `'E` is var 0, `'It` var 1; pre-fix the member row's
/// element variable leaked unrendered, so the Some payload read as `'It`
/// (the `* -> *` constructor variable) and the arms disagreed (`'E` vs
/// `'It`). Post-fix the Generic arm renders the payload through the
/// bindings — it types as `'E`, the arms agree, and the body checks clean.
/// Same link-step shape as G1: the bare `main` is for the linker.
#[test]
fn some_payload_types_as_the_element_variable_not_the_constructor() {
    build_ok(
        "g4-arm-payload-mistype",
        "\
type: Option['T]
| None
| Some 'T
;

trait: Cursor['It: * -> *]
  : next ( 'It['T] -- Option['T] 'It['T] ) ;
;

: drainC ['E 'It: Cursor] ( 'E 'It['E] -- 'E )
  next swap
  ~[ ( None ) drop drop ]
  ~[ ( Some ) Some> |v| drop drop v ]
  Option? ;
: main ( -- ) ;
",
    );
}

/// (G5) Probe F (`probes/s12_f_id_coincidence_space.sth`): the id-coincidence
/// mask, unmasked. Variable ids follow effect-mention order (`'T`=0, `'It`=1
/// here), so the member row's element variable (member id 1) leaked verbatim
/// into the caller's id space *in range* — `Option['It]` where `Option['T]`
/// was declared, a silent mis-typing that only looked right in consumers
/// whose element variable also sat at id 1 (the mechanism
/// `lib/core/iterator.sth`'s `fold` survived on, R6). Post-fix the Generic
/// arm renders through the bindings map, which holds the caller's own
/// answer. Checks clean; bare `main` for the link step, as in G1/G4.
#[test]
fn leaked_member_id_renders_through_bindings_in_range() {
    build_ok(
        "g5-id-coincidence-space",
        "\
type: Option['T]
| None
| Some 'T
;

trait: Cursor['It: * -> *]
  : next ( 'It['T] -- Option['T] 'It['T] ) ;
;

: drainF ['It: Cursor 'T] ( 'T 'It['T] -- 'T 'It['T] Option['T] )
  next swap ;
: main ( -- ) ;
",
    );
}

/// (G6, the phase's exit criterion) Probe H
/// (`probes/s12_h_end_to_end_list_drain.sth`, byte-frozen for its pre-fix
/// baseline) end to end: a List-backed `Cursor` impl (the impl member
/// checks on the mono route pre- and post-fix) and a bound-generic `drain`
/// consumer that pre-fix died at `add` with the leak ("holds 1"). Golden
/// copy in the house spelling: `: mkempty ( -- List[i64] ) Nil ;` added and
/// `main`'s bare `Nil` rewritten to `mkempty` + `^ Cons` (the
/// `tests/phase7b_slice8.rs` List-consumer pattern). No
/// `import: hosted::show` line here — the harness prelude prepends it and a
/// duplicate collides in the seen-map. Builds, runs, prints the fold's sum.
#[test]
fn end_to_end_list_backed_cursor_drain_prints_6() {
    let src = "\
import: core::list | List Nil Cons | ;

type: Option['T]
| None
| Some 'T
;

trait: Cursor['It: * -> *]
  : next ( 'It['T] -- Option['T] 'It['T] ) ;
;

impl: Cursor for List
  : next
    ~[ ( Nil ) drop None Nil ]
    ~[ ( Cons ) Cons> |v rest| rest ^> v Some swap ]
    List? ;
;

: drain ['It: Cursor] ( i64 'It[i64] -- i64 )
  next swap
  ~[ ( None ) drop drop ]
  ~[ ( Some ) Some> rot add swap drain ]
  Option? ;

: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  1 2 3 mkempty ^ Cons ^ Cons ^ Cons
  0 swap drain
  . ;
";
    let (_t, _binary, stdout) = build_run_keep("g6-end-to-end-list-drain", src);
    assert_eq!(stdout, "6\n");
}

/// (G7) The diagnostics twin: a member sig with a ctor-headed input renders
/// in caller space in the *shape* strings. The fixture makes two traits
/// share the member name `f` on two different bound variables (legal to
/// declare, R12) and dispatches `f` with operands matching neither declared
/// shape — the slot-1 `i64` against `Option['U]` / `Box[i64]` — so
/// `no_candidate_fits_operands_error` fires for an operand-shape reason,
/// not the leak (this error family fires identically pre-fix; only its
/// rendering was garbled: member `'U` is id 2, out of the caller's two-var
/// table, so pre-fix the shape read `Option['?2]`). Post-fix
/// `substitute_member_var`'s Generic arm recurses the total rewrite into
/// the args: `Option['V]` in the dispatched variable's own name, and the
/// concrete `Box[i64]` arg passes through. Byte-pinned from the live binary
/// at the harness layout (the two prelude lines put the call on line 18).
#[test]
fn member_sig_diagnostics_render_generic_args_in_caller_space() {
    let stderr = build_error_located(
        "g7-member-sig-caller-space",
        "\
type: Option['T]
| None
| Some 'T
;

type: Box['T] v 'T ;

trait: TA['F: * -> *]
  : f ( 'F['T] Option['U] -- ) ;
;

trait: TB['F: * -> *]
  : f ( 'F['T] Box[i64] -- ) ;
;

: caller ['V: TA 'W: TB] ( 'V['W] i64 -- ) f ;
",
    );
    assert_eq!(
        stderr,
        "error: `f` is required by `TA` on 'V and `TB` on 'W in `caller` (line 18, col 44)\n  note: the operands at this call match none of their declared shapes: `TA` on 'V expects `'V['V] Option['V]` and `TB` on 'W expects `'W['W] Box[i64]`\n"
    );
    assert!(
        !stderr.contains("'?"),
        "no member-space id may leak into the diagnostic: {stderr}"
    );
}

/// (G8, R4 pin) Probe E (`probes/s12_e_cross_call_app_fence.sth`): the
/// cross-call App fence (S1-17.i) stays byte-identical. The slice fixes the
/// member-output *render*; the poly-body cross-call fence over compound
/// receivers is a follow-up slice (SOO-60) and does not move. Built the way
/// the baseline was captured — probe source byte-verbatim (its own
/// `import: intrinsics * ;`, keeping `step ;` on line 13), bare temp dir, no
/// manifest — so the stderr equals the frozen
/// `probes/s12_baseline.md` entry byte for byte.
#[test]
fn cross_call_app_fence_stays_byte_identical() {
    let stderr = build_error_bare(
        "g8-cross-call-app-fence",
        "\\ S12 probe E — control: the cross-call App fence (S1-17.i).\n\
         \\ Expected today: located rejection, not a stack loss.\n\
         import: intrinsics * ;\n\
         \n\
         trait: Cursor['It: * -> *]\n\
         \x20 : next ( 'It['T] -- 'It['T] ) ;\n\
         ;\n\
         \n\
         : step ['It: Cursor] ( i64 'It[i64] -- i64 )\n\
         \x20 drop ;\n\
         \n\
         : outer ['It: Cursor] ( i64 'It[i64] -- i64 )\n\
         \x20 step ;\n",
    );
    assert_eq!(
        stderr,
        "error: `outer` cannot call the polymorphic word `step` (line 13, col 3)\n  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body\n  call `step` from a monomorphic word instead\n"
    );
}

/// (G11, R3 tier-1 pin) The structurally-pinned subclass. `unify_member_operand`
/// has no `Generic` arm, so `accept`'s ctor-headed input `Option['U]`
/// dispatches at the poly-body route only by the tail's structural equality
/// — `'U` (member id 2) is pinned by the admitting equality, never bound.
/// The program compiles today: at `main`'s mono call site the CtorImage
/// mint arm re-grounds the obligation's slots and per-site unifies the impl
/// member's grounded sig via `unify_poly_input`'s Generic arm. The golden
/// pins that the fix does NOT re-type the pinned payload — the unscoped
/// `Var(var)` fallback would re-type it as the header variable (a
/// `* -> *` variable in an element position) and break exactly this
/// program with a body-level residual mismatch. Caller ids follow
/// effect-mention order (`'C`=0, `'D`=1, `'E`=2), so caller `'E` coincides
/// with member `'U` at id 2 and the verbatim tier renders it unchanged.
/// Behavioural pin: the `5` flows through `accept`'s `Option['E]` output,
/// prints, and the build runs clean.
#[test]
fn structurally_pinned_member_input_stays_compiling_byte_identically() {
    let src = "\
import: core::list | List Nil Cons | ;

type: Option['T]
| None
| Some 'T
;

trait: Cursor['S: * -> *]
  : accept ( 'S['T] Option['U] -- Option['U] ) ;
;

impl: Cursor for List
  : accept swap drop ;
;

: ch ['C: Cursor 'D 'E] ( 'C['D] Option['E] -- Option['E] )
  accept ;

: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  1 2 3 mkempty ^ Cons ^ Cons ^ Cons
  5 Some
  ch
  ~[ ( None ) drop 0 . ]
  ~[ ( Some ) Some> . ]
  Option? ;
";
    let (_t, _binary, stdout) = build_run_keep("g11-pinned-member-input", src);
    assert_eq!(stdout, "5\n");
}
