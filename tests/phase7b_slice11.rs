//! P7b.S11 Phase 1 goldens: per-call-site grounding for bare generic
//! constructors.
//!
//! A bare `Ok`/`Err` (or any generated word of a header with a free type
//! parameter) had no per-call-site grounding at all: the outcome was a
//! function of whichever monomorphs some unrelated part of the same module
//! happened to mint at parse time, with a generic `unknown word` on zero
//! mints, an unconditional take of a sole *wrong* mint, and a silent
//! declaration-order-first pick on ties (`probes/dp_findings.md`). This suite
//! pins the ladder that replaces it (spec R-2/R-3/R-4), the three new located
//! diagnostics (R-5), and the undefined-name split (R-7).
//!
//! Fixture texts are the frozen `probes/dp_*.sth` bodies; the error goldens
//! pin the measured rendered bytes (measure-then-pin). Harness styled after
//! `tests/phase7b_slice10.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        static N: AtomicU64 = AtomicU64::new(0);
        let seq = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sooth-p7bs11-{}-{tag}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tree(dir)
    }

    /// The fixture, plus the manifest every file-based fixture needs, written
    /// into a fresh directory. Returns the entry path.
    fn program(tag: &str, body: &str) -> (Tree, PathBuf) {
        let t = Tree::new(tag);
        std::fs::write(
            t.0.join("sooth.pkg"),
            format!(
                "package: p7bs11 ;\nlayer: hosted ;\ndepends: core path \"{root}/lib/core\" ;\ndepends: hosted path \"{root}/lib/hosted\" ;\n",
                root = env!("CARGO_MANIFEST_DIR")
            ),
        )
        .unwrap();
        let entry = t.0.join("main.sth");
        std::fs::write(&entry, body).unwrap();
        (t, entry)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sooth_build(entry: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sooth"))
        .arg("build")
        .arg(entry)
        .output()
        .expect("sooth build should spawn")
}

/// The build's stderr, asserting the located build failure is the exit-1
/// diagnostic path the error goldens pin.
fn build_error(tag: &str, body: &str) -> String {
    let (_t, entry) = Tree::program(tag, body);
    let build = sooth_build(&entry);
    assert_eq!(
        build.status.code(),
        Some(1),
        "build should fail with exit 1; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    String::from_utf8(build.stderr).expect("stderr should be utf8")
}

fn build_and_run(tag: &str, body: &str) -> String {
    let (_t, entry) = Tree::program(tag, body);
    let build = sooth_build(&entry);
    assert!(
        build.status.success(),
        "build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = entry.with_extension("");
    let run = Command::new(&binary)
        .output()
        .expect("the built binary should run");
    assert!(run.status.success(), "the built binary should exit 0");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// The header every fixture shares, verbatim from `probes/dp_*.sth`.
const RES: &str = "import: intrinsics * ;\ntype: Res['T 'E] | Ok 'T | Err 'E ;\n";

// ---------------------------------------------------------------------------
// G1 (dp_c): zero mints, an undetermined parameter.
// ---------------------------------------------------------------------------

/// The unbound parameter is named as itself, with its position in the header
/// and a remedy. Before S11 this shape borrowed `unknown word `Ok``
/// (`probes/dp_baseline.md`) -- the wrong word, and indistinguishable from a
/// genuinely undefined name (G7 below is the other half of that split).
#[test]
fn bare_ctor_with_no_mint_and_an_unbound_parameter_names_the_parameter() {
    let err = build_error("g1", &format!("{RES}: main ( -- ) 1 Ok drop ;\n"));
    assert_eq!(
        err,
        "error: `Ok` in `main` (line 3) cannot be grounded here: `Res['T 'E]`'s type parameter `'E` (parameter 2 of 2) is determined by neither this call site's operands nor its consumer\n  note: pass the value to a consumer whose declared parameter names a concrete `Res[...]`, or name that instantiation in a signature so this call has one to ground at\n"
    );
    assert!(
        !err.contains("unknown word"),
        "R-7: the zero-mint case must not borrow the undefined-name diagnostic"
    );
}

// ---------------------------------------------------------------------------
// G2 (dp_d): a sole mint that disagrees at a bound position.
// ---------------------------------------------------------------------------

/// `mkok`'s signature mints the only `Res` monomorph in the program, and the
/// old single-candidate arm took it unconditionally -- forcing the *nested*
/// `Ok` (whose `'T` the operand pins to `Res[i64 i64]`) onto it and failing
/// three checks later with an operand mismatch that never mentioned
/// grounding. Rejection was already correct; the mechanism reaching it was
/// not.
#[test]
fn bare_ctor_whose_sole_mint_disagrees_at_a_bound_position_is_a_grounding_error() {
    let err = build_error(
        "g2",
        &format!("{RES}: mkok ( i64 -- Res[i64 i64] ) Ok ;\n: main ( -- ) 1 mkok Ok drop ;\n"),
    );
    assert_eq!(
        err,
        "error: `Ok` in `main` (line 4) cannot be grounded here: this call site needs `Res['T 'E]`'s `'T` to be `Res[i64 i64]`, but the only `Res` instantiation in scope is `Res[i64 i64]`, whose `'T` is `i64`\n  note: name the instantiation this call means in a signature, so it is minted here rather than borrowing the one that happens to exist\n"
    );
    assert!(
        !err.contains("type mismatch"),
        "the far-away operand mismatch is replaced, not merely preceded"
    );
}

// ---------------------------------------------------------------------------
// G3 (dp_g): two compatible mints, no determining consumer.
// ---------------------------------------------------------------------------

/// dp_g exits 0 today, silently constructing whichever `Res[i64 ?]` was
/// declared first -- a correctness gap, not a diagnostics gap. First-wins is
/// retired: the tie is a located error, and the tied types are listed sorted
/// by rendered string, so the *bytes* are identical under a declaration-order
/// swap (NFR-3) rather than merely the verdict.
#[test]
fn bare_ctor_with_two_compatible_mints_is_an_order_stable_ambiguity_error() {
    let expected = "error: `Ok` in `main` (line 5) is ambiguous: `Res['T 'E]`'s type parameter `'E` is determined by neither this call site's operands nor its consumer, and 2 instantiations in scope fit it\n  candidate: `Res[i64 cstr]`\n  candidate: `Res[i64 i64]`\n  note: pass the value to a consumer whose declared parameter names the concrete `Res[...]` this call means\n";
    let a_first = build_error(
        "g3-a-first",
        &format!(
            "{RES}: unused_a ( Res[i64 i64] -- ) drop ;\n\
             : unused_b ( Res[i64 cstr] -- ) drop ;\n\
             : main ( -- ) 1 Ok drop ;\n"
        ),
    );
    let b_first = build_error(
        "g3-b-first",
        &format!(
            "{RES}: unused_b ( Res[i64 cstr] -- ) drop ;\n\
             : unused_a ( Res[i64 i64] -- ) drop ;\n\
             : main ( -- ) 1 Ok drop ;\n"
        ),
    );
    assert_eq!(a_first, expected);
    assert_eq!(b_first, expected, "the text is stable across the swap");
}

// ---------------------------------------------------------------------------
// G5: the poly-consumer flavor, grounding at a monomorph nothing has minted.
// ---------------------------------------------------------------------------

/// The consumer is a *polymorphic* word whose declared input names the header
/// (`Res['T 'E]`) and whose explicit type arguments pin it. No signature
/// anywhere in the program spells a concrete `Res[...]`, so grounding here
/// mints `Res[i64 i64]` mid-check -- a fresh monomorph minted after `env` was
/// built, under the same `(header, module, arguments)` key `apply_subst`
/// itself uses, so `apply2`'s own `check_poly_call` then resolves that very
/// monomorph (R-8). The quotation literal between the two calls is stepped
/// over by the consumer lookahead, which reads `apply2`'s input window one
/// slot down.
///
/// Deviation from the spec's `1 Ok [ 1 sub ] map[i64 i64 i64] drop` wording,
/// recorded here rather than silently: a word declaring an *abstract*
/// quotation parameter (`~[ 'T -- 'U ]`) has to be `inline`, and a
/// non-trait-member `inline` word declaring a top-level `PolyType::Generic`
/// input is refused outright at its own declaration
/// (`poly_generic_not_yet_groundable_error`, `check_poly_combinator_standalone`),
/// so no `map` of that shape is declarable at this base -- a pre-existing gap
/// with nothing to do with grounding. `apply2` is the same shape with a
/// *ground* quotation parameter, which is declarable and exercises the
/// identical mechanism.
#[test]
fn bare_ctor_grounds_at_a_poly_consumers_explicit_type_arguments() {
    let out = build_and_run(
        "g5",
        &format!(
            "{RES}import: hosted::show | . | ;\n\
             : apply2 ( Res['T 'E] [ i64 -- i64 ] -- i64 ) | f | drop 41 f call ;\n\
             : main ( -- ) 1 Ok [ 1 add ] apply2[i64 i64] . ;\n"
        ),
    );
    assert_eq!(out, "42\n");
}

// ---------------------------------------------------------------------------
// G7 (R-7): the undefined-name half of the split.
// ---------------------------------------------------------------------------

/// A name no header claims still gets the unchanged `unknown word`, byte for
/// byte -- the diagnostic G1 stopped borrowing.
#[test]
fn genuinely_undefined_ctor_name_is_still_the_unchanged_unknown_word() {
    let err = build_error("g7", &format!("{RES}: main ( -- ) 1 Nope drop ;\n"));
    assert_eq!(err, "error: unknown word `Nope` in `main` (line 3)\n");
}

// ---------------------------------------------------------------------------
// G9 (dp_g2 / dp_g3): competing mints plus a determining consumer.
// ---------------------------------------------------------------------------

/// Ruling (A): a *monomorphic* consumer's signature pins θ statically at the
/// site, so a fully bound θ grounds directly and never reaches candidate
/// selection at all -- the two competing mints are irrelevant. dp_g2 exits 1
/// today and dp_g3 exits 0, differing only in the declaration order of two
/// unrelated unused words; both are accepted here, and the *runtime output*
/// is byte-identical across the swap.
///
/// Acceptance is itself the proof of which monomorph was chosen:
/// `only_takes_cstr_err` accepts `Res[i64 cstr]` alone, so a `Res[i64 i64]`
/// would be an operand mismatch.
#[test]
fn bare_ctor_with_a_determining_mono_consumer_grounds_identically_in_both_orders() {
    let program = |first: &str, second: &str| {
        format!(
            "{RES}import: hosted::show | . | ;\n\
             : {first} ;\n\
             : {second} ;\n\
             : only_takes_cstr_err ( Res[i64 cstr] -- ) drop 5 . ;\n\
             : main ( -- ) 1 Ok only_takes_cstr_err ;\n"
        )
    };
    let a = "unused_a ( Res[i64 i64] -- ) drop";
    let b = "unused_b ( Res[i64 cstr] -- ) drop";
    let a_first = build_and_run("g9-a-first", &program(a, b));
    let b_first = build_and_run("g9-b-first", &program(b, a));
    assert_eq!(a_first, "5\n");
    assert_eq!(b_first, "5\n", "behavior is identical across the swap");
}

// ---------------------------------------------------------------------------
// dp_h: minting is whole-module, declaration-order-independent.
// ---------------------------------------------------------------------------

/// dp_f's twin with the sole mint declared *after* the caller (`main`
/// first, `unused` second): the bare `Ok` still grounds, because grounding
/// reads the whole module's registry rather than what is textually earlier
/// (`probes/dp_findings.md`, dp_h). Both orders accept and behave
/// identically -- the order dependence dp_g2/dp_g3 exposed lived only in
/// the retired first-wins tie-break, never in minting.
#[test]
fn sole_mint_declared_after_the_caller_still_grounds_the_call() {
    let show = "import: hosted::show | . | ;\n";
    let caller_first = format!(
        "{RES}{show}: main ( -- ) 1 Ok drop 3 . ;\n\
         : unused ( Res[i64 i64] -- ) drop ;\n"
    );
    let caller_last = format!(
        "{RES}{show}: unused ( Res[i64 i64] -- ) drop ;\n\
         : main ( -- ) 1 Ok drop 3 . ;\n"
    );
    assert_eq!(build_and_run("dp-h-caller-first", &caller_first), "3\n");
    assert_eq!(
        build_and_run("dp-h-caller-last", &caller_last),
        "3\n",
        "minting is whole-module: both declaration orders ground identically"
    );
}

// ---------------------------------------------------------------------------
// G4 (dp_e) + dp_e2 (R-6): the explicit-args category.
// ---------------------------------------------------------------------------

/// dp_e verbatim: full-arity explicit args on a bare ctor name are the
/// category R-6 admits. The args pin every parameter outright (R-2 input 1),
/// the fully bound θ mints `Res[i64 i64]` mid-check through the ordinary
/// lookup-or-mint (R-8), and the program runs clean. Before this slice the
/// spelling was rejected upstream at the type-args gate, consumer or not
/// (`probes/dp_findings.md`, dp_e).
#[test]
fn explicit_args_ctor_spelling_is_accepted_and_runs_clean() {
    let out = build_and_run(
        "g4-dp-e",
        &format!("{RES}: main ( -- ) 1 Ok[i64 i64] drop ;\n"),
    );
    assert_eq!(out, "");
}

/// dp_e2's substance: the category is independent of whether a consumer
/// exists. Nothing consumes the construction here -- no `drop`, no call after
/// it -- the word's declared output is what keeps it from being forgotten,
/// and the args alone ground it. (The literal dp_e2 probe shape, `1
/// Ok[i64 i64] ;` in a `( -- )` word, now reaches the ordinary forgetting
/// check instead of the old gate rejection; that diagnostic is pinned by the
/// unit `dp_e2_literal_no_output_shape_reaches_the_forgetting_check` beside
/// the grounding site.)
#[test]
fn explicit_args_ctor_grounds_with_no_consumer_and_nothing_forgotten() {
    let out = build_and_run(
        "dp-e2",
        &format!("{RES}: main ( -- Res[i64 i64] ) 1 Ok[i64 i64] ;\n"),
    );
    assert_eq!(out, "");
}

/// R-6's arity rule, byte-exact: a prefix list (`Ok[i64]` meaning
/// `Ok[i64 'E]`) is out of scope, so a wrong-arity list is a located error
/// naming the header's full declared shape -- measure-then-pin.
#[test]
fn explicit_args_ctor_with_wrong_arity_is_a_located_error() {
    let err = build_error(
        "g4-arity",
        &format!("{RES}: main ( -- ) 1 Ok[i64] drop ;\n"),
    );
    assert_eq!(
        err,
        "error: `Ok` in `main` (line 3) takes 2 type arguments (`Res['T 'E]`), but 1 was supplied\n"
    );
}

// ---------------------------------------------------------------------------
// G6 (NFR-2/4): the non-regression half, end to end.
// ---------------------------------------------------------------------------

/// dp_a / dp_b / dp_f still build and run: dp_a and dp_b on a fully bound θ (the word's
/// declared output, resp. the consumer's declared input); dp_f's ladder declines (its
/// sole-mint arm is unreachable at both call sites) and the pre-existing `[only]` take grounds it.
#[test]
fn the_accepting_probe_shapes_still_build_and_run() {
    let show = "import: hosted::show | . | ;\n";
    // dp_a: the expectation flows in from a concretely-typed helper's own
    // declared output effect.
    assert_eq!(
        build_and_run(
            "g6-dp-a",
            &format!(
                "{RES}{show}: mkok ( i64 -- Res[i64 i64] ) Ok ;\n\
                 : main ( -- ) 1 mkok drop 1 . ;\n"
            )
        ),
        "1\n"
    );
    // dp_b: a consumer with a fully concrete declared input.
    assert_eq!(
        build_and_run(
            "g6-dp-b",
            &format!(
                "{RES}{show}: showres ( Res[i64 i64] -- ) drop 2 . ;\n\
                 : main ( -- ) 1 Ok showres ;\n"
            )
        ),
        "2\n"
    );
    // dp_f: an unused, uncalled sibling's mere declaration is the sole
    // compatible mint; `'E` binds from it.
    assert_eq!(
        build_and_run(
            "g6-dp-f",
            &format!(
                "{RES}{show}: unused ( Res[i64 i64] -- ) drop ;\n\
                 : main ( -- ) 1 Ok drop 3 . ;\n"
            )
        ),
        "3\n"
    );
}
