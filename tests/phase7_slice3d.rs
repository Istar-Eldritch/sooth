//! Phase 7 Slice 3d goldens: the two rowless quotation-consumer splices in a
//! **non-inline polymorphic** body. This file lands C1 (`call` on a
//! body-local literal, splicing its body in place) in phase 1; C2 (a literal
//! passed to a concrete `env` word with a ground `Type::Quotation` input)
//! and the shared negatives land in phase 2.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3b.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3d-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, contents).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

fn build_and_run(src: &Path) -> (PathBuf, String, i32) {
    let binary = driver::build(src).expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        binary,
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn check_err(src: &str) -> String {
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    sooth::check::check(&mut module).expect_err("this program should be rejected")
}

fn check_ok(src: &str) {
    let tokens = sooth::lexer::lex(src).unwrap();
    let mut module = sooth::parser::parse(&tokens).unwrap();
    sooth::check::check(&mut module).expect("this program should be accepted");
}

/// C1 behavioural: a non-inline generic word whose literal body names a
/// bound local, run at two distinct instantiations of `'T` so the splice is
/// carried rigidly rather than coincidentally matching.
#[test]
fn c1_call_on_literal_splices_body_in_place() {
    let src = "import: intrinsics * ;\n\
               type: Bool | False | True ;\n\
               : . ( Bool -- ) ~[ ( False ) drop \"False\\n\" . ] ~[ ( True ) drop \"True\\n\" . ] Bool? ;\n\
               : bump ['T: Copy] ( 'T -- 'T 'T )\n\
               | x | [ x x ] call\n\
               ;\n\
               : main ( -- )\n\
                 5 bump . .\n\
                 True bump . .\n\
               ;\n";
    let prog = Scratch::write("c1-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "5\n5\nTrue\nTrue\n",
        "each instantiation must carry `x x` through the splice independently"
    );
}

/// C1: `call` on a **non-literal** (declared/forwarded) quotation operand in
/// a non-inline poly body. The parameter's own effect carries a free `'T`, so
/// it stays `PolyType::Quotation` rather than folding to `PolyType::Concrete`.
/// P7.S3l (R1) flips this from a rejection: the checker's new arm consumes
/// and produces the declared row structurally, so `call` on this shape now
/// checks clean rather than reporting the old blanket "not permitted on a
/// quotation" message. This test exercises only the body-side guard; the
/// call-site argument boundary (`check_poly_call`'s R9p guard,
/// `check_poly_call_materializes_an_abstract_quotation_argument` in
/// `src/check/poly.rs`) is a separate guard this same slice's R5 also
/// flipped to accept -- not out of scope, just a different call path than
/// the one this test drives.
#[test]
fn c1_call_on_non_literal_operand_is_accepted() {
    check_ok(": caller ( 'T [ 'T -- 'T ] -- 'T ) call ;\n");
}

/// C2 behavioural: a poly body passing a literal to a concrete helper that
/// carries real logic around its own `call` (a second, unrelated argument
/// on the same call), run at two distinct instantiations of the outer `'T`.
#[test]
fn c2_literal_grounds_against_concrete_quotation_param() {
    let src = "import: intrinsics * ;\n\
             type: Bool | False | True ;\n\
             : . ( Bool -- ) ~[ ( False ) drop \"False\\n\" . ] ~[ ( True ) drop \"True\\n\" . ] Bool? ;\n\
             : run1 ( [ i64 -- i64 ] i64 -- i64 )\n\
               swap call\n\
             ;\n\
             : c2_apply_and_pass_through ['T: Copy] ( 'T -- 'T i64 )\n\
               | x | x [ 1 add ] 2 run1\n\
             ;\n\
             : main ( -- )\n\
               5 c2_apply_and_pass_through . .\n\
               True c2_apply_and_pass_through . .\n\
             ;\n";
    let prog = Scratch::write("c2-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "3\n5\n3\nTrue\n",
        "each instantiation must ground and apply the literal independently"
    );
}

/// `branch`/`if`/`times`/`tag` on a quotation in a non-inline poly body stay
/// rejected, unaffected by this phase's operand-window carve-out -- each
/// asserted individually since dropping `if` or `tag` from the retained
/// guard is otherwise unobserved by any other test in the suite.
#[test]
fn c2_branch_tag_on_quotation_still_rejected() {
    // `if`/`times` are not in this list: `main` already carries S3b-follow's
    // real row-typed combinator dispatch for them (`poly_row_combinator`),
    // so a quotation reaching either one now actually dispatches rather than
    // hitting this slice's retained guard -- that is S3b-follow's own
    // coverage (`tests/phase7_slice3b_follow.rs`), not this slice's. Only
    // `branch`/`tag` are compiler primitives with no combinator dispatch to
    // fall into, so only they still reach the located "not yet supported"
    // rejection this slice's own name guard (narrowed by R1 to drop `call`)
    // still renders.
    for name in ["branch", "tag"] {
        let err = check_err(&format!(
            ": bad ['T: Copy] ( 'T -- 'T ) [ ] {name} ;\n : main ( -- ) 5 bad drop ;\n"
        ));
        assert!(
            err.contains(&format!(
                "`{name}` on a quotation in the polymorphic body of `bad`"
            )) && err.contains("not yet supported")
                && err.contains("name no follow-up slice yet"),
            "{name}: {err}"
        );
    }
}

/// A literal passed to a **poly** callee stays rejected (S3f's R9p
/// territory), proving R2's concrete-only gate holds. From a poly caller
/// this never reaches `check_poly_call`'s own `reject_quotation_argument`
/// (that path only runs for a *concrete* caller) -- `poly_call_term` cannot
/// see `poly_env` at all, so a poly callee is simply absent from `env` and
/// the pre-existing operand-window guard rejects it first.
#[test]
fn c2_literal_to_poly_callee_is_rejected() {
    let err = check_err(
        ": pq ['U: Copy] ( 'U -- 'U 'U ) dup ;\n\
         : c2_literal_to_poly_callee_is_rejected ['T: Copy] ( 'T -- 'T i64 )\n\
           | x | x [ 1 add ] pq call\n\
         ;\n\
         : main ( -- ) 5 c2_literal_to_poly_callee_is_rejected drop drop drop ;\n",
    );
    assert_eq!(
        err,
        "error: `pq` is not permitted on a quotation literal in `c2_literal_to_poly_callee_is_rejected` (line 3)"
    );
}

/// R2's completeness-gap note: a literal passed as the sole (top-of-window)
/// operand to an **overloaded** concrete name is a located rejection, not
/// `unknown word` -- the pre-existing operand-window guard catches it before
/// this phase's carve-out is ever consulted (the carve-out never matches an
/// overloaded name).
#[test]
fn c2_overloaded_candidate_with_quotation_literal_is_located_rejection() {
    let err = check_err(
        ": run2 ( [ i64 -- i64 ] -- i64 ) 1 swap call ;\n\
         : run2 ( i64 -- i64 ) 1 add ;\n\
         : c2_overloaded_candidate_with_quotation_literal_is_located_rejection ['T: Copy] ( 'T -- 'T i64 )\n\
           | x | x [ 1 add ] run2\n\
         ;\n\
         : main ( -- ) 5 c2_overloaded_candidate_with_quotation_literal_is_located_rejection drop drop ;\n",
    );
    assert_eq!(
        err,
        "error: `run2` is not permitted on a quotation literal in `c2_overloaded_candidate_with_quotation_literal_is_located_rejection` (line 4)"
    );
    assert!(!err.contains("unknown word"), "{err}");
}

/// A concrete word sharing its *surface name* with an unrelated poly word is
/// still `env`'s sole candidate for that name, but `ast::overload_symbols`
/// suffixes its mangled symbol anyway. C2's carve-out must not ground through
/// such a candidate: it records no `builtin_overloads` entry (a literal is
/// never `PolyType::Concrete`, so `exact` is False), so lowering is left to
/// resolve a bare name it cannot find and panics (L1: never an inherited
/// backend panic) instead of reporting this located rejection.
#[test]
fn c2_literal_to_name_shared_with_a_poly_word_is_located_rejection() {
    let err = check_err(
        ": run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;\n\
         : run1 ['U: Copy] ( 'U -- 'U 'U ) dup ;\n\
         : c2_literal_to_name_shared_with_a_poly_word_is_located_rejection ['T: Copy] ( 'T -- 'T i64 )\n\
           | x | x [ 1 add ] 2 run1\n\
         ;\n\
         : main ( -- ) 5 c2_literal_to_name_shared_with_a_poly_word_is_located_rejection drop drop ;\n",
    );
    assert_eq!(
        err,
        "error: `run1` is not permitted on a quotation literal in `c2_literal_to_name_shared_with_a_poly_word_is_located_rejection` (line 4)"
    );
    assert!(!err.contains("unknown word"), "{err}");
}
