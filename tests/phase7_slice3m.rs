//! P7.S3m goldens: a declared quotation effect with two or more outputs. The
//! bundle such a call returns through was minted only for a *word*'s own
//! output tuple, so `bundle_of` found nothing for a quotation's, the
//! `CallIndirect` produced no value, and every output past the first was never
//! pushed -- the first consumer then underflowed the stack. Both goldens print
//! the right values, not merely avoid the panic.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

mod common;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3f.rs`'s
/// pattern), carrying its own `sooth.pkg` so `core::bool` resolves.
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3m-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sooth.pkg"), common::fixture_package("p7s3m")).unwrap();
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

fn build_and_run(src: &Path) -> (PathBuf, String, i32) {
    let binary = driver::build(src).expect("program should build");
    let output = std::process::Command::new(&binary)
        .output()
        .expect("binary should run");
    (
        binary,
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

/// R1, site 1: a concrete word taking `[ i64 -- i64 i64 ]` and calling it.
/// `3 swap call` runs `[ dup ]` on `3`, `add` sums both outputs -- which is
/// only possible if the second one was pushed at all.
#[test]
fn concrete_call_it_pushes_both_quotation_outputs() {
    let src = "import: intrinsics * ;\n\
               : call_it ( [ i64 -- i64 i64 ] -- ) 3 swap call add . ;\n\
               : main ( -- ) [ dup ] call_it ;\n";
    let prog = Scratch::write("concrete", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "6\n",
        "both outputs of the called quotation must reach the stack"
    );
}

/// R1, site 1's output half: a word *returning* a two-output quotation, called
/// by its caller. The word's own output tuple is a single slot, so no
/// word-level bundle covers this -- only descending into the output slot's type
/// finds the quotation's own tuple.
#[test]
fn returned_quotation_pushes_both_outputs() {
    let src = "import: intrinsics * ;\n\
               : mk ( -- [ i64 -- i64 i64 ] ) [ dup ] ;\n\
               : main ( -- ) 3 mk call add . ;\n";
    let prog = Scratch::write("returned", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "6\n",
        "a quotation reached through a declared output must still return both values"
    );
}

/// R1, site 4: the same parameter on a polymorphic word, whose declared shape
/// lives in `w.poly` rather than `w.effect`. `'T` is unrelated to the
/// quotation and carried rigidly through two instantiations (S3f's golden
/// shape).
///
/// `call_it` returns a *single* output deliberately. Giving it the two outputs
/// `( ... -- 'T i64 )` makes the golden a placebo: the `'T = i64`
/// instantiation's own return bundle is `[i64 i64]`, the very tuple the
/// quotation needs, so the program builds and prints correctly even with
/// discovery unwidened.
#[test]
fn polymorphic_call_it_pushes_both_quotation_outputs() {
    let src = "import: intrinsics * ;\n\
               import: core::bool * ;\n\
               : call_it ( 'T: Copy [ i64 -- i64 i64 ] -- 'T ) 3 swap call add . ;\n\
               : main ( -- )\n\
                 9 [ dup ] call_it .\n\
                 True [ dup ] call_it .\n\
               ;\n";
    let prog = Scratch::write("polymorphic", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "6\n9\n6\nTrue\n",
        "the quotation's two outputs must reach the stack at each instantiation of `'T`"
    );
}
