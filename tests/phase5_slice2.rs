//! Phase 5 Slice 2, phase 3 goldens: `Result 'T 'E` and `Option 'T` as real,
//! importable generic library enums (`lib/result.sth`, `lib/option.sth`),
//! each exercising construction, monomorphization, and elimination through to
//! concrete stdout, plus a cross-module import golden that is the direct
//! witness of Phase 2's qualified generic resolution.
//!
//! A cross-module eliminator needs the variant names in scope as arm tags, so
//! each library file exports its variants and each importer names them in a
//! selective import list (`import: r | Ok Err | "..." ;`); the dispatch call
//! itself is the unqualified `Result?`, keyed by the generic's surface name.

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

/// Builds and runs a multi-file source tree rooted at `entry`, one of `files`
/// (each written into a shared temp directory so relative `import:` paths
/// between them resolve).
fn build_and_run_dir(name: &str, files: &[(&str, &str)], entry: &str) -> (String, i32) {
    let dir = std::env::temp_dir().join(format!("sooth-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating temp dir should succeed");
    for (fname, src) in files {
        std::fs::write(dir.join(fname), src).expect("writing temp source should succeed");
    }
    let binary = sooth::driver::build(&dir.join(entry)).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_dir_all(&dir).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// The 2-variable case: a fallible word returns `Result[i64 i64]` (attributeless
/// `Ok 'T` / `Err 'E`, matching `lib/result.sth` exactly) and the generated
/// eliminator handles both arms.
#[test]
fn result_constructs_monomorphizes_and_eliminates_both_arms() {
    let (stdout, code) = build_and_run(
        "slice2-result-basic",
        "type: Result 'T 'E | Ok 'T | Err 'E ;\n\
         : safe-add ( i64 i64 -- Result[i64 i64] )\n\
           dup 0 lt ~[ drop drop -1 Err ] ~[ add Ok ] if ;\n\
         : to-int ( Result[i64 i64] -- i64 )\n\
           ~[ ( Ok )  Ok> ]\n\
           ~[ ( Err ) Err> ]\n\
           Result? ;\n\
         : main ( -- )\n\
           5 7 safe-add to-int .\n\
           5 -3 safe-add to-int . ;\n",
    );
    assert_eq!(stdout, "12\n-1\n");
    assert_eq!(code, 0);
}

/// The 1-variable case: `Option[i64]` constructed, monomorphized, and
/// eliminated through both the `Some` and `None` arms, imported from the
/// real, committed `lib/option.sth` rather than a copy declared inline (the
/// file is otherwise never imported by any test and its `export:` line goes
/// unwitnessed).
#[test]
fn option_constructs_monomorphizes_and_eliminates_both_arms() {
    let (stdout, code) = build_and_run(
        "slice2-option-i64",
        &format!(
            "import: o | Some None | \"{}/lib/option.sth\" ;\n\
             : unwrap-or ( i64 o::Option[i64] -- i64 )\n\
               ~[ ( Some ) Some> swap drop ]\n\
               ~[ ( None ) drop ]\n\
               Option? ;\n\
             : main ( -- )\n\
               9 5 Some unwrap-or .\n\
               9 None unwrap-or . ;\n",
            env!("CARGO_MANIFEST_DIR")
        ),
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
           ~[ ( Some ) Some> swap drop ^> &val @ swap drop ]\n\
           ~[ ( None ) drop ]\n\
           Option? ;\n\
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
/// correctly.
fn result_import(qualifier: &str) -> String {
    format!(
        "import: {qualifier} | Ok Err | \"{}/lib/result.sth\" ;\n",
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
               ~[ ( Ok )  Ok> ]\n\
               ~[ ( Err ) Err> ]\n\
               Result? ;\n\
             : main ( -- )\n\
               12 Ok to-int .\n\
               -3 Err to-int . ;\n",
            result_import("r")
        ),
    );
    assert_eq!(stdout, "12\n-3\n");
    assert_eq!(code, 0);
}

/// The witness that `Result`'s two variables bind *positionally*: every other
/// instantiation in this repo is symmetric (`Result[i64 i64]` above), so
/// rewriting `lib/result.sth` to `| Ok 'E | Err 'T` leaves them all green.
/// Here `Ok` carries `i64` and `Err` carries `str`, so a swap is a type error.
#[test]
fn result_binds_its_two_variables_positionally() {
    let (stdout, code) = build_and_run(
        "slice2-result-asymmetric",
        &format!(
            "{}\
             : report ( r::Result[i64 str] -- )\n\
               ~[ ( Ok )  Ok> . ]\n\
               ~[ ( Err ) Err> . ]\n\
               Result? ;\n\
             : main ( -- )\n\
               12 Ok report\n\
               \"boom\" Err report ;\n",
            result_import("r")
        ),
    );
    assert_eq!(stdout, "12\nboom");
    assert_eq!(code, 0);
}

/// The genuine two-discovery-order witness (OQ1): `use.sth` applies
/// `r::Result[i64 i64]` qualified against the real `lib/result.sth`, and the
/// entry file imports both `use.sth` and `lib/result.sth` directly, in each
/// order in turn, so the closure reaches the applier module either before or
/// after the declaring module has registered its header. The applier-first
/// arrangement is the one the whole-closure header pre-pass exists for:
/// without it, `use.sth` body-parses before `lib/result.sth`'s header is
/// registered, and a legal program fails with `unknown type` on nothing but
/// import order.
#[test]
fn result_cross_module_application_resolves_in_either_discovery_order() {
    let use_src = format!(
        "{}\
         : to-int ( r::Result[i64 i64] -- i64 )\n\
           ~[ ( Ok )  Ok> ]\n\
           ~[ ( Err ) Err> ]\n\
           Result? ;\n\
         : show-ok ( i64 -- ) Ok to-int . ;\n\
         : show-err ( i64 -- ) Err to-int . ;\n\
         export: show-ok ;\n\
         export: show-err ;\n",
        result_import("r")
    );

    let applier_first = format!(
        "import: u \"use.sth\" ;\n{}: main ( -- ) 12 u::show-ok -3 u::show-err ;\n",
        result_import("r")
    );
    let owner_first = format!(
        "{}import: u \"use.sth\" ;\n: main ( -- ) 12 u::show-ok -3 u::show-err ;\n",
        result_import("r")
    );

    for (tag, main_src) in [
        ("slice2-result-xmod-applier-first", applier_first),
        ("slice2-result-xmod-owner-first", owner_first),
    ] {
        let (stdout, code) = build_and_run_dir(
            tag,
            &[("use.sth", &use_src), ("main.sth", &main_src)],
            "main.sth",
        );
        assert_eq!(stdout, "12\n-3\n", "{tag}");
        assert_eq!(code, 0, "{tag}");
    }
}
