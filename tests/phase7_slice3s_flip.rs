//! P7.S3s phase 2 (the flip) goldens: `Ord` is an ordinary library trait, so a
//! user type opts into a comparison-bounded generic word with its own
//! `impl: Ord`, and the two overload-admission filters that used to ask the
//! deleted `is_ord` still keep the library's generic comparisons from
//! swallowing a user's concrete overload of the same name.
//!
//! Everything here goes through the real `sooth` binary against this repo's
//! own `lib/`, because the capability under test spans the whole pipeline:
//! `core::cmp` declaring the trait, `core::prelude` re-exporting it, the
//! parser folding `Ord` into a `Bound::User`, the checker dispatching `cmp`
//! per instantiation, and lowering finding a symbol for each.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch package tree naming this repo's own `lib/` as `core`, so a fixture
/// can `import: core::prelude`/`core::cmp` for real. Removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sooth-p7s3s-flip-{}-{tag}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sooth.pkg"), common::fixture_package(tag)).unwrap();
        Tree(dir)
    }

    fn entry(&self, src: &str) -> PathBuf {
        let path = self.0.join("main.sth");
        std::fs::write(&path, src).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn program(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.entry(src);
    (t, entry)
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(run.status.success(), "the built binary should exit 0");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(
        !build.status.success(),
        "build should have failed; stdout: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    String::from_utf8_lossy(&build.stderr).into_owned()
}

/// `impl: Ord for Point`, written by the user over the library trait -- the
/// exact declaration the pre-flip compiler rejected as a built-in predicate.
/// `cmp` is by value (`( 'T 'T -- Ordering )`, R4), so the two `Point`s are
/// owned locals and are dropped explicitly after their fields are read.
const POINT_IMPL: &str = "type: Point x i64 ;\n\
     impl: Ord for Point\n\
       : cmp\n\
         | a b |\n\
         &a &x @ | ax | &b &x @ | bx |\n\
         a drop b drop\n\
         ax bx lt ~[ Less ] ~[ ax bx gt ~[ Greater ] ~[ Equal ] if ] if ;\n\
     ;\n";

/// The slice's headline exit criterion (1): a `'T: Copy Ord`-bounded generic
/// word instantiated over a **user struct**, built and run.
///
/// `mymax` is instantiated twice in one program, at `Point` and at `i64`, and
/// both answers are asserted. One instantiation would not distinguish "the
/// user's `impl: Ord` was found" from "every `Ord` bound now resolves to the
/// same thing": the `Point` line can only come from the user's own `impl:`
/// block and the `i64` line can only come from `lib/cmp.sth`'s, so the pair
/// also covers criterion 4 (the numeric tower satisfying `Ord` through
/// ordinary `impl:` blocks nobody wrote by hand) in the same run.
#[test]
fn an_ord_bounded_generic_word_instantiates_over_a_user_struct() {
    let (_t, entry) = program(
        "user-struct",
        &format!(
            "import: intrinsics * ;\n\
             import: core::prelude | if Bool Ord lt gt | ;\n\
             import: core::cmp | Ordering Less Equal Greater | ;\n\
             {POINT_IMPL}\
             : mymax ( 'T: Copy Ord 'T -- 'T )\n\
               | a b | a b gt ~[ a ] ~[ b ] if ;\n\
             : main ( -- )\n\
               3 Point 7 Point mymax | m | &m &x @ . m drop\n\
               9 4 mymax . ;\n"
        ),
    );
    assert_eq!(build_and_run(&entry), "7\n9\n");
}

/// An unsatisfied `Ord` is an ordinary user-trait failure now, naming the
/// `impl:` member signature it could not find rather than the deleted
/// `poly_ord_bound_error`'s "not a numeric type" wording (step 8). Asserted by
/// exact text, minus the line/column, which is the fixture's layout rather
/// than the diagnostic's content.
///
/// The error names `cmp`, the trait member, not `lt`, the word the caller
/// wrote: the six surface comparisons are `inline`, so `lt`'s body is spliced
/// and the failing instantiation is `cmp`'s, reported at `lib/cmp.sth`'s own
/// line. The second line -- the useful one, naming the missing `impl:`
/// signature -- is unaffected. Restoring the caller's own attribution needs a
/// splice-origin span carried through unsatisfied-bound reporting, which is a
/// diagnostics feature of its own (recorded as a P7 follow-up).
#[test]
fn an_unsatisfied_ord_bound_names_the_missing_impl() {
    let (_t, entry) = program(
        "no-impl",
        "import: intrinsics * ;\n\
         import: core::prelude | if Bool lt | ;\n\
         type: Vec2 x i64 y i64 ;\n\
         : main ( -- ) 1 1 Vec2 2 2 Vec2 lt ~[ 1 ] ~[ 0 ] if . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("cannot instantiate `'T` of `cmp` with `Vec2` in `main`"),
        "unexpected diagnostic: {err}"
    );
    assert!(
        err.contains("`Vec2` does not satisfy `Ord`: no `( Vec2 Vec2 -- Ordering )` found"),
        "unexpected diagnostic: {err}"
    );
}

/// One module declaring both a concrete `mylt ( Vec2 Vec2 -- Bool )` and an
/// `Ord`-bounded generic `mylt`, where `Vec2` has no `impl: Ord` -- slice
/// 10c's coexistence, preserved across the flip (criterion 6).
///
/// This pins `poly_admits` (`src/check/declarations.rs`), and it is the *only*
/// shape that does: with the `Ord` bound resolving through the `impl:`
/// registry, `poly_admits` declines `Vec2` for the generic candidate, so the
/// two candidates cannot both claim one call site and the pair is declarable.
/// Mutation-verified: replacing `poly_admits`' `PolyType::Var` arm with a bare
/// `true` fails this test with `generic_concrete_overlap_error`. An unbounded
/// generic sibling (`'T: Copy` alone) is rejected by that same rule today, so
/// the `Ord` bound is load-bearing for the coexistence rather than incidental.
///
/// The calls are the *generic* `mylt`, both ways round so the golden pins the
/// dispatched comparison rather than a constant. Calling the concrete `mylt`
/// instead panics at lowering (`checked user word exists`,
/// `src/ir/func_builder/calls.rs`), a pre-existing gap verified at this
/// slice's parent commit with `Bound::Ord` still in place, and unrelated to
/// `Ord`: `ast::overload_symbols` counts poly words when deciding a name is
/// overloaded, so the concrete word gets a `$$0`-suffixed symbol that the
/// call site never records. That is why criterion 6 has no *run* golden for
/// the concrete half; the checker-level half lives in `src/check/poly.rs`
/// (`check_concrete_overload_is_selected_over_an_ord_bounded_generic`).
#[test]
fn a_concrete_overload_coexists_with_an_ord_bounded_generic_of_the_same_name() {
    let (_t, entry) = program(
        "coexist",
        "import: intrinsics * ;\n\
         import: core::prelude | if Bool Ord lt | ;\n\
         type: Vec2 x i64 y i64 ;\n\
         : mylt ( 'T: Copy Ord 'T -- Bool ) lt ;\n\
         : mylt ( Vec2 Vec2 -- Bool )\n\
           | a b | &a &x @ &b &x @ lt | r | a drop b drop r ;\n\
         : main ( -- )
           3 5 mylt ~[ 1 ] ~[ 0 ] if .
           5 3 mylt ~[ 1 ] ~[ 0 ] if . ;\n",
    );
    assert_eq!(build_and_run(&entry), "1\n0\n");
}

/// R6's ruling, made reachable by the flip: give `Vec2` an `impl: Ord` and the
/// coexistence above becomes real ambiguity -- both candidates now admit a
/// `Vec2 Vec2` call -- so `generic_concrete_overlap_error` fires at
/// declaration time. Correct behaviour, not a false positive: ranking two
/// equally-admissible candidates would be real overload resolution, which
/// this language deliberately does not have.
///
/// The twin of the coexistence golden above, differing only by the `impl: Ord
/// for Vec2` block. Together they are a two-way witness that `poly_admits`
/// consults the registry rather than answering a fixed way: one asserts the
/// pair is legal without the `impl:`, the other that it is rejected with it,
/// and no single mutation of that arm can satisfy both.
#[test]
fn an_impl_ord_on_the_concrete_overloads_type_makes_the_pair_an_overlap_error() {
    let (_t, entry) = program(
        "overlap",
        "import: intrinsics * ;\n\
         import: core::prelude | if Bool Ord lt gt | ;\n\
         import: core::cmp | Ordering Less Equal Greater | ;\n\
         type: Vec2 x i64 y i64 ;\n\
         impl: Ord for Vec2\n\
           : cmp\n\
             | a b |\n\
             &a &x @ | ax | &b &x @ | bx |\n\
             a drop b drop\n\
             ax bx lt ~[ Less ] ~[ ax bx gt ~[ Greater ] ~[ Equal ] if ] if ;\n\
         ;\n\
         : mylt ( 'T: Copy Ord 'T -- Bool ) lt ;\n\
         : mylt ( Vec2 Vec2 -- Bool )\n\
           | a b | &a &x @ &b &x @ lt | r | a drop b drop r ;\n\
         : main ( -- ) 5 3 mylt ~[ 1 ] ~[ 0 ] if . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("overlaps a concrete overload of `mylt`; a name cannot mix a generic and a concrete candidate"),
        "unexpected diagnostic: {err}"
    );
}

/// P7.S3s-follow Phase 5: `cmp` is an `inline` trait member, so an `inline`
/// `'T: Ord` word that calls `cmp` has the `impl:` body spliced at the call
/// site. This behavioural golden builds and runs the program, checking the
/// printed output, so a wrong splice is caught as a wrong answer rather than
/// just a shape change. The word is `inline` (spliced into `main`), so the
/// `cmp` call resolves through the `splice_trait_calls` path (keyed by the
/// enclosing splice's uid), the same path the two-splice test below stresses.
const ORD_INLINE_IMPORTS: &str = "import: intrinsics * ;\n\
     import: core::prelude | if Bool Ord lt gt | ;\n\
     import: core::cmp | Ordering Less Equal Greater | ;\n";

#[test]
fn ord_inline_cmp_behavioural_golden() {
    let (_t, entry) = program(
        "ord-inline-golden",
        &format!(
            "{ORD_INLINE_IMPORTS}\
             : my_cmp inline ( 'T: Ord 'T -- i64 )
\
               cmp ~[ ( Less ) drop -1 ] ~[ ( Equal ) drop 0 ] ~[ ( Greater ) drop 1 ] Ordering? ;
\
             : main ( -- )
\
               1 2 my_cmp .
\
               2 1 my_cmp .
\
               1 1 my_cmp . ;
"
        ),
    );
    assert_eq!(build_and_run(&entry), "-1\n1\n0\n");
}

/// P7.S3s-follow Phase 5 (section 3): two member splices under one enclosing
/// uid, checked for the right *value*, not merely for building. Both `cmp`
/// calls resolve through `splice_trait_calls` under the same uid; both are
/// spliced; the two results are independent and correct. They do not collide
/// because a member splice renames through `alpha_rename_member_locals`,
/// whose suffix is disjoint from the enclosing splice's.
///
/// Takes four values (two pairs) rather than reusing one pair twice: the
/// merged checker (P7.S4b `8a5add3`) walks an inline combinator body with a
/// `Bound::User` through `check_poly_body`, so `cmp` is checked as a
/// consuming call and reusing `a`/`b` would be a linear use-after-move. The
/// two splices still happen under the same enclosing uid — the point of the
/// test — they just operate on independent values.
#[test]
fn ord_inline_cmp_two_splices_produce_correct_value() {
    let (_t, entry) = program(
        "ord-inline-two-splice",
        &format!(
            "{ORD_INLINE_IMPORTS}\
             : cmp_twice inline ( 'T: Ord 'T 'T 'T -- i64 i64 )
\
               | a b c d |
\
               a b cmp ~[ ( Less ) drop -1 ] ~[ ( Equal ) drop 0 ] ~[ ( Greater ) drop 1 ] Ordering?
\
               c d cmp ~[ ( Less ) drop -1 ] ~[ ( Equal ) drop 0 ] ~[ ( Greater ) drop 1 ] Ordering? ;
\
             : main ( -- )
\
               1 2 2 1 cmp_twice . .
\
               2 1 1 2 cmp_twice . . ;
"
        ),
    );
    // 1 2 2 1: a=1 b=2 c=2 d=1. a b cmp = Less -> -1; c d cmp = Greater -> 1.
    // 2 1 1 2: a=2 b=1 c=1 d=2. a b cmp = Greater -> 1; c d cmp = Less -> -1.
    assert_eq!(build_and_run(&entry), "1\n-1\n-1\n1\n");
}

/// A spliced member body's locals must not collide with those of a combinator
/// the body itself splices. `alpha_rename_member_locals`'s suffix is what
/// keeps them apart, because the *uid* does not: a member body is spliced
/// under the member word's own check-time seed (P7.S8's R1), and that seed is
/// also the uid minted for the first combinator splice nested inside it, so a
/// shared suffix renames both `| lhs |`s to one name. `self.locals` is scanned
/// front-to-back, so `bump`'s read of its own `lhs` then finds the member
/// body's `lhs` -- the `Point` -- and the comparison silently answers off the
/// wrong values rather than panicking.
///
/// Measured, not assumed: with `MEMBER_SPLICE_SUFFIX` set to `INLINE_SUFFIX`
/// this program prints `1`/`1`/`1` instead of `-1`/`1`/`0`. `bump` must be the
/// member body's *first* nested splice for the uids to meet, which is why it
/// is called before any other combinator in `cmp`.
///
/// The enclosing `cmp_shadowed` binds `lhs`/`rhs` too. That pairing can no
/// longer collide (a caller's splice uid and a member's seed differ), so it is
/// here as the ordinary shadowing control, not as the witness.
#[test]
fn ord_inline_cmp_member_local_colliding_with_a_nested_splices_local_reads_its_own() {
    let (_t, entry) = program(
        "ord-inline-collide",
        &format!(
            "{ORD_INLINE_IMPORTS}\
             type: Point x i64 ;
\
             : bump inline ( i64 -- i64 ) | lhs | lhs 100 add ;
\
             impl: Ord for Point
\
               : cmp
\
                 | lhs rhs |
\
                 &lhs &x @ | a | &rhs &x @ | b |
\
                 lhs drop rhs drop
\
                 a bump | ab | b bump | bb |
\
                 ab bb ult [ Less ] [ ab bb ugt [ Greater ] [ Equal ] branch ] branch ;
\
             ;
\
             : cmp_shadowed inline ( 'T: Ord 'T 'T 'T -- i64 )
\
               | lhs rhs a b |
\
               lhs drop rhs drop
\
               a b cmp ~[ ( Less ) drop -1 ] ~[ ( Equal ) drop 0 ] ~[ ( Greater ) drop 1 ] Ordering? ;
\
             : main ( -- )
\
               9 Point 9 Point 1 Point 2 Point cmp_shadowed .
\
               9 Point 9 Point 2 Point 1 Point cmp_shadowed .
\
               9 Point 9 Point 1 Point 1 Point cmp_shadowed . ;
"
        ),
    );
    // The compared pair is `a b`, and `bump` adds 100 to each side, so the
    // ordering is theirs: 1 2 -> Less, 2 1 -> Greater, 1 1 -> Equal.
    assert_eq!(build_and_run(&entry), "-1\n1\n0\n");
}

// -- P7.S8: the nested-splice uid rule ------------------------------------

/// P7.S8 (R6): the panic the slice fixes, with **no generic word anywhere in
/// the call chain**. `Point`'s `impl: Ord` body calls the library `lt`/`gt`,
/// which are `inline`, so lowering splices `lt`'s body into `main`, then
/// splices `cmp`-for-`Point`'s body into that, then splices `lt`/`gt` again at
/// `i64` inside it -- three splice levels deep with no `mymax` and no `θ`.
///
/// Before the fix the member splice reused the *caller's* `inline_uid`, so the
/// second level's `(uid, span)` lookup missed its `splice_trait_calls` entry
/// and fell through to the ordinary call path, panicking with `checked user
/// word exists` at `ir/func_builder/calls.rs`.
#[test]
fn a_concrete_impl_ord_delegating_to_lt_builds_and_runs() {
    let (_t, entry) = program(
        "concrete-impl-ord",
        &format!(
            "import: intrinsics * ;\n\
             import: core::prelude | if Bool Ord lt gt | ;\n\
             import: core::cmp | Ordering Less Equal Greater | ;\n\
             {POINT_IMPL}\
             : main ( -- )\n\
               3 Point 7 Point lt ~[ 1 ] ~[ 0 ] if .\n\
               7 Point 3 Point lt ~[ 1 ] ~[ 0 ] if .\n\
               5 Point 5 Point lt ~[ 1 ] ~[ 0 ] if . ;\n"
        ),
    );
    assert_eq!(build_and_run(&entry), "1\n0\n0\n");
}

/// P7.S8 (R7): the same member splice inside a self-tail-recursive loop body.
/// uid minting is static and `emit_back_edge` never reads `splice_uid_stack`,
/// so the loop transform and the uid rule are independent -- this is the guard
/// on that, and it does panic at the same site without the fix.
#[test]
fn a_self_tail_word_comparing_a_user_struct_in_its_loop_builds_and_runs() {
    let (_t, entry) = program(
        "self-tail-impl-ord",
        &format!(
            "import: intrinsics * ;\n\
             import: core::prelude | if Bool Ord lt gt | ;\n\
             import: core::cmp | Ordering Less Equal Greater | ;\n\
             {POINT_IMPL}\
             : countdown ( i64 i64 -- i64 )\n\
               | n acc |\n\
               0 Point n Point lt\n\
               ~[ n 1 sub acc n add countdown ]\n\
               ~[ acc ] if ;\n\
             : main ( -- ) 5 0 countdown . ;\n"
        ),
    );
    // 5+4+3+2+1: the loop runs until `0 < n` is false.
    assert_eq!(build_and_run(&entry), "15\n");
}

/// P7.S8 (R1c): a **bound member call inside a combinator's quotation
/// argument**, in a generic body, instantiated at two distinct types. This
/// shape builds and runs on pristine `main`; it is committed here because it is
/// the counter-example that rules out gating `trait_calls` on
/// `splice_uid_stack.is_empty()`. That blanket gate also fires for an ordinary
/// combinator splice like the `if` below, and this call has no
/// `splice_trait_calls` entry to fall through to (`resolve_splice_member_call`
/// needs a `combinator_sig` this shape does not have), so the broad gate turns
/// it into a `checked user word exists` panic. The narrow
/// `member_splice_depth` gate leaves it alone.
#[test]
fn a_bound_member_call_inside_a_quotation_argument_instantiates_at_two_types() {
    let (_t, entry) = program(
        "member-call-in-quotation",
        &format!(
            "import: intrinsics * ;\n\
             import: core::prelude | if Bool True Ord lt gt | ;\n\
             import: core::cmp | Ordering Less Equal Greater | ;\n\
             {POINT_IMPL}\
             : myeq ( 'T: Copy Ord 'T -- i64 )\n\
               | a b |\n\
               True\n\
               ~[ a b cmp ~[ ( Less ) drop -1 ] ~[ ( Equal ) drop 0 ] ~[ ( Greater ) drop 1 ] Ordering? ]\n\
               ~[ 9 ] if ;\n\
             : main ( -- )\n\
               3 Point 7 Point myeq .\n\
               7 Point 3 Point myeq .\n\
               4 4 myeq . ;\n"
        ),
    );
    assert_eq!(build_and_run(&entry), "-1\n1\n0\n");
}

/// P7.S8 (R2): an ordinary word declared **before** the `impl:` block, which is
/// what makes the seed *formula* observable rather than merely its presence.
///
/// Every other fixture here puts the `impl:` block first, so its member lands at
/// `module.words[0]` and `word_idx * INLINE_UID_STRIDE` is 0 -- a constant-`0`
/// seed is accidentally right there. One leading word shifts the member to index
/// 1, so lowering must derive the seed from the member's own index or its body's
/// splices look up a `(0, span)` key the checker never wrote.
///
/// Measured, and worth stating because the obvious alternative fixture does not
/// work: *two* user `impl: Ord` blocks do not discriminate the formula. Under a
/// constant seed their splices collide on a key that resolves to a numeric
/// `impl: Ord` either way, and `lib/cmp.sth`'s numeric bodies are all the same
/// terms over type-directed intrinsics, so the wrong dispatch computes the same
/// answer. A leading word makes the lookup miss outright.
#[test]
fn a_word_declared_before_the_impl_block_shifts_the_members_uid_seed() {
    let (_t, entry) = program(
        "leading-word-impl-ord",
        &format!(
            "import: intrinsics * ;\n\
             import: core::prelude | if Bool Ord lt gt | ;\n\
             import: core::cmp | Ordering Less Equal Greater | ;\n\
             : leading ( i64 -- i64 ) 1 add ;\n\
             {POINT_IMPL}\
             : main ( -- )\n\
               2 leading Point 7 Point lt ~[ 1 ] ~[ 0 ] if .\n\
               7 Point 2 leading Point lt ~[ 1 ] ~[ 0 ] if . ;\n"
        ),
    );
    assert_eq!(build_and_run(&entry), "1\n0\n");
}
