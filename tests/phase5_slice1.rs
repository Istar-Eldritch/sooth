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
