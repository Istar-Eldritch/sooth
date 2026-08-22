//! Phase 7 Slice 3b-follow goldens: a **row-typed inline combinator** consumed
//! in a **non-inline polymorphic** body, so a generic word can loop and branch
//! as a monomorphized function instead of forcing every call site to splice its
//! whole body.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch program directory, removed on drop (`tests/phase7_slice3b.rs`'s
/// own pattern). Multi-file here rather than single-file: `times` lives in
/// `lib/combinators.sth` and is reached by `import:`, which is also what makes
/// its name arrive at the call site *mangled* (`times__m1`) -- the spelling the
/// dispatch has to resolve.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3bf-{}-{tag}-{seq}", std::process::id()));
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

/// An `import:` line for one of the shipped library files, by absolute path, so
/// a scratch program in the temp dir reaches the real `lib/`.
fn import_lib(file: &str, selective: &str) -> String {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib").join(file);
    format!("import: \"{}\" c | {selective} | ;\n", lib.display())
}

fn build_and_run(src: &Path) -> (String, i32) {
    let binary = driver::build_with_manifest(src, common::manifest_for(src).as_deref())
        .expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn build_err(tag: &str, src: &str) -> String {
    let scratch = Scratch::write(tag, src);
    driver::build_with_manifest(
        scratch.path(),
        common::manifest_for(scratch.path()).as_deref(),
    )
    .expect_err("this program should be rejected")
}

/// Two user-written always-spliced combinators whose declared row is the same
/// on both sides, so the library is not the only witness for the mechanism and
/// a single-file program can exercise it. They differ only in how many times
/// they run the arm, which is what makes a run-count observable below.
const ONCE_AND_TWICE: &str = ": once  inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call ;\n\
     : twice inline ( ..s ~[ ..s -- ..s ] -- ..s ) | f | f call f call ;\n";

#[test]
fn times_in_a_non_inline_generic_body_compiles_and_runs() {
    // R2/R3, the non-shape-changing exit criterion: `times` declares
    // `~[ ..s i64 -- ..s ]`, its row grounds to the caller region below its
    // fixed inputs, and the arm is walked over that region. The word is
    // deliberately *not* `inline`, so it is monomorphized once per
    // instantiation rather than spliced per call site -- and it is called at
    // two types, so `'T` is carried rigidly through the loop rather than
    // coincidentally matching one instantiation.
    //
    // The arm swaps the two carried slots, so the result reads the *parity of
    // the iteration count* straight off the loop: an odd count leaves the pair
    // swapped and an even one does not. That is what makes these assertions
    // able to fail while still compiling -- a wrong iteration count or a wrong
    // exit-row join changes the printed value rather than the build.
    let src = format!(
        "{}\
         : lower ( 'T: Copy Ord 'T i64 -- 'T ) ~[ | i | swap ] times drop ;\n\
         : main ( -- )\n\
           5 7 3 lower .\n\
           5 7 2 lower .\n\
           2.5 3.5 1 lower .\n\
           2.5 3.5 0 lower . ;\n",
        import_lib("combinators.sth", "times")
    );
    let scratch = Scratch::write("times-body", &src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "7\n5\n3.5\n2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn user_row_combinator_in_a_non_inline_generic_body_compiles_and_runs() {
    // R2: the dispatch is driven by the callee's declared `PolySig`, not by
    // name, so a *user* combinator with no library involvement reaches it too.
    // The two words differ only in their callee, and the same arm under each
    // prints a different value -- so the arm is genuinely run by the callee's
    // own body, once or twice, rather than checked and discarded.
    let src = format!(
        "{ONCE_AND_TWICE}\
         : odd  ( 'T: Copy Ord 'T -- 'T ) ~[ swap ] once  drop ;\n\
         : even ( 'T: Copy Ord 'T -- 'T ) ~[ swap ] twice drop ;\n\
         : main ( -- ) 5 7 odd . 5 7 even . 2.5 3.5 odd . 2.5 3.5 even . ;\n"
    );
    let scratch = Scratch::write("user-row", &src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "7\n5\n3.5\n2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn single_arm_times_arm_deepening_the_row_is_error() {
    // R3, the soundness test (not a diagnostic test): a lone arm is checked
    // against the **pre-seeded grounded entry row**, not against a sibling.
    // The eliminator's cross-arm rule is seeded from the *first arm's* exit,
    // because an eliminator's arms are supposed to change the shape; with that
    // seeding `times` has nothing to compare against at all and this body --
    // whose arm leaves one more slot than it entered with, so the loop's
    // back-edge stack depth does not match its entry -- is wrongly accepted.
    let err = build_err(
        "times-deepens",
        &format!(
            "{}\
             : bad ( 'T: Copy Ord 'T i64 -- 'T ) ~[ dup ] times drop ;\n\
             : main ( -- ) 5 7 3 bad . ;\n",
            import_lib("combinators.sth", "times")
        ),
    );
    assert!(
        err.contains(
            "the quotation passed to `times` in `bad` (line 2) was declared `~[ ..s i64 -- ..s ]`, but it leaves `'T 'T i64 i64` where that requires `'T 'T`"
        ),
        "the arm must be held to the row it entered with: {err}"
    );
}

#[test]
fn single_arm_times_arm_consuming_the_row_is_error() {
    // R3, the other direction: an arm that eats a slot of the grounded row it
    // entered with. Both directions are pinned because a comparison written
    // one-sided (`exit.len() < want.len()`) passes one and not the other.
    let err = build_err(
        "times-consumes",
        &format!(
            "{}\
             : bad ( 'T: Copy Ord 'T i64 -- 'T ) ~[ | i | drop ] times ;\n\
             : main ( -- ) 5 7 3 bad . ;\n",
            import_lib("combinators.sth", "times")
        ),
    );
    assert!(
        err.contains("but it leaves `'T` where that requires `'T 'T`"),
        "an arm may not consume the row it rides on: {err}"
    );
}

#[test]
fn times_arm_leaving_a_rigid_variable_as_a_concrete_type_is_error() {
    // L1: the row is compared **structurally under rigid variables**. This
    // arm's exit is the same *depth* as the row it entered with, and differs
    // only in that a `'T` slot has become `i64` -- so it is caught by the
    // per-slot comparison and not by the length check above.
    let err = build_err(
        "times-rigid",
        &format!(
            "{}\
             : bad ( 'T: Copy Ord 'T i64 -- 'T ) ~[ | i | drop 1 ] times drop ;\n\
             : main ( -- ) 5 7 3 bad . ;\n",
            import_lib("combinators.sth", "times")
        ),
    );
    assert!(
        err.contains("but it leaves `'T i64` where that requires `'T 'T`"),
        "`'T` is never bound to the arm's `i64`: {err}"
    );
}

#[test]
fn narrowed_guard_keeps_call_branch_and_tag_located() {
    // R4: the three consumers this slice does *not* deliver keep their located
    // rejection. Each asserts the **exact message**, which is what makes the
    // guard's deletion observable at all: these bodies put the quotation on
    // top, so without the guard they fall to the `QuotLit` operand window
    // ("`call` is not permitted on a quotation literal"), not to `unknown
    // word` -- that is only where they land when the quotation sits deeper
    // than the window. Both fallbacks name the word, so a bare "the build
    // fails" assertion, and even one matching on the word alone, is a placebo.
    //
    // `tag`'s arm is reached the same way the other two are: the guard is
    // name-based, so an untagged quotation literal is enough to reach it.
    //
    // The *second* line is pinned too, and separately: it is the one that says
    // which follow-up would take each name, and `call` is the only one of the
    // three that has one -- P7.S3d's spec excludes `branch`/`tag` while
    // pointing them back here. Asserting the first line alone leaves that
    // claim unguarded for two of the three names.
    // `branch`'s condition is a `true` literal rather than a computed
    // comparison (as it is below, at `bad`'s other call sites): this program
    // is never built successfully (the assertion is on the rejection), so the
    // condition's actual value never matters, and a comparison would only
    // hit the P7.S3k gap (a non-inline generic body cannot call another
    // generic word, comparisons included) before ever reaching the rejection
    // this test is pinning.
    for (name, body) in [
        ("call", "~[ drop ] call"),
        ("branch", "true ~[ drop ] ~[ swap drop ] branch"),
        ("tag", "~[ dup ] tag drop"),
    ] {
        let err = build_err(
            name,
            &format!(": bad ( 'T: Copy Ord 'T -- 'T ) {body} ;\n: main ( -- ) 5 7 bad . ;\n"),
        );
        assert!(
            err.contains(&format!(
                "error: `{name}` on a quotation in the polymorphic body of `bad` (line 1) is not yet supported"
            )),
            "`{name}` must stay a located rejection naming itself, not `unknown word`: {err}"
        );
        assert!(
            err.contains(
                "`call` on a literal is P7.S3d's own exit criterion, and `branch`/`tag` name no follow-up slice yet"
            ),
            "`{name}`'s rejection must say which follow-up would take it: {err}"
        );
    }
}

#[test]
fn shape_changing_if_in_a_non_inline_generic_body_compiles_and_runs() {
    // R3, the shape-changing exit criterion. `if` declares
    // `( ..a bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`: nothing fixes the
    // exit row, so the arms are held to *each other* and the call's exit is
    // what they agreed on. Both arms consume one slot of the two-slot row they
    // enter with, so an exit taken from the *entry* row would be one slot too
    // deep and this body would not typecheck at all.
    //
    // `mymax` is the spec's own exit-criterion program, and it is called at two
    // instantiations so `'T` is carried rigidly through the arms rather than
    // coincidentally matching one of them. Both arms are taken at each
    // instantiation, so arm routing is not vacuous: swapping the arms prints
    // the minimum instead and these assertions fail while still compiling.
    //
    // `gt` is computed by `main`, a monomorphic caller, and handed in as a
    // third input rather than called from inside `mymax`'s own (non-inline,
    // generic) body: P8.S2 (`tests/phase8_slice2.rs`'s
    // `a_poly_word_calling_an_imported_poly_word_names_the_narrowing`)
    // deliberately narrowed a non-inline generic body to never reach another
    // generic word, comparisons included -- restoring that reach is P7.S3k,
    // named and scoped but not yet delivered. The comparison is still real
    // per-instantiation data computed at each call, not a hardcoded literal,
    // so arm routing stays genuinely non-vacuous.
    let src = ": mymax ( 'T 'T bool -- 'T ) ~[ drop ] ~[ swap drop ] if ;\n\
               : main ( -- )\n\
                 2 9 over over gt mymax .\n\
                 9 2 over over gt mymax .\n\
                 2.5 9.5 over over gt mymax .\n\
                 9.5 2.5 over over gt mymax . ;\n";
    let scratch = Scratch::write("shape-changing-if", src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "9\n9\n9.5\n9.5\n");
    assert_eq!(code, 0);
}

#[test]
fn unless_reaches_the_dispatch_rather_than_the_operand_window() {
    // R2: `unless` is the witness that the dispatch is driven by the callee's
    // declaration and not by a list of names. It never reached the old name
    // guard at all -- it is not one of the names that guard lists -- and
    // landed on the `QuotLit` operand window instead ("`unless` is not
    // permitted on a quotation literal"), a rejection that did not even
    // mention the deferral. Its arms are `if`'s swapped, so `mymin` returns
    // the *minimum* from the same body `mymax` uses for the maximum: a
    // dispatch that ignored the callee and hardcoded `if` would print the
    // maximum here and fail while still compiling.
    //
    // `gt` is computed by `main` for the same reason `mymax` above computes
    // it there: P7.S3k (not yet delivered).
    let src = ": mymin ( 'T 'T bool -- 'T ) ~[ drop ] ~[ swap drop ] unless ;\n\
               : main ( -- )\n\
                 2 9 over over gt mymin .\n\
                 9 2 over over gt mymin .\n\
                 2.5 9.5 over over gt mymin .\n\
                 9.5 2.5 over over gt mymin . ;\n";
    let scratch = Scratch::write("unless", src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "2\n2\n2.5\n2.5\n");
    assert_eq!(code, 0);
}

#[test]
fn shape_changing_arms_disagreeing_on_a_rigid_variable_is_error() {
    // L1: the arms are compared **structurally under rigid variables**. These
    // two leave the same *depth*, differing only in that one leaves `'T` where
    // the other leaves `i64` -- so the per-slot comparison is what catches it,
    // and binding `'T := i64` (which would silently retype the arm already
    // checked) is what the rejection refuses.
    let err = build_err(
        "if-rigid",
        ": bad ( 'T: Copy Ord 'T -- 'T ) true ~[ drop ] ~[ drop drop 1 ] if ;\n\
         : main ( -- ) 5 7 bad . ;\n",
    );
    assert!(
        err.contains(
            "the arms of `if` in `bad` (line 1) disagree: an earlier one leaves `'T`, this one leaves `i64`"
        ),
        "`'T` is never bound to the sibling arm's `i64`: {err}"
    );
}

#[test]
fn shape_changing_arms_disagreeing_on_depth_is_error() {
    // R3: the other way two arms can disagree. Pinned separately because a
    // per-slot comparison written without a length check passes this one: the
    // shorter arm's slots all agree with the longer arm's prefix.
    let err = build_err(
        "if-depth",
        ": bad ( 'T: Copy Ord 'T -- 'T ) true ~[ drop ] ~[ swap ] if ;\n\
         : main ( -- ) 5 7 bad . ;\n",
    );
    assert!(
        err.contains(
            "the quotations passed to `if` leave different stack shapes: an earlier one leaves `'T`, this one leaves `'T 'T`"
        ),
        "the arms must agree on depth too: {err}"
    );
}

#[test]
fn non_literal_arm_operand_is_located_and_does_not_panic() {
    // OQ4: an arm is a splice-consumed quotation literal written at the call
    // site, or it is a located rejection here. Both routes to a non-literal
    // operand are checked: a value that is not a quotation at all, and a
    // literal that went through a `| f |` bind, which keeps the type and loses
    // the identity. Neither may be carried into lowering, where a quotation in
    // a generic body has no runtime representation and a row-combinator arm
    // that is not a literal is a backend panic -- so the assertion is on the
    // *message*: a panic would fail `build` too, and `expect_err` alone cannot
    // tell the two apart.
    let not_a_quotation = build_err(
        "oq4-value",
        ": bad ( 'T: Copy Ord 'T -- 'T ) true 1 ~[ swap drop ] if ;\n\
         : main ( -- ) 5 7 bad . ;\n",
    );
    assert!(
        not_a_quotation.contains(
            "`if` in the polymorphic body of `bad` (line 1) needs a quotation literal written at the call site, found `i64`"
        ),
        "a data operand at an arm position is located: {not_a_quotation}"
    );
    let through_a_local = build_err(
        "oq4-local",
        ": bad ( 'T: Copy Ord 'T -- 'T ) ~[ drop ] | f | true f ~[ swap drop ] if ;\n\
         : main ( -- ) 5 7 bad . ;\n",
    );
    assert!(
        through_a_local.contains(
            "needs a quotation literal written at the call site, found a quotation read back out of a local"
        ),
        "a quotation that lost its identity is located: {through_a_local}"
    );
}

#[test]
fn ordinary_bracket_arm_is_the_inline_parameter_diagnostic() {
    // L4: the arms stand at parameters declared `~[ ]` (`lib/core.sth`), so an
    // ordinary `[ ... ]` arm is the wrong bracket, not a new kind of error. It
    // must produce the diagnostic the concrete path already gives that
    // mistake, so the two paths do not disagree about one spelling.
    let err = build_err(
        "ordinary-bracket",
        ": bad ( 'T: Copy Ord 'T -- 'T ) true [ drop ] ~[ swap drop ] if ;\n\
         : main ( -- ) 5 7 bad . ;\n",
    );
    assert!(
        err.contains(
            "this argument is an ordinary `[ ... ]` quotation but `if` declares parameter"
        ) && err.contains("as inline `~[ ... ]`"),
        "an ordinary bracket at an inline parameter keeps its own diagnostic: {err}"
    );
}

#[test]
fn arm_borrows_are_unioned_not_picked() {
    // L3, the false-accept guard, now reached through the *combinator*
    // dispatch rather than the eliminator. `PolyScope`'s borrow table is keyed
    // by place and a **missing** record reads as "no conflict", so a merge that
    // picked one arm silently admits a later use of the place the other arm
    // borrowed. Both directions are asserted: "pick arm A" keeps `x` and drops
    // `y`, so an `x`-only assertion would not flip.
    let program = |later: &str| {
        format!(
            "type: P a i64 ;\n\
             : bad ( 'T: Copy P P -- 'T )\n\
               | x y | true ~[ &!x ] ~[ &!y ] if\n\
               {later} drop drop ;\n\
             : main ( -- ) ;\n"
        )
    };
    let err_x = build_err("union-x", &program("x"));
    assert!(
        err_x.contains("cannot name `x`") && err_x.contains("a mutable borrow of it is still live"),
        "arm A's `&!x` must survive the merge: {err_x}"
    );
    let err_y = build_err("union-y", &program("y"));
    assert!(
        err_y.contains("cannot name `y`") && err_y.contains("a mutable borrow of it is still live"),
        "arm B's `&!y` must survive the merge: {err_y}"
    );
}

#[test]
fn cross_arm_borrow_mutability_disagreement_is_error() {
    // L3: one place borrowed at two mutabilities across two arms. The unioned
    // table cannot hold both records, and erasing either would read as "no
    // conflict" at a later use, so the disagreement is named instead.
    let err = build_err(
        "mutability",
        "type: P a i64 ;\n\
         : bad ( 'T: Copy P -- 'T ) | x | true ~[ &!x @ drop ] ~[ &x @ drop ] if ;\n\
         : main ( -- ) ;\n",
    );
    assert!(
        err.contains("the arms of `if` in `bad` (line 2) borrow `x` differently")
            && err.contains("`&!x`")
            && err.contains("`&x`"),
        "{err}"
    );
}

#[test]
fn arm_local_bound_and_leaked_is_error_and_one_sided_binding_is_not() {
    // R1: the `Scope::leave` analogue, through the combinator dispatch. The
    // poly walk has no block scope, so a linear local bound inside an arm and
    // dropped on the floor there is only caught by the arm walk's own check --
    // which must run *before* the truncation that would erase it.
    const SPY: &str = "type: Spy tag i64 ;\n: drop ( Spy -- ) | s | s Spy> drop ;\n";
    let err = build_err(
        "leak",
        &format!(
            "{SPY}: bad ( 'T Spy -- 'T ) true ~[ | z | ] ~[ | z | z drop ] if ;\n\
             : main ( -- ) ;\n"
        ),
    );
    assert!(
        err.contains("the local `z` of type `Spy`, bound in an arm of `if` in `bad` (line 3)")
            && err.contains("is never consumed"),
        "{err}"
    );
    // And the shape that must *not* be an error: one arm binds a local, the
    // other does not. `Moves::join` indexes the sibling's map by this arm's
    // keys, so without the truncation this pair panics rather than compiling.
    let src = format!(
        "{SPY}: ok ( 'T Spy -- 'T ) true ~[ | z | z drop ] ~[ drop ] if ;\n\
         : main ( -- ) 7 9 Spy ok . ;\n"
    );
    let scratch = Scratch::write("one-sided-bind", &src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

#[test]
fn a_slot_declared_above_a_produced_row_is_located() {
    // R3: the produced row is read straight off an arm's exit, so a parameter
    // declaring a slot *above* that row (`~[ ..a -- ..b i64 ]`) would need
    // that slot stripped back off first -- a rule neither quotation consumer
    // shares. Located, and named against the declaration rather than against
    // the arm, which is written correctly.
    //
    // `bad` declares `-- 'T i64`, the *un-stripped* row itself, rather than
    // just `-- 'T`: with the guard deleted, that is what lets this reach
    // `ir/func_builder/quotation.rs`'s row-length arithmetic instead of being
    // caught first by an ordinary stack-effect-mismatch check, so the guard
    // is what stands between this program and a backend panic
    // (`attempt to subtract with overflow`), not just a worse diagnostic.
    let err = build_err(
        "above-the-row",
        ": pick inline ( ..a bool ~[ ..a -- ..b i64 ] ~[ ..a -- ..b i64 ] -- ..b )\n\
           | pick--e | | pick--t | | pick--c | pick--c tag pick--t pick--e branch drop ;\n\
         : bad ( 'T: Copy Ord 'T -- 'T i64 ) true ~[ drop 1 ] ~[ swap drop 1 ] pick ;\n\
         : main ( -- ) 5 7 bad . . ;\n",
    );
    assert!(
        err.contains(
            "`pick` declares `~[ ..a -- ..b i64 ]`, which a call in the polymorphic body of `bad` (line 3) cannot ground"
        ),
        "{err}"
    );
}

#[test]
fn inline_generic_body_still_splices_a_row_combinator() {
    // Regression: the whole `inline` route this slice exists to *avoid* forcing
    // is untouched. An `inline` generic word consuming `if` is spliced into its
    // concrete caller and grounds its row against a real stack
    // (`examples/poly_if.sth`'s shape), and that path does not go through the
    // poly walk at all.
    let src =
        ": mymax inline ( 'T: Copy Ord 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;\n\
               : main ( -- ) 2 9 mymax . 9.5 2.5 mymax . ;\n";
    let scratch = Scratch::write("inline-splice", src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "9\n9.5\n");
    assert_eq!(code, 0);
}

/// Phase 4's golden: a **non-inline** generic word that loops with a literal
/// `times` carrying an **inner `if`**, so both tiers this slice ships (R2's
/// non-shape-changing `times`, R3's shape-changing `if`) are exercised in one
/// undivided body -- and, per the spec's own review fix, with no
/// self-recursion (`P7.S3g`'s standing limit).
///
/// `clampsum` takes three `'T` parameters (`hi`, `lo`, `val`) plus the
/// iteration count: `( 'T: Copy Ord 'T 'T i64 -- 'T )` is the spec's own
/// literal header, and the bound-declaring first `'T` is itself an input, so
/// three `'T` values enter, not two. Each iteration clamps `val` into
/// `[lo, hi]` in two comparisons, each its own `if`: `over over gt` peeks the
/// compared pair the same way `mymax` does (R3), leaving the row intact
/// underneath for the arm to pick from, and a `rot`/`rot rot` re-aligns the
/// three-element row back to the entry convention (`hi lo val`) between and
/// after the two comparisons, so the *next* `times` iteration (and the
/// loop's own row check, R3's pre-seeded baseline) sees the same shape it
/// started with. The clamp is idempotent after the first iteration, so
/// `n == 0` is what discriminates a real loop from one that never executes.
///
/// **Blocked on P7.S3k, not a fixture bug.** `gt`'s comparison is recomputed
/// fresh every iteration against the loop-carried `val`, so unlike `mymax`/
/// `mymin` above it cannot be hoisted out to a monomorphic caller and passed
/// in -- the whole point is a *fresh* comparison each pass. Every other route
/// to a runtime `bool` inside a non-inline generic body is blocked too: the
/// raw comparison intrinsics (`ugt` et al.) only set a condition `branch`
/// consumes, and `branch` itself is one of the three primitives this slice's
/// own R4 keeps as a located rejection in a non-inline body (`call`/`branch`/
/// `tag`); pre-warming a concrete instantiation of `gt` elsewhere in the same
/// file doesn't help either, since the poly-body dispatch never looks in
/// `env` for a callee that has no concrete registration yet. `clampsum` is
/// the exit-criterion program this spec's own text names, and it stays here,
/// unrun (see the two `#[ignore]`d tests below), as the standing witness for
/// why P7.S3k exists rather than being quietly reworked into an algorithm
/// that dodges the gap.
const CLAMPSUM: &str = ": clampsum ( 'T: Copy Ord 'T 'T i64 -- 'T )
  ~[ | i |
     over over gt ~[ drop dup ] ~[ ] if
     rot
     over over gt ~[ swap drop dup ] ~[ ] if
     rot rot
  ] times
  swap drop swap drop ;
";

#[test]
#[ignore = "blocked on P7.S3k (docs/roadmap/P7-language-prereqs.md): `clampsum`'s \
           per-iteration `gt` is a non-inline generic body calling another \
           generic word, which P8.S2 deliberately closed off \
           (tests/phase8_slice2.rs::a_poly_word_calling_an_imported_poly_word_names_the_narrowing); \
           re-enable once P7.S3k lands"]
fn clampsum_golden_behavioural_matrix() {
    // Mutation-tested guard: deleting the R2/R3 combinator dispatch makes
    // this fail to *compile* with the located rejection
    // (`poly_quotation_combinator_unsupported_error`), so a compile failure
    // is the primary failure mode this golden guards against -- which is why
    // the assertions below must also be able to fail *while compiling*, by
    // exercising both `if` arms and an `n == 0` no-loop path across two `'T`
    // instantiations (i64 and f64), so a wrong arm-routing or a wrong
    // exit-row join changes a printed value instead of breaking the build.
    let src = format!(
        "{}{}\
         : main ( -- )\n\
           10 0 3 5 clampsum .\n\
           10 0 -3 5 clampsum .\n\
           10 0 15 5 clampsum .\n\
           10 0 5 0 clampsum .\n\
           10.0 0.0 3.5 2 clampsum .\n\
           10.0 0.0 -3.5 2 clampsum .\n\
           10.0 0.0 15.5 2 clampsum . ;\n",
        import_lib("combinators.sth", "times"),
        CLAMPSUM,
    );
    let scratch = Scratch::write("clampsum-behavioural", &src);
    let (stdout, code) = build_and_run(scratch.path());
    assert_eq!(stdout, "3\n0\n10\n5\n3.5\n0\n10\n");
    assert_eq!(code, 0);
}

#[test]
#[ignore = "blocked on P7.S3k, same reason as clampsum_golden_behavioural_matrix above"]
fn clampsum_structural_characterization_one_definition_per_instantiation() {
    // CHARACTERIZATION ONLY, no mutation-guard claim (per the spec: a wrong
    // R3 that spliced the arms into the caller is a **lowering** property,
    // and this slice changes no lowering -- see the spec's own "Lowering"
    // section). This pins the code-size claim the slice is scheduled on: a
    // non-inline generic word's body is emitted once per instantiation it is
    // actually reached from, not once per call site. Four call sites (three
    // `i64`, one `f64`) must therefore show exactly two definitions -- over
    // emitted QBE, since `nm` is a known placebo here (`poly_indices`
    // excludes poly template words from symbol minting).
    let src = format!(
        "{}{}\
         : main ( -- )\n\
           10 0 3 5 clampsum .\n\
           10 0 -3 5 clampsum .\n\
           10 0 15 5 clampsum .\n\
           10.0 0.0 3.5 2 clampsum . ;\n",
        import_lib("combinators.sth", "times"),
        CLAMPSUM,
    );
    let scratch = Scratch::write("clampsum-structural", &src);
    let ssa = driver::emit_ssa_with_manifest(
        scratch.path(),
        common::manifest_for(scratch.path()).as_deref(),
    )
    .expect("program should build");
    let call_sites = ssa.matches("call $sooth_mono_clampsum").count();
    let definitions = ssa
        .lines()
        .filter(|l| l.contains("function") && l.contains("clampsum"))
        .count();
    assert_eq!(
        call_sites, 4,
        "all four call sites should reach a `clampsum` instantiation: {ssa}"
    );
    assert_eq!(
        definitions, 2,
        "one definition per instantiation (i64, f64), not per call site: {ssa}"
    );
}
