//! P7.S3v goldens: dropping and storing a linear-capturing quotation.
//!
//! P7.S3h shipped `owning [ … ]` with two restrictions, both because nothing
//! could invoke a per-value disposer: `drop` on one was rejected outright, and
//! no aggregate position could hold one. This slice mints a disposer per
//! *construction site* (where the capture types are known) into the quotation
//! value's third word, so both restrictions lift for the three positions whose
//! container synthesizes a destructor.
//!
//! Every golden here disposes a `Spy`, whose user `drop` prints. A leak is
//! silent otherwise: the whole failure mode this slice guards is a `drop` that
//! discharges the obligation and frees nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3h.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3v-{}-{tag}-{seq}", std::process::id()));
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
    let output = Command::new(&binary).output().expect("binary should run");
    assert!(output.status.success(), "the built binary should exit 0");
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

fn build_error(src: &Path) -> String {
    driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect_err("program should not build")
}

/// A forced-linear struct with an observable `drop`, the shape `tests/phase0.rs`
/// uses for the linear core.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | \"drop \" . s Spy> . ;\n";

/// The closure factory every golden below builds on: one `Spy` capture, a body
/// that disposes it. The body prints nothing of its own, so a `drop` golden's
/// output is exactly the capture's disposal.
const MK_DEF: &str = ": mk ( Spy -- owning [ -- ] ) | s | [ s drop ] ;\n";

// -- R4: `drop` on a bare owning closure --------------------------------------

/// Migrated from `tests/phase7_slice3h.rs`'s
/// `dropping_an_owning_closure_is_a_located_rejection`, inverted: `drop` is now
/// a legal consuming use, distinct from `call`. It runs only the synthesized
/// disposer, so the capture is disposed exactly once and the closure body never
/// runs at all.
///
/// This is the monomorphic half of R4's twinned guard: restore `src/check.rs`'s
/// deleted `OwningQuotation` arm in the `"drop"` shuffle and this fails.
#[test]
fn dropping_an_owning_closure_disposes_its_capture_once() {
    let prog = Scratch::write(
        "drop-owning",
        &format!("{SPY_DEF}{MK_DEF}: main ( -- ) 7 Spy mk drop ;\n"),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n");
}

/// The generic-body half of R4's twinned guard, migrated from
/// `dropping_an_owning_closure_in_a_generic_body_is_a_located_rejection`. A
/// generic word cannot *declare* an owning parameter, but it can call a word
/// that returns one, so the value reaches the poly walk's own `"drop"` arm
/// through the body rather than the signature -- a path the monomorphic checker
/// never walks. Restore `src/check/poly.rs`'s deleted arm and this fails while
/// the monomorphic golden above keeps passing.
///
/// `g` must be a real generic word (a `'T: Copy` parameter it actually returns),
/// not a monomorphic body that happens to type-check: the poly walk runs only
/// on a generic body.
#[test]
fn dropping_an_owning_closure_in_a_generic_body_disposes_its_capture_once() {
    let prog = Scratch::write(
        "drop-owning-poly",
        &format!(
            "{SPY_DEF}{MK_DEF}\
             : g ['T: Copy] ( 'T -- 'T ) | x | 7 Spy mk drop x ;\n\
             : main ( -- ) 5 g . ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n5\n");
}

/// R3's headline, and the reason `drop` is not merely a synonym for `call`:
/// they run *different code*. The body prints a line the disposer cannot, and
/// the disposer disposes the capture the body would have disposed itself. Two
/// goldens over one source, branching on `main`'s last word only, so nothing
/// but the consuming use differs between them.
#[test]
fn call_and_drop_run_different_code() {
    let src = |last: &str| {
        format!(
            "{SPY_DEF}\
             : mk ( Spy -- owning [ -- ] ) | s | [ \"body\\n\" . s drop ] ;\n\
             : main ( -- ) 7 Spy mk {last} ;\n"
        )
    };
    let called = Scratch::write("called", &src("call"));
    assert_eq!(build_and_run(called.path()), "body\ndrop 7\n");
    let dropped = Scratch::write("dropped", &src("drop"));
    assert_eq!(build_and_run(dropped.path()), "drop 7\n");
}

/// The null-disposer arm of R3's guarded `emit_drop`, at runtime. A closure
/// with no captures needs no disposer, so its third slot is null and `drop`
/// must branch past the indirect call rather than jump to zero. Every other
/// golden here captures a `Spy`, so all of them take the non-null branch; the
/// null one was pinned only structurally, by `emit_drop`'s unit test. Nothing
/// is printed by the closure itself, so reaching the trailing `ok` *is* the
/// assertion: a mis-emitted guard segfaults instead.
#[test]
fn dropping_a_capture_free_owning_closure_skips_the_null_disposer() {
    let prog = Scratch::write(
        "drop-owning-nullary",
        ": mk ( -- owning [ -- ] ) [ ] ;\n: main ( -- ) mk drop \"ok\\n\" . ;\n",
    );
    assert_eq!(build_and_run(prog.path()), "ok\n");
}

// -- R5/R6: the three admitted storage positions ------------------------------

/// Migrated from `an_owning_quotation_field_is_rejected`, inverted. The struct
/// field carve-out is only half the story: what disposes the field is the
/// container's synthesized destructor, which exists only because R5's
/// `layout_field_is_linear` sees the owning field, and which disposes it only
/// because R5's `field_is_linear` sees it too. Revert either fold alone and
/// this leaks silently -- no diagnostic, no crash, just a missing line.
#[test]
fn an_owning_quotation_field_is_disposed_on_container_drop() {
    let prog = Scratch::write(
        "field",
        &format!(
            "{SPY_DEF}{MK_DEF}\
             type: Box q owning [ -- ] ;\n\
             : main ( -- ) 7 Spy mk Box drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n");
}

/// Migrated from `an_owning_quotation_variant_field_is_rejected`, inverted.
/// Its carve-out is a separate one, not a mirror of the struct-field
/// carve-out: the enum-variant loop had no quotation exception at all before
/// this slice, for either flavour.
#[test]
fn an_owning_quotation_variant_field_is_disposed_on_container_drop() {
    let prog = Scratch::write(
        "variant-field",
        &format!(
            "{SPY_DEF}{MK_DEF}\
             type: E | None | Some q owning [ -- ] ;\n\
             : main ( -- ) 7 Spy mk Some drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n");
}

/// The owned-cell payload, spelled with `^` and `owning` as separate tokens.
/// This spelling already reached the audit before this slice (and was rejected
/// there), so it exercises R6's cell carve-out and nothing of R7's:
/// `split_owning_cell_word`'s empty-remainder branch recurses into
/// `parse_type_expr`, whose `owning` dispatch has existed since P7.S3h.
#[test]
fn an_owning_quotation_cell_payload_is_admitted_spaced() {
    assert_eq!(
        build_and_run(cell_program("spaced", "^ owning [ -- ]").path()),
        "drop 7\n"
    );
}

/// The glued spelling, which is a genuinely different code path: `^` is not a
/// lexer delimiter, so `^owning` arrives as one word and never reaches
/// `parse_type_expr`'s `owning` dispatch at all -- the remainder resolved as an
/// unknown type name until R7. Revert R7's arm in `split_owning_cell_word` and
/// this fails on `unknown type \`owning\`` while the spaced golden above keeps
/// passing, which is what proves the two spellings are two paths rather than
/// one.
#[test]
fn an_owning_quotation_cell_payload_is_admitted_glued() {
    assert_eq!(
        build_and_run(cell_program("glued", "^owning [ -- ]").path()),
        "drop 7\n"
    );
}

/// A closure stored in an owned cell, unwrapped with `^>` and dropped.
/// Parameterized by the payload's *spelling* only, so the two goldens above
/// differ in nothing but the parser path that reads it.
fn cell_program(tag: &str, spelling: &str) -> Scratch {
    Scratch::write(
        tag,
        &format!(
            "{SPY_DEF}{MK_DEF}\
             : boxed ( Spy -- {spelling} ) mk ^ ;\n\
             : main ( -- ) 7 Spy boxed ^> drop ;\n"
        ),
    )
}

/// The cell's own destructor, rather than a user unwrapping the payload first:
/// `drop` on the cell disposes the closure it holds without `^>` ever running.
/// This is the path a container reaches R3's `emit_drop` arm through, so it
/// pins the arm rather than the unwrap.
#[test]
fn an_owning_cell_of_owning_quotation_is_disposed_on_cell_drop() {
    let prog = Scratch::write(
        "cell-drop",
        &format!(
            "{SPY_DEF}{MK_DEF}\
             : boxed ( Spy -- ^ owning [ -- ] ) mk ^ ;\n\
             : main ( -- ) 7 Spy boxed drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 7\n");
}

/// A container holding an owning field *and* an ordinary linear one, dropped
/// once. Both disposal lines appear exactly once each, in field order: the
/// disposer must not double-run, and the container's field glue must not visit
/// the quotation field twice (the failure mode a single-field golden cannot
/// see, since one extra disposal reads as a double free rather than a wrong
/// count).
#[test]
fn an_owning_field_disposes_alongside_its_siblings_exactly_once() {
    let prog = Scratch::write(
        "siblings",
        &format!(
            "{SPY_DEF}{MK_DEF}\
             type: Pair q owning [ -- ] s Spy ;\n\
             : main ( -- ) 1 Spy mk 2 Spy Pair drop ;\n"
        ),
    );
    assert_eq!(build_and_run(prog.path()), "drop 1\ndrop 2\n");
}

// -- The blast-radius guards on R6's carve-outs -------------------------------

/// R6 admits exactly three positions. An array element is not one of them:
/// the audit (`audit_quotation_type_registries`) rejects an owning
/// quotation as an array element because it is a quotation type in an
/// illegal position.  The former linear-array gate is gone (Phase 4
/// admitted linear array elements), but the audit still catches an owning
/// quotation here -- the message is the audit's, which is the point:
/// widening R6 to a fourth position would change it.
#[test]
fn an_array_element_owning_closure_is_still_rejected() {
    let prog = Scratch::write(
        "array-elem",
        "type: Arr xs array[owning [ -- ] 2] ;\n: main ( -- ) ;\n",
    );
    assert_eq!(
        build_error(prog.path()),
        "error: a quotation type `owning [ -- ]` cannot appear as an array element: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7"
    );
}

/// The other position R6 leaves alone. A reference does not own what it points
/// at, so it can never dispose one -- unchanged from P7.S3h, and the audit's
/// reference-referent loop gained no carve-out.
#[test]
fn a_reference_referent_owning_closure_is_still_rejected() {
    let prog = Scratch::write("ref-referent", ": f ( & owning [ -- ] -- ) drop ;\n");
    assert_eq!(
        build_error(prog.path()),
        "error: a quotation type `owning [ -- ]` cannot appear as a reference referent: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7"
    );
}

/// The un-widened half of R6's new cell carve-out, which keys on the *owning*
/// flavour alone. A plain quotation payload has no disposal obligation and no
/// D4 store check behind it, and stays rejected exactly as before.
#[test]
fn an_owning_cell_payload_of_a_plain_quotation_is_still_rejected() {
    let prog = Scratch::write("plain-cell", ": f ( ^ [ -- ] -- ) drop ;\n");
    assert_eq!(
        build_error(prog.path()),
        "error: a quotation type `[ -- ]` cannot appear as an owned-cell payload: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7"
    );
}

// -- R8: the REPL override-epoch obligation, and why it cannot be exercised ---

/// A scripted REPL session, `tests/repl_ux.rs`'s harness: the failure this pair
/// is about is a link error the in-process helpers never see.
fn run_session(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    stdin
        .write_all((lines.join("\n") + "\n").as_bytes())
        .expect("writing stdin should succeed");
    drop(stdin);
    let out = child.wait_with_output().expect("repl should exit cleanly");
    assert!(out.status.success(), "repl exited with {:?}", out.status);
    String::from_utf8(out.stdout).expect("stdout should be utf8")
}

/// R8 wanted `explicit_repl_override_epoch_disposal`: a session holding a user
/// `drop` override, then a later line building an owning closure over a
/// *different* linear struct, stored in a field and dropped, so the disposer's
/// `emit_drop` has to name the capture's destructor at the session-wide
/// override epoch. That golden cannot be written, and this pins why.
///
/// A disposer exists only for a *materialized* closure, and no session line can
/// link one: the code pointer is a non-PIC relocation into the line's own
/// shared object, so the session dies in `ld` before anything runs. Storing the
/// closure in the field is precisely what forces that materialization, so the
/// shape R8 asked for is the shape that cannot link. The epoch obligation is
/// therefore unreachable rather than untested: no session can reach the
/// disposer's `emit_drop` call at all.
///
/// The closure is built and stored on one line, rather than returned from a
/// session-defined factory word: a `: mk ( Wrap -- owning [ -- ] )` definition
/// line materializes on its *own* account and dies before `Box` is ever
/// reached, which is P7.S3h behaviour and would make this a duplicate of the
/// control below. Written this way it discriminates: revert R6's struct-field
/// carve-out (`src/check/audits.rs`) and `Box` is refused outright, so the
/// admission assertion fails and the line never reaches the linker.
///
/// Asserted as the blocked state, not skipped, so it is a tripwire: the day the
/// session-module PIC problem is fixed this fails, and R8's real golden is the
/// session below with `"drop 7"` asserted in place of the link failure. The
/// four assertions are the whole claim -- the container is admitted, a disposer
/// really is minted for the construction site, the disposal still never happens
/// because the blocker is the linker rather than any checker gate this slice
/// controls, and the session survives it.
#[test]
fn explicit_repl_override_epoch_disposal_is_blocked_by_the_repl_link_limit() {
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | \"drop \" . r Res> . ;",
        "type: Wrap r Res ;",
        "type: Box q owning [ -- ] ;",
        "7 Res Wrap | w | [ w drop ] Box drop",
        "1 2 add .",
    ]);
    assert!(
        out.contains("defined type Box"),
        "R6 admits the owning field at the REPL too, so the session gets past the audit: {out}"
    );
    assert!(
        out.contains("__quot0__dispose"),
        "the construction site really does mint a disposer -- the symbol the link step \
         cannot place is R2's, so this session fails one step past the disposer, not \
         before it (the text is `ld`'s relocation warning): {out}"
    );
    assert!(
        !out.contains("drop 7"),
        "the capture is never disposed, because the closure is never built: {out}"
    );
    assert!(
        out.contains("\"cc\" failed"),
        "the blocker is the link step, not a diagnostic -- if this session now links, \
         promote it to R8's real golden and assert `drop 7` instead: {out}"
    );
    assert!(
        out.ends_with("3\nstack: (empty)\n"),
        "a refused line leaves the session usable: {out}"
    );
}

/// The control that keeps the test above from being read as this slice's
/// regression: the same link failure, with no `owning`, no disposer and no
/// third-word write involved -- a plain quotation value in a session line dies
/// identically. P7.S3h's roadmap entry already names this as a standing hazard;
/// this pins it beside the blocked golden so the two are diagnosed together.
#[test]
fn a_plain_quotation_value_hits_the_same_repl_link_limit() {
    let out = run_session(&["type: H f [ i64 -- i64 ] ;", "[ 1 add ] H", "1 2 add ."]);
    assert!(
        out.contains("\"cc\" failed"),
        "a plain quotation value is unlinkable in a session too: {out}"
    );
    assert!(
        out.ends_with("3\nstack: (empty)\n"),
        "a refused line leaves the session usable: {out}"
    );
}
