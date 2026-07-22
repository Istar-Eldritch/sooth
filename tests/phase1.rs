//! Phase 1 golden session tests: spawn the `repl` binary, pipe a scripted
//! stdin session, and assert on stdout. Each test is one exit criterion.

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

#[test]
fn define_then_call_across_lines() {
    let out = run_session(&[": sq ( int -- int ) | n | n n * ;", "5 sq"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sq", "stack: 25"]);
}

#[test]
fn stack_persists_across_lines() {
    let out = run_session(&[": sq ( int -- int ) | n | n n * ;", "5", "sq", "1 +"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sq", "stack: 5", "stack: 25", "stack: 26"]
    );
}

#[test]
fn redefinition_takes_effect_for_later_lines() {
    let out = run_session(&[
        ": sq ( int -- int ) | n | n n * ;",
        "3 sq",
        ": sq ( int -- int ) | n | n n n * * ;",
        "3 sq",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sq", "stack: 9", "defined sq", "stack: 9 27"]
    );
}

#[test]
fn bad_line_reports_and_session_survives() {
    let out = run_session(&["5", "unknown-word", "1 +"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "stack: 5");
    assert!(
        lines[1].starts_with("error"),
        "unknown word should report a diagnostic: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 6");
}

#[test]
fn calculator_session_dogfood() {
    let out = run_session(&[
        ": sq ( int -- int ) | n | n n * ;",
        ": neg ( int -- int ) 0 swap - ;",
        "3 sq",
        "neg",
        "10 +",
        "2 *",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined sq",
            "defined neg",
            "stack: 9",
            "stack: -9",
            "stack: 1",
            "stack: 2",
        ]
    );
}
