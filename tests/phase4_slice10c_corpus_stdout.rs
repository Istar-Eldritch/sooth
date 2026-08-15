//! E-P3-6: every committed `examples/*.sth` that declares its own `main`
//! produces byte-identical stdout across slice 10c's `if`/`else`/`end` ->
//! postfix `[ T ] [ E ] if` migration. The fixtures in `tests/corpus_stdout/`
//! pin each program's pre-migration output: the value that compiler emitted
//! before the swap, verified by rebuilding the compiler at the pre-P3
//! checkpoint and confirming its output for every example is byte-identical to
//! the committed fixture here. A regression in the migrated lowering shows up
//! as a program whose stdout no longer matches its pinned pre-migration value.
//!
//! Regenerate deliberately (never blindly to make a red test pass) when a
//! corpus program's output is *intended* to change:
//! `REGEN_CORPUS_STDOUT=1 cargo test --test phase4_slice10c_corpus_stdout`.

mod common;

use std::path::Path;

/// Every standalone example. `modules_ops.sth`/`modules_point.sth` are
/// excluded: they declare no `main` (they are libraries `modules.sth` imports)
/// and so cannot be run on their own.
const CORPUS: &[&str] = &[
    "array_ctor",
    "array_totals",
    "array_totals_hand",
    "bool_abi",
    "capturing_dispatch",
    "capturing_dispatch_hand",
    "combinator_in_times",
    "combinator_in_times_hand",
    "countdown",
    "factorial",
    "fill_relower",
    "filter_while",
    "filter_while_hand",
    "gcd",
    "inplace_fold",
    "leap",
    "lerp",
    "list",
    "mean",
    "modules",
    "poly_if",
    "refs",
    "resources",
    "rgb",
    "rgb_bits",
    "shapes",
    "sign",
    "stack",
    "strings",
    "times",
    "vectors",
    "vm",
    "vm_table",
];

#[test]
fn corpus_stdout_is_byte_identical_across_the_if_migration() {
    let regen = std::env::var_os("REGEN_CORPUS_STDOUT").is_some();
    for name in CORPUS {
        let binary = common::build_example(&format!("examples/{name}.sth"));
        let out = std::process::Command::new(&binary)
            .output()
            .unwrap_or_else(|e| panic!("running {name}: {e}"));
        assert!(out.status.success(), "{name} exited {}", out.status);
        let stdout = String::from_utf8(out.stdout).expect("corpus output is utf-8");
        let path = Path::new("tests/corpus_stdout").join(format!("{name}.txt"));
        if regen {
            std::fs::write(&path, &stdout).unwrap_or_else(|e| panic!("writing {path:?}: {e}"));
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("reading {path:?} (regenerate with REGEN_CORPUS_STDOUT=1): {e}")
        });
        assert_eq!(
            stdout, expected,
            "`examples/{name}.sth` drifted from its pre-migration stdout"
        );
        std::fs::remove_file(&binary).ok();
    }
}
