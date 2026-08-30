//! P7b.S1 exit goldens: kinds and type-level application. This file starts
//! with the Phase 2 goldens (application parsing + compile-forcing); later
//! phases add to it. Driven through the real `sooth` binary, styled after
//! `tests/phase7_slice6a.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs1-{}-{tag}-{seq}", std::process::id()));
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

fn single_file(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", &format!("import: intrinsics * ;\n{src}"));
    (t, entry)
}

/// The k8 flip (S1-3/S1-5): `array['T 'N]` with no `'N: Len` annotation
/// anywhere -- `'N`'s kind is inferred from the count position it appears
/// in, so the program builds and runs instead of S6a's mandatory-annotation
/// rejection.
#[test]
fn hkt_len_var_inferred_from_count_position_is_accepted() {
    let src = "\
        : sum['T 'N] ( array['T 'N] -- usize ) len swap drop ;\n\
        : main ( -- )\n          0 >u8 4 fill sum .\n        ;\n";
    let (_t, entry) = single_file("k8-flip", src);
    let out = build_and_run(&entry);
    assert_eq!(out, "4\n");
}

/// S1-6 router (both readings in one effect): `'F['T]` is an application,
/// `[ 'T -- 'U ]` right after it is the *next* slot -- a declared quotation
/// parameter, not part of the application. Pins the router: today's F1
/// error at the first `[` must be gone.
#[test]
fn hkt_var_before_quotation_parameter_still_parses() {
    // The output is dropped rather than declared `'F['U]` (S1-11's App
    // grounding, which would let a body genuinely construct one, is Phase
    // 3): what this golden pins is the *parse*, so the body only needs to
    // consume both readings, not produce an application value.
    let src = "\
: fmap['F 'T 'U] ( 'F['T] [ 'T -- 'U ] -- ) drop drop ;
: main ( -- ) ;
";
    let (_t, entry) = single_file("router", src);
    build_ok(&entry);
}

/// Non-regression: a declared quotation parameter and an S3t-style explicit
/// instantiation still parse -- the router must not disturb either existing
/// shape.
#[test]
fn hkt_concrete_generic_effect_and_explicit_instantiation_unchanged() {
    let src = "\
: q['T 'U] ( 'T [ 'T -- 'U ] -- 'U ) call ;
: pairwise ( 'T 'U -- ) drop drop ;
: main ( -- ) 1 2.5 pairwise[i64 f64] ;
";
    let (_t, entry) = single_file("nonregression", src);
    build_ok(&entry);
}
