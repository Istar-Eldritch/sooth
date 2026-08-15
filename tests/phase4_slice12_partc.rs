//! Phase 4 Slice 12, phase 2 exit criteria (part C).
//!
//! `~[ ... ]` is writable as a body literal (R-C1, X6, unit-tested directly in
//! `src/parser.rs`'s new `Token::TildeLBracket` arm) and the tilde is required
//! exactly at a `Type::InlineQuotation` parameter, in both directions (R-C2,
//! X7 = M-C). The migrated corpus stays byte-identical (X8, covered by
//! `tests/phase4_slice10c_corpus_stdout.rs`'s existing fixtures, unchanged by
//! this phase). `examples/capturing_dispatch.sth`'s stored/returned ordinary
//! quotations stay unmigrated and still run (X9, R-C4).

fn check_error(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).expect("lexing should succeed");
    let mut module = sooth::parser::parse(&tokens).expect("parsing should succeed");
    sooth::check::check(&mut module).expect_err("check should fail")
}

/// X7 / M-C, direction 1 (E3a): an ordinary `[ ... ]` literal at a `~[ ... ]`
/// parameter. Exact text, not merely "rejected" -- mutation: delete the
/// flavour comparison in `check_literal_against_declared_effect` and the
/// ordinary literal is silently accepted, so this golden must fail.
#[test]
fn ordinary_literal_at_an_inline_parameter_is_located_error() {
    let err = check_error(
        ": takes-tilde inline ( ~[ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : main ( -- ) 5 [ 1 + ] takes-tilde . ;\n",
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
         : main ( -- ) 5 ~[ 1 + ] takes-ordinary . ;\n",
    );
    assert_eq!(
        err,
        "error: this argument is an inline `~[ ... ]` quotation but `takes-ordinary` declares \
         parameter `[ i64 -- i64 ]` as an ordinary `[ ... ]`; write it `[ ... ]` in `main` (line 2)"
    );
}

/// E3b at the direct-`call` boundary (R-C2's fourth listed boundary): a
/// `~[ ... ]` literal spelled directly before `call` (not forwarded through a
/// combinator's own `~`-declared parameter local, which must keep working --
/// see `check::terms::tests` for that positive case) is rejected, naming
/// `call` since there is no declared parameter to name.
#[test]
fn inline_literal_at_a_direct_call_is_located_error() {
    let err = check_error(": main ( -- ) ~[ 1 + ] call ;\n");
    assert_eq!(
        err,
        "error: this argument is an inline `~[ ... ]` quotation but `call` splices an ordinary \
         `[ ... ]`; write it `[ ... ]` in `main` (line 1)"
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
    let binary = sooth::driver::build(std::path::Path::new(&path)).expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(output.status.success(), "the program should exit 0");
}
