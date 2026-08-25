//! P7.S3t goldens, all three phases. Phase 1 (below) delivers the call-site
//! explicit type instantiation `f[Point]` as a *surface syntax* -- the parse
//! and the rejections, with `check_poly_call` still ignoring the list. Phase
//! 2 seeds `Subst` from it, so a call can ground a variable no operand
//! reaches. Phase 3 relaxes the trait-member declaration gate to admit a
//! nullary member and dogfoods it end to end: a zero-receiver member
//! dispatched through a bounded generic word instantiated at its own
//! concrete call site.
//!
//! The rejections are the load-bearing half. Exactly one of `check_term`'s
//! dozen dispatch routes for a `Call` can consume a type-argument list -- the
//! polymorphic-call interception -- and a route that silently dropped one would
//! link whichever specialization it resolved on its own, which is a miscompile
//! rather than a diagnostic. So each of the routes below is pinned rejecting,
//! and the grammar this syntax narrows (a word followed by a *spaced* bracket)
//! is pinned still parsing as the quotation literal it always was.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

/// A scratch single-file program, removed with its directory on drop.
struct Scratch(PathBuf);

impl Scratch {
    /// Written verbatim: every assertion below pins a `line N` or `col C`, and
    /// several fixtures are *about* what a bracket adjacency means, so nothing
    /// is appended or rewritten by the harness.
    fn write(tag: &str, src: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3t-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.sth");
        std::fs::write(&path, src).unwrap();
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

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(tag: &str, src: &str) -> String {
    let prog = Scratch::write(tag, src);
    let build = sooth_build(prog.path());
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(prog.path().with_extension(""))
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn build_error(tag: &str, src: &str) -> String {
    let prog = Scratch::write(tag, src);
    let build = sooth_build(prog.path());
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// The message every non-polymorphic route reports for a list it cannot
/// consume. One string, so a test that names a route also pins that the route
/// reached *this* rejection rather than some unrelated failure.
fn takes_no_type_arguments(name: &str, line: u32) -> String {
    format!(
        "`{name}` (line {line}) takes no type arguments; \
         only a call to a polymorphic word may be explicitly instantiated"
    )
}

/// R2/R6: a glued list on a polymorphic call parses, survives resolution, and
/// the program runs. Phase 1 ignores the list downstream, so this witnesses the
/// syntax reaching `check_poly_call` at all -- not yet that it grounds
/// anything.
#[test]
fn an_explicit_instantiation_on_a_polymorphic_call_builds_and_runs() {
    let out = build_and_run(
        "poly-call",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : main ( -- ) 7 id[i64] . ;\n",
    );
    assert_eq!(out, "7\n");
}

/// R2's non-break witness, and the mutation subject for the adjacency
/// conjunct: with the column test deleted, both spaced brackets here become
/// explicit instantiations and this program stops building.
#[test]
fn a_spaced_bracket_after_a_word_is_still_a_quotation() {
    let out = build_and_run(
        "spaced-bracket",
        "import: intrinsics * ;\n\
         : main ( -- )\n\
           1 dup [ 2 add ] call add .\n\
           9 dup [ i64 ; 3 ] drop drop drop ;\n",
    );
    assert_eq!(out, "4\n");
}

/// R3, the builtin route: `dup` is dispatched by name ahead of the word
/// environment, so it never reaches the polymorphic interception and has
/// nowhere to put a list.
#[test]
fn a_glued_bracket_after_a_builtin_is_rejected() {
    let err = build_error(
        "glued-builtin",
        "import: intrinsics * ;\n\
         : main ( -- ) 1 dup[i64] . . ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("dup", 2)), "{err}");
}

/// R3, the local-read route: a body local wins over every word, so a list on
/// one names no callee at all.
#[test]
fn a_glued_bracket_after_a_local_is_rejected() {
    let err = build_error(
        "glued-local",
        "import: intrinsics * ;\n\
         : main ( -- ) 1 | x | x[i64] . ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("x", 2)), "{err}");
}

/// R3, the concrete `env` route: a monomorphic word has no type variables to
/// instantiate, so the list is a statement about a signature that does not
/// exist.
#[test]
fn a_glued_bracket_after_a_concrete_word_is_rejected() {
    let err = build_error(
        "glued-concrete",
        "import: intrinsics * ;\n\
         : inc ( i64 -- i64 ) 1 add ;\n\
         : main ( -- ) 1 inc[i64] . ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("inc", 3)), "{err}");
}

/// R3, the combinator route: a `lib/` combinator like `lt` is spliced ahead of
/// the polymorphic-call interception (`poly.combinators`), so it has nowhere
/// to put a list either. Review fix: this clause of the guard had no witness --
/// deleting it let `lt[i64]` build to exit 0.
#[test]
fn a_glued_bracket_after_a_combinator_is_rejected() {
    let err = build_error(
        "glued-combinator",
        "import: intrinsics * ;\n\
         import: core::prelude | lt | ;\n\
         : main ( -- ) lt[i64] ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("lt", 3)), "{err}");
}

/// R3, the eliminator route: an enum eliminator like `Shape?` is intercepted
/// ahead of the polymorphic-call interception (`poly.eliminators`), so it has
/// nowhere to put a list either. Review fix: this clause of the guard had no
/// witness -- deleting it let `Shape?[f64]` build to exit 0.
#[test]
fn a_glued_bracket_after_an_eliminator_is_rejected() {
    let err = build_error(
        "glued-eliminator",
        "import: intrinsics * ;\n\
         type: Shape | Circle r f64 | Rect w f64 h f64 ;\n\
         : main ( -- ) Shape?[f64] ;\n",
    );
    assert!(err.contains(&takes_no_type_arguments("Shape?", 3)), "{err}");
}

/// R7: a type *variable* argument. `parse_type_expr` has no production for one,
/// so without its own message this reads as a missing type declaration rather
/// than as the unsupported forwarding it is.
#[test]
fn a_type_variable_argument_is_rejected() {
    let err = build_error(
        "ty-var-argument",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : g ( 'U -- 'U ) id['U] ;\n\
         : main ( -- ) 1 g . ;\n",
    );
    assert!(
        err.contains(
            "`'U` (line 3, col 21) is a type variable; \
             an explicit instantiation takes concrete types"
        ),
        "{err}"
    );
    assert!(
        err.contains(
            "note: forwarding a caller's type variable through an explicit \
             instantiation is not supported"
        ),
        "{err}"
    );
    assert!(!err.contains("unknown type"), "{err}");
}

/// R1/R3: a call inside a polymorphic word's own body is checked symbolically,
/// against no `Subst`, so there is nothing for a list to seed. Rejected rather
/// than dropped -- and with its own message, because the remedy is one frame up,
/// at the enclosing word's own call site.
#[test]
fn an_instantiation_inside_a_polymorphic_body_is_rejected() {
    let err = build_error(
        "inside-poly-body",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : g ( 'U -- 'U ) drop 7 id[i64] ;\n\
         : main ( -- ) 1 g . ;\n",
    );
    assert!(
        err.contains(
            "`id` in `g` (line 3) cannot be explicitly instantiated inside a \
             polymorphic word's own body"
        ),
        "{err}"
    );
    assert!(
        err.contains("note: instantiate the enclosing word at its own call site instead"),
        "{err}"
    );
}

/// R10: the REPL rejects the syntax outright. A session routes through
/// `lower_instantiation` and skips the module-level checks this slice's
/// correctness argument rests on, so the guard fails closed instead of printing
/// success and binding whichever specialization the session found. Both line
/// shapes are covered, since a definition's body is walked separately from a
/// bare expression's terms.
#[test]
fn explicit_instantiation_is_rejected_at_the_repl() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("repl should spawn");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    stdin
        .write_all(b"1 id[i64] .\n: w ( -- ) 1 id[i64] drop ;\n")
        .expect("writing stdin should succeed");
    drop(stdin);
    let out = child.wait_with_output().expect("repl should exit cleanly");
    let session = String::from_utf8(out.stdout).expect("stdout should be utf8");
    let hits: Vec<&str> = session
        .lines()
        .filter(|l| l.contains("explicit type instantiation is not available at the REPL"))
        .collect();
    assert_eq!(
        hits.len(),
        2,
        "both line shapes reject; session was:\n{session}"
    );
    assert!(hits[0].contains("(line 1, col 3)"), "{session}");
    assert!(hits[1].contains("(line 1, col 14)"), "{session}");
    assert!(
        session.contains(
            "note: `f[Point]` needs a whole-program impl registry a live session \
             does not assemble"
        ),
        "{session}"
    );
    assert!(!session.contains("defined w"), "{session}");
}

// ---------------------------------------------------------------------------
// Phase 2: the list seeds `Subst`.
//
// From here the syntax means something. `check_poly_call` binds the callee's
// declared type variables from the list before it unifies a single operand,
// which is what makes a disagreement a diagnostic instead of an overwrite, and
// what lets a call ground a variable no operand reaches.
// ---------------------------------------------------------------------------

/// The generic word whose `'T` no operand can bind: a declared quotation
/// parameter mentioning `'T` is grounded through θ *before* the caller's
/// literal is materialized against it, so `'T` has to be known by then and the
/// literal cannot supply it. That makes this the ordinary-word shape phase 2
/// is observable on, with no trait member and no recursion in it.
const UNBOUND_QUOT_VAR: &str = "import: intrinsics * ;\n\
     : q ( [ 'T -- ] 'U -- 'U ) swap drop ;\n";

/// R6/R9: the headline for this phase. The same call is a hard error without
/// the list (below) and runs with it, so the list is doing the grounding
/// rather than decorating a call inference had already resolved.
#[test]
fn an_explicit_instantiation_grounds_a_variable_no_operand_binds() {
    let out = build_and_run(
        "grounds-unbound",
        &format!("{UNBOUND_QUOT_VAR}: main ( -- ) [ drop ] 8 q[i64 i64] . ;\n"),
    );
    assert_eq!(out, "8\n");
}

/// R9: `poly_unbound_output_error` had no reference anywhere in the tree, and
/// this slice both pins it and gives it a remedy. The note is half the point:
/// without it the message states a fact about the callee's signature and says
/// nothing the caller can act on.
///
/// The spec's reason for those zero references -- that no legal program
/// reached the message -- is false: `UNBOUND_QUOT_VAR`'s `q` called bare
/// reaches it too, and has since P7.S3l, because pass 2 grounds a declared
/// quotation input through the same walk. There the message calls `'T` an
/// output when `q` declares no output variable at all, so this test uses `f`,
/// whose `'T` is a genuine one (its only operand is its own self-call).
#[test]
fn an_uninstantiated_call_names_the_unbound_output_variable() {
    let err = build_error(
        "unbound-output",
        "import: intrinsics * ;\n: f ( -- 'T ) f ;\n: main ( -- ) f drop ;\n",
    );
    assert!(
        err.contains("`f` in `main` (line 3) has output variable `'T` that no input binds"),
        "{err}"
    );
    assert!(
        err.contains("note: supply it explicitly: `f[SomeType]`"),
        "{err}"
    );
}

/// R5: the seed is pushed before pass 1, so the operand meets a variable that
/// is already bound and takes the conflict branch. The message names which end
/// was written, because the two are not symmetric: the remedy is either a
/// different argument or a different list.
#[test]
fn an_explicit_instantiation_disagreeing_with_an_operand_is_rejected() {
    let err = build_error(
        "seed-conflict",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : main ( -- ) 7 id[f64] drop ;\n",
    );
    assert!(
        err.contains(
            "`id` in `main` (line 3) was instantiated at `'T` = `f64` but its operand is `i64`"
        ),
        "{err}"
    );
}

/// R5's blast-radius guard. `poly_var_conflict_error` is reached by the same
/// branch this slice redirects, so the two-operand case has to be pinned
/// byte-for-byte or a redirect that fired unconditionally would look green.
#[test]
fn two_operands_disagreeing_still_report_the_old_conflict() {
    let err = build_error(
        "operand-conflict",
        "import: intrinsics * ;\n\
         : pairwise ( 'T 'T -- ) drop drop ;\n\
         : main ( -- ) 1 2.5 pairwise ;\n",
    );
    assert!(
        err.contains("`pairwise` in `main` (line 3) resolved `'T` to both `i64` and `f64`"),
        "{err}"
    );
    assert!(!err.contains("was instantiated at"), "{err}");
}

/// R6: legal on *any* polymorphic call, not gated to the variables operands
/// leave ungrounded. Both variables here are inferable and both are given
/// anyway; a gate would have made a call site's legality depend on the
/// callee's input list, which is R4's objection over again.
#[test]
fn an_explicit_instantiation_on_an_already_inferable_call_is_accepted() {
    let out = build_and_run(
        "redundant-instantiation",
        "import: intrinsics * ;\n\
         : two ( 'T 'U -- 'U 'T ) swap ;\n\
         : main ( -- ) 1 2.5 two[i64 f64] . . ;\n",
    );
    assert_eq!(out, "1\n2.5\n");
}

/// R4: exact arity over the callee's declared type variables, in both
/// directions. Not a prefix rule: under one, position `i`'s meaning would
/// depend on which variables the callee's inputs happen to ground, so adding
/// an input to `id` would silently re-point every `id[...]` already written.
#[test]
fn a_wrong_arity_instantiation_is_rejected() {
    let too_many = build_error(
        "arity-too-many",
        "import: intrinsics * ;\n\
         : id ( 'T -- 'T ) ;\n\
         : main ( -- ) 7 id[i64 f64] . ;\n",
    );
    assert!(
        too_many.contains(
            "`id` (line 3) declares 1 type variable (`'T`) but was given 2 type arguments"
        ),
        "{too_many}"
    );
    let too_few = build_error(
        "arity-too-few",
        "import: intrinsics * ;\n\
         : two ( 'T 'U -- 'U 'T ) swap ;\n\
         : main ( -- ) 1 2.5 two[i64] . . ;\n",
    );
    assert!(
        too_few.contains(
            "`two` (line 3) declares 2 type variables (`'T`, `'U`) but was given 1 type argument"
        ),
        "{too_few}"
    );
}

// ---------------------------------------------------------------------------
// Phase 3: `member_binds_trait_var` admits the empty input list, so a
// zero-receiver trait member (`fresh ( -- 'T )`) can be declared and, via a
// bounded generic word instantiated at its own call site, dispatched.
//
// A bound written in *output* position (`( -- 'T: Default )`) declares no
// input slot -- the only spelling that leaves nothing to ground `'T`, so the
// wrapping word itself needs its own explicit instantiation at its call site.
// ---------------------------------------------------------------------------

/// R1/R8, the headline. Two impls of `Default` on distinguishable types, a
/// zero-receiver `fresh`, and a bounded `f` whose own `'T` no operand can
/// bind; `f[Point]` and `f[Other]` at a concrete call site in `main` reach
/// different `fresh` bodies. A golden that only proves the build succeeds is
/// a placebo here -- a wrong-symbol link is exactly the failure mode this
/// slice guards against.
#[test]
fn a_zero_receiver_member_dispatches_through_an_explicit_instantiation() {
    let out = build_and_run(
        "zero-receiver-headline",
        "import: intrinsics * ;\n\
         trait: Default 'T fresh ( -- 'T ) ;\n\
         type: Point x i64 ;\n\
         type: Other x i64 ;\n\
         impl: Default for Point\n\
           : fresh 11 Point ;\n\
         ;\n\
         impl: Default for Other\n\
           : fresh 22 Other ;\n\
         ;\n\
         : f ( -- 'T: Default ) fresh ;\n\
         : main ( -- )\n\
           f[Point] |p| &p &x @ . p drop\n\
           f[Other] |o| &o &x @ . o drop\n\
           ;\n",
    );
    assert_eq!(out, "11\n22\n");
}

/// R8 in isolation. The headline above needs phases 1-2's seed to reach
/// `fresh` at all, so a seed regression reds it and cannot be told apart
/// from a gate regression. Here the wrapping word's `'T` is grounded by an
/// ordinary operand and no type-argument list appears anywhere: the *only*
/// thing standing between this program and a diagnostic is the relaxed
/// declaration gate. Distinguishable impls again, since a build-only
/// assertion would not catch a wrong-symbol link.
#[test]
fn a_nullary_member_dispatches_off_an_operand_grounded_variable() {
    let out = build_and_run(
        "nullary-operand-grounded",
        "import: intrinsics * ;\n\
         trait: Default 'T fresh ( -- i64 ) ;\n\
         type: Point x i64 ;\n\
         type: Other x i64 ;\n\
         impl: Default for Point\n\
           : fresh 11 ;\n\
         ;\n\
         impl: Default for Other\n\
           : fresh 22 ;\n\
         ;\n\
         : g ( 'T: Default -- ) drop fresh . ;\n\
         : main ( -- )\n\
           1 Point g\n\
           2 Other g\n\
           ;\n",
    );
    assert_eq!(out, "11\n22\n");
}

/// R8: the surviving rejection. A receiver mentioned only *nested* inside a
/// composite input (an array element) is still rejected -- grounding it
/// would need structural unification through the array type, which
/// dispatch does not attempt. Byte-identical to the pre-P7.S3t message for
/// this shape, minus the note that used to name the (now-legal) nullary
/// case as a separate deferral.
#[test]
fn a_nested_receiver_member_is_still_rejected() {
    let err = build_error(
        "nested-receiver",
        "import: intrinsics * ;\n\
         trait: Show 'T sum ( [ 'T 4 ] -- i64 ) ;\n",
    );
    assert!(
        err.contains(
            "`sum` of `Show` (line 2, col 8) never takes `'T` (or `&'T`) directly as an input, \
             so a call has nothing to dispatch on"
        ),
        "{err}"
    );
    assert!(
        err.contains("note: a variable nested inside a composite input (an array element, say) does not count"),
        "{err}"
    );
}

/// R8, S3p ruling 4's ambiguity path, single-variable case: two traits
/// bound to the same variable both declare a nullary member of the same
/// name. Same-variable candidates are the unconditional ambiguity --
/// `candidate_fitting_the_operands` never reaches operand shape for them --
/// so this fires regardless of the (empty, here) operand list.
#[test]
fn a_nullary_member_of_two_traits_on_one_variable_is_ambiguous() {
    let err = build_error(
        "ambiguous-one-var",
        "import: intrinsics * ;\n\
         trait: A 'T fresh ( -- 'T ) ;\n\
         trait: B 'T fresh ( -- 'T ) ;\n\
         type: Point x i64 ;\n\
         impl: A for Point\n\
           : fresh 1 Point ;\n\
         ;\n\
         impl: B for Point\n\
           : fresh 2 Point ;\n\
         ;\n\
         : f ( -- 'T: A B ) fresh ;\n\
         : main ( -- ) f[Point] drop ;\n",
    );
    assert!(
        err.contains("`fresh` is required by both `A` and `B` on 'T"),
        "{err}"
    );
}

/// R8, S3p ruling 4's ambiguity path, cross-variable case: a nullary member
/// declares no operand at all, so `candidate_fitting_the_operands`'s
/// operand-shape filter fits *every* candidate spanning different
/// variables -- more than one fit is still the ambiguity error, not a
/// silent first-match.
#[test]
fn a_nullary_member_across_two_variables_is_ambiguous() {
    let err = build_error(
        "ambiguous-two-vars",
        "import: intrinsics * ;\n\
         trait: A 'T fresh ( -- 'T ) ;\n\
         trait: B 'T fresh ( -- 'T ) ;\n\
         type: Point x i64 ;\n\
         type: Other x i64 ;\n\
         impl: A for Point\n\
           : fresh 1 Point ;\n\
         ;\n\
         impl: B for Other\n\
           : fresh 2 Other ;\n\
         ;\n\
         : f ( -- 'T: A 'U: B ) fresh drop fresh ;\n\
         : main ( -- ) f[Point Other] drop ;\n",
    );
    assert!(
        err.contains("`fresh` is required by `A` on 'T and `B` on 'U"),
        "{err}"
    );
    assert!(
        err.contains(
            "note: a member required by two of a variable's bounds cannot be called unqualified"
        ),
        "{err}"
    );
}
