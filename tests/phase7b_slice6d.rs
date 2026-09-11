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
//! Harness style from `tests/phase7b_slice8.rs` (the slice-golden successor
//! convention; `single_file_hosted` / `build_run_keep` /
//! `build_error_located`), self-contained like `tests/phase7b_slice6d_prereq.rs`.

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
