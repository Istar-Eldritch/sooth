//! Phase 5 Slice 2, phase 3 goldens: `Result 'T 'E` and `Option 'T` as real,
//! importable generic library enums (`lib/result.sth`, `lib/option.sth`),
//! each exercising construction, monomorphization, and clause-style
//! elimination through to concrete stdout, plus a cross-module import golden
//! that is the direct witness of Phase 2's qualified generic resolution.

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

/// The 2-variable case: a fallible word returns `Result[i64 i64]` (attributeless
/// `Ok 'T` / `Err 'E`, matching `lib/result.sth` exactly) and a clause
/// eliminator handles both arms.
#[test]
fn result_constructs_monomorphizes_and_eliminates_both_arms() {
    let (stdout, code) = build_and_run(
        "slice2-result-basic",
        "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
         : safe-add ( i64 i64 -- Result[i64 i64] )\n\
           dup 0 < ~[ drop drop -1 Err ] ~[ + Ok ] if ;\n\
         : to-int ( Result[i64 i64] -- i64 )\n\
         | Ok  |v| v\n\
         | Err |e| e ;\n\
         : main ( -- )\n\
           5 7 safe-add to-int .\n\
           5 -3 safe-add to-int . ;\n",
    );
    assert_eq!(stdout, "12\n-1\n");
    assert_eq!(code, 0);
}

/// The 1-variable case: `Option[i64]` constructed, monomorphized, and
/// eliminated through both the `Some` and `None` arms.
#[test]
fn option_constructs_monomorphizes_and_eliminates_both_arms() {
    let (stdout, code) = build_and_run(
        "slice2-option-i64",
        "type: Option 'T | None | Some 'T ;\n\
         : unwrap-or ( i64 Option[i64] -- i64 )\n\
         | Some |v| drop v\n\
         | None ;\n\
         : main ( -- )\n\
           9 5 Some unwrap-or .\n\
           9 None unwrap-or . ;\n",
    );
    assert_eq!(stdout, "5\n9\n");
    assert_eq!(code, 0);
}

/// `Option` instantiated over a pointer type (`^Node`), the nullability
/// shape DESIGN.md names as `Option`'s actual reason for existing: `^T`
/// stays non-null, `Option['T]` is the named answer. Every existing generic
/// instantiation test in this codebase applies `i64`/`bool`/aggregates only,
/// never a pointer argument, so this shape is otherwise unwitnessed.
#[test]
fn option_instantiates_over_a_pointer_type() {
    let (stdout, code) = build_and_run(
        "slice2-option-pointer",
        "type: Option 'T | None | Some 'T ;\n\
         type: Node val i64 ;\n\
         : unwrap-or ( i64 Option[^Node] -- i64 )\n\
         | Some |v| drop v ^> Node>val\n\
         | None ;\n\
         : main ( -- )\n\
           0 7 Node ^ Some unwrap-or .\n\
           0 None unwrap-or . ;\n",
    );
    assert_eq!(stdout, "7\n0\n");
    assert_eq!(code, 0);
}

/// The cross-module witness of Phase 2: a program `import:`s `Result` from
/// the real, committed `lib/result.sth` by ordinary relative (here,
/// absolute-for-temp-dir) path, applies it qualified, and monomorphizes
/// correctly -- in both discovery orders, since the whole-closure header
/// pre-pass (OQ1) is what makes the applier-first order work at all.
fn result_import(qualifier: &str) -> String {
    format!(
        "import: {qualifier} \"{}/lib/result.sth\" ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn result_imports_and_applies_qualified_across_a_module() {
    let (stdout, code) = build_and_run(
        "slice2-result-xmod",
        &format!(
            "{}\
             : to-int ( r::Result[i64 i64] -- i64 )\n\
             | Ok  |v| v\n\
             | Err |e| e ;\n\
             : main ( -- )\n\
               12 Ok to-int .\n\
               -3 Err to-int . ;\n",
            result_import("r")
        ),
    );
    assert_eq!(stdout, "12\n-3\n");
    assert_eq!(code, 0);
}
