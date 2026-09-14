//! P7b.S15 exit goldens: the Applicative.pure declaration surface. Phase 1
//! starts the file with the three fully-measured goldens: the true-shape
//! member `pure ( 'A -- 'F['A] )` declares and its ctor-keyed impl checks
//! (G1, R-30.1), the `ap` parse fence holds (G9, R-30.5), and the zero-input
//! escape-hatch impl mismatch keeps its recorded-not-fixed bytes (G12,
//! R-30.6). Later phases add the call-site route goldens (G6/G8/G11/G13/G14)
//! and the lib goldens (G2-G5, G7). Driven through the real `sooth` binary,
//! harness helpers copied from `tests/phase7b_slice2.rs`; error goldens keep
//! the minimal two-line prefix so their line/column assertions stay readable
//! against the fixture.

// Each helper carries its own `#[allow(dead_code)]` rather than the module
// taking a blanket one: a phase may use only a subset, but a helper nothing
// uses at all should still be reported. (Convention from tests/common/mod.rs.)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs15-{}-{tag}-{seq}", std::process::id()));
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
    let binary = entry.with_extension("");
    std::fs::remove_file(&binary).ok();
}

fn build_error(entry: &Path) -> String {
    let build = sooth_build(entry);
    assert!(!build.status.success(), "build should have failed");
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

/// The printing-golden runner, from `tests/phase7b_slice2.rs`: builds, runs
/// the binary, asserts exit 0, and returns the exact stdout. Phase 2's route
/// goldens (G13/G14) use it; Phase 1's three goldens do not.
#[allow(dead_code)]
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

/// The hosted twin of `single_file`, from `tests/phase7b_slice2.rs`: adds
/// the hosted manifest (a bare package cannot import `core`) and the
/// selective `hosted::show | . |` import. Phase 2/3's lib goldens (G2-G5,
/// G7) use it; Phase 1's goldens do not.
#[allow(dead_code)]
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs15 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// Golden (G1, REQ-30.1): the true-shape member `pure ( 'A -- 'F['A] )`
/// declares under the R-30.1 output arm, and its ctor-keyed impl
/// (`impl: Applicative for Box : pure MkBox ;`) checks clean -- the impl
/// body's `Box['A]` output unifies against the dissolved declared output
/// through the input var, so the P2e identical-renderings wall does not
/// fire on the true shape (r2a; root-caused r2e). Bytes: the r2 baseline's
/// `soo30r2_a_decl_impl.sth` receipt -- build clean, no stderr, exit 0.
/// Fixture: the r2a probe's shape verbatim (the harness prepends the
/// intrinsics import, keeping the probe's line numbering).
#[test]
fn true_shape_member_declares_and_ctor_impl_checks() {
    let src = "\
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 drop ;
";
    let (_t, entry) = single_file("s15-g1-true-shape-decl-impl", src);
    build_ok(&entry);
}

/// Golden (G9, REQ-30.5): `ap` stays undeclarable -- the S1-6 parse fence
/// fires before any checking, so the shipped trait can declare `pure` only
/// and the impl-coverage question stays moot behind the fence (r1 P5). The
/// slice touches no parser code, so the measured bytes are expected
/// unchanged; the assert pins them anyway (diagnostics are behaviour).
/// Fixture: `probes/soo30_p5_ap_in_trait.sth` verbatim minus the intrinsics
/// import (the harness prepends it, keeping the probe's line numbering).
#[test]
fn ap_declaration_stays_parse_fenced() {
    let src = "\
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
  : ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 drop ;
";
    let (_t, entry) = single_file("s15-g9-ap-fence", src);
    let err = build_error(&entry);
    assert!(
        err.contains(
            "error: expected a type, found `[` at line 5, col 14 (a type application's \
             arguments are types, not quotations)"
        ),
        "{err}"
    );
}

/// Golden (G12, REQ-30.4): the zero-input escape hatch's impl mismatch stays
/// byte-identical -- recorded-not-fixed per R-30.6 (r2e root cause:
/// `check_poly_body`'s residual comparison is syntactic `PolyType`
/// `PartialEq`, and the body's minted `Concrete(Box[i64])` vs the declared
/// `Generic{Box, [i64]}` render identically but compare unequal). The
/// zero-input member bypasses the gate via the empty-input arm even
/// pre-relaxation, so these are today-stable bytes the slice must not move.
/// Fixture: `probes/soo30_p2e_applied_instantiation_zero_input.sth` verbatim
/// minus the intrinsics import (the harness prepends it).
#[test]
fn zero_input_escape_hatch_still_fails_impl_check() {
    let src = "\
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( -- 'F[i64] ) ;
;
impl: Applicative for Box
  : pure 42 MkBox ;
;
: main ( -- ) pure[Box[i64]] drop ;
";
    let (_t, entry) = single_file("s15-g12-zero-input-defect", src);
    let err = build_error(&entry);
    assert!(
        err.contains("error: stack effect mismatch in `pure;Applicative;0;Box['T0]`"),
        "{err}"
    );
    assert!(
        err.contains("body leaves `Box[i64]`, but the declared outputs are `Box[i64]`"),
        "{err}"
    );
}
