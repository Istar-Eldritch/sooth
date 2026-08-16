//! Phase 5 Slice 1 golden: a generic `type:` header parses but mints no
//! concrete registry entry until an explicit instantiation exists (Phase 2/3
//! of this slice). This is the end-to-end half of that claim -- a whole
//! program declaring a generic type and never applying it still builds and
//! runs clean, not just parses clean (the parser-level unit test,
//! `parser::tests::parse_generic_typedef_declared_but_never_used_parses_clean`,
//! only covers parsing).

fn build_and_run(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build(&path).expect("build should succeed");
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

fn build_err(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    std::fs::write(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build(&path).expect_err("build should fail");
    std::fs::remove_file(&path).ok();
    err
}

/// The end-to-end half of `check::declarations`'s
/// `duplicate_type_check_includes_generic_headers`, which hand-builds its
/// `GenericStructDecl`s and so cannot see whether a *parsed* generic header
/// ever reaches the checker. Without this, dropping the driver's
/// `generic_structs.extend` (or passing `&[]` to `check_types`) leaves the
/// whole suite green while a generic `Box` silently shadows a concrete one.
#[test]
fn generic_header_colliding_with_a_concrete_type_is_a_duplicate() {
    let err = build_err(
        "phase5-slice1-dup-struct",
        "type: Box x i64 ;\ntype: Box 'T val 'T ;\n: main ( -- ) 1 . ;\n",
    );
    assert!(err.contains("duplicate type `Box`"), "unexpected: {err}");
    assert!(err.contains("line 2, col 1"), "unlocated: {err}");
}

/// The enum twin, guarding the driver's `generic_enums.extend` and the
/// `generic_enums` argument to `check_types` independently of the struct side.
#[test]
fn generic_enum_header_colliding_with_a_concrete_type_is_a_duplicate() {
    let err = build_err(
        "phase5-slice1-dup-enum",
        "type: Opt | None | Some v i64 ;\ntype: Opt 'T | Nothing | Just v 'T ;\n: main ( -- ) 1 . ;\n",
    );
    assert!(err.contains("duplicate type `Opt`"), "unexpected: {err}");
    assert!(err.contains("line 2, col 1"), "unlocated: {err}");
}

#[test]
fn generic_type_declared_but_never_used_builds_and_runs() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-unused-generic",
        "type: Box 'T val 'T ;\n: main ( -- ) 42 . ;\n",
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

#[test]
fn generic_enum_declared_but_never_used_builds_and_runs() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-unused-generic-enum",
        "type: Result 'T 'E | Ok val 'T | Err val 'E ;\n: main ( -- ) 7 . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}
