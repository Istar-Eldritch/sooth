use std::path::{Path, PathBuf};

/// Build a committed example without racing a concurrent build of the same file.
///
/// `driver::build` writes its output to `source.with_extension("")`, a fixed
/// path in the tree, so two processes building one example (a test run plus a
/// manual `sooth build`, or two `cargo test` invocations) race: one execs the
/// binary while the other is still writing it, and the exec fails with
/// `ExecutableFileBusy`. Copying the source to a per-process sibling first makes
/// the output path unique. The copy stays in the example's own directory so
/// relative imports still resolve, and its `.tmpsth` extension keeps both the
/// copy and its binary under `.gitignore`'s `/examples/*`.
pub fn build_example(rel: &str) -> PathBuf {
    let src = Path::new(rel);
    let stem = src
        .file_stem()
        .expect("an example path has a file name")
        .to_string_lossy()
        .into_owned();
    let copy = src.with_file_name(format!("{stem}.{}.tmpsth", std::process::id()));
    std::fs::copy(src, &copy).expect("copying the example should succeed");
    let built = sooth::driver::build(&copy);
    std::fs::remove_file(&copy).ok();
    built.expect("the example should build")
}
