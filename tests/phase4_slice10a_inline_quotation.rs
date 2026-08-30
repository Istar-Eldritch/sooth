//! Slice 10a goldens. Phase 2: `~[ ... ]` parses on every entry point, grounds
//! to `Type::InlineQuotation`, `call` still works on it, all five
//! materialization boundaries reject it, and R3's mismatch is located in both
//! directions. Phase 4 (bottom section): a row inside a `~` effect grounds to
//! the concrete caller region at each of R9's four check-site contexts, the
//! region is stripped from mismatch diagnostics (R10), and the prepend is
//! type-only so a caller borrow in the row is not falsely flagged.

use sooth::{check, lexer, test_support};

mod common;

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    test_support::parse_with_core(&tokens).expect_err("parsing should fail")
}

static NEXT_TEMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn run_src(name: &str, src: &str) -> (String, i32) {
    let id = NEXT_TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}-{id}.sth", std::process::id()));
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

// -- R1: `~[` is a single token, adjacency required -------------------------

#[test]
fn spaced_tilde_bracket_is_a_parse_error() {
    // `~ [` drops the glue (a bare `Word("~")` + `LBracket`), and nothing
    // declares a bare `~` word, so it is a located parse error, not a silent
    // fallback to the ordinary quotation type.
    let err = parse_error(": apply ( i64 ~ [ i64 -- i64 ] -- i64 ) call ;\n");
    assert!(
        err.contains('~'),
        "the spaced-form error should name the bare `~` word, got: {err}"
    );
}

#[test]
fn glued_tilde_bracket_parses_as_a_combinator_parameter() {
    let src = ": apply inline ( i64 ~[ i64 -- i64 ] -- i64 ) call ;\n\
               : main ( -- ) 3 ~[ 1 add ] apply . ;\n";
    let (stdout, code) = run_src("tilde-parses", src);
    assert_eq!(stdout, "4\n");
    assert_eq!(code, 0);
}

// -- R2/R2: `call` on a `~` is accepted --------------------------------------

#[test]
fn call_on_inline_quotation_is_accepted() {
    // The sixth behavioural test: without it, an over-eager materialization
    // check could silently break invocation and nothing would notice.
    let src = ": twice inline ( i64 ~[ i64 -- i64 ] -- i64 ) | f | f call f call ;\n\
               : main ( -- ) 3 ~[ 2 mul ] twice . ;\n";
    let (stdout, code) = run_src("tilde-call", src);
    assert_eq!(stdout, "12\n");
    assert_eq!(code, 0);
}

// -- R2: the five materialization boundaries reject a `~` --------------------

#[test]
fn inline_quotation_as_word_output_is_error() {
    let err = check_error(": mk ( -- ~[ i64 -- i64 ] ) [ 1 add ] ;\n");
    assert!(
        err.contains("output") && err.contains("`mk`"),
        "a `~` word output should be a located rejection naming `mk`, got: {err}"
    );
}

#[test]
fn inline_quotation_as_struct_field_is_error() {
    let err = parse_error("type: Box f ~[ i64 -- i64 ] ;\n");
    assert!(
        err.contains("cannot appear here"),
        "a `~` struct field should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_as_array_element_is_error() {
    // `[ ~[ ... ] 3 ]` here is a *type* position (a struct field's array
    // type), not a term-level quotation literal.
    let err = parse_error("type: Box f array[ ~[ i64 -- i64 ] 3 ] ;\n");
    assert!(
        err.contains("cannot appear here"),
        "a `~` array element should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_as_ref_referent_is_error() {
    // The referent must be spaced off from the sigil (`&! ~[`, not `&!~[`):
    // the lexer only glues `~[` into a `TildeLBracket` when the scanned word
    // is exactly `~`, and `&!~` greedily scans as one word, missing the
    // guard entirely (it fails on an unrelated "unknown type `~`" path).
    let err = parse_error("extern: f ( &! ~[ i64 -- i64 ] -- ) \"f\" ;\n");
    assert!(
        err.contains("cannot appear here"),
        "a `~` reference referent should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_as_extern_parameter_is_error() {
    let err = parse_error("extern: f ( ~[ i64 -- i64 ] -- ) \"f\" ;\n");
    assert!(
        err.contains("cannot appear here"),
        "a `~` extern parameter should be a located rejection, got: {err}"
    );
}

#[test]
fn inline_quotation_nested_in_a_quotation_parameter_is_error() {
    // A `~` is legal only as a word's own *direct* declared parameter, never
    // buried inside another quotation's effect: the outer ordinary quotation
    // is materializable, so a `~` riding inside it would reach a runtime
    // representation -- the one thing `~` forbids. The parameter folds to
    // `Concrete(Type::Quotation)`, so the poly audit must recurse into its
    // effect to catch the inner `~` (the variable-bearing `Quotation` arm
    // never fires on a fully-concrete parameter).
    let err = check_error(": f ( [ ~[ i64 -- i64 ] -- ] -- ) drop ;\n");
    assert!(
        err.contains("nested inside a quotation effect") && err.contains('~'),
        "a `~` nested inside an ordinary quotation parameter should be rejected, got: {err}"
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
    let module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
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
    let inline_err = check_error(": mk ( -- ~[ i64 -- i64 ] ) [ 1 add ] ;\n");
    assert!(inline_err.contains('~'));
    let ordinary_ok = ": mk ( -- [ i64 -- i64 ] ) [ 1 add ] ;\n";
    let tokens = lexer::lex(ordinary_ok).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
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
        ": takes_ordinary inline ( [ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : outer inline ( ~[ i64 -- i64 ] -- i64 ) | g | g takes_ordinary ;\n\
         : main ( -- ) ~[ 1 add ] outer drop ;\n",
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
        ": takes_tilde inline ( ~[ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
         : outer inline ( [ i64 -- i64 ] -- i64 ) | g | g takes_tilde ;\n\
         : main ( -- ) [ 1 add ] outer drop ;\n",
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
    // A positive control: a variable-bearing (non-folded) `~` parameter still
    // runs correctly end to end. (The sibling test
    // `variable_bearing_inline_quotation_still_mismatches_ordinary` is the one
    // that actually discriminates `apply_subst`'s `is_inline` branch -- it
    // fails if that branch is mutated away, this one does not.)
    let src = ": apply inline ( 'T ~[ 'T -- 'T ] -- 'T ) call ;\n\
               : main ( -- ) 3 ~[ 2 mul ] apply . ;\n";
    let (stdout, code) = run_src("tilde-var-ground", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}

#[test]
fn variable_bearing_inline_quotation_still_mismatches_ordinary() {
    // R3 through the variable-bearing (non-folded) path: `outer` forwards its
    // own `'T ~[ 'T -- 'T ]` parameter into `takes_ordinary`'s `'T [ 'T -- 'T ]`.
    // `apply_subst` must ground `outer`'s parameter to `InlineQuotation`, not
    // silently drop the sigil during substitution. This is the test that
    // actually discriminates `apply_subst`'s `is_inline` branch: mutate that
    // branch away (always ground to `Concrete(Type::Quotation)`) and this
    // test fails, since the forward would then silently match.
    //
    // (Its sibling `variable_bearing_inline_quotation_grounds_through_apply_subst`
    // is a positive control -- it exercises the same branch but does not
    // discriminate it.)
    let err = check_error(
        ": takes_ordinary inline ( 'T [ 'T -- 'T ] -- 'T ) call ;\n\
         : outer inline ( 'T ~[ 'T -- 'T ] -- 'T ) | g | g takes_ordinary ;\n\
         : main ( -- ) 3 ~[ 2 mul ] outer drop ;\n",
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
    let src = ": takes_tilde inline ( ~[ i64 -- i64 ] -- i64 ) | f | 5 f call ;\n\
               : outer inline ( ~[ i64 -- i64 ] -- i64 ) | g | g takes_tilde ;\n\
               : main ( -- ) ~[ 1 add ] outer . ;\n";
    let (stdout, code) = run_src("tilde-forward-match", src);
    assert_eq!(stdout, "6\n");
    assert_eq!(code, 0);
}

// -- Phase 4 (R9/R10): a row inside a `~` effect grounds at the check site ---
//
// The combinator under test is `apply-with ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )`:
// a quotation whose declared input row `..s` is the signature's own top-level
// row. At a call site the row grounds to the caller-stack region below the
// fixed inputs; at the definition site it grounds to the empty region. These
// are `times`'s signature shape minus the back-edge (phase 5), so they exercise
// R9 without depending on the self-tail rewrite.

const APPLY_WITH: &str =
    ": apply-with inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s ) | f | f call ;\n";

#[test]
fn row_bearing_inline_quotation_grounds_and_runs() {
    // R9 context 1 (known-literal splice). The caller row `..s` is `[i64]` (the
    // `10`), so the grounded quotation input is `[ i64 i64 ]` and `[ add ]` folds
    // the row's top with the fixed input: 10 + 5 = 15. If the row were not
    // prepended, `[ add ]` would underflow (`add` needs 2, the sub-stack holds 1).
    let src = format!("{APPLY_WITH}: main ( -- ) 10 5 ~[ add ] apply-with . ;\n");
    let (stdout, code) = run_src("row-ground-run", &src);
    assert_eq!(stdout, "15\n");
    assert_eq!(code, 0);
}

#[test]
fn row_grounding_mismatch_strips_the_caller_region() {
    // R9/R10: a literal whose body does not restore the declared output row is
    // a located mismatch. The grounded row region (`[i64]`, the caller's `10`)
    // is stripped before rendering, so the printed effect shows only the
    // quotation's own fixed slots -- `~[ i64 -- ]` declared (the review fix:
    // rendered with the inline-quotation renderer, since `apply-with`'s
    // parameter is declared `~`), `[ i64 -- i64 i64 ]` actual -- never the
    // caller's stack. `[ dup ]` leaves an extra `i64`.
    let src = format!("{APPLY_WITH}: main ( -- ) 10 5 ~[ dup ] apply-with . . ;\n");
    let err = check_error(&common::silent_prints(&src));
    assert!(
        err.contains("the quotation passed to `apply-with` was declared `~[ i64 -- ]` but its body has effect `[ i64 -- i64 i64 ]`"),
        "the grounding mismatch must render the row-stripped effect, got: {err}"
    );
    // The anti-leak assertion pinned to exact text: an unstripped `actual`
    // would carry the row through as a third output (`[ i64 -- i64 i64 i64 ]`)
    // and an unstripped `declared`/input side would show `i64 i64 --`.
    assert!(
        !err.contains("i64 i64 i64") && !err.contains("i64 i64 --"),
        "the caller's row region must not leak into the printed effect, got: {err}"
    );
}

#[test]
fn abstract_row_bearing_quotation_passes_down() {
    // R9 context 2 (abstract pass-down). `outer` forwards its own row-bearing
    // `~` parameter into `apply-with`. The forward is a type comparison of two
    // row-free `QuotEffect`s (the interned effect drops the row, so both sides
    // agree), and the spliced `apply-with` body grounds the row itself.
    let src = format!(
        "{APPLY_WITH}\
         : outer inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s ) | g | g apply-with ;\n\
         : main ( -- ) 10 5 ~[ add ] outer . ;\n"
    );
    let (stdout, code) = run_src("row-passdown", &src);
    assert_eq!(stdout, "15\n");
    assert_eq!(code, 0);
}

#[test]
fn row_bearing_combinator_checks_standalone_with_empty_region() {
    // R9 context 3 (definition-site, no caller). `apply-with` checks standalone
    // with its row grounded to the empty region: `f call` consumes the fixed
    // `i64` and leaves the (empty) row, matching the declared output. No call
    // site is present, yet `check` still checks the word.
    let tokens = lexer::lex(APPLY_WITH).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module)
        .expect("a row-bearing `~` combinator must check standalone against the empty region");
}

#[test]
fn row_bearing_inline_quotation_routes_to_the_poly_path() {
    // R9 context 4 (mono declared parameter is unreachable for a `~`). The
    // unreachability rests on routing, not on the absence of an error:
    // `inline_combinator` branches on `word.poly.is_some()`, so a `~`-bearing
    // signature -- even a row-bearing one -- must set `WordDef.poly`, sending
    // it to `check_poly_combinator_args` and never to the monomorphic
    // declared-parameter path.
    let tokens = lexer::lex(APPLY_WITH).expect("lexing should succeed");
    let module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    assert!(
        module.words[0].poly.is_some(),
        "a row-bearing `~` signature must set `WordDef.poly`, routing to the poly path"
    );
}

#[test]
fn grounded_row_region_is_type_only_so_a_caller_borrow_is_not_flagged() {
    // R9: the prepended row region is type-only (`Slot::computed`, `deriv:
    // None`). A live borrow `&v` riding untouched in the caller row must not be
    // reported by the exit-row borrow guard as `quotation borrows place` -- a
    // False positive that prepending the caller's *real* slots would produce.
    // `[ drop ]` consumes the fixed `i64` and leaves the row (the borrow)
    // untouched.
    let src = format!(
        "type: V x i64 ;\n\
         {APPLY_WITH}\
         : main ( -- )\n\
           0 V | v |\n\
           &v 5 ~[ drop ] apply-with\n\
           drop\n\
           v drop ;\n"
    );
    let (stdout, code) = run_src("row-typeonly-borrow", &src);
    assert_eq!(stdout, "");
    assert_eq!(code, 0);
}
