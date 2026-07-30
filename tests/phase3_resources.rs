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
fn repl_drop_overload_still_runs_on_a_later_line() {
    // Criterion 17/R11.1: the declaring line's `WordDef` dies with that line,
    // but the destructor is re-synthesized into every subsequent line's own
    // module, so the override has to be retained in the session to survive.
    // The body is extern-free on purpose: the REPL still cannot evaluate an
    // `extern:` declaration at all, so the dogfood's own body cannot be used
    // here.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res>n . ;",
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
        ": drop ( Res -- ) | r | r Res>n . ;",
        "7 Res",
        "drop",
        ": drop ( Res -- ) | r | r Res>n 100 + . ;",
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
fn repl_quit_disposes_a_residual_resource_through_its_overload() {
    // The `:quit` LIFO-disposal path derives linearity from the session's
    // current structs, so an overridden struct left on the carried stack is
    // disposed by the user's own body.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res>n . ;",
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
fn repl_resource_field_is_disposed_through_the_overload() {
    // R7's ordinary composition, at the REPL: an enclosing struct declared
    // *after* the override still disposes its resource field by calling that
    // resource's own destructor, not by inlining the field glue.
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res>n . ;",
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
