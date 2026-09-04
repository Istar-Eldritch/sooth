//! P7b.S9 goldens: the operand-provenance fix (R1.1a,
//! `bare_generated_word_own_module_grounding`, `src/check/terms.rs`) for a bare
//! generic-ctor call whose single `env` candidate is another module's
//! eager mint, and the monomorphization-identity fix (R2.1,
//! `instantiation_symbol`, `src/ast.rs`) for two groundings of one bound word
//! whose operands render the same name. Styled after
//! `tests/phase7b_slice5.rs`'s `Tree` harness.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs9-{}-{tag}-{seq}", std::process::id()));
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

fn sooth_build(entry: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

fn build_and_run(entry: &PathBuf) -> String {
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

fn build_error(entry: &PathBuf) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn write_manifest(t: &Tree) {
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs9 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
}

/// G1 (verbatim `pb2`, the Phase-1 diagnosis fixture): two modules each
/// declare their own `Widget['T]` header and their own `impl: Sized for
/// Widget`, but only `b` spells the concrete type explicitly (`usesize`'s
/// param), so pre-fix, `a`'s own `Widget[i64]` instantiation is never minted
/// at all -- both bare `Widget` ctor calls resolve through the single
/// existing (b's) candidate in `env["Widget"]`, and both dispatch to `b`'s
/// impl (`2\n2`). Post-fix (R1.1a), `a`'s bare `Widget` call grounds at its
/// own header (`bare_generated_word_own_module_grounding`), minting its own
/// instantiation, so `a::run` dispatches to `a`'s own impl.
///
/// `b::run` reaches `size` through a direct mono member call
/// (`usesize`'s own body), not through the shared poly word `sized` -- unlike
/// G2/G2r's `mk` shape, this never mints a second grounding of `sized`
/// itself, so it cannot trip V3's `instantiation_symbol` collision (Phase 3);
/// measured deterministic across 6 rebuild+run cycles.
#[test]
fn cross_module_same_shaped_impls_dispatch_each_callers_own_impl() {
    let t = Tree::new("g1-pb2");
    write_manifest(&t);
    t.write(
        "f.sth",
        "import: intrinsics * ;\n\
         trait: Sized['S] : size ( 'S -- i64 ) ; ;\n\
         : sized['S: Sized] ( 'S -- i64 ) size ;\n\
         export: Sized sized ;\n",
    );
    t.write(
        "a.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Sized for Widget : size drop 1 ; ;\n\
         : run ( i64 -- i64 ) Widget sized ;\n\
         export: run ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Sized for Widget : size drop 2 ; ;\n\
         : usesize ( Widget[i64] -- i64 ) size ;\n\
         : run ( i64 -- i64 ) Widget usesize ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::f * ; import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::run . 5 b::run . ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(
        out, "1\n2\n",
        "each caller must dispatch its own impl, not the single, borrowed, cross-module mint"
    );
}

/// The `f.sth` of G1, shared by the regression pins below.
fn write_sized_trait(t: &Tree) {
    t.write(
        "f.sth",
        "import: intrinsics * ;\n\
         trait: Sized['S] : size ( 'S -- i64 ) ; ;\n\
         : sized['S: Sized] ( 'S -- i64 ) size ;\n\
         export: Sized sized ;\n",
    );
}

/// `b.sth` of G1, with `fields` and `run`'s inputs varied: the eager minter,
/// the only module that spells `Widget[i64]` explicitly.
fn write_eager_minter_shaped(t: &Tree, fields: &str, run_inputs: &str) {
    t.write(
        "b.sth",
        &format!(
            "import: intrinsics * ; import: self::f * ;\n\
             type: Widget['T] {fields} ;\n\
             impl: Sized for Widget : size drop 2 ; ;\n\
             : usesize ( Widget[i64] -- i64 ) size ;\n\
             : run ( {run_inputs} -- i64 ) Widget usesize ;\n\
             export: run ;\n"
        ),
    );
}

/// `b.sth` of G1 at its own shape: one `'T` field, one `i64` operand.
fn write_eager_minter(t: &Tree) {
    write_eager_minter_shaped(t, "v 'T", "i64");
}

/// G1a (review round 1): *three* bare generated-word sites in one caller
/// module. `check` flushes the live generic cell around every word, so the
/// mint the first site made is no longer pending by the time the second site
/// grounds -- a grounding that could only read the unflushed tail would miss
/// it and fall back to `b`'s borrowed mint (REQ-2's silent cross-module
/// borrow). The third site destructures: the caller's own mint has no `env`
/// entry of its own, so `Widget>` must ground at the caller's own header too,
/// or it resolves to `b`'s generated word and fails with a
/// self-contradictory `expected Widget[i64], found Widget[i64]`.
///
/// `b`'s header carries a *second* field, so a borrowed grounding is
/// observable at every site: its constructor takes two operands where `a`'s
/// takes one, and each of `a`'s sites supplies one. `sized` still has a
/// single grounding (both `a` sites ground at `a`'s one mint, and `b` reaches
/// `size` through `usesize`'s own mono member call), so this cannot trip V3's
/// `instantiation_symbol` collision either; measured deterministic across 5
/// rebuild+run cycles.
#[test]
fn every_bare_ctor_site_in_one_module_grounds_at_the_callers_own_header() {
    let t = Tree::new("g1a-flush");
    write_manifest(&t);
    write_sized_trait(&t);
    write_eager_minter_shaped(&t, "v 'T w 'T", "i64 i64");
    t.write(
        "a.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Sized for Widget : size drop 1 ; ;\n\
         : run ( i64 -- i64 ) Widget sized ;\n\
         : two ( i64 -- i64 ) Widget sized ;\n\
         : three ( i64 -- i64 ) Widget Widget> ;\n\
         export: run two three ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::f * ; import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::run . 5 a::two . 7 a::three . 5 5 b::run . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "1\n1\n7\n2\n",
        "every site in a grounds at a's own header -- the second one too, after the first mint was flushed"
    );
}

/// G1b (review round 1): a field projection off a caller-grounded value. The
/// mint is made mid-word, so it is still pending in the live generic cell
/// while its own word is checked -- `check_field_projection` indexes the live
/// struct registry by id, which cannot see it.
#[test]
fn field_projection_reads_the_caller_grounded_mints_own_field() {
    let t = Tree::new("g1b-field");
    write_manifest(&t);
    write_sized_trait(&t);
    write_eager_minter(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Sized for Widget : size drop 1 ; ;\n\
         : run ( i64 -- i64 ) Widget | w | &w &v @ | x | w drop x ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::f * ; import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::run . 5 b::run . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "5\n2\n",
        "the projection reads the caller's own field, and b's own dispatch is unchanged"
    );
}

/// G1c (review round 1): the caller's own header cannot be applied to the
/// borrowed mint's argument list at all (`Widget['T 'U]` against `[i64]`).
/// Minting it there would index past the argument list and panic; borrowing
/// the other module's mint instead is the silent cross-pick REQ-2 forbids --
/// so the call site is reported.
#[test]
fn bare_ctor_arity_mismatch_with_the_callers_own_header_is_a_located_error() {
    let t = Tree::new("g1c-arity");
    write_manifest(&t);
    write_sized_trait(&t);
    write_eager_minter(&t);
    t.write(
        "a.sth",
        // The body sits on its own line, one past `run`'s header: the
        // reported line is the offending *call site*, which a one-line word
        // cannot tell from its declaration.
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T 'U] v 'T w 'U ;\n\
         impl: Sized for Widget : size drop 1 ; ;\n\
         : run ( i64 i64 -- i64 )\n\
         \x20 Widget sized ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::f * ; import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 7 a::run . 5 b::run . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: `Widget` in `run` (line 5) cannot ground at this module's own header: it declares 2 type parameters, but the only `Widget` instantiation in scope supplies 1\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's\n"
    );
}

/// G1d (review round 2): the kind twin of G1c. The counts agree (one type
/// parameter each), but the caller's own header binds its variable at `* ->
/// *` and applies it in a field (`v 'F[i64]`), while the borrowed mint
/// supplies a plain `i64`. `substitute_generic_field`'s `App` arm applies a
/// `CtorImage` binding and returns anything else *unapplied*, a fall-back its
/// own doc calls unreachable because the parser's `validate_ctor_arg_kinds`
/// rejects a kind mismatch at the use site -- but the use site here is in
/// another module, so grounding has to re-apply that rule or the field
/// silently gets the wrong type (pre-fix: exit 0, `Widget` taking a bare
/// `i64`).
#[test]
fn bare_ctor_kind_mismatch_with_the_callers_own_header_is_a_located_error() {
    let t = Tree::new("g1d-kind");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Box['T] val 'T ;\n\
         type: Widget['F] v 'F[i64] ;\n\
         : mk ( i64 -- )\n\
         \x20 Widget drop ;\n\
         export: mk ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : use ( Widget[i64] -- i64 ) Widget> ;\n\
         : run ( -- i64 ) 11 Widget use ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 7 a::mk b::run . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: `Widget` in `mk` (line 5) cannot ground at this module's own header: its `'F` takes a type constructor, but the only `Widget` instantiation in scope supplies the concrete type `i64`\n  note: name an instantiation of this module's own `Widget` explicitly (in a signature or an annotation) so it is minted here, rather than borrowing another module's\n"
    );
}

/// G1e (review round 2): the caller-grounded mint gives two *live*
/// `StructDecl`s one name (`type_instantiation_name` encodes no declaring
/// module), and the untouched module's own destructure of its own declared
/// `Widget[i64]` parameter must still lower with its own layout. Two things
/// were keyed module-blind: `ir/layout.rs`'s generated-word registry (`b`'s
/// `Widget>` site carries no resolved-symbol record -- its `env` candidate
/// was unique when `env` was built, before `a` minted -- so it resolves by
/// the bare surface name, last-write-wins) and the emitted QBE type symbol
/// (`type :Widget[i64]` defined twice, the second silently redefining the
/// first). Pre-fix this panicked in lowering (`a word with a declared output
/// leaves one`); the same program with `a`'s mint spelled explicitly instead
/// of grounded has always printed `11`.
#[test]
fn same_named_headers_of_differing_shapes_destructure_each_modules_own_layout() {
    let t = Tree::new("g1e-collide");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk3 ( i64 -- ) Widget drop ;\n\
         export: mk3 ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T w i64 ;\n\
         : mk2 ( Widget[i64] -- i64 ) Widget> drop ;\n\
         : run ( -- i64 ) 11 99 Widget mk2 ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 7 a::mk3 b::run . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "11\n",
        "b's two-field destructure must read b's own layout, not a's one-field mint of the same name"
    );
}

/// G1f (review round 2): G1e's multi-output twin. `b`'s destructure feeds a
/// two-output word, so its result packs through a return bundle -- the shape
/// that read `a`'s one-field layout for `b`'s two-field struct and underflowed
/// (`pack_bundle`, `attempt to subtract with overflow`). Both field values are
/// printed, so a wrong layout is visible in the output and not only in a
/// crash: the same program spelling `a`'s mint explicitly prints `99` then
/// `11`.
#[test]
fn same_named_headers_of_differing_shapes_pack_each_modules_own_field_values() {
    let t = Tree::new("g1f-bundle");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk3 ( i64 -- ) Widget drop ;\n\
         export: mk3 ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T w i64 ;\n\
         : mk2 ( Widget[i64] -- i64 i64 ) Widget> ;\n\
         : run ( -- i64 i64 ) 11 99 Widget mk2 ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::a ; import: self::b ;\n\
         : main ( -- ) 7 a::mk3 b::run . . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "99\n11\n",
        "both of b's own field values survive the bundle, in b's own field order"
    );
}

/// G2 (Phase 3, R2.1/REQ-3): both modules name their own `Widget[i64]`
/// explicitly through their own `mk`, so provenance is never in question --
/// each caller already holds its own grounding. What collapsed them is one
/// stage later: the shared bound word `sized` grounds twice, and
/// `instantiation_symbol` rendered both groundings' operands by name
/// (`Widget_i64_`), so lowering's instantiation dedup kept whichever
/// `CallInst` its randomized `HashMap` iteration reached first and discarded
/// the other whole -- `trait_calls` map included. Both callers then ran the
/// surviving grounding's `size`, printing `1\n1` or `2\n2` by build seed
/// (never asserted: R-NFR3). Post-fix each grounding renders its own
/// `StructId` and keeps its own specialization.
///
/// Same shape as `tests/phase7b_slice4.rs`'s
/// `cross_module_same_shaped_impls_each_dispatch_their_own_impl` (a different
/// trait and text, its own fixture -- neither is churned to match the other).
#[test]
fn cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl() {
    let t = Tree::new("g2-mk");
    write_manifest(&t);
    write_sized_trait(&t);
    for (module, constant) in [("a", "1"), ("b", "2")] {
        t.write(
            &format!("{module}.sth"),
            &format!(
                "import: intrinsics * ; import: self::f * ;\n\
                 type: Widget['T] v 'T ;\n\
                 impl: Sized for Widget : size drop {constant} ; ;\n\
                 : mk ( i64 -- Widget[i64] ) Widget ;\n\
                 : run ( i64 -- i64 ) mk sized ;\n\
                 export: run ;\n"
            ),
        );
    }
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::f * ; import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::run . 6 b::run . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "1\n2\n",
        "two groundings of one bound word must keep their own specializations"
    );
}

/// G2r (Phase 3, R2.1): G1's provenance mirror -- `a` is the eager minter
/// (its `mk` spells `Widget[i64]`) and `b`'s consumer stays bare -- so it
/// pins that the first eager mint does not win regardless of caller. Both
/// halves of the slice are needed to see `1\n2`: pre-Phase-2 it printed a
/// deterministic `1\n1` (b borrowed a's mint), and with the provenance fix
/// alone it went nondeterministic, because unlike G1 *both* of its callers
/// reach `size` through the shared `sized`, which is exactly G2's symbol
/// collision.
#[test]
fn cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller() {
    let t = Tree::new("g2r-eager");
    write_manifest(&t);
    write_sized_trait(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Sized for Widget : size drop 1 ; ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : run ( i64 -- i64 ) mk sized ;\n\
         export: run ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         impl: Sized for Widget : size drop 2 ; ;\n\
         : run ( i64 -- i64 ) Widget sized ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::f * ; import: self::a ; import: self::b ;\n\
         : main ( -- ) 5 a::run . 5 b::run . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "1\n2\n",
        "b's bare ctor grounds at b's own header, and its own grounding of sized survives lowering"
    );
}
