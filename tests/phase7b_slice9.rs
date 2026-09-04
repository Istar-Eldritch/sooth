//! P7b.S9 Phase 2 exit golden: the operand-provenance fix (R1.1a,
//! `bare_ctor_own_module_grounding`, `src/check/terms.rs`) for a bare
//! generic-ctor call whose single `env` candidate is another module's
//! eager mint. Styled after `tests/phase7b_slice5.rs`'s `Tree` harness.

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
/// own header (`bare_ctor_own_module_grounding`), minting its own
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
