//! Phase 7 Slice 3b goldens: a quotation literal written in a **non-inline
//! polymorphic** body and consumed there by an enum eliminator, so a
//! polymorphic word can eliminate an enum at all.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3a.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3b-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, common::fixture_source("prog.sth", contents)).unwrap();
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
    let binary = driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect("program should build");
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
    let mut module = sooth::test_support::parse_with_core(&tokens).unwrap();
    sooth::check::check(&mut module).expect_err("this program should be rejected")
}

/// The enum every eliminating test below writes its arms against.
const SHAPE: &str = "type: Shape | Circle r i64 | Rect w i64 h i64 ;\n";

/// R1/R2/R3, the exit criterion: a polymorphic word eliminates a concrete
/// enum. Both arms are quotation literals in the poly body, both consume the
/// narrowed variant they are tagged for, and `'T` rides the shared caller row
/// *below* the scrutinee untouched across both of them.
///
/// The arms are written in the **reverse** of the enum's declaration order
/// (`Shape | Circle | Rect`, arms `( Rect )` then `( Circle )`) on purpose:
/// arms are matched to variants by their annotation tag, never by slot
/// position, and only a reversed order can tell the two rules apart.
#[test]
fn poly_word_eliminates_a_concrete_enum_runs() {
    let src = format!(
        "{SHAPE}\
         : area_and_keep ( 'T Shape -- 'T )\n\
           ~[ ( Rect )   Rect> mul drop ]\n\
           ~[ ( Circle ) Circle> dup mul 3 mul drop ]\n\
           Shape? ;\n\
         : main ( -- )\n\
           1 5 Circle area_and_keep .\n\
           2 3 4 Rect area_and_keep . ;\n"
    );
    let prog = Scratch::write("golden", &src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "1\n2\n",
        "each call keeps its own `'T` across both arms: the `Circle` call \
         computes and discards 75, the `Rect` call 12, and each prints the \
         `i64` it carried in"
    );
}

/// R3/L1: one body, two instantiations. The arms' shared exit row is compared
/// structurally under *rigid* type variables, so `'T` is never bound mid-body
/// by either arm -- which is what lets the same checked body ground to `i64`
/// at one call site and `str` at the other.
#[test]
fn poly_eliminator_carries_a_type_variable_across_arms_at_two_instantiations() {
    let src = format!(
        "{SHAPE}\
         : pick ( 'T Shape -- 'T )\n\
           ~[ ( Rect )   Rect> mul drop ]\n\
           ~[ ( Circle ) Circle> dup mul 3 mul drop ]\n\
           Shape? ;\n\
         : main ( -- )\n\
           1 5 Circle pick .\n\
           \"hi\" 3 4 Rect pick . ;\n"
    );
    let prog = Scratch::write("twoinst", &src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\nhi", "`'T` grounds to `i64` then to `str`");
}

/// A quotation slot carries its literal's identity, so shuffling one is pure
/// slot motion (pinned at the unit level in `src/check/poly.rs`). What a
/// *source* program may write is narrower: a variant-tagged literal must reach
/// its eliminator by written adjacency, the same rule the concrete path
/// applies, so a `swap` between two arms is rejected on both paths with the
/// same message rather than being quietly legal in a generic body only.
#[test]
fn poly_body_tagged_arm_not_adjacent_to_its_eliminator_is_error() {
    let poly = format!(
        "{SHAPE}\
         : pick ( 'T Shape -- 'T )\n\
           ~[ ( Circle ) Circle> dup mul 3 mul drop ]\n\
           ~[ ( Rect )   Rect> mul drop ]\n\
           swap Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let concrete = format!(
        "{SHAPE}\
         : pick ( Shape -- i64 )\n\
           ~[ ( Circle ) Circle> dup mul 3 mul ]\n\
           ~[ ( Rect )   Rect> mul ]\n\
           swap Shape? ;\n\
         : main ( -- ) ;\n"
    );
    for src in [&poly, &concrete] {
        let err = check_err(src);
        assert!(
            err.contains(
                "an eliminator-arm tag, but it is not consumed by a call to a generated eliminator"
            ),
            "{err}"
        );
    }
}

/// A variant-tagged literal that no eliminator collects would never be checked
/// against anything. Admitting quotation literals in a generic body is exactly
/// what could have let one through, so the poly path is held to the concrete
/// path's rule here too.
#[test]
fn poly_body_orphan_tagged_quotation_is_error() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T: Copy -- 'T ) ~[ ( Rect ) drop ] drop ;\n\
         : main ( -- ) 1 bad . ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("annotated `( Rect )`, an eliminator-arm tag")
            && err.contains("not consumed by a call to a generated eliminator"),
        "{err}"
    );
}

/// L1: one arm leaving `'T` and another leaving `i64` is a located rejection,
/// not a bind of `'T := i64`. Asserted on the *structural pairing* (which side
/// is `'T` and which `i64`), since two rendered types can read alike and a
/// bare "it failed" would not tell a disagreement from a swap of the operands.
#[test]
fn poly_eliminator_arm_output_type_disagreement_is_error() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T: Copy Shape -- 'T )\n\
           ~[ ( Rect )   Rect> drop drop dup ]\n\
           ~[ ( Circle ) Circle> ]\n\
           Shape? drop ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("the arms of `Shape?` in `bad`")
            && err.contains("an earlier one leaves `'T`, this one leaves `i64`"),
        "{err}"
    );
}

/// R3: two arms leaving different stack *depths*, reported with the same
/// cross-arm shape diagnostic the concrete path raises -- rendered over the
/// abstract rows, so `'T` shows as itself rather than as some ground stand-in.
#[test]
fn poly_eliminator_arm_depth_mismatch_reuses_the_branch_shape_diagnostic() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T: Copy Shape -- 'T )\n\
           ~[ ( Rect )   Rect> drop drop ]\n\
           ~[ ( Circle ) Circle> ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("the quotations passed to `Shape?` leave different stack shapes")
            && err.contains("an earlier one leaves `'T`, this one leaves `'T i64`"),
        "{err}"
    );
}

/// R2: eliminating *through a reference* is legal in a concrete body but not
/// here -- every arm it could hand a narrowed `&Shape.Rect` to would need the
/// field projections a generic body does not have. Rejected rather than
/// silently narrowed to an owning scrutinee, which would let an arm consume a
/// borrowed enum. Both routes to a reference scrutinee are checked: a declared
/// `&Shape` input, and a borrow taken in the body.
#[test]
fn poly_eliminator_reference_scrutinee_is_located_error() {
    let declared = format!(
        "{SHAPE}\
         : bad ( 'T &Shape -- 'T )\n\
           ~[ ( Circle ) drop ]\n\
           ~[ ( Rect ) drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let borrowed = format!(
        "{SHAPE}\
         : bad ( 'T: Copy Shape -- 'T )\n\
           | s | &s\n\
           ~[ ( Circle ) drop ]\n\
           ~[ ( Rect ) drop ]\n\
           Shape? drop ;\n\
         : main ( -- ) ;\n"
    );
    for src in [&declared, &borrowed] {
        let err = check_err(src);
        assert!(
            err.contains("eliminates a reference, which is not yet supported in a generic body"),
            "{err}"
        );
    }
}

/// L4, the False-accept guard. `PolyScope`'s borrow table is keyed by place
/// and a *missing* record reads as "no conflict", so the arm merge must
/// **union** both arms' records: a merge that picked one arm would silently
/// admit a later use of the place the other arm borrowed.
///
/// Both directions are asserted, because the union can be half-implemented:
/// "pick arm A" keeps `x` and drops `y`, so an `x`-only assertion would not
/// flip.
#[test]
fn poly_eliminator_arm_borrow_disagreement_is_the_false_accept_guard() {
    // A live reference remains on the stack at the later use, so the poly
    // walk's coarse borrow liveness has not pruned either record away: what
    // is under test is the union, not the pruning.
    let program = |later: &str| {
        format!(
            "type: P a i64 ;\n\
             {SHAPE}\
             : bad ( 'T: Copy P P Shape -- 'T )\n\
               | x y s | s\n\
               ~[ ( Rect )   Rect> drop drop &!x ]\n\
               ~[ ( Circle ) Circle> drop &!y ]\n\
               Shape?\n\
               {later} drop drop ;\n\
             : main ( -- ) ;\n"
        )
    };
    let err_x = check_err(&program("x"));
    assert!(
        err_x.contains("cannot name `x`") && err_x.contains("a mutable borrow of it is still live"),
        "arm A's `&!x` must survive the merge: {err_x}"
    );
    let err_y = check_err(&program("y"));
    assert!(
        err_y.contains("cannot name `y`") && err_y.contains("a mutable borrow of it is still live"),
        "arm B's `&!y` must survive the merge: {err_y}"
    );
}

/// L4: one place borrowed at two different mutabilities across two arms. The
/// merged table cannot hold both, and erasing either would read as "no
/// conflict" later, so the disagreement is named rather than dropped.
#[test]
fn poly_eliminator_cross_arm_borrow_mutability_disagreement_is_error() {
    let src = format!(
        "type: P a i64 ;\n\
         {SHAPE}\
         : bad ( 'T: Copy P Shape -- 'T )\n\
           | x s | s\n\
           ~[ ( Rect )   Rect> drop drop &!x @ drop ]\n\
           ~[ ( Circle ) Circle> drop &x @ drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("borrow `x` differently") && err.contains("`&!x`") && err.contains("`&x`"),
        "{err}"
    );
}

/// R3: a linear local bound *inside* an arm and never consumed there. The
/// concrete path gets this from block exit; the poly walk has no block scope,
/// so the arm walk must check it before truncating the arm's locals away --
/// and must do so *before* the move-state join, or the leak is erased.
#[test]
fn poly_eliminator_arm_binds_and_leaks_a_linear_local_is_error() {
    let src = format!(
        "type: Spy tag i64 ;\n\
         : drop ( Spy -- ) | s | s Spy> drop ;\n\
         {SHAPE}\
         : bad ( 'T Spy Shape -- 'T )\n\
           ~[ ( Rect )   Rect> drop drop | z | ]\n\
           ~[ ( Circle ) Circle> drop | z | z drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("the local `z` of type `Spy`")
            && err.contains("is never consumed")
            && err.contains("in an arm of `Shape?`"),
        "{err}"
    );
}

/// R3, the ICE pin: one arm binds a linear local (and consumes it), the other
/// binds nothing. `Moves::join` iterates the first arm's keys and *indexes*
/// the second's map, so without the per-arm truncation the two arms present
/// divergent key sets and the join panics on the missing name.
#[test]
fn poly_eliminator_one_arm_binds_a_local_the_other_does_not_no_ice() {
    let src = format!(
        "type: Spy tag i64 ;\n\
         : drop ( Spy -- ) | s | s Spy> drop ;\n\
         {SHAPE}\
         : ok ( 'T Spy Shape -- 'T )\n\
           ~[ ( Rect )   Rect> drop drop | z | z drop ]\n\
           ~[ ( Circle ) Circle> drop drop ]\n\
           Shape? ;\n\
         : main ( -- ) 1 0 Spy 5 Circle ok . ;\n"
    );
    let prog = Scratch::write("armkeys", &src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n");
}

/// R2: the poly intercept *reaches* the concrete eliminator diagnostics rather
/// than re-guarding them. These carry no mutation entry of their own -- the
/// arms they exercise are the concrete `check_eliminator_call`'s, already
/// covered there; what these pin is the routing.
#[test]
fn poly_eliminator_non_exhaustive_missing_arm_names_the_variant() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T Shape -- 'T ) ~[ ( Rect ) Rect> drop drop ] Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("non-exhaustive call to `Shape?`")
            && err.contains("missing variant `Circle` of enum `Shape`"),
        "the missing arm must be named, not reported as an operand underflow: {err}"
    );
}

#[test]
fn poly_eliminator_duplicate_arm_is_error() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T Shape -- 'T )\n\
           ~[ ( Rect ) Rect> drop drop ]\n\
           ~[ ( Rect ) Rect> drop drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("duplicate arm for variant `Rect` of enum `Shape`"),
        "{err}"
    );
}

#[test]
fn poly_eliminator_untagged_arm_is_error() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T Shape -- 'T )\n\
           ~[ drop ]\n\
           ~[ ( Circle ) Circle> drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("an arm of `Shape?` requires a variant tag"),
        "{err}"
    );
}

/// L3: a quotation that has been through a `| q |` bind keeps its marker and
/// loses its identity (`PolyScope.locals` carries no `QuotRef`), so naming it
/// back puts an untagged arm in the scrutinee position. It must be reported
/// as that, not sent to the abstract-scrutinee diagnostic, which would ask
/// for an enum-kind bound on a type variable this program never wrote.
#[test]
fn poly_eliminator_bound_quotation_as_scrutinee_is_an_untagged_arm() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T: Copy -- 'T )\n\
           ~[ dup ] | q |\n\
           q\n\
           ~[ ( Rect ) Rect> drop drop ]\n\
           ~[ ( Circle ) Circle> drop ]\n\
           Shape? ;\n\
         : main ( -- ) 1 bad . ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("an arm of `Shape?` requires a variant tag"),
        "{err}"
    );
    assert!(!err.contains("enum-kind bound"), "{err}");
}

/// L2/OQ5, the **word-exit** escape route: a quotation left unconsumed has to
/// *be* a value to leave the word, and it has none in a generic body.
#[test]
fn poly_body_materialized_quotation_is_located_error() {
    let src = ": bad ( 'T: Copy -- 'T ) ~[ dup ] swap ;\n\
               : main ( -- ) 1 bad . ;\n";
    let err = check_err(src);
    assert!(
        err.contains("a quotation in the polymorphic body of `bad`")
            && err.contains("is not consumed there"),
        "{err}"
    );
}

/// L2/OQ5, the **arm-exit** escape route, the third of three: a quotation
/// left on the stack by an eliminator arm would have to be materialised to
/// exist past the arm. The asserted line is the *nested* literal's own, not
/// the enclosing arm's -- the arm is written on line 3 and the offender on
/// line 5, so a report of the arm's span blames a quotation that is in fact
/// consumed by the `One?` call.
#[test]
fn poly_eliminator_arm_unconsumed_quotation_reports_the_inner_literal_span() {
    let err = check_err(
        "type: One | A p i64 ;\n\
         : bad ( 'T: Copy One -- 'T )\n\
           ~[ ( A )\n\
              A> drop\n\
              ~[ ] ] One? ;\n\
         : main ( -- ) 1 9 A bad drop ;\n",
    );
    assert!(
        err.contains("a quotation in the polymorphic body of `bad` (line 5) is not consumed there"),
        "{err}"
    );
}

/// L2/OQ5, the **data-operand** escape route -- a different rejection path
/// from the word-exit one above, and the one most likely to regress silently:
/// a predicate stubbed open would let a quotation slot flow into a constructor
/// and materialise. Both a constructor and an operator are checked, since the
/// two reach the guard from different dispatch positions.
#[test]
fn poly_body_quotation_as_data_operand_is_located_error() {
    let ctor = check_err(
        "type: P a i64 ;\n\
         : bad ( 'T: Copy -- 'T ) ~[ dup ] P swap drop ;\n\
         : main ( -- ) 1 bad . ;\n",
    );
    assert!(
        ctor.contains("`P` is not permitted on a quotation literal in `bad`"),
        "{ctor}"
    );
    let arith = check_err(
        ": bad ( 'T: Copy -- 'T ) 1 ~[ dup ] add drop ;\n\
         : main ( -- ) 1 bad . ;\n",
    );
    assert!(
        arith.contains("`add` is not permitted on a quotation literal in `bad`"),
        "{arith}"
    );
    // The marker one slot down is an operand of a *binary* operator just as
    // much as the top one, and the guard must read the whole operand window
    // to say so: reading the top alone leaves this to `poly_delegate_op`,
    // whose concrete suffix stops at the marker and reports `add` as
    // underflowing a stack that is not actually short.
    let deep = check_err(
        ": bad ( 'T: Copy -- 'T ) 1 ~[ dup ] swap add drop ;\n\
         : main ( -- ) 1 bad . ;\n",
    );
    assert!(
        deep.contains("`add` is not permitted on a quotation literal in `bad`"),
        "{deep}"
    );
    assert!(!deep.contains("needs 2 values"), "{deep}");
}

/// OQ6: the row-typed quotation-*consuming* combinator family is deferred,
/// and says so. Never an `unknown word` fallthrough, which is what these emit
/// otherwise -- `poly_call_term` cannot see the poly env, so none of them is
/// even registered on this path. S3b-follow shipped the row-typed combinators
/// (`times`/`if`/user `~[ ]` combinators), and P7.S3d (R1) delivered `call`
/// on a quotation *literal* (it splices the literal's body in place instead;
/// its own coverage lives in `tests/phase7_slice3d.rs`). `branch` pins that
/// the retained guard still fires for the two compiler primitives that stay
/// deferred with no follow-up slice named yet (`branch`/`tag`).
#[test]
fn poly_body_branch_on_a_quotation_is_located_error() {
    let err = check_err(
        ": bad ( 'T: Copy -- 'T ) ~[ dup ] branch ;\n\
         : main ( -- ) 1 bad . ;\n",
    );
    assert!(
        err.contains("`branch` on a quotation in the polymorphic body of `bad`")
            && err.contains("not yet supported")
            && err.contains("name no follow-up slice yet"),
        "{err}"
    );
    assert!(!err.contains("unknown word"), "{err}");
}

/// OQ2: an abstract scrutinee is a `'T` that is *some* enum, which needs an
/// enum-kind bound (P7.S3d) to be constructible at all.
#[test]
fn poly_eliminator_abstract_scrutinee_is_located_error() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T -- )\n\
           ~[ ( Rect ) Rect> drop drop ]\n\
           ~[ ( Circle ) Circle> drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("eliminates `'T`, which is not a concrete enum")
            && err.contains("enum-kind bound"),
        "{err}"
    );
}

/// An eliminator declares its arm parameters inline, so an ordinary `[ ... ]`
/// arm is the wrong bracket here exactly as it is in a concrete body -- and
/// reports it with the same message, so the two paths do not disagree about
/// one spelling.
#[test]
fn poly_eliminator_ordinary_bracket_arm_is_error() {
    let src = format!(
        "{SHAPE}\
         : bad ( 'T Shape -- 'T )\n\
           [ ( Rect )   Rect> mul drop ]\n\
           [ ( Circle ) Circle> dup mul 3 mul drop ]\n\
           Shape? ;\n\
         : main ( -- ) ;\n"
    );
    let err = check_err(&src);
    assert!(
        err.contains("but `Shape?` declares parameter") && err.contains("as inline `~[ ... ]`"),
        "{err}"
    );
}
