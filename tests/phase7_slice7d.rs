//! P7.S7d phase 1 (R14): a user `extern:` declaring an `f64` parameter
//! delivers that double to the C callee. The probe round's P8 control
//! (`slice7d-probes.md`) compiled and ran but printed `0`, because the backend
//! spelled extern calls fully fixed and QBE therefore emitted no `%al` setup:
//! a variadic C callee reads `%al` to decide whether to spill the xmm
//! registers its `va_arg` then reads back, so the double was never saved and
//! `snprintf` formatted whatever the register save area held.
//!
//! These are build-and-run goldens rather than IL assertions on purpose: a
//! wrong spelling compiles, links, and silently misprints, so only running the
//! binary settles it. The IL form itself is pinned beside `emit_instr`
//! (`src/backend/qbe.rs`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s7d-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_and_run(t: &Tree, main: &str) -> String {
    t.write("sooth.pkg", "package: s7d ;\nlayer: hosted ;\n");
    let entry = t.write("main.sth", main);
    let binary = driver::build(&entry).expect("the fixture should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("the binary should run");
    std::fs::remove_file(&binary).ok();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout should be utf8")
}

/// The P8 control program's externs: `snprintf("%g\n", 2.5)` into a 64-byte
/// buffer, then `write(2)` of the formatted bytes.
const P8_EXTERNS: &str = "import: intrinsics * ;\n\
     extern: g-fmt ( &!array[u8 64] usize cstr f64 -- i32 ) \"snprintf\" ;\n\
     extern: sys-write ( i32 &!array[u8 64] usize -- isize ) \"write\" ;\n\
     extern: sys-strlen ( cstr -- usize ) \"strlen\" ;\n";

/// The spec's P8 control, verbatim. It passes *with or without* the fix: the
/// `fill` loop leaves its counter (64) in `%rax`, `snprintf` reads that as a
/// nonzero `%al` and spills the xmm registers anyway, and the double arrives
/// by luck. Kept as the spec's named exit artifact; the guard is the test
/// below.
#[test]
fn extern_f64_argument_reaches_variadic_callee() {
    let t = Tree::new("snprintf");
    let stdout = build_and_run(
        &t,
        &format!(
            "{P8_EXTERNS}\
             : main ( -- )\n\
             0 >u8 64 fill | buf |\n\
             &!buf 64 >usize \"%g\\n\" cstr 2.5 g-fmt >usize | n |\n\
             1 >i32 &!buf n sys-write drop\n\
             buf drop ;\n"
        ),
    );
    assert_eq!(stdout, "2.5\n", "the f64 argument reached snprintf");
}

/// The same program with one extra extern call ahead of it. `strlen("")`
/// returns 0, so `%al` is 0 on entry to `snprintf` unless the call itself sets
/// it -- which is exactly what the fixed spelling failed to do. Without the
/// preceding call the bug hides: `fill`'s loop counter leaves a nonzero `%rax`
/// behind, `snprintf` spills the xmm registers anyway, and the double arrives
/// by luck. The zero-%al premise is lowering-contingent (nothing guarantees
/// the two calls keep %rax clear between them); the deterministic half of the
/// guard is `emit_extern_call_with_f64_arg_is_all_args_variadic`, which pins
/// the spelling itself.
#[test]
fn extern_f64_argument_survives_a_zeroed_al_register() {
    let t = Tree::new("snprintf-zeroed-al");
    let stdout = build_and_run(
        &t,
        &format!(
            "{P8_EXTERNS}\
             : main ( -- )\n\
             0 >u8 64 fill | buf |\n\
             \"\" cstr sys-strlen drop\n\
             &!buf 64 >usize \"%g\\n\" cstr 2.5 g-fmt >usize | n |\n\
             1 >i32 &!buf n sys-write drop\n\
             buf drop ;\n"
        ),
    );
    assert_eq!(stdout, "2.5\n", "the f64 argument reached snprintf");
}

/// The spelling is applied to every extern, including non-variadic ones: a C
/// function that is not variadic ignores `%al`, so `strlen`/`write` are
/// unaffected. This is the guard on that claim.
#[test]
fn extern_calls_still_run_under_the_variadic_spelling() {
    let t = Tree::new("write-only");
    let stdout = build_and_run(
        &t,
        &format!(
            "{P8_EXTERNS}\
             : main ( -- )\n\
             0 >u8 64 fill | buf |\n\
             &!buf 64 >usize \"hi\\n\" cstr 0.0 g-fmt drop\n\
             \"hi\\n\" cstr sys-strlen | n |\n\
             1 >i32 &!buf n sys-write drop\n\
             buf drop ;\n"
        ),
    );
    assert_eq!(
        stdout, "hi\n",
        "strlen's count and write's bytes both survive"
    );
}
