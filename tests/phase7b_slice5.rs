//! P7b.S5 Phase 2b exit goldens: the tier policy that disambiguates a
//! same-shaped cross-module generic ctor collision at `terms.rs:956`
//! (`select_overload`, R3.5-R3.8). Styled after `tests/phase7b_slice2.rs`'s
//! multi-module `Tree` harness -- golden #10 there
//! (`same_named_ctors_in_two_modules_dispatch_distinct_impls`) is retained
//! unchanged as a payload-split regression witness (R6); these goldens are
//! same-*payload* (`i64`/`i64`), the shape #10's split never exercised.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs5-{}-{tag}-{seq}", std::process::id()));
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
            "package: p7bs5 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
}

/// R3.5's headline tier-1 golden (the `pb2` shape, module-only variant): two
/// modules each declare their own same-shaped, same-payload (`i64`) generic
/// ctor, and each module's own `run` constructs its own instance (via a
/// `mk` helper naming the instantiation, so *both* mint at parse time and
/// both land in the whole-program `env["Widget"]` -- otherwise only whichever
/// module explicitly names `Widget[i64]` ever mints it, and the other's bare
/// ctor call never reaches the multi-candidate collision at all; this is
/// what actually makes the collision reproducible for a discriminating
/// golden) then reads its own field back through the paired generated
/// destructure (`Widget>`, the same `struct_generated_sigs` provenance and
/// collision shape as the ctor) and combines it with a module-specific
/// constant. Deliberately no trait/impl dispatch here: this isolates
/// `select_overload`'s own fix (both `Widget` and `Widget>` collide under
/// Phase 2a's shared multi-candidate arm) from bound-impl-target resolution,
/// a different subsystem this slice does not touch. Before the fix,
/// `terms.rs:956`'s blind first-match made `a::run`/`b::run` both resolve to
/// the module-assembly-order-first candidate; the fix disambiguates on the
/// caller's own module, so each reads back its own field.
#[test]
fn cross_module_same_shaped_ctor_dispatches_callers_own_impl() {
    let t = Tree::new("s5-tier1-pb2");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : run ( i64 -- i64 ) mk Widget> 10 add ;\n\
         export: run ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : run ( i64 -- i64 ) mk Widget> 20 add ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n\
         import: self::a ;\nimport: self::b ;\n\
         : main ( -- ) 5 a::run . 5 b::run . ;\n",
    );
    let out = build_and_run(&entry);
    // `a::run` must read back through its own `Widget` (5 + 10 = 15), and
    // `b::run` through its own (5 + 20 = 25) -- not the module-assembly-
    // order-first candidate for either.
    assert_eq!(out, "15\n25\n");
}

/// R3.5/Fix F's tier-2 golden. Reusing
/// `selectively_imported_generic_name_applies_bare` was rejected (only one
/// declaring module -- a single-candidate fast path never reaches
/// `select_overload`). This fixture is TWO modules (`a`/`b`) each declaring
/// the same-shaped `Widget[i64]` ctor through their own `mk` (as tier-1's,
/// so both mint at parse time and both land in `env["Widget"]`/
/// `env["Widget>"]`), plus a THIRD module `c` that selectively imports only
/// `a`'s `Widget` (`import: a | Widget | ;`, the form that populates
/// `.selective`, which `is_name_visible_to_module` reads) and calls the
/// bare, *undeclared-instantiation* ctor directly -- deliberately no local
/// `mk` in `c`, so `c` mints no third candidate of its own and its call
/// reaches `select_overload` with exactly the 2 existing candidates in
/// `matching`. `c`'s own module is neither `a` nor `b`, so tier 1 misses for
/// both, and exactly one (`a`'s) is visible.
#[test]
fn imported_generic_ctor_resolves_single_visible_candidate() {
    let t = Tree::new("s5-tier2-selective-import");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         export: Widget ;\n",
    );
    t.write(
        "b.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         export: Widget ;\n",
    );
    t.write(
        "c.sth",
        "import: intrinsics * ;\n\
         import: self::a | Widget | ;\nimport: self::b ;\n\
         : run ( i64 -- i64 ) Widget Widget> 30 add ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n\
         import: self::c ;\n\
         : main ( -- ) 5 c::run . ;\n",
    );
    let out = build_and_run(&entry);
    // `c` imports only `a`'s `Widget` bare, so its `run` must construct and
    // read back through `a`'s `Widget` (5 + 30 = 35), never `b`'s -- proving
    // tier 2's visibility filter, not an accidental first-match pick.
    assert_eq!(out, "35\n");
}

/// R4's legitimate-overloading guard, recorded as a vacuity probe rather than
/// a positive golden: a same-*input*/different-*output* overload set within
/// one module is unwitnessable, because `check_duplicate_word_names`
/// (`src/check/declarations.rs`) keys its collision check on
/// `(module, name, input_types)` -- two ordinary words sharing a name and
/// input types in one module are rejected as a duplicate *before* any
/// `Overload` construction, regardless of their output types. There is no
/// route to a same-input, different-output, same-module `Overload` set for
/// `select_overload`'s tier policy to (mis)regress on. This probe fixes that
/// finding as a real assertion: the collision the guard would need is
/// rejected at declaration time, not admitted and then dispatched.
#[test]
fn same_input_different_output_overload_in_one_module_is_rejected_at_declaration() {
    let t = Tree::new("s5-legit-overload-vacuity");
    write_manifest(&t);
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         : mk ( i64 -- i64 ) ;\n\
         : mk ( i64 -- str ) ;\n\
         : main ( -- ) ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("duplicate word `mk`"),
        "expected the pre-existing duplicate-word rejection, got: {err}"
    );
}

/// P7b.S5 Phase 3 (R5): `mono_member_unroutable_error`'s non-inline call site
/// (`resolve_mono_member_call`'s generic-impl branch) is a dead guard --
/// `poly_env` is built once, whole-program, over the fully `assemble_module`-
/// flattened `Module` (`src/check.rs:668-706`), so a found impl's member word
/// is always present in it. Every cross-module attempt at a generic-target
/// member dispatch is intercepted upstream by `mono_member_no_dispatch_error`
/// instead, because `find_bound_impl` requires the caller's concrete
/// instantiation and the impl's target pattern to resolve to the same
/// registry identity, which the cross-module generic-instantiation gap
/// (`project_generic_instantiation_cannot_cross_modules`) prevents. This
/// fixture is the `pa2` probe shape: a mono caller with no bound, in a third
/// module, calling a generic-target trait member whose only impl lives in a
/// second module.
#[test]
fn cross_module_colliding_mono_call_is_no_dispatch_error() {
    let t = Tree::new("s5-p3-mono-unroutable");
    write_manifest(&t);
    t.write(
        "f.sth",
        "import: intrinsics * ;\n\
         trait: Sized['S] : size ( 'S -- i64 ) ; ;\n\
         export: Sized ;\n",
    );
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         import: self::f * ;\n\
         type: Box['T] v 'T ;\n\
         impl: Sized for Box['T]\n\
           : size drop 7 ;\n\
         ;\n\
         export: Box ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n\
         import: self::f * ;\nimport: self::a * ;\n\
         : usesize ( Box[i64] -- i64 ) size ;\n\
         : main ( -- ) 3 Box usesize . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("is a trait member of Sized, but no `impl:` in this program dispatches on these operands"),
        "expected mono_member_no_dispatch_error, got: {err}"
    );
}
