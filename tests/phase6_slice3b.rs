//! Phase 6 Slice 3b goldens: a *generic* enum is eliminated by the generated
//! eliminator word, spelled exactly like the concrete case. The arm tag is
//! recognized by name at parse time and typed at check time against the
//! scrutinee's own instantiation, so `Option[i64]`'s `( Some )` needs no type
//! arguments anywhere in the annotation.

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

/// T5: the core witness. Both arms of a generic `Option[i64]` run, and the
/// instantiation comes from the scrutinee's declared type alone -- before this
/// slice the `( None )` tag was not even recognized as a tag, since the
/// concrete registry could not type a generic enum's variant, and the build
/// failed as `unknown type `None``.
#[test]
fn generic_enum_eliminator_runs_both_arms() {
    let (stdout, code) = build_and_run(
        "s3b-generic-eliminator",
        "type: Option 'T | None | Some val 'T ;\n\
         : to-int ( Option[i64] -- i64 )\n  \
           ~[ ( Some ) Some> ]\n  \
           ~[ ( None ) None> 0 ]\n  \
           Option? ;\n\
         : main ( -- )\n  \
           42 Some to-int .\n  \
           None to-int . ;\n",
    );
    assert_eq!(stdout, "42\n0\n");
    assert_eq!(code, 0);
}

/// T10: the untested product of slice 3's mode-polymorphic scrutinee and this
/// slice's generic capability -- a generic enum eliminated through `&` arms
/// (reading through the narrowed reference) and `&!` arms (mutating a payload
/// in place), leaving `main`'s own values intact to be measured again. Arms are
/// written in reverse declaration order, so a positional (rather than
/// tag-based) routing prints the other variant's field.
#[test]
fn generic_enum_eliminator_by_reference_reads_and_mutates_in_place() {
    let (stdout, code) = build_and_run(
        "s3b-generic-eliminator-ref",
        "type: Cell 'T | Pair a 'T b 'T | One v 'T ;\n\
         : total ( &Cell[i64] -- i64 )\n  \
           ~[ ( &One )  &v @ ]\n  \
           ~[ ( &Pair ) dup &a @ swap &b @ + ]\n  \
           Cell? ;\n\
         : bump ( &!Cell[i64] -- )\n  \
           ~[ ( &!One )  &!v 1 +! ]\n  \
           ~[ ( &!Pair ) &!a 10 +! ]\n  \
           Cell? ;\n\
         : discard ( Cell[i64] -- )\n  \
           ~[ ( One )  One> drop ]\n  \
           ~[ ( Pair ) Pair> drop drop ]\n  \
           Cell? ;\n\
         : main ( -- )\n  \
           3 4 Pair | p |\n  \
           &p total .\n  \
           &!p bump\n  \
           &p total .\n  \
           p discard\n  \
           7 One | o |\n  \
           &o total .\n  \
           &!o bump\n  \
           &o total .\n  \
           o discard ;\n",
    );
    assert_eq!(stdout, "7\n17\n7\n8\n");
    assert_eq!(code, 0);
}

/// T8: recognition is now generic-aware, so a *generic* enum's stray tag (which
/// previously could not parse at all) reaches the guard that rejects a tagged
/// literal never consumed as an arm, rather than being silently unchecked.
#[test]
fn stray_generic_arm_tag_outside_an_eliminator_call_is_error() {
    let err = build_err(
        "s3b-stray-generic-tag",
        "type: Option 'T | None | Some val 'T ;\n\
         : stray ( Option[i64] -- Option[i64] i64 ) ~[ ( Some ) 0 ] drop 1 ;\n\
         : main ( -- ) 42 Some stray . drop ;\n",
    );
    assert!(
        err.contains(
            "this quotation is annotated `( Some )`, an eliminator-arm tag, but it is not consumed by a call to a generated eliminator"
        ),
        "unexpected: {err}"
    );
}

/// T11: a missing arm over a generic enum names the *surface* variant and enum,
/// not the instantiation-suffixed (`Err[i64 bool]`) or mangled (`Result__m0`)
/// spelling -- the same wording the concrete case is pinned to.
#[test]
fn non_exhaustive_generic_eliminator_names_the_surface_variant() {
    let err = build_err(
        "s3b-nonexhaustive-generic",
        "type: Result 'T 'E | Ok val 'T | Err val 'E ;\n\
         : to-int ( Result[i64 bool] -- i64 ) ~[ ( Ok ) Ok> ] Result? ;\n\
         : main ( -- ) 42 Ok to-int . ;\n",
    );
    assert!(
        err.contains("non-exhaustive call to `Result?`"),
        "not surface-named: {err}"
    );
    assert!(
        err.contains("missing variant `Err` of enum `Result`"),
        "not surface-named: {err}"
    );
}

/// T6/T7 (R5, decision 5): **one word** eliminates two *asymmetric*
/// instantiations of the same generic enum -- `Result[i64 bool]` and
/// `Result[bool i64]`, which can only be told apart because they are not the
/// same instantiation (`Result[i64 i64]` could not distinguish `Ok 'T | Err
/// 'E` from its swap). The word calls `Result?` twice, once per
/// instantiation, so the eliminator registry keys both calls to the same
/// base-family entry (`"Result__m0?"`, last write wins): this only routes
/// each call correctly if its operative `EnumId` is read from its own
/// scrutinee rather than from whichever instantiation the registry happened
/// to retain (R5). T7 rides along: `Result[i64 bool]`'s stored variant name
/// is `Ok[i64 bool]`, so a bare `( Ok )` arm only matches it if both sides
/// normalize to the surface name `Ok`.
#[test]
fn two_asymmetric_instantiations_eliminate_independently_in_one_word() {
    let (stdout, code) = build_and_run(
        "s3b-two-asymmetric-instantiations",
        "type: Result 'T 'E | Ok val 'T | Err val 'E ;\n\
         : elim-both ( Result[i64 bool] Result[bool i64] -- i64 i64 )\n  \
           ~[ ( Ok ) Ok> ~[ 10 ] ~[ 20 ] if ]\n  \
           ~[ ( Err ) Err> ]\n  \
           Result?\n  \
           swap\n  \
           ~[ ( Ok ) Ok> ]\n  \
           ~[ ( Err ) Err> ~[ 1 ] ~[ 0 ] if ]\n  \
           Result?\n  \
           swap ;\n\
         : main ( -- )\n  \
           42 Ok 7 Err elim-both . .\n  \
           true Err true Ok elim-both . . ;\n",
    );
    assert_eq!(stdout, "7\n42\n10\n1\n");
    assert_eq!(code, 0);
}
