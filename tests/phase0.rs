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
    let src = ": signed_lt ( -- i64 )\n  200 >i8 5 >i8 < if 1 else 0 end ;\n\n\
: unsigned_lt ( -- i64 )\n  200 >u8 5 >u8 < if 1 else 0 end ;\n\n\
: main ( -- )\n  signed_lt .\n  unsigned_lt . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-signed-vs-unsigned-cmp-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    // comparing it against a clean `u16` `65535` must be `true`.
    let src = ": main ( -- )\n  200 >u8 >i8 >u16 65535 >u16 < if 1 else 0 end . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-widen-subword-cmp-golden-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);
}

#[test]
fn mixed_width_arithmetic_reports_both_types() {
    // X1: an `i32` and an `i64` fed to `+` names both differing types, via the
    // operand-pair-mismatch diagnostic specifically.
    let src = ": f ( -- i32 ) 1 >i32 5 + ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
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
    // X2: `u8` and `i8` fed to `<` names both differing operand types, via the
    // same operand-pair-mismatch diagnostic as X1.
    let src = ": w ( -- bool ) 200 >u8 5 >i8 < ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same numeric type"),
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
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`u8`"), "unexpected message: {err}");
}

#[test]
fn conversion_of_bool_reports_diagnostic() {
    // X4: `>i32` applied to a `bool` is a type error naming the source is not an integer.
    let src = ": w ( -- i32 ) true >i32 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("numeric"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn conversion_unknown_target_reports_diagnostic() {
    // X5: `>i128` reads as an unknown conversion target.
    let src = ": w ( -- i64 ) 5 >i128 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("i128"), "unexpected message: {err}");
}

#[test]
fn if_condition_not_bool_reports_diagnostic() {
    let src = ": oops ( -- i64 )\n  5 if 1 else 2 end ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("expected `bool`"), "unexpected message: {err}");
    assert!(err.contains("found `i64`"), "unexpected message: {err}");
}

#[test]
fn operand_type_mismatch_reports_diagnostic() {
    let src = ": oops ( -- i64 )\n  true 1 + ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn branch_join_type_mismatch_reports_diagnostic() {
    let src = ": oops ( bool -- i64 )\n  if 1 else true end ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("different types"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn declared_output_type_mismatch_reports_diagnostic() {
    let src = ": oops ( i64 -- bool )\n  1 + ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("type mismatch"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn unknown_type_name_reports_diagnostic() {
    let src = ": oops ( foo -- i64 )\n  1 ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let err = parser::parse(&tokens).expect_err("parsing should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("foo"), "unexpected message: {err}");
}

#[test]
fn stack_effect_mismatch_reports_diagnostic() {
    let src = ": oops ( i64 -- i64 )\n  | a | a a + + ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

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
    // S1/S3: `+ - *` run correctly on `f64` and on `f32` (converted back to
    // `f64` for `.`, since `.` prints an `f32` by widening to `f64`).
    let src = ": main ( -- )\n  1.0 2.0 + .\n  5.0 2.0 - .\n  3.0 4.0 * .\n  \
1.5 >f32 2.5 >f32 + >f64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-arith-both-widths-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "3\n3\n12\n4\n");
    assert_eq!(code, 0);
}

#[test]
fn float_division_produces_inf_and_nan_with_nan_detectable_via_self_compare() {
    // S3: `1.0 0.0 /` is inf, `0.0 0.0 /` is NaN, with no trap, and NaN is
    // detectable via `x = x` (false only for NaN, D4). `fdiv` runs the
    // division through a real call boundary so QBE cannot constant-fold the
    // literal `0.0 0.0 /` away (an unrelated compile-time-only restriction).
    let src = ": fdiv ( f64 f64 -- f64 )\n  | a b | a b / ;\n\n\
: main ( -- )\n  1.0 0.0 fdiv .\n  0.0 0.0 fdiv .\n  \
0.0 0.0 fdiv dup = if 1 else 0 end . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-div-inf-nan-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    assert_eq!(lines[2], "0", "NaN = NaN must be false");
    assert_eq!(code, 0);
}

#[test]
fn float_comparison_is_ieee_ordered_and_false_for_nan() {
    // S4: an ordered comparison gives the expected boolean, and every
    // comparison involving a NaN produced by `0.0 0.0 /` is false, including
    // `<` and `>` against a NaN (not just `=`, RISK 1).
    let src = ": fdiv ( f64 f64 -- f64 )\n  | a b | a b / ;\n\n\
: main ( -- )\n  1.0 2.0 < if 1 else 0 end .\n  2.0 1.0 < if 1 else 0 end .\n  \
0.0 0.0 fdiv dup < if 1 else 0 end .\n  0.0 0.0 fdiv dup > if 1 else 0 end . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-float-cmp-ordered-nan-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "44\n100\n");
    assert_eq!(code, 0);
}

#[test]
fn mixed_int_float_arithmetic_reports_diagnostic() {
    // X1 (headline negative, S8): `+` fed an `i64` and an `f64` names both
    // differing types via the operand-pair-mismatch diagnostic.
    let src = ": f ( -- f64 ) 1 3.0 + ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
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
    // X2: `f32` and `f64` fed to `<` names both differing operand types.
    let src = ": w ( -- bool ) 1.0 >f32 2.0 < ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same numeric type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`f32`"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn integer_division_reports_diagnostic() {
    // X3: `/` requires floats; two `i64` operands is an error.
    let src = ": f ( -- i64 ) 6 2 / ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains('/'), "unexpected message: {err}");
    assert!(err.contains("float"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn float_mod_reports_diagnostic() {
    // X4: `mod` stays integer-only; two `f64` operands is an error.
    let src = ": f ( -- f64 ) 6.0 2.0 mod ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("mod"), "unexpected message: {err}");
    assert!(err.contains("integer"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn bool_to_float_conversion_reports_diagnostic() {
    // X5: `>f64` applied to a `bool` names the source and states it must be
    // numeric.
    let src = ": w ( -- f64 ) true >f64 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("numeric"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn unknown_float_conversion_target_reports_diagnostic() {
    // X6: `>f128` is an unknown conversion target.
    let src = ": w ( -- f64 ) 5.0 >f128 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("f128"), "unexpected message: {err}");
}

// Bitwise operators (`and`/`or`/`xor`/`not`/`shl`/`shr`) diagnostics + goldens.

#[test]
fn bitwise_op_on_float_reports_diagnostic() {
    let src = ": w ( -- f64 ) 3.0 5.0 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("integer"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn bitwise_op_on_bool_is_now_accepted() {
    // `and`/`or`/`xor` are type-directed: `bool` is now a valid homogeneous
    // operand class, not just the integer tower.
    let src = ": w ( -- bool ) true false and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("check should succeed");
}

#[test]
fn mixed_bool_int_and_reports_both_types() {
    let src = ": w ( -- bool ) true 5 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same integer or bool type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`bool`"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn mixed_type_and_reports_both_types() {
    let src = ": w ( -- i64 ) 1 >i32 2 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same integer or bool type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`i32`"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn shift_with_non_i64_count_reports_diagnostic() {
    let src = ": w ( -- u8 ) 1 >u8 3 >i32 shl ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    let src = ": main ( -- )\n  1 >i8 7 shl 0 >i8 < if 1 . else 0 . end ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-signed-subword-shift-compare-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    let src = ": main ( -- )\n  1 >u8  0 6 -  shl >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-negative-shift-count-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "4\n");
    assert_eq!(code, 0);
}

// Boolean logical ops (`and`/`or`/`xor`/`not` on `bool`) + the `<= >= <>`
// comparison completion: diagnostics + goldens.

#[test]
fn cmp_le_ge_ne_on_bool_reports_diagnostic() {
    let src = ": w ( -- bool ) true false <= ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(
        err.contains("same numeric type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn logical_and_or_xor_truth_table_on_bools() {
    // `and`/`or`/`xor` on `bool` operands ARE logical and/or/xor (an eager
    // stack language already evaluates both operands, so bitwise-on-0/1 and
    // logical coincide): T and T = T, T and F = F, T or F = T, F or F = F,
    // T xor F = T, T xor T = F.
    let src = ": main ( -- )\n  \
  true true and if 1 else 0 end .\n  \
  true false and if 1 else 0 end .\n  \
  true false or if 1 else 0 end .\n  \
  false false or if 1 else 0 end .\n  \
  true false xor if 1 else 0 end .\n  \
  true true xor if 1 else 0 end . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-logical-and-or-xor-truth-table-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n0\n1\n0\n1\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn not_is_type_directed_bool_logical_vs_integer_bitwise() {
    // `not` is type-directed: on a `bool` it is logical negation
    // (`true not` -> false), giving a DIFFERENT result than the integer
    // bitwise complement on the same underlying bit pattern (`0 >u8 not` ->
    // 255, not 1).
    let src = ": main ( -- )\n  \
  true not if 1 else 0 end .\n  \
  0 >u8 not >i64 . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-not-type-directed-bool-vs-int-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n255\n");
    assert_eq!(code, 0);
}

#[test]
fn le_ge_ne_on_integers_with_signed_unsigned_edge() {
    // The same bit pattern (200) compares differently as `i8` (-56, negative)
    // vs `u8` (200, positive) against 5: `<=`/`>=` flip with the sign, while
    // `<>` stays true either way (not-equal is sign-agnostic like `=`).
    let src = ": main ( -- )\n  \
  200 >i8 5 >i8 <= if 1 else 0 end .\n  \
  200 >u8 5 >u8 <= if 1 else 0 end .\n  \
  200 >i8 5 >i8 >= if 1 else 0 end .\n  \
  200 >u8 5 >u8 >= if 1 else 0 end .\n  \
  200 >i8 5 >i8 <> if 1 else 0 end .\n  \
  200 >u8 5 >u8 <> if 1 else 0 end . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-le-ge-ne-signed-unsigned-edge-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1\n0\n0\n1\n1\n1\n");
    assert_eq!(code, 0);
}

#[test]
fn le_ge_ne_are_ieee_ordered_and_correct_for_nan_floats() {
    // A real NaN (`0.0 0.0 /`, routed through a call so it isn't
    // constant-folded away) must report false for the ordered comparisons
    // `<=`/`>=`/`=`, and true for `<>` (RISK 1): `<>` is the one comparison
    // where "NaN involved" flips the answer relative to `=`.
    let src = ": fdiv ( f64 f64 -- f64 )\n  | a b | a b / ;\n\n\
: main ( -- )\n  \
  0.0 0.0 fdiv dup <= if 1 else 0 end .\n  \
  0.0 0.0 fdiv dup >= if 1 else 0 end .\n  \
  0.0 0.0 fdiv dup <> if 1 else 0 end .\n  \
  0.0 0.0 fdiv dup = if 1 else 0 end . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-le-ge-ne-nan-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n0\n1\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn leap_year_dogfood_compiles_and_runs() {
    let (stdout, code) = run_and_capture_stdout("examples/leap.sth");
    assert_eq!(stdout, "true\nfalse\ntrue\n");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "2.5\n1.5\n");
    assert_eq!(code, 0);
}

#[test]
fn print_bool_prints_true_or_false_not_zero_or_one() {
    let src = ": main ( -- )\n  2 3 < .\n  3 2 < . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-print-bool-true-false-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "true\nfalse\n");
    assert_eq!(code, 0);
}

#[test]
fn f_dot_is_now_an_unknown_word() {
    // `f.` is removed entirely: it reads as any other unknown word.
    let src = ": w ( f64 -- ) f. ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail: `f.` no longer exists");

    assert!(err.contains("unknown word"), "unexpected message: {err}");
    assert!(err.contains("f."), "unexpected message: {err}");
}

// Slice 3 (structs): running-binary goldens for the aggregate codegen (S2-S7,
// NF5). Each builds a struct program to a native binary and checks stdout.

fn run_struct_golden(tag: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-struct-{tag}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(code, 0, "struct golden `{tag}` should exit 0");
    stdout
}

#[test]
fn struct_flat_construct_get_destructure_native() {
    // S2: construct a flat struct, read each field, and destructure it.
    let src = "type: Vec2 x i64 y i64 ;\n\
: main ( -- )\n  3 4 Vec2 dup Vec2>x . Vec2>y .\n  5 6 Vec2 Vec2> . . ;\n";
    // destructure pushes x then y (first deepest); `. .` prints top-first: 6 then 5.
    assert_eq!(run_struct_golden("flat", src), "3\n4\n6\n5\n");
}

#[test]
fn struct_functional_setter_leaves_duped_original_intact_native() {
    // S4: `dup` copies the aggregate; a functional setter on the copy returns a
    // new value while the original is unchanged.
    let src = "type: Vec2 x i64 y i64 ;\n\
: main ( -- )\n  1 2 Vec2 dup 99 Vec2<x Vec2>x . Vec2>x . ;\n";
    assert_eq!(run_struct_golden("setter-intact", src), "99\n1\n");
}

#[test]
fn struct_mixed_i64_f64_field_readback_native() {
    // S3: offset-correct read-back for mixed-width fields (an i64 and an f64).
    let src = "type: Mix a i64 b f64 ;\n\
: main ( -- )\n  7 2.5 Mix dup Mix>a . Mix>b . ;\n";
    assert_eq!(run_struct_golden("mixed", src), "7\n2.5\n");
}

#[test]
fn struct_adjacent_subword_fields_do_not_clobber_native() {
    // RISK 3: two adjacent `i8` fields then an `i64` must each read back their
    // own value; a width-exact field store never clobbers its neighbour.
    let src = "type: P p i8 q i8 r i64 ;\n\
: main ( -- )\n  1 >i8 2 >i8 300 P dup P>p >i64 . dup P>q >i64 . P>r . ;\n";
    assert_eq!(run_struct_golden("packed", src), "1\n2\n300\n");
}

#[test]
fn struct_nested_juxtaposition_access_native() {
    // S3: a nested struct field accessed by juxtaposition (`Segment>to Vec2>x`),
    // read back per-field.
    let src = "type: Vec2 x i64 y i64 ;\n\
type: Segment from Vec2 to Vec2 ;\n\
: main ( -- )\n  1 2 Vec2 3 4 Vec2 Segment\n  dup Segment>from Vec2>x .\n  Segment>to Vec2>y . ;\n";
    assert_eq!(run_struct_golden("nested", src), "1\n4\n");
}

#[test]
fn struct_survives_word_call_boundary_native() {
    // S5: a struct argument and a struct return cross a word-call boundary
    // (by-value QBE C-ABI), then the returned struct's field is read back.
    let src = "type: Vec2 x i64 y i64 ;\n\
: shift ( Vec2 i64 -- Vec2 ) | v d |\n  v Vec2>x d + v Vec2>y Vec2 ;\n\
: main ( -- )\n  10 20 Vec2 5 shift dup Vec2>x . Vec2>y . ;\n";
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
: main ( -- )\n  1 2 Vec2 3 4 Vec2 Segment\n  swap-ends\n  dup Segment>from Vec2>x .\n  Segment>to Vec2>y . ;\n";
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
    // non-trivial. The enum itself can't be read back until the clause
    // eliminator lands (Phase 4), so it is dropped after the round-trip.
    let src = "type: Big | B a i64 b i64 c i64 ;\n\
: id ( Big -- Big ) ;\n\
: main ( -- )\n  42 1 2 3 B id drop . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-enum-call-boundary-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    // prints `span len2 .` (25) and `5 6 Vec2 1 shift-x Vec2>x .` (6).
    let (stdout, code) = run_and_capture_stdout("examples/vectors.sth");
    assert_eq!(stdout, "25\n6\n");
    assert_eq!(code, 0);
}

#[test]
fn shapes_dogfood_compiles_and_runs() {
    // Slice 4 (criteria 3, 6): the clause-style eliminator end-to-end.
    // `area` dispatches over a multi-field-variant `Shape` (a `Rect | w h |`
    // clause-body-locals arm and a `Circle` arm reading its payload
    // first-deepest); `unwrap-or` dispatches over a zero-field `None` (an
    // empty clause yielding the default flowing *underneath* the scrutinee)
    // and a one-field `Some`. All run in one native binary.
    let (stdout, code) = run_and_capture_stdout("examples/shapes.sth");
    assert_eq!(stdout, "12.5664\n12\n5\n7\n");
    assert_eq!(code, 0);
}

#[test]
fn clause_word_over_three_plus_variant_enum_dispatches_correctly() {
    // R16 key risk: an N-way (here 4-way) `Cmp(Eq)`-tag compare-chain must
    // land on each variant's own clause, not a two-way miscompile. Each of
    // the four commands drives a distinct arm; verified at runtime, not by
    // reading IL.
    let src = "type: Cmd | Halt | Push v i64 | Add | Dbl ;\n\
: run ( i64 Cmd -- i64 )\n\
| Halt   drop 0\n\
| Push   swap drop\n\
| Add    1 +\n\
| Dbl    2 *\n\
;\n\
: main ( -- )\n  99 Halt run .\n  1 20 Push run .\n  10 Add run .\n  10 Dbl run . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-nway-clause-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "0\n20\n11\n20\n");
    assert_eq!(code, 0);
}

#[test]
fn nested_aggregate_clause_word_reads_back_through_registries_native() {
    // Slice 4 (criterion 3, D9): a variant carrying a struct payload (`Dot p
    // Vec2`) constructs, passes through a clause word, and its nested field
    // reads back; and an enum used as a struct field (`Wrap s Shape`) is
    // unwrapped through the getter into the same clause word — guarding the
    // combined-registry field sizing in both directions.
    let src = "type: Vec2 x i64 y i64 ;\n\
type: Shape | Dot p Vec2 | Nothing ;\n\
type: Wrap s Shape ;\n\
: px ( Shape -- i64 )\n\
| Dot      Vec2>x\n\
| Nothing  0\n\
;\n\
: main ( -- )\n  3 4 Vec2 Dot px .\n  Nothing px .\n  5 6 Vec2 Dot Wrap Wrap>s px . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-nested-clause-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "3\n0\n5\n");
    assert_eq!(code, 0);
}

#[test]
fn single_variant_clause_word_returns_payload_native() {
    // 9ed63d9 discriminant-skip: a single-variant enum has nothing to
    // disambiguate, so its clause word jumps straight to the sole clause with
    // no `Cmp`/`Jnz`. The IR-shape test asserts zero compares; this proves the
    // no-compare path still returns the right payload value at runtime.
    let src = "type: Id | Wrap v i64 ;\n\
: unwrap ( Id -- i64 )\n\
| Wrap\n\
;\n\
: main ( -- )\n  42 Wrap unwrap . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-single-variant-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn clause_body_containing_if_else_end_joins_correctly() {
    // M6: a clause body may itself use `if/else/end`. The clause-dispatch
    // join's phi predecessor must be the *if's* merged block (captured via
    // `cur_id` after lowering the clause body), not the clause's dispatch
    // block — otherwise the join would read a stale/wrong value. `Zero`
    // exercises a plain clause alongside `NonZero`'s internal branch, so the
    // two clause styles share one dispatch and one join correctly.
    let src = "type: Item | Zero | NonZero v i64 ;\n\
: classify ( Item -- i64 )\n\
| Zero       0\n\
| NonZero    0 > if 1 else -1 end\n\
;\n\
: main ( -- )\n  Zero classify .\n  5 NonZero classify .\n  -5 NonZero classify . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-clause-if-else-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
  2 3 + >usize 4 >usize + .\n\
  10 >usize 3 >usize - .\n\
  3 >usize 5 >usize < .\n\
  5 >usize 3 >usize < .\n\
  7 >usize >i64 1 + .\n\
  9 >usize dup . drop ;\n";
    let path = std::env::temp_dir().join(format!("sooth-usize-tower-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "9\n7\ntrue\nfalse\n8\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn fill_constructs_and_get_reads_every_element_back_native() {
    // Criterion 3: `fill` an `[i64 N]`, then read every element back via
    // `get` and print it; the values match the fill value (unrolled stores +
    // dynamic element addressing, R17/R18).
    let src = ": main ( -- )\n\
  9 4 fill\n\
  dup 0 get . drop\n\
  dup 1 get . drop\n\
  dup 2 get . drop\n\
  3 get . drop ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-fill-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "9\n9\n9\n9\n");
    assert_eq!(code, 0);
}

#[test]
fn set_at_runtime_index_yields_new_array_original_untouched_native() {
    // Criterion 4: `set` at a *runtime* index (computed, not a literal) on a
    // duped array yields a new array with exactly one element changed; the
    // original (kept alongside) is untouched (D5 value semantics). `get`
    // leaves the array on the stack afterwards (non-consuming, R12/M4), so
    // the same array is read from twice in a row without redoing `get`'s
    // array operand.
    let src = ": main ( -- )\n\
  0 4 fill dup\n\
  1 1 + >usize 99 set\n\
  2 >usize get .\n\
  0 >usize get .\n\
  swap\n\
  2 >usize get .\n\
  0 >usize get .\n\
  drop drop ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-set-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    // New array: index 2 changed to 99, index 0 unchanged (0); original
    // array: both indices still 0.
    assert_eq!(stdout, "99\n0\n0\n0\n");
    assert_eq!(code, 0);
}

#[test]
fn constant_out_of_range_array_index_is_compile_error() {
    // Criterion 5(a), X4: a literal index >= N is a sharp, located compile
    // error naming the length and the index.
    let src = ": w ( -- i64 )\n  0 4 fill 9 get drop ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&mut module).expect_err("check should fail");

    assert!(err.contains("out of range"), "unexpected message: {err}");
    assert!(err.contains('9'), "should name the index: {err}");
    assert!(err.contains('4'), "should name the length: {err}");
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
  0 4 fill\n\
  3 4 + >usize get drop drop\n\
  99 . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-array-trap-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = driver::build(&path).expect("build should succeed");
    std::fs::remove_file(&path).ok();

    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
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
fn stack_dogfood_compiles_and_runs() {
    // Criterion 6: `examples/stack.sth`, a bounded `i64` stack embedding a
    // `[i64 16]` field with a runtime `usize` cursor. Exercises
    // array-as-struct-field, `push`/`pop`/`peek`, non-consuming `get`,
    // functional `set`, and the compile-time-constant `len`.
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
| a i | a i get Vec2>x swap drop ;\n\
: inner-at ( [[i64 2] 2] usize usize -- i64 )\n\
| a i j | a i get swap drop j get swap drop ;\n\
: box-at ( Box usize -- i64 )\n\
| b i | b Box>arr i get swap drop ;\n\
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
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
  n 0 = if\n\
    acc\n\
  else\n\
    acc 10 * n + n 1 - digits\n\
  end ;\n\
: main ( -- ) 0 5 digits . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-locals-rebind-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    // clause back-edge path of criterion 3). Both arms happen to do the same
    // arithmetic; what matters is that they are two distinct call sites that
    // both eliminate, at N large enough to overflow if either didn't.
    let src = ": both-tail ( i64 i64 -- i64 )\n\
  | acc n |\n\
  n 0 = if\n\
    acc\n\
  else\n\
    n 500000 > if\n\
      acc n + n 1 - both-tail\n\
    else\n\
      acc n + n 1 - both-tail\n\
    end\n\
  end ;\n\
: main ( -- ) 0 1000000 both-tail . ;\n";
    let path =
        std::env::temp_dir().join(format!("sooth-both-arms-tail-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn clause_multi_tail_runs_in_constant_stack_native() {
    // Criterion 3: a `|`-clause self-tail-recursive word where *both*
    // clauses tail-recurse into one shared header, alternating tags each
    // iteration (each clause contributes its own back-edge).
    let src = "type: Parity | Even | Odd ;\n\
: sum-parity ( i64 i64 Parity -- i64 )\n\
  | Even | acc n | n 0 = if acc else acc n + n 1 - Odd sum-parity end\n\
  | Odd  | acc n | n 0 = if acc else acc n + n 1 - Even sum-parity end\n\
;\n\
: main ( -- ) 0 1000000 Even sum-parity . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-clause-multi-tail-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn mixed_clause_back_edge_and_base_case_runs_in_constant_stack_native() {
    // Criterion 3's mixed-clause golden (R9 / risk 5): both of the `Go`
    // clause's `if` arms are self-tail-calls, so both back-edge to the loop
    // header (one recurses with the `Go` tag, one recurses with the `Halt`
    // tag; neither arm itself `Ret`s). The only genuine base case is the
    // separate `Halt` clause, which `Ret`s with no self-call at all. The loop
    // header's predecessors (entry + `Go`'s two back-edges) and the Slice-4
    // dispatch-join's predecessors must stay disjoint for this to compile and
    // run correctly.
    let src = "type: Step | Go | Halt ;\n\
: run-mix ( i64 i64 Step -- i64 )\n\
  | Go   | acc n | n 0 = if acc n Halt run-mix else acc n + n 1 - Go run-mix end\n\
  | Halt | acc n | acc\n\
;\n\
: main ( -- ) 0 1000000 Go run-mix . ;\n";
    let path = std::env::temp_dir().join(format!(
        "sooth-mixed-clause-tail-{}.sth",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "500000500000\n");
    assert_eq!(code, 0);
}

#[test]
fn enum_get_from_carried_array_clause_dispatch_constant_stack() {
    // Slice 7 criterion 8 (the crux): `get` an `Op` out of the *carried*
    // program array, clause-match it, and tail-recurse. The array-across-the
    // -back-edge half was proven by a prior spike; the residual unproven
    // composition is enum-`get`-from-carried-array + clause dispatch in
    // constant stack. 1_000_000 back-edges each read the enum out of the
    // carried `[Op 2]`, dispatch, and self-tail-call; naive recursion at that
    // depth overflows the default 8MB host stack, which `run_and_capture_stdout`
    // catches as a signal death (no exit code) and turns a no-op Slice 6
    // transform red. `idx` goes bool -> index via `if 1 else 0 end >usize`
    // (a conversion word on a `bool` is a checker error), and `fetch` reads
    // the enum with non-consuming `get` (`swap drop` keeps the `Op`).
    let src = "type: Op | Step | Stop ;\n\
: idx ( i64 -- usize ) | count | count 0 = if 1 else 0 end >usize ;\n\
: fetch ( [Op 2] usize -- Op ) | a i | a i get swap drop ;\n\
: run ( [Op 2] i64 i64 Op -- i64 )\n\
  | Step | prog count acc |\n\
      prog\n\
      count 1 -\n\
      acc 1 +\n\
      prog count 1 - idx fetch\n\
      run\n\
  | Stop | prog count acc | acc\n\
;\n\
: build ( -- [Op 2] ) Step 2 fill 1 Stop set ;\n\
: start ( [Op 2] -- i64 ) | prog | prog 1000000 0 prog 0 fetch run ;\n\
: main ( -- ) build start . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-vm-smoke-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();

    assert_eq!(stdout, "1000000\n");
    assert_eq!(code, 0);
}

#[test]
fn vm_dogfood_compiles_and_runs() {
    // Phase 3 of Slice 7 (criteria 1, 2, 4, 5, 6): the VM's real dispatch
    // loop (`fetch` + the nine-clause self-tail-recursive `run`) interprets
    // the sum-1..N bytecode program (built via `fill`+`set`, no array
    // literal) at N = 10, exercising every opcode the sum program needs
    // (`Push`/`Add`/`Sub`/`Load`/`Store`/`Jz`/`Jmp`/`Halt`) and the `Jz`/`Jmp`
    // backward branch. `Mul` dispatches too (its clause is identical in shape
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
  vm vm Vm>stack vm Vm>sp x set Vm<stack\n\
  vm Vm>sp 1 + Vm<sp ;\n\
: vm-pop ( Vm -- VmPop )\n\
  | vm |\n\
  vm vm Vm>sp 1 - Vm<sp\n\
  vm Vm>stack vm Vm>sp 1 - get\n\
  swap drop\n\
  VmPop ;\n\
: bump-pc ( Vm -- Vm )\n\
  dup Vm>pc 1 + Vm<pc ;\n\
: fetch ( Vm -- Fetched )\n\
  | vm |\n\
  vm\n\
  vm Vm>prog vm Vm>pc get swap drop\n\
  Fetched ;\n\
: run ( Vm Op -- i64 )\n\
| Push  | vm v |\n\
    vm v vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
| Add   | vm |\n\
    vm vm-pop VmPop>\n\
    swap vm-pop VmPop>\n\
    rot\n\
    +\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
| Sub   | vm |\n\
    vm vm-pop VmPop>\n\
    swap vm-pop VmPop>\n\
    rot\n\
    -\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
| Mul   | vm |\n\
    vm vm-pop VmPop>\n\
    swap vm-pop VmPop>\n\
    rot\n\
    *\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
| Load  | vm addr |\n\
    vm vm Vm>mem addr get swap drop\n\
    vm-push\n\
    bump-pc\n\
    fetch Fetched> run\n\
| Store | vm addr |\n\
    vm vm-pop VmPop>\n\
    over Vm>mem\n\
    addr\n\
    rot\n\
    set\n\
    Vm<mem\n\
    bump-pc\n\
    fetch Fetched> run\n\
| Jz    | vm target |\n\
    vm vm-pop VmPop>\n\
    0 =\n\
    if\n\
      target Vm<pc\n\
    else\n\
      bump-pc\n\
    end\n\
    fetch Fetched> run\n\
| Jmp   | vm target |\n\
    vm target Vm<pc\n\
    fetch Fetched> run\n\
| Halt  | vm |\n\
    vm vm-pop VmPop>\n\
    swap drop\n\
;\n\
: build ( -- [Op 13] )\n\
  Halt 13 fill\n\
  0  >usize 0  >usize Load  set\n\
  1  >usize 11 >usize Jz    set\n\
  2  >usize 1  >usize Load  set\n\
  3  >usize 0  >usize Load  set\n\
  4  >usize Add set\n\
  5  >usize 1  >usize Store set\n\
  6  >usize 0  >usize Load  set\n\
  7  >usize 1 Push set\n\
  8  >usize Sub set\n\
  9  >usize 0  >usize Store set\n\
  10 >usize 0  >usize Jmp   set\n\
  11 >usize 1  >usize Load  set\n\
;\n\
: main ( -- )\n\
  build\n\
  0 >usize\n\
  0 8 fill\n\
  0 >usize\n\
  0 4 fill 0 >usize 10 set\n\
  Vm\n\
  fetch Fetched> run . ;\n";
    let path = std::env::temp_dir().join(format!("sooth-vm-small-n-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
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
    // Criterion 5: the existing `examples/factorial.sth` (`dup 1 - factorial
    // *`) has a self-call followed by `*`, so it is deliberately not in tail
    // position and stays a real, un-eliminated `Call` (R10); it still
    // computes correctly at small N. The over-eager-miscompile boundary
    // (self-call inside a non-terminal `if`) is covered by the
    // `self_call_in_non_terminal_if_stays_a_call` unit test in `src/ir.rs`.
    let (stdout, code) = run_and_capture_stdout("examples/factorial.sth");
    assert_eq!(stdout, "120\n");
    assert_eq!(code, 0);
}

// Phase 3 Slice 1: the linear core on bare `__spy` values. Every
// drop-observing golden compares the *whole* stdout, so "exactly once" and drop
// order are actually proven; every negative golden asserts the diagnostic and
// the backticked type name.

fn run_linear_golden(tag: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-linear-{tag}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let (stdout, code) = run_and_capture_stdout(path.to_str().unwrap());
    std::fs::remove_file(&path).ok();
    assert_eq!(code, 0, "linear golden `{tag}` should exit 0");
    stdout
}

fn linear_check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

#[test]
fn dup_of_linear_value_is_error() {
    // Criterion 1: `dup` is the explicit copy and a linear value has no copy.
    let err = linear_check_error(": main ( -- )\n  7 __spy dup drop drop ;\n");
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
    assert!(err.contains("linear"), "unexpected message: {err}");
}

#[test]
fn over_of_linear_value_is_error() {
    // Criterion 1b: `over` copies its second slot, so it is gated too.
    let err = linear_check_error(": main ( -- )\n  7 __spy 1 over drop drop drop ;\n");
    assert!(err.contains("cannot `over`"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
}

#[test]
fn use_after_move_of_linear_local_is_error() {
    // Criterion 2: the second mention errors and names the site of the first.
    let err = linear_check_error(
        ": main ( -- )\n  7 __spy hold ;\n\
: hold ( __spy -- )\n  | s |\n  s drop\n  s drop ;\n",
    );
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
    assert!(
        err.contains("moved at line 5, col 3"),
        "the diagnostic should name the move site: {err}"
    );
}

#[test]
fn explicit_drop_runs_destructor_once() {
    // Criterion 3: the destructor runs exactly once, exactly where the `drop`
    // is written (between the two ordinary prints).
    let stdout = run_linear_golden(
        "drop-once",
        ": main ( -- )\n  1 .\n  7 __spy drop\n  2 . ;\n",
    );
    assert_eq!(stdout, "1\ndrop 7\n2\n");
}

#[test]
fn surplus_linear_on_stack_is_error() {
    // Criterion 4a: forgetting is an error, not a silent drop.
    let err = linear_check_error(": main ( -- )\n  7 __spy ;\n");
    assert!(
        err.contains("linear value left on the stack"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
}

#[test]
fn unconsumed_linear_local_is_error() {
    // Criterion 4b: a linear local never consumed by scope end. Locals are not
    // on the final stack, so this is its own pass, not `check_outputs`.
    let err = linear_check_error(": hold ( __spy -- )\n  | s |\n  1 . ;\n");
    assert!(err.contains("never consumed"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
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
        ": main ( -- )\n  7 __spy 8 __spy swap drop drop\n\
  1 __spy 2 __spy 3 __spy rot drop drop drop ;\n",
    );
    assert_eq!(stdout, "drop 7\ndrop 8\ndrop 1\ndrop 3\ndrop 2\n");
}

#[test]
fn both_arms_consume_linear_ok() {
    // Criterion 10a: consumed in both arms compiles, and each call disposes its
    // own spy exactly once.
    let stdout = run_linear_golden(
        "both-arms",
        ": dispose ( __spy bool -- )\n  | s c |\n  c if s drop else 99 . s drop end ;\n\
: main ( -- )\n  7 __spy true dispose\n  8 __spy false dispose ;\n",
    );
    assert_eq!(stdout, "drop 7\n99\ndrop 8\n");
}

#[test]
fn divergent_arm_use_is_error() {
    // Criterion 10b: consumed in one arm only, then referenced past the join.
    // The join yields `MaybeMoved`, so the later use is a use-after-move.
    let err = linear_check_error(
        ": oops ( __spy bool -- )\n  | s c |\n  c if s drop else 1 . end\n  s drop ;\n",
    );
    assert!(err.contains("use after move"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
}

#[test]
fn divergent_arm_unconsumed_is_error() {
    // Criterion 10c: consumed in one arm only and never referenced again. The
    // compiler errors at scope end rather than inserting a compensating drop.
    let err =
        linear_check_error(": oops ( __spy bool -- )\n  | s c |\n  c if s drop else 1 . end ;\n");
    assert!(err.contains("never consumed"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
}

#[test]
fn linear_across_loop_back_edge_is_located_error() {
    // Criterion 12: a linear value live across the self-tail-call back-edge is
    // deferred (R15/D8), as a located error rather than a miscompile.
    let err = linear_check_error(
        ": spin ( __spy i64 -- i64 )\n  | s n |\n\
  n 0 = if s drop 0 else 9 __spy n 1 - spin end ;\n",
    );
    assert!(
        err.contains("not supported yet"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
    assert!(err.contains("line 3"), "the error should be located: {err}");
}

#[test]
fn copy_loop_still_compiles() {
    // Criterion 12 (other half): a `countdown`-shaped Copy loop is unaffected
    // by the back-edge guard.
    let stdout = run_linear_golden(
        "copy-loop",
        ": countdown ( i64 -- i64 )\n  | n |\n  n 0 = if 0 else n 1 - countdown end ;\n\
: main ( -- )\n  100 countdown . ;\n",
    );
    assert_eq!(stdout, "0\n");
}

// Phase 3 Slice 1, Phase 2: struct aggregates via destructure-whole. A struct
// is linear iff any field is (transitively); `S>fi`/`S<fi`/`drop` on a linear
// struct run compiler-synthesized field drop glue. Every drop-observing
// golden compares the *whole* stdout, so drop count and order are proven, not
// just "it compiled".

#[test]
fn destructure_whole_drops_each_field() {
    // Criterion 5: `S>` a struct of two distinctly-tagged spies pushes both
    // fields (first field deepest), and dropping them top-first proves the
    // destructure moved both fields out rather than just the top one.
    let stdout = run_linear_golden(
        "destructure-whole",
        "type: Pair a __spy b __spy ;\n\
: main ( -- )\n  1 __spy 2 __spy Pair\n  Pair> drop drop ;\n",
    );
    assert_eq!(stdout, "drop 2\ndrop 1\n");
}

#[test]
fn nested_struct_is_linear_transitively() {
    // Criterion 5b: a struct-of-struct-of-spy is linear too. `dup` is
    // rejected exactly like a bare spy, naming the outer struct type, proving
    // linearity propagates through a nested aggregate rather than stopping at
    // the immediate field.
    let err = linear_check_error(
        "type: Inner v __spy ;\ntype: Outer i Inner ;\n\
: main ( -- )\n  5 __spy Inner Outer dup\n  Outer> Inner> drop\n  Outer> Inner> drop ;\n",
    );
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Outer`"), "unexpected message: {err}");

    // And once actually consumed exactly once, the nested destructure/drop
    // chain runs correctly end to end.
    let stdout = run_linear_golden(
        "nested-struct",
        "type: Inner v __spy ;\ntype: Outer i Inner ;\n\
: main ( -- )\n  5 __spy Inner Outer\n  Outer> Inner> drop ;\n",
    );
    assert_eq!(stdout, "drop 5\n");
}

#[test]
fn get_field_drops_the_rest_on_linear_struct() {
    // Criterion 6: `S>fi` still consumes the whole aggregate on a linear
    // receiver, so the non-extracted field (`b`, tag 2) is dropped as part of
    // the getter itself, before the explicit `drop` of the extracted `a`
    // (tag 1) that follows.
    let stdout = run_linear_golden(
        "get-drops-rest",
        "type: Pair a __spy b __spy ;\n\
: main ( -- )\n  1 __spy 2 __spy Pair\n  Pair>a drop ;\n",
    );
    assert_eq!(stdout, "drop 2\ndrop 1\n");
}

#[test]
fn set_field_drops_overwritten_linear_field() {
    // Criterion 8: `S<fi` drops the field it overwrites (old `a`, tag 1)
    // before storing the new value (tag 9); the other field (`b`, tag 2)
    // transfers via the blit untouched, and both surface later at the final
    // destructure+drop.
    let stdout = run_linear_golden(
        "set-drops-overwritten",
        "type: Pair a __spy b __spy ;\n\
: main ( -- )\n  1 __spy 2 __spy Pair\n  9 __spy Pair<a\n  Pair> drop drop ;\n",
    );
    assert_eq!(stdout, "drop 1\ndrop 2\ndrop 9\n");
}

#[test]
fn drop_of_linear_struct_runs_field_glue_in_declaration_order() {
    // Criterion 13: `drop` on the whole struct (no destructure in sight) runs
    // the synthesized destructor, which drops fields in declaration order
    // (`a` tag 1, then `b` tag 2) — not stack/reverse order, proving the glue
    // is field-order-driven, not a generic "drop whatever's on the stack".
    let stdout = run_linear_golden(
        "drop-whole-struct",
        "type: Pair a __spy b __spy ;\n\
: main ( -- )\n  1 __spy 2 __spy Pair drop ;\n",
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
        "type: Inner z __spy ;\ntype: Outer i __spy n __spy w Inner ;\n\
: main ( -- )\n  1 __spy 2 __spy 3 __spy Inner Outer drop ;\n",
    );
    assert_eq!(stdout, "drop 1\ndrop 2\ndrop 3\n");
}

// Phase 3 Slice 1, Phase 3: `S|>fi`, the non-consuming peek. Copy fields only;
// a linear field is a compile error (workaround: `S>`).

#[test]
fn peek_copy_field_keeps_struct() {
    // Criterion 7a: `Pair|>a` peeks the Copy field `a` twice, leaving the
    // aggregate itself live both times (proven because the final `drop` of
    // the whole struct still finds its linear field `b` intact and disposes
    // it exactly once — a consuming `Pair>a` in its place would have dropped
    // `b` at the first peek, or left nothing for the trailing `drop` to see).
    let stdout = run_linear_golden(
        "peek-copy-field",
        "type: Pair a i64 b __spy ;\n\
: main ( -- )\n  5 3 __spy Pair\n  Pair|>a drop\n  Pair|>a drop\n  drop ;\n",
    );
    assert_eq!(stdout, "drop 3\n");
}

#[test]
fn peek_linear_field_is_error() {
    // Criterion 7b: peeking the linear field `b` is a compile error naming
    // both the peek workaround and the offending field's type.
    let err = linear_check_error(
        "type: Pair a i64 b __spy ;\n: main ( -- )\n  5 3 __spy Pair\n  Pair|>b drop drop ;\n",
    );
    assert!(
        err.contains("cannot `Pair|>b`"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
    assert!(err.contains("`S>`"), "unexpected message: {err}");
}

// Phase 3 Slice 1, Phase 4: enums via a synthesized, tag-dispatched
// destructor. A linear enum's `drop` doesn't know at compile time which
// variant is active, so the synthesized destructor tests the runtime tag and
// drops only the active variant's linear payload.

#[test]
fn dup_of_linear_enum_is_error() {
    // The hole Phase 2 left open: `is_copy` used to return `true` for every
    // enum, so an enum with a linear payload was silently duplicable and this
    // exact source compiled, ran, and printed nothing (an exactly-once
    // violation with no diagnostic). It is now rejected like any other linear
    // value, naming the enum type.
    let err = linear_check_error(
        "type: Box | Full v __spy | Empty ;\n\
: main ( -- )\n  1 __spy Full dup drop drop ;\n",
    );
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
    let src = "type: Inner v __spy ;\ntype: Held | Empty | Some i Inner ;\n\
type: Wrap h Held ;\n";
    let err = linear_check_error(&format!(
        "{src}: main ( -- )\n  5 __spy Inner Some Wrap dup drop drop ;\n"
    ));
    assert!(err.contains("cannot `dup`"), "unexpected message: {err}");
    assert!(err.contains("`Wrap`"), "unexpected message: {err}");

    let stdout = run_linear_golden(
        "nested-enum",
        &format!("{src}: main ( -- )\n  5 __spy Inner Some Wrap drop ;\n"),
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
        "type: Item | Empty | Full v __spy ;\n\
: main ( -- )\n  1 .\n  true if 5 __spy Full else Empty end drop\n  2 .\n\
  false if 9 __spy Full else Empty end drop\n  3 . ;\n",
    );
    assert_eq!(stdout, "1\ndrop 5\n2\n3\n");
}

#[test]
fn clause_body_disposes_linear_payload() {
    // Criterion 9b: a clause-style word matching on the enum exposes the
    // active variant's payload on the stack; the matched clause is
    // responsible for disposing it like any other linear value (here via a
    // bare `drop`, no local name needed). The `Empty` clause runs no glue at
    // all (zero fields), proving the drop in the `Full` clause is the clause
    // body's own doing, not compiler-inserted compensation.
    let stdout = run_linear_golden(
        "clause-disposes-payload",
        "type: Item | Empty | Full v __spy ;\n\
: handle ( Item -- )\n| Empty   99 .\n| Full    drop\n;\n\
: main ( -- )\n  Empty handle\n  7 __spy Full handle ;\n",
    );
    assert_eq!(stdout, "99\ndrop 7\n");
}

#[test]
fn unconsumed_linear_clause_payload_is_error() {
    // The clause-body half of criterion 9b: a payload bound to a clause local
    // is subject to the same scope-end rule as any other linear local, so
    // forgetting it is a compile error naming the local and the word. This is
    // the branch Phase 1 left unreachable (a linear clause payload needed the
    // enum linearity this phase adds).
    let err = linear_check_error(
        "type: Item | Empty | Full v __spy ;\n\
: handle ( Item -- )\n| Empty   99 .\n| Full | s |   1 .\n;\n",
    );
    assert!(err.contains("never consumed"), "unexpected message: {err}");
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
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
    let err = linear_check_error(
        ": hold ( __spy __spy -- )\n  | s s |\n  s drop ;\n\
: main ( -- ) 1 __spy 2 __spy hold 99 . ;\n",
    );
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
fn duplicate_clause_body_local_is_error() {
    // The clause-body twin of `duplicate_word_entry_local_is_error`: the
    // same last-wins hazard exists in the `| Variant | s s |` binding path.
    let err = linear_check_error(
        "type: R | Two a __spy b __spy ;\n\
: use ( R -- ) | Two | s s | s drop ;\n\
: main ( -- ) 1 __spy 2 __spy Two use ;\n",
    );
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
    let err = linear_check_error(": main ( -- __spy ) 7 __spy ;\n");
    assert!(
        err.contains("cannot declare a linear type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
    assert!(err.contains("`main`"), "unexpected message: {err}");
}

#[test]
fn main_declaring_linear_input_is_error() {
    // Nothing calls `main`, so a linear input arrives in an uninitialised
    // ABI register; running its destructor would be undefined behaviour.
    let err = linear_check_error(": main ( __spy -- ) | s | s drop ;\n");
    assert!(
        err.contains("cannot declare a linear type"),
        "unexpected message: {err}"
    );
    assert!(err.contains("`__spy`"), "unexpected message: {err}");
    assert!(err.contains("`main`"), "unexpected message: {err}");
}
