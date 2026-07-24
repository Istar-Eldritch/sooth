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
    let src = ": signed_lt ( -- i64 )\n  200 >i8 5 >i8 < if 1 else 0 then ;\n\n\
: unsigned_lt ( -- i64 )\n  200 >u8 5 >u8 < if 1 else 0 then ;\n\n\
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
    let src = ": main ( -- )\n  200 >u8 >i8 >u16 65535 >u16 < if 1 else 0 then . ;\n";
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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`u8`"), "unexpected message: {err}");
}

#[test]
fn conversion_of_bool_reports_diagnostic() {
    // X4: `>i32` applied to a `bool` is a type error naming the source is not an integer.
    let src = ": w ( -- i32 ) true >i32 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("numeric"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn conversion_unknown_target_reports_diagnostic() {
    // X5: `>i128` reads as an unknown conversion target.
    let src = ": w ( -- i64 ) 5 >i128 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("i128"), "unexpected message: {err}");
}

#[test]
fn if_condition_not_bool_reports_diagnostic() {
    let src = ": oops ( -- i64 )\n  5 if 1 else 2 then ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("expected `bool`"), "unexpected message: {err}");
    assert!(err.contains("found `i64`"), "unexpected message: {err}");
}

#[test]
fn operand_type_mismatch_reports_diagnostic() {
    let src = ": oops ( -- i64 )\n  true 1 + ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn branch_join_type_mismatch_reports_diagnostic() {
    let src = ": oops ( bool -- i64 )\n  if 1 else true then ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("different types"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn declared_output_type_mismatch_reports_diagnostic() {
    let src = ": oops ( i64 -- bool )\n  1 + ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
0.0 0.0 fdiv dup = if 1 else 0 then . ;\n";
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
: main ( -- )\n  1.0 2.0 < if 1 else 0 then .\n  2.0 1.0 < if 1 else 0 then .\n  \
0.0 0.0 fdiv dup < if 1 else 0 then .\n  0.0 0.0 fdiv dup > if 1 else 0 then . ;\n";
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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains('/'), "unexpected message: {err}");
    assert!(err.contains("float"), "unexpected message: {err}");
    assert!(err.contains("`i64`"), "unexpected message: {err}");
}

#[test]
fn float_mod_reports_diagnostic() {
    // X4: `mod` stays integer-only; two `f64` operands is an error.
    let src = ": f ( -- f64 ) 6.0 2.0 mod ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("numeric"), "unexpected message: {err}");
    assert!(err.contains("`bool`"), "unexpected message: {err}");
}

#[test]
fn unknown_float_conversion_target_reports_diagnostic() {
    // X6: `>f128` is an unknown conversion target.
    let src = ": w ( -- f64 ) 5.0 >f128 ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("f128"), "unexpected message: {err}");
}

// Bitwise operators (`and`/`or`/`xor`/`not`/`shl`/`shr`) diagnostics + goldens.

#[test]
fn bitwise_op_on_float_reports_diagnostic() {
    let src = ": w ( -- f64 ) 3.0 5.0 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

    assert!(err.contains("integer"), "unexpected message: {err}");
    assert!(err.contains("`f64`"), "unexpected message: {err}");
}

#[test]
fn bitwise_op_on_bool_is_now_accepted() {
    // `and`/`or`/`xor` are type-directed: `bool` is now a valid homogeneous
    // operand class, not just the integer tower.
    let src = ": w ( -- bool ) true false and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&module).expect("check should succeed");
}

#[test]
fn mixed_bool_int_and_reports_both_types() {
    let src = ": w ( -- bool ) true 5 and ;";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
    let src = ": main ( -- )\n  1 >i8 7 shl 0 >i8 < if 1 . else 0 . then ;\n";
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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail");

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
  true true and if 1 else 0 then .\n  \
  true false and if 1 else 0 then .\n  \
  true false or if 1 else 0 then .\n  \
  false false or if 1 else 0 then .\n  \
  true false xor if 1 else 0 then .\n  \
  true true xor if 1 else 0 then . ;\n";
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
  true not if 1 else 0 then .\n  \
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
  200 >i8 5 >i8 <= if 1 else 0 then .\n  \
  200 >u8 5 >u8 <= if 1 else 0 then .\n  \
  200 >i8 5 >i8 >= if 1 else 0 then .\n  \
  200 >u8 5 >u8 >= if 1 else 0 then .\n  \
  200 >i8 5 >i8 <> if 1 else 0 then .\n  \
  200 >u8 5 >u8 <> if 1 else 0 then . ;\n";
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
  0.0 0.0 fdiv dup <= if 1 else 0 then .\n  \
  0.0 0.0 fdiv dup >= if 1 else 0 then .\n  \
  0.0 0.0 fdiv dup <> if 1 else 0 then .\n  \
  0.0 0.0 fdiv dup = if 1 else 0 then . ;\n";
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
    let module = parser::parse(&tokens).expect("parsing should succeed");
    let err = check::check(&module).expect_err("check should fail: `f.` no longer exists");

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
fn vectors_dogfood_compiles_and_runs() {
    // S8: `examples/vectors.sth` — a flat `Vec2` and a nested `Segment`, a
    // reusable componentwise `sub`, `len2`, `span` (= `Segment> swap sub`),
    // and a functional-setter `shift-x` demo. Builds segment (0,0)-(3,4),
    // prints `span len2 .` (25) and `5 6 Vec2 1 shift-x Vec2>x .` (6).
    let (stdout, code) = run_and_capture_stdout("examples/vectors.sth");
    assert_eq!(stdout, "25\n6\n");
    assert_eq!(code, 0);
}
