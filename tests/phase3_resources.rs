//! Goldens for Phase 3 Slice 8b: a user `drop` body as a struct's destructor.
//!
//! Kept out of `tests/phase0.rs` (asserted never to change from this work's
//! base commit) and out of `tests/phase1.rs`, mirroring how slice 8a's
//! goldens live in `tests/phase3_strings.rs`.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a scripted REPL session (one input line per element of `lines`) and
/// return the whole captured stdout, mirroring `tests/phase1.rs`'s harness.
fn run_session(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let script = lines.join("\n") + "\n";
    stdin
        .write_all(script.as_bytes())
        .expect("writing stdin should succeed");
    drop(stdin);
    let output = child.wait_with_output().expect("repl should exit cleanly");
    assert!(
        output.status.success(),
        "repl exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

#[test]
fn slice8b_dogfood_compiles_and_runs() {
    // Criterion 19: `examples/resources.sth` opens, reads and closes a file,
    // with `close` reached only through `File`'s own `drop` overload. It reads
    // a dedicated 3-byte fixture rather than a project document so the golden
    // is deterministic; that makes it the first example to open a file *at
    // run time*, so the working directory is pinned explicitly (every other
    // golden uses its relative path as compiler input only).
    let root = env!("CARGO_MANIFEST_DIR");
    let binary = sooth::driver::build(&std::path::Path::new(root).join("examples/resources.sth"))
        .expect("build should succeed");
    let output = Command::new(&binary)
        .current_dir(root)
        .output()
        .expect("binary should run");
    assert_eq!(output.status.code(), Some(0), "dogfood should exit 0");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "3\n"
    );
}

#[test]
fn repl_dispose_of_session_defined_override_is_unaffected() {
    // Slice 8b, R8: the `drop` import-visibility gate is native-only
    // (`modules: None` on the REPL path), so disposing a session-defined
    // override is byte-for-byte what it was before this slice: the destructor
    // runs (prints `7`) and the residual stack line is exactly empty. Asserting
    // the whole line list, not a `contains`, since the session reprints the
    // entire residual stack every line.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res> . ;",
        "7 Res",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined drop for Res",
            "stack: <Res>",
            "7",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_drop_overload_still_runs_on_a_later_line() {
    // Criterion 17/R11.1: the declaring line's `WordDef` dies with that line,
    // but the destructor is re-synthesized into every subsequent line's own
    // module, so the override has to be retained in the session to survive.
    // The body is extern-free on purpose: the REPL still cannot evaluate an
    // `extern:` declaration at all, so the dogfood's own body cannot be used
    // here.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res> . ;",
        "7 Res",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined drop for Res",
            "stack: <Res>",
            "7",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_redefined_drop_overload_runs_the_new_body() {
    // Criterion 22's behavioural half: two generations define one struct's
    // destructor with two different bodies. Under the unsuffixed symbol both
    // `.so`s would export the same global under `RTLD_GLOBAL` and the first
    // one loaded would keep winning, so a redefinition would silently keep
    // running the old body.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res> . ;",
        "7 Res",
        "drop",
        ": drop ( Res -- ) | r | r Res> 100 add . ;",
        "7 Res",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined drop for Res",
            "stack: <Res>",
            "7",
            "stack: (empty)",
            "defined drop for Res",
            "stack: <Res>",
            "107",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_redefining_an_overrides_callee_leaves_the_override_alone() {
    // R11.3 (code review, phase 4): every later line used to re-lower the
    // retained override body against *that* line's env, so redefining a word
    // the body calls at a different arity read the new arity against the old
    // body's stack and panicked in lowering. The override is lowered once, on
    // its own line, and its destructor symbol is pinned to that line's epoch,
    // so a later redefinition of a callee cannot reach it: `drop` still prints
    // 7 through the original `helper`, the same snapshot an ordinary word's
    // body already gets (its callees bind the generations visible when it was
    // defined).
    let out = run_session(&[
        "type: Res n i64 ;",
        ": helper ( i64 -- ) . ;",
        ": drop ( Res -- ) | r | r Res> helper ;",
        ": helper ( i64 i64 -- ) add . ;",
        "7 Res",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined helper",
            "defined drop for Res",
            "defined helper",
            "stack: <Res>",
            "7",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_declaring_a_second_override_leaves_the_first_alone() {
    // R11.3: a `: drop` line bumps the session epoch, which moves every
    // *other* destructor symbol -- but an override's own symbol is pinned to
    // the epoch it was defined at, so declaring `B`'s override neither
    // re-emits nor re-lowers `A`'s. Without the pinning this line, which does
    // not even mention `A`, hits the same stale-env re-lowering panic.
    let out = run_session(&[
        "type: A n i64 ;",
        "type: B n i64 ;",
        ": helper ( i64 -- ) . ;",
        ": drop ( A -- ) | a | a A> helper ;",
        ": helper ( i64 i64 -- ) add . ;",
        ": drop ( B -- ) | b | b B> . ;",
        "1 A",
        "drop",
        "2 B",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type A",
            "defined type B",
            "defined helper",
            "defined drop for A",
            "defined helper",
            "defined drop for B",
            "stack: <A>",
            "1",
            "stack: (empty)",
            "stack: <B>",
            "2",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_quit_disposes_a_residual_resource_through_its_overload() {
    // The `:quit` LIFO-disposal path derives linearity from the session's
    // current structs, so an overridden struct left on the carried stack is
    // disposed by the user's own body.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res> . ;",
        "7 Res",
        ":quit",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined drop for Res",
            "stack: <Res>",
            "7"
        ]
    );
}

#[test]
fn repl_redefining_drop_overload_refreshes_a_composing_structs_glue() {
    // R11.2 (code review, phase 4): `Holder`'s own destructor never carries
    // an override, but it `Call`s `Res`'s destructor symbol inside its own
    // body -- so redefining `Res`'s override must refresh `Holder`'s glue
    // too, or `Holder`'s frozen first-loaded destructor keeps calling the
    // stale symbol forever under `RTLD_GLOBAL`.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res> . ;",
        "type: Holder r Res ;",
        "7 Res Holder",
        "drop",
        ": drop ( Res -- ) | r | r Res> 100 add . ;",
        "7 Res Holder",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined drop for Res",
            "defined type Holder",
            "stack: <Holder>",
            "7",
            "stack: (empty)",
            "defined drop for Res",
            "stack: <Holder>",
            "107",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_composing_structs_glue_is_correct_when_override_postdates_it() {
    // R11.2 (code review, phase 4): `Holder`'s destructor is first compiled
    // (and, under `RTLD_GLOBAL`, permanently pinned) *before* `Res` ever gets
    // an override, calling generic field glue. Once the override is defined,
    // `Holder`'s glue must be recompiled under a fresh symbol that calls the
    // override, not left running the pre-override body forever.
    let out = run_session(&[
        "type: Spy tag i64 ;",
        ": drop ( Spy -- )  | s | \"drop \" . s Spy> . ;",
        "type: Res n Spy ;",
        "type: Holder r Res ;",
        "1 Spy Res Holder",
        "drop",
        ": drop ( Res -- ) | r | 42 . r Res> drop ;",
        "1 Spy Res Holder",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Spy",
            "defined drop for Spy",
            "defined type Res",
            "defined type Holder",
            "stack: <Holder>",
            "drop 1",
            "stack: (empty)",
            "defined drop for Res",
            "stack: <Holder>",
            "42",
            "drop 1",
            "stack: (empty)",
        ]
    );
}

#[test]
fn repl_resource_field_is_disposed_through_the_overload() {
    // R7's ordinary composition, at the REPL: an enclosing struct declared
    // *after* the override still disposes its resource field by calling that
    // resource's own destructor, not by inlining the field glue.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res> . ;",
        "type: Holder r Res ;",
        "7 Res Holder",
        "drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined type Res",
            "defined drop for Res",
            "defined type Holder",
            "stack: <Holder>",
            "7",
            "stack: (empty)",
        ]
    );
}
