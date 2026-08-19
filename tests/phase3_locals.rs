//! Phase 3 Slice 5 goldens: `| names |` is a term, legal at any point in a
//! body, with its extent running to the end of the enclosing block.

use std::io::Write;
use std::process::{Command, Stdio};

use sooth::{check, lexer, parser};

mod common;

/// Run a scripted REPL session (one input line per element of `lines`) and
/// return the whole captured stdout, mirroring `tests/phase1.rs`'s helper.
fn run_session(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
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

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    parser::parse(&tokens).expect_err("parsing should fail")
}

/// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
/// primitive in Slice 8c: an ordinary one-field struct with a `drop`
/// overload, so it is linear for the same reason any resource is, not by
/// any compiler-known bit. Two lines, so every line number in a source
/// string it is prepended to shifts up by 2.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";

#[test]
fn mid_body_binding_consumes_from_the_stack() {
    // Criterion 1: `| a b |` pops two values where it appears, leaving the `1`
    // beneath them on the stack for the term after the binding's users.
    let (stdout, code) = run_src(
        "mid-body-binding-consumes",
        ": main ( -- )\n  1 2 3\n  | a b |\n  a b add .\n  . ;\n",
    );
    assert_eq!(stdout, "5\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn mid_body_binding_leftmost_name_takes_deepest_value() {
    // Criterion 2: the entry form's rule holds mid-body too.
    let (stdout, code) = run_src(
        "mid-body-binding-leftmost",
        ": main ( -- )\n  10 20\n  | a b |\n  a . b . ;\n",
    );
    assert_eq!(stdout, "10\n20\n");
    assert_eq!(code, 0);
}

#[test]
fn local_bound_in_if_arm_is_not_visible_after_end() {
    // Criterion 3: the arm is the extent (R2), so `x` past `end` is not a name
    // at all: it resolves as a word, and there is no such word.
    let err = check_error(": w ( bool -- i64 )\n  ~[ 7 | x | x ] ~[ 0 ] if\n  x ;\n");
    assert!(err.contains("unknown word"), "unexpected message: {err}");
    assert!(err.contains("`x`"), "unexpected message: {err}");
}

#[test]
fn name_bound_in_one_arm_can_be_rebound_in_sibling_arm() {
    // Criterion 4: the first `v`'s extent ended at `else`, so the second is a
    // fresh binding, not the re-binding R4 rejects. Proves IR-side teardown
    // happens (each arm's locals truncate at its exit).
    let (stdout, code) = run_src(
        "sibling-arm-rebind",
        ": pick ( bool -- i64 )\n  ~[ 1 | v | v 10 mul ] ~[ 2 | v | v 100 mul ] if ;\n\n\
: main ( -- )\n  true pick .\n  false pick . ;\n",
    );
    assert_eq!(stdout, "10\n200\n");
    assert_eq!(code, 0);
}

#[test]
fn rebinding_a_name_in_scope_is_error() {
    // Criterion 5: forced for a linear value (the first binding would become
    // unreachable and could never be consumed), applied uniformly.
    let err = check_error(": w ( i64 -- i64 )\n  | a |\n  5 | a |\n  a ;\n");
    assert!(err.contains("already bound"), "unexpected message: {err}");
    assert!(err.contains("`a`"), "unexpected message: {err}");
    assert!(err.contains("line 3"), "the error should locate it: {err}");
}

#[test]
fn binding_more_values_than_frame_holds_is_error() {
    // Criterion 6: the existing needs-N-holds-M shape, naming the binding.
    let err = check_error(": w ( i64 -- i64 )\n  5 | a b c |\n  a b c add add ;\n");
    assert!(
        err.contains("`| a b c |` needs 3 values, but the stack holds 2"),
        "unexpected message: {err}"
    );
    assert!(err.contains("line 2"), "the error should locate it: {err}");
}

#[test]
fn binding_cannot_reach_beneath_declared_inputs() {
    // Criterion 7: `inner`'s frame is its one declared input, so its binding
    // cannot reach beneath it, regardless of what a caller might have left on
    // the stack (checking is per-word, so no caller is needed to prove this).
    let err = check_error(": inner ( i64 -- i64 )\n  1 drop | a b |\n  a b add ;\n");
    assert!(
        err.contains("`| a b |` needs 2 values, but the stack holds 1"),
        "unexpected message: {err}"
    );
    assert!(err.contains("line 2"), "the error should locate it: {err}");
}

#[test]
fn entry_binding_keeps_its_declared_input_diagnostic() {
    // Criterion 8: the entry position is the one place the declared effect is
    // the frame, so it keeps the message that cites it (R3) instead of
    // degrading to the generic underflow.
    let err = check_error(": w ( i64 -- i64 ) | a b | a ;");
    assert!(
        err.contains("locals bind 2 value(s), but only 1 input(s) are declared"),
        "unexpected message: {err}"
    );
}

#[test]
fn unconsumed_linear_local_errors_at_block_end() {
    // Criterion 9: the firing site is the arm's terminator, and the message
    // names it, so the fix ("consume it before then") has a location. Slice
    // 10c: an arm is a quotation literal now, so the terminator it names is
    // that literal's own `]`, not the deleted `else`/`end` keywords.
    // `SPY_DEF` is two lines, so `w`'s own line 3 lands on line 5.
    let at_then = check_error(&format!(
        "{SPY_DEF}: w ( bool -- )\n  ~[ 7 Spy | s | 0 .\n  ] ~[ 0 . ] if ;\n\
: main ( -- ) true w ;\n"
    ));
    assert!(
        at_then.contains("linear value `s` is never consumed"),
        "unexpected message: {at_then}"
    );
    assert!(
        at_then.contains("scope ends at the `branch arm` on line 4, col 3"),
        "unexpected message: {at_then}"
    );

    let at_else = check_error(&format!(
        "{SPY_DEF}: w ( bool -- )\n  ~[ 0 . ] ~[\n  7 Spy | s | 0 .\n  ] if ;\n\
: main ( -- ) true w ;\n"
    ));
    assert!(
        at_else.contains("scope ends at the `branch arm` on line 4, col 12"),
        "unexpected message: {at_else}"
    );
}

#[test]
fn linear_local_bound_in_arm_and_moved_on_one_nested_path_is_error() {
    // R6: `s` is bound inside the outer arm (not the word body), so its extent
    // is closed by that arm's `leave_block`, not the word-end check. A nested
    // `if` consumes it on one path only, joining to `MaybeMoved`; the outer
    // arm's own end must still catch it, with the `every_path` wording and
    // the outer arm's terminator, not the word-end message.
    // `SPY_DEF` is two lines, so the outer arm opening on `w`'s own line 2
    // lands on line 4.
    let err = check_error(&format!(
        "{SPY_DEF}: w ( bool bool -- )\n  ~[\n    7 Spy | s |\n    ~[ s drop ] ~[ 1 . ] if\n  ] ~[ drop 0 . ] if ;\n: main ( -- ) true true w ;\n"
    ));
    assert!(
        err.contains("is not consumed on every path"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("scope ends at the `branch arm` on line 4, col 3"),
        "unexpected message: {err}"
    );
}

#[test]
fn linear_local_bound_and_consumed_in_arm_is_accepted() {
    // Criterion 10: R6 adds a place where forgetting is caught, not a place
    // where something is dropped for you; disposing it in the arm is fine, and
    // the spy's destructor proves it ran exactly once.
    let (stdout, code) = run_src(
        "linear-local-consumed-in-arm",
        &format!(
            "{SPY_DEF}: w ( bool -- )\n  ~[ 7 Spy | s | s drop\n  ] ~[ 0 . ] if ;\n\n\
: main ( -- )\n  true w\n  false w ;\n"
        ),
    );
    assert_eq!(stdout, "drop 7\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn empty_binding_with_no_names_is_error() {
    // Criterion 11: a stray pipe pair cannot silently mean nothing (R1).
    let err = parse_error(": w ( -- )\n  | | ;\n");
    assert!(err.contains("binds nothing"), "unexpected message: {err}");
    assert!(
        err.contains("line 2, col 3"),
        "the error should locate it: {err}"
    );
}

#[test]
fn mid_body_binding_in_eliminator_arm_binds() {
    // Criterion 15: an arm body may bind partway through, not only at its
    // leading `|`. `Circle`'s arm binds `r` after a `dup`, mid-body.
    let (stdout, code) = run_src(
        "mid-body-binding-in-eliminator-arm",
        "type: Shape | Circle r f64 | Rect w f64 h f64 ;\n\
: area ( Shape -- f64 )\n  ~[ ( Circle )\n  Circle>\n  dup\n  | r |\n  r mul ]\n  ~[ ( Rect )\n  Rect>\n  | w h |\n  w h mul ]\n  Shape? ;\n\n\
: main ( -- )\n  2.0 Circle area .\n  3.0 4.0 Rect area . ;\n",
    );
    assert_eq!(stdout, "4\n12\n");
    assert_eq!(code, 0);
}

#[test]
fn repl_line_binds_a_local() {
    // Criterion 17: a REPL line binds a local and uses it within the line.
    let out = run_session(&["5 | a | a a mul ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["25", "stack: (empty)"]);
}

#[test]
fn repl_line_binding_reaches_earlier_line_values() {
    // Criterion 18 (R7/D6): the frame floor at a REPL line is the session
    // stack depth, so a binding may consume values an earlier line left.
    let out = run_session(&["1 2 3", "| a b | a b add ."]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines, vec!["stack: 1 2 3", "5", "stack: 1"]);
}

#[test]
fn repl_line_binding_more_than_the_session_stack_holds_is_error() {
    // R5: the REPL frame floor is the session stack depth, not a declared
    // input list, so binding more than that depth holds is the same located
    // underflow shape as anywhere else, naming the REPL's stack rather than a
    // word's declared effect.
    let out = run_session(&["| a b |"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 1, "unexpected output:\n{out}");
    assert!(
        lines[0].contains("stack underflow: needs 2 values, but the stack holds 0"),
        "unexpected message: {}",
        lines[0]
    );
}

#[test]
fn failed_repl_line_after_binding_leaves_stack_intact() {
    // Criterion 19: existing REPL transactionality (a failing line never
    // commits) still holds once the line binds a name before failing.
    let out = run_session(&["1 2 3", "| a b | a b add unknown-word", "1 2 3"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "unexpected output:\n{out}");
    assert_eq!(lines[0], "stack: 1 2 3");
    assert!(
        lines[1].contains("unknown word") && lines[1].contains("unknown-word"),
        "unexpected message: {}",
        lines[1]
    );
    assert_eq!(lines[2], "stack: 1 2 3 1 2 3");
}

#[test]
fn repl_line_locals_do_not_survive_to_next_line() {
    // Criterion 20 (D7): a line's locals are scoped to the line; the next
    // line sees no such name, only the session stack it left behind.
    let out = run_session(&["1 2 3", "| a b |", "a"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "unexpected output:\n{out}");
    assert_eq!(lines[0], "stack: 1 2 3");
    assert_eq!(lines[1], "stack: 1");
    assert!(
        lines[2].contains("unknown word") && lines[2].contains("`a`"),
        "unexpected message: {}",
        lines[2]
    );
}

#[test]
fn mid_body_binding_in_self_tail_recursive_word_loops_correctly() {
    // Criterion 21 (R11): a mid-body binding inside a self-tail-recursive
    // `if` arm loops correctly across 100,000 back-edges. Naming both the
    // accumulator and the counter mid-body lets the arm recompute both and
    // tail-call with them in the right order, without ever holding the
    // bound `acc`/`n` live across the back-edge (their extent ends at the
    // arm's `sum-to` tail call, which sits at the terminator). The IR-level
    // structure (no extra header phi) is covered by
    // `lower_mid_body_binding_adds_no_header_phi` in `src/ir.rs`; this is
    // the runtime golden that the loop still computes the right answer.
    let (stdout, code) = run_src(
        "mid-body-binding-self-tail",
        ": sum-to ( i64 i64 -- i64 )\n\
  dup 0 gt ~[\n\
    | acc n |\n\
    acc n add\n\
    n 1 sub\n\
    sum-to\n\
  ] ~[\n\
    drop\n\
  ] if ;\n\
: main ( -- )\n\
  0 100000 sum-to . ;\n",
    );
    assert_eq!(stdout, "5000050000\n");
    assert_eq!(code, 0);
}

#[test]
fn vm_with_mid_body_binding_matches_previous_output() {
    // Criterion 23: `examples/vm.sth`'s `run` word (Phase 3 Slice 5) names
    // the first `vm-pop` result mid-body in its `Add`/`Sub`/`Mul`/`Store`
    // arms instead of shuffling it into position with `swap`/`over`/
    // `rot`. The rewrite must be output-preserving: same sum-1..100_000
    // bytecode program, same `5000050000` result as before the rewrite.
    let source = std::fs::read_to_string("examples/vm.sth").expect("read vm.sth");
    let code: String = source
        .lines()
        .map(|l| l.split('\\').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        code.matches("| b |").count(),
        3,
        "the Add/Sub/Mul arms should each still bind the popped operand"
    );
    assert!(
        code.contains("| v x |"),
        "the Store arm should still bind both operands"
    );
    let binary = common::build_example("examples/vm.sth");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "5000050000\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
