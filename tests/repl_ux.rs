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

/// R19: redefining a word must not leave a stale generation visible in
/// `:words` -- there is exactly one entry, at the new signature.
#[test]
fn repl_words_shows_redefined_word_at_new_generation() {
    let out = run_session(&[
        ": sq ( i64 -- i64 ) dup * ;",
        ": sq ( i64 i64 -- i64 ) * ;",
        ":words",
    ]);
    let sq_lines: Vec<&str> = out.lines().filter(|l| l.starts_with("sq ")).collect();
    assert_eq!(
        sq_lines,
        vec!["sq ( i64 i64 -- i64 )"],
        "expected exactly one, current-generation `sq` entry, got: {out}"
    );
}

/// R19: an imported word must list under the spelling the user typed
/// (`m::inc`), not the internal import-epoch-mangled symbol (`m::inc__import0`).
#[test]
fn repl_words_shows_user_facing_name_for_imported_word() {
    let dir = std::env::temp_dir().join(format!("sooth-replux-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("m.sth");
    std::fs::write(&lib, ": inc ( i64 -- i64 ) 1 + ;\nexport: inc ;\n").unwrap();

    let out = run_session(&[&format!("import: m \"{}\" ;", lib.display()), ":words"]);

    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.contains("m::inc ( i64 -- i64 )"),
        "`:words` should list the user-facing spelling `m::inc`, got: {out}"
    );
    assert!(
        !out.contains("__import"),
        "`:words` must not leak the internal import-epoch-mangled spelling, got: {out}"
    );
}

/// R19/R23: a polymorphic word (kept out of `self.env`, R3) is not invisible
/// to `:words` just because it has no concrete instantiation yet.
#[test]
fn repl_words_lists_polymorphic_word() {
    let out = run_session(&[": alen ( ['T 'N] -- ) drop ;", ":words"]);
    assert!(
        out.contains("alen ( ['T 'N] -- )"),
        "`:words` should list the polymorphic word `alen` with its poly signature, got: {out}"
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
///
/// Deliberately uses a *linear* type with a printing `drop` override
/// (mirroring `repl_drop_overload_still_runs_on_a_later_line` in
/// `tests/phase3_resources.rs`), not a plain `i64`: `dispose_residual`
/// early-returns doing nothing when nothing on the stack is linear, so a
/// non-linear probe would still pass this test even if the dispose call were
/// deleted from `Session::clear` entirely, proving only the reset half of
/// D4/R22 and none of the dispose half.
#[test]
fn repl_clear_disposes_then_resets() {
    let out = run_session(&[
        "type: Res n i64 ;",
        ": drop ( Res -- ) | r | r Res>n . ;",
        "7 Res",
        ":clear",
        ":stack",
        ": sq ( i64 -- i64 ) dup * ;",
        "3 sq",
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
            "defined sq",
            "stack: 9",
        ],
        "expected the `Res` destructor's `7` to print during `:clear`, then a clean reset, got: {out}"
    );
}

/// Slice 12 (R-B2): the REPL runs the same missing-`inline`-on-a-`~`-
/// parameter gate (`check_inline_quotation_requires_inline`) native `check`
/// runs in its pre-pass. Without it a `~[ ... ]` parameter without `inline`
/// is accepted at definition and only panics later, at the call site
/// (`checked user word exists`), instead of failing at `:` with the exact E2
/// text -- so this asserts the exact line, not just that the session errors.
#[test]
fn repl_missing_inline_on_tilde_parameter_is_error() {
    let out = run_session(&[
        ": twice ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;",
        "5 [ 1 + ] twice",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "error: word `twice` declares an inline-quotation parameter `~[ i64 -- i64 ]` but is not `inline`; a `~[ ... ]` quotation can only be spliced, so the word must declare `inline` (line 1, col 3)",
            "error: unknown word `twice`",
        ],
        "expected the definition to be rejected with the exact E2 text (and the later call site to see no `twice`, not panic on a retained-but-invalid word): {out}"
    );
}

/// Regression (slice 6c): once a session can retain a combinator
/// (`self.combinators`), a struct's `drop` override calling one reopens the
/// same gap the native build fixed separately -- `synthesize_aggregate_
/// destructors`' four REPL call sites (`compile_drop_overload`, `eval_def`,
/// `run_terms`) must each be handed `combinator_bodies(&self.combinators)`,
/// not an empty map, or lowering panics ("checked user word exists") instead
/// of splicing the call. `.`'s printed `3` only reaches the real process
/// stdout this subprocess-spawning `run_session` captures, not the in-process
/// `repl_error` helper elsewhere in this suite, so this golden must live
/// here. Exercises three of the four call sites in one session: defining the
/// override (`compile_drop_overload`), defining an unrelated later word
/// (`eval_def`, which also re-synthesizes every session destructor), and the
/// bare line that triggers the drop (`run_terms`).
#[test]
fn repl_combinator_called_from_drop_override_body_runs_without_panicking() {
    let out = run_session(&[
        ": twice inline ( i64 [ i64 -- i64 ] -- i64 ) | q | q call q call ;",
        "type: Bx v i64 ;",
        ": drop ( Bx -- ) Bx>v [ 1 + ] twice . ;",
        ": helper ( i64 -- i64 ) 1 + ;",
        "1 Bx drop",
    ]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "defined twice",
            "defined type Bx",
            "defined drop for Bx",
            "defined helper",
            "3",
            "stack: (empty)",
        ],
        "expected the drop override's `twice`-doubled `3` to print, not a panic: {out}"
    );
}
