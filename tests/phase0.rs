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
    // and prints via `f.`; mean of 10 and 4 prints 2.5.
    let (stdout, code) = run_and_capture_stdout("examples/mean.sth");
    assert_eq!(stdout, "2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn float_arithmetic_runs_on_both_widths_end_to_end() {
    // S1/S3: `+ - *` run correctly on `f64` and on `f32` (converted back to
    // `f64` for `f.`, since `f.` is `f64`-only, D6).
    let src = ": main ( -- )\n  1.0 2.0 + f.\n  5.0 2.0 - f.\n  3.0 4.0 * f.\n  \
1.5 >f32 2.5 >f32 + >f64 f. ;\n";
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
: main ( -- )\n  1.0 0.0 fdiv f.\n  0.0 0.0 fdiv f.\n  \
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
    // each printed via `>f64 f.` where the source is `f32` (D6).
    let src = ": main ( -- )\n  10 >f64 f.\n  3 >f32 >f64 f.\n  3.5 >f32 >f64 f.\n  \
10 >f32 >f64 f. ;\n";
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
    let src = ": main ( -- )\n  4000000000 >u32 >f64 f.\n  -1 >u64 >f64 f. ;\n";
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
