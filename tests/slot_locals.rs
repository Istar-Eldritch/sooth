//! Named-slot-locals sugar (`docs/named-slot-locals-spec.md`), Phase 1
//! goldens: parse/desugar core -- spelling support, the `Bind`-prepend
//! desugar, positional mints, and the duplicate/poly-name rejects.

use sooth::{check, lexer, test_support};

mod common;

/// Compile and run `src`, returning its stdout and exit code. `name`
/// distinguishes the temp source (and so the emitted binary) per test, since
/// the goldens run in parallel in one process.
fn run_src(name: &str, src: &str) -> (String, i32) {
    let path = std::env::temp_dir().join(format!("sooth-{name}-{}.sth", std::process::id()));
    common::write_fixture(&path, src).expect("writing temp source should succeed");
    let binary = sooth::driver::build_with_manifest(&path, common::manifest_for(&path).as_deref())
        .expect("build should succeed");
    let output = std::process::Command::new(&binary)
        .env_remove(sooth::ir::TRACE_ALLOC_ENV)
        .output()
        .expect("binary should run");
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&binary).ok();
    (
        String::from_utf8(output.stdout).expect("stdout should be utf8"),
        output
            .status
            .code()
            .expect("process should exit normally, not die by signal"),
    )
}

fn parse_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    test_support::parse_with_core(&tokens).expect_err("parsing should fail")
}

fn check_error(src: &str) -> String {
    let tokens = lexer::lex(src).expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect_err("check should fail")
}

// ---- R3: all-named slots match their explicit `| ... |` twin ----

#[test]
fn slot_sugar_all_named_matches_explicit_twin_expected() {
    let (sugar_out, sugar_code) = run_src(
        "slot-sugar-all-named",
        ": f ( a: i64 b: i64 -- i64 ) a b add ;\n: main ( -- ) 3 4 f . ;\n",
    );
    let (twin_out, twin_code) = run_src(
        "slot-sugar-all-named-twin",
        ": f ( i64 i64 -- i64 ) | a b | a b add ;\n: main ( -- ) 3 4 f . ;\n",
    );
    assert_eq!(sugar_out, "7\n");
    assert_eq!(sugar_code, 0);
    assert_eq!(sugar_out, twin_out);
    assert_eq!(sugar_code, twin_code);
}

// ---- R5: an out-of-order named slot matches its mint+repush twin ----

#[test]
fn slot_sugar_out_of_order_matches_explicit_twin_expected() {
    let (sugar_out, sugar_code) = run_src(
        "slot-sugar-out-of-order",
        ": f ( a: i64 i64 -- i64 ) | b | a b add ;\n: main ( -- ) 3 4 f . ;\n",
    );
    let (twin_out, twin_code) = run_src(
        "slot-sugar-out-of-order-twin",
        ": f ( i64 i64 -- i64 ) | a t | t | b | a b add ;\n: main ( -- ) 3 4 f . ;\n",
    );
    assert_eq!(sugar_out, "7\n");
    assert_eq!(sugar_code, 0);
    assert_eq!(sugar_out, twin_out);
    assert_eq!(sugar_code, twin_code);
}

// ---- R6: a mint survives a user-named slot0 mint-index collision ----

#[test]
fn slot_sugar_mint_survives_user_slot_name_collision_expected() {
    let (stdout, code) = run_src(
        "slot-sugar-mint-user-collision",
        ": f ( a: i64 i64 -- i64 )\n  | __slot1 |\n  a __slot1 add ;\n\
: main ( -- ) 3 4 f . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

// ---- R6: cascading mint bumps when a sibling mint's own candidate collides ----

#[test]
fn slot_sugar_mint_bumped_on_sibling_mint_collision_expected() {
    let (sugar_out, sugar_code) = run_src(
        "slot-sugar-mint-sibling-mint-collision",
        ": f ( a: i64 i64 i64 -- i64 )\n  drop\n  | __slot1 |\n  a __slot1 add ;\n\
: main ( -- ) 3 4 5 f . ;\n",
    );
    // Explicit twin: the desugar's own bind order is `a`, then a mint for
    // input index 1 (bumped past the user's own `__slot1` to `__slot2`),
    // then a mint for input index 2 (bumped past that to `__slot3`), each
    // immediately re-pushed in original relative order; the body's `drop`
    // consumes index 2's re-pushed value, and the user's own `| __slot1 |`
    // then rebinds index 1's, so `a __slot1 add` is `3 4 add` = 7.
    let (twin_out, twin_code) = run_src(
        "slot-sugar-mint-sibling-mint-collision-twin",
        ": f ( i64 i64 i64 -- i64 )\n  | a __slot2 __slot3 |\n  __slot2 __slot3\n  drop\n  | __slot1 |\n\
  a __slot1 add ;\n: main ( -- ) 3 4 5 f . ;\n",
    );
    assert_eq!(sugar_out, "7\n");
    assert_eq!(sugar_code, 0);
    assert_eq!(sugar_out, twin_out);
    assert_eq!(sugar_code, twin_code);
}

// ---- R6: a mint is fresh against the effect's own `__slot1`-named slot ----

#[test]
fn slot_sugar_mint_bumped_on_sibling_slot_name_collision_expected() {
    let (stdout, code) = run_src(
        "slot-sugar-mint-sibling-slot-name-collision",
        ": f ( __slot1: i64 i64 i64 -- i64 ) drop drop __slot1 ;\n\
: main ( -- ) 3 4 5 f . ;\n",
    );
    assert_eq!(stdout, "3\n");
    assert_eq!(code, 0);
}

// ---- R2 exemption: a qualified type slot is unaffected by the glued hint ----

#[test]
fn parse_slot_qualified_type_slot_unaffected_by_glued_hint_expected() {
    let result_import = format!(
        "import: \"{}/lib/core/result.sth\" r | Ok Err | ;\n",
        env!("CARGO_MANIFEST_DIR")
    );
    let (stdout, code) = run_src(
        "slot-qualified-type-unaffected",
        &format!(
            "{result_import}\
             : to-int ( r::Result[i64 i64] -- i64 )\n\
               ~[ ( Ok )  Ok> ]\n\
               ~[ ( Err ) Err> ]\n\
               Result? ;\n\
             : main ( -- ) 12 Ok to-int . ;\n"
        ),
    );
    assert_eq!(stdout, "12\n");
    assert_eq!(code, 0);
}

// ---- Open Questions ruling: a trailing-colon type name's own slots ----

#[test]
fn parse_worddef_trailing_colon_type_name_is_slot_name_expected() {
    // The parser reads `Foo:` as a slot-name attempt (glued split), then
    // dies sharply because the effect closer follows instead of a type.
    let missing_ty = parse_error("type: Foo: val i64 ;\n: f ( Foo: -- ) ;\n");
    assert!(
        missing_ty.starts_with("error:"),
        "unexpected message: {missing_ty}"
    );

    let (stdout, code) = run_src(
        "slot-trailing-colon-type-name",
        "type: Foo: val i64 ;\n: f ( Foo: i64 -- i64 ) Foo ;\n: main ( -- ) 9 f . ;\n",
    );
    assert_eq!(stdout, "9\n");
    assert_eq!(code, 0);
}

// ---- R11 exemption: `'T :` / `'T:` keep the bound-in-effect error ----

#[test]
fn parse_poly_slot_bound_attempt_keeps_bound_in_effect_error() {
    let spaced = parse_error(": f ( 'T : Copy -- ) drop ;");
    assert!(
        spaced.contains("may not be written inside a stack effect"),
        "unexpected message: {spaced}"
    );

    let glued = parse_error(": f ( 'T: Copy -- ) drop ;");
    assert!(
        glued.contains("may not be written inside a stack effect"),
        "unexpected message: {glued}"
    );
}

// ---- R10: a concrete quotation-effect row's name attempt is unchanged ----

#[test]
fn parse_quotation_row_name_attempt_unchanged_error() {
    let err = parse_error(": f ( [ x : i64 -- i64 ] -- ) drop ;");
    assert!(err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("`x`"), "unexpected message: {err}");
}

// ---- R8/R9: glued spellings on extern/output slots stay doc-only ----

#[test]
fn parse_extern_glued_named_slot_stays_doc_only_expected() {
    let module = test_support::parse_with_core(
        &lexer::lex("extern: f ( a: i64 -- b: i64 ) \"f\" ;\n").unwrap(),
    )
    .expect("extern with glued named slots should parse");
    assert_eq!(
        module.externs[0].effect.inputs[0].name.as_deref(),
        Some("a")
    );
    assert_eq!(
        module.externs[0].effect.outputs[0].name.as_deref(),
        Some("b")
    );
}

#[test]
fn parse_worddef_output_glued_named_slot_stays_doc_only_expected() {
    let module = test_support::parse_with_core(&lexer::lex(": f ( i64 -- x: i64 ) ;\n").unwrap())
        .expect("worddef with a glued named output slot should parse");
    assert_eq!(module.words[0].effect.outputs[0].name.as_deref(), Some("x"));
}

// ---- R1: glued and spaced spellings are twins ----

#[test]
fn slot_sugar_glued_spelling_matches_spaced_expected() {
    let (glued_out, glued_code) = run_src(
        "slot-sugar-glued",
        ": f ( a: i64 -- i64 ) a ;\n: main ( -- ) 5 f . ;\n",
    );
    let (spaced_out, spaced_code) = run_src(
        "slot-sugar-spaced",
        ": f ( a : i64 -- i64 ) a ;\n: main ( -- ) 5 f . ;\n",
    );
    assert_eq!(glued_out, "5\n");
    assert_eq!(glued_code, 0);
    assert_eq!(glued_out, spaced_out);
    assert_eq!(glued_code, spaced_code);
}

// ---- R2: a fully-glued slot-name attempt gets the located hint error ----

#[test]
fn parse_slot_fully_glued_name_is_located_hint_error() {
    let err = parse_error(": f ( x:i64 -- ) drop ;");
    assert!(!err.contains("unknown type"), "unexpected message: {err}");
    assert!(err.contains("space after"), "unexpected message: {err}");
    assert!(err.contains("write `x : i64`"), "unexpected message: {err}");
}

// ---- R12: a duplicate input slot name is a located parse error ----

#[test]
fn parse_worddef_duplicate_input_slot_name_is_error() {
    let err = parse_error(": f ( x : i64 x : i64 -- ) drop drop ;");
    assert!(err.contains('x'), "unexpected message: {err}");
    assert!(
        err.contains("more than once") || err.contains("duplicate"),
        "unexpected message: {err}"
    );
}

// ---- R11: a named poly-effect slot is a located sharp reject ----

#[test]
fn parse_poly_slot_named_is_rejected_with_located_error() {
    let spaced = parse_error(": f ( x : 'T -- ) drop ;");
    assert!(
        spaced.contains("slot names are not supported in polymorphic effects"),
        "unexpected message: {spaced}"
    );

    let glued = parse_error(": f ( x: 'T -- ) drop ;");
    assert!(
        glued.contains("slot names are not supported in polymorphic effects"),
        "unexpected message: {glued}"
    );
}

// ---- R6 freshness sources beyond the body's own top-level Bind ----

// Pins the `TermKind::Quotation` recursion in `collect_bound_names`
// (src/parser.rs:3269): a nested `~[ | __slot1 | ... ]` inside the body binds
// `__slot1`, which must bump the mint for the unnamed input slot to
// `__slot2`. If that recursion were deleted, the freshness scan would miss
// the nested bind, mint `__slot1` for the unnamed slot, and the checker
// would reject the word with `` `__slot1` is already bound in `f` ``.
#[test]
fn slot_sugar_mint_fresh_against_nested_quotation_bind_expected() {
    let (stdout, code) = run_src(
        "slot-sugar-mint-fresh-nested-quotation",
        "import: core::combinators c ;\n\
: f ( a: i64 i64 -- i64 ) | b | 2 ~[ | __slot1 | __slot1 . ] c::times a b add ;\n\
: main ( -- ) 3 4 f . ;\n",
    );
    assert_eq!(stdout, "0\n1\n7\n");
    assert_eq!(code, 0);
}

// Pins `fresh.insert(word_name)` (src/parser.rs:3322): a word named
// `__slot1` with one named and one unnamed input slot must not mint its own
// name for the unnamed slot's positional local. If that insert were deleted,
// the mint would collide with the word's own callable name and the checker
// would reject it with a callable-collision error.
#[test]
fn slot_sugar_mint_fresh_against_word_name_expected() {
    let (stdout, code) = run_src(
        "slot-sugar-mint-fresh-word-name",
        ": __slot1 ( a: i64 i64 -- i64 ) | b | a b add ;\n: main ( -- ) 3 4 __slot1 . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

// Pins the enum-variant loop (src/parser.rs:3328-3332): a module-level enum
// variant named `__slot1` must also be in the mint's freshness set. If that
// loop were deleted, the mint would collide with the variant name and the
// checker would reject it with a variant-collision error.
#[test]
fn slot_sugar_mint_fresh_against_enum_variant_expected() {
    let (stdout, code) = run_src(
        "slot-sugar-mint-fresh-enum-variant",
        "type: E | __slot1 | ;\n: f ( a: i64 i64 -- i64 ) | b | a b add ;\n: main ( -- ) 3 4 f . ;\n",
    );
    assert_eq!(stdout, "7\n");
    assert_eq!(code, 0);
}

// ---- R8/R9: extern/output behaviour is unchanged (spaced spelling) ----

#[test]
fn parse_extern_named_slots_stay_doc_only_expected() {
    let module =
        test_support::parse_with_core(&lexer::lex("extern: f ( a : i64 -- ) \"f\" ;\n").unwrap())
            .expect("extern with a named slot should parse");
    assert_eq!(
        module.externs[0].effect.inputs[0].name.as_deref(),
        Some("a")
    );
}

#[test]
fn parse_worddef_output_slot_name_stays_doc_only_expected() {
    let module = test_support::parse_with_core(&lexer::lex(": f ( i64 -- x : i64 ) ;\n").unwrap())
        .expect("worddef with a named output slot should parse");
    assert_eq!(module.words[0].effect.outputs[0].name.as_deref(), Some("x"));
}

// ---- R14: slot names obey the inherited local rules ----

#[test]
fn slot_local_named_like_word_is_callable_collision_error() {
    // The desugar binds `add` as an ordinary local (`callable_local_error`,
    // `src/check.rs:1235`); a slot named like a callable is exactly as
    // illegal as a hand-written `| add |`.
    let err = check_error(": f ( add : i64 -- i64 ) add ;\n");
    assert!(
        err.contains("local `add` in `f` collides with the callable name `add`"),
        "unexpected message: {err}"
    );
}

#[test]
fn slot_local_rebound_by_body_block_is_rebind_error() {
    // The desugar's leading `Bind(["a"])` puts `a` in scope before the body
    // runs; a body-level `| a |` then rebinds it while still live
    // (`rebound_local_error`, `src/check.rs:2936`).
    let err = check_error(": f ( a : i64 -- i64 ) | a | a ;\n");
    assert!(
        err.contains("`a` is already bound in `f`"),
        "unexpected message: {err}"
    );
}

#[test]
fn slot_local_named_like_variant_is_x12_parameter_error() {
    // X12 (`src/check/word_entry.rs:33-38`) rejects a parameter name equal to
    // a registered variant name; the desugared slot is a parameter for its
    // purposes.
    let err = check_error("type: E | V | ;\n: f ( V : i64 -- i64 ) drop ;\n");
    assert!(err.contains("parameter `V`"), "unexpected message: {err}");
    assert!(
        err.contains("collides with the variant name `V`"),
        "unexpected message: {err}"
    );
}

#[test]
fn slot_local_unused_named_slot_is_linear_leak_error() {
    // `^i64` is linear (an owned cell): a named slot the body never consumes
    // fails the bound-but-unused check (`Scope::leave`,
    // `src/check/engine.rs:557-570`), naming the slot.
    let err = check_error(": f ( x : ^i64 -- ) ;\n");
    assert!(
        err.contains("linear value `x` is never consumed in `f`"),
        "unexpected message: {err}"
    );

    // `array[i64 4]` is Copy: an unused named slot of that type imposes no
    // use obligation and still compiles.
    let tokens = lexer::lex(": f ( x : array[i64 4] -- ) ;\n").expect("lexing should succeed");
    let mut module = test_support::parse_with_core(&tokens).expect("parsing should succeed");
    check::check(&mut module).expect("an unused Copy-typed named slot should compile");
}

// ---- R16: the desugared leading Bind never fires the entry-arity diagnostic ----

#[test]
fn slot_sugar_never_fires_entry_arity_diagnostic_expected() {
    let (stdout, code) = run_src(
        "slot-sugar-entry-arity-corpus",
        ": f ( a: i64 b: i64 -- i64 ) a b add ;\n\
: g ( a: i64 i64 -- i64 ) | b | a b add ;\n\
: main ( -- ) 3 4 f . 3 4 g . ;\n",
    );
    assert_eq!(stdout, "7\n7\n");
    assert_eq!(code, 0);

    // Control: a hand-written over-arity `| a b |` on a 1-input word still
    // trips the entry-arity diagnostic (`src/check/word_entry.rs:200-213`) --
    // the desugar's exemption is not a blanket suppression of the check.
    let err = check_error(": w ( i64 -- i64 ) | a b | a ;\n");
    assert!(
        err.contains("locals bind 2 value(s), but only 1 input(s) are declared"),
        "unexpected message: {err}"
    );
}

// ---- R17: a desugared leading Bind lowers identically on the self-tail path ----

#[test]
fn slot_sugar_tail_call_matches_explicit_twin_expected() {
    // The spec cites the leading-Bind split in `lower_self_tail_combinator`
    // (src/ir/func_builder/calls.rs:100-135); that branch fires only for a
    // quotation-typed leading Bind, which the sugar can never produce (a
    // quotation-typed slot forces the poly path, where R11 rejects the name).
    // This fixture is the strongest reachable witness: a named-slot inline
    // self-tail word whose desugared leading Bind flows through the shared
    // tail walk (see also `param_binds`, src/check/drop_graph.rs:62).
    let (sugar_out, sugar_code) = run_src(
        "slot-sugar-self-tail",
        ": down inline ( n: i64 -- i64 ) n 0 gt ~[ n 1 sub down ] ~[ n ] if ;\n\
: main ( -- ) 5 down . ;\n",
    );
    let (twin_out, twin_code) = run_src(
        "slot-sugar-self-tail-twin",
        ": down inline ( i64 -- i64 ) | n | n 0 gt ~[ n 1 sub down ] ~[ n ] if ;\n\
: main ( -- ) 5 down . ;\n",
    );
    assert_eq!(sugar_out, "0\n");
    assert_eq!(sugar_code, 0);
    assert_eq!(sugar_out, twin_out);
    assert_eq!(sugar_code, twin_code);
}
