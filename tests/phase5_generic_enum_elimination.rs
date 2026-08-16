//! Generic enum clause-style elimination: a generic `type:` enum (e.g.
//! `Result 'T 'E`) can be pattern-matched with a `| Ok ... | Err ...`
//! clause-style word, not merely constructed. Phase 5 Slice 1 shipped
//! construction and explicitly deferred elimination; this closes that gap so
//! `Result`/`Option` become usable in Slice 2.

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

/// The core witness: a generic two-variant enum is eliminated by a clause-style
/// word, and each clause's bound payload flows through. Guards both halves of
/// the fix at once -- if `is_variant_name` stops consulting the generic enum
/// registry, `| Ok` reparses as a locals block (`local `Ok` collides ...`);
/// if `check_clause_word` stops matching clauses by surface name, the checker
/// reports `unknown variant `Ok`` against the mangled instantiation name.
#[test]
fn generic_enum_clause_elimination_runs() {
    let (stdout, code) = build_and_run(
        "gen-enum-elim-basic",
        "type: Result 'T 'E | Ok val 'T | Err val 'E ;\n\
         : to-int ( Result[i64 i64] -- i64 )\n\
         | Ok   | v |  v\n\
         | Err  | e |  e 100 +\n\
         ;\n\
         : main ( -- )\n\
           42 Ok  to-int .\n\
           7  Err to-int . ;\n",
    );
    assert_eq!(stdout, "42\n107\n");
    assert_eq!(code, 0);
}

/// Order-independence, mirroring the concrete-enum "D8" guarantee: the generic
/// `type:` header is declared textually *after* the word that pattern-matches
/// on it, and elimination still parses and runs. Guards that recognition rides
/// on `parse_generic_typedefs` (run before any word body) rather than on the
/// generic registry being populated by the time a body is walked.
#[test]
fn generic_enum_elimination_type_declared_after_matching_word() {
    let (stdout, code) = build_and_run(
        "gen-enum-elim-forward",
        ": to-int ( Result[i64 i64] -- i64 )\n\
         | Ok   | v |  v\n\
         | Err  | e |  e 100 +\n\
         ;\n\
         type: Result 'T 'E | Ok val 'T | Err val 'E ;\n\
         : main ( -- )\n\
           42 Ok  to-int .\n\
           7  Err to-int . ;\n",
    );
    assert_eq!(stdout, "42\n107\n");
    assert_eq!(code, 0);
}

/// Two distinct instantiations of one generic enum are each eliminated by their
/// own clause word and dispatch independently -- the surface-name matching must
/// not collapse `Result[i64 i64]`'s `Ok` with `Result[bool bool]`'s `Ok`,
/// since each instantiation is a distinct concrete enum with its own variant
/// layout.
#[test]
fn two_generic_enum_instantiations_eliminate_independently() {
    let (stdout, code) = build_and_run(
        "gen-enum-elim-two-insts",
        "type: Result 'T 'E | Ok val 'T | Err val 'E ;\n\
         : to-int ( Result[i64 i64] -- i64 )\n\
         | Ok   | v |  v\n\
         | Err  | e |  e 100 +\n\
         ;\n\
         : to-flag ( Result[bool bool] -- i64 )\n\
         | Ok   | b |  b drop  1\n\
         | Err  | e |  e drop  0\n\
         ;\n\
         : main ( -- )\n\
           42 Ok    to-int .\n\
           7  Err   to-int .\n\
           true Ok  to-flag .\n\
           false Err to-flag . ;\n",
    );
    assert_eq!(stdout, "42\n107\n1\n0\n");
    assert_eq!(code, 0);
}

/// A non-exhaustive generic-enum clause word is rejected, and the diagnostic
/// names the *surface* variant and enum (`Err` of `Result`), not the mangled
/// instantiation spelling. Guards the exhaustiveness loop's surface-name
/// lookup: reverting it makes an actually-complete word fail as "missing
/// variant `Ok[i64 i64]`" (the positive tests catch that), while this pins the
/// message text a user actually sees on a genuinely missing clause.
#[test]
fn non_exhaustive_generic_enum_clause_names_surface_variant() {
    let err = build_err(
        "gen-enum-elim-nonexhaustive",
        "type: Result 'T 'E | Ok val 'T | Err val 'E ;\n\
         : to-int ( Result[i64 i64] -- i64 )\n\
         | Ok   | v |  v\n\
         ;\n\
         : main ( -- )  42 Ok to-int . ;\n",
    );
    assert!(err.contains("missing variant `Err`"), "unexpected: {err}");
    assert!(err.contains("enum `Result`"), "not surface-named: {err}");
    assert!(
        !err.contains("Err[i64"),
        "leaked mangled variant name: {err}"
    );
}
