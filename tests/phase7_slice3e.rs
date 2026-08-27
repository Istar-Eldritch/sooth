//! P7.S3e goldens: `trait:` declaration, `impl:` binding, export/import
//! parity with `type:`/`extern:`, and the body-side bound rejections.
//!
//! Driven through the real `sooth` binary, so a cross-module scenario
//! exercises the whole-closure trait pre-pass in `driver::assemble_module`.
//! Phase 4's lowering *is* covered here (a bound-directed call that
//! check-passes now runs and its output is asserted, e.g. the array-sort
//! consumer below), alongside the declaration/export/import/collision
//! rejections. The accepting side's finer-grained detail (obligation
//! recording, per-instantiation symbol resolution) stays pinned at check
//! level in `check::poly`'s and `driver`'s own tests.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch tree of `.sth` files outside any package, so imports resolve by
/// quoted path and no manifest is involved. Removed on drop. Files are
/// written verbatim (no `common::fixture_source` auto-import injection):
/// none of these fixtures need `core`/`intrinsics`.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3e-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sooth_build(entry: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_ok(entry: &Path) {
    let build = sooth_build(entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    std::fs::remove_file(entry.with_extension("")).ok();
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// Phase 4: build, run, and return stdout.
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
    assert!(run.status.success(), "the built binary should exit 0");
    std::fs::remove_file(&binary).ok();
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// A scratch tree with a `sooth.pkg` naming this repo's own `lib/` as `core`,
/// so a fixture can `import: core::prelude`/`core::bool` (P7.S3e's `sort`
/// golden needs `if`/`lt`/`gt`/`Bool` for its comparator).
fn tree_with_core(tag: &str) -> Tree {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: {tag} ;\nlayer: hosted ;\ndepends: core path \"{}/lib\" ;\n",
            env!("CARGO_MANIFEST_DIR")
        ),
    );
    t
}

/// A single-file scratch program, for a golden that needs no import closure.
/// `intrinsics` is imported unconditionally: every fixture in this file uses
/// `drop`, and this is not itself the subject under test.
fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// A trait declares, and a second declaration of the same name in the same
/// module is a duplicate error in the same shape `type:`/`static:` already
/// produce.
#[test]
fn trait_declares_and_duplicate_is_rejected() {
    let (_t, entry) = single_file(
        "duplicate",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("duplicate trait `Show`"), "{err}");
}

/// A trait colliding with a `type:` of the same name in one module is a
/// cross-kind rejection -- the requirement `check_trait_decls`'s generalized
/// `colliding_name_kind` call site exists to catch.
#[test]
fn trait_collides_with_a_type_of_the_same_name() {
    let (_t, entry) = single_file(
        "cross-kind",
        "type: Point x i64 y i64 ;\n\
         trait: Point['T] : foo ( &'T -- ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("already the name of a type"), "{err}");
}

/// A user `trait: Copy` collides with the pre-seeded predicate entry (R2):
/// an ordinary duplicate/collision, not a bespoke reserved-word check.
#[test]
fn user_trait_named_copy_collides_with_the_reserved_entry() {
    let (_t, entry) = single_file(
        "reserved-copy",
        "trait: Copy['T] : foo ( &'T -- ) ; ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("already the name of a trait"), "{err}");
}

/// P7.S3p: a member whose receiver is not its last declared input (an
/// index/lookup shape) declares *and* dispatches -- the variable comes off the
/// body's bounds by member name, not off the top of the stack, so the receiver
/// sits wherever the signature puts it.
#[test]
fn trait_member_with_a_non_trailing_receiver_dispatches() {
    let (_t, entry) = single_file(
        "non-trailing-receiver",
        "type: Point x i64 y i64 ;\n\
         trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
         impl: Indexable for Point\n\
           : at | p n | n drop p &x @ ;\n\
         ;\n\
         : uses ['T: Indexable] ( &'T -- i64 ) 0 at ;\n\
         : main ( -- ) 7 2 Point |p| &p uses . p drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "7\n");
}

/// A member binding the trait's variable in *no* input is admitted at
/// `trait:` declaration time as of P7.S3t: a call to the *wrapping* generic
/// word can now ground the variable explicitly (`f[Point]`), so the gate no
/// longer needs to shut the door before that mechanism runs. See
/// `tests/phase7_slice3t.rs` for the dispatching golden; this only pins that
/// declaration alone no longer fails.
#[test]
fn trait_member_with_a_zero_input_receiver_is_accepted() {
    let (_t, entry) = single_file(
        "zero-input-receiver",
        "trait: Show['T] : fresh ( -- i64 ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let build = sooth_build(&entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
}

/// P7.S3p selects a member by name alone, ahead of every builtin arm, so it
/// cannot fall through to the builtin when the operands do not fit. A member
/// named `call` would therefore capture *every* `call` in any body bounded by
/// its trait, making quotation application unreachable there: a body
/// `( &'T: C [ -- i64 ] -- i64 ) call ...` applying its quotation parameter
/// stopped compiling, reporting `call` "expects `&'T`, found `[ -- i64 ]`".
/// The fixture here only needs the declaration, since the rejection is what
/// keeps that body reachable.
///
/// `call` needs its own gate: it is not in `BUILTIN_WORDS`, so P7.S3r (R4)'s
/// `is_name_dispatched_builtin` rejection does not cover it, and it cannot be
/// added to that set without also making bare `call` require an `intrinsics`
/// import (P8 S2 R2).
#[test]
fn trait_member_named_call_is_rejected() {
    let (_t, entry) = single_file(
        "call-named-member",
        "trait: C['T] : call ( &'T -- i64 ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "trait `C` declares a member named `call`, which is a builtin word (line 2, col 16)"
        ),
        "{err}"
    );
}

/// Same shape as `call` above: `slice` is its own arm in `poly_call_term`,
/// absent from `BUILTIN_WORDS`, so a member named `slice` would otherwise
/// capture every `&[..] slice` in a bounded body.
#[test]
fn trait_member_named_slice_is_rejected() {
    let (_t, entry) = single_file(
        "slice-named-member",
        "trait: C['T] : slice ( &'T -- i64 ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "trait `C` declares a member named `slice`, which is a builtin word (line 2, col 16)"
        ),
        "{err}"
    );
}

/// Same shape as `call` above: `subslice` is its own arm in `poly_call_term`,
/// absent from `BUILTIN_WORDS`, so a member named `subslice` would otherwise
/// capture every `&[..] .. .. subslice` in a bounded body.
#[test]
fn trait_member_named_subslice_is_rejected() {
    let (_t, entry) = single_file(
        "subslice-named-member",
        "trait: C['T] : subslice ( &'T -- i64 ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "trait `C` declares a member named `subslice`, which is a builtin word (line 2, col 16)"
        ),
        "{err}"
    );
}

/// The negative half of the gate above: the six surface comparisons stay legal
/// member names, and a bound really does claim one here (they are `lib/` words,
/// not builtin arms, and a body that imports one receives it mangled, so the
/// spellings never collide). Without this, widening the `call` rejection to the
/// whole comparison set would go unnoticed.
#[test]
fn trait_member_named_after_a_surface_comparison_dispatches() {
    let (_t, entry) = single_file(
        "eq-named-member",
        "type: Tag v i64 ;\n\
         trait: C['T] : eq ( &'T -- i64 ) ; ;\n\
         impl: C for Tag\n\
           : eq | t | t &v @ ;\n\
         ;\n\
         : uses ['T: C] ( &'T -- i64 ) eq ;\n\
         : main ( -- ) 5 Tag |t| &t uses . t drop ;\n",
    );
    assert_eq!(build_and_run(&entry), "5\n");
}

/// The counterexample a post-implementation review built to demonstrate silent
/// mis-dispatch: this program used to build and print `900` (the unrelated
/// concrete `at` word) instead of dispatching to the trait member. Lifting the
/// declaration gate does not bring that back. A real build resolves names
/// before checking, so the concrete `at` this module declares captures the
/// call site's spelling (S3e R18: the trait loses a collision with a word of
/// the member's name, deliberately) -- and the concrete word cannot consume
/// the receiver, so the mis-dispatch is a located rejection rather than a
/// wrong answer. Dispatch winning over an *unresolved* same-named word is
/// pinned in `check::poly`'s own tests.
#[test]
fn a_concrete_word_of_the_members_name_captures_the_call() {
    let (_t, entry) = single_file(
        "member-name-collision",
        "type: Point x i64 y i64 ;\n\
         trait: Indexable['T] : at ( &'T i64 -- i64 ) ; ;\n\
         impl: Indexable for Point\n\
           : at | p n | n drop p &x @ ;\n\
         ;\n\
         : at ( i64 -- i64 ) 900 add ;\n\
         : uses ['T: Indexable] ( &'T -- i64 ) 0 at ;\n\
         : main ( -- ) 7 2 Point |p| &p uses . p drop ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("body leaves `&'T i64`") && err.contains("`uses`"),
        "{err}"
    );
}

/// A trait not exported cannot be named from another module -- the same
/// visibility rule `type:` already enforces.
#[test]
fn trait_requires_export_to_cross_a_module_boundary() {
    let t = Tree::new("no-export");
    t.write("lib.sth", "trait: Show['T] : show ( &'T -- ) ; ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"lib.sth\" l ;\n\
         impl: l::Show for i64\n\
           : show | p | p drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("not exported"), "{err}");
}

/// A trait declared and exported in one module resolves through a selective
/// import in another, and an `impl:` there (for a type declared in the
/// importer's own module, satisfying the orphan rule via the target type's
/// side) binds cleanly.
#[test]
fn trait_export_and_selective_import_resolve_across_modules() {
    let t = Tree::new("cross-module-ok");
    t.write(
        "lib.sth",
        "trait: Show['T] : show ( &'T -- ) ; ;\nexport: Show ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"lib.sth\" l | Show | ;\n\
         type: Point x i64 y i64 ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    build_ok(&entry);
}

/// A locally declared trait colliding with a selectively imported one of the
/// same name is caught (`local_decl_names`'s new `traits` loop).
#[test]
fn local_trait_collides_with_a_selectively_imported_one() {
    let t = Tree::new("local-vs-selective");
    t.write(
        "lib.sth",
        "trait: Show['T] : show ( &'T -- ) ; ;\nexport: Show ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"lib.sth\" l | Show | ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("collides with a local definition of `Show`"),
        "{err}"
    );
}

/// An `impl:` block validates and builds when its member body's own effect
/// matches the trait's declared signature.
#[test]
fn impl_body_validates_and_builds() {
    let (_t, entry) = single_file(
        "impl-ok",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for i64\n\
           : show | p | p drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    build_ok(&entry);
}

/// A missing required member is a located rejection naming it.
#[test]
fn impl_missing_a_required_member_is_rejected() {
    let (_t, entry) = single_file(
        "impl-missing-member",
        "trait: Eq['T] : eq ( &'T &'T -- ) ; : hash ( &'T -- ) ; ;\n\
         impl: Eq for i64\n\
           : eq | a b | a drop b drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("does not bind required member `hash`"),
        "{err}"
    );
}

/// Two `impl:` blocks for the same `(Trait, Type)` pair are a duplicate.
#[test]
fn duplicate_impl_for_the_same_trait_and_type_is_rejected() {
    let (_t, entry) = single_file(
        "impl-duplicate",
        "trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for i64\n\
           : show | p | p drop ;\n\
         ;\n\
         impl: Show for i64\n\
           : show | p | p drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("duplicate `impl:` for `i64`"), "{err}");
}

/// The orphan rule: an `impl:` living in neither the trait's module nor the
/// target type's module is rejected.
#[test]
fn impl_outside_trait_and_type_module_is_an_orphan_rejection() {
    let t = Tree::new("orphan");
    t.write(
        "trait.sth",
        "trait: Show['T] : show ( &'T -- ) ; ;\nexport: Show ;\n",
    );
    t.write("point.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"trait.sth\" t | Show | ;\n\
         import: \"point.sth\" p | Point | ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("must live in the module declaring `Show` or the module declaring `Point`"),
        "{err}"
    );
}

/// The orphan rule's other half: an `impl:` living in the target type's own
/// declaring module (not the trait's) is legal.
#[test]
fn impl_inside_the_target_types_own_module_satisfies_the_orphan_rule() {
    let t = Tree::new("orphan-ok");
    t.write(
        "trait.sth",
        "trait: Show['T] : show ( &'T -- ) ; ;\nexport: Show ;\n",
    );
    t.write(
        "point.sth",
        "import: intrinsics * ;\n\
         import: \"trait.sth\" t | Show | ;\n\
         type: Point x i64 y i64 ;\n\
         export: Point ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"point.sth\" p | Point | ;\n: main ( -- ) ;\n",
    );
    build_ok(&entry);
}

/// R12/decision 5: composing two traits that require the same member name is
/// legal to declare; the unqualified call to that member is the rejection.
#[test]
fn ambiguous_unqualified_member_call_is_rejected() {
    let (_t, entry) = single_file(
        "ambiguous-member",
        "trait: A['T] : t1 ( &'T -- ) ; ;\n\
         trait: B['T] : t1 ( &'T -- ) ; ;\n\
         : f ['T: A B] ( &'T -- ) t1 ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("error: `t1` is required by both `A` and `B` on 'T (line 4, col 26)"),
        "{err}"
    );
    assert!(
        err.contains(
            "note: a member required by two of a variable's bounds cannot be called unqualified"
        ),
        "{err}"
    );
}

/// R9/R17 scope cut (tracked as P7.S3o): a polymorphic combinator's body is
/// checked standalone and its instantiation records never reach
/// `Module::instantiations`, so a user bound on its own type variable has
/// nowhere to resolve against -- an explicit, located rejection rather than a
/// dispatch against records that do not survive.
///
/// P7.S3o Phase 3: a bare member call (`show`) inside a bounded inline
/// combinator now resolves at the splice site via dispatch injection into
/// `check_terms_relaxed`. The standalone check (i64 stand-in) accounts for
/// the member's stack effect without requiring an `impl: Show for i64`, and
/// the actual dispatch happens at each real splice site where θ is concrete.
/// This test now compiles successfully — the gate rejection is gone, and the
/// bare member is no longer an "unknown word".
#[test]
fn a_user_bound_on_a_poly_combinator_compiles() {
    let (_t, entry) = single_file(
        "combinator-bound",
        "type: Point x i64 y i64 ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n\
         : shows inline ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- ) ;\n",
    );
    build_ok(&entry);
}

/// R18(a): a bound naming a qualified trait whose qualifier is not one of this
/// module's import aliases. `parse_capabilities` has no `resolve_type` to
/// delegate to, so this needs its own located rejection rather than the
/// generic unknown-capability message.
#[test]
fn an_unbound_qualifier_in_a_bound_is_rejected() {
    let (_t, entry) = single_file(
        "bound-unbound-qualifier",
        ": shows ['T: q::Show] ( &'T -- ) drop ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: unknown module qualifier `q` in bound `q::Show` at line 2, col 14 (a qualified bound names an import alias)"
        ),
        "{err}"
    );
}

/// R8: a bound instantiated at a concrete type with no satisfying `impl:` is
/// a located rejection naming the trait, the type, and the member signature
/// the missing impl would have to provide.
#[test]
fn a_bound_unsatisfied_at_the_call_site_is_rejected() {
    let (_t, entry) = single_file(
        "bound-unsatisfied",
        "type: Point x i64 y i64 ;\n\
         type: Blip n i64 ;\n\
         trait: Show['T] : show ( &'T -- ) ; ;\n\
         impl: Show for Point\n\
           : show | p | p drop ;\n\
         ;\n\
         : shows ['T: Show] ( &'T -- ) show ;\n\
         : main ( -- ) 1 Blip |b| &b shows b drop ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: cannot instantiate `'T` of `shows` with `Blip` in `main` (line 9, col 29)"
        ),
        "{err}"
    );
    assert!(
        err.contains("`Blip` does not satisfy `Show`: no `( &Blip -- )` found"),
        "{err}"
    );
}

/// Phase 4 (R9/R13/R16): the array `sort` consumer -- the slice's forcing
/// consumer -- run at two distinct concrete instantiations of `'T: Copy
/// Order`. `sort3` is a fully unrolled 3-element compare-swap network rather
/// than the spec's `slice3-dogfood.md` Program 2 insertion sort, because that
/// program's two nested loops are not expressible in a generic body today:
/// `while` on a quotation literal is rejected outright there, and a combinator
/// cannot carry a user bound at all (P7.S3o's scope cut). (An inner loop
/// written as its own generic word was blocked too, by the
/// generic-calls-generic gap; P7.S3k has since closed that one, but it carries
/// a user bound here, which a cross-call does not yet discharge.)
/// Self-recursion alone does work, so a single fused loop is
/// reachable; the unrolled network is the direct form of the same subject.
/// Each comparison dispatches through the bound to its own concrete
/// `impl:`'s `cmp`. `Pair`'s `impl:` orders *descending*, and
/// deliberately so: `Pair` is a one-`i64` struct, so an ascending
/// `cmp-pair` would be behaviourally identical to `cmp-i64` on either
/// receiver's layout and this golden would pass even if both
/// instantiations lowered the same resolution. Reversed, the two
/// instantiations are distinguishable in the output, which is what proves
/// the *per-instantiation* `CallInst::trait_calls` map reached the lowered
/// call site (R9).
#[test]
fn the_array_sort_consumer_runs_at_two_concrete_instantiations() {
    let t = tree_with_core("sort");
    let entry = t.write(
        "main.sth",
"import: intrinsics * ;\nimport: core::prelude | if lt gt | ;\nimport: core::bool | Bool | ;\ntype: Rank | Under | Same | Over ;\ntrait: Order['T] : cmp ( &'T &'T -- Rank ) ; ;\ntype: Pair n i64 ;\nimpl: Order for i64\n  : cmp\n    | b | | a |\n    a @ b @ lt ~[ Under ] ~[\n      a @ b @ gt ~[ Over ] ~[ Same ] if\n    ] if ;\n;\nimpl: Order for Pair\n  : cmp\n    | b | | a |\n    a &n @ | an | b &n @ | bn |\n    an bn lt ~[ Over ] ~[\n      an bn gt ~[ Under ] ~[ Same ] if\n    ] if ;\n;\n: sort3['T: Copy Order] ( array['T 3] -- array['T 3] )\n  | a0 |\n  &a0 0 &> &a0 1 &> cmp\n  ~[ ( Under ) drop a0 ]\n  ~[ ( Same ) drop a0 ]\n  ~[ ( Over )\n     drop\n     &a0 0 &> @ | x0 |\n     &a0 1 &> @ | y0 |\n     &!a0 0 &!> y0 !\n     &!a0 1 &!> x0 !\n     a0\n  ]\n  Rank? | a1 |\n  &a1 1 &> &a1 2 &> cmp\n  ~[ ( Under ) drop a1 ]\n  ~[ ( Same ) drop a1 ]\n  ~[ ( Over )\n     drop\n     &a1 1 &> @ | x1 |\n     &a1 2 &> @ | y1 |\n     &!a1 1 &!> y1 !\n     &!a1 2 &!> x1 !\n     a1\n  ]\n  Rank? | a2 |\n  &a2 0 &> &a2 1 &> cmp\n  ~[ ( Under ) drop a2 ]\n  ~[ ( Same ) drop a2 ]\n  ~[ ( Over )\n     drop\n     &a2 0 &> @ | x2 |\n     &a2 1 &> @ | y2 |\n     &!a2 0 &!> y2 !\n     &!a2 1 &!> x2 !\n     a2\n  ]\n  Rank? ;\n: main ( -- )\n  0 3 fill |a|\n  &!a 0 &!> 3 !\n  &!a 1 &!> 1 !\n  &!a 2 &!> 2 !\n  a sort3 |sorted|\n  &sorted 0 &> @ .\n  &sorted 1 &> @ .\n  &sorted 2 &> @ .\n  sorted drop\n  0 Pair 3 fill |p|\n  &!p 0 &!> 3 Pair !\n  &!p 1 &!> 1 Pair !\n  &!p 2 &!> 2 Pair !\n  p sort3 |ps|\n  &ps 0 &> &n @ .\n  &ps 1 &> &n @ .\n  &ps 2 &> &n @ .\n  ps drop\n  ;\n"
    );
    let stdout = build_and_run(&entry);
    assert_eq!(stdout, "1\n2\n3\n3\n2\n1\n");
}

/// The binding form's counterpart to this golden bound a member's implementing
/// word to an operator-spelled name (`max`), reachable only through a bound.
/// The body form has no separate implementing word to name: the member itself
/// is simply inlined, so there is nothing left for `uncalled_operator_overloads`
/// to prune in the first place. R4's `trait_member_named_after_a_builtin_operator`
/// golden (P7.S3r phase 1) is what now covers the operator-spelled-name ground.
#[test]
fn an_impl_body_member_returning_a_field_builds_and_runs() {
    let (_t, entry) = single_file(
        "impl-body-field-getter",
"trait: Getter['T] : show ( &'T -- i64 ) ; ;\ntype: Pt n i64 ;\nimpl: Getter for Pt\n  : show | p | p &n @ ;\n;\n: getval ['T: Getter] ( &'T -- i64 ) show ;\n: main ( -- ) 7 Pt |p| &p getval . p drop ;\n"
    );
    let stdout = build_and_run(&entry);
    assert_eq!(stdout, "7\n");
}
