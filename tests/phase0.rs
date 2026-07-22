//! Phase 0 golden tests: the exit criterion is that these programs compile to a
//! standalone native binary and run correctly, plus one negative golden for the
//! stack-effect diagnostic.

use std::path::Path;

use sooth::{check, driver, lexer, parser};

fn run_and_capture_stdout(path: &str) -> (String, i32) {
    let binary = driver::build(Path::new(path)).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

#[test]
fn gcd_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/gcd.sth");
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
}

#[test]
fn factorial_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/factorial.sth");
    assert_eq!(stdout, "120\n");
    assert_eq!(code, 0);
}

#[test]
fn lerp_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/lerp.sth");
    assert_eq!(stdout, "30\n");
    assert_eq!(code, 0);
}

#[test]
fn stack_effect_mismatch_reports_diagnostic() {
    let src = ": oops ( i64 -- i64 )\n  | a | a a + + ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("oops"), "error should name the word: {err}");
    assert!(err.contains('+'), "error should name the operator: {err}");
    assert!(
        err.contains("needs 2 values"),
        "error should state the required arity: {err}"
    );
    assert!(
        err.contains("holds 1"),
        "error should state the actual depth: {err}"
    );
    assert!(
        err.contains("( i64 -- i64 )"),
        "error should include the declared effect: {err}"
    );
}

#[test]
fn build_surfaces_checker_error() {
    let src = ": oops ( i64 -- i64 )\n  | a | a a + + ;\n";
    let path = std::env::temp_dir().join(format!("sooth-badsrc-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");

    let err = driver::build(&path).expect_err("build should fail on a bad program");
    std::fs::remove_file(&path).ok();

    assert!(
        err.contains("oops") && err.contains("needs 2 values"),
        "build should propagate the checker diagnostic: {err}"
    );
}
