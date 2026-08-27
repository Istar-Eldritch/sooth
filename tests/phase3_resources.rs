//! Goldens for Phase 3 Slice 8b: a user `drop` body as a struct's destructor.
//!
//! Kept out of `tests/phase0.rs` (asserted never to change from this work's
//! base commit) and out of `tests/phase1.rs`, mirroring how slice 8a's
//! goldens live in `tests/phase3_strings.rs`.

use std::process::Command;

#[test]
fn slice8b_dogfood_compiles_and_runs() {
    // Criterion 19: `examples/resources.sth` opens, reads and closes a file,
    // with `close` reached only through `File`'s own `drop` overload. It reads
    // a dedicated 3-byte fixture rather than a project document so the golden
    // is deterministic; that makes it the first example to open a file *at
    // run time*, so the working directory is pinned explicitly (every other
    // golden uses its relative path as compiler input only).
    let root = env!("CARGO_MANIFEST_DIR");
    let binary = sooth::driver::build(&std::path::Path::new(root).join("examples/resources.sth"))
        .expect("build should succeed");
    let output = Command::new(&binary)
        .current_dir(root)
        .output()
        .expect("binary should run");
    assert_eq!(output.status.code(), Some(0), "dogfood should exit 0");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        "3\n"
    );
}
