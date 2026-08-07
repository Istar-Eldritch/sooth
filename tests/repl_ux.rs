//! REPL UX golden session tests: spawn the `repl` binary, pipe a scripted
//! stdin session, and assert on stdout. Same shape as `tests/phase1.rs`.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a scripted REPL session (one input line per element of `lines`) and
/// return the whole captured stdout.
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
    drop(stdin); // close stdin so the REPL sees EOF and exits

    let output = child.wait_with_output().expect("repl should exit cleanly");
    assert!(
        output.status.success(),
        "repl exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// R12: the piped path never buffers a multi-line definition across lines
/// (that is strictly a tty-editor affordance, slice 2). Splitting a `:`
/// definition across two piped lines keeps today's per-line errors, not a
/// joined, successful definition.
#[test]
fn piped_multiline_def_keeps_per_line_errors() {
    let out = run_session(&[": sq ( i64 -- i64 )", "dup * ;"]);
    assert!(
        out.contains("parse error"),
        "expected per-line parse errors, got: {out}"
    );
    assert!(
        !out.contains("defined sq"),
        "piped path must not join lines into a successful definition: {out}"
    );
}
