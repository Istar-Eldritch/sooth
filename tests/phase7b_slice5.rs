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
///
/// Not a placebo (review round 1 fix): a same-payload `i64`/`i64` roundtrip
/// through a single-field struct produces the identical printed value
/// regardless of which module's `Widget[i64]` wins -- deleting tier 2's
/// visibility filter entirely (`OverloadPick::Pick(matching[0])`) left the
/// original fixture's assertion passing unchanged. `a`'s own
/// `pin['T] ( Widget['T] -- Widget['T] )` closes that: a variable-headed
/// generic type names nothing concrete, so exporting it hits no
/// export-privacy rule, and its parameter is headed on `a`'s own
/// `GenericStructDecl` specifically, fixed at `a`'s own declaration site --
/// unambiguous regardless of how a third module's own local type-name
/// resolution behaves. Verified: swapping tier 2's pick for the *other*
/// (`matching[matching.len() - 1]`, forcing `b`'s candidate) turns this
/// fixture from a passing build into `` `pin` expected `Widget['T]`, found
/// `Widget[i64]` `` -- a genuine type-identity mismatch between two
/// identically-shaped, identically-named, differently-declared structs.
#[test]
fn imported_generic_ctor_resolves_single_visible_candidate() {
    let t = Tree::new("s5-tier2-selective-import");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : pin['T] ( Widget['T] -- Widget['T] ) ;\n\
         export: Widget pin ;\n",
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
         : run ( i64 -- i64 ) Widget a::pin Widget> 30 add ;\n\
         export: run ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n\
         import: self::c ;\n\
         : main ( -- ) 5 c::run . ;\n",
    );
    let out = build_and_run(&entry);
    // `c` imports only `a`'s `Widget` bare, so its bare `Widget` call must
    // construct *`a`'s own* struct -- routing it through `a::pin` (headed on
    // `a`'s own `GenericStructDecl`) before destructuring makes a wrong pick
    // a type mismatch, not merely the same printed value.
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
/// is always present in it.
///
/// Post-merge correction (P7b.S4 landed on `main` after this fixture was
/// written): this exact shape -- a mono caller with no bound, in a third
/// module, calling a generic-target trait member whose only impl lives in a
/// second module -- used to be intercepted upstream by
/// `mono_member_no_dispatch_error`, because `find_bound_impl` required the
/// caller's concrete instantiation and the impl's target pattern to resolve
/// to the same registry identity, which the cross-module generic-
/// instantiation gap prevented. P7b.S4 fixed that gap (instantiations now key
/// on the header's declaring module everywhere), so this shape now dispatches
/// for real -- `main`'s own `Box[i64]` and `a`'s impl target agree on the
/// same identity, and `usesize` correctly reads back `7`. This is a genuine
/// improvement, not a regression; the fixture is kept as a *positive*
/// dispatch golden instead of a `mono_member_no_dispatch_error` witness. The
/// dead-guard verdict above is unaffected: it never depended on which
/// programs reach `find_bound_impl` successfully, only on `poly_env` being
/// whole-program once one is found.
#[test]
fn cross_module_generic_target_member_dispatches_since_p7b_s4() {
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
    let out = build_and_run(&entry);
    assert_eq!(out, "7\n");
}

/// P7b.S5 Phase 3 (R5), real witness (post-merge addition): a mono caller
/// invoking a trait member where genuinely no `impl:` exists anywhere in the
/// program for the operand's type. Unlike the fixture above (which P7b.S4's
/// landing turned into a real dispatch), this shape has no impl to find at
/// all, so it still reaches `mono_member_no_dispatch_error` for real.
#[test]
fn mono_call_with_no_impl_anywhere_is_no_dispatch_error() {
    let t = Tree::new("s5-p3-no-impl-anywhere");
    write_manifest(&t);
    t.write(
        "f.sth",
        "import: intrinsics * ;\n\
         trait: Sized['S] : size ( 'S -- i64 ) ; ;\n\
         export: Sized ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\nimport: hosted::show | . | ;\n\
         import: self::f * ;\n\
         type: Box['T] v 'T ;\n\
         : usesize ( Box[i64] -- i64 ) size ;\n\
         : main ( -- ) 3 Box usesize . ;\n",
    );
    let err = build_error(&entry);
    assert!(
        err.contains("is a trait member of Sized, but no `impl:` in this program dispatches on these operands"),
        "expected mono_member_no_dispatch_error, got: {err}"
    );
}

/// P7b.S5 Phase 2b review-round-1 fix: `poly_call_term`'s own tier-arm
/// (`src/check/poly.rs`, the `env.get(name).and_then` match beside
/// `poly_construct_generic`) was flagged as possibly dead -- three mutation
/// probes (replace the pick with `matching.first()`, panic on
/// `matching.len() > 1`, `eprintln!` in the multi-candidate arm) all left
/// the suite green. Reproduced: true for every *other* existing fixture, but
/// this one reaches it for real. `a`'s `run['S]` is itself generic (an
/// unused `'S` type variable is enough to route its body through
/// `poly_call_term` rather than `check_term`), and its bare `Widget` call
/// operates on an already-concrete `i64` -- `poly_env_exact_match` finds
/// `a`'s own pre-minted `Widget[i64]` (from `mk`) and bails
/// `poly_construct_generic`, landing on the tier arm with `b`'s
/// independently-minted `Widget[i64]` also in `matching` (verified via a
/// temporary `eprintln!`: `matching_len=2`). Discriminating, not a placebo:
/// `pin`'s declared input `Widget[i64]` names *`a`'s own* struct header, so
/// a wrong pick (verified by temporarily swapping the tier pick for
/// `matching.last()`) turns this into `` `pin` is not permitted on
/// `Widget[i64]` `` -- a type mismatch against the wrong module's otherwise
/// identically-shaped and identically-named `Widget[i64]`.
#[test]
fn poly_body_tier_arm_resolves_same_shaped_ctor_to_callers_own_module() {
    let t = Tree::new("s5-poly-tier-arm");
    write_manifest(&t);
    t.write(
        "a.sth",
        "import: intrinsics * ;\n\
         type: Widget['T] v 'T ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : pin ( Widget[i64] -- i64 ) Widget> ;\n\
         : run['S] ( i64 'S -- i64 'S )\n\
         \x20  swap Widget pin swap\n\
         ;\n\
         export: Widget run ;\n",
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
         : main ( -- ) 42 7 a::run drop . ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(out, "42\n");
}
