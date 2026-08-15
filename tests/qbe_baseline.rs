//! R9 baseline golden (slice 8a): the emitted QBE IL for a cross-section of the
//! corpus must stay byte-identical across the builtin-table refactor. The
//! snapshots live in `tests/qbe_baseline/`. This is the phase's exit artifact:
//! program-output goldens catch a behaviour change, but only a committed
//! codegen snapshot catches a *silent* emitted-QBE drift for the corpus.
//!
//! Regenerate deliberately (never blindly to make a red test pass) when codegen
//! is *intended* to change: `REGEN_QBE_BASELINE=1 cargo test --test qbe_baseline`,
//! then review the diff.

use std::path::{Path, PathBuf};

/// The `(gcd/factorial/strings/refs/resources)` cross-section the spec names:
/// arithmetic + control flow, recursion, strings/`cstr`, references, and the
/// resource/allocation path.
///
/// Slice 8a fix 4: every other standalone `examples/*.sth` (one declaring its
/// own `main`) is added too, so the byte-for-byte guarantee covers the whole
/// committed corpus, not just this original cross-section. `modules_ops.sth`/
/// `modules_point.sth` are excluded: they declare no `main` of their own (they
/// are libraries `modules.sth` imports) and so cannot be built standalone by
/// `emit_ssa`; `modules.sth` itself, which pulls both in, is included.
const CORPUS: &[(&str, &str)] = &[
    ("gcd", "examples/gcd.sth"),
    ("factorial", "examples/factorial.sth"),
    ("strings", "examples/strings.sth"),
    ("refs", "examples/refs.sth"),
    ("resources", "examples/resources.sth"),
    ("array_totals", "examples/array_totals.sth"),
    ("array_totals_hand", "examples/array_totals_hand.sth"),
    ("bool_abi", "examples/bool_abi.sth"),
    ("combinator_in_times", "examples/combinator_in_times.sth"),
    (
        "combinator_in_times_hand",
        "examples/combinator_in_times_hand.sth",
    ),
    ("countdown", "examples/countdown.sth"),
    ("filter_while", "examples/filter_while.sth"),
    ("filter_while_hand", "examples/filter_while_hand.sth"),
    ("inplace_fold", "examples/inplace_fold.sth"),
    ("leap", "examples/leap.sth"),
    ("lerp", "examples/lerp.sth"),
    ("list", "examples/list.sth"),
    ("mean", "examples/mean.sth"),
    ("modules", "examples/modules.sth"),
    ("poly_if", "examples/poly_if.sth"),
    ("rgb", "examples/rgb.sth"),
    ("rgb_bits", "examples/rgb_bits.sth"),
    ("shapes", "examples/shapes.sth"),
    ("sign", "examples/sign.sth"),
    ("stack", "examples/stack.sth"),
    ("times", "examples/times.sth"),
    ("vectors", "examples/vectors.sth"),
    ("vm", "examples/vm.sth"),
    ("vm_table", "examples/vm_table.sth"),
];

fn baseline_path(name: &str) -> PathBuf {
    Path::new("tests/qbe_baseline").join(format!("{name}.ssa"))
}

#[test]
fn corpus_qbe_stays_byte_identical_to_baseline() {
    let regen = std::env::var_os("REGEN_QBE_BASELINE").is_some();
    for (name, src) in CORPUS {
        let ssa = sooth::driver::emit_ssa(Path::new(src))
            .unwrap_or_else(|e| panic!("emitting QBE for {src}: {e}"));
        let path = baseline_path(name);
        if regen {
            std::fs::write(&path, &ssa)
                .unwrap_or_else(|e| panic!("writing baseline {path:?}: {e}"));
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("reading baseline {path:?} (regenerate with REGEN_QBE_BASELINE=1): {e}")
        });
        assert_eq!(
            ssa, expected,
            "emitted QBE for {src} drifted from its R9 baseline; if the change is \
             intended, regenerate with REGEN_QBE_BASELINE=1 and review the diff"
        );
    }
}

/// R9, focused: the i64 arithmetic tower and a comparison, resolved through the
/// builtin table, must lower to exactly the pre-refactor instructions
/// (`add`/`sub`/`mul`/`csltl`). The corpus baseline is broad but noisy; this
/// pins the operator-lowering half of R9 to a single readable function block,
/// isolated from the fixed runtime preamble so an unrelated runtime change
/// can't break it.
#[test]
fn operator_i64_lowers_identically_after_table() {
    use sooth::{backend, check, ir, lexer, parser};

    // Slice 10c: the comparison *primitive*. `<` is a `lib/` word now, and
    // its branch-and-construct body adds a diamond here that QBE folds away
    // in the emitted machine code but not in the IL; asserting the IL of `<`
    // would be asserting the library definition, not the operator lowering
    // this test exists to pin.
    let src = ": ops ( i64 i64 -- i64 ) | a b | a b + a b - * a b u< drop ;";
    let tokens = lexer::lex(src).unwrap();
    let mut module = parser::parse(&tokens).unwrap();
    check::check(&mut module).unwrap();
    let ir = ir::lower(&module).unwrap();
    let ssa = backend::qbe::emit(&ir).unwrap();

    let start = ssa
        .find("export function l $ops")
        .expect("the `ops` word is emitted as an exported function");
    let end = ssa[start..]
        .find("\n}\n")
        .map(|rel| start + rel + 3)
        .expect("the `ops` function block closes");

    assert_eq!(
        &ssa[start..end],
        concat!(
            "export function l $ops(l %v0, l %v1) {\n",
            "@start\n",
            "\t%v2 =l add %v0, %v1\n",
            "\t%v3 =l sub %v0, %v1\n",
            "\t%v4 =l mul %v2, %v3\n",
            "\t%v5 =w csltl %v0, %v1\n",
            "\tret %v4\n",
            "}\n",
        ),
    );
}
