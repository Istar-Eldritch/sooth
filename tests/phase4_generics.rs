//! Phase 4 Slice 1 goldens. This phase covers the multi-output aggregate-return
//! ABI (R10, R11): a user word may declare two or more outputs, and calling one
//! now works instead of panicking the compiler.

use std::process::Command;

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process. `trace` sets the allocation
/// trace explicitly either way, so an ambient value can neither hide a trace
/// nor add one.
fn run_src(name: &str, src: &str, trace: bool) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
    let mut cmd = Command::new(&binary);
    match trace {
        true => cmd.env(sooth::ir::TRACE_ALLOC_ENV, "1"),
        false => cmd.env_remove(sooth::ir::TRACE_ALLOC_ENV),
    };
    let output = cmd.output().expect("binary should run");
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
fn two_output_word_call_pushes_both_results() {
    // Criterion 2 (R10, R11): the recon-3 reproduction. `: pair` checked and
    // lowered before this slice, but calling it dropped the second result and
    // panicked lowering on the next consumer of the value never pushed.
    let (stdout, code) = run_src(
        "pair",
        ": pair ( i64 -- i64 i64 ) dup ;\n: main ( -- ) 5 pair . . ;\n",
        false,
    );
    assert_eq!(stdout, "5\n5\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_outputs_arrive_deepest_first() {
    // R10/R11: the bundle preserves output order — the leftmost declared
    // output is deepest, so the two prints come off top-first.
    let (stdout, code) = run_src(
        "order",
        ": two ( -- i64 bool ) 1 true ;\n: main ( -- ) two . . ;\n",
        false,
    );
    assert_eq!(stdout, "true\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn three_output_word_with_an_aggregate_output_runs() {
    // R10/R11: the ABI is count-agnostic (which is what lets a resolved row
    // variable ride it later), and a struct output travels in the bundle by
    // value like any other field.
    let (stdout, code) = run_src(
        "three",
        "type: Vec2 x i64 y i64 ;\n\
         : spread ( -- i64 Vec2 bool ) 1 2 3 Vec2 true ;\n\
         : main ( -- ) spread . Vec2>y . . ;\n",
        false,
    );
    assert_eq!(stdout, "true\n3\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_with_a_linear_output_frees_its_cell_exactly_once() {
    // Criterion 10 (R10, R11, key risk 1): the bundle for `( -- ^i64 i64 )`
    // folds linear, since its first field is an owning cell. The caller's
    // unpack moves that cell out, and the bundle itself carries no destructor,
    // so the trace shows exactly one alloc and one free — no double free, no
    // leak.
    let (stdout, code) = run_src(
        "cellpair",
        ": cell-and-tag ( -- ^i64 i64 ) 7 ^ 3 ;\n: main ( -- ) cell-and-tag . ^> . ;\n",
        true,
    );
    assert_eq!(stdout, "alloc 8\n3\nfree 8\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn two_output_word_result_feeds_another_call() {
    // R11: the unpacked values are ordinary stack values, so they flow into
    // the next word exactly as any other operands do.
    let (stdout, code) = run_src(
        "chain",
        ": split ( i64 -- i64 i64 ) dup 1 - ;\n\
         : main ( -- ) 5 split + . ;\n",
        false,
    );
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}
