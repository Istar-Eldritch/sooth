use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Build a committed example without racing a concurrent build of the same file.
///
/// `driver::build` writes its output to `source.with_extension("")`, a fixed
/// path in the tree, so two builds of one example race: one execs the binary
/// while the other is still writing it (`ExecutableFileBusy`), or one removes it
/// while the other is about to exec it (`NotFound`). This happens both across
/// processes (a test run plus a manual `sooth build`, or two `cargo test`
/// invocations) and across threads of one process (two `#[test]`s in a binary
/// building the same example in parallel). Copying the source to a per-call
/// sibling first makes the output path unique: the pid separates processes and
/// the counter separates calls within a process. The copy stays in the
/// example's own directory so relative imports still resolve, and its `.tmpsth`
/// extension keeps both the copy and its binary under `.gitignore`'s
/// `/examples/*`.
pub fn build_example(rel: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let src = Path::new(rel);
    let stem = src
        .file_stem()
        .expect("an example path has a file name")
        .to_string_lossy()
        .into_owned();
    let copy = src.with_file_name(format!("{stem}.{}.{nonce}.tmpsth", std::process::id()));
    std::fs::copy(src, &copy).expect("copying the example should succeed");
    let built = sooth::driver::build(&copy);
    std::fs::remove_file(&copy).ok();
    built.expect("the example should build")
}
