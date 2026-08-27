//! Phase 5 Slice 1 golden: a generic `type:` header parses but mints no
//! concrete registry entry until an explicit instantiation exists (Phase 2/3
//! of this slice). This is the end-to-end half of that claim -- a whole
//! program declaring a generic type and never applying it still builds and
//! runs clean, not just parses clean (the parser-level unit test,
//! `parser::tests::parse_generic_typedef_declared_but_never_used_parses_clean`,
//! only covers parsing).

mod common;
fn build_and_run(name: &str, src: &str) -> (String, i32) {
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

fn build_err(name: &str, src: &str) -> String {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let err = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect_err("build should fail");
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
        "type: Box x i64 ;\ntype: Box['T] val 'T ;\n: main ( -- ) 1 . ;\n",
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
        "type: Opt | None | Some v i64 ;\ntype: Opt['T] | Nothing | Just v 'T ;\n: main ( -- ) 1 . ;\n",
    );
    assert!(err.contains("duplicate type `Opt`"), "unexpected: {err}");
    assert!(err.contains("line 2, col 1"), "unlocated: {err}");
}

#[test]
fn generic_type_declared_but_never_used_builds_and_runs() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-unused-generic",
        "type: Box['T] val 'T ;\n: main ( -- ) 42 . ;\n",
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

/// R2/R4/R5 end to end: two instantiations of one generic struct reach the
/// backend as ordinary aggregates. Their registry names (`Box[i64]`) are not
/// valid QBE identifiers, so this is also what guards the aggregate-name
/// sanitization at the emission site -- a parse-level test cannot see it.
#[test]
fn generic_instantiations_reach_the_backend_and_run() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-instantiations",
        "type: Box['T] val 'T ;\ntype: Wrap i Box[i64] b Box[Bool] ;\n: f ( Box[i64] -- Box[i64] ) ;\n: main ( -- ) 42 . ;\n",
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

/// The enum twin: a minted enum is laid out and emitted like a hand-written
/// one, tag and payload included.
#[test]
fn generic_enum_instantiation_reaches_the_backend_and_runs() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-enum-instantiation",
        "type: Res['T 'E] | Ok val 'T | Err val 'E ;\n: f ( Res[i64 Bool] -- Res[i64 Bool] ) ;\n: main ( -- ) 9 . ;\n",
    );
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}

/// R3 end to end: the argument-count error survives the whole driver, not
/// just a direct parser call.
#[test]
fn generic_application_with_the_wrong_argument_count_is_a_build_error() {
    let err = build_err(
        "phase5-slice1-arity",
        "type: Pair['A 'B] a 'A b 'B ;\ntype: W x Pair[i64] ;\n: main ( -- ) 1 . ;\n",
    );
    assert!(
        err.contains("generic type `Pair` declares 2 type variables"),
        "unexpected: {err}"
    );
    assert!(err.contains("1 was supplied"), "unexpected: {err}");
    assert!(err.contains("line 2, col 11"), "unlocated: {err}");
}

#[test]
fn generic_enum_declared_but_never_used_builds_and_runs() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-unused-generic-enum",
        "type: Result['T 'E] | Ok val 'T | Err val 'E ;\n: main ( -- ) 7 . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

/// Phase 3 (R5/D7): two instantiations of one generic struct construct and
/// read back through their bare, shared-surface-name constructor (`Box`) and
/// a receiver-directed projection (`&val`) -- `Box` is unspellable as
/// anything else, `[` being a lexer delimiter, so this is the only call-site
/// spelling a real program can ever use. Before phase 3, the second
/// instantiation's registration silently clobbered the first's
/// (`env.insert`), so the wrong constructor resolved for one of the two
/// operand types; the exit case correct layout claim can only be seen by
/// reading back a value narrower than a pointer (`bool`), which a
/// clobbered-but-same-size constructor would still pass.
#[test]
fn two_generic_instantiations_share_a_surface_name_and_dispatch_correctly() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-shared-surface-dispatch",
        "type: Box['T] val 'T ;\ntype: WrapI x Box[i64] ;\ntype: WrapB y Box[Bool] ;\n\
         : main ( -- )\n  7 Box &val @ . drop\n  True Box &val @ . drop\n;\n",
    );
    assert_eq!(stdout, "7\nTrue\n");
    assert_eq!(code, 0);
}

/// R5, destructor half: a monomorphized instantiation's destructor is
/// synthesized and actually runs, not merely constructed and read back
/// (the case above). `Box>` (the generated destructure) is called from a
/// user `drop` overload exactly as a hand-written concrete struct's would
/// be, so this also exercises the destructor-synthesis path over a minted
/// `StructDecl`.
#[test]
fn generic_instantiation_destructor_runs_like_a_concrete_types() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-destructor",
        "type: Box['T] val 'T ;\ntype: WrapI x Box[i64] ;\n\
         : drop ( Box[i64] -- ) Box> . ;\n\
         : main ( -- ) 42 Box drop ;\n",
    );
    assert_eq!(stdout, "42\n");
    assert_eq!(code, 0);
}

/// R2/R5, signature-slot half: a generic-type application at a word's own
/// `( -- )` slot (`parse_slot`/`parse_poly_slot`), distinct from the field-
/// position case `generic_instantiations_reach_the_backend_and_run` already
/// covers -- a separate parser call site, and here also the only site that
/// mints the instantiation at all (no `type:` field ever names `Box[i64]`).
#[test]
fn generic_application_at_a_word_signature_slot_resolves_and_runs() {
    let (stdout, code) = build_and_run(
        "phase5-slice1-signature-slot",
        "type: Box['T] val 'T ;\n: unwrap ( Box[i64] -- i64 ) Box> ;\n\
         : main ( -- ) 7 Box unwrap . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}
