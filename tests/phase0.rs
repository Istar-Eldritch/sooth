//! Phase 0 golden tests: the exit criterion is that these programs compile to a
//! standalone native binary and run correctly, plus one negative golden for the
//! stack-effect diagnostic.

use std::path::Path;

use sooth::{check, driver, lexer, test_support};

mod common;

fn run_and_capture_stdout(path: &str) -> (String, i32) {
    run_binary(path, false)
}

/// Compile and run with the allocation trace enabled (R10). The trace shares
/// stdout with the program's own output, so the caller reads one transcript in
/// program order: `alloc <size>`/`free <size>` lines interleaved with whatever
/// the program printed.
fn run_and_capture_traced_stdout(path: &str) -> (String, i32) {
    run_binary(path, true)
}

fn run_binary(path: &str, trace: bool) -> (String, i32) {
    let binary = driver::build_with_manifest(
        Path::new(path),
        common::manifest_for(Path::new(path)).as_deref(),
    )
    .expect("build should succeed");
    let mut cmd = std::process::Command::new(&binary);
    // The gate is set or cleared explicitly, so an ambient value in the caller's
    // environment can neither hide a trace nor add one.
    match trace {
        true => cmd.env(sooth::ir::TRACE_ALLOC_ENV, "1"),
        false => cmd.env_remove(sooth::ir::TRACE_ALLOC_ENV),
    };
    let output = cmd.output().expect("binary should run");
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
fn alloc_trace_stays_empty_for_a_program_that_never_allocates() {
    // The allocator shim, its trap and its trace are emitted unconditionally (the
    // drop-spy precedent), so a program that constructs no cell never calls them:
    // even with the gate on, its transcript is only its own output.
    let (stdout, code) = run_and_capture_traced_stdout("examples/gcd.sth");
    assert_eq!(stdout, "5\n");
    assert_eq!(code, 0);
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
fn sign_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/sign.sth");
    assert_eq!(stdout, "0\n");
    assert_eq!(code, 0);
}

#[test]
fn bool_crosses_call_boundary_and_runs() {
    // `pos` returns a `bool` that `classify` consumes across a call/ret
    // boundary into `if`, proving a bool survives the word-call ABI.
    let (stdout, code) = run_and_capture_stdout("examples/bool_abi.sth");
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn rgb_pack_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/rgb.sth");
    assert_eq!(stdout, "660510\n30\n");
    assert_eq!(code, 0);
}

#[test]
fn signed_vs_unsigned_compare_differ_on_same_bit_pattern() {
    // Same bit pattern (200), compared as `i8` (negative) vs `u8` (positive)
    // against `5`, must give differing results (proves R10 codegen, S4).
    let src = ": signed_lt ( -- i64 )\n  200 >i8 5 >i8 lt ~[ 1 ] ~[ 0 ] if ;\n\n\
: unsigned_lt ( -- i64 )\n  200 >u8 5 >u8 lt ~[ 1 ] ~[ 0 ] if ;\n\n\
: main ( -- )\n  signed_lt .\n  unsigned_lt . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-signed-vs-unsigned-cmp-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn narrowing_conversion_truncates_and_widens_back_correctly() {
    // S6: `511 >u8 >i64 .` prints `255` (well-defined wrapping truncation).
    let src = ": main ( -- )\n  511 >u8 >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-truncation-golden-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "255\n");
    assert_eq!(code, 0);
}

#[test]
fn float_to_int_truncates_toward_zero_end_to_end() {
    // R18/S5: float->int truncates toward zero, not floor. `3.9 >i64` is `3`
    // and `-3.9 >i64` is `-3` (floor would give `-4`), proving `dtosi` runs in
    // a native binary.
    let src = ": main ( -- )\n  3.9 >i64 .\n  -3.9 >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-to-int-trunc-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "3\n-3\n");
    assert_eq!(code, 0);
}

#[test]
fn signed_widen_surfaces_negative_end_to_end() {
    // FIX 4a: `200 >i8 >i64 .` widens a signed sub-word value and prints the
    // sign-extended result (`200` wraps to `-56` as `i8`), proving a signed
    // widen is correct in a running binary, not just at the IL level.
    let src = ": main ( -- )\n  200 >i8 >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-signed-widen-golden-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "-56\n");
    assert_eq!(code, 0);
}

#[test]
fn signed_widen_to_unsigned_subword_compares_correctly() {
    // FIX 1 (review cycle 2 blocker): widening a signed sub-word source to an
    // unsigned sub-word target must canonicalize to the target's convention.
    // `200 >u8 >i8` is `-56` as `i8`; widened to `u16` it must read as the
    // logical unsigned value `65480`, not the sign-extended bit pattern, so
    // comparing it against a clean `u16` `65535` must be `True`.
    let src = ": main ( -- )\n  200 >u8 >i8 >u16 65535 >u16 lt ~[ 1 ] ~[ 0 ] if . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-widen-subword-cmp-golden-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn mixed_width_arithmetic_reports_both_types() {
    // X1: an `i32` and an `i64` fed to `add` names both differing types, via the
    // operand-pair-mismatch diagnostic specifically.
    let src = ": f ( -- i32 ) 1 >i32 5 add ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same numeric type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`i32`"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn mixed_sign_comparison_reports_both_types() {
    // X2: `u8` and `i8` fed to `lt` names both differing operand types. Slice
    // 10c: `lt` is a `'T: Copy Ord` library word now, so the rejection is its
    // variable conflict rather than the retired builtin row's operand-pair
    // message; both operand types are still named.
    let src = ": w ( -- Bool ) 200 >u8 5 >i8 lt ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("resolved `'T` to both"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`u8`"), "unexpected message: {err}");
    assert!(err.contains("`i8`"), "unexpected message: {err}");
}

#[test]
fn declared_output_needs_conversion_reports_diagnostic() {
    // X3: literal is `i64`, declared output is `u8`; requires an explicit conversion.
    let src = ": f ( -- u8 ) 5 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`u8`"), "unexpected message: {err}");
}

#[test]
fn conversion_of_bool_reports_diagnostic() {
    // X4: `>i32` applied to a `Bool` is a type error naming the source is not an integer.
    let src = ": w ( -- i32 ) True >i32 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("numeric"), "unexpected message: {err}");
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
}

#[test]
fn conversion_unknown_target_reports_diagnostic() {
    // X5: `>i128` reads as an unknown conversion target.
    let src = ": w ( -- i64 ) 5 >i128 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("i128"), "unexpected message: {err}");
}

#[test]
fn if_condition_not_bool_reports_diagnostic() {
    let src = ": oops ( -- i64 )\n  5 ~[ 1 ] ~[ 2 ] if ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("expected `Bool`"), "unexpected message: {err}");
    assert!(err.contains("found `i64`"), "unexpected message: {err}");
}

#[test]
fn operand_type_mismatch_reports_diagnostic() {
    let src = ": oops ( -- i64 )\n  True 1 add ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
}

#[test]
fn branch_join_type_mismatch_reports_diagnostic() {
    // Slice 10c: the arms are quotation literals, so their disagreement is
    // caught at the argument site (R-P2-3) rather than at the join.
    let src = ": oops ( Bool -- i64 )\n  ~[ 1 ] ~[ True ] if ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("leave different stack shapes"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
}

#[test]
fn declared_output_type_mismatch_reports_diagnostic() {
    let src = ": oops ( i64 -- Bool )\n  1 add ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("type mismatch"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
}

#[test]
fn unknown_type_name_reports_diagnostic() {
    let src = ": oops ( foo -- i64 )\n  1 ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let err = test_support::parse_with_core(&tokens).expect_err("parsing should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("foo"), "unexpected message: {err}");
}

#[test]
fn stack_effect_mismatch_reports_diagnostic() {
    let src = ": oops ( i64 -- i64 )\n  | a | a a add add ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("oops"), "error should name the word: {err}");
    assert!(err.contains("add"), "error should name the operator: {err}");
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
    let src = ": oops ( i64 -- i64 )\n  | a | a a add add ;\n";
    let path = std::env::temp_dir().join(format!("sooth-badsrc-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");

    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail on a bad program");
    std::fs::remove_file(&path).ok();

    assert!(
        err.contains("oops") && err.contains("needs 2 values"),
        "build should propagate the checker diagnostic: {err}"
    );
}

// Phase 6: floats dogfood + goldens (S1-S8).

#[test]
fn mean_dogfood_compiles_and_runs() {
    // S8: `examples/mean.sth` converts two integer inputs to `f64`, divides,
    // and prints via `.`; mean of 10 and 4 prints 2.5.
    let (stdout, code) = run_and_capture_stdout("examples/mean.sth");
    assert_eq!(stdout, "2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn float_arithmetic_runs_on_both_widths_end_to_end() {
    // S1/S3: `add sub mul` run correctly on `f64` and on `f32` (converted back to
    // `f64` for `.`, since `.` prints an `f32` by widening to `f64`).
    let src = ": main ( -- )\n  1.0 2.0 add .\n  5.0 2.0 sub .\n  3.0 4.0 mul .\n  \
1.5 >f32 2.5 >f32 add >f64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-arith-both-widths-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "3\n3\n12\n4\n");
    assert_eq!(code, 0);
}

#[test]
fn float_division_produces_inf_and_nan_with_nan_detectable_via_self_compare() {
    // S3: `1.0 0.0 div` is inf, `0.0 0.0 div` is NaN, with no trap, and NaN is
    // detectable via `x = x` (False only for NaN, D4). `fdiv` runs the
    // division through a real call boundary so QBE cannot constant-fold the
    // literal `0.0 0.0 div` away (an unrelated compile-time-only restriction).
    let src = ": fdiv ( f64 f64 -- f64 )\n  | a b | a b div ;\n\n\
: main ( -- )\n  1.0 0.0 fdiv .\n  0.0 0.0 fdiv .\n  \
0.0 0.0 fdiv dup eq ~[ 1 ] ~[ 0 ] if . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-div-inf-nan-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "unexpected output:\n{stdout}");
    assert_eq!(lines[0], "inf");
    assert!(
        lines[1].to_lowercase().contains("nan"),
        "expected a NaN rendering: {}",
        lines[1]
    );
    assert_eq!(lines[2], "0", "NaN eq NaN must be False");
    assert_eq!(code, 0);
}

#[test]
fn float_comparison_is_ieee_ordered_and_false_for_nan() {
    // S4: an ordered comparison gives the expected boolean, and every
    // comparison involving a NaN produced by `0.0 0.0 div` is False, including
    // `lt` and `gt` against a NaN (not just `eq`, RISK 1).
    let src = ": fdiv ( f64 f64 -- f64 )\n  | a b | a b div ;\n\n\
: main ( -- )\n  1.0 2.0 lt ~[ 1 ] ~[ 0 ] if .\n  2.0 1.0 lt ~[ 1 ] ~[ 0 ] if .\n  \
0.0 0.0 fdiv dup lt ~[ 1 ] ~[ 0 ] if .\n  0.0 0.0 fdiv dup gt ~[ 1 ] ~[ 0 ] if . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-cmp-ordered-nan-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n0\n0\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn int_to_float_and_float_to_float_conversions_run_end_to_end() {
    // S5: int->float and float->float in both directions (`f32`<->`f64`),
    // each printed via `.` (an `f32` source widens to `f64` first).
    let src = ": main ( -- )\n  10 >f64 .\n  3 >f32 >f64 .\n  3.5 >f32 >f64 .\n  \
10 >f32 >f64 . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-int-float-conv-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "10\n3\n3.5\n10\n");
    assert_eq!(code, 0);
}

#[test]
fn unsigned_int_to_float_conversions_run_end_to_end() {
    // Review cycle 2 (B1): unsigned->float conversions emit `uwtof`/`ultof`,
    // which an old installed QBE rejected as an unknown keyword (a
    // checker-accepted feature that crashed at build, uncaught because the
    // prior unit tests only string-matched the emitted IL). This golden
    // actually builds and runs. `4000000000` exceeds `i32::MAX`, so `uwtof`
    // (not a signed `swtof`) is load-bearing for a correct, non-negative
    // result. `-1 >u64` bit-reinterprets to `u64::MAX`; `ultof` renders it as
    // a huge positive float, where a signed `sltof` would render `-1`.
    let src = ": main ( -- )\n  4000000000 >u32 >f64 .\n  -1 >u64 >f64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-unsigned-int-to-float-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "4e+09\n1.84467e+19\n");
    assert_eq!(code, 0);
}

#[test]
fn float_to_unsigned_int_conversions_run_end_to_end() {
    // Review cycle 2 (B1): float->unsigned conversions emit `stoui`/`dtoui`,
    // the other half of the same previously-uncrossed keyword gap. Covers
    // both `dtoui` code paths: a sub-word target (`>u8`) that routes through
    // the shared canonicalization point and wraps (`300.0 -> 300 mod 256 =
    // 44`), and a 64-bit target (`>u64`) that writes `dtoui` directly with no
    // canonicalization, truncating toward zero (`100.7 -> 100`).
    let src = ": main ( -- )\n  300.0 >u8 >i64 .\n  100.7 >u64 >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-to-unsigned-int-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "44\n100\n");
    assert_eq!(code, 0);
}

#[test]
fn mixed_int_float_arithmetic_reports_diagnostic() {
    // X1 (headline negative, S8): `add` fed an `i64` and an `f64` names both
    // differing types via the operand-pair-mismatch diagnostic.
    let src = ": f ( -- f64 ) 1 3.0 add ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same numeric type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn mixed_float_width_comparison_reports_diagnostic() {
    // X2: `f32` and `f64` fed to `lt` names both differing operand types
    // (slice 10c: through the library `lt`'s variable conflict, see
    // `mixed_sign_comparison_reports_both_types`).
    let src = ": w ( -- Bool ) 1.0 >f32 2.0 lt ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("resolved `'T` to both"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`f32`"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn integer_division_reports_diagnostic() {
    // X3: `div` requires floats; two `i64` operands is an error.
    let src = ": f ( -- i64 ) 6 2 div ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("div"), "unexpected message: {err}");
    assert!(err.contains("float"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn float_mod_reports_diagnostic() {
    // X4: `mod` stays integer-only; two `f64` operands is an error.
    let src = ": f ( -- f64 ) 6.0 2.0 mod ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("mod"), "unexpected message: {err}");
    assert!(err.contains("integer"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn bool_to_float_conversion_reports_diagnostic() {
    // X5: `>f64` applied to a `Bool` names the source and states it must be
    // numeric.
    let src = ": w ( -- f64 ) True >f64 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("numeric"), "unexpected message: {err}");
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
}

#[test]
fn unknown_float_conversion_target_reports_diagnostic() {
    // X6: `>f128` is an unknown conversion target.
    let src = ": w ( -- f64 ) 5.0 >f128 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("f128"), "unexpected message: {err}");
}

// Bitwise operators (`and`/`or`/`xor`/`not`/`shl`/`shr`) diagnostics + goldens.

#[test]
fn bitwise_op_on_float_reports_diagnostic() {
    let src = ": w ( -- f64 ) 3.0 5.0 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("integer"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn bitwise_op_on_bool_is_now_accepted() {
    // `and`/`or`/`xor` are type-directed: `Bool` is now a valid homogeneous
    // operand class, not just the integer tower.
    let src = ": w ( -- Bool ) True False and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
}

#[test]
fn mixed_bool_int_and_reports_both_types() {
    let src = ": w ( -- Bool ) True 5 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same integer or Bool type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn mixed_type_and_reports_both_types() {
    let src = ": w ( -- i64 ) 1 >i32 2 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same integer or Bool type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`i32`"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn shift_with_non_i64_count_reports_diagnostic() {
    let src = ": w ( -- u8 ) 1 >u8 3 >i32 shl ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("`shl`"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`i32`"), "unexpected message: {err}");
}

#[test]
fn bitwise_and_or_xor_not_produce_known_values() {
    let src = ": main ( -- )\n  12 10 and .\n  12 10 or .\n  12 10 xor .\n  0 not . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-bitwise-and-or-xor-not-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "8\n14\n6\n-1\n");
    assert_eq!(code, 0);
}

#[test]
fn shr_is_type_directed_arithmetic_for_signed_logical_for_unsigned() {
    // The same bit pattern (200), shifted right by 1, gives different
    // results as `i8` (arithmetic, sign-preserving) vs `u8` (logical).
    let src = ": main ( -- )\n  200 >i8 1 shr >i64 .\n  200 >u8 1 shr >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-shr-type-directed-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "-28\n100\n");
    assert_eq!(code, 0);
}

#[test]
fn subword_shift_masks_overshift_count_to_type_width() {
    // A `u8` shifted by a runtime count >= 8 wraps mod 8 (Rust
    // `wrapping_shl`/`shr` semantics), not mod 32 (the `w` register width):
    // `1 shl 10` shifts by `10 mod 8 = 2`, and `255 shl 8` shifts by `0`
    // (no-op). Routed through a real word call so the shift isn't folded away
    // at compile time.
    let src = ": shl8 ( i64 i64 -- i64 )\n  | v c | v >u8 c shl >i64 ;\n\n: shr8 ( i64 i64 -- i64 )\n  | v c | v >u8 c shr >i64 ;\n\n: main ( -- )\n  1 10 shl8 .\n  255 8 shl8 .\n  128 9 shr8 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-subword-shift-overshift-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "4\n255\n64\n");
    assert_eq!(code, 0);
}

#[test]
fn rgb_bits_dogfood_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/rgb_bits.sth");
    assert_eq!(stdout, "660510\n20\n");
    assert_eq!(code, 0);
}

#[test]
fn unsigned_subword_not_canonicalizes_to_type_width() {
    // `not` on a `u8` must re-mask to 8 bits: bitwise-not of 5 is 0xFA (250)
    // in a `u8`, not the i64 all-ones complement (-6).
    let src = ": main ( -- )\n  5 >u8 not >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-unsigned-subword-not-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "250\n");
    assert_eq!(code, 0);
}

#[test]
fn signed_subword_shift_high_bits_are_canonical_for_comparison() {
    // `1 << 7` in an `i8` is -128 (0x80), which must compare as `< 0`. If the
    // high bits weren't kept canonical within the `i8` width, the comparison
    // could see stale bits instead of the correct sign.
    let src = ": main ( -- )\n  1 >i8 7 shl 0 >i8 lt ~[ 1 . ] ~[ 0 . ] if ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-signed-subword-shift-compare-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn negative_shift_count_masks_to_type_width() {
    // A negative runtime shift count must mask to the type width rather than
    // trap or invoke UB: -6 mod 8 = 2, so shifting a `u8` by -6 shifts by 2,
    // giving 4.
    let src = ": main ( -- )\n  1 >u8  0 6 sub  shl >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-negative-shift-count-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "4\n");
    assert_eq!(code, 0);
}

// Boolean logical ops (`and`/`or`/`xor`/`not` on `Bool`) + the `lte gte ne`
// comparison completion: diagnostics + goldens.

#[test]
fn cmp_le_ge_ne_on_bool_reports_diagnostic() {
    let src = ": w ( -- Bool ) True False lte ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same numeric type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Bool`"), "unexpected message: {err}");
}

#[test]
fn logical_and_or_xor_truth_table_on_bools() {
    // `and`/`or`/`xor` on `Bool` operands ARE logical and/or/xor (an eager
    // stack language already evaluates both operands, so bitwise-on-0/1 and
    // logical coincide): T and T = T, T and F = F, T or F = T, F or F = F,
    // T xor F = T, T xor T = F.
    let src = ": main ( -- )\n  \
  True True and ~[ 1 ] ~[ 0 ] if .\n  \
  True False and ~[ 1 ] ~[ 0 ] if .\n  \
  True False or ~[ 1 ] ~[ 0 ] if .\n  \
  False False or ~[ 1 ] ~[ 0 ] if .\n  \
  True False xor ~[ 1 ] ~[ 0 ] if .\n  \
  True True xor ~[ 1 ] ~[ 0 ] if . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-logical-and-or-xor-truth-table-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n0\n1\n0\n1\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn not_is_type_directed_bool_logical_vs_integer_bitwise() {
    // `not` is type-directed: on a `Bool` it is logical negation
    // (`True not` -> False), giving a DIFFERENT result than the integer
    // bitwise complement on the same underlying bit pattern (`0 >u8 not` ->
    // 255, not 1).
    let src = ": main ( -- )\n  \
  True not ~[ 1 ] ~[ 0 ] if .\n  \
  0 >u8 not >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-not-type-directed-Bool-vs-int-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n255\n");
    assert_eq!(code, 0);
}

#[test]
fn le_ge_ne_on_integers_with_signed_unsigned_edge() {
    // The same bit pattern (200) compares differently as `i8` (-56, negative)
    // vs `u8` (200, positive) against 5: `lte`/`gte` flip with the sign, while
    // `ne` stays True either way (not-equal is sign-agnostic like `eq`).
    let src = ": main ( -- )\n  \
  200 >i8 5 >i8 lte ~[ 1 ] ~[ 0 ] if .\n  \
  200 >u8 5 >u8 lte ~[ 1 ] ~[ 0 ] if .\n  \
  200 >i8 5 >i8 gte ~[ 1 ] ~[ 0 ] if .\n  \
  200 >u8 5 >u8 gte ~[ 1 ] ~[ 0 ] if .\n  \
  200 >i8 5 >i8 ne ~[ 1 ] ~[ 0 ] if .\n  \
  200 >u8 5 >u8 ne ~[ 1 ] ~[ 0 ] if . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-le-ge-ne-signed-unsigned-edge-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n0\n0\n1\n1\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn le_ge_ne_are_ieee_ordered_and_correct_for_nan_floats() {
    // A real NaN (`0.0 0.0 div`, routed through a call so it isn't
    // constant-folded away) must report False for the ordered comparisons
    // `lte`/`gte`/`eq`, and True for `ne` (RISK 1): `ne` is the one comparison
    // where "NaN involved" flips the answer relative to `eq`.
    let src = ": fdiv ( f64 f64 -- f64 )\n  | a b | a b div ;\n\n\
: main ( -- )\n  \
  0.0 0.0 fdiv dup lte ~[ 1 ] ~[ 0 ] if .\n  \
  0.0 0.0 fdiv dup gte ~[ 1 ] ~[ 0 ] if .\n  \
  0.0 0.0 fdiv dup ne ~[ 1 ] ~[ 0 ] if .\n  \
  0.0 0.0 fdiv dup eq ~[ 1 ] ~[ 0 ] if . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-le-ge-ne-nan-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n0\n1\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn leap_year_dogfood_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/leap.sth");
    assert_eq!(stdout, "True\nFalse\nTrue\n");
    assert_eq!(code, 0);
}

// Phase 2: `.` becomes type-directed over every printable scalar; `f.` is gone.

#[test]
fn print_signed_negative_i64_prints_signed_decimal() {
    let src = ": main ( -- )\n  -42 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-print-signed-negative-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "-42\n");
    assert_eq!(code, 0);
}

#[test]
fn print_unsigned_u64_high_bit_set_prints_unsigned_decimal() {
    // The headline gap-closer: a `u64` with the high bit set (`-1 >u64`) must
    // print its full unsigned value, not `-1` reinterpreted as signed.
    let src = ": main ( -- )\n  -1 >u64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-print-unsigned-u64-high-bit-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "18446744073709551615\n");
    assert_eq!(code, 0);
}

#[test]
fn print_unsigned_subword_widths_print_unsigned_decimal() {
    // `u8` in range, and a `u32` with its high bit set (also negative if
    // misread as signed): both must print unsigned.
    let src = ": main ( -- )\n  255 >u8 .\n  4000000000 >u32 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-print-unsigned-subword-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "255\n4000000000\n");
    assert_eq!(code, 0);
}

#[test]
fn print_float_f64_and_f32_via_dot() {
    // `f.` is gone; `.` prints both float widths (an `f32` widens to `f64`
    // first).
    let src = ": main ( -- )\n  2.5 .\n  1.5 >f32 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-print-float-widths-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "2.5\n1.5\n");
    assert_eq!(code, 0);
}

#[test]
fn print_bool_prints_true_or_false_not_zero_or_one() {
    let src = ": main ( -- )\n  2 3 lt .\n  3 2 lt . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-print-Bool-True-False-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "True\nFalse\n");
    assert_eq!(code, 0);
}

#[test]
fn f_dot_is_now_an_unknown_word() {
    // `f.` is removed entirely: it reads as any other unknown word.
    let src = ": w ( f64 -- ) f. ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail: `f.` no longer exists");

    assert!(err.contains("unknown word"), "unexpected message: {err}");
    assert!(err.contains("f."), "unexpected message: {err}");
}

// Slice 3 (structs): running-binary goldens for the aggregate codegen (S2-S7,
// NF5). Each builds a struct program to a native binary and checks stdout.

fn run_struct_golden(tag: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-struct-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(code, 0, "struct golden `{tag}` should exit 0");
    stdout
}

#[test]
fn struct_flat_construct_get_destructure_native() {
    // S2: construct a flat struct, read each field, and destructure it.
    let src = "type: Vec2 x i64 y i64 ;\n\
: main ( -- )\n  3 4 Vec2 &x @ . &y @ . drop\n  5 6 Vec2 Vec2> . . ;\n";
    // destructure pushes x then y (first deepest); `. .` prints top-first: 6 then 5.
    assert_eq!(run_struct_golden("flat", src), "3\n4\n6\n5\n");
}

#[test]
fn struct_functional_setter_leaves_duped_original_intact_native() {
    // S4: `dup` copies the aggregate; a functional setter on the copy returns a
    // new value while the original is unchanged.
    let src = "type: Vec2 x i64 y i64 ;\n\
: main ( -- )\n  1 2 Vec2 dup &!x 99 ! &x @ . drop &x @ . drop ;\n";
    assert_eq!(run_struct_golden("setter-intact", src), "99\n1\n");
}

#[test]
fn struct_mixed_i64_f64_field_readback_native() {
    // S3: offset-correct read-back for mixed-width fields (an i64 and an f64).
    let src = "type: Mix a i64 b f64 ;\n\
: main ( -- )\n  7 2.5 Mix &a @ . &b @ . drop ;\n";
    assert_eq!(run_struct_golden("mixed", src), "7\n2.5\n");
}

#[test]
fn struct_adjacent_subword_fields_do_not_clobber_native() {
    // RISK 3: two adjacent `i8` fields then an `i64` must each read back their
    // own value; a width-exact field store never clobbers its neighbour.
    let src = "type: P p i8 q i8 r i64 ;\n\
: main ( -- )\n  1 >i8 2 >i8 300 P &p @ >i64 . &q @ >i64 . &r @ . drop ;\n";
    assert_eq!(run_struct_golden("packed", src), "1\n2\n300\n");
}

#[test]
fn struct_nested_juxtaposition_access_native() {
    // S3: a nested struct field accessed by a chained projection (`&to &x @`),
    // read back per-field.
    let src = "type: Vec2 x i64 y i64 ;\n\
type: Segment from Vec2 to Vec2 ;\n\
: main ( -- )\n  1 2 Vec2 3 4 Vec2 Segment\n  &from &x @ .\n  &to &y @ . drop ;\n";
    assert_eq!(run_struct_golden("nested", src), "1\n4\n");
}

#[test]
fn struct_survives_word_call_boundary_native() {
    // S5: a struct argument and a struct return cross a word-call boundary
    // (by-value QBE C-ABI), then the returned struct's field is read back.
    let src = "type: Vec2 x i64 y i64 ;\n\
: shift ( Vec2 i64 -- Vec2 ) | v d |\n  &v &x @ d add &v &y @ Vec2 ;\n\
: main ( -- )\n  10 20 Vec2 5 shift &x @ . &y @ . drop ;\n";
    assert_eq!(run_struct_golden("call-boundary", src), "15\n20\n");
}

#[test]
fn struct_nested_struct_crosses_word_call_boundary_native() {
    // A struct with a nested-struct field (Segment, holding two Vec2s) passed
    // directly as a word argument and returned directly as the word's result,
    // isolating the nested-aggregate ABI rather than only exercising it
    // transitively through the `vectors` dogfood's `span`.
    let src = "type: Vec2 x i64 y i64 ;\n\
type: Segment from Vec2 to Vec2 ;\n\
: swap-ends ( Segment -- Segment )\n  Segment> swap Segment ;\n\
: main ( -- )\n  1 2 Vec2 3 4 Vec2 Segment\n  swap-ends\n  &from &x @ .\n  &to &y @ . drop ;\n";
    // from=(1,2) to=(3,4) swapped -> from=(3,4) to=(1,2): from.x=3, to.y=2.
    assert_eq!(run_struct_golden("nested-call-boundary", src), "3\n2\n");
}

#[test]
fn struct_zero_field_unit_end_to_end_native() {
    // S7/M3: a zero-field struct constructs and destructures with no crash.
    let src = "type: Unit ;\n\
: main ( -- )\n  Unit Unit>\n  42 . ;\n";
    assert_eq!(run_struct_golden("unit", src), "42\n");
}

#[test]
fn enum_crosses_word_call_boundary_with_scalar_underneath_native() {
    // Slice 4 (criterion 7, native half): an enum value passes through a
    // word-call boundary by value (QBE aggregate C-ABI) and back, with a
    // scalar sitting *underneath* it on the caller's stack. A mis-sized
    // aggregate slot in the ABI classification would clobber that scalar, so
    // recovering `42` intact proves the by-value enum ABI. A three-`i64`
    // large-payload variant (exceeding one 8-byte cell) makes the boundary
    // non-trivial. The enum itself can't be read back until the eliminator
    // lands (Phase 4), so it is dropped after the round-trip.
    let src = "type: Big | B a i64 b i64 c i64 ;\n\
: id ( Big -- Big ) ;\n\
: main ( -- )\n  42 1 2 3 B id drop . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-enum-call-boundary-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn vectors_dogfood_compiles_and_runs() {
    // S8: `examples/vectors.sth` — a flat `Vec2` and a nested `Segment`, a
    // reusable componentwise `sub`, `len2`, `span` (= `Segment> swap sub`),
    // and a functional-setter `shift-x` demo. Builds segment (0,0)-(3,4),
    // prints `span len2 .` (25) and `5 6 Vec2 1 shift-&x &x @ .` (6).
    let (stdout, code) = run_and_capture_stdout("examples/vectors.sth");
    assert_eq!(stdout, "25\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn shapes_dogfood_compiles_and_runs() {
    // Slice 4 (criteria 3, 6): enum elimination end-to-end. `area`
    // dispatches over a multi-field-variant `Shape` (a `Rect` arm binding its
    // destructured payload with `| w h |` and a `Circle` arm reading its
    // single field); `unwrap-or` dispatches over a zero-field `None` (whose
    // arm drops the narrowed variant, yielding the default flowing
    // *underneath* the scrutinee) and a one-field `Some`. All run in one
    // native binary.
    let (stdout, code) = run_and_capture_stdout("examples/shapes.sth");
    assert_eq!(stdout, "12.5664\n12\n5\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn eliminator_over_three_plus_variant_enum_dispatches_correctly() {
    // R16 key risk: an N-way (here 4-way) `Cmp(Eq)`-tag compare-chain must
    // land on each variant's own arm, not a two-way miscompile. Each of
    // the four commands drives a distinct arm; verified at runtime, not by
    // reading IL.
    let src = "type: Cmd | Halt | Push v i64 | Add | Dbl ;\n\
: run ( i64 Cmd -- i64 )\n\
  ~[ ( Halt ) drop drop 0 ]\n\
  ~[ ( Push ) Push> swap drop ]\n\
  ~[ ( Add )  drop 1 add ]\n\
  ~[ ( Dbl )  drop 2 mul ]\n\
  Cmd? ;\n\
: main ( -- )\n  99 Halt run .\n  1 20 Push run .\n  10 Add run .\n  10 Dbl run . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-nway-elim-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n20\n11\n20\n");
    assert_eq!(code, 0);
}

#[test]
fn nested_aggregate_eliminator_reads_back_through_registries_native() {
    // Slice 4 (criterion 3, D9): a variant carrying a struct payload (`Dot p
    // Vec2`) constructs, passes through an eliminating word, and its nested
    // field reads back; and an enum used as a struct field (`Wrap s Shape`) is
    // unwrapped through the destructure into the same word — guarding the
    // combined-registry field sizing in both directions.
    let src = "type: Vec2 x i64 y i64 ;\n\
type: Shape | Dot p Vec2 | Nothing ;\n\
type: Wrap s Shape ;\n\
: px ( Shape -- i64 )\n\
  ~[ ( Dot )     Dot> &x @ swap drop ]\n\
  ~[ ( Nothing ) drop 0 ]\n\
  Shape? ;\n\
: main ( -- )\n  3 4 Vec2 Dot px .\n  Nothing px .\n  5 6 Vec2 Dot Wrap Wrap> px . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-nested-elim-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "3\n0\n5\n");
    assert_eq!(code, 0);
}

#[test]
fn single_variant_eliminator_returns_payload_native() {
    // 9ed63d9 discriminant-skip: a single-variant enum has nothing to
    // disambiguate, so the dispatch jumps straight to the sole arm with
    // no `Cmp`/`Jnz`. The IR-shape test asserts zero compares; this proves the
    // no-compare path still returns the right payload value at runtime.
    let src = "type: Id | Wrap v i64 ;\n\
: unwrap ( Id -- i64 )\n\
  ~[ ( Wrap ) Wrap> ]\n\
  Id? ;\n\
: main ( -- )\n  42 Wrap unwrap . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-single-variant-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn eliminator_arm_containing_if_joins_correctly() {
    // M6: an arm body may itself use `if`. The dispatch join's phi predecessor
    // must be the *if's* merged block (captured via `cur_id` after lowering
    // the arm body), not the arm's dispatch block — otherwise the join would
    // read a stale/wrong value. `Zero` exercises a plain arm alongside
    // `NonZero`'s internal branch, so the two arm shapes share one dispatch
    // and one join correctly.
    let src = "type: Item | Zero | NonZero v i64 ;\n\
: classify ( Item -- i64 )\n\
  ~[ ( Zero )    drop 0 ]\n\
  ~[ ( NonZero ) NonZero> 0 gt ~[ 1 ] ~[ -1 ] if ]\n\
  Item? ;\n\
: main ( -- )\n  Zero classify .\n  5 NonZero classify .\n  -5 NonZero classify . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-arm-if-else-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n1\n-1\n");
    assert_eq!(code, 0);
}

// Slice 5 (fixed-size arrays + `usize`): success criteria 2-7.

#[test]
fn usize_arithmetic_comparison_and_conversion_native() {
    // Criterion 2: a native golden exercising the `usize` tower end to end -
    // `usize` arithmetic and comparison, `>usize` on a computed value, a
    // `usize`->`i64` conversion (`>i64`), type-directed `.` on a `usize`, and
    // a bare literal coercing into a `usize` position without `>usize` (D8).
    let src = ": main ( -- )\n\
  2 3 add >usize 4 >usize add .\n\
  10 >usize 3 >usize sub .\n\
  3 >usize 5 >usize lt .\n\
  5 >usize 3 >usize lt .\n\
  7 >usize >i64 1 add .\n\
  9 >usize dup . drop ;\n";
    let path = std::env::temp_dir().join(format!("sooth-usize-tower-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "9\n7\nTrue\nFalse\n8\n9\n");
    assert_eq!(code, 0);
}

// Phase 3 slice 3, phase 1 (R1): `isize` mirrors `usize` end to end.

#[test]
fn isize_round_trips_arithmetic_and_conversion() {
    // Criterion 1: `isize` arithmetic and comparison, `>isize` on a computed
    // value, an `isize`->`i64` conversion, type-directed `.` printing signed
    // (a negative result must print as negative, unlike `usize`'s `%lu`), and
    // a bare literal coercing into an `isize` position without `>isize` (D8).
    // Comparisons and the shift both include a negative operand so a
    // wrongly-unsigned codegen arm (falling through past `Isize`) would flip
    // the result instead of silently agreeing with the signed answer.
    let src = ": main ( -- )\n\
  2 3 add >isize 4 >isize add .\n\
  3 >isize 10 >isize sub .\n\
  3 >isize 5 >isize lt .\n\
  5 >isize 3 >isize lt .\n\
  0 >isize 5 >isize sub 1 >isize lt .\n\
  0 >isize 5 >isize sub 0 >isize gt .\n\
  0 >isize 8 >isize sub 1 shr .\n\
  0 >isize 7 >isize sub 2 >isize mod .\n\
  7 >isize >i64 1 add .\n\
  9 >isize dup . drop ;\n";
    let path = std::env::temp_dir().join(format!("sooth-isize-tower-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "9\n-7\nTrue\nFalse\nTrue\nFalse\n-4\n-1\n8\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn fill_constructs_and_reads_every_element_back_native() {
    // Criterion 3: `fill` an `[i64 N]`, then read every element back through
    // a reference (`&>` then `@`) and print it; the values match the fill
    // value (unrolled stores + dynamic element addressing, R17/R18).
    let src = ": main ( -- )\n\
  9 4 fill | a |\n\
  &a 0 &> @ .\n\
  &a 1 &> @ .\n\
  &a 2 &> @ .\n\
  &a 3 &> @ . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-fill-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "9\n9\n9\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn in_place_mutation_of_a_duped_array_leaves_the_original_untouched_native() {
    // Criterion 4: mutating one `dup`ed copy of an array in place through a
    // *runtime* index (computed, not a literal) leaves the other copy
    // untouched (D5 value semantics), since `dup` deep-copies rather than
    // aliasing.
    let src = ": main ( -- )\n\
  0 4 fill dup | a b |\n\
  &!a 1 1 add >usize &!> 99 !\n\
  &a 2 &> @ .\n\
  &a 0 &> @ .\n\
  &b 2 &> @ .\n\
  &b 0 &> @ . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-set-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    // Mutated copy: index 2 changed to 99, index 0 unchanged (0); original
    // copy: both indices still 0.
    assert_eq!(stdout, "99\n0\n0\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn constant_out_of_range_array_index_is_compile_error() {
    // Criterion 5(a), X4: a literal index >= N is a sharp, located compile
    // error naming the length and the index.
    let src = ": w ( -- )\n  0 4 fill | a |\n  &a 9 &> drop ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("out of range"), "unexpected message: {err}");
    assert!(err.contains('9'), "should name the index: {err}");
    assert!(err.contains('4'), "should name the length: {err}");
}

#[test]
fn constant_index_at_length_boundary_is_compile_error() {
    // Index == length is the first invalid index (valid range 0..length-1);
    // distinct off-by-one boundary from the gross violation above.
    let src = ": w ( -- )\n  0 4 fill | a |\n  &a 4 &> drop ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("out of range"), "unexpected message: {err}");
    assert!(err.contains("index 4"), "should name the index: {err}");
    assert!(err.contains("length 4"), "should name the length: {err}");
}

#[test]
fn runtime_out_of_range_array_index_traps_and_aborts_native() {
    // Criterion 5(b): a runtime out-of-range index traps rather than
    // corrupting. Exit code is nonzero, the located message names the length
    // and index, and a sentinel `.` placed *before* the access prints while a
    // sentinel placed *after* it does not, proving the trap fired (aborted)
    // rather than falling through. Length (4) and index (7) are deliberately
    // distinct: the trap format string takes separate `%ld` args for each, so
    // a swapped or duplicated arg would still pass a same-valued assertion.
    let src = ": main ( -- )\n\
  1 .\n\
  0 4 fill | a |\n\
  &a 3 4 add >usize &> @ drop\n\
  99 . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-trap-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    std::fs::remove_file(&path).ok();

    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    let code = output
        .status
        .code()
        .expect("process should exit normally, not die by signal");

    assert_eq!(stdout, "1\n", "sentinel before the trap should print");
    assert!(
        !stdout.contains("99"),
        "sentinel after the trap must not print: {stdout}"
    );
    assert_ne!(code, 0, "an out-of-bounds access must exit nonzero");
    assert!(
        stderr.contains("out of range"),
        "trap message should say it's out of range: {stderr}"
    );
    assert!(
        stderr.contains("index 7"),
        "trap message should name the distinct index (7): {stderr}"
    );
    assert!(
        stderr.contains("length 4"),
        "trap message should name the distinct length (4): {stderr}"
    );
}

#[test]
fn runtime_index_at_length_boundary_traps_and_aborts_native() {
    // Index == length is the first invalid index (valid range 0..length-1),
    // the off-by-one boundary distinct from the gross violation above.
    let src = ": main ( -- )\n\
  1 .\n\
  0 4 fill | a |\n\
  &a 2 2 add >usize &> @ drop\n\
  99 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-array-trap-boundary-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    std::fs::remove_file(&path).ok();

    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    let code = output
        .status
        .code()
        .expect("process should exit normally, not die by signal");

    assert_eq!(stdout, "1\n", "sentinel before the trap should print");
    assert!(
        !stdout.contains("99"),
        "sentinel after the trap must not print: {stdout}"
    );
    assert_ne!(code, 0, "an out-of-bounds access must exit nonzero");
    assert!(
        stderr.contains("out of range"),
        "trap message should say it's out of range: {stderr}"
    );
    assert!(
        stderr.contains("index 4"),
        "trap message should name the boundary index (4): {stderr}"
    );
    assert!(
        stderr.contains("length 4"),
        "trap message should name the length (4): {stderr}"
    );
}

#[test]
fn stack_dogfood_compiles_and_runs() {
    // Criterion 6: `examples/stack.sth`, a bounded `i64` stack embedding a
    // `[i64 16]` field with a runtime `usize` cursor. Exercises
    // array-as-struct-field, `push`/`pop`/`peek` reading and writing an
    // element through a reference, and the compile-time-constant `len`.
    let (stdout, code) = run_and_capture_stdout("examples/stack.sth");
    assert_eq!(stdout, "3\n3\n2\n1\n16\n");
    assert_eq!(code, 0);
}

#[test]
fn nested_array_shapes_construct_and_read_back_native() {
    // Criterion 7: nesting both directions in one binary — array-of-struct
    // (`[Vec2 2]`), array-of-array (`[[i64 2] 2]`), and struct-with-an-
    // array-field (`Box { arr: [i64 3] }`) each construct via `fill` and read
    // back correctly through the combined struct/array registry (R16, M3).
    let src = "type: Vec2 x i64 y i64 ;\n\
type: Box arr [i64 3] ;\n\
: vx ( [Vec2 2] usize -- i64 )\n\
| a i | &a i &> &x @ ;\n\
: inner-at ( [[i64 2] 2] usize usize -- i64 )\n\
| a i j | &a i &> j &> @ ;\n\
: box-at ( Box usize -- i64 )\n\
| b i | &b &arr i &> @ ;\n\
: main ( -- )\n\
  1 2 Vec2 2 fill\n\
  dup 0 vx .\n\
  1 vx .\n\
  9 2 fill 2 fill\n\
  dup 0 0 inner-at .\n\
  1 1 inner-at .\n\
  0 3 fill Box\n\
  dup 0 box-at .\n\
  2 box-at . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-nesting-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n1\n9\n9\n0\n0\n");
    assert_eq!(code, 0);
}

// Phase 2 Slice 6 (self-tail-call -> loop lowering): success criteria 1, 3, 4,
// 7. Every N here is >= 1_000_000: naive (un-transformed) recursion at that
// depth overflows the 8MB default host stack, so a regression that silently
// disables the transform turns these goldens red rather than passing small.

#[test]
fn countdown_dogfood_runs_in_constant_stack() {
    // Criterion 1 (D8): `examples/countdown.sth`, a tail-recursive
    // accumulator summing 1..=1_000_000, completes and prints the right
    // total, and the "recursive case in one arm, base case in the other"
    // half of criterion 4. Criterion 7 (locals rebind correctly across
    // iterations) has its own dedicated golden below, since a commutative sum
    // wouldn't surface a swapped/stale rebind until the very last iteration.
    let (stdout, code) = run_and_capture_stdout("examples/countdown.sth");
    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn locals_rebind_correctly_across_tail_iterations_native() {
    // Criterion 7: `| acc n |` must rebind to the *new* tail-call arguments
    // on every iteration, not to stale or swapped values. `acc*10 + n` is
    // order-sensitive (unlike a sum), so a wrong rebind produces a wrong
    // digit sequence immediately; this only needs a handful of iterations,
    // deliberately kept separate from the constant-stack goldens above.
    let src = ": digits ( i64 i64 -- i64 )\n\
  | acc n |\n\
  n 0 eq ~[\n\
    acc\n\
  ] ~[\n\
    acc 10 mul n add n 1 sub digits\n\
  ] if ;\n\
: main ( -- ) 0 5 digits . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-locals-rebind-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "54321\n");
    assert_eq!(code, 0);
}

#[test]
fn terminal_if_both_arms_tail_produce_two_back_edges_native() {
    // Criterion 4's both-arms-tail golden: a self-tail-call in each arm of a
    // terminal `if` produces two back-edges from the `if` path into one
    // header (R8 multi-arm back-patch through `lower_if`, distinct from the
    // tag-dispatch back-edge path of criterion 3). Both arms happen to do the same
    // arithmetic; what matters is that they are two distinct call sites that
    // both eliminate, at N large enough to overflow if either didn't.
    let src = ": both-tail ( i64 i64 -- i64 )\n\
  | acc n |\n\
  n 0 eq ~[\n\
    acc\n\
  ] ~[\n\
    n 500000 gt ~[\n\
      acc n add n 1 sub both-tail\n\
    ] ~[\n\
      acc n add n 1 sub both-tail\n\
    ] if\n\
  ] if ;\n\
: main ( -- ) 0 1000000 both-tail . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-both-arms-tail-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn eliminator_multi_tail_runs_in_constant_stack_native() {
    // Criterion 3: a self-tail-recursive word where *both* arms tail-recurse
    // into one shared header, alternating tags each iteration (each arm
    // contributes its own back-edge).
    let src = "type: Parity | Even | Odd ;\n\
: sum-parity ( i64 i64 Parity -- i64 )\n\
  ~[ ( Even ) drop | acc n | n 0 eq ~[ acc ] ~[ acc n add n 1 sub Odd sum-parity ] if ]\n\
  ~[ ( Odd )  drop | acc n | n 0 eq ~[ acc ] ~[ acc n add n 1 sub Even sum-parity ] if ]\n\
  Parity? ;\n\
: main ( -- ) 0 1000000 Even sum-parity . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-elim-multi-tail-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn mixed_eliminator_back_edge_and_base_case_runs_in_constant_stack_native() {
    // Criterion 3's mixed-arm golden (R9 / risk 5): both of the `Go` arm's
    // `if` arms are self-tail-calls, so both back-edge to the loop header (one
    // recurses with the `Go` tag, one recurses with the `Halt` tag; neither
    // itself `Ret`s). The only genuine base case is the separate `Halt` arm,
    // which `Ret`s with no self-call at all. The loop header's predecessors
    // (entry + `Go`'s two back-edges) and the Slice-4 dispatch-join's
    // predecessors must stay disjoint for this to compile and run correctly.
    let src = "type: Step | Go | Halt ;\n\
: run-mix ( i64 i64 Step -- i64 )\n\
  ~[ ( Go )   drop | acc n | n 0 eq ~[ acc n Halt run-mix ] ~[ acc n add n 1 sub Go run-mix ] if ]\n\
  ~[ ( Halt ) drop | acc n | acc ]\n\
  Step? ;\n\
: main ( -- ) 0 1000000 Go run-mix . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-mixed-elim-tail-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn enum_get_from_carried_array_eliminator_dispatch_constant_stack() {
    // Slice 7 criterion 8 (the crux): read an `Op` out of the *carried*
    // program array, dispatch on it, and tail-recurse. The array-across-the
    // -back-edge half was proven by a prior spike; the residual unproven
    // composition is enum-`get`-from-carried-array + tag dispatch in
    // constant stack. 1_000_000 back-edges each read the enum out of the
    // carried `[Op 2]`, dispatch, and self-tail-call; naive recursion at that
    // depth overflows the default 8MB host stack, which `run_and_capture_stdout`
    // catches as a signal death (no exit code) and turns a no-op Slice 6
    // transform red. `idx` goes Bool -> index via `if 1 else 0 end >usize`
    // (a conversion word on a `Bool` is a checker error), and `fetch` reads
    // the enum through a reference (`&>` then `@`) rather than `get`.
    let src = "type: Op | Step | Stop ;\n\
: idx ( i64 -- usize ) | count | count 0 eq ~[ 1 ] ~[ 0 ] if >usize ;\n\
: fetch ( [Op 2] usize -- Op ) | a i | &a i &> @ ;\n\
: run ( [Op 2] i64 i64 Op -- i64 )\n\
  ~[ ( Step ) drop | prog count acc |\n\
      prog\n\
      count 1 sub\n\
      acc 1 add\n\
      prog count 1 sub idx fetch\n\
      run ]\n\
  ~[ ( Stop ) drop | prog count acc | acc ]\n\
  Op? ;\n\
: build ( -- [Op 2] ) Step 2 fill | prog | &!prog 1 &!> Stop ! prog ;\n\
: start ( [Op 2] -- i64 ) | prog | prog 1000000 0 prog 0 fetch run ;\n\
: main ( -- ) build start . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-vm-smoke-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1000000\n");
    assert_eq!(code, 0);
}

#[test]
fn vm_dogfood_compiles_and_runs() {
    // Phase 3 of Slice 7 (criteria 1, 2, 4, 5, 6): the VM's real dispatch
    // loop (`fetch` + the nine-arm self-tail-recursive `run`) interprets
    // the sum-1..N bytecode program (built via `fill` and a reference into
    // a named local, no array literal) at N = 10, exercising every opcode
    // the sum program needs
    // (`Push`/`Add`/`Sub`/`Load`/`Store`/`Jz`/`Jmp`/`Halt`) and the `Jz`/`Jmp`
    // backward branch. `Mul` dispatches too (its arm is identical in shape
    // to `Add`/`Sub`) but sum-1..N never multiplies, so this golden doesn't
    // exercise it. This is an inline temp-source copy of `examples/vm.sth`
    // at a small N: Phase 4 scales the committed example to N = 100_000 for
    // the constant-stack golden below, so this fast correctness check keeps
    // its own small-N copy rather than sharing the committed example's `main`.
    let src = "type: Op\n\
| Push v i64\n\
| Add\n\
| Sub\n\
| Mul\n\
| Load  addr usize\n\
| Store addr usize\n\
| Jz    target usize\n\
| Jmp   target usize\n\
| Halt\n\
;\n\
type: Vm\n\
  prog  [Op 13]\n\
  pc    usize\n\
  stack [i64 8]\n\
  sp    usize\n\
  mem   [i64 4]\n\
;\n\
type: Fetched vm Vm op Op ;\n\
type: VmPop vm Vm val i64 ;\n\
: vm-push ( Vm i64 -- Vm )\n\
  | vm x |\n\
  &vm &sp @ | i |\n\
  &!vm &!stack i &!> x !\n\
  &vm &sp @ 1 add | newsp |\n\
  vm &!sp newsp ! ;\n\
: vm-pop ( Vm -- VmPop )\n\
  | vm |\n\
  &vm &sp @ 1 sub | i |\n\
  &vm &stack i &> @ | x |\n\
  vm &!sp i !\n\
  x\n\
  VmPop ;\n\
: bump-pc ( Vm -- Vm )\n\
  &pc @ 1 add | newpc |\n\
  &!pc newpc ! ;\n\
: fetch ( Vm -- Fetched )\n\
  | vm |\n\
  &vm &pc @ | i |\n\
  &vm &prog i &> @ | op |\n\
  vm op Fetched ;\n\
: run ( Vm Op -- i64 )\n\
  ~[ ( Push )\n\
    Push> | vm v |\n\
    vm v vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Add )\n\
    drop | vm |\n\
    vm vm-pop VmPop>\n\
    | b |\n\
    vm-pop VmPop>\n\
    b add\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Sub )\n\
    drop | vm |\n\
    vm vm-pop VmPop>\n\
    | b |\n\
    vm-pop VmPop>\n\
    b sub\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Mul )\n\
    drop | vm |\n\
    vm vm-pop VmPop>\n\
    | b |\n\
    vm-pop VmPop>\n\
    b mul\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Load )\n\
    Load> | vm addr |\n\
    &vm &mem addr &> @ | x |\n\
    vm x vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Store )\n\
    Store> | vm addr |\n\
    vm vm-pop VmPop>\n\
    | v x |\n\
    &!v &!mem addr &!> x !\n\
    v\n\
    bump-pc\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Jz )\n\
    Jz> | vm target |\n\
    vm vm-pop VmPop>\n\
    0 eq\n\
    ~[\n\
      &!pc target !\n\
    ] ~[\n\
      bump-pc\n\
    ] if\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Jmp )\n\
    Jmp> | vm target |\n\
    vm &!pc target !\n\
    fetch Fetched> run\n\
  ]\n\
  ~[ ( Halt )\n\
    drop | vm |\n\
    vm vm-pop VmPop>\n\
    swap drop\n\
  ]\n\
  Op? ;\n\
: build ( -- [Op 13] )\n\
  Halt 13 fill | prog |\n\
  &!prog 0  >usize &!> 0  >usize Load  !\n\
  &!prog 1  >usize &!> 11 >usize Jz    !\n\
  &!prog 2  >usize &!> 1  >usize Load  !\n\
  &!prog 3  >usize &!> 0  >usize Load  !\n\
  &!prog 4  >usize &!> Add !\n\
  &!prog 5  >usize &!> 1  >usize Store !\n\
  &!prog 6  >usize &!> 0  >usize Load  !\n\
  &!prog 7  >usize &!> 1 Push !\n\
  &!prog 8  >usize &!> Sub !\n\
  &!prog 9  >usize &!> 0  >usize Store !\n\
  &!prog 10 >usize &!> 0  >usize Jmp   !\n\
  &!prog 11 >usize &!> 1  >usize Load  !\n\
  prog ;\n\
: main ( -- )\n\
  build\n\
  0 >usize\n\
  0 8 fill\n\
  0 >usize\n\
  0 4 fill | mem |\n\
  &!mem 0 >usize &!> 10 !\n\
  mem\n\
  Vm\n\
  fetch Fetched> run . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-vm-small-n-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "55\n");
    assert_eq!(code, 0);
}

#[test]
fn vm_dispatch_loop_runs_in_constant_stack() {
    // Phase 4 of Slice 7 (criterion 3): the committed `examples/vm.sth` runs
    // its sum-1..N program at N = 100_000. The loop body is 11 opcodes
    // (`Load Jz Load Load Add Store Load Push Sub Store Jmp`), so 100_000
    // trips execute ~1_100_000 dispatch steps, clearing the >=1_000_000-
    // dispatch-step rule (Slice 6's constant-stack rule applied at the
    // dispatch level). `run`'s self-tail-call keeps this in constant stack;
    // naive recursion at this depth would overflow the default 8MB host
    // stack, which `run_and_capture_stdout` catches as a signal death (no
    // exit code), turning a no-op Slice 6 transform red.
    let (stdout, code) = run_and_capture_stdout("examples/vm.sth");
    assert_eq!(stdout, "5000050000\n");
    assert_eq!(code, 0);
}

#[test]
fn non_tail_factorial_still_a_real_call_native() {
    // Criterion 5: the existing `examples/factorial.sth` (`dup 1 sub factorial
    // mul`) has a self-call followed by `mul`, so it is deliberately not in tail
    // position and stays a real, un-eliminated `Call` (R10); it still
    // computes correctly at small N. The over-eager-miscompile boundary
    // (self-call inside a non-terminal `if`) is covered by the
    // `self_call_in_non_terminal_if_stays_a_call` unit test in `src/ir.rs`.
    let (stdout, code) = run_and_capture_stdout("examples/factorial.sth");
    assert_eq!(stdout, "120\n");
    assert_eq!(code, 0);
}

// Phase 3 Slice 1: the linear core on bare `Spy` values. Every
// drop-observing golden compares the *whole* stdout, so "exactly once" and drop
// order are actually proven; every negative golden asserts the diagnostic and
// the backticked type name.

fn run_linear_golden(tag: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-linear-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(code, 0, "linear golden `{tag}` should exit 0");
    stdout
}

fn linear_check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

/// The Phase 3 Slice 1 linear-mechanics stand-in, retired as a compiler
/// primitive in Slice 8c: an ordinary one-field struct with a `drop`
/// overload, so it is linear for the same reason any resource is, not by
/// any compiler-known bit. Two lines, so every line number in a source
/// string it is prepended to shifts up by 2.
const SPY_DEF: &str = "type: Spy tag i64 ;\n: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n";

#[test]
fn dup_of_linear_value_is_error() {
    // Criterion 1: `dup` is the explicit copy and a linear value has no copy.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: main ( -- )\n  7 Spy dup drop drop ;\n"
    ));
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(err.contains("linear"), "unexpected message: {err}");
}

#[test]
fn over_of_linear_value_is_error() {
    // Criterion 1b: `over` copies its second slot, so it is gated too.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: main ( -- )\n  7 Spy 1 over drop drop drop ;\n"
    ));
    assert!(err.contains("cannot `over`"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn use_after_move_of_linear_local_is_error() {
    // Criterion 2: the second mention errors and names the site of the first.
    // `SPY_DEF` is two lines, so `hold`'s own line 3 (the first `s drop`)
    // lands on line 7.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: main ( -- )\n  7 Spy hold ;\n\
: hold ( Spy -- )\n  | s |\n  s drop\n  s drop ;\n"
    ));
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(
        err.contains("moved at line 7, col 3"),
        "the diagnostic should name the move site: {err}"
    );
}

#[test]
fn explicit_drop_runs_destructor_once() {
    // Criterion 3: the destructor runs exactly once, exactly where the `drop`
    // is written (between the two ordinary prints).
    let stdout = run_linear_golden(
        "drop-once",
        &format!("{SPY_DEF}: main ( -- )\n  1 .\n  7 Spy drop\n  2 . ;\n"),
    );
    assert_eq!(stdout, "1\ndrop 7\n2\n");
}

#[test]
fn surplus_linear_on_stack_is_error() {
    // Criterion 4a: forgetting is an error, not a silent drop.
    let err = linear_check_error(&format!("{SPY_DEF}: main ( -- )\n  7 Spy ;\n"));
    assert!(
        err.contains("linear value left on the stack"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn unconsumed_linear_local_is_error() {
    // Criterion 4b: a linear local never consumed by scope end. Locals are not
    // on the final stack, so this is its own pass, not `check_outputs`.
    let err = linear_check_error(&format!("{SPY_DEF}: hold ( Spy -- )\n  | s |\n  1 . ;\n"));
    assert!(err.contains("never consumed"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(
        err.contains("`s`"),
        "the error should name the local: {err}"
    );
}

#[test]
fn surplus_copy_value_keeps_existing_error() {
    // Criterion 4c: no misfire. A surplus Copy value keeps the arity error it
    // always had, with no linear wording.
    let err = linear_check_error(": main ( -- )\n  1 ;\n");
    assert!(
        err.contains("body leaves 1 values"),
        "unexpected message: {err}"
    );
    assert!(!err.contains("linear"), "unexpected message: {err}");
}

#[test]
fn swap_of_linear_values_is_allowed() {
    // Criterion `swap`: the pure reorderings move rather than copy, so the
    // `dup`/`over` gate must not reach them. The drop order proves the reorder
    // actually happened: `swap` makes 7 the top, and `rot` (a b c -> b c a)
    // makes 1 the top.
    let stdout = run_linear_golden(
        "reorder",
        &format!(
            "{SPY_DEF}: main ( -- )\n  7 Spy 8 Spy swap drop drop\n\
  1 Spy 2 Spy 3 Spy rot drop drop drop ;\n"
        ),
    );
    assert_eq!(stdout, "drop 7\ndrop 8\ndrop 1\ndrop 3\ndrop 2\n");
}

#[test]
fn both_arms_consume_linear_ok() {
    // Criterion 10a: consumed in both arms compiles, and each call disposes its
    // own spy exactly once.
    let stdout = run_linear_golden(
        "both-arms",
        &format!(
            "{SPY_DEF}: dispose ( Spy Bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 99 . s drop ] if ;\n\
: main ( -- )\n  7 Spy True dispose\n  8 Spy False dispose ;\n"
        ),
    );
    assert_eq!(stdout, "drop 7\n99\ndrop 8\n");
}

#[test]
fn divergent_arm_use_is_error() {
    // Criterion 10b: consumed in one arm only, then referenced past the join.
    // The join yields `MaybeMoved`, so the later use is a use-after-move.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: oops ( Spy Bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 1 . ] if\n  s drop ;\n"
    ));
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn divergent_arm_unconsumed_is_error() {
    // Criterion 10c: consumed in one arm only and never referenced again. The
    // compiler errors at scope end rather than inserting a compensating drop.
    // `s` WAS consumed on the `then` arm, so the diagnostic must not claim it
    // was never touched: the bug is the `else` arm forgetting it.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: oops ( Spy Bool -- )\n  | s c |\n  c ~[ s drop ] ~[ 1 . ] if ;\n"
    ));
    assert!(
        err.contains("not consumed on every path"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn linear_across_loop_back_edge_is_located_error() {
    // Criterion 12: a linear value live across the self-tail-call back-edge is
    // deferred (R15/D8), as a located error rather than a miscompile. `SPY_DEF`
    // is two lines, so `spin`'s own line 3 lands on line 5.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: spin ( Spy i64 -- i64 )\n  | s n |\n\
  n 0 eq ~[ s drop 0 ] ~[ 9 Spy n 1 sub spin ] if ;\n"
    ));
    assert!(
        err.contains("not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(err.contains("line 5"), "the error should be located: {err}");
}

#[test]
fn copy_loop_still_compiles() {
    // Criterion 12 (other half): a `countdown`-shaped Copy loop is unaffected
    // by the back-edge guard.
    let stdout = run_linear_golden(
        "copy-loop",
        ": countdown ( i64 -- i64 )\n  | n |\n  n 0 eq ~[ 0 ] ~[ n 1 sub countdown ] if ;\n\
: main ( -- )\n  100 countdown . ;\n",
    );
    assert_eq!(stdout, "0\n");
}

// Phase 3 Slice 1, Phase 2: struct aggregates via destructure-whole. A struct
// is linear iff any field is (transitively); `drop` on a linear struct runs
// compiler-synthesized field drop glue. Every drop-observing golden compares
// the *whole* stdout, so drop count and order are proven, not just "it
// compiled".

#[test]
fn destructure_whole_drops_each_field() {
    // Criterion 5: `S>` a struct of two distinctly-tagged spies pushes both
    // fields (first field deepest), and dropping them top-first proves the
    // destructure moved both fields out rather than just the top one.
    let stdout = run_linear_golden(
        "destructure-whole",
        &format!(
            "{SPY_DEF}type: Pair a Spy b Spy ;\n\
: main ( -- )\n  1 Spy 2 Spy Pair\n  Pair> drop drop ;\n"
        ),
    );
    assert_eq!(stdout, "drop 2\ndrop 1\n");
}

#[test]
fn nested_struct_is_linear_transitively() {
    // Criterion 5b: a struct-of-struct-of-spy is linear too. `dup` is
    // rejected exactly like a bare spy, naming the outer struct type, proving
    // linearity propagates through a nested aggregate rather than stopping at
    // the immediate field.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Inner v Spy ;\ntype: Outer i Inner ;\n\
: main ( -- )\n  5 Spy Inner Outer dup\n  Outer> Inner> drop\n  Outer> Inner> drop ;\n"
    ));
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Outer`"), "unexpected message: {err}");

    // And once actually consumed exactly once, the nested destructure/drop
    // chain runs correctly end to end.
    let stdout = run_linear_golden(
        "nested-struct",
        &format!(
            "{SPY_DEF}type: Inner v Spy ;\ntype: Outer i Inner ;\n\
: main ( -- )\n  5 Spy Inner Outer\n  Outer> Inner> drop ;\n"
        ),
    );
    assert_eq!(stdout, "drop 5\n");
}

#[test]
fn destructure_extracts_a_field_with_no_implicit_disposal() {
    // P7 slice 1 (D3/R9 deletion guard): `S>` destructure moves every field
    // out onto the stack with no implicit disposal of its own, so ordering
    // is exactly what the program writes (here `b` first, since it is
    // destructured deeper but consumed first). This golden alone does not
    // discriminate a reinstated sibling-drop: `S>`'s destructure of a
    // freshly built struct never had one to reinstate. The guard that does
    // catch a reinstated implicit disposal is
    // `projection_read_of_copy_field_keeps_struct`, whose non-consuming `&a`
    // read is a real sibling-drop hazard: it fails if `b` is dropped early.
    let stdout = run_linear_golden(
        "destructure-no-implicit-drop",
        &format!(
            "{SPY_DEF}type: Pair a Spy b Spy ;\n\
: main ( -- )\n  1 Spy 2 Spy Pair\n  Pair> drop drop ;\n"
        ),
    );
    assert_eq!(stdout, "drop 2\ndrop 1\n");
}

#[test]
fn store_over_a_linear_field_through_a_reference_is_error() {
    // P7 slice 1 (D3/R11 deletion guard): the retired `S<fi` setter used to
    // drop the field it overwrote. Its replacement, `&!f v !`, has no such
    // glue to reinstate -- `!` already refuses to store over a linear
    // referent outright (the value being overwritten would otherwise leak
    // with nothing to drop it), so this guard is a diagnostic assertion, not
    // a drop-count golden.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Pair a Spy b Spy ;\n\
: main ( -- )\n  1 Spy 2 Spy Pair\n  &!a 9 Spy !\n  Pair> drop drop ;\n"
    ));
    assert!(
        err.contains("`!` cannot access the linear referent `Spy`"),
        "unexpected message: {err}"
    );
}

#[test]
fn drop_of_linear_struct_runs_field_glue_in_declaration_order() {
    // Criterion 13: `drop` on the whole struct (no destructure in sight) runs
    // the synthesized destructor, which drops fields in declaration order
    // (`a` tag 1, then `b` tag 2) — not stack/reverse order, proving the glue
    // is field-order-driven, not a generic "drop whatever's on the stack".
    let stdout = run_linear_golden(
        "drop-whole-struct",
        &format!("{SPY_DEF}type: Pair a Spy b Spy ;\n: main ( -- )\n  1 Spy 2 Spy Pair drop ;\n"),
    );
    assert_eq!(stdout, "drop 1\ndrop 2\n");
}

#[test]
fn drop_of_nested_linear_struct_recurses_into_the_synthesized_destructor() {
    // Criterion 5b + 13 combined: `drop` on the *outer* struct's own synthesized
    // destructor must itself call `sooth_struct_drop_Inner` for the nested field,
    // rather than only handling one level of nesting. Field order is `i` (tag 1),
    // then `Inner` (whose own destructor drops `z`, tag 3) is inside `Outer`
    // at declaration order after a scalar sibling `n` (tag 2), proving the
    // recursion isn't just "the nested struct happens to be first".
    let stdout = run_linear_golden(
        "drop-whole-nested-struct",
        &format!(
            "{SPY_DEF}type: Inner z Spy ;\ntype: Outer i Spy n Spy w Inner ;\n\
: main ( -- )\n  1 Spy 2 Spy 3 Spy Inner Outer drop ;\n"
        ),
    );
    assert_eq!(stdout, "drop 1\ndrop 2\ndrop 3\n");
}

// Phase 3 Slice 1, Phase 3: the non-consuming read, `&f @`. Projecting a
// linear field is legal; moving one out through `@` is a compile error
// (workaround: `S>`).

#[test]
fn projection_read_of_copy_field_keeps_struct() {
    // D2: `&a @` (the retired `Pair|>a` peek's replacement) is non-consuming,
    // leaving the aggregate itself live both times (proven because the final
    // `drop` of the whole struct still finds its linear field `b` intact and
    // disposes it exactly once -- a consuming read in its place would have
    // dropped `b` at the first read, or left nothing for the trailing `drop`
    // to see).
    let stdout = run_linear_golden(
        "projection-read-copy-field",
        &format!(
            "{SPY_DEF}type: Pair a i64 b Spy ;\n\
: main ( -- )\n  5 3 Spy Pair\n  &a @ drop\n  &a @ drop\n  drop ;\n"
        ),
    );
    assert_eq!(stdout, "drop 3\n");
}

#[test]
fn projection_read_of_linear_field_is_error() {
    // D2 (verified 2026-08-17): producing `&b` off a linear field is legal --
    // a projection borrows rather than duplicates. The rejection moves to
    // `@`, which refuses to read a linear referent, symmetric to `!`'s
    // refusal to store over one.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Pair a i64 b Spy ;\n: main ( -- )\n  5 3 Spy Pair\n  &b @ drop drop ;\n"
    ));
    assert!(err.contains("`@`"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

// Phase 3 Slice 1, Phase 4: enums via a synthesized, tag-dispatched
// destructor. A linear enum's `drop` doesn't know at compile time which
// variant is active, so the synthesized destructor tests the runtime tag and
// drops only the active variant's linear payload.

#[test]
fn dup_of_linear_enum_is_error() {
    // The hole Phase 2 left open: `is_copy` used to return `True` for every
    // enum, so an enum with a linear payload was silently duplicable and this
    // exact source compiled, ran, and printed nothing (an exactly-once
    // violation with no diagnostic). It is now rejected like any other linear
    // value, naming the enum type.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Box | Full v Spy | Empty ;\n\
: main ( -- )\n  1 Spy Full dup drop drop ;\n"
    ));
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Box`"), "unexpected message: {err}");
    assert!(err.contains("linear"), "unexpected message: {err}");
}

#[test]
fn nested_enum_is_linear_transitively() {
    // The enum half of criterion 5b: linearity propagates through a
    // struct-of-enum-of-struct-of-spy, so `dup` on the outer struct is
    // rejected naming `Wrap`, and dropping it whole runs the chain of
    // synthesized destructors (struct -> tag dispatch -> struct -> spy).
    let src = format!(
        "{SPY_DEF}type: Inner v Spy ;\ntype: Held | Empty | Some i Inner ;\n\
type: Wrap h Held ;\n"
    );
    let err = linear_check_error(&format!(
        "{src}: main ( -- )\n  5 Spy Inner Some Wrap dup drop drop ;\n"
    ));
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Wrap`"), "unexpected message: {err}");

    let stdout = run_linear_golden(
        "nested-enum",
        &format!("{src}: main ( -- )\n  5 Spy Inner Some Wrap drop ;\n"),
    );
    assert_eq!(stdout, "drop 5\n");
}

#[test]
fn drop_of_linear_enum_dispatches_on_tag() {
    // Criterion 9: a linear enum built behind an `if` (so its active variant
    // is a runtime fact, not something lowering can fold), dropped whole
    // (never destructured/matched). The synthesized destructor's tag dispatch
    // must find the right variant each time: the `Full` branch's spy payload
    // is dropped, the `Empty` branch's `drop` is a no-op, and the surrounding
    // prints prove the dispatch didn't run the wrong arm's glue (or both).
    let stdout = run_linear_golden(
        "enum-tag-dispatch",
        &format!(
            "{SPY_DEF}type: Item | Empty | Full v Spy ;\n\
: main ( -- )\n  1 .\n  True ~[ 5 Spy Full ] ~[ Empty ] if drop\n  2 .\n\
  False ~[ 9 Spy Full ] ~[ Empty ] if drop\n  3 . ;\n"
        ),
    );
    assert_eq!(stdout, "1\ndrop 5\n2\n3\n");
}

#[test]
fn eliminator_arm_disposes_linear_payload() {
    // Criterion 9b: destructuring the narrowed variant exposes its payload on
    // the stack; the arm is responsible for disposing it like any other linear
    // value (here via a bare `drop`, no local name needed). The `Empty` arm
    // disposes only its own payload-free receiver, proving the second `drop`
    // in the `Full` arm is the arm body's own doing, not compiler-inserted
    // compensation.
    let stdout = run_linear_golden(
        "eliminator-disposes-payload",
        &format!(
            "{SPY_DEF}type: Item | Empty | Full v Spy ;\n\
: handle ( Item -- )\n  ~[ ( Empty ) drop 99 . ]\n  ~[ ( Full ) Full> drop ]\n  Item? ;\n\
: main ( -- )\n  Empty handle\n  7 Spy Full handle ;\n"
        ),
    );
    assert_eq!(stdout, "99\ndrop 7\n");
}

#[test]
fn unconsumed_linear_eliminator_payload_is_error() {
    // The arm-body half of criterion 9b: a payload bound to an arm-body local
    // is subject to the same scope-end rule as any other linear local, so
    // forgetting it is a compile error naming the local and the word. This is
    // the branch Phase 1 left unreachable (a linear variant payload needed the
    // enum linearity this phase adds).
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Item | Empty | Full v Spy ;\n\
: handle ( Item -- )\n  ~[ ( Empty ) drop 99 . ]\n  ~[ ( Full ) Full> | s | 1 . ]\n  Item? ;\n"
    ));
    assert!(err.contains("never consumed"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(
        err.contains("`s`"),
        "the error should name the local: {err}"
    );
    assert!(
        err.contains("`handle`"),
        "the error should name the word: {err}"
    );
}

#[test]
fn duplicate_word_entry_local_is_error() {
    // A repeated binding name (`| s s |`) must not collapse to last-wins in
    // the name -> type map: that would silently drop the earlier binding
    // (and any linear value in it) from all tracking, with no diagnostic.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: hold ( Spy Spy -- )\n  | s s |\n  s drop ;\n\
: main ( -- ) 1 Spy 2 Spy hold 99 . ;\n"
    ));
    assert!(err.contains("duplicate local"), "unexpected message: {err}");
    assert!(
        err.contains("`s`"),
        "the error should name the duplicated local: {err}"
    );
    assert!(
        err.contains("`hold`"),
        "the error should name the word: {err}"
    );
}

#[test]
fn duplicate_eliminator_arm_local_is_error() {
    // The arm-body twin of `duplicate_word_entry_local_is_error`: the same
    // last-wins hazard exists in an arm's own `| s s |` binding path.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: R | Two a Spy b Spy ;\n\
: use ( R -- ) ~[ ( Two ) Two> | s s | s drop ] R? ;\n\
: main ( -- ) 1 Spy 2 Spy Two use ;\n"
    ));
    assert!(err.contains("duplicate local"), "unexpected message: {err}");
    assert!(
        err.contains("`s`"),
        "the error should name the duplicated local: {err}"
    );
    assert!(
        err.contains("`use`"),
        "the error should name the word: {err}"
    );
}

#[test]
fn main_declaring_linear_output_is_error() {
    // Nothing calls `main`, so a linear output would leak past the program
    // boundary unnoticed instead of being disposed.
    let err = linear_check_error(&format!("{SPY_DEF}: main ( -- Spy ) 7 Spy ;\n"));
    assert!(
        err.contains("cannot declare a linear type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(err.contains("`main`"), "unexpected message: {err}");
}

#[test]
fn main_declaring_linear_input_is_error() {
    // Nothing calls `main`, so a linear input arrives in an uninitialised
    // ABI register; running its destructor would be undefined behaviour.
    let err = linear_check_error(&format!("{SPY_DEF}: main ( Spy -- ) | s | s drop ;\n"));
    assert!(
        err.contains("cannot declare a linear type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
    assert!(err.contains("`main`"), "unexpected message: {err}");
}

#[test]
fn fill_of_linear_element_is_error() {
    // Array-element linearity isn't tracked transitively by the drop-glue
    // path yet, so `fill` rejects a linear element outright.
    let err = linear_check_error(&format!("{SPY_DEF}: main ( -- )\n  0 Spy 3 fill drop ;\n"));
    assert!(
        err.contains("linear array elements are not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn linear_array_element_in_word_signature_is_error() {
    // The array-type boundary (not just `fill`) rejects a linear element: a
    // `[Spy 2]` slot in a word's stack effect names the type directly. The
    // parser cannot know `Spy` is linear until struct/enum fields are
    // resolved, so this is a checker error, not a parse error.
    let err = linear_check_error(&format!(
        "{SPY_DEF}: w ( [Spy 2] -- )\n  | a | a drop ;\n: main ( -- ) 0 . ;\n"
    ));
    assert!(
        err.contains("linear array elements are not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn linear_array_element_in_struct_field_is_error() {
    // Same boundary, reached via a `type:` field declaration instead of a
    // word signature.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Bag xs [Spy 2] ;\n: main ( -- ) 0 . ;\n"
    ));
    assert!(
        err.contains("linear array elements are not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn linear_array_element_via_linear_struct_in_struct_field_is_error() {
    // Indirect linearity: `Arr`'s field isn't itself `Spy`, but `Holds`
    // (its element) contains one transitively, so the array is linear too.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Holds s Spy ;\ntype: Arr a [Holds 2] ;\n: main ( -- ) 0 . ;\n"
    ));
    assert!(
        err.contains("linear array elements are not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Holds`"), "unexpected message: {err}");
}

#[test]
fn linear_array_element_via_linear_struct_in_word_signature_is_error() {
    // Same indirection, reached via a word signature slot instead of a
    // struct field.
    let err = linear_check_error(&format!(
        "{SPY_DEF}type: Holds s Spy ;\n: w ( [Holds 2] -- )\n  | a | a drop ;\n: main ( -- ) 0 . ;\n"
    ));
    assert!(
        err.contains("linear array elements are not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`Holds`"), "unexpected message: {err}");
}

// Phase 3 Slice 2: the three owning-cell access words (`^ ^> ^|>`). A
// runtime golden uses a temp `.sth` file exactly like the linear-core section
// above; a trace-observing golden additionally sets `SOOTH_TRACE_ALLOC` via
// `run_and_capture_traced_stdout`. Phase 3 constructs and unwraps a cell but
// never `drop`s one (that's Phase 4's drop-glue arm), so every golden here
// disposes via `^>`.

fn run_owned_golden(tag: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-owned-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(code, 0, "owned-cell golden `{tag}` should exit 0");
    stdout
}

fn run_owned_traced_golden(tag: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-owned-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_traced_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(code, 0, "owned-cell golden `{tag}` should exit 0");
    stdout
}

#[test]
fn unconsumed_owned_is_error() {
    // Criterion 2: forgetting to dispose a cell is a compile error, exactly
    // like a bare linear value (R4: `^T` is always linear).
    let err = linear_check_error(": main ( -- )\n  5 ^ ;\n");
    assert!(
        err.contains("linear value left on the stack"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`^i64`"), "unexpected message: {err}");
}

#[test]
fn dup_of_owned_is_error() {
    // Criterion 3: `dup` of `^i64` errors even though the *payload* (`i64`)
    // is Copy, proving the cell itself is linear regardless of what it holds
    // (R4).
    let err = linear_check_error(": main ( -- )\n  5 ^ dup ;\n");
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`^i64`"), "unexpected message: {err}");
}

#[test]
fn over_of_owned_is_error() {
    // Criterion 3b: `over` copies its second slot, gated the same way.
    let err = linear_check_error(": main ( -- )\n  5 ^ 1 over ;\n");
    assert!(err.contains("cannot `over`"), "unexpected message: {err}");
    assert!(err.contains("`^i64`"), "unexpected message: {err}");
}

#[test]
fn use_after_move_of_owned_is_error() {
    // Criterion 4: the second mention errors and names the site of the
    // first, mirroring the bare linear-value case.
    let err = linear_check_error(
        ": main ( -- )\n  5 ^ hold ;\n\
: hold ( ^i64 -- )\n  | s |\n  s ^> drop\n  s ^> drop ;\n",
    );
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains("`^i64`"), "unexpected message: {err}");
    assert!(
        err.contains("moved at line 5, col 3"),
        "the diagnostic should name the move site: {err}"
    );
}

#[test]
fn owned_unwrap_returns_payload_and_frees_once() {
    // Criterion 5: unwrap returns the payload value; the transcript is
    // exactly one `alloc` (construct) then one `free` (unwrap) at the scalar
    // `i64` payload's 8-byte size.
    let stdout = run_owned_traced_golden("unwrap-scalar", ": main ( -- )\n  5 ^ ^> . ;\n");
    assert_eq!(stdout, "alloc 8\nfree 8\n5\n");
}

#[test]
fn owned_unwrap_sub_word_scalar_is_width_exact() {
    // R13: a sub-word payload's `FieldLoad`/`FieldStore` is width-exact, not
    // padded to a word; `^u8` allocates and frees exactly 1 byte, unlike the
    // 8-byte `^i64` case above.
    let stdout = run_owned_traced_golden("unwrap-u8", ": main ( -- )\n  200 >u8 ^ ^> . ;\n");
    assert_eq!(stdout, "alloc 1\nfree 1\n200\n");
}

#[test]
fn owned_unwrap_aggregate_copies_out_before_free() {
    // Criterion 5b: unwrap materialises an aggregate payload before releasing
    // the cell (R13); a bare field read right after the free could pass by
    // luck (whether the allocator's free() bookkeeping happens to clobber the
    // read offset). Interposing a second same-size allocation between the
    // free and the read forces the issue: glibc's tcache reuses a freed
    // block LIFO within its size class, so if unwrap had aliased the cell
    // instead of copying out, this second `Point`-sized alloc would
    // deterministically clobber it before `p` is read.
    let stdout = run_owned_golden(
        "unwrap-aggregate",
        "type: Point x i64 y i64 ;\n\
: use ( Point -- )\n  \
  | p |\n  \
  3 4 Point ^ ^> drop\n  \
  &p &y @ . ;\n\
: main ( -- )\n  1 2 Point ^ ^> use ;\n",
    );
    assert_eq!(stdout, "2\n");
}

#[test]
fn peek_owned_linear_payload_is_error() {
    // Criterion 7: `^|>` on a linear payload is a compile error naming the
    // payload's type (R11/R14); `^Spy` proves it via a linear payload.
    let err = linear_check_error(&format!("{SPY_DEF}: main ( -- )\n  7 Spy ^ ^|> ;\n"));
    assert!(err.contains("cannot `^|>`"), "unexpected message: {err}");
    assert!(err.contains("`Spy`"), "unexpected message: {err}");
}

#[test]
fn struct_containing_owned_is_linear() {
    // Criterion 9: a struct with a cell field is linear (R4 propagates
    // transitively via Slice 1's rules); `dup` on it errors naming the
    // struct, not the cell.
    let err = linear_check_error("type: Box v ^i64 ;\n: main ( -- )\n  5 ^ Box dup ;\n");
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Box`"), "unexpected message: {err}");
}

#[test]
fn alloc_trace_is_silent_when_unset() {
    // Criterion 18: the gate is off by default (unset); a program that
    // constructs and disposes a cell prints only its own output, none of the
    // trace. Without this, a regression inverting the gate ships green.
    let stdout = run_owned_golden("gate-off", ": main ( -- )\n  5 ^ ^> . ;\n");
    assert_eq!(stdout, "5\n");
}

// Phase 4: drop glue (`emit_drop`'s `OwnedCell` arm plus a synthesized
// per-cell destructor, R5/R8) and the allocation-observing goldens it makes
// possible. Every golden below is free to `drop` a cell, unlike Phase 3's.

fn run_owned_memory_bounded_golden(tag: &str, src: &str, limit_kb: u64) -> i32 {
    let path = std::env::temp_dir().join(format!("sooth-owned-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    std::fs::remove_file(&path).ok();
    // Gate off (the env var is removed, not just unset in this process)
    // under a `limit_kb` `RLIMIT_AS` (`ulimit -v`, in KB): repeated leaked
    // memory grows unbounded across iterations and necessarily trips the
    // limit, while a genuine free-per-iteration loop stays comfortably
    // within it. Unlike criterion 14's OOM trap, this doesn't need to
    // distinguish a NULL `malloc` from a live one, only survive; see the
    // spec's "why criterion 14 is not a runtime golden" for why that
    // distinction specifically is unsound to probe at runtime.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -v {limit_kb} && exec \"{}\"",
            binary.display()
        ))
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .status()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    status
        .code()
        .expect("process should exit normally, not die by signal")
}

#[test]
fn owned_alloc_and_drop_traces_one_pair() {
    // Criterion 1: construct then `drop` (not `^>`): the transcript is
    // exactly one `alloc` and one `free`, at the `i64` payload's 8-byte size.
    let stdout = run_owned_traced_golden("drop-scalar", ": main ( -- )\n  5 ^ drop ;\n");
    assert_eq!(stdout, "alloc 8\nfree 8\n");
}

#[test]
fn owned_alloc_dispose_loop_stays_within_memory_bound() {
    // Criterion 1b: ~100k construct-and-dispose iterations of a `[u8 1024]`
    // cell under a 64 MB `RLIMIT_AS`, gate off. Mirrors `countdown.sth`'s
    // tail-call -> loop shape (constant stack), so the outer loop itself
    // never grows memory; only a broken `free` (a real leak, or a fake one)
    // would.
    let src = ": loop-owned ( i64 -- )\n  dup 0 eq ~[\n    drop\n  ] ~[\n    0 >u8 1024 fill ^ drop\n    1 sub loop-owned\n  ] if ;\n\
: main ( -- )\n  100000 loop-owned ;\n";
    let code = run_owned_memory_bounded_golden("mem-bound", src, 65536);
    assert_eq!(
        code, 0,
        "a real free-per-iteration loop should stay within the 64 MB bound"
    );
}

#[test]
fn peek_owned_copy_payload_keeps_cell_live() {
    // Criterion 6: peek twice then dispose; the first peeked value is printed
    // (pinning it against garbage, not just against the second peek), the two
    // peeks are then asserted equal (proving the cell stayed live and
    // unchanged across them), and the transcript shows exactly one `free`.
    let stdout = run_owned_traced_golden(
        "peek-twice",
        ": main ( -- )\n  5 ^ ^|> swap ^|> rot dup . eq . drop ;\n",
    );
    assert_eq!(stdout, "alloc 8\n5\nTrue\nfree 8\n");
}

#[test]
fn owned_linear_payload_frees_before_dropping_payload() {
    // Criterion 8: `^Spy` disposal. The transcript is `free 8` *then*
    // `drop 7`, one stdout stream so the order is real (R8: the cell frees
    // before the payload's own destructor runs).
    let stdout = run_owned_traced_golden(
        "spy-payload",
        &format!("{SPY_DEF}: main ( -- )\n  7 Spy ^ drop ;\n"),
    );
    assert_eq!(stdout, "alloc 8\nfree 8\ndrop 7\n");
}

#[test]
fn owned_aggregate_payload_frees_before_dropping_fields() {
    // Criterion 6: R8 ordering for an aggregate payload (a struct with a
    // linear field) held in a cell, the `Blit`-into-a-frame-slot arm of
    // `load_owned_payload`, where freeing early is most likely to bite. The
    // cell frees (`free 16`) before the copied-out struct's own destructor
    // drops its linear field (`drop 1`).
    let stdout = run_owned_traced_golden(
        "aggregate-payload",
        &format!("{SPY_DEF}type: Holds a Spy b i64 ;\n: main ( -- )\n  1 Spy 2 Holds ^ drop ;\n"),
    );
    assert_eq!(stdout, "alloc 16\nfree 16\ndrop 1\n");
}

#[test]
fn dropping_struct_with_owned_frees_cell() {
    // Criterion 9b: dropping a struct containing a cell field frees the
    // cell (the struct destructor's synthesized drop of its linear field).
    let stdout = run_owned_traced_golden(
        "struct-with-cell",
        "type: Box v ^i64 ;\n: main ( -- )\n  5 ^ Box drop ;\n",
    );
    assert_eq!(stdout, "alloc 8\nfree 8\n");
}

#[test]
fn enum_variant_with_owned_frees_on_drop() {
    // Criterion 10: an enum variant carrying a cell, built behind an `if` so
    // the active variant is a runtime fact, not a compile-time one. Dropping
    // the cell-carrying variant frees exactly once; dropping the *other*
    // variant frees zero times. Both are asserted in one transcript: only
    // one alloc/free pair appears, from the `Full` branch.
    let stdout = run_owned_traced_golden(
        "enum-variant",
        "type: Item | Empty | Full v ^i64 ;\n\
: main ( -- )\n  True ~[ 5 ^ Full ] ~[ Empty ] if drop\n  \
False ~[ 9 ^ Full ] ~[ Empty ] if drop ;\n",
    );
    assert_eq!(stdout, "alloc 8\nfree 8\n");
}

#[test]
fn nested_owned_frees_outer_before_inner() {
    // Criterion 11: `^^[u8 24]`. The inner and outer sizes are deliberately
    // distinct (24 vs. the pointer-width 8) so the transcript order proves
    // the outer cell frees *before* the inner one (R8); equal sizes could not
    // distinguish that from the reverse.
    let stdout = run_owned_traced_golden("nested", ": main ( -- )\n  0 >u8 24 fill ^ ^ drop ;\n");
    assert_eq!(stdout, "alloc 24\nalloc 8\nfree 8\nfree 24\n");
}

#[test]
fn owned_zero_sized_payload_allocs_one_byte() {
    // Criterion 12: a zero-sized (`Unit`) payload. The transcript shows
    // `alloc 1`/`free 1`, witnessing R15's `max(size, 1)` adjustment;
    // asserting the *size* (not just a count) matters, since glibc's
    // `malloc(0)` returns non-NULL and would pass a count-only test even if
    // the adjustment were deleted.
    let stdout = run_owned_traced_golden(
        "zero-sized",
        "type: Unit ;\n: main ( -- )\n  Unit ^ drop ;\n",
    );
    assert_eq!(stdout, "alloc 1\nfree 1\n");
}

#[test]
fn owned_byte_buffer_peek_reads_and_frees_once() {
    // Criterion 13: `^[u8 N]` constructed from a filled array, peeked, a
    // byte read off the peeked copy through `&>`/`@`, then `drop`; exactly
    // one alloc/free.
    let stdout = run_owned_traced_golden(
        "byte-buffer",
        ": main ( -- )\n  7 >u8 4 fill ^ ^|> | arr |\n  &arr 0 &> @ .\n  drop ;\n",
    );
    assert_eq!(stdout, "alloc 4\n7\nfree 4\n");
}

#[test]
fn peek_aggregate_does_not_alias_cell() {
    // Criterion 13 (continued): peek an aggregate, dispose the cell, *then*
    // read the peeked copy. If `^|>` had aliased the cell instead of copying
    // out, the read after `drop` would see freed memory; reading the right
    // value after the free proves it didn't.
    let stdout = run_owned_traced_golden(
        "peek-no-alias",
        ": main ( -- )\n  9 >u8 4 fill ^ ^|> | arr |\n  drop\n  &arr 0 &> @ . ;\n",
    );
    assert_eq!(stdout, "alloc 4\nfree 4\n9\n");
}

#[test]
fn caret_field_suffix_is_unknown_word() {
    // Criterion 21 (R12b): `^>x` and `^|>x` lex as one word each and match
    // none of the three exact cell-word spellings, so they fall through to
    // the ordinary unknown-word error. This pins the exact-name matching only,
    // *not* R12b's arm-ordering clause, which no longer has two arms to order:
    // P7 slice 1 retired the fused struct peek family that clause was written
    // against, so nothing but the exact cell-word names can claim these.
    let err = linear_check_error(": main ( -- )\n  5 ^ ^>x ;\n");
    assert!(err.contains("unknown word"), "unexpected message: {err}");
    assert!(err.contains("^>x"), "unexpected message: {err}");

    let err = linear_check_error(": main ( -- )\n  5 ^ ^|>x ;\n");
    assert!(err.contains("unknown word"), "unexpected message: {err}");
    assert!(err.contains("^|>x"), "unexpected message: {err}");
}

// Phase 3 slice 3: recursion through `^` and the fused iterative destructor
// (R11-R16). A directly self-recursive type disposes in one loop over the
// reused frame slot instead of a `cell_drop`/`enum_drop` pair per node.

/// Like `run_owned_memory_bounded_golden`, but bounding the *stack* rather
/// than the address space: 1 MB via `ulimit -s`, the budget a per-node
/// recursive destructor blows long before a million nodes. Returns the exit
/// code, or `None` when the child died by signal (a stack overflow is a
/// SIGSEGV, which the shared `run_binary` would `expect` away as a panic
/// naming the wrong thing).
fn run_stack_bounded_golden(tag: &str, src: &str) -> Option<i32> {
    let path = std::env::temp_dir().join(format!("sooth-deep-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    std::fs::remove_file(&path).ok();
    // `exec` replaces the shell, so the child's signal death propagates as the
    // shell's own status rather than being flattened into an exit code.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -s 1024 && exec \"{}\"", binary.display()))
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .status()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    status.code()
}

/// A million-node list, built by a self-tail-recursive word (so building is
/// already constant-stack under Slice 6's TCO) and disposed by one `drop`.
const DEEP_LIST_SRC: &str = "type: List | Nil | Cons v i64 next ^List ;\n\
: build ( i64 List -- List )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub n acc ^ Cons build\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Nil build drop ;\n";

#[test]
fn deep_list_disposes_in_constant_stack() {
    // Criterion 8 (R11): a 1,000,000-node list disposes under a 1 MB stack.
    // Verified to fail before the fused loop existed (R21): the pre-change
    // compiler segfaults on this program at 1 MB, 8 MB and 64 MB alike, so
    // the pass cannot be vacuous.
    assert_eq!(
        run_stack_bounded_golden("list", DEEP_LIST_SRC),
        Some(0),
        "a 1M-node list should dispose in constant stack, not overflow it"
    );
}

#[test]
fn recursive_list_disposes_in_expected_order() {
    // Criterion 5: a list with a `Spy` per node builds, walks one node off
    // the front (printing its value, dropping its spy, unwrapping its tail),
    // and disposes the remainder through the loop. The trace is rooted at the
    // bare `List` value, not a `^List`, so no line of it depends on the cell
    // destructor's own free ordering.
    let stdout = run_owned_traced_golden(
        "recursive-list",
        &format!(
            "{SPY_DEF}type: List | Nil | Cons v i64 tag Spy next ^List ;\n\
: push-front ( List i64 -- List )\n  \
  | rest v |\n  \
  v v Spy rest ^ Cons ;\n\
: step ( List -- List )\n\
  ~[ ( Nil )  drop Nil ]\n\
  ~[ ( Cons ) Cons> | v t n | v . t drop n ^> ]\n\
  List? ;\n\
: main ( -- )\n  \
  Nil 3 push-front 2 push-front 1 push-front\n  \
  step drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 32\nalloc 32\nalloc 32\n1\ndrop 1\nfree 32\ndrop 2\nfree 32\ndrop 3\nfree 32\n"
    );
}

#[test]
fn recursive_disposal_is_pre_order() {
    // Criterion 9 (R10): a node's own spy drops before the next node is even
    // reached, so the tags come out `1, 2, 3`. Post-order disposal (the
    // deepest node first, which is what the pre-slice compiler did before R8
    // reversed the cell ordering) would print `3, 2, 1`, and equal tags could
    // not tell the two apart.
    let stdout = run_owned_traced_golden(
        "pre-order",
        &format!(
            "{SPY_DEF}type: L | Nil | Cons tag Spy next ^L ;\n\
: push-front ( L i64 -- L )\n  \
  | rest v |\n  \
  v Spy rest ^ Cons ;\n\
: main ( -- )\n  \
  Nil 3 push-front 2 push-front 1 push-front drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\nalloc 24\ndrop 1\nfree 24\ndrop 2\nfree 24\ndrop 3\nfree 24\n"
    );
}

#[test]
fn recursive_destructor_reads_node_before_overwriting_slot() {
    // Criterion 10 (R12): the recursive field is declared *first*, with two
    // distinct spies after it. The loop reuses one frame slot, so the copy-out
    // of the next node overwrites the current one; emitting the fields in
    // declaration order instead would drop the Spy values of whatever node the
    // slot happened to hold, printing garbage tags (or repeating a node's)
    // while leaving the alloc/free trace perfectly balanced. Only the tags
    // catch it.
    let stdout = run_owned_traced_golden(
        "copyout-order",
        &format!(
            "{SPY_DEF}type: L | Nil | Cons next ^L a Spy b Spy ;\n\
: push-front ( L i64 -- L )\n  \
  | rest v |\n  \
  rest ^ v Spy v 1 add Spy Cons ;\n\
: main ( -- )\n  \
  Nil 5 push-front 3 push-front 1 push-front drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 32\nalloc 32\nalloc 32\n\
drop 1\ndrop 2\nfree 32\ndrop 3\ndrop 4\nfree 32\ndrop 5\ndrop 6\nfree 32\n"
    );
}

#[test]
fn non_recursive_cell_shapes_are_not_treated_as_recursive() {
    // Criterion 11 (R13/R15): three near misses for the detection pass, each
    // of which must keep straight-line synthesis. A False positive would blit
    // the enclosing type's bytes out of a cell that does not hold one, so the
    // spy tags and the sizes are both load-bearing: `Outer` holds a cell of a
    // *different* struct, `Twice` is a `^^Spy` whose inner payload is a cell
    // rather than the enclosing type (the `^^Self` near miss), and `Holder`
    // holds a cell of an unrelated enum.
    let stdout = run_owned_traced_golden(
        "near-miss",
        &format!(
            "{SPY_DEF}type: Inner t Spy ;\n\
type: Outer c ^Inner ;\n\
type: Twice c ^^Spy ;\n\
type: Payload | A t Spy | B ;\n\
type: Holder c ^Payload ;\n\
: main ( -- )\n  \
  1 Spy Inner ^ Outer drop\n  \
  2 Spy ^ ^ Twice drop\n  \
  3 Spy A ^ Holder drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 8\nfree 8\ndrop 1\n\
alloc 8\nalloc 8\nfree 8\nfree 8\ndrop 2\n\
alloc 16\nfree 16\ndrop 3\n"
    );
}

#[test]
fn recursive_disposal_path_backtracks_past_a_misleading_last_field() {
    // Phase 3 Slice 4, criterion 1's runtime counterpart (R3): `Node`
    // declares its dead-end cell field (`bait`, into `Leafy` via `Bait`,
    // never reaching `Node`) *after* its genuine recursive field (`good`),
    // so the reverse-declaration-order scan tries `bait` first, must walk
    // into it and fail, and only then backtrack to `good`. A greedy search
    // that committed to the first cell-typed field it saw would either loop
    // forever descending `Bait`/`Leafy` or simply miss the real edge and fall
    // back to ordinary recursion; either way this golden's fused loop would
    // never fire and `deep_recursive_chain_disposes_within_bounded_memory`'s
    // sibling shapes are the ones that would actually catch the regression at
    // scale, but this small trace pins the *order* a False positive on `bait`
    // would scramble.
    let stdout = run_owned_traced_golden(
        "misleading-last-field",
        &format!(
            "{SPY_DEF}type: Leafy v i64 ;\n\
type: Bait c ^Leafy ;\n\
type: Node | End | More good ^Node bait ^Bait tag Spy ;\n\
: main ( -- )\n  \
  End ^\n  9 Leafy ^ Bait ^ 1 Spy More ^\n  9 Leafy ^ Bait ^ 2 Spy More\n  drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 32\nalloc 8\nalloc 8\nalloc 32\nalloc 8\nalloc 8\n\
free 8\nfree 8\ndrop 2\nfree 32\nfree 8\nfree 8\ndrop 1\nfree 32\n"
    );
}

#[test]
fn two_unrelated_self_recursive_types_dispose_independently() {
    // Phase 3 Slice 4, criterion 1's runtime counterpart (R3): each of two
    // structurally-unrelated self-recursive types finds and fuses its own
    // loop; distinct spy tags per type (odd for `R1`, even for `R2`) pin that
    // neither's disposal wanders into the other's fields.
    let stdout = run_owned_traced_golden(
        "two-unrelated-recursive",
        &format!(
            "{SPY_DEF}type: R1 | End1 | More1 tag Spy next ^R1 ;\n\
type: R2 | End2 | More2 tag Spy next ^R2 ;\n\
: main ( -- )\n  \
  3 Spy End1 ^ More1 1 Spy swap ^ More1 drop\n  \
  4 Spy End2 ^ More2 2 Spy swap ^ More2 drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\ndrop 1\nfree 24\ndrop 3\nfree 24\n\
alloc 24\nalloc 24\ndrop 2\nfree 24\ndrop 4\nfree 24\n"
    );
}

#[test]
fn self_recursive_struct_destructor_compiles() {
    // Criterion 15 (R16): a self-recursive struct is uninhabited (a `^` is
    // non-null, so building one needs one first), but a destructor is
    // synthesized for every declared type, so the declaration alone emits a
    // loop with no exit. That is a legal QBE function, but only if the
    // trailing `Ret` is skipped: an unconditional seal after the back-edge
    // emits a duplicate block label and QBE rejects the whole module.
    let stdout = run_owned_golden(
        "self-recursive-struct",
        &format!("{SPY_DEF}type: Cyclic v Spy next ^Cyclic ;\n: main ( -- )\n  0 . ;\n"),
    );
    assert_eq!(stdout, "0\n");
}

// Phase 3 slice 3, phase 5: multi-child and mutually recursive types (R17,
// R18), the documented depth limitations (R14), and the `list.sth` dogfood.
// A tree node's two `^` fields are both recursive edges (R13's detection
// still fires per field, not per type), so the reverse-declaration-order walk
// (R17) picks the *last* declared one for the loop and the rest fall back to
// an ordinary recursive drop call.

#[test]
fn recursive_tree_builds_and_disposes() {
    // Criterion 12: a 7-node perfect binary tree (tags 1..7, pre-order:
    // root=1, left subtree rooted at 2 with leaves 4/5, right subtree rooted
    // at 3 with leaves 6/7) builds and disposes with every tag distinct, so
    // no drop can be mistaken for a sibling's. R10's pre-order contract still
    // holds per node (a node's own tag drops before its children are
    // reached); the *last* field (`right`) is the looped one, so a node's
    // `left` subtree is fully disposed by an ordinary recursive call before
    // the loop steps to `right`.
    let stdout = run_owned_traced_golden(
        "tree-7",
        &format!(
            "{SPY_DEF}type: Tree | Leaf | Node tag Spy left ^Tree right ^Tree ;\n\
: main ( -- )\n  \
  1 Spy\n  \
  2 Spy\n    \
    4 Spy Leaf ^ Leaf ^ Node ^\n    \
    5 Spy Leaf ^ Leaf ^ Node ^\n    \
    Node ^\n  \
  3 Spy\n    \
    6 Spy Leaf ^ Leaf ^ Node ^\n    \
    7 Spy Leaf ^ Leaf ^ Node ^\n    \
    Node ^\n  \
  Node drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\n\
alloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\n\
drop 1\nfree 32\ndrop 2\nfree 32\ndrop 4\nfree 32\nfree 32\nfree 32\n\
drop 5\nfree 32\nfree 32\nfree 32\ndrop 3\nfree 32\ndrop 6\nfree 32\nfree 32\n\
free 32\ndrop 7\nfree 32\nfree 32\n"
    );
}

#[test]
fn multi_child_destructor_loops_on_last_recursive_field() {
    // Criterion 13a: a 3-node tree (root=1, left leaf-child=2, right
    // leaf-child=3) whose fields are declared `tag`, `left`, `right`. Looping
    // the *last* field (`right`) prints tags `1, 2, 3`: root's own tag drops,
    // then `left` (non-looped) is fully recursively disposed (tag 2), then
    // the loop frees root's own cell and steps to `right` (tag 3). Looping
    // the *first* field instead would recurse `right` before stepping to
    // `left`, printing `1, 3, 2` (R17 names this exact contrast).
    let stdout = run_owned_traced_golden(
        "tree-order",
        &format!(
            "{SPY_DEF}type: Tree | Leaf | Node tag Spy left ^Tree right ^Tree ;\n\
: main ( -- )\n  \
  1 Spy\n  \
  2 Spy Leaf ^ Leaf ^ Node ^\n  \
  3 Spy Leaf ^ Leaf ^ Node ^\n  \
  Node drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\nalloc 32\n\
drop 1\nfree 32\ndrop 2\nfree 32\nfree 32\nfree 32\ndrop 3\nfree 32\nfree 32\n"
    );
}

#[test]
fn deep_right_leaning_tree_disposes_in_constant_stack() {
    // Criterion 13b: 13a alone would also pass on a compiler that never
    // builds a multi-child loop at all (it only distinguishes traversal
    // order); a 1,000,000-node right-leaning tree disposing under a 1 MB
    // stack is what proves the loop exists, exactly as criterion 8 does for
    // the list. Each node's `left` is a fresh `Leaf` and `right` is the
    // growing chain, so the loop (on the last field, `right`) takes the deep
    // path and the non-looped `left` recursion never grows past depth 1.
    let src = "type: Tree | Leaf | Node left ^Tree right ^Tree ;\n\
: build ( i64 Tree -- Tree )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub Leaf ^ acc ^ Node build\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Leaf build drop ;\n";
    assert_eq!(
        run_stack_bounded_golden("right-tree", src),
        Some(0),
        "a 1M-node right-leaning tree should dispose in constant stack, not overflow it"
    );
}

#[test]
fn mutually_recursive_types_dispose_in_constant_stack() {
    // Criterion 14, half of it inverted by Phase 3 Slice 4. A small A/B
    // chain (base case in `A`, since an all-struct or all-linked pair with
    // no base variant would be uninhabited per R5) disposes with the same
    // trace as before, alternating tags between the two types: the
    // generalized loop walks the cycle in the same pre-order the recursive
    // path did. The deep chain, which Slice 3 asserted *must* overflow a 1 MB
    // stack, now exits 0 — one fused loop per participating type spans the
    // whole two-type cycle, which is Slice 4's point.
    let stdout = run_owned_traced_golden(
        "mutual-small",
        &format!(
            "{SPY_DEF}type: A | ANil | ACons tag Spy next ^B ;\n\
type: B | BNil | BCons tag Spy next ^A ;\n\
: build ( i64 A -- A )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub\n    \
    n Spy acc ^ BCons ^\n    \
    n Spy swap ACons\n    \
    build\n  \
  ] if ;\n\
: main ( -- )\n  3 ANil build drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\n\
drop 1\nfree 24\ndrop 1\nfree 24\ndrop 2\nfree 24\ndrop 2\nfree 24\n\
drop 3\nfree 24\ndrop 3\nfree 24\n"
    );

    let deep_src = format!(
        "{SPY_DEF}type: A | ANil | ACons tag Spy next ^B ;\n\
type: B | BNil | BCons tag Spy next ^A ;\n\
: build ( i64 A -- A )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub\n    \
    n Spy acc ^ BCons ^\n    \
    n Spy swap ACons\n    \
    build\n  \
  ] if ;\n\
: main ( -- )\n  300000 ANil build drop ;\n"
    );
    assert_eq!(
        run_stack_bounded_golden("mutual-deep", &deep_src),
        Some(0),
        "a mutually recursive chain should dispose in constant stack"
    );
}

#[test]
fn wrapper_indirection_disposes_in_constant_stack_left_leaning_tree_stays_depth_limited() {
    // Criterion 16, half of it inverted by Phase 3 Slice 4: indirection
    // through a wrapper struct is a route the generalized loop now walks, so
    // that list exits 0 at 1,000,000 nodes under a 1 MB stack. The
    // left-leaning tree still does not, and is now the sole surviving proof
    // that D1's one-edge narrowing held: a struct level picks exactly one
    // recursive field (the last), so a chain grown through the *first* stays
    // on the ordinary recursive path.
    let wrapper_src = "type: Wrap v i64 n ^List ;\n\
type: List | Nil | Cons w Wrap ;\n\
: build ( i64 List -- List )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub n acc ^ Wrap Cons build\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Nil build drop ;\n";
    assert_eq!(
        run_stack_bounded_golden("wrapper-list", wrapper_src),
        Some(0),
        "a wrapper-struct list should dispose in constant stack: the byval \
         hop is a step on the path, not a dead end"
    );

    let left_leaning_src = "type: Tree | Leaf | Node left ^Tree right ^Tree ;\n\
: build ( i64 Tree -- Tree )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub acc ^ Leaf ^ Node build\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Leaf build drop ;\n";
    assert_ne!(
        run_stack_bounded_golden("left-tree", left_leaning_src),
        Some(0),
        "a left-leaning tree should stay depth-limited: the loop takes the \
         last field (`right`), not the chain-growing first field (`left`)"
    );
}

// Phase 3 slice 4: the fused loop generalizes from a direct `^Self` field to
// a whole *path* back to the type, so a wrapper struct, a `^^Self`, and a
// multi-type cycle each dispose in one loop of their own. Criteria 2-4c and
// 8-9; the small-N traces below are correctness-preservation (all three
// shapes already produced them on the recursive path), so only the
// constant-stack goldens prove a loop exists at all.

#[test]
fn wrapper_struct_recursive_list_disposes_in_expected_order() {
    // Criterion 2: the `^List` sits inside `Wrap`, one byval hop off the
    // enum's variant, so the path is `Cons -> Project(w) -> Unwrap(next)`.
    // `Wrap` declares its cell field *before* its spy (R12), so emitting
    // `Wrap`'s fields in declaration order would free the next node's cell
    // before dropping this node's spy — the copy-out would then overwrite the
    // slot the spy is read from, printing a repeated or garbage tag with the
    // alloc/free trace still balanced.
    let stdout = run_owned_traced_golden(
        "wrapper-list-order",
        &format!(
            "{SPY_DEF}type: Wrap next ^List tag Spy ;\n\
type: List | Nil | Cons w Wrap ;\n\
: push-front ( List i64 -- List )\n  \
  | rest v |\n  \
  rest ^ v Spy Wrap Cons ;\n\
: main ( -- )\n  \
  Nil 3 push-front 2 push-front 1 push-front drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\nalloc 24\n\
drop 1\nfree 24\ndrop 2\nfree 24\ndrop 3\nfree 24\n"
    );
}

#[test]
fn double_cell_recursive_list_disposes_in_expected_order() {
    // Criterion 3: `^^L` is two `Unwrap` steps in one iteration, and only the
    // second is hazardous (R8): the first strips `^^L` to `^L`, a scalar
    // `FieldLoad` with no frame slot, while the second blits the whole node
    // into the reused slot. The cell field is declared before the spy, so a
    // declaration-order emission would drop the wrong node's spy. The two
    // free sizes differ (8 for the outer pointer cell, 24 for the node
    // itself), which pins that both unwraps free their own cell exactly once.
    let stdout = run_owned_traced_golden(
        "double-cell-list",
        &format!(
            "{SPY_DEF}type: L | Nil | Cons next ^^L tag Spy ;\n\
: push-front ( L i64 -- L )\n  \
  | rest v |\n  \
  rest ^ ^ v Spy Cons ;\n\
: main ( -- )\n  \
  Nil 3 push-front 2 push-front 1 push-front drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 8\nalloc 24\nalloc 8\nalloc 24\nalloc 8\n\
drop 1\nfree 8\nfree 24\ndrop 2\nfree 8\nfree 24\ndrop 3\nfree 8\nfree 24\n"
    );
}

/// An A/B cycle whose `A` level declares its recursive field *before* its spy
/// and whose `B` level declares it after, so one ordering trap is live at each
/// level whichever end the disposal starts from. A-node tags are `n * 10`,
/// B-node tags `n`, so every node in the chain is distinguishable.
const MUTUAL_CHAIN_TYPES: &str = "type: Spy tag i64 ;\n\
: drop ( Spy -- )  | s | \"drop \" . s Spy> . ;\n\
type: A | ANil | ACons next ^B tag Spy ;\n\
type: B | BNil | BCons tag Spy next ^A ;\n\
: build ( i64 A -- A )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub\n    \
    n Spy acc ^ BCons ^\n    \
    n 10 mul Spy ACons\n    \
    build\n  \
  ] if ;\n";

#[test]
fn mutual_recursive_chain_disposes_from_both_directions() {
    // Criterion 4: the same chain disposed as an `A` and as a `B`. Neither
    // destructor calls the other (R6): each discovers the same cycle rotated
    // to start at its own type, so `drop_B` dispatches on `B`'s tag first and
    // on `A`'s mid-loop, and `drop_A` the reverse. The traces differ only by
    // that rotation — tags interleave `10, 1, 20, 2, 30, 3` either way, with
    // the B-rooted one prefixed by its own head node.
    let stdout = run_owned_traced_golden(
        "mutual-from-a",
        &format!("{MUTUAL_CHAIN_TYPES}: main ( -- )\n  3 ANil build drop ;\n"),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\n\
drop 10\nfree 24\ndrop 1\nfree 24\n\
drop 20\nfree 24\ndrop 2\nfree 24\n\
drop 30\nfree 24\ndrop 3\nfree 24\n"
    );

    // Rooted at `B`: an extra head node (tag 0) wrapping the same chain, so
    // disposal enters through `drop_B`'s own loop. This is the direction that
    // discriminates against the rejected "`drop_B` just calls `drop_A`"
    // design, which would recurse one native frame per node.
    let stdout = run_owned_traced_golden(
        "mutual-from-b",
        &format!(
            "{MUTUAL_CHAIN_TYPES}: main ( -- )\n  \
  3 ANil build ^ 0 Spy swap BCons drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\nalloc 24\n\
drop 0\nfree 24\n\
drop 10\nfree 24\ndrop 1\nfree 24\n\
drop 20\nfree 24\ndrop 2\nfree 24\n\
drop 30\nfree 24\ndrop 3\nfree 24\n"
    );
}

#[test]
fn multi_variant_recursive_enum_disposes_in_expected_order() {
    // Criterion 4b: two variants reach `Self` independently, and each gets
    // its own back-edge — an enum's variants are mutually exclusive at
    // runtime, so this is not D1's simultaneously-live branching case. `X`
    // declares its cell before its spy and `Y` after, so each arm's own field
    // ordering is trapped. This small trace alone cannot catch a collapse to
    // one looping variant (the other would merely recurse and print the same
    // thing); `deep_multi_variant_enum_disposes_in_constant_stack` is what
    // proves both arms loop.
    let stdout = run_owned_traced_golden(
        "multi-variant",
        &format!(
            "{SPY_DEF}type: T | Nil | X next ^T tag Spy | Y tag Spy next ^T ;\n\
: push-x ( T i64 -- T )\n  | rest v |  rest ^ v Spy X ;\n\
: push-y ( T i64 -- T )\n  | rest v |  v Spy rest ^ Y ;\n\
: main ( -- )\n  \
  Nil 4 push-y 3 push-x 2 push-y 1 push-x drop ;\n"
        ),
    );
    assert_eq!(
        stdout,
        "alloc 24\nalloc 24\nalloc 24\nalloc 24\n\
drop 1\nfree 24\ndrop 2\nfree 24\ndrop 3\nfree 24\ndrop 4\nfree 24\n"
    );
}

#[test]
fn deep_multi_variant_enum_disposes_in_constant_stack() {
    // Criterion 4c: 1,000,000 alternating `X`/`Y` nodes under a 1 MB stack.
    // Unlike criteria 5-7 this shape already passed on the base commit (the
    // old per-variant detection gave each variant its own back-edge too), so
    // it is a preservation golden: it fails the moment the generalized
    // `Branch` keeps only one `Some` variant, since every second node would
    // then recurse.
    let src = "type: T | Nil | X next ^T v i64 | Y v i64 next ^T ;\n\
: build ( i64 T -- T )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 2 mod 0 eq ~[\n      \
      n 1 sub acc ^ n X build\n    \
    ] ~[\n      \
      n 1 sub n acc ^ Y build\n    \
    ] if\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Nil build drop ;\n";
    assert_eq!(
        run_stack_bounded_golden("multi-variant-deep", src),
        Some(0),
        "both recursive variants should back-edge, not just one"
    );
}

#[test]
fn all_struct_recursive_cycle_destructor_compiles() {
    // Criterion 8: Slice 3's exit-less loop, generalized to a two-type cycle.
    // Neither `P` nor `Q` has a base case, so each destructor is a loop with
    // no `Ret` at all; sealing one anyway after the back-edge emits a
    // duplicate block label and `qbe` rejects the whole module. Both types
    // are uninhabited (a `^` is non-null, so building one needs the other
    // first), so this proves compilation, not disposal.
    let stdout = run_owned_golden(
        "all-struct-cycle",
        "type: P q ^Q ;\ntype: Q r ^P ;\n: main ( -- )\n  0 . ;\n",
    );
    assert_eq!(stdout, "0\n");
}

#[test]
fn all_struct_cycle_with_wrapper_hop_destructor_compiles() {
    // Criterion 8, second sub-shape: the same exit-less cycle with a byval
    // wrapper hop in it, so `W`'s own loop ends on a `Project` rather than an
    // `Unwrap` and its back-edge carries an interior pointer into the slot
    // the previous unwrap wrote (R7).
    let stdout = run_owned_golden(
        "all-struct-cycle-hop",
        "type: P q ^Q ;\ntype: Q w W ;\ntype: W p ^P ;\n: main ( -- )\n  0 . ;\n",
    );
    assert_eq!(stdout, "0\n");
}

#[test]
fn intermediate_dispatch_with_base_case_declared_first_terminates_correctly() {
    // Criterion 9: `B` is a plain struct, so its loop dispatches on `A`'s tag
    // *mid-path*, and each block that dispatch starts needs the
    // reset-then-check discipline (R10). Tags 1/2/3 alternate between the two
    // levels, and the two free sizes distinguish an `A` cell (24) from a `B`
    // cell (16). Both variant orders are exercised, because only the second
    // actually drifts: with `ANil` declared first (the shape the spec names)
    // no arm has yet back-edged when the terminating arm is emitted, so
    // deleting the per-arm `terminated` reset leaves this half green.
    let base_first = format!(
        "{SPY_DEF}type: A | ANil | ACons x Spy next ^B ;\n\
type: B y Spy z ^A ;\n\
: main ( -- )\n  \
  1 Spy 2 Spy 3 Spy ANil ^ B ^ ACons ^ B drop ;\n"
    );
    let expected = "alloc 24\nalloc 16\nalloc 24\n\
drop 1\nfree 24\ndrop 2\nfree 16\ndrop 3\nfree 24\n";
    assert_eq!(
        run_owned_traced_golden("mid-dispatch-base-first", &base_first),
        expected
    );

    // The continuing variant declared *first*: its arm back-edges and leaves
    // the builder marked terminated, so without the reset the terminating
    // arm's block is never sealed at all and `qbe` rejects the module with
    // `block @blk3 is used undefined` — a build failure, not a wrong trace.
    let continuing_first = format!(
        "{SPY_DEF}type: A | ACons x Spy next ^B | ANil ;\n\
type: B y Spy z ^A ;\n\
: main ( -- )\n  \
  1 Spy 2 Spy 3 Spy ANil ^ B ^ ACons ^ B drop ;\n"
    );
    assert_eq!(
        run_owned_traced_golden("mid-dispatch-continuing-first", &continuing_first),
        expected
    );
}

#[test]
fn deep_wrapper_struct_list_disposes_in_constant_stack() {
    // Criterion 5: a 1,000,000-node wrapper-struct list disposes under a
    // 1 MB stack. Verified against the pre-Slice-4 compiler (base commit
    // `6f22576`) on this exact program: SIGSEGV (139) under the same bound
    // (R13), since `recursive_loop_field` never looked one byval hop inside
    // `Wrap` for the cell.
    let src = "type: Wrap v i64 n ^List ;\n\
type: List | Nil | Cons w Wrap ;\n\
: build ( i64 List -- List )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub n acc ^ Wrap Cons build\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Nil build drop ;\n";
    assert_eq!(
        run_stack_bounded_golden("deep-wrapper-list", src),
        Some(0),
        "a 1M-node wrapper-struct list should dispose in constant stack"
    );
}

#[test]
fn deep_double_cell_list_disposes_in_constant_stack() {
    // Criterion 6: a 1,000,000-node `^^Self` list disposes under a 1 MB
    // stack. Verified against the base commit on this exact program:
    // SIGSEGV under the same bound (R13), since `recursive_loop_field` only
    // ever recognized a `^Self` field directly on the enclosing type, never
    // a cell nested inside another cell.
    let src = "type: L | Nil | Cons next ^^L v i64 ;\n\
: build ( i64 L -- L )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub acc ^ ^ n Cons build\n  \
  ] if ;\n\
: main ( -- )\n  1000000 Nil build drop ;\n";
    assert_eq!(
        run_stack_bounded_golden("deep-double-cell-list", src),
        Some(0),
        "a 1M-node ^^Self list should dispose in constant stack"
    );
}

/// The mutual A/B chain used by the two deep constant-stack goldens below:
/// both `A` and `B` are enums (unlike `MUTUAL_CHAIN_TYPES` above, whose `A`
/// is a struct), matching the `^Branch`-rooted-in-`Branch` shape R2 names.
const DEEP_MUTUAL_CHAIN_TYPES: &str = "type: A | ANil | ACons next ^B tag i64 ;\n\
type: B | BNil | BCons tag i64 next ^A ;\n\
: build ( i64 A -- A )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub\n    \
    n acc ^ BCons ^\n    \
    n 10 mul ACons\n    \
    build\n  \
  ] if ;\n";

#[test]
fn deep_mutual_chain_disposes_in_constant_stack_from_a() {
    // Criterion 7: a 1,000,000-node mutual A/B chain disposes under a 1 MB
    // stack, disposed as an `A`. Verified against the base commit on this
    // exact program: SIGSEGV under the same bound (R13).
    let src = format!("{DEEP_MUTUAL_CHAIN_TYPES}: main ( -- )\n  1000000 ANil build drop ;\n");
    assert_eq!(
        run_stack_bounded_golden("deep-mutual-from-a", &src),
        Some(0),
        "a 1M-node mutual A/B chain should dispose in constant stack from drop_A"
    );
}

#[test]
fn deep_mutual_chain_disposes_in_constant_stack_from_b() {
    // Criterion 7, `drop_B` direction: R6's sole runtime discriminator
    // against the rejected "`drop_B` calls the already-synthesized `drop_A`"
    // design, which would recurse one native frame per node and blow the
    // same 1 MB stack this golden proves stays flat. An extra `B` head node
    // wraps the same chain, so disposal enters through `drop_B`'s own loop.
    let src = format!(
        "{DEEP_MUTUAL_CHAIN_TYPES}: main ( -- )\n  \
  1000000 ANil build ^ 0 swap BCons drop ;\n"
    );
    assert_eq!(
        run_stack_bounded_golden("deep-mutual-from-b", &src),
        Some(0),
        "a 1M-node mutual A/B chain should dispose in constant stack from drop_B"
    );
}

#[test]
fn deep_recursive_chain_disposes_within_bounded_memory() {
    // A single build-then-drop can't discriminate a leak from a real free:
    // the whole chain exists at once during construction either way, so
    // both peak at the same size. Churn instead: build and drop a
    // 10,000-node chain 1,000 times through a self-tail-recursive driver.
    // A genuine free-per-node loop's peak stays flat across iterations
    // (measured ~2.8 MB here); a leak would instead accumulate all 10M
    // nodes built across the run (~290 MB, measured from the same per-node
    // cost). The 8 MB bound below sits comfortably above the real case and
    // an order of magnitude below what a leak would need.
    let src = "type: Wrap v i64 n ^List ;\n\
type: List | Nil | Cons w Wrap ;\n\
: build ( i64 List -- List )\n  \
  | n acc |\n  \
  n 0 eq ~[\n    \
    acc\n  \
  ] ~[\n    \
    n 1 sub n acc ^ Wrap Cons build\n  \
  ] if ;\n\
: churn ( i64 -- )\n  \
  dup 0 eq ~[\n    \
    drop\n  \
  ] ~[\n    \
    10000 Nil build drop\n    \
    1 sub churn\n  \
  ] if ;\n\
: main ( -- )\n  1000 churn ;\n";
    let code = run_owned_memory_bounded_golden("deep-mem-bound", src, 8192);
    assert_eq!(
        code, 0,
        "1,000 churns of a 10,000-node chain should stay within the 8 MB bound"
    );
}

#[test]
fn example_list_matches_golden() {
    // Criterion 17: `examples/list.sth` builds a list, sums the first three
    // nodes off the front via a consuming walk (`pop`/`sum-first`), then
    // `drop`s the remaining seven nodes through the fused destructor loop
    // — both a walk and a disposal, not disposal alone.
    let (stdout, code) = run_and_capture_stdout("examples/list.sth");
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}

#[test]
fn distinct_symbol_named_words_no_longer_collide_at_the_assembler() {
    // Regression: the QBE backend's symbol sanitizer (`qbe_name`) used to
    // replace every character outside `[A-Za-z0-9_.]` with a bare `_`, so
    // two distinct word names built entirely of such characters could
    // collapse onto the identical symbol, failing at the assembler with
    // `symbol `_' is already defined` well before either word could be called.
    //
    // The fixture must stay *symbolic*: with the operators-as-words rename
    // `+`/`-` are no longer builtin names but ordinary user words, which is
    // exactly what keeps them a valid subject here. Migrating them to
    // `add`/`sub` made this a placebo -- neither name contains a character
    // `qbe_name` touches, so no sanitizer behaviour was reachable at all.
    let src = ": + ( i64 i64 -- i64 ) drop ;\n\
: - ( i64 i64 -- i64 ) drop ;\n\
: main ( -- ) 1 2 + . 3 4 - . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-qbe-name-injective-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let built = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref());
    std::fs::remove_file(&path).ok();
    let binary = built.expect("two distinctly-sanitized word names build cleanly");

    let nm = std::process::Command::new("nm")
        .arg(&binary)
        .output()
        .expect("nm should run");
    std::fs::remove_file(&binary).ok();
    let symbols = String::from_utf8_lossy(&nm.stdout);
    let names: Vec<&str> = symbols
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .collect();
    // The injectivity itself, not merely "it linked": `+` is codepoint 0x2b and
    // `-` is 0x2d, so the two own visibly different symbols. Collapsing both to
    // `_` (the old scheme) satisfies neither.
    assert!(
        names.contains(&".2b.__m0") && names.contains(&".2d.__m0"),
        "each symbolic word owns its own escaped symbol; nm found:\n{symbols}"
    );
}

// -- slice 8a fix 1 (R7): lowering dispatches builtin overloads -------------

/// Write, build, and run `src`, returning `(stdout, exit_code)`. `tag`
/// distinguishes the temp source (and its emitted binary) per test.
fn run_overload_src(tag: &str, src: &str) -> (String, i32) {
    let path =
        std::env::temp_dir().join(format!("sooth-overload-{tag}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code().expect("process exits normally"),
    )
}

#[test]
fn a_tail_call_to_a_builtin_is_not_an_edge_to_its_overload() {
    // `foo` ends in the builtin `add` on two `i64`s. The tail-call cycle pass
    // runs before any body is checked, so it saw only the name and credited
    // an edge `foo -> add` to the `Vec2` overload, closing a cycle with that
    // overload's tail call to `foo` and rejecting this valid program as
    // `mutual tail recursion`.
    //
    // P8.S2 (R3a): the witness is `add`, not `lt`. The six surface
    // comparisons left `is_operator_dispatch_name` with the prelude, so a
    // module declaring its own `lt` binds every bare `lt` in it to that
    // overload rather than reaching `check_operator`'s operand dispatch --
    // `add` is where a builtin name still carries the dispatch this guards.
    let src = "type: Vec2 x i64 y i64 ;\n\
: foo ( i64 i64 -- i64 ) add ;\n\
: add ( Vec2 Vec2 -- i64 ) | a b | &a &x @ &b &x @ foo ;\n\
: main ( -- ) 1 0 Vec2 5 0 Vec2 add . ;\n";
    let (stdout, code) = run_overload_src("tail-cycle-builtin", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}

#[test]
fn a_tail_call_to_an_overloaded_ordinary_name_is_not_a_fabricated_cycle() {
    // `p`'s tail call means the i64 `show` (a leaf); `show(Vec2)`'s tail call
    // to `p` is the only real edge. name_to_idx previously mapped the shared
    // name `show` to a single word index via `.collect()`, silently keeping
    // whichever candidate was indexed last, so `p`'s tail call landed on
    // `show(Vec2)` instead and closed a cycle that does not exist.
    let src = "type: Vec2 x i64 y i64 ;\n\
: show ( i64 -- ) . ;\n\
: p ( Vec2 -- ) | v | &v &x @ show ;\n\
: show ( Vec2 -- ) | v | v p ;\n\
: main ( -- ) 3 4 Vec2 show ;\n";
    let (stdout, code) = run_overload_src("tail-cycle-ordinary-overload", src);
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

#[test]
fn mutual_tail_recursion_between_ordinary_words_is_still_an_error() {
    // The guard above must not have disarmed the pass for ordinary names,
    // which is the case it exists for.
    let src = ": a ( i64 -- i64 ) b ;\n: b ( i64 -- i64 ) a ;\n: main ( -- ) 1 a . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-tail-cycle-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("mutual tail recursion") && err.contains("`a`") && err.contains("`b`"),
        "unexpected message: {err}"
    );
}

#[test]
fn overloads_of_a_combinator_name_are_both_reachable() {
    // main rejects two same-name `apply` combinators outright. R1's widened
    // key admits them as distinct definitions once their concrete parameter
    // types differ, but collect_combinators stayed a bare-name-keyed
    // single-value map, so the second candidate silently displaced the first
    // exactly as env's Sig did before B1 (and poly_env did for a poly word).
    let src = ": apply inline ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
: apply inline ( Bool [ Bool -- Bool ] -- Bool ) call ;\n\
: main ( -- ) 5 [ 2 mul ] apply . True [ not ] apply . ;\n";
    let (stdout, code) = run_overload_src("combinator-overload", src);
    assert_eq!(stdout, "10\nFalse\n");
    assert_eq!(code, 0);
}

#[test]
fn a_combinator_call_matching_no_overload_names_the_candidates() {
    let src = ": apply inline ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
: apply inline ( Bool [ Bool -- Bool ] -- Bool ) call ;\n\
: main ( -- ) \"x\" [ drop \"y\" ] apply drop ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-combinator-overload-nomatch-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("no overload of `apply`") && err.contains("accepts these operands"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("candidate: `i64`") && err.contains("candidate: `Bool`"),
        "expected both candidates listed: {err}"
    );
}

#[test]
fn overloads_of_a_polymorphic_word_name_are_both_reachable() {
    // main rejects two same-name poly words outright (`duplicate word`); R1
    // widened the concrete-word key but poly_env stayed a bare-name-keyed
    // single-value map, so the second candidate silently displaced the first
    // exactly as env's Sig did before B1.
    let src = ": idpair ( 'T 'T -- 'T ) drop ;\n\
: idpair ( 'T Bool -- 'T ) drop ;\n\
: main ( -- ) 1 2 idpair . 7 True idpair . ;\n";
    let (stdout, code) = run_overload_src("poly-overload", src);
    assert_eq!(stdout, "1\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn a_polymorphic_call_matching_no_candidate_names_the_signatures() {
    let src = ": idpair ( 'T 'T -- 'T ) drop ;\n\
: idpair ( 'T Bool -- 'T ) drop ;\n\
: main ( -- ) 1 2.5 idpair . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-poly-overload-nomatch-{}.sth",
        std::process::id()
    ));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("no overload of `idpair`") && err.contains("accepts these operands"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("candidate: : idpair ( 'T 'T -- 'T )")
            && err.contains("candidate: : idpair ( 'T Bool -- 'T )"),
        "expected both candidate signatures listed: {err}"
    );
}

#[test]
fn two_poly_words_declaring_the_same_signature_is_a_duplicate_error() {
    // Deferred from round 3: unlike a genuinely different second candidate
    // (the two tests above), a *second* poly word declaring the exact same
    // signature as the first has no legitimate reason to exist -- it would
    // silently resolve to the first, forever, the second dead code rather
    // than a reachable overload.
    let path = std::env::temp_dir().join(format!(
        "sooth-poly-dup-signature-{}.sth",
        std::process::id()
    ));
    std::fs::write(
        &path,
        ": idpair ( 'T 'T -- 'T ) drop ;\n\
: idpair ( 'T 'T -- 'T ) drop drop ;\n\
: main ( -- ) 1 2 idpair . ;\n",
    )
    .expect("writing temp source should succeed");
    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("duplicate overload") && err.contains(": idpair ( 'T 'T -- 'T )"),
        "unexpected message: {err}"
    );
}

#[test]
fn two_poly_words_declaring_an_alpha_equivalent_signature_is_a_duplicate_error() {
    // Same shape as the test above, spelled with a different variable name
    // (`'U` instead of `'T`) -- a variable's id is assigned by
    // first-appearance order per signature (`PolySig`'s own doc), so this is
    // structurally the same signature, not a different one, and must still
    // be caught rather than passing because the surface spelling differs.
    let path = std::env::temp_dir().join(format!(
        "sooth-poly-dup-signature-alpha-{}.sth",
        std::process::id()
    ));
    std::fs::write(
        &path,
        ": idpair ( 'T 'T -- 'T ) drop ;\n\
: idpair ( 'U 'U -- 'U ) drop drop ;\n\
: main ( -- ) 1 2 idpair . ;\n",
    )
    .expect("writing temp source should succeed");
    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("duplicate overload"),
        "unexpected message: {err}"
    );
}

#[test]
fn overloads_of_an_ordinary_word_name_are_both_reachable() {
    // R1 widened the duplicate-word key to admit these two definitions, but
    // the checking env held one `Sig` per name, so the second silently
    // displaced the first: the call `42 show` then failed against the `Bool`
    // signature and the `i64` body was unreachable code. Both candidates must
    // resolve, which is what makes the widened key mean anything.
    let src = ": show ( i64 -- ) . ;\n\
: show ( Bool -- ) . ;\n\
: main ( -- ) 42 show True show ;\n";
    let (stdout, code) = run_overload_src("user-name-overloads", src);
    assert_eq!(stdout, "42\nTrue\n");
    assert_eq!(code, 0);
}

#[test]
fn overloads_of_an_ordinary_word_name_get_distinct_symbols() {
    // Each candidate's body is minted under its own symbol
    // (`ast::overload_symbols`); sharing one would collide at the assembler
    // exactly as two symbol-named words did before the `qbe_name` fix. Both
    // bodies here are distinguishable at runtime, so a collision that kept
    // only one body would print the wrong pair.
    let src = "type: Vec2 x i64 y i64 ;\n\
: mag ( i64 -- i64 ) 10 mul ;\n\
: mag ( Vec2 -- i64 ) | v | &v &x @ &v &y @ add ;\n\
: main ( -- ) 7 mag . 3 4 Vec2 mag . ;\n";
    let (stdout, code) = run_overload_src("user-name-symbols", src);
    assert_eq!(stdout, "70\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn a_call_matching_no_overload_names_the_candidates() {
    // R3: the resolution failure is located and lists what the name does
    // accept, rather than reporting a mismatch against whichever candidate
    // happened to be stored last.
    let src = ": show ( i64 -- ) . ;\n\
: show ( Bool -- ) . ;\n\
: main ( -- ) 1.5 show ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-overload-nomatch-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    assert!(
        err.contains("no overload of `show`") && err.contains("accepts these operands"),
        "unexpected message: {err}"
    );
    assert!(
        err.contains("candidate: `i64`") && err.contains("candidate: `Bool`"),
        "expected both candidates listed: {err}"
    );
}

#[test]
fn overload_vec2_plus_dispatches_to_user_word() {
    // `Module::builtin_overloads` records this call site's resolution to the
    // user `add` overload, but before the fix `lower_call`'s name-directed
    // `"+" | "-" | ...` arm never consulted it, so it always emitted
    // `Instr::Bin(Add)` on the two `Vec2` struct pointers (an address add),
    // producing a garbage pointer that segfaulted on the following field
    // reads. `lower_call` must check `builtin_overloads` first and emit an
    // `Instr::Call` to the user word instead.
    let src = "type: Vec2 x i64 y i64 ;\n\
: add ( Vec2 Vec2 -- Vec2 ) | a b | &a &x @ &b &x @ add &a &y @ &b &y @ add Vec2 ;\n\
: main ( -- ) 1 2 Vec2 3 4 Vec2 add &x @ . &y @ . drop ;\n";
    let (stdout, code) = run_overload_src("plus", src);
    assert_eq!(stdout, "4\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn overload_vec2_minus_dispatches_to_user_word() {
    // Same bug, `sub`: unfixed, `Instr::Bin(Sub)` on the two struct pointers
    // subtracts addresses rather than fields, yielding a bogus pointer whose
    // field reads then segfault.
    let src = "type: Vec2 x i64 y i64 ;\n\
: sub ( Vec2 Vec2 -- Vec2 ) | a b | &a &x @ &b &x @ sub &a &y @ &b &y @ sub Vec2 ;\n\
: main ( -- ) 5 6 Vec2 1 2 Vec2 sub &x @ . &y @ . drop ;\n";
    let (stdout, code) = run_overload_src("minus", src);
    assert_eq!(stdout, "4\n4\n");
    assert_eq!(code, 0);
}

#[test]
fn overload_vec2_lt_dispatches_to_user_word() {
    // Same bug, `lt`: unfixed, `Instr::Cmp(Lt)` compares the two struct
    // pointers' addresses rather than dispatching to the user overload, so
    // the printed boolean tracks allocation order, not the operands' values.
    // Negative coordinates whose semantic sum is negative (so the correct
    // answer is `False`) still allocate `a` before `b` (a lower address), so
    // the old pointer-compare silently printed `True` here.
    let src = "type: Vec2 x i64 y i64 ;\n\
: lt ( Vec2 Vec2 -- Bool ) | a b | &a &x @ &b &x @ add &a &y @ &b &y @ add add 0 gt ;\n\
: main ( -- ) -3 -4 Vec2 -1 -2 Vec2 lt . ;\n";
    let (stdout, code) = run_overload_src("lt", src);
    assert_eq!(stdout, "False\n");
    assert_eq!(code, 0);
}

#[test]
fn overload_ending_in_its_own_builtin_name_calls_the_builtin_not_itself() {
    // The tail term `add` shares the enclosing word's name but resolves to the
    // *builtin* `add` on two `i64` fields, not to a recursive call. Before the
    // fix `has_self_tail_call` matched on the bare name, so the word was
    // treated as self-tail-recursive: lowering opened loop machinery, the
    // back-edge pushed the two `i64`s as phi operands for a header expecting
    // two `Vec2`s, and the compiler panicked on the missing header block
    // (`expect("header block")`) rather than emitting the arithmetic.
    //
    // P8.S2 (R3a): `add`, not `lt`, for the reason spelled out in
    // `a_tail_call_to_a_builtin_is_not_an_edge_to_its_overload`.
    let src = "type: Vec2 x i64 y i64 ;\n\
: add ( Vec2 Vec2 -- i64 ) | a b | &a &x @ &b &x @ add ;\n\
: main ( -- ) 1 2 Vec2 3 4 Vec2 add . ;\n";
    let (stdout, code) = run_overload_src("add-tail-self-name", src);
    assert_eq!(stdout, "4\n");
    assert_eq!(code, 0);
}

#[test]
fn print_overload_ending_in_its_own_builtin_name_compiles_and_prints() {
    // The same defect on `.`, the shape the slice's own review first hit: a
    // `Vec2` print overload naturally ends by printing its last field with
    // the builtin `.`, which is also its own name.
    let src = "type: Vec2 x i64 y i64 ;\n\
: . ( Vec2 -- ) | v | &v &x @ . &v &y @ . ;\n\
: main ( -- ) 3 4 Vec2 . ;\n";
    let (stdout, code) = run_overload_src("print-tail-self-name", src);
    assert_eq!(stdout, "3\n4\n");
    assert_eq!(code, 0);
}

#[test]
fn overload_from_poly_body_dispatches_to_user_word() {
    // Slice 8a fix 2: a genuinely polymorphic word (`pair-sum`, generic in
    // `'T` for an unrelated passthrough slot) whose body calls `add` on two
    // *concretely*-typed `Vec2` operands from its own signature. Before the
    // fix, `poly_call_term`'s env-based dispatch intercepted `add` by name
    // alone and never recorded the call site, so lowering fell through to
    // the builtin `Instr::Bin(Add)` arm on the two struct pointers and
    // segfaulted, identically to the monomorphic bug fix 1 addresses.
    let src = "type: Vec2 x i64 y i64 ;\n\
: add ( Vec2 Vec2 -- Vec2 ) | a b | &a &x @ &b &x @ add &a &y @ &b &y @ add Vec2 ;\n\
: pair-sum ( 'T Vec2 Vec2 -- 'T Vec2 ) add ;\n\
: main ( -- ) 42 1 2 Vec2 3 4 Vec2 pair-sum swap drop &x @ . &y @ . drop ;\n";
    let (stdout, code) = run_overload_src("poly-plus", src);
    assert_eq!(stdout, "4\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn traits_dogfood_compiles_and_runs() {
    // P7.S3r phase 3: `examples/traits.sth` is the only `.sth` dogfood of the
    // `impl:` body form. `show_larger` takes a `'T: Order Show` bound, so both
    // members dispatch to `Point`'s own `impl:` block at monomorphization.
    // (0,0) `cmp` (3,4) is `Less`, so the larger of the two shows: (3,4).
    // `show` prints `(`/`,`/`)` with no trailing newline of its own.
    let (stdout, code) = run_and_capture_stdout("examples/traits.sth");
    assert_eq!(stdout, "(3\n,4\n)");
    assert_eq!(code, 0);
}

#[test]
fn overload_exact_type_beats_numeric_coercion_at_the_call_site() {
    // R2: the resolver runs an exact-input-type pass across every candidate
    // (builtin rows and user overloads) before numeric coercion ever runs.
    // `add ( usize i64 -- usize )` is a legal overload (its mixed input types
    // match no homogeneous builtin row, R1), and `5 >usize 3 add` presents
    // exactly those operand types (a `usize` and an unconverted `i64`
    // literal) -- without this overload, that same call site would coerce
    // the literal into the builtin homogeneous `usize add` (`unify_pair`'s
    // literal-coercion arm) and print `8`. The overload must win instead:
    // if a later addition ever let coercion run first, or ran it whenever an
    // exact candidate merely *exists* without checking the operands first,
    // this site would silently start printing `8` instead of the
    // overload's sentinel.
    let src = ": add ( usize i64 -- usize ) drop drop 999 ;\n\
: main ( -- ) 5 >usize 3 add . ;\n";
    let (stdout, code) = run_overload_src("exact-beats-coercion", src);
    assert_eq!(stdout, "999\n");
    assert_eq!(code, 0);
}
