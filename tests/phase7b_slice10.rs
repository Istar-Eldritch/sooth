//! P7b.S10 goldens: header-level export ambiguity for the third-module bare
//! caller. A bare generic-ctor call in a module that declares no `Widget`
//! header of its own used to silently dispatch on whichever module happened
//! to spell the instantiation eagerly (S9's Residual); S10 replaces that
//! silent pick with a located compile-time error at the single-candidate
//! grounding fall-through (`bare_generated_word_own_module_grounding`'s
//! headerless arm, `foreign_single_candidate_grounding` in
//! `src/check/terms.rs`), scoped to the headers reachable through the
//! caller's own import set.
//!
//! Fixture texts are verbatim `slice10-paper-tests.md`; the error goldens
//! pin the measured rendered bytes (measure-then-pin). Styled after
//! `tests/phase7b_slice9.rs`'s `Tree` harness.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs10-{}-{tag}-{seq}", std::process::id()));
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

/// The build's stderr, asserting the located build failure is the exit-1
/// diagnostic path the error goldens pin.
fn build_error(entry: &PathBuf) -> String {
    let build = sooth_build(entry);
    assert_eq!(
        build.status.code(),
        Some(1),
        "build should fail with exit 1; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn write_manifest(t: &Tree) {
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs10 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
}

/// `f.sth`, shared by every fixture (`slice10-paper-tests.md`): the `Sized`
/// trait with its `size` member and the shared poly consumer `sized`.
fn write_sized_trait(t: &Tree) {
    t.write(
        "f.sth",
        "import: intrinsics * ;\n\
         trait: Sized['S] : size ( 'S -- i64 ) ; ;\n\
         : sized['S: Sized] ( 'S -- i64 ) size ;\n\
         export: Sized sized ;\n",
    );
}

/// A generic-header module: `type: Widget['T] v 'T ;` with its own
/// `impl: Sized for Widget` dispatching the given constant, then the body
/// lines and export list that make it the fixture's exact paper-test module.
fn write_widget_module(t: &Tree, file: &str, constant: &str, body: &str, export: &str) {
    t.write(
        file,
        &format!(
            "import: intrinsics * ; import: self::f * ;\n\
             type: Widget['T] v 'T ;\n\
             impl: Sized for Widget : size drop {constant} ; ;\n\
             {body}\
             {export}"
        ),
    );
}

/// The bare consumer word (`Widget sized`), no explicit instantiation
/// spelled: this module never eagerly mints.
const BARE_RUN_BODY: &str = ": run ( i64 -- i64 ) Widget sized ;\n";

/// The eager-minter shape (`usesize` spells `Widget[i64]` in its own
/// signature, a parse-time mint) plus its bare `run` consumer.
const EAGER_MINTER_BODY: &str = ": usesize ( Widget[i64] -- i64 ) size ;\n\
                                 : run ( i64 -- i64 ) Widget usesize ;\n";

/// `c.sth` of the central shape: imports both header modules, declares no
/// `Widget` header of its own, bare-calls `Widget` into `size`.
fn write_third_module_c(t: &Tree, first: &str, second: &str, selective: Option<&str>) {
    let import_line = match selective {
        Some(name) => format!("import: self::{name} | Widget | ;\n"),
        None => String::new(),
    };
    t.write(
        "c.sth",
        &format!(
            "import: intrinsics * ; import: self::f * ;\n\
             {import_line}\
             import: self::{first} ; import: self::{second} ;\n\
             : try ( i64 -- i64 ) Widget size ;\n\
             export: try ;\n"
        ),
    );
}

/// `main.sth` that prints `c::try` of 5.
fn write_main_c(t: &Tree) {
    t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::c ;\n\
         : main ( -- ) 5 c::try . ;\n",
    );
}

/// The measured ambiguity error (GA/GB; byte-exact across both import
/// orders and both minter placements -- REQ-5's determinism pin, R5's
/// measured contract). GK's tree places `try` one line lower (its selective
/// import is its own line), so its pinned text carries line 4; GP names the
/// wildcard-bound module structurally (R4).
const AMBIGUITY_ERROR: &str = "error: `Widget` in `try` (line 3, col 22) is ambiguous: declared in modules `a` and `b`, and `try`'s module declares no `Widget`\n  note: declare your own `Widget` header and impl, or selectively import the module whose `Widget` you want -- if that module does not itself instantiate `Widget[i64]`, also spell the type in your own word's signature (`import: self::a | Widget | ;` then `: mk ( i64 -- Widget[i64] ) Widget ;`)\n";

/// GA (`p1-a-b` / `p1-b-a`) -- this is S9's own G4
/// (`third_module_bare_caller_dispatches_the_single_shared_env_instantiation`,
/// retired in `tests/phase7b_slice9.rs`), inverted: the silent `2`/`1`
/// dispatch becomes a located compile-time ambiguity error naming `Widget`,
/// modules `a`/`b` (lexicographically, whichever import order -- R4), and
/// the call site; exit 1. Both modules declare a reachable `Widget` header,
/// only `b` eagerly mints, and `c` performs no explicit resolution, so no
/// exemption holds.
#[test]
fn third_module_bare_caller_with_ambiguous_headers_is_a_located_error() {
    for (first, second) in [("a", "b"), ("b", "a")] {
        let t = Tree::new(&format!("ga-{first}-{second}"));
        write_manifest(&t);
        write_sized_trait(&t);
        write_widget_module(&t, "a.sth", "1", BARE_RUN_BODY, "export: run ;\n");
        write_widget_module(&t, "b.sth", "2", EAGER_MINTER_BODY, "export: run ;\n");
        write_third_module_c(&t, first, second, None);
        write_main_c(&t);
        assert_eq!(
            build_error(&t.0.join("main.sth")),
            AMBIGUITY_ERROR,
            "import order {first}/{second} must not change the rendered module ordering (R4)"
        );
    }
}

/// GB (`p8-a-eager`) -- the minter-swap twin: now `a` is the sole eager
/// minter (`a`'s `usesize` spells the instantiation) and `b` is bare, so the
/// pre-S10 output flipped from `2` to `1` purely on minter placement. The
/// error must be byte-identical to GA's: the declaring modules are named
/// lexicographically, independent of which module minted (REQ-5).
#[test]
fn third_module_bare_caller_error_is_independent_of_the_eager_minter() {
    let t = Tree::new("gb-a-eager");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: run ;\n");
    write_widget_module(&t, "b.sth", "2", BARE_RUN_BODY, "export: run ;\n");
    write_third_module_c(&t, "a", "b", None);
    write_main_c(&t);
    assert_eq!(
        build_error(&t.0.join("main.sth")),
        AMBIGUITY_ERROR,
        "the minter placement must not decide the rendered text"
    );
}

/// GC (`p2-both-eager`) -- both modules spell `Widget[i64]` eagerly, so the
/// call reaches the pre-existing multi-candidate arm: the S5
/// `select_overload` ambiguity error, byte-identical (REQ-4; diagnostics are
/// behaviour -- S10 adds no module names to it and leaves it untouched).
#[test]
fn both_modules_eager_2_candidate_ambiguity_error_unchanged() {
    let t = Tree::new("gc-both-eager");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: run ;\n");
    write_widget_module(&t, "b.sth", "2", EAGER_MINTER_BODY, "export: run ;\n");
    write_third_module_c(&t, "a", "b", None);
    write_main_c(&t);
    assert_eq!(
        build_error(&t.0.join("main.sth")),
        "error: no overload of `Widget` in `try` (line 3) accepts these operands\n  candidate: `i64`\n  candidate: `i64`\n",
        "the existing 2-candidate error must stay byte-identical"
    );
}

/// GD (`p3a-single-lib-private`) -- one same-named header program-wide,
/// declared and minted by the module the caller imports: exemption 2 (<= 1
/// reachable header, declaring module reachable) licenses the existing
/// borrow. `lib.sth`'s `usesize` stays private, so the R18 export gate is
/// not tripped.
#[test]
fn single_declaring_header_bare_caller_still_resolves() {
    let t = Tree::new("gd-single-lib");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(
        &t,
        "lib.sth",
        "7",
        ": usesize ( Widget[i64] -- i64 ) size ;\n",
        "",
    );
    write_widget_module(&t, "app.sth", "", "", "");
    // app.sth is not a Widget module here: write it directly.
    t.write(
        "app.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::lib ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::app ;\n\
         : main ( -- ) 5 app::try . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "7\n",
        "one reachable header is not ambiguity"
    );
}

/// GE (`p5g2-selective-type`) -- `c`'s own `mk` spells `Widget[i64]` in its
/// own signature, so `c` itself is the *instantiating* module for that
/// mint: the call exits at the `owning_module == caller_module` check
/// (`terms.rs`) before any exemption is consulted. Pins that arm, not
/// exemption 2 (corrected mechanism attribution; REQ-4).
#[test]
fn single_reachable_header_with_selective_import_still_resolves() {
    let t = Tree::new("ge-selective-type");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: Widget ;\n");
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a | Widget | ;\n\
         : mk ( i64 -- Widget[i64] ) Widget ;\n\
         : try ( i64 -- i64 ) mk size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    let entry = t.0.join("main.sth");
    assert_eq!(
        build_and_run(&entry),
        "1\n",
        "c's own-signature mint makes c the instantiating module; the call exits at the foreign check"
    );
}

/// GF (`p7c3-qualified-type`) -- the same `:1504` mechanism as GE, via a
/// qualified signature: `c`'s own `try` spells `a::Widget[i64]` (declaring
/// module `a`, instantiating module `c`), so `owning_module ==
/// caller_module` and the call exits before any exemption runs.
#[test]
fn single_reachable_header_with_qualified_signature_still_resolves() {
    let t = Tree::new("gf-qualified-type");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: Widget ;\n");
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a ;\n\
         : try ( a::Widget[i64] -- i64 ) size ;\n\
         export: try ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::c ; import: self::a ;\n\
         : main ( -- ) 5 Widget c::try . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "1\n",
        "the qualified-signature mint is c's own"
    );
}

/// GG (`p4-c-annotates`) -- an annotated signature without any import
/// bringing the name into scope is already the type-position rule's error,
/// unchanged: S10 governs term-position bare ctor calls only.
#[test]
fn unimported_foreign_type_annotation_is_still_an_error() {
    let t = Tree::new("gg-c-annotates");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: run ;\n");
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a ;\n\
         : try ( Widget[i64] -- i64 ) size ;\n\
         export: try ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::c ; import: self::a ;\n\
         : main ( -- ) 5 a::run drop 5 c::try . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: unknown type `Widget` at line 3, col 9\n",
        "the type-position rule is unchanged"
    );
}

/// GH (`overfire`) -- two headers exist program-wide (`lib`, `z`), but
/// `app` imports only `lib`; `z`'s header is not reachable from `app`'s own
/// imports, so it does not count toward the >= 2 threshold even though it
/// exists in the program closure. This is the fixture that justifies
/// reachability-scoping the count (R1) rather than a program-wide one.
#[test]
fn unimported_declaring_module_does_not_count_toward_ambiguity() {
    let t = Tree::new("gh-overfire");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(
        &t,
        "lib.sth",
        "7",
        ": usesize ( Widget[i64] -- i64 ) size ;\n",
        "",
    );
    write_widget_module(&t, "z.sth", "9", BARE_RUN_BODY, "export: run ;\n");
    t.write(
        "app.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::lib ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::app ; import: self::z ;\n\
         : main ( -- ) 5 app::try . ;\n",
    );
    assert_eq!(
        build_and_run(&entry),
        "7\n",
        "the unimported second header must not flag the call ambiguous"
    );
}

/// GI (`sel1`) -- two reachable headers (`a`, `b`); only `a` ever eagerly
/// mints; `c` selectively imports `a`'s `Widget`, matching the sole
/// candidate's declaring module: exemption 4 fires, no error.
#[test]
fn matching_selective_import_of_the_sole_minter_still_resolves() {
    let t = Tree::new("gi-sel1");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: Widget ;\n");
    write_widget_module(&t, "b.sth", "2", "", "export: Widget ;\n");
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a | Widget | ;\n\
         import: self::b ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    let entry = t.0.join("main.sth");
    assert_eq!(
        build_and_run(&entry),
        "1\n",
        "the named selective import matches the sole minter: exempt"
    );
}

/// GJ (`p5i2-selective-one-of-two`) -- both modules mint (multi-candidate
/// arm); the existing S5 tier-2 pinning selects the selectively imported
/// `a`: exemption 3, untouched by S10.
#[test]
fn selective_import_pins_the_named_exporter_when_both_mint() {
    let t = Tree::new("gj-p5i2");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: Widget ;\n");
    write_widget_module(&t, "b.sth", "2", EAGER_MINTER_BODY, "export: Widget ;\n");
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a | Widget | ;\n\
         import: self::b ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    let entry = t.0.join("main.sth");
    assert_eq!(
        build_and_run(&entry),
        "1\n",
        "tier-2 pinning selects the selectively imported exporter, unchanged"
    );
}

/// GK (`p9-mismatched-selective`) -- the soundness-critical case: `c`
/// selectively imports `a`'s `Widget`, but `b` is the sole eager minter. A
/// blanket "a selective import exists => exempt" reading would keep the
/// silent mis-dispatch (`2`, contradicting the caller's own selection)
/// alive behind a selective import; the match requirement makes it the same
/// located ambiguity error as GA (its `try` sits one line lower, so line 4).
#[test]
fn mismatched_selective_import_is_still_a_located_error() {
    let t = Tree::new("gk-mismatched");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", BARE_RUN_BODY, "export: run Widget ;\n");
    write_widget_module(&t, "b.sth", "2", EAGER_MINTER_BODY, "export: run ;\n");
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a | Widget | ;\n\
         import: self::b ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    assert_eq!(
        build_error(&t.0.join("main.sth")),
        "error: `Widget` in `try` (line 4, col 22) is ambiguous: declared in modules `a` and `b`, and `try`'s module declares no `Widget`\n  note: declare your own `Widget` header and impl, or selectively import the module whose `Widget` you want -- if that module does not itself instantiate `Widget[i64]`, also spell the type in your own word's signature (`import: self::a | Widget | ;` then `: mk ( i64 -- Widget[i64] ) Widget ;`)\n",
        "the selected module (a) is not the sole candidate's declaring module (b): no exemption"
    );
}

/// GL (`hub`) -- `h` re-exports `a`'s `Widget` with no header of its own;
/// `c` selectively imports `Widget` from `h` (its raw `ModuleInfo.selective`
/// value is `h`, not `a`), plus plain imports of `a` and `b`; only `a`
/// mints. The exemption-4 match resolves through `h`'s re-export chain to
/// `a` before comparing (R2) -- a raw one-hop comparison would wrongly
/// error this correctly-resolving program.
#[test]
fn hub_reexported_selective_import_still_resolves() {
    let t = Tree::new("gl-hub");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: Widget ;\n");
    write_widget_module(&t, "b.sth", "2", "", "export: Widget ;\n");
    t.write(
        "h.sth",
        "import: intrinsics * ; import: self::a | Widget | ;\n\
         export: Widget ;\n",
    );
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::h | Widget | ;\n\
         import: self::a ; import: self::b ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    let entry = t.0.join("main.sth");
    assert_eq!(
        build_and_run(&entry),
        "1\n",
        "the hub-resolved match lands on the sole declaring module: exempt"
    );
}

/// GM (`under`) -- the soundness-critical twin of GH: `app` imports only
/// `lib`, which declares its own `Widget` header but never eagerly mints;
/// the sole existing instantiation belongs to `z`, a module `app` never
/// imports in any form. Exemption 2's second half fails (`z` is
/// unreachable), so the call is a located *reach failure* -- one candidate,
/// not an ambiguity -- and since `app` has no qualifier for `z` at all, the
/// message names it structurally (R2/R3, the `drop`-diagnostic precedent).
#[test]
fn unreachable_minter_bare_call_is_a_located_error() {
    let t = Tree::new("gm-under");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "lib.sth", "7", BARE_RUN_BODY, "export: run ;\n");
    write_widget_module(&t, "z.sth", "9", EAGER_MINTER_BODY, "export: run ;\n");
    t.write(
        "app.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::lib ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ; import: hosted::show | . | ;\n\
         import: self::app ; import: self::z ;\n\
         : main ( -- ) 5 app::try . ;\n",
    );
    assert_eq!(
        build_error(&entry),
        "error: `Widget` in `try` (line 3, col 22) is unresolved: the only `Widget[i64]` instantiation in scope is declared in a module `try`'s module does not import\n  note: import the module that declares the instantiation you want, or declare and instantiate your own `Widget` header\n",
        "one reachable header but an unreachable minter: the reach-failure shape, named structurally"
    );
}

/// GN (`hubq`) -- `c` plainly imports `h` (no selective clause at all); `h`
/// has no header of its own but re-exports `a`'s `Widget`. Reachability's
/// walk-extension resolves `h` to `a`, so `a` joins the reachable set: one
/// header program-wide, nothing to mis-dispatch to, exemption 2 holds.
#[test]
fn hub_reexport_reachable_through_plain_import_still_resolves() {
    let t = Tree::new("gn-hubq");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", EAGER_MINTER_BODY, "export: Widget ;\n");
    t.write(
        "h.sth",
        "import: intrinsics * ; import: self::a | Widget | ;\n\
         export: Widget ;\n",
    );
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::h ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    let entry = t.0.join("main.sth");
    assert_eq!(
        build_and_run(&entry),
        "1\n",
        "the walk-extension sees the header behind the hub: nothing to mis-dispatch to"
    );
}

/// GO (`selother`) -- `d` declares and mints `Widget` (and separately
/// `Gadget`); `c` selectively imports only `Gadget` -- a name *other than*
/// the surface name under check -- and bare-calls `Widget`. `d` is still
/// reachable: the raw reachable set is `imports` UNION `selective`
/// target-module values, name-independent (R1/GO), and `d` is also the sole
/// candidate's declaring module, so both halves of exemption 2 hold.
#[test]
fn selective_import_of_different_name_still_grants_reachability() {
    let t = Tree::new("go-selother");
    write_manifest(&t);
    write_sized_trait(&t);
    t.write(
        "d.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         type: Widget['T] v 'T ;\n\
         type: Gadget['T] g 'T ;\n\
         impl: Sized for Widget : size drop 4 ; ;\n\
         : usesize ( Widget[i64] -- i64 ) size ;\n\
         export: Widget Gadget ;\n",
    );
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::d | Gadget | ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    let entry = t.0.join("main.sth");
    assert_eq!(
        build_and_run(&entry),
        "4\n",
        "a selective import of a different name still makes its target reachable"
    );
}

/// GP (`wild`) -- the soundness-critical wildcard case: `a` and `b` both
/// declare `Widget`; only `b` mints; `c` plainly imports `a` and
/// wildcard-imports `b`, then bare-calls `Widget`. The wildcard's
/// per-export desugar puts `Widget -> b` into `c`'s `ModuleInfo.selective`
/// exactly as a named clause would, but it never explicitly resolved the
/// name, so exemption 4 does not fire (R2/GP); the two reachable headers
/// reach the general rule. `b` has no bound qualifier (a wildcard binds
/// none), so the message names it by the existing wildcard-import phrasing
/// (R4) while `a` is named by its qualifier.
#[test]
fn wildcard_import_does_not_exempt_the_ambiguity_check() {
    let t = Tree::new("gp-wild");
    write_manifest(&t);
    write_sized_trait(&t);
    write_widget_module(&t, "a.sth", "1", "", "export: Widget ;\n");
    write_widget_module(
        &t,
        "b.sth",
        "2",
        ": usesize ( Widget[i64] -- i64 ) size ;\n",
        "export: Widget ;\n",
    );
    t.write(
        "c.sth",
        "import: intrinsics * ; import: self::f * ;\n\
         import: self::a ; import: self::b * ;\n\
         : try ( i64 -- i64 ) Widget size ;\n\
         export: try ;\n",
    );
    write_main_c(&t);
    assert_eq!(
        build_error(&t.0.join("main.sth")),
        "error: `Widget` in `try` (line 3, col 22) is ambiguous: declared in modules `a` and its wildcard-imported module, and `try`'s module declares no `Widget`\n  note: declare your own `Widget` header and impl, or selectively import the module whose `Widget` you want -- if that module does not itself instantiate `Widget[i64]`, also spell the type in your own word's signature (`import: self::a | Widget | ;` then `: mk ( i64 -- Widget[i64] ) Widget ;`)\n",
        "a wildcard desugar never counts as explicit resolution; the wildcard-bound module is named structurally"
    );
}
