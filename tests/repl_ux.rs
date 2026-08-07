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
    let error_lines: Vec<&str> = out.lines().filter(|l| l.contains("parse error")).collect();
    assert_eq!(
        error_lines.len(),
        2,
        "expected one parse error per piped line (no cross-line joining), got: {out}"
    );
    assert!(
        !out.contains("defined sq"),
        "piped path must not join lines into a successful definition: {out}"
    );
}

/// #22: `:help` lists the meta-commands.
#[test]
fn repl_help_lists_meta_commands() {
    let out = run_session(&[":help"]);
    for cmd in [":help", ":words", ":type", ":stack", ":clear", ":quit"] {
        assert!(out.contains(cmd), "`:help` output missing `{cmd}`: {out}");
    }
}

/// #23: `:words` lists defined words with their declared signatures.
#[test]
fn repl_words_lists_words_with_signatures() {
    let out = run_session(&[": sq ( i64 -- i64 ) dup * ;", ":words"]);
    assert!(
        out.contains("sq ( i64 -- i64 )"),
        "`:words` should list `sq` with its signature, got: {out}"
    );
}

/// #24: `:type` prints the resulting stack effect and executes nothing.
#[test]
fn repl_type_prints_effect_without_executing() {
    let out = run_session(&[":type 1 2 +", ":stack"]);
    assert!(
        out.contains("( -- i64 )"),
        "`:type` should print the checked effect, got: {out}"
    );
    assert!(
        out.contains("stack: (empty)"),
        "`:type` must not mutate the residual stack, got: {out}"
    );
}

/// #26: `:stack` prints the residual stack without pushing or consuming.
#[test]
fn repl_stack_prints_without_mutating() {
    let out = run_session(&["1 2", ":stack", "+"]);
    let stack_lines: Vec<&str> = out.lines().filter(|l| l.starts_with("stack:")).collect();
    assert_eq!(
        stack_lines.len(),
        3,
        "expected three stack lines, got: {out}"
    );
    assert_eq!(stack_lines[0], "stack: 1 2");
    assert_eq!(stack_lines[1], "stack: 1 2");
    assert_eq!(stack_lines[2], "stack: 3");
}

/// #27: `:clear` disposes the residual stack (runs destructors) then resets
/// the session -- a redefinition after `:clear` behaves like a fresh session.
#[test]
fn repl_clear_disposes_then_resets() {
    let out = run_session(&[
        "1 2",
        ":clear",
        ":stack",
        ": sq ( i64 -- i64 ) dup * ;",
        "3 sq",
    ]);
    assert!(
        out.contains("stack: (empty)"),
        "`:clear` should reset the stack, got: {out}"
    );
    assert!(out.contains("defined sq"), "got: {out}");
    assert!(
        out.contains("stack: 9"),
        "session should work normally after `:clear`, got: {out}"
    );
}
