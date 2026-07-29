//! Goldens for Phase 3 Slice 8a: `str`/`cstr` and their literals.
//!
//! Kept out of `tests/phase0.rs` deliberately: that file is asserted never to
//! change from this work's base commit, so a new golden belongs somewhere the
//! addition-only check has nothing to reason about.

use sooth::ir::{lower, Instr};
use sooth::{check, lexer, parser};

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
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
    let mut module = parser::parse(&tokens).unwrap();
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
fn slice8a_dogfood_compiles_and_runs() {
    // Criterion 16: the committed dogfood, `examples/strings.sth`, declares
    // `strlen` and `puts` against a literal and runs to the documented
    // output; the two `12`s agreeing is criterion 7 again, this time via the
    // committed example rather than an inline golden.
    let binary = sooth::driver::build(std::path::Path::new("examples/strings.sth"))
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    assert_eq!(output.status.code(), Some(0), "golden should exit 0");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "12\n12\nhello, sooth\n"
    );
}
