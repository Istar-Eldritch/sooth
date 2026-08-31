//! Phase 6 Slice 1, phase 3 goldens: quotation effect annotations.
//!
//! The three exit cases from the spec: a standalone annotation/body
//! disagreement, a parameter-filling annotation/parameter disagreement (only
//! catchable through R4's poly positional bridge, since R3/R11 both absorb an
//! identity body), and the agreeing case (both standalone and
//! parameter-filling) building and running unchanged.

mod common;
fn build_check_error(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail its check");
    std::fs::remove_file(&path).ok();
    err
}

fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
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

/// Exit case 1 (R3): a standalone annotation that disagrees with its own
/// literal's body, with no consuming parameter present at all.
#[test]
fn annotated_literal_body_mismatch_diagnostic() {
    let src = ": w ( -- ) [ ( i64 -- i64 ) dup 10 lt ] drop ;\n";
    let err = build_check_error("phase6_slice1_body_mismatch", src);
    assert_eq!(
        err,
        "error: this quotation is annotated `[ i64 -- i64 ]` but its body has effect `[ i64 -- i64 Bool ]` in `w` (line 1)"
    );
}

/// Exit case 2 (R4): `'T` grounds to `bool` through `on`'s poly parameter, the
/// annotation claims `i64` at that position, and the identity body `dup drop`
/// keeps R3/R11 from firing on their own -- only R4's positional bridge
/// catches the conflict.
#[test]
fn annotated_literal_parameter_mismatch_diagnostic() {
    let src = ": on inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;\n\
        : main ( -- ) True ~[ ( i64 -- i64 ) dup drop ] on drop ;\n";
    let err = build_check_error("phase6_slice1_param_mismatch", src);
    assert_eq!(
        err,
        "error: the quotation passed to `on` is annotated `~[ i64 -- i64 ]` but `on` declares it `~[ Bool -- Bool ]` in `main` (line 2)"
    );
}

/// Exit: additive, agreeing case is accepted. One literal is checked
/// standalone against a concrete annotation that matches its body, and a
/// second fills `on`'s poly parameter with an annotation that matches the
/// grounded `Bool` effect.
#[test]
fn annotated_literal_agreeing_builds() {
    let src = ": on inline ( 'T ~[ 'T -- 'T ] -- 'T ) | f | f call ;\n\
        : main ( -- )\n\
        \x20 [ ( i64 -- i64 Bool ) dup 10 lt ] drop\n\
        \x20 True ~[ ( Bool -- Bool ) dup drop ] on . ;\n";
    let (stdout, code) = run_src("phase6_slice1_agreeing", src);
    assert_eq!(stdout, "true\n");
    assert_eq!(code, 0);
}
