// Each helper below is consumed by only some of the test binaries that
// include this module, so an individual binary using a subset is not dead
// code.
#![allow(dead_code)]

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

/// Strip `\` line comments and collapse all whitespace to single spaces, so
/// two Sooth source snippets can be compared up to formatting.
fn normalize_sooth(src: &str) -> String {
    src.lines()
        .map(|line| line.split('\\').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Assert a test file's hand-copied `times`/`times-helper` definition (kept so
/// in-process `check_error`/`check_ok`, which never resolve `import:`, can
/// exercise `times` without a file-based import) is still a verbatim copy of
/// the real `lib/combinators.sth`, modulo whitespace. Without this a body
/// change to `times`/`times-helper` would leave every hand-copy silently
/// exercising the old shape. `renames` substitutes each `(from, to)`
/// whole-token identifier in `hand_copy` before comparing, for a copy that
/// must bind its quotation parameter under a different local name (see
/// `phase3_refs.rs`, which cannot shadow its own `f` word).
pub fn assert_pinned_to_combinators_lib(hand_copy: &str, renames: &[(&str, &str)]) {
    let mut normalized = normalize_sooth(hand_copy);
    for (from, to) in renames {
        normalized = normalized
            .split(' ')
            .map(|tok| if tok == *from { to } else { tok })
            .collect::<Vec<_>>()
            .join(" ");
    }
    let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/combinators.sth"))
        .expect("the combinator library should be readable");
    let lib = normalize_sooth(&lib);
    assert!(
        lib.contains(&normalized),
        "hand-copied times/times-helper has drifted from lib/combinators.sth\n  copy (normalized): {normalized}\n  lib (normalized):  {lib}"
    );
}
