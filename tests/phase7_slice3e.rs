//! P7.S3e goldens: `trait:` declaration, `impl:` binding, export/import
//! parity with `type:`/`extern:`, and the body-side bound rejections.
//!
//! Driven through the real `sooth` binary, so a cross-module scenario
//! exercises the whole-closure trait pre-pass in `driver::assemble_module`.
//! Only *rejections* of bound consumption are golden here: a bound-directed
//! call that check-passes has no lowering yet (the resolved symbol is Phase
//! 3's `CallInst` field and Phase 4's plumbing), so its goldens land with
//! that work. The accepting side is pinned at check level meanwhile, in
//! `check::poly`'s and `driver`'s own tests.

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
        "trait: Show 'T show ( &'T -- ) ;\n\
         trait: Show 'T show ( &'T -- ) ;\n\
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
         trait: Point 'T foo ( &'T -- ) ;\n\
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
        "trait: Copy 'T foo ( &'T -- ) ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("already the name of a trait"), "{err}");
}

/// A trait not exported cannot be named from another module -- the same
/// visibility rule `type:` already enforces.
#[test]
fn trait_requires_export_to_cross_a_module_boundary() {
    let t = Tree::new("no-export");
    t.write("lib.sth", "trait: Show 'T show ( &'T -- ) ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"lib.sth\" l ;\n\
         : int-show ( &i64 -- ) drop ;\n\
         impl: l::Show for i64  show int-show ;\n\
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
        "trait: Show 'T show ( &'T -- ) ;\nexport: Show ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"lib.sth\" l | Show | ;\n\
         type: Point x i64 y i64 ;\n\
         : point-show ( &Point -- ) drop ;\n\
         impl: Show for Point  show point-show ;\n\
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
        "trait: Show 'T show ( &'T -- ) ;\nexport: Show ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: \"lib.sth\" l | Show | ;\n\
         trait: Show 'T show ( &'T -- ) ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("collides with a local definition of `Show`"),
        "{err}"
    );
}

/// `impl: Show for i64  show int-show ;` validates and registers when
/// `int-show` is concrete and its signature matches the trait's declared
/// member.
#[test]
fn impl_binding_validates_and_builds() {
    let (_t, entry) = single_file(
        "impl-ok",
        "trait: Show 'T show ( &'T -- ) ;\n\
         : int-show ( &i64 -- ) drop ;\n\
         impl: Show for i64  show int-show ;\n\
         : main ( -- ) ;\n",
    );
    build_ok(&entry);
}

/// An `impl:` binding a member to a polymorphic word is a located rejection
/// at the `impl:` site (decision 2), not a later call-site error.
#[test]
fn impl_binding_a_polymorphic_word_is_rejected_at_the_impl_site() {
    let (_t, entry) = single_file(
        "impl-poly-member",
        "trait: Show 'T show ( &'T -- ) ;\n\
         : poly-show ( 'U: Copy &'U -- ) drop ;\n\
         impl: Show for i64  show poly-show ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("polymorphic"), "{err}");
    assert!(err.contains("poly-show"), "{err}");
}

/// A missing required member is a located rejection naming it.
#[test]
fn impl_binding_missing_a_required_member_is_rejected() {
    let (_t, entry) = single_file(
        "impl-missing-member",
        "trait: Eq 'T eq ( &'T &'T -- ) hash ( &'T -- ) ;\n\
         : int-eq ( &i64 &i64 -- ) drop drop ;\n\
         impl: Eq for i64  eq int-eq ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("does not bind required member `hash`"),
        "{err}"
    );
}

/// A binding to an unknown member name is rejected.
#[test]
fn impl_binding_an_unknown_member_is_rejected() {
    let (_t, entry) = single_file(
        "impl-unknown-member",
        "trait: Show 'T show ( &'T -- ) ;\n\
         : int-show ( &i64 -- ) drop ;\n\
         impl: Show for i64  show int-show  bogus int-show ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("is not a member of trait `Show`"), "{err}");
}

/// A binding whose signature does not match the trait's declared member
/// (grounded at the target type) is rejected.
#[test]
fn impl_binding_with_a_mismatched_signature_is_rejected() {
    let (_t, entry) = single_file(
        "impl-signature-mismatch",
        "trait: Show 'T show ( &'T -- i64 ) ;\n\
         : int-show ( &i64 -- ) drop ;\n\
         impl: Show for i64  show int-show ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(err.contains("does not match"), "{err}");
}

/// Two `impl:` blocks for the same `(Trait, Type)` pair are a duplicate.
#[test]
fn duplicate_impl_for_the_same_trait_and_type_is_rejected() {
    let (_t, entry) = single_file(
        "impl-duplicate",
        "trait: Show 'T show ( &'T -- ) ;\n\
         : int-show ( &i64 -- ) drop ;\n\
         : int-show2 ( &i64 -- ) drop ;\n\
         impl: Show for i64  show int-show ;\n\
         impl: Show for i64  show int-show2 ;\n\
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
        "trait: Show 'T show ( &'T -- ) ;\nexport: Show ;\n",
    );
    t.write("point.sth", "type: Point x i64 y i64 ;\nexport: Point ;\n");
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"trait.sth\" t | Show | ;\n\
         import: \"point.sth\" p | Point | ;\n\
         : point-show ( &Point -- ) drop ;\n\
         impl: Show for Point  show point-show ;\n\
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
        "trait: Show 'T show ( &'T -- ) ;\nexport: Show ;\n",
    );
    t.write(
        "point.sth",
        "import: intrinsics * ;\n\
         import: \"trait.sth\" t | Show | ;\n\
         type: Point x i64 y i64 ;\n\
         export: Point ;\n\
         : point-show ( &Point -- ) drop ;\n\
         impl: Show for Point  show point-show ;\n",
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
        "trait: A 'T t1 ( &'T -- ) ;\n\
         trait: B 'T t1 ( &'T -- ) ;\n\
         : f ( &'T: A B -- ) t1 ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("error: `t1` is required by both `A` and `B` on 'T (line 4, col 21)"),
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
#[test]
fn a_user_bound_on_a_poly_combinator_is_rejected() {
    let (_t, entry) = single_file(
        "combinator-bound",
        "type: Point x i64 y i64 ;\n\
         trait: Show 'T show ( &'T -- ) ;\n\
         : point-show ( &Point -- ) drop ;\n\
         impl: Show for Point  show point-show ;\n\
         : shows inline ( &'T: Show -- ) show ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: `'T: Show` on the combinator `shows` at line 6, col 3 is not supported"
        ),
        "{err}"
    );
}

/// R18(a): a bound naming a qualified trait whose qualifier is not one of this
/// module's import aliases. `parse_capabilities` has no `resolve_type` to
/// delegate to, so this needs its own located rejection rather than the
/// generic unknown-capability message.
#[test]
fn an_unbound_qualifier_in_a_bound_is_rejected() {
    let (_t, entry) = single_file(
        "bound-unbound-qualifier",
        ": shows ( &'T: q::Show -- ) drop ;\n: main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: unknown module qualifier `q` in bound `q::Show` at line 2, col 16 (a qualified bound names an import alias)"
        ),
        "{err}"
    );
}
