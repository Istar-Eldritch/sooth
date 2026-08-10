//! Behavioural goldens for the single-file symbol-hijack fix. A single-file
//! build used to emit unmangled symbols, so a user word whose bare name equalled
//! a libc symbol (or a runtime shim's callee) silently hijacked it at link time.
//! Native builds now mangle even a one-file closure (`resolve::resolve_modules`
//! forced on via `driver::assemble_module`'s `always_mangle`), so `main` and
//! `drop` aside, every user word is `name__m0` and can no longer collide.
//!
//! Each test asserts exact stdout and exit code, not merely "does not crash":
//! before the fix each program produced observably different output (mode 1
//! recursed to a SIGSEGV; mode 2 ran the user word a second time from the heap
//! shim), so the exact-output assertion is what makes the hijack visible.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-hijack-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prog.sth");
        std::fs::write(&path, contents).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

/// Build and run the program, returning `(stdout, exit_code)`. A build failure
/// panics: these programs are all well-typed, so a build error is itself a
/// regression to surface, not an expected outcome.
fn build_and_run(src: &Path) -> (String, Option<i32>) {
    let binary = driver::build(src).expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output.status.code(),
    )
}

/// Mode 1: a user word `close` shares the ABI symbol of a bound `extern:`
/// ("close"). The `Fd` destructor calls the extern; before the fix its `$close`
/// link-resolved to the user word, whose body drops a `File` and so re-enters
/// the same destructor: unbounded recursion printing `111` until the stack
/// overflows (SIGSEGV). After the fix the extern reaches libc `close`, the fd is
/// released once, and the program prints a single `111` and exits cleanly.
#[test]
fn single_file_word_named_like_extern_symbol_does_not_recurse() {
    let prog = Scratch::write(
        "extern-collision",
        "extern: close-fd ( i64 -- i64 ) \"close\" ;\n\
         type: Fd n i64 ;\n\
         : drop ( Fd -- ) | h | 111 . h Fd>n close-fd drop ;\n\
         type: File fd Fd ;\n\
         : close ( File -- ) drop ;\n\
         : main ( -- ) 7 Fd File | f | f close ;\n",
    );
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(
        stdout, "111\n",
        "the fd is released exactly once, no recursion"
    );
    assert_eq!(code, Some(0), "clean exit, not a SIGSEGV (exit code None)");
}

/// Mode 2: a user word named `free`. The compiler's heap shim `sooth_free`
/// calls the C `free`; before the fix the user word owned the bare `free`
/// symbol, so releasing an owned cell (`c drop`) re-ran the user word instead of
/// libc `free`. The user word prints `99`, so the hijack shows up as a *second*
/// `99` (once for the explicit `42 free`, once smuggled in via the shim). After
/// the fix the shim reaches libc `free` and only the explicit call prints.
#[test]
fn single_file_word_named_free_does_not_hijack_heap_shim() {
    let prog = Scratch::write(
        "free-collision",
        ": free ( i64 -- ) 99 . drop ;\n\
         : main ( -- ) 7 ^ | c | 42 free c drop ;\n",
    );
    let (stdout, code) = build_and_run(prog.path());
    assert_eq!(
        stdout, "99\n",
        "only the explicit `42 free` prints; the heap shim reaches libc free"
    );
    assert_eq!(code, Some(0));
}
