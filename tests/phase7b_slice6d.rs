//! P7b.S6d Phase 2 exit goldens: the two-half sentinel fence lift (REQ-2).
//!
//! G13 `trait_level_inline_spelling_still_builds_for_a_probe_local_slice_impl`
//! -- the sentinel grounds the App-headed protocol row against the concrete
//! slice target, so the probe-local impl (trait-level `next inline`, the
//! `as-done` body) builds and runs: one mono `next` step over a 5-element
//! view prints the element `3` and the remainder's length `4`.
//! G10 `noninline_slice_member_output_ban_holds_post_fix` -- with both
//! spellings non-inline, the member word's own output ban replaces the old
//! desugar fence, byte-exact (S6d-8.4(iii)).
//! G11 `fence_byte_stability_for_a_non_slice_concrete_target` -- the sentinel
//! branch claims `Concrete(Type::Slice(..))` only, so a `Unit` target keeps
//! raising the S2-6 fence byte-identically (negative control (i)'s permanent
//! shadow).
//!
//! P7b.S6d Phase 3 also lands its golden here (this file is the slice6d
//! golden successor): G7 `clobber_touch_mint_leaves_list_drain_resolutions_
//! untouched` (REQ-4) -- the pre-existing S8b-class clobber, fixed by the
//! env-hit candidate arm's union with the live check-time mints.
//!
//! P7b.S6d Phase 4 lands its goldens here too (REQ-5): G3
//! `rootless_join_asymmetry_refused_today_carried_by_the_union` (the union
//! carries the rootless deriv; build_ok), G4
//! `rooted_join_asymmetry_stays_refused_byte_identically` and G5
//! `two_root_join_asymmetry_stays_refused_byte_identically` (the refusals
//! keep their clean-tree bytes; the union is conditioned on
//! `owned_root.is_none()`). The join rule is generic -- no impl needed.
//!
//! G12 (negative control (ii), the reverted-dispatch-arm panic at
//! `ast.rs`'s `ground_member_type` App arm) is a MANUAL implementation-time
//! spike, deliberately NOT a suite golden: the panic it guards is unreachable
//! once the fix lands, and `build_error_located`'s no-`panic` discipline
//! would make any suite spelling of it self-defeating. Its evidence lives in
//! the implementation report; its positive twin is the checker-level unit
//! `slice_mono_member_call_reads_the_already_grounded_word_effect`
//! (`src/check/poly/ground.rs`).
//!
//! P7b.S6d Phase 5 (REQ-6, the library impl + consumer goldens) lands here
//! too. The shipped impl lives in `lib/core/iterator.sth` (the placement
//! gate's only legal home for a built-in-view target); these goldens run the
//! full pin list that phase's exit names. The build_run row: G1, G2, G6,
//! G8 (the phase's exit criterion: both lib impls drain through ONE
//! imported protocol), G14, G15, G16, G18. The byte-exact refusal row: G9,
//! G17, G19, G20, G21 (G10 landed with Phase 2 above). G22 is the whole
//! suite staying green with the six List/Range canaries of
//! `tests/phase7b_slice8.rs`.
//!
//! Harness style from `tests/phase7b_slice8.rs` (the slice-golden successor
//! convention; `single_file_hosted` / `build_run_keep` /
//! `build_error_located`), self-contained like `tests/phase7b_slice6d_prereq.rs`.
//! Phase 5's fixtures are written as verbatim multi-line strings (the
//! `phase7b_slice6d_prereq.rs` spelling) rather than `\n\` continuations:
//! the refusal goldens pin line/col measured in THIS harness context from
//! the composed file, and a verbatim string makes the composed bytes exactly
//! what the doc-comments measured. Every fixture drops its own
//! `import: hosted::show | . | ;` (the harness prepends it; a duplicate
//! collides) and keeps its `import: intrinsics * ;` (a duplicate wildcard is
//! harmless), so each composed file is the standalone fixture shifted by one
//! line — re-measured here, never transcribed from the probes doc.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs6d-{}-{tag}-{seq}", std::process::id()));
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
/// the package name. The prepended imports mean a golden's inline source must
/// NOT re-import `hosted::show`, and every `(line N)` below is measured in
/// this harness context (never transcribed from the standalone probes).
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs6d ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

/// Build `src`, assert it succeeds outright (the `build_ok` golden shape;
/// `tests/phase7b_slice8.rs`'s helper, verbatim but for the doc line).
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

/// Build `src`, assert it fails *located* (a clean non-zero exit with an
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
        "the admission-safety sweep must never panic, got: {stderr}"
    );
    stderr
}

/// G13 (`probes/s6d_a_fence_baseline.sth`'s spelling, harness-adapted): the
/// probe-local slice impl with the trait-level `next inline` spelling and the
/// `as-done` body builds and runs once the sentinel lifts the S2-6 fence --
/// the trait-level spelling stays legal where it always was (a probe-local
/// trait has no List/Range impls to break); the per-impl keyword (REQ-3) is
/// additive. Expected stdout `3\n4\n` (element, remainder length).
#[test]
fn trait_level_inline_spelling_still_builds_for_a_probe_local_slice_impl() {
    let (_t, _binary, stdout) = build_run_keep(
        "g13_slice_impl_trait_inline",
        // The fixture's own `import: hosted::show | . | ;` is dropped (the
        // harness prepends it); the duplicate `intrinsics *` is harmless.
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         \n\
         type: Step['T 'Rest]\n\
         | Done\n\
         | More 'T 'Rest\n\
         ;\n\
         \n\
         trait: Iterator['It: * -> *]\n\
         : next inline ( 'It['T] -- Step['T 'It['T]] ) ;\n\
         ;\n\
         \n\
         \\ Poly on purpose: a mono non-inline twin is banned at the word entry\n\
         \\ (Ruling B), and an inline twin splices to `drop Done`, whose push\n\
         \\ carries no deriv (the join disagreement again; S6d-8.2).\n\
         : as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;\n\
         \n\
         impl: Iterator for Slice[i64]\n\
         : next\n\
         | s |\n\
         s dup len |n|\n\
         n 0 >usize eq\n\
         ~[ as-done ]\n\
         ~[\n\
         dup 0 >usize &> @ |v|\n\
         1 >usize n 1 >usize sub subslice\n\
         v swap More\n\
         ]\n\
         if\n\
         ;\n\
         ;\n\
         \n\
         : main ( -- )\n\
         3 5 fill |buf|\n\
         &buf slice\n\
         next\n\
         ~[ ( Done ) drop ]\n\
         ~[ ( More ) More> |v r| v . r len >i64 . ]\n\
         Step?\n\
         buf drop\n\
         ;\n",
    );
    assert_eq!(stdout, "3\n4\n");
}

/// G10 (`probes/s6d_next_noninline.sth`'s spelling, harness-adapted): with
/// the trait member AND the impl member both non-inline, the sentinel grounds
/// the member word and the word's own output ban replaces the old desugar
/// fence -- byte-exact per S6d-8.4(iii). Pins the default rule (no impl
/// keyword -> the trait member's `declares_inline`, false here) and the
/// safety posture: a slice-bearing output may not escape a non-inline member.
#[test]
fn noninline_slice_member_output_ban_holds_post_fix() {
    let stderr = build_error_located(
        "g10_noninline_slice_member",
        "import: core::prelude * ;\n\
         \n\
         type: Step['T 'Rest]\n\
         | Done\n\
         | More 'T 'Rest\n\
         ;\n\
         \n\
         trait: Iterator['It: * -> *]\n\
         : next ( 'It['T] -- Step['T 'It['T]] ) ;\n\
         ;\n\
         \n\
         : as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;\n\
         \n\
         impl: Iterator for Slice[i64]\n\
         : next\n\
         | s |\n\
         s dup len |n|\n\
         n 0 >usize eq\n\
         ~[ as-done ]\n\
         ~[\n\
         dup 0 >usize &> @ |v|\n\
         1 >usize n 1 >usize sub subslice\n\
         v swap More\n\
         ]\n\
         if\n\
         ;\n\
         ;\n\
         \n\
         : main ( -- )\n\
         3 5 fill |buf|\n\
         &buf slice\n\
         next\n\
         ~[ ( Done ) drop ]\n\
         ~[ ( More ) More> |v r| v . r len >i64 . ]\n\
         Step?\n\
         buf drop\n\
         ;\n",
    );
    assert!(
        stderr.contains(
            "error: a reference cannot be stored: `next` (member of trait `Iterator` for `Slice[i64]`) declares the output `Step[i64 Slice[i64]]`\n  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead"
        ),
        "the member output ban fires byte-exact, got: {stderr}"
    );
}

/// G11 (`probes/s6d_l_fence_nonslice_target.sth`'s spelling,
/// harness-adapted): the sentinel branch claims `Concrete(Type::Slice(..))`
/// only, so a probe-local one-field-struct `Unit` target with the same
/// App-headed member keeps raising the S2-6 fence byte-identically (negative
/// control (i)'s permanent shadow). The `(line 16, col 3)` is measured in
/// THIS harness context, for this source's own spelling (the `\n\`
/// continuation strips the fixture's two-space member indent, so the member
/// token sits at col 3 here vs col 5 in the standalone probe; the message
/// text is the byte-stable part).
#[test]
fn fence_byte_stability_for_a_non_slice_concrete_target() {
    let stderr = build_error_located(
        "g11_nonslice_fence",
        "\n\
         type: Step['T 'Rest]\n\
         | Done\n\
         | More 'T 'Rest\n\
         ;\n\
         \n\
         type: Unit ;\n\
         \n\
         trait: Iterator['It: * -> *]\n\
           : next ( 'It['T] -- Step['T 'It['T]] ) ;\n\
         ;\n\
         \n\
         impl: Iterator for Unit\n\
           : next drop Done ;\n\
         ;\n\
         \n\
         : main ( -- ) 1 drop ;\n",
    );
    assert!(
        stderr.contains(
            "error: trait member `next` of `Iterator` (line 16, col 3) applies the trait variable `'It`, but the impl target `Unit` is concrete\n  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead"
        ),
        "the non-slice concrete-target fence stays byte-identical, got: {stderr}"
    );
}

/// G7 (`probes/s6d_m_clobber_touch.sth`'s spelling, harness-adapted):
/// `clobber_touch_mint_leaves_list_drain_resolutions_untouched` -- the
/// pre-existing S8b-class clobber (REQ-4, frame item 4). A bare signature
/// mention minting `Step[i64 Slice[i64]]` at parse (`touch`), placed before
/// a List-drain consumer, used to re-type the drain's bare `More>` to the
/// wrong monomorph: the env-hit candidate arm returned only the flushed
/// parse-time candidate and never saw the check-time `Step[i64 List[i64]]`
/// mint. Post-fix (the env-hit union with the live check-time mints,
/// keyed per-monomorph) both monomorphs are visible at the bare-name
/// lookup, the operand filter picks each site's own, and the drain runs.
/// Expected stdout `1\n2\n3\n`; the clean-tree capture was the byte-exact
/// `` `More>` expected `Step[i64 Slice[i64]].More`, found
/// `Step[i64 List[i64]].More` `` at this drain's `More>` line.
#[test]
fn clobber_touch_mint_leaves_list_drain_resolutions_untouched() {
    let (_t, _binary, stdout) = build_run_keep(
        "g7_clobber_touch_mint",
        // The fixture's own `import: hosted::show | . | ;` is dropped (the
        // harness prepends it); the duplicate `import: intrinsics * ;` is
        // harmless. The `\n\` continuation strips the fixture's two-space
        // member indent; G7 pins no line/col (a build_run golden), so the
        // strip is free here.
        "import: intrinsics * ;\n\
         import: core::prelude * ;\n\
         import: core::list | List Nil Cons | ;\n\
         import: core::iterator | Step Done More Iterator | ;\n\
         \n\
         : touch inline ( Step[i64 Slice[i64]] -- ) drop ;\n\
         \n\
         : drain ( List[i64] -- )\n\
         next\n\
         ~[ ( Done ) drop ]\n\
         ~[ ( More ) More> |v r| v . r drain ]\n\
         Step? ;\n\
         \n\
         : mkempty ( -- List[i64] ) Nil ;\n\
         \n\
         : main ( -- )\n\
         3 mkempty ^ Cons\n\
         2 swap ^ Cons\n\
         1 swap ^ Cons\n\
         drain ;\n",
    );
    assert_eq!(stdout, "1\n2\n3\n");
}

/// G3 (`probes/s6d_q_join_rootless_asymmetry.sth`'s spelling, harness-adapted
/// plus a trivial `main` so the golden links): the ROOTLESS one-sided join
/// asymmetry is carried by the union (REQ-5, frame item 5(a)). One if-arm
/// packs the word's seeded slice parameter directly (deriv-free -- parameters
/// are `Slot::computed`), the other binds then names it (the naming mint is a
/// reborrow with no `owned_root`). Clean tree: the refusal with S6d-8.2's
/// exact wording (``the second arm leaves a borrow with no local root``).
/// Post-fix: the union keeps that arm's deriv and the word checks --
/// `build_ok` -- with no other change.
#[test]
fn rootless_join_asymmetry_refused_today_carried_by_the_union() {
    build_ok(
        "g3_rootless_join_asymmetry",
        "import: core::prelude * ;\n\
         \n\
         type: Step['T 'Rest]\n\
         | Done\n\
         | More 'T 'Rest\n\
         ;\n\
         \n\
         : rootless-asym inline ( Slice[i64] -- Step[i64 Slice[i64]] )\n\
         \x20 True\n\
         \x20 ~[ 7 swap More ]\n\
         \x20 ~[ |x| x 7 swap More ]\n\
         \x20 if ;\n\
         \n\
         : main ( -- ) ;\n",
    );
}

/// G4 (`probes/s6d_j_join_rooted_asymmetry.sth`'s spelling, harness-adapted):
/// the ROOTED one-sided asymmetry stays refused BYTE-IDENTICALLY (desk-check
/// A.ii) -- the union is conditioned on `owned_root.is_none()`, and this
/// deriv's root is the frame local `a`. A `Step?` dispatch in a never-called
/// inline word with a declared `Step` output (the arms' bare ctors ground via
/// the tail channel): the Done arm rebuilds the state deriv-free
/// (`done-empty`), the More arm repacks the remainder of a view of `a`
/// (rooted deriv). Also pins the eliminator merge site
/// (`merge_arm_output_slot`) -- proof the second join site needs the same
/// treatment and keeps its rooted refusal. `(line 17)` is measured in THIS
/// harness context (the two prepended imports); the message body is the
/// byte-stable part.
#[test]
fn rooted_join_asymmetry_stays_refused_byte_identically() {
    let stderr = build_error_located(
        "g4_rooted_join_asymmetry",
        "import: core::prelude * ;\n\
         \n\
         type: Step['T 'Rest]\n\
         | Done\n\
         | More 'T 'Rest\n\
         ;\n\
         \n\
         : done-empty inline ( -- Step[i64 Slice[i64]] ) Done ;\n\
         \n\
         : rooted-asym inline ( -- Step[i64 Slice[i64]] )\n\
         \x20 3 1 fill |a|\n\
         \x20 &a slice |va|\n\
         \x20 va 7 swap More\n\
         \x20 ~[ ( Done ) drop done-empty ]\n\
         \x20 ~[ ( More ) More> More ]\n\
         \x20 Step? ;\n",
    );
    assert!(
        stderr.contains(
            "error: borrow state disagrees at the branch join in `rooted-asym` (line 17)\n  the first arm leaves no live borrow, the second arm leaves a borrow of `a`: both arms must agree on which place, if any, stays borrowed past the join\n  note: declared ( -- Step[i64 Slice[i64]] )"
        ),
        "the rooted refusal stays byte-identical, got: {stderr}"
    );
}

/// G5 (`probes/s6d_k_join_two_roots.sth`'s spelling, harness-adapted): the
/// (Some, Some) different-root refusal keeps happening byte-identically
/// (desk-check A.iii; Ruling F / SOO-41 territory at the join): two views of
/// two DIFFERENT frame locals, each arm packing its own, refuse rather than
/// pick one arm's root. `(line 14)` is measured in THIS harness context; the
/// message body is the byte-stable part.
#[test]
fn two_root_join_asymmetry_stays_refused_byte_identically() {
    let stderr = build_error_located(
        "g5_two_root_join_asymmetry",
        "import: core::prelude * ;\n\
         \n\
         type: Step['T 'Rest]\n\
         | Done\n\
         | More 'T 'Rest\n\
         ;\n\
         \n\
         : main ( -- )\n\
         \x20 3 1 fill |a| 4 1 fill |b|\n\
         \x20 &a slice |va| &b slice |vb|\n\
         \x20 True\n\
         \x20 ~[ vb 7 swap More ]\n\
         \x20 ~[ va 8 swap More ]\n\
         \x20 if\n\
         \x20 ~[ ( Done ) drop ] ~[ ( More ) More> drop drop ] Step?\n\
         \x20 a drop b drop ;\n",
    );
    assert!(
        stderr.contains(
            "error: borrow state disagrees at the branch join in `main` (line 14)\n  the first arm leaves a borrow of `b`, the second arm leaves a borrow of `a`: both arms must agree on which place, if any, stays borrowed past the join\n  note: declared ( -- )"
        ),
        "the two-root refusal stays byte-identical, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// P7b.S6d Phase 5 (REQ-6): the library impl + consumer goldens. The shipped
// impl lives in `lib/core/iterator.sth`; the fixtures below are the paper
// tests' G-set, written verbatim (real newlines, original indentation) so the
// pinned line/col are exactly the composed file's. Each drops the fixture's
// own `import: hosted::show | . | ;` (the harness prepends it) and keeps its
// `import: intrinsics * ;` (a duplicate wildcard is harmless) -- the composed
// file is the standalone fixture shifted by one line, re-measured here.
// ---------------------------------------------------------------------------

/// G1 (`probes/s6d_n_perimpl_inline_member.sth`'s spelling, harness-adapted):
/// `slice_impl_member_per_impl_inline_builds_and_runs` -- the per-impl
/// `inline` keyword (REQ-3) with the sentinel lift (REQ-2). The trait member
/// stays NON-inline (the HEAD spelling everywhere in `lib/`); the impl member
/// carries `: next inline`, so the synthesized member word's
/// `declares_inline` is the impl's spelling. The body keeps the `as-done`
/// Done arm so this golden isolates the inline-spelling item from 5(a) (the
/// natural body is G2's). Expected stdout `3\n4\n` (element, remainder
/// length): one `next` step over a 5-element view.
#[test]
fn slice_impl_member_per_impl_inline_builds_and_runs() {
    let (_t, _binary, stdout) = build_run_keep(
        "g1_perimpl_inline",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next inline
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

: main ( -- )
  3 5 fill |buf|
  &buf slice
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r len >i64 . ]
  Step?
  buf drop ;
"#,
    );
    assert_eq!(stdout, "3\n4\n");
}

/// G2 (`probes/s6d_p_natural_next_join.sth`'s spelling, harness-adapted):
/// `natural_next_body_checks_with_the_rootless_join_union` -- the NATURAL
/// next body (REQ-6): the Done arm is the protocol's own straight-line
/// spelling (`~[ drop Done ]`, no helper). The More arm's Step carries the
/// remainder view's deriv -- rootless, the receiver being a parameter, so
/// the naming mint has no owned root -- while the Done arm's nullary Step
/// carries none; the borrow join (REQ-5, P4) unions that rootless one-sided
/// asymmetry instead of refusing, and the member body checks once at
/// declaration (the tail channel grounds the arm-tail `Done` against the
/// member's declared output; verdict D). Expected stdout `3\n4\n`.
#[test]
fn natural_next_body_checks_with_the_rootless_join_union() {
    let (_t, _binary, stdout) = build_run_keep(
        "g2_natural_next",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next ( 'It['T] -- Step['T 'It['T]] ) ;
;

impl: Iterator for Slice[i64]
  : next inline
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ drop Done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

: main ( -- )
  3 5 fill |buf|
  &buf slice
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r len >i64 . ]
  Step?
  buf drop ;
"#,
    );
    assert_eq!(stdout, "3\n4\n");
}

/// G6 (`probes/s6d_h_while_drain.sth`, harness-adapted with ONE measured
/// semantic correction -- see below):
/// `while_threaded_slice_drain_runs_once_back_edge_outs_forward_deriv`.
/// The while-threaded drain: the threaded state is the `Step` value itself,
/// the row pinned by the quotation annotation. 5(b)'s `back_edge_outs`
/// deriv forwarding is what lets the while-internal join
/// (`lib/core/combinators.sth`'s `~[ p while ] ~[ ] if`) accept the
/// deriv-carrying state crossing the self-tail back edge; without it the
/// build dies at that join (S6d-10.3's capture, the PREREQ-mask finding).
///
/// SEMANTIC ADAPTATION (the first in this slice -- every earlier harness
/// adaptation was spelling-only). The frozen fixture's More arm is
/// `~[ ( More ) More> |v r| v v . r More True ]`: it repacks the SAME
/// remainder into `More` without calling `next`, so the while state is a
/// non-advancing fixed point and the drain never terminates -- measured:
/// unbounded `6` lines, killed at timeout. The paper tests' post-fix stdout
/// `6\n6\n6\n` was an unmeasured desk-prediction (verdict C: "the post-fix
/// run is a prediction from the traced mechanics (no patched tree was built
/// this round)"), and the fixture could never have been run end to end
/// before this slice because on the clean tree it dies at while's internal
/// join. The corrected arm,
/// `~[ ( More ) More> |v r| v . r next True ]`, advances the state (one
/// `next` step per iteration; the duplicate `v` existed only to feed the
/// same-arm repack) and produces exactly the predicted stdout. The property
/// under test is unchanged: the state crossing while's back edge is still
/// the `Step` whose payload carries the rootless view deriv, so the join
/// still requires 5(b)'s forward. F5's stall clause ("if 5(b) stalls, G6 is
/// withdrawn") was considered and does not apply: 5(b) landed; the frozen
/// fixture's own text was what could not produce the pinned stdout.
/// Expected stdout `6\n6\n6\n`, exit 0 (3 elements of 6, one print per
/// `More`).
#[test]
fn while_threaded_slice_drain_runs_once_back_edge_outs_forward_deriv() {
    let (_t, _binary, stdout) = build_run_keep(
        "g6_while_drain",
        r#"import: intrinsics * ;
import: core::prelude * ;
import: core::combinators c | while | ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The while-threaded drain (S6d-10.3): the threaded state 'a is the STEP
\ value itself, the row pinned by the quotation annotation so the inner
\ arms' reconstructions have a concrete consumer.
: drain ( Slice[i64] -- )
  |s|
  s next
  ~[ ( Step[i64 Slice[i64]] -- Step[i64 Slice[i64]] Bool )
     ~[ ( Done ) drop s 0 >usize 0 >usize subslice as-done False ]
     ~[ ( More ) More> |v r| v . r next True ]
     Step? ]
  while
  drop ;

: main ( -- )
  6 3 fill |buf|
  &buf slice drain
  buf drop
;
"#,
    );
    assert_eq!(stdout, "6\n6\n6\n");
}

/// G8 (`probes/s6d_o_lib_slice_end_to_end.sth`'s spelling, harness-adapted)
/// -- THE PHASE'S EXIT CRITERION:
/// `list_and_slice_impls_drain_through_one_imported_protocol`. One program,
/// both lib impls, bare `next` dispatching to each: a List drain (1 2 3)
/// and a slice drain (3 3 3), two `Step` monomorphs coexisting -- the
/// clobber fix (REQ-4) proven by coexistence through the exported surface.
/// Before the slice impl shipped, the slice drain's `next` call had nothing
/// to dispatch on (`mono_member_no_dispatch_error` naming `Slice[i64]`).
/// Expected stdout `1\n2\n3\n3\n3\n3\n`, exit 0.
#[test]
fn list_and_slice_impls_drain_through_one_imported_protocol() {
    let (_t, _binary, stdout) = build_run_keep(
        "g8_one_protocol",
        r#"import: intrinsics * ;
import: core::prelude * ;
import: core::list | List Nil Cons | ;
import: core::iterator | Step Done More Iterator | ;

: drain-list ( List[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r drain-list ]
  Step? ;

: drain-slice ( Slice[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r drain-slice ]
  Step? ;

: mkempty ( -- List[i64] ) Nil ;

: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  drain-list
  3 3 fill |buf|
  &buf slice drain-slice
  buf drop ;
"#,
    );
    assert_eq!(stdout, "1\n2\n3\n3\n3\n3\n");
}

/// G14 (`probes/s6d_b_step_shared_inline.sth`, harness-adapted):
/// `prereq_admissions_stay_byte_green` -- the PREREQ admissions isolated
/// from the fence: a plain inline word packs a shared slice into the
/// declared `Step` payload (REQ-5 admit-and-taint), a mono consumer
/// destructures it, and the packed container is `dup`'d and double-dropped
/// clean (a shared-slice container stays Copy, Ruling A). No trait, no
/// impl. Today AND post-fix: stdout `41\n5\n`; the golden fails if any
/// frame item disturbed the PREREQ admissions.
#[test]
fn prereq_admissions_stay_byte_green() {
    let (_t, _binary, stdout) = build_run_keep(
        "g14_prereq_admissions",
        r#"import: intrinsics * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

: mk inline ( i64 Slice[i64] -- Step[i64 Slice[i64]] ) More ;

\ REQ-5 copy check: the packed container dups and double-drops clean.
: dup-check ( i64 Slice[i64] -- ) More dup drop drop ;

: main ( -- )
  3 5 fill |buf|
  &buf slice |s|
  41 s mk
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r len >i64 . ]
  Step?
  42 s dup-check
  s drop
  buf drop
;
"#,
    );
    assert_eq!(stdout, "41\n5\n");
}

/// G15 (`probes/s6d_c_impl_consumer.sth`, harness-adapted):
/// `monomorphic_consumer_drains_two_views_end_to_end` -- the monomorphic
/// consumer (S6d-8.3): `drain` walks a view with bare `next` (the
/// `resolve_mono_member_call` slice arm), `Step?` dispatch arms and a
/// `More>` destructure. The self-call is NOT in tail position (`r drain v .`
/// -- the element prints after the recursion returns), the S6d-10.2 shape:
/// a bare `Slice[i64]` input to a non-inline word is admissible. The second
/// drain (a 1-element view) proves the `Done` path runs at runtime. Expected
/// stdout `3\n3\n3\n3\n3\n9\n`.
#[test]
fn monomorphic_consumer_drains_two_views_end_to_end() {
    let (_t, _binary, stdout) = build_run_keep(
        "g15_mono_consumer",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

: drain ( Slice[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| r drain v . ]
  Step? ;

: main ( -- )
  3 5 fill |buf|
  &buf slice drain

  9 1 fill |one|
  &one slice drain

  buf drop one drop
;
"#,
    );
    assert_eq!(stdout, "3\n3\n3\n3\n3\n9\n");
}

/// G16 (`probes/s6d_d_selftail.sth`, harness-adapted):
/// `selftail_drain_passes_the_back_edge_guard_with_a_parameter_rooted_remainder`
/// -- the self-TAIL drain: `r drain` is the arm's last term, so the P7.S3g
/// transform lowers the recursion to a loop back-edge and the remainder (a
/// tainted view) crosses it. Parameter-rooted remainders have no
/// `owned_root`, so `check_reference_across_back_edge`'s accept-case admits
/// them (S6d-10.1: the prediction was falsified -- this is the measured
/// accept). Expected stdout `3\n3\n3\n3\n3\n`.
#[test]
fn selftail_drain_passes_the_back_edge_guard_with_a_parameter_rooted_remainder() {
    let (_t, _binary, stdout) = build_run_keep(
        "g16_selftail",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The self-TAIL drain: `r drain` is the arm's last term, so the P7.S3g
\ transform lowers the recursion to a loop back-edge and the remainder
\ (a tainted view) would cross it.
: drain ( Slice[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r drain ]
  Step? ;

: main ( -- )
  3 5 fill |buf|
  &buf slice drain
  buf drop
;
"#,
    );
    assert_eq!(stdout, "3\n3\n3\n3\n3\n");
}

/// G18 (`probes/s6d_e_nontail_drain.sth`, harness-adapted):
/// `nontail_drain_runs_with_one_frame_per_element` -- the NON-tail drain:
/// the self-call is not the arm's last term, the element prints after the
/// recursion returns, and the drain runs in O(n) stack (one real frame per
/// element) -- the admissible non-tail consumer shape (S6d-10.2; a bare
/// `Slice[i64]` input to a non-inline word stays admissible, the
/// `examples/slices.sth` `sum` precedent). Expected stdout `4\n4\n4\n4\n`.
#[test]
fn nontail_drain_runs_with_one_frame_per_element() {
    let (_t, _binary, stdout) = build_run_keep(
        "g18_nontail",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The NON-tail drain: the self-call is NOT the arm's last term — the element
\ prints after the recursion returns. A bare Slice[i64] input to a non-inline
\ word is admissible (the examples/slices.sth `sum` precedent): a real call
\ frame cannot outlive its caller. Runs in O(n) stack (one frame per element).
: drain ( Slice[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| r drain v . ]
  Step? ;

: main ( -- )
  4 4 fill |buf|
  &buf slice drain
  buf drop
;
"#,
    );
    assert_eq!(stdout, "4\n4\n4\n4\n");
}

/// G9 (`probes/s6d_g_mut_impl.sth`, harness-adapted):
/// `mutable_slice_impl_first_firing_is_the_enum_payload_sweep` -- the
/// mutable `!Slice[i64]` target's deferral (REQ-1, SOO-45): Ruling A's
/// enum-payload sweep fires FIRST, at the fixture's own `Step` type's
/// `| More 'T 'Rest` variant span, before any member check (the member is
/// inline, so no word-entry ban). Byte-exact per S6d-9, re-measured in THIS
/// harness context: the standalone capture's `(line 7, col 3)` is `(line 8,
/// col 3)` here (the two prepended imports minus the dropped
/// `hosted::show` import).
#[test]
fn mutable_slice_impl_first_firing_is_the_enum_payload_sweep() {
    let stderr = build_error_located(
        "g9_mut_sweep",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for !Slice[i64]
  : next
    | s |
    s len |n|
    n 0 >usize eq
    ~[ s as-done ]
    ~[
      s 0 >usize &!> @ |v|
      s 1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

: main ( -- ) 1 drop ;
"#,
    );
    assert!(
        stderr.contains(
            "error: a reference cannot be stored: payload field 1 of variant `More[i64 !Slice[i64]]` of type `Step[i64 !Slice[i64]]` has type `!Slice[i64]` (line 8, col 3)\n  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow"
        ),
        "the Ruling A sweep fires first, byte-exact, got: {stderr}"
    );
}

/// G17 (`probes/s6d_d2_selftail_inline.sth`, harness-adapted):
/// `selftail_inline_drain_still_rejected_at_the_back_edge_guard` -- G16's
/// sharper twin: the drain word itself spelled `inline` splices into `main`,
/// so the remainder crossing the lowered back edge derives from `buf`, a
/// local of the caller's frame -- and `check_reference_across_back_edge`
/// (untouched by this slice, the SOO-42 gate) rejects it at the
/// root-visibility boundary. Byte-exact per S6d-10.1, re-measured in THIS
/// harness context: the standalone capture's `(line 37)` is `(line 38)`
/// here.
#[test]
fn selftail_inline_drain_still_rejected_at_the_back_edge_guard() {
    let stderr = build_error_located(
        "g17_selftail_inline",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The self-TAIL drain: `r drain` is the arm's last term, so the P7.S3g
\ transform lowers the recursion to a loop back-edge and the remainder
\ (a tainted view) would cross it.
: drain inline ( Slice[i64] -- )
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> |v r| v . r drain ]
  Step? ;

: main ( -- )
  3 5 fill |buf|
  &buf slice drain
  buf drop
;
"#,
    );
    assert!(
        stderr.contains(
            "error: a reference to a local cannot cross a loop in `main` (line 38)\n  a reference derived from `buf`, a local of this frame, crosses the self-tail-call back-edge to `drain`: that local's storage does not survive to the next iteration\n  note: declared ( -- )"
        ),
        "the back-edge guard reject-case stays byte-identical, got: {stderr}"
    );
}

/// G19: `slice_impl_over_an_imported_trait_still_must_live_in_the_declaring_module`
/// -- the placement gate (S6d-8.6 gate 1), byte-exact in THIS harness
/// context. TRAIT SUBSTITUTION, measured: the frozen fixture
/// (`probes/s6d_f0_libiter_gate.sth`) spells the imported trait as
/// `Iterator`, but Phase 5's own lib impl now lawfully occupies the
/// `(Iterator, Slice[i64])` slot, and the duplicate-impl scan
/// (`check_impl_decls`'s first loop, which precedes the orphan rule)
/// fires first for that exact spelling -- measured here:
/// ``duplicate `impl:` for `Slice[i64]` (line 80, col 1); first declared at
/// line 9, col 1`` (the first span is the lib impl's own in
/// `lib/core/iterator.sth`; the lib-internal line is deliberately not
/// pinned). The gate itself is unchanged and still fires for a slice
/// target over any other imported trait, so this golden pins it via
/// `Ord` from `core::cmp` -- same target, same None-target-module clause,
/// byte-exact: the co-declaration arm is structurally unavailable for a
/// built-in view (`Slice` declares no module of its own), so
/// `core/iterator.sth` stays the only home for the `Iterator` impl.
#[test]
fn slice_impl_over_an_imported_trait_still_must_live_in_the_declaring_module() {
    let stderr = build_error_located(
        "g19_placement_gate",
        r#"import: intrinsics * ;
import: core::cmp | Ord | ;

impl: Ord for Slice[i64]
  : cmp drop 0 ;
;

: main ( -- ) 1 drop ;
"#,
    );
    assert!(
        stderr.contains(
            "error: `impl: Ord for Slice[i64]` at line 6, col 1 must live in the module declaring `Ord` (`Slice[i64]` declares no module of its own)"
        ),
        "the placement gate fires byte-exact for a slice target, got: {stderr}"
    );
}

/// G20: `bound_generic_consumers_stay_closed_at_the_slot_unification` --
/// the SOO-60 boundary (S6d-8.6 gate 3): for_each/fold are NOT touched by
/// this slice. Both twins (the lib consumers' bodies verbatim over a
/// probe-local Iterator with an inline member, per the frozen fixtures
/// `probes/s6d_f_for_each_slice.sth` and `probes/s6d_f2_fold_slice.sth`)
/// die at the CALL SITE, before any App fence or back-edge check: the
/// bound slot `'It['T]` is App-headed and decomposes only against a ctor
/// application -- a slice is a bare `Concrete` view with no ctor head, so
/// `'It` has nothing to bind to (`unify_poly_input`). Byte-exact per
/// S6d-8.6, re-measured in THIS harness context: the standalone captures'
/// `(line 41)`/`(line 49)` are `(line 42)`/`(line 50)` here. The golden
/// guards against accidentally admitting a slice to the bound slot.
#[test]
fn bound_generic_consumers_stay_closed_at_the_slot_unification() {
    let for_each_stderr = build_error_located(
        "g20_bound_for_each",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The lib's for_each, verbatim, over the probe-local trait.
: for_each ['It: Iterator 'T] ( 'It['T] [ 'T -- ] -- )
  | f |
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> | v rest | v f call rest f for_each ]
  Step? ;

: main ( -- )
  3 5 fill |buf|
  &buf slice [ . ] for_each
  buf drop
;
"#,
    );
    assert!(
        for_each_stderr.contains(
            "error: type mismatch in `main` (line 42)\n  `for_each` expected `'It['T]`, found `Slice[i64]`\n  note: declared ( -- )"
        ),
        "the for_each twin stays closed at the slot unification, byte-exact, got: {for_each_stderr}"
    );
    let fold_stderr = build_error_located(
        "g20_bound_fold",
        r#"import: intrinsics * ;
import: core::prelude * ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The lib's for_each, verbatim, over the probe-local trait.
: for_each ['It: Iterator 'T] ( 'It['T] [ 'T -- ] -- )
  | f |
  next
  ~[ ( Done ) drop ]
  ~[ ( More ) More> | v rest | v f call rest f for_each ]
  Step? ;


: fold ['It: Iterator 'T 'A] ( 'It['T] 'A [ 'A 'T -- 'A ] -- 'A )
  | f | | acc |
  next
  ~[ ( Done ) drop acc ]
  ~[ ( More ) More> | v rest | rest acc v f call f fold ]
  Step? ;

: main ( -- )
  3 5 fill |buf|
  &buf slice 0 [ add ] fold .
  buf drop
;
"#,
    );
    assert!(
        fold_stderr.contains(
            "error: type mismatch in `main` (line 50)\n  `fold` expected `'It['T]`, found `Slice[i64]`\n  note: declared ( -- )"
        ),
        "the fold twin stays closed at the slot unification, byte-exact, got: {fold_stderr}"
    );
}

/// G21 (`probes/s6d_i_times_drain.sth`, harness-adapted):
/// `times_hosted_slice_drain_stays_closed_at_the_abstract_row` -- the
/// times-bounded drain (S6d-10.4): the times quotation is checked
/// standalone against the abstract row, so `next`'s member dispatch has no
/// concrete operand -- `mono_member_no_dispatch_error` with an EMPTY
/// operand-type list (the row renders as nothing; the cosmetic is noted,
/// not fixed, in this slice). The wall is the standalone check, not the
/// member spelling -- unchanged by the frame. Byte-exact re-measured in
/// THIS harness context: the standalone capture's `(line 43, col 7)` is
/// `(line 44, col 7)` here.
#[test]
fn times_hosted_slice_drain_stays_closed_at_the_abstract_row() {
    let stderr = build_error_located(
        "g21_times_abstract_row",
        r#"import: intrinsics * ;
import: core::prelude * ;
import: core::combinators c | times | ;

type: Step['T 'Rest]
| Done
| More 'T 'Rest
;

trait: Iterator['It: * -> *]
  : next inline ( 'It['T] -- Step['T 'It['T]] ) ;
;

: as-done ['R] ( 'R -- Step[i64 'R] ) drop Done ;

impl: Iterator for Slice[i64]
  : next
    | s |
    s dup len |n|
    n 0 >usize eq
    ~[ as-done ]
    ~[
      dup 0 >usize &> @ |v|
      1 >usize n 1 >usize sub subslice
      v swap More
    ]
    if
;
;

\ The times-bounded drain (S6d-10.4): exactly 3 next-steps over a 5-element
\ view, the row-threaded state the view. REJECTED on this tree: the times
\ quotation is checked standalone against the abstract row, so `next`'s
\ member dispatch has no concrete operand (mono_member_no_dispatch_error,
\ empty operand list); annotating the quotation concretely is refused too
\ (times' declared row renders `~[ i64 -- ]`, the ..s row is not
\ annotation-comparable). Bounded stepping works unrolled (see the report).
: drain3 ( Slice[i64] -- )
  |s|
  3 ~[
      drop
      next
      ~[ ( Done ) drop s 0 >usize 0 >usize subslice ]
      ~[ ( More ) More> |v r| v . r ]
      Step?
    ] times
  len >i64 . ;

: main ( -- )
  3 5 fill |buf|
  &buf slice drain3
  buf drop
;
"#,
    );
    assert!(
        stderr.contains(
            "error: `next` in `drain3` (line 44, col 7) is a trait member of Iterator, but no `impl:` in this program dispatches on these operands\n  the operand types here are ``; declare an impl of one of those traits for the operand's type, or import a word that claims this name"
        ),
        "the abstract-row no-dispatch stays byte-identical, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Round-1 review fixes (the P3 env-hit union vs the P5 consumer-type
// tie-break). The union arm sets `from_fallback`, which routes the
// multi-candidate dispatch through `select_overload_fallback_sourced` --
// whose tier-1 miss ends in `matching.first()`, i.e. parse order -- and that
// preempts the `OverloadPick::Ambiguous` arm's
// `generated_enum_consumer_type_pick`: with a pending same-family mint live,
// a nullary generated-enum ctor site with a UNIQUE consumer_expected_type
// match resolved by parse-order luck or refused with a misleading
// type-mismatch instead of resolving from its consumer. The fix runs the
// same tie-break over the union'd set before the fallback dispatch; a
// decline falls through byte-identically (pinned by the second unit below).
// ---------------------------------------------------------------------------

/// The P1 repro shape, end to end: `f` pins a bare nullary `Done` to
/// `Step[i64 i64]` via its own declared output, `g`'s signature mention
/// flushes a *different* monomorph (`Step[str i64]`) into the word env ahead
/// of it (parse order), and one unrelated mid-word poly instantiation
/// (`1 pack`, at `'R = i64`, minting the same-family `Step[u8 i64]`) makes
/// the pending-mint check true, so the env-hit arm unions flushed candidates
/// with live mints and dispatches fallback-sourced. Before the fix the
/// tier-1-miss first-match picked `g`'s earlier-flushed monomorph and `f`
/// refused (`body leaves `Step[str i64]` where the declaration requires
/// `Step[i64 i64]``); the control without the pack line printed 44. After
/// the fix the tie-break fires over the union'd set -- the consumer's
/// `Step[i64 i64]` matches exactly one candidate -- and the program resolves
/// to the consumer's mint and prints the control's output.
#[test]
fn env_hit_union_with_pending_mint_and_unique_consumer_resolves_to_consumer_type() {
    let (_t, _binary, stdout) = build_run_keep(
        "s6d_p1_env_hit_union_consumer_pick",
        r#"import: core::iterator | Step Done More Iterator | ;

: g ( Step[str i64] -- ) drop ;

: pack ['R] ( 'R -- Step[u8 'R] ) drop Done ;

: f ( -- Step[i64 i64] )
  1 pack drop
  Done ;

: main ( -- )
  f
  ~[ ( Done ) drop 44 . ]
  ~[ ( More ) More> drop drop ]
  Step? ;
"#,
    );
    assert_eq!(stdout, "44\n");
}

/// The fix's decline path, pinned: the same union'd set (one flushed
/// candidate plus one pending same-family mint), but the site's consumer
/// (`h`) expects a type from another family entirely, so
/// `generated_enum_consumer_type_pick` finds no unique match, declines, and
/// the pre-existing fallback-sourced dispatch decides -- today, tier-1 miss
/// then `matching.first()`, which is the first env candidate (parse order:
/// the union chains env candidates ahead of the mints, so the pick is
/// deterministic on this tree). This unit DISCLOSES that it pins the current
/// fallback-sourced first-match behaviour, wrong monomorph and all: the
/// `h`-site mismatch names `g`'s earlier-flushed `Step[str i64]`. A later
/// slice that re-works the parse-order pick must retire this pin
/// deliberately, as the slice8b fence golden was.
#[test]
fn env_hit_union_fall_through_no_unique_consumer_keeps_fallback_first_match_bytes() {
    let stderr = build_error_located(
        "s6d_p1_fallthrough_bytes",
        r#"import: core::iterator | Step Done More Iterator | ;
import: core::list | List | ;

: g ( Step[str i64] -- ) drop ;

: h ( List[i64] -- ) drop ;

: pack ['R] ( 'R -- Step[u8 'R] ) drop Done ;

: f ( -- )
  1 pack drop
  Done
  h ;
"#,
    );
    assert!(
        stderr.contains(
            "error: type mismatch in `f` (line 15)\n  `h` expected `List[i64]`, found `Step[str i64]`"
        ),
        "the no-unique-match fall-through keeps today's fallback first-match bytes, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// P0 (the follow-up review of 20d4add itself): the pre-dispatch tie-break's
// unique-consumer pick can BE a check-time mint -- its `Type::Enum` id is
// past the frozen registry snapshot (`ctx.enums()`), so the push path's
// nullary-variant read (`terms.rs`'s `nullary_variant_idx`, the
// `ctx.enums()[id.index()]` indexing) panicked with an index-out-of-bounds,
// exit 101, no diagnostic. Hardening: the read goes through
// `with_extended_type_slices` (frozen prefix ++ the live cell's pending
// tail), the same convention `is_generated_enum_word`/`splice_enum_site`
// already follow -- so a pick of a live mint resolves the way the ratified
// tie-break contract's success path says, and compiles + runs.
// ---------------------------------------------------------------------------

/// The P0 repro, end to end: `1 pack` (a mid-word poly call at `'R = i64`)
/// mints the check-time `Step[u8 i64]` family monomorph into the live cell,
/// so the `Done` site's env-hit union contains it; the consumer pin
/// (`use[i64]` wants `Step[u8 i64]`) makes the tie-break's unique match that
/// mint. PRE-FIX (captured on 20d4add): the build panicked
/// ``thread 'main' panicked at src/check/terms.rs:1491:21:
/// index out of bounds: the len is 3 but the index is 3``, exit 101, no
/// diagnostic. POST-FIX the site resolves to the consumer's declared type
/// and the program runs, printing `use`'s body output (`0`).
#[test]
fn tie_break_pick_of_a_check_time_mint_resolves_instead_of_panicking() {
    let (_t, _binary, stdout) = build_run_keep(
        "s6d_p0_tie_break_mint_push",
        r#"import: core::iterator | Step Done More Iterator | ;

: pack ['R] ( 'R -- Step[u8 'R] ) drop Done ;

: use ['A] ( Step[u8 'A] -- i64 ) drop 0 ;

: f ( -- i64 ) 1 pack drop Done use[i64] ;

: main ( -- ) f . ;
"#,
    );
    assert_eq!(stdout, "0\n");
}

/// The env-MISS arm's all-mints shape, honestly pinned: the arm IS entered
/// with live mints, but the pick is S11 grounding, not the fallback dispatch.
/// `sdone` is a bare ctor of the caller's OWN generic header with zero
/// parse-time mints (nothing concrete names `S` in any signature), so `env`
/// has no `sdone` entry; the two mid-word poly calls (`1 wr` at `'R = i64`,
/// `True wr` at `'R = bool`) each mint a check-time `S[...]` monomorph into
/// the live cell, so `mint_fallback_candidates` returns 2 live candidates and
/// the all-mints arm hands them to the candidate ladder. Measured on an
/// instrumented tree: `ground_bare_generic_ctor` runs over those candidates
/// and GROUNDS from the consumer pin (`use2[i64]` wants `S[i64]`) onto the
/// pre-existing check-time mint -- `select_overload_fallback_sourced` never
/// runs for this shape. With a single own-module header, S11 either grounds
/// or errors; it never declines to the dispatch (that route is the fourth
/// unit below, which needs S11 to decline). So this pins the env-miss
/// all-mints ARM ENTRY plus the S11-grounding pick onto a PRE-EXISTING mint
/// -- distinct from the third unit, which grounds a FRESH mint from an empty
/// cell. Fail-before (captured on the pre-fix 20d4add tree): the same
/// ``index out of bounds: the len is 2 but the index is 2`` at
/// `terms.rs:1491`, exit 101. POST-FIX it reads the pending decl through the
/// extended slices and the site resolves (`use2[i64]`'s pin matches the
/// grounded mint's output exactly), so the program runs and prints `0`.
#[test]
fn env_miss_all_mints_arm_s11_grounds_onto_a_pre_existing_check_time_mint_resolves() {
    let (_t, _binary, stdout) = build_run_keep(
        "s6d_p0_env_miss_all_mints",
        r#"import: intrinsics * ;


type: S['A]
| sdone
| smore 'A
;

: wr ['R] ( 'R -- S['R] ) drop sdone ;

: use2 ['A] ( S['A] -- i64 ) drop 0 ;

: f ( -- i64 )
  1 wr drop
  True wr drop
  sdone use2[i64] ;

: main ( -- ) f . ;
"#,
    );
    assert_eq!(stdout, "0\n");
}

/// The second S11 route of the same P0 read, also reachable pre-20d4add (the
/// review cites b777cbb for it too; the fail-before capture is on the
/// pre-fix 20d4add tree, the same ``index out of bounds: the len is 2 but
/// the index is 2``, exit 101): an own-module header whose S11 grounding
/// succeeds from a consumer pin onto a check-time mint. `f` has no prior
/// poly call, so at the bare `sdone` site the live cell is empty and the
/// env-miss arm takes the S11 ladder; the consumer pin (`use2[i64]` wants
/// `S[i64]`) fully determines theta, the grounding mints that instantiation
/// check-time (its id lands past the frozen snapshot), and the grounded
/// candidate's nullary output carries the mint's id straight into the push
/// path's read. PRE-FIX that read panicked; POST-FIX it resolves through
/// the extended slices and the program runs, printing `0`.
#[test]
fn s11_consumer_pin_grounding_onto_a_check_time_mint_resolves() {
    let (_t, _binary, stdout) = build_run_keep(
        "s6d_p0_s11_grounding_mint",
        r#"import: intrinsics * ;


type: S['A]
| sdone
| smore 'A
;

: use2 ['A] ( S['A] -- i64 ) drop 0 ;

: f ( -- i64 ) sdone use2[i64] ;

: main ( -- ) f . ;
"#,
    );
    assert_eq!(stdout, "0\n");
}

/// The TRUE env-miss dispatch-over-mints route, now pinned. S11 must DECLINE
/// at the bare ctor site so the multi-candidate fallback dispatch runs, and
/// the measured way to decline is 2+ groundable own-module headers claiming
/// the ctor's surface name: `ctor_grounding_header` counts claimants across
/// every own-module generic header and returns `None` on 2+. (Two same-named
/// *headers* are not the way -- `check_duplicate_type_names` rejects them
/// before any body is checked, measured on 20d4add: `duplicate type 'S'` --
/// so the reachable decline is two differently-named headers sharing the
/// variant ctor's name, here `S`'s and `T`'s `sdone`.) Only the FIRST
/// same-named claimant is ever minted (`poly_construction_header` is a
/// first-match find), so the `T` header stays unminted: the two mid-word
/// poly calls mint `S[i64]` and `S[bool]`, the bare `sdone` site enters the
/// env-miss all-mints arm with those 2 live candidates, S11 declines, and
/// `select_overload_fallback_sourced` runs -- tier-1's caller-module find
/// matches both mints (both minted under the caller's module) and takes the
/// FIRST, `sdone[i64]` (measured on an instrumented tree: candidates=2,
/// caller_module=0, syms=["sdone[i64]", "sdone[Bool]"]). The env-miss arm
/// sets no `env_hit_union`, so the round-1 consumer-type tie-break does not
/// preempt the dispatch here. Fail-before (captured on the pre-fix 20d4add
/// tree): ``thread 'main' panicked at src/check/terms.rs:1491:21: index out
/// of bounds: the len is 2 but the index is 2``, exit 101, no diagnostic.
/// POST-FIX the push path's nullary read resolves the first mint through the
/// extended slices, `use2[i64]`'s pin matches its `S[i64]` output, and the
/// program runs, printing `0`.
#[test]
fn env_miss_dispatch_over_mints_picks_the_first_check_time_mint_resolves() {
    let (_t, _binary, stdout) = build_run_keep(
        "s6d_p0_env_miss_dispatch_over_mints",
        r#"import: intrinsics * ;


type: S['A]
| sdone
| smore 'A
;

type: T['A]
| sdone
| tmore 'A
;

: wr ['R] ( 'R -- S['R] ) drop sdone ;

: use2 ['A] ( S['A] -- i64 ) drop 0 ;

: f ( -- i64 )
  1 wr drop
  True wr drop
  sdone use2[i64] ;

: main ( -- ) f . ;
"#,
    );
    assert_eq!(stdout, "0\n");
}

/// The tag path's identical crash class, closed the same way -- pinned as the
/// refusal it is, not a resolution. `sdone[i64]` grounds through S11's
/// explicit-args route, minting the check-time `S[i64]` monomorph (frozen
/// registry len 2 -- `Bool` plus `Ordering` (core's two concrete enums;
/// `S` itself is a generic header, registered only in the live cell, not
/// the frozen registry); the mint's `EnumId` is 2, past the
/// snapshot) onto the stack, where `tag` reads it. Fail-before (captured on
/// the P0-fixed, tag-unfixed tree -- on the bare pre-fix 20d4add the ctor's
/// own push-path read panics first, at `terms.rs:1491`, so the tag read is
/// never reached): ``thread 'main' panicked at src/check/word_families.rs:774:9:
/// index out of bounds: the len is 2 but the index is 2``, exit 101, no
/// diagnostic. POST-FIX the declaration read goes through
/// `with_extended_type_slices` and finds the mint -- and then the
/// pre-existing scalar-domain gate refuses, because a scalar generic enum
/// CANNOT exist: the phantom-parameter rule rejects a header whose type
/// variable appears in no field (``type: S['A] | sdone | salso ;`` is itself
/// a located declaration error: "a phantom parameter cannot be disambiguated
/// at a call site"), so every generic enum has a payload variant, so the
/// all-payload-free predicate is false for every mint. tag-on-mint never
/// worked (it panicked); the pin is the located refusal that replaces the
/// ICE, with tag's frozen-id behavior byte-identical (frozen scalar enums
/// read through the unchanged prefix).
#[test]
fn tag_on_an_s11_grounded_check_time_mint_refuses_located_instead_of_panicking() {
    let stderr = build_error_located(
        "s6d_p0_tag_on_s11_mint_refusal",
        r#"import: intrinsics * ;


type: S['A]
| sdone
| smore 'A
;

: f ( -- ) sdone[i64] tag . ;

: main ( -- ) f ;
"#,
    );
    assert!(
        stderr.contains("error: type mismatch in `f` (line 11)"),
        "the refusal must be located at the tag site, got: {stderr}"
    );
    assert!(
        stderr
            .contains("`tag` requires an enum whose variants all carry no payload, found `S[i64]`"),
        "the refusal must be the pre-existing payload-enum message with the mint named, got: {stderr}"
    );
}
