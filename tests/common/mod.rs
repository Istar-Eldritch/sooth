// Each helper carries its own `#[allow(dead_code)]` rather than the module
// taking a blanket one: a test binary including this module may use only a
// subset, but a helper nothing uses at all should still be reported.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// P8.S2 (R7): the one manifest every file-based fixture resolves `core`
/// against, named on the command line via `--manifest` rather than discovered.
/// A fixture written to a temp directory has no ancestor `sooth.pkg`, and
/// inheriting a developer's user-level manifest would make the suite depend on
/// machine-local config; a path named here is as reproducible as the fixture
/// itself.
#[allow(dead_code)]
pub fn fixture_manifest() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/sooth.pkg"
    ))
}

/// Which manifest `entry` should be built against: `None` when it already has
/// an ancestor `sooth.pkg` (a committed example, or a package tree a test wrote
/// on purpose -- in both cases ordinary discovery is the thing under test), and
/// the shared fixture manifest otherwise. That covers every harness-written
/// fixture in a temp directory, which has no ancestor manifest and would
/// otherwise be an implicit anonymous package unable to name `core` at all.
#[allow(dead_code)]
pub fn manifest_for(entry: &Path) -> Option<PathBuf> {
    let mut dir = entry.parent();
    while let Some(d) = dir {
        if d.join("sooth.pkg").is_file() {
            return None;
        }
        dir = d.parent();
    }
    Some(fixture_manifest())
}

/// The eight words `core::prelude` re-exports: the typed core the compiler used
/// to inject into every program before P8.S2 deleted the prelude.
const CORE_WORDS: [&str; 8] = ["if", "unless", "eq", "lt", "gt", "lte", "gte", "ne"];

/// A `sooth.pkg` body for a fixture tree that needs its *imported* modules to
/// resolve `core` as well as its entry file. `--manifest` covers the entry only
/// (S1b R3: a transitively imported file re-derives its own package), so a
/// multi-file fixture where the dependency itself names `core` has to be a real
/// package with a real ancestor manifest. `depends:` names `lib/` absolutely,
/// since the tree lives in a temp directory at an unrelated depth.
#[allow(dead_code)]
pub fn fixture_package(name: &str) -> String {
    format!(
        "package: {name} ;\nlayer: hosted ;\ndepends: core path \"{}/lib\" ;\n",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// P8.S2 (R3): the REPL session line that brings the typed core into a session.
/// A session no longer auto-seeds `if`/`lt` -- it imports them like a file does
/// -- and it names the two declaring modules rather than the `core::prelude`
/// hub, because the REPL's dlopen retention keeps a module's own exported
/// *definitions* and so cannot follow a hub's re-export.
#[allow(dead_code)]
pub fn repl_core_import(module: &str, names: &str) -> String {
    format!(
        "import: \"{}/lib/{module}.sth\" {module} | {names} | ;",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// `repl_core_import` for both modules as two ready-to-feed session lines: the
/// whole typed core, for a golden that would rather not enumerate what it uses.
#[allow(dead_code)]
pub fn repl_core_lines() -> String {
    format!(
        "{}\n{}\n",
        repl_core_import("cmp", "eq lt gt lte gte ne"),
        repl_core_import("bool", "if unless")
    )
}

/// What `repl_core_lines` itself prints, so a golden can keep pinning the exact
/// transcript rather than loosening to `contains`.
#[allow(dead_code)]
pub const REPL_CORE_ECHO: &str = "imported cmp\nimported bool\n";

/// P8.S2 (R7): the `import:` lines a file-based fixture needs, derived from the
/// fixture itself and appended by the harness -- for the same reason
/// `fixture_manifest` is one shared file rather than one per grouping: every
/// fixture wants exactly the imports its own text implies, so several hundred
/// hand-written copies would be identical lines that nothing keeps in step.
///
/// `intrinsics` is unconditional: it binds no name (it is a visibility line, not
/// a selective import), so it cannot collide with anything the fixture
/// declares. The `core` half is *selective and computed*: a fixture that
/// declares its own `lt` overload must not also have `lt` bound from `core`,
/// which is a hard collision, so only the core words the fixture calls **and
/// does not declare** are imported.
///
/// *Appended*, not prepended, and that is load-bearing: a fixture's own line
/// numbers are what its located-diagnostic assertions pin, and `import:` is
/// legal anywhere at the top level, so adding lines at the end leaves every
/// `line N, col C` in the suite meaning what it meant before.
///
/// Private: fixtures reach it through `write_fixture`/`fixture_source`, so the
/// "which imports" rule has exactly one implementation and no second caller
/// deriving its own.
fn fixture_imports(src: &str) -> String {
    let tokens: Vec<&str> = src.split_whitespace().collect();
    let declared: Vec<&str> = tokens
        .windows(2)
        .filter(|w| matches!(w[0], ":" | "static:" | "type:"))
        .map(|w| w[1])
        .collect();
    let wanted: Vec<&str> = CORE_WORDS
        .iter()
        .copied()
        .filter(|w| !declared.contains(w) && tokens.contains(w))
        .collect();
    // A fixture copied out of the committed corpus already carries its own
    // imports; adding a second binding of the same name is a hard collision.
    let mut out = String::new();
    if !src.contains("import: intrinsics") {
        out.push_str("\nimport: intrinsics * ;\n");
    }
    if !wanted.is_empty() && !src.contains("import: core::prelude") {
        out.push_str(&format!(
            "import: core::prelude | {} | ;\n",
            wanted.join(" ")
        ));
    }
    out
}

/// Write a fixture source to `path`, with `fixture_imports` appended.
///
/// **Not for a fixture whose subject is `import:` itself.** Appending the
/// imports a fixture's text implies is what keeps several hundred goldens from
/// restating them, but it also means a fixture can never be observed *missing*
/// one -- so a test asserting that an unimported name is refused would pass
/// against a source the harness had already fixed. Write those verbatim
/// instead (`phase8_slice2.rs`'s `write_raw` is the pattern).
#[allow(dead_code)]
pub fn write_fixture(path: &Path, src: &str) -> std::io::Result<()> {
    std::fs::write(path, format!("{src}{}", fixture_imports(src)))
}

/// The bytes to write for one file of a multi-file fixture tree, keyed on its
/// name: `fixture_imports` is appended to Sooth source and to nothing else, so
/// the same tree helper can write a `sooth.pkg` beside a `.sth`. Carries
/// `write_fixture`'s caveat: not for a fixture about `import:` lines.
#[allow(dead_code)]
pub fn fixture_source(name: &str, contents: &str) -> String {
    match name.ends_with(".sth") {
        true => format!("{contents}{}", fixture_imports(contents)),
        false => contents.to_string(),
    }
}

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
///
/// P8.S2 (R7): deliberately *not* `--manifest`-passing. `examples/` is a real
/// package now (`examples/sooth.pkg`, depending on `core` at `../lib`), so a
/// committed example resolves through its own ancestor manifest -- which is
/// what `sooth build examples/gcd.sth` does too, and what makes its `self::`
/// imports spellable at all (a `--manifest` site rejects those). Fixtures
/// written to temp directories are the ones that need the shared manifest;
/// `manifest_for` is where that choice lives.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
