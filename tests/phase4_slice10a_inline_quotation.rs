//! Slice 10a, phase 2 goldens: `~[ ... ]` parses on every entry point,
//! grounds to `Type::InlineQuotation`, `call` still works on it, all five
//! materialization boundaries reject it, and R3's mismatch is located in
//! both directions.

use sooth::{check, lexer, parser};

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    parser::parse(&tokens).expect_err("parsing should fail")
}

fn run_src(name: &str, src: &str) -> (String, i32) {
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

// -- R1: `~[` is a single token, adjacency required -------------------------

#[test]
fn spaced_tilde_bracket_is_a_parse_error() {
    // `~ [` drops the glue (a bare `Word("~")` + `LBracket`), and nothing
    // declares a bare `~` word, so it is a located parse error, not a silent
    // fallback to the ordinary quotation type.
    let err = parse_error(": apply ( i64 ~ [ i64 -- i64 ] -- i64 ) call ;\n");
    assert!(
        err.starts_with("parse error") || err.starts_with("error"),
        "a spaced `~ [` should be a parse error, got: {err}"
    );
    assert!(
        err.contains('~'),
        "the spaced-form error should name the bare `~` word, got: {err}"
    );
}

#[test]
fn glued_tilde_bracket_parses_as_a_combinator_parameter() {
    let src = ": apply ( i64 ~[ i64 -- i64 ] -- i64 ) call ;\n\
               : main ( -- ) 3 [ 1 + ] apply . ;\n";
    let (stdout, code) = run_src("tilde-parses", src);
    assert_eq!(stdout, "4\n");
    assert_eq!(code, 0);
}

// -- R2/R2: `call` on a `~` is accepted --------------------------------------

#[test]
fn call_on_inline_quotation_is_accepted() {
    // The sixth behavioural test: without it, an over-eager materialization
    // check could silently break invocation and nothing would notice.
    let src = ": twice ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;\n\
               : main ( -- ) 3 [ 2 * ] twice . ;\n";
    let (stdout, code) = run_src("tilde-call", src);
    assert_eq!(stdout, "12\n");
    assert_eq!(code, 0);
}

// -- R2: the five materialization boundaries reject a `~` --------------------

#[test]
fn inline_quotation_as_word_output_is_error() {
    let err = check_error(": mk ( -- ~[ i64 -- i64 ] ) [ 1 + ] ;\n");
    assert!(
        err.contains("output") && err.contains("`mk`"),
        "a `~` word output should be a located rejection naming `mk`, got: {err}"
    );
}

#[test]
fn inline_quotation_as_struct_field_is_error() {
    let err = parse_error("type: Box f ~[ i64 -- i64 ] ;\n");
    assert!(
        err.contains("`~`"),
        "a `~` struct field should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_as_array_element_is_error() {
    // `[ ~[ ... ] 3 ]` here is a *type* position (a struct field's array
    // type), not a term-level quotation literal.
    let err = parse_error("type: Box f [ ~[ i64 -- i64 ] 3 ] ;\n");
    assert!(
        err.contains("`~`"),
        "a `~` array element should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_as_ref_referent_is_error() {
    let err = parse_error("extern: f ( &!~[ i64 -- i64 ] -- ) \"f\" ;\n");
    assert!(
        err.contains("`~`"),
        "a `~` reference referent should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_as_extern_parameter_is_error() {
    let err = parse_error("extern: f ( ~[ i64 -- i64 ] -- ) \"f\" ;\n");
    assert!(
        err.contains("`~`"),
        "a `~` extern parameter should be a located rejection, got: {err}"
    );
}

// The fifth materialization boundary -- capture admission -- has no
// source-level golden here: a poly signature (which a `~` parameter forces)
// cannot place an ordinary quotation anywhere but a direct top-level
// parameter (`reject_poly_quotation_anywhere`, predates this slice), so no
// `.sth` program can drive a `~` local into an escaping-closure boundary
// this slice. Pinned directly instead:
// `check::tests::check_capture_admission_rejects_captured_inline_quotation`.

// -- Boundary 3 (declared parameter of a mono combinator): unreachable ------

#[test]
fn tilde_bearing_signature_always_routes_to_the_poly_parser() {
    // R1: a `~` is poly-forced even when its effect is otherwise fully
    // concrete, so a mono combinator can never declare a `~` parameter and
    // `materialize_quotation_at_boundary`'s declared-parameter boundary
    // (`:8570`) never sees one. Asserted directly on the parsed `WordDef`,
    // since "the word has no other error" would not distinguish routing
    // from coincidence.
    let src = ": apply ( i64 ~[ i64 -- i64 ] -- i64 ) call ;\n";
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let module = parser::parse(&tokens).expect("parsing should succeed");
    assert!(
        module.words[0].poly.is_some(),
        "a `~`-bearing signature must set `WordDef.poly`, even fully concrete"
    );
}

// -- R3: no implicit coercion, in both directions ----------------------------

#[test]
fn inline_quotation_type_differs_from_ordinary_at_the_output_boundary() {
    // A `~`-declared output is rejected the same way an ordinary quotation
    // output would be accepted: the boundary treats the two variants as
    // unequal types, not interchangeable spellings of "a quotation".
    let inline_err = check_error(": mk ( -- ~[ i64 -- i64 ] ) [ 1 + ] ;\n");
    assert!(inline_err.contains("`~`") || inline_err.contains('~'));
    let ordinary_ok = ": mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;\n";
    let tokens = lexer::lex(ordinary_ok).expect("lexing should succeed");
    let mut module = parser::parse(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("an ordinary quotation output should still be legal");
}

#[test]
fn forwarding_inline_quotation_into_an_ordinary_declared_parameter_is_error() {
    // Direction 1: a combinator forwards its own `~`-declared parameter (an
    // abstract `InlineQuotation`) into a nested combinator that declares an
    // *ordinary* quotation parameter. `Type` derives structural equality, so
    // `InlineQuotation(eff) != Quotation(eff)` even though `eff` is
    // identical, and the mismatch is located naming both spellings.
    let err = check_error(
        ": takes_ordinary ( [ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : outer ( ~[ i64 -- i64 ] -- i64 ) | g | g takes_ordinary ;\n\
         : main ( -- ) [ 1 + ] outer . ;\n",
    );
    assert!(
        err.contains("`takes_ordinary`")
            && err.contains("~[ i64 -- i64 ]")
            && err.contains("[ i64 -- i64 ]"),
        "forwarding a `~` into an ordinary-declared parameter should name both spellings, got: {err}"
    );
}

#[test]
fn forwarding_ordinary_quotation_into_an_inline_declared_parameter_is_error() {
    // Direction 2, the mirror image: an ordinary-declared parameter forwarded
    // into a nested combinator that declares `~`.
    let err = check_error(
        ": takes_tilde ( ~[ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : outer ( [ i64 -- i64 ] -- i64 ) | g | g takes_tilde ;\n\
         : main ( -- ) [ 1 + ] outer . ;\n",
    );
    assert!(
        err.contains("`takes_tilde`")
            && err.contains("~[ i64 -- i64 ]")
            && err.contains("[ i64 -- i64 ]"),
        "forwarding an ordinary quotation into a `~`-declared parameter should name both spellings, got: {err}"
    );
}

#[test]
fn variable_bearing_inline_quotation_grounds_through_apply_subst() {
    // Every other `~` golden here is fully concrete, so it folds to
    // `Concrete(InlineQuotation)` at *parse* time (`raw_to_poly_type`) and
    // never reaches `apply_subst`'s `PolyType::Quotation` arm. `'T` keeps
    // this one a real `PolyType::Quotation`, so grounding it against a
    // concrete call site is what exercises `apply_subst`'s `is_inline`
    // branch.
    let src = ": apply ( 'T ~[ 'T -- 'T ] -- 'T ) call ;\n\
               : main ( -- ) 3 [ 2 * ] apply . ;\n";
    let (stdout, code) = run_src("tilde-var-ground", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}

#[test]
fn variable_bearing_inline_quotation_still_mismatches_ordinary() {
    // R3 through the variable-bearing (non-folded) path: `outer` forwards its
    // own `'T ~[ 'T -- 'T ]` parameter into `takes_ordinary`'s `'T [ 'T -- 'T ]`.
    // `apply_subst` must ground `outer`'s parameter to `InlineQuotation`, not
    // silently drop the sigil during substitution.
    let err = check_error(
        ": takes_ordinary ( 'T [ 'T -- 'T ] -- 'T ) call ;\n\
         : outer ( 'T ~[ 'T -- 'T ] -- 'T ) | g | g takes_ordinary ;\n\
         : main ( -- ) 3 [ 2 * ] outer . ;\n",
    );
    assert!(
        err.contains("`takes_ordinary`") && err.contains('~'),
        "a variable-bearing `~` forward should still mismatch an ordinary declared parameter, got: {err}"
    );
}

#[test]
fn forwarding_inline_quotation_into_a_matching_inline_declared_parameter_runs() {
    // The positive control for both directions above: when the forwarded
    // type and the declared type agree (both `~`), the forward is accepted
    // and runs -- so the two negative goldens are catching a real type
    // mismatch, not merely rejecting every forward.
    let src = ": takes_tilde ( ~[ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
               : outer ( ~[ i64 -- i64 ] -- i64 ) | g | g takes_tilde ;\n\
               : main ( -- ) [ 1 + ] outer . ;\n";
    let (stdout, code) = run_src("tilde-forward-match", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}
