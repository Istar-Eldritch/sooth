//! Goldens for Phase 3 Slice 8a: `str`/`cstr` and their literals.
//!
//! Kept out of `tests/phase0.rs` deliberately: that file is asserted never to
//! change from this work's base commit, so a new golden belongs somewhere the
//! addition-only check has nothing to reason about.

use std::io::Write;
use std::process::{Command, Stdio};

use sooth::ir::{lower, Instr};
use sooth::{check, lexer, test_support};

mod common;

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

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
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

#[test]
fn len_of_a_str_emits_no_call_native() {
    // Criterion 6/R8: `len` on a `str` is the carried length, read straight
    // out of the descriptor with no `Instr::Call` anywhere in the lowered
    // function, and the native binary prints the right value.
    let src = ": main ( -- )\n  \"hello\" len . ;";
    let tokens = lexer::lex(src).unwrap();
    let mut module = test_support::parse_with_core(&tokens).unwrap();
    check::check(&mut module).unwrap();
    let ir = lower(&module).unwrap();
    let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
    let calls = main
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Call(..)))
        .count();
    assert_eq!(calls, 0, "len should emit no call");

    let (stdout, code) = run_src("len-no-call", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "5\n");
}

#[test]
fn str_length_and_foreign_strlen_agree_native() {
    // Criterion 7/R6: the literal's terminator is emitted but not counted in
    // the carried length, so Sooth's `len` and C's `strlen` (which scans to
    // that same terminator) agree.
    let src = concat!(
        "extern: strlen ( cstr -- usize ) \"strlen\" ;\n\n",
        ": main ( -- )\n",
        "  \"hello, sooth\" | s |\n",
        "  s len .\n",
        "  s cstr strlen . ;"
    );
    let (stdout, code) = run_src("strlen-agrees", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "12\n12\n");
}

#[test]
fn foreign_call_writes_through_a_mutable_reference_native() {
    // Criterion 14: a reference crosses the C boundary as an input, and the
    // callee's write through it (libc's `memset`, writing one byte) is
    // observable back in Sooth once the call returns.
    let src = concat!(
        "extern: memset ( &!u8 i64 usize -- ) \"memset\" ;\n\n",
        ": main ( -- )\n",
        "  0 >u8 4 fill | a |\n",
        "  &!a 0 &!> 65 >i64 1 >usize memset\n",
        "  &a 0 &> @ . ;"
    );
    let (stdout, code) = run_src("memset-write", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "65\n");
}

#[test]
fn str_carried_across_a_repl_line_then_print_is_correct_not_a_pointer() {
    // A carried `str`'s `IrType` used to be discarded at the REPL line
    // boundary, so `.` on a carried `str` printed a raw decimal pointer
    // instead of the content.
    let out = run_session(&["\"hi\"", "."]);
    // "hi" and "stack:" run together: `.` on a str appends no newline today; R9 records this as pending resolution.
    assert_eq!(out, "stack: <str>\nhistack: (empty)\n");
}

#[test]
fn str_carried_across_a_repl_line_then_len_is_correct_not_a_panic() {
    // The same discarded `IrType` made `len` fall into the array-`len` path
    // and hit `unreachable!` at src/ir.rs.
    let out = run_session(&["\"hi\"", "len"]);
    assert_eq!(out, "stack: <str>\nstack: 2\n");
}

#[test]
fn cstr_carried_across_a_repl_line_then_print_is_correct_not_a_pointer() {
    let out = run_session(&["\"hi\" cstr", "."]);
    // "hi" and "stack:" run together: `.` on a cstr appends no newline, like `str` above.
    assert_eq!(out, "stack: <cstr>\nhistack: (empty)\n");
}

#[test]
fn str_carried_across_a_repl_line_alongside_another_slot_is_correct() {
    // T4: a `str` slot's buffer offset must stay correct when a scalar slot
    // sits above it on the carried stack, covering the offset arithmetic end
    // to end rather than only through the Rust-side `format_stack` units.
    let out = run_session(&["\"hi\"", "7", ". ."]);
    // The first `.` pops `7` (an `i64`, which appends a newline); the second
    // pops the `str` and prints "hi" with none, running into "stack:".
    assert_eq!(out, "stack: <str>\nstack: <str> 7\n7\nhistack: (empty)\n");
}

#[test]
fn bool_carried_across_a_repl_line_then_print_is_true_not_one() {
    // The carried-slot prologue's `_` wildcard used to load every
    // unrecognised slot, including `Bool`, as a bare `IrType::I64`, so a
    // carried `true` printed `1` (the old backend `Bool` Print arm was
    // unreachable from the carried path) instead of `true`. Same-line
    // `true .` was always correct; only the carried path degraded.
    let out = run_session(&["true", "."]);
    assert_eq!(out, "stack: true\ntrue\nstack: (empty)\n");
}

#[test]
fn bool_print_dispatches_to_library_overload_same_line() {
    // Slice 9 phase 2 (R6): the primitive `bool` printable row is gone from
    // `.`'s `BUILTIN_TABLE`; `true .`/`false .` now resolve through 8a's
    // `builtin_overloads` dispatch to `core::bool`'s own `.` overload, which
    // prints identically. A REPL twin of
    // `leap_year_dogfood_compiles_and_runs`'s native coverage, proving the
    // session's own copy (`Session::new`'s `eval_def`) resolves too, with no
    // fall-through to the deleted builtin `Instr::Print` arm (R7).
    let out = run_session(&["true .", "false ."]);
    assert_eq!(out, "true\nstack: (empty)\nfalse\nstack: (empty)\n");
}

#[test]
fn usize_carried_across_a_repl_line_renders_unsigned_not_negative() {
    // The same wildcard degraded a carried `usize` to `I64`, so an
    // underflowed `usize` (whose `i64` bit pattern is negative) rendered
    // signed in the stack display instead of unsigned.
    let out = run_session(&["\"h\" len \"hh\" len sub"]);
    assert_eq!(out, "stack: 18446744073709551615\n");
}

#[test]
fn usize_comparison_across_a_repl_line_matches_same_line_semantics() {
    // The sharpest regression: a degraded carried `usize` compares signed
    // (`lt`'s dispatch reads the value's `IrType`), which is a semantic
    // change, not just cosmetic. The wrapped-around difference is negative
    // as an `i64` but enormous as a `usize`, so `lt "hh" len` (2) flips
    // between `true` (signed) and `false` (unsigned, correct); the same-line
    // form is the reference.
    // P8.S2 (R3): `lt` is a `core` word a session imports, not a seeded one.
    let cmp = common::repl_core_import("cmp", "lt");
    let same_line = run_session(&[&cmp, "\"h\" len \"hh\" len sub \"hh\" len lt ."]);
    let carried = run_session(&[&cmp, "\"h\" len \"hh\" len sub", "\"hh\" len lt ."]);
    assert_eq!(same_line, "imported cmp\nfalse\nstack: (empty)\n");
    assert_eq!(
        carried,
        "imported cmp\nstack: 18446744073709551615\nfalse\nstack: (empty)\n"
    );
}

#[test]
fn print_of_a_str_native() {
    // Criterion 8, run: `emit_print_of_str_uses_precision_format` only
    // asserts the QBE IL substring and never executes it.
    let src = ": main ( -- )\n  \"hello\" . ;";
    let (stdout, code) = run_src("print-str", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "hello");
}

#[test]
fn print_of_a_cstr_native() {
    // Criterion 9, run: `emit_print_of_cstr_uses_string_format` only asserts
    // the QBE IL substring and never executes it.
    let src = ": main ( -- )\n  \"hello\" cstr . ;";
    let (stdout, code) = run_src("print-cstr", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "hello");
}

#[test]
fn embedded_newline_escape_prints_as_a_literal_byte_native() {
    // R9: `.` on a `str`/`cstr` writes exactly the literal's bytes and
    // appends nothing, so a newline only appears where the source spelled
    // `\n` itself — pinning that the current no-trailing-newline behaviour is
    // not also silently swallowing embedded ones.
    let src = ": main ( -- )\n  \"one\\ntwo\\n\" . \"a\\tb\" . ;";
    let (stdout, code) = run_src("embedded-newline", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "one\ntwo\na\tb");
}

#[test]
fn extern_word_name_may_differ_from_its_c_symbol_native() {
    // R1: the C symbol is a separate string from the declared word name
    // (binding `openat` as `open` must be possible); every other golden
    // declares an identical name and symbol, so a lowering bug emitting
    // `call $<word-name>` instead of the declared symbol would go uncaught.
    let src = concat!(
        "extern: clen ( cstr -- usize ) \"strlen\" ;\n\n",
        ": main ( -- )\n",
        "  \"hello\" cstr clen . ;"
    );
    let (stdout, code) = run_src("extern-rename", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "5\n");
}

#[test]
fn interior_nul_diverges_sooth_len_from_c_strlen_native() {
    // T6: pins the interior-NUL behaviour rather than leaving it a surprise.
    // Sooth's `len` counts every byte the literal named (5, including the
    // embedded `\0`); C's `strlen` and `%.*s`-bounded `printf`'s underlying
    // `%s`-style scan both stop at the first NUL, so `cstr strlen` reads 2
    // and `cstr .` prints only `ab`. `\0` is a supported escape (R6), so
    // this is reachable, not rejected.
    let src = concat!(
        "extern: strlen ( cstr -- usize ) \"strlen\" ;\n\n",
        ": main ( -- )\n",
        "  \"ab\\0cd\" | s |\n",
        "  s len .\n",
        "  s cstr strlen .\n",
        "  s cstr . ;"
    );
    let (stdout, code) = run_src("interior-nul", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "5\n2\nab");
}

#[test]
fn str_stored_in_a_struct_field_round_trips_native() {
    // T8/criterion 15, run: `str_and_cstr_are_copy_and_storable` (check.rs)
    // only asserts the checker accepts storing a `str` in a field; this runs
    // the round trip through the generated getter and prints via `len`/`.`.
    let src = concat!(
        "type: Box s str ;\n\n",
        ": main ( -- )\n",
        "  \"hi\" Box | b |\n",
        "  b Box> len .\n",
        "  b Box> . ;"
    );
    let (stdout, code) = run_src("str-struct-field", src);
    assert_eq!(code, 0, "golden should exit 0");
    assert_eq!(stdout, "2\nhi");
}

#[test]
fn slice8a_dogfood_compiles_and_runs() {
    // Criterion 16: the committed dogfood, `examples/strings.sth`, declares
    // `strlen` and `puts` against a literal and runs to the documented
    // output; the two `12`s agreeing is criterion 7 again, this time via the
    // committed example rather than an inline golden.
    let binary = common::build_example("examples/strings.sth");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    assert_eq!(output.status.code(), Some(0), "golden should exit 0");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "12\n12\nhello, sooth\n"
    );
}
