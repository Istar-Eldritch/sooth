//! P7.S3s phase 0 goldens: a trait re-exported through a hub module resolves
//! as a bound and as an `impl:` target, through the real `sooth` binary so
//! the whole-closure `resolve_trait_export_origins` walk in
//! `driver::assemble_module` is actually exercised (`find_trait_in_module`'s
//! own unit tests cover the function directly; these pin the end-to-end
//! wiring: `trait_origin` threaded through `parse_bodies` and consulted by
//! both `find_trait_in_module` call sites).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod common;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3s-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(
            &path,
            format!("{contents}{}", common::printing_import(contents)),
        )
        .unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_and_run(entry: &Path) -> String {
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg("--manifest")
        .arg(common::fixture_manifest())
        .arg(entry)
        .output()
        .expect("sooth build should spawn");
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

/// `base.sth` declares `Greet` and exports it; `hub.sth` never declares
/// `Greet` itself, only re-exports the name it reached through
/// `hub_import` -- which decides which arm of `walk_trait_export_origin`
/// carries the hop out of the hub.
fn base_and_hub(t: &Tree, hub_import: &str) {
    t.write(
        "base.sth",
        "trait: Greet['T] : greet ( &'T -- ) ; ;\nexport: Greet ;\n",
    );
    t.write("hub.sth", &format!("{hub_import}\nexport: Greet ;\n"));
}

const SELECTIVE_HUB_IMPORT: &str = "import: \"base.sth\" b | Greet | ;";
const QUALIFIED_HUB_IMPORT: &str = "import: \"base.sth\" b ;";

/// A trait declared in module A, re-exported (not declared) by hub module B,
/// consumed by module C via a selective import of the hub -- used as a bound
/// on a generic word's own type variable.
#[test]
fn hub_reexported_trait_resolves_as_a_bound_via_selective_import() {
    let t = Tree::new("selective");
    base_and_hub(&t, SELECTIVE_HUB_IMPORT);
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"hub.sth\" h | Greet | ;\n\
         type: Point x i64 y i64 ;\n\
         impl: Greet for Point\n\
           : greet | p | p drop 42 .  ;\n\
         ;\n\
         : greets ['T: Greet] ( &'T -- ) greet ;\n\
         : main ( -- ) 1 2 Point |p| &p greets p drop ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(out, "42\n");
}

/// The same trait, reached from module C by bare qualifier (`h::Greet`)
/// instead of a selective import -- `find_trait_in_module`'s qualified
/// branch's own `trait_origin` fallback.
#[test]
fn hub_reexported_trait_resolves_as_a_bound_via_bare_qualifier() {
    let t = Tree::new("qualified");
    base_and_hub(&t, SELECTIVE_HUB_IMPORT);
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"hub.sth\" h ;\n\
         type: Point x i64 y i64 ;\n\
         impl: h::Greet for Point\n\
           : greet | p | p drop 42 .  ;\n\
         ;\n\
         : greets ['T: h::Greet] ( &'T -- ) greet ;\n\
         : main ( -- ) 1 2 Point |p| &p greets p drop ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(out, "42\n");
}

/// C4's second caller: an `impl:` naming a hub-re-exported trait as its
/// target resolves too, not only a bound use. Deliberately has no bound
/// consumer of `Greet` (unlike the two goldens above): every `impl:` still
/// needs to parse for the bound goldens to build too, so a broken
/// `parse_impl_decl` fallback fails all three -- but only this golden keeps
/// passing if `bound_trait_id`'s fallback alone regresses, which is what
/// isolates the `parse_impl_decl` call site from the `bound_trait_id` one.
/// Nothing calls `greet` here, so `main` prints a value the impl body never
/// prints: the assertion is about the `impl:` header parsing, not dispatch.
#[test]
fn hub_reexported_trait_resolves_as_an_impl_target() {
    let t = Tree::new("impl-target");
    base_and_hub(&t, SELECTIVE_HUB_IMPORT);
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"hub.sth\" h | Greet | ;\n\
         type: Point x i64 y i64 ;\n\
         impl: Greet for Point\n\
           : greet | p | p drop 7 .  ;\n\
         ;\n\
         : main ( -- ) 1 2 Point |p| p drop 5 . ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(out, "5\n");
}

/// The hub imports its base with a qualifier only, putting nothing on its
/// selective map, so the hop out of the hub goes through
/// `walk_trait_export_origin`'s second arm (the first import target that
/// declares the name) instead of the first.
#[test]
fn trait_reexported_through_a_qualifier_only_hub_resolves() {
    let t = Tree::new("qualifier-hub");
    base_and_hub(&t, QUALIFIED_HUB_IMPORT);
    let entry = t.write(
        "main.sth",
        "import: intrinsics * ;\n\
         import: \"hub.sth\" h | Greet | ;\n\
         type: Point x i64 y i64 ;\n\
         impl: Greet for Point\n\
           : greet | p | p drop 42 .  ;\n\
         ;\n\
         : greets ['T: Greet] ( &'T -- ) greet ;\n\
         : main ( -- ) 1 2 Point |p| &p greets p drop ;\n",
    );
    let out = build_and_run(&entry);
    assert_eq!(out, "42\n");
}
