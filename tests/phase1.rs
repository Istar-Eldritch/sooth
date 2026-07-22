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
    let out = run_session(&[": sq ( i64 -- i64 ) | n | n n * ;", "5 sq"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sq", "stack: 25"]);
}

#[test]
fn stack_persists_across_lines() {
    let out = run_session(&[": sq ( i64 -- i64 ) | n | n n * ;", "5", "sq", "1 +"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec!["defined sq", "stack: 5", "stack: 25", "stack: 26"]
    );
}

#[test]
fn redefinition_takes_effect_for_later_lines() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n * ;",
        "3 sq",
        ": sq ( i64 -- i64 ) | n | n n n * * ;",
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
        lines[1].contains("unknown word") && lines[1].contains("unknown-word"),
        "expected an unknown-word diagnostic naming `unknown-word`: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 6");
}

#[test]
fn type_error_line_reports_and_session_survives() {
    let out = run_session(&["5", "true 1 +", "1 +"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "stack: 5");
    assert!(
        lines[1].contains("expected `i64`") && lines[1].contains("found `bool`"),
        "expected a type-mismatch diagnostic: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 6");
}

#[test]
fn failed_redefinition_keeps_old_generation_resident() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n * ;",
        "3 sq",
        ": sq ( i64 -- i64 ) dup dup * ;",
        "3 sq",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 6, "unexpected output:\n{out}");
    assert_eq!(lines[0], "defined sq");
    assert_eq!(lines[1], "stack: 9");
    assert!(
        lines[2].contains("stack effect mismatch in `sq`"),
        "expected a check error for the bad redefinition: {}",
        lines[2]
    );
    assert!(lines[3].contains("body leaves 2 values"));
    assert!(lines[3].contains("declares 1 outputs"));
    // The failed redefinition never committed: `sq` still resolves to the
    // original generation (`n n *`), and the stack from the first `3 sq` is
    // untouched, so the second `3 sq` appends its own 9.
    assert_eq!(lines[5], "stack: 9 9");
}

#[test]
fn sign_definable_and_callable_in_repl() {
    let out = run_session(&[
        ": sign ( i64 -- i64 ) 0 > if 1 else 0 then ;",
        "-7 sign",
        "7 sign",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["defined sign", "stack: 0", "stack: 0 1"]);
}

#[test]
fn bool_residual_displays_as_zero_or_one() {
    let out = run_session(&["true", "false"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["stack: 1", "stack: 1 0"]);
}

#[test]
fn calculator_session_dogfood() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) | n | n n * ;",
        ": neg ( i64 -- i64 ) 0 swap - ;",
        "3 sq",
        "neg",
        "10 +",
        "2 *",
        ".",
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
            "2",
            "stack: (empty)",
        ]
    );
}

/// Guards the flush-before-call discipline the spec flags as a determinism
/// risk: the host's stdout buffer and the loaded code's C stdio buffer must
/// both be flushed so `.` output lands before the next `stack:` line, in
/// program order, on every run.
#[test]
fn dot_output_interleaves_before_stack() {
    let out = run_session(&["5 .", "2 3 + ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["5", "stack: (empty)", "5", "stack: (empty)"]);
}
