//! Phase 4 Slice 12, phase 2 exit criteria (part C).
//!
//! `~[ ... ]` is writable as a body literal (R-C1, X6, unit-tested directly in
//! `src/parser.rs`'s new `Token::TildeLBracket` arm) and the tilde is required
//! exactly at a `Type::InlineQuotation` parameter, in both directions (R-C2,
//! X7 = M-C). The migrated corpus stays byte-identical (X8, covered by
//! `tests/phase4_slice10c_corpus_stdout.rs`'s existing fixtures, unchanged by
//! this phase). `examples/capturing_dispatch.sth`'s stored/returned ordinary
//! quotations stay unmigrated and still run (X9, R-C4).

mod common;
fn check_error(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).expect("lexing should succeed");
    let mut module = sooth::test_support::parse_with_core(&tokens).expect("parsing should succeed");
    sooth::check::check(&mut module).expect_err("check should fail")
}

/// Build and run `src`, returning its stdout.
fn run(name: &str, src: &str) -> String {
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
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// X7 / M-C, direction 1 (E3a): an ordinary `[ ... ]` literal at a `~[ ... ]`
/// parameter. Exact text, not merely "rejected" -- mutation: delete the
/// flavour comparison in `check_literal_against_declared_effect` and the
/// ordinary literal is silently accepted, so this golden must fail.
#[test]
fn ordinary_literal_at_an_inline_parameter_is_located_error() {
    let err = check_error(
        ": takes-tilde inline ( ~[ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : main ( -- ) [ 1 add ] takes-tilde drop ;\n",
    );
    assert_eq!(
        err,
        "error: this argument is an ordinary `[ ... ]` quotation but `takes-tilde` declares \
         parameter `~[ i64 -- i64 ]` as inline `~[ ... ]`; write it `~[ ... ]` in `main` (line 2)"
    );
}

/// X7 / M-C, direction 2 (E3b): a `~[ ... ]` literal at an ordinary
/// `Type::Quotation` boundary. The mirror of the golden above, guarding the
/// same mutation from the other direction.
#[test]
fn inline_literal_at_an_ordinary_parameter_is_located_error() {
    let err = check_error(
        ": takes-ordinary ( [ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : main ( -- ) ~[ 1 add ] takes-ordinary drop ;\n",
    );
    assert_eq!(
        err,
        "error: this quotation is inline `~[ ... ]` but `takes-ordinary` expects \
         `[ i64 -- i64 ]`, an ordinary `[ ... ]`; write it `[ ... ]` in `main` (line 2)"
    );
}

/// X7 (R-C2): E3b at the word-output boundary. Same funnel as the parameter
/// golden above, but reached through `materialize_quotation_at_boundary`
/// rather than the argument loop, so only a test says the funnel covers it.
/// The wording claims no parameter, because there is none.
#[test]
fn inline_literal_at_a_word_output_is_located_error() {
    let err = check_error(
        ": mk ( -- [ i64 -- i64 ] ) ~[ 1 add ] ;\n\
         : main ( -- ) 5 mk call drop ;\n",
    );
    assert_eq!(
        err,
        "error: this quotation is inline `~[ ... ]` but `mk` expects `[ i64 -- i64 ]`, \
         an ordinary `[ ... ]`; write it `[ ... ]` in `mk` (line 1)"
    );
}

/// X7 (R-C2): E3b at the array-store boundary, the third of the three. Here
/// the expectation belongs to the store operator, not to any word.
#[test]
fn inline_literal_at_an_array_store_is_located_error() {
    let err = check_error(
        ": seed ( -- [ -- i64 ] ) [ 0 ] ;\n\
         : main ( -- )\n\
         seed 2 fill | tbl |\n\
         &!tbl 0 >usize &!> ~[ 1 ] !\n\
         &tbl 0 &> @ call drop\n\
         tbl drop ;\n",
    );
    assert_eq!(
        err,
        "error: this quotation is inline `~[ ... ]` but `!` expects `[ -- i64 ]`, \
         an ordinary `[ ... ]`; write it `[ ... ]` in `main` (line 4)"
    );
}

/// R-C2: a direct `call` is *not* a flavour boundary. Nothing is materialized
/// there -- `call` splices a literal under either spelling -- so both are
/// accepted and both print `6`.
#[test]
fn both_quotation_flavours_are_accepted_at_a_direct_call() {
    assert_eq!(
        run("call-ordinary", ": main ( -- ) 5 [ 1 add ] call . ;\n"),
        "6\n"
    );
    assert_eq!(
        run("call-inline", ": main ( -- ) 5 ~[ 1 add ] call . ;\n"),
        "6\n"
    );
}

/// X9 (R-C4): `examples/capturing_dispatch.sth` stores ordinary `[ ... ]`
/// closures into a table and returns one from `seed`; both stay unmigrated
/// (no tilde) and the program still runs, since E3b (asserted above) is what
/// actually enforces the ordinary/inline distinction here, not merely leaves
/// it available.
#[test]
fn capturing_dispatch_example_stays_unmigrated_and_runs() {
    let path = format!(
        "{}/examples/capturing_dispatch.sth",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).expect("the example should be readable");
    assert!(
        !src.contains('~'),
        "capturing_dispatch.sth's stored/returned quotations must stay ordinary `[ ... ]`: {src}"
    );
    let binary = sooth::driver::build_with_manifest(
        std::path::Path::new(&path),
        common::manifest_for(std::path::Path::new(&path)).as_deref(),
    )
    .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(output.status.success(), "the program should exit 0");
}
