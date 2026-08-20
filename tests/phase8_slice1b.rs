//! P8.S1b goldens: the `--manifest` CLI flag and the fallback chain past it
//! (user-level manifest, then the implicit anonymous package). Every
//! negative golden pins the exact diagnostic substring, never a bare
//! `is_err()`. Every golden spawns the real `sooth` binary (never
//! `driver::build_with_manifest`/`select_site`/`resolve_import` directly):
//! a direct-call test would prove the library honours the flag without
//! proving `main` ever passes it along (S1a's lesson, generalized).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A scratch tree of packages (each its own directory with a `sooth.pkg` and
/// `.sth` files), removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p8s1b-{}-{tag}-{seq}", std::process::id()));
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

/// Build the entry file through the real `sooth build --manifest` CLI
/// invocation (matching the exit criterion literally, not
/// `driver::build_with_manifest` called directly), then run the resulting
/// binary and return `(stdout, exit_code)`. Spawning the binary pins
/// `main`'s own wiring of the flag on the `build` arm, not just the
/// library's handling of it once wired.
fn build_and_run_with_manifest(entry: &Path, manifest: &Path) -> (String, i32) {
    let build_output = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .arg("--manifest")
        .arg(manifest)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build_output.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build_output.stderr)
    );
    // A clean build writes nothing to stderr, so this pins the "silently" half
    // of Golden 2: a conflict diagnostic about `--manifest` shadowing an
    // ancestor manifest would fail here rather than pass unnoticed.
    assert!(
        build_output.stderr.is_empty(),
        "build should emit no diagnostics; stderr: {}",
        String::from_utf8_lossy(&build_output.stderr)
    );
    let binary = entry.with_extension("");
    let output = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code().expect("process exits normally"),
    )
}

/// Goldens 3-5 exercise the fallback tiers past `--manifest`
/// (`ResolutionConfig::from_env`, R6), which reads `$XDG_CONFIG_HOME`/`$HOME`
/// -- process environment, not exposed by `driver::build`'s public
/// signature. Reading real per-process env from a parallel test suite would
/// race (R6 forbids exactly that), so these run the actual `sooth` CLI
/// binary as a child process, scoping `XDG_CONFIG_HOME` to that one child
/// rather than mutating this test process's environment. `HOME` is cleared
/// too, so a developer's real `~/.config/sooth/global_sooth.pkg` can never
/// leak into a golden that expects tier 4.
fn run_sooth_build(entry: &Path, xdg_config_home: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .env("XDG_CONFIG_HOME", xdg_config_home)
        .env_remove("HOME")
        .output()
        .expect("sooth build should spawn")
}

/// Golden 1 (the S2 fixture pattern): an entry `.sth` sits outside any
/// package tree, and `--manifest` names a package elsewhere on disk whose
/// `depends:` grants the entry's import. `build entry.sth --manifest
/// dep/sooth.pkg` resolves, builds, and runs clean.
#[test]
fn flag_resolves_entry_outside_its_package_tree() {
    let t = Tree::new("flag-resolves-outside-tree");
    t.write(
        "dep/sooth.pkg",
        "package: dep ; layer: hosted ; module: lib ;",
    );
    t.write("dep/lib.sth", ": lw ( -- i64 ) 7 ;\nexport: lw ;\n");
    let manifest = t.write(
        "flag/sooth.pkg",
        r#"package: flagpkg ; layer: hosted ; depends: dep path "../dep" ;"#,
    );
    let entry = t.write(
        "scratch/main.sth",
        "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
    );
    let (stdout, code) = build_and_run_with_manifest(&entry, &manifest);
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

/// Golden 2: the entry file sits inside package `p` (an ancestor manifest is
/// present), but `--manifest q/sooth.pkg` is given, naming a different
/// package whose `depends:` grants the import. Resolution goes to `q`
/// silently -- no conflict diagnostic, and `p`'s own manifest (which does not
/// grant the import) is never consulted.
#[test]
fn flag_overrides_ancestor_manifest_silently() {
    let t = Tree::new("flag-overrides-ancestor");
    t.write("p/sooth.pkg", "package: p ; layer: hosted ;");
    t.write(
        "dep/sooth.pkg",
        "package: dep ; layer: hosted ; module: lib ;",
    );
    t.write("dep/lib.sth", ": lw ( -- i64 ) 9 ;\nexport: lw ;\n");
    let manifest = t.write(
        "q/sooth.pkg",
        r#"package: q ; layer: hosted ; depends: dep path "../dep" ;"#,
    );
    let entry = t.write(
        "p/main.sth",
        "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
    );
    let (stdout, code) = build_and_run_with_manifest(&entry, &manifest);
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}

/// Golden 2b: the exit criterion names `build`/`run`, and `run` is a second,
/// independently-wired call site in `main`. The other goldens all reach the
/// flag through the `build` arm and then execute the produced binary
/// directly, so none of them notice if the `run` arm drops the flag. This one
/// goes through `sooth run --manifest` end to end.
#[test]
fn flag_wires_through_the_run_subcommand() {
    let t = Tree::new("flag-through-run");
    t.write(
        "dep/sooth.pkg",
        "package: dep ; layer: hosted ; module: lib ;",
    );
    t.write("dep/lib.sth", ": lw ( -- i64 ) 3 ;\nexport: lw ;\n");
    let manifest = t.write(
        "flag/sooth.pkg",
        r#"package: flagpkg ; layer: hosted ; depends: dep path "../dep" ;"#,
    );
    let entry = t.write(
        "scratch/main.sth",
        "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("run")
        .arg(&entry)
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .expect("sooth run should spawn");
    assert!(
        output.status.success(),
        "run should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "3\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Golden 3: a manifest-less entry resolves a dependency-anchored import
/// against the user-level manifest at `$XDG_CONFIG_HOME/sooth/global_sooth.pkg`
/// (tier 3), through the real CLI entry point.
#[test]
fn user_level_manifest_resolves_scratch_file() {
    let t = Tree::new("user-level-resolves");
    t.write(
        "dep/sooth.pkg",
        "package: dep ; layer: hosted ; module: lib ;",
    );
    t.write("dep/lib.sth", ": lw ( -- i64 ) 5 ;\nexport: lw ;\n");
    t.write(
        "cfg/sooth/global_sooth.pkg",
        r#"depends: dep path "../../dep" ;"#,
    );
    let entry = t.write(
        "scratch/main.sth",
        "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
    );
    let output = run_sooth_build(&entry, &t.0.join("cfg"));
    assert!(
        output.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary).output().expect("binary should run");
    std::fs::remove_file(&binary).ok();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "5\n",
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Golden 4: same shape as Golden 3, but the user-level manifest has no
/// `depends:` entry for the imported package -- pins R2(b) including the
/// user-manifest path.
#[test]
fn user_level_manifest_missing_depends_names_its_remedy() {
    let t = Tree::new("user-level-missing-depends");
    let user_manifest = t.write("cfg/sooth/global_sooth.pkg", "");
    let entry = t.write(
        "scratch/main.sth",
        "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
    );
    let output = run_sooth_build(&entry, &t.0.join("cfg"));
    assert!(!output.status.success(), "build should fail");
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("import `dep::lib` at line 1, col 1 in")
            && err.contains(&format!(
                "resolves against the user-level manifest {}",
                user_manifest.display()
            ))
            && err.contains("which has no `depends:` entry for `dep`")
            && err.contains(&format!(
                "add `depends: dep path \"<path>\" ;` to {}",
                user_manifest.display()
            )),
        "unexpected message: {err}"
    );
}

/// Golden 5: a manifest-less entry with no user-level manifest, importing a
/// dependency module, falls to the implicit anonymous package and names
/// itself (R2c): the anonymous status, and the three-way remedy.
#[test]
fn anonymous_package_names_itself() {
    let t = Tree::new("anonymous-names-itself");
    // An empty `XDG_CONFIG_HOME` with no `sooth/global_sooth.pkg` under it:
    // tier 3 is absent, so resolution falls all the way to tier 4.
    std::fs::create_dir_all(t.0.join("cfg")).unwrap();
    let entry = t.write(
        "scratch/main.sth",
        "import: dep::lib l ;\n: main ( -- ) l::lw . ;\n",
    );
    let output = run_sooth_build(&entry, &t.0.join("cfg"));
    assert!(!output.status.success(), "build should fail");
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("import `dep::lib` at line 1, col 1 in")
            && err.contains(
                "has no ancestor `sooth.pkg` and no user-level manifest, so it is an implicit anonymous package"
            )
            && err.contains("`dep` cannot be resolved")
            && err.contains("write $XDG_CONFIG_HOME/sooth/global_sooth.pkg with a `depends:` entry")
            && err.contains("add an ancestor `sooth.pkg`")
            && err.contains("pass `--manifest <path>`"),
        "unexpected message: {err}"
    );
}
