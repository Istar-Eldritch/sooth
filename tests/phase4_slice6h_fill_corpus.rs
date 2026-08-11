//! Phase 4 slice 6h: `fill`'s re-lowering (unrolled stores -> a counted loop)
//! must not change any program's output. The `.stdout` goldens in
//! `tests/fill_corpus/` were captured from the PRE-change binary and committed
//! before `lower_array_word`'s `"fill"` arm was touched, so this regression is
//! a real guard rather than a tautology written from the post-change binary.
//! Every committed example that both declares its own `main` and calls `fill`
//! is covered.

use std::path::Path;
use std::process::Command;

mod common;

const FILL_EXAMPLES: &[&str] = &[
    "array_totals",
    "array_totals_hand",
    "capturing_dispatch",
    "capturing_dispatch_hand",
    "combinator_in_times",
    "combinator_in_times_hand",
    "filter_while",
    "filter_while_hand",
    "inplace_fold",
    "refs",
    "resources",
    "stack",
    "vm",
    "vm_table",
];

#[test]
fn fill_using_examples_match_pre_change_stdout_baseline() {
    for name in FILL_EXAMPLES {
        let binary = common::build_example(&format!("examples/{name}.sth"));
        let output = Command::new(&binary)
            .output()
            .unwrap_or_else(|e| panic!("running {name}: {e}"));
        std::fs::remove_file(&binary).ok();
        let stdout = String::from_utf8(output.stdout)
            .unwrap_or_else(|e| panic!("{name} stdout not utf8: {e}"));
        let golden_path = Path::new("tests/fill_corpus").join(format!("{name}.stdout"));
        let expected = std::fs::read_to_string(&golden_path)
            .unwrap_or_else(|e| panic!("reading baseline {golden_path:?}: {e}"));
        assert_eq!(
            stdout, expected,
            "{name} stdout drifted from its pre-change fill baseline"
        );
        assert_eq!(output.status.code(), Some(0), "{name} exited nonzero");
    }
}
