//! Phase 7 Slice 3f goldens: a ground `Type::Quotation` value crossing the
//! polymorphism boundary. This file lands the argument-boundary golden
//! (R1/R2) in phase 1; the body-boundary and round-trip goldens (R3) land in
//! phase 2, and the negatives land alongside whichever phase's fix they
//! exercise.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sooth::driver;

/// A scratch single-file program, removed on drop (`tests/phase7_slice3d.rs`'s
/// own pattern).
struct Scratch(PathBuf);

impl Scratch {
    fn write(tag: &str, contents: &str) -> Scratch {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7s3f-{}-{tag}-{seq}", std::process::id()));
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

/// R1/R2 behavioural: a poly word declaring both a real type variable and a
/// ground `Type::Quotation` parameter, called from a concrete body with a
/// literal quotation argument, run at two distinct instantiations of the
/// variable so it is carried rigidly rather than coincidentally matching.
#[test]
fn argument_boundary_materializes_ground_quotation_param() {
    let src = "import: intrinsics * ;\n\
               : run_it ( 'T: Copy [ i64 -- i64 ] -- 'T ) drop ;\n\
               : main ( -- )\n\
                 7 [ 1 add ] run_it .\n\
                 true [ 1 add ] run_it .\n\
               ;\n";
    let prog = Scratch::write("argument-boundary-behavioural", src);
    let (binary, stdout, code) = build_and_run(prog.path());
    std::fs::remove_file(&binary).ok();
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "7\ntrue\n",
        "each instantiation of `'T` must carry the materialized quotation argument independently"
    );
}
