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
//! - G1/G3/G4/G6/G7/G8 (phase 2, the output arms): the lifted
//!   `poly_cross_output` renders Generic/Array/App outputs through the
//!   mapping, and the grounding chain (compose → apply_subst → lowering)
//!   carries them end to end. G1 doubles as G9 — the mechanism walkthrough
//!   (input arm binds head+arg, output arm renders `'It['T]`, compose folds
//!   θ_h, apply_subst's App arm grounds, lowering asserts the
//!   App-head-CtorImage invariant) — and is the golden that drives the whole
//!   chain, so it rides `build_run_keep`; G8 rides `build_ok` only (no array
//!   constructor exists intrinsics-only, so no grounding main is spellable
//!   and the claim is the walk-time render). All the green goldens' stdout
//!   is pinned empty (nothing prints).
//!
//! G12 (the Cursor-bounded grounding twin) lands with phase 3. G10's four
//! retargets live where the pins live: `src/check/poly/tests.rs` (:608,
//! :1782; the :3935 sub-fixture-2 retarget landed with phase 2) and
//! `tests/phase7b_slice12.rs` (:450, retargeted in phase 1).
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

/// (G1, phase 2) Probe E (`probes/s13_e_post_lift_green.sth`, verbatim but
/// for its own `import:` line — the harness prelude supplies it): the
/// end-to-end App pass-through cross-call — a mono `main` grounding
/// `Wrap`/i64, a poly caller whose own body walk hits the cross-call, and a
/// poly callee with an App-typed pass-through signature. Grounding is the
/// point (it drives the whole chain: mono App grounding → compose →
/// apply_subst's App arm → lowering's CtorImage expect), so this rides
/// `build_run_keep`: exit 0, stdout empty (the program is `42 Wrap outer
/// drop`; nothing prints).
///
/// This golden doubles as G9, the mechanism walkthrough: the input arm binds
/// the callee's head and argument to caller images (`F→CallerVar(It)`,
/// `T→CallerVar(T)`), the output App arm renders `'It['T]` back in the
/// caller's variable space (matching outer's declared output), compose folds
/// the mapping into θ_h = {F→CtorImage(Wrap), T→i64}, apply_subst resolves
/// step's `'F['T]` → `Wrap[i64]`, and lowering asserts the
/// App-head-CtorImage invariant (`src/ir/driver.rs:708-711`).
#[test]
fn app_pass_through_cross_call_builds_and_runs_clean() {
    let (_t, _binary, stdout) = build_run_keep(
        "g1-app-pass-through",
        "\\ S13 probe E — the end-to-end program the cross-call App lift should make pass.\n\
         \\ Helper with an App-typed pass-through signature, a poly caller doing the\n\
         \\ cross-call, and a concrete grounding in main (Wrap over i64, intrinsics only).\n\
         \\ Today: the S1-17.i fence must fire (captured here as the pre-lift golden);\n\
         \\ post-lift this is the primary green golden candidate.\n\
         \n\
         type: Wrap['X] | Wrap 'X ;\n\
         \n\
         : step ['F 'T] ( 'F['T] -- 'F['T] ) ;\n\
         : outer ['It 'T] ( 'It['T] -- 'It['T] ) step ;\n\
         : main ( -- ) 42 Wrap outer drop ;\n",
    );
    assert_eq!(stdout, "", "the program prints nothing");
}

/// (G3, phase 2) Edited D1 (`probes/s13_d_arg_var_mismatch.sth`): App-vs-App
/// with the argument variables differing — callee `'F['T]`, caller
/// `'It['U]`. The edits are forced: the callee declares pass-through (an
/// empty body fails its own stack check), outer declares its output (without
/// it the caller's own stack check fails — the body leaves `'It['U]` against
/// declared outputs empty), and a grounding `main` is spelled out (the link
/// step fails by design without one). Mechanism: the input arm binds head
/// `F→CallerVar(It)` and arg `T→CallerVar(U)`; the output App arm renders
/// `'It['U]` through the same lookup, matching outer's declared output;
/// main's grounding composes θ_h = {F→CtorImage(Wrap), U→i64}. Exit 0,
/// stdout empty.
#[test]
fn app_vs_app_head_and_arg_vars_both_bind() {
    let (_t, _binary, stdout) = build_run_keep(
        "g3-app-vs-app",
        "\\ S13 golden G3 — D1 post-lift green: App-vs-App, arg vars differ ('T callee, 'U caller).\n\
         \n\
         type: Wrap['X] | Wrap 'X ;\n\
         \n\
         : step ['F 'T] ( 'F['T] -- 'F['T] ) ;\n\
         : outer ['It 'U] ( 'It['U] -- 'It['U] ) step ;\n\
         : main ( -- ) 42 Wrap outer drop ;\n",
    );
    assert_eq!(stdout, "", "the program prints nothing");
}

/// (G4, phase 2) Edited D2 (`probes/s13_d_arg_concrete.sth`), same edit
/// shape as G3: a concrete argument grounds the element — `'It[i64]`
/// supplied where `'F['T]` is declared binds `T→Image::Concrete(i64)`
/// directly (the Var arm's Concrete case), so the element's concreteness
/// travels through the mapping, not around it; main grounds `It→`
/// `CtorImage(Wrap)` and compose builds θ_h = {F→CtorImage(Wrap), T→i64}.
/// Exit 0, stdout empty.
#[test]
fn app_vs_app_concrete_arg_grounds_the_element() {
    let (_t, _binary, stdout) = build_run_keep(
        "g4-concrete-arg",
        "\\ S13 golden G4 — D2 post-lift green: App-vs-App, concrete arg ('It[i64] vs 'F['T]).\n\
         \n\
         type: Wrap['X] | Wrap 'X ;\n\
         \n\
         : step ['F 'T] ( 'F['T] -- 'F['T] ) ;\n\
         : outer ['It] ( 'It[i64] -- 'It[i64] ) step ;\n\
         : main ( -- ) 42 Wrap outer drop ;\n",
    );
    assert_eq!(stdout, "", "the program prints nothing");
}

/// (G6, phase 2) Edited D4 (`probes/s13_d_generic_supplied.sth`): ruling
/// R-13.1 end to end — a concrete ctor supplied as an App head binds the
/// callee's head variable to a ctor-valued image (`Image::Concrete(`
/// `Type::CtorImage)`); the output App arm renders that head image back to
/// `PolyType::Generic{Wrap, [Var(T)]}`, matching outer's declared
/// `Wrap['T]`; main grounds `T→i64`; compose builds
/// θ_h = {F→CtorImage(Wrap), T→i64}; apply_subst's App arm resolves step's
/// `'F['T]` → `Wrap[i64]`. This is the golden that exercises R-13.1's
/// load-bearing claim (the head-image grounding, not just the walk-time
/// match). Exit 0, stdout empty.
#[test]
fn concrete_ctor_supplied_as_head_binds_the_ctor_image() {
    let (_t, _binary, stdout) = build_run_keep(
        "g6-ctor-image",
        "\\ S13 golden G6 — D4 post-lift green (ruling R-13.1: ctor-image head bind).\n\
         \n\
         type: Wrap['X] | Wrap 'X ;\n\
         \n\
         : step ['F 'T] ( 'F['T] -- 'F['T] ) ;\n\
         : outer ['T] ( Wrap['T] -- Wrap['T] ) step ;\n\
         : main ( -- ) 42 Wrap outer drop ;\n",
    );
    assert_eq!(stdout, "", "the program prints nothing");
}

/// (G7, phase 2) Edited C2 (`probes/s13_c_generic_output.sth`), aligned
/// spelling: the callee returns a concrete-ctor output and the poly caller
/// cross-calls it asking for the same variable it passes. The committed C2
/// caller asks for a DIFFERENT var than it passes (`['T 'U] ( 'T --
/// Wrap['U] ) mk ;`); post-lift that dies in the caller's own body check,
/// because the render is faithful to the mapping — it returns
/// `Wrap[caller's 'T]`, not the asked-for `Wrap['U]` — so the golden pins
/// the aligned spelling (the callee is the sub-fixture-2 pattern with the
/// caller aligned to what the mapping can carry). Mechanism: input
/// `T→CallerVar(U)` (Var arm, clean); the output Generic arm recurses
/// Wrap's argument through the Var-arm lookup → `PolyType::Generic{Wrap,
/// [Var(U)]}`, matching caller's declared output; main grounds `U→i64`;
/// compose grounds mk's `Wrap['T]` → `Wrap[i64]`. Exit 0, stdout empty.
#[test]
fn generic_output_renders_through_the_mapping() {
    let (_t, _binary, stdout) = build_run_keep(
        "g7-generic-output",
        "\\ S13 golden G7 — C2 post-lift green: Generic output rendered through the mapping.\n\
         \n\
         type: Wrap['X] | Wrap 'X ;\n\
         \n\
         : mk ['T] ( 'T -- Wrap['T] ) Wrap ;\n\
         : caller ['U] ( 'U -- Wrap['U] ) mk ;\n\
         : main ( -- ) 42 Wrap caller drop ;\n",
    );
    assert_eq!(stdout, "", "the program prints nothing");
}

/// (G8, phase 2) Edited C4 (`probes/s13_c_array_output.sth`), whose only
/// edit is appending the empty main: an Array output rendered through the
/// mapping — the element var maps `U→T` on the input side (clean), so the
/// output Array arm recurses the element through the Var-arm lookup →
/// `array['U 4]`, matching caller's declared output, the length riding
/// through concrete. `build_ok` ONLY: no array constructor exists
/// intrinsics-only, so no grounding main is spellable and compose's array
/// interning (`intern_array_type`) is not exercised by a golden — the claim
/// is the walk-time render.
#[test]
fn array_output_renders_through_the_mapping() {
    build_ok(
        "g8-array-output",
        "\\ S13 golden G8 — C4 post-lift green: Array output rendered through the mapping.\n\
         \n\
         : mk ['T] ( array['T 4] -- array['T 4] ) ;\n\
         : caller ['U] ( array['U 4] -- array['U 4] ) mk ;\n\
         : main ( -- ) ;\n",
    );
}
