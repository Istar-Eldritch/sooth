//! P7b.S13 goldens — the cross-call App lift (SOO-60).
//!
//! The two located fences in `poly_cross_match` / `poly_cross_output`
//! (`src/check/poly/crosscall.rs`) reject poly→poly cross-calls over
//! higher-kinded shapes. This file pins the lift's verdicts as they land,
//! phase by phase:
//!
//! - G2: the growth ban does not move (REQ-3, probe fact 1). An App operand
//!   facing a bare-var declared input is the GROWTH error, byte-identical to
//!   the frozen baseline (`probes/s13_baseline.md` B2) — the Var arm
//!   (`crosscall.rs:208`) precedes the lifted App arm and is untouched. If
//!   this golden moves, the slice has broken precedence.
//! - G5: the declared-App/supplied-bare-var face MOVES from the S1-17.i
//!   fence to the rendered mismatch (REQ-1's sub-dispatch: `Var(_)` routes
//!   to the catch-all `mismatch()`). The bytes are pinned from the live
//!   binary (the only moving verdicts of the slice; the `note: declared`
//!   tail renders the caller's effect via `effect_str`).
//!
//! Goldens G1/G3/G4/G6–G9 (the output arms and the end-to-end grounding
//! chain) land with the slice's phase 2; G12 (the Cursor-bounded grounding
//! twin) with phase 3. G10's four retargets live where the pins live:
//! `src/check/poly/tests.rs` (:608, :1782) and `tests/phase7b_slice12.rs`
//! (:450, retargeted in phase 1), plus `tests.rs:3935` sub-fixture 2 in
//! phase 2.
//!
//! Harness style from `tests/phase7b_slice8.rs` / `tests/phase7b_slice12.rs`.
//! Golden sources are embedded strings, not reads of `probes/` (those files
//! stay byte-frozen as the pre-fix baseline). The harness prelude's
//! `import: intrinsics * ;` replaces a fixture's own import line — a
//! duplicate collides in the import seen-map — which shifts a diagnostic one
//! line down, so a baseline byte-pin cannot ride `build_error_located`: G2
//! embeds its fixture byte-verbatim (own import, bare temp dir, no manifest)
//! through `build_error_bare` and pins the baseline bytes exactly.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs13-{}-{tag}-{seq}", std::process::id()));
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

/// `tests/phase7b_slice8.rs`'s hosted single-file fixture, verbatim but for
/// the package name.
fn single_file_hosted(tag: &str, src: &str) -> (Tree, PathBuf) {
    let t = Tree::new(tag);
    t.write(
        "sooth.pkg",
        &format!(
            "package: p7bs13 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
            root = env!("CARGO_MANIFEST_DIR")
        ),
    );
    let entry = t.write(
        "main.sth",
        &format!("import: intrinsics * ;\nimport: hosted::show | . | ;\n{src}"),
    );
    (t, entry)
}

fn build_run_keep(tag: &str, src: &str) -> (Tree, PathBuf, String) {
    let (t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
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
        .expect("the binary should run");
    assert_eq!(
        run.status.code(),
        Some(0),
        "the built binary should exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    (t, binary, stdout)
}

/// Build `src` and assert it succeeds, discarding the binary and its output.
fn build_ok(tag: &str, src: &str) {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
}

/// Build `src`, assert it fails *located* (a clean non-zero exit with a
/// `error: ...` stderr, never a panic), and return that stderr.
fn build_error_located(tag: &str, src: &str) -> String {
    let (_t, entry) = single_file_hosted(tag, src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    let stderr = String::from_utf8(build.stderr).expect("stderr should be utf8");
    assert!(
        !stderr.contains("panicked"),
        "a check rejection must never panic, got: {stderr}"
    );
    stderr
}

/// Build `src` the way the frozen baseline was captured — the source
/// byte-verbatim (its own `import: intrinsics * ;` line included) in a bare
/// temp dir with no `sooth.pkg`, so line numbers match
/// `probes/s13_baseline.md`'s capture exactly. Used only by G2's
/// byte-identity pin (REQ-3): every other golden rides the harness prelude.
fn build_error_bare(tag: &str, src: &str) -> String {
    let t = Tree::new(tag);
    let entry = t.write("main.sth", src);
    let build = Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(&entry)
        .output()
        .expect("sooth build should spawn");
    assert!(!build.status.success(), "build should have failed");
    let stderr = String::from_utf8(build.stderr).expect("stderr should be utf8");
    assert!(
        !stderr.contains("panicked"),
        "a check rejection must never panic, got: {stderr}"
    );
    stderr
}

/// (G2, REQ-3 must-not-move) Probe B2
/// (`probes/s13_b_bare_var_growth.sth`, verbatim): a callee declaring a
/// bare var, the caller handing it an App-typed value. The `(Var(v), _)`
/// arm precedes the lifted App arm and its supplied-match is untouched, so
/// the verdict is the growth error — never the (deleted) fence, never a
/// bind. Built the way the baseline was captured — byte-verbatim, bare temp
/// dir, no manifest — so the stderr equals the frozen `probes/s13_baseline.md`
/// entry (B2) byte for byte. If these bytes move, the slice has broken
/// precedence (probe fact 1).
#[test]
fn bare_var_supplied_app_operand_still_grows_byte_identically() {
    let stderr = build_error_bare(
        "g2-bare-var-growth",
        "\\ S13 probe B2 — callee declares a bare var; caller hands it an App-typed value.\n\
         \\ Predicted: NOT the fence — the growth error (Var arm precedes the App arm).\n\
         import: intrinsics * ;\n\
         \n\
         : step ['T] ( 'T -- 'T ) ;\n\
         : outer ['It 'T] ( 'It['T] -- 'It['T] ) step ;\n\
         : main ( -- ) ;\n",
    );
    assert_eq!(
        stderr,
        "error: `outer` cannot pass `'It['T]` to `'T` of the polymorphic word `step` (line 6, col 41)\n  a polymorphic call site may pass a type variable only bare: wrapping it in `'It['T]` builds a larger type at every hop of a recursive call, which has no finite set of instantiations\n  declare `step`'s parameter as `'It['T]` so the shape is matched structurally, or call it from a monomorphic word\n"
    );
}

/// (G5, phase 1) Probe D3 (`probes/s13_d_bare_supplied.sth`, verbatim but
/// for its own `import:` line — the harness prelude supplies it): a
/// declared App slot facing a supplied bare variable. A bare var always has
/// kind `*` — unbounded vars infer `*`, and a `* -> *` variable cannot be
/// used bare — so it can never fill an applied head; binding it would type
/// a value-kind variable as a constructor. The sub-dispatch routes it to
/// the catch-all `mismatch()`, so the verdict MOVES from the S1-17.i fence
/// to the rendered mismatch. Bytes pinned from the live binary at the
/// harness layout (the prelude puts the call on line 9; the caller's own
/// `note: declared` tail renders its effect).
#[test]
fn bare_var_supplied_where_app_declared_is_a_rendered_mismatch() {
    let stderr = build_error_located(
        "g5-bare-supplied-mismatch",
        "\\ S13 probe D3 — declared App, supplied bare var ('F as a plain value).\n\
         \\ First write had an unused bound 'T in outer (rejected: \"never appears in the\n\
         \\ effect\" — recorded); binds now list only 'F. Callee declares pass-through.\n\
         \\ Predicted: the S1-17.i input fence (declared-App arm).\n\
         \n\
         : step ['F 'T] ( 'F['T] -- 'F['T] ) ;\n\
         : outer ['F] ( 'F -- ) step ;\n",
    );
    assert_eq!(
        stderr,
        "error: type mismatch in `outer` (line 9)\n  `step` expected `'F['T]`, found `'F`\n  note: declared ( -- )\n"
    );
}
